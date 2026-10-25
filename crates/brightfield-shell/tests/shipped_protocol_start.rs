//! A start that opens a shipped Protocol writes its folder, runs it with the
//! embedded arc runner when it has not run, and lands on its first panel
//! beside its grid.
//!
//! The Protocol is a small committed fixture (`tests/data/shipped-protocol/`:
//! one bundled CSV, one SQL step, one panel), handed to the window and to
//! `Boot` as a start's own would be. The runs are real: `arc` through the
//! `brightfield-shell` binary cargo built, with its stores in a temp root
//! (`support/agent_protocol.rs`). Nothing here leaves the loopback interface:
//! the start that fetches is pointed at a port with nothing listening, the way
//! `remote_start.rs` points a fetch.

#[path = "support/agent_protocol.rs"]
mod agent_protocol;
#[path = "support/shipped_protocol.rs"]
mod shipped_protocol;

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::run::Runner;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

/// The id the tests take the fixture's start under. Not a shipped start's.
const ID: &str = "shipped-sample";

/// How long a run may take before a test calls it hung.
const PATIENCE: Duration = Duration::from_secs(180);

/// The config directory, and with it the datasets directory, is
/// process-wide: tests take turns with it.
static CONFIG: Mutex<()> = Mutex::new(());

/// Point the datasets directory at a scratch directory of this test's own,
/// emptied first, for as long as the guard is held.
#[must_use = "the directory is only this test's while the guard is held"]
fn datasets_into_scratch(test: &str) -> MutexGuard<'static, ()> {
    let held = CONFIG.lock().unwrap_or_else(PoisonError::into_inner);
    agent_protocol::arc_stores();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("shipped_protocol_start")
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch config directory");
    std::env::set_var(brightfield_shell::startup::CONFIG_DIR_VAR, &dir);
    held
}

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

#[cfg(unix)]
/// A runner that can find no DuckDB CLI: the `brightfield-shell` binary
/// started with `ARC_DUCKDB_BIN` unset and an empty directory as its search
/// path.
fn runner_without_duckdb(scratch: &Path) -> Runner {
    use std::os::unix::fs::PermissionsExt;
    let empty = scratch.join("empty-path");
    std::fs::create_dir_all(&empty).unwrap();
    let script = scratch.join("runner-without-duckdb");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nexec /usr/bin/env -u {} PATH={} {} \"$@\"\n",
            brightfield_shell::run::ENGINE_ENV,
            empty.display(),
            env!("CARGO_BIN_EXE_brightfield-shell")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    Runner::at(script)
}

/// A URL on the loopback interface with nothing listening: a connection to
/// it is refused rather than routed off the machine.
fn nothing_listening(path: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    drop(listener);
    format!("http://{addr}/{path}")
}

/// A device-free window over the front door, with a runner.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
}

impl Window {
    fn open(runner: Runner) -> Self {
        Self::over(Boot::empty(), runner)
    }

    /// A window over `boot`, with a runner.
    fn over(boot: Boot, runner: Runner) -> Self {
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(runner)),
            ctx: egui::Context::default(),
        };
        win.settle();
        win
    }

    fn frame(&mut self) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 820.0),
            )),
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame();
        }
    }

    /// Take the start over `protocol`, and drive frames until its run lands.
    fn take_and_land(&mut self, protocol: &'static starts::ShippedProtocol) {
        let ctx = self.ctx.clone();
        self.app.open_shipped_protocol(&ctx, ID, protocol);
        let deadline = Instant::now() + PATIENCE;
        while self.app.run_outstanding() {
            assert!(Instant::now() < deadline, "the start's run never landed");
            std::thread::sleep(Duration::from_millis(50));
            self.frame();
        }
        self.settle();
    }

    /// Each raised banner, as its title and body.
    fn banners(&self) -> Vec<String> {
        self.app
            .notifications()
            .iter()
            .map(|n| format!("{} {}", n.title, n.body.clone().unwrap_or_default()))
            .collect()
    }
}

/// Whether the Protocol in `dir` has a database on disk.
fn has_database(dir: &Path) -> bool {
    arc::spec::protocol_db_path(dir).is_ok_and(|db| db.is_file())
}

