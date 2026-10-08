//! **An axis's settings row is set by key and by click, and the chart redraws.**
//!
//! `h` and `l` (and `←` `→`) step a scale or a switch row, a click on a row's
//! value does what `l` does, and `⌫` puts a set row back to auto. The list
//! reports the write, the window makes it through the shelf's edit path and
//! draws the chart at once, and `u` steps over it.
//!
//! Three kinds of test, each reading what the thing under test produced: the
//! list alone (what its keys and clicks report, what it draws), the edit path
//! alone (the spec and the text it writes), and the window (the chart it draws,
//! the status band, the unsaved mark, the file Save writes).

use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_shell::app::CHART;
use brightfield_shell::design::{self, Mode};
use brightfield_shell::shelf::{
    foot_sentence, Binding, ChannelSettings, ColumnList, ColumnListRequest, ListColumn, ListDrawn,
    ListReport, ListTab, RowEdit, SettingRow, SettingValue, ShelfChannels, GRID_ROW, REVERSE_ROW,
    SCALE_ROW, ZERO_ROW,
};
use brightfield_shell::shelf_edit::{put_setting, SettingWrite};
use brightfield_shell::startup::default_layout;
use brightfield_shell::text_ink;
use brightfield_shell::window::{Boot, MeridianApp, SHELF_EDIT_STATUS_ID, UNSAVED_MARK};
use brightfield_protocol::{write_chart_edit, ChartTextRefusal};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::{Spec, SpecValue};
use brightfield_spec::edit::{plot_at_path, ChartEdit};
use brightfield_spec::{parse_spec, Format};
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::PaneKey;

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

fn channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("population".to_string()),
        y: Binding::Column("latitude".to_string()),
        colour: Binding::Unset,
    }
}

const SOURCE: &str = "data:\n  t:\n    - { a: 1 }\nplot:\n  - mark: dot\n    data: { from: t }\n    x: a\n    y: a\nwidth: 600\nheight: 300\n";

fn spec_of(attrs: &str) -> Spec {
    parse_spec(&format!("{SOURCE}{attrs}\n"), Format::Yaml)
        .expect("the spec parses")
        .spec
}

fn settings_of(attrs: &str, drawn: &ScaleSet) -> ChannelSettings {
    let spec = spec_of(attrs);
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    ChannelSettings::of_plot_drawn(&spec, plot, &channels(), drawn)
}

/// A list on x's settings over a plot that writes `attrs`, drawn against
/// `drawn`, the cursor on the first row.
fn list_over(attrs: &str, drawn: &ScaleSet) -> ColumnList {
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
    list.set_settings(settings_of(attrs, drawn));
    list.feed_events(&[key_event(egui::Key::Tab)]);
    assert_eq!(list.tab(), ListTab::Settings, "Tab turned the list");
    list
}

fn list_with(attrs: &str) -> ColumnList {
    list_over(attrs, &ScaleSet::new())
}

/// Put the cursor on the settings row `name` the way an analyst does: `/`, its
/// name, `Enter`, which ends the query's typing and keeps the rows it found.
fn find(list: &mut ColumnList, name: &str) {
    list.feed_events(&[key_event(egui::Key::Slash)]);
    for ch in name.chars() {
        list.feed_events(&[egui::Event::Text(ch.to_string())]);
    }
    list.feed_events(&[key_event(egui::Key::Enter)]);
    assert_eq!(
        list.setting_cursor().map(|r| r.name),
        Some(match name {
            "grid" => GRID_ROW,
            "zero" => ZERO_ROW,
            "reverse" => REVERSE_ROW,
            other => panic!("{other} is not a row found by name"),
        })
    );
}

fn set(row: &'static str, value: SettingValue) -> ListReport {
    ListReport::Set(RowEdit {
        channel: ShelfChannel::X,
        row,
        value,
    })
}

