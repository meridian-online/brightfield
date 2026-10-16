//! A folder holding a data-file load and steps arc generated reopens as the
//! Protocol it is: every step laid out, none run, and no run started.
//!
//! A generated step nobody has run has no table, so its grid shows a first
//! page brightfield computes from the data file on a connection of its own,
//! labelled as a preview and claiming no count. The band under it is read-only
//! and `w` there offers Run, because filtering a step starts from a run of it.
//!
//! Each folder is made the way an analyst makes one: a window over the
//! housing file applies a view and Saves, which records it through arc (the
//! `brightfield-shell` binary cargo built for this suite is the runner). The
//! folder is then reopened through [`Boot::open`] on its manifest, the route a
//! front-door row and the command line take.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_engine::RowsView;
use brightfield_protocol::layout::Flow;
use brightfield_shell::data_grid::{
    use_own_preview_gate_on_this_thread, PREVIEW_COUNT, PREVIEW_LABEL, PREVIEW_READING,
    PREVIEW_ROWS,
};
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::{SpineRole, OFFLINE_VAR};
use brightfield_shell::run::Runner;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp, GRID_FOOT_STATUS_ID};
use brightfield_workbench::arrangement::LEDGER_RAIL;

const FILTER_STEP: &str = "california_housing_filtered_by_house_age";
const SORT_STEP: &str = "california_housing_sorted_by_median_income";

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
            "bf-generated-reopen-{name}-{}-{}",
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

fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, out);
            }
        }
        _ => {}
    }
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

/// A headless window.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    texts: Vec<String>,
    /// Whether any frame since the last click drew the reading state.
    saw_reading: bool,
}

impl Window {
    /// The window's documents read their previews behind a gate of this
    /// test's own, so no test here waits behind another's reads.
    fn over(boot: Boot) -> Self {
        use_own_preview_gate_on_this_thread();
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(runner())),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            texts: Vec::new(),
            saw_reading: false,
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
        let mut texts = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut texts);
        }
        self.texts = texts;
        self.saw_reading |= self.drew(PREVIEW_READING);
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    fn drew(&self, text: &str) -> bool {
        self.texts.iter().any(|t| t.contains(text))
    }

    /// Click the `grid` row listed under the asset row labelled `node`.
    fn click_grid_only(&mut self, node: &str) {
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
        let centre = grid.rect.center();
        self.saw_reading = false;
        self.run(vec![egui::Event::PointerMoved(centre)]);
        self.run(vec![button(centre, true), button(centre, false)]);
        self.settle();
    }

    /// [`Self::click_grid_only`], then wait for the preview it shows.
    fn click_grid_under(&mut self, node: &str) {
        self.click_grid_only(node);
        self.wait_for_preview();
    }

    /// Draw frames until the preview the grid shows is no longer being read.
    fn wait_for_preview(&mut self) {
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
            .expect("the column drew a header cell");
        let (_, line, line_clip) = drawn
            .row_cells
            .iter()
            .find(|(r, ..)| *r == row)
            .expect("the row was drawn");
        let at = egui::pos2(
            head.intersect(*head_clip).center().x,
            line.intersect(*line_clip).center().y,
        );
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(at, true), button(at, false)]);
        self.settle();
    }

    fn key(&mut self, key: egui::Key, text: &str) {
        self.run(vec![press(key), egui::Event::Text(text.to_owned())]);
        self.settle();
    }

    /// The status band's line for the band, as the frame drew it.
    fn foot_status(&self) -> Option<String> {
        if !self.app.rail().drawn.contains(&GRID_FOOT_STATUS_ID) {
            return None;
        }
        self.app.chart_doc().foot_said()
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

    /// The value of each cell of `column` on the preview page, in full rather
    /// than rounded as the cell shows it.
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
        page.rows
            .iter()
            .map(|row| row[at].full().to_owned())
            .collect()
    }
}

