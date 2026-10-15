//! **A row at the foot of the grid asks a condition, and a key fills it in
//! from the cursor's cell.**
//!
//! The band under the table holds a `where` field and an `order by` field, and
//! the count of rows and columns at its trailing end. `w` on a cell opens
//! `where` with the cell's column compared to its value, the value selected; a
//! comparison key swaps the sign and keeps the value, and any other key
//! replaces the value. The grid follows each condition that parses while the
//! analyst types, and Enter applies it as the document's grid view.
//!
//! Every assertion reads a drawn frame of a headless window — the grid's drawn
//! record, the band's record, the text the frame drew — and the counts it
//! compares against are the engine's own, asked through the same session.

use brightfield_engine::RowsAudience;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::data_grid::{GridCursor, TableDrawn};
use brightfield_shell::design::Mode;
use brightfield_shell::grid_foot::{FootField, FOOT_BAND_HEIGHT};
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp, GRID_FOOT_STATUS_ID};

fn data(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
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

/// The column of the housing sample a press puts the cursor in: `house_age`.
const HOUSE_AGE: usize = 1;

/// One headless window over a data file.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    shapes: Vec<egui::Shape>,
}

impl Window {
    fn open(name: &str) -> Self {
        let path = data(name);
        let boot = Boot::data_file(path.to_str().expect("utf-8 path")).expect("the file opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            shapes: Vec::new(),
        };
        win.settle();
        win
    }

    fn housing() -> Self {
        Self::open("california_housing_sample.csv")
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

    fn drawn(&self) -> TableDrawn {
        self.doc()
            .grid_drawn()
            .expect("the grid drew a table this frame")
            .clone()
    }

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

    fn press_cell(&mut self, row: u64, col: usize) {
        let at = self.cell_centre(row, col);
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(at, true), button(at, false)]);
        self.settle();
    }

    fn key(&mut self, key: egui::Key) {
        self.run(vec![press(key)]);
        self.settle();
    }

    /// Press `key` as a keyboard does, with the text the keystroke makes.
    fn keystroke(&mut self, key: egui::Key, text: &str) {
        self.run(vec![press(key), egui::Event::Text(text.to_owned())]);
        self.settle();
    }

    /// Type `text` as a keyboard does: the text a keystroke makes.
    fn type_text(&mut self, text: &str) {
        self.run(vec![egui::Event::Text(text.to_owned())]);
        self.settle();
    }

    fn text(&self) -> Vec<String> {
        let mut out = Vec::new();
        for shape in &self.shapes {
            collect_text(shape, &mut out);
        }
        out.into_iter().map(|(_, t)| t).collect()
    }

    fn text_in(&self, rect: egui::Rect) -> Vec<String> {
        let mut out = Vec::new();
        for shape in &self.shapes {
            collect_text(shape, &mut out);
        }
        out.into_iter()
            .filter(|(pos, _)| rect.contains(*pos + egui::vec2(1.0, 1.0)))
            .map(|(_, t)| t)
            .collect()
    }

    /// The value the grid drew in the cursor's cell.
    fn cursor_value(&self) -> String {
        self.drawn()
            .cursor
            .and_then(|c| c.value)
            .expect("the cursor's cell drew a value")
    }

    /// The engine's count of the grid's step under `condition`, or the error
    /// it refuses the condition with.
    fn engine_count(&mut self, condition: &str) -> Result<u64, String> {
        let doc = self.app.chart_doc_mut();
        let mark = doc.live_dashboard().map_or(0, LiveDashboard::rows_mark);
        let session = doc.live_coordinator().expect("a live session").session();
        session
            .step_rows_count_where(mark, RowsAudience::Reader, condition)
            .map_err(|e| e.to_string())
    }

    fn asking(&self) -> Option<(FootField, String)> {
        self.doc()
            .foot_asking()
            .map(|(field, text)| (field, text.to_owned()))
    }

    /// The status band's line for the band, as the frame drew it.
    fn foot_status(&self) -> Option<String> {
        if !self.app.rail().drawn.contains(&GRID_FOOT_STATUS_ID) {
            return None;
        }
        self.doc().foot_said()
    }
}

