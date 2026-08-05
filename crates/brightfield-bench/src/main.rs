//! `brightfield-bench` — the measured performance baseline.
//!
//! One command produces the numbers this project is allowed to quote about
//! itself: interaction latency and frame time, against row count across four
//! orders of magnitude, on a recorded machine — plus steady-state frame times
//! over the shipped example corpus. Nothing here inherits an upstream figure;
//! everything is measured on THIS engine, in THIS repo, and written down with
//! its date, machine, dataset and methodology.
//!
//! Usage (from the repository root; release profile is the measurement
//! profile):
//!
//! ```text
//! cargo run --release -p brightfield-bench                # the full baseline
//! cargo run --release -p brightfield-bench -- --quick     # a fast smoke pass
//! cargo run --release -p brightfield-bench -- --skip-frames   # engine only, no GPU
//! ```
//!
//! Results land in `benchmarks/results/` as a JSON record and a generated
//! Markdown summary. Re-measuring after an engine change is running the same
//! command again — the scenario specs are compiled into the binary from
//! `benchmarks/specs/`, so the committed scenario and the executed scenario
//! cannot drift apart.

mod data;
mod frame_ink;
mod frames;
mod machine;
mod mem;
mod scenario;
mod stats;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;

use brightfield_render::sample_policy::{
    MEASURED_BLANK_MIN, MEASURED_INKED_MAX, VELLO_BIN_DATA_PANIC_DOTS,
};

use crate::machine::MachineProfile;
use crate::scenario::{Drag, EngineMeasurement, Scenario};
use crate::stats::Stats;

/// The density-pair scenario spec, compiled in from `benchmarks/specs/`.
const SPEC_DENSITY: &str = include_str!("../../../benchmarks/specs/crossfilter-density.yaml");
/// The bounded-cardinality density spec, compiled in from `benchmarks/specs/`.
const SPEC_BINNED: &str = include_str!("../../../benchmarks/specs/crossfilter-binned-density.yaml");
/// The raw-dot scenario spec, compiled in from `benchmarks/specs/`.
const SPEC_DOTS: &str = include_str!("../../../benchmarks/specs/crossfilter-dots.yaml");
/// The range-slider drag spec, compiled in from `benchmarks/specs/`.
const SPEC_SLIDER: &str = include_str!("../../../benchmarks/specs/slider-drag.yaml");
/// The crosswalk scenario spec (opt-in, fixed scale), compiled in from
/// `benchmarks/specs/`.
const SPEC_CROSSWALK: &str = include_str!("../../../benchmarks/specs/crosswalk-confidence.yaml");

/// The benchmarks README, compiled in for tests only: it makes claims ABOUT
/// this harness, and a claim that drifts from the code is how a record grows a
/// methodology section its own JSON refutes.
#[cfg(test)]
const BENCH_README: &str = include_str!("../../../benchmarks/README.md");

/// The render crate's blank-frame gate, compiled in for tests only.
///
/// That gate renders a dot scatter AT this harness's cap and requires it to
/// reach the target, and one past the measured overflow onset and requires it
/// not to — which is what makes the cap's margin a measurement rather than a
/// preference. Both counts are literals over there, because a binary crate's
/// constants cannot be imported; reading the file back is how the two stay one
/// fact instead of two that agree today.
#[cfg(test)]
const RENDER_INK_GATE: &str = include_str!("../../brightfield-render/tests/frame_ink.rs");

/// One committed scaling scenario: which spec, which gesture, which column,
/// what the pre-aggregation layer is expected to do.
struct SpecDef {
    name: &'static str,
    template: &'static str,
    /// The gesture the timed steps imitate — a brush moves both interval
    /// endpoints, a slider moves one handle across fixed stops.
    drag: Drag,
    brush_column: &'static str,
    brush_domain: (f64, f64),
    /// Whether the enabled run must show the cube engaging (checked both
    /// ways — see the scenario module's non-vacuity guards).
    expect_cube: bool,
    /// Marks in this spec that draw ONE primitive per materialised row, so
    /// their contribution to the encoded scene scales with the table. An
    /// aggregating mark counts zero: its picture stays O(bins) at any row
    /// count. This is what [`FRAME_DRAWN_ROW_CAP`] is applied against.
    row_level_marks: u64,
}

/// The scaling scenarios, in report order.
const SCENARIOS: &[SpecDef] = &[
    SpecDef {
        name: "brush-density",
        template: SPEC_DENSITY,
        drag: Drag::Brush,
        brush_column: "value_a",
        brush_domain: (0.0, 100.0),
        // The cube engages, but `value_a` is ~unique per row and active
        // dimensions are raw-valued in the first cut, so the cube grows with
        // the table — the record shows what that costs.
        expect_cube: true,
        // The scatter is row-per-mark; the densityX beside it is not.
        row_level_marks: 1,
    },
    SpecDef {
        name: "brush-binned-density",
        template: SPEC_BINNED,
        drag: Drag::Brush,
        brush_column: "value_c",
        brush_domain: (0.0, 40.0),
        // Forty distinct brushed values: the cube stays O(bins × 40) at any
        // row count — the layer's intended shape.
        expect_cube: true,
        // Same shape as brush-density: one scatter, one aggregating mark.
        row_level_marks: 1,
    },
    SpecDef {
        name: "crossfilter-dots",
        template: SPEC_DOTS,
        drag: Drag::Brush,
        brush_column: "value_a",
        brush_domain: (0.0, 100.0),
        // Row-level marks: nothing to pre-aggregate; the layer must stay
        // silent.
        expect_cube: false,
        // BOTH plots are row-per-mark — the scene carries two primitives per
        // table row, so this scenario hits the encode cap a magnitude earlier
        // than the single-scatter ones.
        row_level_marks: 2,
    },
    SpecDef {
        name: "slider-drag",
        template: SPEC_SLIDER,
        // The gesture that had never been measured: one handle, fixed stops.
        drag: Drag::Slider,
        brush_column: "value_c",
        brush_domain: (0.0, 40.0),
        // Forty stops on the brushed axis and `intersect` resolution: both
        // views subscribe and both get a cube, bounded at O(bins × 40).
        expect_cube: true,
        // Both views aggregate, so the drawn picture stays O(bins) at every
        // magnitude — a drag frame here is the drag, not a raster, and no
        // encode cap applies.
        row_level_marks: 0,
    },
];

/// The most row-level primitives a frame suite may ask the renderer to encode
/// — summed across every row-per-mark mark in the scene. Above it the FRAME
/// suites are skipped; the engine suites still run at every magnitude.
///
/// This is not a wall-clock convenience. Since the compose began assembling
/// EVERY Arrow chunk rather than the first, a row-per-mark scene really does
/// carry one primitive per table row, and what the renderer can encode has a
/// ceiling.
///
/// **This harness's POLICY cap, not the ceiling.** The ceiling is
/// [`MEASURED_INKED_MAX`], which
/// `brightfield_render::sample_policy` owns and documents with the measurement
/// it came from; this sits under it with margin so a suite is declined rather
/// than timed at a count close enough to the bracket to be in doubt. Above the
/// ceiling a frame does not degrade — it comes back BLANK at exit 0, which is
/// what an unattended capture records as a pass.
///
/// **The previous value, 10⁶, described the wrong ceiling.** It was derived
/// from the storage-buffer binding size — the conservative 128 MiB the
/// renderer used to ask for, at ~128 bytes per drawn primitive. Two things are
/// now known about that. First, every device brightfield creates asks the
/// adapter for its real limits, and a Metal adapter reports 4 GiB, so that
/// ceiling moved by a factor of 32 and no longer binds. Second, and the reason
/// the number here went DOWN rather than up: a much lower ceiling was always
/// underneath it, and nothing was looking at it.
const FRAME_DRAWN_ROW_CAP: u64 = 100_000;

// The cap must stay UNDER the largest count measured to ink a frame: a cap at
// or above `MEASURED_BLANK_MIN` would let the harness time a frame already
// proven blank, which is the defect the cap exists to prevent. A compile-time
// fact, so the build checks it rather than a test that has to be run.
const _: () = assert!(FRAME_DRAWN_ROW_CAP < MEASURED_INKED_MAX);

/// Why a frame suite was declined, in the words a reader of the record sees.
///
/// The string this replaced said the primitive count "exceeds the
/// {FRAME_DRAWN_ROW_CAP} the renderer can encode in one scene buffer — the
/// frame does not render at all". Both halves are refuted by the measurement
/// documented on [`FRAME_DRAWN_ROW_CAP`]. The cap is this harness's POLICY
/// number, not what the renderer can encode — the renderer inks past it, up to
/// [`MEASURED_INKED_MAX`]. And "one scene buffer" pointed at the storage-buffer
/// binding size, which this adapter reports as 4 GiB and which does not bind;
/// what binds is vello's FIXED 2^21-element `seg_counts`, which no adapter
/// limit moves. A reader given the old sentence goes looking for a bigger GPU.
fn frames_skipped_reason(drawn_primitives: u64) -> String {
    format!(
        "{drawn_primitives} row-level primitives is past the MEASURED \
         drawn-primitive ceiling: {MEASURED_INKED_MAX} dots is the largest \
         count measured to ink a frame and {MEASURED_BLANK_MIN} the smallest \
         measured to come back BLANK — vello returns Ok, a frame is written, \
         and every pixel is transparent. The ceiling is vello's fixed \
         2^21-element seg_counts buffer, NOT a device limit: this adapter \
         reports a 4 GiB max storage buffer binding size and no adapter limit \
         moves seg_counts. The harness declines the suite at \
         {FRAME_DRAWN_ROW_CAP} rather than time a picture that is not there."
    )
}

/// The device-pixel scale frames render at — 2.0 matches the Retina-class
/// displays the live window actually runs on.
const FRAME_SCALE: f32 = 2.0;

