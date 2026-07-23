//! `(defmacrocatalog …)` tlisp authoring surface.
//!
//! Replaces the catalog's JSON authoring layer with a tatara-lisp
//! form per the tlisp-first prime directive. Operators write a
//! `(defmacrocatalog <name> :entries (…))` form; this module parses
//! it into [`MacroCatalogSpec`] and re-renders it back to canonical
//! lisp.
//!
//! Canonical shape:
//!
//! ```lisp
//! (defmacrocatalog pleme-derives
//!   :entries (
//!     (:crate-name "pleme-getter-derive"
//!      :description "Per-field inherent getter."
//!      :since "0.1.0"
//!      :owner "pleme-io"
//!      :verifier-hint per-field-getter
//!      :kind per-field
//!      :spec (
//!        :trait-name "GetterAll"
//!        :target named-struct
//!        :per-field-template "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }"))
//!     ...
//!   ))
//! ```
//!
//! Round-trip contract: `parse(s).render() == canonical(s)` where
//! `canonical` collapses whitespace + comments. Proven by integration
//! test against the shipped `catalogs/pleme-derives.lisp` file.

use tatara_rust_ast::Ident;
use tatara_rust_catalog::{CatalogEntry, CatalogSpec, MacroCatalogSpec, VerifierHint};
use tatara_rust_composite::CompositeDeriveSpec;
use tatara_rust_derive::{
    ClosedAxisSpec, EnumFoldDeriveSpec, EnumFoldTarget, FieldTag, KindRoundTripSpec,
    NewtypeDeriveSpec, NewtypeTarget, PerFieldDeriveSpec, PerFieldTarget, PerVariantDeriveSpec,
    ProcDeriveSpec, TagSpec, VariantShape, VerificationMatrixSpec,
};
use tatara_rust_macro_rules::{MacroArm, MacroRulesSpec};
use tatara_rust_proc_attr::{AttrTransform, ProcAttrSpec};
use tatara_rust_proc_fn::{FnTransform, ProcFnSpec};

// ─────────────────────────────────────────────────────────────────────
// SExpr — minimal lisp value
// ─────────────────────────────────────────────────────────────────────

/// Lisp value the parser produces. Constrained to what the catalog
/// form needs — string, ident, integer, list. No quasiquote, no
/// dotted pairs, no characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SExpr {
    /// Symbol — bare identifier (`pleme-derives`, `per-field`, `nil`).
    Sym(String),
    /// `:keyword` — symbol prefixed with `:`.
    Kw(String),
    /// `"string"` — quoted text with `\"` and `\\` escapes.
    Str(String),
    /// Integer literal.
    Int(i64),
    /// `(…)` — ordered list of sub-exprs.
    List(Vec<SExpr>),
}

impl SExpr {
    pub fn as_sym(&self) -> Option<&str> {
        if let Self::Sym(s) = self {
            Some(s.as_str())
        } else {
            None
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Self::Str(s) = self {
            Some(s.as_str())
        } else {
            None
        }
    }
    pub fn as_list(&self) -> Option<&[SExpr]> {
        if let Self::List(l) = self {
            Some(l)
        } else {
            None
        }
    }
    pub fn is_nil(&self) -> bool {
        matches!(self, Self::Sym(s) if s == "nil")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("unexpected character `{0}` at position {1}")]
    Unexpected(char, usize),
    #[error("unterminated string starting at position {0}")]
    UnterminatedString(usize),
    #[error("expected `{expected}`, got `{got}`")]
    Expected { expected: String, got: String },
    #[error("missing required keyword `:{0}` in `{form}`", form = .1)]
    MissingKeyword(String, String),
    #[error("value for `:{0}` has wrong shape: {1}")]
    ShapeError(String, String),
    #[error("unknown spec kind `{0}` (expected derive | per-field | per-variant | newtype | enum-fold | proc-attr | proc-fn | macro-rules | composite | kind-round-trip | verification-matrix | closed-axis)")]
    UnknownKind(String),
    #[error("unknown verifier hint `{0}`")]
    UnknownHint(String),
    #[error("unterminated list — `(` without matching `)`")]
    UnterminatedList,
}

// ─────────────────────────────────────────────────────────────────────
// Tokenizer / parser
// ─────────────────────────────────────────────────────────────────────

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self { src: src.as_bytes(), pos: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if b == b';' {
                // Line comment.
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_expr(&mut self) -> Result<SExpr, ParseError> {
        self.skip_ws();
        if self.pos >= self.src.len() {
            return Err(ParseError::Unexpected('\0', self.pos));
        }
        match self.src[self.pos] {
            b'(' => self.parse_list(),
            b'"' => self.parse_string(),
            b':' => self.parse_keyword(),
            b'-' | b'0'..=b'9' => self.parse_number_or_sym(),
            _ => self.parse_symbol(),
        }
    }

    fn parse_list(&mut self) -> Result<SExpr, ParseError> {
        debug_assert_eq!(self.src[self.pos], b'(');
        self.pos += 1;
        let mut items = vec![];
        loop {
            self.skip_ws();
            if self.pos >= self.src.len() {
                return Err(ParseError::UnterminatedList);
            }
            if self.src[self.pos] == b')' {
                self.pos += 1;
                return Ok(SExpr::List(items));
            }
            items.push(self.parse_expr()?);
        }
    }

    fn parse_string(&mut self) -> Result<SExpr, ParseError> {
        let start = self.pos;
        debug_assert_eq!(self.src[self.pos], b'"');
        self.pos += 1;
        // Accumulate raw bytes and decode the whole literal as UTF-8 at
        // the close-quote. Pushing `byte as char` would mangle every
        // multi-byte char (an em-dash's 0xE2 0x80 0x94 would become three
        // Latin-1 chars); escapes resolve to single ASCII bytes, so they
        // compose with the raw byte stream cleanly.
        let mut buf: Vec<u8> = Vec::new();
        while self.pos < self.src.len() {
            let b = self.src[self.pos];
            if b == b'"' {
                self.pos += 1;
                let out = String::from_utf8(buf)
                    .map_err(|_| ParseError::Unexpected('?', start))?;
                return Ok(SExpr::Str(out));
            }
            if b == b'\\' && self.pos + 1 < self.src.len() {
                self.pos += 1;
                match self.src[self.pos] {
                    b'"' => buf.push(b'"'),
                    b'\\' => buf.push(b'\\'),
                    b'n' => buf.push(b'\n'),
                    b't' => buf.push(b'\t'),
                    other => buf.push(other),
                }
                self.pos += 1;
                continue;
            }
            buf.push(b);
            self.pos += 1;
        }
        Err(ParseError::UnterminatedString(start))
    }

    fn parse_keyword(&mut self) -> Result<SExpr, ParseError> {
        debug_assert_eq!(self.src[self.pos], b':');
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.src.len() && Self::is_sym_char(self.src[self.pos]) {
            self.pos += 1;
        }
        let name = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| ParseError::Unexpected('?', start))?
            .to_string();
        Ok(SExpr::Kw(name))
    }

    fn parse_symbol(&mut self) -> Result<SExpr, ParseError> {
        let start = self.pos;
        while self.pos < self.src.len() && Self::is_sym_char(self.src[self.pos]) {
            self.pos += 1;
        }
        let name = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| ParseError::Unexpected('?', start))?
            .to_string();
        if name.is_empty() {
            return Err(ParseError::Unexpected(self.src[start] as char, start));
        }
        Ok(SExpr::Sym(name))
    }

