//! Materialize a trivial Rust library to disk, then prove the green
//! gate runner reports `GateOutcome::Passed`. Establishes the end-to-end
//! contract: `Cargo.toml` + `src/lib.rs` → green sweep.
//!
//! Skipped automatically if `cargo` isn't on PATH (CI matrix without
//! Rust toolchain).

use std::path::PathBuf;
use tatara_rust_gate::{Gate, GateConfig, GateOutcome, green_gate};

fn cargo_present() -> bool {
    std::process::Command::new("cargo")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn write_trivial_lib(root: &PathBuf) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        concat!(
            "[package]\n",
            "name = \"tatara-gate-canonical\"\n",
            "version = \"0.1.0\"\n",
            "edition = \"2024\"\n",
            "\n",
            "[lib]\n",
            "path = \"src/lib.rs\"\n",
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        concat!(
            "//! tatara-gate canonical fixture.\n",
            "#[must_use]\n",
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
            "\n",
            "#[cfg(test)]\n",
            "mod tests { use super::*; #[test] fn adds() { assert_eq!(add(1, 2), 3); } }\n",
        ),
    )
    .unwrap();
}

#[test]
#[ignore = "spawns cargo; ~10s; skip with --ignored"]
fn build_and_test_gates_pass_on_trivial_lib() {
    if !cargo_present() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-gate-canonical-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_trivial_lib(&tmp);

    let cfg = GateConfig {
        gates: vec![Gate::Build, Gate::Test], // skip clippy — toolchain may lack the lint set
        skip_if_no_cargo_toml: true,
    };
    let outcome = green_gate(&tmp, &cfg).unwrap();
    assert!(
        outcome.is_passed(),
        "expected Passed, got: {:?}",
        match &outcome {
            GateOutcome::Failed { gate, stderr, .. } => {
                format!("Failed at {}: {}", gate.label(), stderr)
            }
            other => format!("{:?}", other),
        }
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
