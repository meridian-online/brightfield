# Decision Pack — Card 0005: Reactive Parameters with Input Widgets

Card goal: an analyst adjusts a slider, menu, search box, or table selection and watches the view update live — without re-authoring the query.

Scope: the reactive propagation model, widget-param declaration surface, and edge-case semantics. Implementation approach is left to the implementing agent.

Evidence citations use repo-relative paths. Prior decisions referenced:
- `orbit/specs/2026-04-20-mosaic-spec-driven-visualisation/decisions.md` (card 0001 — AST, vocabulary registry, `ValueOrParamRef<T>`, `ExpressionNode`, `ParamRef`).
- `orbit/specs/2026-04-21-fluid-interaction-at-dataset-scale/decisions.md` (card 0003 — QueryPlan IR, hybrid binding model, selection compilation, `EmittedQuery`).

Shipped-code touchpoints:
- `crates/brightfield-spec/src/ast.rs` — `Input { kind: InputKind, status: ImplStatus, options: IndexMap<String, ValueOrParamRef<SpecValue>> }`, `ParamNode::Value | Selection`, `ParamRef`, `ExpressionNode { spans, params }`.
- `crates/brightfield-spec/src/vocab.rs` — `InputKind::{Menu, Search, Slider, Table}`, all `Unimplemented`.
- `crates/brightfield-sql/src/binding.rs` — `Binding::{Scalar, Selection}`, `BindingMode::{Prepared, Interpolated}`, `EmittedQuery { sql, bindings, plan_hash }`.
- `crates/brightfield-sql/src/emit.rs` — `emit_query`, `emit_all_queries`, `collect_marks` (currently ignores `Component::Input` nodes).
- Corpus evidence: `vendor/mosaic-specs/yaml/athletes.yaml` — three inputs (two menus, one search) bound via `as: $category` / `as: $query` to selections, with `filterBy:` on marks and other inputs. Demonstrates chained propagation (search `filterBy: $category` cascades through `$query`).

---

## D1 — Propagation guarantee: what does "reactive" mean for param updates?

**Context.** The card's scenarios require that when a param value changes, *all* subscribing queries re-fire and *no* stale view is left behind (scenario 3). This is a correctness guarantee — the system must define what "subscribing" means, what order updates happen in, and whether partial/inconsistent states are visible.

**Options.**

- **A. Eventual consistency — fire-and-forget.** When a param changes, the system enqueues re-query for every subscriber. Subscribers may briefly show stale data while their queries are in flight. No ordering guarantees between subscribers. Simple to implement; matches browser event-loop semantics.
- **B. Atomic snapshot — all subscribers see the same param epoch.** Each param change increments an epoch counter. A subscriber's query always executes against the param values at a single epoch — no mixed reads. If param A changes while subscriber B's query from the prior epoch is still running, B's result is discarded and re-queued at the new epoch. This prevents "tearing" where two plots show data from different slider positions.
- **C. Synchronous barrier — all subscriber queries complete before the next param change is accepted.** The slider blocks until every dependent query returns. Guarantees visual consistency but introduces input lag proportional to the slowest subscriber.

**Trade-offs.**

- **A.** Cheapest path. Risk: the card's scenario 3 says "no stale view is left behind" — under A, stale views are transiently visible. Whether that violates the scenario depends on whether "left behind" means "permanently stale" or "ever visible". For a slider being dragged at 60Hz, brief transient staleness is normal and expected in Mosaic's own implementation.
- **B.** Prevents tearing without blocking input. Cost: requires an epoch/generation counter and a "discard stale results" policy. Mosaic's coordinator uses a similar model — each query carries a request ID, and stale responses are dropped. This is the right granularity for the "no stale view" guarantee: a view is never updated with data from a superseded param state.
- **C.** Strongest guarantee but unusable at interactive rates. A slider connected to two plots that each take 50ms means 100ms per frame — below 10Hz input rate. Violates the card 0003 <100ms budget for the system, not per-subscriber.

**Recommendation: B.**

The reactive model should guarantee epoch-consistent updates: every subscriber sees param values from one epoch, never a mix. Stale-epoch results are silently discarded. This matches Mosaic's own coordinator semantics (brief: "When a param updates, all subscribing clients re-query and re-render") and is compatible with the prepared-statement binding path from card 0003 D4 — the prepared statement stays valid across epochs, only the bound values change.

