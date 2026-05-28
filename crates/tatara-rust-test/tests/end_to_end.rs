//! End-to-end: typed `ProcDeriveSpec` → real Cargo crates on disk →
//! `cargo test` against the generated derive macro succeeds.
//!
//! This test runs `cargo test` in a temp dir to PROVE the generated
//! crate compiles and the derive actually works on a consumer struct.
//! Slow (~30s on first run); gated behind a CARGO_TARGET_TMPDIR check.

use std::process::Command;

use tatara_rust_ast::{
    Block, CompileToCrate, Expr, Fn as RsFn, FnSig, Generics, Ident, RefKind, Stmt, TypeRef,
};
use tatara_rust_derive::ProcDeriveSpec;

fn static_name_spec() -> ProcDeriveSpec {
    ProcDeriveSpec::new(
        "StaticName",
        vec![RsFn {
            sig: FnSig {
                name: Ident::new("type_name"),
                generics: Generics::default(),
                params: vec![],
                return_type: Some(TypeRef {
                    ident: Ident::new("str"),
                    generics: vec![],
                    reference: Some(RefKind::shared_lifetime("static")),
                }),
            },
            body: Block {
                stmts: vec![Stmt::Tail {
                    expr: Expr::MacroCall {
                        path: vec![Ident::new("stringify")],
                        tokens: "#__SELF_NAME__".into(),
                    },
                }],
            },
        }],
    )
}

#[test]
#[ignore = "slow: runs `cargo test` in a temp dir; opt in with --include-ignored"]
fn generated_derive_compiles_and_runs() {
    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-derive-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // 1. Emit the derive crate alongside a trait crate alongside a
    //    consumer crate. The trait crate publishes `StaticName`; the
    //    derive crate generates the impl; the consumer wires both.
    let spec = static_name_spec();
    let derive_root = tmp.join("static-name-derive");
    spec.compile_to_crate("static-name-derive")
        .unwrap()
        .write_to(&derive_root)
        .unwrap();

    // Trait crate — defines `pub trait StaticName { fn type_name() -> &'static str; }`.
    let trait_root = tmp.join("static-name");
    std::fs::create_dir_all(trait_root.join("src")).unwrap();
    std::fs::write(
        trait_root.join("Cargo.toml"),
        r#"[package]
name = "static-name"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();
    std::fs::write(
        trait_root.join("src/lib.rs"),
        r#"pub trait StaticName { fn type_name() -> &'static str; }
"#,
    )
    .unwrap();

    // Consumer crate — uses both.
    let cons_root = tmp.join("static-name-consumer");
    std::fs::create_dir_all(cons_root.join("src")).unwrap();
    std::fs::write(
        cons_root.join("Cargo.toml"),
        r#"[package]
name = "static-name-consumer"
version = "0.1.0"
edition = "2024"

[dependencies]
static-name = { path = "../static-name" }
static-name-derive = { path = "../static-name-derive" }

[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();
    std::fs::write(
        cons_root.join("src/lib.rs"),
        r#"use static_name::StaticName;
use static_name_derive::StaticName as StaticNameDerive;

#[derive(StaticNameDerive)]
pub struct Widget;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn widget_has_static_name() {
        assert_eq!(Widget::type_name(), "Widget");
    }
}
"#,
    )
    .unwrap();

    // 2. Drive `cargo test` against the consumer.
    let status = Command::new("cargo")
        .arg("test")
        .current_dir(&cons_root)
        .status()
        .expect("spawn cargo");

    assert!(
        status.success(),
        "generated derive failed cargo test at {}",
        cons_root.display()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