/// **The first click runs the Protocol and lands on its panel beside its
/// grid**, not on the tiles: the folder is written where the start keeps it,
/// the run is under way after the click with the window drawing, and when it
/// lands the first panel is the chart document with the Data pane beside it
/// listing the table its marks read.
#[test]
fn a_shipped_protocol_start_runs_and_lands_on_its_panel_beside_its_grid() {
    let _config = datasets_into_scratch("lands");
    let protocol = shipped_protocol::fixture("shipped-sample", 1);
    let mut win = Window::open(runner());
    let ctx = win.ctx.clone();
    win.app.open_shipped_protocol(&ctx, ID, protocol);
    assert!(
        win.app.run_outstanding(),
        "a Protocol that has not run was not run"
    );
    let dir = starts::protocol_path(protocol);
    assert!(
        dir.join("arcform.yaml").is_file(),
        "the folder was not written"
    );

    let deadline = Instant::now() + PATIENCE;
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run never landed");
        std::thread::sleep(Duration::from_millis(50));
        win.frame();
    }
    win.settle();

    assert!(win.banners().is_empty(), "{:?}", win.banners());
    assert_eq!(
        win.app.panel_tiles().and_then(|t| t.selected()),
        Some(0),
        "the run landed on the tiles rather than on the first panel"
    );
    let panes = win.app.canvas_panes();
    assert!(
        panes.pane("map").is_some() && panes.pane("grid").is_some(),
        "the canvas did not draw the panel beside its grid"
    );
    let drawn = win
        .app
        .chart_doc()
        .grid_drawn()
        .expect("the grid drew a table");
    assert_eq!(
        (drawn.rows, drawn.columns),
        (12, 3),
        "the grid lists another table"
    );
    assert!(has_database(&dir), "the run made no database");
    assert!(!starts::needs_run(&dir), "a landed run reads as not run");

    // A second click finds the run and opens the panel at once, with no run.
    let mut again = Window::open(runner());
    let ctx = again.ctx.clone();
    again.app.open_shipped_protocol(&ctx, ID, protocol);
    assert!(
        !again.app.run_outstanding(),
        "a Protocol that has run was run again"
    );
    again.settle();
    assert_eq!(again.app.panel_tiles().and_then(|t| t.selected()), Some(0));
}

/// **A changed revision is written afresh**, beside the older folder rather
/// than over it, and its first open runs it rather than reopening the older
/// folder's files or the database the older run made. A second write of the
/// same revision leaves the folder alone.
#[test]
fn a_shipped_protocol_of_a_new_revision_is_written_afresh() {
    let _config = datasets_into_scratch("revision");
    let older = shipped_protocol::fixture("shipped-sample", 1);
    let older_dir = starts::write_protocol(older).expect("the older revision writes");
    Boot::shipped_protocol(ID, older, Flow::Vertical, Some(&runner()))
        .expect("the older revision runs and opens");
    assert!(has_database(&older_dir));

    // The same revision again: the folder, and what its run left, stay.
    let marker = older_dir.join("panels/overview.yaml");
    std::fs::write(&marker, "title: written by an earlier open\n").unwrap();
    assert_eq!(starts::write_protocol(older).unwrap(), older_dir);
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        "title: written by an earlier open\n",
        "a second write of the same revision replaced the folder"
    );

    // A new revision, whose panel says something else.
    let mut files = shipped_protocol::files_of(older);
    for (path, bytes) in &mut files {
        if path == "panels/overview.yaml" {
            let text = String::from_utf8(bytes.clone()).unwrap();
            *bytes = text
                .replace(
                    "Body mass against flipper length",
                    "Revised: mass by flipper",
                )
                .into_bytes();
        }
    }
    let newer = shipped_protocol::shipped("shipped-sample", 2, files);
    let newer_dir = starts::write_protocol(newer).expect("the new revision writes");
    assert_ne!(
        newer_dir, older_dir,
        "the new revision was written over the older one"
    );
    assert!(
        std::fs::read_to_string(newer_dir.join("panels/overview.yaml"))
            .unwrap()
            .contains("Revised: mass by flipper"),
        "the new revision's folder holds the older files"
    );
    assert!(
        starts::needs_run(&newer_dir),
        "the new revision read as run off the older revision's database"
    );
    let boot = Boot::shipped_protocol(ID, newer, Flow::Vertical, Some(&runner()))
        .expect("the new revision runs and opens");
    let panels = boot
        .panels
        .as_ref()
        .expect("the new revision opened on its panels");
    assert!(panels.tiles[0].source.contains("Revised: mass by flipper"));
    assert_ne!(panels.db, arc::spec::protocol_db_path(&older_dir).unwrap());
}

