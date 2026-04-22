# Spec Review — Multi-View Dashboard Composition

**Spec:** `orbit/specs/2026-04-22-multi-view-dashboard-composition/spec.yaml`
**Date:** 2026-04-22
**Reviewer:** spec-review agent (forked context)

---

## Card Scenario Coverage

| Card Scenario | Covering ACs | Status |
|---|---|---|
| Horizontal and vertical composition | ac-04, ac-05, ac-06 | Covered |
| Legends participate as interactors | ac-11 | Covered (verify-only, appropriate) |
| Nested composition creates grid layouts | ac-08 | Covered |
| Spacing controls separate views visually | ac-06, ac-07 | Covered |
| Plots, inputs, and legends compose together | ac-09 | Covered |

All 5 card scenarios are traceable to at least one AC.

## Constraint Review

All 5 constraints are clear, measurable, and non-contradictory:
- Pure function constraint aligns with placement in brightfield-spec
- Box model constraint scopes the layout algorithm appropriately
- No new crate / no new dependencies constraints are enforceable at build time
- Default sizes are concrete and testable

## Acceptance Criteria Review

### Findings

| # | AC | Severity | Finding |
|---|---|---|---|
| 1 | ac-02 | [LOW] | The LayoutNode enum lists Mark and Interactor as variants, but these are rare at the composition level (normally inside plots). The design interview (Q4) confirms unified treatment. No change needed — just noting the intent is deliberate. |
| 2 | ac-03 | [LOW] | The return type is called "LayoutTree" in ac-03 but not formally defined. Implementation should clarify whether this is a type alias for `Option<LayoutNode>` or a wrapper struct. Either works; the spec leaves room for the implementer's judgment. |
| 3 | ac-07 | [LOW] | The spec says "reject other units" but doesn't specify what error type to return. Since this is a pure computation module, returning a default (0.0) or panicking are both options. Implementation note should guide toward returning 0.0 with a warning, not panicking. |
| 4 | ac-08 | [LOW] | Verification says "C.x equals A.width" — this is correct for the described layout (hconcat of two vconcat columns). The AC description is clear. |

### Structural Assessment

- **AC count (13) is proportionate** to the feature scope — layout is a single module with clear boundaries
- **All ACs are ac_type: code** — appropriate for a pure computation module
- **Test prefix (mvdc)** is unique and won't collide with existing prefixes
- **ac-11 is verify-only** — confirms the subscriber graph already handles legend `as:` bindings. This is the right scope for this card (layout, not interaction wiring)
- **ac-13 is a corpus integration test** — catches regressions against real-world specs

### Missing Coverage

No significant gaps. The spec covers all card scenarios, tests both leaf nodes and composition, and includes corpus validation.

## Goal Assessment

The goal is specific, measurable, and achievable within the brightfield-spec crate. "Pixel-accurate coordinates" is the right framing for the box model — every position is deterministic from declared sizes and defaults.

---

**Verdict:** APPROVE
