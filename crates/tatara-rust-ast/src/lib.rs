//! `tatara-rust-ast` — typed Rust AST primitives.
//!
//! L0 of the `(defrust …)` ladder. Everything here is plain typed data
//! (Serde + serde_json round-trippable). Emission, parsing, and
//! macro-shape primitives live in sibling crates that consume these
//! types via the three load-bearing traits:
//!
//! - [`ToRustTokens`] — every node materializes to `proc_macro2::TokenStream`.
//! - [`FromSyn`] — every node parses from its `syn` counterpart for round-trip tests.
//! - [`CompileToCrate`] — macro-shape specs (derive, attr, fn-like) compile
//!   to a complete proc-macro crate scaffold (Cargo.toml + src/lib.rs).
//!
//! Scope of v0: enough primitives to author a working derive proc macro.
//! Extension shape: every new node type follows the same "impl all three
//! traits, add a serde-tagged enum arm" template. Single-shape extension.

use proc_macro2::TokenStream;
use quote::quote;
use serde::{Deserialize, Serialize};

pub mod cargo;
pub mod error;
pub mod from_syn;
pub mod scaffold;
pub mod traits;

pub use cargo::render_proc_macro_cargo_toml;
pub use error::AstError;
pub use scaffold::{CrateScaffold, FileEntry};
pub use traits::{CompileToCrate, FromSyn, ToRustTokens};

// ============================================================================
// IDENTIFIERS, PATHS, TYPES
// ============================================================================

/// A Rust identifier — `Foo`, `my_fn`, `T`. Validation deferred to `syn::Ident::new`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ident(pub String);

impl Ident {
    #[must_use]
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl ToRustTokens for Ident {
    fn to_rust_tokens(&self) -> TokenStream {
        let id = syn::Ident::new(&self.0, proc_macro2::Span::call_site());
        quote!(#id)
    }
}

/// A type reference — `String`, `Vec<T>`, `&'a str`. Recursive on `generics`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeRef {
    pub ident: Ident,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generics: Vec<TypeRef>,
    /// Reference shape (`&` or `&mut`) plus optional lifetime. None for owned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<RefKind>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum RefKind {
    /// `&` or `&'<lifetime>`.
    Shared {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lifetime: Option<String>,
    },
    /// `&mut` or `&'<lifetime> mut`.
    Mut {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lifetime: Option<String>,
    },
}

impl RefKind {
    /// Shorthand `&` (no lifetime).
    #[must_use]
    pub fn shared() -> Self {
        Self::Shared { lifetime: None }
    }
    /// Shorthand `&'static` / `&'a`.
    #[must_use]
    pub fn shared_lifetime(lt: impl Into<String>) -> Self {
        Self::Shared {
            lifetime: Some(lt.into()),
        }
    }
    /// Shorthand `&mut`.
    #[must_use]
    pub fn mut_() -> Self {
        Self::Mut { lifetime: None }
    }
}

impl TypeRef {
    #[must_use]
    pub fn simple(ident: impl Into<String>) -> Self {
        Self {
            ident: Ident::new(ident),
            generics: vec![],
            reference: None,
        }
    }