/// **No network on the first click of a fetched Protocol start**: the banner
/// names the network and the URL, the window stays up on the door, and the
/// failed run leaves no database, so the next open runs it again rather than
/// reopening what a failed run left.
#[test]
fn a_fetched_protocol_start_with_no_network_raises_a_banner_naming_the_url_and_leaves_nothing_to_reopen(
) {
    let _config = datasets_into_scratch("no_network");
    let url = nothing_listening("measures.csv");
    let protocol = shipped_protocol::fetching_fixture("shipped-fetching", &url);
    assert_eq!(
        starts::fetch_urls(
            std::str::from_utf8(
                protocol
                    .files
                    .iter()
                    .find(|f| f.0 == "arcform.yaml")
                    .unwrap()
                    .1
            )
            .unwrap()
        ),
        vec![url.clone()]
    );
    let mut win = Window::open(runner());
    win.take_and_land(protocol);

    let banners = win.banners();
    assert_eq!(banners.len(), 1, "{banners:?}");
    assert!(banners[0].contains("network"), "{}", banners[0]);
    assert!(banners[0].contains(&url), "{}", banners[0]);
    assert!(
        win.app.front_door_is_live(),
        "the window did not go back to the door over the refusal"
    );
    let dir = starts::protocol_path(protocol);
    assert!(!has_database(&dir), "the failed run left its database");
    assert!(
        starts::needs_run(&dir),
        "the next open would trust what the failed run left"
    );
    assert!(
        !dir.join(shipped_protocol::DATA_FILE).exists(),
        "the failed fetch left a data file"
    );
}

/// **The runner cannot find a DuckDB CLI**: the start fails with a banner that
/// says so, over the door, rather than leaving a Protocol at *not run*.
#[cfg(unix)]
#[test]
fn a_shipped_protocol_start_with_no_duckdb_cli_says_so() {
    let _config = datasets_into_scratch("no_duckdb");
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("shipped_protocol_start")
        .join("no_duckdb");
    let protocol = shipped_protocol::fixture("shipped-sample", 1);
    let mut win = Window::open(runner_without_duckdb(&scratch));
    win.take_and_land(protocol);

    let banners = win.banners();
    assert_eq!(banners.len(), 1, "{banners:?}");
    // arc's own words for an engine the search path cannot find.
    assert!(
        banners[0].contains("engine 'duckdb' not found"),
        "the banner does not say DuckDB could not be found: {}",
        banners[0]
    );
    assert!(
        win.app.front_door_is_live(),
        "the window was left on the tiles"
    );
    assert!(win.app.panel_tiles().is_none());
    assert!(!has_database(&starts::protocol_path(protocol)));
}

/// **A fetched Protocol start boots offline from its stand-in**: `Boot` runs
/// the variant whose `http_fetch` steps are replaced by committed copies to
/// completion, and draws its panels, while the URL's port is closed.
#[test]
fn a_fetched_protocol_start_boots_from_its_stand_in_with_no_network() {
    let _config = datasets_into_scratch("stand_in");
    let url = nothing_listening("measures.csv");
    let protocol = shipped_protocol::fetching_fixture("shipped-fetching", &url);
    let copy = shipped_protocol::fixture_dir().join(shipped_protocol::DATA_FILE);
    let offline = shipped_protocol::stand_in(protocol, &[(url.as_str(), copy.as_path())]);
    let boot = Boot::shipped_protocol(ID, offline, Flow::Vertical, Some(&runner()))
        .expect("the stand-in runs and opens");
    let panels = boot
        .panels
        .as_ref()
        .expect("the stand-in opened on its panels");
    assert_eq!(panels.tiles.len(), 1);
    assert!(panels.tiles[0].outcome.is_ok(), "the panel did not draw");
}

