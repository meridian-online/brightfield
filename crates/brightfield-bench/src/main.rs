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
mod scenario;
mod stats;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;

use crate::machine::MachineProfile;
use crate::scenario::{EngineMeasurement, Scenario};
use crate::stats::Stats;

/// The density-pair scenario spec, compiled in from `benchmarks/specs/`.
const SPEC_DENSITY: &str = include_str!("../../../benchmarks/specs/crossfilter-density.yaml");
/// The bounded-cardinality density spec, compiled in from `benchmarks/specs/`.
const SPEC_BINNED: &str = include_str!("../../../benchmarks/specs/crossfilter-binned-density.yaml");
/// The raw-dot scenario spec, compiled in from `benchmarks/specs/`.
const SPEC_DOTS: &str = include_str!("../../../benchmarks/specs/crossfilter-dots.yaml");
/// The crosswalk scenario spec (opt-in, fixed scale), compiled in from
/// `benchmarks/specs/`.
const SPEC_CROSSWALK: &str = include_str!("../../../benchmarks/specs/crosswalk-confidence.yaml");

/// One committed scaling scenario: which spec, which brush, what the
/// pre-aggregation layer is expected to do.
struct SpecDef {
    name: &'static str,
    template: &'static str,
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
        brush_column: "value_a",
        brush_domain: (0.0, 100.0),
        // Row-level marks: nothing to pre-aggregate; the layer must stay
        // silent.
        expect_cube: false,
        frames_capped: true,
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
        Ok(args)
    }
}

/// One scenario × row-count row of the baseline. The engine suites run twice
/// on identical code — pre-aggregation enabled (`engine`, the shipped
/// configuration) and disabled (`engine_direct`) — so the delta between the
/// two brush-step latencies is attributable to the layer alone.
#[derive(Debug, Serialize)]
struct ScalingResult {
    scenario: String,
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
        schema: "brightfield-bench/v2",
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
        "Latency cells are `p50 / p95 / max` in milliseconds over {} timed brush steps \
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
         cubes built / brush steps served from a cube."
    );
    let _ = writeln!(md);
    let _ = writeln!(
        md,
        "| Scenario | Rows | Cold open (load + first query, ms) | \
         Brush → data, direct (ms) | Brush → data, cubed (ms) | \
         Brush → scene, direct (ms) | Brush → scene, cubed (ms) | Cube | \
         Steady frame (ms) | Interaction frame (ms) | Drawn/materialised rows |"
    );
    let _ = writeln!(
        md,
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
    );
    for s in &r.scaling {
        let drawn = s
            .engine
            .marks
            .iter()
            .map(|mk| format!("{}/{}", mk.drawn_rows, mk.materialised_rows))
            .collect::<Vec<_>>()
            .join(" · ");
        let _ = writeln!(
            md,
            "| {} | {} | {:.1} + {:.1} | {} | {} | {} | {} | {}/{} | {} | {} | {} |",
            s.scenario,
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
        );
    }
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
        assert!(SPEC_CROSSWALK.contains("__DATA_PARQUET__"));
    }
}
