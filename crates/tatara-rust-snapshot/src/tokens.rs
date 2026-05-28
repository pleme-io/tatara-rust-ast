//! Token-stable assertion primitives.
//!
//! The pleme-io substrate emits Rust source via `quote!{}` +
//! `syn::parse2` + `prettyplease::unparse`. Prettyplease's whitespace
//! choices (`#field_name` vs `# field_name`, `&self` vs `& self`,
//! line breaks, trailing commas) are formatting normalizations — the
//! TOKENS are stable across them.
//!
//! Tests that assert on raw strings (`s.contains("pub fn #field_name")`)
//! end up brittle to formatter version bumps and prettyplease updates.
//! The canonical fix: parse both sides as `TokenStream`, render via
//! `to_string()` (which uses quote's canonical spacing), then compare.
//!
//! That's what these primitives do.

use proc_macro2::TokenStream;

/// Reduce a Rust source string to its canonical token-stream
/// rendering. Calls `parse::<TokenStream>()` then `to_string()`.
/// Whitespace, formatter spacing, comment positions all collapse;
/// what survives is the sequence of TokenTree variants rendered with
/// `quote`'s canonical spacing rules.
///
/// **Panics** if the source doesn't parse as a TokenStream — that's
/// usually a sign that the input is malformed, not that this helper
/// is wrong.
#[must_use]
pub fn canonical_tokens(src: &str) -> String {
    let ts: TokenStream = src
        .parse()
        .unwrap_or_else(|e| panic!("not parseable as TokenStream: {e}\nsource:\n{src}"));
    ts.to_string()
}

/// Assert that `actual` (a Rust source string) contains every token
/// of `expected` (a Rust source fragment) as a contiguous substring
/// after both are reduced to canonical token form.
///
/// Catches genuine missing tokens; ignores prettyplease spacing,
/// comments, line breaks.
#[macro_export]
macro_rules! assert_tokens_contain {
    ($actual:expr, $expected:expr $(,)?) => {{
        let actual_canon = $crate::tokens::canonical_tokens($actual);
        let expected_canon = $crate::tokens::canonical_tokens($expected);
        assert!(
            actual_canon.contains(&expected_canon),
            "assert_tokens_contain! failed\n\
             expected fragment (canonical):\n  {expected_canon}\n\
             not found in actual (canonical, head):\n  {actual_head}\n",
            expected_canon = expected_canon,
            actual_head = &actual_canon.chars().take(400).collect::<String>(),
        );
    }};
}

/// Assert that two Rust source strings are token-equivalent after
/// canonicalization. Full token-sequence equality, modulo whitespace.
#[macro_export]
macro_rules! assert_tokens_eq {
    ($actual:expr, $expected:expr $(,)?) => {{
        let actual_canon = $crate::tokens::canonical_tokens($actual);
        let expected_canon = $crate::tokens::canonical_tokens($expected);
        assert_eq!(
            actual_canon, expected_canon,
            "assert_tokens_eq! failed\n\
             actual (canonical):\n  {actual_canon}\n\
             expected (canonical):\n  {expected_canon}\n",
        );
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_collapses_whitespace() {
        let a = canonical_tokens("pub fn foo(  x  :  i32 ) {  x  +  1  }");
        let b = canonical_tokens("pub fn foo(x:i32){x+1}");
        assert_eq!(a, b);
    }

    #[test]
    fn canonical_handles_quote_interpolation_syntax() {
        // `#field_name` and `# field_name` tokenize identically.
        let a = canonical_tokens("quote! { pub fn #field_name(&self) {} }");
        let b = canonical_tokens("quote! { pub fn # field_name(& self) {} }");
        assert_eq!(a, b);
    }

    #[test]
    fn contains_matches_modulo_spacing() {
        let actual = "pub fn foo(& self) -> & i32 { & self . x }";
        assert_tokens_contain!(actual, "pub fn foo(&self) -> &i32");
        assert_tokens_contain!(actual, "&self.x");
    }

    #[test]
    fn eq_treats_token_equivalents_as_equal() {
        assert_tokens_eq!(
            "impl Foo { pub fn bar(&self) {} }",
            "impl Foo {\n    pub fn bar(&self) {}\n}",
        );
    }

    #[test]
    #[should_panic(expected = "expected fragment (canonical)")]
    fn contains_panics_on_missing() {
        assert_tokens_contain!("pub fn foo() {}", "pub fn bar() {}");
    }

    #[test]
    #[should_panic(expected = "assert_tokens_eq! failed")]
    fn eq_panics_on_mismatch() {
        assert_tokens_eq!("fn a() {}", "fn b() {}");
    }
}
