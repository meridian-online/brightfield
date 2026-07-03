# Raster mark — follow-ups

**Card 0008 (mark coverage breadth).** The `raster` mark landed as a **binned 2D
count heatmap**: filled cells, one per occupied bin, opacity ∝ raw count. It
reuses the existing 2D density binning end-to-end (`DensityLowerer{TwoD}` emits
`(x_centre, y_centre, __bf_count)`), so the whole mark is one renderer +
registration — no new SQL, no new scale type, no new channel.

## What it is (and isn't)

- `RasterRenderer` (brightfield-render `mark.rs`) forks `Density2DRenderer` but
  draws a `kurbo::Rect` per bin (centre ± half a bin) instead of a KDE-smoothed
  circle, and encodes the **raw** count rather than a smoothed density.
- Encoding is **alpha on one hue** (steelblue), with a `RASTER_MIN_ALPHA` floor
  so every occupied cell (count ≥ 1) stays visible.
- `augment_scales` widens the linear x/y domains by half a bin so the edge cells
  fit inside the plot rather than overflowing into the axis margins.

## Follow-ups

1. **Sequential colour ramp.** The single biggest gap. `Scale` has no
   continuous-colour variant — `infer_column_scale` maps a numeric count to
   `Scale::Linear`, and `resolve_colour` only reads categorical string fills. A
   true viridis-style `count → gradient` needs a new
   `Scale::Sequential { domain, stops }` + interpolation + inference + colour-legend
   wiring. That's net-new scale infrastructure that dwarfs the renderer, so this
   mark ships with alpha-on-one-hue. When the sequential scale lands, raster
   (and a future `heatmap`/`cell`) should switch to it.
2. **Sparse-data bin pitch.** `bin_step` recovers the bin width as the **GCD** of
   the gaps between occupied centres (equiwidth centres sit at `lo+(bucket+0.5)·w`,
   so every gap is an integer multiple of `w`). This is correct even when no two
   occupied bins are adjacent — the case the adversarial review caught, where
   discrete integer data with `bins` over-specified relative to the range put
   consecutive values in non-adjacent buckets and a min-gap estimate drew cells 2×
   too wide. The GCD search is capped at `k ≤ 12`, so pathologically sparse data
   (occupied bins always >12 apart) degrades gracefully to the min gap (cells
   over-cover, never gap or panic). A lowerer that emitted the bin width explicitly
   in the batch would remove the inference entirely — the cleaner long-term fix.
3. **Highlight / cross-filter.** Like `Density2DRenderer`, the raster ignores the
   `highlight` argument — a brushed selection doesn't dim non-matching cells.
   Aggregated marks don't carry per-source-row identity, so highlighting a raster
   needs a different approach (re-bin under the selection predicate, which the
   crossfilter path already does at the data layer).
4. **The rest of the specialised-mark set.** `hexbin` (hexagonal binning — new
   SQL hex math + hexagon geometry), `contour` (marching squares over the density
   grid — builds on the same 2D KDE), `heatmap`/`cell` (categorical grid — needs
   the sequential scale from #1), and `geo`/`raster`-with-interpolation remain.
   `raster` is the first and most tractable because the 2D binning already existed.
