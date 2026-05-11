use std::{
    borrow::Cow,
    error::Error,
    io,
    path::{Path, PathBuf},
};

use either::Either;
use thiserror::Error;

use crate::ConstContainer;

/// Represents an error condition while parsing the codebase with [`scan()`].
///
/// The error is produced when something git-related fails, or if the constants
/// fail to parse. Otherwise the error is deemed to be an I/O-bound
/// [`io::Error`] or a task-bound [`JoinError`] error.
///
/// Note no external errors are exposed, and so `tokio` task-bound errors are
/// type erased on construction.
///
/// [`scan()`]: `crate::scan()`
/// [`io::Error`]: `std::io::Error`
/// [`JoinError`]: `tokio::task::JoinError`
#[derive(Debug, Error)]
#[repr(transparent)]
#[error(transparent)]
pub struct ScanError(#[from] ScanErrorRepr);

#[derive(Debug, Error)]
pub(crate) enum ScanErrorRepr {
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

macro_rules! repo_error_impl {
    (for $($t:ty),+ ;) => {
        $(
            impl $t {
                pub(crate) fn into_inner(self) -> RepoErrorRepr {
                    let Self { repr } = self;

                    repr
                }
            }

            impl From<RepoErrorRepr> for $t {
                fn from(value: RepoErrorRepr) -> Self {
                    Self { repr: value }
                }
            }
        )+
    };
}

repo_error_impl! { for DiscoverRepoError, CloneRepoError; }

#[derive(Debug)]
pub(crate) struct DiscoverRepoError {
    repr: RepoErrorRepr,
}

#[derive(Debug)]
pub(crate) struct CloneRepoError {
    repr: RepoErrorRepr,
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
///
/// [`ConstContainer::filter()`]: `crate::ConstContainer::filter`
/// [`regex`]: `regex`
#[derive(Debug, Error)]
#[repr(transparent)]
#[error(transparent)]
pub struct FilterError(#[from] FilterErrorRepr);

impl FilterError {
    /// Fetches the source regex with which the error was produced and returns a
    /// reference to the underlying string slice.
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
///
/// [`ConstContainer::effect_changes()`]: `crate::ConstContainer::effect_changes`
#[derive(Debug, Error)]
#[repr(transparent)]
#[error(transparent)]
pub struct MakeChangesError(#[from] MakeChangesErrorRepr);

impl MakeChangesError {
    /// Returns the source path where an underlying [`io::Error`] took place if
    /// the underlying error is, indeed, an `io::Error`.
    ///
    /// Returns `None` if the underlying error is not an `io::Error`.
    ///
    /// [`io::Error`]: `std::io::Error`
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn io_source(&self) -> Option<impl AsRef<Path>> {
        if let MakeChangesErrorRepr::IoBound(ref ch) = self.0 {
            ch.source_path().into()
        } else {
            None
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum MakeChangesErrorRepr {
    #[error("io error while {}: `{}`", .0.error(), .0.source_io())]
    IoBound(IoBoundChanges),
    #[error("failed to parse file: {0}")]
    Parse(Cow<'static, Path>),
    #[error(transparent)]
    Other(Box<dyn Error + Send + Sync>),
}

#[derive(Debug)]
pub(crate) struct IoBoundChanges {
    origin: ChangesSrc,
    inner: io::Error,
}

impl IoBoundChanges {
    pub(crate) fn new(path: impl AsRef<Path>, inner: io::Error, kind: ChangesKind) -> Self {
        Self {
            origin: ChangesSrc::new(kind, path.as_ref().to_owned()),
            inner,
        }
    }

    pub(crate) fn error(&self) -> String {
        self.origin
            .with_path(
                |path| format!("fetching file {}", path.display()),
                |path| format!("saving fle {}", path.display()),
            )
            .into_inner()
    }

    pub(crate) fn source_io(&self) -> &io::Error {
        &self.inner
    }

    pub(crate) fn source_path(&self) -> impl AsRef<Path> {
        self.origin.path()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChangesKind {
    repr: ChangesKindRepr,
}

impl ChangesKind {
    fn to_changes_src(self, path: PathBuf) -> ChangesSrc {
        match self.repr {
            ChangesKindRepr::FetchOp => ChangesSrc::fetch(path),
            ChangesKindRepr::SaveOp => ChangesSrc::save(path),
        }
    }

    pub(crate) fn fetch() -> Self {
        Self {
            repr: ChangesKindRepr::FetchOp,
        }
    }

    pub(crate) fn save() -> Self {
        Self {
            repr: ChangesKindRepr::SaveOp,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ChangesKindRepr {
    FetchOp,
    SaveOp,
}

#[derive(Debug, Clone)]
struct ChangesSrc {
    repr: ChangesSrcRepr,
}

impl ChangesSrc {
    fn new(kind: ChangesKind, path: PathBuf) -> Self {
        kind.to_changes_src(path)
    }

    fn path(&self) -> impl AsRef<Path> {
        self.repr.path()
    }

    fn with_path<T, U>(
        &self,
        fetch: impl FnOnce(&Path) -> T,
        save: impl FnOnce(&Path) -> U,
    ) -> Either<T, U> {
        self.repr.visit(fetch, save)
    }

    fn fetch(path: PathBuf) -> Self {
        Self {
            repr: ChangesSrcRepr::fetch(path),
        }
    }

    fn save(path: PathBuf) -> Self {
        Self {
            repr: ChangesSrcRepr::save(path),
        }
    }
}

#[derive(Debug, Clone)]
enum ChangesSrcRepr {
    FetchOp(PathBuf),
    SaveOp(PathBuf),
}

impl ChangesSrcRepr {
    fn visit<T, U>(
        &self,
        fetch: impl FnOnce(&Path) -> T,
        save: impl FnOnce(&Path) -> U,
    ) -> Either<T, U> {
        match self {
            Self::FetchOp(path) => Either::Left(fetch(path.as_path())),
            Self::SaveOp(path) => Either::Right(save(path.as_path())),
        }
    }

    fn path(&self) -> impl AsRef<Path> {
        let (Self::FetchOp(path) | Self::SaveOp(path)) = self;

        path
    }

    fn fetch(path: PathBuf) -> Self {
        Self::FetchOp(path)
    }

    fn save(path: PathBuf) -> Self {
        Self::SaveOp(path)
    }
}
