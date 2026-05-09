use std::path::PathBuf;

use syn::{
    Attribute, Block, Item, ItemConst, Stmt, Token,
    parse::{Parse, ParseStream},
};

use crate::Const;

#[derive(Debug)]
pub(crate) struct MacroParser(Vec<ItemConst>);

impl MacroParser {
    pub(crate) fn into_iter(self, source: PathBuf) -> impl Iterator<Item = Const> {
        let Self(buffer) = self;

        buffer
            .into_iter()
            .map(move |constant| Const::from_item(constant, source.clone()))
    }
}

impl Parse for MacroParser {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut buf = Vec::new();
        parse(&mut buf, input)?;

        Ok(Self(buf))
    }
}

fn parse(buf: &mut Vec<ItemConst>, input: ParseStream) -> syn::Result<()> {
    // NOTE: if at some point we find that a `cfg_if` macro invocation contains
    // another `cfg_if` macro invocation nested within it, this should be fairly
    // simple to fix. Currently, we assume that upon entering the macro invocation,
    // no further expansions will happen, and thus only items (possibly constants)
    // are to be found.
    macro_rules! extract_consts {
        () => {{
            let Block { stmts, .. } = input.parse()?;

            stmts
                .into_iter()
                .filter_map(|stmt| {
                    if let Stmt::Item(Item::Const(constant)) = stmt {
                        constant.into()
                    } else {
                        None
                    }
                })
                .for_each(|constant| buf.push(constant));
        }};
    }

    input.parse::<Token![if]>()?;
    input.call(Attribute::parse_outer)?;

    extract_consts!();

    if input.peek(Token![else]) {
        input.parse::<Token![else]>()?;

        if input.peek(Token![if]) {
            parse(buf, input)?;
        } else {
            extract_consts!();
        }
    }

    Ok(())
}
