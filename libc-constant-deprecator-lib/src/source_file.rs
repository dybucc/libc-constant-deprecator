use std::path::{Path, PathBuf};

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
pub(crate) struct SourceFile {
    inner: File,
    source: PathBuf,
}

impl SourceFile {
    pub(crate) fn new(contents: File, source: PathBuf) -> Self {
        Self {
            inner: contents,
            source,
        }
    }

    pub(crate) fn parsed_file(&self) -> &File {
        let Self { inner, .. } = self;

        inner
    }

    pub(crate) fn path(&self) -> impl AsRef<Path> {
        let Self { source, .. } = self;

        source
    }
}
