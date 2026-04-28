# PR review — card 0008 statistical marks (fresh independent pass)

- **Date:** 2026-04-28
- **Branch:** rally/statistical-marks (HEAD 4ceae91)
- **Base:** main (901c3c7)
- **Commits:** eeabf7e (feat), b2cdeea (orbit chore), 4ceae91 (drive complete chore)
- **Diff:** 18 files, +2510 / -32
- **Reviewer:** Claude (orthogonal pass — separate context from drive sub-agent)
- **Mode:** read-only review; no code modifications

## Verdict

**Verdict:** REQUEST_CHANGES

The implementation is structurally sound, the build is clean, and 389/389 tests pass (25 gomb_ tests for this slice). The core contribution — `QueryPlan::AggregateScalar`, the registry-based mark dispatch, the KDE/regression math, and the SQL cache — is well thought out and reasonably tested. However, several spec deviations and test-quality gaps weaken the verification evidence the PR claims to provide. None are catastrophic, and most are easy to address. The current "drive complete" status overstates the conformance level.

Independently verified:
- `cargo build --workspace` — clean (no warnings, no errors).
- `cargo test --workspace` — 389 passed across 27 suites in ~2.11s.
- `cargo test --workspace gomb_` — 25 passed, 364 filtered out.

## Findings

### HIGH — must address before ship

1. **HIGH: gomb_ac12 does not exercise the property the spec verifies.**
   The spec acceptance criterion is "param mutation that does not change SQL string keeps the cached SQL slot warm — verify via `propagate_param("bandwidth", v)` followed by re-execution." The test in `crates/brightfield-engine/src/lib.rs` (gomb_ac12) calls `test_execute_emitted` three times with the same SQL string and asserts the cache size stays 1. It never invokes `propagate_param`, never mutates a runtime param, and therefore does not demonstrate the cache-warm-on-param-change behaviour the spec promises. The current test passes trivially because identical SQL strings collide on the same cache key by construction. Fix: drive the cache through the runtime param coordinator the way the spec specifies, or downgrade the AC text.

2. **HIGH: gomb_ac11 LRU eviction test does not verify which entry was evicted.**
   In `crates/brightfield-engine/src/lib.rs` (gomb_ac11), the test inserts 33 entries into a cap-32 cache and asserts the cache size stays at 32 and that the duckdb execute count is 33. It never asserts that the *least-recently-used* entry was the one evicted. A FIFO, random, or MRU policy would also pass this test. The cache code does implement LRU (move-on-touch in the underlying `LinkedHashMap` / equivalent), but the test does not lock that behaviour in. Fix: after eviction, re-insert one of the early keys and assert it triggers a full duckdb execute (i.e. it was evicted), while a recently-touched key short-circuits.

3. **HIGH: `count_scene_fills` is a stub that always returns 0.**
   `crates/brightfield-render/src/mark.rs` defines `count_scene_fills` returning `0` with a comment that Vello does not expose draw-count introspection. Spec ACs gomb_ac03 and gomb_ac04 ("renderer emits ≥1 fill for density" / "≥1 line + ≥1 fill for regression band") were intended to assert against this counter. Because the stub returns 0, those assertions are either disabled, weakened to `>= 0`, or skipped. This is the single largest gap between claimed and actual rendering verification. Fix: either swap to a Scene-walking visitor that counts `Fill` and `Stroke` segments, or call out the limitation explicitly in the spec and downgrade the AC to "renderer runs without panicking and produces a Scene with non-empty bounds."

4. **HIGH: Density default `n_bins` diverges from spec (32 vs 100).**
   Spec line 355 says density evaluation grid defaults to 100 bins per axis. `crates/brightfield-sql/src/lower.rs` `DensityLowerer` uses 32. The chosen value is reasonable for performance, but it is a silent contract drift; conformance fixtures and the regression baseline will be tuned against 32 bins, making downstream snapshot comparisons against any spec-faithful reference impossible. Fix: either change the constant to 100 to match spec, or update the spec + decisions.md to record the new default and the reasoning.

