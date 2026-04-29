# Spec Review — Cycle 2

**Date:** 2026-04-29
**Reviewer:** Context-separated agent (forked, cycle 2)
**Spec:** orbit/specs/2026-04-29-reactive-parameters-runtime/spec.yaml (v1.1)
**Prior review:** orbit/specs/2026-04-29-reactive-parameters-runtime/review-spec-2026-04-29.md (cycle 1, REQUEST_CHANGES)
**Verdict:** APPROVE

---

## Review Depth

| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 0 |
| 2 — Assumption & failure | content signals (cross-system seam, partial-failure regime, schema-adjacent helper) | 0 |
| 3 — Adversarial | not triggered (no cascading or rollback risk; revisions tightened the falsifiability of every cycle-1 finding) | — |

---

## Cycle-1 finding-by-finding resolution

### [MEDIUM-1, cycle 1] AC ids ↔ test names diverged — RESOLVED

The v1.1 spec adopts cycle-1's preferred Option 1: 15 code ACs (`ac-01..ac-15`) + 1 gate (`ac-16`), with each AC's `verification` naming exactly one `rpw3_acNN_*` test function whose suffix matches the AC index. Spec-line evidence:
- `ac-12.verification` → `rpw3_ac12_slider_on_mouse_up_dispatches` (only this test).
- `ac-13.verification` → `rpw3_ac13_slider_no_drag_no_dispatch` (split out cleanly, separate AC).
- `ac-14.verification` → `rpw3_ac14_input_kind_slider_implemented` (vocab promoted to a code AC).
- `ac-15.verification` → `rpw3_ac15_propagate_param_does_not_mutate_downstream_params` (case-iii guard, see LOW-3 below).
- `ac-16` is the gate.

The constraint at line 19 now says "15 code ACs + 1 gate (ac-16)" and "AC ids align 1:1 with test-function suffixes (rpw3_acNN_*)" — explicit, falsifiable, and consistent with the `/orb:audit` skill's prefix-match assumption. The `metadata.review_revisions` block (lines 220-226) pins the changes back to cycle-1 findings 1-7. Resolved.

### [MEDIUM-2, cycle 1] ac-08 partial-failure under-specified — RESOLVED

The revised ac-08 (lines 57-68) names every previously-missing detail:
- Failure mode: `Err(EngineError::EmitFailed { cause: UnsupportedMark, .. })`.
- Subscription edge: `data: { from: q, filterBy: $param }` on both marks.
- Lowerer registration: "registers only the dot lowerer via `default_lowerers` (rect lowerer absent)".
- param-state assertion: `session.current_params().get("param") == Some(&new_value)`.
- Result-vec assertions are itemised (a)–(d).
- The "exercises the previously-unreachable Err branch" line ties the strengthening back to the cycle-1 v2 review's MEDIUM that this AC closes.

This is bit-for-bit the cfs2_ac08 sibling pattern with the param coordinator substituted in. Resolved.

### [MEDIUM-3, cycle 1] ac-16 gate not falsifiable for "untouched" rally seam — RESOLVED

The revised ac-16 (lines 121-144) lists four mechanical checks (1)–(4), each with a deterministic outcome:
1. `cargo test -p brightfield-engine -p brightfield-spec -p brightfield-ui -p brightfield-render` (behavioural).
2. `cargo tree -p brightfield-render | rg gpui` must produce zero matches (no-gpui invariant).
3. `git diff origin/main` over the four touched files, piped through `rg -F` against seven enumerated public signatures (the seam fingerprint), must produce zero hits.
4. `git diff --stat origin/main -- 'crates/*/src/**rpw2*' '...cfs*'` must be empty (v2/cfs2 test sources untouched).

Check (3) is the cycle-1 reviewer's recommended seam-fingerprint diff verbatim. It catches "implementer changed signature and updated v2/cfs2 tests in lockstep" — the highest-risk seam regression. Check (4) catches the simpler "edited the v2 tests to make them pass" failure mode. Together they make the gate falsifiable in a way prose alone could not. Resolved.

### [LOW-1, cycle 1] Dedup invariant not stated as first-level-wins — RESOLVED