/// Holds the preview workers of the document it was taken from before they
/// read, until it is released or dropped — so a test that fails while holding
/// them leaves no worker waiting.
struct Hold(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Hold {
    fn on(win: &Window) -> Self {
        let held = win.app.chart_doc().preview_hold();
        held.store(true, std::sync::atomic::Ordering::SeqCst);
        Self(held)
    }

    fn release(&self) {
        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.release();
    }
}

/// A folder over the housing file in which a window applied `condition` and
/// `order` as the grid's view and Saved, so arc recorded each as a step.
fn saved_folder(root: &TempDir, condition: Option<&str>, order: Option<&str>) -> PathBuf {
    let folder = root.0.join("data");
    std::fs::create_dir_all(&folder).expect("the data file's folder");
    let data = folder.join(starts::HOUSING_FILE);
    std::fs::write(&data, starts::HOUSING_PARQUET).expect("the housing file writes");
    let boot = Boot::data_file(&data.to_string_lossy()).expect("the housing file opens");
    let mut win = Window::over(boot);
    win.app.chart_doc_mut().apply_grid_view(RowsView {
        condition: condition.map(str::to_owned),
        order: order.map(str::to_owned),
    });
    win.settle();
    win.app
        .save_protocol(&win.ctx)
        .expect("the window has a Protocol to save")
        .expect("the Save records");
    folder
}

fn manifest(folder: &Path) -> String {
    folder.join("arcform.yaml").to_string_lossy().into_owned()
}

fn reopen(folder: &Path) -> Window {
    let boot = Boot::open(&manifest(folder), Flow::Vertical, None).expect("the folder opens");
    Window::over(boot)
}

/// Every file under `dir`, recursively; none when it is not there.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(files_under(&path));
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

#[test]
fn a_load_plus_generated_folder_opens_as_a_protocol_without_the_offline_variable() {
    assert!(
        std::env::var(OFFLINE_VAR).is_err(),
        "the suite must run without {OFFLINE_VAR}"
    );
    let root = TempDir::new("opens");
    let folder = saved_folder(&root, Some("house_age > 28"), None);

    let win = reopen(&folder);

    let names: Vec<String> = win
        .app
        .protocol_model()
        .sheet()
        .rows()
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(names, vec!["load".to_string(), FILTER_STEP.to_string()]);
    assert!(
        !win.app.rail_is_collapsed(LEDGER_RAIL),
        "the ledger rail is shut"
    );
    assert_eq!(win.status_of(FILTER_STEP), "not run");
}

#[test]
fn opening_starts_no_run() {
    let root = TempDir::new("no-run");
    let folder = saved_folder(&root, Some("house_age > 28"), Some("median_income DESC"));
    let runs = folder.join("build").join(".arcform").join("runs");
    let before = files_under(&runs);

    let mut win = reopen(&folder);
    win.click_grid_under(SORT_STEP);
    win.settle();

    assert!(!win.app.run_outstanding(), "opening started a run");
    assert_eq!(files_under(&runs), before, "opening wrote a run record");
}

#[test]
fn a_never_run_steps_grid_shows_a_labelled_first_page() {
    let root = TempDir::new("preview");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let mut win = reopen(&folder);

    win.click_grid_under(FILTER_STEP);

    assert!(win.saw_reading, "no frame said the preview was being read");
    assert!(win.drew(PREVIEW_LABEL), "the preview label was not drawn");
    let ages = win.preview_cells("house_age");
    assert_eq!(ages.len() as u64, PREVIEW_ROWS);
    assert!(
        ages.iter()
            .all(|a| a.parse::<f64>().expect("a number") > 28.0),
        "a previewed row fails the step's condition: {ages:?}"
    );
    let band = win.app.chart_doc().foot_drawn().expect("the band drew");
    assert_eq!(band.count, PREVIEW_COUNT);
    assert!(
        !win.drew("8,831"),
        "the preview claims the step's total count"
    );
    let drawn = win.app.chart_doc().grid_drawn().expect("the grid drew");
    assert_eq!(
        drawn.rows, PREVIEW_ROWS,
        "the grid scrolls past the first page"
    );
    assert_eq!(win.status_of(FILTER_STEP), "not run");

    // The cursor's line places the row on the first page and counts nothing.
    win.press_cell(0, 1);
    let row_lines: Vec<&String> = win.texts.iter().filter(|t| t.starts_with("row ")).collect();
    assert!(
        row_lines
            .iter()
            .any(|t| t.starts_with("row 1 of the first page \u{b7} preview")),
        "{row_lines:?}"
    );
    assert!(
        !row_lines
            .iter()
            .any(|t| t.contains(&PREVIEW_ROWS.to_string())),
        "the cursor's line claims a count: {row_lines:?}"
    );
}

#[test]
fn a_return_to_a_steps_grid_reads_nothing_again() {
    let root = TempDir::new("return");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let mut win = reopen(&folder);
    win.click_grid_under(FILTER_STEP);
    assert_eq!(win.app.chart_doc().preview_reads_started(), 1);

    win.click_grid_under("california_housing");
    assert!(win.app.chart_doc().grid_preview().is_none());
    win.click_grid_under(FILTER_STEP);

    assert_eq!(
        win.app.chart_doc().preview_reads_started(),
        1,
        "the return read the file again"
    );
    assert!(!win.saw_reading, "the return drew the reading state");
    assert!(win.drew(PREVIEW_LABEL));
}

#[test]
fn a_name_with_capitals_previews_and_is_selected_after_save() {
    let root = TempDir::new("capitals");
    let folder = root.0.join("data");
    std::fs::create_dir_all(&folder).expect("the data file's folder");
    let data = folder.join("Housing.parquet");
    std::fs::write(&data, starts::HOUSING_PARQUET).expect("the housing file writes");
    let boot = Boot::data_file(&data.to_string_lossy()).expect("the housing file opens");
    let mut win = Window::over(boot);
    win.app.chart_doc_mut().apply_grid_view(RowsView {
        condition: Some("house_age > 28".to_owned()),
        order: None,
    });
    win.settle();
    win.app
        .save_protocol(&win.ctx)
        .expect("the window has a Protocol to save")
        .expect("the Save records");
    win.settle();
    let step = "Housing_filtered_by_house_age";
    assert!(
        win.app
            .protocol_model()
            .selected()
            .is_some_and(|id| id.to_lowercase().contains(&step.to_lowercase())),
        "the new step is not selected: {:?}",
        win.app.protocol_model().selected()
    );

    let mut win = reopen(&folder);
    win.click_grid_under(&step.to_lowercase());
    assert!(win.drew(PREVIEW_LABEL), "the preview label was not drawn");
}

#[test]
fn a_manifest_outside_the_data_files_folder_is_not_opened_this_way() {
    let root = TempDir::new("elsewhere");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    // The data file moves into a folder of its own, and the manifest and the
    // load name it there.
    let inner = folder.join("data");
    std::fs::create_dir_all(&inner).expect("the inner folder");
    std::fs::rename(
        folder.join(starts::HOUSING_FILE),
        inner.join(starts::HOUSING_FILE),
    )
    .expect("the data file moves");
    let spelled = format!("./{}", starts::HOUSING_FILE);
    let moved = format!("data/{}", starts::HOUSING_FILE);
    for file in [
        folder.join("arcform.yaml"),
        folder.join("models").join("load.sql"),
    ] {
        let text = std::fs::read_to_string(&file).expect("the file reads");
        assert!(text.contains(&spelled), "{text}");
        std::fs::write(&file, text.replace(&spelled, &moved)).expect("the file writes");
    }

    let refused = Boot::open(&manifest(&folder), Flow::Vertical, None)
        .err()
        .expect("a manifest away from its data file opened through it");

    assert!(refused.contains(OFFLINE_VAR), "{refused}");
}

#[test]
fn a_sorted_steps_preview_is_in_its_order() {
    let root = TempDir::new("sorted");
    let folder = saved_folder(&root, Some("house_age > 28"), Some("median_income DESC"));
    let mut win = reopen(&folder);

    win.click_grid_under(SORT_STEP);

    let income: Vec<f64> = win
        .preview_cells("median_income")
        .iter()
        .map(|v| v.parse().expect("a number"))
        .collect();
    assert!(!income.is_empty());
    assert!(
        income.windows(2).all(|w| w[0] >= w[1]),
        "the preview is not in the step's order: {income:?}"
    );
    let ages = win.preview_cells("house_age");
    assert!(
        ages.iter()
            .all(|a| a.parse::<f64>().expect("a number") > 28.0),
        "the sort's preview skipped the filter it reads: {ages:?}"
    );
}

#[test]
fn a_hand_edited_generated_model_is_refused_naming_the_step() {
    let root = TempDir::new("edited");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let models = folder.join("models");
    let model = std::fs::read_dir(&models)
        .expect("the models folder")
        .flatten()
        .map(|e| e.path())
        .find(|p| std::fs::read_to_string(p).is_ok_and(|t| t.starts_with("-- generated:")))
        .expect("a generated model");
    let text = std::fs::read_to_string(&model).expect("the model reads");
    std::fs::write(&model, text.replace("SELECT *", "SELECT house_age"))
        .expect("the model is edited");

    let refused = Boot::open(&manifest(&folder), Flow::Vertical, None)
        .err()
        .expect("a hand-edited generated model opened");

    assert!(refused.contains(FILTER_STEP), "{refused}");
}

#[test]
fn w_on_a_generated_step_offers_run() {
    let root = TempDir::new("w");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let mut win = reopen(&folder);
    win.click_grid_under(FILTER_STEP);
    win.press_cell(0, 1);

    win.key(egui::Key::W, "w");

    assert_eq!(win.app.chart_doc().foot_asking(), None);
    let said = win.foot_status().expect("the status band says why");
    assert!(said.contains("Run"), "{said}");
    assert!(said.contains("not been run"), "{said}");
}

#[test]
fn after_a_run_the_step_reads_as_run_and_still_shows_the_labelled_preview() {
    let root = TempDir::new("ran");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let mut win = reopen(&folder);

    assert!(win.app.run_protocol(&win.ctx), "the run did not start");
    let deadline = Instant::now() + PATIENCE;
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run did not land");
        win.run(Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    win.settle();

    assert_eq!(win.app.protocol_model().sheet().len(), 2);
    assert_eq!(win.status_of(FILTER_STEP), "ok");
    win.click_grid_under(FILTER_STEP);
    assert!(win.drew(PREVIEW_LABEL), "the preview label was not drawn");
    win.press_cell(0, 1);
    win.key(egui::Key::W, "w");
    let said = win.foot_status().expect("the status band says why");
    assert!(said.contains("later version"), "{said}");
}

#[test]
fn a_step_over_the_limit_says_why_and_offers_run() {
    let root = TempDir::new("too-large");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let mut win = reopen(&folder);
    win.app.chart_doc_mut().set_preview_file_limit(1);

    win.click_grid_under(FILTER_STEP);

    assert!(win.drew("too large to preview"), "{:?}", win.texts);
    assert!(
        win.texts.iter().any(|t| t.contains("Run")),
        "{:?}",
        win.texts
    );
    assert!(!win.drew(PREVIEW_LABEL));
    assert_eq!(win.status_of(FILTER_STEP), "not run");
}

#[test]
fn one_preview_worker_runs_at_a_time_and_a_step_is_read_once() {
    let root = TempDir::new("one-worker");
    let folder = saved_folder(&root, Some("house_age > 28"), Some("median_income DESC"));
    let mut win = reopen(&folder);
    let hold = Hold::on(&win);

    // A, then B while A is read, then back to A before it lands.
    win.click_grid_only(FILTER_STEP);
    win.click_grid_only(SORT_STEP);
    win.click_grid_only(FILTER_STEP);
    assert_eq!(
        win.app.chart_doc().preview_reads_started(),
        1,
        "a read started while another was under way"
    );

    hold.release();
    win.wait_for_preview();
    assert!(win.drew(PREVIEW_LABEL), "the filter's preview did not land");
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(5));
        win.run(Vec::new());
    }
    assert_eq!(
        win.app.chart_doc().preview_reads_started(),
        1,
        "B was read though nobody went back to it"
    );

    win.click_grid_under(SORT_STEP);
    assert!(win.drew(PREVIEW_LABEL), "the sort's preview did not land");
    win.click_grid_under(FILTER_STEP);

    let doc = win.app.chart_doc();
    assert_eq!(
        doc.preview_reads_started(),
        2,
        "a step was read more than once"
    );
    assert_eq!(doc.preview_workers_most_live(), 1);
}

