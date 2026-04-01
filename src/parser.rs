use std::path::Path;

use syn::{ImplItem, Item, ItemConst, ItemImpl, ItemTrait, TraitItem};

use crate::{Const, SourceFile};

#[expect(
  clippy::must_use_candidate,
  reason = "It's not a bug not to use the result of this routine."
)]
pub fn parse_constants(files: &[SourceFile]) -> Vec<Const> {
  // TODO: handle inline modules (those that `syn` has parsed as having a
  // `Some(_)` in their `content` field) by checking recursively for all items
  // within them.
  files.iter().fold(
    Vec::with_capacity(files.len()),
    |mut parsed_files, SourceFile { contents, source }| {
      parsed_files.append(
        &mut contents
          .items
          .iter()
          .filter_map(|item| match item {
            | Item::Const(constant) => Some(process_constant(constant, source)),
            | Item::Impl(impl_block) =>
              Some(process_impl_block(impl_block, source)),
            | Item::Trait(trait_block) =>
              Some(process_trait_block(trait_block, source)),
            | _ => None,
          })
          .fold(Vec::new(), |mut constants, mut item| {
            constants.append(&mut item);

            constants
          }),
      );

      parsed_files
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
  block
    .items
    .iter()
    .filter_map(|item| {
      if let ImplItem::Const(constant) = item { Some(constant) } else { None }
    })
    .fold(Vec::new(), |mut constants, item| {
      constants.push(Const::from_impl(item, &source));

      constants
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
      if let TraitItem::Const(constant) = item { Some(constant) } else { None }
    })
    .fold(Vec::new(), |mut constants, item| {
      constants.push(Const::from_trait(item, &source));

      constants
    })
}
