//! **The range row takes two ends, and a scale change or a new column puts a set
//! range back to auto.**
//!
//! The row stands third at the head of an axis's list. It reads *auto* with the
//! ends the axis was drawn over in muted ink while the file sets none, and the
//! file's two ends in full ink with a filled dot when it does. `Enter` opens a low
//! and a high field, `Enter` on the low moves to the high, and `Enter` on the high
//! keeps the pair.
//!
//! This file holds the list alone and the write path. The pixels of the row at
//! rest and mid-edit are not committed here; the pull request names that.

use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, ScaleSet};
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

/// A list on x's settings with the cursor on the range row, the third.
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
    list.feed_events(&[key_event(egui::Key::J), key_event(egui::Key::J)]);
    assert_eq!(list.setting_cursor().map(|r| r.name), Some(RANGE_ROW));
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
/// under itself, `Enter` reports it for the status band, and nothing is kept.
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
