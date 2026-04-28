# Design: Cross-Filtered Selections v2 — Runtime Coordinator

**Date:** 2026-04-28
**Interviewer:** Nightingale (rally lead)
**Card:** orbit/cards/0006-cross-filtered-selections-across-linked-views.yaml
**Rally:** orbit/specs/2026-04-28-runtime-selections-statistical-marks-rally/
**Decision pack:** decisions.md (six decisions, all accepted wholesale)

---

## Context

Card: *Cross-filtered selections across linked views* — 3 scenarios, goal: a brush in one plot updates the resolved predicate on a shared selection and triggers re-query of all linked views.

Prior specs: 1 — the 2026-04-21 spec shipped pure static analysis: filterBy validation, `SelectionSubscriberGraph`, `InteractorBinding` list, the corpus regression gate. `compile_selection` lives in `crates/brightfield-sql/src/lower.rs:94` and already implements all four resolution strategies (Crossfilter / Intersect / Union / Single) plus the self-exclusion rule, but it is only exercised by unit tests today — `emit_query` ignores its `_param_values` argument and never threads selections into the plan. Status: complete, merged at commit 4dd422e.

Companion prior art: card 0005 v2 (commit 8ca4283) shipped the runtime coordinator pattern this slice mirrors. `Session::propagate_param(name, value)` lives at `crates/brightfield-engine/src/lib.rs:244-291`. The selections coordinator follows the same shape, with two additional concerns the param coordinator does not have: multi-contributor merge (via the resolution strategy) and per-subscriber predicate synthesis (the "view's own predicate excluded from its own filter" rule).

Gap: nothing walks selection state at runtime. Brushes are not yet wired to the engine. `chart_view.rs:on_mouse_up` only resets `InteractionState::Idle`. The static-analysis subscriber graph exists but has no live consumer.

## Q&A

### Q1: Coordinator entry point — separate `propagate_selection`, unified `propagate_event`, or overload `propagate_param`?

**Q:** A selection update is structurally `(selection_name, contributor_id, predicate, generation)` — different shape from a param's `(name, scalar)`. Which API do we expose?

**A:** **Separate `Session::propagate_selection(name, contributor, predicate)`.** Selections and params are already separate abstractions in the codebase (`subscriber_graph` vs `selection_subscribers`, scalar vs predicate, single-writer vs multi-contributor). A separate entry keeps each surface tight, mirrors the existing analysis-side separation, and avoids stretching `SpecValue` into a payload-of-anything role. A unified `propagate_event` can be revisited later when more event kinds emerge — internal helpers (`execute_emitted`, partial-failure pattern) are shared without forcing a single signature.

### Q2: Wire format — what does an interactor emit, and how is it stored?

**Q:** A brush release produces a `Rect` in chart coordinates. To filter a SQL query that becomes a predicate. Where does the predicate live between emission and dispatch?

**A:** **New `selection_state: HashMap<String, Vec<(ComponentPath, Predicate)>>` field on `Session`.** Stores exactly what `compile_selection` already consumes (`&[(String, Predicate)]`). `Predicate` is the existing IR enum at `crates/brightfield-sql/src/ir.rs:36-53` — adequate for `intervalX`, `intervalY`, `intervalXY` via `And` of range expressions. ContributorId = ComponentPath gives crossfilter self-exclusion a stable identity. No `SpecValue` extension, no AST churn, no risk to the corpus parse gate. The brush-to-predicate conversion (`Rect` → `Predicate`) lives at the dispatch boundary in the UI layer.

### Q3: When does intersect / union / single resolve?

**Q:** `compile_selection` is pure and infallible. Do we resolve at dispatch time per subscriber, or cache the resolved predicate?

**A:** **Per-subscriber, dispatch-time, no caching beyond DuckDB's plan-hash cache.** Resolution is fast at Mosaic spec scale (≤5 contributors × ≤5 subscribers per selection in the corpus). DuckDB's `plan_hash` already structurally distinguishes per-subscriber predicates so the prepared-statement cache still works. No new memo or invalidation logic. Mirrors `propagate_param`'s "always re-emit" pattern.

### Q4: Self-exclusion identity — what counts as "the view's own predicate"?

**Q:** V1 D3 fixed the rule as "per-view, not per-interactor." `compile_selection` takes a `self_source: &str`. What string do we pass at runtime?

**A:** **Parent plot path.** When dispatching for mark M (path `…/plot[i]/mark[…]`), `self_source` is the parent plot prefix `…/plot[i]`. Contributor `source_name` in `selection_state` is set to the parent plot path of the contributing interactor (`…/plot[i]/interactor[intervalX]` → `…/plot[i]`). String equality in `compile_selection` (already implemented) does the rest. Add a small helper `parent_plot(path: &str) -> &str` near the path-walking utilities — strip back to the longest prefix ending in `/plot[N]`. Marks not inside a plot get the full mark path as a degenerate self_source; never collides with an interactor path → all predicates retained (correct fallback).

### Q5: Concurrency and event ordering during a fast brush drag?

**Q:** Two-tier interaction model is documented (immediate overlay during drag, deferred re-query on release). What guarantees does the coordinator make?

