# Spec Review

**Date:** 2026-04-24
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-24-gpu-mark-rendering/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 2 |
| 2 — Assumption & failure | content signals (GPU/wgpu, cross-crate boundary) | 1 |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [LOW] Goal claims 60+ FPS but no AC measures or asserts frame timing
**Category:** test-gap
**Pass:** 1
**Description:** The goal states "at 60+ FPS" but no acceptance criterion verifies frame timing. The exit conditions include a 10ms profiling threshold for filing a follow-up card, which is a reasonable escape valve, but the goal's FPS claim remains unverified by the spec itself.
**Evidence:** Goal: "repaints on data or layout changes at 60+ FPS." Exit condition: "If profiling reveals paint() exceeds 10ms for test scenes, file a card for async render extraction before shipping." No AC measures elapsed time.
**Recommendation:** No spec change needed — the exit condition is sufficient for v2 scope. The goal's FPS claim is aspirational rather than contractual. If this matters later, a profiling AC can be added to the app shell card.

### [LOW] AC-06 verification assumes cx.notify() can be tested outside GPUI runtime
**Category:** test-gap
**Pass:** 1
**Description:** AC-06 verification says "verify that entity.update(|state, cx| { state.set_scene(new); cx.notify() }) triggers notification." This requires a GPUI runtime context (Entity, cx). The spec does not clarify whether this is tested via GPUI's test harness or is a compile-time / smoke-test assertion.
**Evidence:** AC-06 verification references Entity and cx, which are GPUI runtime primitives. AC-04 acknowledges this: "Full paint cycle requires GPUI runtime — verify via a GPUI test harness or manual smoke test."
**Recommendation:** No spec change needed. AC-04 already establishes the precedent that GPUI-runtime-dependent testing is either via test harness or manual smoke test. AC-06 follows the same pattern implicitly.

### [LOW] Transition in ChartState but cx.on_next_frame() integration deferred
**Category:** assumption
**Pass:** 2
**Description:** AC-01 includes Transition in ChartState fields. The implementation note says "NavigationState and Transition are included in ChartState fields but their event-routing integration (pan/zoom gestures, transition scheduling via cx.on_next_frame) is deferred to the app shell card." This is clear and well-scoped, but an implementer might wonder whether to include a Transition field that is structurally present but functionally inert.
**Evidence:** AC-01 field list includes Transition. Implementation note at line 150 explicitly defers integration. The Transition struct lives in brightfield-render (not brightfield-ui), so including it in ChartState is a forward-looking field with no wiring in this card.
**Recommendation:** No spec change needed. The implementation note is explicit about the deferral. Including the field now avoids a ChartState API break later when the app shell card wires transitions.

---

## Honest Assessment

This spec is ready for implementation. The v1.1 revision addressed all seven findings from the v1 review cleanly: the ChartState/ChartElement ownership boundary is now explicit (constraint at line 14), GPU test gating is constrained (line 15), VelloRenderer failure behaviour is specified (line 16), and NavigationState/Transition inclusion with deferred integration is documented (implementation note at line 150). The three remaining LOW findings are informational — none require spec changes. The biggest implementation risk remains the synchronous-render-in-paint assumption, but the exit condition (10ms threshold triggers a follow-up card) makes this auditable rather than implicit. The spec is well-scoped for a v2 card that establishes GPUI Element machinery without over-reaching into the app shell or async rendering.
