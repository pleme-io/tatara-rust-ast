//! `VerificationMatrixSpec` — the farm's **first test-generation** primitive.
//!
//! Every emitter in the farm so far produces an *impl* derive (getter,
//! setter, isvariant, newtype From, enum-fold ALL/COUNT/NAMES, paired
//! round-trip). This is the first Spec whose emitted crate ships
//! *declarative test macros* rather than an impl proc-macro.
//!
//! It materializes the ★★ CLOSED-LOOP MASS-SYNTHESIS "verification
//! matrix as forcing function" rule (org CLAUDE.md, rule 1) as a
//! **consumable** primitive: a normal library crate (NOT a proc-macro,
//! NOT an extra-dependency crate — peer of `tatara-rust-macro-rules`)
//! exposing two `#[macro_export] macro_rules!` macros:
//!
//! 1. `verification_matrix!` — expands a typed `const ROWS: &[Row]`
//!    table + a per-row exercise closure into ONE aggregate `#[test]`
//!    that runs every row and **aggregates all failures before
//!    asserting** (so one matrix run reports every broken row, not just
//!    the first).
//! 2. `matrix_covers_all!` — expands into a `#[test]` build-gate that
//!    asserts `ROWS.len() == <count>` — typically an enum's
//!    `pleme-variantcount-derive` `COUNT`. A new variant landing without
//!    a matrix row trips this test (the forcing function).
//!
//! Composing the two with `pleme-variantcount-derive` is exactly the
//! `repo_emission_matrix.rs` shape (one row per CatalogSpec variant +
//! aggregate-before-assert + `matrix_covers_every_catalog_spec_variant`)
//! made reusable. Survey signal (pleme-io 2026-05-31, mado): the five VT
//! matrices (`vt_case!` / `sgr_matrix!` / `dec_mode_matrix!` /
//! `osc_matrix!` / `cursor_style_matrix!`) and the caps honesty matrix
//! all reduce to this one shape; mado is the third+ consumer after
//! pleme-doc-gen's 37-row matrix and caixa-forge.
//!
//! **Dependency-free by contract.** The emitted macros reference only
//! `::std` paths and the *consumer's* own row table + closure — no
//! `paste`, no proc-macro plumbing. The covers-all gate takes an
//! explicit test-fn name (rather than concatenating `<name>_covers_all`)
//! precisely so the crate stays a plain `[lib]` with zero deps, matching
//! `MacroRulesSpec`'s contract.
//!
//! Authoring shape:
//!
//! ```ignore
//! // canonical fleet macro names:
//! VerificationMatrixSpec::canonical()
//! // custom macro names:
//! VerificationMatrixSpec { matrix_macro: "vt_matrix".into(),
//!     covers_macro: "vt_covers_all".into() }
//! ```
//!
//! Consumer (after `#[derive(VariantCount)]` on the enum the matrix covers):
//!
//! ```ignore
//! const ROWS: &[Row] = &[ /* one row per variant */ ];
//!
//! verification_matrix! {
//!     name = every_row_holds,
//!     rows = ROWS,
//!     run  = |r: &Row| -> Result<(), String> { /* exercise + return Err on fail */ },
//! }
//! matrix_covers_all! {
//!     name   = rows_cover_all_variants,
//!     rows   = ROWS,
//!     covers = MyEnum::COUNT,
//! }
//! ```

use serde::{Deserialize, Serialize};
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold};

/// Typed spec for the two declarative verification-matrix test macros.
///
/// Both macro identifiers are parameters so a consumer can ship multiple
/// domain-named matrix surfaces (e.g. `sgr_matrix!`) without import
/// collisions. Default names are the fleet-canonical
/// `verification_matrix!` / `matrix_covers_all!`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationMatrixSpec {
    /// Name of the per-row aggregate-test macro (`verification_matrix`).
    pub matrix_macro: String,
    /// Name of the covers-all build-gate macro (`matrix_covers_all`).
    pub covers_macro: String,
}

impl Default for VerificationMatrixSpec {
    fn default() -> Self {
        Self::canonical()
    }
}

impl VerificationMatrixSpec {
    /// Fleet-canonical spec: `verification_matrix!` + `matrix_covers_all!`.
    #[must_use]
    pub fn canonical() -> Self {
        Self {
            matrix_macro: "verification_matrix".into(),
            covers_macro: "matrix_covers_all".into(),
        }
    }

    /// User-facing identifier reported to the verifier — the matrix
    /// macro is the headline surface.
    #[must_use]
    pub fn primary_name(&self) -> &str {
        &self.matrix_macro
    }
}

