//! End-to-end of the **Tier-D** layer:
//!
//! - `tatara-rust-validate` — every Spec runs through correctness checks
//! - `tatara-rust-catalog` — typed fleet registry materializes to a workspace
//! - `tatara-rust-attr` — `#[derive(Prefixed)] #[prefixed(prefix = "with_")]`
//!   produces a per-field setter family whose method names are parameterized
//!   by the consumer's attribute.

use std::process::Command;

use tatara_rust_ast::{CompileToCrate, Ident};
use tatara_rust_attr::{AttrKnob, AttrValueKind, PerAttrDeriveSpec};
use tatara_rust_catalog::{CatalogEntry, CatalogSpec, MacroCatalogSpec};
use tatara_rust_derive::{PerFieldDeriveSpec, PerFieldTarget};
use tatara_rust_validate::{Validate, Violation};

fn prefixed_spec() -> PerAttrDeriveSpec {
    PerAttrDeriveSpec {
        trait_name: Ident::new("Prefixed"),
        knobs: vec![AttrKnob {
            name: "prefix".into(),
            kind: AttrValueKind::Str,
            default: Some("with_".into()),
        }],
        per_field_template:
            "pub fn #prefix(mut self, v: #field_ty) -> Self { self.#field_name = v; self }"
                .into(),
    }
}

#[test]
fn validate_catches_empty_trait_name() {
    let mut s = prefixed_spec();
    s.trait_name = Ident::new("");
    // PerAttrDeriveSpec is not Validate today (only the named L2 set has
    // impls); use a Spec that IS Validate for the smoke.
    let mut pfs = PerFieldDeriveSpec {
        trait_name: Ident::new(""),
        target: PerFieldTarget::NamedStruct,
        trait_ref: None,
        per_field_template:
            "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
        method_name_template: None,
        impl_prelude: None,
        skip_fields: vec![],
        field_attribute: None,
        field_tag: None,
    };
    let v = pfs.validate();
    assert!(matches!(v.first(), Some(Violation::EmptyIdent { .. })));
    pfs.trait_name = Ident::new("Ok");
    assert!(pfs.validate().is_empty());
    let _ = s;
}

#[test]
fn catalog_materializes_and_runs_validation() {
    let cat = MacroCatalogSpec {
        title: "tier-d smoke".into(),
        entries: vec![CatalogEntry {
            crate_name: "getter-derive".into(),
            description: "Per-field getters".into(),
            since: "0.1.0".into(),
            owner: "pleme-io".into(),
            verifier_hint: None,
            spec: CatalogSpec::PerField {
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
        }],
    };
    let scaffold = cat.compile_to_catalog().expect("clean catalog");
    assert!(scaffold.catalog_json.contains("getter-derive"));
    assert_eq!(scaffold.member_crates.len(), 1);
    assert_eq!(scaffold.docs_md.len(), 1);
    assert!(scaffold.workspace_md.contains("`getter-derive`"));
}

#[test]
#[ignore = "slow: runs `cargo test` in a temp dir; opt in with `-- --ignored`"]
fn per_attr_derive_with_consumer_supplied_prefix() {
    let spec = prefixed_spec();
    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-attr-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let crate_name = "prefixed-derive";
    spec.compile_to_crate(crate_name)
        .unwrap()
        .write_to(&tmp.join(crate_name))
        .unwrap();

    let cons = tmp.join("consumer");
    std::fs::create_dir_all(cons.join("src")).unwrap();
    std::fs::write(
        cons.join("Cargo.toml"),
        format!(
            r#"[package]
name = "consumer"
version = "0.1.0"
edition = "2024"

[dependencies]
{crate_name} = {{ path = "../{crate_name}" }}

[lib]
path = "src/lib.rs"
"#
        ),
    )
    .unwrap();

    std::fs::write(
        cons.join("src/lib.rs"),
        r#"use prefixed_derive::Prefixed;

// Default prefix: `with_`.
#[derive(Prefixed)]
pub struct DefaultBuilder { pub a: i32, pub b: String }
impl DefaultBuilder { pub fn new() -> Self { Self { a: 0, b: String::new() } } }

// Override: `set_` prefix via the #[prefixed(prefix = …)] attribute.
#[derive(Prefixed)]
#[prefixed(prefix = "set_")]
pub struct SetBuilder { pub a: i32, pub b: String }
impl SetBuilder { pub fn new() -> Self { Self { a: 0, b: String::new() } } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_prefix_with() {
        let b = DefaultBuilder::new().with_a(42).with_b("hi".into());
        assert_eq!(b.a, 42);
        assert_eq!(b.b, "hi");
    }
    #[test]
    fn override_prefix_set() {
        let b = SetBuilder::new().set_a(7).set_b("yo".into());
        assert_eq!(b.a, 7);
        assert_eq!(b.b, "yo");
    }
}
"#,
    )
    .unwrap();

    let status = Command::new("cargo")
        .arg("test")
        .current_dir(&cons)
        .status()
        .expect("spawn cargo");
    assert!(status.success(), "per-attr derive consumer failed cargo test");

    let _ = std::fs::remove_dir_all(&tmp);
}
