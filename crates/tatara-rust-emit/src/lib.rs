//! `tatara-rust-emit` — typed Rust AST → formatted source.
//!
//! Operator-facing entry points wrapping the [`ToRustTokens`] trait:
//! - [`emit_file`] — render any single AST node into a stand-alone file string.
//! - [`format_tokens`] — pretty-print a `TokenStream` via `prettyplease`.
//!
//! Everything else in the L1 emission story (per-node Display impls,
//! pretty-printer config) lives upstream in `tatara-rust-ast`. This crate
//! is the thin operator-shaped facade.

use proc_macro2::TokenStream;
use quote::quote;
use tatara_rust_ast::ToRustTokens;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EmitError {
    #[error("syn could not parse rendered tokens: {0}")]
    Parse(String),
}

/// Render any typed Rust AST node into a `syn::File`-wrapped, pretty-printed
/// Rust source string. Used to write generated `.rs` files to disk.
pub fn emit_file<T: ToRustTokens>(node: &T) -> Result<String, EmitError> {
    let ts = node.to_rust_tokens();
    format_tokens(ts)
}

/// Pretty-print a `TokenStream` via `prettyplease`. Wraps the tokens in a
/// `syn::File` first — the printer expects file-level input.
pub fn format_tokens(ts: TokenStream) -> Result<String, EmitError> {
    let wrapped = quote!(#ts);
    let file: syn::File =
        syn::parse2(wrapped).map_err(|e| EmitError::Parse(e.to_string()))?;
    Ok(prettyplease::unparse(&file))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_ast::{
        Block, Expr, Fn, FnSig, Generics, Ident, Impl, RefKind, Stmt, TypeRef,
    };

    #[test]
    fn emit_file_for_impl() {
        let i = Impl {
            generics: Generics::default(),
            trait_ref: Some(TypeRef::simple("Greet")),
            self_type: TypeRef::simple("Foo"),
            items: vec![Fn {
                sig: FnSig {
                    name: Ident::new("greet"),
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
                            value: "\"hi\"".into(),
                        },
                    }],
                },
            }],
        };
        let out = emit_file(&i).unwrap();
        assert!(out.contains("impl Greet for Foo"));
        assert!(out.contains("fn greet"));
    }
}
