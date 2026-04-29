# review-spec cycle 1 — cfs3 (cross-filtered selections, interactor variants)

**Date:** 2026-04-29
**Reviewer:** cold-context spec reviewer (general-purpose subagent)
**Spec:** orbit/specs/2026-04-29-cross-filtered-selections-interactors/spec.yaml v1.0

## Coverage table

```
| Card scenario / extension                                    | Interview decision | AC(s)              | Notes                                                                                  |
|--------------------------------------------------------------|--------------------|--------------------|----------------------------------------------------------------------------------------|
| Scenario 4 — clearing retracts contributor's predicate       | D1                 | ac-01, ac-02, ac-03| Coordinator API + UI gesture both exercised; subscriber re-execution covered by ac-01. |
| Scenario 5 — plot drives multiple selections                 | D3                 | ac-04              | Synthetic spec (two intervalXY interactors on one plot) — corpus has no native case.    |
| Scenario 5 — point selection on one channel                  | D2                 | ac-09              | Forward-compat type only; no chart-side path. Wiring deferred to rpw3 v3 / future card. |
| Scenario 6 — selections persist across param changes         | D5                 | ac-07              | Pins v2 lib.rs:464-466 behaviour as a regression test.                                  |
| Rally seam — propagate_param does not clobber selection_state | D5 + cross-cutting | ac-08, ac-10       | Two-call regression + signature-fingerprint gate against rally/reactive-parameters-runtime. |
| Brushable-binding metadata for chart_view construction       | D4                 | ac-05, ac-06       | Derived view + From conversion; v1's interactor_bindings shape preserved.               |
| cfs3_ test prefix                                            | D6                 | (constraint 6)     | No dedicated AC; enforced by the AC-id ↔ test-name 1:1 rule (constraint 6).             |
| Click-vs-drag discrimination at on_mouse_down                | D1 cross-cutting   | (impl note 7)      | Resolved at release-time (zero-area branch), not at brush-start. No AC; see findings.    |
| Escape-key clearing                                          | OUT-OF-SCOPE       | (impl note 8)      | Explicitly deferred. Visible in implementation_notes para 8.                            |
| Chart-side click-to-point                                    | OUT-OF-SCOPE       | (constraint 2)     | Explicitly deferred. Visible in constraint 2 + implementation_notes.                    |
```

## AC quality table

```
| AC    | Test name                                                 | Fixture concrete? | Assertion falsifiable? | 1:1 with test? | Notes                                                                                       |
|-------|-----------------------------------------------------------|-------------------|------------------------|----------------|---------------------------------------------------------------------------------------------|
| ac-01 | cfs3_ac01_clear_selection_removes_contributor             | yes (two contribs, named paths) | yes (count + Ok + non-empty batches) | yes | Strong — exercises both the state mutation and the dispatch loop's re-execution path.        |
| ac-02 | cfs3_ac02_clear_selection_unsubscribed_silent             | yes (two sub-cases) | yes (empty vec, no panic, list unchanged) | yes | Two-arm coverage (unknown name + unknown contributor on a known name).                       |
| ac-03 | cfs3_ac03_click_outside_active_brush_clears               | yes (Idle + zero-area + non-zero) | yes (recording dispatcher count + arg match) | yes | Three-arm coverage including the negative case (non-zero rejects → release). Excellent.       |
| ac-04 | cfs3_ac04_plot_drives_multiple_selections                 | yes (two bindings, IntervalXY + IntervalX) | yes (call count + per-call selection name + kind-compatibility filter) | yes | Strong; verifies the kind-compatibility filter (intervalX ignores y bounds).                  |
| ac-05 | cfs3_ac05_brushable_bindings_built                        | yes (one intervalXY + one panZoom; channels speed/delay) | yes (len==1 + field-by-field equality) | yes | **Path fixture wrong** — see Finding F1 (`interactor[0]` vs `interactor[intervalXY]`).         |
| ac-06 | cfs3_ac06_brushable_binding_to_brush_binding              | yes (named selection / kind / channels) | yes (every-field equality) | yes | Adequate; one-line conversion test, low coverage value but correct.                          |
| ac-07 | cfs3_ac07_param_change_preserves_selection                | yes (param P + selection brush + 1 mark) | yes (HashMap equality + SQL contains brush WHERE-fragment) | yes | Strong — pins both state preservation AND the SQL passthrough.                               |
| ac-08 | cfs3_ac08_propagate_param_does_not_clobber_selection_state | yes (populated state + 2 propagate calls) | yes (field-by-field including insertion order) | yes | The rally seam regression test. Two-call coverage handles re-entry path. Excellent.          |
| ac-09 | cfs3_ac09_brush_kind_point_constructs                     | yes (column "category", value "'Athletics'") | yes (variant identity + Expr body contains substrings + filter exclusion) | yes | Three-arm coverage — variant uniqueness, predicate shape, brushable-set exclusion.           |
| ac-10 | (gate)                                                    | partial — see F2/F4 | partial — see F2/F4    | n/a            | Glob check (4) trivially passes; surface-untouched check covers rpw3 only. See Findings.    |
```

