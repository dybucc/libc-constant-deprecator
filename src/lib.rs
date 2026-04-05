#![feature(bool_to_result)]

pub(crate) mod constant;
pub(crate) mod constant_container;
pub(crate) mod errors;
#[macro_use]
pub(crate) mod macros;
pub(crate) mod parser;
pub(crate) mod scanner;
pub(crate) mod source_file;
pub(crate) mod support;

#[expect(
  clippy::single_component_path_imports,
  reason = "The macro is reexported at the crate level after having tagged \
            the corresponding `macros` module with `macro_use`."
)]
pub(crate) use deprecate;

#[cfg_attr(
  not(test),
  expect(
    clippy::wildcard_imports,
    reason = "This module is meant to reexport all its items."
  )
)]
pub(crate) use crate::errors::*;
#[doc(inline)]
pub use crate::{
  constant::Const,
  constant_container::ConstContainer,
  errors::{
    FetchError,
    FilterError,
    IoBoundChanges,
    MakeChangesError,
    SaveError,
    ScanFilesError,
  },
  parser::parse_constants,
  scanner::scan_files,
  source_file::SourceFile,
};

// TODO: implement functionality to both check and embed information on the
// location of a file with the constants formatted inside the `Cargo.toml` of
// the `libc` repo.
