macro_rules! deprecate {
    ($msg:expr) => {{
        let msg = $msg;

        $crate::support::parse_quote! {
            #[deprecated(since = "1.0.0", note = #msg)]
        }
    }};
}

macro_rules! sealed_impl {
    (for $($t:ty),+ ;) => {
        $(
            impl $crate::support::Sealed for $t {}
        )+
    };
}
