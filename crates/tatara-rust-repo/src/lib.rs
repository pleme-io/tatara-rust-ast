//! `tatara-rust-repo` — typed publishable-repo decorator.
//!
//! `RepoSpec` composes a generated `CrateScaffold` with the canonical
//! pleme-io OSS-publish surface:
//!   - `flake.nix`                    (substrate `mkRustToolFlake`)
//!   - `caixa.lisp`                   (`(defcaixa :kind Biblioteca)`)
//!   - `.github/workflows/auto-release.yml`  (3-line shim → substrate reusable)
//!   - `clippy.toml`                  (TYPED-EMISSION directive: ban `std::format`)
//!   - `LICENSE`                      (MIT, year + holder configurable)
//!   - `.gitignore`                   (Rust + flake + nix-result)
//!   - `README.md`                    (deterministic from Spec — title, install, example)
//!   - `rust-toolchain.toml`          (pinned channel)
//!
//! Every decorator is idempotent (custom-supplied files survive). The
//! whole composition is one fluent builder + one `compile()` returning
//! a `CrateScaffold` ready for `write_to(path)`.
//!
//! This is the typescape primitive a `MacroCatalogSpec` consumes when
//! it materializes N independently-publishable repos.

use serde::{Deserialize, Serialize};
use tatara_rust_ast::CrateScaffold;
use tatara_rust_caixa::{CaixaConfig, attach_caixa_biblioteca};
use tatara_rust_flake::attach_substrate_flake;

/// License choice. Fleet default is MIT; extend the enum to broaden.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum License {
    Mit { year: u32, holder: String },
    Apache2 { year: u32, holder: String },
    Mpl2 { year: u32, holder: String },
}

impl License {
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Mit { year, holder } => render_mit(*year, holder),
            Self::Apache2 { year, holder } => render_apache2(*year, holder),
            Self::Mpl2 { year, holder } => render_mpl2(*year, holder),
        }
    }
    #[must_use]
    pub fn spdx(&self) -> &'static str {
        match self {
            Self::Mit { .. } => "MIT",
            Self::Apache2 { .. } => "Apache-2.0",
            Self::Mpl2 { .. } => "MPL-2.0",
        }
    }
}

/// Typed publishable-repo spec — wraps a generated crate with every
/// piece of OSS-publish surface the pleme-io substrate expects.
///
/// Builder methods all return `Self` for one-line fluent composition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepoSpec {
    pub scaffold: CrateScaffold,
    pub flake: bool,
    pub caixa: Option<CaixaConfigOwned>,
    pub auto_release: bool,
    pub clippy_format_ban: bool,
    pub license: Option<License>,
    pub gitignore: bool,
    pub rust_toolchain: Option<String>,
    pub readme: Option<ReadmeSpec>,
}

/// Owned mirror of `tatara_rust_caixa::CaixaConfig` (which is not
/// `Serialize`). Kept here so a `MacroCatalogSpec` can author RepoSpec
/// values from JSON.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CaixaConfigOwned {
    pub description: Option<String>,
    pub attach_auto_release: bool,
}

impl From<&CaixaConfigOwned> for CaixaConfig {
    fn from(c: &CaixaConfigOwned) -> Self {
        CaixaConfig {
            description: c.description.clone(),
            attach_auto_release: c.attach_auto_release,
        }
    }
}

/// README body inputs — rendered deterministically from the Spec, no
/// per-repo prose to author.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadmeSpec {
    pub title: String,
    pub one_line: String,
    /// Repo URL on GitHub (used for badges + links).
    pub repo_url: String,
    /// Optional usage example body (Markdown fenced block).
    pub usage_example: Option<String>,
}

impl RepoSpec {
    /// Wrap a generated `CrateScaffold` with sensible OSS defaults:
    /// flake on, clippy-format-ban on, MIT license, gitignore on,
    /// auto-release on, caixa Biblioteca attached, README from the
    /// scaffold's name. Builder methods override.
    #[must_use]
    pub fn defaults_for(
        scaffold: CrateScaffold,
        readme_title: impl Into<String>,
        repo_url: impl Into<String>,
        one_line: impl Into<String>,
    ) -> Self {
        let title = readme_title.into();
        Self {
            scaffold,
            flake: true,
            caixa: Some(CaixaConfigOwned {
                description: None,
                attach_auto_release: true,
            }),
            auto_release: true,
            clippy_format_ban: true,
            license: Some(License::Mit {
                year: 2026,
                holder: "pleme-io".into(),
            }),
            gitignore: true,
            rust_toolchain: Some("1.89.0".into()),
            readme: Some(ReadmeSpec {
                title,
                one_line: one_line.into(),
                repo_url: repo_url.into(),
                usage_example: None,
            }),
        }
    }

