# Decision Pack -- Card 0013 v2: GPU-Accelerated Mark Rendering (Live Window)

Rally: **first end-to-end render**.
Card: `orbit/cards/0013-gpu-accelerated-mark-rendering.yaml`.
Scope: shipping the v2 goal -- ChartElement implements the GPUI Element trait, renders a Vello scene as a GPU texture in a live window, routes mouse events into InteractionState, and repaints on data/layout changes at 60+ FPS.

## What v1 shipped (not re-decided here)

All 11 v1 ACs are complete (progress.md shows full green). The following are settled:

- **Vello as 2D backend** (v1 Decision 1): all chart content renders into a single `vello::Scene`.
- **CPU readback for texture handoff** (v1 Decision 2): Vello renders to wgpu texture, pixels read back to GPUI image element. On Apple Silicon unified memory this is near-free.
- **All-in-Vello scene** (v1 Decision 3): marks, axes, grid, legend are in one Vello scene -- no hybrid GPUI/Vello split.
- **Immediate overlay + deferred re-query** (v1 Decision 4): brush/hover overlays are pure GPU; DuckDB re-queries fire on release.
- **Two-crate split** (v1 Decision 5): `brightfield-render` (headless, no gpui dep) + `brightfield-ui` (GPUI shell).
- **Fixed margin model** (v1 Decision 6): Observable Plot defaults (20/20/30/40).
- **InteractionState enum** (v1 ac-10): Idle, Brushing, Hovering states with `render_overlay()`.
- **NavigationState** (interactive-navigation card): pan/zoom with debounce settle.
- **Transition struct** (interactive-feedback card): mark-level lerp with linear easing.

What v1 deferred: the actual GPUI `Element` trait impl on `ChartElement`, the Vello-to-texture render pipeline, mouse event routing from GPUI into InteractionState, and the reactive repaint loop on data/layout changes.

---

## Decision 1 -- Element trait implementation strategy

### Context

`ChartElement` exists as a plain struct holding a `vello::Scene`, `InteractionState`, width, and height. It has no `impl Element` -- v1 explicitly deferred this ("which requires the gpui runtime"). GPUI's `Element` trait requires `request_layout()` (returns a layout ID and element state) and `paint()` (draws into the window). The v2 goal says ChartElement must implement this trait so it participates in GPUI's layout and paint cycle.

### Options

- **A. Direct `impl Element for ChartElement`.** ChartElement implements `Element` directly. `request_layout()` returns a fixed-size or flex layout node. `paint()` renders the Vello scene to a wgpu texture, reads back pixels, and draws via GPUI's image/surface API. ChartElement owns a wgpu device handle (or shares one via `Arc`).
- **B. Wrapper pattern: `ChartView` component + `ChartElement` inner.** A `ChartView` struct implements `gpui::Render` (the component trait) and owns a `gpui::Model<ChartState>`. `ChartState` holds the `vello::Scene`, `InteractionState`, `NavigationState`, and `Transition`. `ChartView::render()` returns a `ChartElement` that implements `IntoElement`. This separates reactive state management (Model) from layout/paint (Element).
- **C. Use GPUI's `Canvas` element (from PR #42905).** If the canvas PR has landed, use GPUI's native canvas callback to draw Vello content. ChartElement becomes a thin wrapper around a canvas closure.

### Trade-offs

- **A** is the most direct path. Pro: minimal indirection; the struct already exists with the right fields. Con: conflates state ownership with rendering -- `ChartElement` would need to hold `Arc<wgpu::Device>`, `Model<InteractionState>`, and mutable scene state, making it heavy for an Element (which GPUI may recreate each frame in its retained diff cycle).
- **B** follows GPUI's idiomatic pattern: `Render` components own `Model`s that trigger notifications on change; the `render()` method returns a lightweight `Element` that reads from the model. Pro: GPUI's reactivity system handles invalidation -- calling `model.update()` on data change automatically triggers repaint. Mouse event handlers live on the component and call `model.update()`, which is the standard GPUI pattern. Con: two structs instead of one; the boundary between `ChartView` and `ChartElement` must be defined.
- **C** depends on an unmerged upstream PR. The Zed codebase shows `Canvas` in development but not stable. Blocking on this violates "ship the loop before optimising it." If canvas lands later, migrating from B to canvas is a paint-method swap.

