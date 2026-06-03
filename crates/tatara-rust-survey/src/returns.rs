//! `returns` — diminishing-returns economics for the macro farm.
//!
//! The survey tells you **what** is adoptable. The economics tell you
//! **whether to keep farming**. This module turns a typed
//! [`FleetSurveyReport`] (plus operator estimates for clusters the
//! survey can't yet see) into a per-pattern lift/adopt/stop decision
//! and a single fleet-level verdict — the load-bearing governor the
//! operator consults to decide when refactoring has plateaued and
//! it's time to pivot to documentation / codegen.
//!
//! ```text
//! benefit(P) = candidates(P) × loc_per_candidate     (toil deleted)
//! cost(P)    = lift_cost(readiness(P))                (one-time substrate)
//! roi(P)     = benefit(P) / cost(P)
//!
//! decision(P):
//!   benefit < MIN_BENEFIT          → Defer   (cluster too small to churn repos)
//!   roi     < MIN_ROI              → Stop    (building it costs more than it saves)
//!   else                          → Harvest (adopt now)
//!
//! verdict(fleet):
//!   any Harvest                    → ContinueFarming
//!   none                          → Plateau  (stop refactoring; pivot to docs)
//! ```
//!
//! The metric's honesty comes from reasoning about **two** populations:
//!
//!   1. **Adoptable** patterns — every [`MatchedPattern`] in the
//!      detector registry has both a published emitter and a survey
//!      detector, so the typed survey measures them exactly. Lifting
//!      one is just running `survey-fleet-apply` (cost ≈ `adopt_cost`).
//!
//!   2. **Frontier** clusters — repeating-impl shapes the survey can't
//!      see yet (no detector, maybe no emitter). The operator supplies
//!      an estimate (grep proxy, prior fleet census); the model prices
//!      the one-time detector/emitter build against that estimate. This
//!      is where the metric earns its keep: it is the principled basis
//!      for the decision **not** to build a new derive when the cluster
//!      it would harvest is too small to repay the substrate.
//!
//! No I/O, no writes — a pure scoring function over a survey report.

use serde::Serialize;
use std::collections::BTreeMap;

use crate::fleet::FleetSurveyReport;

/// Substrate-readiness of a repeating-impl pattern — the dominant term
/// in its one-time lift cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Readiness {
    /// Detector **and** emitter both ship. Adopting fleet-wide is a
    /// single `survey-fleet-apply` run — no new substrate.
    Adoptable,
    /// The published derive (emitter) ships, but the survey has no
    /// detector for it: the fleet can be neither measured nor
    /// auto-adopted until a detector is written.
    NeedsDetector,
    /// Neither emitter nor detector exists — a brand-new spec kind plus
    /// its detector. The full price of an earned substrate primitive.
    NeedsEmitter,
}

/// Tunable one-time lift costs, expressed in **toil-equivalent LOC** —
/// the same unit `estimated_loc_saved` is denominated in — so benefit
/// and cost share one scale and `roi` is dimensionless.
///
/// Defaults are grounded in the macro-farm's own authoring history
/// (see `docs/pleme-io-docs/macro-farm.md`, the mado dogfood loop):
/// a new emitter primitive is `Spec` struct + `CompileToCrate` +
/// `Validate` + `SpecKind` + catalog row + repo-emission-matrix row +
/// consumer-verify renderer — call it ~140 LOC of substrate that must
/// be authored once and maintained forever. A new detector is one
/// `MatchedPattern` variant + a zero-sized `Detector` impl + a
/// `canonical_example` — call it ~60. Adopting an already-instrumented
/// pattern is the operator-review cost of reading one `survey-fleet-apply`
/// diff — call it ~8. Override any of these to re-price the fleet.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct LiftCostModel {
    /// One-time cost to adopt an already-instrumented pattern.
    pub adopt_cost: usize,
    /// Incremental cost of authoring a new survey detector.
    pub detector_cost: usize,
    /// Incremental cost of authoring a new emitter (spec kind).
    pub emitter_cost: usize,
    /// Benefit floor (LOC). Below this a cluster isn't worth the
    /// cross-repo churn even when the substrate already exists.
    pub min_benefit: usize,
    /// Break-even ROI. `1.0` means "only build it if it deletes at
    /// least as much toil as it costs to build and maintain."
    pub min_roi: f64,
}

impl Default for LiftCostModel {
    fn default() -> Self {
        Self {
            adopt_cost: 8,
            detector_cost: 60,
            emitter_cost: 140,
            min_benefit: 30,
            min_roi: 1.0,
        }
    }
}

