# Interview — Card 0007: Interactive Navigation

Card: `orbit/cards/0007-interactive-navigation.yaml`
Decision pack: `orbit/specs/2026-04-22-interactive-navigation/decisions.md`
Mode: design interview — all six decisions accepted by the author without overrides.

## Card summary

| Field | Value |
|-------|-------|
| Feature | Interactive navigation |
| As a | analyst zooming in on a region of interest |
| I want | to pan and zoom inside a plot |
| So that | I can move between overview and detail without editing the spec |

Scenarios (5):
1. Pan within a plot — drag shifts the visible data range, view updates continuously
2. Zoom within a plot — scale domain adjusts, marks re-render at new extent
3. Zoom resets to full extent — double-click or reset shortcut restores original domain
4. Pan and zoom respect axis lock — only the navigable axis moves when the spec declares a single axis
5. Zoom settles and re-queries data — DuckDB re-queries for the visible extent after the gesture settles

## Context

The codebase has no pan/zoom handling. `InteractionState` in `crates/brightfield-ui/src/interaction.rs` tracks `Idle`, `Brushing`, and `Hovering` — no navigation states. `ChartElement` (`crates/brightfield-ui/src/chart_element.rs`) holds a `Scene` and `InteractionState` with no scale-domain awareness. Scale domains are inferred from data at build time (`infer_scales` in `crates/brightfield-render/src/scale.rs`) and the `Scale` enum's domain fields are immutable after construction. `build_chart_scene` (`crates/brightfield-render/src/scene.rs:31`) returns `(Scene, ScaleSet)` with no external domain override. The vocabulary registry declares six navigation interactor kinds at `crates/brightfield-spec/src/vocab.rs:208-213` — `Pan`, `PanX`, `PanY`, `PanZoom`, `PanZoomX`, `PanZoomY` — all `Unimplemented`. The engine's `Session::update_param` (`crates/brightfield-engine/src/lib.rs:161-202`) is the existing deferred dispatch point but has no debounce or zoom-aware re-query path.

## Approved decisions

### Q1: Where does the current view extent live during navigation?

Pan and zoom modify the visible data range across many frames. The `Scale` enum is immutable after construction — `Scale::Linear` stores `domain_min`, `domain_max`, `range_start`, `range_end` as fixed values (`scale.rs:19-24`). Navigation needs a mutable "current view extent" that overrides the data-inferred domain, survives across frames, and resets cleanly for scenario 3.

**Decision (D1): Separate `ViewExtent` struct alongside `ScaleSet`; `Scale` stays immutable; `None` = full extent.**

A `ViewExtent { x: Option<(f64, f64)>, y: Option<(f64, f64)> }` lives in `ChartElement` (or a new `NavigationState`) next to the `ScaleSet`. At render time, `build_chart_scene` receives an `Option<&ViewExtent>` parameter — `None` means "show full data extent" (current behaviour), `Some` means "override domain to this range". The `Scale` enum stays a value type. The original data-inferred domain is preserved in `ScaleSet` so that reset (scenario 3) is trivial: set `ViewExtent` back to `None`.

This separation is load-bearing for scenario 5: `ViewExtent` is a plain data struct that can flow both to the renderer (domain override) and to the engine (for re-query `WHERE` clause) without coupling `brightfield-render` to `brightfield-ui`. `build_chart_scene` already returns `ScaleSet` alongside the `Scene` (`scene.rs:31`), so the caller can store the original `ScaleSet` and derive `ViewExtent` deltas from it. The engine's `update_param` (`lib.rs:161-202`) already takes `SpecValue` parameters — a `ViewExtent` can be expressed as a param update without changing the engine API.

**Open question (OQ1):** Should `ViewExtent` live in `brightfield-ui` or in a shared types crate? It flows from UI to engine — if it's in `brightfield-ui`, the engine would need to depend on the UI crate (wrong direction). Placing it in `brightfield-render` (which both UI and engine can depend on) resolves this.

### Q2: How do pixel-space gestures translate to data-space pan and zoom?

A 50px drag or a scroll-wheel tick is a pixel-space event. The current `Scale` has `map_f64` (data-to-pixel, `scale.rs:48-77`) but no inverse (pixel-to-data). `Scale::Linear` stores `domain_min`, `domain_max`, `range_start`, `range_end` — the inverse is a linear interpolation. `Scale::Band` maps discrete categories (`scale.rs:26-30`) — continuous pan/zoom is undefined on categorical axes.

**Decision (D2): Normalised deltas for pan/zoom; `Scale::inverse_f64` for point queries.**

