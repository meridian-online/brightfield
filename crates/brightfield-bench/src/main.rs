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
mod frames;
mod machine;
mod mem;
mod scenario;
mod stats;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;

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
    /// Whether frame suites are capped by row count for this scenario.
    frames_capped: bool,
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
        frames_capped: false,
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
        frames_capped: false,
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
        frames_capped: true,
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
        // magnitude — a drag frame here is the drag, not a raster.
        frames_capped: false,
    },
];

/// Row counts above this skip the raw-dot scenario's FRAME suites (its engine
/// suites run at every magnitude): an interaction frame over ten million raw
/// dots spends its whole budget inside the apply the engine suites already
/// time, and adds nothing but wall-clock to the run.
const DOTS_FRAME_ROW_CAP: u64 = 1_000_000;

/// The device-pixel scale frames render at — 2.0 matches the Retina-class
/// displays the live window actually runs on.
const FRAME_SCALE: f32 = 2.0;

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
            warmup_frames: 5,
            measured_frames: 30,
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
                    args.measured_frames = 8;
                    args.warmup_frames = 2;
                    args.corpus_frames = 6;
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        if args.rows.is_empty() || args.iterations == 0 || args.measured_frames == 0 {
            return Err("rows, iterations and frames must be non-zero".into());
        }
        validate_iterations(args.iterations)?;
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
    methodology: Vec<&'static str>,
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
const SCHEMA: &str = "brightfield-bench/v3";

