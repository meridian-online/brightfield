//! **The format row says when a format was chosen from the column's type.**
//!
//! A file's specifier equal to what the rule gives now reads `auto`, with that specifier beside
//! it; a file with none reads `default` and offers the rule's; anything else is shown as typed.
//! `auto` is a preset that writes the rule's specifier, `default` one that takes the key out, and
//! the stepper skips `auto` where the rule gives nothing and skips a preset that writes what
//! `auto` would.
//!
//! The labels are nominated through a column record, as an analyst who accepted them would have,
//! so no FineType bundle is needed. The stepper is driven on the list alone; the rows a window
//! hands it, and the spec file a window leaves as it was while its format is edited, are read
//! through a window over the data file. A hand-written spec cannot be opened that way: the
//! shelf's settings list belongs to a data-file window's protocol model, so a saved chart whose
//! label later changed is held on the rows alone.

use std::path::PathBuf;

use brightfield_shell::app::CHART;
use brightfield_shell::column_record;
use brightfield_shell::data_file::{self, OpenOptions};
use brightfield_shell::design::Mode;
use brightfield_shell::shelf::{
    Binding, ChannelSettings, ColumnList, ColumnListRequest, ListColumn, ListReport, RowEdit,
    SettingRow, SettingValue, ShelfChannels, AUTO, DEFAULT_FORMAT, FORMAT_ROW, TITLE_ROW,
};
use brightfield_shell::startup::default_layout;
use brightfield_shell::text_ink;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_workbench::channel::ShelfChannel;
use brightfield_workbench::PaneKey;

const CSV: &str = "\
revenue,share,downloads
\"£2,412,345,000.00\",45%,1234567
\"£500,000,000.00\",10%,10
\"£3,100,000,000.00\",80%,900
";

/// The tick format the rule chose for `revenue`, which the generated tile writes.
fn revenue_rule() -> String {
    opened().1
}

fn scratch_config() {
    use std::sync::OnceLock;
    static CONFIG: OnceLock<PathBuf> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("bf-auto-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch config folder");
        std::env::set_var(brightfield_shell::startup::CONFIG_DIR_VAR, &dir);
        dir
    });
}

/// The fixture in a folder of the calling test's own, with a record nominating labels for
/// `revenue` and `share` and none for `downloads`.
fn fixture() -> PathBuf {
    scratch_config();
    let thread = std::thread::current();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("shelf_settings_format_auto")
        .join(thread.name().unwrap_or("main").replace("::", "-"));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let path = dir.join("typed_auto.csv");
    std::fs::write(&path, CSV).expect("write fixture");
    let mut record = column_record::SourceRecord::default();
    for (column, label) in [
        ("revenue", "finance.currency.amount"),
        ("share", "representation.numeric.percentage"),
    ] {
        record.fields.insert(
            column.to_string(),
            column_record::FieldRecord {
                label: Some(label.to_string()),
                nominated: true,
                ..column_record::FieldRecord::default()
            },
        );
    }
    let package = column_record::flat_record_path(&path).expect("a record path");
    column_record::write_record(&package, data_file::SOURCE, &record).expect("write record");
    path
}

fn unbundled() -> OpenOptions {
    OpenOptions {
        type_source: None,
        ..OpenOptions::default()
    }
}

/// The fixture's path, and the rule's tick format for `revenue`.
fn opened() -> (PathBuf, String) {
    let path = fixture();
    let (opened, _) = data_file::open_traced(&path.to_string_lossy(), &unbundled())
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let rule = opened
        .protocol
        .columns
        .iter()
        .find(|c| c.column == "revenue")
        .and_then(|c| c.representation.tick_format.clone())
        .expect("the rule chose a tick format for revenue");
    (path, rule)
}

// ---------------------------------------------------------------------------
// The list alone: rows read from a spec, the rule handed in as a window hands it.
// ---------------------------------------------------------------------------

const SOURCE: &str = "data:\n  t:\n    - { a: 1 }\nplot:\n  - mark: dot\n    data: { from: t }\n    x: a\n    y: a\nwidth: 600\nheight: 300\n";

fn channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("a".to_string()),
        y: Binding::Column("a".to_string()),
        colour: Binding::Unset,
    }
}

