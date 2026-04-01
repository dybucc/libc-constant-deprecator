use std::{
  env,
  fs,
  path::{Path, PathBuf},
  process::Command,
  sync::atomic::AtomicBool,
};

use cargo_metadata::MetadataCommand;
use gix::progress::Discard;
use syn::File;
use walkdir::WalkDir;

use crate::{
  EDITION,
  ExpansionError,
  FetchDetailsError,
  LIBC_REPO,
  ParseFilesError,
  ScanFilesError,
  SourceFile,
};

pub fn scan_files(
  libc_path: impl AsRef<Path>,
) -> Result<Vec<SourceFile>, ScanFilesError> {
  // TODO: instead of only interacting with git to clone the repo, use it to
  // both verify that it's, indeed, the `libc` repo, and to extract the current
  // worktree's commit sha-1 to embed into the file that will persist the
  // current changes to constants in memory.

  (!libc_path.as_ref().try_exists().is_ok_and(|inner| inner)).ok_or_else(
    || ScanFilesError::MissingDirectoryAccess(libc_path.as_ref().to_owned()),
  )?;
  libc_path
    .as_ref()
    .read_dir()
    .is_ok_and(|libc_path| libc_path.count() == 0)
    .then_some(())
    .iter()
    .try_for_each(|()| {
      gix::prepare_clone(LIBC_REPO, libc_path.as_ref())
        .map_err(|_| {
          ScanFilesError::RepoCloningError(libc_path.as_ref().to_owned())
        })?
        .fetch_then_checkout(Discard, &AtomicBool::new(false))
        .map_err(|_| {
          ScanFilesError::RepoCloningError(libc_path.as_ref().to_owned())
        })?
        .0
        .main_worktree(Discard, &AtomicBool::new(false))
        .map_err(|_| {
          ScanFilesError::RepoCloningError(libc_path.as_ref().to_owned())
        })?;

      Ok(())
    })?;
  env::set_current_dir(libc_path.as_ref())
    .map_err(ScanFilesError::PwdSetting)?;

  parse_files(fetch_details().map_err(|e| match e {
    | FetchDetailsError::CargoMetadata =>
      ScanFilesError::WorkspaceScanning(libc_path.as_ref().to_owned()),
    | FetchDetailsError::NoLibc =>
      ScanFilesError::NoLibc(libc_path.as_ref().to_owned()),
  })?)
  .map_err(|ParseFilesError(path)| ScanFilesError::ParseError(path))
}

pub(crate) fn fetch_details() -> Result<Vec<PathBuf>, FetchDetailsError> {
  Ok(
    WalkDir::new(
      MetadataCommand::new()
        .exec()
        .map_err(|_| FetchDetailsError::CargoMetadata)?
        .workspace_packages()
        .iter()
        .find_map(|&pkg| {
          (pkg.name == "libc")
            .then(|| pkg.manifest_path.parent().unwrap().to_owned())
        })
        .ok_or(FetchDetailsError::NoLibc)?,
    )
    .sort_by_file_name()
    .contents_first(true)
    .into_iter()
    .filter_map(|entry| {
      let entry = entry.ok()?;

      (entry.file_type().is_file())
        .then(|| entry.into_path())
        .filter(|inner| inner.extension().is_some_and(|ext| ext == "rs"))
    })
    .collect(),
  )
}

pub(crate) fn parse_files(
  files: Vec<PathBuf>,
) -> Result<Vec<SourceFile>, ParseFilesError> {
  // TODO: see how viable it is to expand macros for all of the files, as it's
  // highly unlikely that we would be able to perform another roundtrip from the
  // expanded output's span to the original source code's span when modifying
  // constants.

  files.into_iter().try_fold(Vec::new(), |mut files, file| {
    files.push(SourceFile::new(
      syn::parse_file(
        &fs::read_to_string(&file)
          .map_err(|_| ParseFilesError(file.clone()))?,
      )
      .map_err(|_| ParseFilesError(file.clone()))?,
      file,
    ));

    Ok(files)
  })
}

#[expect(
  unused,
  clippy::needless_pass_by_value,
  reason = "Macro expansion is not fully implemented just yet."
)]
pub(crate) fn expand_macros(file: File) -> Result<File, ExpansionError> {
  let command = Command::new("rustc")
    .env("RUSTC_BOOTSTRAP", "1")
    .args([
      "-Zunpretty=expanded",
      "--edition",
      EDITION,
      env::current_dir()
        .map_err(|_| todo!())?
        .join("src/lib.rs")
        .to_str()
        .ok_or(todo!())?,
    ])
    .output()
    .map_err(|_| ExpansionError::ExpansionCommand)?
    .exit_ok()
    .map_err(|_| ExpansionError::ExpansionCommand)?;
  let out = String::from_utf8_lossy_owned(command.stdout);

  todo!()
}
