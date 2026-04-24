# Decision Pack — Card 0007: Interactive Navigation

Card goal: let an analyst pan and zoom inside a plot to move between overview and detail without editing the spec.

Scope: pan, zoom, zoom reset, axis-locked navigation, and zoom-triggered data re-query. The card's five scenarios all live at the intersection of `brightfield-ui` (GPUI event handling, interaction state), `brightfield-render` (scale domain management, scene rebuild), and `brightfield-engine` (re-query on zoom settle).

Evidence citations use repo-relative paths. Prior decisions referenced:
- `orbit/specs/2026-04-21-fluid-interaction-at-dataset-scale/decisions.md` (card 0003 — QueryPlan IR, prepared-statement caching, shape-cache keying).
- `orbit/specs/2026-04-21-cross-filtered-selections-across-linked-views/decisions.md` (card 0006 — cross-filter selection compilation).

Shipped-code touchpoints:
- `crates/brightfield-ui/src/interaction.rs` — `InteractionState::{Idle, Brushing, Hovering}` with `render_overlay`. No pan/zoom states exist.
- `crates/brightfield-ui/src/chart_element.rs` — `ChartElement { scene, interaction, width, height }`. Holds one scene; no scale-domain awareness.
- `crates/brightfield-render/src/scale.rs` — `Scale::{Linear, Band, Time, Colour}` with `map_f64`, `domain_min`, `domain_max`. No inverse mapping (pixel-to-data). No mutable domain.
- `crates/brightfield-render/src/scene.rs` — `build_chart_scene` takes `ChartData` and returns `(Scene, ScaleSet)`. Scales are inferred from data; no external domain override.
- `crates/brightfield-engine/src/lib.rs` — `Session::update_param` re-executes marks subscribing to a named parameter. No debounce, no zoom-aware re-query path.
- `crates/brightfield-spec/src/vocab.rs:208-213` — `InteractorKind::{Pan, PanX, PanY, PanZoom, PanZoomX, PanZoomY}`, all `Unimplemented`.
- `crates/brightfield-spec/src/ast.rs:308-315` — `Interactor { kind, status, options }`. Options bag is untyped `IndexMap<String, ValueOrParamRef<SpecValue>>`.

---

## D1 — Navigation state model: where does the current view extent live?

**Context.** Pan and zoom modify the visible data range. The current codebase infers scale domains from the full dataset (`infer_scales` in `scale.rs`), and the `Scale` enum's domain fields are immutable after construction. Navigation needs a mutable "current view extent" that overrides the data-inferred domain for rendering. This state must survive across frames (a drag produces many frames) and be resettable to the original extent (scenario 3: double-click reset).

**Options.**

- **A. View extent as mutable fields on `Scale`.** Add `view_domain_min`/`view_domain_max` to `Scale::Linear` and `Scale::Time`. `map_f64` uses the view domain when set, falls back to the data domain. Zoom/pan mutate the scale in place.
- **B. Separate `ViewExtent` struct alongside `ScaleSet`.** A `ViewExtent { x: Option<(f64, f64)>, y: Option<(f64, f64)> }` lives in `ChartElement` (or a new `NavigationState`) next to the `ScaleSet`. At render time, `build_chart_scene` receives the view extent and overrides scale domains before building. The `Scale` enum stays immutable — the override is applied at scene-build time.
- **C. Transform-based: apply an affine transform to the scene, not the scale domain.** Pan is a translation; zoom is a scale. The Vello scene is rendered at full extent and the GPUI element applies a `kurbo::Affine` to clip/transform the visible region.

**Trade-offs.**

- **A.** Simplest mutation path — pan/zoom directly update the scale, and `map_f64` automatically uses the new domain. Risk: `Scale` is currently `Clone` and passed by value in `ScaleSet`; making it mutable couples rendering to interaction state. Re-query (scenario 5) needs the view domain to build a `WHERE` clause — if the domain lives inside `Scale`, extracting it for SQL emission requires reaching into the render layer, breaking the render/sql/engine dependency chain.
- **B.** Clean separation: `ViewExtent` is a plain data struct that can flow both to the renderer (for domain override) and to the engine (for re-query `WHERE` clause). The `Scale` enum stays a value type. Cost: `build_chart_scene` needs an additional parameter, and there's a second source of truth for "what domain is the chart showing" (the scale vs the extent). Mitigated by making the override explicit — `build_chart_scene` applies it or not.
- **C.** Cheapest GPU-side — a single affine avoids re-rendering marks. But axis labels, tick positions, and grid lines all depend on the domain, so they'd be wrong (stretched/shifted) unless also re-rendered. For analytical charts (not maps), wrong axis labels are unacceptable. Also, affine zoom doesn't interact with re-query (scenario 5) — the engine has no idea what extent is visible.

