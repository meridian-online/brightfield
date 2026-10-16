//! **Cmd+Enter keeps the statement on the statement form as a step**: a step
//! no file records yet, laid out at the foot of the Protocol with its table
//! brought to the front in the grid, counted off the frame, and readable by
//! the next statement. A statement held on the form again is corrected in
//! place, or, when it reads its own step's table, made the step after it.
//!
//! The window is handed the `brightfield-shell` binary cargo built for this
//! suite as its runner, so what arc makes of a statement is arc's own answer.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_shell::data_grid::use_own_preview_gate_on_this_thread;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::SpineRole;
use brightfield_shell::run::Runner;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

/// The housing file's `house_age` column, which a press puts the cursor in.
const HOUSE_AGE: usize = 1;

const BY_AGE_LINES: [&str; 3] = [
    "select house_age, count(*) as n",
    "from california_housing",
    "group by house_age",
];
const BY_AGE: &str = "select house_age, count(*) as n\nfrom california_housing\ngroup by house_age";
const STEP: &str = "california_housing_sql_by_house_age";

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// A directory of this test's own. The window over it hands back a
/// [`Removed`] for it, so the directory lasts as long as that does.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-typed-sql-keep-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory for the fixture");
        Self(dir)
    }
}

/// Removes its directory when the test ends.
struct Removed(PathBuf);

impl Drop for Removed {
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

/// A headless window.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    texts: Vec<(egui::Pos2, String)>,
    /// The egui clock, a second on per frame, so a layer that fades in is
    /// drawn whole by the frame after it opens, as a person sees it.
    time: f64,
}

impl Window {
    /// A window over the housing file's bytes, written into `root`'s `data`
    /// folder, and what removes `root` once dropped. Its documents read their
    /// previews behind a gate of this test's own.
    fn housing(root: &TempDir) -> (Self, Removed) {
        use_own_preview_gate_on_this_thread();
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(starts::HOUSING_FILE);
        std::fs::write(&data, starts::HOUSING_PARQUET).expect("the housing file writes");
        let boot = Boot::data_file(&data.to_string_lossy()).expect("the housing file opens");
        let history: &Path = &root.0.join("history");
        let app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light)
            .running_with(Some(runner()))
            .keeping_history(Some(brightfield_protocol::HistoryStore::At(
                history.to_path_buf(),
            )));
        let mut win = Self {
            app,
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            texts: Vec::new(),
            time: 0.0,
        };
        win.settle();
        (win, Removed(root.0.clone()))
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        self.time += 1.0;
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            time: Some(self.time),
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

    /// Cmd+Enter, which keeps the statement on the form.
    fn cmd_enter(&mut self) {
        self.run(vec![press(egui::Key::Enter, egui::Modifiers::COMMAND)]);
        self.settle();
    }

