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
                        ParseErrorSrc::Source
                    },
                    line_num,
                    non_matching: input,
                },
                ParseError::UnexpectedEof { line_num } => FetchErrorRepr::ParseError {
                    source: ParseErrorSrc::Eof,
                    line_num,
                    non_matching: "".into(),
                },
            })
        })
    }

    pub fn save_to_disk(&self, path: impl AsRef<Path>) -> Result<(), SaveError> {
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
                                let attr: TokenStream = deprecate!(Self::DEPRECATION_NOTICE);

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
        reason = "The panic code path is (at the time of writing) impossible to reach."
    )]
    pub fn filter(&mut self, re: impl AsRef<str>) -> Result<Vec<&mut Const>, FilterError> {
        let Self { inner, re_cache } = self;

        let re = if let Some(re) = re_cache.get(re.as_ref()) {
            re
        } else {
            // TODO: handle regex error for compilation size, as that shouldn't be
            // immediately fatal.
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

static CONSTANT_READER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?<ident>[[:alnum:][:punct:]]+)(?<deprecated>\s\[deprecated\])?$").unwrap()
});

static SRC_READER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(?<path>"(/[[:ascii:]]*)+") (?<line>line:\d+) (?<col>col:\d+)$"#).unwrap()
});

pub(crate) fn parse_file(input: impl AsRef<[u8]>) -> Result<ConstContainer, ParseError> {
    let mut buf = String::with_capacity(input.as_ref().len());

    // NOTE: this is used for error reporting purposes, not for parsing purposes.
    let mut line_counter = 1;

    let mut parsed_items = Vec::new();

    let mut constant_reader_captures = CONSTANT_READER.capture_locations();
    let mut src_reader_captures = SRC_READER.capture_locations();

    // NOTE: parsing is performed in two steps, each corresponding to one of the two
    // lines epxected to match with the above two regexes, to fill out the
    // information required for the in-memory representation of a constant.
    //
    // Beyond regex matching, the logic is strightforward. At any point in time,
    // whenever the first line is not EOF and matches its regex, the second line
    // ought not be EOF and must match the regex. It is only if one of these two
    // conditions don't hold, that we get any form of parsing error.
    //
    // This invariant holds because for any potential constant embedded in the file,
    // there is always a two-element pair of (1) identifier and deprecation
    // information, and of (2) source file and span information. Each of these is
    // always separated by a newline byte.
    loop {
        // TODO: finish the below as it currently is not working, but should likely only
        // take into consideration a single regex, including the multiline, such that
        // the same underlying buffer is used for all capture groups, and there's no
        // need to own the results of the captures for the buffer prior to parsing the
        // second line.

        if input
            .as_ref()
            .read_line(&mut buf)
            .map_err(|inner| ParseError::LineReading {
                line_num: line_counter,
                inner,
            })?
            == 0
        {
            break;
        }

        CONSTANT_READER
            .captures_read(&mut constant_reader_captures, buf.as_bytes().trim_ascii())
            .ok_or_else(|| ParseError::ExtraneousInput {
                bad_seq: buf.clone().into(),
                expected: ConstFormatToken::Constant,
                line_num: line_counter,
            })?;

        let ident = {
            let (start, end) = constant_reader_captures
                .get(0)
                .expect("the constant identifier should always be present");

            buf.get(start..end)
        };
        let deprecated = constant_reader_captures.get(1);

        line_counter += 1;

        input
            .as_ref()
            .read_line(&mut buf)
            .map_err(|inner| ParseError::LineReading {
                line_num: line_counter,
                inner,
            })
            .and_then(|maybe_eof| {
                if maybe_eof == 0 {
                    Err(ParseError::UnexpectedEof {
                        line_num: line_counter,
                    })
                } else {
                    Ok(())
                }
            })?;

        SRC_READER.captures_read(&mut src_reader_captures, buf.as_bytes().trim_ascii());

        let path = src_reader_captures
            .get(0)
            .expect("the source path of the item should always be present");
        let line = src_reader_captures
            .get(1)
            .expect("the line information should always be present");
        let col = src_reader_captures
            .get(2)
            .expect("the columns information should always be present");

        {
            // NOTE: spans have a fallback when not running inside a proc-macro, and the
            // library doesn't ever require knowning span information beyond what is already
            // stored inside the `Const`. Also, `Span`s can't be arbitrarily built so we
            // don't store them directly in the `Const`.
            parsed_items.push(Const::from_file(
                Ident::new(ident, Span::call_site()),
                deprecated,
                PathBuf::from(path.trim_matches('"')),
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

#[inline]
pub(crate) fn build_re(re: impl AsRef<str>) -> Result<Regex, regex::Error> {
    RegexBuilder::new(re.as_ref())
        .unicode(false)
        .size_limit(const { 2_usize.pow(11) })
        .case_insensitive(true)
        .build()
}
