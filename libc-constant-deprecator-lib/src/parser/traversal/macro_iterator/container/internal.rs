use std::sync::{Arc, Mutex};

use quote::{ToTokens, TokenStreamExt};
use syn::{Attribute, Block, Item, ItemConst, Stmt, Token, parse::ParseStream};

#[derive(Debug, Default)]
pub(super) struct MacroContainerRepr {
    repr: Vec<Arc<Mutex<ItemConst>>>,
}

impl MacroContainerRepr {
    pub(super) fn get(&self, index: usize) -> Option<&Arc<Mutex<ItemConst>>> {
        let MacroContainerRepr { repr } = self;

        repr.get(index)
    }

    pub(super) fn parse(&mut self, input: ParseStream) -> syn::Result<()> {
        let MacroContainerRepr { repr } = self;

        input.parse::<Token![if]>()?;
        input.call(Attribute::parse_outer)?;

        extract_consts(repr, input)?;

        if input.peek(Token![else]) {
            input.parse::<Token![else]>()?;

            if input.peek(Token![if]) {
                self.parse(input)?;
            } else {
                extract_consts(repr, input)?;
            }
        }

        Ok(())
    }
}

impl ToTokens for MacroContainerRepr {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let MacroContainerRepr { repr } = self;

        tokens.append_all(repr.iter().map(|item| item.get_cloned().unwrap()));
    }

    fn into_token_stream(self) -> proc_macro2::TokenStream
    where
        Self: Sized,
    {
        let MacroContainerRepr { repr } = self;
        let mut out = proc_macro2::TokenStream::new();

        // NOTE: we trust here that the context in which this is used will always ensure
        // the constants do not have a strong count greater than 1. This holds a few
        // layers above this module because we iterate first with the types that build
        // on this type, and only then do we consume the entire iterator (which contains
        // `Self` here.)
        out.append_all(repr.into_iter().map(|ptr| {
            Arc::into_inner(ptr)
                .map(Mutex::into_inner)
                .map(Result::unwrap)
                .unwrap()
        }));

        out
    }
}

fn extract_consts(buf: &mut Vec<Arc<Mutex<ItemConst>>>, input: ParseStream) -> syn::Result<()> {
    input.parse().map(|Block { stmts, .. }| {
        stmts
            .into_iter()
            .filter_map(|stmt| {
                if let Stmt::Item(item) = stmt
                    && let Item::Const(constant) = item
                {
                    constant.into()
                } else {
                    None
                }
            })
            .map(Mutex::new)
            .map(Arc::new)
            .for_each(|constant| buf.push(constant));
    })
}