    #[must_use]
    pub fn without_flake(mut self) -> Self {
        self.flake = false;
        self
    }
    #[must_use]
    pub fn without_caixa(mut self) -> Self {
        self.caixa = None;
        self
    }
    #[must_use]
    pub fn without_auto_release(mut self) -> Self {
        self.auto_release = false;
        self
    }
    #[must_use]
    pub fn with_license(mut self, l: License) -> Self {
        self.license = Some(l);
        self
    }
    #[must_use]
    pub fn with_usage_example(mut self, example: impl Into<String>) -> Self {
        if let Some(r) = &mut self.readme {
            r.usage_example = Some(example.into());
        }
        self
    }

    /// Apply every decoration in order. The same `CrateScaffold` flows
    /// through each `attach_*` mutator; idempotent everywhere.
    #[must_use]
    pub fn compile(self) -> CrateScaffold {
        let mut s = self.scaffold;
        if self.flake {
            attach_substrate_flake(&mut s);
        }
        if let Some(c) = &self.caixa {
            // CaixaConfig's `attach_auto_release` is separate from our
            // top-level `auto_release` knob — prefer the top-level.
            let cfg = CaixaConfig {
                description: c.description.clone(),
                attach_auto_release: self.auto_release,
            };
            attach_caixa_biblioteca(&mut s, &cfg);
        } else if self.auto_release && !s.files.iter().any(|f| f.path == ".github/workflows/auto-release.yml") {
            // No caixa, but operator wants auto-release: attach the workflow directly.
            s.add_file(
                ".github/workflows/auto-release.yml",
                tatara_rust_caixa::render_auto_release_workflow(),
            );
        }
        if self.clippy_format_ban && !s.files.iter().any(|f| f.path == "clippy.toml") {
            s.add_file("clippy.toml", clippy_format_ban_toml());
        }
        if let Some(l) = &self.license
            && !s.files.iter().any(|f| f.path == "LICENSE")
        {
            s.add_file("LICENSE", l.render());
        }
        if self.gitignore && !s.files.iter().any(|f| f.path == ".gitignore") {
            s.add_file(".gitignore", rust_gitignore());
        }
        if let Some(channel) = &self.rust_toolchain
            && !s.files.iter().any(|f| f.path == "rust-toolchain.toml")
        {
            s.add_file("rust-toolchain.toml", rust_toolchain_toml(channel));
        }
        if let Some(r) = &self.readme
            && !s.files.iter().any(|f| f.path == "README.md")
        {
            s.add_file("README.md", render_readme(r, &s.name));
        }
        s
    }
}

// ─────────────────────────────────────────────────────────────────────
// Pure renderers (deterministic; no `format!` of code)
// ─────────────────────────────────────────────────────────────────────

#[must_use]
pub fn clippy_format_ban_toml() -> String {
    // Per ★★ TYPED EMISSION directive — `format!()` is banned fleet-wide
    // in favor of typed Display / typed AST renderers.
    r#"# Generated by tatara-rust-repo.
# Per pleme-io ★★ TYPED EMISSION directive — every string we emit
# must come from a typed surface (Display impl, typed logging macro,
# typed AST renderer). `std::format!` is banned.
disallowed-macros = ["std::format"]
"#
    .to_string()
}

#[must_use]
pub fn rust_gitignore() -> String {
    r#"# Generated by tatara-rust-repo.
/target
/result
/result-*
Cargo.lock.bak
.direnv/
.envrc.local
"#
    .to_string()
}

#[must_use]
pub fn rust_toolchain_toml(channel: &str) -> String {
    format!(
        r#"[toolchain]
channel = "{channel}"
components = ["rustfmt", "clippy"]
"#
    )
}

fn render_mit(year: u32, holder: &str) -> String {
    format!(
        r#"MIT License

Copyright (c) {year} {holder}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
"#
    )
}

fn render_apache2(year: u32, holder: &str) -> String {
    format!(
        r#"                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

Copyright (c) {year} {holder}

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
"#
    )
}

fn render_mpl2(year: u32, holder: &str) -> String {
    format!(
        r#"Mozilla Public License Version 2.0
==================================

Copyright (c) {year} {holder}

This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
file, You can obtain one at https://mozilla.org/MPL/2.0/.
"#
    )
}

fn render_readme(spec: &ReadmeSpec, crate_name: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {}\n\n", spec.title));
    s.push_str(&format!("{}\n\n", spec.one_line));
    s.push_str("[![Build](https://github.com/");
    if let Some(slug) = github_slug(&spec.repo_url) {
        s.push_str(&slug);
    }
    s.push_str(
        "/actions/workflows/auto-release.yml/badge.svg)](#)\n",
    );
    s.push_str(&format!(
        "[![crates.io](https://img.shields.io/crates/v/{}.svg)](https://crates.io/crates/{})\n\n",
        crate_name, crate_name
    ));
    s.push_str("## Install\n\n```toml\n[dependencies]\n");
    s.push_str(&format!("{} = \"*\"\n", crate_name));
    s.push_str("```\n\n");
    if let Some(ex) = &spec.usage_example {
        s.push_str("## Usage\n\n");
        s.push_str(ex);
        s.push('\n');
    }
    s.push_str("## Generation\n\n");
    s.push_str(
        "This crate is mechanically emitted by [`tatara-rust-ast`](https://github.com/pleme-io/tatara-rust-ast). \
         The author surface is a typed `(defmacro …)` Spec — the proc-macro implementation, \
         tests, Nix flake, caixa wrapper, and CI workflow are all generated. \
         See the catalog at `catalog.json` in the parent registry.\n",
    );
    s
}

