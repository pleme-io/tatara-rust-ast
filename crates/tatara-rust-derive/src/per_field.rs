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
    /// Exhaustive multi-tag classification: every included field must
    /// carry exactly one of [`TagSpec::tags`]'s named attributes, or the
    /// derive emits a `compile_error!()` naming the field (when
    /// [`TagSpec::exhaustive`] is `true`) — or is silently excluded
    /// (when `false`, the N-tag generalization of [`Self::field_attribute`]'s
    /// existing single-tag behavior). Each tag carries its OWN
    /// `per_field_template` (unlike [`Self::per_field_template`], which
    /// is one template applied uniformly) plus any named string
    /// arguments the consumer's attribute must supply
    /// (`#[restart_required(reason = "...")]`'s `reason`).
    ///
    /// Mutually exclusive with [`Self::field_attribute`] at the type
    /// level is NOT enforced here (both can be set), but the generated
    /// derive dispatches through `field_tag` when set — it takes over
    /// per-field rendering, and [`Self::per_field_template`] /
    /// [`Self::field_attribute`] are IGNORED. `skip_fields` still
    /// applies as a pre-filter in both modes.
    ///
    /// `None` = today's uniform-template behavior, unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_tag: Option<TagSpec>,
}

/// The AGGREGATE emission shape for `field_tag` — see
/// [`TagSpec::aggregate`]. When set, `field_tag` stops emitting N
/// independent per-field items and instead emits exactly TWO items,
/// each populated by a per-field-repeated FRAGMENT (not a full item)
/// contributed by [`FieldTag::aggregate_const_entry`] /
/// [`FieldTag::aggregate_stmt`]: a `const` whose array literal holds one
/// entry per field, and a method whose body accumulates one statement
/// per field. This is the shape a real classification trait needs (e.g.
/// `HotSwapClassifier::{FIELD_CLASSES, classify_change}`,
/// `theory/CALHA.md` §4) — comparing `self` against `new` field-by-field
/// inside ONE method, not N separate ones.
///
/// **Every field here MUST be independently balanced-delimiter Rust
/// (parseable as a standalone `TokenStream` on its own)** — the array
/// `[...]` and the method body `{...}` are constructed programmatically
/// via `proc_macro2::Group` at codegen time, never by splitting a
/// bracket/brace across two separately-parsed strings. An earlier draft
/// tried `const_prelude: "... = &["` / `const_epilogue: "];"` and hit a
/// real `LexError` on real `cargo test` — `TokenStream::parse` requires
/// each parsed fragment to be self-balanced; a lone unmatched `[` cannot
/// tokenize. Caught by the real e2e test, not by inspection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateSpec {
    /// The const's signature up to (not including) the array literal —
    /// balanced on its own, e.g.
    /// `"const FIELD_CLASSES: &'static [(&'static str, HotSwapClass)] = "`
    /// (the `[...]` here is a TYPE annotation, already balanced; the
    /// array VALUE's brackets are added programmatically).
    pub const_signature: String,
    /// The method's signature up to (not including) the opening brace —
    /// balanced on its own, e.g.
    /// `"fn classify_change(&self, new: &Self) -> SwapDecision"`.
    pub method_signature: String,
    /// Statements run at the START of the method body, before the
    /// per-field statements — balanced, complete statements, e.g.
    /// `"let mut reasons: Vec<&'static str> = Vec::new();"`.
    pub method_setup: String,
    /// The method body's final expression (no trailing `;` needed) —
    /// balanced on its own, e.g.
    /// `"if reasons.is_empty() { SwapDecision::Free } else { SwapDecision::RequiresRestart(reasons) }"`.
    pub method_return: String,
}

