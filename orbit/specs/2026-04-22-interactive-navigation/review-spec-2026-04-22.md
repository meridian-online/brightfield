# Spec Review

**Date:** 2026-04-22
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-22-interactive-navigation/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 2 |
| 2 — Assumption & failure | content signal: cross-system boundaries (UI -> render -> SQL -> engine) | 3 |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [LOW] AC-03 verification does not assert ticks/grid/marks reflect overridden domain
**Category:** test-gap
**Pass:** 1
**Description:** AC-03 claims "marks, axes, grid, and ticks reflect the overridden domain" but the verification only checks `scale domain_min/domain_max match the override`. Verifying that the returned ScaleSet has correct domain bounds does not prove that ticks, grid lines, and mark positions were computed from those bounds — it proves only that the override was applied to the scale, not that downstream rendering consumed it.
**Evidence:** AC-03 verification text: "assert scale domain_min/domain_max match the override". The actual rendering pipeline calls `compute_ticks`, `render_x_grid`, `render_y_grid`, and mark renderers with the scale — if the scale is correct, rendering correctness follows from existing mark/axis tests. The gap is minor because those downstream functions are already tested in isolation.
**Recommendation:** Accept as-is. The scale-domain assertion is sufficient given that mark/axis rendering from a ScaleSet is already covered by existing tests (gpu_ac08 series). Adding explicit tick-position assertions would be gold-plating.

### [LOW] AC-08 debounce test may be flaky without controlled time
**Category:** test-gap
**Pass:** 1
**Description:** AC-08's verification says "simulate rapid zoom events, assert re-query fires only after the debounce window." Real-time debounce tests are inherently timing-sensitive. The implementation note mentions "GPUI's timer primitives if available; otherwise a simple Instant-based check." A unit test that relies on actual elapsed time may be flaky in CI under load.
**Evidence:** AC-08 verification text. Implementation note 4: "use GPUI's timer primitives if available."
**Recommendation:** The implementer should use a mockable clock or manual timer advance in the debounce test rather than sleeping for real milliseconds. This is an implementation-level concern that does not require a spec change — the AC itself is testable; only the test strategy needs care.

### [LOW] AC-10 integration test scope is underspecified
**Category:** assumption
**Pass:** 2
**Description:** AC-10 verification says "call update_extent with a ViewExtent, assert emitted SQL contains BETWEEN clause." This requires a live DuckDB session with a loaded spec to produce SQL. The spec does not clarify whether this is a true end-to-end test (Engine::load_spec + Session::update_extent) or a unit test of the pass pipeline in isolation. The existing engine tests (in brightfield-engine) use real DuckDB connections, so the integration path is viable.
**Evidence:** AC-10 verification text. Engine test patterns in `crates/brightfield-engine/src/lib.rs` use `Engine::new().load_spec(...)`.
**Recommendation:** No spec change needed. The implementer should follow the existing engine test pattern (real DuckDB connection, emit + execute, inspect SQL). The AC is clear enough to implement against.

### [LOW] Assumption: build_chart_scene rebuild is under 16ms
**Category:** assumption
**Pass:** 2
**Description:** Constraint 5 ("full scene rebuild every frame during active gesture") and evaluation principle 2 ("scene rebuild completes within 16ms") assume that `build_chart_scene` for post-aggregation mark counts is fast enough. The spec explicitly acknowledges this in D6 and provides an upgrade path (cache RecordBatch, rebuild only mark layer). The assumption is reasonable for the stated workload (hundreds to low thousands of marks) and Vello's design point.
**Evidence:** Interview D6: "For analytical dashboards (hundreds to low thousands of marks after pre-aggregation), full rebuild at 60Hz is well within budget." Constraint 5 text.
**Recommendation:** No action. The spec correctly treats this as an assumption with a known escape hatch. If profiling reveals a problem, the upgrade path is documented.

### [LOW] NavigationFilterPass uses BETWEEN but spec says Expr-based predicates
**Category:** assumption
**Pass:** 2
**Description:** AC-09 says the pass "inserts Filter node with BETWEEN predicate." The existing IR uses `Predicate::Expr(String)` for raw SQL expressions. The BETWEEN clause will be emitted as a string like `"col BETWEEN min AND max"` or as `And([Expr("col >= min"), Expr("col <= max")])`. The interview's D5 section shows the latter form. These are semantically equivalent, but the AC text says "BETWEEN predicate" while the interview says `Expr("col >= min")` / `Expr("col <= max")`. Minor inconsistency.
**Evidence:** AC-09: "BETWEEN predicate for each navigable axis." Interview D5: `Filter { predicate: And([Expr("col >= min"), Expr("col <= max")]) }`. Implementation note 5 in the spec matches the interview form.
**Recommendation:** No spec change. The interview and implementation notes clarify the intended form (AND of two Expr predicates). The AC's "BETWEEN predicate" is shorthand, not a literal SQL keyword requirement.

---

## Honest Assessment

This spec is ready for implementation. The 11 acceptance criteria are specific, testable, and well-scoped to the goal. The decisions are thoroughly documented with clear rationale, and the interview resolves all five open questions. The cross-crate data flow (ViewExtent from UI through render to engine) is the most complex aspect, but the spec handles it cleanly by placing ViewExtent in brightfield-render where both UI and engine can depend on it. The constraint that Scale stays immutable is a good architectural guardrail. The biggest implementation risk is the debounce timer testability and ensuring the NavigationFilterPass integrates cleanly with the existing pass pipeline — but both are well-understood problems with existing patterns in the codebase. No structural concerns were found that would warrant Pass 3.
