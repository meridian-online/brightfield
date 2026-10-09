//! **The range row at the window: its two ends preview on the axis and are kept
//! as `xDomain`, `⌫` takes them out, and a scale change or a new column on the
//! axis puts a set range back to auto in the same edit as the change.**
//!
//! The list alone is in `shelf_settings_range.rs`. These tests press the keys in
//! a headless window over the housing sample and read the plot the chart holds,
//! the scale it drew and the words the status band says.

use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::app::CHART;
use brightfield_shell::design::Mode;
use brightfield_shell::shelf::{ColumnList, ListTab, LOG_CANNOT, RANGE_ROW, SCALE_ROW};
use brightfield_shell::startup::default_layout;
use brightfield_shell::text_ink;
use brightfield_shell::window::{Boot, MeridianApp, SHELF_REFUSAL_STATUS_ID};
use brightfield_spec::ast::SpecValue;
use brightfield_spec::edit::plot_at_path;
use brightfield_workbench::PaneKey;

const HOUSING_FILE: &str = "california_housing_sample.csv";

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-shelf-range-{name}-{}-{}",
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

fn key_event(key: egui::Key) -> egui::Event {
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
    texts: Vec<text_ink::DrawnText>,
    _root: TempDir,
}

impl Window {
    fn housing(name: &str) -> Self {
        let root = TempDir::new(name);
        let folder = root.0.join("data");
        std::fs::create_dir_all(&folder).expect("the data file's folder");
        let data = folder.join(HOUSING_FILE);
        std::fs::copy(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data")
                .join(HOUSING_FILE),
            &data,
        )
        .expect("the housing fixture copies");
        let boot = Boot::data_file(data.to_str().expect("utf-8 path")).expect("the sample opens");
        let mut win = Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
            texts: Vec::new(),
            _root: root,
        };
        win.settle();
        assert!(
            win.app.focus_pane(PaneKey::new(CHART)),
            "the pane takes focus"
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
        self.run(vec![key_event(key), egui::Event::Text(text.to_owned())]);
        self.run(Vec::new());
    }

    /// Type `text` into the field that is open.
    fn type_text(&mut self, text: &str) {
        self.run(vec![egui::Event::Text(text.to_owned())]);
        self.run(Vec::new());
    }

    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_event(key)]);
        self.run(Vec::new());
    }

    fn list(&self) -> &ColumnList {
        self.app
            .protocol_model()
            .column_list()
            .expect("a list is open")
    }

    /// `e`, `x`, up to `median_income`, `Enter`: the column kept on x, which takes
    /// the map's projection out and leaves a plot with axes to set.
    fn keep_income_on_x(&mut self) {
        self.type_letter(egui::Key::E, "e");
        self.type_letter(egui::Key::X, "x");
        for _ in 0..12 {
            if self.list().cursor() == Some("median_income") {
                break;
            }
            self.type_letter(egui::Key::K, "k");
        }
        self.press(egui::Key::Enter);
    }

    /// With the band holding the keys: x's list, turned to its settings.
    fn open_x_settings(&mut self) {
        self.type_letter(egui::Key::X, "x");
        self.press(egui::Key::Tab);
        assert_eq!(
            self.list().tab(),
            ListTab::Settings,
            "Tab did not turn x's list to its settings"
        );
    }

    /// The cursor to the settings row `name`, down from where it stands and then
    /// up, so no row is reached by counting the ones above it.
    fn cursor_to(&mut self, name: &'static str) {
        for (key, text) in [(egui::Key::J, "j"), (egui::Key::K, "k")] {
            for _ in 0..6 {
                if self.list().setting_cursor().map(|r| r.name) == Some(name) {
                    return;
                }
                self.type_letter(key, text);
            }
        }
        assert_eq!(
            self.list().setting_cursor().map(|r| r.name),
            Some(name),
            "the cursor did not reach the {name} row"
        );
    }

    /// Keep the range `lo` to `hi` on x by the keys: `Enter` opens both fields,
    /// each end is typed over the selected one, and `Enter` on each moves on.
    fn keep_range(&mut self, lo: &str, hi: &str) {
        self.cursor_to(RANGE_ROW);
        self.press(egui::Key::Enter);
        self.type_text(lo);
        self.press(egui::Key::Enter);
        self.type_text(hi);
        self.press(egui::Key::Enter);
        assert!(self.list().field().is_none(), "keeping closed the fields");
    }

    /// What the hero's plot holds under `key`.
    fn attribute(&self, key: &str) -> Option<SpecValue> {
        let spec = self.app.chart_doc().live_spec().expect("a live spec");
        let path = &self.app.chart_doc().composed.plots[0].path;
        plot_at_path(spec, path)
            .expect("the hero's plot")
            .attributes
            .get(key)
            .cloned()
    }

    /// The scale the hero was last drawn against on x.
    fn drawn_x_scale(&self) -> Option<Scale> {
        self.app.chart_doc().composed.plots[0]
            .scales
            .get(Channel::X)
            .cloned()
    }

    /// The ends of the linear scale the hero was last drawn against on x.
    fn x_ends(&self) -> (f64, f64) {
        match self.drawn_x_scale() {
            Some(Scale::Linear {
                domain_min,
                domain_max,
                ..
            }) => (domain_min, domain_max),
            other => panic!("x was not drawn linear: {other:?}"),
        }
    }

    /// The range row as the list reads it: its value and whether the file set it.
    fn range_row(&self) -> (String, bool) {
        let row = self
            .list()
            .settings()
            .iter()
            .find(|r| r.name == RANGE_ROW)
            .expect("x has a range row");
        (row.value.clone(), row.set)
    }

    /// What the status band drew, left to right.
    fn status_text(&self) -> Vec<String> {
        let band = self.app.rail().rect.expect("the status band drew");
        let mut drawn: Vec<(f32, String)> = self
            .texts
            .iter()
            .filter(|t| band.contains_rect(t.visible) && t.visible.is_positive())
            .map(|t| (t.visible.left(), t.text.clone()))
            .collect();
        drawn.sort_by(|a, b| a.0.total_cmp(&b.0));
        drawn.into_iter().map(|(_, text)| text).collect()
    }
}

