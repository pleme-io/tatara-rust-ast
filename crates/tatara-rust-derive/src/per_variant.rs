//! `PerVariantDeriveSpec` — higher-order derive primitive for **enums**.
//!
//! Walks `Data::Enum`'s variants at consumer-compile-time and emits one
//! fragment per variant, wrapped in an outer impl. Mirrors
//! [`crate::PerFieldDeriveSpec`] for structs.
//!
//! Splice holes available in the per-variant template:
//! - `#variant_name` — the variant's identifier
//! - `#self_name`    — the consumer enum's identifier
//! - `#method_ident` — `format_ident!(method_name_template, variant_name)`
//!                     (only when [`PerVariantDeriveSpec::method_name_template`] set)
//! - `#variant_shape_arm` — `Self::VariantName(..)` / `Self::VariantName { .. }`
//!                          / `Self::VariantName` — pre-shaped match arm head
//!                          accepting any payload. Use inside `match self { … }`.
//!
//! Real worked Specs in `tatara-rust-examples` (IsVariant, VariantName).

use serde::{Deserialize, Serialize};
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold, Ident};

/// Where the derive emits per-variant code today: any variant shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VariantShape {
    /// All variant shapes (unit, tuple, struct) — emitter generates a
    /// shape-tolerant match arm via `Self::V { .. }` / `Self::V(..)`.
    Any,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerVariantDeriveSpec {
    pub trait_name: Ident,
    pub variant_shape: VariantShape,
    /// `impl <trait_ref> for $Self` when set; inherent impl when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trait_ref: Option<String>,
    /// Per-variant code fragment, embedded raw inside a `quote!{}` block.
    pub per_variant_template: String,
    /// `format_ident!` template for the derived method name —
    /// e.g. `"is_{}"` → `is_<variant>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_name_template: Option<String>,
    /// Optional impl-block prelude before the per-variant fragments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impl_prelude: Option<String>,
}

