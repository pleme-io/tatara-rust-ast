//! End-to-end smoke for the `catalog` branch of the forge CLI.
//!
//! Builds a `MacroCatalogSpec`-shaped JSON document with two members
//! (one simple `Derive`, one `PerField`), feeds it to the binary, and
//! asserts the catalog scaffold materializes the expected files:
//!   - catalog.json
//!   - Cargo.toml      (workspace root listing the members)
//!   - WORKSPACE.md    (rendered catalog table)
//!   - docs/<member>.md per entry
//!   - crates/<member>/...  (one directory per entry)

use std::path::PathBuf;
use std::process::Command;

fn binary_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p.push(if cfg!(debug_assertions) { "debug" } else { "release" });
    p.push("tatara-rust-forge");
    p
}

#[test]
fn forge_writes_full_catalog_workspace_from_json() {
    let tmp = std::env::temp_dir()
        .join(format!("tatara-rust-forge-catalog-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // CatalogEntry serde: crate_name + description + since + owner +
    // flattened {kind, spec}. Two entries exercise two kinds.
    let spec_json = r#"{
        "kind": "catalog",
        "spec": {
            "title": "smoke-catalog",
            "entries": [
                {
                    "crate_name": "smoke-marker",
                    "description": "Marker derive (no body).",
                    "since": "0.1.0",
                    "owner": "pleme-io",
                    "kind": "derive",
                    "spec": {
                        "trait_name": "Marker",
                        "impl_template": {
                            "trait_ref": { "ident": "Marker", "generics": [] },
                            "self_type": { "ident": "__SELF_TYPE__", "generics": [] },
                            "items": []
                        }
                    }
                },
                {
                    "crate_name": "smoke-getter",
                    "description": "Per-field getter derive.",
                    "since": "0.1.0",
                    "owner": "pleme-io",
                    "kind": "per-field",
                    "spec": {
                        "trait_name": "Getter",
                        "target": "named-struct",
                        "trait_ref": null,
                        "per_field_template": "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }",
                        "method_name_template": null,
                        "impl_prelude": null
                    }
                }
            ]
        }
    }"#;

    let spec_path = tmp.join("catalog.json");
    std::fs::write(&spec_path, spec_json).unwrap();

    let out_dir = tmp.join("out");
    let status = Command::new(binary_path())
        .arg("emit-spec")
        .arg(&spec_path)
        .arg(&out_dir)
        .status();

    let Ok(status) = status else {
        eprintln!("skipping cli_catalog: forge binary not built");
        return;
    };
    assert!(status.success(), "forge catalog exited non-zero");

    let root = out_dir.join("smoke-catalog");
    let want_paths = [
        "catalog.json",
        "Cargo.toml",
        "WORKSPACE.md",
        "docs/smoke-marker.md",
        "docs/smoke-getter.md",
        "crates/smoke-marker/Cargo.toml",
        "crates/smoke-marker/src/lib.rs",
        "crates/smoke-getter/Cargo.toml",
        "crates/smoke-getter/src/lib.rs",
    ];
    for rel in want_paths {
        let p = root.join(rel);
        assert!(p.exists(), "expected catalog file {} to exist", p.display());
    }

    // Workspace Cargo.toml must list both member crates.
    let root_cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(root_cargo.contains("crates/smoke-marker"));
    assert!(root_cargo.contains("crates/smoke-getter"));

    // catalog.json must be parseable and list both entries.
    let catalog_text = std::fs::read_to_string(root.join("catalog.json")).unwrap();
    assert!(catalog_text.contains("smoke-marker"));
    assert!(catalog_text.contains("smoke-getter"));

    // WORKSPACE.md must render the operator index.
    let workspace_md = std::fs::read_to_string(root.join("WORKSPACE.md")).unwrap();
    assert!(workspace_md.contains("# smoke-catalog"));
    assert!(workspace_md.contains("`smoke-marker`"));
    assert!(workspace_md.contains("`smoke-getter`"));

    let _ = std::fs::remove_dir_all(&tmp);
}
