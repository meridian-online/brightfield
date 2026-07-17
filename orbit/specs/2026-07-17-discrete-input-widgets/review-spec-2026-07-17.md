# Spec Review — Discrete input widgets (menu / radio / checkbox)

**Date:** 2026-07-17
**Reviewer:** Context-separated agent (fresh session, cold read from disk)
**Spec:** orbit/specs/2026-07-17-discrete-input-widgets/spec.yaml (v1.0, cards: 0024)
**Supporting artifacts read:** tabletop.md (same dir), orbit/cards/0024-discrete-input-widgets.yaml

---

## Review Depth

| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 3 (gate-AC check clean; anchor sweep found 2 anchor errors + 1 scope-claim error) |
| 2 — Assumption & failure | content signals (engine API surface, hot-reload/watcher boundary, machine gates) | 4 |
| 3 — Adversarial | AC-vs-decision contradiction + reload cascade found in Pass 2 | 2 |

Gate-AC deterministic check (Pass 1, rule 5): diw-ac12 and diw-ac15 are the two `ac_type: gate` ACs; both descriptions are non-empty, non-placeholder, and well over 20 chars. PASS.

## Anchor Verification (task 1)

Every file:line anchor in the spec was checked against the working tree at HEAD (`fef54fe`). **Verified exact** (line-precise):

