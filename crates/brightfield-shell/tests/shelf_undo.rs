//! **`u` takes back the last column kept from the shelf, and the status band
//! names it.**
//!
//! A column kept from the shelf's list is one edit; `u` takes the last one back,
//! the hero drawn again from the spec it was kept onto. It acts with the list
//! open, with the band holding the keys and with the pane holding them, and
//! `⌘Z` acts from the list's query line, where a bare letter is text. The status
//! band names the last kept column in the shelf's words at its leading end,
//! before the row count, with the key beside it, and a click there sends the same verb. A
//! Save puts what it wrote beyond the reach of `u`.
//!
//! Every assertion reads what a frame drew: the hero's composition, the text the
//! frame put on the screen, the window title — or, for Save, the chart file on
//! disk.

use brightfield_render::channel::Channel;
use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::startup::default_layout;
use brightfield_shell::text_ink;
use brightfield_shell::window::{Boot, MeridianApp, SHELF_EDIT_STATUS_ID, UNSAVED_MARK};
use brightfield_spec::layout::{PlotAxis, ScaleType};
use brightfield_workbench::{PaneKey, Verb};

const HOUSING_FILE: &str = "california_housing_sample.csv";
const INCOME: &str = "median_income";
const VALUE: &str = "median_house_value";
const LON: &str = "longitude";
const LAT: &str = "latitude";

/// The words the band names for `median_income` kept on x of the generated map.
const X_WORDS: &str = "x axis: longitude \u{2192} median_income";
/// …and for `median_house_value` kept on colour, which held no column.
const COLOUR_WORDS: &str = "colour: none \u{2192} median_house_value";

fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(HOUSING_FILE)
}

/// A directory of this test's own, removed when the test ends: a Save writes
/// beside the data file, and the data file is copied here so that is not the
/// repository.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-shelf-undo-{name}-{}-{}",
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

fn key_down(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
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

/// One headless window over a copy of the housing sample, the hero's pane
/// focused.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The data file's folder, where Save writes `panels/`.
    folder: std::path::PathBuf,
    /// The text the frame before drew.
    texts: Vec<text_ink::DrawnText>,
    /// Held last, so the folder outlives the window.
    _root: TempDir,
}

