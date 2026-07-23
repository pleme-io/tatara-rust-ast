//! End-to-end of `field_tag`'s AGGREGATE shape (`AggregateSpec`) — the
//! `HotSwapClassifier` trait-impl shape `theory/CALHA.md`'s real
//! `pleme-hotswap`/`pleme-hotswap-derive` crates need: one `const
//! FIELD_CLASSES` + one `classify_change` method spanning ALL fields,
//! not N independent per-field items (that's `field_tag_end_to_end.rs`).
//!
//! Materializes the generated derive crate + a real consumer that
//! locally declares the `HotSwapClass`/`SwapDecision`/
//! `HotSwapClassifier` types (self-contained, no external crate dep —
//! matching this repo's own convention), applies `#[derive(HotSwapClassifier)]`
//! to a two-field struct, and proves `classify_change` correctly
//! aggregates BOTH fields into one `SwapDecision` across four real
//! scenarios (no change / only the Free field changed / only the
//! restart-required field changed / both changed).

use std::process::Command;

use tatara_rust_ast::CompileToCrate;
use tatara_rust_examples::hot_swap_classifier_spec;

#[test]
#[ignore = "slow: runs `cargo test` in a temp dir; opt in with `-- --ignored`"]
fn hot_swap_classifier_aggregates_all_fields_correctly() {
    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-fieldtag-agg-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    hot_swap_classifier_spec()
        .compile_to_crate("hot-swap-classifier-derive")
        .unwrap()
        .write_to(&tmp.join("hot-swap-classifier-derive"))
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
hot-swap-classifier-derive = { path = "../hot-swap-classifier-derive" }

[lib]
path = "src/lib.rs"
"#,
    )
    .unwrap();

    std::fs::write(
        cons.join("src/lib.rs"),
        r#"use hot_swap_classifier_derive::HotSwapClassifier;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotSwapClass {
    Free,
    RequiresRestart { reason: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwapDecision {
    Free,
    RequiresRestart(Vec<&'static str>),
}

pub trait HotSwapClassifier {
    const FIELD_CLASSES: &'static [(&'static str, HotSwapClass)];
    fn classify_change(&self, new: &Self) -> SwapDecision;
}

#[derive(HotSwapClassifier, Debug, Clone, PartialEq, Eq)]
pub struct Config {
    #[hot_swap]
    pub log_level: String,
    #[restart_required(reason = "bound at process start")]
    pub bind_addr: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Config {
        Config { log_level: "info".into(), bind_addr: "0.0.0.0:8080".into() }
    }

    #[test]
    fn field_classes_reports_both() {
        assert_eq!(Config::FIELD_CLASSES.len(), 2);
        assert_eq!(Config::FIELD_CLASSES[0], ("log_level", HotSwapClass::Free));
        assert_eq!(
            Config::FIELD_CLASSES[1],
            ("bind_addr", HotSwapClass::RequiresRestart { reason: "bound at process start" })
        );
    }

    #[test]
    fn no_change_is_free() {
        let a = base();
        let b = base();
        assert_eq!(a.classify_change(&b), SwapDecision::Free);
    }

    #[test]
    fn only_free_field_changed_is_still_free() {
        let a = base();
        let mut b = base();
        b.log_level = "debug".into();
        assert_eq!(a.classify_change(&b), SwapDecision::Free);
    }

    #[test]
    fn only_restart_field_changed_requires_restart_with_reason() {
        let a = base();
        let mut b = base();
        b.bind_addr = "0.0.0.0:9090".into();
        assert_eq!(
            a.classify_change(&b),
            SwapDecision::RequiresRestart(vec!["bound at process start"])
        );
    }

    #[test]
    fn both_fields_changed_requires_restart() {
        let a = base();
        let mut b = base();
        b.log_level = "debug".into();
        b.bind_addr = "0.0.0.0:9090".into();
        assert_eq!(
            a.classify_change(&b),
            SwapDecision::RequiresRestart(vec!["bound at process start"])
        );
    }
}
"#,
    )
    .unwrap();

    let status = Command::new("cargo")
        .arg("test")
        .current_dir(&cons)
        .status()
        .expect("spawn cargo test");

    assert!(
        status.success(),
        "hot-swap-classifier consumer failed cargo test at {}",
        cons.display()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
