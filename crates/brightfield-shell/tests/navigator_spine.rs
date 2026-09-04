//! **The navigator rail's pane is the Protocol** — a spine of what the file is,
//! what read it, what that made, and how you can look at what it made.
//!
//! Each assertion here reads a **laid-out frame**: the rows come off
//! [`MeridianApp::spine_rows`], which the pane fills as it draws, and the
//! markers and the header text come off the shapes the frame painted. Nothing
//! here asks the model what it would have drawn — a test that did would stay
//! green through a pane that drew none of it, which is the failure the rows
//! hook exists to make impossible.
//!
//! # What is covered and what is not
//!
//! Covered: the rows and their order, the two marks and the one mechanism each,
//! what a click on a view row moves, and the contract's measurements at two
//! window sizes. Not covered here: the pixels. The four re-photographed
//! baselines in `tests/surfaces.rs` and the two in `tests/dashboard_baseline.rs`
//! are that half, and they are a different kind of evidence — an image reddens
//! on a font bump as loudly as on a dropped row, which is why the structural
//! half is here and reads in sentences.

use brightfield_protocol::contract_graph::{AssetMeta, SeamStatus};
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::{NodeView, SpineMarker, SpineRole, SpineRowDrawn};
use brightfield_shell::window::{Boot, CanvasHolds, MeridianApp};
use meridian_design::{control, semantic, spacing};

