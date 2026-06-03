//! Drift guard — the committed docs MUST be a byte-exact render of the
//! live catalog. This is the ★★ CATALOG REFLECTION discipline applied to
//! on-disk documentation: a catalog change that lands without
//! regenerating the docs trips this test instead of silently rotting the
//! reference. Regenerate with:
//!
//! ```bash
//! tatara-rust-forge catalog-emit-docs catalogs/pleme-derives.lisp --out .
//! ```
//!
//! The test is skipped (not failed) when the committed artifacts don't
//! exist yet, so a fresh checkout that hasn't run the generator once is
//! not a hard error — it becomes one the moment the docs are committed.

use std::path::PathBuf;

use tatara_rust_docs::render_docs;
use tatara_rust_tlisp::parse_macrocatalog;

fn repo_root() -> PathBuf {
    // crates/tatara-rust-docs → repo root is two levels up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

#[test]
fn committed_docs_match_catalog_render() {
    let root = repo_root();
    let catalog_path = root.join("catalogs/pleme-derives.lisp");
    let reference_path = root.join("docs/derives-reference.md");
    let skill_path = root.join("skills/consume-pleme-derives/SKILL.md");
    let claude_path = root.join("docs/CLAUDE-derives-fragment.md");

    // Fresh checkout that hasn't generated docs yet: nothing to drift.
    if !reference_path.exists() {
        eprintln!(
            "skip: {} not generated yet — run `catalog-emit-docs`",
            reference_path.display()
        );
        return;
    }

    let src = std::fs::read_to_string(&catalog_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", catalog_path.display()));
    let catalog = parse_macrocatalog(&src).expect("catalog parses");
    let bundle = render_docs(&catalog);

    let pairs = [
        (reference_path, bundle.reference_md, "docs/derives-reference.md"),
        (claude_path, bundle.claude_fragment_md, "docs/CLAUDE-derives-fragment.md"),
        (skill_path, bundle.skill_md, "skills/consume-pleme-derives/SKILL.md"),
    ];
    for (path, rendered, label) in pairs {
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert_eq!(
            committed, rendered,
            "{label} is stale — regenerate with `tatara-rust-forge catalog-emit-docs catalogs/pleme-derives.lisp --out .`"
        );
    }
}
