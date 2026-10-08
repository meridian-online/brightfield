//! **The title and ticks rows take typed text, with a preview and a refusal.**
//!
//! `Enter` on a typed row opens a field on the row's value, selected. What is
//! typed is drawn on the axis without being kept, `Esc` takes it back, and
//! `Enter` keeps it through the write a step makes, so `u` takes it back too. A
//! title with no text and a count the axis cannot aim at are refused in words
//! under the row, and the field stays open.
//!
//! Three altitudes, each reading what the thing under test produced:
//!
//! - **The list alone**: the field's state and the reports a key answers with,
//!   since a write or a preview the list does not report is one the chart never
//!   gets.
//! - **The window**: the attribute the plot holds and what the scene *paints*.
//!   A title is read as a glyph run of its own length, a tick count as the
//!   number of label runs under the hero's x axis; neither is read back from
//!   the resolver that was handed the value.
//! - **Pixels**: the open title field with its selection, and a refused count
//!   with the sentence under the row, in both themes. Regenerate them with
//!   `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test
//!   shelf_settings_typed`, and read what moved before committing it.

use brightfield_render::axis::compute_ticks;
use brightfield_render::channel::Channel;
use brightfield_render::text::LABEL_SIZE;
use brightfield_shell::app::CHART;
use brightfield_shell::design::{self, Mode};
use brightfield_shell::pipeline::Composed;
use brightfield_shell::shelf::{
    ticks_count, Binding, ChannelSettings, ColumnList, ColumnListRequest, ListColumn, ListReport,
    ListTab, RowEdit, RowField, SettingValue, ShelfChannels, TICKS_ROW, TITLE_NEEDS_TEXT,
    TITLE_ROW,
};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp, UNSAVED_MARK};
use brightfield_spec::ast::SpecValue;
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::layout::{tick_count_target, DEFAULT_TICK_COUNT, MAX_TICK_COUNT};
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::PaneKey;
use egui_kittest::{Harness, SnapshotOptions};

// ---------------------------------------------------------------------------
// The list alone.
// ---------------------------------------------------------------------------

fn key_event(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn text(typed: &str) -> egui::Event {
    egui::Event::Text(typed.to_string())
}

fn channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("population".to_string()),
        y: Binding::Column("latitude".to_string()),
        colour: Binding::Unset,
    }
}

const SOURCE: &str = "data:\n  t:\n    - { a: 1 }\nplot:\n  - mark: dot\n    data: { from: t }\n    x: a\n    y: a\nwidth: 600\nheight: 300\n";

fn settings_of(attrs: &str) -> ChannelSettings {
    let spec = parse_spec(&format!("{SOURCE}{attrs}\n"), Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    ChannelSettings::of_plot(&spec, plot, &channels())
}

/// A list on x's settings over a plot that writes `attrs`, the cursor on the
/// title row, which is the first.
fn list_over(attrs: &str) -> ColumnList {
    let mut list = ColumnList::new(ColumnListRequest {
        tile: "hero".to_string(),
        channel: ShelfChannel::X,
        channels: channels(),
        columns: vec![ListColumn {
            name: "population".to_string(),
            kind: "BIGINT".to_string(),
            moments: None,
        }],
    });
    list.set_settings(settings_of(attrs));
    list.feed_events(&[key_event(egui::Key::Tab)]);
    assert_eq!(list.tab(), ListTab::Settings, "Tab turned the list");
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(TITLE_ROW));
    list
}

/// The same list with the cursor on the ticks row, found as an analyst finds it:
/// `/`, its name, `Enter`.
fn list_on_ticks(attrs: &str) -> ColumnList {
    let mut list = list_over(attrs);
    list.feed_events(&[key_event(egui::Key::Slash)]);
    for ch in "ticks".chars() {
        list.feed_events(&[text(&ch.to_string())]);
    }
    list.feed_events(&[key_event(egui::Key::Enter)]);
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(TICKS_ROW));
    assert!(!list.querying() && list.field().is_none());
    list
}

