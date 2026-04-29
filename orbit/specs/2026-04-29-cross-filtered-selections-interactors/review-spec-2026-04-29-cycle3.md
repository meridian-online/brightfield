# review-spec cycle 3 — cfs3 v1.2

**Date:** 2026-04-29
**Reviewer:** cold-context spec reviewer (cycle 3)
**Spec:** orbit/specs/2026-04-29-cross-filtered-selections-interactors/spec.yaml v1.2

## Cycle-2 finding resolution

```
| Cycle-2 finding                                                              | Class | Fix in v1.2                                                                                                                                                           | Status   |
|------------------------------------------------------------------------------|-------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------|
| N1 ac-10 step (4) baselines disagree with the actual rpw3 baseline           | HIGH  | Lines 211-213 now read `cfs_=12, cfs2_=16, rpw3_=16`. Re-verified on `origin/rally/reactive-parameters-runtime` (tip 69ed0e4): cfs_=12, cfs2_=9+1+4+2=16, rpw3_=6+3+1+6=16. Counts match. | RESOLVED |
| N2 ac-10 step (3) fingerprint diff omits files where some named symbols live | HIGH  | Diff path list extended (lines 169-175) to include `crates/brightfield-ui/src/chart_view.rs` and `crates/brightfield-sql/src/emit.rs`. Re-verified that `pub struct BrushBinding` lives at chart_view.rs:165, `pub fn emit_query` at emit.rs:320, `pub fn emit_query_with_passes` at emit.rs:334. All three patterns now have a chance to match. | RESOLVED |
```

## Empirical re-verification

```
$ git show origin/rally/reactive-parameters-runtime -- counts per prefix on rally tip 69ed0e4
cfs_   → 12 (analysis.rs)
cfs2_  → 16 (lib.rs:9 + brush.rs:4 + chart_view.rs:2 + analysis.rs:1)
rpw3_  → 16 (lib.rs:6 + slider.rs:6 + analysis.rs:3 + vocab.rs:1)
```

Spec baselines agree with the empirical counts. The reformatted file-path list under ac-10 step (3) covers every fingerprinted symbol's home file, including the two missing in v1.1 (chart_view.rs for BrushBinding; emit.rs for emit_query / emit_query_with_passes). The spec also adds an explicit comment block (lines 191-200) mapping each pattern to its file, which makes future drift between pattern list and path list easier to spot.

## Cycle-1 spot-check

F1 (ac-05 fixture path), F4 (constraint 7 carve-out for `on_mouse_up_with_dispatch`), F5 (ac-04 lifted-helper boundary), F7 (ac-09 value contract), F8 (escape-key memo path), F9 (click-vs-drag rationale shift), F10 (ac-09 enum disambiguation) — all marked RESOLVED in cycle 2, still present and unchanged in v1.2 (verified by inspection of constraint 7 line 17, ac-04 line 66, ac-05 line 90, ac-09 lines 138-153, impl-notes paras 7-8 lines 241-242). Prior verification stands.

F2 (ac-10 step (4) falsifiable) and F3 (ac-10 step (3) v1/v2 surface coverage) — these were the partial-fix items that cycle 2 escalated as N1 / N2 above. Now closed.

## New findings (cycle 3)

None.

I scanned the v1.2 revision diff (the review_revisions list, lines 310-312, only adds the N1 baseline correction and the N2 path-list extension). No new constraints, ACs, or ontology fields were introduced. The four-step gate verification reads coherently end-to-end: the cargo test, gpui dependency check, signature-fingerprint diff, and test-function count gate are mutually consistent and all four are now falsifiable on a freshly-branched cfs3 tree.

The carve-out for `on_mouse_up_with_dispatch` (constraint 7, ac-04 verification, ac-10 step (3) prose at lines 165-167 and 197-200) remains internally consistent: chart_view.rs is in the diff path list to pin BrushBinding, and the rg pattern list deliberately does not match `on_mouse_up_with_dispatch`. The cfs2_ac11 wrapper preservation (impl-notes para 5, line 239) is the documented bridge.

## Verdict

APPROVE