fn word(name: &str) -> SettingValue {
    SettingValue::Word(name.to_string())
}

/// **AC1.** On the scale row `l` steps linear to log to symlog and stops there,
/// `h` steps back, and `→` and `←` do what `l` and `h` do.
#[test]
fn l_steps_the_scale_forward_and_h_back_and_the_arrows_do_the_same() {
    let step = |attrs: &str, key: egui::Key| {
        let mut list = list_with(attrs);
        list.feed_events(&[key_event(egui::Key::J)]);
        assert_eq!(list.setting_cursor().map(|r| r.name), Some(SCALE_ROW));
        list.feed_events(&[key_event(key)])
    };
    assert_eq!(step("", egui::Key::L), [set(SCALE_ROW, word("log"))]);
    assert_eq!(
        step("xScale: log", egui::Key::L),
        [set(SCALE_ROW, word("symlog"))]
    );
    assert_eq!(step("xScale: symlog", egui::Key::L), []);
    assert_eq!(
        step("xScale: symlog", egui::Key::H),
        [set(SCALE_ROW, word("log"))]
    );
    assert_eq!(
        step("xScale: log", egui::Key::H),
        [set(SCALE_ROW, word("linear"))]
    );
    assert_eq!(step("", egui::Key::H), []);
    assert_eq!(
        step("", egui::Key::ArrowRight),
        [set(SCALE_ROW, word("log"))]
    );
    assert_eq!(
        step("xScale: log", egui::Key::ArrowLeft),
        [set(SCALE_ROW, word("linear"))]
    );
}

/// **AC1.** Grid, zero and reverse switch their value on `h` and on `l`, found
/// by their names, and the arrows step them from the query line too.
#[test]
fn h_and_l_turn_a_switch_row_over() {
    let turn = |attrs: &str, row: &str, key: egui::Key| {
        let mut list = list_with(attrs);
        find(&mut list, row);
        list.feed_events(&[key_event(key)])
    };
    assert_eq!(
        turn("", "grid", egui::Key::H),
        [set(GRID_ROW, SettingValue::Switch(false))]
    );
    assert_eq!(
        turn("xGrid: false", "grid", egui::Key::L),
        [set(GRID_ROW, SettingValue::Switch(true))]
    );
    assert_eq!(
        turn("", "zero", egui::Key::L),
        [set(ZERO_ROW, SettingValue::Switch(true))]
    );
    assert_eq!(
        turn("", "reverse", egui::Key::ArrowRight),
        [set(REVERSE_ROW, SettingValue::Switch(true))]
    );
    // From the query line, where `l` is a letter, the arrow is the step.
    let mut list = list_with("");
    list.feed_events(&[key_event(egui::Key::Slash)]);
    list.feed_events(&[egui::Event::Text("zero".to_string())]);
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::ArrowRight)]),
        [set(ZERO_ROW, SettingValue::Switch(true))]
    );
}

/// A row the render crate's judge says does not apply to the axis drawn: zero
/// on a log axis.
fn muted_zero() -> ColumnList {
    let mut drawn = ScaleSet::new();
    drawn.insert(
        Channel::X,
        Scale::Log {
            domain_min: 1.0,
            domain_max: 1000.0,
            range_start: 0.0,
            range_end: 100.0,
        },
    );
    list_over("xScale: log\nxZero: true", &drawn)
}

/// **AC1.** On a row drawn muted as not applying, `h`, `l`, `←`, `→`, `Enter`
/// and `⌫` change nothing, and the foot reads the reason.
#[test]
fn a_row_that_does_not_apply_changes_nothing_and_the_foot_gives_the_reason() {
    let mut list = muted_zero();
    find(&mut list, "zero");
    let row = list.setting_cursor().expect("the cursor is on zero").clone();
    let reason = row.reason.clone().expect("zero does not apply on a log axis");
    for key in [
        egui::Key::H,
        egui::Key::L,
        egui::Key::ArrowLeft,
        egui::Key::ArrowRight,
        egui::Key::Enter,
        egui::Key::Backspace,
    ] {
        assert_eq!(
            list.feed_events(&[key_event(key)]),
            [],
            "{key:?} wrote to a row that does not apply"
        );
    }
    let foot = foot_sentence(&row);
    assert!(
        foot.to_lowercase().starts_with(&reason.to_lowercase()),
        "the foot reads {foot:?}, not the reason {reason:?}"
    );
}