fn edit(row: &'static str, value: SettingValue) -> RowEdit {
    RowEdit {
        channel: ShelfChannel::X,
        row,
        value,
    }
}

fn field(row: &'static str, typed: &str, selected: bool, refusal: Option<&str>) -> RowField {
    RowField {
        row,
        text: typed.to_string(),
        selected,
        refusal: refusal.map(str::to_string),
    }
}

/// **AC1.** `Enter` on the title row opens the field on brightfield's derived
/// title, the column's name, selected.
#[test]
fn enter_on_the_title_row_opens_the_field_on_the_derived_title_selected() {
    let mut list = list_over("");
    let reports = list.feed_events(&[key_event(egui::Key::Enter)]);
    assert_eq!(reports, []);
    assert_eq!(
        list.field(),
        Some(&field(TITLE_ROW, "population", true, None))
    );
    assert!(list.typing());
}

/// **AC1.** The first text typed replaces the selection, and each keystroke
/// reports the title as it stands as a preview: the axis is drawn with it.
#[test]
fn what_is_typed_replaces_the_selection_and_is_reported_as_a_preview_each_keystroke() {
    let mut list = list_over("");
    list.feed_events(&[key_event(egui::Key::Enter)]);
    let first = list.feed_events(&[text("M")]);
    assert_eq!(
        first,
        [ListReport::Preview(Some(edit(
            TITLE_ROW,
            SettingValue::Text("M".to_string())
        )))]
    );
    let more = list.feed_events(&[text("edian")]);
    assert_eq!(
        more,
        [ListReport::Preview(Some(edit(
            TITLE_ROW,
            SettingValue::Text("Median".to_string())
        )))]
    );
    assert_eq!(list.field(), Some(&field(TITLE_ROW, "Median", false, None)));
}

/// **AC1.** `Enter` keeps the typed title as a write the window makes, and the
/// field closes.
#[test]
fn enter_keeps_the_typed_title_and_closes_the_field() {
    let mut list = list_over("");
    list.feed_events(&[key_event(egui::Key::Enter), text("Median income")]);
    let kept = list.feed_events(&[key_event(egui::Key::Enter)]);
    assert_eq!(
        kept,
        [ListReport::Set(edit(
            TITLE_ROW,
            SettingValue::Text("Median income".to_string())
        ))]
    );
    assert_eq!(list.field(), None);
    assert_eq!(list.tab(), ListTab::Settings, "the list stays on its rows");
}

/// **AC1.** `Esc` drops the preview and the field and leaves the list open on
/// its rows; a second `Esc`, with no field open, is the one that backs out.
#[test]
fn esc_drops_the_preview_and_the_field_and_leaves_the_list_open() {
    let mut list = list_over("");
    list.feed_events(&[key_event(egui::Key::Enter), text("Median")]);
    let dropped = list.feed_events(&[key_event(egui::Key::Escape)]);
    assert_eq!(dropped, [ListReport::Preview(None)]);
    assert_eq!(list.field(), None);
    assert_eq!(list.tab(), ListTab::Settings);
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::Escape)]),
        [ListReport::BackedOut],
        "with no field open the same key backs out of the list"
    );
}

/// **AC1.** `Enter` on an emptied title is refused under the row in the words
/// that a title needs text; the field stays open and nothing is written. The
/// refusal waits for `Enter`: a title emptied on the way to its replacement is
/// not scolded as it is typed.
#[test]
fn enter_on_an_emptied_title_is_refused_in_words_and_the_field_stays_open() {
    let mut list = list_over("");
    list.feed_events(&[key_event(egui::Key::Enter)]);
    let emptied = list.feed_events(&[key_event(egui::Key::Backspace)]);
    assert_eq!(
        emptied,
        [ListReport::Preview(None)],
        "an emptied title draws nothing and says nothing yet"
    );
    assert_eq!(list.field(), Some(&field(TITLE_ROW, "", false, None)));
    let refused = list.feed_events(&[key_event(egui::Key::Enter)]);
    assert_eq!(refused, [], "a refusal reports no write");
    assert_eq!(
        list.field(),
        Some(&field(TITLE_ROW, "", false, Some(TITLE_NEEDS_TEXT)))
    );
    assert!(TITLE_NEEDS_TEXT.contains("title needs text"));
    // Text typed after the refusal clears it, and `Enter` then keeps.
    list.feed_events(&[text("Income")]);
    assert_eq!(
        list.field().map(|f| f.refusal.clone()),
        Some(None),
        "typing clears the refusal"
    );
    assert!(matches!(
        list.feed_events(&[key_event(egui::Key::Enter)]).as_slice(),
        [ListReport::Set(_)]
    ));
}

