# Diminishing-returns metric — when to stop farming

> **Status:** operational. Implemented in `tatara-rust-survey::returns`,
> exposed as `tatara-rust-forge survey-fleet-returns <org-root>`.
> First fleet measurement: 2026-06-03 over 228 public first-party
> pleme-io Rust libraries.

The macro farm answers *what* mechanical impl shapes to lift into the
emitter substrate. This metric answers the orthogonal question every
refactoring campaign must eventually answer honestly: **when do I
stop?** Without it, "refactor for maximal reuse" has no terminating
condition and degenerates into busywork against ever-smaller clusters.

## The model

Every repeating-impl pattern is scored:

```
benefit(P) = candidates(P) × loc_per_candidate     (toil deleted)
cost(P)    = lift_cost(readiness(P))                (one-time substrate)
roi(P)     = benefit(P) / cost(P)

decision(P):
  benefit < MIN_BENEFIT   → Defer    (cluster too small to churn repos)
  roi     < MIN_ROI       → Stop     (building it costs more than it saves)
  else                    → Harvest  (adopt now)

verdict(fleet):
  any Harvest             → ContinueFarming
  none                    → Plateau  (stop refactoring; pivot to docs/codegen)
```

`lift_cost` keys on **substrate readiness** — the dominant cost term:

| Readiness | Means | Default cost (toil-equiv LOC) |
|---|---|---|
| `Adoptable` | detector + emitter both ship → just run `survey-fleet-apply` | 8 |
| `NeedsDetector` | derive published, but no survey detector | 8 + 60 |
| `NeedsEmitter` | brand-new spec kind + detector | 8 + 60 + 140 |

Defaults (`adopt=8 detector=60 emitter=140 min_benefit=30 min_roi=1.0`)
are grounded in the farm's own authoring history (see
[`macro-farm.md`](https://github.com/pleme-io/blackmatter-pleme/blob/main/docs/pleme-io-docs/macro-farm.md),
the mado dogfood loop) and are tunable on `LiftCostModel`.

The metric reasons about **two** populations so it can decide *not* to
build something:

1. **Adoptable** patterns — measured exactly by the typed survey.
2. **Frontier** clusters — shapes the survey can't see yet (no detector,
   maybe no emitter). The operator supplies an estimate; the model prices
   the build against it. This is where the metric earns its keep.

## The 2026-06-03 measurement

`survey-fleet-returns` over the 228 public first-party libs:

| pattern | readiness | candidates | loc | cost | roi | decision |
|---|---|---:|---:|---:|---:|---|
| WithBuilder | Adoptable | 18 | 255 | 8 | 31.9 | **Harvest** |
| IsVariant | Adoptable | 17 | 225 | 8 | 28.1 | **Harvest** |
| GetterAll | Adoptable | 12 | 130 | 8 | 16.3 | **Harvest** |
| AllVariants | Adoptable | 19 | 95 | 8 | 11.9 | **Harvest** |
| AsRefNewtype | NeedsDetector | 0 | 0 | 68 | 0 | Defer |
| DerefNewtype | NeedsDetector | 0 | 0 | 68 | 0 | Defer |
| DisplayNewtype | NeedsDetector | 0 | 0 | 68 | 0 | Defer |
| ImplFrom | NeedsDetector | 0 | 0 | 68 | 0 | Defer |
| StringEnum | NeedsEmitter | 0 | 0 | 208 | 0 | Defer |

**Verdict: ContinueFarming** — ~705 addressable LOC across 4 patterns.

### Two findings that made the metric honest

1. **The newtype frontier is empty.** A precise body-shape census
   (`fn as_ref(&self) -> &Inner { &self.0 }` and friends) over all 1790
   first-party source files found **zero** mechanical newtype
   delegations. The 2026-05-28 whole-fleet survey's large numbers (840
   `From`, 414 `FromStr`+`Display`) were measured across private +
   vendored code and do **not** hold in the public first-party scope.
   The metric therefore prices every `NeedsDetector`/`NeedsEmitter`
   newtype/string cluster at `Defer` — **authoring those detectors would
   harvest nothing.** This is a decision *not* to build, made with a
   number instead of a hunch.

2. **Addressable ≠ realized.** A live `survey-apply-all` on shikumi
   applied all 20 of its candidates, but the cargo gate caught 102
   semantic errors (its enums are embedded in trait machinery — a
   `const ALL = Self::ALL` cycle, E0391) and **rolled every file back**.
   The survey's optimistic candidate count is the *addressable* market;
   the *realized* harvest is whatever survives the per-crate cargo gate.
   `survey-fleet-validate` only checks syntactic re-parse — semantic
   safety requires the gate. The W12 atomic pipeline makes this safe:
   an optimistic candidate that doesn't compile is never landed.

## The stop decision

- **Now:** ContinueFarming. The 4 Adoptable patterns are harvestable via
  `survey-fleet-apply` (per-crate, atomic, gate-checked, rollback on red).
  Realized gains are bounded by each crate's cargo gate.
- **After harvest:** re-running the metric drops those four to ~0 →
  **Plateau**. There is no positive-ROI mechanical refactoring left in
  the public first-party Rust surface; the macro farm has already
  absorbed it.
- **The frontier stays Deferred** until a future fleet census shows a
  cluster large enough to repay a new detector/emitter. Re-measure;
  don't guess.

**Therefore: refactoring is at its diminishing-returns boundary.** The
compounding move is no longer "adopt more derives" — it is to make the
substrate *self-documenting* so every future crate consumes the derives
as it is written. See `tatara-rust-docs` and
[`docs/derives-reference.md`](./derives-reference.md).
