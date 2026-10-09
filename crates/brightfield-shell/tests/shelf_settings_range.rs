//! **The range row takes two ends, and a scale change or a new column puts a set
//! range back to auto.**
//!
//! The row stands third at the head of an axis's list. It reads *auto* with the
//! ends the axis was drawn over in muted ink while the file sets none, and the
//! file's two ends in full ink with a filled dot when it does. `Enter` opens a low
//! and a high field, `Enter` on the low moves to the high, and `Enter` on the high
//! keeps the pair.
//!
//! This file holds the list alone, the write path and the pixels of the row at
//! rest and mid-edit in both themes. The window's keys, the status band and the
//! chart's draw are in `shelf_settings_range_window.rs`, and a loaded file's log
//! axis in `shelf_settings_range_log.rs`.

use brightfield_render::channel::Channel;
use brightfield_render::mark::Projection;
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_shell::design::{self, Mode};
use brightfield_shell::shelf::{
    Binding, ChannelSettings, ColumnList, ColumnListRequest, End, ListColumn, ListReport, ListTab,
    SettingValue, ShelfChannels, LOG_CANNOT, RANGE_ROW,
};
use brightfield_shell::shelf_edit::put_range_to_auto;
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::SpecValue;
use brightfield_spec::edit::{plot_at_path, ChartEdit};
use brightfield_spec::layout::PlotAxis;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_workbench::channel::ShelfChannel;
use egui_kittest::{Harness, SnapshotOptions};