impl Window {
    fn housing(name: &str) -> Self {
        let root = TempDir::new(name);
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(HOUSING_FILE);
        std::fs::copy(housing(), &data).expect("the housing fixture copies");
        let boot = Boot::data_file(data.to_str().expect("utf-8 path")).expect("the sample opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            folder,
            texts: Vec::new(),
            _root: root,
        };
        win.settle();
        assert!(
            win.app.focus_pane(PaneKey::new(CHART)),
            "the dashboard's pane takes focus"
        );
        win.settle();
        win
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events,
            ..Default::default()
        };
        let mut texts = Vec::new();
        let _ = self.ctx.run_ui(raw, |ui| {
            self.app.draw(ui);
            texts = text_ink::frame_text(ui.ctx());
        });
        self.texts = texts;
    }

    fn settle(&mut self) {
        for _ in 0..3 {
            self.run(Vec::new());
        }
    }

    /// Press a letter, as a keyboard brings it, and let the frame after run.
    fn type_letter(&mut self, key: egui::Key, text: &str) {
        self.run(vec![
            key_down(key, egui::Modifiers::NONE),
            egui::Event::Text(text.to_owned()),
        ]);
        self.run(Vec::new());
    }

    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_down(key, egui::Modifiers::NONE)]);
        self.run(Vec::new());
    }

    /// `⌘Z`, as a keyboard brings it: the key with the command held and no text.
    fn cmd_z(&mut self) {
        self.run(vec![key_down(egui::Key::Z, egui::Modifiers::COMMAND)]);
        self.run(Vec::new());
    }

    /// A click at `at`: the pointer moved there, pressed, released, and the
    /// frames after it settled.
    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.run(vec![button(at, true)]);
        self.run(vec![button(at, false)]);
        self.settle();
    }

    /// The column under the open list's cursor.
    fn cursor(&self) -> Option<String> {
        self.app
            .protocol_model()
            .column_list()
            .and_then(|l| l.cursor().map(str::to_owned))
    }

    /// Whether the Outline's list is open.
    fn list_is_open(&self) -> bool {
        self.app.protocol_model().column_list().is_some()
    }

    /// Move the open list's cursor to `column` by its keys, as many presses as
    /// it takes.
    fn walk_to(&mut self, column: &str, key: egui::Key, text: &str) {
        for _ in 0..12 {
            if self.cursor().as_deref() == Some(column) {
                return;
            }
            self.type_letter(key, text);
        }
        panic!(
            "the list's cursor did not reach {column} by `{text}`: it is on {:?}",
            self.cursor()
        );
    }

    /// The column the hero's top layer draws on `channel`, as the composition
    /// that ran records it.
    fn hero_column(&self, channel: Channel) -> Option<String> {
        self.app
            .chart_doc()
            .composed
            .plots
            .first()
            .expect("the page composed its hero")
            .hover
            .as_ref()
            .and_then(|layer| layer.column(channel).map(str::to_owned))
    }

    /// Whether the hero is drawn as the map: through a projection.
    fn hero_is_a_map(&self) -> bool {
        self.app
            .chart_doc()
            .composed
            .plots
            .first()
            .expect("the page composed its hero")
            .scales
            .projection()
            .is_some()
    }

    /// Whether the window title carries the unsaved mark.
    fn marked_unsaved(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    /// `e`, `x`, the cursor on `median_income`, `Enter`: `median_income` kept on
    /// x, the list closed and the band holding the keys.
    fn keep_income_on_x(&mut self) {
        self.type_letter(egui::Key::E, "e");
        self.type_letter(egui::Key::X, "x");
        assert_eq!(
            self.cursor().as_deref(),
            Some(LON),
            "x's list opens on x's column"
        );
        self.walk_to(INCOME, egui::Key::K, "k");
        self.press(egui::Key::Enter);
        assert_eq!(
            self.hero_column(Channel::X).as_deref(),
            Some(INCOME),
            "Enter did not keep median_income on x"
        );
    }

    /// With the band holding the keys, `c` opens colour's list and `Enter` on
    /// `median_house_value` keeps it.
    fn keep_value_on_colour(&mut self) {
        self.type_letter(egui::Key::C, "c");
        self.walk_to(VALUE, egui::Key::J, "j");
        self.press(egui::Key::Enter);
        assert_eq!(
            self.hero_column(Channel::Fill).as_deref(),
            Some(VALUE),
            "Enter did not keep median_house_value on colour"
        );
    }

    /// The text the last frame drew inside the status band, left to right, with
    /// where each began.
    fn status_text(&self) -> Vec<(f32, String)> {
        let band = self.app.rail().rect.expect("the status band drew");
        let mut drawn: Vec<(f32, String)> = self
            .texts
            .iter()
            .filter(|t| band.contains_rect(t.visible) && t.visible.is_positive())
            .map(|t| (t.visible.left(), t.text.clone()))
            .collect();
        drawn.sort_by(|a, b| a.0.total_cmp(&b.0));
        drawn
    }

    /// The generated text a first Save places its edits into.
    fn generated_text(&self) -> String {
        let path = self
            .app
            .chart_doc()
            .spec_path
            .clone()
            .expect("a generated dashboard carries its spec file");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read the generated spec {}: {e}", path.display()))
    }

    /// Save through the window's own entry, as a test of Save does.
    fn save(&mut self) {
        let ctx = self.ctx.clone();
        self.app
            .save_protocol(&ctx)
            .expect("a data file's window has a Protocol to save")
            .expect("the Protocol saves");
        self.settle();
    }

    /// The chart file Save wrote.
    fn written(&self) -> String {
        let chart = std::fs::read_dir(self.folder.join("panels"))
            .expect("Save wrote a panels folder")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "yaml"))
            .expect("Save wrote a chart file");
        std::fs::read_to_string(&chart).expect("the chart file reads")
    }
}

/// The map as the generator drew it: x is longitude and y latitude, through a
/// projection, nothing paints it, and the title carries no mark.
fn assert_the_map(win: &Window, when: &str) {
    assert!(win.hero_is_a_map(), "{when}: the hero is not the map");
    assert_eq!(
        win.hero_column(Channel::X).as_deref(),
        Some(LON),
        "{when}: the hero's x"
    );
    assert_eq!(
        win.hero_column(Channel::Y).as_deref(),
        Some(LAT),
        "{when}: the hero's y"
    );
    assert_eq!(
        win.hero_column(Channel::Fill),
        None,
        "{when}: the hero's colour"
    );
}

