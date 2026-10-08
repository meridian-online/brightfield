//! **The format row offers presets with a custom specifier behind them, and
//! shows what the axis's largest tick prints as.**
//!
//! `h` and `l` on the format row step auto, number, short, percent, currency and
//! custom. Each preset writes its specifier to the file and the axis redraws;
//! `l` onto custom, and `Enter` on any preset, open a field on the current
//! specifier, selected. The row's value carries, in muted ink, what the axis's top
//! drawn tick prints as under the row's preset. A specifier no preset matches
//! reads as custom, and the field refuses a type letter d3-format does not name,
//! saying which letters it takes.
//!
//! Three altitudes, each reading what the thing under test produced:
//!
//! - **The list alone**: the reports a key answers with, the field's state, the
//!   row's value, sample and foot, since a write the list does not report is one
//!   the chart never gets.
//! - **The window**: the attribute the plot holds and what the scene *paints*:
//!   the glyph count of each x tick label, held against the labels the render
//!   crate's own ticks print under the format, so a preset that wrote its key and
//!   did not redraw is a test that reddens.
//! - **Pixels**: the row on short, with custom open and with a file's `$,.2f`, in
//!   both themes. Regenerate them with `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p
//!   brightfield-shell --test shelf_settings_format`, and read what moved before
//!   committing it.

use brightfield_render::axis::compute_ticks_formatted;
use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_render::text::LABEL_SIZE;
use brightfield_shell::app::CHART;
use brightfield_shell::design::{self, Mode};
use brightfield_shell::pipeline::Composed;
use brightfield_shell::shelf::{
    foot_sentence, format_preset, format_specifier, Binding, ChannelSettings, ColumnList,
    ColumnListRequest, ListColumn, ListReport, ListTab, RowEdit, RowField, SettingRow,
    SettingValue, ShelfChannels, AUTO, FORMAT_PRESETS, FORMAT_ROW, TITLE_ROW,
};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::ast::SpecValue;
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::layout::AxisFormat;
use brightfield_spec::number_format::NumberFormat;
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

/// A letter as a keyboard brings it: the key, then the text it types.
fn typed(key: egui::Key, letter: &str) -> Vec<egui::Event> {
    vec![key_event(key), text(letter)]
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

/// The x axis as the chart drew it for a column that runs 0 to `domain_max`: the
/// scale a window hands the settings, which the row's sample is read off.
fn drawn_to(domain_max: f64) -> ScaleSet {
    let mut set = ScaleSet::new();
    set.insert(
        Channel::X,
        Scale::Linear {
            domain_min: 0.0,
            domain_max,
            range_start: 40.0,
            range_end: 600.0,
        },
    );
    set
}

fn settings_over(attrs: &str, drawn: &ScaleSet) -> ChannelSettings {
    let spec = parse_spec(&format!("{SOURCE}{attrs}\n"), Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    ChannelSettings::of_plot_drawn(&spec, plot, &channels(), drawn)
}

fn format_row(attrs: &str, drawn: &ScaleSet) -> SettingRow {
    settings_over(attrs, drawn)
        .rows(ShelfChannel::X)
        .iter()
        .find(|row| row.name == FORMAT_ROW)
        .expect("x has a format row")
        .clone()
}

/// A list on x's settings over a plot that writes `attrs`, drawn to `drawn`, the
/// cursor on the format row, which is the third.
fn list_on_format_drawn(attrs: &str, drawn: &ScaleSet) -> ColumnList {
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
    list.set_settings(settings_over(attrs, drawn));
    list.feed_events(&[key_event(egui::Key::Tab)]);
    assert_eq!(list.tab(), ListTab::Settings, "Tab turned the list");
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(TITLE_ROW));
    list.feed_events(&typed(egui::Key::J, "j"));
    list.feed_events(&typed(egui::Key::J, "j"));
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(FORMAT_ROW));
    list
}

fn list_on_format(attrs: &str) -> ColumnList {
    list_on_format_drawn(attrs, &ScaleSet::new())
}

fn set(value: SettingValue) -> ListReport {
    ListReport::Set(RowEdit {
        channel: ShelfChannel::X,
        row: FORMAT_ROW,
        value,
    })
}

