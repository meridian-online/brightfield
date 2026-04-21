# Spec Review

**Date:** 2026-04-21
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-21-cross-filtered-selections-across-linked-views/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by         | Findings |
|------|----------------------|----------|
| 1 — Structural scan       | always   | 1        |
| 2 — Assumption & failure  | content signal (cross-system boundary: card 0005 integration) | 1 |
| 3 — Adversarial           | not triggered | —      |
```

## Findings

### [LOW] AC-09 integration surface with card 0005's SpecAnalysis is implicit
**Category:** assumption
**Pass:** 1
**Description:** AC-09 states that `SpecAnalysis` gains `selection_subscribers` and `interactor_bindings` fields, and that new validation runs "alongside existing param analysis." The existing `SpecAnalysis` struct (analysis.rs:425) already has four fields (`subscriber_graph`, `dependency_edges`, `topological_order`, `warnings`). The spec assumes adding two new fields is non-breaking but does not explicitly state whether downstream consumers of `SpecAnalysis` (e.g. the SQL emitter, conformance tests) need updating. Since `SpecAnalysis` is a plain struct (not `#[non_exhaustive]`), adding fields will break any exhaustive destructuring patterns.
**Evidence:** `SpecAnalysis` at analysis.rs:425-434 is a plain struct. The spec's implementation_notes bullet 6 says "Existing SpecAnalysis struct gains two new fields" but no AC covers downstream compilation.
**Recommendation:** This is low-risk because Rust's compiler will catch any broken destructuring at build time, and the exit condition "cargo test passes workspace-wide" covers it. No spec change needed; noting for implementer awareness.

### [LOW] Ambiguity in `build_selection_subscriber_graph` vs `build_subscriber_graph` naming
**Category:** assumption
**Pass:** 2
**Description:** Implementation note 5 explicitly distinguishes `build_selection_subscriber_graph` (new, card 0006) from `build_subscriber_graph` (existing, card 0005). Both return maps of param-name to component-path sets but with different scope. The spec is clear that the new function tracks "only selection-consuming filterBy refs, not all param refs." However, AC-07 and AC-08 name the functions (`build_selection_subscriber_graph`, `build_interactor_bindings`) without specifying whether they are standalone public functions or private helpers called within `analyse_spec`. This is fine for implementation flexibility but worth noting: the implementer should decide visibility and document it.
**Evidence:** AC-07 verification: "crossfilter spec with 2 plots both filterBy: $brush. Graph has 'brush' -> [path1, path2]." AC-09 verification: "full crossfilter-style spec through analyse_spec." The integration path is clear even if function visibility is unspecified.
**Recommendation:** No change needed. The implementer has enough latitude, and the AC-09 integration test will catch any wiring issues.

---

## Honest Assessment

This spec is well-constructed and ready for implementation. The goal is tightly scoped to AST-level validation and graph construction -- it does not attempt runtime behaviour, which keeps risk low. Every AC is testable with specific inputs and expected outputs. The interview decisions (D1-D5) are crisply recorded and the spec references them as constraints rather than re-deriving them. The only content signal that triggered Pass 2 was the cross-card integration with card 0005's `SpecAnalysis`, but the integration surface is small (two new struct fields) and Rust's type system provides a compile-time safety net. The vendored corpus regression gate (AC-10) is the strongest risk mitigator -- it prevents false positives structurally. The biggest real-world risk is not in the spec but in sequencing: if card 0005 lands changes to `SpecAnalysis` concurrently, a merge conflict is possible, but that is an operational concern outside the spec's scope.