## Rally seam check

The rally seam is shaped from two angles: (a) **rpw3 surface** (the freshly-shipped paired card) and (b) **v1/v2 surface** (the cfs / cfs2 surfaces this card stacks on top of).

**rpw3 surface protection (ac-10 step 3 fingerprint):**
- `propagate_param`, `topological_descendants`, `ParamDispatcher`, `SliderBinding`, `commit_slider_release`, `SliderState` — all six rpw3-shipped symbols are pinned by the `rg -F -e ...` line. Diff base is `origin/rally/reactive-parameters-runtime` (verified to exist on origin). The fingerprint is non-trivially falsifiable: any signature edit to those six symbols on this branch produces a hit and the gate fails. ✓

**v1/v2 surface protection (ac-10 missing entries):**
- The constraint-7 paragraph **commits** to leaving `propagate_selection` / `BrushBinding` fields / `brush_rect_to_predicate` / `on_mouse_up_with_dispatch` / `emit_query` / `interactor_bindings` schema untouched, but ac-10's signature-fingerprint diff (step 3) **does not include any of these**. Compare with rpw3's ac-16 which DID include `propagate_selection`, `BrushBinding`, `brush_rect_to_predicate`, `on_mouse_up_with_dispatch`, `emit_query`. The cfs3 gate is **under-broad on the v1/v2 axis**.
- See Finding F3 below — a HIGH finding.

**Source-untouched gate (ac-10 step 4):** the glob `crates/*/src/**cfs2*` matches **zero files** because cfs/cfs2/rpw3 tests live INLINE inside existing files (`crates/brightfield-engine/src/lib.rs`, `crates/brightfield-spec/src/analysis.rs`, etc.) — there are no `cfs2_*.rs` filenames in the tree. The check trivially passes regardless of edits. See Finding F4 below — HIGH.

**Constraint-vs-AC contradiction on `on_mouse_up_with_dispatch`:**
- Constraint 7 says "on_mouse_up_with_dispatch signature ... remain[s] untouched". Yet ac-04 description says "ChartView::on_mouse_up_with_dispatch with a Vec<BrushBinding> dispatches one propagate_selection per kind-compatible binding. The result type generalises to Vec<(selection_name, Vec<(usize, Result<...>)>)>" — i.e. the parameter type AND the return type both change. This is a direct contradiction. See Finding F5 below — HIGH.

## Findings

### HIGH (block approval)

**F1 — ac-05 fixture path is wrong (`interactor[0]`).**
The spec says ac-05's expected `interactor_path` is `root/plot[0]/interactor[0]`. The existing builder at `crates/brightfield-spec/src/analysis.rs:910` constructs paths as `root/plot[i]/interactor[<kind.wire_name()>]` (e.g. `interactor[intervalXY]`), confirmed at `analysis.rs:102`'s comment and `analysis.rs:1114` test fixture. Either:
- The AC fixture must be updated to match the existing convention (`root/plot[0]/interactor[intervalXY]`), or
- `build_brushable_bindings` must explicitly use a numeric index (introducing a divergence from `interactor_bindings`) — but this would be inconsistent with the spec's own claim that "v1's interactor_bindings shape and count are untouched" and breaks the From conversion's "preserves selection_name, contributor (= parent_plot), kind, and channels verbatim" expectation.

