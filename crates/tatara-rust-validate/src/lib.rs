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
    ClosedAxisSpec, EnumFoldDeriveSpec, KindRoundTripSpec, NewtypeDeriveSpec, PerFieldDeriveSpec,
    PerVariantDeriveSpec, ProcDeriveSpec, TagSpec, VerificationMatrixSpec,
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

    /// `ClosedAxisSpec.axis_trait_path` is empty — the emitted
    /// `impl <path> for Self` would be `impl  for Self` and fail to parse.
    EmptyTraitPath { spec_name: String },

    /// `PerFieldDeriveSpec.field_tag` is `Some` but `tags` is empty —
    /// nothing could ever match, so every field would hit the
    /// zero-tags-matched path unconditionally.
    FieldTagEmpty { spec_name: String },

    /// Two entries in `field_tag.tags` share the same `name` — the
    /// generated derive's `attributes(...)` list would register the
    /// same helper attribute twice and the dispatch `if` chain would
    /// never reach the second one.
    FieldTagDuplicateName { spec_name: String, duplicate_name: String },

    /// A `field_tag.tags[]` entry's own `per_field_template` contains no
    /// splice holes — same mistake `TemplateMissingSpliceHoles` catches
    /// for the single-template case, generalized per-tag.
    FieldTagTemplateMissingSpliceHoles {
        spec_name: String,
        tag_name: String,
        template: String,
    },
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
        // `field_tag` mode takes over per-field rendering entirely (see
        // its own doc comment) — `per_field_template` is unused in that
        // mode, so checking IT for splice holes would be checking dead
        // text. Check each tag's OWN template instead.
        if self.field_tag.is_none() {
            check_template_splices(
                "PerFieldDeriveSpec",
                &self.trait_name.0,
                &self.per_field_template,
                &["#field_name", "#field_ty", "#method_ident", "#self_name"],
                &mut v,
            );
        }
        check_method_template(
            &self.trait_name.0,
            self.method_name_template.as_deref(),
            &mut v,
        );
        if let Some(tag_spec) = &self.field_tag {
            check_field_tag(&self.trait_name.0, tag_spec, &mut v);
        }
        v
    }
}

fn check_field_tag(spec_name: &str, tag_spec: &TagSpec, out: &mut Vec<Violation>) {
    if tag_spec.tags.is_empty() {
        out.push(Violation::FieldTagEmpty {
            spec_name: spec_name.into(),
        });
        return;
    }
    let mut seen = std::collections::HashSet::new();
    for tag in &tag_spec.tags {
        if !seen.insert(tag.name.as_str()) {
            out.push(Violation::FieldTagDuplicateName {
                spec_name: spec_name.into(),
                duplicate_name: tag.name.clone(),
            });
        }
        let mut candidates: Vec<&str> = vec!["#field_name", "#field_ty", "#method_ident", "#self_name"];
        let arg_holes: Vec<String> = tag.required_args.iter().map(|a| format!("#{a}")).collect();
        candidates.extend(arg_holes.iter().map(String::as_str));
        if !candidates.iter().any(|h| tag.per_field_template.contains(h)) {
            out.push(Violation::FieldTagTemplateMissingSpliceHoles {
                spec_name: spec_name.into(),
                tag_name: tag.name.clone(),
                template: tag.per_field_template.clone(),
            });
        }
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

impl Validate for ClosedAxisSpec {
    fn validate(&self) -> Vec<Violation> {
        // `trait_name` is the `#[derive(<id>)]` identifier — a legal Rust
        // ident. `axis_trait_path` is a fully-qualified path (`::a::B`),
        // substituted into `impl <path> for Self`, so it just has to be
        // non-empty; rustc validates the path at consumer compile time.
        let mut v = vec![];
        check_ident("ClosedAxisSpec.trait_name", &self.trait_name.0, &mut v);
        if self.axis_trait_path.trim().is_empty() {
            v.push(Violation::EmptyTraitPath {
                spec_name: self.trait_name.0.clone(),
            });
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
            field_tag: None,
        }
    }

    fn good_field_tag_spec() -> PerFieldDeriveSpec {
        PerFieldDeriveSpec {
            trait_name: Ident::new("HotSwap"),
            target: PerFieldTarget::NamedStruct,
            trait_ref: None,
            per_field_template: String::new(),
            method_name_template: None,
            impl_prelude: None,
            skip_fields: vec![],
            field_attribute: None,
            field_tag: Some(TagSpec {
                exhaustive: true,
                aggregate: None,
                tags: vec![
                    tatara_rust_derive::FieldTag {
                        name: "hot_swap".into(),
                        required_args: vec![],
                        per_field_template: "(#field_name, HotSwapClass::Free)".into(),
                        aggregate_const_entry: None,
                        aggregate_stmt: None,
                    },
                    tatara_rust_derive::FieldTag {
                        name: "restart_required".into(),
                        required_args: vec!["reason".into()],
                        per_field_template:
                            "(#field_name, HotSwapClass::RequiresRestart { reason: #reason })".into(),
                        aggregate_const_entry: None,
                        aggregate_stmt: None,
                    },
                ],
            }),
        }
    }

    #[test]
    fn good_field_tag_spec_validates_clean() {
        assert!(good_field_tag_spec().validate().is_empty());
    }

    #[test]
    fn field_tag_empty_tags_caught() {
        let mut s = good_field_tag_spec();
        s.field_tag = Some(TagSpec {
            tags: vec![],
            exhaustive: true,
            aggregate: None,
        });
        let v = s.validate();
        assert!(matches!(v.first(), Some(Violation::FieldTagEmpty { .. })));
    }

    #[test]
    fn field_tag_duplicate_name_caught() {
        let mut s = good_field_tag_spec();
        if let Some(ts) = s.field_tag.as_mut() {
            let first = ts.tags[0].clone();
            ts.tags.push(first);
        }
        let v = s.validate();
        assert!(v.iter().any(|x| matches!(x, Violation::FieldTagDuplicateName { .. })));
    }

    #[test]
    fn field_tag_template_missing_splice_holes_caught() {
        let mut s = good_field_tag_spec();
        if let Some(ts) = s.field_tag.as_mut() {
            ts.tags[0].per_field_template = "()".into();
        }
        let v = s.validate();
        assert!(v
            .iter()
            .any(|x| matches!(x, Violation::FieldTagTemplateMissingSpliceHoles { .. })));
    }

    #[test]
    fn per_field_template_check_skipped_in_field_tag_mode() {
        // good_field_tag_spec() has an EMPTY per_field_template (unused
        // in field_tag mode) -- if the old uniform-template check still
        // ran, this would spuriously fail with TemplateMissingSpliceHoles
        // on the ROOT spec (not a per-tag one).
        let v = good_field_tag_spec().validate();
        assert!(!v.iter().any(|x| matches!(
            x,
            Violation::TemplateMissingSpliceHoles { spec_name, .. } if spec_name == "HotSwap"
        )));
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
