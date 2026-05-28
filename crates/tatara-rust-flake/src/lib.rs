//! `tatara-rust-flake` — substrate-shaped flake emission for generated crates.
//!
//! Single typed primitive: takes a [`CrateScaffold`] (any emitted macro
//! crate) and decorates it with a `flake.nix` so consumers can
//! `nix build .` / `nix run github:owner/repo` without hand-writing
//! the flake.
//!
//! The flake uses substrate's canonical `mkRustToolFlake { src = ./.; }`
//! shape (zero-argument; reads `Cargo.toml` to derive toolName + repo).
//! Per the pleme-io substrate.rust.* surface.

use tatara_rust_ast::CrateScaffold;

/// Decorate `scaffold` with a substrate-shaped `flake.nix`. Idempotent —
/// if the scaffold already has a `flake.nix`, this is a no-op.
pub fn attach_substrate_flake(scaffold: &mut CrateScaffold) {
    if scaffold.files.iter().any(|f| f.path == "flake.nix") {
        return;
    }
    scaffold.add_file("flake.nix", canonical_flake());
}

/// The canonical 3-line substrate flake. Reads `Cargo.toml` to derive
/// everything else.
#[must_use]
pub fn canonical_flake() -> String {
    r#"{
  inputs.substrate.url = "github:pleme-io/substrate";
  outputs = inputs: inputs.substrate.mkRustToolFlake {
    inherit inputs;
    src = ./.;
  };
}
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_ast::CrateScaffold;

    #[test]
    fn attaches_flake() {
        let mut s = CrateScaffold::new("foo", "0.1.0");
        s.add_file("Cargo.toml", "[package]\nname = \"foo\"\n");
        attach_substrate_flake(&mut s);
        let files = s.to_files();
        assert!(files.contains_key("flake.nix"));
        assert!(files["flake.nix"].contains("substrate.mkRustToolFlake"));
    }

    #[test]
    fn idempotent() {
        let mut s = CrateScaffold::new("foo", "0.1.0");
        s.add_file("flake.nix", "custom");
        attach_substrate_flake(&mut s);
        // The custom flake.nix survives.
        assert_eq!(s.to_files()["flake.nix"], "custom");
    }

    #[test]
    fn canonical_flake_uses_substrate() {
        let f = canonical_flake();
        assert!(f.contains("inputs.substrate.url"));
        assert!(f.contains("github:pleme-io/substrate"));
        assert!(f.contains("mkRustToolFlake"));
        assert!(f.contains("src = ./."));
    }
}
