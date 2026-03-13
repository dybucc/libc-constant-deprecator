//! The library entry-point is the `scan_files()` function. It takes in a single
//! path, and attempts to scan the given directory for Rust files, or
//! alternatively clones into the given directory the `libc` crate from
//! upstream. This process involves querying the workspace to check that,
//! indeed, it is a Rust workspace, and only then proceeds to start scanning the
//! files. If the function clones the repo, the same source file scanning
//! process as detailed in [Source file scanning] will also take place.
//!
//! # Source file scanning
//!
//! The library uses [`syn`] to scan source files. Source file workspace package
//! locations are provided by the details of the `cargo-metadata` command run on
//! the Rust workspace, and is then parsed with the corresponding
//! [`cargo_metadata`] crate. If one of these processes fails, `scan_files()`
//! returns with [`Result::Err`]. Upon locating all source files, it parses each
//! into a [`syn::File`], and returns a collection of parsed token trees.
//!
//! # Source file parsing
//!
//! Upon completing the source file scanning process, the entry point to
//! manipulating constants goes through the [`parse_constants()`] routine, which
//! should be fed the output of the [`scan_files()`] routine, and should return
//! a set of syntax trees for constants abstracted over the [`Const`] type. This
//! type generalizes over all places a constant may be found in Rust, including
//! inherent impl blocks, trait impl blocks, and item-level constants. This type
//! is the entry point to the rest of the functionality concerning saving
//! records to disk reflecting both the constants, as well as the source file in
//! which they were found.
//!
//! # Manipulating syntax trees for constants
//!
//! Upon finishing the parsing process, a vector of [`Const`] items is returned.
//! An extension trait for this specific type (`Vec<Const>`) is included in the
//! library to facilitate bulk manipulation of these syntax trees. The allowed
//! operations include general getter/setter methods for both a specific
//! constant value, as well as methods to bulk drop the contents of the vector
//! onto a file of choice. This last operation uses a specific format to keep
//! track of both the constant's current identifier and the source file from
//! which the constant was sourced from. The format also follows a simple
//! mnemonic where the latest commit SHA-1 onto which parsing was performed is
//! recorded in the first 40 bytes of the text file. This should allow updating
//! the current values of constants in the file where some changes to be made
//! that would otherwise corrupt the data found in the record files (e.g.
//! changing the location of some constant's identifier.) Operations for this
//! are also provided in the extension trait.

#![feature(bool_to_result, exit_status_error, string_from_utf8_lossy_owned)]

use std::{
    env,
    fs,
    io,
    ops::Deref,
    path::{Path, PathBuf},
    process::Command,
    slice::SliceIndex,
    sync::atomic::AtomicBool,
};

use cargo_metadata::MetadataCommand;
use gix::progress::Discard;
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

const LIBC_REPO: &str = "https://github.com/rust-lang/libc.git";

#[derive(Debug, Error)]
pub enum ScanFilesError {
    #[expect(private_interfaces, reason = "The whole point is to make this opaque.")]
    #[error("failed to set pwd: {0}")]
    PwdSetting(PathErrorRepr),
    #[error(
        "directory `{0}` doesn't exist; both cloning and sourcing an existing copy of `libc` \
             require a preexisting directory"
    )]
    MissingDirectoryAccess(PathBuf),
    #[error("error while cloning git repo to path {0}")]
    RepoCloningError(PathBuf),
    #[error("directory `{0}` doesn't contain a cargo workspace with `libc` in it")]
    NoLibc(PathBuf),
    #[error("workspace querying through `cargo-metadata` failed for directory `{0}`")]
    WorkspaceScanning(PathBuf),
    #[error("failed parsing rust source file `{0}`")]
    ParseError(PathBuf),
}

