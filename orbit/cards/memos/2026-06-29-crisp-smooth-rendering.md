# Crisp + smooth rendering — HiDPI rasterisation + cached base raster

Card 0013. Two render-quality wins flagged in the live-window review: the chart was
soft on Retina (rasterised at logical resolution, then upscaled by the compositor),
and every hover/brush frame re-rasterised the whole scene through Vello with a
synchronous GPU readback.

## What shipped (`brightfield-ui`)

- **Device-resolution rasterisation (crisp).** `ChartState::base_image(scale_factor)`
  rasterises the scene at `ceil(width × scale_factor) × ceil(height × scale_factor)`,
  scaling the (logical-coordinate) scene to fill exactly that device size via
  `Affine::scale_non_uniform`. `paint_image` then maps the device-res tile 1:1 into
  the scaled bounds, so the chart is sharp on HiDPI instead of a stretched
  logical-res tile. Exact at integer Retina scale (2.0) and at fractional factors.
- **Cached base raster (smooth).** `base_image` caches the rendered `Arc<RenderImage>`
  and reuses it while the scene and device dimensions are unchanged; the cache is
  invalidated in `set_scene` / `set_dimensions`. The cache lives on `ChartState`
  (one per chart) — not the shared renderer — so multiple charts never serve or
  evict each other's raster.
- **Overlay as GPUI quads.** The brush rectangle and hover marker are now painted as
  `paint_quad`s on top of the cached image (`chart_element::paint_overlay`) instead of
  being composited into the Vello scene. So a hover/brush repaint is a cache-hit
  `Arc` clone + one quad — no Vello re-raster. Colours/geometry mirror the old
  in-scene overlay exactly.
- **`BRIGHTFIELD_DUMP_SCALE`** added to the PNG path so the device-res rendering is
  verifiable headlessly.

## Verification — and its limit

- Crispness is **PNG-verified**: `BRIGHTFIELD_DUMP_SCALE=1/1.5/2` produce correct,
  full charts at 640×480 / 960×720 / 1280×960 (100% coverage, scene fills each size).
- Compiles clean (no warnings); `cargo test --workspace` green.
- The window path (cache + quad overlay) can't be runtime-tested here (no
  display/Metal). It was adversarially reviewed against the gpui source — verdict:
  crisp and smooth will work correctly in a real single-chart macOS window; the
  smoothness holds (hover/brush hit the cache, no `render_to_pixels`), and the
  overlay quads land exactly where the in-scene overlay did.
- **Needs a human to confirm in a window**: that the chart looks sharp on a Retina
  display and that hovering/brushing feels smooth.

## Review fixes applied

- Moved the cache from the shared `VelloRenderer` to per-chart `ChartState` — the
  shared single-slot cache would have served the wrong chart / thrashed once the
  multi-view card shares one renderer.
- `ceil` device dims + non-uniform scene scaling, so fractional scale factors
  (Linux/Windows fractional scaling) stay crisp (no ≤1px edge stretch).
- Hoisted the zero-size guard above `base_image` so a transient zero-size frame
  doesn't waste a render.

## Deferred follow-ups

- A scene change drops the old `RenderImage` but never calls `window.drop_image`, so
  one sprite-atlas tile leaks per reload. Vastly better than the old path (which
  minted a new `RenderImage` every paint), but worth a `drop_image` on invalidation.
- The quad border is drawn inset vs the old centred kurbo stroke (≤0.75px cosmetic).
- Multi-view (card 0006/0009) gets correct per-chart caching for free from this
  design — no shared-renderer cache to untangle.