impl CompileToCrate for VerificationMatrixSpec {
    fn compile_to_crate(&self, crate_name: &str) -> Result<CrateScaffold, AstError> {
        let mut s = CrateScaffold::new(crate_name, "0.1.0");
        s.add_file("Cargo.toml", render_cargo_toml(crate_name));
        s.add_file("src/lib.rs", render_lib_rs(self));
        Ok(s)
    }
}

fn render_cargo_toml(crate_name: &str) -> String {
    // Declarative-macro crate — a NORMAL [lib], not a proc-macro, and no
    // dependencies. Mirrors `tatara-rust-macro-rules::render_cargo_toml`.
    format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2024"
license = "MIT"
description = "Verification-matrix test macros emitted from a tatara-rust-derive VerificationMatrixSpec (CLOSED-LOOP MASS-SYNTHESIS rule 1)."

[lib]
"#
    )
}

/// The macro crate's `lib.rs` as a sentinel-bearing template. The two
/// `__MATRIX_MACRO__` / `__COVERS_MACRO__` sentinels carry the macro
/// identifiers; substitution is by `String::replace` (the
/// `kind_round_trip.rs` idiom) and the result is validated + formatted
/// through `syn::parse_str` + `prettyplease` per ★★ TYPED EMISSION.
///
/// Every `$…` token is a `macro_rules!` metavariable that must survive
/// verbatim into the emitted crate — it is plain text here, never
/// interpolated by the emitter.
const LIB_RS_TEMPLATE: &str = r####"
#![doc = "GENERATED by tatara-rust-derive::VerificationMatrixSpec."]
#![doc = "Do not hand-edit; regenerate from the (defverification-matrix …) source."]
#![doc = ""]
#![doc = "Two declarative test macros realizing the CLOSED-LOOP"]
#![doc = "MASS-SYNTHESIS verification-matrix-as-forcing-function rule."]

