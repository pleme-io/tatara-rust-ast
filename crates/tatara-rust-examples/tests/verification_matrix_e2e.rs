//! End-to-end worked example for `pleme-verification-matrix` (F3).
//!
//! Proves the emitted declarative-macro crate is a real, consumable
//! test-generation primitive — NOT just well-formed text. Strategy:
//! emit the macro crate's `src/lib.rs`, persist it as a committed
//! `verification_matrix_emitted.rs`, and `include!` it inside a `mod`
//! so the two `#[macro_export]` macros land crate-globally (the emitted
//! crate is a NORMAL lib, so its `macro_rules!` source includes cleanly
//! — no proc-macro plumbing). Then drive a 3-row matrix through both
//! macros, AND prove the covers-all gate FIRES on a wrong count — the
//! forcing function the CLOSED-LOOP MASS-SYNTHESIS rule promises.

use tatara_rust_ast::CompileToCrate;
use tatara_rust_examples::verification_matrix_spec;

/// The emitted macro bodies, `include!`d at file scope so the two
/// `#[macro_export]` macros land crate-globally (the emitted crate is a
/// NORMAL lib, so its `macro_rules!` source includes cleanly — no
/// proc-macro plumbing). The committed file is the emission with its
/// leading crate-inner `#![doc]` header stripped (see [`macro_body`] +
/// the determinism test) because crate-inner attributes are illegal
/// inside an `include!`d fragment.
include!("verification_matrix_emitted.rs");

/// Drop the leading crate-inner doc header (`#![doc=...]` /
/// prettyplease-rendered `//!` lines) from emitted lib.rs so the
/// remainder is `include!`able at non-crate-root scope. Keeps the macro
/// bodies verbatim — the determinism check compares this transform of a
/// fresh emission against the committed file.
fn macro_body(emitted_lib_rs: &str) -> String {
    emitted_lib_rs
        .lines()
        .skip_while(|l| {
            let t = l.trim_start();
            t.is_empty() || t.starts_with("//!") || t.starts_with("#![")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug)]
#[allow(dead_code)] // `name` is read only via the derived Debug in failure messages.
struct Row {
    name: &'static str,
    input: u32,
    doubled: u32,
}

#[allow(dead_code)]
const ROWS: &[Row] = &[
    Row { name: "one", input: 1, doubled: 2 },
    Row { name: "two", input: 2, doubled: 4 },
    Row { name: "three", input: 3, doubled: 6 },
];

verification_matrix! {
    name = every_row_doubles,
    rows = ROWS,
    run  = |r: &Row| -> Result<(), String> {
        let got = r.input * 2;
        if got == r.doubled { Ok(()) } else { Err(format!("{got} != {}", r.doubled)) }
    },
}

matrix_covers_all! {
    name   = rows_cover_all,
    rows   = ROWS,
    covers = 3usize,
}

// Forcing-function proof: a wrong covers count → the gate FAILS.
matrix_covers_all! {
    #[should_panic(expected = "must cover all 4 cases; got 3 rows")]
    name   = covers_all_detects_drift,
    rows   = ROWS,
    covers = 4usize,
}

// Forcing-function proof: a bad row → the aggregate test FAILS, and the
// message names the failing row (aggregation-before-assert).
#[allow(dead_code)]
const BAD_ROWS: &[Row] = &[Row { name: "bad", input: 5, doubled: 999 }];
verification_matrix! {
    #[should_panic(expected = "matrix rows failed")]
    name = bad_row_trips_matrix,
    rows = BAD_ROWS,
    run  = |r: &Row| -> Result<(), String> {
        if r.input * 2 == r.doubled { Ok(()) } else { Err("mismatch".into()) }
    },
}

#[test]
fn committed_emitted_macros_match_fresh_emission() {
    // Spec-alongside-artifact (CLOSED-LOOP rule 3): the committed
    // `verification_matrix_emitted.rs` that this test `include!`s must be
    // byte-identical to a fresh emission of the spec. Re-render drift is
    // a test failure, proving the emitter is deterministic.
    let scaffold = verification_matrix_spec()
        .compile_to_crate("pleme-verification-matrix")
        .expect("verification-matrix spec must compile to a crate");
    let fresh_lib = scaffold
        .to_files()
        .get("src/lib.rs")
        .expect("emitted crate must have src/lib.rs")
        .clone();
    let fresh_body = macro_body(&fresh_lib);
    let committed = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("verification_matrix_emitted.rs"),
    )
    .unwrap();
    assert_eq!(
        committed.trim_end(),
        fresh_body.trim_end(),
        "committed verification_matrix_emitted.rs drifted from a fresh emission; \
         regenerate it via macro_body(VerificationMatrixSpec::canonical() emit)"
    );
}
