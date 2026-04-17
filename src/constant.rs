use std::path::PathBuf;

use proc_macro2::LineColumn;
use syn::{Ident, ItemConst, spanned::Spanned};

// NOTE: we don't hold onto a full `proc_macro2::Span` here because recovering
// that information from a file (the file this utility embeds its constant
// information on, to be precise) would be impossible as a `Span` can only be
// created from `syn`'s parsing facilities. And keeping two different types for
// file-fetched constants and parse-sourced constants is just a hassle.
#[derive(Debug, Clone)]
pub struct Const {
    pub(crate) ident: Ident,
    pub(crate) deprecated: bool,
    pub(crate) span: LineColumn,
    pub(crate) source: PathBuf,
}

impl Const {
    pub(crate) fn from_item(item: ItemConst, source: PathBuf) -> Self {
        Self {
            span: item.span().start(),
            ident: item.ident,
            deprecated: false,
            source,
        }
    }

    pub(crate) fn from_file(
        ident: Ident,
        deprecated: bool,
        source: PathBuf,
        span: LineColumn,
    ) -> Self {
        Self {
            ident,
            deprecated,
            span,
            source,
        }
    }

    pub fn deprecated(&mut self, yes: bool) {
        self.deprecated = yes;
    }
}
