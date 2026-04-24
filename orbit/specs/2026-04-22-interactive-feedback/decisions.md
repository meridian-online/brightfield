# Decision Pack — Card 0010: Interactive Feedback and Focus

Rally: **interactive navigation and feedback**.
Card: `orbit/cards/0010-interactive-feedback-and-focus.yaml`.
Scope: deciding how brightfield implements hover/tooltip for nearest-point detail, highlight/dim for selection focus, and smooth easing-based transitions on data or selection updates.

## What is already fixed (not up for debate here)

These are inherited from completed cards and the existing codebase:

- **InteractionState enum** (`crates/brightfield-ui/src/interaction.rs:14-29`): `Idle | Brushing { start, current } | Hovering { point }`. Already renders a hover highlight circle and brush overlay into a Vello `Scene`. The hover state stores a raw chart-coordinate point but does not resolve to the nearest data point.
- **ChartElement** (`crates/brightfield-ui/src/chart_element.rs`): wraps a `vello::Scene` + `InteractionState` for GPUI display. Provides `set_interaction()` to update state.
- **MarkRenderer trait** (`crates/brightfield-render/src/mark.rs`): `render(&self, scene, batch, channel_map, scales)` for Dot, Bar, Line families. Each renderer iterates over `RecordBatch` rows and emits Vello fill/stroke operations with per-row positions resolved through scales.
- **Scene builder** (`crates/brightfield-render/src/scene.rs`): `build_chart_scene()` orchestrates grid -> marks -> axes -> legend. Returns `(Scene, ScaleSet)` — the `ScaleSet` is available for coordinate inversion.
- **GPUI dependency** (`crates/brightfield-ui/Cargo.toml`): `gpui` from the Zed repo is a direct dependency. GPUI provides `with_animation()`, easing functions, and `AnimationElement` for frame-driven transitions.
- **Interactor vocabulary** (`crates/brightfield-spec/src/vocab.rs:195-214`): `Nearest`, `NearestX`, `NearestY`, `Highlight` are registered as `Unimplemented` interactor kinds.
- **Opacity channel** (`crates/brightfield-spec/src/parse.rs:53-55`): `opacity`, `fillOpacity`, `strokeOpacity` are recognised parse keys but have no rendering support.

---

## D1 — Nearest-point resolution: where and how is the closest data point found?

### Context

The card's first scenario says "a tooltip appears showing the nearest point's details" on hover. The current `InteractionState::Hovering { point }` stores a raw cursor position in chart coordinates but does not identify which data row is closest. Resolution requires: (a) a spatial index or scan over the data positions, (b) access to the original `RecordBatch` and `ScaleSet` to map between pixel and data coordinates, and (c) a decision about what "nearest" means for different mark types (Euclidean distance for dots, x-only snapping for lines, band membership for bars). Mosaic's `nearest` interactor snaps to the closest point in x-only or y-only by default, with `nearestXY` for 2D proximity.

### Options

- **A. Brute-force scan at hover time.** On each `mousemove`, iterate the `RecordBatch` rows, map each row to pixel coordinates via `ScaleSet`, compute distance to the cursor, and return the closest row index. No persistent spatial index.
- **B. Build a spatial index on scene construction.** After `build_chart_scene()`, construct a lightweight lookup structure (e.g. a flat sorted-by-x list for 1D nearest, or a k-d tree for 2D) from the rendered mark positions. Query the index on hover.
- **C. GPU-side hit testing via colour-coded pick buffer.** Render each mark with a unique colour ID into an off-screen buffer; read the pixel under the cursor to determine the hit mark/row.

### Trade-offs

