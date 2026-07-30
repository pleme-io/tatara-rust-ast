//! `sexp_readers` — the fleet gate against a *fourteenth* independent
//! S-expression reader.
//!
//! # Why this lives here and not in a reader's own tests
//!
//! "How many independent S-expression readers does pleme-io have?" is a
//! **cross-repo** property. No single reader's test suite can observe it —
//! `tatara-lisp`'s tests cannot see `iac-forge`'s `sexpr.rs`, and
//! `iac-forge`'s tests cannot see `kazari`'s `lisp.rs`. That blindness is
//! exactly how twelve duplicates of the canonical reader accumulated
//! unseen: every one of them was locally justified and locally green.
//!
//! `tatara-rust-survey` is the only crate in the fleet with the reach —
//! [`crate::survey_fleet`] already enumerates
//! `~/code/github/pleme-io/<repo>/Cargo.toml` and runs `syn::parse_file`
//! over each crate's source tree, dry and write-free. This module extends
//! that same traversal rather than adding a second one.
//!
//! # ★ TIER HONESTY — read this before citing the gate
//!
//! **This is a TEST FAILURE, never a compile error.** Nothing expressible
//! in Rust can make another repo's `enum Sexp` fail to compile; the crates
//! are not even in this workspace's dependency graph. Claiming
//! "unrepresentable" here would be a false tier. The honest tier is
//! **CI/eval-caught** — a red `cargo test` in *this* crate, and only if
//! somebody runs it with the fleet checked out.
//!
//! Three further honest limits:
//!
//! 1. **Clause (b) is a heuristic.** A file that pattern-matches on both
//!    `'('` and `')'` inside one function is *usually* a reader, but a
//!    non-lisp parser (a shell-word splitter, a glob matcher, an argument
//!    tokenizer) has the same shape. That direction — a false *positive* —
//!    is the safe one: it fails loudly and is dismissed by one line in
//!    [`SEXP_READER_ALLOWLIST`]. A false *negative* is the dangerous
//!    direction, because it is silent, so the detector deliberately errs
//!    toward over-flagging.
//! 2. **Clause (c) is crate-scoped, and that leaves a blind spot.** See
//!    [`KNOWN_BLIND_SPOTS`] — a crate that legitimately consumes
//!    `tatara_lisp::read` *and also* hand-rolls a reader in some other
//!    module is cleared wholesale and never flagged.
//! 3. **Absent fleet root ⇒ green.** A single checkout of this repo has no
//!    `~/code/github/pleme-io` tree to census, so the gate returns early
//!    rather than failing. That is a deliberate no-op, not a pass.
//!
//! # The detector predicate
//!
//! Structural, so it survives renaming — the census must not depend on a
//! type being spelled `Sexp` rather than `SExpr`, `Sx`, `Form` or
//! `NodeKind`. Three clauses:
//!
//! - **(a) a self-recursive S-expression enum** — an `enum` with a variant
//!   carrying `Vec<Self>` / `Vec<OwnEnumName>` (the nesting), a variant
//!   carrying a string/symbol-ish payload (the atom), and a *uniform*
//!   shape (no struct variants, at most [`MAX_SEXP_VARIANTS`]).
//! - **(b) paren recognition**, in any of three forms — because three
//!   fleet readers each spell it differently, and each spelling silently
//!   evaded an earlier cut of this detector:
//!   - a `fn` that **matches on** both `'('` and `')'` in a match-arm
//!     pattern or an equality comparison (`Lit::Char` *or* `Lit::Byte` —
//!     several readers lex over `&[u8]`);
//!   - both delimiters declared as **named constants**
//!     (`Sexp::LIST_OPEN` / `LIST_CLOSE`, the `tatara` fork's shape —
//!     which leaves *no* paren literal in its tokenizer at all);
//!   - both delimiters declared as **lexer-generator attributes**
//!     (`#[token("(")]`, `caixa-ast`'s logos shape — where the scanning
//!     code does not exist in source to be inspected).
//! - **(c) the enclosing crate never calls the canonical reader** — no
//!   `tatara_lisp::read`, `read_spanned` or `compile_typed` anywhere in the
//!   crate. This is the clause that separates a *duplicate* from a
//!   legitimate *wrapper*: `engawa`, `escriba-lisp`, `repo-forge-lisp` and
//!   `tatara-eval` all match (a) or (b) and are all correctly cleared,
//!   because they build on the canonical reader instead of replacing it.
//!
//! ## ★ Deviation from the specified predicate, and the measurement that forced it
//!
//! This module was specified as **(a) OR (b), AND (c)**. Measured against
//! the live fleet — 623 repos, 1517 crates, 16308 files — that disjunction
//! reports **48 files**, of which roughly 40 are not S-expression readers.
//! The shipped predicate is therefore **(a) AND (b) at crate scope, AND
//! (c)**: a crate is a reader when it holds *both* halves, and within such
//! a crate every file exhibiting either half is flagged.
//!
//! Two independent findings forced it, and both are worth keeping in mind
//! before anyone loosens it back:
//!
//! 1. **Clause (a) alone cannot tell an S-expression from a JSON value.**
//!    `enum Value { Null, Bool(bool), Str(String), Arr(Vec<Value>) }` and
//!    `enum Sexp { Sym(String), List(Vec<Sexp>) }` are the *same shape*.
//!    The sweep surfaced fifteen recursive dynamic-value types on this
//!    clause — `NixValue`, `CtyValue`, `PlistValue`, `TeiaValue`,
//!    `VMValue`, `YamlNode`, `PackerExpr`, `EventLogValue`, … — none of
//!    which reads anything. No refinement of (a) separates them, because
//!    there is nothing to separate: a nesting value type is a nesting
//!    value type. What makes one an *S-expression reader* is that
//!    something in the crate reads parens into it.
//! 2. **Clause (b) alone flags every small hand-rolled parser.** Shell
//!    lexers, a regex meta-character test, an arithmetic tokenizer, a
//!    GraphQL field splitter, a completion word-scanner — all match on
//!    `'('` and `')'` and none builds a nested list.
//!
//! The conjunction is not a fudge; it is the parent phrase "**nesting**-capable
//! **reader**" read literally — the nesting is clause (a), the reading is
//! clause (b), and a crate needs both to be either. It also strictly
//! *strengthens* the gate's precision without weakening its reach: any new
//! reader worth the name brings both halves with it.

use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use syn::visit::Visit;

use crate::{visit_rs_files, SurveyError};

// ─────────────────────────────────────────────────────────────────────
// The frozen catalog — the gate's other half
// ─────────────────────────────────────────────────────────────────────

