# Spec Review

**Date:** 2026-04-24
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-24-mosaic-spec-visualisation/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by         | Findings |
|------|----------------------|----------|
| 1 — Structural scan       | always               | 1        |
| 2 — Assumption & failure  | content signal (cross-system boundaries) | 2        |
| 3 — Adversarial           | not triggered         | —        |
```

## Findings

### [LOW] AC-04 verification is partially manual
**Category:** test-gap
**Pass:** 1
**Description:** AC-04's verification includes "A smoke test with a valid spec YAML file opens a window (manual verification)". The binary compilation check (`cargo build -p brightfield-app`) is automatable and sufficient for CI, but the window-open check is inherently manual. This is acceptable for v2 but worth noting — there is no automated assertion that the GPUI window actually renders.
**Evidence:** AC-04 verification field: "A smoke test with a valid spec YAML file opens a window (manual verification)."
**Recommendation:** No change required for v2. Consider a headless render-to-buffer test in a future card to make this automatable.

### [LOW] Assumption: ChartView/paint_image pipeline from card 0013 is stable
**Category:** assumption
**Pass:** 2
**Description:** Implementation note references "The ChartView/ChartElement paint path from card 0013 (CPU readback via paint_image) is the mechanism." AC-04 depends on this pipeline being complete and functional. If card 0013's paint path has issues, AC-04's window rendering will fail even if all other ACs pass.
**Evidence:** Implementation notes line referencing card 0013. Interview Q5 decision references canvas()/img() as the paint mechanism.
**Recommendation:** No spec change needed. The implementer should verify the card 0013 paint path works before wiring AC-04's window creation.

### [LOW] Assumption: brightfield-render's ChartLayout vs brightfield-ui's ChartLayout naming collision
**Category:** assumption
**Pass:** 2
**Description:** The implementation notes explicitly call out that "ChartLayout in brightfield-render is distinct from ChartLayout in brightfield-ui." The spec already surfaces this as a known distinction, and the constraint that brightfield-render must not depend on brightfield-ui prevents accidental conflation. This is well-handled.
**Evidence:** Implementation notes: "ChartLayout in brightfield-render is distinct from ChartLayout in brightfield-ui."
**Recommendation:** None — the spec already documents this. Included here for completeness.

---

## Honest Assessment

This spec is ready for implementation. It is well-scoped, testable, and anchored in concrete types that already exist in the codebase (MarkLower trait, ChannelMap, build_chart_scene, infer_scales). The ACs map cleanly to the existing crate structure. The constraint preventing dependency leakage (render must not depend on engine/sql, ui must not depend on engine/sql) is explicit and protects the architecture. The graceful degradation path (AC-05) is a mature design choice for an integration layer. The biggest risk is the GPUI window rendering path (AC-04) depending on card 0013's paint pipeline, but this is a runtime integration risk, not a spec risk — the spec correctly scopes its verification to compilation plus manual smoke test.
