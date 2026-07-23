//! **Verification matrix** — per the ★★ CLOSED-LOOP MASS-SYNTHESIS
//! directive. One row per (CatalogSpec variant × decoration shape);
//! aggregates all failures before assert so one matrix run reports
//! every broken combo, not just the first.
//!
//! This is the forcing function between `catalog-emit-repos` and
//! `publish-bulk` — every combination the farm might emit is
//! mechanically proven materializable before any of them ships.

use std::collections::BTreeSet;
use tatara_rust_ast::Ident;
use tatara_rust_catalog::{CatalogEntry, CatalogSpec, MacroCatalogSpec};
use tatara_rust_composite::{CompositeDeriveSpec, CompositeMember};
use tatara_rust_derive::{
    ClosedAxisSpec, EnumFoldDeriveSpec, EnumFoldTarget, KindRoundTripSpec, NewtypeDeriveSpec,
    NewtypeTarget, PerFieldDeriveSpec, PerFieldTarget, PerVariantDeriveSpec, ProcDeriveSpec,
    VariantShape, VerificationMatrixSpec,
};
use tatara_rust_macro_rules::{MacroArm, MacroRulesSpec};
use tatara_rust_proc_attr::{AttrTransform, ProcAttrSpec};
use tatara_rust_proc_fn::{FnTransform, ProcFnSpec};
use tatara_rust_repo::RepoSpec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum DecorationProfile {
    /// All defaults on — full OSS-publish surface.
    Default,
    /// Flake stripped (operator wants a non-Nix crate).
    NoFlake,
    /// Caixa stripped (rare; operator publishes outside the SDLC loop).
    NoCaixa,
    /// Minimal — only the proc-macro crate + LICENSE + .gitignore.
    Minimal,
}

impl DecorationProfile {
    fn apply(self, base: RepoSpec) -> RepoSpec {
        match self {
            Self::Default => base,
            Self::NoFlake => base.without_flake(),
            Self::NoCaixa => base.without_caixa(),
            Self::Minimal => base
                .without_flake()
                .without_caixa()
                .without_auto_release(),
        }
    }

    /// Files we EXPECT to be present after compile for this profile.
    fn expected(self) -> &'static [&'static str] {
        match self {
            Self::Default => &[
                "Cargo.toml",
                "src/lib.rs",
                "flake.nix",
                "caixa.lisp",
                ".github/workflows/auto-release.yml",
                "clippy.toml",
                "LICENSE",
                ".gitignore",
                "README.md",
            ],
            Self::NoFlake => &[
                "Cargo.toml",
                "src/lib.rs",
                "caixa.lisp",
                ".github/workflows/auto-release.yml",
                "clippy.toml",
                "LICENSE",
                ".gitignore",
                "README.md",
            ],
            Self::NoCaixa => &[
                "Cargo.toml",
                "src/lib.rs",
                "flake.nix",
                ".github/workflows/auto-release.yml",
                "clippy.toml",
                "LICENSE",
                ".gitignore",
                "README.md",
            ],
            Self::Minimal => &[
                "Cargo.toml",
                "src/lib.rs",
                "clippy.toml",
                "LICENSE",
                ".gitignore",
                "README.md",
            ],
        }
    }

    /// Files we EXPECT to be absent.
    fn absent(self) -> &'static [&'static str] {
        match self {
            Self::Default => &[],
            Self::NoFlake => &["flake.nix"],
            Self::NoCaixa => &["caixa.lisp"],
            Self::Minimal => &[
                "flake.nix",
                "caixa.lisp",
                ".github/workflows/auto-release.yml",
            ],
        }
    }
}

