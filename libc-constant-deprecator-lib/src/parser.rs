use std::{iter, path::Path};

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
#[tracing::instrument(skip_all, ret)]
pub fn parse_constants(files: &[SourceFile]) -> ConstContainer {
    macro_rules! cast {
        ($iter:expr) => {{ Box::new($iter) as Box<dyn Iterator<Item = Const>> }};
    }

    ConstContainer::new(files.iter().fold(
        Vec::new(),
        |mut parsed_constants, SourceFile { inner, source }| {
            parsed_constants.extend(
                inner
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        Item::Const(constant) => cast!(process_constant(constant, source)).into(),
                        Item::Macro(ItemMacro {
                            mac: mac @ Macro { path, .. },
                            ..
                        }) if path.is_ident("cfg_if") => cast!(process_macro(mac, source)).into(),
                        _ => None,
                    })
                    .flatten(),
            );

            parsed_constants
        },
    ))
}

pub(crate) fn process_constant(
    constant: &ItemConst,
    source: impl AsRef<Path>,
) -> impl Iterator<Item = Const> {
    iter::once(Const::from_item(
        constant.clone(),
        source.as_ref().to_owned(),
    ))
}

pub(crate) fn process_macro(mac: &Macro, source: impl AsRef<Path>) -> impl Iterator<Item = Const> {
    mac.parse_body::<MacroParser>()
        .expect("macro body couldn't be parsed correctly; time to check the implementation")
        .into_iter(source.as_ref().to_owned())
}
