//! `ClosedAxisSpec` — emit `#[derive(ClosedAxis)]` for a unit enum:
//! the inherent `const ALL: &'static [Self]` PLUS an
//! `impl shikumi::ClosedAxis` referencing the same slice.
//!
//! The farm already ships `allvariants` (an [`crate::EnumFoldDeriveSpec`]
//! emitting an inherent `Self::ALL`). `ClosedAxis` is *allvariants
//! targeting the shikumi trait* (plan F2): a unit enum gains both the
//! inherent `ALL` and `impl shikumi::ClosedAxis for Self`, so it plugs
//! straight into `shikumi::axis_iter` / `axis_cardinality` /
//! `ProductCube` with zero hand-written `ALL`.
//!
//! Survey signal (pleme-io 2026-05-31, mado): six closed config enums —
//! `CursorStyle`, `TearMode`, `TearRuntime`, `MouseShiftCapture`,
//! `ColorblindMode`, `QuickTerminalEdge` — each need a hand-written
//! `const ALL` to enter the config-permutation cube today. One derive
//! collapses them; this is also the fleet primitive every shikumi
//! `TieredConfig` consumer (tear/frost/tend/kikai/kurage/namimado)
//! wants.
//!
//! **No duplicated emitter.** Per the ★★ EMITTER SUBSTRATE rule
//! (solve once, don't duplicate), `ClosedAxisSpec` is a thin typed
//! border that *delegates* its `compile_to_crate` to an internally
//! constructed `EnumFoldDeriveSpec` whose `fold_template` emits the two
//! items. The typed `ClosedAxisSpec` exists so the catalog gets a
//! named, validated, verifier-dispatchable surface; the proven EnumFold
//! emitter does the byte-level work.
//!
//! Authoring shape:
//!
//! ```ignore
//! // canonical: trait path `shikumi::ClosedAxis`, derive `ClosedAxis`:
//! ClosedAxisSpec::canonical()
//! // custom trait path (e.g. a re-export):
//! ClosedAxisSpec { trait_name: "ClosedAxis".into(),
//!     axis_trait_path: "::my_crate::Axis".into() }
//! ```
//!
//! Consumer (the enum must `#[derive(Clone, Copy, PartialEq, Eq, Hash)]`
//! to satisfy the `shikumi::ClosedAxis: Copy + Eq + Hash + 'static`
//! bound):
//!
//! ```ignore
//! #[derive(Clone, Copy, PartialEq, Eq, Hash, ClosedAxis)]
//! enum CursorStyle { Block, Bar, Underline }
//! assert_eq!(CursorStyle::ALL.len(), 3);
//! assert_eq!(shikumi::axis_cardinality::<CursorStyle>(), 3);
//! ```

use serde::{Deserialize, Serialize};
use tatara_rust_ast::{AstError, CompileToCrate, CrateScaffold, Ident};

use crate::{EnumFoldDeriveSpec, EnumFoldTarget};

/// Typed spec for a `#[derive(ClosedAxis)]` over a **unit-variant** enum.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClosedAxisSpec {
    /// `#[derive(<trait_name>)]` user-facing identifier (e.g. `ClosedAxis`).
    pub trait_name: Ident,
    /// Fully-qualified path to the `ClosedAxis` trait the emitted impl
    /// targets — `::shikumi::ClosedAxis` by default. Substituted verbatim
    /// into the emitted `impl <path> for Self` head.
    pub axis_trait_path: String,
}

impl Default for ClosedAxisSpec {
    fn default() -> Self {
        Self::canonical()
    }
}

impl ClosedAxisSpec {
    /// Fleet-canonical spec: derive `ClosedAxis`, target
    /// `::shikumi::ClosedAxis`.
    #[must_use]
    pub fn canonical() -> Self {
        Self {
            trait_name: Ident::new("ClosedAxis"),
            axis_trait_path: "::shikumi::ClosedAxis".into(),
        }
    }

