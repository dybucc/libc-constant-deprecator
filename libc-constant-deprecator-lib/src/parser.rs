use std::{iter, path::PathBuf};

use syn::{Item, ItemConst, ItemMacro, Macro};
use tracing::info;

use crate::{Const, IrContainer, MacroParser, SourceFile};

pub(crate) mod ir_container;
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
#[tracing::instrument(skip_all)]
pub(crate) fn parse_constants(file: &SourceFile) -> IrContainer {
    macro_rules! cast {
        ($iter:expr) => {{ Box::new($iter) as Box<dyn Iterator<Item = Const>> }};
    }

    let syn_file = file.parsed_file();
    let path = file.path();

    info!(file_to_parse = %path.as_ref().display());

    syn_file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Const(constant) => {
                info!("found constant symbol");

                cast!(process_constant(constant.clone(), path.as_ref().to_owned())).into()
            }
            Item::Macro(ItemMacro {
                mac: mac @ Macro { path: mac_path, .. },
                ..
            }) if mac_path.is_ident("cfg_if") => {
                info!("found macro");

                cast!(process_macro(mac, path.as_ref().to_owned())).into()
            }
            _ => None,
        })
        .flatten()
        .collect()
}

fn process_constant(constant: ItemConst, source: PathBuf) -> impl Iterator<Item = Const> {
    iter::once(Const::from_item(constant, source))
}

fn process_macro(mac: &Macro, source: PathBuf) -> impl Iterator<Item = Const> {
    mac.parse_body::<MacroParser>()
        .expect("macro body couldn't be parsed correctly; time to check the implementation")
        .into_iter(source)
}
