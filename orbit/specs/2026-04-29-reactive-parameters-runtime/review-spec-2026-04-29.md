# Spec Review

**Date:** 2026-04-29
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-29-reactive-parameters-runtime/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 4 |
| 2 — Assumption & failure | content signals (cross-system seam, schema-adjacent, partial-failure regime) + MEDIUM finding (AC-id ↔ test-name divergence) | 3 |
| 3 — Adversarial | not triggered (no cascading or rollback risk) | — |

---

## Findings

### [MEDIUM] AC IDs and test-function names diverge — ac-12 packs two tests, ac-13 verifies with rpw3_ac14
**Category:** test-gap
**Pass:** 1
**Description:** The spec declares 14 acceptance criteria (`ac-01 … ac-14`) but the `verification` strings reference test names that drift from the AC index:

- `ac-12.verification` names **two** test functions: `rpw3_ac12_slider_on_mouse_up_dispatches` and `rpw3_ac13_slider_no_drag_no_dispatch`. They check distinct behaviours (mouse_up triggers; mouse_down alone does not) glued under one AC.
- `ac-13.description` covers the `InputKind::Slider` vocab flip, but its `verification` string names the test `rpw3_ac14_input_kind_slider_implemented`.
- `ac-14` is the gate AC (rally seam).

Net effect: tests `rpw3_ac12`, `rpw3_ac13`, `rpw3_ac14` exist, but the AC numbering says ac-12, ac-13, ac-14, with the latter being a gate, not a code AC. The orbit `/orb:audit` skill matches by AC id ↔ test prefix, so the traceability table will report ac-12 having two tests, ac-13 with a mismatched test name, and ac-14 (the gate) with none of the rpw3_ac\* code tests. The spec also claims "≥10 tests" — true once you count the bundled pair under ac-12 — but the constraint and the numbering disagree on whether there are 13 or 14 rpw3_ tests.

**Evidence:**
- spec.yaml line 79 (`rpw3_ac12_slider_on_mouse_up_dispatches and rpw3_ac13_slider_no_drag_no_dispatch`)
- spec.yaml line 84 (`rpw3_ac14_input_kind_slider_implemented in crates/brightfield-spec/src/vocab.rs`)
- spec.yaml line 18 (constraint: "Test prefix is rpw3_; ≥10 tests")
- decisions.md line 380 itemises rpw3_ac09 + rpw3_ac10 for the slider UI tests — the spec renumbered them silently to ac-12/ac-13.

**Recommendation:** Renumber so AC ids and test-function suffixes align 1:1. Either:
1. Split ac-12 into ac-12 (`rpw3_ac12_slider_on_mouse_up_dispatches`) and ac-13 (`rpw3_ac13_slider_no_drag_no_dispatch`), promote the vocab flip to ac-14 (renaming its test to `rpw3_ac14_input_kind_slider_implemented`), and renumber the gate to ac-15. Result: 15 ACs, 14 rpw3_ tests + one gate.
2. Keep 14 ACs but rename the bundled tests to `rpw3_ac12a_*` / `rpw3_ac12b_*` and rename `rpw3_ac14_input_kind_slider_implemented` to `rpw3_ac13_input_kind_slider_implemented`. Less disruptive but breaks the simple "ac-N ↔ rpw3_acN_*" rule the audit skill assumes.

Option 1 is preferred — it preserves the audit invariant and makes the slider UI breadth (two tests, not one) visible at the AC index.

### [MEDIUM] ac-08 partial-failure construction is under-specified for `propagate_param`
**Category:** test-gap
**Pass:** 1, deepened in 2
**Description:** ac-08 says "build a spec with one dot subscriber and one rect subscriber to the same param" and assert `results.len() == 2` with one Ok + one Err. The cfs2_ac08 sibling does this for `propagate_selection` over a `filterBy: $brush` edge. The param coordinator's subscriber lookup is `analysis.subscriber_graph[name]` keyed by params *consumed* by a mark's `data.from`/`filter_by`/option ParamRef. The construction "two marks subscribing to the same param" is not difficult, but the spec does not state:

(a) which subscription edge wires each mark to the param (filterBy? options ParamRef? a `from: q` whose query body references `$param`?);
(b) whether the rect mark is supposed to fail at the *emit* layer (no registered lowerer in `default_lowerers`) or at the *execute* layer (DuckDB error);
(c) what `param_state assertion` looks like — `current_params()[name] == new_value`?

