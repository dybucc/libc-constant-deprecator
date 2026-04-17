#![feature(bool_to_result)]

mod constant;
mod constant_container;
mod errors;
#[macro_use]
mod macros;
mod parser;
mod scanner;
mod source_file;
mod support;

// TODO: implement functionality to both check and embed information on the
// location of a file with the constants formatted inside the `Cargo.toml` of
// the `libc` repo. This is best implemented on a separate module, as most of
// the interface around `ConstContainer` does not implement functionality for
// storing the path of the file.

#[expect(
    clippy::single_component_path_imports,
    reason = "The macro is reexported at the crate level but is not part of the public API."
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
        FetchError, FilterError, IoBoundChanges, MakeChangesError, SaveError, ScanFilesError,
    },
    parser::parse_constants,
    scanner::scan_files,
    source_file::SourceFile,
};
