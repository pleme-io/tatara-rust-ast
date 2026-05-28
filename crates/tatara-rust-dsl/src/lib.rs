//! `tatara-rust-dsl` — declarative-macro DSL for ergonomic Spec construction.
//!
//! Each macro is a thin shape-converter — `defperfield!` becomes a
//! [`PerFieldDeriveSpec`] literal, etc. The DSL is meta: it's a
//! `macro_rules!` that COULD itself be authored via `MacroRulesSpec`
//! (self-hosting), but ships hand-written for now for compile-speed
//! and to break a chicken-and-egg dependency on the rest of the
//! workspace at consumer-build time.
//!
//! Examples:
//!
//! ```
//! use tatara_rust_dsl::{defperfield, defprocderive};
//! use tatara_rust_derive::{ProcDeriveSpec, PerFieldDeriveSpec};
//!
//! let getter: PerFieldDeriveSpec = defperfield! {
//!     trait_name: "GetterAll",
//!     template: "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }",
//! };
//! assert_eq!(getter.trait_name.0, "GetterAll");
//!
//! let marker: ProcDeriveSpec = defprocderive!("Marker", vec![]);
//! assert_eq!(marker.trait_name.0, "Marker");
//! ```

/// Build a [`tatara_rust_derive::PerFieldDeriveSpec`] from `key: value` pairs.
///
/// Required: `trait_name` (string), `template` (string).
/// Optional: `method_template` (string), `prelude` (string), `trait_ref` (string).
#[macro_export]
macro_rules! defperfield {
    (
        trait_name: $tname:expr,
        template: $tpl:expr
        $(, method_template: $mt:expr)?
        $(, trait_ref: $tr:expr)?
        $(, prelude: $prelude:expr)?
        $(,)?
    ) => {
        $crate::__private::PerFieldDeriveSpec {
            trait_name: $crate::__private::Ident::new($tname),
            target: $crate::__private::PerFieldTarget::NamedStruct,
            trait_ref: $crate::__opt_string!($($tr)?),
            per_field_template: ($tpl).to_string(),
            method_name_template: $crate::__opt_string!($($mt)?),
            impl_prelude: $crate::__opt_string!($($prelude)?),
            skip_fields: vec![],
            field_attribute: None,
        }
    };
}

/// Build a [`tatara_rust_derive::PerVariantDeriveSpec`].
///
/// Required: `trait_name`, `template`.
/// Optional: `method_template`, `trait_ref`, `prelude`.
#[macro_export]
macro_rules! defpervariant {
    (
        trait_name: $tname:expr,
        template: $tpl:expr
        $(, method_template: $mt:expr)?
        $(, trait_ref: $tr:expr)?
        $(, prelude: $prelude:expr)?
        $(,)?
    ) => {
        $crate::__private::PerVariantDeriveSpec {
            trait_name: $crate::__private::Ident::new($tname),
            variant_shape: $crate::__private::VariantShape::Any,
            trait_ref: $crate::__opt_string!($($tr)?),
            per_variant_template: ($tpl).to_string(),
            method_name_template: $crate::__opt_string!($($mt)?),
            impl_prelude: $crate::__opt_string!($($prelude)?),
        }
    };
}

/// Build a [`tatara_rust_derive::ProcDeriveSpec`] from a trait name +
/// a Vec<Fn> of items.
#[macro_export]
macro_rules! defprocderive {
    ( $tname:expr, $items:expr ) => {
        $crate::__private::ProcDeriveSpec::new($tname, $items)
    };
}

/// Helper — turn an optional `expr` into `Some(expr.to_string())`,
/// or `None` when the option was omitted.
#[doc(hidden)]
#[macro_export]
macro_rules! __opt_string {
    () => {
        ::core::option::Option::None
    };
    ($v:expr) => {
        ::core::option::Option::Some(($v).to_string())
    };
}

/// Re-exports the macro bodies consume. Hidden because consumers should
/// reach the types through `tatara-rust-ast` / `tatara-rust-derive`.
#[doc(hidden)]
pub mod __private {
    pub use tatara_rust_ast::Ident;
    pub use tatara_rust_derive::{
        PerFieldDeriveSpec, PerFieldTarget, PerVariantDeriveSpec, ProcDeriveSpec, VariantShape,
    };
}

#[cfg(test)]
mod tests {
    use tatara_rust_derive::{PerFieldDeriveSpec, PerVariantDeriveSpec, ProcDeriveSpec};

    #[test]
    fn defperfield_minimal() {
        let s: PerFieldDeriveSpec = crate::defperfield! {
            trait_name: "GetterAll",
            template: "pub fn #field_name() {}",
        };
        assert_eq!(s.trait_name.0, "GetterAll");
        assert_eq!(s.per_field_template, "pub fn #field_name() {}");
        assert_eq!(s.method_name_template, None);
        assert_eq!(s.impl_prelude, None);
    }

    #[test]
    fn defperfield_with_method_template() {
        let s: PerFieldDeriveSpec = crate::defperfield! {
            trait_name: "WithBuilder",
            template: "pub fn #method_ident(self, v: #field_ty) -> Self { self }",
            method_template: "with_{}",
        };
        assert_eq!(s.method_name_template.as_deref(), Some("with_{}"));
    }

    #[test]
    fn defperfield_with_all_optionals() {
        let s: PerFieldDeriveSpec = crate::defperfield! {
            trait_name: "Foo",
            template: "// per",
            method_template: "x_{}",
            trait_ref: "MyTrait",
            prelude: "const X: usize = 0;",
        };
        assert_eq!(s.trait_ref.as_deref(), Some("MyTrait"));
        assert_eq!(s.impl_prelude.as_deref(), Some("const X: usize = 0;"));
    }

    #[test]
    fn defpervariant_minimal() {
        let s: PerVariantDeriveSpec = crate::defpervariant! {
            trait_name: "IsVariant",
            template: "pub fn #method_ident(&self) -> bool { false }",
            method_template: "is_{}",
        };
        assert_eq!(s.trait_name.0, "IsVariant");
    }

    #[test]
    fn defprocderive_minimal() {
        let s: ProcDeriveSpec = crate::defprocderive!("Marker", vec![]);
        assert_eq!(s.trait_name.0, "Marker");
    }
}
