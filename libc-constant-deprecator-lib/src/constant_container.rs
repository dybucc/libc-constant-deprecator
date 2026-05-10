use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use regex::bytes::{Regex, RegexBuilder};
use syn::{Item, ItemConst, spanned::Spanned};
use tokio::{fs, task::JoinSet};

use crate::{
    BorrowedContainer, ChangesKind, Const, FilterError, FilterErrorRepr, IoBoundChanges,
    MakeChangesError, MakeChangesErrorRepr, deprecate,
};

pub(crate) mod borrowed;

#[derive(Debug)]
pub struct ConstContainer {
    inner: Vec<Arc<(Const, bool)>>,
    // NOTE: this uses an unbounded cache for prior computed regexes, in the hopes that full
    // regexes that get submitted to `ConstContainer::filter()` are "human-paced" (i.e. they aren't
    // coming at such speed and volume as to cause issues with memory consumption.)
    re_cache: HashMap<String, Regex>,
}

macro_rules! filter_impl {
    (@doc filter_with) => {
        "This routine does not allocate a new container for the borrowed view,\ninstead reusing \
         the provided one.\n\n"
    };
    (@doc filter) => {
        ""
    };
    (@doc $it:tt { $f:item }) => {
        /// Filters out constants by identifier provided a regex matching against
        /// those identifiers.
        ///
        /// Returns a borrowed container that can bulk deprecate the matched
        /// constants at once, and remembers which constants have been modified to
        /// only effect those changes to disk later on.
        #[doc = filter_impl! { @doc $it }]
        /// # Errors
        ///
        /// Fails if the regex failed to compile. This may be due to a byte size
        /// failure at the regex engine level, or due to a parsing failure at the
        /// regex syntax level.
        $f
    };
    (@body $it:tt => $self:expr, $re:expr, $iter:expr) => {
        let $crate::ConstContainer { inner, re_cache } = $self;
        let re = $crate::constant_container::probe_re($re, re_cache)?;
        $iter = inner
            .iter()
            .filter(|ptr| re.is_match(ptr.0.ident().to_string().as_bytes()));
    };
    (@filter_with => $iter:expr, $borrowed:expr) => {
        _ = $iter.map(Arc::downgrade).collect_into($borrowed.buffer())
    };
    (@filter => $iter:expr) => {
        $iter.cloned().collect::<Vec<_>>()
    };
    // This subtree requires so much repetition because once it recurses to the branch that
    // generates the docstrings, the macro system seems to require the item to already be there, not
    // requiring further macro invocations to actually resolve to an item. Otherwise, the docstrings
    // don't get attached to it, and you get the lints against free docstrings.
    () => {
        filter_impl! { @doc filter_with {
            pub fn filter_with(
                &mut self,
                re: impl AsRef<str>,
                borrowed_container: &mut BorrowedContainer,
            ) -> Result<(), FilterError> {
                let iter;
                filter_impl! { @body filter_with => self, re, iter }

                Ok(filter_impl! { @filter_with => iter, borrowed_container })
            }
        } }

        filter_impl! { @doc filter {
            pub fn filter(
                &mut self,
                re: impl AsRef<str>,
            ) -> Result<BorrowedContainer, FilterError> {
                let iter;
                filter_impl! { @body filter => self, re, iter }

                Ok(BorrowedContainer::from_container(&filter_impl! { @filter => iter }))
            }
        } }
    };
}

impl ConstContainer {
    pub(crate) const MAX_RE_POWER: u8 = 20;

    pub(crate) const DEPRECATION_NOTICE: &str = "This constant, among others often used in C for \
                                                 the purposes of denoting the latest value or \
                                                 limit in a set of constants, has been \
                                                 deprecated. See #3131 for details and discussion.";

    pub(crate) fn new(inner: Vec<Const>) -> Self {
        Self {
            inner: inner
                .into_iter()
                .map(|constant| Arc::new((constant, false)))
                .collect(),
            re_cache: HashMap::new(),
        }
    }

    /// Returns a borrowed container that can be reused across calls to
    /// [`filter_with()`].
    ///
    /// This can be paired up with `filter_with()`, which is a more efficient
    /// alternative to [`filter()`], as it will reuse a given borrowed view
    /// instead of allocating a new one.
    ///
    /// The reason why there's two and not just one of these filtering routines
    /// is that there's a tradeoff in allocations and potential allocations.
    /// `filter_with()` requires a borrowed container, which may be obtained
    /// through either one of `filter()`'s allocated container, or `borrowed()`.
    /// If sourced from the former, the container may reallocate if some future
    /// filtering operation requires resolving a larger amount of constants. If
    /// sourced from the latter, the container is guaranteed to never reallocate
    /// as it will always have a capacity equivalent to that of a borrowed
    /// container returned by a filtering operation with an input regex `.*`.
    ///
    /// [`filter_with()`]: `crate::ConstContainer::filter_with()`
    /// [`filter()`]: `crate::ConstContainer::filter()`
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn borrowed(&self) -> BorrowedContainer {
        BorrowedContainer::from_container(&self.inner.clone())
    }

    filter_impl! {}

