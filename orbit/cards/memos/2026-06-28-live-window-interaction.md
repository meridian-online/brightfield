# Live-window interaction unlock — wiring mouse events + overlay compositing

The high-value UX review found that the live macOS window was a **frozen screenshot**:
every interaction handler (mouse down/move/up, hover, brush) was fully written and
unit-tested but never wired into `ChartView::render`, and `InteractionState::render_overlay`
(which paints the brush rect / hover marker) was never composited into the displayed
scene. So the cross-filter sprint's payoff was invisible. This memo records the unlock.

## What shipped

`brightfield-ui` — the GPUI shell now routes mouse input and paints the overlay:

- **`ChartElement`** (rewritten) now holds the `Entity<ChartState>`. In `paint()` it:
  1. registers three `window.on_mouse_event` listeners (down / move / up) that map the
     window-space pointer to chart-local coordinates (`element_origin = bounds.origin`)
     and drive the interaction via `ChartState::pointer_*`;
  2. composites `InteractionState::render_overlay` onto a clone of the current scene;
  3. rasterises the composited scene with Vello and paints it.
  A state change calls `window.refresh()`, which (ChartView is the non-cached window
  root) re-runs `render → paint` and re-registers the per-frame listeners.
- **`ChartState::pointer_down / pointer_move / pointer_up`** — the canonical interaction
  transitions (window→local mapping, plot-area containment, brush/hover state). Single
  source of truth shared by the live `ChartElement` path and the legacy `ChartView`
  handler API (which now delegates to them).
- **`ChartView::render`** — now simply `ChartElement::new(self.state.clone())`.

## Verification — and its limit

This was implemented and **fully compiles** (ui + app build warning-free; `cargo test
--workspace` green, incl. new `ChartState` pointer tests under `gpu-tests`). But the dev
environment has **no display and no Metal compiler**, so the live window could not be
opened or driven — runtime behaviour is unverified here.

In place of runtime testing, the GPUI integration was put through an **adversarial review**
(three independent reviewers reading the actual gpui source — event routing, repaint/
reactivity, coordinate/overlay alignment — plus an adjudication pass). Verdict: brushing
and hover **will work and land under the cursor** in the single-chart case — the
paint-time `on_mouse_event` pattern is supported, bubble-phase gating fires each handler
once, the mouse-down hitbox resolves true, `event.position` and `bounds.origin` share the
same window-space frame, and `window.refresh()` genuinely forces the repaint. No blocker,
no infinite repaint loop.

**Confirmed working in a real macOS window by the user on 2026-06-29** — brushing
(blue rect tracking + clamped to the plot) and hover (orange marker following the cursor)
behave as intended. The adversarial review's positive verdict held up at runtime.

## Review fixes applied in this pass

- Brush extension now checks `MouseMoveEvent.pressed_button`; a release that never reaches
  us (focus steal, release outside the window on Linux/Windows) ends the brush instead of
  leaving it rubber-banding to the cursor.
- Brush current point is clamped to the plot area, so dragging into the margins no longer
  overdraws the axes/labels.
- Mouse listeners are registered before the zero-size early-return, so input can't be
  disabled by a transient zero-size frame.
- Dropped the redundant `cx.notify()` inside the live listeners — `window.refresh()` is the
  load-bearing repaint trigger (ChartView does not observe the entity).

## Deferred follow-ups (from the review)

- **Retina crispness** — `render_to_pixels` rasterises at logical resolution; `paint_image`
  upscales it on a 2× display, so the chart is soft. This is **pre-existing** (the window
  path always rasterised at logical size), not introduced here. Fix: read
  `window.scale_factor()`, rasterise at device resolution with an `Affine::scale(sf)` on the
  composited scene. Positions are unaffected. → card 0013 (GPU-accelerated rendering).
- **Per-move re-raster cost** — every hover/drag move re-rasterises the whole scene with a
  synchronous GPU readback on the UI thread. Bounded (no loop), but janky on dense scenes.
  Fix: cache the base raster, composite only the cheap overlay. → card 0013 / 0003 (fluid
  interaction at scale).
- **Brush → selection dispatch** — `pointer_up` only resets to idle; it does not dispatch a
  predicate, and the app shell wires no `SelectionDispatcher`. So a completed brush has no
  data effect yet. This is the cross-filter wiring — → card 0006.
- **Multi-view polish** — gate the hover branch on `hitbox.is_hovered` and clear hover on
  `MouseExitEvent`, so linked multi-view layouts don't cross-talk. → card 0006 / 0009.
