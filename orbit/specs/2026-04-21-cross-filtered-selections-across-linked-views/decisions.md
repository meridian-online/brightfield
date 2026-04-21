# Decision Pack: Cross-Filtered Selections Across Linked Views

**Card:** 0006-cross-filtered-selections-across-linked-views
**Date:** 2026-04-21

---

## Decision 1: What does "empty selection" mean for the user?

### Context

When no interactor has contributed a predicate to a selection (initial page load, or after the user clears a brush), every linked view must decide what to show. The current `compile_selection` in `lower.rs` returns `Predicate::True` for empty predicates, which means "show all rows." But there are two legitimate user expectations: "show everything" (unfiltered) or "show nothing" (no match yet). The choice affects whether the dashboard looks populated or blank on first render.

### Options

**A. Empty selection = unfiltered (Predicate::True -- show all rows)**
- Gains: Dashboard is immediately useful on load. Users see the full dataset and narrow down by brushing. Matches Mosaic's behaviour.
- Loses: No visual distinction between "nothing selected" and "everything selected." Could confuse users who expect a blank state before interaction.

**B. Empty selection = empty result (Predicate::False -- show no rows)**
- Gains: Clear visual signal that interaction is required. Useful for guided workflows.
- Loses: Dashboard appears broken on load. Violates the Mosaic convention. Every spec in the corpus (`crossfilter.yaml`, `flights-200k.yaml`, `overview-detail.yaml`) assumes plots are populated before any brush.

**C. Selection-level `empty` option controls the behaviour**
- Gains: Spec authors can choose per-selection. The `SelectionNode.options` bag already has space for an `empty` key.
- Loses: Adds a decision point for spec authors that Mosaic itself does not surface. Additional branching in compile_selection.

### Recommendation

**Option A.** The codebase already implements this (`compile_selection` returns `Predicate::True` for empty). All three vendored crossfilter specs assume populated plots on load. This matches Mosaic's semantics. Option C could be added later if a use case emerges, but adding it now is speculative.

---

## Decision 2: How should interactors declare which selection they write to?

### Context

An interactor (e.g. `intervalX`) produces a predicate and must route it to a named selection. The Mosaic spec uses the `as: $brush` option on interactors to establish this binding. The parser already lifts this to `ValueOrParamRef::Param(ParamRef("brush"))` (verified in `crossfilter.rs` test, line 50). The question is whether the spec contract should allow additional binding shapes (e.g. writing to multiple selections, or implicit binding by plot co-location).

### Options

**A. Explicit `as: $selection` only -- one interactor writes to exactly one selection**
- Gains: Simple, declarative, auditable. Matches every Mosaic spec in the corpus. The AST already represents this via `Interactor.options["as"]`. No ambiguity about which predicate lands where.
- Loses: An interactor cannot drive two independent selections without duplicating it in the spec.

**B. Allow `as: [$sel1, $sel2]` -- one interactor writes to multiple selections**
- Gains: Avoids duplication when one brush feeds multiple selection strategies.
- Loses: Not in the Mosaic spec. Would require extending the parser's lift logic. No corpus example uses this pattern. Increases compile_selection complexity (same predicate appears in multiple selection pools).

**C. Implicit binding by plot co-location (interactor auto-binds to the selection referenced by marks in the same plot)**
- Gains: Less boilerplate for simple dashboards.
- Loses: Fragile -- adding a mark to a plot silently changes selection routing. Makes the spec harder to reason about. Violates clarity-over-magic principle.

### Recommendation

**Option A.** Every vendored spec uses explicit `as: $name`. The parser already handles it. The structural test `dfspec_ac13_crossfilter_structural` validates this exact shape. Duplication is rare and preferable to implicit magic.

---

## Decision 3: What guarantees should cross-filter self-exclusion provide?

### Context

Card scenario 3 states: "the plot's own predicate is excluded from its own filter so it retains context around the selection." The existing `compile_selection` implements this by matching `source_name != self_source` (lower.rs line 83). But "own predicate" needs a precise definition: is it scoped per-plot, per-interactor, or per-data-source? Consider a plot with two interactors both writing to the same crossfilter selection -- should both predicates be excluded from that plot's filter, or only the one the user is currently dragging?

### Options

**A. Self-exclusion is per-view (all predicates originating from the requesting view are excluded)**
- Gains: Simple identity model -- a view either contributed or it didn't. Consistent with Mosaic's crossfilter semantics where each view is one source. The current `compile_selection` signature takes `self_source: &str` which maps naturally to a view/plot identity.
- Loses: If a plot has two interactors contributing to the same crossfilter, brushing one still sees the other's predicate from the same view excluded. This may surprise in edge cases, but those edge cases are pathological (two brushes on one plot).

