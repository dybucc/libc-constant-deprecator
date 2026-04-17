use std::path::Path;

use proc_macro2::Span;
use syn::{Ident, Item, ItemConst, ItemMacro, Macro};

use crate::{Const, SourceFile};

pub(crate) mod macro_parser;

pub(crate) use macro_parser::MacroParser;

// NOTE: this only scans through top-level module items and the `cfg_if` macro
// because there's no inner scopes (e.g. inherent impls and traits) that contain
// constants that ought be deprecated, and because the `cfg_if` macro is the
// only one in which constant items relevant to target platforms are declared
// (barring the `c_enum` macro, the generated constants of which we don't
// require deprecating.)
#[expect(
    clippy::must_use_candidate,
    reason = "It's not a bug not to use the result of this routine."
)]
pub fn parse_constants(files: &[SourceFile]) -> Vec<Const> {
    let mut parsed_constants = Vec::with_capacity(files.len());

    for SourceFile { inner, source } in files {
        let mut file_constants = Vec::new();

        for item in &inner.items {
            if let Some(mut constants) = match item {
                Item::Const(constant) => process_constant(constant, source).into(),
                Item::Macro(ItemMacro {
                    mac: mac @ Macro { path, .. },
                    ..
                }) if path.is_ident(&Ident::new("cfg_if", Span::call_site())) => {
                    process_macro(mac).into()
                }
                _ => None,
            } {
                file_constants.append(&mut constants);
            }
        }

        parsed_constants.append(&mut file_constants);
    }

    parsed_constants
}

#[inline]
pub(crate) fn process_constant(constant: &ItemConst, source: impl AsRef<Path>) -> Vec<Const> {
    vec![Const::from_item(
        constant.clone(),
        source.as_ref().to_owned(),
    )]
}

pub(crate) fn process_macro(mac: &Macro) -> Vec<Const> {
    mac.parse_body::<MacroParser>()
        .expect("macro body couldn't be parsed correctly; time to check the implementation again")
        .into_vec()
}