5. **HIGH: Regression x-mean column is `mean_x`, spec mandates `x_bar`.**
   Spec section "regression scalar projection" requires `regr_avgx(...) AS x_bar`. `lower.rs` (RegressionLowerer) emits `AS mean_x`, and `mark.rs` (RegressionRenderer) consumes the same `mean_x` alias. The lowerer + renderer are internally consistent so nothing breaks at runtime, but any external consumer (e.g. a Mosaic-spec round-trip, conformance corpus, or downstream tooling diffing emitted SQL) that expects `x_bar` will mismatch. Fix: rename to `x_bar` to honour the spec contract, or amend the spec.

### MEDIUM — should address

6. **MEDIUM: Density renderers ignore user-supplied `bandwidth`.**
   `Density1DRenderer` and `Density2DRenderer` always call `silverman_1d` / `silverman_2d_per_axis`, never reading the `bandwidth` option that `parse.rs` LIFT_SURFACE_FIELDS goes out of its way to lift. Spec lists `bandwidth` as a first-class mark option. As wired, a user-specified bandwidth is silently discarded. Fix: read `mark.options.bandwidth` (numeric or "scott"/"silverman" enum) and only fall back to Silverman when absent.

7. **MEDIUM: Density2D encoding uses alpha-modulation instead of radius scaling.**
   Spec section "density 2D surface" describes per-cell radius proportional to density. `Density2DRenderer` keeps a fixed cell radius and varies alpha. The visual result is similar but the encoding semantics differ (radius is area-proportional; alpha is a perceptual luminance ramp that does not preserve area). Fix: either implement radius-scaled rendering or update the spec.

8. **MEDIUM: Regression CI band uses z-table critical value (1.96) not Student-t.**
   `mark.rs` `RegressionRenderer` computes `se(ŷ|x) · 1.96`. For n ≥ 30 the difference is negligible, but the curated regression fixture (Anscombe set) has n = 11 where t(0.025, 9) ≈ 2.262. The 95% claim is therefore ~13% too narrow on the canonical test fixture. Fix: add a small `t_critical(n - 2)` table or use a quantile approximation; alternatively document the z-approximation in decisions.md.

9. **MEDIUM: gomb_ac13 SQL "snapshots" are substring assertions, not on-disk snapshots.**
   `crates/brightfield-sql/src/emit.rs` gomb_ac13 tests assert `assert!(sql.contains("regr_slope"))` and similar. The spec calls these "conformance snapshots" implying golden files. The current form will not catch reordering, alias renames (see finding 5), or accidental column additions. Fix: write the rendered SQL to a `tests/snapshots/` directory and use `insta` (already in workspace) or a pinned `assert_eq!` against a checked-in string.

10. **MEDIUM: Regression silently skips when n < 3.**
    `RegressionRenderer` returns early without rendering when `regr_count < 3`. Spec says: draw the line if n ≥ 2, suppress only the CI band when degrees-of-freedom is insufficient. Current behaviour drops both. Fix: render the regression line for n ≥ 2 and only gate the CI band on n ≥ 3.

11. **MEDIUM: `bin_size` derivation assumes uniformly-spaced KDE grid.**
    `Density1DRenderer` computes `bin_size = pairs[1].0 - pairs[0].0` and treats it as the width for every bar. This is correct *only* if the grid is uniform, which is true today but is an undocumented invariant. Fix: either (a) compute width per-bar from neighbour positions, or (b) add a debug assertion and a comment that the grid is uniform by construction.

### LOW — polish

12. **LOW: `silverman_axis` duplicates `silverman_1d`.**
    `crates/brightfield-render/src/kde.rs` defines `silverman_axis` (used by `silverman_2d_per_axis`) with a body that is a near-clone of `silverman_1d` modulo the `n^(-1/6)` exponent. Fix: take the exponent as a parameter and collapse to one implementation.

