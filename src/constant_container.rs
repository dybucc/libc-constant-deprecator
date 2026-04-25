use std::{collections::HashMap, process::Command, sync::Arc};

use regex::bytes::{Regex, RegexBuilder};
use syn::{Item, ItemConst, spanned::Spanned};
use tokio::{fs, task::JoinSet};

use crate::{
    BorrowedContainer, ChangesSrc, Const, FilterError, IoBoundChanges, MakeChangesError, deprecate,
};

pub(crate) mod borrowed;

#[derive(Debug)]
pub struct ConstContainer {
    pub(crate) inner: Vec<(Const, bool)>,
    pub(crate) re_cache: HashMap<String, Regex>,
}

impl ConstContainer {
    pub(crate) const MAX_RE_POWER: u8 = 20;
    pub(crate) const DEPRECATION_NOTICE: &str = "This constant, among others often used in C for \
                                                 the purposes of denoting the latest value or \
                                                 limit in a set of constants, has been \
                                                 deprecated. See #3131 for details and discussion.";

    /// Filters out constants by identifier provided a regex matching against
    /// those identifiers.
    ///
    /// Returns a borrowed container that can bulk deprecate the matched
    /// constants at once, and remembers which constants have been modified to
    /// effect those changes to disk later on.
    ///
    /// # Errors
    ///
    /// Fails if either the regex failed to compile. This may be due to a byte
    /// size failure at the regex engine level, or due to a parsing failure
    /// at the regex syntax level.
    #[expect(
        clippy::missing_panics_doc,
        reason = "The panic code path is (at the time of writing) impossible to reach."
    )]
    pub fn filter(&mut self, re: impl AsRef<str>) -> Result<BorrowedContainer<'_>, FilterError> {
        let Self { inner, re_cache } = self;

        let re = if let Some(re) = re_cache.get(re.as_ref()) {
            re
        } else {
            re_cache.insert(
                re.as_ref().to_string(),
                build_re(&re).map_err(|err| match err {
                    regex::Error::Syntax(_) => {
                        FilterError::RegexSyntax(re.as_ref().to_owned().into())
                    }
                    regex::Error::CompiledTooBig(_) => {
                        FilterError::RegexTooBig(re.as_ref().to_owned().into())
                    }
                    _ => todo!(),
                })?,
            );

            re_cache.get(re.as_ref()).unwrap()
        };

        Ok(BorrowedContainer::new(
            inner
                .iter_mut()
                .filter(|(constant, _)| re.is_match(constant.ident.to_string().as_bytes()))
                .collect(),
        ))
    }

    /// Effects the changes registered thus far over the parsed constants to the
    /// on-disk `libc` codebase.
    ///
    /// This is a no-op if no constants have been changed, as the implementation
    /// will only write to disk whichever constants have been marked deprecated.
    ///
    /// # Errors
    ///
    /// Fails if some I/O-bound operation fails while writing to disk, or if any
    /// one of (1) parsing the existing file from the codebase, or (2)
    /// formatting that file once the changes are made fails.
    pub async fn effect_changes(&self) -> Result<(), MakeChangesError> {
        // NOTE: the pointer here is actually meant to hold a reference into each of the
        // fields of a given `Const`. It just so happens that `tokio`'s tasks require a
        // `'static` lifetime on the futures they run, but references can't easily be
        // coerced to the lifetime that rules them all. This is solved by way of raw
        // pointers, which themselves require an unsafe impl to be accepted as
        // thread-safe. It is sound to do this because the tasks are only accessed from
        // the thread in which they are runnning. On the lifetime side, it is also sound
        // for the moved newtype in the future not to be `'static` because all of the
        // tasks spawned here are awaited before the function returns.
        #[repr(transparent)]
        struct ThreadedPtr<T>(*const T);

        unsafe impl<T> Send for ThreadedPtr<T> {}
        unsafe impl<T> Sync for ThreadedPtr<T> {}

        let mut task_pool = JoinSet::new();

        for constant in self
            .inner
            .iter()
            .filter_map(|(constant, modified)| if *modified { Some(constant) } else { None })
        {
            let threaded_constant = Arc::new(ThreadedPtr(&raw const *constant));

            task_pool.spawn(async move {
                let Const {
                    ident: ref_ident,
                    deprecated,
                    span: ref_span,
                    source,
                } = unsafe { threaded_constant.0.as_ref_unchecked() };

                let mut file =
                    syn::parse_file(&fs::read_to_string(source).await.map_err(|inner| {
                        MakeChangesError::IoBound(IoBoundChanges {
                            origin: ChangesSrc::FetchOp(source.clone()),
                            inner,
                        })
                    })?)
                    .map_err(|_| MakeChangesError::Parse(source.clone().into()))?;

                // NOTE: the check for the span comes before the destructuring pattern because
                // otherwise borrowck complains about `item` already being exclusively borrowed
                // with the variable bindings of that pattern.
                file.items
                    .iter_mut()
                    .filter_map(|item| {
                        if item.span().start() == *ref_span
                            && let Item::Const(ItemConst { attrs, ident, .. }) = item
                            && ident == ref_ident
                            && *deprecated
                        {
                            Some(attrs)
                        } else {
                            None
                        }
                    })
                    .for_each(|attrs| attrs.push(deprecate!(ConstContainer::DEPRECATION_NOTICE)));

                fs::write(source, prettyplease::unparse(&file))
                    .await
                    .map_err(|inner| {
                        MakeChangesError::IoBound(IoBoundChanges {
                            origin: ChangesSrc::SaveOp(source.clone()),
                            inner,
                        })
                    })?;

                Ok::<_, MakeChangesError>(())
            });
        }

        while let Some(res) = task_pool.join_next().await {
            // TODO: map the error to this routine's error and make the above future
            // thread-safe by wrapping the `syn::File` in a newtype with `unsafe impl`s for
            // `Send` and `Sync`.
            res?;
        }

        // NOTE: this may require some tweaking to make the changes to the codebase
        // comply with those required by the `style_check.py` script. Either way, it
        // should be left to the contributor's discretion to have the right formatting
        // applied to the codebase before submitting a PR with the deprecation changes.
        Command::new("cargo")
            .args(["fmt"])
            .status()
            .map(|status| status.exit_ok().map_err(|_| MakeChangesError::Format))
            .map_err(|_| MakeChangesError::Format)??;

        Ok(())
    }
}

pub(crate) fn build_re(re: impl AsRef<str>) -> Result<Regex, regex::Error> {
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
