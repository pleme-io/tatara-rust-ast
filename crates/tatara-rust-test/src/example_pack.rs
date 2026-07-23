//! `DeriveExamplePackSpec` — typed test-by-construction primitive.
//!
//! Pairs a derive Spec with N `(consumer_source, assertion_block)`
//! pairs. Each pack materializes a temp workspace + consumer crate
//! that imports the derive, applies it to the consumer source, and
//! runs the assertion block via `cargo test`. The Spec ships with its
//! own proof of correctness.
//!
//! Same shape works for any `CompileToCrate` Spec (Derive, PerField,
//! PerVariant, Composite). The pack is generic over the kind via a
//! boxed `&dyn CompileToCrate` reference.
//!
//! Real usage in `crates/tatara-rust-test/tests/example_pack_e2e.rs`.

use std::path::Path;
use std::process::Command;
use tatara_rust_ast::{AstError, CompileToCrate};

/// One worked example: a consumer struct/enum + the assertions that
/// should hold after applying the derive.
#[derive(Clone, Debug)]
pub struct Example {
    /// Human-readable name — used as the consumer crate suffix.
    pub name: String,
    /// Source of the consumer struct/enum (raw Rust). The `#[derive(<X>)]`
    /// attribute is added by the harness; do not include it.
    pub consumer_item: String,
    /// `#[test]` body that runs against the consumer item. Has access to
    /// the type via `super::*`.
    pub assertion_body: String,
}

/// An example pack — a derive Spec + N worked examples.
pub struct DeriveExamplePackSpec<'a, T: CompileToCrate + ?Sized> {
    pub derive_crate_name: String,
    /// The trait/derive identifier the consumer writes as `#[derive(...)]`.
    pub trait_name: String,
    /// The derive Spec under test.
    pub spec: &'a T,
    /// Imports the consumer needs in addition to the derive crate — e.g.
    /// `"use my_trait::Marker;"` when the derive emits an impl whose
    /// trait lives outside the derive crate. One per line.
    pub extra_consumer_imports: Vec<String>,
    /// Trait crates the consumer also needs. Path-deps in the consumer
    /// Cargo.toml. Each tuple is `(crate_name, src_lib_rs_contents)`.
    pub auxiliary_trait_crates: Vec<(String, String)>,
    /// The worked examples.
    pub examples: Vec<Example>,
}

/// Result of running an example pack against `cargo test`.
#[derive(Debug)]
pub struct PackRunReport {
    pub temp_root: std::path::PathBuf,
    pub cargo_test_succeeded: bool,
}

impl<'a, T: CompileToCrate + ?Sized> DeriveExamplePackSpec<'a, T> {
    /// Materialize the derive crate + every auxiliary trait crate + one
    /// consumer crate per Example under `root`. Drives `cargo test`
    /// against the consumer; returns true on success.
    pub fn run_under(&self, root: &Path) -> Result<PackRunReport, AstError> {
        std::fs::create_dir_all(root)?;

        // 1. Derive crate.
        let derive_root = root.join(&self.derive_crate_name);
        self.spec
            .compile_to_crate(&self.derive_crate_name)?
            .write_to(&derive_root)?;

        // 2. Auxiliary trait crates.
        for (name, lib_rs) in &self.auxiliary_trait_crates {
            let dir = root.join(name).join("src");
            std::fs::create_dir_all(&dir)?;
            std::fs::write(
                root.join(name).join("Cargo.toml"),
                format!(
                    r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#
                ),
            )?;
            std::fs::write(dir.join("lib.rs"), lib_rs)?;
        }

        // 3. One consumer crate with all examples folded in.
        let consumer = root.join("consumer");
        std::fs::create_dir_all(consumer.join("src"))?;
        std::fs::write(consumer.join("Cargo.toml"), self.render_consumer_cargo())?;
        std::fs::write(consumer.join("src/lib.rs"), self.render_consumer_lib())?;

        // 4. Drive `cargo test`.
        let status = Command::new("cargo")
            .arg("test")
            .current_dir(&consumer)
            .status()?;
        Ok(PackRunReport {
            temp_root: root.to_path_buf(),
            cargo_test_succeeded: status.success(),
        })
    }

