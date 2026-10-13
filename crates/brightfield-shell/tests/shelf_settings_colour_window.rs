//! **In the window, colour's settings rows act on the chart they sit over: the
//! keys and the clicks write the scheme, the scale, the domain, the legend and
//! reverse, the marks and the legend are drawn again from them, and Save keeps
//! them.**
//!
//! `shelf_settings_colour.rs` drives the list alone. What the list cannot say by
//! itself is this file's: that `Tab` on colour's cell reaches its rows, that a
//! step of the scheme row recomposes the hero with the scheme's stops and the
//! legend beside it with the same, that a click on a ramp in the strip does the
//! same through the card hung from the cell, that `reverse` runs the ramp end for
//! end (and the categories of a column of names from the last), and that a chart
//! Saved and opened again draws each value it was left with.
//!
//! Every read is of what the window composed or drew, or of the bytes Save wrote:
//! the hero's fill scale, the legend the page draws from it, the card's drawn
//! rows and strip, and the chart file.

use std::path::PathBuf;

use brightfield_protocol::layout::Flow;
use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, SequentialScheme};
use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{self, LegendSpec};
use brightfield_shell::shelf::{
    Binding, CardDrawn, ListTab, SchemeDrawn, SettingRow, SettingRowDrawn, DOMAIN_ROW, LEGEND_ROW,
    PIVOT_ROW, REVERSE_ROW, SCALE_ROW, SCHEME_ROW, STEPS_ROW,
};
use brightfield_shell::startup::default_layout;
use brightfield_shell::text_ink;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_workbench::arrangement;
use brightfield_workbench::PaneKey;

const HOUSING_FILE: &str = "california_housing_sample.csv";
const VALUE: &str = "median_house_value";
const NAMES: &str = "ocean_proximity";

/// The sample's four classes of `ocean_proximity`, dealt to its rows in turn.
const CLASSES: [&str; 4] = ["INLAND", "NEAR BAY", "NEAR OCEAN", "ISLAND"];

fn housing() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(HOUSING_FILE)
}

/// A directory of this test's own, removed when the test ends: a Save writes
/// beside the data file, and the data file is copied here so that is not the
/// repository.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-shelf-colour-{name}-{}-{}",
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

fn key_down(key: egui::Key) -> egui::Event {
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

/// One headless window over a copy of the housing sample, the hero's pane
/// focused.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    /// The data file's folder, where Save writes `arcform.yaml` and `panels/`.
    folder: PathBuf,
    /// The text the frame before drew.
    texts: Vec<text_ink::DrawnText>,
    /// Held last, so the folder outlives the window.
    _root: TempDir,
}

impl Window {
    /// A window over a copy of the sample in a folder of `name`'s own.
    fn housing(name: &str) -> Self {
        Self::over(name, false)
    }

    /// A window over a copy of the sample with a column of names, `ocean_proximity`,
    /// dealt four classes in turn down its rows.
    fn housing_with_names(name: &str) -> Self {
        Self::over(name, true)
    }

    fn over(name: &str, with_names: bool) -> Self {
        let root = TempDir::new(name);
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(HOUSING_FILE);
        if with_names {
            let sample = std::fs::read_to_string(housing()).expect("the housing sample reads");
            let mut lines = sample.lines().filter(|l| !l.trim().is_empty());
            let mut out = format!("{},{NAMES}\n", lines.next().expect("a header"));
            for (i, line) in lines.enumerate() {
                out.push_str(&format!("{line},{}\n", CLASSES[i % CLASSES.len()]));
            }
            std::fs::write(&data, out).expect("the fixture writes");
        } else {
            std::fs::copy(housing(), &data).expect("the housing fixture copies");
        }
        let boot = Boot::data_file(data.to_str().expect("utf-8 path")).expect("the sample opens");
        Self::around(boot, folder, root)
    }

