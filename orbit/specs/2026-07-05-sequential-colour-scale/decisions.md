# Decision Pack: Sequential Colour Scale (`Scale::Sequential`)

The highest-leverage render gap. The `raster` mark (card 0008) shipped as a binned
2D count heatmap that encodes count as **alpha on one hue** (steelblue) because
`Scale` has no continuous-colour form — `crates/brightfield-render/src/scale.rs:17`
only has `Linear | Band | Time | Colour`, and `Colour` is *categorical*
(`{categories, palette}`, `scale.rs:40`). The raster follow-ups memo
(`orbit/cards/memos/2026-07-03-raster-mark-followups.md`, follow-up 1) names this
as "the single biggest gap": a true viridis-style `count → gradient` needs a new
`Scale::Sequential` + interpolation + inference + colour-legend wiring, and it
blocks true heatmaps, the `heatmap`/`cell` marks, and continuous colour legends.

A recon of four subsystems established the shape:

- **Scales** — `Scale` enum + `infer_column_scale` (`scale.rs:559`) build categorical
  `Colour` only from a `Utf8` column bound to `Channel::Fill`/`Stroke` (`scale.rs:673`);
  a numeric column bound to Fill falls through to `Linear`. `map_colour` (`scale.rs:159`)
  is a discrete category→index lookup. Raster never binds Fill — its `render`
  (`mark.rs:1381`) reads the `__bf_count` column directly and mixes count into
  `DEFAULT_COLOUR`'s alpha with a `RASTER_MIN_ALPHA = 0.25` floor (`mark.rs:1317`,
  `mark.rs:1440`) so every occupied cell stays visible.
- **Legends** — `legend.rs` renders categorical *swatches* only: `colour_legend_size`
  (`legend.rs:30`) and `render_colour_legend_at` (`legend.rs:67`) both `match … Colour`
  and early-return for anything else. The inline legend keys off `Channel::Fill`
  (`scene.rs:187`, `scene.rs:303`); the standalone `legend:` path resolves a plot's
  colour scale via `colour_scale_of` (`main.rs:106`, an `is_colour` filter over
  Fill-then-Stroke) → `resolve_legends` → `render_colour_legend_at` at the legend's
  layout rect.
- **Vocabulary** — `colorScheme`, `colorDomain`, `colorRange` are already on the
  recognised-attribute allowlist (`parse.rs:155`) and parse into `PlotNode.attributes`
  (an open bag, `ast.rs:259`); nothing consumes them. There is no vocab_enum for
  colour schemes (the `vocab.rs` registries name marks/interactors/inputs/legend
  channels, not scale attributes).
- **Mosaic alignment** — Mosaic/Observable Plot express continuous colour with the
  flat attributes `colorScheme` (named ramp), `colorScale` (`linear` default, also
  `pow`/`sqrt`/`log`/`symlog`), `colorDomain`, `colorRange`, `colorReverse`. Plot's
  default quantitative **scheme** is `turbo` and default **type** is `linear`. Scheme
  names are lowercase: `viridis`, `blues`, `turbo`, `magma`, … We keep exactly these
  names so a spec stays portable.

Seven decisions follow.

## Decision 1: New `Scale::Sequential { domain_min, domain_max, stops }`.

