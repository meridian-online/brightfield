# Multi-view dashboards — headless foundation (increments 1–4)

Card 0009. First push toward rendering a spec's `hconcat`/`vconcat` layout of
multiple plots instead of collapsing everything into one. A design workflow
mapped the subsystems and chose the hosting approach; this memo records the
headless foundation that's now shipped.

## Design decision (from the workflow)

**Host N independent `ChartElement`s in a GPUI flex container** (`div().flex_row()/
.flex_col()` mirroring hconcat/vconcat), one `ChartState` per plot — *not* one
composite Vello scene. Rationale: reuses the per-chart raster cache, crispness,
and per-element interaction routing just shipped; and it's the correct substrate
for cross-filter (card 0006), which is *data-level* selection dispatch keyed by
`ComponentPath`, not shared pixel scales. (The composite-scene alternative was
rejected — it collapses interaction to one hitbox and re-rasterises everything on
every hover.)

Key discovery: a full layout pass (`compute_layout` → positioned `LayoutNode`
rects) already existed and was tested, but was dead code and carried no
`ComponentPath` identity. So the work is mostly *connecting* existing pieces.

## Shipped (headless, verified by tests + a PNG)

1. **Per-plot mark grouping** — `brightfield_sql::collect_plot_groups(spec) ->
   Vec<PlotGroup{ plot_path, mark_indices }>` (emit.rs). Correctness note: the
   path scheme labels each *item* of a plot `plot[i]`, so `analysis::parent_plot`
   does NOT group marks to their plot (it keeps the item-index segment). Grouping
   uses a dedicated tree walk keyed on the plot node's own path. `mark_indices`
   index the flat `collect_marks`/`execute_all` order.
2. **Layout identity join** — `brightfield_spec::layout::placed_plots(spec,
   viewport) -> Vec<PlacedPlot{ path, rect }>` (layout.rs), walking the existing
   layout tree with the same path convention as `collect_plot_groups`, so a
   positioned rect joins to its data. (Left the public `LayoutNode` enum and its
   pinned geometry tests untouched.)
3. **Render composer** — `brightfield_render::scene::build_dashboard_scene(w, h,
   &[DashboardPlot])` builds each plot's scene with its OWN axes/scales (domains
   unioned only within a plot) and composites them at their origins via
   `Scene::append(translate)`, over a white dashboard background.
4. **App assembly** — `run_pipeline` now groups marks per plot, builds per-plot
   `ChartData` sized from the layout rects, and composites; returns `(Scene,
   width, height)` (the dashboard bounding box) instead of `(Scene, ScaleSet,
   usize)`. `main` sizes the PNG/window from those dims.

Verified: `cargo test --workspace` green (new tests: grouping in 2-plot hconcat +
single plot; `placed_plots` paths/rects; dashboard composition). `examples/
dashboard.yaml` (scatter + bar, inline data) renders headlessly to a 720×300 PNG
with two plots side by side, each with independent axes. Single-plot specs
(scatter/bars) are unchanged.

## Window hosting (increment 5 — done)

The macOS window now hosts **one `ChartElement` per plot**, each absolutely
positioned at its layout rect inside a relative container sized to the dashboard,
with its own `ChartState` (so its own raster cache *and* interaction). `ChartView`
holds `Vec<PlacedChart{x,y,w,h,state}>`; `run_pipeline` returns a `Dashboard` of
per-plot scenes (the PNG path composites them, the window hosts them). The N
states share one `VelloRenderer` (per-`ChartState` caches, so no collision).

Per-plot interaction isolation: mouse-down is hitbox-gated so a brush starts only
in the plot under the cursor; hover is treated as a no-button gesture (a held
button means a drag is in progress, so it never lights a sibling's hover marker).
Hot-reload matches plots by stable `ComponentPath` + geometry — a data/visual
edit swaps each plot's scene in place; any structural change (count, which plots
render, or any size/position) prints "restart to apply".

The GPUI hosting was adversarially reviewed against the gpui source (verdict:
renders correctly with independent per-plot brush; positioning, hitbox-gated
press, per-state rendering and repaint are sound). The two flagged defects —
sibling hover bleed while dragging a brush across plots, and a count-only reload
guard that could corrupt the view on a size change or plot reshuffle — are fixed.
**Needs a window eyeball** to confirm side-by-side plots + independent hover/brush.

## Next (increment 6)

- Wire `Input`(slider)/`Legend`/spacer nodes into the host (plots-only today).
- Structural hot-reload that rebuilds/repositions N states in place (hand the
  watcher the `ChartView` entity) instead of "restart to apply"; window resize /
  overflow policy.

## Deferred

- `plotDefaults` sizing (splom/mark-types use 150×150/160×100; today non-default
  plots need explicit width/height).
- Per-plot margins from spec `marginX` attrs (currently default inset).
- External legend `for:` name→rect resolution; overflow/scroll for grids larger
  than the window.
