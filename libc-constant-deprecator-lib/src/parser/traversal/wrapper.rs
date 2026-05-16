use syn::{ItemConst, ItemMacro};

use self::internal::WrapperConstRepr;

mod internal;

#[derive(Debug)]
pub(super) struct WrapperConst<'a> {
    inner: WrapperConstRepr<'a>,
}

impl WrapperConst<'_> {
    pub(super) fn with_constant(&mut self, f: impl FnOnce(&mut ItemConst)) -> &mut Self {
        let Self { inner } = self;

        inner.with_constant(f);

        self
    }

    pub(super) fn with_macro(&mut self, f: impl FnOnce(&mut ItemMacro)) -> &mut Self {
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
