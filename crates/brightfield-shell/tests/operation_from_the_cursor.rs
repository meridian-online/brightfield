//! **An operation applied from the keyboard to what the cursor is on, driven
//! end to end**: the cursor on a cell, the key that asks, the condition or
//! order typed, the view applied, the panel that explains it, Save recording it
//! through arc, the folder reopened with the new step laid out and previewed,
//! and a Run that makes the step real. Charts read the load the whole way.
//!
//! The window is handed the `brightfield-shell` binary cargo built for this
//! suite as its runner, so the panel's sentences are arc's own `operation
//! describe`, the step Save writes is arc's own `operation record`, and the
//! Run is arc's own `run`. Every count is the engine's, asked over the same
//! housing file the window reads.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_engine::RowsAudience;
use brightfield_protocol::layout::Flow;
use brightfield_shell::data_grid::{
    use_own_preview_gate_on_this_thread, PREVIEW_COUNT, PREVIEW_LABEL, PREVIEW_ROWS,
};
use brightfield_shell::design::Mode;
use brightfield_shell::grid_foot::FootField;
use brightfield_shell::operation::{self, OperationDescription};
use brightfield_shell::pipeline::LiveDashboard;
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::run::{self, Runner};
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp, GRID_FOOT_STATUS_ID};
use brightfield_workbench::arrangement::INSPECTOR_RAIL;

/// The table the housing file's load makes.
const ROOT: &str = "california_housing";
const FILTER_STEP: &str = "california_housing_filtered_by_house_age";
const SORT_STEP: &str = "california_housing_sorted_by_median_income";

/// The housing file's columns a press puts the cursor in.
const MEDIAN_INCOME: usize = 0;
const HOUSE_AGE: usize = 1;

/// How long a run may take before the test calls it hung.
const PATIENCE: Duration = Duration::from_secs(180);

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-operation-from-the-cursor-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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

fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
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

fn described(long_name: &str) -> OperationDescription {
    operation::describe(&runner(), long_name).expect("arc describes the operation")
}

/// A headless window, over the housing file or over the folder it was saved
/// into.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    texts: Vec<(egui::Pos2, String)>,
}