- **A (brute-force)** — zero additional memory, zero build cost. For the record batch sizes that survive SQL aggregation (typically hundreds to low thousands of rows after pre-aggregation), a linear scan at 60Hz is well under 1ms. Cost: scales linearly with row count; breaks down for raw scatter plots over 100K+ unaggregated rows. But the database-first design principle means renderers receive aggregated data, keeping n small.
- **B (spatial index)** — O(log n) query at hover time. Cost: index build on every re-render (data change, brush update). For n < 10K (post-aggregation), the build cost exceeds the per-frame savings from brute-force. A k-d tree or sorted list also adds a data structure and invalidation lifecycle. Worth it only if n regularly exceeds ~10K rendered points.
- **C (pick buffer)** — constant-time lookup regardless of point count. Cost: requires a second render pass into an off-screen texture, doubling GPU work per frame. Vello does not natively support per-primitive IDs in its scene encoding; implementing this would require a custom shader pass outside Vello's pipeline. Significantly more complex than the problem warrants at current data scales.

### Recommendation

**Option A for v1, with the seam left for B later.** The database-first principle (README: "All data-intensive computation is pushed to DuckDB. The renderer receives only the minimal data needed for display") guarantees that rendered point counts stay modest. A brute-force scan over a few hundred aggregated rows per `mousemove` is negligible. The implementation shape is a function `find_nearest(cursor: Point, batch: &RecordBatch, channel_map: &ChannelMap, scales: &ScaleSet, mode: NearestMode) -> Option<NearestHit>` where `NearestMode` is `X | Y | XY` (matching the `Nearest/NearestX/NearestY` interactor variants). This function lives in `brightfield-render` (it needs `ScaleSet` and `RecordBatch`, not GPUI). If profiling later shows hover jank on large unaggregated datasets, upgrade to option B by swapping the scan for a sorted-list lookup — the API surface (`find_nearest`) stays the same.

---

## D2 — Tooltip rendering: Vello scene overlay or GPUI element?

### Context

Once the nearest point is identified, a tooltip showing its data values must appear. Two rendering paths exist: (a) draw the tooltip as Vello primitives (rectangles + text glyphs) into the chart's `Scene`, or (b) render the tooltip as a GPUI element overlaid on the chart. The choice determines where tooltip styling, layout, and text shaping live.

### Options

- **A. Vello scene overlay.** Extend `InteractionState::render_overlay()` to draw a tooltip background rectangle and text glyphs using Vello's `Scene::fill()` and glyph rendering. The tooltip is part of the chart scene, rendered in the same GPU pass.
- **B. GPUI overlay element.** The tooltip is a separate GPUI `div` (or custom element) positioned above the chart element using GPUI's layout system. Text rendering uses GPUI's native text shaping (which uses the system font stack). The chart scene and tooltip are composited by GPUI.
- **C. Hybrid — Vello highlight on the chart, GPUI element for the text.** The nearest-point highlight circle stays in the Vello scene (as it is today). The text tooltip is a GPUI element positioned at the highlight's screen coordinates.

### Trade-offs

- **A (Vello-only)** — keeps everything in one render pass; no coordinate mapping between Vello and GPUI. Cost: text rendering in Vello requires loading font data, shaping runs, and positioning glyphs manually — Vello's text API is lower-level than GPUI's. Tooltip layout (multi-line, column alignment, padding) would need hand-rolled layout logic. The current `render_overlay()` already uses this path for the hover circle, but text is a significant step up in complexity.
- **B (GPUI element)** — GPUI's text rendering is production-quality (it powers Zed's entire UI). Layout, styling, and theming are solved problems in GPUI. Cost: requires mapping chart-pixel coordinates to GPUI layout coordinates for positioning, and the tooltip element lives outside the chart's Vello scene, adding a layer to the rendering architecture.
- **C (hybrid)** — best of both worlds: the highlight (simple geometry) stays in Vello where it's cheap; the text tooltip (complex layout) uses GPUI where it's mature. Cost: two coordinate systems to bridge (chart-local Vello coords and GPUI screen coords), and the tooltip's lifecycle spans both rendering paths.

### Recommendation

