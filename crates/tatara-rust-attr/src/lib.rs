//! `tatara-rust-attr` — typed attribute model + `PerAttrDeriveSpec`.
//!
//! Two primitives:
//! - [`AttrKnob`] — typed description of a consumer-side `#[trait(key = …)]`
//!   knob the derive understands. Each knob has a name + value-kind +
//!   optional default.
//! - [`PerAttrDeriveSpec`] — derive shape that reads its knobs from the
//!   consumer's `#[<trait_name>(…)]` attribute(s) and parameterizes
//!   its emission on the parsed values.
//!
//! Today's emit shape is the **prefix-template** family: a knob named
//! `prefix` with a string default. The per-field template re-uses the
//! prefix via `#prefix`. Sufficient for serde-style rename / derive-
//! builder-style setter-prefix patterns.
//!
//! Authoring shape:
//!
//! ```
//! use tatara_rust_ast::{CompileToCrate, Ident};
//! use tatara_rust_attr::{AttrKnob, AttrValueKind, PerAttrDeriveSpec};
//!
//! let spec = PerAttrDeriveSpec {
//!     trait_name: Ident::new("Prefixed"),
//!     knobs: vec![AttrKnob {
//!         name: "prefix".into(),
//!         kind: AttrValueKind::Str,
//!         default: Some("with_".into()),
//!     }],
//!     per_field_template:
//!         "pub fn #prefix #field_name(self, v: #field_ty) -> Self { self }".into(),
//! };
//! let scaffold = spec.compile_to_crate("prefixed-derive").unwrap();
//! assert!(scaffold.to_files().contains_key("src/lib.rs"));
//! ```

