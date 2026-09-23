//! **The table's one grid, and the two spots it draws in.**
//!
//! The grid is a view of the table node, not a pane that owns a record: it
//! draws beside the hero on the canvas or in the ledger rail's Rows spot, never
//! both in one frame, and it takes its column widths and its scroll with it
//! when it moves. Every assertion here reads a drawn frame of the housing
//! sample — where the table's header landed, what the Rows spot's galleys say,
//! how many tables the frame filed — rather than the spot field the window
//! latches, because a field can say *ledger* over a canvas still drawing the
//! grid.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_protocol::layout::Flow;
use brightfield_shell::app::{GridLayout, GridSpot};
use brightfield_shell::data_grid::{GRID_ON_CANVAS, MOVE_GRID};
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::NodeView;
use brightfield_shell::startup::{default_layout, opening_boot};
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::ComponentPath;
use brightfield_sql::ir::ScalarValue;
use brightfield_workbench::arrangement::{CANVAS, LEDGER_RAIL, STATUS_BAND};
use brightfield_workbench::RunState;

/// The housing sample every criterion opens.
fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// The ledger strip's Rows name: Log, Quality, Rows, Editor.
const ROWS_NAME: usize = 2;

/// The ledger strip's names in the order it draws them, and so the index each
/// is clicked at.
const LEDGER_NAMES: [&str; 4] = ["Log", "Quality", "Rows", "Editor"];

/// The height the ledger rail opened at over every pane before the Rows spot
/// asked for its rows. A number and not the constant it came from: what is
/// held is that no pane opens shorter than it did, and a floor read off the
/// declaration would move with the declaration.
const LEDGER_OPENED_AT: f32 = 180.0;

/// How many data rows the Rows spot has to show at its opening height.
const READABLE_RUN: usize = 5;

struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn over(boot: Boot, layout: brightfield_workbench::SavedLayout) -> Self {
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, layout, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
        };
        win.settle();
        win
    }

    fn housing() -> Self {
        let path = housing();
        Self::over(
            Boot::data_file(path.to_str().expect("utf-8 path")).expect("the sample opens"),
            default_layout(),
        )
    }

    /// One frame, and the text it handed the painter.
    fn run(&mut self, events: Vec<egui::Event>) -> Vec<(egui::Pos2, String)> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        let mut text = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut text);
        }
        text
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(at, true), button(at, false)]);
        self.settle();
    }

    /// Click the navigator spine's row labelled `label`, where the last frame
    /// drew it.
    fn pick_spine_row(&mut self, label: &str) {
        let at = self
            .app
            .spine_rows()
            .iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| {
                let drawn: Vec<&str> = self
                    .app
                    .spine_rows()
                    .iter()
                    .map(|r| r.label.as_str())
                    .collect();
                panic!("the spine drew no `{label}` row; it drew {drawn:?}")
            })
            .rect
            .center();
        self.click(at);
    }

    fn pick_rows(&mut self) {
        self.pick_ledger_name(ROWS_NAME);
    }

    /// Click the ledger strip's name at `index`, where the last frame drew it.
    fn pick_ledger_name(&mut self, index: usize) {
        let at = self
            .app
            .rail_name_rect(LEDGER_RAIL, index)
            .unwrap_or_else(|| {
                panic!(
                    "the ledger strip drew no {} name",
                    LEDGER_NAMES.get(index).unwrap_or(&"such")
                )
            })
            .center();
        self.click(at);
    }

    /// The part of the ledger rail a reader sees: the rail's rect, less the
    /// status band where that band floats over the rail's foot.
    fn ledger_seen(&self) -> egui::Rect {
        let ledger = self.rect(LEDGER_RAIL);
        match self.app.region_rect(STATUS_BAND) {
            Some(status) if status.intersects(ledger) => egui::Rect::from_min_max(
                ledger.min,
                egui::pos2(ledger.right(), status.top().min(ledger.bottom())),
            ),
            _ => ledger,
        }
    }

    /// Click `spot` on the grid's own spot switch, wherever the grid drew it.
    fn click_spot_switch(&mut self, spot: GridSpot) {
        let at = self
            .app
            .chart_doc()
            .grid_spot_switch
            .as_ref()
            .expect("the grid's header band drew its spot switch")
            .states
            .iter()
            .find(|(s, _)| *s == spot)
            .expect("the switch offers that spot")
            .1
            .center();
        self.click(at);
    }

    fn cmd_j(&mut self) {
        self.run(vec![egui::Event::Key {
            key: egui::Key::J,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }]);
        self.settle();
    }

    fn brush_longitude(&mut self, lo: f64, hi: f64) {
        let hero = ComponentPath(self.app.chart_doc().composed.plots[0].path.clone());
        assert!(self
            .app
            .chart_doc_mut()
            .apply_interaction(Interaction::Select {
                name: brightfield_shell::dashboard::SELECTION.to_string(),
                contributor: hero,
                predicate: SqlPredicate::Interval {
                    column: "longitude".to_string(),
                    lo: ScalarValue::Float(lo),
                    hi: ScalarValue::Float(hi),
                    meta: None,
                },
            }));
        self.settle();
    }

    fn rect(&self, region: brightfield_workbench::arrangement::RegionId) -> egui::Rect {
        self.app.region_rect(region).expect("the region drew")
    }

    /// Where this frame's one table put its first header cell.
    fn table_head(&self) -> egui::Rect {
        self.app
            .chart_doc()
            .grid_drawn()
            .expect("a table was drawn this frame")
            .header_cells
            .first()
            .expect("the table drew a header cell")
            .1
    }

    fn run_frames(&mut self, n: usize) {
        for _ in 0..n {
            self.run(Vec::new());
        }
    }

    /// The first column's value in the topmost row the table draws inside
    /// `within`, below its header — read off the galleys, at the header cell's
    /// own x range, so a band statistic above the rows is not mistaken for one.
    fn top_row_value(&mut self, within: egui::Rect) -> String {
        let head = self.table_head();
        let rows = egui::Rect::from_min_max(
            egui::pos2(head.left(), head.bottom() + 1.0),
            egui::pos2(head.right(), within.bottom()),
        )
        .intersect(within);
        let mut cells: Vec<(egui::Pos2, String)> = self
            .run(Vec::new())
            .into_iter()
            .filter(|(at, t)| rows.contains(*at) && t.parse::<f64>().is_ok())
            .collect();
        cells.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
        cells.first().expect("the table drew a row").1.clone()
    }

    fn pane_names(&self) -> Vec<&'static str> {
        self.app
            .canvas_panes()
            .panes
            .iter()
            .map(|p| p.name)
            .collect()
    }
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

fn collect_text(shape: &egui::epaint::Shape, into: &mut Vec<(egui::Pos2, String)>) {
    match shape {
        egui::epaint::Shape::Text(t) => into.push((t.pos, t.galley.text().to_string())),
        egui::epaint::Shape::Vec(shapes) => {
            for s in shapes {
                collect_text(s, into);
            }
        }
        _ => {}
    }
}

fn text_in(text: &[(egui::Pos2, String)], rect: egui::Rect) -> Vec<String> {
    text.iter()
        .filter(|(at, _)| rect.contains(*at))
        .map(|(_, t)| t.clone())
        .collect()
}

/// The sample's own count of rows with longitude in `lo..=hi`.
fn rows_between(lo: f64, hi: f64) -> u64 {
    let text = std::fs::read_to_string(housing()).expect("the sample reads");
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("a header").split(',').collect();
    let col = header
        .iter()
        .position(|h| *h == "longitude")
        .expect("a longitude column");
    lines
        .filter(|l| !l.trim().is_empty())
        .filter(|l| {
            let v: f64 = l
                .split(',')
                .nth(col)
                .expect("a cell")
                .parse()
                .expect("a number");
            (lo..=hi).contains(&v)
        })
        .count() as u64
}

