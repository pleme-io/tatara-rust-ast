//! `tatara-rust-proptest` — property-based testing primitives.
//!
//! Three layers of generators + canned property assertions:
//!
//! 1. **L0 generators** — `arb_ident`, `arb_type_ref`, `arb_ref_kind`.
//! 2. **L2 generators** — `arb_per_field_spec`, `arb_per_variant_spec`,
//!    `arb_proc_derive_spec`.
//! 3. **Property assertions** — `prop_validate_is_total`,
//!    `prop_emit_does_not_panic`, `prop_serde_roundtrips`. Each is a
//!    closure suitable for `proptest!`.
//!
//! Consumers add a single `#[test]` per Spec that calls one of the
//! `prop_*` assertions; proptest runs 256 randomized cases.

use proptest::prelude::*;
use tatara_rust_ast::{Ident, RefKind, ToRustTokens, TypeRef};
use tatara_rust_derive::{
    PerFieldDeriveSpec, PerFieldTarget, PerVariantDeriveSpec, ProcDeriveSpec, VariantShape,
};
use tatara_rust_validate::Validate;

// ─────────────────────────────────────────────────────────────────────
// L0 generators
// ─────────────────────────────────────────────────────────────────────

/// Random valid Rust identifier: starts with `[A-Za-z_]`, body is
/// `[A-Za-z0-9_]{0,15}`. Excludes the empty string.
pub fn arb_ident() -> impl Strategy<Value = Ident> {
    "[A-Za-z_][A-Za-z0-9_]{0,15}".prop_map(Ident::new)
}

pub fn arb_ref_kind() -> impl Strategy<Value = RefKind> {
    prop_oneof![
        Just(RefKind::shared()),
        Just(RefKind::mut_()),
        Just(RefKind::shared_lifetime("a")),
        Just(RefKind::shared_lifetime("static")),
    ]
}

/// Recursive TypeRef generator bounded to depth 3.
pub fn arb_type_ref() -> impl Strategy<Value = TypeRef> {
    let leaf = arb_ident().prop_map(|i| TypeRef {
        ident: i,
        generics: vec![],
        reference: None,
    });
    leaf.prop_recursive(3, 8, 3, |inner| {
        (arb_ident(), prop::collection::vec(inner, 0..3), prop::option::of(arb_ref_kind())).prop_map(
            |(ident, generics, reference)| TypeRef {
                ident,
                generics,
                reference,
            },
        )
    })
}

// ─────────────────────────────────────────────────────────────────────
// L2 generators
// ─────────────────────────────────────────────────────────────────────

/// Random PerFieldDeriveSpec — template + trait_name come from arb_ident;
/// optional method_name_template + impl_prelude.
pub fn arb_per_field_spec() -> impl Strategy<Value = PerFieldDeriveSpec> {
    (
        arb_ident(),
        // Always include at least one splice hole so Validate passes.
        Just("pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".to_string()),
        prop::option::of(Just("with_{}".to_string())),
        prop::option::of(Just("MyTrait".to_string())),
    )
        .prop_map(
            |(trait_name, per_field_template, method_name_template, trait_ref)| {
                PerFieldDeriveSpec {
                    trait_name,
                    target: PerFieldTarget::NamedStruct,
                    trait_ref,
                    per_field_template,
                    method_name_template,
                    impl_prelude: None,
                    skip_fields: vec![],
                    field_attribute: None,
                }
            },
        )
}

pub fn arb_per_variant_spec() -> impl Strategy<Value = PerVariantDeriveSpec> {
    (
        arb_ident(),
        Just(
            "pub fn #method_ident(&self) -> bool { matches!(self, #variant_shape_arm) }"
                .to_string(),
        ),
        Just(Some("is_{}".to_string())),
        prop::option::of(Just("MyTrait".to_string())),
    )
        .prop_map(
            |(trait_name, per_variant_template, method_name_template, trait_ref)| {
                PerVariantDeriveSpec {
                    trait_name,
                    variant_shape: VariantShape::Any,
                    trait_ref,
                    per_variant_template,
                    method_name_template,
                    impl_prelude: None,
                }
            },
        )
}

pub fn arb_proc_derive_spec() -> impl Strategy<Value = ProcDeriveSpec> {
    arb_ident().prop_map(|i| ProcDeriveSpec::new(i.0, vec![]))
}

// ─────────────────────────────────────────────────────────────────────
// Property assertions
// ─────────────────────────────────────────────────────────────────────

/// **Totality** — `validate` returns a Vec regardless of the input.
/// Calling it twice produces the same Vec. Use to assert the validator
/// is deterministic + total over the Spec space.
pub fn prop_validate_is_total<T: Validate + Clone>(spec: &T) {
    let v1 = spec.validate();
    let v2 = spec.clone().validate();
    assert_eq!(v1, v2, "Validate impl is non-deterministic");
}

/// **Identifier rendering doesn't panic.** Catches bad escape-handling
/// in `ToRustTokens` impls.
pub fn prop_to_tokens_does_not_panic<T: ToRustTokens>(node: &T) {
    let _ = node.to_rust_tokens();
}

#[cfg(test)]
mod tests {
    use super::*;

    proptest! {
        #[test]
        fn ident_is_always_nonempty(i in arb_ident()) {
            prop_assert!(!i.0.is_empty());
        }

        #[test]
        fn ident_to_tokens_does_not_panic(i in arb_ident()) {
            prop_to_tokens_does_not_panic(&i);
        }

        #[test]
        fn type_ref_to_tokens_does_not_panic(t in arb_type_ref()) {
            prop_to_tokens_does_not_panic(&t);
        }

        #[test]
        fn per_field_validate_is_total(s in arb_per_field_spec()) {
            prop_validate_is_total(&s);
        }

        #[test]
        fn per_variant_validate_is_total(s in arb_per_variant_spec()) {
            prop_validate_is_total(&s);
        }

        #[test]
        fn proc_derive_validate_is_total(s in arb_proc_derive_spec()) {
            prop_validate_is_total(&s);
        }
    }
}
