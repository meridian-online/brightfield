# Spec Review

**Date:** 2026-04-21
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-21-direct-data-source-loading/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 1 |
| 2 — Assumption & failure | content signals: cross-crate modification, new crate scaffold | 1 |
| 3 — Adversarial | not triggered | — |
```

## V1 Review Resolution

All five findings from v1 (review-spec-2026-04-21.md) have been addressed:

1. **Missing dispatch for Typed/Opaque** — constraint 3 now explicitly maps both to `EmitError::InvariantViolation`. AC-17 adds three unit tests covering `Typed` without file, `Opaque`, and unknown extension.
2. **Interview-spec signature divergence** — informational only, no change needed. Spec remains source of truth.
3. **ParseOutput semver-breaking change** — constraint 2 now explicitly acknowledges this as accepted tech debt with rationale ("the crate has no external consumers yet").
4. **No AC for unknown-extension error path** — AC-17 includes `dfsql_unknown_extension_errors` for `.xlsx`.
5. **Inline-row column ordering** — constraint 5 now states "preserving YAML source order via IndexMap — deterministic for snapshot comparison".

## Findings

### [LOW] `emit_sources` returns `Result<Vec<SourceDdl>, EmitError>` — first error halts all emission
**Category:** assumption
**Pass:** 1
**Description:** The signature in AC-04 and constraint 7 returns a single `EmitError`, meaning emission stops at the first failing data source. If a spec has 5 data sources and the 3rd has an unknown extension, sources 4 and 5 are never attempted. This is a reasonable v1 choice (fail-fast), but it means a user cannot see all emission errors at once. The spec does not state whether this is intentional or a simplification.
**Evidence:** AC-04 line 71: `-> Result<Vec<SourceDdl>, EmitError>`. No constraint mentions error accumulation or fail-fast policy.
**Recommendation:** No change required — fail-fast is the simpler and safer default for v1. Noting for traceability: if card 0003 or a future card wants accumulated diagnostics, the return type will need to change to something like `(Vec<SourceDdl>, Vec<EmitError>)`.

### [LOW] `Shorthand` variant mentioned in AC-10 but absent from `SourceKindTag` enum and constraint 3 dispatch table
**Category:** content-signal
**Pass:** 2
**Description:** AC-10 says `DataSourceKind::Shorthand` is "treated as a query (bare table name or inline SQL)" and uses `SourceKindTag::Query` for both Query and Shorthand. Constraint 3 lists dispatch for `File`, `InlineRows`, `Query`, `Typed`, and `Opaque` but does not mention `Shorthand` explicitly. The behaviour is inferable (AC-10 states it clearly), but the dispatch table in constraint 3 has a gap in its enumeration of all `DataSourceKind` variants.
**Evidence:** Constraint 3 omits `Shorthand`. AC-10 covers it. `SourceKindTag` in AC-04 includes `Query` but not `Shorthand` (correctly, since Shorthand maps to Query).
**Recommendation:** No change required — AC-10 is unambiguous and the test coverage (`dfsql_shorthand_emission`) will verify it. The constraint's dispatch table could be more exhaustive, but the AC fills the gap.

---

## Honest Assessment

This spec is ready for implementation. The v1 review's substantive findings have all been addressed cleanly — error-path dispatch is now fully enumerated in both constraints and ACs, the semver concern is explicitly acknowledged as tech debt, and column ordering is pinned to insertion order. The two low-severity findings noted here are documentation-completeness observations, not implementation risks. The 17 ACs provide thorough coverage across all format dispatch arms, error paths, conformance integration, and the deviation registry. The spec's scope is well-bounded (DDL emission only, no runtime, no I/O), which limits the blast radius and makes testing straightforward.
