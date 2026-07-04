# Legends & spacers hosting — follow-ups (multi-view inc 6)

This increment hosts **spacers** (already functional — see below) and renders
**standalone colour legends** in the headless composite. A standalone
`legend: color for: <name>` now **relocates** the plot's legend — the plot's own
inline (top-right) legend is suppressed so the scale isn't drawn twice (item 7).
What's deferred:

## 1. Window hosting of standalone legends (eyeball-gated)

Standalone `legend:` nodes render into the **headless/PNG composite**
(`render_colour_legend_at` at the node's layout rect). They are NOT yet hosted as
**window elements** — the window path destructures `Dashboard.legends` and ignores
it (main.rs). Hosting needs a static GPUI element that rasterises the legend scene
at its rect (like `ChartElement` does for a plot, but non-interactive — no hover/
brush). Spacers, by contrast, are fully window-functional already: they carry no
pixels; `placed_plots` offsets the neighbouring plots and the window reads those
same rects, so a gap "just works" in both paths (verified by
`hspace_offsets_subsequent_plot` + `examples/layout-spacer.yaml`).

→ Follow-up: a `LegendElement` (static scene panel) hosted in `ChartView`, plus a
`Dashboard.legends` → `PlacedLegend`(ui) hand-off. Small, but pure untestable GPUI
plumbing, so held out of this headless increment.

## 2. Legend interaction — click a swatch to filter (`as: $sel`)

A `legend:` node can carry `as: $selection` (Mosaic's clickable legend → a
categorical point selection on the fill column). Not wired: the legend is static.
This is a **categorical string point-selection**, which the point-click gesture
memo (`2026-07-03-point-selection-followups.md`, item 1) already flags as needing
type-aware predicates (`col = 'value'` with escaping) + categorical nearest. Do it
once, share it between legend-click and categorical `toggleX`.

## 3. Opacity / symbol legend channels

Only `legend: color` is implemented (`LegendChannel::Color` → Implemented).
`opacity` and `symbol` stay `Unimplemented` — no renderer, and no opacity/size
scale is inferred yet. A node with those channels is skipped with a diagnostic.

## 4. `for:` resolution semantics — confirm the model with product testing

A standalone legend resolves its colour scale from the plot its `for:` names
(matched against the plot's `name` attribute). Current rules (all warn + skip on
failure — see `resolve_legends`/`legend_for`/`colour_scale_of` in main.rs):

- Explicit `for: <name>` → that named plot's colour scale, or **skip + warn** if
  no colour-encoded plot has that name (never silently borrow another plot's).
- Absent `for:` → the dashboard's **sole** colour-encoded plot, if exactly one;
  else skip + warn.
- Param-valued `for: $x` → **skip + warn** (unsupported; a legend names a plot by
  a literal string). Duplicate colour-plot `name`s → **warn** (`for:` is then
  ambiguous; last in tree order wins).
- Fill vs stroke colour: `colour_scale_of` filters *each* channel to a Colour
  scale before falling back, so a numeric fill doesn't mask a categorical stroke.

Still open UX questions for product testing: should `for:` be required? should an
unresolved `for:` be a hard parse error rather than a skip? These are reasonable
defaults from the AST's documented `for: <plot-name>`, not settled decisions.

## 5. Legend panel size vs the layout slot — clipping FIXED, overlap residual

`layout_component` reserves a fixed 120×24 for a legend node (the resolved scale
is unknown at layout time). `resolve_legends` now **re-sizes each `LegendPlacement`
to the panel `colour_legend_size` will actually draw**, so the composite
bounding-box fold reserves enough room and **the legend is never clipped
off-canvas** (was a silent data-loss bug — categories vanished on any vconcat
legend or long labels). Verified by a 4-category vconcat rendering full-height.

**Residual:** a legend *followed by another element in the same concat* can still
visually overlap it — single-pass layout advanced the following sibling by only
the nominal 24/120, and the app can't reflow siblings post-execution. The common
placements (legend trailing its concat, or beside a taller plot) are clean.
Proper fix: thread the resolved legend size back into a layout re-pass, or size
the layout slot from a generous default.

## 6. A dropped legend still reserves its layout slot

`layout_component` reserves the 120×24 slot for *every* `legend:` node, but
`resolve_legends` drops legends that don't resolve (non-`color` channel, unmatched
`for:`, no colour scale). Because `placed_plots` bakes the reserved slot into the
neighbouring plots' offsets, a *dropped* legend that sits *between* two plots
leaves a ~120px blank gap with no legend (a warning fires about the skip, but not
the gap). Cosmetic, gated behind a resolution-failure warning. Same root as #5 —
layout reserves before the app knows whether the legend resolves.

## 7. Inline-legend suppression — residual cases

An explicit `legend: color for: <name>` now suppresses that plot's inline legend
(main.rs computes `legend_suppressed` from the plot-name→legend `for:` map and
passes `draw_inline_legend=false` to `build_multi_mark_scene`). Two residuals:

- **A bare `legend:` (no `for:`) still ADDS** rather than relocating — it resolves
  to the sole colour-encoded plot only *after* the scenes are built (needs the
  scales), so suppressing it would mean a scene rebuild. Deliberately left as an
  addition: an explicit `for:` reads as "move it here," a bare legend as "also show
  one." Revisit if product testing wants bare legends to relocate too.
- **A cross-filter re-render re-draws the inline legend** — `crossfilter.rs` passes
  `draw_inline_legend=true` because standalone-legend suppression is resolved at the
  app layer, not in the UI crate. A plot that is BOTH cross-filtered and has a
  relocated legend would regain its inline legend after a brush. No current example
  hits this; the clean fix is to thread the suppression flag into the crossfilter
  scene rebuild.
