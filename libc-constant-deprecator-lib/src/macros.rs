macro_rules! deprecate {
    ($msg:expr) => {{
        let msg = $msg;

        $crate::support::parse_quote! {
            #[deprecated(since = "1.0.0", note = #msg)]
        }
    }};
}

macro_rules! send_sync_impl {
    (for $t:ty; $($($(#[$doc:meta])+)? $it:ident)+) => {
        $($($(#[$doc])+)? unsafe impl $it for $t {})+
    };
}
