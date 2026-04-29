use std::path::PathBuf;

use syn::File;

/// Represents an intermediate representation between a parsed file from the
/// `libc` codebase, and the [`ConstContainer`] type.
///
/// This type is required to wrap the result of parsing a file, and keeping
/// track of its path, which are often naturally tied when the file is parsed
/// from the context of the compiler in a proc-macro invocation. That span
/// information is not available when parsing with [`syn`] outside proc-macros.
///
/// This type is produced by [`scan_files()`] and is used in
/// [`parse_constants()`].
///
/// [`ConstContainer`]: `crate::ConstContainer`
/// [`scan_files()`]: `crate::scan_files()`
/// [`parse_constants()`]: `crate::parse_constants()`
#[derive(Debug)]
pub struct SourceFile {
    pub(crate) inner: File,
    pub(crate) source: PathBuf,
}

impl SourceFile {
    pub(crate) fn new(contents: File, source: PathBuf) -> Self {
        Self {
            inner: contents,
            source,
        }
    }
}