fn settings(attrs: &str, rule: Option<&str>) -> ChannelSettings {
    let spec = parse_spec(&format!("{SOURCE}{attrs}\n"), Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the root is a plot");
    ChannelSettings::of_plot(&spec, plot, "root", &channels()).with_rule_formats(rule, None)
}

fn format_row(attrs: &str, rule: Option<&str>) -> SettingRow {
    settings(attrs, rule)
        .rows(ShelfChannel::X)
        .iter()
        .find(|row| row.name == FORMAT_ROW)
        .expect("x has a format row")
        .clone()
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

fn typed(key: egui::Key, letter: &str) -> Vec<egui::Event> {
    vec![key_event(key), egui::Event::Text(letter.to_string())]
}

/// A list on x's settings over a plot writing `attrs`, the cursor on the format row.
fn list_on_format(attrs: &str, rule: Option<&str>) -> ColumnList {
    let mut list = ColumnList::new(ColumnListRequest {
        tile: "hero".to_string(),
        channel: ShelfChannel::X,
        channels: channels(),
        columns: vec![ListColumn {
            name: "a".to_string(),
            kind: "BIGINT".to_string(),
            moments: None,
        }],
    });
    list.set_settings(settings(attrs, rule));
    list.feed_events(&[key_event(egui::Key::Tab)]);
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(TITLE_ROW));
    while list.setting_cursor().map(|r| r.name) != Some(FORMAT_ROW) {
        list.feed_events(&typed(egui::Key::J, "j"));
    }
    list
}

/// The attribute line a report's value writes, so the list can be stepped on.
fn written(report: &ListReport) -> Option<String> {
    match report {
        ListReport::Set(RowEdit {
            value: SettingValue::Text(text),
            ..
        }) => Some(format!("xTickFormat: '{text}'")),
        ListReport::Set(RowEdit {
            value: SettingValue::Auto,
            ..
        }) => Some(String::new()),
        _ => None,
    }
}

/// The names the format row reads, `l` step by `l` step from `attrs`, until a step writes
/// nothing more or opens the field. The attribute line each step wrote feeds the next.
fn names_stepping(attrs: &str, rule: Option<&str>, key: egui::Key, letter: &str) -> Vec<String> {
    let mut names = vec![format_row(attrs, rule).value];
    let mut attrs = attrs.to_string();
    for _ in 0..12 {
        let mut list = list_on_format(&attrs, rule);
        let reports = list.feed_events(&typed(key, letter));
        let Some(next) = reports.first().and_then(written) else {
            break;
        };
        attrs = next;
        names.push(format_row(&attrs, rule).value);
    }
    names
}

#[test]
fn auto_shows_the_rule_format() {
    let rule = revenue_rule();
    let row = format_row(&format!("xTickFormat: '{rule}'"), Some(&rule));
    assert_eq!(row.value, AUTO);
    assert!(!row.set, "the rule's own format is brightfield's choice");
    let facts = row.format.expect("a format row carries its facts");
    assert_eq!(facts.rule.as_deref(), Some(rule.as_str()));
    assert_eq!(facts.specifier.as_deref(), Some(rule.as_str()));
}

#[test]
fn typed_format_is_not_auto() {
    let row = format_row("xTickFormat: ',d'", Some("(,~f"));
    assert_ne!(row.value, AUTO);
    assert!(row.set);
    assert_eq!(
        row.format.and_then(|f| f.specifier).as_deref(),
        Some(",d"),
        "the specifier typed is the one the row holds"
    );
}

#[test]
fn empty_specifier_suggests_the_rule() {
    let row = format_row("", Some("(,~f"));
    assert_eq!(row.value, DEFAULT_FORMAT);
    assert!(!row.set);
    assert_eq!(
        row.format.and_then(|f| f.rule).as_deref(),
        Some("(,~f"),
        "the rule's specifier is offered beside default"
    );
}

#[test]
fn auto_preset_writes_the_rule() {
    let mut list = list_on_format("", Some("(,~f"));
    let reports = list.feed_events(&typed(egui::Key::L, "l"));
    assert_eq!(
        reports,
        [ListReport::Set(RowEdit {
            channel: ShelfChannel::X,
            row: FORMAT_ROW,
            value: SettingValue::Text("(,~f".to_string()),
        })],
        "`l` from default picks auto, which writes the rule's specifier"
    );
    assert_eq!(format_row("xTickFormat: '(,~f'", Some("(,~f")).value, AUTO);
    let mut list = list_on_format("xTickFormat: '(,~f'", Some("(,~f"));
    let reports = list.feed_events(&typed(egui::Key::H, "h"));
    assert_eq!(
        reports,
        [ListReport::Set(RowEdit {
            channel: ShelfChannel::X,
            row: FORMAT_ROW,
            value: SettingValue::Auto,
        })],
        "`h` from auto picks default, which takes the key out"
    );
}

#[test]
fn unlabelled_column_steps_past_auto() {
    let names = names_stepping("", None, egui::Key::L, "l");
    assert_eq!(names, ["default", "number", "short", "percent", "currency"]);
    assert!(!names.iter().any(|n| n == AUTO), "{names:?}");
}

#[test]
fn rule_matching_a_preset_does_not_trap_the_stepper() {
    let forward = names_stepping("", Some("~s"), egui::Key::L, "l");
    assert_eq!(
        forward,
        ["default", "auto", "number", "percent", "currency"],
        "stepping `l` from default reaches currency, and skips the preset auto already is"
    );
    let mut list = list_on_format("xTickFormat: '$,f'", Some("~s"));
    let reports = list.feed_events(&typed(egui::Key::L, "l"));
    assert_eq!(reports, [], "`l` from currency opens the field on custom");
    assert!(list.field().is_some(), "currency steps on to custom");
    let back = names_stepping("xTickFormat: '$,f'", Some("~s"), egui::Key::H, "h");
    assert_eq!(
        back,
        ["currency", "percent", "number", "auto", "default"],
        "stepping `h` returns to default"
    );
}

#[test]
fn reset_puts_a_type_chosen_format_back_to_auto() {
    let mut list = list_on_format("xTickFormat: ',d'", Some("(,~f"));
    let reports = list.feed_events(&[key_event(egui::Key::Backspace)]);
    assert_eq!(
        reports,
        [ListReport::Set(RowEdit {
            channel: ShelfChannel::X,
            row: FORMAT_ROW,
            value: SettingValue::Text("(,~f".to_string()),
        })],
        "reset writes the rule's specifier, so the row reads auto"
    );
    let mut list = list_on_format("xTickFormat: ',d'", None);
    let reports = list.feed_events(&[key_event(egui::Key::Backspace)]);
    assert_eq!(
        reports,
        [ListReport::Set(RowEdit {
            channel: ShelfChannel::X,
            row: FORMAT_ROW,
            value: SettingValue::Auto,
        })],
        "with no rule to write, reset takes the key out"
    );
}

/// Reset means back to Brightfield's choice, so on a labelled column's unset row it writes the
/// rule's format, which the row then reads as `auto`.
#[test]
fn reset_on_an_unset_row_of_a_labelled_column_writes_the_rule_format() {
    let mut list = list_on_format("", Some("(,~f"));
    assert_eq!(format_row("", Some("(,~f")).value, DEFAULT_FORMAT);
    let reports = list.feed_events(&[key_event(egui::Key::Backspace)]);
    assert_eq!(
        reports,
        [ListReport::Set(RowEdit {
            channel: ShelfChannel::X,
            row: FORMAT_ROW,
            value: SettingValue::Text("(,~f".to_string()),
        })],
        "reset on default writes the rule's specifier"
    );
    assert_eq!(format_row("xTickFormat: '(,~f'", Some("(,~f")).value, AUTO);
}

#[test]
fn the_foot_lists_the_presets_the_row_steps_through() {
    use brightfield_shell::shelf::foot_sentence;
    let takes = |attrs: &str, rule: Option<&str>| {
        let foot = foot_sentence(&format_row(attrs, rule));
        let at = foot.find("Takes: ").expect("the foot lists the presets") + 7;
        foot[at..].split('.').next().unwrap_or_default().to_string()
    };
    assert_eq!(
        takes("", None),
        "default, number, short, percent, currency, custom",
        "no rule, no auto"
    );
    assert_eq!(
        takes("", Some("(,~f")),
        "default, auto, number, short, percent, currency, custom"
    );
    assert_eq!(
        takes("", Some("~s")),
        "default, auto, number, percent, currency, custom",
        "the preset auto already writes is left out"
    );
}

// ---------------------------------------------------------------------------
// The window: the rows it hands the list, and the file a saved chart leaves alone.
// ---------------------------------------------------------------------------

const SCREEN: egui::Rect = egui::Rect {
    min: egui::Pos2::ZERO,
    max: egui::pos2(1440.0, 900.0),
};

fn frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        screen_rect: Some(SCREEN),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| {
        app.draw(ui);
        let _ = text_ink::frame_text(ui.ctx());
    });
}