/// **`Boot` of a Protocol whose run fails is an error, not the not-run
/// state**: a thumbnail or a hermetic test that boots it fails by name.
#[test]
fn a_boot_of_a_protocol_whose_run_fails_says_why() {
    let _config = datasets_into_scratch("boot_fails");
    let url = nothing_listening("measures.csv");
    let protocol = shipped_protocol::fetching_fixture("shipped-fetching", &url);
    let Err(e) = Boot::shipped_protocol(ID, protocol, Flow::Vertical, Some(&runner())) else {
        panic!("a Protocol whose fetch could not connect booted");
    };
    assert!(e.contains(&url) && e.contains("network"), "{e}");
    assert!(!has_database(&starts::protocol_path(protocol)));
}

/// The fixture with `step` (a name) and its SQL added as the last step.
fn with_step(dir_name: &str, step: &str, sql: &str) -> &'static starts::ShippedProtocol {
    let mut files = shipped_protocol::files_of(shipped_protocol::fixture(dir_name, 1));
    for (path, bytes) in &mut files {
        if path == "arcform.yaml" {
            let mut text = String::from_utf8(bytes.clone()).unwrap();
            text.push_str(&format!("  - name: {step}\n    sql: models/{step}.sql\n"));
            *bytes = text.into_bytes();
        }
    }
    files.push((format!("models/{step}.sql"), sql.as_bytes().to_vec()));
    shipped_protocol::shipped(dir_name, 1, files)
}

/// **A folder whose newest run failed is run again from nothing**: its record
/// does not say `success`, so the tables in its database are not a finished run's, and
/// the database is removed rather than reopened.
#[test]
fn a_shipped_protocol_whose_last_run_failed_is_run_again_from_nothing() {
    let _config = datasets_into_scratch("last_run_failed");
    let protocol = with_step(
        "shipped-broken",
        "broken",
        "CREATE OR REPLACE TABLE \"broken\" AS SELECT * FROM no_such_table\n",
    );
    let dir = starts::write_protocol(protocol).expect("the folder writes");
    let out = brightfield_shell::run::arc_with_args(&runner(), &dir, &["run"]);
    assert!(!out.succeeded, "the broken step ran");
    assert!(has_database(&dir), "the failed run made no database");
    let newest = brightfield_shell::run::records_newest_first(&dir, "shipped-sample")
        .next()
        .expect("the failed run wrote a record");
    // arc records a run with one good step and one failed one as partial.
    assert_ne!(
        newest.run.outcome,
        brightfield_protocol::contract::Outcome::Success,
        "the broken step ran, so this test proves nothing"
    );

    assert!(
        starts::needs_run(&dir),
        "a folder whose newest run failed reads as run"
    );
    assert!(
        !has_database(&dir),
        "the failed run's database was kept for the next open"
    );
}

/// **A second click while the first click's run is going lands on the panel**:
/// it does not remove the database that run is writing, and when the run
/// lands the window opens its first panel.
#[test]
fn a_second_click_while_the_run_is_going_lands_on_the_panel() {
    let _config = datasets_into_scratch("second_click");
    // A step slow enough for the second click to come while it runs.
    let protocol = with_step(
        "shipped-slow",
        "slow",
        "CREATE OR REPLACE TABLE \"slow\" AS SELECT sum(i) AS s FROM range(3000000000) t(i)\n",
    );
    let dir = starts::protocol_path(protocol);
    let mut win = Window::open(runner());
    let ctx = win.ctx.clone();
    win.app.open_shipped_protocol(&ctx, ID, protocol);
    assert!(win.app.run_outstanding());
    // Wait, drawing no frame so the run cannot land, until the run has made
    // its database and not yet written its record.
    let deadline = Instant::now() + PATIENCE;
    while !has_database(&dir) {
        assert!(Instant::now() < deadline, "the run never made its database");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        brightfield_shell::run::record_paths_newest_first(&dir).is_empty(),
        "the run finished before the second click, so this test proves nothing"
    );
    win.app.open_shipped_protocol(&ctx, ID, protocol);
    assert!(
        has_database(&dir),
        "the second click removed the database the run is writing"
    );
    let deadline = Instant::now() + PATIENCE;
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run never landed");
        std::thread::sleep(Duration::from_millis(50));
        win.frame();
    }
    win.settle();
    assert!(win.banners().is_empty(), "{:?}", win.banners());
    assert_eq!(
        win.app.panel_tiles().and_then(|t| t.selected()),
        Some(0),
        "the run landed somewhere other than the first panel"
    );
}

