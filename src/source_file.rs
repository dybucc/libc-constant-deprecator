use std::path::{Path, PathBuf};

use syn::File;

#[derive(Debug)]
pub struct SourceFile(pub(crate) File);

impl SourceFile {
    pub(crate) fn new(contents: impl Into<File>) -> Self {
        Self(contents.into())
    }

    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use this function."
    )]
    pub fn syntax_tree(&self) -> &File {
        &self.0
    }

    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use this function."
    )]
    pub fn into_syntax_tree(self) -> File {
        self.0
    }
}
