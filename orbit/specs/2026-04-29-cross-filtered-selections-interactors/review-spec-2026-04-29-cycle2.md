# review-spec cycle 2 — cfs3 v1.1

**Date:** 2026-04-29
**Reviewer:** cold-context spec reviewer (cycle 2)
**Spec:** orbit/specs/2026-04-29-cross-filtered-selections-interactors/spec.yaml v1.1

## Cycle-1 finding resolution

```
| Cycle-1 finding                                               | Class  | Fix in v1.1                                                                                                                                                                                                                                                              | Status     |
|---------------------------------------------------------------|--------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------|
| F1 ac-05 fixture path uses interactor[intervalXY]             | HIGH   | ac-05 verification block updated to `root/plot[0]/interactor[intervalXY]` and now cites the kind.wire_name() convention at analysis.rs:910.                                                                                                                                  | RESOLVED   |
| F2 ac-10 step (4) replaced with falsifiable check             | HIGH   | Path-glob replaced with rg-based test-function count gate (`rg -n '\bfn cfs_' crates/ \| wc -l` etc.). However, the **hardcoded baselines are wrong**: spec says cfs_=9, rpw3_=13 but the actual counts on `origin/rally/reactive-parameters-runtime` are cfs_=12 and rpw3_=16 (cfs2_=16 is correct). See cycle-2 N1 below.                                          | PARTIAL    |
| F3 ac-10 step (3) fingerprint extended to v1/v2 surfaces      | HIGH   | rg `-e` list gained `propagate_selection`, `BrushBinding`, `brush_rect_to_predicate`, `emit_query`, `emit_query_with_passes`, `InteractorBinding`. However, the `git diff` file list does NOT contain the files where three of those symbols actually live (`BrushBinding` in `chart_view.rs`; `emit_query` / `emit_query_with_passes` in `crates/brightfield-sql/src/emit.rs`). Those three pattern lines can never match. See cycle-2 N2 below.                                                                                                                              | PARTIAL    |
| F4 constraint 7 carves out on_mouse_up_with_dispatch          | HIGH   | Constraint 7 now ends with `**Carved out (intentionally changes)**: ChartView::on_mouse_up_with_dispatch parameter type and return shape … see ac-04` plus the cfs2_ac11 wrapper note. ac-10 step (3) prose explicitly excludes `on_mouse_up_with_dispatch`. Contradiction gone. | RESOLVED   |
| F5 ac-04 names lifted-helper boundary                         | MEDIUM | ac-04 description rewritten: "Verified through the cfs2_ac11 lifted-helper boundary (`commit_brush_release_multi`) — end-to-end GPUI event simulation is out of scope, matching the cfs2_ac11 / rpw3_ac12 precedent."                                                          | RESOLVED   |
| F7 ac-09 documents point_predicate value contract             | LOW    | ac-09 description gained the explicit clause: `value` is "an already-formatted SQL literal" — the helper preserves the caller's quoting.                                                                                                                                       | RESOLVED   |
| F8 escape-key follow-up memo path                             | LOW    | impl-notes para 8 now names `orbit/cards/memos/2026-04-29-escape-key-clearing.md` and references the Decision 5 memo for symmetry.                                                                                                                                              | RESOLVED   |
| F9 click-vs-drag rationale shift                              | LOW    | impl-notes para 7 gained a "**Rationale shift from decisions.md Q1**" paragraph explaining the move from start-time gating to release-time zero-area branch.                                                                                                                    | RESOLVED   |
| F10 ac-09 enum disambiguation                                 | LOW    | ac-09 sub-clause (c) reframed to use a panZoom fixture and assert "non-brushable kinds excluded", avoiding the cross-crate enum coupling.                                                                                                                                       | RESOLVED   |
```

## New issues (cycle 2)

### HIGH

**N1 — ac-10 step (4) baselines disagree with the actual rpw3 baseline.**
The spec hardcodes `cfs_=9, cfs2_=16, rpw3_=13` as the "pre-cfs3 baseline". I verified against `origin/rally/reactive-parameters-runtime` (commit 69ed0e4, "review-pr(rpw3): cycle 1 APPROVE — 2 LOW findings"):

```
| Prefix | Spec baseline | Actual on rally/reactive-parameters-runtime | Files                                                                                                                  |
|--------|---------------|----------------------------------------------|------------------------------------------------------------------------------------------------------------------------|
| cfs_   | 9             | 12                                           | crates/brightfield-spec/src/analysis.rs (12)                                                                           |
| cfs2_  | 16            | 16                                           | brightfield-engine/src/lib.rs (9), brightfield-ui/src/brush.rs (4), chart_view.rs (2), brightfield-spec/src/analysis.rs (1) |
| rpw3_  | 13            | 16                                           | brightfield-engine/src/lib.rs (6), brightfield-ui/src/slider.rs (6), brightfield-spec/src/analysis.rs (3), vocab.rs (1) |
```

Effect: a fresh implementer who runs the gate verbatim on a freshly-branched cfs3 tree will see `12 != 9` and `16 != 13` and fail the gate before writing any code. The gate is supposed to be "no cfs3-introduced churn on cfs/cfs2/rpw3"; with these baselines it's "the rpw3 PR has more tests than the spec author expected". The fix is to update the literals to 12, 16, 16 (or to delete the literals and require equality with the rally-branch count via the `git show` loop the spec already sketches).