Mirror `Colour`'s shape — a domain plus a colour list — but continuous. `stops:
Vec<[f32; 4]>` are evenly-spaced RGBA control points; the value→colour map
normalises and piecewise-lerps between adjacent stops.

**Chosen:** add
```
Sequential { domain_min: f64, domain_max: f64, stops: Vec<[f32; 4]> }
```
plus a method `map_continuous(&self, value: f64) -> [f32; 4]`:
`t = clamp((value - domain_min) / (domain_max - domain_min), 0, 1)`; locate the
bracketing pair `stops[i], stops[i+1]` for `t*(n-1)` and lerp per channel. A
degenerate domain (`domain_max - domain_min < EPSILON`) returns the **top** stop
(matches how `map_f64` collapses a zero-span linear domain to a single point). This
is the one new pure function and the core unit-test surface.

Adding the variant deliberately **breaks every exhaustive `match Scale`** so the
compiler enumerates each site that must decide: `union_scales` (`scale.rs:455`, union
two Sequentials by min/max domain), `compute_ticks` in `axis.rs:39` (a colour ramp
has no positional ticks → `Vec::new()`, like `Colour`), and `range_start`/`range_end`
(`scale.rs:191`,`:201` → `0.0`, like `Colour`). The `_ =>`/`Band|Colour` arms in
`map_f64`/`inverse_f64`/`map_category`/`band_width`/`map_colour` extend cleanly
(Sequential behaves like `Colour`: no positional/inverse/category mapping). Give
`domain_min`/`domain_max` (`scale.rs:173`,`:183`) a `Sequential` arm too — the legend
ticks read the ramp extent through them.

## Decision 2: Built-in schemes `viridis | blues | turbo`, as control-point stops.

A `SequentialScheme` enum with `wire_name`/`from_wire` (lowercase, Mosaic-aligned)
and `stops() -> Vec<[f32; 4]>` returning 7–9 hand-transcribed RGBA control points
per scheme (enough to read as the intended ramp; not a full 256-entry LUT — that is
a later refinement). Three schemes cover the needed space: **viridis**
(perceptually-uniform, colourblind-safe, dark→bright), **blues** (single-hue
sequential, light→dark — the classic count map), **turbo** (Mosaic's declared
quantitative default, included for spec fidelity). An unknown/unsupported
`colorScheme` value logs a warning and falls back to the default.

**Chosen:** `SequentialScheme::{Viridis, Blues, Turbo}`, `stops()` per scheme, living
in `scale.rs` beside `CATEGORICAL_PALETTE` (`scale.rs:249`).

## Decision 3: Default scheme = **viridis** (deliberate deviation from Mosaic's turbo).

Mosaic/Plot default the quantitative scheme to `turbo`. We default to **viridis**
instead: it is perceptually uniform and colourblind-safe, whereas turbo is a rainbow
map with known perceptual artefacts at the extremes, and viridis is the de-facto
modern default (matplotlib, ggplot). `turbo` stays available by name, so a spec that
declares `colorScheme: turbo` still renders turbo — portability is preserved; only
the *unspecified* default differs. **Flagged for ratification (open question 1).**

## Decision 4: The Sequential scale lives under `Channel::Fill`, produced by the raster's `augment_scales`.

The inline legend (`scene.rs:187`,`:303`), `colour_scale_of` (`main.rs:106`) and
`resolve_legends` all already key off `Channel::Fill`. Putting the Sequential scale
there means the entire legend-resolution and standalone-`legend:` path picks it up
for free once the render/size functions branch on the variant — no parallel channel.
The count column (`__bf_count`) is not in the raster's channel map, so generic
`infer_column_scale` can't build it; the mark contributes it the same way regression
contributes its x/y extents — via `MarkRenderer::augment_scales` (`mark.rs:1456`,
trait `mark.rs:86`), which already runs after inference and has the batch in hand.

**Chosen:** `RasterRenderer::augment_scales` reads `column_as_f64(batch,
DENSITY_COUNT_COL)`, computes the count domain (Decision 5), and inserts
`Scale::Sequential { domain, stops: self.scheme.stops() }` under `Channel::Fill`
(keeping its existing half-bin x/y widening). `render` reads the Fill scale back and
maps `count → map_continuous`. If the Fill scale is somehow absent it falls back to
the old steelblue-alpha path (graceful, never panics).

## Decision 5: Zero-anchored domain `[0, max_count]` + a ramp-position floor replacing `RASTER_MIN_ALPHA`.

With a real ramp, cells render at **full alpha in the ramp colour** — the alpha-fade
floor is no longer the visibility mechanism. But the floor idea still matters: a
light-anchored scheme (blues starts near-white `#f7fbff`) would render the sparsest
occupied cells nearly invisible against the white plot background. Anchoring the
domain at zero (`[0, max_count]`, not the data extent `[min, max]`) is the honest
choice for counts and means an occupied cell (count ≥ 1) maps to `t = count/max > 0`
rather than `t = 0`; combined with a small **ramp-position floor** `RASTER_MIN_T`
(sample at `t.max(RASTER_MIN_T)` for occupied cells) every occupied cell gets a
visibly-tinted colour under *both* dark- and light-anchored schemes. This replaces
`RASTER_MIN_ALPHA` (`mark.rs:1317`) with `RASTER_MIN_T` — the same guarantee,
re-expressed as a floor on ramp position instead of alpha.