const METHODOLOGY: &[&str] = &[
    "Interaction latency is measured at the coordinator seam the live window blocks its frame on: one committed brush step = Coordinator::apply (predicate push-down into DuckDB + re-query of every affected mark). live_apply adds the re-composite into a Vello scene (LiveDashboard::apply), which is the full in-frame cost of a brush step in the live window.",
    "Every timed brush uses a distinct interval: the engine caches repeated identical SQL, so a repeated interval would time the cache. A non-vacuity check requires the brush to have actually reduced the cross-filtered step's row count, and every apply must affect at least one mark.",
    "Frame times are headless: the real MeridianApp drawn by egui's real wgpu backend into an offscreen texture, timed per frame through GPU completion (submit + blocking wait). No swapchain, no present, no vsync — the number is the cost of producing a frame, not displaying one. Warm-up frames are discarded.",
    "steady frames draw with nothing changing (the shell's floor). interaction frames each push one committed brush step through the live document before drawing, so they carry re-query + re-composite + canvas re-raster + GPU wait.",
    "The composed scene draws EVERY materialised Arrow chunk: a mark's result batches are assembled into one drawable batch (assemble_batches), the same path the presentation layer uses. drawn_rows vs materialised_rows is the cross-check — they are equal, so the drawn picture holds every row the query answered (the raw-dot scenario spans many ~2048-row chunks and still draws them all). A future regression that reintroduced a first-chunk cap would show drawn_rows < materialised_rows here; an assembly that could not proceed fails the run loudly by name rather than reporting a smaller drawn count.",
    "cold open = Coordinator::load (DDL, no mark queries) then the first full materialisation of every mark, on a session in the same process; the Parquet file is warm in the OS page cache.",
    "Datasets are deterministic pure functions of the row index via DuckDB hash() — no RNG. The raw-dot scenario's frame suites are capped at one million rows; its engine suites run at every magnitude.",
    "The emitted SQL applies a selection predicate INSIDE an aggregating mark's query — it filters the base rows that get aggregated (row-level marks are wrapped whole). The aggregating scenarios keep their original brush-the-binned-column shape so the measured series stays comparable across harness runs.",
    "Each scenario's engine suites run twice on identical code: automatic pre-aggregation enabled (the shipped configuration) and disabled (the direct-query control). The delta between the two brush-step latencies is the layer's contribution. Cube engagement is verified per run — engaged and serving where the scenario expects it, silent where it does not — and a run whose cube behaviour contradicts the expectation FAILS instead of reporting.",
    "Active interval dimensions enter a cube at RAW data values in this first cut (answer-exactness over cube size). A cube over a ~unique-per-row brushed column (brush-density's value_a) therefore approaches the base table's size and buys little; the bounded-cardinality scenario (brush-binned-density, forty distinct brushed values) and the crosswalk scenario measure the shape the layer is built for. Frame suites run in the shipped configuration only.",
    "The crosswalk scenario is opt-in (--crosswalk-parquet) and fixed-scale: it measures the published company-identifier crosswalk dataset as-is; the harness records the file's row count rather than generating data.",
    "In the enabled run, the FIRST brush step carries the one-time cube build (a full-table aggregation); it surfaces in the max percentile, while p50 reflects the steady per-step serve cost. Cubes are session-scoped and never persist.",
    "Each row records which GESTURE drove its timed steps (`drag`). A brush moves BOTH interval endpoints as a rectangle is dragged inside a chart; a slider pins its lower end and advances one handle across fixed stops. They are not interchangeable: the slider's steps are monotone and land exactly on the brushed axis's forty stops (a step falling between two stops would emit different SQL while selecting identical rows, reporting a re-query that did no new work as though it had).",
    "The slider-drag scenario drives a SELECTION, not a scalar param. Upstream Mosaic expresses a range slider as `select: interval`, and this engine's pre-aggregation layer keys its cubes off that structured clause — the only path reaching it is selection propagation. A slider wired to a scalar param arrives at the query layer as a substituted expression predicate that decomposes into no cube at all, so measuring it would measure a different mechanism under the slider's name. The spec vocabulary has no slider-to-selection widget form yet, so the contributor is declared as the interval interactor it does have and the harness drives it with slider-shaped steps; no row in this record took the scalar-param path.",
    "slider-drag resolves its selection as `intersect`, not `crossfilter`. A slider is not a view and has no picture of its own to spare, so EVERY subscriber is filtered, including the plot that declares the contributor — both views re-query on every step. A crossfilter brush exempts its own plot, so a brush row's per-step work is one view lighter than a slider row's at the same row count; compare the two knowing that.",
    "compose_memory is what the client held while the first full scene was composed — the window around LiveDashboard::present, which executes every mark and hands the whole result set to compose_from_results, which assembles every chunk of every mark and holds them all while it builds the scene. Two figures, because neither alone is honest. arrow_chunks_mib / arrow_assembled_mib are EXACT and deterministic, summed from the batches themselves (RecordBatch::get_array_memory_size) — the data the compose holds, identical on any host. A single-chunk mark assembles by pass-through, so its assembled bytes are the SAME allocation counted again, not a second copy.",
    "rss_*_mib is the process's resident set size, polled from the OS across that same window (Linux /proc/self/status; macOS `ps -o rss=`, ~5ms resolution, so `rss_samples` states how coarse a given cell is). It is the only figure that sees the Arrow buffers' real cost: the chunks are imported from DuckDB over the C data interface, so DuckDB's C++ allocator owns them and a Rust allocator counter would not see them. Read the PEAK, not the growth: RSS is cumulative within one harness run because a general-purpose allocator does not return freed pages promptly, so a scenario measured late starts from an already-high floor and its growth can read near zero while holding hundreds of megabytes. The poller stops before the timed applies, so it cannot perturb a reported latency.",
    "Record schema v3. v2 carried `first_batch_rows` beside `materialised_rows` — the pre-assembly presentation that drew a mark's FIRST Arrow chunk only. That field is gone; `drawn_rows` names what the field now holds. A v2 record's row counts describe code that no longer ships and must not be quoted against a v3 one.",
];

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
            let frames_capped = def.frames_capped && rows > DOTS_FRAME_ROW_CAP;
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

            let frames = if args.skip_frames || frames_capped {
                None
            } else {
                // The spec must exist as a file for the shell's boot path.
                let spec_path = args.data_dir.join(format!("{}_{rows}.yaml", def.name));
                std::fs::write(&spec_path, &sc.spec_text)
                    .map_err(|e| format!("write {}: {e}", spec_path.display()))?;
                let parsed =
                    brightfield_spec::parse_spec(&sc.spec_text, brightfield_spec::Format::Yaml)
                        .map_err(|e| format!("{}: parse: {e}", def.name))?;
                let bindings = brightfield_spec::analysis::build_brushable_bindings(&parsed.spec);
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
        methodology: METHODOLOGY.to_vec(),
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
         cubes built / drag steps served from a cube. `Drag` is the gesture the \
         timed steps imitate: a **brush** moves both interval endpoints inside a \
         chart and exempts its own plot (crossfilter); a **slider** pins its \
         lower end, advances one handle across fixed stops, and filters every \
         view including the contributor's (intersect) — so a slider step \
         re-queries one more view than a brush step at the same row count."
    );
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "| Scenario | Drag | Rows | Cold open (load + first query, ms) | \
         Step → data, direct (ms) | Step → data, cubed (ms) | \
         Step → scene, direct (ms) | Step → scene, cubed (ms) | Cube | \
         Steady frame (ms) | Interaction frame (ms) | Drawn/materialised rows | \
         Compose peak RSS (MiB) | Arrow held (MiB) |"
    );
    let _ = writeln!(
        md,
        "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
    );
    for s in &r.scaling {
        let drawn = s
            .engine
            .marks
            .iter()
            .map(|mk| format!("{}/{}", mk.drawn_rows, mk.materialised_rows))
            .collect::<Vec<_>>()
            .join(" · ");
        // The DIRECT run's window: it runs first at each row count, so its
        // allocator floor is the lower of the pair. The compose itself is
        // identical either side of the toggle — the first present happens
        // before any interaction, so no cube exists yet to change it.
        let mem = &s.engine_direct.compose_memory;
        let _ = writeln!(
            md,
            "| {} | {} | {} | {:.1} + {:.1} | {} | {} | {} | {} | {}/{} | {} | {} | {} | {} | {:.1} |",
            s.scenario,
            drag_label(s.drag),
            s.rows,
            s.engine.load_ms,
            s.engine.first_materialise_ms,
            fmt_stats(&s.engine_direct.coordinator_apply),
            fmt_stats(&s.engine.coordinator_apply),
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
            mem.rss_peak_mib.map_or("—".into(), |p| format!(
                "{p:.0} (+{})",
                mem.rss_growth_mib.map_or("?".into(), |g| format!("{g:.0}"))
            )),
            mem.arrow_chunks_mib,
        );
    }
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "`Compose peak RSS` is the whole process's high-water mark across the \
         first full compose — every mark executed, every Arrow chunk assembled, \
         the scene built — with the growth over the pre-compose reading in \
         brackets. It is CUMULATIVE within one run: the allocator does not \
         return freed pages promptly, so a scenario measured late starts from an \
         already-high floor and its growth understates what it held. Read the \
         peak. The cell is the direct-configuration run (the first of the pair, \
         so the lower floor); the cubed run's window is in the JSON beside it, \
         and the compose is identical either way — the first present precedes \
         any interaction, so no cube exists yet. `Arrow held` is exact and \
         deterministic: the summed size of every materialised chunk the compose \
         holds at once, read from the batches themselves. Sampler: {}.",
        r.scaling
            .first()
            .map_or("none", |s| s.engine_direct.compose_memory.sampler)
    );
    let _ = writeln!(md);
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

    /// The schema id must move when the field set does. v2 shipped
    /// `first_batch_rows`; v3 ships `drawn_rows`, `drag` and `compose_memory`.
    /// Leaving the id at v2 is what let two different field sets share one
    /// version — the failure this assertion exists to stop repeating.
    #[test]
    fn the_schema_id_is_past_the_first_batch_era() {
        assert_eq!(SCHEMA, "brightfield-bench/v3");
        assert!(
            METHODOLOGY
                .iter()
                .any(|m| m.contains("first_batch_rows") && m.contains("drawn_rows")),
            "the record must state the migration, so a v2 record is not quoted \
             against a v3 one"
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
}
