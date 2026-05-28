//! Canonical Cargo.toml renderers for emitted proc-macro crates.
//!
//! Every L2 emitter (per-field / per-variant / newtype / enum-fold /
//! proc-attr / proc-fn / macro-rules / composite) needs the same
//! `[package]` + `[lib] proc-macro=true` + `[dependencies]
//! proc-macro2/quote/syn` shape. This module is the single source of
//! truth so a fleet-wide change (toolchain bump, dep version pin,
//! description format) lands in one place.

/// Render a proc-macro crate's `Cargo.toml`. The only per-emitter
/// variation is `description`; everything else is fixed by fleet
/// policy (edition 2024, MIT, syn 2 with `full` + `extra-traits`).
#[must_use]
pub fn render_proc_macro_cargo_toml(crate_name: &str, description: &str) -> String {
    format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2024"
license = "MIT"
description = "{description}"

[lib]
proc-macro = true

[dependencies]
proc-macro2 = "1"
quote = "1"
syn = {{ version = "2", features = ["full", "extra-traits"] }}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_every_required_section() {
        let toml = render_proc_macro_cargo_toml("foo-derive", "Foo desc.");
        // Sections every fleet proc-macro crate must carry.
        assert!(toml.contains("[package]"));
        assert!(toml.contains(r#"name = "foo-derive""#));
        assert!(toml.contains(r#"version = "0.1.0""#));
        assert!(toml.contains(r#"edition = "2024""#));
        assert!(toml.contains(r#"license = "MIT""#));
        assert!(toml.contains(r#"description = "Foo desc.""#));
        assert!(toml.contains("[lib]"));
        assert!(toml.contains("proc-macro = true"));
        assert!(toml.contains("[dependencies]"));
        assert!(toml.contains(r#"proc-macro2 = "1""#));
        assert!(toml.contains(r#"quote = "1""#));
        assert!(toml.contains(r#"syn = { version = "2", features = ["full", "extra-traits"] }"#));
    }

    #[test]
    fn description_is_quoted_correctly() {
        // Descriptions are emitted verbatim; the helper does not escape
        // embedded quotes (callers control the text).
        let toml = render_proc_macro_cargo_toml("x", "A description with a comma, period.");
        assert!(toml.contains(r#"description = "A description with a comma, period.""#));
    }

    #[test]
    fn output_is_byte_stable_for_a_given_input() {
        let a = render_proc_macro_cargo_toml("x", "y");
        let b = render_proc_macro_cargo_toml("x", "y");
        assert_eq!(a, b);
    }
}
