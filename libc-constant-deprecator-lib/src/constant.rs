use std::path::PathBuf;

use proc_macro2::LineColumn;
use syn::{Attribute, Ident, ItemConst, spanned::Spanned};

// TODO(perf): instead of only storing the identifier of the constant, it may be
// a better idea to store an enumeration akin to `std::Cow`, where the
// identifier is the base type, and if at some point we require converting one
// of them into strings, it starts storing them as strings instead.
/// In-memory representation of parsed constants within [`ConstContainer`]s
/// produced as part of [`scan()`].
///
/// This type contains additional information on the file span, and on whether
/// the constant item has been marked deprecated.
///
/// [`ConstContainer`]: `crate::ConstContainer`
/// [`scan()`]: `crate::scan()`
#[derive(Debug, Clone)]
pub struct Const {
    ident: Ident,
    deprecated: bool,
    span: LineColumn,
    source: PathBuf,
}

// NOTE: these are necessary for some async stuff that has collections of
// `Const`s passed between threads. The reason why this is sound is that the
// backing `Ident` in the `ident` field (which is the one causing `Const: !Send
// + !Sync`) is actually the `fallback::Ident` in `proc_macro2`, which itself is
// thread-safe. This isn't a public implementation detail, but it is the way
// that crate handles the `Ident` type when outside the context of a proc-macro.
// This crate is not a proc-macro, so we can trust the `Ident` type is _not_ the
// thread-unsafe variant in the `proc_macro` compiler-provided crate. The only
// other source of issues for use on this crate would be span information, which
// `proc-macro2` saves in TLS. This is mitigated by storing inline the span
// information, itself fetched in-place in the same thread where the file we
// source it from is being parsed (and having its TLS source map written to.)

unsafe impl Send for Const {}

unsafe impl Sync for Const {}

impl Const {
    /// Checks if the constant is marked deprecated or not.
    ///
    /// Note this check is not necessarily relative to its current state in the
    /// codebase, but rather to its current state in memory. It could very well
    /// be that the constant has been loaded and modified, but changes not
    /// effected to disk. In that instance, the symbol would report being
    /// deprecated if it wasn't on-disk, and undeprecated if it were so on-disk.
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn is_deprecated(&self) -> bool {
        self.deprecated
    }

    /// Fetches the identifier of the constant symbol.
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn ident(&self) -> &Ident {
        &self.ident
    }

    /// Fetches the path to the source file where this constant got extracted
    /// from.
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn path(&self) -> &PathBuf {
        &self.source
    }

    pub(crate) fn span(&self) -> LineColumn {
        self.span
    }
}

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

    // NOTE: the return value reports whether the symbol was previously deprecated,
    // such that the `BorrowedContainer` deprecation routines can more easily
    // determine whether the modified flag should be set for a given symbol.
    #[tracing::instrument]
    pub fn deprecate(&mut self, yes: bool) -> bool {
        match (self.deprecated, yes) {
            (true, false) => {
                self.deprecated = false;
                true
            }
            (false, true) => {
                self.deprecated = true;
                true
            }
            (true, true) | (false, false) => false,
        }
    }
}
