//! `tatara-rust-survey` — adoption multiplier for the macro farm.
//!
//! Walks Rust source via `syn::parse_file`, visits every `impl` block,
//! and classifies each method body against the catalog of published
//! pleme-io derives. Reports a typed [`RefactorCandidate`] punch list
//! the operator can act on — either by hand or via a future
//! `survey-apply` mode that uses `syn::visit_mut` to rewrite the file.
//!
//! This is the **discovery** half of the structured adoption pipeline:
//!
//! ```text
//! survey  →  apply  →  validate
//!   │         │          │
//!   │         │          └── cargo test on the modified crate;
//!   │         │              landed only when green
//!   │         └── syn::visit_mut transform; produces a diff
//!   └── identify candidates; produce typed RefactorCandidate values
//! ```
//!
//! Today we ship the four densest patterns from the 2026-05-28 fleet
//! survey: **GetterAll**, **SetterAll**, **WithBuilder**, **IsVariant**.
//! Each one is a detector that inspects a `syn::ImplItem::Fn` and
//! returns `Some(MatchedPattern)` if the body matches the canonical
//! derive shape exactly. New detectors plug in as one more enum
//! variant + one more pattern-matching function.

use proc_macro2::Span;
use serde::Serialize;
use std::path::{Path, PathBuf};
use syn::visit::Visit;

pub mod apply;
pub mod cargo_deps;
pub mod detector;
pub mod fleet;
pub mod pipeline;
pub use apply::{apply_to_source, ApplyError};
pub use cargo_deps::{inject_deps, CargoDepsError, DepSource, InjectOutcome};
pub use detector::{detectors, Detector};
pub use fleet::{
    survey_fleet, survey_fleet_apply, CrateSurveyEntry, FleetApplyEntry, FleetApplyOpts,
    FleetApplyReport, FleetSurveyReport,
};
pub use pipeline::{
    apply_all_to_source, survey_apply_validate, FileOutcome, PipelineError, PipelineOpts,
    PipelineOutcome,
};

/// One opportunity to replace a hand-written impl with a farm derive.
/// The `current_impl` and `loc_saved` fields let `apply` produce a
/// precise diff; the `derive_crate` field tells the operator which
/// git dep to add.
#[derive(Clone, Debug, Serialize)]
pub struct RefactorCandidate {
    pub file: PathBuf,
    /// 1-based line of the impl block's `impl` keyword.
    pub line: usize,
    /// The pleme-io published derive crate the operator would consume.
    pub derive_crate: &'static str,
    /// The trait identifier the user would write inside `#[derive(…)]`.
    pub derive_trait: &'static str,
    /// Which farm pattern this site matches.
    pub pattern: MatchedPattern,
    /// The struct/enum name the impl block targets.
    pub target_type: String,
    /// How many lines of impl-block boilerplate adopting this derive
    /// would delete (approximate — counts the function bodies).
    pub estimated_loc_saved: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchedPattern {
    /// `pub fn <field>(&self) -> &<T> { &self.<field> }` per field.
    /// Adopt: `pleme-getter-derive` → `#[derive(GetterAll)]`.
    GetterAll,
    /// `pub fn set_<field>(&mut self, v: <T>) { self.<field> = v; }` per field.
    /// Adopt: `pleme-setter-derive` → `#[derive(SetterAll)]`.
    SetterAll,
    /// `pub fn with_<field>(mut self, v: <T>) -> Self { self.<field> = v; self }` per field.
    /// Adopt: `pleme-builder-derive` → `#[derive(WithBuilder)]`.
    WithBuilder,
    /// `pub fn is_<variant>(&self) -> bool { matches!(self, Self::<variant>(..) | Self::<variant> { .. } | Self::<variant>) }` per enum variant.
    /// Adopt: `pleme-isvariant-derive` → `#[derive(IsVariant)]`.
    IsVariant,
}

impl MatchedPattern {
    /// Derive crate name (kebab-case) for this pattern. Routed
    /// through the [`detector`] registry so MatchedPattern stays a
    /// pure identifier — the per-pattern data lives on the detector
    /// that owns it. Panics if a pattern variant has no detector
    /// (registry-invariant: every variant MUST have one).
    pub fn derive_crate(self) -> &'static str {
        detector::detectors()
            .iter()
            .find(|d| d.pattern() == self)
            .map(|d| d.derive_crate())
            .expect("every MatchedPattern variant has a Detector in the registry")
    }
    /// Trait identifier (PascalCase) for this pattern. Same routing
    /// as [`Self::derive_crate`].
    pub fn derive_trait(self) -> &'static str {
        detector::detectors()
            .iter()
            .find(|d| d.pattern() == self)
            .map(|d| d.derive_trait())
            .expect("every MatchedPattern variant has a Detector in the registry")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SurveyError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("syn parse failed for {path}: {err}")]
    Parse {
        path: PathBuf,
        err: syn::Error,
    },
}