### Recommendation

**Option B.** The wrapper pattern is idiomatic GPUI. `ChartView` implements `Render`, owns `Model<ChartState>`, and registers mouse/scroll event handlers that update the model. `ChartElement` (returned from `ChartView::render()`) implements `IntoElement` and handles layout sizing + paint (Vello scene to texture to GPUI surface). The Model's notify-on-update mechanism is what drives the repaint loop (Decision 3). The existing `ChartElement` struct can evolve into the inner element; `ChartView` is the new outer component.

---

## Decision 2 -- Vello scene to GPU texture pipeline

### Context

v1 documented CPU readback as the texture handoff strategy but never implemented it. v2 must actually execute the pipeline: take a `vello::Scene`, render it via Vello's wgpu renderer, and present the result in GPUI's paint cycle. The key question is how to manage the wgpu device and when rendering happens relative to GPUI's frame.

### Options

- **A. Lazy wgpu device, render in `paint()`.** Create (or reuse) a `wgpu::Device` on first paint. In the Element's `paint()` method, call `vello::Renderer::render_to_texture()`, read back pixels synchronously, and submit to GPUI as a `RenderImage`. The device is stored in a shared `Arc` (e.g., on the `ChartState` model or a global resource).
- **B. Pre-render on scene change, cache texture.** When `ChartState.scene` changes (via `Model::update()`), immediately render the Vello scene to a pixel buffer on a background thread. Store the resulting `Vec<u8>` (RGBA pixels). In `paint()`, just submit the cached pixel buffer as a GPUI image -- no wgpu work in the paint path.
- **C. Double-buffered async render.** Maintain two pixel buffers. Scene changes trigger an async render to the back buffer. On completion, swap buffers. `paint()` always reads from the front buffer. Allows rendering to overlap with the next frame.

### Trade-offs

- **A** is simplest. Vello's `render_to_texture()` on Apple Silicon with a small chart (~640x480) completes in <2ms. For a 1920x1080 chart it may take 3-5ms, leaving 11-13ms of the 16ms frame budget for GPUI's own work. Pro: no threading, no buffer management, deterministic frame content. Con: rendering blocks the UI thread during paint; complex scenes or large resolutions could cause frame drops.
- **B** decouples rendering from paint. Pro: paint is fast (just submitting a pre-computed image), so GPUI never blocks on Vello. Con: adds latency -- the rendered image is always one "change" behind. Threading requires `Send` bounds on the Vello renderer and careful synchronisation. The pixel buffer must be shared safely between the render thread and the paint callback.
- **C** is the most performant but the most complex. Pro: zero-latency front buffer, pipelined rendering. Con: significant complexity for v2 -- double-buffer management, synchronisation primitives, potential for visual tearing if not carefully coordinated.

### Recommendation

**Option A for v2.** Render synchronously in `paint()`. The target frame budget is 16ms (60 FPS); Vello's GPU compute renderer on Apple Silicon handles typical chart scenes in well under 5ms. This leaves ample headroom for GPUI's layout and compositor. The synchronous path avoids all threading complexity and guarantees that every painted frame reflects the current scene state -- critical for interaction responsiveness. If profiling shows paint-time rendering exceeds budget for complex dashboards, Option B is a clean upgrade: extract the `render_to_texture()` call to a background task and cache the result. The interface between ChartElement and ChartState does not change.

---

## Decision 3 -- Reactive repaint trigger mechanism

### Context

The v2 goal requires repaint on data change, interaction state change, and window resize. GPUI has a built-in reactivity system: `Model<T>` notifications trigger re-render of any component observing that model. The question is how data/interaction changes flow into the repaint cycle without polling or manual invalidation.

### Options

- **A. Model notifications only.** `ChartState` is wrapped in `Model<ChartState>`. Any mutation (new scene from data change, InteractionState update from mouse event, layout change from resize) goes through `model.update(|state, cx| { ... ; cx.notify() })`. GPUI automatically schedules a repaint for components observing this model.
- **B. Model notifications + explicit `cx.notify()` on window resize.** Same as A, but window resize is handled via GPUI's `WindowContext::on_resize()` callback, which updates the layout dimensions in the model and triggers notify.
- **C. Timer-driven render loop (requestAnimationFrame-style).** Schedule a 16ms timer that checks for dirty state and triggers repaint. Repaint happens every tick regardless of whether state changed.