/// Emit ONE aggregate `#[test] fn $name` that runs `$run` against every
/// row of `$rows`, collecting EVERY failure before asserting — so one
/// run reports all broken rows, not just the first.
///
/// `$run` is any `Fn(&Row) -> Result<(), E>` where `E: Display` and
/// `Row: Debug`. The row table is anything with `.iter()` + `.len()`
/// (`&[Row]`, `Vec<Row>`, an array).
#[macro_export]
macro_rules! __MATRIX_MACRO__ {
    (
        $(#[$meta:meta])*
        name = $name:ident,
        rows = $rows:expr,
        run  = $run:expr $(,)?
    ) => {
        $(#[$meta])*
        #[test]
        fn $name() {
            let __vm_run = $run;
            let __vm_rows = $rows;
            let mut __vm_failures: ::std::vec::Vec<::std::string::String> =
                ::std::vec::Vec::new();
            for __vm_row in __vm_rows.iter() {
                if let ::std::result::Result::Err(__vm_e) = __vm_run(__vm_row) {
                    __vm_failures.push(::std::format!("{:?}: {}", __vm_row, __vm_e));
                }
            }
            ::std::assert!(
                __vm_failures.is_empty(),
                "{}/{} matrix rows failed:\n  - {}",
                __vm_failures.len(),
                __vm_rows.len(),
                __vm_failures.join("\n  - "),
            );
        }
    };
}

/// Emit a `#[test] fn $name` build-gate asserting the row table covers
/// exactly `$count` cases — typically an enum's `pleme-variantcount-derive`
/// `COUNT`. A new variant (or dispatch arm) landing without a matrix row
/// trips this test: the forcing function.
#[macro_export]
macro_rules! __COVERS_MACRO__ {
    (
        $(#[$meta:meta])*
        name   = $name:ident,
        rows   = $rows:expr,
        covers = $count:expr $(,)?
    ) => {
        $(#[$meta])*
        #[test]
        fn $name() {
            let __vm_rows = $rows;
            ::std::assert_eq!(
                __vm_rows.len(),
                $count,
                "matrix `{}` must cover all {} cases; got {} rows (a new variant/arm landed without a matrix row)",
                ::std::stringify!($name),
                $count,
                __vm_rows.len(),
            );
        }
    };
}
"####;

/// Render lib.rs by substituting the two macro identifiers into
/// [`LIB_RS_TEMPLATE`], then validating + formatting through
/// `syn::parse_str` + `prettyplease`.
fn render_lib_rs(spec: &VerificationMatrixSpec) -> String {
    let src = LIB_RS_TEMPLATE
        .replace("__MATRIX_MACRO__", &spec.matrix_macro)
        .replace("__COVERS_MACRO__", &spec.covers_macro);

    let parsed: syn::File =
        syn::parse_str(&src).expect("emitted lib.rs must parse as syn::File");
    prettyplease::unparse(&parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_snapshot::assert_tokens_contain;

    #[test]
    fn canonical_uses_fleet_default_macro_names() {
        let s = VerificationMatrixSpec::canonical();
        assert_eq!(s.matrix_macro, "verification_matrix");
        assert_eq!(s.covers_macro, "matrix_covers_all");
        assert_eq!(VerificationMatrixSpec::default(), s);
    }

    #[test]
    fn compiles_to_cargo_and_lib() {
        let s = VerificationMatrixSpec::canonical()
            .compile_to_crate("pleme-verification-matrix")
            .unwrap();
        let files = s.to_files();
        assert!(files.contains_key("Cargo.toml"));
        assert!(files.contains_key("src/lib.rs"));
    }

    #[test]
    fn cargo_toml_is_normal_lib_not_proc_macro_no_deps() {
        let s = VerificationMatrixSpec::canonical()
            .compile_to_crate("pleme-verification-matrix")
            .unwrap();
        let toml = s.to_files().get("Cargo.toml").unwrap().clone();
        // Declarative macros ship in a NORMAL lib — never proc-macro.
        assert!(!toml.contains("proc-macro"));
        assert!(toml.contains("[lib]"));
        // No [dependencies] section — the emitted macros reference only ::std.
        assert!(!toml.contains("[dependencies]"));
    }

    #[test]
    fn emits_both_macros_with_export_attr() {
        let s = VerificationMatrixSpec::canonical()
            .compile_to_crate("v")
            .unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // Multi-token positives go through token-subsequence matching per
        // the ★★ TOKEN-STABILITY directive (prettyplease line-wraps the
        // `#[macro_export] macro_rules! name` head).
        assert_tokens_contain!(&lib, "#[macro_export]");
        assert_tokens_contain!(&lib, "macro_rules! verification_matrix");
        assert_tokens_contain!(&lib, "macro_rules! matrix_covers_all");
    }

    #[test]
    fn aggregates_failures_before_assert() {
        let s = VerificationMatrixSpec::canonical()
            .compile_to_crate("v")
            .unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // The aggregate test must collect into a failures Vec and join
        // before asserting (CLOSED-LOOP rule 1: one run reports all).
        assert!(lib.contains("__vm_failures"), "failure-accumulator missing");
        assert!(lib.contains("matrix rows failed"), "aggregate message missing");
        assert!(lib.contains("__vm_failures . is_empty") || lib.contains("__vm_failures.is_empty"));
    }

    #[test]
    fn covers_macro_asserts_row_count_equals_covers() {
        let s = VerificationMatrixSpec::canonical()
            .compile_to_crate("v")
            .unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // The forcing-function message text lives in one string literal.
        assert!(
            lib.contains("must cover all"),
            "covers-all forcing-function message missing"
        );
        assert!(
            lib.contains("without a matrix row"),
            "drift-explanation message missing"
        );
    }

    #[test]
    fn custom_macro_names_substitute() {
        let spec = VerificationMatrixSpec {
            matrix_macro: "vt_matrix".into(),
            covers_macro: "vt_covers".into(),
        };
        let lib = spec
            .compile_to_crate("v")
            .unwrap()
            .to_files()
            .get("src/lib.rs")
            .unwrap()
            .clone();
        assert_tokens_contain!(&lib, "macro_rules! vt_matrix");
        assert_tokens_contain!(&lib, "macro_rules! vt_covers");
        // Default names must NOT leak when overridden.
        assert!(!lib.contains("macro_rules ! verification_matrix"));
        assert!(!lib.contains("macro_rules! verification_matrix"));
    }

    #[test]
    fn no_sentinel_leaks() {
        let lib = VerificationMatrixSpec::canonical()
            .compile_to_crate("v")
            .unwrap()
            .to_files()
            .get("src/lib.rs")
            .unwrap()
            .clone();
        assert!(!lib.contains("__MATRIX_MACRO__"), "matrix sentinel leaked");
        assert!(!lib.contains("__COVERS_MACRO__"), "covers sentinel leaked");
    }

    #[test]
    fn serde_round_trip() {
        let spec = VerificationMatrixSpec {
            matrix_macro: "sgr_matrix".into(),
            covers_macro: "sgr_covers_all".into(),
        };
        let j = serde_json::to_string(&spec).unwrap();
        let back: VerificationMatrixSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(spec, back);
    }
}
