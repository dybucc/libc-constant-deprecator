use std::path::PathBuf;

use syn::File;

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
