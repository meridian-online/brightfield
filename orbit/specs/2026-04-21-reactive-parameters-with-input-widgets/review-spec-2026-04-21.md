# Spec Review

**Date:** 2026-04-21
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-21-reactive-parameters-with-input-widgets/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by        | Findings |
|------|---------------------|----------|
| 1 — Structural scan       | always              | 3        |
| 2 — Assumption & failure  | not triggered        | —        |
| 3 — Adversarial           | not triggered        | —        |
```

## Findings

### [LOW] AC-12 verification is not an executable shell command
**Category:** test-gap
**Pass:** 1
**Description:** The verification string `grep -r 'fn rpw_' crates/ | wc -l >= 15` is not valid shell — `wc -l` outputs a number but `>= 15` is not a shell comparison. This will not execute as-is during implementation verification.
**Evidence:** spec.yaml line 145: `verification: "grep -r 'fn rpw_' crates/ | wc -l >= 15"`
**Recommendation:** Either rewrite as an executable assertion (e.g., `test $(grep -rc 'fn rpw_' crates/ | awk '{s+=$1}END{print s}') -ge 15`) or note that this is a human-verified count check, not a runnable command. Low severity because the implementer will count tests regardless.

### [LOW] AC-08 type mapping omits Array variant for ParamDeclaredType
**Category:** test-gap
**Pass:** 1
**Description:** AC-08's verification says "Assert from_param_node maps Value(Integer)->ScalarNumeric, Value(String)->ScalarString, Selection->Selection, etc." but does not explicitly mention Value(Array)->Array or Value(Bool)->ScalarBool, despite the enum declaring those variants. The "etc." is slightly vague.
**Evidence:** spec.yaml lines 106-109 (AC-08 verification). The enum lists ScalarBool and Array but the verification only gives three explicit mappings plus "etc."
**Recommendation:** Expand the verification to enumerate all five ParamDeclaredType variants explicitly. Low severity because the enum definition in the description itself is complete — the verification is just slightly hand-wavy.

### [LOW] AC-07 "Table writing to a Scalar param" — direction may be ambiguous
**Category:** assumption
**Pass:** 1
**Description:** AC-07 lists "Table writing to a Scalar param" as a provably incompatible pair. The spec defines Table's WidgetOutputType as Selection (AC-08). Writing a Selection into a Value (scalar) param is indeed a type mismatch, but the AC-07 verification says "table bound to a value param" while AC-08 maps Table->Selection. The implementer needs to understand that the mismatch is Selection (widget output) vs ScalarNumeric/ScalarString (param declared type). This is implicit but clear enough from context.
**Evidence:** spec.yaml lines 89-95 (AC-07), lines 107-108 (AC-08 Table->Selection mapping).
**Recommendation:** No change needed — the mapping is unambiguous when AC-07 and AC-08 are read together. Noting for completeness.

---

## Gate-AC Verification Check

AC-11 is the only `ac_type: gate`. Its verification field is `"cargo test --workspace"` (20 characters, non-empty, not a placeholder token). **Pass** on all three deterministic rules.

---

## Content Signal Scan

No deepening triggers detected. The spec is scoped to pure static analysis at parse/load time within the `brightfield-spec` crate. No deployment, infrastructure, cross-system, security, data migration, or training data concerns.

---

## Honest Assessment

This spec is ready for implementation. It is well-scoped, tightly constrained to static analysis (no runtime coordinator), and each AC has a concrete verification method. The goal-to-scope alignment is strong: the interview's runtime concerns (epoch propagation, coordinator) are explicitly deferred, and the spec builds exactly the static infrastructure those future features will consume. The biggest risk is minor: the type-mapping enums (AC-08) introduce vocabulary that has no corpus validation yet (all four InputKind variants are currently `Unimplemented`), so the implementer will be working from the Mosaic specification rather than tested corpus behaviour. The constraints adequately mitigate this by requiring vendored-spec regression (AC-02, AC-11). The three LOW findings are polish items, not blockers.
