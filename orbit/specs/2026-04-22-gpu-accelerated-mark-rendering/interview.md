# Design Interview — Card 0013: GPU-Accelerated Mark Rendering

**Card:** `orbit/cards/0013-gpu-accelerated-mark-rendering.yaml`
**Date:** 2026-04-22
**Mode:** Rally design (agent self-answers from card + codebase evidence; author approves at consolidated gate)

---

## Q1: What 2D rendering backend should brightfield use?

**Decision:** Vello (GPU compute 2D renderer by Linebender), rendering to a texture that GPUI composites as an image element.

**Rationale:** GPUI has no built-in 2D canvas (PR #42905 is pending upstream). gpui-plot is abandoned. Vello's GPU compute renderer handles the full range of 2D primitives (circles, lines, rects, area fills, text) at frame rate regardless of element count. The `vello::Scene` API is high-level: `fill()`, `stroke()`, `draw_glyphs()`. The render-to-texture pattern decouples chart rendering from GPUI internals. If GPUI canvas lands later, migration is a backend swap — `MarkRenderer` and `ScaleSet` are unchanged.

**Files affected:** New crate `crates/brightfield-render/` — depends on `vello`, `kurbo`, `peniko`, `skrifa`.

---

## Q2: How does the Vello-rendered texture reach GPUI?

**Decision:** CPU readback to GPUI image element for v1. On Apple Silicon (unified memory), this is near-free — no actual GPU-to-CPU copy. Re-render only on state change, not every frame.

**Rationale:** Shared GPU texture (zero-copy via Metal `IOSurface`) is optimal but requires understanding GPUI's internal Metal device management — high risk for v1. CPU readback via `buffer.map_read()` works on every platform and uses GPUI's stable public `ImageSource` API. Apple Silicon is the primary development target; unified memory makes readback a pointer cast. Discrete GPU optimisation is a future card.

**Files affected:** `crates/brightfield-ui/src/chart_element.rs` — GPUI element wrapper with texture management.

---

## Q3: Where are axes, grid lines, and legends rendered?

**Decision:** All-in-Vello. The entire chart — marks, axes, ticks, labels, grid, legend — is a single Vello scene rendered to one texture.

**Rationale:** Single rendering context = single coordinate system = no alignment seams between marks and axes. Vello handles text via `draw_glyphs()` with font shaping via `skrifa`. Tick placement is a pure computation (consumes `ScaleSet`, produces `Vec<Tick { value, label, position }>`) that runs before rendering. Mouse coordinates map to chart coordinates via a single affine transform.

**Files affected:** `crates/brightfield-render/src/axis.rs`, `crates/brightfield-render/src/legend.rs`, `crates/brightfield-render/src/grid.rs`.

---

## Q4: How does interaction hit 120 FPS during brush/hover?

**Decision:** Two-tier rendering — immediate overlay + deferred re-query. Mouse events during drag update an interaction state (brush rect, hovered point) rendered as an overlay in the Vello scene. No DuckDB query during drag. On brush release, emit a param update triggering `session.update_param()`, which re-queries affected marks.

**Target:** 120 FPS for single-chart interaction (matching Zed's frame rate). 60+ FPS floor for complex cross-filtered dashboards with multiple linked views.

**Rationale:** Overlay rendering is pure GPU work — no I/O, no query latency. This guarantees frame rate during interaction regardless of data complexity. DuckDB re-query on release keeps the interaction smooth. Matches Mosaic-web's pattern where crossfilter re-query fires on `postQuery` after interaction completes.

**Files affected:** `crates/brightfield-ui/src/interaction.rs` — event routing, interaction state, overlay rendering.

---

## Q5: What is the crate structure?

**Decision:** Two new crates:

1. **`brightfield-render`** (`crates/brightfield-render/`) — depends on `brightfield-spec` (AST types, ChannelMap), `arrow`, `vello`, `kurbo`, `peniko`. Owns: `MarkRenderer` impls, `ScaleSet`, axis/legend/grid scene building, tick computation. Produces: `vello::Scene`. Testable headless (render to pixel buffer, assert structural/pixel properties).

2. **`brightfield-ui`** (`crates/brightfield-ui/`) — depends on `brightfield-render`, `brightfield-engine`, `gpui`, `wgpu`. Owns: GPUI element wrappers, Vello-to-texture rendering, event routing, interaction state. This is the application shell.

**Rationale:** `brightfield-render` can be tested without GPUI, compiled without GPUI, and potentially reused for non-GPUI targets (headless PNG export). The `vello::Scene` boundary between the two crates is Vello's public API.

---

## Q6: What coordinate system and layout model?

**Decision:** Fixed margin model with Observable Plot defaults (top: 20, right: 20, bottom: 30, left: 40). Spec attributes (`marginLeft`, `marginTop`, etc.) override defaults. Plot area = element bounds minus margins. Scale ranges derived from plot area dimensions.

**Rationale:** Observable Plot's defaults work for the card's three scenarios (dot, bar, line). Adaptive margins (measuring text widths) are a natural follow-up once the rendering pipeline is proven — text measurement infrastructure will exist from axis label rendering.

**Files affected:** `crates/brightfield-render/src/layout.rs` — chart-internal layout (margins, plot area, legend placement).

---

## Summary of key files and symbols

### New crate: `crates/brightfield-render/`
- `src/lib.rs` — crate root, public API
- `src/mark.rs` — `MarkRenderer` trait + impls (DotRenderer, LineRenderer, BarRenderer, RectRenderer)
- `src/scale.rs` — `ScaleSet`, `Scale` types, `infer_scales()` function
- `src/axis.rs` — axis rendering, tick computation
- `src/legend.rs` — legend rendering
- `src/grid.rs` — grid line rendering
- `src/layout.rs` — chart-internal layout (margins, plot area rect)
- `src/scene.rs` — `build_chart_scene()` orchestrator: data → scales → marks + axes + legend → `vello::Scene`

### New crate: `crates/brightfield-ui/`
- `src/lib.rs` — crate root
- `src/chart_element.rs` — GPUI element wrapping Vello texture
- `src/interaction.rs` — event routing, brush/hover state, overlay rendering
- `src/app.rs` — GPUI application shell (window, session lifecycle)

### Modified files
- `Cargo.toml` — workspace members: add `brightfield-render`, `brightfield-ui`
- `crates/brightfield-spec/src/vocab.rs` — no changes (ImplStatus already set for Phase 1 marks)
- `crates/brightfield-engine/src/lib.rs` — no changes (Session API already returns RecordBatch)
