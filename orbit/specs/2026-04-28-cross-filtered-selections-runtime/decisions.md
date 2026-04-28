# Decision Pack: Cross-Filtered Selections — Runtime Coordinator (v2)

**Card:** orbit/cards/0006-cross-filtered-selections-across-linked-views.yaml
**Date:** 2026-04-28
**Slice:** v2 — runtime coordinator (v1 static analysis shipped at commit 4dd422e)
**Prior art:** card 0005 v2 runtime coordinator at orbit/specs/2026-04-24-reactive-parameters-with-input-widgets/

---

## Context summary

V1 of this card (commit 4dd422e) shipped pure static analysis: filterBy validation, the `SelectionSubscriberGraph` keyed by selection name to subscribing component paths, the `InteractorBinding` list for interactor `as: $selection` declarations, and the corpus regression gate. None of it fires at runtime. `compile_selection` exists in `crates/brightfield-sql/src/lower.rs:94` and already implements the crossfilter self-exclusion rule by string source identity, but it is **only exercised by unit tests** — `emit_query` in `crates/brightfield-sql/src/emit.rs:269` ignores its `_param_values` argument and never threads selections into the plan (see also v2 PR review for card 0005, finding "emit_query ignores param_values argument").

Card 0005 v2 (commit 8ca4283) shipped the runtime coordinator pattern this slice will mirror. The shape, in `crates/brightfield-engine/src/lib.rs:244-291`:

```
fn propagate_param(&mut self, name: &str, value: SpecValue)
    -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>
{
    self.param_state.insert(name.to_string(), value);
    let subscribers = analysis.subscriber_graph.get(name).unwrap_or_default();
    let mark_indices = subscribers filtered through mark_index_map;
    if mark_indices.is_empty() { return vec![]; }
    for idx in mark_indices {
        emit_query(spec, idx, Some(&self.param_state)) -> execute_emitted
        results.push((idx, result));  // partial failure: continue on Err
    }
    results
}
```

The selections runtime coordinator must do the analogous thing for selection-update events, but with two extra concerns the param coordinator does not have:

