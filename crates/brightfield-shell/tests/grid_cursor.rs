//! **The grid's cursor is on a cell, and the keys move it.**
//!
//! A press on a cell of the table's grid puts the cursor there and gives the
//! grid the keyboard; the grid's keys then move the cursor one cell at a time.
//! The cell is ringed in the focus ink, its row and its column stand on a
//! quiet ground, and its column's header carries the focus ink along its foot.
//! The status band names where the cursor is, and while the grid holds focus
//! the keys that pan and zoom the chart leave the chart alone.
//!
//! Every assertion reads a drawn frame of the housing sample — the grid's
//! drawn record, the shapes the frame handed the painter, the text the status
//! band drew — rather than the field the press or the key wrote, because a
//! field can say *row 3* over a frame that drew the ring nowhere.

use brightfield_shell::app::ChartDoc;
use brightfield_shell::data_grid::{
    show_table_sized, CellText, ColumnWidths, CursorDrawn, GridColumn, GridCursor, HeaderStyle,
    RowGround, RowSource, SetWidths, TableCursor, TableDrawn, CURSOR_FOOT_HEIGHT,
    CURSOR_GROUND_OPACITY, CURSOR_RING_WIDTH, DATA,
};
use brightfield_shell::design::Mode;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp, GRID_CURSOR_STATUS_ID};
use brightfield_workbench::{chrome, PaneKey};
use meridian_design::semantic;

/// The housing sample every window criterion opens.
fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// The focus ink, in the light mode every window here draws in.
fn focus_ink() -> egui::Color32 {
    chrome::colour(semantic(false).borders.focus)
}

/// The row hover ground, whole — the row under the pointer.
fn hover_ground() -> egui::Color32 {
    chrome::colour(semantic(false).rows.hover_background)
}

fn press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Every text galley a frame handed the painter, with where it went.
fn collect_text(shape: &egui::Shape, out: &mut Vec<(egui::Pos2, String)>) {
    match shape {
        egui::Shape::Text(text) => out.push((text.pos, text.galley.text().to_owned())),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, out);
            }
        }
        _ => {}
    }
}

/// Every rect a frame stroked, flattened in paint order.
fn collect_strokes(shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Stroke)>) {
    match shape {
        egui::Shape::Rect(rect) if rect.stroke.width > 0.0 => out.push((rect.rect, rect.stroke)),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_strokes(shape, out);
            }
        }
        _ => {}
    }
}

/// One headless window over the housing sample.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The shapes the last frame handed the painter, in paint order.
    shapes: Vec<egui::Shape>,
}

impl Window {
    fn housing() -> Self {
        let path = housing();
        let boot = Boot::data_file(path.to_str().expect("utf-8 path")).expect("the sample opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            shapes: Vec::new(),
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
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        self.shapes = out
            .shapes
            .into_iter()
            .map(|clipped| clipped.shape)
            .collect();
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    fn doc(&self) -> &ChartDoc {
        self.app.chart_doc()
    }

    /// What the grid's table laid out on the last frame.
    fn drawn(&self) -> TableDrawn {
        self.doc()
            .grid_drawn()
            .expect("the grid drew a table this frame")
            .clone()
    }

    /// The cursor's marks as the last frame drew them.
    fn cursor(&self) -> CursorDrawn {
        self.drawn()
            .cursor
            .expect("the grid drew a cursor this frame")
    }

    /// The middle of the visible part of the cell at (`row`, `col`), where
    /// the last frame drew it.
    fn cell_centre(&self, row: u64, col: usize) -> egui::Pos2 {
        let drawn = self.drawn();
        let (_, head, head_clip) = drawn
            .header_cells
            .iter()
            .find(|(c, ..)| *c == col)
            .unwrap_or_else(|| panic!("column {col} drew no header cell"));
        let (_, line, line_clip) = drawn
            .row_cells
            .iter()
            .find(|(r, ..)| *r == row)
            .unwrap_or_else(|| panic!("row {row} was not drawn"));
        egui::pos2(
            head.intersect(*head_clip).center().x,
            line.intersect(*line_clip).center().y,
        )
    }

    /// Press the cell at (`row`, `col`) and let the window settle.
    fn press_cell(&mut self, row: u64, col: usize) {
        let at = self.cell_centre(row, col);
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(at, true), button(at, false)]);
        self.settle();
    }