    fn around(boot: Boot, folder: PathBuf, root: TempDir) -> Self {
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

    /// Close this window and open the Protocol it saved, into a new one.
    fn reopen(self) -> Self {
        let manifest = self
            .folder
            .join("arcform.yaml")
            .to_str()
            .expect("utf-8 path")
            .to_owned();
        let Self {
            folder,
            _root: root,
            ..
        } = self;
        let boot = Boot::open(&manifest, Flow::Vertical, None)
            .unwrap_or_else(|e| panic!("reopen {manifest}: {e}"));
        Self::around(boot, folder, root)
    }

    /// This window with its navigator rail shut, as a click on the collapse
    /// control shuts it: the list is then drawn as a card hung from the cell.
    fn rail_shut(mut self) -> Self {
        let at = self
            .app
            .rail_collapse_rect(arrangement::NAVIGATOR_RAIL)
            .expect("the navigator rail drew a collapse control")
            .center();
        self.click(at);
        assert!(
            self.app.rail_is_collapsed(arrangement::NAVIGATOR_RAIL),
            "a click on the collapse control did not shut the navigator rail"
        );
        self
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
        self.run(vec![key_down(key), egui::Event::Text(text.to_owned())]);
        self.run(Vec::new());
    }

    /// Press a key that makes no text, and let the frame after run.
    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_down(key)]);
        self.run(Vec::new());
    }

    /// A click at `at`: the pointer moved there, pressed, released, and the
    /// frames after it settled.
    fn click(&mut self, at: egui::Pos2) {
        self.run(vec![egui::Event::PointerMoved(at)]);
        self.settle();
        self.run(vec![button(at, true)]);
        self.run(vec![button(at, false)]);
        self.settle();
    }

    /// `e` and then the channel's letter: the keys that open a cell.
    fn open_cell(&mut self, key: egui::Key, letter: &str) {
        self.type_letter(egui::Key::E, "e");
        self.type_letter(key, letter);
    }

    /// Keep `column` on colour by the band's keys: the list's cursor walked down
    /// to it, and `Enter`.
    fn keep_on_colour(&mut self, column: &str) {
        self.open_cell(egui::Key::C, "c");
        for _ in 0..24 {
            if self.cursor().as_deref() == Some(column) {
                break;
            }
            self.type_letter(egui::Key::J, "j");
        }
        assert_eq!(
            self.cursor().as_deref(),
            Some(column),
            "the list's cursor did not reach {column}"
        );
        self.press(egui::Key::Enter);
        assert_eq!(
            self.fill_column().as_deref(),
            Some(column),
            "Enter did not keep {column} on colour"
        );
    }

    /// Open colour's list on its settings, the cursor on the scheme row.
    fn open_colour_settings(&mut self) {
        self.open_cell(egui::Key::C, "c");
        self.press(egui::Key::Tab);
        assert_eq!(
            self.list_tab(),
            Some(ListTab::Settings),
            "Tab did not turn colour's list to its settings"
        );
        assert_eq!(self.setting_cursor(), Some(SCHEME_ROW));
    }

    /// Walk the open list's cursor down to the settings row `name`.
    fn move_to_row(&mut self, name: &str) {
        for _ in 0..12 {
            if self.setting_cursor() == Some(name) {
                return;
            }
            self.type_letter(egui::Key::J, "j");
        }
        panic!("the settings cursor did not reach {name}");
    }

    /// Find the settings row `name` by typing its name after `/`, and `Enter`.
    fn find_row(&mut self, name: &str) {
        self.type_letter(egui::Key::Slash, "/");
        for letter in name.chars() {
            let key = egui::Key::from_name(&letter.to_ascii_uppercase().to_string())
                .unwrap_or_else(|| panic!("no key for {letter}"));
            self.type_letter(key, &letter.to_string());
        }
        self.press(egui::Key::Enter);
        assert_eq!(
            self.setting_cursor(),
            Some(name),
            "typing {name} did not find its row"
        );
    }

    fn list_tab(&self) -> Option<ListTab> {
        self.app.protocol_model().column_list().map(|l| l.tab())
    }

    fn cursor(&self) -> Option<String> {
        self.app
            .protocol_model()
            .column_list()
            .and_then(|l| l.cursor().map(str::to_owned))
    }

    fn setting_cursor(&self) -> Option<&'static str> {
        self.app
            .protocol_model()
            .column_list()
            .and_then(|l| l.setting_cursor().map(|r| r.name))
    }

    /// The names of the open list's head settings rows, in order.
    fn row_names(&self) -> Vec<&'static str> {
        self.app
            .protocol_model()
            .column_list()
            .expect("a list is open")
            .settings()
            .iter()
            .filter(|r| !r.by_name)
            .map(|r| r.name)
            .collect()
    }

    /// The open list's settings row `name`, as it reads now.
    fn row(&self, name: &str) -> SettingRow {
        self.app
            .protocol_model()
            .column_list()
            .expect("a list is open")
            .settings()
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("the list has no {name} row"))
            .clone()
    }

    /// The card the last frame hung from the cell.
    fn card(&self) -> &CardDrawn {
        self.app
            .shelf_card_drawn()
            .expect("the last frame hung a card")
    }

    /// The settings row `name` as the card drew it.
    fn row_drawn(&self, name: &str) -> &SettingRowDrawn {
        self.card()
            .list
            .settings
            .iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("the card drew no {name} row"))
    }

    /// The strip's entry for `scheme`, as the card drew it.
    fn scheme_drawn(&self, scheme: SequentialScheme) -> &SchemeDrawn {
        self.row_drawn(SCHEME_ROW)
            .schemes
            .iter()
            .find(|s| s.scheme == Some(scheme))
            .unwrap_or_else(|| panic!("the strip drew no {}", scheme.wire_name()))
    }

    /// The schemes of the strip that wear the ring, as the card drew it.
    fn ringed(&self) -> Vec<SequentialScheme> {
        self.row_drawn(SCHEME_ROW)
            .schemes
            .iter()
            .filter(|s| s.ring.is_some())
            .filter_map(|s| s.scheme)
            .collect()
    }

    /// The text the last frame drew inside `rect`, joined by spaces in the order
    /// the frame drew it.
    fn text_in(&self, rect: egui::Rect) -> String {
        self.texts
            .iter()
            .filter(|t| rect.contains_rect(t.visible) && t.visible.is_positive())
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The hero's plot, as the page composed it.
    fn hero(&self) -> &brightfield_shell::pipeline::PlotHandle {
        self.app
            .chart_doc()
            .composed
            .plots
            .first()
            .expect("the page composed its hero")
    }

    /// The column the hero's fill is drawn from.
    fn fill_column(&self) -> Option<String> {
        self.hero().fill_column.clone()
    }

    /// The stops of the hero's ramp, which the marks are painted along.
    fn fill_stops(&self) -> Vec<[f32; 4]> {
        match self.hero().scales.get(Channel::Fill) {
            Some(Scale::Sequential { stops, .. }) => stops.clone(),
            other => panic!("the hero's fill is not a ramp: {other:?}"),
        }
    }

    /// The stops of the ramp the page's legend draws.
    fn legend_stops(&self) -> Vec<[f32; 4]> {
        let blocks = legend::blocks(&self.app.chart_doc().composed);
        match blocks.as_slice() {
            [(_, LegendSpec::Sequential { stops, .. })] => stops.clone(),
            other => panic!("the page does not draw one ramp legend: {other:?}"),
        }
    }

    /// The categories and the palette of the hero's fill, drawn from a column of
    /// names.
    fn fill_categories(&self) -> (Vec<String>, Vec<[f32; 4]>) {
        match self.hero().scales.get(Channel::Fill) {
            Some(Scale::Colour {
                categories,
                palette,
            }) => (categories.clone(), palette.clone()),
            other => panic!("the hero's fill is not a set of names: {other:?}"),
        }
    }

    /// The categories the page's legend lists, in the order it lists them, each
    /// beside the colour it wears.
    fn legend_entries(&self) -> Vec<(String, [f32; 4])> {
        let blocks = legend::blocks(&self.app.chart_doc().composed);
        match blocks.as_slice() {
            [(_, LegendSpec::Categorical { entries })] => entries
                .iter()
                .map(|e| (e.label.clone(), e.colour))
                .collect(),
            other => panic!("the page does not draw one categorical legend: {other:?}"),
        }
    }

    /// The words the status band names the last kept shelf edit by.
    fn note(&self) -> Option<String> {
        self.app.chart_doc().last_shelf_edit().map(str::to_owned)
    }

    /// Whether the window title carries the unsaved mark.
    fn marked_unsaved(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    /// How many edits the chart holds unsaved.
    fn unsaved(&self) -> usize {
        self.app.chart_doc().unsaved_edits().len()
    }

    /// **Save, through the gesture a person has**: the chart palette on
    /// `space`, the verb typed, confirmed with enter.
    fn save(&mut self) {
        assert!(
            self.app.has_protocol_to_save(),
            "this window has no Protocol behind it to save"
        );
        // `Esc` backs out of the list, and again out of the band, and hands the
        // keys back to the pane, as a person leaving them does before reaching
        // for the palette.
        for _ in 0..4 {
            if self.app.protocol_model().column_list().is_none() && !self.app.shelf_holds_keys() {
                break;
            }
            self.press(egui::Key::Escape);
        }
        self.press(egui::Key::Space);
        assert_eq!(
            self.app.open_overlay(),
            Some("palette"),
            "space did not open the palette"
        );
        self.settle();
        self.run(vec![egui::Event::Text("save-spec".to_owned())]);
        self.run(Vec::new());
        self.press(egui::Key::Enter);
        assert_eq!(
            self.app.open_overlay(),
            None,
            "confirming save-spec did not close the palette"
        );
        self.settle();
    }

    /// The chart file Save wrote under `panels/`, as text.
    fn chart_text(&self) -> String {
        let chart = std::fs::read_dir(self.folder.join("panels"))
            .expect("Save wrote a panels folder")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "yaml"))
            .expect("Save wrote a chart file");
        std::fs::read_to_string(&chart).unwrap_or_else(|e| panic!("read {}: {e}", chart.display()))
    }
}

/// `stops` run from the other end.
fn flipped(stops: &[[f32; 4]]) -> Vec<[f32; 4]> {
    stops.iter().rev().copied().collect()
}

/// The lines of `text` that read `key: value`, whichever way the value is
/// quoted.
fn lines_of_key(text: &str, key: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| l.starts_with(&format!("{key}:")))
        .map(|l| l.replace(['\'', '"'], ""))
        .collect()
}

// ---------------------------------------------------------------------------
// AC1: Tab on colour's cell.
// ---------------------------------------------------------------------------

/// **`Tab` on colour's cell turns its list to the settings, whose head rows are
/// scheme, scale, legend and reverse in that order, each reading auto, with the sentence of the
/// row under the cursor at the foot.** In the Outline, and in the card a shut rail
/// hangs from the cell.
#[test]
fn tab_on_colour_shows_scheme_scale_legend_then_reverse_with_the_foot() {
    let mut win = Window::housing("tab");
    win.keep_on_colour(VALUE);
    win.open_colour_settings();
    assert_eq!(
        win.row_names(),
        [SCHEME_ROW, SCALE_ROW, LEGEND_ROW, REVERSE_ROW],
        "colour's head rows, in order"
    );
    for row in [SCHEME_ROW, SCALE_ROW, LEGEND_ROW, REVERSE_ROW] {
        assert!(
            !win.row(row).set,
            "{row} reads brightfield's own on a plot that sets nothing"
        );
    }
    assert_eq!(win.row(SCHEME_ROW).value, "viridis");
    assert_eq!(win.row(REVERSE_ROW).value, "off");

    // The same list in the card of a shut rail, which draws what the Outline
    // would, and the foot.
    let mut shut = Window::housing("tab-shut").rail_shut();
    shut.keep_on_colour(VALUE);
    shut.open_colour_settings();
    assert_eq!(shut.card().list.tab, ListTab::Settings);
    assert_eq!(
        shut.card()
            .list
            .settings
            .iter()
            .map(|r| r.name)
            .collect::<Vec<_>>(),
        [SCHEME_ROW, SCALE_ROW, LEGEND_ROW, REVERSE_ROW],
        "the card draws the scheme, scale, legend and reverse rows"
    );
    let sentence = shut.card().list.sentence.expect("the foot drew a sentence");
    assert_eq!(
        shut.text_in(sentence),
        "The ramp a number's colour runs along, which auto draws as viridis.",
        "the foot says what the scheme row does"
    );
    shut.type_letter(egui::Key::J, "j");
    assert_eq!(shut.setting_cursor(), Some(SCALE_ROW));
    shut.type_letter(egui::Key::J, "j");
    assert_eq!(shut.setting_cursor(), Some(LEGEND_ROW));
    shut.type_letter(egui::Key::J, "j");
    assert_eq!(shut.setting_cursor(), Some(REVERSE_ROW));
    let sentence = shut.card().list.sentence.expect("the foot drew a sentence");
    assert_eq!(
        shut.text_in(sentence),
        "Whether colour runs the other way, a ramp from its far end and names from the last, which auto does not.",
        "the foot follows the cursor to the reverse row"
    );
}