- `slider.rs:39` ParamDispatcher trait; `:20-25` the false "no coordinator change required" header; `:91` as_param-else-None; `:191` hardcoded `SpecValue::Float` dispatch; `:283` RecordingDispatcher; `:314` input_fixture; `:11-19` Decision-2-case-iii deferral note
- `brightfield-engine/src/lib.rs:685` propagate_param; `:913` `#[cfg(test)]` on execute_raw_sql; `:943` profile_sources (`&self`, doc confirms launch-session + throwaway-session safety)
- `crossfilter.rs:213` slider_bindings field; `:821` commit_slider; `:847` apply_slider; `:1192` coordinator_has_live_surface (disjunction exactly as described)
- `layout.rs:173` the `Component::Input` arm; DEFAULT_INPUT_WIDTH/HEIGHT = 200.0/32.0 (`:47/:49`)
- `vocab.rs:253` Menu→Unimplemented; `:366` slw_ac08 test (currently asserts `implemented == vec![Slider]`, exactly what diw-ac10 says must be updated)
- `parse.rs:1162` the options-bag catch-all arm; `parse.rs:1116/1123` UnknownName with `NameSurface::Input` (the variant exists, error.rs:44)
- `ast.rs:320` Input struct (as_param/from_source/filter_by/options as claimed); `SpecValue::Array` exists (ast.rs:470) and `spec_value` converts YAML sequences to it — the literal `options:` list rides the bag as claimed
- `binding.rs:92` BindingMode::Interpolated; `emit.rs:195` spec_value_to_sql_literal; `:200` String quote-doubling (comment cites card 0014 ac-02 — the tabletop's String-param kill condition is pre-verified shipped)
- `chart_view.rs:101` sliders field; `:189-204` slider hosting block; `:205` legends block
- `main.rs:758` slider-clamp loop; `:1080` slider placements; `:1115-1130` bounding-box fold; `:1410` HeadlessDump arm with `:1428-1445` resting-slider composition; `:1569` profile-failure warning loop; `:1627` registration-order comment; `:1630` slider_bindings; `:1643` CrossfilterCoordinator::new callsite
- `scene.rs:419-422` SLIDER_* constants; `:429` render_slider; `text.rs:79` draw_text; `slider_element.rs:28-38` the twin constants
- `examples/param-slider.yaml:30-35` the input block; 30 example yamls confirmed (30/30 claim arithmetically right); `dump_seam.rs` aws_ac01/aws_ac07 exist as named
- Bonus confirmation: `analysis.rs:37` already types `InputKind::Menu → WidgetOutputType::ScalarString` — the analysis layer expects exactly the param-valued String menu this spec builds

**Anchor errors found:** diw-ac15's "serialise module" (Finding M1), diw-ac06's "dump/embed paths" callsites (Finding L1). Card-only slip: card 0024 cites propagate_param at "brightfield-ui/src/lib.rs:685" — it lives in brightfield-**engine**; the spec has it right (Finding L4).

---

## Findings

### [HIGH] H1 — Bare `style: checkbox` is contradicted three ways: decisions_locked gives it a default, diw-ac01 drops it, diw-ac14 degrades it

**Category:** constraint-conflict
**Pass:** 3
**Description:** decisions_locked #6 says: *"checkbox requires exactly two options (default [true, false] when `options:` absent)"* — i.e. `input: menu, style: checkbox, as: $flag` with **no** `options:` is a legitimate, working checkbox. But diw-ac01 says MenuBinding::from_input *"returns None when neither options nor a derivable source exist"* and its verification list pins an **"empty-options None"** test with no style carve-out — under that rule the bare checkbox yields no binding, i.e. a silently dropped widget (the exact class diw-ac14 forbids: "No degrade path panics, **drops the widget silently**…"). And diw-ac14's own degrade list ("style: checkbox with ≠2 options → menu presentation") reads a zero-option checkbox as a *third* behaviour: degrade to a menu over an empty list. Card scenario 4 ("Checkbox toggles a boolean param … bound to a param consumed in a WHERE flag") is the flagship shape most naturally authored with no `options:` — the spec's tests as written would pin it dead.
**Evidence:** spec.yaml decisions_locked #6 vs diw-ac01 description + verification ("empty-options None") vs diw-ac14 description; card 0024 scenario 4; tabletop Trade-offs ("default `[true, false]`").
**Recommendation:** Amend diw-ac01 to enumerate the checkbox arm explicitly: *for `style: checkbox` with `options:` absent, from_input synthesises the default [Bool(true), Bool(false)] and returns a binding; the "empty-options None" test applies to style menu/radio only, and a checkbox-default test pins the synthesised pair.* Also state the reconciliation ordering (does the prepend-on-absent-default rule run before or after the exactly-two check? a param default `"maybe"` prepended onto [true,false] makes three options → degrade — pick and pin one order), and define which of the two options renders as "checked" (natural: the option equal to the current param value is the checked state; the click dispatches the other).

### [HIGH] H2 — Hot-reload behaviour of a hosted menu is undefined; the substrate contradicts the spec's staleness story, and the "both paths" verification can pass while the user sees silently stale options

**Category:** failure-mode / test-gap
**Pass:** 3
**Description:** The watcher (`spawn_spec_watcher`, main.rs:1190) re-runs the full pipeline off-thread but **swaps plot scenes only** (matched by path), refreshes sidebar profiles via the dedicated `set_profiles` tap, and gates chrome changes to "restart to apply" via `ChromeSnapshot`/`chrome_divergence` (main.rs:567/650). ChromeSnapshot carries title, legends, legend_bindings, and plot_render_meta — **no input-widget slice**, and `same_layout` checks plots only. Consequences the spec never resolves: (a) a spec edit that changes a menu's `options:`/`style:`/`from:` hot-swaps the plot scenes while the hosted widget and the coordinator's menu_bindings stay at launch state — silently stale, no warning, no restart-to-apply gate (widgets have no set_profiles-equivalent tap, and the adjacent "coordinator refresh on hot reload" chore is explicitly NOT folded, per risks); (b) out_of_scope's rationale *"live options re-query (external data change shows on reload, profile staleness parity)"* is **false** as written — profiles refresh on reload because they have a tap; resolved options would be recomputed inside `run_pipeline` and then discarded with the Dashboard; (c) diw-ac04 requires resolution "on BOTH the launch path and the watcher/reload rebuild path", but since both paths share the assembly (`run_pipeline` wraps `build_everything`), an assembly-level test passes trivially while saying nothing about what the watcher path *surfaces* — the truncation/failure Log Warnings must cross the Send boundary in `run_pipeline`'s return the way `Vec<SourceProfile>` does, or the watcher path warns nothing; (d) diw-ac14's "never double-warns **on reload**" is verified by "assembly-level tests", which cannot exercise the watcher loop at all. This is the repo's recurring green-tests/stale-canvas class, on the reload surface instead of the commit surface.
**Evidence:** main.rs:1252-1310 (scene-swap-only reload body, `same_layout` over plots only), main.rs:567-591 (ChromeSnapshot fields — no widgets), main.rs:672 (`run_pipeline` returns `(Dashboard, ChromeSnapshot, Vec<SourceProfile>)` — profiles cross the boundary, nothing else does), spec risks bullet 4 ("diw-ac04 covers both paths explicitly" — it doesn't, as verified), out_of_scope bullet 3.
**Recommendation:** Pick and pin one honest reload posture: **(preferred, cheapest)** extend ChromeSnapshot with a widget slice (per-input: rect, style, binding param, resolved options list) so any menu-affecting edit gates to "restart to apply" — the exact colorScheme/inset/titles precedent — with a `chrome_divergence` unit test; then correct out_of_scope bullet 3 to say an options-affecting change requires restart (parity with sliders, which are equally launch-fixed today). Additionally, amend diw-ac04's verification to name the watcher-side warning transport (resolution warnings returned from `run_pipeline` like profiles, surfaced in the same block as main.rs:1325's profile warnings) and move the "no double-warn on reload" assertion to a test that actually exercises that transport (or re-scope it to "per assembly pass").

### [MEDIUM] M1 — diw-ac15's machine gate is not implementable as written: there is no "serialise module", and the file that holds serialise_spec is the same file two ACs add tests to

**Category:** constraint-conflict / test-gap
**Pass:** 2
**Description:** serialise_spec is a function at **parse.rs:1412** — not a module, and not its own file. The constraint says serialise_spec is "byte-untouched by the same gate over **its file**" and diw-ac15 says "brightfield-spec's **serialise module** is byte-untouched". But parse.rs also hosts the crate's parse test module (`#[cfg(test)]` at parse.rs:1741, 37 tests), which is where diw-ac10's two new parse tests (`style:` lands verbatim in the options bag; `input: radio`/`input: checkbox` → UnknownName) and diw-ac15's own serialise round-trip test would naturally land. An empty-`git diff` gate over parse.rs therefore contradicts diw-ac10 and diw-ac15's round-trip test. (Verified mitigating fact: no existing parse.rs test mentions "menu", so the vocab flip itself forces no parse.rs edit — only the *new* tests collide.)
**Evidence:** `grep -n "fn serialise_spec" crates/brightfield-spec/src/parse.rs` → 1412; `#[cfg(test)]` at parse.rs:1741; diw-ac10 verification ("parse unit tests"); diw-ac15 verification ("Empty-diff check … + round-trip test").
**Recommendation:** Reword both sites to name the real layout ("serialise_spec, parse.rs:1412") and re-scope the gate to something machine-checkable: either (a) require the new diw-ac10/ac15 tests to live in a separate integration-test file (e.g. `crates/brightfield-spec/tests/`) and keep the empty-diff gate over `parse.rs`, or (b) drop the file-diff gate for brightfield-spec and rely on the round-trip test as the falsifier (the brightfield-sql crate-level empty-diff gate is unaffected and stays). State the choice in the constraint so the implementer can't quietly weaken it.

### [MEDIUM] M2 — diw-ac08, the load-bearing silent-no-op defence, names a function (`commit_menu`) that cannot be driven by a headless default-gate test

**Category:** test-gap
**Pass:** 2
**Description:** diw-ac08 says the live-DuckDB end-to-end test "drives **commit_menu**". Per diw-ac06, commit_menu mirrors commit_slider (crossfilter.rs:821), whose signature takes `cx: &mut App` and repaints gpui entities — not constructible in the default gate ("no gpu-tests feature", per diw-ac08's own verification). The repo's precedent is explicit: slw_ac06 (crossfilter.rs:2957) drives the gpui-free data-half `apply_slider` on a coordinator built with an **empty plots vec** ("An empty `plots` vec means no gpui App is needed") and asserts row counts on the swapped batch. As written, the AC is unimplementable verbatim, and this is precisely the AC where an implementer "adapting" the wording under pressure is most dangerous.
**Evidence:** crossfilter.rs:821-826 (commit_slider signature), crossfilter.rs:2952-3035 (slw_ac06 empty-plots pattern with live DuckDB + row-count assertions), diw-ac08 verification ("both in the default gate").
**Recommendation:** Reword diw-ac08 to drive `apply_menu` (the data half, per the diw-ac06 split) on a coordinator constructed with an empty plots vec and a real Session, asserting `marks[i].batch` row counts before/after — the slw_ac06 shape — with the negative control (out-of-range binding index → `None`, batch unchanged) at the same surface. Keep Hugh's eyeball as the canvas half (already the exit condition).

### [LOW] L1 — diw-ac06 names ctor callsites that do not exist

**Category:** assumption
**Pass:** 1
**Description:** diw-ac06 says `CrossfilterCoordinator::new` "gains the parallel param (callsites main.rs:1643, **dump/embed paths**, tests)". The only callsites in the tree are main.rs:1643 (the authoring window) and two `#[cfg(test)]` sites (main.rs:2322, main.rs:3195). The dump path never builds a coordinator — crossfilter.rs's own ctor doc says "the dump/watcher paths never build one".
**Evidence:** `grep -rn "CrossfilterCoordinator::new" crates` → 3 non-definition hits; main.rs:1410-1420 (dump arm returns before workspace construction).
**Recommendation:** Correct to "callsites main.rs:1643 + the two test sites (main.rs:2322, :3195)". Cosmetic, but this spec's stated evaluation principle is "claims about what changes name their sites".

### [LOW] L2 — Radio height formula's "chrome padding" is unpinned, making the ac05 test circular

**Category:** test-gap
**Pass:** 1
**Description:** diw-ac05 sizes a literal-N radio at "height 22·N + chrome padding (Meridian row ladder)" without naming the padding constant. The verification ("radio-literal height formula") can only pin whatever the implementation chose — the test cannot fail.
**Evidence:** diw-ac05 description vs verification.
**Recommendation:** Either name the constant (e.g. "22·N + 10, matching …") or state explicitly that the padding is implementer-chosen at build time and the test pins the chosen value against a named constant shared with the render twin (the SLIDER_* sync convention diw-ac11 already invokes).

### [LOW] L3 — Default-reconciliation coverage is derived-only; literal-list and type-equality cases unpinned

**Category:** test-gap
**Pass:** 2
**Description:** decisions_locked #6 prepends a param default "absent from the resolved options" — generic over literal and derived lists — but diw-ac04's verification pins only the derived case ("param default not in column"). Unpinned: a literal `options:` list omitting the param default (prepended or not?), and the equality semantics of "absent" across SpecValue types (param default `Integer(2)` vs a DuckDB-derived `Float(2.0)` or `String("2")` — a strict-variant comparison would spuriously prepend).
**Evidence:** diw-ac04 verification list; decisions_locked #6; spec_value_to_sql_literal's type-per-variant behaviour (emit.rs:195-215) makes the variant identity load-bearing downstream.
**Recommendation:** Add a literal-list prepend test to diw-ac04's verification, and one sentence pinning the comparison rule (e.g. SpecValue PartialEq, with numeric cross-variant equality explicitly out — the derived path should surface column values in their native variant so a same-typed default compares equal).

### [LOW] L4 — Artifact-hygiene notes: card anchor slip; tabletop routing superseded (deliberately)

**Category:** assumption
**Pass:** 1
**Description:** (a) Card 0024's scope comment cites "propagate_param brightfield-ui/src/lib.rs:685" — the function is in brightfield-**engine**/src/lib.rs:685; the spec cites it correctly. (b) The tabletop's Adjacent-code section routes options resolution "via Session::execute_raw_sql lib.rs:914", which is `#[cfg(test)]`-gated and unusable from brightfield-app; the spec correctly supersedes this with the new public `distinct_values` (decisions_locked #4 documents the correction and keeps the cfg-gate). No spec change needed — recorded so the divergence from the tabletop reads as deliberate, not drift.
**Evidence:** card 0024 line 9; tabletop "Adjacent code"; engine lib.rs:913-914.
**Recommendation:** Fix the card's crate name when it is next touched; no action on the tabletop.

---

## Contradiction/overreach sweep summary (tasks 2–5)

- **Spec vs tabletop:** faithful throughout — values, halt conditions (z-order pivot, >100ms options-cost), escalation triggers (`style:` collision, PNG churn, coordinator surgery), and kill conditions all carried into ACs/risks/open_questions. The one routing divergence (execute_raw_sql → distinct_values) is a justified correction (L4). The tabletop's coordinator-surgery escalation is properly discharged by the spec's honest-reuse-boundary framing and the header-comment correction in diw-ac06.
- **Scope vs card:** no overreach found. All six card scenarios map to ACs (1→ac08, 2→ac03/ac04, 3→ac02/ac07, 4→ac01/ac02 *modulo H1*, 5→ac15, 6→ac10). out_of_scope matches the card's deferrals; the "label:" deferral honestly matches the code (render_slider draws no label).
- **Verification classifications:** honest. The two `stand-in` declarations (ac07, ac11) are exactly the pixels-on-screen cases the window-verification convention covers, each with the acceptance rationale inline. All `capability` claims are backed by live-DuckDB or unit surfaces that exist — except ac08's naming issue (M2) and ac04's path-coverage gap (H2).
- **Silent-no-op class:** the commit-path defence (ac08 negative control + any_menu in the live-surface gate, ac06) is well-designed and matches how the card-0023 bug was actually fixed. The uncovered instance of the class is the reload surface (H2).

## Honest Assessment

This is one of the strongest-anchored specs I have cold-reviewed: of ~35 file:line anchors, all but two are line-exact against HEAD, the reuse boundary is honestly drawn (the falsified slider.rs header is named and scheduled for correction), and the commit-path silent-no-op defence is the correct shape with a real negative control. The two HIGH findings are both resolvable with spec-text amendments, not redesign: H1 is a three-way internal contradiction that would pin the card's flagship checkbox scenario dead if the "empty-options None" test lands as written, and H2 is the recurring silently-stale-canvas class surfacing on hot reload, where the spec claims coverage ("diw-ac04 covers both paths") that its verification cannot deliver and the substrate (scene-swap-only watcher, widget-less ChromeSnapshot) actively contradicts the out_of_scope staleness story. Fix H1's checkbox arm, pin a reload posture (ChromeSnapshot widget slice → restart-to-apply is the cheap, precedented one), rescope the two gates/tests (M1, M2), and this spec is ready to implement.

**Verdict:** REQUEST_CHANGES