fn ends(lo: i64, hi: i64) -> SpecValue {
    SpecValue::Array(vec![SpecValue::Integer(lo), SpecValue::Integer(hi)])
}

/// **AC2.** A typed low end previews on the axis before it is kept, `Enter` on the
/// low moves to the high, `Enter` on the high keeps `xDomain: [low, high]` and the
/// axis is drawn over it, and `⌫` takes the key out so the axis runs over its rows
/// again.
#[test]
fn typed_ends_preview_on_the_axis_are_kept_as_xdomain_and_backspace_takes_them_out() {
    let mut win = Window::housing("keys");
    win.keep_income_on_x();
    win.open_x_settings();
    let rows = win.x_ends();
    assert_eq!(win.attribute("xDomain"), None);
    assert_eq!(win.range_row().1, false, "the row reads auto");

    win.cursor_to(RANGE_ROW);
    win.press(egui::Key::Enter);
    win.type_text("1");
    assert!(
        win.app.chart_doc().shelf_preview().is_some(),
        "a typed low end is previewed"
    );
    assert_eq!(
        win.x_ends().0,
        1.0,
        "the preview draws the axis from the low"
    );
    // Dropping the field takes the preview back: nothing was kept.
    win.press(egui::Key::Escape);
    assert_eq!(win.attribute("xDomain"), None, "dropping kept nothing");
    assert_eq!(win.x_ends(), rows, "and the axis is over its rows");

    win.press(egui::Key::Enter);
    win.type_text("1");
    win.press(egui::Key::Enter);
    win.type_text("10");
    win.press(egui::Key::Enter);
    assert_eq!(win.attribute("xDomain"), Some(ends(1, 10)));
    assert_eq!(win.x_ends(), (1.0, 10.0), "the axis is drawn over the ends");
    assert_eq!(win.range_row(), ("1 \u{2013} 10".to_string(), true));

    win.press(egui::Key::Backspace);
    assert_eq!(win.attribute("xDomain"), None, "\u{232b} took the key out");
    assert_eq!(win.x_ends(), rows, "and the axis runs over its rows again");
    assert_eq!(win.range_row().1, false);
}

/// **AC2.** A high at or below the low is refused under the row and nothing is
/// kept, so the axis stays over its rows.
#[test]
fn a_high_at_or_below_the_low_keeps_nothing_in_the_window() {
    let mut win = Window::housing("high");
    win.keep_income_on_x();
    win.open_x_settings();
    let rows = win.x_ends();
    win.cursor_to(RANGE_ROW);
    win.press(egui::Key::Enter);
    win.type_text("5");
    win.press(egui::Key::Enter);
    win.type_text("5");
    win.press(egui::Key::Enter);
    assert!(win.list().field().is_some(), "the edit stays open");
    assert_eq!(win.attribute("xDomain"), None);
    assert_eq!(win.x_ends(), rows);
}

