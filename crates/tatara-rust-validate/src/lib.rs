//! `tatara-rust-validate` — typed correctness gate for every macro Spec.
//!
//! One trait, one enum:
//! - [`Validate`] — every Spec kind implements `validate(&self) -> Vec<Violation>`.
//!   Empty vec ⇒ structurally well-formed.
//! - [`Violation`] — typed enum of every way a Spec can be malformed.
//!
//! Each violation is named, deterministic, and ASCII-only renderable so
//! it round-trips through JSON for cross-process error reporting (e.g.
//! the forge CLI exits non-zero with a serialized Vec<Violation>).
//!
//! Adding a new check = add a `Violation` variant + bump the matching
//! per-Spec `validate` impl. Existing checks compose by Vec-append.

use serde::{Deserialize, Serialize};
use tatara_rust_composite::{CompositeDeriveSpec, CompositeMember};
use tatara_rust_derive::{
    EnumFoldDeriveSpec, KindRoundTripSpec, NewtypeDeriveSpec, PerFieldDeriveSpec,
    PerVariantDeriveSpec, ProcDeriveSpec, VerificationMatrixSpec,
};
use tatara_rust_macro_rules::MacroRulesSpec;
use tatara_rust_proc_attr::ProcAttrSpec;
use tatara_rust_proc_fn::ProcFnSpec;

/// Every way a Spec can fail to be structurally well-formed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rule", rename_all = "kebab-case")]
pub enum Violation {
    /// Empty identifier where one was required.
    EmptyIdent { field: String },

    /// Identifier doesn't start with an ASCII alpha or `_`.
    InvalidIdentStart { field: String, ident: String },

    /// Identifier contains chars rustc would reject (whitespace, hyphens, etc.).
    InvalidIdentChars { field: String, ident: String },

    /// Per-field/variant template contains no splice holes — almost
    /// certainly a copy-paste mistake (the consumer struct's fields
    /// won't appear in the output).
    TemplateMissingSpliceHoles { spec_name: String, template: String },

    /// `method_name_template` is `Some("…")` but `{}` is missing —
    /// `format_ident!` would emit a non-templated identifier.
    MethodTemplateMissingBrace { spec_name: String, template: String },

    /// A `CompositeDeriveSpec` has no inner members.
    CompositeWithNoMembers { bundle_name: String },

    /// Two inner members of one Composite share the same inner trait_name
    /// (would emit two impls that conflict at consumer expand time).
    CompositeDuplicateMember {
        bundle_name: String,
        duplicate_trait_name: String,
    },

    /// `MacroRulesSpec` declares no arms — the emitted macro_rules can never match.
    MacroRulesEmpty { macro_name: String },

    /// `PrependPrelude { prelude_tokens: "" }` is a no-op transform.
    EmptyPreludeTokens { spec_name: String },
}

pub trait Validate {
    fn validate(&self) -> Vec<Violation>;
}

// ─────────────────────────────────────────────────────────────────────
// Per-Spec implementations
// ─────────────────────────────────────────────────────────────────────

impl Validate for ProcDeriveSpec {
    fn validate(&self) -> Vec<Violation> {
        let mut v = vec![];
        check_ident("ProcDeriveSpec.trait_name", &self.trait_name.0, &mut v);
        v
    }
}

impl Validate for PerFieldDeriveSpec {
    fn validate(&self) -> Vec<Violation> {
        let mut v = vec![];
        check_ident("PerFieldDeriveSpec.trait_name", &self.trait_name.0, &mut v);
        check_template_splices(
            "PerFieldDeriveSpec",
            &self.trait_name.0,
            &self.per_field_template,
            &["#field_name", "#field_ty", "#method_ident", "#self_name"],
            &mut v,
        );
        check_method_template(
            &self.trait_name.0,
            self.method_name_template.as_deref(),
            &mut v,
        );
        v
    }
}

impl Validate for PerVariantDeriveSpec {
    fn validate(&self) -> Vec<Violation> {
        let mut v = vec![];
        check_ident("PerVariantDeriveSpec.trait_name", &self.trait_name.0, &mut v);
        check_template_splices(
            "PerVariantDeriveSpec",
            &self.trait_name.0,
            &self.per_variant_template,
            &[
                "#variant_name",
                "#variant_shape_arm",
                "#method_ident",
                "#self_name",
            ],
            &mut v,
        );
        check_method_template(
            &self.trait_name.0,
            self.method_name_template.as_deref(),
            &mut v,
        );
        v
    }
}

impl Validate for NewtypeDeriveSpec {
    fn validate(&self) -> Vec<Violation> {
        let mut v = vec![];
        check_ident("NewtypeDeriveSpec.trait_name", &self.trait_name.0, &mut v);
        check_template_splices(
            "NewtypeDeriveSpec",
            &self.trait_name.0,
            &self.impl_template,
            // Either splice hole is enough — some derives (e.g. an inherent
            // helper) might only reference #self_name; others (From) need both.
            &["#self_name", "#inner_ty"],
            &mut v,
        );
        v
    }
}

