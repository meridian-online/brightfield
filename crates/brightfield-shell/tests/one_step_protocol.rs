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
//!
//! That leaves the *labelled* branch — the one a packaged build takes for every
//! column — unreached by anything an engine can produce here.
//! [`a_labelled_column_sends_its_leaf_to_the_rail_and_its_whole_label_to_the_inspector`]
//! reaches it without a bundle, by handing the generator a
//! [`ColumnProfile`](brightfield_engine::ColumnProfile) that carries a label
//! and driving the real inspector over the result.
//!
//! # What is read back is what the frame painted
//!
//! Where a claim is about a rail, it is asserted against the galleys the frame
//! handed the painter — [`Window::drawn_text`] — rather than against the
//! document field the rail is drawn from. Both were true in the round of this
//! file that was refused, and only one of them is what a person sees: the
//! inspector's whole column block could be deleted and every
//! document-field assertion would stay green.

use std::path::{Path, PathBuf};

use brightfield_engine::semantic::ValueCheck;
use brightfield_engine::{ColumnProfile, SemanticType};
use brightfield_protocol::layout::Flow;
use brightfield_shell::dashboard::{ChosenBy, Dashboard};
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

/// [`HARBOUR_CSV`]'s columns reordered so the **declined** one comes first.
///
/// The order is the assertion. `wire_columns` hands the chart document one
/// entry per tile in the composition's plot order; building that list by
/// filtering the *column* list instead is indistinguishable from it right up
/// until a column with no tile sits ahead of one that has a picture, at which
/// point index 0 is `survey` and plot 0 draws `station`.
const DECLINED_FIRST_CSV: &str = "survey,station,reading,depth\n\
                                  autumn,north,12,4.5\n\
                                  autumn,north,18,6.0\n\
                                  autumn,south,31,2.5\n\
                                  autumn,south,44,9.5\n\
                                  autumn,east,7,1.0\n\
                                  autumn,east,25,7.5\n\
                                  autumn,west,52,3.0\n\
                                  autumn,west,63,8.0\n";

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

    fn key(&mut self, key: egui::Key) {
        self.run(vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }]);
        self.run(Vec::new());
    }

    fn type_text(&mut self, text: &str) {
        self.run(vec![egui::Event::Text(text.to_owned())]);
        self.run(Vec::new());
    }

    /// Every string this window's next frame hands the painter **inside
    /// `rect`** — the device for asking what one rail drew rather than what
    /// the window drew.
    fn drawn_text_in(&mut self, rect: egui::Rect) -> Vec<String> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        let mut text = Vec::new();
        for clipped in &out.shapes {
            collect_text_in(&clipped.shape, rect, &mut text);
        }
        text
    }

    /// Pick rail `id`'s `index`-th name, at the place a pointer would find it.
    fn pick_rail_tab(&mut self, id: brightfield_workbench::arrangement::RegionId, index: usize) {
        let at = self
            .app
            .rail_name_rect(id, index)
            .expect("the rail drew that name")
            .center();
        self.run(vec![
            egui::Event::PointerMoved(at),
            button_at(at, true),
            button_at(at, false),
        ]);
        self.settle();
    }

    /// Every string this window's next frame hands the painter.
    ///
    /// Read off the frame's own shapes rather than off a document field,
    /// because the claim is about what a person sees — `one_window.rs` and
    /// `front_door.rs` read their rails the same way and for the same reason.
    fn drawn_text(&mut self) -> Vec<String> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        let mut text = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut text);
        }
        text
    }

    /// **Save, through the gesture a person has.** Open the chart command
    /// palette with `space`, type the verb's longname (an exact match ranks
    /// first) and confirm with enter — `overlay_wiring.rs`'s
    /// `confirm_chart_verb`, which is the path that sweep drives each chart
    /// verb through.
    ///
    /// Calling `MeridianApp::save_protocol` directly is what the refused round
    /// of this file did, and it proved the method rather than the product: no
    /// gesture in the shipped app produced the verb, so replacing the call
    /// with a no-op left the suite green.
    fn save_through_the_palette(&mut self) {
        assert!(
            self.app.has_protocol_to_save(),
            "this window has no Protocol behind it, so the palette will not \
             offer Save and the gesture below would be typing into a list that \
             does not contain it"
        );
        self.key(egui::Key::Space);
        assert_eq!(
            self.app.open_overlay(),
            Some("palette"),
            "space did not open the chart palette"
        );
        self.settle();
        // The row is there BEFORE it is typed. What a window offers is a
        // property of that window's state, and the other half of that claim —
        // that a chart-spec window is offered nothing — is
        // `overlay_wiring.rs::a_chart_start_is_offered_no_save`.
        let rows = self.app.open_palette_rows();
        assert!(
            rows.iter().any(|r| r == "save-spec"),
            "the palette over a window with a Protocol does not offer Save: \
             {rows:?}"
        );
        self.type_text("save-spec");
        self.key(egui::Key::Enter);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "confirming save-spec did not close the palette"
        );
        self.settle();
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