/// **AC4.** A scale change on the axis takes a set range out in the same edit:
/// the row reads auto, the status band says so in the shelf's words, and one `u`
/// restores the scale and the range together.
#[test]
fn a_scale_change_puts_a_set_range_back_to_auto_and_one_u_restores_both() {
    let mut win = Window::housing("scale");
    win.keep_income_on_x();
    win.open_x_settings();
    win.keep_range("1", "10");
    assert_eq!(win.attribute("xDomain"), Some(ends(1, 10)));

    win.cursor_to(SCALE_ROW);
    win.type_letter(egui::Key::L, "l");
    assert_eq!(
        win.attribute("xScale"),
        Some(SpecValue::String("log".into()))
    );
    assert_eq!(
        win.attribute("xDomain"),
        None,
        "the range went with the scale"
    );
    assert_eq!(win.range_row().1, false, "the row reads auto");
    let said = win.status_text();
    assert!(
        said.iter().any(|t| t.contains(
            "x axis scale: linear \u{2192} log, and its range 1 \u{2013} 10 \u{2192} auto"
        )),
        "the band said {said:?}"
    );

    win.type_letter(egui::Key::U, "u");
    assert_eq!(win.attribute("xScale"), None, "one u restored the scale");
    assert_eq!(
        win.attribute("xDomain"),
        Some(ends(1, 10)),
        "and the range with it"
    );
    assert_eq!(win.x_ends(), (1.0, 10.0));
}

/// **AC4.** A new column put on the axis takes a set range out in the same edit,
/// the status band says so, and one `u` restores the column and the range
/// together.
#[test]
fn a_new_column_puts_a_set_range_back_to_auto_and_one_u_restores_both() {
    let mut win = Window::housing("column");
    win.keep_income_on_x();
    win.open_x_settings();
    win.keep_range("1", "10");
    assert_eq!(win.attribute("xDomain"), Some(ends(1, 10)));

    win.press(egui::Key::Tab);
    assert_eq!(win.list().tab(), ListTab::Columns);
    assert_eq!(win.list().cursor(), Some("median_income"));
    win.type_letter(egui::Key::J, "j");
    let other = win
        .list()
        .cursor()
        .expect("a column under the cursor")
        .to_owned();
    assert_ne!(other, "median_income");
    win.press(egui::Key::Enter);
    assert_eq!(
        win.attribute("xDomain"),
        None,
        "the range went with the column"
    );
    let said = win.status_text();
    assert!(
        said.iter()
            .any(|t| t.contains(", and its range 1 \u{2013} 10 \u{2192} auto")),
        "the band said {said:?}"
    );

    win.type_letter(egui::Key::U, "u");
    assert_eq!(
        win.attribute("xDomain"),
        Some(ends(1, 10)),
        "one u restored the range"
    );
    win.type_letter(egui::Key::X, "x");
    assert_eq!(
        win.list().cursor(),
        Some("median_income"),
        "and the column with it"
    );
}

/// **AC6.** On a log axis a low end at zero is refused: the status band names it,
/// nothing is kept, and the chart keeps the ends it last drew.
#[test]
fn a_log_axis_refuses_a_range_through_zero_in_the_window_and_keeps_its_drawn_ends() {
    let mut win = Window::housing("log");
    win.keep_income_on_x();
    win.open_x_settings();
    win.cursor_to(SCALE_ROW);
    win.type_letter(egui::Key::L, "l");
    assert!(matches!(win.drawn_x_scale(), Some(Scale::Log { .. })));
    let drawn = format!("{:?}", win.drawn_x_scale());

    win.cursor_to(RANGE_ROW);
    win.press(egui::Key::Enter);
    win.type_text("0");
    win.press(egui::Key::Enter);
    assert!(
        win.app.rail().drawn.contains(&SHELF_REFUSAL_STATUS_ID),
        "the band drew no line for the refusal"
    );
    let said = win.status_text();
    assert!(
        said.iter().any(|t| t.starts_with(LOG_CANNOT)),
        "the band said {said:?}"
    );
    assert!(win.list().field().is_some(), "the edit stays open");
    assert_eq!(win.attribute("xDomain"), None);
    assert_eq!(
        format!("{:?}", win.drawn_x_scale()),
        drawn,
        "the chart kept its drawn ends"
    );
}
