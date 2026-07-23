//! Worked-example macro Specs — the canonical real-world primitives.
//!
//! Each function returns a typed Spec value that compiles to a publishable
//! proc-macro crate. Combined, they prove `tatara-rust-ast` produces useful,
//! reusable derive macros from typed data alone — no per-derive bespoke
//! proc-macro authoring.
//!
//! End-to-end coverage in `tatara-rust-test/tests/per_field_end_to_end.rs`.

use tatara_rust_ast::Ident;
use tatara_rust_derive::{
    AggregateSpec, FieldTag, PerFieldDeriveSpec, PerFieldTarget, TagSpec, VerificationMatrixSpec,
};

/// `pleme-verification-matrix` — the farm's first test-generation
/// primitive. Emits the dependency-free `verification_matrix!` +
/// `matrix_covers_all!` declarative-macro pair (CLOSED-LOOP
/// MASS-SYNTHESIS rule 1). Exercised end-to-end in
/// `tests/verification_matrix_e2e.rs`.
#[must_use]
pub fn verification_matrix_spec() -> VerificationMatrixSpec {
    VerificationMatrixSpec::canonical()
}

/// `#[derive(GetterAll)]` — `pub fn <field>(&self) -> &<Type>` for every named field.
/// Inherent impl. Method name = field name.
#[must_use]
pub fn getter_all_spec() -> PerFieldDeriveSpec {
    PerFieldDeriveSpec {
        trait_name: Ident::new("GetterAll"),
        target: PerFieldTarget::NamedStruct,
        trait_ref: None,
        per_field_template:
            "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
        method_name_template: None,
        impl_prelude: None,
        skip_fields: vec![],
        field_attribute: None,
        field_tag: None,
    }
}

/// `#[derive(WithBuilder)]` — `pub fn with_<field>(mut self, v: <Type>) -> Self`
/// for every named field. Inherent fluent builder.
#[must_use]
pub fn with_builder_spec() -> PerFieldDeriveSpec {
    PerFieldDeriveSpec {
        trait_name: Ident::new("WithBuilder"),
        target: PerFieldTarget::NamedStruct,
        trait_ref: None,
        per_field_template: concat!(
            "pub fn #method_ident(mut self, v: #field_ty) -> Self ",
            "{ self.#field_name = v; self }"
        )
        .into(),
        method_name_template: Some("with_{}".into()),
        impl_prelude: None,
        skip_fields: vec![],
        field_attribute: None,
        field_tag: None,
    }
}

/// `#[derive(SetterAll)]` — `pub fn set_<field>(&mut self, v: <Type>)`
/// for every named field. Inherent setters.
#[must_use]
pub fn setter_all_spec() -> PerFieldDeriveSpec {
    PerFieldDeriveSpec {
        trait_name: Ident::new("SetterAll"),
        target: PerFieldTarget::NamedStruct,
        trait_ref: None,
        per_field_template:
            "pub fn #method_ident(&mut self, v: #field_ty) { self.#field_name = v; }".into(),
        method_name_template: Some("set_{}".into()),
        impl_prelude: None,
        skip_fields: vec![],
        field_attribute: None,
        field_tag: None,
    }
}

/// `#[derive(HotSwap)]` — the `field_tag` (exhaustive multi-tag
/// classification) worked example. Every named field must carry either
/// `#[hot_swap]` or `#[restart_required(reason = "...")]`, or the
/// consumer's compile fails. Self-contained (no external runtime crate,
/// matching this file's other worked examples) — emits, per field, an
/// inherent `pub fn <field>_hot_swap_class() -> (bool, Option<&'static str>)`
/// (is-hot-swappable, restart reason), via [`PerFieldDeriveSpec::method_name_template`]
/// (already wired to apply uniformly across `field_tag` mode's tags too,
/// same as the non-`field_tag` derives below). This is the SAME
/// mechanism `theory/CALHA.md`'s real `pleme-hotswap`/`pleme-hotswap-derive`
/// crates target — proven here against a generic, dependency-free shape
/// first (per Care #1: ground the mechanism before wiring the specific
/// consumer). See `tatara-rust-test/tests/field_tag_end_to_end.rs` for the
/// real compiled-and-invoked proof, including the exhaustiveness failure.
///
/// **Load-bearing correction (found by that e2e test, not by inspection):**
/// an earlier draft of this spec used bare tuple-EXPRESSION per-field
/// templates (`"(stringify!(#field_name), true, None)"`). The generated
/// derive always wraps per-field output in `impl #self_name { #(#per_field)* }`
/// — a list of associated ITEMS, not expressions — so that draft failed
/// real `cargo build` with "non-item in item list". Every per-tag
/// template MUST emit a valid item (a function, const, or type alias),
/// exactly like this file's other worked examples already do.
#[must_use]
pub fn hot_swap_spec() -> PerFieldDeriveSpec {
    PerFieldDeriveSpec {
        trait_name: Ident::new("HotSwap"),
        target: PerFieldTarget::NamedStruct,
        trait_ref: None,
        per_field_template: String::new(), // unused -- field_tag mode
        method_name_template: Some("{}_hot_swap_class".into()),
        impl_prelude: None,
        skip_fields: vec![],
        field_attribute: None,
        field_tag: Some(TagSpec {
            exhaustive: true,
            aggregate: None,
            tags: vec![
                FieldTag {
                    name: "hot_swap".into(),
                    required_args: vec![],
                    per_field_template:
                        "pub fn #method_ident() -> (bool, Option<&'static str>) { (true, None) }".into(),
                    aggregate_const_entry: None,
                    aggregate_stmt: None,
                },
                FieldTag {
                    name: "restart_required".into(),
                    required_args: vec!["reason".into()],
                    per_field_template: concat!(
                        "pub fn #method_ident() -> (bool, Option<&'static str>) ",
                        "{ (false, Some(#reason)) }"
                    )
                    .into(),
                    aggregate_const_entry: None,
                    aggregate_stmt: None,
                },
            ],
        }),
    }
}