**Recommendation: B.**

`ViewExtent` is a lightweight struct that both the renderer and the engine can consume without coupling. It flows from interaction (user drags) to rendering (domain override) and to the engine (zoom-settle re-query). The `Scale` enum stays immutable and inferrable from data. `build_chart_scene` gains an `Option<&ViewExtent>` parameter — `None` means "show full data extent" (current behaviour), `Some` means "override domain to this range". The original data-inferred domain is preserved in `ScaleSet` so that reset (scenario 3) is trivial: set `ViewExtent` back to `None`.

Evidence: `build_chart_scene` already returns `ScaleSet` alongside the `Scene` (`scene.rs:31`). The caller can store the original `ScaleSet` and derive `ViewExtent` deltas from it. The engine's `update_param` (`lib.rs:161-202`) already takes `SpecValue` parameters — a `ViewExtent` can be expressed as a param update for re-query without changing the engine API.

---

## D2 — Gesture-to-domain mapping: how do pixel-space gestures translate to data-space pan/zoom?

**Context.** Pan and zoom are pixel-space gestures (drag delta in px, scroll wheel delta). The current `Scale` has `map_f64` (data-to-pixel) but no inverse (pixel-to-data). Translating a 50px drag into a data-domain shift requires inverting the scale. This is straightforward for `Linear` and `Time` but undefined for `Band` (categorical axes cannot be continuously panned). The card's axis-lock scenario (scenario 4) requires knowing which axis a gesture applies to.

**Options.**

- **A. Add `inverse_map` to `Scale` for all variants.** `Scale::map_f64` goes data-to-pixel; `Scale::inverse_f64` goes pixel-to-data. `Band` and `Colour` return `None` (pan/zoom not applicable). `Linear` and `Time` compute the linear inverse.
- **B. Compute the inverse externally from domain/range.** A free function `pixel_to_data(pixel: f64, domain: (f64, f64), range: (f64, f64)) -> f64` that doesn't touch the `Scale` enum. The caller extracts domain/range from the scale and calls the function.
- **C. Work entirely in normalised [0,1] space.** Gestures produce a normalised delta (px_delta / range_width). Pan shifts the domain by `delta * (domain_max - domain_min)`. Zoom scales the domain around the cursor's normalised position. No inverse needed — the transform is purely proportional.

**Trade-offs.**

- **A.** Natural API — mirrors `map_f64`. But adds methods to `Scale` for a concern (`brightfield-ui` interaction) that the render crate shouldn't know about. Also, `Band` returning `None` is an awkward API surface for something that should be a type-level prohibition.
- **B.** Keeps `Scale` clean but scatters the inversion logic. Every call site must extract domain/range and call the function — error-prone and repetitive.
- **C.** Simplest and most robust. Normalised deltas are scale-type-agnostic for continuous scales (Linear, Time). Band/Colour scales don't produce continuous ranges, so the interaction layer can simply skip them (axis lock). No new method on `Scale`; no external function needed. The normalised delta is also directly usable for the re-query `WHERE` clause — `new_domain_min = old_min + delta * extent`.

**Recommendation: C, with a convenience `inverse_f64` on `Scale` for point queries (e.g., "what data value is under the cursor?").**

The primary gesture-to-domain path uses normalised deltas — cheap, scale-type-agnostic, and composable. A single `Scale::inverse_f64(&self, pixel: f64) -> Option<f64>` method (returns `None` for Band/Colour) is added for point-query needs like tooltip positioning, but it is not on the critical path for pan/zoom. The interaction layer in `brightfield-ui` computes normalised deltas from pixel events and applies them to `ViewExtent` (D1).

