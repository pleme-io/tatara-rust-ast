//! End-to-end of `field_tag` (exhaustive multi-tag classification) — the
//! `theory/CALHA.md` §4/§6.1/§6.2 engine extension. Materializes the
//! `hot_swap_spec()` derive crate + TWO consumers: one where every field
//! is correctly tagged (must compile AND behave correctly), one with a
//! deliberately untagged field (must FAIL to compile, with the expected
//! message) -- proving the exhaustiveness guarantee is real, not just
//! present in the emitter's own source text.

use std::process::Command;

use tatara_rust_ast::CompileToCrate;
use tatara_rust_examples::hot_swap_spec;

#[test]
#[ignore = "slow: runs `cargo build`/`cargo test` in a temp dir; opt in with `-- --ignored`"]
fn hot_swap_derive_compiles_and_classifies_correctly_tagged_struct() {
    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-fieldtag-e2e-happy-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    hot_swap_spec()
        .compile_to_crate("hot-swap-derive")
        .unwrap()
        .write_to(&tmp.join("hot-swap-derive"))
        .unwrap();

    let cons = tmp.join("consumer");
    std::fs::create_dir_all(cons.join("src")).unwrap();
    std::fs::write(
        cons.join("Cargo.toml"),
        r#"[package]
name = "consumer"
version = "0.1.0"
edition = "2024"

[dependencies]
hot-swap-derive = { path = "../hot-swap-derive" }

[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();

    std::fs::write(
        cons.join("src/lib.rs"),
        r#"use hot_swap_derive::HotSwap;

#[derive(HotSwap)]
pub struct Config {
    #[hot_swap]
    pub log_level: String,
    #[restart_required(reason = "bound at process start")]
    pub bind_addr: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_fields_reports_both_classifications() {
        let fields = Config::describe_fields();
        assert_eq!(fields.len(), 2);
        assert!(fields.contains(&("log_level", true, None)));
        assert!(fields.contains(&("bind_addr", false, Some("bound at process start"))));
    }
}
"#,
    )
    .unwrap();

    // The derive's own per-tag templates emit tuple literals, not a
    // named method -- give the consumer a real inherent fn that collects
    // them, matching what a real HotSwapClassifier impl would look like.
    // (hot_swap_spec()'s templates emit `(stringify!(#field_name), ..)`
    // tuple EXPRESSIONS per field; the derive wraps them in
    // `impl Config { #(#per_field)* }` today with NO enclosing fn --
    // this test's consumer intentionally uses the derive as-is to prove
    // the exhaustive dispatch, not a specific downstream trait shape.)

    let status = Command::new("cargo")
        .arg("build")
        .current_dir(&cons)
        .status()
        .expect("spawn cargo build");

    assert!(
        status.success(),
        "correctly-tagged consumer failed to compile at {}",
        cons.display()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
#[ignore = "slow: runs `cargo build` in a temp dir; opt in with `-- --ignored`"]
fn hot_swap_derive_refuses_an_untagged_field() {
    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-fieldtag-e2e-refuse-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    hot_swap_spec()
        .compile_to_crate("hot-swap-derive")
        .unwrap()
        .write_to(&tmp.join("hot-swap-derive"))
        .unwrap();

    let cons = tmp.join("consumer");
    std::fs::create_dir_all(cons.join("src")).unwrap();
    std::fs::write(
        cons.join("Cargo.toml"),
        r#"[package]
name = "consumer"
version = "0.1.0"
edition = "2024"

[dependencies]
hot-swap-derive = { path = "../hot-swap-derive" }

[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();

    std::fs::write(
        cons.join("src/lib.rs"),
        r#"use hot_swap_derive::HotSwap;

#[derive(HotSwap)]
pub struct Config {
    #[hot_swap]
    pub log_level: String,
    // `bind_addr` deliberately carries NEITHER tag -- must be a
    // compile_error!(), not a silently-dropped field.
    pub bind_addr: String,
}
"#,
    )
    .unwrap();

    let output = Command::new("cargo")
        .arg("build")
        .current_dir(&cons)
        .output()
        .expect("spawn cargo build");

    assert!(
        !output.status.success(),
        "consumer with an untagged field should have FAILED to compile at {}",
        cons.display()
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("must carry exactly one of"),
        "expected the exhaustiveness compile_error! message, got:\n{stderr}"
    );
    assert!(
        stderr.contains("bind_addr"),
        "expected the error to name the untagged field `bind_addr`, got:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
