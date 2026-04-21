# Spec Review

**Date:** 2026-04-21
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-20-mosaic-web-spec-portability/spec.yaml
**Verdict:** APPROVE

---

## Review Depth

```
| Pass | Triggered by                                                         | Findings |
|------|----------------------------------------------------------------------|----------|
| 1    | always                                                               | 1        |
| 2    | content signals (cross-crate dep on card 0001; shared root artefact) | 2        |
| 3    | not triggered                                                        | —        |
```

Pass 1 surfaced no gate-verification violations and no structural contradictions; Pass 2 ran because the spec touches cross-system boundaries (reach-in to card 0001's kind-registry and `ImplStatus`) and produces shared repo-root artefacts (`deviations.yaml`, `DEVIATIONS.md`). Pass 2 found only non-blocking clarifications — no cascading failure modes, no untestable ACs, no rollback concern (v1 is additive: new crate + new root files). Pass 3 was not triggered.

## Findings

### [LOW] `ImplStatus` semantics are parser-level, but `SupportReport` is framed renderer-facing
**Category:** assumption
**Pass:** 2
**Description:** Card 0001 defines `ImplStatus = Implemented | Planned | Unimplemented` as a *parser-level* status — whether the parser typed-stubs a node vs. fully parses its payload (card 0001 interview Q3, AC-03). The interview and card 0002's scenarios describe `SupportReport` as a *renderer-level* concept — "identifies every Mosaic mark/interactor/input not yet implemented in brightfield." In v1 this is harmless because brightfield has no renderer yet; the parser-level status is a reasonable proxy. It will diverge the moment a renderer lands and a mark is parser-`Implemented` but renderer-unimplemented. No AC or constraint in this spec acknowledges that divergence or the upgrade path (e.g. a separate render-status field on the vocabulary registry, or a union check in preflight).
**Evidence:** spec.yaml constraint #3 routes `SupportEntry.status` directly from `brightfield_spec::ImplStatus`; AC-02, AC-03 re-export it verbatim. Card 0001 AC-03 describes the status as flagged at the vocabulary-registry level with no render distinction. Interview §Q3 frames the report as "unsupported nodes … available to tools … without needing to parse separately" — an ambiguous framing that reads as render-facing to a new reader.
**Recommendation:** Non-blocking for v1. Add a short note to AC-17's README requirements noting that `ImplStatus` is parser-level in v1 and that a renderer-level status will be layered in (likely as a second `ImplStatus` or a compound predicate) when the renderer lands. Keeps future readers from assuming `is_renderable()` reflects actual rendering capability.

### [LOW] AC-16's "accounted-for" gate needs curated specs to either use only parser-Implemented vocabulary or ship with pre-populated deviation entries
**Category:** test-gap
**Pass:** 2
**Description:** AC-16 requires that every curated spec's `preflight(&spec).blocking()` is either empty or every blocking `ComponentIdentity` appears in `deviations.yaml` with a matching `affected_specs` entry. Given card 0001's registry flags many vocabulary entries `Planned`/`Unimplemented` (AC-03 covers Mosaic's *full* vocabulary), curated specs drawn from the interview's starter set (`crossfilter.yaml`, `legends.yaml`, `overview-detail.yaml`, etc.) are likely to exercise at least one `Unimplemented` component on day one. The spec does not state which of the two paths v1 takes: (a) pick a curated subset whose every component is `Implemented`, or (b) ship `deviations.yaml` with bootstrap entries covering the gap. AC-09 permits an empty `deviations: []` initial state, which is only consistent with path (a). If (a) is intended, the starter list in constraint #8 likely needs pruning; if (b) is intended, AC-09's permission to ship empty is internally inconsistent with AC-16. The registry-integrity gate (AC-14) will still pass in either case; AC-16 is the gate that trips.
**Evidence:** spec.yaml constraint #8 enumerates the 10 starter specs; AC-09 allows empty initial registry; AC-16 requires accounted-for blocking entries. Card 0001 AC-03 requires the kind registry to cover Mosaic's *full* 0.24.x vocabulary with mixed `ImplStatus` — so `blocking()` is almost certainly non-empty for specs like `overview-detail.yaml` or `legends.yaml`.
**Recommendation:** Non-blocking — this is resolvable at implementation time by either shrinking the day-one curated list or seeding `deviations.yaml` with bootstrap entries. A one-line clarification in constraint #16 ("V1 ships whichever combination of (curated subset, bootstrap deviations) makes AC-16 green; the exact split is an implementation decision") would make the contract self-consistent on the page.

---

## Honest Assessment

The spec is unusually tight for a card this wide in scope. Seventeen ACs, a clean gate/code split, deterministic-output discipline on the doc generator, LF-everywhere rule, explicit separation between loader-local and cross-artefact integrity checks (AC-14 owns the bidirectional gate; `load_deviations` does not), and a sealed `LayerOutcome` that distinguishes `Pending` from `Suppressed` in a way that keeps v1 honest rather than fake-green. The constraint list reads like it was hammered on hard in the interview — every near-miss (e.g. the "`Pending` vs `Suppressed`" distinction, the "observed corpus by reference, not re-vendor" decision, the `CARGO_MANIFEST_DIR`-resolved path) is called out and justified. I could not find a structural contradiction or an untestable AC.

The two low-severity findings are both clarifications rather than corrections: (1) the parser-vs-render status conflation is harmless in v1 but will bite when a renderer lands, and (2) AC-09/AC-16 leave the bootstrap-content decision implicit in a way that implementers will hit on day one. Neither blocks implementation; both are cheap to fix in a future edit or to resolve at implementation time. The biggest risk remaining is external to the spec: this crate depends on card 0001's kind-registry enums being publicly exported under the names used here (`MarkKind`, `InteractorKind`, `InputKind`, `ComponentKind`, `ImplStatus`, `SourceSpan`). Card 0001's shipped spec (already merged) confirms those names and surfaces, so that dependency is sound.

Ready to implement.
