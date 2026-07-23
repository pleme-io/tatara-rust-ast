//! Contract test: explicit `VerifierHint` produces the SAME smoke
//! body as the inference path for canonical Specs.
//!
//! This is the forcing function that prevents drift between the two
//! dispatch paths. If a new strategy is added to one path and not the
//! other, this test fails.

use tatara_rust_ast::Ident;
use tatara_rust_catalog::{CatalogEntry, CatalogSpec, MacroCatalogSpec, VerifierHint};
use tatara_rust_derive::{
    EnumFoldDeriveSpec, EnumFoldTarget, NewtypeDeriveSpec, NewtypeTarget, PerFieldDeriveSpec,
    PerFieldTarget, PerVariantDeriveSpec, ProcDeriveSpec, VariantShape,
};
use tatara_rust_verify::render_lib_rs;

/// Build a single-entry catalog with the given spec + optional hint.
fn cat(entry: CatalogEntry) -> MacroCatalogSpec {
    MacroCatalogSpec {
        title: "hint-roundtrip".into(),
        entries: vec![entry],
    }
}

/// Build a catalog entry from a spec + hint, keeping all other fields fixed.
fn entry(crate_name: &str, hint: Option<VerifierHint>, spec: CatalogSpec) -> CatalogEntry {
    CatalogEntry {
        crate_name: crate_name.into(),
        description: "Hint-roundtrip fixture.".into(),
        since: "0.1.0".into(),
        owner: "test".into(),
        verifier_hint: hint,
        spec,
    }
}

/// For each canonical (spec, hint) pair, assert: rendering with the
/// explicit hint produces the same lib.rs body as rendering with no
/// hint (inference path).
#[test]
fn explicit_hint_matches_inferred_for_canonical_specs() {
    let cases: Vec<(CatalogSpec, VerifierHint, &str)> = vec![
        (
            CatalogSpec::PerField {
                spec: PerFieldDeriveSpec {
                    trait_name: Ident::new("Getter"),
                    target: PerFieldTarget::NamedStruct,
                    trait_ref: None,
                    per_field_template:
                        "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
                    method_name_template: None,
                    impl_prelude: None,
                    skip_fields: vec![],
                    field_attribute: None,
                    field_tag: None,
                },
            },
            VerifierHint::PerFieldGetter,
            "getter",
        ),
        (
            CatalogSpec::Newtype {
                spec: NewtypeDeriveSpec {
                    trait_name: Ident::new("ImplFrom"),
                    target: NewtypeTarget::Tuple,
                    impl_template:
                        "impl ::std::convert::From<#inner_ty> for #self_name { fn from(v: #inner_ty) -> Self { Self(v) } } impl ::std::convert::From<#self_name> for #inner_ty { fn from(w: #self_name) -> Self { w.0 } }".into(),
                },
            },
            VerifierHint::NewtypeImplFrom,
            "impl-from",
        ),
        (
            CatalogSpec::EnumFold {
                spec: EnumFoldDeriveSpec {
                    trait_name: Ident::new("AllVariants"),
                    target: EnumFoldTarget::UnitVariantsOnly,
                    per_variant_fragment: "Self::#variant_name".into(),
                    fold_template: "impl #self_name { pub const ALL: &'static [Self] = &[#fold]; pub const fn all() -> &'static [Self] { Self::ALL } }".into(),
                },
            },
            VerifierHint::EnumFoldAllVariants,
            "all-variants",
        ),
        (
            CatalogSpec::PerVariant {
                spec: PerVariantDeriveSpec {
                    trait_name: Ident::new("IsVariant"),
                    variant_shape: VariantShape::Any,
                    trait_ref: None,
                    per_variant_template:
                        "pub fn #method_ident(&self) -> bool { matches!(self, #variant_shape_arm) }"
                            .into(),
                    method_name_template: Some("is_{}".into()),
                    impl_prelude: None,
                },
            },
            VerifierHint::PerVariantIsVariant,
            "is-variant",
        ),
    ];

    for (spec, hint, label) in cases {
        let inferred = render_lib_rs(&cat(entry(&format!("{label}-derive"), None, spec.clone())));
        let explicit = render_lib_rs(&cat(entry(
            &format!("{label}-derive"),
            Some(hint),
            spec,
        )));
        assert_eq!(
            inferred, explicit,
            "verifier dispatch drift for hint {hint:?} ({label}): inferred and explicit paths emit different bodies"
        );
    }
}

/// A spec marked with `VerifierHint::CompileOnly` (regardless of its
/// kind) should always produce the canonical compile-only smoke.
#[test]
fn compile_only_hint_short_circuits_inference() {
    let spec = CatalogSpec::PerField {
        spec: PerFieldDeriveSpec {
            trait_name: Ident::new("Getter"),
            target: PerFieldTarget::NamedStruct,
            trait_ref: None,
            per_field_template:
                "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
            method_name_template: None,
            impl_prelude: None,
            skip_fields: vec![],
            field_attribute: None,
            field_tag: None,
        },
    };
    let inferred = render_lib_rs(&cat(entry("foo-derive", None, spec.clone())));
    let compile_only = render_lib_rs(&cat(entry(
        "foo-derive",
        Some(VerifierHint::CompileOnly),
        spec,
    )));
    // Inference picks getter; CompileOnly hint forces compile-only. So
    // these MUST differ — that's the point of the hint.
    assert_ne!(
        inferred, compile_only,
        "CompileOnly hint should override inferred Getter strategy"
    );
    assert!(compile_only.contains("derive_compiles"));
    assert!(!compile_only.contains("getters_return_borrowed_fields"));
}