/// **AC3.** `⌫` on a set row reports the row back to auto, and while the query
/// has the keys it edits the query and reports nothing.
#[test]
fn backspace_puts_a_set_row_back_to_auto_and_edits_the_query_while_it_is_typed() {
    let mut list = list_with("xScale: log");
    list.feed_events(&[key_event(egui::Key::J)]);
    assert_eq!(
        list.feed_events(&[key_event(egui::Key::Backspace)]),
        [set(SCALE_ROW, SettingValue::Auto)]
    );
    list.feed_events(&[key_event(egui::Key::Slash)]);
    list.feed_events(&[egui::Event::Text("gri".to_string())]);
    assert_eq!(list.feed_events(&[key_event(egui::Key::Backspace)]), []);
    assert_eq!(list.query(), "gr", "⌫ took a letter off the query");
}

/// One headless frame of the list, with `events`.
struct Stage {
    ctx: egui::Context,
}

impl Stage {
    fn new() -> Self {
        let ctx = egui::Context::default();
        design::apply(&ctx, Mode::Light);
        Self { ctx }
    }

    fn draw(&self, list: &mut ColumnList, events: Vec<egui::Event>) -> ListDrawn {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let mut drawn = None;
        let _ = self.ctx.run_ui(raw, |ui| {
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                    egui::pos2(20.0, 20.0),
                    egui::vec2(320.0, 560.0),
                )),
                |ui| drawn = Some(list.show(ui, Mode::Light)),
            );
        });
        drawn.expect("the list drew")
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

/// **AC5.** A click on a row's value does what `l` does; a click on the `←` chip
/// does what `h` does; a click on a row's name only moves the cursor to it. The
/// row under the pointer draws the chips the cursor's row draws, and a row that
/// takes no step draws none.
#[test]
fn a_click_on_a_rows_value_steps_it_and_the_row_under_the_pointer_draws_chips() {
    let stage = Stage::new();
    let mut list = list_with("xScale: log");
    for _ in 0..3 {
        stage.draw(&mut list, vec![]);
    }
    let rest = stage.draw(&mut list, vec![]);
    let scale = rest.settings.iter().find(|r| r.name == SCALE_ROW).expect("scale row");
    let title = rest.settings.iter().find(|r| r.name == "title").expect("title row");
    assert!(
        title.chips.is_none(),
        "the title row takes typed text and draws no chips"
    );
    assert!(scale.chips.is_none(), "the scale row is not under the cursor or the pointer");

    // The pointer over the scale row: its chips are drawn.
    let value = scale.value_zone.center();
    let over = stage.draw(&mut list, vec![egui::Event::PointerMoved(value)]);
    let over = stage.draw(&mut list, vec![]).settings.len().min(over.settings.len());
    assert!(over > 0);
    let after = stage.draw(&mut list, vec![]);
    let scale = after.settings.iter().find(|r| r.name == SCALE_ROW).expect("scale row");
    let [back, forward] = scale.chips.expect("the row under the pointer draws its chips");
    assert!(
        back.right() <= forward.left() && forward.right() <= scale.marker.left(),
        "the chips stand at the trailing edge, before the marker"
    );

    let mut reports = Vec::new();
    for events in [
        vec![button(value, true)],
        vec![button(value, false)],
        vec![],
    ] {
        reports.extend(stage.draw(&mut list, events).reports);
    }
    assert_eq!(reports, [set(SCALE_ROW, word("symlog"))], "a click on the value");

    let back_at = back.center();
    let mut reports = Vec::new();
    for events in [
        vec![egui::Event::PointerMoved(back_at)],
        vec![button(back_at, true)],
        vec![button(back_at, false)],
        vec![],
    ] {
        reports.extend(stage.draw(&mut list, events).reports);
    }
    assert_eq!(reports, [set(SCALE_ROW, word("linear"))], "a click on the ← chip");

    let name_at = scale.name_rect.center();
    let mut reports = Vec::new();
    for events in [
        vec![egui::Event::PointerMoved(name_at)],
        vec![button(name_at, true)],
        vec![button(name_at, false)],
        vec![],
    ] {
        reports.extend(stage.draw(&mut list, events).reports);
    }
    assert_eq!(reports, [], "a click on the name writes nothing");
}