**A:** **Sync coordinator + UI debounce on brush release.** `&mut self` makes re-entry impossible by Rust's borrow rules. Coordinator returns when all subscribers have re-executed. UI throttles via the same debounce idiom already used in `NavigationState.check_settle` at `crates/brightfield-ui/src/interaction.rs:292-303` — `propagate_selection` is only called on `on_mouse_up` (or after a brief settle pause), never during the drag. Async dispatch with cancellation is deferred — off-sprint, no profiling driver.

### Q6: Failure isolation — one subscriber's re-query fails?

**Q:** `propagate_param` returns `Vec<(usize, Result<…, EngineError>)>` and `continue`s on per-subscriber error. Same shape for selections?

**A:** **Yes — identical pattern.** Per-subscriber `Result` vec, `continue` on emit/execute error, `selection_state` always updated regardless of subscriber outcomes. `compile_selection` is infallible by signature so no extra error variant needed. The 0005 v2 PR review (`review-pr-2026-04-24-v2.md`) explicitly approved this pattern. Test prefix `cfs2_` (mirrors `rpw_`→`rpw2_`).

---

## Summary

### Goal

Runtime coordinator on `Session` that receives selection-update events from interactors, walks the `selection_subscribers` graph, resolves per-subscriber predicates via `compile_selection` (with parent-plot self-exclusion), and dispatches re-emit + re-execute to subscribing marks. Builds directly on v1's static-analysis layer (validation, subscriber graph, resolution rules).

### Constraints

- Coordinator lives on `Session` in `brightfield-engine` — no new crate.
- Predicate IR (`crates/brightfield-sql/src/ir.rs:36-53`) is the runtime currency; no new AST or `SpecValue` variants.
- Sync dispatch via `&mut self`; UI is responsible for debouncing.
- Existing Session API (`propagate_param`, `update_param`, `execute_mark`, `execute_all`) unchanged.
- The corpus regression gate (cfs ac-10) must remain green.
- `brightfield-render` keeps its no-gpui-dependency invariant.

### Success Criteria

- `Session::propagate_selection(name, contributor, predicate)` updates `selection_state`, dispatches to all subscribers, returns per-subscriber `Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>`.
- Predicates resolved per-subscriber at dispatch time using the declared resolution strategy (intersect/union/single/crossfilter).
- A view's own predicate is excluded from its own filter (parent-plot-path equality).
- Updates from unsubscribed selections (no `selection_subscribers[name]` entry) are silently absorbed — no errors, no queries.
- Partial failure: one subscriber's emit/execute error does not block the others.
- `emit_query` actually consumes its predicate inputs (closes the `_param_values` LOW from 0005 v2 review).
- End-to-end integration test against vendored `crossfilter.yaml`: brush in plot A → resolved predicate dispatched to plot B → fresh `RecordBatch` returned.

### Decisions Surfaced

- **D1 separate entry point** — `Session::propagate_selection`, mirroring `propagate_param` but with a Predicate-typed payload.
- **D2 typed runtime state** — `selection_state: HashMap<String, Vec<(ComponentPath, Predicate)>>`; brush-to-predicate adapter sits in the UI layer.
- **D3 dispatch-time resolution, no caching** — `compile_selection` is pure and fast; rely on DuckDB plan-hash cache.
- **D4 parent-plot path equality** — `parent_plot(&str)` helper for self-exclusion identity.
- **D5 sync coordinator + UI debounce** — re-entry impossible by `&mut self`; UI throttles on brush release.
- **D6 partial-failure pattern from `propagate_param`** — `continue` on per-subscriber error.

### Implementation Notes

- **Brush-to-predicate adapter** (UI side): converts a `Rect` plus the channel-binding spec (`intervalX` writes a range predicate on the bound x channel, etc.) into a `Predicate`. Small, tested function alongside the chart_view mouse handlers.
- **`on_mouse_up` wiring**: today only sets `InteractionState::Idle` (`crates/brightfield-ui/src/chart_view.rs:102-109`). v2 calls `session.propagate_selection(...)` here. The UI does not currently hold a `Session`; the app shell (`brightfield-app`) is the integration point — the same wiring problem the param coordinator faced.
- **`emit_query` predicate threading**: this slice is where the `_param_values` LOW from the 0005 v2 PR review closes for selections — `emit_query` must consume both `param_values` and (new) `selection_predicates`. Decide at spec time whether the signature gains a new argument or whether `propagate_selection` lowers and renders inline before delegating to `emit_query`.
- **Test prefix `cfs2_`** mirrors the `rpw_`→`rpw2_` precedent. Target: at least 8 tests covering each decision's runtime behaviour, plus one end-to-end against `crossfilter.yaml`. One should mirror rpw2_ac04: two subscribers, one supported and one unsupported, assert the result vec contains one Ok and one Err.
- **Spec joins card's `specs:` array** as the v2 entry, alongside `orbit/specs/2026-04-21-cross-filtered-selections-across-linked-views/spec.yaml` (v1).

### Open Questions

- None — remaining questions are implementation-level and derivable from the codebase. The signature change to `emit_query` is the one design decision that benefits from spec-time confirmation; both options (extra argument vs lower-and-render-inline-in-coordinator) preserve the API contracts above.