/// **AC1 and AC2: one grid, in one spot per frame, narrowed to one count by a
/// brush in either spot.**
///
/// Walked as a reader walks it. The file opens with the grid beside the hero.
/// The Rows name on the ledger strip sends it to the ledger: the canvas is the
/// hero alone, the table's header lands inside the ledger, and the frame filed
/// one table. The grid's own spot switch, now on its band in the ledger, brings
/// it back: the canvas holds the pair again, and the ledger — still open on
/// Rows — draws the line saying where the grid is rather than a second table.
///
/// Under a brush the count is read in both spots and held to the sample's own
/// count inside the interval, so two spots agreeing on a wrong number fail.
///
/// Watched redden, three mutations: the ledger drawing the grid whatever the
/// canvas draws (`item == ROWS && !canvas_draws_grid` to `item == ROWS`) fails
/// on two tables filed after the return; the canvas ignoring the spot fails on
/// the pane group drawn beside a grid in the ledger; and the Rows spot's empty
/// state returning `None` fails on the missing line.
#[test]
fn one_grid_draws_in_one_spot_and_a_brush_narrows_it_in_either() {
    const LO: f64 = -122.5;
    const HI: f64 = -121.5;
    let inside = rows_between(LO, HI);
    assert!(
        inside > 0 && inside < 240,
        "the interval selects {inside} of the sample's rows, which narrows nothing"
    );

    let mut win = Window::housing();
    assert_eq!(
        win.app.grid_spot(),
        GridSpot::Canvas,
        "a file opens with the grid on the canvas"
    );
    assert_eq!(win.pane_names(), vec!["map", "grid"]);
    assert_eq!(win.app.chart_doc().tables_filed(), 1);

    // Into the ledger, by its Rows name.
    win.pick_rows();
    win.run(Vec::new());
    assert_eq!(win.app.grid_spot(), GridSpot::Ledger);
    let canvas = win.rect(CANVAS);
    let ledger = win.rect(LEDGER_RAIL);
    assert_eq!(
        win.pane_names(),
        vec!["map"],
        "with the grid in the ledger the canvas still drew a grid pane beside the hero"
    );
    let map = win
        .app
        .canvas_panes()
        .pane("map")
        .expect("the hero drew")
        .rect;
    assert!(
        (map.width() - canvas.width()).abs() <= 2.0,
        "the hero pane is {} wide on a canvas {} wide — it did not take the canvas",
        map.width(),
        canvas.width()
    );
    assert_eq!(
        win.app.chart_doc().tables_filed(),
        1,
        "one frame filed more than one table"
    );
    assert!(
        ledger.contains_rect(win.table_head().shrink(1.0)),
        "the table's header drew at {:?}, outside the ledger {ledger:?}",
        win.table_head()
    );
    assert_eq!(win.app.chart_doc().grid_drawn().expect("drawn").rows, 240);
    win.brush_longitude(LO, HI);
    assert_eq!(win.app.chart_doc().tables_filed(), 1);
    assert_eq!(
        win.app.chart_doc().grid_drawn().expect("drawn").rows,
        inside,
        "in the ledger the brushed grid does not list the rows inside the interval"
    );

    // Back onto the canvas, by the switch on the grid's own band.
    win.click_spot_switch(GridSpot::Canvas);
    let text = win.run(Vec::new());
    assert_eq!(win.app.grid_spot(), GridSpot::Canvas);
    assert_eq!(win.pane_names(), vec!["map", "grid"]);
    assert_eq!(
        win.app.chart_doc().tables_filed(),
        1,
        "with the grid back on the canvas and the ledger open on Rows, the frame \
         filed two tables — the Rows spot drew a second grid"
    );
    let grid_pane = win
        .app
        .canvas_panes()
        .pane("grid")
        .expect("the grid pane drew")
        .body;
    assert!(grid_pane.contains_rect(win.table_head().shrink(1.0)));
    let ledger = win.rect(LEDGER_RAIL);
    let said = text_in(&text, ledger);
    assert!(
        said.iter().any(|t| t == GRID_ON_CANVAS),
        "the ledger's Rows spot drew {said:?}, with no word about where the grid went"
    );
    assert_eq!(
        win.app.chart_doc().grid_drawn().expect("drawn").rows,
        inside,
        "on the canvas the same brush lists a different count than in the ledger"
    );
}

