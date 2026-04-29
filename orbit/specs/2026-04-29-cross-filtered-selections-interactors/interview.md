# Design: Cross-Filtered Selections v3 — Interactor Surface & Lifecycle

**Date:** 2026-04-29
**Interviewer:** Nightingale (rally lead)
**Card:** orbit/cards/0006-cross-filtered-selections-across-linked-views.yaml
**Rally:** orbit/specs/2026-04-29-live-reactivity-rally/
**Decision pack:** decisions.md (six decisions, all accepted wholesale)

---

## Context

Card: *Cross-filtered selections across linked views* — widened from 3 → 6 scenarios. The first three (brush-filters-others, resolution-strategy-in-spec, view-own-selection-does-not-filter-itself) shipped at the runtime layer in v2. The three new ones — **clearing retracts a contributor's predicate**, **a plot can drive multiple selections at once**, and **selections persist across param changes when the domain still applies** — are the centre of gravity for this slice.

Prior specs: 2.
- v1 (2026-04-21, commit 4dd422e): static analysis. `filterBy` validation, `SelectionSubscriberGraph`, `InteractorBinding` list, the corpus regression gate. `cfs_*` test prefix.
- v2 (2026-04-28, commit 8ca4283): runtime coordinator. `Session::propagate_selection` at `crates/brightfield-engine/src/lib.rs:262-335`, `selection_state: HashMap<String, Vec<(ComponentPath, Predicate)>>` at `lib.rs:152`, `parent_plot` helper in `brightfield-spec`, `brush_rect_to_predicate` and `BrushBinding` in `brightfield-ui`, brush release wired to coordinator via `on_mouse_up_with_dispatch` at `chart_view.rs:128-157`. `emit_query` now consumes both `param_values` and `selection_predicates` (the `_param_values` LOW from 0005 v2 closed). `cfs2_*` test prefix, 16 tests, approved in `review-pr-2026-04-28.md`.

Gap this slice closes:
- **Clearing**: `propagate_selection` only inserts/replaces a contributor's entry — there is no retract path. `chart_view.on_mouse_up` only commits a brush; idle clicks vanish.
- **Multi-binding**: `BrushBinding` is singular (one selection, one kind, one channel set). The corpus's multi-selection patterns (protein-design.yaml, athletes.yaml) compose multiple components per plot; the runtime must support multiple contributor bindings on a single ChartView.
- **Persistence across param changes**: v2 already preserves `selection_state` across `propagate_param` calls (verified at `lib.rs:464-466`), but the contract is not regression-tested. This slice nails it down.

Rally seam: paired with card 0005 v3 in `orbit/specs/2026-04-29-reactive-parameters-runtime/`. Both cards share `Session` but touch disjoint mutable fields — this card writes `selection_state`, card 0005 writes `param_state`. The rally gate is: neither coordinator clobbers the other's state. Decision 5 commits the regression test that pins it.

## Q&A

### Q1: How does a user retract a selection's contribution at runtime, and what does the coordinator API look like?

**Q:** Scenario 4 says "clicking outside the brushed region or otherwise clearing it" retracts the contribution. The shipped `propagate_selection` only inserts/replaces — there is no remove. Do we add a dedicated method, overload via a sentinel `Predicate::True`, or introduce a `SelectionUpdate` enum?