ac-05 (lines 43-45) now asserts ordering, not just count: m_AB's index in `results` is strictly less than m_B's. The implementation note (lines 152-160) restates this as a code-side invariant: "each mark_idx appears in results at most once, at the topologically-earliest level in the walk where its subscription edge first matches (first-level-wins)." A last-level-wins implementation would now fail ac-05 deterministically. Resolved.

### [LOW-2, cycle 1] Lifted-helper boundary not made explicit for ac-12/13 — RESOLVED

Both ac-12 (lines 86-95) and ac-13 (lines 99-104) now reference the cfs2_ac11 lifted-helper precedent verbatim, name `commit_slider_release` as the test entry point, and explicitly mark "End-to-end GPUI event simulation is out of scope". The implementation notes (lines 150, 187) reinforce this by binding `commit_slider_release` to its `commit_brush_release` analogue at chart_view.rs:181-206 (the exact lifted helper cfs2_ac11 used). Resolved.

### [LOW-3, cycle 1] Case-iii deferral has no behavioural guard — RESOLVED

The new ac-15 (lines 111-119) is exactly the cycle-1 reviewer's proposed test: build `A → B` with B at `b0`, propagate A, assert `current_params()["A"] == new_a` AND `current_params()["B"] == b0`. This locks the case-iii deferral as a behavioural property of the walk, not just a comment. The implementation note at line 163 also names ac-15 as the lock. Resolved.

### [LOW-4, cycle 1] selection_state passthrough not asserted — RESOLVED

The revised ac-04 (lines 38-40) explicitly asserts the selection_state passthrough at every level: "the selection_state slice handed to emit_query at level B is the same one handed at level A." The verification (line 40) prescribes pre-populating `session.selection_state` with a brush predicate, propagating A across the two-level DAG, and asserting both marks' emitted SQL contains the brush's WHERE-clause fragment. The implementation note (lines 152-160) reinforces this with the "captured before the loop, never re-read inside the loop" rule and a code-review-visible invariant. The constraint at line 17 also pins selection_state passthrough to "every level of the walk receives the same selections_ref slice".

This was the cycle-1 reviewer's flagged "highest-leverage hidden regression risk" — and it is now the most heavily-asserted property in the spec. Resolved.

---

## Cycle-2 fresh review (independent pass)

I read the spec independently before consulting cycle 1, then cross-checked. My independent findings are listed below.

### Pass 1 — Structural scan

- **Goal clarity**: line 1-9 names the slice precisely (chained walk + slider), the user-visible behaviour ("when an analyst drags a slider"), and the seam to cfs2. No drift from card 0005's scenario 4 + scenario 2.
- **Constraints**: 7 constraints, each a bounded commitment. The constraint at line 18 enumerates the rally-seam read-only/untouched set in the same order as ac-16's check (3). No contradictions with the AC body.
- **AC count and coverage**: 15 code ACs cover (a) the new pure helper `topological_descendants` (ac-01..ac-03 — simple chain, corpus chain, leaf root), (b) the walk's six behavioural properties (ac-04 chained + selection passthrough, ac-05 first-level dedup, ac-06 descendants-only scope, ac-07 leaf no-op, ac-08 partial failure, ac-15 case-iii deferral), (c) the dispatcher trait (ac-09), (d) the slider widget (ac-10 binding, ac-11 commit helper, ac-12 mouse_up dispatch, ac-13 no-drag-no-dispatch), (e) the vocab flip (ac-14). The gate (ac-16) is the rally-seam regression check. No AC is orphaned; no scenario from card 0005 is unrepresented at the runtime layer.
- **Implementation notes**: the seven notes at lines 147-165 each name a file and line range. The walk algorithm at lines 152-160 enumerates 5 numbered steps with two named invariants. A fresh implementer can write the body without a second pass.
- **Ontology schema**: 7 fields, each tied to a file and signature. No abstract surface.
- **Exit conditions**: 5 conditions, each falsifiable.

No structural findings.

### Pass 2 — Assumption & failure-mode probe

I probed three high-risk areas because of content signals (cross-coordinator seam, schema-adjacent helper, partial-failure regime).

**Probe A: does the spec assume `topological_descendants` matches `analysis.topological_order`'s projection?** ac-02 (line 30) says "asserts the returned ordering matches the analysis.topological_order projection over descendants." This is the right anchor — `topological_descendants` is required to be a *projection* of the existing topo order, not an independent traversal. If the implementer writes a fresh DFS, ac-02 will catch the divergence. No finding.

