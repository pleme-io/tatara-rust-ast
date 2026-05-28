//! End-to-end: a `MacroSuiteSpec` bundling a derive + a macro_rules
//! materializes to a real Cargo workspace on disk; running `cargo test`
//! against it builds both members + a consumer that uses both macros.

use std::process::Command;

use tatara_rust_ast::Ident;
use tatara_rust_derive::ProcDeriveSpec;
use tatara_rust_macro_rules::{MacroArm, MacroRulesSpec};
use tatara_rust_suite::{MacroMemberSpec, MacroSuiteSpec};

#[test]
#[ignore = "slow: runs `cargo test` in a temp dir; opt in with `-- --ignored`"]
fn macro_suite_compiles_end_to_end() {
    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-suite-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let suite = MacroSuiteSpec {
        workspace_name: "demo-macros".into(),
        members: vec![
            // A derive that emits an `impl Marker for $T {}` empty impl.
            MacroMemberSpec::Derive {
                crate_name: "marker-derive".into(),
                spec: ProcDeriveSpec::new("Marker", vec![]),
            },
            // A macro_rules that's `identity!(x)` → `{ x }`.
            MacroMemberSpec::MacroRules {
                crate_name: "identity-macros".into(),
                spec: MacroRulesSpec {
                    macro_name: Ident::new("identity"),
                    arms: vec![MacroArm {
                        matcher: "($x:expr)".into(),
                        transcriber: "{ $x }".into(),
                    }],
                },
            },
        ],
    };

    let ws_root = tmp.join("demo-macros-ws");
    suite
        .compile_to_workspace()
        .unwrap()
        .write_to(&ws_root)
        .unwrap();

    // A trait crate for the derive to impl. Pre-existing module in the
    // workspace so the derive has something to implement against.
    let trait_dir = ws_root.join("crates/marker-trait");
    std::fs::create_dir_all(trait_dir.join("src")).unwrap();
    std::fs::write(
        trait_dir.join("Cargo.toml"),
        r#"[package]
name = "marker-trait"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();
    std::fs::write(trait_dir.join("src/lib.rs"), "pub trait Marker {}\n").unwrap();

    // Consumer that exercises both members. Lives inside the same
    // workspace so the `path = "../X"` deps resolve.
    let cons_dir = ws_root.join("crates/consumer");
    std::fs::create_dir_all(cons_dir.join("src")).unwrap();
    std::fs::write(
        cons_dir.join("Cargo.toml"),
        r#"[package]
name = "consumer"
version = "0.1.0"
edition = "2024"

[dependencies]
marker-trait    = { path = "../marker-trait" }
marker-derive   = { path = "../marker-derive" }
identity-macros = { path = "../identity-macros" }

[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();
    std::fs::write(
        cons_dir.join("src/lib.rs"),
        r#"use marker_trait::Marker;
use marker_derive::Marker as MarkerDerive;

#[derive(MarkerDerive)]
pub struct Thing;

pub fn run() -> i32 {
    identity_macros::identity!(42)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thing_is_marker() {
        fn assert_marker<T: Marker>(_t: &T) {}
        assert_marker(&Thing);
    }
    #[test]
    fn identity_macro_returns_input() {
        assert_eq!(run(), 42);
    }
}
"#,
    )
    .unwrap();

    // Update the root Cargo.toml's members to include the new crates.
    // (The suite emitter only includes the auto-generated members; we
    // append the hand-rolled consumer + trait.)
    let root_toml = ws_root.join("Cargo.toml");
    let mut current = std::fs::read_to_string(&root_toml).unwrap();
    current = current.replace(
        r#"members = [
  "crates/marker-derive","#,
        r#"members = [
  "crates/marker-trait",
  "crates/consumer",
  "crates/marker-derive","#,
    );
    std::fs::write(&root_toml, current).unwrap();

    // Drive cargo test against the whole workspace.
    let status = Command::new("cargo")
        .arg("test")
        .arg("--workspace")
        .current_dir(&ws_root)
        .status()
        .expect("spawn cargo");

    assert!(
        status.success(),
        "suite workspace failed cargo test at {}",
        ws_root.display()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
