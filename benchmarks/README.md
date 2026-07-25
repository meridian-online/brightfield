# Benchmarks — the measured performance baseline

This directory holds the numbers this project is allowed to quote about
itself. Nothing here is inherited: Mosaic's published figures were measured on
Mosaic's coordinator, and adopting its architecture is not the same as
inheriting its benchmarks. Every number in `results/` was measured in this
repository, by the committed harness, and is recorded with its **date,
machine, dataset and methodology** — a number without those four is not a
result. A CI gate (`scripts/check-borrowed-benchmarks.sh`) keeps upstream
figures from being restated as this project's own.

## Re-measuring is one command

```sh
./scripts/bench-baseline.sh                 # the full baseline (release build)
./scripts/bench-baseline.sh -- --quick      # fast smoke pass
./scripts/bench-baseline.sh -- --skip-frames    # engine suites only (no GPU)
```

The harness is `crates/brightfield-bench`. It writes a JSON record and a
generated Markdown summary into `results/`, named `<date>-<machine>`.
Comparing an engine change is running the same command on the same machine
and reading the two records side by side — the scenario specs in `specs/` are
compiled into the harness, so the committed scenario and the executed
scenario cannot drift apart.

## What is measured

Four scenarios scale over deterministic Parquet datasets at 10⁴, 10⁵, 10⁶
and 10⁷ rows (columns are pure functions of the row index via DuckDB `hash()`
— no RNG; the files regenerate on demand into `.data/`, which is gitignored):

- **brush-density** — a raw dot scatter carrying an interval brush, beside a
  `densityX` that re-aggregates the full table on every brush step. The
  aggregating shape: the picture stays O(bins) while the data path scales
  with rows. Its brushed column (`value_a`) is ~unique per row — the shape
  where the pre-aggregation layer's first cut (raw-valued active dimensions)
  buys little, recorded on purpose.
- **brush-binned-density** — the same shape over a brushed column with
  exactly forty distinct values (`value_c`): the derived cube stays
  O(bins × 40) at any row count. The pre-aggregation layer's intended shape.
- **crossfilter-dots** — two linked raw dot plots, each cross-filtering the
  other. The row-per-mark shape: both the data path and the picture scale
  with rows; there is nothing to pre-aggregate, and the harness verifies the
  layer stays silent. Its frame suites cap at 10⁶ rows; its engine suites run
  at every magnitude.
- **slider-drag** — a range slider dragged across its stops, filtering every
  view. The only scenario whose gesture is not a brush, and the reason the
  record carries a `drag` field per row (see *Two gestures* below).

One fixed-scale scenario is opt-in, because its dataset is real rather than
generated: **crosswalk-confidence** measures the published EDGAR–GLEIF
company-identifier crosswalk (an interval brush over its `confidence`
column, re-aggregating a density). Pass a local copy of the published
parquet: `--crosswalk-parquet <path>`.

## Two gestures, and why the slider drives a selection

A brush and a slider are not the same interaction wearing different clothes,
so the record names which one produced each row:

- A **brush** is dragged inside a chart and moves BOTH interval endpoints. It
  resolves as `crossfilter`, which exempts the contributor's own plot from its
  own predicate — so a brush step re-queries every view *except* the one being
  dragged in.
- A **slider** pins its lower end and advances one handle across fixed stops.
  It is not a view and has no picture of its own to spare, so it resolves as
  `intersect` and filters EVERY subscriber, the plot that declares it
  included. A slider step therefore re-queries one more view than a brush step
  over the same data.

The slider scenario contributes an **interval clause to a selection**, not a
scalar param. Upstream Mosaic expresses a range slider as `select: interval`,
and this engine's pre-aggregation layer keys its cubes off that structured
clause — the only path that reaches it is selection propagation. A slider
wired to a scalar param arrives at the query layer as a substituted expression
predicate, which decomposes into no cube at all; measuring that would measure
a different mechanism under the slider's name. The vocabulary has no
slider-to-selection widget form yet, so the scenario spec declares the
contributor as the interval interactor it does have, and the harness drives it
with slider-shaped steps. **No row in any committed record took the
scalar-param path.**

