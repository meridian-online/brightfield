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
//! window sizes. Not covered here: the pixels. The re-photographed baselines in
//! `tests/surfaces.rs` and `tests/dashboard_baseline.rs` are that half, and
//! they are a different kind of evidence — an image reddens on a font bump as
//! loudly as on a dropped row, which is why the structural half is here and
//! reads in sentences.

use brightfield_protocol::contract_graph::{AssetMeta, SeamStatus};
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::{
    GraphChipDrawn, NodeView, SpineMarker, SpineRole, SpineRowDrawn,
};
use brightfield_shell::window::{Boot, CanvasHolds, MeridianApp};
use meridian_design::{control, semantic, spacing};

/// The committed table this file's windows are opened over — its nine columns
/// are [`HOUSING_COLUMNS`], and two of them are a coordinate pair.
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

    /// The graph chip the head row drew — panics when the head drew no chip,
    /// naming what the first row was instead, so a chip dropped off the head
    /// fails here with a sentence rather than with `unwrap` on a `None`.
    fn chip(&self) -> GraphChipDrawn {
        let rows = self.rows();
        let head = rows.first().unwrap_or_else(|| {
            panic!("the rail drew no rows at all, so it drew no head to carry a chip")
        });
        head.chip.unwrap_or_else(|| {
            panic!(
                "the spine's head row {:?} carries no graph chip",
                head.label
            )
        })
    }

    /// Click where the last frame drew the graph chip.
    fn click_chip(&mut self) {
        let at = self.chip().rect.center();
        self.run(vec![click_at(at), Vec::new(), Vec::new()]);
    }

    /// Click where the last frame drew the canvas chip for `view` on the node
    /// the canvas is showing views of — panics naming what was drawn instead.
    fn click_canvas_chip(&mut self, view: NodeView) {
        let chips = self.app.canvas_chips().to_vec();
        let chip = chips
            .iter()
            .find(|chip| chip.view == view)
            .unwrap_or_else(|| {
                let drawn: Vec<NodeView> = chips.iter().map(|chip| chip.view).collect();
                panic!(
                    "the canvas drew no chip for {:?}; it drew {drawn:?}",
                    view.label()
                )
            });
        let at = chip.rect.center();
        self.run(vec![click_at(at), Vec::new(), Vec::new()]);
    }

    /// Drag region `id`'s resize edge until it is `want` points across —
    /// `tests/arrangement.rs`'s `Live::drag_edge_to`, over this file's own
    /// window. Five frames because the press, the move and the release are
    /// each a frame, and egui reads the handle's response from the frame
    /// before.
    fn drag_edge_to(&mut self, id: brightfield_workbench::arrangement::RegionId, want: f32) {
        let rect = self
            .app
            .region_rect(id)
            .unwrap_or_else(|| panic!("{id} did not draw"));
        let grab = egui::pos2(rect.right(), rect.center().y);
        let to = egui::pos2(rect.left() + want, rect.center().y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.run(vec![
            vec![egui::Event::PointerMoved(grab)],
            vec![egui::Event::PointerMoved(grab), button(grab, true)],
            vec![egui::Event::PointerMoved(to)],
            vec![egui::Event::PointerMoved(to)],
            vec![egui::Event::PointerMoved(to), button(to, false)],
            Vec::new(),
            Vec::new(),
        ]);
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

/// The text galleys the frame painted, with the rect each landed in and the
/// font its first section was set in — the sole section for a galley built
/// here from `layout_no_wrap` or `painter.text` — which is the face a caller
/// reads to catch a label drawn in the wrong one, a fact a rect alone does
/// not show.
///
/// Recursive, because a pane's chrome nests its shapes: a `Shape::Vec` holding
/// a fill and a stroke is one row's wash, and the galley under it is a level
/// down.
fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Rect, egui::FontId)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect, egui::FontId)>) {
        match shape {
            egui::Shape::Text(text) => {
                let font = text
                    .galley
                    .job
                    .sections
                    .first()
                    .map(|section| section.format.font_id.clone())
                    .unwrap_or_else(egui::FontId::default);
                out.push((
                    text.galley.text().to_string(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                    font,
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

/// [`texts`] with the clip rect each galley was painted under — the one field
/// that [`texts`] does not return.
///
/// A clip narrows what reaches the screen without moving the galley's own
/// rect — `spine_head_row`'s own doc says so, "the rect handed back is the
/// galley's own, clip or no clip" — so a claim about what a *reader* sees has
/// to read this. `texts` alone would miss a caption that falls outside its
/// clip. Not recursive into `Shape::Vec`: a sub-painter's
/// `with_clip_rect` call adds its shape as its own entry in the frame's list
/// rather than nesting inside one, so the clip rect on each top-level
/// `ClippedShape` already belongs to whatever galley is under it.
fn clipped_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Rect, egui::Rect)> {
    fn walk(
        shape: &egui::Shape,
        clip: egui::Rect,
        out: &mut Vec<(String, egui::Rect, egui::Rect)>,
    ) {
        match shape {
            egui::Shape::Text(text) => {
                out.push((
                    text.galley.text().to_string(),
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                    clip,
                ));
            }
            egui::Shape::Vec(shapes) => {
                for s in shapes {
                    walk(s, clip, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in shapes {
        walk(&clipped.shape, clipped.clip_rect, &mut out);
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

/// **The Operator tab, switched to on a fresh open, describes the table the
/// canvas holds — it does not say "Nothing selected."**
///
/// The inspector rail's strip offers two tabs over one document each —
/// `Operator` (the Protocol's) and `Inspector` (the chart's), sharing
/// `INSPECTOR_PANES` behind one selector — and a fresh data-file open leaves
/// the chart's own tab active, but a reader can switch. `has_selection` and
/// `inspector` both fall back to the node the canvas holds when no asset is
/// explicitly picked, held by
/// `switching_to_operator_on_a_fresh_open_describes_the_canvas_held_table`
/// (this test) — so switching to Operator there answers for the table rather
/// than for an empty state. Before that fallback existed, this tab read
/// `Nothing selected` regardless of which row the bar was on.
///
/// The Address field's own explainer is a separate claim from the fallback
/// above, and this test holds it too. A data-file window does not feed the
/// `y` keystroke to the model — `MeridianApp::draw` gates that on the graph
/// being on the canvas, which it is not here — so the drawn copy names no
/// keystroke. A round that widened this pane's reach on a fresh open once
/// left the old "press y to copy it" clause on a window where `y` does
/// nothing.
#[test]
fn switching_to_operator_on_a_fresh_open_describes_the_canvas_held_table() {
    let mut win = Live::open(housing_boot());
    win.settle();

    let shapes = win.shapes();
    let operator = texts(&shapes)
        .into_iter()
        .find(|(text, _, _)| text == "Operator")
        .map(|(_, rect, _)| rect)
        .expect("the inspector rail's strip offers an Operator tab");
    win.run(vec![click_at(operator.center()), Vec::new(), Vec::new()]);

    let shapes = win.shapes();
    let drawn: Vec<String> = texts(&shapes).into_iter().map(|(t, _, _)| t).collect();
    assert!(
        !drawn.iter().any(|t| t == "Nothing selected"),
        "the Operator pane still shows its empty state after a fresh open, \
         though the canvas holds the table's dashboard: {drawn:?}"
    );
    assert!(
        drawn.iter().any(|t| t == "ADDRESS"),
        "the Operator pane drew no Address field for the table the canvas \
         holds: {drawn:?}"
    );
    assert!(
        !drawn.iter().any(|t| t.contains("press y")),
        "the Address field names a keystroke on a window whose grammar is \
         not fed — a data-file window never feeds `y` to the model, so \
         pressing it here does nothing the copy just promised: {drawn:?}"
    );
    let table = win
        .app
        .protocol_model()
        .table()
        .cloned()
        .expect("the fixture opened as a one-step Protocol with a table");
    assert!(
        drawn.iter().any(|t| t == &table),
        "the Operator pane's Address field does not name the table the \
         canvas holds ({table}): {drawn:?}"
    );
}

/// **A selected node on a manifest window draws the `y` hint, because that
/// window is the one where the grammar reaches the model.**
///
/// The companion to `switching_to_operator_on_a_fresh_open_describes_the_canvas_held_table`'s
/// negative: gating the Address field's explainer on
/// `CanvasHolds::Graph` has to still show the clause where it is true, or
/// the fix would have swapped one wrong answer for the opposite wrong
/// answer. `MeridianApp::draw` feeds `y` to the model while the graph is what
/// the canvas holds and no overlay owns the keyboard, so the clause is true on
/// exactly the windows whose canvas holds the graph. A manifest opened with no
/// chart beside it latches the graph;
/// `a_windows_latched_canvas_agrees_with_the_derived_answer` pins that on one
/// settled frame of the edgar_gleif fixture, beside a data-file window that
/// latches a view. Selecting an asset row and switching to Operator here is the
/// frame the keystroke actually reaches on that same fixture.
#[test]
fn a_selected_node_on_a_manifest_window_draws_the_yank_hint() {
    let spec = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/protocol/edgar_gleif/arcform.yaml");
    let inputs = brightfield_shell::protocol::load_protocol_offline(
        spec.to_str().expect("utf-8 fixture path"),
    )
    .unwrap_or_else(|e| panic!("load {}: {e}", spec.display()));
    let mut win = Live::open(Boot::protocol(
        inputs,
        brightfield_protocol::layout::Flow::Vertical,
        None,
    ));
    win.settle();
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::Graph,
        "this test relies on the graph being on the canvas, which is what \
         feeds `y` to the model"
    );

    let asset_row = win
        .rows()
        .into_iter()
        .find(|row| row.role == SpineRole::Asset)
        .expect("a manifest with steps draws at least one asset row");
    win.click_row(&asset_row.label);

    let shapes = win.shapes();
    let operator = texts(&shapes)
        .into_iter()
        .find(|(text, _, _)| text == "Operator")
        .map(|(_, rect, _)| rect)
        .expect("the inspector rail's strip offers an Operator tab");
    win.run(vec![click_at(operator.center()), Vec::new(), Vec::new()]);

    let shapes = win.shapes();
    let drawn: Vec<String> = texts(&shapes).into_iter().map(|(t, _, _)| t).collect();
    assert!(
        drawn.iter().any(|t| t.contains("press y to copy it")),
        "a selected node on a window whose grammar IS fed dropped the \
         keystroke hint it should carry: {drawn:?}"
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
    let body = grid.body;
    let shapes = win.shapes();
    let title = texts(&shapes)
        .into_iter()
        .find(|(_, rect, _)| header.contains_rect(*rect))
        .map(|(text, _, _)| text);
    assert_eq!(
        title.as_deref(),
        Some("Grid \u{b7} california_housing_sample"),
        "the pane's header band names the table the grid is of"
    );

    // **The grid actually drew the table, not just a correctly headed empty
    // pane.** The header band names the table whether or not
    // `draw_chart_body` is ever called under it — the two are painted by two
    // different calls, and the assertions above this line pass either way.
    // `california_housing_sample.csv`'s first row has a `population` of
    // 3244, a value distinctive enough that its presence means the engine's
    // own session, not a fixture string, put it on screen.
    let in_body: Vec<String> = texts(&shapes)
        .into_iter()
        .filter(|(_, rect, _)| body.contains_rect(*rect))
        .map(|(text, _, _)| text)
        .collect();
    assert!(
        in_body.len() > 20,
        "the grid pane's body drew {} galleys — a table of 240 rows by 9 \
         columns draws far more cells than that: {in_body:?}",
        in_body.len()
    );
    assert!(
        in_body.iter().any(|t| t == "3244"),
        "the grid pane's body drew no cell reading 3244 — the table's first \
         row's population — so the pane drew a header with no table under \
         it: {in_body:?}"
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

/// **Opening a second data file while the canvas holds the first table's
/// grid resets the latch to the new table's dashboard — by identity, not by
/// "a View is already latched."**
///
/// `MeridianApp::reconcile_canvas_holds` keeps a latched `View` when, and
/// just when, its node still names the CURRENT table (`*node == table`) —
/// held by
/// `opening_a_second_file_over_a_grid_resets_the_latch_to_the_new_tables_dashboard`
/// (this test). Drop that comparison for "any View counts as held" and it
/// stays green with the bar sitting on the first table's `grid` row after a
/// second, unrelated file has opened — the state a reader would see as the
/// canvas showing a table that is no longer the one behind the rail.
#[test]
fn opening_a_second_file_over_a_grid_resets_the_latch_to_the_new_tables_dashboard() {
    let mut win = Live::open(housing_boot());
    win.settle();
    win.click_row("grid");
    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Grid),
        "the fixture: the first table's grid is on the canvas before the \
         second file opens"
    );

    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/point_map_baseline.csv");
    let ctx = win.ctx.clone();
    win.app
        .open_data_file(&ctx, path.to_str().expect("utf-8 fixture path"));
    win.settle();

    let table = win
        .app
        .protocol_model()
        .table()
        .cloned()
        .expect("the second file opened as a one-step Protocol with a table");
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: table,
            view: NodeView::Dashboard,
        },
        "the latch still names the first table's grid after a second, \
         unrelated file opened"
    );
    let bar_row: Vec<String> = win
        .rows()
        .into_iter()
        .filter(|row| row.on_canvas.is_some())
        .map(|row| row.label)
        .collect();
    assert_eq!(
        bar_row,
        vec!["dashboard".to_string()],
        "the on-canvas bar does not sit on the new table's dashboard row: \
         {bar_row:?}"
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
/// `graph_on_canvas` reads [`CanvasHolds`], the latch, directly; it performs
/// no derivation of its own. The derived answer is `graph_takes_the_canvas`,
/// and it is the latch that gets reconciled from that function each frame,
/// not the other way around. Two windows, one on each side of the question,
/// read off a real frame.
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
    // The graph is not a row of the LIST, so no row of the list is marked for
    // it — the head row is, which is where the chip that names the graph sits.
    assert!(
        manifest
            .rows()
            .iter()
            .skip(1)
            .all(|row| row.on_canvas.is_none()),
        "a row of the spine's list is marked for a graph that is not in it"
    );
    let head = manifest.rows().first().cloned().expect("a head row");
    assert!(
        head.on_canvas.is_some(),
        "the spine's head carries no on-canvas bar on a window whose canvas \
         holds nothing but the graph"
    );
    let chip = manifest.chip();
    assert!(
        chip.filled,
        "…and its chip is not the state the canvas is in"
    );
    assert!(
        !chip.live,
        "the chip is a control on a Protocol whose canvas can only ever hold \
         the graph — a click there would be undone by the next frame's \
         reconciliation, so it must not offer one"
    );
}

/// **A rail with no Protocol behind it reports no rows.**
///
/// The pane draws its empty state instead, which means `OutlinePane::ui` does
/// not run — so the record has to be cleared by the frame rather than by the
/// pane, or a row list from a previous document answers for a rail that is
/// drawing "No assets yet". That is the same failure the canvas's pane record
/// is cleared per frame for, one rail over.
#[test]
fn a_window_with_no_protocol_reports_no_spine_rows() {
    let composed = brightfield_shell::pipeline::compose_spec("../../examples/dashboard.yaml")
        .expect("compose the shipped dashboard");
    let mut win = Live::open(Boot::charts(composed));
    win.settle();
    assert!(
        win.app.spine_rows().is_empty(),
        "a chart-only window drew {:?} in a rail whose pane is its empty state",
        win.app
            .spine_rows()
            .iter()
            .map(|row| row.label.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        win.app.spine_body().is_none(),
        "…and reported a content box for a pane that laid none out"
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
        // One more settled frame, purely to read the fonts the galleys were
        // built with — `SpineRowDrawn` carries the rect a kind label landed
        // in, not the face it was set in, so that has to come off the shapes
        // themselves.
        let shapes = win.shapes();
        let painted = texts(&shapes);
        // The contract's mono caption face — one step under the UI size, the
        // same construction `caption_font` makes in `protocol.rs`, restated
        // here rather than called across the crate boundary a `tests/`
        // binary sits on the far side of.
        let mono_caption = egui::FontId::monospace(meridian_design::typography::UI_SIZE - 1.0);

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
                // The face, not just the place. A column row keeps the
                // outline's own `ui_font()` — that pre-dates this contract and
                // is not what it governs — but a spine row's kind (the
                // step's or the asset's, at the trailing end) is the mono
                // caption face in `text.muted`, held below for each row this
                // loop reaches, so a run state reads as a value rather than
                // as prose beside it.
                if row.role != SpineRole::Column {
                    let drawn = painted
                        .iter()
                        .find(|(text, rect, _)| {
                            *text == row.kind && kind.expand(0.5).contains_rect(*rect)
                        })
                        .map(|(_, _, font)| font.clone());
                    assert_eq!(
                        drawn.as_ref(),
                        Some(&mono_caption),
                        "at {size:?} the kind on {:?} is drawn in {drawn:?}, not \
                         the mono caption face {mono_caption:?}",
                        row.label
                    );
                }
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
        .find(|(_, rect, _)| name.expand(2.0).contains_rect(*rect))
        .map(|(text, _, _)| text);
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

    // The graph holds this canvas and the graph is not a row of the LIST, so
    // the bar is on the head — the row that names the whole Protocol, which is
    // what the graph is — and on nothing under it.
    assert!(
        rows.iter().skip(1).all(|row| row.on_canvas.is_none()),
        "a row of the spine's list is marked for a graph that is not in it"
    );
    assert!(
        rows.first().is_some_and(|head| head.on_canvas.is_some()),
        "the spine's head carries no bar on a canvas holding the graph"
    );
    let washed = rows.iter().filter(|row| row.washed).count();
    assert_eq!(
        washed, 1,
        "a manifest-opened Protocol keeps its boot selection — one row washed, \
         not {washed}"
    );
}

// ---------------------------------------------------------------------------
// The graph chip: the way from the picture to the map, and back
// ---------------------------------------------------------------------------

/// **AC1.** The spine's head carries a `graph` chip at its trailing end, and on
/// a fresh open of a data file it is unfilled — because what the canvas holds
/// is the table's dashboard, not the graph.
///
/// Every measure is read off the drawn frame: the chip's box off the head row's
/// own record, the word and the caption off the galleys the frame painted. The
/// caption's clearance is the one that would fail silently otherwise — a
/// caption laid out over the chip is clipped by the row rather than refused, so
/// what is asserted is where the galley *is*, which the record carries whether
/// the clip cut it or not.
#[test]
fn the_spines_head_carries_an_unfilled_graph_chip_over_a_dashboard() {
    let mut win = Live::open(housing_boot());
    win.settle();

    let head = win
        .rows()
        .first()
        .cloned()
        .expect("a caption leads the pane");
    assert_eq!(
        head.role,
        SpineRole::Caption,
        "the chip belongs to the head row, and the head row is the caption"
    );
    let chip = win.chip();
    assert!(
        !chip.filled,
        "a fresh open holds the table's dashboard, so the graph chip is not \
         the state the canvas is in"
    );
    assert!(
        chip.live,
        "the fixture's Protocol has a table with views, so the chip has \
         somewhere to take the canvas and back"
    );
    assert!(
        (chip.rect.height() - control::HEIGHT_XS).abs() < 0.01,
        "the chip is {} tall, not the HEIGHT_XS rung ({})",
        chip.rect.height(),
        control::HEIGHT_XS
    );
    assert!(
        (head.rect.right() - chip.rect.right() - spacing::SPACE_4).abs() < 0.01,
        "the chip sits {} from the head row's trailing edge, not SPACE_4",
        head.rect.right() - chip.rect.right()
    );
    assert!(
        (chip.rect.center().y - head.rect.center().y).abs() < 0.01,
        "the chip's centre is {} off the row's",
        chip.rect.center().y - head.rect.center().y
    );

    // The word, off the frame — a box with no `graph` in it is a box.
    let shapes = win.shapes();
    let painted = texts(&shapes);
    let word = painted
        .iter()
        .find(|(text, rect, _)| text == "graph" && chip.rect.expand(0.5).contains_rect(*rect));
    assert!(
        word.is_some(),
        "no galley reading \"graph\" landed inside the chip's box {:?}; the \
         frame painted {:?}",
        chip.rect,
        painted
            .iter()
            .map(|(text, _, _)| text.as_str())
            .collect::<Vec<_>>()
    );

    // **The caption ends left of the chip.** `SPACE_4` of it, which is the
    // clearance the head's clip rect is built from.
    assert!(
        head.name_rect.right() <= chip.rect.left() - spacing::SPACE_4 + 0.01,
        "the head caption's galley ends at {} and the chip starts at {}, so \
         the caption is running under the chip rather than stopping SPACE_4 \
         short of it",
        head.name_rect.right(),
        chip.rect.left()
    );
}

/// **AC1, at the rail's floor rather than its default.** The test above skips
/// `spine_head_row`'s `with_clip_rect(room)` call: at the default
/// 240-point rail the caption's own galley already ends left of the chip. Its
/// "ends left of the chip" assertion reads `name_rect` — which
/// `spine_head_row`'s own doc says is "the galley's own, clip or no clip" —
/// and would pass identically if the clip were deleted.
///
/// Drag the rail to `NAVIGATOR_RAIL`'s declared floor, where the same
/// caption's unclipped galley runs well past the chip, and read what the
/// frame actually painted through [`clipped_texts`] rather than the galley's
/// own rect: the clip rect egui recorded against the caption's `Shape::Text`,
/// intersected with the galley to get the extent that actually reached the
/// screen.
#[test]
fn the_head_captions_clip_keeps_it_off_the_chip_at_the_rails_floor() {
    use brightfield_workbench::arrangement::{self, NAVIGATOR_RAIL};

    let arrangement::Extent::Rail { min, .. } = arrangement::default_arrangement()
        .expect_region(NAVIGATOR_RAIL)
        .extent
    else {
        panic!("the navigator rail is declared a rail");
    };

    let mut win = Live::open(housing_boot());
    win.settle();
    // Well past the floor, so what stops the drag is the floor rather than
    // where the pointer was let go — `a_rail_dragged_past_its_floor_stops_
    // at_the_floor_it_declares` in `tests/arrangement.rs` is the same move.
    win.drag_edge_to(NAVIGATOR_RAIL, min / 2.0);

    let rail = win
        .app
        .region_rect(NAVIGATOR_RAIL)
        .expect("the navigator rail drew");
    assert!(
        (rail.width() - min).abs() < 1e-3,
        "the drag did not reach the rail's declared floor: it drew at {}pt \
         against a {min}pt floor",
        rail.width()
    );

    let head = win
        .rows()
        .first()
        .cloned()
        .expect("a caption leads the pane");
    let chip = win.chip();

    // Prove this test is exercising the clip at all: at the floor, the
    // caption's own unclipped galley has to overrun the chip, or nothing
    // below distinguishes the clip existing from the clip being deleted.
    assert!(
        head.name_rect.right() > chip.rect.left(),
        "at the {min}pt floor the caption's unclipped galley ends at {}, \
         still left of the chip at {} — narrow further, or this test proves \
         nothing about the clip",
        head.name_rect.right(),
        chip.rect.left()
    );

    let shapes = win.shapes();
    let painted = clipped_texts(&shapes);
    let (_, caption_rect, caption_clip) = painted
        .iter()
        .find(|(text, _, _)| text == &head.label)
        .unwrap_or_else(|| {
            panic!(
                "no galley reading {:?} landed in the frame; it painted {:?}",
                head.label,
                painted
                    .iter()
                    .map(|(text, _, _)| text.as_str())
                    .collect::<Vec<_>>()
            )
        });
    let visible = caption_rect.intersect(*caption_clip);
    assert!(
        visible.right() <= chip.rect.left() - spacing::SPACE_4 + 0.01,
        "the caption painted at {caption_rect:?} clipped to {caption_clip:?} \
         still reaches {}, against a chip starting at {} — the clip is not \
         keeping the caption off the chip",
        visible.right(),
        chip.rect.left()
    );
}

/// **AC2.** Clicking the chip puts the asset graph on the canvas, and clicking
/// it again brings back exactly the view it left.
///
/// Six facts, each read off a drawn frame rather than off the model: what the
/// latch holds, whether the pane group drew, where the on-canvas bar is, the
/// chip's own fill, whether the key-hint band is there, and whether the title
/// band offers the flow toggle. The band and the toggle are in here rather than
/// in a test of their own because they are the reason `graph_on_canvas` had to
/// stop being derived — a window drawing the graph with no hint band under it
/// is a window whose chrome is describing the other document.
#[test]
fn clicking_the_graph_chip_puts_the_graph_on_the_canvas_and_a_second_click_brings_the_view_back() {
    let mut win = Live::open(housing_boot());
    win.settle();
    assert_eq!(
        win.app
            .canvas_panes()
            .panes
            .iter()
            .map(|p| p.name)
            .collect::<Vec<_>>(),
        vec!["map", "rows", "columns"],
        "the fresh open draws the dashboard as the pane group"
    );
    assert!(
        win.row("dashboard").on_canvas.is_some(),
        "…with the bar on the dashboard row"
    );

    win.click_chip();

    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::Graph,
        "the chip put the graph on the canvas"
    );
    assert!(
        win.app.graph_on_canvas(),
        "…and the window's answer to \"is the graph on the canvas\" follows it, \
         which is what every band below reads"
    );
    assert!(
        win.app.canvas_panes().panes.is_empty(),
        "the pane group is still drawn over the graph: {:?}",
        win.app
            .canvas_panes()
            .panes
            .iter()
            .map(|p| p.name)
            .collect::<Vec<_>>()
    );
    let canvas = win
        .app
        .region_rect(brightfield_workbench::arrangement::CANVAS)
        .expect("the canvas region drew");
    let pane = win
        .app
        .canvas_viewport()
        .expect("the DAG canvas pane drew, so it recorded the box it was given");
    assert!(
        canvas.expand(0.5).contains_rect(pane),
        "the DAG pane drew at {pane:?}, which is not inside the canvas region \
         {canvas:?}"
    );

    assert!(win.chip().filled, "the chip is the state the canvas is in");
    assert!(
        win.rows().first().and_then(|row| row.on_canvas).is_some(),
        "the spine's head carries the on-canvas bar while the graph is on it"
    );
    for view in NodeView::ALL {
        assert!(
            win.row(view.label()).on_canvas.is_none(),
            "the {} row still carries the bar with the graph on the canvas",
            view.label()
        );
    }

    let shapes = win.shapes();
    let painted = texts(&shapes);
    assert!(
        painted
            .iter()
            .any(|(text, _, _)| text.contains("producer\u{b7}consumer")),
        "the key-hint band is not drawn over a graph that reached the canvas \
         through the chip, so a reader is being given the DAG grammar with \
         nothing telling them the keys: {:?}",
        painted
            .iter()
            .map(|(text, _, _)| text.as_str())
            .collect::<Vec<_>>()
    );
    let title = win
        .app
        .region_rect(brightfield_workbench::arrangement::TITLE_BAND)
        .expect("the title band drew");
    assert!(
        painted
            .iter()
            .any(|(text, rect, _)| text.starts_with("flow: ") && title.contains_rect(*rect)),
        "the title band offers no flow toggle over a graph the chip put on the \
         canvas, though it offers one over every other graph"
    );

    win.click_chip();

    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Dashboard),
        "the second click gives the canvas back to the view the chip took it \
         from"
    );
    assert!(!win.chip().filled, "…and the chip is unfilled again");
    assert_eq!(
        win.app
            .canvas_panes()
            .panes
            .iter()
            .map(|p| p.name)
            .collect::<Vec<_>>(),
        vec!["map", "rows", "columns"],
        "…and the pane group is back"
    );
    assert!(
        win.row("dashboard").on_canvas.is_some(),
        "…and the bar with it"
    );
    assert!(
        win.rows().first().and_then(|row| row.on_canvas).is_none(),
        "…and off the head"
    );
}

/// **AC2, the round trip is to where you were and not to a default.** The chip
/// clicked off the `grid` comes back to the `grid`.
///
/// The pair with the test above matters: that one leaves from the dashboard,
/// which is also what a fresh open holds, so a chip that always came back to
/// the table's dashboard would pass it. This one leaves from somewhere else.
#[test]
fn the_graph_chip_comes_back_to_the_view_it_left_and_not_to_the_dashboard() {
    let mut win = Live::open(housing_boot());
    win.settle();
    win.click_row("grid");
    assert_eq!(win.app.canvas_holds().view(), Some(NodeView::Grid));

    win.click_chip();
    assert_eq!(win.app.canvas_holds(), &CanvasHolds::Graph);

    win.click_chip();
    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Grid),
        "the chip came back to the table's dashboard from a canvas it took off \
         the grid"
    );
    assert!(
        win.row("grid").on_canvas.is_some(),
        "…and the bar is back on the grid row"
    );
}

/// **AC3, the click half.** A click on the `grid` chip in the table node's foot
/// puts the grid on the canvas, with the bar on the `grid` row.
///
/// This is the assertion the chips exist for, and it is driven through the real
/// pointer path: the click goes at the rect the canvas pane recorded, through
/// the pane's own `Sense::click`, into `hit_test`. A chip that is drawn and not
/// reachable fails here, and so does a `hit_test` that resolves the node under
/// the chip first — which it would, a chip's page being page its node covers
/// too.
#[test]
fn clicking_a_view_chip_on_the_graph_puts_that_view_on_the_canvas() {
    let mut win = Live::open(housing_boot());
    win.settle();
    win.click_chip();
    assert_eq!(win.app.canvas_holds(), &CanvasHolds::Graph);

    let chips = win.app.canvas_chips().to_vec();
    let table = win
        .app
        .canvas_chips()
        .first()
        .map(|chip| chip.node.clone())
        .expect("the graph drew the table node's chips");
    assert_eq!(
        chips
            .iter()
            .map(|chip| (chip.node.clone(), chip.view))
            .collect::<Vec<_>>(),
        vec![
            (table.clone(), NodeView::Dashboard),
            (table.clone(), NodeView::Grid),
        ],
        "the canvas drew a different set of chips than the table's two views"
    );

    win.click_canvas_chip(NodeView::Grid);

    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: table,
            view: NodeView::Grid,
        },
        "the chip in the node's foot did not put the grid on the canvas"
    );
    assert!(
        win.row("grid").on_canvas.is_some(),
        "…and the bar did not follow it to the grid row"
    );
    assert!(
        !win.chip().filled,
        "…and the graph chip is still the state the canvas is in"
    );
}

/// **Every view reads back from the word its chip carries.**
///
/// `brightfield_protocol::layout` lays the chips out from words — that crate
/// has no view type and should not grow one — so a click resolved against a
/// chip rectangle comes back holding a string, and `NodeView::from_label` is
/// what turns it back into a view. A view whose word did not round-trip would
/// draw a chip nothing could resolve: the chip would be there and the click
/// would fall through to the node under it.
#[test]
fn every_view_reads_back_from_the_word_the_chip_carries() {
    for view in NodeView::ALL {
        assert_eq!(
            NodeView::from_label(view.label()),
            Some(view),
            "{:?} does not read back from its own word",
            view
        );
    }
    assert_eq!(
        NodeView::chip_labels(),
        NodeView::ALL
            .iter()
            .map(|view| view.label().to_string())
            .collect::<Vec<_>>(),
        "the chips a node draws are a different list than the views it has"
    );
    assert_eq!(
        NodeView::from_label("dashboards"),
        None,
        "a word that is not a view reads back as one"
    );
}

/// **The layout the boot computes is the layout the canvas draws.**
///
/// Chips make the node that carries them taller and wider, so a layout computed
/// without them places the cards somewhere else. This view lays out in four
/// places for four purposes, and they go through one private
/// `ProtocolModel::layout_config` — which is a claim about a private helper, so
/// what is asserted here is the consequence: the two `Layout` values a reader
/// meets first, whole, field for field. A second spelling of the configuration
/// at either site fails this on `view_chips`, and on the positions with it.
#[test]
fn the_boot_layout_is_the_layout_the_canvas_draws() {
    use brightfield_protocol::layout::Flow;
    use brightfield_shell::protocol::ProtocolModel;

    for flow in [Flow::Horizontal, Flow::Vertical] {
        let booted = ProtocolModel::boot_layout(&housing_boot().protocol, flow);
        let model = ProtocolModel::new(housing_boot().protocol, flow);
        assert_eq!(
            model.layout(),
            &booted,
            "at {flow:?} the boot's layout and the model's opening layout are \
             two different arrangements of the same Protocol"
        );
        let table = housing_boot()
            .protocol
            .table
            .clone()
            .expect("the fixture has a table");
        assert_eq!(
            booted
                .view_chips
                .get(&table)
                .map(|chips| chips.iter().map(|c| c.label.clone()).collect::<Vec<_>>()),
            Some(NodeView::chip_labels()),
            "the boot laid the table out without the chips it draws"
        );
    }
}

/// **Opening a second data file while the canvas holds the GRAPH comes back to
/// the new table's dashboard**, by the same identity rule a grid comes back by.
///
/// The sibling of
/// `opening_a_second_file_over_a_grid_resets_the_latch_to_the_new_tables_dashboard`,
/// and it needs its own test because `CanvasHolds::Graph` names no node and so
/// cannot be compared against the current table the way a `View` is.
/// `MeridianApp::graph_reached_from` is the record that carries the identity for
/// it. Take the comparison out — let a latched `Graph` count as held on a
/// window that has a table — and a second, unrelated file opens onto the first
/// file's map, with the rail listing a Protocol the canvas is not drawing.
#[test]
fn opening_a_second_file_over_the_graph_comes_back_to_the_new_tables_dashboard() {
    let mut win = Live::open(housing_boot());
    win.settle();
    win.click_chip();
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::Graph,
        "the fixture: the first file's graph is on the canvas before the \
         second file opens"
    );

    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/point_map_baseline.csv");
    let ctx = win.ctx.clone();
    win.app
        .open_data_file(&ctx, path.to_str().expect("utf-8 fixture path"));
    win.settle();

    let table = win
        .app
        .protocol_model()
        .table()
        .cloned()
        .expect("the second file opened as a one-step Protocol with a table");
    assert_eq!(
        win.app.canvas_holds(),
        &CanvasHolds::View {
            node: table,
            view: NodeView::Dashboard,
        },
        "the latch still holds the graph after a second, unrelated file opened"
    );
    assert!(
        !win.chip().filled,
        "…and the chip still says the canvas holds the graph"
    );

    // …and the round trip is now the new file's. A `graph_reached_from` left
    // pointing at the first file's table would send this click back to a node
    // the rail no longer lists.
    win.click_chip();
    win.click_chip();
    assert_eq!(
        win.app.canvas_holds().view(),
        Some(NodeView::Dashboard),
        "the chip's round trip landed somewhere other than the new table's \
         dashboard"
    );
    assert!(
        win.row("dashboard").on_canvas.is_some(),
        "…and the bar is not on the new table's dashboard row"
    );
}