/// **AC1.** A title of spaces alone is no text.
#[test]
fn a_title_of_spaces_is_refused_as_an_empty_one_is() {
    let mut list = list_over("");
    list.feed_events(&[key_event(egui::Key::Enter), text("   ")]);
    assert_eq!(list.feed_events(&[key_event(egui::Key::Enter)]), []);
    assert_eq!(
        list.field().and_then(|f| f.refusal.as_deref()),
        Some(TITLE_NEEDS_TEXT)
    );
}

/// **AC1.** `⌫` on the title row, with no field open, puts the derived title
/// back: it reports the row put to auto.
#[test]
fn backspace_on_the_title_row_puts_it_back_to_auto() {
    let mut list = list_over("xLabel: Median income");
    assert_eq!(
        list.setting_cursor().map(|r| (r.value.as_str(), r.set)),
        Some(("Median income", true))
    );
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::Backspace)]),
        [ListReport::Set(edit(TITLE_ROW, SettingValue::Auto))]
    );
}

/// **AC2.** `0`, `2.5` and `abc` leave the field open, report no write and no
/// preview, and put a sentence under the row; `Enter` on them is refused too.
#[test]
fn a_count_that_is_not_one_leaves_the_field_open_and_says_why() {
    for typed in ["0", "2.5", "abc", "-3", "1001", "4 ticks"] {
        let mut list = list_on_ticks("");
        list.feed_events(&[key_event(egui::Key::Enter)]);
        let reports = list.feed_events(&[text(typed)]);
        assert_eq!(
            reports,
            [ListReport::Preview(None)],
            "`{typed}` is drawn as nothing"
        );
        let open = list
            .field()
            .unwrap_or_else(|| panic!("`{typed}` closed the field"));
        let said = open
            .refusal
            .as_deref()
            .unwrap_or_else(|| panic!("`{typed}` was refused in no words"));
        assert!(
            said.contains("whole number") && said.contains(&MAX_TICK_COUNT.to_string()),
            "`{typed}` was refused as {said:?}"
        );
        assert_eq!(
            list.feed_events(&[key_event(egui::Key::Enter)]),
            [],
            "`Enter` kept `{typed}`"
        );
        assert!(
            list.field().is_some(),
            "`Enter` closed the field on `{typed}`"
        );
    }
}

/// **AC2.** `4` and `Enter` keep a count of four; typing is previewed on the way.
#[test]
fn a_count_is_previewed_as_it_is_typed_and_kept_by_enter() {
    let mut list = list_on_ticks("");
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::Enter)]),
        [],
        "opening the field reports nothing"
    );
    assert_eq!(list.field(), Some(&field(TICKS_ROW, "5", true, None)));
    assert_eq!(
        list.feed_events(&[text("4")]),
        [ListReport::Preview(Some(edit(
            TICKS_ROW,
            SettingValue::Count(4)
        )))]
    );
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::Enter)]),
        [ListReport::Set(edit(TICKS_ROW, SettingValue::Count(4)))]
    );
    assert_eq!(list.field(), None);
}

/// **AC2.** A refusal clears once the text is a count again, and the row then
/// previews.
#[test]
fn a_refusal_clears_when_the_text_becomes_a_count_again() {
    let mut list = list_on_ticks("");
    list.feed_events(&[key_event(egui::Key::Enter), text("2.")]);
    assert!(list.field().is_some_and(|f| f.refusal.is_some()));
    let reports = list.feed_events(&[key_event(egui::Key::Backspace)]);
    assert_eq!(
        reports,
        [ListReport::Preview(Some(edit(
            TICKS_ROW,
            SettingValue::Count(2)
        )))]
    );
    assert_eq!(list.field(), Some(&field(TICKS_ROW, "2", false, None)));
}