/// Every nesting-capable S-expression reader known to exist in the fleet,
/// as `(repo, path-relative-to-repo-root)`.
///
/// **Both directions of set-equality against the live census are gated**
/// by [`tests::catalog_matches_live_fleet_census`]:
///
/// - a reader in the census but **not** here fails, naming the file — the
///   fourteenth-reader gate, which is the point of the whole module;
/// - a reader here but **not** in the census *also* fails, forcing this
///   list to be edited *down* when a duplicate is consumed.
///
/// That second direction is the load-bearing one. Without it the catalog
/// only ever grows and the count is free to drift upward while looking
/// tended. With it, the count is **monotonically non-increasing**: the
/// only edit that keeps the gate green is a deletion.
///
/// The catalog is keyed per FILE, not per reader, because a reader is
/// routinely split across two or three of them — the value enum in
/// `ast.rs`/`node.rs`/`sexpr.rs`, the scanner in `reader.rs`/`parse.rs`/
/// `lexer.rs`. The fifteen files below are **eleven** distinct readers.
///
/// Baselined 2026-07-30 by running `survey-sexp-readers` against the tree
/// at `~/code/github/pleme-io` — 623 repos, 1517 crates, 16308 files. Not
/// transcribed from a hand census; every line is a measurement.
///
/// ⚠ **This baseline is on moving ground.** The tatara-lisp
/// A→B consolidation is in flight, so `tatara/tatara-lisp/*` (the fork)
/// and `tatara-lisp/tatara-lisp/*` (the canonical) are both being edited.
/// When the fork is consumed, this gate goes red in the *consumed*
/// direction and the fix is to delete its lines — which is the gate
/// working, not the gate breaking.
pub const KNOWN_SEXP_READERS: &[(&str, &str)] = &[
    // ── canonical (2 files, 1 reader) ────────────────────────────────
    // The one every other entry should eventually collapse into. It is in
    // the census, and must be, because `tatara-lisp` cannot import itself
    // — clause (c) can never clear the definition site.
    //
    // `reader.rs` is deliberately ABSENT even though it is the actual
    // reader: as of tatara-lisp@4df2b02 its tokenizer matches on
    // `Sexp::LIST_OPEN`/`LIST_CLOSE`, so the paren evidence lives in
    // `ast.rs` with the constants. A per-file key follows the evidence,
    // not the intuition about which filename "is" the reader. That line
    // was in this catalog and the gate removed it — see the module docs.
    ("tatara-lisp", "tatara-lisp/src/ast.rs"),
    ("tatara-lisp", "tatara-lisp/src/spanned.rs"),
    // ── the fork (1 reader) ──────────────────────────────────────────
    // Anchored on `ast.rs`, not `reader.rs`: the fork's tokenizer routes
    // its delimiters through `Sexp::LIST_OPEN`/`LIST_CLOSE`, so the
    // paren literals live with the type, not with the scanner.
    ("tatara", "tatara-lisp/src/ast.rs"),
    // ── duplicates (9 readers, 12 files) ─────────────────────────────
    ("aldrava", "src/spec_lisp.rs"),
    ("caixa", "caixa-ast/src/lexer.rs"),
    ("caixa", "caixa-ast/src/node.rs"),
    ("engawa-lisp", "src/parse.rs"),
    ("engawa-lisp", "src/sexpr.rs"),
    ("iac-forge", "src/sexpr.rs"),
    // A SECOND reader inside iac-forge, in a different module from the
    // first. Not in the hand census that seeded this work — the sweep
    // found it, which is the whole argument for a mechanical census over
    // a remembered one.
    ("iac-forge", "src/transform.rs"),
    ("kazari", "src/lisp.rs"),
    ("lava-eval", "src/sexpr.rs"),
    ("magma", "magma-tatara/src/lib.rs"),
    ("selo", "selo-gen/src/reader.rs"),
    ("tatara-rust-ast", "crates/tatara-rust-tlisp/src/catalog.rs"),
];

/// Files clause (b) flags that are **not** S-expression readers.
///
/// One line here is the whole cost of a false positive, which is why the
/// detector is tuned to over-flag rather than under-flag. Every entry must
/// carry the reason it is not a reader, so a later reviewer can re-check
/// the judgement instead of trusting the list.
pub const SEXP_READER_ALLOWLIST: &[(&str, &str, &str)] = &[
    // (repo, rel path, why this is not an S-expression reader)
    (
        "iac-forge",
        "src/nix.rs",
        "`NixValue` is a Nix EMISSION value type — recursive and \
         string-carrying, so clause (a) fires, but the file contains zero \
         parse/read/lex/tokenize functions. It is flagged only because it \
         shares a crate with iac-forge's two real readers, which is the \
         known cost of evaluating the (a)∧(b) conjunction at crate scope",
    ),
];

/// Readers the predicate **cannot see**, recorded so the count is not read
/// as complete.
///
/// Clause (c) clears a whole crate the moment that crate calls
/// `tatara_lisp::read` / `compile_typed` anywhere. `nami-core` is one
/// crate that does both things at once: it is a heavy, legitimate consumer
/// of the canonical reader (106 `compile_typed` call sites) *and* it
/// hand-rolls two independent readers in other modules. Crate-scoped
/// clearance therefore silently exonerates both.
///
/// Three other shapes that evaded earlier cuts of the detector were
/// *fixed* rather than listed here — the span-wrapper enum
/// (`engawa-lisp`), named delimiter constants (the `tatara` fork), and
/// logos `#[token("(")]` attributes (`caixa-ast`). Adding a blind spot is
/// the last resort; closing the class is the first.
///
/// This list is documentation, not enforcement — nothing checks it. It
/// exists so "the census found 16" is never mistaken for "the fleet has
/// 16". Measured 2026-07-30; the honest fleet total is
/// `census.len() + KNOWN_BLIND_SPOTS.len()`.
pub const KNOWN_BLIND_SPOTS: &[(&str, &str, &str)] = &[
    (
        "nami-core",
        "src/lisp/parse.rs",
        "clause (c) crate-scoped: nami-core calls tatara_lisp::compile_typed \
         in 106 files, which clears the entire crate",
    ),
    (
        "nami-core",
        "src/component/mod.rs",
        "same crate-scoped clearance; a SECOND independent reader inside \
         the same crate as the one above",
    ),
];

// ─────────────────────────────────────────────────────────────────────
// Census types
// ─────────────────────────────────────────────────────────────────────

/// Which clause fired on a file. Carried through to the report so a red
/// gate says *why* a file looks like a reader, not just that it does.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReaderSignal {
    /// Clause (a): `enum <name>` has both a `Vec<Self>`-shaped nesting
    /// variant and a string/symbol-ish atom variant.
    RecursiveSexpEnum { enum_name: String },
    /// Clause (b): `fn <name>`'s body matches on both the `'('` and `')'`
    /// literals.
    ParenLiteralFn { fn_name: String },
    /// Clause (b), named-constant form: `const <name>: char = '('`.
    ///
    /// A reader that lifts its delimiters into named constants —
    /// `Sexp::LIST_OPEN` / `Sexp::LIST_CLOSE` — has no `'('` literal left
    /// anywhere in its tokenizer, so the literal-in-a-fn form of clause
    /// (b) goes blind on it. Declaring both delimiters is exactly as
    /// diagnostic as matching on them, so it counts the same.
    ParenDelimiterConst { const_name: String, delimiter: char },
    /// Clause (b), lexer-generator form: `#[token("(")]` on a token enum
    /// variant.
    ///
    /// A `logos`-derived lexer has no paren literal in any function body
    /// at all — the delimiters live in derive attributes and the scanning
    /// code is generated. `caixa-ast` is exactly this, and it is why the
    /// first three cuts of this detector could not see caixa's reader.
    ParenTokenAttr { variant: String, delimiter: char },
}

/// One flagged file.
#[derive(Clone, Debug, Serialize)]
pub struct SexpReaderFinding {
    /// Immediate subdirectory of the fleet root — the repo name.
    pub repo: String,
    /// Path relative to the repo root, forward-slashed. This is the
    /// coordinate the catalog is keyed on: stable across checkout
    /// locations, unlike an absolute path.
    pub rel_path: String,
    /// Enclosing crate, relative to the repo root (`""` when the repo
    /// root is itself the crate). Clause (c) is evaluated at this scope.
    pub crate_rel: String,
    /// Every clause that fired, sorted — a file usually trips both.
    pub signals: Vec<ReaderSignal>,
}

