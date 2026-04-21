# Design: Reactive Parameters with Input Widgets

**Date:** 2026-04-21
**Interviewer:** Nightingale
**Card:** orbit/cards/0005-reactive-parameters-with-input-widgets.yaml

---

## Context

Card: *Reactive parameters with input widgets* — 5 scenarios, goal: an analyst adjusts a slider, menu, search box, or table selection and watches the view update live without re-authoring the query.
Prior specs: 0 — this is the first spec for card 0005.
Gap: The AST already represents `Input` nodes with `InputKind` variants and `ParamNode`/`ParamRef` types (card 0001), and the SQL emitter handles binding and selection compilation (card 0003). What remains is the reactive propagation model, the semantic contract for widget-param bindings, and edge-case behaviour for dead params, chained propagation, and type mismatches.

## Q&A

### Q1: Consistency guarantee for reactive updates

**Q:** When a param changes and multiple views depend on it, what should the user experience? Should two side-by-side plots ever show data from different slider positions, or must they always reflect the same parameter state?

**A:** All views must reflect the same parameter state. Epoch-consistent updates — each param change increments an epoch, every subscriber's query executes against one epoch's values, and results from superseded epochs are silently discarded. Transient staleness during query flight is acceptable (a view can show the prior epoch while its query runs), but no view should ever display results from a *different* epoch than its peers once results arrive. This prevents "tearing" across the dashboard.

### Q2: Widget-param binding as a first-class contract

**Q:** When an input widget declares which parameter it writes to, should that binding be discoverable from the spec's structure alone, or is it acceptable to require runtime inspection of the widget's options?

**A:** The binding must be statically visible at the AST level. Promote `as:`, `from:`, and `filterBy:` to typed fields on `Input` — not buried in the generic options bag. This makes widget-param relationships enumerable at parse time: the coordinator can build the full routing table without interpreting option values. The precedent is `Mark.data` — already a typed field alongside its options bag. The corpus universally uses these keys on inputs, so they are not edge cases.

### Q3: Behaviour when a parameter has no subscribers

**Q:** If an analyst declares a parameter but nothing in the spec references it — perhaps a widget was removed during development — what should happen when that parameter's value changes?

**A:** Two-layer response. At spec-load time, emit a non-fatal warning (`ParseWarning::DeadParam`) so the author knows something is unconnected — this catches typos like `$threshhold` vs `$threshold`. At runtime, updates to dead params are silently absorbed: no queries fire, no errors surface. This satisfies the card's scenario 5 ("silently absorbed") while still providing author feedback where it matters.

### Q4: Cascading changes through chained parameters

**Q:** When one parameter drives a query whose result sets a second parameter, and a plot depends on that second parameter, what ordering guarantee should the user experience? Should downstream views always see settled values, or is it acceptable for intermediate states to be visible?

**A:** Downstream views must always see settled values. Build a topological DAG of param dependencies at spec-load time — nodes are params, edges represent "param X is consumed by a component that writes param Y." Propagate in topological order within each epoch so that by the time a downstream subscriber fires, all upstream params have stabilised. Cycles are detected at load time and rejected as spec authoring errors — a cycle means the spec cannot reach a stable state.

### Q5: Type safety between widgets and parameters

**Q:** If an input widget emits a value whose type does not match what the parameter's subscribers expect — say, a menu bound to a numeric parameter — should the system catch that at authoring time, or let the SQL engine surface it as a query error?

**A:** Catch it at authoring time with static type inference. Infer the param's expected type from its declaration (literal value implies scalar type; selection declaration implies selection type) and the widget's output type from its `InputKind` (slider emits numeric, menu emits string, search emits string, table emits selection). Emit a non-fatal `ParseWarning::ParamTypeMismatch` for provably incompatible bindings. Leave ambiguous cases unchecked — the SQL engine remains the backstop for edge cases. This aligns with engineering principle 3: programmatic checks for validation.

---

## Summary

### Goal