**F2 — ac-10 source-untouched glob is unfalsifiable.**
`git diff --stat origin/rally/reactive-parameters-runtime -- 'crates/*/src/**cfs2*' 'crates/*/tests/**cfs2*'` matches **zero files** under the current source layout. cfs/cfs2/rpw3 tests live inline inside existing files (`crates/brightfield-engine/src/lib.rs`, `analysis.rs`, `brush.rs`, etc.), not in separate `cfs2_*.rs` files. The check trivially passes for any change, including ones that delete or rewrite cfs/cfs2/rpw3 test functions. Suggested fix: use a function-grep diff such as `git diff origin/<base> -- <files>` piped through `rg -F 'fn cfs2_'` and require zero **modifying** hunks, OR add a count-based gate `rg -n '\bfn cfs2_' crates/ | wc -l` that asserts the count is exactly the pre-cfs3 baseline (16 for cfs2, plus the cfs1/rpw3 counts).

**F3 — ac-10 fingerprint diff omits v1/v2 surfaces.**
The signature-fingerprint diff (ac-10 step 3) only covers rpw3 symbols. Constraint 7's "v2 propagate_selection signature, BrushBinding fields, brush_rect_to_predicate signature, on_mouse_up_with_dispatch signature, emit_query/emit_query_with_passes signatures, analysis.{interactor_bindings,topological_order,dependency_dag} schemas remain untouched" is a behavioural commitment with no falsifiable check. rpw3's ac-16 covered both axes; cfs3's ac-10 should too. Add `pub fn propagate_selection`, `pub struct BrushBinding`, `pub fn brush_rect_to_predicate`, `pub fn on_mouse_up_with_dispatch`, `pub fn emit_query`, and (for the analysis schema) at least `pub struct InteractorBinding` to the rg `-e` list.

**F4 — Constraint 7 contradicts ac-04 on `on_mouse_up_with_dispatch`.**
Constraint 7 lists `on_mouse_up_with_dispatch signature` as untouched. ac-04 explicitly changes both its parameter type (`&BrushBinding` → `&[BrushBinding]`) and its return shape (`Vec<(usize, Result<...>)>` → `Vec<(String, Vec<(usize, Result<...>)>)>`). One of these must give. Either:
- Drop `on_mouse_up_with_dispatch signature` from constraint 7 (acknowledge it changes; then ac-10 must NOT pin its old signature — and the v2 cfs2_ac11 lifted-helper test stays green only if `commit_brush_release` is preserved as a single-binding wrapper, which implementation_notes para 5 already commits to), or
- Leave `on_mouse_up_with_dispatch` singular and have ChartView dispatch to multi via a NEW method (e.g. `on_mouse_up_with_dispatch_multi`). This avoids the contradiction but introduces a redundant entry-point.
The simpler resolution is the first: explicitly carve `on_mouse_up_with_dispatch` out of constraint 7 (and out of any v2 fingerprint added per F3).

### MEDIUM (REQUEST_CHANGES)

**F5 — ac-04 verification tests `commit_brush_release_multi`, not `on_mouse_up_with_dispatch`.**
ac-04's description names `ChartView::on_mouse_up_with_dispatch with a Vec<BrushBinding>` but its verification harness drives `commit_brush_release_multi` (the lifted helper). This is fine — the lifted-helper boundary is the precedent established by cfs2_ac11 — but the AC text should make it explicit that the GPUI surface is exercised via the lifted helper, the same way rpw3_ac12/ac-13 say so. As written, a reviewer who only reads ac-04's first sentence might expect a GPUI test, not find one, and conclude coverage is missing. One-line clarification.

**F6 — ac-03's "Mirrors commit_brush_release at chart_view.rs:181-206" is correct but the spec elsewhere implies brush.rs.**
The decisions doc (decisions.md) repeatedly refers to `crates/brightfield-ui/src/brush.rs:120-129` for `SelectionDispatcher` (correct — that's where the trait lives) and to `chart_view.rs:181-206` for `commit_brush_release` (also correct — confirmed at `chart_view.rs:181`). The spec ac-06 says "From<&BrushableBinding> for BrushBinding lives in brightfield-ui (BrushBinding's home crate)" — `BrushBinding` lives in `chart_view.rs:165-174`, which IS in brightfield-ui, so the crate-level claim is correct. The implementation_notes file-list paragraph 1 says `crates/brightfield-ui/src/brush.rs (add BrushKind::Point variant, point_predicate, SelectionDispatcher::clear method + Session impl)` — correct (BrushKind/SelectionDispatcher live in brush.rs; Session's impl of SelectionDispatcher is at brush.rs:131). All cross-references hold; this is just a sanity-check for the implementer.

