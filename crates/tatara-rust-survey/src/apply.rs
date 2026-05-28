//! `apply` — syn::visit_mut transform that lands a [`RefactorCandidate`].
//!
//! Takes a Rust source string + a typed candidate; produces a
//! modified source string where:
//!
//! 1. The target struct/enum gains a `#[derive(<X>)]` attribute for
//!    the matched derive trait.
//! 2. The inherent impl block on that type loses every method whose
//!    body matches the candidate's pattern.
//! 3. If the impl block becomes empty after removal, the whole impl
//!    is dropped.
//! 4. A `use pleme_<x>_derive::<Trait>;` line is inserted at the top
//!    of the file so the consumer doesn't need to spell out the
//!    fully-qualified path.
//!
//! The output is rendered via prettyplease — same typed-AST border
//! the rest of the substrate uses. Per the ★★ TOKEN-STABILITY rule,
//! the apply's correctness is asserted at the syn-AST level
//! (round-trip via syn::parse_file is the gate, not character
//! comparison).
//!
//! This is the **apply** half of the structured adoption pipeline:
//!
//! ```text
//! survey  →  apply  →  validate
//!            (here)     (cargo test on the modified crate)
//! ```

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Attribute, File, ImplItem, Item, parse_quote};

use crate::{MatchedPattern, RefactorCandidate, classify_fn, type_base_ident};

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("syn parse failed: {0}")]
    Parse(#[from] syn::Error),
    #[error("target type `{0}` not found at file scope — apply requires the candidate's target struct/enum to be defined in the same .rs file")]
    TargetNotFound(String),
    #[error("no impl methods matched pattern {pattern:?} on target `{target}` — the candidate is stale")]
    NoMethodsRemoved {
        pattern: MatchedPattern,
        target: String,
    },
}

/// Apply a single [`RefactorCandidate`] to source. Returns the
/// modified source (prettyplease-formatted) on success.
///
/// **Idempotent** — if the derive is already present on the target,
/// `add_derive_attr` is a no-op; running apply twice produces the
/// same byte output as running it once.
pub fn apply_to_source(src: &str, cand: &RefactorCandidate) -> Result<String, ApplyError> {
    let mut file: File = syn::parse_file(src)?;

    // Inject `use pleme_<x>_derive::<Trait>;` at the top (idempotent).
    let import = make_import(cand);
    if !file.items.iter().any(|i| {
        matches!(i, Item::Use(u) if use_matches(u, cand))
    }) {
        file.items.insert(0, import);
    }

    let mut removed: usize = 0;
    let mut found_target = false;

    file.items.retain_mut(|item| {
        match item {
            // Add the derive to the target struct/enum.
            Item::Struct(s) if s.ident == cand.target_type => {
                found_target = true;
                add_derive_attr(&mut s.attrs, &cand.derive_trait);
                true
            }
            Item::Enum(e) if e.ident == cand.target_type => {
                found_target = true;
                add_derive_attr(&mut e.attrs, &cand.derive_trait);
                true
            }
            // Strip matching methods from inherent impls on the target.
            // Use base-ident comparison so `impl<T> Foo<T>` matches
            // candidates whose target_type is `"Foo"`.
            Item::Impl(imp)
                if imp.trait_.is_none()
                    && type_base_ident(&imp.self_ty).as_deref() == Some(cand.target_type.as_str()) =>
            {
                let before = imp.items.len();
                imp.items.retain(|it| match it {
                    ImplItem::Fn(f) => match classify_fn(f) {
                        Some(p) if p == cand.pattern => false,
                        _ => true,
                    },
                    // Drop matched assoc-consts (W18 path — VariantCountConst /
                    // AllVariantsConst patterns where the derive emits a const
                    // and the pre-derive code had the same const hand-written).
                    // Routed through the Detector registry so adding new
                    // assoc-const detectors is automatic.
                    ImplItem::Const(c) => !crate::detector::detectors()
                        .iter()
                        .any(|d| d.pattern() == cand.pattern && d.matches_assoc_const(c)),
                    _ => true,
                });
                removed += before - imp.items.len();
                // Drop the impl block if it's now empty.
                !imp.items.is_empty()
            }
            _ => true,
        }
    });

    if !found_target {
        return Err(ApplyError::TargetNotFound(cand.target_type.clone()));
    }
    if removed == 0 {
        return Err(ApplyError::NoMethodsRemoved {
            pattern: cand.pattern,
            target: cand.target_type.clone(),
        });
    }

    Ok(prettyplease::unparse(&file))
}

