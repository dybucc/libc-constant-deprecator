#![feature(
  bool_to_result, exit_status_error, string_from_utf8_lossy_owned,
  iter_next_chunk
)]

use std::{
  borrow::{Borrow, Cow},
  collections::HashMap,
  env,
  fmt::{self, Display, Formatter},
  fs,
  io::{self, BufRead},
  iter,
  path::{Path, PathBuf},
  process::Command,
  str::FromStr,
  sync::{LazyLock, atomic::AtomicBool},
};

use cargo_metadata::MetadataCommand;
use gix::progress::Discard;
use itertools::Itertools;
use proc_macro2::Span;
use regex::bytes::{Regex, RegexBuilder};
use syn::{
  File,
  Ident,
  ImplItem,
  ImplItemConst,
  Item,
  ItemConst,
  ItemImpl,
  ItemTrait,
  TraitItem,
  TraitItemConst,
};
use thiserror::Error;
use walkdir::WalkDir;

// TODO: implement functionality to both check and embed information on the
// location of a vector of constants inside the `Cargo.toml` of the `libc` repo.

const LIBC_REPO: &str = "https://github.com/rust-lang/libc.git";

#[derive(Debug, Error)]
pub enum ScanFilesError {
  #[expect(
    private_interfaces,
    reason = "The whole point is to make this opaque."
  )]
  #[error("failed to set pwd: {0}")]
  PwdSetting(PwdSettingRepr),
  #[error(
    "directory `{0}` doesn't exist; both cloning and sourcing an existing \
     copy of `libc` require a preexisting directory"
  )]
  MissingDirectoryAccess(PathBuf),
  #[error("error while cloning git repo to path {0}")]
  RepoCloningError(PathBuf),
  #[error(
    "directory `{0}` doesn't contain a cargo workspace with `libc` in it"
  )]
  NoLibc(PathBuf),
  #[error(
    "workspace querying through `cargo-metadata` failed for directory `{0}`"
  )]
  WorkspaceScanning(PathBuf),
  #[error("failed parsing rust source file `{0}`")]
  ParseError(PathBuf),
}

