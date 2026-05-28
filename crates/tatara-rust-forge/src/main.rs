//! `tatara-rust-forge` — operator CLI for the macro-farm.
//!
//! Subcommands:
//!
//!   forge emit-spec <spec.json> <out-dir>
//!     Materialize one tagged-JSON Spec (any of: derive, proc-attr,
//!     proc-fn, macro-rules, suite, catalog) to disk.
//!
//!   forge catalog-emit-repos <catalog.json> --out <dir>
//!                            [--repo-url-prefix <url>]
//!     Materialize a `MacroCatalogSpec` as N **independently
//!     publishable** repos under `<dir>/<crate_name>/…`. Each repo
//!     gets the full pleme-io OSS surface (flake, caixa, auto-release,
//!     clippy-format-ban, LICENSE, .gitignore, rust-toolchain,
//!     README).
//!
//!   forge catalog-gate-all <dir> [--skip-clippy]
//!     Run the green gate (`cargo build && cargo test [&& cargo clippy]`)
//!     against every immediate sub-directory of `<dir>`. Fail-fast.
//!
//! Exit codes:
//!   0  success
//!   1  spec parse / IO error / gate failure
//!   2  usage / missing args

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::{Deserialize, Serialize};
use tatara_rust_ast::CompileToCrate;
use tatara_rust_catalog::MacroCatalogSpec;
use tatara_rust_derive::ProcDeriveSpec;
use tatara_rust_gate::{Gate, GateConfig, GateOutcome, green_gate, green_gate_batch};
use tatara_rust_macro_rules::MacroRulesSpec;
use tatara_rust_proc_attr::ProcAttrSpec;
use tatara_rust_proc_fn::ProcFnSpec;
use tatara_rust_suite::MacroSuiteSpec;
use tatara_rust_verify::build_consumer_verify;
use tatara_rust_publish::{
    PublishConfig, PublishOutcome, RepoPublishSpec, RepoVisibility, publish_all,
};
use tatara_rust_tlisp::{parse_macrocatalog, render_macrocatalog};
use tatara_rust_survey::{
    apply_to_source, survey_apply_validate, survey_file, survey_fleet, survey_tree, PipelineOpts,
};

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum ForgeInput {
    Derive {
        crate_name: String,
        spec: ProcDeriveSpec,
    },
    ProcAttr {
        crate_name: String,
        spec: ProcAttrSpec,
    },
    ProcFn {
        crate_name: String,
        spec: ProcFnSpec,
    },
    MacroRules {
        crate_name: String,
        spec: MacroRulesSpec,
    },
    Suite {
        spec: MacroSuiteSpec,
    },
    Catalog {
        spec: MacroCatalogSpec,
    },
}

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  \
        tatara-rust-forge emit-spec <spec.json> <out-dir>\n  \
        tatara-rust-forge catalog-emit-repos <catalog.json> --out <dir> [--repo-url-prefix <url>]\n  \
        tatara-rust-forge catalog-emit-verify <catalog.json> --out <dir>\n  \
        tatara-rust-forge catalog-gate-all <dir> [--skip-clippy]\n  \
        tatara-rust-forge catalog-publish-all <catalog.json> --dir <dir> --org <github-org> [--private] [--continue-on-error]\n  \
        tatara-rust-forge catalog-instantiate <catalog.json> --out <dir> [--repo-url-prefix <url>] [--skip-clippy] [--no-gate] [--publish --org <org>]\n  \
        tatara-rust-forge catalog-from-lisp <catalog.lisp> <out.json>   (parse defmacrocatalog → JSON)\n  \
        tatara-rust-forge catalog-to-lisp <catalog.json> <out.lisp>     (render JSON → defmacrocatalog)\n  \
        tatara-rust-forge survey <crate-src-path> [--json]              (scan a Rust crate for derive-adoption candidates)\n  \
        tatara-rust-forge survey-apply <file.rs> [--write]              (apply the first candidate to a file; --write commits to disk)\n  \
        tatara-rust-forge survey-apply-all <crate-root> [--write] [--no-validate] [--skip-clippy] [--no-inject-deps] (whole-crate survey → apply → inject Cargo.toml deps → validate; rolls back on red gate)\n  \
        tatara-rust-forge survey-fleet <org-root> [--threshold N] [--json] (aggregate every crate under org-root; leaderboard sorted by candidate count)"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return usage();
    }
    let cmd = args[0].as_str();
    let rest = &args[1..];

    let result: Result<String, String> = match cmd {
        "emit-spec" => cmd_emit_spec(rest),
        "catalog-emit-repos" => cmd_catalog_emit_repos(rest),
        "catalog-emit-verify" => cmd_catalog_emit_verify(rest),
        "catalog-gate-all" => cmd_catalog_gate_all(rest),
        "catalog-publish-all" => cmd_catalog_publish_all(rest),
        "catalog-instantiate" => cmd_catalog_instantiate(rest),
        "catalog-from-lisp" => cmd_catalog_from_lisp(rest),
        "catalog-to-lisp" => cmd_catalog_to_lisp(rest),
        "survey" => cmd_survey(rest),
        "survey-apply" => cmd_survey_apply(rest),
        "survey-apply-all" => cmd_survey_apply_all(rest),
        "survey-fleet" => cmd_survey_fleet(rest),
        "--help" | "-h" => {
            usage();
            return ExitCode::SUCCESS;
        }
        _ => return usage(),
    };

    match result {
        Ok(msg) => {
            println!("tatara-rust-forge: {msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("tatara-rust-forge: {e}");
            ExitCode::from(1)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// emit-spec — legacy single-tagged-JSON materialization
// ─────────────────────────────────────────────────────────────────────

fn cmd_emit_spec(args: &[String]) -> Result<String, String> {
    let [spec_path, out_dir] = args else {
        return Err("emit-spec: needs <spec.json> <out-dir>".into());
    };
    let spec_path = Path::new(spec_path);
    let out_dir = Path::new(out_dir);

    let text = std::fs::read_to_string(spec_path)
        .map_err(|e| format!("read {}: {e}", spec_path.display()))?;
    let input: ForgeInput = serde_json::from_str(&text)
        .map_err(|e| format!("parse {}: {e}", spec_path.display()))?;

    match input {
        ForgeInput::Derive { crate_name, spec } => materialize_crate(&spec, &crate_name, out_dir),
        ForgeInput::ProcAttr { crate_name, spec } => materialize_crate(&spec, &crate_name, out_dir),
        ForgeInput::ProcFn { crate_name, spec } => materialize_crate(&spec, &crate_name, out_dir),
        ForgeInput::MacroRules { crate_name, spec } => {
            materialize_crate(&spec, &crate_name, out_dir)
        }
        ForgeInput::Suite { spec } => materialize_workspace(&spec, out_dir),
        ForgeInput::Catalog { spec } => materialize_catalog(&spec, out_dir),
    }
}

fn materialize_crate<T: CompileToCrate>(
    spec: &T,
    crate_name: &str,
    out_dir: &Path,
) -> Result<String, String> {
    let scaffold = spec
        .compile_to_crate(crate_name)
        .map_err(|e| format!("compile: {e}"))?;
    let path = out_dir.join(crate_name);
    scaffold
        .write_to(&path)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(format!("wrote {}", path.display()))
}

fn materialize_workspace(spec: &MacroSuiteSpec, out_dir: &Path) -> Result<String, String> {
    let ws = spec
        .compile_to_workspace()
        .map_err(|e| format!("compile workspace: {e}"))?;
    let path = out_dir.join(&ws.workspace_name);
    ws.write_to(&path)
        .map_err(|e| format!("write workspace {}: {e}", path.display()))?;
    Ok(format!("wrote workspace {}", path.display()))
}

fn materialize_catalog(spec: &MacroCatalogSpec, out_dir: &Path) -> Result<String, String> {
    let scaffold = spec
        .compile_to_catalog()
        .map_err(|e| format!("compile catalog: {e}"))?;
    let path = out_dir.join(&spec.title);
    scaffold
        .write_to(&path)
        .map_err(|e| format!("write catalog {}: {e}", path.display()))?;
    Ok(format!("wrote catalog {}", path.display()))
}

// ─────────────────────────────────────────────────────────────────────
// catalog-emit-repos — N independent decorated repos
// ─────────────────────────────────────────────────────────────────────

fn cmd_catalog_emit_repos(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("catalog-emit-repos: needs <catalog.json> --out <dir>".into());
    }
    let catalog_path = Path::new(&args[0]);
    let mut out_dir: Option<PathBuf> = None;
    let mut repo_url_prefix = String::from("https://github.com/pleme-io");
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out_dir = Some(PathBuf::from(args.get(i).ok_or("--out needs <dir>")?));
            }
            "--repo-url-prefix" => {
                i += 1;
                repo_url_prefix = args
                    .get(i)
                    .ok_or("--repo-url-prefix needs <url>")?
                    .clone();
            }
            other => return Err(format!("unknown flag: {other}")),
        }
        i += 1;
    }
    let out_dir = out_dir.ok_or("--out is required")?;

    let catalog = load_catalog(catalog_path)?;
    let repos = catalog
        .compile_to_repos(&repo_url_prefix)
        .map_err(|e| format!("compile repos: {e}"))?;

    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("mkdir {}: {e}", out_dir.display()))?;

    let mut written = 0usize;
    for spec in repos {
        let compiled = spec.compile();
        let path = out_dir.join(&compiled.name);
        compiled
            .write_to(&path)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        written += 1;
    }

    Ok(format!("emitted {written} repos under {}", out_dir.display()))
}

