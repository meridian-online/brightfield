# Decision Pack — Card 0003: Fluid Interaction at Dataset Scale (Layer-2 SQL Emission lens)

Rally goal: **layer 2 SQL emission** — the stage that converts the Mosaic spec AST (shipped by cards 0001/0002) into DuckDB-dialect SQL the query engine runs.

Scope for this pack: the subset of card 0003's scenarios that Layer-2 emission directly enables — brush-driven predicate changes, slider-driven parameter rebinding, and the <100ms filter response budget. Scenarios that depend on rendering (60+ FPS redraw, billion-row dashboard solvency) constrain the emitter's latency budget but are not solved at this layer.

Evidence citations use repo-relative paths. Prior decisions referenced:
- `orbit/specs/2026-04-20-mosaic-spec-driven-visualisation/decisions.md` (card 0001 — AST, vocabulary registry, `ValueOrParamRef<T>`, `ExpressionNode`, `ParamRef`).
- `orbit/specs/2026-04-20-mosaic-web-spec-portability/decisions.md` (card 0002 — four-layer conformance model, `SupportReport`, deviation registry, `LayerCheck` trait).

Shipped-code touchpoints:
- `crates/brightfield-spec/src/ast.rs` — `Spec`, `Component`, `Mark { kind, status, data, options }`, `MarkData::From { source, filter_by, extras }`, `ParamNode`, `SelectionNode`, `ExpressionNode { spans, params }`, `ValueOrParamRef`, `SpecValue::{Param, Expression, ...}`.
- `crates/brightfield-spec/src/vocab.rs` — `*Kind` enums tagged with `ImplStatus::{Implemented, Planned, Unimplemented}`.
- `crates/brightfield-conformance/src/layer.rs` — `ConformanceLayer::SqlEquivalence = 2`, `LayerCheck` trait, and a `SqlEquivalenceCheck` stub that returns `LayerOutcome::Pending { reason: "SQL emitter not yet available" }`. That `Pending` literal is the hole this rally fills.
- `crates/brightfield-conformance/src/expectations.rs` — `LayerNExpectation::{Pass | Pending | Suppressed(DEV-NNNN)}`; the curated corpus's `<name>.expected.yaml` files (`line.expected.yaml`, `crossfilter.expected.yaml`, etc.) all declare `layer_2: pending` today.
- `crates/brightfield-spec/vendor/mosaic-specs/yaml/crossfilter.yaml` — the exact two-view `rectY` + `intervalX` + `filterBy: $brush` shape the card's cross-filter scenario targets.

No SQL-generation code exists in the workspace today (`rg` across `crates/` for `sql`/`emit`/`emission`/`layer_2` returns only conformance/fixture references). This pack decides the starting shape.

---

## D1 — AST traversal strategy: how does the emitter consume the AST?

**Context.** The AST (card 0001) is a sealed tree of Rust structs/enums: `Spec → Component → {Plot, HConcat, VConcat, Legend, Mark, Interactor, Input}`, with `Mark { kind: MarkKind, status: ImplStatus, data: Option<MarkData>, options: IndexMap<String, ValueOrParamRef<SpecValue>> }` as the hot node (see `crates/brightfield-spec/src/ast.rs:293-304`). Every mark kind needs a SQL recipe (binning, aggregation, stacking, M4 downsampling, pre-aggregation). The emitter must turn a `Mark` into a SQL query and the AST's surrounding `filterBy`/selection wiring into a `WHERE` clause. How does control flow through the AST?

**Options.**

- **A. Inline match on `MarkKind`, one big function per mark family.** A free function `emit_mark(mark, ctx) -> Sql` that dispatches on `mark.kind` and writes SQL directly. Everything the match arm needs (binning, filter predicates, pre-aggregation) is threaded through via a `Context` struct.
- **B. Visitor trait with one method per mark kind.** `trait MarkEmitter { fn emit_line_y(&self, mark: &Mark, ctx: &Ctx) -> Sql; fn emit_bar_y(...); ... }`. Default impls return an `Unimplemented` stub; concrete impls override per kind.
- **C. AST → typed query IR → SQL.** Lower the AST to an intermediate representation modelled on SQL semantics — `QueryPlan { source: Source, filters: Vec<Predicate>, group_by: Vec<Expr>, aggregations: Vec<Agg>, select: Vec<Projection>, limit: Option<u64> }` — then render the IR to a SQL string. The AST→IR step is per-mark; the IR→SQL step is dialect-specific and shared.

