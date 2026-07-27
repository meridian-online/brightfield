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
  layer stays silent. Its engine suites run at every magnitude; its frame
  suites stop after 10⁵, because two dot plots over 10⁶ rows is 2,000,000
  drawn primitives against a uniform 1,000,000 cap.
- **slider-drag** — a range slider dragged across its stops, filtering every
  view. The only scenario whose gesture is not a brush, and the reason the
  record carries a `drag` field per row (see *Two gestures* below).

One fixed-scale scenario is opt-in, because its dataset is real rather than
generated: **crosswalk-confidence** measures the published EDGAR–GLEIF
company-identifier crosswalk (an interval brush over its `confidence`
column, re-aggregating a density). Pass a local copy of the published
parquet: `--crosswalk-parquet <path>`.

Every scenario is additionally measured under a **settled pan/zoom**: one
navigation extent applied to plot B along that plot's own x channel, with every
mark of that plot re-queried. It is a third gesture rather than a fourth
scenario, because what it measures is a different PATH over the same shapes:
navigation resolves to an extent on the session, not to a selection.

That column is why this gesture is measured at all. It used to record a
navigation taking the direct query at every row count in BOTH configurations,
because the pre-aggregation layer's only trigger was selection propagation and
an extent reached it by no path — a chart whose brush was instant and whose
zoom was not, on the same plot, in the same session. The engine now keys a cube
off the extent as well, so the column is printed for both configurations,
`Settled zoom → data, direct` and `… cubed`, and their delta is what the
connection bought. Read the cubed figure beside `Step → data, cubed`: on a
shape whose cube derives, a settled zoom and a brush step are now the same kind
of query over the same pre-aggregate. On a shape whose cube does not derive —
a row-level mark has nothing to pre-aggregate, and an axis naming an aggregate
output cannot be pushed beneath the `GROUP BY` — the two settled-zoom columns
should agree, and a difference there is a finding, not a speed-up.

The harness reads the navigated column and its range off the mark's own drawn
batch rather than reusing the brushed column, and fails the run if the extent
did not reduce that mark's rows. An earlier revision reused the brush column,
which two of the four scenarios do not plot on that axis: the extent matched no
mark, the emitted SQL came back unchanged, the cache served it, and the record
printed 0.0 ms for a gesture that had done nothing.

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
carries the layer's behaviour counters (`preagg`: cubes built, **mark
re-queries** served from a cube, failures), and a run whose cube behaviour
contradicts the scenario's expectation **fails instead of reporting**.

`cube_hits` counts one hit per MARK the layer serves, not one per drag step.
A scenario whose selection filters two subscribing marks records two hits per
step, so a twenty-step slider suite reads `2/40` — two cubes, forty mark
re-queries — in a document whose preamble says twenty steps. Reading that
column as a step count is the misreading it invites; the generated summary's
legend now says which it is.

Per scenario × row count, the record holds:

| Field | Meaning |
|---|---|
| `load_ms` | `Coordinator::load` — parse-to-ready session (DDL, no mark queries) |
| `first_materialise_ms` | first full materialisation of every mark |
| `coordinator_apply` | per-brush-step latency at the coordinator seam: predicate push-down into DuckDB + re-query of every affected mark. Recorded twice: `engine_direct` (layer off) vs `engine` (layer on) — **the before/after pair the pre-aggregation layer is measured by.** |
| `live_apply` | `coordinator_apply` plus the re-composite into a Vello scene — the full cost the live window's frame blocks on for one committed brush step (the shell applies interactions synchronously in-frame) |
| `preagg` | what the layer did during the suite: enabled, cubes built, **mark re-queries** served from a cube (one per served mark per drag step — not a step count), build/serve failures — the non-vacuity evidence beside the latencies |
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
  compose holds, and **this is the figure to quote**.
- `rss_before_mib` / `rss_peak_mib` / `rss_growth_mib` — the **process's**
  resident set size, polled across that window. It is the only figure that
  sees those buffers' real cost: the chunks are imported from DuckDB over the
  C data interface, so DuckDB's C++ allocator owns them and a Rust
  global-allocator counter would not see them at all.

**The RSS peak is one sample of a whole-process quantity, and it does not
reproduce on its own.** Every scenario records two windows — layer off and
layer on — over the *same* compose work, because the first present happens
before any interaction and the toggle cannot change it. Their disagreement is
this figure's run-to-run noise; in the committed record it reaches a factor of
two. The generated summary prints both peaks and states the widest spread it
observed, and **a peak must not be quoted without that spread**.

**The pre-compose reading is not a floor that only rises.** The OS reclaims
pages between windows: in the committed record `rss_before` falls sharply
between adjacent scenarios as often as it climbs. So `rss_growth_mib` is
neither the compose's cost nor a lower bound on it. An earlier revision of this
README told readers to treat resident size as monotone within one harness run
and to read the peak rather than the growth; the series printed beside it
refuted both, and the guidance is withdrawn.

