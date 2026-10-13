//! **The window's half of the ring round the hero's legend: the chart document
//! says colour's list is open on either of its tabs, at rest it says not, and
//! the legend the ring would stand round is where the legend row left it.**
//!
//! A headless window rasters nothing, and a legend is painted only over a
//! raster, so no ring is painted here. `legend_ring_baseline.rs` photographs the
//! ring itself and reads its pixels; `legend.rs` holds the rule that cuts it back
//! at the data area.
//!
//! The window is the housing sample with `median_income` kept on colour.

use std::path::PathBuf;

use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::shelf::LEGEND_ROW;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::PaneKey;

const HOUSING_FILE: &str = "california_housing_sample.csv";
const COLUMN: &str = "median_income";

/// A directory of this test's own, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("bf-legend-ring-{name}-{}", std::process::id()));
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

/// One headless window over a copy of the housing sample, the hero's pane
/// focused.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    _root: TempDir,
}

impl Window {
    fn housing(name: &str) -> Self {
        let root = TempDir::new(name);
        let data = root.0.join(HOUSING_FILE);
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data")
            .join(HOUSING_FILE);
        std::fs::copy(source, &data).expect("the housing fixture copies");
        let boot = Boot::data_file(data.to_str().expect("utf-8 path")).expect("the sample opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            _root: root,
        };
        win.settle();
        assert!(win.app.focus_pane(PaneKey::new(CHART)));
        win.settle();
        win
    }

    fn run(&mut self, events: Vec<egui::Event>) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
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

    fn type_letter(&mut self, key: egui::Key, text: &str) {
        self.run(vec![key_down(key), egui::Event::Text(text.to_owned())]);
        self.run(Vec::new());
    }

    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_down(key)]);
        self.run(Vec::new());
    }

    /// Keep `median_income` on colour by the band's keys.
    fn keep_on_colour(&mut self) {
        self.type_letter(egui::Key::E, "e");
        self.type_letter(egui::Key::C, "c");
        for _ in 0..24 {
            let at = self
                .app
                .protocol_model()
                .column_list()
                .and_then(|l| l.cursor().map(str::to_owned));
            if at.as_deref() == Some(COLUMN) {
                break;
            }
            self.type_letter(egui::Key::J, "j");
        }
        self.press(egui::Key::Enter);
        assert_eq!(
            self.app.chart_doc().composed.plots[0]
                .fill_column
                .as_deref(),
            Some(COLUMN)
        );
        // Out of the list and the band, back to rest.
        for _ in 0..4 {
            self.press(egui::Key::Escape);
        }
        self.settle();
    }

    /// Colour's list opened on its columns.
    fn open_colour_columns(&mut self) {
        self.type_letter(egui::Key::E, "e");
        self.type_letter(egui::Key::C, "c");
        assert!(self.app.protocol_model().column_list().is_some());
    }

    /// The list turned to its settings, the cursor on the legend row.
    fn open_legend_row(&mut self) {
        self.press(egui::Key::Tab);
        for _ in 0..12 {
            let at = self
                .app
                .protocol_model()
                .column_list()
                .and_then(|l| l.setting_cursor().map(|r| r.name));
            if at == Some(LEGEND_ROW) {
                return;
            }
            self.type_letter(egui::Key::J, "j");
        }
        panic!("the settings cursor did not reach the legend row");
    }
}

/// At rest, with colour holding a column, colour's list is not open.
#[test]
fn at_rest_colours_list_is_not_open() {
    let mut win = Window::housing("rest");
    win.keep_on_colour();
    assert!(!win.app.chart_doc().colour_list_open);
}

/// Colour's list open on its columns, and then on its settings, is open; closed
/// again it is not.
#[test]
fn colours_list_is_open_on_either_tab() {
    let mut win = Window::housing("tabs");
    win.keep_on_colour();
    win.open_colour_columns();
    win.settle();
    assert!(win.app.chart_doc().colour_list_open, "on the columns tab");
    win.press(egui::Key::Tab);
    win.settle();
    assert!(win.app.chart_doc().colour_list_open, "on the settings tab");
    for _ in 0..4 {
        win.press(egui::Key::Escape);
    }
    win.settle();
    assert!(!win.app.chart_doc().colour_list_open, "after Esc");
}

/// Another channel's list is not colour's: x's list leaves the flag down.
#[test]
fn another_channels_list_does_not_raise_it() {
    let mut win = Window::housing("x");
    win.keep_on_colour();
    win.type_letter(egui::Key::E, "e");
    win.type_letter(egui::Key::X, "x");
    win.settle();
    assert!(win.app.protocol_model().column_list().is_some());
    assert!(!win.app.chart_doc().colour_list_open);
}

/// The legend row moves the legend the ring stands round: at the right it
/// reserves a band there, below it reserves one under the plot, and with none
/// there is neither (and so no block to ring), while the list stays open.
#[test]
fn the_legend_row_moves_the_block_the_ring_would_stand_round() {
    let mut win = Window::housing("row");
    win.keep_on_colour();
    win.open_colour_columns();
    win.settle();
    assert!(win.app.chart_doc().legend_rect.is_some());
    assert!(win.app.chart_doc().composed.plots[0].legend_below.is_none());

    win.open_legend_row();
    win.type_letter(egui::Key::L, "l");
    win.settle();
    assert!(win.app.chart_doc().legend_rect.is_none());
    assert!(win.app.chart_doc().composed.plots[0].legend_below.is_some());
    assert!(win.app.chart_doc().colour_list_open);

    win.type_letter(egui::Key::L, "l");
    win.settle();
    assert!(win.app.chart_doc().legend_rect.is_none());
    assert!(win.app.chart_doc().composed.plots[0].legend_below.is_none());
    assert!(win.app.chart_doc().colour_list_open);
}