/// **AC3: a dragged column width and the row scrolled to both move with the
/// grid.**
///
/// The width is dragged at the first header cell's trailing edge on the canvas
/// and read back off the ledger's drawn header. The scroll is a wheel over the
/// canvas grid's body; what is compared is text — the first column's value at
/// the top of the canvas table after the scroll must be drawn in the ledger,
/// and the file's first row, drawn before the scroll, must not be. A move that
/// reset the scroll would draw the first row and not the one scrolled to.
///
/// Watched redden, two mutations: drawing the table under the spot's own `Ui`
/// id rather than `grid_state_id` fails the scroll half, and clearing the set
/// widths whenever the parent `Ui` id changes fails the width half.
#[test]
fn a_dragged_width_and_the_scroll_move_with_the_grid() {
    let mut win = Window::housing();
    let head = win.table_head();
    let first_value = win.top_row_value(win.app.canvas_panes().pane("grid").expect("grid").body);

    // Drag the first column's trailing edge 60 points wider.
    let edge = egui::pos2(head.right(), head.center().y);
    let to = edge + egui::vec2(60.0, 0.0);
    win.run(vec![egui::Event::PointerMoved(edge)]);
    win.run(vec![button(edge, true)]);
    win.run(vec![egui::Event::PointerMoved(
        edge + egui::vec2(20.0, 0.0),
    )]);
    win.run(vec![egui::Event::PointerMoved(to)]);
    win.run(vec![button(to, false)]);
    win.settle();
    let dragged = win.table_head().width();
    assert!(
        (dragged - (head.width() + 60.0)).abs() <= 1.5,
        "the first column is {dragged} wide after a 60-point drag from {}",
        head.width()
    );

    // Scroll the canvas grid down, let the wheel's animation finish, and read
    // the row now at the top of the table.
    let grid_body = win.app.canvas_panes().pane("grid").expect("grid").body;
    win.run(vec![egui::Event::PointerMoved(grid_body.center())]);
    win.run(vec![egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -600.0),
        modifiers: egui::Modifiers::default(),
        phase: egui::TouchPhase::Move,
    }]);
    win.run_frames(120);
    let top_value = win.top_row_value(grid_body);
    assert_ne!(top_value, first_value, "the wheel did not scroll the grid");

    win.pick_rows();
    win.run_frames(120);
    let ledger = win.rect(LEDGER_RAIL);
    assert!(
        ledger.contains_rect(win.table_head().shrink(1.0)),
        "the grid did not move"
    );
    assert!(
        (win.table_head().width() - dragged).abs() <= 1.0,
        "the first column is {} wide in the ledger where it was dragged to {dragged}",
        win.table_head().width()
    );
    let in_ledger = win.top_row_value(ledger);
    assert_eq!(
        in_ledger, top_value,
        "the ledger's table opens on {in_ledger}, where the reader had scrolled to \
         {top_value} — the file's first row is {first_value}"
    );
}