fn specifier(written: &str) -> SettingValue {
    SettingValue::Text(written.to_string())
}

fn field(typed: &str, selected: bool, refusal: Option<&str>) -> RowField {
    RowField {
        row: FORMAT_ROW,
        text: typed.to_string(),
        selected,
        refusal: refusal.map(str::to_string),
    }
}

/// **AC1.** `l` steps auto, number, short, percent and currency, and each writes
/// the specifier its name stands for: `,f`, `~s`, `%` and `$,f`.
#[test]
fn l_steps_through_the_presets_and_each_writes_its_specifier() {
    let steps: [(&str, ListReport); 4] = [
        ("", set(specifier(",f"))),
        ("xTickFormat: ',f'", set(specifier("~s"))),
        ("xTickFormat: '~s'", set(specifier("%"))),
        ("xTickFormat: '%'", set(specifier("$,f"))),
    ];
    for (attrs, expected) in steps {
        let mut list = list_on_format(attrs);
        let reports = list.feed_events(&typed(egui::Key::L, "l"));
        assert_eq!(reports, [expected], "`l` over `{attrs}`");
        assert!(list.field().is_none(), "`l` over `{attrs}` opened no field");
    }
}

/// **AC1.** `h` steps back, and back from the first preset puts the key out: the
/// number preset steps to auto, which writes nothing and takes the key away.
#[test]
fn h_steps_back_through_the_presets_and_number_steps_to_auto() {
    let steps: [(&str, Option<ListReport>); 5] = [
        ("xTickFormat: ',f'", Some(set(SettingValue::Auto))),
        ("xTickFormat: '~s'", Some(set(specifier(",f")))),
        ("xTickFormat: '%'", Some(set(specifier("~s")))),
        ("xTickFormat: '$,f'", Some(set(specifier("%")))),
        ("", None),
    ];
    for (attrs, expected) in steps {
        let mut list = list_on_format(attrs);
        let reports = list.feed_events(&typed(egui::Key::H, "h"));
        assert_eq!(
            reports,
            expected.into_iter().collect::<Vec<_>>(),
            "`h` over `{attrs}`"
        );
    }
}

/// **AC1.** `l` onto custom opens the field on the current specifier, selected,
/// and writes nothing yet; `Esc` leaves the row as it was.
#[test]
fn l_onto_custom_opens_the_field_on_the_current_specifier_selected() {
    let mut list = list_on_format("xTickFormat: '$,f'");
    let reports = list.feed_events(&typed(egui::Key::L, "l"));
    assert_eq!(reports, [], "stepping onto custom writes nothing");
    assert_eq!(list.field(), Some(&field("$,f", true, None)));
    assert!(list.typing(), "the open field has the keys");
    list.feed_events(&[key_event(egui::Key::Escape)]);
    assert!(list.field().is_none(), "Esc drops the field");
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(FORMAT_ROW));
}

/// **AC1.** `Enter` on any preset opens the same field, on the specifier the file
/// holds; on auto the file holds none, so the field opens empty.
#[test]
fn enter_on_any_preset_opens_the_field_on_its_specifier() {
    let rows = [
        ("", "", false),
        ("xTickFormat: ',f'", ",f", true),
        ("xTickFormat: '~s'", "~s", true),
        ("xTickFormat: '%'", "%", true),
        ("xTickFormat: '$,f'", "$,f", true),
        ("xTickFormat: '$,.2f'", "$,.2f", true),
    ];
    for (attrs, held, selected) in rows {
        let mut list = list_on_format(attrs);
        let reports = list.feed_events(&[key_event(egui::Key::Enter)]);
        assert_eq!(reports, [], "Enter over `{attrs}` writes nothing");
        assert_eq!(
            list.field(),
            Some(&field(held, selected, None)),
            "Enter over `{attrs}`"
        );
    }
}

