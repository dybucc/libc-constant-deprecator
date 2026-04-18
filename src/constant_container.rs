use std::{
    collections::HashMap,
    fs,
    io::BufRead,
    iter,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use proc_macro2::{LineColumn, Span, TokenStream};
use regex::bytes::{Regex, RegexBuilder};
use syn::{Ident, Item, ItemConst, spanned::Spanned};

use crate::{
    ChangesSrc, Const, ConstFormatToken, FetchError, FetchErrorRepr, FilterError, IoBoundChanges,
    IoBoundErrorKind, MakeChangesError, ParseError, ParseErrorSrc, SaveError, deprecate,
};

#[derive(Debug)]
pub struct ConstContainer {
    pub(crate) inner: Vec<Const>,
    pub(crate) re_cache: HashMap<String, Regex>,
}

impl ConstContainer {
    pub(crate) const DEPRECATION_NOTICE: &str = "This constant, among others often used in C for \
                                                 the purposes of denoting the latest value or \
                                                 limit in a set of constants, has been \
                                                 deprecated. See #3131 for details and discussion.";

    pub(crate) fn new(constants: Vec<Const>) -> Self {
        Self {
            inner: constants,
            re_cache: HashMap::new(),
        }
    }

    pub fn fetch_from_disk(path: impl AsRef<Path>) -> Result<Self, FetchError> {
        let file = fs::read_to_string(path)
            .map_err(|inner| FetchError(FetchErrorRepr::IoBound(IoBoundErrorKind::Fs(inner))))?;

        parse_file(file).map_err(|inner| {
            FetchError(match inner {
                ParseError::LineReading { line_num, inner } => {
                    FetchErrorRepr::IoBound(IoBoundErrorKind::Parsing { inner, line_num })
                }
                ParseError::ExtraneousInput {
                    bad_seq: input,
                    expected,
                    line_num,
                } => FetchErrorRepr::ParseError {
                    source: if let ConstFormatToken::Constant = expected {
                        ParseErrorSrc::Constant
                    } else {
                        ParseErrorSrc::Path
                    },
                    line_num,
                    non_matching: input,
                },
            })
        })
    }

    pub fn save_to_disk(&self, path: impl AsRef<Path>) -> Result<(), SaveError> {
        let attr: TokenStream = deprecate!(Self::DEPRECATION_NOTICE);

        fs::write(
            path,
            self.inner
                .iter()
                .flat_map(
                    |Const {
                         ident,
                         source,
                         deprecated,
                         span: LineColumn { line, column },
                     }| {
                        ident
                            .to_string()
                            .into_bytes()
                            .into_iter()
                            .chain(iter::once(b' '))
                            .chain(if *deprecated {
                                attr.to_string().into_bytes()
                            } else {
                                Vec::new()
                            })
                            .chain(iter::once(b'\n'))
                            .chain(source.as_os_str().as_encoded_bytes().iter().copied())
                            .chain(format!(" line:{line} col:{column}\n").into_bytes())
                    },
                )
                .collect::<Vec<_>>(),
        )
        .map_err(SaveError)
    }

    #[expect(
        clippy::missing_panics_doc,
        reason = "The source of panics is (at the time of writing) trivially not impossible to \
                  panic."
    )]
    pub fn filter(&mut self, re: impl AsRef<str>) -> Result<Vec<&mut Const>, FilterError> {
        let Self { inner, re_cache } = self;

        let re = if let Some(re) = re_cache.get(re.as_ref()) {
            re
        } else {
            re_cache.insert(
                re.as_ref().to_string(),
                build_re(&re).map_err(|_| FilterError::RegexCompilation {
                    input_str: re.as_ref().to_string(),
                })?,
            );

            re_cache.get(re.as_ref()).unwrap()
        };

        Ok(inner
            .iter_mut()
            .filter(|Const { ident, .. }| re.is_match(ident.to_string().as_bytes()))
            .collect())
    }

    pub fn effect_changes(&self) -> Result<(), MakeChangesError> {
        for Const {
            ident: ref_ident,
            deprecated,
            span,
            source,
        } in &self.inner
        {
            let mut file = syn::parse_file(&fs::read_to_string(source).map_err(|inner| {
                MakeChangesError::IoBound(IoBoundChanges {
                    origin: ChangesSrc::FetchOp(source.clone()),
                    inner,
                })
            })?)
            .map_err(|_| MakeChangesError::Parse(source.clone().into()))?;

            // NOTE: the check for the span comes before the destructuring pattern because
            // otherwise borrowck complains about `item` already being exclusively borrowed
            // with the variable bindings of that pattern.
            for item in &mut file.items {
                if item.span().start() == *span
                    && let Item::Const(ItemConst { attrs, ident, .. }) = item
                    && ident == ref_ident
                    && *deprecated
                {
                    attrs.push(deprecate!(ConstContainer::DEPRECATION_NOTICE));
                }
            }

            fs::write(source, prettyplease::unparse(&file)).map_err(|inner| {
                MakeChangesError::IoBound(IoBoundChanges {
                    origin: ChangesSrc::SaveOp(source.clone()),
                    inner,
                })
            })?;
        }

        Ok(())
    }
}