fn settle(app: &mut MeridianApp, ctx: &egui::Context) {
    for _ in 0..3 {
        frame(app, ctx, Vec::new());
    }
}

/// Open x's settings in `app` and return the format row it reads.
fn x_format_row(app: &mut MeridianApp, ctx: &egui::Context) -> SettingRow {
    assert!(app.focus_pane(PaneKey::new(CHART)));
    settle(app, ctx);
    for (key, letter) in [
        (egui::Key::E, "e"),
        (egui::Key::X, "x"),
        (egui::Key::Tab, ""),
    ] {
        let mut events = vec![key_event(key)];
        if !letter.is_empty() {
            events.push(egui::Event::Text(letter.to_string()));
        }
        frame(app, ctx, events);
        frame(app, ctx, Vec::new());
    }
    app.protocol_model()
        .column_list()
        .expect("x's list is open")
        .settings()
        .iter()
        .find(|row| row.name == FORMAT_ROW)
        .expect("x has a format row")
        .clone()
}

fn press_letter(app: &mut MeridianApp, ctx: &egui::Context, key: egui::Key, letter: &str) {
    frame(
        app,
        ctx,
        vec![key_event(key), egui::Event::Text(letter.to_string())],
    );
    frame(app, ctx, Vec::new());
}

fn format_row_of(app: &MeridianApp) -> SettingRow {
    app.protocol_model()
        .column_list()
        .expect("x's list is open")
        .settings()
        .iter()
        .find(|row| row.name == FORMAT_ROW)
        .expect("x has a format row")
        .clone()
}

