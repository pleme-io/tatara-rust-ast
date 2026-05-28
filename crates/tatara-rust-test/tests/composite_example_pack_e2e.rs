//! End-to-end of the **full stack** built in Round 1+2+3:
//!
//! - PerField + PerVariant Specs bundled by CompositeDeriveSpec
//! - DSL macros constructing the Specs
//! - Caixa + flake decoration verified on the in-memory scaffold
//! - DeriveExamplePackSpec driving the consumer cargo test
//!
//! One typed value → one publishable crate + a passing assertion.

use tatara_rust_ast::CompileToCrate;
use tatara_rust_composite::{CompositeDeriveSpec, CompositeMember};
use tatara_rust_test::{DeriveExamplePackSpec, Example};

#[test]
fn decorations_attach_to_composite_scaffold() {
    let getter = tatara_rust_dsl::defperfield! {
        trait_name: "AccessorGetter",
        template: "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }",
    };
    let setter = tatara_rust_dsl::defperfield! {
        trait_name: "AccessorSetter",
        template: "pub fn #method_ident(&mut self, v: #field_ty) { self.#field_name = v; }",
        method_template: "set_{}",
    };
    let bundle = CompositeDeriveSpec {
        bundle_name: tatara_rust_ast::Ident::new("Accessor"),
        members: vec![
            CompositeMember::PerField(getter),
            CompositeMember::PerField(setter),
        ],
    };
    let mut scaffold = bundle.compile_to_crate("accessor-derive").unwrap();
    tatara_rust_flake::attach_substrate_flake(&mut scaffold);
    tatara_rust_caixa::attach_caixa_biblioteca(
        &mut scaffold,
        &tatara_rust_caixa::CaixaConfig {
            description: Some("Composite getter+setter derive".into()),
            attach_auto_release: true,
        },
    );
    let files = scaffold.to_files();
    assert!(files.contains_key("Cargo.toml"));
    assert!(files.contains_key("src/lib.rs"));
    assert!(files.contains_key("flake.nix"));
    assert!(files.contains_key("caixa.lisp"));
    assert!(files.contains_key(".github/workflows/auto-release.yml"));
    assert!(files["src/lib.rs"].contains("#[proc_macro_derive(Accessor)]"));
    assert!(files["caixa.lisp"].contains(r#":kind         "Biblioteca""#));
    assert!(files["flake.nix"].contains("mkRustToolFlake"));
}

#[test]
#[ignore = "slow: runs `cargo test` in a temp dir; opt in with `-- --ignored`"]
fn composite_accessor_via_example_pack() {
    let getter = tatara_rust_dsl::defperfield! {
        trait_name: "AccessorGetter",
        template: "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }",
    };
    let setter = tatara_rust_dsl::defperfield! {
        trait_name: "AccessorSetter",
        template: "pub fn #method_ident(&mut self, v: #field_ty) { self.#field_name = v; }",
        method_template: "set_{}",
    };
    let bundle = CompositeDeriveSpec {
        bundle_name: tatara_rust_ast::Ident::new("Accessor"),
        members: vec![
            CompositeMember::PerField(getter),
            CompositeMember::PerField(setter),
        ],
    };

    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-composite-pack-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);

    let pack = DeriveExamplePackSpec {
        derive_crate_name: "accessor-derive".into(),
        trait_name: "Accessor".into(),
        spec: &bundle,
        extra_consumer_imports: vec![],
        auxiliary_trait_crates: vec![],
        examples: vec![Example {
            name: "thing".into(),
            consumer_item: "pub struct Thing { pub a: i32, pub b: String }".into(),
            assertion_body: r#"
        let mut t = Thing { a: 1, b: "x".into() };
        assert_eq!(*t.a(), 1);
        t.set_a(42);
        t.set_b("y".into());
        assert_eq!(*t.a(), 42);
        assert_eq!(t.b(), "y");"#
                .into(),
        }],
    };
    let report = pack.run_under(&tmp).unwrap();
    assert!(report.cargo_test_succeeded, "consumer cargo test failed");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
#[ignore = "slow: runs `cargo test` in a temp dir; opt in with `-- --ignored`"]
fn per_variant_is_variant_via_example_pack() {
    let spec = tatara_rust_dsl::defpervariant! {
        trait_name: "IsVariant",
        template: "pub fn #method_ident(&self) -> bool { matches!(self, #variant_shape_arm) }",
        method_template: "is_{}",
    };

    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-variant-pack-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);

    let pack = DeriveExamplePackSpec {
        derive_crate_name: "is-variant-derive".into(),
        trait_name: "IsVariant".into(),
        spec: &spec,
        extra_consumer_imports: vec![],
        auxiliary_trait_crates: vec![],
        examples: vec![Example {
            name: "color".into(),
            consumer_item: "pub enum Color { Red, Green(i32), Blue { shade: u8 } }".into(),
            assertion_body: r#"
        assert!(Color::Red.is_red());
        assert!(!Color::Red.is_green());
        assert!(Color::Green(0).is_green());
        assert!(!Color::Green(0).is_blue());
        assert!(Color::Blue { shade: 1 }.is_blue());
        assert!(!Color::Blue { shade: 1 }.is_red());"#
                .into(),
        }],
    };
    let report = pack.run_under(&tmp).unwrap();
    assert!(report.cargo_test_succeeded, "consumer cargo test failed");

    let _ = std::fs::remove_dir_all(&tmp);
}