impl Window {
    /// The window's documents read their previews behind a gate of this
    /// test's own, so neither test here waits behind the other's reads.
    fn over(boot: Boot, history: &Path) -> Self {
        use_own_preview_gate_on_this_thread();
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(runner()))
                .keeping_history(Some(brightfield_protocol::HistoryStore::At(
                    history.to_path_buf(),
                ))),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            texts: Vec::new(),
        };
        win.settle();
        win
    }

    /// A window over the housing file's bytes, written into `root`'s `data`
    /// folder, which is where Save puts the Protocol.
    fn housing(root: &TempDir) -> (Self, PathBuf) {
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(starts::HOUSING_FILE);
        std::fs::write(&data, starts::HOUSING_PARQUET).expect("the housing file writes");
        let boot = Boot::data_file(&data.to_string_lossy()).expect("the housing file opens");
        (Self::over(boot, &root.0.join("history")), folder)
    }

    /// The folder Save wrote, opened the way a front-door row or the command
    /// line opens it.
    fn reopen(root: &TempDir, folder: &Path) -> Self {
        let manifest = folder.join("arcform.yaml");
        let boot = Boot::open(&manifest.to_string_lossy(), Flow::Vertical, None)
            .expect("the folder opens");
        Self::over(boot, &root.0.join("history"))
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        let mut texts = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        self.texts = texts;
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    fn drew(&self, text: &str) -> bool {
        self.texts.iter().any(|(_, t)| t.contains(text))
    }

    fn key(&mut self, key: egui::Key) {
        self.run(vec![press(key, egui::Modifiers::NONE)]);
        self.settle();
    }

    /// Press `key` as a keyboard does, with the text the keystroke makes.
    fn keystroke(&mut self, key: egui::Key, text: &str) {
        self.run(vec![
            press(key, egui::Modifiers::NONE),
            egui::Event::Text(text.to_owned()),
        ]);
        self.settle();
    }

    /// Type `text` as a keyboard does: the text a keystroke makes.
    fn type_text(&mut self, text: &str) {
        self.run(vec![egui::Event::Text(text.to_owned())]);
        self.settle();
    }

    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(at, true), button(at, false)]);
        self.settle();
    }

    /// Press the cell of the drawn grid at `row`, `col`.
    fn press_cell(&mut self, row: u64, col: usize) {
        let drawn = self
            .app
            .chart_doc()
            .grid_drawn()
            .expect("the grid drew a table")
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
        self.click(egui::pos2(
            head.intersect(*head_clip).center().x,
            line.intersect(*line_clip).center().y,
        ));
    }

    /// The value the grid drew in the cursor's cell.
    fn cursor_value(&self) -> String {
        self.app
            .chart_doc()
            .grid_drawn()
            .and_then(|d| d.cursor.as_ref())
            .and_then(|c| c.value.clone())
            .expect("the cursor's cell drew a value")
    }

    /// Put the cursor on the first cell of `col` whose value reads `value`,
    /// moving down from the top a row at a time with `j`, as an analyst does.
    fn cursor_to(&mut self, col: usize, value: &str) {
        self.press_cell(0, col);
        let mut seen = Vec::new();
        for _ in 0..200 {
            let at = self.cursor_value();
            if at == value {
                return;
            }
            seen.push(at);
            self.key(egui::Key::J);
        }
        panic!("no cell of column {col} near the top reads {value}: {seen:?}");
    }

    /// The rows the grid drew a count for.
    fn grid_rows(&self) -> u64 {
        self.app
            .chart_doc()
            .grid_drawn()
            .expect("the grid drew a table")
            .rows
    }

    /// The engine's count of the loaded table's rows under `condition`, asked
    /// through the window's own session.
    fn engine_count(&mut self, condition: &str) -> u64 {
        let doc = self.app.chart_doc_mut();
        let mark = doc.live_dashboard().map_or(0, LiveDashboard::rows_mark);
        let session = doc.live_coordinator().expect("a live session").session();
        session
            .step_rows_count_where(mark, RowsAudience::Reader, condition)
            .expect("the engine counts the condition")
    }

    fn asking(&self) -> Option<(FootField, String)> {
        self.app
            .chart_doc()
            .foot_asking()
            .map(|(field, text)| (field, text.to_owned()))
    }

    /// The status band's line for the band, as the frame drew it.
    fn foot_status(&self) -> Option<String> {
        if !self.app.rail().drawn.contains(&GRID_FOOT_STATUS_ID) {
            return None;
        }
        self.app.chart_doc().foot_said()
    }

    /// The text the Inspector rail drew on the last frame.
    fn rail_text(&self) -> Vec<String> {
        let Some(rect) = self.app.region_rect(INSPECTOR_RAIL) else {
            return Vec::new();
        };
        self.texts
            .iter()
            .filter(|(pos, _)| rect.contains(*pos + egui::vec2(1.0, 1.0)))
            .map(|(_, t)| t.clone())
            .collect()
    }

    /// Draw frames until the rail draws `text`, or give up after a while: the
    /// description is asked on a worker, so it lands a few frames late.
    fn until_rail_draws(&mut self, text: &str) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            self.run(Vec::new());
            let drawn = self.rail_text();
            if drawn.iter().any(|t| t == text) || Instant::now() > deadline {
                return drawn;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// **Save, through the gesture a person has**: the palette on `space`,
    /// the verb typed, confirmed with Enter.
    fn save(&mut self) {
        assert!(self.app.has_protocol_to_save(), "nothing to save");
        self.key(egui::Key::Space);
        assert_eq!(self.app.open_overlay(), Some("palette"));
        self.settle();
        self.run(vec![egui::Event::Text("save-spec".to_owned())]);
        self.run(Vec::new());
        self.key(egui::Key::Enter);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "confirming save-spec did not close the palette"
        );
        self.settle();
    }

    /// **Run, through its binding**: cmd-enter, then frames until arc's run
    /// lands.
    fn run_protocol(&mut self) {
        let command = egui::Modifiers {
            command: true,
            ..Default::default()
        };
        self.run(vec![press(egui::Key::Enter, command)]);
        assert!(self.app.run_in_progress(), "cmd-enter did not start a run");
        let deadline = Instant::now() + PATIENCE;
        while self.app.run_outstanding() {
            assert!(Instant::now() < deadline, "the run did not land");
            self.run(Vec::new());
            std::thread::sleep(Duration::from_millis(5));
        }
        self.settle();
    }

    /// The names of the steps the Steps sheet lists, in order.
    fn sheet_steps(&self) -> Vec<String> {
        self.app
            .protocol_model()
            .sheet()
            .rows()
            .iter()
            .map(|r| r.name.clone())
            .collect()
    }

    /// The status the Steps sheet gives the step `name`.
    fn status_of(&self, name: &str) -> String {
        self.app
            .protocol_model()
            .sheet()
            .rows()
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("the sheet lists no step {name}"))
            .status
            .to_string()
    }

    /// Click the `grid` row the rail lists under the asset row `node`, then
    /// draw frames until the preview it shows has been read.
    fn click_grid_under(&mut self, node: &str) {
        let rows = self.app.spine_rows().to_vec();
        let at = rows
            .iter()
            .position(|row| row.role == SpineRole::Asset && row.label == node)
            .unwrap_or_else(|| {
                let drawn: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
                panic!("the rail drew no asset row labelled {node:?}: {drawn:?}")
            });
        let grid = rows
            .get(at + 1)
            .filter(|row| row.role == SpineRole::View && row.label == "grid")
            .unwrap_or_else(|| panic!("no grid row stands under {node:?}"));
        self.click(grid.rect.center());
        let deadline = Instant::now() + Duration::from_secs(60);
        while self
            .app
            .chart_doc()
            .grid_preview()
            .is_some_and(|p| p.is_reading())
        {
            assert!(Instant::now() < deadline, "the preview never arrived");
            std::thread::sleep(Duration::from_millis(5));
            self.run(Vec::new());
        }
        self.settle();
    }

    /// The text of each cell of `column` on the preview page.
    fn preview_cells(&self, column: &str) -> Vec<String> {
        let page = self
            .app
            .chart_doc()
            .grid_preview()
            .and_then(|p| p.page())
            .expect("the grid holds a preview page");
        let at = page
            .columns
            .iter()
            .position(|c| c.name == column)
            .unwrap_or_else(|| panic!("the preview has no column {column}"));
        page.rows.iter().map(|row| row[at].text.clone()).collect()
    }

    /// The data sources the chart is drawn from, each with what it reads.
    fn chart_sources(&self) -> Vec<(String, String)> {
        self.app
            .chart_doc()
            .live_spec()
            .expect("a live chart")
            .data
            .iter()
            .map(|(name, source)| (name.clone(), format!("{:?}", source.kind)))
            .collect()
    }
}

