use std::{
    borrow::Cow,
    error::Error,
    io,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::ConstContainer;

#[derive(Debug, Error)]
pub enum ScanFilesError {
    #[error("{}", .0.error())]
    RepoError(RepoErrorRepr),
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
}

impl RepoErrorRepr {
    pub(crate) fn error(&self) -> String {
        match self {
            Self::Discover { path, kind } => {
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
                        }
                        DiscoverErrorKind::WrongUtf8 => "; found invalid utf-8 in path".to_string(),
                        DiscoverErrorKind::Other(err) => format!("; {err}"),
                    }
                )
            }
            Self::Clone { path, kind } => {
                format!(
                    "failed to clone repo to path: {}{}",
                    path.display(),
                    match kind {
                        CloneErrorKind::LibcUrl => {
                            "; libc repo url is wrong; report this to the maintainer".to_string()
                        }
                        CloneErrorKind::InvalidRepoConfig => {
                            "; git repo config is not valid".to_string()
                        }
                        CloneErrorKind::DirectoryNotEmpty => "; directory not empty".to_string(),
                        CloneErrorKind::IllegalUtf8 => "; found invalid utf-8 in path".to_string(),
                        CloneErrorKind::IoBound(err) => format!("; {err}"),
                        CloneErrorKind::Other(err) => format!("; {err}"),
                    }
                )
            }
        }
    }
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
    ParsingFailed(PathBuf),
    IoBound(io::Error),
    Other(Box<dyn Error + Send + Sync>),
}

#[derive(Debug, Error)]
pub enum FilterError {
    #[error("regex byte size exceeds 2^{} bytes", ConstContainer::MAX_RE_POWER)]
    RegexTooBig(Cow<'static, str>),
    #[error("failed to parse regex")]
    RegexSyntax(Cow<'static, str>),
}

impl FilterError {
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug for the result of this routine not to be used."
    )]
    pub fn source_re(&self) -> &str {
        match self {
            Self::RegexTooBig(out) | Self::RegexSyntax(out) => out,
        }
    }
}

#[derive(Debug, Error)]
pub enum MakeChangesError {
    #[error("io error while {}: `{}`", .0.error(), .0.inner)]
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
    pub(crate) fn error(&self) -> String {
        match &self.origin {
            ChangesSrc::FetchOp(erred_path) => format!("fetching file {}", erred_path.display()),
            ChangesSrc::SaveOp(erred_path) => format!("saving file {}", erred_path.display()),
        }
    }
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