/// The galleys in `shape`, flattened. `Shape::Vec` nests, so a walk that reads
/// the top level and stops misses whatever a widget put inside a group.
fn collect_text(shape: &egui::epaint::Shape, into: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(t) => into.push(t.galley.text().to_string()),
        egui::epaint::Shape::Vec(shapes) => {
            for s in shapes {
                collect_text(s, into);
            }
        }
        _ => {}
    }
}

/// [`collect_text`], kept to the galleys drawn inside `rect`.
fn collect_text_in(shape: &egui::epaint::Shape, rect: egui::Rect, into: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(t) if rect.contains(t.pos) => {
            into.push(t.galley.text().to_string());
        }
        egui::epaint::Shape::Vec(shapes) => {
            for s in shapes {
                collect_text_in(s, rect, into);
            }
        }
        _ => {}
    }
}

/// One frame's worth of the `open-home` keystroke, cmd-shift-h. `command` and
/// `shift` are what `consume_key`'s logical match reads — mac_cmd/ctrl are
/// platform detail the pattern ignores — so this fires the same whichever
/// runner it is on. `front_door.rs`'s spelling, for the same reason.
fn press_home() -> Vec<egui::Event> {
    let modifiers = egui::Modifiers {
        command: true,
        shift: true,
        ..Default::default()
    };
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::Key {
            key: egui::Key::H,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        })
        .collect()
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
            let tile =
                opened.dashboard.tiles().iter().find(|t| {
                    t.column() == c.column || t.paired_column() == Some(c.column.as_str())
                });
            let decided = match tile.map(brightfield_shell::dashboard::Tile::chosen_by) {
                Some(ChosenBy::Storage { type_name }) => type_name.clone(),
                Some(ChosenBy::Meaning { label, .. }) => {
                    label.rsplit('.').next().unwrap_or(label).to_string()
                }
                // A point map is chosen from the pair rather than from either
                // column's own type, so the rail falls back to each column's
                // own label — or, with no label, to its storage type.
                Some(ChosenBy::CoordinatePair { .. }) => c.label.as_deref().map_or_else(
                    || c.storage.clone(),
                    |l| l.rsplit('.').next().unwrap_or(l).to_string(),
                ),
                // A declined column has no tile, so no label chose one for it;
                // the rail shows what the engine stored it as.
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

/// A coordinate pair: eight points around one city, plus a reading.
///
/// `longitude` and `latitude` are drawn as ONE point map — the fixture
/// `tests/point_map_baseline.rs` uses, in this file for the other half of that
/// fact: two column rows over one plot.
const COORDINATE_CSV: &str = "longitude,latitude,reading\n\
                              -122.40,37.77,12\n\
                              -122.42,37.75,18\n\
                              -122.41,37.79,25\n\
                              -122.43,37.78,9\n\
                              -122.39,37.76,31\n\
                              -122.44,37.80,14\n\
                              -122.38,37.74,22\n\
                              -122.45,37.81,6\n";

/// **Both halves of a coordinate pair are listed as drawn, and the two of them
/// share one plot.**
///
/// A point map is one tile over two columns, which is the one shape where the
/// navigator rail's list and the chart document's tile list cannot be the same
/// list. Matching a column to a tile by the tile's own column alone leaves
/// `latitude` reading as declined in a rail that is sitting beside a map of it;
/// filtering the column list to the ones with a tile puts two entries where the
/// composition places one plot, and every click from there on names the column
/// next door.
#[test]
fn both_halves_of_a_coordinate_pair_are_drawn_and_share_one_plot() {
    let dir = TempDir::new("coordinates");
    let path = dir.write("points.csv", COORDINATE_CSV);
    let mut win =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));

    let columns = win.app.protocol_model().columns().to_vec();
    let lon = columns
        .iter()
        .find(|c| c.column == "longitude")
        .expect("the rail lists longitude");
    let lat = columns
        .iter()
        .find(|c| c.column == "latitude")
        .expect("the rail lists latitude");
    assert!(
        lat.tile.is_some(),
        "latitude reads as declined in the rail while the canvas draws a map \
         of it: {}",
        lat.because
    );
    assert_eq!(lon.tile, lat.tile, "both halves name the same picture");
    assert_eq!(lon.paired.as_deref(), Some("latitude"));
    assert_eq!(lat.paired.as_deref(), Some("longitude"));

    // One plot for the pair, and the tile list is as long as the plot list.
    let tiles = win.app.chart_doc().tile_columns().len();
    assert_eq!(
        tiles,
        win.app.chart_doc().composed.plots.len(),
        "the window holds {tiles} tile columns for {} placed plots — a point \
         map is one tile over two columns, so a list built by filtering the \
         COLUMNS is one longer than the plots it indexes",
        win.app.chart_doc().composed.plots.len()
    );

    // …and a click on the map names a column that map draws.
    win.click_tile(0);
    let picked = win
        .app
        .chart_doc()
        .selected_column()
        .cloned()
        .expect("the click selects");
    let drawn_by_plot = plot_columns(&win.app, 0);
    assert!(
        drawn_by_plot.contains(&picked.column),
        "the inspector names `{}` for plot 0, which draws {drawn_by_plot:?}",
        picked.column
    );
    let after = win.drawn_text();
    assert!(
        after.iter().any(|t| t == "drawn with"),
        "the inspector says nothing about the other half of the pair, so a \
         reader looking at a map is told about one axis of it: {after:?}"
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

/// The columns the plot at `index` actually draws, off its
/// [`PlotHandle`](brightfield_shell::pipeline::PlotHandle) — the x and y
/// channels of its first mark, with the count column the SQL layer synthesises
/// dropped.
///
/// This is the independent side of AC3's join. The document's own
/// `tile_columns[n]` is what `wire_columns` put there; the plot handle is what
/// the composition placed. Comparing the inspector against the first is
/// circular — it is the same value read twice — and comparing it against the
/// second is the assertion that catches a list that has slipped out of step
/// with the plots.
fn plot_columns(app: &MeridianApp, index: usize) -> Vec<String> {
    let plot = &app.chart_doc().composed.plots[index];
    [plot.x_column.as_ref(), plot.y_column.as_ref()]
        .into_iter()
        .flatten()
        .filter(|c| !c.starts_with("__bf_"))
        .cloned()
        .collect()
}

/// **Clicking a tile selects the column that plot draws, and the inspector
/// draws that column's block.**
///
/// Two independent claims, and the refused round of this file had neither.
///
/// The first is the join: the column the inspector names is a column the
/// *clicked plot* draws, read off its `PlotHandle` rather than off the list
/// `wire_columns` built. The fixture's first column earns no tile, so a
/// tile-column list built by filtering the columns instead of by walking the
/// dashboard's tiles is off by one from plot 0 onwards — and the assertion
/// below is what says so.
///
/// The second is that the rail drew something: the column's name, the
/// `finetype` caption and its type, harvested from the frame's own galleys. An
/// inspector whose column block returned immediately leaves `selected_column`
/// set and the rail blank, which an assertion against the document alone
/// passes through.
#[test]
fn clicking_a_tile_selects_the_column_that_plot_draws_and_the_inspector_shows_it() {
    let dir = TempDir::new("tile-click");
    // `survey` is first and earns no tile — see DECLINED_FIRST_CSV.
    let path = dir.write("harbour.csv", DECLINED_FIRST_CSV);
    let mut win =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));

    assert!(
        win.app.chart_doc().selected_column().is_none(),
        "nothing is selected before anything is clicked — otherwise this test \
         cannot tell the click apart from the initial state"
    );
    let before = win.drawn_text();
    assert!(
        before.iter().any(|t| t == "Nothing selected"),
        "the inspector rail does not start on its empty state, so a later \
         assertion that it left that state proves nothing: {before:?}"
    );
    let tiles = win.app.chart_doc().tile_columns().len();
    assert_eq!(
        tiles,
        win.app.chart_doc().composed.plots.len(),
        "the window holds {tiles} tile columns for {} placed plots — a click \
         on the last plot would index the wrong column or none",
        win.app.chart_doc().composed.plots.len()
    );
    assert!(tiles >= 2, "this fixture needs two tiles; saw {tiles}");

    win.click_tile(0);
    let picked = win.app.chart_doc().selected_column().cloned().expect(
        "clicking a tile selects the column it draws — the inspector \
                 would still read `Nothing selected`",
    );

    // The join, against the composition rather than against the list under test.
    let drawn_by_plot = plot_columns(&win.app, 0);
    assert!(
        !drawn_by_plot.is_empty(),
        "plot 0 names no column on either channel, so the comparison below \
         would pass over an empty set"
    );
    assert!(
        drawn_by_plot.contains(&picked.column),
        "the inspector names `{}` for a click on plot 0, which draws {:?} — \
         the tile-column list is out of step with the plots the composition \
         placed",
        picked.column,
        drawn_by_plot
    );
    assert_ne!(
        picked.column, DECLINED,
        "the click named the column the generator DECLINED, which draws no \
         plot at all"
    );

    // …and the rail drew it.
    let after = win.drawn_text();
    for expected in [picked.column.as_str(), "finetype", "storage", "tile"] {
        assert!(
            after.iter().any(|t| t == expected),
            "the inspector rail drew no `{expected}` after the click, so the \
             column block is not on screen: {after:?}"
        );
    }
    assert!(
        after.iter().any(|t| t == picked.full_type()),
        "the inspector rail drew no type for the selected column: {after:?}"
    );
    assert!(
        !after.iter().any(|t| t == "Nothing selected"),
        "the inspector rail is still on its empty state after a click"
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

    // A second tile moves the selection rather than adding to it, and lands on
    // that plot's column too.
    win.click_tile(1);
    let second = win
        .app
        .chart_doc()
        .selected_column()
        .cloned()
        .expect("the second click selects");
    assert_ne!(second.column, picked.column, "the selection did not move");
    assert!(
        plot_columns(&win.app, 1).contains(&second.column),
        "the second click named a column plot 1 does not draw"
    );
}