fn manifest_steps(folder: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(folder.join("arcform.yaml")).expect("the manifest reads");
    brightfield_protocol::parse_manifest_str(&text)
        .expect("the manifest parses")
        .steps
        .into_iter()
        .map(|s| s.name)
        .collect()
}

/// The model of the step `name` in the folder's manifest.
fn model(folder: &Path, name: &str) -> String {
    let text = std::fs::read_to_string(folder.join("arcform.yaml")).expect("the manifest reads");
    let manifest = brightfield_protocol::parse_manifest_str(&text).expect("the manifest parses");
    let step = manifest
        .steps
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no step {name} in the manifest"));
    std::fs::read_to_string(folder.join(step.sql.as_ref().expect("a sql step")))
        .expect("the model reads")
}

/// The chart the Save wrote reads the load's table, and no generated step.
fn saved_chart_reads_the_load(folder: &Path) {
    let chart = std::fs::read_to_string(folder.join("panels").join(format!("{ROOT}.yaml")))
        .expect("the chart file was written");
    assert!(chart.contains(ROOT), "{chart}");
    assert!(
        !chart.contains(FILTER_STEP) && !chart.contains(SORT_STEP),
        "the chart reads a generated step: {chart}"
    );
}

fn has(drawn: &[String], text: &str) -> bool {
    drawn.iter().any(|t| t == text)
}

