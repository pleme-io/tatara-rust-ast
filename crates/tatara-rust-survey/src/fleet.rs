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

use crate::{survey_tree, MatchedPattern, RefactorCandidate, SurveyError};

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

fn pattern_label(p: MatchedPattern) -> String {
    match p {
        MatchedPattern::GetterAll => "GetterAll",
        MatchedPattern::SetterAll => "SetterAll",
        MatchedPattern::WithBuilder => "WithBuilder",
        MatchedPattern::IsVariant => "IsVariant",
    }
    .to_string()
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
}
