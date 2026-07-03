# Decision Pack: Slider Widget — Live Reactive Param Input (card 0005 v2)

Window-gated completion of card 0005. The reactive **engine** (`propagate_param`,
card 0005 rpw3) and the **query effect** (card 0014, params drive SQL) are both
shipped and merged. The slider's **pure half** — `SliderBinding` / `SliderState`
/ `commit_slider_release` / `ParamDispatcher` (impl for `Session`) — already
exists in `brightfield-ui/src/slider.rs` and is unit-tested. What is missing is
purely the **live wiring**: extract the input's layout rect, build its binding,
draw a GPUI widget, capture its drag, and re-render subscriber plots on release.

A recon of four subsystems (slider-commit chain, layout, GPUI render, live
cross-filter) established that the **live cross-filter brush path is a near-exact
template**: `CrossfilterCoordinator::{absorb, build_plot_scene}` are
param-agnostic and consume the *identical* `Vec<(mark_index, Result<Vec<RecordBatch>>)>`
shape `propagate_param` returns. Five decisions follow.

## Decision 1: Coordinator — extend `CrossfilterCoordinator`, don't add a parallel one.

The coordinator already owns the live `Session`, the `Vec<MarkInput>`, the
`Vec<LivePlot>`, and `mark_to_plot`. A slider commit needs exactly those, plus
`SliderBinding`s. A separate `ParamCoordinator` would either duplicate that state
or fight the `Session` single-ownership.

**Chosen:** add `slider_bindings: Vec<SliderBinding>` + a `commit_slider(&mut
self, slider_index, state, cx) -> bool` method that calls `commit_slider_release`
(→ `propagate_param`), then reuses `absorb` + `build_plot_scene` + `set_scene` /
`notify` verbatim — the same body as `commit_brush` minus the pixel→data
inversion and predicate build. `absorb`/`build_plot_scene` are unchanged.

## Decision 2: Commit-on-release, not live-drag.

`commit_slider_release` dispatches only on `SliderState::Released` (rpw3_ac13
proves mid-drag never dispatches). Mirrors the brush (re-query on release, not
per-move) and avoids re-executing the query on every mouse-move.

**Chosen:** mid-drag mutates `SliderState` + `window.refresh()` (overlay-only,
the thumb moves over the cached raster — no `set_scene`, no `propagate_param`);
release commits once. Continuous live-drag re-execution is a future enhancement.

## Decision 3: Fixed 200×32 footprint at the declared layout slot.

The layout box model already reserves `DEFAULT_INPUT_WIDTH`×`DEFAULT_INPUT_HEIGHT`
(200×32) for an `input:` node wherever it sits in an hconcat/vconcat, but it does
not stretch (no flex). A full-width slider band matching a sibling plot would need
a stretch/align policy the box model lacks.

**Chosen:** v1 hosts the slider at its declared 200×32 rect. Stretch/align is a
layout enhancement (deferred, recorded). The window bounding-box fold must be
widened to include input rects or the slider is clipped.

## Decision 4: Surface placed inputs via the same two-walker path-join plots use.

Plots join a layout rect (`placed_plots`, spec) to their data (`collect_plot_groups`,
sql) by component path (`root/hconcat[i]`). `compute_layout` maps Component →
LayoutNode 1:1, so the same join works for inputs.

**Chosen:** add `placed_inputs` (LayoutNode walker → `PlacedInput{path, rect}`)
and `collect_input_nodes` (Component walker → `(path, &Input)`) in
`brightfield-spec/src/layout.rs` (both path-schemes in one file so they can't
drift), plus a `placed_input_nodes(spec, viewport) -> Vec<(Rect, &Input)>`
convenience that joins them. Both walkers recurse HConcat/VConcat and stop at
Plot (a slider is a composition-level sibling, matching `collect_placed_plots`).

## Decision 5: New GPUI `SliderElement` by copying `ChartElement`.

`slider.rs` is model-only; there is no `gpui::Element`. `ChartElement` is the one
custom Element and the exact pattern: `request_layout` (fixed size) → `prepaint`
(`insert_hitbox`) → `paint` (re-register `on_mouse_event` down/move/up each frame,
map pixel-x → value, draw track+thumb via `window.paint_quad`, `window.refresh()`).

**Chosen:** build `SliderElement` in `brightfield-ui/src/slider.rs` mirroring
`ChartElement`: track = thin full-width quad, thumb = a rounded quad at
`min + frac*(max-min)`; horizontal-only pixel→value map with step snapping;
overlay-only repaint on drag, `coordinator.commit_slider` on release. Host it in
`ChartView` alongside `PlacedChart` (a new `PlacedSlider`), holding the shared
`Rc<RefCell<CrossfilterCoordinator>>`. Re-promote `InputKind::Slider` to
`Implemented` (reversing PR #22 for slider only) once wired.

### Verification (window-gated)

Headless-provable: rect extraction + join, binding construction, and the full
`SliderState::Released → commit_slider → propagate_param → absorb → new scene`
path (unit/integration tests). A **resting-widget PNG** (render the slider into
the headless `compose_dashboard` path) eyeballs layout/geometry. The **drag
interaction** itself needs the macOS app + human eyeball — build, PNG-verify at
rest, then hold the PR.

### Deferred (recorded)

Live-drag continuous re-exec; slider stretch/align (full-width band); computed-param
sliders (`min`/`max`/`step` as `$param` refs — `from_input` already returns None);
non-slider inputs (menu/search/table); structural hot-reload of a new slider region.
