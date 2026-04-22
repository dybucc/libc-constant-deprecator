use std::{
    env,
    path::{Path, PathBuf},
};

use cargo_metadata::MetadataCommand;
use gix::{
    clone, create,
    discover::{self, upwards},
    init, open,
    path::relative_path,
};
use tokio::{fs, task};
use walkdir::WalkDir;

use crate::{
    CloneErrorKind, DiscoverErrorKind, DiscoverRepoError, FetchDetailsError, ParseFilesError,
    RepoErrorRepr, ScanFilesError, SourceFile,
};

pub(crate) const LIBC_REPO: &str = "https://github.com/rust-lang/libc.git";

pub async fn scan_files(libc_path: impl AsRef<Path>) -> Result<Vec<SourceFile>, ScanFilesError> {
    if fs::try_exists(&libc_path)
        .await
        .map_err(ScanFilesError::IoBound)?
    {
        discover_repo(libc_path.as_ref().to_owned())
            .await
            .map_err(|err| match err {
                DiscoverRepoError::Error(err) => ScanFilesError::RepoError(err),
                DiscoverRepoError::Task(err) => {
                    ScanFilesError::RepoError(RepoErrorRepr::Other(err))
                }
            })?;
    } else {
        let path = libc_path.as_ref().to_owned();

        task::spawn_blocking(|| {
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
                            | create::Error::CreateDirectory { source: err, .. } => {
                                RepoErrorRepr::Clone {
                                    path,
                                    kind: CloneErrorKind::IoBound(err),
                                }
                            }
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
                            open::Error::PrefixNotRelative(relative_path::Error::IllegalUtf8(
                                _,
                            )) => RepoErrorRepr::Clone {
                                path,
                                kind: CloneErrorKind::IllegalUtf8,
                            },
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
                .map(|_| ())
        })
        .await
        .map_err(|inner| ScanFilesError::RepoError(RepoErrorRepr::Other(inner.into())))?
        .map_err(ScanFilesError::RepoError)?;
    }

    env::set_current_dir(libc_path.as_ref()).map_err(ScanFilesError::PwdSetting)?;

    parse_files(fetch_details().map_err(|e| match e {
        FetchDetailsError::CargoMetadata => {
            ScanFilesError::WorkspaceScanning(libc_path.as_ref().to_owned())
        }
        FetchDetailsError::NoLibc => ScanFilesError::NoLibc(libc_path.as_ref().to_owned()),
    })?)
    .map_err(|ParseFilesError(path)| ScanFilesError::ParseError(path))
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

// TODO: finish the below two routines.

pub(crate) fn clone_repo(path: PathBuf) -> Result<(), CloneRepoError> {
    task::spawn_blocking(|| {
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
                        | create::Error::CreateDirectory { source: err, .. } => {
                            RepoErrorRepr::Clone {
                                path,
                                kind: CloneErrorKind::IoBound(err),
                            }
                        }
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
            .map(|_| ())
    })
    .await
}

pub(crate) fn prepare_clone(path: impl AsRef<Path>) -> Result<(), CloneRepoError> {
    gix::prepare_clone(LIBC_REPO, &path).map_err(|err| match err {
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
                create::Error::DirectoryExists { .. } | create::Error::DirectoryNotEmpty { .. } => {
                    RepoErrorRepr::Clone {
                        path,
                        kind: CloneErrorKind::DirectoryNotEmpty,
                    }
                }
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
        clone::Error::UrlParse(_) | clone::Error::CanonicalizeUrl { .. } => RepoErrorRepr::Clone {
            path,
            kind: CloneErrorKind::LibcUrl,
        },
    })
}

pub(crate) fn fetch_details() -> Result<Vec<PathBuf>, FetchDetailsError> {
    Ok(WalkDir::new(
        MetadataCommand::new()
            .exec()
            .map_err(|_| FetchDetailsError::CargoMetadata)?
            .workspace_packages()
            .iter()
            .find_map(|&pkg| {
                (pkg.name == "libc").then(|| pkg.manifest_path.parent().unwrap().to_owned())
            })
            .ok_or(FetchDetailsError::NoLibc)?,
    )
    .sort_by_file_name()
    .contents_first(true)
    .into_iter()
    .filter_map(|entry| {
        entry.ok().map(|entry| {
            (entry.file_type().is_file())
                .then(|| entry.into_path())
                .filter(|inner| inner.extension().is_some_and(|ext| ext == "rs"))
        })?
    })
    .collect())
}

pub(crate) fn parse_files(files: Vec<PathBuf>) -> Result<Vec<SourceFile>, ParseFilesError> {
    files.into_iter().try_fold(Vec::new(), |mut out, file| {
        out.push(SourceFile::new(
            syn::parse_file(
                &std::fs::read_to_string(&file).map_err(|_| ParseFilesError(file.clone()))?,
            )
            .map_err(|_| ParseFilesError(file.clone()))?,
            file,
        ));

        Ok::<_, ParseFilesError>(out)
    })
}
