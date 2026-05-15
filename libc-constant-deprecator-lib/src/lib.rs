//! This crate is aimed at holding basic functionality to allow deprecation of
//! constant symbols in the `libc` crate.
//!
//! It is most useful when paired with the accompanying binary.

#![feature(bool_to_result, range_into_bounds, try_trait_v2)]

mod constant;
mod constant_container;
mod errors;
mod parser;
mod scanner;
mod source_file;
mod support;
mod private {
    pub(crate) trait Sealed {}
}

// Private reexports.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "This module is meant to reexport all its items."
    )
)]
pub(crate) use crate::errors::*;
pub(crate) use crate::{
    constant_container::borrowed::borrowed_element::BorrowedElement,
    parser::{const_container_builder::ConstContainerBuilder, macro_parser::MacroParser},
    private::Sealed,
    source_file::SourceFile,
};

// Public reexports.

// NOTE: we skip reformatting here to avoid `rustfmt` from removing separataion
// whitespace with the above private reexports.
#[rustfmt::skip]
#[doc(inline)]
pub use crate::{
    constant::Const,
    constant_container::{
        ConstContainer,
        borrowed::{BorrowedContainer, BorrowedSubset, Visit},
    },
    errors::{FilterError, MakeChangesError, ScanError},
    scanner::scan,
};

// Macro reexports; left last to force all other crate modules to import the
// macros as items and not just have them immediately available.

#[macro_use]
mod macros;

// NOTE: we reexport the macros through another macro to avoid getting the
// `clippy::single_component_path_imports` lint and thus to avoid having to
// `#[expect]` twice.
macro_rules! decl {
    ($(use $id:ident;)+) => { $(pub(crate) use $id;)+ };
}

decl! {
    use deprecate;
    use borrowed;
}