Evidence: `Scale::Linear` stores `domain_min, domain_max, range_start, range_end` (`scale.rs:19-24`), so the normalised transform is `delta_data = (px_delta / (range_end - range_start)) * (domain_max - domain_min)`. `Scale::Time` has the same shape with `i64` timestamps. Both are trivial to normalise. `Band` categories are discrete (`scale.rs:26-30`) — pan/zoom on categorical axes is nonsensical, which aligns with scenario 4's axis-lock requirement.

---

## D3 — Axis-lock resolution: how does the system determine which axes are navigable?

**Context.** Scenario 4: "a spec where only one axis is declared as navigable — only the navigable axis moves; the locked axis stays fixed." The AST already has `InteractorKind::{Pan, PanX, PanY, PanZoom, PanZoomX, PanZoomY}` (`vocab.rs:208-213`) — the suffix encodes the navigable axes (no suffix = both, `X` = x-only, `Y` = y-only). The `Interactor` struct carries an untyped options bag. The question is whether axis-lock is resolved at parse time (from the `InteractorKind` variant) or at interaction time (from the gesture context).

**Options.**

- **A. Derive axis-lock from `InteractorKind` at parse time.** When the spec says `interactor: panZoomX`, the parser (or a post-parse analysis step) emits a `NavigationConfig { axes: AxisLock::XOnly, zoom: true, pan: true }` that the UI layer consumes. No runtime interpretation of the kind string.
- **B. Interpret `InteractorKind` at interaction time.** The UI layer matches on the `InteractorKind` variant during gesture handling: `PanX | PanZoomX => apply only to x; PanY | PanZoomY => apply only to y; Pan | PanZoom => apply to both`. No intermediate struct.
- **C. Use the interactor's options bag.** The spec declares `interactor: panZoom` with `options: { axes: "x" }`. This is more flexible but departs from Mosaic's vocabulary, which encodes the axis in the kind name, not options.

**Trade-offs.**

- **A.** Clean separation — the UI layer receives a typed `NavigationConfig` and doesn't need to know about `InteractorKind` variants. Cost: an additional struct and a mapping function. But this mapping is trivial (six `match` arms) and the struct is reusable across interaction types.
- **B.** Fewer types but tighter coupling — the UI layer must import `InteractorKind` from `brightfield-spec` and match on all variants. If new interactor kinds are added, the UI match must be updated.
- **C.** Most flexible, but Mosaic doesn't use this pattern. The entire `PanX`/`PanY`/`PanZoomX`/`PanZoomY` vocabulary exists specifically to encode axis lock in the kind. Adding an options-based override would be inventing new spec semantics without prior art.

**Recommendation: A.**

A `NavigationConfig` struct (lives in `brightfield-ui` or a shared types crate) is derived from `InteractorKind` and captures `{ pan: bool, zoom: bool, x_navigable: bool, y_navigable: bool }`. The mapping is a single `match` on the six pan/zoom variants. The UI interaction handler consults `NavigationConfig` to gate axis deltas — if `!x_navigable`, the x component of any gesture delta is zeroed. Reset (scenario 3) respects the same config — only navigable axes reset.

Evidence: `vocab.rs:208-213` defines exactly six variants with the axis suffix pattern. Mosaic's JS interactor vocabulary (`@uwdata/vgplot`) uses the same naming (`panX`, `panZoomY`, etc.) — the suffix is the canonical axis-lock declaration. No Mosaic spec uses an options-based axis override.

---

## D4 — Zoom-settle detection: when does a zoom gesture "settle" for re-query?

**Context.** Scenario 5: "when the zoom gesture settles, DuckDB re-queries for the visible extent." Zoom gestures produce a continuous stream of events (scroll wheel ticks, pinch gesture updates). Re-querying DuckDB on every event would overwhelm the engine. The system needs a "settle" heuristic — a point at which the gesture is considered complete and a re-query is worthwhile. The existing two-tier model (`interaction.rs:1-6`) already separates immediate overlay from deferred query — brush release fires `session.update_param`. Navigation needs an equivalent deferred trigger.

**Options.**