impl Validate for EnumFoldDeriveSpec {
    fn validate(&self) -> Vec<Violation> {
        let mut v = vec![];
        check_ident("EnumFoldDeriveSpec.trait_name", &self.trait_name.0, &mut v);
        // The fragment may legitimately not reference any splice hole
        // (e.g. `"1"` for a variant-count fold where only #fold_count
        // is used at the template level) — so don't check fragment
        // splice holes. The contract that matters is the template.
        check_template_splices(
            "EnumFoldDeriveSpec.fold_template",
            &self.trait_name.0,
            &self.fold_template,
            &["#fold", "#fold_count", "#self_name"],
            &mut v,
        );
        v
    }
}

impl Validate for KindRoundTripSpec {
    fn validate(&self) -> Vec<Violation> {
        // Fixed-template spec (no splice holes) — the contract that
        // matters is that every parameterized identifier is a legal Rust
        // ident, since each is substituted directly into the emitted
        // proc-macro source.
        let mut v = vec![];
        check_ident("KindRoundTripSpec.trait_name", &self.trait_name.0, &mut v);
        check_ident("KindRoundTripSpec.helper_attr", &self.helper_attr, &mut v);
        check_ident("KindRoundTripSpec.as_str_method", &self.as_str_method, &mut v);
        check_ident("KindRoundTripSpec.from_str_method", &self.from_str_method, &mut v);
        if self.with_byte {
            check_ident("KindRoundTripSpec.as_byte_method", &self.as_byte_method, &mut v);
            check_ident("KindRoundTripSpec.from_byte_method", &self.from_byte_method, &mut v);
        }
        v
    }
}

impl Validate for VerificationMatrixSpec {
    fn validate(&self) -> Vec<Violation> {
        // Both emitted macro identifiers are substituted directly into
        // `macro_rules! <ident>` heads, so each must be a legal Rust ident.
        let mut v = vec![];
        check_ident("VerificationMatrixSpec.matrix_macro", &self.matrix_macro, &mut v);
        check_ident("VerificationMatrixSpec.covers_macro", &self.covers_macro, &mut v);
        v
    }
}

impl Validate for ProcAttrSpec {
    fn validate(&self) -> Vec<Violation> {
        let mut v = vec![];
        check_ident("ProcAttrSpec.macro_name", &self.macro_name.0, &mut v);
        let tatara_rust_proc_attr::AttrTransform::PrependPrelude { prelude_tokens } =
            &self.transform;
        if prelude_tokens.is_empty() {
            v.push(Violation::EmptyPreludeTokens {
                spec_name: self.macro_name.0.clone(),
            });
        }
        v
    }
}

impl Validate for ProcFnSpec {
    fn validate(&self) -> Vec<Violation> {
        let mut v = vec![];
        check_ident("ProcFnSpec.macro_name", &self.macro_name.0, &mut v);
        let tatara_rust_proc_fn::FnTransform::PrependPrelude { prelude_tokens } = &self.transform;
        if prelude_tokens.is_empty() {
            v.push(Violation::EmptyPreludeTokens {
                spec_name: self.macro_name.0.clone(),
            });
        }
        v
    }
}

impl Validate for MacroRulesSpec {
    fn validate(&self) -> Vec<Violation> {
        let mut v = vec![];
        check_ident("MacroRulesSpec.macro_name", &self.macro_name.0, &mut v);
        if self.arms.is_empty() {
            v.push(Violation::MacroRulesEmpty {
                macro_name: self.macro_name.0.clone(),
            });
        }
        v
    }
}

impl Validate for CompositeDeriveSpec {
    fn validate(&self) -> Vec<Violation> {
        let mut v = vec![];
        check_ident("CompositeDeriveSpec.bundle_name", &self.bundle_name.0, &mut v);
        if self.members.is_empty() {
            v.push(Violation::CompositeWithNoMembers {
                bundle_name: self.bundle_name.0.clone(),
            });
        }
        // Inner-member validate is recursive — bubble up everything.
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for m in &self.members {
            let (inner_name, inner_violations) = match m {
                CompositeMember::Simple(s) => (s.trait_name.0.clone(), s.validate()),
                CompositeMember::PerField(s) => (s.trait_name.0.clone(), s.validate()),
                CompositeMember::PerVariant(s) => (s.trait_name.0.clone(), s.validate()),
            };
            if !seen.insert(inner_name.clone()) {
                v.push(Violation::CompositeDuplicateMember {
                    bundle_name: self.bundle_name.0.clone(),
                    duplicate_trait_name: inner_name,
                });
            }
            v.extend(inner_violations);
        }
        v
    }
}

// ─────────────────────────────────────────────────────────────────────
// Internal check primitives
// ─────────────────────────────────────────────────────────────────────