/// **AC4: the grid's spot is remembered per document across a relaunch.**
///
/// The grid is sent to the ledger, the Protocol saved and the layout flushed; a
/// second window over an empty boot, built on the file read back, opens the
/// saved Protocol and draws the hero alone with the grid in the ledger. The
/// second window is asserted to hold the grid on the canvas first, so the last
/// assertion cannot pass on a latch.
///
/// Watched redden, three mutations: dropping the `grid_spot_of` restore from
/// `open_protocol_path`, passing `GridSpot::default()` in `save_protocol`, and
/// writing the restored spot as a bare field rather than through
/// `set_grid_spot` — the rail defaults close the ledger, and the table is
/// drawn nowhere.
#[test]
fn the_grid_left_in_the_ledger_reopens_in_the_ledger() {
    let dir = std::env::temp_dir().join(format!("bf-grid-spot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the scratch dir");
    let csv = dir.join("housing.csv");
    std::fs::copy(housing(), &csv).expect("the sample copies");

    let mut win = Window::over(
        Boot::data_file(csv.to_str().expect("utf-8")).expect("the copy opens"),
        default_layout(),
    );
    win.cmd_j();
    assert_eq!(win.app.grid_spot(), GridSpot::Ledger);
    let saved = win
        .app
        .save_protocol(&win.ctx)
        .expect("a data file has a Protocol");
    let manifest = saved.expect("the Protocol saved");
    let layout_path = dir.join(brightfield_workbench::persist::LAYOUT_FILE);
    win.app
        .flush_layout(&layout_path)
        .expect("the save left the layout dirty")
        .expect("the layout wrote");
    let (restored, _) = brightfield_workbench::persist::load(&layout_path, default_layout);

    let mut reopened = Window::over(Boot::empty(), restored);
    assert_eq!(reopened.app.grid_spot(), GridSpot::Canvas);
    let spelled = manifest.to_str().expect("utf-8").to_string();
    reopened.app.open_protocol_path(&reopened.ctx, &spelled);
    reopened.settle();
    assert_eq!(
        reopened.app.grid_spot(),
        GridSpot::Ledger,
        "the Protocol was saved with the grid in the ledger and reopened with it on the canvas"
    );
    assert_eq!(reopened.pane_names(), vec!["map"]);
    let ledger = reopened.rect(LEDGER_RAIL);
    assert!(
        ledger.contains_rect(reopened.table_head().shrink(1.0)),
        "the table's header drew at {:?}, outside the ledger {ledger:?}",
        reopened.table_head()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **AC5: the move is a verb a reader can find, and both spots take it.**
///
/// The registry entry is built, bound and labelled; the chart palette offers
/// it; its key moves the grid out of the canvas and back; the grid's own spot
/// switch on the canvas sends it to the ledger. The Rows name and the ledger's
/// switch are walked in [`one_grid_draws_in_one_spot_and_a_brush_narrows_it_in_either`].
///
/// Watched redden, three mutations: deleting the `grid_key` call, deleting
/// `MOVE_GRID` from `CHART_PALETTE_VERBS`, and deleting the canvas group's
/// `record_spot_switch`.
#[test]
fn the_move_is_a_bound_verb_and_both_spots_take_it() {
    let reg = brightfield_keys::registry();
    let entry = reg
        .iter()
        .find(|v| v.longname == MOVE_GRID)
        .expect("move-grid is in the registry");
    assert!(!entry.is_reserved(), "move-grid is still reserved");
    assert_eq!(entry.primary_key(), Some("cmd-j"));
    assert!(!entry.help.is_empty());
    assert!(
        brightfield_shell::overlays::chart_palette_verbs(true).contains(&MOVE_GRID),
        "the chart palette does not offer the move"
    );

    let mut win = Window::housing();
    win.cmd_j();
    assert_eq!(
        win.app.grid_spot(),
        GridSpot::Ledger,
        "cmd-j did not move the grid to the ledger"
    );
    assert_eq!(win.pane_names(), vec!["map"]);
    win.cmd_j();
    assert_eq!(
        win.app.grid_spot(),
        GridSpot::Canvas,
        "cmd-j did not bring the grid back"
    );
    assert_eq!(win.pane_names(), vec!["map", "grid"]);

    win.click_spot_switch(GridSpot::Ledger);
    assert_eq!(
        win.app.grid_spot(),
        GridSpot::Ledger,
        "the spot switch on the grid's own band on the canvas did not move it"
    );
    assert!(win
        .rect(LEDGER_RAIL)
        .contains_rect(win.table_head().shrink(1.0)));
}

/// **Moving the grid out of the node's grid view leaves the canvas on the
/// dashboard view.**
///
/// The spine's `grid` row puts the grid on the canvas as the node's whole
/// view. `move-grid` from there sends it to the ledger, and a canvas still
/// holding the grid view would be the same table drawn a second time — so the
/// canvas falls back to the dashboard: the hero alone, the table's header in
/// the ledger.
///
/// Watched redden, one mutation: deleting the `CanvasHolds::View { view:
/// NodeView::Grid }` to `NodeView::Dashboard` reset inside `set_grid_spot`.
/// The canvas stays on the grid view, which draws the table, and the ledger
/// draws none.
#[test]
fn moving_the_grid_out_of_its_view_leaves_the_canvas_on_the_dashboard() {
    let mut win = Window::housing();
    win.pick_spine_row("grid");
    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Grid),
        "the spine's grid row did not put the node's grid view on the canvas"
    );
    assert_eq!(win.pane_names(), vec!["grid"]);

    win.cmd_j();
    win.run(Vec::new());
    assert_eq!(win.app.grid_spot(), GridSpot::Ledger);
    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Dashboard),
        "the grid moved to the ledger and the canvas is still on the grid view"
    );
    assert_eq!(
        win.pane_names(),
        vec!["map"],
        "the canvas did not come back to the hero alone"
    );
    let ledger = win.rect(LEDGER_RAIL);
    assert!(
        ledger.contains_rect(win.table_head().shrink(1.0)),
        "the table's header drew at {:?}, outside the ledger {ledger:?}",
        win.table_head()
    );
}

