//! `pipeline` — survey → apply → validate, the structured-and-proven
//! adoption flow over a whole Cargo crate.
//!
//! Layers on top of [`crate::survey_tree`] (discovery),
//! [`crate::apply::apply_to_source`] (per-candidate transform), and
//! [`tatara_rust_gate::green_gate`] (cargo build + test + clippy).
//! Adds:
//!
//! 1. **Multi-candidate per file** via [`apply_all_to_source`] — when
//!    a single file has GetterAll + SetterAll on the same struct,
//!    both derives land in one pass.
//! 2. **Crate-level orchestration** via [`survey_apply_validate`] —
//!    discover across the whole crate, group by file, back up each
//!    modified file in-memory, write, run the cargo gate.
//! 3. **Rollback on red gate** — if `cargo build`/`test`/`clippy`
//!    fails after the transforms land, every modified file is
//!    restored from the in-memory backup so the operator's working
//!    tree returns to its pre-pipeline state.
//!
//! The contract: the operator runs `tatara-rust-forge
//! survey-apply-all <crate-root>` and either sees their crate
//! transformed (every typed candidate landed, all tests still green)
//! or sees their working tree unchanged with a typed failure report
//! — never a half-applied middle state.

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tatara_rust_gate::{green_gate, GateConfig, GateOutcome};

use crate::{
    apply::ApplyError, apply_to_source, cargo_deps, inject_deps, survey_tree, CargoDepsError,
    DepSource, RefactorCandidate, SurveyError,
};

/// Apply every [`RefactorCandidate`] for a single file in sequence.
/// Each apply sees the output of the previous one — so multiple
/// candidates targeting the same struct (GetterAll + SetterAll on
/// `Config`) compose. Candidates whose pattern has already been
/// applied (returns [`ApplyError::NoMethodsRemoved`]) are silently
/// skipped — they're stale relative to the in-flight source.
///
/// Returns `(modified_source, applied_count)`.
pub fn apply_all_to_source(
    src: &str,
    cands: &[RefactorCandidate],
) -> Result<(String, usize), PipelineError> {
    let mut current = src.to_string();
    let mut applied = 0usize;
    for cand in cands {
        match apply_to_source(&current, cand) {
            Ok(next) => {
                current = next;
                applied += 1;
            }
            Err(ApplyError::NoMethodsRemoved { .. }) => {
                // Stale candidate — a prior apply already removed
                // these methods (e.g. duplicate survey hit). Skip.
            }
            Err(ApplyError::TargetNotFound(t)) => {
                // The candidate's target wasn't in this file — survey
                // bug, propagate.
                return Err(PipelineError::Apply(ApplyError::TargetNotFound(t)));
            }
            Err(ApplyError::Parse(e)) => return Err(PipelineError::Apply(ApplyError::Parse(e))),
        }
    }
    Ok((current, applied))
}

/// Operator-facing options for [`survey_apply_validate`].
#[derive(Clone, Debug)]
pub struct PipelineOpts {
    /// If `true`, write modified files to disk. If `false`, the
    /// pipeline is a dry run — files are inspected + transformed in
    /// memory + the gate is NOT run (gating a dry-run crate would be
    /// meaningless since the original source remains on disk).
    pub write: bool,
    /// If `true`, run the cargo build + test + clippy gate after
    /// writing. Roll back to backups on red. Only meaningful when
    /// `write = true`.
    pub validate: bool,
    /// Gate config — defaults to all three gates (build/test/clippy).
    pub gate_cfg: GateConfig,
    /// If `true`, after the .rs transforms land, inject every
    /// applied candidate's `derive_crate` into the crate's
    /// `Cargo.toml` `[dependencies]`. Without this, the apply lands
    /// `use pleme_<x>_derive::…` but cargo build fails because the
    /// dep doesn't exist. Default `true` — set false for crates that
    /// already declare the deps under workspace inheritance.
    pub inject_cargo_deps: bool,
    /// Source for the injected deps. Default is the canonical
    /// pleme-io git URL on `main`.
    pub dep_source: DepSource,
}

impl Default for PipelineOpts {
    fn default() -> Self {
        Self {
            write: false,
            validate: true,
            gate_cfg: GateConfig::default(),
            inject_cargo_deps: true,
            dep_source: DepSource::default(),
        }
    }
}

