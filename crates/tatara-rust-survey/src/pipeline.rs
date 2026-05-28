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

use crate::{apply::ApplyError, apply_to_source, survey_tree, RefactorCandidate, SurveyError};

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
}

impl Default for PipelineOpts {
    fn default() -> Self {
        Self {
            write: false,
            validate: true,
            gate_cfg: GateConfig::default(),
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
}

/// In-memory backup so rollback can restore the original bytes of
/// every file the pipeline touched.
struct FileBackup {
    path: PathBuf,
    original: String,
}

/// Full crate-level survey → apply → validate. Writes only when
/// `opts.write`; rolls back to backups when `opts.validate` AND the
/// cargo gate fails. The operator's tree never ends in a
/// half-applied state.
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
        }

        file_outcomes.push(FileOutcome {
            file: file.clone(),
            candidates_attempted: cands.len(),
            candidates_applied: applied,
            written,
        });
    }

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
        };
        assert!(!outcome.is_clean_success());
    }
}