    /// Press `key` and let the window settle.
    fn key(&mut self, key: egui::Key) {
        self.run(vec![press(key)]);
        self.settle();
    }

    /// The pointer over `at`, held there for a settled frame.
    fn hover(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.settle();
    }

    fn text(&self) -> Vec<(egui::Pos2, String)> {
        let mut out = Vec::new();
        for shape in &self.shapes {
            collect_text(shape, &mut out);
        }
        out
    }

    fn strokes(&self) -> Vec<(egui::Rect, egui::Stroke)> {
        let mut out = Vec::new();
        for shape in &self.shapes {
            collect_strokes(shape, &mut out);
        }
        out
    }

    /// The galley the last frame drew inside `rect`, the cell's value.
    fn text_in(&self, rect: egui::Rect) -> String {
        self.text()
            .into_iter()
            .filter(|(pos, _)| rect.contains(*pos + egui::vec2(1.0, 1.0)))
            .map(|(_, text)| text)
            .collect::<Vec<_>>()
            .join("")
    }

    /// The chart's frame as the navigation verbs left it: each plot's extent
    /// and the axis lock.
    fn chart_frame(&self) -> (String, String) {
        let doc = self.doc();
        let extents = doc
            .live_dashboard()
            .map(|live| {
                let mut plots: Vec<_> = live.view_extents().iter().collect();
                plots.sort_by(|a, b| a.0.cmp(b.0));
                format!("{plots:?}")
            })
            .unwrap_or_default();
        (extents, format!("{:?}", doc.axis_lock))
    }
}

/// The grid's pane key — what a press on a cell gives focus to.
fn grid_key() -> PaneKey {
    PaneKey::new(DATA)
}

/// **AC1: a press on a cell focuses the grid and puts the cursor there, and
/// each of the grid's keys then moves it one cell.**
#[test]
fn a_press_on_a_cell_focuses_the_grid_and_each_key_moves_the_cursor_a_cell() {
    let mut win = Window::housing();
    assert_eq!(
        win.drawn().cursor,
        None,
        "the grid drew a cursor before anything was pressed"
    );
    assert_ne!(win.app.focused_pane(), Some(grid_key()));

    win.press_cell(2, 1);
    assert_eq!(
        win.app.focused_pane(),
        Some(grid_key()),
        "a press on a cell did not give the grid focus"
    );
    assert_eq!(win.cursor().at, GridCursor { row: 2, col: 1 });

    for (key, row, col) in [
        (egui::Key::J, 3, 1),
        (egui::Key::K, 2, 1),
        (egui::Key::L, 2, 2),
        (egui::Key::H, 2, 1),
        (egui::Key::ArrowDown, 3, 1),
        (egui::Key::ArrowUp, 2, 1),
        (egui::Key::ArrowRight, 2, 2),
        (egui::Key::ArrowLeft, 2, 1),
    ] {
        win.key(key);
        assert_eq!(
            win.cursor().at,
            GridCursor { row, col },
            "after {key:?} the grid's record names the wrong cell"
        );
        assert!(
            win.cursor().ring.is_some(),
            "after {key:?} the cursor's cell drew no ring"
        );
    }

    // And a second press moves it there, rather than the first press latching.
    win.press_cell(4, 3);
    assert_eq!(win.cursor().at, GridCursor { row: 4, col: 3 });
}

