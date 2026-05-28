//! `cargo_deps` — format-preserving Cargo.toml dep injection.
//!
//! When `apply` lands `#[derive(GetterAll)]` + `use
//! pleme_getter_derive::GetterAll` in a `.rs` file, the crate's
//! `Cargo.toml` also needs to declare `pleme-getter-derive` in
//! `[dependencies]` or `cargo build` will fail. This module closes
//! that gap.
//!
//! Powered by `toml_edit` — preserves comments, spacing, and
//! existing dep order. Idempotent: if the dep is already present
//! (under any source — git, crates.io, path) we leave it alone.
//!
//! All pleme-io farm derive crates live on GitHub at
//! `github.com/pleme-io/<crate>`. They are not yet on crates.io,
//! so the canonical dep source is the git URL on the main branch.
//! The default [`DepSource`] reflects that.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum CargoDepsError {
    #[error("read {path}: {err}")]
    Read { path: PathBuf, err: std::io::Error },
    #[error("write {path}: {err}")]
    Write { path: PathBuf, err: std::io::Error },
    #[error("parse {path}: {err}")]
    Parse {
        path: PathBuf,
        err: toml_edit::TomlError,
    },
    #[error("{path}: [dependencies] is not a table — Cargo.toml is malformed")]
    DepsNotATable { path: PathBuf },
}

/// How a dep should appear in `[dependencies]`. The default —
/// [`DepSource::PlemeIoGit`] — produces
/// `{ git = "https://github.com/pleme-io/<crate>.git", branch = "main" }`
/// which is the canonical pleme-io farm-derive consumption shape.
#[derive(Clone, Debug, Serialize)]
pub enum DepSource {
    /// `{ git = "https://github.com/pleme-io/<crate>.git", branch = "main" }`
    PlemeIoGit,
    /// `"*"` — pulled from crates.io. Use only for crates that have
    /// landed on crates.io.
    CratesIoStar,
    /// `<version-string>` — pulled from crates.io at a pinned version.
    CratesIoVersion(String),
}

impl Default for DepSource {
    fn default() -> Self {
        Self::PlemeIoGit
    }
}

impl DepSource {
    fn into_toml_value(self, crate_name: &str) -> toml_edit::Item {
        match self {
            Self::PlemeIoGit => {
                let mut tbl = toml_edit::InlineTable::new();
                tbl.insert(
                    "git",
                    format!("https://github.com/pleme-io/{crate_name}.git").into(),
                );
                tbl.insert("branch", "main".into());
                toml_edit::Item::Value(toml_edit::Value::InlineTable(tbl))
            }
            Self::CratesIoStar => toml_edit::Item::Value("*".into()),
            Self::CratesIoVersion(v) => toml_edit::Item::Value(v.into()),
        }
    }
}

/// Result of one dep-injection pass.
#[derive(Debug, Default, Serialize)]
pub struct InjectOutcome {
    /// Crates that were added to `[dependencies]` this pass.
    pub added: Vec<String>,
    /// Crates that were already present — left untouched.
    pub already_present: Vec<String>,
}

impl InjectOutcome {
    pub fn changed(&self) -> bool {
        !self.added.is_empty()
    }
}

