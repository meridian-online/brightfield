//! **The Inspector rail explains the operation in hand.**
//!
//! While the grid's foot row asks `where`, the operation in hand is
//! `filter-rows`; while it asks `order by`, it is `sort-rows`. The key that
//! opens the field opens the Inspector rail on a panel that says what the
//! operation reads and takes, in arc's own sentences, beside the values
//! brightfield holds for it and where each came from. Esc from the field shuts
//! the rail again when the key was what opened it.
//!
//! The runner is the `brightfield-shell` binary cargo built for this suite, so
//! every sentence the panel is checked for is read from arc's own
//! `operation describe` rather than written down here.

use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::grid_foot::FootField;
use brightfield_shell::operation::{self, OperationDescription};
use brightfield_shell::run::Runner;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement::INSPECTOR_RAIL;

fn data(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
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

/// The table the housing sample's one step makes: the file's stem, not the
/// step's name.
const TABLE: &str = "california_housing_sample";

/// The title the operation pane carries in the rail's strip.
const PANE_TITLE: &str = "Operation";

/// One headless window over the housing sample.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    shapes: Vec<egui::Shape>,
}

impl Window {
    fn housing(runner: Option<Runner>) -> Self {
        let path = data("california_housing_sample.csv");
        let boot = Boot::data_file(path.to_str().expect("utf-8 path")).expect("the file opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(runner),
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

    fn cell_centre(&self, row: u64, col: usize) -> egui::Pos2 {
        let drawn = self
            .doc()
            .grid_drawn()
            .expect("the grid drew a table this frame")
            .clone();
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

    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(at, true), button(at, false)]);
        self.settle();
    }

    fn press_cell(&mut self, row: u64, col: usize) {
        let at = self.cell_centre(row, col);
        self.click(at);
    }

    /// Press `key` as a keyboard does, with the text the keystroke makes.
    fn keystroke(&mut self, key: egui::Key, text: &str) {
        self.run(vec![press(key), egui::Event::Text(text.to_owned())]);
        self.settle();
    }

    fn key(&mut self, key: egui::Key) {
        self.run(vec![press(key)]);
        self.settle();
    }

    /// The text the inspector rail drew on the last frame.
    fn rail_text(&self) -> Vec<String> {
        let Some(rect) = self.app.region_rect(INSPECTOR_RAIL) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for shape in &self.shapes {
            collect_text(shape, &mut out);
        }
        out.into_iter()
            .filter(|(pos, _)| rect.contains(*pos + egui::vec2(1.0, 1.0)))
            .map(|(_, t)| t)
            .collect()
    }

