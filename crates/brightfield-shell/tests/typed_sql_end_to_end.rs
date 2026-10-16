//! **Typed SQL driven end to end, through both of Cmd+Enter's outcomes.** A
//! statement arc reads as a filter of the loaded table lands in `where` and is
//! recorded as the step the filter key records; any other statement is kept
//! as a step of its own, laid out unrecorded, saved through arc, reopened and
//! run. A run's record stays with the steps it ran while a typed step waits to
//! be saved.
//!
//! The window is handed the `brightfield-shell` binary cargo built for this
//! suite as its runner, so what arc makes of a statement, the steps Save
//! writes and the Run are all arc's own.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_protocol::layout::Flow;
use brightfield_shell::data_grid::use_own_preview_gate_on_this_thread;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::{SpineMarker, SpineRole};
use brightfield_shell::run::{self, Runner};
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

/// The table the housing file's load makes.
const ROOT: &str = "california_housing";

/// The housing file's `house_age` column, which a press puts the cursor in.
const HOUSE_AGE: usize = 1;

/// A filter of the loaded table, as typed.
const FILTER: &str = "select * from california_housing where house_age > 28";
/// The step the filter key and the typed filter both record.
const FILTER_STEP: &str = "california_housing_filtered_by_house_age";

const BY_AGE_LINES: [&str; 3] = [
    "select house_age, count(*) as n",
    "from california_housing",
    "group by house_age",
];
const BY_AGE: &str = "select house_age, count(*) as n\nfrom california_housing\ngroup by house_age";
const STEP: &str = "california_housing_sql_by_house_age";

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
            "bf-typed-sql-end-to-end-{name}-{}-{}",
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

fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// A headless window, over the housing file or over the folder it was saved
/// into.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The egui clock, a second on per frame, so a layer that fades in is
    /// drawn whole by the frame after it opens.
    time: f64,
}

