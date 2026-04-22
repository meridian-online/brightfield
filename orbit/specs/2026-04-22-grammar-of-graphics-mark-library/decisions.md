# Decision Pack — Card 0008: Grammar-of-Graphics Mark Library

Rally: **mark lowering and DuckDB execution**.
Card: `orbit/cards/0008-grammar-of-graphics-mark-library.yaml`.
Scope: deciding how brightfield implements the full mark library — from AST `Mark` nodes through SQL lowering (`MarkLower` trait) to rendered geometry on screen (gpui-plot). Covers core marks (lineY, barY, dot, areaY, rect, text, rule), statistical marks (density, regression), and specialised marks (geo, hexbin, contour, raster/heatmap).

## What is already fixed (not up for debate here)

These are inherited from completed cards and the existing codebase:

- **Mark AST structure** (`crates/brightfield-spec/src/ast.rs:293-304`): `Mark { kind: MarkKind, status: ImplStatus, data: Option<MarkData>, options: IndexMap<String, ValueOrParamRef<SpecValue>> }`. Options are a flat bag; no per-mark typed option structs yet.
- **MarkKind vocabulary** (`crates/brightfield-spec/src/vocab.rs:101-186`): 55 mark variants registered, all `Unimplemented`. Wire names match Mosaic 0.24.x exactly.
- **MarkLower trait** (`crates/brightfield-sql/src/lower.rs:29-32`): `fn lower(&self, mark: &Mark, ctx: &LowerCtx) -> Result<QueryPlan, EmitError>`. Extension point exists; no concrete implementations registered in `default_lowerers()`.
- **QueryPlan IR** (`crates/brightfield-sql/src/ir.rs`): Source, Filter, Projection, Aggregation, Bin, Order, Limit variants. Render to DuckDB SQL via `render_query()`.
- **Selection compilation** (`crates/brightfield-sql/src/lower.rs:71-104`): crossfilter, union, intersect, single resolution already implemented.
- **Data source views** (card 0004): sources registered as `CREATE OR REPLACE VIEW` at spec-mount; marks reference bare table names.
- **Arrow record batches** as data transport between DuckDB and renderer (card 0012's contract).
- **gpui-plot** as the rendering foundation (project brief commitment).
- **Vendored corpus**: 54 Mosaic specs at `crates/brightfield-spec/vendor/mosaic-specs/yaml/`; `mark-types.yaml` exercises barY, lineY, text, tickY, areaY, regressionY, hexbin, contour, heatmap, denseLine in a single spec.

---

## Decision 1 — Mark family taxonomy: how to organise the 55 MarkKind variants

### Context
The card requires implementing marks across three tiers (core, statistical, specialised). The `MarkLower` trait requires one implementation per `MarkKind`, but many marks share identical lowering logic — `lineY` and `lineX` differ only in which axis carries the dependent variable; `barY` and `barX` are the same shape with swapped axes. Implementing 55 independent lowerers is prohibitive; grouping them by shared geometry and SQL patterns makes the library tractable. The question is: what is the right grouping, and does each group share a single `MarkLower` impl or does each variant get a thin wrapper?

### Options
- **A. Family-based lowerers with axis configuration.** Define ~10 "mark families" (line, bar, dot, area, rect, rule, text, density, regression, geo) each implemented as a single struct parameterised by axis orientation (X/Y/XY) and variant flags (e.g. `bar` vs `cell` vs `waffle`). Register all 55 `MarkKind` variants by mapping each to its family + config. The `MarkLower::lower()` call resolves to the family impl.
- **B. One lowerer per MarkKind.** 55 structs, each with its own `lower()`. Shared logic extracted into utility functions but no formal family concept.
- **C. Trait hierarchy: `MarkFamily` trait with subtrait per family.** `LineFamilyLower`, `BarFamilyLower`, etc. Each subtrait has a default `lower()` that individual variants can override.

### Trade-offs
- **A (family + config)** — matches Observable Plot's own internal structure (Plot groups marks by "channel semantics" — line marks all share the same channel resolution logic, just with swapped x/y). The 55 → ~10 reduction makes implementation and testing tractable. The axis parameter is a natural decomposition — corpus evidence shows `lineY` and `lineX` differ only in which channel is the "position" channel (see `mark-types.yaml:29` vs `line-multi-series.yaml`). Cost: the family struct needs enough configuration surface to handle variant differences (e.g. `dot` vs `circle` differ in size semantics); over-parameterisation risks a "god object".
- **B (per-variant)** — maximum flexibility, no abstraction to fight. Cost: 55 implementations to write and maintain, extensive code duplication. The `crossfilter.yaml` spec uses `rectY` — its lowering logic (bin + count + filterBy) would be copy-pasted for `rectX`, `barY`, `barX`, etc.
- **C (trait hierarchy)** — Rust's trait system supports this but ergonomics are poor (no default method specialisation in stable Rust; subtrait dispatch requires dynamic dispatch or manual enum matching). Over-engineered for the problem.

### Recommendation
**Option A.** Define a `MarkFamilyLowerer` struct parameterised by `MarkFamily` enum + `AxisOrientation` + variant-specific flags. The `default_lowerers()` registry maps each `MarkKind` to its family config. This is the "appropriate framework for marks" the user asked for — invest the time in the family abstraction so that adding a new mark variant later is a one-line registry entry, not a new lowerer. Target ~10 families: Line, Bar, Dot, Area, Rect, Rule, Text, Density, Regression, Geo. Each family's `lower()` produces the `QueryPlan` tree appropriate to its geometry (e.g. Line family always produces Source → Filter → Projection → Order; Bar family produces Source → Filter → Bin → Aggregation).

---

## Decision 2 — Channel resolution: how mark options become SQL columns

### Context
Mosaic marks carry channels as options: `x: Date`, `y: Close`, `fill: sex`, `x: { bin: delay }`, `y: { count: }`. These must be resolved into SQL column expressions for the `QueryPlan`. The current AST stores channels as `ValueOrParamRef<SpecValue>` in the flat `options` bag — there is no typed "channel" concept in the IR. The lowerer must interpret `x: Date` as "project column Date", `x: { bin: delay }` as "bin column delay into x", and `y: { count: }` as "COUNT(*) grouped by x". Observable Plot defines ~20 channel names (x, y, x1, x2, y1, y2, z, fill, stroke, opacity, r, symbol, text, href, title, etc.) with type-dependent semantics.

### Options
- **A. Typed ChannelMap extracted during lowering.** The lowerer's first step is to parse the `options` bag into a `ChannelMap { position: Vec<PositionChannel>, aesthetic: Vec<AestheticChannel>, transform: Vec<TransformChannel> }` where each channel carries its resolved column expression and any transform (bin, count, sum, etc.). The `ChannelMap` is the input to `QueryPlan` construction.
- **B. Ad-hoc option inspection per family.** Each mark family reaches into `options` for the keys it cares about (e.g. Line looks for `x`, `y`, `stroke`, `z`; Bar looks for `x`, `y`, `fill`). No shared channel abstraction.
- **C. Schema-driven channel validation.** Define a per-`MarkKind` channel schema (required/optional channels with expected types) and validate + extract in a single pass before lowering.

### Trade-offs
- **A (ChannelMap)** — mirrors how Observable Plot works internally (channels are resolved to "scaled" and "unscaled" columns before rendering). The typed intermediate makes it possible to share channel-resolution logic across all 10 families — `bin` and `count` transforms work identically regardless of whether they appear on a bar, rect, or area mark. Cost: designing the `ChannelMap` type is upfront work, and the mapping from `SpecValue` variants (`String`, `Object { bin: ... }`, `Object { count: }`) to typed channels needs a well-defined dispatch table.
- **B (ad-hoc)** — fastest to start, but every family reimplements "parse `x: { bin: delay }` into a binning expression". Corpus evidence: `crossfilter.yaml`, `flights-200k.yaml`, and `flights-density.yaml` all use `x: { bin: delay }` and `y: { count: }` on different mark types (rectY, barY) — the transform resolution must be identical across them.
- **C (schema-driven)** — most correct but highest cost. Observable Plot has no formal channel schema — channels are permissive and marks silently ignore channels they don't use. Enforcing a schema would reject valid Mosaic specs.

### Recommendation
**Option A.** Introduce a `ChannelMap` type in `brightfield-sql` that the lowerer populates from the mark's `options` bag. Channel resolution is a shared function (`resolve_channels(options, mark_family) -> ChannelMap`) that handles the three `SpecValue` shapes: bare string (column reference), object with transform key (`bin`, `count`, `sum`, `avg`, etc.), and param ref. The `ChannelMap` then drives `QueryPlan` construction — position channels become Projection/Order, transforms become Aggregation/Bin, aesthetics become additional projected columns. This is the investment that pays off across all 10 families.

---

## Decision 3 — Transform lowering: where do bin/count/density compute?

### Context
Mosaic marks carry inline transforms: `x: { bin: delay }`, `y: { count: }`, `y: { avg: salary }`, bandwidth/thresholds on density marks. These transforms determine the SQL shape: `bin` adds a `width_bucket()` call (already an IR variant), `count` adds `COUNT(*)` with GROUP BY, `avg`/`sum`/`min`/`max` add aggregate functions. Statistical transforms (density, regression) are more complex — 1D/2D KDE and linear regression are not standard SQL. The card says density and regression should draw "from the underlying data without a separate precompute step".

### Options
- **A. All transforms in SQL.** `bin`/`count`/`avg` are standard SQL (already supported by QueryPlan IR). Density (KDE) uses DuckDB's window functions or a UDF. Regression uses DuckDB's `regr_slope()`/`regr_intercept()` aggregate functions. Everything runs server-side in DuckDB.
- **B. Simple transforms in SQL, statistical transforms client-side.** `bin`/`count`/`avg` lower to SQL. Density and regression fetch raw data via SQL and compute in Rust on the Arrow record batch before rendering.
- **C. All transforms client-side.** SQL only does source + filter; all aggregation and computation happens in Rust on the fetched Arrow data.

### Trade-offs
- **A (all SQL)** — maximises DuckDB pushdown; for the `gaia.yaml` spec (5M rows), computing density server-side avoids transferring all rows to the renderer. DuckDB has `regr_slope(y, x)` and `regr_intercept(y, x)` built-in, making regression trivial. KDE is harder: DuckDB has no native KDE function, so a true 2D KDE in SQL requires a self-join or a UDF extension — complex and potentially slow. Cost: `QueryPlan` IR needs new variants for statistical functions, and the KDE SQL would be fragile.
- **B (hybrid)** — simple transforms use DuckDB's strength (aggregation at scale); statistical transforms use Rust's strength (numerical computation on moderate-sized data). Density marks in the corpus (e.g. `flights-density.yaml`, `density1d.yaml`, `density2d.yaml`) operate on datasets that, after filtering, are small enough to transfer. The Arrow record batch is already in-process (no serialisation boundary per card 0012). Cost: two compute paths, and the Rust-side KDE needs a numerical library (or hand-rolled implementation).
- **C (all client-side)** — simplest SQL (just SELECT * with filters). Loses: defeats the purpose of DuckDB for aggregation; transferring 200K+ rows for a `y: { count: }` histogram is wasteful when DuckDB can return a 50-row grouped result.

### Recommendation
**Option B.** Lower `bin`, `count`, `sum`, `avg`, `min`, `max` to SQL via the existing `QueryPlan::Aggregation` and `QueryPlan::Bin` IR nodes. Lower `regression` to SQL using DuckDB's built-in `regr_slope()`/`regr_intercept()` aggregates (renders as a two-point line from the regression parameters). Lower `density` (1D and 2D KDE) client-side in Rust: the mark's SQL fetches the filtered raw data, and a Rust KDE routine (Gaussian kernel, configurable bandwidth) computes the density surface on the Arrow record batch before passing to the renderer. This matches the card's "without a separate precompute step" requirement — the compute happens inline during the mark's render cycle, not as a separate pipeline stage. The `contour`, `heatmap`, and `denseLine` marks all consume the same KDE output and differ only in how they visualise it (isolines vs colour-mapped grid vs line opacity).

---

## Decision 4 — Rendering architecture: how Arrow record batches become geometry

### Context
gpui-plot is the committed rendering foundation. Each mark type needs to map Arrow columns to visual geometry (lines, rectangles, circles, text glyphs, etc.). The question is whether each mark family defines its own rendering path from Arrow to gpui-plot primitives, or whether there is a shared "mark renderer" abstraction that maps channel semantics to geometry generically. Arrow record batches arrive with columns named by the SQL projection — the renderer must know which column maps to which visual property (x-position, y-position, fill colour, etc.).

### Options
- **A. MarkRenderer trait per family.** Define `trait MarkRenderer { fn render(&self, batch: &RecordBatch, channel_map: &ChannelMap, scales: &ScaleSet) -> Vec<GpuiElement>; }` with one impl per mark family (~10 impls). Each impl knows how to map its channels to gpui-plot primitives.
- **B. Generic geometry mapper.** A single renderer that reads channel semantics from `ChannelMap` and dispatches to gpui-plot primitives based on the mark family's "geometry type" (point, line, rect, area, text). Mark families declare their geometry type; the renderer handles layout.
- **C. Direct gpui-plot calls in the lowerer.** No abstraction — each mark family's lowering produces both SQL and a rendering closure that runs when the Arrow batch arrives.

### Trade-offs
- **A (trait per family)** — clear separation of concerns; each family's renderer is self-contained and testable. Cost: 10 renderer impls, but they are simple (map columns to coordinates, apply scales, emit gpui-plot elements). Matches the SQL-side family taxonomy from Decision 1.
- **B (generic mapper)** — fewer implementations, but the generic mapper must handle every geometry type's quirks (area marks need stacking, bar marks need baseline alignment, text marks need font metrics). The abstraction would leak family-specific logic, becoming a god function.
- **C (rendering closures)** — tightest coupling between SQL and rendering. Loses: the lowerer becomes responsible for both SQL generation and rendering, violating single responsibility. Testing requires a gpui-plot context.

### Recommendation
**Option A.** Define a `MarkRenderer` trait parallel to `MarkLower`, with one impl per mark family. The rendering pipeline is: `Mark` → `MarkLower::lower()` → `QueryPlan` → SQL → DuckDB → `RecordBatch` → `MarkRenderer::render()` → gpui-plot elements. The `ChannelMap` (Decision 2) bridges the two: it tells the lowerer which columns to project and tells the renderer which columns to read from the batch. Scale resolution (mapping data values to pixel coordinates and colours) is a shared concern consumed by all renderers — it lives outside the trait as a `ScaleSet` passed to `render()`.

---

## Decision 5 — Scale inference and resolution strategy

### Context
Observable Plot auto-infers scales from data and channel types: a numeric `x` channel gets a linear scale, a string `x` gets a band scale, `fill` with a string gets an ordinal colour scale, `r` gets a sqrt scale. Specs can override with plot-level attributes (`xDomain`, `colorScheme`, `yDomain: Fixed`, etc. — see `crossfilter.yaml:17` `xDomain: Fixed`, `athletes.yaml:58` `xyDomain: Fixed`). Scale resolution must happen after data is available (domain inference needs min/max from the record batch) but before rendering (pixel mapping needs scales). The question is where and how this inference runs.

### Options
- **A. Infer scales from RecordBatch metadata.** After query execution, scan the Arrow schema + batch statistics to determine scale type and domain. Override with any spec-declared scale attributes. Produce a `ScaleSet` before rendering.
- **B. Infer scales in SQL.** Add MIN/MAX/DISTINCT queries to the lowerer's output for each channel. The renderer receives pre-computed domain bounds alongside the data batch.
- **C. Two-pass rendering.** First pass collects data extents from the batch; second pass renders with resolved scales. No extra SQL.

### Trade-offs
- **A (batch metadata)** — Arrow record batches from DuckDB carry schema types (Int64, Utf8, Float64, etc.) which directly map to scale types. Domain inference (min/max for quantitative, unique values for ordinal) requires a scan of the column data, but this is an O(n) pass over in-memory Arrow arrays — trivial for the record batch sizes that survive SQL aggregation. `xDomain: Fixed` (crossfilter pattern) means "hold the domain across re-queries" — this is a rendering-level concern, not a SQL concern. Cost: adds a domain-inference step in Rust between query return and render.
- **B (SQL-side)** — pushes domain computation to DuckDB, reducing Rust-side work. Cost: doubles the query count (one for data, one for extents per channel), complicates the reactive re-emission path (param change now re-runs extent queries too), and `Fixed` domains still need client-side caching regardless.
- **C (two-pass)** — functionally equivalent to A but with explicit pass separation. Cost: no real advantage over A since Arrow arrays support random access; the "two passes" are just two iterations over the same in-memory data.

### Recommendation
**Option A.** Infer scales from the Arrow record batch after query execution. The pipeline becomes: SQL → DuckDB → `RecordBatch` → `infer_scales(batch, channel_map, plot_attrs) -> ScaleSet` → `MarkRenderer::render(batch, channel_map, scales)`. Arrow schema types drive scale-type selection (numeric → linear, string → band, date → temporal). Domain is computed from column data unless overridden by spec attributes. `Fixed` domains are cached at the plot level across re-queries. This keeps the query count minimal (one query per mark) and concentrates scale logic in a single Rust module that all mark renderers consume.

---

## Decision 6 — Implementation phasing: which marks ship first

### Context
The card names three tiers — core (lineY, barY, dot, areaY, rect, text, rule), statistical (density, regression), specialised (geo, hexbin, contour, raster/heatmap). Implementing all 55 variants simultaneously is impractical. The question is the implementation order, which determines what the first usable release can render and which corpus specs pass end-to-end.

### Options
- **A. Core first, statistical second, specialised third.** Ship lineY, barY, dot, areaY, rect, text, rule as the initial set. This covers the card's first scenario and the most-used marks in the corpus (lineY appears in 8 specs, barY in 5, dot in 12, rectY in 4).
- **B. Coverage-maximising order.** Prioritise by corpus frequency: dot (12), lineY (8), barY (5), rectY (4), areaY (3), text (3), ruleY (2), then density/contour/heatmap (4 specs combined), then geo (8 specs — but all require spatial extension).
- **C. Vertical slice per spec.** Pick one complex corpus spec (e.g. `crossfilter.yaml`) and implement everything it needs end-to-end, then expand.

### Trade-offs
- **A (tier-based)** — aligns with the card's own scenario structure. Delivers a functional dashboard toolkit quickly (7 mark types cover ~80% of non-specialised corpus specs). Statistical and specialised marks can be separate cards if needed. Cost: delays density/regression which appear in visually impressive demo specs.
- **B (frequency-based)** — maximises corpus pass rate fastest. Very similar to A in practice, since the most frequent marks are the core marks. Adds rectY early (crossfilter is a key demo spec).
- **C (vertical slice)** — proves the full pipeline works for one real spec. Cost: `crossfilter.yaml` needs rectY + bin + count + intervalX interactor + crossfilter selection — pulls in interactor support which is a different card's concern.

### Recommendation
**Option B with tier awareness.** Implement in this order: (1) dot, lineY, barY, rectY — covers the most corpus specs and proves all four geometry families (point, line, rect, area). (2) areaY, text, ruleY/ruleX, tickY/tickX — completes the core tier. (3) regressionY — simple SQL-side aggregate, quick win. (4) density, contour, heatmap, denseLine — share the KDE infrastructure from Decision 3. (5) geo, hexbin — require spatial extension or custom computation. Each phase should flip the relevant `MarkKind` status from `Unimplemented` to `Implemented` in vocab.rs and add conformance tests. Phase 1 (4 marks) is the minimum viable delivery for this card.

---

## Summary table

```
| #  | Decision                                    | Recommendation                                                       |
|----|---------------------------------------------|----------------------------------------------------------------------|
| 1  | Mark family taxonomy                        | ~10 family lowerers parameterised by axis + variant flags            |
| 2  | Channel resolution                          | Typed ChannelMap extracted during lowering; shared across families    |
| 3  | Transform lowering                          | Simple transforms in SQL; density/KDE client-side in Rust            |
| 4  | Rendering architecture                      | MarkRenderer trait per family; ChannelMap bridges SQL and rendering   |
| 5  | Scale inference                              | Infer from Arrow RecordBatch post-query; cache Fixed domains         |
| 6  | Implementation phasing                      | Frequency-ordered: dot/lineY/barY/rectY first, then expand by tier   |
```

## Cross-cutting notes

- **Interaction with card 0012 (DuckDB execution engine):** Decisions 3 and 5 depend on DuckDB returning Arrow record batches in shared memory. The `MarkRenderer` (Decision 4) consumes those batches directly. Implementation of card 0008 should proceed in lockstep with card 0012 — the lowering side (Decisions 1-3) can be built and tested against SQL string output, while the rendering side (Decisions 4-5) requires the execution engine to be functional.
- **ChannelMap is the keystone:** Decisions 1, 2, 4, and 5 all flow through the `ChannelMap` type. Getting its design right is the highest-leverage investment in this card. It should be the first thing specified and reviewed.
- **vocab.rs status transitions:** Each implemented mark should flip from `Unimplemented` to `Implemented` in `crates/brightfield-spec/src/vocab.rs`. The preflight `SupportReport` (card 0002) already reads these statuses — implementing a mark automatically makes it pass preflight.
- **Conformance:** Per-mark SQL emission should produce conformance snapshots (extending the card 0004 pattern). The rendering side is harder to snapshot — consider pixel-diff or structural-geometry comparison for a future conformance card.
- **Out of scope for this card:** interactor wiring (highlight, pan, zoom — separate card), input widgets, layout/composition (hconcat/vconcat), legend rendering, faceting (axisFx/axisFy). These consume mark output but are not part of the mark library itself.