/// The generated text with `median_income` placed on x by the writer Save
/// calls: each layer's `x:` reads it and the projection line is gone.
fn with_income_on_x(generated: &str) -> String {
    generated
        .replace("        x: 'longitude'\n", "        x: 'median_income'\n")
        .replace("      projectionType: equirectangular\n", "")
}

/// **AC1.** After `median_income` is kept on x and `median_house_value` on
/// colour, `u` takes back the colour — the hero is drawn without it and still
/// on x — and a second `u` takes back x, the hero the map again. A third has
/// nothing to take back and leaves the map as it is.
#[test]
fn u_takes_back_the_colour_and_a_second_u_takes_back_x() {
    let mut win = Window::housing("two");
    win.keep_income_on_x();
    win.keep_value_on_colour();
    assert!(win.app.shelf_holds_keys(), "the band let go of the keys");

    win.type_letter(egui::Key::U, "u");
    assert_eq!(
        win.hero_column(Channel::Fill),
        None,
        "u left the colour on the hero"
    );
    assert_eq!(
        win.hero_column(Channel::X).as_deref(),
        Some(INCOME),
        "the first u took x back with the colour"
    );
    assert!(
        win.marked_unsaved(),
        "x is still kept, and the title lost its mark"
    );

    win.type_letter(egui::Key::U, "u");
    assert_the_map(&win, "after the second u");
    assert!(
        !win.marked_unsaved(),
        "nothing is kept and the title is marked"
    );

    win.type_letter(egui::Key::U, "u");
    assert_the_map(&win, "after a third u with nothing to take back");
}

/// **AC1, the band follows.** The cells name what the spec binds after `u`: x's
/// cell reads `longitude` again, and the colour cell reads no column.
#[test]
fn the_band_names_the_column_u_took_back_to() {
    use brightfield_workbench::channel::ShelfChannel;
    let mut win = Window::housing("cells");
    win.keep_income_on_x();
    win.type_letter(egui::Key::U, "u");
    let cell = win.app.shelf_drawn().expect("the band drew").cells[ShelfChannel::X.index()];
    let named: Vec<&str> = win
        .texts
        .iter()
        .filter(|t| cell.contains_rect(t.visible) && t.visible.is_positive())
        .map(|t| t.text.as_str())
        .collect();
    assert!(
        named.contains(&LON) && !named.contains(&INCOME),
        "x's cell drew {named:?} after u, not longitude alone"
    );
}

/// **AC2, the list open.** `u` with x's list open takes back the kept column:
/// the hero is the map, the list stays open and its cursor is on the column x
/// holds again. A column the cursor has moved to, drawn as a preview, is backed
/// out of, not taken as the edit.
#[test]
fn u_acts_with_the_list_open_and_takes_back_the_kept_column_not_the_preview() {
    let mut win = Window::housing("list");
    win.keep_income_on_x();
    win.type_letter(egui::Key::X, "x");
    assert_eq!(win.cursor().as_deref(), Some(INCOME));
    win.walk_to("avg_occupancy", egui::Key::J, "j");
    assert_eq!(
        win.hero_column(Channel::X).as_deref(),
        Some("avg_occupancy"),
        "the cursor's column is not previewed"
    );

    win.type_letter(egui::Key::U, "u");
    assert_the_map(&win, "after u in the list");
    assert!(
        win.app.chart_doc().shelf_preview().is_none(),
        "the preview was left standing"
    );
    assert!(win.list_is_open(), "u closed the list");
    assert_eq!(
        win.cursor().as_deref(),
        Some(LON),
        "the list's cursor did not go to the column x holds again"
    );
    assert!(!win.marked_unsaved());
}

/// **AC2, the band at rest.** With the pane holding the keys — `Esc` handed
/// them back from the band — `u` takes the kept column back.
#[test]
fn u_acts_with_the_band_at_rest() {
    let mut win = Window::housing("rest");
    win.keep_income_on_x();
    win.press(egui::Key::Escape);
    assert!(
        !win.app.shelf_holds_keys(),
        "Esc left the band holding the keys"
    );
    assert_eq!(win.hero_column(Channel::X).as_deref(), Some(INCOME));

    win.type_letter(egui::Key::U, "u");
    assert_the_map(&win, "after u with the band at rest");
    assert!(!win.marked_unsaved());
}

