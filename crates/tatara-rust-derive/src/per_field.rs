//! `PerFieldDeriveSpec` — higher-order derive primitive.
//!
//! Whereas [`crate::ProcDeriveSpec`] emits a fixed impl block, this one
//! parameterizes over the consumer struct's field shape. The generated
//! derive walks `DeriveInput`'s named fields at consumer-compile-time
//! and emits one fragment per field, wrapped in a single impl block.
//!
//! Three splice holes in the per-field template:
//! - `#field_name` — the field's identifier (`name`, `age`, …)
//! - `#field_ty`   — the field's type (`String`, `i32`, …)
//! - `#self_name`  — the consumer struct's identifier
//! - `#method_ident` — a `format_ident!`-built method name, only present
//!                     when [`PerFieldDeriveSpec::method_name_template`]
//!                     is set (e.g. `"with_{}"` → `with_<field>`).
//!
//! Real worked Specs ship in `tatara-rust-examples`.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde::{Deserialize, Serialize};
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold, Ident};

/// What kind of input the derive supports. Today: named-fields structs only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PerFieldTarget {
    /// `Data::Struct(_, Fields::Named(_))`.
    NamedStruct,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerFieldDeriveSpec {
    /// `#[derive(<trait_name>)]` user-facing identifier.
    pub trait_name: Ident,
    /// Input shape we iterate.
    pub target: PerFieldTarget,
    /// `impl <trait_ref> for $Self` when set; inherent impl when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trait_ref: Option<String>,
    /// Per-field code fragment, embedded raw inside a `quote!{}` block.
    /// Use `#field_name` / `#field_ty` / `#method_ident` as splice holes.
    pub per_field_template: String,
    /// `format_ident!` template for the derived method name — `"with_{}"`
    /// → `with_<field>`. Bound to `#method_ident` in the template above.
    /// None means the template uses `#field_name` directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method_name_template: Option<String>,
    /// Optional impl-block prelude before the per-field fragments.
    /// Has access to `#self_name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impl_prelude: Option<String>,
    /// Named fields the consumer wants EXCLUDED from the per-field
    /// iteration. The canonical use is the **InvalidatingSetter**
    /// pattern: a struct has a cache-invalidation marker field (e.g.
    /// `last_seqno: u64`) that every setter resets but that itself
    /// doesn't get a setter. Listing it here filters it out of the
    /// generated impl block; the per-field template body can still
    /// reference `self.<skip_field>` verbatim (resolved by syn at
    /// consumer compile time).
    ///
    /// Empty by default — all named fields emit a method.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip_fields: Vec<String>,
    /// Consumer-side opt-in attribute. When set, the generated macro
    /// only emits methods for fields marked `#[<attr>]`. Lets consumers
    /// choose which fields participate without the derive author
    /// knowing field names. The canonical use is the
    /// **InvalidatingSetter** derive on large structs (e.g. mado's
    /// `TerminalRenderer`) where only 6 of 30 fields should have
    /// public setters.
    ///
    /// `None` = no filtering by attribute (all fields participate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_attribute: Option<String>,
}

impl PerFieldDeriveSpec {
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

impl CompileToCrate for PerFieldDeriveSpec {
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
        "Per-field derive proc-macro emitted from a tatara-rust-derive PerFieldDeriveSpec.",
    )
}