/// **A landing does not open the panel over unsaved edits**: a grid order the
/// reader applied while the run was going is still there when it lands, the
/// tiles stay, and the panel-unsaved warning says why.
#[test]
fn a_landing_over_unsaved_edits_keeps_them_and_says_so() {
    let _config = datasets_into_scratch("unsaved");
    let protocol = shipped_protocol::fixture("shipped-sample", 1);
    let mut win = Window::open(runner());
    let ctx = win.ctx.clone();
    win.app.open_shipped_protocol(&ctx, ID, protocol);
    assert!(win.app.run_outstanding());
    win.app
        .chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: None,
            order: Some("mass_g".into()),
        });
    assert!(win.app.chart_doc().has_unsaved_work());
    let deadline = Instant::now() + PATIENCE;
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run never landed");
        std::thread::sleep(Duration::from_millis(50));
        win.frame();
    }
    win.settle();
    assert!(
        win.app.chart_doc().has_unsaved_work(),
        "the landing dropped the reader's edits"
    );
    assert_eq!(win.app.panel_tiles().and_then(|t| t.selected()), None);
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title.contains("unsaved edits")),
        "no banner says why the panel did not open: {:?}",
        win.banners()
    );
    assert_ne!(
        win.app.layout().opened.as_deref(),
        Some(ID),
        "the start was recorded as landed on a panel it did not open"
    );
}

/// **A folder an earlier write left part-made is written again**: a target
/// with no manifest is replaced rather than failing every later open, and a
/// staging folder a stopped write left behind is swept.
#[test]
fn a_folder_left_without_its_manifest_is_written_again() {
    let _config = datasets_into_scratch("part_made");
    let protocol = shipped_protocol::fixture("shipped-sample", 1);
    let dir = starts::protocol_path(protocol);
    std::fs::create_dir_all(dir.join("panels")).unwrap();
    std::fs::write(dir.join("panels/stray.yaml"), "title: stray\n").unwrap();
    // A staging folder another process left an hour ago, one another
    // process is writing now, and an old one of a different folder whose
    // name starts with this one's.
    let stale = dir.with_file_name("shipped-sample.123-0.part");
    let in_flight = dir.with_file_name("shipped-sample.124-0.part");
    let other = dir.with_file_name("shipped-sample.bar.125-0.part");
    for folder in [&stale, &in_flight, &other] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let an_hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    for folder in [&stale, &other] {
        std::fs::File::open(folder)
            .and_then(|f| f.set_modified(an_hour_ago))
            .unwrap_or_else(|e| panic!("age {}: {e}", folder.display()));
    }

    assert_eq!(
        starts::write_protocol(protocol).expect("the folder writes"),
        dir
    );
    assert!(dir.join("arcform.yaml").is_file());
    assert!(
        !dir.join("panels/stray.yaml").exists(),
        "the part-made folder's files were kept"
    );
    assert!(!stale.exists(), "the stale staging folder was left");
    assert!(
        in_flight.exists(),
        "a staging folder another process is writing now was swept"
    );
    assert!(
        other.exists(),
        "another folder's staging folder was swept as this one's"
    );
}

/// **`Boot` with no runner refuses a Protocol that has not run**, rather than
/// running it: the runner `Boot::start` hands a run to is none, so a test
/// harness that boots every start is never started as `arc`.
#[test]
fn a_boot_with_no_runner_refuses_a_protocol_that_has_not_run() {
    let _config = datasets_into_scratch("no_runner");
    let protocol = shipped_protocol::fixture("shipped-sample", 1);
    let Err(e) = Boot::shipped_protocol(ID, protocol, Flow::Vertical, None) else {
        panic!("a Protocol that has not run booted with no runner");
    };
    assert!(e.contains("no runner"), "{e}");
    let dir = starts::protocol_path(protocol);
    assert!(
        brightfield_shell::run::record_paths_newest_first(&dir).is_empty(),
        "a run was made with no runner given"
    );
}

