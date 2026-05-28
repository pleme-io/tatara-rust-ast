//! `tatara-rust-parse` — round-trip helpers.
//!
//! The `FromSyn` impls themselves live in `tatara-rust-ast` to satisfy
//! the orphan rule. This crate's job is the operator-facing helpers
//! that drive "render → parse → identity" property tests.

use proc_macro2::TokenStream;
use syn::parse2;
use tatara_rust_ast::{AstError, FromSyn, Impl, ToRustTokens};

/// Render an `Impl` to tokens, parse back via syn, return the typed Impl.
/// Useful for property tests that assert round-trip identity.
pub fn roundtrip_impl(i: &Impl) -> Result<Impl, AstError> {
    let ts: TokenStream = i.to_rust_tokens();
    let parsed: syn::ItemImpl =
        parse2(ts).map_err(|e| AstError::SynParse(e.to_string()))?;
    Impl::from_syn(&parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_ast::{Generics, Ident, TypeRef};

    #[test]
    fn ident_roundtrips_via_syn() {
        let id = Ident::new("Foo");
        let syn_id = syn::Ident::new(&id.0, proc_macro2::Span::call_site());
        let back = Ident::from_syn(&syn_id).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn impl_shell_roundtrips() {
        let i = Impl {
            generics: Generics::default(),
            trait_ref: Some(TypeRef::simple("Greet")),
            self_type: TypeRef::simple("Foo"),
            items: vec![],
        };
        let back = roundtrip_impl(&i).unwrap();
        assert_eq!(i.trait_ref, back.trait_ref);
        assert_eq!(i.self_type.ident, back.self_type.ident);
    }
}
