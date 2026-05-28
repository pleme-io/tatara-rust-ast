//! Worked-example macro Specs — the canonical real-world primitives.
//!
//! Each function returns a typed Spec value that compiles to a publishable
//! proc-macro crate. Combined, they prove `tatara-rust-ast` produces useful,
//! reusable derive macros from typed data alone — no per-derive bespoke
//! proc-macro authoring.
//!
//! End-to-end coverage in `tatara-rust-test/tests/per_field_end_to_end.rs`.

use tatara_rust_ast::Ident;
use tatara_rust_derive::{PerFieldDeriveSpec, PerFieldTarget};

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
        for spec in [getter_all_spec(), with_builder_spec(), setter_all_spec()] {
            let j = serde_json::to_string(&spec).unwrap();
            let back: PerFieldDeriveSpec = serde_json::from_str(&j).unwrap();
            assert_eq!(spec, back, "round-trip failed for {}", spec.trait_name.0);
        }
    }
}
