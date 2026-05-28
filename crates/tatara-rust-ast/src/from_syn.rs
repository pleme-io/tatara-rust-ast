//! `FromSyn` impls for the typed AST nodes.
//!
//! Lives in the same crate as the traits + types to satisfy the orphan
//! rule. tatara-rust-parse holds the round-trip helpers and tests.

use crate::{AstError, FromSyn, Generics, Ident, Impl, RefKind, TypeRef};

impl FromSyn<syn::Ident> for Ident {
    fn from_syn(t: &syn::Ident) -> Result<Self, AstError> {
        Ok(Self(t.to_string()))
    }
}

impl FromSyn<syn::Type> for TypeRef {
    fn from_syn(t: &syn::Type) -> Result<Self, AstError> {
        match t {
            syn::Type::Path(tp) => {
                let last = tp
                    .path
                    .segments
                    .last()
                    .ok_or_else(|| AstError::SynParse("empty type path".into()))?;
                let ident = Ident::from_syn(&last.ident)?;
                let generics = match &last.arguments {
                    syn::PathArguments::None => vec![],
                    syn::PathArguments::AngleBracketed(ab) => ab
                        .args
                        .iter()
                        .filter_map(|a| match a {
                            syn::GenericArgument::Type(ty) => TypeRef::from_syn(ty).ok(),
                            _ => None,
                        })
                        .collect(),
                    syn::PathArguments::Parenthesized(_) => vec![],
                };
                Ok(Self {
                    ident,
                    generics,
                    reference: None,
                })
            }
            syn::Type::Reference(r) => {
                let inner = TypeRef::from_syn(&r.elem)?;
                let lifetime = r.lifetime.as_ref().map(|lt| lt.ident.to_string());
                let kind = if r.mutability.is_some() {
                    Some(RefKind::Mut { lifetime })
                } else {
                    Some(RefKind::Shared { lifetime })
                };
                Ok(Self {
                    reference: kind,
                    ..inner
                })
            }
            _ => Err(AstError::SynParse(
                "unsupported syn::Type variant for round-trip".into(),
            )),
        }
    }
}

impl FromSyn<syn::ItemImpl> for Impl {
    fn from_syn(t: &syn::ItemImpl) -> Result<Self, AstError> {
        let self_type = TypeRef::from_syn(&t.self_ty)?;
        let trait_ref = t
            .trait_
            .as_ref()
            .map(|(_, p, _)| {
                let last = p
                    .segments
                    .last()
                    .ok_or_else(|| AstError::SynParse("empty trait path".into()))?;
                Ok::<_, AstError>(TypeRef {
                    ident: Ident::from_syn(&last.ident)?,
                    generics: vec![],
                    reference: None,
                })
            })
            .transpose()?;
        // Fn round-trip is out of scope for this v0; round-trip tests
        // assert on the impl shell (trait + target) only.
        Ok(Self {
            generics: Generics::default(),
            trait_ref,
            self_type,
            items: vec![],
        })
    }
}
