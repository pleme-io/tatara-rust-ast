//! `tatara-rust-composite` — L2 composition primitive.
//!
//! `CompositeDeriveSpec` bundles N inner derive Specs (any kind) under
//! one user-facing `#[derive(<name>)]`. The emitted proc-macro
//! crate dispatches the consumer's `DeriveInput` to each inner Spec's
//! own emitter and concatenates the produced `TokenStream`s.
//!
//! Authoring shape:
//!
//! ```
//! use tatara_rust_ast::{CompileToCrate, Ident};
//! use tatara_rust_derive::{PerFieldDeriveSpec, PerFieldTarget};
//! use tatara_rust_composite::{CompositeDeriveSpec, CompositeMember};
//!
//! let getter = PerFieldDeriveSpec {
//!     trait_name: Ident::new("AccessorGetter"),
//!     target: PerFieldTarget::NamedStruct,
//!     trait_ref: None,
//!     per_field_template:
//!         "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
//!     method_name_template: None,
//!     impl_prelude: None,
//!     skip_fields: vec![],
//!     field_attribute: None,
//! };
//! let setter = PerFieldDeriveSpec {
//!     trait_name: Ident::new("AccessorSetter"),
//!     target: PerFieldTarget::NamedStruct,
//!     trait_ref: None,
//!     per_field_template:
//!         "pub fn #method_ident(&mut self, v: #field_ty) { self.#field_name = v; }".into(),
//!     method_name_template: Some("set_{}".into()),
//!     impl_prelude: None,
//!     skip_fields: vec![],
//!     field_attribute: None,
//! };
//! let bundle = CompositeDeriveSpec {
//!     bundle_name: Ident::new("Accessor"),
//!     members: vec![
//!         CompositeMember::PerField(getter),
//!         CompositeMember::PerField(setter),
//!     ],
//! };
//! let scaffold = bundle.compile_to_crate("accessor-derive").unwrap();
//! assert!(scaffold.to_files().contains_key("src/lib.rs"));
//! ```

use serde::{Deserialize, Serialize};
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold, Ident, ToRustTokens};
use tatara_rust_derive::{PerFieldDeriveSpec, PerVariantDeriveSpec, ProcDeriveSpec};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositeDeriveSpec {
    /// User-facing `#[derive(<bundle_name>)]`.
    pub bundle_name: Ident,
    /// Inner derive Specs whose emissions get concatenated.
    pub members: Vec<CompositeMember>,
}

/// Tagged-enum dispatch over the three derive shapes we know how to
/// emit today. Adding a 4th kind = one variant + one match arm in
/// `member_emit_call`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CompositeMember {
    Simple(ProcDeriveSpec),
    PerField(PerFieldDeriveSpec),
    PerVariant(PerVariantDeriveSpec),
}

