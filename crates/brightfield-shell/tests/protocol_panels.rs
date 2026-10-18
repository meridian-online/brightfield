//! An agent-written Protocol's panels load headless, one outcome per tile.
//!
//! Every test here reads a real run: the CO2 fixture copied into a directory of
//! the test's own and run by `arc` through the `brightfield-shell` binary cargo
//! built (`support/agent_protocol.rs` says how the fetch step is kept off the
//! network). What a tile holds is either the composed chart or the one problem
//! that kept it from drawing, and the other tiles draw regardless.

#[path = "support/agent_protocol.rs"]
mod agent_protocol;

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use agent_protocol::{
    copied_fixture, copied_fixture_in, drop_fetch_steps, engine_cli, force_run_protocol,
    is_fetch_op, ran_fixture, run_protocol,
};
use brightfield_model::panel_capture::panel_file;
use brightfield_shell::protocol_panels::{
    agent_panels_in, load, with_protocol_tables, OpenProblem, PanelProblem, ProtocolPanels,
};
use brightfield_spec::layout::Rect;

const CO2: &str = "co2-per-capita";
const CHANGE: &str = "panels/co2_per_capita_change_pct.yaml";
const SINCE: &str = "panels/co2_per_capita_since_1990.yaml";

/// The box a chart is composed into: none, so each panel takes the size it
/// declares, as a chart opened in the window does before its pane reports one.
fn viewport() -> Rect {
    Rect::zero()
}

fn loaded(dir: &Path) -> ProtocolPanels {
    match load(dir, viewport()) {
        Ok(panels) => panels,
        Err(problem) => panic!("the Protocol did not open: {}", problem.message()),
    }
}

/// The tile for the panel file `file`.
fn tile<'a>(
    panels: &'a ProtocolPanels,
    file: &str,
) -> &'a brightfield_shell::protocol_panels::PanelTile {
    panels
        .tiles
        .iter()
        .find(|t| t.file == Path::new(file))
        .unwrap_or_else(|| panic!("no tile for {file}"))
}

fn assert_draws(panels: &ProtocolPanels, file: &str) {
    if let Err(problems) = &tile(panels, file).outcome {
        let messages: Vec<String> = problems.iter().map(PanelProblem::message).collect();
        panic!("{file} did not draw: {}", messages.join("; "));
    }
}

/// Every problem the tile for `file` holds.
fn problems<'a>(panels: &'a ProtocolPanels, file: &str) -> &'a [PanelProblem] {
    match &tile(panels, file).outcome {
        Ok(_) => panic!("{file} drew, but a problem was expected"),
        Err(problems) => problems,
    }
}

fn edit(path: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(path).expect("the file reads");
    assert!(text.contains(from), "{} has no {from:?}", path.display());
    std::fs::write(path, text.replace(from, to)).expect("the file writes");
}

fn set_db(dir: &Path, db: &str) {
    let manifest = dir.join("arcform.yaml");
    let mut text = std::fs::read_to_string(&manifest).expect("the manifest reads");
    text.push_str(&format!("db: {db}\n"));
    std::fs::write(&manifest, text).expect("the manifest writes");
}

#[test]
fn the_co2_protocol_draws_both_panels() {
    let fixture = ran_fixture(CO2);
    let panels = loaded(&fixture.dir);

    let files: Vec<&Path> = panels.tiles.iter().map(|t| t.file.as_path()).collect();
    assert_eq!(files, [Path::new(CHANGE), Path::new(SINCE)]);
    assert_draws(&panels, CHANGE);
    assert_draws(&panels, SINCE);
    assert_eq!(
        tile(&panels, CHANGE).title.as_deref(),
        Some("Change in per-capita CO2, 1990 to 2024 (percent)")
    );
    assert_eq!(
        tile(&panels, SINCE).title.as_deref(),
        Some("Per-capita CO2 since 1990, G7 countries vs China and India (tonnes per person)")
    );
    let Ok(line) = &tile(&panels, SINCE).outcome else {
        unreachable!()
    };
    assert_eq!(line.plots.len(), 1);
    assert_eq!(panels.name, "co2-per-capita");
    assert!(panels.run_id.is_some(), "the newest run's id is read");
    assert_eq!(
        tile(&panels, CHANGE).source,
        std::fs::read_to_string(fixture.dir.join(CHANGE)).unwrap()
    );
}