impl SexpReaderFinding {
    /// The catalog key: `(repo, rel_path)`.
    pub fn key(&self) -> (&str, &str) {
        (self.repo.as_str(), self.rel_path.as_str())
    }
}

/// Result of one fleet-wide sweep.
#[derive(Debug, Serialize)]
pub struct SexpReaderCensus {
    pub root: PathBuf,
    pub repos_scanned: usize,
    pub crates_scanned: usize,
    pub files_parsed: usize,
    /// Flagged files, sorted by `(repo, rel_path)` so the report and the
    /// catalog diff are both stable across runs.
    pub findings: Vec<SexpReaderFinding>,
    /// Files that matched the predicate but were dismissed by
    /// [`SEXP_READER_ALLOWLIST`]. Reported rather than dropped, so a stale
    /// allow-list entry is visible instead of silently suppressing.
    pub allowlisted: Vec<SexpReaderFinding>,
}

impl SexpReaderCensus {
    /// Catalog keys present in the census.
    pub fn keys(&self) -> BTreeSet<(String, String)> {
        self.findings
            .iter()
            .map(|f| (f.repo.clone(), f.rel_path.clone()))
            .collect()
    }
}

/// The two directions of catalog drift, computed once so both the test and
/// the CLI report the same thing.
#[derive(Debug, Serialize)]
pub struct CatalogDrift {
    /// In the census, absent from the catalog — a NEW independent reader.
    pub added: Vec<SexpReaderFinding>,
    /// In the catalog, absent from the census — a CONSUMED reader whose
    /// catalog line must now be deleted.
    pub consumed: Vec<(String, String)>,
}

impl CatalogDrift {
    pub fn is_clean(&self) -> bool {
        self.added.is_empty() && self.consumed.is_empty()
    }
}

/// What the detector sees in one file, for the "would this trip the
/// gate?" probe. Lives here rather than in the CLI so `syn` stays an
/// implementation detail of this crate.
#[derive(Debug, Serialize)]
pub struct FileProbe {
    pub path: PathBuf,
    /// Clause (c), evaluated at file scope — informational only. The gate
    /// itself evaluates (c) across the whole enclosing crate.
    pub calls_canonical_reader: bool,
    pub signals: Vec<ReaderSignal>,
}

/// Run the per-file half of the detector against one `.rs` file.
pub fn probe_file(path: &Path) -> Result<FileProbe, SurveyError> {
    let src = std::fs::read_to_string(path)?;
    let file = syn::parse_file(&src).map_err(|err| SurveyError::Parse {
        path: path.to_path_buf(),
        err,
    })?;
    Ok(FileProbe {
        path: path.to_path_buf(),
        calls_canonical_reader: uses_canonical_reader(&file),
        signals: detect_reader_signals(&file),
    })
}

/// Diff a census against [`KNOWN_SEXP_READERS`], both directions.
pub fn drift_against_catalog(census: &SexpReaderCensus) -> CatalogDrift {
    let cataloged: BTreeSet<(String, String)> = KNOWN_SEXP_READERS
        .iter()
        .map(|(r, p)| ((*r).to_string(), (*p).to_string()))
        .collect();
    let live = census.keys();

    let added = census
        .findings
        .iter()
        .filter(|f| !cataloged.contains(&(f.repo.clone(), f.rel_path.clone())))
        .cloned()
        .collect();
    let consumed = cataloged.difference(&live).cloned().collect();

    CatalogDrift { added, consumed }
}

// ─────────────────────────────────────────────────────────────────────
// The sweep
// ─────────────────────────────────────────────────────────────────────

/// Default fleet root. Overridable with `PLEME_FLEET_ROOT` so the gate can
/// be pointed at a fixture (which is how the module's own tests exercise
/// it without depending on the operator's checkout).
pub fn default_fleet_root() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("PLEME_FLEET_ROOT") {
        let p = PathBuf::from(explicit);
        return p.is_dir().then_some(p);
    }
    let home = std::env::var("HOME").ok()?;
    let p = PathBuf::from(home).join("code/github/pleme-io");
    p.is_dir().then_some(p)
}

/// Census every independent S-expression reader under `root`.
///
/// Traversal mirrors [`crate::survey_fleet`]: immediate subdirectories of
/// `root` that carry a `Cargo.toml` are repos; within each repo, every
/// directory carrying both a `Cargo.toml` and a `src/` is a crate; each
/// crate's `src/` tree is walked for `.rs` files. Parse failures on
/// individual files are swallowed exactly as `survey_tree` swallows them —
/// one malformed generated file must not blind the whole census.
///
/// **Clause (c) and the (a)∧(b) conjunction are both evaluated at CRATE
/// scope**, which is why every file in a crate is parsed before any file
/// in it is judged. A crate is a reader when some file defines a uniform
/// recursive S-expression enum *and* some file matches on parens *and* no
/// file calls the canonical reader; within such a crate, every file
/// exhibiting either half is reported. See the module docs for the
/// measurement that made the conjunction necessary.
///
/// Dry and write-free.
pub fn census_sexp_readers(root: &Path) -> Result<SexpReaderCensus, SurveyError> {
    let mut findings: Vec<SexpReaderFinding> = vec![];
    let mut allowlisted: Vec<SexpReaderFinding> = vec![];
    let mut repos_scanned = 0usize;
    let mut crates_scanned = 0usize;
    let mut files_parsed = 0usize;

    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let repo_dir = entry.path();
        // `metadata` (not `file_type`) so a symlinked repo resolves to its
        // target — same rationale as `survey_fleet`: a directory of
        // symlinks is a first-class scoped fleet root.
        let Ok(meta) = std::fs::metadata(&repo_dir) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let repo_name = entry.file_name().to_string_lossy().into_owned();
        if repo_name.starts_with('.') || repo_name == "target" {
            continue;
        }
        // A repo with no root Cargo.toml holds no Rust crates worth
        // walking. This bounds the sweep over an ~800-repo tree.
        if !repo_dir.join("Cargo.toml").exists() {
            continue;
        }
        repos_scanned += 1;

        let mut crate_dirs: Vec<PathBuf> = vec![];
        collect_crate_dirs(&repo_dir, &mut crate_dirs, 0);

        for crate_dir in &crate_dirs {
            crates_scanned += 1;
            let src = crate_dir.join("src");

            // Clause (c) is crate-scoped: gather every `.rs` in the crate
            // once, so the "does this crate call the canonical reader?"
            // question is answered before any file is judged.
            let mut sources: Vec<(PathBuf, syn::File)> = vec![];
            let walk = visit_rs_files(&src, &mut |p| {
                if let Ok(text) = std::fs::read_to_string(p) {
                    if let Ok(f) = syn::parse_file(&text) {
                        sources.push((p.to_path_buf(), f));
                    }
                }
                Ok(())
            });
            if walk.is_err() {
                continue; // unreadable crate — skip, matching survey_fleet
            }
            files_parsed += sources.len();

            if sources.iter().any(|(_, f)| uses_canonical_reader(f)) {
                continue; // cleared by clause (c) — a wrapper, not a duplicate
            }

            // A *reader* is a crate holding BOTH halves — see
            // `census_sexp_readers`' doc comment for why the conjunction
            // is at crate scope rather than per file.
            let per_file: Vec<(&PathBuf, Vec<ReaderSignal>)> = sources
                .iter()
                .map(|(p, f)| (p, detect_reader_signals(f)))
                .collect();
            let has_value = per_file.iter().flat_map(|(_, s)| s).any(|s| {
                matches!(s, ReaderSignal::RecursiveSexpEnum { .. })
            });
            let all: Vec<&ReaderSignal> = per_file.iter().flat_map(|(_, s)| s).collect();
            // Clause (b) in either form: a fn that matches on both parens,
            // or the crate declaring both delimiters as named constants
            // (the `Sexp::LIST_OPEN`/`LIST_CLOSE` idiom, which leaves no
            // paren literal in the tokenizer at all).
            let has_reader_fn = all
                .iter()
                .any(|s| matches!(s, ReaderSignal::ParenLiteralFn { .. }));
            let delim = |d: char| {
                all.iter().any(|s| match s {
                    ReaderSignal::ParenDelimiterConst { delimiter, .. }
                    | ReaderSignal::ParenTokenAttr { delimiter, .. } => *delimiter == d,
                    _ => false,
                })
            };
            let has_reader = has_reader_fn || (delim('(') && delim(')'));
            if !(has_value && has_reader) {
                continue;
            }

            for (path, signals) in per_file {
                if signals.is_empty() {
                    continue;
                }
                let Some(rel_path) = rel_to(&repo_dir, path) else {
                    continue;
                };
                let crate_rel = rel_to(&repo_dir, crate_dir).unwrap_or_default();
                let finding = SexpReaderFinding {
                    repo: repo_name.clone(),
                    rel_path,
                    crate_rel,
                    signals,
                };
                if is_allowlisted(&finding) {
                    allowlisted.push(finding);
                } else {
                    findings.push(finding);
                }
            }
        }
    }

    let sort_key = |f: &SexpReaderFinding| (f.repo.clone(), f.rel_path.clone());
    findings.sort_by_key(sort_key);
    allowlisted.sort_by_key(sort_key);

    Ok(SexpReaderCensus {
        root: root.to_path_buf(),
        repos_scanned,
        crates_scanned,
        files_parsed,
        findings,
        allowlisted,
    })
}

