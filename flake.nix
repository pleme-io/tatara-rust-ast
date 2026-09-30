{
  description = "tatara-rust-ast — typed Rust AST primitives, emission, and the macro farm built on them";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    crate2nix.url = "github:nix-community/crate2nix";
    flake-utils.url = "github:numtide/flake-utils";
    substrate = {
      url = "github:pleme-io/substrate";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    fenix.follows = "substrate/fenix";
  };

  # The package is the emitter's CLI (typed AST as JSON in, Rust out), the
  # door a compiler outside Rust uses; the libraries publish to crates.io.
  # The release gate (substrate's rust-auto-release) runs the tests in this
  # flake's devShell; without a flake it failed at setup on every tick since
  # 0.1.7, so nothing after 0.1.7 reached crates.io.
  outputs = { self, nixpkgs, crate2nix, flake-utils, substrate, fenix }:
    (import "${substrate}/lib/rust-workspace-release-flake.nix" {
      inherit nixpkgs crate2nix flake-utils fenix;
    }) {
      toolName = "tatara-rust-emit";
      packageName = "tatara-rust-emit";
      src = self;
      repo = "pleme-io/tatara-rust-ast";
    };
}
