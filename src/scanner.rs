use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

use futures::future;
use gix::{
    clone::{self, PrepareCheckout, PrepareFetch, fetch},
    config, create,
    discover::{self, upwards},
    init, open,
    path::relative_path,
    progress::Discard,
    remote::{self, connect},
};
use syn::File;
use tokio::{
    fs,
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    task::{self, JoinSet},
};
use walkdir::WalkDir;

use crate::{
    CloneErrorKind, CloneRepoError, DiscoverErrorKind, DiscoverRepoError, FetchParseError,
    RepoErrorRepr, ScanFilesError, SourceFile,
};

pub(crate) const LIBC_REPO: &str = "https://github.com/rust-lang/libc.git";

/// Scans the `libc` codebase in the provided path, cloning it from upstream if
/// the path does not exist.
///
/// # Errors
///
/// The function may fail if it fails to discover/clone the repo on the given
/// path and, when dealing with an existing path, its ancestors.
pub async fn scan_files(libc_path: impl AsRef<Path>) -> Result<Vec<SourceFile>, ScanFilesError> {
    // NOTE: it's more intuitive to have the routine name be the token tree we
    // accept for recursive munching, but that way the recursive macro invocation
    // would have to replace the start of the path of enum variant, which seems to
    // trigger errors.
    macro_rules! handle_result {
        (@DiscoverRepoError) => {
            discover_repo
        };
        (@CloneRepoError) => {
            clone_repo
        };
        ($err:tt) => {{
            handle_result!(@$err)(libc_path.as_ref().to_owned())
                .await
                .map_err(|err| match err {
                    $err::Error(err) => ScanFilesError::RepoError(err),
                    $err::Task(err) => ScanFilesError::Other(err),
                })?
        }};
    }

    if fs::try_exists(&libc_path)
        .await
        .map_err(ScanFilesError::IoBound)?
    {
        handle_result!(DiscoverRepoError);
    } else {
        handle_result!(CloneRepoError);
    }

    let (tx, rx) = mpsc::unbounded_channel();

    match future::try_join(
        fetch_details(libc_path.as_ref().to_owned(), tx),
        parse_files(rx),
    )
    .await
    {
        Ok(((), res)) => Ok(res),
        Err(err) => match err {
            FetchParseError::ParsingFailed(path) => Err(ScanFilesError::ParseError(path)),
            FetchParseError::IoBound(err) => Err(ScanFilesError::IoBound(err)),
            FetchParseError::Other(err) => Err(ScanFilesError::Other(err)),
        },
    }
}

pub(crate) async fn discover_repo(path: PathBuf) -> Result<(), DiscoverRepoError> {
    task::spawn_blocking(|| {
        gix::discover(&path)
            .map_err(|err| match err {
                discover::Error::Discover(err) => match err {
                    upwards::Error::CurrentDir(err) => RepoErrorRepr::Discover {
                        path,
                        kind: DiscoverErrorKind::InvalidDir(err.into()),
                    },
                    upwards::Error::InvalidInput { .. }
                    | upwards::Error::InaccessibleDirectory { .. }
                    | upwards::Error::NoMatchingCeilingDir => RepoErrorRepr::Discover {
                        path,
                        kind: DiscoverErrorKind::InvalidDir(None),
                    },
                    err @ (upwards::Error::NoTrustedGitRepository { .. }
                    | upwards::Error::CheckTrust { .. }) => RepoErrorRepr::Discover {
                        path,
                        kind: DiscoverErrorKind::Other(err.into()),
                    },
                    upwards::Error::NoGitRepository { .. }
                    | upwards::Error::NoGitRepositoryWithinFs { .. }
                    | upwards::Error::NoGitRepositoryWithinCeiling { .. } => {
                        RepoErrorRepr::Discover {
                            path,
                            kind: DiscoverErrorKind::NoRepository,
                        }
                    }
                },
                discover::Error::Open(err) => match err {
                    open::Error::Config(_) => RepoErrorRepr::Discover {
                        path,
                        kind: DiscoverErrorKind::InvalidRepoConfig,
                    },
                    open::Error::NotARepository { .. } => RepoErrorRepr::Discover {
                        path,
                        kind: DiscoverErrorKind::NoRepository,
                    },
                    open::Error::Io(err) => RepoErrorRepr::Discover {
                        path,
                        kind: DiscoverErrorKind::InvalidDir(err.into()),
                    },
                    open::Error::PrefixNotRelative(relative_path::Error::IllegalUtf8(_)) => {
                        RepoErrorRepr::Discover {
                            path,
                            kind: DiscoverErrorKind::WrongUtf8,
                        }
                    }
                    err @ (open::Error::UnsafeGitDir { .. }
                    | open::Error::EnvironmentAccessDenied(_)
                    | open::Error::PrefixNotRelative(_)) => RepoErrorRepr::Discover {
                        path,
                        kind: DiscoverErrorKind::Other(err.into()),
                    },
                },
            })
            .map(|_| ())
    })
    .await
    .map_err(Into::into)
    .map_err(DiscoverRepoError::Task)?
    .map_err(DiscoverRepoError::Error)
}

