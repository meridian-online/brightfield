//! **Typed SQL that arc reads as a filter of the loaded table lands in the
//! `where` field**, as the filter key leaves it: the grid previews the
//! condition with the band's count while the statement is on the form, and
//! Cmd+Enter applies it as the grid's view, unrecorded, with no step of its
//! own. A filter of another table, or one beside a recorded condition the
//! field holds, is kept as a SQL step.
//!
//! The window is handed the `brightfield-shell` binary cargo built for this
//! suite as its runner, so what arc makes of a statement is arc's own answer.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use brightfield_engine::RowsView;
use brightfield_shell::data_grid::use_own_preview_gate_on_this_thread;
use brightfield_shell::design::Mode;
use brightfield_shell::run::Runner;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

const FILTER: &str = "select * from california_housing where house_age > 28";
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
            "bf-typed-sql-where-{name}-{}-{}",
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
        std::fs::write(&data, include_bytes!("data/california_housing.parquet"))
            .expect("the housing file writes");
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

    /// Keep `text`: the palette on `space`, the text typed, its read
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

    /// The rows the grid's table drew: what the band counts.
    fn grid_rows(&self) -> u64 {
        self.app
            .chart_doc()
            .grid_drawn()
            .expect("the grid drew a table")
            .rows
    }
}

#[test]
fn a_typed_filter_lands_in_where_as_the_filter_key_leaves_it() {
    let (mut win, _f) = Window::housing(&TempDir::new("where"));
    assert_eq!(win.grid_rows(), 16_640);
    win.key(egui::Key::Space);
    win.type_text(FILTER);
    win.app.finish_typed_reads(Duration::from_secs(60));
    win.settle();
    assert_eq!(
        win.app.typed_said().as_deref(),
        Some("filter-rows on california_housing \u{b7} 8,831 rows")
    );
    assert_eq!(
        win.grid_rows(),
        8_831,
        "the grid's count changes before Cmd+Enter"
    );
    assert!(!win.app.carries_unsaved_mark());
    win.cmd_enter();
    assert_eq!(win.app.open_overlay(), None);
    assert_eq!(
        win.app.chart_doc().grid_view().condition.as_deref(),
        Some("house_age > 28")
    );
    assert!(win.app.chart_doc().foot_drawn().unwrap().where_dot);
    assert!(win.app.carries_unsaved_mark());
    assert!(win.app.chart_doc().typed_steps().is_empty());
}

#[test]
fn a_typed_filter_in_another_case_still_lands_in_where() {
    let (mut win, _f) = Window::housing(&TempDir::new("case"));
    win.keep("SELECT * FROM California_Housing WHERE house_age > 28");
    assert_eq!(
        win.app.chart_doc().grid_view().condition.as_deref(),
        Some("house_age > 28")
    );
}

#[test]
fn a_filter_of_an_unsaved_table_becomes_a_sql_step() {
    let (mut win, _f) = Window::housing(&TempDir::new("on-typed"));
    win.keep(BY_AGE);
    win.key(egui::Key::Space);
    win.type_text(&format!("select * from {STEP} where n > 100"));
    win.app.finish_typed_reads(Duration::from_secs(60));
    assert_eq!(win.app.typed_said().as_deref(), Some("a SQL step"));
    win.cmd_enter();
    assert_eq!(win.app.chart_doc().typed_steps().len(), 2);
    assert_eq!(win.app.chart_doc().grid_view().condition, None);
}

#[test]
fn a_filter_beside_a_held_condition_becomes_a_sql_step() {
    let (mut win, _f) = Window::housing(&TempDir::new("held"));
    win.app.chart_doc_mut().apply_grid_view(RowsView {
        condition: Some("house_age > 10".into()),
        order: Some("house_age".into()),
    });
    win.app
        .chart_doc_mut()
        .mark_grid_condition_recorded("california_housing_filtered_by_house_age".into());
    win.keep(FILTER);
    assert_eq!(win.app.chart_doc().typed_steps().len(), 1);
    assert_eq!(
        win.app.chart_doc().grid_view().condition.as_deref(),
        Some("house_age > 10")
    );
}

#[test]
fn esc_after_a_typed_filter_gives_the_grid_its_rows_back() {
    let (mut win, _f) = Window::housing(&TempDir::new("esc"));
    win.key(egui::Key::Space);
    win.type_text(FILTER);
    win.app.finish_typed_reads(Duration::from_secs(60));
    win.settle();
    win.key(egui::Key::Escape);
    assert_eq!(win.grid_rows(), 16_640);
    assert_eq!(win.app.chart_doc().grid_view().condition, None);
}

/// The typed filter's count is read on a worker, as the statement's page is:
/// while a read is held, frames go on drawing, and the grid shows the count
/// last answered until the new one lands.
#[test]
fn typed_where_preview_does_not_block_the_ui_thread() {
    let (mut win, _f) = Window::housing(&TempDir::new("held-read"));
    win.key(egui::Key::Space);
    // One character short of the filter: `house_age > 2`.
    win.type_text(&FILTER[..FILTER.len() - 1]);
    win.app.finish_typed_reads(Duration::from_secs(60));
    win.settle();
    let before = win.grid_rows();
    assert!(
        before != 16_640 && before != 8_831,
        "the shorter filter's own count: {before}"
    );
    win.app.chart_doc().hold_preview_reads(true);
    let started = Instant::now();
    win.type_text("8");
    // Past the pause, so the read of the new text has started and is held.
    std::thread::sleep(Duration::from_millis(400));
    win.settle();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "frames drew while the read was held"
    );
    assert_eq!(
        win.grid_rows(),
        before,
        "the last answered count while the new read is held"
    );
    win.app.chart_doc().hold_preview_reads(false);
    assert!(win.app.finish_typed_reads(Duration::from_secs(60)));
    win.settle();
    assert_eq!(win.grid_rows(), 8_831);
}

#[test]
fn a_kept_statement_rewritten_as_a_filter_of_the_loaded_table_stays_a_step() {
    let (mut win, _f) = Window::housing(&TempDir::new("rewrite"));
    win.keep(BY_AGE);
    // Over the kept statement's grid, the form holds it selected, so the
    // filter typed replaces it.
    win.keep(FILTER);
    let steps = win.app.chart_doc().typed_steps();
    assert_eq!(steps.len(), 1);
    assert_eq!(
        (steps[0].name.as_str(), steps[0].statement.as_str()),
        (STEP, FILTER)
    );
    assert_eq!(win.app.chart_doc().grid_view().condition, None);
}

#[test]
fn a_typed_filter_keeps_the_pending_order() {
    let (mut win, _f) = Window::housing(&TempDir::new("order"));
    win.app.chart_doc_mut().apply_grid_view(RowsView {
        condition: None,
        order: Some("house_age desc".into()),
    });
    win.keep(FILTER);
    assert_eq!(
        win.app.chart_doc().grid_view(),
        &RowsView {
            condition: Some("house_age > 28".into()),
            order: Some("house_age desc".into()),
        }
    );
}
