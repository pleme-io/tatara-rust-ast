//! `tatara-rust-gate` — typed green-gate runner.
//!
//! Given a directory holding a generated Rust repo, runs the three
//! gates a publishable crate must pass:
//!   1. `cargo build`               (build)
//!   2. `cargo test`                (behavior)
//!   3. `cargo clippy -- -D warnings` (lints + format-ban etc.)
//!
//! Each gate captures stdout+stderr so failures land in the typed
//! `GateOutcome::Failed { gate, stdout, stderr, exit }` value. Batch
//! mode (`green_gate_batch`) fails fast — the first red gate aborts
//! and the per-repo outcomes returned so far are reported.
//!
//! No shell. No `bash -c`. Each gate is one `Command` invocation; the
//! whole runner stays in typed-Rust territory.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Gate {
    Build,
    Test,
    Clippy,
}

impl Gate {
    /// `(program, args)` for the gate. Returned as &str pairs so
    /// callers can `Command::new`/`.args(...)` directly.
    fn argv(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Build => ("cargo", &["build", "--workspace", "--all-targets"]),
            Self::Test => ("cargo", &["test", "--workspace", "--all-targets"]),
            Self::Clippy => (
                "cargo",
                &[
                    "clippy",
                    "--workspace",
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
            ),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Test => "test",
            Self::Clippy => "clippy",
        }
    }
}

/// Outcome for one repo's full gate sweep.
#[derive(Clone, Debug, Serialize)]
pub enum GateOutcome {
    Passed,
    Failed {
        gate: Gate,
        exit: Option<i32>,
        stdout: String,
        stderr: String,
    },
    SkippedNoCargo,
}

impl GateOutcome {
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::Passed)
    }
}

/// Which gates to run. Default is all three; tests may want a subset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateConfig {
    pub gates: Vec<Gate>,
    /// Skip with `GateOutcome::SkippedNoCargo` if no Cargo.toml at the
    /// repo root. Defaults to `true`.
    pub skip_if_no_cargo_toml: bool,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            gates: vec![Gate::Build, Gate::Test, Gate::Clippy],
            skip_if_no_cargo_toml: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GateError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Run every configured gate against `repo_root`. Captures output;
/// returns at the first failure.
pub fn green_gate(repo_root: &Path, cfg: &GateConfig) -> Result<GateOutcome, GateError> {
    if cfg.skip_if_no_cargo_toml && !repo_root.join("Cargo.toml").exists() {
        return Ok(GateOutcome::SkippedNoCargo);
    }
    for &gate in &cfg.gates {
        let (program, args) = gate.argv();
        let output: Output = Command::new(program)
            .args(args)
            .current_dir(repo_root)
            .output()?;
        if !output.status.success() {
            return Ok(GateOutcome::Failed {
                gate,
                exit: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
    }
    Ok(GateOutcome::Passed)
}

/// Result of a batch gate run. Stops at the first failure.
#[derive(Debug, Serialize)]
pub struct BatchOutcome {
    pub results: Vec<(PathBuf, GateOutcome)>,
    pub stopped_at_failure: bool,
}

impl BatchOutcome {
    pub fn all_passed(&self) -> bool {
        !self.stopped_at_failure && self.results.iter().all(|(_, o)| o.is_passed())
    }
    pub fn passed_count(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, o)| o.is_passed())
            .count()
    }
}

/// Run `green_gate` on every repo in `roots`. First failure aborts the
/// batch; results so far are returned with `stopped_at_failure = true`.
pub fn green_gate_batch(roots: &[PathBuf], cfg: &GateConfig) -> Result<BatchOutcome, GateError> {
    let mut results: Vec<(PathBuf, GateOutcome)> = vec![];
    for root in roots {
        let outcome = green_gate(root, cfg)?;
        let failed = matches!(outcome, GateOutcome::Failed { .. });
        results.push((root.clone(), outcome));
        if failed {
            return Ok(BatchOutcome {
                results,
                stopped_at_failure: true,
            });
        }
    }
    Ok(BatchOutcome {
        results,
        stopped_at_failure: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_labels_are_stable() {
        assert_eq!(Gate::Build.label(), "build");
        assert_eq!(Gate::Test.label(), "test");
        assert_eq!(Gate::Clippy.label(), "clippy");
    }

    #[test]
    fn argv_is_cargo() {
        for g in [Gate::Build, Gate::Test, Gate::Clippy] {
            let (program, args) = g.argv();
            assert_eq!(program, "cargo");
            assert!(!args.is_empty());
        }
    }

    #[test]
    fn skip_if_no_cargo_toml_returns_skipped() {
        let tmp = std::env::temp_dir().join(format!("tatara-gate-skip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // No Cargo.toml.
        let cfg = GateConfig::default();
        let outcome = green_gate(&tmp, &cfg).unwrap();
        assert!(matches!(outcome, GateOutcome::SkippedNoCargo));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn outcome_is_passed_inspects_variant() {
        assert!(GateOutcome::Passed.is_passed());
        assert!(!GateOutcome::SkippedNoCargo.is_passed());
        assert!(!(GateOutcome::Failed {
            gate: Gate::Build,
            exit: Some(1),
            stdout: String::new(),
            stderr: String::new(),
        })
        .is_passed());
    }

    #[test]
    fn batch_empty_input_passes() {
        let cfg = GateConfig::default();
        let b = green_gate_batch(&[], &cfg).unwrap();
        assert!(b.all_passed());
        assert_eq!(b.passed_count(), 0);
        assert!(!b.stopped_at_failure);
    }

    #[test]
    fn batch_all_skipped_still_all_passed_is_false() {
        // SkippedNoCargo is NOT a pass — flagged in `all_passed`.
        let tmp = std::env::temp_dir().join(format!(
            "tatara-gate-batch-skip-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let cfg = GateConfig::default();
        let b = green_gate_batch(&[tmp.clone()], &cfg).unwrap();
        assert!(!b.all_passed(), "skipped repos shouldn't count as passed");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
