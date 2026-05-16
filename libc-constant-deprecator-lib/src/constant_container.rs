use std::{collections::HashMap, sync::Arc};

use regex::bytes::{Regex, RegexBuilder};
use syn::{Attribute, ItemConst};
use tokio::{fs, task::JoinSet};
use tracing::info;

use crate::{
    BorrowedContainer, ChangesKind, Const, FilterError, FilterErrorRepr, IoBoundChanges,
    MakeChangesError, MakeChangesErrorRepr, deprecate, traverse_constants,
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
        let re = $crate::constant_container::probe_re(&$re, re_cache)?;

        // NOTE: when issuing a regex that matches all symbols, we prefer to
        // keep the logs relatively clean and thus skip reporting those events.
        if $re.as_ref() != ".*" {
            ::tracing::info!(?re);
        }

        $iter = inner
            .iter()
            .map(|ptr| {
                if re.is_match(ptr.0.ident().to_string().as_bytes())
                    .then(|| if $re.as_ref() != ".*" {
                        ::tracing::info!(matched_symbol = %ptr.0.ident())
                    })
                    .is_some()
                {
                    ptr.into()
                } else {
                    None
                }
            });

        ::tracing::info!("filtering done");
    };
    (@debug => $span:expr, $borrowed:expr) => {
        #[cfg(debug_assertions)]
        {
            let span = ::tracing::info_span!($span);

            $borrowed.traverse(|constant, init_state| {
                if constant.ident().to_string().contains("MINI") {
                    ::tracing::info!(
                        parent: &span,
                        constant = %constant.ident(),
                        init_state_during_filtering = init_state,
                    );
                }
            });
        }
    };
    (@filter_with => $iter:expr, $borrowed:expr) => {{
        filter_impl!(@debug => "preemptive_information_prefilling", $borrowed);

        $iter
            .map(|maybe_matched| maybe_matched.map(Arc::downgrade))
            .zip($borrowed.buffer_mut())
            .for_each(|(ptr, buf_elem)| {
                if let Some(ptr) = ptr {
                    *buf_elem = $crate::borrowed!(ptr);
                } else {
                    *buf_elem = $crate::borrowed!();
                }
            });

        filter_impl!(@debug => "preemptive_information_postfilling", $borrowed);

        ::tracing::info!("gathering_done");
    }};
    (@filter => $iter:expr) => {{
        // NOTE: when the filering operation is meant to create a whole new
        // non-owning container, then we don't provide all values wrapped in
        // `Option`s because there is no initialization state to keep track of
        // in the borrowed container.
        let out = $iter.filter_map(|ptr| ptr).cloned().collect::<Vec<_>>();

        ::tracing::info!("gathering done");

        out
    }};
    // This subtree requires so much repetition because once it recurses to the
    // branch that generates the docstrings, the macro system seems to require
    // the item to already be there, not requiring further macro invocations to
    // actually resolve to an item. Otherwise, the docstrings don't get attached
    // to it, and you get the lints against free docstrings.
    () => {
        filter_impl! { @doc filter_with {
            #[cfg_attr(debug_assertions, tracing::instrument(skip_all, err(level = "info")))]
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
            #[cfg_attr(debug_assertions, tracing::instrument(skip_all, err(level = "info")))]
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
    #[cfg_attr(debug_assertions, tracing::instrument(skip_all, err(level = "info")))]
    pub async fn effect_changes(&self) -> Result<(), MakeChangesError> {
        let mut task_pool = JoinSet::new();

        self.inner
            .iter()
            .filter(|ptr| ptr.1)
            .cloned()
            .for_each(|symbol| {
                task_pool.spawn(async move {
                    let constant = &symbol.0;

                    info!(constant_to_save = %constant.ident(), state = constant.is_deprecated());

                    // NOTE: this is purposefully sandwiched between await points because it handles
                    // `File: !Send`.
                    let modified_file = prettyplease::unparse(
                        &traverse_constants(constant.path(), |ItemConst { attrs, .. }| {
                            // TODO: ensure the constant we are iterating through is, indeed, the
                            // constant that we are iterating through in this inner closure.
                            if constant.is_deprecated() {
                                attrs.push(deprecate!(Self::DEPRECATION_NOTICE));
                            } else {
                                let Some(attr_to_remove) = attrs
                                    .iter()
                                    .map(Attribute::path)
                                    .position(|attr_ident| attr_ident.is_ident("deprecated"))
                                else {
                                    return;
                                };

                                attrs.swap_remove(attr_to_remove);
                            }
                        })
                        .await
                        .map_err(|err| {
                            err.with_consuming_io(|io_error| {
                                MakeChangesErrorRepr::IoBound(IoBoundChanges::new(
                                    constant.path().clone(),
                                    io_error,
                                    ChangesKind::fetch(),
                                ))
                            })
                            .either(
                                |err| err,
                                |_| MakeChangesErrorRepr::Parse(constant.path().clone().into()),
                            )
                        })?,
                    );

                    fs::write(constant.path(), modified_file)
                        .await
                        .map_err(|inner| {
                            MakeChangesErrorRepr::IoBound(IoBoundChanges::new(
                                constant.path().clone(),
                                inner,
                                ChangesKind::save(),
                            ))
                        })?;

                    Ok::<_, MakeChangesError>(())
                });
            });

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