    /// Draw frames until the rail draws `text`, or give up after a while: the
    /// description is asked on a worker, so it lands a few frames late.
    fn until_rail_draws(&mut self, text: &str) -> Vec<String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            self.run(Vec::new());
            let drawn = self.rail_text();
            if drawn.iter().any(|t| t == text) || std::time::Instant::now() > deadline {
                return drawn;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn showing_operation_pane(&self) -> bool {
        !self.app.rail_is_collapsed(INSPECTOR_RAIL)
            && self.app.rail_pane_title(INSPECTOR_RAIL).as_deref() == Some(PANE_TITLE)
    }
}

fn described(long_name: &str) -> OperationDescription {
    operation::describe(&runner(), long_name).expect("arc describes the operation")
}

fn has(drawn: &[String], text: &str) -> bool {
    drawn.iter().any(|t| t == text)
}

/// **`w` on a cell opens the Inspector rail on the operation panel.** A data
/// file's rail starts collapsed, so the key is what opens it.
#[test]
fn w_opens_the_inspector_on_the_operation_panel() {
    let mut win = Window::housing(Some(runner()));
    assert!(
        win.app.rail_is_collapsed(INSPECTOR_RAIL),
        "a data file's inspector rail starts collapsed"
    );
    win.press_cell(2, HOUSE_AGE);
    win.keystroke(egui::Key::W, "w");
    assert!(matches!(
        win.doc().foot_asking(),
        Some((FootField::Where, _))
    ));
    assert!(
        win.showing_operation_pane(),
        "the rail is not open on the operation pane: collapsed {}, showing {:?}",
        win.app.rail_is_collapsed(INSPECTOR_RAIL),
        win.app.rail_pane_title(INSPECTOR_RAIL)
    );
}

/// **Esc from the field shuts the rail the key opened.**
#[test]
fn escape_shuts_the_rail_it_opened() {
    let mut win = Window::housing(Some(runner()));
    win.press_cell(2, HOUSE_AGE);
    win.keystroke(egui::Key::W, "w");
    assert!(win.showing_operation_pane());
    win.key(egui::Key::Escape);
    assert_eq!(win.doc().foot_asking(), None);
    assert!(
        win.app.rail_is_collapsed(INSPECTOR_RAIL),
        "Esc left open the rail the key opened"
    );
}

/// **Esc leaves open a rail that was open before the key**, and Enter leaves
/// the rail as it is.
#[test]
fn escape_leaves_an_already_open_rail_open() {
    let mut win = Window::housing(Some(runner()));
    let collapse = win
        .app
        .rail_collapse_rect(INSPECTOR_RAIL)
        .expect("the collapsed rail draws its control");
    win.click(collapse.center());
    assert!(
        !win.app.rail_is_collapsed(INSPECTOR_RAIL),
        "the control did not open the rail"
    );

    win.press_cell(2, HOUSE_AGE);
    win.keystroke(egui::Key::W, "w");
    assert!(win.showing_operation_pane());
    win.key(egui::Key::Escape);
    assert_eq!(win.doc().foot_asking(), None);
    assert!(
        !win.app.rail_is_collapsed(INSPECTOR_RAIL),
        "Esc shut a rail the key did not open"
    );

    // Enter leaves the rail as it is, even one the key opened.
    let mut win = Window::housing(Some(runner()));
    win.press_cell(2, HOUSE_AGE);
    win.keystroke(egui::Key::W, "w");
    win.key(egui::Key::Enter);
    assert_eq!(win.doc().foot_asking(), None);
    assert!(
        win.showing_operation_pane(),
        "Enter moved the rail the key opened"
    );
}

/// **The panel lists the table, the column and `where`, with arc's sentences
/// and where each value came from.**
#[test]
fn the_panel_lists_table_column_and_where_with_arcs_sentences() {
    let arc = described("filter-rows");
    let where_ = arc
        .parameters
        .iter()
        .find(|p| p.key == "where")
        .expect("arc's filter-rows takes `where`");

    let mut win = Window::housing(Some(runner()));
    win.press_cell(2, HOUSE_AGE);
    win.keystroke(egui::Key::W, "w");
    let (_, text) = win.doc().foot_asking().expect("the field asks");
    let text = text.to_owned();

    let drawn = win.until_rail_draws(&where_.description);
    for expected in [
        "filter-rows",
        arc.summary.as_str(),
        TABLE,
        "the node in view",
        "house_age",
        "the cursor",
        "where",
        text.as_str(),
        "asked",
        where_.description.as_str(),
    ] {
        assert!(
            has(&drawn, expected),
            "the panel did not draw {expected:?}: {drawn:?}"
        );
    }
    for role in &arc.reads {
        assert!(
            has(&drawn, &role.description),
            "the panel did not draw arc's sentence for {}: {drawn:?}",
            role.name
        );
    }
    assert_eq!(where_.offered.len(), 6, "arc offers six comparisons");
    for offered in &where_.offered {
        assert!(
            has(&drawn, &offered.word) && has(&drawn, &offered.sign),
            "the panel did not offer {} ({}): {drawn:?}",
            offered.word,
            offered.sign
        );
    }
    assert!(
        !has(&drawn, "load"),
        "the panel named the step rather than the table it makes: {drawn:?}"
    );
}

/// **With no runner, the row still asks and the panel says why it has nothing
/// to show.**
#[test]
fn with_no_runner_the_row_still_asks_and_the_panel_says_why() {
    let mut win = Window::housing(None);
    win.press_cell(2, HOUSE_AGE);
    win.keystroke(egui::Key::W, "w");
    assert!(matches!(
        win.doc().foot_asking(),
        Some((FootField::Where, _))
    ));
    assert!(win.showing_operation_pane());
    win.settle();
    let drawn = win.rail_text();
    assert!(has(&drawn, "filter-rows"), "{drawn:?}");
    assert!(
        drawn
            .iter()
            .any(|t| t.starts_with("No description of filter-rows")),
        "the panel does not say it has no description: {drawn:?}"
    );
    assert!(
        drawn.iter().any(|t| t.contains("no runner")),
        "the panel does not say why: {drawn:?}"
    );
}

/// **`o` puts `sort-rows` in hand**, drawn from `order_by` and its directions.
#[test]
fn the_panel_shows_sort_rows_for_order_by() {
    let arc = described("sort-rows");
    let order_by = arc
        .parameters
        .iter()
        .find(|p| p.key == "order_by")
        .expect("arc's sort-rows takes `order_by`");

    let mut win = Window::housing(Some(runner()));
    win.press_cell(2, HOUSE_AGE);
    win.key(egui::Key::O);
    assert!(win.showing_operation_pane());

    let drawn = win.until_rail_draws(&order_by.description);
    for expected in [
        "sort-rows",
        TABLE,
        "house_age",
        "the cursor",
        "order_by",
        order_by.description.as_str(),
    ] {
        assert!(
            has(&drawn, expected),
            "the panel did not draw {expected:?}: {drawn:?}"
        );
    }
    for offered in &order_by.offered {
        assert!(
            has(&drawn, &offered.word),
            "the panel did not offer {}: {drawn:?}",
            offered.word
        );
    }
}

/// **Esc from a field opened by a press shuts no rail**, even after a key
/// opened the rail for an asking that ended another way.
#[test]
fn escape_from_a_field_a_press_opened_leaves_the_rail_open() {
    let mut win = Window::housing(Some(runner()));
    win.press_cell(2, HOUSE_AGE);
    win.keystroke(egui::Key::W, "w");
    assert!(win.showing_operation_pane());
    // A press on a cell takes the keyboard from the field, which ends the
    // asking without Esc.
    win.press_cell(3, HOUSE_AGE);
    assert_eq!(win.doc().foot_asking(), None);

    let band = win.doc().foot_drawn().expect("the band drew").rect;
    win.click(egui::pos2(band.left() + 80.0, band.center().y));
    assert!(
        matches!(win.doc().foot_asking(), Some((FootField::Where, _))),
        "the press did not open `where`"
    );
    win.key(egui::Key::Escape);
    assert_eq!(win.doc().foot_asking(), None);
    assert!(
        !win.app.rail_is_collapsed(INSPECTOR_RAIL),
        "Esc shut a rail no key opened for this asking"
    );
}