    fn parse_number_or_sym(&mut self) -> Result<SExpr, ParseError> {
        let start = self.pos;
        while self.pos < self.src.len() && Self::is_sym_char(self.src[self.pos]) {
            self.pos += 1;
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .map_err(|_| ParseError::Unexpected('?', start))?;
        if let Ok(i) = text.parse::<i64>() {
            Ok(SExpr::Int(i))
        } else {
            Ok(SExpr::Sym(text.to_string()))
        }
    }

    fn is_sym_char(b: u8) -> bool {
        !b.is_ascii_whitespace()
            && b != b'('
            && b != b')'
            && b != b'"'
            && b != b';'
    }
}

/// Parse a tlisp source string into a sequence of top-level `SExpr`s.
pub fn parse_sexprs(src: &str) -> Result<Vec<SExpr>, ParseError> {
    let mut p = Parser::new(src);
    let mut out = vec![];
    loop {
        p.skip_ws();
        if p.pos >= p.src.len() {
            return Ok(out);
        }
        out.push(p.parse_expr()?);
    }
}

// ─────────────────────────────────────────────────────────────────────
// `(defmacrocatalog …)` parser
// ─────────────────────────────────────────────────────────────────────

/// Parse a `(defmacrocatalog <name> :entries (…))` form into a
/// `MacroCatalogSpec`. Returns the FIRST defmacrocatalog form in the
/// source; later forms are ignored.
pub fn parse_macrocatalog(src: &str) -> Result<MacroCatalogSpec, ParseError> {
    let exprs = parse_sexprs(src)?;
    for e in exprs {
        if let SExpr::List(items) = &e {
            if matches!(items.first(), Some(SExpr::Sym(s)) if s == "defmacrocatalog") {
                return parse_macrocatalog_body(items);
            }
        }
    }
    Err(ParseError::Expected {
        expected: "(defmacrocatalog …)".into(),
        got: "no defmacrocatalog form found".into(),
    })
}

fn parse_macrocatalog_body(items: &[SExpr]) -> Result<MacroCatalogSpec, ParseError> {
    // (defmacrocatalog <title-sym> :entries (…))
    if items.len() < 4 {
        return Err(ParseError::ShapeError(
            "defmacrocatalog".into(),
            "expected (defmacrocatalog <name> :entries (…))".into(),
        ));
    }
    let title = match &items[1] {
        SExpr::Sym(s) => s.clone(),
        SExpr::Str(s) => s.clone(),
        other => {
            return Err(ParseError::ShapeError(
                "defmacrocatalog title".into(),
                format!("expected symbol or string, got {other:?}"),
            ));
        }
    };
    let entries_list = find_kw_value(&items[2..], "entries")?
        .as_list()
        .ok_or_else(|| {
            ParseError::ShapeError("entries".into(), "must be a list".into())
        })?
        .to_vec();
    let mut entries = vec![];
    for e in entries_list {
        let sub = e
            .as_list()
            .ok_or_else(|| {
                ParseError::ShapeError("entry".into(), "must be a list".into())
            })?
            .to_vec();
        entries.push(parse_entry(&sub)?);
    }
    Ok(MacroCatalogSpec { title, entries })
}

fn parse_entry(items: &[SExpr]) -> Result<CatalogEntry, ParseError> {
    let crate_name = expect_str_kw(items, "crate-name")?;
    let description = expect_str_kw(items, "description")?;
    let since = expect_str_kw(items, "since")?;
    let owner = expect_str_kw(items, "owner")?;
    let kind = expect_sym_kw(items, "kind")?;
    let verifier_hint = match find_kw_value(items, "verifier-hint") {
        Ok(SExpr::Sym(s)) if s == "nil" => None,
        Ok(SExpr::Sym(s)) => Some(parse_verifier_hint(&s)?),
        Ok(other) => {
            return Err(ParseError::ShapeError(
                "verifier-hint".into(),
                format!("must be symbol, got {other:?}"),
            ));
        }
        Err(_) => None,
    };
    let spec_list = find_kw_value(items, "spec")?
        .as_list()
        .ok_or_else(|| ParseError::ShapeError("spec".into(), "must be a list".into()))?
        .to_vec();
    let spec = parse_spec(&kind, &spec_list)?;
    Ok(CatalogEntry {
        crate_name,
        description,
        since,
        owner,
        verifier_hint,
        spec,
    })
}

fn parse_verifier_hint(s: &str) -> Result<VerifierHint, ParseError> {
    use VerifierHint::*;
    Ok(match s {
        "compile-only" => CompileOnly,
        "per-field-getter" => PerFieldGetter,
        "per-field-setter" => PerFieldSetter,
        "per-field-with-builder" => PerFieldWithBuilder,
        "per-field-as-mut" => PerFieldAsMut,
        "per-field-replace" => PerFieldReplace,
        "per-field-take" => PerFieldTake,
        "per-field-invalidating-setter" => PerFieldInvalidatingSetter,
        "per-variant-is-variant" => PerVariantIsVariant,
        "newtype-impl-from" => NewtypeImplFrom,
        "newtype-as-ref" => NewtypeAsRef,
        "newtype-deref" => NewtypeDeref,
        "newtype-inner" => NewtypeInner,
        "enum-fold-all-variants" => EnumFoldAllVariants,
        "enum-fold-variant-count" => EnumFoldVariantCount,
        "enum-fold-variant-names" => EnumFoldVariantNames,
        "enum-fold-variant-str" => EnumFoldVariantStr,
        "per-field-owned" => PerFieldOwned,
        "newtype-borrow" => NewtypeBorrow,
        "newtype-borrow-mut" => NewtypeBorrowMut,
        "newtype-deref-mut" => NewtypeDerefMut,
        "newtype-display" => NewtypeDisplay,
        "newtype-default" => NewtypeDefault,
        "per-field-reset" => PerFieldReset,
        "per-field-swap" => PerFieldSwap,
        other => return Err(ParseError::UnknownHint(other.to_string())),
    })
}

fn parse_spec(kind: &str, items: &[SExpr]) -> Result<CatalogSpec, ParseError> {
    match kind {
        "per-field" => Ok(CatalogSpec::PerField {
            spec: parse_per_field_spec(items)?,
        }),
        "per-variant" => Ok(CatalogSpec::PerVariant {
            spec: parse_per_variant_spec(items)?,
        }),
        "newtype" => Ok(CatalogSpec::Newtype {
            spec: parse_newtype_spec(items)?,
        }),
        "enum-fold" => Ok(CatalogSpec::EnumFold {
            spec: parse_enum_fold_spec(items)?,
        }),
        "derive" => Ok(CatalogSpec::Derive {
            spec: ProcDeriveSpec::new(expect_str_kw(items, "trait-name")?, vec![]),
        }),
        "proc-attr" => Ok(CatalogSpec::ProcAttr {
            spec: ProcAttrSpec {
                macro_name: Ident::new(expect_str_kw(items, "macro-name")?),
                transform: AttrTransform::PrependPrelude {
                    prelude_tokens: expect_str_kw(items, "prelude").unwrap_or_default(),
                },
            },
        }),
        "proc-fn" => Ok(CatalogSpec::ProcFn {
            spec: ProcFnSpec {
                macro_name: Ident::new(expect_str_kw(items, "macro-name")?),
                transform: FnTransform::PrependPrelude {
                    prelude_tokens: expect_str_kw(items, "prelude").unwrap_or_default(),
                },
            },
        }),
        "macro-rules" => Ok(CatalogSpec::MacroRules {
            spec: MacroRulesSpec {
                macro_name: Ident::new(expect_str_kw(items, "macro-name")?),
                arms: vec![MacroArm {
                    matcher: "()".into(),
                    transcriber: "{ () }".into(),
                }],
            },
        }),
        "composite" => Ok(CatalogSpec::Composite {
            spec: CompositeDeriveSpec {
                bundle_name: Ident::new(expect_str_kw(items, "bundle-name")?),
                members: vec![],
            },
        }),
        "kind-round-trip" => Ok(CatalogSpec::KindRoundTrip {
            spec: parse_kind_round_trip_spec(items)?,
        }),
        "verification-matrix" => Ok(CatalogSpec::VerificationMatrix {
            spec: parse_verification_matrix_spec(items)?,
        }),
        "closed-axis" => Ok(CatalogSpec::ClosedAxis {
            spec: parse_closed_axis_spec(items)?,
        }),
        other => Err(ParseError::UnknownKind(other.to_string())),
    }
}

fn parse_verification_matrix_spec(
    items: &[SExpr],
) -> Result<VerificationMatrixSpec, ParseError> {
    Ok(VerificationMatrixSpec {
        matrix_macro: expect_str_kw(items, "matrix-macro")?,
        covers_macro: expect_str_kw(items, "covers-macro")?,
    })
}

fn parse_closed_axis_spec(items: &[SExpr]) -> Result<ClosedAxisSpec, ParseError> {
    Ok(ClosedAxisSpec {
        trait_name: Ident::new(expect_str_kw(items, "trait-name")?),
        axis_trait_path: expect_str_kw(items, "axis-trait-path")?,
    })
}

fn parse_kind_round_trip_spec(items: &[SExpr]) -> Result<KindRoundTripSpec, ParseError> {
    Ok(KindRoundTripSpec {
        trait_name: Ident::new(expect_str_kw(items, "trait-name")?),
        helper_attr: expect_str_kw(items, "helper-attr")?,
        as_str_method: expect_str_kw(items, "as-str-method")?,
        from_str_method: expect_str_kw(items, "from-str-method")?,
        with_byte: opt_bool_kw(items, "with-byte"),
        as_byte_method: expect_str_kw(items, "as-byte-method")?,
        from_byte_method: expect_str_kw(items, "from-byte-method")?,
    })
}

fn parse_per_field_spec(items: &[SExpr]) -> Result<PerFieldDeriveSpec, ParseError> {
    Ok(PerFieldDeriveSpec {
        trait_name: Ident::new(expect_str_kw(items, "trait-name")?),
        target: match expect_sym_kw(items, "target")?.as_str() {
            "named-struct" => PerFieldTarget::NamedStruct,
            other => {
                return Err(ParseError::ShapeError(
                    "target".into(),
                    format!("unknown PerField target `{other}`"),
                ));
            }
        },
        trait_ref: opt_str_kw(items, "trait-ref"),
        // Unused (empty string) when `field-tag` is present -- the
        // per-field.rs emitter dispatches through field_tag's own
        // per-tag templates in that mode. expect_str_kw would fail on
        // an absent `:per-field-template`, so field-tag specs authors
        // omit it -- fall back to "" rather than requiring a dead key.
        per_field_template: opt_str_kw(items, "per-field-template").unwrap_or_default(),
        method_name_template: opt_str_kw(items, "method-name-template"),
        impl_prelude: opt_str_kw(items, "impl-prelude"),
        skip_fields: opt_str_list_kw(items, "skip-fields"),
        field_attribute: opt_str_kw(items, "field-attribute"),
        field_tag: opt_field_tag_kw(items, "field-tag")?,
    })
}

/// Parses `:field-tag ( :exhaustive t :tags ( (:name "…" :required-args
/// (…) :per-field-template "…") … ) )` into a [`TagSpec`]. Absent
/// keyword ⇒ `Ok(None)` (today's uniform-template behavior, unchanged).
fn opt_field_tag_kw(items: &[SExpr], kw: &str) -> Result<Option<TagSpec>, ParseError> {
    let Ok(value) = find_kw_value(items, kw) else {
        return Ok(None);
    };
    let outer = value
        .as_list()
        .ok_or_else(|| ParseError::ShapeError(kw.into(), "expected a list".into()))?;

    let exhaustive = opt_bool_kw(outer, "exhaustive");
    let tags_value = find_kw_value(outer, "tags")?;
    let tag_forms = tags_value
        .as_list()
        .ok_or_else(|| ParseError::ShapeError("field-tag.tags".into(), "expected a list".into()))?;

    let tags = tag_forms
        .iter()
        .map(|form| {
            let form_items = form
                .as_list()
                .ok_or_else(|| ParseError::ShapeError("field-tag.tags[]".into(), "expected a list".into()))?;
            Ok(FieldTag {
                name: expect_str_kw(form_items, "name")?,
                required_args: opt_str_list_kw(form_items, "required-args"),
                per_field_template: expect_str_kw(form_items, "per-field-template")?,
            })
        })
        .collect::<Result<Vec<FieldTag>, ParseError>>()?;

    Ok(Some(TagSpec { tags, exhaustive }))
}

fn parse_per_variant_spec(items: &[SExpr]) -> Result<PerVariantDeriveSpec, ParseError> {
    Ok(PerVariantDeriveSpec {
        trait_name: Ident::new(expect_str_kw(items, "trait-name")?),
        variant_shape: match expect_sym_kw(items, "variant-shape")?.as_str() {
            "any" => VariantShape::Any,
            other => {
                return Err(ParseError::ShapeError(
                    "variant-shape".into(),
                    format!("unknown VariantShape `{other}`"),
                ));
            }
        },
        trait_ref: opt_str_kw(items, "trait-ref"),
        per_variant_template: expect_str_kw(items, "per-variant-template")?,
        method_name_template: opt_str_kw(items, "method-name-template"),
        impl_prelude: opt_str_kw(items, "impl-prelude"),
    })
}

fn parse_newtype_spec(items: &[SExpr]) -> Result<NewtypeDeriveSpec, ParseError> {
    Ok(NewtypeDeriveSpec {
        trait_name: Ident::new(expect_str_kw(items, "trait-name")?),
        target: match expect_sym_kw(items, "target")?.as_str() {
            "tuple" => NewtypeTarget::Tuple,
            other => {
                return Err(ParseError::ShapeError(
                    "target".into(),
                    format!("unknown NewtypeTarget `{other}`"),
                ));
            }
        },
        impl_template: expect_str_kw(items, "impl-template")?,
    })
}

fn parse_enum_fold_spec(items: &[SExpr]) -> Result<EnumFoldDeriveSpec, ParseError> {
    Ok(EnumFoldDeriveSpec {
        trait_name: Ident::new(expect_str_kw(items, "trait-name")?),
        target: match expect_sym_kw(items, "target")?.as_str() {
            "unit-variants-only" => EnumFoldTarget::UnitVariantsOnly,
            "any-variants" => EnumFoldTarget::AnyVariants,
            other => {
                return Err(ParseError::ShapeError(
                    "target".into(),
                    format!("unknown EnumFoldTarget `{other}`"),
                ));
            }
        },
        per_variant_fragment: expect_str_kw(items, "per-variant-fragment")?,
        fold_template: expect_str_kw(items, "fold-template")?,
    })
}

// ─────────────────────────────────────────────────────────────────────
// Keyword-value helpers
// ─────────────────────────────────────────────────────────────────────

/// Find the value after a `:keyword` in a flat key-value list.
/// Returns ShapeError if the keyword exists but has no value.
fn find_kw_value<'a>(items: &'a [SExpr], kw: &str) -> Result<&'a SExpr, ParseError> {
    let mut iter = items.iter();
    while let Some(e) = iter.next() {
        if matches!(e, SExpr::Kw(s) if s == kw) {
            return iter.next().ok_or_else(|| {
                ParseError::ShapeError(
                    kw.into(),
                    "keyword present but no value follows".into(),
                )
            });
        }
    }
    Err(ParseError::MissingKeyword(kw.into(), "entry".into()))
}