// ─────────────────────────────────────────────────────────────────────
// catalog-emit-verify — auto-emit consumer-verify/ harness
// ─────────────────────────────────────────────────────────────────────

fn cmd_catalog_emit_verify(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("catalog-emit-verify: needs <catalog.json> --out <dir>".into());
    }
    let catalog_path = Path::new(&args[0]);
    let mut out_dir: Option<PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out_dir = Some(PathBuf::from(args.get(i).ok_or("--out needs <dir>")?));
            }
            other => return Err(format!("unknown flag: {other}")),
        }
        i += 1;
    }
    let out_dir = out_dir.ok_or("--out is required")?;

    let catalog = load_catalog(catalog_path)?;
    let scaffold = build_consumer_verify(&catalog, |c| format!("../{c}"));
    let verify_root = out_dir.join("consumer-verify");
    scaffold
        .write_to(&verify_root)
        .map_err(|e| format!("write {}: {e}", verify_root.display()))?;
    Ok(format!(
        "wrote consumer-verify ({} entries) at {}",
        catalog.entries.len(),
        verify_root.display()
    ))
}

// ─────────────────────────────────────────────────────────────────────
// catalog-gate-all — green-gate every repo under <dir>
// ─────────────────────────────────────────────────────────────────────

fn cmd_catalog_gate_all(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("catalog-gate-all: needs <dir>".into());
    }
    let dir = PathBuf::from(&args[0]);
    let mut skip_clippy = false;
    for a in &args[1..] {
        match a.as_str() {
            "--skip-clippy" => skip_clippy = true,
            other => return Err(format!("unknown flag: {other}")),
        }
    }

    let mut roots: Vec<PathBuf> = vec![];
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for e in entries {
        let e = e.map_err(|e| format!("read_dir entry: {e}"))?;
        let p = e.path();
        if p.is_dir() && p.join("Cargo.toml").exists() {
            roots.push(p);
        }
    }
    roots.sort();

    let gates = if skip_clippy {
        vec![Gate::Build, Gate::Test]
    } else {
        vec![Gate::Build, Gate::Test, Gate::Clippy]
    };
    let cfg = GateConfig {
        gates,
        skip_if_no_cargo_toml: true,
    };

    let outcome = green_gate_batch(&roots, &cfg)
        .map_err(|e| format!("gate batch: {e}"))?;

    if outcome.all_passed() {
        Ok(format!("{}/{} repos passed", outcome.passed_count(), roots.len()))
    } else {
        // Surface the first failure for the operator.
        for (path, oc) in &outcome.results {
            if let GateOutcome::Failed {
                gate,
                exit,
                stderr,
                ..
            } = oc
            {
                return Err(format!(
                    "{} failed at {} (exit {:?}):\n{}",
                    path.display(),
                    gate.label(),
                    exit,
                    stderr
                ));
            }
        }
        Err(format!(
            "batch did not all-pass: {} of {} green",
            outcome.passed_count(),
            roots.len()
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────
// catalog-publish-all — git+gh publish every emitted repo
// ─────────────────────────────────────────────────────────────────────

fn cmd_catalog_publish_all(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("catalog-publish-all: needs <catalog.json> --dir <dir> --org <org>".into());
    }
    let catalog_path = Path::new(&args[0]);
    let mut dir: Option<PathBuf> = None;
    let mut org: Option<String> = None;
    let mut private = false;
    let mut continue_on_error = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--dir" => {
                i += 1;
                dir = Some(PathBuf::from(args.get(i).ok_or("--dir needs <dir>")?));
            }
            "--org" => {
                i += 1;
                org = Some(args.get(i).ok_or("--org needs <github-org>")?.clone());
            }
            "--private" => private = true,
            "--continue-on-error" => continue_on_error = true,
            other => return Err(format!("unknown flag: {other}")),
        }
        i += 1;
    }
    let dir = dir.ok_or("--dir is required")?;
    let org = org.ok_or("--org is required")?;

    let catalog = load_catalog(catalog_path)?;

    let mut specs: Vec<RepoPublishSpec> = vec![];
    for entry in &catalog.entries {
        specs.push(RepoPublishSpec {
            path: dir.join(&entry.crate_name),
            full_name: format!("{org}/{}", entry.crate_name),
            description: entry.description.clone(),
            commit_message: None,
        });
    }

    let cfg = PublishConfig {
        visibility: if private {
            RepoVisibility::Private
        } else {
            RepoVisibility::Public
        },
        fail_fast: !continue_on_error,
        ..PublishConfig::default()
    };

    let batch = publish_all(&specs, &cfg).map_err(|e| format!("publish batch: {e}"))?;

    if batch.all_succeeded() {
        Ok(format!(
            "published {}/{} repos to {} ({} created, {} updated, {} up-to-date)",
            batch.success_count(),
            specs.len(),
            org,
            batch.results.iter().filter(|o| matches!(o, PublishOutcome::Created { .. })).count(),
            batch.results.iter().filter(|o| matches!(o, PublishOutcome::Updated { .. })).count(),
            batch.results.iter().filter(|o| matches!(o, PublishOutcome::UpToDate { .. })).count(),
        ))
    } else {
        // Surface the first failure.
        for oc in &batch.results {
            if let PublishOutcome::Failed { full_name, step, exit, stderr } = oc {
                return Err(format!(
                    "publish: {} failed at {} (exit {:?}):\n{}",
                    full_name,
                    step.label(),
                    exit,
                    stderr
                ));
            }
        }
        Err(format!(
            "batch did not all-succeed: {}/{} green",
            batch.success_count(),
            specs.len()
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────
// catalog-instantiate — ONE-SHOT: emit-repos + emit-verify + gate
// ─────────────────────────────────────────────────────────────────────

fn cmd_catalog_instantiate(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("catalog-instantiate: needs <catalog.json> --out <dir>".into());
    }
    let catalog_path = Path::new(&args[0]);
    let mut out_dir: Option<PathBuf> = None;
    let mut repo_url_prefix = String::from("https://github.com/pleme-io");
    let mut skip_clippy = false;
    let mut no_gate = false;
    let mut publish_org: Option<String> = None;
    let mut publish_private = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out_dir = Some(PathBuf::from(args.get(i).ok_or("--out needs <dir>")?));
            }
            "--repo-url-prefix" => {
                i += 1;
                repo_url_prefix = args
                    .get(i)
                    .ok_or("--repo-url-prefix needs <url>")?
                    .clone();
            }
            "--skip-clippy" => skip_clippy = true,
            "--no-gate" => no_gate = true,
            "--publish" => {
                // Next arg can be --org X or skipped if --org appears later.
            }
            "--org" => {
                i += 1;
                publish_org = Some(args.get(i).ok_or("--org needs <github-org>")?.clone());
            }
            "--private" => publish_private = true,
            other => return Err(format!("unknown flag: {other}")),
        }
        i += 1;
    }
    let out_dir = out_dir.ok_or("--out is required")?;

    let catalog = load_catalog(catalog_path)?;

    // Phase 1 — repos.
    let repos = catalog
        .compile_to_repos(&repo_url_prefix)
        .map_err(|e| format!("compile repos: {e}"))?;
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("mkdir {}: {e}", out_dir.display()))?;
    for spec in repos {
        let compiled = spec.compile();
        let path = out_dir.join(&compiled.name);
        compiled
            .write_to(&path)
            .map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    let repo_count = catalog.entries.len();

    // Phase 2 — consumer-verify.
    let verify_scaffold = build_consumer_verify(&catalog, |c| format!("../{c}"));
    let verify_root = out_dir.join("consumer-verify");
    verify_scaffold
        .write_to(&verify_root)
        .map_err(|e| format!("write {}: {e}", verify_root.display()))?;

    if no_gate {
        return Ok(format!(
            "instantiated {repo_count} repos + consumer-verify at {} (gate skipped)",
            out_dir.display()
        ));
    }

    // Phase 3 — gate each repo individually, then the verify crate.
    let mut roots: Vec<PathBuf> = vec![];
    let entries =
        std::fs::read_dir(&out_dir).map_err(|e| format!("read_dir {}: {e}", out_dir.display()))?;
    for e in entries {
        let e = e.map_err(|e| format!("read_dir entry: {e}"))?;
        let p = e.path();
        if p.is_dir() && p.join("Cargo.toml").exists() {
            roots.push(p);
        }
    }
    roots.sort();

    let gates = if skip_clippy {
        vec![Gate::Build, Gate::Test]
    } else {
        vec![Gate::Build, Gate::Test, Gate::Clippy]
    };
    let cfg = GateConfig {
        gates,
        skip_if_no_cargo_toml: true,
    };

    // Gate the N derive repos first.
    let batch = green_gate_batch(&roots, &cfg).map_err(|e| format!("gate batch: {e}"))?;
    if !batch.all_passed() {
        for (path, oc) in &batch.results {
            if let GateOutcome::Failed {
                gate,
                exit,
                stderr,
                ..
            } = oc
            {
                return Err(format!(
                    "instantiate: {} failed at {} (exit {:?}):\n{}",
                    path.display(),
                    gate.label(),
                    exit,
                    stderr
                ));
            }
        }
    }

    // Gate the consumer-verify crate explicitly — it depends on the
    // N derives + must compile and pass tests.
    let verify_outcome =
        green_gate(&verify_root, &cfg).map_err(|e| format!("verify gate: {e}"))?;
    if let GateOutcome::Failed {
        gate,
        exit,
        stderr,
        ..
    } = &verify_outcome
    {
        return Err(format!(
            "instantiate: consumer-verify failed at {} (exit {:?}):\n{}",
            gate.label(),
            exit,
            stderr
        ));
    }

    // Phase 4 — publish (optional, only when --org provided).
    let publish_summary = if let Some(org) = publish_org {
        let mut specs: Vec<RepoPublishSpec> = vec![];
        for entry in &catalog.entries {
            specs.push(RepoPublishSpec {
                path: out_dir.join(&entry.crate_name),
                full_name: format!("{org}/{}", entry.crate_name),
                description: entry.description.clone(),
                commit_message: None,
            });
        }
        let pcfg = PublishConfig {
            visibility: if publish_private {
                RepoVisibility::Private
            } else {
                RepoVisibility::Public
            },
            ..PublishConfig::default()
        };
        let pbatch =
            publish_all(&specs, &pcfg).map_err(|e| format!("publish batch: {e}"))?;
        if !pbatch.all_succeeded() {
            for oc in &pbatch.results {
                if let PublishOutcome::Failed {
                    full_name,
                    step,
                    exit,
                    stderr,
                } = oc
                {
                    return Err(format!(
                        "instantiate: publish {} failed at {} (exit {:?}):\n{}",
                        full_name,
                        step.label(),
                        exit,
                        stderr
                    ));
                }
            }
        }
        Some(pbatch.success_count())
    } else {
        None
    };

    let publish_note = match publish_summary {
        Some(n) => format!(", published {n} to GitHub"),
        None => String::new(),
    };

    Ok(format!(
        "instantiated {repo_count} repos + consumer-verify at {} ({} repos passed all gates, consumer-verify {}{publish_note})",
        out_dir.display(),
        batch.passed_count(),
        if verify_outcome.is_passed() {
            "passed"
        } else {
            "skipped"
        }
    ))
}