/// **AC2, `⌘Z` from the query.** In the query line a `u` is text and takes
/// nothing back; `⌘Z` does, and the list stays open.
#[test]
fn cmd_z_acts_from_the_query_line_where_a_u_is_typed() {
    let mut win = Window::housing("query");
    win.keep_income_on_x();
    win.type_letter(egui::Key::X, "x");
    win.type_letter(egui::Key::Slash, "/");
    assert!(
        win.app
            .protocol_model()
            .column_list()
            .is_some_and(|l| l.querying()),
        "/ did not give the keys to the query"
    );

    win.type_letter(egui::Key::U, "u");
    assert_eq!(
        win.app.protocol_model().column_list().map(|l| l.query()),
        Some("u"),
        "a u in the query is not typed"
    );
    // The query's `u` moved the cursor to a column that has one, which the hero
    // draws as a preview over the kept column: the kept edit is what `u` would
    // have taken back.
    assert_eq!(
        win.app.chart_doc().last_shelf_edit(),
        Some(X_WORDS),
        "a u typed in the query took the column back"
    );

    win.cmd_z();
    assert_the_map(&win, "after cmd-Z from the query");
    assert!(
        win.app.chart_doc().shelf_preview().is_none(),
        "the preview the query's cursor drew was left standing"
    );
    assert_eq!(win.app.chart_doc().last_shelf_edit(), None);
    assert!(win.list_is_open(), "cmd-Z closed the list");
    assert!(!win.marked_unsaved());
}

/// **AC3.** After `u` takes back the only edit since the last Save the title
/// carries no unsaved mark, and a Save writes the chart file the generator's
/// text makes, with no trace of the edit.
#[test]
fn after_the_only_edit_is_taken_back_the_title_is_clean_and_save_writes_no_trace() {
    let mut win = Window::housing("only");
    let generated = win.generated_text();
    win.keep_income_on_x();
    assert!(win.marked_unsaved());
    win.type_letter(egui::Key::U, "u");
    assert!(
        !win.marked_unsaved(),
        "the title is marked after the only edit was taken back"
    );
    win.save();
    assert_eq!(
        win.written(),
        generated,
        "the chart file Save wrote is not the generator's text"
    );
}

/// **AC3, only the edit taken back goes.** Colour is kept after x and taken
/// back; the title stays marked for x, and a Save writes x's edit alone — no
/// `fill`, scheme or legend for the colour, which is more than one edit.
#[test]
fn save_after_taking_back_the_colour_writes_the_x_edit_alone() {
    let mut win = Window::housing("colour");
    let generated = win.generated_text();
    win.keep_income_on_x();
    win.keep_value_on_colour();
    win.type_letter(egui::Key::U, "u");
    assert!(win.marked_unsaved(), "x is kept and the title is clean");
    win.save();
    assert_eq!(
        win.written(),
        with_income_on_x(&generated),
        "the chart file is not the generated text with the x edit alone"
    );
}