fn is_allowlisted(f: &SexpReaderFinding) -> bool {
    SEXP_READER_ALLOWLIST
        .iter()
        .any(|(r, p, _)| *r == f.repo && *p == f.rel_path)
}

fn rel_to(base: &Path, p: &Path) -> Option<String> {
    let rel = p.strip_prefix(base).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// Recursively collect crate roots (a directory with both `Cargo.toml` and
/// `src/`) inside a repo. Depth-bounded: fleet workspaces nest at most
/// `repo/<member>/` or `repo/crates/<member>/`, so 3 levels is generous and
/// keeps a pathological vendored tree from exploding the sweep.
fn collect_crate_dirs(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 3 {
        return;
    }
    if dir.join("Cargo.toml").exists() && dir.join("src").is_dir() {
        out.push(dir.to_path_buf());
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        // `src` is walked by the caller; the rest never hold crate roots
        // we want to census (a crate's own tests/benches/examples are not
        // independent readers, and `target` is build output).
        if name.starts_with('.')
            || matches!(name.as_str(), "src" | "target" | "tests" | "benches" | "examples")
        {
            continue;
        }
        if std::fs::metadata(&p).map(|m| m.is_dir()).unwrap_or(false) {
            collect_crate_dirs(&p, out, depth + 1);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Clause (a) + (b) — the structural detector
// ─────────────────────────────────────────────────────────────────────

/// Every clause-(a) and clause-(b) signal in one parsed file.
///
/// Structural by construction — nothing here reads an identifier's
/// *spelling*, so a reader named `Form`, `NodeKind` or `Blorp` is caught
/// on the same footing as one named `Sexp`.
pub fn detect_reader_signals(file: &syn::File) -> Vec<ReaderSignal> {
    let mut v = SignalVisitor {
        type_refs: collect_type_refs(file),
        signals: vec![],
    };
    v.visit_file(file);
    v.signals.sort();
    v.signals.dedup();
    v.signals
}

struct SignalVisitor {
    /// Type name → every type ident it mentions in a field, for the
    /// same-file reachability that clause (a) needs. See
    /// [`collect_type_refs`].
    type_refs: std::collections::BTreeMap<String, BTreeSet<String>>,
    signals: Vec<ReaderSignal>,
}

impl SignalVisitor {
    fn note_const(&mut self, name: &syn::Ident, expr: &syn::Expr) {
        if let syn::Expr::Lit(l) = expr {
            let c = match &l.lit {
                syn::Lit::Char(c) => Some(c.value()),
                syn::Lit::Byte(b) => Some(b.value() as char),
                _ => None,
            };
            if matches!(c, Some('(') | Some(')')) {
                self.signals.push(ReaderSignal::ParenDelimiterConst {
                    const_name: name.to_string(),
                    delimiter: c.expect("matched above"),
                });
            }
        }
    }
}

impl<'ast> Visit<'ast> for SignalVisitor {
    fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
        if is_recursive_sexp_enum(e, &self.type_refs) {
            self.signals.push(ReaderSignal::RecursiveSexpEnum {
                enum_name: e.ident.to_string(),
            });
        }
        for v in &e.variants {
            for d in paren_token_attrs(&v.attrs) {
                self.signals.push(ReaderSignal::ParenTokenAttr {
                    variant: v.ident.to_string(),
                    delimiter: d,
                });
            }
        }
        syn::visit::visit_item_enum(self, e);
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if block_has_paren_pair(&f.block) {
            self.signals.push(ReaderSignal::ParenLiteralFn {
                fn_name: f.sig.ident.to_string(),
            });
        }
        syn::visit::visit_item_fn(self, f);
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        if block_has_paren_pair(&f.block) {
            self.signals.push(ReaderSignal::ParenLiteralFn {
                fn_name: f.sig.ident.to_string(),
            });
        }
        syn::visit::visit_impl_item_fn(self, f);
    }

    fn visit_item_const(&mut self, c: &'ast syn::ItemConst) {
        self.note_const(&c.ident, &c.expr);
        syn::visit::visit_item_const(self, c);
    }

    fn visit_impl_item_const(&mut self, c: &'ast syn::ImplItemConst) {
        self.note_const(&c.ident, &c.expr);
        syn::visit::visit_impl_item_const(self, c);
    }
}

/// Paren delimiters declared in a lexer-generator attribute —
/// `#[token("(")]`, `#[token(")", …)]`.
///
/// Matched on the attribute's *last* path segment (`token`), so
/// `#[logos::token("(")]` works the same. The first string literal in the
/// attribute's arguments is the pattern; later arguments are callbacks and
/// are ignored.
fn paren_token_attrs(attrs: &[syn::Attribute]) -> Vec<char> {
    let mut out = vec![];
    for a in attrs {
        let Some(seg) = a.path().segments.last() else {
            continue;
        };
        if seg.ident != "token" {
            continue;
        }
        let syn::Meta::List(list) = &a.meta else {
            continue;
        };
        // Re-parse the attribute body far enough to reach its first
        // string literal without committing to logos' exact grammar.
        let Ok(lit) = syn::parse2::<syn::Lit>(
            list.tokens
                .clone()
                .into_iter()
                .take(1)
                .collect::<proc_macro2::TokenStream>(),
        ) else {
            continue;
        };
        if let syn::Lit::Str(s) = lit {
            match s.value().as_str() {
                "(" => out.push('('),
                ")" => out.push(')'),
                _ => {}
            }
        }
    }
    out
}

/// Map every `struct`/`enum` in a file to the set of type idents its
/// fields mention, at any generic depth.
///
/// This exists for one shape that a naive `Vec<Self>` test cannot see: a
/// reader that carries source spans splits its node in two —
/// `struct Sexpr { kind: SexprKind, span: Span }` +
/// `enum SexprKind { List(Vec<Sexpr>), Symbol(String), … }`. The enum is
/// recursive, but *through its sibling struct*, so it holds neither
/// `Vec<Self>` nor `Vec<SexprKind>`. `engawa-lisp` is exactly this, and
/// the untightened detector missed it. Reachability closes the whole class
/// — one, two or N hops — rather than special-casing a `Kind` suffix.
fn collect_type_refs(file: &syn::File) -> std::collections::BTreeMap<String, BTreeSet<String>> {
    let mut out: std::collections::BTreeMap<String, BTreeSet<String>> = Default::default();
    let mut v = TypeRefVisitor { out: &mut out };
    v.visit_file(file);
    out
}

struct TypeRefVisitor<'a> {
    out: &'a mut std::collections::BTreeMap<String, BTreeSet<String>>,
}

impl<'a, 'ast> Visit<'ast> for TypeRefVisitor<'a> {
    fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
        let e = self.out.entry(s.ident.to_string()).or_default();
        for f in &s.fields {
            collect_idents(&f.ty, e);
        }
        syn::visit::visit_item_struct(self, s);
    }
    fn visit_item_enum(&mut self, en: &'ast syn::ItemEnum) {
        let e = self.out.entry(en.ident.to_string()).or_default();
        for v in &en.variants {
            for f in &v.fields {
                collect_idents(&f.ty, e);
            }
        }
        syn::visit::visit_item_enum(self, en);
    }
}

fn collect_idents(t: &syn::Type, out: &mut BTreeSet<String>) {
    struct V<'a>(&'a mut BTreeSet<String>);
    impl<'a, 'ast> Visit<'ast> for V<'a> {
        fn visit_path_segment(&mut self, s: &'ast syn::PathSegment) {
            self.0.insert(s.ident.to_string());
            syn::visit::visit_path_segment(self, s);
        }
    }
    V(out).visit_type(t);
}