**Trade-offs.**

- **A.** Cheapest to write — ~50 match arms and done. Fights the `Implemented | Planned | Unimplemented` status enum, because every new mark kind forces editing the same mega-function. No natural seam for the later-card work (pre-aggregation, M4, result caching) — every optimisation becomes another flag threaded through `Ctx`. Conformance layer-2 tests compare SQL strings; string concatenation in match arms makes diff-shaped errors ("expected newline, got space") noisy.
- **B.** Aligns 1:1 with Mosaic's JS codebase (`packages/vgplot/spec/src/ast/PlotMarkNode.js` and the `query()` method each mark kind exposes — Mosaic literally has one method per mark's query shape). Easy to add a new mark without touching the others. But bypasses the IR layer entirely — pre-aggregation, cache-keying, M4 downsampling, and cross-filter `WHERE` merging each need a pass over "the final query", not "the mark-specific code". With pure-visitor emission, those passes have to walk the SQL string or re-synthesise structure. That's the same trap the brief §3.2 warns against (database-first, push work to the engine — needs a structured plan to reason about).
- **C.** The IR is the substrate the rally actually needs. Selection predicates merge at the IR layer (`plan.filters.push(predicate)`); pre-aggregation is a rewrite rule on the IR (`Bin + Count → GroupBy`); cache keys are `hash(plan)` rather than `hash(sql_string)` and so survive whitespace changes; M4 downsampling is an IR-level rewrite gated on row estimates. The cost is non-trivial — an IR type is real work, and writing the IR→SQL renderer is a second phase. Carries precedent risk: if the IR diverges from DuckDB's grammar, it traps us.

**Recommendation: C.**

The card's <100ms budget is met by not re-emitting from scratch on every brush drag (see D5). That requires a diffable query representation, and the IR is that representation. A pure visitor (B) forces every downstream concern — caching, pre-aggregation, brush re-query — to walk strings, which is worse than useless when the whole point of layer 2 is reasoning about query shape. Start the IR minimal: `Source | Filter | Projection | Aggregation | Bin | Order | Limit`. Mirror DuckDB's grammar deliberately — DuckDB is the only target (brief README: "DuckDB in-process via duckdb-rs"). The IR→SQL renderer is a single match. Mark-specific AST→IR lowering uses a visitor interface (B's shape, C's output) so new marks land without touching others.

Evidence: Mosaic's own `mosaic-sql` package (`/Users/hugh/github/uwdata/mosaic/packages/sql/src/*`) builds a `Query` builder object, not raw strings — exactly the IR shape. The brief README §Architecture names "Query Engine (SQL gen)" as a component equivalent to `mosaic-sql`, strongly implying parity. Conformance-layer-2 expectations (`crates/brightfield-conformance/vendor/curated/yaml/*.expected.yaml`) all say `layer_2: pending`, and will flip to `layer_2: pass` when the IR-based emitter's golden SQL matches Mosaic's — see D6.

---

## D2 — SQL dialect and query shape: what exactly does the emitter target?

**Context.** DuckDB is the only engine target (README §Technology Stack — "DuckDB via duckdb-rs ... best-in-class analytical database, in-process, Arrow-native"). The emitter can either stay in a generic-ANSI subset or lean on DuckDB's analytical extensions (`LIST_AGG`, `FILTER`, columnar `SAMPLE`, `ASOF JOIN`, `UNPIVOT`, `PIVOT`, positional arguments, `QUALIFY`, `VALUES (...)` table literals, the `parquet_scan` and `read_csv_auto` table functions). Further: card 0003 explicitly demands <100ms filter response on multi-million-row data, which effectively mandates pre-aggregation and M4 downsampling for mark families that would otherwise scan the full table on every brush (density, line, raster).

**Options.**

