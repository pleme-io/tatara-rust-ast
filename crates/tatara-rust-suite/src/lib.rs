//! `tatara-rust-suite` — L5 bundle primitive.
//!
//! A `MacroSuiteSpec` packs N macro Specs of any kind (derive / proc-attr
//! / proc-fn / macro-rules) into one Cargo workspace ready to publish.
//! `compile_to_workspace(name)` produces a `WorkspaceScaffold` with a
//! root `Cargo.toml` listing every member and a `CrateScaffold` per
//! member crate.
//!
//! Authoring shape:
//!
//! ```
//! use tatara_rust_ast::{CompileToCrate, Ident};
//! use tatara_rust_derive::ProcDeriveSpec;
//! use tatara_rust_macro_rules::{MacroArm, MacroRulesSpec};
//! use tatara_rust_suite::{MacroMemberSpec, MacroSuiteSpec};
//!
//! let derive_spec = ProcDeriveSpec::new("Marker", vec![]);
//! let rules_spec = MacroRulesSpec {
//!     macro_name: Ident::new("identity"),
//!     arms: vec![MacroArm { matcher: "($x:expr)".into(), transcriber: "{ $x }".into() }],
//! };
//! let suite = MacroSuiteSpec {
//!     workspace_name: "my-macros".into(),
//!     members: vec![
//!         MacroMemberSpec::Derive {
//!             crate_name: "marker-derive".into(),
//!             spec: derive_spec,
//!         },
//!         MacroMemberSpec::MacroRules {
//!             crate_name: "identity-macros".into(),
//!             spec: rules_spec,
//!         },
//!     ],
//! };
//! let ws = suite.compile_to_workspace().unwrap();
//! assert_eq!(ws.member_crates.len(), 2);
//! assert!(ws.root_cargo_toml.contains("members"));
//! ```

use serde::{Deserialize, Serialize};
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold};
use tatara_rust_derive::ProcDeriveSpec;
use tatara_rust_macro_rules::MacroRulesSpec;
use tatara_rust_proc_attr::ProcAttrSpec;
use tatara_rust_proc_fn::ProcFnSpec;

/// A bundle of macro Specs plus their target crate names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacroSuiteSpec {
    pub workspace_name: String,
    pub members: Vec<MacroMemberSpec>,
}

/// Tagged enum — each variant carries the kind-specific Spec plus the
/// crate name to emit it under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MacroMemberSpec {
    Derive {
        crate_name: String,
        spec: ProcDeriveSpec,
    },
    ProcAttr {
        crate_name: String,
        spec: ProcAttrSpec,
    },
    ProcFn {
        crate_name: String,
        spec: ProcFnSpec,
    },
    MacroRules {
        crate_name: String,
        spec: MacroRulesSpec,
    },
}

impl MacroMemberSpec {
    pub fn crate_name(&self) -> &str {
        match self {
            Self::Derive { crate_name, .. }
            | Self::ProcAttr { crate_name, .. }
            | Self::ProcFn { crate_name, .. }
            | Self::MacroRules { crate_name, .. } => crate_name,
        }
    }

    pub fn compile(&self) -> Result<CrateScaffold, AstError> {
        match self {
            Self::Derive { crate_name, spec } => spec.compile_to_crate(crate_name),
            Self::ProcAttr { crate_name, spec } => spec.compile_to_crate(crate_name),
            Self::ProcFn { crate_name, spec } => spec.compile_to_crate(crate_name),
            Self::MacroRules { crate_name, spec } => spec.compile_to_crate(crate_name),
        }
    }
}

/// Output of `compile_to_workspace`: the root Cargo.toml + every
/// member scaffold. Writable as one tree via [`write_to`].
pub struct WorkspaceScaffold {
    pub workspace_name: String,
    pub root_cargo_toml: String,
    pub member_crates: Vec<CrateScaffold>,
}

impl WorkspaceScaffold {
    /// Write the entire workspace to disk under `root`. Creates the
    /// root `Cargo.toml` and every member crate's tree.
    pub fn write_to(&self, root: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        std::fs::write(root.join("Cargo.toml"), &self.root_cargo_toml)?;
        for m in &self.member_crates {
            let dir = root.join(format!("crates/{}", m.name));
            m.write_to(&dir)?;
        }
        Ok(())
    }
}

