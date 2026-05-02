use std::path::Path;

use syn::{Item, ItemConst, ItemMacro, Macro};

use crate::{Const, ConstContainer, MacroParser, SourceFile};

pub(crate) mod macro_parser;

/// Parses constants provided a collection of [`SourceFile`]s yield from
/// [`scan_files()`].
///
/// This routine will recursively traverse all files and scan for constant items
/// at module-level scope and within the `cfg_if!` macro body. These are the
/// only scopes in which such items can be found in the `libc` codebase, beyond
/// those declared for auxiliary purposes in the `c_enum!` macro (itself from
/// `libc`.)
///
/// [`scan_files()`]: `crate::scan_files()`
#[expect(
    clippy::must_use_candidate,
    reason = "It's not a bug not to use the result of this routine."
)]
pub fn parse_constants(files: &[SourceFile]) -> ConstContainer {
    ConstContainer::new(files.iter().fold(
        Vec::new(),
        |mut parsed_constants, SourceFile { inner, source }| {
            parsed_constants.append(
                &mut inner
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        Item::Const(constant) => process_constant(constant, source).into(),
                        Item::Macro(ItemMacro {
                            mac: mac @ Macro { path, .. },
                            ..
                        }) if path.is_ident("cfg_if") => process_macro(mac, source).into(),
                        _ => None,
                    })
                    .fold(Vec::new(), |mut file_constants, mut constants| {
                        file_constants.append(&mut constants);

                        file_constants
                    }),
            );

            parsed_constants
        },
    ))
}

pub(crate) fn process_constant(constant: &ItemConst, source: impl AsRef<Path>) -> Vec<Const> {
    vec![Const::from_item(
        constant.clone(),
        source.as_ref().to_owned(),
    )]
}

pub(crate) fn process_macro(mac: &Macro, source: impl AsRef<Path>) -> Vec<Const> {
    mac.parse_body::<MacroParser>()
        .expect("macro body couldn't be parsed correctly; time to check the implementation again")
        .into_vec(source.as_ref())
}
