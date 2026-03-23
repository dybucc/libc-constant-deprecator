#![feature(bool_to_result, exit_status_error, string_from_utf8_lossy_owned)]

use std::{
  borrow::{Borrow, Cow},
  collections::HashMap,
  env,
  fs,
  io::{self, BufRead},
  iter,
  path::{Path, PathBuf},
  process::Command,
  sync::{LazyLock, atomic::AtomicBool},
};

use cargo_metadata::MetadataCommand;
use gix::progress::Discard;
use itertools::Itertools;
use proc_macro2::Span;
use quote::ToTokens;
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
  parse_quote,
};
use thiserror::Error;
use walkdir::WalkDir;

// TODO: implement functionality to both check and embed information on the
// location of a file with the constants formatted inside the `Cargo.toml` of
// the `libc` repo.

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

  #[expect(
    clippy::must_use_candidate,
    reason = "It's not a bug not to use this function."
  )]
  pub fn contents(&self) -> &File { &self.contents }

  #[expect(
    clippy::must_use_candidate,
    reason = "It's not a bug not to use this function."
  )]
  pub fn into_contents(self) -> File { self.contents }

  #[expect(
    clippy::must_use_candidate,
    reason = "It's not a bug not to use this function."
  )]
  pub fn path(&self) -> &Path { &self.source }

  #[expect(
    clippy::must_use_candidate,
    reason = "It's not a bug not to use this function."
  )]
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
    .map_err(PwdSettingRepr)
    .map_err(ScanFilesError::PwdSetting)?;
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

      (entry.file_type().is_file())
        .then(|| entry.into_path())
        .filter(|inner| inner.extension().is_some_and(|ext| ext == "rs"))
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

#[expect(
  clippy::must_use_candidate,
  reason = "It's not a bug not to use the result of this routine."
)]
pub fn parse_constants(files: &[SourceFile]) -> Vec<Const> {
  // TODO: handle inline modules (those that `syn` has parsed as having a
  // `Some(_)` in their `content` field) by checking recursively for all items
  // within them.
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

#[derive(Debug, Clone)]
pub struct Const {
  repr:       ConstRepr,
  ident:      Ident,
  source:     PathBuf,
  deprecated: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum ConstRepr {
  Item(ItemConst),
  Trait(TraitItemConst),
  Impl(ImplItemConst),
  File,
}

impl Const {
  pub(crate) fn from_item(item: &ItemConst, source: impl AsRef<Path>) -> Self {
    let ident = item.ident.clone();

    Self {
      repr: ConstRepr::Item(item.clone()),
      ident,
      source: source.as_ref().to_owned(),
      deprecated: false,
    }
  }

  pub(crate) fn from_trait(
    item: &TraitItemConst,
    source: impl AsRef<Path>,
  ) -> Self {
    let ident = item.ident.clone();

    Self {
      repr: ConstRepr::Trait(item.clone()),
      ident,
      source: source.as_ref().to_owned(),
      deprecated: false,
    }
  }

  pub(crate) fn from_impl(
    item: &ImplItemConst,
    source: impl AsRef<Path>,
  ) -> Self {
    let ident = item.ident.clone();

    Self {
      repr: ConstRepr::Impl(item.clone()),
      ident,
      source: source.as_ref().to_owned(),
      deprecated: false,
    }
  }

  pub(crate) fn from_file(
    ident: Ident,
    source: impl AsRef<Path>,
    deprecated: bool,
  ) -> Self {
    Self {
      repr: ConstRepr::File,
      ident,
      source: source.as_ref().to_owned(),
      deprecated,
    }
  }
}

#[derive(Debug, Error)]
#[error("{}", match .0 {
  FetchErrorRepr::IoBound(repr) => match repr {
    IoBoundErrorKind::StdType(inner) =>
      format!("failed while reading in input file: {inner}"),
    IoBoundErrorKind::Parsing { inner, line_num } =>
      format!("io bound error on line `{line_num}` while parsing: {inner}"),
  },
  FetchErrorRepr::ParseError { source, line_num, non_matching } =>
    format!(
      "failed parsing; bad {} at line {line_num}: `{non_matching}`",
      match source {
        ParseErrorSrc::Constant => "constant",
        ParseErrorSrc::Path => "path"
      }
    ),
})]
pub struct FetchError(FetchErrorRepr);

