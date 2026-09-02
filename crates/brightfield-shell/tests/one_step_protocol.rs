//! **What a data file opens as, on the side of the window that is not the
//! picture**: one SQL step, one table, and the table's own columns in the
//! navigator rail.
//!
//! The dashboard half of opening a file is held by `tests/data_file.rs` and
//! `tests/dashboard_baseline.rs`. What is held here is the other document —
//! the Protocol brightfield writes and never runs — and the three surfaces it
//! fills: the navigator rail's outline, the Steps pane, and the inspector rail
//! when a tile is clicked. Plus the two things that make it durable: a spec the
//! pinned `arc` loader accepts, and a front door that lists it afterwards.
//!
//! # The fixtures are written at test time
//!
//! Every test here opens a real file through DuckDB, and Save writes beside
//! that file — so a committed fixture would mean a test that dirties the
//! checkout it runs in. Each test builds its own directory and removes it.
//!
//! # What the type beside a column is, in a test binary
//!
//! `LoadOptions::packaged` looks for a FineType bundle beside the running
//! executable and a `cargo test` binary has none, so every column here arrives
//! `SemanticType::NotAsked` and its DuckDB type is what the generator chose the
//! tile from — the same fact `tests/dashboard_baseline.rs` records for the
//! picture. So the assertions below compare the rail's note against
//! `Tile::chosen_by`, which is *what decided this tile*, rather than against a
//! label that would be present only on a machine carrying a bundle. A build
//! that does carry one fails these by naming the label it found, which is the
//! right way round.

use std::path::{Path, PathBuf};

use brightfield_protocol::layout::Flow;
use brightfield_shell::dashboard::ChosenBy;
use brightfield_shell::design::Mode;
use brightfield_shell::one_step::{self, OneStepProtocol};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_shell::{data_file, protocol};
use brightfield_workbench::RunState;

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir =
            std::env::temp_dir().join(format!("bf-one-step-{name}-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, contents).expect("the fixture writes");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Four columns, three of which earn a tile: a `VARCHAR` of stations ranked, a
/// `BIGINT` of readings binned, a `DOUBLE` of depths binned — and a `survey`
/// column of one distinct value, which a picture of it draws as a single bar,
/// so the generator declines it.
///
/// The declined column is the point of the fourth: the navigator rail lists the
/// **table's** columns, not the dashboard's tiles, so a column with no picture
/// still has a row.
const HARBOUR_CSV: &str = "station,reading,depth,survey\n\
                           north,12,4.5,autumn\n\
                           north,18,6.0,autumn\n\
                           south,31,2.5,autumn\n\
                           south,44,9.5,autumn\n\
                           east,7,1.0,autumn\n\
                           east,25,7.5,autumn\n\
                           west,52,3.0,autumn\n\
                           west,63,8.0,autumn\n";

/// The columns [`HARBOUR_CSV`] declares, in the file's own order.
const HARBOUR_COLUMNS: &[&str] = &["station", "reading", "depth", "survey"];

/// The column the generator declines — one distinct value.
const DECLINED: &str = "survey";

/// A window under test, with one `egui::Context` for its whole life and one
/// screen rect. The arrangement `tests/data_file.rs` drives its gestures
/// through, so the geometry a click aims at here is the geometry that file's
/// clicks land on.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn over(boot: Boot) -> Self {
        Self::with_layout(boot, default_layout())
    }

    fn with_layout(boot: Boot, layout: brightfield_workbench::SavedLayout) -> Self {
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, layout, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        };
        win.settle();
        win
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    /// A pointer position in the middle of plot `index`'s data area, in the
    /// window coordinates the raster was presented at — `tests/data_file.rs`'s
    /// `at`, at the one fraction this file needs.
    fn middle_of(&self, index: usize) -> egui::Pos2 {
        let doc = self.app.chart_doc();
        let raster = doc
            .raster_rect
            .expect("a settled frame presented the raster");
        let plot = &doc.composed.plots[index];
        let l = &plot.layout;
        let x = plot.rect.x + (l.plot_x_start() + l.plot_x_end()) / 2.0;
        let y = plot.rect.y + (l.plot_y_start() + l.plot_y_end()) / 2.0;
        egui::pos2(raster.min.x + x as f32, raster.min.y + y as f32)
    }

    /// Press and release on plot `index` — two frames, because the chart's
    /// gesture machine is edge-triggered on the button state at the end of a
    /// frame and a press plus a release inside one frame leaves it unchanged.
    fn click_tile(&mut self, index: usize) {
        let pos = self.middle_of(index);
        self.run(vec![egui::Event::PointerMoved(pos), button_at(pos, true)]);
        self.run(vec![button_at(pos, false)]);
        self.settle();
    }
}

