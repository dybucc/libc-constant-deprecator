use std::path::PathBuf;

use proc_macro2::Span;
use syn::{Ident, ItemConst};

#[derive(Debug, Clone)]
pub struct Const {
    pub(crate) ident: Ident,
    pub(crate) deprecated: bool,
    pub(crate) span: Span,
}

impl Const {
    pub(crate) fn from_item(item: ItemConst, source: PathBuf) -> Self {
        Self {
            ident: item.ident,
            source,
            deprecated: false,
        }
    }

    pub(crate) fn from_file(ident: Ident, source: PathBuf) -> Self {
        Self {
            ident,
            source,
            deprecated: false,
        }
    }

    pub(crate) fn deprecated(&mut self, yes: bool) {
        self.deprecated = yes;
    }
}
