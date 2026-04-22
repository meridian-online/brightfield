# Design Interview — Card 0008: Grammar-of-Graphics Mark Library

Rally: mark lowering and DuckDB execution
Card: orbit/cards/0008-grammar-of-graphics-mark-library.yaml
Date: 2026-04-22
Mode: rally decision-pack (agent-proposed, author-approved)

---

## Q1: How should the 55 MarkKind variants be organised for implementation?

**Decision:** ~10 mark family lowerers parameterised by axis orientation + variant flags.

Families: Line, Bar, Dot, Area, Rect, Rule, Text, Density, Regression, Geo. Each family implements `MarkLower` once. The `default_lowerers()` registry maps each `MarkKind` to its family + config. Adding a new variant is a one-line registry entry.

Evidence: Observable Plot groups marks by channel semantics internally. Corpus confirms lineY/lineX differ only in axis (mark-types.yaml, line-multi-series.yaml). The 55→10 reduction makes the library tractable.

---

## Q2: How do mark options (channels) become SQL column expressions?

**Decision:** Typed `ChannelMap` extracted during lowering.

`ChannelMap { position: Vec<PositionChannel>, aesthetic: Vec<AestheticChannel>, transform: Vec<TransformChannel> }`. Shared function `resolve_channels(options, mark_family) -> ChannelMap` handles three SpecValue shapes: bare string (column ref), object with transform key (bin/count/sum/avg), param ref.

ChannelMap is the keystone type — consumed by both the lowerer (to build QueryPlan) and the renderer (to read the right columns from RecordBatch). Getting this right is the highest-leverage investment.

---

## Q3: Where do transforms (bin/count/density/regression) compute?

**Decision:** Simple transforms in SQL; statistical transforms (density/KDE) client-side in Rust.

- `bin`, `count`, `sum`, `avg`, `min`, `max` → SQL via existing QueryPlan::Aggregation and QueryPlan::Bin IR nodes.
- `regression` → SQL using DuckDB's built-in `regr_slope()`/`regr_intercept()`.
- `density` (1D/2D KDE) → client-side in Rust. Mark's SQL fetches filtered raw data; Rust KDE routine (Gaussian kernel, configurable bandwidth) computes density on the Arrow record batch. `contour`, `heatmap`, `denseLine` all consume the same KDE output.

Evidence: DuckDB has no native KDE function. Post-filter datasets (e.g. flights-density.yaml after crossfilter brush) are small enough for in-memory Rust computation. Arrow record batches are in-process — no serialisation boundary.

---

## Q4: How do Arrow record batches become rendered geometry?

**Decision:** `MarkRenderer` trait per family, parallel to `MarkLower`.

`trait MarkRenderer { fn render(&self, batch: &RecordBatch, channel_map: &ChannelMap, scales: &ScaleSet) -> Vec<GpuiElement>; }`

One impl per mark family (~10 impls). Pipeline: Mark → MarkLower::lower() → QueryPlan → SQL → DuckDB → RecordBatch → MarkRenderer::render() → gpui-plot elements.

ChannelMap bridges the two: tells the lowerer which columns to project, tells the renderer which columns to read.

---

## Q5: How are scales inferred and resolved?

**Decision:** Infer from Arrow RecordBatch post-query.

Pipeline: SQL → DuckDB → RecordBatch → `infer_scales(batch, channel_map, plot_attrs) -> ScaleSet` → `MarkRenderer::render(batch, channel_map, scales)`.

- Arrow schema types drive scale-type selection: numeric → linear, string → band, date → temporal.
- Domain computed from column data unless overridden by spec attributes.
- `xDomain: Fixed` cached at plot level across re-queries (crossfilter pattern).
- One query per mark — no extra extent queries.

---

## Q6: What is the implementation phasing?

**Decision:** Frequency-ordered with tier awareness.

1. **Phase 1 (minimum viable):** dot, lineY, barY, rectY — 4 marks covering the most corpus specs. Proves all four geometry families (point, line, rect, area).
2. **Phase 2:** areaY, text, ruleY/ruleX, tickY/tickX — completes core tier.
3. **Phase 3:** regressionY — simple SQL-side aggregate, quick win.
4. **Phase 4:** density, contour, heatmap, denseLine — share KDE infrastructure.
5. **Phase 5:** geo, hexbin — require spatial extension or custom computation.

Each phase flips `ImplStatus` in vocab.rs from `Unimplemented` to `Implemented`. Per-mark SQL emission produces conformance snapshots.

---

## Out of Scope

- Interactor wiring (highlight, pan, zoom — separate cards)
- Input widgets
- Layout/composition (hconcat/vconcat)
- Legend rendering
- Faceting (axisFx/axisFy)

## Key References

- brightfield-brief.md (mark catalogue, gpui-plot commitment, Arrow transport)
- crates/brightfield-spec/src/ast.rs:293-304 (Mark struct)
- crates/brightfield-spec/src/vocab.rs:101-186 (MarkKind vocabulary, ImplStatus)
- crates/brightfield-sql/src/lower.rs:29-32 (MarkLower trait)
- crates/brightfield-sql/src/ir.rs (QueryPlan IR)
- Vendored corpus: crates/brightfield-spec/vendor/mosaic-specs/yaml/ (54 specs)
