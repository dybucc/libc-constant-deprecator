macro_rules! deprecate {
    ($msg:expr) => {{
        let msg = $msg;

        $crate::support::parse_quote! {
            #[deprecated(since = "1.0.0", note = #msg)]
        }
    }};
}
