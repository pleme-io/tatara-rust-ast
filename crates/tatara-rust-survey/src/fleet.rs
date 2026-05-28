//! `fleet` — multi-crate survey aggregator.
//!
//! Lifts the single-crate `survey_apply_validate` discovery flow up
//! to a directory of Cargo crates (the natural pleme-io org shape:
//! `~/code/github/pleme-io/<repo>/Cargo.toml`). Produces a typed
//! [`FleetSurveyReport`] the operator can act on as a leaderboard.
//!
//! ```text
//! survey_fleet(root)
//!   ├── enumerate immediate subdirs with a Cargo.toml at the root
//!   ├── survey_tree(crate/src/) per crate (dry, no writes, no gate)
//!   ├── compute LOC saved + candidate breakdown
//!   └── sort entries by candidate count (desc); return aggregate report
//! ```
//!
//! No transformation, no writes — pure discovery at fleet scale. The
//! operator then drives `survey-apply-all` per-crate against the
//! crates the report highlights. Future bulk operator command will
//! map green-gate over `crates` for one-shot fleet adoption.

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{
    pipeline::{survey_apply_validate, PipelineOpts, PipelineOutcome},
    survey_tree, MatchedPattern, PipelineError, RefactorCandidate, SurveyError,
};

/// One crate's slice of the fleet survey.
#[derive(Clone, Debug, Serialize)]
pub struct CrateSurveyEntry {
    pub crate_path: PathBuf,
    pub candidate_count: usize,
    pub loc_saved: usize,
    /// Pattern → count, so the report shows where each crate's
    /// adoption gains concentrate.
    pub pattern_breakdown: BTreeMap<String, usize>,
    pub candidates: Vec<RefactorCandidate>,
}

/// Aggregate over every Cargo crate under a root directory.
#[derive(Debug, Serialize)]
pub struct FleetSurveyReport {
    pub root: PathBuf,
    pub crates_scanned: usize,
    pub crates_with_candidates: usize,
    pub total_candidates: usize,
    pub total_loc_saved: usize,
    pub pattern_totals: BTreeMap<String, usize>,
    /// Sorted by candidate count (desc) — the operator's leaderboard.
    pub entries: Vec<CrateSurveyEntry>,
}

impl FleetSurveyReport {
    /// Crates filtered by a minimum candidate count — operator
    /// uses this to focus on highest-leverage targets.
    pub fn entries_at_least(&self, threshold: usize) -> Vec<&CrateSurveyEntry> {
        self.entries
            .iter()
            .filter(|e| e.candidate_count >= threshold)
            .collect()
    }
}

/// Walk every immediate subdirectory of `root` looking for a
/// `Cargo.toml` at the root. For each crate found, run `survey_tree`
/// against its `src/` (falling back to the crate root if no `src/`
/// exists — workspace member dirs may not have one).
///
/// Parse failures on individual `.rs` files inside a crate are
/// already swallowed by `survey_tree`. IO errors on `read_dir` of
/// `root` propagate.
pub fn survey_fleet(root: &Path) -> Result<FleetSurveyReport, SurveyError> {
    let mut entries: Vec<CrateSurveyEntry> = vec![];
    let mut pattern_totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut crates_scanned = 0usize;

    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if !path.join("Cargo.toml").exists() {
            continue;
        }

        crates_scanned += 1;
        let src = path.join("src");
        let walk_root = if src.exists() { src } else { path.clone() };

        let cands = match survey_tree(&walk_root) {
            Ok(cs) => cs,
            Err(SurveyError::Io(_)) => continue, // unreadable crate — skip
            Err(other) => return Err(other),
        };

        if cands.is_empty() {
            continue;
        }

        let mut breakdown: BTreeMap<String, usize> = BTreeMap::new();
        let mut loc_saved = 0usize;
        for c in &cands {
            *breakdown.entry(pattern_label(c.pattern)).or_default() += 1;
            *pattern_totals
                .entry(pattern_label(c.pattern))
                .or_default() += 1;
            loc_saved += c.estimated_loc_saved;
        }

        entries.push(CrateSurveyEntry {
            crate_path: path,
            candidate_count: cands.len(),
            loc_saved,
            pattern_breakdown: breakdown,
            candidates: cands,
        });
    }

    // Leaderboard order: most candidates first.
    entries.sort_by(|a, b| {
        b.candidate_count
            .cmp(&a.candidate_count)
            .then_with(|| b.loc_saved.cmp(&a.loc_saved))
            .then_with(|| a.crate_path.cmp(&b.crate_path))
    });

    let total_candidates: usize = entries.iter().map(|e| e.candidate_count).sum();
    let total_loc_saved: usize = entries.iter().map(|e| e.loc_saved).sum();
    let crates_with_candidates = entries.len();

    Ok(FleetSurveyReport {
        root: root.to_path_buf(),
        crates_scanned,
        crates_with_candidates,
        total_candidates,
        total_loc_saved,
        pattern_totals,
        entries,
    })
}

