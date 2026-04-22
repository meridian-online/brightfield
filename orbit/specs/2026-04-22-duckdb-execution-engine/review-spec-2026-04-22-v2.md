# Spec Review

**Date:** 2026-04-22
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-22-duckdb-execution-engine/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 1 |
| 2 — Assumption & failure | cross-system boundary content signal (consumes brightfield-spec + brightfield-sql APIs) | 1 |
| 3 — Adversarial | not triggered | — |
```

## Prior Review Resolution

The v1 review (review-spec-2026-04-22.md) raised 6 findings. Checking each against the current spec:

1. **[MEDIUM] Subscriber graph maps to component paths, not mark indices** -- RESOLVED. AC-06 now specifies "the engine builds a mark-index map during load_spec by walking the component tree and collecting mark positions." Implementation note 6 describes the MarkIndexMap strategy explicitly.

2. **[LOW] AC-06 return type loses error granularity on partial failure** -- RESOLVED. AC-06 now returns `Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>` with explicit partial failure semantics mirroring execute_all.

3. **[LOW] AC-07 verification does not specify how to observe cache hits** -- RESOLVED. AC-07 now specifies `#[cfg(test)] method session.cache_len()` to assert cache size stays constant across scalar rebinds. Implementation note 7 confirms this.

4. **[MEDIUM] Subscriber graph includes non-mark components** -- RESOLVED. AC-06 now states "Only mark components are dispatched -- inputs, interactors, and legends in the subscriber graph are filtered out." Implementation note 6 repeats this.

5. **[LOW] emit_sources warnings are surfaced but behaviour is unspecified** -- RESOLVED. AC-03 now specifies `LoadResult { session, warnings }` and `session.ddl_warnings() -> &[ParseWarning]` with a verification step for warning accessibility.

6. **[LOW] Arrow version coupling is acknowledged but not pinned** -- RESOLVED. Implementation note 2 now says "Use duckdb::arrow re-export for RecordBatch and all Arrow types -- do NOT add an independent arrow dependency."

All six v1 findings have been addressed in the current spec revision.

## Findings

### [LOW] Interview API signature for update_param diverges from spec
**Category:** missing-requirement
**Pass:** 1
**Description:** The interview (Q6) documents `update_param` as returning `Result<Vec<(usize, Vec<RecordBatch>)>, EngineError>` (single Result, no partial failure). The spec (AC-06) correctly evolved this to `Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>` to support partial failure. This is the right call, but the interview document is now stale on this point. Since the spec is the authoritative artifact, this is informational only.
**Evidence:** Interview Q6 vs AC-06 description.
**Recommendation:** No action required. The spec supersedes the interview. Note this if anyone references the interview during implementation.

### [LOW] AC-03 load_spec return type wrapping is implicit
**Category:** assumption
**Pass:** 2
**Description:** AC-03 says `load_spec` returns `LoadResult { session, warnings }`, but does not explicitly state the outer `Result<LoadResult, EngineError>` wrapping. The interview (Q6) shows `Result<Session, EngineError>` as the return type. Since load_spec can fail (e.g., ConnectionFailed, DdlFailed), the full signature is presumably `Result<LoadResult, EngineError>`. AC-08 confirms DdlFailed errors surface from load_spec, which implies the Result wrapping. This is inferable but not stated.
**Evidence:** AC-03 describes LoadResult but not the error path. AC-08 describes DdlFailed errors from data source loading.
**Recommendation:** No action required -- the error wrapping is unambiguously implied by AC-08. Implementor should use `Result<LoadResult, EngineError>`.

---

## Gate-AC Verification Check

AC-11 is the only gate-type AC. Its verification field: "Review the engine's use statements -- only pub items from upstream crates." (72 chars, non-placeholder, non-empty). Passes all three deterministic rules.

---

## Honest Assessment

This spec is ready for implementation. The v1 review's findings have all been addressed -- the subscriber graph mapping strategy, partial failure semantics, cache observability, warning surfacing, and Arrow type compatibility are now explicitly specified. The two remaining LOW findings are informational (stale interview text, implicit Result wrapping) and do not require spec changes. The biggest implementation risk is the MarkIndexMap construction (implementation note 6) -- it requires correctly walking the component tree and matching paths from the subscriber graph -- but the spec provides sufficient direction for the implementor to proceed. The prepared statement cache design is sound and well-aligned with the existing `EmittedQuery.plan_hash` and `Binding` types in brightfield-sql.
