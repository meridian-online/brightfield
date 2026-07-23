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
/// The raw-dot scenario spec, compiled in from `benchmarks/specs/`.
const SPEC_DOTS: &str = include_str!("../../../benchmarks/specs/crossfilter-dots.yaml");

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

/// One scenario × row-count row of the baseline.
#[derive(Debug, Serialize)]
struct ScalingResult {
    scenario: String,
    rows: u64,
    dataset: String,
    #[serde(flatten)]
    engine: EngineMeasurement,
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
    "The composed scene currently draws a mark's FIRST Arrow batch only; materialised_rows vs first_batch_rows records where the drawn picture holds fewer rows than the query answered (the aggregating scenario fits one batch by construction; the raw-dot scenario does not past one batch).",
    "cold open = Coordinator::load (DDL, no mark queries) then the first full materialisation of every mark, on a session in the same process; the Parquet file is warm in the OS page cache.",
    "Datasets are deterministic pure functions of the row index via DuckDB hash() — no RNG. The raw-dot scenario's frame suites are capped at one million rows; its engine suites run at every magnitude.",
    "The emitted SQL applies a selection predicate OUTSIDE a mark's query, so an aggregating mark can only be cross-filtered by a column its aggregation projects (a foreign column is a binder error today). The brush-density scenario therefore bins the brushed column itself: each brush step still re-runs the full-table aggregation, and the predicate filters bins after it. Moving that predicate inside the aggregation is what a pre-aggregation layer changes; this baseline measures what exists.",
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
                 [--out-dir D] [--data-dir D] [--label NAME]"
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
        for (name, template) in [
            ("brush-density", SPEC_DENSITY),
            ("crossfilter-dots", SPEC_DOTS),
        ] {
            let frames_capped = name == "crossfilter-dots" && rows > DOTS_FRAME_ROW_CAP;
            eprintln!("scenario {name} @ {rows} rows ...");
            let spec_text = template.replace(
                "__DATA_PARQUET__",
                dataset.to_str().ok_or("dataset path is not UTF-8")?,
            );
            let sc = Scenario { name, spec_text };

            let engine = scenario::run_engine_suites(&sc, None, args.iterations)?;

            let frames = if args.skip_frames || frames_capped {
                None
            } else {
                // The spec must exist as a file for the shell's boot path.
                let spec_path = args.data_dir.join(format!("{name}_{rows}.yaml"));
                std::fs::write(&spec_path, &sc.spec_text)
                    .map_err(|e| format!("write {}: {e}", spec_path.display()))?;
                let parsed =
                    brightfield_spec::parse_spec(&sc.spec_text, brightfield_spec::Format::Yaml)
                        .map_err(|e| format!("{name}: parse: {e}"))?;
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
                scenario: name.to_string(),
                rows,
                dataset: dataset
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                engine,
                frames,
            });
        }
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
        schema: "brightfield-bench/v1",
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
        "| Scenario | Rows | Cold open (load + first query, ms) | Brush → data (ms) | \
         Brush → scene (ms) | Steady frame (ms) | Interaction frame (ms) | Drawn/materialised rows |"
    );
    let _ = writeln!(md, "|---|---:|---:|---:|---:|---:|---:|---:|");
    for s in &r.scaling {
        let drawn = s
            .engine
            .marks
            .iter()
            .map(|mk| format!("{}/{}", mk.first_batch_rows, mk.materialised_rows))
            .collect::<Vec<_>>()
            .join(" · ");
        let _ = writeln!(
            md,
            "| {} | {} | {:.1} + {:.1} | {} | {} | {} | {} | {} |",
            s.scenario,
            s.rows,
            s.engine.load_ms,
            s.engine.first_materialise_ms,
            fmt_stats(&s.engine.coordinator_apply),
            fmt_stats(&s.engine.live_apply),
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
        assert!(SPEC_DOTS.contains("__DATA_PARQUET__"));
    }
}