**Option C.** The highlight circle already lives in Vello (see `interaction.rs:96-98`) and should stay there — it's simple geometry tightly coupled to chart coordinates. The text tooltip should be a GPUI element because: (1) GPUI handles text shaping, font fallback, and multi-line layout natively — reimplementing this in Vello is wasted effort that violates the "use the framework's primitives" principle; (2) Zed's own tooltips use GPUI elements, providing a proven pattern; (3) the tooltip content (field names + values) benefits from GPUI's styling system for consistent typography with the rest of the application shell. The coordinate bridge is straightforward: `ChartElement` knows its position in GPUI's layout tree, and the highlight point in chart coordinates can be translated to GPUI screen coordinates via the element's bounds.

---

## D3 — Highlight/dim mechanism: per-mark opacity or separate render passes?

### Context

The card's second scenario says "marks outside the selection are visually dimmed so the selection stands out." This requires rendering selected marks at full opacity and non-selected marks at reduced opacity (or greyed out). The `Highlight` interactor in Mosaic applies an opacity reduction to non-matching marks. The current `MarkRenderer` implementations render all rows identically — there is no per-row visual variation based on selection state.

### Options

- **A. Per-row opacity parameter in MarkRenderer.** Add an `opacity: f64` per row to the rendering loop. When a highlight selection is active, rows matching the selection render at opacity 1.0; non-matching rows render at a reduced opacity (e.g. 0.2). The `MarkRenderer::render()` signature gains an `Option<&HighlightState>` parameter. Each renderer applies the opacity to its `Color` alpha channel before calling `scene.fill()`.
- **B. Two-pass rendering: full scene at reduced opacity, then selected marks at full opacity.** Render all marks at dimmed opacity first, then re-render only the selected subset at full opacity on top. The second pass overwrites the dimmed marks for selected rows.
- **C. Post-processing opacity mask.** Render the full scene normally, then apply a semi-transparent overlay mask that covers non-selected regions, dimming them. The mask shape is derived from the selection bounds.

### Trade-offs