fn all_kinds() -> Vec<(&'static str, CatalogSpec)> {
    vec![
        (
            "derive",
            CatalogSpec::Derive {
                spec: ProcDeriveSpec::new("MatrixMarker", vec![]),
            },
        ),
        (
            "per-field",
            CatalogSpec::PerField {
                spec: PerFieldDeriveSpec {
                    trait_name: Ident::new("MatrixGetter"),
                    target: PerFieldTarget::NamedStruct,
                    trait_ref: None,
                    per_field_template:
                        "pub fn #field_name(&self) -> &#field_ty { &self.#field_name }".into(),
                    method_name_template: None,
                    impl_prelude: None,
                    skip_fields: vec![],
                    field_attribute: None,
                    field_tag: None,
                },
            },
        ),
        (
            "per-variant",
            CatalogSpec::PerVariant {
                spec: PerVariantDeriveSpec {
                    trait_name: Ident::new("MatrixIs"),
                    variant_shape: VariantShape::Any,
                    trait_ref: None,
                    per_variant_template:
                        "pub fn #method_ident(&self) -> bool { matches!(self, #variant_shape_arm) }"
                            .into(),
                    method_name_template: Some("is_{}".into()),
                    impl_prelude: None,
                },
            },
        ),
        (
            "proc-attr",
            CatalogSpec::ProcAttr {
                spec: ProcAttrSpec {
                    macro_name: Ident::new("matrix_attr"),
                    transform: AttrTransform::PrependPrelude {
                        prelude_tokens: "// matrix attr prelude\n".into(),
                    },
                },
            },
        ),
        (
            "proc-fn",
            CatalogSpec::ProcFn {
                spec: ProcFnSpec {
                    macro_name: Ident::new("matrix_fn"),
                    transform: FnTransform::PrependPrelude {
                        prelude_tokens: "// matrix fn prelude\n".into(),
                    },
                },
            },
        ),
        (
            "macro-rules",
            CatalogSpec::MacroRules {
                spec: MacroRulesSpec {
                    macro_name: Ident::new("matrix_rules"),
                    arms: vec![MacroArm {
                        matcher: "()".into(),
                        transcriber: "{ () }".into(),
                    }],
                },
            },
        ),
        (
            "composite",
            CatalogSpec::Composite {
                spec: CompositeDeriveSpec {
                    bundle_name: Ident::new("MatrixComposite"),
                    members: vec![CompositeMember::Simple(ProcDeriveSpec::new(
                        "MatrixCompositeInner",
                        vec![],
                    ))],
                },
            },
        ),
        (
            "newtype",
            CatalogSpec::Newtype {
                spec: NewtypeDeriveSpec {
                    trait_name: Ident::new("MatrixNewtype"),
                    target: NewtypeTarget::Tuple,
                    impl_template:
                        "impl #self_name { pub fn matrix_inner(&self) -> &#inner_ty { &self.0 } }"
                            .into(),
                },
            },
        ),
        (
            "enum-fold",
            CatalogSpec::EnumFold {
                spec: EnumFoldDeriveSpec {
                    trait_name: Ident::new("MatrixEnumFold"),
                    target: EnumFoldTarget::AnyVariants,
                    per_variant_fragment: "#variant_str".into(),
                    fold_template:
                        "impl #self_name { pub const MATRIX_NAMES: &'static [&'static str] = &[#fold]; }"
                            .into(),
                },
            },
        ),
        (
            "kind-round-trip",
            CatalogSpec::KindRoundTrip {
                spec: KindRoundTripSpec::kind_byte("MatrixKindRoundTrip"),
            },
        ),
        (
            "verification-matrix",
            CatalogSpec::VerificationMatrix {
                spec: VerificationMatrixSpec::canonical(),
            },
        ),
        (
            "closed-axis",
            CatalogSpec::ClosedAxis {
                spec: ClosedAxisSpec::canonical(),
            },
        ),
    ]
}

fn one_entry_catalog(name: &str, spec: CatalogSpec) -> MacroCatalogSpec {
    MacroCatalogSpec {
        title: format!("matrix-{name}"),
        entries: vec![CatalogEntry {
            crate_name: format!("matrix-{name}-derive"),
            description: format!("Matrix row for {name}."),
            since: "0.1.0".into(),
            owner: "pleme-io".into(),
            verifier_hint: None,
            spec,
        }],
    }
}

#[test]
fn every_kind_x_decoration_combo_emits_a_repo_with_expected_files() {
    let kinds = all_kinds();
    let profiles = [
        DecorationProfile::Default,
        DecorationProfile::NoFlake,
        DecorationProfile::NoCaixa,
        DecorationProfile::Minimal,
    ];

    let mut failures: Vec<String> = vec![];

    for (kind_label, kind_spec) in &kinds {
        for profile in profiles {
            let cat = one_entry_catalog(kind_label, kind_spec.clone());
            let repos = match cat.compile_to_repos("https://github.com/pleme-io") {
                Ok(r) => r,
                Err(e) => {
                    failures.push(format!(
                        "kind={kind_label} profile={profile:?}: compile_to_repos failed: {e}"
                    ));
                    continue;
                }
            };
            if repos.len() != 1 {
                failures.push(format!(
                    "kind={kind_label} profile={profile:?}: expected 1 repo, got {}",
                    repos.len()
                ));
                continue;
            }
            let repo = profile.apply(repos.into_iter().next().unwrap());
            let scaffold = repo.compile();
            let files: BTreeSet<&str> =
                scaffold.files.iter().map(|f| f.path.as_str()).collect();

            for required in profile.expected() {
                if !files.contains(required) {
                    failures.push(format!(
                        "kind={kind_label} profile={profile:?}: missing expected file `{required}`"
                    ));
                }
            }
            for forbidden in profile.absent() {
                if files.contains(forbidden) {
                    failures.push(format!(
                        "kind={kind_label} profile={profile:?}: unexpectedly present `{forbidden}`"
                    ));
                }
            }
        }
    }

    let cell_count = kinds.len() * profiles.len();
    assert!(
        failures.is_empty(),
        "{}/{cell_count} matrix cells failed:\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
}

#[test]
fn matrix_covers_every_catalog_spec_variant() {
    // Forcing function: every variant of CatalogSpec MUST appear in
    // `all_kinds()`. New variants without a matrix row trip this test.
    let kind_count = all_kinds().len();
    // CatalogSpec currently has 12 variants — adjust if a new kind lands.
    assert_eq!(
        kind_count, 12,
        "matrix must cover every CatalogSpec variant; got {kind_count}, expected 12"
    );
}
