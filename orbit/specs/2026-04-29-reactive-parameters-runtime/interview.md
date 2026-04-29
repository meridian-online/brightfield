# Design: Reactive Parameters v3 — Live Reactivity (chained walk + slider wiring)

**Date:** 2026-04-29
**Interviewer:** Nightingale (rally lead)
**Card:** orbit/cards/0005-reactive-parameters-with-input-widgets.yaml
**Rally:** orbit/specs/2026-04-29-live-reactivity-rally/
**Decision pack:** decisions.md (six decisions, all accepted wholesale)

---

## Context

Card: *Reactive parameters with input widgets* — 7 scenarios, goal: an analyst adjusts a slider, menu, search box, or table selection and watches the view update live. v1 (commit shipped in the 2026-04-21 spec) delivered pure static analysis: typed `Input.{as_param, from_source, filter_by}` fields, `subscriber_graph`, dependency DAG, `topological_order`, cycle detection, type-mismatch warnings. v2 (the 2026-04-24 spec) shipped the runtime coordinator's *direct-only* slice: `Session::propagate_param(name, value)` at `crates/brightfield-engine/src/lib.rs:433-489`, `param_state: ParamValues`, partial-failure pattern, `current_params()` accessor. Status: both complete, merged.

Companion prior art: card 0006 v2 (the 2026-04-28 selections runtime) shipped the sibling pattern this slice mirrors. `Session::propagate_selection` at `crates/brightfield-engine/src/lib.rs:262-336`, `selection_state` field, `SelectionDispatcher` trait at `crates/brightfield-ui/src/brush.rs:120-140`, `on_mouse_up_with_dispatch` at `crates/brightfield-ui/src/chart_view.rs:128-157`, and the `emit_query` widening to consume both `param_values` and `selection_predicates`. The selections rally proved the shape; this slice closes the param half of the live-reactivity sprint.

Gap: two pieces remain. (1) `propagate_param` ignores `analysis.topological_order` — scenario 4 ("Chained params propagate in DAG order") is unimplemented; v2 interview Q2 explicitly deferred it. (2) Nothing in `crates/brightfield-ui/src/` calls `propagate_param`. `InputKind::{Slider, Menu, Search, Table}` exist in `crates/brightfield-spec/src/vocab.rs:218-225` but are all flagged `Unimplemented` and have no widget code. Scenario 2 ("Input widgets are first-class param emitters") cannot fire. The rally seam is clean: the selections half is shipped and untouched by this slice.

## Q&A

### Q1: Coordinator entry point — extend `propagate_param` or add `propagate_param_chain`?

**Q:** v2's `propagate_param` does direct-only dispatch. To honour scenario 4 it needs to walk `analysis.topological_order`. Do we strengthen the existing method or add a sibling?