/// Default discarded warm-up frames per frame suite.
const DEFAULT_WARMUP_FRAMES: usize = 5;
/// Default timed frames per frame suite. Chosen with
/// [`DEFAULT_WARMUP_FRAMES`] so the summed budget lands exactly on the drag
/// generators' distinct period — see [`validate_frame_steps`].
const DEFAULT_MEASURED_FRAMES: usize = 30;
/// `--quick` warm-up frames.
const QUICK_WARMUP_FRAMES: usize = 2;
/// `--quick` timed frames.
const QUICK_MEASURED_FRAMES: usize = 8;

#[derive(Debug)]
struct Args {
    rows: Vec<u64>,
    iterations: usize,
    warmup_frames: usize,
    measured_frames: usize,
    corpus_frames: usize,
    skip_frames: bool,
    skip_corpus: bool,
    out_dir: PathBuf,
    data_dir: PathBuf,
    label: Option<String>,
    /// Opt-in: a local copy of the published crosswalk parquet; when present
    /// the fixed-scale crosswalk scenario runs against it.
    crosswalk_parquet: Option<PathBuf>,
}

impl Args {
    fn parse(root: &Path) -> Result<Self, String> {
        let mut args = Self {
            rows: vec![10_000, 100_000, 1_000_000, 10_000_000],
            iterations: 20,
            warmup_frames: DEFAULT_WARMUP_FRAMES,
            measured_frames: DEFAULT_MEASURED_FRAMES,
            corpus_frames: 20,
            skip_frames: false,
            skip_corpus: false,
            out_dir: root.join("benchmarks/results"),
            data_dir: root.join("benchmarks/.data"),
            label: None,
            crosswalk_parquet: None,
        };
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
            match a.as_str() {
                "--rows" => {
                    args.rows = val("--rows")?
                        .split(',')
                        .map(|s| s.trim().parse::<u64>().map_err(|e| format!("--rows: {e}")))
                        .collect::<Result<_, _>>()?;
                }
                "--iterations" => {
                    args.iterations = val("--iterations")?
                        .parse()
                        .map_err(|e| format!("--iterations: {e}"))?
                }
                "--frames" => {
                    args.measured_frames = val("--frames")?
                        .parse()
                        .map_err(|e| format!("--frames: {e}"))?
                }
                "--warmup-frames" => {
                    args.warmup_frames = val("--warmup-frames")?
                        .parse()
                        .map_err(|e| format!("--warmup-frames: {e}"))?
                }
                "--out-dir" => args.out_dir = PathBuf::from(val("--out-dir")?),
                "--data-dir" => args.data_dir = PathBuf::from(val("--data-dir")?),
                "--label" => args.label = Some(val("--label")?),
                "--crosswalk-parquet" => {
                    args.crosswalk_parquet = Some(PathBuf::from(val("--crosswalk-parquet")?));
                }
                "--skip-frames" => args.skip_frames = true,
                "--skip-corpus" => args.skip_corpus = true,
                "--quick" => {
                    args.rows = vec![10_000, 100_000];
                    args.iterations = 5;
                    args.measured_frames = QUICK_MEASURED_FRAMES;
                    args.warmup_frames = QUICK_WARMUP_FRAMES;
                    args.corpus_frames = 6;
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        if args.rows.is_empty() || args.iterations == 0 || args.measured_frames == 0 {
            return Err("rows, iterations and frames must be non-zero".into());
        }
        validate_iterations(args.iterations)?;
        validate_frame_steps(args.warmup_frames, args.measured_frames)?;
        Ok(args)
    }
}

/// Reject an iteration count the brush generator cannot serve with distinct
/// intervals. Above [`scenario::DISTINCT_BRUSH_INTERVALS`], `brush_interval`
/// repeats a pair it has already yielded, and the engine's SQL cache
/// short-circuits the repeat — so the extra iterations would time the cache,
/// not the engine, silently making the measurement a lie rather than merely a
/// slow run. Failing loudly is the honest response; the operator can lower the
/// count or, if more samples are truly wanted, the generator's period is what
/// must grow first.
fn validate_iterations(iterations: usize) -> Result<(), String> {
    if iterations > scenario::DISTINCT_BRUSH_INTERVALS {
        return Err(format!(
            "--iterations {iterations} exceeds the {} distinct brush intervals the \
             harness generates; a larger count would re-issue an interval and time \
             the SQL cache instead of the engine",
            scenario::DISTINCT_BRUSH_INTERVALS
        ));
    }
    Ok(())
}

/// Reject a frame-suite step budget the drag generators cannot serve with
/// distinct steps.
///
/// The interaction frame suite indexes its drag step with the FRAME counter,
/// which runs over `warmup + measured` — a different budget from
/// `--iterations`, driven by different flags, and for a long time validated by
/// nothing. `--iterations` being capped did not protect it: the defaults
/// ([`DEFAULT_WARMUP_FRAMES`] + [`DEFAULT_MEASURED_FRAMES`]) land exactly on
/// the generators' period, so the first step past them wraps onto step 0 and
/// the frame times the engine's SQL cache instead of the engine — the failure
/// the methodology claims cannot happen. `--frames 31` was enough to cause it
/// silently.
///
/// Both generators share one period (asserted in the scenario module), so one
/// bound covers brush and slider suites alike.
fn validate_frame_steps(warmup: usize, measured: usize) -> Result<(), String> {
    let total = warmup.saturating_add(measured);
    if total > scenario::DISTINCT_BRUSH_INTERVALS {
        return Err(format!(
            "--warmup-frames {warmup} + --frames {measured} = {total} frame steps exceeds \
             the {} distinct drag steps the harness generates; the frame suite indexes its \
             step by the frame counter, so a larger budget would re-issue a step and time \
             the SQL cache instead of the engine",
            scenario::DISTINCT_BRUSH_INTERVALS
        ));
    }
    Ok(())
}

/// One scenario × row-count row of the baseline. The engine suites run twice
/// on identical code — pre-aggregation enabled (`engine`, the shipped
/// configuration) and disabled (`engine_direct`) — so the delta between the
/// two brush-step latencies is attributable to the layer alone.
#[derive(Debug, Serialize)]
struct ScalingResult {
    scenario: String,
    /// Which gesture drove this row's timed steps.
    drag: Drag,
    rows: u64,
    dataset: String,
    /// The shipped configuration: automatic pre-aggregation enabled.
    engine: EngineMeasurement,
    /// The same suites with the layer disabled — the direct-query control.
    engine_direct: EngineMeasurement,
    #[serde(skip_serializing_if = "Option::is_none")]
    frames: Option<frames::FrameMeasurement>,
    /// Why this row has no frame cells, when it has none. A blank cell that
    /// does not say why reads as "fast" to somebody skimming; this row's
    /// frames were not slow, they were not produced.
    #[serde(skip_serializing_if = "Option::is_none")]
    frames_skipped: Option<String>,
    /// What this row's composed picture put on the target, read back from the
    /// renderer before any frame was timed. `None` where no frame suite was
    /// attempted, so there was nothing to prove.
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_ink: Option<frame_ink::FrameInk>,
    /// This row's picture was rendered and came back EMPTY — a per-cell
    /// failure, and a different event from [`Self::frames_skipped`]. A skip
    /// declined to measure; this measured and found nothing, which is what
    /// would otherwise have been published as a fast frame. Recording it does
    /// not fail the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    frames_blank: Option<String>,
}

/// One shipped example's steady-state frame time.
#[derive(Debug, Serialize)]
struct CorpusResult {
    example: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_steady: Option<Stats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skipped: Option<String>,
}

/// The complete baseline record — everything a later re-measurement compares
/// against, and everything a reader needs to know what the numbers mean.
#[derive(Debug, Serialize)]
struct BaselineReport {
    schema: &'static str,
    machine: MachineProfile,
    config: RunConfig,
    methodology: Vec<String>,
    scaling: Vec<ScalingResult>,
    corpus: Vec<CorpusResult>,
}

#[derive(Debug, Serialize)]
struct RunConfig {
    rows: Vec<u64>,
    iterations: usize,
    warmup_frames: usize,
    measured_frames: usize,
    corpus_frames: usize,
    frame_scale: f32,
}

/// The record's schema id.
///
/// v2 → v3 is a field migration, not a re-shape: v2 carried `first_batch_rows`
/// beside `materialised_rows`, describing a presentation that drew a mark's
/// FIRST Arrow chunk only. That presentation is gone — the compose assembles
/// every chunk — so the field is now `drawn_rows` and names what it holds. It
/// stayed labelled v2 for one commit after the behaviour changed, which means
/// two different field sets shipped under one version; the bump is what makes
/// a v2 record un-mistakable for a v3 one. v3 also adds `drag` (which gesture
/// produced a row's steps), per-mark chunk/byte shape, and `compose_memory`.
///
/// v3 → v4 adds `frame_ink` and `frames_blank`. A v3 frame cell states a time
/// and nothing about what was on screen while the clock ran: the harness
/// decided a picture existed from a primitive count computed before rendering,
/// and that arithmetic cannot see an empty frame. A v4 frame cell states a time
/// beside the readback that proved there was a picture to time.
const SCHEMA: &str = "brightfield-bench/v4";

/// Every statement a record carries about how it was measured.
///
/// A function rather than a table of literals because one of these sentences
/// states the measured drawn-primitive ceiling, and that figure has exactly one
/// home — `brightfield_render::sample_policy`. Written out here it would be a
/// copy, and a copy is a thing that can be true on the day it is typed and
/// false afterwards.
fn methodology() -> Vec<String> {
    vec![
        "Interaction latency is measured at the coordinator seam the live window blocks its frame on: one committed brush step = Coordinator::apply (predicate push-down into DuckDB + re-query of every affected mark). live_apply adds the re-composite into a Vello scene (LiveDashboard::apply), which is the full in-frame cost of a brush step in the live window.".to_string(),
        "Every timed drag step uses a distinct interval: the engine caches repeated identical SQL, so a repeated interval would time the cache. The harness has TWO step budgets and bounds BOTH against the generators' 35-step period — the engine suites' --iterations, and the frame suites' --warmup-frames + --frames (the interaction frame suite indexes its step by the frame counter, so it wraps independently of --iterations; validating only the latter left `--frames 31` free to re-issue step 0 silently). A non-vacuity check requires the drag to have actually reduced the cross-filtered step's row count, and every apply must affect at least one mark.".to_string(),
        "Frame times are headless: the real MeridianApp drawn by egui's real wgpu backend into an offscreen texture, timed per frame through GPU completion (submit + blocking wait). No swapchain, no present, no vsync — the number is the cost of producing a frame, not displaying one. Warm-up frames are discarded.".to_string(),
        "steady frames draw with nothing changing (the shell's floor). interaction frames each push one committed brush step through the live document before drawing, so they carry re-query + re-composite + canvas re-raster + GPU wait.".to_string(),
        "The composed scene draws EVERY materialised Arrow chunk: a mark's result batches are assembled into one drawable batch (assemble_batches), the same path the presentation layer uses. drawn_rows vs materialised_rows is the cross-check — they are equal, so the drawn picture holds every row the query answered (the raw-dot scenario spans many ~2048-row chunks and still draws them all). A future regression that reintroduced a first-chunk cap would show drawn_rows < materialised_rows here; an assembly that could not proceed fails the run loudly by name rather than reporting a smaller drawn count.".to_string(),
        "cold open = Coordinator::load (DDL, no mark queries) then the first full materialisation of every mark, on a session in the same process; the Parquet file is warm in the OS page cache.".to_string(),
        format!(
            "Datasets are deterministic pure functions of the row index via DuckDB hash() — no RNG. \
             Frame suites are capped by DRAWN row-level primitives, not by table rows: a scenario's \
             row-per-mark marks each contribute one primitive per materialised row, an aggregating \
             mark contributes none, and past the drawn-primitive ceiling the frame comes back BLANK. \
             THE CEILING IS MEASURED, NOT INHERITED, and it is not the cap: on the reference machine \
             through the production render path, {MEASURED_INKED_MAX} dots is the largest count that \
             inked a frame and {MEASURED_BLANK_MIN} the smallest that came back with every pixel \
             rgba(0,0,0,0). It is vello's FIXED 2^21-element seg_counts buffer and NOT a device limit \
             — this adapter reports a 4 GiB max storage buffer binding size, and no adapter limit \
             moves seg_counts. Vello sets a failed bit in a GPU-side counter, does not re-run coarse \
             and returns Ok, so nothing in the render path raises an error and a blank frame is \
             indistinguishable from a fast one by timing alone; the harness therefore declines the \
             suite at a policy cap of {FRAME_DRAWN_ROW_CAP}, under the measured ceiling with margin, \
             rather than time a picture that is not there. A second and exact ceiling sits an order \
             of magnitude above: bin_data is 2^18 elements and the encode panics at \
             {VELLO_BIN_DATA_PANIC_DOTS} dots in that fixture. Engine suites run at every magnitude \
             regardless. This cap became load-bearing when the compose began assembling every Arrow \
             chunk instead of the first: a record measured before that change reports frame times \
             for scenes that were drawing ~2048 rows per mark, whatever its row column says."
        ),
        "The emitted SQL applies a selection predicate INSIDE an aggregating mark's query — it filters the base rows that get aggregated (row-level marks are wrapped whole). The aggregating scenarios keep their original brush-the-binned-column shape so the measured series stays comparable across harness runs.".to_string(),
        "Each scenario's engine suites run twice on identical code: automatic pre-aggregation enabled (the shipped configuration) and disabled (the direct-query control). The delta between the two brush-step latencies is the layer's contribution. Cube engagement is verified per run — engaged and serving where the scenario expects it, silent where it does not — and a run whose cube behaviour contradicts the expectation FAILS instead of reporting.".to_string(),
        "Active interval dimensions enter a cube at RAW data values in this first cut (answer-exactness over cube size). A cube over a ~unique-per-row brushed column (brush-density's value_a) therefore approaches the base table's size and buys little; the bounded-cardinality scenario (brush-binned-density, forty distinct brushed values) and the crosswalk scenario measure the shape the layer is built for. Frame suites run in the shipped configuration only.".to_string(),
        "THE SETTLED-ZOOM NUMBERS DO NOT GENERALISE ACROSS COLUMN CARDINALITY, and the committed scenarios are the favourable case. The raw-values rule above applies to a navigated column exactly as it does to a brushed one, but bites harder: on navigation the active column is usually the same column being binned, so the cube becomes GROUP BY bin(x), x. Measured on a densityX over 20,000 distinct doubles, the cube materialises 20,000 rows against a 20,000-row source — no reduction at all, plus the build cost, and every later zoom scans a table the size of the base. Answers stay exact; only the speed collapses. Every navigated column in the committed scenarios is low-cardinality, so this shape is absent from the recorded figures and must not be read out of them.".to_string(),
        "The crosswalk scenario is opt-in (--crosswalk-parquet) and fixed-scale: it measures the published company-identifier crosswalk dataset as-is; the harness records the file's row count rather than generating data.".to_string(),
        "In the enabled run, the FIRST brush step carries the one-time cube build (a full-table aggregation); it surfaces in the max percentile, while p50 reflects the steady per-step serve cost. Cubes are session-scoped and never persist.".to_string(),
        "Each row records which GESTURE drove its timed steps (`drag`). A brush moves BOTH interval endpoints as a rectangle is dragged inside a chart; a slider pins its lower end and advances one handle across fixed stops. They are not interchangeable: the slider's steps are monotone and land exactly on the brushed axis's forty stops (a step falling between two stops would emit different SQL while selecting identical rows, reporting a re-query that did no new work as though it had).".to_string(),
        "The slider-drag scenario drives a SELECTION, not a scalar param. Upstream Mosaic expresses a range slider as `select: interval`, and this engine's pre-aggregation layer keys its cubes off that structured clause — the only path reaching it is selection propagation. A slider wired to a scalar param arrives at the query layer as a substituted expression predicate that decomposes into no cube at all, so measuring it would measure a different mechanism under the slider's name. The spec vocabulary has no slider-to-selection widget form yet, so the contributor is declared as the interval interactor it does have and the harness drives it with slider-shaped steps; no row in this record took the scalar-param path.".to_string(),
        "slider-drag resolves its selection as `intersect`, not `crossfilter`. A slider is not a view and has no picture of its own to spare, so EVERY subscriber is filtered, including the plot that declares the contributor — both views re-query on every step. A crossfilter brush exempts its own plot, so a brush row's per-step work is one view lighter than a slider row's at the same row count; compare the two knowing that.".to_string(),
        "compose_memory is what the client held while the first full scene was composed — the window around LiveDashboard::present, which executes every mark and hands the whole result set to compose_from_results, which assembles every chunk of every mark and holds them all while it builds the scene. The window opens only after the coordinator-seam phase is gone: its session is dropped and its complete result set is moved into the shape pass, which consumes and drops it. That ordering is the measurement — resident size is a whole-process quantity, so a phase still holding its Arrow would be reported as this compose's cost. Two figures, because neither alone is honest. arrow_chunks_mib / arrow_assembled_mib are EXACT and deterministic, summed from the batches themselves (RecordBatch::get_array_memory_size) — the data the compose holds, identical on any host. A single-chunk mark assembles by pass-through, so its assembled bytes are the SAME allocation counted again, not a second copy.".to_string(),
        "rss_*_mib is the process's resident set size, polled from the OS across that same window (Linux /proc/self/status; macOS `ps -o rss=`, ~5ms resolution, so `rss_samples` states how coarse a given cell is). It is the only figure that sees the Arrow buffers' real cost: the chunks are imported from DuckDB over the C data interface, so DuckDB's C++ allocator owns them and a Rust allocator counter would not see them. It is also a SINGLE sample of a whole-process quantity and is NOT reproducible on its own. Every scenario records two windows — pre-aggregation off and on — over the same compose work (the first present precedes any interaction, so the toggle cannot change it); their disagreement is this figure's run-to-run noise and the generated summary states the widest one observed. Quote a peak only with that spread beside it; quote arrow_chunks_mib when a deterministic number is wanted. The pre-compose floor is not monotone: the OS reclaims pages between windows, so rss_before falls as often as it rises and rss_growth is not the compose's cost. The poller stops before the timed applies, so it cannot perturb a reported latency.".to_string(),
        "A cell's picture is rendered and read back BEFORE the cell is timed. The cell's spec is composed through the production pipeline, rendered once through VelloRenderer at the frame scale, and the target is read back and counted: frame_ink records the device size, how many of those pixels differ from the colour the render cleared to, whether the target came back holding one repeated value, and the verdict. A cell whose picture reads back empty publishes NO timing. It records frames_blank instead — a per-cell FAILURE, a different event from frames_skipped, which declined to measure at all — and the run continues and exits 0. The drawn-primitive cap still declines the suites it declines; what changed is that a cell UNDER the cap is no longer assumed to have drawn.".to_string(),
        "The ink probe is a SEPARATE submission from the frames the suite times, on the same composed scene through the same renderer at the same device scale. The timed frames go through the shell's egui path, which does not read back, by design — a readback there would time a cost the live window does not pay. So the probe answers whether this cell's picture can be produced at all, immediately before the cell is timed; it does not answer whether one particular timed submission drew. A defect that made a picture appear and then vanish between the probe and the timing is outside what this evidence covers. So is a THINNED picture: the overflow this detects emits nothing at all, so the two verdicts are a picture and no picture. inked_fraction is a share of the whole target, and a composed dashboard paints its page tone across that target before a mark is drawn — a row that rendered therefore reports a high share, and the figure is not mark coverage.".to_string(),
        "Record schema v4. v2 carried `first_batch_rows` beside `materialised_rows` — the pre-assembly presentation that drew a mark's FIRST Arrow chunk only. That field is gone; `drawn_rows` names what the field now holds. A v2 record's row counts describe code that no longer ships and must not be quoted against a v3 one. v3 → v4 adds frame_ink and frames_blank: a v3 frame cell states a time and nothing about what was on screen while the clock ran, because the harness of that era decided a picture existed from a primitive count computed before rendering.".to_string(),
    ]
}

fn main() -> ExitCode {
    // The repo root: compiled in, so the harness runs correctly from any CWD.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels under the repo root")
        .to_path_buf();

    let args = match Args::parse(&root) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: brightfield-bench [--rows N,N,..] [--iterations N] [--frames N] \
                 [--warmup-frames N] [--skip-frames] [--skip-corpus] [--quick] \
                 [--out-dir D] [--data-dir D] [--label NAME] \
                 [--crosswalk-parquet FILE]"
            );
            return ExitCode::from(2);
        }
    };

    match run(&root, &args) {
        Ok(paths) => {
            for p in paths {
                println!("wrote {}", p.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn run(root: &Path, args: &Args) -> Result<Vec<PathBuf>, String> {
    if cfg!(debug_assertions) {
        eprintln!(
            "warning: measuring a DEBUG build — the baseline profile is release \
             (cargo run --release -p brightfield-bench)"
        );
    }

    let gen_conn = duckdb::Connection::open_in_memory().map_err(|e| format!("duckdb: {e}"))?;
    let machine = MachineProfile::collect(data::duckdb_version(&gen_conn));
    eprintln!(
        "machine: {} | {} | {}",
        machine.cpu, machine.os, machine.gpu_adapter
    );

    let mut scaling = Vec::new();
    for &rows in &args.rows {
        let dataset = data::ensure_dataset(&gen_conn, &args.data_dir, rows)?;
        for def in SCENARIOS {
            let drawn_primitives = def.row_level_marks * rows;
            let frames_capped = drawn_primitives > FRAME_DRAWN_ROW_CAP;
            eprintln!("scenario {} @ {rows} rows ...", def.name);
            let spec_text = def.template.replace(
                "__DATA_PARQUET__",
                dataset.to_str().ok_or("dataset path is not UTF-8")?,
            );
            let sc = Scenario {
                name: def.name.to_string(),
                spec_text,
                drag: def.drag,
                brush_column: def.brush_column,
                brush_domain: def.brush_domain,
                expect_cube: def.expect_cube,
            };

            // The A/B pair: direct control first, then the shipped
            // configuration — identical code, the toggle is the only change.
            let engine_direct = scenario::run_engine_suites(&sc, None, args.iterations, false)?;
            let engine = scenario::run_engine_suites(&sc, None, args.iterations, true)?;

            let frames_skipped = if args.skip_frames {
                Some("--skip-frames".to_string())
            } else if frames_capped {
                Some(frames_skipped_reason(drawn_primitives))
            } else {
                None
            };
            // The readback stands between the decision to time this cell and
            // the timing itself: compose the picture, render it once, ask what
            // reached the target. A cell that came back empty is timed by
            // nothing, because a blank frame is a fast frame on the clock.
            let mut ink = None;
            let mut frames_blank = None;
            let frames = if frames_skipped.is_some() {
                None
            } else {
                // The spec must exist as a file for the shell's boot path and
                // for the ink probe's compose.
                let spec_path = args.data_dir.join(format!("{}_{rows}.yaml", def.name));
                std::fs::write(&spec_path, &sc.spec_text)
                    .map_err(|e| format!("write {}: {e}", spec_path.display()))?;
                let probe = frame_ink::probe(&spec_path, FRAME_SCALE)?;
                let drew = probe.drew_ink;
                eprintln!(
                    "  picture: {}/{} device pixels inked",
                    probe.inked_pixels, probe.total_pixels
                );
                if !drew {
                    let why = frame_ink::blank_reason(&probe);
                    eprintln!("  {} @ {rows} rows: {why}", def.name);
                    frames_blank = Some(why);
                }
                ink = Some(probe);
                if !drew {
                    None
                } else {
                    let parsed =
                        brightfield_spec::parse_spec(&sc.spec_text, brightfield_spec::Format::Yaml)
                            .map_err(|e| format!("{}: parse: {e}", def.name))?;
                    let bindings =
                        brightfield_spec::analysis::build_brushable_bindings(&parsed.spec);
                    let b = bindings.first().ok_or("no brushable binding")?;
                    let steady = frames::frames_steady(
                        &spec_path,
                        FRAME_SCALE,
                        args.warmup_frames,
                        args.measured_frames,
                    )?;
                    let interaction = frames::frames_interaction(
                        &spec_path,
                        def.drag,
                        def.brush_column,
                        def.brush_domain,
                        &b.selection,
                        &b.parent_plot,
                        FRAME_SCALE,
                        args.warmup_frames,
                        args.measured_frames,
                    )?;
                    Some(frames::FrameMeasurement {
                        steady,
                        interaction: Some(interaction),
                    })
                }
            };

            scaling.push(ScalingResult {
                scenario: def.name.to_string(),
                drag: def.drag,
                rows,
                dataset: dataset
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                engine,
                engine_direct,
                frames,
                frames_skipped,
                frame_ink: ink,
                frames_blank,
            });
        }
    }

    // The fixed-scale crosswalk scenario, when a local copy was supplied.
    if let Some(parquet) = &args.crosswalk_parquet {
        let parquet_str = parquet.to_str().ok_or("crosswalk path is not UTF-8")?;
        let rows: u64 = gen_conn
            .query_row(
                &format!("SELECT COUNT(*) FROM read_parquet('{parquet_str}')"),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| format!("crosswalk row count: {e}"))?
            .try_into()
            .map_err(|_| "crosswalk row count is negative".to_string())?;
        eprintln!("scenario crosswalk-confidence @ {rows} rows (fixed) ...");
        let sc = Scenario {
            name: "crosswalk-confidence".to_string(),
            spec_text: SPEC_CROSSWALK.replace("__DATA_PARQUET__", parquet_str),
            drag: Drag::Brush,
            brush_column: "confidence",
            // Slightly wider than the data's [0.8, 1.0] so the drag's interval
            // endpoints sweep ACROSS the crosswalk's few distinct confidence
            // tiers — successive steps select varying non-empty subsets
            // rather than always all-or-nothing.
            brush_domain: (0.75, 1.05),
            expect_cube: true,
        };
        let engine_direct = scenario::run_engine_suites(&sc, None, args.iterations, false)?;
        let engine = scenario::run_engine_suites(&sc, None, args.iterations, true)?;
        scaling.push(ScalingResult {
            scenario: sc.name.clone(),
            drag: sc.drag,
            rows,
            dataset: parquet
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
            engine,
            engine_direct,
            frames: None,
            frames_skipped: Some(
                "fixed-scale scenario: engine suites only, no frame suites".to_string(),
            ),
            // No frame suite was attempted, so nothing was rendered to read
            // back and there is no picture to judge.
            frame_ink: None,
            frames_blank: None,
        });
    }

    let mut corpus = Vec::new();
    if !args.skip_frames && !args.skip_corpus {
        let examples = root.join("examples");
        let mut names: Vec<_> = std::fs::read_dir(&examples)
            .map_err(|e| format!("read {}: {e}", examples.display()))?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "yaml"))
            .map(|e| e.path())
            .collect();
        names.sort();
        for path in names {
            let example = path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            eprintln!("corpus {example} ...");
            match frames::frames_steady(&path, FRAME_SCALE, args.warmup_frames, args.corpus_frames)
            {
                Ok(stats) => corpus.push(CorpusResult {
                    example,
                    frame_steady: Some(stats),
                    skipped: None,
                }),
                Err(e) => corpus.push(CorpusResult {
                    example,
                    frame_steady: None,
                    skipped: Some(e),
                }),
            }
        }
    }

    let report = BaselineReport {
        schema: SCHEMA,
        machine,
        config: RunConfig {
            rows: args.rows.clone(),
            iterations: args.iterations,
            warmup_frames: args.warmup_frames,
            measured_frames: args.measured_frames,
            corpus_frames: args.corpus_frames,
            frame_scale: FRAME_SCALE,
        },
        methodology: methodology(),
        scaling,
        corpus,
    };

    std::fs::create_dir_all(&args.out_dir)
        .map_err(|e| format!("create {}: {e}", args.out_dir.display()))?;
    let date = report
        .machine
        .captured_at
        .split('T')
        .next()
        .unwrap_or("undated")
        .to_string();
    let slug = args
        .label
        .clone()
        .unwrap_or_else(|| slugify(&report.machine.cpu));
    let base = args.out_dir.join(format!("{date}-{slug}"));

    let json_path = base.with_extension("json");
    let json = serde_json::to_string_pretty(&report).map_err(|e| format!("serialise: {e}"))?;
    std::fs::write(&json_path, json + "\n").map_err(|e| format!("write json: {e}"))?;

    let md_path = base.with_extension("md");
    std::fs::write(&md_path, render_markdown(&report))
        .map_err(|e| format!("write markdown: {e}"))?;

    Ok(vec![json_path, md_path])
}

fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn fmt_stats(s: &Stats) -> String {
    format!("{:.1} / {:.1} / {:.1}", s.p50_ms, s.p95_ms, s.max_ms)
}

/// The widest disagreement between a row's two compose-memory windows, and the
/// row it came from.
///
/// The two windows wrap the SAME compose work — the first present happens
/// before any interaction, so the pre-aggregation toggle cannot change it — so
/// their ratio is not a finding about the layer. It is the reproducibility of a
/// single whole-process RSS sample, measured by the record on itself. The
/// generated summary states it beside the column so no peak is quoted as
/// though it were stable.
fn worst_compose_rss_spread(r: &BaselineReport) -> Option<(f64, &ScalingResult)> {
    r.scaling
        .iter()
        .filter_map(|s| {
            let a = s.engine_direct.compose_memory.rss_peak_mib?;
            let b = s.engine.compose_memory.rss_peak_mib?;
            let (lo, hi) = if a < b { (a, b) } else { (b, a) };
            if lo > 0.0 {
                Some((hi / lo, s))
            } else {
                None
            }
        })
        .max_by(|x, y| x.0.total_cmp(&y.0))
}

/// The gesture name a report row carries.
fn drag_label(d: Drag) -> &'static str {
    match d {
        Drag::Brush => "brush",
        Drag::Slider => "slider",
    }
}

/// The human-readable face of the JSON record — same data, no extra claims.
fn render_markdown(r: &BaselineReport) -> String {
    use std::fmt::Write as _;
    let mut md = String::new();
    let m = &r.machine;
    let _ = writeln!(
        md,
        "# Performance baseline — {}",
        m.captured_at.split('T').next().unwrap_or("")
    );
    let _ = writeln!(md);
    let _ = writeln!(md, "Measured on this repository at commit `{}`.", m.commit);
    let _ = writeln!(md);
    let _ = writeln!(md, "| Machine | |");
    let _ = writeln!(md, "|---|---|");
    let _ = writeln!(md, "| CPU | {} ({} logical) |", m.cpu, m.logical_cpus);
    let _ = writeln!(md, "| Memory | {} GiB |", m.memory_gib);
    let _ = writeln!(md, "| OS | {} |", m.os);
    let _ = writeln!(md, "| GPU | {} |", m.gpu_adapter);
    let _ = writeln!(md, "| Toolchain | {} ({}) |", m.rustc, m.build_profile);
    let _ = writeln!(md, "| DuckDB | {} |", m.duckdb);
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "Latency cells are `p50 / p95 / max` in milliseconds over {} timed drag steps \
         (each a distinct interval). Frame cells are `p50 / p95 / max` in milliseconds \
         over {} timed frames at {}x scale, after {} discarded warm-up frames. \
         Full definitions: the `methodology` block in the JSON record beside this file.",
        r.config.iterations, r.config.measured_frames, r.config.frame_scale, r.config.warmup_frames
    );
    let _ = writeln!(md);
    let _ = writeln!(md, "## Interaction latency and frame time vs row count");
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "`direct` disables the automatic pre-aggregation layer; `cubed` is the \
         shipped configuration. Identical code either side of the toggle — the \
         delta is the layer. `cube` shows what the enabled run's layer did: \
         cubes built / **mark re-queries** served from a cube. The second \
         number is not a step count: the engine counts one hit per mark it \
         serves, so a scenario filtering two subscribing marks records two hits \
         per drag step and a twenty-step suite reads `2/40`. `Drag` is the gesture the \
         timed steps imitate: a **brush** moves both interval endpoints inside a \
         chart and exempts its own plot (crossfilter); a **slider** pins its \
         lower end, advances one handle across fixed stops, and filters every \
         view including the contributor's (intersect) — so a slider step \
         re-queries one more view than a brush step at the same row count. \
         **`Settled zoom → data`** is one pan/zoom extent applied to the \
         aggregating plot and every mark of that plot re-queried — the cost a \
         gesture pays ONCE, when it stops. It is printed for both \
         configurations, because the engine now keys a cube off a navigation \
         extent as well as off a selection clause: an extent IS a structured \
         interval clause, and the axes of one that push beneath the `GROUP BY` \
         become the cube's active dimensions. Where the shape yields a cube, \
         read the cubed figure beside `Step → data, cubed` — the two gestures \
         are then the same kind of query over the same pre-aggregate. Where it \
         does not (a row-level mark, or an axis naming an aggregate output), \
         the two zoom columns should agree."
    );
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "| Scenario | Drag | Rows | Cold open (load + first query, ms) | \
         Step → data, direct (ms) | Step → data, cubed (ms) | \
         Settled zoom → data, direct (ms) | Settled zoom → data, cubed (ms) | \
         Step → scene, direct (ms) | Step → scene, cubed (ms) | Cube | \
         Steady frame (ms) | Interaction frame (ms) | Drawn/materialised rows | \
         Arrow held (MiB) | Compose peak RSS, direct / cubed (MiB) | Picture |"
    );
    let _ = writeln!(
        md,
        "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
    );
    for s in &r.scaling {
        let drawn = s
            .engine
            .marks
            .iter()
            .map(|mk| format!("{}/{}", mk.drawn_rows, mk.materialised_rows))
            .collect::<Vec<_>>()
            .join(" · ");
        // BOTH configurations' windows, because they measure the SAME compose
        // work (the first present precedes any interaction, so no cube exists
        // yet) — their disagreement is this figure's run-to-run noise, and
        // printing one of them alone hides it.
        let peak = |m: &crate::mem::ComposeMemory| {
            m.rss_peak_mib.map_or("—".into(), |p| format!("{p:.0}"))
        };
        // What the renderer said this cell's picture held, in the row beside
        // the times it produced. A frame column and a BLANK picture together
        // are a contradiction a reader must be able to see without opening the
        // JSON — which is the whole point of measuring it.
        let picture = s.frame_ink.as_ref().map_or("—".into(), |ink| {
            if ink.drew_ink {
                format!("{:.0}% inked", ink.inked_fraction * 100.0)
            } else {
                "**BLANK**".to_string()
            }
        });
        let _ = writeln!(
            md,
            "| {} | {} | {} | {:.1} + {:.1} | {} | {} | {} | {} | {} | {} | {}/{} | {} | {} | {} | {:.1} | {} / {} | {} |",
            s.scenario,
            drag_label(s.drag),
            s.rows,
            s.engine.load_ms,
            s.engine.first_materialise_ms,
            fmt_stats(&s.engine_direct.coordinator_apply),
            fmt_stats(&s.engine.coordinator_apply),
            fmt_stats(&s.engine_direct.navigation_apply),
            fmt_stats(&s.engine.navigation_apply),
            fmt_stats(&s.engine_direct.live_apply),
            fmt_stats(&s.engine.live_apply),
            s.engine.preagg.cubes_built,
            s.engine.preagg.cube_hits,
            s.frames
                .as_ref()
                .map_or("—".into(), |f| fmt_stats(&f.steady)),
            s.frames
                .as_ref()
                .and_then(|f| f.interaction.as_ref())
                .map_or("—".into(), fmt_stats),
            drawn,
            s.engine_direct.compose_memory.arrow_chunks_mib,
            peak(&s.engine_direct.compose_memory),
            peak(&s.engine.compose_memory),
            picture,
        );
    }
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "**`Arrow held` is the figure to quote.** It is exact and \
         deterministic — the summed size of every materialised chunk the \
         compose holds at once, read from the batches themselves — so it is the \
         same on any host and reproduces run to run."
    );
    let _ = writeln!(md);
    let spread = worst_compose_rss_spread(r);
    let _ = writeln!(
        md,
        "`Compose peak RSS` is the whole process's high-water mark across the \
         first full compose — every mark executed, every Arrow chunk assembled, \
         the scene built — sampled from the OS ({sampler}). Both configurations' \
         windows are shown because they measure the SAME compose work: the first \
         present precedes any interaction, so no cube exists yet either side of \
         the toggle. Each cell is therefore a SINGLE sample of a whole-process \
         quantity, and the gap between the pair is this measurement's \
         run-to-run noise — {spread_note} **Do not quote a peak without that \
         spread.** The pre-compose floor is not a monotone baseline either: the \
         OS reclaims pages between windows, so it falls as often as it rises and \
         the growth over it is not the compose's cost. `rss_before_mib`, \
         `rss_peak_mib`, `rss_growth_mib` and `rss_samples` are in the JSON per \
         configuration for anyone who wants them.",
        sampler = r
            .scaling
            .first()
            .map_or("none", |s| s.engine_direct.compose_memory.sampler),
        spread_note = spread.map_or_else(
            || "no row recorded both windows, so it cannot be stated here.".to_string(),
            |(ratio, s)| format!(
                "across the rows above it reaches {ratio:.1}x ({} @ {} rows).",
                s.scenario, s.rows
            )
        ),
    );
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "**`Picture` is read back from the renderer, not inferred.** Before a \
         row's frame suites run, its spec is composed and rendered once at the \
         frame scale and the target is counted: the cell states what share of \
         those device pixels carry anything the clear did not put there. \
         `BLANK` means the render returned success and the target came back \
         empty — vello can overflow one of its fixed buffers, emit nothing and \
         report `Ok`, and by the clock alone that is a very fast frame. A row \
         whose picture is `BLANK` carries no frame timing, and it is a \
         **failure**, not a skip: a skip declined to measure, this measured and \
         found nothing. `—` means no frame suite was attempted for that row, so \
         nothing was rendered to judge. The share is of the whole target and a \
         composed dashboard paints its page tone across that target before a \
         mark is drawn, so a row that rendered reports a high share: the figure \
         separates a picture from no picture, and it is not mark coverage."
    );
    let _ = writeln!(md);
    let blank: Vec<&ScalingResult> = r
        .scaling
        .iter()
        .filter(|s| s.frames_blank.is_some())
        .collect();
    if !blank.is_empty() {
        let _ = writeln!(md, "### Rows whose picture came back empty");
        let _ = writeln!(md);
        let _ = writeln!(
            md,
            "These rows were rendered and produced nothing. They are recorded \
             as failures and no timing is published for them; the run they came \
             from still completed."
        );
        let _ = writeln!(md);
        for s in blank {
            let _ = writeln!(
                md,
                "- **{} @ {} rows** — {}",
                s.scenario,
                s.rows,
                s.frames_blank.as_deref().unwrap_or("")
            );
        }
        let _ = writeln!(md);
    }
    let skipped: Vec<&ScalingResult> = r
        .scaling
        .iter()
        .filter(|s| s.frames.is_none() && s.frames_skipped.is_some())
        .collect();
    if !skipped.is_empty() {
        let _ = writeln!(md, "### Rows with no frame cells, and why");
        let _ = writeln!(md);
        let _ = writeln!(
            md,
            "A blank frame cell is not a fast frame. These rows produced no \
             frame at all:"
        );
        let _ = writeln!(md);
        for s in skipped {
            let _ = writeln!(
                md,
                "- **{} @ {} rows** — {}",
                s.scenario,
                s.rows,
                s.frames_skipped.as_deref().unwrap_or("")
            );
        }
        let _ = writeln!(md);
    }
    if !r.corpus.is_empty() {
        let _ = writeln!(md, "## Example corpus — steady-state frame time");
        let _ = writeln!(md);
        let _ = writeln!(md, "| Example | Frame p50 / p95 / max (ms) |");
        let _ = writeln!(md, "|---|---:|");
        for c in &r.corpus {
            match (&c.frame_steady, &c.skipped) {
                (Some(s), _) => {
                    let _ = writeln!(md, "| {} | {} |", c.example, fmt_stats(s));
                }
                (None, Some(why)) => {
                    let _ = writeln!(md, "| {} | skipped: {} |", c.example, why);
                }
                _ => {}
            }
        }
        let _ = writeln!(md);
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_filename_safe() {
        assert_eq!(slugify("Apple M3 Pro"), "apple-m3-pro");
        assert_eq!(slugify("Intel(R) Core(TM) i7"), "intel-r-core-tm-i7");
    }

    #[test]
    fn scenario_templates_carry_the_substitution_token() {
        assert!(SPEC_DENSITY.contains("__DATA_PARQUET__"));
        assert!(SPEC_BINNED.contains("__DATA_PARQUET__"));
        assert!(SPEC_DOTS.contains("__DATA_PARQUET__"));
        assert!(SPEC_SLIDER.contains("__DATA_PARQUET__"));
        assert!(SPEC_CROSSWALK.contains("__DATA_PARQUET__"));
    }

    /// The slider scenario's whole reason to exist is that it drives a
    /// selection. A spec edit that swapped the interval contributor for a
    /// scalar param would still run, still produce numbers, and quietly
    /// measure a path with no cube — so the shape is asserted, not assumed.
    #[test]
    fn the_slider_scenario_drives_a_selection_not_a_scalar_param() {
        let def = SCENARIOS
            .iter()
            .find(|d| d.name == "slider-drag")
            .expect("the slider scenario is committed");
        assert_eq!(def.drag, Drag::Slider, "driven with slider-shaped steps");
        assert!(
            SPEC_SLIDER.contains("select: intersect"),
            "a slider filters every view — crossfilter would exempt its own plot"
        );
        assert!(
            SPEC_SLIDER.contains("select: intervalX"),
            "the contributor must be an interval interactor, so the clause \
             reaches the engine as a structured Interval the cube can key off"
        );
        assert!(
            !SPEC_SLIDER.contains("input: slider"),
            "an input widget writes a SCALAR param, which reaches the IR as a \
             substituted expression predicate and decomposes into no cube"
        );
        assert!(
            def.expect_cube,
            "forty stops on the brushed axis: the cube must engage, and a run \
             where it did not must fail rather than report"
        );
    }

    /// A slider row and a brush row are only comparable when the reader knows
    /// which is which, so every committed scenario names its gesture.
    #[test]
    fn every_scenario_names_its_gesture() {
        assert!(SCENARIOS.iter().any(|d| d.drag == Drag::Slider));
        assert!(SCENARIOS.iter().any(|d| d.drag == Drag::Brush));
        for d in SCENARIOS {
            assert!(!drag_label(d.drag).is_empty(), "{} has a label", d.name);
        }
    }

    /// The frame cap is a MEASURED boundary on this machine, not a taste
    /// call — and the measurement it reproduces is the one that looks at the
    /// written PNG, not the one that waits for the process to abort.
    ///
    /// The old expectations asserted here said a one-scatter scene renders at
    /// 10⁶ rows. It does return `Ok` and it does write a file. The file is
    /// **blank**: past [`MEASURED_BLANK_MIN`] drawn primitives vello's fixed
    /// `seg_counts` buffer overflows, coarse emits nothing, and every pixel comes back
    /// transparent. A cap that only caught the abort was letting a whole
    /// magnitude of empty frames through and timing them.
    ///
    /// The per-scenario primitive counts are what put each row on the right
    /// side of the line, so they are asserted rather than trusted.
    #[test]
    fn the_frame_cap_reproduces_the_measured_render_boundary() {
        let caps = |name: &str, rows: u64| -> bool {
            let d = SCENARIOS
                .iter()
                .find(|d| d.name == name)
                .expect("committed scenario");
            d.row_level_marks * rows > FRAME_DRAWN_ROW_CAP
        };
        // Measured inked: one scatter at a hundred thousand rows.
        assert!(!caps("brush-density", 100_000));
        assert!(!caps("brush-binned-density", 100_000));
        // Measured BLANK (`Ok` returned, PNG written, every pixel transparent):
        // one scatter an order of magnitude up, and two scatters at the same
        // row count.
        assert!(caps("brush-density", 1_000_000));
        assert!(caps("crossfilter-dots", 100_000));
        // Two scatters stay inside it at half that.
        assert!(!caps("crossfilter-dots", 50_000));
        // Ten million row-level rows is past the boundary in every shape.
        assert!(caps("brush-density", 10_000_000));
        assert!(caps("crossfilter-dots", 10_000_000));
        // Aggregating-only scenes never hit it: their picture is O(bins).
        assert!(!caps("slider-drag", 10_000_000));
    }

    /// The cap's margin is a claim about the renderer, and the render crate is
    /// where it is measured — a scatter at this cap must reach the target, and
    /// one past the recorded onset must not. Both counts live there as
    /// literals. Held equal here so moving one alone reddens rather than
    /// leaving a gate measuring a boundary the harness no longer draws.
    #[test]
    fn the_render_crates_ink_gate_brackets_this_harness_at_these_numbers() {
        let literal = |name: &str| -> u64 {
            let line = RENDER_INK_GATE
                .lines()
                .find(|l| l.contains(&format!("const {name}: usize =")))
                .unwrap_or_else(|| panic!("the ink gate declares {name}"));
            line.split('=')
                .nth(1)
                .expect("a const declaration has a value")
                .trim()
                .trim_end_matches(';')
                .replace('_', "")
                .parse()
                .expect("the value is a decimal literal")
        };
        assert_eq!(
            literal("AT_THE_HARNESS_CAP"),
            FRAME_DRAWN_ROW_CAP,
            "the gate must render at the count this harness actually admits"
        );
        assert!(
            literal("PAST_THE_ONSET") > MEASURED_BLANK_MIN,
            "the gate's blank case must sit past the onset this record states, \
             or it proves nothing about the boundary"
        );
        assert!(
            literal("PAST_THE_ONSET") < VELLO_BIN_DATA_PANIC_DOTS,
            "and under the count where the encode panics instead of drawing \
             nothing, which is a different failure"
        );
    }

    /// A blank frame was explained, on every surface a reader could reach, by a
    /// mechanism that measurement refuted: the storage-buffer binding size (4
    /// GiB on this adapter — it does not bind) and a wgpu validation abort
    /// (vello returns `Ok`; the frame is written and empty). Sending a reader
    /// after a bigger GPU is worse than saying nothing.
    ///
    /// Checked on ALL THREE surfaces because they fail independently: the
    /// README is read before a run, `METHODOLOGY` ships inside every record,
    /// and `frames_skipped_reason` is the sentence printed against the very
    /// cell that has no number in it.
    #[test]
    fn no_surface_still_explains_a_blank_frame_with_the_refuted_mechanism() {
        let flatten = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let methodology = flatten(&methodology().join("\n"));
        let readme = flatten(BENCH_README);
        let reason = flatten(&frames_skipped_reason(200_000));

        for (surface, text) in [
            ("README", &readme),
            ("methodology", &methodology),
            ("skip reason", &reason),
        ] {
            for refuted in [
                "aborts inside the wgpu validation layer",
                "max_*_buffer_binding_size",
                "exceeds the renderer's max buffer binding size",
                "the renderer can encode in one scene buffer",
            ] {
                assert!(
                    !text.contains(refuted),
                    "the {surface} still explains a blank frame with {refuted:?}"
                );
            }
        }
    }

    /// The README described this harness as a guard that "never asks the
    /// renderer whether the frame it produced had ink in it". It asks now, and
    /// the README is the document read before a run — a reader left with the
    /// old sentence goes looking for a gap that is closed, or discounts a
    /// number that was checked.
    #[test]
    fn no_surface_still_says_the_harness_does_not_ask_the_renderer() {
        let flatten = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let readme = flatten(BENCH_README);
        let methodology = flatten(&methodology().join("\n"));
        for (surface, text) in [("README", &readme), ("methodology", &methodology)] {
            for refuted in [
                "That is a guard, not a detector",
                "never asks the renderer",
                "the cap is doing prediction's job",
            ] {
                assert!(
                    !text.contains(refuted),
                    "the {surface} still claims {refuted:?}"
                );
            }
        }
        assert!(
            readme.contains("A cell is now rendered before it is timed"),
            "the README must state what stands where the prediction did"
        );
        assert!(
            readme.contains("frames_blank"),
            "and name the field a failed cell lands in"
        );
        assert!(
            readme.contains("brightfield-bench/v4"),
            "and name the schema that carries it"
        );
    }

    /// Every surface states the MEASURED ceiling, not only the policy cap. A
    /// record that names the cap and stops has told the reader a harness
    /// constant and called it a property of the renderer.
    ///
    /// The expected figures are **read from the constants**, so this holds the
    /// surfaces to `brightfield_render::sample_policy` rather than to a second
    /// transcription of it here.
    #[test]
    fn every_generated_surface_states_the_measured_ceiling_not_only_the_cap() {
        let flatten = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let methodology = flatten(&methodology().join("\n"));
        let reason = flatten(&frames_skipped_reason(200_000));
        // The README writes its figures for humans, with thousands separators.
        let readme = flatten(BENCH_README);
        let plain = |n: u64| n.to_string();
        // Group the digits from the right, which is what a human-facing
        // document does with a five-or-more-digit count.
        let grouped = |n: u64| {
            let d = n.to_string();
            let mut out = String::new();
            for (i, c) in d.chars().enumerate() {
                if i > 0 && (d.len() - i).is_multiple_of(3) {
                    out.push(',');
                }
                out.push(c);
            }
            out
        };

        for (surface, text, inked, blank) in [
            (
                "methodology",
                &methodology,
                plain(MEASURED_INKED_MAX),
                plain(MEASURED_BLANK_MIN),
            ),
            (
                "skip reason",
                &reason,
                plain(MEASURED_INKED_MAX),
                plain(MEASURED_BLANK_MIN),
            ),
            (
                "README",
                &readme,
                grouped(MEASURED_INKED_MAX),
                grouped(MEASURED_BLANK_MIN),
            ),
        ] {
            let (inked, blank) = (inked.as_str(), blank.as_str());
            assert!(
                text.contains(inked) && text.contains(blank),
                "the {surface} must state the measured bracket, not only the cap"
            );
            assert!(
                text.contains("seg_counts"),
                "the {surface} must name what actually sets the ceiling"
            );
        }
        // The cap may still be named — it is what the harness enforces — but
        // never as the renderer's own limit.
        assert!(
            reason.contains(&FRAME_DRAWN_ROW_CAP.to_string()),
            "the reason must still say where the harness drew its line"
        );
    }

    /// The schema id must move when the field set does. v2 shipped
    /// `first_batch_rows`; v3 shipped `drawn_rows`, `drag` and
    /// `compose_memory`; v4 adds `frame_ink` and `frames_blank`. Leaving the id
    /// at v2 is what let two different field sets share one version — the
    /// failure this assertion exists to stop repeating.
    #[test]
    fn the_schema_id_is_past_the_first_batch_era() {
        assert_eq!(SCHEMA, "brightfield-bench/v4");
        assert!(
            methodology()
                .iter()
                .any(|m| m.contains("first_batch_rows") && m.contains("drawn_rows")),
            "the record must state the migration, so a v2 record is not quoted \
             against a v3 one"
        );
        assert!(
            methodology()
                .iter()
                .any(|m| m.contains("frame_ink") && m.contains("frames_blank")),
            "the record must state what v4 added, so a v3 frame cell is not \
             read as though something had checked it"
        );
    }

    /// The claim the whole readback rests on: a timed frame is one whose
    /// picture was rendered and found. A methodology that only described the
    /// cap would be describing a guard and calling it a detector.
    #[test]
    fn the_methodology_states_that_a_timed_cell_was_proven_to_have_a_picture() {
        let all = methodology().join("\n");
        assert!(
            all.contains("read back") && all.contains("frame_ink"),
            "the methodology must name the readback and the field it lands in"
        );
        assert!(
            all.contains("UNDER the cap is no longer assumed to have drawn"),
            "the methodology must say what changed about a cell below the cap"
        );
        // The honest boundary of the evidence. The probe renders its own
        // submission; the timed frames are not read back, and a methodology
        // that let a reader think otherwise would be overclaiming.
        assert!(
            all.contains("SEPARATE submission"),
            "the methodology must state what the probe does not cover"
        );
        // The second boundary, and the one a reader is likelier to overshoot:
        // the share is of the whole target, which a composed dashboard fills
        // with its own page tone before a mark exists. Read as mark coverage
        // it would say a picture is complete when it holds furniture alone.
        assert!(
            all.contains("THINNED") && all.contains("not mark coverage"),
            "the methodology must say the figure separates a picture from no \
             picture rather than measuring how much ink reached one"
        );
    }

    /// A blank picture and a skip both leave the frame columns empty, and they
    /// are opposite events. The summary has to separate them on the page, not
    /// only in the JSON.
    #[test]
    fn a_blank_picture_is_reported_as_a_failure_and_not_as_a_skip() {
        let mut r = synthetic_report();
        let ink = blank_ink();
        r.scaling[0].frames_blank = Some(frame_ink::blank_reason(&ink));
        r.scaling[0].frame_ink = Some(ink);
        r.scaling[0].frames_skipped = None;
        let md = render_markdown(&r);
        assert!(
            md.contains("### Rows whose picture came back empty"),
            "the failure gets its own section: {md}"
        );
        assert!(
            !md.contains("### Rows with no frame cells, and why"),
            "this row was not skipped, so it must not appear as one: {md}"
        );
        assert!(
            md.contains("FAILED, not skipped"),
            "the row's reason must say which of the two it is: {md}"
        );
        assert!(
            md.contains("| **BLANK** |"),
            "the table cell must say it too, beside the empty frame columns: {md}"
        );
    }

    /// The column exists so a reader of the table can see the contradiction —
    /// a frame time beside a picture that was never there — without opening
    /// the JSON.
    #[test]
    fn a_row_with_a_picture_states_what_share_of_it_was_inked() {
        let mut r = synthetic_report();
        r.scaling[0].frame_ink = Some(frame_ink::FrameInk {
            width: 1_280,
            height: 960,
            inked_pixels: 921_600,
            total_pixels: 1_228_800,
            inked_fraction: 0.75,
            uniform: false,
            drew_ink: true,
        });
        let md = render_markdown(&r);
        assert!(md.contains("| 75% inked |"), "{md}");
        assert!(
            md.contains("read back from the renderer, not inferred"),
            "the legend must say where the number came from: {md}"
        );
    }

    /// A row that attempted no frame suite rendered nothing, so it has no
    /// picture to report — and must not borrow a verdict it did not earn.
    #[test]
    fn a_row_that_attempted_no_frame_suite_claims_no_picture() {
        let md = render_markdown(&synthetic_report());
        assert!(
            md.contains("| — |"),
            "an unattempted row's picture cell is a gap: {md}"
        );
        assert!(
            !md.contains("### Rows whose picture came back empty"),
            "nothing was rendered, so nothing came back empty: {md}"
        );
    }

    #[test]
    fn iterations_at_the_distinct_limit_are_accepted() {
        // The default (20) and --quick (5) both sit under the cap; the cap
        // itself is the largest count the brush generator can serve distinctly.
        assert!(validate_iterations(1).is_ok());
        assert!(validate_iterations(20).is_ok());
        assert!(validate_iterations(scenario::DISTINCT_BRUSH_INTERVALS).is_ok());
    }

    #[test]
    fn iterations_past_the_distinct_limit_are_rejected() {
        let err = validate_iterations(scenario::DISTINCT_BRUSH_INTERVALS + 1)
            .expect_err("one past the distinct limit must be rejected");
        assert!(
            err.contains("SQL cache"),
            "the error must explain why a larger count is unsound: {err}"
        );
    }

    /// The frame suite's step budget is `--warmup-frames + --frames`, not
    /// `--iterations`, and it is indexed by the frame counter. It needs its
    /// own bound: `--iterations` being capped never constrained it.
    #[test]
    fn the_frame_step_budget_is_capped_at_the_distinct_period() {
        assert!(validate_frame_steps(0, 1).is_ok());
        assert!(
            validate_frame_steps(0, scenario::DISTINCT_BRUSH_INTERVALS).is_ok(),
            "the period itself is the largest distinct budget"
        );
        let err = validate_frame_steps(0, scenario::DISTINCT_BRUSH_INTERVALS + 1)
            .expect_err("one step past the period re-issues step 0");
        assert!(
            err.contains("SQL cache"),
            "the error must explain why a larger budget is unsound: {err}"
        );
    }

    /// The two flags are summed, so a budget can breach the period without
    /// either flag being large — the shape that made the default so close to
    /// the edge.
    #[test]
    fn warmup_and_measured_frames_are_capped_together() {
        assert!(
            validate_frame_steps(DEFAULT_WARMUP_FRAMES, DEFAULT_MEASURED_FRAMES).is_ok(),
            "the shipped default must be inside the period"
        );
        assert!(
            validate_frame_steps(QUICK_WARMUP_FRAMES, QUICK_MEASURED_FRAMES).is_ok(),
            "--quick must be inside the period"
        );
        // The default sits ON the boundary: one more measured frame breaches
        // it. That is the silent failure the reviewer found — `--frames 31`
        // with the shipped warm-up re-issues step 0.
        assert_eq!(
            DEFAULT_WARMUP_FRAMES + DEFAULT_MEASURED_FRAMES,
            scenario::DISTINCT_BRUSH_INTERVALS,
            "the default budget is exactly the period, so the cap is load-bearing"
        );
        assert!(
            validate_frame_steps(DEFAULT_WARMUP_FRAMES, DEFAULT_MEASURED_FRAMES + 1).is_err(),
            "one measured frame past the default must be rejected, not silently cached"
        );
        assert!(
            validate_frame_steps(DEFAULT_WARMUP_FRAMES + 1, DEFAULT_MEASURED_FRAMES).is_err(),
            "one warm-up frame past the default must be rejected too"
        );
    }

    /// A budget large enough to overflow `usize` must be rejected, not panic
    /// in a debug build.
    #[test]
    fn an_absurd_frame_budget_is_rejected_rather_than_overflowing() {
        assert!(validate_frame_steps(usize::MAX, usize::MAX).is_err());
    }

    /// A minimal report shaped like the real one: twenty timed steps, a
    /// two-mark scenario whose layer served both marks on every step (so
    /// `cube_hits` is 40, twice the step count), and two compose-memory
    /// windows that disagree by a factor of two on identical compose work.
    /// Both are shapes the committed record actually contains.
    fn synthetic_report() -> BaselineReport {
        let stats = Stats::from_ms(vec![1.0, 2.0, 3.0]).expect("a non-empty sample");
        let window = |peak: f64| mem::ComposeMemory {
            sampler: "test probe",
            rss_before_mib: Some(10.0),
            rss_peak_mib: Some(peak),
            rss_growth_mib: Some(peak - 10.0),
            rss_samples: 3,
            arrow_chunks_mib: 7.5,
            arrow_assembled_mib: 7.5,
        };
        let measurement = |peak: f64| EngineMeasurement {
            load_ms: 1.0,
            first_materialise_ms: 2.0,
            marks: vec![scenario::MarkRows {
                mark: 0,
                materialised_rows: 100,
                drawn_rows: 100,
                chunks: 1,
                chunk_bytes: 1024,
                assembled_bytes: 1024,
            }],
            coordinator_apply: stats.clone(),
            navigation_apply: stats.clone(),
            live_apply: stats.clone(),
            brushed_step_rows: 10,
            unfiltered_step_rows: 100,
            preagg: scenario::PreAggSummary {
                enabled: true,
                cubes_built: 2,
                cube_hits: 40,
                build_failures: 0,
                serve_failures: 0,
            },
            compose_memory: window(peak),
        };
        let field = |s: &str| s.to_string();
        BaselineReport {
            schema: SCHEMA,
            machine: MachineProfile {
                cpu: field("Test CPU"),
                logical_cpus: field("8"),
                memory_gib: field("16"),
                os: field("test os"),
                gpu_adapter: field("test gpu"),
                rustc: field("rustc test"),
                build_profile: field("release"),
                commit: field("0000000"),
                captured_at: field("2026-07-25T00:00:00+00:00"),
                duckdb: field("v1.5.2"),
            },
            config: RunConfig {
                rows: vec![10_000_000],
                iterations: 20,
                warmup_frames: DEFAULT_WARMUP_FRAMES,
                measured_frames: DEFAULT_MEASURED_FRAMES,
                corpus_frames: 20,
                frame_scale: FRAME_SCALE,
            },
            methodology: methodology(),
            scaling: vec![ScalingResult {
                scenario: field("slider-drag"),
                drag: Drag::Slider,
                rows: 10_000_000,
                dataset: field("uniform.parquet"),
                engine: measurement(200.0),
                engine_direct: measurement(100.0),
                frames: None,
                frames_skipped: Some(field("capped")),
                frame_ink: None,
                frames_blank: None,
            }],
            corpus: vec![],
        }
    }

    /// A cell whose picture was rendered and came back empty: the shape the
    /// readback exists to catch, and the one the primitive count cannot see.
    fn blank_ink() -> frame_ink::FrameInk {
        frame_ink::FrameInk {
            width: 1_280,
            height: 960,
            inked_pixels: 0,
            total_pixels: 1_228_800,
            inked_fraction: 0.0,
            uniform: true,
            drew_ink: false,
        }
    }

    /// `cube_hits` counts MARK re-queries, one per mark the layer serves — so
    /// a two-mark scenario over twenty steps reads `2/40`. Calling that column
    /// "drag steps served from a cube" put a number twice the step count under
    /// a step-count legend, in the same document whose preamble states the
    /// step count.
    #[test]
    fn the_cube_legend_names_mark_requeries_not_drag_steps() {
        let md = render_markdown(&synthetic_report());
        assert!(
            md.contains("20 timed drag steps"),
            "the preamble states the step count"
        );
        assert!(md.contains("| 2/40 |"), "the cell holds cubes/hits: {md}");
        assert!(
            !md.contains("drag steps served from a cube"),
            "40 hits over 20 steps are not 40 steps"
        );
        assert!(
            md.contains("**mark re-queries** served from a cube"),
            "the legend must name what the counter counts"
        );
        assert!(
            md.contains("two hits per drag step"),
            "the legend must explain why the number exceeds the step count"
        );
    }

    /// Two windows over the same compose work, differing 2x. The record must
    /// state that spread rather than presenting one peak as the compose's
    /// client cost.
    #[test]
    fn the_memory_note_states_its_own_spread_and_drops_the_cumulative_claim() {
        let md = render_markdown(&synthetic_report());
        assert!(
            md.contains("| Arrow held (MiB) | Compose peak RSS, direct / cubed (MiB) |"),
            "both configurations' peaks are columns, and the deterministic \
             figure leads: {md}"
        );
        assert!(md.contains("| 100 / 200 |"), "the pair is printed: {md}");
        assert!(
            md.contains("2.0x (slider-drag @ 10000000 rows)"),
            "the note must state the widest observed spread and where: {md}"
        );
        assert!(
            md.contains("Do not quote a peak without that spread"),
            "the note must say so plainly"
        );
        assert!(
            md.contains("`Arrow held` is the figure to quote"),
            "the reproducible figure must be named as the quotable one"
        );
        // The refuted claims. `rss_before` falls between adjacent windows in
        // the committed record, so RSS is not cumulative within a run, and
        // "read the peak, not the growth" rests on that premise.
        for refuted in [
            "Read the peak",
            "read the peak",
            "CUMULATIVE within one run",
            "the first of the pair, so the lower floor",
            "the compose is identical either way",
        ] {
            assert!(
                !md.contains(refuted),
                "the record must not repeat the refuted claim {refuted:?}"
            );
        }
    }

    /// With one configuration's window missing there is no spread to state,
    /// and the note must say that rather than printing a ratio it does not
    /// have.
    #[test]
    fn a_report_with_no_paired_window_states_no_spread() {
        let mut r = synthetic_report();
        r.scaling[0].engine.compose_memory.rss_peak_mib = None;
        assert!(worst_compose_rss_spread(&r).is_none());
        let md = render_markdown(&r);
        assert!(md.contains("it cannot be stated here"), "{md}");
        assert!(
            md.contains("| 100 / — |"),
            "a missing window is a gap: {md}"
        );
    }

    /// The methodology block ships inside every JSON record, so a claim left
    /// there outlives this README and every summary. Three of its claims about
    /// the memory column were refuted by the JSON printed beside them.
    #[test]
    fn the_methodology_block_carries_no_refuted_memory_claim() {
        let all = methodology().join("\n");
        for refuted in [
            "Read the PEAK",
            "RSS is cumulative",
            "cumulative within one harness run",
        ] {
            assert!(
                !all.contains(refuted),
                "the methodology must not repeat {refuted:?}: rss_before FALLS \
                 between adjacent windows in the committed record"
            );
        }
        assert!(
            all.contains("SINGLE sample") && all.contains("run-to-run noise"),
            "the methodology must state what the RSS figure actually is"
        );
        assert!(
            all.contains("--warmup-frames + --frames"),
            "the methodology must name the frame suites' own step budget"
        );
    }

    /// The README describes this harness to anyone reading a record. Three of
    /// its claims were refuted by the JSON printed beside them, a fourth
    /// described a cap the harness did not enforce, and two more outlived the
    /// engine: navigation had exactly one thing wrong with it, it was fixed,
    /// and a methodology section that still explained why a zoom can never be
    /// cubed would send a reader looking for the wrong number.
    #[test]
    fn the_readme_makes_no_claim_the_harness_contradicts() {
        // BOTH surfaces, because a retired claim can come back on either and
        // only one of them was ever checked. The README is what someone reads
        // before running the harness; `METHODOLOGY` is what ships inside every
        // record and is rendered into the markdown beside it — so it is what a
        // reader of a *result* sees, which is the more consequential of the
        // two. A review found both retired sentences could be re-inserted into
        // the rendered blurb with all 42 tests still green.
        let rendered = methodology().join("\n");
        for (surface, text) in [("README", BENCH_README), ("methodology", &rendered[..])] {
            for refuted in [
                "Read the peak, not the growth",
                "RSS is cumulative within a run",
                "single `--iterations` cap keeps the guarantee",
                "brush steps served from a cube",
                "only call site is selection propagation",
                "quoted from the direct run alone",
            ] {
                assert!(
                    !text.contains(refuted),
                    "the {surface} still claims {refuted:?}"
                );
            }
        }
        // Line wrapping is not part of a claim, so match on the flattened
        // text — a re-wrap must not turn this gate off.
        let flat = BENCH_README
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        for required in [
            "--warmup-frames + --frames",
            "**mark re-queries** served from a cube",
            "one sample of a whole-process quantity, and it does not reproduce on its own",
            "a peak must not be quoted without that spread",
        ] {
            assert!(
                flat.contains(required),
                "the README must state {required:?}"
            );
        }
    }

    /// The scenario spec files are committed public documents and are compiled
    /// into this harness, so a stale claim in one of them ships twice. The
    /// dots spec described the pre-assembly presentation as current behaviour.
    #[test]
    fn no_committed_spec_still_describes_the_first_chunk_presentation() {
        for (name, text) in [
            ("crossfilter-density", SPEC_DENSITY),
            ("crossfilter-binned-density", SPEC_BINNED),
            ("crossfilter-dots", SPEC_DOTS),
            ("slider-drag", SPEC_SLIDER),
            ("crosswalk-confidence", SPEC_CROSSWALK),
        ] {
            let flat = text.replace("\n# ", " ").replace('\n', " ");
            for stale in [
                "draws a mark's first",
                "first Arrow batch",
                "first Arrow chunk",
                "first batch only",
            ] {
                assert!(
                    !flat.contains(stale),
                    "{name}.yaml still states {stale:?} as current behaviour — the \
                     compose assembles every chunk"
                );
            }
        }
        assert!(
            SPEC_DOTS.contains("assembles every"),
            "the dots spec must state the behaviour that ships"
        );
    }
}