impl LiftCostModel {
    /// One-time lift cost for a pattern at the given readiness. Each
    /// rung adds the work the previous one didn't have to do.
    pub fn lift_cost(&self, r: Readiness) -> usize {
        match r {
            Readiness::Adoptable => self.adopt_cost,
            Readiness::NeedsDetector => self.adopt_cost + self.detector_cost,
            Readiness::NeedsEmitter => self.adopt_cost + self.detector_cost + self.emitter_cost,
        }
    }
}

/// Per-pattern lift/adopt/stop decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    /// Benefit clears the floor and ROI clears break-even — adopt now.
    Harvest,
    /// Benefit below the floor — the cluster is real but too small to
    /// justify churning repos. Revisit when it grows.
    Defer,
    /// ROI below break-even — building the substrate to harvest this
    /// cluster would cost more toil than it deletes. Do not build it.
    Stop,
}

/// One pattern's full economics row.
#[derive(Clone, Debug, Serialize)]
pub struct PatternEconomics {
    /// Derive trait name (the canonical pattern label, e.g. `GetterAll`).
    pub pattern: String,
    /// Published derive crate that adopts this pattern.
    pub derive_crate: String,
    pub readiness: Readiness,
    /// Sites found (survey) or estimated (frontier).
    pub candidates: usize,
    /// Toil deleted by adopting every site — the benefit term.
    pub loc_saved: usize,
    /// One-time substrate cost to make this pattern harvestable.
    pub lift_cost: usize,
    /// `loc_saved / lift_cost`, rounded to 2 dp for the report.
    pub roi: f64,
    pub decision: Decision,
}

/// An un-instrumented frontier cluster the operator measured by other
/// means (grep proxy, a prior whole-fleet census). Lets the metric
/// price patterns the typed survey can't yet see.
#[derive(Clone, Debug)]
pub struct FrontierEstimate {
    /// Pattern label (the would-be derive trait name).
    pub pattern: &'static str,
    /// The would-be / existing derive crate.
    pub derive_crate: &'static str,
    pub readiness: Readiness,
    pub estimated_candidates: usize,
    /// Toil deleted per site if adopted (LOC).
    pub loc_per_candidate: usize,
}

/// The fleet-level verdict — the signal the operator's loop branches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FleetVerdict {
    /// At least one pattern is `Harvest` — keep farming.
    ContinueFarming,
    /// No pattern clears the bar — refactoring has plateaued. Stop
    /// refactoring and pivot to documentation / codegen.
    Plateau,
}

/// Aggregate diminishing-returns report over a fleet survey.
#[derive(Debug, Serialize)]
pub struct FleetReturnsReport {
    /// The cost model used — echoed so a JSON report is self-describing.
    pub model: LiftCostModel,
    /// Every scored pattern, sorted by benefit (desc).
    pub patterns: Vec<PatternEconomics>,
    /// Sum of `loc_saved` over patterns whose decision is `Harvest`.
    pub total_harvestable_loc: usize,
    /// Count of patterns at each decision — the histogram the operator
    /// reads first.
    pub decision_counts: BTreeMap<String, usize>,
    pub verdict: FleetVerdict,
}

