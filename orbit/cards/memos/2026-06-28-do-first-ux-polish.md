# Do-first UX polish — bringing the first render to life

A deep UX review (5 dimensions × find → adversarial-verify → synthesise) surfaced a
ranked set of low-hanging-fruit improvements. This memo records the **do-first tier**:
the changes with the highest perceived payoff and trivial/small effort. Each is
follow-up work against an existing card — distil into specs, don't open new cards.

The review's headline finding: the rendered chart — the actual product — was the
*least alive* surface, and a working render looked broken. These four changes (plus a
shipped example) close that gap. Two larger structural opportunities (real glyph text;
wiring mouse events into the live window) were deliberately deferred to the high-value
tier — see the end of this memo.

## Shipped in this pass

### 1. White chart background (was rendering onto transparency)

`build_chart_scene` / `build_multi_mark_scene` drew grid, marks, axes and legend onto
a transparent canvas (`base_color: Color::TRANSPARENT` in the Vello renderer). A
`BRIGHTFIELD_DUMP_PNG` export therefore read as a black/checkerboard backdrop and the
0.867-grey gridlines floated on nothing — a finished render looked broken.

Fix: a `render_background` helper fills an opaque white rect over the full layout as the
first geometry in both scene builders.

- `crates/brightfield-render/src/scene.rs` — new `render_background`, called at the top
  of `build_chart_scene` and `build_multi_mark_scene`.
- → card 0013 (GPU-accelerated mark rendering) / card 0001.

### 2. Skipped-mark and skipped-channel warnings now reach the user

Both "no renderer for mark kind — skipping" (app) and "skipping ParamRef channel" (render)
were emitted via `tracing::warn!`, but **no `tracing` subscriber exists anywhere in the
workspace**, so they went to a no-op dispatcher and vanished. A mark that parsed and
executed but had no renderer disappeared from the chart with zero explanation — the most
confusing failure mode for an author.

Fix: both sites now `eprintln!` a `warning:` line, consistent with the rest of `main.rs`
(`"warning: skipping mark {i}: {e}"`). No new dependency. `tracing` was used *solely* for
these two warnings, so it was dropped from both crates' `Cargo.toml`.

- `crates/brightfield-app/src/main.rs` (no-renderer branch) — `tracing::warn!` → `eprintln!`;
  `tracing` dep removed.
- `crates/brightfield-render/src/channel.rs` (`from_mark` ParamRef branch) — same; `tracing`
  dep removed from `brightfield-render`.
- → card 0001 (graceful degradation) / card 0008.

### 3. Stop silently truncating results to the first Arrow batch

`run_pipeline` did `batches.into_iter().next()` — keeping only the **first** DuckDB chunk.
DuckDB streams results one batch per internal vector (~2048 rows), so any query wider than
a single chunk silently lost the rest: a 50k-point scatter rendered ~2k points with no
warning and a plausible-but-wrong chart. Silent data loss is the worst failure mode for an
analytics-authoring tool.

Fix: a `concat_result_batches` helper concatenates all chunks via
`arrow::compute::concat_batches`, returns `None` for an empty result, and on the rare concat
error falls back to the first chunk *with a warning* rather than dropping the mark. (DuckDB's
re-exported Arrow and the app's `arrow` crate resolve to the same `58.1.0`, so the batch
types are identical and no Cargo feature change was needed.)

- `crates/brightfield-app/src/main.rs` — new `concat_result_batches`, called in the `Ok`
  branch of the execute loop.
- → card 0012 (DuckDB execution engine).

### 4. A runnable first experience: example spec + README Quick Start

A fresh clone was a dead start — the README was rich on architecture but never said how to
run anything, and **no bundled spec is self-contained** (the curated specs all read from
`data/*.parquet` files that aren't in the repo).

Fix:
- `examples/scatter.yaml` — a self-contained inline-data scatter (dot mark, categorical
  `fill` to also exercise the colour legend). Renders with the shipped `DotRenderer`, no
  external files.
- `README.md` — a **Quick Start** section: the exact `cargo run` command, the
  `BRIGHTFIELD_DUMP_PNG` headless/Linux variant, a pointer to the bundled corpus (with the
  honest caveat that many need external data), and the macOS-only-window note.
- → card 0011 (single native binary distribution) / card 0001.

## Verification

- `cargo test -p brightfield-render` — 78 passed (covers the background fill and the
  `channel.rs` warning change; the empty-entries scene test still asserts 0 path tags
  because its early return precedes the background fill).
- Full `brightfield-app` build + `BRIGHTFIELD_DUMP_PNG` render of `examples/scatter.yaml`
  used to confirm the white background and end-to-end run. (See the PR / session notes for
  the recorded coverage figure.)

## Deferred to the high-value tier (next)

- **Real tick/legend text.** Every axis tick and legend label is still drawn as a
  fully-transparent placeholder rect (`axis.rs`, `legend.rs`); `skrifa` is already a
  dependency. Until this lands, axes are unlabelled — the single biggest remaining visual gap.
- **Wire mouse events into the live window.** Mouse/hover/brush/scroll/resize handlers are
  fully written and unit-tested but never wired into `ChartView::render`, and
  `InteractionState::render_overlay` is never composited — so the live macOS window is a
  frozen screenshot and the cross-filter sprint's payoff is invisible. Three "obvious"
  interaction one-liners were **refuted** in review because they all depend on this single
  unlock.
- **Zero-baseline bars** and **clip-to-plot-area** — bar charts overflow the x-axis because
  the y-domain isn't zero-anchored.
