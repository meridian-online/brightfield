# Design: Reactive Parameters v2 — Runtime Coordinator

**Date:** 2026-04-24
**Interviewer:** Nightingale
**Card:** orbit/cards/0005-reactive-parameters-with-input-widgets.yaml

---

## Context

Card: *Reactive parameters with input widgets* — 7 scenarios, goal: runtime coordinator that propagates param changes through the subscriber graph to trigger re-query and re-render.

Prior specs: 1 — the 2026-04-21 spec shipped the static analysis layer: typed Input fields (as_param, from_source, filter_by), subscriber graph, param DAG with topological sort, cycle detection, type mismatch warnings. All 12 ACs verified, 16 rpw_ tests. Status: complete, merged.

Engine state: `Session::update_param(name, value)` already exists — it re-executes subscribing marks for a single param with partial failure handling. The subscriber graph and topological order from `SpecAnalysis` are available at runtime.

Gap: No coordinator that orchestrates param propagation across the subscriber graph. `update_param` handles a single param but nothing walks the DAG for chained params or provides the entry point that widgets call.

## Q&A

### Q1: Where should the coordinator live architecturally?
**Q:** The coordinator needs to walk the DAG when a root param changes. Should it live in brightfield-engine (extending Session), a new brightfield-coordinator crate, or in brightfield-app?
**A:** In brightfield-engine — extend Session with a `propagate_param()` method. Session already owns all the pieces (conn, spec, analysis, update_param). A new crate adds overhead without value. App-level coordination is fragile across multiple frontends.

### Q2: Should chained param propagation be in scope?
**Q:** Chained params need to derive downstream values from intermediate query results. How should the coordinator extract a param value from a RecordBatch?
**A:** Defer chained propagation to a follow-up. v2 handles direct param→subscribers only (5/7 scenarios). Chained extraction has unresolved design questions (which column, which row, multi-row results). Ship the direct path first, learn from real usage.

---

## Summary

### Goal
Runtime coordinator in brightfield-engine that receives param-change events and dispatches re-query + re-render to all direct subscribers of the changed param. Builds on v1's static analysis (subscriber graph, DAG, topo order). Chained DAG propagation deferred.

### Constraints
- Coordinator lives in brightfield-engine, extending Session — not a new crate
- Direct propagation only (single-hop param→subscribers) — chained DAG walking is v3
- Partial failure handling: one mark's re-query failure must not prevent others from updating
- Existing Session API (update_param, execute_mark, execute_all) unchanged
- All existing tests must continue to pass

### Success Criteria
- Session::propagate_param(name, value) dispatches to all direct subscribers and returns per-mark results
- Unsubscribed param changes produce empty results (no error)
- Partial failure: one subscriber fails, others succeed, warning surfaces
- Param values are stored in session state so subsequent queries see the updated value
- End-to-end test: parse spec with param + slider + subscribing plot → propagate_param → get fresh RecordBatch

### Decisions Surfaced
- **Coordinator in engine, not new crate**: Session already owns conn, spec, analysis, and update_param. Natural extension. Avoids crate proliferation.
- **Defer chained propagation**: Direct param→subscriber covers 5/7 card scenarios. Chained extraction (RecordBatch→param value) has ambiguity that needs real-world usage evidence before committing to a convention.

### Implementation Notes
- `Session::update_param()` already does the heavy lifting — subscriber graph lookup, mark filtering, emit + execute with param values. `propagate_param()` may be a thin wrapper initially, but establishes the coordinator entry point.
- Session needs a `param_state: HashMap<String, SpecValue>` field to track current param values. `propagate_param` updates this before dispatching queries.
- The `ParamValues` type in brightfield-sql::binding is already used by `emit_query` — param_state feeds into this.
- Consider adding `Session::current_params() -> &HashMap<String, SpecValue>` for the app/UI layer to read current values.
- Error handling: `propagate_param` returns the same `Vec<(usize, Result<...>)>` shape as `update_param` — callers already handle partial failure.
- Test prefix: `rpw2_` to distinguish from v1's `rpw_` tests.

### Open Questions
- None — remaining questions are implementation-level and derivable from the codebase.