/// **AC2.** What the field keeps as a count is what the spec's own reader takes
/// as one: the two are asked about every count from nought past the ceiling, so
/// a range typed into the row and not into the reader would show here.
#[test]
fn the_field_and_the_reader_agree_over_what_is_a_count() {
    for n in 0..=MAX_TICK_COUNT + 2 {
        let kept = ticks_count(&n.to_string()).ok();
        let read = tick_count_target(&SpecValue::Integer(i64::try_from(n).expect("small")));
        assert_eq!(kept, read, "the field and the reader disagree over {n}");
    }
}

/// **AC1, AC2.** A letter typed into a field is text, not a verb: `j` does not
/// move the cursor and `u` does not undo.
#[test]
fn a_letter_typed_into_the_field_is_text_and_not_a_verb() {
    let mut list = list_over("");
    list.feed_events(&[key_event(egui::Key::Enter)]);
    for (key, typed) in [
        (egui::Key::J, "j"),
        (egui::Key::U, "u"),
        (egui::Key::Slash, "/"),
        (egui::Key::H, "h"),
    ] {
        let reports = list.feed_events(&[key_event(key), text(typed)]);
        assert!(
            reports
                .iter()
                .all(|r| matches!(r, ListReport::Preview(Some(_)))),
            "`{typed}` was taken as a verb: {reports:?}"
        );
    }
    assert_eq!(list.field().map(|f| f.text.as_str()), Some("ju/h"));
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(TITLE_ROW));
    assert!(!list.querying());
}

// ---------------------------------------------------------------------------
// The window.
// ---------------------------------------------------------------------------

const HOUSING_FILE: &str = "california_housing_sample.csv";

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bf-shelf-typed-{name}-{}-{}",
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

/// A title no tick label, axis title or tile title in the housing dashboard
/// shares a length with.
const TYPED_TITLE: &str = "Household-income-in-tens-of-thousands";

