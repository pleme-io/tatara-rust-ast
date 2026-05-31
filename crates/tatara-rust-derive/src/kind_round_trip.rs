//! `KindRoundTripSpec` — attribute-aware **paired-fold** derive primitive.
//!
//! Whereas [`crate::EnumFoldDeriveSpec`] folds variants into ONE method
//! and [`crate::PerVariantDeriveSpec`] emits one method PER variant, this
//! primitive folds the variants into a **pair** of inverse methods —
//! `as_str(&self) -> &'static str` and `from_str_kind(&str) -> Option<Self>`
//! — and (optionally) the byte pair `as_byte`/`from_byte`. The two match
//! tables are by-construction inverses, so the round-trip property
//! `from_str_kind(x.as_str()) == Some(x)` holds for every variant the
//! derive ever sees.
//!
//! It is the farm's first **attribute-aware** spec: the emitted proc-macro
//! registers `#[proc_macro_derive(_, attributes(kind))]` and parses each
//! variant's `#[kind(name = "...", alias = "...", byte = N)]` helper
//! attribute via `syn`'s `parse_nested_meta`. `name` overrides the wire
//! string (default = the bare variant ident); `alias` (repeatable) adds
//! extra accepted `from_str` spellings; `byte` supplies the wire byte when
//! [`KindRoundTripSpec::with_byte`] is set.
//!
//! Survey signal (pleme-io 2026-05-31, mado): ≥6 hand-written paired
//! `match`-table methods — `PointerShape`, `ClipboardKind`, `PromptKind`,
//! `Placement`, `Action`, `DiscoveryOutcome` — each a separate hand-kept
//! `as_str`/`from_str` pair where a dropped arm is a silent round-trip
//! bug. One typed spec collapses them and makes the inverse-table drift
//! impossible by construction.
//!
//! Authoring shape:
//!
//! ```ignore
//! // string-only round-trip:
//! KindRoundTripSpec::kind_str("KindStr")
//! // string + byte round-trip:
//! KindRoundTripSpec::kind_byte("ClipboardKind")
//! ```
//!
//! Consumer:
//!
//! ```ignore
//! #[derive(KindStr)]
//! enum PromptKind {
//!     #[kind(name = "ps1")] Primary,
//!     #[kind(name = "ps2", alias = "cont")] Continuation,
//! }
//! assert_eq!(PromptKind::Primary.as_str(), "ps1");
//! assert_eq!(PromptKind::from_str_kind("cont"), Some(PromptKind::Continuation));
//! ```

use serde::{Deserialize, Serialize};
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold, Ident};

/// Typed spec for a paired `as_str`/`from_str_kind` (+ optional byte)
/// round-trip derive over a **unit-variant** enum.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindRoundTripSpec {
    /// `#[derive(<trait_name>)]` user-facing identifier (e.g. `KindStr`).
    pub trait_name: Ident,
    /// Per-variant helper-attribute name. The consumer writes
    /// `#[<helper_attr>(name = "...", alias = "...", byte = N)]`.
    pub helper_attr: String,
    /// Name of the emitted `&self -> &'static str` method.
    pub as_str_method: String,
    /// Name of the emitted `&str -> Option<Self>` method.
    pub from_str_method: String,
    /// When `true`, also emit `as_byte(&self) -> u8` +
    /// `from_byte(u8) -> Option<Self>`, and require every variant carry a
    /// `#[<helper_attr>(byte = N)]`.
    pub with_byte: bool,
    /// Name of the emitted `&self -> u8` method (used only when
    /// [`Self::with_byte`]).
    pub as_byte_method: String,
    /// Name of the emitted `u8 -> Option<Self>` method (used only when
    /// [`Self::with_byte`]).
    pub from_byte_method: String,
}

impl KindRoundTripSpec {
    /// Canonical string-only round-trip spec with fleet-default method
    /// names (`as_str` / `from_str_kind`) and the `kind` helper attr.
    #[must_use]
    pub fn kind_str(trait_name: impl Into<String>) -> Self {
        Self {
            trait_name: Ident::new(trait_name),
            helper_attr: "kind".into(),
            as_str_method: "as_str".into(),
            from_str_method: "from_str_kind".into(),
            with_byte: false,
            as_byte_method: "as_byte".into(),
            from_byte_method: "from_byte".into(),
        }
    }