This finding is mechanical, but it IS falsifiability-relevant: if the implementer "fixes" the baseline by editing the spec to match observed reality without verifying *why* the count differs, they could mask a legitimate regression in their own changes.

**N2 — ac-10 step (3) fingerprint diff omits files where some of the named symbols live.**
The diff command is:

```
git diff origin/rally/reactive-parameters-runtime -- \
  crates/brightfield-engine/src/lib.rs \
  crates/brightfield-spec/src/analysis.rs \
  crates/brightfield-ui/src/slider.rs \
  crates/brightfield-ui/src/brush.rs \
| rg -F -e '...'
```

Grepped against `origin/rally/reactive-parameters-runtime`:

```
| Pattern                              | Actual location on rally branch                  | In diff list? |
|--------------------------------------|--------------------------------------------------|---------------|
| pub fn propagate_param               | brightfield-engine/src/lib.rs:456                | YES           |
| pub fn topological_descendants       | brightfield-spec/src/analysis.rs:410             | YES           |
| pub trait ParamDispatcher            | brightfield-ui/src/slider.rs:39                  | YES           |
| pub struct SliderBinding             | brightfield-ui/src/slider.rs:67                  | YES           |
| pub fn commit_slider_release         | brightfield-ui/src/slider.rs:175                 | YES           |
| pub enum SliderState                 | brightfield-ui/src/slider.rs:128                 | YES           |
| pub fn propagate_selection           | brightfield-engine/src/lib.rs:262                | YES           |
| pub struct BrushBinding              | brightfield-ui/src/chart_view.rs:165             | **NO**        |
| pub fn brush_rect_to_predicate       | brightfield-ui/src/brush.rs:81                   | YES           |
| pub fn emit_query                    | brightfield-sql/src/emit.rs:320                  | **NO**        |
| pub fn emit_query_with_passes        | brightfield-sql/src/emit.rs:334                  | **NO**        |
| pub struct InteractorBinding         | brightfield-spec/src/analysis.rs:731             | YES           |
```

Three of the twelve fingerprint lines (BrushBinding, emit_query, emit_query_with_passes) are silently dead — they will never match a hunk because their files are not in the `git diff` path list. Net effect: a future change that, say, edits `pub struct BrushBinding { ... new_field: T }` in chart_view.rs would pass the gate, even though constraint 7 explicitly commits to leaving BrushBinding fields untouched.

The cycle-1 F3 finding flagged exactly this risk for the v1/v2 surface, and the v1.1 fix added the rg patterns but did not add the corresponding files. The fix is to extend the diff path list:

```
git diff origin/rally/reactive-parameters-runtime -- \
  crates/brightfield-engine/src/lib.rs \
  crates/brightfield-spec/src/analysis.rs \
  crates/brightfield-ui/src/slider.rs \
  crates/brightfield-ui/src/brush.rs \
  crates/brightfield-ui/src/chart_view.rs \
  crates/brightfield-sql/src/emit.rs \
  | rg -F -e ...
```

Note that `chart_view.rs` is also where `on_mouse_up_with_dispatch` lives. ac-10's step (3) prose says it is "INTENTIONALLY omitted". With chart_view.rs added to the diff list, that intentional omission is enforced by NOT having a rg `-e` pattern for it — which is already the case. So adding `chart_view.rs` to the path list is safe with respect to F4's carve-out.

### MEDIUM / LOW

None observed in cycle 2.

## Implementation-readiness check

The structural spec is sound. All four cycle-1 HIGH findings (F1, F4) and the MEDIUM/LOW items (F5, F7-F10) have substantive, well-targeted resolutions. The carve-out language for `on_mouse_up_with_dispatch` is explicit and the cfs2_ac11 preservation pathway via the single-binding wrapper is clearly documented. The rally-branch base name (`origin/rally/reactive-parameters-runtime`) matches reality (verified at commit 69ed0e4). Constraints, ACs, ontology, and exit conditions are mutually consistent.

However, ac-10 — the rally-seam regression gate — has two falsifiability holes that survived (or were introduced by) the v1.1 revision. N1 is mechanical (wrong literal counts) and N2 is a bug in the F3 fix (rg patterns added without the corresponding files). Both are in the same gate that exists to catch this exact class of mistake on the implementation side. They block implementation-readiness because:

- An implementer following the spec literally will hit a false-negative gate failure (N1) before writing any code, which forces them to either edit the spec (introducing drift) or skip the gate (defeating its purpose).
- Three of ac-10's signature-fingerprint pins are silently dead (N2), so the gate can't catch the regressions it claims to catch.

Both fixes are one-line / few-line edits. With them in place, ac-10 is genuinely falsifiable and a fresh engineer can implement against the spec without ambiguity.

## Verdict

REQUEST_CHANGES

Two new HIGH findings introduced (or surfaced) by the v1.1 revision — both in ac-10's rally-seam gate, the centrepiece falsifiability mechanism. N1 is wrong baseline literals (mechanical fix); N2 is a partial F3 implementation that leaves three signature pins dead because their files aren't in the diff path list. The other nine findings (F1, F4, F5, F7-F10) are cleanly resolved; F2 and F3's revisions are on the right track but incomplete. A cycle-3 review can approve once the diff path list includes `chart_view.rs` + `crates/brightfield-sql/src/emit.rs` and the baselines read cfs_=12, cfs2_=16, rpw3_=16.