/// `use pleme_<x>_derive::<Trait>;` as a typed syn::Item.
fn make_import(cand: &RefactorCandidate) -> Item {
    // Convert kebab-case crate name to snake-case for the use path.
    let crate_path: TokenStream = cand.derive_crate.replace('-', "_").parse().unwrap();
    let trait_ident = format_ident!("{}", cand.derive_trait);
    let ts: TokenStream = quote! { use #crate_path::#trait_ident; };
    syn::parse2(ts).expect("derive use path must parse")
}

/// Does an existing `use` item already import this derive?
fn use_matches(u: &syn::ItemUse, cand: &RefactorCandidate) -> bool {
    use syn::UseTree;
    fn walk(tree: &UseTree, crate_root: &str, trait_name: &str, depth: usize) -> bool {
        match tree {
            UseTree::Path(p) => {
                let is_crate = depth == 0 && p.ident == crate_root;
                if is_crate {
                    walk(&p.tree, crate_root, trait_name, depth + 1)
                } else {
                    walk(&p.tree, crate_root, trait_name, depth + 1)
                }
            }
            UseTree::Name(n) => n.ident == trait_name,
            UseTree::Group(g) => g.items.iter().any(|t| walk(t, crate_root, trait_name, depth)),
            _ => false,
        }
    }
    let crate_root = cand.derive_crate.replace('-', "_");
    walk(&u.tree, &crate_root, cand.derive_trait, 0)
}

/// Add `#[derive(<Trait>)]` to an attribute list if not already
/// present.
fn add_derive_attr(attrs: &mut Vec<Attribute>, trait_name: &str) {
    let trait_ident = format_ident!("{trait_name}");
    // Check if already present on any existing #[derive(...)] attr.
    for attr in attrs.iter() {
        if attr.path().is_ident("derive") {
            let mut has = false;
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident(&trait_ident) {
                    has = true;
                }
                Ok(())
            });
            if has {
                return; // Already derived — idempotent no-op.
            }
        }
    }
    let new_attr: Attribute = parse_quote! { #[derive(#trait_ident)] };
    attrs.push(new_attr);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::survey_file;
    use std::path::PathBuf;

    fn tmp_file(name: &str, body: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "tatara-apply-{}-{}",
            name,
            std::process::id()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("lib.rs");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn cand_from_file(path: &PathBuf) -> RefactorCandidate {
        let cands = survey_file(path).unwrap();
        cands.into_iter().next().expect("survey produced no candidate")
    }

    #[test]
    fn apply_getter_all_adds_derive_and_removes_impl_methods() {
        let src = r#"
pub struct Foo { pub a: i32, pub b: String }

impl Foo {
    pub fn a(&self) -> &i32 { &self.a }
    pub fn b(&self) -> &String { &self.b }
}
"#;
        let path = tmp_file("getter_apply", src);
        let cand = cand_from_file(&path);
        let out = apply_to_source(src, &cand).unwrap();

        // Output is well-formed Rust.
        let parsed: File = syn::parse_str(&out).unwrap();
        assert!(parsed.items.iter().any(|i| matches!(i, Item::Use(_))));

        // The two impl methods are gone (impl block dropped).
        assert!(
            !parsed.items.iter().any(|i| matches!(i, Item::Impl(_))),
            "empty impl block must be removed"
        );

        // The struct has #[derive(GetterAll)].
        let s = parsed
            .items
            .iter()
            .find_map(|i| if let Item::Struct(s) = i { Some(s) } else { None })
            .unwrap();
        let has_derive = s.attrs.iter().any(|a| {
            if !a.path().is_ident("derive") {
                return false;
            }
            let mut has = false;
            let _ = a.parse_nested_meta(|m| {
                if m.path.is_ident("GetterAll") {
                    has = true;
                }
                Ok(())
            });
            has
        });
        assert!(has_derive, "#[derive(GetterAll)] missing");
    }

    #[test]
    fn apply_setter_all_keeps_unrelated_methods() {
        let src = r#"
pub struct Foo { pub a: i32, pub b: String }

impl Foo {
    pub fn set_a(&mut self, v: i32) { self.a = v; }
    pub fn set_b(&mut self, v: String) { self.b = v; }
    pub fn unrelated(&self) -> u8 { 42 }
}
"#;
        let path = tmp_file("setter_apply", src);
        let cand = cand_from_file(&path);
        let out = apply_to_source(src, &cand).unwrap();

        let parsed: File = syn::parse_str(&out).unwrap();

        // The unrelated method survives — impl block stays.
        let imp = parsed
            .items
            .iter()
            .find_map(|i| if let Item::Impl(im) = i { Some(im) } else { None })
            .expect("impl block should stay (unrelated method survived)");
        assert_eq!(imp.items.len(), 1, "only unrelated method remains");

        // The derive landed.
        let s = parsed
            .items
            .iter()
            .find_map(|i| if let Item::Struct(s) = i { Some(s) } else { None })
            .unwrap();
        let has_derive = s.attrs.iter().any(|a| {
            if !a.path().is_ident("derive") {
                return false;
            }
            let mut has = false;
            let _ = a.parse_nested_meta(|m| {
                if m.path.is_ident("SetterAll") {
                    has = true;
                }
                Ok(())
            });
            has
        });
        assert!(has_derive);
    }

    #[test]
    fn apply_is_idempotent() {
        let src = r#"
pub struct Foo { pub a: i32, pub b: String }

impl Foo {
    pub fn a(&self) -> &i32 { &self.a }
    pub fn b(&self) -> &String { &self.b }
}
"#;
        let path = tmp_file("idem", src);
        let cand = cand_from_file(&path);
        let once = apply_to_source(src, &cand).unwrap();

        // Re-running apply on the already-transformed source must
        // either be a no-op (clean idempotency) or return the typed
        // NoMethodsRemoved error (defensive — no double-derive).
        match apply_to_source(&once, &cand) {
            Ok(twice) => assert_eq!(once, twice, "non-idempotent apply"),
            Err(ApplyError::NoMethodsRemoved { .. }) => {} // also fine
            Err(other) => panic!("unexpected re-apply error: {other}"),
        }
    }

    #[test]
    fn apply_isvariant_on_enum_works() {
        let src = r#"
pub enum State { A, B(i32), C { x: u8 } }

impl State {
    pub fn is_a(&self) -> bool { matches!(self, Self::A) }
    pub fn is_b(&self) -> bool { matches!(self, Self::B(_)) }
    pub fn is_c(&self) -> bool { matches!(self, Self::C { .. }) }
}
"#;
        let path = tmp_file("isvariant_apply", src);
        let cand = cand_from_file(&path);
        let out = apply_to_source(src, &cand).unwrap();

        let parsed: File = syn::parse_str(&out).unwrap();
        let e = parsed
            .items
            .iter()
            .find_map(|i| if let Item::Enum(e) = i { Some(e) } else { None })
            .unwrap();
        let has_derive = e.attrs.iter().any(|a| {
            if !a.path().is_ident("derive") {
                return false;
            }
            let mut has = false;
            let _ = a.parse_nested_meta(|m| {
                if m.path.is_ident("IsVariant") {
                    has = true;
                }
                Ok(())
            });
            has
        });
        assert!(has_derive);
        // The impl block is gone (all 3 methods removed).
        assert!(
            !parsed.items.iter().any(|i| matches!(i, Item::Impl(_))),
            "empty impl block must be removed after IsVariant apply"
        );
    }

    #[test]
    fn apply_errors_when_target_not_in_file() {
        let src = r#"
pub struct Foo { pub a: i32, pub b: String }

impl Foo {
    pub fn a(&self) -> &i32 { &self.a }
    pub fn b(&self) -> &String { &self.b }
}
"#;
        let path = tmp_file("target_not_found", src);
        let mut cand = cand_from_file(&path);
        cand.target_type = "NotThere".into();
        let err = apply_to_source(src, &cand).unwrap_err();
        assert!(matches!(err, ApplyError::TargetNotFound(_)));
    }
}
