//! Opening a data file the user chose, through the entry points a user
//! actually reaches.
//!
//! Two of them, and both are here on purpose. `data_file::open` is the seam the
//! window calls; `MeridianApp::open_data_file` is what the front door's control
//! reaches through, and it is where "a failure must never be a blank frame"
//! either holds or does not — the function under it can return a perfectly
//! worded `Err` while the window swallows it.
//!
//! **No dialog is opened anywhere in this file.** `data_file::pick` is one call
//! into rfd and nothing else; every decision worth gating sits on the other side
//! of it and takes a string. A test that raised an operating-system modal would
//! hijack the desktop of whoever ran the suite.
//!
//! The fixtures are written to a temp directory at test time rather than
//! committed, because what is being gated is DuckDB reading a real file off a
//! real filesystem — a fixture path that resolved relative to the checkout
//! would pass from the repo root and nowhere else, which is the exact defect
//! `starts.rs` exists to remember.

use std::path::{Path, PathBuf};

use brightfield_shell::data_file::{self, FirstLook};
use brightfield_shell::design::Mode;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::ViewKind;

/// A directory of this test's own, removed when the test ends.
///
/// `std::env::temp_dir` plus the test's name plus the process id: unique per
/// run, so a suite running its tests concurrently cannot have two of these
/// collide, and readable in a failure message.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir = std::env::temp_dir().join(format!(
            "bf-data-file-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    /// Write `contents` to `name` inside this directory and hand back the path.
    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).expect("the fixture writes");
        path
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A CSV with a numeric column and a categorical one — the ordinary shape.
const READINGS_CSV: &str = "region,reading\n\
                            north,12\n\
                            north,18\n\
                            south,31\n\
                            south,44\n\
                            east,7\n\
                            east,25\n\
                            west,52\n\
                            west,63\n";

/// A window under test, with one `egui::Context` for its whole life — the same
/// arrangement `front_door.rs` uses, and for the same reason: egui resolves a
/// click against a widget id registered on a previous frame.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn open() -> Self {
        Self {
            app: MeridianApp::headless_with_layout(Boot::empty(), default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        }
    }

    fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                events,
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }

    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new()]);
    }
}

/// One frame's worth of a pointer move and a primary click at `pos`.
fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::PointerMoved(pos)];
    for pressed in [true, false] {
        events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
    events
}

// ---------------------------------------------------------------------------
// AC1 — the file becomes a queryable table
// ---------------------------------------------------------------------------

/// A CSV the user chose opens as a table the engine can be **queried** for, not
/// as a picture of one.
///
/// The assertion is the row count and the schema the live session returns for
/// the step the Data pane tabulates — `SELECT * FROM <the file's view>` — read
/// back through the same windowed seam that pane reads through. Asserting that
/// the composition has a plot would not have caught a chart drawn over a
/// rolled-up view with the file nowhere in the session.
#[test]
fn a_chosen_csv_becomes_a_table_the_session_can_be_queried_for() {
    let dir = TempDir::new("csv-table");
    let path = dir.write("readings.csv", READINGS_CSV);

    let (mut live, composed) =
        data_file::open(&path.to_string_lossy()).expect("an ordinary CSV opens");

    assert!(
        composed.width > 0 && composed.height > 0,
        "the open has to land on a drawn result, not an empty frame"
    );

    // The step the grid reads is mark 0's SOURCE — the file's own view. Eight
    // data rows in, eight rows out.
    let session = live.coordinator().session();
    assert_eq!(
        session.step_rows_count(0).expect("the step counts"),
        8,
        "every row of the file is in the table, unaggregated"
    );

    // …and its columns are the file's columns, in the file's order.
    let batches = session
        .execute_step_rows_window(0, 0, 8)
        .expect("the windowed read the Data pane makes");
    let schema = batches.first().expect("a batch of rows").schema();
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(
        names,
        ["region", "reading"],
        "the grid shows the file's schema, not a rolled-up one"
    );
}