The epoch counter is a natural extension of `EmittedQuery.plan_hash` — the hash identifies the query *shape*, and the epoch identifies which param *values* it was executed against. Together they form the cache key for card 0003 D5's result-cache.

---

## D2 — Widget-param binding declaration: how does a widget declare which param it writes to?

**Context.** Mosaic's input widgets declare their output binding via an `as:` key — e.g. `input: slider, as: $threshold` means "this slider writes to param `threshold`". The AST already parses `as:` into the Input's `options` map as a `ValueOrParamRef::Param(ParamRef("threshold"))` (see `ast.rs:319-326`, where options are lifted by the parser). But the *semantic contract* — that `as:` means "this widget is the writer for this param" — is not yet codified. The system needs to know which widget writes which param so it can route updates correctly.

**Options.**

- **A. Convention-only — `as:` in the options bag, interpreted by the coordinator at runtime.** No new AST structure. The coordinator inspects each Input's options at startup, finds `as:` entries, and builds the widget-to-param routing table. The AST remains a generic options bag for inputs.
- **B. Promote `as:` to a typed field on `Input`.** Add `pub as_param: Option<ParamRef>` to the `Input` struct (alongside `kind`, `status`, `options`). The parser lifts `as:` out of the generic bag into this typed field. The system can statically enumerate all widget-param bindings without inspecting options bags.
- **C. Introduce a `WritesParam` trait or marker on components.** Any component that can write a param (Input, Interactor, Mark with `as:`) implements `WritesParam { fn target_params(&self) -> Vec<ParamRef> }`. Generic over component type.

**Trade-offs.**

- **A.** Zero AST changes. Cost: every consumer that needs to know widget-param bindings must parse the options bag, handling `ValueOrParamRef::Param` vs `ValueOrParamRef::Value` at each site. Duplicated interpretation logic. The `as:` key could be misspelled or missing with no compile-time signal.
- **B.** Clean static contract. The parser already lifts `$param` references — this is one more lift. Cost: breaks the "Input options are a flat bag" pattern established in card 0001 (ac-02: "Mark / Interactor / Input are structs keyed by their respective `*Kind` enums" with options as IndexMap). But precedent exists: `Mark` already has `data: Option<MarkData>` as a typed field alongside its options bag — `Input` gaining `as_param` follows the same pattern.
- **C.** Most general, but premature. Today only Input widgets write params via `as:`; interactors write selections via `as:` too, but that's already handled by the selection compilation path (card 0003). A trait adds indirection without a second consumer to justify it.

**Recommendation: B.**

Promote `as:` to `pub as_param: Option<ParamRef>` on `Input`. This makes widget-param binding statically visible at the AST level — the coordinator can enumerate all writers without options-bag inspection. The parser already does the `$param` lift (see `parse.rs:910` — `self.lift_field(&key, val)`); extracting `as:` before inserting into the options bag is a small parser change.

Evidence: `Mark` already has the precedent of a typed field (`data: Option<MarkData>`) alongside its generic options bag. The corpus spec `athletes.yaml` shows `as: $category` and `as: $query` on every input widget — `as:` is universal for inputs, not an edge case. Also promote `from:` and `filterBy:` to typed fields for the same reason — inputs in the corpus universally declare these (menu needs `from:` + `column:` to populate its options; search needs `filterBy:` to scope results).

---

## D3 — Dead params: what happens when a param has no subscribers?

**Context.** Scenario 5 says: "a param exists but no plot or widget references it — the update is silently absorbed." This defines the edge behaviour. The question is whether the system should detect dead params at spec-load time (static analysis) or at update time (runtime).

**Options.**