/// Walk a single `.rs` file and return every refactor candidate found.
pub fn survey_file(path: &Path) -> Result<Vec<RefactorCandidate>, SurveyError> {
    let src = std::fs::read_to_string(path)?;
    let file: syn::File = syn::parse_file(&src).map_err(|err| SurveyError::Parse {
        path: path.to_path_buf(),
        err,
    })?;
    let mut v = SurveyVisitor::new(path.to_path_buf());
    v.visit_file(&file);
    Ok(v.candidates)
}

/// Walk every `.rs` file under `root` recursively. Skips `target/`,
/// `.git/`, and hidden directories.
pub fn survey_tree(root: &Path) -> Result<Vec<RefactorCandidate>, SurveyError> {
    let mut out = vec![];
    visit_rs_files(root, &mut |p| {
        match survey_file(p) {
            Ok(mut cands) => out.append(&mut cands),
            Err(SurveyError::Parse { .. }) => {
                // Parse errors on individual files (e.g. malformed
                // generated code) shouldn't abort the whole survey.
            }
            Err(other) => return Err(other),
        }
        Ok(())
    })?;
    Ok(out)
}

fn visit_rs_files(
    root: &Path,
    f: &mut dyn FnMut(&Path) -> Result<(), SurveyError>,
) -> Result<(), SurveyError> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            visit_rs_files(&path, f)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            f(&path)?;
        }
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
// SurveyVisitor — syn::Visit walker
// ─────────────────────────────────────────────────────────────────────

struct SurveyVisitor {
    file: PathBuf,
    candidates: Vec<RefactorCandidate>,
}

impl SurveyVisitor {
    fn new(file: PathBuf) -> Self {
        Self { file, candidates: vec![] }
    }
}

impl<'ast> Visit<'ast> for SurveyVisitor {
    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        // Only inherent impls (no `for Trait`) are candidates for
        // GetterAll / SetterAll / WithBuilder / IsVariant — those
        // are typed-fleet derives that emit inherent impls.
        if i.trait_.is_some() {
            return;
        }

        // Extract the BASE identifier — for `impl<T> Foo<T> { ... }`
        // we want `target_type == "Foo"` so it matches `Item::Struct`
        // whose `ident` is the unadorned name. `type_to_string` would
        // produce `"Foo < T >"` here (token spaces) and break the
        // downstream apply lookup.
        let Some(target_type) = type_base_ident(&i.self_ty) else {
            return; // weird shape (Trait object, tuple, etc.) — skip
        };
        // proc-macro2's Span::start() is gated behind the "span-locations"
        // feature; under stable rustc without that feature we just
        // report line 1. Operators get the file path either way.
        let line = 1;

        // Walk the impl items and classify each `fn`. To get a
        // **per-field** match (GetterAll vs SetterAll vs WithBuilder),
        // we count how many fns in this impl match the pattern;
        // require ≥2 to call it a "candidate worth reporting" — a
        // single getter could be hand-rolled with reason.
        let mut counts: std::collections::HashMap<MatchedPattern, usize> =
            std::collections::HashMap::new();
        for item in &i.items {
            if let syn::ImplItem::Fn(f) = item {
                if let Some(pat) = classify_fn(f) {
                    *counts.entry(pat).or_default() += 1;
                }
            }
        }

