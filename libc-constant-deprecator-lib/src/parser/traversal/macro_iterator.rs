use syn::parse::{Parse, ParseStream};

use self::{container::MacroContainer, item_type::ItemType};

mod container;
mod item_type;

#[derive(Debug, Default)]
pub(super) struct MacroIter {
    inner: MacroContainer,
    current: usize,
}

impl MacroIter {
    pub(super) fn into_container(self) -> MacroContainer {
        let MacroIter { inner, .. } = self;

        inner
    }
}

impl Parse for MacroIter {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            inner: input.parse()?,
            ..Default::default()
        })
    }
}

impl Iterator for MacroIter {
    type Item = ItemType;

    fn next(&mut self) -> Option<Self::Item> {
        let MacroIter { inner, current } = self;
        let out = inner.get(*current)?;

        *current += 1;

        ItemType::new(out).into()
    }
}