// ---------------------------------------------------------------------------
// The edit path alone.
// ---------------------------------------------------------------------------

const BASE: &str = "data:\n  t:\n    - { a: 1 }\nplot:\n  - mark: dot\n    data: { from: t }\n    x: a\n    y: a\nwidth: 600\nheight: 300\n";

fn row_in(text: &str, name: &str) -> SettingRow {
    let spec = parse_spec(text, Format::Yaml).expect("the text parses").spec;
    let plot = plot_at_path(&spec, "root").expect("the root plot");
    ChannelSettings::of_plot(&spec, plot, &channels())
        .rows(ShelfChannel::X)
        .iter()
        .find(|r| r.name == name)
        .expect("x has the row")
        .clone()
}

/// Make `write` to `key` on the root plot of `text` through the edit path, and
/// write the edit into the text as Save does. `None` where no edit was made.
fn through_save(
    text: &str,
    key: &str,
    default: &SpecValue,
    write: &SettingWrite,
) -> (Option<ChartEdit>, String) {
    let mut spec = parse_spec(text, Format::Yaml).expect("the text parses").spec;
    let edit = put_setting(&mut spec, &ComponentPath("root".to_string()), key, default, write)
        .expect("the root plot takes the edit");
    let written = edit.as_ref().map_or_else(
        || text.to_string(),
        |e| write_chart_edit(text, e).expect("Save places the edit"),
    );
    (edit, written)
}

fn attribute(text: &str, key: &str) -> Option<SpecValue> {
    let spec = parse_spec(text, Format::Yaml).expect("the text parses").spec;
    plot_at_path(&spec, "root")
        .expect("the root plot")
        .attributes
        .get(key)
        .cloned()
}

/// **AC2.** A number or a switch reaches the file as that type, and a string is
/// not what a number was typed as.
#[test]
fn a_number_and_a_switch_reach_the_file_as_their_type() {
    for (key, default, value) in [
        ("xTicks", SpecValue::Integer(5), SpecValue::Integer(3)),
        ("xGrid", SpecValue::Bool(true), SpecValue::Bool(false)),
        ("xZero", SpecValue::Bool(false), SpecValue::Bool(true)),
        ("xReverse", SpecValue::Bool(false), SpecValue::Bool(true)),
    ] {
        let (edit, written) = through_save(BASE, key, &default, &SettingWrite::Value(value.clone()));
        assert!(edit.is_some(), "{key}: no edit was made");
        assert_eq!(
            attribute(&written, key),
            Some(value.clone()),
            "{key}: the file reads it back as {value:?}\n{written}"
        );
    }
}