/// `#[derive(HotSwapClassifier)]` — `field_tag`'s AGGREGATE shape (see
/// [`AggregateSpec`]), matching the REAL target trait
/// `theory/CALHA.md`'s `pleme-hotswap`/`pleme-hotswap-derive` crates
/// need: `const FIELD_CLASSES` (introspection) + `fn classify_change`
/// (comparing `self` against `new` field-by-field into ONE
/// `SwapDecision`) — unlike [`hot_swap_spec`] above, which emits N
/// independent per-field methods, this emits exactly TWO trait-impl
/// items. Consumer must bring `HotSwapClass`/`SwapDecision`/
/// `HotSwapClassifier` into scope (this spec references them
/// unqualified, matching how a real `pleme-hotswap` consumer would
/// `use pleme_hotswap::{HotSwapClass, SwapDecision, HotSwapClassifier};`).
/// See `tatara-rust-test/tests/field_tag_aggregate_end_to_end.rs` for
/// the real compiled-and-invoked proof.
#[must_use]
pub fn hot_swap_classifier_spec() -> PerFieldDeriveSpec {
    PerFieldDeriveSpec {
        trait_name: Ident::new("HotSwapClassifier"),
        target: PerFieldTarget::NamedStruct,
        trait_ref: Some("HotSwapClassifier".into()),
        per_field_template: String::new(), // unused -- aggregate mode
        method_name_template: None,
        impl_prelude: None,
        skip_fields: vec![],
        field_attribute: None,
        field_tag: Some(TagSpec {
            exhaustive: true,
            tags: vec![
                FieldTag {
                    name: "hot_swap".into(),
                    required_args: vec![],
                    per_field_template: String::new(),
                    aggregate_const_entry: Some(
                        "(stringify!(#field_name), HotSwapClass::Free),".into(),
                    ),
                    // A Free field changing needs no statement at all --
                    // the default (empty reasons -> SwapDecision::Free)
                    // already covers it. An empty template is valid
                    // here (repeated zero times contributes nothing).
                    aggregate_stmt: Some(String::new()),
                },
                FieldTag {
                    name: "restart_required".into(),
                    required_args: vec!["reason".into()],
                    per_field_template: String::new(),
                    aggregate_const_entry: Some(
                        "(stringify!(#field_name), HotSwapClass::RequiresRestart { reason: #reason }),".into(),
                    ),
                    aggregate_stmt: Some(
                        "if self.#field_name != new.#field_name { reasons.push(#reason); }".into(),
                    ),
                },
            ],
            aggregate: Some(AggregateSpec {
                const_signature: "const FIELD_CLASSES: &'static [(&'static str, HotSwapClass)] = ".into(),
                method_signature: "fn classify_change(&self, new: &Self) -> SwapDecision".into(),
                method_setup: "let mut reasons: Vec<&'static str> = Vec::new();".into(),
                method_return: concat!(
                    "if reasons.is_empty() { SwapDecision::Free } ",
                    "else { SwapDecision::RequiresRestart(reasons) }"
                )
                .into(),
            }),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_ast::CompileToCrate;
    use tatara_rust_snapshot::assert_tokens_contain;

    #[test]
    fn getter_all_emits_field_getter_template() {
        let s = getter_all_spec().compile_to_crate("getter-all-derive").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // Per the ★★ TOKEN-STABILITY directive: assert on tokens, not
        // characters. Whitespace, prettyplease formatting choices,
        // and line breaks are invisible.
        assert_tokens_contain!(&lib, "pub fn #field_name(&self) -> &#field_ty");
        assert_tokens_contain!(&lib, "#[proc_macro_derive(GetterAll)]");
    }

    #[test]
    fn with_builder_uses_method_ident_template() {
        let s = with_builder_spec().compile_to_crate("with-builder-derive").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert_tokens_contain!(&lib, "format_ident!");
        assert_tokens_contain!(&lib, r#""with_{}""#);
        assert_tokens_contain!(&lib, "#[proc_macro_derive(WithBuilder)]");
    }

    #[test]
    fn setter_all_uses_method_ident_template() {
        let s = setter_all_spec().compile_to_crate("setter-all-derive").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert_tokens_contain!(&lib, r#""set_{}""#);
        assert_tokens_contain!(&lib, "#[proc_macro_derive(SetterAll)]");
    }

    #[test]
    fn every_spec_is_self_consistent_serde() {
        for spec in [
            getter_all_spec(),
            with_builder_spec(),
            setter_all_spec(),
            hot_swap_spec(),
        ] {
            let j = serde_json::to_string(&spec).unwrap();
            let back: PerFieldDeriveSpec = serde_json::from_str(&j).unwrap();
            assert_eq!(spec, back, "round-trip failed for {}", spec.trait_name.0);
        }
    }

    #[test]
    fn hot_swap_declares_both_tag_attributes() {
        let s = hot_swap_spec().compile_to_crate("hot-swap-derive").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert_tokens_contain!(&lib, "#[proc_macro_derive(HotSwap, attributes(hot_swap, restart_required))]");
    }
}