The slider's steps land exactly on the brushed axis's forty stops. A step
falling *between* two stops would emit different SQL while selecting identical
rows — reporting a re-query that did no new work as though it had.

**Every engine suite runs twice on identical code** — automatic
pre-aggregation enabled (`engine`, the shipped configuration) and disabled
(`engine_direct`, the direct-query control) — so the difference between the
two brush-step latencies is attributable to the layer alone. The record
carries the layer's behaviour counters (`preagg`: cubes built, brush steps
served from a cube, failures), and a run whose cube behaviour contradicts
the scenario's expectation **fails instead of reporting**.

Per scenario × row count, the record holds:

| Field | Meaning |
|---|---|
| `load_ms` | `Coordinator::load` — parse-to-ready session (DDL, no mark queries) |
| `first_materialise_ms` | first full materialisation of every mark |
| `coordinator_apply` | per-brush-step latency at the coordinator seam: predicate push-down into DuckDB + re-query of every affected mark. Recorded twice: `engine_direct` (layer off) vs `engine` (layer on) — **the before/after pair the pre-aggregation layer is measured by.** |
| `live_apply` | `coordinator_apply` plus the re-composite into a Vello scene — the full cost the live window's frame blocks on for one committed brush step (the shell applies interactions synchronously in-frame) |
| `preagg` | what the layer did during the suite: enabled, cubes built, brush steps served from a cube, build/serve failures — the non-vacuity evidence beside the latencies |
| `frames.steady` | headless full-window frame time with nothing changing — the shell's floor |
| `frames.interaction` | headless full-window frame time where every frame carries one committed brush step: re-query + re-composite + canvas re-raster + GPU wait |
| `marks[].materialised_rows` / `drawn_rows` | how many rows the mark's query answered vs how many the composed scene draws — the presentation layer assembles **every** Arrow chunk (measured through that same assembly path), so the two are equal; a regression that reintroduced a first-chunk cap would show `drawn_rows` < `materialised_rows` here |
| `marks[].chunks` / `chunk_bytes` / `assembled_bytes` | the compose's working set per mark: how many Arrow chunks it is handed, their exact byte size, and the assembled drawable's. A single-chunk mark assembles by pass-through, so its assembled bytes are the SAME allocation counted again, not a second copy |
| `compose_memory` | what the client held while the first full scene was composed — see *The compose's memory* below |
| `unfiltered_step_rows` / `brushed_step_rows` | non-vacuity evidence: the cross-filtered step's row count without and under the final drag step — the harness fails if a drag filtered nothing |
| `drag` | which gesture drove this row's timed steps (`brush` or `slider`) |

Plus `corpus`: steady-state frame time for every spec in `examples/*.yaml`.

## The compose's memory

`compose_from_results` assembles every Arrow chunk of every mark and holds
them all while it lays out and rasters the scene. At ten million row-level
rows that is the whole result set resident in the client. `compose_memory`
records the window around the first full compose two ways, because neither is
honest alone:

- `arrow_chunks_mib` / `arrow_assembled_mib` — **exact and deterministic**,
  summed from the batches themselves (`RecordBatch::get_array_memory_size`).
  Identical on any host, independent of the allocator. This is the data the
  compose holds.
- `rss_before_mib` / `rss_peak_mib` / `rss_growth_mib` — the **process's**
  resident set size, polled across that window. It is the only figure that
  sees those buffers' real cost: the chunks are imported from DuckDB over the
  C data interface, so DuckDB's C++ allocator owns them and a Rust
  global-allocator counter would not see them at all.

