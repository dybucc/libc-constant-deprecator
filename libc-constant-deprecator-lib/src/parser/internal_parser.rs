use std::path::Path;

use syn::{File, Item, ItemConst, ItemMacro};
use tokio::fs;

use crate::ParseError;

#[derive(Debug)]
pub(crate) struct WrapperConst<'a> {
    inner: WrapperConstRepr<'a>,
}

impl WrapperConst<'_> {
    fn with_constant(&mut self, f: impl FnOnce(&mut ItemConst)) -> &mut Self {
        let Self { inner } = self;

        inner.with_constant(f);

        self
    }

    fn with_macro(&mut self, f: impl FnOnce(&mut ItemMacro)) -> &mut Self {
        let Self { inner } = self;

        inner.with_macro(f);

        self
    }
}

impl<'a> From<&'a mut ItemConst> for WrapperConst<'a> {
    fn from(value: &'a mut ItemConst) -> Self {
        Self {
            inner: WrapperConstRepr::from_item(value),
        }
    }
}

impl<'a> From<&'a mut ItemMacro> for WrapperConst<'a> {
    fn from(value: &'a mut ItemMacro) -> Self {
        Self {
            inner: WrapperConstRepr::from_macro(value),
        }
    }
}

#[derive(Debug)]
enum WrapperConstRepr<'a> {
    Item(&'a mut ItemConst),
    MacroItem(&'a mut ItemMacro),
}

impl<'a> WrapperConstRepr<'a> {
    fn with_constant(&mut self, f: impl FnOnce(&mut ItemConst)) {
        if let Self::Item(constant) = self {
            f(constant)
        }
    }

    fn with_macro(&mut self, f: impl FnOnce(&mut ItemMacro)) {
        if let Self::MacroItem(mac) = self {
            f(mac)
        }
    }

    fn from_item(item: &'a mut ItemConst) -> Self {
        Self::Item(item)
    }

    fn from_macro(mac: &'a mut ItemMacro) -> Self {
        Self::MacroItem(mac)
    }
}

pub(crate) async fn traverse_constants<T>(
    file: impl AsRef<Path>,
    mut f: impl FnMut(&mut ItemConst),
) -> Result<File, ParseError> {
    let mut file = syn::parse_file(&fs::read_to_string(file).await?)?;

    // TODO: see below for the `todo` string.
    file.items
        .iter_mut()
        .filter_map(|item| {
            match item {
                Item::Const(constant) => WrapperConst::from(constant),
                Item::Macro(mac @ ItemMacro { .. }) if mac.mac.path.is_ident("cfg_if") => {
                    WrapperConst::from(mac)
                }
                _ => return None,
            }
            .into()
        })
        .for_each(|mut item| {
            item.with_constant(|constant| f(constant))
                .with_macro(|mac| {
                    todo!(
                        "Implement macro parsing akin to the `MacroParser` type in the `super` \
                         module and provide a view into the potential constants that may have \
                         been parsed from that macro."
                    )
                });
        });

    todo!()
}