/// **A launch by the args route onto a document saved with the grid in the
/// ledger opens with it there.**
///
/// The route `main.rs` takes for a path on the command line —
/// `startup::opening_boot` over the path, into a window built over the saved
/// layout — rather than the front door's row, which
/// [`the_grid_left_in_the_ledger_reopens_in_the_ledger`] walks through
/// `open_protocol_path`. The two restores are different lines, and a mutation
/// of either leaves the other's test green.
///
/// Watched redden, one mutation: replacing the boot-route lookup in
/// `MeridianApp`'s constructor (`grid_spot_of(id)` off `opened_id`) with
/// `GridSpot::default()` — the window opens with the grid beside the hero.
#[test]
fn a_launch_over_a_saved_path_opens_with_the_grid_in_its_saved_spot() {
    let path = housing();
    let spelled = path.to_str().expect("utf-8 fixture path").to_string();
    let mut saved = default_layout();
    saved.remember(
        &spelled,
        "Housing",
        RunState::NeverRun,
        GridLayout::Rows,
        GridSpot::Ledger,
        1_000,
    );

    let boot =
        opening_boot(Some(&spelled), None, Flow::Vertical, None).expect("the named file opens");
    let mut win = Window::over(boot, saved);
    win.run(Vec::new());

    assert_eq!(
        win.app.grid_spot(),
        GridSpot::Ledger,
        "a launch straight onto a path saved with the grid in the ledger \
         opened with it on the canvas — the args route does not consult the \
         spot saved for the document it was told to open"
    );
    assert_eq!(win.pane_names(), vec!["map"]);
    let ledger = win.rect(LEDGER_RAIL);
    assert!(
        ledger.contains_rect(win.table_head().shrink(1.0)),
        "the table's header drew at {:?}, outside the ledger {ledger:?}",
        win.table_head()
    );
}

