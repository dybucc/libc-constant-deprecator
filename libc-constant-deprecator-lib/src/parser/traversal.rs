use std::path::Path;

use quote::ToTokens;
use syn::{File, Item, ItemConst, ItemMacro};
use tokio::fs;

use self::{macro_iterator::MacroIter, wrapper::WrapperConst};
use crate::ParseError;

mod macro_iterator;
mod wrapper;

pub(crate) async fn traverse_constants(
    file: impl AsRef<Path>,
    mut f: impl FnMut(&mut ItemConst),
) -> Result<File, ParseError> {
    syn::parse_file(&fs::read_to_string(file).await?)
        .map(|mut file| {
            file.items
                .iter_mut()
                .filter_map(|item| {
                    // NOTE: unfortunately, we cannot destructure the `ItemMacro` with bindings to
                    // the record's elements, and to also keep a mutable reference into the
                    // `ItemMacro`. But we can destructure an existing binding to the `ItemMacro`
                    // that takes fields by reference.
                    match item {
                        Item::Const(constant) => WrapperConst::from(constant),
                        Item::Macro(mac @ ItemMacro { .. })
                            if let ItemMacro {
                                ref ident,
                                mac: ref inner_mac,
                                ..
                            } = *mac
                                && ident.is_none()
                                && inner_mac.path.is_ident("cfg_if") =>
                        {
                            WrapperConst::from(mac)
                        }
                        _ => return None,
                    }
                    .into()
                })
                .for_each(|mut item| {
                    item.with_constant(|constant| f(constant)).with_macro(
                        |ItemMacro { mac, .. }| {
                            let Ok(mut constant_iter) = mac.parse_body::<MacroIter>() else {
                                return;
                            };

                            constant_iter
                                .by_ref()
                                .for_each(|mut item| f(&mut item.get()));

                            mac.tokens = constant_iter.into_container().into_token_stream();
                        },
                    );
                });

            file
        })
        .map_err(Into::into)
}