The primary gesture-to-domain path uses normalised deltas. Gestures produce a normalised delta `(px_delta / range_width)`. Pan shifts the domain by `delta * (domain_max - domain_min)`. Zoom scales the domain around the cursor's normalised position. No inverse method needed on the critical path — the transform is purely proportional and scale-type-agnostic for continuous scales (Linear, Time). Band/Colour scales don't produce continuous ranges, so the interaction layer skips them (aligning with scenario 4's axis-lock requirement).

A convenience `Scale::inverse_f64(&self, pixel: f64) -> Option<f64>` method is added for point-query needs like tooltip positioning. It returns `None` for Band/Colour. For `Linear`: `data = domain_min + (pixel - range_start) / (range_end - range_start) * (domain_max - domain_min)`. For `Time`: same shape with `i64` timestamps cast to `f64`.

**Open question (OQ2):** For `Scale::Time`, the inverse maps a pixel to a microsecond timestamp. Should the API return `f64` (consistent with `map_f64`) or `i64` (matching `domain_min_us`/`domain_max_us`)? `f64` is simpler and avoids truncation issues during continuous gestures.

### Q3: How does the system determine which axes are navigable?

Scenario 4: "only the navigable axis moves; the locked axis stays fixed." The AST has `InteractorKind::{Pan, PanX, PanY, PanZoom, PanZoomX, PanZoomY}` (`vocab.rs:208-213`) where the suffix encodes navigable axes. The `Interactor` struct (`ast.rs:307-315`) carries `kind: InteractorKind` plus an untyped options bag.

**Decision (D3): Derive `NavigationConfig` from `InteractorKind` at parse time; six-arm match.**

A `NavigationConfig` struct captures `{ pan: bool, zoom: bool, x_navigable: bool, y_navigable: bool }`. The mapping is a single `match` on the six pan/zoom variants:

- `Pan` -> pan=true, zoom=false, x=true, y=true
- `PanX` -> pan=true, zoom=false, x=true, y=false
- `PanY` -> pan=true, zoom=false, x=false, y=true
- `PanZoom` -> pan=true, zoom=true, x=true, y=true
- `PanZoomX` -> pan=true, zoom=true, x=true, y=false
- `PanZoomY` -> pan=true, zoom=true, x=false, y=true

The UI interaction handler consults `NavigationConfig` to gate axis deltas — if `!x_navigable`, the x component of any gesture delta is zeroed. Reset (scenario 3) respects the same config — only navigable axes reset. The struct lives in `brightfield-ui` and is computed from `InteractorKind` at the boundary between spec and UI, keeping the UI layer decoupled from the full `InteractorKind` enum.

### Q4: When does a zoom gesture "settle" to trigger re-query?

Scenario 5: "when the zoom gesture settles, DuckDB re-queries for the visible extent." Zoom gestures produce a continuous stream of events. Re-querying DuckDB on every event would overwhelm the engine. The existing two-tier model (`interaction.rs:1-6`) separates immediate overlay rendering from deferred query — brush release fires `session.update_param`. Navigation needs an equivalent deferred trigger.

**Decision (D4): Debounce timer (150ms default); gesture-phase events as future refinement.**

After the last zoom event, a timer starts (150ms default, configurable). If no further zoom event arrives before the timer fires, the zoom has "settled" and the re-query dispatches. Each new zoom event resets the timer. The 150ms wait is below the card's <100ms budget for re-query execution (the 150ms is wait time, not execution time).

This extends the existing two-tier model: immediate = scale-domain update + scene rebuild (pure rendering, no I/O); deferred = DuckDB re-query on settle via `Session::update_param` (`lib.rs:161-202`). The shape-cache from card 0003 D5 mitigates premature re-queries: if the user pauses mid-zoom then resumes, the intermediate result is just evicted.

Later, when GPUI exposes gesture-phase events (macOS `scrollPhase`, Linux `libinput` direction events), the debounce can be replaced with platform-native settle detection. The debounce path remains as fallback.

**Open question (OQ3):** Does GPUI currently expose scroll-phase events on macOS? If so, platform-native settle could be the v1 path. Worth checking `gpui::ScrollWheelEvent` for a `phase` field.

### Q5: How does the zoomed extent become a SQL filter for re-query?

Scenario 5: "DuckDB re-queries for the visible extent, replacing the preview with full-resolution data." After zoom settles, the engine must issue a query that fetches only data visible in the current `ViewExtent`. This is distinct from the existing `filterBy` mechanism (which filters by selection/brush) — it is a navigation filter driven by the view extent.

**Decision (D5): IR pass (`NavigationFilterPass`) inserts `Filter` node into `QueryPlan`.**

A new `NavigationFilterPass` implements the existing `trait Pass { fn apply(&self, plan: QueryPlan) -> QueryPlan; }` from `brightfield-sql/src/passes.rs` (card 0003 D2). The pass receives the `ViewExtent` and the channel-to-column mapping (from `ChannelMap` in `crates/brightfield-render/src/channel.rs`), then inserts `Filter { predicate: And([Expr("col >= min"), Expr("col <= max")]) }` into the `QueryPlan`. The pass is only activated when a `ViewExtent` is present — when the view is at full extent, no pass runs and the query is unchanged.

The plan hash naturally reflects the presence/absence of the navigation filter, so the shape-cache (card 0003 D5) correctly distinguishes "full extent" from "zoomed" queries. `QueryPlan::Filter` already exists in `brightfield-sql/src/ir.rs` with a `Predicate` tree — the pass composes naturally. The channel-to-column mapping is available from `ChannelMap`, which maps `Channel::X` to a column name string (`channel.rs:91-93`).

This is the first real pass registered in the pipeline, validating the pass architecture that card 0003 D2 explicitly designed as "load-bearing for future pre-aggregation and M4 passes."

**Open question (OQ4):** The `NavigationFilterPass` needs the column name for the navigable axis. This comes from `ChannelMap`, which is constructed in the render crate. Should the channel-to-column mapping be lifted into `brightfield-spec` analysis (alongside `SpecAnalysis`) so the engine can access it without depending on the render crate?

**Open question (OQ5):** For categorical (band) axes, navigation filtering via `WHERE col BETWEEN min AND max` is meaningless. The pass should be a no-op for non-continuous axes. Confirm that the interaction layer (D3) already prevents navigation gestures on categorical axes, so the pass never receives a band-axis extent.

### Q6: What renders during an active pan/zoom gesture?

Scenarios 1 and 2 require "the view updates continuously" during drag and zoom. `build_chart_scene` (`scene.rs`) runs the full pipeline: infer scales, render grid, render marks, render axes. At 60Hz, this must complete in <16ms per frame.

**Decision (D6): Full scene rebuild every frame (Vello's design point).**

On each gesture event: update `ViewExtent` (D1), call `build_chart_scene` with the new extent, replace the scene in `ChartElement` via `set_scene` (`chart_element.rs:44-46`). Vello's `Scene` struct is a lightweight encoding buffer, not a retained scene graph — rebuilding it is O(n_marks) with small constants. For analytical dashboards (hundreds to low thousands of marks after pre-aggregation), full rebuild at 60Hz is well within budget.

`build_chart_scene` is already a stateless function that takes `ChartData` and returns `(Scene, ScaleSet)` (`scene.rs:31`). Calling it every frame is the intended usage pattern. No caching, no intermediate representation, no affine-transform workaround.

If profiling reveals that scene building exceeds 16ms for realistic workloads, a clean upgrade path exists: cache the last `RecordBatch`, re-map through updated scales, and rebuild only the mark layer. The `ViewExtent` and `build_chart_scene` API are the same in both cases — only the internal rebuild strategy changes.

## Open questions carried into spec

```
| ID  | Question | Disposition for spec |
|-----|----------|----------------------|
| OQ1 | Should `ViewExtent` live in `brightfield-ui` or `brightfield-render`? | Place in `brightfield-render` — both UI and engine already depend on it. Spec should declare the crate. |
| OQ2 | `Scale::inverse_f64` for Time: return `f64` or `i64`? | Return `f64` for API consistency with `map_f64`. Spec should note this. |
| OQ3 | Does GPUI expose scroll-phase events on macOS? | Investigate before spec. If available, gesture-phase settle could be v1 instead of debounce. |
| OQ4 | Should `ChannelMap` be lifted into `brightfield-spec` analysis? | Defer — pass receives channel-to-column mapping as an argument. The mapping source is an implementation detail. |
| OQ5 | Does the interaction layer gate navigation on categorical axes? | Yes — D3's `NavigationConfig` derives navigability from `InteractorKind`, and D2's normalised-delta path skips Band/Colour scales. The pass never receives a band-axis extent. Spec AC should assert this. |
```

## Implementation surface

### Modified: `crates/brightfield-render/src/scale.rs`

- Add `Scale::inverse_f64(&self, pixel: f64) -> Option<f64>` method. Returns `None` for `Band` and `Colour`. For `Linear`: `domain_min + (pixel - range_start) / (range_end - range_start) * (domain_max - domain_min)`. For `Time`: same shape cast through `f64`.
- Add `ViewExtent` struct: `pub struct ViewExtent { pub x: Option<(f64, f64)>, pub y: Option<(f64, f64)> }`. Lives alongside `ScaleSet` in this module.

### Modified: `crates/brightfield-render/src/scene.rs`

- Extend `build_chart_scene` signature to accept `Option<&ViewExtent>`. When `Some`, override the inferred scale domains before computing ticks, rendering grid, marks, and axes. When `None`, current behaviour is preserved.
- The `ChartData` struct gains an optional `view_extent: Option<ViewExtent>` field.

### Modified: `crates/brightfield-ui/src/interaction.rs`

- Add `Panning` and `Zooming` variants to `InteractionState` (or a separate `NavigationState` enum).
- Add `NavigationConfig { pan: bool, zoom: bool, x_navigable: bool, y_navigable: bool }` struct.
- Add `NavigationConfig::from_interactor_kind(kind: InteractorKind) -> Option<NavigationConfig>` — the six-arm match (D3). Returns `None` for non-navigation interactor kinds.
- Gesture-to-domain translation: normalised delta computation from pixel drag/scroll events applied to `ViewExtent`.

### Modified: `crates/brightfield-ui/src/chart_element.rs`

- Add `view_extent: Option<ViewExtent>` field to `ChartElement`.
- Add `original_scales: Option<ScaleSet>` field to preserve data-inferred scales for reset.
- Add pan/zoom event handlers that update `view_extent` and trigger scene rebuild via `build_chart_scene` + `set_scene`.
- Add reset handler (double-click) that sets `view_extent` to `None` and rebuilds scene from original scales.
- Add debounce timer for zoom-settle detection (D4). On fire: dispatch re-query to engine.

### New: `crates/brightfield-sql/src/navigation_filter_pass.rs`

- `pub struct NavigationFilterPass` implementing `trait Pass`.
- `fn apply(&self, plan: QueryPlan) -> QueryPlan` — inserts `Filter { predicate: And([Expr("col >= min"), Expr("col <= max")]) }` for each navigable axis with a non-`None` extent.
- Constructor: `NavigationFilterPass::new(view_extent: &ViewExtent, channel_columns: &[(Channel, &str)])`.

### Modified: `crates/brightfield-engine/src/lib.rs`

- Add `Session::update_extent` (or extend `update_param`) to accept a `ViewExtent`, activate the `NavigationFilterPass` in the pass pipeline, and re-execute subscribing marks.
- Alternatively, express the navigation extent as `SpecValue` parameters (`$nav_x_min`, `$nav_x_max`) and flow through the existing `update_param` path — the pass reads these params from `ParamValues`.

### Modified: `crates/brightfield-spec/src/vocab.rs`

- Flip `InteractorKind::{Pan, PanX, PanY, PanZoom, PanZoomX, PanZoomY}` status from `Unimplemented` to `Implemented` once the feature ships.

## Key types

```
| Type | Crate | Purpose |
|------|-------|---------|
| `ViewExtent` | `brightfield-render` | Mutable view domain override — `Option<(f64, f64)>` per axis |
| `NavigationConfig` | `brightfield-ui` | Typed axis-lock + pan/zoom capability derived from `InteractorKind` |
| `NavigationFilterPass` | `brightfield-sql` | IR pass inserting `Filter` node for zoomed extent |
| `Scale::inverse_f64` | `brightfield-render` | Pixel-to-data inverse mapping for point queries |
```

## Cross-card touchpoints

- **Card 0003 (shipped).** `NavigationFilterPass` is the first real pass registered in the pass pipeline designed by D2. Uses `QueryPlan::Filter` and `Predicate` from `brightfield-sql/src/ir.rs`. Shape-cache keying (D5) naturally includes the navigation filter.
- **Card 0006 (shipped).** Cross-filter selection compilation must exclude navigation filters from brush predicates. Navigation is a view-level concern, not a data-level selection.
- **Card 0001 (shipped).** `InteractorKind` variants, `Interactor` struct, and `ImplStatus` consumed as-is. No modifications to the AST.

## Integration sequence

1. Add `ViewExtent` to `brightfield-render/src/scale.rs` and `Scale::inverse_f64`.
2. Extend `build_chart_scene` in `scene.rs` to accept `Option<&ViewExtent>`.
3. Add `NavigationConfig` and navigation state to `brightfield-ui/src/interaction.rs`.
4. Wire gesture handlers in `chart_element.rs` — pan, zoom, reset, debounce timer.
5. Add `NavigationFilterPass` to `brightfield-sql/src/navigation_filter_pass.rs`.
6. Connect zoom-settle to engine re-query via `Session::update_param` or new `update_extent`.
7. Flip `InteractorKind` status to `Implemented` for the six navigation variants.