- **A. ANSI-portable SQL.** Emit only SQL that would run on PostgreSQL and DuckDB. Wrap data sources as `FROM table_name` strings. Pre-aggregation, M4, and `SAMPLE` become user-supplied wrapping queries, not emitter concerns.
- **B. DuckDB-native emission, no optimisation passes in v1.** Use DuckDB's `parquet_scan('path.parquet')`, `read_csv_auto`, positional aggregates, `QUALIFY`, and `FILTER (WHERE ...)`. Emit straight-line SQL for every mark; rely on DuckDB's planner and the row count staying modest in v1. No pre-aggregation, no M4, no density-grid rewrite.
- **C. DuckDB-native + pre-aggregation + M4 as pluggable optimisation passes.** Same DuckDB base as B, but run the AST→IR output through an optimiser pipeline before rendering. Passes in the pipeline: pre-aggregation (bin at pixel resolution for `barY`, `rectY`, `rect`), M4 downsampling (for `lineY`, `areaY`), density-grid rewrite (for `density`, `raster`, `hexbin`). Each pass is opt-in via a pass registry; v1 ships with none wired, but the pipeline exists.

**Trade-offs.**

- **A.** Gains engine-portability as an option for later. Loses on day one against the card's budget: a naive `SELECT count(*) GROUP BY bin` over 10M rows is already close to the 100ms budget, and M4 downsampling has no ANSI equivalent. Brief README explicitly names DuckDB as "the query engine" — portability is not a stated goal and buying it costs real performance now.
- **B.** Minimum viable — gets the emitter to "SQL runs, produces results" quickly. Loses on the card's 60+ FPS + <100ms scenarios over multi-million-row data: M4 and pre-aggregation aren't optional at that scale, they're the mechanism by which the budget is met. Leaving the optimisation seam for later means retrofitting it, which invariably pollutes the emitter with feature flags.
- **C.** Gains the seam cleanly — the IR is the substrate and optimisation passes are IR→IR rewrites. V1 can ship with zero passes registered and still emit correct SQL; card 0006 or later lands pre-aggregation and flips its flag on. The cost is designing the pipeline before we have the rewrites to justify it. Risk: if the IR doesn't cleanly admit these passes, we discover that late.

**Recommendation: C, with the caveat that v1 ships zero passes registered.**

The IR chosen in D1 is the carrier; passes are `fn(QueryPlan) -> QueryPlan`. For v1, the emitter should produce correct DuckDB-native SQL for the `Implemented` subset of `MarkKind` — which is empty today (`vocab.rs` declares every mark `Unimplemented`). As marks are implemented (card 0008 territory), each lands both its AST→IR lowering and, if the card's budget demands it, a first-cut pre-aggregation pass. Defining the pass registry now prevents the "this doesn't belong" refactor later.

Concrete DuckDB idioms the emitter should lean on from day one: `parquet_scan('...')` / `read_csv_auto('...')` for data-source lowering (card 0004 territory but the emitter consumes it), `FILTER (WHERE ...)` for per-mark selective aggregates, `QUALIFY` for top-N per group, `WITH RECURSIVE` for hierarchical filters, and positional `GROUP BY 1, 2` where it simplifies emitted SQL.

Evidence: brief README §Features lists "automatic pre-aggregation at pixel resolution, M4 downsampling for line/area marks" as a first-class feature — these are non-negotiable for the <100ms budget. Mosaic's `packages/mosaic-core/src/preagg/*` ships this already; parity is plausible but requires the IR-pass shape. Curated corpus spec `flights-200k.yaml` (200k rows of `rectY` over `filterBy: $brush`) is the most obvious v1 test for pre-aggregation.

---

## D3 — Selection compilation: how do cross-filter predicates land in SQL?

**Context.** Mosaic selections are first-class predicate objects. A `params: { brush: { select: crossfilter } }` declaration combined with `filterBy: $brush` on two views produces a cross-filter: plot A's brush filters plot B's data *except* plot A's own view (`crossfilter` resolution). `intersect` takes the AND of all contributions; `union` takes OR; `single` replaces on each update. See `crates/brightfield-spec/src/ast.rs:331-340` — `MarkData::From { source, filter_by: Option<ParamRef>, ... }` already carries the selection identity as a lifted `ParamRef`, and `crates/brightfield-spec/vendor/mosaic-specs/yaml/crossfilter.yaml` is the canonical two-view test. Emission must turn the selection's current predicate set into SQL at query time.