- **A. Debounce timer.** After the last zoom event, start a timer (e.g., 150ms). If no further zoom event arrives before the timer fires, the zoom has "settled" — fire the re-query. Each new zoom event resets the timer.
- **B. Velocity-based settle.** Track the rate of domain change. When the rate drops below a threshold (e.g., < 1% of domain extent per frame), declare settled. No timer — purely event-driven.
- **C. Explicit gesture end.** For scroll-wheel: settle on a platform "scroll ended" event (macOS `NSEvent.phase == .ended`). For pinch: settle on `gestureEnded`. No artificial timer — use the platform's own gesture lifecycle.

**Trade-offs.**

- **A.** Platform-agnostic and simple. Works for any input device. 150ms is perceptible but below the card's <100ms budget for re-query execution (the 150ms is *wait* time, not *execution* time). Risk: if the user pauses mid-zoom (thinking), the timer fires and a re-query runs for an intermediate extent, wasting a DuckDB round-trip. Mitigated by the shape-cache (card 0003 D5) — if the user resumes zooming, the intermediate result is just evicted.
- **B.** More responsive — fires as soon as the gesture decelerates, not after an arbitrary delay. But velocity computation requires frame-rate-synchronised delta tracking, and the threshold is hardware-dependent (trackpad vs mouse wheel vs touch). Tuning is fragile.
- **C.** Most accurate — the platform knows when the gesture is truly done. But GPUI's event model may not expose scroll-phase or pinch-phase events on all platforms (GPUI targets macOS/Metal primarily, with Linux/Vulkan coming). If the platform event is unavailable, the system has no fallback.

**Recommendation: A (debounce), with C as a refinement when GPUI exposes gesture-phase events.**

A debounce timer is the simplest mechanism that works across all input devices and platforms. The debounce duration should be configurable (default 150ms) so it can be tuned per platform later. When the timer fires: (1) compute the current `ViewExtent` (D1), (2) express it as a `WHERE` clause filter on the navigable axis/axes, (3) call `session.update_param` (or a new `session.update_extent`) to re-query. The re-query uses the existing prepared-statement cache (card 0003 D5) — if the plan shape hasn't changed, the engine rebinds the extent values without re-planning.

Later, when GPUI exposes gesture-phase events (macOS `scrollPhase`, Linux `libinput_event_pointer_scroll_direction`), the debounce timer can be replaced with platform-native settle detection on a per-platform basis. The debounce path remains as fallback for platforms without phase events.

Evidence: the two-tier model in `interaction.rs:1-6` ("Immediate: overlay renders during drag ... Deferred: DuckDB re-query fires on brush release") is the exact pattern. Navigation extends it: immediate = scale-domain update and scene rebuild (pure rendering); deferred = DuckDB re-query on settle. `Session::update_param` (`lib.rs:161-202`) is the deferred dispatch point.

---

## D5 — Re-query mechanism: how does the zoomed extent become a SQL filter?

**Context.** Scenario 5: "DuckDB re-queries for the visible extent, replacing the preview with full-resolution data." After zoom settles (D4), the engine must issue a query that fetches only the data visible in the current `ViewExtent`. This is different from the existing `filterBy` mechanism (which filters by selection/brush) — it's a *navigation* filter, driven by the view extent rather than user-drawn selections. The question is how the zoom extent enters the query pipeline.

**Options.**

- **A. Express zoom extent as a Mosaic param.** The navigation interactor writes its extent to a spec-level param (e.g., `$nav_x_min`, `$nav_x_max`). Marks with `filterBy: $nav` pick it up through the existing selection compilation path (card 0003 D3). Re-query uses the existing `session.update_param`.
- **B. Engine-level `WHERE` injection.** The engine appends a `WHERE <x_col> BETWEEN <min> AND <max>` clause to the emitted SQL, outside the spec's own filter chain. The injection point is in `Session::execute_emitted` or a new `Session::execute_with_extent`.
- **C. QueryPlan IR pass.** A new optimisation pass (`NavigationFilterPass`) inserts a `Filter` node into the `QueryPlan` IR before SQL rendering. The pass is registered in the pass pipeline (card 0003 D2) and activated when a `ViewExtent` is provided. The emitter produces SQL that already contains the navigation filter.

**Trade-offs.**

