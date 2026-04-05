use std::path::{Path, PathBuf};

use syn::{Ident, ImplItemConst, ItemConst, TraitItemConst};

#[derive(Debug, Clone)]
pub struct Const {
  pub(crate) ident:      Ident,
  pub(crate) source:     PathBuf,
  pub(crate) deprecated: bool,
}

impl Const {
  pub(crate) fn from_item(item: &ItemConst, source: impl AsRef<Path>) -> Self {
    let ident = item.ident.clone();

    Self { ident, source: source.as_ref().to_owned(), deprecated: false }
  }

  pub(crate) fn from_trait(
    item: &TraitItemConst,
    source: impl AsRef<Path>,
  ) -> Self {
    let ident = item.ident.clone();

    Self { ident, source: source.as_ref().to_owned(), deprecated: false }
  }

  pub(crate) fn from_impl(
    item: &ImplItemConst,
    source: impl AsRef<Path>,
  ) -> Self {
    let ident = item.ident.clone();

    Self { ident, source: source.as_ref().to_owned(), deprecated: false }
  }

  pub(crate) fn from_file(ident: Ident, source: impl AsRef<Path>) -> Self {
    Self { ident, source: source.as_ref().to_owned(), deprecated: false }
  }

  pub(crate) fn deprecated(&mut self, yes: bool) { self.deprecated = yes }
}
