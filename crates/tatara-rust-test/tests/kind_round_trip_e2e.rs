//! End-to-end proof for `KindRoundTripSpec` (F1 — the paired round-trip
//! derive). Each test materializes the emitted derive crate + a consumer
//! enum that uses `#[kind(...)]` helper attrs, compiles it, and asserts
//! the **inverse-table property** holds at runtime:
//!
//!   for every variant v:  from_str_kind(v.as_str()) == Some(v)
//!   for every variant v:  from_byte(v.as_byte())    == Some(v)   (byte mode)
//!
//! This is the construction proof that distinguishes "the emitted source
//! parses" (the unit tests) from "the derive actually works when applied."

use tatara_rust_derive::KindRoundTripSpec;
use tatara_rust_test::{DeriveExamplePackSpec, Example};

#[test]
#[ignore = "slow: runs `cargo test` in a temp dir; opt in with `-- --ignored`"]
fn kind_str_round_trip_via_example_pack() {
    let spec = KindRoundTripSpec::kind_str("KindStr");

    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-kindstr-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);

    let pack = DeriveExamplePackSpec {
        derive_crate_name: "kindstr-derive".into(),
        trait_name: "KindStr".into(),
        spec: &spec,
        extra_consumer_imports: vec![],
        auxiliary_trait_crates: vec![],
        examples: vec![Example {
            name: "prompt-kind".into(),
            consumer_item: r#"#[derive(Debug, PartialEq, Clone, Copy)]
pub enum PromptKind {
    #[kind(name = "ps1")]
    Primary,
    #[kind(name = "ps2", alias = "cont")]
    Continuation,
    Right,
}"#
            .into(),
            assertion_body: r#"
        // as_str honors name override + bare-ident default.
        assert_eq!(PromptKind::Primary.as_str(), "ps1");
        assert_eq!(PromptKind::Continuation.as_str(), "ps2");
        assert_eq!(PromptKind::Right.as_str(), "Right");
        // from_str_kind honors name, alias, default, and rejects unknown.
        assert_eq!(PromptKind::from_str_kind("ps2"), Some(PromptKind::Continuation));
        assert_eq!(PromptKind::from_str_kind("cont"), Some(PromptKind::Continuation));
        assert_eq!(PromptKind::from_str_kind("Right"), Some(PromptKind::Right));
        assert_eq!(PromptKind::from_str_kind("nope"), None);
        // inverse-table property holds for every variant.
        for v in [PromptKind::Primary, PromptKind::Continuation, PromptKind::Right] {
            assert_eq!(PromptKind::from_str_kind(v.as_str()), Some(v));
        }"#
            .into(),
        }],
    };

    let report = pack.run_under(&tmp).unwrap();
    assert!(report.cargo_test_succeeded, "KindStr consumer cargo test failed");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
#[ignore = "slow: runs `cargo test` in a temp dir; opt in with `-- --ignored`"]
fn kind_byte_round_trip_via_example_pack() {
    let spec = KindRoundTripSpec::kind_byte("ClipKind");

    let tmp = std::env::temp_dir().join(format!(
        "tatara-rust-kindbyte-e2e-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);

    let pack = DeriveExamplePackSpec {
        derive_crate_name: "clipkind-derive".into(),
        trait_name: "ClipKind".into(),
        spec: &spec,
        extra_consumer_imports: vec![],
        auxiliary_trait_crates: vec![],
        examples: vec![Example {
            name: "clip-kind".into(),
            consumer_item: r#"#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ClipKind {
    #[kind(name = "clipboard", byte = 99)]
    Clipboard,
    #[kind(name = "primary", alias = "sel", byte = 112)]
    Primary,
}"#
            .into(),
            assertion_body: r#"
        assert_eq!(ClipKind::Clipboard.as_str(), "clipboard");
        assert_eq!(ClipKind::Clipboard.as_byte(), 99u8);
        assert_eq!(ClipKind::from_str_kind("sel"), Some(ClipKind::Primary));
        assert_eq!(ClipKind::from_byte(112), Some(ClipKind::Primary));
        assert_eq!(ClipKind::from_byte(0), None);
        // both inverse-table properties hold for every variant.
        for v in [ClipKind::Clipboard, ClipKind::Primary] {
            assert_eq!(ClipKind::from_str_kind(v.as_str()), Some(v));
            assert_eq!(ClipKind::from_byte(v.as_byte()), Some(v));
        }"#
            .into(),
        }],
    };

    let report = pack.run_under(&tmp).unwrap();
    assert!(report.cargo_test_succeeded, "ClipKind consumer cargo test failed");
    let _ = std::fs::remove_dir_all(&tmp);
}