/// **The spine's `grid` row brings the grid back from the ledger, and the
/// document is saved with it on the canvas.**
///
/// With the grid in the ledger, the spine's `grid` row is a way back as well
/// as a view of the node. The `dashboard` row picked after it changes the
/// view and not the spot, so the canvas draws the pair again — the grid
/// beside the hero — and a save writes the canvas as the grid's spot.
///
/// Watched redden, one mutation: deleting the `self.grid_spot =
/// GridSpot::Canvas` the spine's view pick makes for the grid row. The spot
/// stays the ledger, so the dashboard view draws the hero alone and the save
/// writes the ledger.
#[test]
fn the_spines_grid_row_brings_the_grid_back_to_the_canvas() {
    let dir = std::env::temp_dir().join(format!("bf-grid-spot-spine-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the scratch dir");
    let csv = dir.join("housing.csv");
    std::fs::copy(housing(), &csv).expect("the sample copies");

    let mut win = Window::over(
        Boot::data_file(csv.to_str().expect("utf-8")).expect("the copy opens"),
        default_layout(),
    );
    win.pick_rows();
    assert_eq!(win.app.grid_spot(), GridSpot::Ledger);
    assert_eq!(win.pane_names(), vec!["map"]);

    win.pick_spine_row("grid");
    win.pick_spine_row("dashboard");
    win.run(Vec::new());
    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Dashboard),
        "the spine's dashboard row did not put the dashboard back on the canvas"
    );
    assert_eq!(
        win.pane_names(),
        vec!["map", "grid"],
        "after the spine's grid row and then its dashboard row, the canvas \
         does not draw the grid beside the hero"
    );
    let grid_pane = win
        .app
        .canvas_panes()
        .pane("grid")
        .expect("the grid pane drew")
        .body;
    assert!(grid_pane.contains_rect(win.table_head().shrink(1.0)));

    let manifest = win
        .app
        .save_protocol(&win.ctx)
        .expect("a data file has a Protocol")
        .expect("the Protocol saved");
    let layout_path = dir.join(brightfield_workbench::persist::LAYOUT_FILE);
    win.app
        .flush_layout(&layout_path)
        .expect("the save left the layout dirty")
        .expect("the layout wrote");
    let (restored, _) = brightfield_workbench::persist::load(&layout_path, default_layout);
    assert_eq!(
        restored.grid_spot_of(manifest.to_str().expect("utf-8")),
        Some(GridSpot::Canvas),
        "the grid was brought back by the spine's grid row and the save wrote \
         some other spot for it"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **AC1: the Rows spot opens on a readable run of rows.** The housing sample
/// at 1440 by 900, the grid sent to the ledger by the Rows name and the rail
/// left at the height it opens at: the table draws at least
/// [`READABLE_RUN`] data rows whole, under its compact band and inside the part
/// of the ledger a reader sees — the rail, less the status band floated over
/// its foot.
///
/// Counted off the rects the table drew its rows at, each held entirely inside
/// its own clip and the seen rail, so a row cut by the pane's foot or hidden
/// under the status band is not counted. No drag runs: a count reached by
/// dragging the rail taller is the defect this holds against.
#[test]
fn the_rows_spot_opens_on_a_readable_run_of_rows() {
    let mut win = Window::housing();
    win.pick_rows();
    assert_eq!(
        win.app.grid_spot(),
        GridSpot::Ledger,
        "the Rows name did not send the grid to the ledger"
    );
    let seen = win.ledger_seen();
    let head = win.table_head();
    assert!(
        seen.contains_rect(head.shrink(1.0)),
        "the table's header drew at {head:?}, outside the ledger {seen:?}"
    );
    let drawn = win
        .app
        .chart_doc()
        .grid_drawn()
        .expect("a table was drawn this frame")
        .clone();
    let under_band = egui::Rect::from_min_max(egui::pos2(seen.left(), head.bottom()), seen.max);
    let whole = drawn.rows_whole_within(under_band);
    assert!(
        whole >= READABLE_RUN,
        "the Rows spot opened at {:.0}pt and showed {whole} whole data rows \
         under its band, short of {READABLE_RUN} — the rows it laid out: {:?}",
        win.rect(LEDGER_RAIL).height(),
        drawn
            .row_cells
            .iter()
            .map(|(row, rect, _)| (*row, rect.top(), rect.bottom()))
            .collect::<Vec<_>>()
    );
}

/// **AC2: no ledger pane opens shorter than it did.** On the housing sample,
/// each of the strip's names opens the rail from its collapsed strip at least
/// [`LEDGER_OPENED_AT`] tall. Then each is clicked on a rail the grid is
/// already holding, so a record pane reached from the grid's taller rail is
/// read too: the two keep separate heights, and a record pane that inherited
/// the grid's would pass the floor while opening at the wrong one — which is
/// why the second walk also holds the record panes to the height the first
/// walk read.
#[test]
fn no_ledger_pane_opens_shorter_than_it_did() {
    let mut record = None;
    for (index, name) in LEDGER_NAMES.iter().enumerate() {
        for after_grid in [false, true] {
            let mut win = Window::housing();
            if after_grid {
                win.pick_rows();
            }
            win.pick_ledger_name(index);
            let height = win.rect(LEDGER_RAIL).height();
            let how = if after_grid {
                "on a rail the grid was holding"
            } else {
                "from the collapsed strip"
            };
            assert!(
                height >= LEDGER_OPENED_AT - 1e-3,
                "{name}, clicked {how}, drew the ledger rail {height:.1}pt tall, \
                 shorter than the {LEDGER_OPENED_AT}pt it opened at before"
            );
            if index == ROWS_NAME {
                continue;
            }
            let first = *record.get_or_insert(height);
            assert!(
                (height - first).abs() < 1e-3,
                "{name}, clicked {how}, drew the ledger rail {height:.1}pt tall \
                 where the record panes open at {first:.1}pt"
            );
        }
    }
}