/// **`w` on a cell opens `where` with the cell's column equal to its value,
/// and the band draws it after its word.**
#[test]
fn w_on_a_cell_asks_where_from_that_cell() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    let value = win.cursor_value();
    // The key's own text is not typed into the field it opens.
    win.keystroke(egui::Key::W, "w");

    let expected = format!("house_age = {value}");
    assert_eq!(win.asking(), Some((FootField::Where, expected.clone())));
    let band = win.doc().foot_drawn().expect("the band drew").clone();
    assert_eq!(band.where_text, expected);
    let drawn = win.text_in(band.rect);
    assert!(
        drawn.iter().any(|t| t == "where") && drawn.iter().any(|t| t == &expected),
        "the band does not read `where {expected}`: {drawn:?}"
    );
}

/// **A comparison key swaps the sign and keeps the value, and the status band
/// names the comparison in words.**
#[test]
fn a_comparison_key_replaces_the_sign_and_keeps_the_value() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    let value = win.cursor_value();
    win.key(egui::Key::W);

    let frame = win.doc().navigated();
    for (typed, sign, word) in [
        ("=", "=", "is"),
        (">", ">", "over"),
        ("!", "!=", "is not"),
        ("<", "<", "under"),
    ] {
        if typed == "=" {
            // `=` is the chart's zoom key too; the field holds the keyboard.
            win.keystroke(egui::Key::Equals, typed);
            assert_eq!(win.doc().navigated(), frame, "`=` zoomed the chart");
        } else {
            win.type_text(typed);
        }
        assert_eq!(
            win.asking(),
            Some((FootField::Where, format!("house_age {sign} {value}"))),
            "after `{typed}`"
        );
        let said = format!("house_age {word} {value}");
        assert_eq!(win.foot_status(), Some(said.clone()), "after `{typed}`");
        assert!(
            win.text().contains(&said),
            "the status band drew no {said:?}"
        );
    }
}

/// **Any other key replaces the value.**
#[test]
fn another_key_replaces_the_value() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    win.key(egui::Key::W);
    win.type_text("4");
    win.type_text("0");
    assert_eq!(
        win.asking(),
        Some((FootField::Where, "house_age = 40".to_owned()))
    );
    assert_eq!(win.foot_status(), Some("house_age is 40".to_owned()));
}

/// **The grid and its count follow a condition that parses, before Enter, and
/// the document's applied view is left alone.**
#[test]
fn the_grid_follows_each_condition_that_parses() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    let value = win.cursor_value();
    win.key(egui::Key::W);
    win.type_text(">");

    let want = win.engine_count(&format!("house_age > {value}")).unwrap();
    assert!(want > 0 && want < 240, "a count that tells nothing: {want}");
    assert_eq!(win.drawn().rows, want);
    let band = win.doc().foot_drawn().expect("the band drew").clone();
    assert_eq!(band.count, format!("{want} rows \u{b7} 9 columns"));
    assert_eq!(
        win.doc().grid_view().condition,
        None,
        "applied before Enter"
    );
    assert!(!win.app.carries_unsaved_mark(), "marked before Enter");
}

/// **Enter applies the condition as the grid's view, the window's unsaved mark
/// covers it, and the field carries its not-recorded dot.**
#[test]
fn enter_applies_and_marks_the_view_unrecorded() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    let value = win.cursor_value();
    win.key(egui::Key::W);
    win.type_text(">");
    assert!(!win.doc().foot_drawn().unwrap().where_dot);
    win.key(egui::Key::Enter);

    let condition = format!("house_age > {value}");
    let want = win.engine_count(&condition).unwrap();
    assert_eq!(win.asking(), None, "the field still asks after Enter");
    assert_eq!(win.doc().grid_view().condition, Some(condition.clone()));
    assert_eq!(win.drawn().rows, want);
    assert!(win.app.carries_unsaved_mark());
    let band = win.doc().foot_drawn().unwrap().clone();
    assert!(band.where_dot, "the applied field drew no not-recorded dot");
    assert_eq!(band.where_text, condition);
}

/// **Esc puts the field back as it was before editing, and the grid with it.**
#[test]
fn escape_restores_the_field_before_editing() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    let value = win.cursor_value();
    win.key(egui::Key::W);
    win.type_text(">");
    win.key(egui::Key::Enter);
    let applied = format!("house_age > {value}");
    let rows = win.drawn().rows;

    win.key(egui::Key::W);
    win.type_text("<");
    assert_ne!(win.drawn().rows, rows, "the grid did not follow `<`");
    win.key(egui::Key::Escape);

    assert_eq!(win.asking(), None);
    assert_eq!(win.doc().grid_view().condition, Some(applied.clone()));
    assert_eq!(win.doc().foot_drawn().unwrap().where_text, applied);
    assert_eq!(win.drawn().rows, rows);

    // And from an empty field, Esc leaves the whole table.
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    win.key(egui::Key::W);
    win.type_text(">");
    win.key(egui::Key::Escape);
    assert_eq!(win.drawn().rows, 240);
    assert_eq!(win.doc().foot_drawn().unwrap().where_text, "");
    assert!(!win.app.carries_unsaved_mark());
}

