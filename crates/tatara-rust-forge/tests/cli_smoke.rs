//! Smoke test: the `tatara-rust-forge` CLI binary materializes a
//! JSON-described Spec to disk and exits 0.

use std::path::PathBuf;
use std::process::Command;

fn binary_path() -> PathBuf {
    // Walk up from CARGO_MANIFEST_DIR (crates/tatara-rust-forge) to the
    // workspace root, then into target/.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("target");
    p.push(if cfg!(debug_assertions) { "debug" } else { "release" });
    p.push("tatara-rust-forge");
    p
}

#[test]
fn forge_writes_derive_crate_from_json() {
    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-forge-cli-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // Minimal valid Derive spec — exercise the kind dispatch + write path.
    let spec_json = r#"{
        "kind": "derive",
        "crate_name": "smoke-derive",
        "spec": {
            "trait_name": "Smoke",
            "impl_template": {
                "trait_ref": { "ident": "Smoke", "generics": [] },
                "self_type": { "ident": "__SELF_TYPE__", "generics": [] },
                "items": []
            }
        }
    }"#;
    let spec_path = tmp.join("spec.json");
    std::fs::write(&spec_path, spec_json).unwrap();

    let out_dir = tmp.join("out");
    let status = Command::new(binary_path())
        .arg("emit-spec")
        .arg(&spec_path)
        .arg(&out_dir)
        .status();

    // Skip when the binary isn't built (e.g. someone ran `cargo test -p tatara-rust-forge --lib`).
    let Ok(status) = status else {
        eprintln!("skipping cli_smoke: forge binary not built");
        return;
    };
    assert!(status.success(), "forge exited non-zero");

    let lib = out_dir.join("smoke-derive/src/lib.rs");
    assert!(lib.exists(), "expected {} to exist", lib.display());
    let contents = std::fs::read_to_string(&lib).unwrap();
    assert!(contents.contains("#[proc_macro_derive(Smoke)]"));

    let cargo = out_dir.join("smoke-derive/Cargo.toml");
    assert!(cargo.exists());
    assert!(
        std::fs::read_to_string(&cargo)
            .unwrap()
            .contains("proc-macro = true")
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