### LOW (advisory)

**F7 — ac-09 substring assertion uses `'Athletics'` with quotes embedded in the literal.**
The verification says `point_predicate("category", "'Athletics'")` — implying the caller is expected to pass already-quoted SQL literal. That's consistent with `Predicate::Expr(String)` being raw SQL, and matches the IR convention at `crates/brightfield-sql/src/ir.rs:36` (Expr is opaque text). Worth confirming at implement-time that the helper's signature documents this contract — `point_predicate(column: &str, value: &str)` where value is "an already-formatted SQL literal" — so the next consumer (rpw3 v3's input-table widget per implementation_notes para 1) doesn't accidentally pass an unquoted string and produce a broken predicate. Mention in the helper doc-comment.

**F8 — Scenario 4 "or otherwise clearing it" — escape-key deferral is documented but not enumerated as a follow-up.**
implementation_notes para 8 says "Escape-key clearing is OUT OF SCOPE for this slice. ... Filed as a follow-up if the next sprint touches GPUI input." There is no actual filed memo or card. The interview does not name a memo path. If the deferral is intentional, fine; but the analogous Decision 5 deferral DOES name a memo (`orbit/cards/memos/2026-04-29-selection-domain-meaningfulness.md`). For symmetry, either drop the parenthetical or commit to a memo path.

**F9 — Click-vs-drag discrimination is moved from brush-start (interview Q1) to release-time (impl-notes para 7) without a decision-pack revision.**
The decisions doc Q1's resolution says "gate brush-start on a minimum drag distance, or defer brush-start until first mouse-move-while-down" — i.e. discrimination at start time. The spec moves it to release time via a zero-area check at `commit_brush_clear` (impl-notes para 7). Both are reasonable; the spec's choice is arguably cleaner (no UX feedback change during drag). But the design rationale shift is silent — implementation_notes simply states the new approach. For audit trail, a sentence in implementation_notes acknowledging the shift from the decision-pack's start-time gating to the release-time zero-area branch (and why: "user feedback during drag is unchanged") would help the implementer not waste cycles wondering which to follow.

**F10 — ac-09's "BrushKind::Point is NOT in the brushable-kinds set consumed by analysis::build_brushable_bindings" is a useful negative test, but the property lives in two crates.**
ac-09 sub-clause (c) asserts that `BrushKind::Point` interactors don't surface as brushable. But `analysis.rs` cannot reference `BrushKind::Point` directly (it lives in brightfield-ui; impl-notes para 3 introduces a mirror enum in brightfield-spec). The test must therefore use the spec-side mirror's `Point` variant when constructing a fixture, or — more practically — assert "an interactor of kind X (a kind without a brush rect) is filtered out", which is the same shape as ac-05's panZoom case. Recommend tightening ac-09(c) to name the spec-side mirror or reframe as "any non-{IntervalX, IntervalY, IntervalXY} kind is filtered out" with an explicit fixture (e.g. add a Point-kind interactor to the spec via the mirror enum and assert it's excluded). As written it's slightly under-specified about which crate's enum is exercised.

## Verdict

**REQUEST_CHANGES**

Three HIGH findings (F1 fixture path, F2 unfalsifiable source-diff glob, F3 missing v1/v2 surface fingerprint) and one HIGH structural contradiction (F4 between constraint 7 and ac-04's description) all need resolution before implementation. The contradictions and unfalsifiable gates are the same class of risk that the rally seam is designed to catch — leaving them in the spec means the eventual PR review may approve a regression. The MEDIUM and LOW items are advisory but several (F5, F9) are quick text edits that materially improve implementer clarity.

The core design is sound: 9 code ACs aligned 1:1 with `cfs3_acNN_*` test names, all 6 interview decisions reflected in constraints, all 6 card scenarios (including the three new ones) covered, deferrals (chart-side click-to-point, escape-key) are explicit. Resolve the HIGH findings — particularly F1 (a concrete fixture-string fix), F3 (extend the rg `-e` list), and F4 (carve `on_mouse_up_with_dispatch` out of constraint 7) — and a cycle-2 review should approve.