fn button_at(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// What the generator decided each column's type was, in the file's own order —
/// the answer the rail's note has to agree with.
fn decided_types(opened: &data_file::OpenedFile) -> Vec<(String, String)> {
    opened
        .protocol
        .columns
        .iter()
        .map(|c| {
            let tile = opened
                .dashboard
                .tiles()
                .iter()
                .find(|t| t.column() == c.column);
            let decided = match tile.map(brightfield_shell::dashboard::Tile::chosen_by) {
                Some(ChosenBy::Storage { type_name }) => type_name.clone(),
                Some(ChosenBy::Meaning { label, .. }) => {
                    label.rsplit('.').next().unwrap_or(label).to_string()
                }
                // A declined column has no tile, so nothing chose one from a
                // label; the rail shows what the engine stored it as.
                None => c.storage.clone(),
            };
            (c.column.clone(), decided)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// AC1 — the navigator rail lists the table and its columns
// ---------------------------------------------------------------------------

/// **The navigator rail lists one table asset and, beneath it, every column of
/// the file with the type the generator chose its tile from.**
///
/// Three separable claims in one test because they are one row set: the table
/// is there, every column of the file is under it at depth 1, and each one's
/// note is the type that decided its picture. An empty outline fails the first,
/// a missing column fails the second, and a note naming something other than
/// what `Tile::chosen_by` says fails the third.
#[test]
fn the_navigator_rail_lists_the_table_and_every_column_under_it() {
    let dir = TempDir::new("outline");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let opened = data_file::open(&path.to_string_lossy()).expect("an ordinary CSV opens");
    let inputs = opened.protocol.inputs().expect("the Protocol builds");
    let model = protocol::ProtocolModel::new(inputs, Flow::Vertical);

    let rows: Vec<(u8, String, String)> = model
        .outline()
        .into_iter()
        .map(|row| {
            let note = row
                .note
                .unwrap_or_else(|| brightfield_protocol::kind_label(row.kind).to_string());
            (row.depth, row.label, note)
        })
        .collect();
    assert!(
        !rows.is_empty(),
        "the outline is empty — the rail would still read `No assets yet` over a \
         file the window has open"
    );

    // One table asset, named after the file, at the top level.
    let tables: Vec<&(u8, String, String)> = rows
        .iter()
        .filter(|(depth, _, note)| *depth == 0 && note == "table")
        .collect();
    assert_eq!(
        tables.len(),
        1,
        "expected exactly one table asset in the outline, saw {rows:?}"
    );
    assert_eq!(tables[0].1, "harbour", "the table is named after the file");

    // The file it reads is there too, spelled the way the spec spells it.
    assert!(
        rows.iter()
            .any(|(depth, label, note)| *depth == 0 && note == "file" && label == "./harbour.csv"),
        "the file the step reads has no row: {rows:?}"
    );

    // Every column of the file, in the file's own order, under the table.
    let columns: Vec<&str> = rows
        .iter()
        .filter(|(depth, _, _)| *depth == 1)
        .map(|(_, label, _)| label.as_str())
        .collect();
    assert_eq!(
        columns, HARBOUR_COLUMNS,
        "the columns under the table are not the file's columns in the file's order"
    );
    assert!(
        columns.contains(&DECLINED),
        "`{DECLINED}` earns no tile, and the rail lists the TABLE's columns \
         rather than the dashboard's tiles — leaving it out would make a column \
         of the file invisible"
    );

    // …each with the type that decided its picture.
    let noted: Vec<(String, String)> = rows
        .iter()
        .filter(|(depth, _, _)| *depth == 1)
        .map(|(_, label, note)| (label.clone(), note.clone()))
        .collect();
    assert_eq!(
        noted,
        decided_types(&opened),
        "a column's note in the rail is not the type the generator chose its \
         tile from — left is what the rail drew, right is what `chosen_by` says"
    );
}

// ---------------------------------------------------------------------------
// AC2 — the Steps pane lists the one step
// ---------------------------------------------------------------------------

/// **The Steps pane lists exactly one step: a SQL read of the file, not run.**
///
/// Read off the sheet the pane renders (`data_grid::StepSheetRows` is a
/// projection of exactly these rows), so a step that is not emitted fails the
/// count and a step emitted as the wrong kind or with a run status it did not
/// earn fails the two assertions after it.
#[test]
fn the_steps_pane_lists_one_sql_step_that_has_not_run() {
    let dir = TempDir::new("steps");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let opened = data_file::open(&path.to_string_lossy()).expect("an ordinary CSV opens");
    let inputs = opened.protocol.inputs().expect("the Protocol builds");
    let model = protocol::ProtocolModel::new(inputs, Flow::Vertical);

    let rows = model.sheet().rows();
    assert_eq!(
        rows.len(),
        1,
        "a data file is one step, and the Steps pane draws one row per step: {:?}",
        rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>()
    );
    assert_eq!(rows[0].name, one_step::STEP_NAME);
    assert_eq!(rows[0].kind, "sql", "the step reads the file with SQL");
    assert_eq!(
        rows[0].detail,
        one_step::MODEL_PATH,
        "the row names the model the step runs"
    );
    assert_eq!(
        rows[0].status, "not run",
        "brightfield writes the spec and never a run record, so the step has to \
         say it has not run rather than showing a status it did not earn"
    );
}

// ---------------------------------------------------------------------------
// AC3 — a click on a tile fills the inspector rail
// ---------------------------------------------------------------------------

/// **Clicking a tile selects the column it draws, and the inspector shows that
/// column's name, its semantic type and the tile kind chosen for it.**
///
/// Driven through a real frame: the click is a pointer press and release at the
/// middle of a placed plot's data area, in the coordinates that frame presented
/// the raster at. What is read back is the document the inspector pane draws
/// from — with nothing selected it draws the `Nothing selected` empty state and
/// `selected_column` is `None`, which is the failure this test exists to catch.
#[test]
fn clicking_a_tile_selects_its_column_and_fills_the_inspector() {
    let dir = TempDir::new("tile-click");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let mut win =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));

    assert!(
        win.app.chart_doc().selected_column().is_none(),
        "nothing is selected before anything is clicked — otherwise this test \
         cannot tell the click apart from the initial state"
    );
    let tiles = win.app.chart_doc().tile_columns().len();
    assert!(
        tiles >= 2,
        "this fixture needs at least two tiles for the second click below to \
         mean anything; saw {tiles}"
    );

    win.click_tile(1);
    let picked = win.app.chart_doc().selected_column().cloned().expect(
        "clicking a tile selects the column it draws — the inspector \
                 would still read `Nothing selected`",
    );
    let expected = win.app.chart_doc().tile_columns()[1].clone();
    assert_eq!(
        picked.column, expected.column,
        "the click selected a different column from the one that tile draws"
    );
    assert!(
        picked.tile.is_some(),
        "a tile's column has to carry the kind drawn over it — the inspector's \
         `tile` line has nothing to say otherwise"
    );
    assert!(
        !picked.full_type().is_empty(),
        "the inspector names the column's type, and an empty one is a blank row"
    );

    // …and the navigator rail's highlight follows it, so the two rails cannot
    // name two different columns.
    let highlighted: Vec<String> = win
        .app
        .protocol_model()
        .outline()
        .into_iter()
        .filter(|r| r.depth == 1 && r.selected)
        .map(|r| r.label)
        .collect();
    assert_eq!(
        highlighted,
        vec![picked.column.clone()],
        "the outline highlights exactly the column the inspector is showing"
    );

    // A second tile moves the selection rather than adding to it.
    win.click_tile(0);
    assert_eq!(
        win.app
            .chart_doc()
            .selected_column()
            .map(|c| c.column.clone()),
        Some(win.app.chart_doc().tile_columns()[0].column.clone())
    );
}

// ---------------------------------------------------------------------------
// AC4 — Save, and the round trip back
// ---------------------------------------------------------------------------

/// **Save writes an `arcform.yaml` the pinned `arc` loader accepts, and
/// reopening that file regenerates the same dashboard over the same columns.**
///
/// The loader is `brightfield_protocol::parse_manifest_str`, which is
/// `arc::spec::Manifest::from_yaml_str` — the same gate `arc run` loads with,
/// linked from the pinned revision rather than reimplemented here.
///
/// The round trip goes through `Boot::open`, which is what `main` reaches and
/// what a front-door row reaches, so what is compared is the product's own
/// route rather than a second one written for the test.
#[test]
fn save_writes_a_spec_the_loader_accepts_and_reopening_it_regenerates_the_dashboard() {
    let dir = TempDir::new("round-trip");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let named = path.to_string_lossy().into_owned();

    let ctx = egui::Context::default();
    let mut win = Window::over(Boot::data_file(&named).expect("the file opens as a boot"));
    let before: Vec<(String, Option<String>)> = win
        .app
        .protocol_model()
        .columns()
        .iter()
        .map(|c| (c.column.clone(), c.tile.clone()))
        .collect();

    let saved = win
        .app
        .save_protocol(&ctx)
        .expect("a window over a data file has a Protocol to save")
        .expect("the save writes");
    assert_eq!(
        saved,
        dir.path().join("arcform.yaml"),
        "the spec is written beside the data it reads"
    );
    assert!(
        dir.path().join(one_step::MODEL_PATH).is_file(),
        "the step's model has to be written too — a manifest naming a model \
         that is not there is a Protocol `arc run` refuses at its first step"
    );

    // The pinned loader's own verdict on the bytes on disk.
    let text = std::fs::read_to_string(&saved).expect("the manifest reads back");
    let manifest = brightfield_protocol::parse_manifest_str(&text)
        .expect("the spec Save wrote has to load through arc's own gate");
    assert_eq!(manifest.steps.len(), 1);
    assert_eq!(
        manifest.steps[0].depends_on,
        vec!["./harbour.csv".to_string()],
        "the step declares the file it reads"
    );
    assert_eq!(
        manifest.steps[0].produces,
        vec!["harbour".to_string()],
        "…and the table it produces"
    );

    // Reopening the saved spec — the route `main` and the front door take.
    let reopened = Boot::open(&saved.to_string_lossy(), Flow::Vertical, None)
        .expect("the saved Protocol reopens");
    assert!(
        !reopened.graph_on_canvas(),
        "reopening a one-step Protocol lands on the dashboard, not on a \
         lineage graph over a run that never happened"
    );
    let win2 = Window::over(reopened);
    let after: Vec<(String, Option<String>)> = win2
        .app
        .protocol_model()
        .columns()
        .iter()
        .map(|c| (c.column.clone(), c.tile.clone()))
        .collect();
    assert_eq!(
        after, before,
        "reopening the saved spec drew a different set of tile kinds over a \
         different set of columns"
    );
    assert_eq!(
        win2.app.chart_doc().composed.plots.len(),
        win.app.chart_doc().composed.plots.len(),
        "the reopened dashboard places a different number of plots"
    );
}

/// A spec whose bytes the loader would refuse is **not written**: the gate runs
/// in memory, before the directory is touched.
///
/// Driven by handing `save_to` a manifest that will not load, which is the one
/// thing `OneStepProtocol` cannot produce on its own — so this is the guard on
/// the guard, not a restatement of the test above.
#[test]
fn a_spec_the_loader_refuses_is_not_written() {
    let dir = TempDir::new("refused");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let opened = data_file::open(&path.to_string_lossy()).expect("an ordinary CSV opens");
    let mut broken = opened.protocol.clone();
    // Two steps with one name — arc's validator refuses it, so this is a spec
    // `arc run` would not load either.
    broken.manifest = format!(
        "name: broken\nsteps:\n  - name: load\n    sql: {model}\n  - name: load\n    sql: {model}\n",
        model = one_step::MODEL_PATH
    );

    let out = TempDir::new("refused-out");
    let err = broken
        .save_to(out.path())
        .expect_err("a spec the loader refuses must not be written");
    assert!(
        err.contains("duplicate step name"),
        "the refusal carries arc's own diagnostic: {err}"
    );
    assert!(
        !OneStepProtocol::manifest_path_in(out.path()).exists(),
        "the gate ran after the write — a refused spec left a file behind"
    );
}

// ---------------------------------------------------------------------------
// AC5 — the front door lists it on the next launch
// ---------------------------------------------------------------------------

/// **On the next launch the front door's Protocols section lists the saved
/// Protocol, ahead of the curated Datasets.**
///
/// "The next launch" is a second window built over the layout the first one
/// wrote into — the same value `startup::boot_layout` would hand back — over
/// an empty boot, which is the front door. What is read is the frame: the sections
/// in the order they were drawn, and the rows drawn under the Protocols
/// heading, off the galleys the painter was handed.
#[test]
fn a_saved_protocol_leads_the_front_door_on_the_next_launch() {
    let dir = TempDir::new("front-door");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let ctx = egui::Context::default();

    let mut first =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));
    let saved = first
        .app
        .save_protocol(&ctx)
        .expect("there is a Protocol to save")
        .expect("the save writes");
    let layout = first.app.layout().clone();
    assert!(
        layout
            .recents
            .iter()
            .any(|r| r.id == saved.to_string_lossy()),
        "saving records the Protocol in the layout's recents, which is the only \
         thing a later launch has to go on: {:?}",
        layout.recents
    );

    // The next launch: the same layout, opened on nothing.
    let next = Window::with_layout(Boot::empty(), layout);
    assert!(
        next.app.front_door_is_live(),
        "a launch that opens nothing shows the front door"
    );
    let sections: Vec<&str> = next
        .app
        .front_door_sections()
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(
        sections,
        vec!["Protocols", "Datasets"],
        "with work to return to, the analyst's own Protocols lead the door and \
         the curated Datasets follow"
    );
    let rows: Vec<&str> = next
        .app
        .front_door_rows()
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert!(
        rows.contains(&"harbour"),
        "the saved Protocol has no row under the Protocols heading: {rows:?}"
    );
}

