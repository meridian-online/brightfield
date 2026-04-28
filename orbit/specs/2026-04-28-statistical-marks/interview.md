# Design: Grammar-of-Graphics Mark Library — Statistical Slice

**Date:** 2026-04-28
**Interviewer:** Nightingale (rally lead)
**Card:** orbit/cards/0008-grammar-of-graphics-mark-library.yaml
**Rally:** orbit/specs/2026-04-28-runtime-selections-statistical-marks-rally/
**Decision pack:** decisions.md (six decisions, all accepted wholesale)

---

## Context

Card: *Grammar-of-graphics mark library* — 3 scenarios across core, statistical, and specialised mark groups. This slice covers **only the statistical marks**: `density` (1D and 2D KDE) and `regression` (linear OLS + confidence band). Specialised marks (geo, hexbin, contour, raster) are explicitly deferred. Cards are intentionally delivered in slices via the spec-array.

Prior context:
- The core slice (`lineY`, `barY`, `dot`) and the GPU rendering machinery shipped together with card 0013 in commit f21b555. Renderer trait at `crates/brightfield-render/src/mark.rs:31-62`; dispatch (today a flat match with silent dot fallback) at `crates/brightfield-app/src/main.rs:98-108`; `MarkLower` registry at `crates/brightfield-sql/src/lower.rs:68`.
- The original whole-card design pack (`orbit/specs/2026-04-22-grammar-of-graphics-mark-library/`) recommended hybrid SQL + Rust compute but predates both the GPU pipeline and the param coordinator (commit 8ca4283). This slice updates that recommendation against the now-existing extension points.
- Vendored corpus already contains the target shapes: `density1d.yaml`, `density2d.yaml`, `density-groups.yaml`, `linear-regression.yaml`, `linear-regression-10m.yaml`. Vocabulary entries `Density`, `DensityX`, `DensityY`, `RegressionX`, `RegressionY` are declared at `crates/brightfield-spec/src/vocab.rs:144-162` but all currently flagged `Unimplemented`.
- DuckDB ships full ordinary-least-squares aggregates (`regr_slope`, `regr_intercept`, `regr_count`, `regr_avgx`, `regr_avgy`, `regr_sxx`, `regr_syy`, `regr_sxy`) as built-ins. It has no native KDE primitive; convolution must run in Rust on a pre-binned histogram.

Gap: the rendering pipeline and the SQL-emission pipeline both have extension points but no implementations of statistical marks. Specs that use `densityY` or `regressionY` today silently render as dots (the renderer fallback at `main.rs:105`).

## Q&A

### Q1: Where does the statistical compute happen — SQL, Rust, or hybrid?

**Q:** The card scenario says "draws from the underlying data without a separate precompute step". DuckDB has full `regr_*` aggregates but no KDE. Where do we split the work?

**A:** **Hybrid.** Regression: SQL emits a single-row aggregate (`SELECT regr_slope(y,x) AS slope, regr_intercept(y,x) AS intercept, regr_count(y,x) AS n, regr_avgx(y,x) AS x_bar, regr_sxx(y,x) AS sxx, regr_syy(y,x) AS syy, regr_sxy(y,x) AS sxy FROM <source> [WHERE filter]`) — DuckDB does the heavy lifting; the renderer draws a two-point line (extents transformed through scales) plus an analytic 95% CI band sampled at ~32 grid points. Density: SQL emits `SELECT width_bucket(x, dom_min, dom_max, n_bins) AS bin, COUNT(*) AS c FROM <source> [WHERE filter] GROUP BY bin` (or the 2D equivalent on `(x, y)`). Rust convolves the resulting histogram with a Gaussian kernel of σ = bandwidth in data units. KDE convolution lives in a new `brightfield-render/src/kde.rs` module — pure Rust + Arrow, no GPU compute, no gpui dependency (preserves the brightfield-render constraint from card 0013).

This adds **one** new IR variant: `QueryPlan::AggregateScalar { input, aggregates: Vec<String> }` — a no-group-by aggregate-only projection for regression. Density reuses existing `QueryPlan::Bin` + `QueryPlan::Aggregation`.