#[derive(Debug)]
pub(crate) enum FetchErrorRepr {
  IoBound(IoBoundErrorKind),
  ParseError {
    source:       ParseErrorSrc,
    line_num:     usize,
    non_matching: Cow<'static, str>,
  },
}

#[derive(Debug)]
pub(crate) enum IoBoundErrorKind {
  // This serves as an error type for unknown, I/O-bound errors that happen
  // prior to starting the parsing process.
  StdType(io::Error),
  // This corresponds with error types that take place during parsing.
  Parsing { inner: io::Error, line_num: usize },
}

#[derive(Debug)]
pub(crate) enum ParseErrorSrc {
  Constant,
  Path,
}

#[derive(Debug, Error)]
#[error("failed to save constants to file: `{0}`")]
pub struct SaveError(io::Error);

#[derive(Debug, Error)]
pub enum FilterError {
  #[error("regex compilation failed for needle: `{input_str}`")]
  RegexCompilation { input_str: String },
}

#[derive(Debug, Error)]
pub enum MakeChangesError {
  #[error("io error while {}: `{inner}`", match .origin {
    ChangesSrc::FetchOp(erred_path) =>
      format!("fetching file {}", erred_path.display()),
    ChangesSrc::SaveOp(erred_path) =>
      format!("saving file {}", erred_path.display())
  })]
  IoBound { origin: ChangesSrc, inner: io::Error },
}

#[derive(Debug)]
pub(crate) enum ChangesSrc {
  FetchOp(PathBuf),
  SaveOp(PathBuf),
}

#[derive(Debug)]
pub struct ConstContainer {
  inner:    Vec<Const>,
  re_cache: HashMap<String, Regex>,
}

macro_rules! deprecate {
  ($msg:expr) => {{
    let msg = $msg;
    parse_quote! { #[deprecated(since = "1.0.0", note = #msg)] }
  }};
}

impl ConstContainer {
  pub(crate) const DEPRECATION_NOTICE: &str =
    "This constant, among others often used in C for the purposes of denoting \
     the latest value or limit in a set of constants, has been deprecated. \
     See #3131 for details and discussion.";

  pub(crate) fn new(constants: Vec<Const>) -> Self {
    Self { inner: constants, re_cache: HashMap::new() }
  }

  pub fn fetch_from_disk(path: impl AsRef<Path>) -> Result<Self, FetchError> {
    let file = fs::read_to_string(path).map_err(|inner| {
      FetchError(FetchErrorRepr::IoBound(IoBoundErrorKind::StdType(inner)))
    })?;

    ConstFormat::parse_file(file).map_err(|inner| {
      FetchError({
        match inner {
          | ParseError::LineReading { line_num, inner } =>
            FetchErrorRepr::IoBound(IoBoundErrorKind::Parsing {
              inner,
              line_num,
            }),
          | ParseError::ExtraneousInput { input, expected, line_num } =>
            FetchErrorRepr::ParseError {
              source: match expected {
                | ConstFormatToken::Constant => ParseErrorSrc::Constant,
                | ConstFormatToken::Path => ParseErrorSrc::Path,
              },
              line_num,
              non_matching: input,
            },
        }
      })
    })
  }

  pub fn save_to_disk(&self, path: impl AsRef<Path>) -> Result<(), SaveError> {
    let contents = FullAllocator::coarse_allocate(&self.inner);

    fs::write(path, &contents).map_err(SaveError)
  }

  #[expect(
    clippy::missing_panics_doc,
    reason = "The only source of panics is devoid of meaning because it \
              performs a bounds-access computation right after making sure \
              there's at least one element in the container at hand."
  )]
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

  pub fn effect_changes(&self) -> Result<(), MakeChangesError> {
    // TODO: expand macros prior to parsing contents with `syn`.
    self.inner.iter().try_for_each(|constant @ Const { source, .. }| {
      let mut file =
        syn::parse_file(&fs::read_to_string(source).map_err(|inner| {
          MakeChangesError::IoBound {
            origin: ChangesSrc::FetchOp(source.clone()),
            inner,
          }
        })?)
        .map_err(|_| todo!())?;
      file.items.iter_mut().for_each(|item| {
        change_constant_in(item, constant);
      });
      fs::write(source, file.into_token_stream().to_string()).map_err(
        |inner| MakeChangesError::IoBound {
          origin: ChangesSrc::SaveOp(source.clone()),
          inner,
        },
      )?;

      Ok(())
    })
  }
}