**B. Self-exclusion is per-interactor (only the predicate from the specific interactor being brushed is excluded)**
- Gains: Fine-grained. A plot with two interactors would see the "other" interactor's predicate in its own filter.
- Loses: Requires tracking which interactor contributed which predicate, adding a contributor ID to the `(source_name, Predicate)` tuple. More complex signature for `compile_selection`. No corpus spec exercises this case.

### Recommendation

**Option A.** The Mosaic model treats each view as a single contributor. The corpus specs (`crossfilter.yaml`, `flights-200k.yaml`) have exactly one interactor per plot. The current `self_source: &str` parameter already encodes per-view identity. Per-interactor exclusion can be added later by refining the source identifier, but building for it now adds complexity without a driving use case.

---

## Decision 4: How should the mark's `filterBy` and the selection's resolution strategy interact when they conflict?

### Context

A mark declares `data: { from: flights, filterBy: $brush }` to subscribe to a selection. The selection declares its resolution strategy (`crossfilter`, `intersect`, `union`, `single`). But `filterBy` is a simple reference -- it does not say *how* to apply the filter. If a spec author puts `filterBy: $brush` on a mark but declares `brush: { select: single }`, the mark gets the last predicate only. This is coherent. But what if a mark references a selection that does not exist in `params`? Or references a value param instead of a selection?

### Options

**A. Strict validation -- filterBy must reference a declared SelectionNode; referencing a missing or non-selection param is an error**
- Gains: Fast failure. Spec authors learn immediately that their wiring is broken. Aligns with the project principle "programmatic checks for validation." The parser already distinguishes `ParamNode::Value` from `ParamNode::Selection`.
- Loses: Rejects specs that might work in Mosaic (where params are more loosely typed). Slightly less forgiving during iterative spec authoring.

**B. Permissive fallback -- filterBy referencing a missing or value param produces Predicate::True (unfiltered)**
- Gains: Graceful degradation. The mark renders with all data, which is at least visible. Useful during development.
- Loses: Silent misconfiguration. A typo in `filterBy: $bruch` silently shows all data instead of failing. Violates "testing over trust."

**C. Warning + fallback -- produce Predicate::True but emit a diagnostic warning**
- Gains: Visible during development, non-fatal in production. Follows the parser's existing pattern of collecting `ParseWarning::UnknownOption` for unrecognised keys.
- Loses: Warnings are easily ignored. The failure mode (showing all data) is the same as "no filter applied," making the bug hard to notice visually.

### Recommendation

**Option A.** The parser already resolves `filterBy: $brush` to a typed `ParamRef` and `params.brush` to a typed `ParamNode::Selection`. Validation at the lowering boundary (when `compile_selection` receives its inputs) should reject references that do not resolve to a `SelectionNode`. This catches typos and wiring errors before they reach the user. The project's principle "programmatic checks for validation" strongly favours strict checking over permissive fallback.

---

## Decision 5: Should resolution strategy be changeable at runtime, or fixed at spec parse time?

### Context

The selection's resolution strategy (`crossfilter`, `intersect`, `union`, `single`) is declared in the spec under `params`. The current `SelectionNode.select` is set at parse time and fed to `compile_selection`. But an input widget (e.g. a dropdown) could theoretically let the user switch strategy at runtime. This would mean the same selection could be "intersect" in one state and "union" in another, changing how predicates combine.

### Options

**A. Fixed at parse time -- resolution strategy is structural, not reactive**
- Gains: `compile_selection` can be a pure function of the spec's static declaration plus runtime predicates. No need for runtime resolution-strategy state. Query plan structural hashes remain stable across interactions (only predicate values change, not plan shape). Matches Mosaic's spec model where `select:` is a declaration, not a parameterised value.
- Loses: Cannot build a "switch resolution mode" UI control.

**B. Reactive -- resolution strategy can be a param reference, resolved at runtime**
- Gains: Full flexibility. An analyst could compare how the same data looks under intersect vs union.
- Loses: Changing resolution strategy changes the *shape* of the compiled predicate (AND vs OR vs last-only), which invalidates the structural hash and forces query re-compilation rather than just re-binding. Adds significant complexity to the coordinator. No Mosaic spec uses this pattern.

### Recommendation

**Option A.** Resolution strategy is a structural property of the selection, not a runtime parameter. The `SelectionResolution` enum in the IR is derived from the AST at lowering time (`impl From<ast::SelectionResolution>`). The structural hash depends on predicate shape, so changing resolution mid-session would invalidate caching. No corpus spec parameterises resolution. If this need emerges, it can be modelled as switching between two distinct selections rather than mutating one.