For cfs2_ac08 these are clear because rect-with-no-lowerer has been the canonical "Err" subscriber for two prior slices and the verification cites `EngineError::EmitFailed { cause: UnsupportedMark }`. The decisions.md doc nails this (lines 178-180); the spec under review elides it.

**Evidence:**
- spec.yaml line 58-59 (ac-08 description and verification)
- decisions.md lines 178-203 (Decision 4 — names the emit-error path, registered-lowerer set, EngineError discriminant)
- crates/brightfield-engine/src/lib.rs:475-481 (the `continue` boundary it asserts against)

**Recommendation:** Tighten ac-08's verification to mirror cfs2_ac08:
- Name the failure mode: "rect mark with no entry in default_lowerers produces `Err(EngineError::EmitFailed { cause: UnsupportedMark })`; dot mark with registered lowerer produces `Ok(non-empty Vec<RecordBatch>)`".
- Name the param-state assertion: "`session.current_params().get(name) == Some(&new_value)`".
- State which subscription edge wires the marks (e.g. `data: { from: q, filterBy: $param }` on both, or `where: $param > 0` in the query expression).

### [MEDIUM] ac-14 gate is comprehensive but the verification command is ambiguous about "untouched"
**Category:** test-gap
**Pass:** 1, deepened in 2
**Description:** ac-14 enumerates eight rally-seam invariants ("propagate_selection, current_selections, SelectionDispatcher, BrushBinding, brush_rect_to_predicate, on_mouse_up_with_dispatch, emit_query/emit_query_with_passes signatures, update_param, and analysis.{subscriber_graph,topological_order,dependency_dag} schemas are untouched") plus the no-gpui invariant on brightfield-render. The verification is:

> Run cargo test … and cargo tree -p brightfield-render | rg gpui (must produce no match). All v2/cfs2 tests pass with zero modifications to their source.

This catches:
- Behavioural regressions in v2/cfs2 (covered by `cargo test`).
- Render-crate gpui leakage (covered by `cargo tree | rg gpui`).
- Source-edits to v2/cfs2 test files (only if the implementer notices that "zero modifications" means a `git diff --stat -- crates/*/tests/ crates/*/src/**rpw2*` or similar must be empty — but the spec does not say that).

It does **not** catch:
- A signature change to `propagate_selection`, `update_param`, or `emit_query` that the implementer accommodates by also editing v2/cfs2 tests (so they keep passing).
- A schema change to `analysis.subscriber_graph` that compiles because both producer and consumer change in lockstep.
- A schema change to `SelectionDispatcher` or `BrushBinding` that ripples into ChartView, where v2/cfs2 tests still pass but the rally seam is no longer honoured.

The "zero modifications to their source" clause is the actual seam check, but it is buried in prose without a falsifiable command. The implementer (or a future reviewer) would have to interpret it.

**Evidence:**
- spec.yaml lines 86-89 (ac-14)
- The constraint at line 17 ("Rally seam commitments — read-only or untouched: …") is the binding list, but ac-14's verification does not enumerate file/path checks.

**Recommendation:** Add a falsifiable command to ac-14's verification:
```
git diff origin/main -- \
  crates/brightfield-engine/src/lib.rs \
  crates/brightfield-spec/src/analysis.rs \
  crates/brightfield-ui/src/brush.rs \
  crates/brightfield-ui/src/chart_view.rs \
  | rg -F 'pub fn propagate_selection' or 'pub fn update_param' or \
       'pub trait SelectionDispatcher' or 'pub struct BrushBinding' or \
       'pub fn brush_rect_to_predicate' or 'pub fn emit_query' or \
       'pub fn on_mouse_up_with_dispatch'
```
…must produce no signature-line hits. Or, simpler: enumerate the symbol signatures verbatim in a "rally seam fingerprint" block in the spec, and assert the implementer's PR diff does not change any of them. The cfs2 ac-13 corpus regression gate is the precedent — it has a name (the iteration test) and a deterministic outcome.

A weaker but adequate alternative: add a second cargo command that compiles a tiny external test crate against the rally-seam public API (re-importing the listed symbols by name), so a signature change *does* break the test even if the signature-bearing crate's tests are updated.

