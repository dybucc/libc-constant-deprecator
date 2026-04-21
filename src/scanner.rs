use std::{
    env,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

use cargo_metadata::MetadataCommand;
use gix::{
    discover::{self, upwards},
    progress::Discard,
};
use tokio::{fs, task};
use walkdir::WalkDir;

use crate::{FetchDetailsError, ParseFilesError, RepoErrorRepr, ScanFilesError, SourceFile};

pub(crate) const LIBC_REPO: &str = "https://github.com/rust-lang/libc.git";

pub async fn scan_files(libc_path: impl AsRef<Path>) -> Result<Vec<SourceFile>, ScanFilesError> {
    let path = libc_path.as_ref().to_owned().clone();

    // TODO: finish this up and keep working on making other I/O-bound operations
    // async.
    if !fs::try_exists(&libc_path)
        .await
        .map_err(ScanFilesError::IoBound)
    {
        task::spawn(async move {
            gix::prepare_clone(LIBC_REPO, path)
                .map_err(|_| ScanFilesError::RepoError(RepoErrorRepr::Clone(path)))
        })
        .await;
    } else {
        task::spawn(async move {
            gix::discover(path).map_err(|err| match err {
                discover::Error::Discover(err) => match err {
                    upwards::Error::CurrentDir(err) => todo!(),
                    upwards::Error::InvalidInput { directory } => todo!(),
                    upwards::Error::InaccessibleDirectory { path } => todo!(),
                    upwards::Error::NoGitRepository { path } => todo!(),
                    upwards::Error::NoGitRepositoryWithinCeiling {
                        path,
                        ceiling_height,
                    } => todo!(),
                    upwards::Error::NoGitRepositoryWithinFs { path, limit } => todo!(),
                    upwards::Error::NoMatchingCeilingDir => todo!(),
                    upwards::Error::NoTrustedGitRepository {
                        path,
                        candidate,
                        required,
                    } => todo!(),
                    upwards::Error::CheckTrust { path, err } => todo!(),
                },
                discover::Error::Open(err) => todo!(),
            })
        })
        .await;
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
            syn::parse_file(&fs::read_to_string(&file).map_err(|_| ParseFilesError(file.clone()))?)
                .map_err(|_| ParseFilesError(file.clone()))?,
            file,
        ));

        Ok::<_, ParseFilesError>(out)
    })
}