/// **AC2.** A key the plot takes from `plotDefaults` is written on the plot and
/// the plot's value wins, where a removal could not be written at all.
#[test]
fn a_key_the_plot_inherits_from_plot_defaults_is_written_on_the_plot_and_wins() {
    let text = format!("plotDefaults:\n  xGrid: false\n{BASE}");
    let default = SpecValue::Bool(true);
    let (edit, written) = through_save(&text, "xGrid", &default, &SettingWrite::Value(default.clone()));
    assert!(
        matches!(edit, Some(ChartEdit::SetPlotAttribute { .. })),
        "the default's own value is a write on the plot here, not a removal: {edit:?}"
    );
    assert_eq!(attribute(&written, "xGrid"), Some(SpecValue::Bool(true)));
    assert!(
        written.contains("plotDefaults:") && written.contains("xGrid: true"),
        "the plot's own line stands beside the default:\n{written}"
    );
    let removal = ChartEdit::RemovePlotAttribute {
        plot: ComponentPath("root".to_string()),
        key: "xGrid".to_string(),
    };
    assert!(
        matches!(
            write_chart_edit(&text, &removal),
            Err(ChartTextRefusal::Inherited { .. })
        ),
        "a removal of the inherited key is what Save refuses"
    );
    // ⌫ on the same row takes the same road.
    let (edit, _) = through_save(&text, "xGrid", &default, &SettingWrite::Auto);
    assert!(matches!(edit, Some(ChartEdit::SetPlotAttribute { .. })));
}