// ─────────────────────────────────────────────────────────────────────
// load_catalog — auto-detect JSON vs tlisp by file extension
// ─────────────────────────────────────────────────────────────────────

/// Load a `MacroCatalogSpec` from either JSON or tlisp form. The
/// extension drives the dispatch: `.lisp` / `.tlisp` → tlisp parser;
/// anything else → serde_json. Lets every subcommand that takes a
/// catalog accept both formats transparently.
fn load_catalog(path: &Path) -> Result<MacroCatalogSpec, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "lisp" | "tlisp" => parse_macrocatalog(&text)
            .map_err(|e| format!("parse {} (tlisp): {e}", path.display())),
        _ => serde_json::from_str(&text)
            .map_err(|e| format!("parse {} (json): {e}", path.display())),
    }
}

// ─────────────────────────────────────────────────────────────────────
// catalog-from-lisp / catalog-to-lisp — tlisp authoring round-trip
// ─────────────────────────────────────────────────────────────────────

fn cmd_catalog_from_lisp(args: &[String]) -> Result<String, String> {
    let [lisp_path, json_out] = args else {
        return Err("catalog-from-lisp: needs <catalog.lisp> <out.json>".into());
    };
    let src = std::fs::read_to_string(lisp_path)
        .map_err(|e| format!("read {lisp_path}: {e}"))?;
    let catalog = parse_macrocatalog(&src)
        .map_err(|e| format!("parse {lisp_path}: {e}"))?;
    let json = serde_json::to_string_pretty(&catalog)
        .map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(json_out, &json)
        .map_err(|e| format!("write {json_out}: {e}"))?;
    Ok(format!(
        "wrote {} ({} entries) — (defmacrocatalog) → JSON",
        json_out,
        catalog.entries.len()
    ))
}

