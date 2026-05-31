//! `tatara-rust-catalog` — fleet-aware macro registry.
//!
//! One typed `MacroCatalogSpec` value lists every macro the platform
//! ships, plus per-entry metadata (doc, since, owner). Compiles into:
//! - `catalog.json` — machine-readable registry
//! - `docs/<crate>.md` — one Markdown page per entry
//! - `Cargo.toml` — workspace root listing every entry's crate
//! - `WORKSPACE.md` — operator-facing index
//!
//! Discovery becomes mechanical: `cargo doc --workspace` + `jq` on
//! `catalog.json` gives the operator the full surface.
//!
//! Compounding: validation runs on every catalog entry's inner Spec
//! before the catalog accepts it. Bad Specs can never join the
//! registry.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold};
use tatara_rust_composite::CompositeDeriveSpec;
use tatara_rust_derive::{
    EnumFoldDeriveSpec, KindRoundTripSpec, NewtypeDeriveSpec, PerFieldDeriveSpec,
    PerVariantDeriveSpec, ProcDeriveSpec, VerificationMatrixSpec,
};
use tatara_rust_macro_rules::MacroRulesSpec;
use tatara_rust_proc_attr::ProcAttrSpec;
use tatara_rust_proc_fn::ProcFnSpec;
use tatara_rust_repo::RepoSpec;
use tatara_rust_validate::{Validate, Violation};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroCatalogSpec {
    /// Human-readable catalog title — appears in WORKSPACE.md.
    pub title: String,
    /// Catalog entries.
    pub entries: Vec<CatalogEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// Cargo crate name the entry materializes under.
    pub crate_name: String,
    /// One-line description for the docs site.
    pub description: String,
    /// Version the entry was added to the catalog (semver string).
    pub since: String,
    /// Free-form owner — usually a team or person handle.
    pub owner: String,
    /// Optional explicit verifier strategy for `tatara-rust-verify`.
    ///
    /// When set, `consumer-verify` dispatches directly to the named
    /// smoke-test renderer. When `None`, the verifier falls back to
    /// template-text classification (back-compat). Explicit hints are
    /// preferred — they remove the fragility of substring matching
    /// and let new strategy variants be added without touching
    /// `tatara-rust-verify`'s classifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verifier_hint: Option<VerifierHint>,
    /// The Spec under tagged dispatch.
    #[serde(flatten)]
    pub spec: CatalogSpec,
}