static CONSTANT_READER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[[:alnum:][:punct:]]+(\s\[deprecated\])?$").unwrap());

static SRC_READER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^"(/[[:ascii:]]*)+" line:\d+ col:\d+$"#).unwrap());

pub(crate) fn parse_file(input: impl AsRef<[u8]>) -> Result<ConstContainer, ParseError> {
    let mut buf = String::with_capacity(input.as_ref().len());

    // NOTE: this is used for error reporting purposes, not for parsing purposes.
    let mut line_counter = 0;

    let mut parsed_items = Vec::new();

    while input
        .as_ref()
        .read_line(&mut buf)
        .map_err(|inner| ParseError::LineReading {
            line_num: line_counter + 1,
            inner,
        })?
        != 0
    {
        if CONSTANT_READER
            .is_match(buf.as_bytes().trim_ascii())
            .ok_or_else(|| ParseError::ExtraneousInput {
                bad_seq: buf.clone().into(),
                expected: ConstFormatToken::Constant,
                line_num: line_counter + 1,
            })
            .map(|()| true)?
            && let src_reader_check = {
                line_counter += 1;

                input
                    .as_ref()
                    .read_line(&mut buf)
                    .map_err(|inner| ParseError::LineReading {
                        line_num: line_counter + 1,
                        inner,
                    })?;

                SRC_READER
                    .is_match(buf.as_bytes().trim_ascii())
                    .ok_or_else(|| ParseError::ExtraneousInput {
                        bad_seq: buf.clone().into(),
                        expected: ConstFormatToken::Path,
                        line_num: line_counter + 1,
                    })
                    .map(|()| true)?
            }
            && src_reader_check
        {
            let Some((constant_line, source_line)) = buf.split_once('\n') else {
                panic!("if both regexes matched, there should be two lines in the buffer");
            };

            let mut constant_iter = constant_line.split_ascii_whitespace();

            let ident = constant_iter.next().expect(
                "if the `CONSTANT_READER` regex matched, there should be at least 1 element in \
                 the iterator",
            );
            let deprecated = constant_iter.next().is_some();

            let Some((path_line, col)) = source_line.rsplit_once(' ') else {
                panic!(
                    "if the `SRC_READER` regex matched, there should be at least a path, a line \
                     and a column indicators"
                );
            };
            let Some((path, line)) = path_line.rsplit_once(' ') else {
                panic!(
                    "if the `SRC_READER` regex matched, there should be at least a path, a line \
                     and a column indicators"
                );
            };

            // NOTE: spans have a fallback when not running inside a proc-macro, and the
            // library doesn't ever require knowning span information beyond what is already
            // stored inside the `Const`. Also, `Span`s can't be arbitrarily built.
            parsed_items.push(Const::from_file(
                Ident::new(ident, Span::call_site()),
                deprecated,
                PathBuf::from(path),
                LineColumn {
                    line: line.trim_start_matches("line:").parse().unwrap(),
                    column: col.trim_start_matches("col:").parse().unwrap(),
                },
            ));
        }

        line_counter += 1;

        buf.clear();
    }

    Ok(ConstContainer::new(parsed_items))
}

pub(crate) fn build_re(re: impl AsRef<str>) -> Result<Regex, regex::Error> {
    RegexBuilder::new(re.as_ref())
        .size_limit(512)
        .case_insensitive(true)
        .build()
}