/// **AC4.** The status band names the last kept column in the shelf's words,
/// after the row count it leads with and with the key beside it; a second kept
/// column is named in place of the first; and a click on the line takes the
/// column back, which names the one before.
#[test]
fn the_status_band_names_the_last_kept_column_and_a_click_takes_it_back() {
    let mut win = Window::housing("band");
    assert!(
        !win.app.rail().drawn.contains(&SHELF_EDIT_STATUS_ID),
        "the band names an edit before one is kept"
    );

    win.keep_income_on_x();
    let key = Verb::new("undo")
        .keys()
        .expect("the registry binds a key to undo");
    let line = format!("{X_WORDS} \u{b7} {key} undo");
    assert!(
        win.app.rail().drawn.contains(&SHELF_EDIT_STATUS_ID),
        "the band drew no line for the edit: {:?}",
        win.app.rail().drawn
    );
    let drawn = win.status_text();
    let at = drawn
        .iter()
        .position(|(_, t)| *t == line)
        .unwrap_or_else(|| panic!("the band drew {drawn:?}, not {line:?}"));
    let counted = drawn
        .iter()
        .position(|(_, t)| t.contains(" rows"))
        .unwrap_or_else(|| panic!("the band drew no row count in {drawn:?}"));
    assert!(
        at < counted,
        "the edit is not named at the band's leading end, before the row count: {drawn:?}"
    );

    win.keep_value_on_colour();
    let drawn = win.status_text();
    let line = format!("{COLOUR_WORDS} \u{b7} {key} undo");
    assert!(
        drawn.iter().any(|(_, t)| *t == line),
        "the band drew {drawn:?}, not the colour edit {line:?}"
    );
    assert!(
        !drawn.iter().any(|(_, t)| t.starts_with("x axis")),
        "the band still names the earlier edit: {drawn:?}"
    );

    let control = win
        .app
        .rail()
        .controls
        .iter()
        .find(|c| c.name.starts_with("undo"))
        .unwrap_or_else(|| panic!("the band offers no undo: {:?}", win.app.rail().controls))
        .rect;
    win.click(control.center());
    assert_eq!(
        win.hero_column(Channel::Fill),
        None,
        "a click on the edit's line left the colour on the hero"
    );
    assert_eq!(win.hero_column(Channel::X).as_deref(), Some(INCOME));
    let drawn = win.status_text();
    let line = format!("{X_WORDS} \u{b7} {key} undo");
    assert!(
        drawn.iter().any(|(_, t)| *t == line),
        "after the click the band drew {drawn:?}, not the edit before it"
    );

    let control = win
        .app
        .rail()
        .controls
        .iter()
        .find(|c| c.name.starts_with("undo"))
        .expect("the band still offers undo")
        .rect;
    win.click(control.center());
    assert_the_map(&win, "after the second click");
    assert!(
        !win.app.rail().drawn.contains(&SHELF_EDIT_STATUS_ID),
        "the band names an edit when none is kept"
    );
}

/// **AC5.** After a Save, `u` takes back no edit made before it: the column
/// stays, the title stays clean and the file keeps what it was written with. An
/// edit kept after the Save is taken back, and a second `u` goes no further.
#[test]
fn after_a_save_u_takes_back_nothing_made_before_it() {
    let mut win = Window::housing("saved");
    let generated = win.generated_text();
    win.keep_income_on_x();
    win.save();
    assert!(!win.marked_unsaved());
    let saved = win.written();
    assert_eq!(saved, with_income_on_x(&generated));

    win.type_letter(egui::Key::U, "u");
    assert_eq!(
        win.hero_column(Channel::X).as_deref(),
        Some(INCOME),
        "u took back an edit a Save had written"
    );
    assert!(!win.marked_unsaved(), "u marked a saved window unsaved");
    assert!(
        !win.app.rail().drawn.contains(&SHELF_EDIT_STATUS_ID),
        "the band names an edit that is in the file"
    );

    win.keep_value_on_colour();
    assert!(win.marked_unsaved());
    win.type_letter(egui::Key::U, "u");
    assert_eq!(win.hero_column(Channel::Fill), None);
    assert_eq!(win.hero_column(Channel::X).as_deref(), Some(INCOME));
    assert!(
        !win.marked_unsaved(),
        "the edit after the Save was taken back"
    );
    win.type_letter(egui::Key::U, "u");
    assert_eq!(
        win.hero_column(Channel::X).as_deref(),
        Some(INCOME),
        "a second u went back past the Save"
    );
    assert_eq!(win.written(), saved, "u changed the chart file on disk");
}

/// **A tile's switch seals what came before it.** The shelf's snapshots are
/// whole specs, so `u` after a switch would take the switch back with the
/// column and leave the switch's edit on the list Save writes. `u` takes back
/// nothing across it: the scale stays and the column stays.
#[test]
fn u_takes_back_nothing_across_a_tiles_switch() {
    let mut win = Window::housing("switch");
    win.keep_income_on_x();
    assert!(
        win.app
            .chart_doc_mut()
            .set_plot_scale(0, PlotAxis::X, ScaleType::Log),
        "the scale switch was refused"
    );
    win.settle();
    let before = win
        .app
        .chart_doc()
        .live_dashboard()
        .map(|l| l.spec().clone());

    win.type_letter(egui::Key::U, "u");
    assert_eq!(
        win.app.chart_doc().last_shelf_edit(),
        None,
        "the band names an edit that u cannot take back across a switch"
    );
    assert_eq!(
        win.app
            .chart_doc()
            .live_dashboard()
            .map(|l| l.spec().clone()),
        before,
        "u changed the spec across a switch"
    );
    assert!(win.marked_unsaved());
}