/// **A click on the start while the tiles' own Run is going lands on the
/// panel**: the run the reader started from the tiles is not a start's run,
/// and the click still keeps the database it is writing, takes that run as
/// the start's, and opens the first panel when it lands.
#[test]
fn a_click_while_the_tiles_own_run_is_going_lands_on_the_panel() {
    let _config = datasets_into_scratch("tiles_run_then_click");
    let protocol = with_step(
        "shipped-slow",
        "slow",
        "CREATE OR REPLACE TABLE \"slow\" AS SELECT sum(i) AS s FROM range(3000000000) t(i)\n",
    );
    let dir = starts::write_protocol(protocol).expect("the folder writes");
    let manifest = dir.join("arcform.yaml");
    let boot = Boot::open(&manifest.to_string_lossy(), Flow::Vertical, None)
        .expect("the folder opens as its tiles");
    let mut win = Window::over(boot, runner());
    let ctx = win.ctx.clone();
    assert!(win.app.run_protocol(&ctx), "the tiles' Run did not start");
    let deadline = Instant::now() + PATIENCE;
    while !has_database(&dir) {
        assert!(Instant::now() < deadline, "the run never made its database");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        brightfield_shell::run::record_paths_newest_first(&dir).is_empty(),
        "the run finished before the click, so this test proves nothing"
    );
    win.app.open_shipped_protocol(&ctx, ID, protocol);
    assert!(
        has_database(&dir),
        "the click removed the database the tiles' run is writing"
    );
    let deadline = Instant::now() + PATIENCE;
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run never landed");
        std::thread::sleep(Duration::from_millis(50));
        win.frame();
    }
    win.settle();
    assert!(win.banners().is_empty(), "{:?}", win.banners());
    assert_eq!(
        win.app.panel_tiles().and_then(|t| t.selected()),
        Some(0),
        "the run did not land on the first panel through the start's landing"
    );
}

// ---------------------------------------------------------------------------
// The three fetched gallery starts
// ---------------------------------------------------------------------------

/// The columns of `table` in the database the Protocol in `dir` made, in order.
fn columns_of(dir: &Path, table: &str) -> Vec<String> {
    let db = arc::spec::protocol_db_path(dir).expect("the Protocol names its database");
    let conn = duckdb::Connection::open_with_flags(
        &db,
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .expect("a read-only config"),
    )
    .unwrap_or_else(|e| panic!("open {}: {e}", db.display()));
    let mut stmt = conn
        .prepare("SELECT column_name FROM information_schema.columns WHERE table_name = ? ORDER BY ordinal_position")
        .expect("prepare the column query");
    let names = stmt
        .query_map([table], |row| row.get::<_, String>(0))
        .expect("query the columns")
        .collect::<Result<Vec<_>, _>>()
        .expect("read the columns");
    names
}

/// How many rows `table` holds in the database the Protocol in `dir` made.
fn rows_of(dir: &Path, table: &str) -> i64 {
    let db = arc::spec::protocol_db_path(dir).expect("the Protocol names its database");
    let conn = duckdb::Connection::open_with_flags(
        &db,
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .expect("a read-only config"),
    )
    .unwrap_or_else(|e| panic!("open {}: {e}", db.display()));
    conn.query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |row| {
        row.get(0)
    })
    .unwrap_or_else(|e| panic!("count {table}: {e}"))
}

/// The columns of Epoch AI's `models` table, which its panel reads.
const EPOCH_MODELS_COLUMNS: [&str; 10] = [
    "model",
    "organization",
    "publication_date",
    "publication_year",
    "training_compute_flop",
    "training_compute_log10",
    "domain",
    "organization_type",
    "access",
    "compute_record",
];

