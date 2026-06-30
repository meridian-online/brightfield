# Cross-filter — live window wiring (card 0006)

Builds on the merged headless foundation (`2026-06-30-crossfilter-foundation.md`).
Brushing a plot in the running window now re-queries and re-renders the linked
plots. The headless integration test already proved the data path; this
increment is the GPUI glue that drives it live.

## What shipped

A new `CrossfilterCoordinator` (`brightfield-ui/src/crossfilter.rs`) keeps the
engine `Session` and per-mark / per-plot render metadata alive past the initial
render. The chain on a brush release:

```
pixel brush rect (element-local)
  → invert to data coords via the plot's ScaleSet   (invert_pixel_brush, unit-tested)
    → commit_brush_release_multi into the live Session
      → re-executed subscriber batches
        → rebuild each affected plot's scene          (build_plot_scene)
          → set_scene + notify on its ChartState
```

Wiring:
- **`build_everything`** (main.rs) replaces `run_pipeline`'s guts and returns
  `(Dashboard, LiveParts)`. `run_pipeline` stays a thin wrapper returning only
  the `Dashboard` and **dropping** the `LiveParts` — that drop of the non-`Send`
  `Session` is what keeps the hot-reload watcher's off-thread pipeline run
  `Send`-safe (unchanged).
- The macOS window builds the coordinator from `LiveParts` + the per-plot
  `ChartState` entities (joined in dashboard order), wraps it `Rc<RefCell<…>>`,
  and threads it into each `PlacedChart` → `ChartElement`.
- `ChartElement`'s mouse-up handler reads this plot's `Brushing` rect (before
  `pointer_up` clears it) and calls `coordinator.commit_brush(plot_index, …, cx)`.
- The coordinator is `None` when no plot has a brush binding, so non-interactive
  specs behave exactly as before.

`Session` can't pull in `arrow`, so `brightfield_engine::concat_batches` was
added (uses duckdb's bundled Arrow) to fold re-execution chunks into one batch.

Interaction model (v1): **drag to filter, click to clear.** A drag dispatches the
selection; a click (zero-area gesture) on a plot retracts its contribution.

## Key correctness decisions

- **Stable plot identity** (from the foundation fix): the brush contributor and
  the subscriber `self_source` both use the plot-node path, so a plot never
  filters itself.
- **Multi-listener gating.** `on_mouse_event` listeners are window-level, so
  every plot's up-listener fires on each release. `commit_brush` returns
  immediately unless THIS plot's interaction is `Brushing`, so an idle sibling
  can't clear its own selection on someone else's release. (Only the press-target
  plot is ever `Brushing`: mouse-down is hitbox-gated, and `pointer_move` only
  extends an existing brush — dragging over a sibling doesn't start one there.)
- **Verifiability.** The risky surface was kept small: `invert_pixel_brush` is a
  pure, unit-tested function (incl. the y-axis flip and a categorical fallback);
  the dispatch→re-execute→scene-rebuild path is the already-merged headless
  integration test. Only the thin GPUI listener/`set_scene` glue is unverified
  here (no display/Metal in CI) — adversarially reviewed against the gpui source.

## Deferred / known rough edges

- **Persistent brush rectangle.** `pointer_up` resets the overlay on release, so
  the brushed rect vanishes even though the filter stays applied. Mosaic keeps
  the selection rect visible; that needs a committed-brush visual state.
- **Hot-reload vs. cross-filter.** The watcher rebuilds scenes from a fresh
  `Session`; the coordinator holds the original. After a live spec edit the
  coordinator's data/scales are stale until restart. Reconciliation (re-apply
  the selection after reload, or hand the watcher the coordinator) is a follow-up.
- **Axes rescale on filter.** A re-filtered subscriber re-infers its domains, so
  axes can jump. Mosaic uses fixed domains for the brushed dimension
  (`xDomain: Fixed`) — wire that through for stable axes.
- **Empty selection → blank plot.** A brush that filters a plot to zero rows
  rebuilds to an empty scene (no axes). Acceptable, but a "no data" state would
  be friendlier.
- **Categorical brush axes** invert to a no-op (only numeric Linear/Time axes
  brush meaningfully today).
- **Channels from the first mark only** (carried from the foundation): multi-mark
  plots brush against the first mark's x/y.

## Needs a window eyeball

`cargo run -p brightfield-app -- examples/crossfilter.yaml`: two linked scatter
plots; drag an x-range in either to filter the other, click to clear. (The
static initial render is verified headlessly; the live brush interaction is the
part to confirm by eye.)