    #[must_use]
    pub fn generic(ident: impl Into<String>, generics: Vec<TypeRef>) -> Self {
        Self {
            ident: Ident::new(ident),
            generics,
            reference: None,
        }
    }
}

impl ToRustTokens for TypeRef {
    fn to_rust_tokens(&self) -> TokenStream {
        let id = self.ident.to_rust_tokens();
        let base = if self.generics.is_empty() {
            quote!(#id)
        } else {
            let gs = self.generics.iter().map(ToRustTokens::to_rust_tokens);
            quote!(#id<#(#gs),*>)
        };
        match &self.reference {
            None => base,
            Some(RefKind::Shared { lifetime: None }) => quote!(& #base),
            Some(RefKind::Shared { lifetime: Some(lt) }) => {
                let lt = lifetime_tok(lt);
                quote!(& #lt #base)
            }
            Some(RefKind::Mut { lifetime: None }) => quote!(&mut #base),
            Some(RefKind::Mut { lifetime: Some(lt) }) => {
                let lt = lifetime_tok(lt);
                quote!(& #lt mut #base)
            }
        }
    }
}

// ============================================================================
// GENERICS
// ============================================================================

/// Generic parameter list (`<T: Trait, 'a>`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generics {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<TypeParam>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lifetimes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeParam {
    pub name: Ident,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bounds: Vec<TypeRef>,
}

impl Generics {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.type_params.is_empty() && self.lifetimes.is_empty()
    }
}

/// Build a `syn::Lifetime` from a name that may or may not include the
/// leading apostrophe (`'a` and `a` are both accepted; both render `'a`).
fn lifetime_tok(name: &str) -> syn::Lifetime {
    let owned;
    let s = if name.starts_with('\'') {
        name
    } else {
        owned = format!("'{name}");
        &owned
    };
    syn::Lifetime::new(s, proc_macro2::Span::call_site())
}

impl ToRustTokens for Generics {
    fn to_rust_tokens(&self) -> TokenStream {
        if self.is_empty() {
            return TokenStream::new();
        }
        let lts = self.lifetimes.iter().map(|lt| {
            let lt = lifetime_tok(lt);
            quote!(#lt)
        });
        let tps = self.type_params.iter().map(|p| {
            let id = p.name.to_rust_tokens();
            if p.bounds.is_empty() {
                quote!(#id)
            } else {
                let bs = p.bounds.iter().map(ToRustTokens::to_rust_tokens);
                quote!(#id: #(#bs)+*)
            }
        });
        quote!(< #(#lts,)* #(#tps),* >)
    }
}

// ============================================================================
// EXPRESSIONS + STATEMENTS + BLOCKS
// ============================================================================

/// Subset of Rust expressions sufficient for derive-macro bodies.
/// Extension shape: add a variant + Display impl + ToRustTokens arm.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Expr {
    /// A literal — `"hi"`, `42`, `true`.
    Literal { value: String },
    /// A path expression — `self`, `name`, `crate::Foo::bar`.
    Path { segments: Vec<Ident> },
    /// Method call — `x.foo(arg)`.
    MethodCall {
        receiver: Box<Expr>,
        method: Ident,
        args: Vec<Expr>,
    },
    /// Macro invocation — `format!("...", x)`. Stored as raw token text
    /// because macros are arbitrary; downstream re-parses with `syn`.
    MacroCall {
        path: Vec<Ident>,
        tokens: String,
    },
    /// A `quote!{}` template — escape hatch for emission. `tokens` is the
    /// interior text of the `quote!{}`. Used inside derive-macro fn bodies
    /// to defer to `proc_macro2`'s splicing.
    QuoteTemplate { tokens: String },
}

impl ToRustTokens for Expr {
    fn to_rust_tokens(&self) -> TokenStream {
        match self {
            Self::Literal { value } => {
                let v: proc_macro2::TokenStream = value.parse().unwrap_or_else(|_| {
                    let s = value;
                    quote!(#s)
                });
                v
            }
            Self::Path { segments } => {
                let segs: Vec<_> = segments.iter().map(ToRustTokens::to_rust_tokens).collect();
                quote!( #(#segs)::* )
            }
            Self::MethodCall {
                receiver,
                method,
                args,
            } => {
                let r = receiver.to_rust_tokens();
                let m = method.to_rust_tokens();
                let a = args.iter().map(ToRustTokens::to_rust_tokens);
                quote!(#r.#m(#(#a),*))
            }
            Self::MacroCall { path, tokens } => {
                let segs: Vec<_> = path.iter().map(ToRustTokens::to_rust_tokens).collect();
                let body: TokenStream = tokens.parse().unwrap_or_else(|_| quote!());
                quote!( #(#segs)::*!(#body) )
            }
            Self::QuoteTemplate { tokens } => {
                let body: TokenStream = tokens.parse().unwrap_or_else(|_| quote!());
                quote!(quote::quote! { #body })
            }
        }
    }
}

/// Statement — `let x = …;`, `expr;`, or `expr` (final tail).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Stmt {
    Let { name: Ident, value: Expr },
    Semi { expr: Expr },
    Tail { expr: Expr },
}

impl ToRustTokens for Stmt {
    fn to_rust_tokens(&self) -> TokenStream {
        match self {
            Self::Let { name, value } => {
                let n = name.to_rust_tokens();
                let v = value.to_rust_tokens();
                quote!(let #n = #v;)
            }
            Self::Semi { expr } => {
                let e = expr.to_rust_tokens();
                quote!(#e;)
            }
            Self::Tail { expr } => expr.to_rust_tokens(),
        }
    }
}

/// `{ stmt; stmt; tail }` block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub stmts: Vec<Stmt>,
}

impl ToRustTokens for Block {
    fn to_rust_tokens(&self) -> TokenStream {
        let s = self.stmts.iter().map(ToRustTokens::to_rust_tokens);
        quote!({ #(#s)* })
    }
}

// ============================================================================
// FN + IMPL — the impl block a derive macro produces
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnParam {
    pub name: String,
    pub ty: TypeRef,
}

impl ToRustTokens for FnParam {
    fn to_rust_tokens(&self) -> TokenStream {
        if self.name == "self" {
            return match &self.ty.reference {
                Some(RefKind::Shared { .. }) => quote!(&self),
                Some(RefKind::Mut { .. }) => quote!(&mut self),
                None => quote!(self),
            };
        }
        let name = syn::Ident::new(&self.name, proc_macro2::Span::call_site());
        let ty = self.ty.to_rust_tokens();
        quote!(#name: #ty)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnSig {
    pub name: Ident,
    #[serde(default, skip_serializing_if = "Generics::is_empty")]
    pub generics: Generics,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<FnParam>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_type: Option<TypeRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fn {
    pub sig: FnSig,
    pub body: Block,
}

impl ToRustTokens for Fn {
    fn to_rust_tokens(&self) -> TokenStream {
        let name = self.sig.name.to_rust_tokens();
        let gens = self.sig.generics.to_rust_tokens();
        let params = self.sig.params.iter().map(ToRustTokens::to_rust_tokens);
        let ret = match &self.sig.return_type {
            None => quote!(),
            Some(t) => {
                let t = t.to_rust_tokens();
                quote!(-> #t)
            }
        };
        let body = self.body.to_rust_tokens();
        quote!(fn #name #gens (#(#params),*) #ret #body)
    }
}

/// `impl <Trait> for <TypeRef> { fn… fn… }`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Impl {
    #[serde(default, skip_serializing_if = "Generics::is_empty")]
    pub generics: Generics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trait_ref: Option<TypeRef>,
    pub self_type: TypeRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Fn>,
}

impl ToRustTokens for Impl {
    fn to_rust_tokens(&self) -> TokenStream {
        let gens = self.generics.to_rust_tokens();
        let target = self.self_type.to_rust_tokens();
        let trait_for = match &self.trait_ref {
            None => quote!(),
            Some(t) => {
                let t = t.to_rust_tokens();
                quote!(#t for)
            }
        };
        let items = self.items.iter().map(ToRustTokens::to_rust_tokens);
        quote!(impl #gens #trait_for #target { #(#items)* })
    }
}

/// `use a::b::c;` — emitted at the top of a generated proc-macro crate's lib.rs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UseStmt {
    pub path: Vec<Ident>,
}

impl ToRustTokens for UseStmt {
    fn to_rust_tokens(&self) -> TokenStream {
        let segs: Vec<_> = self.path.iter().map(ToRustTokens::to_rust_tokens).collect();
        quote!(use #(#segs)::*;)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prettyplease::unparse;
    use syn::{File, parse2};

    fn render(item: TokenStream) -> String {
        let wrapped: TokenStream = quote!(#item);
        let f: File = parse2(quote!( #wrapped )).expect("parse file");
        unparse(&f)
    }

    #[test]
    fn ident_roundtrips() {
        let id = Ident::new("Foo");
        assert_eq!(id.to_rust_tokens().to_string(), "Foo");
    }

    #[test]
    fn type_ref_simple() {
        let t = TypeRef::simple("String");
        assert_eq!(t.to_rust_tokens().to_string(), "String");
    }

    #[test]
    fn type_ref_generic() {
        let t = TypeRef::generic("Vec", vec![TypeRef::simple("u8")]);
        assert_eq!(t.to_rust_tokens().to_string(), "Vec < u8 >");
    }

    #[test]
    fn impl_with_method_renders() {
        let i = Impl {
            generics: Generics::default(),
            trait_ref: Some(TypeRef::simple("StaticName")),
            self_type: TypeRef::simple("Foo"),
            items: vec![Fn {
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
                        expr: Expr::Literal {
                            value: "\"Foo\"".to_string(),
                        },
                    }],
                },
            }],
        };
        let rendered = render(i.to_rust_tokens());
        assert!(rendered.contains("impl StaticName for Foo"));
        assert!(rendered.contains("fn type_name"));
    }

    #[test]
    fn serde_round_trip_impl() {
        let i = Impl {
            generics: Generics::default(),
            trait_ref: Some(TypeRef::simple("Greet")),
            self_type: TypeRef::simple("Bar"),
            items: vec![],
        };
        let j = serde_json::to_string(&i).unwrap();
        let back: Impl = serde_json::from_str(&j).unwrap();
        assert_eq!(i, back);
    }
}