/// Take the shipped start `id` over the network, as a click does, and return
/// the folder its run wrote once the run has landed on its panel.
fn land_fetched_start(id: &str) -> PathBuf {
    let start = starts::find(id).expect("the shipped set has it");
    assert!(start.remote, "{id} is meant to reach the network");
    let protocol = start.protocol.as_ref().expect("it is a Protocol start");
    let mut win = Window::open(runner());
    win.take_and_land(protocol);
    assert!(win.banners().is_empty(), "{id}: {:?}", win.banners());
    assert_eq!(
        win.app.panel_tiles().and_then(|t| t.selected()),
        Some(0),
        "{id}'s run did not land on its panel"
    );
    drop(win);
    starts::protocol_path(protocol)
}

/// The taxi start fetches the TLC's month and makes the `trips` table its
/// panel reads, with rows in it. Columns and shape only: the TLC may revise
/// the month.
///
/// `cargo test -p brightfield-shell --test shipped_protocol_start -- --ignored`.
#[test]
#[ignore = "network: fetches January 2026's yellow taxi trips from the TLC, about 64 MB"]
fn the_nyc_yellow_taxi_protocol_fetches_and_produces_the_columns_its_panel_reads() {
    let _config = datasets_into_scratch("network_taxi");
    let dir = land_fetched_start(starts::NYC_YELLOW_TAXI);
    assert_eq!(
        columns_of(&dir, "trips"),
        [
            "pickup_time",
            "pickup_date",
            "pickup_hour",
            "weekday",
            "trip_miles",
            "pickup_place",
            "trip_record"
        ],
        "the trips table no longer has the columns the panel reads"
    );
    assert!(rows_of(&dir, "trips") > 0, "the month made no trips");
}

/// The Online Retail start fetches UCI's archive, reads both sheets of the
/// workbook inside it, and makes the `retail_orders` table its panel reads.
/// No table it makes holds the Customer ID.
///
/// `cargo test -p brightfield-shell --test shipped_protocol_start -- --ignored`.
#[test]
#[ignore = "network: fetches Online Retail II from the UCI Machine Learning Repository, about 45 MB"]
fn the_online_retail_ii_protocol_fetches_and_produces_the_columns_its_panel_reads() {
    let _config = datasets_into_scratch("network_retail");
    let dir = land_fetched_start(starts::ONLINE_RETAIL);
    assert_eq!(
        columns_of(&dir, "retail_orders"),
        [
            "invoice",
            "invoice_time",
            "month",
            "month_number",
            "country",
            "lines",
            "order_value",
            "customer",
            "customer_order_number",
            "customer_record"
        ],
        "the retail_orders table no longer has the columns the panel reads"
    );
    assert!(
        rows_of(&dir, "retail_orders") > 0,
        "the archive made no orders"
    );
    for table in ["retail_lines", "retail_orders"] {
        assert!(
            !columns_of(&dir, table).iter().any(|c| c == "Customer ID"),
            "{table} holds the Customer ID, which no table may"
        );
    }
}

/// The Epoch AI start fetches Epoch AI's file and makes the `models` table its
/// panel reads. No table it makes holds the Authors or Abstract columns.
/// Columns and shape only: the file grows as models are released.
///
/// `cargo test -p brightfield-shell --test shipped_protocol_start -- --ignored`.
#[test]
#[ignore = "network: fetches Epoch AI's notable models, about 2 MB"]
fn the_epoch_ai_models_protocol_fetches_and_produces_the_columns_its_panel_reads() {
    let _config = datasets_into_scratch("network_epoch");
    let dir = land_fetched_start(starts::EPOCH_AI_MODELS);
    assert_eq!(
        columns_of(&dir, "models"),
        EPOCH_MODELS_COLUMNS,
        "the models table no longer has the columns the panel reads"
    );
    assert!(rows_of(&dir, "models") > 0, "the file made no models");
    for table in ["raw_models", "models"] {
        let columns = columns_of(&dir, table);
        assert!(
            !columns.iter().any(|c| c == "Authors" || c == "Abstract"),
            "{table} holds Authors or Abstract, which no table may: {columns:?}"
        );
    }
}