/// A recent naming a Protocol that has since been deleted draws no row — the
/// same rule a start id this build no longer ships already gets.
#[test]
fn a_saved_protocol_that_has_gone_draws_no_row() {
    let dir = TempDir::new("front-door-gone");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let ctx = egui::Context::default();

    let mut first =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));
    let saved = first
        .app
        .save_protocol(&ctx)
        .expect("there is a Protocol to save")
        .expect("the save writes");
    let mut layout = first.app.layout().clone();
    // The layout still remembers it; the file is gone.
    std::fs::remove_file(&saved).expect("the spec is removed");
    layout.opened = None;

    let next = Window::with_layout(Boot::empty(), layout);
    let rows: Vec<&str> = next
        .app
        .front_door_rows()
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert!(
        !rows.contains(&"harbour"),
        "a row whose click cannot land is worse than a shorter list: {rows:?}"
    );
    let sections: Vec<&str> = next
        .app
        .front_door_sections()
        .iter()
        .map(|(name, _)| *name)
        .collect();
    assert_eq!(
        sections,
        vec!["Datasets", "Protocols"],
        "with nothing left to return to the door is a first run again"
    );
}

/// Clicking the row reopens the Protocol as its dashboard — the whole point of
/// listing it.
#[test]
fn clicking_a_saved_protocols_row_reopens_it_as_the_dashboard() {
    let dir = TempDir::new("door-click");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let ctx = egui::Context::default();

    let mut first =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));
    let saved = first
        .app
        .save_protocol(&ctx)
        .expect("there is a Protocol to save")
        .expect("the save writes");
    let layout = first.app.layout().clone();

    let mut next = Window::with_layout(Boot::empty(), layout);
    let row = next
        .app
        .front_door_rows()
        .iter()
        .find(|r| r.id == saved.to_string_lossy())
        .expect("the saved Protocol has a row")
        .clone();
    let centre = row.rect.center();
    next.run(vec![
        egui::Event::PointerMoved(centre),
        button_at(centre, true),
        button_at(centre, false),
    ]);
    next.settle();

    assert!(
        !next.app.front_door_is_live(),
        "the click left the door up — nothing was opened"
    );
    assert!(
        !next.app.chart_doc().is_empty(),
        "the row reopened the Protocol without the dashboard it is a Protocol for"
    );
    assert_eq!(
        next.app
            .protocol_model()
            .columns()
            .iter()
            .map(|c| c.column.clone())
            .collect::<Vec<_>>(),
        HARBOUR_COLUMNS
            .iter()
            .map(|c| (*c).to_string())
            .collect::<Vec<_>>(),
        "the reopened window's rails list the file's columns"
    );
}