/// Typed verifier-smoke-renderer dispatch. Each variant maps to one
/// `render_<kind>_smoke` function in `tatara-rust-verify`. New strategies
/// land here + in `tatara-rust-verify::render_per_kind_body` (one match arm).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VerifierHint {
    /// Apply the derive; assert the consumer struct/enum compiles.
    /// No method calls. Falls back here when no explicit hint + no
    /// inferred strategy.
    CompileOnly,
    /// `pub fn <field>(&self) -> &<Type>`.
    PerFieldGetter,
    /// `pub fn set_<field>(&mut self, v: <Type>)`.
    PerFieldSetter,
    /// `pub fn with_<field>(mut self, v: <Type>) -> Self`.
    PerFieldWithBuilder,
    /// `pub fn <field>_mut(&mut self) -> &mut <Type>`.
    PerFieldAsMut,
    /// `pub fn replace_<field>(&mut self, v: <Type>) -> <Type>`.
    PerFieldReplace,
    /// `pub fn take_<field>(&mut self) -> <Type>` (T: Default at call site).
    PerFieldTake,
    /// `pub fn set_<field>(&mut self, v: <T>) { self.<field> = v; self.last_seqno = 0; }`
    /// — cache-invalidating setter. Sample struct must include
    /// `last_seqno: u64` field (consumer contract).
    PerFieldInvalidatingSetter,
    /// `pub fn is_<variant>(&self) -> bool` matches-style predicate.
    PerVariantIsVariant,
    /// Newtype `From<Inner> for Wrapper` AND `From<Wrapper> for Inner`.
    NewtypeImplFrom,
    /// Newtype `AsRef<Inner>`.
    NewtypeAsRef,
    /// Newtype `Deref` with `Target = Inner`.
    NewtypeDeref,
    /// Newtype `pub fn inner(&self) -> &Inner` + `into_inner(self) -> Inner`.
    NewtypeInner,
    /// Enum-fold `const ALL: &[Self]` + `all()` for unit-only enums.
    EnumFoldAllVariants,
    /// Enum-fold `const COUNT: usize` + `count()`.
    EnumFoldVariantCount,
    /// Enum-fold `const NAMES: &[&str]` + `names()`.
    EnumFoldVariantNames,
    /// `pub fn into_<field>(self) -> <Type>` — per-field consuming getter.
    PerFieldOwned,
    /// Newtype `Borrow<Inner>`.
    NewtypeBorrow,
    /// Newtype `BorrowMut<Inner>`.
    NewtypeBorrowMut,
    /// Newtype `DerefMut` with `Target = Inner` (pairs with NewtypeDeref).
    NewtypeDerefMut,
    /// Newtype `Display` — delegates to inner's Display impl.
    NewtypeDisplay,
    /// Newtype `Default` where `Inner: Default` — forwards.
    NewtypeDefault,
    /// Enum-fold `pub fn as_str(&self) -> &'static str` returning the
    /// bare variant name. Unit-variant enums only.
    EnumFoldVariantStr,
    /// `pub fn reset_<field>(&mut self) where <T>: Default { … }` per field.
    PerFieldReset,
    /// `pub fn swap_<field>(&mut self, other: &mut Self) { mem::swap(…) }` per field.
    PerFieldSwap,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CatalogSpec {
    Derive { spec: ProcDeriveSpec },
    PerField { spec: PerFieldDeriveSpec },
    PerVariant { spec: PerVariantDeriveSpec },
    Newtype { spec: NewtypeDeriveSpec },
    EnumFold { spec: EnumFoldDeriveSpec },
    ProcAttr { spec: ProcAttrSpec },
    ProcFn { spec: ProcFnSpec },
    MacroRules { spec: MacroRulesSpec },
    Composite { spec: CompositeDeriveSpec },
    KindRoundTrip { spec: KindRoundTripSpec },
    VerificationMatrix { spec: VerificationMatrixSpec },
}

/// Self-describing capability surface every catalog-eligible Spec
/// implements. Collapses what were four parallel 9-arm matches on
/// `CatalogSpec` into one match returning `&dyn SpecKind` + four
/// 1-line delegates. Adding a new spec kind becomes:
///   1. `impl SpecKind for NewSpec { … }` (single block)
///   2. `Self::NewVariant { spec } => spec,` in `as_spec_kind`
///   3. `CatalogSpec::NewVariant { spec: NewSpec }` (the enum arm)
///
/// `CompileToCrate` + `Validate` are supertraits so a single
/// `&dyn SpecKind` value gives the consumer compile-to-crate, validate,
/// kind_label, and trait-name accessors in one pointer.
pub trait SpecKind: CompileToCrate + Validate {
    /// Kebab-case identifier used in `catalog.json` `"kind"` field +
    /// `WORKSPACE.md` table rendering.
    fn kind_label(&self) -> &'static str;
    /// User-facing identifier (the thing inside `#[derive(…)]` or
    /// `#[my_attr]` / `my_macro!`). The verifier embeds this in the
    /// generated `consumer-verify` source.
    fn trait_name_for_verifier(&self) -> &str;
}

