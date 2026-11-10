//! Items and whole files (0.1.8): what a compiler emitting a crate needs
//! beyond a single `fn` or `impl`.
//!
//! A [`File`] is inner attributes plus items. Attributes are typed as a path
//! and its word arguments (`#![forbid(unsafe_code)]`), never as token text,
//! so an emitted file cannot carry an attribute its author did not name.

use proc_macro2::TokenStream;
use quote::quote;
use serde::{Deserialize, Serialize};

use crate::{Expr, Fn, Ident, ToRustTokens, TypeRef, UseStmt};

/// `#![path]` or `#![path(arg, arg)]`, e.g. `#![no_std]`,
/// `#![forbid(unsafe_code)]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InnerAttr {
    pub path: Vec<Ident>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Ident>,
}

impl ToRustTokens for InnerAttr {
    fn to_rust_tokens(&self) -> TokenStream {
        let segs: Vec<_> = self.path.iter().map(ToRustTokens::to_rust_tokens).collect();
        if self.args.is_empty() {
            quote!(#![ #(#segs)::* ])
        } else {
            let a = self.args.iter().map(ToRustTokens::to_rust_tokens);
            quote!(#![ #(#segs)::*( #(#a),* ) ])
        }
    }
}

/// Item visibility.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    #[default]
    Private,
    Pub,
    PubCrate,
}

impl ToRustTokens for Visibility {
    fn to_rust_tokens(&self) -> TokenStream {
        match self {
            Self::Private => quote!(),
            Self::Pub => quote!(pub),
            Self::PubCrate => quote!(pub(crate)),
        }
    }
}

/// A module-level item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Item {
    /// `[vis] fn …`.
    Fn {
        #[serde(default)]
        vis: Visibility,
        item: Fn,
    },
    /// `use a::b;`.
    Use { item: UseStmt },
    /// `[vis] const NAME: T = value;`.
    Const {
        #[serde(default)]
        vis: Visibility,
        name: Ident,
        ty: TypeRef,
        value: Expr,
    },
    /// `#[derive(…)] [vis] struct Name { field: T, … }` (0.1.11).
    Struct {
        #[serde(default)]
        vis: Visibility,
        name: Ident,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        derives: Vec<Ident>,
        #[serde(default)]
        fields: Vec<StructField>,
    },
    /// `#[derive(…)] [vis] enum Name { Unit, Named { field: T }, … }`
    /// (0.1.11). Each variant holds only its own fields, so a value is as
    /// large as its largest variant and no larger.
    Enum {
        #[serde(default)]
        vis: Visibility,
        name: Ident,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        derives: Vec<Ident>,
        variants: Vec<Variant>,
    },
}

/// A named field of a struct or a struct variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructField {
    #[serde(default)]
    pub vis: Visibility,
    pub name: Ident,
    pub ty: TypeRef,
}

impl ToRustTokens for StructField {
    fn to_rust_tokens(&self) -> TokenStream {
        let (v, n, t) = (
            self.vis.to_rust_tokens(),
            self.name.to_rust_tokens(),
            self.ty.to_rust_tokens(),
        );
        quote!(#v #n: #t)
    }
}

/// An enum variant: a struct variant when it names fields, a tuple variant
/// when it lists element types, else a unit variant. Naming both is refused
/// when the file is emitted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variant {
    pub name: Ident,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<StructField>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elems: Vec<TypeRef>,
}

impl ToRustTokens for Variant {
    fn to_rust_tokens(&self) -> TokenStream {
        let n = self.name.to_rust_tokens();
        let f = self.fields.iter().map(ToRustTokens::to_rust_tokens);
        let e = self.elems.iter().map(ToRustTokens::to_rust_tokens);
        match (self.fields.is_empty(), self.elems.is_empty()) {
            (true, true) => quote!(#n),
            (false, true) => quote!(#n { #(#f),* }),
            (true, false) => quote!(#n(#(#e),*)),
            (false, false) => quote!(#n { #(#f),* } (#(#e),*)),
        }
    }
}

fn derive_tokens(derives: &[Ident]) -> TokenStream {
    if derives.is_empty() {
        quote!()
    } else {
        let d = derives.iter().map(ToRustTokens::to_rust_tokens);
        quote!(#[derive(#(#d),*)])
    }
}

impl ToRustTokens for Item {
    fn to_rust_tokens(&self) -> TokenStream {
        match self {
            Self::Fn { vis, item } => {
                let (v, f) = (vis.to_rust_tokens(), item.to_rust_tokens());
                quote!(#v #f)
            }
            Self::Use { item } => item.to_rust_tokens(),
            Self::Const {
                vis,
                name,
                ty,
                value,
            } => {
                let (v, n, t, e) = (
                    vis.to_rust_tokens(),
                    name.to_rust_tokens(),
                    ty.to_rust_tokens(),
                    value.to_rust_tokens(),
                );
                quote!(#v const #n: #t = #e;)
            }
            Self::Struct {
                vis,
                name,
                derives,
                fields,
            } => {
                let (d, v, n) = (
                    derive_tokens(derives),
                    vis.to_rust_tokens(),
                    name.to_rust_tokens(),
                );
                let f = fields.iter().map(ToRustTokens::to_rust_tokens);
                quote!(#d #v struct #n { #(#f),* })
            }
            Self::Enum {
                vis,
                name,
                derives,
                variants,
            } => {
                let (d, v, n) = (
                    derive_tokens(derives),
                    vis.to_rust_tokens(),
                    name.to_rust_tokens(),
                );
                let vs = variants.iter().map(ToRustTokens::to_rust_tokens);
                quote!(#d #v enum #n { #(#vs),* })
            }
        }
    }
}

/// A whole source file: inner attributes, then items.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attrs: Vec<InnerAttr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
}

impl ToRustTokens for File {
    fn to_rust_tokens(&self) -> TokenStream {
        let a = self.attrs.iter().map(ToRustTokens::to_rust_tokens);
        let i = self.items.iter().map(ToRustTokens::to_rust_tokens);
        quote!(#(#a)* #(#i)*)
    }
}