#[test]
fn a_data_file_rewritten_while_its_steps_grid_is_open_is_read_again() {
    let root = TempDir::new("rewritten");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let mut win = reopen(&folder);
    win.click_grid_under(FILTER_STEP);
    assert_eq!(win.app.chart_doc().preview_reads_started(), 1);

    let data = folder.join(starts::HOUSING_FILE);
    let later = std::fs::metadata(&data)
        .and_then(|m| m.modified())
        .expect("the file's time")
        + Duration::from_secs(60);
    std::fs::File::options()
        .write(true)
        .open(&data)
        .and_then(|f| f.set_modified(later))
        .expect("the file's time is moved on");
    win.run(Vec::new());
    win.wait_for_preview();

    assert_eq!(
        win.app.chart_doc().preview_reads_started(),
        2,
        "the rewritten file kept the old page"
    );
    assert!(win.drew(PREVIEW_LABEL));
}

#[test]
fn a_read_that_ends_without_an_answer_is_read_again_on_the_next_visit() {
    let root = TempDir::new("lost");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let mut win = reopen(&folder);
    win.app.chart_doc().fail_next_preview_read();

    win.click_grid_under(FILTER_STEP);
    assert!(win.drew("stopped without an answer"), "{:?}", win.texts);

    win.click_grid_under("california_housing");
    win.click_grid_under(FILTER_STEP);

    assert!(win.drew(PREVIEW_LABEL), "the next visit did not read again");
    assert_eq!(win.app.chart_doc().preview_reads_started(), 2);
}