/// **AC1.** The foot names the specifier the file gets, on a preset and on custom.
#[test]
fn the_foot_names_the_specifier_the_file_gets() {
    let drawn = ScaleSet::new();
    let auto = foot_sentence(&format_row("", &drawn));
    assert!(
        !auto.contains("Writes"),
        "auto writes no specifier, and the foot says none: {auto}"
    );
    for (attrs, name, written) in [
        ("xTickFormat: ',f'", "number", ",f"),
        ("xTickFormat: '~s'", "short", "~s"),
        ("xTickFormat: '%'", "percent", "%"),
        ("xTickFormat: '$,f'", "currency", "$,f"),
    ] {
        let row = format_row(attrs, &drawn);
        assert_eq!(row.value, name);
        let foot = foot_sentence(&row);
        assert!(
            foot.contains(&format!("Writes xTickFormat: \"{written}\".")),
            "the foot over `{attrs}` names the key and the specifier: {foot}"
        );
    }
    for name in ["auto", "number", "short", "percent", "currency", "custom"] {
        assert!(auto.contains(name), "the foot lists {name}: {auto}");
    }
}

/// **AC3.** A specifier no preset matches, `$,.2f`, reads as custom, set, with the
/// specifier the file holds, and the foot says what `h` and `l` would do to it.
#[test]
fn a_specifier_no_preset_matches_reads_as_custom_and_the_foot_says_what_h_and_l_do() {
    let row = format_row("xTickFormat: '$,.2f'", &ScaleSet::new());
    assert_eq!((row.value.as_str(), row.set), ("custom", true));
    assert_eq!(
        row.format.as_ref().and_then(|f| f.specifier.as_deref()),
        Some("$,.2f")
    );
    let foot = foot_sentence(&row);
    for expected in [
        "Writes xTickFormat: \"$,.2f\"",
        "h leaves \"$,.2f\" for currency",
        "l has nowhere to go",
        "u takes the change back",
    ] {
        assert!(
            foot.contains(expected),
            "the foot says `{expected}`: {foot}"
        );
    }
    let mut list = list_on_format("xTickFormat: '$,.2f'");
    assert_eq!(
        list.feed_events(&typed(egui::Key::L, "l")),
        [],
        "`l` has nowhere to go from custom"
    );
    assert!(list.field().is_none(), "`l` from custom opens no field");
    assert_eq!(
        list.feed_events(&typed(egui::Key::H, "h")),
        [set(specifier("$,f"))],
        "`h` leaves custom for currency"
    );
}

/// **AC4.** A type letter d3-format does not name is refused under the row, naming
/// the letters it takes, and the field stays open: `Enter` keeps nothing.
#[test]
fn a_type_letter_d3_format_does_not_name_is_refused_naming_the_letters_and_stays_open() {
    let mut list = list_on_format("xTickFormat: ',f'");
    list.feed_events(&[key_event(egui::Key::Enter)]);
    let reports = list.feed_events(&[text(".2q")]);
    assert_eq!(
        reports,
        [ListReport::Preview(None)],
        "a refused specifier previews nothing"
    );
    let refusal = list
        .field()
        .and_then(|f| f.refusal.clone())
        .expect("the field says why it refuses .2q");
    assert!(
        refusal.contains("\".2q\"") && refusal.contains("% b c d e f g n o p r s X x"),
        "the refusal quotes what was typed and names the letters: {refusal}"
    );
    let reports = list.feed_events(&[key_event(egui::Key::Enter)]);
    assert_eq!(reports, [], "Enter keeps no refused specifier");
    assert_eq!(
        list.field().map(|f| (f.text.as_str(), f.refusal.is_some())),
        Some((".2q", true)),
        "the field stays open on what was typed, refused"
    );
    // The refusal clears once the text names a type again, and Enter then keeps it.
    list.feed_events(&[key_event(egui::Key::Backspace)]);
    let reports = list.feed_events(&[text("f")]);
    assert_eq!(
        reports,
        [ListReport::Preview(Some(RowEdit {
            channel: ShelfChannel::X,
            row: FORMAT_ROW,
            value: specifier(".2f"),
        }))],
        "a specifier the row keeps is previewed on the axis as it is typed"
    );
    assert_eq!(list.field().and_then(|f| f.refusal.clone()), None);
    let reports = list.feed_events(&[key_event(egui::Key::Enter)]);
    assert_eq!(reports, [set(specifier(".2f"))]);
    assert!(
        list.field().is_none(),
        "Enter keeps it and closes the field"
    );
}