use serde::{Deserialize, Serialize};
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold, Ident};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttrValueKind {
    /// `key = "string"`.
    Str,
    /// `key = 42` — parsed as i64.
    Int,
    /// `key = true`.
    Bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttrKnob {
    /// `#[trait(<name> = …)]`.
    pub name: String,
    pub kind: AttrValueKind,
    /// Default value rendered as source text (with quotes for Str etc.).
    /// `None` ⇒ knob is required; emitted derive errors at consumer-expand
    /// time if missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerAttrDeriveSpec {
    pub trait_name: Ident,
    pub knobs: Vec<AttrKnob>,
    /// Per-field template with `#<knob_name>` + `#field_name` + `#field_ty`
    /// splice holes. Embedded raw inside `quote!{}`.
    pub per_field_template: String,
}

impl PerAttrDeriveSpec {
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

impl CompileToCrate for PerAttrDeriveSpec {
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
        "Per-attr derive proc-macro emitted from a tatara-rust-attr PerAttrDeriveSpec.",
    )
}

fn render_lib_rs(spec: &PerAttrDeriveSpec) -> String {
    let trait_id = &spec.trait_name.0;
    let fn_name = spec.fn_name();
    let trait_lower = trait_id.to_lowercase();
    let per_field_tpl = &spec.per_field_template;

    // For each knob, emit:
    //   let mut <knob> = <default-expr or compile_error>;
    //   <walk #[trait(...)] meta items + assign>
    let mut knob_let = String::new();
    let mut knob_parse = String::new();
    for k in &spec.knobs {
        let n = &k.name;
        let default_expr = match (&k.default, &k.kind) {
            (Some(d), AttrValueKind::Str) => format!("{d:?}.to_string()"),
            (Some(d), AttrValueKind::Int) => d.clone(),
            (Some(d), AttrValueKind::Bool) => d.clone(),
            (None, AttrValueKind::Str) => "String::new()".to_string(),
            (None, AttrValueKind::Int) => "0i64".to_string(),
            (None, AttrValueKind::Bool) => "false".to_string(),
        };
        knob_let.push_str(&format!("    let mut {n} = {default_expr};\n"));
        let parse_arm = match k.kind {
            AttrValueKind::Str => format!(
                "                    if path.is_ident({n:?}) {{ if let syn::Expr::Lit(syn::ExprLit {{ lit: syn::Lit::Str(s), .. }}) = &mv.value {{ {n} = s.value(); }} }}"
            ),
            AttrValueKind::Int => format!(
                "                    if path.is_ident({n:?}) {{ if let syn::Expr::Lit(syn::ExprLit {{ lit: syn::Lit::Int(i), .. }}) = &mv.value {{ {n} = i.base10_parse::<i64>().unwrap_or(0); }} }}"
            ),
            AttrValueKind::Bool => format!(
                "                    if path.is_ident({n:?}) {{ if let syn::Expr::Lit(syn::ExprLit {{ lit: syn::Lit::Bool(b), .. }}) = &mv.value {{ {n} = b.value; }} }}"
            ),
        };
        knob_parse.push_str(&parse_arm);
        knob_parse.push('\n');
    }

    let mut out = String::new();
    out.push_str("// GENERATED by tatara-rust-attr::PerAttrDeriveSpec.\n");
    out.push_str("use proc_macro::TokenStream;\n");
    out.push_str("use quote::quote;\n");
    out.push_str(
        "use syn::{Data, DataStruct, DeriveInput, Fields, Meta, parse_macro_input};\n\n",
    );
    out.push_str(&format!(
        "#[proc_macro_derive({trait_id}, attributes({trait_lower}))]\n"
    ));
    out.push_str(&format!(
        "pub fn {fn_name}(input: TokenStream) -> TokenStream {{\n"
    ));
    out.push_str("    let input = parse_macro_input!(input as DeriveInput);\n");
    out.push_str("    let self_name = &input.ident;\n\n");
    out.push_str(&knob_let);
    out.push('\n');
    out.push_str(&format!(
        "    for attr in &input.attrs {{\n        if attr.path().is_ident({trait_lower:?}) {{\n"
    ));
    out.push_str("            if let Ok(metas) = attr.parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated) {\n");
    out.push_str("                for meta in metas {\n");
    out.push_str("                    let Meta::NameValue(mv) = &meta else { continue };\n");
    out.push_str("                    let path = mv.path.clone();\n");
    out.push_str(&knob_parse);
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");
    out.push_str("    let fields = match &input.data {\n");
    out.push_str(
        "        Data::Struct(DataStruct { fields: Fields::Named(named), .. }) => &named.named,\n",
    );
    out.push_str("        _ => return syn::Error::new_spanned(self_name, \"PerAttrDerive requires a named-fields struct\").to_compile_error().into(),\n");
    out.push_str("    };\n\n");
    out.push_str("    let per_field = fields.iter().map(|f| {\n");
    out.push_str("        let field_name = f.ident.as_ref().expect(\"named field\");\n");
    out.push_str("        let field_ty = &f.ty;\n");
    // For each knob, splice it into the per_field template via quote::format_ident! or quote interpolation.
    // String knobs we splice as Ident via format_ident! so they prefix-concatenate cleanly.
    for k in &spec.knobs {
        let n = &k.name;
        match k.kind {
            AttrValueKind::Str => {
                out.push_str(&format!(
                    "        let {n}_ident = quote::format_ident!(\"{{}}{{}}\", {n}, field_name.to_string());\n"
                ));
                out.push_str(&format!(
                    "        let {n} = &{n}_ident;\n"
                ));
            }
            AttrValueKind::Int | AttrValueKind::Bool => {
                out.push_str(&format!("        let {n} = &{n};\n"));
            }
        }
    }
    out.push_str("        quote! {\n");
    out.push_str("            ");
    out.push_str(per_field_tpl);
    out.push_str("\n        }\n");
    out.push_str("    });\n\n");
    out.push_str("    let expanded = quote! {\n");
    out.push_str("        impl #self_name {\n");
    out.push_str("            #(#per_field)*\n");
    out.push_str("        }\n");
    out.push_str("    };\n");
    out.push_str("    TokenStream::from(expanded)\n");
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefixed() -> PerAttrDeriveSpec {
        PerAttrDeriveSpec {
            trait_name: Ident::new("Prefixed"),
            knobs: vec![AttrKnob {
                name: "prefix".into(),
                kind: AttrValueKind::Str,
                default: Some("with_".into()),
            }],
            per_field_template:
                "pub fn #prefix(self, v: #field_ty) -> Self { self }".into(),
        }
    }

    #[test]
    fn compiles_to_lib_and_cargo() {
        let s = prefixed().compile_to_crate("prefixed-derive").unwrap();
        let files = s.to_files();
        assert!(files.contains_key("Cargo.toml"));
        assert!(files.contains_key("src/lib.rs"));
    }

    #[test]
    fn proc_macro_derive_declares_attribute() {
        let s = prefixed().compile_to_crate("p").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // attributes() must include the lowercased trait name so #[prefixed(...)] parses.
        assert!(lib.contains("#[proc_macro_derive(Prefixed, attributes(prefixed))]"));
    }

    #[test]
    fn lib_rs_initializes_default_and_parses_knob() {
        let s = prefixed().compile_to_crate("p").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("let mut prefix"));
        assert!(lib.contains(r#""with_""#));
        assert!(lib.contains(r#"path.is_ident("prefix")"#));
        assert!(lib.contains("syn::Lit::Str"));
    }

    #[test]
    fn string_knob_format_idents_for_prefix_concat() {
        let s = prefixed().compile_to_crate("p").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("let prefix_ident = quote::format_ident!"));
    }

    #[test]
    fn multiple_knobs_each_get_default_let() {
        let mut s = prefixed();
        s.knobs.push(AttrKnob {
            name: "inline".into(),
            kind: AttrValueKind::Bool,
            default: Some("false".into()),
        });
        s.knobs.push(AttrKnob {
            name: "max".into(),
            kind: AttrValueKind::Int,
            default: Some("10".into()),
        });
        let lib = s.compile_to_crate("p").unwrap().to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("let mut prefix"));
        assert!(lib.contains("let mut inline"));
        assert!(lib.contains("let mut max"));
        assert!(lib.contains("syn::Lit::Bool"));
        assert!(lib.contains("syn::Lit::Int"));
    }

    #[test]
    fn serde_roundtrip() {
        let s = prefixed();
        let j = serde_json::to_string(&s).unwrap();
        let back: PerAttrDeriveSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