/// A Parquet opens on the same path as a CSV, and the file is *read* rather
/// than copied into memory — the whole reason the engine holds it as a view.
///
/// The Parquet is written by the same DuckDB the engine reads it back with, so
/// this gate needs no fixture committed to the tree and no second writer to
/// keep in step with the reader.
#[test]
fn a_chosen_parquet_opens_on_the_same_path() {
    let dir = TempDir::new("parquet-table");
    let csv = dir.write("readings.csv", READINGS_CSV);
    let parquet = dir.path().join("readings.parquet");

    write_parquet(&csv, &parquet);

    let (mut live, _composed) =
        data_file::open(&parquet.to_string_lossy()).expect("a Parquet opens");
    assert_eq!(
        live.coordinator()
            .session()
            .step_rows_count(0)
            .expect("the step counts"),
        8
    );
}

/// Write `csv` out as a Parquet at `parquet`, through the **same DuckDB** the
/// engine reads it back with — `brightfield-engine`'s own dependency, on the
/// same version line, so this gate cannot pass because a second writer's idea
/// of Parquet happens to agree with the reader's.
fn write_parquet(csv: &Path, parquet: &Path) {
    let conn = duckdb::Connection::open_in_memory().expect("an in-memory DuckDB");
    conn.execute_batch(&format!(
        "COPY (SELECT * FROM read_csv('{}')) TO '{}' (FORMAT PARQUET)",
        csv.display(),
        parquet.display()
    ))
    .expect("the COPY runs");
    assert!(parquet.is_file(), "the Parquet was written");
}

/// The first look is chosen from the table's own profile, and a numeric column
/// wins — asserted through `open`, so a change that stopped profiling and
/// hard-coded a picture reddens here rather than passing the unit test.
#[test]
fn the_first_look_over_a_numeric_column_is_its_distribution() {
    let dir = TempDir::new("first-look");
    let path = dir.write("readings.csv", READINGS_CSV);

    let columns = {
        // Same two-step the open makes: profile, then choose.
        let (mut live, _) = data_file::open(&path.to_string_lossy()).expect("opens");
        live.coordinator()
            .session()
            .profile_sources()
            .into_iter()
            .find(|p| p.name == data_file::SOURCE)
            .map(|p| match p.outcome {
                brightfield_engine::ProfileOutcome::Profiled { columns, .. } => columns,
                other => panic!("the opened file did not profile: {other:?}"),
            })
            .expect("the opened file is the session's source")
    };
    assert_eq!(
        data_file::first_look(&columns),
        Some(FirstLook::Histogram {
            column: "reading".to_string()
        }),
        "a numeric column is a distribution, and a distribution is the most \
         informative thing that can be drawn about a column nobody described"
    );
}

/// A table with no numeric column opens on the OTHER first look — the count
/// grid — and it too reads the file's own view, so the Data pane still holds
/// every row.
///
/// This is here because the grid is the branch a unit test cannot vouch for:
/// `cell` with a self-aggregating `fill: {count:}` is a different lowerer from
/// the histogram's, and a spec that merely parses proves nothing about whether
/// DuckDB will run it or the renderer will place it.
#[test]
fn a_table_with_no_numeric_column_opens_on_a_count_grid() {
    let dir = TempDir::new("grid");
    let path = dir.write(
        "links.csv",
        "tier,method\n\
         authoritative,sec-registration\n\
         authoritative,sec-ncen\n\
         candidate,jaro_winkler\n\
         candidate,exact_name\n\
         authoritative,sec-registration\n\
         candidate,jaro_winkler\n",
    );

    let (mut live, composed) =
        data_file::open(&path.to_string_lossy()).expect("a table of two categorical columns opens");
    assert!(
        composed.width > 0 && composed.height > 0,
        "the grid has to land on a drawn result"
    );
    assert!(
        composed.mark_faults.is_empty(),
        "the engine refused the grid's own mark: {:?}",
        composed.mark_faults
    );
    assert_eq!(
        live.coordinator()
            .session()
            .step_rows_count(0)
            .expect("the step counts"),
        6,
        "the grid aggregates in its own query, so the table behind it is still \
         the file"
    );
}

