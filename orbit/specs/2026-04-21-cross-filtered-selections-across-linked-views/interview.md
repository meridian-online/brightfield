# Design: Cross-Filtered Selections Across Linked Views

**Date:** 2026-04-21
**Interviewer:** Nightingale
**Card:** orbit/cards/0006-cross-filtered-selections-across-linked-views.yaml

---

## Context

Card: *Cross-filtered selections across linked views* -- 3 scenarios, goal: enable brush/click in one view to filter linked views via first-class selection predicates.
Prior specs: 0 -- this is the first spec addressing this card.
Gap: Full capability -- selection resolution, cross-filter self-exclusion, and filterBy wiring are all unimplemented at the spec level.

## Q&A

### Q1: What should the user see before any interaction?

**Q:** When a dashboard loads and no one has brushed or clicked anything yet, what should the linked views show -- the full dataset (everything visible, ready to narrow down), or an empty state signalling that interaction is required?

**A:** Full dataset, unfiltered. The dashboard should be immediately useful on load. An empty state looks broken. This matches Mosaic's behaviour and every vendored crossfilter spec assumes populated plots before any brush. Predicate::True for empty selections.

### Q2: How explicitly should interactors declare their selection binding?

**Q:** An interactor like `intervalX` needs to route its predicate to a named selection. Should this binding always be an explicit `as: $selection` declaration, or should there be convenience shortcuts -- writing to multiple selections from one interactor, or implicit binding based on which marks share the same plot?

**A:** Explicit single `as: $selection` binding only. One interactor writes to exactly one selection. It is simple, declarative, and auditable. Implicit binding by co-location is fragile and violates clarity-over-magic. If an interactor needs to drive two selections, duplicate it in the spec -- that is rare and preferable to hidden wiring.

### Q3: When a view filters itself, what should be excluded?

**Q:** A plot that contributes a predicate to a crossfilter selection also subscribes to that selection. To preserve context around the brushed region, the plot should not filter itself. But if a plot has multiple interactors writing to the same selection, should all of that plot's predicates be excluded from its own filter, or only the one the user is currently dragging?

**A:** Per-view self-exclusion. All predicates originating from a view are excluded from that view's own filter. The Mosaic model treats each view as a single contributor, and every corpus spec has exactly one interactor per plot. Per-interactor exclusion adds complexity without a driving use case -- if it is needed later, the source identifier can be refined.

### Q4: What should happen when a mark references a broken or mistyped selection?

**Q:** A mark declares `filterBy: $brush` to subscribe to a selection. What should happen if that reference points to a param that does not exist, or to a value param instead of a selection -- should the system silently show all data, warn and show all data, or reject the spec outright?

**A:** Strict validation. Reject references to missing or non-selection params as an error. Fast failure catches typos and wiring mistakes before they reach the user. Silent fallback to unfiltered data hides bugs. The parser already distinguishes value params from selection params, so validation is straightforward. This aligns with the project principle of programmatic checks for validation.

### Q5: Can an analyst change the resolution strategy at runtime?

**Q:** A selection declares its resolution strategy (intersect, union, crossfilter, single) in the spec. Should this be a fixed structural property, or should a UI control be able to switch strategies at runtime -- for instance, letting an analyst compare intersect versus union on the same data?

**A:** Fixed at parse time. Resolution strategy is structural, not reactive. Changing it at runtime would invalidate the structural hash and force query re-compilation rather than just re-binding predicates. No Mosaic spec parameterises resolution. If this need emerges, it can be modelled as switching between two distinct selections rather than mutating one.

---

## Summary

### Goal

Enable brush and click interactions in one view to filter linked views via first-class selection predicates, with cross-filter self-exclusion so each view retains context around its own contribution.

### Constraints

- Mosaic spec compatibility -- all vendored specs must continue to parse and behave correctly
- Resolution strategy is a structural property, fixed at parse time
- One interactor binds to exactly one selection via explicit `as:` declaration
- Self-exclusion is scoped per-view, not per-interactor

### Success Criteria

- A brush in one plot filters all linked plots via the declared selection
- Multiple interactors contributing to the same selection resolve according to the declared strategy (intersect, union, crossfilter, single)
- A view's own predicates are excluded from its own filter
- `filterBy` referencing a missing or non-selection param produces a validation error
- Empty selections (no predicates contributed) resolve to Predicate::True (show all rows)

### Decisions Surfaced

- **Empty selection semantics:** chose Predicate::True (unfiltered) over Predicate::False (empty result) because dashboards should be immediately useful on load, matching Mosaic convention
- **Interactor-to-selection binding:** chose explicit single `as: $selection` over multi-binding or implicit co-location because clarity and auditability outweigh convenience for a rare case
- **Self-exclusion scope:** chose per-view over per-interactor because the Mosaic model treats views as contributors and no corpus spec exercises multi-interactor-per-plot
- **filterBy validation:** chose strict rejection over permissive fallback because silent misconfiguration hides bugs and violates programmatic-checks-for-validation principle
- **Resolution strategy mutability:** chose fixed-at-parse-time over reactive because structural hash stability matters and no use case drives runtime switching

### Implementation Notes

- `compile_selection` in `lower.rs` already returns `Predicate::True` for empty predicates -- D1 confirms this is correct behaviour, not a placeholder
- The `self_source: &str` parameter in `compile_selection` (lower.rs line 83) maps directly to per-view identity for D3 self-exclusion
- Parser already lifts `as: $brush` to `ValueOrParamRef::Param(ParamRef("brush"))` -- verified in `crossfilter.rs` test line 50
- `ParamNode::Value` vs `ParamNode::Selection` distinction is already in the AST -- D4 validation should be added at the lowering boundary
- `SelectionResolution` enum has `From<ast::SelectionResolution>` impl -- resolution is already derived at lowering time, confirming D5

### Open Questions

- None -- all five decision points resolved.