    /// Canonical string+byte round-trip spec (string methods plus
    /// `as_byte` / `from_byte`).
    #[must_use]
    pub fn kind_byte(trait_name: impl Into<String>) -> Self {
        Self {
            with_byte: true,
            ..Self::kind_str(trait_name)
        }
    }

    /// `derive_<snake(trait_name)>` — the proc-macro fn identifier.
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

impl CompileToCrate for KindRoundTripSpec {
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
        "Paired round-trip derive proc-macro emitted from a tatara-rust-derive KindRoundTripSpec.",
    )
}

/// The proc-macro `lib.rs` source as a sentinel-bearing template. The five
/// `__UPPER__` sentinels carry the spec's parameterized identifiers; the
/// three `/*__BYTE_*__*/` comment sentinels delimit the optional byte
/// round-trip regions (substituted to `""` when `with_byte` is false).
///
/// All literal `#…` tokens are consumer-compile-time `quote!` splices and
/// must survive verbatim into the emitted crate — they are plain text in
/// this template, never interpolated here. Substitution is by
/// `String::replace` (the `ProcDeriveSpec::derive_fn_body` /
/// `EnumFoldDeriveSpec` idiom), and the result is validated + formatted
/// through `syn::parse_str` + `prettyplease` per ★★ TYPED EMISSION.
const LIB_RS_TEMPLATE: &str = r####"
#![doc = "GENERATED by tatara-rust-derive::KindRoundTripSpec."]
#![doc = "Do not hand-edit; regenerate from the (defkind-round-trip …) source."]

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DataEnum, DeriveInput, Fields, LitStr};
/*__BYTE_USE__*/

