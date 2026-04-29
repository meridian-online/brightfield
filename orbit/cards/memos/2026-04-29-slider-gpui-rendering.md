# Slider GPUI rendering — follow-up to rpw3

**Date:** 2026-04-29
**Source:** rpw3 PR review LOW-2 finding
**Spec:** `orbit/specs/2026-04-29-reactive-parameters-runtime/spec.yaml`
**Card:** `orbit/cards/0005-reactive-parameters-with-input-widgets.yaml`

## What rpw3 shipped

`brightfield-ui::slider` now exposes:

- `ParamDispatcher` trait + `impl ParamDispatcher for Session` (forwarding to `propagate_param`).
- `SliderBinding` constructed from an `Input` AST node's `as_param` + `min`/`max`/`step` options.
- `SliderState` enum (`Idle | Dragging { value } | Released { value }`) with `start_drag` / `update_drag` / `release` transitions.
- `commit_slider_release` pure helper that dispatches a `SpecValue::Float(value)` and returns the result vec.

Coverage: rpw3_ac09..ac-13 via the cfs2_ac11 lifted-helper boundary.

## What is NOT yet rendered

- No `gpui::Element` / `Render` impl for a Slider widget.
- No Track + thumb GPUI primitives.
- No mouse_down / mouse_up GPUI event handlers wired into a real window.
- No `ChartView::propagate_param_and_redraw` helper that observes the dispatch result vec and triggers re-render.

This means: an analyst who opens the brightfield app and loads a spec with `input: slider` cannot drag the slider yet. The state machine + dispatch surface are wired and tested, but the visible+draggable widget is not.

## Why this passes rpw3 strictly

The rpw3 spec's ac-12 verification text explicitly says: *"End-to-end GPUI event simulation is out of scope; the lifted-helper coverage matches the cfs2_ac11 precedent."* The AC scope was authored to verify the binding + state-machine + dispatch surfaces, not the rendered widget. The reviewer flagged it LOW (advisory) for that reason.

## Why it deserves a card-shaped follow-up

The rally goal narrative for "live reactivity" implies a draggable slider that re-renders downstream views. The data-side wiring is now complete (param coordinator, selection passthrough, slider state machine, dispatcher trait, vocab flip). The remaining gap is one layer thick: GPUI primitives + ChartView wiring.

## Suggested next-card scope

A card titled something like *"Slider widget rendering — Track, thumb, mouse handlers"* covering:

1. `gpui::Element` impl for a Slider widget (Track rectangle + Thumb circle, painted via Vello).
2. Mouse handlers: `on_mouse_down → SliderState::start_drag`, `on_mouse_move → update_drag`, `on_mouse_up → release + commit_slider_release`.
3. `ChartView::propagate_param_and_redraw` helper that consumes the result vec, applies updated batches to `ChartState`, and triggers `cx.notify()`.
4. End-to-end smoke: a spec with `input: slider as: $threshold` + a plot subscribing to `$threshold` re-renders when the slider drags.
5. Visual treatment: range labels, thumb hit area, focus ring (deferrable to a polish card).

## Adjacent work that pairs well

- **Menu / Search / Table widgets** (Decision 5 deferred). They reuse `ParamDispatcher` + `commit_<widget>_release` verbatim. Adding them after Slider GPUI primitives lands is mechanical.
- **Slider input fixture in the corpus.** `vendor/mosaic-specs/yaml/` should grow a slider-driven spec to exercise the full chain in `brightfield-app`.

## Status

Captured for the next sprint. Not blocking rpw3 ship.