/// **A labelled column sends its leaf to the navigator rail and its whole
/// label to the inspector.**
///
/// The branch a packaged build takes for every column, and the one no engine
/// in this test binary can produce: `LoadOptions::packaged` finds no FineType
/// bundle beside a `cargo test` executable, so every profiled column arrives
/// `SemanticType::NotAsked`. The profile is therefore built here, carrying a
/// label, and put through the real generator and the real inspector.
///
/// Four claims, because the rail and the inspector deliberately show different
/// amounts of one string: the rail's note is the label's **leaf** (240 logical
/// points do not hold `representation.numeric.decimal_number` beside a column
/// name), the inspector's `finetype` row is the **whole** label, `storage`
/// stays the DuckDB type, and the tile's reason names the semantic type rather
/// than the storage one.
#[test]
fn a_labelled_column_sends_its_leaf_to_the_rail_and_its_whole_label_to_the_inspector() {
    const LABEL: &str = "representation.numeric.decimal_number";
    let profile = ColumnProfile {
        name: "median_income".to_string(),
        type_name: "DOUBLE".to_string(),
        non_null: 16_640,
        nulls: 0,
        distinct: 12_000,
        min: Some("0.4999".to_string()),
        max: Some("15.0001".to_string()),
        semantic: SemanticType::Labelled {
            label: LABEL.to_string(),
            confidence: 0.99,
            check: ValueCheck::Checked {
                checked: 100,
                failed: 0,
            },
        },
    };
    let path = Path::new("/data/california_housing.parquet");
    let dashboard = Dashboard::of(path, std::slice::from_ref(&profile));
    assert_eq!(
        dashboard.tiles().len(),
        1,
        "the generator declined the labelled column, so there is no tile to \
         read a reason off: {:?}",
        dashboard.omitted()
    );
    assert!(
        matches!(dashboard.tiles()[0].chosen_by(), ChosenBy::Meaning { .. }),
        "the label did not decide the tile, so this fixture is exercising the \
         storage branch under a different name: {:?}",
        dashboard.tiles()[0].chosen_by()
    );

    let spec = OneStepProtocol::of(path, std::slice::from_ref(&profile), &dashboard);
    let facts = &spec.columns[0];
    assert_eq!(
        facts.leaf, "decimal_number",
        "the navigator rail draws the label's leaf, not the whole of it"
    );
    assert_eq!(
        facts.full_type(),
        LABEL,
        "the inspector's `finetype` row is the whole label"
    );
    assert_eq!(facts.storage, "DOUBLE", "storage is the DuckDB type");
    assert!(
        facts.because.contains(LABEL),
        "the tile's reason has to name the semantic type it was chosen from: {}",
        facts.because
    );

    // …and the real inspector draws it. The document is a real window's — only
    // the tile columns are this fixture's, because a labelled profile cannot
    // come out of the engine here.
    let dir = TempDir::new("labelled-inspector");
    let real = dir.write("harbour.csv", HARBOUR_CSV);
    let mut win =
        Window::over(Boot::data_file(&real.to_string_lossy()).expect("the file opens as a boot"));
    win.app.chart_doc_mut().set_tile_columns(spec.tiles.clone());
    win.app.chart_doc_mut().select_tile(0);
    win.settle();
    let drawn = win.drawn_text();
    assert!(
        drawn.iter().any(|t| t == LABEL),
        "the inspector rail drew no whole label for a labelled column — it \
         shows the leaf, or nothing: {drawn:?}"
    );
    assert!(
        drawn.iter().any(|t| t == "DOUBLE"),
        "the inspector rail drew no storage type beside the label: {drawn:?}"
    );
}