/// The reduced copies the three fetched starts are drawn from offline hold
/// none of the columns those starts may not show, and each is the copy its
/// provenance file records, which names the issuer, terms and credit.
#[test]
fn the_fetched_starts_reduced_copies_hold_no_forbidden_column_and_match_their_provenance() {
    use sha2::{Digest, Sha256};
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    for id in [
        starts::NYC_YELLOW_TAXI,
        starts::ONLINE_RETAIL,
        starts::EPOCH_AI_MODELS,
    ] {
        let copy = data.join(format!("{id}-reduced.parquet"));
        let conn = duckdb::Connection::open_in_memory().expect("open DuckDB");
        let mut stmt = conn
            .prepare("SELECT name FROM parquet_schema(?) WHERE num_children IS NULL")
            .expect("prepare the schema query");
        let columns = stmt
            .query_map([copy.display().to_string()], |row| row.get::<_, String>(0))
            .expect("read the schema")
            .collect::<Result<Vec<_>, _>>()
            .expect("read the column names");
        for forbidden in ["Authors", "Abstract", "Customer ID"] {
            assert!(
                !columns.iter().any(|c| c == forbidden),
                "{} holds {forbidden}: {columns:?}",
                copy.display()
            );
        }
        let provenance: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(data.join(format!("{id}-reduced.json")))
                .unwrap_or_else(|e| panic!("{id}'s provenance file: {e}")),
        )
        .expect("the provenance file parses");
        let bytes = std::fs::read(&copy).expect("read the copy");
        assert_eq!(
            provenance["fixture"]["sha256"].as_str(),
            Some(format!("{:x}", Sha256::digest(&bytes)).as_str()),
            "{} is not the copy its provenance file records: re-cut it with \
             scripts/cut-start-fixtures.sh, which writes both",
            copy.display()
        );
        for key in ["issuer", "credit"] {
            assert!(
                provenance[key].as_str().is_some_and(|s| !s.is_empty()),
                "{id}'s provenance file names no {key}"
            );
        }
        assert!(
            provenance["terms"]["title"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "{id}'s provenance file names no terms"
        );
    }
}

/// Epoch AI's shipped load step leaves out `Authors` and `Abstract`, checked
/// with no network: the start's Protocol runs over the synthetic CSV in
/// `PROTOCOL_FETCH_COPIES`, which has the file's real columns with those two
/// as placeholders, so the step's own `EXCLUDE` is what removes them.
#[test]
fn the_epoch_ai_models_load_step_leaves_out_authors_and_abstract_offline() {
    let _config = datasets_into_scratch("epoch_load_step");
    let start = starts::find(starts::EPOCH_AI_MODELS).expect("the shipped set has it");
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let copies: Vec<(&str, PathBuf)> = shipped_protocol::PROTOCOL_FETCH_COPIES
        .iter()
        .map(|(url, file)| (*url, crate_dir.join(file)))
        .collect();
    let synthetic = &copies
        .iter()
        .find(|(url, _)| url.starts_with("https://epoch.ai/"))
        .expect("Epoch AI's URL has a fetch copy")
        .1;
    let header = std::fs::read_to_string(synthetic).expect("read the synthetic CSV");
    let header = header.lines().next().expect("the CSV has a header");
    assert!(
        header.contains("Authors") && header.contains("Abstract"),
        "the synthetic CSV lacks the columns the load step must leave out, so \
         this test would pass without the step doing anything"
    );
    let borrowed: Vec<(&str, &Path)> = copies
        .iter()
        .map(|(url, path)| (*url, path.as_path()))
        .collect();
    let offline = shipped_protocol::stand_in(
        start.protocol.as_ref().expect("it is a Protocol start"),
        &borrowed,
    );
    let mut win = Window::open(runner());
    win.take_and_land(offline);
    assert!(win.banners().is_empty(), "{:?}", win.banners());
    drop(win);
    let dir = starts::protocol_path(offline);
    for table in ["raw_models", "models"] {
        let columns = columns_of(&dir, table);
        assert!(
            !columns.iter().any(|c| c == "Authors" || c == "Abstract"),
            "{table} holds Authors or Abstract: {columns:?}"
        );
    }
    assert!(
        columns_of(&dir, "raw_models").iter().any(|c| c == "Model"),
        "the load step made no raw_models table to check"
    );
    assert_eq!(columns_of(&dir, "models"), EPOCH_MODELS_COLUMNS);
    assert_eq!(rows_of(&dir, "models"), 6, "the six synthetic models");
}