/// **`l` on the legend row moves the legend below the plot and then off the
/// page, and the rows keep reading the plot**: the plot's path changes when it
/// is wrapped in a `vconcat`, and the list is rebuilt for the plot where it now
/// is.
#[test]
fn the_legend_row_moves_the_legend_below_and_the_list_still_reads_the_plot() {
    let mut win = Window::housing("legend-below");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.move_to_row(LEGEND_ROW);
    assert_eq!(win.row(LEGEND_ROW).value, "right");
    assert!(!win.row(LEGEND_ROW).set);
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(LEGEND_ROW).value, "below");
    assert!(win.row(LEGEND_ROW).set, "below carries the dot");
    assert_eq!(
        win.row(SCALE_ROW).value,
        "linear",
        "the rows still read the plot"
    );
    assert!(
        win.hero().legend_below.is_some(),
        "the legend is under the plot"
    );
    assert_eq!(
        win.app.shelf_band().map(|b| b.channels().colour.clone()),
        Some(Binding::Column("median_income".to_string())),
        "the band still reads colour from the plot in the concat"
    );
    win.save();
    let text = win.chart_text();
    assert!(
        text.contains("for: "),
        "below is a legend for the plot: {text}"
    );

    win.open_colour_settings();
    win.move_to_row(LEGEND_ROW);
    assert_eq!(
        win.row(LEGEND_ROW).value,
        "below",
        "the file's placement is read"
    );
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(LEGEND_ROW).value, "none");
    assert!(win.hero().legend_below.is_none());
    win.save();
    let text = win.chart_text();
    assert!(!text.contains("legend: color"), "{text}");

    // Back to the right on a file that holds no legend writes one.
    win.open_colour_settings();
    win.move_to_row(LEGEND_ROW);
    win.type_letter(egui::Key::H, "h");
    win.type_letter(egui::Key::H, "h");
    assert_eq!(win.row(LEGEND_ROW).value, "right");
    assert!(!win.row(LEGEND_ROW).set);
    assert!(win.hero().legend_below.is_none());
    assert!(
        win.hero().legend_declared,
        "the hero draws its legend at the right"
    );
    win.save();
    let text = win.chart_text();
    assert!(text.contains("legend: color"), "{text}");
    assert!(!text.contains("for: "), "{text}");
}

/// **The note of a legend move names the placement the row moved to**: a move
/// below wraps the plot in a `vconcat` and a move from below unwraps it, and the
/// note reads the plot where it now is.
#[test]
fn the_note_of_a_legend_move_names_where_the_legend_went() {
    let mut win = Window::housing("legend-note");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.move_to_row(LEGEND_ROW);
    win.type_letter(egui::Key::L, "l");
    assert_eq!(
        win.note().as_deref(),
        Some("colour legend: right \u{2192} below")
    );
    win.type_letter(egui::Key::L, "l");
    assert_eq!(
        win.note().as_deref(),
        Some("colour legend: below \u{2192} none")
    );
    win.press(egui::Key::Backspace);
    assert_eq!(
        win.note().as_deref(),
        Some("colour legend: none \u{2192} right")
    );
}

/// **`⌫` on the legend row puts the legend back at the right**, from below and
/// from none, in the file and on the chart.
#[test]
fn backspace_on_the_legend_row_puts_the_legend_back_at_the_right() {
    for steps in [1, 2] {
        let mut win = Window::housing("legend-backspace");
        win.keep_on_colour("median_income");
        win.open_colour_settings();
        win.move_to_row(LEGEND_ROW);
        for _ in 0..steps {
            win.type_letter(egui::Key::L, "l");
        }
        let at = if steps == 1 { "below" } else { "none" };
        assert_eq!(win.row(LEGEND_ROW).value, at);
        win.press(egui::Key::Backspace);
        let row = win.row(LEGEND_ROW);
        assert_eq!((row.value.as_str(), row.set), ("right", false), "from {at}");
        assert!(win.hero().legend_below.is_none());
        assert!(win.hero().legend_declared, "from {at}");
        win.save();
        let text = win.chart_text();
        assert!(text.contains("legend: color"), "from {at}: {text}");
        assert!(!text.contains("for: "), "from {at}: {text}");
    }
}

/// **With nothing on colour the legend row is muted with its reason**, carries
/// no dot, and `l`, `h` and `⌫` on it write nothing.
#[test]
fn the_legend_row_is_muted_while_nothing_is_on_colour_and_writes_nothing() {
    let mut win = Window::housing("legend-muted");
    win.open_colour_settings();
    win.move_to_row(LEGEND_ROW);
    let row = win.row(LEGEND_ROW);
    assert_eq!(
        row.reason.as_deref(),
        Some("nothing is on colour, so there is no legend to place")
    );
    assert!(!row.set);
    let unsaved = win.unsaved();
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.unsaved(), unsaved, "l on a muted row is no edit");
    win.type_letter(egui::Key::H, "h");
    assert_eq!(win.unsaved(), unsaved, "h on a muted row is no edit");
    win.press(egui::Key::Backspace);
    assert_eq!(win.unsaved(), unsaved, "⌫ on a muted row is no edit");
    assert!(win.hero().legend_below.is_none());
}

/// **With nothing on colour the scale, domain, pivot and steps rows are muted
/// with their reasons**, and `l`, `h` and `⌫` on the scale and domain rows write
/// nothing.
#[test]
fn the_scale_and_domain_rows_are_muted_while_nothing_is_on_colour_and_write_nothing() {
    let mut win = Window::housing("scale-muted");
    win.open_colour_settings();
    for (name, reason) in [
        (
            SCALE_ROW,
            "nothing is on colour, so there is no scale to set",
        ),
        (
            DOMAIN_ROW,
            "nothing is on colour, so there are no ends to set",
        ),
        (
            PIVOT_ROW,
            "nothing is on colour, so there is no pivot to set",
        ),
        (
            STEPS_ROW,
            "nothing is on colour, so there are no steps to set",
        ),
    ] {
        let row = win.row(name);
        assert_eq!(row.reason.as_deref(), Some(reason), "{name}");
        assert!(!row.set, "{name} carries no dot");
    }
    let unsaved = win.unsaved();
    win.move_to_row(SCALE_ROW);
    for (key, text) in [(egui::Key::L, "l"), (egui::Key::H, "h")] {
        win.type_letter(key, text);
        assert_eq!(win.unsaved(), unsaved, "{text} on the muted scale row");
    }
    win.press(egui::Key::Backspace);
    assert_eq!(win.unsaved(), unsaved, "⌫ on the muted scale row");
    win.find_row(DOMAIN_ROW);
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.unsaved(), unsaved, "l on the muted domain row");
    win.press(egui::Key::Backspace);
    assert_eq!(win.unsaved(), unsaved, "⌫ on the muted domain row");
    assert_eq!(win.row(SCALE_ROW).value, "linear");
    assert_eq!(win.row(DOMAIN_ROW).value, "auto");
}

/// **`l` on the scale row steps it to diverging and writes the auto pivot**: the
/// row reads diverging with its pivot unmarked, and the file names the scale and
/// the pivot and no longer the scheme.
#[test]
fn stepping_the_scale_row_to_diverging_writes_it_with_the_auto_pivot() {
    let mut win = Window::housing("scale-diverging");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.type_letter(egui::Key::J, "j");
    assert_eq!(win.setting_cursor(), Some(SCALE_ROW));
    win.type_letter(egui::Key::L, "l");
    let row = win.row(SCALE_ROW);
    assert_eq!(row.value, "diverging");
    assert!(!row.parameter_set(), "the pivot written is the auto pivot");
    let facts = row.colour.expect("the scale row carries its facts");
    match win.hero().scales.get(Channel::Fill) {
        Some(Scale::Diverging { pivot, .. }) => {
            assert_eq!(
                Some(*pivot),
                facts.auto_pivot,
                "the chart redrew about the auto pivot"
            );
        }
        other => panic!("the hero's fill is not diverging: {other:?}"),
    }
    win.save();
    let text = win.chart_text();
    assert!(text.contains("colorScale: diverging"), "{text}");
    assert!(text.contains("colorPivot: "), "{text}");
    assert!(!text.contains("colorScheme"), "{text}");
}