    fn render_consumer_cargo(&self) -> String {
        let derive_under = self.derive_crate_name.replace('-', "_");
        let derive_dep = format!(
            r#"{derive_crate} = {{ path = "../{derive_crate}" }}"#,
            derive_crate = self.derive_crate_name
        );
        let aux_deps = self
            .auxiliary_trait_crates
            .iter()
            .map(|(n, _)| format!(r#"{n} = {{ path = "../{n}" }}"#))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = derive_under;
        format!(
            r#"[package]
name = "consumer"
version = "0.1.0"
edition = "2024"

[dependencies]
{derive_dep}
{aux_deps}

[lib]
path = "src/lib.rs"
"#
        )
    }

    fn render_consumer_lib(&self) -> String {
        let derive_under = self.derive_crate_name.replace('-', "_");
        let extra_imports = self.extra_consumer_imports.join("\n");
        let mut items = String::new();
        let mut tests = String::new();
        for ex in &self.examples {
            let ex_under = ex.name.replace('-', "_");
            items.push_str(&format!(
                "#[derive({trait_name})]\n{src}\n\n",
                trait_name = self.trait_name,
                src = ex.consumer_item
            ));
            tests.push_str(&format!(
                "    #[test] fn ex_{ex_under}() {{\n{body}\n    }}\n",
                body = ex.assertion_body
            ));
        }
        format!(
            r#"use {derive_under}::{trait_name};
{extra_imports}

{items}

#[cfg(test)]
mod tests {{
    use super::*;
{tests}
}}
"#,
            trait_name = self.trait_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_derive::{PerFieldDeriveSpec, PerFieldTarget};
    use tatara_rust_ast::Ident;

    fn getter_spec() -> PerFieldDeriveSpec {
        PerFieldDeriveSpec {
            trait_name: Ident::new("GetterPack"),
            target: PerFieldTarget::NamedStruct,
            trait_ref: None,
            per_field_template:
                "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
            method_name_template: None,
            impl_prelude: None,
            skip_fields: vec![],
            field_attribute: None,
            field_tag: None,
        }
    }

    #[test]
    fn pack_renders_consumer_with_derives_and_assertions() {
        let spec = getter_spec();
        let pack = DeriveExamplePackSpec {
            derive_crate_name: "getter-pack-derive".into(),
            trait_name: "GetterPack".into(),
            spec: &spec,
            extra_consumer_imports: vec![],
            auxiliary_trait_crates: vec![],
            examples: vec![Example {
                name: "two-fields".into(),
                consumer_item: "pub struct TwoFields { pub a: i32, pub b: String }".into(),
                assertion_body: r#"
        let t = TwoFields { a: 1, b: "x".into() };
        assert_eq!(*t.a(), 1);
        assert_eq!(t.b(), "x");"#
                    .into(),
            }],
        };
        let lib = pack.render_consumer_lib();
        assert!(lib.contains("use getter_pack_derive::GetterPack;"));
        assert!(lib.contains("#[derive(GetterPack)]"));
        assert!(lib.contains("pub struct TwoFields"));
        assert!(lib.contains("fn ex_two_fields"));
    }

    #[test]
    fn pack_cargo_lists_aux_trait_path_deps() {
        let spec = getter_spec();
        let pack = DeriveExamplePackSpec {
            derive_crate_name: "x-derive".into(),
            trait_name: "X".into(),
            spec: &spec,
            extra_consumer_imports: vec!["use x_trait::X;".into()],
            auxiliary_trait_crates: vec![("x-trait".into(), "pub trait X {}".into())],
            examples: vec![],
        };
        let cargo = pack.render_consumer_cargo();
        assert!(cargo.contains(r#"x-derive = { path = "../x-derive" }"#));
        assert!(cargo.contains(r#"x-trait = { path = "../x-trait" }"#));
    }
}