/// **AC2: the cell ringed in the focus ink, its row and column on the row
/// hover ground at 0.7, its column's header with the focus ink along its foot,
/// and the row under the pointer drawn apart from the cursor's.**
#[test]
fn the_cursors_cell_is_ringed_and_its_row_and_column_stand_on_a_quiet_ground() {
    let mut win = Window::housing();
    win.press_cell(2, 1);
    // The pointer off the cursor's row, on a row of its own.
    let elsewhere = win.cell_centre(5, 3);
    win.hover(elsewhere);

    let drawn = win.drawn();
    let cursor = drawn.cursor.clone().expect("a cursor drew");

    // The ring: the focus ink at 1.5, around the cursor's cell — and painted,
    // not only recorded.
    let (ring, stroke) = cursor.ring.expect("the cursor's cell drew a ring");
    assert_eq!(stroke, egui::Stroke::new(CURSOR_RING_WIDTH, focus_ink()));
    assert!((CURSOR_RING_WIDTH - 1.5).abs() < f32::EPSILON);
    assert!(
        win.strokes().contains(&(ring, stroke)),
        "the record names a ring at {ring:?} that the frame did not paint"
    );
    let (_, head, _) = drawn.header_cells.iter().find(|(c, ..)| *c == 1).unwrap();
    let (_, line, _) = drawn.row_cells.iter().find(|(r, ..)| *r == 2).unwrap();
    assert!(
        (ring.left() - head.left()).abs() < 0.5
            && (ring.right() - head.right()).abs() < 0.5
            && (ring.top() - line.top()).abs() < 0.5
            && (ring.bottom() - line.bottom()).abs() < 0.5,
        "the ring {ring:?} is not the cell at column {head:?} and row {line:?}"
    );

    // The header's foot: the focus ink, 2 high, along the bottom of the
    // cursor column's header.
    let (foot, ink) = cursor.header_foot.expect("the header drew its foot");
    assert_eq!(ink, focus_ink());
    assert!((foot.height() - CURSOR_FOOT_HEIGHT).abs() < 0.01);
    assert!((CURSOR_FOOT_HEIGHT - 2.0).abs() < f32::EPSILON);
    assert!((foot.bottom() - head.bottom()).abs() < 0.5);
    assert!((foot.left() - head.left()).abs() < 0.5 && (foot.right() - head.right()).abs() < 0.5);

    // The ground: the row hover ground at 0.7, under the cursor's row and down
    // every other drawn row's cell in its column.
    assert!((CURSOR_GROUND_OPACITY - 0.7).abs() < f32::EPSILON);
    assert_eq!(cursor.ground, hover_ground().gamma_multiply(0.7));
    assert_eq!(drawn.row_grounds.get(&2), Some(&RowGround::Cursor));
    let others: Vec<u64> = drawn
        .row_cells
        .iter()
        .map(|(row, ..)| *row)
        .filter(|row| *row != 2)
        .collect();
    assert!(!others.is_empty());
    for row in &others {
        assert!(
            cursor.column_ground.contains(row),
            "row {row}'s cell in the cursor's column stands off the ground: {:?}",
            cursor.column_ground
        );
    }
    assert!(
        !cursor.column_ground.contains(&2),
        "the cursor's own cell took the column's ground over its row's"
    );

    // The row under the pointer: the hover ground whole, apart from the
    // cursor's row; and the cursor's row keeps its ground under the pointer.
    assert_eq!(drawn.row_grounds.get(&5), Some(&RowGround::Hover));
    assert_ne!(cursor.ground, hover_ground());
    let on_cursor_row = win.cell_centre(2, 3);
    win.hover(on_cursor_row);
    assert_eq!(win.drawn().row_grounds.get(&2), Some(&RowGround::Cursor));
}

/// A row source with a selected row, as the Steps sheet's is: the data grid's
/// rows never report one, so the selection's half of AC2 is held on the table
/// every grid draws through.
struct Selecting {
    columns: Vec<GridColumn>,
}

impl RowSource for Selecting {
    fn prepare(&mut self, _visible: std::ops::Range<u64>) {}

    fn total_rows(&self) -> u64 {
        8
    }

    fn columns(&self) -> &[GridColumn] {
        &self.columns
    }

    fn cell_text(&self, row: u64, col: usize) -> Option<CellText> {
        Some(CellText::primary(format!("r{row}c{col}")))
    }

    fn selected_row(&self) -> Option<u64> {
        Some(2)
    }
}