**Probe B: does ac-04's "same selection_state slice" survive the borrow-checker honestly?** The implementation note (line 158) prescribes `let selections_ref: &[Predicate] = self.selection_state.as_slice() ONCE before the loop`. This contrasts with the existing `propagate_selection` body (engine/lib.rs:305-312), which clones `param_state` to escape the borrow-checker. The slice in `propagate_param`'s walk needs the same care — but `selection_predicates_for_emit()` (engine/lib.rs:467) already returns owned data the existing v2 propagate_param body holds across the loop. The "capture once before the loop" wording is honest about ownership; ac-04's verification ("both emit_query invocations received Some(predicates) with the same predicate set") is implementable without runtime aliasing trouble. No finding.

**Probe C: does ac-08's "rect lowerer absent" actually trigger `EmitFailed { cause: UnsupportedMark }`, given the registered-lowerer set may have shifted since cfs2_ac08?** I traced: cfs2_ac08 succeeds today with rect-no-lowerer producing exactly `EmitFailed { cause: UnsupportedMark }` (decisions.md line 178-179 cites this and the canonical Err discriminant). The dispatch in propagate_param at engine/lib.rs:475-481 is structurally identical to propagate_selection's at engine/lib.rs:319-324. The rect-without-lowerer pattern is the canonical Err and ac-08 names it correctly. No finding.

### Pass 3 — Adversarial

Pass-3 triggers on cascading-failure or rollback risk. The slice is small (one method body, one new file, one helper, one vocab flip); rollback is `git revert` of the rally branch with no schema migration. ac-16's check (4) gives the implementer a deterministic "I haven't touched the rally seam" signal mid-implementation, so the failure mode of "v2/cfs2 broken in lockstep" is caught early. Pass 3 not triggered.

---

## New findings (cycle 2)

None.

The spec is unusually well-bounded for a runtime change: the rally seam is fenced with a falsifiable signature-fingerprint diff, the chained walk has six independent behavioural ACs (one of which is the case-iii guard), the slider is shipped end-to-end with a lifted-helper boundary that mirrors cfs2_ac11 verbatim, and the partial-failure AC closes the v2 review's known gap by exercising the previously-unreachable Err branch.

The two assertions I would have added in a hypothetical cycle 3 — both LOW and both already partially covered — are:

- A test asserting that `topological_descendants` is robust to a param name not present in the analysis (returns `[root]` or empty? the spec's behaviour is "root included as first element" per line 149, but ac-03 only covers leaf-with-zero-edges, not unknown-name). This is a defensive test, not a slice-blocking gap.
- An assertion that the walk handles a self-referential edge (e.g. `wnba-shots.yaml`'s "widget filters itself by the selection it contributes to" — see analysis.rs:391-401). The DAG construction skips self-edges (analysis.rs:393), so the walk inherits cycle-freedom by construction; this is documented in decisions.md line 374. Adding a regression test would be tightening, not blocking.

Both are below the bar for REQUEST_CHANGES — they are the kind of follow-up a v3.1 amendment or the next slice's discovery interview would absorb.

---

## Honest assessment

Cycle 1 raised three MEDIUM findings and four LOW findings. v1.1 addresses all seven, with the MEDIUMs closed by structural changes (renumbering, named EngineError discriminant, falsifiable seam diff) and the LOWs closed by tightening AC verifications (ordering assertions, lifted-helper precedent citation, case-iii guard, selection_state passthrough). The metadata's `review_revisions` block (lines 220-226) maps each revision back to its cycle-1 finding number — a clean traceability gesture that makes the cycle-2 verification trivial.

The spec is now in a state where:
- An implementer can write the chained walk in one pass against the line-numbered notes and 5-step algorithm.
- A reviewer can run the four `ac-16` mechanical checks without interpretation.
- The `/orb:audit` skill will see 15 ACs, 15 `rpw3_acNN_*` test functions, and the gate, with no traceability gaps.
- The single highest-leverage regression risk (selection_state passthrough at every level of a chained walk) is now the spec's most heavily-asserted property.

Verdict: **APPROVE.** Proceed to implementation. No residual findings worth blocking on; the two LOW-LOW observations in cycle 2 are tightening-class, fold opportunistically or defer.