impl MacroSuiteSpec {
    /// Compile every member; assemble the workspace root + member dirs.
    pub fn compile_to_workspace(&self) -> Result<WorkspaceScaffold, AstError> {
        let mut member_crates = Vec::with_capacity(self.members.len());
        let mut member_paths = Vec::with_capacity(self.members.len());
        for m in &self.members {
            let scaffold = m.compile()?;
            member_paths.push(format!("crates/{}", scaffold.name));
            member_crates.push(scaffold);
        }
        let members_array = member_paths
            .iter()
            .map(|p| format!("  \"{p}\""))
            .collect::<Vec<_>>()
            .join(",\n");
        let root_cargo_toml = format!(
            r#"[workspace]
resolver = "2"
members = [
{members_array}
]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"

[workspace.lints.clippy]
pedantic = {{ level = "warn", priority = -1 }}
"#
        );
        Ok(WorkspaceScaffold {
            workspace_name: self.workspace_name.clone(),
            root_cargo_toml,
            member_crates,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_ast::Ident;
    use tatara_rust_macro_rules::MacroArm;

    fn sample() -> MacroSuiteSpec {
        MacroSuiteSpec {
            workspace_name: "my-macros".into(),
            members: vec![
                MacroMemberSpec::Derive {
                    crate_name: "marker-derive".into(),
                    spec: ProcDeriveSpec::new("Marker", vec![]),
                },
                MacroMemberSpec::MacroRules {
                    crate_name: "identity-macros".into(),
                    spec: MacroRulesSpec {
                        macro_name: Ident::new("identity"),
                        arms: vec![MacroArm {
                            matcher: "($x:expr)".into(),
                            transcriber: "{ $x }".into(),
                        }],
                    },
                },
            ],
        }
    }

    #[test]
    fn workspace_compiles() {
        let ws = sample().compile_to_workspace().unwrap();
        assert_eq!(ws.workspace_name, "my-macros");
        assert_eq!(ws.member_crates.len(), 2);
    }

    #[test]
    fn root_cargo_toml_lists_members() {
        let ws = sample().compile_to_workspace().unwrap();
        assert!(ws.root_cargo_toml.contains(r#""crates/marker-derive""#));
        assert!(ws.root_cargo_toml.contains(r#""crates/identity-macros""#));
        assert!(ws.root_cargo_toml.contains("[workspace]"));
        assert!(ws.root_cargo_toml.contains("resolver = \"2\""));
    }

    #[test]
    fn member_crates_carry_their_files() {
        let ws = sample().compile_to_workspace().unwrap();
        let derive = ws
            .member_crates
            .iter()
            .find(|c| c.name == "marker-derive")
            .unwrap();
        assert!(derive.to_files().contains_key("src/lib.rs"));
        let rules = ws
            .member_crates
            .iter()
            .find(|c| c.name == "identity-macros")
            .unwrap();
        assert!(rules.to_files().contains_key("src/lib.rs"));
    }

    #[test]
    fn member_kind_dispatch_is_total() {
        // Every variant of MacroMemberSpec has a compile() path; this
        // test exercises all four to guard against future-added variants
        // missing their dispatch arm.
        let m1 = MacroMemberSpec::Derive {
            crate_name: "a".into(),
            spec: ProcDeriveSpec::new("A", vec![]),
        };
        let m2 = MacroMemberSpec::ProcAttr {
            crate_name: "b".into(),
            spec: ProcAttrSpec {
                macro_name: Ident::new("b"),
                transform: tatara_rust_proc_attr::AttrTransform::PrependPrelude {
                    prelude_tokens: String::new(),
                },
            },
        };
        let m3 = MacroMemberSpec::ProcFn {
            crate_name: "c".into(),
            spec: ProcFnSpec {
                macro_name: Ident::new("c"),
                transform: tatara_rust_proc_fn::FnTransform::PrependPrelude {
                    prelude_tokens: String::new(),
                },
            },
        };
        let m4 = MacroMemberSpec::MacroRules {
            crate_name: "d".into(),
            spec: MacroRulesSpec {
                macro_name: Ident::new("d"),
                arms: vec![],
            },
        };
        for m in [m1, m2, m3, m4] {
            assert!(m.compile().is_ok(), "{m:?}");
        }
    }

    #[test]
    fn serde_roundtrip() {
        let s = sample();
        let j = serde_json::to_string(&s).unwrap();
        let back: MacroSuiteSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