fn check_ident(field: &str, ident: &str, out: &mut Vec<Violation>) {
    if ident.is_empty() {
        out.push(Violation::EmptyIdent {
            field: field.into(),
        });
        return;
    }
    let first = ident.chars().next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        out.push(Violation::InvalidIdentStart {
            field: field.into(),
            ident: ident.into(),
        });
    }
    if !ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        out.push(Violation::InvalidIdentChars {
            field: field.into(),
            ident: ident.into(),
        });
    }
}

fn check_template_splices(
    kind: &str,
    spec_name: &str,
    template: &str,
    candidates: &[&str],
    out: &mut Vec<Violation>,
) {
    let _ = kind;
    if !candidates.iter().any(|h| template.contains(h)) {
        out.push(Violation::TemplateMissingSpliceHoles {
            spec_name: spec_name.into(),
            template: template.into(),
        });
    }
}

fn check_method_template(spec_name: &str, tpl: Option<&str>, out: &mut Vec<Violation>) {
    let Some(t) = tpl else { return };
    if !t.contains("{}") {
        out.push(Violation::MethodTemplateMissingBrace {
            spec_name: spec_name.into(),
            template: t.into(),
        });
    }
}

/// Convenience: `Ok(())` when no violations, else a `Vec<Violation>` Err.
/// Forge uses this to exit non-zero on malformed input.
pub fn assert_valid<T: Validate>(spec: &T) -> Result<(), Vec<Violation>> {
    let v = spec.validate();
    if v.is_empty() { Ok(()) } else { Err(v) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_ast::Ident;
    use tatara_rust_derive::PerFieldTarget;

    fn good_per_field() -> PerFieldDeriveSpec {
        PerFieldDeriveSpec {
            trait_name: Ident::new("Good"),
            target: PerFieldTarget::NamedStruct,
            trait_ref: None,
            per_field_template:
                "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
            method_name_template: None,
            impl_prelude: None,
            skip_fields: vec![],
            field_attribute: None,
        }
    }

    #[test]
    fn good_per_field_validates_clean() {
        assert!(good_per_field().validate().is_empty());
    }

    #[test]
    fn empty_trait_name_caught() {
        let mut s = good_per_field();
        s.trait_name = Ident::new("");
        let v = s.validate();
        assert!(matches!(v.first(), Some(Violation::EmptyIdent { .. })));
    }

    #[test]
    fn invalid_trait_name_start_caught() {
        let mut s = good_per_field();
        s.trait_name = Ident::new("1Bad");
        let v = s.validate();
        assert!(matches!(
            v.first(),
            Some(Violation::InvalidIdentStart { .. })
        ));
    }

    #[test]
    fn invalid_trait_name_chars_caught() {
        let mut s = good_per_field();
        s.trait_name = Ident::new("Bad-Name");
        let v = s.validate();
        assert!(matches!(
            v.first(),
            Some(Violation::InvalidIdentChars { .. })
        ));
    }

    #[test]
    fn template_missing_splice_holes_caught() {
        let mut s = good_per_field();
        s.per_field_template = "pub fn x() {}".into();
        let v = s.validate();
        assert!(matches!(
            v.first(),
            Some(Violation::TemplateMissingSpliceHoles { .. })
        ));
    }

    #[test]
    fn method_template_missing_brace_caught() {
        let mut s = good_per_field();
        s.method_name_template = Some("with_no_brace".into());
        let v = s.validate();
        assert!(matches!(
            v.iter().find(|x| matches!(x, Violation::MethodTemplateMissingBrace { .. })),
            Some(_),
        ));
    }

    #[test]
    fn composite_no_members_caught() {
        let s = CompositeDeriveSpec {
            bundle_name: Ident::new("Empty"),
            members: vec![],
        };
        let v = s.validate();
        assert!(matches!(
            v.first(),
            Some(Violation::CompositeWithNoMembers { .. })
        ));
    }

    #[test]
    fn composite_duplicate_members_caught() {
        let inner = good_per_field();
        let mut other = good_per_field();
        other.trait_name = inner.trait_name.clone(); // same name
        let s = CompositeDeriveSpec {
            bundle_name: Ident::new("Dup"),
            members: vec![
                CompositeMember::PerField(inner),
                CompositeMember::PerField(other),
            ],
        };
        let v = s.validate();
        assert!(v.iter().any(|x| matches!(
            x,
            Violation::CompositeDuplicateMember { .. }
        )));
    }

    #[test]
    fn macro_rules_empty_caught() {
        let s = MacroRulesSpec {
            macro_name: Ident::new("noarm"),
            arms: vec![],
        };
        let v = s.validate();
        assert!(matches!(v.first(), Some(Violation::MacroRulesEmpty { .. })));
    }

    #[test]
    fn assert_valid_round_trip() {
        assert!(assert_valid(&good_per_field()).is_ok());
        let mut bad = good_per_field();
        bad.trait_name = Ident::new("");
        let err = assert_valid(&bad).unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn violations_serde_round_trip() {
        let v = Violation::EmptyIdent {
            field: "x".into(),
        };
        let j = serde_json::to_string(&v).unwrap();
        let back: Violation = serde_json::from_str(&j).unwrap();
        assert_eq!(v, back);
    }
}
