#![feature(exit_status_error)]

mod constant;
mod constant_container;
mod errors;
#[macro_use]
mod macros;
mod parser;
mod scanner;
mod source_file;
mod support;

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
    constant_container::{ConstContainer, borrowed::BorrowedContainer},
    errors::{FilterError, IoBoundChanges, MakeChangesError, SaveError, ScanFilesError},
    parser::parse_constants,
    scanner::scan_files,
    source_file::SourceFile,
};
