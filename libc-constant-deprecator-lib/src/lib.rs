//! This crate is aimed at holding basic functionality to allow deprecation of
//! constant symbols in the `libc` crate.
//!
//! It is most useful when paired with the accompanying binary.

#![feature(iter_collect_into)]

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
pub(crate) use crate::parser::macro_parser::MacroParser;
#[doc(inline)]
pub use crate::{
    constant::Const,
    constant_container::{ConstContainer, borrowed::BorrowedContainer},
    errors::{FilterError, MakeChangesError, ScanFilesError},
    parser::parse_constants,
    scanner::scan_files,
    source_file::SourceFile,
};