**A:** **Extend `propagate_param` in place.** The v2 spec explicitly framed chained walking as deferred work *on this same method*, not as a new entry point ("Direct propagation only … chained DAG walking is deferred to a future spec"). One entry point keeps the widget-side simple — the slider always calls `propagate_param(name, value)` and the coordinator decides whether to walk one level or many. `propagate_selection` followed the same single-method shape (engine/lib.rs:262); this preserves cross-coordinator symmetry. v2 callers (rpw2 tests, gomb_ac12, dex_ac06's `update_param` legacy path) see no behavioural change for non-chained specs because decision 3's first-level-wins dedup makes the direct case bit-identical.

### Q2: DAG walk semantics — what does each level *compute*?

**Q:** v2 interview Q2 named the unresolved design question: when the walk reaches a downstream param B, where does B's value come from? "Which column, which row, multi-row results."

**A:** **Topological re-execution against full `param_state`; computed-param case deferred.** Three structural cases exist: (i) simple multi-subscriber (already shipped); (ii) *filtered widget chain* — an input has both `filter_by: $A` and `as_param: $B`, so changing A re-fires the widget's `from`-source query and the user's *next* interaction writes B (this is `athletes.yaml`); (iii) *computed param chain* — a query result derives a param value (no corpus example, no `ParamNode::FromQuery` AST surface). The walk handles (i) and (ii) by re-emitting subscribing marks at each level against the current `param_state`; case (iii) stays out of scope until a corpus or user driver appears. Evidence: `analysis.dependency_dag` edges (analysis.rs:385-427) only fire when a single component both consumes and writes params — the corpus path is exactly case (ii). Document the deferral in the spec's implementation notes.

### Q3: Per-walk dedup — first-level-wins or last-level-wins?

**Q:** A mark whose query references *both* `$A` and `$B` (where B descends from A in the DAG) appears in `subscriber_graph[A]` and `subscriber_graph[B]`. Re-executing twice is wasteful and breaks the "one fresh RecordBatch per affected mark" reading of scenario 4.

**A:** **First-level-wins via a `dispatched_marks: HashSet<usize>` carried across the walk.** Maintain the set across levels; before dispatching a mark, check membership and skip if present. Produces a result vec with one `(mark_idx, Result)` per affected mark in topological order of first appearance — same shape as v2 `propagate_param` and `propagate_selection`. For in-scope cases (i) and (ii), `param_state` is the source of truth from level 0 onward, so first-level dispatch sees current values for all upstream params. The existing in-level `mark_indices.sort(); mark_indices.dedup();` (engine/lib.rs:456-457) handles the same-level case; the new HashSet handles the cross-level case.

### Q4: Partial-failure isolation — match `propagate_selection` exactly?

**Q:** v2's `propagate_param` uses the `continue` pattern (engine/lib.rs:475-481), but v2's ac-04 review (`review-pr-2026-04-24-v2.md` MEDIUM) flagged the test couldn't exercise mixed Ok/Err because no lowerers were registered. Lowerers are now registered. Does the v3 slice strengthen this and stay aligned with `cfs2_ac08`?

**A:** **Yes — match `propagate_selection`'s shape exactly and strengthen ac-04.** Per-subscriber `Result`, `continue` on emit/execute error, `param_state` always updated regardless of outcomes, walk continues across levels regardless of per-mark errors. Strengthened ac-04 mirrors `cfs2_ac08`: two subscribers, dot (supported lowerer) + rect (no registered lowerer), assert `results.len() == 2` with one Ok + one Err. Scenario 7's "warning surfaces the error" is partially satisfied by the Err entry in the result vec; a richer warning channel is deferred (UI concern, no current driver).

### Q5: Widget→coordinator wiring — slider only, all four widgets, or trait-only?

**Q:** Card scenario 2 names slider, menu, search, and table. The selections runtime (cfs2_ac10/ac11) shipped *one* input source (the brush) plus a `SelectionDispatcher` trait. Which input widget(s) land in this slice?

**A:** **Slider only.** Mirrors the cfs2 precedent: ship one widget end-to-end, define the trait that lets the others slot in later. Concretely:
- New `crates/brightfield-ui/src/slider.rs` — GPUI widget with track + thumb, mouse handlers, `value: f64` bound via `Input.options` (`min`, `max`, `step`).
- `ParamDispatcher` trait alongside `SelectionDispatcher` (in `slider.rs` or a new `param.rs`) with `dispatch(&mut self, name: &str, value: SpecValue) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>`.
- `impl ParamDispatcher for brightfield_engine::Session` forwarding to `propagate_param`.
- `SliderBinding { param_name, min, max, step }` analogous to `BrushBinding` (chart_view.rs:165-174).
- `InputKind::Slider` flips from `Unimplemented` to `Implemented` in `crates/brightfield-spec/src/vocab.rs:222`.

Menu/Search/Table remain `Unimplemented` and are deferred to a future sprint candidate. Scenario 2 is satisfied at the *runtime* layer (any widget *can* dispatch via the trait) and at the *integration* layer for the canonical widget kind.

### Q6: Re-render integration — coordinator returns batches, or dispatches render directly?

**Q:** `propagate_param` returns `Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>` today. Does v3 keep that shape or hand the engine a `RenderSink`?

**A:** **Coordinator returns batches; UI observes the result vec and re-renders.** Symmetric with v2 and `propagate_selection`. Engine stays free of any render dependency, preserving `brightfield-render`'s no-gpui invariant. The slider widget's dispatcher invocation returns the result vec; the app shell or a `ChartView::propagate_param_and_redraw` helper updates `ChartState` and triggers `cx.notify()`. UI-side debounce mirrors decision 5 of the cfs2 pack: slider commits its value to the dispatcher only on `mouse_up`; mid-drag is overlay-only state on the slider widget itself.

---

## Implementation contract

### Files modified
- `crates/brightfield-engine/src/lib.rs` — `propagate_param` body changes from direct dispatch (lines 433-489) to a topological walk; signature unchanged.
- `crates/brightfield-spec/src/analysis.rs` — adds `topological_descendants(analysis, root_param) -> Vec<String>` near `build_dependency_dag` (lines 315-383).
- `crates/brightfield-spec/src/vocab.rs` — `InputKind::Slider` flips from `Unimplemented` to `Implemented` (line 222).
- `crates/brightfield-ui/src/lib.rs` — re-export the new `slider` module.

### Files added
- `crates/brightfield-ui/src/slider.rs` — `Slider` GPUI widget, `SliderBinding`, `ParamDispatcher` trait, `Session: ParamDispatcher` impl, `commit_slider_release` pure helper for testability.

### New types / traits / functions
- `pub trait ParamDispatcher { fn dispatch(&mut self, name: &str, value: SpecValue) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>; }` — mirrors `SelectionDispatcher` (`crates/brightfield-ui/src/brush.rs:120-140`).
- `pub struct SliderBinding { param_name: String, min: f64, max: f64, step: Option<f64> }` — mirrors `BrushBinding` (`crates/brightfield-ui/src/chart_view.rs:165-174`).
- `pub fn topological_descendants(analysis: &SpecAnalysis, root: &str) -> Vec<String>` — pure DAG traversal restricted to descendants of `root`, root included as first element.
- `commit_slider_release` — pure helper analogous to `commit_brush_release` (chart_view.rs:181-206), enables UI tests against a recording dispatcher double.

### Vocab status flips
- `InputKind::Slider`: `Unimplemented` → `Implemented`. Paired with a conformance assertion to prevent regression.

### Test prefix and AC count target
- Prefix: `rpw3_` (mirrors `rpw_` → `rpw2_` → `rpw3_`).
- Target: ≥10 tests. Coverage: engine (chained walk, dedup, partial failure, unsubscribed-leaf no-op, descendants-only scope, dispatcher trait forwarding); spec (`topological_descendants` simple + athletes.yaml chain); UI (slider on_mouse_up dispatches, no-drag no-dispatch).

### Rally seam commitments (read-only crossings, untouched fields)
- `selection_state` — **read only** (passed through to `emit_query` so chained re-executions honour the active brush; v2 already does this at engine/lib.rs:467-472).
- `propagate_selection`, `current_selections`, `SelectionDispatcher`, `BrushBinding`, `brush_rect_to_predicate`, `on_mouse_up_with_dispatch` — **untouched**.
- `emit_query` / `emit_query_with_passes` — **signature untouched** (cfs2 already widened them to consume both `param_values` and `selection_predicates`; this slice consumes that surface unchanged).
- `update_param` — **untouched** legacy method; has its own callers (dex_ac06). Cleanup deferred.
- `analysis.subscriber_graph`, `analysis.topological_order`, `analysis.dependency_dag` — **read only**; no schema or builder changes.
- `brightfield-render` — **untouched** (no-gpui invariant intact).
- Corpus regression gate (cfs ac-13) — **must remain green**; this slice has no AST or parser changes.