impl PerVariantDeriveSpec {
    fn fn_name(&self) -> String {
        let s = &self.trait_name.0;
        let mut out = String::from("derive_");
        for (i, c) in s.chars().enumerate() {
            if c.is_uppercase() {
                if i > 0 {
                    out.push('_');
                }
                out.extend(c.to_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }
}

impl CompileToCrate for PerVariantDeriveSpec {
    fn compile_to_crate(&self, crate_name: &str) -> Result<CrateScaffold, AstError> {
        let mut s = CrateScaffold::new(crate_name, "0.1.0");
        s.add_file("Cargo.toml", render_cargo_toml(crate_name));
        s.add_file("src/lib.rs", render_lib_rs(self));
        Ok(s)
    }
}

fn render_cargo_toml(crate_name: &str) -> String {
    tatara_rust_ast::render_proc_macro_cargo_toml(
        crate_name,
        "Per-variant derive proc-macro emitted from a tatara-rust-derive PerVariantDeriveSpec.",
    )
}

/// Render lib.rs via the typed AST substrate (proc-macro2 + quote +
/// syn + prettyplease). Replaces the prior `push_str` tower per the
/// fleet's TYPED EMISSION directive — peer of `per_field::render_lib_rs`.
fn render_lib_rs(spec: &PerVariantDeriveSpec) -> String {
    use proc_macro2::TokenStream;
    use quote::{format_ident, quote};

    let trait_id = format_ident!("{}", spec.trait_name.0);
    let fn_name = format_ident!("{}", spec.fn_name());

    let per_variant_body: TokenStream = spec
        .per_variant_template
        .parse()
        .expect("per_variant_template must parse as TokenStream");

    // `#self_name` and `#variant_name { .. }` style fragments need to
    // pass through to the consumer's compile-time quote!{} — pre-build
    // them as TokenStream variables so the outer quote splices them
    // verbatim (including the literal `#` punct token).
    let hash_self_name: TokenStream =
        "#self_name".parse().expect("static literal must parse");
    let hash_variant_name: TokenStream =
        "#variant_name".parse().expect("static literal must parse");
    let hash_per_variant_repeat: TokenStream =
        "#(#per_variant)*".parse().expect("static literal must parse");

    let method_ident_binding: TokenStream = match &spec.method_name_template {
        None => quote! {},
        Some(tpl) => quote! {
            let method_ident = quote::format_ident!(#tpl, variant_name.to_string().to_lowercase());
        },
    };

    let impl_open: TokenStream = match &spec.trait_ref {
        None => quote! { impl #hash_self_name },
        Some(t) => {
            let path: TokenStream = t
                .parse()
                .expect("trait_ref must parse as TokenStream");
            quote! { impl #path for #hash_self_name }
        }
    };

    let prelude: TokenStream = match &spec.impl_prelude {
        None => quote! {},
        Some(p) => p.parse().expect("impl_prelude must parse as TokenStream"),
    };

    let file: TokenStream = quote! {
        #![doc = "GENERATED by tatara-rust-derive::PerVariantDeriveSpec."]
        #![doc = "Do not hand-edit; regenerate from the (defpervariant …) source."]

        use proc_macro::TokenStream;
        use quote::quote;
        use syn::{Data, DataEnum, DeriveInput, Fields, parse_macro_input};

        #[proc_macro_derive(#trait_id)]
        pub fn #fn_name(input: TokenStream) -> TokenStream {
            let input = parse_macro_input!(input as DeriveInput);
            let self_name = &input.ident;

            let variants = match &input.data {
                Data::Enum(DataEnum { variants, .. }) => variants,
                _ => {
                    return syn::Error::new_spanned(
                        self_name,
                        "PerVariantDerive requires an enum",
                    )
                    .to_compile_error()
                    .into();
                }
            };

            let per_variant = variants.iter().map(|v| {
                let variant_name = &v.ident;
                let variant_shape_arm = match &v.fields {
                    Fields::Named(_)   => quote! { Self::#hash_variant_name { .. } },
                    Fields::Unnamed(_) => quote! { Self::#hash_variant_name(..) },
                    Fields::Unit       => quote! { Self::#hash_variant_name },
                };
                #method_ident_binding
                quote! {
                    #per_variant_body
                }
            });

            let expanded = quote! {
                #impl_open {
                    #prelude
                    #hash_per_variant_repeat
                }
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

    fn is_variant() -> PerVariantDeriveSpec {
        PerVariantDeriveSpec {
            trait_name: Ident::new("IsVariant"),
            variant_shape: VariantShape::Any,
            trait_ref: None,
            per_variant_template: "pub fn #method_ident(&self) -> bool { matches!(self, #variant_shape_arm) }".into(),
            method_name_template: Some("is_{}".into()),
            impl_prelude: None,
        }
    }

    #[test]
    fn compiles_to_lib_and_cargo() {
        let s = is_variant().compile_to_crate("is-variant-derive").unwrap();
        let files = s.to_files();
        assert!(files.contains_key("Cargo.toml"));
        assert!(files.contains_key("src/lib.rs"));
    }

    #[test]
    fn lib_rs_walks_enum_variants() {
        let s = is_variant().compile_to_crate("v").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("Data::Enum(DataEnum"));
        assert!(lib.contains("variants.iter()"));
        assert!(lib.contains("variant_shape_arm"));
        assert!(lib.contains("#[proc_macro_derive(IsVariant)]"));
    }

    #[test]
    fn shape_arm_handles_all_variant_kinds() {
        let s = is_variant().compile_to_crate("v").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("Fields::Named(_)"));
        assert!(lib.contains("Fields::Unnamed(_)"));
        assert!(lib.contains("Fields::Unit"));
    }

    #[test]
    fn method_ident_binding_when_template_set() {
        let s = is_variant().compile_to_crate("v").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("format_ident!"));
        assert!(lib.contains("\"is_{}\""));
    }

    #[test]
    fn serde_roundtrip() {
        let s = is_variant();
        let j = serde_json::to_string(&s).unwrap();
        let back: PerVariantDeriveSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
