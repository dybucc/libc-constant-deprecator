macro_rules! deprecate {
  ($msg:expr) => {{
    $crate::support::iter::once($msg)
      .map(|msg| {
        $crate::support::parse_quote! {
          #[deprecated(since = "1.0.0", note = #msg)]
        }
      })
      .next()
      .unwrap()
  }};
}
