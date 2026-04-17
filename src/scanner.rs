use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

use cargo_metadata::MetadataCommand;
use gix::progress::Discard;
use walkdir::WalkDir;

use crate::{FetchDetailsError, ParseFilesError, ScanFilesError, SourceFile};

pub(crate) const LIBC_REPO: &str = "https://github.com/rust-lang/libc.git";

pub fn scan_files(libc_path: impl AsRef<Path>) -> Result<Vec<SourceFile>, ScanFilesError> {
    // TODO: instead of only interacting with git to clone the repo, use it to
    // both verify that it's, indeed, the `libc` repo, and to extract the current
    // worktree's commit sha-1 to embed into the file that will persist the
    // current changes to constants in memory.

    (!libc_path.as_ref().try_exists().is_ok_and(|inner| inner))
        .ok_or_else(|| ScanFilesError::MissingDirectoryAccess(libc_path.as_ref().to_owned()))?;
    libc_path
        .as_ref()
        .read_dir()
        .is_ok_and(|libc_path| libc_path.count() == 0)
        .then_some(())
        .iter()
        .try_for_each(|()| {
            gix::prepare_clone(LIBC_REPO, libc_path.as_ref())
                .map_err(|_| ScanFilesError::RepoCloningError(libc_path.as_ref().to_owned()))?
                .fetch_then_checkout(Discard, &AtomicBool::new(false))
                .map_err(|_| ScanFilesError::RepoCloningError(libc_path.as_ref().to_owned()))?
                .0
                .main_worktree(Discard, &AtomicBool::new(false))
                .map_err(|_| ScanFilesError::RepoCloningError(libc_path.as_ref().to_owned()))?;

            Ok(())
        })?;
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
    let mut out = Vec::new();

    for file in files {
        out.push(SourceFile::new(
            syn::parse_file(&fs::read_to_string(&file).map_err(|_| ParseFilesError(file.clone()))?)
                .map_err(|_| ParseFilesError(file.clone()))?,
            file,
        ));
    }

    Ok(out)
}