/// **The inspector rail draws no Save on either kind of window, while the
/// palette on the data-file window offers one** — the two surfaces read off
/// the same two windows, in the same frames.
///
/// The rail draws a *pane's* toolbar entry, and the entry declared for
/// `save-spec` is `EditorPane`'s: the editor's own buffer save. The verb,
/// dispatched, writes the Protocol. One name, two writes — so a Save button in
/// this rail is dead over a clean buffer and wrong over a dirty one, where the
/// click saves the Protocol and reports success over an edit that was never
/// written. It draws neither, and the palette carries the Protocol save, where
/// the row is the verb rather than a pane's button.
///
/// Both windows carry a spec file, so the editor pane has a buffer and
/// declares its Save entry in either case — otherwise the absences below would
/// be true for the wrong reason. The rail naming that file is the guard.
#[test]
fn the_inspector_rail_draws_no_save_while_the_palette_offers_one() {
    use brightfield_workbench::arrangement::{INSPECTOR_RAIL, LEDGER_RAIL};

    /// Focus the editor pane over `win` and read the inspector rail's text.
    fn rail_text(win: &mut Window) -> Vec<String> {
        // The Editor is the ledger rail's second name; the pane has to DRAW
        // before it opens a buffer, and it only draws when its tab is picked.
        win.pick_rail_tab(LEDGER_RAIL, 1);
        assert!(
            win.app.focus_pane(brightfield_workbench::PaneKey::new(
                brightfield_shell::editor::EDITOR
            )),
            "the editor pane is not in this window's tree"
        );
        win.settle();
        let rect = win
            .app
            .region_rect(INSPECTOR_RAIL)
            .expect("the inspector rail drew");
        win.drawn_text_in(rect)
    }

    // A window over a chart spec: a buffer to save, and no Protocol.
    let spec = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/dashboard.yaml");
    let mut chart = Window::over(
        Boot::open(spec.to_str().expect("utf-8"), Flow::Vertical, None).expect("the spec opens"),
    );
    assert!(!chart.app.has_protocol_to_save());
    let chart_rail = rail_text(&mut chart);
    // The editor pane titles its subject from its open FILE, so the rail
    // naming one is proof the pane has a buffer — and `EditorPane::describe`
    // declares its Save entry under exactly that condition. Without this the
    // absences below could be an editor that never opened anything.
    assert!(
        chart_rail.iter().any(|t| t == "dashboard.yaml"),
        "the rail is not naming a focused editor with a buffer, so the \
         toolbar it filters is empty and the assertion below proves nothing: \
         {chart_rail:?}"
    );
    assert!(
        !chart_rail.iter().any(|t| t == "Save"),
        "the inspector rail drew a Save button on a window with no Protocol \
         behind it — pressing it reaches `save_protocol`, finds no source and \
         returns: {chart_rail:?}"
    );

    // A window a data file opened: the same focused pane, and a Protocol —
    // and the rail still draws no Save, because the entry is the editor's.
    let dir = TempDir::new("rail-save");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let mut data =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));
    assert!(data.app.has_protocol_to_save());
    let data_rail = rail_text(&mut data);
    assert!(
        data_rail.iter().any(|t| t == "harbour.yaml"),
        "the rail is not naming a focused editor with a buffer over the \
         generated spec: {data_rail:?}"
    );
    assert!(
        !data_rail.iter().any(|t| t == "Save"),
        "the inspector rail drew the EDITOR's Save on a window whose \
         `save-spec` writes the PROTOCOL — a dirty buffer clicked there is \
         reported saved and is not: {data_rail:?}"
    );

    // …and the surface that does offer it, on the same window, in the same
    // state: the palette, where the row is the verb and not a pane's button.
    data.key(egui::Key::Space);
    assert_eq!(data.app.open_overlay(), Some("palette"));
    data.settle();
    let rows = data.app.open_palette_rows();
    assert!(
        rows.iter().any(|r| r == "save-spec"),
        "the Protocol save is offered nowhere at all on a window that has \
         one to write: {rows:?}"
    );
    data.key(egui::Key::Escape);
}