/// Render lib.rs via the typed AST substrate (proc-macro2 + quote +
/// syn + prettyplease). Replaces the prior `push_str` tower per the
/// fleet's TYPED EMISSION directive.
///
/// Key escape trick: parts of the emitted source contain `#`-prefixed
/// tokens that should appear LITERALLY (they are quote interpolations
/// that resolve at consumer compile time, not at emit time). We
/// pre-parse those fragments into `TokenStream` variables and
/// interpolate the WHOLE VARIABLE via `#var` — quote splices the
/// variable's tokens verbatim, including any `#` tokens inside.
fn render_lib_rs(spec: &PerFieldDeriveSpec) -> String {
    let trait_id = format_ident!("{}", spec.trait_name.0);
    let fn_name = format_ident!("{}", spec.fn_name());

    // The per-field template body (user-supplied Rust fragment, may
    // contain `#field_name` etc.). Parsed once as TokenStream; goes
    // verbatim into the inner quote!{} that runs at consumer compile.
    let per_field_body: TokenStream = spec
        .per_field_template
        .parse()
        .expect("per_field_template must parse as TokenStream");

    // The `#self_name` literal as a 2-token stream (Punct(#) + Ident).
    // We need it as a variable to interpolate; writing #self_name
    // directly inside the outer quote would try to bind a non-existent
    // emit-time variable.
    let hash_self_name: TokenStream =
        "#self_name".parse().expect("static literal must parse");
    // Same trick for the inner quote repetition.
    let hash_per_field_repeat: TokenStream =
        "#(#per_field)*".parse().expect("static literal must parse");

    let method_ident_binding: TokenStream = match &spec.method_name_template {
        None => quote! {},
        Some(tpl) => quote! {
            let method_ident = quote::format_ident!(#tpl, field_name.to_string());
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
        Some(p) => p
            .parse()
            .expect("impl_prelude must parse as TokenStream"),
    };

    // Two independent filters: skip_fields (exclude by name) and
    // field_attribute (include only by consumer-side `#[<attr>]`).
    // Both compose: a field passes only when it's not in SKIP_FIELDS
    // AND (no attr filter, OR it carries the attr).
    let skip_const: TokenStream = if spec.skip_fields.is_empty() {
        quote! {}
    } else {
        let lits: Vec<&String> = spec.skip_fields.iter().collect();
        quote! {
            const SKIP_FIELDS: &[&str] = &[#( #lits ),*];
        }
    };

    let skip_filter: TokenStream = if spec.skip_fields.is_empty() {
        quote! {}
    } else {
        quote! {
            .filter(|f| {
                f.ident
                    .as_ref()
                    .is_none_or(|id| !SKIP_FIELDS.contains(&id.to_string().as_str()))
            })
        }
    };

    let attr_filter: TokenStream = match &spec.field_attribute {
        None => quote! {},
        Some(attr_name) => quote! {
            .filter(|f| {
                f.attrs.iter().any(|a| a.path().is_ident(#attr_name))
            })
        },
    };

    let fields_iter: TokenStream = quote! {
        fields.iter() #skip_filter #attr_filter
    };

    // When the spec opts into consumer-side per-field attributes,
    // `#[proc_macro_derive(Trait, attributes(<attr>))]` must list
    // them — otherwise syn rejects the consumer's `#[<attr>]` as
    // unknown.
    let derive_attr: TokenStream = match &spec.field_attribute {
        None => quote! { #[proc_macro_derive(#trait_id)] },
        Some(attr) => {
            let attr_id = format_ident!("{attr}");
            quote! { #[proc_macro_derive(#trait_id, attributes(#attr_id))] }
        }
    };

    let file: TokenStream = quote! {
        #![doc = "GENERATED by tatara-rust-derive::PerFieldDeriveSpec."]
        #![doc = "Do not hand-edit; regenerate from the (defperfield …) source."]

        use proc_macro::TokenStream;
        use quote::quote;
        use syn::{Data, DataStruct, DeriveInput, Fields, parse_macro_input};

        #derive_attr
        pub fn #fn_name(input: TokenStream) -> TokenStream {
            let input = parse_macro_input!(input as DeriveInput);
            let self_name = &input.ident;

            let fields = match &input.data {
                Data::Struct(DataStruct { fields: Fields::Named(named), .. }) => &named.named,
                _ => {
                    return syn::Error::new_spanned(
                        self_name,
                        "PerFieldDerive requires a named-fields struct",
                    )
                    .to_compile_error()
                    .into();
                }
            };

            #skip_const

            let per_field = #fields_iter.map(|f| {
                let field_name = f.ident.as_ref().expect("named field has ident");
                let field_ty = &f.ty;
                #method_ident_binding
                quote! {
                    #per_field_body
                }
            });

            let expanded = quote! {
                #impl_open {
                    #prelude
                    #hash_per_field_repeat
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

    fn getter_all() -> PerFieldDeriveSpec {
        PerFieldDeriveSpec {
            trait_name: Ident::new("GetterAll"),
            target: PerFieldTarget::NamedStruct,
            trait_ref: None,
            per_field_template:
                "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
            method_name_template: None,
            impl_prelude: None,
            skip_fields: vec![],
            field_attribute: None,
        }
    }

    fn with_builder() -> PerFieldDeriveSpec {
        PerFieldDeriveSpec {
            trait_name: Ident::new("WithBuilder"),
            target: PerFieldTarget::NamedStruct,
            trait_ref: None,
            per_field_template: "pub fn #method_ident(mut self, v: #field_ty) -> Self { self.#field_name = v; self }".into(),
            method_name_template: Some("with_{}".into()),
            impl_prelude: None,
            skip_fields: vec![],
            field_attribute: None,
        }
    }

    #[test]
    fn compiles_to_lib_and_cargo() {
        let s = getter_all().compile_to_crate("getter-all-derive").unwrap();
        let files = s.to_files();
        assert!(files.contains_key("Cargo.toml"));
        assert!(files.contains_key("src/lib.rs"));
    }

    #[test]
    fn cargo_toml_is_proc_macro() {
        let s = getter_all().compile_to_crate("g").unwrap();
        let toml = s.to_files().get("Cargo.toml").unwrap().clone();
        assert!(toml.contains("proc-macro = true"));
    }

    // Behavioral contracts now live in `tests/snapshots/` — the
    // emitter's output is byte-pinned via insta. Drift surfaces as
    // a visible diff on `cargo test`. This is the substrate's own
    // `tatara-rust-snapshot` primitive (Tier-E) applied to its own
    // emitter — no string-substring matching, no whitespace bandaids.
    //
    // The two specifically-structural smoke tests below stay because
    // they catch a bug that snapshots wouldn't: a fundamentally
    // malformed emit that happens to be byte-stable (e.g. emitting
    // an empty file would snapshot fine but the contract is broken).
    // They check structural existence, not text shape.

    #[test]
    fn lib_rs_is_parseable_rust() {
        let s = getter_all().compile_to_crate("g").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // The substrate's contract: every emit MUST be valid Rust
        // that syn can parse as a syn::File. The typescape refactor
        // makes this enforced at emit time too, but pinning it here
        // catches regressions if an emitter ever bypasses
        // syn::parse2 + prettyplease.
        let _: syn::File = syn::parse_str(&lib)
            .unwrap_or_else(|e| panic!("emitted lib.rs must parse: {e}\n{lib}"));
    }

    #[test]
    fn lib_rs_declares_one_proc_macro_derive() {
        let s = getter_all().compile_to_crate("g").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        let parsed: syn::File = syn::parse_str(&lib).unwrap();
        let derive_fn_count = parsed
            .items
            .iter()
            .filter(|item| matches!(item,
                syn::Item::Fn(f)
                    if f.attrs.iter().any(|a| a.path().is_ident("proc_macro_derive"))
            ))
            .count();
        assert_eq!(derive_fn_count, 1, "expected exactly one #[proc_macro_derive] fn");
    }

    #[test]
    fn serde_roundtrip() {
        let s = with_builder();
        let j = serde_json::to_string(&s).unwrap();
        let back: PerFieldDeriveSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
