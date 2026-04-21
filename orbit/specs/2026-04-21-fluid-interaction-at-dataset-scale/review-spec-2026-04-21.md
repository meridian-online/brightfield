# Spec Review

**Date:** 2026-04-21
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-21-fluid-interaction-at-dataset-scale/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 5 |
| 2 — Assumption & failure | content signals (cross-system boundaries, shared config between cards 0003/0004) + Pass 1 findings | 3 |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [MEDIUM] AC-08 signature diverges from D6 decision
**Category:** constraint-conflict
**Pass:** 1
**Description:** Decision D6 in the interview specifies: `fn emit(spec: &Spec, preflight: &SupportReport) -> Result<EmittedQuery, EmitError>`. AC-08 specifies: `fn emit_query(spec: &Spec, mark_index: usize) -> Result<EmittedQuery, EmitError>`. The preflight `SupportReport` argument from D6 is dropped entirely, replaced by a `mark_index: usize` parameter. The spec says "emitter trusts preflight has already rejected" but removes the mechanism (taking `SupportReport` as argument) that D6 uses to "assert its guarantee". The interview implementation surface section (line 79) also shows the D6 signature.
**Evidence:** Interview D6: "API: `fn emit(spec: &Spec, preflight: &SupportReport) -> Result<EmittedQuery, EmitError>`. Takes preflight as argument to assert its guarantee." AC-08: `fn emit_query(spec: &Spec, mark_index: usize)`.
**Recommendation:** Reconcile the function signature. Either (a) keep `SupportReport` as a parameter per D6 and add `mark_index` to select which mark, or (b) explicitly decide to drop the `SupportReport` argument and update D6's status. The current state is a silent contradiction.

