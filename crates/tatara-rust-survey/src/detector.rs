//! `detector` — typed per-pattern classifier registry.
//!
//! Each known [`MatchedPattern`] is owned by a zero-sized
//! [`Detector`] impl. The registry [`detectors()`] returns every
//! detector the substrate ships. [`crate::classify_fn`] becomes a
//! 3-line iteration over the registry — no hand-coded match arm.
//!
//! Adding a 5th pattern is:
//!   1. Add the variant to [`MatchedPattern`].
//!   2. Add a zero-sized struct + `impl Detector`.
//!   3. Append it to [`detectors()`].
//!
//! No other change anywhere in the survey crate.

use crate::MatchedPattern;
use syn::ImplItemFn;

/// One pattern's typed classifier — owns its [`MatchedPattern`]
/// identity, the derive crate + trait names to surface to the
/// operator, and the per-function matching rule.
pub trait Detector: Send + Sync {
    /// Which [`MatchedPattern`] this detector produces on a match.
    fn pattern(&self) -> MatchedPattern;
    /// Crate name (kebab-case) that publishes the matching derive.
    fn derive_crate(&self) -> &'static str;
    /// Trait identifier to write inside `#[derive(…)]`.
    fn derive_trait(&self) -> &'static str;
    /// Does this function body match the canonical shape this
    /// detector recognizes?
    fn matches(&self, f: &ImplItemFn) -> bool;
}

/// The canonical fleet detector registry. Order is meaningful only
/// in that the first matching detector wins — pick disjoint shapes
/// so order is irrelevant. (Today all four are disjoint by fn-name
/// prefix + body shape.)
pub fn detectors() -> &'static [&'static dyn Detector] {
    &[
        &GetterAllDetector,
        &SetterAllDetector,
        &WithBuilderDetector,
        &IsVariantDetector,
    ]
}

// ─────────────────────────────────────────────────────────────────────
// GetterAll: `pub fn <field>(&self) -> &<T> { &self.<field> }`
// ─────────────────────────────────────────────────────────────────────

pub struct GetterAllDetector;

impl Detector for GetterAllDetector {
    fn pattern(&self) -> MatchedPattern {
        MatchedPattern::GetterAll
    }
    fn derive_crate(&self) -> &'static str {
        "pleme-getter-derive"
    }
    fn derive_trait(&self) -> &'static str {
        "GetterAll"
    }
    fn matches(&self, f: &ImplItemFn) -> bool {
        let name = f.sig.ident.to_string();
        is_getter_shape(f, &name)
    }
}

fn is_getter_shape(f: &ImplItemFn, field: &str) -> bool {
    if !matches!(
        f.sig.inputs.first(),
        Some(syn::FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none()
    ) {
        return false;
    }
    if f.sig.inputs.len() != 1 {
        return false;
    }
    let syn::ReturnType::Type(_, ret) = &f.sig.output else {
        return false;
    };
    if !matches!(ret.as_ref(), syn::Type::Reference(_)) {
        return false;
    }
    let stmts = &f.block.stmts;
    if stmts.len() != 1 {
        return false;
    }
    let syn::Stmt::Expr(expr, None) = &stmts[0] else {
        return false;
    };
    let syn::Expr::Reference(r) = expr else {
        return false;
    };
    matches_field_access(&r.expr, field)
}

// ─────────────────────────────────────────────────────────────────────
// SetterAll: `pub fn set_<field>(&mut self, v: <T>) { self.<field> = v; }`
// ─────────────────────────────────────────────────────────────────────

pub struct SetterAllDetector;

impl Detector for SetterAllDetector {
    fn pattern(&self) -> MatchedPattern {
        MatchedPattern::SetterAll
    }
    fn derive_crate(&self) -> &'static str {
        "pleme-setter-derive"
    }
    fn derive_trait(&self) -> &'static str {
        "SetterAll"
    }
    fn matches(&self, f: &ImplItemFn) -> bool {
        let name = f.sig.ident.to_string();
        let Some(field) = name.strip_prefix("set_") else {
            return false;
        };
        is_setter_shape(f, field)
    }
}