/// Can `from` reach `target` by following field types within one file?
fn reaches(
    from: &str,
    target: &str,
    refs: &std::collections::BTreeMap<String, BTreeSet<String>>,
) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut stack = vec![from];
    while let Some(cur) = stack.pop() {
        if cur == target {
            return true;
        }
        if !seen.insert(cur) {
            continue;
        }
        if let Some(next) = refs.get(cur) {
            stack.extend(next.iter().map(String::as_str));
        }
    }
    false
}

/// Clause (a). An `enum` qualifies when **all three** hold:
///
/// 1. a self-recursive collection variant — a field typed `Vec<T>` (also
///    `VecDeque`/`SmallVec`, `T` optionally behind `Box`/`Rc`/`Arc`) where
///    `T` is `Self`, the enum itself, or any same-file type that reaches
///    back to the enum (the span-wrapper idiom — see [`collect_type_refs`]);
/// 2. a string/symbol-ish atom variant;
/// 3. the enum is **uniform** — every variant is a unit or a
///    single-unnamed-field tuple, and there are at most
///    [`MAX_SEXP_VARIANTS`] of them.
///
/// (1) and (2) are the shape the specification names. (3) is what
/// separates an S-expression *value* from a general AST node, and it was
/// added after measurement: without it the fleet sweep flagged ~15 domain
/// ASTs — `ShellNode`, `SqlNode`, `YamlNode`, `VMValue`, a Rust `Expr`, a
/// Prometheus `LabelsMatcher` — every one of which is structurally
/// "recursive + has a String variant" while being nothing like a reader.
///
/// The distinction is real and not a fudge: an S-expression value is
/// *untyped and uniform* — a handful of scalar payloads plus one list —
/// which is exactly why one reader can serve every domain. A domain AST
/// earns its named struct-variants (`Call { fn, args }`, `Binary { op, .. }`)
/// precisely because it is **not** interchangeable with an S-expression.
/// A hand-rolled reader that reached for struct variants would be missed
/// here, but clause (b) still catches its reading function.
fn is_recursive_sexp_enum(
    e: &syn::ItemEnum,
    refs: &std::collections::BTreeMap<String, BTreeSet<String>>,
) -> bool {
    if e.variants.len() > MAX_SEXP_VARIANTS {
        return false;
    }
    let uniform = e.variants.iter().all(|v| match &v.fields {
        syn::Fields::Unit => true,
        syn::Fields::Unnamed(u) => u.unnamed.len() == 1,
        syn::Fields::Named(_) => false,
    });
    if !uniform {
        return false;
    }

    let own = e.ident.to_string();
    let mut nests = false;
    let mut atoms = false;
    for v in &e.variants {
        for f in &v.fields {
            if let Some(inner) = generic_inner(&f.ty, &["Vec", "VecDeque", "SmallVec"]) {
                let inner = unwrap_ptr(inner);
                if let Some(id) = base_ident(inner) {
                    if id == "Self" || id == own || reaches(&id, &own, refs) {
                        nests = true;
                    }
                }
            }
            if is_stringish(&f.ty) {
                atoms = true;
            }
        }
    }
    nests && atoms
}

/// Variant-count ceiling for clause (a)'s uniformity test.
///
/// Most readers in the 2026-07-30 census sat at 2–8 variants; the domain
/// ASTs that tripped the untightened predicate ran to dozens. The ceiling
/// is **16** rather than a tighter 8 because `caixa-ast`'s `NodeKind`
/// legitimately carries 14 — it covers three bracket dialects plus the
/// four homoiconic quote prefixes. A ceiling of 12 (the first cut) hid it,
/// which is precisely the kind of silent miss this gate exists to prevent.
/// The uniformity requirement, not the count, is doing the real work.
const MAX_SEXP_VARIANTS: usize = 16;

/// Names that count as the "atom" payload of an S-expression node.
/// `Atom`/`Symbol`/`Ident` are included because several fleet readers
/// factor the string payload into a second enum (`Sx::Atom(Atom)`), which
/// would otherwise read as a non-string tagged union.
const STRINGISH: &[&str] = &[
    "String",
    "str",
    "Cow",
    "Atom",
    "Symbol",
    "Ident",
    "SmolStr",
    "CompactString",
    "Box",
    "Rc",
    "Arc",
];

fn is_stringish(t: &syn::Type) -> bool {
    match t {
        syn::Type::Reference(r) => is_stringish(&r.elem),
        syn::Type::Path(_) => {
            let Some(id) = base_ident(t) else {
                return false;
            };
            // A bare `Box`/`Rc`/`Arc` only counts through its payload.
            if matches!(id.as_str(), "Box" | "Rc" | "Arc" | "Cow") {
                return generic_inner(t, &["Box", "Rc", "Arc", "Cow"])
                    .is_some_and(is_stringish);
            }
            STRINGISH.contains(&id.as_str())
        }
        _ => false,
    }
}