    /// Effects the changes registered thus far over the parsed constants to the
    /// on-disk `libc` codebase.
    ///
    /// This is a no-op if no constants have been changed, as the implementation
    /// will only write to disk whichever constants have had their deprecation
    /// status modified from the time they were loaded into memory.
    ///
    /// Note this makes changes to the current codebase, and attempts to keep
    /// the rewritten files fairly well-behaved when it comes to formatting
    /// guarantees when _manually_ running `rustfmt` on the codebase.
    ///
    /// # Errors
    ///
    /// Fails if some I/O-bound operation fails while writing to disk, or if any
    /// one of (1) parsing the existing file from the codebase, or (2)
    /// formatting that file once the changes are made, fails.
    #[cfg_attr(debug_assertions, tracing::instrument(skip_all))]
    pub async fn effect_changes(&self) -> Result<(), MakeChangesError> {
        // NOTE: the pointer here is actually meant to hold a reference into a given
        // `Const`. It just so happens that `tokio`'s tasks require a `'static` lifetime
        // on the futures they run, but we have invariant references to `'static'`. This
        // is solved by way of raw pointers, which themselves require an unsafe impl to
        // be accepted as thread-safe. It is sound to do this because all of the tasks
        // spawned here are awaited before the function returns.
        #[repr(transparent)]
        struct ThreadedPtr<T>(*const T);

        unsafe impl<T> Send for ThreadedPtr<T> {}

        unsafe impl<T> Sync for ThreadedPtr<T> {}

        impl<T> Deref for ThreadedPtr<T> {
            type Target = *const T;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<T> DerefMut for ThreadedPtr<T> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        let mut task_pool = JoinSet::new();

        for constant in self
            .inner
            .iter()
            .map(|ptr| (&ptr.0, &ptr.1))
            .filter_map(|(constant, modified)| if *modified { Some(constant) } else { None })
            .map(|constant| &raw const *constant)
            .map(ThreadedPtr)
        {
            task_pool.spawn(async move {
                // NOTE: we only extract `source` from `constant` because other fields are
                // `!Send` and we prefer to keep that from going across await points.
                let source = unsafe { &constant.as_ref_unchecked().source() };

                let contents = fs::read_to_string(source).await.map_err(|inner| {
                    MakeChangesErrorRepr::IoBound(IoBoundChanges::new(
                        (*source).clone(),
                        inner,
                        ChangesKind::fetch(),
                    ))
                })?;

                // NOTE: this is purposefully sandwiched between await points because it handles
                // `!Send` types.
                let modified_file = {
                    let mut file = syn::parse_file(&contents)
                        .map_err(|_| MakeChangesErrorRepr::Parse((*source).into()))?;

                    let (ref_ident, deprecated, ref_span) = {
                        let constant = unsafe { constant.as_ref_unchecked() };

                        (constant.ident(), constant.is_deprecated(), constant.span())
                    };

                    file.items
                        .iter_mut()
                        .filter_map(|item| {
                            if item.span().start() == ref_span
                                && let Item::Const(ItemConst { attrs, ident, .. }) = item
                                && ident == ref_ident
                                && deprecated
                            {
                                Some(attrs)
                            } else {
                                None
                            }
                        })
                        .for_each(|attrs| {
                            attrs.push(deprecate!(Self::DEPRECATION_NOTICE));
                        });

                    prettyplease::unparse(&file)
                };

                fs::write(source, modified_file).await.map_err(|inner| {
                    MakeChangesErrorRepr::IoBound(IoBoundChanges::new(
                        (*source).clone(),
                        inner,
                        ChangesKind::save(),
                    ))
                })?;

                Ok::<_, MakeChangesError>(())
            });
        }

        // NOTE: we cleanly shut down the tasks instead of just aborting them by letting
        // the task pool drop because we're handling the FS in each task, and it's best
        // to be safe than to be sorry.
        while let Some(res) = task_pool.join_next().await {
            match res {
                Ok(Err(err)) => {
                    task_pool.shutdown().await;

                    return Err(err);
                }
                Err(err) => {
                    task_pool.shutdown().await;

                    return Err(MakeChangesErrorRepr::Other(err.into()).into());
                }
                _ => (),
            }
        }

        Ok(())
    }
}

fn probe_re(
    re: impl AsRef<str>,
    cache: &mut HashMap<String, Regex>,
) -> Result<&Regex, FilterError> {
    // NOTE: yes, this is ugly and could be made into an `if-let`, but borrowck
    // complains that the shared reference we get in the condition with
    // `cache.get()` makes it impossible to fetch an exclusive reference in the
    // `else` branch.
    if cache.get(re.as_ref()).is_none() {
        cache.insert(
            re.as_ref().to_string(),
            build_re(&re).map_err(|err| match err {
                regex::Error::Syntax(_) => {
                    FilterErrorRepr::RegexSyntax(re.as_ref().to_owned().into())
                }
                regex::Error::CompiledTooBig(_) => {
                    FilterErrorRepr::RegexTooBig(re.as_ref().to_owned().into())
                }
                _ => unimplemented!("An unhandled case in the `regex::Error` type has appeared!"),
            })?,
        );

        return Ok(cache.get(re.as_ref()).unwrap());
    }

    Ok(cache.get(re.as_ref()).unwrap())
}

fn build_re(re: impl AsRef<str>) -> Result<Regex, regex::Error> {
    let mut size_power: u8 = 11;

    loop {
        match RegexBuilder::new(re.as_ref())
            .unicode(false)
            .size_limit(2_usize.pow(u32::from(size_power)))
            .case_insensitive(true)
            .build()
        {
            Err(regex::Error::CompiledTooBig(_)) if size_power < ConstContainer::MAX_RE_POWER => {
                size_power += 1;
            }
            other => break other,
        }
    }
}