/// One named field-classification tag (`theory/CALHA.md` §4/§6.1's
/// `field_tag`/`TagSpec`, e.g. `["hot_swap", "restart_required"]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldTag {
    /// The consumer-side attribute name, e.g. `"hot_swap"` or
    /// `"restart_required"`.
    pub name: String,
    /// Named string arguments this tag's attribute must supply, e.g.
    /// `["reason"]` for `#[restart_required(reason = "...")]`. Empty for
    /// a bare `#[hot_swap]`. Each name becomes an additional splice hole
    /// in this tag's `per_field_template`, spliced as `#<name>` bound to
    /// the parsed string literal — e.g. `required_args: ["reason"]`
    /// makes `#reason` available in `per_field_template`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_args: Vec<String>,
    /// This tag's own per-field code fragment. Same splice holes as
    /// [`PerFieldDeriveSpec::per_field_template`] (`#field_name`,
    /// `#field_ty`, `#method_ident` when `method_name_template` is set),
    /// PLUS one hole per entry in `required_args`. Ignored when the
    /// enclosing [`TagSpec::aggregate`] is `Some` — use
    /// `aggregate_const_entry`/`aggregate_stmt` instead in that mode.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub per_field_template: String,
    /// AGGREGATE mode only (see [`TagSpec::aggregate`]): this tag's one
    /// array-literal ENTRY (an expression, not an item), spliced inside
    /// [`AggregateSpec::const_prelude`]/`const_epilogue`'s repetition —
    /// e.g. `"(stringify!(#field_name), HotSwapClass::Free),"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate_const_entry: Option<String>,
    /// AGGREGATE mode only: this tag's one STATEMENT fragment, spliced
    /// inside [`AggregateSpec::method_prelude`]/`method_epilogue`'s
    /// repetition. Can reference `self.#field_name` / `new.#field_name`
    /// directly (both are in scope inside the generated method) — e.g.
    /// `"if self.#field_name != new.#field_name { worst = SwapDecision::RequiresRestart(vec![#reason]); }"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate_stmt: Option<String>,
}

/// Exhaustive multi-tag classification config — see
/// [`PerFieldDeriveSpec::field_tag`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagSpec {
    /// The set of tags a field may carry. Must be non-empty
    /// (`compile_to_crate` returns [`AstError`] otherwise) and every
    /// [`FieldTag::name`] must be unique.
    pub tags: Vec<FieldTag>,
    /// `true`: every included field MUST carry exactly one of `tags`, or
    /// the generated derive emits a `compile_error!()` naming the field
    /// and listing the legal tag names. A field carrying MORE than one
    /// tag is always a `compile_error!()` (ambiguous), regardless of
    /// this flag.
    /// `false`: an untagged field is silently excluded from the
    /// generated impl (matches today's single-tag `field_attribute`
    /// behavior, generalized to N tags).
    pub exhaustive: bool,
    /// When set, switches from N-independent-items emission to the
    /// AGGREGATE shape (one const + one method, each populated by a
    /// per-field-repeated fragment) — see [`AggregateSpec`]. Every
    /// [`FieldTag`] must then set `aggregate_const_entry` +
    /// `aggregate_stmt` (validated in `compile_to_crate`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate: Option<AggregateSpec>,
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
        validate_field_tag(self.field_tag.as_ref())?;
        let mut s = CrateScaffold::new(crate_name, "0.1.0");
        s.add_file("Cargo.toml", render_cargo_toml(crate_name));
        s.add_file("src/lib.rs", render_lib_rs(self));
        Ok(s)
    }
}

