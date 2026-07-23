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

Three scenarios scale over deterministic Parquet datasets at 10⁴, 10⁵, 10⁶
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

One fixed-scale scenario is opt-in, because its dataset is real rather than
generated: **crosswalk-confidence** measures the published EDGAR–GLEIF
company-identifier crosswalk (an interval brush over its `confidence`
column, re-aggregating a density). Pass a local copy of the published
parquet: `--crosswalk-parquet <path>`.

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
| `marks[].materialised_rows` / `first_batch_rows` | how many rows the mark's query answered vs how many the composed scene draws — the presentation layer currently composes a mark's **first Arrow batch only**, and this records where that truncates the picture |
| `unfiltered_step_rows` / `brushed_step_rows` | non-vacuity evidence: the cross-filtered step's row count without and under the final brush — the harness fails if a brush filtered nothing |

Plus `corpus`: steady-state frame time for every spec in `examples/*.yaml`.

## Methodology honesty

- **Brush steps never repeat an interval** — the engine caches repeated
  identical SQL, and a repeated interval would time the cache, not the
  engine.
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
  size. brush-density records that honestly; brush-binned-density and the
  crosswalk record the bounded-cardinality shape the layer is built for.
  Frame suites run in the shipped (layer-on) configuration only.

The full methodology text ships inside every JSON record, so a result file
remains self-describing after this README moves on.
