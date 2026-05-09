//! This crate is aimed at holding basic functionality to allow deprecation of
//! constant symbols in the `libc` crate.
//!
//! It is most useful when paired with the accompanying binary.

#![feature(iter_collect_into, bool_to_result)]

mod constant;
mod constant_container;
mod errors;
mod parser;
mod scanner;
mod source_file;
mod support;

// Private reexports.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "This module is meant to reexport all its items."
    )
)]
pub(crate) use crate::errors::*;
pub(crate) use crate::parser::{ir_container::IrContainer, macro_parser::MacroParser};

// Public reexports.

#[rustfmt::skip]
#[doc(inline)]
pub use crate::{
    constant::Const,
    constant_container::{
        ConstContainer,
        borrowed::{BorrowedContainer, BorrowedSubset, Visit},
    },
    errors::{FilterError, MakeChangesError, ScanFilesError},
    scanner::scan,
    source_file::SourceFile,
};

// Macro reexports; left last to force all other crate modules to import the
// macros as items and not just have them immediately available.

#[macro_use]
mod macros;

#[expect(
    clippy::single_component_path_imports,
    reason = "The macro is reexported at the crate level but is not part of the public API."
)]
pub(crate) use deprecate;
#[expect(
    clippy::single_component_path_imports,
    reason = "The macro is reexported at the crate level but is not part of the public API."
)]
pub(crate) use send_sync_impl;