impl SpecKind for ProcDeriveSpec {
    fn kind_label(&self) -> &'static str { "derive" }
    fn trait_name_for_verifier(&self) -> &str { &self.trait_name.0 }
}
impl SpecKind for PerFieldDeriveSpec {
    fn kind_label(&self) -> &'static str { "per-field" }
    fn trait_name_for_verifier(&self) -> &str { &self.trait_name.0 }
}
impl SpecKind for PerVariantDeriveSpec {
    fn kind_label(&self) -> &'static str { "per-variant" }
    fn trait_name_for_verifier(&self) -> &str { &self.trait_name.0 }
}
impl SpecKind for NewtypeDeriveSpec {
    fn kind_label(&self) -> &'static str { "newtype" }
    fn trait_name_for_verifier(&self) -> &str { &self.trait_name.0 }
}
impl SpecKind for EnumFoldDeriveSpec {
    fn kind_label(&self) -> &'static str { "enum-fold" }
    fn trait_name_for_verifier(&self) -> &str { &self.trait_name.0 }
}
impl SpecKind for ProcAttrSpec {
    fn kind_label(&self) -> &'static str { "proc-attr" }
    fn trait_name_for_verifier(&self) -> &str { &self.macro_name.0 }
}
impl SpecKind for ProcFnSpec {
    fn kind_label(&self) -> &'static str { "proc-fn" }
    fn trait_name_for_verifier(&self) -> &str { &self.macro_name.0 }
}
impl SpecKind for MacroRulesSpec {
    fn kind_label(&self) -> &'static str { "macro-rules" }
    fn trait_name_for_verifier(&self) -> &str { &self.macro_name.0 }
}
impl SpecKind for CompositeDeriveSpec {
    fn kind_label(&self) -> &'static str { "composite" }
    fn trait_name_for_verifier(&self) -> &str { &self.bundle_name.0 }
}
impl SpecKind for KindRoundTripSpec {
    fn kind_label(&self) -> &'static str { "kind-round-trip" }
    fn trait_name_for_verifier(&self) -> &str { &self.trait_name.0 }
}
impl SpecKind for VerificationMatrixSpec {
    fn kind_label(&self) -> &'static str { "verification-matrix" }
    fn trait_name_for_verifier(&self) -> &str { self.primary_name() }
}

