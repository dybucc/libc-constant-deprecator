use std::{
    borrow::Cow,
    error::Error,
    io,
    path::{Path, PathBuf},
};

use thiserror::Error;

use crate::ConstContainer;

/// Represents an error condition while parsing the codebase with
/// [`scan_files()`].
///
/// The error is produced when something git-related fails, or if the constants
/// fail to parse. Otherwise the error is deemed to be an I/O-bound
/// [`io::Error`] or a task-bound [`JoinError`] error.
///
/// [`scan_files()`]: `crate::scan_files()`
/// [`JoinError`]: `tokio::task::JoinError`
#[derive(Debug, Error)]
#[repr(transparent)]
#[error(transparent)]
pub struct ScanFilesError(#[from] pub(crate) ScanFilesErrorRepr);

#[derive(Debug, Error)]
pub(crate) enum ScanFilesErrorRepr {
    #[error("{}", .0.error())]
    RepoError(RepoErrorRepr),
    #[error("failed parsing rust source file `{0}`")]
    ParseError(PathBuf),
    #[error("internal io error: `{0}`")]
    IoBound(io::Error),
    #[error("path was not libc repo")]
    NotLibcRepo,
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
pub(crate) enum RepoErrorRepr {
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
pub(crate) enum DiscoverErrorKind {
    NoRepository,
    InvalidDir(Option<io::Error>),
    InvalidRepoConfig,
    WrongUtf8,
    Other(Box<dyn Error + Send + Sync>),
}

#[derive(Debug)]
pub(crate) enum CloneErrorKind {
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

/// Represents an error condition when filtering errors with a regex in
/// [`ConstContainer::filter()`].
///
/// The only two possible error conditions correspond with those currently in
/// the [`regex`] crate.
#[derive(Debug, Error)]
#[repr(transparent)]
#[error(transparent)]
pub struct FilterError(#[from] pub(crate) FilterErrorRepr);

impl FilterError {
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn source_re(&self) -> &str {
        self.0.source_re()
    }
}

#[derive(Debug, Error)]
pub(crate) enum FilterErrorRepr {
    #[error("regex byte size exceeds 2^{} bytes", ConstContainer::MAX_RE_POWER)]
    RegexTooBig(Cow<'static, str>),
    #[error("failed to parse regex")]
    RegexSyntax(Cow<'static, str>),
}

impl FilterErrorRepr {
    pub(crate) fn source_re(&self) -> &str {
        match self {
            Self::RegexTooBig(out) | Self::RegexSyntax(out) => out,
        }
    }
}

/// Represents errors that have taken place as part of effecting changes to disk
/// in [`ConstContainer::effect_changes()`].
#[derive(Debug, Error)]
#[repr(transparent)]
#[error(transparent)]
pub struct MakeChangesError(#[from] pub(crate) MakeChangesErrorRepr);

impl MakeChangesError {
    /// Returns the source path where an underlying [`io::Error`] took place if
    /// the underlying error is, indeed, an `io::Error`.
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn io_source(&self) -> Option<&Path> {
        if let MakeChangesErrorRepr::IoBound(ref ch) = self.0 {
            ch.source_path().into()
        } else {
            None
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum MakeChangesErrorRepr {
    #[error("io error while {}: `{}`", .0.error(), .0.inner)]
    IoBound(IoBoundChanges),
    #[error("failed to parse file: {0}")]
    Parse(Cow<'static, Path>),
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

#[derive(Debug)]
pub(crate) struct IoBoundChanges {
    pub(crate) origin: ChangesSrc,
    pub(crate) inner: io::Error,
}

impl IoBoundChanges {
    pub(crate) fn error(&self) -> String {
        match self.origin {
            ChangesSrc::FetchOp(ref erred_path) => {
                format!("fetching file {}", erred_path.display())
            }
            ChangesSrc::SaveOp(ref erred_path) => format!("saving file {}", erred_path.display()),
        }
    }
}

impl IoBoundChanges {
    pub(crate) fn source_path(&self) -> &Path {
        match self.origin {
            ChangesSrc::FetchOp(ref path) | ChangesSrc::SaveOp(ref path) => path,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ChangesSrc {
    FetchOp(PathBuf),
    SaveOp(PathBuf),
}