/// The committed table every window in this file is opened over: 240 rows, nine
/// columns, one coordinate pair.
fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// A boot over [`housing`], as the front door's picker and
/// `brightfield-shot --spec table.csv` both build it.
fn housing_boot() -> Boot {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

/// A window that keeps its own `egui::Context` for its whole life, because a
/// click is resolved against the widget id a *previous* frame registered — the
/// same harness `tests/arrangement.rs` uses and for the same reason.
struct Live {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Live {
    /// A window over `boot` at an explicit logical size.
    fn at(boot: Boot, size: (f32, f32)) -> Self {
        Self {
            app: MeridianApp::headless(boot, Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size.0, size.1)),
        }
    }

    /// A window over `boot` at the size that boot asks for — the dashboard
    /// baseline's window.
    fn open(boot: Boot) -> Self {
        let size = boot.window_size();
        Self::at(boot, size)
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

    /// Three frames with no events — one more than the layout needs, for the
    /// reason `tests/arrangement.rs` runs three.
    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new(), Vec::new()]);
    }

    /// One more frame with no events, handing back every shape it painted.
    fn shapes(&mut self) -> Vec<egui::epaint::ClippedShape> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        self.ctx.run_ui(raw, |ui| self.app.draw(ui)).shapes
    }

    /// The rows the rail drew, cloned off the last frame.
    fn rows(&self) -> Vec<SpineRowDrawn> {
        self.app.spine_rows().to_vec()
    }

    /// The row whose label is `label` — panics naming what was drawn instead,
    /// so a dropped row fails with a list rather than with an index.
    fn row(&self, label: &str) -> SpineRowDrawn {
        let rows = self.rows();
        rows.iter()
            .find(|row| row.label == label)
            .cloned()
            .unwrap_or_else(|| {
                let drawn: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
                panic!("the rail drew no row labelled {label:?}; it drew {drawn:?}")
            })
    }

    /// Click where the last frame drew the row labelled `label`.
    fn click_row(&mut self, label: &str) {
        let at = self.row(label).rect.center();
        self.run(vec![click_at(at), Vec::new(), Vec::new()]);
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

/// Every text galley the frame painted, with the rect it landed in.
///
/// Recursive, because a pane's chrome nests its shapes: a `Shape::Vec` holding
/// a fill and a stroke is one row's wash, and the galley under it is a level
/// down.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) => {
                out.push((
                    text.galley.text().to_string(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                ));
            }
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    walk(s, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// Every circle the frame painted.
fn circles(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::epaint::CircleShape> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::epaint::CircleShape>) {
        match shape {
            egui::Shape::Circle(circle) => out.push(*circle),
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    walk(s, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// A design token as the colour egui painted it.
fn ink(token: meridian_design::colour::Rgba) -> egui::Color32 {
    brightfield_workbench::chrome::colour(token)
}

/// The label, kind, depth and marker of one row — what a failure prints.
fn shape_of(row: &SpineRowDrawn) -> (SpineRole, &str, &str, u8, SpineMarker) {
    (
        row.role,
        row.label.as_str(),
        row.kind.as_str(),
        row.depth,
        row.marker,
    )
}

/// The nine columns of the fixture, in the file's own order, with the leaf of
/// the type each row draws at its trailing end.
const HOUSING_COLUMNS: &[(&str, &str)] = &[
    ("median_income", "DOUBLE"),
    ("house_age", "BIGINT"),
    ("avg_rooms", "DOUBLE"),
    ("avg_bedrooms", "DOUBLE"),
    ("population", "BIGINT"),
    ("avg_occupancy", "DOUBLE"),
    ("latitude", "DOUBLE"),
    ("longitude", "DOUBLE"),
    ("median_house_value", "DOUBLE"),
];

// ---------------------------------------------------------------------------
// AC1 — the rows, in order, each with its label, kind, depth and marker
// ---------------------------------------------------------------------------

/// **AC1.** Opening the fixture draws the spine: the file, the step that reads
/// it with its kind and run state, the table, that table's two views as child
/// rows, then the outline's caption and the column rows.
///
/// The whole list is asserted, in order, rather than a row at a time: a missing
/// row, a row in the wrong place and a step row carrying a run state the model
/// does not hold each fail here, and each fails printing the list that was
/// drawn.
#[test]
fn the_spine_lists_the_file_the_step_that_read_it_and_the_table_it_made() {
    let mut win = Live::open(housing_boot());
    win.settle();

    let rows = win.rows();
    let drawn: Vec<(SpineRole, &str, &str, u8, SpineMarker)> = rows.iter().map(shape_of).collect();

    let mut want: Vec<(SpineRole, &str, &str, u8, SpineMarker)> = vec![
        (
            SpineRole::Caption,
            "SPINE   \u{b7}   1 step",
            "",
            0,
            SpineMarker::None,
        ),
        // The file is on disk — the Protocol was opened by reading it — so it
        // is the one thing here that exists.
        (
            SpineRole::Asset,
            "./california_housing_sample.csv",
            "file",
            0,
            SpineMarker::Filled,
        ),
        // Brightfield writes the spec and runs no step, so this one says so
        // in the words `status_word` uses elsewhere.
        (
            SpineRole::Step,
            "load",
            "sql \u{b7} not run",
            0,
            SpineMarker::Hollow,
        ),
        (
            SpineRole::Asset,
            "california_housing_sample",
            "table",
            0,
            SpineMarker::Hollow,
        ),
        (SpineRole::View, "dashboard", "view", 1, SpineMarker::None),
        (SpineRole::View, "grid", "view", 1, SpineMarker::None),
        // No table clause: at 240 points the caption that named it clipped
        // mid-word and took the count off the edge with it, and the table's own
        // name is three rows above in full.
        (
            SpineRole::Caption,
            "OUTLINE   \u{b7}   9 columns",
            "",
            0,
            SpineMarker::None,
        ),
    ];
    for (column, leaf) in HOUSING_COLUMNS {
        want.push((SpineRole::Column, column, leaf, 1, SpineMarker::None));
    }

    assert_eq!(
        drawn, want,
        "the rail drew a different Protocol than the one this file opened"
    );
}

/// **AC1, the run state.** The step row's words are the model's, not a literal
/// this pane types: move the step to a run that succeeded and the row says so.
///
/// The pair matters. The test above pins `sql · not run` against a fixture
/// whose step has never run, and a row that hardcoded that string would pass
/// it. This one changes the fact and reads the row again.
#[test]
fn the_step_rows_run_state_is_the_models_and_not_a_literal() {
    let mut fresh = Live::open(housing_boot());
    fresh.settle();
    assert_eq!(fresh.row("load").kind, "sql \u{b7} not run");

    // The same file, opened as though a run had happened: the step succeeded
    // and the table it produces is still not materialised. Two facts on two
    // levels, moved independently, because a marker reading the wrong one of
    // them is invisible while they agree.
    let mut boot = housing_boot();
    let table = boot
        .protocol
        .table
        .clone()
        .expect("the fixture has a table");
    boot.protocol
        .statuses
        .insert("load".to_string(), SeamStatus::Ok);
    let mut win = Live::open(boot);
    win.settle();

    let step = win.row("load");
    assert_eq!(
        step.kind, "sql \u{b7} ok",
        "the step row is spelling a run state the model does not hold"
    );
    assert_eq!(
        step.marker,
        SpineMarker::Filled,
        "a step that ran to success is a thing that happened, and the marker \
         says so in shape"
    );
    assert_eq!(
        win.row("california_housing_sample").marker,
        SpineMarker::Hollow,
        "the table is not materialised — a step that ran is not an asset that \
         exists, and one marker must not be reading the other's fact"
    );

    // …and now the other level: the asset was measured as materialised while
    // its step's status stays unrecorded.
    let mut boot = housing_boot();
    boot.protocol.assets.insert(
        table,
        AssetMeta {
            row_count: Some(240),
            materialized: true,
            bytes: None,
            content_hash: None,
        },
    );
    let mut win = Live::open(boot);
    win.settle();
    assert_eq!(
        win.row("california_housing_sample").marker,
        SpineMarker::Filled,
        "an asset a run materialised exists, whatever its step's status map says"
    );
    assert_eq!(
        win.row("load").marker,
        SpineMarker::Hollow,
        "…and the step's own marker did not move with it"
    );
}

/// **AC1, the markers as ink.** The two markers are two shapes in two inks, off
/// the frame's own circles rather than off the hook that records them.
///
/// The hook is a record the pane writes; this is what the pane painted. Both,
/// because a pane that recorded `Filled` and left the marker unpainted would
/// pass the list above.
#[test]
fn the_spines_markers_are_a_filled_disc_and_a_hollow_ring_where_the_row_says() {
    let mut win = Live::open(housing_boot());
    win.settle();
    let file = win.row("./california_housing_sample.csv");
    let step = win.row("load");
    let shapes = win.shapes();
    let circles = circles(&shapes);
    let sem = semantic(false);

    // The marker's leading edge sits SPACE_4 in from the row's left, so its
    // centre is one radius past that.
    let at = |row: &SpineRowDrawn| {
        egui::pos2(
            row.rect.left() + spacing::SPACE_4 + 2.5,
            row.rect.center().y,
        )
    };

    let filled = circles
        .iter()
        .find(|c| c.center.distance(at(&file)) < 0.5)
        .unwrap_or_else(|| panic!("no marker on the file row at {:?}", at(&file)));
    assert!(
        (filled.radius - 2.5).abs() < 0.01,
        "the file's marker is radius {}, not 2.5",
        filled.radius
    );
    assert_eq!(
        filled.fill,
        ink(sem.text.secondary),
        "a filled marker is drawn in text.secondary"
    );

    let hollow = circles
        .iter()
        .find(|c| c.center.distance(at(&step)) < 0.5)
        .unwrap_or_else(|| panic!("no marker on the step row at {:?}", at(&step)));
    assert_eq!(
        hollow.fill,
        egui::Color32::TRANSPARENT,
        "a step that has not run draws a ring, not a disc"
    );
    assert_eq!(
        hollow.stroke.color,
        ink(sem.text.muted),
        "a hollow marker is stroked in text.muted"
    );
    assert!(
        (hollow.stroke.width - 1.0).abs() < 0.01,
        "a hollow marker is stroked at 1.0, not {}",
        hollow.stroke.width
    );
}

// ---------------------------------------------------------------------------
// AC2 — two marks, and what a click on a view row moves
// ---------------------------------------------------------------------------

/// **AC2.** A fresh open holds the table's `dashboard`: that row carries the
/// bar, no row carries the wash, and the canvas is the pane group's three
/// panes.
#[test]
fn a_fresh_open_holds_the_dashboard_and_marks_the_row_that_says_so() {
    let mut win = Live::open(housing_boot());
    win.settle();

    let table = win
        .app
        .protocol_model()
        .table()
        .cloned()
        .expect("the fixture opened as a one-step Protocol with a table");
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: table,
            view: NodeView::Dashboard,
        },
        "a data file opens holding its table's dashboard"
    );

    let dashboard = win.row("dashboard");
    let bar = dashboard
        .on_canvas
        .expect("the dashboard row carries the on-canvas bar");
    assert!(
        (bar.left() - dashboard.rect.left()).abs() < 0.01,
        "the bar is at the row's leading edge"
    );

    let washed: Vec<String> = win
        .rows()
        .iter()
        .filter(|row| row.washed)
        .map(|row| row.label.clone())
        .collect();
    assert!(
        washed.is_empty(),
        "a fresh open selects nothing, so no row may carry the selection wash \
         — {washed:?} do"
    );

    let names: Vec<&str> = win
        .app
        .canvas_panes()
        .panes
        .iter()
        .map(|pane| pane.name)
        .collect();
    assert_eq!(
        names,
        vec!["map", "rows", "columns"],
        "the dashboard is the canvas's pane group"
    );
}

/// **AC2.** A click on `grid` puts the table's grid on the canvas: the bar
/// moves to that row, the canvas body becomes one pane headed
/// `Grid · california_housing_sample`, and a click on `dashboard` brings the
/// group back.
#[test]
fn clicking_a_view_row_moves_the_canvas_and_the_bar_with_it() {
    let mut win = Live::open(housing_boot());
    win.settle();
    // The canvas body, read off the group that fills it — what the one grid
    // pane is asserted to occupy below. Reading it off the arrangement instead
    // would compare a declaration with itself.
    let body = win
        .app
        .canvas_panes()
        .panes
        .iter()
        .map(|pane| pane.rect)
        .reduce(|a, b| a.union(b))
        .expect("the pane group drew");

    win.click_row("grid");

    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Grid),
        "clicking the grid row puts the grid on the canvas"
    );
    assert!(
        win.row("dashboard").on_canvas.is_none(),
        "the bar moved off the dashboard row"
    );
    assert!(
        win.row("grid").on_canvas.is_some(),
        "the bar moved onto the grid row"
    );

    let panes = win.app.canvas_panes().panes.clone();
    assert_eq!(
        panes.iter().map(|p| p.name).collect::<Vec<_>>(),
        vec!["grid"],
        "the grid takes the whole canvas body, as one pane"
    );
    let grid = panes.first().expect("one pane");
    assert!(
        (grid.rect.left() - body.left()).abs() < 0.5
            && (grid.rect.right() - body.right()).abs() < 0.5
            && (grid.rect.top() - body.top()).abs() < 0.5
            && (grid.rect.bottom() - body.bottom()).abs() < 0.5,
        "the grid pane is {:?}, which is not the canvas body {body:?}",
        grid.rect
    );

    let header = grid.header;
    let shapes = win.shapes();
    let title = texts(&shapes)
        .into_iter()
        .find(|(_, rect)| header.contains_rect(*rect))
        .map(|(text, _)| text);
    assert_eq!(
        title.as_deref(),
        Some("Grid \u{b7} california_housing_sample"),
        "the pane's header band names the table the grid is of"
    );

    win.click_row("dashboard");
    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Dashboard),
        "clicking the dashboard row brings the dashboard back"
    );
    assert_eq!(
        win.app
            .canvas_panes()
            .panes
            .iter()
            .map(|p| p.name)
            .collect::<Vec<_>>(),
        vec!["map", "rows", "columns"],
        "…and with it the pane group's three panes"
    );
    assert!(
        win.row("dashboard").on_canvas.is_some(),
        "…and the bar with it"
    );
}