### Q2: Density mark surface — one renderer, two, or three?

**Q:** Mosaic vocabulary declares `Density`, `DensityX`, `DensityY`. The card says "1D or 2D KDE". The 1D and 2D outputs differ fundamentally (curve vs grid). How do we split renderers?

**A:** **Two renderers.** `Density1DRenderer { axis: DensityAxis }` handles `DensityX` (data on y, density along x) and `DensityY` (data on x, density along y). `Density2DRenderer` handles the 2D `Density` mark, emitting a `width × height` circle grid where circle radius scales with per-cell density (matches `density2d.yaml`). 1D and 2D share no real code — convolving a 1D histogram into an areaY-shaped curve vs convolving a 2D grid and emitting per-cell circles — so a single param-dispatched renderer adds ceremony without sharing implementation. `DenseLine`, `Heatmap`, `Contour`, `Raster` are deferred to the specialised slice.

### Q3: Regression surface — linear-only or polynomial-capable? CI band default-on?

**Q:** `linear-regression.yaml` has no explicit `ci:` field but the spec prose says a 95% confidence interval band appears around the line. Mosaic's web library supports linear/quadratic/exponential/logarithmic. What ships in this slice?

**A:** **Linear OLS only via DuckDB `regr_*` aggregates. CI band on by default at 95%.** Matches `linear-regression.yaml` implicit-CI semantics and Mosaic web defaults. `ci: false` disables the band; `ci: <0..1>` overrides the level. Polynomial regression and `type:` other than linear are explicitly out of slice scope — emit `EmitError::UnsupportedMark` with a clear message ("regression type X not yet supported, only linear"). Per-series regression (e.g. `stroke: sex` → grouped by category) uses the existing `QueryPlan::Aggregation` with `group_by: [stroke_col]`.

Spec surface accepted in this slice:
```yaml
mark: regressionY
data: { from: athletes, filterBy: $query }
x: weight
y: height
stroke: sex          # optional categorical → grouped regression
ci: 0.95             # optional, default 0.95; false to disable
```

### Q4: Bandwidth selection for KDE — required, defaulted, pixel-space?

**Q:** Specs sometimes set bandwidth explicitly (`bandwidth: 20` or `bandwidth: $bandwidth`); sometimes not. What's the default? What's the unit?

**A:** **Silverman's rule when omitted; data units when specified.** Silverman: `1.06 · σ̂ · n^(-1/5)` for 1D; the 2D rule uses `0.9 · min(σ̂, IQR̂/1.34) · n^(-1/6)` per dimension. Standard in d3-density, ggplot2, scipy. Computed from the (filtered) histogram at convolve time — one extra accumulator on the existing iteration. Bandwidth is in **data units** (matches Mosaic web semantics — `density1d.yaml:11` `bandwidth: 20` is in `delay` minutes, the slider is `min: 0.1, max: 100` in those same units). Pixel-space rejected: under pan/zoom the same pixel-bandwidth produces a 10× narrower data-domain kernel at 10× zoom, breaking spec compatibility.

`bandwidth` is already accepted by the parser (`crates/brightfield-spec/src/parse.rs:117`). The renderer reads it via a new `KDEParams { bandwidth: Option<f64>, normalize: NormalizeMode, stack: bool, offset: Option<OffsetMode> }` struct extracted alongside the channel map. `normalize`, `stack`, `offset`, `ci`, `thresholds` may need to be added to the parser's known-keys allowlist — verify during implementation, add as needed.

### Q5: Reactivity for bandwidth-only param changes?

**Q:** A bandwidth slider drag should re-render but doesn't need a new SQL query (the histogram is unchanged). The param coordinator currently re-queries on every param change. What's the policy?

**A:** **Renderer-side cache by emitted-SQL string for this slice; flag two-tier param-effect routing as the next runtime-reactivity card.** Concretely:
1. Bandwidth and other render-only params register as subscribers via the existing analysis path (no change).
2. When `propagate_param` re-emits the query, if the resulting `EmittedQuery.sql` is byte-identical to the previously-executed one for that mark, skip the DuckDB execute and reuse the cached `RecordBatch`. The Rust convolution re-runs with the new bandwidth value.
3. This is a one-line change in `Session::execute_mark`: cache by emitted-SQL string, return cached batches if hit.

