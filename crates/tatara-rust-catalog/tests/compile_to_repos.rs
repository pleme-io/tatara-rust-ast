//! `MacroCatalogSpec::compile_to_repos` end-to-end — 3-entry catalog
//! (derive + per-field + per-variant) → 3 fully decorated `RepoSpec`s,
//! each writes to its own directory.

use tatara_rust_ast::Ident;
use tatara_rust_ast::CompileToCrate as _;
use tatara_rust_catalog::{CatalogEntry, CatalogSpec, MacroCatalogSpec};
use tatara_rust_derive::{
    PerFieldDeriveSpec, PerFieldTarget, PerVariantDeriveSpec, ProcDeriveSpec, VariantShape,
};

fn cat() -> MacroCatalogSpec {
    MacroCatalogSpec {
        title: "tests-mini".into(),
        entries: vec![
            CatalogEntry {
                crate_name: "marker-derive".into(),
                description: "Marker trait derive.".into(),
                since: "0.1.0".into(),
                owner: "pleme-io".into(),
                verifier_hint: None,
                spec: CatalogSpec::Derive {
                    spec: ProcDeriveSpec::new("Marker", vec![]),
                },
            },
            CatalogEntry {
                crate_name: "getter-all-derive".into(),
                description: "Per-field inherent getter derive.".into(),
                since: "0.1.0".into(),
                owner: "pleme-io".into(),
                verifier_hint: None,
                spec: CatalogSpec::PerField {
                    spec: PerFieldDeriveSpec {
                        trait_name: Ident::new("GetterAll"),
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
            },
            CatalogEntry {
                crate_name: "is-variant-derive".into(),
                description: "Per-variant matches-style predicate.".into(),
                since: "0.1.0".into(),
                owner: "pleme-io".into(),
                verifier_hint: None,
                spec: CatalogSpec::PerVariant {
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
            },
        ],
    }
}

#[test]
fn three_entries_emit_three_decorated_repos() {
    let repos = cat()
        .compile_to_repos("https://github.com/pleme-io")
        .unwrap();
    assert_eq!(repos.len(), 3);

    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-catalog-compile-to-repos-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    for spec in repos {
        let compiled = spec.compile();
        let name = compiled.name.clone();
        let root = tmp.join(&name);
        compiled.write_to(&root).unwrap();
        for rel in [
            "Cargo.toml",
            "src/lib.rs",
            "flake.nix",
            "caixa.lisp",
            ".github/workflows/auto-release.yml",
            "clippy.toml",
            "LICENSE",
            ".gitignore",
            "README.md",
        ] {
            let p = root.join(rel);
            assert!(
                p.exists(),
                "{} missing in materialized repo {}",
                rel,
                name
            );
        }
        // README must name the right crate (proves per-entry parameterization).
        let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
        assert!(readme.contains(&format!("# {name}")));
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn compile_to_repos_rejects_invalid_entries() {
    let mut bad = cat();
    if let CatalogSpec::PerField { spec } = &mut bad.entries[1].spec {
        spec.trait_name = Ident::new("");
    }
    let err = bad
        .compile_to_repos("https://github.com/pleme-io")
        .unwrap_err();
    assert!(matches!(
        err,
        tatara_rust_catalog::CatalogError::InvalidEntries { .. }
    ));
}

#[test]
fn each_emitted_scaffold_still_compiles_via_kind_dispatch() {
    // Sanity: every catalog entry's spec is callable through the typed
    // CompileToCrate trait directly; compile_to_repos is just a wrapper.
    for e in cat().entries {
        let _ = e
            .spec
            .compile_to_crate(&e.crate_name)
            .expect("each entry compiles");
    }
}