impl CompositeDeriveSpec {
    fn fn_name(&self) -> String {
        let s = &self.bundle_name.0;
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

impl CompileToCrate for CompositeDeriveSpec {
    fn compile_to_crate(&self, crate_name: &str) -> Result<CrateScaffold, AstError> {
        let mut s = CrateScaffold::new(crate_name, "0.1.0");
        s.add_file("Cargo.toml", render_cargo_toml(crate_name));
        s.add_file("src/lib.rs", render_lib_rs(self)?);
        Ok(s)
    }
}

fn render_cargo_toml(crate_name: &str) -> String {
    tatara_rust_ast::render_proc_macro_cargo_toml(
        crate_name,
        "Composite derive proc-macro — fans one #[derive(...)] out to N inner Specs.",
    )
}

/// Compile each inner member to its own derive lib.rs, then inline the
/// per-member dispatch into a single fn that fans the DeriveInput out
/// to every member's emit logic. To keep the spike simple, we embed
/// each member's lib.rs body verbatim — without the `#[proc_macro_derive]`
/// attribute (only the outer bundle gets it) — and concatenate the
/// resulting TokenStream pieces.
fn render_lib_rs(spec: &CompositeDeriveSpec) -> Result<String, AstError> {
    let bundle = &spec.bundle_name.0;
    let fn_name = spec.fn_name();

    // For each inner member, emit a closure that builds the impl shape.
    // The closure takes `&DeriveInput`, returns `proc_macro2::TokenStream`.
    let mut closures = String::new();
    let mut calls = String::new();
    for (i, m) in spec.members.iter().enumerate() {
        let cname = format!("__member_{i}");
        closures.push_str(&format!(
            "    let {cname} = |input: &syn::DeriveInput| -> proc_macro2::TokenStream {{\n"
        ));
        closures.push_str(&render_member_body(m));
        closures.push_str("    };\n");
        calls.push_str(&format!("    let __out_{i} = {cname}(&input);\n"));
    }
    let stitched = (0..spec.members.len())
        .map(|i| format!("#__out_{i}"))
        .collect::<Vec<_>>()
        .join(" ");

    let mut out = String::new();
    out.push_str("// GENERATED by tatara-rust-composite::CompositeDeriveSpec.\n");
    out.push_str("use proc_macro::TokenStream;\n");
    out.push_str("use quote::quote;\n");
    out.push_str("use syn::parse_macro_input;\n\n");
    out.push_str(&format!("#[proc_macro_derive({bundle})]\n"));
    out.push_str(&format!(
        "pub fn {fn_name}(input: TokenStream) -> TokenStream {{\n"
    ));
    out.push_str("    let input = parse_macro_input!(input as syn::DeriveInput);\n");
    out.push_str(&closures);
    out.push_str(&calls);
    for i in 0..spec.members.len() {
        let _ = i;
    }
    out.push_str(&format!(
        "    let expanded = quote! {{ {stitched} }};\n"
    ));
    out.push_str("    TokenStream::from(expanded)\n");
    out.push_str("}\n");

    Ok(out)
}

/// Render a single inner Spec's emit logic as a closure body. Mirrors
/// the body of each Spec's own `derive_*` fn (sans the outer
/// `#[proc_macro_derive]` attribute + the parse_macro_input call).
fn render_member_body(m: &CompositeMember) -> String {
    match m {
        CompositeMember::Simple(spec) => {
            // Simple ProcDerive: emit `impl Trait for #name { items… }`.
            // The impl is rendered into the closure body verbatim.
            let body = spec.impl_template.to_rust_tokens().to_string();
            let body = body
                .replace(tatara_rust_derive::SENTINEL_SELF_TYPE, "#name")
                .replace("# __SELF_NAME__", "#name")
                .replace(tatara_rust_derive::SENTINEL_SELF_NAME, "#name");
            format!(
                r#"        let name = &input.ident;
        quote! {{ {body} }}
"#
            )
        }
        CompositeMember::PerField(spec) => render_per_field_body(spec),
        CompositeMember::PerVariant(spec) => render_per_variant_body(spec),
    }
}

fn render_per_field_body(spec: &PerFieldDeriveSpec) -> String {
    let impl_open = match &spec.trait_ref {
        None => "impl #self_name".to_string(),
        Some(t) => format!("impl {t} for #self_name"),
    };
    let method_ident_let = match &spec.method_name_template {
        None => String::new(),
        Some(tpl) => format!(
            "            let method_ident = quote::format_ident!(\"{tpl}\", field_name.to_string());\n"
        ),
    };
    let prelude = spec.impl_prelude.as_deref().unwrap_or("");
    let tpl = &spec.per_field_template;

    let mut out = String::new();
    out.push_str("        let self_name = &input.ident;\n");
    out.push_str("        let fields = match &input.data {\n");
    out.push_str("            syn::Data::Struct(syn::DataStruct { fields: syn::Fields::Named(named), .. }) => &named.named,\n");
    out.push_str("            _ => return syn::Error::new_spanned(self_name, \"composite per-field member needs named-fields struct\").to_compile_error(),\n");
    out.push_str("        };\n");
    out.push_str("        let per_field = fields.iter().map(|f| {\n");
    out.push_str("            let field_name = f.ident.as_ref().expect(\"named\");\n");
    out.push_str("            let field_ty = &f.ty;\n");
    out.push_str(&method_ident_let);
    out.push_str("            quote! { ");
    out.push_str(tpl);
    out.push_str(" }\n");
    out.push_str("        });\n");
    out.push_str(&format!(
        "        quote! {{ {impl_open} {{ {prelude} #(#per_field)* }} }}\n"
    ));
    out
}

fn render_per_variant_body(spec: &PerVariantDeriveSpec) -> String {
    let impl_open = match &spec.trait_ref {
        None => "impl #self_name".to_string(),
        Some(t) => format!("impl {t} for #self_name"),
    };
    let method_ident_let = match &spec.method_name_template {
        None => String::new(),
        Some(tpl) => format!(
            "            let method_ident = quote::format_ident!(\"{tpl}\", variant_name.to_string());\n"
        ),
    };
    let prelude = spec.impl_prelude.as_deref().unwrap_or("");
    let tpl = &spec.per_variant_template;

    let mut out = String::new();
    out.push_str("        let self_name = &input.ident;\n");
    out.push_str("        let variants = match &input.data {\n");
    out.push_str("            syn::Data::Enum(syn::DataEnum { variants, .. }) => variants,\n");
    out.push_str("            _ => return syn::Error::new_spanned(self_name, \"composite per-variant member needs an enum\").to_compile_error(),\n");
    out.push_str("        };\n");
    out.push_str("        let per_variant = variants.iter().map(|v| {\n");
    out.push_str("            let variant_name = &v.ident;\n");
    out.push_str("            let variant_shape_arm = match &v.fields {\n");
    out.push_str("                syn::Fields::Named(_)   => quote! { Self::#variant_name { .. } },\n");
    out.push_str("                syn::Fields::Unnamed(_) => quote! { Self::#variant_name(..) },\n");
    out.push_str("                syn::Fields::Unit       => quote! { Self::#variant_name },\n");
    out.push_str("            };\n");
    out.push_str(&method_ident_let);
    out.push_str("            quote! { ");
    out.push_str(tpl);
    out.push_str(" }\n");
    out.push_str("        });\n");
    out.push_str(&format!(
        "        quote! {{ {impl_open} {{ {prelude} #(#per_variant)* }} }}\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_derive::{PerFieldDeriveSpec, PerFieldTarget};

    fn accessor_bundle() -> CompositeDeriveSpec {
        let getter = PerFieldDeriveSpec {
            trait_name: Ident::new("AccessorGetter"),
            target: PerFieldTarget::NamedStruct,
            trait_ref: None,
            per_field_template:
                "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
            method_name_template: None,
            impl_prelude: None,
            skip_fields: vec![],
            field_attribute: None,
        };
        let setter = PerFieldDeriveSpec {
            trait_name: Ident::new("AccessorSetter"),
            target: PerFieldTarget::NamedStruct,
            trait_ref: None,
            per_field_template:
                "pub fn #method_ident(&mut self, v: #field_ty) { self.#field_name = v; }".into(),
            method_name_template: Some("set_{}".into()),
            impl_prelude: None,
            skip_fields: vec![],
            field_attribute: None,
        };
        CompositeDeriveSpec {
            bundle_name: Ident::new("Accessor"),
            members: vec![
                CompositeMember::PerField(getter),
                CompositeMember::PerField(setter),
            ],
        }
    }

    #[test]
    fn compiles_to_lib_and_cargo() {
        let s = accessor_bundle().compile_to_crate("accessor-derive").unwrap();
        let files = s.to_files();
        assert!(files.contains_key("Cargo.toml"));
        assert!(files.contains_key("src/lib.rs"));
    }

    #[test]
    fn lib_rs_emits_one_proc_macro_for_bundle() {
        let s = accessor_bundle().compile_to_crate("a").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // Exactly ONE outer derive attribute, no matter how many inner members.
        assert_eq!(
            lib.matches("#[proc_macro_derive(Accessor)]").count(),
            1,
            "expected one outer derive, got: {lib}"
        );
    }

    #[test]
    fn lib_rs_creates_one_closure_per_member() {
        let s = accessor_bundle().compile_to_crate("a").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("__member_0"));
        assert!(lib.contains("__member_1"));
        assert!(lib.contains("__out_0"));
        assert!(lib.contains("__out_1"));
    }

    #[test]
    fn lib_rs_stitches_member_outputs() {
        let s = accessor_bundle().compile_to_crate("a").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("quote! { #__out_0 #__out_1 }"));
    }

    #[test]
    fn serde_roundtrip() {
        let s = accessor_bundle();
        let j = serde_json::to_string(&s).unwrap();
        let back: CompositeDeriveSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