- **A (per-row opacity)** — most precise: each individual mark gets the correct opacity regardless of overlap or mark type. Cost: every renderer must check selection membership per row, adding a branch to the inner loop. The branch is cheap (a boolean lookup or range check per row), and the renderer already iterates per-row. Vello's colour model supports per-fill alpha natively (`Color::new([r, g, b, alpha])`). This is how Observable Plot's `highlight` interactor works — it sets CSS opacity per mark element.
- **B (two-pass)** — simpler per-renderer logic (no per-row branching), but doubles the draw calls for selected marks and produces incorrect results where selected and non-selected marks overlap (the full-opacity selected mark composites over the dimmed one, changing the effective colour). Also, partial bar overlap or adjacent marks with anti-aliasing would show dimmed edges bleeding through.
- **C (mask overlay)** — works for rectangular selections but fails for point-based highlight (nearest-point highlight dims everything except one point — a mask can't express an arbitrary set of excepted marks). Also, the mask approach doesn't distinguish individual marks; all non-selected content including axes and grid lines would be dimmed.

### Recommendation

**Option A.** Per-row opacity is the correct semantic match for Mosaic's `highlight` interactor. The implementation is minimal: (1) define a `HighlightState { predicate: Box<dyn Fn(usize) -> bool> }` that tests whether row `i` is in the selection; (2) in each `MarkRenderer::render()`, if a `HighlightState` is present, set the alpha channel to `dimmed_alpha` (e.g. 0.15) for non-matching rows and leave it at the original alpha for matching rows. The existing colour resolution (`resolve_colour()` in `mark.rs:100-118`) returns a `Color` — multiplying its alpha by the dim factor is one line. This approach scales to any mark type, composes correctly with existing colour scales, and matches how Observable Plot handles highlight.

---

## D4 — Animation system: GPUI's `with_animation` or custom interpolation loop?

### Context

The card's third scenario says "marks transition smoothly rather than snapping, using easing-based animation" when data or selections change. The brief explicitly names "GPUI's built-in animation system (easing functions, transformations, repeating animations)" as the mechanism. GPUI provides `with_animation()` on elements, easing functions (ease-in, ease-out, ease-in-out, linear, spring), and `AnimationElement` for frame-driven transitions. However, the current rendering path builds a complete Vello `Scene` from scratch on every data change — there is no concept of "previous state" to interpolate from.

### Options

- **A. GPUI `with_animation()` on the chart element.** Treat the entire chart as a single animated element. On data change, GPUI interpolates between the old and new rendered state using its built-in easing. The chart element exposes animatable properties (e.g. opacity for fade transitions, transform for slide transitions).
- **B. Mark-level interpolation in the rendering pipeline.** Maintain a "previous positions" buffer alongside the current `RecordBatch`. On data change, the renderer interpolates between old and new mark positions over a duration, producing intermediate `Scene` frames. Easing functions (from GPUI or hand-rolled) control the interpolation curve. The animation drives `ChartElement::set_scene()` at frame rate.
- **C. Scene-level crossfade.** On data change, render both the old and new scenes. Blend them with a time-varying alpha (old fades out, new fades in) over a transition duration. No per-mark position interpolation; the transition is a visual dissolve.

### Trade-offs

- **A (GPUI element animation)** — leverages the framework as the brief intends. But GPUI's animation system operates on element properties (transform, opacity, layout values), not on the internal positions of Vello primitives within a scene. Animating the entire chart element as a unit produces effects like fade-in or slide-in of the whole chart, not smooth per-mark movement. This doesn't match the card's intent ("marks transition smoothly").
- **B (mark-level interpolation)** — produces the correct visual: marks glide from old to new positions, bars grow/shrink, lines morph. Cost: requires maintaining a "previous state" (old positions per mark) and a time-driven interpolation loop. The interpolation can use GPUI's easing functions for the curve shape while driving the actual per-mark position updates in Rust. Each animation frame calls `build_chart_scene()` with interpolated data positions. This is the approach used by D3.js transitions and Observable Plot's `transition` option.
- **C (crossfade)** — visually acceptable for many cases (a dissolve between chart states is a common dashboard pattern). Cost: doesn't convey *which* marks changed — a bar growing vs disappearing looks the same through a dissolve. Fails the card's scenario which specifies marks "transition smoothly", implying positional continuity.

### Recommendation

**Option B, using GPUI's easing functions for the curve but driving interpolation in the render pipeline.** The animation lifecycle is: (1) on data change, snapshot the current per-mark positions as `prev_positions: Vec<(f64, f64)>`; (2) compute new positions from the new `RecordBatch` + `ScaleSet`; (3) start a transition with a duration (e.g. 300ms) and an easing function from GPUI (e.g. `ease_in_out`); (4) on each frame, compute `t = easing(elapsed / duration)`, interpolate each mark's position as `lerp(prev, next, t)`, build the scene with interpolated positions, and call `set_scene()`; (5) when `t >= 1.0`, the transition completes and the scene settles to the final state. GPUI's `request_animation_frame()` or equivalent drives the frame loop. This uses the framework's easing primitives (satisfying the brief) while keeping per-mark interpolation in the rendering pipeline where the data lives. The `MarkRenderer` trait gains an optional `render_interpolated()` method with a default impl that falls back to `render()` (no animation).

---

## D5 — Transition scope: what animates and what snaps?

### Context

Not all visual changes should animate. A tooltip appearing on hover should be immediate; a brush overlay should track the cursor in real-time with no lag. But when the underlying data changes (a filter update, a selection change that triggers a re-query, a slider drag that re-bins), mark positions should transition smoothly. The card needs a clear rule for which state changes animate and which are immediate.

### Options

- **A. Animate data-driven changes only.** Mark positions, bar heights, line paths, and area fills animate when the `RecordBatch` changes. Interaction overlays (brush rect, hover circle, tooltip) are always immediate. Selection highlight/dim opacity changes are immediate (snap to dimmed/full).
- **B. Animate data-driven changes and selection highlight transitions.** Same as A, but opacity changes from highlight/dim also animate (smooth fade to dimmed state rather than snapping). Interaction overlays remain immediate.
- **C. Animate everything.** All visual changes — including brush overlays, tooltip appearance, highlight state — use easing transitions.

### Trade-offs

- **A (data-only)** — simplest scope. Animating mark positions is the highest-value transition (it shows the viewer *what changed* in the data). Immediate highlight/dim is acceptable — Observable Plot's highlight interactor snaps opacity without transition. Cost: highlight transitions can feel abrupt, especially when toggling between selection states.
- **B (data + highlight)** — adds polish to selection interactions. An opacity fade from 1.0 to 0.15 over ~150ms is noticeable and communicates the selection state change more clearly than a snap. Cost: the highlight animation must be short (under 200ms) to not impede interaction speed, and the animation system must handle concurrent transitions (data changing while a highlight fade is in progress).
- **C (everything)** — maximum smoothness. Cost: animating brush overlays introduces latency between cursor movement and visual feedback, which directly degrades the "fluid interaction" promise. Tooltip animation (fade-in) adds perceived latency to information retrieval. The interaction module's current `render_overlay()` is designed for zero-latency immediate rendering; adding animation would undermine this.

### Recommendation

**Option B.** Data-driven mark transitions are the primary animation (D4). Highlight/dim opacity transitions are a lightweight addition — the alpha interpolation is a single `lerp()` per mark per frame, piggybacking on the existing per-row opacity from D3. Use a short duration (100-150ms) for highlight fades so they feel responsive. Brush overlays, hover highlights, and tooltip appearance remain immediate — these are interaction-feedback primitives where latency is the enemy. The rule: **if the change is driven by incoming data (RecordBatch update), animate; if the change is driven by direct user input (cursor position, brush drag), render immediately; if the change is selection state (highlight/dim), animate briefly.**

---

## Summary table

```
| #  | Decision                              | Recommendation                                                                   |
|----|---------------------------------------|----------------------------------------------------------------------------------|
| D1 | Nearest-point resolution              | Brute-force scan over post-aggregation RecordBatch; seam for spatial index later |
| D2 | Tooltip rendering                     | Hybrid: Vello highlight circle + GPUI element for text tooltip                   |
| D3 | Highlight/dim mechanism               | Per-row opacity in MarkRenderer; dimmed alpha for non-selected rows              |
| D4 | Animation system                      | Mark-level interpolation in render pipeline; GPUI easing functions for curves    |
| D5 | Transition scope                      | Animate data changes + highlight fades; immediate for overlays and tooltips      |
```

## Cross-cutting notes

- **Interaction with card 0007 (interactive navigation):** Pan/zoom changes the visible extent and triggers a re-query (card 0007 scenario 5). The re-query delivers a new `RecordBatch`, which triggers D4's mark transition. The zoom-settle -> re-query -> animated mark transition is the full pipeline. These cards share the `InteractionState` enum and should coordinate on extending it.
- **InteractionState needs extension:** The current enum (`Idle | Brushing | Hovering`) will need a `Highlighting` variant or a parallel `HighlightState` to carry selection membership information for D3. This is additive — existing variants are unaffected.
- **MarkRenderer signature change:** D3 adds `Option<&HighlightState>` to `render()` and D4 adds `render_interpolated()`. These are the only breaking changes to the existing rendering API. Both can be introduced with backward-compatible defaults.
- **Opacity channel:** `crates/brightfield-spec/src/parse.rs` recognises `opacity`/`fillOpacity`/`strokeOpacity` as parse keys, and `Channel` in `channel.rs` does not yet include an opacity variant. D3's per-row opacity mechanism should compose with a future `Opacity` channel — the highlight dim factor multiplies the channel-driven opacity, not replaces it.
- **vocab.rs status transitions:** Implementing this card should flip `Nearest`, `NearestX`, `NearestY`, and `Highlight` from `Unimplemented` to `Implemented` in `crates/brightfield-spec/src/vocab.rs:200-206`.