#[test]
fn the_generated_revenue_tile_reads_auto() {
    let (path, rule) = opened();
    let boot = Boot::data_file_with(&path.to_string_lossy(), &unbundled()).expect("the file opens");
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    let ctx = egui::Context::default();
    let row = x_format_row(&mut app, &ctx);
    assert_eq!(
        row.value, AUTO,
        "the generated tile writes the rule's format"
    );
    assert_eq!(
        row.format.and_then(|f| f.rule).as_deref(),
        Some(rule.as_str())
    );
}

#[test]
fn saved_chart_keeps_its_format() {
    let (path, rule) = opened();
    let saved = "$,.2f";
    assert_ne!(
        saved, rule,
        "fixture check: the saved format is not today's rule"
    );
    let (opened, _) = data_file::open_traced(&path.to_string_lossy(), &unbundled())
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let generated = opened.dashboard.to_spec();
    let data = &generated[..generated.find("\nhconcat:").expect("a generated layout")];
    let spec = format!(
        "{data}\nplot:\n  - mark: dot\n    data: {{ from: \"opened\" }}\n    x: \"revenue\"\n    y: \"share\"\nxTickFormat: \"{saved}\"\nwidth: 600\nheight: 400\n"
    );
    let parsed = parse_spec(&spec, Format::Yaml)
        .expect("the saved chart parses")
        .spec;
    let plot = plot_at_path(&parsed, "root").expect("the chart is a plot");
    let channels = ShelfChannels::of_plot(plot).expect("the chart has channels");
    let row = ChannelSettings::of_plot(&parsed, plot, "root", &channels)
        .with_rule_formats(Some(&rule), None)
        .rows(ShelfChannel::X)
        .iter()
        .find(|row| row.name == FORMAT_ROW)
        .expect("x has a format row")
        .clone();
    assert_ne!(
        row.value, AUTO,
        "a label that changed leaves the saved format as it was"
    );
    assert_eq!(
        row.format.and_then(|f| f.specifier).as_deref(),
        Some(saved),
        "the row shows the specifier the chart was saved with"
    );
}

/// Editing the format in a window changes the picture and not the spec file the window was
/// opened on: the row reads the edit, and the file's bytes are the ones it was opened with.
#[test]
fn editing_the_format_leaves_the_spec_file_as_it_was() {
    let (path, _) = opened();
    let boot = Boot::data_file_with(&path.to_string_lossy(), &unbundled()).expect("the file opens");
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    let ctx = egui::Context::default();
    let spec_file = app
        .chart_doc()
        .spec_path
        .clone()
        .expect("a data file's generated spec is on disk");
    let before = std::fs::read(&spec_file).expect("read the spec");
    assert_eq!(x_format_row(&mut app, &ctx).value, AUTO);
    for _ in 0..3 {
        press_letter(&mut app, &ctx, egui::Key::J, "j");
    }
    press_letter(&mut app, &ctx, egui::Key::L, "l");
    settle(&mut app, &ctx);
    let row = format_row_of(&app);
    assert_eq!(row.value, "number", "`l` from auto steps to number");
    assert_eq!(
        row.format.and_then(|f| f.specifier).as_deref(),
        Some(",f"),
        "the row holds what the edit wrote, which is not the rule's"
    );
    assert_eq!(
        std::fs::read(&spec_file).expect("read the spec"),
        before,
        "an edit is not a save: the file is as it was opened"
    );
}

/// The command log names the row's value as the inspector reads it: stepping off `auto` says
/// `auto`, not `default`.
#[test]
fn the_command_log_names_auto_as_the_inspector_reads_it() {
    let (path, _) = opened();
    let boot = Boot::data_file_with(&path.to_string_lossy(), &unbundled()).expect("the file opens");
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    let ctx = egui::Context::default();
    assert_eq!(x_format_row(&mut app, &ctx).value, AUTO);
    for _ in 0..3 {
        press_letter(&mut app, &ctx, egui::Key::J, "j");
    }
    press_letter(&mut app, &ctx, egui::Key::L, "l");
    let said = app
        .chart_doc()
        .last_shelf_edit()
        .expect("the edit is logged")
        .to_string();
    assert!(
        said.contains("auto") && said.contains("number"),
        "the log names the step from auto to number: {said}"
    );
}