fn key_event(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn typed(text: &str) -> egui::Event {
    egui::Event::Text(text.to_string())
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

fn spec_of(attrs: &str) -> brightfield_spec::ast::Spec {
    parse_spec(&format!("{SOURCE}{attrs}\n"), Format::Yaml)
        .expect("the spec parses")
        .spec
}

fn linear(lo: f64, hi: f64) -> ScaleSet {
    let mut drawn = ScaleSet::new();
    drawn.insert(
        Channel::X,
        Scale::Linear {
            domain_min: lo,
            domain_max: hi,
            range_start: 0.0,
            range_end: 100.0,
        },
    );
    drawn
}

fn log(lo: f64, hi: f64) -> ScaleSet {
    let mut drawn = ScaleSet::new();
    drawn.insert(
        Channel::X,
        Scale::Log {
            domain_min: lo,
            domain_max: hi,
            range_start: 0.0,
            range_end: 100.0,
        },
    );
    drawn
}

fn settings_over(attrs: &str, drawn: &ScaleSet) -> ChannelSettings {
    let spec = spec_of(attrs);
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    ChannelSettings::of_plot_drawn(&spec, plot, &channels(), drawn)
}

/// A list on x's settings with the cursor on the range row, reached by its name.
fn list_on_range(attrs: &str, drawn: &ScaleSet) -> ColumnList {
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
    assert_eq!(list.tab(), ListTab::Settings);
    while list.setting_cursor().map(|r| r.name) != Some(RANGE_ROW) {
        let was = list.setting_cursor().map(|r| r.name);
        list.feed_events(&[key_event(egui::Key::J)]);
        assert_ne!(
            list.setting_cursor().map(|r| r.name),
            was,
            "`j` stopped before the range row"
        );
    }
    list
}

/// **AC1.** The row is third, ahead of the format, and reads the drawn ends as
/// auto, or the file's two ends as a set value.
#[test]
fn the_range_row_stands_third_and_reads_auto_or_the_files_two_ends() {
    let drawn = linear(5.0, 95.0);
    let rows = settings_over("", &drawn);
    let names: Vec<&str> = rows
        .rows(ShelfChannel::X)
        .iter()
        .filter(|r| !r.by_name)
        .map(|r| r.name)
        .collect();
    assert_eq!(names, ["title", "scale", "range", "format"]);

    let auto = &rows.rows(ShelfChannel::X)[2];
    assert_eq!((auto.value.as_str(), auto.set), ("5 \u{2013} 95", false));

    let rows = settings_over("xDomain: [10, 90]", &drawn);
    let set = &rows.rows(ShelfChannel::X)[2];
    assert_eq!((set.value.as_str(), set.set), ("10 \u{2013} 90", true));
}

/// **AC2.** `Enter` opens both fields on the drawn ends with the low selected,
/// `Enter` on the low moves to the high, and `Enter` on the high keeps the pair.
#[test]
fn enter_opens_low_and_high_and_keeps_the_pair() {
    let mut list = list_on_range("", &linear(5.0, 95.0));
    list.feed_events(&[key_event(egui::Key::Enter)]);
    let field = list.field().expect("Enter opened the fields");
    let fields = field.ends.as_ref().expect("the range row has two fields");
    assert_eq!(
        (field.text.as_str(), field.selected, fields.other.as_str()),
        ("5", true, "95")
    );
    assert_eq!(fields.on, End::Low);

    list.feed_events(&[typed("10"), key_event(egui::Key::Enter)]);
    let field = list.field().expect("the field stays open on the high");
    assert_eq!(field.ends.as_ref().map(|f| f.on), Some(End::High));
    assert_eq!((field.text.as_str(), field.selected), ("95", true));

    let reports = list.feed_events(&[typed("90"), key_event(egui::Key::Enter)]);
    let kept: Vec<_> = reports
        .iter()
        .filter_map(|r| match r {
            ListReport::Set(edit) => Some(edit),
            _ => None,
        })
        .collect();
    assert_eq!(kept.len(), 1, "Enter on the high kept one edit");
    let SettingValue::Ends(ends) = &kept[0].value else {
        panic!("the edit is the two ends");
    };
    assert_eq!((ends.lo, ends.hi), (10.0, 90.0));
    assert!(list.field().is_none(), "keeping closed the fields");
}

/// **AC2.** A high at or below the low is refused under the row and the edit
/// stays open.
#[test]
fn a_high_at_or_below_the_low_is_refused_and_the_edit_stays_open() {
    let mut list = list_on_range("", &linear(5.0, 95.0));
    list.feed_events(&[key_event(egui::Key::Enter), typed("50")]);
    list.feed_events(&[key_event(egui::Key::Enter)]);
    let reports = list.feed_events(&[typed("50"), key_event(egui::Key::Enter)]);
    assert!(
        !reports.iter().any(|r| matches!(r, ListReport::Set(_))),
        "a high equal to the low is kept as nothing"
    );
    let field = list.field().expect("the field stays open");
    assert!(
        field
            .refusal
            .as_deref()
            .is_some_and(|s| s.contains("high end")),
        "{:?}",
        field.refusal
    );
}

/// **AC6.** A log axis refuses a low end at zero as it is typed: the row says why
/// under itself, `Enter` reports it for the status band, and no edit is kept.
#[test]
fn a_log_axis_refuses_a_range_through_zero() {
    let mut list = list_on_range("xScale: log", &log(1.0, 1000.0));
    list.feed_events(&[key_event(egui::Key::Enter)]);
    let reports = list.feed_events(&[typed("0")]);
    assert!(
        !reports
            .iter()
            .any(|r| matches!(r, ListReport::Preview(Some(_)))),
        "a range through zero is not previewed"
    );
    let reports = list.feed_events(&[key_event(egui::Key::Enter)]);
    assert!(
        reports
            .iter()
            .any(|r| matches!(r, ListReport::Refused(s) if s.starts_with(LOG_CANNOT))),
        "{reports:?}"
    );
    let field = list.field().expect("the field stays open");
    assert!(field
        .refusal
        .as_deref()
        .is_some_and(|s| s.starts_with(LOG_CANNOT)));
    assert_eq!(field.ends.as_ref().map(|f| f.on), Some(End::Low));
}

/// **AC4.** A range the file set comes out of the plot, and a pair held under
/// `xyDomain` is handed to the other axis under its own key.
#[test]
fn putting_a_range_back_to_auto_takes_the_axis_keys_out() {
    let path = ComponentPath("root".to_string());
    let mut spec = spec_of("xDomain: [0, 100]");
    let edits = put_range_to_auto(&mut spec, &path, PlotAxis::X).expect("the plot exists");
    assert_eq!(edits.len(), 1);
    let plot = plot_at_path(&spec, "root").expect("plot");
    assert!(!plot.attributes.contains_key("xDomain"));

    let mut spec = spec_of("xyDomain: [0, 100]");
    let edits = put_range_to_auto(&mut spec, &path, PlotAxis::X).expect("the plot exists");
    assert!(edits
        .iter()
        .any(|e| matches!(e, ChartEdit::SetPlotAttribute { key, .. } if key == "yDomain")));
    let plot = plot_at_path(&spec, "root").expect("plot");
    assert!(!plot.attributes.contains_key("xyDomain"));
    assert!(matches!(
        plot.attributes.get("yDomain"),
        Some(SpecValue::Array(ends)) if ends.len() == 2
    ));

    let mut spec = spec_of("");
    let edits = put_range_to_auto(&mut spec, &path, PlotAxis::X).expect("the plot exists");
    assert!(edits.is_empty(), "no range set, nothing to take out");
}

/// A band scale: an axis of names.
fn names() -> ScaleSet {
    let mut drawn = ScaleSet::new();
    drawn.insert(
        Channel::X,
        Scale::Band {
            categories: vec!["north".to_string(), "south".to_string()],
            range_start: 0.0,
            range_end: 100.0,
            padding: 0.1,
        },
    );
    drawn
}

/// A time scale: an axis of dates.
fn dates() -> ScaleSet {
    let mut drawn = ScaleSet::new();
    drawn.insert(
        Channel::X,
        Scale::Time {
            domain_min_us: 0,
            domain_max_us: 86_400_000_000,
            range_start: 0.0,
            range_end: 100.0,
        },
    );
    drawn
}

/// A map: the plot draws through a projection, which no axis instruction reaches.
fn map() -> ScaleSet {
    let mut drawn = linear(5.0, 95.0);
    drawn.set_projection(Projection::Mercator, None);
    drawn
}

/// **AC3.** On an axis of dates, an axis of names and a map the row carries its
/// reason, and `Enter` on it opens no field and keeps no edit, even where the
/// file holds a pair.
#[test]
fn on_dates_names_and_a_map_the_row_carries_its_reason_and_enter_changes_nothing() {
    let cases = [
        ("dates", dates(), "a range on a date axis is not read yet"),
        ("names", names(), "an axis of names has no ends to set"),
        (
            "a map",
            map(),
            "a map's x and y are its projection, which has no axis to set",
        ),
    ];
    for (what, drawn, reason) in cases {
        for attrs in ["", "xDomain: [10, 90]"] {
            let rows = settings_over(attrs, &drawn);
            let row = rows
                .rows(ShelfChannel::X)
                .iter()
                .find(|r| r.name == RANGE_ROW)
                .expect("x has a range row");
            assert_eq!(
                row.reason.as_deref(),
                Some(reason),
                "{what} with `{attrs}` carries its reason"
            );

            let mut list = list_on_range(attrs, &drawn);
            let reports = list.feed_events(&[key_event(egui::Key::Enter)]);
            assert!(
                list.field().is_none(),
                "Enter opened a field on {what} with `{attrs}`"
            );
            assert!(
                !reports.iter().any(|r| matches!(r, ListReport::Set(_))),
                "Enter kept something on {what} with `{attrs}`: {reports:?}"
            );
        }
    }
    // The same plot over a linear axis gives no reason, so the three above are the
    // axis's doing and not the row's.
    let rows = settings_over("", &linear(5.0, 95.0));
    let row = rows
        .rows(ShelfChannel::X)
        .iter()
        .find(|r| r.name == RANGE_ROW)
        .expect("x has a range row");
    assert!(row.reason.is_none(), "a linear axis takes a range");
}

/// **AC2.** `\u{232b}` on the range row reports it put back to auto, which the
/// window turns into the key coming out of the file.
#[test]
fn backspace_on_the_range_row_reports_it_back_to_auto() {
    let mut list = list_on_range("xDomain: [10, 90]", &linear(5.0, 95.0));
    let reports = list.feed_events(&[key_event(egui::Key::Backspace)]);
    assert!(
        reports.iter().any(|r| matches!(
            r,
            ListReport::Set(edit) if edit.row == RANGE_ROW && edit.value == SettingValue::Auto
        )),
        "{reports:?}"
    );
}

/// **AC2.** Typing a low end previews the pair on the axis, and a low typed above
/// the high is no preview and no refusal, since the high is asked next.
#[test]
fn a_typed_pair_previews_on_the_axis_and_a_low_above_the_high_does_not() {
    let mut list = list_on_range("", &linear(5.0, 95.0));
    list.feed_events(&[key_event(egui::Key::Enter)]);
    let reports = list.feed_events(&[typed("20")]);
    let previewed = reports.iter().find_map(|r| match r {
        ListReport::Preview(Some(edit)) => Some(edit),
        _ => None,
    });
    let SettingValue::Ends(ends) = &previewed.expect("a pair below the high is previewed").value
    else {
        panic!("the preview is the two ends");
    };
    assert_eq!((ends.lo, ends.hi), (20.0, 95.0));

    list.feed_events(&[
        key_event(egui::Key::Backspace),
        key_event(egui::Key::Backspace),
    ]);
    let reports = list.feed_events(&[typed("99")]);
    assert!(
        !reports
            .iter()
            .any(|r| matches!(r, ListReport::Preview(Some(_)))),
        "a low above the high is not previewed: {reports:?}"
    );
    assert_eq!(
        list.field().and_then(|f| f.refusal.as_deref()),
        None,
        "and it is not refused either"
    );
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

/// The row over a population axis drawn from 0 to 35,682 that the file sets
/// nothing on: *auto*, with the two drawn ends in muted ink and a ring.
fn auto() -> ColumnList {
    list_on_range("", &linear(0.0, 35_682.0))
}

/// The row over the same axis with the file's `[0, 20000]`: the two ends in full
/// ink and a filled dot.
fn set() -> ColumnList {
    list_on_range("xDomain: [0, 20000]", &linear(0.0, 35_682.0))
}

/// The row with `Enter` having opened the two fields on the drawn ends, the low
/// selected.
fn editing() -> ColumnList {
    let mut list = auto();
    list.feed_events(&[key_event(egui::Key::Enter)]);
    list
}

#[test]
fn the_range_row_on_auto_light_matches_its_baseline() {
    baseline_of("shelf_settings_range_auto_light", Mode::Light, auto());
}

#[test]
fn the_range_row_on_auto_dark_matches_its_baseline() {
    baseline_of("shelf_settings_range_auto_dark", Mode::Dark, auto());
}

#[test]
fn the_range_row_on_a_files_two_ends_light_matches_its_baseline() {
    baseline_of("shelf_settings_range_set_light", Mode::Light, set());
}

#[test]
fn the_range_row_on_a_files_two_ends_dark_matches_its_baseline() {
    baseline_of("shelf_settings_range_set_dark", Mode::Dark, set());
}

#[test]
fn the_range_fields_open_on_the_low_light_matches_its_baseline() {
    baseline_of("shelf_settings_range_open_light", Mode::Light, editing());
}

#[test]
fn the_range_fields_open_on_the_low_dark_matches_its_baseline() {
    baseline_of("shelf_settings_range_open_dark", Mode::Dark, editing());
}