#[test]
fn a_read_lost_while_nobody_waits_for_it_is_not_reported_on_the_next_visit() {
    let root = TempDir::new("lost-away");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let mut win = reopen(&folder);
    let hold = Hold::on(&win);
    win.app.chart_doc().fail_next_preview_read();
    win.click_grid_only(FILTER_STEP);

    // Away to the load's grid, and the worker ends there without an answer.
    win.click_grid_only("california_housing");
    hold.release();
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(5));
        win.run(Vec::new());
    }
    win.click_grid_under(FILTER_STEP);

    assert!(
        !win.drew("stopped without an answer"),
        "a lost read leaked into the next visit"
    );
    assert!(win.drew(PREVIEW_LABEL), "the next visit did not read again");
    assert_eq!(win.app.chart_doc().preview_reads_started(), 2);
}

#[test]
fn a_document_opened_over_a_read_still_under_way_waits_for_it() {
    let root = TempDir::new("replaced");
    let first = saved_folder(&root, Some("house_age > 28"), None);
    let other = TempDir::new("replacing");
    let second = saved_folder(&other, Some("house_age > 28"), None);
    let mut win = reopen(&first);
    let hold = Hold::on(&win);
    win.click_grid_only(FILTER_STEP);
    let deadline = Instant::now() + Duration::from_secs(60);
    while win.app.chart_doc().preview_gate_live() == 0 {
        assert!(Instant::now() < deadline, "the first read never began");
        std::thread::sleep(Duration::from_millis(5));
    }

    // The document goes away with its worker still reading, and a document
    // of its own, opened after it, asks for a read.
    drop(win);
    let mut win = reopen(&second);
    win.click_grid_only(FILTER_STEP);
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(5));
        win.run(Vec::new());
        assert!(
            win.app.chart_doc().preview_gate_live() <= 1,
            "two preview reads ran at once"
        );
    }
    assert_eq!(win.app.chart_doc().preview_reads_started(), 1);
    assert!(
        win.app
            .chart_doc()
            .grid_preview()
            .is_some_and(|p| p.is_reading()),
        "the new document's read did not wait for the replaced one's"
    );
    assert!(
        win.drew(PREVIEW_READING),
        "the waiting read drew no reading state"
    );

    hold.release();
    win.wait_for_preview();

    assert!(
        win.drew(PREVIEW_LABEL),
        "the new document's preview did not land"
    );
    assert_eq!(win.app.chart_doc().preview_gate_most_live(), 1);
}

