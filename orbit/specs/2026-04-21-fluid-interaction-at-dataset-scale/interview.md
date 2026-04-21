# Interview — Card 0003: Fluid Interaction at Dataset Scale (Layer-2 SQL Emission lens)

Card: `orbit/cards/0003-fluid-interaction-at-dataset-scale.yaml`
Rally: `layer 2 SQL emission` (`orbit/specs/2026-04-21-layer-2-sql-emission-rally/rally.yaml`)
Decision pack: `orbit/specs/2026-04-21-fluid-interaction-at-dataset-scale/decisions.md`
Mode: rally design — decision pack authored by forked sub-agent, all seven decisions approved wholesale by the author at the consolidated decision gate.

## Card summary

| Field | Value |
|-------|-------|
| Feature | Fluid interaction at dataset scale |
| As a | analyst exploring a large dataset |
| I want | brushes, filters, and parameter changes to feel instantaneous even on millions to billions of rows |
| So that | I can follow my train of thought without the tool getting in the way |
| Goal | 60+ FPS, <100ms filter response on a multi-million-row cross-filtered dashboard |

Scenarios (4):
1. Interactive framerate on a multi-million-row dashboard
2. Scales to billion-row datasets without blowing up
3. Cross-filter brush keeps both views in sync under 100ms
4. Parameter slider updates downstream queries reactively

**Layer-2 scope for this card.** The rally lens narrows this card to the SQL emission subset of its scenarios: brush-driven predicate changes (scenario 3), slider-driven parameter rebinding (scenario 4), and the <100ms filter response budget (scenarios 1 & 3). Scenarios 1 and 2 also depend on rendering (60+ FPS redraw, billion-row solvency) which constrains the emitter's latency budget but is solved outside Layer 2.

## Context

No SQL-generation code exists in the workspace today. The hole this fills is visible in `crates/brightfield-conformance/src/layer.rs:162-178` — `SqlEquivalenceCheck` is a stub returning `LayerOutcome::Pending { reason: "SQL emitter not yet available" }`, and every curated `<name>.expected.yaml` declares `layer_2: pending`. This card lands the emitter and flips those to `pass`.

The AST (card 0001) is sealed and already carries everything the emitter needs: `Mark { kind, status, data, options }` with `MarkData::From { source, filter_by: Option<ParamRef>, extras }`, `ParamNode`, `SelectionNode`, `ExpressionNode { spans, params }`, and `ValueOrParamRef<T>` at value slots. The vocabulary registry's `ImplStatus::{Implemented | Planned | Unimplemented}` gates which marks the emitter will actually consume.

## Approved decisions

### D1 — AST traversal: AST → typed query IR → SQL