/// **AC2: a selected row keeps its selection wash, and the cursor's ring is
/// drawn over it; the row under the pointer is drawn apart from the cursor's.**
#[test]
fn a_selected_row_keeps_its_wash_and_the_ring_is_drawn_over_it() {
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let mut source = Selecting {
        columns: (0..3)
            .map(|i| GridColumn {
                name: format!("c{i}"),
                numeric: false,
            })
            .collect(),
    };
    let cursor = TableCursor {
        at: Some(GridCursor { row: 2, col: 1 }),
        reveal: false,
    };
    let mut frame = |events: Vec<egui::Event>| {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let mut drawn = None;
        let out = ctx.run_ui(raw, |ui| {
            drawn = Some(show_table_sized(
                ui,
                "selecting",
                Mode::Light,
                &mut source,
                ColumnWidths::Declared,
                HeaderStyle::Plain,
                &SetWidths::new(),
                Some(cursor),
            ));
        });
        (drawn.expect("the table drew"), out.shapes)
    };
    let (first, _) = frame(Vec::new());
    let (_, row4, _) = *first.row_cells.iter().find(|(r, ..)| *r == 4).unwrap();
    let (drawn, shapes) = frame(vec![egui::Event::PointerMoved(row4.center())]);

    assert_eq!(drawn.row_grounds.get(&2), Some(&RowGround::Selection));
    assert_eq!(drawn.row_grounds.get(&4), Some(&RowGround::Hover));
    let (ring, stroke) = drawn
        .cursor
        .as_ref()
        .and_then(|c| c.ring)
        .expect("the cursor's cell drew a ring on the selected row");

    // In paint order: the wash's border, then the ring over it.
    let mut strokes = Vec::new();
    for clipped in &shapes {
        collect_strokes(&clipped.shape, &mut strokes);
    }
    let border = chrome::colour(semantic(false).rows.selected_border);
    let wash = strokes
        .iter()
        .position(|(_, s)| s.color == border)
        .expect("the selected row drew its wash");
    let ringed = strokes
        .iter()
        .position(|entry| *entry == (ring, stroke))
        .expect("the frame painted the ring it recorded");
    assert!(
        ringed > wash,
        "the ring was painted under the selection wash ({ringed} before {wash})"
    );
    // And the selected row's cell in the cursor's column keeps the wash rather
    // than taking the column's ground.
    let column_ground = &drawn.cursor.as_ref().unwrap().column_ground;
    assert!(!column_ground.contains(&2));
}

/// **AC3: the status band names the cursor's row among the rows shown, its
/// column's name and the cell's value.**
#[test]
fn the_status_band_names_the_cursors_row_column_and_value() {
    let mut win = Window::housing();
    win.press_cell(2, 1);
    win.key(egui::Key::J);

    let drawn = win.drawn();
    let cursor = drawn.cursor.clone().expect("a cursor drew");
    let (ring, _) = cursor.ring.expect("the ring drew");
    // The value as the grid drew it in the cursor's cell, and the column's
    // name as its header drew it — both read off the frame's galleys.
    let value = win.text_in(ring);
    assert!(!value.is_empty(), "the cursor's cell drew no value");
    let (_, head, _) = drawn.header_cells.iter().find(|(c, ..)| *c == 1).unwrap();
    let column = win.text_in(*head);
    assert!(
        column.contains("house_age"),
        "column 1's header drew {column:?}"
    );

    let expected = format!("row 4 of {} · house_age · {value}", drawn.rows);
    assert!(
        win.app.rail().drawn.contains(&GRID_CURSOR_STATUS_ID),
        "the status band drew no cursor line: {:?}",
        win.app.rail().drawn
    );
    let lines: Vec<String> = win.text().into_iter().map(|(_, t)| t).collect();
    assert!(
        lines.iter().any(|line| line == &expected),
        "the status band does not read {expected:?}; the frame drew {:?}",
        lines
            .iter()
            .filter(|l| l.starts_with("row "))
            .collect::<Vec<_>>()
    );
}