/// Per-file outcome record — what the pipeline did to one file.
#[derive(Clone, Debug, Serialize)]
pub struct FileOutcome {
    pub file: PathBuf,
    pub candidates_attempted: usize,
    pub candidates_applied: usize,
    /// `true` if the file was actually written to disk (only when
    /// `opts.write = true` AND something changed AND the gate passed).
    pub written: bool,
}

/// What the pipeline produced + how it ended.
#[derive(Debug, Serialize)]
pub struct PipelineOutcome {
    pub crate_root: PathBuf,
    pub files: Vec<FileOutcome>,
    pub total_candidates: usize,
    pub total_applied: usize,
    pub gate: Option<GateOutcome>,
    /// If `true`, files that were written got restored to their
    /// original contents because the gate went red.
    pub rolled_back: bool,
    /// Outcome of injecting `pleme-*-derive` deps into the crate's
    /// `Cargo.toml`. `None` when `opts.inject_cargo_deps == false`
    /// or when the pipeline was a dry run.
    pub deps_injected: Option<cargo_deps::InjectOutcome>,
}

impl PipelineOutcome {
    pub fn is_clean_success(&self) -> bool {
        !self.rolled_back
            && self
                .gate
                .as_ref()
                .is_none_or(GateOutcome::is_passed)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("survey: {0}")]
    Survey(#[from] SurveyError),
    #[error("apply: {0}")]
    Apply(#[from] ApplyError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("gate: {0}")]
    Gate(#[from] tatara_rust_gate::GateError),
    #[error("cargo-deps: {0}")]
    CargoDeps(#[from] CargoDepsError),
}

/// In-memory backup so rollback can restore the original bytes of
/// every file the pipeline touched.
struct FileBackup {
    path: PathBuf,
    original: String,
}

/// Full crate-level survey → apply → inject-deps → validate.
/// Writes only when `opts.write`; rolls back to backups (both `.rs`
/// AND `Cargo.toml`) when `opts.validate` AND the cargo gate fails.
/// The operator's tree never ends in a half-applied state.
pub fn survey_apply_validate(
    crate_root: &Path,
    opts: &PipelineOpts,
) -> Result<PipelineOutcome, PipelineError> {
    let src_root = crate_root.join("src");
    let walk_root: &Path = if src_root.exists() { &src_root } else { crate_root };
    let all_cands = survey_tree(walk_root)?;

    // Group by file.
    let mut by_file: BTreeMap<PathBuf, Vec<RefactorCandidate>> = BTreeMap::new();
    for cand in all_cands {
        by_file.entry(cand.file.clone()).or_default().push(cand);
    }

    let mut file_outcomes: Vec<FileOutcome> = vec![];
    let mut backups: Vec<FileBackup> = vec![];
    let mut total_attempted = 0usize;
    let mut total_applied = 0usize;
    // Every derive_crate touched by an applied candidate — gets
    // injected into Cargo.toml so cargo build can resolve the deps.
    let mut applied_derive_crates: std::collections::BTreeSet<&'static str> =
        std::collections::BTreeSet::new();

    for (file, cands) in &by_file {
        let original = std::fs::read_to_string(file)?;
        let (modified, applied) = apply_all_to_source(&original, cands)?;
        total_attempted += cands.len();
        total_applied += applied;

        let changed = applied > 0 && modified != original;
        let mut written = false;
        if opts.write && changed {
            backups.push(FileBackup {
                path: file.clone(),
                original: original.clone(),
            });
            std::fs::write(file, &modified)?;
            written = true;
            // Collect every derive_crate the applied candidates need.
            for c in cands.iter().take(applied) {
                applied_derive_crates.insert(c.derive_crate);
            }
        }

        file_outcomes.push(FileOutcome {
            file: file.clone(),
            candidates_attempted: cands.len(),
            candidates_applied: applied,
            written,
        });
    }

    // Inject Cargo.toml deps so cargo build can resolve them — done
    // BEFORE the gate so the build sees the deps. Backed up
    // symmetric to the .rs files so rollback restores everything.
    let deps_injected = if opts.write && opts.inject_cargo_deps && !applied_derive_crates.is_empty()
    {
        let cargo_toml = crate_root.join("Cargo.toml");
        if cargo_toml.exists() {
            let cargo_original = std::fs::read_to_string(&cargo_toml)?;
            let names: Vec<&str> = applied_derive_crates.iter().copied().collect();
            let outcome = inject_deps(&cargo_toml, &names, opts.dep_source.clone())?;
            if outcome.changed() {
                backups.push(FileBackup {
                    path: cargo_toml,
                    original: cargo_original,
                });
            }
            Some(outcome)
        } else {
            None
        }
    } else {
        None
    };

    // Validate only when we actually wrote — gating with the original
    // source on disk would produce a misleading green.
    let (gate, rolled_back) = if opts.write && opts.validate && !backups.is_empty() {
        let outcome = green_gate(crate_root, &opts.gate_cfg)?;
        let rolled_back = if matches!(outcome, GateOutcome::Failed { .. }) {
            for b in &backups {
                std::fs::write(&b.path, &b.original)?;
            }
            for fo in &mut file_outcomes {
                if backups.iter().any(|b| b.path == fo.file) {
                    fo.written = false;
                }
            }
            true
        } else {
            false
        };
        (Some(outcome), rolled_back)
    } else {
        (None, false)
    };

    Ok(PipelineOutcome {
        crate_root: crate_root.to_path_buf(),
        files: file_outcomes,
        total_candidates: total_attempted,
        total_applied,
        gate,
        rolled_back,
        deps_injected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_crate(name: &str, lib_body: &str, manifest_extra: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "tatara-pipeline-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::write(
            tmp.join("Cargo.toml"),
            format!(
                r#"[package]
name = "tatara-pipeline-test-{name}"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"

{manifest_extra}
"#
            ),
        )
        .unwrap();
        std::fs::write(tmp.join("src/lib.rs"), lib_body).unwrap();
        tmp
    }

    #[test]
    fn apply_all_handles_multiple_candidates_same_struct() {
        // Getter + setter on the same struct — survey produces TWO
        // candidates for one file; apply_all should land both.
        let src = r#"
pub struct Config { pub host: String, pub port: u16 }

impl Config {
    pub fn host(&self) -> &String { &self.host }
    pub fn port(&self) -> &u16 { &self.port }
    pub fn set_host(&mut self, v: String) { self.host = v; }
    pub fn set_port(&mut self, v: u16) { self.port = v; }
}
"#;
        let crate_root = tmp_crate("multi-cand", src, "");
        let cands = survey_tree(&crate_root.join("src")).unwrap();
        assert!(cands.len() >= 2, "expected ≥2 candidates, got {}", cands.len());

        let (out, applied) = apply_all_to_source(src, &cands).unwrap();
        assert_eq!(applied, cands.len(), "every candidate must land");

        let parsed: syn::File = syn::parse_str(&out).unwrap();
        // Both derives present on Config.
        let s = parsed
            .items
            .iter()
            .find_map(|i| if let syn::Item::Struct(s) = i { Some(s) } else { None })
            .unwrap();
        let derives: Vec<String> = s
            .attrs
            .iter()
            .filter(|a| a.path().is_ident("derive"))
            .flat_map(|a| {
                let mut out = vec![];
                let _ = a.parse_nested_meta(|m| {
                    if let Some(id) = m.path.get_ident() {
                        out.push(id.to_string());
                    }
                    Ok(())
                });
                out
            })
            .collect();
        assert!(derives.contains(&"GetterAll".to_string()), "GetterAll missing");
        assert!(derives.contains(&"SetterAll".to_string()), "SetterAll missing");

        // Impl block is fully removed.
        assert!(
            !parsed.items.iter().any(|i| matches!(i, syn::Item::Impl(_))),
            "impl block should be dropped after both derives land"
        );
    }

    /// Two-field getter struct — the minimum shape survey detects as
    /// GetterAll (a single getter could just be hand-rolled; the
    /// derive earns its keep at ≥2 fields).
    const TWO_FIELD_GETTER_SRC: &str = r#"
pub struct Foo { pub a: i32, pub b: String }

impl Foo {
    pub fn a(&self) -> &i32 { &self.a }
    pub fn b(&self) -> &String { &self.b }
}
"#;

    #[test]
    fn pipeline_dry_run_reports_candidates_without_writing() {
        let crate_root = tmp_crate("dry-run", TWO_FIELD_GETTER_SRC, "");
        let opts = PipelineOpts {
            write: false,
            validate: false,
            gate_cfg: GateConfig::default(),
            inject_cargo_deps: true,
            dep_source: crate::DepSource::default(),
        };
        let out = survey_apply_validate(&crate_root, &opts).unwrap();
        assert!(
            out.total_candidates >= 1,
            "dry run must surface ≥1 candidate, got {}",
            out.total_candidates
        );
        assert!(out.total_applied >= 1);
        assert!(out.files.iter().all(|f| !f.written), "dry run must not write");
        assert!(out.gate.is_none(), "dry run must not gate");
        assert!(!out.rolled_back);

        // The file on disk is byte-identical to the original.
        let on_disk = std::fs::read_to_string(crate_root.join("src/lib.rs")).unwrap();
        assert_eq!(on_disk, TWO_FIELD_GETTER_SRC, "dry run must not mutate disk");
    }

    #[test]
    fn pipeline_write_without_validate_persists_changes() {
        let crate_root = tmp_crate("write-no-val", TWO_FIELD_GETTER_SRC, "");
        let opts = PipelineOpts {
            write: true,
            validate: false,
            gate_cfg: GateConfig::default(),
            inject_cargo_deps: false, // legacy test predates dep injection
            dep_source: crate::DepSource::default(),
        };
        let out = survey_apply_validate(&crate_root, &opts).unwrap();
        assert!(out.total_applied >= 1);
        assert!(out.files.iter().any(|f| f.written));
        assert!(out.gate.is_none(), "validate=false skips gate");
        assert!(!out.rolled_back);

        // File on disk has the derive landed.
        let on_disk = std::fs::read_to_string(crate_root.join("src/lib.rs")).unwrap();
        assert!(on_disk.contains("GetterAll"));
        assert!(!on_disk.contains("impl Foo"), "empty impl dropped");
    }

    #[test]
    fn is_clean_success_holds_when_no_writes() {
        let outcome = PipelineOutcome {
            crate_root: PathBuf::from("/tmp"),
            files: vec![],
            total_candidates: 0,
            total_applied: 0,
            gate: None,
            rolled_back: false,
            deps_injected: None,
        };
        assert!(outcome.is_clean_success());
    }

    #[test]
    fn is_clean_success_false_when_rolled_back() {
        let outcome = PipelineOutcome {
            crate_root: PathBuf::from("/tmp"),
            files: vec![],
            total_candidates: 1,
            total_applied: 1,
            gate: Some(GateOutcome::Failed {
                gate: tatara_rust_gate::Gate::Test,
                exit: Some(1),
                stdout: String::new(),
                stderr: "boom".into(),
            }),
            rolled_back: true,
            deps_injected: None,
        };
        assert!(!outcome.is_clean_success());
    }

    #[test]
    fn write_path_injects_pleme_io_git_deps_into_cargo_toml() {
        let crate_root = tmp_crate("inject-deps", TWO_FIELD_GETTER_SRC, "");
        let opts = PipelineOpts {
            write: true,
            validate: false, // don't actually try to compile against a non-existent git remote
            gate_cfg: GateConfig::default(),
            inject_cargo_deps: true,
            dep_source: crate::DepSource::PlemeIoGit,
        };
        let out = survey_apply_validate(&crate_root, &opts).unwrap();
        assert!(out.total_applied >= 1);
        let deps = out.deps_injected.expect("deps_injected populated when write=true + opt_in");
        assert!(deps.changed(), "first apply must add the dep");
        assert!(deps.added.iter().any(|n| n == "pleme-getter-derive"));

        let cargo_after = std::fs::read_to_string(crate_root.join("Cargo.toml")).unwrap();
        assert!(cargo_after.contains("pleme-getter-derive"));
        assert!(cargo_after.contains("github.com/pleme-io/pleme-getter-derive.git"));
    }

    #[test]
    fn opt_out_skips_cargo_toml_injection() {
        let crate_root = tmp_crate("no-inject", TWO_FIELD_GETTER_SRC, "");
        let opts = PipelineOpts {
            write: true,
            validate: false,
            gate_cfg: GateConfig::default(),
            inject_cargo_deps: false,
            dep_source: crate::DepSource::PlemeIoGit,
        };
        let out = survey_apply_validate(&crate_root, &opts).unwrap();
        assert!(out.deps_injected.is_none(), "opt-out leaves deps_injected=None");
        let cargo_after = std::fs::read_to_string(crate_root.join("Cargo.toml")).unwrap();
        assert!(!cargo_after.contains("pleme-getter-derive"));
    }
}