/// Inject each crate in `crate_names` into `[dependencies]` of the
/// `Cargo.toml` at `cargo_toml_path`. Idempotent. Writes back to
/// disk iff at least one dep was actually added.
///
/// Returns the typed [`InjectOutcome`] so the pipeline can report
/// what changed + drive rollback (the caller backs up `Cargo.toml`
/// only when `outcome.changed()`).
pub fn inject_deps(
    cargo_toml_path: &Path,
    crate_names: &[&str],
    source: DepSource,
) -> Result<InjectOutcome, CargoDepsError> {
    let text = std::fs::read_to_string(cargo_toml_path).map_err(|err| CargoDepsError::Read {
        path: cargo_toml_path.to_path_buf(),
        err,
    })?;
    let mut doc: toml_edit::DocumentMut =
        text.parse().map_err(|err| CargoDepsError::Parse {
            path: cargo_toml_path.to_path_buf(),
            err,
        })?;

    // Ensure [dependencies] exists.
    if doc.get("dependencies").is_none() {
        doc.insert("dependencies", toml_edit::Item::Table(toml_edit::Table::new()));
    }
    let deps = doc.get_mut("dependencies").expect("just inserted").as_table_mut().ok_or_else(
        || CargoDepsError::DepsNotATable {
            path: cargo_toml_path.to_path_buf(),
        },
    )?;

    let mut outcome = InjectOutcome::default();
    // Dedup the input — survey can produce the same derive_crate
    // across multiple candidates in one crate.
    let mut sorted: Vec<&&str> = crate_names.iter().collect();
    sorted.sort();
    sorted.dedup();

    for crate_name in sorted {
        if deps.contains_key(crate_name) {
            outcome.already_present.push((*crate_name).to_string());
            continue;
        }
        let value = source.clone().into_toml_value(crate_name);
        deps.insert(crate_name, value);
        outcome.added.push((*crate_name).to_string());
    }

    if outcome.changed() {
        std::fs::write(cargo_toml_path, doc.to_string()).map_err(|err| CargoDepsError::Write {
            path: cargo_toml_path.to_path_buf(),
            err,
        })?;
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_cargo(name: &str, body: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "tatara-cargo-deps-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("Cargo.toml");
        std::fs::write(&path, body).unwrap();
        path
    }

    const BASELINE: &str = r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"
"#;

    #[test]
    fn injects_new_dep_with_pleme_io_git_default() {
        let path = tmp_cargo("git-default", BASELINE);
        let out = inject_deps(
            &path,
            &["pleme-getter-derive", "pleme-isvariant-derive"],
            DepSource::PlemeIoGit,
        )
        .unwrap();
        assert_eq!(out.added.len(), 2);
        assert!(out.already_present.is_empty());

        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("pleme-getter-derive"));
        assert!(after.contains("pleme-isvariant-derive"));
        assert!(after.contains("github.com/pleme-io/pleme-getter-derive.git"));
        assert!(after.contains("branch = \"main\""));
        // serve preserved.
        assert!(after.contains(r#"serde = "1""#));
    }

    #[test]
    fn idempotent_when_dep_already_present() {
        let with_existing = r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"

[dependencies]
pleme-getter-derive = { git = "https://github.com/pleme-io/pleme-getter-derive.git", branch = "main" }
"#;
        let path = tmp_cargo("idempotent", with_existing);
        let out = inject_deps(&path, &["pleme-getter-derive"], DepSource::PlemeIoGit).unwrap();
        assert!(out.added.is_empty());
        assert_eq!(out.already_present, vec!["pleme-getter-derive".to_string()]);
        // File on disk unchanged byte-for-byte.
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, with_existing);
    }

    #[test]
    fn creates_dependencies_table_when_missing() {
        let no_deps_table = r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"
"#;
        let path = tmp_cargo("no-deps-table", no_deps_table);
        let out = inject_deps(&path, &["pleme-getter-derive"], DepSource::PlemeIoGit).unwrap();
        assert_eq!(out.added, vec!["pleme-getter-derive".to_string()]);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("[dependencies]"));
        assert!(after.contains("pleme-getter-derive"));
    }

    #[test]
    fn dedups_duplicate_input_crates() {
        let path = tmp_cargo("dedup", BASELINE);
        let out = inject_deps(
            &path,
            &["pleme-getter-derive", "pleme-getter-derive", "pleme-getter-derive"],
            DepSource::PlemeIoGit,
        )
        .unwrap();
        assert_eq!(out.added.len(), 1, "duplicates collapse to one add");
    }

    #[test]
    fn crates_io_version_source_works() {
        let path = tmp_cargo("crates-io-version", BASELINE);
        let out = inject_deps(
            &path,
            &["thiserror"],
            DepSource::CratesIoVersion("2".into()),
        )
        .unwrap();
        assert_eq!(out.added, vec!["thiserror".to_string()]);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains(r#"thiserror = "2""#));
    }

    #[test]
    fn returns_no_changes_when_all_deps_present() {
        let all_present = r#"[package]
name = "demo"
version = "0.1.0"
edition = "2021"

[dependencies]
pleme-getter-derive = "*"
pleme-setter-derive = "*"
"#;
        let path = tmp_cargo("all-present", all_present);
        let out = inject_deps(
            &path,
            &["pleme-getter-derive", "pleme-setter-derive"],
            DepSource::PlemeIoGit,
        )
        .unwrap();
        assert!(!out.changed(), "no deps added → no write");
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, all_present);
    }
}
