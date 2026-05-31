//! `CargoManifest` — the typed, fleet-wide Cargo.toml emitter.
//!
//! **★★ TYPED EMISSION.** Cargo.toml is TOML *syntax*; composing it with
//! `format!()` is the banned free-form-string anti-pattern. This module
//! is the one typed surface every emitter in the farm renders manifests
//! through: build a [`CargoManifest`] value, then serialize it via
//! `toml::to_string` (the typed AST serializer — `toml::ser::Error`
//! already flows into [`crate::AstError`] via its `TomlSer` variant).
//!
//! Solved once, used everywhere:
//! - [`crate::render_proc_macro_cargo_toml`] builds the proc-macro shape.
//! - `tatara-rust-verify`'s consumer-verify crate builds the umbrella
//!   shape (one path dep per catalog entry + optional shikumi git dep).
//!
//! The value is `#[derive(Serialize)]` so `toml::to_string` emits the
//! canonical table ordering; `Dep` is an untagged enum so a simple
//! version string and a detailed `{ version, features, git, path,
//! proc-macro }` table both serialize to idiomatic TOML.

use indexmap::IndexMap;
use serde::Serialize;

use crate::AstError;

/// A `[dependencies]` value: either a bare version string
/// (`serde = "1"`) or a detailed inline table
/// (`syn = { version = "2", features = [...] }`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Dep {
    /// `name = "<version>"`.
    Simple(String),
    /// `name = { version = .., features = .., git = .., path = .. }`.
    Detailed(DetailedDep),
}

/// The detailed-dependency table. Every field is skipped when `None`
/// so the emitted TOML carries only the keys the caller set.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DetailedDep {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl DetailedDep {
    /// `{ path = "<p>" }` — the canonical workspace-member path dep.
    #[must_use]
    pub fn path(p: impl Into<String>) -> Self {
        Self {
            path: Some(p.into()),
            ..Self::default()
        }
    }

    /// `{ git = "<url>" }` — a bare git dep (default branch).
    #[must_use]
    pub fn git(url: impl Into<String>) -> Self {
        Self {
            git: Some(url.into()),
            ..Self::default()
        }
    }

    /// `{ version = "<v>", features = [...] }`.
    #[must_use]
    pub fn versioned(version: impl Into<String>, features: Vec<String>) -> Self {
        Self {
            version: Some(version.into()),
            features: if features.is_empty() {
                None
            } else {
                Some(features)
            },
            ..Self::default()
        }
    }
}

/// `[package]` table. Fields fixed by fleet policy for emitted crates
/// (edition 2024, MIT) plus the per-crate name/description.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub edition: String,
    pub license: String,
    pub description: String,
}

/// `[lib]` table. `proc-macro = true` for derive/attr crates; `path`
/// for the consumer-verify umbrella crate.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Lib {
    #[serde(rename = "proc-macro", skip_serializing_if = "Option::is_none")]
    pub proc_macro: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// A complete, serializable Cargo manifest. Serializes to a canonical
/// TOML document via [`Self::to_toml`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CargoManifest {
    pub package: Package,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lib: Option<Lib>,
    /// `IndexMap` preserves insertion order so the emitted TOML's
    /// dependency ordering is deterministic across runs.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub dependencies: IndexMap<String, Dep>,
}

impl CargoManifest {
    /// Construct an empty-dependency manifest for `package`/`lib`.
    #[must_use]
    pub fn new(package: Package, lib: Option<Lib>) -> Self {
        Self {
            package,
            lib,
            dependencies: IndexMap::new(),
        }
    }

    /// Insert one dependency, preserving call order.
    pub fn add_dep(&mut self, name: impl Into<String>, dep: Dep) {
        self.dependencies.insert(name.into(), dep);
    }

    /// Serialize to a canonical TOML document. The typed-TOML
    /// chokepoint — every emitted manifest renders through here, never
    /// through `format!()` of TOML syntax.
    ///
    /// # Errors
    /// Returns [`AstError::TomlSer`] if `toml::to_string` fails (e.g. a
    /// non-string map key, which the typed value can't construct).
    pub fn to_toml(&self) -> Result<String, AstError> {
        Ok(toml::to_string(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg() -> Package {
        Package {
            name: "foo-derive".into(),
            version: "0.1.0".into(),
            edition: "2024".into(),
            license: "MIT".into(),
            description: "Foo desc.".into(),
        }
    }

    #[test]
    fn round_trips_through_typed_toml() {
        let mut m = CargoManifest::new(
            pkg(),
            Some(Lib {
                proc_macro: Some(true),
                path: None,
            }),
        );
        m.add_dep("proc-macro2", Dep::Simple("1".into()));
        m.add_dep(
            "syn",
            Dep::Detailed(DetailedDep::versioned(
                "2",
                vec!["full".into(), "extra-traits".into()],
            )),
        );

        let rendered = m.to_toml().unwrap();
        // Parse the emitted TOML back into a typed value and assert on
        // the structure — never substring-match the syntax.
        let parsed: toml::Table = toml::from_str(&rendered).unwrap();
        assert_eq!(parsed["package"]["name"].as_str(), Some("foo-derive"));
        assert_eq!(parsed["package"]["edition"].as_str(), Some("2024"));
        assert_eq!(parsed["lib"]["proc-macro"].as_bool(), Some(true));
        assert_eq!(parsed["dependencies"]["proc-macro2"].as_str(), Some("1"));
        assert_eq!(
            parsed["dependencies"]["syn"]["version"].as_str(),
            Some("2")
        );
        let feats = parsed["dependencies"]["syn"]["features"]
            .as_array()
            .unwrap();
        assert_eq!(feats.len(), 2);
        assert_eq!(feats[0].as_str(), Some("full"));
    }

    #[test]
    fn detailed_dep_skips_unset_fields() {
        let mut m = CargoManifest::new(pkg(), None);
        m.add_dep("shikumi", Dep::Detailed(DetailedDep::git("https://x/y")));
        let parsed: toml::Table = toml::from_str(&m.to_toml().unwrap()).unwrap();
        let shik = &parsed["dependencies"]["shikumi"];
        assert_eq!(shik["git"].as_str(), Some("https://x/y"));
        // version / features / path not present.
        assert!(shik.get("version").is_none());
        assert!(shik.get("features").is_none());
        assert!(shik.get("path").is_none());
    }

    #[test]
    fn empty_dependencies_omits_section() {
        let m = CargoManifest::new(pkg(), None);
        let parsed: toml::Table = toml::from_str(&m.to_toml().unwrap()).unwrap();
        assert!(parsed.get("dependencies").is_none());
        assert!(parsed.get("lib").is_none());
    }

    #[test]
    fn output_is_byte_stable() {
        let a = CargoManifest::new(pkg(), None).to_toml().unwrap();
        let b = CargoManifest::new(pkg(), None).to_toml().unwrap();
        assert_eq!(a, b);
    }
}