### Trade-offs

- **A+B** are event-driven: the UI thread is idle when nothing changes. This is GPUI's intended pattern and matches Zed's own architecture (editors repaint only when text/cursor changes). Pro: zero CPU usage when idle; battery-friendly. Con: none for this use case -- all state changes are discrete events (data arrives, mouse moves, window resizes).
- **C** wastes CPU cycles when idle. A render loop running at 60 FPS consumes measurable CPU even when the chart is static. This matters for a desktop application that may be open all day. Pro: simplifies timing for animations (transitions). Con: GPUI already provides animation scheduling via `cx.on_next_frame()` for active transitions.

### Recommendation

**Option B (Model notifications with explicit resize handling).** All repaint triggers flow through GPUI's Model notification system. Data changes: `session.execute_mark()` completes -> `model.update(|s, cx| { s.scene = new_scene; cx.notify() })`. Mouse events: GPUI delivers `MouseDownEvent` / `MouseMoveEvent` / `MouseUpEvent` to the ChartView -> handler updates `InteractionState` via model -> notify. Resize: GPUI's `observe_window_bounds()` or equivalent callback fires -> handler updates `ChartLayout` dimensions in model -> notify. For active transitions (`Transition.state == Running`), use `cx.on_next_frame()` to schedule the next tick until the transition completes -- this avoids a persistent timer.

---

## Decision 4 -- Mouse event routing and coordinate mapping

### Context

GPUI delivers mouse events (down, move, up, scroll) to elements via their hitbox. The chart is a single element with a single hitbox covering its bounds. Mouse events arrive in window-pixel coordinates. InteractionState (brush, hover) and NavigationState (pan, zoom) operate in chart-data coordinates. The mapping requires: window pixels -> chart-element-local pixels -> data coordinates (via inverse scale transform).

### Options

- **A. Hitbox on ChartElement, coordinate transform in event handler.** Register a single hitbox covering the chart element's bounds during `paint()`. GPUI event handlers on ChartView receive `MouseDownEvent { position }` in window coordinates. Subtract the element's origin to get local coordinates. Apply inverse scale transform (`scale.invert(pixel) -> data_value`) to get data coordinates. Update InteractionState/NavigationState accordingly.
- **B. Multiple hitboxes (plot area, axes, legend).** Register separate hitboxes for different chart regions. Plot-area hitbox handles brush/hover/pan; axis hitboxes handle axis-specific interactions; legend hitbox handles legend clicks.
- **C. Raw window-level event handling.** Bypass GPUI's hitbox system; register global mouse handlers on the window and manually hit-test against the chart element's bounds.

### Trade-offs

- **A** is the simplest and matches the single-element architecture (v1 Decision 3: all-in-Vello). One hitbox, one coordinate transform. The inverse scale transform is straightforward: `Scale::invert(pixel) -> f64` for linear/time scales, `BandScale::category_at(pixel) -> &str` for band scales. Pro: clean, minimal code. Con: cannot distinguish clicks on the legend vs. the plot area at the GPUI level -- would need manual region-testing after coordinate transform.
- **B** is more precise. Pro: GPUI handles hit-testing natively; event handlers are scoped to regions. Con: requires multiple elements or sub-hitboxes, which contradicts the single-element design. GPUI's hitbox API may not support sub-element regions without splitting into child elements.
- **C** bypasses GPUI's event system entirely. Anti-pattern; fragile and hard to maintain.

### Recommendation

**Option A.** Single hitbox covering the full chart element. In the event handler, check whether the local-pixel coordinate falls within the plot area (`ChartLayout.plot_x_start..plot_x_end`, `plot_y_start..plot_y_end`) before initiating brush/hover/pan interactions. Clicks outside the plot area (on axes, legend) are ignored for v2; legend interactivity and axis-click-to-zoom are future features. The coordinate transform is a two-step pipeline stored on ChartState: (1) `window_pos - element_origin -> local_px`, (2) `scale.invert(local_px) -> data_value`. The element origin is captured during `paint()` from the element bounds.

---

## Decision 5 -- wgpu device lifecycle and sharing

### Context

Vello's renderer requires a `wgpu::Device` and `wgpu::Queue` to execute GPU compute shaders. GPUI also uses wgpu (or Metal directly) for its own rendering. The question is whether to share GPUI's GPU device or create a separate one, and how to manage its lifecycle.