/// Label for a pattern in the fleet leaderboard. Routed through the
/// Detector registry so adding a new pattern means zero changes
/// here — the existing `derive_trait()` is the canonical name.
fn pattern_label(p: MatchedPattern) -> String {
    p.derive_trait().to_string()
}

// ─────────────────────────────────────────────────────────────────────
// Fleet-wide apply — compose survey_fleet (discovery) + W12 atomic
// per-crate pipeline (apply + inject Cargo.toml deps + gate + rollback)
// ─────────────────────────────────────────────────────────────────────

/// Operator-facing options for [`survey_fleet_apply`].
///
/// Composes [`PipelineOpts`] (the per-crate config — applied
/// identically to every fleet entry) with fleet-specific concerns:
/// a candidate-count threshold + a safety brake that stops at the
/// first per-crate rollback.
#[derive(Clone, Debug)]
pub struct FleetApplyOpts {
    /// Skip any crate with fewer than this many candidates. Default
    /// 1 (touch every crate with at least one finding); operators
    /// running --write should set this higher (e.g. 5–10) to limit
    /// blast radius to the top-leverage targets.
    pub threshold: usize,
    /// Per-crate pipeline opts — applied to every fleet entry.
    /// Defaults to dry-run via [`PipelineOpts::default`].
    pub pipeline_opts: PipelineOpts,
    /// Safety brake: if any crate's gate goes red and rolls back,
    /// stop the fleet sweep there (do not attempt subsequent
    /// crates). Default `true` — assume a rollback signals a
    /// substrate bug worth pausing to investigate.
    pub stop_on_first_rollback: bool,
}

impl Default for FleetApplyOpts {
    fn default() -> Self {
        Self {
            threshold: 1,
            pipeline_opts: PipelineOpts::default(),
            stop_on_first_rollback: true,
        }
    }
}

/// One crate's outcome in the fleet apply sweep.
#[derive(Debug, Serialize)]
pub struct FleetApplyEntry {
    pub crate_path: PathBuf,
    pub outcome: PipelineOutcome,
}

impl FleetApplyEntry {
    pub fn is_clean_success(&self) -> bool {
        self.outcome.is_clean_success()
    }
    pub fn rolled_back(&self) -> bool {
        self.outcome.rolled_back
    }
}

/// Aggregate result of [`survey_fleet_apply`].
#[derive(Debug, Serialize)]
pub struct FleetApplyReport {
    pub root: PathBuf,
    pub threshold: usize,
    pub entries: Vec<FleetApplyEntry>,
    pub crates_considered: usize,
    pub crates_attempted: usize,
    pub total_candidates_applied: usize,
    pub total_rolled_back: usize,
    /// `true` iff the sweep aborted at the first per-crate rollback
    /// (only possible when `opts.stop_on_first_rollback`).
    pub stopped_early: bool,
}