fn is_setter_shape(f: &ImplItemFn, field: &str) -> bool {
    if !matches!(
        f.sig.inputs.first(),
        Some(syn::FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_some()
    ) {
        return false;
    }
    if f.sig.inputs.len() != 2 {
        return false;
    }
    if !matches!(f.sig.output, syn::ReturnType::Default) {
        return false;
    }
    let stmts = &f.block.stmts;
    if stmts.len() != 1 {
        return false;
    }
    matches_setter_assign(&stmts[0], field)
}

// ─────────────────────────────────────────────────────────────────────
// WithBuilder: `pub fn with_<field>(mut self, v: <T>) -> Self { self.<field> = v; self }`
// ─────────────────────────────────────────────────────────────────────

pub struct WithBuilderDetector;

impl Detector for WithBuilderDetector {
    fn pattern(&self) -> MatchedPattern {
        MatchedPattern::WithBuilder
    }
    fn derive_crate(&self) -> &'static str {
        "pleme-builder-derive"
    }
    fn derive_trait(&self) -> &'static str {
        "WithBuilder"
    }
    fn matches(&self, f: &ImplItemFn) -> bool {
        let name = f.sig.ident.to_string();
        let Some(field) = name.strip_prefix("with_") else {
            return false;
        };
        is_with_builder_shape(f, field)
    }
}

fn is_with_builder_shape(f: &ImplItemFn, field: &str) -> bool {
    if !matches!(
        f.sig.inputs.first(),
        Some(syn::FnArg::Receiver(r)) if r.reference.is_none() && r.mutability.is_some()
    ) {
        return false;
    }
    if f.sig.inputs.len() != 2 {
        return false;
    }
    let syn::ReturnType::Type(_, ret) = &f.sig.output else {
        return false;
    };
    let syn::Type::Path(tp) = ret.as_ref() else {
        return false;
    };
    if !tp.path.is_ident("Self") {
        return false;
    }
    let stmts = &f.block.stmts;
    if stmts.len() != 2 {
        return false;
    }
    if !matches_setter_assign(&stmts[0], field) {
        return false;
    }
    matches!(
        &stmts[1],
        syn::Stmt::Expr(syn::Expr::Path(p), _)
            if p.path.is_ident("self")
    )
}

// ─────────────────────────────────────────────────────────────────────
// IsVariant: `pub fn is_<variant>(&self) -> bool { matches!(self, …) }`
// ─────────────────────────────────────────────────────────────────────

pub struct IsVariantDetector;

impl Detector for IsVariantDetector {
    fn pattern(&self) -> MatchedPattern {
        MatchedPattern::IsVariant
    }
    fn derive_crate(&self) -> &'static str {
        "pleme-isvariant-derive"
    }
    fn derive_trait(&self) -> &'static str {
        "IsVariant"
    }
    fn matches(&self, f: &ImplItemFn) -> bool {
        let name = f.sig.ident.to_string();
        if !name.starts_with("is_") {
            return false;
        }
        is_isvariant_shape(f)
    }
}