**Chosen:** raster domain `[0, max_count]`; occupied cells sampled at
`map_continuous`-of-`t.max(RASTER_MIN_T)`; `RASTER_MIN_ALPHA` removed; the empty-grid
early return (`mark.rs:1417`, `max_count <= 0`) is unchanged. Data-extent anchoring
`[min, max]` is the deferred alternative if a future mark needs it.

## Decision 6: `colorScheme` consumed on the headless authoring path; a scheme-configured `RasterRenderer { scheme }`.

`RasterRenderer` becomes a config struct `RasterRenderer { scheme: SequentialScheme }`
— the established pattern (`AreaRenderer { axis }`, `RectRenderer { kind }`,
`mark.rs:1808`,`:1812`). `default_renderers()` (`mark.rs:1797`) registers it with the
default (viridis). The **app assembly** (`main.rs:384`–`448`) builds the raster
renderer per-mark from `mark.attributes.get("colorScheme")` instead of borrowing the
registry default, so a plot that declares `colorScheme: blues` renders blues. This is
the primary authoring/headless (`BRIGHTFIELD_DUMP_PNG`) path.

The **live cross-filter** path (`crossfilter.rs:285`–`304`, `MarkInput` carries only
`kind`) inherits the viridis default in v1; threading `colorScheme` into `MarkInput`
so a live dashboard honours it is a small, recorded follow-up (it does not change the
scale/legend infra, only where the scheme value is read). **Scope flagged (open
question 2).** `colorScale`/`colorDomain`/`colorRange`/`colorReverse` (log/sqrt
transforms, custom domains/ranges, reversal) are **deferred** — only `colorScheme`
(implicitly `linear`) is consumed. They parse harmlessly today (open attribute bag),
so no parser change is required to defer them.

## Decision 7: Continuous legend = a gradient bar variant, dispatched by scale kind.

Add `sequential_legend_size(&Scale) -> Option<(f64, f64)>` and
`render_sequential_legend_at(scene, x, y, &Scale)` alongside the categorical pair in
`legend.rs`. The bar is a vertical ramp — drawn as ~48 stacked sampled quads (no
gradient-brush dependency; `map_continuous` at evenly-spaced `t`) — with **min / mid /
max** numeric tick labels beside it (read from `domain_min`/`domain_max`). Make
`colour_legend_size` and the inline/standalone render calls **dispatch on the variant**:
`Colour` → swatches (unchanged), `Sequential` → gradient bar. Extend
`colour_scale_of`'s `is_colour` predicate (`main.rs:110`) to also accept `Sequential`,
so a standalone `legend: color for: <raster-plot>` resolves and renders a gradient
bar at its layout rect. `LegendChannel::Color` stays `Implemented` (`vocab.rs:276`) —
it now covers continuous as well as categorical.

## Verification (all headless)

Every acceptance criterion is provable without a window: the ramp math, scheme stops,
domain inference, `colorScheme` consumption, and legend geometry are unit/integration
tests; the raster and legend *appearance* are `BRIGHTFIELD_DUMP_PNG` dumps eyeballed
once (the only manual ACs), plus a conformance/preflight fixture proving a
`colorScheme` raster parses clean and preflights `Implemented`. No macOS window is
required — unlike the slider, there is no drag interaction here.

## Deferred (recorded)

- Diverging schemes (RdBu, BrBG, …) — needs a two-sided domain + midpoint.
- `colorScale` transforms: `log` / `sqrt` / `symlog` / `pow` (v1 is `linear` only).
- Custom `colorDomain` / `colorRange` / `colorReverse` from the spec.
- Per-mark scheme override on the **live cross-filter** path (headless honours it;
  live inherits the viridis default — Decision 6).
- Full-resolution (256-entry) scheme LUTs — v1 uses 7–9 control points interpolated.
- The `heatmap` / `cell` categorical-grid marks that will *consume* this scale.
- Generic numeric-Fill → Sequential inference in `infer_column_scale` (v1 produces the
  scale only via the raster's `augment_scales`; a `dot`/`cell` with `fill: <numeric>`
  driving a continuous scale is future work).