fn cmd_catalog_to_lisp(args: &[String]) -> Result<String, String> {
    let [json_path, lisp_out] = args else {
        return Err("catalog-to-lisp: needs <catalog.json> <out.lisp>".into());
    };
    let src = std::fs::read_to_string(json_path)
        .map_err(|e| format!("read {json_path}: {e}"))?;
    let catalog: MacroCatalogSpec = serde_json::from_str(&src)
        .map_err(|e| format!("parse {json_path}: {e}"))?;
    let lisp = render_macrocatalog(&catalog);
    std::fs::write(lisp_out, &lisp)
        .map_err(|e| format!("write {lisp_out}: {e}"))?;
    Ok(format!(
        "wrote {} ({} entries) — JSON → (defmacrocatalog)",
        lisp_out,
        catalog.entries.len()
    ))
}

// ─────────────────────────────────────────────────────────────────────
// survey — find derive-adoption candidates in a Rust crate
// ─────────────────────────────────────────────────────────────────────

fn cmd_survey(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("survey: needs <crate-src-path> [--json]".into());
    }
    let root = PathBuf::from(&args[0]);
    let json_out = args.iter().any(|a| a == "--json");

    let cands = survey_tree(&root)
        .map_err(|e| format!("survey {}: {e}", root.display()))?;

    if json_out {
        let s = serde_json::to_string_pretty(&cands).map_err(|e| format!("serialize: {e}"))?;
        println!("{s}");
        return Ok(format!("{} candidates", cands.len()));
    }

    // Operator-facing punch list.
    if cands.is_empty() {
        return Ok(format!(
            "no derive-adoption candidates found under {}",
            root.display()
        ));
    }
    println!("# Derive adoption survey: {}\n", root.display());
    println!("Found {} candidates:\n", cands.len());
    let mut total_loc = 0;
    for c in &cands {
        println!(
            "  {} (line {}): impl {} → #[derive({})]",
            c.file.display(),
            c.line,
            c.target_type,
            c.derive_trait,
        );
        println!(
            "    pattern: {:?}  |  add dep: {}  |  ~{} LOC saved",
            c.pattern, c.derive_crate, c.estimated_loc_saved,
        );
        println!();
        total_loc += c.estimated_loc_saved;
    }
    Ok(format!(
        "{} candidates; ~{} LOC saveable across {} files",
        cands.len(),
        total_loc,
        cands
            .iter()
            .map(|c| c.file.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
    ))
}