/// Validates a [`TagSpec`] at spec-authoring time (not consumer-compile
/// time) — a spec author error here is a Rust compile error in THIS
/// crate's own tests / `catalog-instantiate` run, never surfaced as a
/// confusing error deep in a consumer's derive expansion.
fn validate_field_tag(spec: Option<&TagSpec>) -> Result<(), AstError> {
    let Some(spec) = spec else {
        return Ok(());
    };
    if spec.tags.is_empty() {
        return Err(AstError::InvalidSpec(
            "field_tag.tags must be non-empty when field_tag is set".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for tag in &spec.tags {
        if tag.name.is_empty() {
            return Err(AstError::InvalidSpec(
                "field_tag.tags[].name must be non-empty".into(),
            ));
        }
        if !seen.insert(tag.name.as_str()) {
            return Err(AstError::InvalidSpec(format!(
                "field_tag.tags[].name `{}` is declared more than once",
                tag.name
            )));
        }
        if spec.aggregate.is_some() && (tag.aggregate_const_entry.is_none() || tag.aggregate_stmt.is_none()) {
            return Err(AstError::InvalidSpec(format!(
                "field_tag.tags[].name `{}`: aggregate mode requires both aggregate_const_entry and aggregate_stmt",
                tag.name
            )));
        }
        if spec.aggregate.is_none() && tag.per_field_template.is_empty() {
            return Err(AstError::InvalidSpec(format!(
                "field_tag.tags[].name `{}`: per_field_template is required outside aggregate mode",
                tag.name
            )));
        }
    }
    Ok(())
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
    // unknown. `field_tag` mode lists every declared tag name.
    let derive_attr: TokenStream = match (&spec.field_attribute, &spec.field_tag) {
        (_, Some(tag_spec)) => {
            let attr_ids: Vec<syn::Ident> = tag_spec
                .tags
                .iter()
                .map(|t| format_ident!("{}", t.name))
                .collect();
            quote! { #[proc_macro_derive(#trait_id, attributes(#(#attr_ids),*))] }
        }
        (None, None) => quote! { #[proc_macro_derive(#trait_id)] },
        (Some(attr), None) => {
            let attr_id = format_ident!("{attr}");
            quote! { #[proc_macro_derive(#trait_id, attributes(#attr_id))] }
        }
    };

    // `field_tag` mode takes over per-field rendering entirely (its own
    // dispatch closure below), ignoring `per_field_template`/
    // `field_attribute` — see `PerFieldDeriveSpec::field_tag`'s own doc
    // comment for why these stay mutually exclusive AT THE CODEGEN
    // level rather than being enforced unrepresentable at the type
    // level (a spec author setting both is a spec smell, not something
    // this emitter needs to police beyond "field_tag wins").
    //
    // `setup` builds whatever per-field collection(s) the impl body
    // below needs; `impl_body` is the literal tokens placed inside
    // `#impl_open { ... }`. Three shapes: today's uniform template, N
    // independent field_tag items, or field_tag's AGGREGATE shape (one
    // const + one method, each populated by a DIFFERENT per-field
    // fragment collection — see `render_field_tag_aggregate`).
    let (setup, impl_body): (TokenStream, TokenStream) = match &spec.field_tag {
        Some(tag_spec) if tag_spec.aggregate.is_some() => {
            render_field_tag_aggregate(tag_spec, &method_ident_binding, &fields_iter)
        }
        Some(tag_spec) => {
            let setup = render_field_tag_per_field(tag_spec, &method_ident_binding, &fields_iter);
            (setup, quote! { #prelude #hash_per_field_repeat })
        }
        None => {
            let setup = quote! {
                let per_field = #fields_iter.map(|f| {
                    let field_name = f.ident.as_ref().expect("named field has ident");
                    let field_ty = &f.ty;
                    #method_ident_binding
                    quote! {
                        #per_field_body
                    }
                });
            };
            (setup, quote! { #prelude #hash_per_field_repeat })
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

            #setup

            let expanded = quote! {
                #impl_open {
                    #impl_body
                }
            };
            TokenStream::from(expanded)
        }
    };

    let parsed: syn::File =
        syn::parse2(file).expect("emitted lib.rs must parse as syn::File");
    prettyplease::unparse(&parsed)
}

/// Builds the `let per_field = ...;` binding for `field_tag` (exhaustive
/// multi-tag) mode. Generates code that, AT CONSUMER-DERIVE-RUNTIME (i.e.
/// when a downstream crate's struct is being macro-expanded): for each
/// field, determines which of the declared tags it carries (0/1/N+),
/// extracts that tag's required string arguments via
/// `syn::Attribute::parse_nested_meta`, and renders that tag's OWN
/// `per_field_template` — or emits a `compile_error!()` naming the field
/// for the zero-tags (when exhaustive) or multiple-tags case.
fn render_field_tag_per_field(
    tag_spec: &TagSpec,
    method_ident_binding: &TokenStream,
    fields_iter: &TokenStream,
) -> TokenStream {
    let tag_name_lits: Vec<&str> = tag_spec.tags.iter().map(|t| t.name.as_str()).collect();
    let exhaustive = tag_spec.exhaustive;

    let match_arms: Vec<TokenStream> = tag_spec
        .tags
        .iter()
        .map(|tag| render_tag_arm(tag, method_ident_binding))
        .collect();
    let tag_paths: Vec<syn::Ident> = tag_spec
        .tags
        .iter()
        .map(|t| format_ident!("{}", t.name))
        .collect();

    // The literal `#msg` two-token sequence, for splicing into the INNER
    // (consumer-derive-runtime) `quote::quote! { compile_error!(#msg); }`
    // below — same escape trick `render_lib_rs` documents at its top:
    // writing `#msg` directly inside THIS function's own `quote!{}` would
    // have the OUTER (emitter-layer) quote! try to resolve `msg` from
    // ITS OWN scope (where it doesn't exist), instead of splicing the
    // literal tokens for the inner quote! to resolve later.
    let hash_msg: TokenStream = "#msg".parse().expect("static literal must parse");

    let zero_match_arm: TokenStream = if exhaustive {
        quote! {
            0 => {
                let msg = format!(
                    "field `{}` must carry exactly one of: {}",
                    field_name,
                    [#(#tag_name_lits),*].join(", "),
                );
                Some(quote::quote! { compile_error!(#hash_msg); })
            }
        }
    } else {
        quote! {
            0 => None,
        }
    };

    quote! {
        const TAG_NAMES: &[&str] = &[#(#tag_name_lits),*];

        let per_field = #fields_iter.filter_map(|f| {
            let field_name = f.ident.as_ref().expect("named field has ident");
            let field_ty = &f.ty;

            let matched: Vec<&syn::Attribute> = f
                .attrs
                .iter()
                .filter(|a| TAG_NAMES.iter().any(|&n| a.path().is_ident(n)))
                .collect();

            match matched.len() {
                #zero_match_arm
                1 => {
                    let attr = matched[0];
                    #(
                        if attr.path().is_ident(stringify!(#tag_paths)) {
                            #match_arms
                        } else
                    )* {
                        unreachable!("attr matched TAG_NAMES but no tag branch handled it")
                    }
                }
                _ => {
                    let msg = format!(
                        "field `{}` carries more than one hot-swap-style tag ({}) -- exactly one is required",
                        field_name,
                        matched
                            .iter()
                            .filter_map(|a| a.path().get_ident())
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    Some(quote::quote! { compile_error!(#hash_msg); })
                }
            }
        });
    }
}

/// One `if attr.path().is_ident(...) { ... }` arm's body for a single
/// [`FieldTag`]: parses this tag's `required_args` off `attr` (a
/// `compile_error!()` per missing arg, not a panic — a proc-macro panic
/// aborts with an opaque rustc-internal message instead of a clean
/// diagnostic), then renders `tag.per_field_template` with those args
/// (plus `#field_name`/`#field_ty`/`#method_ident`) as splice holes.
fn render_tag_arm(tag: &FieldTag, method_ident_binding: &TokenStream) -> TokenStream {
    let per_field_body: TokenStream = tag
        .per_field_template
        .parse()
        .unwrap_or_else(|e| panic!("tag `{}`'s per_field_template must parse as TokenStream: {e}", tag.name));

    if tag.required_args.is_empty() {
        return quote! {
            #method_ident_binding
            Some(quote::quote! { #per_field_body })
        };
    }

    let arg_idents: Vec<syn::Ident> = tag
        .required_args
        .iter()
        .map(|a| format_ident!("{}", a))
        .collect();
    let arg_names: Vec<&str> = tag.required_args.iter().map(String::as_str).collect();
    let tag_name = &tag.name;
    // Same escape trick as `render_field_tag_per_field`'s `hash_msg` --
    // `msg` below is a variable that exists only at layer 2 (the
    // generated derive's own runtime body), never in THIS function's
    // scope, so it must be spliced as literal tokens, not resolved here.
    let hash_msg: TokenStream = "#msg".parse().expect("static literal must parse");

    quote! {
        #( let mut #arg_idents: Option<syn::LitStr> = None; )*
        let parse_result = attr.parse_nested_meta(|meta| {
            #(
                if meta.path.is_ident(#arg_names) {
                    #arg_idents = Some(meta.value()?.parse()?);
                    return Ok(());
                }
            )*
            Ok(())
        });
        if let Err(e) = parse_result {
            let msg = e.to_string();
            return Some(quote::quote! { compile_error!(#hash_msg); });
        }
        #(
            let #arg_idents = match #arg_idents {
                Some(v) => v,
                None => {
                    let msg = format!(
                        "#[{}] on field `{}` is missing required argument `{}`",
                        #tag_name, field_name, #arg_names,
                    );
                    return Some(quote::quote! { compile_error!(#hash_msg); });
                }
            };
        )*
        #method_ident_binding
        Some(quote::quote! { #per_field_body })
    }
}

/// Builds the `(setup, impl_body)` pair for `field_tag`'s AGGREGATE
/// shape (see [`AggregateSpec`]). `setup` accumulates two parallel
/// `Vec<TokenStream>` (one const-array entry + one method statement per
/// matched field) via a `for` loop (not `.map()`/`.filter_map()` — each
/// field now contributes to TWO outputs, not one, so an explicit loop
/// with two `.push()` calls is the natural shape). `impl_body` wraps
/// each collection in its own `#(#entries)*`-style repetition inside
/// the spec's `const_prelude`/`epilogue` and `method_prelude`/`epilogue`.
fn render_field_tag_aggregate(
    tag_spec: &TagSpec,
    method_ident_binding: &TokenStream,
    fields_iter: &TokenStream,
) -> (TokenStream, TokenStream) {
    let agg = tag_spec
        .aggregate
        .as_ref()
        .expect("render_field_tag_aggregate called with aggregate.is_none()");

    let tag_name_lits: Vec<&str> = tag_spec.tags.iter().map(|t| t.name.as_str()).collect();
    let exhaustive = tag_spec.exhaustive;
    let tag_paths: Vec<syn::Ident> = tag_spec
        .tags
        .iter()
        .map(|t| format_ident!("{}", t.name))
        .collect();
    let match_arms: Vec<TokenStream> = tag_spec
        .tags
        .iter()
        .map(|tag| render_tag_arm_aggregate(tag, method_ident_binding))
        .collect();

    let hash_msg: TokenStream = "#msg".parse().expect("static literal must parse");

    let zero_match_arm: TokenStream = if exhaustive {
        quote! {
            0 => {
                let msg = format!(
                    "field `{}` must carry exactly one of: {}",
                    field_name,
                    [#(#tag_name_lits),*].join(", "),
                );
                method_stmts.push(quote::quote! { compile_error!(#hash_msg); });
            }
        }
    } else {
        quote! {
            0 => {}
        }
    };

    let setup: TokenStream = quote! {
        const TAG_NAMES: &[&str] = &[#(#tag_name_lits),*];

        let mut const_entries: Vec<proc_macro2::TokenStream> = Vec::new();
        let mut method_stmts: Vec<proc_macro2::TokenStream> = Vec::new();

        for f in #fields_iter {
            let field_name = f.ident.as_ref().expect("named field has ident");
            let field_ty = &f.ty;

            let matched: Vec<&syn::Attribute> = f
                .attrs
                .iter()
                .filter(|a| TAG_NAMES.iter().any(|&n| a.path().is_ident(n)))
                .collect();

            match matched.len() {
                #zero_match_arm
                1 => {
                    let attr = matched[0];
                    #(
                        if attr.path().is_ident(stringify!(#tag_paths)) {
                            #match_arms
                        } else
                    )* {
                        unreachable!("attr matched TAG_NAMES but no tag branch handled it")
                    }
                }
                _ => {
                    let msg = format!(
                        "field `{}` carries more than one hot-swap-style tag ({}) -- exactly one is required",
                        field_name,
                        matched
                            .iter()
                            .filter_map(|a| a.path().get_ident())
                            .map(|i| i.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    method_stmts.push(quote::quote! { compile_error!(#hash_msg); });
                }
            }
        }
    };

    // Every AggregateSpec string field is independently balanced-delimiter
    // Rust (see the type's own doc comment for why) -- each parses clean
    // as its own standalone TokenStream, no dangling bracket/brace.
    let const_signature: TokenStream = agg
        .const_signature
        .parse()
        .expect("AggregateSpec.const_signature must parse as TokenStream");
    let method_signature: TokenStream = agg
        .method_signature
        .parse()
        .expect("AggregateSpec.method_signature must parse as TokenStream");
    let method_setup: TokenStream = agg
        .method_setup
        .parse()
        .expect("AggregateSpec.method_setup must parse as TokenStream");
    let method_return: TokenStream = agg
        .method_return
        .parse()
        .expect("AggregateSpec.method_return must parse as TokenStream");
    let hash_method_stmts: TokenStream =
        "#(#method_stmts)*".parse().expect("static literal must parse");

    // The outer `[...]`/`{...}` are built PROGRAMMATICALLY via
    // `proc_macro2::Group` (layer-2 code, appended to `setup`) instead of
    // splitting the bracket/brace across two separately-parsed strings —
    // `TokenStream::parse` requires each parsed fragment to be balanced
    // on its own; an earlier draft's `"... = &["` / `"];"` split hit a
    // real `LexError` on real `cargo test`, not caught by inspection.
    let build_groups: TokenStream = quote! {
        let const_array = {
            let bracket_group = proc_macro2::TokenTree::Group(proc_macro2::Group::new(
                proc_macro2::Delimiter::Bracket,
                {
                    let mut ts = proc_macro2::TokenStream::new();
                    for e in &const_entries {
                        ts.extend(e.clone());
                    }
                    ts
                },
            ));
            // `const_signature`'s type is `&'static [...]` (a slice
            // reference) -- the bracket group alone is an array literal
            // `[(...); N]`, which does not coerce to `&'static [...]` in
            // const position (E0308, caught by the real e2e test, not by
            // inspection). Prepend `&` so the value is a slice reference.
            let mut ts = proc_macro2::TokenStream::new();
            ts.extend(std::iter::once(proc_macro2::TokenTree::Punct(
                proc_macro2::Punct::new('&', proc_macro2::Spacing::Alone),
            )));
            ts.extend(std::iter::once(bracket_group));
            ts
        };
        let method_body = proc_macro2::TokenTree::Group(proc_macro2::Group::new(
            proc_macro2::Delimiter::Brace,
            quote::quote! { #method_setup #hash_method_stmts #method_return },
        ));
    };

    let setup: TokenStream = quote! {
        #setup
        #build_groups
    };

    // `#const_array`/`#method_body` here name LAYER-2 local variables
    // `build_groups` just declared (`let const_array = ...; let
    // method_body = ...;`) -- same escape trick as `hash_msg`/
    // `hash_method_stmts` above: must be spliced as LITERAL tokens, not
    // resolved against THIS function's own scope (which has no such
    // variables).
    let hash_const_array: TokenStream =
        "#const_array".parse().expect("static literal must parse");
    let hash_method_body: TokenStream =
        "#method_body".parse().expect("static literal must parse");

    let impl_body: TokenStream = quote! {
        #const_signature #hash_const_array ;
        #method_signature #hash_method_body
    };

    (setup, impl_body)
}

/// One `if attr.path().is_ident(...) { ... }` arm's body for
/// [`render_field_tag_aggregate`] — the aggregate-mode counterpart of
/// [`render_tag_arm`]. Same required-args parsing (a `compile_error!()`
/// per missing arg, pushed into `method_stmts` — `const_entries` gets
/// nothing for an errored field, which is fine, the build fails either
/// way), but ends by pushing this tag's `aggregate_const_entry` +
/// `aggregate_stmt` fragments into the two accumulators instead of
/// returning a single value.
fn render_tag_arm_aggregate(tag: &FieldTag, method_ident_binding: &TokenStream) -> TokenStream {
    let const_entry_body: TokenStream = tag
        .aggregate_const_entry
        .as_deref()
        .expect("aggregate mode requires aggregate_const_entry (validated in compile_to_crate)")
        .parse()
        .unwrap_or_else(|e| panic!("tag `{}`'s aggregate_const_entry must parse as TokenStream: {e}", tag.name));
    let stmt_body: TokenStream = tag
        .aggregate_stmt
        .as_deref()
        .expect("aggregate mode requires aggregate_stmt (validated in compile_to_crate)")
        .parse()
        .unwrap_or_else(|e| panic!("tag `{}`'s aggregate_stmt must parse as TokenStream: {e}", tag.name));

    let hash_msg: TokenStream = "#msg".parse().expect("static literal must parse");

    if tag.required_args.is_empty() {
        return quote! {
            #method_ident_binding
            const_entries.push(quote::quote! { #const_entry_body });
            method_stmts.push(quote::quote! { #stmt_body });
        };
    }

    let arg_idents: Vec<syn::Ident> = tag
        .required_args
        .iter()
        .map(|a| format_ident!("{}", a))
        .collect();
    let arg_names: Vec<&str> = tag.required_args.iter().map(String::as_str).collect();
    let tag_name = &tag.name;

    quote! {
        #( let mut #arg_idents: Option<syn::LitStr> = None; )*
        let parse_result = attr.parse_nested_meta(|meta| {
            #(
                if meta.path.is_ident(#arg_names) {
                    #arg_idents = Some(meta.value()?.parse()?);
                    return Ok(());
                }
            )*
            Ok(())
        });
        if let Err(e) = parse_result {
            let msg = e.to_string();
            method_stmts.push(quote::quote! { compile_error!(#hash_msg); });
        } else {
            let mut all_present = true;
            #(
                let #arg_idents = match #arg_idents {
                    Some(v) => v,
                    None => {
                        all_present = false;
                        syn::LitStr::new("", proc_macro2::Span::call_site())
                    }
                };
            )*
            if all_present {
                #method_ident_binding
                const_entries.push(quote::quote! { #const_entry_body });
                method_stmts.push(quote::quote! { #stmt_body });
            } else {
                let msg = format!(
                    "#[{}] on field `{}` is missing a required argument",
                    #tag_name, field_name,
                );
                method_stmts.push(quote::quote! { compile_error!(#hash_msg); });
            }
        }
    }
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
            field_tag: None,
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
            field_tag: None,
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

    // ── field_tag (exhaustive multi-tag classification) ─────────────────
    // The shape `theory/CALHA.md` §4/§6.1/§6.2 names: a `HotSwap` derive
    // with `#[hot_swap]` (no args) and `#[restart_required(reason = "...")]`
    // (one required arg), exhaustive over every field.

    fn hot_swap_spec() -> PerFieldDeriveSpec {
        PerFieldDeriveSpec {
            trait_name: Ident::new("HotSwap"),
            target: PerFieldTarget::NamedStruct,
            trait_ref: Some("pleme_hotswap::HotSwapClassifier".into()),
            per_field_template: String::new(), // unused in field_tag mode
            method_name_template: None,
            impl_prelude: None,
            skip_fields: vec![],
            field_attribute: None,
            field_tag: Some(TagSpec {
                exhaustive: true,
                aggregate: None,
                tags: vec![
                    FieldTag {
                        name: "hot_swap".into(),
                        required_args: vec![],
                        per_field_template: "(stringify!(#field_name), pleme_hotswap::HotSwapClass::Free)".into(),
                        aggregate_const_entry: None,
                        aggregate_stmt: None,
                    },
                    FieldTag {
                        name: "restart_required".into(),
                        required_args: vec!["reason".into()],
                        per_field_template:
                            "(stringify!(#field_name), pleme_hotswap::HotSwapClass::RequiresRestart { reason: #reason })"
                                .into(),
                        aggregate_const_entry: None,
                        aggregate_stmt: None,
                    },
                ],
            }),
        }
    }

    #[test]
    fn validate_rejects_empty_tags() {
        let err = validate_field_tag(Some(&TagSpec {
            tags: vec![],
            exhaustive: true,
            aggregate: None,
        }))
        .unwrap_err();
        assert!(matches!(err, AstError::InvalidSpec(_)));
    }

    #[test]
    fn validate_rejects_duplicate_tag_names() {
        let err = validate_field_tag(Some(&TagSpec {
            exhaustive: true,
            aggregate: None,
            tags: vec![
                FieldTag {
                    name: "hot_swap".into(),
                    required_args: vec![],
                    per_field_template: "()".into(),
                    aggregate_const_entry: None,
                    aggregate_stmt: None,
                },
                FieldTag {
                    name: "hot_swap".into(),
                    required_args: vec![],
                    per_field_template: "()".into(),
                    aggregate_const_entry: None,
                    aggregate_stmt: None,
                },
            ],
        }))
        .unwrap_err();
        assert!(matches!(err, AstError::InvalidSpec(_)));
    }

    #[test]
    fn validate_accepts_none() {
        validate_field_tag(None).unwrap();
    }

    #[test]
    fn field_tag_spec_compiles_to_parseable_lib_rs() {
        let s = hot_swap_spec().compile_to_crate("hot-swap-derive").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        let _: syn::File =
            syn::parse_str(&lib).unwrap_or_else(|e| panic!("emitted lib.rs must parse: {e}\n{lib}"));
    }

    #[test]
    fn field_tag_spec_declares_derive_with_all_tag_attributes() {
        let s = hot_swap_spec().compile_to_crate("hot-swap-derive").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // Every declared tag name must be a registered helper attribute,
        // or syn rejects the consumer's `#[hot_swap]`/`#[restart_required]`
        // as unknown at THEIR compile time.
        assert!(lib.contains("hot_swap"));
        assert!(lib.contains("restart_required"));
        assert!(lib.contains("proc_macro_derive"));
    }

    #[test]
    fn field_tag_spec_references_compile_error_for_exhaustiveness() {
        let s = hot_swap_spec().compile_to_crate("hot-swap-derive").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // The generated derive's own SOURCE must contain the machinery
        // that emits compile_error! at consumer-derive-time for an
        // untagged or ambiguously-tagged field. This does not prove the
        // consumer-side behavior (that needs a real compiled-and-invoked
        // proof — see tatara-rust-examples' hotswap trybuild fixtures);
        // it proves the emitter didn't silently drop the exhaustiveness
        // path.
        assert!(lib.contains("compile_error"));
        assert!(lib.contains("must carry exactly one of"));
    }

    #[test]
    fn field_tag_non_exhaustive_omits_zero_match_compile_error() {
        let mut spec = hot_swap_spec();
        if let Some(tag_spec) = spec.field_tag.as_mut() {
            tag_spec.exhaustive = false;
        }
        let s = spec.compile_to_crate("hot-swap-derive").unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // Non-exhaustive mode's zero-tags arm is `0 => None,` -- no
        // "must carry exactly one of" message should be emitted (the
        // ambiguous multi-tag error stays either way).
        assert!(!lib.contains("must carry exactly one of"));
    }
}
