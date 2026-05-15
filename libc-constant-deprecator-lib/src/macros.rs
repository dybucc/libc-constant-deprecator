macro_rules! deprecate {
    ($msg:expr) => {{
        let msg = $msg;

        $crate::support::parse_quote! {
            #[deprecated(since = "1.0.0", note = #msg)]
        }
    }};
}

macro_rules! borrowed {
    ($elem:expr) => {{ $crate::support::BorrowedElement::new($elem.into()) }};
    () => {{ $crate::support::BorrowedElement::new(Option::default()) }};
}
