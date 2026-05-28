//! `tatara-rust-test` — L3 scaffolding.
//!
//! Given a typed [`tatara_rust_derive::ProcDeriveSpec`], materialize a
//! test environment on disk that exercises the generated proc-macro
//! end-to-end:
//! - emit the derive crate to a temp dir
//! - emit a minimal consumer crate that depends on it
//! - exposes paths for an external driver to run `cargo test`
//!
//! Snapshot-friendly: [`as_files`] returns the in-memory file map for
//! `insta::assert_yaml_snapshot!`.

pub mod example_pack;
pub use example_pack::{DeriveExamplePackSpec, Example, PackRunReport};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold};
use tatara_rust_derive::ProcDeriveSpec;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TestError {
    #[error("ast: {0}")]
    Ast(#[from] AstError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Lay out a two-crate scaffold (derive + consumer) under `root`.
pub struct TestLayout {
    pub root: PathBuf,
    pub derive_crate_name: String,
    pub consumer_crate_name: String,
}

impl TestLayout {
    pub fn write(
        spec: &ProcDeriveSpec,
        derive_crate_name: &str,
        consumer_crate_name: &str,
        root: &Path,
    ) -> Result<Self, TestError> {
        let derive_root = root.join(derive_crate_name);
        spec.compile_to_crate(derive_crate_name)?.write_to(&derive_root)?;

        let consumer = consumer_scaffold(spec, derive_crate_name, consumer_crate_name);
        let consumer_root = root.join(consumer_crate_name);
        consumer.write_to(&consumer_root)?;

        Ok(Self {
            root: root.to_path_buf(),
            derive_crate_name: derive_crate_name.into(),
            consumer_crate_name: consumer_crate_name.into(),
        })
    }

    /// In-memory view of the two scaffolds — useful for snapshot tests
    /// without touching disk.
    #[must_use]
    pub fn as_files(
        spec: &ProcDeriveSpec,
        derive_crate_name: &str,
        consumer_crate_name: &str,
    ) -> BTreeMap<String, String> {
        let mut out: BTreeMap<String, String> = BTreeMap::new();
        if let Ok(d) = spec.compile_to_crate(derive_crate_name) {
            for f in d.files {
                out.insert(format!("{derive_crate_name}/{}", f.path), f.contents);
            }
        }
        let c = consumer_scaffold(spec, derive_crate_name, consumer_crate_name);
        for f in c.files {
            out.insert(format!("{consumer_crate_name}/{}", f.path), f.contents);
        }
        out
    }
}

fn consumer_scaffold(
    spec: &ProcDeriveSpec,
    derive_crate_name: &str,
    consumer_crate_name: &str,
) -> CrateScaffold {
    let trait_id = &spec.trait_name.0;
    let derive_under = derive_crate_name.replace('-', "_");
    let consumer_under = consumer_crate_name.replace('-', "_");

    let mut s = CrateScaffold::new(consumer_crate_name, "0.1.0");

    s.add_file(
        "Cargo.toml",
        format!(
            r#"[package]
name = "{consumer_crate_name}"
version = "0.1.0"
edition = "2024"

[dependencies]
{derive_crate_name} = {{ path = "../{derive_crate_name}" }}

[lib]
path = "src/lib.rs"
"#
        ),
    );

    s.add_file(
        "src/lib.rs",
        format!(
            r#"// Consumer crate — exercises the generated derive macro end-to-end.
use {derive_under}::{trait_id};

#[derive({trait_id})]
pub struct Thing;

#[cfg(test)]
mod tests {{
    use super::*;
    #[test]
    fn smoke() {{
        // Compilation of the derive IS the assertion.
        let _t = Thing;
    }}
}}
"#
        ),
    );
    let _ = consumer_under; // currently unused; keep for forward compat.
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_ast::{
        Block, Expr, Fn as RsFn, FnSig, Generics, Ident, RefKind, Stmt, TypeRef,
    };

    fn spec() -> ProcDeriveSpec {
        ProcDeriveSpec::new(
            "Probe",
            vec![RsFn {
                sig: FnSig {
                    name: Ident::new("probe"),
                    generics: Generics::default(),
                    params: vec![],
                    return_type: Some(TypeRef {
                        ident: Ident::new("str"),
                        generics: vec![],
                        reference: Some(RefKind::shared_lifetime("static")),
                    }),
                },
                body: Block {
                    stmts: vec![Stmt::Tail {
                        expr: Expr::Literal {
                            value: "\"ok\"".into(),
                        },
                    }],
                },
            }],
        )
    }

    #[test]
    fn as_files_contains_both_crates() {
        let files = TestLayout::as_files(&spec(), "probe-derive", "probe-consumer");
        assert!(files.contains_key("probe-derive/Cargo.toml"));
        assert!(files.contains_key("probe-derive/src/lib.rs"));
        assert!(files.contains_key("probe-consumer/Cargo.toml"));
        assert!(files.contains_key("probe-consumer/src/lib.rs"));
    }

    #[test]
    fn consumer_uses_derive_path_dep() {
        let files = TestLayout::as_files(&spec(), "probe-derive", "probe-consumer");
        let toml = files.get("probe-consumer/Cargo.toml").unwrap();
        assert!(toml.contains(r#"probe-derive = { path = "../probe-derive" }"#));
    }
}