/// One headless window over a copy of the housing sample, with median income
/// kept on x so the plot has axes to set, and x's settings list open.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    folder: std::path::PathBuf,
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
            folder,
            _root: root,
        };
        win.settle();
        assert!(
            win.app.focus_pane(PaneKey::new(CHART)),
            "the pane takes focus"
        );
        win.settle();
        win.keep_income_on_x();
        win.type_letter(egui::Key::X, "x");
        win.press(egui::Key::Tab);
        assert_eq!(
            win.app.protocol_model().column_list().map(ColumnList::tab),
            Some(ListTab::Settings),
            "Tab did not turn x's list to its settings"
        );
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

    fn type_letter(&mut self, key: egui::Key, typed: &str) {
        self.run(vec![key_event(key), text(typed)]);
        self.run(Vec::new());
    }

    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_event(key)]);
        self.run(Vec::new());
    }

    /// Text typed into the open field, as a keyboard brings it.
    fn type_text(&mut self, typed: &str) {
        self.run(vec![text(typed)]);
        self.run(Vec::new());
    }

    /// `e`, `x`, up to `median_income`, `Enter`: the column kept on x, which
    /// takes the map's projection out and leaves a plot with axes to set.
    fn keep_income_on_x(&mut self) {
        self.type_letter(egui::Key::E, "e");
        self.type_letter(egui::Key::X, "x");
        for _ in 0..12 {
            if self
                .app
                .protocol_model()
                .column_list()
                .and_then(|l| l.cursor().map(str::to_owned))
                .as_deref()
                == Some("median_income")
            {
                break;
            }
            self.type_letter(egui::Key::K, "k");
        }
        self.press(egui::Key::Enter);
    }

    /// The cursor to the ticks row, found by its name.
    fn cursor_to_ticks(&mut self) {
        self.type_letter(egui::Key::Slash, "/");
        for ch in "ticks".chars() {
            let key = egui::Key::from_name(&ch.to_uppercase().to_string()).expect("a letter");
            self.type_letter(key, &ch.to_string());
        }
        self.press(egui::Key::Enter);
        assert_eq!(
            self.list().setting_cursor().map(|r| r.name),
            Some(TICKS_ROW)
        );
    }

    fn list(&self) -> &ColumnList {
        self.app
            .protocol_model()
            .column_list()
            .expect("the list is open")
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

    fn composed(&self) -> &Composed {
        &self.app.chart_doc().composed
    }

    /// The glyph runs the scene paints, as the glyph count of each.
    fn runs(&self) -> Vec<usize> {
        self.composed()
            .scene
            .encoding()
            .resources
            .glyph_runs
            .iter()
            .map(|run| run.glyphs.len())
            .collect()
    }

    /// How many runs the scene paints that are `glyphs` long: the typed title,
    /// which no other text on the page matches in length.
    fn runs_of(&self, glyphs: usize) -> usize {
        self.runs().iter().filter(|n| **n == glyphs).count()
    }

    /// How many tick labels the hero paints under its x axis.
    fn x_tick_labels(&self) -> usize {
        let composed = self.composed();
        let plot = &composed.plots[0];
        let row = plot.rect.y + plot.layout.plot_y_end() + 10.0;
        let (left, right) = (
            plot.rect.x + plot.layout.plot_x_start() - 20.0,
            plot.rect.x + plot.layout.plot_x_end() + 20.0,
        );
        composed
            .scene
            .encoding()
            .resources
            .glyph_runs
            .iter()
            .filter(|run| {
                (run.font_size - LABEL_SIZE).abs() < 0.01
                    && run.transform.matrix[0].abs() > 0.5
                    && f64::from(run.transform.translation[1]) > row
                    && (left..=right).contains(&f64::from(run.transform.translation[0]))
            })
            .count()
    }

    /// The ticks the axis would draw at `count`, asked of the scale the hero was
    /// drawn against.
    fn ticks_at(&self, count: usize) -> usize {
        let scale = self.composed().plots[0]
            .scales
            .get(Channel::X)
            .expect("the hero's x scale");
        compute_ticks(scale, count).len()
    }

    fn marked_unsaved(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

    fn last_edit(&self) -> Option<String> {
        self.app.chart_doc().last_shelf_edit().map(str::to_owned)
    }

    fn save(&mut self) -> String {
        let ctx = self.ctx.clone();
        self.app
            .save_protocol(&ctx)
            .expect("a data file's window has a Protocol to save")
            .expect("the Protocol saves");
        self.settle();
        let chart = std::fs::read_dir(self.folder.join("panels"))
            .expect("Save wrote a panels folder")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "yaml"))
            .expect("Save wrote a chart file");
        std::fs::read_to_string(&chart).expect("the chart file reads")
    }
}