    /// Keep `text` as a step: the palette on `space`, the text typed (over
    /// a held statement, which opens selected, it replaces it), its read
    /// finished, and Cmd+Enter.
    fn keep(&mut self, text: &str) {
        self.key(egui::Key::Space);
        self.type_text(text);
        assert!(
            self.app.finish_typed_reads(Duration::from_secs(60)),
            "the read of {text:?} did not finish"
        );
        self.cmd_enter();
        assert_eq!(self.app.open_overlay(), None, "{text:?} was not kept");
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
}

#[test]
fn cmd_enter_keeps_a_statement_as_a_new_step_in_front() {
    let (mut win, _f) = Window::housing(&TempDir::new("keep"));
    win.key(egui::Key::Space);
    for (i, line) in BY_AGE_LINES.iter().enumerate() {
        if i > 0 {
            win.key(egui::Key::Enter);
        }
        win.type_text(line);
    }
    win.app.finish_typed_reads(Duration::from_secs(60));
    win.settle();
    assert!(
        win.app
            .chart_doc()
            .grid_preview()
            .is_some_and(|p| p.page().is_some()),
        "rows before Cmd+Enter"
    );
    win.cmd_enter();
    assert_eq!(win.app.open_overlay(), None);
    assert_eq!(
        win.app
            .chart_doc()
            .typed_steps()
            .iter()
            .map(|s| (s.name.as_str(), s.statement.as_str()))
            .collect::<Vec<_>>(),
        [(STEP, BY_AGE)]
    );
    assert!(win.app.carries_unsaved_mark());
    win.until_preview_read();
    assert_eq!(win.app.chart_doc().grid_preview().unwrap().step(), STEP);
    assert_eq!(
        win.app.chart_doc().grid_cursor.map(|c| (c.row, c.col)),
        Some((0, 0))
    );
    win.until(|w| {
        w.app
            .spine_rows()
            .iter()
            .any(|r| r.label == STEP && r.kind == "table \u{b7} 52 rows")
    });
    let at = |label: &str| {
        win.app
            .spine_rows()
            .iter()
            .position(|r| r.label == label)
            .unwrap()
    };
    assert!(at("california_housing") < at(STEP));
}

#[test]
fn cmd_enter_on_a_refused_statement_keeps_nothing_and_says_why() {
    let (mut win, _f) = Window::housing(&TempDir::new("refuse"));
    for (sql, said) in [
        ("select * from nowhere", "nowhere"),
        ("select 1; select 2", "2 statements"),
        ("select * from read_csv('/etc/hosts')", "external access"),
        (
            "with t as (select 1) insert into u select * from t",
            "not one query that reads rows",
        ),
    ] {
        win.key(egui::Key::Space);
        win.type_text(sql);
        win.app.finish_typed_reads(Duration::from_secs(60));
        win.cmd_enter();
        assert_eq!(win.app.open_overlay(), Some("palette"), "{sql}");
        let shown = win.app.typed_said();
        assert!(
            shown.as_deref().is_some_and(|s| s.contains(said)),
            "{sql}: {shown:?}"
        );
        assert!(win.app.chart_doc().typed_steps().is_empty());
        win.key(egui::Key::Escape);
    }
}

#[test]
fn a_second_statement_reads_the_first_unsaved_table() {
    let (mut win, _f) = Window::housing(&TempDir::new("chain"));
    win.keep(BY_AGE);
    win.key(egui::Key::Space);
    assert_eq!(
        win.app.typed_statement(),
        Some(BY_AGE),
        "the palette holds the statement in front"
    );
    // Typed over the held statement, which opens selected, so it replaces it.
    win.type_text(&format!("from {STEP} select house_age, n order by n desc"));
    win.app.finish_typed_reads(Duration::from_secs(60));
    win.cmd_enter();
    let names: Vec<_> = win
        .app
        .chart_doc()
        .typed_steps()
        .iter()
        .map(|s| s.name.clone())
        .collect();
    assert_eq!(names, [STEP.to_string(), format!("{STEP}_sql")]);
}

#[test]
fn a_correction_keeps_the_name_a_later_statement_reads() {
    let (mut win, _f) = Window::housing(&TempDir::new("correct"));
    win.keep(BY_AGE);
    win.keep(&format!("from {STEP} select house_age, n order by n desc"));
    win.click_grid_under(STEP);
    win.key(egui::Key::Space);
    // The held statement opens selected; this puts the caret at its end.
    win.key(egui::Key::ArrowRight);
    win.key(egui::Key::Enter);
    win.type_text("having count(*) > 100");
    win.app.finish_typed_reads(Duration::from_secs(60));
    win.cmd_enter();
    let steps = win.app.chart_doc().typed_steps();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].name, STEP);
    assert!(
        steps[0].statement.ends_with("having count(*) > 100"),
        "{}",
        steps[0].statement
    );
    win.click_grid_under(&format!("{STEP}_sql"));
    assert!(
        win.app.chart_doc().grid_preview().unwrap().page().is_some(),
        "the later step still binds"
    );
}

#[test]
fn a_pending_view_stays_pending_beside_a_typed_step() {
    let (mut win, _f) = Window::housing(&TempDir::new("view"));
    win.apply_where_at(HOUSE_AGE, "28.0", ">");
    win.keep(BY_AGE);
    assert_eq!(
        win.app.chart_doc().grid_view().condition.as_deref(),
        Some("house_age > 28")
    );
    assert!(win.app.chart_doc().grid_view_unrecorded());
}

#[test]
fn the_close_question_names_typed_steps() {
    let (mut win, _f) = Window::housing(&TempDir::new("close"));
    win.keep(BY_AGE);
    assert!(win.app.ask_before_closing());
    win.settle();
    assert!(win.drew(&format!("SQL step \u{b7} {STEP}")));
}

#[test]
fn a_key_on_a_kept_statements_grid_says_save_records_it() {
    let (mut win, _f) = Window::housing(&TempDir::new("offer"));
    win.keep(BY_AGE);
    win.until_preview_read();
    win.keystroke(egui::Key::W, "w");
    assert_eq!(
        win.app.chart_doc().foot_said().as_deref(),
        Some("this step is not recorded yet; Save records it")
    );
}
