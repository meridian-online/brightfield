# Design Interview — Card 0010: Interactive Feedback and Focus

**Card:** `orbit/cards/0010-interactive-feedback-and-focus.yaml`
**Date:** 2026-04-22
**Mode:** Rally design (agent self-answers from card + codebase evidence; author approves at consolidated gate)

---

## Q1: How does the renderer find the nearest data point on hover?

**Decision:** Brute-force scan over the post-aggregation `RecordBatch` at hover time, with a clean seam for a spatial index later.

**Rationale:** The database-first design principle (all heavy computation in DuckDB) guarantees that rendered row counts stay modest -- typically hundreds to low thousands after SQL aggregation. A linear scan over that many rows at 60 Hz is well under 1 ms. A spatial index (k-d tree or sorted list) would add build cost on every re-render that exceeds per-frame savings at n < 10 K. A GPU pick buffer would require a second render pass and custom shaders outside Vello's pipeline -- far more complexity than the problem warrants.

The implementation is a pure function in `brightfield-render`:

```rust
pub enum NearestMode { X, Y, XY }

pub struct NearestHit {
    pub row: usize,
    pub point: kurbo::Point,       // pixel position of the hit mark
    pub distance: f64,             // pixel distance from cursor
}

pub fn find_nearest(
    cursor: kurbo::Point,
    batch: &RecordBatch,
    channel_map: &ChannelMap,
    scales: &ScaleSet,
    mode: NearestMode,
) -> Option<NearestHit>
```

This function lives in `brightfield-render` (not `brightfield-ui`) because it needs `ScaleSet`, `ChannelMap`, and `RecordBatch` -- all render-crate types. The `NearestMode` variants map directly to the `Nearest`, `NearestX`, `NearestY` interactor kinds already registered in `crates/brightfield-spec/src/vocab.rs:200-202`. If profiling later reveals hover jank on large unaggregated datasets, upgrade to a sorted-list index behind the same `find_nearest` API.

**Files affected:**
- `crates/brightfield-render/src/nearest.rs` (new module) -- `NearestMode`, `NearestHit`, `find_nearest()`
- `crates/brightfield-render/src/lib.rs` -- add `pub mod nearest;` and re-exports
- `crates/brightfield-ui/src/interaction.rs` -- call `find_nearest()` from hover handler, replacing the raw `point` with a resolved `NearestHit`

---

## Q2: How is the tooltip rendered -- Vello scene overlay or GPUI element?

**Decision:** Hybrid approach. The nearest-point highlight circle stays in the Vello scene; the text tooltip is a GPUI element positioned at the highlight's screen coordinates.

**Rationale:** The highlight circle is simple geometry tightly coupled to chart coordinates -- it already exists in `interaction.rs:96-98` as a `kurbo::Circle` fill into the Vello `Scene`. Moving it to GPUI would add complexity for no benefit.

The text tooltip is a different story. Tooltip content (field names, formatted values, multi-line layout) requires text shaping, font fallback, padding, and column alignment. Vello's text API is lower-level -- rendering glyphs manually would reimplement what GPUI already does natively. GPUI handles production-quality text rendering (it powers Zed's entire UI), and Zed's own tooltips use GPUI elements. Using GPUI for the text tooltip follows the "use the framework's primitives" principle.

The coordinate bridge is straightforward: `ChartElement` (`crates/brightfield-ui/src/chart_element.rs:16-25`) knows its bounds in GPUI's layout tree. The highlight point in chart-local Vello coordinates translates to GPUI screen coordinates by adding the element's origin offset.

**Files affected:**
- `crates/brightfield-ui/src/tooltip.rs` (new module) -- GPUI tooltip element, positioned from chart coordinates
- `crates/brightfield-ui/src/chart_element.rs` -- extend to manage tooltip child element, coordinate bridge from Vello chart-local to GPUI screen coords
- `crates/brightfield-ui/src/interaction.rs` -- `InteractionState::Hovering` gains an optional `NearestHit` field; `render_overlay()` draws the highlight circle at the resolved hit point

**Key types:**
- `TooltipContent { fields: Vec<(String, String)> }` -- field name/value pairs extracted from the `RecordBatch` row identified by `NearestHit`
- `TooltipElement` -- GPUI element rendering `TooltipContent` as a styled card

---

## Q3: How does highlight/dim work for selection focus?

**Decision:** Per-row opacity in `MarkRenderer`. When a highlight selection is active, matching rows render at full alpha; non-matching rows render at a dimmed alpha (e.g. 0.15).

