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
    ChangesSrc, Const, FetchError, FetchErrorRepr, FilterError, IoBoundChanges, IoBoundErrorKind,
    MakeChangesError, ParseError, ParseErrorSrc, SaveError, deprecate,
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
                    src: source,
                    bad_seq: input,
                    line_num,
                } => FetchErrorRepr::ParseError {
                    source,
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

static MATCHER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^([[:alnum:][:punct:]]+)( \[deprecated\])?\n("(/[[:ascii:]]*)+") (line:\d+) (col:\d+)$"#,
    )
    .unwrap()
});

pub(crate) fn parse_file(input: impl AsRef<[u8]>) -> Result<ConstContainer, ParseError> {
    let mut buf = String::new();
    let mut line_counter = 1;

    // NOTE: we reuse this buffer of regex captures across all matched constants.
    let mut captures = MATCHER.capture_locations();

    let mut out = Vec::new();

    loop {
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

        line_counter += 1;

        input
            .as_ref()
            .read_line(&mut buf)
            .map_err(|inner| ParseError::LineReading {
                line_num: line_counter,
                inner,
            })
            .and_then(|res| {
                if res == 0 {
                    Err(ParseError::UnexpectedEof {
                        line_num: line_counter,
                    })
                } else {
                    Ok(())
                }
            })?;

        MATCHER
            .captures_read(&mut captures, buf.as_bytes())
            .ok_or_else(|| ParseError::ExtraneousInput {
                src: ParseErrorSrc::Token,
                bad_seq: buf.clone().into(),
                line_num: line_counter,
            })?;

        macro_rules! extract {
            ($idx:expr, $msg:expr) => {{
                let (start, end) = captures.get($idx).expect($msg);

                buf.get(start..end).unwrap()
            }};
        }

        // NOTE: the below indices into the regex capture group are 1-indexed because
        // index 0 holds the complete match.

        // NOTE: these are the contents of the first line for a single constant.
        let ident = extract!(1, "the identifier of a constant should always be present");
        let deprecated = captures.get(2).is_some();

        // NOTE: and these are the contents of the second line for a single constant.
        let source = extract!(
            3,
            "the source file path of a constant should always be present"
        );
        let line = extract!(
            4,
            "the line information of a constant should always be present"
        );
        let column = extract!(
            5,
            "the column information of a constant should always be present"
        );

        macro_rules! extract_num {
            ($str:expr, $spec:expr) => {{
                $str.trim_start_matches($spec)
                    .parse()
                    .map_err(|_| ParseError::ExtraneousInput {
                        src: ParseErrorSrc::Token,
                        bad_seq: buf.clone().into(),
                        line_num: line_counter,
                    })?
            }};
        }

        out.push(Const::from_file(
            Ident::new(ident, Span::call_site()),
            deprecated,
            PathBuf::from(source.trim_matches('"')),
            LineColumn {
                line: extract_num!(line, "line:"),
                column: extract_num!(column, "col:"),
            },
        ));

        line_counter += 1;
        buf.clear();
    }

    Ok(ConstContainer::new(out))
}

#[inline]
pub(crate) fn build_re(re: impl AsRef<str>) -> Result<Regex, regex::Error> {
    RegexBuilder::new(re.as_ref())
        .unicode(false)
        .size_limit(const { 2_usize.pow(11) })
        .case_insensitive(true)
        .build()
}
