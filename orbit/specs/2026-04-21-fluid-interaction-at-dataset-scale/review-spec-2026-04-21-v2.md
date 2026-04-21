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
| 2 — Assumption & failure | content signals (cross-system boundary: card 0004 shared crate; shared config: ExpressionNode invariant) | 3 |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [LOW] AC-01 SelectionResolution mirrors ast but lives in ir.rs — relationship unclear
**Category:** missing-requirement
**Pass:** 1
**Description:** AC-01 says "SelectionResolution enum mirrors ast::SelectionResolution: Crossfilter, Intersect, Union, Single" but does not specify whether this is a re-export, a newtype wrapper, or a fully independent enum. If independent, there is no AC covering the mapping between `ast::SelectionResolution` and `ir::SelectionResolution`.
**Evidence:** AC-01 description, line "SelectionResolution enum mirrors ast::SelectionResolution". The AST type lives in `crates/brightfield-spec/src/vocab.rs:244` and is already public.
**Recommendation:** Clarify: either re-export the AST type (and state so), or add a `From<ast::SelectionResolution>` conversion requirement to AC-01 or AC-05.

### [MEDIUM] AC-08 emit_query signature diverges from interview D6, and spec is internally inconsistent about it
**Category:** constraint-conflict
**Pass:** 1
**Description:** Interview D6 defines the public API as `fn emit(spec: &Spec, preflight: &SupportReport) -> Result<EmittedQuery, EmitError>`, taking a `SupportReport` argument. AC-08 instead defines `fn emit_query(spec: &Spec, mark_index: usize, param_values: Option<&ParamValues>) -> Result<EmittedQuery, EmitError>` — no `SupportReport`, different name, different parameters. The spec acknowledges the D6 departure for `Result` wrapping (AC-03 note) but does not acknowledge or justify the removal of `SupportReport` from the signature, the name change from `emit` to `emit_query`, or the addition of `mark_index` and `param_values` parameters.
**Evidence:** AC-08 description vs interview D6. AC-08 also says "emitter trusts preflight has already rejected; UnsupportedMark is the defence-in-depth variant" — but if there is no `SupportReport` parameter, what enforces the "preflight has already rejected" contract? The trust model works but the departure from D6 should be explicitly noted as a deliberate design refinement, not silently changed.
**Recommendation:** Add a brief note to AC-08 acknowledging the D6 divergence and the rationale (mark_index enables per-mark emission, param_values enables hybrid binding mode from D4, SupportReport dropped because preflight is a separate phase). This is important for traceability.

### [MEDIUM] AC-06 EmittedQuery omits `dependencies` field present in D4
**Category:** scope vs goal
**Pass:** 1
**Description:** Interview D4 specifies the emitter output as `EmittedQuery { sql, bindings, dependencies: QueryDeps }`. AC-06 defines `EmittedQuery { sql, bindings, plan_hash }`. The `dependencies` / `QueryDeps` field is gone and `plan_hash` is added. The interview implementation surface (line 79) also lists `dependencies` as part of `EmittedQuery`. Neither the spec nor any AC explains the removal of `QueryDeps` or why `plan_hash` replaces `dependencies`.
**Evidence:** Interview D4 output shape vs AC-06 description. The `dependencies` field matters for incremental re-query (D5) — understanding which params a query depends on determines when re-emission is needed.
**Recommendation:** Either (a) add `dependencies` back alongside `plan_hash`, or (b) add an explicit note that `plan_hash` subsumes the `dependencies` use case for v1 and `QueryDeps` is deferred. Without this, an implementer may re-introduce `QueryDeps` thinking it was accidentally omitted.

### [LOW] AC-04 render_query mentions "expression_to_sql for param-bearing predicates" but AC-10 is the actual definition
**Category:** missing-requirement
**Pass:** 1
**Description:** AC-04 references `expression_to_sql` as if it exists, but it is defined in AC-10. The dependency direction is implicit. AC-04's verification says "Binding vector is populated for plans containing Predicate::Param nodes" — but the Binding population logic is in `expression_to_sql` (AC-10), not `render_query` (AC-04).
**Evidence:** AC-04 description vs AC-10 description.
**Recommendation:** No change required, but note for the implementer: AC-10 should be implemented before AC-04's param-bearing predicate tests can pass.

