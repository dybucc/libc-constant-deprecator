use std::path::Path;

use proc_macro2::TokenStream;
use syn::{Item, ItemConst, ItemMacro, Macro};

use crate::{Const, SourceFile};

/// This only scans through module level items and macros, as a quick search
/// through the `libc` codebase with `rg -C 10 -e "^(mod|trait|impl)\s+"` yields
/// no results that would make scanning for inline modules and inherent/trait
/// impl blocks worth it.
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
                | Item::Macro(ItemMacro {
                  mac: Macro { tokens, .. }, ..
                }) => process_macro(tokens),
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

pub(crate) fn process_macro(tokens: &TokenStream) -> Vec<Const> { todo!() }