/// **AC4.** Text that is no specifier at all is refused as well, in words that
/// say it is no specifier; an empty field is refused on `Enter` with the letters.
#[test]
fn text_that_is_no_specifier_is_refused_and_an_empty_field_says_what_it_takes() {
    let mut list = list_on_format("xTickFormat: ',f'");
    list.feed_events(&[key_event(egui::Key::Enter)]);
    list.feed_events(&[text("~~")]);
    let refusal = list
        .field()
        .and_then(|f| f.refusal.clone())
        .expect("refused");
    assert!(
        refusal.contains("\"~~\" is not a specifier"),
        "the refusal says it is no specifier: {refusal}"
    );
    list.feed_events(&[key_event(egui::Key::Backspace)]);
    list.feed_events(&[key_event(egui::Key::Backspace)]);
    list.feed_events(&[key_event(egui::Key::Backspace)]);
    let reports = list.feed_events(&[key_event(egui::Key::Enter)]);
    assert_eq!(reports, [], "an empty field keeps nothing");
    let refusal = list
        .field()
        .and_then(|f| f.refusal.clone())
        .expect("refused");
    assert!(
        refusal.contains("% b c d e f g n o p r s X x"),
        "an empty field names the letters: {refusal}"
    );
}

/// **AC4.** The field's judge and the reader take the same specifiers apart at the
/// type: the reader takes a letter that names no type (the row reads the file's
/// `.2q` as custom, set, since the axis draws it), and the field refuses it.
#[test]
fn the_field_refuses_what_the_reader_takes_exactly_at_a_type_no_format_names() {
    const NAMED: &str = "%bcdefgnoprsXx";
    for letter in ('a'..='z').chain('A'..='Z').chain(['%']) {
        let written = format!(".2{letter}");
        let kept = format_specifier(&written);
        assert_eq!(
            kept.is_ok(),
            NAMED.contains(letter),
            "the field over `{written}`: {kept:?}"
        );
        assert_eq!(
            NumberFormat::parse(&written).is_some(),
            true,
            "the reader takes `{written}`"
        );
        let row = format_row(&format!("xTickFormat: '{written}'"), &ScaleSet::new());
        assert!(
            row.set,
            "a file's `{written}` is drawn, so the row reads it"
        );
        assert_eq!(
            row.value,
            format_preset(&written),
            "a file's `{written}` reads as the preset it equals, or custom"
        );
    }
}

/// **AC1, AC3.** The presets the row steps are the presets it names, in the order
/// the foot lists them, and each specifier is one the row reads back as its name.
#[test]
fn each_preset_reads_back_as_its_own_name() {
    let names: Vec<&str> = FORMAT_PRESETS.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        names,
        ["auto", "number", "short", "percent", "currency", "custom"]
    );
    for (name, written) in FORMAT_PRESETS {
        if let Some(written) = written {
            assert_eq!(format_preset(written), name);
            assert!(
                format_specifier(written).is_ok(),
                "the field would keep `{written}`, the specifier of {name}"
            );
            let row = format_row(&format!("xTickFormat: '{written}'"), &ScaleSet::new());
            assert_eq!((row.value.as_str(), row.set), (name, true));
        }
    }
    let row = format_row("", &ScaleSet::new());
    assert_eq!((row.value.as_str(), row.set), (AUTO, false));
}