### [LOW] AC-16 test count threshold is fragile
**Category:** test-gap
**Pass:** 1
**Description:** AC-16 specifies minimum test counts per module (>=5 in ir.rs, >=5 in lower.rs, etc., totalling >=23). These thresholds are tightly coupled to the current AC set. If any AC is descoped or simplified during implementation, the thresholds become misleading.
**Evidence:** AC-16 verification: "cargo test -p brightfield-sql shows >=23 new tests passing".
**Recommendation:** Keep the thresholds as a guide but consider the exit condition met if every AC's verification is satisfied, even if a module has one fewer test than the threshold.

### [MEDIUM] ExpressionNode.spans/params invariant is load-bearing but has no defensive AC
**Category:** assumption
**Pass:** 2
**Description:** Constraint 8 states "ExpressionNode.spans/params interleaving invariant (spans.len() == params.len() + 1) is load-bearing for SQL rendering". AC-10's `expression_to_sql` depends on this invariant. However, no AC requires `expression_to_sql` to validate the invariant at runtime or produce a clear error if it is violated. The invariant is upheld by the parser (`crates/brightfield-spec/src/expr.rs:29`), but if a hand-constructed `ExpressionNode` breaks it, `expression_to_sql` would panic or produce garbage SQL.
**Evidence:** Constraint 8, AC-10 description, `expr.rs:25` ("The result's invariant holds: spans.len() == params.len() + 1").
**Recommendation:** Add a debug_assert or explicit check in `expression_to_sql` that returns `EmitError::InvariantViolation` if `spans.len() != params.len() + 1`. This can be a sub-bullet of AC-10 rather than a separate AC.

### [LOW] Card 0004 serial ordering — merge conflict risk on render.rs
**Category:** failure-mode
**Pass:** 2
**Description:** The spec adds `render_query` to `render.rs` which currently contains only `canonicalise_ddl`. Constraint 1 says "extend, do not restructure" and the interview notes serial ordering with card 0004. If card 0004 is not fully merged before this card begins, both cards mutate `render.rs` and `lib.rs`.
**Evidence:** Constraint 1, interview cross-card touchpoints section, current `render.rs` content.
**Recommendation:** No spec change needed — the rally's serial ordering handles this. Flagging for awareness only: implementation must confirm card 0004 is merged before starting.

### [LOW] Predicate::Expr(String) is a raw SQL string escape hatch — no sanitisation AC
**Category:** assumption
**Pass:** 2
**Description:** AC-02 defines `Predicate::Expr(String)` as a variant that carries a raw SQL string. If this is used for user-provided expressions, it is an injection vector. The spec's pure-function constraint means no DB connection, so exploitation requires a downstream consumer to execute unsanitised SQL.
**Evidence:** AC-02 description: "Expr(String)".
**Recommendation:** No change needed for v1 — the emitter is a pure function and the `Expr` variant carries AST-derived content, not user input. But document the assumption that `Expr` content originates from the parsed AST (which is trusted). A one-line doc comment on the variant suffices.

---

## Honest Assessment

This spec is thorough and well-structured — 16 ACs covering the full IR-to-SQL pipeline with clear verification methods. The biggest risk is the silent divergence from interview decisions D4 and D6 in the public API shape (AC-06 dropping `QueryDeps`, AC-08 dropping `SupportReport`). These are likely deliberate refinements that improve the design, but they need to be explicitly called out so an implementer does not waste time reconciling the interview with the spec. The ExpressionNode invariant gap (no defensive check in `expression_to_sql`) is a minor correctness risk that is cheap to close. With those three MEDIUM findings addressed — even just as clarifying notes — this spec is ready for implementation.
