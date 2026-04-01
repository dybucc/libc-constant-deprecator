use std::path::{Path, PathBuf};

use syn::File;

#[derive(Debug)]
pub struct SourceFile {
  pub(crate) contents: File,
  pub(crate) source:   PathBuf,
}

impl SourceFile {
  pub(crate) fn new(contents: impl Into<File>, path: impl AsRef<Path>) -> Self {
    Self { contents: contents.into(), source: path.as_ref().to_owned() }
  }

  #[expect(
    clippy::must_use_candidate,
    reason = "It's not a bug not to use this function."
  )]
  pub fn syntax_tree(&self) -> &File { &self.contents }

  #[expect(
    clippy::must_use_candidate,
    reason = "It's not a bug not to use this function."
  )]
  pub fn into_syntax_tree(self) -> File { self.contents }

  #[expect(
    clippy::must_use_candidate,
    reason = "It's not a bug not to use this function."
  )]
  pub fn path(&self) -> &Path { &self.source }

  #[expect(
    clippy::must_use_candidate,
    reason = "It's not a bug not to use this function."
  )]
  pub fn into_path(self) -> PathBuf { self.source }
}