#[test]
fn a_missing_column_is_its_tiles_message() {
    let fixture = ran_fixture(CO2);
    edit(
        &fixture.dir.join(CHANGE),
        "x: change_pct",
        "x: change_percent",
    );
    let panels = loaded(&fixture.dir);

    assert_eq!(
        problems(&panels, CHANGE),
        [PanelProblem::MissingColumn {
            from: "co2_per_capita_change".into(),
            column: "change_percent".into(),
        }]
    );
    assert!(problems(&panels, CHANGE)[0]
        .message()
        .contains("change_percent"));
    assert_draws(&panels, SINCE);
}

#[test]
fn a_tile_holds_every_problem_in_its_panel() {
    let fixture = ran_fixture(CO2);
    let change = fixture.dir.join(CHANGE);
    edit(&change, "x: change_pct", "x: change_percent");
    edit(&change, "y: country", "y: nation");
    let panels = loaded(&fixture.dir);

    assert_eq!(
        problems(&panels, CHANGE),
        [
            PanelProblem::MissingColumn {
                from: "co2_per_capita_change".into(),
                column: "change_percent".into(),
            },
            PanelProblem::MissingColumn {
                from: "co2_per_capita_change".into(),
                column: "nation".into(),
            },
        ]
    );
    assert_draws(&panels, SINCE);
}

/// DuckDB and arc's check both match table names without regard to case, so
/// `CO2_Per_Capita_Change` and `co2_per_capita_change` are one table: copied
/// once, to one Parquet file, which both spellings read.
#[test]
fn two_spellings_of_one_table_both_draw() {
    let fixture = ran_fixture(CO2);
    edit(
        &fixture.dir.join(CHANGE),
        "from: co2_per_capita_change",
        "from: CO2_Per_Capita_Change",
    );
    std::fs::write(
        fixture.dir.join("panels/mixed_case.yaml"),
        "\
plot:
  - mark: barX
    data: { from: co2_per_capita_change }
    x: change_pct
    y: country
  - mark: dot
    data: { from: CO2_PER_CAPITA_CHANGE }
    x: change_pct
    y: country
",
    )
    .unwrap();
    let panels = loaded(&fixture.dir);

    assert_draws(&panels, CHANGE);
    assert_draws(&panels, "panels/mixed_case.yaml");
    assert_draws(&panels, SINCE);
    let snapshot: Vec<String> = std::fs::read_dir(panels.tables_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("co2_per_capita_change"))
        .collect();
    assert_eq!(snapshot, ["co2_per_capita_change.parquet"]);
}

#[test]
fn an_unsupported_mark_is_its_tiles_message() {
    let fixture = ran_fixture(CO2);
    edit(&fixture.dir.join(CHANGE), "mark: barX", "mark: waffleY");
    let panels = loaded(&fixture.dir);

    assert_eq!(
        problems(&panels, CHANGE),
        [PanelProblem::Unsupported(vec!["waffleY".into()])]
    );
    assert!(problems(&panels, CHANGE)[0].message().contains("waffleY"));
    assert_draws(&panels, SINCE);
}

#[test]
fn a_panel_that_is_not_yaml_is_its_tiles_message() {
    let fixture = ran_fixture(CO2);
    std::fs::write(
        fixture.dir.join("panels/broken.yaml"),
        "plot: [ { mark: dot\n",
    )
    .unwrap();
    let panels = loaded(&fixture.dir);

    assert_eq!(panels.tiles.len(), 3);
    assert!(matches!(
        problems(&panels, "panels/broken.yaml"),
        [PanelProblem::Unreadable(_)]
    ));
    assert_draws(&panels, CHANGE);
    assert_draws(&panels, SINCE);
}

/// Panels other than the one chart file Brightfield writes are an agent's.
#[test]
fn agent_panels_are_panels_brightfield_did_not_write() {
    let fixture = copied_fixture(CO2);
    let dir = &fixture.dir;
    let panels_dir = dir.join("panels");
    std::fs::remove_dir_all(&panels_dir).unwrap();
    assert!(!agent_panels_in(dir), "no panels/ folder");

    std::fs::create_dir_all(&panels_dir).unwrap();
    let own = panel_file(dir, "co2-per-capita");
    std::fs::write(&own, "plot:\n  - mark: dot\n").unwrap();
    assert!(!agent_panels_in(dir), "only {} is there", own.display());

    std::fs::write(panels_dir.join("other.yaml"), "plot:\n  - mark: dot\n").unwrap();
    assert!(agent_panels_in(dir), "another panel is there");
}