/// **AC1.** The title is typed, previewed on the axis, dropped by `Esc`, and
/// kept by `Enter`: the preview is painted but is no edit (the window's last
/// edit, its unsaved state and the file Save writes are what they were), `Esc`
/// paints the axis as before, and `Enter` writes `xLabel`.
#[test]
fn a_title_is_previewed_on_the_axis_dropped_by_esc_and_kept_by_enter() {
    let mut win = Window::housing("title");
    let glyphs = TYPED_TITLE.chars().count();
    // Save seals the shelf's undo, so the window's last edit is read after it:
    // a preview that was kept would show as the edit it made.
    let saved = win.save();
    let before = (win.runs_of(glyphs), win.last_edit());
    assert_eq!(before.1, None, "Save left an edit to take back");
    let derived = win.runs_of("median_income".chars().count());
    assert_eq!(before.0, 0, "a title this long is not on the page already");

    // Enter opens the field on the derived title, selected.
    win.press(egui::Key::Enter);
    assert_eq!(
        win.list().field(),
        Some(&field(TITLE_ROW, "median_income", true, None))
    );

    // Typed, it is painted on the axis and is no edit.
    win.type_text(TYPED_TITLE);
    assert_eq!(win.runs_of(glyphs), 1, "the typed title is not painted");
    assert_eq!(
        win.runs_of("median_income".chars().count()),
        derived - 1,
        "the derived title is still painted beside the typed one"
    );
    assert_eq!(win.last_edit(), before.1, "a preview is no kept edit");

    // Esc puts the axis back as it was; the list is still open.
    win.press(egui::Key::Escape);
    assert_eq!(win.runs_of(glyphs), 0, "Esc left the typed title painted");
    assert_eq!(win.runs_of("median_income".chars().count()), derived);
    assert_eq!(win.last_edit(), before.1);
    assert!(win.list().field().is_none());
    assert_eq!(win.save(), saved, "a dropped title reached the file");

    // Typed again and kept with Enter.
    win.press(egui::Key::Enter);
    win.type_text(TYPED_TITLE);
    win.press(egui::Key::Enter);
    assert!(win.list().field().is_none());
    assert_eq!(
        win.attribute("xLabel"),
        Some(SpecValue::String(TYPED_TITLE.to_string()))
    );
    assert_eq!(win.runs_of(glyphs), 1);
    assert!(win.marked_unsaved());
    let written = win.save();
    assert_eq!(
        written.matches(&format!("xLabel: {TYPED_TITLE}")).count(),
        1,
        "Save did not write the kept title"
    );
    assert_ne!(written, saved);
}

/// **AC1.** `Enter` on an emptied title, in the window, is refused with the
/// field open, the axis as it was and no edit added.
#[test]
fn an_emptied_title_is_refused_in_the_window_and_adds_no_edit() {
    let mut win = Window::housing("title-empty");
    let edit_before = win.last_edit();
    win.press(egui::Key::Enter);
    win.press(egui::Key::Backspace);
    win.press(egui::Key::Enter);
    assert_eq!(
        win.list().field().and_then(|f| f.refusal.as_deref()),
        Some(TITLE_NEEDS_TEXT)
    );
    assert_eq!(win.attribute("xLabel"), None);
    assert_eq!(win.last_edit(), edit_before);
}

/// **AC1.** `⌫` on a kept title puts the derived one back: `xLabel` comes out
/// and the axis paints the column's name again.
#[test]
fn backspace_puts_the_derived_title_back_and_redraws() {
    let mut win = Window::housing("title-backspace");
    let glyphs = TYPED_TITLE.chars().count();
    win.press(egui::Key::Enter);
    win.type_text(TYPED_TITLE);
    win.press(egui::Key::Enter);
    assert_eq!(win.runs_of(glyphs), 1);
    win.press(egui::Key::Backspace);
    assert_eq!(win.attribute("xLabel"), None);
    assert_eq!(win.runs_of(glyphs), 0, "the typed title is still painted");
}

/// **AC3.** `u` after a kept title restores the one before and redraws, and
/// after two kept titles restores the first.
#[test]
fn u_after_a_kept_title_restores_the_one_before_and_redraws() {
    let mut win = Window::housing("title-undo");
    let glyphs = TYPED_TITLE.chars().count();
    let first = "Household-income";
    win.press(egui::Key::Enter);
    win.type_text(first);
    win.press(egui::Key::Enter);
    win.press(egui::Key::Enter);
    win.type_text(TYPED_TITLE);
    win.press(egui::Key::Enter);
    assert_eq!(win.runs_of(glyphs), 1);

    win.type_letter(egui::Key::U, "u");
    assert_eq!(
        win.attribute("xLabel"),
        Some(SpecValue::String(first.to_string())),
        "u did not restore the title before"
    );
    assert_eq!(win.runs_of(glyphs), 0, "the undone title is still painted");
    assert_eq!(win.runs_of(first.chars().count()), 1);

    win.type_letter(egui::Key::U, "u");
    assert_eq!(win.attribute("xLabel"), None);
    assert_eq!(win.runs_of(first.chars().count()), 0);
}

