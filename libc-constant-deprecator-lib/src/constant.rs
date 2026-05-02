use std::path::PathBuf;

use proc_macro2::LineColumn;
use syn::{Attribute, Ident, ItemConst, spanned::Spanned};

// TODO(perf): instead of only storing the identifier of the constant, it may be
// a better idea to store an enumeration akin to `std::Cow`, where the
// identifier is the base type, and if at some point we require converting one
// of them into strings, it starts storing them as strings instead.
/// In-memory representation of parsed constants produced as part of
/// [`parse_constants()`], within [`ConstContainer`]s.
///
/// This type contains additional information on the file span, and on whether
/// the constant item has been marked deprecated.
///
/// [`parse_constants()`]: `crate::parse_constants()`
/// [`ConstContainer`]: `crate::ConstContainer`
#[derive(Debug, Clone)]
pub struct Const {
    pub(crate) ident: Ident,
    pub(crate) deprecated: bool,
    pub(crate) span: LineColumn,
    pub(crate) source: PathBuf,
}

macro_rules! impl_doc {
    ($($it:tt)+) => {
        $(
/// These are necessary for some async stuff that has collections of `Const`s
/// passed between threads. The reason why this is sound is that the backing
/// `Ident` in the `ident` field (which is the one causing
/// `Const: !Send + !Sync`) is actually the `fallback::Ident` in `proc_macro2`,
/// which itself is thread-safe. This isn't a public implementation detail, but
/// it is the way that crate handles the `Ident` type when outside the context
/// of a proc-macro. This crate is not a proc-macro, so we can trust the `Ident`
/// type is _not_ the thread-unsafe variant in the `proc_macro`
/// compiler-provided crate.
unsafe impl $it for Const {}
        )+
    };
}

impl_doc! { Send Sync }

impl Const {
    pub(crate) fn from_item(item: ItemConst, source: PathBuf) -> Self {
        Self {
            deprecated: item
                .attrs
                .iter()
                .map(Attribute::path)
                .any(|attr_name| attr_name.is_ident("deprecated")),
            span: item.span().start(),
            ident: item.ident,
            source,
        }
    }

    pub(crate) fn deprecated(&mut self, yes: bool) {
        self.deprecated = yes;
    }
}