/// A table that admits neither first look is refused **by name and by
/// reason**, rather than composing an empty window.
///
/// A single column of free text has no distribution to bin and nothing to
/// cross, and the honest answer is a sentence — the composition path returns
/// `Err` when no mark renders, so the alternative is not a blank chart, it is
/// an unexplained one. Reopening this shape as a table with no picture is
/// residual scope, and the message says what is missing so the gap is legible
/// rather than mysterious.
#[test]
fn a_table_with_nothing_to_draw_says_what_it_is_missing() {
    let dir = TempDir::new("nothing-to-draw");
    let path = dir.write("names.csv", "name\nada\ngrace\nbarbara\nkaren\n");

    let refusal = data_file::open(&path.to_string_lossy())
        .err()
        .expect("one free-text column admits no first look");
    assert!(refusal.contains("names.csv"), "{refusal}");
    assert!(refusal.contains("numeric"), "{refusal}");
    assert!(refusal.contains("1 column"), "{refusal}");
}

// ---------------------------------------------------------------------------
// AC2 — the front door offers it
// ---------------------------------------------------------------------------

/// The front door draws an open-a-file control, inside the window, and clicking
/// it asks for the dialog.
///
/// Clicked where the last frame actually laid it out rather than at a
/// coordinate typed here, for the reason `front_door.rs` records: a hand-typed
/// point lands today and goes on being green while clicking empty background
/// the first time a padding moves. What is asserted is the **request**, not a
/// dialog: the picker is the one thing here a headless test may not run.
#[test]
fn the_front_door_offers_opening_a_file_and_the_control_is_wired() {
    let mut win = Window::open();
    win.settle();
    assert!(
        win.app.front_door_is_live(),
        "a launch with nothing named shows the door"
    );

    let target = win
        .app
        .front_door_open_file_rect()
        .expect("the door draws an open-a-data-file control");
    assert!(
        win.screen.contains_rect(target),
        "the control drew at {target:?}, outside the window — nothing could \
         click it"
    );
    assert!(!win.app.pick_requested(), "nothing has been clicked yet");

    win.run(vec![click_at(target.center())]);
    assert!(
        win.app.pick_requested(),
        "clicking the control has to ask for the file dialog"
    );
}