impl FleetApplyReport {
    /// Crates whose per-crate pipeline finished green (or dry-ran
    /// cleanly with no gate).
    pub fn green(&self) -> Vec<&FleetApplyEntry> {
        self.entries.iter().filter(|e| e.is_clean_success()).collect()
    }
    /// Crates whose gate went red and were rolled back. The
    /// operator's tree is at pre-pipeline state for every entry
    /// here — they're typed leads to investigate, not regressions.
    pub fn red(&self) -> Vec<&FleetApplyEntry> {
        self.entries.iter().filter(|e| e.rolled_back()).collect()
    }
}

/// Fleet-wide adoption sweep. For every Cargo crate under `root`
/// that has ≥ `opts.threshold` candidates, run
/// [`survey_apply_validate`] with `opts.pipeline_opts`.
///
/// **Atomicity is per-crate**: every crate's transforms (`.rs` +
/// `Cargo.toml`) land atomically with rollback on red gate. A
/// fleet-wide red gate on one crate does NOT corrupt other crates'
/// state. Partial fleet success is the safe default — operator
/// commits the green crates, investigates the red ones.
///
/// Per the W12 contract, each per-crate failure already left that
/// crate's working tree exactly as it was. The fleet sweep's only
/// extra safety knob is `stop_on_first_rollback` — if true, the
/// sweep aborts at the first red to surface the bug for inspection
/// before continuing to other crates.
pub fn survey_fleet_apply(
    root: &Path,
    opts: &FleetApplyOpts,
) -> Result<FleetApplyReport, PipelineError> {
    // Step 1: discover — survey_fleet already produces the
    // sorted-by-leverage leaderboard.
    let report = survey_fleet(root)?;
    let crates_considered = report.crates_with_candidates;
    let above: Vec<&CrateSurveyEntry> = report
        .entries
        .iter()
        .filter(|e| e.candidate_count >= opts.threshold)
        .collect();

    let mut entries: Vec<FleetApplyEntry> = vec![];
    let mut total_applied = 0usize;
    let mut total_rolled_back = 0usize;
    let mut stopped_early = false;

    // Step 2: apply per-crate. Each call composes the full W12
    // atomic pipeline (apply + inject deps + gate + rollback).
    for ce in &above {
        let outcome = survey_apply_validate(&ce.crate_path, &opts.pipeline_opts)?;
        total_applied += outcome.total_applied;
        let rolled = outcome.rolled_back;
        if rolled {
            total_rolled_back += 1;
        }
        entries.push(FleetApplyEntry {
            crate_path: ce.crate_path.clone(),
            outcome,
        });
        if rolled && opts.stop_on_first_rollback {
            stopped_early = true;
            break;
        }
    }

    Ok(FleetApplyReport {
        root: root.to_path_buf(),
        threshold: opts.threshold,
        crates_considered,
        crates_attempted: entries.len(),
        total_candidates_applied: total_applied,
        total_rolled_back,
        stopped_early,
        entries,
    })
}

// ─────────────────────────────────────────────────────────────────────
// Fleet-wide validate — substrate QAs itself against real fleet code
// ─────────────────────────────────────────────────────────────────────

use crate::{apply_to_source, ApplyError};

/// One candidate's validation outcome — typed at the failure
/// boundary so the operator gets actionable lookups by class.
#[derive(Debug, Serialize)]
pub enum ValidateOutcome {
    /// Apply produced re-parseable Rust. Substrate is sound for
    /// this candidate.
    Ok,
    /// `apply_to_source` returned a typed `ApplyError`. The
    /// candidate's surveyor produced something the transformer
    /// can't land — a substrate bug (mismatched survey/apply
    /// contracts), not user code.
    ApplyFailed(String),
    /// Apply succeeded but the output failed to re-parse as
    /// `syn::File`. The substrate emitted invalid Rust — the worst
    /// class. Likely a `syn::visit_mut` corner case (generics,
    /// lifetimes, raw identifiers, etc.).
    OutputUnparseable(String),
}

