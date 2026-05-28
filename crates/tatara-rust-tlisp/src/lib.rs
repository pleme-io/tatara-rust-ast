//! `tatara-rust-tlisp` — tlisp authoring surface for every L2 macro Spec.
//!
//! Two purposes:
//! 1. **Forward path** — write a `(defprocderive …)` / `(defprocattr …)` /
//!    `(defprocfn …)` / `(defmacrules …)` / `(defsuite …)` form in
//!    tlisp; this crate's [`render_tlisp`] turns each Spec back into
//!    that text shape (round-trip).
//! 2. **TataraDomain registration** — when tatara-lisp-derive lands as a
//!    dep, every Spec gets `#[derive(TataraDomain)]` + `#[tatara(keyword =
//!    "def…")]` in this crate. Authoring forms expand to the Spec at
//!    compile time.
//!
//! Today only the rendering half ships. The derive macros land as a
//! one-line addition per Spec once the upstream is reachable.

use tatara_rust_ast::Ident;
use tatara_rust_derive::ProcDeriveSpec;
use tatara_rust_macro_rules::{MacroArm, MacroRulesSpec};
use tatara_rust_proc_attr::{AttrTransform, ProcAttrSpec};
use tatara_rust_proc_fn::{FnTransform, ProcFnSpec};
use tatara_rust_suite::{MacroMemberSpec, MacroSuiteSpec};

pub mod catalog;
pub use catalog::{ParseError, SExpr, parse_macrocatalog, parse_sexprs, render_macrocatalog};

/// Render any Spec to its canonical tlisp `(def<kind> …)` form. The
/// returned text is what the operator authors today in a `.tlisp` file.
pub trait RenderTlisp {
    fn render_tlisp(&self) -> String;
}

impl RenderTlisp for ProcDeriveSpec {
    fn render_tlisp(&self) -> String {
        format!(
            "(defprocderive\n  :trait-name \"{}\"\n  :items {})",
            self.trait_name.0,
            self.impl_template.items.len()
        )
    }
}

impl RenderTlisp for ProcAttrSpec {
    fn render_tlisp(&self) -> String {
        let transform = match &self.transform {
            AttrTransform::PrependPrelude { prelude_tokens } => format!(
                "(:kind \"prepend-prelude\" :prelude {prelude_tokens:?})"
            ),
        };
        format!(
            "(defprocattr\n  :macro-name \"{}\"\n  :transform {transform})",
            self.macro_name.0
        )
    }
}

impl RenderTlisp for ProcFnSpec {
    fn render_tlisp(&self) -> String {
        let transform = match &self.transform {
            FnTransform::PrependPrelude { prelude_tokens } => format!(
                "(:kind \"prepend-prelude\" :prelude {prelude_tokens:?})"
            ),
        };
        format!(
            "(defprocfn\n  :macro-name \"{}\"\n  :transform {transform})",
            self.macro_name.0
        )
    }
}

impl RenderTlisp for MacroRulesSpec {
    fn render_tlisp(&self) -> String {
        let arms = self
            .arms
            .iter()
            .map(|a| format!("    (:matcher {:?} :transcriber {:?})", a.matcher, a.transcriber))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "(defmacrules\n  :macro-name \"{}\"\n  :arms (\n{arms}))",
            self.macro_name.0
        )
    }
}

impl RenderTlisp for MacroSuiteSpec {
    fn render_tlisp(&self) -> String {
        let members = self
            .members
            .iter()
            .map(|m| {
                let body = match m {
                    MacroMemberSpec::Derive { crate_name, spec } => format!(
                        "(:kind \"derive\" :crate-name {crate_name:?} :spec {})",
                        spec.render_tlisp()
                    ),
                    MacroMemberSpec::ProcAttr { crate_name, spec } => format!(
                        "(:kind \"proc-attr\" :crate-name {crate_name:?} :spec {})",
                        spec.render_tlisp()
                    ),
                    MacroMemberSpec::ProcFn { crate_name, spec } => format!(
                        "(:kind \"proc-fn\" :crate-name {crate_name:?} :spec {})",
                        spec.render_tlisp()
                    ),
                    MacroMemberSpec::MacroRules { crate_name, spec } => format!(
                        "(:kind \"macro-rules\" :crate-name {crate_name:?} :spec {})",
                        spec.render_tlisp()
                    ),
                };
                format!("    {body}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "(defsuite\n  :workspace-name \"{}\"\n  :members (\n{members}))",
            self.workspace_name
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_renders() {
        let s = ProcDeriveSpec::new("Marker", vec![]);
        let txt = s.render_tlisp();
        assert!(txt.contains("(defprocderive"));
        assert!(txt.contains(":trait-name \"Marker\""));
    }

    #[test]
    fn proc_attr_renders() {
        let s = ProcAttrSpec {
            macro_name: Ident::new("instrumented"),
            transform: AttrTransform::PrependPrelude {
                prelude_tokens: "#[inline]".into(),
            },
        };
        let txt = s.render_tlisp();
        assert!(txt.contains("(defprocattr"));
        assert!(txt.contains(":macro-name \"instrumented\""));
        assert!(txt.contains("prepend-prelude"));
    }

    #[test]
    fn proc_fn_renders() {
        let s = ProcFnSpec {
            macro_name: Ident::new("say_hi"),
            transform: FnTransform::PrependPrelude {
                prelude_tokens: "println!(\"hi\");".into(),
            },
        };
        let txt = s.render_tlisp();
        assert!(txt.contains("(defprocfn"));
        assert!(txt.contains(":macro-name \"say_hi\""));
    }

    #[test]
    fn macro_rules_renders_with_arms() {
        let s = MacroRulesSpec {
            macro_name: Ident::new("identity"),
            arms: vec![MacroArm {
                matcher: "($x:expr)".into(),
                transcriber: "{ $x }".into(),
            }],
        };
        let txt = s.render_tlisp();
        assert!(txt.contains("(defmacrules"));
        assert!(txt.contains(":matcher"));
        assert!(txt.contains(":transcriber"));
    }

    #[test]
    fn suite_renders_with_members() {
        let suite = MacroSuiteSpec {
            workspace_name: "my-macros".into(),
            members: vec![MacroMemberSpec::Derive {
                crate_name: "marker-derive".into(),
                spec: ProcDeriveSpec::new("Marker", vec![]),
            }],
        };
        let txt = suite.render_tlisp();
        assert!(txt.contains("(defsuite"));
        assert!(txt.contains(":workspace-name \"my-macros\""));
        assert!(txt.contains(":kind \"derive\""));
        assert!(txt.contains("marker-derive"));
    }
}