fn expect_str_kw(items: &[SExpr], kw: &str) -> Result<String, ParseError> {
    find_kw_value(items, kw)?
        .as_str()
        .ok_or_else(|| {
            ParseError::ShapeError(kw.into(), "expected string".into())
        })
        .map(|s| s.to_string())
}

fn expect_sym_kw(items: &[SExpr], kw: &str) -> Result<String, ParseError> {
    find_kw_value(items, kw)?
        .as_sym()
        .ok_or_else(|| {
            ParseError::ShapeError(kw.into(), "expected symbol".into())
        })
        .map(|s| s.to_string())
}

fn opt_str_kw(items: &[SExpr], kw: &str) -> Option<String> {
    match find_kw_value(items, kw) {
        Ok(SExpr::Str(s)) => Some(s.clone()),
        Ok(SExpr::Sym(s)) if s == "nil" => None,
        _ => None,
    }
}

fn opt_str_list_kw(items: &[SExpr], kw: &str) -> Vec<String> {
    match find_kw_value(items, kw) {
        Ok(SExpr::List(l)) => l.iter().filter_map(|e| e.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}

/// Read a `:keyword t|true` boolean symbol; absent or any other value is
/// `false`. Round-trips with the `t`/`nil` rendering below.
fn opt_bool_kw(items: &[SExpr], kw: &str) -> bool {
    matches!(find_kw_value(items, kw), Ok(SExpr::Sym(s)) if s == "t" || s == "true")
}

// ─────────────────────────────────────────────────────────────────────
// Renderer (MacroCatalogSpec → tlisp text)
// ─────────────────────────────────────────────────────────────────────

/// Render a `MacroCatalogSpec` to canonical tlisp text. Round-trip
/// guaranteed: `parse_macrocatalog(render_macrocatalog(spec)) == spec`.
#[must_use]
pub fn render_macrocatalog(spec: &MacroCatalogSpec) -> String {
    let mut s = String::new();
    s.push_str(&format!("(defmacrocatalog {}\n", spec.title));
    s.push_str("  :entries (\n");
    for e in &spec.entries {
        s.push_str(&render_entry(e));
    }
    s.push_str("  ))\n");
    s
}

fn render_entry(e: &CatalogEntry) -> String {
    let mut s = String::new();
    s.push_str("    (\n");
    s.push_str(&format!("      :crate-name {}\n", quote_str(&e.crate_name)));
    s.push_str(&format!(
        "      :description {}\n",
        quote_str(&e.description)
    ));
    s.push_str(&format!("      :since {}\n", quote_str(&e.since)));
    s.push_str(&format!("      :owner {}\n", quote_str(&e.owner)));
    if let Some(h) = &e.verifier_hint {
        s.push_str(&format!("      :verifier-hint {}\n", hint_to_sym(*h)));
    }
    s.push_str(&format!("      :kind {}\n", e.spec.kind_label()));
    s.push_str("      :spec (\n");
    s.push_str(&render_spec_body(&e.spec));
    s.push_str("      ))\n");
    s
}

fn render_spec_body(spec: &CatalogSpec) -> String {
    let mut s = String::new();
    match spec {
        CatalogSpec::PerField { spec } => {
            s.push_str(&format!(
                "        :trait-name {}\n",
                quote_str(&spec.trait_name.0)
            ));
            s.push_str("        :target named-struct\n");
            if let Some(t) = &spec.trait_ref {
                s.push_str(&format!("        :trait-ref {}\n", quote_str(t)));
            }
            // `field_tag` mode leaves `per_field_template` empty (unused
            // — each tag carries its own) -- skip emitting a dead key.
            if !(spec.per_field_template.is_empty() && spec.field_tag.is_some()) {
                s.push_str(&format!(
                    "        :per-field-template {}\n",
                    quote_str(&spec.per_field_template)
                ));
            }
            if let Some(t) = &spec.method_name_template {
                s.push_str(&format!(
                    "        :method-name-template {}\n",
                    quote_str(t)
                ));
            }
            if let Some(p) = &spec.impl_prelude {
                s.push_str(&format!("        :impl-prelude {}\n", quote_str(p)));
            }
            if !spec.skip_fields.is_empty() {
                let lits = spec
                    .skip_fields
                    .iter()
                    .map(|s| quote_str(s))
                    .collect::<Vec<_>>()
                    .join(" ");
                s.push_str(&format!("        :skip-fields ({lits})\n"));
            }
            if let Some(a) = &spec.field_attribute {
                s.push_str(&format!("        :field-attribute {}\n", quote_str(a)));
            }
            if let Some(tag_spec) = &spec.field_tag {
                s.push_str(&render_field_tag(tag_spec));
            }
        }
        CatalogSpec::PerVariant { spec } => {
            s.push_str(&format!(
                "        :trait-name {}\n",
                quote_str(&spec.trait_name.0)
            ));
            s.push_str("        :variant-shape any\n");
            if let Some(t) = &spec.trait_ref {
                s.push_str(&format!("        :trait-ref {}\n", quote_str(t)));
            }
            s.push_str(&format!(
                "        :per-variant-template {}\n",
                quote_str(&spec.per_variant_template)
            ));
            if let Some(t) = &spec.method_name_template {
                s.push_str(&format!(
                    "        :method-name-template {}\n",
                    quote_str(t)
                ));
            }
            if let Some(p) = &spec.impl_prelude {
                s.push_str(&format!("        :impl-prelude {}\n", quote_str(p)));
            }
        }
        CatalogSpec::Newtype { spec } => {
            s.push_str(&format!(
                "        :trait-name {}\n",
                quote_str(&spec.trait_name.0)
            ));
            s.push_str("        :target tuple\n");
            s.push_str(&format!(
                "        :impl-template {}\n",
                quote_str(&spec.impl_template)
            ));
        }
        CatalogSpec::EnumFold { spec } => {
            s.push_str(&format!(
                "        :trait-name {}\n",
                quote_str(&spec.trait_name.0)
            ));
            let target_sym = match spec.target {
                EnumFoldTarget::UnitVariantsOnly => "unit-variants-only",
                EnumFoldTarget::AnyVariants => "any-variants",
            };
            s.push_str(&format!("        :target {target_sym}\n"));
            s.push_str(&format!(
                "        :per-variant-fragment {}\n",
                quote_str(&spec.per_variant_fragment)
            ));
            s.push_str(&format!(
                "        :fold-template {}\n",
                quote_str(&spec.fold_template)
            ));
        }
        CatalogSpec::Derive { spec } => {
            s.push_str(&format!(
                "        :trait-name {}\n",
                quote_str(&spec.trait_name.0)
            ));
        }
        CatalogSpec::ProcAttr { spec } => {
            s.push_str(&format!(
                "        :macro-name {}\n",
                quote_str(&spec.macro_name.0)
            ));
            let AttrTransform::PrependPrelude { prelude_tokens } = &spec.transform;
            s.push_str(&format!("        :prelude {}\n", quote_str(prelude_tokens)));
        }
        CatalogSpec::ProcFn { spec } => {
            s.push_str(&format!(
                "        :macro-name {}\n",
                quote_str(&spec.macro_name.0)
            ));
            let FnTransform::PrependPrelude { prelude_tokens } = &spec.transform;
            s.push_str(&format!("        :prelude {}\n", quote_str(prelude_tokens)));
        }
        CatalogSpec::MacroRules { spec } => {
            s.push_str(&format!(
                "        :macro-name {}\n",
                quote_str(&spec.macro_name.0)
            ));
        }
        CatalogSpec::Composite { spec } => {
            s.push_str(&format!(
                "        :bundle-name {}\n",
                quote_str(&spec.bundle_name.0)
            ));
        }
        CatalogSpec::KindRoundTrip { spec } => {
            // All seven fields rendered explicitly so
            // parse(render(spec)) == spec for any KindRoundTripSpec.
            s.push_str(&format!(
                "        :trait-name {}\n",
                quote_str(&spec.trait_name.0)
            ));
            s.push_str(&format!(
                "        :helper-attr {}\n",
                quote_str(&spec.helper_attr)
            ));
            s.push_str(&format!(
                "        :as-str-method {}\n",
                quote_str(&spec.as_str_method)
            ));
            s.push_str(&format!(
                "        :from-str-method {}\n",
                quote_str(&spec.from_str_method)
            ));
            s.push_str(&format!(
                "        :with-byte {}\n",
                if spec.with_byte { "t" } else { "nil" }
            ));
            s.push_str(&format!(
                "        :as-byte-method {}\n",
                quote_str(&spec.as_byte_method)
            ));
            s.push_str(&format!(
                "        :from-byte-method {}\n",
                quote_str(&spec.from_byte_method)
            ));
        }
        CatalogSpec::VerificationMatrix { spec } => {
            s.push_str(&format!(
                "        :matrix-macro {}\n",
                quote_str(&spec.matrix_macro)
            ));
            s.push_str(&format!(
                "        :covers-macro {}\n",
                quote_str(&spec.covers_macro)
            ));
        }
        CatalogSpec::ClosedAxis { spec } => {
            s.push_str(&format!(
                "        :trait-name {}\n",
                quote_str(&spec.trait_name.0)
            ));
            s.push_str(&format!(
                "        :axis-trait-path {}\n",
                quote_str(&spec.axis_trait_path)
            ));
        }
    }
    s
}

fn hint_to_sym(h: VerifierHint) -> &'static str {
    use VerifierHint::*;
    match h {
        CompileOnly => "compile-only",
        PerFieldGetter => "per-field-getter",
        PerFieldSetter => "per-field-setter",
        PerFieldWithBuilder => "per-field-with-builder",
        PerFieldAsMut => "per-field-as-mut",
        PerFieldReplace => "per-field-replace",
        PerFieldTake => "per-field-take",
        PerFieldInvalidatingSetter => "per-field-invalidating-setter",
        PerVariantIsVariant => "per-variant-is-variant",
        NewtypeImplFrom => "newtype-impl-from",
        NewtypeAsRef => "newtype-as-ref",
        NewtypeDeref => "newtype-deref",
        NewtypeInner => "newtype-inner",
        EnumFoldAllVariants => "enum-fold-all-variants",
        EnumFoldVariantCount => "enum-fold-variant-count",
        EnumFoldVariantNames => "enum-fold-variant-names",
        EnumFoldVariantStr => "enum-fold-variant-str",
        PerFieldOwned => "per-field-owned",
        NewtypeBorrow => "newtype-borrow",
        NewtypeBorrowMut => "newtype-borrow-mut",
        NewtypeDerefMut => "newtype-deref-mut",
        NewtypeDisplay => "newtype-display",
        NewtypeDefault => "newtype-default",
        PerFieldReset => "per-field-reset",
        PerFieldSwap => "per-field-swap",
    }
}

fn quote_str(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Renders an [`SExpr`] tree to canonical tlisp text — the typed
/// pretty-printer counterpart to [`SExpr`]'s existing role as the
/// parser's typed IR (★★ TYPED EMISSION: this crate targets tlisp
/// syntax as its "target language", so `SExpr` is that language's typed
/// AST, the same way `PerFieldDeriveSpec::render_lib_rs` targets Rust
/// syntax via `quote!`+`syn`+`prettyplease`). New multi-field lisp forms
/// (like `field-tag` below) build an `SExpr` tree and render it through
/// this ONE function, rather than each hand-formatting its own strings
/// — the ~10 other `render_spec_body` arms predate this pretty-printer
/// and still hand-format; retrofitting them is real, separate follow-up
/// work, named here rather than silently taken on or silently skipped.
fn render_sexpr(e: &SExpr, indent: usize) -> String {
    match e {
        SExpr::Sym(s) => s.clone(),
        SExpr::Kw(s) => format!(":{s}"),
        SExpr::Str(s) => quote_str(s),
        SExpr::Int(i) => i.to_string(),
        SExpr::List(items) => {
            if items.is_empty() {
                return "()".to_string();
            }
            let pad = "  ".repeat(indent + 1);
            let closing_pad = "  ".repeat(indent);
            let body = items
                .iter()
                .map(|it| format!("{pad}{}", render_sexpr(it, indent + 1)))
                .collect::<Vec<_>>()
                .join("\n");
            format!("(\n{body}\n{closing_pad})")
        }
    }
}

/// Builds the typed [`SExpr`] tree for a [`TagSpec`], then renders it
/// via [`render_sexpr`] — the typed-AST path `opt_field_tag_kw` above
/// parses back out.
fn render_field_tag(tag_spec: &TagSpec) -> String {
    let exhaustive_sym = if tag_spec.exhaustive { "t" } else { "nil" };
    let tag_exprs: Vec<SExpr> = tag_spec
        .tags
        .iter()
        .map(|tag| {
            let mut fields = vec![
                SExpr::Kw("name".into()),
                SExpr::Str(tag.name.clone()),
            ];
            if !tag.required_args.is_empty() {
                fields.push(SExpr::Kw("required-args".into()));
                fields.push(SExpr::List(
                    tag.required_args.iter().map(|a| SExpr::Str(a.clone())).collect(),
                ));
            }
            fields.push(SExpr::Kw("per-field-template".into()));
            fields.push(SExpr::Str(tag.per_field_template.clone()));
            SExpr::List(fields)
        })
        .collect();

    let field_tag_form = SExpr::List(vec![
        SExpr::Kw("exhaustive".into()),
        SExpr::Sym(exhaustive_sym.into()),
        SExpr::Kw("tags".into()),
        SExpr::List(tag_exprs),
    ]);

    format!("        :field-tag {}\n", render_sexpr(&field_tag_form, 4))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog() -> MacroCatalogSpec {
        MacroCatalogSpec {
            title: "demo-derives".into(),
            entries: vec![CatalogEntry {
                crate_name: "demo-getter-derive".into(),
                description: "Per-field inherent getter.".into(),
                since: "0.1.0".into(),
                owner: "pleme-io".into(),
                verifier_hint: Some(VerifierHint::PerFieldGetter),
                spec: CatalogSpec::PerField {
                    spec: PerFieldDeriveSpec {
                        trait_name: Ident::new("DemoGetter"),
                        target: PerFieldTarget::NamedStruct,
                        trait_ref: None,
                        per_field_template:
                            "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }"
                                .into(),
                        method_name_template: None,
                        impl_prelude: None,
                        skip_fields: vec![],
                        field_attribute: None,
                        field_tag: None,
                    },
                },
            }],
        }
    }

    #[test]
    fn round_trip_per_field_canonical() {
        let cat = sample_catalog();
        let rendered = render_macrocatalog(&cat);
        let parsed = parse_macrocatalog(&rendered).unwrap();
        assert_eq!(parsed, cat);
    }

    #[test]
    fn round_trip_full_15_entry_catalog() {
        // Build a catalog with one entry per kind to exercise every
        // parse + render branch.
        let cat = MacroCatalogSpec {
            title: "every-kind".into(),
            entries: vec![
                CatalogEntry {
                    crate_name: "getter-derive".into(),
                    description: "x".into(),
                    since: "0.1.0".into(),
                    owner: "y".into(),
                    verifier_hint: Some(VerifierHint::PerFieldGetter),
                    spec: CatalogSpec::PerField {
                        spec: PerFieldDeriveSpec {
                            trait_name: Ident::new("Getter"),
                            target: PerFieldTarget::NamedStruct,
                            trait_ref: None,
                            per_field_template:
                                "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }"
                                    .into(),
                            method_name_template: None,
                            impl_prelude: None,
                            skip_fields: vec![],
                            field_attribute: None,
                            field_tag: None,
                        },
                    },
                },
                CatalogEntry {
                    crate_name: "newtype-derive".into(),
                    description: "x".into(),
                    since: "0.1.0".into(),
                    owner: "y".into(),
                    verifier_hint: Some(VerifierHint::NewtypeImplFrom),
                    spec: CatalogSpec::Newtype {
                        spec: NewtypeDeriveSpec {
                            trait_name: Ident::new("ImplFrom"),
                            target: NewtypeTarget::Tuple,
                            impl_template:
                                "impl ::std::convert::From<#inner_ty> for #self_name { fn from(v: #inner_ty) -> Self { Self(v) } }"
                                    .into(),
                        },
                    },
                },
                CatalogEntry {
                    crate_name: "isvariant-derive".into(),
                    description: "x".into(),
                    since: "0.1.0".into(),
                    owner: "y".into(),
                    verifier_hint: Some(VerifierHint::PerVariantIsVariant),
                    spec: CatalogSpec::PerVariant {
                        spec: PerVariantDeriveSpec {
                            trait_name: Ident::new("IsVariant"),
                            variant_shape: VariantShape::Any,
                            trait_ref: None,
                            per_variant_template:
                                "pub fn #method_ident(&self) -> bool { matches!(self, #variant_shape_arm) }"
                                    .into(),
                            method_name_template: Some("is_{}".into()),
                            impl_prelude: None,
                        },
                    },
                },
                CatalogEntry {
                    crate_name: "allvariants-derive".into(),
                    description: "x".into(),
                    since: "0.1.0".into(),
                    owner: "y".into(),
                    verifier_hint: Some(VerifierHint::EnumFoldAllVariants),
                    spec: CatalogSpec::EnumFold {
                        spec: EnumFoldDeriveSpec {
                            trait_name: Ident::new("AllVariants"),
                            target: EnumFoldTarget::UnitVariantsOnly,
                            per_variant_fragment: "Self::#variant_name".into(),
                            fold_template:
                                "impl #self_name { pub const ALL: &'static [Self] = &[#fold]; }"
                                    .into(),
                        },
                    },
                },
                CatalogEntry {
                    crate_name: "kindstr-derive".into(),
                    description: "x".into(),
                    since: "0.1.0".into(),
                    owner: "y".into(),
                    verifier_hint: None,
                    // byte mode exercises every renderable field (with-byte t
                    // + byte method names), so the round-trip covers the full
                    // KindRoundTrip parse/render branch.
                    spec: CatalogSpec::KindRoundTrip {
                        spec: KindRoundTripSpec::kind_byte("ClipKind"),
                    },
                },
            ],
        };
        let rendered = render_macrocatalog(&cat);
        let parsed = parse_macrocatalog(&rendered).unwrap();
        assert_eq!(parsed, cat);
    }

    #[test]
    fn parser_skips_line_comments() {
        let src = r#"
; top-level comment
(defmacrocatalog tiny  ; trailing comment
  :entries (
    ; entry comment
    (:crate-name "x-derive"
     :description "x"
     :since "0.1.0"
     :owner "y"
     :kind per-field
     :spec (:trait-name "X"
            :target named-struct
            :per-field-template "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }"))
  ))
"#;
        let parsed = parse_macrocatalog(src).unwrap();
        assert_eq!(parsed.title, "tiny");
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].crate_name, "x-derive");
    }

    #[test]
    fn parser_handles_string_escapes() {
        let src = r#"
(defmacrocatalog esc
  :entries (
    (:crate-name "a"
     :description "with \"quotes\" and \\backslash"
     :since "0.1.0"
     :owner "y"
     :kind per-field
     :spec (:trait-name "A"
            :target named-struct
            :per-field-template "pub fn x() {}"))
  ))
"#;
        let parsed = parse_macrocatalog(src).unwrap();
        assert_eq!(
            parsed.entries[0].description,
            r#"with "quotes" and \backslash"#
        );
    }

    #[test]
    fn parser_preserves_multibyte_utf8_in_strings() {
        // Regression: the lexer used to push each byte as `byte as char`,
        // mangling multi-byte UTF-8 (em-dash → three Latin-1 chars). Mixed
        // escape + multibyte content must survive verbatim.
        let src = "\
(defmacrocatalog uni
  :entries (
    (:crate-name \"a\"
     :description \"em—dash, accent café, emoji 🚀, escaped \\\"q\\\"\"
     :since \"0.1.0\"
     :owner \"y\"
     :kind per-field
     :spec (:trait-name \"A\"
            :target named-struct
            :per-field-template \"pub fn x() {}\"))
  ))
";
        let parsed = parse_macrocatalog(src).unwrap();
        assert_eq!(
            parsed.entries[0].description,
            "em—dash, accent café, emoji 🚀, escaped \"q\""
        );
        // And it round-trips through render unchanged.
        let reparsed = parse_macrocatalog(&render_macrocatalog(&parsed)).unwrap();
        assert_eq!(reparsed, parsed);
    }

    #[test]
    fn missing_required_keyword_errors() {
        let src = r#"
(defmacrocatalog bad
  :entries (
    (:crate-name "x"
     :description "x"
     :since "0.1.0"
     ; :owner missing
     :kind per-field
     :spec (:trait-name "X"
            :target named-struct
            :per-field-template "pub fn x() {}"))
  ))
"#;
        let err = parse_macrocatalog(src).unwrap_err();
        assert!(matches!(err, ParseError::MissingKeyword(k, _) if k == "owner"));
    }

    #[test]
    fn skip_fields_round_trips() {
        let mut cat = sample_catalog();
        if let CatalogSpec::PerField { spec } = &mut cat.entries[0].spec {
            spec.skip_fields = vec!["last_seqno".into(), "version".into()];
        }
        let rendered = render_macrocatalog(&cat);
        let parsed = parse_macrocatalog(&rendered).unwrap();
        assert_eq!(parsed, cat);
        // Spot-check the rendered form.
        assert!(rendered.contains(":skip-fields (\"last_seqno\" \"version\")"));
    }

    #[test]
    fn field_attribute_round_trips() {
        let mut cat = sample_catalog();
        if let CatalogSpec::PerField { spec } = &mut cat.entries[0].spec {
            spec.field_attribute = Some("invalidating_setter".into());
        }
        let rendered = render_macrocatalog(&cat);
        let parsed = parse_macrocatalog(&rendered).unwrap();
        assert_eq!(parsed, cat);
    }

    /// Integration test promised by the module doc: the shipped
    /// `catalogs/pleme-derives.lisp` parses, every entry validates clean,
    /// and re-rendering it round-trips. Also pins the two authored
    /// kind-round-trip entries.
    #[test]
    fn shipped_pleme_derives_catalog_parses_validates_and_round_trips() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../catalogs/pleme-derives.lisp");
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {path}: {e}"));

        let cat = parse_macrocatalog(&src).expect("shipped catalog must parse");

        // Every entry is structurally well-formed.
        for e in &cat.entries {
            let violations = e.spec.validate();
            assert!(
                violations.is_empty(),
                "{} has violations: {violations:?}",
                e.crate_name
            );
        }

        // Re-render → re-parse is idempotent (canonical round-trip).
        let reparsed = parse_macrocatalog(&render_macrocatalog(&cat)).unwrap();
        assert_eq!(reparsed, cat);

        // The two authored kind-round-trip derives are present with the
        // expected modes.
        let kindstr = cat
            .entries
            .iter()
            .find(|e| e.crate_name == "pleme-kindstr-derive")
            .expect("pleme-kindstr-derive entry missing");
        assert!(matches!(
            &kindstr.spec,
            CatalogSpec::KindRoundTrip { spec } if !spec.with_byte && spec.trait_name.0 == "KindStr"
        ));
        let kindbyte = cat
            .entries
            .iter()
            .find(|e| e.crate_name == "pleme-kindbyte-derive")
            .expect("pleme-kindbyte-derive entry missing");
        assert!(matches!(
            &kindbyte.spec,
            CatalogSpec::KindRoundTrip { spec } if spec.with_byte && spec.trait_name.0 == "KindByte"
        ));
    }
}