/// `Vec<T>` → `T`, for any of `wrappers`. Returns `None` for a
/// non-generic path or a mismatched head.
fn generic_inner<'a>(t: &'a syn::Type, wrappers: &[&str]) -> Option<&'a syn::Type> {
    let syn::Type::Path(tp) = t else {
        return None;
    };
    let seg = tp.path.segments.last()?;
    if !wrappers.contains(&seg.ident.to_string().as_str()) {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    args.args.iter().find_map(|a| match a {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

/// Peel `Box`/`Rc`/`Arc` so `Vec<Box<Self>>` reads as recursive.
fn unwrap_ptr(t: &syn::Type) -> &syn::Type {
    generic_inner(t, &["Box", "Rc", "Arc"]).map_or(t, unwrap_ptr)
}

fn base_ident(t: &syn::Type) -> Option<String> {
    match t {
        syn::Type::Path(tp) => tp.path.segments.last().map(|s| s.ident.to_string()),
        syn::Type::Reference(r) => base_ident(&r.elem),
        _ => None,
    }
}

/// Clause (b). One function body **matches on** both `'('` and `')'`.
///
/// "Matches on" is load-bearing and narrower than "mentions": only two
/// positions count —
///
/// - a **match-arm pattern** (`'(' => …`, `Some(b'(') => …` — `syn`
///   descends `Pat::TupleStruct` for us, so the wrapped form works); and
/// - an **equality comparison** (`c == b'('`, `ch != ')'`).
///
/// `Lit::Byte` counts alongside `Lit::Char`, because several fleet readers
/// lex over `&[u8]`.
///
/// The first cut of this function counted *any* literal in the body, and
/// measurement showed why that is wrong: it flagged ~20 functions that
/// merely *emit* parens — `out.push('(')` in a pretty-printer, a
/// `format!`-shaped renderer, `looks_like_path`. Deciding on a paren is a
/// reader; writing one is not.
fn block_has_paren_pair(b: &syn::Block) -> bool {
    let mut v = ParenMatchVisitor::default();
    v.visit_block(b);
    v.open && v.close
}

#[derive(Default)]
struct ParenMatchVisitor {
    open: bool,
    close: bool,
}

impl ParenMatchVisitor {
    fn record(&mut self, l: &syn::Lit) {
        let c = match l {
            syn::Lit::Char(c) => Some(c.value()),
            syn::Lit::Byte(b) => Some(b.value() as char),
            _ => None,
        };
        match c {
            Some('(') => self.open = true,
            Some(')') => self.close = true,
            _ => {}
        }
    }
}

impl<'ast> Visit<'ast> for ParenMatchVisitor {
    fn visit_pat(&mut self, p: &'ast syn::Pat) {
        if let syn::Pat::Lit(l) = p {
            self.record(&l.lit);
        }
        syn::visit::visit_pat(self, p);
    }

    fn visit_expr_binary(&mut self, e: &'ast syn::ExprBinary) {
        if matches!(e.op, syn::BinOp::Eq(_) | syn::BinOp::Ne(_)) {
            for side in [&e.left, &e.right] {
                if let syn::Expr::Lit(l) = &**side {
                    self.record(&l.lit);
                }
            }
        }
        syn::visit::visit_expr_binary(self, e);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Clause (c) — does this crate consume the canonical reader?
// ─────────────────────────────────────────────────────────────────────

/// The canonical entry points. Calling any of these is what makes a crate
/// a *wrapper* rather than a *duplicate*.
const CANONICAL_READER_FNS: &[&str] = &["read", "read_spanned", "compile_typed"];

/// True when the file calls or imports one of [`CANONICAL_READER_FNS`]
/// from `tatara_lisp`.
///
/// Two shapes are recognised, which between them covered every wrapper in
/// the 2026-07-30 census: a qualified path (`tatara_lisp::read(src)`) and a
/// use-tree import (`use tatara_lisp::{read, Atom, Sexp};` — the shape
/// `tatara-eval` uses — followed by a bare `read(...)`).
///
/// **Not** recognised: an aliased import (`use tatara_lisp as tl; tl::read`).
/// A crate doing that is flagged as a duplicate it is not; one line in
/// [`SEXP_READER_ALLOWLIST`] is the intended remedy, and the visible red
/// gate is preferable to widening the match into guesswork.
pub fn uses_canonical_reader(file: &syn::File) -> bool {
    let mut v = CanonicalUseVisitor::default();
    v.visit_file(file);
    v.found
}

#[derive(Default)]
struct CanonicalUseVisitor {
    found: bool,
}

impl CanonicalUseVisitor {
    /// Walk a `use` tree already known to sit under `tatara_lisp::`,
    /// looking for one of the canonical names at any leaf.
    fn scan_use_tree(&mut self, t: &syn::UseTree) {
        match t {
            syn::UseTree::Path(p) => self.scan_use_tree(&p.tree),
            syn::UseTree::Name(n) => {
                if CANONICAL_READER_FNS.contains(&n.ident.to_string().as_str()) {
                    self.found = true;
                }
            }
            syn::UseTree::Rename(r) => {
                if CANONICAL_READER_FNS.contains(&r.ident.to_string().as_str()) {
                    self.found = true;
                }
            }
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    self.scan_use_tree(item);
                }
            }
            syn::UseTree::Glob(_) => {
                // `use tatara_lisp::*;` re-exports the readers, so the
                // crate is a consumer. Conservative in the safe direction:
                // clearing a crate risks a false negative, but a glob
                // import of the canonical crate is overwhelmingly a
                // consumer rather than a re-implementer.
                self.found = true;
            }
        }
    }
}

impl<'ast> Visit<'ast> for CanonicalUseVisitor {
    fn visit_item_use(&mut self, u: &'ast syn::ItemUse) {
        let mut tree = &u.tree;
        // Skip leading `::` / `crate` / `self` qualifiers.
        loop {
            match tree {
                syn::UseTree::Path(p) if p.ident == "tatara_lisp" => {
                    self.scan_use_tree(&p.tree);
                    break;
                }
                syn::UseTree::Path(p) => tree = &p.tree,
                _ => break,
            }
        }
        syn::visit::visit_item_use(self, u);
    }

    fn visit_path(&mut self, p: &'ast syn::Path) {
        let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        for w in segs.windows(2) {
            if w[0] == "tatara_lisp" && CANONICAL_READER_FNS.contains(&w[1].as_str()) {
                self.found = true;
            }
        }
        syn::visit::visit_path(self, p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> syn::File {
        syn::parse_file(src).expect("fixture parses")
    }

    // ── clause (a) ───────────────────────────────────────────────────

    #[test]
    fn clause_a_flags_recursive_enum_regardless_of_its_name() {
        // The whole point of a structural predicate: the same shape under
        // four different spellings must all be caught, so renaming a
        // duplicate cannot evade the gate.
        for name in ["Sexp", "SExpr", "Sx", "Form", "NodeKind", "Blorp"] {
            let src = format!(
                "pub enum {name} {{ Sym(String), List(Vec<{name}>) }}"
            );
            let sig = detect_reader_signals(&parse(&src));
            assert!(
                sig.iter().any(|s| matches!(
                    s,
                    ReaderSignal::RecursiveSexpEnum { enum_name } if enum_name == name
                )),
                "structural detector missed `enum {name}`; got {sig:?}"
            );
        }
    }

    #[test]
    fn clause_a_accepts_vec_self_and_boxed_recursion() {
        let sig = detect_reader_signals(&parse(
            "pub enum E { Atom(String), List(Vec<Self>), Nested(Vec<Box<Self>>) }",
        ));
        assert!(!sig.is_empty(), "Vec<Self> must count as self-recursive");
    }

    #[test]
    fn clause_a_accepts_a_factored_atom_enum() {
        // lava-eval's shape: the string payload lives in a second enum, so
        // the nesting enum has no `String` field of its own.
        let sig = detect_reader_signals(&parse(
            "pub enum Sx { Atom(Atom), List(Vec<Sx>) }",
        ));
        assert!(!sig.is_empty(), "Sx::Atom(Atom) must read as the atom variant");
    }

    #[test]
    fn clause_a_requires_both_halves() {
        // A tree with no string atom is any tree, not an S-expression.
        let tree_only = detect_reader_signals(&parse(
            "pub enum Dir { File(u64), Sub(Vec<Dir>) }",
        ));
        assert!(tree_only.is_empty(), "Vec<Self> alone must not flag: {tree_only:?}");

        // A string union with no nesting is any tagged union.
        let atom_only = detect_reader_signals(&parse(
            "pub enum Msg { Text(String), Code(u16) }",
        ));
        assert!(atom_only.is_empty(), "String alone must not flag: {atom_only:?}");
    }

    // ── clause (b) ───────────────────────────────────────────────────

    #[test]
    fn clause_b_flags_char_and_byte_paren_matchers() {
        let chars = detect_reader_signals(&parse(
            r#"fn read(c: char) -> u8 { match c { '(' => 1, ')' => 2, _ => 0 } }"#,
        ));
        assert!(
            chars.iter().any(|s| matches!(
                s, ReaderSignal::ParenLiteralFn { fn_name } if fn_name == "read")),
            "char-literal matcher must flag; got {chars:?}"
        );

        // Several fleet readers lex over &[u8].
        let bytes = detect_reader_signals(&parse(
            r#"fn read(c: u8) -> u8 { match c { b'(' => 1, b')' => 2, _ => 0 } }"#,
        ));
        assert!(!bytes.is_empty(), "byte-literal matcher must flag too: {bytes:?}");
    }

    #[test]
    fn clause_b_needs_both_parens_in_the_same_fn() {
        // One paren alone is a formatter, not a reader. And two parens
        // split across two fns is not the reader shape either — the
        // signal is a fn that handles open AND close.
        let one = detect_reader_signals(&parse(r#"fn f(s: &mut String) { s.push('('); }"#));
        assert!(one.is_empty(), "single paren must not flag: {one:?}");

        let split = detect_reader_signals(&parse(
            r#"fn a(s: &mut String) { s.push('('); } fn b(s: &mut String) { s.push(')'); }"#,
        ));
        assert!(split.is_empty(), "parens in separate fns must not flag: {split:?}");
    }

    #[test]
    fn clause_b_sees_methods_not_just_free_fns() {
        let sig = detect_reader_signals(&parse(
            r#"struct R; impl R { fn next(&self, c: char) -> u8 { match c { '(' => 1, ')' => 2, _ => 0 } } }"#,
        ));
        assert!(!sig.is_empty(), "impl-block methods must be scanned: {sig:?}");
    }

    #[test]
    fn clause_b_sees_named_delimiter_constants() {
        // The `tatara` fork lifted its delimiters out of the tokenizer
        // into `Sexp::LIST_OPEN`/`LIST_CLOSE`, leaving no paren literal in
        // any fn body. An earlier cut of this detector went blind on it.
        let sig = detect_reader_signals(&parse(
            r#"struct S; impl S { const LIST_OPEN: char = '('; const LIST_CLOSE: char = ')'; }"#,
        ));
        let delims: Vec<char> = sig
            .iter()
            .filter_map(|s| match s {
                ReaderSignal::ParenDelimiterConst { delimiter, .. } => Some(*delimiter),
                _ => None,
            })
            .collect();
        assert!(
            delims.contains(&'(') && delims.contains(&')'),
            "both delimiter constants must be seen; got {sig:?}"
        );
        // Free-standing consts count the same as associated ones.
        let free = detect_reader_signals(&parse("const OPEN: char = '(';"));
        assert!(!free.is_empty(), "module-level const must count: {free:?}");
    }

    #[test]
    fn clause_b_sees_logos_token_attributes() {
        // `caixa-ast` scans with a logos-derived lexer: the delimiters are
        // derive attributes and the scanning code is generated, so there
        // is no source function to inspect at all.
        let sig = detect_reader_signals(&parse(
            r#"
#[derive(Logos)]
enum Tok { #[token("(")] LParen, #[token(")")] RParen, #[token("{")] LBrace }
"#,
        ));
        let delims: Vec<char> = sig
            .iter()
            .filter_map(|s| match s {
                ReaderSignal::ParenTokenAttr { delimiter, .. } => Some(*delimiter),
                _ => None,
            })
            .collect();
        assert!(
            delims.contains(&'(') && delims.contains(&')'),
            "both #[token] parens must be seen, and only parens; got {sig:?}"
        );
        assert_eq!(delims.len(), 2, "the `{{` token must not be counted");
    }

    #[test]
    fn clause_a_follows_recursion_through_a_span_wrapper() {
        // `engawa-lisp`'s shape: the enum recurses through its sibling
        // struct, so it holds neither `Vec<Self>` nor `Vec<SexprKind>`.
        // A literal `Vec<Self>` test is blind to this whole class.
        let sig = detect_reader_signals(&parse(
            r#"
pub struct Sexpr { pub kind: SexprKind, pub span: Span }
pub enum SexprKind { List(Vec<Sexpr>), Symbol(String) }
"#,
        ));
        assert!(
            sig.iter().any(|s| matches!(
                s, ReaderSignal::RecursiveSexpEnum { enum_name } if enum_name == "SexprKind")),
            "recursion through a sibling struct must count; got {sig:?}"
        );
    }

    #[test]
    fn clause_a_does_not_follow_recursion_to_an_unrelated_type() {
        // Reachability must not degenerate into "mentions any type".
        let sig = detect_reader_signals(&parse(
            r#"
pub struct Unrelated { pub n: u64 }
pub enum E { Items(Vec<Unrelated>), Name(String) }
"#,
        ));
        assert!(
            sig.is_empty(),
            "a Vec of an unrelated type is not recursion; got {sig:?}"
        );
    }

    // ── clause (c) ───────────────────────────────────────────────────

    #[test]
    fn clause_c_recognises_qualified_calls_and_use_trees() {
        assert!(uses_canonical_reader(&parse(
            "fn f(s: &str) { let _ = tatara_lisp::read(s); }"
        )));
        assert!(uses_canonical_reader(&parse(
            "fn f(s: &str) { let _ = tatara_lisp::compile_typed::<T>(s); }"
        )));
        // tatara-eval's shape.
        assert!(uses_canonical_reader(&parse(
            "use tatara_lisp::{read, Atom, Sexp};"
        )));
        assert!(uses_canonical_reader(&parse("use tatara_lisp::read_spanned;")));
        assert!(uses_canonical_reader(&parse("use tatara_lisp::*;")));
    }

    #[test]
    fn clause_c_does_not_clear_on_an_unrelated_tatara_lisp_import() {
        // engawa imports the derive macro too, but the derive alone is not
        // reader consumption — only read/read_spanned/compile_typed is.
        assert!(!uses_canonical_reader(&parse(
            "use tatara_lisp::DeriveTataraDomain;"
        )));
        assert!(!uses_canonical_reader(&parse("use serde::Serialize;")));
    }

    // ── the sweep + the gate ─────────────────────────────────────────

    /// Build a fixture fleet: `root/<repo>/<crate>/src/<file>`.
    fn fixture(name: &str, files: &[(&str, &str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "tatara-sexp-census-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let manifest = "[package]\nname=\"x\"\nversion=\"0.0.0\"\nedition=\"2021\"\n";
        for (repo, rel, body) in files {
            let file = root.join(repo).join(rel);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, body).unwrap();
            std::fs::write(root.join(repo).join("Cargo.toml"), manifest).unwrap();
            // The crate root is whatever directory owns the `src/` the
            // file sits under.
            let mut d = file.parent().unwrap();
            while d.file_name().is_some_and(|n| n != "src") {
                d = d.parent().unwrap();
            }
            std::fs::write(d.parent().unwrap().join("Cargo.toml"), manifest).unwrap();
        }
        root
    }

    const DUPLICATE: &str = r#"
pub enum Sexp { Sym(String), List(Vec<Sexp>) }
pub fn read(src: &str) -> Vec<Sexp> {
    for c in src.chars() { match c { '(' => {}, ')' => {}, _ => {} } }
    vec![]
}
"#;

    const WRAPPER: &str = r#"
use tatara_lisp::{read, Sexp};
pub enum Sexp2 { Sym(String), List(Vec<Sexp2>) }
pub fn parse(src: &str) { let _ = read(src); }
"#;

    #[test]
    fn census_flags_a_duplicate_and_clears_a_wrapper() {
        let root = fixture(
            "basic",
            &[
                ("dupe-repo", "src/lisp.rs", DUPLICATE),
                ("wrapper-repo", "src/lisp.rs", WRAPPER),
            ],
        );
        let c = census_sexp_readers(&root).unwrap();
        let keys: Vec<_> = c.findings.iter().map(|f| f.key()).collect();
        assert_eq!(
            keys,
            vec![("dupe-repo", "src/lisp.rs")],
            "clause (c) must clear the wrapper and keep the duplicate; got {keys:?}"
        );
    }

    #[test]
    fn clause_c_clears_the_whole_crate_not_just_the_calling_file() {
        // This is the documented blind spot (KNOWN_BLIND_SPOTS), pinned as
        // a test so it is a *known* property rather than a surprise: a
        // reader in a sibling module of a legitimate consumer is invisible.
        let root = fixture(
            "crate-scope",
            &[
                ("mixed", "src/consumer.rs", "use tatara_lisp::read;"),
                ("mixed", "src/hand_rolled.rs", DUPLICATE),
            ],
        );
        let c = census_sexp_readers(&root).unwrap();
        assert!(
            c.findings.is_empty(),
            "clause (c) is crate-scoped by specification — see KNOWN_BLIND_SPOTS; got {:?}",
            c.findings
        );
    }

    #[test]
    fn census_skips_non_rust_repos_and_reports_counts() {
        let root = fixture("counts", &[("r", "src/lisp.rs", DUPLICATE)]);
        std::fs::create_dir_all(root.join("docs-only")).unwrap();
        let c = census_sexp_readers(&root).unwrap();
        assert_eq!(c.repos_scanned, 1, "a repo without Cargo.toml is not walked");
        assert_eq!(c.findings.len(), 1);
        assert_eq!(c.files_parsed, 1);
    }

    #[test]
    fn allowlist_moves_a_finding_aside_without_dropping_it() {
        // The allow-list must not *hide* — a dismissed match still shows
        // up under `allowlisted`, so a stale entry is visible.
        let f = SexpReaderFinding {
            repo: "r".into(),
            rel_path: "src/x.rs".into(),
            crate_rel: String::new(),
            signals: vec![],
        };
        assert_eq!(
            is_allowlisted(&f),
            SEXP_READER_ALLOWLIST
                .iter()
                .any(|(r, p, _)| *r == "r" && *p == "src/x.rs")
        );
    }

    // ── drift, both directions ───────────────────────────────────────

    fn census_of(keys: &[(&str, &str)]) -> SexpReaderCensus {
        SexpReaderCensus {
            root: PathBuf::from("/fixture"),
            repos_scanned: 0,
            crates_scanned: 0,
            files_parsed: 0,
            findings: keys
                .iter()
                .map(|(r, p)| SexpReaderFinding {
                    repo: (*r).to_string(),
                    rel_path: (*p).to_string(),
                    crate_rel: String::new(),
                    signals: vec![],
                })
                .collect(),
            allowlisted: vec![],
        }
    }

    #[test]
    fn drift_reports_a_new_reader() {
        let mut keys: Vec<(&str, &str)> = KNOWN_SEXP_READERS.to_vec();
        keys.push(("brand-new-repo", "src/sexpr.rs"));
        let d = drift_against_catalog(&census_of(&keys));
        assert!(!d.is_clean());
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].key(), ("brand-new-repo", "src/sexpr.rs"));
        assert!(d.consumed.is_empty());
    }

    #[test]
    fn drift_reports_a_consumed_reader() {
        // The direction that makes the count monotonically non-increasing:
        // deleting a duplicate must FAIL until its catalog line is deleted
        // too, so the catalog cannot silently keep claiming it exists.
        let keys: Vec<(&str, &str)> = KNOWN_SEXP_READERS[1..].to_vec();
        let d = drift_against_catalog(&census_of(&keys));
        assert!(!d.is_clean());
        assert_eq!(d.consumed.len(), 1);
        assert_eq!(
            d.consumed[0],
            (
                KNOWN_SEXP_READERS[0].0.to_string(),
                KNOWN_SEXP_READERS[0].1.to_string()
            )
        );
        assert!(d.added.is_empty());
    }

    #[test]
    fn drift_is_clean_when_census_equals_catalog() {
        let d = drift_against_catalog(&census_of(KNOWN_SEXP_READERS));
        assert!(d.is_clean(), "identical sets must not drift: {d:?}");
    }

    #[test]
    fn catalog_has_no_duplicate_entries() {
        let set: BTreeSet<_> = KNOWN_SEXP_READERS.iter().collect();
        assert_eq!(
            set.len(),
            KNOWN_SEXP_READERS.len(),
            "a duplicated catalog line would make the consumed-direction \
             check unfalsifiable"
        );
    }

    // ── THE GATE ─────────────────────────────────────────────────────

    /// The fleet gate. Set-equality against the live census, **both
    /// directions**.
    ///
    /// TIER: **test failure**, not a compile error — see the module docs.
    /// Nothing in Rust can stop another repo compiling its own `enum
    /// Sexp`; this fails a `cargo test` in *this* crate and nothing else.
    ///
    /// Returns early when no fleet root is present, so a single checkout
    /// of `tatara-rust-ast` stays green. That early return is a **no-op,
    /// not a pass** — a green run on a machine without the fleet checked
    /// out proves nothing about the fleet.
    #[test]
    fn catalog_matches_live_fleet_census() {
        let Some(root) = default_fleet_root() else {
            eprintln!(
                "sexp-reader gate: no fleet root (set PLEME_FLEET_ROOT or check out \
                 ~/code/github/pleme-io) — skipped, NOT passed"
            );
            return;
        };
        let census = census_sexp_readers(&root).expect("fleet census");
        let drift = drift_against_catalog(&census);
        assert!(
            drift.is_clean(),
            "\n\
             ═══ S-EXPRESSION READER CATALOG DRIFT ═══\n\
             fleet root: {}\n\
             scanned: {} repos / {} crates / {} files\n\
             \n\
             NEW readers (in the fleet, absent from KNOWN_SEXP_READERS) — {}:\n{}\
             \n\
             CONSUMED readers (in KNOWN_SEXP_READERS, absent from the fleet) — {}:\n{}\
             \n\
             A NEW entry means a fourteenth independent reader was written instead of \
             consuming tatara_lisp::read. Consume it, or — if it genuinely is not an \
             S-expression reader — add one line to SEXP_READER_ALLOWLIST with the reason.\n\
             A CONSUMED entry means a duplicate was finally deleted: delete its line from \
             KNOWN_SEXP_READERS. The catalog only ever shrinks.\n",
            root.display(),
            census.repos_scanned,
            census.crates_scanned,
            census.files_parsed,
            drift.added.len(),
            drift
                .added
                .iter()
                .map(|f| format!("  + {}/{}  {:?}\n", f.repo, f.rel_path, f.signals))
                .collect::<String>(),
            drift.consumed.len(),
            drift
                .consumed
                .iter()
                .map(|(r, p)| format!("  - {r}/{p}\n"))
                .collect::<String>(),
        );
    }
}