/// **AC2, the two marks are two mechanisms.** Picking a column washes that row
/// and moves nothing: the bar stays where the canvas is.
///
/// This is the assertion a shared mechanism fails. A rail that drew one mark
/// for both states passes every test above — the bar would simply follow the
/// selection and there would be no frame in which they disagree.
#[test]
fn selecting_a_column_washes_that_row_and_leaves_the_bar_where_the_canvas_is() {
    let mut win = Live::open(housing_boot());
    win.settle();
    let before = win.row("dashboard").rect;

    win.click_row("house_age");

    let washed: Vec<String> = win
        .rows()
        .iter()
        .filter(|row| row.washed)
        .map(|row| row.label.clone())
        .collect();
    assert_eq!(
        washed,
        vec!["house_age".to_string()],
        "the picked column, and only it, carries the wash"
    );

    let dashboard = win.row("dashboard");
    assert!(
        dashboard.on_canvas.is_some(),
        "the canvas still holds the dashboard, so its row still carries the bar"
    );
    assert_eq!(
        dashboard.rect, before,
        "picking a column moved a row of the spine"
    );
    assert!(
        !dashboard.washed,
        "the row on the canvas is not the row that was picked, and must not \
         wear the picked row's mark"
    );
    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Dashboard),
        "a column pick is not a canvas move"
    );
}