/// …and the control belongs to the door: once something is open it is gone,
/// exactly as the gallery cards are.
#[test]
fn the_open_a_file_control_goes_with_the_door() {
    let dir = TempDir::new("door-morph");
    let path = dir.write("readings.csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    assert!(win.app.front_door_open_file_rect().is_some());

    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    assert!(
        !win.app.front_door_is_live(),
        "an opened file is content, and content outcompetes the door"
    );
    assert!(
        win.app.front_door_open_file_rect().is_none(),
        "a test asking where the control was after the door has gone must \
         hear 'nowhere'"
    );
}

/// Opening a file through the window puts the file on the charts view, live —
/// so a brush over the histogram re-queries rather than filtering a frozen
/// batch, and the Data pane has rows to read.
#[test]
fn opening_a_file_lands_on_the_charts_view_with_a_live_session() {
    let dir = TempDir::new("lands-live");
    let path = dir.write("readings.csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    assert_eq!(win.app.active(), ViewKind::Charts);
    assert!(
        win.app.chart_doc().is_live(),
        "a file opened from the door has to arm its own gestures"
    );
    assert!(
        win.app.title().contains("readings.csv"),
        "the window names the file the user picked, not the path they never \
         typed: {}",
        win.app.title()
    );
    assert!(
        win.app.notifications().is_empty(),
        "an ordinary open raises no banner"
    );
}

// ---------------------------------------------------------------------------
// AC3 — a file the engine cannot read fails by name
// ---------------------------------------------------------------------------

/// A file DuckDB will not read fails with the path and the engine's own reason,
/// and the window stays exactly as it was.
///
/// Asserted through the window rather than through `data_file::open` alone,
/// which is the difference between "the function returns a good message" and
/// "the user sees one": a banner is what stands between this and a blank frame.
#[test]
fn a_file_the_engine_cannot_read_names_itself_and_leaves_the_window_up() {
    let dir = TempDir::new("unreadable");
    // A .parquet that is not a Parquet: the extension passes, the reader does
    // not. This is the ordinary shape of the failure — a download that landed
    // as an HTML error page, a truncated copy.
    let path = dir.write("broken.parquet", "this is not a parquet file at all\n");

    let refusal = data_file::open(&path.to_string_lossy())
        .err()
        .expect("a file that is not a Parquet does not open");
    assert!(
        refusal.contains("broken.parquet"),
        "the failure has to name the file: {refusal}"
    );
    assert!(
        refusal.len() > "broken.parquet: ".len() + 8,
        "the failure has to carry a reason as well as a name: {refusal}"
    );

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    assert_eq!(
        win.app.notifications().len(),
        1,
        "the reason has to reach a surface the user is looking at"
    );
    assert!(
        win.app.front_door_is_live(),
        "a failed open changes nothing — the door is still standing, which is \
         the whole of 'never a blank frame'"
    );
    assert!(
        win.app.front_door_open_file_rect().is_some(),
        "…and the control that failed is still there to try again with"
    );
}

/// A second failure replaces the first's banner rather than stacking a history
/// of what did not open, and a success takes it down.
#[test]
fn a_failed_open_leaves_one_banner_and_a_success_clears_it() {
    let dir = TempDir::new("banner-life");
    let broken = dir.write("broken.parquet", "not a parquet\n");
    let good = dir.write("readings.csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();

    win.app.open_data_file(&ctx, &broken.to_string_lossy());
    assert_eq!(win.app.notifications().len(), 1);
    win.app.open_data_file(&ctx, "/no/such/file.csv");
    assert_eq!(
        win.app.notifications().len(),
        1,
        "the last attempt is the only one that matters"
    );
    win.app.open_data_file(&ctx, &good.to_string_lossy());
    assert!(
        win.app.notifications().is_empty(),
        "a success takes its own failure banner down"
    );
}

/// A path naming nothing fails before the engine is built, and says so in the
/// words of the thing the user picked.
#[test]
fn a_path_naming_nothing_fails_by_name() {
    let refusal = data_file::open("/no/such/directory/readings.csv")
        .err()
        .expect("a missing file does not open");
    assert!(refusal.contains("readings.csv"), "{refusal}");
}

// ---------------------------------------------------------------------------
// AC4 — a URL is refused
// ---------------------------------------------------------------------------

/// A URL is refused, through the same entry point the door's control reaches,
/// with a message saying it is a URL — and nothing is opened.
///
/// This is the offline promise's cheapest failure mode. DuckDB binds a view
/// over an `https://` Parquet through `httpfs` eagerly and without complaint,
/// so a path box that merely *passed the string on* would fetch, succeed, and
/// leave the promise broken with nothing red anywhere.
#[test]
fn a_url_is_refused_at_the_window_and_opens_nothing() {
    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();

    win.app
        .open_data_file(&ctx, "https://openlake.meridian.online/edgar_gleif.parquet");
    win.settle();

    assert!(
        win.app.front_door_is_live(),
        "a URL opens nothing at all — the door is still standing"
    );
    assert_eq!(
        win.app.notifications().len(),
        1,
        "…and it is refused out loud, not ignored"
    );
    assert!(
        !win.app.chart_doc().is_live(),
        "no session was built over a remote table"
    );
}

/// The refusal is by scheme and happens before any engine exists, so a URL that
/// would have resolved is turned away exactly as one that would not.
#[test]
fn every_url_scheme_is_refused_before_the_engine() {
    for url in [
        "https://openlake.meridian.online/edgar_gleif.parquet",
        "http://127.0.0.1:1/never.csv",
        "s3://bucket/key.parquet",
        "ducklake:https://example.com/catalog.ducklake",
    ] {
        let refusal = data_file::open(url)
            .err()
            .unwrap_or_else(|| panic!("{url} is not a local file and must not open"));
        assert!(refusal.contains("URL"), "{url}: {refusal}");
    }
}