**Options.**

- **A. Per-view subquery, predicates inlined as `WHERE (...)`.** At emit time the emitter resolves the selection (`brush`) to its current predicate list, filters out contributions from the view's own source, joins the survivors with AND, and splices the result straight into the view's SQL as a `WHERE` clause.
- **B. Shared CTE containing filtered rows.** Emit a `WITH filtered_flights AS (SELECT * FROM flights WHERE <resolved predicates>)` at the top of every query that depends on the selection; each dependent view reads `FROM filtered_flights`. Predicates live in one place per (source, selection) pair.
- **C. DuckDB-prepared view + bind on update.** Register a view per (source, selection) in DuckDB (`CREATE OR REPLACE VIEW v_flights_brush AS SELECT * FROM flights WHERE <predicates>`), then views `FROM v_flights_brush`. On selection change, re-issue the view definition; views run their query fresh each time they are scanned.

**Trade-offs.**

- **A.** Straightforward to emit and to reason about. Each view's SQL is self-contained — good for conformance-layer-2 string comparison (card 0002's layer-2 gate wants to diff SQL strings / IRs). Cost: duplicated predicate material across views. In a 10-view dashboard with a shared brush, 10 `WHERE (...)` clauses must each be re-rendered when the brush moves.
- **B.** Deduplicates predicates within a single query request. But Mosaic's model has *each view* issue its own query (brief §3.3: "When a param updates, all subscribing clients re-query and re-render"), and DuckDB in-process has no cross-query plan cache. So the CTE only helps when a single spec issues multiple dependent views in one query — which it doesn't today; the coordinator issues one query per subscriber. Adds engine coupling (CTE-support is ANSI but DuckDB's inliner treats CTEs differently than subqueries — see DuckDB docs on materialised CTEs).
- **C.** Lets the emitter hold the view definition constant and re-bind its predicates without touching the downstream SQL. But `CREATE OR REPLACE VIEW` is DDL — it takes a catalog lock and isn't free, and views in DuckDB aren't indexed so this buys nothing on the read side. Also muddies cache-key semantics (D5): is the cache keyed on the view name or the view's underlying predicate set? — two keys for the same shape.

**Recommendation: A, with the `crossfilter` resolution implemented as an IR-level operation.**

Each view emits its own `WHERE` clause; the emitter is responsible for resolving the selection to a predicate list and excluding the view's own contribution when the resolution is `crossfilter`. This decision composes cleanly with D1 (IR): a selection is an IR-level `Filter { source_of_contribution: SelectionName, predicates: Vec<Predicate>, resolution: SelectionResolution }` node, and `crossfilter` exclusion is a method on `Filter` that drops predicates whose `source_of_contribution == this_view.source`.

Predicate shape: each contribution to a selection is (by Mosaic's model) itself a SQL expression — the brush contributes `x BETWEEN lo AND hi` where `lo`, `hi` are the brush's current values. The AST already tokenises such expressions into `ExpressionNode { spans, params }` (`ast.rs:354-360`). At emission, `ExpressionNode::to_sql(param_values: &ParamValues)` renders the spans with current param values interpolated — safely, via DuckDB's `?` parameter binding where possible, and via rendered literals where not (see D4 for the bind-vs-interpolate split).

Evidence: Mosaic's `crossfilter` resolution is documented as "intersect all clauses except the one from the view's own source" (Mosaic docs, `Selection.crossfilter`). Per-view emission matches how Mosaic's JS coordinator dispatches queries — each subscriber gets its own query. CTE-based sharing (B) would be a premature optimisation that we'd then need to undo for the cache-key design in D5.

---

## D4 — Parameter binding: prepared statements or query rewriting?

**Context.** Param references appear in two syntactic contexts in the AST: (i) at outer value slots as a lifted `ParamRef` (via `ValueOrParamRef::Param`, `ast.rs:211-220`) — e.g. `filterBy: $brush`, `fill: $colour`; (ii) inside tokenised SQL strings as `ExpressionNode::params` — e.g. `{sql: "delay > $threshold"}`. On a slider drag, a param updates continuously; every subscriber to that param re-queries. The <100ms budget is dominated by this path — sub-frame re-query on every `pointermove`. Emission can either produce one SQL string per param change (rebuild from the AST each time) or one parameterised SQL once (prepared statement) with fresh values bound on each change.

**Options.**

- **A. Rebuild SQL string on every param change.** No prepared statements. Each param change invalidates the emitted SQL; the emitter runs again over the AST with the new param values substituted into `ExpressionNode` spans and `ValueOrParamRef::Param` slots. Simple, no lifecycle.
- **B. Prepared statement with `?` placeholders; bind values on update.** Emit `WHERE delay > ?` once; keep the prepared statement alive in duckdb-rs; `execute(&[new_threshold])` on each param change. Re-emit the SQL only when the *shape* of the query changes (a new mark added, a data source swapped) — not when a param value changes.
- **C. Hybrid: prepared statements for scalar params, SQL-rewrite for predicate-shaped selections.** A scalar param (`threshold: 5`) binds as `?`; a selection (`brush` contributing `x BETWEEN lo AND hi`) cannot bind as `?` because its *predicate list* changes shape (a brush gains a second range, or a `union` selection's member count grows). Scalar path is prepared; selection path rebuilds.

**Trade-offs.**

- **A.** Simplest lifecycle. Doesn't meet the budget — for a slider bound to a `WHERE delay > $threshold` over 10M rows, the query plan is the same every drag, but rebuilding the string and re-parsing it in DuckDB wastes maybe 2–5ms per frame on planner cost alone. At 60Hz drag rate that's 120–300ms/second of planner overhead. The planner cost is what prepared statements exist to eliminate.
- **B.** Best path for scalar-param reactivity (sliders, menus, text inputs). Prepared statements across duckdb-rs are well-documented and survive many executions. Fails for selections where the predicate-list shape changes (adding a brush range, changing resolution). Also fails for structural changes (a new mark, a new view) — those need re-emission, which option B already requires.
- **C.** Right granularity. Scalar param path is a prepared statement; selection path rebuilds the `WHERE` clause from scratch (still cheap — the predicate list is small, and DuckDB's planner isn't expensive for small filter trees over a view on Parquet). The emitter tracks which params are scalar vs selection (the AST already distinguishes: `SelectionNode` is distinct from `ParamNode`), and chooses binding strategy per param.

**Recommendation: C.**

The emitter's output for a query is `EmittedQuery { sql: String, bindings: Vec<Binding>, dependencies: QueryDeps }` where `Binding` is a pairing of a `?`-position with a `ParamRef`, and `QueryDeps` records which params/selections this query depends on. A slider drag dispatches to `execute(stmt, &[latest_values])` via duckdb-rs' prepared-statement API; a brush drag triggers a re-emit of the `WHERE` clause (and only that — the rest of the SQL and the prepared statement shell are cached). Structural changes invalidate the prepared statement and trigger a full re-emit.

The split aligns with Mosaic's own distinction — `Param` vs `Selection` are different classes in `@uwdata/mosaic-core`, with different update semantics. The AST already honours the split (see `vocab.rs` `SelectionResolution` enum).

Evidence: `ExpressionNode::to_wire()` round-trips param-interpolated SQL (`ast.rs:381-391`), so option A's mechanism works but pays the planner cost. Brief README §Query Engine: "dynamic parameter substitution via Mosaic's `$param` expression syntax" — doesn't mandate prepared statements, but the <100ms-at-multi-million-rows scenario does.

---

## D5 — Incremental re-query on brush movement: what's cached and what's re-emitted?

**Context.** A brush drag produces (at interactive rates) a stream of `brush` param updates. Each update must land a result in <100ms. If the emitter rebuilds SQL from scratch and DuckDB re-parses and re-plans every frame, the budget is spent before reaching the data. This decision determines what layer of artefact is cached and what invalidates it.

**Options.**

- **A. No caching; emit and execute fresh every update.** Measure first, optimise later. May be fast enough for simple queries over pre-aggregated bins.
- **B. Cache the SQL string keyed by `hash(structural_plan)`; re-execute with new bindings via prepared statements (per D4).** The "structural plan" excludes param values but includes everything else — mark kind, source, filter *shape* (which params are referenced, which resolution), projections, group-by keys, limit. Param value changes hit the bindings path; shape changes miss the cache and re-emit.
- **C. Cache the result set keyed by `hash(full_plan + param_values)`; return cached Arrow batches on identical requests.** Mosaic's coordinator does this (`@uwdata/mosaic-core` `QueryManager.cache` — SQL-keyed result cache). A drag through the same brush position twice (e.g. hovering on a plateau) returns instantly from cache.

**Trade-offs.**

- **A.** No artefact lifecycle to manage. Fails the budget for non-trivial queries; we'd discover that the first time someone brushes over `flights-200k.yaml`.
- **B.** Eliminates DuckDB-side planner overhead (the dominant cost for repeated execution of the same shape). Doesn't help when the same query+values fires twice. Straightforward to implement on top of the IR from D1 — `hash(QueryPlan)` before param substitution is the structural key.
- **C.** Covers both repeat-query cases (shape-cache) and steady-state cases (result-cache). Cost: Arrow batch memory in-process, plus the question of eviction. Mosaic's cache lives in JS heap with a simple LRU; DuckDB's Arrow batches in Rust memory are similar — we own the memory, LRU is straightforward.

**Recommendation: Both B and C, stacked.**

B is the prepared-statement lifecycle described in D4 — it's the same thing. C is an additional result-cache keyed on (structural plan + param values). On a slider drag, many frames hit the prepared-statement path; the shape is cached. On a brush snap-back (e.g. "reset brush to previous position"), the result-cache returns the Arrow batches without re-executing. The two caches are orthogonal layers; both are keyed by `hash(plan)`-ish material, both evict LRU.

Caveat: v1 should ship the prepared-statement path (B) and the shape-cache it implies. The result-cache (C) can slot in later — it's purely additive. Card 0003's <100ms budget is met by B alone for steady-state dragging; C exists for repeat-visit patterns where it's pure speedup.

Structural plan hash must be stable across param values — i.e., `QueryPlan::hash()` deliberately excludes the values inside `Filter::predicates` that are bound via `?`, but *includes* the `?` positions themselves and every other IR node. This is the invariant the emitter owes the coordinator.

Evidence: brief §3.3 / README §Features name "SQL-keyed result caching" as a coordinator-owned feature. That's option C, and it lives above the emitter — but the emitter must produce a *hashable* plan for the cache key. That constraint is the reason D1's recommendation (IR) is load-bearing: string SQL is not a stable cache key (whitespace, alias renaming, field ordering all break it).

---

## D6 — Vocabulary status handling: what does the emitter do with `Planned` and `Unimplemented` nodes?

**Context.** The vocabulary registry (`crates/brightfield-spec/src/vocab.rs`) annotates every mark/interactor/input/component with `ImplStatus::{Implemented | Planned | Unimplemented}`. Card 0002's `SupportReport` walks an AST and enumerates every `Unimplemented` and `Planned` node before rendering starts. The emitter sits between the AST and the renderer: when asked to emit SQL for a spec that contains a `Planned` or `Unimplemented` mark, it must do something. The `SqlEquivalenceCheck` stub in `crates/brightfield-conformance/src/layer.rs:160-178` is the downstream gate — it's currently `LayerOutcome::Pending { reason: "SQL emitter not yet available" }` and will become a real pass/fail the moment the emitter lands.

**Options.**

- **A. Hard error on any non-`Implemented` mark.** `emit_query(spec) -> Result<EmittedQuery, EmitError::Unimplemented { kind, status }>`. Conformance-layer-2 for specs containing such marks reports `Fail` with the error.
- **B. Skip non-`Implemented` nodes silently; emit SQL for the implemented subset.** A spec with a mix of marks produces a query that covers only the known ones; the others contribute nothing.
- **C. Emit a stub `SELECT 1 WHERE FALSE` (or `NULL`-shaped rows) for non-`Implemented` marks; continue with the rest.** Downstream consumers (renderer) see an empty result set for unknowns.
- **D. Respect the preflight contract (card 0002 D3): the emitter *refuses to run* on a spec that failed preflight. Otherwise, assume all nodes in the input are `Implemented` — the invariant is upheld by the caller.**

**Trade-offs.**

- **A.** Clear failure mode, but duplicates preflight. The preflight (card 0002 `SupportReport`) already catches non-`Implemented` nodes; re-catching them at emit time is belt-and-braces that confuses the error story ("was this a preflight miss or an emit-time surprise?").
- **B.** Silent partial emission is precisely the "silently omitting or approximating" failure mode card 0002's D3 forbids (scenario 3 of card 0002). Rejected by prior decision.
- **C.** Gives the renderer something to consume but produces a query whose result set misleadingly suggests "this mark has no data", when the truth is "this mark is not supported". Same failure shape as B in user terms; deeper failure in data terms.
- **D.** Honest separation of concerns. Preflight is the gate for spec acceptance; the emitter's contract is "given an `Implemented`-only spec, produce SQL". A debug-assertion or explicit `EmitError::InvariantViolation` fires if a non-`Implemented` node reaches the emitter — which should never happen if preflight ran.

**Recommendation: D, with an `EmitError::InvariantViolation` for defence-in-depth.**

The emitter API shape: `fn emit(spec: &Spec, preflight: &SupportReport) -> Result<EmittedQuery, EmitError>`. Takes the preflight report as an argument — not to re-validate, but to assert its guarantee. Inside, a debug-only `assert!(mark.status == ImplStatus::Implemented)` catches drift during development; release builds trust the preflight. This matches the architecture that card 0002 laid down (AST totality, preflight gate, renderer respects preflight) and extends it one layer.

This also clarifies the conformance-layer-2 contract: a curated spec's `layer_2: pass` expectation is predicated on `layer_0_preflight: all_implemented`. A spec that fails preflight cannot reach layer 2; its `layer_2` expectation is `pending` or `suppressed` via the deviation registry, not `fail`.

Evidence: card 0002 decisions D3 ("full-vocabulary AST + preflight `SupportReport`") and D6 ("AST totality — parser must accept any valid spec"). `crates/brightfield-conformance/src/support.rs` is where `SupportReport` lives — the emitter should take `&SupportReport` to make the contract explicit in the type signature. `LayerOutcome::Pending` in `layer.rs:174-178` is the slot whose `reason` literal flips from `"SQL emitter not yet available"` to either `Pass`, or `Fail { details: String }` — no plumbing change required, as `layer.rs` comments promise.

---

## D7 — Conformance capture: how is emitted SQL compared against Mosaic's?

**Context.** Layer-2 conformance (card 0002 D1, layer 2: SQL equivalence) says "emitted SQL produces matching result sets". That's a strong claim with two plausible interpretations: (a) the SQL *strings* match (after normalisation), or (b) the SQL *result sets* match when both are executed. Today the infrastructure exists for neither — `SqlEquivalenceCheck::run` returns `Pending`. This decision sets the gate shape and therefore what the emitter must expose to be testable.

**Options.**

- **A. String-snapshot comparison.** Golden SQL files per curated spec: `vendor/curated/yaml/line.golden.sql`. The emitter's output is compared to the golden after light normalisation (whitespace, alias ordering). Easy to read in a diff, trivial to update on intentional change.
- **B. Structural comparison of emitted SQL via a parsed AST.** Use a SQL parser (e.g., `sqlparser-rs`) to parse both the emitter's output and Mosaic's `mosaic-sql` output (captured as fixture), normalise both to an AST, compare. Tolerates whitespace/alias/syntax-sugar variation.
- **C. Result-set comparison against DuckDB.** Execute both the emitter's SQL and a fixture SQL (captured from Mosaic's `mosaic-sql`) against the same DuckDB instance with the same data source, compare the returned Arrow batches. Tolerates any SQL-shape variation as long as semantics match.

**Trade-offs.**

- **A.** Cheapest by far. Fragile — a whitespace tweak, an alias rename, or a legal SQL rearrangement (join order, `SELECT` column order) registers as a failure. Spec authors reading a diff won't know whether a change is semantic or cosmetic. Fixture maintenance burden grows with corpus size.
- **B.** Stronger than A — catches true structural regressions while tolerating cosmetic ones. Cost: a SQL parser dependency (sqlparser-rs is mature, ~400KB of Rust). Doesn't catch cases where the emitter's SQL is *structurally different but semantically equivalent* (e.g., `WHERE a AND b` vs `WHERE b AND a`, different join order); those need C. Deviation registry entries (card 0002 D4) still slot in as `DEV-NNNN`-tagged acceptable structural diffs.
- **C.** The strongest gate — semantic equivalence is what "layer 2: SQL equivalence" actually means (card 0002 decisions D1: "emits SQL whose result sets match"). Cost: running DuckDB on fixture data in every test (conformance-layer-2 per-PR as card 0002 D5 lays down). Fixture data has to be committed or synthesised.

**Recommendation: B as the primary gate, with C available as an escalation for specs where structural diffs are ambiguous.**

Structural comparison via sqlparser-rs gives the right signal/noise ratio: a typo in the emitter breaks the structural AST, a whitespace change doesn't. Golden SQL files live alongside the curated specs (`vendor/curated/yaml/line.golden.sql`) and are parsed on test load. Each structural divergence that is accepted as a deviation (e.g., DuckDB-specific `parquet_scan` vs Mosaic's `FROM "foo.parquet"`) earns a `DEV-NNNN` registry entry — card 0002's deviation registry already supports this (`LayerNExpectation::Suppressed("DEV-XXXX")`).

Result-set comparison (C) is the fallback for cases where two structurally-different SQL strings produce the same result (e.g., `EXISTS (SELECT 1 FROM ...)` vs `IN (...)` rewrites). Not shipped in v1; invoked ad-hoc when structural diff + registry entry isn't sufficient.

The `EmittedQuery` struct thus carries two exposed surfaces: the SQL text (for B and C) and the IR (for D5's cache key). Both are load-bearing for different consumers.

Evidence: card 0002 decision D1 option B ("SQL equivalence — the query engine emits SQL whose result sets match the ones Mosaic's `mosaic-sql` would produce"). `SqlEquivalenceCheck` in `crates/brightfield-conformance/src/layer.rs:162-178` is the concrete hook; its `run` method takes the spec, fixture, and registry — all three are present, all three are what option B needs.

---

## Summary

```
| #  | Decision                           | Recommendation                                                                      |
|----|------------------------------------|-------------------------------------------------------------------------------------|
| D1 | AST traversal strategy             | AST → typed query IR → SQL; per-mark visitor for AST→IR; shared IR→SQL renderer     |
| D2 | SQL dialect and query shape        | DuckDB-native with pluggable optimisation passes; v1 ships no passes registered     |
| D3 | Selection compilation              | Per-view `WHERE` emission; `crossfilter` resolution handled at IR level             |
| D4 | Parameter binding                  | Hybrid — prepared statements for scalar params; rebuild `WHERE` for selections      |
| D5 | Incremental re-query               | Shape-cache (prepared stmt) in v1; result-cache layered in later; both key on IR    |
| D6 | Vocabulary status handling         | Emitter trusts preflight; invariant asserts on non-`Implemented` nodes              |
| D7 | Conformance capture                | Structural SQL diff via sqlparser-rs; result-set diff as escalation path            |
```

Open questions flagged for the review gate:

- **OQ1 (from D2).** Which mark families does card 0003's <100ms budget actually require pre-aggregation for, given that v1 targets `flights-200k.yaml` (200k rows, not millions)? If the budget is met without pre-aggregation for v1's corpus, delay the pass registry's first entry until a mark lands that needs it.
- **OQ2 (from D3).** For `union` and `intersect` selection resolutions with empty predicate lists, what does the emitted `WHERE` degrade to? (Mosaic's JS treats empty as "no filter"; we should match.)
- **OQ3 (from D5).** Does the result-cache (C) live in the emitter crate or the coordinator? Per brief §Architecture it's a coordinator concern, which means the emitter only needs to expose the IR's hash — confirm that boundary.
- **OQ4 (from D7).** Mosaic's `mosaic-sql` outputs as fixture — captured how? Either (a) a one-off vendored dump of Mosaic's JS output against the curated corpus, or (b) a test harness that invokes Node + Mosaic per-test. (a) is simpler and closer to card 0002's D2 curated-corpus shape.