/// **AC5.** Typing the derived title is no choice: the key comes out, as `⌫`
/// takes it out, and the file names nothing it does not need.
#[test]
fn typing_the_derived_title_removes_the_key() {
    let mut win = Window::housing("title-derived");
    let saved = win.save();
    win.press(egui::Key::Enter);
    win.type_text(TYPED_TITLE);
    win.press(egui::Key::Enter);
    assert!(win.attribute("xLabel").is_some());
    win.press(egui::Key::Enter);
    win.type_text("median_income");
    win.press(egui::Key::Enter);
    assert_eq!(win.attribute("xLabel"), None);
    assert_eq!(win.save(), saved, "the derived title was written as a key");
}

/// **AC2.** A count is typed, previewed on the axis, refused when it is not one,
/// dropped by `Esc` and kept by `Enter`. Painted tick labels are counted against
/// the number the scale itself would tick at that target, and the two targets
/// are checked to differ so the count read is the typed one.
#[test]
fn a_count_is_previewed_refused_dropped_and_kept_on_the_axis() {
    let mut win = Window::housing("ticks");
    let saved = win.save();
    let at_default = win.ticks_at(DEFAULT_TICK_COUNT);
    let (at_eight, at_four) = (win.ticks_at(8), win.ticks_at(4));
    assert_ne!(
        at_default, at_eight,
        "the fixture's x axis ticks alike at 8 and at {DEFAULT_TICK_COUNT}, so a typed 8 would show nothing"
    );
    assert_eq!(win.x_tick_labels(), at_default, "the axis at rest");
    win.cursor_to_ticks();
    win.press(egui::Key::Enter);
    assert_eq!(win.list().field(), Some(&field(TICKS_ROW, "5", true, None)));

    // 0, 2.5 and abc: the field stays open, the axis and the file as they were.
    for typed in ["0", "2.5", "abc"] {
        win.press(egui::Key::Backspace);
        win.type_text(typed);
        assert!(
            win.list().field().is_some_and(|f| f.refusal.is_some()),
            "`{typed}` was not refused"
        );
        assert_eq!(win.x_tick_labels(), at_default, "`{typed}` redrew the axis");
        assert_eq!(win.attribute("xTicks"), None);
        win.press(egui::Key::Enter);
        assert!(win.list().field().is_some(), "Enter kept `{typed}`");
        assert_eq!(win.attribute("xTicks"), None);
        // Clear the field for the next: select-all is gone once typed, so
        // empty it with ⌫ past the text.
        for _ in 0..typed.len() {
            win.press(egui::Key::Backspace);
        }
    }

    // 8 is previewed, and Esc paints the axis as before.
    win.type_text("8");
    assert_eq!(win.x_tick_labels(), at_eight, "the preview is not painted");
    assert_eq!(win.attribute("xTicks"), Some(SpecValue::Integer(8)));
    win.press(egui::Key::Escape);
    assert_eq!(win.x_tick_labels(), at_default, "Esc left the preview");
    assert_eq!(win.attribute("xTicks"), None);
    assert_eq!(win.save(), saved, "a dropped count reached the file");

    // 4 and Enter write `xTicks: 4` and the axis draws that many.
    win.press(egui::Key::Enter);
    win.type_text("4");
    win.press(egui::Key::Enter);
    assert!(win.list().field().is_none());
    assert_eq!(win.attribute("xTicks"), Some(SpecValue::Integer(4)));
    assert_eq!(win.x_tick_labels(), at_four);
    assert!(win.marked_unsaved());
    let written = win.save();
    assert_eq!(
        written.matches("xTicks: 4").count(),
        saved.matches("xTicks: 4").count() + 1,
        "Save did not write the kept count"
    );
}

