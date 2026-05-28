//! The three load-bearing traits every typed Rust AST node implements.
//!
//! - [`ToRustTokens`] — every node materializes to `proc_macro2::TokenStream`.
//!   Required for all primitive types — that's what makes them generative.
//! - [`FromSyn`] — every node parses from its `syn` counterpart. Optional but
//!   strongly recommended; enables `(defrust …)` round-trip testing.
//! - [`CompileToCrate`] — macro-shape specs (derive / attr / fn-like) compile
//!   to a complete proc-macro crate scaffold. The L2 contract.

use crate::scaffold::CrateScaffold;
use crate::error::AstError;
use proc_macro2::TokenStream;

/// Every typed AST node renders to a Rust `TokenStream`. The output is what
/// either gets inlined into a `quote!{}` body OR written to source via
/// `prettyplease::unparse`.
pub trait ToRustTokens {
    fn to_rust_tokens(&self) -> TokenStream;
}

/// Round-trip parse: given a `syn` AST node of type `T`, produce the typed
/// equivalent. Used for property tests ("emit then re-parse == identity").
pub trait FromSyn<T> {
    fn from_syn(t: &T) -> Result<Self, AstError>
    where
        Self: Sized;
}

/// Macro-shape specs implement this to compile down to a complete proc-macro
/// crate scaffold (Cargo.toml + src/lib.rs + tests). One `(defprocderive …)`
/// in a tlisp file ≡ one shippable Cargo crate on disk.
pub trait CompileToCrate {
    /// `crate_name` becomes the crate's `[package].name` and the proc-macro's
    /// dispatch name (`my-derive` → `my_derive` as the macro name).
    fn compile_to_crate(&self, crate_name: &str) -> Result<CrateScaffold, AstError>;
}