Per-mark visitor for the AST → IR lowering (Mosaic's own `mosaic-sql` shape); shared IR → SQL renderer. Start the IR minimal: `Source | Filter | Projection | Aggregation | Bin | Order | Limit`. Mirror DuckDB's grammar deliberately — DuckDB is the only target. The IR is the substrate later decisions (caching, pre-aggregation, M4) stand on; a pure visitor that emits strings forces every optimisation to walk the SQL text.

### D2 — DuckDB-native dialect with pluggable optimisation passes; v1 ships zero passes registered

`fn(QueryPlan) -> QueryPlan` pipeline. V1 emits correct DuckDB-native SQL for the `Implemented` subset of `MarkKind` (currently empty). Each implemented mark lands its AST→IR lowering and, if its budget demands it, a first-cut pre-aggregation pass. DuckDB idioms on from day one: `parquet_scan()`, `read_csv_auto()`, `FILTER (WHERE …)`, `QUALIFY`, positional `GROUP BY`.

### D3 — Selection compilation: per-view `WHERE` with IR-level `crossfilter` resolution

Each view emits its own `WHERE` clause. Selection is an IR-level `Filter { source_of_contribution, predicates, resolution }` node; `crossfilter` exclusion drops predicates whose `source_of_contribution == this_view.source`. `ExpressionNode` is the carrier for contribution predicates (it already tokenises spans + params — `ast.rs:354-360`); `ExpressionNode::to_sql(param_values)` renders with values either bound via DuckDB `?` placeholders or interpolated as literals per D4.

### D4 — Hybrid parameter binding: prepared statements for scalar params, rebuild `WHERE` for selections

Emitter output is `EmittedQuery { sql: String, bindings: Vec<Binding>, dependencies: QueryDeps }`. Scalar params (`threshold`, slider values) bind as `?` — slider drag dispatches `execute(stmt, &[latest_values])`. Selection params (brush with changing range count, `union` with growing member list) trigger re-emission of the `WHERE` clause only — the rest of the SQL stays; the prepared statement shell is preserved where possible. Structural changes (new mark, swapped source) invalidate the prepared statement.

### D5 — Incremental re-query: shape-cache in v1, result-cache layered later

Both keyed on IR hash. **Shape-cache (v1)** is the prepared-statement path from D4 — `hash(structural_plan)` (excludes param values bound via `?`, includes everything else including `?` positions). **Result-cache (later)** is Arrow-batch-keyed by `hash(full_plan + param_values)`; purely additive, lives in the coordinator per brief §Architecture. Emitter's obligation: a `QueryPlan::hash()` stable across param values — string SQL is not a stable key; this is the reason D1's IR is load-bearing.

### D6 — Vocabulary status: emitter trusts preflight

API: `fn emit(spec: &Spec, preflight: &SupportReport) -> Result<EmittedQuery, EmitError>`. Takes preflight as argument to assert its guarantee — not to re-validate. Debug-only `assert!(mark.status == ImplStatus::Implemented)` for defence-in-depth; release builds trust the gate. A non-`Implemented` node reaching the emitter is an `EmitError::InvariantViolation`. This composes with card 0002's `SupportReport` exactly: preflight rejects the spec, emit never runs.

### D7 — Conformance capture: structural SQL diff via sqlparser-rs, result-set diff as escalation

`sqlparser-rs` (~400KB, mature) parses both the emitter's output and Mosaic's fixture (captured from `mosaic-sql`); both normalise to an AST; compare. Tolerates whitespace/alias variation, catches structural drift. Deviation registry entries (card 0002 D4) slot in as `DEV-NNNN` for acceptable structural diffs. Result-set comparison is the escalation path for semantically-equivalent-but-structurally-different SQL; not shipped in v1. `EmittedQuery` exposes both the SQL text (for conformance) and the IR (for D5 cache key).

## Open questions carried into spec

| ID | Question | Disposition for spec |
|----|----------|----------------------|
| OQ1 | Does v1's corpus (`flights-200k.yaml`, 200k rows) need pre-aggregation to hit <100ms? | Defer the first pass registration until a mark lands that demonstrably needs it. V1 registers the pass-pipeline shape; registers no passes. |
| OQ2 | Empty `union`/`intersect` selection → what `WHERE`? | Match Mosaic JS: empty = no filter (`WHERE TRUE`). Spec ac. |
| OQ3 | Result-cache boundary: emitter or coordinator? | Coordinator (per brief §Architecture). Emitter exposes `QueryPlan::hash()` as the cache-key substrate. |
| OQ4 | Mosaic `mosaic-sql` fixture capture: one-off vendor dump or live Node harness? | One-off vendored dump — cheapest, matches card 0002's curated-corpus shape. Revisit if corpus drift makes it stale. |

## Implementation surface

### New crate: `brightfield-sql` (emitter)

Module layout:
- `ir.rs` — `QueryPlan`, `Source`, `Filter`, `Projection`, `Aggregation`, `Bin`, `Order`, `Limit`, `Predicate`, `SelectionResolution::{Intersect, Union, Crossfilter, Single}`
- `lower.rs` — AST→IR lowering; per-mark visitor trait `trait MarkLower { fn lower(&self, mark: &Mark, ctx: &LowerCtx) -> QueryPlan; }` with one impl per `MarkKind` (default returns `Err(EmitError::InvariantViolation)`)
- `render.rs` — IR→SQL renderer (DuckDB dialect); single match over `QueryPlan` shape
- `emit.rs` — public `fn emit(spec: &Spec, preflight: &SupportReport) -> Result<EmittedQuery, EmitError>`; orchestrates lower+render; assembles `EmittedQuery { sql, bindings, dependencies, plan_hash }`
- `binding.rs` — `Binding`, `QueryDeps`, scalar-vs-selection classification
- `passes.rs` — `trait Pass { fn apply(&self, plan: QueryPlan) -> QueryPlan; }`; empty pass registry for v1
- `error.rs` — `EmitError::{InvariantViolation, UnknownFormat, …}`

### Modified: `crates/brightfield-conformance/src/layer.rs`

`SqlEquivalenceCheck` flips from `LayerOutcome::Pending { … }` to a real pass/fail backed by sqlparser-rs structural diff against a fixture. Shared with card 0004 — 0004 lands the data-source DDL portion first, 0003 lands the query portion on top.

### New conformance fixtures

`crates/brightfield-conformance/vendor/curated/yaml/<name>.expected.sql` (per-spec Mosaic-captured golden SQL). One-off vendored dump per OQ4.

### New dependency

`sqlparser-rs` in `brightfield-sql` and `brightfield-conformance`.

## Cross-card touchpoints

- **Card 0004 (sibling in rally).** 0004 establishes `brightfield-sql` crate scaffold + data-source DDL emission + string-snapshot conformance for the DDL portion. 0003 extends the crate with the IR + query emitter + structural conformance. Both mutate `crates/brightfield-conformance/src/layer.rs` `SqlEquivalenceCheck` — **serial ordering**.
- **Card 0002 (shipped).** `SupportReport` is the gate D6 takes as argument. No change to card 0002's surface.
- **Card 0001 (shipped).** AST, vocabulary registry, `ValueOrParamRef`, `ExpressionNode::spans`, `ImplStatus` — all consumed as-is. No modifications.
