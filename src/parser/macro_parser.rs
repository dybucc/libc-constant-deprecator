use std::path::Path;

use syn::{
    Attribute, Block, Item, ItemConst, Stmt, Token,
    parse::{Parse, ParseStream},
};

use crate::Const;

#[derive(Debug)]
pub(crate) struct MacroParser(pub(crate) Vec<ItemConst>);

impl MacroParser {
    pub(crate) fn into_vec(self, source: impl AsRef<Path>) -> Vec<Const> {
        let mut out = Vec::with_capacity(self.0.len());

        for constant in self.0 {
            out.push(Const::from_item(constant, source.as_ref().to_owned()));
        }

        out
    }
}

impl Parse for MacroParser {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        input.parse::<Token![if]>()?;
        input.call(Attribute::parse_outer)?;

        let Block { stmts, .. } = input.parse()?;
        let mut out = Vec::new();

        for stmt in stmts {
            if let Stmt::Item(item) = stmt
                && let Item::Const(constant) = item
            {
                out.push(constant);
            }
        }

        if input.peek(Token![else]) {
            input.parse::<Token![else]>()?;

            // TODO: this may be wrong.
            Self::parse(input)?;
        }

        Ok(Self(out))
    }
}
