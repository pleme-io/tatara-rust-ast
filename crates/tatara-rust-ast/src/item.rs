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