/// **A colour scale write names the scale alone**: colour has no range, so the
/// note says nothing of one, though the write touches several keys.
#[test]
fn the_note_of_a_colour_scale_step_names_no_range() {
    let mut win = Window::housing("scale-note");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.move_to_row(SCALE_ROW);
    win.type_letter(egui::Key::L, "l");
    assert_eq!(
        win.note().as_deref(),
        Some("colour scale: linear \u{2192} diverging")
    );
}

/// **A column put on colour over a set domain names no range in its note**, though
/// the change takes the old column's domain out.
#[test]
fn the_note_of_a_colour_column_change_over_a_set_domain_names_no_range() {
    let mut win = Window::housing("column-note");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.type_letter(egui::Key::Slash, "/");
    for (key, text) in [
        (egui::Key::D, "d"),
        (egui::Key::O, "o"),
        (egui::Key::M, "m"),
    ] {
        win.type_letter(key, text);
    }
    win.press(egui::Key::Enter);
    assert_eq!(win.setting_cursor(), Some(DOMAIN_ROW));
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(DOMAIN_ROW).value, "fixed");
    for _ in 0..4 {
        if win.app.protocol_model().column_list().is_none() && !win.app.shelf_holds_keys() {
            break;
        }
        win.press(egui::Key::Escape);
    }
    win.keep_on_colour(VALUE);
    let note = win.note().expect("the column change left a note");
    assert!(!note.contains("range"), "{note}");
}

/// **`⌫` on the scale row from steps puts it back to linear**: the row reads
/// linear and auto, and the file carries neither `colorScale` nor `colorN`.
#[test]
fn backspace_on_the_scale_row_from_steps_takes_the_scale_and_its_count_out() {
    let mut win = Window::housing("scale-backspace");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.type_letter(egui::Key::J, "j");
    win.type_letter(egui::Key::L, "l");
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCALE_ROW).value, "steps");
    win.press(egui::Key::Backspace);
    let row = win.row(SCALE_ROW);
    assert_eq!((row.value.as_str(), row.set), ("linear", false));
    win.save();
    let text = win.chart_text();
    assert!(!text.contains("colorScale"), "{text}");
    assert!(!text.contains("colorN"), "{text}");
}

/// **`⌫` on the pivot row found by name puts the auto pivot back, by name**:
/// Mosaic's own pivot is 0, so the file keeps `colorPivot` at the auto value
/// rather than losing the key.
#[test]
fn backspace_on_the_pivot_row_writes_the_auto_pivot_and_keeps_the_key() {
    let mut win = Window::housing("pivot-backspace");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.move_to_row(SCALE_ROW);
    win.type_letter(egui::Key::L, "l");
    let auto = win
        .row(SCALE_ROW)
        .colour
        .and_then(|c| c.auto_pivot)
        .expect("the scale row knows its auto pivot");
    win.press(egui::Key::Enter);
    win.run(vec![egui::Event::Text("2".to_owned())]);
    win.press(egui::Key::Enter);
    assert!(win.row(SCALE_ROW).parameter_set(), "fixture check: pivot 2");
    win.find_row(PIVOT_ROW);
    win.press(egui::Key::Backspace);
    assert!(!win.row(PIVOT_ROW).set, "the pivot reads auto again");
    assert_eq!(win.row(SCALE_ROW).value, "diverging");
    win.save();
    let text = win.chart_text();
    let pivots = lines_of_key(&text, "colorPivot");
    let [line] = pivots.as_slice() else {
        panic!("the file names one pivot:\n{text}");
    };
    let written: f64 = line["colorPivot:".len()..]
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("{line}: {e}"));
    assert!(
        (written - auto).abs() < 1e-9,
        "the pivot written is the auto one, {auto}: {line}"
    );
    assert!(text.contains("colorScale: diverging"), "{text}");
}

/// **`⌫` on the steps row found by name puts the count back to five, by name**:
/// Mosaic's thresholds differ from the default's, so `colorN` stays written.
#[test]
fn backspace_on_the_steps_row_writes_five_and_keeps_the_key() {
    let mut win = Window::housing("steps-backspace");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.move_to_row(SCALE_ROW);
    win.type_letter(egui::Key::L, "l");
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCALE_ROW).value, "steps");
    win.press(egui::Key::Enter);
    win.run(vec![egui::Event::Text("7".to_owned())]);
    win.press(egui::Key::Enter);
    assert_eq!(win.row(SCALE_ROW).colour.and_then(|c| c.count), Some(7));
    win.find_row(STEPS_ROW);
    win.press(egui::Key::Backspace);
    assert_eq!(win.row(STEPS_ROW).value, "5");
    assert!(!win.row(STEPS_ROW).set);
    win.save();
    let text = win.chart_text();
    assert_eq!(lines_of_key(&text, "colorN"), ["colorN: 5"], "{text}");
    assert!(text.contains("colorScale: quantize"), "{text}");
}

/// **A click on the scale row's dot puts it back to linear**, as `⌫` does: the
/// file names neither the scale nor its pivot.
#[test]
fn a_click_on_the_scale_rows_dot_puts_it_back_to_linear() {
    let mut win = Window::housing("scale-dot").rail_shut();
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.move_to_row(SCALE_ROW);
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCALE_ROW).value, "diverging");
    assert!(
        win.row(SCALE_ROW).set,
        "fixture check: diverging has its dot"
    );
    let dot = win.row_drawn(SCALE_ROW).marker.center();
    win.click(dot);
    let row = win.row(SCALE_ROW);
    assert_eq!((row.value.as_str(), row.set), ("linear", false));
    win.save();
    let text = win.chart_text();
    assert!(!text.contains("colorScale"), "{text}");
    assert!(!text.contains("colorPivot"), "{text}");
}

/// **A pivot typed behind `diverging` is drawn as it is typed and kept on
/// `Enter`**: the hero's diverging scale is about the typed pivot before it is
/// kept, the row then reads the parameter as the analyst's, and the file names it.
#[test]
fn a_typed_pivot_previews_and_enter_keeps_it_with_its_dot() {
    let mut win = Window::housing("pivot-typed");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.type_letter(egui::Key::J, "j");
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCALE_ROW).value, "diverging");
    win.press(egui::Key::Enter);
    win.run(vec![egui::Event::Text("2".to_owned())]);
    win.run(Vec::new());
    match win.hero().scales.get(Channel::Fill) {
        Some(Scale::Diverging { pivot, .. }) => {
            assert_eq!(*pivot, 2.0, "the typed pivot is drawn before it is kept");
        }
        other => panic!("the hero's fill is not diverging: {other:?}"),
    }
    win.press(egui::Key::Enter);
    assert!(win.row(SCALE_ROW).parameter_set());
    win.save();
    let text = win.chart_text();
    assert!(text.contains("colorPivot: 2"), "{text}");
}

/// **The pivot row found by name opens on the pivot the file holds, not the one
/// the row displays**: `Enter` and `Enter` again with nothing typed keep a pivot
/// past four figures whole, rather than rounding it to what the row reads.
#[test]
fn opening_the_pivot_row_and_keeping_it_leaves_a_fine_pivot_whole() {
    let mut win = Window::housing("pivot-whole");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.move_to_row(SCALE_ROW);
    win.type_letter(egui::Key::L, "l");
    win.press(egui::Key::Enter);
    win.run(vec![egui::Event::Text("3.470004".to_owned())]);
    win.press(egui::Key::Enter);
    assert_eq!(
        win.row(SCALE_ROW).colour.and_then(|c| c.pivot),
        Some(3.470004),
        "fixture check: the typed pivot is kept"
    );
    win.find_row(PIVOT_ROW);
    assert_eq!(
        win.row(PIVOT_ROW).value,
        "3.47",
        "the row displays four figures"
    );
    win.press(egui::Key::Enter);
    win.press(egui::Key::Enter);
    win.save();
    let text = win.chart_text();
    assert_eq!(
        lines_of_key(&text, "colorPivot"),
        ["colorPivot: 3.470004"],
        "{text}"
    );
}