impl CatalogSpec {
    /// **Single dispatch surface** — returns the inner Spec as
    /// `&dyn SpecKind`. Every per-method accessor below delegates
    /// through this to avoid duplicated match arms.
    pub fn as_spec_kind(&self) -> &dyn SpecKind {
        match self {
            Self::Derive { spec } => spec,
            Self::PerField { spec } => spec,
            Self::PerVariant { spec } => spec,
            Self::Newtype { spec } => spec,
            Self::EnumFold { spec } => spec,
            Self::ProcAttr { spec } => spec,
            Self::ProcFn { spec } => spec,
            Self::MacroRules { spec } => spec,
            Self::Composite { spec } => spec,
            Self::KindRoundTrip { spec } => spec,
            Self::VerificationMatrix { spec } => spec,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        self.as_spec_kind().kind_label()
    }

    pub fn validate(&self) -> Vec<Violation> {
        self.as_spec_kind().validate()
    }

    pub fn compile_to_crate(&self, crate_name: &str) -> Result<CrateScaffold, AstError> {
        self.as_spec_kind().compile_to_crate(crate_name)
    }

    /// Best-effort trait-name accessor for the verifier. Returns the
    /// user-facing derive identifier (the thing inside `#[derive(...)]`).
    /// For ProcAttr/ProcFn/MacroRules the returned name is the macro
    /// invocation token (`#[my_attr]` / `my_macro!`). Wrapped in `Option`
    /// for back-compat with consumers that handle a missing trait name.
    pub fn trait_name_for_verifier(&self) -> Option<&str> {
        Some(self.as_spec_kind().trait_name_for_verifier())
    }
}

#[derive(Debug)]
pub struct CatalogScaffold {
    pub catalog_json: String,
    pub workspace_md: String,
    pub root_cargo_toml: String,
    pub member_crates: Vec<CrateScaffold>,
    pub docs_md: Vec<(String, String)>, // (relative path, contents)
}

impl CatalogScaffold {
    /// Write everything under `root`. Creates `<root>/catalog.json`,
    /// `<root>/WORKSPACE.md`, `<root>/Cargo.toml`, `<root>/docs/<name>.md`,
    /// `<root>/crates/<name>/…` for each entry.
    pub fn write_to(&self, root: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        std::fs::write(root.join("catalog.json"), &self.catalog_json)?;
        std::fs::write(root.join("WORKSPACE.md"), &self.workspace_md)?;
        std::fs::write(root.join("Cargo.toml"), &self.root_cargo_toml)?;
        for (path, contents) in &self.docs_md {
            let p = root.join(path);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(p, contents)?;
        }
        for c in &self.member_crates {
            c.write_to(&root.join("crates").join(&c.name))?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("ast: {0}")]
    Ast(#[from] AstError),
    #[error("catalog rejected {invalid} invalid entr{plural} (see violations field)", plural = if *invalid == 1 { "y" } else { "ies" })]
    InvalidEntries {
        invalid: usize,
        violations: Vec<(String, Vec<Violation>)>,
    },
    #[error("duplicate crate name `{0}` across entries")]
    DuplicateCrateName(String),
    #[error("serde_json: {0}")]
    Json(#[from] serde_json::Error),
}

impl MacroCatalogSpec {
    /// Validate every entry + global uniqueness, then materialize each
    /// entry as an **independently publishable** repo (each gets its
    /// own flake, caixa.lisp, auto-release workflow, clippy.toml,
    /// LICENSE, .gitignore, rust-toolchain.toml, README). Returns one
    /// `RepoSpec` per entry; consumer calls `.compile()` to get a
    /// `CrateScaffold` ready for `write_to(<repo-root>)`.
    ///
    /// `repo_url_prefix` is concatenated with the entry's `crate_name`
    /// to form the per-repo GitHub URL (e.g. `https://github.com/pleme-io`).
    pub fn compile_to_repos(
        &self,
        repo_url_prefix: &str,
    ) -> Result<Vec<RepoSpec>, CatalogError> {
        self.validate_all()?;
        let mut out = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            let scaffold = e.spec.compile_to_crate(&e.crate_name)?;
            let repo_url = format!(
                "{}/{}",
                repo_url_prefix.trim_end_matches('/'),
                e.crate_name
            );
            out.push(RepoSpec::defaults_for(
                scaffold,
                &e.crate_name,
                repo_url,
                &e.description,
            ));
        }
        Ok(out)
    }

    /// Internal: validate every inner Spec + global uniqueness.
    fn validate_all(&self) -> Result<(), CatalogError> {
        let mut all_violations: Vec<(String, Vec<Violation>)> = vec![];
        for e in &self.entries {
            let v = e.spec.validate();
            if !v.is_empty() {
                all_violations.push((e.crate_name.clone(), v));
            }
        }
        if !all_violations.is_empty() {
            return Err(CatalogError::InvalidEntries {
                invalid: all_violations.len(),
                violations: all_violations,
            });
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for e in &self.entries {
            if !seen.insert(e.crate_name.as_str()) {
                return Err(CatalogError::DuplicateCrateName(e.crate_name.clone()));
            }
        }
        Ok(())
    }

    /// Validate every entry + global uniqueness, then compile to a
    /// `CatalogScaffold`. Rejects if any entry has Violations.
    pub fn compile_to_catalog(&self) -> Result<CatalogScaffold, CatalogError> {
        self.validate_all()?;

        // Compile each entry's Spec.
        let mut members = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            members.push(e.spec.compile_to_crate(&e.crate_name)?);
        }

        // 4. Emit JSON manifest.
        let catalog_json = serde_json::to_string_pretty(self)?;

        // 5. WORKSPACE.md operator index.
        let workspace_md = render_workspace_md(&self.title, &self.entries);

        // 6. docs/<name>.md one page per entry.
        let docs_md = self
            .entries
            .iter()
            .map(|e| {
                (
                    format!("docs/{}.md", e.crate_name),
                    render_entry_md(e),
                )
            })
            .collect();

        // 7. Cargo.toml workspace root.
        let root_cargo_toml = render_root_cargo_toml(&self.entries);

        Ok(CatalogScaffold {
            catalog_json,
            workspace_md,
            root_cargo_toml,
            member_crates: members,
            docs_md,
        })
    }
}

fn render_workspace_md(title: &str, entries: &[CatalogEntry]) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {title}\n\n"));
    s.push_str(&format!("{} entries.\n\n", entries.len()));
    s.push_str("| Crate | Kind | Description | Since | Owner |\n");
    s.push_str("|---|---|---|---|---|\n");
    for e in entries {
        s.push_str(&format!(
            "| [`{name}`](docs/{name}.md) | `{kind}` | {desc} | {since} | {owner} |\n",
            name = e.crate_name,
            kind = e.spec.kind_label(),
            desc = e.description,
            since = e.since,
            owner = e.owner
        ));
    }
    s
}

fn render_entry_md(e: &CatalogEntry) -> String {
    format!(
        r#"# `{name}`

**Kind:** `{kind}`
**Since:** {since}
**Owner:** {owner}

{desc}

## Materialization

The crate `{name}` is generated by `tatara-rust-catalog` from a
`CatalogEntry` of kind `{kind}`. See `catalog.json` for the typed Spec.

To consume:

```rust
use {name_under}::*;
```
"#,
        name = e.crate_name,
        name_under = e.crate_name.replace('-', "_"),
        kind = e.spec.kind_label(),
        since = e.since,
        owner = e.owner,
        desc = e.description,
    )
}

fn render_root_cargo_toml(entries: &[CatalogEntry]) -> String {
    let members = entries
        .iter()
        .map(|e| format!("  \"crates/{}\"", e.crate_name))
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        r#"[workspace]
resolver = "2"
members = [
{members}
]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_ast::Ident;
    use tatara_rust_derive::PerFieldTarget;

    fn sample_entry() -> CatalogEntry {
        CatalogEntry {
            crate_name: "getter-all-derive".into(),
            description: "Per-field getters.".into(),
            since: "0.1.0".into(),
            owner: "pleme-io".into(),
            verifier_hint: None,
            spec: CatalogSpec::PerField {
                spec: PerFieldDeriveSpec {
                    trait_name: Ident::new("GetterAll"),
                    target: PerFieldTarget::NamedStruct,
                    trait_ref: None,
                    per_field_template:
                        "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
                    method_name_template: None,
                    impl_prelude: None,
                    skip_fields: vec![],
                    field_attribute: None,
                },
            },
        }
    }

    fn sample_catalog() -> MacroCatalogSpec {
        MacroCatalogSpec {
            title: "pleme-io macros".into(),
            entries: vec![sample_entry()],
        }
    }

    #[test]
    fn compiles_clean_catalog() {
        let out = sample_catalog().compile_to_catalog().unwrap();
        assert_eq!(out.member_crates.len(), 1);
        assert!(out.catalog_json.contains("getter-all-derive"));
        assert!(out.workspace_md.contains("# pleme-io macros"));
        assert!(out.workspace_md.contains("`getter-all-derive`"));
        assert_eq!(out.docs_md.len(), 1);
        assert!(out.root_cargo_toml.contains(r#""crates/getter-all-derive""#));
    }

    #[test]
    fn rejects_invalid_entry() {
        let mut cat = sample_catalog();
        if let CatalogSpec::PerField { spec } = &mut cat.entries[0].spec {
            spec.trait_name = Ident::new("");
        }
        let err = cat.compile_to_catalog().unwrap_err();
        assert!(matches!(
            err,
            CatalogError::InvalidEntries { invalid: 1, .. }
        ));
    }

    #[test]
    fn rejects_duplicate_crate_names() {
        let mut cat = sample_catalog();
        let dup = sample_entry();
        cat.entries.push(dup);
        let err = cat.compile_to_catalog().unwrap_err();
        assert!(matches!(err, CatalogError::DuplicateCrateName(_)));
    }

    #[test]
    fn workspace_md_lists_every_entry() {
        let mut cat = sample_catalog();
        let mut e2 = sample_entry();
        e2.crate_name = "setter-all-derive".into();
        cat.entries.push(e2);
        let out = cat.compile_to_catalog().unwrap();
        assert!(out.workspace_md.contains("`getter-all-derive`"));
        assert!(out.workspace_md.contains("`setter-all-derive`"));
        assert!(out.workspace_md.contains("2 entries"));
    }

    #[test]
    fn entry_md_shows_kind_and_metadata() {
        let out = sample_catalog().compile_to_catalog().unwrap();
        let (_path, contents) = out.docs_md.first().unwrap();
        assert!(contents.contains("# `getter-all-derive`"));
        assert!(contents.contains("**Kind:** `per-field`"));
        assert!(contents.contains("**Since:** 0.1.0"));
    }

    #[test]
    fn catalog_json_round_trips() {
        let cat = sample_catalog();
        let j = serde_json::to_string(&cat).unwrap();
        let back: MacroCatalogSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(cat, back);
    }
}