/// **An emptied field and Enter show the whole table again.** The field is
/// emptied through the band's entry point every source of a condition shares.
#[test]
fn an_emptied_field_and_enter_shows_the_whole_table() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    win.key(egui::Key::W);
    win.type_text(">");
    win.key(egui::Key::Enter);
    assert!(win.drawn().rows < 240);

    win.app
        .chart_doc_mut()
        .foot_offer(FootField::Where, String::new());
    win.settle();
    assert_eq!(win.asking(), Some((FootField::Where, String::new())));
    win.key(egui::Key::Enter);

    assert_eq!(win.asking(), None);
    assert_eq!(win.doc().grid_view().condition, None);
    assert_eq!(win.drawn().rows, 240);
    assert!(!win.app.carries_unsaved_mark());
}

/// **A condition DuckDB refuses leaves the rows as they were, and the status
/// band says DuckDB's words; Enter does not apply it.** One condition names a
/// column the table does not have, the other carries a second statement.
#[test]
fn a_refused_condition_keeps_the_rows_and_says_why() {
    for refused in ["nosuch > 1", "house_age > 1; select 1"] {
        let mut win = Window::housing();
        win.press_cell(2, HOUSE_AGE);
        win.key(egui::Key::W);
        win.app
            .chart_doc_mut()
            .foot_offer(FootField::Where, String::new());
        win.settle();
        win.type_text(refused);

        let why = win.engine_count(refused).expect_err("DuckDB takes it");
        assert_eq!(win.asking(), Some((FootField::Where, refused.to_owned())));
        assert_eq!(win.drawn().rows, 240, "{refused}: the rows moved");
        assert_eq!(win.foot_status(), Some(why.clone()), "{refused}");

        win.key(egui::Key::Enter);
        assert_eq!(win.doc().grid_view().condition, None, "{refused} applied");
        assert_eq!(win.drawn().rows, 240);
        assert_eq!(
            win.asking(),
            Some((FootField::Where, refused.to_owned())),
            "Enter on {refused} closed the field"
        );
    }
}

/// **A NULL cell asks `is null`, and a column whose name is not a plain
/// identifier, or is one of DuckDB's words, is quoted.** A float's literal
/// keeps the row it came from.
#[test]
fn a_null_cell_asks_is_null() {
    let mut win = Window::open("with_nulls.csv");
    // with_nulls.csv: city, house_age, order, median income; Berkeley's
    // house_age is empty.
    win.press_cell(1, 1);
    win.key(egui::Key::W);
    assert_eq!(
        win.asking(),
        Some((FootField::Where, "house_age is null".to_owned()))
    );
    win.key(egui::Key::Enter);
    assert_eq!(win.drawn().rows, 1);

    let mut win = Window::open("with_nulls.csv");
    win.press_cell(1, 2);
    win.key(egui::Key::W);
    assert_eq!(
        win.asking(),
        Some((FootField::Where, "\"order\" = 2".to_owned()))
    );
    win.key(egui::Key::Escape);

    win.press_cell(2, 3);
    win.key(egui::Key::W);
    let (_, text) = win.asking().expect("the field asks");
    assert!(
        text.starts_with("\"median income\" = "),
        "the column is not quoted: {text:?}"
    );
    win.key(egui::Key::Enter);
    assert_eq!(win.drawn().rows, 1, "{text} lost the row it came from");
}

/// **While the field has the keyboard no row cursor is drawn, and the grid's
/// keys and the frame's keys leave the cursor and the chart alone.**
#[test]
fn no_row_cursor_is_drawn_while_asking() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    assert!(win.drawn().cursor.and_then(|c| c.ring).is_some());
    win.key(egui::Key::W);

    assert_eq!(win.drawn().cursor.and_then(|c| c.ring), None);
    let at = win.doc().grid_cursor;
    let navigated = win.doc().navigated();
    for key in [egui::Key::J, egui::Key::L, egui::Key::Equals, egui::Key::W] {
        win.key(key);
        assert_eq!(win.doc().grid_cursor, at, "{key:?} moved the cursor");
        assert_eq!(win.doc().navigated(), navigated, "{key:?} moved the chart");
    }
    assert!(
        win.asking().is_some(),
        "a key the grid binds took the field"
    );
}

