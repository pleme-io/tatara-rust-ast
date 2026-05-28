//! End-to-end of the **PerFieldDerive** higher-order primitive +
//! the worked example Specs in `tatara-rust-examples`.
//!
//! Materializes a derive crate per example + a consumer that exercises
//! each derive against a real `struct Foo { a: i32, b: String }`,
//! then drives `cargo test` to prove every emitted method works.

use std::process::Command;

use tatara_rust_ast::CompileToCrate;
use tatara_rust_examples::{getter_all_spec, setter_all_spec, with_builder_spec};

#[test]
#[ignore = "slow: runs `cargo test` in a temp dir; opt in with `-- --ignored`"]
fn worked_derives_compile_and_pass_consumer_assertions() {
    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-perfield-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let derives: &[(fn() -> _, &str)] = &[
        (getter_all_spec, "getter-all-derive"),
        (with_builder_spec, "with-builder-derive"),
        (setter_all_spec, "setter-all-derive"),
    ];
    for (spec_fn, crate_name) in derives {
        spec_fn()
            .compile_to_crate(crate_name)
            .unwrap()
            .write_to(&tmp.join(crate_name))
            .unwrap();
    }

    let cons = tmp.join("consumer");
    std::fs::create_dir_all(cons.join("src")).unwrap();
    std::fs::write(
        cons.join("Cargo.toml"),
        r#"[package]
name = "consumer"
version = "0.1.0"
edition = "2024"

[dependencies]
getter-all-derive   = { path = "../getter-all-derive" }
with-builder-derive = { path = "../with-builder-derive" }
setter-all-derive   = { path = "../setter-all-derive" }

[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();

    std::fs::write(
        cons.join("src/lib.rs"),
        r#"use getter_all_derive::GetterAll;
use with_builder_derive::WithBuilder;
use setter_all_derive::SetterAll;

#[derive(GetterAll, WithBuilder, SetterAll)]
pub struct Foo {
    pub a: i32,
    pub b: String,
}

impl Foo {
    pub fn new() -> Self { Self { a: 0, b: String::new() } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getter_all_returns_borrows() {
        let f = Foo { a: 7, b: "hi".into() };
        assert_eq!(*f.a(), 7);
        assert_eq!(f.b(), "hi");
    }

    #[test]
    fn with_builder_chains_fluently() {
        let f = Foo::new().with_a(42).with_b("world".into());
        assert_eq!(*f.a(), 42);
        assert_eq!(f.b(), "world");
    }

    #[test]
    fn setter_all_mutates_in_place() {
        let mut f = Foo::new();
        f.set_a(99);
        f.set_b("set".into());
        assert_eq!(*f.a(), 99);
        assert_eq!(f.b(), "set");
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

    assert!(
        status.success(),
        "per-field derive consumer failed cargo test at {}",
        cons.display()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