/// **The latch and the derived answer agree about the graph.**
///
/// [`CanvasHolds`] is latched and `graph_on_canvas` is derived, and the reason
/// the second is allowed to stay derived is that the first is reconciled from
/// it each frame. Two windows, one on each side of the question, read off a
/// real frame.
#[test]
fn a_windows_latched_canvas_agrees_with_the_derived_answer() {
    let mut data = Live::open(housing_boot());
    data.settle();
    assert!(!data.app.graph_on_canvas());
    assert_ne!(data.app.canvas_holds(), &CanvasHolds::Graph);

    let spec = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/protocol/edgar_gleif/arcform.yaml");
    let inputs = brightfield_shell::protocol::load_protocol_offline(
        spec.to_str().expect("utf-8 fixture path"),
    )
    .unwrap_or_else(|e| panic!("load {}: {e}", spec.display()));
    let mut manifest = Live::open(Boot::protocol(
        inputs,
        brightfield_protocol::layout::Flow::Vertical,
        None,
    ));
    manifest.settle();
    assert!(manifest.app.graph_on_canvas());
    assert_eq!(
        manifest.app.canvas_holds(),
        &CanvasHolds::Graph,
        "a Protocol with no chart beside it keeps the graph on the canvas"
    );
    assert!(
        manifest.rows().iter().all(|row| row.on_canvas.is_none()),
        "the graph is not a row of the spine, so no row is marked for it — the \
         chip in the spine's head is a later card"
    );
}