/// **AC2.** The sample is what the axis's top drawn tick prints as under the row's
/// preset, read off the scale the chart was drawn against: a column that runs to
/// 5,565 draws its top tick at 5,000, and the sample under percent is that tick as
/// a percent, not the column's maximum.
#[test]
fn the_sample_is_the_top_drawn_tick_under_the_preset_and_not_the_columns_maximum() {
    let drawn = drawn_to(5565.0);
    let cases = [
        ("", "5000"),
        ("xTickFormat: ',f'", "5,000"),
        ("xTickFormat: '~s'", "5k"),
        ("xTickFormat: '%'", "500000%"),
        ("xTickFormat: '$,f'", "$5,000"),
        ("xTickFormat: '$,.2f'", "$5,000.00"),
    ];
    for (attrs, expected) in cases {
        let row = format_row(attrs, &drawn);
        assert_eq!(
            row.format.and_then(|f| f.sample).as_deref(),
            Some(expected),
            "the sample over `{attrs}`"
        );
    }
    // The column's maximum, printed at the step the axis draws its ticks at, is a
    // different text from the top tick's, so a sample read off the maximum would
    // be told from one read off the tick.
    let at_the_step = NumberFormat::parse("%")
        .expect("percent")
        .tick_format(0.0, 5565.0, 1000.0);
    assert_eq!(at_the_step.format(5565.0), "556500%");
    assert_eq!(at_the_step.format(5000.0), "500000%");
    assert_ne!(
        format_row("xTickFormat: '%'", &drawn)
            .format
            .and_then(|f| f.sample)
            .as_deref(),
        Some("556500%"),
        "the sample is not the column's maximum"
    );
    // With no scale drawn there is no tick to read, so the row carries no sample.
    assert_eq!(
        format_row("xTickFormat: '%'", &ScaleSet::new())
            .format
            .and_then(|f| f.sample),
        None
    );
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
            "bf-shelf-format-{name}-{}-{}",
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

/// One headless window over a copy of the housing sample, with population kept
/// on x so the plot has an axis to set, and x's settings list open on its format
/// row.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
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
            _root: root,
        };
        win.settle();
        assert!(
            win.app.focus_pane(PaneKey::new(CHART)),
            "the pane takes focus"
        );
        win.settle();
        win.keep_population_on_x();
        win.type_letter(egui::Key::X, "x");
        win.press(egui::Key::Tab);
        assert_eq!(
            win.app.protocol_model().column_list().map(ColumnList::tab),
            Some(ListTab::Settings),
            "Tab did not turn x's list to its settings"
        );
        win.type_letter(egui::Key::J, "j");
        win.type_letter(egui::Key::J, "j");
        assert_eq!(
            win.list().setting_cursor().map(|r| r.name),
            Some(FORMAT_ROW),
            "two steps down from the title reach the format row"
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

    fn type_text(&mut self, typed: &str) {
        self.run(vec![text(typed)]);
        self.run(Vec::new());
    }

    /// `e`, `x`, to `population`, `Enter`: the column kept on x, which takes the
    /// map's projection out and leaves a plot with axes to set.
    fn keep_population_on_x(&mut self) {
        self.type_letter(egui::Key::E, "e");
        self.type_letter(egui::Key::X, "x");
        for (key, letter) in [(egui::Key::K, "k"), (egui::Key::J, "j")] {
            for _ in 0..14 {
                if self
                    .app
                    .protocol_model()
                    .column_list()
                    .and_then(|l| l.cursor().map(str::to_owned))
                    .as_deref()
                    == Some("population")
                {
                    break;
                }
                self.type_letter(key, letter);
            }
        }
        assert_eq!(
            self.app
                .protocol_model()
                .column_list()
                .and_then(|l| l.cursor().map(str::to_owned))
                .as_deref(),
            Some("population"),
            "the list's cursor reached population"
        );
        self.press(egui::Key::Enter);
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

    /// The glyph count of each tick label the hero paints under its x axis,
    /// sorted: what the scene shows, not what the resolver was handed.
    fn x_tick_glyphs(&self) -> Vec<usize> {
        let composed = self.composed();
        let plot = &composed.plots[0];
        let row = plot.rect.y + plot.layout.plot_y_end() + 10.0;
        let (left, right) = (
            plot.rect.x + plot.layout.plot_x_start() - 20.0,
            plot.rect.x + plot.layout.plot_x_end() + 20.0,
        );
        let mut counts: Vec<usize> = composed
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
            .map(|run| run.glyphs.len())
            .collect();
        counts.sort_unstable();
        counts
    }

    /// The tick text the render crate's own axis prints for the hero's x scale
    /// under `format`, as a sorted list of glyph counts.
    fn ticks_glyphs_under(&self, format: Option<&str>) -> Vec<usize> {
        let scale = self.composed().plots[0]
            .scales
            .get(Channel::X)
            .expect("the hero's x scale");
        let format = format.map(|s| AxisFormat::Number(NumberFormat::parse(s).expect("a format")));
        let mut counts: Vec<usize> = compute_ticks_formatted(scale, 5, format.as_ref())
            .iter()
            .map(|tick| tick.label.chars().count())
            .collect();
        counts.sort_unstable();
        counts
    }

    fn row(&self) -> SettingRow {
        self.list().setting_cursor().expect("a row").clone()
    }

    fn last_edit(&self) -> Option<String> {
        self.app.chart_doc().last_shelf_edit().map(str::to_owned)
    }
}

/// **AC1, AC2.** On population, `l` four times writes `,f`, `~s`, `%` and `$,f` in
/// turn, the plot holds each, the row reads its name, and the axis paints the tick
/// text the render crate prints under it. Under percent the sample is the top
/// drawn tick as a percent, and the tick is below the column's maximum.
#[test]
fn l_writes_each_preset_to_the_plot_and_the_axis_repaints_under_it() {
    let mut win = Window::housing("presets");
    assert_eq!(win.attribute("xTickFormat"), None);
    assert_eq!(win.row().value, AUTO);
    let plain = win.x_tick_glyphs();
    assert_eq!(
        plain,
        win.ticks_glyphs_under(None),
        "auto paints the axis's own text"
    );

    let mut painted = vec![plain];
    for (name, written) in [
        ("number", ",f"),
        ("short", "~s"),
        ("percent", "%"),
        ("currency", "$,f"),
    ] {
        win.type_letter(egui::Key::L, "l");
        assert_eq!(
            win.attribute("xTickFormat"),
            Some(SpecValue::String(written.to_string())),
            "the plot holds {name}'s specifier"
        );
        assert_eq!(win.row().value, name, "the row reads {name}");
        let glyphs = win.x_tick_glyphs();
        assert_eq!(
            glyphs,
            win.ticks_glyphs_under(Some(written)),
            "the axis paints the text {name} prints"
        );
        painted.push(glyphs);
    }
    painted.dedup();
    assert!(
        painted.len() >= 4,
        "the presets do not all paint alike, so the check can tell them apart: {painted:?}"
    );

    // Percent again: the sample is the top drawn tick read as a percent.
    win.type_letter(egui::Key::H, "h");
    win.type_letter(egui::Key::H, "h");
    assert_eq!(win.row().value, "short");
    win.type_letter(egui::Key::L, "l");
    let row = win.row();
    assert_eq!(row.value, "percent");
    let scale = win.composed().plots[0]
        .scales
        .get(Channel::X)
        .expect("the hero's x scale");
    let Scale::Linear {
        domain_min,
        domain_max,
        ..
    } = scale
    else {
        panic!("population draws on a linear scale");
    };
    let mut ticks = compute_ticks_formatted(
        scale,
        5,
        Some(&AxisFormat::Number(
            NumberFormat::parse("%").expect("percent"),
        )),
    );
    ticks.sort_by(|a, b| b.value.total_cmp(&a.value));
    let (top, next) = (&ticks[0], &ticks[1]);
    assert!(
        top.value < 5565.0,
        "the axis's top tick ({}) is below the column's maximum of 5,565",
        top.value
    );
    let maximum = NumberFormat::parse("%")
        .expect("percent")
        .tick_format(*domain_min, *domain_max, top.value - next.value)
        .format(5565.0);
    let sample = row
        .format
        .and_then(|f| f.sample)
        .expect("a drawn axis has a sample");
    assert_eq!(
        sample, top.label,
        "the sample is the top drawn tick as a percent"
    );
    assert_ne!(sample, maximum, "and not the column's maximum");
}

/// **AC1, AC3, AC4.** A specifier typed into the field is previewed on the axis,
/// refused when its type is no format's and kept by `Enter`; it reads as custom,
/// and `h` leaves it for currency where `u` puts the typed specifier back.
#[test]
fn a_typed_specifier_is_previewed_refused_kept_and_put_back_by_u_after_a_preset() {
    let mut win = Window::housing("typed");
    // Four `l`s reach currency; the fifth opens the field on `$,f`, selected.
    for _ in 0..4 {
        win.type_letter(egui::Key::L, "l");
    }
    assert_eq!(win.row().value, "currency");
    win.type_letter(egui::Key::L, "l");
    assert_eq!(
        win.list().field().map(|f| (f.text.as_str(), f.selected)),
        Some(("$,f", true)),
        "`l` onto custom opens the field on the current specifier, selected"
    );
    assert_eq!(
        win.attribute("xTickFormat"),
        Some(SpecValue::String("$,f".to_string())),
        "opening the field changes nothing"
    );

    let edits_before = win.last_edit();
    // A type that names no format is refused and draws nothing of its own.
    win.type_text("$,.2q");
    assert!(
        win.list().field().and_then(|f| f.refusal.clone()).is_some(),
        "$,.2q is refused"
    );
    assert_eq!(
        win.x_tick_glyphs(),
        win.ticks_glyphs_under(Some("$,f")),
        "a refused specifier previews nothing: the axis still paints currency"
    );

    // A specifier that names one is previewed as it is typed.
    win.press(egui::Key::Backspace);
    win.type_text("f");
    assert_eq!(win.list().field().and_then(|f| f.refusal.clone()), None);
    assert_eq!(
        win.x_tick_glyphs(),
        win.ticks_glyphs_under(Some("$,.2f")),
        "the axis paints the typed specifier before it is kept"
    );
    assert_eq!(
        win.last_edit(),
        edits_before,
        "the preview is no edit: the last edit the shelf holds is the one before it"
    );
    win.press(egui::Key::Enter);
    assert!(win.list().field().is_none());
    assert_eq!(
        win.attribute("xTickFormat"),
        Some(SpecValue::String("$,.2f".to_string()))
    );
    assert_eq!(
        win.row().value,
        "custom",
        "a specifier no preset writes reads as custom"
    );

    // `h` leaves custom for currency, and `u` takes the typed specifier back.
    win.type_letter(egui::Key::H, "h");
    assert_eq!(
        win.attribute("xTickFormat"),
        Some(SpecValue::String("$,f".to_string()))
    );
    assert_eq!(win.row().value, "currency");
    win.type_letter(egui::Key::U, "u");
    assert_eq!(
        win.attribute("xTickFormat"),
        Some(SpecValue::String("$,.2f".to_string())),
        "`u` after a preset replaced a custom specifier puts the specifier back"
    );
    assert_eq!(win.row().value, "custom");
    assert!(win.last_edit().is_some());
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

/// The format row on short over a population axis that runs to 35,682: the preset's
/// name, its sample, `30k`, in muted ink, and the foot naming `~s`.
fn short() -> ColumnList {
    list_on_format_drawn("xTickFormat: '~s'", &drawn_to(35_682.0))
}

/// The format row over a file's `,d`, which no preset writes, with `Enter` having
/// opened the field on it: the specifier selected, in the sunken field ruled in the
/// focus ink, and the foot printing what the field takes and the two keys.
fn custom_open() -> ColumnList {
    let mut list = list_on_format_drawn("xTickFormat: ',d'", &drawn_to(35_682.0));
    list.feed_events(&[key_event(egui::Key::Enter)]);
    assert_eq!(list.field(), Some(&field(",d", true, None)));
    list
}

/// The format row over a file's `$,.2f`: custom, the sample `$30,000.00`, and the
/// foot saying what `h` and `l` do to it.
fn custom_from_file() -> ColumnList {
    list_on_format_drawn("xTickFormat: '$,.2f'", &drawn_to(35_682.0))
}

#[test]
fn the_format_row_on_short_light_matches_its_baseline() {
    baseline_of("shelf_settings_format_short_light", Mode::Light, short());
}

#[test]
fn the_format_row_on_short_dark_matches_its_baseline() {
    baseline_of("shelf_settings_format_short_dark", Mode::Dark, short());
}

#[test]
fn the_format_custom_field_open_light_matches_its_baseline() {
    baseline_of(
        "shelf_settings_format_custom_open_light",
        Mode::Light,
        custom_open(),
    );
}

#[test]
fn the_format_custom_field_open_dark_matches_its_baseline() {
    baseline_of(
        "shelf_settings_format_custom_open_dark",
        Mode::Dark,
        custom_open(),
    );
}

#[test]
fn the_format_row_on_a_files_custom_light_matches_its_baseline() {
    baseline_of(
        "shelf_settings_format_custom_file_light",
        Mode::Light,
        custom_from_file(),
    );
}

#[test]
fn the_format_row_on_a_files_custom_dark_matches_its_baseline() {
    baseline_of(
        "shelf_settings_format_custom_file_dark",
        Mode::Dark,
        custom_from_file(),
    );
}