#[proc_macro_derive(__TRAIT__, attributes(__ATTR__))]
pub fn __FN__(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let self_name = &input.ident;

    let de = match &input.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        _ => {
            return syn::Error::new_spanned(self_name, "__TRAIT__ requires an enum")
                .to_compile_error()
                .into();
        }
    };

    let mut as_str_arms = Vec::new();
    let mut from_str_arms = Vec::new();
    /*__BYTE_VECS__*/
    let mut errors = Vec::new();

    for v in de.iter() {
        let variant_name = &v.ident;
        if !matches!(v.fields, Fields::Unit) {
            errors.push(
                syn::Error::new_spanned(
                    variant_name,
                    "__TRAIT__ requires unit (fieldless) variants",
                )
                .to_compile_error(),
            );
            continue;
        }

        let mut name = variant_name.to_string();
        let mut aliases: Vec<String> = Vec::new();
        /*__BYTE_DECL__*/

        for attr in &v.attrs {
            if !attr.path().is_ident("__ATTR__") {
                continue;
            }
            let r = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    let s: LitStr = meta.value()?.parse()?;
                    name = s.value();
                } else if meta.path.is_ident("alias") {
                    let s: LitStr = meta.value()?.parse()?;
                    aliases.push(s.value());
                /*__BYTE_PARSE__*/
                } else {
                    return Err(meta.error("unknown `__ATTR__` key (expected name/alias/byte)"));
                }
                Ok(())
            });
            if let Err(e) = r {
                errors.push(e.to_compile_error());
            }
        }

        as_str_arms.push(quote! { Self::#variant_name => #name, });
        from_str_arms.push(quote! { #name => ::core::option::Option::Some(Self::#variant_name), });
        for a in &aliases {
            from_str_arms.push(quote! { #a => ::core::option::Option::Some(Self::#variant_name), });
        }
        /*__BYTE_ARMS__*/
    }

    if !errors.is_empty() {
        let mut ts = proc_macro2::TokenStream::new();
        for e in errors {
            ts.extend(e);
        }
        return ts.into();
    }

    let expanded = quote! {
        impl #self_name {
            #[doc = "Wire string for this variant (paired inverse of `from_str_kind`)."]
            pub fn __AS_STR__(&self) -> &'static str {
                match self {
                    #(#as_str_arms)*
                }
            }
            #[doc = "Parse a variant from its wire string (paired inverse of `as_str`)."]
            pub fn __FROM_STR__(s: &str) -> ::core::option::Option<Self> {
                match s {
                    #(#from_str_arms)*
                    _ => ::core::option::Option::None,
                }
            }
            /*__BYTE_METHODS__*/
        }
    };
    TokenStream::from(expanded)
}
"####;

const BYTE_USE: &str = "use syn::LitInt;";
const BYTE_VECS: &str = "let mut as_byte_arms = Vec::new();\n    let mut from_byte_arms = Vec::new();";
const BYTE_DECL: &str = "let mut byte: ::core::option::Option<u8> = ::core::option::Option::None;";
const BYTE_PARSE: &str = "} else if meta.path.is_ident(\"byte\") {\n                    let n: LitInt = meta.value()?.parse()?;\n                    byte = ::core::option::Option::Some(n.base10_parse()?);";
const BYTE_ARMS: &str = r#"match byte {
            ::core::option::Option::Some(b) => {
                as_byte_arms.push(quote! { Self::#variant_name => #b, });
                from_byte_arms.push(quote! { #b => ::core::option::Option::Some(Self::#variant_name), });
            }
            ::core::option::Option::None => {
                errors.push(
                    syn::Error::new_spanned(
                        variant_name,
                        "__TRAIT__ (with_byte) requires every variant carry #[__ATTR__(byte = N)]",
                    )
                    .to_compile_error(),
                );
            }
        }"#;
const BYTE_METHODS: &str = r#"#[doc = "Wire byte for this variant (paired inverse of `from_byte`)."]
            pub fn __AS_BYTE__(&self) -> u8 {
                match self {
                    #(#as_byte_arms)*
                }
            }
            #[doc = "Parse a variant from its wire byte (paired inverse of `as_byte`)."]
            pub fn __FROM_BYTE__(b: u8) -> ::core::option::Option<Self> {
                match b {
                    #(#from_byte_arms)*
                    _ => ::core::option::Option::None,
                }
            }"#;

/// Render lib.rs by substituting the spec's identifiers + the optional
/// byte regions into [`LIB_RS_TEMPLATE`], then validating + formatting the
/// result through `syn::parse_str` + `prettyplease`.
fn render_lib_rs(spec: &KindRoundTripSpec) -> String {
    let (byte_use, byte_vecs, byte_decl, byte_parse, byte_arms, byte_methods) = if spec.with_byte {
        (BYTE_USE, BYTE_VECS, BYTE_DECL, BYTE_PARSE, BYTE_ARMS, BYTE_METHODS)
    } else {
        ("", "", "", "", "", "")
    };

    let src = LIB_RS_TEMPLATE
        .replace("/*__BYTE_USE__*/", byte_use)
        .replace("/*__BYTE_VECS__*/", byte_vecs)
        .replace("/*__BYTE_DECL__*/", byte_decl)
        .replace("/*__BYTE_PARSE__*/", byte_parse)
        .replace("/*__BYTE_ARMS__*/", byte_arms)
        .replace("/*__BYTE_METHODS__*/", byte_methods)
        .replace("__AS_BYTE__", &spec.as_byte_method)
        .replace("__FROM_BYTE__", &spec.from_byte_method)
        .replace("__AS_STR__", &spec.as_str_method)
        .replace("__FROM_STR__", &spec.from_str_method)
        .replace("__TRAIT__", &spec.trait_name.0)
        .replace("__ATTR__", &spec.helper_attr)
        .replace("__FN__", &spec.fn_name());

    let parsed: syn::File =
        syn::parse_str(&src).expect("emitted lib.rs must parse as syn::File");
    prettyplease::unparse(&parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_snapshot::assert_tokens_contain;

    #[test]
    fn fn_name_snake_cases_trait_id() {
        assert_eq!(KindRoundTripSpec::kind_str("KindStr").fn_name(), "derive_kind_str");
        assert_eq!(KindRoundTripSpec::kind_byte("ClipboardKind").fn_name(), "derive_clipboard_kind");
    }

    #[test]
    fn compiles_to_cargo_and_lib() {
        let s = KindRoundTripSpec::kind_str("KindStr").compile_to_crate("kindstr-derive").unwrap();
        let files = s.to_files();
        assert!(files.contains_key("Cargo.toml"));
        assert!(files.contains_key("src/lib.rs"));
        assert!(files.get("Cargo.toml").unwrap().contains("proc-macro = true"));
    }

    #[test]
    fn registers_helper_attribute() {
        let s = KindRoundTripSpec::kind_str("KindStr").compile_to_crate("k").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // The helper attr MUST be registered or the consumer's
        // #[kind(...)] is rejected by rustc as an unknown attribute.
        assert_tokens_contain!(&lib, "#[proc_macro_derive(KindStr, attributes(kind))]");
        assert_tokens_contain!(&lib, "pub fn derive_kind_str");
    }

    #[test]
    fn string_only_emits_paired_methods_and_no_byte() {
        let s = KindRoundTripSpec::kind_str("KindStr").compile_to_crate("k").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // Multi-token positives go through token-subsequence matching
        // (prettyplease line-wraps `pub fn\n  as_str` inside the inner
        // quote!{} — raw substring would span the wrap; banned by the
        // ★★ TOKEN-STABILITY directive).
        assert_tokens_contain!(&lib, "pub fn as_str");
        assert_tokens_contain!(&lib, "pub fn from_str_kind");
        // byte regions stripped → none of the byte machinery present.
        // Negatives are on single identifier tokens (never wrap-split).
        assert!(!lib.contains("as_byte_arms"), "byte vecs leaked into string-only output");
        assert!(!lib.contains("base10_parse"), "byte parse leaked into string-only output");
        assert!(!lib.contains("as_byte"), "byte method leaked into string-only output");
    }

    #[test]
    fn with_byte_emits_byte_pair_and_byte_required_guard() {
        let s = KindRoundTripSpec::kind_byte("ClipboardKind").compile_to_crate("c").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert_tokens_contain!(&lib, "pub fn as_byte");
        assert_tokens_contain!(&lib, "pub fn from_byte");
        assert!(lib.contains("base10_parse"), "byte parse missing");
        // guard text lives inside one string-literal token — raw contains is fine.
        assert!(lib.contains("requires every variant carry"), "byte-required guard missing");
    }

    #[test]
    fn parses_name_and_alias_keys() {
        let s = KindRoundTripSpec::kind_str("KindStr").compile_to_crate("k").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("meta.path.is_ident(\"name\")"), "name key parse missing");
        assert!(lib.contains("meta.path.is_ident(\"alias\")"), "alias key parse missing");
        assert!(lib.contains("aliases.push"), "alias accumulation missing");
    }

    #[test]
    fn rejects_non_enum_and_non_unit_variants() {
        let s = KindRoundTripSpec::kind_str("KindStr").compile_to_crate("k").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        assert!(lib.contains("requires an enum"), "non-enum guard missing");
        assert!(lib.contains("requires unit (fieldless) variants"), "non-unit guard missing");
    }

    #[test]
    fn no_sentinel_leaks() {
        for spec in [KindRoundTripSpec::kind_str("KindStr"), KindRoundTripSpec::kind_byte("ByteKind")] {
            let lib = spec.compile_to_crate("k").unwrap().to_files().get("src/lib.rs").unwrap().clone();
            assert!(!lib.contains("__TRAIT__"), "trait sentinel leaked");
            assert!(!lib.contains("__FN__"), "fn sentinel leaked");
            assert!(!lib.contains("__AS_STR__"), "as_str sentinel leaked");
            assert!(!lib.contains("__FROM_STR__"), "from_str sentinel leaked");
            assert!(!lib.contains("__ATTR__"), "attr sentinel leaked");
            assert!(!lib.contains("__BYTE_"), "byte region sentinel leaked");
        }
    }

    #[test]
    fn custom_method_names_substitute() {
        let spec = KindRoundTripSpec {
            as_str_method: "wire".into(),
            from_str_method: "parse_wire".into(),
            ..KindRoundTripSpec::kind_str("Wire")
        };
        let lib = spec.compile_to_crate("w").unwrap().to_files().get("src/lib.rs").unwrap().clone();
        assert_tokens_contain!(&lib, "pub fn wire");
        assert_tokens_contain!(&lib, "pub fn parse_wire");
    }

    #[test]
    fn serde_round_trip() {
        let spec = KindRoundTripSpec::kind_byte("ClipboardKind");
        let j = serde_json::to_string(&spec).unwrap();
        let back: KindRoundTripSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(spec, back);
    }
}