#[test]
fn w_on_a_step_whose_run_failed_offers_run() {
    let root = TempDir::new("failed");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let mut win = reopen(&folder);
    // Once the folder is open, the step's model is pointed at a column the
    // load's table does not have, so the run makes the load's table and then
    // fails on the step. Opening refuses an edited model, so the edit comes
    // after it.
    let model = std::fs::read_dir(folder.join("models"))
        .expect("the models folder")
        .flatten()
        .map(|e| e.path())
        .find(|p| std::fs::read_to_string(p).is_ok_and(|t| t.starts_with("-- generated:")))
        .expect("a generated model");
    let text = std::fs::read_to_string(&model).expect("the model reads");
    assert!(text.contains("house_age > 28"), "{text}");
    std::fs::write(
        &model,
        text.replace("house_age > 28", "no_such_column > 28"),
    )
    .expect("the model is edited");

    assert!(win.app.run_protocol(&win.ctx), "the run did not start");
    let deadline = Instant::now() + PATIENCE;
    while win.app.run_outstanding() {
        assert!(Instant::now() < deadline, "the run did not land");
        win.run(Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    win.settle();
    assert_eq!(win.status_of(FILTER_STEP), "failed");

    // The model is put back and the folder reopened, so the step previews
    // again while its last run's record still says it failed.
    std::fs::write(&model, text).expect("the model is put back");
    let mut win = reopen(&folder);
    assert_eq!(win.status_of(FILTER_STEP), "failed");
    win.click_grid_under(FILTER_STEP);
    assert!(win.drew(PREVIEW_LABEL), "the preview label was not drawn");
    win.press_cell(0, 1);
    win.key(egui::Key::W, "w");

    assert_eq!(win.app.chart_doc().foot_asking(), None);
    let said = win.foot_status().expect("the status band says why");
    assert!(said.contains("Run"), "{said}");
    assert!(said.contains("not been run"), "{said}");
}

#[test]
fn a_column_with_capitals_previews_and_is_selected_after_save() {
    let root = TempDir::new("column-capitals");
    let folder = root.0.join("data");
    std::fs::create_dir_all(&folder).expect("the data file's folder");
    let data = folder.join("people.csv");
    std::fs::write(&data, "name,Age\nann,31\nbo,24\ncy,45\ndee,29\n").expect("the file writes");
    let boot = Boot::data_file(&data.to_string_lossy()).expect("the file opens");
    let mut win = Window::over(boot);
    win.app.chart_doc_mut().apply_grid_view(RowsView {
        condition: Some("Age > 28".to_owned()),
        order: None,
    });
    win.settle();
    win.app
        .save_protocol(&win.ctx)
        .expect("the window has a Protocol to save")
        .expect("the Save records");
    win.settle();
    // arc spells the step's name in lower case; the column keeps its capital.
    let step = "people_filtered_by_age";
    assert!(
        win.app
            .protocol_model()
            .selected()
            .is_some_and(|id| id.contains(step)),
        "the new step is not selected: {:?}",
        win.app.protocol_model().selected()
    );

    let mut win = reopen(&folder);
    win.click_grid_under(step);
    assert!(win.drew(PREVIEW_LABEL), "the preview label was not drawn");
    let ages = win.preview_cells("Age");
    assert_eq!(ages, vec!["31", "45", "29"]);
}

/// The generated model in `folder`, by the header arc opens it with.
fn generated_model(folder: &Path) -> PathBuf {
    std::fs::read_dir(folder.join("models"))
        .expect("the models folder")
        .flatten()
        .map(|e| e.path())
        .find(|p| std::fs::read_to_string(p).is_ok_and(|t| t.starts_with("-- generated:")))
        .expect("a generated model")
}

/// Rewrite `file` with `from` replaced by `to`, which must occur in it.
fn rewrite(file: &Path, from: &str, to: &str) {
    let text = std::fs::read_to_string(file).expect("the file reads");
    assert!(text.contains(from), "{from:?} is not in {text}");
    std::fs::write(file, text.replace(from, to)).expect("the file writes");
}

#[test]
fn a_load_brightfield_did_not_write_is_not_opened_this_way() {
    // A load that makes a table of another name.
    let root = TempDir::new("renamed-table");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    rewrite(
        &folder.join("models").join("load.sql"),
        "TABLE california_housing AS",
        "TABLE housing AS",
    );
    let model = generated_model(&folder);
    rewrite(&model, "on california_housing", "on housing");
    rewrite(&model, "FROM california_housing", "FROM housing");
    let refused = Boot::open(&manifest(&folder), Flow::Vertical, None)
        .err()
        .expect("a load of another table opened as brightfield's");
    assert!(refused.contains(OFFLINE_VAR), "{refused}");

    // A load whose model is at another path.
    let root = TempDir::new("renamed-model");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    std::fs::rename(
        folder.join("models").join("load.sql"),
        folder.join("models").join("00_load.sql"),
    )
    .expect("the load's model moves");
    rewrite(
        &folder.join("arcform.yaml"),
        "sql: models/load.sql",
        "sql: models/00_load.sql",
    );
    let refused = Boot::open(&manifest(&folder), Flow::Vertical, None)
        .err()
        .expect("a load at another path opened as brightfield's");
    assert!(refused.contains(OFFLINE_VAR), "{refused}");
}

#[test]
fn a_manifest_outside_the_data_files_folder_is_not_refused_for_its_steps() {
    let root = TempDir::new("elsewhere-edited");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    let inner = folder.join("data");
    std::fs::create_dir_all(&inner).expect("the inner folder");
    std::fs::rename(
        folder.join(starts::HOUSING_FILE),
        inner.join(starts::HOUSING_FILE),
    )
    .expect("the data file moves");
    let spelled = format!("./{}", starts::HOUSING_FILE);
    let moved = format!("data/{}", starts::HOUSING_FILE);
    rewrite(&folder.join("arcform.yaml"), &spelled, &moved);
    rewrite(&folder.join("models").join("load.sql"), &spelled, &moved);
    rewrite(&generated_model(&folder), "SELECT *", "SELECT house_age");

    let refused = Boot::open(&manifest(&folder), Flow::Vertical, None)
        .err()
        .expect("a manifest away from its data file opened through it");

    assert!(refused.contains(OFFLINE_VAR), "{refused}");
    assert!(
        !refused.contains(FILTER_STEP),
        "refused for a step it does not open: {refused}"
    );
}

#[test]
fn a_data_file_beside_a_hand_edited_step_opens_its_load_and_says_why() {
    let root = TempDir::new("edited-beside");
    let folder = saved_folder(&root, Some("house_age > 28"), None);
    rewrite(&generated_model(&folder), "SELECT *", "SELECT house_age");
    let before = files_under(&folder)
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(&p).expect("a readable file");
            (p, bytes)
        })
        .collect::<Vec<_>>();

    let boot = Boot::data_file(&folder.join(starts::HOUSING_FILE).to_string_lossy())
        .expect("the data file opens");
    let mut win = Window::over(boot);

    let names: Vec<String> = win
        .app
        .protocol_model()
        .sheet()
        .rows()
        .iter()
        .map(|r| r.name.clone())
        .collect();
    assert_eq!(names, vec!["load".to_string()]);
    let banner = win
        .app
        .notifications()
        .iter()
        .find(|n| n.title == "The steps saved beside this file could not be read")
        .map(|n| n.body.clone().unwrap_or_default())
        .expect("no banner says why the steps were not laid out");
    assert!(banner.contains(FILTER_STEP), "{banner}");

    let err = win
        .app
        .save_protocol(&win.ctx)
        .expect("the window has a Protocol to save")
        .expect_err("a Save over the unreadable steps was not refused");
    assert!(err.contains(FILTER_STEP), "{err}");
    let after = files_under(&folder)
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(&p).expect("a readable file");
            (p, bytes)
        })
        .collect::<Vec<_>>();
    assert_eq!(after, before, "the refused Save wrote into the folder");
}