/// **After Enter the cursor sits on the first row shown, in its column.**
#[test]
fn the_cursor_lands_on_the_first_row_after_enter() {
    let mut win = Window::housing();
    win.press_cell(5, HOUSE_AGE);
    win.key(egui::Key::W);
    win.type_text(">");
    win.key(egui::Key::Enter);

    let cursor = win.drawn().cursor.expect("a cursor drew after Enter");
    assert_eq!(
        cursor.at,
        GridCursor {
            row: 0,
            col: HOUSE_AGE
        }
    );
    assert!(cursor.ring.is_some());
}

/// **The band is 26 high, under the table, and its trailing end holds the
/// count of rows and columns.**
#[test]
fn the_band_is_26_high_and_holds_the_count() {
    let win = Window::housing();
    assert!((FOOT_BAND_HEIGHT - 26.0).abs() < f32::EPSILON);
    let band = win.doc().foot_drawn().expect("the band drew").clone();
    assert!((band.rect.height() - 26.0).abs() < 0.01, "{:?}", band.rect);

    let drawn = win.drawn();
    let count = format!("{} rows \u{b7} {} columns", drawn.rows, drawn.columns);
    assert_eq!(band.count, count);
    assert!(
        win.text_in(band.rect).contains(&count),
        "the band drew no {count:?}"
    );
    // Under the table: no row the grid drew reaches into the band.
    for (row, rect, clip) in &drawn.row_cells {
        let shown = rect.intersect(*clip);
        assert!(
            shown.bottom() <= band.rect.top() + 0.5,
            "row {row} is drawn into the band"
        );
    }
    // The order field is drawn after its word too.
    assert!(win.text_in(band.rect).iter().any(|t| t == "order by"));
}

/// **`o` opens `order by` with the cursor's column.**
#[test]
fn o_on_a_cell_asks_order_by_that_column() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    win.key(egui::Key::O);
    assert_eq!(
        win.asking(),
        Some((FootField::OrderBy, "house_age".to_owned()))
    );
}

/// **`=` directly after a comparison key extends it**: `>` `=` asks `>=` and
/// reads *at least*, `<` `=` asks `<=` and reads *at most*, and `!` `=` stays
/// `!=`.
#[test]
fn an_equals_after_a_comparison_key_extends_it() {
    for (keys, sign, word) in [
        ([">", "="], ">=", "at least"),
        (["<", "="], "<=", "at most"),
        (["!", "="], "!=", "is not"),
    ] {
        let mut win = Window::housing();
        win.press_cell(2, HOUSE_AGE);
        let value = win.cursor_value();
        win.key(egui::Key::W);
        for key in keys {
            win.type_text(key);
        }
        assert_eq!(
            win.asking(),
            Some((FootField::Where, format!("house_age {sign} {value}"))),
            "after {keys:?}"
        );
        assert_eq!(
            win.foot_status(),
            Some(format!("house_age {word} {value}")),
            "after {keys:?}"
        );
        let want = win
            .engine_count(&format!("house_age {sign} {value}"))
            .unwrap();
        assert_eq!(win.drawn().rows, want, "after {keys:?}");
    }
}

/// **Enter applies the text the field holds after the frame's own keys**: a
/// last character, or a paste, that lands in the frame Enter does is applied
/// with it.
#[test]
fn a_last_key_and_enter_in_one_frame_apply_the_text_the_field_holds() {
    for last in [
        egui::Event::Text("4".to_owned()),
        egui::Event::Paste("4".to_owned()),
    ] {
        let mut win = Window::housing();
        win.press_cell(2, HOUSE_AGE);
        win.key(egui::Key::W);
        win.run(vec![last.clone(), press(egui::Key::Enter)]);
        win.settle();
        assert_eq!(win.asking(), None, "{last:?}: the field still asks");
        assert_eq!(
            win.doc().grid_view().condition.as_deref(),
            Some("house_age = 4"),
            "{last:?}"
        );
        let want = win.engine_count("house_age = 4").unwrap();
        assert_eq!(win.drawn().rows, want, "{last:?}");
    }
}