/// **AC3.** A value equal to brightfield's own is the key taken out, and `⌫`
/// takes out the axis's own key and nothing else: with `grid:` naming both
/// axes, the row then reads the value from `grid`, as set, and the foot says so.
#[test]
fn a_default_is_the_key_taken_out_and_backspace_leaves_the_both_axes_key_standing() {
    let text = format!("{BASE}xScale: log\n");
    let linear = SpecValue::String("linear".to_string());
    let (edit, written) = through_save(&text, "xScale", &linear, &SettingWrite::Value(linear.clone()));
    assert!(matches!(edit, Some(ChartEdit::RemovePlotAttribute { .. })), "{edit:?}");
    assert!(!written.contains("xScale"), "linear is written as no key:\n{written}");
    assert!(!row_in(&written, SCALE_ROW).set, "the row reads auto");

    let text = format!("{BASE}grid: false\nxGrid: true\n");
    assert_eq!(row_in(&text, GRID_ROW).value, "on");
    let on = SpecValue::Bool(true);
    let (edit, written) = through_save(&text, "xGrid", &on, &SettingWrite::Auto);
    assert!(matches!(edit, Some(ChartEdit::RemovePlotAttribute { .. })), "{edit:?}");
    assert!(!written.contains("xGrid"), "the plot's xGrid is out:\n{written}");
    assert!(written.contains("grid: false"), "the bare grid stands:\n{written}");
    let row = row_in(&written, GRID_ROW);
    assert_eq!(
        (row.value.as_str(), row.set, row.from),
        ("off", true, Some("grid")),
        "the row reads the inherited value as set"
    );
    assert!(
        foot_sentence(&row).contains("grid"),
        "the foot says it comes from grid: {:?}",
        foot_sentence(&row)
    );
    // Stepping grid on while `grid:` says off writes the axis's own `true`.
    let (edit, written) = through_save(&written, "xGrid", &on, &SettingWrite::Value(on.clone()));
    assert!(matches!(edit, Some(ChartEdit::SetPlotAttribute { .. })), "{edit:?}");
    assert_eq!(attribute(&written, "xGrid"), Some(SpecValue::Bool(true)));
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
            "bf-shelf-write-{name}-{}-{}",
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

/// One headless window over a copy of the housing sample, the hero's pane
/// focused.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
    folder: std::path::PathBuf,
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
            folder,
            texts: Vec::new(),
            _root: root,
        };
        win.settle();
        assert!(win.app.focus_pane(PaneKey::new(CHART)), "the pane takes focus");
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

    fn press(&mut self, key: egui::Key) {
        self.run(vec![key_event(key)]);
        self.run(Vec::new());
    }

    /// `e`, `x`, down to `median_income`, `Enter`: the column kept on x, which
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

    /// With the band holding the keys: x's list, turned to its settings.
    fn open_x_settings(&mut self) {
        self.type_letter(egui::Key::X, "x");
        self.press(egui::Key::Tab);
        assert_eq!(
            self.app.protocol_model().column_list().map(ColumnList::tab),
            Some(ListTab::Settings),
            "Tab did not turn x's list to its settings"
        );
    }

    /// The cursor to the settings row `name`: from the top for a head row, by
    /// the query for a row found by name.
    fn to_row(&mut self, name: &str) {
        if name == SCALE_ROW {
            self.type_letter(egui::Key::J, "j");
        } else {
            self.type_letter(egui::Key::Slash, "/");
            for ch in name.chars() {
                let key = egui::Key::from_name(&ch.to_uppercase().to_string()).expect("a letter");
                self.type_letter(key, &ch.to_string());
            }
            self.press(egui::Key::Enter);
        }
        assert_eq!(
            self.app
                .protocol_model()
                .column_list()
                .and_then(|l| l.setting_cursor().map(|r| r.name)),
            Some(match name {
                "scale" => SCALE_ROW,
                "grid" => GRID_ROW,
                "reverse" => REVERSE_ROW,
                other => panic!("no row {other}"),
            })
        );
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
        self.app.chart_doc().composed.plots[0].scales.get(Channel::X).cloned()
    }

    fn marked_unsaved(&self) -> bool {
        self.app.title().contains(UNSAVED_MARK)
    }

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

    fn save(&mut self) {
        let ctx = self.ctx.clone();
        self.app
            .save_protocol(&ctx)
            .expect("a data file's window has a Protocol to save")
            .expect("the Protocol saves");
        self.settle();
    }

    fn written(&self) -> String {
        let chart = std::fs::read_dir(self.folder.join("panels"))
            .expect("Save wrote a panels folder")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "yaml"))
            .expect("Save wrote a chart file");
        std::fs::read_to_string(&chart).expect("the chart file reads")
    }

    /// Keep x's scale at log, its grid off and its reverse on, by the keys.
    fn log_then_grid_off_then_reverse_on(&mut self) {
        self.open_x_settings();
        self.to_row("scale");
        self.type_letter(egui::Key::L, "l");
        assert_eq!(self.attribute("xScale"), Some(SpecValue::String("log".into())));
        assert!(
            matches!(self.drawn_x_scale(), Some(Scale::Log { .. })),
            "the chart did not draw the log scale at once"
        );
        self.press(egui::Key::Escape);
        self.to_row("grid");
        self.type_letter(egui::Key::H, "h");
        assert_eq!(self.attribute("xGrid"), Some(SpecValue::Bool(false)));
        self.press(egui::Key::Escape);
        self.to_row("reverse");
        self.type_letter(egui::Key::L, "l");
        assert_eq!(self.attribute("xReverse"), Some(SpecValue::Bool(true)));
    }
}

