# Cross-filter linked views — headless foundation + a self-exclusion bug

Card 0006. With multi-view dashboards shipped, brushing a range in one plot
should re-query and re-render the *other* plots, filtered to the selection.
A design workflow mapped the existing machinery against the new render
substrate; this memo records the headless foundation now in place.

## What the design pass found

Both halves of cross-filter already existed and were unit-tested *in isolation*,
but nothing connected them live:

- **Producer (UI):** `InteractionState` brush → `brush_rect_to_predicate` →
  `commit_brush_release_multi` dispatching `BrushBinding`s into a
  `SelectionDispatcher`. `From<&BrushableBinding>` converts spec-derived
  bindings to UI ones.
- **Analysis (static):** `analyse_spec` produces `brushable_bindings` (the brush
  sources, with resolved x/y channel columns) and `selection_subscribers` (the
  filtered targets). They join purely on the selection-name string.
- **Engine (consumer):** `Session::propagate_selection` stores the predicate,
  finds subscriber marks, re-emits each query (threading the predicate through
  `compile_selection` → a `Filter` node) and re-executes — returning
  `(mark_index, batches)` per subscriber. `Session` *is* a `SelectionDispatcher`.

The gap: every `commit_brush_*` and `propagate_selection` test drove a
`RecordingDispatcher` **double** — no test ever ran a brush through a *live*
`Session` over a real multi-plot spec. And the live GPUI window has no Session,
a dead mouse-up handler, no pixel→data inversion, and no result→scene bridge.

## Shipped this increment (headless, no GPUI)

`crates/brightfield-ui/tests/crossfilter_integration.rs` — the first end-to-end
proof of the chain with a real `Session` and a real analysis-derived
`BrushBinding` (not a hand-built struct, not a double):

> two-plot hconcat; plot A has `intervalX as: $brush`, plot B has a dot
> `filterBy: $brush`. Brush x∈[2.5,4.5] in **data** coordinates → plot B
> re-executes **6→2 rows**, `build_multi_mark_scene` over the filtered batches
> draws paths, and a clear round-trips back to 6 rows.

Coordinates are authored directly in column units; the pixel→data inversion the
live window needs is deferred (it's tractable — see below).

## Bug surfaced: a plot filters itself (self-exclusion broken)

The same test harness, pointed at a single plot that both brushes **and** is
filtered by its own selection, shows the plot filtering **itself** (`left: 2,
right: 6`). Under Mosaic `crossfilter` resolution a plot must *not* filter
itself. Kept as an `#[ignore]`d executable repro
(`crossfilter_plot_does_not_filter_itself`).

Root cause: `analysis::parent_plot()` returns the *item-index* segment of
whichever component path you hand it. The brush contributor is
`parent_plot(interactor_path)` = `…/plot[<interactor item index>]`; the
subscriber's `self_source` (emit.rs) is `parent_plot(mark_path)` =
`…/plot[<mark item index>]`. Within one plot the interactor and the mark have
different item indices, so the two "plot identities" never match and
`compile_selection`'s self-exclusion drop never fires.

This is a pre-existing latent defect masked by the `RecordingDispatcher` (which
never did real path matching). The fix is a **stable plot identity** shared by
both sides — the plot *node* path (the prefix before the synthetic `/plot[i]`),
which is exactly what `collect_plot_groups` already keys on. Cross-cutting
(analysis + emit + the cfs tests that hard-code `root/plot[0]` contributors), so
it's the next increment.

## Next increments

1. **Self-exclusion fix** — stable plot identity; un-`#[ignore]` the repro.
2. **Live window wiring** — keep the `Session` alive past `run_pipeline`
   (`Rc<RefCell<Session>>` on the main thread, since DuckDB is `!Send`); store
   each plot's `ScaleSet` on its `ChartState` (today discarded as `_scales`);
   invert the pixel brush via `Scale::inverse_f64` (already exists — it accounts
   for the y range direction, so this is wiring, not new math); route mouse-up
   through `commit_brush_release_multi`/`commit_brush_clear`; bridge returned
   batches back into the subscriber plot's scene via `set_scene` + notify (the
   same swap the hot-reload watcher already uses).
3. **Clear/retract + watcher reconciliation**, plus an `examples/crossfilter.yaml`
   the window can demonstrate.

## Deferred / known limits (from the design pass)

- `brushable_bindings` resolves channels from a plot's *first* child mark only;
  multi-mark plots or later-mark brushed columns aren't handled.
- Selections don't flow through the topological param walk (separate single-level
  fan-out), so chained selection→param→query cascades won't propagate.
- A degenerate brush (unbound channel) dispatches `Predicate::True` rather than a
  clear — decide no-op vs clear when wiring the live path.
