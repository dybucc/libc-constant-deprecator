use std::path::Path;

use syn::{
    Attribute, Block, Item, ItemConst, Stmt, Token,
    parse::{Parse, ParseStream},
};

use crate::Const;

#[derive(Debug)]
pub(crate) struct MacroParser(pub(crate) Vec<ItemConst>);

impl MacroParser {
    // NOTE: we don't take ownership of `source` because we're going to clone it
    // anyway however as many times as there are elements in the underlying
    // `ItemConst` buffer.
    pub(crate) fn into_vec(self, source: impl AsRef<Path>) -> Vec<Const> {
        let Self(buffer) = self;
        let mut out = Vec::with_capacity(buffer.len());

        for constant in buffer {
            out.push(Const::from_item(constant, source.as_ref().to_owned()));
        }

        out
    }
}

impl Parse for MacroParser {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut out = Vec::new();
        parse(&mut out, input)?;

        Ok(Self(out))
    }
}

pub(crate) fn parse(alloc: &mut Vec<ItemConst>, input: ParseStream) -> syn::Result<()> {
    macro_rules! extract_consts {
        () => {
            let Block { stmts, .. } = input.parse()?;

            for stmt in stmts {
                if let Stmt::Item(item) = stmt
                    && let Item::Const(constant) = item
                {
                    alloc.push(constant);
                }
            }
        };
    }

    input.parse::<Token![if]>()?;
    input.call(Attribute::parse_outer)?;

    extract_consts!();

    if input.peek(Token![else]) {
        input.parse::<Token![else]>()?;

        if input.peek(Token![if]) {
            parse(alloc, input)?;
        } else {
            extract_consts!();
        }
    }

    Ok(())
}
