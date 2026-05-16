use syn::{ItemConst, ItemMacro};

#[derive(Debug)]
pub(super) enum WrapperConstRepr<'a> {
    Item(&'a mut ItemConst),
    MacroItem(&'a mut ItemMacro),
}

impl<'a> WrapperConstRepr<'a> {
    pub(super) fn with_constant(&mut self, f: impl FnOnce(&mut ItemConst)) {
        if let Self::Item(constant) = self {
            f(constant);
        }
    }

    pub(super) fn with_macro(&mut self, f: impl FnOnce(&mut ItemMacro)) {
        if let Self::MacroItem(mac) = self {
            f(mac);
        }
    }

    pub(super) fn from_item(item: &'a mut ItemConst) -> Self {
        Self::Item(item)
    }

    pub(super) fn from_macro(mac: &'a mut ItemMacro) -> Self {
        Self::MacroItem(mac)
    }
}
