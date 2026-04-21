use std::{
    borrow::Cow,
    io,
    path::{Path, PathBuf},
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScanFilesError {
    #[error("failed to change pwd: {0}")]
    PwdSetting(io::Error),
    #[error(
        "{}",
        match .0 {
            RepoErrorRepr::Clone(path) => format!("failed to clone repo to path {}", path.display())
        }
    )]
    RepoError(RepoErrorRepr),
    #[error("directory `{0}` doesn't contain a cargo workspace with `libc` in it")]
    NoLibc(PathBuf),
    #[error("workspace querying through `cargo-metadata` failed for directory `{0}`")]
    WorkspaceScanning(PathBuf),
    #[error("failed parsing rust source file `{0}`")]
    ParseError(PathBuf),
    #[error("internal io error: {0}")]
    IoBound(io::Error),
}

#[derive(Debug)]
pub(crate) enum FetchDetailsError {
    CargoMetadata,
    NoLibc,
}

#[derive(Debug)]
pub(crate) enum RepoErrorRepr {
    Clone(PathBuf),
}

#[derive(Debug)]
pub(crate) struct ParseFilesError(pub(crate) PathBuf);

#[derive(Debug, Error)]
#[error("failed to save constants to file: `{0}`")]
pub struct SaveError(pub(crate) io::Error);

#[derive(Debug, Error)]
pub enum FilterError {
    #[error("regex compilation failed for needle: `{input_str}`")]
    RegexCompilation { input_str: String },
}

#[derive(Debug, Error)]
pub enum MakeChangesError {
    #[error(
        "io error while {}: `{}`",
        match &.0.origin {
            ChangesSrc::FetchOp(erred_path) => format!("fetching file {}", erred_path.display()),
            ChangesSrc::SaveOp(erred_path) => format!("saving file {}", erred_path.display())
        },
        .0.inner
    )]
    IoBound(IoBoundChanges),
    #[error("failed to parse file: {0}")]
    Parse(Cow<'static, Path>),
    #[error("failed to format codebase while effecting changes to disk")]
    Format,
}

#[derive(Debug)]
pub struct IoBoundChanges {
    pub(crate) origin: ChangesSrc,
    pub(crate) inner: io::Error,
}

impl IoBoundChanges {
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn source(&self) -> &Path {
        match &self.origin {
            ChangesSrc::FetchOp(path) | ChangesSrc::SaveOp(path) => path,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ChangesSrc {
    FetchOp(PathBuf),
    SaveOp(PathBuf),
}
