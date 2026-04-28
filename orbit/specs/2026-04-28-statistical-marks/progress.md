# Implementation Progress

Spec path: orbit/specs/2026-04-28-statistical-marks/spec.yaml
Spec hash: sha256:538ff46e936f001362ffdffd173643547c45aa9ab1e421adc472df17f2b142de
Started: 2026-04-28
Current AC: complete

## Hard Constraints
- [x] brightfield-render keeps its no-gpui invariant — kde.rs is pure Rust + Arrow
- [x] No new GPU compute shaders — Vello renders 2D paths only; KDE convolution is CPU
- [x] Bandwidth in data units — Silverman's rule when omitted; matches Mosaic web defaults
- [x] Density renderer split is geometry-driven (1D vs 2D), not vocabulary-driven
- [x] Linear OLS only; ci default 0.95; polynomial/exponential reject with EmitError::UnsupportedMark
- [x] default_renderers() replaces the silent _ => DotRenderer fallback in brightfield-app/src/main.rs
- [x] Existing MarkRenderer trait, MarkLower registry, ChannelMap, ScaleSet, propagate_param/propagate_selection signatures unchanged
- [x] No new dependencies (no nalgebra) — convolution and OLS analytics in pure Rust
- [x] Renderer-side SQL cache is a stand-in for proper two-tier param-effect routing — TODO(card-runtime-reactivity) comment references the future card
- [x] Existing tests must continue to pass: cargo test --workspace
- [x] Corpus parse gate (cfs ac-10 / cfs2 ac-13 equivalent) remains green — vocab flips do not break parser
- [x] Spec joins card 0008's specs: array as the statistical-marks slice

## Detours

## Acceptance Criteria
- [x] ac-01: QueryPlan::AggregateScalar IR variant + emitter (no GROUP BY)
- [x] ac-02: kde.rs module — kde_1d, kde_2d (flat Vec), silverman_1d, silverman_2d_per_axis
- [x] ac-03: Density1DRenderer { axis: DensityAxis } for DensityX/DensityY
- [x] ac-04: Density2DRenderer for Density (circle grid)
- [x] ac-05: RegressionRenderer — line + 32-point CI band
- [x] ac-06: RegressionLowerer — regr_* aggregates, group_by stroke, polynomial reject
- [x] ac-07: DensityLowerer — width_bucket 1D and 2D
- [x] ac-08: default_renderers() registry + find_renderer helper
- [x] ac-09: Vocab flips — Density/DensityX/DensityY/RegressionY/RegressionX → Implemented
- [x] ac-10: main.rs dispatch site uses find_renderer; silent dot fallback removed
- [x] ac-11: SQL cache (capped LRU 32) + duckdb_execute_count test accessor
- [x] ac-12: Bandwidth param drag re-renders without re-querying (cache hit)
- [x] ac-13: Conformance snapshots for density1d/density2d/linear-regression
- [x] ac-14: Parser known-keys allowlist accepts bandwidth/normalize/stack/offset/ci/thresholds
- [x] ac-15 (gate): cargo test --workspace green
- [x] ac-16 (gate): Corpus parse regression gate green
- [x] ac-17: ≥8 gomb_ tests across crates/ (25 gomb_ tests across 6 modules)