#[test]
fn a_protocol_that_has_not_run_says_so() {
    let fixture = copied_fixture(CO2);
    match load(&fixture.dir, viewport()) {
        Err(problem @ OpenProblem::NotRun { .. }) => {
            assert!(problem.message().contains("run"), "{}", problem.message());
        }
        Err(other) => panic!("expected NotRun, got {}", other.message()),
        Ok(_) => panic!("an unrun Protocol opened"),
    }
}

/// The database held for writing by another process, as `arc run` holds it:
/// the DuckDB CLI arc runs its steps with, open on the file and waiting on its
/// stdin. A read-write `duckdb::Connection` in this test's own process would
/// not do: DuckDB's lock is a POSIX record lock, which a process never
/// conflicts with itself on, so the attach in `load` succeeded beside it.
#[test]
fn a_locked_database_says_the_protocol_is_running() {
    let fixture = ran_fixture(CO2);
    let db = arc::spec::protocol_db_path(&fixture.dir).unwrap();
    let mut held = Command::new(engine_cli())
        .arg(&db)
        .args(["-noheader", "-list"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the DuckDB CLI starts");
    let mut stdin = held.stdin.take().unwrap();
    writeln!(stdin, "SELECT 'holding';").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    BufReader::new(held.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "holding", "the CLI opened the database");

    let outcome = load(&fixture.dir, viewport());
    drop(stdin);
    let _ = held.kill();
    let _ = held.wait();
    match outcome {
        Err(problem @ OpenProblem::Locked { .. }) => {
            assert!(
                problem.message().contains("being run"),
                "{}",
                problem.message()
            );
        }
        Err(other) => panic!("expected Locked, got {}", other.message()),
        Ok(_) => panic!("the Protocol opened under a writer's lock"),
    }
}

/// Open tiles keep drawing through a reload that meets a run's lock, and pick
/// up the edit that prompted it once the lock is gone.
#[test]
fn a_reload_under_a_runs_lock_keeps_the_tiles_and_tries_again() {
    use brightfield_shell::panel_tiles::{PanelTiles, RUN_CHECK_EVERY};
    let fixture = ran_fixture(CO2);
    let mut tiles = PanelTiles::new(loaded(&fixture.dir), egui::vec2(400.0, 300.0));
    let db = arc::spec::protocol_db_path(&fixture.dir).unwrap();
    let mut held = Command::new(engine_cli())
        .arg(&db)
        .args(["-noheader", "-list"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the DuckDB CLI starts");
    let mut stdin = held.stdin.take().unwrap();
    writeln!(stdin, "SELECT 'holding';").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    BufReader::new(held.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "holding", "the CLI opened the database");

    // An edit to a panel while the run holds the database.
    let panel = fixture.dir.join(CHANGE);
    let text = std::fs::read_to_string(&panel).unwrap();
    std::fs::write(&panel, format!("{text}\n# edited\n")).unwrap();
    std::thread::sleep(RUN_CHECK_EVERY);
    assert!(tiles.reload_due(), "the edit is seen");
    tiles.reload();
    let kept = tiles.panels().is_some();
    drop(stdin);
    let _ = held.kill();
    let _ = held.wait();
    assert!(
        kept,
        "the tiles went to {:?}",
        tiles.problem().map(OpenProblem::message)
    );

    std::thread::sleep(RUN_CHECK_EVERY);
    assert!(
        tiles.reload_due(),
        "the edit the locked reload could not read is still due"
    );
    tiles.reload();
    assert!(
        tiles.panels().is_some(),
        "the tiles open once the run is done"
    );
    std::thread::sleep(RUN_CHECK_EVERY);
    assert!(!tiles.reload_due(), "and the edit is taken as read");
}

/// A run whose record lands while it still holds the database, with no panel
/// file changed: the reload meets the lock, and the tiles still move to the
/// new run once the lock is gone and the retry comes round.
#[test]
fn a_new_run_met_under_its_lock_is_drawn_once_the_lock_is_gone() {
    use brightfield_shell::panel_tiles::{PanelTiles, RUN_CHECK_EVERY};
    let fixture = ran_fixture(CO2);
    let mut tiles = PanelTiles::new(loaded(&fixture.dir), egui::vec2(400.0, 300.0))
        .with_locked_retry_every(std::time::Duration::ZERO);
    let first = tiles.panels().and_then(|p| p.run_id.clone());

    force_run_protocol(&fixture.dir);
    let db = arc::spec::protocol_db_path(&fixture.dir).unwrap();
    let mut held = Command::new(engine_cli())
        .arg(&db)
        .args(["-noheader", "-list"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the DuckDB CLI starts");
    let mut stdin = held.stdin.take().unwrap();
    writeln!(stdin, "SELECT 'holding';").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    BufReader::new(held.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line.trim(), "holding", "the CLI opened the database");

    std::thread::sleep(RUN_CHECK_EVERY);
    assert!(tiles.reload_due(), "the new run's record is seen");
    tiles.reload();
    let kept = tiles.panels().and_then(|p| p.run_id.clone());
    drop(stdin);
    let _ = held.kill();
    let _ = held.wait();
    assert_eq!(
        kept, first,
        "the tiles keep the run they drew under the lock"
    );

    std::thread::sleep(RUN_CHECK_EVERY);
    assert!(tiles.reload_due(), "the refused reload is tried again");
    tiles.reload();
    let now = tiles.panels().and_then(|p| p.run_id.clone());
    assert!(
        now.is_some() && now != first,
        "the tiles draw the new run: {now:?}"
    );
}

#[test]
fn a_folder_with_a_space_and_a_quote_opens() {
    let fixture = copied_fixture_in(CO2, "it's a test");
    run_protocol(&fixture.dir);
    let panels = loaded(&fixture.dir);

    assert_eq!(panels.tiles.len(), 2);
    assert_draws(&panels, CHANGE);
    assert_draws(&panels, SINCE);
}

#[test]
fn only_yaml_files_in_panels_are_tiles() {
    let fixture = ran_fixture(CO2);
    let panels_dir = fixture.dir.join("panels");
    let extra = std::fs::read_to_string(panels_dir.join("co2_per_capita_change_pct.yaml")).unwrap();
    std::fs::write(panels_dir.join("x.yml"), &extra).unwrap();
    std::fs::write(panels_dir.join(".hidden.yaml"), &extra).unwrap();
    std::fs::create_dir_all(panels_dir.join("sub")).unwrap();
    std::fs::write(panels_dir.join("sub/y.yaml"), &extra).unwrap();
    let panels = loaded(&fixture.dir);

    let files: Vec<PathBuf> = panels.tiles.iter().map(|t| t.file.clone()).collect();
    let (_, checked) = arc::spec::check_panels(&fixture.dir).unwrap();
    let named: Vec<PathBuf> = checked.into_iter().map(|(path, _)| path).collect();
    assert_eq!(files, named);
    // arc's check reads a dotfile with a `.yaml` extension as a panel, and its
    // report lists it, so it is a tile here too; the `.yml` file and the one in
    // a subfolder are neither.
    assert_eq!(
        files,
        [
            PathBuf::from("panels/.hidden.yaml"),
            PathBuf::from(CHANGE),
            PathBuf::from(SINCE)
        ]
    );
}

#[test]
fn an_explicit_db_is_read() {
    let fixture = copied_fixture(CO2);
    set_db(&fixture.dir, "local.duckdb");
    run_protocol(&fixture.dir);
    let panels = loaded(&fixture.dir);

    assert_eq!(panels.db, fixture.dir.join("local.duckdb"));
    assert_draws(&panels, CHANGE);
    assert_draws(&panels, SINCE);
}

#[test]
fn with_protocol_tables_leaves_inline_data_alone() {
    let panel = "\
data:
  t:
    - { k: a, v: 1 }
    - { k: b, v: 2 }
plot:
  - mark: dot
    data: { from: t }
    x: v
    y: k
  - mark: barX
    data: { from: co2_per_capita_change }
    x: change_pct
    y: country
";
    let tables = Path::new("/somewhere/tables");
    let written = with_protocol_tables(panel, tables).expect("the panel is YAML");
    let before: serde_yaml::Value = serde_yaml::from_str(panel).unwrap();
    let after: serde_yaml::Value = serde_yaml::from_str(&written).unwrap();

    let data = after["data"].as_mapping().expect("a data mapping");
    assert_eq!(data.len(), 2, "{written}");
    assert_eq!(after["data"]["t"], before["data"]["t"]);
    assert_eq!(
        after["data"]["co2_per_capita_change"]["file"].as_str(),
        Some(
            tables
                .join("co2_per_capita_change.parquet")
                .to_str()
                .unwrap()
        )
    );
    assert_eq!(after["plot"], before["plot"]);
}

#[test]
fn arc_can_run_while_the_protocol_is_open() {
    let fixture = ran_fixture(CO2);
    let panels = loaded(&fixture.dir);

    // Forced: a second plain run finds every step fresh and opens nothing.
    let out = force_run_protocol(&fixture.dir);
    assert!(!out.stdout.contains("skipped"), "{}", out.stdout);
    let db = duckdb::Connection::open(&panels.db).expect("a writer opens the database");
    drop(db);
    assert_draws(&panels, CHANGE);
    drop(panels);
}

/// DuckDB's checksum of a block: 5381 xor each little-endian u64 word times a
/// constant, over the block after its own eight checksum bytes.
fn duckdb_block_checksum(block: &[u8]) -> u64 {
    block.chunks_exact(8).fold(5381u64, |sum, word| {
        sum ^ u64::from_le_bytes(word.try_into().unwrap()).wrapping_mul(0xbf58_476d_1ce4_e5b9)
    })
}

#[test]
fn a_database_from_another_duckdb_says_so() {
    let fixture = copied_fixture(CO2);

    // A file whose header says a storage version from a later DuckDB: written
    // by this DuckDB, then its version word (bytes 12..20 of the main header)
    // raised and the header's checksum (bytes 0..8) made to match.
    let newer = fixture.dir.join("newer.duckdb");
    {
        let conn = duckdb::Connection::open(&newer).unwrap();
        conn.execute_batch("CREATE TABLE co2_per_capita_change AS SELECT 1 AS x")
            .unwrap();
    }
    let mut bytes = std::fs::read(&newer).unwrap();
    bytes[12..20].copy_from_slice(&99u64.to_le_bytes());
    let sum = duckdb_block_checksum(&bytes[8..4096]);
    bytes[0..8].copy_from_slice(&sum.to_le_bytes());
    std::fs::write(&newer, bytes).unwrap();
    set_db(&fixture.dir, "newer.duckdb");

    match load(&fixture.dir, viewport()) {
        Err(problem @ OpenProblem::EngineMismatch { .. }) => {
            let message = problem.message();
            assert!(message.contains("newer.duckdb"), "{message}");
            assert!(message.contains("99"), "{message}");
        }
        Err(other) => panic!("expected EngineMismatch, got {}", other.message()),
        Ok(_) => panic!("a database from a later DuckDB opened"),
    }

    // Garbage bytes are no DuckDB's: the answer names the file, and nothing panics.
    let garbage = fixture.dir.join("garbage.duckdb");
    std::fs::write(&garbage, b"not a database at all\n".repeat(400)).unwrap();
    edit(
        &fixture.dir.join("arcform.yaml"),
        "db: newer.duckdb",
        "db: garbage.duckdb",
    );
    match load(&fixture.dir, viewport()) {
        Err(problem) => {
            assert!(
                problem.message().contains("garbage.duckdb"),
                "{}",
                problem.message()
            );
        }
        Ok(_) => panic!("garbage opened as a database"),
    }
}

#[test]
fn a_copy_drops_every_version_of_a_fetch_step_and_keeps_the_rest() {
    assert!(is_fetch_op("http_fetch"));
    assert!(is_fetch_op("http_fetch@1"));
    assert!(is_fetch_op("http_fetch@12"));
    assert!(!is_fetch_op("http_fetch@"));
    assert!(!is_fetch_op("http_fetch@x"));
    assert!(!is_fetch_op("http_fetcher"));
    assert!(!is_fetch_op("sql"));

    let tmp = tempfile::tempdir().expect("a temp directory");
    std::fs::write(
        tmp.path().join("arcform.yaml"),
        "name: p\nsteps:\n\
         - name: plain\n  op: http_fetch\n  with: { out: data/a.csv, url: https://example.invalid/a }\n\
         - name: versioned\n  op: http_fetch@1\n  with: { out: data/b.csv, url: https://example.invalid/b }\n\
         - name: load\n  sql: models/load.sql\n",
    )
    .expect("the manifest writes");
    drop_fetch_steps(tmp.path());
    let manifest: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(tmp.path().join("arcform.yaml")).expect("the manifest reads"),
    )
    .expect("the manifest parses");
    let names: Vec<&str> = manifest["steps"]
        .as_sequence()
        .expect("a step list")
        .iter()
        .filter_map(|step| step["name"].as_str())
        .collect();
    assert_eq!(
        names,
        ["load"],
        "both fetch steps went and the SQL step stayed"
    );
}