### Options

- **A. Dedicated wgpu device for Vello, created once on app startup.** Create a standalone `wgpu::Instance` / `Adapter` / `Device` / `Queue` at application init. Store in an `Arc` on a global resource or passed to ChartState. Vello's renderer uses this device exclusively.
- **B. Share GPUI's underlying GPU device.** Extract the `wgpu::Device` (or `MTLDevice` on macOS) from GPUI's rendering context and pass it to Vello's renderer. Both GPUI and Vello submit work to the same device/queue.
- **C. Create a new device per ChartElement.** Each chart gets its own wgpu device.

### Trade-offs

- **A** is safe and isolated. Two GPU devices in the same process is well-supported on modern GPUs (macOS allocates separate command queues). Pro: no coupling to GPUI internals; no risk of resource contention between GPUI's rendering and Vello's compute shaders. Device creation is ~10ms, done once. Con: slightly higher GPU memory footprint (two device contexts). On Apple Silicon, both devices share the same unified memory pool anyway.
- **B** is theoretically optimal (one device, shared memory, coordinated scheduling). Con: GPUI does not expose its underlying wgpu device via public API. Extracting it requires unsafe access to GPUI internals or upstream changes. High coupling risk; any GPUI GPU abstraction change breaks this.
- **C** is wasteful. Multiple devices for multiple charts in a dashboard would exhaust GPU resources. Not viable.

### Recommendation

**Option A.** Create a single dedicated wgpu device at application startup, wrap in `Arc<VelloRenderer>` (holding device, queue, and `vello::Renderer`), and inject into every `ChartState`. This is what Vello's own examples do (`examples/with_winit`). The isolation from GPUI's rendering context eliminates a class of synchronisation bugs. The memory overhead is negligible on modern hardware. If GPUI later exposes device-sharing hooks, migrating to Option B is a renderer-init change, not an architectural one.

---

## Summary table

```
| #  | Decision                               | Recommendation                                                                 |
|----|----------------------------------------|--------------------------------------------------------------------------------|
| 1  | Element trait implementation            | Wrapper pattern: ChartView (Render + Model) -> ChartElement (IntoElement)      |
| 2  | Vello-to-texture pipeline              | Synchronous render in paint(); <5ms on Apple Silicon, within 16ms budget       |
| 3  | Reactive repaint triggers              | Model notifications + resize callback; cx.on_next_frame() for transitions      |
| 4  | Mouse event routing                    | Single hitbox, plot-area bounds check, inverse scale transform for data coords |
| 5  | wgpu device lifecycle                  | Dedicated device created once at startup, shared via Arc across charts         |
```

## Cross-cutting notes

- **GPUI API stability:** GPUI is Zed's internal framework with no stability guarantees. The Element trait, Hitbox API, and Model system are the most stable parts (used throughout Zed's editor). Pin to a specific Zed commit in Cargo.toml (already done: `gpui = { git = "..." }`).
- **Transition animation during repaint:** Decision 3 uses `cx.on_next_frame()` for transition ticks. The existing `Transition` struct (from the interactive-feedback card) provides `tick(now) -> (t, state)`. During a transition, `ChartView` registers a next-frame callback that calls `tick()`, updates the interpolated scene via `render_interpolated()`, and notifies the model. When `state == Complete`, the callback is not re-registered.
- **Data change flow:** The full pipeline for a data update is: DuckDB query completes -> new `RecordBatch` -> `build_chart_scene()` produces new `vello::Scene` -> `model.update(|s, cx| { s.scene = new_scene; cx.notify() })` -> GPUI schedules repaint -> `paint()` renders scene to texture -> GPUI composites.
- **Resize flow:** GPUI window resize -> callback fires -> `model.update(|s, cx| { s.layout = ChartLayout::new(new_w, new_h); cx.notify() })` -> repaint with new dimensions. Scale ranges are recomputed from the new layout in `build_chart_scene()`. This requires re-running the scene build, not just rescaling the texture.
- **Dependencies on prior cards:** Decisions here build on the interactive-feedback card (Transition, HighlightState, find_nearest) and the interactive-navigation card (NavigationState, pan/zoom handlers). Both are complete per their progress.md files. The v2 Element impl is the consumer that wires these capabilities into the GPUI event loop.