13. **LOW: Density2DRenderer uses O(n²) linear search for grid centre lookup.**
    `mark.rs` Density2DRenderer iterates the entire pairs vector to find each cell centre. Fine for the default 32×32 grid but quadratic in `n_bins`. Fix: index pairs into a 2D `Vec<Vec<f64>>` once at the top of `render`, then O(1) lookup.

14. **LOW: `default_renderers()` registers more kinds than the spec enumerates.**
    `mark.rs` registers `DotX`, `DotY`, `Circle`, `LineX`, `LineY` in addition to the spec's `Dot`, `Bar`, `Line`, `Density`, `DensityX`, `DensityY`, `RegressionY`, `RegressionX`. Likely benign — these probably reuse existing renderers — but the registry now claims support for marks the spec/vocab still lists as Unimplemented. Worth a one-line audit to make sure `find_renderer` does not advertise capability the engine cannot honour end-to-end.

15. **LOW: `_as` subquery alias in `AggregateScalar` rendering.**
    `crates/brightfield-sql/src/render.rs` renders as `SELECT … FROM (<input>) AS _as`. DuckDB accepts the underscore-prefix alias, but it is conventional to use `t` or the source table name. Cosmetic; flag only because emitted SQL will appear in logs and conformance fixtures.

## What was verified well

- **`QueryPlan::AggregateScalar` IR variant** — Cleanly separated from `Aggregation`; no `GROUP BY` semantics conflated. Hash-stable (covered by gomb_ac01-style tests). Display/render logic in `render.rs` is straightforward.
- **`default_renderers()` registry** — Hand-rolled `HashMap<MarkKind, Box<dyn MarkRenderer>>` with a `find_renderer` lookup; main.rs no longer falls through to `DotRenderer` for unknown kinds (it logs `tracing::warn!` and skips). This is a real improvement on the previous silent-coercion behaviour.
- **Silverman bandwidth math** — `kde.rs` handles n < 2 (returns 1.0), zero variance (returns 1.0 floor), and uses the textbook constants (1.06 for 1D, 0.9 for 2D-per-axis with `min(σ, IQR/1.34)`). Reasonable edge-case coverage.
- **OLS via DuckDB `regr_*`** — The aggregate-scalar projection (`regr_slope`, `regr_intercept`, `regr_count`, `regr_avgx`, `regr_sxx`, `regr_sxy`, `regr_syy`) is the right hybrid split: DuckDB does the O(n) reduction; Rust does the per-pixel `se(ŷ|x) = s · √(1/n + (x-x̄)²/Sxx)`. Idiomatic.
- **SQL cache LRU at cap 32** — Implemented on `Session` with `duckdb_execute_count()` and `sql_cache_len()` introspection helpers. Hit rate is observable from outside, which is how you write good tests for it (even if the current ones don't use the affordance fully — see findings 1 and 2).
- **Build hygiene** — Zero warnings, no `dbg!`/`println!` left behind in production paths, no commented-out code blocks.

## Test summary

```
cargo test --workspace      → 389 passed, 0 failed (27 suites)
cargo test --workspace gomb_ → 25 passed, 364 filtered out
```

25 gomb_ tests is comfortably above the spec's implied lower bound (≥1 per AC × 17 ACs would be 17). Distribution skews toward the SQL emission and IR variants; the engine-side cache and renderer-side draw-call tests are the thinnest (see findings 1, 2, 3).

## Recommendation

REQUEST_CHANGES. The five HIGH findings each represent a concrete gap between what the spec says will be verified and what the merged code will actually verify. Two of them (gomb_ac11/ac12) are test-only fixes; the other three are small code or spec amendments. None block the architectural direction, and the bulk of the contribution (IR variant, registry, KDE/OLS math, cache plumbing) is good work that should land. After addressing the HIGHs and at minimum acknowledging the MEDIUMs in decisions.md, this is ready to ship.