    /// The internal `EnumFoldDeriveSpec` this spec delegates emission to.
    /// `fold_template` emits BOTH the inherent `const ALL` (so a consumer
    /// can name `Self::ALL` directly) AND the `impl <trait> for Self`
    /// with the trait-associated `ALL` bound to the same slice. `#fold`
    /// (replaced with the comma-joined per-variant fragments by the
    /// EnumFold emitter) appears twice — both `&[…]` slices enumerate
    /// every unit variant in declaration order.
    fn as_enum_fold(&self) -> EnumFoldDeriveSpec {
        EnumFoldDeriveSpec {
            trait_name: self.trait_name.clone(),
            target: EnumFoldTarget::UnitVariantsOnly,
            per_variant_fragment: "Self::#variant_name".into(),
            fold_template: format!(
                "impl #self_name {{ pub const ALL: &'static [Self] = &[#fold]; }} \
                 impl {path} for #self_name {{ const ALL: &'static [Self] = &[#fold]; }}",
                path = self.axis_trait_path
            ),
        }
    }
}

impl CompileToCrate for ClosedAxisSpec {
    fn compile_to_crate(&self, crate_name: &str) -> Result<CrateScaffold, AstError> {
        // Delegate to the proven EnumFold emitter — no second emission path.
        self.as_enum_fold().compile_to_crate(crate_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tatara_rust_snapshot::assert_tokens_contain;

    #[test]
    fn canonical_targets_shikumi_closed_axis() {
        let s = ClosedAxisSpec::canonical();
        assert_eq!(s.trait_name.0, "ClosedAxis");
        assert_eq!(s.axis_trait_path, "::shikumi::ClosedAxis");
        assert_eq!(ClosedAxisSpec::default(), s);
    }

    #[test]
    fn compiles_to_cargo_and_lib() {
        let s = ClosedAxisSpec::canonical()
            .compile_to_crate("pleme-closedaxis-derive")
            .unwrap();
        let files = s.to_files();
        assert!(files.contains_key("Cargo.toml"));
        assert!(files.contains_key("src/lib.rs"));
        // The delegated EnumFold emitter produces a proc-macro crate.
        assert!(files.get("Cargo.toml").unwrap().contains("proc-macro = true"));
    }

    #[test]
    fn emits_both_inherent_all_and_trait_impl() {
        let s = ClosedAxisSpec::canonical()
            .compile_to_crate("pleme-closedaxis-derive")
            .unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // Token-subsequence positives per the ★★ TOKEN-STABILITY directive.
        assert_tokens_contain!(&lib, "#[proc_macro_derive(ClosedAxis)]");
        // Inherent ALL.
        assert_tokens_contain!(&lib, "pub const ALL: &'static [Self]");
        // The trait impl head targets the shikumi path. The fold emitter
        // emits the path as quote tokens, so the leading `::` survives.
        assert_tokens_contain!(&lib, "impl ::shikumi::ClosedAxis for");
        // EnumFold's #fold substitution must have fired (no literal #fold).
        assert!(!lib.contains("&[#fold]"), "literal #fold leaked");
    }

    #[test]
    fn unit_only_guard_present() {
        let s = ClosedAxisSpec::canonical()
            .compile_to_crate("c")
            .unwrap();
        let lib = s.to_files().get("src/lib.rs").unwrap().clone();
        // ClosedAxis is unit-variants-only (a tuple variant has no
        // canonical `Self::Variant` constructor for the ALL slice).
        assert!(
            lib.contains("EnumFoldDerive (unit-only)"),
            "unit-only guard missing"
        );
    }

    #[test]
    fn custom_trait_path_substitutes() {
        let spec = ClosedAxisSpec {
            trait_name: Ident::new("Axis"),
            axis_trait_path: "::my_crate::axes::Axis".into(),
        };
        let lib = spec
            .compile_to_crate("c")
            .unwrap()
            .to_files()
            .get("src/lib.rs")
            .unwrap()
            .clone();
        assert_tokens_contain!(&lib, "#[proc_macro_derive(Axis)]");
        assert_tokens_contain!(&lib, "impl ::my_crate::axes::Axis for");
        assert!(!lib.contains("shikumi"), "default path leaked under override");
    }

    #[test]
    fn serde_round_trip() {
        let spec = ClosedAxisSpec::canonical();
        let j = serde_json::to_string(&spec).unwrap();
        let back: ClosedAxisSpec = serde_json::from_str(&j).unwrap();
        assert_eq!(spec, back);
    }
}