**Rationale:** Per-row opacity is the correct semantic match for Mosaic's `highlight` interactor. The current `MarkRenderer::render()` implementations (`crates/brightfield-render/src/mark.rs`) already iterate per-row and resolve a `Color` via `resolve_colour()` (line 100-118), which returns a `peniko::Color` with an alpha channel. Multiplying that alpha by a dim factor is one line of code per row.

A two-pass approach (render all dimmed, then re-render selected at full opacity) doubles draw calls and produces incorrect compositing where marks overlap. A post-processing mask can't express "dim everything except these specific marks" for point-based highlights.

The `MarkRenderer::render()` signature gains an `Option<&HighlightState>` parameter:

```rust
pub struct HighlightState {
    pub predicate: Box<dyn Fn(usize) -> bool>,
    pub dimmed_alpha: f64,  // e.g. 0.15
}
```

Each renderer checks: if `HighlightState` is present and `predicate(row_index)` is false, multiply the resolved colour's alpha by `dimmed_alpha`. The `Highlight` interactor kind in `vocab.rs:206` flips from `Unimplemented` to `Implemented`.

**Files affected:**
- `crates/brightfield-render/src/mark.rs` -- `MarkRenderer::render()` gains `Option<&HighlightState>` parameter; `resolve_colour()` or call sites apply dim factor; `HighlightState` struct defined here
- `crates/brightfield-render/src/scene.rs` -- `build_chart_scene()` passes `HighlightState` through to `renderer.render()`; `ChartData` struct gains optional `highlight: Option<HighlightState>`
- `crates/brightfield-spec/src/vocab.rs:206` -- `Highlight` status changes from `Unimplemented` to `Implemented`
- `crates/brightfield-render/src/channel.rs` -- note: the `Channel` enum does not yet have an `Opacity` variant; per-row highlight opacity should compose multiplicatively with a future `Opacity` channel, not replace it

---

## Q4: What animation system drives smooth mark transitions?

**Decision:** Mark-level interpolation in the render pipeline, using GPUI's easing functions for the curve shape.

**Rationale:** GPUI's `with_animation()` operates on element-level properties (transform, opacity, layout values), not on the internal positions of Vello primitives within a scene. Animating the entire `ChartElement` as a unit produces fade-in or slide-in effects for the whole chart -- not the per-mark positional continuity the card requires ("marks transition smoothly").

A scene-level crossfade (dissolve between old and new scenes) doesn't convey which marks changed -- a bar growing looks the same as a bar disappearing through a dissolve.

Mark-level interpolation produces the correct visual: marks glide from old to new positions, bars grow/shrink, lines morph. The implementation:

1. On data change, snapshot current per-mark pixel positions as `prev_positions: Vec<(f64, f64)>`
2. Compute new positions from the new `RecordBatch` + `ScaleSet`
3. Start a transition with duration (e.g. 300 ms) and a GPUI easing function (e.g. `ease_in_out`)
4. Each frame: compute `t = easing(elapsed / duration)`, interpolate each mark as `lerp(prev, next, t)`, build the scene with interpolated positions, call `ChartElement::set_scene()`
5. When `t >= 1.0`, settle to final state

The `MarkRenderer` trait gains an optional `render_interpolated()` method with a default impl falling back to `render()`:

```rust
fn render_interpolated(
    &self,
    scene: &mut Scene,
    batch: &RecordBatch,
    channel_map: &ChannelMap,
    scales: &ScaleSet,
    prev_positions: &[(f64, f64)],
    t: f64,
    highlight: Option<&HighlightState>,
) {
    // Default: ignore interpolation, render final state
    self.render(scene, batch, channel_map, scales);
}
```

**Files affected:**
- `crates/brightfield-render/src/mark.rs` -- `MarkRenderer` trait gains `render_interpolated()` with default impl; `DotRenderer`, `BarRenderer`, `LineRenderer` override with per-mark lerp logic
- `crates/brightfield-render/src/transition.rs` (new module) -- `Transition` struct (prev positions, start time, duration, easing fn), `TransitionState` enum (`Idle | Running { ... } | Complete`)
- `crates/brightfield-render/src/scene.rs` -- `build_chart_scene()` accepts optional `Transition` to route through `render_interpolated()`
- `crates/brightfield-ui/src/chart_element.rs` -- `ChartElement` gains `transition: Option<Transition>` field; frame loop drives interpolation via `request_animation_frame()` or equivalent GPUI mechanism

---

## Q5: What animates and what renders immediately?

**Decision:** Data-driven mark changes and highlight/dim opacity transitions animate. Brush overlays, hover highlights, and tooltip appearance are immediate.

