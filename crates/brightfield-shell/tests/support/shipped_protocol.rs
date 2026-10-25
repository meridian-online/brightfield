//! Shipped Protocols built for a test, and the offline stand-in for one that
//! fetches.
//!
//! A [`ShippedProtocol`] is `'static` because a start's is compiled in. A test
//! builds its own from files on disk at run time, so each one here is leaked:
//! a few kilobytes for the life of a test binary.
//!
//! **The stand-in.** A Protocol start whose steps include an `http_fetch` reads
//! the network on its first run, which a hermetic test may not. [`stand_in`]
//! makes the variant such a test boots instead: each `http_fetch` step whose
//! URL has a committed copy is dropped from the manifest, and the copy is
//! written where the step would have put the file, the way
//! `support/agent_protocol.rs` keeps the CO2 fixture off the network. Every
//! other file is the start's own, so what the variant draws is what the start
//! draws over the copy. Where a copy of the fetched file cannot be committed,
//! a copy of the table its load step makes stands in for that step and the
//! ones before it instead ([`stand_in_with_loads`]).

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use brightfield_shell::starts::{is_fetch_op, ShippedProtocol};
use brightfield_shell::window::Boot;

/// The committed fixture folder: one bundled CSV, one SQL step, one panel.
pub fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/shipped-protocol")
}

/// The fixture's files, relative to its folder.
const FIXTURE_FILES: &[&str] = &[
    "arcform.yaml",
    "models/measures.sql",
    "data/measures.csv",
    "panels/overview.yaml",
];

/// The data file the fixture's SQL step reads, relative to its folder.
pub const DATA_FILE: &str = "data/measures.csv";

fn leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn leak_files(files: Vec<(String, Vec<u8>)>) -> &'static [(&'static str, &'static [u8])] {
    let files: Vec<(&'static str, &'static [u8])> = files
        .into_iter()
        .map(|(path, bytes)| {
            let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
            (leak_str(path), bytes)
        })
        .collect();
    Box::leak(files.into_boxed_slice())
}

/// The files of `protocol`, owned, so a variant can be made of them.
pub fn files_of(protocol: &ShippedProtocol) -> Vec<(String, Vec<u8>)> {
    protocol
        .files
        .iter()
        .map(|(path, bytes)| ((*path).to_string(), bytes.to_vec()))
        .collect()
}

/// A shipped Protocol of `files`, written as `dir_name` at `revision`.
pub fn shipped(
    dir_name: &str,
    revision: u32,
    files: Vec<(String, Vec<u8>)>,
) -> &'static ShippedProtocol {
    Box::leak(Box::new(ShippedProtocol {
        dir_name: leak_str(dir_name.to_string()),
        revision,
        files: leak_files(files),
    }))
}

/// The committed fixture as a shipped Protocol named `dir_name`.
pub fn fixture(dir_name: &str, revision: u32) -> &'static ShippedProtocol {
    let files = FIXTURE_FILES
        .iter()
        .map(|file| {
            let bytes = std::fs::read(fixture_dir().join(file))
                .unwrap_or_else(|e| panic!("the fixture's {file}: {e}"));
            ((*file).to_string(), bytes)
        })
        .collect();
    shipped(dir_name, revision, files)
}

/// The fixture as a Protocol that **fetches** its data: an `http_fetch` step
/// of `url` into the data file comes first, and the bundled copy is gone.
pub fn fetching_fixture(dir_name: &str, url: &str) -> &'static ShippedProtocol {
    let mut files = files_of(fixture(dir_name, 1));
    files.retain(|(path, _)| path != DATA_FILE);
    for (path, bytes) in &mut files {
        if path == "arcform.yaml" {
            let text = String::from_utf8(bytes.clone()).expect("the manifest is text");
            let fetch = format!(
                "steps:\n  - name: fetch_measures\n    op: http_fetch\n    with:\n      \
                 out: {DATA_FILE}\n      url: {url}\n    description: Fetch the measurements\n"
            );
            assert!(text.contains("steps:\n"), "the manifest has a steps list");
            *bytes = text.replacen("steps:\n", &fetch, 1).into_bytes();
        }
    }
    shipped(dir_name, 1, files)
}

/// The URL and output path of each `http_fetch` step in `manifest` that
/// names both, read through the start's own [`brightfield_shell::starts::fetch_steps`].
fn fetch_steps(manifest: &serde_yaml::Value) -> Vec<(String, String)> {
    let text = serde_yaml::to_string(manifest).expect("the manifest serialises");
    brightfield_shell::starts::fetch_steps(&text)
        .into_iter()
        .filter_map(|step| Some((step.url, step.out?)))
        .collect()
}