An analyst adjusts input widgets (slider, menu, search, table) and all dependent views update reactively — no re-authoring, no tearing, no stale states left behind.

### Constraints

- Epoch-consistent propagation: no mixed-epoch results visible across subscribers
- Widget-param bindings must be statically enumerable from the AST
- Dead params must not cause runtime errors
- Chained propagation must settle in topological order; cycles are rejected
- Type mismatches caught at load time where provable; SQL engine is backstop for ambiguous cases

### Success Criteria

- All subscribing views reflect the same param epoch once their queries complete
- `Input` struct exposes `as_param`, `from`, and `filter_by` as typed fields
- `ParseWarning::DeadParam` emitted for params with zero subscribers
- Param dependency DAG built at load time; cycles produce a spec error
- `ParseWarning::ParamTypeMismatch` emitted for provably incompatible widget-param bindings

### Decisions Surfaced

- **D1 — Epoch-consistent updates** (option B): chose epoch-based propagation over fire-and-forget (A) or synchronous barrier (C) because it prevents tearing without blocking input. Stale-epoch results are discarded.
- **D2 — Typed fields on Input** (option B): chose promoting `as:`/`from:`/`filterBy:` to typed fields over convention-only (A) or a trait (C) because static visibility matters for the coordinator and follows the `Mark.data` precedent.
- **D3 — Static warning + runtime no-op** (option B): chose dual-layer over runtime-only (A) or static error (C) because it catches typos without breaking valid specs that have temporarily unused params.
- **D4 — Topological DAG** (option A): chose DAG-ordered propagation over reactive pull (B) or flat propagation (C) because topological order guarantees settled values at every level and cycle detection prevents infinite loops.
- **D5 — Static type inference** (option B): chose load-time type checking over no checking (A) or runtime coercion (C) because programmatic checks for validation are a project principle, and cryptic SQL errors are unacceptable UX.

### Implementation Notes

- The epoch counter extends naturally from `EmittedQuery.plan_hash` — hash identifies query shape, epoch identifies param values. Together they form the result-cache key (card 0003 D5).
- `Input` struct in `crates/brightfield-spec/src/ast.rs` currently stores `as:` in the generic options `IndexMap`. Parser change: extract `as:`, `from:`, and `filterBy:` before inserting remaining keys into options. See `parse.rs:910` for the existing `lift_field` pattern.
- `ParseWarning::UnknownOption` in `crates/brightfield-spec/src/parse.rs` is the precedent for new warning variants (`DeadParam`, `ParamTypeMismatch`).
- The subscriber graph needed for D1/D3/D4 can be built in one pass over the component tree: for each component, collect param refs from expressions, `filterBy:` fields, and `from:` fields. Invert to get param-to-subscribers map.
- `emit.rs` currently ignores `Component::Input` nodes — the emitter will need to handle inputs that have backing queries (e.g., a menu with `from:` that populates its options from a data source).
- Corpus reference: `vendor/mosaic-specs/yaml/athletes.yaml` demonstrates the full pattern — two menus with `as:`/`from:`/`column:`, a search with `filterBy:`/`as:`, and downstream marks consuming the resulting params.
- Widget output type vocabulary is small and fixed: `Scalar(Numeric)` for Slider, `Scalar(String) | Array(String)` for Menu, `Scalar(String)` for Search, `Selection` for Table.

### Open Questions

- **OQ1:** Should the epoch counter live in the emitter crate or the coordinator? The emitter produces `EmittedQuery` with `plan_hash`; the epoch is a runtime concern that may belong above the emitter.
- **OQ2:** Should `Interactor` also gain typed `as:` / `filterBy:` fields for symmetry, or is the existing options-bag treatment sufficient given card 0003's selection compilation handles interactor bindings?
- **OQ3:** What is the maximum chain depth before warning? Corpus specs show at most 2-level chains. A hard limit (e.g. 5) would prevent combinatorial explosion in deeply-chained specs.
- **OQ4:** Should menu widgets infer option types from the data source schema (via `from:` + `column:`) at load time, or defer that to a later spec?