pub(crate) async fn clone_repo(path: PathBuf) -> Result<(), CloneRepoError> {
    task::spawn_blocking(|| {
        fetch_repo(path.clone(), prepare_clone(path.clone())?)?
            .main_worktree(Discard, &AtomicBool::new(false))
            .map_err(|err| RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::Other(err.into()),
            })
            .map_err(CloneRepoError::Error)
            .map(|_| ())
    })
    .await
    .map_err(Into::into)
    .map_err(CloneRepoError::Task)?
}

pub(crate) fn prepare_clone(path: PathBuf) -> Result<PrepareFetch, CloneRepoError> {
    gix::prepare_clone(LIBC_REPO, &path)
        .map_err(|err| match err {
            clone::Error::Init(err) => match err {
                init::Error::CurrentDir(err) => RepoErrorRepr::Clone {
                    path,
                    kind: CloneErrorKind::IoBound(err),
                },
                init::Error::Init(err) => match err {
                    create::Error::CurrentDir(err)
                    | create::Error::IoOpen { source: err, .. }
                    | create::Error::IoWrite { source: err, .. }
                    | create::Error::CreateDirectory { source: err, .. } => RepoErrorRepr::Clone {
                        path,
                        kind: CloneErrorKind::IoBound(err),
                    },
                    create::Error::DirectoryExists { .. }
                    | create::Error::DirectoryNotEmpty { .. } => RepoErrorRepr::Clone {
                        path,
                        kind: CloneErrorKind::DirectoryNotEmpty,
                    },
                },
                init::Error::Open(err) => match err {
                    open::Error::Config(_) => RepoErrorRepr::Clone {
                        path,
                        kind: CloneErrorKind::InvalidRepoConfig,
                    },
                    open::Error::NotARepository { source, .. } => RepoErrorRepr::Clone {
                        path,
                        kind: CloneErrorKind::Other(source.into()),
                    },
                    open::Error::Io(err) => RepoErrorRepr::Clone {
                        path,
                        kind: CloneErrorKind::IoBound(err),
                    },
                    err @ open::Error::UnsafeGitDir { .. } => RepoErrorRepr::Clone {
                        path,
                        kind: CloneErrorKind::Other(err.into()),
                    },
                    open::Error::EnvironmentAccessDenied(err) => RepoErrorRepr::Clone {
                        path,
                        kind: CloneErrorKind::Other(err.into()),
                    },
                    open::Error::PrefixNotRelative(relative_path::Error::IllegalUtf8(_)) => {
                        RepoErrorRepr::Clone {
                            path,
                            kind: CloneErrorKind::IllegalUtf8,
                        }
                    }
                    open::Error::PrefixNotRelative(err) => RepoErrorRepr::Clone {
                        path,
                        kind: CloneErrorKind::Other(err.into()),
                    },
                },
                init::Error::InvalidBranchName { .. } => RepoErrorRepr::Clone {
                    path,
                    kind: CloneErrorKind::LibcUrl,
                },
                init::Error::EditHeadForDefaultBranch(err) => RepoErrorRepr::Clone {
                    path,
                    kind: CloneErrorKind::Other(err.into()),
                },
            },
            clone::Error::CommitterOrFallback(_) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::InvalidRepoConfig,
            },
            clone::Error::UrlParse(_) | clone::Error::CanonicalizeUrl { .. } => {
                RepoErrorRepr::Clone {
                    path,
                    kind: CloneErrorKind::LibcUrl,
                }
            }
        })
        .map_err(CloneRepoError::Error)
}