fn github_slug(url: &str) -> Option<String> {
    // Accept https://github.com/org/repo[.git] or git@github.com:org/repo[.git]
    let s = url.trim_end_matches(".git");
    if let Some(rest) = s.strip_prefix("https://github.com/") {
        Some(rest.to_string())
    } else if let Some(rest) = s.strip_prefix("git@github.com:") {
        Some(rest.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_ast::CrateScaffold;

    fn empty_scaffold(name: &str) -> CrateScaffold {
        let mut s = CrateScaffold::new(name, "0.1.0");
        s.add_file("Cargo.toml", format!("[package]\nname = \"{name}\"\n"));
        s.add_file("src/lib.rs", "// generated\n".to_string());
        s
    }

    #[test]
    fn defaults_attach_full_oss_surface() {
        let spec = RepoSpec::defaults_for(
            empty_scaffold("smoke"),
            "smoke",
            "https://github.com/pleme-io/smoke",
            "A test crate.",
        );
        let compiled = spec.compile();
        let files = compiled.to_files();
        for required in [
            "Cargo.toml",
            "src/lib.rs",
            "flake.nix",
            "caixa.lisp",
            ".github/workflows/auto-release.yml",
            "clippy.toml",
            "LICENSE",
            ".gitignore",
            "rust-toolchain.toml",
            "README.md",
        ] {
            assert!(files.contains_key(required), "missing {required}");
        }
    }

    #[test]
    fn license_renders_with_spdx_label() {
        let l = License::Mit {
            year: 2026,
            holder: "x".into(),
        };
        assert_eq!(l.spdx(), "MIT");
        assert!(l.render().contains("MIT License"));
        assert!(l.render().contains("2026 x"));
    }

    #[test]
    fn clippy_toml_bans_std_format() {
        assert!(clippy_format_ban_toml().contains(r#"disallowed-macros = ["std::format"]"#));
    }

    #[test]
    fn idempotent_on_pre_existing_files() {
        let mut sc = empty_scaffold("smoke");
        sc.add_file("LICENSE", "custom-license");
        sc.add_file("README.md", "custom-readme");
        let spec = RepoSpec::defaults_for(sc, "smoke", "https://github.com/x/y", "one-line");
        let f = spec.compile().to_files();
        assert_eq!(f["LICENSE"], "custom-license");
        assert_eq!(f["README.md"], "custom-readme");
    }

    #[test]
    fn without_flake_skips_flake() {
        let spec = RepoSpec::defaults_for(
            empty_scaffold("smoke"),
            "smoke",
            "https://github.com/x/y",
            "one-line",
        )
        .without_flake();
        let f = spec.compile().to_files();
        assert!(!f.contains_key("flake.nix"));
    }

    #[test]
    fn without_caixa_still_attaches_workflow_if_auto_release_on() {
        let spec = RepoSpec::defaults_for(
            empty_scaffold("smoke"),
            "smoke",
            "https://github.com/x/y",
            "one-line",
        )
        .without_caixa();
        let f = spec.compile().to_files();
        assert!(!f.contains_key("caixa.lisp"));
        assert!(f.contains_key(".github/workflows/auto-release.yml"));
    }

    #[test]
    fn readme_has_badges_and_install_block() {
        let spec = RepoSpec::defaults_for(
            empty_scaffold("foo-derive"),
            "foo-derive",
            "https://github.com/pleme-io/foo-derive",
            "Foo derive macro.",
        );
        let readme = spec.compile().to_files().get("README.md").unwrap().clone();
        assert!(readme.contains("# foo-derive"));
        assert!(readme.contains("crates.io/crates/foo-derive"));
        assert!(readme.contains("foo-derive = \"*\""));
    }

    #[test]
    fn github_slug_parses_both_url_shapes() {
        assert_eq!(
            github_slug("https://github.com/pleme-io/x.git"),
            Some("pleme-io/x".to_string())
        );
        assert_eq!(
            github_slug("git@github.com:pleme-io/x.git"),
            Some("pleme-io/x".to_string())
        );
        assert_eq!(github_slug("https://gitlab.com/x/y"), None);
    }

    #[test]
    fn serde_round_trip() {
        let spec = RepoSpec::defaults_for(
            empty_scaffold("foo-derive"),
            "foo-derive",
            "https://github.com/pleme-io/foo-derive",
            "Foo derive macro.",
        );
        let j = serde_json::to_string(&spec).unwrap();
        let back: RepoSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(back.scaffold.name, "foo-derive");
        assert!(back.flake);
        assert!(back.caixa.is_some());
    }
}
