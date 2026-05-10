use std::path::{Path, PathBuf};

use syn::File;

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