// ---------------------------------------------------------------------------
// The window as a whole
// ---------------------------------------------------------------------------

/// A window over a data file draws the **chart** on the canvas and the Protocol
/// in the rails — not the lineage graph.
///
/// The rule is `graph_takes_the_canvas`: the graph takes it when there is no
/// chart to hold it. Filling the protocol document was the change that could have moved
/// this, and moving it would replace the analyst's dashboard with a two-node
/// diagram.
#[test]
fn a_data_file_keeps_its_chart_on_the_canvas_now_that_the_rails_are_full() {
    let dir = TempDir::new("canvas");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let boot = Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot");
    assert!(
        !boot.graph_on_canvas(),
        "the canvas holds the chart; the rails hold the Protocol"
    );
    let win = Window::over(boot);
    assert!(!win.app.graph_on_canvas());
    assert!(
        win.app.protocol_model().has_assets(),
        "the rails have something to draw"
    );
}

/// The run state a saved Protocol is remembered in is **never run** — because
/// brightfield writes the spec and no run record, so there is no run to report.
#[test]
fn a_saved_protocol_is_remembered_as_never_run() {
    let dir = TempDir::new("run-state");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let ctx = egui::Context::default();
    let mut win =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));
    let saved = win
        .app
        .save_protocol(&ctx)
        .expect("there is a Protocol to save")
        .expect("the save writes");
    let layout = win.app.layout();
    let recent = layout
        .recents
        .iter()
        .find(|r| r.id == saved.to_string_lossy())
        .expect("the save is remembered");
    assert_eq!(recent.run, RunState::NeverRun);
    assert_eq!(recent.name, "harbour");
}
