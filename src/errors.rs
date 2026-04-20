use std::{
    borrow::Cow,
    io,
    path::{Path, PathBuf},
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScanFilesError {
    #[error("failed to change pwd: {0}")]
    PwdSetting(io::Error),
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

#[derive(Debug)]
pub(crate) enum FetchDetailsError {
    CargoMetadata,
    NoLibc,
}

#[derive(Debug)]
pub(crate) struct ParseFilesError(pub(crate) PathBuf);

#[derive(Debug, Error)]
#[error(
    "{}",
    match .0 {
        FetchErrorRepr::IoBound(repr) => match repr {
            IoBoundErrorKind::Fs(inner) => format!("failed while reading in input file: {inner}"),
            IoBoundErrorKind::Parsing { inner, line_num } => {
                format!("io bound error on line `{line_num}` while parsing: {inner}")
            }
        },
        FetchErrorRepr::ParseError { source, line_num, non_matching } => {
            match source {
                ParseErrorSrc::Token => {
                    format!("failed parsing; bad input at line {line_num}: `{non_matching}`")
                }
                ParseErrorSrc::Eof => {
                    "unexpected eof; expected further input for complete parsing".to_string()
                },
            }
        }
    }
)]
pub struct FetchError(pub(crate) FetchErrorRepr);

#[derive(Debug)]
pub(crate) enum FetchErrorRepr {
    IoBound(IoBoundErrorKind),
    ParseError {
        source: ParseErrorSrc,
        line_num: usize,
        non_matching: Cow<'static, str>,
    },
}

#[derive(Debug)]
pub(crate) enum IoBoundErrorKind {
    Fs(io::Error),
    Parsing { inner: io::Error, line_num: usize },
}

#[derive(Debug)]
pub(crate) enum ParseErrorSrc {
    Token,
    Eof,
}

#[derive(Debug, Error)]
#[error("failed to save constants to file: `{0}`")]
pub struct SaveError(pub(crate) io::Error);

#[derive(Debug, Error)]
pub enum FilterError {
    #[error("regex compilation failed for needle: `{input_str}`")]
    RegexCompilation { input_str: String },
}

#[derive(Debug, Error)]
pub enum MakeChangesError {
    #[error(
        "io error while {}: `{}`",
        match &.0.origin {
            ChangesSrc::FetchOp(erred_path) => format!("fetching file {}", erred_path.display()),
            ChangesSrc::SaveOp(erred_path) => format!("saving file {}", erred_path.display())
        },
        .0.inner
    )]
    IoBound(IoBoundChanges),
    #[error("failed to parse file: {0}")]
    Parse(Cow<'static, Path>),
}

#[derive(Debug)]
pub struct IoBoundChanges {
    pub(crate) origin: ChangesSrc,
    pub(crate) inner: io::Error,
}

impl IoBoundChanges {
    #[expect(
        clippy::must_use_candidate,
        reason = "It's not a bug not to use the result of this routine."
    )]
    pub fn source(&self) -> &Path {
        match &self.origin {
            ChangesSrc::FetchOp(path) | ChangesSrc::SaveOp(path) => path,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ChangesSrc {
    FetchOp(PathBuf),
    SaveOp(PathBuf),
}

#[derive(Debug, Error)]
pub(crate) enum ParseError {
    #[error("failed reading from bytes at line {line_num}: {inner}")]
    LineReading { line_num: usize, inner: io::Error },
    #[error(
        "{}",
        match src {
            ParseErrorSrc::Token => {
                format!(
                    "bad input while parsing at line {line_num}; expected token, found `{bad_seq}`"
                )
            },
            ParseErrorSrc::Eof => "unexpected eof".to_string(),
        }
    )]
    ExtraneousInput {
        src: ParseErrorSrc,
        bad_seq: Cow<'static, str>,
        line_num: usize,
    },
    #[error("expected further information in the stream, got EOF")]
    UnexpectedEof { line_num: usize },
}