/// **On a diverging scale the scheme row steps between brightfield's pair and
/// `rdbu`**: `l` names `rdbu` and takes the pair's colours out, and `h` takes the
/// scheme out and writes the pair back.
#[test]
fn on_diverging_the_scheme_row_writes_rdbu_and_the_pair() {
    let mut win = Window::housing("diverging-scheme");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.type_letter(egui::Key::J, "j");
    win.type_letter(egui::Key::L, "l");
    win.type_letter(egui::Key::K, "k");
    assert_eq!(win.setting_cursor(), Some(SCHEME_ROW));
    assert_eq!(win.row(SCHEME_ROW).value, "blue-red");
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCHEME_ROW).value, "rdbu");
    win.type_letter(egui::Key::H, "h");
    let row = win.row(SCHEME_ROW);
    assert_eq!((row.value.as_str(), row.set), ("blue-red", false));
    win.save();
    let text = win.chart_text();
    assert!(!text.contains("colorScheme"), "{text}");
    assert!(text.contains("colorRange"), "{text}");

    let mut win = Window::housing("diverging-scheme-rdbu");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.type_letter(egui::Key::J, "j");
    win.type_letter(egui::Key::L, "l");
    win.type_letter(egui::Key::K, "k");
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCHEME_ROW).value, "rdbu");
    win.save();
    let text = win.chart_text();
    assert!(text.contains("colorScheme: rdbu"), "{text}");
    assert!(!text.contains("colorRange"), "{text}");
}

// ---------------------------------------------------------------------------
// AC2: the scheme row.
// ---------------------------------------------------------------------------

/// **`l` and `h` on the scheme row write `colorScheme`: the hero's marks are
/// painted along the new scheme's stops and the legend draws the same.** The
/// hero's fill stops equal the scheme's own after each step, and the legend's
/// move with them.
#[test]
fn l_and_h_write_the_scheme_and_the_hero_and_its_legend_are_drawn_in_it() {
    let mut win = Window::housing("scheme-keys");
    win.keep_on_colour(VALUE);
    let before_legend = win.legend_stops();
    assert_eq!(
        win.fill_stops(),
        SequentialScheme::Viridis.stops(),
        "fixture check: the hero starts in viridis"
    );
    assert_eq!(before_legend, SequentialScheme::Viridis.stops());

    win.open_colour_settings();
    let unsaved = win.unsaved();
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCHEME_ROW).value, "blues");
    assert!(win.row(SCHEME_ROW).set, "blues is the analyst's");
    assert_eq!(
        win.fill_stops(),
        SequentialScheme::Blues.stops(),
        "l painted the hero along blues"
    );
    assert_eq!(
        win.legend_stops(),
        SequentialScheme::Blues.stops(),
        "l drew the legend in blues"
    );
    assert_ne!(win.legend_stops(), before_legend, "the legend moved");
    assert!(
        win.unsaved() > unsaved && win.marked_unsaved(),
        "the step is an edit the chart holds unsaved"
    );

    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.fill_stops(), SequentialScheme::Turbo.stops());
    assert_eq!(win.legend_stops(), SequentialScheme::Turbo.stops());

    win.type_letter(egui::Key::H, "h");
    win.type_letter(egui::Key::H, "h");
    assert_eq!(win.row(SCHEME_ROW).value, "viridis");
    assert_eq!(
        win.fill_stops(),
        SequentialScheme::Viridis.stops(),
        "h twice walked back to viridis"
    );
    assert_eq!(win.legend_stops(), SequentialScheme::Viridis.stops());
}

/// **A click on a ramp of the strip, or on its name, writes that scheme, through
/// the card a shut rail hangs from the cell, and the ring follows.**
#[test]
fn a_click_on_a_ramp_or_its_name_writes_that_scheme_and_the_ring_follows() {
    let mut win = Window::housing("scheme-click").rail_shut();
    win.keep_on_colour(VALUE);
    win.open_colour_settings();
    assert_eq!(
        win.ringed(),
        [SequentialScheme::Viridis],
        "the current scheme wears the ring"
    );

    let turbo = win.scheme_drawn(SequentialScheme::Turbo).ramp.center();
    win.click(turbo);
    assert_eq!(win.row(SCHEME_ROW).value, "turbo");
    assert_eq!(
        win.fill_stops(),
        SequentialScheme::Turbo.stops(),
        "a click on turbo's ramp painted the hero along turbo"
    );
    assert_eq!(win.legend_stops(), SequentialScheme::Turbo.stops());
    assert_eq!(
        win.ringed(),
        [SequentialScheme::Turbo],
        "the ring moved to the scheme the click wrote"
    );

    let name = win.scheme_drawn(SequentialScheme::Meridian).name.center();
    win.click(name);
    assert_eq!(
        win.fill_stops(),
        SequentialScheme::Meridian.stops(),
        "a click on meridian's name wrote meridian"
    );
    assert_eq!(win.ringed(), [SequentialScheme::Meridian]);

    let viridis = win.scheme_drawn(SequentialScheme::Viridis).ramp.center();
    win.click(viridis);
    assert_eq!(win.fill_stops(), SequentialScheme::Viridis.stops());
    assert_eq!(win.legend_stops(), SequentialScheme::Viridis.stops());
    assert_eq!(win.ringed(), [SequentialScheme::Viridis]);
}

// ---------------------------------------------------------------------------
// AC3: the reverse row.
// ---------------------------------------------------------------------------

/// **The reverse row switches `colorReverse` by `h`, `l` or a click, and the
/// hero's ramp and the legend's run end for end.**
#[test]
fn reverse_switches_by_h_l_and_a_click_and_the_hero_and_its_legend_run_end_for_end() {
    let mut win = Window::housing("reverse").rail_shut();
    win.keep_on_colour(VALUE);
    win.open_colour_settings();
    win.type_letter(egui::Key::L, "l");
    let blues = SequentialScheme::Blues.stops();
    assert_eq!(
        win.fill_stops(),
        blues,
        "fixture check: blues, not reversed"
    );

    win.move_to_row(REVERSE_ROW);
    assert_eq!(win.setting_cursor(), Some(REVERSE_ROW));
    assert_eq!(win.row(REVERSE_ROW).value, "off");

    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(REVERSE_ROW).value, "on");
    assert!(win.row(REVERSE_ROW).set);
    assert_eq!(
        win.fill_stops(),
        flipped(&blues),
        "l ran the hero's ramp from its far end"
    );
    assert_eq!(
        win.legend_stops(),
        flipped(&blues),
        "and the legend's ramp the same way"
    );

    win.type_letter(egui::Key::H, "h");
    assert_eq!(win.row(REVERSE_ROW).value, "off");
    assert_eq!(win.fill_stops(), blues, "h turned it back");
    assert_eq!(win.legend_stops(), blues);

    let zone = win.row_drawn(REVERSE_ROW).value_zone.center();
    win.click(zone);
    assert_eq!(win.row(REVERSE_ROW).value, "on");
    assert_eq!(
        win.fill_stops(),
        flipped(&blues),
        "a click on the value switched it on"
    );
    assert_eq!(win.legend_stops(), flipped(&blues));
    win.click(zone);
    assert_eq!(win.row(REVERSE_ROW).value, "off");
    assert_eq!(win.fill_stops(), blues, "a second click switched it off");
}

// ---------------------------------------------------------------------------
// AC4: a column of names.
// ---------------------------------------------------------------------------

