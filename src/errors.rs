use std::{
    borrow::Cow,
    error::Error,
    io,
    path::{Path, PathBuf},
};

use thiserror::Error;

// TODO: get the error printing logic refactored into a separate function.

#[derive(Debug, Error)]
pub enum ScanFilesError {
    #[error(
        "{}",
        match .0 {
            RepoErrorRepr::Discover { path, kind } => {
                format!(
                    "failed to discover repo at path: {}{}",
                    path.display(),
                    match kind {
                        DiscoverErrorKind::NoRepository => {
                            "; no repo found within it or upwards".to_string()
                        }
                        DiscoverErrorKind::InvalidDir(source) => {
                            if let Some(err) = source {
                                format!("; {err}")
                            } else {
                                String::new()
                            }
                        }
                        DiscoverErrorKind::InvalidRepoConfig => {
                            "; git config in repo is not valid".to_string()
                        },
                        DiscoverErrorKind::WrongUtf8 => "; found invalid utf-8 in path".to_string(),
                        DiscoverErrorKind::Other(err) => format!("; {err}")
                    }
                )
            },
            RepoErrorRepr::Clone { path, kind } => {
                format!(
                    "failed to clone repo to path: {}{}",
                    path.display(),
                    match kind {
                        CloneErrorKind::LibcUrl => {
                            "; libc repo url is wrong; report this to the maintainer".to_string()
                        },
                        CloneErrorKind::InvalidRepoConfig => {
                            "; git repo config is not valid".to_string()
                        }
                        CloneErrorKind::DirectoryNotEmpty => "; directory not empty".to_string(),
                        CloneErrorKind::IllegalUtf8 => "; found invalid utf-8 in path".to_string(),
                        CloneErrorKind::IoBound(err) => format!("; {err}"),
                        CloneErrorKind::Other(err) => format!("; {err}"),
                    }
                )
            },
            RepoErrorRepr::Other(err) => format!("{err}")
        }
    )]
    RepoError(RepoErrorRepr),
    #[error("FIXME")]
    FetchError(),
    #[error("failed parsing rust source file `{0}`")]
    ParseError(PathBuf),
    #[error("internal io error: {0}")]
    IoBound(io::Error),
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

// NOTE: the error variants in this enum may seem like they suffer from
// fragmentation in that task errors can also be gathered by `RepoErrorRepr`. We
// separate them into two different variants for the purposes of unit testing.
#[derive(Debug)]
pub(crate) enum DiscoverRepoError {
    Error(RepoErrorRepr),
    Task(Box<dyn Error + Send + Sync>),
}

#[derive(Debug)]
pub(crate) enum CloneRepoError {
    Error(RepoErrorRepr),
    Task(Box<dyn Error + Send + Sync>),
}

#[derive(Debug)]
pub enum RepoErrorRepr {
    Discover {
        path: PathBuf,
        kind: DiscoverErrorKind,
    },
    Clone {
        path: PathBuf,
        kind: CloneErrorKind,
    },
    Other(Box<dyn Error + Send + Sync>),
}

#[derive(Debug)]
pub enum DiscoverErrorKind {
    NoRepository,
    InvalidDir(Option<io::Error>),
    InvalidRepoConfig,
    WrongUtf8,
    Other(Box<dyn Error + Send + Sync>),
}

#[derive(Debug)]
pub enum CloneErrorKind {
    DirectoryNotEmpty,
    InvalidRepoConfig,
    LibcUrl,
    IllegalUtf8,
    IoBound(io::Error),
    Other(Box<dyn Error + Send + Sync>),
}

#[derive(Debug)]
pub(crate) enum FetchParseError {
    ParsingFailed,
    IoBound(io::Error),
    Other(Box<dyn Error + Send + Sync>),
}

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
