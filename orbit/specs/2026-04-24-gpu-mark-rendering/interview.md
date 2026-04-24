# Design Interview Record — Card 0013 v2: GPU-Accelerated Mark Rendering

Card: `orbit/cards/0013-gpu-accelerated-mark-rendering.yaml`
Rally: "first end-to-end render"
Date: 2026-04-24

## Goal

ChartElement implements the GPUI Element trait, rendering a Vello scene as a
GPU texture in a live window. Mouse events flow into InteractionState and the
render loop repaints on data or layout changes at 60+ FPS.

## Prior Art

v1 shipped (all 11 ACs complete):
- Vello as 2D backend; all chart content in a single vello::Scene
- CPU readback for texture handoff (documented, not implemented)
- Two-crate split: brightfield-render (headless) + brightfield-ui (GPUI shell)
- Fixed margin model (Observable Plot defaults: 20/20/30/40)
- InteractionState enum (Idle, Brushing, Hovering) with render_overlay()
- NavigationState (pan/zoom with debounce settle) — from card 0007
- Transition struct (mark-level lerp with linear easing) — from card 0010
- HighlightState, find_nearest, TooltipContent — from card 0010

## Decisions

### Q1: How should ChartElement implement the GPUI Element trait?

**Decision: Wrapper pattern — ChartView (Render + Model) → ChartElement (IntoElement).**

ChartView is a new struct implementing gpui::Render. It owns a Model<ChartState>
where ChartState holds the vello::Scene, InteractionState, NavigationState,
Transition state, and layout dimensions. ChartView::render() returns a
ChartElement that implements IntoElement and handles layout sizing + paint.

The existing ChartElement struct evolves into the inner element. ChartView is
the new outer component that owns reactive state via GPUI's Model system.

Rationale: This is idiomatic GPUI. Model<T> notifications drive repaint
automatically. Mouse event handlers live on ChartView and mutate state via
model.update(). Matches Zed's editor architecture.

### Q2: How does the Vello scene become pixels in the GPUI window?

**Decision: Synchronous render in paint().**

In ChartElement's paint method, call vello::Renderer::render_to_texture()
with the dedicated wgpu device, read back pixels, and submit as a GPUI image.
On Apple Silicon unified memory, this completes in <5ms for typical charts,
well within the 16ms frame budget.

Rationale: Simplest path. No threading, no buffer management, deterministic
frame content. If profiling shows budget pressure for complex dashboards,
extract to a background task and cache the result — the ChartState interface
doesn't change.

### Q3: What triggers repaint when data or interaction changes?

**Decision: Model notifications + explicit resize callback + cx.on_next_frame() for transitions.**

All repaint triggers flow through GPUI's Model notification system:
- Data changes: new RecordBatch → new Scene → model.update() → cx.notify()
- Mouse events: GPUI delivers MouseDown/Move/Up → handler updates
  InteractionState via model → notify
- Resize: observe_window_bounds() callback → update layout dimensions → notify
- Transitions: cx.on_next_frame() schedules ticks until Transition::Complete

Rationale: Event-driven, zero CPU when idle, battery-friendly. GPUI's
intended pattern.

### Q4: How do mouse events reach InteractionState and NavigationState?

**Decision: Single hitbox, plot-area bounds check, inverse scale transform.**

Register one hitbox covering the full chart element during paint(). Event
handlers on ChartView receive mouse events in window coordinates. Transform:
1. window_pos - element_origin → local_px
2. Check if local_px falls within plot area bounds (ChartLayout)
3. scale.invert(local_px) → data_value

Clicks outside the plot area (axes, legend) are ignored for v2.

Rationale: Matches the single-element architecture. Legend and axis
interactivity are future features. The inverse scale transform is already
implemented (Scale::inverse_f64 from card 0007).

### Q5: How is the wgpu device for Vello managed?

**Decision: Dedicated wgpu device created once at app startup, shared via Arc.**

Create a standalone wgpu Instance/Adapter/Device/Queue at application init.
Wrap in Arc<VelloRenderer> (holding device, queue, and vello::Renderer).
Inject into every ChartState. Isolated from GPUI's own rendering context.

Rationale: Safe, no coupling to GPUI internals. Device creation is ~10ms,
done once. Memory overhead is negligible. Matches Vello's own examples.

## Constraints

- brightfield-render must NOT gain a gpui dependency (v1 constraint, still holds)
- ChartView and its GPUI wiring live in brightfield-ui
- All existing brightfield-render and brightfield-ui tests must continue to pass
- The render pipeline must work on Apple Silicon (macOS Metal)