/// **AC3.** `u` after a kept count restores the one before and redraws, and
/// after the first restores no key at all.
#[test]
fn u_after_a_kept_count_restores_the_one_before_and_redraws() {
    let mut win = Window::housing("ticks-undo");
    let at_default = win.ticks_at(DEFAULT_TICK_COUNT);
    let (eight, twelve) = (win.ticks_at(8), win.ticks_at(12));
    assert!(
        at_default != eight && eight != twelve && at_default != twelve,
        "the fixture's x axis does not tick differently at 8, 12 and the default"
    );
    win.cursor_to_ticks();
    for typed in ["8", "12"] {
        win.press(egui::Key::Enter);
        win.type_text(typed);
        win.press(egui::Key::Enter);
    }
    assert_eq!(win.x_tick_labels(), twelve);
    win.type_letter(egui::Key::U, "u");
    assert_eq!(win.attribute("xTicks"), Some(SpecValue::Integer(8)));
    assert_eq!(
        win.x_tick_labels(),
        eight,
        "u did not redraw the count before"
    );
    win.type_letter(egui::Key::U, "u");
    assert_eq!(win.attribute("xTicks"), None);
    assert_eq!(win.x_tick_labels(), at_default);
}

/// **AC5.** Typing the default count removes the key, as `⌫` does, and the row
/// reads *auto*. `row_default`'s count is what decides it, so a default that
/// drifts from `DEFAULT_TICK_COUNT` writes `xTicks: 5` here and reddens this.
#[test]
fn typing_the_default_tick_count_removes_the_key_and_the_row_reads_auto() {
    let mut win = Window::housing("ticks-default");
    let saved = win.save();
    win.cursor_to_ticks();
    win.press(egui::Key::Enter);
    win.type_text("4");
    win.press(egui::Key::Enter);
    assert_eq!(win.attribute("xTicks"), Some(SpecValue::Integer(4)));
    assert!(win.list().setting_cursor().is_some_and(|r| r.set));

    win.press(egui::Key::Enter);
    win.type_text(&DEFAULT_TICK_COUNT.to_string());
    win.press(egui::Key::Enter);
    assert_eq!(
        win.attribute("xTicks"),
        None,
        "the default count was written as a key"
    );
    assert!(
        win.list()
            .setting_cursor()
            .is_some_and(|r| !r.set && r.value == DEFAULT_TICK_COUNT.to_string()),
        "the row does not read auto"
    );
    assert_eq!(win.save(), saved, "the default count reached the file");
}

// ---------------------------------------------------------------------------
// Pixels.
// ---------------------------------------------------------------------------

/// The Outline rail's default width.
const WIDTH: f32 = 240.0;

/// Draw `list` through the wgpu renderer and compare it with the committed
/// baseline `name`.
fn baseline_of(name: &str, mode: Mode, mut list: ColumnList) {
    let size = egui::vec2(WIDTH, 420.0);
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(2.0)
        .wgpu()
        .build_ui(move |ui| {
            design::apply(ui.ctx(), mode);
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                |ui| {
                    list.show(ui, mode);
                },
            );
        });
    harness.run();
    harness.snapshot_options(name, &SnapshotOptions::default());
}

/// The title row with its field open on the derived title, selected: the sunken
/// field ruled in the focus ink with the selection's wash behind the words, the
/// `auto` word and the marker's ring where they were, and the foot printing the
/// two keys the field answers.
fn title_field_open() -> ColumnList {
    let mut list = list_over("");
    list.feed_events(&[key_event(egui::Key::Enter)]);
    list
}

#[test]
fn the_title_field_open_light_matches_its_baseline() {
    baseline_of(
        "shelf_settings_title_field_light",
        Mode::Light,
        title_field_open(),
    );
}

#[test]
fn the_title_field_open_dark_matches_its_baseline() {
    baseline_of(
        "shelf_settings_title_field_dark",
        Mode::Dark,
        title_field_open(),
    );
}

/// The ticks row with `2.5` typed: the field holds the text with its caret, and
/// the sentence that refuses it stands under the row in full ink.
fn ticks_refused() -> ColumnList {
    let mut list = list_on_ticks("");
    list.feed_events(&[key_event(egui::Key::Enter), text("2.5")]);
    list
}

#[test]
fn a_refused_count_light_matches_its_baseline() {
    baseline_of(
        "shelf_settings_ticks_refused_light",
        Mode::Light,
        ticks_refused(),
    );
}

#[test]
fn a_refused_count_dark_matches_its_baseline() {
    baseline_of(
        "shelf_settings_ticks_refused_dark",
        Mode::Dark,
        ticks_refused(),
    );
}