**Read the peak, not the growth.** RSS is cumulative within a run — a
general-purpose allocator does not hand freed pages straight back, so a
scenario measured late starts from an already-high floor and its growth can
read near zero while the process holds hundreds of megabytes. The probe is
best-effort and names itself (`sampler`): Linux reads `/proc/self/status`,
macOS shells out to `ps -o rss=` (~5 ms resolution, so `rss_samples` states
how coarse a given cell is), and a host with neither records `null` rather
than a number it did not measure. The poller stops before the timed applies,
so it cannot perturb a reported latency.

## Record schema

Records carry a `schema` id and are **not** comparable across a bump.

- `brightfield-bench/v2` carried `first_batch_rows` beside
  `materialised_rows`: the presentation of the day drew a mark's FIRST Arrow
  chunk only, so a ten-million-row query showed 2048 drawn. That presentation
  is gone — the compose assembles every chunk — and the field is now
  `drawn_rows`. The id stayed at v2 for one commit after the behaviour
  changed, which shipped two field sets under one version.
- `brightfield-bench/v3` names what it holds: `drawn_rows`, plus `drag`,
  per-mark chunk/byte shape, `compose_memory`, and `frames_skipped`.

**A v2 record's numbers describe code that no longer ships.** Do not quote one
against a v3 record; re-measure instead. `results/2026-07-23-apple-m1-pro.*`
is the last v2 record and is kept only as history: its frame cells for
row-per-mark scenarios were measured on scenes drawing ~2048 rows per mark,
whatever the row count in the same table row says.

## When a frame cell is blank

Frame suites are capped by **drawn row-level primitives**, not table rows. Each
row-per-mark mark contributes one primitive per materialised row (an
aggregating mark contributes none — its picture stays O(bins)), and above one
million summed primitives the composed scene exceeds the renderer's
`max_*_buffer_binding_size` and **the frame does not render at all**. On the
reference machine the process aborts inside the wgpu validation layer, so the
harness cannot record an error — it can only decline to try.

The cap became load-bearing the moment the compose began assembling every
Arrow chunk instead of the first: before that, a "ten-million-row" frame cell
was a ~2048-row scene. Every skipped row records **why**, in the JSON
(`frames_skipped`) and as a named list under the generated table. A blank
frame cell is not a fast frame.

## Methodology honesty

- **Drag steps never repeat an interval** — the engine caches repeated
  identical SQL, and a repeated interval would time the cache, not the
  engine. Both step generators (brush and slider) hold the same distinct
  period, so the harness's single `--iterations` cap keeps the guarantee for
  both shapes.
- **Frame times are headless**: the real `MeridianApp` drawn by egui's real
  wgpu backend into an offscreen texture, timed per frame through GPU
  completion. No swapchain, no present, no vsync — the cost of *producing* a
  frame, not displaying one. Warm-up frames are discarded.
- **Cold open is process-warm**: the session is fresh but the Parquet file is
  warm in the OS page cache.
- **Selection placement**: the emitted SQL applies a selection predicate
  *inside* an aggregating mark's query — it filters the base rows that get
  aggregated (row-level marks are wrapped whole). The aggregating scenarios
  keep their original brush-the-binned-column shape so the measured series
  stays comparable across harness runs.
- **The cube's first cut is raw-valued**: active interval dimensions enter a
  derived cube at raw data values (answer-exactness over cube size), so a
  cube over a ~unique-per-row brushed column approaches the base table's
  size. brush-density records that honestly; brush-binned-density,
  slider-drag and the crosswalk record the bounded-cardinality shape the
  layer is built for. Frame suites run in the shipped (layer-on)
  configuration only.
- **A cubed cell with `Cube 0/0` is not a cube cost.** Where the layer built
  nothing (crossfilter-dots: row-level marks, nothing to pre-aggregate), the
  `cubed` and `direct` columns ran the SAME code and their difference is
  run-to-run variance, not the price of anything. Quoting a `Cube 0/0` cubed
  figure as a cost is a misreading the record cannot prevent — only the
  reader can.

The full methodology text ships inside every JSON record, so a result file
remains self-describing after this README moves on.