### [LOW] Implementation Notes do not name the dedup invariant in algorithmic form
**Category:** missing-requirement
**Pass:** 1
**Description:** The implementation note for the chained walk (spec.yaml line 97) describes the algorithm in five numbered steps. It states "skip indices already in dispatched, dedup in-level (existing sort+dedup), then dispatch via emit_query and append (mark_idx, Result) to results, inserting into dispatched on dispatch." Reading this end-to-end, a fresh agent can write the body, but the dedup invariant — "a mark appears at most once in `results`, at the topologically-earliest level whose subscriber list contains it" — is not stated as a property. It has to be inferred from the order of operations.

This matters because the test for ac-05 (`rpw3_ac05_propagate_param_first_level_wins_dedup`) asserts `at most once`, but does not assert *which* level. An implementation that dispatches at the deepest level (last-level-wins) would *also* produce at-most-once and silently pass ac-05. Decision 3 explicitly chose first-level-wins; the AC needs to reflect that choice.

**Evidence:**
- spec.yaml line 44 (ac-05: "asserts the mark appears in the result vec at most once")
- decisions.md lines 144-164 (Decision 3 chose first-level-wins explicitly, distinguishing from option B last-level-wins)
- spec.yaml line 97 (implementation note describes the algorithm but not the level-membership invariant)

