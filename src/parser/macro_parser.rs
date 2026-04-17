use syn::{
    Token,
    parse::{Parse, ParseStream},
};

use crate::Const;

#[derive(Debug)]
pub(crate) struct MacroParser(pub(crate) Vec<Const>);

impl MacroParser {
    pub(crate) fn into_vec(self) -> Vec<Const> {
        self.0
    }
}

impl Parse for MacroParser {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        if input.peek(Token![if]) {
            input.parse::<Token![if]>()?;

            // TODO: the next token ought be an outer attribute, after which
            // there should be a block with regular module-level item
            // declarations. Outside the block, there should be an `else` token,
            // (possibly, but not surely) an `if` token and the same block.
        }

        todo!()
    }
}
