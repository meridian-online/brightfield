# Decision Pack — Card 0013: GPU-Accelerated Mark Rendering

Rally: **first visible output**.
Card: `orbit/cards/0013-gpu-accelerated-mark-rendering.yaml`.
Scope: deciding how brightfield renders Arrow record batches as GPU-accelerated interactive charts in a native window. Covers the 2D rendering backend, GPUI integration strategy, scale/axis/legend rendering, interaction at frame rate, and crate structure.

## What is already fixed (not up for debate here)

These are inherited from completed cards and the existing codebase:

- **GPUI as the application framework** (project brief commitment): the native window, event loop, and UI tree are GPUI. This card does not re-evaluate the choice of GPUI.
- **DuckDB execution engine** (card 0012): `Session::execute_mark()` returns `Vec<RecordBatch>`. Arrow record batches are the data transport. The engine crate (`brightfield-engine`) owns the connection; the renderer consumes owned `RecordBatch` data.
- **Mark family taxonomy** (card 0008, Decision 1): ~10 mark families (line, bar, dot, area, rect, rule, text, density, regression, geo) parameterised by axis orientation. `MarkLower` trait per family produces `QueryPlan`.
- **ChannelMap** (card 0008, Decision 2): typed channel extraction from mark options, bridging SQL lowering and rendering.
- **Scale inference from RecordBatch** (card 0008, Decision 5): `infer_scales(batch, channel_map, plot_attrs) -> ScaleSet` runs after query execution, before rendering.
- **MarkRenderer trait** (card 0008, Decision 4): `trait MarkRenderer { fn render(&self, batch: &RecordBatch, channel_map: &ChannelMap, scales: &ScaleSet) -> Vec<GpuiElement>; }` with one impl per mark family.
- **Single native binary** (card 0011): no webview, no HTTP server, no runtime dependencies beyond graphics drivers.
- **gpui-plot abandoned** (card reference): the proof-of-concept crate `gpui-plot` by JakkuSakura is not maintained and is not a viable foundation.

What this pack decides: which 2D rendering backend draws the actual geometry inside GPUI, how that backend integrates with GPUI's rendering pipeline, how axes/legends/grid are drawn, how interaction achieves 60+ FPS, and where the rendering crate lives in the workspace.

---

## Decision 1 — 2D rendering backend: Vello vs Lyon+wgpu vs GPUI canvas