/// **AC4, AC6.** Keep scale log, then grid off, then reverse on: the status
/// band's leading end names the last edit and the key that undoes it; `u` three
/// times restores reverse, grid and scale in turn, each redrawn; after the third
/// the window is not marked unsaved and Save writes what it wrote before.
#[test]
fn u_steps_back_over_a_scale_a_grid_and_a_reverse_each_redrawn() {
    let mut win = Window::housing("undo");
    win.keep_income_on_x();
    win.save();
    let before = win.written();
    assert!(!win.marked_unsaved());

    win.open_x_settings();
    win.to_row("scale");
    win.type_letter(egui::Key::L, "l");
    let key = brightfield_workbench::Verb::new("undo")
        .keys()
        .expect("the registry binds a key to undo");
    let line = format!("x axis scale: linear \u{2192} log \u{b7} {key} undo");
    let drawn = win.status_text();
    let at = drawn
        .iter()
        .position(|(_, t)| *t == line)
        .unwrap_or_else(|| panic!("the band drew {drawn:?}, not {line:?}"));
    let counted = drawn
        .iter()
        .position(|(_, t)| t.contains(" rows"))
        .unwrap_or_else(|| panic!("the band drew no row count in {drawn:?}"));
    assert!(at < counted, "the edit is not at the leading end: {drawn:?}");
    assert!(win.app.rail().drawn.contains(&SHELF_EDIT_STATUS_ID));
    win.press(egui::Key::Escape);
    win.to_row("grid");
    win.type_letter(egui::Key::H, "h");
    win.press(egui::Key::Escape);
    win.to_row("reverse");
    win.type_letter(egui::Key::L, "l");
    assert!(win.marked_unsaved());

    win.type_letter(egui::Key::U, "u");
    assert_eq!(win.attribute("xReverse"), None, "the first u restores reverse");
    assert_eq!(win.attribute("xGrid"), Some(SpecValue::Bool(false)));
    win.type_letter(egui::Key::U, "u");
    assert_eq!(win.attribute("xGrid"), None, "the second u restores grid");
    assert!(matches!(win.drawn_x_scale(), Some(Scale::Log { .. })));
    win.type_letter(egui::Key::U, "u");
    assert_eq!(win.attribute("xScale"), None, "the third u restores scale");
    assert!(
        matches!(win.drawn_x_scale(), Some(Scale::Linear { .. })),
        "the chart is drawn linear again"
    );
    assert!(!win.marked_unsaved(), "the window is still marked unsaved");
    win.save();
    assert_eq!(win.written(), before, "Save wrote a trace of the stepped values");
}

/// **AC7.** After Save the file and the chart both show each value set, and the
/// row stepped back to brightfield's own reads auto with no key in the file.
#[test]
fn save_writes_each_value_set_and_none_for_a_row_stepped_back_to_auto() {
    let mut win = Window::housing("saved");
    win.keep_income_on_x();
    win.log_then_grid_off_then_reverse_on();
    // Reverse back to brightfield's own: the row reads auto and the key goes.
    win.type_letter(egui::Key::H, "h");
    assert_eq!(win.attribute("xReverse"), None, "stepped back, the key is out");
    // A count typed into a row reaches the file as a number.
    let ticks = RowEdit {
        channel: ShelfChannel::X,
        row: "ticks",
        value: SettingValue::Count(3),
    };
    assert!(win.app.chart_doc_mut().set_axis_row(0, &ticks));
    assert_eq!(win.attribute("xTicks"), Some(SpecValue::Integer(3)));
    win.save();
    let written = win.written();
    assert_eq!(attribute(&written, "xScale"), Some(SpecValue::String("log".into())), "{written}");
    assert_eq!(attribute(&written, "xGrid"), Some(SpecValue::Bool(false)));
    assert_eq!(attribute(&written, "xTicks"), Some(SpecValue::Integer(3)));
    assert!(!written.contains("xReverse"), "no key for auto:\n{written}");
    let reopened = |name: &str| {
        let spec = parse_spec(&written, Format::Yaml).expect("the file parses").spec;
        let plot = plot_at_path(&spec, "root")
            .or_else(|| plot_at_path(&spec, &win.app.chart_doc().composed.plots[0].path))
            .expect("the plot");
        let channels = brightfield_shell::shelf::ShelfChannels::of_plot(plot).expect("channels");
        ChannelSettings::of_plot(&spec, plot, &channels)
            .rows(ShelfChannel::X)
            .iter()
            .find(|r| r.name == name)
            .expect("the row")
            .clone()
    };
    assert_eq!((reopened(SCALE_ROW).value, reopened(SCALE_ROW).set), ("log".to_string(), true));
    assert_eq!((reopened(GRID_ROW).value, reopened(GRID_ROW).set), ("off".to_string(), true));
    assert!(!reopened(REVERSE_ROW).set, "the row stepped back reads auto");
}