impl FleetReturnsReport {
    /// Patterns the operator should adopt now, highest-benefit first.
    pub fn harvest(&self) -> Vec<&PatternEconomics> {
        self.patterns
            .iter()
            .filter(|p| p.decision == Decision::Harvest)
            .collect()
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn score_one(
    pattern: String,
    derive_crate: String,
    readiness: Readiness,
    candidates: usize,
    loc_saved: usize,
    model: &LiftCostModel,
) -> PatternEconomics {
    let lift_cost = model.lift_cost(readiness).max(1);
    let roi = loc_saved as f64 / lift_cost as f64;
    let decision = if loc_saved < model.min_benefit {
        Decision::Defer
    } else if roi < model.min_roi {
        Decision::Stop
    } else {
        Decision::Harvest
    };
    PatternEconomics {
        pattern,
        derive_crate,
        readiness,
        candidates,
        loc_saved,
        lift_cost,
        roi: round2(roi),
        decision,
    }
}

/// Score a fleet survey plus a set of frontier estimates into a
/// diminishing-returns report. Pure function — no I/O.
///
/// Adoptable patterns are read straight from the survey (every pattern
/// the survey can produce is, by registry construction, `Adoptable`).
/// Frontier estimates are appended and priced at their stated readiness.
/// A frontier estimate whose `pattern` already appears in the survey is
/// merged into that row (survey count is authoritative; the estimate's
/// readiness is ignored in favour of the survey's `Adoptable`).
pub fn fleet_returns(
    report: &FleetSurveyReport,
    frontier: &[FrontierEstimate],
    model: &LiftCostModel,
) -> FleetReturnsReport {
    // 1. Aggregate Adoptable patterns out of the survey's candidates.
    let mut agg: BTreeMap<String, (String, usize, usize)> = BTreeMap::new(); // pattern → (crate, count, loc)
    for entry in &report.entries {
        for c in &entry.candidates {
            let key = c.derive_trait.to_string();
            let slot = agg
                .entry(key)
                .or_insert_with(|| (c.derive_crate.to_string(), 0, 0));
            slot.1 += 1;
            slot.2 += c.estimated_loc_saved;
        }
    }

    let mut patterns: Vec<PatternEconomics> = agg
        .into_iter()
        .map(|(pattern, (derive_crate, count, loc))| {
            score_one(pattern, derive_crate, Readiness::Adoptable, count, loc, model)
        })
        .collect();

    let surveyed: std::collections::BTreeSet<String> =
        patterns.iter().map(|p| p.pattern.clone()).collect();

    // 2. Fold in frontier estimates the survey can't see.
    for f in frontier {
        if surveyed.contains(f.pattern) {
            continue; // survey count is authoritative for instrumented patterns
        }
        let loc = f.estimated_candidates * f.loc_per_candidate;
        patterns.push(score_one(
            f.pattern.to_string(),
            f.derive_crate.to_string(),
            f.readiness,
            f.estimated_candidates,
            loc,
            model,
        ));
    }

    // 3. Sort by benefit, then ROI, then name — stable leaderboard.
    patterns.sort_by(|a, b| {
        b.loc_saved
            .cmp(&a.loc_saved)
            .then(b.roi.partial_cmp(&a.roi).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.pattern.cmp(&b.pattern))
    });

    let mut decision_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total_harvestable_loc = 0usize;
    for p in &patterns {
        let label = match p.decision {
            Decision::Harvest => "harvest",
            Decision::Defer => "defer",
            Decision::Stop => "stop",
        };
        *decision_counts.entry(label.to_string()).or_default() += 1;
        if p.decision == Decision::Harvest {
            total_harvestable_loc += p.loc_saved;
        }
    }

    let verdict = if patterns.iter().any(|p| p.decision == Decision::Harvest) {
        FleetVerdict::ContinueFarming
    } else {
        FleetVerdict::Plateau
    };

    FleetReturnsReport {
        model: *model,
        patterns,
        total_harvestable_loc,
        decision_counts,
        verdict,
    }
}

/// The documented frontier census for the **public first-party** Rust
/// surface, measured 2026-06-03 by precise body-shape grep over 228
/// public pleme-io libraries (1790 source files). These are the
/// repeating-impl shapes the typed survey cannot yet see — the newtype
/// trait-impl delegation family and the stringly-typed round-trip combo.
///
/// The measured count of mechanical newtype delegations
/// (`as_ref/deref/borrow/Display { &self.0 } / Self(..)`) was **zero**:
/// the macro farm's newtype derives are published but the public
/// first-party surface has no remaining hand-written sites to harvest.
/// Folding these into the metric is what lets it conclude — with a
/// number, not a hunch — that authoring newtype **detectors** would
/// repay nothing. Re-measure and update these as the fleet grows.
pub fn first_party_frontier_2026_06() -> Vec<FrontierEstimate> {
    vec![
        FrontierEstimate {
            pattern: "AsRefNewtype",
            derive_crate: "pleme-asref-derive",
            readiness: Readiness::NeedsDetector,
            estimated_candidates: 0,
            loc_per_candidate: 3,
        },
        FrontierEstimate {
            pattern: "DerefNewtype",
            derive_crate: "pleme-deref-derive",
            readiness: Readiness::NeedsDetector,
            estimated_candidates: 0,
            loc_per_candidate: 3,
        },
        FrontierEstimate {
            pattern: "DisplayNewtype",
            derive_crate: "pleme-display-derive",
            readiness: Readiness::NeedsDetector,
            estimated_candidates: 0,
            loc_per_candidate: 3,
        },
        FrontierEstimate {
            pattern: "ImplFrom",
            derive_crate: "pleme-implfrom-derive",
            readiness: Readiness::NeedsDetector,
            estimated_candidates: 0,
            loc_per_candidate: 5,
        },
        // Stringly-typed round-trip (FromStr+TryFrom+Display match table):
        // 25 `impl FromStr` sites exist but they are real parsers, not
        // mechanical unit-enum tables — no emitter exists and the cluster
        // does not justify one. Priced at NeedsEmitter to show the metric
        // rejecting it on ROI even at a generous count.
        FrontierEstimate {
            pattern: "StringEnum",
            derive_crate: "pleme-stringenum-derive",
            readiness: Readiness::NeedsEmitter,
            estimated_candidates: 0,
            loc_per_candidate: 8,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::survey_fleet;
    use std::path::PathBuf;

    fn tmp_org(name: &str, crates: &[(&str, &str)]) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("tatara-returns-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        for (cn, body) in crates {
            let d = tmp.join(cn);
            std::fs::create_dir_all(d.join("src")).unwrap();
            std::fs::write(
                d.join("Cargo.toml"),
                format!("[package]\nname = \"{cn}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[lib]\npath = \"src/lib.rs\"\n"),
            )
            .unwrap();
            std::fs::write(d.join("src/lib.rs"), body).unwrap();
        }
        tmp
    }

    // A struct with 6 getters → benefit 30 LOC (6 × 5), Adoptable.
    const SIX_GETTERS: &str = r#"
pub struct A { pub a: i32, pub b: i32, pub c: i32, pub d: i32, pub e: i32, pub f: i32 }
impl A {
    pub fn a(&self) -> &i32 { &self.a }
    pub fn b(&self) -> &i32 { &self.b }
    pub fn c(&self) -> &i32 { &self.c }
    pub fn d(&self) -> &i32 { &self.d }
    pub fn e(&self) -> &i32 { &self.e }
    pub fn f(&self) -> &i32 { &self.f }
}
"#;

    #[test]
    fn adoptable_pattern_above_floor_is_harvest() {
        let root = tmp_org("harvest", &[("alpha", SIX_GETTERS)]);
        let report = survey_fleet(&root).unwrap();
        let model = LiftCostModel::default();
        let r = fleet_returns(&report, &[], &model);
        let getter = r
            .patterns
            .iter()
            .find(|p| p.pattern == "GetterAll")
            .expect("GetterAll present");
        assert_eq!(getter.readiness, Readiness::Adoptable);
        assert!(getter.loc_saved >= model.min_benefit, "30 LOC clears the floor");
        assert_eq!(getter.decision, Decision::Harvest);
        assert_eq!(r.verdict, FleetVerdict::ContinueFarming);
    }

    #[test]
    fn empty_frontier_cluster_never_harvests() {
        // No survey candidates + a zero-count NeedsEmitter frontier →
        // the metric must refuse to build it.
        let root = tmp_org("plateau", &[("empty", "pub fn noop() {}\n")]);
        let report = survey_fleet(&root).unwrap();
        let model = LiftCostModel::default();
        let frontier = vec![FrontierEstimate {
            pattern: "StringEnum",
            derive_crate: "pleme-stringenum-derive",
            readiness: Readiness::NeedsEmitter,
            estimated_candidates: 0,
            loc_per_candidate: 8,
        }];
        let r = fleet_returns(&report, &frontier, &model);
        let se = r.patterns.iter().find(|p| p.pattern == "StringEnum").unwrap();
        assert_eq!(se.decision, Decision::Defer, "0 sites → below benefit floor");
        assert_eq!(r.verdict, FleetVerdict::Plateau, "nothing to harvest → plateau");
    }

    #[test]
    fn frontier_cluster_below_breakeven_is_stop_not_harvest() {
        // A NeedsEmitter cluster with some sites but not enough to repay
        // a ~148 LOC build → Stop. 10 sites × 8 LOC = 80 benefit, cost
        // 148 → roi 0.54 < 1.0.
        let root = tmp_org("breakeven", &[("empty", "pub fn noop() {}\n")]);
        let report = survey_fleet(&root).unwrap();
        let model = LiftCostModel::default();
        let frontier = vec![FrontierEstimate {
            pattern: "StringEnum",
            derive_crate: "pleme-stringenum-derive",
            readiness: Readiness::NeedsEmitter,
            estimated_candidates: 10,
            loc_per_candidate: 8,
        }];
        let r = fleet_returns(&report, &frontier, &model);
        let se = r.patterns.iter().find(|p| p.pattern == "StringEnum").unwrap();
        assert!(se.loc_saved >= model.min_benefit, "80 LOC clears benefit floor");
        assert!(se.roi < model.min_roi, "but ROI below break-even");
        assert_eq!(se.decision, Decision::Stop);
    }

    #[test]
    fn documented_frontier_is_all_zero_and_pleme_plateaus_without_survey() {
        // The 2026-06 first-party census: every frontier cluster is
        // empty, so on its own it can never justify new substrate.
        let frontier = first_party_frontier_2026_06();
        assert!(frontier.iter().all(|f| f.estimated_candidates == 0));
    }
}