- **A. Runtime no-op — update fires, finds no subscribers, returns immediately.** No static analysis. Dead params are invisible; the system never warns about them.
- **B. Static warning at spec-load time, runtime no-op at update time.** When the spec is loaded, the system builds a subscriber graph and emits a non-fatal warning for any param declared in `params:` that has zero subscribers (no mark's `filterBy:`, no expression's `$param` reference, no input's `filterBy:`). At runtime, updates to dead params are still silently absorbed.
- **C. Static error — dead params are rejected.** A param with no subscribers is treated as a spec authoring error and fails validation.

**Trade-offs.**

- **A.** Matches the scenario exactly: "silently absorbed." But leaves authors with no feedback when they mistype a param name (e.g. `$threshhold` instead of `$threshold`) — a common source of "why doesn't my slider work?" frustration.
- **B.** Best of both worlds: runtime silence (no crashes, no errors — the scenario is met) plus author feedback at load time. The warning is non-fatal, so it doesn't block rendering. The subscriber graph is needed anyway for propagation (D1) — the dead-param check is a free by-product.
- **C.** Too strict. Mosaic itself allows dead params (a spec might declare params for future use, or a widget might be commented out during development). Rejecting dead params would break valid Mosaic specs.

**Recommendation: B.**

Build the subscriber graph at spec-load time (which the reactive propagation model from D1 requires anyway). Emit a `ParseWarning::DeadParam { name }` for any param with zero subscribers. At runtime, updates to dead params are no-ops — no queries fire, no errors surface.

Evidence: the card's scenario 5 explicitly requires silent absorption. The subscriber graph is a natural by-product of the param-dependency analysis needed for D1's epoch-consistent propagation. `ParseWarning` already exists as the non-fatal warning type (`crates/brightfield-spec/src/parse.rs` — `ParseWarning::UnknownOption` is the existing variant; `DeadParam` follows the same pattern).

---

## D4 — Chained param propagation: how do intermediate queries feed downstream params?

**Context.** Scenario 4 describes chained params: "param A drives a query whose result sets param B, and a plot subscribes to param B." The corpus example in `athletes.yaml` shows this pattern: menu widgets write to `$category`, and the search widget declares `filterBy: $category` and `as: $query` — so `$category` changes cascade through the search widget's backing query to update `$query`, which downstream plots consume. The system must define how this cascade works and what ordering guarantees it provides.

**Options.**

- **A. Topological propagation — build a DAG of param dependencies and propagate in topological order.** When param A changes, the system identifies all transitive dependents, sorts them topologically, and processes each level in order. Level 0: param A's direct subscribers. Level 1: params written by level-0 widgets that depend on param A. And so on. Cycles are detected at spec-load time and rejected.
- **B. Reactive pull — each subscriber lazily queries its dependencies when asked.** When a plot needs to render, it pulls its param values, which triggers the intermediate query, which pulls *its* param values, recursively. No explicit ordering; the call stack is the ordering.
- **C. Flat propagation — only direct subscribers re-query; chained params require explicit coordinator wiring.** The system propagates one level deep. If the search widget's backing query needs to re-run when `$category` changes, the coordinator must know that the search widget is both a subscriber of `$category` and a writer of `$query`. The coordinator handles the multi-level cascade.

**Trade-offs.**

- **A.** Correct by construction — topological order guarantees that by the time a downstream subscriber fires, all upstream params are settled. Cost: requires building and maintaining a DAG. Cycle detection prevents infinite loops (which are impossible in a well-formed spec but possible if an author writes `as: $a, filterBy: $a` on the same widget — a self-loop). The DAG is a static artefact of the spec, computed once at load time.
- **B.** Simple implementation but hard to reason about. Re-entrancy is possible (query A triggers query B which triggers query A again if there's a cycle). No natural place to detect cycles. Performance is unpredictable — a deep chain with large intermediate queries stalls the leaf subscriber.
- **C.** Shifts complexity to the coordinator. The coordinator already needs to know widget-param bindings (D2), so adding the transitive cascade there is natural. But "flat propagation" means the coordinator must explicitly handle every level of the chain — it's really just option A implemented incrementally rather than as a single DAG walk.

**Recommendation: A.**

Build a param dependency DAG at spec-load time. Nodes are params; edges are "param X is consumed by a component that writes param Y." Detect cycles and report them as spec errors (a cycle means the spec cannot stabilise — it's a real authoring bug, not a valid pattern). Propagate in topological order within each epoch (D1).

Evidence: `athletes.yaml` demonstrates a two-level chain: `$category` -> search widget -> `$query` -> plots. Mosaic's coordinator processes these in dependency order (Mosaic's `Coordinator.requestQuery` queues queries and processes them in registration order, which for well-formed specs happens to be topological). The DAG is cheap to build — the spec's component tree has at most tens of nodes, and the param map has at most tens of entries.

---

## D5 — Type safety at the widget-param boundary: what happens when a widget emits a value incompatible with its param's declared type?

**Context.** A slider emits a numeric value; a menu emits a string (or array of strings for multi-select); a search emits a string pattern; a table emits a selection (row set). The param's declaration (`params: { threshold: 5 }`) implies a type (numeric). If a menu is accidentally bound to a numeric param, the emitted string value will produce a type mismatch when interpolated into a SQL expression like `delay > $threshold`. The system must define what happens at this boundary.

**Options.**

- **A. No type checking — the SQL engine catches it.** Widget values are passed through to SQL as-is. If the type doesn't match, DuckDB returns a query error at execution time. The error surfaces to the user as a failed query.
- **B. Static type inference at spec-load time.** Infer the param's expected type from its declaration (`params: { threshold: 5 }` implies numeric; `params: { category: { select: intersect } }` implies selection) and the widget's output type (slider -> numeric, menu -> string/array, search -> string, table -> selection). Warn on mismatches at load time.
- **C. Runtime coercion — the system attempts to coerce widget output to the param's declared type.** Slider string "42" -> integer 42. Menu selection ["foo"] for a scalar param -> "foo". Coercion failures produce a warning and leave the param unchanged.

**Trade-offs.**

- **A.** Simplest. The error message from DuckDB will be cryptic ("cannot compare VARCHAR to INTEGER") with no link to the widget-param binding that caused it. Debugging requires the author to trace from the SQL error back through the param to the widget — painful.
- **B.** Best author experience — catches the bug before any query runs. Cost: type inference from `ParamNode::Value(SpecValue::Integer(5))` is straightforward for literals, but `ParamNode::Selection(...)` has no single "type" — selections are predicate-shaped, not value-shaped. The system would need a type vocabulary (scalar-numeric, scalar-string, selection, array) and inference rules per widget kind. This is real work, but the type vocabulary is small and fixed.
- **C.** Fragile. Coercion rules are a source of subtle bugs ("why does my menu with value '42' work with numeric params but '42.5' doesn't?"). Implicit coercion is the opposite of the project's "clarity over ceremony" value.

**Recommendation: B.**

Infer param types from their declarations and widget output types from `InputKind`. Emit a non-fatal `ParseWarning::ParamTypeMismatch { param, expected, actual }` at spec-load time when the types are provably incompatible (slider writing to a selection param, or table writing to a scalar param). Leave ambiguous cases (menu writing to a numeric param — could be valid if menu options are numeric strings) as unchecked, with the SQL engine as the backstop.

The type vocabulary is small: `Scalar(Numeric | String | Bool)`, `Selection`, `Array`. Widget output types are fixed per `InputKind`: Slider -> `Scalar(Numeric)`, Menu -> `Scalar(String) | Array(String)`, Search -> `Scalar(String)`, Table -> `Selection`. Param declared types are inferred from `ParamNode::Value(v)` (type of `v`) or `ParamNode::Selection(_)` (Selection).

Evidence: the card's scenarios don't explicitly address type mismatches, but the "explore how a parameter affects the data" framing (the `so_that` clause) implies the author expects things to work — a silent SQL error that produces an empty plot is the worst outcome. The project's engineering principle #3 ("programmatic checks for validation") supports catching this statically rather than relying on the SQL engine.

---

## Summary

```
| #  | Decision                              | Recommendation                                                                      |
|----|---------------------------------------|-------------------------------------------------------------------------------------|
| D1 | Propagation guarantee                 | Epoch-consistent updates — stale-epoch results discarded, no tearing across views   |
| D2 | Widget-param binding declaration      | Promote `as:` (and `from:`, `filterBy:`) to typed fields on `Input`                 |
| D3 | Dead params                           | Static warning at load time, silent no-op at runtime                                |
| D4 | Chained param propagation             | Topological DAG built at load time; cycles rejected as spec errors                  |
| D5 | Type safety at widget-param boundary  | Static type inference with non-fatal warnings; SQL engine as backstop               |
```

Open questions flagged for review:

- **OQ1 (from D1).** Should the epoch counter live in the emitter crate or the coordinator? The emitter produces `EmittedQuery` with `plan_hash`; the epoch is a runtime concern that may belong above the emitter. Confirm the boundary.
- **OQ2 (from D2).** Should `Interactor` also gain typed `as:` / `filterBy:` fields for symmetry, or is the existing options-bag treatment sufficient given that interactor-selection binding is already handled by card 0003's selection compilation?
- **OQ3 (from D4).** What is the maximum chain depth the system should support before warning? Mosaic's corpus specs show at most 2-level chains. A hard limit (e.g. 5) would prevent accidental combinatorial explosion in deeply-chained specs.
- **OQ4 (from D5).** Menu widgets with `column:` declarations could have their option types inferred from the data source's schema (if available at load time). Should this be in scope for v1, or deferred?
