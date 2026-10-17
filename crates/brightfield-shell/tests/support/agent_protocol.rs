//! The agent-written Protocol fixtures, copied and run by `arc` for a test.
//!
//! Each fixture under `tests/fixtures/agent-protocols/` is a Protocol an agent
//! wrote, kept as written. A test gets a copy in a directory of its own and,
//! through [`ran_fixture`], the database `arc run` makes for that copy, so what
//! the test reads back is a real run's tables rather than a database built to
//! look like one.
//!
//! **No network.** The CO2 fixture's first step is an `http_fetch` of a remote
//! CSV into `data/owid-co2-data.csv`, and the committed file is already that
//! CSV cut to the rows the Protocol keeps. arc's `http_fetch` does not take an
//! existing `out` as done: unless the step pins a `sha256` the file matches, or
//! a `<out>.arcmeta` sidecar gives it a validator to ask with, it GETs the URL
//! and writes the whole remote file over `out`. The fixture has neither, so a
//! run with the step in place would download 13.7 MB and replace the cut file.
//! [`copied_fixture`] therefore drops every `http_fetch` step from the COPY's
//! `arcform.yaml` (never the fixture's), and [`run_protocol`] asserts the data
//! file is byte-identical after the run, which a fetch could not leave it.
//!
//! **One store per test binary.** arc keys a Protocol's database by a hash of
//! its folder, under `ARCFORM_DB_DIR`, and keeps history under
//! `ARCFORM_HISTORY_DIR`. Both point at one temp root per test binary, set once
//! through a `OnceLock` before any test reads them, so no run touches a
//! developer's `~/.arcform` and copies in different folders cannot collide.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use brightfield_shell::run::{self, Runner};
use tempfile::TempDir;

/// A copy of a fixture Protocol in a directory the test owns, removed when it drops.
pub struct RanFixture {
    /// The copied Protocol folder.
    pub dir: PathBuf,
    _tmp: TempDir,
}

/// The data file the CO2 fixture's fetch step names, relative to the Protocol.
pub const FETCHED_DATA: &str = "data/owid-co2-data.csv";

/// The committed fixture folder `name`.
pub fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/agent-protocols")
        .join(name)
}

/// Point arc's database and history stores at this binary's temp root, and its
/// engine at the pinned CLI when the environment names none.
///
/// The variables are set before the first test that copies a fixture reads
/// them, and never again; every later caller waits on the one initialisation.
pub fn arc_stores() -> &'static Path {
    static ROOT: OnceLock<TempDir> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = tempfile::Builder::new()
            .prefix("bf-agent-protocol-")
            .tempdir()
            .expect("a temp root for arc's stores");
        let db = root.path().join("db");
        let history = root.path().join("history");
        for d in [&db, &history] {
            std::fs::create_dir_all(d).expect("an arc store directory");
        }
        std::env::set_var("ARCFORM_DB_DIR", &db);
        std::env::set_var("ARCFORM_HISTORY_DIR", &history);
        if std::env::var_os(run::ENGINE_ENV).is_none() {
            let pinned =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/duckdb-cli/duckdb");
            if pinned.is_file() {
                std::env::set_var(run::ENGINE_ENV, pinned);
            }
        }
        root
    })
    .path()
}

/// The DuckDB CLI an `arc` child runs its steps with: the one the environment
/// names, else `duckdb` on the search path.
pub fn engine_cli() -> PathBuf {
    arc_stores();
    std::env::var_os(run::ENGINE_ENV).map_or_else(|| PathBuf::from("duckdb"), PathBuf::from)
}

/// The fixture `name` copied, unrun, into a fresh temp directory as `name/`.
pub fn copied_fixture(name: &str) -> RanFixture {
    copied_fixture_in(name, name)
}

/// The fixture `name` copied, unrun, into a fresh temp directory as `folder/`,
/// with its `http_fetch` steps dropped from the copy's `arcform.yaml`.
pub fn copied_fixture_in(name: &str, folder: &str) -> RanFixture {
    arc_stores();
    let tmp = tempfile::Builder::new()
        .prefix("bf-agent-fixture-")
        .tempdir()
        .expect("a temp directory for the fixture");
    let dir = tmp.path().join(folder);
    copy_tree(&fixture_dir(name), &dir);
    drop_fetch_steps(&dir);
    RanFixture { dir, _tmp: tmp }
}

/// The fixture `name` copied and run.
pub fn ran_fixture(name: &str) -> RanFixture {
    let fixture = copied_fixture(name);
    run_protocol(&fixture.dir);
    fixture
}

/// Run `arc run` over the Protocol in `dir` through the `brightfield-shell`
/// binary cargo built, and assert it succeeded without replacing the data file.
pub fn run_protocol(dir: &Path) -> run::ArcOutput {
    run_protocol_with(dir, &["run"])
}

/// [`run_protocol`] with `--force`, so every step runs and writes the database
/// even when arc finds them all fresh, which a second run otherwise would.
pub fn force_run_protocol(dir: &Path) -> run::ArcOutput {
    run_protocol_with(dir, &["run", "--force"])
}

fn run_protocol_with(dir: &Path, args: &[&str]) -> run::ArcOutput {
    arc_stores();
    let data = dir.join(FETCHED_DATA);
    let before = std::fs::read(&data).ok();
    let out = run::arc_with_args(
        &Runner::at(env!("CARGO_BIN_EXE_brightfield-shell")),
        dir,
        args,
    );
    assert!(
        out.succeeded,
        "arc run failed in {}:\n{}\n{}",
        dir.display(),
        out.stdout,
        out.stderr
    );
    if let Some(before) = before {
        let after = std::fs::read(&data).expect("the data file is still there");
        assert!(before == after, "the run replaced {}", data.display());
    }
    out
}

/// Remove every `http_fetch` step from the `arcform.yaml` in `dir`.
fn drop_fetch_steps(dir: &Path) {
    let path = dir.join("arcform.yaml");
    let text = std::fs::read_to_string(&path).expect("the copy has an arcform.yaml");
    let mut manifest: serde_yaml::Value = serde_yaml::from_str(&text).expect("the manifest parses");
    if let Some(steps) = manifest
        .get_mut("steps")
        .and_then(serde_yaml::Value::as_sequence_mut)
    {
        steps.retain(|step| {
            step.get("op").and_then(serde_yaml::Value::as_str) != Some("http_fetch")
        });
    }
    std::fs::write(
        &path,
        serde_yaml::to_string(&manifest).expect("the manifest serialises"),
    )
    .expect("the copy's manifest writes");
}

/// Copy the directory `from` to `to`, recursively.
fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("the copy's directory");
    for entry in std::fs::read_dir(from).expect("the fixture reads") {
        let entry = entry.expect("a fixture entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("a fixture file copies");
        }
    }
}