1. **Multi-contributor merge**: a selection collects predicates from many interactors and resolves under a strategy (intersect / union / single / crossfilter).
2. **Per-subscriber predicate synthesis**: when a view contributes to its own selection, its own predicate must be excluded from its own filter (`compile_selection`'s `self_source` parameter — currently a string equal to the source view path).

Six decisions follow.

---

## Decision 1: Coordinator entry point — separate `propagate_selection` or unify under `propagate_event`?

### Context

The param coordinator exposes `Session::propagate_param(name, value)`. A selection update is structurally different: it is not a name+scalar pair but a `(selection_name, contributor_id, predicate, generation)` tuple. The interactor (or input widget) emits the contribution; the coordinator stores it, resolves the merged predicate per subscriber, re-emits, and re-executes. The choice is whether selections get their own entry point, share with params, or sit behind a unified event API.

Evidence:
- `Session::propagate_param` lives at `crates/brightfield-engine/src/lib.rs:244` and takes `(name: &str, value: SpecValue)`.
- `SpecValue` enum at `crates/brightfield-spec/src/ast.rs:408-427` has no Interval or Predicate variant — shoehorning a brush update into `SpecValue` would require either a new variant (cross-cutting AST change) or stringly-typed `SpecValue::Object`.
- `analysis.subscriber_graph` (params) and `analysis.selection_subscribers` (selections) are **separate maps** at `crates/brightfield-spec/src/analysis.rs:779-792`, built by different walkers. They overlap on filterBy edges but the v1 card deliberately kept them distinct.
- The card 0005 v1→v2 progression treats params as scalars; the param coordinator's `partial-failure` and `unsubscribed` behaviour assumes a single new value replacing the old.

### Options

**A. Separate `Session::propagate_selection(selection, contributor, predicate)` method**
- Gains: Clean signature reflecting the actual data flow (predicate, not scalar). Selection-specific concerns (merge, self-exclusion) live in one place. Mirrors the v1 separation of `subscriber_graph` vs `selection_subscribers`. No need to extend `SpecValue` with a Predicate variant.
- Loses: Two coordinator entry points (params and selections) — UI has to know which to call. Some specs use `as: $param` where the param is a value, not a selection — UI needs the param-vs-selection distinction at dispatch.

**B. Unified `Session::propagate_event(name, payload: PropagationPayload)` method**
- Gains: Single dispatch path for all reactive updates. Future event kinds (pointer-position, viewport-extent, hover) plug in without API growth.
- Loses: `PropagationPayload` becomes a sum type that today has only two variants (Scalar, Predicate). Adds an indirection layer with no current use case justifying it. The card 0005 v2 review explicitly approved the param-specific surface; bending it now widens scope.

**C. Overload `propagate_param` to accept `SpecValue::Object`-encoded predicate payloads**
- Gains: One method, one API. Wire compatible with the existing test fixtures.
- Loses: Stringly-typed payload — every consumer parses an Object back into a Predicate. Hides the structural distinction. The 0005 v2 review (LOW finding "emit_query ignores param_values") already shows the cost of underspecified payload semantics. Re-introduces that smell.

### Recommendation

**Option A.** Selections and params are different abstractions in the codebase already (`subscriber_graph` vs `selection_subscribers`, scalar values vs predicates, single-writer vs multi-contributor). A separate `propagate_selection` keeps each surface tight, mirrors the existing analysis-side separation, and avoids stretching `SpecValue` into a payload-of-anything role. Option B can be revisited later if more event kinds emerge — composability is preserved by sharing internal helpers (`execute_emitted`, partial-failure pattern).

---

## Decision 2: Wire format — what does an interactor/brush emit, and how is it stored?

### Context

A brush release in `crates/brightfield-ui/src/interaction.rs` produces a `Rect` in chart coordinates. To filter a SQL query that rect must become a predicate (e.g. `x BETWEEN 100 AND 200 AND y BETWEEN 5 AND 50`). Today there is no internal representation for "the set of predicates currently active on selection $brush" — `compile_selection` already accepts `&[(String, Predicate)]` (lower.rs:97) but no one stores or hands it that slice.

Evidence:
- `compile_selection` at `crates/brightfield-sql/src/lower.rs:94-127` takes `predicates: &[(String, Predicate)]` where the string is the contributor source name (currently used for crossfilter self-exclusion at lower.rs:106).
- `Predicate` enum at `crates/brightfield-sql/src/ir.rs:36-53` is the existing IR type — it has `Expr(String)`, `Param`, `And`, `Or`, `True`, `False`. Adequate for `intervalX` (single AND of two range bounds) and `intervalY` (single AND of two range bounds) and `intervalXY` (AND of four).
- `analysis.interactor_bindings: Vec<InteractorBinding>` (analysis.rs:570-575) maps interactor path → selection name. The interactor's `path: ComponentPath` is already a stable identifier.
- `SpecValue` does not have a Predicate variant. Trying to store predicates in `param_state: ParamValues` (an `IndexMap<String, SpecValue>`) would require encoding.
- Selection params are explicitly excluded from initial `param_state` population (engine/lib.rs:87-91) — confirming `param_state` is for scalar values only.

### Options

**A. New `selection_state: HashMap<String, Vec<(ContributorId, Predicate)>>` field on Session**
- Gains: Stores exactly what `compile_selection` already consumes. `ContributorId` is just a `ComponentPath` (the interactor's path). No `SpecValue` extension. Mirrors the `param_state` field shape (one map of live state on Session).
- Loses: Two state fields on Session (param_state for scalars, selection_state for predicates). Two readers — the lowering boundary needs both.

**B. Encode predicates inside `SpecValue::Expression` and reuse `param_state`**
- Gains: Single state field. No new types.
- Loses: `ExpressionNode` is a tokenised SQL string, not a structural Predicate — losing the And/Or tree means losing union/intersect resolution. Multi-contributor merge requires reparsing strings. Self-exclusion needs the contributor identity, which `ExpressionNode` does not carry. Effectively rebuilds Predicate inside SpecValue.

**C. Add `SpecValue::Selection { contributors: Vec<(String, ExpressionNode)> }` variant**
- Gains: Single state field. Roundtrips through the existing serialise path.
- Loses: AST-level type for a runtime concept. Forces every SpecValue consumer (parse, analysis, conformance, round-trip tests) to handle a variant that never appears in source YAML. Risk of breaking the corpus parse gate (cfs ac-10).

### Recommendation

**Option A.** Add a typed `selection_state: HashMap<String, Vec<(ComponentPath, Predicate)>>` field on Session, alongside the existing `param_state`. The Predicate type is already in the IR and is precisely what `compile_selection` accepts. ContributorId = ComponentPath gives crossfilter self-exclusion a stable identity (the brushing interactor's path matches the subscribing mark's parent plot path by prefix match — see Decision 4 for the exclusion rule). No AST changes, no SpecValue churn, no risk to the corpus parse gate. The brush layer (UI) is responsible for converting a `Rect` to a `Predicate` using the spec's bound channels — that translation is the brush-to-predicate adapter and lives at the dispatch boundary.

---

## Decision 3: Resolution strategy — where does intersect/union/single resolve?

### Context

`compile_selection` already implements all four resolution strategies (Crossfilter, Intersect, Union, Single) at `crates/brightfield-sql/src/lower.rs:99-127`. The strategy is a structural property of the `SelectionNode` (D5 from v1). The runtime question is *when* the resolved predicate is computed and *where* it is stored.

Evidence:
- `compile_selection(selection, self_source, predicates)` is pure — takes a slice, returns a Predicate. No memoisation today.
- Subscribers are typically a small set (the corpus crossfilter spec has 2-3 plots per selection — `crates/brightfield-spec/vendor/mosaic-specs/yaml/crossfilter.yaml`).
- `emit_query` at `crates/brightfield-sql/src/emit.rs:269` is invoked once per (mark, propagation) pair. Resolved predicates are SQL-text different per subscriber when self-exclusion fires.
- `EmittedQuery.plan_hash` at `crates/brightfield-sql/src/binding.rs:44` is the cache key — different predicates produce different hashes by design (lower.rs computes structural hash on the post-resolution plan).

### Options

**A. Resolve at dispatch time, per subscriber, no caching**
- Gains: Always correct — the predicate is rebuilt from current `selection_state` on every re-emission. Self-exclusion is per-subscriber-trivial since `compile_selection` already takes `self_source`. No memo invalidation logic.
- Loses: Recompute cost. For a selection with N contributors and K subscribers, that is N*K predicate clones per propagation. At Mosaic spec scale (N, K ≤ 5) this is negligible.

**B. Cache resolved predicate per (selection, self_source) and invalidate on selection_state mutation**
- Gains: Avoids recomputation when only one subscriber re-emits in isolation.
- Loses: Adds a cache that is invalidated on every brush move — exactly the time you would want it. Self-exclusion creates K different cached values (one per subscriber). Implementation cost > benefit at Mosaic scale.

**C. Pre-resolve once per propagation and dispatch only the indices needing self-exclusion separately**
- Gains: Cleaner separation between "compute the merged set" and "compute the per-subscriber hole".
- Loses: Same total work as A. Two code paths.

### Recommendation

**Option A.** Resolution is fast and pure; dispatch-time computation is correct by construction. This mirrors the param coordinator's "always re-emit" pattern in `propagate_param` — no special-case caching for the predicate, only DuckDB's prepared-statement cache (which still works since `plan_hash` is structural). The dispatch loop becomes:

```
for subscriber_path in selection_subscribers[name]:
    self_source = subscriber_path.parent_plot()  // see Decision 4
    predicate = compile_selection(selection, self_source, &selection_state[name])
    plan = lower(mark, ctx).filter_by(predicate)
    emit + execute
```

If profiling reveals contention, B can be added later as an optimisation, behind a feature flag.

---

## Decision 4: Self-exclusion identity — what counts as "the view's own predicate"?

### Context

V1 D3 fixed the rule as "per-view, not per-interactor." `compile_selection` accepts `self_source: &str` and currently takes string equality (lower.rs:106 — `source != self_source`). The runtime needs a concrete contract: when the coordinator dispatches a re-emission for mark M, what string does it pass as `self_source`, and how is each predicate's `source_name` set when it enters `selection_state`?

Evidence:
- ComponentPath is the canonical identifier across analysis: `crates/brightfield-spec/src/analysis.rs:570-575` (`InteractorBinding.path: ComponentPath`), 637 (`SelectionSubscriberGraph: HashMap<String, Vec<ComponentPath>>`).
- ComponentPath is built as a slash-delimited string like `root/vconcat[0]/plot[1]/mark[dot]` (see `crates/brightfield-engine/src/lib.rs:435-465` and analysis.rs walkers).
- An interactor path is `root/vconcat[0]/plot[1]/interactor[intervalX]`. A mark in the same plot is `root/vconcat[0]/plot[1]/mark[dot]`. They share the prefix `root/vconcat[0]/plot[1]` (the plot path).
- D3 says a *view* (plot) is the unit of self-exclusion, not interactor or mark.

### Options

**A. `self_source` = the parent plot path; contributor `source_name` = parent plot path of the contributing interactor**
- Gains: Honours D3 directly — string equality matches when interactor and mark live under the same plot. Survives multi-mark / multi-interactor cases (every component under one plot gets excluded together). Stable identifier (plot indices are derived deterministically by analysis walkers).
- Loses: Requires a `parent_plot(component_path)` helper. Trivial — slice up to the last `/plot[`.

**B. `self_source` = the full mark path; contributor `source_name` = the full interactor path**
- Gains: No prefix logic.
- Loses: A plot with one interactor and two marks fails D3 — the interactor's own predicate would still appear in the second mark's filter. Per-interactor exclusion was explicitly rejected in v1.

**C. `self_source` = a user-declared `name:` field on plots; contributor source_name = same**
- Gains: Author-controlled.
- Loses: No corpus spec uses `name:` on plots. Adds AST surface.

### Recommendation

**Option A.** Compute `self_source` as the parent plot path of the dispatching mark (or input widget). Contributor `source_name` in `selection_state` entries is set to the parent plot path of the contributing interactor. String equality in `compile_selection` (already implemented) does the rest. Add a `parent_plot(path: &str) -> &str` helper near the path-walking utilities — strip back to the longest prefix ending in `/plot[N]`. Dot-marks not inside a plot (rare; mostly malformed specs) get the full mark path as a degenerate self_source — they will never collide with an interactor path, so they receive all predicates (correct fallback).

---

## Decision 5: Concurrency and event ordering — what happens during a fast brush drag?

### Context

The interaction layer at `crates/brightfield-ui/src/interaction.rs:6` documents a two-tier model: "Immediate: overlay renders during drag (brush rect, highlight) — pure GPU, no I/O. Deferred: DuckDB re-query fires on brush release via session.update_param()." Today, `on_mouse_up` at `crates/brightfield-ui/src/chart_view.rs:102-109` only resets `InteractionState::Idle` — it does not call any session method. So nothing fires today. The runtime coordinator needs an explicit policy for what happens if multiple selection updates land in flight (two brushes in different plots, drag-throttled mid-drag updates if the model changes).

Evidence:
- `propagate_param` at engine/lib.rs:244 is `&mut self` — single-threaded by Rust's borrow rules. Re-entry is impossible by construction.
- The 0005 v2 spec deferred concurrency (no AC mentions interleaving, no test for it). It returns synchronously.
- DuckDB execution per mark is sub-second for the corpus specs (flights-200k = 200k rows; subscribing dot mark queries are simple SELECTs).
- The card 0010 (interactive feedback) ships a debounce model for navigation extents — `NavigationState.check_settle` at `crates/brightfield-ui/src/interaction.rs:292-303`. Selections do not yet have one.

### Options

**A. Synchronous, single-threaded, last-event-wins. Drop in-flight earlier dispatches by structure (re-entry impossible).**
- Gains: Mirrors the param coordinator exactly — no new threading model. Re-entry is impossible because `&mut self` blocks it. The dispatcher returns when all subscribers have re-executed; the next event runs to completion next.
- Loses: Long-running queries block the event loop. If a single subscriber takes 500ms, brushing freezes for 500ms.

**B. Synchronous coordinator + UI-side debounce on brush release (mirroring `NavigationState.check_settle`)**
- Gains: Cheap mitigation — the UI never fires `propagate_selection` until the user lifts the mouse or pauses. Selection updates land at human-perceivable rates only. Aligns with the existing two-tier model in interaction.rs.
- Loses: Adds a debounce timer in the UI layer. Slightly more event-handling code.

**C. Async dispatch with cancellation tokens**
- Gains: Most flexible.
- Loses: Off-sprint scope (sprint goal is "first end-to-end render"). Adds an executor and cancellation infrastructure with no current user-facing requirement. Card 0005 v2 ships sync; matching that pattern is the focus-gate-correct choice.

### Recommendation

**Option B.** Coordinator stays synchronous (Option A's contract); the UI throttles via the same debounce idiom already implemented for navigation. On `on_mouse_up`, the UI calls `session.propagate_selection(...)`. During drag, only the overlay renders — no engine call. This matches the explicit two-tier comment at interaction.rs:6 and avoids re-entry by design. Concurrency primitives are deferred until profiling shows a queue building up.

---

## Decision 6: Failure isolation — what happens when one subscriber's re-query fails?

### Context

The param coordinator's `propagate_param` returns `Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>` — one entry per subscriber, with `continue` on error so one failure does not block the others (engine/lib.rs:217-228). The selection coordinator faces the same shape but with one extra failure mode: `compile_selection` itself could fail (e.g. self_source not found, malformed contributor list). The contract for that error path needs nailing.

Evidence:
- `compile_selection` returns `Predicate` directly — it is infallible by signature today. Empty input → `Predicate::True`. Malformed self_source string is harmless (no match → all predicates retained).
- `propagate_param`'s partial-failure pattern at engine/lib.rs:217-223 uses `match … Err(e) => { results.push((idx, Err(...))); continue; }`.
- `EngineError` enum is already established (engine/error.rs is the home; types include `EmitFailed`, `QueryFailed`, `DdlFailed`).

### Options

**A. Per-subscriber result vec, `continue` on emit/execute error, identical to `propagate_param`. No new error variants.**
- Gains: Symmetric with the param coordinator. The 0005 v2 PR review explicitly approved the `continue`-on-emit-error pattern as structurally correct (review-pr-2026-04-24-v2.md §"continue pattern in propagate_param"). `compile_selection` is infallible so no extra variant needed.
- Loses: Nothing — no failure mode is unhandled.

**B. All-or-nothing: any subscriber failure rolls back `selection_state` to its prior value**
- Gains: State integrity — never have a partial visualisation.
- Loses: One bad mark blocks all updates. Diverges from `propagate_param`'s behaviour and the user expectation (Mosaic gracefully degrades). `selection_state` rollback adds bookkeeping that does not pay off.

**C. Mark failed subscribers and skip them on subsequent propagations until reset**
- Gains: Avoids spamming errors on a persistently-broken subscriber.
- Loses: Hides the error condition. State machine for "skipped" subscribers adds complexity. No driver for it now.

### Recommendation

**Option A.** Match `propagate_param` exactly: per-subscriber `Result`, `continue` on emit/execute error, `selection_state` always updated regardless of subscriber outcomes. The 0005 v2 review record establishes this is the project's accepted pattern. Test prefix `cfs2_` (matching the v1→v2 prefix transition `rpw_`→`rpw2_`). One AC should mirror rpw2_ac04: spec with two subscribers — one supported, one unsupported — assert `results.len() == 2` with one Ok and one Err entry.

---

## Decision summary table

```
| # | Decision                                      | Recommendation                                                            |
|---|-----------------------------------------------|---------------------------------------------------------------------------|
| 1 | Coordinator entry point                       | Separate `Session::propagate_selection(name, contributor, predicate)`     |
| 2 | Wire format / runtime state                   | New `selection_state: HashMap<String, Vec<(ComponentPath, Predicate)>>`   |
| 3 | Where resolution strategy resolves            | Per-subscriber, dispatch-time, no caching beyond DuckDB's plan-hash cache |
| 4 | Self-exclusion identity                       | Parent plot path; add `parent_plot(&str)` helper                          |
| 5 | Concurrency / ordering                        | Sync coordinator + UI debounce on brush release (mirrors navigation)      |
| 6 | Failure isolation                             | Per-subscriber Result vec, `continue` on error — matches `propagate_param`|
```

## Cross-cutting implementation notes (not decisions, but consequences)

- **emit_query must start consuming its `param_values` argument and a new `selection_predicates` argument** (or equivalent), or `propagate_selection` must lower-and-render selections inline before delegating to emit_query. The 0005 v2 review surfaced "emit_query ignores param_values" as a LOW finding; this slice is where that gap closes for selections. Decide at spec time which signature change is preferred.
- **A brush-to-predicate adapter is needed in the UI layer** — converts `Rect` plus channel bindings (e.g. `intervalX` writes a range predicate on the `x:` channel) into a `Predicate`. This is small but explicit; it should land in a tested function alongside the chart_view mouse handlers.
- **`on_mouse_up` in chart_view.rs needs wiring** — today it only sets `InteractionState::Idle` (chart_view.rs:102-109). v2 should add the `session.propagate_selection(...)` call. The UI does not currently hold a `Session`; the app shell (brightfield-app) is the integration point.
- **Test prefix `cfs2_`** matches the `rpw_`→`rpw2_` precedent. Suggested test count target (mirroring rpw2's 10): 8 tests minimum covering each decision's runtime behaviour.
- **The corpus regression gate (cfs ac-10) must remain green** — this slice does not change parsing or AST, so the gate is structurally protected, but the new fields on Session and any new Predicate construction paths should be exercised by the integration test (v2 should ship at least one cross-spec end-to-end test using `crossfilter.yaml`).
