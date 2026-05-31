//! `tatara-rust-derive` — L2 authoring surface for Rust **derive** proc-macros.
//!
//! One typed `ProcDeriveSpec` value → one complete proc-macro crate
//! scaffold (Cargo.toml + src/lib.rs + tests/trybuild_pass.rs) via the
//! [`CompileToCrate`] trait. The L0 [`Impl`] from `tatara-rust-ast` is
//! the body the derive macro emits at consumer compile time.
//!
//! Authoring shape:
//!
//! ```
//! use tatara_rust_ast::{Block, Expr, Fn, FnSig, Generics, Ident,
//!     RefKind, Stmt, TypeRef, CompileToCrate};
//! use tatara_rust_derive::ProcDeriveSpec;
//!
//! let spec = ProcDeriveSpec::new(
//!     "StaticName",
//!     vec![Fn {
//!         sig: FnSig {
//!             name: Ident::new("type_name"),
//!             generics: Generics::default(),
//!             params: vec![],
//!             return_type: Some(TypeRef {
//!                 ident: Ident::new("str"),
//!                 generics: vec![],
//!                 reference: Some(RefKind::shared_lifetime("static")),
//!             }),
//!         },
//!         body: Block { stmts: vec![Stmt::Tail {
//!             expr: Expr::MacroCall {
//!                 path: vec![Ident::new("stringify")],
//!                 tokens: "#__SELF_NAME__".into(),
//!             },
//!         }]},
//!     }],
//! );
//! let scaffold = spec.compile_to_crate("static-name-derive").unwrap();
//! assert!(scaffold.to_files().contains_key("src/lib.rs"));
//! ```
//!
//! Substitution sentinels (replaced by `quote!{}` at macro-expansion time):
//! - `__SELF_TYPE__` (TypeRef ident) → `#name` (the consumer's struct ident).
//! - `__SELF_NAME__` (Expr text) → `#name` (same).

use serde::{Deserialize, Serialize};
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold, Ident, Impl, ToRustTokens};

pub mod enum_fold;
pub mod kind_round_trip;
pub mod newtype;
pub mod per_field;
pub mod per_variant;
pub mod verification_matrix;
pub use enum_fold::{EnumFoldDeriveSpec, EnumFoldTarget};
pub use kind_round_trip::KindRoundTripSpec;
pub use newtype::{NewtypeDeriveSpec, NewtypeTarget};
pub use per_field::{PerFieldDeriveSpec, PerFieldTarget};
pub use per_variant::{PerVariantDeriveSpec, VariantShape};
pub use verification_matrix::VerificationMatrixSpec;

pub const SENTINEL_SELF_TYPE: &str = "__SELF_TYPE__";
pub const SENTINEL_SELF_NAME: &str = "#__SELF_NAME__";

/// Typed spec for a `#[derive(MyDerive)]` proc-macro.
///
/// `trait_name` is the user-facing derive identifier (`#[derive(StaticName)]`).
/// `impl_template` is the L0 IR for the impl that the derive emits, with
/// `__SELF_TYPE__` standing in for the consumer's type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcDeriveSpec {
    pub trait_name: Ident,
    pub impl_template: Impl,
}

impl ProcDeriveSpec {
    /// Helper for the common shape: `impl Trait for $SELF { <items> }`.
    /// Skips the boilerplate of constructing the sentinel `self_type`.
    #[must_use]
    pub fn new(trait_name: impl Into<String>, items: Vec<tatara_rust_ast::Fn>) -> Self {
        let t = Ident::new(trait_name);
        let trait_ref = tatara_rust_ast::TypeRef::simple(&t.0);
        Self {
            trait_name: t,
            impl_template: Impl {
                generics: tatara_rust_ast::Generics::default(),
                trait_ref: Some(trait_ref),
                self_type: tatara_rust_ast::TypeRef::simple(SENTINEL_SELF_TYPE),
                items,
            },
        }
    }

    /// The derive-fn body that lives inside the generated proc-macro
    /// crate's `lib.rs`. Wraps the impl_template in a `quote!{}` and
    /// splices the consumer's `Ident` over the sentinels.
    fn derive_fn_body(&self) -> String {
        // Render the impl, then text-replace sentinels with quote splices.
        let rendered = self.impl_template.to_rust_tokens().to_string();
        rendered
            .replace(SENTINEL_SELF_TYPE, "#name")
            .replace("# __SELF_NAME__", "#name")
            .replace(SENTINEL_SELF_NAME, "#name")
    }
}