// ---------------------------------------------------------------------------
// AC3 — the measurements, at two window sizes
// ---------------------------------------------------------------------------

/// The window the contract names beside the boot's own.
const MEASURED_AT: (f32, f32) = (1440.0, 900.0);

/// **AC3.** The contract's measurements, off the drawn rects, at 1440x900 and
/// at the window the dashboard baseline is photographed in.
///
/// Two sizes because every measure here is a constant offset from an edge, and
/// a rail whose width came out of a share rather than a token would hold at one
/// size and drift at the other.
#[test]
fn the_spines_measurements_hold_at_both_windows() {
    for size in [MEASURED_AT, housing_boot().window_size()] {
        let mut win = Live::at(housing_boot(), size);
        win.settle();
        let rows = win.rows();
        assert!(
            rows.len() > 6,
            "at {size:?} the rail drew {} rows",
            rows.len()
        );

        let body = win.app.spine_body().expect("the pane was laid out");
        let first = rows.first().expect("a caption leads the pane");
        assert!(
            (first.rect.top() - body.top() - spacing::SPACE_1).abs() < 0.01,
            "at {size:?} the first caption sits {} below the body's top, not \
             SPACE_1",
            first.rect.top() - body.top()
        );

        for row in &rows {
            assert!(
                (row.rect.height() - spacing::ROW_DENSE).abs() < 0.01,
                "at {size:?} the row {:?} is {} tall, not ROW_DENSE",
                row.label,
                row.rect.height()
            );
            // The contract's SPACE_4 is the SPINE's measure. A column row is
            // drawn as the outline draws it — the dense binding's own `pad_x`
            // — and is asserted against that rather than skipped, because "as
            // today" is a claim like any other.
            let want = match row.role {
                SpineRole::Column => control::binding(spacing::ROW_DENSE).pad_x,
                _ => spacing::SPACE_4,
            };
            if let Some(kind) = row.kind_rect.filter(|_| !row.kind.is_empty()) {
                assert!(
                    (row.rect.right() - kind.right() - want).abs() < 0.01,
                    "at {size:?} the kind on {:?} sits {} from the row's right \
                     edge, not {want}",
                    row.label,
                    row.rect.right() - kind.right()
                );
            }
        }

        // **Both captions fit the rail.** The rect is the galley's own whether
        // the clip cut it or not, so a caption too wide for the pane fails here
        // rather than being cropped quietly and shipped half-read — which is
        // what the caption naming the table did on its first render.
        for caption in rows.iter().filter(|row| row.role == SpineRole::Caption) {
            assert!(
                body.contains_rect(caption.name_rect),
                "at {size:?} the caption {:?} drew at {:?}, which is not inside \
                 the pane {body:?} — it is being cut by the clip rect",
                caption.label,
                caption.name_rect
            );
        }

        let table = win.row("california_housing_sample");
        for view in NodeView::ALL {
            let row = win.row(view.label());
            assert!(
                (row.name_rect.left() - table.name_rect.left() - spacing::SPACE_5).abs() < 0.01,
                "at {size:?} the {} row's name starts {} past the node's, not \
                 SPACE_5",
                view.label(),
                row.name_rect.left() - table.name_rect.left()
            );
        }

        let bar = win
            .row("dashboard")
            .on_canvas
            .expect("the dashboard is on the canvas");
        let row = win.row("dashboard").rect;
        assert!(
            (bar.width() - 2.0).abs() < 0.01,
            "at {size:?} the on-canvas bar is {} points wide, not two",
            bar.width()
        );
        assert!(
            (bar.left() - row.left()).abs() < 0.01,
            "at {size:?} the bar is {} from the row's left edge",
            bar.left() - row.left()
        );
        assert!(
            (bar.height() - row.height()).abs() < 0.01,
            "at {size:?} the bar is {} tall against a row of {}",
            bar.height(),
            row.height()
        );
    }
}

