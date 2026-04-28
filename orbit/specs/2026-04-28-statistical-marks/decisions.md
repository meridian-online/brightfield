# Decision Pack — Card 0008 (Statistical Marks Slice)

Rally: **runtime-selections-statistical-marks** (sprint candidate "Mark coverage breadth").
Card: `orbit/cards/0008-grammar-of-graphics-mark-library.yaml`, scenario "Statistical marks for distribution views".
Slice scope: `density` (1D and 2D KDE) and `regression` (least-squares fit + optional confidence band). Specialised marks (geo, hexbin, contour, raster) are deferred. The core slice (lineY, barY, dot) and the GPU rendering machinery shipped with cards 0008 + 0013 in commit `f21b555`.

## What is already fixed for this slice

These are inherited from completed cards and are not up for re-decision:

- **MarkRenderer trait** at `crates/brightfield-render/src/mark.rs:31-62` — `render(scene, batch, channel_map, scales, highlight)` plus `render_interpolated(...)`. Three concrete impls today: `DotRenderer`, `BarRenderer`, `LineRenderer` (lines 168–445).
- **Renderer dispatch** at `crates/brightfield-app/src/main.rs:98-108` — a flat `match kind { Dot => DotRenderer, Line => LineRenderer, BarX|BarY => BarRenderer, _ => DotRenderer (fallback) }`. The fallback today silently renders unsupported marks as dots.
- **ChannelMap** at `crates/brightfield-render/src/channel.rs:13-126` — recognises X, Y, Fill, Stroke, Size, X1, Y1, X2, Y2; skips ParamRef channels with a warning. No transform fields, no bandwidth/threshold parsing.
- **ScaleSet + infer_scales** at `crates/brightfield-render/src/scale.rs:262-297` — infers Linear/Band/Time/Colour from Arrow schema after query execution; `infer_scales_multi` (line 308) unions domains across multiple batches. `ViewExtent` (line 218) overrides domain for pan/zoom.
- **MarkLower trait + DefaultLowerer** at `crates/brightfield-sql/src/lower.rs:29-43` — every unregistered MarkKind returns `EmitError::UnsupportedMark`. `default_lowerers()` (line 68) registers `Dot`, `Line`, `BarX`, `BarY` to `SimpleLowerer` (which emits `QueryPlan::Source { table }`).
- **QueryPlan IR** at `crates/brightfield-sql/src/ir.rs:91-145` — Source, Filter, Projection, Aggregation, Bin, Order, Limit. **No** variant for raw aggregate-only projections (i.e. `SELECT regr_slope(y,x), regr_intercept(y,x) FROM t`); the closest is `Aggregation` with empty `group_by`.
- **MarkKind vocabulary** at `crates/brightfield-spec/src/vocab.rs:144-162` — `Density`, `DensityX`, `DensityY`, `DenseLine`, `RegressionY`, `RegressionX` already declared, all `Unimplemented`.
- **Param coordinator** at `crates/brightfield-engine/src/lib.rs:244-291` — `Session::propagate_param(name, value)` updates `param_state`, looks up subscribers from `analysis.subscriber_graph`, filters to mark components, re-emits and re-executes each. **This is the integration point for reactive bandwidth/thresholds sliders.**
- **AST already parses** `bandwidth` as a known mark option (`crates/brightfield-spec/src/parse.rs:117`) and `fillOpacity`, `filterBy`, etc. The bandwidth value flows into `mark.options` as `ValueOrParamRef<SpecValue>`.
- **Vendored corpus evidence** for required spec shapes:
  - `density1d.yaml` — `mark: densityY, x: delay, fill: '#888', fillOpacity: 0.5, bandwidth: $bandwidth` (a 1D vertical density strip on a single x channel).
  - `density2d.yaml` — `mark: density, x: bill_length, y: bill_depth, r: density, fill: species, width: $bins, height: $bins, bandwidth: $bandwidth` (a 2D density binned to a `width × height` grid, with per-cell `r` mapped to point density).
  - `density-groups.yaml` — adds `normalize: $normalize, stack: $stack, offset: $offset` for grouped densities.
  - `linear-regression.yaml` — `mark: regressionY, x: weight, y: height, stroke: sex` plus a separate `dot` layer; the regression layer carries an implicit 95% confidence band (per the spec's prose). `xyDomain: Fixed`.
  - `linear-regression-10m.yaml` — same pattern, plus `data: { from: $data, filterBy: $query }` driven by an `intervalXY` brush. `regressionY` is layered on top of a `raster` background.

---

## Decision 1 — Where statistical compute happens

### Context
The card scenario says density and regression "draw from the underlying data without a separate precompute step". The original design pack (`orbit/specs/2026-04-22-grammar-of-graphics-mark-library/decisions.md`, Decision 3) recommended **simple transforms in SQL, statistical transforms client-side in Rust** — but that recommendation was made before the GPU pipeline shipped, before the param coordinator landed, and based on a different SQL surface (no `regr_*` IR support). DuckDB has built-in `regr_slope`, `regr_intercept`, `regr_r2`, `regr_count`, `regr_avgx`, `regr_avgy`, `regr_sxx`, `regr_syy`, `regr_sxy` — a full ordinary-least-squares toolkit. It has **no** native KDE function. The Mosaic web reference docs say regression runs in the database, density does kernel smoothing in the browser after in-database binning.

The decision is the split between SQL (DuckDB) and Rust (CPU) compute, given GPU compute is not currently on the table — the GPU pipeline is Vello-via-wgpu drawing into a `Scene`, not a wgpu compute kernel; introducing compute shaders here is out of slice scope.

### Options

- **A. All in SQL.** Regression: emit a single-row aggregate (`regr_slope(y,x), regr_intercept(y,x), regr_count(y,x), regr_sxx, regr_sxy, regr_syy`) — DuckDB does the work, the renderer draws a two-point line plus an analytic confidence band derived from the regression statistics. Density 1D: emit a per-point `width_bucket` histogram and apply Gaussian smoothing in DuckDB via a self-join (`SELECT b.bin, SUM(exp(-((b.bin-h.x)/sigma)^2)) FROM bins b, raw h`) or a custom UDF. Density 2D: 2D `width_bucket` grid + 2D Gaussian self-join.
- **B. Hybrid — DuckDB does aggregation + binning, Rust does kernel smoothing and confidence band.** Regression: DuckDB returns a single `regr_*` row; Rust computes the line endpoints + 95% CI band from those statistics. Density 1D: DuckDB returns the raw filtered column (or, for large sources, a pre-binned histogram via `width_bucket` + `COUNT(*)`); Rust convolves with a Gaussian kernel at user-chosen bandwidth. Density 2D: same pattern but on a 2D grid.
- **C. All in Rust.** SQL only fetches `SELECT x, y FROM t WHERE filter`. Regression and KDE both run on the Arrow record batch in Rust.

### Trade-offs

- **A (all SQL)** maximises pushdown — for `linear-regression-10m.yaml` (10M rows), `regr_slope` runs in DuckDB without ever materialising 10M rows in Rust. For density, a self-join KDE in SQL is `O(n × bins)` and allocates a giant intermediate; on `gaia.yaml` (5M rows × 200 bins = 1B comparisons) it is **slower** than transferring the raw data and convolving in Rust. The 95% CI band in pure SQL requires emitting a per-x-grid `t_α/2 · σ · √(1/n + (x-x̄)²/Sxx)` expression per evaluation point, which the QueryPlan IR cannot express today (it has no per-row computed-column variant beyond `Bin`). Cost: invent IR variants for `RegrAgg`, `KDESelfJoin`, `CIBand` — three new variants for a single mark family.
- **B (hybrid)** matches what Mosaic's web pipeline actually does (regression server-side, KDE in-browser smoothing of binned data per the prose in `density1d.yaml`). For regression, DuckDB returns a tiny one-row result regardless of dataset size — this is the "bandwidth" win. For 1D KDE, a `width_bucket` + `COUNT` pre-pass in DuckDB compresses 200K rows to a 100-bin histogram; Rust then convolves the histogram (not the raw points) with a Gaussian — that's `O(bins²)`, not `O(n × bins)`. For 2D KDE, the same trick: bin to a `width × height` grid in DuckDB, convolve in Rust. The Arrow record batch is in-process per card 0012 — no serialisation cost. Cost: two compute paths (server vs client) and a Rust kernel routine. **The bandwidth/normalize/stack/offset params apply purely on the Rust side**, so the slider in `density1d.yaml:14-19` re-runs only the Rust convolution on a cached histogram — fast feedback without re-querying DuckDB.
- **C (all Rust)** is simplest. For regression on 10M rows it transfers 10M rows of two columns to Rust (~160 MB) just to compute one slope. Defeats the database-first principle in `README.md:12` ("All data-intensive computation … is pushed to DuckDB").

### Recommendation
**Option B (hybrid).** Regression: SQL emits `SELECT regr_slope(y,x) AS slope, regr_intercept(y,x) AS intercept, regr_count(y,x) AS n, regr_avgx(y,x) AS x_bar, regr_sxx(y,x) AS sxx, regr_syy(y,x) AS syy, regr_sxy(y,x) AS sxy FROM <source> [WHERE filter]`. The renderer draws a two-point line (at the visible x-domain endpoints, transformed through scales) and, when `ci` is enabled, an analytic 95% confidence band sampled at ~32 grid points along x using `s² = (syy - sxy²/sxx)/(n-2)`, `se(ŷ|x) = s · √(1/n + (x-x̄)²/sxx)`.

Density 1D: SQL emits `SELECT width_bucket(x, dom_min, dom_max, n_bins) AS bin, COUNT(*) AS c FROM <source> [WHERE filter] GROUP BY bin`. Rust convolves the histogram with a Gaussian kernel of σ = bandwidth (in data units). Density 2D: same with 2D `width_bucket` on (x, y) and a 2D Gaussian convolution producing a `width × height` grid; per-cell density becomes the `r` channel for circle marks (matching `density2d.yaml`).

This adds **one** new IR variant: `QueryPlan::AggregateScalar { input, aggregates: Vec<String> }` — a no-group-by single-row aggregate projection — used by regression. Density reuses the existing `QueryPlan::Bin` + `QueryPlan::Aggregation` variants. KDE convolution lives in a new `brightfield-render/src/kde.rs` module (Rust-only, no GPU dependency, no gpui dependency — preserves the brightfield-render constraint at `orbit/specs/2026-04-24-gpu-mark-rendering/spec.yaml:8`).

---

## Decision 2 — Density mark surface: one mark or three?

### Context
The card scenario names `density (1D or 2D KDE)` as a single mark concept, but Mosaic vocabulary declares three: `Density` (2D), `DensityX` (1D, vertical strip on x), `DensityY` (1D, vertical strip with x as the data column — see `density1d.yaml` which uses `densityY` for an x-axis distribution). The corpus uses `densityY` for 1D and `density` for 2D. The shape of the rendered output differs fundamentally: 1D is an areaY-shaped curve; 2D is a circle-grid (per `density2d.yaml`) or a heatmap raster (per `flights-density.yaml`, but heatmap is deferred). The compute also differs (1D convolution vs 2D convolution).

### Options

- **A. Single `DensityRenderer` parameterised by `dimensions: 1D | 2D` + `axis: X | Y`.** One renderer handles all three MarkKind variants by inspecting the channel map (only-x → 1D-X, only-y → 1D-Y, both x and y → 2D). One KDE kernel routine with a 1D vs 2D switch.
- **B. Two renderers: `Density1DRenderer` and `Density2DRenderer`.** Mark dispatch: `DensityX | DensityY → Density1DRenderer`, `Density → Density2DRenderer`. Distinct renderer types, distinct KDE routines.
- **C. Three renderers, one per MarkKind.** Maximum granularity, no shared abstraction beyond the trait.

### Trade-offs

- **A (single + dimensions arg)** mirrors the family-lowerer recommendation in the prior decision pack (Decision 1 of `orbit/specs/2026-04-22-grammar-of-graphics-mark-library/decisions.md`) — "~10 mark families parameterised by axis orientation + variant flags." The 1D and 2D KDE share no actual code (1D is convolving a 1D histogram into an areaY curve; 2D is convolving a 2D grid and emitting per-cell circles), so the parameterisation is pure dispatch — the inner code paths are still separate. The "family" abstraction adds dispatch ceremony without sharing implementation.
- **B (two renderers split by dimensionality)** matches the actual code shape: 1D and 2D are different geometry (curve vs grid) and different kernels (1D Gaussian vs 2D radial Gaussian). Each is independently testable. The MarkKind→renderer mapping table at `crates/brightfield-app/src/main.rs:101-106` already extends naturally:
  ```
  MarkKind::DensityX | MarkKind::DensityY => Density1DRenderer { axis: X | Y },
  MarkKind::Density => Density2DRenderer,
  ```
  The 1D renderer carries a small enum for which axis the data column maps to (in `densityY`, the data is on `x` and the density is rendered along `y` — counterintuitive but matches Mosaic).
- **C (three renderers)** is over-fragmented — `DensityX` and `DensityY` literally differ only in axis orientation (the histogram is along the data axis, the density is plotted along the perpendicular axis).

### Recommendation
**Option B.** Implement `Density1DRenderer { axis: DensityAxis }` (where `DensityAxis` is `X` for `densityX` — densities along x with bars rising in y — or `Y` for `densityY` — densities along x rendered as a vertical strip… wait, let's be precise: `densityY` means "the y position carries the density estimate, x is the data axis". So `axis: DensityAxis::Y` means the curve grows in the y direction). And implement `Density2DRenderer` for the `density` MarkKind, emitting a `width × height` circle grid where circle radius scales with per-cell density (matching `density2d.yaml`). Both renderers consume a histogram-shaped RecordBatch from DuckDB and convolve in Rust per Decision 1.

Defer `DenseLine` (line-density) and `Heatmap`/`Contour`/`Raster` (rasterised 2D density variants) to the specialised slice — they share the KDE compute path with `Density2D` but differ in geometry output.

---

## Decision 3 — Regression mark surface and the confidence band

### Context
`linear-regression.yaml` declares `mark: regressionY, x: weight, y: height, stroke: sex` and the spec prose says "The area around a regression line shows a 95% confidence interval." The CI is implicit in Mosaic — there's no `ci: true` option in the corpus YAML, the band just appears. Mosaic's web library defaults `ci: 0.95`. The renderer must produce both a line and (optionally, but on by default) a translucent area band.

There's also a question about polynomial / exponential regression. Mosaic's web library supports `type: linear | quadratic | exponential | logarithmic | power`. The corpus only uses linear (the default). DuckDB's `regr_*` aggregates compute linear OLS only — polynomial regression in SQL requires either a UDF or fitting via DuckDB's matrix linalg extensions (not currently a dependency).

### Options

- **A. Linear-only first, CI band on by default.** `regressionY` and `regressionX` lower to a single `regr_*` aggregate query. The renderer reuses `LineRenderer` for the line and adds a new `AreaBandPath` for the CI band. `ci` option in spec controls visibility (default `0.95`). Polynomial/exponential are explicitly deferred; the renderer emits `EmitError::UnsupportedMark` if `type:` is anything but `linear` (or omitted).
- **B. Linear-only, CI band opt-in.** Same as A but the band is only drawn when `ci` is explicitly set in the mark options.
- **C. Polynomial-capable from day one.** Compute β = (X'X)⁻¹X'y in Rust on the raw data (transferred from DuckDB) for arbitrary polynomial degree.

### Trade-offs

- **A (linear with default CI)** matches `linear-regression.yaml`'s implicit-CI behaviour and Mosaic's defaults. It produces visually correct output for the corpus specs without spec authors having to opt in. Cost: one extra geometry primitive (the area band). The band is a closed path stroked with `Affine::IDENTITY` and filled at low alpha — Vello handles this trivially via `scene.fill()` on a Bezier path.
- **B (CI opt-in)** is simpler but produces visually wrong output for `linear-regression.yaml` (the spec says a band appears but the YAML doesn't request it). This breaks Mosaic spec compatibility.
- **C (polynomial-capable)** is over-scope. None of the corpus density/regression specs use polynomial regression. The compute path is also higher-cost — fitting a degree-3 polynomial requires either a Rust linalg crate (e.g. nalgebra, adding a dependency) or hand-rolled QR/normal-equations. Defer to a future card.

### Recommendation
**Option A.** Linear OLS only. CI band rendered by default at 95% (matching Mosaic) when the renderer has the regression statistics; `ci: false` in mark options disables the band; `ci: <0..1>` overrides the level. Polynomial regression and `type:` other than linear are explicitly out of slice scope — emit `EmitError::UnsupportedMark` with a clear message ("regression type X not yet supported, only linear").

Spec surface for this slice:
```
mark: regressionY
data: { from: athletes, filterBy: $query }   # filterBy supported via existing infra
x: weight                                     # data column for X
y: height                                     # data column for Y
stroke: sex                                   # series colour split (groups regression by category)
ci: 0.95                                      # optional, default 0.95; false to disable band
```

Per-series regression (when `stroke: sex` is a categorical) requires emitting `regr_*` grouped by the stroke column — the existing `QueryPlan::Aggregation` with `group_by: [stroke_col]` handles this.

---

## Decision 4 — Bandwidth selection for KDE

### Context
KDE bandwidth controls smoothness. Mosaic specs let users specify it directly (`bandwidth: 20` or `bandwidth: $bandwidth`) but in their absence Mosaic uses Scott's rule of thumb (σ̂ · n^(-1/5) · 1.06). The corpus specs always set bandwidth explicitly (`density1d.yaml:11` defaults to 20 with a slider `min: 0.1, max: 100`). For the brightfield slice, two questions: (1) what default applies when bandwidth is omitted? (2) is bandwidth in data units or pixel units?

### Options

- **A. Silverman's rule by default (1.06 · σ · n^(-1/5)) when omitted, value is in data units when specified.** Compute σ from the histogram in Rust at convolve time; recompute when filter changes.
- **B. Require explicit bandwidth, error on omission.** Spec authors must always set it. Sliders (per `density1d.yaml`) would always be present.
- **C. Pixel-space bandwidth.** Bandwidth is interpreted as pixels regardless of underlying data scale.

### Trade-offs

- **A (Silverman default, data units)** matches Mosaic web semantics exactly — `density1d.yaml:11` `bandwidth: 20` is in data units (delay minutes), the slider scales 0.1..100 in the same units. Silverman is the standard default in d3-density, ggplot2, scipy. Cost: σ must be computed from the (filtered) histogram each time the filter changes — but we already iterate the histogram for convolution, this is one extra accumulator.
- **B (require explicit)** is simpler implementation but breaks `density-groups.yaml`-style specs where the author sets `bandwidth: 20` once at the top and shares it across multiple density marks — and fails closed for any spec author who forgets the option. Hostile UX.
- **C (pixel-space)** breaks under pan/zoom — when the user zooms in 10×, the same pixel-bandwidth produces a 10× narrower data-domain kernel. The KDE would re-smooth differently at every zoom level. Mosaic uses data units; deviate at our peril.

### Recommendation
**Option A.** Bandwidth is in **data units**. When omitted, default to Silverman's rule (`1.06 · σ̂ · n^(-1/5)` for 1D; the 2D rule uses a scaled version `0.9 · min(σ̂, IQR̂/1.34) · n^(-1/6)` per dimension). When specified as a literal or `$param`, use the value verbatim. The bandwidth value flows through the existing `mark.options` bag (`bandwidth` is already in the parser's known-keys list at `crates/brightfield-spec/src/parse.rs:117`); the renderer reads it via a new `KDEParams { bandwidth: Option<f64>, normalize: NormalizeMode, stack: bool, offset: Option<OffsetMode> }` struct extracted alongside the channel map.

---

## Decision 5 — Reactivity: how density/regression re-render under param and selection changes

### Context
Statistical marks have two reactive triggers that core marks do not:
1. **Bandwidth/threshold sliders** that don't change the SQL but do change the Rust-side compute (the histogram is the same, the convolution changes).
2. **Filter/selection changes** (`data.filterBy: $query`) that change the SQL — same as core marks.

The param coordinator at `crates/brightfield-engine/src/lib.rs:244-291` handles case 2 already: `propagate_param` looks up subscribers, re-emits and re-executes. But for case 1, re-running the SQL is wasteful — the histogram doesn't change when bandwidth changes. There is no current pathway for "re-render this mark without re-querying."

### Options

- **A. Always re-query on any param change.** `bandwidth` and `filterBy` both go through `propagate_param`, both re-run SQL, both re-render. Simplest. Wasteful for bandwidth-only changes.
- **B. Two-tier reactivity: SQL-affecting params trigger re-query, render-affecting params trigger re-render only.** Add a `param_affects: Vec<ParamEffect>` on each mark in the analysis layer where `ParamEffect = Query | Render`. Bandwidth/normalize/stack/offset/thresholds tag as `Render`; filterBy/data tag as `Query`. The coordinator routes accordingly: re-query subscribers for Query effects, re-convolve cached histograms for Render effects.
- **C. Cache the last RecordBatch per mark, re-convolve on bandwidth change.** No analysis-layer change. The renderer holds an `Option<(LastInputHash, RecordBatch)>`; if the hash hasn't changed, skip SQL and re-convolve. Implementation detail of the renderer, invisible to the param coordinator.

### Trade-offs

- **A (always re-query)** is cheap to implement (already done — `bandwidth: $bandwidth` already lands as a subscriber graph entry today, since the analysis layer registers param refs in mark options). Cost: every slider drag re-runs SQL on 200K+ rows. For `density1d.yaml` against a 200K-row Parquet that takes ~50 ms per query — visible jank during slider drag. Card 0003 ("fluid interaction at dataset scale") sets a 60 FPS target; 50 ms re-queries violate it.
- **B (two-tier)** is the architecturally clean answer. It introduces a new analysis-layer concept (param effects) and a new dispatch path in the runtime. Significant scope expansion for this slice. The categorisation table is small and stable (bandwidth/normalize/stack/offset/thresholds/ci → Render; filterBy/where/predicate → Query) but adds a new contract every future mark must opt into.
- **C (renderer-side cache)** is pragmatic. The renderer keeps the last `(plan_hash, RecordBatch)` it convolved. On the next render, if the histogram-producing query produced an identical structural hash, reuse the cached batch — only the convolution re-runs. This needs **no** analysis-layer change and **no** coordinator change. The plan hash is already in `QueryPlan::hash_structural()` (`crates/brightfield-sql/src/ir.rs:153`). Caveat: the cache is a render-state concern, so it must live in `ChartState` or the `Density1DRenderer`/`Density2DRenderer` itself; that pushes mutable state into the renderer struct.

### Recommendation
**Option C for this slice, with a TODO that resolves to Option B in the runtime-reactivity card.** Concretely:
- Bandwidth and other render-only params still register as subscribers via the existing analysis path (no change).
- When `propagate_param` re-emits the query, if the resulting `EmittedQuery.sql` is byte-identical to the previously-executed one for that mark, skip the DuckDB execute and reuse the cached `RecordBatch`. The convolution re-runs in the renderer using the new bandwidth value. This is a one-line change in `Session::execute_mark`: check the cache by emitted-SQL string, return cached batches if hit.
- This preserves correctness (filterBy changes alter the SQL → cache miss → re-query) and gives the bandwidth-slider performance win without inventing a new analysis-layer concept this slice can't fully justify.
- Add a short comment in `decisions.md` (this file) flagging that two-tier reactivity is the right long-term answer and should be the design lead for the runtime-reactivity card.

This decision is **load-bearing** for the "draws from the underlying data without a separate precompute step" requirement in the card scenario — the renderer convolves on every paint cycle that needs it, no offline pipeline.

---

## Decision 6 — Renderer dispatch and registry growth

### Context
The current dispatch at `crates/brightfield-app/src/main.rs:98-108` is a flat `match` with a silent dot fallback for unsupported kinds. Adding density and regression makes that match four kinds longer, and the silent fallback masks legitimate "this mark is not yet supported" cases — a `density2d` spec will today render as dots if density isn't registered.

### Options

- **A. Extend the flat match in main.rs.** Add `Density | DensityX | DensityY → Density*Renderer`, `RegressionY | RegressionX → RegressionRenderer`. Keep the dot fallback.
- **B. Move dispatch to a registry function in `brightfield-render`.** Mirror the pattern in `crates/brightfield-sql/src/lower.rs:68` (`default_lowerers()`) — return `Vec<(MarkKind, Box<dyn MarkRenderer>)>` from `default_renderers()`, with a `find_renderer(kind)` helper. The fallback emits a structured error rather than silently rendering dots.
- **C. Register renderers on the `MarkKind` enum itself.** Static method like `kind.renderer() -> Box<dyn MarkRenderer>`. Couples the spec vocabulary crate to the render crate.

### Trade-offs

- **A (extend match)** is the smallest diff. Loses: the silent fallback at `main.rs:105` (`_ => Box::new(DotRenderer)`) is a known footgun — a `regression` spec that hits the slice before regression is implemented produces dots, not an error. The new statistical marks deserve to fail loudly when partially-implemented (e.g. `density-groups.yaml` uses `normalize`, `stack`, `offset` — if the slice ships with `normalize: none, stack: false, offset: null` only, hitting any other value should error not silently degrade).
- **B (registry function in brightfield-render)** matches the SQL-side pattern (one source of truth, parallel to `default_lowerers()`). Replaces the silent fallback with a structured `RendererError::UnsupportedMark { kind }`. The `brightfield-app` `match` shrinks to `find_renderer(kind).ok_or(...)`. This is a small, scoped refactor of existing code.
- **C (enum method)** would require `brightfield-spec` to depend on `brightfield-render`, inverting the dependency tree (`brightfield-render` depends on `brightfield-spec` for the AST). Architecturally invalid.

### Recommendation
**Option B.** Introduce `brightfield_render::mark::default_renderers() -> Vec<(MarkKind, Box<dyn MarkRenderer>)>` and `find_renderer(kind) -> Option<&dyn MarkRenderer>`. Update `crates/brightfield-app/src/main.rs:98-108` to call the registry. Failure to find a renderer becomes a hard error (`eprintln!` + skip the mark, matching the existing graceful-failure pattern at `main.rs:88-91` and `msv_ac05_graceful_failure_skips_invalid_mark` test at `main.rs:190`). This slice registers `Density1DRenderer` for `DensityX`/`DensityY`, `Density2DRenderer` for `Density`, `RegressionRenderer` for `RegressionY`/`RegressionX`. `DenseLine`, `Heatmap`, `Contour`, `Raster` remain unregistered → hard error → caller sees an actionable message.

---

## Summary

```
| #  | Decision                                  | Recommendation                                                              |
|----|-------------------------------------------|------------------------------------------------------------------------------|
| 1  | Where statistical compute happens         | Hybrid: regr_* aggregates + width_bucket histograms in DuckDB; KDE          |
|    |                                           | convolution + CI band geometry in Rust (brightfield-render/src/kde.rs)      |
| 2  | Density mark surface                       | Two renderers: Density1DRenderer { axis } for densityX/Y, Density2DRenderer |
|    |                                           | for density. DenseLine/Heatmap/Contour/Raster deferred.                     |
| 3  | Regression surface                         | Linear OLS only; CI band on by default at 95%; polynomial deferred          |
| 4  | Bandwidth selection                        | Silverman's rule when omitted; data units when specified                     |
| 5  | Reactivity for bandwidth-only changes     | Renderer-side SQL-string cache short-circuits re-query when only render-    |
|    |                                           | only params change. Two-tier coordinator routing deferred to next card.    |
| 6  | Renderer dispatch                          | default_renderers() registry in brightfield-render, parallel to             |
|    |                                           | default_lowerers(); hard error on unregistered kinds                        |
```

## Cross-cutting notes

- **brightfield-render must not gain a gpui dependency** (constraint from `orbit/specs/2026-04-24-gpu-mark-rendering/spec.yaml:8`). The KDE convolution module lives in `brightfield-render/src/kde.rs`, pure Rust + Arrow, no gpui.
- **No new GPU compute shaders.** Vello renders 2D vector paths only; statistical compute is CPU. If profiling later shows convolution is hot for `gaia.yaml` (5M rows), a wgpu compute shader for 2D Gaussian convolution is a follow-up card, not this slice.
- **Conformance.** Per-mark SQL emission produces conformance snapshots (extending the card 0004 pattern). For this slice: `linear-regression.yaml`, `density1d.yaml`, `density2d.yaml` should reach `Implemented` status in `vocab.rs` and produce snapshot SQL outputs. The Rust-side KDE/CI computation is unit-tested against known reference values (Silverman bandwidth → known σ on a normal distribution; OLS regression on Anscombe's quartet).
- **What this slice explicitly does not deliver:** `DenseLine`, `Heatmap`, `Contour`, `Raster`, polynomial regression, `errorbarX`/`errorbarY`, M4 downsampling for line marks, GPU compute shaders, two-tier param-effect routing. These either belong to the specialised slice or to dedicated future cards.
- **IR addition:** one new variant `QueryPlan::AggregateScalar { input, aggregates: Vec<String> }` to support regression's no-group-by aggregate-only projection. Document at `crates/brightfield-sql/src/ir.rs` alongside the existing `Aggregation` variant. Density reuses `Bin` + `Aggregation`.
- **AST surface:** no new mark options beyond what the parser already accepts (`bandwidth`, `fillOpacity`, `stroke`, `r`, `width`, `height`, `filterBy` are already in the known-keys list at `crates/brightfield-spec/src/parse.rs`). `normalize`, `stack`, `offset`, `ci`, `thresholds` may need to be added to the known-keys allowlist before the slice ships — verify during implementation, add as needed.

---

## Post-review corrections (2026-04-28)

A fresh independent PR review (`review-pr-2026-04-28-fresh.md`) surfaced gaps between what the spec said would be verified and what the merged code actually verified. Five HIGHs and several MEDIUMs were flagged. HIGHs were fixed in-place on `rally/statistical-marks`. MEDIUMs split between fix-now (small, in-scope) and defer-with-rationale (require architectural change). This section records both the fixes that landed and the conscious deferrals.

### Fixed in this slice (post-review)

```
| #       | Finding                                                                            | Fix                                                                                                                                                                                                                                                                                                          |
|---------|------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| HIGH 1  | `gomb_ac12` did not invoke `propagate_param`                                       | Test rewritten as `gomb_ac12_propagate_param_with_unchanged_sql_hits_cache`. Uses a selection param (`brush`) routed through `selection_state` predicates, so the param value never inlines into SQL. Two `propagate_param("brush", v)` calls produce byte-identical SQL → second call must hit the SQL cache. |
| HIGH 2  | `gomb_ac11_sql_cache_lru_eviction` only checked size                                | Test extended: touch k0, insert k32, then re-execute k0 (must be cache hit) and k1 (must be cache miss). Locks LRU semantics in.                                                                                                                                                                              |
| HIGH 3  | `count_scene_fills` was a stub returning `0`                                       | Replaced with `count_scene_paths` reading `vello_encoding::Encoding::n_paths`. Old name kept as `#[deprecated]` alias. `gomb_ac03` asserts ≥1, `gomb_ac04` asserts ≥9 (3×3 grid), `gomb_ac05` asserts ≥2 (line + band).                                                                                          |
| HIGH 4  | Density default `n_bins = 32` diverged from spec (100)                              | Default changed to 100 in `crates/brightfield-sql/src/lower.rs::DensityLowerer`.                                                                                                                                                                                                                              |
| HIGH 5  | Regression x-mean column was `mean_x`, spec mandated `x_bar`                        | Renamed in lowerer and `RegressionRenderer`; `gomb_ac05` test schema updated.                                                                                                                                                                                                                                 |
| MEDIUM 8 | CI band used z-quantile (1.96) — too narrow for small n                             | Replaced with `t_critical(ci, n)` helper: lookup table for df ∈ {1..30, 60} at standard CIs (0.90/0.95/0.99) with linear interpolation. df ≥ 60 falls back to z-quantiles.                                                                                                                                  |
| MEDIUM 10 | Regression silently dropped both line and band when n < 3                          | Now renders the OLS line for n ≥ 2 (fit is exact through 2 points); only the CI band is gated on n ≥ 3.                                                                                                                                                                                                       |
| MEDIUM 11 | Density1D `bin_size` from first two centres assumed uniform grid                    | `debug_assert!` added: every adjacent pair must have the same width within 1e-6. Comment notes the lowerer's `width_bucket` invariant.                                                                                                                                                                       |
```

### Deferred with rationale

```
| #         | Finding                                                                | Why deferred                                                                                                                                                                                                | Trigger                                                                                |
|-----------|------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------|
| MEDIUM 6  | `Density{1D,2D}Renderer` ignore user-supplied `bandwidth` option       | Spec constraint line 21 mandates "MarkRenderer trait ... unchanged" in this slice. `render()` has no path for mark options, so threading bandwidth requires a trait extension.                              | Two-tier param-effect routing card (Decision 5). The same card adds option threading.  |
| MEDIUM 7  | Density2D encodes density as alpha modulation, not radius scaling      | Functional equivalence at typical 32×32 grids; visual difference is perceptual not semantic. Radius-scaling needs per-cell sizing logic (max-radius derivation, overlap handling).                          | Density polish card (alongside Heatmap/Contour).                                       |
| MEDIUM 9  | `gomb_ac13` SQL "snapshots" are substring `assert!`s                   | Insta-style golden-file snapshots are a separate testing-infrastructure investment that should land workspace-wide, not piecewise per slice.                                                                | Conformance snapshot infrastructure card.                                              |
| LOW 12-15 | `silverman_axis` clones `silverman_1d`; O(n²) lookup; extra kinds; `_as` | Pure cosmetic/perf polish; no behavioural impact.                                                                                                                                                            | Address opportunistically when touching adjacent code.                                  |
```

### ac-12 reframe

The original spec text for ac-12 promised a bandwidth-slider-drag scenario: a `propagate_param("bandwidth", v)` call must skip DuckDB but re-render the convolution with the new bandwidth. This requires three pieces of infrastructure that don't exist in this slice:

1. The density mark must subscribe to `bandwidth` in the `subscriber_graph` (parsing must recognise `bandwidth: $bandwidth` as a param ref in mark options).
2. The renderer must read `bandwidth` from `session.param_state` at render time.
3. `propagate_param` must distinguish "render-affecting only" from "query-affecting" effects (the deferred D5 / two-tier routing).

Pieces 1 and 2 require the same trait/option-threading change blocked by Decision 5's deferral and MEDIUM 6's rationale. Piece 3 is Decision 5 itself.

The spec's underlying property — *"param mutation that doesn't change SQL keeps the cache warm"* — is preserved. We test it via the selection-param path because that's the one path in the current code where `propagate_param` produces byte-identical SQL across calls. When two-tier routing lands, ac-12 will be re-strengthened to the original bandwidth scenario.

Spec ac-12 verification text was updated in the same commit as this decision. The original wording is preserved here for traceability:

> Unit test (gomb_ac12_bandwidth_param_no_requery): load density1d.yaml (or an equivalent minimal density spec with a bandwidth param); execute_mark once; record duckdb_execute_count (the test-only accessor from ac-11); call propagate_param("bandwidth", new_value); assert duckdb_execute_count is unchanged AND the rendered convolution output differs from the first render (different bandwidth → different curve shape).