/// `protocol` with each `http_fetch` step whose URL is in `copies` dropped,
/// and the committed copy of what it fetches written at its `out`.
///
/// A fetch with no copy in `copies` fails here by URL, rather than leaving a
/// step that would reach the network.
pub fn stand_in(protocol: &ShippedProtocol, copies: &[(&str, &Path)]) -> &'static ShippedProtocol {
    stand_in_with_loads(protocol, copies, &[])
}

/// [`stand_in`], where some copies stand in for the **table** a load step
/// makes rather than for the file a fetch writes: `loads` is
/// `(url, step, copy)`.
///
/// For each whose URL the Protocol fetches, the steps before `step` (the
/// fetch, and whatever unpacks it) are dropped, the copy is written under
/// `data/`, and `step`'s model becomes a read of it, so every step after
/// `step` runs as it is shipped. That is for a source a hermetic test cannot
/// read as it arrives, such as a zipped workbook, or whose load step drops
/// columns the committed copy must not hold.
pub fn stand_in_with_loads(
    protocol: &ShippedProtocol,
    copies: &[(&str, &Path)],
    loads: &[(&str, &str, &Path)],
) -> &'static ShippedProtocol {
    let mut files = files_of(protocol);
    let manifest_at = files
        .iter()
        .position(|(path, _)| path == "arcform.yaml")
        .expect("a shipped Protocol has an arcform.yaml");
    let mut manifest: serde_yaml::Value =
        serde_yaml::from_slice(&files[manifest_at].1).expect("the manifest parses");
    let mut written = Vec::new();
    let fetched: Vec<String> = fetch_steps(&manifest)
        .into_iter()
        .map(|(url, _)| url)
        .collect();
    for (url, step, copy) in loads {
        if !fetched.iter().any(|u| u == url) {
            continue;
        }
        let steps = manifest
            .get_mut("steps")
            .and_then(serde_yaml::Value::as_sequence_mut)
            .expect("a Protocol that fetches has steps");
        let at = steps
            .iter()
            .position(|s| s.get("name").and_then(serde_yaml::Value::as_str) == Some(step))
            .unwrap_or_else(|| {
                panic!(
                    "{} has no step {step} to stand a copy in for",
                    protocol.dir_name
                )
            });
        let model = steps[at]
            .get("sql")
            .and_then(serde_yaml::Value::as_str)
            .unwrap_or_else(|| panic!("{step} in {} is not a SQL step", protocol.dir_name))
            .to_string();
        steps.drain(..at);
        let name = copy
            .file_name()
            .and_then(|n| n.to_str())
            .expect("a copy has a file name");
        let data = format!("data/{name}");
        let bytes = std::fs::read(copy).unwrap_or_else(|e| panic!("{}: {e}", copy.display()));
        written.push((data.clone(), bytes));
        written.push((
            model,
            format!(
                "CREATE OR REPLACE TABLE \"{step}\" AS\nSELECT * FROM read_parquet('{data}')\n"
            )
            .into_bytes(),
        ));
    }
    for (url, out) in fetch_steps(&manifest) {
        let (_, copy) = copies.iter().find(|(u, _)| *u == url).unwrap_or_else(|| {
            panic!(
                "{} fetches {url} and nothing stands in for it, so it cannot \
                     run offline",
                protocol.dir_name
            )
        });
        let bytes = std::fs::read(copy).unwrap_or_else(|e| panic!("{}: {e}", copy.display()));
        written.push((out, bytes));
    }
    if let Some(steps) = manifest
        .get_mut("steps")
        .and_then(serde_yaml::Value::as_sequence_mut)
    {
        steps.retain(|step| {
            !step
                .get("op")
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(is_fetch_op)
        });
    }
    files[manifest_at].1 = serde_yaml::to_string(&manifest)
        .expect("the manifest serialises")
        .into_bytes();
    for (out, bytes) in written {
        files.retain(|(path, _)| *path != out);
        files.push((out, bytes));
    }
    shipped(protocol.dir_name, protocol.revision, files)
}