pub(crate) fn change_constant_in(
  item: &mut Item,
  constant @ Const { ident: ref_ident, deprecated, .. }: &Const,
) {
  let msg = ConstContainer::DEPRECATION_NOTICE;
  match item {
    | Item::Const(ItemConst { attrs, ident, .. })
      if ident == ref_ident && *deprecated =>
      attrs.push(deprecate!(msg)),
    | Item::Impl(ItemImpl { items, .. }) => items
      .iter_mut()
      .filter_map(|item| {
        if let ImplItem::Const(constant) = item
          && let ImplItemConst { ident, .. } = constant
          && ident == ref_ident
          && *deprecated
        {
          Some(constant)
        } else {
          None
        }
      })
      .for_each(|ImplItemConst { attrs, .. }| attrs.push(deprecate!(msg))),
    | Item::Trait(ItemTrait { items, .. }) => items
      .iter_mut()
      .filter_map(|item| {
        if let TraitItem::Const(constant) = item
          && let TraitItemConst { ident, .. } = constant
          && ident == ref_ident
          && *deprecated
        {
          Some(constant)
        } else {
          None
        }
      })
      .for_each(|TraitItemConst { attrs, .. }| attrs.push(deprecate!(msg))),
    | Item::Mod(syn::ItemMod { content: Some((_, content)), .. }) =>
      for item in content.iter_mut() {
        change_constant_in(item, constant);
      },
    | _ => (),
  }
}

#[derive(Debug)]
pub(crate) enum ParseError {
  LineReading {
    line_num: usize,
    inner:    io::Error,
  },
  ExtraneousInput {
    input:    Cow<'static, str>,
    expected: ConstFormatToken,
    line_num: usize,
  },
}

#[derive(Debug)]
pub(crate) enum ConstFormatToken {
  Constant,
  Path,
}

static CONSTANT_READER: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^[[:alnum:][:punct:]]+(\s\[deprecated\])?$").unwrap()
});

static PATH_READER: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^(/[[:ascii:]]*)+$").unwrap());

// Example grammar for document holding information on constants:
// (<ident>( <deprecated>)?\n<path-to-ident-decl>\n)*
#[derive(Debug)]
pub(crate) struct ConstFormat;

impl ConstFormat {
  pub(crate) fn parse_file(
    input: impl AsRef<[u8]>,
  ) -> Result<ConstContainer, ParseError> {
    let (mut buf, mut line_counter, mut inner) =
      (String::with_capacity(input.as_ref().len()), 0, Vec::new());
    while input.as_ref().read_line(&mut buf).map_err(|inner| {
      ParseError::LineReading { line_num: line_counter + 1, inner }
    })?
      != 0
    {
      line_counter += 1;
      if CONSTANT_READER
        .is_match(buf.as_bytes())
        .ok_or_else(|| ParseError::ExtraneousInput {
          input:    Cow::from(buf.clone()),
          expected: ConstFormatToken::Constant,
          line_num: line_counter + 1,
        })
        .map(|()| true)?
        && let (components, check) = {
          let components: Vec<String> =
            buf.split_ascii_whitespace().map_into().collect();
          buf.clear();
          input.as_ref().read_line(&mut buf).map_err(|inner| {
            ParseError::LineReading { line_num: line_counter + 1, inner }
          })?;
          line_counter += 1;

          (components, PATH_READER.is_match(buf.as_bytes()))
        }
        && check
          .ok_or_else(|| ParseError::ExtraneousInput {
            input:    Cow::from(buf.clone()),
            expected: ConstFormatToken::Path,
            line_num: line_counter + 1,
          })
          .map(|()| true)?
      {
        inner.push(Const::from_file(
          Ident::new(
            components.first().expect(
              "there should be at least one token if `PATH_READER` matched",
            ),
            Span::call_site(),
          ),
          PathBuf::from(buf.trim()),
          components.len() > 1,
        ));
      }
      buf.clear();
    }

    Ok(ConstContainer::new(inner))
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
