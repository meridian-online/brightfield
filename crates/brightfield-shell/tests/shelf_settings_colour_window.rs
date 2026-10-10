//! **In the window, colour's scheme and reverse rows act on the chart they sit
//! over: the keys and the clicks write `colorScheme` and `colorReverse`, the marks
//! and the legend are drawn again from them, and Save keeps them.**
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
    CardDrawn, ListTab, SchemeDrawn, SettingRow, SettingRowDrawn, REVERSE_ROW, SCHEME_ROW,
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

    /// The names of the open list's settings rows, in order.
    fn row_names(&self) -> Vec<&'static str> {
        self.app
            .protocol_model()
            .column_list()
            .expect("a list is open")
            .settings()
            .iter()
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
            .find(|s| s.scheme == scheme)
            .unwrap_or_else(|| panic!("the strip drew no {}", scheme.wire_name()))
    }

    /// The schemes of the strip that wear the ring, as the card drew it.
    fn ringed(&self) -> Vec<SequentialScheme> {
        self.row_drawn(SCHEME_ROW)
            .schemes
            .iter()
            .filter(|s| s.ring.is_some())
            .map(|s| s.scheme)
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
/// scheme and reverse in that order, each reading auto, with the sentence of the
/// row under the cursor at the foot.** In the Outline, and in the card a shut rail
/// hangs from the cell.
#[test]
fn tab_on_colour_shows_scheme_then_reverse_each_auto_with_the_foot() {
    let mut win = Window::housing("tab");
    win.keep_on_colour(VALUE);
    win.open_colour_settings();
    assert_eq!(
        win.row_names(),
        [SCHEME_ROW, REVERSE_ROW],
        "colour's head rows, in order"
    );
    for row in [SCHEME_ROW, REVERSE_ROW] {
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
        [SCHEME_ROW, REVERSE_ROW],
        "the card draws the scheme row and then the reverse row"
    );
    let sentence = shut.card().list.sentence.expect("the foot drew a sentence");
    assert_eq!(
        shut.text_in(sentence),
        "The ramp a number's colour runs along, which auto draws as viridis.",
        "the foot says what the scheme row does"
    );
    shut.type_letter(egui::Key::J, "j");
    assert_eq!(shut.setting_cursor(), Some(REVERSE_ROW));
    let sentence = shut.card().list.sentence.expect("the foot drew a sentence");
    assert_eq!(
        shut.text_in(sentence),
        "Whether colour runs the other way, a ramp from its far end and names from the last, which auto does not.",
        "the foot follows the cursor to the reverse row"
    );
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

    win.type_letter(egui::Key::J, "j");
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

    win.type_letter(egui::Key::J, "j");
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
            win.type_letter(egui::Key::J, "j");
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
    win.type_letter(egui::Key::J, "j");
    win.type_letter(egui::Key::L, "l");
    assert_eq!(win.row(REVERSE_ROW).value, "on");
    win.press(egui::Key::Backspace);
    assert_eq!(
        win.row(REVERSE_ROW).value,
        "off",
        "⌫ on reverse switched it off"
    );
    assert_eq!(win.fill_stops(), SequentialScheme::Blues.stops());
    win.type_letter(egui::Key::K, "k");
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