This preserves correctness (filterBy changes alter the SQL → cache miss → re-query) and gives the bandwidth-slider performance win (avoids ~50ms per drag tick on 200K rows, which would violate card 0003's 60 FPS target). Architectural cleanup — explicit `param_affects: Query | Render` tagging in the analysis layer with two-tier coordinator routing — is the right long-term answer and is **load-bearing input for the next runtime-reactivity card**. Worth recording as an MADR alongside this work.

### Q6: Renderer dispatch — extend the flat match or registry function?

**Q:** Today's dispatch at `main.rs:98-108` is a flat `match` with `_ => DotRenderer` (silent fallback for unsupported kinds). Adding density and regression doubles the arms. What's the structural fix?

**A:** **`default_renderers()` registry in `brightfield-render`.** Mirrors the SQL-side pattern (`default_lowerers()` at `crates/brightfield-sql/src/lower.rs:68`). Returns `Vec<(MarkKind, Box<dyn MarkRenderer>)>`; helper `find_renderer(kind) -> Option<&dyn MarkRenderer>`. The flat match in `brightfield-app` shrinks to `find_renderer(kind).ok_or_else(...)`. Failure to find a renderer becomes a structured error (`RendererError::UnsupportedMark { kind }`) — replacing the silent dot fallback. This slice registers `Density1DRenderer` for `DensityX`/`DensityY`, `Density2DRenderer` for `Density`, `RegressionRenderer` for `RegressionY`/`RegressionX`. `DenseLine`/`Heatmap`/`Contour`/`Raster` remain unregistered → loud failure (the structured-error path at `main.rs:88-91` already exists for graceful skip).

---

## Summary

### Goal

Implement the statistical-marks slice of card 0008: `density` (1D and 2D KDE) and `regression` (linear OLS with default 95% confidence band). Rendering integrates with the existing GPU pipeline shipped in 0013; statistical compute splits hybrid between DuckDB aggregates and Rust convolution. Replace the silent dot-fallback dispatcher with a registry function. Specialised marks (geo, hexbin, contour, raster) and polynomial regression are explicitly out of slice.

### Constraints

- `brightfield-render` keeps its no-gpui invariant (constraint from `orbit/specs/2026-04-24-gpu-mark-rendering/spec.yaml:8`). KDE module is pure Rust + Arrow.
- No new GPU compute shaders. Vello renders 2D paths only; convolution is CPU.
- Bandwidth in data units; Silverman default when omitted.
- Density renderer split is geometry-driven (1D vs 2D), not vocabulary-driven (no `density{X,Y}` enum dispatch).
- The corpus parser must continue to accept `density1d.yaml`, `density2d.yaml`, `density-groups.yaml`, `linear-regression.yaml`, `linear-regression-10m.yaml` — vocab entries flip from `Unimplemented` to `Implemented` only after the renderer/lowerer pair lands.
- Existing `MarkRenderer` trait, `MarkLower` registry, `ChannelMap`, `ScaleSet` unchanged.
- No new dependencies — Rust linalg (nalgebra) explicitly rejected by Decision 3.

### Success Criteria

- `Density1DRenderer` renders `densityX` and `densityY` against the corpus `density1d.yaml`.
- `Density2DRenderer` renders the 2D `density` mark against `density2d.yaml`, producing a `width × height` circle grid with circle radius scaling with per-cell density.
- `RegressionRenderer` renders `regressionY` (and `regressionX`) against `linear-regression.yaml`, with a default 95% CI band on. `linear-regression-10m.yaml` runs interactively (regression aggregate stays in DuckDB; renderer geometry only).
- Per-series regression (e.g. `stroke: sex`) groups by the stroke column.
- Bandwidth slider drag (`density1d.yaml:14-19`) re-renders without re-querying DuckDB when only `bandwidth` changes (renderer-side SQL cache hits).
- `default_renderers()` exists; the silent dot fallback in `main.rs` is gone; unsupported marks emit `RendererError::UnsupportedMark` and trigger graceful skip.
- Conformance: `density1d.yaml`, `density2d.yaml`, `linear-regression.yaml` reach `Implemented` status in `vocab.rs` and produce snapshot SQL outputs.
- Unit tests: Silverman bandwidth → known σ on a normal distribution; OLS regression on Anscombe's quartet; KDE convolution against a fixed reference output.

### Decisions Surfaced

- **D1 hybrid SQL + Rust compute** — `regr_*` and `width_bucket` in DuckDB; KDE convolution and CI band geometry in Rust. New `QueryPlan::AggregateScalar` IR variant.
- **D2 two density renderers** — `Density1DRenderer { axis }` and `Density2DRenderer`. Specialised raster/contour marks deferred.
- **D3 linear-only regression with default CI** — `ci: 0.95` default. Polynomial deferred.
- **D4 Silverman bandwidth in data units** — matches Mosaic web defaults.
- **D5 renderer-side SQL-string cache** — short-circuit re-query when only render-affecting params change. Two-tier coordinator routing flagged as load-bearing for the next runtime-reactivity card.
- **D6 `default_renderers()` registry** — replaces the silent dot fallback; structured error on unsupported kinds.

### Implementation Notes

- **New module** `crates/brightfield-render/src/kde.rs` — 1D and 2D Gaussian convolution against a histogram-shaped Arrow batch, plus Silverman bandwidth helper. Pure Rust + Arrow. Unit tests use Anscombe / known-σ normal distributions.
- **New IR variant** `QueryPlan::AggregateScalar { input: Box<QueryPlan>, aggregates: Vec<String> }` at `crates/brightfield-sql/src/ir.rs`. Emits `SELECT <aggs> FROM (<input>)` with no GROUP BY. Unit-test on its own.
- **Renderer additions** under `crates/brightfield-render/src/marks/` (or wherever the existing dot/line/bar renderers live): `density.rs` with `Density1DRenderer` + `Density2DRenderer`; `regression.rs` with `RegressionRenderer` (line + optional CI band). Both consume the channel map + scale set + the histogram/aggregate Arrow batch.
- **Registry** `brightfield_render::mark::default_renderers() -> Vec<(MarkKind, Box<dyn MarkRenderer>)>` and `find_renderer(kind) -> Option<&dyn MarkRenderer>`.
- **Dispatch site** `crates/brightfield-app/src/main.rs:98-108` rewires to the registry. Silent `_ => DotRenderer` fallback removed; unsupported-mark path uses the existing graceful-skip pattern (`main.rs:88-91`).
- **SQL execute cache** — minimal change in `Session::execute_mark`: a `HashMap<String, Vec<RecordBatch>>` keyed by emitted SQL, populated on each successful execute, consulted before dispatching to DuckDB. Eviction policy: cleared on `propagate_param` for any param tagged `filterBy`-affecting, or simpler — capped LRU. Capped LRU is simpler and correct enough.
- **Vocabulary table flips** to `Implemented` for `Density`, `DensityX`, `DensityY`, `RegressionY`, `RegressionX` once the renderer/lowerer pair is registered. `DenseLine`, `Heatmap`, `Contour`, `Raster` stay `Unimplemented`.
- **Test prefix `gomb_`** (or follow existing 0008 conventions) — TBD at spec time. Cover: SQL emission per mark, KDE convolution against reference, OLS against Anscombe, registry-miss error, bandwidth-cache reactivity (param drag re-renders without re-query).
- **Spec joins card's `specs:` array** as the statistical entry, alongside the deferred core/specialised slices.

### Open Questions

- None at design time. Two questions resolve at spec time:
  1. Cache eviction policy — capped LRU vs predicate-aware invalidation. Capped LRU is the recommended starting point.
  2. Test prefix — `gomb_` is the obvious choice mirroring `cfs2_` / `rpw2_`, but if there's an existing convention from the 0008 core slice (commit f21b555) we adopt that instead.
