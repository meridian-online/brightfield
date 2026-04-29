# Escape-key clearing — follow-up to cfs3

**Date:** 2026-04-29
**Source:** cfs3 spec impl-notes paragraph 8
**Spec:** `orbit/specs/2026-04-29-cross-filtered-selections-interactors/spec.yaml`
**Card:** `orbit/cards/0006-cross-filtered-selections-across-linked-views.yaml`

## What cfs3 shipped

`brightfield-engine::Session` now exposes `clear_selection(name, contributor)` —
the runtime surface for retracting a contributor's predicate from a named
selection. `brightfield-ui::brush::SelectionDispatcher` exposes `clear(...)`
forwarding to it, and `chart_view::commit_brush_clear` dispatches a clear
when the user releases the mouse on an Idle interaction or with a
zero-area Brushing rect (within `ZERO_AREA_EPSILON = 0.5`).

Coverage: cfs3_ac01..cfs3_ac03 — clear_selection unit (engine), unsubscribed
silent no-op, click-outside-active-brush dispatch (3 sub-cases: Idle,
zero-area Brushing, non-zero Brushing).

## What is NOT yet wired

- No `KeyDownEvent` / `KeyUpEvent` handler on `ChartView`.
- No mapping from the GPUI `Escape` key code to `commit_brush_clear`.
- No focus-aware routing (which view "owns" the escape press when multiple
  views are mounted).
- No global key handler installed on the window root.

This means: an analyst whose plot is brushed cannot press Escape to clear.
They must click outside the brush region (the canonical clearing path
shipped in cfs3) or programmatically dispatch a clear via the engine.

## Why this passes cfs3 strictly

The cfs3 spec's impl-notes paragraph 8 explicitly says:

> *"Escape-key clearing is OUT OF SCOPE for this slice. Click-outside-active-brush
> is the canonical clearing path; escape-key routing requires GPUI keyboard
> event wiring not in this card."*

The AC scope was authored to verify the runtime + dispatch surfaces, not the
keyboard input layer. Decision 1 of the design pack chose
click-outside-active-brush as the canonical UX path with escape-key as a
nice-to-have that depends on focus management.

## Why it deserves a card-shaped follow-up

Escape-key clearing is a strong UX convention — analysts hitting Escape
expect *something* to be dismissed. Without it, the canonical clearing
path is discoverable only by trial (clicking outside the brush). The
runtime side is already complete (`Session::clear_selection`,
`SelectionDispatcher::clear`, `commit_brush_clear`); the remaining gap
is one layer thick: GPUI keyboard event wiring + focus routing.

## Suggested next-card scope

A card titled something like *"Escape-key clears active selection"* covering:

1. `ChartView` registers a `KeyDownEvent` listener that filters on
   `Keystroke::escape`.
2. On Escape, dispatch `commit_brush_clear` against every active brush
   binding the chart owns (multi-binding case from cfs3_ac04).
3. Focus management: which view owns the press when multiple views are
   mounted. Likely the most-recently-interacted view, scoped via GPUI
   `FocusHandle`.
4. Visual feedback: brush rectangle disappears, downstream views
   re-resolve.
5. End-to-end smoke: a spec with two linked plots, brush plot A,
   press Escape, plot B reverts to unfiltered data.

## Adjacent work that pairs well

- **Selection toolbar widget** — a small "Clear all selections" affordance
  for users who don't reach for Escape. Reuses `Session::clear_selection`
  in a loop.
- **Escape-stack semantics** — if Brightfield grows modals or popovers,
  Escape should pop the topmost overlay first and only then clear
  selections. Out of scope until a second consumer of Escape exists.

## Status

Captured for the next sprint. Not blocking cfs3 ship.