### Context
GPUI provides a GPU-accelerated UI framework (Metal on macOS, Vulkan on Linux/Windows) but does not natively expose a general-purpose 2D canvas for drawing arbitrary paths, fills, and strokes. The card requires rendering positioned points, lines, rectangles, areas, and text glyphs at 60+ FPS. Three integration paths are referenced: Vello (GPU compute 2D renderer by Linebender), Lyon+wgpu (mature CPU tessellation piped to GPU), and the pending GPUI canvas PR (zed-industries/zed#42905).

### Options
- **A. Vello render-to-texture, composited into GPUI.** Use Vello to render the chart scene to a texture (via its `wgpu` backend), then composite that texture into a GPUI element. Vello handles path filling, stroking, anti-aliasing, and text via GPU compute shaders. The GPUI element wraps the texture as an image surface.
- **B. Lyon tessellation + wgpu, composited into GPUI.** Use Lyon to tessellate paths (lines, rects, circles) on CPU into triangle meshes, submit them via wgpu to a render texture, composite into GPUI. Text rendered separately via a glyph rasteriser (e.g. cosmic-text or GPUI's own text engine).
- **C. Wait for GPUI canvas PR (#42905).** Block on upstream support for arbitrary 2D drawing within GPUI's own rendering pipeline. Use GPUI's native path/fill/stroke primitives once available.

### Trade-offs
- **A (Vello)** — actively developed by the Linebender project (same team behind `peniko`, `kurbo`, `skrifa`). GPU compute rendering means path complexity does not degrade frame rate — ideal for dense scatter plots (card scenario: positioned points at correct x/y). Vello's `Scene` API is high-level: `fill()`, `stroke()`, `draw_glyphs()`. Render-to-texture integration with external compositors is a documented pattern (Vello examples include `with_winit` and headless rendering). Cost: adds a `wgpu` + Vello dependency alongside GPUI's own GPU stack; two GPU contexts in the same process. The texture handoff (Vello renders to wgpu texture, GPUI reads it) requires shared GPU memory or a CPU-side copy per frame. On macOS with Metal, `wgpu` and GPUI can share the same `MTLDevice` in principle, but this requires careful plumbing.
- **B (Lyon+wgpu)** — Lyon is mature and widely used for 2D tessellation. CPU tessellation is predictable (no GPU compute shader compilation). Triangle meshes are straightforward to submit via wgpu. Cost: CPU tessellation scales with path complexity — a 50K-point scatter plot generates 50K quads (200K triangles), tessellation taking ~10ms on a modern CPU. This is acceptable for static renders but re-tessellation on every frame during interaction (brush, pan) would miss the 60 FPS target. Incremental tessellation or caching mitigates this. Text is a separate concern — Lyon does not handle glyphs; a separate text pipeline is needed for axis labels and legends.
- **C (GPUI canvas)** — tightest integration (no texture handoff, shared GPU context, native event routing). Would give brightfield first-class access to GPUI's layout, hit-testing, and accessibility. Cost: PR #42905 is pending with no merge date. Blocking on upstream is a hard dependency on an external project's timeline. If the PR ships with a limited API (e.g. no gradient fills, no dashed strokes), brightfield would need workarounds. The card says "120 FPS interactive" — waiting months for an upstream PR violates "ship the loop before optimising it."

### Recommendation
**Option A (Vello).** Vello's GPU compute renderer handles the full range of 2D primitives this card needs (circles, lines, rects, area fills, text glyphs) at frame rate regardless of element count. The render-to-texture pattern decouples chart rendering from GPUI's own rendering pass — chart content is a texture that GPUI composites as an image element, avoiding any dependency on GPUI's internal drawing API. The Vello+wgpu dependency is significant but justified: it is the standard Rust 2D GPU rendering stack, actively maintained, and the same team ships `kurbo` (geometry primitives) and `peniko` (brush/colour types) which the chart renderer will use directly. If GPUI canvas lands later, migrating from Vello Scene API to GPUI canvas API is a rendering-backend swap, not an architectural change — the `MarkRenderer` trait and `ScaleSet` are unchanged.

---

## Decision 2 — GPUI integration: texture handoff strategy

### Context
Given Decision 1 (Vello), the chart scene renders to a GPU texture via wgpu. GPUI must display this texture in its element tree. GPUI renders via Metal (macOS) or Vulkan (Linux) through its own GPU abstraction (`gpui::Window`, `gpui::Element`). The question is how the Vello-rendered texture reaches GPUI's compositor.

### Options
- **A. CPU readback + GPUI image element.** Vello renders to a wgpu texture, reads pixels back to CPU (`buffer.map_read()`), creates a GPUI `ImageSource` from the pixel data. GPUI composites the image like any other image element.
- **B. Shared GPU texture via platform API.** On macOS, both wgpu (Metal backend) and GPUI use Metal. Share the `MTLTexture` handle between Vello's wgpu device and GPUI's Metal context using `IOSurface` or direct texture import. Zero-copy GPU-to-GPU.
- **C. Vello renders directly into GPUI's render pass.** Inject Vello's GPU compute dispatch into GPUI's command buffer, rendering into GPUI's own surface. Requires deep integration with GPUI internals.

### Trade-offs
- **A (CPU readback)** — works on every platform, no shared-context complexity. GPUI's `InteractiveImage` / raw image element API accepts pixel buffers. Cost: GPU-to-CPU readback is the bottleneck. For a 1920x1080 chart at 4 bytes/pixel, that is ~8MB per frame. At 60 FPS, this is ~480 MB/s of PCIe traffic. On Apple Silicon (unified memory), the "readback" is effectively a pointer cast — no actual copy. On discrete GPUs, this is a real cost. Mitigation: only re-render when data or interaction state changes, not every frame; GPUI can cache the texture between frames.
- **B (shared GPU texture)** — zero-copy, best performance. Cost: platform-specific (Metal-only path, Vulkan-only path), requires understanding GPUI's internal Metal/Vulkan device management, and is fragile if GPUI changes its GPU abstraction. The Zed codebase does not currently expose hooks for external texture import. High implementation risk for v1.
- **C (render pass injection)** — theoretically optimal but requires forking or deeply instrumenting GPUI's rendering pipeline. Not feasible without upstream cooperation.

### Recommendation
**Option A (CPU readback) for v1, with a platform-specific fast path on Apple Silicon.** On Apple Silicon, wgpu's Metal backend and the system's unified memory architecture mean "CPU readback" is a virtual operation — the texture data is already in shared memory. The 8MB-per-frame concern applies only to discrete GPUs, and even there, re-rendering only on state change (not every frame) keeps the bandwidth modest. GPUI's image element API is stable and public. This approach ships immediately with no GPUI internals dependency. A future card can add Option B as an optimisation once the rendering pipeline is proven. This follows "ship the loop before optimising it."

---

## Decision 3 — Axis, grid, and legend rendering approach

### Context
The card requires axes with tick marks, labels, and grid lines reflecting data domain and scale type (linear, band, time). It also requires a colour legend showing colour-to-value mapping. These are 2D drawing tasks (lines for ticks/grid, text for labels, coloured rectangles for legend swatches). The question is whether axes/legends are drawn in the Vello scene alongside marks, or as GPUI native UI elements outside the chart texture.

### Options
- **A. Axes and legends in the Vello scene.** Everything inside the chart boundary — marks, axes, ticks, labels, grid, legend — is rendered by Vello into the same texture. The chart element is a single opaque image in GPUI's tree.
- **B. Hybrid: marks in Vello, axes/legends as GPUI elements.** The Vello texture covers only the plot area (data region). Axes, tick labels, and legends are GPUI text/box elements laid out around the texture using GPUI's native layout system.
- **C. Everything as GPUI elements.** No Vello — marks are also GPUI elements (one element per data point). Axes/legends are GPUI elements.

### Trade-offs
- **A (all-in-Vello)** — single rendering context, single coordinate system, no alignment seams between marks and axes. Vello handles text glyphs natively (`draw_glyphs()` with font shaping via `skrifa`). Grid lines are simple stroked paths. Cost: axis tick label layout (deciding which ticks to show, spacing, rotation for dense time axes) must be implemented in Rust — no reuse of GPUI's text layout engine. Tooltip/hover interaction requires mapping GPUI mouse events back into Vello scene coordinates.
- **B (hybrid)** — leverages GPUI's text rendering for labels (kerning, font fallback, accessibility), GPUI's layout for positioning axes relative to the plot area. Cost: alignment between GPUI-laid-out tick positions and Vello-rendered grid lines requires precise coordinate synchronisation. Two rendering systems must agree on the exact pixel positions of scale boundaries. Any mismatch produces visual artefacts (grid lines not aligned with tick labels). Complexity increases for every layout change (resize, legend toggle).
- **C (all-GPUI)** — simplest architecture but violates the card's 60+ FPS requirement for dense plots. A 10K-point scatter plot means 10K GPUI elements — GPUI's element diffing would dominate frame time. Not viable for the data densities in the card's scenarios.

### Recommendation
**Option A (all-in-Vello).** Rendering the entire chart — marks, axes, grid, legend — in a single Vello scene eliminates coordinate-system seams and keeps the rendering path simple. Vello's text rendering via `draw_glyphs()` is sufficient for axis labels and legend text; the font shaping stack (`skrifa` + `peniko`) handles Latin/numeric glyphs required for data labels. Tick placement logic (choosing tick values, formatting labels, handling overlaps) is a pure computation that runs before rendering — it consumes the `ScaleSet` and produces a `Vec<Tick { value, label, position }>` that the scene builder draws as lines and text. This computation is mark-independent and shared across all chart types. The single-texture approach also simplifies interaction: mouse coordinates map to chart coordinates via a single affine transform stored on the chart element.

---

## Decision 4 — Interaction architecture: event routing and frame-rate rendering

### Context
The card requires brush and hover interactors at 60+ FPS. GPUI delivers mouse/keyboard events to elements. The chart is a single GPUI element (an image from Vello). Interaction requires: (1) mapping GPUI mouse events to data coordinates, (2) updating visual feedback (selection rectangle, highlight) without re-querying DuckDB, (3) triggering DuckDB re-query when the interaction completes (brush release). The brief specifies "sub-100ms filter response times" for cross-filtered dashboards.

### Options
- **A. Two-tier rendering: immediate overlay + deferred re-query.** Mouse events update an interaction state (brush rect, hovered point) that is rendered immediately by re-drawing the Vello scene with an overlay (selection rectangle, highlight). No DuckDB query during drag. On brush release, emit a param update triggering `session.update_param()`, which re-queries and re-renders affected marks.
- **B. Query-on-every-frame during interaction.** Every mouse move during a brush drag triggers a param update and DuckDB re-query. The rendered chart always reflects the current selection.
- **C. Debounced re-query.** Mouse moves are debounced (e.g. 100ms). During the debounce window, the overlay shows the current brush position; after the debounce, DuckDB re-queries.

### Trade-offs
- **A (immediate overlay + deferred)** — guarantees 60+ FPS during interaction because the overlay is pure rendering (a rectangle or highlight colour applied to existing geometry), no I/O. DuckDB re-query happens only on release, keeping the interaction smooth. This is how Mosaic-web works: brush drag updates a selection param, but crossfilter re-query fires on `postQuery` after the brush interaction completes. Cost: during a brush drag, linked views do not update — the user sees the final result only on release. For cross-filtered dashboards, this means a brief stale period during drag.
- **B (query-every-frame)** — most responsive: linked views update continuously during drag. Cost: at 60 FPS, this is 60 DuckDB queries/second. DuckDB handles simple aggregations in <5ms (card 0012 Decision 4 notes sub-millisecond rebind), so for simple specs this works. For complex specs with multiple linked views (crossfilter.yaml has 3 marks), 60 * 3 = 180 queries/second — DuckDB can likely sustain this for aggregated queries but it taxes CPU and risks frame drops if any query exceeds 16ms.
- **C (debounced)** — compromise: overlay during debounce window, re-query after. Cost: 100ms debounce is perceptible lag in linked-view updates. Tuning the debounce interval is a UX question.

### Recommendation
**Option A for v1, with Option C as a future enhancement.** Immediate overlay rendering during interaction (brush rectangle, point highlight) keeps the frame rate at 60+ FPS unconditionally. DuckDB re-query fires on interaction completion (brush release, hover dwell). This matches the card's scenario: "the visual feedback responds at 60+ FPS without perceptible lag" — the feedback is the overlay, not the re-queried data. For the crossfilter scenario, linked views update on brush release, which is the standard UX pattern in Observable Plot and Vega-Lite. A future card can add Option C (debounced live re-query during drag) for specs that benefit from continuous feedback, gated by a spec attribute or a performance heuristic.

---

## Decision 5 — Crate structure: where does the rendering code live

### Context
The workspace currently has four crates: `brightfield-spec` (parse), `brightfield-sql` (emit), `brightfield-conformance` (test), `brightfield-engine` (execute). The rendering layer is a new concern with significant dependencies (Vello, wgpu, GPUI, kurbo, peniko). It consumes `Vec<RecordBatch>` from the engine and produces GPUI elements. The question is whether it lives in a new crate or in an existing one.

### Options
- **A. New crate `brightfield-render` at `crates/brightfield-render/`.** Depends on `brightfield-spec` (for AST types, ChannelMap), `brightfield-engine` (for RecordBatch re-export), plus `vello`, `wgpu`, `kurbo`, `peniko`. Does NOT depend on `gpui` — it produces a pixel buffer or texture that the application layer composites.
- **B. New crate `brightfield-ui` that owns both GPUI integration and rendering.** Depends on everything: `gpui`, `vello`, `wgpu`, all upstream brightfield crates. The "application" crate.
- **C. Split into two new crates: `brightfield-render` (Vello scene building) and `brightfield-ui` (GPUI element wrappers).** `brightfield-render` produces a `vello::Scene`; `brightfield-ui` renders it to texture and wraps in GPUI elements.

### Trade-offs
- **A (brightfield-render)** — clean separation: the render crate knows how to turn Arrow data + scales into a 2D scene and pixel output, but knows nothing about GPUI. Testable without a window (render to an in-memory buffer, compare pixels or structural scene output). Cost: the GPUI integration (element wrapper, event routing from Decision 4) must live elsewhere — either in a separate crate or in the binary crate.
- **B (brightfield-ui)** — everything in one place. Loses: the crate becomes large and GPUI-dependent, making headless testing impossible. Compilation slows for any change.
- **C (two crates)** — maximally separated: `brightfield-render` is testable headless, `brightfield-ui` is a thin GPUI adapter. Cost: two new crates, and the interface between them (a `vello::Scene` or pixel buffer) must be well-defined.

### Recommendation
**Option C.** Two crates with a clean boundary:
- `brightfield-render`: depends on `brightfield-spec`, `arrow`, `vello`, `kurbo`, `peniko`. Owns: `MarkRenderer` impls, `ScaleSet`, axis/legend scene building, tick computation. Produces: `vello::Scene` (the chart's visual representation as a Vello scene graph). Testable headless by rendering to a pixel buffer and asserting structural or pixel-level properties.
- `brightfield-ui`: depends on `brightfield-render`, `brightfield-engine`, `gpui`, `wgpu`. Owns: GPUI element wrappers, Vello-to-texture rendering, event routing (Decision 4), interaction state. This is the application shell.

This split means `brightfield-render` can be tested without GPUI, compiled without GPUI, and potentially reused for non-GPUI targets (e.g. a headless PNG export mode, which the brief mentions as a future goal). The `vello::Scene` boundary between the two crates is Vello's own public API — stable and well-documented.

---

## Decision 6 — Chart coordinate system and layout model

### Context
A rendered chart has distinct regions: the plot area (where marks are drawn), margins (where axes and labels live), and optional legend area. GPUI provides the outer bounds (the element's allocated size). The chart must partition this space, compute scale ranges (pixel extents) from the plot area dimensions, and position all elements accordingly. Observable Plot uses a margin model: `marginTop`, `marginRight`, `marginBottom`, `marginLeft` default to values that accommodate axes, and the plot area fills the remainder.

### Options
- **A. Fixed margin model with defaults.** Define default margins (e.g. top: 20, right: 30, bottom: 40, left: 50) that accommodate typical axis label widths. Spec attributes can override. Plot area = element bounds minus margins. Scale ranges are derived from the plot area dimensions.
- **B. Adaptive margin model.** Measure tick label text widths (using Vello's font metrics) and compute margins dynamically. A y-axis with values like "1,000,000" gets a wider left margin than one with values "0-9".
- **C. Constraint-based layout.** Define the chart as a set of regions with constraints (axis width = max label width + tick length + padding) resolved by a layout solver.

### Trade-offs
- **A (fixed margins)** — simplest. Observable Plot uses this model with sensible defaults that work for most charts. Cost: long tick labels may be clipped; very short labels waste space. The 80% case is covered.
- **B (adaptive margins)** — better use of space, no clipping. Cost: requires a text measurement pass before layout (measure all tick labels, find max width). Vello's `skrifa` provides font metrics and glyph advance widths, so this is feasible. Adds a layout-measure-render three-phase cycle instead of a simpler two-phase.
- **C (constraint-based)** — most flexible, handles edge cases (nested facets, multi-axis charts). Cost: a layout solver is heavy machinery for v1 where the card targets single-plot charts. Faceting and multi-view layout are separate cards (0009).

### Recommendation
**Option A for v1 with a path to Option B.** Ship with Observable Plot's default margins (top: 20, right: 20, bottom: 30, left: 40, matching Plot's defaults). Spec attributes (`marginLeft`, `marginTop`, etc.) override defaults. The plot area is the remaining rectangle; scale ranges (`x: [left_margin, width - right_margin]`, `y: [top_margin, height - bottom_margin]`) are derived directly. This is sufficient for the card's three scenarios (dot, bar, line) and matches Observable Plot's behaviour, which the Mosaic spec format assumes. Option B (adaptive margins based on label measurement) is a natural follow-up once the rendering pipeline is proven — the text measurement infrastructure will already exist from axis label rendering.

---

## Summary table

```
| #  | Decision                               | Recommendation                                                                |
|----|----------------------------------------|-------------------------------------------------------------------------------|
| 1  | 2D rendering backend                   | Vello (GPU compute 2D renderer), render-to-texture into GPUI                  |
| 2  | GPUI integration (texture handoff)     | CPU readback for v1; unified memory makes this near-free on Apple Silicon      |
| 3  | Axis/grid/legend rendering             | All-in-Vello: single scene, single coordinate system, no alignment seams      |
| 4  | Interaction architecture               | Immediate overlay + deferred re-query; 60+ FPS guaranteed during interaction   |
| 5  | Crate structure                        | Two new crates: `brightfield-render` (headless, Vello scene) + `brightfield-ui` (GPUI shell) |
| 6  | Chart coordinate system and layout     | Fixed margin model with Observable Plot defaults; spec overrides supported     |
```

## Cross-cutting notes

- **Dependency on card 0008 (mark library):** Decisions 1 and 3 here implement the `MarkRenderer` trait defined in card 0008 Decision 4. The `MarkRenderer::render()` method produces Vello scene fragments (not raw `GpuiElement` as originally sketched in 0008 — the return type should be `vello::Scene` or a scene fragment type). This is a refinement of 0008's rendering architecture decision, not a contradiction.
- **Dependency on card 0012 (execution engine):** The rendering pipeline receives `Vec<RecordBatch>` from `Session::execute_mark()`. The interaction architecture (Decision 4) calls `session.update_param()` on brush release. The engine's prepared statement cache (0012 Decision 4) ensures re-queries on param change are fast.
- **Vello version pinning:** Vello is pre-1.0. Pin to a specific commit or minor version and vendor if needed. The `vello::Scene` API is relatively stable but breaking changes are possible. The two-crate split (Decision 5) contains Vello dependency to `brightfield-render` only.
- **Text rendering:** Vello uses `skrifa` (font parsing) and `peniko` (brush types) for text. Axis labels and legend text are drawn via `scene.draw_glyphs()`. A system font (e.g. the platform's default sans-serif) should be loaded at startup. Custom font support is out of scope for v1.
- **Apple Silicon fast path:** Decision 2 notes that CPU readback on Apple Silicon unified memory is near-free. This covers the primary development and deployment target. Discrete GPU performance can be optimised in a later card if profiling shows the readback is a bottleneck.
- **Out of scope for this card:** multi-view composition (card 0009), input widgets (card 0005), faceting, annotation layers, PNG/SVG export, accessibility, responsive resize behaviour. These consume the rendering pipeline but are not part of the GPU mark rendering card.
