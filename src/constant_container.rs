use std::{
  collections::HashMap,
  fs,
  io::BufRead,
  iter,
  path::{Path, PathBuf},
  sync::LazyLock,
};

use itertools::Itertools;
use proc_macro2::{Span, TokenStream};
use quote::ToTokens;
use regex::bytes::{Regex, RegexBuilder};
use syn::{
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

use crate::{
  ChangesSrc,
  Const,
  ConstFormatToken,
  FetchError,
  FetchErrorRepr,
  FilterError,
  IoBoundChanges,
  IoBoundErrorKind,
  MakeChangesError,
  ParseError,
  ParseErrorSrc,
  SaveError,
  deprecate,
};

#[derive(Debug)]
pub struct ConstContainer {
  pub(crate) inner:    Vec<Const>,
  pub(crate) re_cache: HashMap<String, Regex>,
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
    // TODO: expand macros prior to getting a string and parsing.
    let file = fs::read_to_string(path).map_err(|inner| {
      FetchError(FetchErrorRepr::IoBound(IoBoundErrorKind::Fs(inner)))
    })?;

    parse_file(file).map_err(|inner| {
      FetchError({
        match inner {
          | ParseError::LineReading { line_num, inner } =>
            FetchErrorRepr::IoBound(IoBoundErrorKind::Parsing {
              inner,
              line_num,
            }),
          | ParseError::ExtraneousInput {
            bad_seq: input,
            expected,
            line_num,
          } => FetchErrorRepr::ParseError {
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
    let attr: TokenStream = deprecate!(Self::DEPRECATION_NOTICE);

    fs::write(
      path,
      self
        .inner
        .iter()
        .flat_map(|Const { ident, source, deprecated, .. }| {
          ident
            .to_string()
            .into_bytes()
            .into_iter()
            .chain(iter::once(b' '))
            .chain(match deprecated {
              | true => attr.to_string().into_bytes().into_iter(),
              | false => Vec::new().into_iter(),
            })
            .chain(iter::once(b'\n'))
            .chain(source.as_os_str().as_encoded_bytes().iter().copied())
        })
        .collect::<Vec<_>>(),
    )
    .map_err(SaveError)
  }

  #[expect(
    clippy::missing_panics_doc,
    reason = "The source of panics is (at the time of writing) trivially not \
              impossible to panic."
  )]
  pub fn filter(
    &mut self,
    re: impl AsRef<str>,
  ) -> Result<Vec<&mut Const>, FilterError> {
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
      inner
        .iter_mut()
        .filter(|Const { ident, .. }| re.is_match(ident.to_string().as_bytes()))
        .collect(),
    )
  }

  #[expect(
    clippy::missing_panics_doc,
    reason = "The source of panics is (at the time of writing) trivially not \
              impossible to panic."
  )]
  pub fn effect_changes(&self) -> Result<(), MakeChangesError> {
    // TODO: expand macros prior to both string reading and `syn` parsing.
    self.inner.iter().try_for_each(|constant @ Const { source, .. }| {
      let file =
        syn::parse_file(&fs::read_to_string(source).map_err(|inner| {
          MakeChangesError::IoBound(IoBoundChanges {
            origin: ChangesSrc::FetchOp(source.clone()),
            inner,
          })
        })?)
        .map_err(|_| MakeChangesError::Parse(source.clone().into()))?;

      fs::write(
        source,
        (0..file.items.len())
          .fold(file, |mut file, idx| {
            (
              change_constant_in(file.items.get_mut(idx).unwrap(), constant),
              file,
            )
              .1
          })
          .into_token_stream()
          .to_string(),
      )
      .map_err(|inner| {
        MakeChangesError::IoBound(IoBoundChanges {
          origin: ChangesSrc::SaveOp(source.clone()),
          inner,
        })
      })
    })
  }
}

pub(crate) fn change_constant_in(
  item: &mut Item,
  constant @ Const { ident: ref_ident, deprecated, .. }: &Const,
) {
  match item {
    | Item::Const(ItemConst { attrs, ident, .. })
      if ident == ref_ident && *deprecated =>
      attrs.push(deprecate!(ConstContainer::DEPRECATION_NOTICE)),
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
      .for_each(|ImplItemConst { attrs, .. }| {
        attrs.push(deprecate!(ConstContainer::DEPRECATION_NOTICE));
      }),
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
      .for_each(|TraitItemConst { attrs, .. }| {
        attrs.push(deprecate!(ConstContainer::DEPRECATION_NOTICE));
      }),
    | Item::Mod(syn::ItemMod { content: Some((_, content)), .. }) =>
      for item in content.iter_mut() {
        change_constant_in(item, constant);
      },
    | _ => (),
  }
}

static CONSTANT_READER: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"^[[:alnum:][:punct:]]+(\s\[deprecated\])?$").unwrap()
});

static PATH_READER: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^(/[[:ascii:]]*)+$").unwrap());

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
    if CONSTANT_READER
      .is_match(buf.as_bytes().trim_ascii())
      .ok_or_else(|| ParseError::ExtraneousInput {
        bad_seq:  buf.clone().into(),
        expected: ConstFormatToken::Constant,
        line_num: line_counter + 1,
      })
      .map(|()| true)?
      && let (components, check) = {
        let components: Vec<String> =
          buf.split_ascii_whitespace().map_into().collect();
        buf.clear();
        line_counter += 1;
        input.as_ref().read_line(&mut buf).map_err(|inner| {
          ParseError::LineReading { line_num: line_counter + 1, inner }
        })?;

        (components, PATH_READER.is_match(buf.as_bytes().trim_ascii()))
      }
      && check
        .ok_or_else(|| ParseError::ExtraneousInput {
          bad_seq:  buf.clone().into(),
          expected: ConstFormatToken::Path,
          line_num: line_counter + 1,
        })
        .map(|()| true)?
    {
      inner.push({
        let mut out = Const::from_file(
          Ident::new(
            components.first().expect(
              "there should be at least one token if `PATH_READER` matched",
            ),
            Span::call_site(),
          ),
          PathBuf::from(buf.trim()),
        );
        out.deprecated(components.len() > 1);

        out
      });
    }
    line_counter += 1;
    buf.clear();
  }

  Ok(ConstContainer::new(inner))
}

pub(crate) fn build_re(re: impl AsRef<str>) -> Result<Regex, regex::Error> {
  RegexBuilder::new(re.as_ref()).size_limit(512).case_insensitive(true).build()
}