#[derive(Debug, Error)]
#[error(transparent)]
pub(crate) struct PwdSettingRepr(#[from] io::Error);

#[derive(Debug)]
pub struct SourceFile {
  contents: File,
  source:   PathBuf,
}

impl SourceFile {
  pub(crate) fn new(contents: impl Into<File>, path: impl AsRef<Path>) -> Self {
    Self { contents: contents.into(), source: path.as_ref().to_owned() }
  }

  pub fn contents(&self) -> &File { &self.contents }

  pub fn into_contents(self) -> File { self.contents }

  pub fn path(&self) -> &Path { &self.source }

  pub fn into_path(self) -> PathBuf { self.source }
}

pub fn scan_files<T: AsRef<Path>>(
  libc_path: &T,
) -> Result<Vec<SourceFile>, ScanFilesError> {
  // TODO: instead of only interacting with git to clone the repo, use it to
  // both verify that it's, indeed, the `libc` repo, and to extract the
  // current worktree's commit sha-1 to embed into the file that will persist
  // the current changes to constants in memory.

  (!libc_path.as_ref().try_exists().is_ok_and(|inner| inner)).ok_or_else(
    || ScanFilesError::MissingDirectoryAccess(libc_path.as_ref().to_owned()),
  )?;
  libc_path
    .as_ref()
    .read_dir()
    .map(|libc_path| libc_path.count() == 0)
    .unwrap_or(false)
    .then_some(())
    .iter()
    .try_for_each(|_| {
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
    .map_err(|e| ScanFilesError::PwdSetting(PwdSettingRepr(e)))?;
  let files = fetch_details().map_err(|e| match e {
    | FetchDetailsError::CargoMetadata =>
      ScanFilesError::WorkspaceScanning(libc_path.as_ref().to_owned()),
    | FetchDetailsError::NoLibc =>
      ScanFilesError::NoLibc(libc_path.as_ref().to_owned()),
  })?;

  parse_files(files)
    .map_err(|ParseFilesError(path)| ScanFilesError::ParseError(path))
}

#[derive(Debug)]
pub(crate) enum FetchDetailsError {
  CargoMetadata,
  NoLibc,
}

pub(crate) fn fetch_details() -> Result<Vec<PathBuf>, FetchDetailsError> {
  let metadata = MetadataCommand::new()
    .exec()
    .map_err(|_| FetchDetailsError::CargoMetadata)?;
  let packages = metadata.workspace_packages();
  let libc_pkg = packages
    .iter()
    .find_map(|&pkg| {
      (pkg.name == "libc")
        .then(|| pkg.manifest_path.parent().unwrap().to_owned())
    })
    .ok_or(FetchDetailsError::NoLibc)?;
  let files: Vec<_> = WalkDir::new(libc_pkg)
    .sort_by_file_name()
    .contents_first(true)
    .into_iter()
    .filter_map(|entry| {
      let entry = entry.ok()?;

      (entry.file_type().is_file()).then(|| entry.into_path()).filter(|inner| {
        inner.extension().map(|ext| ext == "rs").unwrap_or(false)
      })
    })
    .collect();

  Ok(files)
}

#[derive(Debug)]
pub(crate) struct ParseFilesError(PathBuf);

pub(crate) fn parse_files(
  files: Vec<PathBuf>,
) -> Result<Vec<SourceFile>, ParseFilesError> {
  // TODO: see how viable it is to expand macros for all of the files, as it's
  // highly unlikely that we would be able to perform another roundtrip from
  // the expanded output's span to the original source code's span when
  // modifying constants.

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

#[derive(Debug)]
pub(crate) enum ExpansionError {
  ExpansionCommand,
}

#[expect(unused, reason = "Macro expansion is not fully implemented just yet.")]
pub(crate) fn expand_macros(file: File) -> Result<File, ExpansionError> {
  let command = Command::new("cargo")
    .args(["expand"])
    .output()
    .map_err(|_| ExpansionError::ExpansionCommand)?
    .exit_ok()
    .map_err(|_| ExpansionError::ExpansionCommand)?;
  let out = String::from_utf8_lossy_owned(command.stdout);

  todo!()
}

pub fn parse_constants(files: Vec<SourceFile>) -> Vec<Const> {
  files.iter().fold(
    Vec::with_capacity(files.len()),
    |mut parsed_files, SourceFile { contents, source }| {
      parsed_files.append(
        &mut contents
          .items
          .iter()
          .filter_map(|item| match item {
            | Item::Const(constant) => Some(process_constant(constant, source)),
            | Item::Impl(impl_block) =>
              Some(process_impl_block(impl_block, source)),
            | Item::Trait(trait_block) =>
              Some(process_trait_block(trait_block, source)),
            | _ => None,
          })
          .fold(Vec::new(), |mut constants, mut item| {
            constants.append(&mut item);

            constants
          }),
      );

      parsed_files
    },
  )
}

pub(crate) fn process_constant(
  constant: &ItemConst,
  source: impl AsRef<Path>,
) -> Vec<Const> {
  vec![Const::from_item(constant, source)]
}

pub(crate) fn process_impl_block(
  block: &ItemImpl,
  source: impl AsRef<Path>,
) -> Vec<Const> {
  block
    .items
    .iter()
    .filter_map(|item| {
      if let ImplItem::Const(constant) = item { Some(constant) } else { None }
    })
    .fold(Vec::new(), |mut constants, item| {
      constants.push(Const::from_impl(item, &source));

      constants
    })
}

pub(crate) fn process_trait_block(
  block: &ItemTrait,
  source: impl AsRef<Path>,
) -> Vec<Const> {
  block
    .items
    .iter()
    .filter_map(|item| {
      if let TraitItem::Const(constant) = item { Some(constant) } else { None }
    })
    .fold(Vec::new(), |mut constants, item| {
      constants.push(Const::from_trait(item, &source));

      constants
    })
}

#[derive(Debug)]
pub struct Const {
  repr:       ConstRepr,
  ident:      Ident,
  source:     PathBuf,
  deprecated: bool,
}

#[derive(Debug)]
pub(crate) enum ConstRepr {
  Item(ItemConst),
  Trait(TraitItemConst),
  Impl(ImplItemConst),
  File,
}

impl Const {
  fn from_item(item: &ItemConst, source: impl AsRef<Path>) -> Self {
    let ident = item.ident.clone();

    Self {
      repr: ConstRepr::Item(item.clone()),
      ident,
      source: source.as_ref().to_owned(),
      deprecated: false,
    }
  }

  fn from_trait(item: &TraitItemConst, source: impl AsRef<Path>) -> Self {
    let ident = item.ident.clone();

    Self {
      repr: ConstRepr::Trait(item.clone()),
      ident,
      source: source.as_ref().to_owned(),
      deprecated: false,
    }
  }

  fn from_impl(item: &ImplItemConst, source: impl AsRef<Path>) -> Self {
    let ident = item.ident.clone();

    Self {
      repr: ConstRepr::Impl(item.clone()),
      ident,
      source: source.as_ref().to_owned(),
      deprecated: false,
    }
  }
}

#[derive(Debug, Error)]
#[error("failed to {ty} file with constants: {inner}")]
pub struct FsError {
  inner: io::Error,
  ty:    FsErrorKind,
}

#[derive(Debug)]
pub enum FsErrorKind {
  SaveOp,
  FetchOp,
}

impl Display for FsErrorKind {
  fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
    match self {
      | Self::SaveOp => write!(f, "save"),
      | Self::FetchOp => write!(f, "fetch"),
    }
  }
}

#[derive(Debug, Error)]
pub enum FilterError {
  #[error("regex compilation failed for needle: `{input_str}`")]
  RegexCompilation { input_str: String },
}

#[derive(Debug)]
pub struct ConstContainer {
  inner:    Vec<Const>,
  re_cache: HashMap<String, Regex>,
}

impl ConstContainer {
  // Example grammar for document holding information on constants:
  // (<ident>( <deprecated>)?\n<path-to-ident-decl>\n)*
  pub fn fetch_from_disk(path: impl AsRef<Path>) -> Result<Self, FsError> {
    let file = fs::read_to_string(path)
      .map_err(|inner| FsError { inner, ty: FsErrorKind::FetchOp })?;

    todo!()
  }

  pub fn save_to_disk(&self, path: impl AsRef<Path>) -> Result<(), FsError> {
    let contents = FullAllocator::coarse_allocate(&self.inner);

    fs::write(path, &contents)
      .map_err(|inner| FsError { inner, ty: FsErrorKind::SaveOp })
  }

  pub fn filter(
    &mut self,
    re: impl AsRef<str>,
  ) -> Result<Vec<&Const>, FilterError> {
    let Self { inner, re_cache } = self;
    let re = match re_cache.get(re.as_ref()) {
      | Some(re) => re,
      | None => {
        re_cache.insert(
          re.as_ref().to_string(),
          build_re(&re).map_err(|_| FilterError::RegexCompilation {
            input_str: re.as_ref().to_string(),
          })?,
        );

        re_cache.get(re.as_ref()).unwrap()
      },
    };

    Ok(
      ContentAllocator::fine_allocate(inner)
        .iter()
        .enumerate()
        .filter_map(|(i, constant)| {
          re.is_match(constant).then_some(&self.inner[i])
        })
        .collect(),
    )
  }
}

#[derive(Debug, Error)]
pub(crate) enum FormatCreationError {
  #[error("failed to read (0-indexed) `0` line from start of input")]
  LineReading(usize),
}

static CONSTANT_READER: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^[[:alnum:][:punct:]]+(\s\[deprecated\])?$").unwrap()
});

static PATH_READER: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^(/[[:ascii:]]*)+$").unwrap());

#[derive(Debug)]
pub(crate) struct ConstFormat;

impl ConstFormat {
  pub(crate) fn parse_file(
    input: impl AsRef<[u8]>,
  ) -> Result<ConstContainer, FormatCreationError> {
    let (mut buf, mut line_counter, mut constant_buf, mut inner) =
      (String::with_capacity(input.as_ref().len()), 0, None, Vec::new());
    while let Ok(n) = input.as_ref().read_line(&mut buf)
      && n != 0
    {
      line_counter += 1;
      if CONSTANT_READER.is_match(buf.as_bytes())
        && let (components, check) = {
          let components: Vec<String> =
            buf.split_ascii_whitespace().map_into().collect();
          buf.clear();
          input
            .as_ref()
            .read_line(&mut buf)
            .map_err(|_| FormatCreationError::LineReading(line_counter))?;
          line_counter += 1;

          (components, PATH_READER.is_match(buf.as_bytes()))
        }
        && check
      {
        constant_buf = Some(Const {
          repr:       ConstRepr::File,
          ident:      Ident::new(
            components.first().expect(
              "if the regex matched, then there should be at least one token",
            ),
            Span::call_site(),
          ),
          source:     PathBuf::from(buf.trim()),
          deprecated: components.last().is_some(),
        });
      } else {
        constant_buf = None;
      }
      if let Some(constant) = constant_buf {
        inner.push(constant);
      }
      buf.clear();
    }

    Ok(ConstContainer { inner, re_cache: HashMap::new() })
  }
}

pub(crate) trait ConstAllocator {
  const DEPRECATED_ATTR: &str = "[deprecated]";

  fn coarse_allocate(input: impl IntoIterator<Item: Borrow<Const>>) -> Vec<u8>;

  fn fine_allocate(
    input: impl IntoIterator<Item: Borrow<Const>>,
  ) -> Vec<Vec<u8>>;
}

#[derive(Debug)]
pub(crate) struct ContentAllocator;

impl ConstAllocator for ContentAllocator {
  fn coarse_allocate(input: impl IntoIterator<Item: Borrow<Const>>) -> Vec<u8> {
    input
      .into_iter()
      .flat_map(|constant| {
        constant
          .borrow()
          .ident
          .to_string()
          .into_bytes()
          .into_iter()
          .chain(iter::once(b'\n'))
      })
      .collect()
  }

  fn fine_allocate(
    input: impl IntoIterator<Item: Borrow<Const>>,
  ) -> Vec<Vec<u8>> {
    input
      .into_iter()
      .map(|constant| constant.borrow().ident.to_string().into_bytes())
      .collect()
  }
}

#[derive(Debug)]
pub(crate) struct FullAllocator;

impl ConstAllocator for FullAllocator {
  fn coarse_allocate(input: impl IntoIterator<Item: Borrow<Const>>) -> Vec<u8> {
    input
      .into_iter()
      .flat_map(|constant| {
        constant
          .borrow()
          .ident
          .to_string()
          .into_bytes()
          .into_iter()
          .chain(
            if constant.borrow().deprecated {
              <Self as ConstAllocator>::DEPRECATED_ATTR
            } else {
              ""
            }
            .bytes(),
          )
          .chain(iter::once(b'\n'))
          .chain(
            constant.borrow().source.as_os_str().as_encoded_bytes().to_owned(),
          )
          .chain(iter::once(b'\n'))
      })
      .collect()
  }

  fn fine_allocate(
    input: impl IntoIterator<Item: Borrow<Const>>,
  ) -> Vec<Vec<u8>> {
    input
      .into_iter()
      .map(|constant| {
        let mut out = constant.borrow().ident.to_string().into_bytes();
        out.extend(
          if constant.borrow().deprecated {
            <Self as ConstAllocator>::DEPRECATED_ATTR
          } else {
            ""
          }
          .bytes()
          .chain(iter::once(b'\n'))
          .chain(
            constant
              .borrow()
              .source
              .as_os_str()
              .as_encoded_bytes()
              .iter()
              .copied(),
          ),
        );

        out
      })
      .collect()
  }
}

pub(crate) fn build_re(re: impl AsRef<str>) -> Result<Regex, regex::Error> {
  RegexBuilder::new(re.as_ref()).size_limit(512).case_insensitive(true).build()
}