**Rationale:** The rule is grounded in input latency: if the change is driven by direct user input (cursor position, brush drag), rendering must be immediate -- any animation delay between cursor movement and visual feedback degrades the "fluid interaction" promise. The current `render_overlay()` in `interaction.rs:80-101` is designed for zero-latency immediate rendering and should stay that way.

Data-driven transitions (new `RecordBatch` from a filter update or re-query) are the highest-value animation -- they show the viewer what changed. These use D4's mark-level interpolation with ~300 ms duration.

Highlight/dim opacity transitions are a lightweight addition. When selection state changes, the alpha interpolation is a single `lerp()` per mark per frame, piggybacking on D3's per-row opacity. A short duration (100-150 ms) keeps it responsive while communicating the state change more clearly than a snap.

The classification:

```
| Change type                          | Behaviour       | Duration  |
|--------------------------------------|-----------------|-----------|
| RecordBatch update (data change)     | Animate (D4)    | ~300 ms   |
| Highlight/dim (selection change)     | Animate (fade)  | 100-150ms |
| Brush overlay (drag in progress)     | Immediate       | 0         |
| Hover highlight circle               | Immediate       | 0         |
| Tooltip appearance/disappearance     | Immediate       | 0         |
```

**Files affected:**
- `crates/brightfield-render/src/transition.rs` -- `TransitionKind` enum (`Data | Highlight`) with per-kind default durations
- `crates/brightfield-ui/src/interaction.rs` -- no animation added; `render_overlay()` stays immediate
- `crates/brightfield-ui/src/chart_element.rs` -- transition dispatch: data changes start a `Data` transition; highlight state changes start a `Highlight` transition; overlay state changes call `set_scene()` immediately

---

## Summary of key files and symbols

### New modules

- **`crates/brightfield-render/src/nearest.rs`** -- nearest-point resolution
  - `NearestMode` enum (`X | Y | XY`)
  - `NearestHit` struct (row index, pixel point, distance)
  - `find_nearest()` -- brute-force scan over RecordBatch rows

- **`crates/brightfield-render/src/transition.rs`** -- animation state
  - `Transition` struct (prev positions, start time, duration, easing)
  - `TransitionState` enum (`Idle | Running | Complete`)
  - `TransitionKind` enum (`Data | Highlight`) with default durations

- **`crates/brightfield-ui/src/tooltip.rs`** -- GPUI tooltip element
  - `TooltipContent` struct (field name/value pairs)
  - `TooltipElement` -- GPUI element rendering tooltip card

### Modified modules

- **`crates/brightfield-render/src/mark.rs`**
  - `HighlightState` struct (predicate, dimmed_alpha)
  - `MarkRenderer::render()` -- gains `Option<&HighlightState>` parameter
  - `MarkRenderer::render_interpolated()` -- new method with default impl
  - `DotRenderer`, `BarRenderer`, `LineRenderer` -- per-row opacity + interpolation overrides

- **`crates/brightfield-render/src/scene.rs`**
  - `ChartData` gains `highlight: Option<HighlightState>` field
  - `build_chart_scene()` passes highlight and transition state to renderers

- **`crates/brightfield-ui/src/interaction.rs`**
  - `InteractionState::Hovering` gains optional `NearestHit`
  - Hover handler calls `find_nearest()` to resolve cursor to nearest data row

- **`crates/brightfield-ui/src/chart_element.rs`**
  - `ChartElement` gains `transition: Option<Transition>` field
  - Coordinate bridge for tooltip positioning (chart-local to GPUI screen)
  - Frame-driven transition loop via GPUI animation primitives

- **`crates/brightfield-spec/src/vocab.rs`**
  - `Nearest`, `NearestX`, `NearestY`, `Highlight` flip from `Unimplemented` to `Implemented`

### Integration points

- **`brightfield-render` <-> `brightfield-ui`:** `find_nearest()` is called from the UI hover handler with render-crate types (`ScaleSet`, `ChannelMap`, `RecordBatch`). The `NearestHit` result flows back to the UI for tooltip content extraction and highlight circle positioning.
- **`brightfield-render` <-> `brightfield-spec`:** `NearestMode` maps to `InteractorKind::Nearest/NearestX/NearestY` from `vocab.rs`. The spec parser determines which mode a plot uses; the render crate executes it.
- **`brightfield-ui` <-> GPUI:** Tooltip uses GPUI's native text rendering and layout. Transition frame loop uses GPUI's `request_animation_frame()` or equivalent. Easing functions imported from GPUI.
- **Card 0007 (interactive navigation):** Pan/zoom triggers re-query, delivering a new `RecordBatch` that enters D4's mark transition pipeline. Both cards share `InteractionState` and should coordinate on extending it.