The window measures ONE compose only because everything earlier is dropped
first: the coordinator-seam phase's session goes, and its complete result set
is moved into a consuming shape pass rather than left in scope. Resident size
counts the whole process, so a phase still holding its Arrow lands in the
compose's number — which is what an earlier revision of the harness did,
adding the coordinator phase's entire result set to the largest cell.

The probe is best-effort and names itself (`sampler`): Linux reads
`/proc/self/status`, macOS shells out to `ps -o rss=` (~5 ms resolution, so
`rss_samples` states how coarse a given cell is), and a host with neither
records `null` rather than a number it did not measure. The poller stops
before the timed applies, so it cannot perturb a reported latency.

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

## Errata — what this directory has published and withdrawn

Kept because a record that quietly corrects itself teaches nobody what to
distrust next time.

**A compose-memory column that measured two result sets.** The first record to
carry `compose_memory` opened its sampling window while the coordinator-seam
phase's complete Arrow result set was still alive: the coordinator was dropped,
its result set was not. Resident size is a whole-process quantity, so the
published figure was both phases summed. The arithmetic gave it away in the
record's own JSON — the largest cell reported 210.6 MiB of growth for a compose
assembling 729.5 MiB of fresh Arrow, which is only possible if the pages were
already there. The shape pass now consumes the result set, and the whole record
was re-measured rather than annotated.

That defect was not confined to the memory column. Holding a second full result
set through the timed loop cost latency at ten million rows, and re-measuring
moved published figures the record presented as engine behaviour:

| Figure (10⁷ rows, p50) | Contaminated | Re-measured |
|---|---:|---:|
| crossfilter-dots · step → data, direct | 305.3 ms | 135.3 ms |
| brush-density · step → scene, direct | 4458.6 ms | 2859.1 ms |
| brush-density · step → scene, cubed | 11958.1 ms | 2738.8 ms |
| brush-density · step → data, cubed | 23.8 ms | 4.1 ms |
| brush-binned-density · step → scene, cubed | 2596.5 ms | 1967.2 ms |

**A data-seam claim built on that contamination.** The contaminated record
showed `crossfilter-dots` at 10⁷ moving 2.3x at the coordinator seam — a seam
that composes nothing — and the summary hedged its "the data seam barely moved"
finding to "below 10⁷" to accommodate it. Re-measured, that cell reads 135.3 ms
against the v2 record's 130.1 ms. **The data seam moved by at most ~13% at every
magnitude measured, 10⁷ included**; the hedge was describing the defect, not the
engine.

**`Cube 2/40` never meant forty drag steps.** The generated summary's legend
read "cubes built / drag steps served from a cube" while the engine counts one
hit per MARK it serves. A twenty-step slider suite filtering two views records
forty hits, in a document whose own preamble says twenty steps. The legend, this
README and the methodology block now say "mark re-queries". A commit message on
this branch describes that row as "all 40 steps served"; that wording is wrong
and is corrected here rather than by rewriting history.

**What the RSS peak still is not.** Even with the result set dropped, the peak
at the largest magnitudes is a process high-water mark rather than the compose's
marginal cost: the allocator does not return freed pages to the OS within a run,
so the compose is partly served from pages the coordinator phase already had.
`crossfilter-dots` at 10⁷ shows it plainly — 332 MiB of growth while assembling
729.5 MiB of Arrow, which means most of that Arrow needed no new pages. Quote
`arrow_chunks_mib`; treat the peak as an upper bound on the process, stated with
its spread.

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

**Three cells lost frame coverage under this cap**, and the current record has
gaps where the v2 record had numbers:

| Cell | v2 steady / interaction (p50, ms) | now |
|---|---:|---|
| crossfilter-dots @ 10⁶ | 1.6 / 21.8 | no frame |
| brush-density @ 10⁷ | 1.6 / 9.0 | no frame |
| brush-binned-density @ 10⁷ | 1.6 / 5.4 | no frame |

Those v2 numbers are not a baseline the current record failed to beat — they
timed scenes of ~2048 rows per row-per-mark mark, which is why they could be
produced at all. The honest statement is that **at these magnitudes this
renderer cannot produce a frame for a row-per-mark scene**, and the coverage
that existed before was coverage of a different picture. Any claim that a
re-measurement covered "every existing row" is true per row and false per
cell: three cells regressed to gaps.

## Methodology honesty

- **Drag steps never repeat an interval** — the engine caches repeated
  identical SQL, and a repeated interval would time the cache, not the engine.
  Both step generators (brush and slider) hold the same 35-step period, so one
  bound covers both shapes — but the harness has **two step budgets** and each
  needs its own check. The engine suites step with `--iterations`; the
  interaction *frame* suite indexes its step by the frame counter, so its
  budget is `--warmup-frames + --frames`. Both are validated at startup and a
  run that would wrap is **rejected**, not silently cached. The shipped default
  (5 + 30) sits exactly on the period, so the frame check is load-bearing:
  before it existed, `--frames 31` re-issued step 0 and timed the cache.
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