/// **On a string column on colour the scheme row is drawn muted with its reason
/// and writes nothing, and reverse acts: the hero's categories and the legend's
/// entries run from the last.**
#[test]
fn on_a_column_of_names_the_scheme_row_is_muted_and_reverse_reverses_the_categories() {
    let mut win = Window::housing_with_names("names").rail_shut();
    win.keep_on_colour(NAMES);
    let (categories, palette) = win.fill_categories();
    assert_eq!(
        categories.len(),
        CLASSES.len(),
        "fixture check: four classes"
    );
    let listed: Vec<String> = win.legend_entries().into_iter().map(|(l, _)| l).collect();
    assert_eq!(
        listed, categories,
        "fixture check: the legend lists the scale's order"
    );

    win.open_colour_settings();
    let scheme = win.row(SCHEME_ROW);
    assert!(
        scheme.reason.is_some(),
        "the scheme row names why it does not apply to names"
    );
    let drawn = win.row_drawn(SCHEME_ROW);
    assert!(drawn.reason_rect.is_some(), "the reason was drawn");
    assert!(
        drawn.ramp.is_none() && drawn.schemes.is_empty(),
        "a muted scheme row paints no ramp and offers no strip"
    );
    let unsaved = win.unsaved();
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.unsaved(), unsaved, "l on a muted row is no edit");
    assert_eq!(win.fill_categories(), (categories.clone(), palette.clone()));

    win.move_to_row(REVERSE_ROW);
    assert_eq!(win.setting_cursor(), Some(REVERSE_ROW));
    assert!(
        win.row(REVERSE_ROW).reason.is_none(),
        "reverse applies to names on a chart of dots"
    );
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(REVERSE_ROW).value, "on");
    let mut last_first = categories.clone();
    last_first.reverse();
    let (reversed, reversed_palette) = win.fill_categories();
    assert_eq!(
        reversed, last_first,
        "the scale lists the categories from the last"
    );
    assert_eq!(
        reversed_palette, palette,
        "each place keeps its colour, so the first category wears the last one's"
    );
    assert_eq!(
        win.legend_entries(),
        last_first
            .iter()
            .cloned()
            .zip(palette.iter().copied())
            .collect::<Vec<_>>(),
        "the legend lists the categories from the last, each beside the colour it wears"
    );

    win.type_letter(egui::Key::H, "h");
    assert_eq!(
        win.fill_categories(),
        (categories, palette),
        "h turned it back"
    );
}

// ---------------------------------------------------------------------------
// AC5: Save, close and reopen.
// ---------------------------------------------------------------------------

/// Steps the scheme row of `win` along the strip by `l` until it reads `scheme`.
fn step_to(win: &mut Window, scheme: SequentialScheme) {
    for _ in 0..8 {
        if win.row(SCHEME_ROW).value == scheme.wire_name() {
            return;
        }
        win.type_letter(egui::Key::L, "l");
    }
    panic!("l did not reach {}", scheme.wire_name());
}

/// **After Save, close and reopen, the chart, its legend and the file show each
/// value set.** The file names the scheme, viridis included, since brightfield's
/// default is not Mosaic's, and `colorReverse` where it is on; the reopened chart
/// is painted along the same stops, its legend draws them, and colour's rows read
/// what was left.
#[test]
fn saved_and_opened_again_the_chart_its_legend_and_the_file_show_each_value() {
    let cases = [
        ("viridis-reversed", SequentialScheme::Viridis, true),
        ("blues", SequentialScheme::Blues, false),
    ];
    for (name, scheme, reversed) in cases {
        let mut win = Window::housing(&format!("save-{name}"));
        win.keep_on_colour(VALUE);
        win.open_colour_settings();
        if scheme == SequentialScheme::Viridis {
            // Viridis is reached by a round trip: the row reads it as auto, and
            // the file names it all the same.
            win.type_letter(egui::Key::L, "l");
            win.type_letter(egui::Key::H, "h");
            assert_eq!(win.row(SCHEME_ROW).value, "viridis");
        } else {
            step_to(&mut win, scheme);
        }
        if reversed {
            win.move_to_row(REVERSE_ROW);
            win.type_letter(egui::Key::L, "l");
        }
        let mut stops = scheme.stops();
        if reversed {
            stops = flipped(&stops);
        }
        assert_eq!(win.fill_stops(), stops, "{name}: before Save");
        win.save();
        assert!(!win.marked_unsaved(), "{name}: Save left the unsaved mark");

        let written = win.chart_text();
        assert_eq!(
            lines_of_key(&written, "colorScheme"),
            [format!("colorScheme: {}", scheme.wire_name())],
            "{name}: the file names the scheme:\n{written}"
        );
        assert_eq!(
            lines_of_key(&written, "colorReverse"),
            if reversed {
                vec!["colorReverse: true".to_string()]
            } else {
                Vec::new()
            },
            "{name}: the file names reverse only where it is on:\n{written}"
        );

        let mut reopened = win.reopen();
        assert_eq!(
            reopened.fill_stops(),
            stops,
            "{name}: the reopened chart is painted along the same stops"
        );
        assert_eq!(
            reopened.legend_stops(),
            stops,
            "{name}: and its legend draws them"
        );
        assert!(!reopened.marked_unsaved(), "{name}: reopened unmarked");
        reopened.open_colour_settings();
        assert_eq!(
            reopened.row(SCHEME_ROW).value,
            scheme.wire_name(),
            "{name}: the scheme row reads what was left"
        );
        assert_eq!(
            reopened.row(REVERSE_ROW).value,
            if reversed { "on" } else { "off" },
            "{name}: and so does the reverse row"
        );
        assert_eq!(
            reopened.chart_text(),
            written,
            "{name}: reopening rewrote the chart file"
        );
    }
}

/// What the colour round trip below leaves set before Save.
#[derive(Clone, Copy, Debug)]
enum Left {
    /// A diverging scale about a typed pivot of 2.
    DivergingAboutTwo,
    /// A stepped scale of seven steps.
    SevenSteps,
    /// The domain fixed.
    DomainFixed,
    /// The legend below the plot.
    LegendBelow,
}

/// **After Save, close and reopen, the chart and the file show the scale, the
/// domain and the legend left set**: a diverging scale about a typed pivot, steps
/// at seven, the domain fixed (with the colour pin captured on reopen), and the
/// legend below the plot.
#[test]
fn saved_and_opened_again_the_scale_domain_and_legend_hold() {
    for left in [
        Left::DivergingAboutTwo,
        Left::SevenSteps,
        Left::DomainFixed,
        Left::LegendBelow,
    ] {
        let mut win = Window::housing(&format!("save-{left:?}"));
        win.keep_on_colour("median_income");
        win.open_colour_settings();
        match left {
            Left::DivergingAboutTwo => {
                win.move_to_row(SCALE_ROW);
                win.type_letter(egui::Key::L, "l");
                win.press(egui::Key::Enter);
                win.run(vec![egui::Event::Text("2".to_owned())]);
                win.press(egui::Key::Enter);
            }
            Left::SevenSteps => {
                win.move_to_row(SCALE_ROW);
                win.type_letter(egui::Key::L, "l");
                win.type_letter(egui::Key::L, "l");
                win.press(egui::Key::Enter);
                win.run(vec![egui::Event::Text("7".to_owned())]);
                win.press(egui::Key::Enter);
            }
            Left::DomainFixed => {
                win.find_row(DOMAIN_ROW);
                win.type_letter(egui::Key::L, "l");
                assert_eq!(win.row(DOMAIN_ROW).value, "fixed");
            }
            Left::LegendBelow => {
                win.move_to_row(LEGEND_ROW);
                win.type_letter(egui::Key::L, "l");
                assert_eq!(win.row(LEGEND_ROW).value, "below");
            }
        }
        win.save();
        assert!(
            !win.marked_unsaved(),
            "{left:?}: Save left the unsaved mark"
        );
        let written = win.chart_text();
        let has = |key: &str, line: &str| {
            assert_eq!(
                lines_of_key(&written, key),
                [line],
                "{left:?}: the file names {line}:\n{written}"
            );
        };
        match left {
            Left::DivergingAboutTwo => {
                has("colorScale", "colorScale: diverging");
                has("colorPivot", "colorPivot: 2");
            }
            Left::SevenSteps => {
                has("colorScale", "colorScale: quantize");
                has("colorN", "colorN: 7");
            }
            Left::DomainFixed => has("colorDomain", "colorDomain: Fixed"),
            Left::LegendBelow => {
                assert!(written.contains("vconcat:"), "{left:?}:\n{written}");
                assert!(written.contains("for: "), "{left:?}:\n{written}");
            }
        }

        let mut reopened = win.reopen();
        assert!(!reopened.marked_unsaved(), "{left:?}: reopened unmarked");
        match left {
            Left::DivergingAboutTwo => match reopened.hero().scales.get(Channel::Fill) {
                Some(Scale::Diverging { pivot, .. }) => {
                    assert_eq!(*pivot, 2.0, "the reopened chart is drawn about 2");
                }
                other => panic!("the reopened fill is not diverging: {other:?}"),
            },
            Left::SevenSteps => match reopened.hero().scales.get(Channel::Fill) {
                Some(Scale::Quantized { colours, .. }) => {
                    assert_eq!(colours.len(), 7, "the reopened chart draws seven steps");
                }
                other => panic!("the reopened fill is not stepped: {other:?}"),
            },
            Left::DomainFixed => {
                let hero = reopened.hero();
                let (lo, hi) = match hero.scales.get(Channel::Fill) {
                    Some(Scale::Sequential {
                        domain_min,
                        domain_max,
                        ..
                    }) => (*domain_min, *domain_max),
                    other => panic!("the reopened fill is not a ramp: {other:?}"),
                };
                let pin = reopened
                    .app
                    .chart_doc()
                    .live_dashboard()
                    .expect("the reopened chart is live")
                    .pins()
                    .get(&hero.path)
                    .and_then(|p| p.colour)
                    .expect("reopening captured the colour pin");
                assert_eq!((pin.lo, pin.hi), (lo, hi), "the pin holds the drawn ends");
            }
            Left::LegendBelow => {
                assert!(
                    reopened.hero().legend_below.is_some(),
                    "the reopened chart lays its legend out below the plot"
                );
                let doc = reopened.app.chart_doc();
                assert!(doc.legend_rect.is_none(), "and none at the right");
                let below = legend::below_blocks(&doc.composed);
                let [(0, _, band)] = below.as_slice() else {
                    panic!("the reopened page draws one legend, under the hero: {below:?}");
                };
                let area = reopened.hero().data_area();
                assert!(
                    band.y >= area.y + area.height,
                    "the legend's band {band:?} stands below the data area {area:?}"
                );
            }
        }
        reopened.open_colour_settings();
        match left {
            Left::DivergingAboutTwo => {
                let row = reopened.row(SCALE_ROW);
                assert_eq!(row.value, "diverging");
                assert!(row.parameter_set(), "the pivot reads as the analyst's");
            }
            Left::SevenSteps => {
                let row = reopened.row(SCALE_ROW);
                assert_eq!(row.value, "steps");
                assert_eq!(row.colour.and_then(|c| c.count), Some(7));
            }
            Left::DomainFixed => assert_eq!(reopened.row(DOMAIN_ROW).value, "fixed"),
            Left::LegendBelow => assert_eq!(reopened.row(LEGEND_ROW).value, "below"),
        }
        assert_eq!(
            reopened.chart_text(),
            written,
            "{left:?}: reopening rewrote the chart file"
        );
    }
}

