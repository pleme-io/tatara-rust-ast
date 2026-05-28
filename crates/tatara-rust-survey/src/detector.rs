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
        &AsMutAllDetector,
        &OwnedAllDetector,
        &ReplaceAllDetector,
        &TakeAllDetector,
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
// AsMutAll: `pub fn <field>_mut(&mut self) -> &mut <T> { &mut self.<field> }`
// ─────────────────────────────────────────────────────────────────────

pub struct AsMutAllDetector;

impl Detector for AsMutAllDetector {
    fn pattern(&self) -> MatchedPattern { MatchedPattern::AsMutAll }
    fn derive_crate(&self) -> &'static str { "pleme-asmut-derive" }
    fn derive_trait(&self) -> &'static str { "AsMutAll" }
    fn matches(&self, f: &ImplItemFn) -> bool {
        let name = f.sig.ident.to_string();
        let Some(field) = name.strip_suffix("_mut") else {
            return false;
        };
        is_asmut_shape(f, field)
    }
}

fn is_asmut_shape(f: &ImplItemFn, field: &str) -> bool {
    // Receiver: &mut self.
    if !matches!(
        f.sig.inputs.first(),
        Some(syn::FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_some()
    ) {
        return false;
    }
    if f.sig.inputs.len() != 1 {
        return false;
    }
    // Return: &mut <T>.
    let syn::ReturnType::Type(_, ret) = &f.sig.output else {
        return false;
    };
    let syn::Type::Reference(tr) = ret.as_ref() else {
        return false;
    };
    if tr.mutability.is_none() {
        return false;
    }
    // Body: single `&mut self.<field>` expression.
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
    if r.mutability.is_none() {
        return false;
    }
    matches_field_access(&r.expr, field)
}

// ─────────────────────────────────────────────────────────────────────
// OwnedAll: `pub fn into_<field>(self) -> <T> { self.<field> }`
// ─────────────────────────────────────────────────────────────────────

pub struct OwnedAllDetector;

impl Detector for OwnedAllDetector {
    fn pattern(&self) -> MatchedPattern { MatchedPattern::OwnedAll }
    fn derive_crate(&self) -> &'static str { "pleme-owned-derive" }
    fn derive_trait(&self) -> &'static str { "OwnedAll" }
    fn matches(&self, f: &ImplItemFn) -> bool {
        let name = f.sig.ident.to_string();
        let Some(field) = name.strip_prefix("into_") else {
            return false;
        };
        is_owned_shape(f, field)
    }
}

fn is_owned_shape(f: &ImplItemFn, field: &str) -> bool {
    // Receiver: `self` (no & no mut).
    if !matches!(
        f.sig.inputs.first(),
        Some(syn::FnArg::Receiver(r)) if r.reference.is_none() && r.mutability.is_none()
    ) {
        return false;
    }
    if f.sig.inputs.len() != 1 {
        return false;
    }
    // Return: `<T>` (NOT reference) — owned value.
    let syn::ReturnType::Type(_, ret) = &f.sig.output else {
        return false;
    };
    if matches!(ret.as_ref(), syn::Type::Reference(_)) {
        return false;
    }
    // Body: single `self.<field>` (no `&`).
    let stmts = &f.block.stmts;
    if stmts.len() != 1 {
        return false;
    }
    let syn::Stmt::Expr(expr, None) = &stmts[0] else {
        return false;
    };
    // Direct field access, NOT wrapped in Reference.
    matches!(expr, syn::Expr::Field(_)) && matches_field_access(expr, field)
}

// ─────────────────────────────────────────────────────────────────────
// ReplaceAll: `pub fn replace_<field>(&mut self, v: <T>) -> <T> {
//                 std::mem::replace(&mut self.<field>, v) }`
// ─────────────────────────────────────────────────────────────────────

pub struct ReplaceAllDetector;

impl Detector for ReplaceAllDetector {
    fn pattern(&self) -> MatchedPattern { MatchedPattern::ReplaceAll }
    fn derive_crate(&self) -> &'static str { "pleme-replace-derive" }
    fn derive_trait(&self) -> &'static str { "ReplaceAll" }
    fn matches(&self, f: &ImplItemFn) -> bool {
        let name = f.sig.ident.to_string();
        let Some(field) = name.strip_prefix("replace_") else {
            return false;
        };
        is_replace_shape(f, field)
    }
}

fn is_replace_shape(f: &ImplItemFn, field: &str) -> bool {
    // Receiver: &mut self.
    if !matches!(
        f.sig.inputs.first(),
        Some(syn::FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_some()
    ) {
        return false;
    }
    if f.sig.inputs.len() != 2 {
        return false;
    }
    // Body: single `std::mem::replace(...)` macro call.
    let stmts = &f.block.stmts;
    if stmts.len() != 1 {
        return false;
    }
    let syn::Stmt::Expr(expr, None) = &stmts[0] else {
        return false;
    };
    let syn::Expr::Call(call) = expr else {
        return false;
    };
    let syn::Expr::Path(p) = call.func.as_ref() else {
        return false;
    };
    // Accept `std::mem::replace`, `core::mem::replace`, `::std::mem::replace`, or bare `mem::replace`.
    if !path_ends_with(&p.path, &["mem", "replace"]) {
        return false;
    }
    // First arg is `&mut self.<field>`.
    let Some(syn::Expr::Reference(r)) = call.args.first() else {
        return false;
    };
    if r.mutability.is_none() {
        return false;
    }
    matches_field_access(&r.expr, field)
}

// ─────────────────────────────────────────────────────────────────────
// TakeAll: `pub fn take_<field>(&mut self) -> <T> { std::mem::take(&mut self.<field>) }`
// ─────────────────────────────────────────────────────────────────────

pub struct TakeAllDetector;

impl Detector for TakeAllDetector {
    fn pattern(&self) -> MatchedPattern { MatchedPattern::TakeAll }
    fn derive_crate(&self) -> &'static str { "pleme-take-derive" }
    fn derive_trait(&self) -> &'static str { "TakeAll" }
    fn matches(&self, f: &ImplItemFn) -> bool {
        let name = f.sig.ident.to_string();
        let Some(field) = name.strip_prefix("take_") else {
            return false;
        };
        is_take_shape(f, field)
    }
}

fn is_take_shape(f: &ImplItemFn, field: &str) -> bool {
    // Receiver: &mut self.
    if !matches!(
        f.sig.inputs.first(),
        Some(syn::FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_some()
    ) {
        return false;
    }
    if f.sig.inputs.len() != 1 {
        return false;
    }
    let stmts = &f.block.stmts;
    if stmts.len() != 1 {
        return false;
    }
    let syn::Stmt::Expr(expr, None) = &stmts[0] else {
        return false;
    };
    let syn::Expr::Call(call) = expr else {
        return false;
    };
    let syn::Expr::Path(p) = call.func.as_ref() else {
        return false;
    };
    if !path_ends_with(&p.path, &["mem", "take"]) {
        return false;
    }
    let Some(syn::Expr::Reference(r)) = call.args.first() else {
        return false;
    };
    if r.mutability.is_none() {
        return false;
    }
    matches_field_access(&r.expr, field)
}

/// Path tail-match helper. `std::mem::replace`, `core::mem::replace`,
/// `::core::mem::replace`, and `mem::replace` all return true for
/// `&["mem", "replace"]`. Lets the detectors accept the operator's
/// preferred std-path prefix without enumerating them.
fn path_ends_with(p: &syn::Path, tail: &[&str]) -> bool {
    if p.segments.len() < tail.len() {
        return false;
    }
    let offset = p.segments.len() - tail.len();
    p.segments
        .iter()
        .skip(offset)
        .zip(tail.iter())
        .all(|(seg, want)| seg.ident == *want)
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
        assert_eq!(dets.len(), 8, "registry has the eight typed detectors");
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
        assert_eq!(crates.len(), 8, "every detector points at a distinct derive crate");
    }

    #[test]
    fn asmut_detector_matches_canonical_shape() {
        let f: ImplItemFn = parse_quote! {
            pub fn host_mut(&mut self) -> &mut String { &mut self.host }
        };
        assert!(AsMutAllDetector.matches(&f));
        // Getter (returns shared ref) must NOT match.
        let getter: ImplItemFn = parse_quote! {
            pub fn host(&self) -> &String { &self.host }
        };
        assert!(!AsMutAllDetector.matches(&getter));
    }

    #[test]
    fn owned_detector_matches_consuming_getter() {
        let f: ImplItemFn = parse_quote! {
            pub fn into_host(self) -> String { self.host }
        };
        assert!(OwnedAllDetector.matches(&f));
        // GetterAll returns &, OwnedAll consumes self → reject getter.
        let getter: ImplItemFn = parse_quote! {
            pub fn into_host(self) -> String { self.host.clone() }
        };
        assert!(!OwnedAllDetector.matches(&getter), "extra call → reject");
    }

    #[test]
    fn replace_detector_accepts_std_and_core_paths() {
        for body in [
            "{ std::mem::replace(&mut self.host, v) }",
            "{ ::std::mem::replace(&mut self.host, v) }",
            "{ core::mem::replace(&mut self.host, v) }",
            "{ mem::replace(&mut self.host, v) }",
        ] {
            let src = format!("pub fn replace_host(&mut self, v: String) -> String {body}");
            let f: ImplItemFn = syn::parse_str(&src).unwrap();
            assert!(
                ReplaceAllDetector.matches(&f),
                "must match canonical mem::replace shape: {body}"
            );
        }
        // Wrong fn body must NOT match.
        let bad: ImplItemFn = parse_quote! {
            pub fn replace_host(&mut self, v: String) -> String { self.host = v; v }
        };
        assert!(!ReplaceAllDetector.matches(&bad));
    }

    #[test]
    fn take_detector_matches_std_mem_take() {
        let f: ImplItemFn = parse_quote! {
            pub fn take_host(&mut self) -> String { std::mem::take(&mut self.host) }
        };
        assert!(TakeAllDetector.matches(&f));
        // Not a take — direct return instead of mem::take.
        let bad: ImplItemFn = parse_quote! {
            pub fn take_host(&mut self) -> String { self.host.clone() }
        };
        assert!(!TakeAllDetector.matches(&bad));
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