pub(crate) fn fetch_repo(
    path: PathBuf,
    mut clone_handle: PrepareFetch,
) -> Result<PrepareCheckout, CloneRepoError> {
    clone_handle
        .fetch_then_checkout(Discard, &AtomicBool::new(false))
        .map_err(|err| match err {
            fetch::Error::Connect(connect::Error::InvalidRemoteRepositoryPath { .. }) => {
                RepoErrorRepr::Clone {
                    path,
                    kind: CloneErrorKind::LibcUrl,
                }
            }
            fetch::Error::Connect(err) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::Other(err.into()),
            },
            fetch::Error::PrepareFetch(err) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::Other(err.into()),
            },
            fetch::Error::Fetch(err) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::Other(err.into()),
            },
            fetch::Error::RemoteInit(err) => match err {
                remote::init::Error::Url(_) | remote::init::Error::RewrittenUrlInvalid { .. } => {
                    RepoErrorRepr::Clone {
                        path,
                        kind: CloneErrorKind::LibcUrl,
                    }
                }
            },
            fetch::Error::RemoteConfiguration(err) | fetch::Error::RemoteConnection(err) => {
                RepoErrorRepr::Clone {
                    path,
                    kind: CloneErrorKind::Other(err),
                }
            }
            fetch::Error::RemoteName(_)
            | fetch::Error::ParseConfig(_)
            | fetch::Error::ApplyConfig(_)
            | fetch::Error::CommitterOrFallback(_) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::InvalidRepoConfig,
            },
            fetch::Error::LoadConfig(config::file::init::from_paths::Error::Io {
                source, ..
            }) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::IoBound(source),
            },
            fetch::Error::LoadConfig(err) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::Other(err.into()),
            },
            fetch::Error::SaveConfig(err) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::Other(err.into()),
            },
            fetch::Error::SaveConfigIo(err) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::IoBound(err),
            },
            fetch::Error::InvalidHeadRef { source, .. } => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::Other(source.into()),
            },
            fetch::Error::HeadUpdate(err) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::Other(err.into()),
            },
            err @ (fetch::Error::RefNameMissing { .. } | fetch::Error::RefNameAmbiguous { .. }) => {
                RepoErrorRepr::Clone {
                    path,
                    kind: CloneErrorKind::Other(err.into()),
                }
            }
            fetch::Error::RefMap(err) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::Other(err.into()),
            },
            fetch::Error::ReferenceName(err) => RepoErrorRepr::Clone {
                path,
                kind: CloneErrorKind::Other(err.into()),
            },
        })
        .map_err(CloneRepoError::Error)
        .map(|(out, _)| out)
}

pub(crate) async fn fetch_details(
    mut path: PathBuf,
    tx: UnboundedSender<PathBuf>,
) -> Result<(), FetchParseError> {
    // NOTE: we don't implement manual directory traversal with something like
    // `tokio`'s `fs` functions because those use the same `std` blocking functions
    // only on a separate, "blocking-friendly" thread, so we may as well keep
    // `walkdir` but run it also in a separate thread.
    task::spawn_blocking(move || {
        path.push("src");

        WalkDir::new(path)
            .sort_by_file_name()
            .contents_first(true)
            .into_iter()
            .filter_map(|entry| {
                entry.ok().and_then(|entry| {
                    (entry.file_type().is_file())
                        .then(|| entry.into_path())
                        .filter(|inner| inner.extension().is_some_and(|ext| ext == "rs"))
                })
            })
            .for_each(|entry| _ = tx.send(entry));
    })
    .await
    .map_err(Into::into)
    .map_err(FetchParseError::Other)
}

pub(crate) async fn parse_files(
    mut rx: UnboundedReceiver<PathBuf>,
) -> Result<Vec<SourceFile>, FetchParseError> {
    // NOTE: this is necessary to have `syn::File` be `Send`, as otherwise we can't
    // have any degree of parallelism/concurrency.
    #[repr(transparent)]
    struct FileWrapper(File);

    unsafe impl Send for FileWrapper {}

    let mut task_pool = JoinSet::new();
    let (inner_tx, mut inner_rx) = mpsc::unbounded_channel();

    let gatherer = task::spawn(async move {
        let mut out = Vec::new();

        while let Some(res) = inner_rx.recv().await {
            out.push(res);
        }

        out
    });

    while let Some(path) = rx.recv().await {
        let inner_tx = inner_tx.clone();
        task_pool.spawn(async move {
            inner_tx
                .send((
                    FileWrapper(
                        syn::parse_file(
                            &fs::read_to_string(&path)
                                .await
                                .map_err(FetchParseError::IoBound)?,
                        )
                        .map_err(|_| path.clone())
                        .map_err(FetchParseError::ParsingFailed)?,
                    ),
                    path,
                ))
                .map_err(|_| FetchParseError::Other("synchronization error".into()))?;
            Ok::<_, FetchParseError>(())
        });
    }

    // NOTE: we must explicitly drop the transmitter here because it's been
    // relentlessly cloned throughout tasks and the receiver inside the gatherer
    // task will not stop looping until the last instance of the transmitter is
    // dropped. All cloned instances are dropped the moment they send their values
    // to the gatherer except for the original transmitter, which we drop here.
    drop(inner_tx);

    gatherer
        .await
        .map(|out| {
            out.into_iter()
                .map(|(FileWrapper(file), path)| SourceFile::new(file, path))
                .collect()
        })
        .map_err(Into::into)
        .map_err(FetchParseError::Other)
}