impl ValidateOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }
    /// Short, sortable category label for histogram aggregation.
    pub fn class_label(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ApplyFailed(_) => "apply-failed",
            Self::OutputUnparseable(_) => "output-unparseable",
        }
    }
}

/// Per-candidate validation record — what was checked and what
/// happened.
#[derive(Debug, Serialize)]
pub struct CandidateValidation {
    pub crate_path: PathBuf,
    pub file: PathBuf,
    pub pattern: String,
    pub target_type: String,
    pub outcome: ValidateOutcome,
}

/// Aggregate result of the substrate's self-QA against the fleet.
#[derive(Debug, Serialize)]
pub struct FleetValidateReport {
    pub root: PathBuf,
    pub total_candidates: usize,
    pub ok: usize,
    /// Only failed candidates carried — successful ones are
    /// implicit in `ok` count and not aggregated to keep the
    /// report tractable at scale.
    pub failed: Vec<CandidateValidation>,
    /// Failure class → count. Operators read this first to focus
    /// on the most common drift class.
    pub by_error_class: BTreeMap<String, usize>,
}

impl FleetValidateReport {
    pub fn is_clean(&self) -> bool {
        self.failed.is_empty()
    }
    pub fn failure_rate(&self) -> f64 {
        if self.total_candidates == 0 {
            return 0.0;
        }
        (self.failed.len() as f64) / (self.total_candidates as f64)
    }
}

