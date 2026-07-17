# Tabletop — Discrete input widgets (menu / radio / checkbox)

**Date:** 2026-07-17
**Cards in scope:** 0024 (discrete input widgets); extends 0005 (reactive params, shipped machinery) + 0017 (authoring workspace, hosting)
**Output spec:** orbit/specs/2026-07-17-discrete-input-widgets/spec.yaml

Capability ambition, one sentence: *an analyst declares a menu, radio group,
or checkbox in the spec and clicks it to drive a param live — the slider's
loop, discrete.* One card, one spec — no cluster carving.

Forks were closed with Hugh 2026-07-17 (AskUserQuestion, all four
recommendations accepted): **slider-parity hand-drawn construction** (GPUI
quads + vello resting twin, model/element split); **style attribute on menu**
(`input: menu` + `style: radio|checkbox` in the preserved-verbatim options
bag — NO new InputKind variants, portable by construction); **param-valued
only** in v1 (selection-valued menus defer, pairing with filterBy);
**data-derived options static at load** (SELECT DISTINCT at assembly, capped,
warning beyond the cap).

---

## Values

1. **Reuse over invention** (load-bearing). Every mechanism already exists:
   `ParamDispatcher` + `commit_slider_release` (slider.rs — whose header
   explicitly reserves this reuse: "no coordinator change required"),
   `placed_input_nodes` placement, the vello resting-twin convention
   (render_slider → PNG dumps), the vocab-honesty promotion discipline
   (slw ac-08). The work is a widget, not an architecture.
2. **Portability by construction.** The style-attribute design means every
   spec this card enables parses as plain Mosaic `input: menu`; serialise_spec
   is byte-untouched. Zero new vocabulary.
3. **gpui-free semantics** (egui-exit posture, design ADR 0003). Option
   resolution, one-of-N / toggle state machines, param binding, commit logic —
   all headless-testable; the GPUI element is a thin shim (slider.rs vs
   slider_element.rs split, verbatim precedent).

## Trade-offs

- **Hand-drawn menu popup vs gpui-component richness** — acceptable. v1 caps
  visible options (no scroll); a long list degrades honestly (cap + warning).
  Buys: PNG dumps keep showing widgets, semantics stay gpui-free.
- **Checkbox is a two-option menu** — acceptable semantic constraint. `style:
  checkbox` requires exactly two options (default `[true, false]`); the widget
  toggles between them. Anything else refuses at parse/analysis with a warning
  and falls back to menu presentation.
- **Static options at load** — acceptable staleness. Distinct values resolve
  once at assembly (profile precedent); an external data change shows on
  reload, like profiles. filterBy narrowing deferred.
- **Param-only v1** — no cross-filter menus yet; equality-clause Selection
  menus are a later card. Keeps this card M.

## Halt conditions

- **Popup z-order failure**: if a menu's open option-list cannot paint above
  sibling plots/widgets inside the existing element tree (measured: overlay
  visibly clipped in the demo spec), halt the popup arm and pivot v1 menu
  presentation to inline expansion (radio-style expanded list) — semantics and
  tests unchanged, presentation-only pivot.
- **Assembly-time options query cost**: if resolving distinct options
  measurably delays first render on the example corpus (>100ms added vs
  main, timed), move options resolution onto the existing off-thread
  profiling session pattern rather than blocking assembly.

## Escalation triggers

- **`style:` key collision** — if Mosaic/vgplot is found to assign meaning to
  a `style` key on menu inputs (check during implement), surface to Hugh with
  a rename proposal (`presentation:`) before any code lands on the key.
- **Existing PNG churn** — the design predicts 30/30 existing example PNGs
  byte-identical (widgets are additive; no existing example declares a menu).
  ANY existing-baseline diff halts the PR and surfaces the diff gallery.
- **Coordinator surgery** — the slider.rs header promises "no coordinator
  change required." If menu commit genuinely needs CrossfilterCoordinator
  changes beyond registering bindings (the slider precedent), surface the
  diff and rationale before proceeding (see kill condition).

## Kill conditions

- **"Pure reuse" claim dies** if the param dispatch path cannot carry a
  String-valued param end-to-end (slider only ever dispatched floats; menu
  dispatches strings — SQL substitution must quote/escape correctly). If
  String params need engine/SQL surgery beyond what card 0005 shipped, the
  UI-mostly framing is dead: re-scope with an explicit engine AC and its own
  live-DuckDB tests, and re-review the spec (REQUEST_CHANGES myself rather
  than absorb silently).
- **"Portable by construction" claim dies** if `style:`-carrying specs fail
  to parse in reference Mosaic tooling (spot-check against vendored parse
  corpus conventions). Pivot: move presentation hint to a Brightfield-
  namespaced key documented as DEV entry.

## Verification posture

- Menu/radio/checkbox drive a live re-render: `verifies: capability` —
  headless data-effect test through propagate_param with a real DuckDB
  session, plus a NEGATIVE CONTROL (the card-0021/0022 silent-no-op defence:
  a deliberately unwired double proves the test can fail).
- Distinct-options resolution: `verifies: capability` — live DuckDB, ORDER BY
  pinned, cap behaviour asserted at cap+1.
- Popup/hover/pressed visuals: `verifies: stand-in (real thing is pixels on
  screen), accepted because the repo's convention is state-machine assertions
  headless + Hugh's in-app eyeball for pixels (window-verification memory).`
- Resting twin in PNG dumps: `verifies: capability` — new example baseline
  contains the widget; existing 30 baselines byte-identical.

## Budget

1–2 Claude-execution days: model/state modules + element + resting twin +
assembly wiring + example + tests. The popup is the only genuinely new UI
machinery; everything else is patterned.

## Adjacent code (Q8 routing — details to spec Implementation Notes)

brightfield-ui (new menu.rs + menu_element.rs, chart_view.rs hosting,
crossfilter.rs binding registration), brightfield-app (assembly: options
resolution via Session::execute_raw_sql lib.rs:914, default reconciliation —
the slider-clamp analogue main.rs:758), brightfield-render (resting twins
beside render_slider), brightfield-spec (vocab promotion Menu→Implemented +
tests; NO new variants), examples/ (new menu example + baseline).

## Hot-wash

- **recurred**: every design fork resolved toward an existing precedent —
  the codebase's conventions (model/element split, resting twin, vocab
  honesty, negative controls) now effectively design new features themselves.
- **surprised**: radio/checkbox needed no vocabulary at all — the
  options-bag-preserved-verbatim parse decision (staged typing) from months
  ago is what makes portability-by-construction free today.
- **friction**: the orbit CLI (0.4.38 substrate) doesn't see this repo's
  pre-CLI `orbit/` layout — drive is being run hand-rolled against the
  repo-native conventions instead of `orbit spec promote`.
- **meta-patterns-for-future-tabletops**: the String-param kill condition was
  found by asking "what did the precedent never exercise?" (slider = floats
  only) — worth asking of every "just repeat the pattern" card.
