use std::{collections::HashMap, fs, process::Command};

use regex::bytes::{Regex, RegexBuilder};
use syn::{Item, ItemConst, spanned::Spanned};

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
    pub(crate) const DEPRECATION_NOTICE: &str = "This constant, among others often used in C for \
                                                 the purposes of denoting the latest value or \
                                                 limit in a set of constants, has been \
                                                 deprecated. See #3131 for details and discussion.";

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
                build_re(&re).map_err(|_| FilterError::RegexCompilation {
                    input_str: re.as_ref().to_string(),
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

    pub fn effect_changes(&self) -> Result<(), MakeChangesError> {
        for Const {
            ident: ref_ident,
            deprecated,
            span: ref_span,
            source,
        } in self
            .inner
            .iter()
            .filter_map(|(constant, modified)| if *modified { Some(constant) } else { None })
        {
            let mut file = syn::parse_file(&fs::read_to_string(source).map_err(|inner| {
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

            fs::write(source, prettyplease::unparse(&file)).map_err(|inner| {
                MakeChangesError::IoBound(IoBoundChanges {
                    origin: ChangesSrc::SaveOp(source.clone()),
                    inner,
                })
            })?;
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
    const MAX_POWER: u8 = 20;

    let mut size_power: u8 = 11;

    loop {
        match RegexBuilder::new(re.as_ref())
            .unicode(false)
            .size_limit(2_usize.pow(u32::from(size_power)))
            .case_insensitive(true)
            .build()
        {
            Err(regex::Error::CompiledTooBig(_)) if size_power < MAX_POWER => size_power += 1,
            other => break other,
        }
    }
}