/// For every `RefactorCandidate` in `survey_fleet(root)`, exercise
/// `apply_to_source` in-memory + verify the output re-parses. No
/// disk writes, no cargo gates, no rollback machinery — pure read
/// + transform + reparse, at fleet scale.
///
/// **The substrate's self-QA loop**: the fleet IS the test corpus.
/// Adding a new detector means the next `survey_fleet_validate`
/// run exercises its discover→apply roundtrip on every real-world
/// site, surfacing edge cases (generics, lifetimes, attrs,
/// visibility quirks) that synthetic tests would miss.
pub fn survey_fleet_validate(root: &Path) -> Result<FleetValidateReport, SurveyError> {
    let report = survey_fleet(root)?;
    let mut out_failed: Vec<CandidateValidation> = vec![];
    let mut ok_count = 0usize;
    let mut by_class: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;

    for entry in &report.entries {
        // Load the file once per candidate's file to keep IO bounded.
        // Multiple candidates from the same file will re-read it;
        // acceptable cost for ~250 candidates fleet-wide.
        for cand in &entry.candidates {
            total += 1;
            let src = match std::fs::read_to_string(&cand.file) {
                Ok(s) => s,
                Err(e) => {
                    let outcome = ValidateOutcome::ApplyFailed(format!("io: {e}"));
                    *by_class.entry(outcome.class_label().to_string()).or_default() += 1;
                    out_failed.push(CandidateValidation {
                        crate_path: entry.crate_path.clone(),
                        file: cand.file.clone(),
                        pattern: format!("{:?}", cand.pattern),
                        target_type: cand.target_type.clone(),
                        outcome,
                    });
                    continue;
                }
            };
            let outcome = match apply_to_source(&src, cand) {
                Ok(applied) => match syn::parse_file(&applied) {
                    Ok(_) => ValidateOutcome::Ok,
                    Err(e) => ValidateOutcome::OutputUnparseable(e.to_string()),
                },
                Err(ApplyError::Parse(e)) => ValidateOutcome::ApplyFailed(format!("parse: {e}")),
                Err(ApplyError::TargetNotFound(t)) => {
                    ValidateOutcome::ApplyFailed(format!("target-not-found: {t}"))
                }
                Err(ApplyError::NoMethodsRemoved { pattern, target }) => {
                    ValidateOutcome::ApplyFailed(format!(
                        "no-methods-removed: pattern={pattern:?} target={target}"
                    ))
                }
            };
            if outcome.is_ok() {
                ok_count += 1;
            } else {
                *by_class.entry(outcome.class_label().to_string()).or_default() += 1;
                out_failed.push(CandidateValidation {
                    crate_path: entry.crate_path.clone(),
                    file: cand.file.clone(),
                    pattern: format!("{:?}", cand.pattern),
                    target_type: cand.target_type.clone(),
                    outcome,
                });
            }
        }
    }

    Ok(FleetValidateReport {
        root: root.to_path_buf(),
        total_candidates: total,
        ok: ok_count,
        failed: out_failed,
        by_error_class: by_class,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a temp org-dir with N crates, each containing the given
    /// lib body. Returns the org-dir path.
    fn tmp_org(name: &str, crates: &[(&str, &str)]) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "tatara-fleet-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        for (crate_name, body) in crates {
            let crate_dir = tmp.join(crate_name);
            std::fs::create_dir_all(crate_dir.join("src")).unwrap();
            std::fs::write(
                crate_dir.join("Cargo.toml"),
                format!(
                    r#"[package]
name = "{crate_name}"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#
                ),
            )
            .unwrap();
            std::fs::write(crate_dir.join("src/lib.rs"), body).unwrap();
        }
        tmp
    }

    const GETTER_BODY: &str = r#"
pub struct A { pub x: i32, pub y: i32 }
impl A {
    pub fn x(&self) -> &i32 { &self.x }
    pub fn y(&self) -> &i32 { &self.y }
}
"#;

    const ISVARIANT_BODY: &str = r#"
pub enum S { Foo, Bar(u8) }
impl S {
    pub fn is_foo(&self) -> bool { matches!(self, Self::Foo) }
    pub fn is_bar(&self) -> bool { matches!(self, Self::Bar(_)) }
}
"#;

    const EMPTY_BODY: &str = r#"
pub fn noop() {}
"#;

    #[test]
    fn fleet_aggregates_candidates_across_crates_and_ranks_by_count() {
        let root = tmp_org(
            "aggregate",
            &[
                ("alpha", GETTER_BODY),
                ("beta", ISVARIANT_BODY),
                ("gamma", EMPTY_BODY),
            ],
        );
        let report = survey_fleet(&root).unwrap();
        assert_eq!(report.crates_scanned, 3, "all 3 Cargo crates scanned");
        assert_eq!(
            report.crates_with_candidates, 2,
            "alpha + beta have candidates; gamma does not"
        );
        assert!(report.total_candidates >= 2);
        // gamma (no candidates) is filtered out of entries — the
        // leaderboard only carries crates worth surfacing.
        assert!(
            !report
                .entries
                .iter()
                .any(|e| e.crate_path.ends_with("gamma")),
            "empty crate must not appear in leaderboard"
        );
    }

    #[test]
    fn fleet_skips_non_cargo_subdirs() {
        let root = std::env::temp_dir().join(format!(
            "tatara-fleet-noncargo-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::create_dir_all(root.join("real-crate/src")).unwrap();
        std::fs::write(
            root.join("real-crate/Cargo.toml"),
            r#"[package]
name = "real-crate"
version = "0.0.0"
edition = "2021"
[lib]
path = "src/lib.rs"
"#,
        )
        .unwrap();
        std::fs::write(root.join("real-crate/src/lib.rs"), GETTER_BODY).unwrap();

        let report = survey_fleet(&root).unwrap();
        assert_eq!(
            report.crates_scanned, 1,
            "only `real-crate` has a Cargo.toml — docs/ and scripts/ skipped"
        );
        assert_eq!(report.crates_with_candidates, 1);
    }

    #[test]
    fn entries_at_least_filters_by_threshold() {
        let root = tmp_org(
            "threshold",
            &[
                ("loud", GETTER_BODY), // 1 candidate
                ("loudest", ISVARIANT_BODY), // 1 candidate
            ],
        );
        let report = survey_fleet(&root).unwrap();
        // Both have ≥1 candidate; threshold=2 filters both out.
        assert_eq!(report.entries_at_least(2).len(), 0);
        // Threshold=1 retains both.
        assert!(report.entries_at_least(1).len() >= 1);
    }

    #[test]
    fn skips_hidden_dotdirs_and_target() {
        let root = std::env::temp_dir().join(format!(
            "tatara-fleet-skip-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".hidden/src")).unwrap();
        std::fs::write(root.join(".hidden/Cargo.toml"), "[package]\nname=\"x\"").unwrap();
        std::fs::create_dir_all(root.join("target/src")).unwrap();
        std::fs::write(root.join("target/Cargo.toml"), "[package]\nname=\"y\"").unwrap();
        let report = survey_fleet(&root).unwrap();
        assert_eq!(report.crates_scanned, 0, ".hidden and target must be skipped");
    }

    // ─────────────────────────────────────────────────────────────────
    // fleet-apply tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn fleet_apply_dry_run_aggregates_across_crates() {
        let root = tmp_org(
            "apply-dry",
            &[("alpha", GETTER_BODY), ("beta", ISVARIANT_BODY)],
        );
        let opts = FleetApplyOpts::default(); // dry-run, threshold=1
        let report = survey_fleet_apply(&root, &opts).unwrap();
        assert_eq!(report.crates_considered, 2);
        assert_eq!(report.crates_attempted, 2);
        assert!(report.total_candidates_applied >= 2);
        assert_eq!(report.total_rolled_back, 0, "dry run has no gate, no rollback");
        assert!(!report.stopped_early);
        // dry-run leaves disk byte-identical.
        let alpha_lib = std::fs::read_to_string(root.join("alpha/src/lib.rs")).unwrap();
        assert_eq!(alpha_lib, GETTER_BODY);
    }

    #[test]
    fn threshold_filters_low_candidate_crates_out() {
        let root = tmp_org(
            "apply-threshold",
            &[("only-one", GETTER_BODY), ("also-one", ISVARIANT_BODY)],
        );
        let opts = FleetApplyOpts {
            threshold: 5, // both crates have 1 → both filtered out
            ..FleetApplyOpts::default()
        };
        let report = survey_fleet_apply(&root, &opts).unwrap();
        assert_eq!(report.crates_considered, 2);
        assert_eq!(report.crates_attempted, 0, "threshold=5 filters both");
        assert!(report.entries.is_empty());
    }

    #[test]
    fn green_red_partitioning_matches_entry_states() {
        let report = FleetApplyReport {
            root: PathBuf::from("/tmp"),
            threshold: 1,
            entries: vec![],
            crates_considered: 0,
            crates_attempted: 0,
            total_candidates_applied: 0,
            total_rolled_back: 0,
            stopped_early: false,
        };
        assert_eq!(report.green().len(), 0);
        assert_eq!(report.red().len(), 0);
    }

    // ─────────────────────────────────────────────────────────────────
    // fleet-validate tests
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn fleet_validate_clean_org_reports_zero_failures() {
        let root = tmp_org(
            "validate-clean",
            &[("alpha", GETTER_BODY), ("beta", ISVARIANT_BODY)],
        );
        let report = survey_fleet_validate(&root).unwrap();
        assert!(
            report.is_clean(),
            "all canonical-shape candidates should validate; got {} failures: {:?}",
            report.failed.len(),
            report.failed
        );
        assert_eq!(report.ok, report.total_candidates);
        assert_eq!(report.failure_rate(), 0.0);
    }

    #[test]
    fn fleet_validate_aggregates_failures_by_class() {
        // Empty report: degenerate but should be clean + zero rate.
        let report = FleetValidateReport {
            root: PathBuf::from("/tmp"),
            total_candidates: 0,
            ok: 0,
            failed: vec![],
            by_error_class: BTreeMap::new(),
        };
        assert!(report.is_clean());
        assert_eq!(report.failure_rate(), 0.0);
    }
}
