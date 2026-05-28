//! End-to-end smoke for `catalog-emit-repos`: catalog JSON →
//! N independently decorated repos on disk. Every repo carries the
//! full OSS-publish surface (flake, caixa, auto-release, clippy, license,
//! gitignore, rust-toolchain, README).

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
fn catalog_emit_repos_materializes_n_decorated_repos() {
    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-forge-emit-repos-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let catalog_json = r#"{
        "title": "smoke-farm",
        "entries": [
            {
                "crate_name": "smoke-marker",
                "description": "Marker derive.",
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
    }"#;
    let catalog_path = tmp.join("catalog.json");
    std::fs::write(&catalog_path, catalog_json).unwrap();
    let out_dir = tmp.join("out");

    let status = Command::new(binary_path())
        .arg("catalog-emit-repos")
        .arg(&catalog_path)
        .arg("--out")
        .arg(&out_dir)
        .arg("--repo-url-prefix")
        .arg("https://github.com/pleme-io")
        .status();

    let Ok(status) = status else {
        eprintln!("skipping cli_emit_repos: forge binary not built");
        return;
    };
    assert!(status.success(), "catalog-emit-repos exited non-zero");

    for repo_name in ["smoke-marker", "smoke-getter"] {
        let root = out_dir.join(repo_name);
        for rel in [
            "Cargo.toml",
            "src/lib.rs",
            "flake.nix",
            "caixa.lisp",
            ".github/workflows/auto-release.yml",
            "clippy.toml",
            "LICENSE",
            ".gitignore",
            "rust-toolchain.toml",
            "README.md",
        ] {
            let p = root.join(rel);
            assert!(p.exists(), "{repo_name}: missing {}", p.display());
        }
        // README must carry the per-repo crate name.
        let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
        assert!(readme.contains(&format!("# {repo_name}")));
    }

    let _ = std::fs::remove_dir_all(&tmp);
}
