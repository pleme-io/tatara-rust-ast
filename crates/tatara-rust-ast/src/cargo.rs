//! Canonical Cargo.toml renderers for emitted proc-macro crates.
//!
//! Every L2 emitter (per-field / per-variant / newtype / enum-fold /
//! proc-attr / proc-fn / macro-rules / composite) needs the same
//! `[package]` + `[lib] proc-macro=true` + `[dependencies]
//! proc-macro2/quote/syn` shape. This module is the single source of
//! truth so a fleet-wide change (toolchain bump, dep version pin,
//! description format) lands in one place.
//!
//! **★★ TYPED EMISSION.** The manifest is built as a typed
//! [`CargoManifest`] value and serialized via `toml::to_string` — never
//! composed as a `format!()` of TOML syntax. See [`crate::cargo_manifest`].

use crate::cargo_manifest::{CargoManifest, Dep, DetailedDep, Lib, Package};

/// Render a proc-macro crate's `Cargo.toml`. The only per-emitter
/// variation is `description`; everything else is fixed by fleet
/// policy (edition 2024, MIT, syn 2 with `full` + `extra-traits`).
///
/// # Panics
/// Never in practice — the typed manifest has only string keys, so
/// `toml::to_string` cannot fail. The `expect` documents that invariant.
#[must_use]
pub fn render_proc_macro_cargo_toml(crate_name: &str, description: &str) -> String {
    let mut m = CargoManifest::new(
        Package {
            name: crate_name.to_string(),
            version: "0.1.0".into(),
            edition: "2024".into(),
            license: "MIT".into(),
            description: description.to_string(),
        },
        Some(Lib {
            proc_macro: Some(true),
            path: None,
        }),
    );
    m.add_dep("proc-macro2", Dep::Simple("1".into()));
    m.add_dep("quote", Dep::Simple("1".into()));
    m.add_dep(
        "syn",
        Dep::Detailed(DetailedDep::versioned(
            "2",
            vec!["full".into(), "extra-traits".into()],
        )),
    );
    m.to_toml()
        .expect("proc-macro Cargo manifest is all-string keys; toml::to_string cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_every_required_section() {
        let rendered = render_proc_macro_cargo_toml("foo-derive", "Foo desc.");
        // Parse the emitted TOML into a typed value, then assert on the
        // structure — never substring-match the syntax (★★ TYPED EMISSION).
        let t: toml::Table = toml::from_str(&rendered).unwrap();
        assert_eq!(t["package"]["name"].as_str(), Some("foo-derive"));
        assert_eq!(t["package"]["version"].as_str(), Some("0.1.0"));
        assert_eq!(t["package"]["edition"].as_str(), Some("2024"));
        assert_eq!(t["package"]["license"].as_str(), Some("MIT"));
        assert_eq!(t["package"]["description"].as_str(), Some("Foo desc."));
        assert_eq!(t["lib"]["proc-macro"].as_bool(), Some(true));
        assert_eq!(t["dependencies"]["proc-macro2"].as_str(), Some("1"));
        assert_eq!(t["dependencies"]["quote"].as_str(), Some("1"));
        assert_eq!(t["dependencies"]["syn"]["version"].as_str(), Some("2"));
        let feats = t["dependencies"]["syn"]["features"].as_array().unwrap();
        let feats: Vec<&str> = feats.iter().filter_map(toml::Value::as_str).collect();
        assert_eq!(feats, vec!["full", "extra-traits"]);
    }

    #[test]
    fn description_is_carried_verbatim() {
        let rendered =
            render_proc_macro_cargo_toml("x", "A description with a comma, period.");
        let t: toml::Table = toml::from_str(&rendered).unwrap();
        assert_eq!(
            t["package"]["description"].as_str(),
            Some("A description with a comma, period.")
        );
    }

    #[test]
    fn output_is_byte_stable_for_a_given_input() {
        let a = render_proc_macro_cargo_toml("x", "y");
        let b = render_proc_macro_cargo_toml("x", "y");
        assert_eq!(a, b);
    }
}
