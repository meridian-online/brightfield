# Spec Review — Cross-Filtered Selections Runtime Coordinator (card 0006 v2)

**Spec:** orbit/specs/2026-04-28-cross-filtered-selections-runtime/spec.yaml
**Date:** 2026-04-28
**Mode:** cold cycle 1 (forked-Agent semantics, executed inline; the review brief
contains only the spec and the canonical verdict-line contract)

## Summary

The spec is a runtime coordinator slice that mirrors the card 0005 v2
`propagate_param` pattern, adapted for the multi-contributor / per-subscriber
predicate-synthesis shape selections require. 15 ACs (12 code + 2 gate + 1
test-count gate). The decision pack and interview accepted six decisions
wholesale; the spec implements all six faithfully.

## Findings

```
| #  | severity | scope               | finding                                                              |
|----|----------|---------------------|----------------------------------------------------------------------|
| 1  | MEDIUM   | ac-09               | Signature commitment was previously deferred to "implementation"     |
| 2  | LOW      | ac-11               | "test double" mechanism unspecified; fallback clause covers it       |
| 3  | LOW      | ac-12               | Row-count "fewer than unfiltered" depends on chosen predicate        |
| 4  | LOW      | ac-07               | Single AC covers both unsubscribed and unknown-selection cases       |
```

### 1. ac-09 — emit_query signature commitment (MEDIUM, addressed pre-verdict)

The decision pack and interview both flag the `emit_query ignores _param_values`
LOW finding from the card 0005 v2 PR review as load-bearing for this slice. The
draft AC text said "exact signature is finalised in implementation"; the
implementation note already preferred the additive-third-argument route, but the
AC itself did not carry the commitment.

A spec whose primary deliverable is closing a known API gap should commit to the
shape of the API, not just the observable. Without that commitment the implement
stage could legitimately choose either route, and the PR review would have to
re-litigate the decision against the 0005 v2 review record.

**Resolution applied during review:** ac-09 now commits to the signature
`emit_query(spec, mark_index, param_values: Option<&ParamValues>,
selection_predicates: Option<&[(String, Predicate)]>) -> Result<EmittedQuery, EmitError>`
and explicitly enumerates the call-site updates. Implementation may still factor
through a private helper, but the public surface is now fixed at spec time.

### 2. ac-11 — test double mechanism (LOW)

`ChartView` does not currently hold a `Session`; the implementation note
suggests `Arc<Mutex<Session>>`. The AC verification permits "mock or
test-double" without naming the mechanism. The fallback clause ("If wiring
through a real Session in tests is impractical, an equivalent integration test
driving on_mouse_up against a real Session in a headless harness satisfies the
AC") makes the AC satisfiable in either direction. Acceptable for a slice
where the wiring shape is genuinely an implementation choice.

### 3. ac-12 — row-count heuristic (LOW)

"At least one returned RecordBatch has fewer rows than the unfiltered
execution" is a structural assertion that a non-trivial predicate filtered
something, but it depends on the tester picking a range that matches the
chosen vendored data. The vendored crossfilter.yaml uses a small flights
dataset; an x-range covering the whole domain would trivially pass without
exercising the predicate threading. Implementer should pick a predicate that
demonstrably reduces row count (e.g. a 25th-percentile range). Annotating
this as "tester discretion" in the verification text is sufficient — the
intent is clear.

### 4. ac-07 — unsubscribed vs unknown (LOW)

The card 0005 v2 spec splits these (rpw_ac03 unsubscribed, rpw_ac05 unknown).
For selections the distinction is less crisp because selections are not
pre-declared in `param_state` form — the empty-subscriber-graph case is the
only meaningful "absorb silently" trigger. One AC is sufficient.

## Constraint coverage check

```
| constraint                                                          | covered by   |
|---------------------------------------------------------------------|--------------|
| Coordinator on Session in brightfield-engine                        | ac-02        |
| Predicate IR is the runtime currency; no AST/SpecValue variants     | ac-13        |
| Sync &mut self; UI debounces                                        | ac-11, notes |
| Existing Session API unchanged                                      | ac-14        |
| Self-exclusion via parent plot path                                 | ac-04, ac-05 |
| Per-subscriber, dispatch-time, no caching                           | ac-02, ac-06 |
| Partial failure: continue on subscriber error                       | ac-08        |
| selection_state always updated regardless of outcomes               | ac-08        |
| Corpus regression gate green                                        | ac-13        |
| brightfield-render no-gpui invariant                                | ac-10 (note) |
| All existing tests pass                                             | ac-14        |
```

Every constraint maps to at least one AC. The brightfield-render no-gpui
invariant is asserted in the ac-10 implementation note rather than a dedicated
AC; this is structurally protected (ac-10 places the brush adapter in
brightfield-ui or a UI-imported adapter, never in brightfield-render) and the
ac-14 workspace test gate is the trip-wire if anyone violates it.

## Decision-pack alignment

All six decisions are reflected in the spec:

```
| Decision pack                                            | spec carrier         |
|----------------------------------------------------------|----------------------|
| D1 — separate propagate_selection                        | ac-02                |
| D2 — typed selection_state with Predicate                | ac-01, ac-02         |
| D3 — dispatch-time resolution, no caching                | ac-06, notes         |
| D4 — parent-plot path equality                           | ac-04, ac-05         |
| D5 — sync coordinator + UI debounce                      | ac-11, notes         |
| D6 — partial-failure pattern                             | ac-08                |
```

## Test plan adequacy

The spec enumerates 12 cfs2_ test names across 12 code-typed ACs, plus the
explicit ≥8 count gate (ac-15). Test names follow the rpw2_ convention.
ac-08 mirrors rpw2_ac04's two-subscriber one-supported / one-unsupported
shape exactly. End-to-end coverage via ac-12 satisfies the rally constraint
that this slice ship at least one cross-spec end-to-end test against
vendored crossfilter.yaml.

## Disposition

Findings are LOW or addressed-pre-verdict. The MEDIUM was a signature
commitment gap closed by editing ac-09 in place during review. No structural
issues with the decision-pack faithfulness, AC coverage, test plan, or
constraint mapping.

## Verdict

**Verdict:** APPROVE
