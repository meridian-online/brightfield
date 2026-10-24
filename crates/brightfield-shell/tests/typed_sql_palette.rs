//! **SQL typed on the chart palette's query line**: text that reads as a
//! statement turns the palette into the statement form, a card with no scrim
//! that stands clear of the grid, and the grid previews the statement's rows
//! once the text has rested.
//!
//! The window is handed the `brightfield-shell` binary cargo built for this
//! suite as its runner, so what arc makes of a statement is arc's own answer.

use std::path::{Path, PathBuf};
use std::time::Duration;

use brightfield_shell::data_grid::use_own_preview_gate_on_this_thread;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec_in_mode;
use brightfield_shell::run::Runner;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

fn runner() -> Runner {
    Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))
}

/// A directory of this test's own. The window over it hands back a
/// [`Removed`] for it, so the directory lasts as long as that does, though the
/// `TempDir` itself is dropped at the end of the statement that made it.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-typed-sql-palette-{name}-{}-{}",
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

/// The filled rects of `shape`, with their fill.
fn collect_rects(shape: &egui::Shape, out: &mut Vec<(egui::Rect, egui::Color32)>) {
    match shape {
        egui::Shape::Rect(rect) => out.push((rect.rect, rect.fill)),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_rects(shape, out);
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
    rects: Vec<(egui::Rect, egui::Color32)>,
    /// The egui clock, a second on per frame, so a layer that fades in is
    /// drawn whole by the frame after it opens, as a person sees it.
    time: f64,
}

impl Window {
    /// The window's documents read their previews behind a gate of this
    /// test's own, so no test here waits behind another's reads.
    fn over(app: MeridianApp) -> Self {
        let mut win = Self {
            app,
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            texts: Vec::new(),
            rects: Vec::new(),
            time: 0.0,
        };
        win.settle();
        win
    }

    /// A window over the housing file's bytes, written into `root`'s `data`
    /// folder, and what removes `root` once dropped.
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
        (Self::over(app), Removed(root.0.clone()))
    }

    /// A window over a chart spec, booted as `surfaces.rs` boots one: no data
    /// file, so no Protocol to save.
    fn over_chart_spec(path: &str) -> Self {
        use_own_preview_gate_on_this_thread();
        let spec = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path);
        let composed =
            compose_spec_in_mode(spec.to_str().expect("utf-8 fixture path"), Mode::Light)
                .unwrap_or_else(|e| panic!("compose {}: {e}", spec.display()));
        Self::over(MeridianApp::headless_with_layout(
            Boot::charts(composed),
            default_layout(),
            Mode::Light,
        ))
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
        let mut rects = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut texts);
            collect_rects(&clipped.shape, &mut rects);
        }
        self.texts = texts;
        self.rects = rects;
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

    /// Type `text` as a keyboard does: the text a keystroke makes.
    fn type_text(&mut self, text: &str) {
        self.run(vec![egui::Event::Text(text.to_owned())]);
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

    /// Select everything in the field that holds the keyboard, and erase it.
    fn erase_all(&mut self) {
        self.run(vec![press(egui::Key::A, egui::Modifiers::COMMAND)]);
        self.run(vec![press(egui::Key::Backspace, egui::Modifiers::NONE)]);
        self.settle();
    }

    /// Whether the last frame painted a rect over the whole screen in either
    /// theme's scrim ink: the backdrop a modal layer dims the page with.
    fn painted_a_scrim(&self) -> bool {
        let scrims = [false, true].map(|dark| {
            meridian_egui::theme::to_color32(meridian_design::semantic(dark).surfaces.scrim)
        });
        self.rects
            .iter()
            .any(|(rect, fill)| rect.contains_rect(self.screen) && scrims.contains(fill))
    }

    /// The text of each cell of `column` in the preview the grid holds.
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

#[test]
fn a_statement_on_the_query_line_turns_the_palette_into_the_statement_form() {
    let (mut win, _f) = Window::housing(&TempDir::new("form"));
    win.key(egui::Key::Space);
    win.type_text("selection");
    assert_eq!(win.app.typed_statement(), None);
    assert!(win
        .app
        .open_palette_rows()
        .contains(&"clear-selection".to_string()));
    win.erase_all();
    win.type_text("  SELECT");
    assert_eq!(win.app.typed_statement(), Some("  SELECT"));
    assert_eq!(win.app.open_overlay(), Some("palette"));
    win.erase_all();
    win.type_text("clear");
    assert_eq!(win.app.typed_statement(), None);
}

#[test]
fn enter_adds_a_line_and_the_card_grows_up_to_the_tile() {
    let (mut win, _f) = Window::housing(&TempDir::new("lines"));
    win.key(egui::Key::Space);
    win.type_text("select *");
    let one = win.app.typed_card_rect().unwrap();
    win.key(egui::Key::Enter);
    win.type_text("from california_housing");
    assert_eq!(
        win.app.typed_statement(),
        Some("select *\nfrom california_housing")
    );
    let two = win.app.typed_card_rect().unwrap();
    assert!(two.height() > one.height());
    for _ in 0..80 {
        win.key(egui::Key::Enter);
    }
    let map = win.app.canvas_panes().pane("map").unwrap().rect;
    assert!(map.contains_rect(win.app.typed_card_rect().unwrap()));
}

#[test]
fn the_card_stands_clear_of_the_grid_with_no_scrim() {
    let (mut win, _f) = Window::housing(&TempDir::new("scrim"));
    win.key(egui::Key::Space);
    // The palette is a modal layer, so its frame paints the scrim: what tells
    // this check from one that could never see a scrim at all.
    assert!(win.painted_a_scrim(), "the palette dims the page");
    win.type_text("select * from california_housing");
    let card = win.app.typed_card_rect().unwrap();
    let grid = win.app.canvas_panes().pane("grid").unwrap().rect;
    assert!(!card.intersects(grid));
    assert!(
        !win.painted_a_scrim(),
        "a rect over the whole screen in the scrim ink was painted"
    );
}

#[test]
fn the_grid_previews_the_statement_after_a_pause() {
    let (mut win, _f) = Window::housing(&TempDir::new("preview"));
    win.key(egui::Key::Space);
    win.type_text("select * from california_housing order by median_income desc");
    assert!(win.app.finish_typed_reads(Duration::from_secs(60)));
    win.settle();
    assert_eq!(win.preview_cells("median_income")[0], "15.0001");
    assert_eq!(win.app.typed_said().as_deref(), Some("a SQL step"));
    assert!(win.drew("Grid \u{b7} california_housing_sql"));
}

#[test]
fn a_filter_of_the_loaded_table_says_how_many_rows_it_keeps() {
    let (mut win, _f) = Window::housing(&TempDir::new("filter"));
    win.key(egui::Key::Space);
    win.type_text("select * from california_housing where house_age > 28");
    assert!(win.app.finish_typed_reads(Duration::from_secs(60)));
    win.settle();
    assert_eq!(
        win.app.typed_said().as_deref(),
        Some("filter-rows on california_housing \u{b7} 8,831 rows")
    );
}

#[test]
fn the_filter_row_names_the_loaded_table_as_it_loaded() {
    let (mut win, _f) = Window::housing(&TempDir::new("spelling"));
    win.key(egui::Key::Space);
    win.type_text("select * from California_Housing where house_age > 28");
    assert!(win.app.finish_typed_reads(Duration::from_secs(60)));
    win.settle();
    assert_eq!(
        win.app.typed_said().as_deref(),
        Some("filter-rows on california_housing \u{b7} 8,831 rows")
    );
}

#[test]
fn a_click_on_the_grid_beside_the_card_leaves_the_keys_with_the_statement() {
    let (mut win, _f) = Window::housing(&TempDir::new("click"));
    win.key(egui::Key::Space);
    win.type_text("select *");
    let grid = win.app.canvas_panes().pane("grid").unwrap().rect;
    win.click(grid.center());
    win.type_text(" from california_housing");
    assert_eq!(
        win.app.typed_statement(),
        Some("select * from california_housing")
    );
}

#[test]
fn a_refused_statement_keeps_the_last_rows_and_shows_duckdbs_reason() {
    let (mut win, _f) = Window::housing(&TempDir::new("refused"));
    win.key(egui::Key::Space);
    win.type_text("select * from california_housing order by median_income desc");
    win.app.finish_typed_reads(Duration::from_secs(60));
    win.settle();
    win.type_text(" nonsense nonsense");
    win.app.finish_typed_reads(Duration::from_secs(60));
    win.settle();
    assert_eq!(win.preview_cells("median_income")[0], "15.0001");
    assert!(
        win.app.typed_said().unwrap().contains("rror"),
        "DuckDB's words are shown"
    );
}

/// An edit stops the read under way even when it returns to text already
/// answered, so the held read of the text in between never lands and the grid
/// keeps previewing the text on the line.
#[test]
fn an_edit_back_to_answered_text_stops_the_read_in_between() {
    let (mut win, _f) = Window::housing(&TempDir::new("back"));
    win.key(egui::Key::Space);
    win.type_text("select * from california_housing order by median_income desc");
    assert!(win.app.finish_typed_reads(Duration::from_secs(60)));
    win.settle();
    assert_eq!(win.preview_cells("median_income")[0], "15.0001");
    win.app.chart_doc().hold_preview_reads(true);
    for _ in 0.." desc".len() {
        win.key(egui::Key::Backspace);
    }
    assert_eq!(
        win.app.typed_statement(),
        Some("select * from california_housing order by median_income")
    );
    let before = win.app.typed_reads_superseded().unwrap();
    win.app.finish_typed_reads(Duration::from_millis(100));
    win.type_text(" desc");
    assert_eq!(
        win.app.typed_reads_superseded(),
        Some(before + 1),
        "going back to the answered text stops the read in between"
    );
    win.app.chart_doc().hold_preview_reads(false);
    std::thread::sleep(Duration::from_secs(2));
    win.settle();
    assert_eq!(win.preview_cells("median_income")[0], "15.0001");
    assert_eq!(win.app.typed_said().as_deref(), Some("a SQL step"));
}

#[test]
fn esc_while_a_read_is_in_flight_drops_its_answer() {
    let (mut win, _f) = Window::housing(&TempDir::new("esc"));
    win.app.chart_doc().hold_preview_reads(true);
    win.key(egui::Key::Space);
    win.type_text("select * from california_housing order by house_age");
    win.app.finish_typed_reads(Duration::from_millis(100));
    win.key(egui::Key::Escape);
    win.app.chart_doc().hold_preview_reads(false);
    std::thread::sleep(Duration::from_secs(2));
    win.settle();
    assert_eq!(win.app.open_overlay(), None);
    assert!(win.app.chart_doc().grid_preview().is_none());
    assert!(!win.app.carries_unsaved_mark());
}

#[test]
fn a_window_without_a_data_file_reads_select_as_a_search() {
    let mut win = Window::over_chart_spec("examples/dashboard.yaml");
    win.key(egui::Key::Space);
    win.type_text("select");
    assert_eq!(win.app.typed_statement(), None);
}
