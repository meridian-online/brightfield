# Spec Review

**Date:** 2026-04-24
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-24-reactive-parameters-with-input-widgets/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 2 |
| 2 — Assumption & failure | Pass 1 findings (MEDIUM) | 1 |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [MEDIUM] Selection params have no SpecValue default for param_state initialisation
**Category:** missing-requirement
**Pass:** 1
**Description:** AC-01 states "Initial values are populated from the spec's declared param defaults at load time" and the implementation note says "Use SpecValue::from(param_node.default) or similar." However, `ParamNode` is an enum with two variants: `Value(SpecValue)` (which has a value to populate) and `Selection(SelectionNode)` (which has no SpecValue equivalent). The spec does not address how Selection params are represented in `param_state`, or whether they should be excluded from it entirely.
**Evidence:** `ParamNode` definition in `crates/brightfield-spec/src/ast.rs:169-175`. `Selection(SelectionNode)` contains a `SelectionResolution` and options, but no default SpecValue. AC-01's verification only tests "params that have default values" which implicitly avoids the Selection case but doesn't acknowledge it.
**Recommendation:** Add a sentence to AC-01 or implementation_notes clarifying that Selection params are excluded from `param_state` (since they are driven by interactor events, not literal defaults), or define a sentinel SpecValue for uninitialised selections. The former is simpler and matches the interview's scope (direct param propagation only, selections are deferred).

### [LOW] AC-09 verification is a shell heuristic, not a test assertion
**Category:** test-gap
**Pass:** 1
**Description:** AC-09's verification is `grep -r 'fn rpw2_' crates/ | wc -l >= 8`. This is a shell command, not a test that runs in `cargo test`. It could pass if test functions exist but are `#[ignore]`d, or if non-test functions happen to start with `rpw2_`.
**Evidence:** AC-09 verification field.
**Recommendation:** Accept as-is. This is a meta-check on test count. The real verification is that all other ACs have proper test coverage. The implementer will naturally create the tests to satisfy AC-01 through AC-07.

### [LOW] update_param builds single-param ParamValues; propagate_param must build full param_state — behavioural difference could surprise callers
**Category:** assumption
**Pass:** 2
**Description:** The existing `update_param` creates a `ParamValues` containing only the single changed param (`let mut param_values = ParamValues::new(); param_values.insert(name, value)`). The spec correctly calls out that `propagate_param` should pass "the full param_state (not just the changed param)" so multi-param queries see all current values. However, the spec also says `propagate_param` is "a thin orchestration over update_param" (implementation note 1), which contradicts the need to build a different ParamValues. The implementer needs to either: (a) not delegate to `update_param` for the query dispatch, or (b) modify `update_param` to accept full param state (which would change its internal behaviour, potentially conflicting with the constraint that existing APIs are "unchanged in behaviour").
**Evidence:** `crates/brightfield-engine/src/lib.rs:185-186` — `update_param` builds single-param ParamValues. Spec constraint: "Existing Session API unchanged in signature and behaviour." Implementation note 1: "propagate_param is a thin orchestration over update_param."
**Recommendation:** Clarify implementation note 1: `propagate_param` should replicate the subscriber-lookup and mark-dispatch logic from `update_param` (or extract shared helpers) rather than literally calling `update_param`. The note already hedges with "may be a thin wrapper initially" so the intent is clear enough, but the tension between "thin wrapper over update_param" and "passes full param_state" should be acknowledged. This is implementer-navigable as-is; no spec change required.

---

## Honest Assessment

This spec is well-scoped and ready for implementation. The ACs are specific, testable, and aligned with the stated goal. The two substantive findings are both navigable: the Selection param gap is real but the implementer will naturally exclude Selection variants from `param_state` since they have no SpecValue; the update_param delegation tension is noted in the implementation notes and the implementer has clear guidance on the desired behaviour (full param_state). The decision to defer chained DAG propagation is sound — it keeps this spec focused on a single well-defined hop. The biggest risk is the Selection param edge case causing a `match` arm panic if the implementer forgets to handle it, but that would surface immediately in existing tests that use Selection params.