/// **Going Home takes the Save offer with the document.**
///
/// A window walked from a data file, home, and into a shipped chart start. The
/// palette is built from a flag written fresh each frame; going Home empties
/// the protocol document without going through the adoption path, so a flag
/// that went up and stayed up would leave the start offering a Save that
/// reaches `save_protocol`, finds no source and returns.
///
/// The start is `signals-dashboard`, one of the four shipped starts that open
/// a chart spec.
#[test]
fn going_home_takes_the_save_offer_with_the_start() {
    let dir = TempDir::new("home-then-start");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let mut win =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));

    // 1. The data file: a Protocol, and the offer.
    assert!(win.app.has_protocol_to_save());
    win.key(egui::Key::Space);
    assert_eq!(win.app.open_overlay(), Some("palette"));
    win.settle();
    assert!(
        win.app.open_palette_rows().iter().any(|r| r == "save-spec"),
        "the fixture starts without the offer, so its disappearance below \
         would prove nothing"
    );
    win.key(egui::Key::Escape);

    // 2. Home. Both documents are emptied in place.
    win.run(press_home());
    win.settle();
    assert!(
        win.app.front_door_is_live(),
        "cmd-shift-h did not reach the front door"
    );
    assert!(!win.app.has_protocol_to_save());

    // 3. A shipped chart start, off the door's own card.
    let card = win
        .app
        .front_door_card_rect(brightfield_shell::starts::DASHBOARD)
        .expect("the door draws a card for the signals dashboard")
        .center();
    win.run(vec![
        egui::Event::PointerMoved(card),
        button_at(card, true),
        button_at(card, false),
    ]);
    win.settle();
    assert!(
        !win.app.front_door_is_live(),
        "the card click opened nothing"
    );
    assert!(
        !win.app.has_protocol_to_save(),
        "a shipped chart start has no Protocol behind it"
    );

    win.key(egui::Key::Space);
    assert_eq!(win.app.open_overlay(), Some("palette"));
    win.settle();
    let rows = win.app.open_palette_rows();
    assert!(
        rows.iter().any(|r| r == "clear-selection"),
        "the palette is missing the verbs every chart window offers, so this \
         is not the chart palette: {rows:?}"
    );
    assert!(
        !rows.iter().any(|r| r == "save-spec"),
        "the Save offer outlived the document it was true of — confirming it \
         here closes the palette and does nothing: {rows:?}"
    );
    win.key(egui::Key::Escape);
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

    let mut win = Window::over(Boot::data_file(&named).expect("the file opens as a boot"));
    let before: Vec<(String, Option<String>)> = win
        .app
        .protocol_model()
        .columns()
        .iter()
        .map(|c| (c.column.clone(), c.tile.clone()))
        .collect();

    let saved = dir.path().join("arcform.yaml");
    assert!(
        !saved.exists(),
        "the spec is unsaved until the gesture — otherwise the assertion \
         below cannot tell the Save apart from the open"
    );
    win.save_through_the_palette();
    assert!(
        saved.is_file(),
        "the palette's Save wrote nothing to {} — the verb reached no \
         handler, which is what happens when `save-spec` has no producer at \
         the chart altitude",
        saved.display()
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

/// A **model** the loader would never look at, and Save refuses it anyway.
///
/// `arc::spec::Manifest::from_yaml_str` touches no filesystem, so a manifest
/// naming a `sql:` model that is malformed — or absent — is a manifest it
/// accepts. The manifest gate alone would write both files and report success
/// over a model nothing can read. `save_to` puts the model through the same
/// derivation the rails use and refuses a spec that would draw an issue chip.
#[test]
fn a_model_that_will_not_parse_is_not_written() {
    let dir = TempDir::new("bad-model");
    let path = dir.write("harbour.csv", HARBOUR_CSV);
    let opened = data_file::open(&path.to_string_lossy()).expect("an ordinary CSV opens");
    let mut broken = opened.protocol.clone();
    broken.model =
        "CREATE OR REPLACE TABLE harbour AS SELECT * FROM read_csv('unterminated;\n".to_string();

    // The manifest is untouched, so the loader is happy with it — which is the
    // point: the refusal below cannot come from that gate.
    brightfield_protocol::parse_manifest_str(&broken.manifest)
        .expect("the manifest itself is still valid");

    let out = TempDir::new("bad-model-out");
    let err = broken
        .save_to(out.path())
        .expect_err("a model that does not parse must not be written");
    assert!(
        err.contains("clean graph"),
        "the refusal names what failed: {err}"
    );
    assert!(
        !OneStepProtocol::manifest_path_in(out.path()).exists(),
        "the manifest was written beside a model that cannot be read"
    );
    assert!(
        !out.path().join(one_step::MODEL_PATH).exists(),
        "the model was written after the gate refused it"
    );
}

/// **A file name with an apostrophe in it.**
///
/// `data_file::accept` admits one — it is not glob syntax and not a control
/// character — so it reaches the spec, where the path crosses two languages
/// that both need it escaped and are escaped separately: a YAML scalar in
/// `depends_on:` and a SQL string literal in the model. Doubling it in one and
/// not the other writes an unterminated literal to `models/load.sql` and
/// reports success.
///
/// Held three ways, none of which needs a network or an external binary:
///
/// 1. the spec's own graph derivation, which parses the model with the same
///    sqlparser the rails use, resolves **one** file node and reports no
///    degrade — an unterminated literal degrades the step to an issue chip;
/// 2. `save_to`, which now refuses such a spec rather than writing it, so a
///    written spec is one that parsed;
/// 3. **DuckDB itself**, executing the written model. The engine's own
///    dependency, on the version the shell links, run from the protocol
///    directory because that is where `arc run` resolves a relative
///    `depends_on:` from.
#[test]
fn a_file_name_with_an_apostrophe_survives_both_escapes() {
    let dir = TempDir::new("apostrophe");
    let path = dir.write("it's harbour.csv", HARBOUR_CSV);
    let opened = data_file::open(&path.to_string_lossy()).expect("an apostrophe is openable");
    let spec = &opened.protocol;
    assert_eq!(spec.spelled, "./it's harbour.csv");
    assert_eq!(
        spec.name, "it_s_harbour",
        "a stem with an apostrophe and a space is sanitised into an unquoted \
         identifier, so the model's CREATE TABLE target needs no quoting"
    );
    assert!(
        spec.model.contains("read_csv('./it''s harbour.csv')"),
        "the SQL literal doubles the apostrophe: {}",
        spec.model
    );
    assert!(
        spec.manifest.contains("'./it''s harbour.csv'"),
        "the YAML scalar doubles it too: {}",
        spec.manifest
    );

    // 1. The graph reads one file, and nothing degraded.
    let inputs = spec.inputs().expect("the Protocol builds");
    assert!(
        inputs.degrade_report().is_empty(),
        "the model did not parse: {:?}",
        inputs.degrade_report()
    );
    let files: Vec<String> = inputs
        .graph_full
        .nodes
        .values()
        .filter(|n| n.kind == brightfield_protocol::AssetKind::File)
        .map(|n| n.label.clone())
        .collect();
    assert_eq!(
        files,
        vec!["./it's harbour.csv".to_string()],
        "the two escapes decoded to different strings, so the graph holds two \
         nodes for one file"
    );

    // 2. It is written rather than refused.
    let saved = spec.save_to(dir.path()).expect("the spec saves");
    assert!(saved.is_file());
    let model = std::fs::read_to_string(dir.path().join(one_step::MODEL_PATH))
        .expect("the model reads back");

    // 3. DuckDB runs it, from the directory arc would run it from.
    let here = std::env::current_dir().expect("a working directory");
    std::env::set_current_dir(dir.path()).expect("the protocol directory");
    let conn = duckdb::Connection::open_in_memory().expect("an in-memory DuckDB");
    let ran = conn.execute_batch(&model);
    // The table is named from the sanitised stem — `it's harbour` cannot be an
    // unquoted identifier, so it is `it_s_harbour` — and the count is asked of
    // the name the spec declares rather than of one typed here.
    let counted: Result<i64, _> =
        conn.query_row(&format!("SELECT count(*) FROM {}", spec.name), [], |r| {
            r.get(0)
        });
    std::env::set_current_dir(here).expect("the working directory is restored");
    ran.unwrap_or_else(|e| panic!("DuckDB refused the model brightfield wrote: {e}\n{model}"));
    assert_eq!(
        counted.expect("the table is there to count"),
        8,
        "the model read a different file, or none"
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
    let mut first =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));
    first.save_through_the_palette();
    let saved = dir.path().join("arcform.yaml");
    assert!(saved.is_file(), "the palette's Save wrote the spec");
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
    let mut first =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));
    first.save_through_the_palette();
    let saved = dir.path().join("arcform.yaml");
    assert!(saved.is_file(), "the palette's Save wrote the spec");
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
    let mut first =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));
    first.save_through_the_palette();
    let saved = dir.path().join("arcform.yaml");
    assert!(saved.is_file(), "the palette's Save wrote the spec");
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
    let mut win =
        Window::over(Boot::data_file(&path.to_string_lossy()).expect("the file opens as a boot"));
    win.save_through_the_palette();
    let saved = dir.path().join("arcform.yaml");
    assert!(saved.is_file(), "the palette's Save wrote the spec");
    let layout = win.app.layout();
    let recent = layout
        .recents
        .iter()
        .find(|r| r.id == saved.to_string_lossy())
        .expect("the save is remembered");
    assert_eq!(recent.run, RunState::NeverRun);
    assert_eq!(recent.name, "harbour");
}