fn is_isvariant_shape(f: &ImplItemFn) -> bool {
    if !matches!(
        f.sig.inputs.first(),
        Some(syn::FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none()
    ) {
        return false;
    }
    if f.sig.inputs.len() != 1 {
        return false;
    }
    let syn::ReturnType::Type(_, ret) = &f.sig.output else {
        return false;
    };
    let syn::Type::Path(tp) = ret.as_ref() else {
        return false;
    };
    if !tp.path.is_ident("bool") {
        return false;
    }
    let stmts = &f.block.stmts;
    if stmts.len() != 1 {
        return false;
    }
    let syn::Stmt::Expr(expr, _) = &stmts[0] else {
        return false;
    };
    let syn::Expr::Macro(m) = expr else {
        return false;
    };
    m.mac.path.is_ident("matches")
}

// ─────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────

fn matches_field_access(expr: &syn::Expr, field: &str) -> bool {
    let syn::Expr::Field(fe) = expr else {
        return false;
    };
    let syn::Expr::Path(p) = fe.base.as_ref() else {
        return false;
    };
    if !p.path.is_ident("self") {
        return false;
    }
    match &fe.member {
        syn::Member::Named(id) => id == field,
        syn::Member::Unnamed(_) => false,
    }
}

fn matches_setter_assign(stmt: &syn::Stmt, field: &str) -> bool {
    let expr = match stmt {
        syn::Stmt::Expr(e, _) => e,
        _ => return false,
    };
    let syn::Expr::Assign(a) = expr else {
        return false;
    };
    matches_field_access(&a.left, field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn registry_has_one_detector_per_pattern() {
        let dets = detectors();
        assert_eq!(dets.len(), 4, "registry has exactly the four canonical detectors");
        // No two detectors claim the same pattern.
        let mut seen: Vec<MatchedPattern> = vec![];
        for d in dets {
            assert!(
                !seen.contains(&d.pattern()),
                "pattern {:?} claimed by two detectors",
                d.pattern()
            );
            seen.push(d.pattern());
        }
    }

    #[test]
    fn each_detector_advertises_a_unique_derive_crate() {
        let dets = detectors();
        let mut crates: Vec<&str> = dets.iter().map(|d| d.derive_crate()).collect();
        crates.sort();
        crates.dedup();
        assert_eq!(crates.len(), 4, "every detector points at a distinct derive crate");
    }

    #[test]
    fn getter_detector_matches_canonical_shape() {
        let f: ImplItemFn = parse_quote! {
            pub fn host(&self) -> &String { &self.host }
        };
        assert!(GetterAllDetector.matches(&f));
        // A setter must NOT match the getter detector.
        let setter: ImplItemFn = parse_quote! {
            pub fn set_host(&mut self, v: String) { self.host = v; }
        };
        assert!(!GetterAllDetector.matches(&setter));
    }

    #[test]
    fn setter_detector_matches_canonical_shape() {
        let f: ImplItemFn = parse_quote! {
            pub fn set_port(&mut self, v: u16) { self.port = v; }
        };
        assert!(SetterAllDetector.matches(&f));
        // A getter must NOT match the setter detector.
        let getter: ImplItemFn = parse_quote! {
            pub fn port(&self) -> &u16 { &self.port }
        };
        assert!(!SetterAllDetector.matches(&getter));
    }

    #[test]
    fn with_builder_detector_matches_canonical_shape() {
        let f: ImplItemFn = parse_quote! {
            pub fn with_max(mut self, v: usize) -> Self { self.max = v; self }
        };
        assert!(WithBuilderDetector.matches(&f));
        // Wrong receiver (& mut instead of mut self) must NOT match.
        let bad: ImplItemFn = parse_quote! {
            pub fn with_max(&mut self, v: usize) -> Self { self.max = v; self.clone() }
        };
        assert!(!WithBuilderDetector.matches(&bad));
    }

    #[test]
    fn isvariant_detector_matches_canonical_shape() {
        let f: ImplItemFn = parse_quote! {
            pub fn is_idle(&self) -> bool { matches!(self, Self::Idle) }
        };
        assert!(IsVariantDetector.matches(&f));
        // Body is NOT a matches! macro → reject.
        let bad: ImplItemFn = parse_quote! {
            pub fn is_idle(&self) -> bool { true }
        };
        assert!(!IsVariantDetector.matches(&bad));
    }

    #[test]
    fn unrelated_fn_matches_no_detector() {
        let f: ImplItemFn = parse_quote! {
            pub fn frobnicate(&self) -> i32 { 42 }
        };
        for d in detectors() {
            assert!(!d.matches(&f), "detector {:?} mistakenly matched frobnicate", d.pattern());
        }
    }
}
