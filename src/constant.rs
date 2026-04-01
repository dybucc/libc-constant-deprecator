use std::path::{Path, PathBuf};

use syn::{Ident, ImplItemConst, ItemConst, TraitItemConst};

#[derive(Debug, Clone)]
pub struct Const {
  #[expect(unused, reason = "It may be used in the future.")]
  pub(crate) repr:       ConstRepr,
  pub(crate) ident:      Ident,
  pub(crate) source:     PathBuf,
  pub(crate) deprecated: bool,
}

#[expect(
  unused,
  reason = "It may be used in the future, in the `repr` field of `Const`."
)]
#[derive(Debug, Clone)]
pub(crate) enum ConstRepr {
  Item(ItemConst),
  Trait(TraitItemConst),
  Impl(ImplItemConst),
  File,
}

impl Const {
  pub(crate) fn from_item(item: &ItemConst, source: impl AsRef<Path>) -> Self {
    let ident = item.ident.clone();

    Self {
      repr: ConstRepr::Item(item.clone()),
      ident,
      source: source.as_ref().to_owned(),
      deprecated: false,
    }
  }

  pub(crate) fn from_trait(
    item: &TraitItemConst,
    source: impl AsRef<Path>,
  ) -> Self {
    let ident = item.ident.clone();

    Self {
      repr: ConstRepr::Trait(item.clone()),
      ident,
      source: source.as_ref().to_owned(),
      deprecated: false,
    }
  }

  pub(crate) fn from_impl(
    item: &ImplItemConst,
    source: impl AsRef<Path>,
  ) -> Self {
    let ident = item.ident.clone();

    Self {
      repr: ConstRepr::Impl(item.clone()),
      ident,
      source: source.as_ref().to_owned(),
      deprecated: false,
    }
  }

  pub(crate) fn from_file(ident: Ident, source: impl AsRef<Path>) -> Self {
    Self {
      repr: ConstRepr::File,
      ident,
      source: source.as_ref().to_owned(),
      deprecated: false,
    }
  }

  pub(crate) fn deprecated(&mut self, yes: bool) { self.deprecated = yes }
}