/// **The rail's selector strip names the pane `Protocol`.**
///
/// Off the strip the frame drew rather than off the registry, because the strip
/// draws the pane's own `Subject` and the two could disagree.
#[test]
fn the_navigator_rails_strip_names_the_pane_protocol() {
    let mut win = Live::open(housing_boot());
    win.settle();
    let name = win
        .app
        .rail_name_rect(brightfield_workbench::arrangement::NAVIGATOR_RAIL, 0)
        .expect("the navigator rail drew a name in its strip");
    let shapes = win.shapes();
    let drawn = texts(&shapes)
        .into_iter()
        .find(|(_, rect)| name.expand(2.0).contains_rect(*rect))
        .map(|(text, _)| text);
    assert_eq!(
        drawn.as_deref(),
        Some("Protocol"),
        "the navigator rail's strip names the pane it draws"
    );
}

// ---------------------------------------------------------------------------
// The other Protocols: a manifest of many steps gets the same spine
// ---------------------------------------------------------------------------

/// A window over the shipped crosswalk manifest — twelve steps, no profiled
/// table, and the graph on the canvas.
fn crosswalk() -> Live {
    let spec = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/protocol/edgar_gleif/arcform.yaml");
    let inputs = brightfield_shell::protocol::load_protocol_offline(
        spec.to_str().expect("utf-8 fixture path"),
    )
    .unwrap_or_else(|e| panic!("load {}: {e}", spec.display()));
    Live::open(Boot::protocol(
        inputs,
        brightfield_protocol::layout::Flow::Vertical,
        None,
    ))
}