// ---------------------------------------------------------------------------
// ⌫.
// ---------------------------------------------------------------------------

/// **`⌫` on the scheme row names viridis where the plot names a scheme and is no
/// edit where it names none; `⌫` on reverse takes the key out.**
#[test]
fn backspace_names_viridis_only_where_a_scheme_is_named_and_takes_reverse_out() {
    // Where the plot names none, as the opened dashboard's map does until a
    // number column is kept on colour (keeping one writes the scheme with it):
    // no edit, and the title stays unmarked.
    let mut win = Window::housing("backspace-none");
    win.open_colour_settings();
    assert_eq!(
        win.unsaved(),
        0,
        "fixture check: the opened chart holds no edit"
    );
    win.press(egui::Key::Backspace);
    assert_eq!(
        win.unsaved(),
        0,
        "⌫ on a plot that names no scheme is no edit"
    );
    assert!(!win.marked_unsaved(), "and the title is not marked");

    // Where it names one: viridis, by name, and reverse's key out.
    let mut win = Window::housing("backspace-named");
    win.keep_on_colour(VALUE);
    win.open_colour_settings();
    win.type_letter(egui::Key::L, "l");
    win.move_to_row(REVERSE_ROW);
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(REVERSE_ROW).value, "on");
    win.press(egui::Key::Backspace);
    assert_eq!(
        win.row(REVERSE_ROW).value,
        "off",
        "⌫ on reverse switched it off"
    );
    assert_eq!(win.fill_stops(), SequentialScheme::Blues.stops());
    for _ in 0..3 {
        win.type_letter(egui::Key::K, "k");
    }
    assert_eq!(win.setting_cursor(), Some(SCHEME_ROW));
    win.press(egui::Key::Backspace);
    assert_eq!(win.row(SCHEME_ROW).value, "viridis");
    assert!(!win.row(SCHEME_ROW).set);
    assert_eq!(win.fill_stops(), SequentialScheme::Viridis.stops());
    win.save();
    let written = win.chart_text();
    assert_eq!(
        lines_of_key(&written, "colorScheme"),
        ["colorScheme: viridis"],
        "⌫ wrote viridis by name and did not take the key out:\n{written}"
    );
    assert!(
        lines_of_key(&written, "colorReverse").is_empty(),
        "⌫ on reverse took its key out:\n{written}"
    );
}

/// **A count typed into the steps field stays written, even at the default**: the
/// file names the count, since Mosaic's thresholds differ from the default's.
#[test]
fn a_count_of_five_typed_over_seven_stays_in_the_file() {
    let mut win = Window::housing("steps-five");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.type_letter(egui::Key::J, "j");
    win.type_letter(egui::Key::L, "l");
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCALE_ROW).value, "steps");
    for typed in ["7", "5"] {
        win.press(egui::Key::Enter);
        win.run(vec![egui::Event::Text(typed.to_owned())]);
        win.press(egui::Key::Enter);
    }
    let row = win.row(SCALE_ROW);
    assert_eq!(row.colour.and_then(|c| c.count), Some(5));
    assert!(!win.row(SCALE_ROW).parameter_set());
    win.save();
    let text = win.chart_text();
    assert!(text.contains("colorN: 5"), "{text}");
}

/// **`Esc` in the scale row's pivot field drops what was typed**: the hero is
/// drawn about the pivot it had.
#[test]
fn esc_in_the_pivot_field_puts_the_heros_pivot_back() {
    let mut win = Window::housing("pivot-esc");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.type_letter(egui::Key::J, "j");
    win.type_letter(egui::Key::L, "l");
    let pivot = |win: &Window| match win.hero().scales.get(Channel::Fill) {
        Some(Scale::Diverging { pivot, .. }) => *pivot,
        other => panic!("the hero's fill is not diverging: {other:?}"),
    };
    let before = pivot(&win);
    win.press(egui::Key::Enter);
    win.run(vec![egui::Event::Text("1234".to_owned())]);
    win.run(Vec::new());
    assert_eq!(pivot(&win), 1234.0, "the typed pivot is previewed");
    win.press(egui::Key::Escape);
    assert_eq!(pivot(&win), before, "Esc put the pivot back");
}

/// **Stepping the domain row to fixed writes `colorDomain: Fixed`**, with its
/// capital, which is the word Mosaic reads.
#[test]
fn stepping_the_domain_row_to_fixed_writes_colordomain_fixed() {
    let mut win = Window::housing("domain-fixed");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.type_letter(egui::Key::Slash, "/");
    for (key, text) in [
        (egui::Key::D, "d"),
        (egui::Key::O, "o"),
        (egui::Key::M, "m"),
    ] {
        win.type_letter(key, text);
    }
    assert_eq!(win.setting_cursor(), Some(DOMAIN_ROW));
    win.press(egui::Key::Enter);
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(DOMAIN_ROW).value, "fixed");
    win.save();
    let text = win.chart_text();
    assert!(text.contains("colorDomain: Fixed"), "{text}");
}