/// **AC4: moving past the last row drawn scrolls the grid so the cursor's row
/// is drawn; a move past the table's last row or column leaves the cursor.**
#[test]
fn moving_past_the_last_row_drawn_scrolls_and_the_tables_edge_holds_the_cursor() {
    let mut win = Window::housing();
    win.press_cell(0, 0);

    // The last row drawn whole inside the grid's clip.
    let whole_rows = |drawn: &TableDrawn| -> Vec<u64> {
        drawn
            .row_cells
            .iter()
            // Whole top to bottom: a row spans every column, and the clip is
            // as wide as the columns that fit.
            .filter(|(_, rect, clip)| {
                clip.y_range().contains(rect.top() + 0.5)
                    && clip.y_range().contains(rect.bottom() - 0.5)
            })
            .map(|(row, ..)| *row)
            .collect()
    };
    let last_whole = *whole_rows(&win.drawn()).iter().max().expect("rows drew");
    let target = last_whole + 6;
    for _ in 0..target {
        win.key(egui::Key::J);
    }
    let drawn = win.drawn();
    assert_eq!(drawn.cursor.as_ref().unwrap().at.row, target);
    assert!(
        whole_rows(&drawn).contains(&target),
        "the cursor moved to row {target}, past the {last_whole} drawn at the \
         start, and the grid drew it nowhere whole: {:?}",
        whole_rows(&drawn)
    );
    assert!(drawn.cursor.as_ref().unwrap().ring.is_some());

    // The table's last row and last column hold the cursor.
    let (rows, columns) = (drawn.rows, drawn.columns);
    win.app.chart_doc_mut().grid_cursor = Some(GridCursor {
        row: rows - 1,
        col: columns - 1,
    });
    win.app.chart_doc_mut().grid_cursor_reveal = true;
    win.settle();
    let corner = GridCursor {
        row: rows - 1,
        col: columns - 1,
    };
    for key in [
        egui::Key::J,
        egui::Key::ArrowDown,
        egui::Key::L,
        egui::Key::ArrowRight,
    ] {
        win.key(key);
        assert_eq!(
            win.cursor().at,
            corner,
            "{key:?} moved the cursor off the table"
        );
    }
    // And the first row and column, the other way.
    win.app.chart_doc_mut().grid_cursor = Some(GridCursor { row: 0, col: 0 });
    win.app.chart_doc_mut().grid_cursor_reveal = true;
    win.settle();
    for key in [
        egui::Key::K,
        egui::Key::ArrowUp,
        egui::Key::H,
        egui::Key::ArrowLeft,
    ] {
        win.key(key);
        assert_eq!(
            win.cursor().at,
            GridCursor { row: 0, col: 0 },
            "{key:?} moved the cursor off the table"
        );
    }
}

/// The frame keys AC5 names.
const FRAME_KEYS: [egui::Key; 8] = [
    egui::Key::ArrowLeft,
    egui::Key::ArrowRight,
    egui::Key::ArrowUp,
    egui::Key::ArrowDown,
    egui::Key::Equals,
    egui::Key::Minus,
    egui::Key::X,
    egui::Key::Num0,
];

/// **AC5: while the grid has focus the arrows, `=`, `-`, `x` and `0` leave the
/// chart's extent as it was; with no pane focused, or another, they move it.**
#[test]
fn with_the_grid_focused_the_frame_keys_leave_the_chart_alone() {
    let mut win = Window::housing();
    win.press_cell(2, 1);
    assert_eq!(win.app.focused_pane(), Some(grid_key()));

    let before = win.chart_frame();
    for key in FRAME_KEYS {
        win.key(key);
        assert_eq!(
            win.chart_frame(),
            before,
            "{key:?} moved the chart while the grid held focus"
        );
    }
    assert!(!win.doc().navigated(), "the chart was left navigated");

    // With no pane focused, the same keys move the frame as before.
    win.app.clear_focus();
    win.key(egui::Key::Equals);
    assert!(
        win.doc().navigated(),
        "`=` did not zoom the chart with no pane focused"
    );
    win.key(egui::Key::Num0);
    assert!(!win.doc().navigated(), "`0` did not reset the chart");
    let before = win.chart_frame();
    win.key(egui::Key::X);
    assert_ne!(win.chart_frame(), before, "`x` did not cycle the axis lock");

    // And with another pane focused.
    let other = PaneKey::new(brightfield_shell::app::CHART);
    assert!(win.app.focus_pane(other), "the chart pane took no focus");
    win.key(egui::Key::Equals);
    assert!(
        win.doc().navigated(),
        "`=` did not zoom the chart with the chart pane focused"
    );
}
