# Spec Review

**Date:** 2026-04-21
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-21-direct-data-source-loading/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 2 |
| 2 — Assumption & failure | content signals: cross-crate modification, shared card surface | 3 |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [MEDIUM] Missing dispatch arm for `DataSourceKind::Typed` and `DataSourceKind::Opaque`
**Category:** missing-requirement
**Pass:** 1
**Description:** The spec's constraint 3 defines dispatch for File (by extension), InlineRows, Query, and Shorthand. It does not specify what `emit_sources` should do when it encounters `DataSourceKind::Typed(s)` (without a `file:` key) or `DataSourceKind::Opaque`. These are valid AST variants produced by the parser (see `parse.rs:498` and `parse.rs:508`). The `SourceKindTag` enum in AC-04 has no arm for either.
**Evidence:** `crates/brightfield-spec/src/parse.rs:498` — `type:` without `file:` or `query:` produces `Typed(s)`. Line 508 — mapping with no recognised key produces `Opaque`. Neither appears in constraint 3's dispatch table or in AC-04's `SourceKindTag` enum.
**Recommendation:** Add a constraint specifying the behaviour: either `EmitError::UnknownFormat` for both, or skip with a `ParseWarning`. Add a corresponding AC with a unit test. If these variants are expected to be unreachable in the curated corpus, state that explicitly and use `InvariantViolation` as the error.

### [LOW] Interview-spec signature divergence on `emit_sources`
**Category:** content-signal
**Pass:** 1
**Description:** The interview's implementation surface section (line 87) defines the signature as `emit_sources(spec: &Spec, preflight: &SupportReport)`, but the spec's AC-04 defines it as `emit_sources(spec: &Spec, base_dir: Option<&Path>)`. Additionally, the interview's D8 uses `ParseError::MalformedDataDef` for the inline row limit, while the spec uses `EmitError::InlineRowLimit`. The spec should be the source of truth, but the divergence suggests the design evolved and the interview was not updated.
**Evidence:** Interview line 87 vs spec AC-04 line 69; Interview D8 vs spec AC-09 / constraint 5.
**Recommendation:** No spec change needed — the spec is more precise and internally consistent. Noting for traceability only. If the interview is a living document, consider updating it.

### [MEDIUM] `ParseOutput` field addition is a semver-breaking change without mitigation
**Category:** assumption
**Pass:** 2
**Description:** Adding `pub base_dir: Option<PathBuf>` to `ParseOutput` (AC-03) breaks any downstream code that constructs `ParseOutput` with struct literal syntax, since the struct has no `#[non_exhaustive]` attribute and derives `Default`. The spec's constraint 2 claims "no existing callers break" and the interview acknowledges this is "a breaking change in strict interpretation" but waves it off as "rally-internal". This is true today — but it is an assumption that no external consumer exists or will exist before a `#[non_exhaustive]` annotation is added.
**Evidence:** `parse.rs:206-212` — `ParseOutput` has no `#[non_exhaustive]`; `Default` derive means `..Default::default()` workaround is available but not enforced.
**Recommendation:** Either (a) add `#[non_exhaustive]` to `ParseOutput` in this card's scope and note it as an additive constraint, or (b) explicitly document in the spec that this is accepted tech debt to be addressed when the crate's public API stabilises. Option (b) is fine — just make it explicit rather than silent.

### [MEDIUM] No AC covers `DataSourceKind::Opaque` or unknown-extension error path end-to-end
**Category:** test-gap
**Pass:** 2
**Description:** Constraint 3 says "Unknown extension -> `EmitError::UnknownFormat`", but no AC includes a verification test for an unknown extension. AC-07 tests `.geojson` without `type: spatial` as one `UnknownFormat` path, but a file like `data.xlsx` hitting the catch-all dispatch is not tested.
**Evidence:** Searching all AC verifications — none mention an unknown extension test.
**Recommendation:** Add a unit test (could be folded into AC-02's `EmitError` variant tests or a new AC) that verifies `file: data.xlsx` produces `EmitError::UnknownFormat { extension: "xlsx" }`.

### [LOW] Inline-row column ordering assumption
**Category:** assumption
**Pass:** 2
**Description:** AC-09 says "column names from first row keys (object-per-row)". In Rust, `serde_yaml::Value::Mapping` preserves insertion order, and the parser converts to `SpecValue` (which uses `IndexMap`). However, the spec does not state which serialisation of the keys is canonical — if the first row has keys `{b, a, c}`, does the VALUES clause use that order or sort alphabetically? This matters for snapshot determinism.
**Evidence:** AC-09 and constraint 9 (canonicalisation sorts kwargs alphabetically) — but kwargs in reader functions and column names in VALUES are different concerns. Column order in VALUES affects the view schema.
**Recommendation:** Add a clarifying note to AC-09 or constraint 5: column names preserve insertion order from the first row (matching YAML source order), or are sorted alphabetically for determinism. Either choice is valid; it just needs to be stated.

---

## Honest Assessment

This is a well-structured spec with high testability and clear scope boundaries. The two medium findings that matter most are the missing dispatch arms for `Typed`/`Opaque` variants and the missing unknown-extension test. Both are straightforward to address — likely one new constraint clause and one or two additional test assertions. The `ParseOutput` breaking-change point is low-risk in practice but should be acknowledged explicitly rather than silently assumed. Once these gaps are addressed, the spec is ready for implementation.
