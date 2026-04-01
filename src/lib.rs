#![feature(bool_to_result, exit_status_error, string_from_utf8_lossy_owned)]

pub(crate) mod constant;
pub(crate) mod constant_container;
pub(crate) mod errors;
pub(crate) mod parser;
pub(crate) mod scanner;
pub(crate) mod source_file;
pub(crate) mod support;

#[cfg_attr(
  not(test),
  expect(
    clippy::wildcard_imports,
    reason = "The `errors` module is meant to reexport all its items."
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

#[macro_export]
macro_rules! deprecate {
  ($msg:expr) => {{
    let msg = $msg;

    $crate::support::parse_quote! {
      #[deprecated(since = "1.0.0", note = #msg)]
    }
  }};
}

// TODO: implement functionality to both check and embed information on the
// location of a file with the constants formatted inside the `Cargo.toml` of
// the `libc` repo.

pub(crate) const LIBC_REPO: &str = "https://github.com/rust-lang/libc.git";

pub(crate) const EDITION: &str = "2021";
