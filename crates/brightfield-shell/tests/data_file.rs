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

use brightfield_protocol::layout::Flow;
use brightfield_shell::chart_kinds;
use brightfield_shell::data_file;
use brightfield_shell::design::Mode;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::registry::ChartKindId;
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

    let (mut live, composed, _look) =
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

    let (mut live, _composed, _look) =
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
        let (mut live, _, _) = data_file::open(&path.to_string_lossy()).expect("opens");
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
    let look = data_file::first_look(&columns).expect("a numeric column admits a first look");
    assert_eq!(
        look.kind(),
        chart_kinds::BINNED_HISTOGRAM,
        "a numeric column is a distribution, and a distribution is the most \
         informative thing that can be drawn about a column nobody described"
    );
    assert!(
        look.block().contains("x: { bin: 'reading' }"),
        "the kind bound the profiled column: {look:?}"
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

    let (mut live, composed, _look) =
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

/// A table of one free-text column **is** a picture now: the registry carries a
/// kind whose single slot a category fills, so the column that used to be
/// refused opens on its ranking.
///
/// Asserted through `open`, so what is pinned is what a person gets — a live
/// session over their file with something drawn on it — rather than which
/// branch chose it.
#[test]
fn a_table_of_one_category_opens_on_its_ranking() {
    let dir = TempDir::new("one-category");
    let path = dir.write("names.csv", "name\nada\ngrace\nbarbara\nkaren\nada\n");

    let (mut live, composed, look) =
        data_file::open(&path.to_string_lossy()).expect("one category is a ranking, not a refusal");
    assert_eq!(look.kind(), brightfield_shell::ranked_bars::KIND_ID);
    assert!(
        !composed.plots.is_empty(),
        "the ranking composed no plot at all"
    );
    // …and the Data pane beside it still holds the file's own rows, which is
    // the property every kind in the registry is held to.
    assert_eq!(
        live.coordinator()
            .session()
            .step_rows_count(0)
            .expect("the step counts"),
        5,
        "the ranking aggregates in its own query, so the table behind it is \
         still the file"
    );
}

/// **A file that opened cleanly says nothing.** The window carries no banner
/// over a picture it drew from a table the user merely opened.
///
/// Asserted at the window rather than on the diagnostics, because the banner is
/// the artefact: `MeridianApp::say_load_diagnostics` turns a load's advisories
/// into one `Severity::Warning` reading *"… had no effect"*, and the user has
/// no spec of their own to go and correct — the spec was synthesised by the
/// chart kind. The one-category table is the case that reached this: its block
/// binds `$sel` from a `toggleY` and a `highlight`, and a block that binds a
/// selection it does not declare earns exactly that advisory.
#[test]
fn a_one_category_table_opens_without_a_banner_over_its_picture() {
    let dir = TempDir::new("one-category-banner");
    let path = dir.write("names.csv", "name\nada\ngrace\nbarbara\nkaren\nada\n");

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    let said: Vec<String> = win
        .app
        .load_diagnostics()
        .diagnostics
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        win.app.notifications().len(),
        0,
        "the window put a banner over a file that opened cleanly: {said:?}"
    );
}

