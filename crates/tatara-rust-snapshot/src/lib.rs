//! `tatara-rust-snapshot` — golden-file + token-stable test surface
//! for emitter output.
//!
//! **★★ TOKEN-STABILITY directive.** Every assertion against emitter
//! output in the pleme-io substrate uses one of two primitives from
//! this crate:
//!
//! - [`snapshot_spec!`] — golden insta snapshot of every file the
//!   Spec emits. Drift surfaces as a visible diff; review with
//!   `cargo insta review`. Use for full-file contracts.
//!
//! - [`assert_tokens_contain!`] / [`assert_tokens_eq!`] — typed
//!   `TokenStream::to_string()` comparison after parsing both sides.
//!   Whitespace, comment positions, prettyplease spacing choices —
//!   invisible. Use for targeted fragment assertions.
//!
//! **Banned:** raw `s.contains("pub fn #field_name")` on emitter
//! output. The bandaid pattern of stripping prettyplease spacing in
//! tests is the symptom of asserting on characters when the contract
//! is at the token level. Use the primitives above instead.

pub use insta;
pub use proc_macro2;

pub mod tokens;

/// `src/lib.rs` → `src__lib_rs`. Keeps snapshot file names filesystem-safe.
#[doc(hidden)]
#[must_use]
pub fn flatten_path(p: &str) -> String {
    p.replace(['/', '\\', '.'], "_")
}

/// Render `$spec` as the named crate, then `insta::assert_snapshot!`
/// every emitted file. Snapshot names are `<stem>__<flattened-path>`.
///
/// Panics if compile-to-crate fails — same fail-loud contract as
/// `insta::assert_snapshot!`.
#[macro_export]
macro_rules! snapshot_spec {
    ($spec:expr, $crate_name:expr, $stem:expr $(,)?) => {{
        use $crate::insta as __insta;
        let __sc = $crate::__compile($spec, $crate_name);
        for __f in &__sc.files {
            let __snap = format!("{}__{}", $stem, $crate::flatten_path(&__f.path));
            __insta::assert_snapshot!(__snap, &__f.contents);
        }
    }};
}

/// Internal helper for the `snapshot_spec!` macro. Hidden from docs.
#[doc(hidden)]
#[must_use]
pub fn __compile<T: tatara_rust_ast::CompileToCrate>(
    spec: &T,
    crate_name: &str,
) -> tatara_rust_ast::CrateScaffold {
    spec.compile_to_crate(crate_name)
        .expect("snapshot_spec!: compile_to_crate failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatten_keeps_segments() {
        assert_eq!(flatten_path("Cargo.toml"), "Cargo_toml");
        assert_eq!(flatten_path("src/lib.rs"), "src_lib_rs");
        assert_eq!(flatten_path(".github/workflows/x.yml"), "_github_workflows_x_yml");
    }
}
