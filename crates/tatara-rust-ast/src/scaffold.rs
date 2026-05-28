//! `CrateScaffold` — typed value representing a complete Cargo crate
//! ready to be written to disk. Macro-shape specs (derive / attr /
//! fn-like) all materialize into this.
//!
//! Two write modes:
//! - `write_to(path)` — emit the entire scaffold to a directory.
//! - `to_files()` — yield the in-memory file map for snapshot testing.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub contents: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateScaffold {
    /// Crate name (`[package].name`). Hyphenated form; rustc replaces
    /// `-` → `_` to derive the macro name.
    pub name: String,
    pub version: String,
    /// One file per entry; written verbatim under `<root>/<path>`.
    pub files: Vec<FileEntry>,
}

impl CrateScaffold {
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            files: vec![],
        }
    }

    pub fn add_file(&mut self, path: impl Into<String>, contents: impl Into<String>) {
        self.files.push(FileEntry {
            path: path.into(),
            contents: contents.into(),
        });
    }

    /// Write the scaffold to disk under `root`. Creates parents.
    pub fn write_to(&self, root: &Path) -> std::io::Result<()> {
        for f in &self.files {
            let p = root.join(&f.path);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&p, &f.contents)?;
        }
        Ok(())
    }

    /// Path → contents map. Stable iteration via `IndexMap`. Useful for
    /// snapshot tests (`insta::assert_yaml_snapshot!`).
    #[must_use]
    pub fn to_files(&self) -> IndexMap<String, String> {
        self.files
            .iter()
            .map(|f| (f.path.clone(), f.contents.clone()))
            .collect()
    }
}