/// **Two typed ends write `colorDomain: [0, 4]`, and `⌫` takes the key out**:
/// the row keeps the ends under its own name, not the axis range's.
#[test]
fn typing_two_ends_into_the_domain_row_writes_them_and_backspace_resets() {
    let mut win = Window::housing("domain-ends");
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.type_letter(egui::Key::Slash, "/");
    for (key, text) in [
        (egui::Key::D, "d"),
        (egui::Key::O, "o"),
        (egui::Key::M, "m"),
    ] {
        win.type_letter(key, text);
    }
    win.press(egui::Key::Enter);
    assert_eq!(win.setting_cursor(), Some(DOMAIN_ROW));
    win.press(egui::Key::Enter);
    for typed in ["0", "4"] {
        win.run(vec![egui::Event::Text(typed.to_owned())]);
        win.press(egui::Key::Enter);
    }
    assert_eq!(win.row(DOMAIN_ROW).value, "0 \u{2013} 4");
    win.save();
    let text = win.chart_text();
    assert!(text.contains("colorDomain: [0, 4]"), "{text}");

    // Save left the list; open it again on the row.
    win.open_colour_settings();
    win.type_letter(egui::Key::Slash, "/");
    for (key, text) in [
        (egui::Key::D, "d"),
        (egui::Key::O, "o"),
        (egui::Key::M, "m"),
    ] {
        win.type_letter(key, text);
    }
    win.press(egui::Key::Enter);
    assert_eq!(win.row(DOMAIN_ROW).value, "0 \u{2013} 4");
    win.press(egui::Key::Backspace);
    assert_eq!(win.row(DOMAIN_ROW).value, "auto");
    win.save();
    let text = win.chart_text();
    assert!(!text.contains("colorDomain"), "{text}");
}

/// A one-plot hero of cells filled by a number column, which draws no dot. The
/// shelf band opens on a cell chart as it does on a dot chart, so no raster or
/// heatmap stands in for it.
fn cell_hero(data: &str) -> String {
    format!(
        "data:\n  opened:\n    file: '{data}'\nplot:\n  - mark: cell\n    data: {{ from: opened }}\n    x: {{ bin: median_income }}\n    y: {{ bin: house_age }}\n    fill: {VALUE}\nwidth: 400\nheight: 300\n"
    )
}

/// The window hands colour's settings the marks the hero drew, which is what
/// mutes reverse on a chart with no dot. Every other window test's hero draws
/// dots, so this is the one that tells the marks drawn from no marks at all.
#[test]
fn reverse_is_muted_on_a_hero_that_draws_no_dot() {
    let mut w = Window::housing("cells");
    w.save();
    let chart = std::fs::read_dir(w.folder.join("panels"))
        .expect("Save wrote a panels folder")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "yaml"))
        .expect("Save wrote a chart file");
    let data = w.folder.join(HOUSING_FILE);
    std::fs::write(&chart, cell_hero(data.to_str().expect("utf-8 path")))
        .expect("the cell hero writes");
    let mut w = w.reopen();
    w.open_colour_settings();
    let reverse = w.row(REVERSE_ROW);
    assert!(
        reverse
            .reason
            .as_deref()
            .is_some_and(|r| r.contains("this chart draws none")),
        "reverse says why it does not apply, got {:?}",
        reverse.reason
    );
}

// ---------------------------------------------------------------------------
// An analyst's own colours on a diverging scale.
// ---------------------------------------------------------------------------

/// Three colours of the analyst's own, none of them brightfield's pair.
const OWN_COLOURS: [&str; 3] = ["#1b7837", "#f7f7f7", "#762a83"];

/// A window over a diverging hero whose file writes [`OWN_COLOURS`] as its
/// `colorRange`, opened again from that file, with colour's settings open.
fn diverging_over_own_colours(name: &str) -> Window {
    let mut win = Window::housing(name);
    win.keep_on_colour("median_income");
    win.open_colour_settings();
    win.move_to_row(SCALE_ROW);
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCALE_ROW).value, "diverging");
    win.save();
    let chart = std::fs::read_dir(win.folder.join("panels"))
        .expect("Save wrote a panels folder")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "yaml"))
        .expect("Save wrote a chart file");
    let text = std::fs::read_to_string(&chart).expect("the chart reads");
    // The pair's `colorRange` out, the analyst's in beside the scale.
    let mut out = Vec::new();
    let mut in_range = None;
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        if let Some(at) = in_range {
            if indent > at || (indent == at && line.trim_start().starts_with("- ")) {
                continue;
            }
            in_range = None;
        }
        if line.trim_start().starts_with("colorRange:") {
            in_range = Some(indent);
            continue;
        }
        out.push(line.to_string());
        if line.trim_start() == "colorScale: diverging" {
            let quoted: Vec<String> = OWN_COLOURS.iter().map(|c| format!("'{c}'")).collect();
            out.push(format!(
                "{}colorRange: [{}]",
                " ".repeat(indent),
                quoted.join(", ")
            ));
        }
    }
    let written = out.join("\n") + "\n";
    assert!(written.contains(OWN_COLOURS[0]), "{written}");
    std::fs::write(&chart, written).expect("the chart writes");
    let mut win = win.reopen();
    win.open_colour_settings();
    win
}

/// Whether the saved file still holds the analyst's colours.
fn holds_own_colours(text: &str) -> bool {
    OWN_COLOURS.iter().all(|c| text.contains(c))
}

/// **The scheme row reads an analyst's own `colorRange` as custom, and set**:
/// those colours are what the chart draws, whatever scheme the file names.
#[test]
fn on_diverging_the_scheme_row_reads_an_own_colour_range_as_custom() {
    let win = diverging_over_own_colours("own-range-reads");
    let row = win.row(SCHEME_ROW);
    assert_eq!((row.value.as_str(), row.set), ("custom", true));
}

/// **`l` on the scheme row over custom colours names `rdbu` and takes the
/// colours out**, so the key written is the one drawn.
#[test]
fn l_on_custom_colours_writes_rdbu_and_takes_the_range_out() {
    let mut win = diverging_over_own_colours("own-range-rdbu");
    assert_eq!(win.setting_cursor(), Some(SCHEME_ROW));
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCHEME_ROW).value, "rdbu");
    win.save();
    let text = win.chart_text();
    assert!(text.contains("colorScheme: rdbu"), "{text}");
    assert!(!text.contains("colorRange"), "{text}");
}

/// **`⌫` on the scheme row over custom colours takes them out**: the row reads
/// the diverging scale's auto again.
#[test]
fn backspace_on_custom_colours_takes_the_range_out() {
    let mut win = diverging_over_own_colours("own-range-backspace");
    assert_eq!(win.setting_cursor(), Some(SCHEME_ROW));
    win.press(egui::Key::Backspace);
    let row = win.row(SCHEME_ROW);
    assert_eq!((row.value.as_str(), row.set), ("blue-red", false));
    win.save();
    let text = win.chart_text();
    assert!(!holds_own_colours(&text), "{text}");
}

/// **A step of the scale to linear and back to diverging leaves the analyst's
/// colours as they are**: no automatic write takes them out or covers them
/// with the pair.
#[test]
fn a_scale_step_to_linear_and_back_keeps_custom_colours() {
    let mut win = diverging_over_own_colours("own-range-scale");
    win.move_to_row(SCALE_ROW);
    win.type_letter(egui::Key::H, "h");
    assert_eq!(win.row(SCALE_ROW).value, "linear");
    assert_eq!(win.row(SCHEME_ROW).value, "custom", "custom on linear too");
    win.save();
    let text = win.chart_text();
    assert!(holds_own_colours(&text), "linear keeps them:\n{text}");

    let mut win = win.reopen();
    win.open_colour_settings();
    win.move_to_row(SCALE_ROW);
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCALE_ROW).value, "diverging");
    assert_eq!(win.row(SCHEME_ROW).value, "custom");
    win.save();
    let text = win.chart_text();
    assert!(holds_own_colours(&text), "diverging keeps them:\n{text}");
    assert_eq!(
        lines_of_key(&text, "colorRange").len(),
        1,
        "one colorRange, the analyst's:\n{text}"
    );
}

/// **On a linear scale, `l` on the scheme row over custom colours names the
/// strip's first scheme and takes the colours out.**
#[test]
fn on_linear_l_on_custom_colours_writes_the_scheme_and_takes_the_range_out() {
    let mut win = diverging_over_own_colours("own-range-linear");
    win.move_to_row(SCALE_ROW);
    win.type_letter(egui::Key::H, "h");
    assert_eq!(win.row(SCALE_ROW).value, "linear");
    for _ in 0..4 {
        if win.setting_cursor() == Some(SCHEME_ROW) {
            break;
        }
        win.type_letter(egui::Key::K, "k");
    }
    assert_eq!(win.row(SCHEME_ROW).value, "custom");
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(SCHEME_ROW).value, "viridis");
    win.save();
    let text = win.chart_text();
    assert!(text.contains("colorScheme: viridis"), "{text}");
    assert!(!text.contains("colorRange"), "{text}");
}