/// **An order naming a column the step does not have is refused while it is
/// typed, in DuckDB's words, and Enter applies nothing.**
#[test]
fn an_unknown_column_in_order_by_is_refused_and_enter_applies_nothing() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    win.key(egui::Key::O);
    win.app
        .chart_doc_mut()
        .foot_offer(FootField::OrderBy, String::new());
    win.settle();
    win.type_text("house_ag");

    let said = win.foot_status().expect("the status band says why");
    assert!(
        said.contains("house_ag"),
        "the refusal is not DuckDB's: {said}"
    );
    win.key(egui::Key::Enter);
    assert_eq!(win.doc().grid_view().order, None, "the order was applied");
    assert_eq!(
        win.asking(),
        Some((FootField::OrderBy, "house_ag".to_owned()))
    );
    assert_eq!(win.drawn().rows, 240);
    assert!(
        win.doc().grid_drawn().is_some(),
        "the grid drew no table under the refused order"
    );
}

/// **Esc and Enter stay the window's until the field has held the keys**: a
/// field opened by `foot_offer` and not yet drawn does not take them.
#[test]
fn esc_is_the_windows_until_the_field_has_held_the_keys() {
    let mut win = Window::housing();
    win.app
        .chart_doc_mut()
        .foot_offer(FootField::Where, "house_age > 30".to_owned());
    // The same frame: the field has not been drawn, so the Esc is not its.
    win.run(vec![press(egui::Key::Escape)]);
    assert!(win.asking().is_some(), "a field never drawn took the Esc");
    win.settle();
    win.key(egui::Key::Escape);
    assert_eq!(win.asking(), None, "the drawn field did not take the Esc");
}

/// **A view changes the grid only**: the chart on the same step reads every
/// row it read before, while a condition is previewed and once it is applied.
#[test]
fn a_chart_on_the_same_step_keeps_its_rows_under_the_grids_view() {
    let mut win = Window::housing();
    let chart_rows = |win: &mut Window| -> u64 {
        let doc = win.app.chart_doc_mut();
        let mark = doc.live_dashboard().map_or(0, LiveDashboard::rows_mark);
        let session = doc.live_coordinator().expect("a live session").session();
        session
            .step_rows_count(mark, RowsAudience::Plot)
            .expect("the chart's rows count")
    };
    let before = chart_rows(&mut win);
    assert_eq!(before, 240);

    win.press_cell(2, HOUSE_AGE);
    win.key(egui::Key::W);
    win.type_text(">");
    assert!(
        win.drawn().rows < 240,
        "the grid did not follow the preview"
    );
    assert_eq!(
        chart_rows(&mut win),
        before,
        "the preview narrowed the chart"
    );

    win.key(egui::Key::Enter);
    assert!(win.drawn().rows < 240);
    assert_eq!(
        chart_rows(&mut win),
        before,
        "the applied view narrowed the chart"
    );
}

/// **`ALL` as an order is refused while it is typed**, because the grid's read
/// puts its tie-break after the order and DuckDB takes `ALL` only alone; Enter
/// applies nothing.
#[test]
fn all_as_an_order_is_refused_and_enter_applies_nothing() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    win.key(egui::Key::O);
    win.app
        .chart_doc_mut()
        .foot_offer(FootField::OrderBy, String::new());
    win.settle();
    win.type_text("all");
    assert!(
        win.doc().grid_foot().refused(),
        "`all` was taken as an order"
    );
    assert!(win.foot_status().is_some(), "the status band says nothing");
    win.key(egui::Key::Enter);
    assert_eq!(win.doc().grid_view().order, None, "`all` was applied");
    assert_eq!(win.drawn().rows, 240);
}

/// **A field the last frame did not draw holds no keys**: with the grid laid
/// out as columns the band is drawn nowhere, and Esc is left to the window.
#[test]
fn a_field_not_drawn_last_frame_holds_no_keys() {
    let mut win = Window::housing();
    win.press_cell(2, HOUSE_AGE);
    win.key(egui::Key::W);
    assert!(win.doc().grid_foot().holds_keys());

    win.app
        .set_grid_layout(brightfield_shell::app::GridLayout::Columns);
    win.settle();
    assert!(
        win.doc().foot_drawn().is_none(),
        "the band drew in the columns layout"
    );
    assert!(!win.doc().grid_foot().holds_keys());
    win.key(egui::Key::Escape);
    assert!(win.asking().is_some(), "a band drawn nowhere took the Esc");
}