impl CompileToCrate for ProcDeriveSpec {
    fn compile_to_crate(&self, crate_name: &str) -> Result<CrateScaffold, AstError> {
        let mut scaffold = CrateScaffold::new(crate_name, "0.1.0");
        scaffold.add_file("Cargo.toml", render_cargo_toml(crate_name)?);
        scaffold.add_file("src/lib.rs", render_lib_rs(self));
        // No per-derive-crate `tests/`: proc-macro crates can't export
        // anything besides their proc-macros, so a trybuild fixture
        // can't import its companion trait. Use `tatara-rust-test`'s
        // `TestLayout` for the trait+consumer harness.
        Ok(scaffold)
    }
}

fn render_cargo_toml(crate_name: &str) -> Result<String, AstError> {
    Ok(tatara_rust_ast::render_proc_macro_cargo_toml(
        crate_name,
        "Proc-macro derive crate emitted from a tatara-rust-derive ProcDeriveSpec.",
    ))
}

fn render_lib_rs(spec: &ProcDeriveSpec) -> String {
    use proc_macro2::TokenStream;
    use quote::{format_ident, quote};

    let trait_id = format_ident!("{}", spec.trait_name.0);
    let fn_name_str = format!(
        "derive_{}",
        spec.trait_name
            .0
            .chars()
            .flat_map(|c| {
                if c.is_uppercase() {
                    vec!['_', c.to_ascii_lowercase()]
                } else {
                    vec![c]
                }
            })
            .skip_while(|c| *c == '_')
            .collect::<String>()
    );
    let fn_name = format_ident!("{fn_name_str}");

    let body: TokenStream = spec
        .derive_fn_body()
        .parse()
        .expect("derive_fn_body must parse as TokenStream");

    let file: TokenStream = quote! {
        #![doc = "GENERATED by tatara-rust-derive from a ProcDeriveSpec."]
        #![doc = "Do not hand-edit; regenerate from the (defprocderive …) source."]

        use proc_macro::TokenStream;
        use quote::quote;
        use syn::{parse_macro_input, DeriveInput};

        #[proc_macro_derive(#trait_id)]
        pub fn #fn_name(input: TokenStream) -> TokenStream {
            let input = parse_macro_input!(input as DeriveInput);
            let name = &input.ident;
            let expanded = quote! {
                #body
            };
            TokenStream::from(expanded)
        }
    };

    let parsed: syn::File =
        syn::parse2(file).expect("emitted lib.rs must parse as syn::File");
    prettyplease::unparse(&parsed)
}


#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_snapshot::assert_tokens_contain;
    use tatara_rust_ast::{
        Block, Expr, Fn as RsFn, FnSig, Generics, RefKind, Stmt, TypeRef,
    };

    fn static_name_spec() -> ProcDeriveSpec {
        ProcDeriveSpec::new(
            "StaticName",
            vec![RsFn {
                sig: FnSig {
                    name: Ident::new("type_name"),
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
                        expr: Expr::MacroCall {
                            path: vec![Ident::new("stringify")],
                            tokens: "#__SELF_NAME__".into(),
                        },
                    }],
                },
            }],
        )
    }

    #[test]
    fn spec_compiles_to_cargo_and_lib() {
        let scaffold = static_name_spec().compile_to_crate("static-name").unwrap();
        let files = scaffold.to_files();
        assert!(files.contains_key("Cargo.toml"));
        assert!(files.contains_key("src/lib.rs"));
    }

    #[test]
    fn cargo_toml_marks_proc_macro() {
        let scaffold = static_name_spec().compile_to_crate("static-name").unwrap();
        let toml = scaffold.to_files().get("Cargo.toml").unwrap().clone();
        assert!(toml.contains("proc-macro"));
        assert!(toml.contains("name = \"static-name\""));
    }

    #[test]
    fn lib_rs_declares_derive_fn() {
        let scaffold = static_name_spec().compile_to_crate("static-name").unwrap();
        let lib = scaffold.to_files().get("src/lib.rs").unwrap().clone();
        assert_tokens_contain!(&lib, "#[proc_macro_derive(StaticName)]");
        assert_tokens_contain!(&lib, "pub fn derive_static_name");
        assert_tokens_contain!(&lib, "parse_macro_input!(input as DeriveInput)");
    }

    #[test]
    fn lib_rs_splices_self_name_into_quote() {
        let scaffold = static_name_spec().compile_to_crate("static-name").unwrap();
        let lib = scaffold.to_files().get("src/lib.rs").unwrap().clone();
        // The sentinel must have been replaced with the `#name` quote-splice.
        assert!(!lib.contains("__SELF_NAME__"), "sentinel not replaced: {lib}");
        assert_tokens_contain!(&lib, "#name");
    }

    #[test]
    fn snake_case_helper_handles_camel() {
        // Render-and-grep to verify the snake-case conversion picks the right name.
        let s = ProcDeriveSpec::new("MyTrait", vec![]);
        let scaffold = s.compile_to_crate("my-trait").unwrap();
        let lib = scaffold.to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("pub fn derive_my_trait"), "got: {lib}");
    }
}