**A:** **New `Session::clear_selection(name: &str, contributor: ComponentPath) -> Vec<(usize, Result<…>)>` symmetric to `propagate_selection`.** Linear-scan find-and-remove in `selection_state[name]` (the same machinery used at `lib.rs:271-275` for replacement), then re-emit + re-execute through every subscriber via the existing dispatch loop. `compile_selection` at `lower.rs:357-362` already returns `Predicate::True` when the contributor list is empty, so removal is mechanically the right thing — the now-shorter slice flows through unchanged. Sentinel-via-`Predicate::True` (Option B) was rejected because a legitimate "interval over the full domain" brush could legally produce that predicate; conflating it with absence reintroduces the stringly-typed-payload smell the v2 decisions explicitly flagged. The `SelectionDispatcher` trait at `crates/brightfield-ui/src/brush.rs:120-129` grows a `clear` method; the `Session: SelectionDispatcher` impl forwards. Chart_view dispatches `clear` on click-outside-active-brush in `chart_view.rs:107-114` (today's `on_mouse_up` only resets to `Idle`). Click-vs-drag discrimination requires a small change: gate brush-start on a minimum drag distance, or defer brush-start until first mouse-move-while-down — a zero-area brush today would otherwise look indistinguishable from a click.

### Q2: How is a "point selection" produced — chart-side click, or input widgets?

**Q:** Scenario 5 mentions "a point selection on one channel and an interval selection on another." Chart-side click-to-point requires a row-identity convention in the SQL layer (no spec declares one); input-driven point selections (the corpus's `input: table` writing `as: $point` pattern, e.g. protein-design.yaml:147-148, athletes.yaml:67-68) are already shaped for the coordinator. Which surface produces the point predicate in this slice?

**A:** **Defer chart-side click-to-point; treat input-widget-driven point selections as the canonical driver.** Card 0005 v3 (the rally pair) is where input widgets become live emitters — landing point-via-input there avoids duplicating the work and keeps this card's scope on interval-brush surface coverage. To unblock card 0005 without forcing it to define new types, **this card lands `BrushKind::Point` as a forward-compat enum variant** in `crates/brightfield-ui/src/brush.rs:21-28` plus a `point_predicate(column, value)` adapter alongside `brush_rect_to_predicate`. The variant is constructed but not wired to mouse-up's brush rect — it has no chart_view dispatch path in this slice. Chart-side click-on-mark (Option B in the decision pack) was deferred because it requires either a unique-key column convention or a DuckDB-specific `rowid` anchor — both are larger commitments than scenario 5 needs, and neither corpus spec declares row identity.

### Q3: A plot binds to two selections — singular `BrushBinding` or `Vec`?

**Q:** Today `BrushBinding { selection_name, contributor, kind, channels }` (`chart_view.rs:165-174`) is a single tuple. Scenario 5 says "a plot is bound as contributor to two distinct selections." Do we generalise the type, hold a list, or defer entirely?

**A:** **ChartView holds `Vec<BrushBinding>`; `on_mouse_up_with_dispatch` iterates and dispatches one `propagate_selection` per binding whose kind is compatible with the produced brush rect.** The `BrushBinding` struct stays singular; the multiplicity lives at the call site, mirroring `propagate_param`'s subscriber loop shape. Each binding consumes only its kind's coordinates from the rect — an `intervalX` binding ignores the y-range, an `intervalXY` binding consumes both. The result vec generalises to `Vec<(selection_name, Vec<(usize, Result<…>)>)>` so the test double can record per-binding outcomes. The corpus has no per-plot multi-binding case today, so the AC drives a synthetic spec (two `intervalXY` interactors on one plot, writing `as: $a` and `as: $b`) — same shape as v2's `cfs2_ac06_resolution_strategies_runtime` mini-specs. Generalising the struct itself (Option B) was rejected as over-fitting with no concrete consumer; deferring entirely (Option C) was rejected because scenario 5 demands AC-level evidence that the multi-binding shape works.

### Q4: Where does interactor kind and channel metadata live so chart_view can build `BrushBinding`s?

**Q:** `analysis.interactor_bindings` (`analysis.rs:622-628`) records only `path: ComponentPath` and `selection: String`. Chart_view needs the interactor kind (to map to `BrushKind`) and the parent plot's bound channel columns. Do we extend `InteractorBinding`, add a sibling helper, or add a new derived view?

**A:** **Add a derived `analysis.brushable_bindings: Vec<BrushableBinding>` field; leave v1's `interactor_bindings` untouched.** Each `BrushableBinding` carries the interactor path, the parent plot path (the contributor identity), the selection name, the interactor kind, and the resolved channel columns from the parent plot's `x:`/`y:` options. Brush-incompatible kinds (`Toggle`, `Highlight`, `Nearest*`, `Pan*`) are filtered out — those run on a different runtime path and do not need a `BrushBinding`. Extending `InteractorBinding` directly (Option A) was rejected because it breaks v1's `cfs_ac08` count test, the round-trip property at `dfspec_ac11`, and any binding-destructuring consumer; the derived-view route preserves all v1 surfaces. A `From<&BrushableBinding> for BrushBinding` conversion lives in `brightfield-ui` (the BrushBinding's home crate) so the layering stays clean. **Card 0005 v3 may add a parallel `analysis.input_bindings` for input widgets** (`as: $name` on `input:` declarations is walked separately from interactors today at `analysis.rs:798-806`); the two new fields are non-overlapping.

### Q5: Selections persisting across param changes — domain check, auto-clear, or trust the predicate?

**Q:** A brush is `delay BETWEEN 50 AND 100`. The user moves a slider that shifts the data so no rows match `[50, 100]`. Scenario 6 says the brush "continues to filter downstream views — the selection is independent of param-driven re-execution as long as the brushed domain is still meaningful." What does "meaningful" mean to the runtime?

**A:** **Trust the predicate — brush persists verbatim.** v2 already preserves `selection_state` across `propagate_param` (`lib.rs:464-466`: "Selection predicates are threaded from the live selection_state so a propagate_param call after a brush release continues to honour the active selection"); this slice pins the contract with two regression tests rather than inferring meaningfulness. Empty results are the truthful answer; the user clears via Decision 1's path. Surfacing a `SelectionDomainOutOfRange` warning (Option B) needs a new runtime-warning channel and structural range extraction from `Predicate::Expr(String)` — the IR has no typed ranges, only opaque expression strings. Auto-clearing (Option C) is a UX footgun: zero rows could be the correct answer, and destroying user state on heuristic grounds inverts the principle of trust. The qualifier in the card text is satisfied at the user-experience level, not the runtime. A follow-up memo captures the option for revisit if user feedback reveals confusion.

### Q6: Test prefix — extend `cfs2_` or create `cfs3_`?

**Q:** v1 tests use `cfs_*`, v2 uses `cfs2_*`, the v2 ac-15 gate counts `cfs2_` tests literally. Where do this slice's new tests live?

**A:** **New prefix `cfs3_`.** Mirrors the `rpw_`→`rpw2_` precedent the v2 spec itself called out, and matches the rally's pairing (card 0005 v3 will use `rpw3_`). The v3 ac-count gate is `rg -n '\bfn cfs3_' crates/ | wc -l >= 7` — one per code-typed AC enumerated in the decision pack (clearing × 2, click-outside × 1-2, multi-binding × 1, brushable_bindings × 2, persistence × 2, BrushKind::Point × 1). v1's `cfs_*` and v2's `cfs2_*` stay green and untouched. Re-prefixing v2 tests (Option C) was rejected as destructive churn against an already-approved review.

---

## Implementation contract

### Files modified

- `crates/brightfield-engine/src/lib.rs` — adds `Session::clear_selection`; the body mirrors `propagate_selection`'s dispatch loop with linear-scan removal in place of insertion. No change to `propagate_param`.
- `crates/brightfield-ui/src/brush.rs` — adds `BrushKind::Point`, `point_predicate(column, value)` adapter, `SelectionDispatcher::clear` trait method.
- `crates/brightfield-ui/src/chart_view.rs` — generalises ChartView to hold `Vec<BrushBinding>`; `on_mouse_up_with_dispatch` iterates bindings and dispatches one `propagate_selection` per kind-compatible binding; new `commit_brush_clear` pure helper for click-outside-clear; click-vs-drag discrimination via minimum-drag-distance gate (or deferred-brush-start).
- `crates/brightfield-spec/src/analysis.rs` — adds `BrushableBinding` struct and `build_brushable_bindings` walker; new field `brushable_bindings: Vec<BrushableBinding>` on `SpecAnalysis`. v1's `interactor_bindings` shape is untouched.

### New types / traits / functions

- `Session::clear_selection(&mut self, name: &str, contributor: ComponentPath) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>`
- `SelectionDispatcher::clear(&mut self, name: &str, contributor: ComponentPath) -> Vec<(usize, Result<…>)>`
- `BrushKind::Point` variant + `point_predicate(column: &str, value: &str) -> Predicate`
- `analysis::BrushableBinding { interactor_path, parent_plot, selection, kind, channels }`
- `analysis::SpecAnalysis::brushable_bindings: Vec<BrushableBinding>`
- `From<&BrushableBinding> for BrushBinding` (in brightfield-ui)
- `commit_brush_clear` pure helper (sibling to `commit_brush_release`) in `chart_view.rs`

### Test surface

- Test prefix: `cfs3_`. AC count target: ≥ 7. Distribution:
  - `brightfield-engine`: `cfs3_ac01_clear_selection_removes_contributor`, `cfs3_ac02_clear_selection_unsubscribed_silent`, `cfs3_ac05_plot_drives_multiple_selections`, `cfs3_ac08_param_change_preserves_selection`, `cfs3_ac09_propagate_param_does_not_clobber_selection_state`.
  - `brightfield-spec`: `cfs3_ac06_brushable_bindings_built`, `cfs3_ac07_brushable_binding_to_brush_binding`.
  - `brightfield-ui`: `cfs3_ac03_click_outside_active_brush_clears`, `cfs3_ac10_brush_kind_point_constructs` (and `cfs3_ac04_escape_key_clears_active_brush` if escape routing is in scope at spec time).
- Corpus regression gate (`cfs ac-10`) and v2 gates (`cfs2_*`) remain green. No AST or parser changes.

### Rally seam commitments

- `propagate_param` reads `selection_state` (via `selection_predicates_for_emit`) but never writes — pinned by `cfs3_ac09`.
- `propagate_selection` and `clear_selection` never touch `param_state`.
- `analysis.interactor_bindings` shape is unchanged; v1's `cfs_ac08` count test stays green.
- `analysis.brushable_bindings` (this card) and `analysis.input_bindings` (card 0005 v3, if introduced) are non-overlapping new fields. PR review for both cards must confirm `git diff` on `analysis.rs` is structurally non-overlapping in the SpecAnalysis struct.
- `emit_query` signature is unchanged — both `param_values` and `selection_predicates` arguments already exist post-v2.

### Coordination notes / known follow-ups

- **Point selection driver gap (Decision 2 ↔ card 0005 D5):** chart-side click-to-point is deferred to a future card. Input-widget-driven point selections are card 0005 v3's surface; this card lands only the `BrushKind::Point` type for forward compat. **Rally review and PR descriptions for both cards must reference this gap explicitly** so the absence of a chart-side click handler is not mistaken for an oversight.
- **Selection-domain meaningfulness (Decision 5):** captured as a memo (`orbit/cards/memos/2026-04-29-selection-domain-meaningfulness.md`) for revisit if user feedback shows confusion when a persisted brush returns zero rows after a param change.
- **Click-vs-drag gate (Decision 1 consequence):** `chart_view.on_mouse_down` today starts a brush immediately. The minimum-drag-distance gate (or deferred-start) is small but visible behaviour change; the spec records the choice explicitly.
- **App shell wiring of `Vec<BrushBinding>`:** today `BrushBinding` is constructed only in tests (no matches in `brightfield-app/`). Picking up `analysis.brushable_bindings` at ChartView construction is an integration step the app shell card will pick up; this card's ACs use test doubles where needed.

### Open Questions

- None — the six decisions cover the surface. Implementation choices (linear-scan vs `IndexMap` for `selection_state`, `From` impl placement, exact escape-key routing) are in-scope for spec time and derivable from the codebase.