/// The committed copies of what the shipped Protocol starts fetch, as
/// `(url, file under this crate)`: what [`boot_offline`] runs a Protocol
/// start's stand-in over. A Protocol start fetching a URL missing here fails
/// [`stand_in`] by that URL rather than reaching the network.
///
/// Epoch AI's entry is a synthetic CSV of six made-up models with the file's
/// real columns, `Authors` and `Abstract` among them as placeholders. A start
/// [`boot_offline`] boots reads its reduced copy in [`PROTOCOL_LOAD_COPIES`]
/// instead; the synthetic file is there so a test can run the shipped load
/// step, which leaves those two columns out, with no network.
pub const PROTOCOL_FETCH_COPIES: &[(&str, &str)] = &[
    (
        "https://d37ci6vzurychx.cloudfront.net/trip-data/yellow_tripdata_2026-01.parquet",
        "tests/data/nyc-yellow-taxi-reduced.parquet",
    ),
    (
        "https://epoch.ai/data/notable_ai_models.csv",
        "tests/data/epoch-ai-models-synthetic.csv",
    ),
];

/// The committed copies that stand in for the table a shipped Protocol's
/// load step makes, as `(url fetched, load step, file under this crate)`: see
/// [`stand_in_with_loads`]. Online Retail's source is a zipped workbook, and
/// Epoch AI's load step drops columns its copy must not hold, so neither
/// copy can stand in for the fetched file itself. `scripts/cut-start-fixtures.sh`
/// cuts each by running the start's own load step.
///
/// So the load step a copy stands in for does not run offline. Epoch AI's
/// runs in `the_epoch_ai_models_load_step_leaves_out_authors_and_abstract_offline`,
/// over a synthetic CSV in [`PROTOCOL_FETCH_COPIES`]. Online Retail's
/// `read_xlsx` load runs in the `#[ignore]`d network test
/// `the_online_retail_ii_protocol_fetches_and_produces_the_columns_its_panel_reads`
/// and in a re-cut, which runs it on UCI's workbook, and in no per-push test.
pub const PROTOCOL_LOAD_COPIES: &[(&str, &str, &str)] = &[
    (
        "https://archive.ics.uci.edu/static/public/502/online+retail+ii.zip",
        "retail_lines",
        "tests/data/online-retail-ii-reduced.parquet",
    ),
    (
        "https://epoch.ai/data/notable_ai_models.csv",
        "raw_models",
        "tests/data/epoch-ai-models-reduced.parquet",
    ),
];

/// The runner a Protocol start is run by in a test: the `brightfield-shell`
/// binary cargo built, never the test harness, which reads `run` as a filter.
pub fn runner() -> brightfield_shell::run::Runner {
    crate::agent_protocol::arc_stores();
    brightfield_shell::run::Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// Boot `start` as a test with no network does: a Protocol start through
/// [`boot_offline`], run first by the cargo-built runner over its stand-in
/// (`Boot::start` has no runner to hand a run to), and any other start
/// through `Boot::start`.
pub fn boot_start(start: &'static brightfield_shell::starts::Start) -> Result<Boot, String> {
    if start.protocol.is_some() {
        boot_offline(start)
    } else {
        Boot::start(start.id, brightfield_protocol::layout::Flow::Vertical)
    }
}

/// Boot the Protocol start `start` with no network: its stand-in over
/// [`PROTOCOL_FETCH_COPIES`], run to completion by [`runner`] first.
///
/// # Panics
///
/// If `start` is not a Protocol start.
pub fn boot_offline(start: &'static brightfield_shell::starts::Start) -> Result<Boot, String> {
    let protocol = start
        .protocol
        .as_ref()
        .unwrap_or_else(|| panic!("{} is not a Protocol start", start.id));
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let copies: Vec<(&str, PathBuf)> = PROTOCOL_FETCH_COPIES
        .iter()
        .map(|(url, file)| (*url, crate_dir.join(file)))
        .collect();
    let borrowed: Vec<(&str, &Path)> = copies
        .iter()
        .map(|(url, path)| (*url, path.as_path()))
        .collect();
    let loads: Vec<(&str, &str, PathBuf)> = PROTOCOL_LOAD_COPIES
        .iter()
        .map(|(url, step, file)| (*url, *step, crate_dir.join(file)))
        .collect();
    let loads: Vec<(&str, &str, &Path)> = loads
        .iter()
        .map(|(url, step, path)| (*url, *step, path.as_path()))
        .collect();
    let offline = stand_in_with_loads(protocol, &borrowed, &loads);
    Boot::shipped_protocol(
        start.id,
        offline,
        brightfield_protocol::layout::Flow::Vertical,
        Some(&runner()),
    )
}