/// **A Protocol of many steps gets the same spine.** Every step row stands
/// above the asset it produces; the hosts the run reads from stand alone and
/// filled; both captions are drawn and the outline names no table.
///
/// The step-above-its-asset rule is the one worth having a test for. It reads
/// as an ordering claim and it is a **lineage** claim: `AssetNode::step` means
/// *produced by* except on a `Source` node, where it names the step that
/// fetches from that host. Taking it at face value drew `fetch_edgar` above
/// `openlake.meridian.online` — a row saying the fetch made the website.
#[test]
fn a_manifest_of_many_steps_puts_each_step_above_the_asset_it_produced() {
    let mut win = crosswalk();
    win.settle();
    let rows = win.rows();

    let first = rows.first().expect("a caption leads the pane");
    assert_eq!(
        first.label, "SPINE   \u{b7}   12 steps",
        "the spine's caption counts the steps it lists"
    );
    let steps: std::collections::BTreeSet<&str> = rows
        .iter()
        .filter(|row| row.role == SpineRole::Step)
        .map(|row| row.label.as_str())
        .collect();
    assert_eq!(
        steps.len(),
        12,
        "the caption says twelve and the rail drew {} distinct steps: {steps:?}",
        steps.len()
    );

    for pair in rows.windows(2) {
        if pair[0].role == SpineRole::Step {
            assert_eq!(
                pair[1].role,
                SpineRole::Asset,
                "the step {:?} stands above {:?}, which is not an asset — a \
                 step row is a claim that this asset came through this step",
                pair[0].label,
                pair[1].label
            );
        }
    }

    let hosts: Vec<&SpineRowDrawn> = rows.iter().filter(|row| row.kind == "source").collect();
    assert!(
        !hosts.is_empty(),
        "the crosswalk reads from hosts; the rail listed none"
    );
    for host in &hosts {
        assert_eq!(
            host.marker,
            SpineMarker::Filled,
            "{:?} is a host the run reads from — an external input the \
             Protocol has before it starts",
            host.label
        );
    }
    let above: Vec<&str> = rows
        .windows(2)
        .filter(|pair| pair[1].kind == "source" && pair[0].role == SpineRole::Step)
        .map(|pair| pair[0].label.as_str())
        .collect();
    assert!(
        above.is_empty(),
        "{above:?} were drawn above a host, which says a fetch produced the \
         website it reads from"
    );

    assert!(
        rows.iter().all(|row| row.role != SpineRole::View),
        "a manifest declares relations and profiles no table, so it has no \
         views to list"
    );
    let outline = rows
        .iter()
        .filter(|row| row.role == SpineRole::Caption)
        .nth(1)
        .expect("the outline's caption is drawn too");
    assert_eq!(
        outline.label, "OUTLINE   \u{b7}   0 columns",
        "a manifest profiles no table and has no columns, and says so"
    );
    assert!(
        rows.iter().all(|row| row.role != SpineRole::Column),
        "…and lists none"
    );

    assert!(
        rows.iter().all(|row| row.on_canvas.is_none()),
        "the graph is on this canvas and the graph is not a row of the spine"
    );
    let washed = rows.iter().filter(|row| row.washed).count();
    assert_eq!(
        washed, 1,
        "a manifest-opened Protocol keeps its boot selection — one row washed, \
         not {washed}"
    );
}