/// **Which chart documents carry the kind that chose their picture** —
/// enumerated over the routes that open one, rather than swept.
///
/// The chart pane draws a document through that kind's `ChartModule` exactly
/// when the document carries an `Authored` record; with none it presents
/// directly. So a route that synthesises a picture *from a chart kind* and does
/// not record which one draws that picture around the module rather than
/// through it — and nothing on screen changes when it happens, which is why
/// the routes are listed here instead of grepped for.
///
/// The routes this build opens a chart document on:
///
/// - `MeridianApp::open_data_file` — a table with no spec. The registry chose
///   the picture, so the kind is recorded and it is one this build has.
/// - `Boot::start` — the shipped starts, `include_str!`-ed spec source. The
///   registry was never asked, so there is no kind and no binding to record,
///   and the pane presents these directly.
/// - `Boot::open` / the spec editor — a spec someone wrote, which is the same
///   answer as the starts for the same reason. Covered by the start arm below;
///   both reach `ChartDoc::open`, which clears the record.
///
/// `starts::CROSSWALK_CHART` is deliberately not in the list: its spec reads a
/// source over https, and a test that opens it fails on a train.
#[test]
fn a_chart_kinds_picture_carries_its_kind_and_a_written_spec_carries_none() {
    let dir = TempDir::new("authored-routes");
    let path = dir.write("readings.csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &path.to_string_lossy());
    win.settle();

    let authored = win.app.chart_doc().authored().cloned().expect(
        "the open-a-data-file route drew a chart kind's picture and recorded no \
         kind, so the pane draws it around the module instead of through it",
    );
    assert!(
        chart_kinds::find(authored.kind).is_some(),
        "the recorded kind {} is not in this build's registry, so the pane has \
         nothing to draw the picture with",
        authored.kind
    );

    for id in [starts::DASHBOARD, starts::DISTRIBUTION, starts::BREAKDOWN] {
        let boot = Boot::start(id, Flow::Vertical).unwrap_or_else(|e| panic!("{id}: {e}"));
        let app = MeridianApp::headless(boot, Mode::Light);
        assert_eq!(
            app.chart_doc().authored(),
            None,
            "{id}: a spec someone wrote was recorded as a chart kind's picture"
        );
    }
}

/// Two categorical columns and nothing to bin — the shape `count-grid` takes.
///
/// A column's `distinct` comes back from `approx_count_distinct` (see
/// `Session::profile_sources`), so which kind a fixture reaches is decided by an
/// estimate rather than by counting its rows: a two-valued column in a
/// five-row file was estimated at one and dropped the fixture to a single
/// field. Which kind each file below actually reaches is left to the coverage
/// assertion at the foot of the test rather than asserted per fixture here.
const CROSSED_CSV: &str = "tier,method\n\
                           authoritative,sec-registration\n\
                           authoritative,sec-ncen\n\
                           candidate,jaro_winkler\n\
                           candidate,exact_name\n\
                           authoritative,sec-registration\n\
                           candidate,jaro_winkler\n";

/// One categorical column — the shape ranked category bars take.
const ONE_CATEGORY_CSV: &str = "name\nada\ngrace\nbarbara\nkaren\nada\n";

/// **A file a user opened arrives on screen as a picture**, for each kind the
/// registry ships, drawn through that kind's module.
///
/// The gap this closes was measured rather than imagined. Emptying the
/// `Authored` record's `fields` in `MeridianApp::open_data_file`, or its
/// `block`, left the whole `brightfield-shell` suite green while the chart pane
/// went blank for every file opened from the front door — the first stops at
/// `ChartKind::bind` inside `ChartModule::ui`, the second at
/// `ChartDoc::draw_module`'s comparison, and both end with no raster, no legend
/// band and no `empty_state` to explain it. Nothing could see it: the tests in
/// `tests/chart_module.rs` construct their own `Authored` and never take the
/// one this route writes, and
/// `a_chart_kinds_picture_carries_its_kind_and_a_written_spec_carries_none`
/// above reads that record without drawing from it.
///
/// So the assertion is `raster_rect` on a settled window — the observable
/// `ChartDoc::present_raster` writes, and the one a GPU-free machine has (see
/// `tests/chart_module.rs`'s header for why a rect rather than pixels). Paired
/// with the recorded kind being one this build has, it says the picture arrived
/// by the **module** arm rather than beside it: `module_of` answers `Some` on
/// exactly that pair, and the pane's other arm is the one it answers `None`
/// for.
///
/// Written over `registry().kinds()` rather than over one fixture, so a kind
/// added with no file that reaches it reddens here instead of shipping unseen.
#[test]
fn every_shipped_kind_draws_its_picture_from_the_open_a_file_route() {
    let dir = TempDir::new("open-draws-a-picture");
    let mut drawn: Vec<ChartKindId> = Vec::new();

    for (name, contents) in [
        ("readings.csv", READINGS_CSV),
        ("crossed.csv", CROSSED_CSV),
        ("names.csv", ONE_CATEGORY_CSV),
    ] {
        let path = dir.write(name, contents);
        let mut win = Window::open();
        win.settle();
        let ctx = win.ctx.clone();
        win.app.open_data_file(&ctx, &path.to_string_lossy());
        win.settle();

        let authored = win.app.chart_doc().authored().cloned().unwrap_or_else(|| {
            panic!("{name}: opened with no kind recorded, so the pane drew it around the module")
        });
        assert!(
            chart_kinds::find(authored.kind).is_some(),
            "{name}: opened as {}, which is not in this build's registry, so \
             the pane has nothing to draw the picture with",
            authored.kind
        );
        let raster = win.app.chart_doc().raster_rect.unwrap_or_else(|| {
            panic!(
                "{name}: opened as {} and nothing reached the pane — the \
                 module drew no raster, so what the user gets is a blank \
                 chart with no sentence on it",
                authored.kind
            )
        });
        assert!(
            raster.width() > 0.0 && raster.height() > 0.0,
            "{name}: the raster was reserved at {raster:?}, which has no room \
             for a picture in it"
        );
        drawn.push(authored.kind);
    }

    for kind in chart_kinds::registry().kinds() {
        assert!(
            drawn.contains(&kind.id),
            "{}: no fixture here opens a file that chooses it, so nothing \
             holds that it draws through the door a user comes in by — the \
             fixtures above reached {drawn:?}",
            kind.id
        );
    }
}

/// A table no chart kind fits is refused **by name and by reason**, rather than
/// composing an empty window.
///
/// One column of identifiers: too many distinct values to read as an axis, and
/// nothing numeric to bin. The composition path returns `Err` when no mark
/// renders, so the alternative to a sentence is not a blank chart, it is an
/// unexplained one. Reopening this shape as a table with no picture is residual
/// scope, and the message says what the build's charts *do* take — read off the
/// registry, so the sentence cannot describe a set this build does not have.
#[test]
fn a_table_with_nothing_to_draw_says_what_it_is_missing() {
    let dir = TempDir::new("nothing-to-draw");
    let mut csv = String::from("id\n");
    for i in 0..200 {
        csv.push_str(&format!("row-{i}\n"));
    }
    let path = dir.write("identifiers.csv", &csv);

    let refusal = data_file::open(&path.to_string_lossy())
        .err()
        .expect("200 distinct identifiers fill no chart kind's slot");
    assert!(refusal.contains("identifiers.csv"), "{refusal}");
    assert!(refusal.contains("1 column"), "{refusal}");
    for kind in chart_kinds::registry().kinds() {
        assert!(
            refusal.contains(kind.description),
            "the refusal must say what {} takes: {refusal}",
            kind.id
        );
    }
}

// ---------------------------------------------------------------------------
// AC1, the other half — the file that opens is the file that was CHOSEN
// ---------------------------------------------------------------------------

/// Assert the invariant this section exists for, against a real decoy on a real
/// filesystem: **either the chosen file's rows come back, or the open is
/// refused by name.** Silently opening the other file is the one outcome barred.
///
/// Written as a constraint rather than as "it returns `Err`" on purpose. The
/// refusal is this build's answer, but a later one that escaped the name
/// faithfully instead would be a *better* answer, and a test asserting the
/// mechanism would redden on the improvement while staying green on the defect.
/// What must never pass is `Ok` over the decoy.
fn assert_opens_the_chosen_file_or_refuses(chosen: &Path, chosen_rows: u64, decoy_rows: u64) {
    let shown = chosen.display().to_string();
    match data_file::open(&chosen.to_string_lossy()) {
        Err(refusal) => {
            let name = chosen
                .file_name()
                .map(|n| n.to_string_lossy().escape_debug().to_string())
                .unwrap_or_default();
            assert!(
                refusal.contains(&name),
                "a refusal has to name the file the user picked. Wanted {name:?} \
                 in: {refusal}"
            );
            assert!(
                refusal.len() > name.len() + 8,
                "…and has to carry a reason as well as a name: {refusal}"
            );
        }
        Ok((mut live, _composed, _look)) => {
            let rows = live
                .coordinator()
                .session()
                .step_rows_count(0)
                .expect("the step counts");
            assert_ne!(
                rows, decoy_rows,
                "{shown} opened, and the table behind it holds the DECOY's \
                 {decoy_rows} rows — the window is titled from the picked name, \
                 so this is one file's name over another file's data"
            );
            assert_eq!(
                rows, chosen_rows,
                "{shown} opened over neither file's row count"
            );
        }
    }
}

/// A file name holding a bracket sits beside the file a **glob** of that name
/// matches, and the app does not quietly read the neighbour.
///
/// `sales[1].csv` is not a contrived name — it is what a browser writes for a
/// second download of `sales.csv`. DuckDB resolves a reader path as a glob, so
/// `read_csv('…/sales[1].csv')` binds over `sales1.csv`: measured on this
/// build's DuckDB, 3 rows where the chosen file has 8. Nothing is red when it
/// happens, which is why this gate is here rather than a comment.
#[test]
fn a_bracket_in_the_name_does_not_open_the_file_a_glob_would_match() {
    let dir = TempDir::new("glob-decoy");
    // The decoy is what `sales[1].csv` matches as a pattern: 3 rows.
    dir.write("sales1.csv", "region,reading\nnorth,12\nsouth,31\neast,7\n");
    // …and the file actually picked, 8 rows.
    let chosen = dir.write("sales[1].csv", READINGS_CSV);

    assert_opens_the_chosen_file_or_refuses(&chosen, 8, 3);
}

/// The same, one layer up: the **folder** carries the bracket. A reader path is
/// globbed whole, so a directory component matches exactly as a file name does,
/// and a guard that only looked at `file_name()` would pass this.
#[test]
fn a_bracket_in_a_parent_folder_is_caught_too() {
    let dir = TempDir::new("glob-decoy-dir");
    std::fs::create_dir_all(dir.path().join("q1")).expect("the decoy folder");
    std::fs::write(
        dir.path().join("q1").join("sales.csv"),
        "region,reading\nnorth,12\nsouth,31\neast,7\n",
    )
    .expect("the decoy writes");
    std::fs::create_dir_all(dir.path().join("q[1]")).expect("the chosen folder");
    let chosen = dir.path().join("q[1]").join("sales.csv");
    std::fs::write(&chosen, READINGS_CSV).expect("the fixture writes");

    assert_opens_the_chosen_file_or_refuses(&chosen, 8, 3);
}

/// A file name holding a **line break** sits beside the file that name folds
/// to, and the app does not quietly read the neighbour.
///
/// A line break is legal in a POSIX file name and YAML folds one inside a
/// quoted scalar to a space, so `sales<LF>2026.csv` written into the
/// synthesised spec parses back as `sales 2026.csv` — a different file, which
/// on this fixture has 3 rows rather than 8. DuckDB reads the real name
/// perfectly well; the loss is entirely in the round trip through the spec.
#[test]
fn a_line_break_in_the_name_does_not_open_the_file_it_folds_to() {
    let dir = TempDir::new("fold-decoy");
    // The decoy is what the line break folds to — a space: 3 rows.
    dir.write(
        "sales 2026.csv",
        "region,reading\nnorth,12\nsouth,31\neast,7\n",
    );
    let chosen = dir.path().join("sales\n2026.csv");
    let Ok(()) = std::fs::write(&chosen, READINGS_CSV) else {
        // A filesystem that refuses the name has nothing to gate here.
        return;
    };

    assert_opens_the_chosen_file_or_refuses(&chosen, 8, 3);
}

/// A refused name reaches the **window** as a banner and leaves the door up,
/// which is what makes the refusal an answer rather than a dropped click.
///
/// `data_file::open` returning a good sentence is not the same as a user seeing
/// one: `open_data_file` is the seam the door's control reaches through, and
/// the swallow would happen there.
#[test]
fn a_pattern_name_refused_at_the_window_opens_nothing_and_says_so() {
    let dir = TempDir::new("glob-window");
    dir.write("sales1.csv", "region,reading\nnorth,12\nsouth,31\neast,7\n");
    let chosen = dir.write("sales[1].csv", READINGS_CSV);

    let mut win = Window::open();
    win.settle();
    let ctx = win.ctx.clone();
    win.app.open_data_file(&ctx, &chosen.to_string_lossy());
    win.settle();

    assert!(
        !win.app.chart_doc().is_live(),
        "no session may be built over a name the reader would resolve to a \
         different file"
    );
    assert_eq!(
        win.app.notifications().len(),
        1,
        "…and it is refused out loud, not ignored"
    );
    assert!(
        win.app.front_door_is_live(),
        "the door is still standing — a refusal is never a blank frame"
    );
}

/// **The dialect gate.** Ask this build's DuckDB which characters it resolves
/// as a pattern, and require `accept` to refuse every one of them.
///
/// The refusal list in `data_file.rs` is a constant, and a constant about
/// somebody else's parser goes stale silently — a DuckDB bump that taught the
/// reader a new metacharacter would reopen exactly the defect this section
/// gates, with every test above still green. So the list is not trusted here:
/// each candidate character gets a real file of its own in a directory of
/// plausible siblings, `glob()` is asked what that path matches, and the
/// **danger condition is measured** — a non-empty match set that is not the
/// file itself. When nothing matches, DuckDB falls back to the literal path,
/// which is why most punctuation is safe and is asserted safe rather than
/// assumed.
///
/// This gate is one-directional by design: it requires the refusal list to
/// COVER what DuckDB globs, not to equal it. Refusing a character DuckDB
/// happens to resolve literally today is the deliberate margin documented on
/// `PATTERN_CHARACTERS`.
#[test]
fn every_character_this_duckdb_reads_as_a_pattern_is_refused() {
    let dir = TempDir::new("dialect");
    // Siblings a pattern could plausibly land on instead of the file itself.
    for sibling in [
        "s1.csv", "sa.csv", "sb.csv", "sX.csv", "sq.csv", "sxyz.csv", "s.csv",
    ] {
        dir.write(sibling, "region,reading\nnorth,12\n");
    }

    let conn = duckdb::Connection::open_in_memory().expect("an in-memory DuckDB");
    let mut globbed_by_duckdb: Vec<char> = Vec::new();

    // Every printable ASCII character, which is the alphabet a file name is
    // realistically drawn from. `/` is excluded because no POSIX file name may
    // hold one; anything else the filesystem refuses is skipped below rather
    // than predicted here.
    let candidates: Vec<char> = (0x20u8..0x7f)
        .map(char::from)
        .filter(|c| *c != '/')
        .collect();

    for candidate in candidates {
        let name = format!("s{candidate}.csv");
        let chosen = dir.path().join(&name);
        // Windows and some filesystems refuse a few of these outright; a name
        // that cannot exist cannot be opened, so it is not this gate's business.
        if std::fs::write(&chosen, READINGS_CSV).is_err() {
            continue;
        }
        let literal = chosen.display().to_string();
        let sql = format!("SELECT file FROM glob('{}')", literal.replace('\'', "''"));
        let mut statement = conn.prepare(&sql).expect("glob() prepares");
        let matched: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("glob() runs")
            .map(|row| row.expect("a matched path"))
            .collect();
        let resolves_elsewhere = !matched.is_empty() && matched != vec![literal.clone()];
        if resolves_elsewhere {
            globbed_by_duckdb.push(candidate);
            assert!(
                data_file::accept(&literal).is_err(),
                "DuckDB resolves a path containing `{candidate}` to {matched:?} \
                 rather than to the file itself, so opening it would read a \
                 different file — `accept` has to refuse it and does not"
            );
        }
        let _ = std::fs::remove_file(&chosen);
    }

    // The sweep has to have found something, or a `glob()` that silently
    // stopped working would make this test vacuous — the shape a structural
    // gate fails in.
    assert!(
        globbed_by_duckdb.contains(&'*') && globbed_by_duckdb.contains(&'?'),
        "the sweep did not observe DuckDB globbing `*` or `?`, so it proved \
         nothing about anything. Observed: {globbed_by_duckdb:?}"
    );
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
/// This is the cheapest way for the box to stop being about this machine.
/// DuckDB binds a view over an `https://` Parquet through `httpfs` eagerly and
/// without complaint, so a path box that merely *passed the string on* would
/// fetch, succeed, and leave nothing red anywhere.
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
