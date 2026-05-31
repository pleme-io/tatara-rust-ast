//! Contract tests for the `SpecKind` trait dispatch.
//!
//! Forcing functions:
//! 1. Every `CatalogSpec` variant must yield a `&dyn SpecKind` whose
//!    `kind_label()` matches what the catalog JSON `"kind"` tag would
//!    emit (kebab-case of the variant name).
//! 2. Every kind_label must be globally unique (one variant ↔ one label).
//! 3. The variant count this test asserts must equal the actual
//!    `CatalogSpec` variant count — drift detector for new kinds.

use std::collections::BTreeSet;
use tatara_rust_ast::Ident;
use tatara_rust_catalog::{CatalogSpec, SpecKind};
use tatara_rust_composite::CompositeDeriveSpec;
use tatara_rust_derive::{
    EnumFoldDeriveSpec, EnumFoldTarget, KindRoundTripSpec, NewtypeDeriveSpec, NewtypeTarget,
    PerFieldDeriveSpec, PerFieldTarget, PerVariantDeriveSpec, ProcDeriveSpec, VariantShape,
    VerificationMatrixSpec,
};
use tatara_rust_macro_rules::{MacroArm, MacroRulesSpec};
use tatara_rust_proc_attr::{AttrTransform, ProcAttrSpec};
use tatara_rust_proc_fn::{FnTransform, ProcFnSpec};

/// Every variant `CatalogSpec::X { spec: T }` paired with the
/// `kind_label()` `T`'s `SpecKind` impl returns.
fn every_variant() -> Vec<(CatalogSpec, &'static str)> {
    vec![
        (
            CatalogSpec::Derive {
                spec: ProcDeriveSpec::new("KindMarker", vec![]),
            },
            "derive",
        ),
        (
            CatalogSpec::PerField {
                spec: PerFieldDeriveSpec {
                    trait_name: Ident::new("KindGetter"),
                    target: PerFieldTarget::NamedStruct,
                    trait_ref: None,
                    per_field_template:
                        "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
                    method_name_template: None,
                    impl_prelude: None,
                    skip_fields: vec![],
                    field_attribute: None,
                },
            },
            "per-field",
        ),
        (
            CatalogSpec::PerVariant {
                spec: PerVariantDeriveSpec {
                    trait_name: Ident::new("KindIs"),
                    variant_shape: VariantShape::Any,
                    trait_ref: None,
                    per_variant_template:
                        "pub fn #method_ident(&self) -> bool { matches!(self, #variant_shape_arm) }"
                            .into(),
                    method_name_template: Some("is_{}".into()),
                    impl_prelude: None,
                },
            },
            "per-variant",
        ),
        (
            CatalogSpec::Newtype {
                spec: NewtypeDeriveSpec {
                    trait_name: Ident::new("KindNewtype"),
                    target: NewtypeTarget::Tuple,
                    impl_template:
                        "impl #self_name { pub fn n(&self) -> &#inner_ty { &self.0 } }".into(),
                },
            },
            "newtype",
        ),
        (
            CatalogSpec::EnumFold {
                spec: EnumFoldDeriveSpec {
                    trait_name: Ident::new("KindFold"),
                    target: EnumFoldTarget::AnyVariants,
                    per_variant_fragment: "#variant_str".into(),
                    fold_template:
                        "impl #self_name { pub const NS: &'static [&'static str] = &[#fold]; }"
                            .into(),
                },
            },
            "enum-fold",
        ),
        (
            CatalogSpec::ProcAttr {
                spec: ProcAttrSpec {
                    macro_name: Ident::new("kind_attr"),
                    transform: AttrTransform::PrependPrelude {
                        prelude_tokens: "// kind\n".into(),
                    },
                },
            },
            "proc-attr",
        ),
        (
            CatalogSpec::ProcFn {
                spec: ProcFnSpec {
                    macro_name: Ident::new("kind_fn"),
                    transform: FnTransform::PrependPrelude {
                        prelude_tokens: "// kind\n".into(),
                    },
                },
            },
            "proc-fn",
        ),
        (
            CatalogSpec::MacroRules {
                spec: MacroRulesSpec {
                    macro_name: Ident::new("kind_rules"),
                    arms: vec![MacroArm {
                        matcher: "()".into(),
                        transcriber: "{ () }".into(),
                    }],
                },
            },
            "macro-rules",
        ),
        (
            CatalogSpec::Composite {
                spec: CompositeDeriveSpec {
                    bundle_name: Ident::new("KindComposite"),
                    members: vec![tatara_rust_composite::CompositeMember::Simple(
                        ProcDeriveSpec::new("KindCompositeInner", vec![]),
                    )],
                },
            },
            "composite",
        ),
        (
            CatalogSpec::KindRoundTrip {
                spec: KindRoundTripSpec::kind_byte("KindRoundTripSample"),
            },
            "kind-round-trip",
        ),
        (
            CatalogSpec::VerificationMatrix {
                spec: VerificationMatrixSpec::canonical(),
            },
            "verification-matrix",
        ),
    ]
}

#[test]
fn every_variant_has_a_spec_kind_impl() {
    // Smoke: calling as_spec_kind() must not panic for any variant.
    for (cat_spec, _) in every_variant() {
        let _sk: &dyn SpecKind = cat_spec.as_spec_kind();
        // Trait dispatch works — kind_label() must succeed.
        let _label = cat_spec.kind_label();
    }
}

#[test]
fn kind_label_matches_inner_spec_kind_impl() {
    for (cat_spec, expected_label) in every_variant() {
        assert_eq!(
            cat_spec.kind_label(),
            expected_label,
            "CatalogSpec dispatch returned wrong label: got {} expected {}",
            cat_spec.kind_label(),
            expected_label
        );
        // Confirm the dispatched trait method returns the same label.
        let sk = cat_spec.as_spec_kind();
        assert_eq!(sk.kind_label(), expected_label);
    }
}

#[test]
fn kind_labels_are_globally_unique() {
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    for (cat_spec, _) in every_variant() {
        let label = cat_spec.kind_label();
        assert!(
            seen.insert(label),
            "duplicate kind_label `{label}` across CatalogSpec variants"
        );
    }
}

#[test]
fn variant_count_matches_expected_total() {
    // Drift detector — every new CatalogSpec variant must add a row
    // to `every_variant()`. Otherwise the matrix + this contract test
    // both fail (and that's the point).
    let count = every_variant().len();
    assert_eq!(
        count, 11,
        "every_variant() must cover every CatalogSpec variant; got {count}, expected 11"
    );
}

#[test]
fn trait_name_for_verifier_returns_some_for_every_variant() {
    for (cat_spec, _) in every_variant() {
        assert!(
            cat_spec.trait_name_for_verifier().is_some(),
            "kind {} returned None for trait_name_for_verifier",
            cat_spec.kind_label()
        );
    }
}