**Recommendation:** Strengthen ac-05's verification to assert *both* properties:
1. The mark appears exactly once in `results`.
2. Its position in `results` corresponds to A's level (the first-level position), not B's. Concretely: if B has another sole-subscriber `m_B`, then `m_AB`'s entry in `results` precedes `m_B`'s (since `topological_descendants(A)` yields `[A, B]` and dispatch iterates in that order, with the mark inserted at A's level).

A cleaner restatement: "rpw3_ac05_propagate_param_first_level_wins_dedup constructs A → B and a mark m_AB whose query references both $A and $B, plus a mark m_B whose query references only $B; propagates A; asserts (a) results contains m_AB exactly once and (b) m_AB's index in results comes before m_B's index."

Add a one-line invariant to the implementation note: `// invariant: each mark_idx appears in results at most once, at the topologically-earliest level in the walk where its subscription edge first matches`.

### [LOW] ac-11/ac-12 pure-helper boundary is sound, but ac-12's "real GPUI mouse events" is not testable without a GPUI test harness
**Category:** failure-mode
**Pass:** 2
**Description:** ac-11 verifies `commit_slider_release` against a recording-dispatcher double — pure, fast, no GPUI. ac-12 says "Slider widget on_mouse_up triggers commit_slider_release; mouse_down without mouse_up never triggers it" and the verification names two UI tests "against a recording dispatcher double." The boundary between "Slider widget" and "commit_slider_release" matters: if the test directly invokes `commit_slider_release` after manually transitioning the widget's drag state to e.g. `Released { value: x }`, the test does not actually exercise the GPUI mouse-event handler (the mouse handler closure that sits on a `gpui::Element` and calls `commit_slider_release` on a real release). cfs2 had this same boundary problem (cfs2_ac11) and resolved it by accepting that "if wiring through a real Session in tests is impractical, an equivalent integration test driving on_mouse_up against a real Session in a headless harness satisfies the AC" — i.e. it explicitly allowed the lifted-helper test to satisfy the AC.

The current ac-12 verification does not name this fallback. A literal reading would require simulating GPUI mouse events end-to-end, which has no precedent in the repo.

**Evidence:**
- spec.yaml line 78-79 (ac-12 description and verification)
- crates/brightfield-ui/src/chart_view.rs:128-157 (`on_mouse_up_with_dispatch` is the real GPUI surface; cfs2 tested it via `commit_brush_release` lifted helper at brush.rs:181-206)
- cfs2 spec ac-11 verification text explicitly allows the test-double pattern as the AC-satisfying form.

**Recommendation:** Restate ac-12's verification to mirror cfs2_ac11 and clarify the lifted-helper boundary:
> "rpw3_ac12_slider_on_mouse_up_dispatches and rpw3_ac13_slider_no_drag_no_dispatch construct a SliderState (or equivalent UI state struct) with a recording-dispatcher double and invoke commit_slider_release after a state transition that simulates the mouse event sequence (mouse_down → mouse_up; mouse_down only). The first asserts one dispatch with the expected (name, value); the second asserts zero dispatches. End-to-end GPUI event testing is out of scope; the lifted-helper coverage matches the cfs2_ac11 precedent."

This makes the helper boundary the AC's verification harness and removes the implicit dependency on a GPUI test rig.

### [LOW] Decision 2 case (iii) deferral is documented, but the spec does not surface a guard against accidental regression
**Category:** missing-requirement
**Pass:** 2
**Description:** Decisions 2 and the implementation notes (line 100) explicitly defer the computed-param case (`ParamNode::FromQuery`). The deferral is sound — there is no AST surface, no corpus example. But the spec adds no negative test or guard ensuring a future implementer does not silently extend the walk to handle it half-correctly.

This is a low-risk gap because:
- The spec adds no `ParamNode::FromQuery` variant.
- The walk consumes `param_state` only; it has no plumbing to derive values from query results.
- Adding case (iii) requires AST changes, which would surface in PR review.

But a one-line guard test would close it cheaply: assert that `propagate_param` does *not* mutate `param_state` for any param other than the explicitly named one (i.e. the walk is read-only for downstream params). This is a directly testable property: walk a chain `A → B`, call `propagate_param("A", _)`, assert `current_params()["B"]` is unchanged from its initial state.

**Evidence:**
- spec.yaml line 100 (deferral document only; no test)
- decisions.md lines 100-106, 125 (Decision 2 commits to "Topological re-execution against full param_state; computed-param case deferred")

**Recommendation:** Add ac-15 (after renumbering per finding 1) or fold into ac-04: "rpw3_ac\*\_propagate_param_does_not_mutate_downstream_params constructs A → B with an initial param_state where `B = b0`, calls propagate_param("A", new_a), asserts current_params()["B"] == b0". This locks down the case-(iii) deferral as a behavioural property, not just a comment.

### [LOW] Cross-coordinator symmetry is preserved structurally but ac-04 does not assert the selection_state passthrough
**Category:** test-gap
**Pass:** 2
**Description:** Implementation note line 98 says "the new walk hands the same selection_state slice to every level, so chained re-execution honours the active brush." This is the *real* cross-coordinator symmetry property — without it, a brush set before a slider change would be lost during the chained walk. None of ac-01..ac-14 asserts this directly. ac-04 (`rpw3_ac04_propagate_param_chained_walk`) only asserts the result vec contains the B-subscribing mark.

If an implementer writes the walk but accidentally short-circuits selection_predicates to None at the second level (e.g. via a mis-scoped variable), every test in this spec passes. The bug surfaces only when a user brushes, then drags a slider whose subscriber chain re-executes — a regression that the cfs2 corpus integration test (cfs2_ac12) doesn't cover because it does not exercise chained param walks against a live selection.

**Evidence:**
- spec.yaml line 98 (implementation note asserts the property)
- spec.yaml lines 38-39 (ac-04 — does not test selection_state passthrough)
- crates/brightfield-engine/src/lib.rs:467-472 (existing direct-only path threads `selections_ref` once; the chained walk has more places where this can be lost)

**Recommendation:** Add a sub-AC or extend ac-04: "with selection_state pre-populated for selection $brush, propagate_param("A", v), assert that the emit_query call for the B-subscribing mark received the same selection_predicates slice as the A-subscribing mark." A simpler form: assert the SQL emitted at level B contains the selection's WHERE clause. This makes the "every level honours active brush" property falsifiable.

---

## Honest Assessment

The plan is sound at the architectural level. Decisions 1-6 are tightly motivated, the rally seam is well-bounded, the cross-coordinator symmetry with cfs2 is clear, and the slider end-to-end shape mirrors the brush precedent. The slice has narrow blast radius — one method body, one new helper, one new file, one vocab flip.

The blocker is bookkeeping discipline at the AC layer. The AC-id ↔ test-name divergence (finding 1) is the kind of thing that breaks the orbit audit downstream and makes the review-pr cycle painful. The partial-failure AC (finding 2) is the strengthened version of the v2 review's MEDIUM, and under-specifying it now repeats that mistake. The rally-seam gate (finding 3) is comprehensive in prose but its falsifiability is one cargo command short of being deterministic.

The strongest-leverage fix is renumbering ACs to align with test names (finding 1) and adding a falsifiable seam check to ac-14 (finding 3). Findings 4-7 are tightening rather than blocking — the spec would still ship a working slice without them, but they sharpen the "did the implementer write the algorithm I described, or just one that passes the tests" gap.

Recommend REQUEST_CHANGES to address findings 1, 2, and 3 before implementation begins. Findings 4-7 can be folded in opportunistically or deferred to a v3.1 amendment. The biggest hidden risk is finding 7 (selection_state passthrough during chained walk) — it is the single regression the rally seam most cares about, and no AC in the spec catches it.