### [MEDIUM] AC-03 MarkLower trait returns Result but interview says QueryPlan
**Category:** constraint-conflict
**Pass:** 1
**Description:** AC-03 defines: `fn lower(&self, mark: &Mark, ctx: &LowerCtx) -> Result<QueryPlan, EmitError>`. The interview implementation surface (line 77) defines: `fn lower(&self, mark: &Mark, ctx: &LowerCtx) -> QueryPlan`. The spec version (returning Result) is the better design, but the inconsistency should be acknowledged so implementers have a single source of truth.
**Evidence:** Interview line 77 vs AC-03.
**Recommendation:** No code change needed (the spec's Result return is correct), but note the deviation from the interview explicitly so there is no ambiguity during implementation.

### [LOW] AC-06 mentions plan_hash field but AC-14 defines hash_structural method
**Category:** assumption
**Pass:** 1
**Description:** AC-06 says `EmittedQuery { sql, bindings, plan_hash: u64 }` — a stored field. AC-14 defines `QueryPlan::hash_structural(&self) -> u64` — a computed method. The spec does not clarify whether `plan_hash` in `EmittedQuery` is populated by calling `hash_structural()` on the plan before it is dropped, or computed independently. This is implied but not stated.
**Evidence:** AC-06 verification: "Plan hash excludes bound param values". AC-14: "Hashes the plan structure excluding bound parameter values".
**Recommendation:** Add a single sentence to AC-08 (the orchestration AC) stating that `plan_hash` is populated by calling `plan.hash_structural()` after the pass pipeline but before rendering.

### [MEDIUM] AC-09 UnsupportedMark vs InvariantViolation overlap
**Category:** constraint-conflict
**Pass:** 1
**Description:** AC-09 introduces `UnsupportedMark { kind: String }` for "marks that reach the emitter despite being Unimplemented (defence in depth)". AC-08 says "Unimplemented marks return EmitError::InvariantViolation". AC-03 says the default impl "returns EmitError::InvariantViolation for unimplemented marks". The spec now has two error variants both intended for the same failure mode (an unimplemented mark reaching the emitter). Which one is returned when?
**Evidence:** AC-03: "Default impl returns EmitError::InvariantViolation". AC-08: "Unimplemented marks return EmitError::InvariantViolation". AC-09: "UnsupportedMark { kind } for marks that reach the emitter despite being Unimplemented".
**Recommendation:** Clarify the distinction. Likely: AC-08/AC-03 should return `UnsupportedMark` (the new, more descriptive variant) and `InvariantViolation` is reserved for truly unexpected states. Update AC-03 and AC-08 to reference `UnsupportedMark` instead, or remove `UnsupportedMark` and keep `InvariantViolation` with a descriptive `detail` string.

### [LOW] Content signal: cross-system boundary with card 0004
**Category:** content-signal
**Pass:** 1
**Description:** The spec extends brightfield-sql (owned by card 0004) and adds modules alongside card 0004's existing code. The constraint "extend, do not restructure" is stated but the serial ordering guarantee (card 0004 lands first) is only documented in the interview, not in the spec's constraints. If implementation order changes, both cards mutate `lib.rs`, `render.rs`, and `error.rs`.
**Evidence:** Interview cross-card touchpoints: "Both mutate crates/brightfield-conformance/src/layer.rs SqlEquivalenceCheck — serial ordering". Spec constraint 1: "extend, do not restructure". Rally already enforces serial order (0004 before 0003).
**Recommendation:** No action required if the rally enforces order. Noting for completeness.

---

### [MEDIUM] AC-04 render_query takes &QueryPlan but must also produce bindings
**Category:** assumption
**Pass:** 2
**Description:** AC-04 defines `fn render_query(plan: &QueryPlan) -> String` which returns only a SQL string. But AC-06 requires `EmittedQuery { sql, bindings, plan_hash }` with bindings populated. AC-10 defines `expression_to_sql` which takes `&mut Vec<Binding>` to accumulate bindings during rendering. The render_query signature in AC-04 has no way to thread bindings out. Either render_query must return `(String, Vec<Binding>)`, or binding collection happens at a different layer (AC-08's orchestration). The spec is silent on how bindings flow from rendering to EmittedQuery assembly.
**Evidence:** AC-04: `fn render_query(plan: &QueryPlan) -> String`. AC-10: `fn expression_to_sql(expr: &ExpressionNode, bindings: &mut Vec<Binding>, mode: BindingMode) -> String`. AC-06: `EmittedQuery { sql, bindings, plan_hash }`.
**Recommendation:** Either (a) change AC-04's signature to `fn render_query(plan: &QueryPlan, bindings: &mut Vec<Binding>) -> String`, or (b) add a sentence to AC-08 explaining that the orchestrator collects bindings separately from rendering. The current spec leaves the implementer to guess.

### [MEDIUM] AC-10 ExpressionNode rendering assumes literal values available at emit time
**Category:** assumption
**Pass:** 2
**Description:** AC-10 specifies an `Interpolated` binding mode that "emits literal values" (e.g. `x > 42 AND x < 100`). But the spec's design (D4, D5) is built around the emitter being a pure compile-time function that does not have runtime parameter values. The interview says scalar params bind as `?` and only at `execute()` time do values flow in. If the emitter has no access to current parameter values, what does `Interpolated` mode render? This matters for selection params (D4: "trigger re-emission of the WHERE clause only").
**Evidence:** AC-10 verification: "Same expression in Interpolated mode renders `x > 42 AND x < 100` with literal values." D4: "Scalar params bind as ? — slider drag dispatches execute(stmt, &[latest_values])."
**Recommendation:** Clarify where literal values come from in Interpolated mode. Likely the `LowerCtx` or a separate `ParamValues` map must be threaded through. Add a parameter to the function signature or document that Interpolated mode is only used during selection re-emission when current values are known.

### [LOW] AC-11 conform.rs location ambiguity
**Category:** assumption
**Pass:** 2
**Description:** AC-11 says "a new conform.rs module in brightfield-sql". AC-16 counts conform.rs tests toward the brightfield-sql test total. But the interview implementation surface says structural SQL diff lives in brightfield-conformance (line 84-86: "Modified: crates/brightfield-conformance/src/layer.rs — SqlEquivalenceCheck flips from Pending to real pass/fail backed by sqlparser-rs"). The conformance crate already owns SQL comparison. Adding conform.rs to brightfield-sql means two crates both do SQL structural comparison.
**Evidence:** AC-11: "new conform.rs module in brightfield-sql". Interview line 84: conformance check in brightfield-conformance. Spec constraint 14: "sqlparser-rs is the only new external dependency (add to brightfield-sql and brightfield-conformance)".
**Recommendation:** This is likely intentional — brightfield-sql's conform.rs provides low-level parse+compare utilities, brightfield-conformance calls them for layer-2 checks. But state this explicitly to avoid the implementer questioning whether conform.rs belongs in brightfield-sql or brightfield-conformance.

---

## Honest Assessment

This spec is thorough and well-structured for a complex IR + rendering infrastructure. The 16 ACs cover the right surface area and the constraints are well-chosen. The biggest risk is the **signature and error-variant contradictions** (AC-08 vs D6, AC-09 vs AC-03/AC-08, AC-04's return type vs binding flow). These are not design flaws — they read like a spec that was written fast after a strong interview and did not get a reconciliation pass against the decisions. An implementer following ACs literally will hit ambiguity on function signatures within the first hour. A 30-minute reconciliation pass addressing the four MEDIUM findings (especially the emit_query signature and the UnsupportedMark/InvariantViolation overlap) would make this spec unambiguous enough to implement without guesswork.
