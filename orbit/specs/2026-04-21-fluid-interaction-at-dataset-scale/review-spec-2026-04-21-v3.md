# Spec Review

**Date:** 2026-04-21
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-21-fluid-interaction-at-dataset-scale/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 0 |
| 2 — Assumption & failure | not triggered | — |
| 3 — Adversarial | not triggered | — |
```

## Findings

None.

---

## Prior Review Findings — Disposition

This is the third review cycle. All MEDIUM findings from v1 and v2 have been resolved in the current spec:

- **AC-08 signature divergence from D6** (v1, v2): Now carries explicit inline rationale — mark_index enables per-mark emission, param_values enables D4 hybrid binding, SupportReport dropped because preflight is a separate upstream phase, name changes to coexist with card 0004's emit_sources.
- **AC-09 UnsupportedMark vs InvariantViolation overlap** (v1): AC-08 and AC-03 now consistently reference UnsupportedMark for unimplemented marks. InvariantViolation is explicitly reserved for truly unexpected states.
- **AC-04 render_query binding threading** (v1): Signature updated to `fn render_query(plan: &QueryPlan, bindings: &mut Vec<Binding>) -> String`.
- **AC-06 EmittedQuery omits dependencies field** (v2): Inline note explains plan_hash subsumes QueryDeps for v1; QueryDeps deferred to result-cache coordinator card.
- **ExpressionNode invariant defensive check** (v2): AC-10 now specifies `Returns EmitError::InvariantViolation if spans.len() != params.len() + 1`.
- **AC-10 Interpolated mode param values** (v1): AC-10 now defines `BindingMode::Interpolated { values: &ParamValues }` and AC-08 threads `param_values: Option<&ParamValues>` through the public API.
- **AC-01 SelectionResolution relationship** (v2): Now specifies `impl From<ast::SelectionResolution>` and explicitly states the IR type is independent to avoid AST serde coupling.
- **AC-03 Result wrapper vs interview** (v1): Inline note acknowledges the intentional improvement over the interview signature.

## Gate-AC Verification (deterministic)

| Gate AC | Verification field | Non-empty | Not placeholder | >= 20 chars | Result |
|---------|--------------------|-----------|-----------------|-------------|--------|
| ac-15   | "CI gate: cargo test --workspace exits 0." | yes | yes | 42 chars | PASS |

## Content Signals Checked

- **Cross-system boundary (card 0004 shared crate):** Rally enforces serial ordering (0004 before 0003). Constraint 1 ("extend, do not restructure") is clear. Flagged in both prior reviews and accepted as managed risk.

## Codebase Verification

- `crates/brightfield-sql/src/` exists with `emit.rs`, `error.rs`, `lib.rs`, `render.rs`, `source.rs` — matches constraint 1.
- `ast::SelectionResolution` variants (Crossfilter, Intersect, Single, Union) at `vocab.rs:244` match AC-01.
- `ExpressionNode { spans: Vec<String>, params: Vec<ParamRef> }` at `ast.rs:355` matches AC-10's invariant assumption.
- `lib.rs` already references card 0003 extension in its doc comment (line 9).

## Honest Assessment

This spec is ready for implementation. The two prior review cycles surfaced real ambiguities — signature contradictions, error variant overlap, missing binding threading, unexplained departures from interview decisions — and all have been resolved with explicit inline rationale. The 16 ACs are internally consistent, every verification method is concrete enough to write a test for, and the codebase types the spec references exist as described. The biggest remaining risk is the cross-card boundary with card 0004, but the rally's serial ordering and the "extend, do not restructure" constraint manage this adequately. No findings to report.
