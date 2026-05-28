//! Golden snapshots of the canonical example Specs.
//!
//! Each test renders an example Spec, snapshots every file the
//! scaffold emits, and persists the result under `tests/snapshots/`
//! (insta's default location, resolved relative to *this* file
//! because `snapshot_spec!` expands at the call site). Drift in
//! any emitter surfaces as a snapshot diff during `cargo test`.

use tatara_rust_examples::{getter_all_spec, setter_all_spec, with_builder_spec};
use tatara_rust_snapshot::snapshot_spec;

#[test]
fn snapshot_getter_all() {
    snapshot_spec!(&getter_all_spec(), "getter-all-derive", "getter_all");
}

#[test]
fn snapshot_with_builder() {
    snapshot_spec!(&with_builder_spec(), "with-builder-derive", "with_builder");
}

#[test]
fn snapshot_setter_all() {
    snapshot_spec!(&setter_all_spec(), "setter-all-derive", "setter_all");
}
