use syn::parse::{Parse, ParseStream};

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
        todo!()
    }
}