/// **A filter from the cursor, from the cell to a run of the step it
/// recorded.**
#[test]
fn filter_from_the_cursor_end_to_end() {
    let root = TempDir::new("filter");
    let (mut win, folder) = Window::housing(&root);
    let charts = win.chart_sources();
    assert!(
        charts.len() == 1 && charts[0].1.contains(starts::HOUSING_FILE),
        "the chart does not read the load's file: {charts:?}"
    );
    let total = win.grid_rows();
    assert_eq!(total, win.engine_count("true"));

    // 1. The cursor on a `house_age` cell holding 28 (a double, drawn `28.0`).
    win.cursor_to(HOUSE_AGE, "28.0");

    // 2. The key asks `where` from that cell, and the panel lists the table,
    //    the column and `where` in arc's words.
    win.keystroke(egui::Key::W, "w");
    let (field, asked) = win.asking().expect("the key opened a field");
    assert_eq!(field, FootField::Where);
    assert_eq!(asked, "house_age = 28");
    assert_eq!(win.engine_count(&asked), 375, "{asked}");
    let arc = described("filter-rows");
    let where_ = arc
        .parameters
        .iter()
        .find(|p| p.key == "where")
        .expect("arc's filter-rows takes `where`");
    let drawn = win.until_rail_draws(&where_.description);
    for expected in [
        "filter-rows",
        ROOT,
        "house_age",
        "where",
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

    // 3. `>` swaps the comparison, and the grid follows: 8,831 rows.
    win.type_text(">");
    let (_, condition) = win.asking().expect("the field still asks");
    assert_eq!(condition, "house_age > 28");
    let kept = win.engine_count(&condition);
    assert_eq!(kept, 8_831);
    assert_eq!(win.grid_rows(), kept);
    assert_eq!(
        win.app
            .chart_doc()
            .foot_drawn()
            .expect("the band drew")
            .count,
        "8,831 rows \u{b7} 9 columns"
    );
    assert!(!win.app.carries_unsaved_mark(), "marked before Enter");

    // 4. Enter applies the view: the dot, and the window's unsaved mark.
    win.key(egui::Key::Enter);
    assert_eq!(win.asking(), None);
    assert_eq!(
        win.app.chart_doc().grid_view().condition.as_deref(),
        Some(condition.as_str())
    );
    assert!(
        win.app
            .chart_doc()
            .foot_drawn()
            .expect("the band drew")
            .where_dot,
        "the applied view drew no not-recorded dot"
    );
    assert!(win.app.carries_unsaved_mark());
    assert_eq!(win.grid_rows(), kept);
    assert_eq!(win.chart_sources(), charts, "the view moved the chart");

    // 5. Save records the step through arc, and the window shows two steps.
    win.save();
    assert!(!win.app.carries_unsaved_mark(), "the Save left the mark on");
    assert_eq!(
        manifest_steps(&folder),
        vec!["load".to_string(), FILTER_STEP.to_string()]
    );
    let recorded = model(&folder, FILTER_STEP);
    assert!(
        recorded.starts_with(&format!("-- generated: filter-rows on {ROOT}")),
        "{recorded}"
    );
    assert!(recorded.contains(&condition), "{recorded}");
    assert_eq!(win.app.protocol_model().sheet().len(), 2);
    assert_eq!(win.chart_sources(), charts, "the Save moved the chart");
    saved_chart_reads_the_load(&folder);
    drop(win);

    // 6. Reopened: two steps, the second not run, and its grid a labelled
    //    first page of rows the condition keeps, with no count. The key there
    //    offers Run.
    let mut win = Window::reopen(&root, &folder);
    assert_eq!(
        win.sheet_steps(),
        vec!["load".to_string(), FILTER_STEP.to_string()]
    );
    assert_eq!(win.status_of(FILTER_STEP), "not run");
    win.click_grid_under(FILTER_STEP);
    assert!(win.drew(PREVIEW_LABEL), "the preview label was not drawn");
    let ages = win.preview_cells("house_age");
    assert_eq!(ages.len() as u64, PREVIEW_ROWS);
    assert!(
        ages.iter()
            .all(|a| a.parse::<f64>().expect("a number") > 28.0),
        "a previewed row fails the step's condition: {ages:?}"
    );
    assert_eq!(
        win.app
            .chart_doc()
            .foot_drawn()
            .expect("the band drew")
            .count,
        PREVIEW_COUNT
    );
    assert!(!win.drew("8,831"), "the preview claims the step's count");
    win.press_cell(0, HOUSE_AGE);
    win.keystroke(egui::Key::W, "w");
    assert_eq!(win.asking(), None, "the key asked on a generated step");
    let said = win.foot_status().expect("the status band says why");
    assert!(said.contains("Run"), "{said}");
    assert!(said.contains("not been run"), "{said}");

    // 7. Run: the run succeeds, and the step reads as run.
    win.run_protocol();
    let record = run::records_newest_first(&folder, ROOT)
        .next()
        .expect("the run wrote a record");
    assert_eq!(
        record.run.outcome,
        brightfield_protocol::contract::Outcome::Success,
        "the run did not succeed"
    );
    assert_eq!(win.status_of(FILTER_STEP), "ok");

    // 8. Charts drew from the load throughout.
    assert_eq!(
        manifest_steps(&folder),
        vec!["load".to_string(), FILTER_STEP.to_string()]
    );
    saved_chart_reads_the_load(&folder);
}

/// **An order from the cursor, the same way**: `o` on the column asks `order
/// by` it, and the step Save records is `sort-rows`.
#[test]
fn order_from_the_cursor_end_to_end() {
    let order = "median_income DESC";
    let root = TempDir::new("order");
    let (mut win, folder) = Window::housing(&root);
    let charts = win.chart_sources();
    let total = win.grid_rows();
    assert_eq!(total, win.engine_count("true"));

    // 1–2. The cursor in `median_income`, and `o` asks `order by` it, with the
    //      panel listing the table, the column and `order_by`.
    win.press_cell(0, MEDIAN_INCOME);
    win.key(egui::Key::O);
    assert_eq!(
        win.asking(),
        Some((FootField::OrderBy, "median_income".to_owned()))
    );
    let arc = described("sort-rows");
    let order_by = arc
        .parameters
        .iter()
        .find(|p| p.key == "order_by")
        .expect("arc's sort-rows takes `order_by`");
    let drawn = win.until_rail_draws(&order_by.description);
    for expected in [
        "sort-rows",
        ROOT,
        "median_income",
        "order_by",
        order_by.description.as_str(),
    ] {
        assert!(
            has(&drawn, expected),
            "the panel did not draw {expected:?}: {drawn:?}"
        );
    }

    // 3. ` DESC` typed after the column, and the grid follows with every row.
    win.type_text(" DESC");
    assert_eq!(win.asking(), Some((FootField::OrderBy, order.to_owned())));
    assert_eq!(win.grid_rows(), total);
    assert!(!win.app.carries_unsaved_mark(), "marked before Enter");

    // 4. Enter applies the view: the dot, the unsaved mark, and the highest
    //    income first.
    win.key(egui::Key::Enter);
    assert_eq!(win.asking(), None);
    assert_eq!(
        win.app.chart_doc().grid_view().order.as_deref(),
        Some(order)
    );
    assert!(
        win.app
            .chart_doc()
            .foot_drawn()
            .expect("the band drew")
            .order_dot,
        "the applied order drew no not-recorded dot"
    );
    assert!(win.app.carries_unsaved_mark());
    assert_eq!(win.grid_rows(), total);
    let mut incomes = Vec::new();
    for row in 0..5 {
        win.press_cell(row, MEDIAN_INCOME);
        incomes.push(win.cursor_value().parse::<f64>().expect("a number"));
    }
    assert!(
        incomes.windows(2).all(|w| w[0] >= w[1]),
        "the grid is not in the order applied: {incomes:?}"
    );
    assert_eq!(
        win.engine_count(&format!("median_income > {}", incomes[0])),
        0,
        "the first row is not the highest income"
    );
    assert_eq!(win.chart_sources(), charts, "the view moved the chart");

    // 5. Save records `sort-rows` through arc, on the load's table.
    win.save();
    assert!(!win.app.carries_unsaved_mark(), "the Save left the mark on");
    assert_eq!(
        manifest_steps(&folder),
        vec!["load".to_string(), SORT_STEP.to_string()]
    );
    let recorded = model(&folder, SORT_STEP);
    assert!(
        recorded.starts_with(&format!("-- generated: sort-rows on {ROOT}")),
        "{recorded}"
    );
    assert!(recorded.contains(order), "{recorded}");
    assert_eq!(win.app.protocol_model().sheet().len(), 2);
    assert_eq!(win.chart_sources(), charts, "the Save moved the chart");
    saved_chart_reads_the_load(&folder);
    drop(win);

    // 6. Reopened: the sort not run, and its grid a labelled first page in the
    //    step's order, with no count. The key there offers Run.
    let mut win = Window::reopen(&root, &folder);
    assert_eq!(
        win.sheet_steps(),
        vec!["load".to_string(), SORT_STEP.to_string()]
    );
    assert_eq!(win.status_of(SORT_STEP), "not run");
    win.click_grid_under(SORT_STEP);
    assert!(win.drew(PREVIEW_LABEL), "the preview label was not drawn");
    let previewed: Vec<f64> = win
        .preview_cells("median_income")
        .iter()
        .map(|v| v.parse().expect("a number"))
        .collect();
    assert_eq!(previewed.len() as u64, PREVIEW_ROWS);
    assert_eq!(previewed[0], incomes[0], "the preview starts elsewhere");
    assert!(
        previewed.windows(2).all(|w| w[0] >= w[1]),
        "the preview is not in the step's order: {previewed:?}"
    );
    assert_eq!(
        win.app
            .chart_doc()
            .foot_drawn()
            .expect("the band drew")
            .count,
        PREVIEW_COUNT
    );
    win.press_cell(0, MEDIAN_INCOME);
    win.key(egui::Key::O);
    assert_eq!(win.asking(), None, "the key asked on a generated step");
    let said = win.foot_status().expect("the status band says why");
    assert!(said.contains("Run"), "{said}");
    assert!(said.contains("not been run"), "{said}");

    // 7. Run: the run succeeds, and the step reads as run.
    win.run_protocol();
    let record = run::records_newest_first(&folder, ROOT)
        .next()
        .expect("the run wrote a record");
    assert_eq!(
        record.run.outcome,
        brightfield_protocol::contract::Outcome::Success,
        "the run did not succeed"
    );
    assert_eq!(win.status_of(SORT_STEP), "ok");

    // 8. Charts drew from the load throughout.
    saved_chart_reads_the_load(&folder);
}
