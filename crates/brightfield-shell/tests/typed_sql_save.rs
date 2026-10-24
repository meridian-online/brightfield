//! **Save records the views and the kept statements in the order they were
//! made**, each through arc: a view as `filter-rows` and `sort-rows`, a kept
//! statement as a SQL step `arc sql record` writes. The folder then reopens
//! with its SQL steps laid out, previews them, and takes more.
//!
//! The window is handed the `brightfield-shell` binary cargo built for this
//! suite as its runner, so the steps Save writes are arc's own, and a Run is
//! arc's own `run`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_protocol::layout::Flow;
use brightfield_shell::data_grid::use_own_preview_gate_on_this_thread;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::{SpineMarker, SpineRole};
use brightfield_shell::run::Runner;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

/// The housing file's `house_age` column, which a press puts the cursor in.
const HOUSE_AGE: usize = 1;

const BY_AGE: &str = "select house_age, count(*) as n\nfrom california_housing\ngroup by house_age";
const STEP: &str = "california_housing_sql_by_house_age";
const FILTER_STEP: &str = "california_housing_filtered_by_house_age";

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
            "bf-typed-sql-save-{name}-{}-{}",
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

/// A runner that refuses, in arc's way, the recording of the step `name`
/// and hands everything else to the real one.
#[cfg(unix)]
fn refusing_step(dir: &Path, name: &str) -> Runner {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("refusing-step.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n  *' --name {name} -- '*) echo 'error: this step is refused here' >&2; exit 2;;\nesac\nexec '{}' \"$@\"\n",
            env!("CARGO_BIN_EXE_brightfield-shell")
        ),
    )
    .expect("the script writes");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("the script is executable");
    Runner::at(script)
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
    fn over(boot: Boot, history: &Path, runner: Runner) -> Self {
        use_own_preview_gate_on_this_thread();
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
                .running_with(Some(runner))
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
        Self::housing_with(root, runner())
    }

    fn housing_with(root: &TempDir, runner: Runner) -> (Self, PathBuf) {
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(starts::HOUSING_FILE);
        std::fs::write(&data, include_bytes!("data/california_housing.parquet"))
            .expect("the housing file writes");
        let boot = Boot::data_file(&data.to_string_lossy()).expect("the housing file opens");
        (Self::over(boot, &root.0.join("history"), runner), folder)
    }

    /// The folder Save wrote, opened the way a front-door row or the command
    /// line opens it.
    fn reopen(root: &TempDir, folder: &Path) -> Self {
        let manifest = folder.join("arcform.yaml");
        let boot = Boot::open(&manifest.to_string_lossy(), Flow::Vertical, None)
            .expect("the folder opens");
        Self::over(boot, &root.0.join("history"), runner())
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

    /// Keep `text` as a step: the palette on `space`, the text typed, its
    /// read finished, and Cmd+Enter.
    fn keep(&mut self, text: &str) {
        self.key(egui::Key::Space);
        self.type_text(text);
        assert!(
            self.app.finish_typed_reads(Duration::from_secs(60)),
            "the read of {text:?} did not finish"
        );
        self.run(vec![press(egui::Key::Enter, egui::Modifiers::COMMAND)]);
        self.settle();
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
        self.until(|w| {
            w.app
                .chart_doc()
                .grid_preview()
                .is_none_or(|p| !p.is_reading())
        });
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

    fn typed_names(&self) -> Vec<String> {
        self.app
            .chart_doc()
            .typed_steps()
            .iter()
            .map(|s| s.name.clone())
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

#[test]
fn save_records_views_and_typed_steps_in_the_order_made() {
    let root = TempDir::new("order");
    let (mut win, folder) = Window::housing(&root);
    win.apply_where_at(HOUSE_AGE, "28.0", ">");
    win.keep(BY_AGE);
    win.save();
    assert_eq!(manifest_steps(&folder), ["load", FILTER_STEP, STEP]);
    assert_eq!(
        model(&folder, STEP),
        format!("CREATE OR REPLACE TABLE \"{STEP}\" AS\n{BY_AGE}\n")
    );
    assert!(win.app.chart_doc().typed_steps().is_empty() && !win.app.carries_unsaved_mark());
    assert!(win
        .app
        .spine_rows()
        .iter()
        .all(|r| r.marker != SpineMarker::Unrecorded));
}

#[test]
fn a_view_applied_after_a_typed_step_is_recorded_after_it() {
    let root = TempDir::new("after");
    let (mut win, folder) = Window::housing(&root);
    win.keep(BY_AGE);
    win.click_grid_under("california_housing");
    win.apply_where_at(HOUSE_AGE, "28.0", ">");
    win.save();
    assert_eq!(manifest_steps(&folder), ["load", STEP, FILTER_STEP]);
}

#[test]
fn a_saved_folder_with_sql_steps_reopens_with_them_and_saves_again() {
    let root = TempDir::new("reopen");
    let (mut win, folder) = Window::housing(&root);
    win.keep(BY_AGE);
    win.save();
    drop(win);
    let mut win = Window::reopen(&root, &folder);
    assert_eq!(win.sheet_steps(), ["load", STEP]);
    assert_eq!(win.status_of(STEP), "not run");
    win.click_grid_under(STEP);
    assert!(!win.preview_cells("n").is_empty());
    win.keep(&format!("from {STEP} select house_age order by house_age"));
    win.save();
    assert_eq!(
        manifest_steps(&folder),
        ["load", STEP, format!("{STEP}_sql").as_str()]
    );
}

#[test]
fn a_statement_ending_in_a_semicolon_is_recorded_and_runs() {
    let root = TempDir::new("semicolon");
    let (mut win, folder) = Window::housing(&root);
    win.keep("select house_age, count(*) as n from california_housing group by house_age;");
    // Run records pending work first, as Save does.
    win.run_protocol();
    assert_eq!(manifest_steps(&folder), ["load", STEP]);
    assert_eq!(win.status_of(STEP), "ok");
}

#[test]
fn a_typed_step_arc_reads_as_a_filter_is_recorded_as_one_and_reopens() {
    let root = TempDir::new("filter");
    let (mut win, folder) = Window::housing(&root);
    win.keep(BY_AGE);
    win.keep(&format!("select * from {STEP} where n > 100"));
    let names = win.typed_names();
    assert_eq!(names.len(), 2, "{names:?}");
    win.save();
    assert_eq!(manifest_steps(&folder), ["load", STEP, names[1].as_str()]);
    let filter = model(&folder, &names[1]);
    assert!(
        filter.starts_with(&format!("-- generated: filter-rows on {STEP}\n")),
        "{filter}"
    );
    drop(win);
    let win = Window::reopen(&root, &folder);
    assert_eq!(win.sheet_steps(), ["load", STEP, names[1].as_str()]);
}

#[cfg(unix)]
#[test]
fn a_refusal_leaves_what_was_recorded_and_lays_out_the_rest_unrecorded() {
    let root = TempDir::new("partial");
    let scripts = TempDir::new("partial-runner");
    let later = format!("{STEP}_sql");
    let (mut win, folder) = Window::housing_with(&root, refusing_step(&scripts.0, &later));
    win.keep(BY_AGE);
    win.keep(&format!("from {STEP} select house_age, n order by n desc"));
    assert_eq!(win.typed_names(), [STEP.to_string(), later.clone()]);
    win.save();

    assert_eq!(manifest_steps(&folder), ["load", STEP]);
    assert_eq!(win.typed_names(), [later.as_str()]);
    assert!(win.app.carries_unsaved_mark());
    assert!(
        win.app
            .notifications()
            .iter()
            .any(|n| n.title == "Could not record the SQL step"
                && n.body.as_deref() == Some("this step is refused here")),
        "the refusal reached no banner"
    );
    assert_eq!(win.sheet_steps(), ["load", STEP, later.as_str()]);
    let recorded = win.markers_of(STEP);
    assert!(
        !recorded.is_empty() && !recorded.contains(&SpineMarker::Unrecorded),
        "{recorded:?}"
    );
    let refused = win.markers_of(&later);
    assert!(
        !refused.is_empty() && refused.iter().all(|m| *m == SpineMarker::Unrecorded),
        "{refused:?}"
    );
}