- **A.** Reuses the entire existing param/selection pipeline. No new engine API. But navigation extent is not semantically a "selection" — it's a view-level concern, not a data-level one. Crossfilter resolution would need to exclude navigation params from brush predicates, adding complexity. Also, every spec would need to declare navigation params explicitly — the card's goal ("without editing the spec") is undermined if the spec must declare `$nav_x_min`.
- **B.** Clean separation — the engine handles navigation filtering as an execution concern, not a spec concern. The emitter is unaware. Cost: SQL string manipulation after emission is fragile. The shape-cache (card 0003 D5) would need to include the extent in the cache key, and the plan hash would no longer match the actual executed SQL.
- **C.** Architecturally sound — the IR pass slots into the pipeline designed for exactly this kind of concern (card 0003 D2: "pass pipeline shape is load-bearing for future pre-aggregation and M4 passes"). The pass adds a `Filter` node, so the emitted SQL naturally includes the `WHERE` clause. The plan hash reflects the navigation filter, keeping the cache consistent. Cost: the pass needs to know the data column that the navigable axis maps to (the x/y encoding channel's column name), which is spec-level information that must flow through to the pass.

**Recommendation: C.**

This is exactly the kind of concern the pass pipeline was built for. A `NavigationFilterPass` receives the `ViewExtent` and the channel-to-column mapping, and inserts `Filter { predicate: And([Expr("col >= min"), Expr("col <= max")]) }` into the `QueryPlan`. The pass is only activated when a `ViewExtent` is present (zoom has occurred) — when the view is at full extent, no pass runs and the query is unchanged. The plan hash naturally reflects the presence/absence of the navigation filter, so the shape-cache correctly distinguishes "full extent" from "zoomed" queries.

The channel-to-column mapping is available from `ChannelMap` (`channel.rs`), which maps `Channel::X` to a column name. The `NavigationFilterPass` receives `(channel: Channel, column: &str, min: f64, max: f64)` and emits the appropriate `Filter` node.

Evidence: card 0003's pass pipeline (`passes.rs`) is explicitly described as "load-bearing for future pre-aggregation and M4 passes" (spec ac-07). Navigation filtering is a structurally identical concern — an IR-level rewrite that adds a `Filter` node. The pipeline currently ships with zero passes registered; this would be the first real pass, validating the architecture. `QueryPlan::Filter` already exists in `ir.rs` with a `Predicate` tree — the pass composes naturally.

---

## D6 — Scene rebuild strategy: what renders during an active pan/zoom gesture?

**Context.** Scenarios 1 and 2 require "the view updates continuously" during drag and zoom. Currently `build_chart_scene` (`scene.rs`) does a full pipeline: infer scales, render grid, render marks, render axes. At 60Hz interaction rate, this must complete in <16ms per frame. For small datasets this is trivial; for large ones (10k+ marks) it may not be. The question is whether the scene is fully rebuilt on every gesture frame or whether a cheaper intermediate representation is used during active gestures.

**Options.**

- **A. Full scene rebuild every frame.** On each gesture event, update `ViewExtent` (D1), call `build_chart_scene` with the new extent, replace the scene in `ChartElement`. Simple, correct, and leverages Vello's fast scene-building.
- **B. Affine-transform the existing scene during gesture; full rebuild on settle.** During active pan/zoom, apply a `kurbo::Affine` translation/scale to the existing scene content (marks move, axes stay). On settle, do a full rebuild with the final extent plus re-query (D4/D5). Fast during gesture (no scene rebuild), correct after settle.
- **C. Hybrid: rebuild marks from cached data with new scale mapping; defer re-query.** Keep the last-fetched data in memory. During gesture, re-map cached data through updated scales (new `ViewExtent`) and rebuild only the mark layer. Axes and grid rebuild from the new domain. No DuckDB query until settle.

**Trade-offs.**

- **A.** Simplest and always correct. Vello scene building is designed for 60Hz rebuild — the library's entire architecture assumes full scene rebuild each frame (like `wgpu` render loops). For 10k marks this is well within budget on modern hardware. Risk: at 100k+ marks, scene building may exceed 16ms. But the card's re-query scenario (scenario 5) implies that at large scale, pre-aggregation reduces mark count anyway.
- **B.** Fastest during gesture — a single affine multiplication per frame. But axes and grid lines are wrong during the gesture (labels show stale positions, grid doesn't align). For analytical charts, wrong axes during a drag are disorienting. The "snap to correct" on settle creates a jarring visual discontinuity.
- **C.** Correct axes and marks during gesture, no DuckDB overhead. Cost: requires keeping the last data batch in memory alongside the scene. The re-mapping step is `O(n_marks)` per frame, which is the same cost as A but without the scene-building overhead (just scale mapping, no Vello path encoding). Slightly more complex than A.

**Recommendation: A for v1, with a path to C if profiling shows scene-building is the bottleneck.**

Vello is designed for per-frame scene rebuilding. The `Scene` struct is a lightweight encoding buffer, not a retained scene graph — rebuilding it is O(n_marks) with small constants. For the mark counts typical in analytical dashboards (hundreds to low thousands after pre-aggregation), full rebuild at 60Hz is well within budget. Starting with A avoids premature optimisation and keeps the interaction path simple: gesture event -> update `ViewExtent` -> call `build_chart_scene` -> present.

If profiling reveals that scene building exceeds 16ms for realistic workloads, option C is a clean upgrade path: cache the last `RecordBatch`, re-map through updated scales, and rebuild only the changed layers. The `ViewExtent` (D1) and `build_chart_scene` API are the same in both cases — only the internal rebuild strategy changes.

Evidence: Vello's README states "designed for interactive 2D graphics" with "efficient re-encoding" as a design goal. The existing `build_chart_scene` (`scene.rs`) is already a stateless function that takes data and returns a scene — calling it every frame is the intended usage pattern. The `ChartElement::set_scene` method (`chart_element.rs:44-46`) exists specifically for scene replacement.

---

## Summary

```
| #  | Decision                           | Recommendation                                                                       |
|----|------------------------------------|--------------------------------------------------------------------------------------|
| D1 | Navigation state model             | Separate `ViewExtent` struct; `Scale` stays immutable; full-extent = `None`          |
| D2 | Gesture-to-domain mapping          | Normalised deltas for pan/zoom; add `inverse_f64` on Scale for point queries         |
| D3 | Axis-lock resolution               | Derive `NavigationConfig` from `InteractorKind` at parse time; six-arm match         |
| D4 | Zoom-settle detection              | Debounce timer (150ms default); gesture-phase events as future refinement            |
| D5 | Re-query mechanism                 | IR pass (`NavigationFilterPass`) inserts `Filter` node into `QueryPlan`              |
| D6 | Scene rebuild strategy             | Full scene rebuild every frame (Vello's design point); upgrade to cached-data path   |
```

Open questions flagged for the review gate:

- **OQ1 (from D1).** Should `ViewExtent` live in `brightfield-ui` or in a shared types crate? It flows from UI to engine — if it's in `brightfield-ui`, the engine would need to depend on the UI crate (wrong direction). A lightweight types crate or placing it in `brightfield-render` (which both UI and engine can depend on) resolves this.
- **OQ2 (from D2).** For `Scale::Time`, the inverse maps a pixel to a microsecond timestamp. Should the API return `f64` (consistent with `map_f64`) or `i64` (matching the `domain_min_us`/`domain_max_us` fields)? `f64` is simpler and avoids truncation issues during continuous gestures.
- **OQ3 (from D4).** Does GPUI currently expose scroll-phase events on macOS? If so, option C (explicit gesture end) could be the v1 path, not a future refinement. Worth checking `gpui::ScrollWheelEvent` for a `phase` field.
- **OQ4 (from D5).** The `NavigationFilterPass` needs the column name for the navigable axis. This comes from the `ChannelMap`, which is currently constructed in the render crate. Should the channel-to-column mapping be lifted into `brightfield-spec` analysis (alongside the `SpecAnalysis` subscriber graph) so the engine can access it without depending on the render crate?
- **OQ5 (from D5).** For categorical (band) axes, navigation filtering via `WHERE col BETWEEN min AND max` is meaningless. The pass should be a no-op for non-continuous axes. Confirm that the interaction layer (D3) already prevents navigation gestures on categorical axes, so the pass never receives a band-axis extent.