impl Window {
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
            time: 0.0,
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
        self.time += 1.0;
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.time),
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

    /// Cmd+Enter, which keeps the statement on the form.
    fn cmd_enter(&mut self) {
        self.run(vec![press(egui::Key::Enter, egui::Modifiers::COMMAND)]);
        self.settle();
    }

    /// Finish the read of the statement on the form and draw its answer.
    fn finish_read(&mut self) {
        assert!(
            self.app.finish_typed_reads(Duration::from_secs(60)),
            "the read of the statement did not finish"
        );
        self.settle();
    }

    /// Keep `text` as a step: the palette on `space`, the text typed (over
    /// a held statement, which opens selected, it replaces it), its read
    /// finished, and Cmd+Enter.
    fn keep(&mut self, text: &str) {
        self.key(egui::Key::Space);
        self.type_text(text);
        self.finish_read();
        self.cmd_enter();
        assert_eq!(self.app.open_overlay(), None, "{text:?} was not kept");
    }

    /// **Save, through the gesture a person has**: the palette on `space`,
    /// the verb typed, confirmed with Enter.
    fn save(&mut self) {
        assert!(self.app.has_protocol_to_save(), "nothing to save");
        self.key(egui::Key::Space);
        assert_eq!(self.app.open_overlay(), Some("palette"));
        // A palette over a kept statement opens holding it, selected, so the
        // verb typed replaces it.
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
        self.run(vec![press(egui::Key::Enter, egui::Modifiers::COMMAND)]);
        assert!(self.app.run_in_progress(), "cmd-enter did not start a run");
        let deadline = Instant::now() + PATIENCE;
        while self.app.run_outstanding() {
            assert!(Instant::now() < deadline, "the run did not land");
            self.run(Vec::new());
            std::thread::sleep(Duration::from_millis(5));
        }
        self.settle();
    }

    /// Draw frames until `pred` holds, or fail after a minute.
    fn until(&mut self, pred: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !pred(self) {
            assert!(Instant::now() < deadline, "waited a minute in vain");
            std::thread::sleep(Duration::from_millis(5));
            self.run(Vec::new());
        }
        self.settle();
    }

    /// Draw frames until the grid's preview has been read.
    fn until_preview_read(&mut self) {
        self.until(|w| {
            w.app
                .chart_doc()
                .grid_preview()
                .is_some_and(|p| !p.is_reading())
        });
    }

    fn click(&mut self, at: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(true), button(false)]);
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
    /// moving down from the top a row at a time with `j`.
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

    /// Apply a `where` from the cell of `col` reading `value`, with the
    /// comparison `sign`: the cursor on the cell, `w`, the sign, Enter.
    fn apply_where_at(&mut self, col: usize, value: &str, sign: &str) {
        self.cursor_to(col, value);
        self.keystroke(egui::Key::W, "w");
        self.type_text(sign);
        self.key(egui::Key::Enter);
    }

    /// Click the `grid` row under the asset row labelled `node`, and wait
    /// for its preview.
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
        self.until_preview_read();
    }

    /// The rows the grid's table drew: what the band counts.
    fn grid_rows(&self) -> u64 {
        self.app
            .chart_doc()
            .grid_drawn()
            .expect("the grid drew a table")
            .rows
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

    /// The value of each cell of `column` on the preview page.
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

    /// The markers of the spine rows labelled `label`.
    fn markers_of(&self, label: &str) -> Vec<SpineMarker> {
        self.app
            .spine_rows()
            .iter()
            .filter(|r| r.label == label)
            .map(|r| r.marker)
            .collect()
    }

    /// The marker of the spine's asset row labelled `label`: the table.
    fn table_marker(&self, label: &str) -> SpineMarker {
        self.app
            .spine_rows()
            .iter()
            .find(|r| r.role == SpineRole::Asset && r.label == label)
            .unwrap_or_else(|| panic!("the spine drew no table {label}"))
            .marker
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

/// Whether the folder's newest run record says the table `name` was written.
fn materialised(folder: &Path, name: &str) -> bool {
    let record = run::records_newest_first(folder, ROOT)
        .next()
        .expect("the run wrote a record");
    let ids: Vec<&String> = record.assets.keys().collect();
    record
        .assets
        .iter()
        .find(|(id, _)| id.rsplit(['.', '/', ':']).next() == Some(name))
        .unwrap_or_else(|| panic!("the run record has no asset {name}: {ids:?}"))
        .1
        .materialized
}

#[test]
fn a_typed_filter_records_the_same_step_as_the_filter_key() {
    // Folder A, by the key.
    let root_a = TempDir::new("by-key");
    let (mut win, a) = Window::housing(&root_a);
    win.apply_where_at(HOUSE_AGE, "28.0", ">");
    win.save();
    drop(win);

    // Folder B, by typing.
    let root_b = TempDir::new("by-typing");
    let (mut win, b) = Window::housing(&root_b);
    win.key(egui::Key::Space);
    win.type_text(FILTER);
    win.finish_read();
    assert_eq!(
        win.grid_rows(),
        8_831,
        "the grid's count changes before Cmd+Enter"
    );
    win.cmd_enter();
    assert_eq!(win.app.open_overlay(), None, "the filter was not kept");
    assert_eq!(
        win.app.chart_doc().grid_view().condition.as_deref(),
        Some("house_age > 28")
    );
    win.save();
    drop(win);

    assert_eq!(manifest_steps(&b), manifest_steps(&a));
    assert_eq!(manifest_steps(&b), ["load", FILTER_STEP]);
    assert_eq!(
        model(&b, FILTER_STEP).into_bytes(),
        model(&a, FILTER_STEP).into_bytes()
    );

    let win = Window::reopen(&root_a, &a);
    assert_eq!(win.sheet_steps(), ["load", FILTER_STEP]);
    drop(win);
    let mut win = Window::reopen(&root_b, &b);
    assert_eq!(win.sheet_steps(), ["load", FILTER_STEP]);
    win.run_protocol();
    assert_eq!(win.status_of(FILTER_STEP), "ok");
}

#[test]
fn a_typed_statement_no_operation_covers_is_kept_saved_reopened_and_run() {
    let root = TempDir::new("statement");
    let (mut win, folder) = Window::housing(&root);

    // Typed over three lines, with Enter between them.
    win.key(egui::Key::Space);
    for (i, line) in BY_AGE_LINES.iter().enumerate() {
        if i > 0 {
            win.key(egui::Key::Enter);
        }
        win.type_text(line);
    }
    assert_eq!(win.app.typed_statement(), Some(BY_AGE));
    win.finish_read();
    let preview = win
        .app
        .chart_doc()
        .grid_preview()
        .expect("the grid previews the statement before Cmd+Enter");
    assert_eq!(preview.step(), STEP);
    assert!(preview.page().is_some(), "its first page is in the grid");

    // Kept: the step and its table at the spine's foot, unrecorded, and the
    // table in the grid.
    win.cmd_enter();
    assert_eq!(win.app.open_overlay(), None, "the statement was not kept");
    win.until_preview_read();
    assert_eq!(win.app.chart_doc().grid_preview().unwrap().step(), STEP);
    let unrecorded = win.markers_of(STEP);
    assert!(
        unrecorded.len() >= 2 && unrecorded.iter().all(|m| *m == SpineMarker::Unrecorded),
        "the step and its table carry the not-recorded mark: {unrecorded:?}"
    );
    let rows = win.app.spine_rows();
    let last_of = |label: &str| rows.iter().rposition(|r| r.label == label).unwrap();
    let first_of = |label: &str| rows.iter().position(|r| r.label == label).unwrap();
    assert!(
        last_of(ROOT) < first_of(STEP) && last_of("load") < first_of(STEP),
        "the kept step stands after every recorded one"
    );

    // A second statement, FROM-first, reading the first's unsaved table.
    let second = format!("{STEP}_sql");
    win.keep(&format!("from {STEP} select house_age, n order by n desc"));
    assert_eq!(
        win.app
            .chart_doc()
            .typed_steps()
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        [STEP, second.as_str()]
    );
    win.save();
    drop(win);

    assert_eq!(manifest_steps(&folder), ["load", STEP, second.as_str()]);
    assert_eq!(
        model(&folder, STEP),
        format!("CREATE OR REPLACE TABLE \"{STEP}\" AS\n{BY_AGE}\n")
    );

    // Reopened: the sheet lists all three, and each SQL step's grid previews.
    let mut win = Window::reopen(&root, &folder);
    assert_eq!(win.sheet_steps(), ["load", STEP, second.as_str()]);
    for step in [STEP, second.as_str()] {
        win.click_grid_under(step);
        assert!(
            !win.preview_cells("house_age").is_empty(),
            "{step}'s grid previews no rows"
        );
    }

    // Run: every step reads ok, and the record has both tables written.
    win.run_protocol();
    for step in ["load", STEP, second.as_str()] {
        assert_eq!(win.status_of(step), "ok", "{step}");
    }
    for table in [STEP, second.as_str()] {
        assert!(
            materialised(&folder, table),
            "the run record does not mark {table} materialised"
        );
    }
}

#[test]
fn a_run_record_stays_with_the_recorded_steps_while_a_typed_step_waits() {
    let root = TempDir::new("run-stays");
    let (mut win, folder) = Window::housing(&root);
    win.keep(BY_AGE);
    // Run records the pending step first, as Save does, then runs it.
    win.run_protocol();
    assert_eq!(manifest_steps(&folder), ["load", STEP]);
    assert_eq!(win.status_of(STEP), "ok");
    assert_eq!(win.table_marker(STEP), SpineMarker::Filled);

    // Another statement typed and kept, reading the run's table.
    let waiting = format!("{STEP}_sql");
    win.keep(&format!("from {STEP} select house_age, n order by n desc"));
    assert_eq!(win.sheet_steps(), ["load", STEP, waiting.as_str()]);
    assert_eq!(
        win.status_of(STEP),
        "ok",
        "keeping a statement dropped the run's record of the step before it"
    );
    assert_eq!(
        win.table_marker(STEP),
        SpineMarker::Filled,
        "keeping a statement forgot that the run wrote the table before it"
    );
    assert_eq!(win.table_marker(&waiting), SpineMarker::Unrecorded);
}