        for (pat, count) in counts {
            if count >= 2 {
                self.candidates.push(RefactorCandidate {
                    file: self.file.clone(),
                    line,
                    derive_crate: pat.derive_crate(),
                    derive_trait: pat.derive_trait(),
                    pattern: pat,
                    target_type: target_type.clone(),
                    // Rough estimate: 5 LOC per impl fn (open + body + close).
                    estimated_loc_saved: count * 5,
                });
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Per-fn classifiers
// ─────────────────────────────────────────────────────────────────────

/// Classify a single `impl` method against the farm derive shapes.
/// Returns `None` if the fn doesn't match any [`Detector`] in the
/// registry.
///
/// First-match-wins: the registry is ordered, but the canonical four
/// detectors are disjoint by fn-name prefix + body shape, so order
/// doesn't currently matter. Future detectors must preserve that
/// invariant or land registry-order tests asserting precedence.
pub(crate) fn classify_fn(f: &syn::ImplItemFn) -> Option<MatchedPattern> {
    detector::detectors()
        .iter()
        .find(|d| d.matches(f))
        .map(|d| d.pattern())
}

pub(crate) fn type_to_string(t: &syn::Type) -> String {
    use quote::ToTokens;
    let _ = Span::call_site();
    let mut buf = proc_macro2::TokenStream::new();
    t.to_tokens(&mut buf);
    buf.to_string()
}

/// Return the FINAL path segment's identifier as a String — strips
/// generic arguments + module path. For `Foo<T>` returns `"Foo"`;
/// for `a::b::Foo<T, U>` returns `"Foo"`; for `&[u8]`, tuples, trait
/// objects, etc., returns `None`. This is what `Item::Struct.ident`
/// compares against — the bare struct name, not the token-rendered
/// generic spelling.
pub(crate) fn type_base_ident(t: &syn::Type) -> Option<String> {
    let syn::Type::Path(tp) = t else {
        return None;
    };
    tp.path.segments.last().map(|seg| seg.ident.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_file(name: &str, body: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "tatara-survey-{}-{}",
            name,
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("lib.rs");
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn detects_getter_all_pattern() {
        let path = tmp_file(
            "getter",
            r#"
pub struct Foo { pub a: i32, pub b: String }

impl Foo {
    pub fn a(&self) -> &i32 { &self.a }
    pub fn b(&self) -> &String { &self.b }
}
"#,
        );
        let cands = survey_file(&path).unwrap();
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].pattern, MatchedPattern::GetterAll);
        assert_eq!(cands[0].derive_crate, "pleme-getter-derive");
        assert_eq!(cands[0].target_type, "Foo");
    }

    #[test]
    fn detects_setter_all_pattern() {
        let path = tmp_file(
            "setter",
            r#"
pub struct Foo { pub a: i32, pub b: String }

impl Foo {
    pub fn set_a(&mut self, v: i32) { self.a = v; }
    pub fn set_b(&mut self, v: String) { self.b = v; }
}
"#,
        );
        let cands = survey_file(&path).unwrap();
        assert!(cands.iter().any(|c| c.pattern == MatchedPattern::SetterAll));
    }

    #[test]
    fn detects_with_builder_pattern() {
        let path = tmp_file(
            "builder",
            r#"
pub struct Foo { pub a: i32, pub b: String }

impl Foo {
    pub fn with_a(mut self, v: i32) -> Self { self.a = v; self }
    pub fn with_b(mut self, v: String) -> Self { self.b = v; self }
}
"#,
        );
        let cands = survey_file(&path).unwrap();
        assert!(cands.iter().any(|c| c.pattern == MatchedPattern::WithBuilder));
    }

    #[test]
    fn detects_isvariant_pattern() {
        let path = tmp_file(
            "isvariant",
            r#"
pub enum State { A, B(i32), C { x: u8 } }

impl State {
    pub fn is_a(&self) -> bool { matches!(self, Self::A) }
    pub fn is_b(&self) -> bool { matches!(self, Self::B(_)) }
    pub fn is_c(&self) -> bool { matches!(self, Self::C { .. }) }
}
"#,
        );
        let cands = survey_file(&path).unwrap();
        assert!(cands.iter().any(|c| c.pattern == MatchedPattern::IsVariant));
    }

    #[test]
    fn skips_non_inherent_impls() {
        let path = tmp_file(
            "trait_impl",
            r#"
pub struct Foo;

impl std::fmt::Display for Foo {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) }
}
"#,
        );
        let cands = survey_file(&path).unwrap();
        assert!(cands.is_empty(), "trait impls aren't farm-derive targets");
    }

    #[test]
    fn generic_impl_target_type_strips_to_base_ident() {
        // Regression for real-fleet drift surfaced 2026-05-28:
        // `impl<T> Attested<T> { ... }` previously produced
        // target_type == "Attested < T >" (TokenStream::to_string
        // spaces around generics), breaking the downstream apply
        // match against Item::Struct.ident which is the bare name.
        let path = tmp_file(
            "generic-target",
            r#"
pub struct Attested<T> { pub inner: T, pub sig: String }

impl<T> Attested<T> {
    pub fn inner(&self) -> &T { &self.inner }
    pub fn sig(&self) -> &String { &self.sig }
}
"#,
        );
        let cands = survey_file(&path).unwrap();
        assert!(!cands.is_empty(), "must surface candidates on generic struct");
        for c in &cands {
            assert_eq!(
                c.target_type, "Attested",
                "target_type must strip generics + module path"
            );
        }
        // Apply must succeed on the same source — proves the
        // end-to-end roundtrip works for generic types.
        let src = std::fs::read_to_string(&path).unwrap();
        let out = apply_to_source(&src, &cands[0]).unwrap();
        assert!(out.contains("#[derive(GetterAll)]"));
        assert!(out.contains("pub struct Attested<T>"));
    }

    #[test]
    fn skips_singleton_patterns() {
        // One getter alone isn't worth a derive — only flag at ≥2.
        let path = tmp_file(
            "singleton",
            r#"
pub struct Foo { pub a: i32 }

impl Foo {
    pub fn a(&self) -> &i32 { &self.a }
}
"#,
        );
        let cands = survey_file(&path).unwrap();
        assert!(cands.is_empty(), "single match doesn't justify a derive");
    }

    #[test]
    fn classifies_invalidating_setter_as_non_setter() {
        // mado's invalidating-setters have an extra `self.last_seqno = 0;`
        // statement; canonical SetterAll has only the assign. The detector
        // should NOT flag invalidating-setters as plain SetterAll.
        let path = tmp_file(
            "invalidating",
            r#"
pub struct R { pub bg: [f32; 4], pub last_seqno: u64 }

impl R {
    pub fn set_bg(&mut self, v: [f32; 4]) { self.bg = v; self.last_seqno = 0; }
    pub fn set_fg(&mut self, v: [f32; 4]) { self.fg = v; self.last_seqno = 0; }
}
"#,
        );
        let cands = survey_file(&path).unwrap();
        // The bodies have 2 stmts each — SetterAll requires exactly 1.
        // Should NOT match.
        assert!(
            !cands.iter().any(|c| c.pattern == MatchedPattern::SetterAll),
            "invalidating-setters must not be confused with plain SetterAll"
        );
    }

    #[test]
    fn estimated_loc_saved_scales_with_field_count() {
        let path = tmp_file(
            "many",
            r#"
pub struct Foo { pub a: i32, pub b: i32, pub c: i32, pub d: i32 }

impl Foo {
    pub fn a(&self) -> &i32 { &self.a }
    pub fn b(&self) -> &i32 { &self.b }
    pub fn c(&self) -> &i32 { &self.c }
    pub fn d(&self) -> &i32 { &self.d }
}
"#,
        );
        let cands = survey_file(&path).unwrap();
        let getter = cands
            .iter()
            .find(|c| c.pattern == MatchedPattern::GetterAll)
            .unwrap();
        // 4 fields × 5 lines = 20.
        assert_eq!(getter.estimated_loc_saved, 20);
    }
}
