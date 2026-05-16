use std::{iter, path::PathBuf};

use syn::{Item, ItemConst, ItemMacro, Macro};
use tracing::info;

use self::macro_parser::MacroParser;
use crate::{Const, ConstContainerBuilder, SourceFile};

pub(crate) mod const_container_builder;
mod macro_parser;
pub(crate) mod traversal;

#[cfg_attr(debug_assertions, tracing::instrument(skip_all))]
pub(crate) fn parse_constants(file: &SourceFile) -> ConstContainerBuilder {
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
