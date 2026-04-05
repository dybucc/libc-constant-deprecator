use std::{iter, path::Path};

use itertools::Itertools;
use proc_macro2::TokenStream;
use syn::{
  ImplItem,
  ImplItemConst,
  ImplItemMacro,
  Item,
  ItemConst,
  ItemImpl,
  ItemMod,
  ItemTrait,
  Macro,
  TraitItem,
  TraitItemConst,
};

use crate::{Const, SourceFile};

#[expect(
  clippy::must_use_candidate,
  reason = "It's not a bug not to use the result of this routine."
)]
pub fn parse_constants(files: &[SourceFile]) -> Vec<Const> {
  files.iter().fold(
    Vec::with_capacity(files.len()),
    |mut parsed_constants, SourceFile { contents, source }| {
      (
        parsed_constants.append(
          &mut contents
            .items
            .iter()
            .filter_map(|item| {
              match item {
                | Item::Const(constant) => process_constant(constant, source),
                | Item::Impl(impl_block) =>
                  process_impl_block(impl_block, source),
                | Item::Trait(trait_block) =>
                  process_trait_block(trait_block, source),
                | Item::Mod(ItemMod {
                  content: Some((_, contents)), ..
                }) => process_mod_block(contents, source),
                | _ => return None,
              }
              .into()
            })
            .fold(Vec::new(), |mut constants, mut item| {
              (constants.append(&mut item), constants).1
            }),
        ),
        parsed_constants,
      )
        .1
    },
  )
}

pub(crate) fn process_constant(
  constant: &ItemConst,
  source: impl AsRef<Path>,
) -> Vec<Const> {
  vec![Const::from_item(constant, source)]
}

pub(crate) fn process_impl_block(
  block: &ItemImpl,
  source: impl AsRef<Path>,
) -> Vec<Const> {
  #[derive(Debug)]
  enum Unifier<'a, C: ConstConvertible> {
    Const(C),
    Tokens(&'a TokenStream),
  }

  block
    .items
    .iter()
    .filter_map(|item| {
      match item {
        | ImplItem::Const(constant) => Unifier::Const(constant),
        | ImplItem::Macro(ImplItemMacro {
          mac: Macro { tokens, .. }, ..
        }) => Unifier::Tokens(tokens),
        | _ => return None,
      }
      .into()
    })
    .flat_map(|constant| match constant {
      | Unifier::Const(constant) => vec![constant],
      | Unifier::Tokens(tokens) => process_macro(tokens),
    })
    .fold(Vec::new(), |mut constants, gathered_constants| {
      (
        constants.append(Const::from_impl(gathered_constants, &source)),
        constants,
      )
        .1
    })
}

pub(crate) fn process_trait_block(
  block: &ItemTrait,
  source: impl AsRef<Path>,
) -> Vec<Const> {
  block
    .items
    .iter()
    .filter_map(|item| {
      match item {
        | TraitItem::Const(constant) => constant,
        | _ => return None,
      }
      .into()
    })
    .fold(Vec::new(), |mut constants, item| {
      (constants.push(Const::from_trait(item, &source)), constants).1
    })
}

pub(crate) fn process_mod_block(
  block: &[Item],
  source: impl AsRef<Path>,
) -> Vec<Const> {
  block
    .iter()
    .filter_map(|item| {
      match item {
        | Item::Const(constant) => process_constant(constant, &source),
        | Item::Impl(impl_block) => process_impl_block(impl_block, &source),
        | Item::Trait(trait_block) => process_trait_block(trait_block, &source),
        | Item::Mod(ItemMod { content: Some((_, contents)), .. }) =>
          process_mod_block(contents, &source),
        | _ => return None,
      }
      .into()
    })
    .fold(Vec::new(), |mut constants, mut gathered_constants| {
      (constants.append(&mut gathered_constants), constants).1
    })
}

pub(crate) fn process_macro(
  tokens: &TokenStream,
) -> Vec<impl ConstConvertible> {
  todo!()
}

pub(crate) trait ConstConvertible {
  fn convert(self, source: impl AsRef<Path>) -> Const;
}

impl ConstConvertible for ItemConst {
  fn convert(self, source: impl AsRef<Path>) -> Const {
    Const::from_item(&self, source)
  }
}

impl ConstConvertible for ImplItemConst {
  fn convert(self, source: impl AsRef<Path>) -> Const {
    Const::from_impl(&self, source)
  }
}

impl ConstConvertible for TraitItemConst {
  fn convert(self, source: impl AsRef<Path>) -> Const {
    Const::from_trait(&self, source)
  }
}
