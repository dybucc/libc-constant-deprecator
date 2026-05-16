use std::sync::{Arc, Mutex};

use quote::ToTokens;
use syn::{
    ItemConst,
    parse::{Parse, ParseStream},
};

use self::internal::MacroContainerRepr;

mod internal;

#[derive(Debug, Default)]
pub(crate) struct MacroContainer {
    repr: MacroContainerRepr,
}

impl MacroContainer {
    pub(super) fn get(&self, index: usize) -> Option<&Arc<Mutex<ItemConst>>> {
        let MacroContainer { repr } = self;

        repr.get(index)
    }
}

impl ToTokens for MacroContainer {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let MacroContainer { repr } = self;

        repr.to_tokens(tokens);
    }

    fn into_token_stream(self) -> proc_macro2::TokenStream
    where
        Self: Sized,
    {
        let MacroContainer { repr } = self;

        repr.into_token_stream()
    }
}

impl Parse for MacroContainer {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut out = Self::default();
        let MacroContainer { repr } = &mut out;

        repr.parse(input)?;

        Ok(out)
    }
}