#[derive(Debug, Error)]
#[error(transparent)]
pub(crate) struct PathErrorRepr(#[from] io::Error);

#[derive(Debug)]
pub struct SourceFile {
    contents: File,
    source:   PathBuf,
}

impl SourceFile {
    pub(crate) fn new(contents: impl Into<File>, path: impl AsRef<Path>) -> Self {
        Self { contents: contents.into(), source: path.as_ref().to_owned() }
    }

    pub(crate) fn contents(&self) -> &File { &self.contents }

    pub(crate) fn into_contents(self) -> File { self.contents }

    pub(crate) fn path(&self) -> &Path { &self.source }

    pub(crate) fn into_path(self) -> PathBuf { self.source }
}

pub fn scan_files<T: AsRef<Path>>(libc_path: &T) -> Result<Vec<SourceFile>, ScanFilesError> {
    // TODO: instead of only interacting with git to clone the repo, use it to
    // both verify that it's, indeed, the `libc` repo, and to extract the
    // current worktree's commit sha-1 to embed into the file that will persist
    // the current changes to constants in memory.

    (!libc_path.as_ref().try_exists().is_ok_and(|inner| inner))
        .ok_or_else(|| ScanFilesError::MissingDirectoryAccess(libc_path.as_ref().to_owned()))?;
    if libc_path.as_ref().read_dir().map(|libc_path| libc_path.count() == 0).unwrap_or(false) {
        // Simply clone the repo and let it persist in memory.
        gix::prepare_clone(LIBC_REPO, libc_path.as_ref())
            .map_err(|_| ScanFilesError::RepoCloningError(libc_path.as_ref().to_owned()))?
            .fetch_then_checkout(Discard, &AtomicBool::new(false))
            .map_err(|_| ScanFilesError::RepoCloningError(libc_path.as_ref().to_owned()))?
            .0
            .main_worktree(Discard, &AtomicBool::new(false))
            .map_err(|_| ScanFilesError::RepoCloningError(libc_path.as_ref().to_owned()))?;
    }
    env::set_current_dir(libc_path.as_ref())
        .map_err(|e| ScanFilesError::PwdSetting(PathErrorRepr(e)))?;
    let files = fetch_details().map_err(|e| match e {
        | FetchDetailsError::CargoMetadata =>
            ScanFilesError::WorkspaceScanning(libc_path.as_ref().to_owned()),
        | FetchDetailsError::NoLibc => ScanFilesError::NoLibc(libc_path.as_ref().to_owned()),
    })?;

    parse_files(files).map_err(|ParseFilesError(path)| ScanFilesError::ParseError(path))
}

#[derive(Debug)]
pub(crate) enum FetchDetailsError {
    CargoMetadata,
    NoLibc,
}

pub(crate) fn fetch_details() -> Result<Vec<PathBuf>, FetchDetailsError> {
    let metadata = MetadataCommand::new().exec().map_err(|_| FetchDetailsError::CargoMetadata)?;
    let packages = metadata.workspace_packages();
    let libc_pkg = packages
        .iter()
        .find_map(|&pkg| {
            (pkg.name == "libc").then(|| pkg.manifest_path.parent().unwrap().to_owned())
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
                .filter(|inner| inner.extension().map(|ext| ext == "rs").unwrap_or(false))
        })
        .collect();

    Ok(files)
}

#[derive(Debug)]
pub(crate) struct ParseFilesError(PathBuf);

pub(crate) fn parse_files(files: Vec<PathBuf>) -> Result<Vec<SourceFile>, ParseFilesError> {
    // TODO: see how viable it is to expand macros for all of the files, as it's
    // highly unlikely that we would be able to perform another roundtrip from
    // the expanded output's span to the original source code's span when
    // modifying constants.

    files.into_iter().try_fold(Vec::new(), |mut files, file| {
        files.push(SourceFile::new(
            syn::parse_file(&fs::read_to_string(&file).map_err(|_| ParseFilesError(file.clone()))?)
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
                        | Item::Impl(impl_block) => Some(process_impl_block(impl_block, source)),
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

pub(crate) fn process_constant(constant: &ItemConst, source: impl AsRef<Path>) -> Vec<Const> {
    vec![Const::from_item(constant, source)]
}

pub(crate) fn process_impl_block(block: &ItemImpl, source: impl AsRef<Path>) -> Vec<Const> {
    block
        .items
        .iter()
        .filter_map(|item| if let ImplItem::Const(constant) = item { Some(constant) } else { None })
        .fold(Vec::new(), |mut constants, item| {
            constants.push(Const::from_impl(item, &source));

            constants
        })
}

pub(crate) fn process_trait_block(block: &ItemTrait, source: impl AsRef<Path>) -> Vec<Const> {
    block
        .items
        .iter()
        .filter_map(
            |item| if let TraitItem::Const(constant) = item { Some(constant) } else { None },
        )
        .fold(Vec::new(), |mut constants, item| {
            constants.push(Const::from_trait(item, &source));

            constants
        })
}

#[derive(Debug)]
pub struct Const {
    repr:   ConstRepr,
    ident:  Ident,
    source: PathBuf,
}

#[derive(Debug)]
pub(crate) enum ConstRepr {
    Item(ItemConst),
    Trait(TraitItemConst),
    Impl(ImplItemConst),
}

#[derive(Debug)]
pub(crate) enum ConstUpdateError {
    IoBound(io::Error),
    Parsing(syn::Error),
}

pub(crate) struct Similarity {
    inner:      Const,
    percentage: usize,
}

pub(crate) trait ConstConvertible {
    fn convert(&self, source: impl AsRef<Path>) -> Const;
}

impl ConstConvertible for ItemConst {
    fn convert(&self, source: impl AsRef<Path>) -> Const { self.convert(source) }
}

impl ConstConvertible for &ItemConst {
    fn convert(&self, source: impl AsRef<Path>) -> Const { Const::from_item(self, source) }
}

impl ConstConvertible for &mut ItemConst {
    fn convert(&self, source: impl AsRef<Path>) -> Const { self.convert(source) }
}

impl ConstConvertible for TraitItemConst {
    fn convert(&self, source: impl AsRef<Path>) -> Const { self.convert(source) }
}

impl ConstConvertible for &TraitItemConst {
    fn convert(&self, source: impl AsRef<Path>) -> Const { Const::from_trait(self, source) }
}

impl ConstConvertible for &mut TraitItemConst {
    fn convert(&self, source: impl AsRef<Path>) -> Const { self.convert(source) }
}

impl ConstConvertible for ImplItemConst {
    fn convert(&self, source: impl AsRef<Path>) -> Const { self.convert(source) }
}

impl ConstConvertible for &ImplItemConst {
    fn convert(&self, source: impl AsRef<Path>) -> Const { Const::from_impl(self, source) }
}

impl ConstConvertible for &mut ImplItemConst {
    fn convert(&self, source: impl AsRef<Path>) -> Const { self.convert(source) }
}

impl Similarity {
    fn new(constant: impl ConstConvertible, source: impl AsRef<Path>) -> Self {
        Similarity { inner: constant.convert(source), percentage: 0 }
    }

    fn compare_with(&mut self, other: &Const) {
        let (self_repr, other_repr) = (self.inner.repr, other.repr);
        match (self_repr, other_repr) {
            | (
                ConstRepr::Item(ItemConst {
                    attrs: attrs1,
                    vis: vis1,
                    ident: ident1,
                    generics: generics1,
                    ty: ty1,
                    ..
                }),
                Const::Item(ItemConst {
                    attrs: attrs2,
                    vis: vis2,
                    ident: ident2,
                    generics: generics2,
                    ty: ty2,
                    ..
                }),
            ) => {
                todo!()
            },
            | _ => todo!(),
        }
    }
}

pub(crate) fn compare_constant(
    constant: &ItemConst,
    Const { repr, .. }: &Const,
    source: impl AsRef<Path>,
) -> Similarity {
    let mut similarity = Similarity::new(constant, source);

    todo!()
}

impl Const {
    fn from_item(item: &ItemConst, source: impl AsRef<Path>) -> Self {
        let ident = item.ident.clone();

        Self { repr: ConstRepr::Item(item.clone()), ident, source: source.as_ref().to_owned() }
    }

    fn from_trait(item: &TraitItemConst, source: impl AsRef<Path>) -> Self {
        let ident = item.ident.clone();

        Self { repr: ConstRepr::Trait(item.clone()), ident, source: source.as_ref().to_owned() }
    }

    fn from_impl(item: &ImplItemConst, source: impl AsRef<Path>) -> Self {
        let ident = item.ident.clone();

        Self { repr: ConstRepr::Impl(item.clone()), ident, source: source.as_ref().to_owned() }
    }

    fn update_source(&mut self, new_source: impl AsRef<Path>) -> Result<(), ConstUpdateError> {
        self.source = new_source.as_ref().to_owned();

        self.update(new_source)
    }

    fn update(&mut self, new_source: impl AsRef<Path>) -> Result<(), ConstUpdateError> {
        let file =
            syn::parse_file(&fs::read_to_string(new_source).map_err(ConstUpdateError::IoBound)?)
                .map_err(ConstUpdateError::Parsing)?;
        file.items.iter().for_each(|item| match item {
            | Item::Const(constant) => compare_constant(constant, &self, new_source),
            | Item::Impl(impl_block) => compare_impl_block(impl_block, &self, new_source),
            | Item::Trait(trait_block) => compare_trait_block(trait_block, &self, new_source),
        })?;

        Ok(())
    }
}

pub(crate) trait Sealed {}

#[expect(private_bounds, reason = "It's an intentional decission.")]
pub trait ConstVec: Sealed {
    fn get(&self, index: impl SliceIndex<[Const]>) -> Option<&[Const]>;
}

impl Sealed for Vec<Const> {}

impl ConstVec for Vec<Const> {
    fn get(&self, index: impl SliceIndex<[Const]>) -> Option<&[Const]> { todo!() }
}