// ─────────────────────────────────────────────────────────────────────
// survey-apply — land the refactor in source via syn::visit_mut
// ─────────────────────────────────────────────────────────────────────

fn cmd_survey_apply(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("survey-apply: needs <file.rs> [--write]".into());
    }
    let file_path = PathBuf::from(&args[0]);
    let write = args.iter().any(|a| a == "--write");

    let cands = survey_file(&file_path)
        .map_err(|e| format!("survey {}: {e}", file_path.display()))?;
    let Some(cand) = cands.into_iter().next() else {
        return Ok(format!(
            "no candidates in {} — nothing to apply",
            file_path.display()
        ));
    };

    let src = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("read {}: {e}", file_path.display()))?;
    let modified = apply_to_source(&src, &cand)
        .map_err(|e| format!("apply: {e}"))?;

    if write {
        std::fs::write(&file_path, &modified)
            .map_err(|e| format!("write {}: {e}", file_path.display()))?;
        Ok(format!(
            "applied {:?} to {} → impl {} now derives {}; wrote to disk. \
             Don't forget to add `{}` to Cargo.toml as a git dep.",
            cand.pattern,
            file_path.display(),
            cand.target_type,
            cand.derive_trait,
            cand.derive_crate,
        ))
    } else {
        println!("=== DRY-RUN: tatara-rust-forge survey-apply ===");
        println!("File:        {}", file_path.display());
        println!("Target type: {}", cand.target_type);
        println!("Pattern:     {:?}", cand.pattern);
        println!("Derive:      #[derive({})]", cand.derive_trait);
        println!("Cargo dep:   {}", cand.derive_crate);
        println!("─────── modified source ───────");
        println!("{modified}");
        println!("─────── (re-run with --write to apply to disk) ───────");
        Ok(format!(
            "preview ready ({} bytes); re-run with --write to land",
            modified.len()
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────
// survey-apply-all — whole-crate adoption with rollback on red gate
// ─────────────────────────────────────────────────────────────────────

fn cmd_survey_apply_all(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err(
            "survey-apply-all: needs <crate-root> [--write] [--no-validate] [--skip-clippy] [--no-inject-deps]"
                .into(),
        );
    }
    let crate_root = PathBuf::from(&args[0]);
    let write = args.iter().any(|a| a == "--write");
    let no_validate = args.iter().any(|a| a == "--no-validate");
    let skip_clippy = args.iter().any(|a| a == "--skip-clippy");
    let no_inject_deps = args.iter().any(|a| a == "--no-inject-deps");

    let mut gate_cfg = GateConfig::default();
    if skip_clippy {
        gate_cfg.gates.retain(|g| !matches!(g, Gate::Clippy));
    }

    let opts = PipelineOpts {
        write,
        validate: !no_validate,
        gate_cfg,
        inject_cargo_deps: !no_inject_deps,
        dep_source: tatara_rust_survey::DepSource::default(),
    };

    let out = survey_apply_validate(&crate_root, &opts)
        .map_err(|e| format!("pipeline: {e}"))?;

    let mode_label = match (write, no_validate) {
        (false, _) => "DRY-RUN",
        (true, true) => "WRITE (no-validate)",
        (true, false) => "WRITE + VALIDATE",
    };
    println!("=== tatara-rust-forge survey-apply-all ===");
    println!("Crate:  {}", out.crate_root.display());
    println!("Mode:   {mode_label}");
    println!(
        "Files:  {} touched, {} written",
        out.files.len(),
        out.files.iter().filter(|f| f.written).count(),
    );
    println!(
        "Candidates: {} attempted, {} applied",
        out.total_candidates, out.total_applied,
    );
    if let Some(deps) = &out.deps_injected {
        if deps.changed() {
            println!(
                "Deps:   injected {} into Cargo.toml ({})",
                deps.added.len(),
                deps.added.join(", "),
            );
        } else if !deps.already_present.is_empty() {
            println!("Deps:   already present ({})", deps.already_present.join(", "));
        }
    }
    if let Some(gate) = &out.gate {
        match gate {
            GateOutcome::Passed => println!("Gate:   ✓ passed (build + test{})", if skip_clippy { "" } else { " + clippy" }),
            GateOutcome::SkippedNoCargo => println!("Gate:   skipped (no Cargo.toml at root)"),
            GateOutcome::Failed { gate: g, exit, stderr, .. } => {
                println!("Gate:   ✗ FAILED at `{}` (exit {:?})", g.label(), exit);
                println!("──── stderr (tail) ────");
                for line in stderr.lines().rev().take(20).collect::<Vec<_>>().iter().rev() {
                    println!("{line}");
                }
            }
        }
    } else if write {
        println!("Gate:   skipped (--no-validate)");
    } else {
        println!("Gate:   skipped (dry run)");
    }
    if out.rolled_back {
        println!("Rolled back: every written file restored to its pre-pipeline contents.");
    }

    if out.rolled_back {
        Err(format!(
            "rolled back ({} applied across {} files; gate went red — working tree restored)",
            out.total_applied,
            out.files.iter().filter(|f| f.candidates_applied > 0).count(),
        ))
    } else {
        Ok(format!(
            "{} candidates applied across {} files; {}",
            out.total_applied,
            out.files.iter().filter(|f| f.candidates_applied > 0).count(),
            match (write, no_validate) {
                (false, _) => "dry run — re-run with --write to land",
                (true, true) => "written to disk (no validation requested)",
                (true, false) => "validated green — changes live",
            },
        ))
    }
}

// ─────────────────────────────────────────────────────────────────────
// survey-fleet — aggregate every crate under an org directory
// ─────────────────────────────────────────────────────────────────────

fn cmd_survey_fleet(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("survey-fleet: needs <org-root> [--threshold N] [--json]".into());
    }
    let root = PathBuf::from(&args[0]);
    let json = args.iter().any(|a| a == "--json");
    let threshold: usize = {
        let mut t = 1usize;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a == "--threshold" {
                if let Some(n) = it.next() {
                    t = n.parse().map_err(|e| format!("--threshold: {e}"))?;
                }
            }
        }
        t
    };

    let report = survey_fleet(&root).map_err(|e| format!("fleet: {e}"))?;

    if json {
        let s = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        println!("{s}");
        return Ok(format!(
            "{} crates scanned; {} with ≥{} candidates",
            report.crates_scanned,
            report.entries_at_least(threshold).len(),
            threshold,
        ));
    }

    println!("=== tatara-rust-forge survey-fleet ===");
    println!("Root: {}", report.root.display());
    println!(
        "Scanned: {} Cargo crates    With candidates: {}",
        report.crates_scanned, report.crates_with_candidates,
    );
    println!(
        "Totals: {} candidates    ~{} LOC saveable",
        report.total_candidates, report.total_loc_saved,
    );
    println!("Pattern totals:");
    for (pat, n) in &report.pattern_totals {
        println!("  {pat:<14} {n}");
    }
    println!();

    let above = report.entries_at_least(threshold);
    if above.is_empty() {
        println!("No crates ≥ threshold ({threshold}).");
    } else {
        println!("─── Leaderboard (≥ {threshold} candidates) ───");
        println!("{:>4}  {:>4}  {:<60}  patterns", "cand", "loc", "crate");
        for e in &above {
            let breakdown: Vec<String> = e
                .pattern_breakdown
                .iter()
                .map(|(p, n)| format!("{p}:{n}"))
                .collect();
            println!(
                "{:>4}  {:>4}  {:<60}  {}",
                e.candidate_count,
                e.loc_saved,
                e.crate_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| e.crate_path.display().to_string()),
                breakdown.join(" "),
            );
        }
    }

    Ok(format!(
        "{} crates ≥ threshold ({}); {} total candidates fleet-wide",
        above.len(),
        threshold,
        report.total_candidates,
    ))
}
