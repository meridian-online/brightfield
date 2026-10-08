//! **The scale row of an axis of names or dates reads the scale the chart draws,
//! band or time, and stays unset.**
//!
//! The row used to read the type the plot resolves to, which is `linear`, `log`
//! or `symlog`, so an analyst read `linear` over an axis of names or of dates,
//! which the chart draws as a band or a time scale. The row now reads what the
//! chart was drawn against
//! (`an_axis_of_names_reads_band_and_an_axis_of_dates_reads_time`). It stays
//! *auto* with no dot, since band and time are brightfield's own choice there
//! and a value is the analyst's when it differs from brightfield's own
//! (`a_band_or_time_row_is_not_set_because_it_is_brightfields_own_choice`).
//! The cell carrying the scale's name is `shelf_band.rs`; the window handing
//! the list the scales its chart was drawn against is
//! `shelf_settings_window.rs`.
//!
//! Each reading is at the altitude its claim lives at. What the row reads and
//! whether it is set are read off the rows `ChannelSettings` builds from a
//! scale set. What `h` and `l` do is read off the reports a list answers a key
//! with, since a write the list does not report is a write the chart does not
//! get. The sentence under the cursor is read as the foot builds it.

use brightfield_render::channel::Channel;
use brightfield_render::scale::{Scale, ScaleSet};
use brightfield_shell::shelf::{
    foot_sentence, Binding, ChannelSettings, ColumnList, ColumnListRequest, ListColumn, ListReport,
    ListTab, SettingRow, ShelfChannels, BAND_SCALE, SCALE_ROW, TIME_SCALE,
};
use brightfield_spec::edit::plot_at_path;
use brightfield_spec::parse::{parse_spec, Format};
use brightfield_workbench::channel::ShelfChannel;

// ---------------------------------------------------------------------------
// The fixture: the channels the tile takes, and the scales it drew.
// ---------------------------------------------------------------------------

fn channels() -> ShelfChannels {
    ShelfChannels {
        mark: "dot".to_string(),
        x: Binding::Column("ocean_proximity".to_string()),
        y: Binding::Column("latitude".to_string()),
        colour: Binding::Unset,
    }
}

fn linear() -> Scale {
    Scale::Linear {
        domain_min: 0.0,
        domain_max: 10.0,
        range_start: 0.0,
        range_end: 100.0,
    }
}

fn time() -> Scale {
    Scale::Time {
        domain_min_us: 0,
        domain_max_us: 86_400_000_000,
        range_start: 0.0,
        range_end: 100.0,
    }
}

fn names() -> Scale {
    Scale::Band {
        categories: vec!["inland".to_string(), "coast".to_string()],
        range_start: 0.0,
        range_end: 100.0,
        padding: 0.1,
    }
}

/// What a plot draws when its x is `x` and its y is `y`.
fn drawn_with(x: &Scale, y: &Scale) -> ScaleSet {
    let mut set = ScaleSet::new();
    set.insert(Channel::X, x.clone());
    set.insert(Channel::Y, y.clone());
    set
}

/// The plot attributes' reading as the window hands it to the list: a one-plot
/// spec whose top-level lines after `height` are `attrs`, read against the
/// scales `drawn`.
fn settings_of(attrs: &str, drawn: &ScaleSet) -> ChannelSettings {
    let source = format!(
        "data:\n  t:\n    - {{ a: 1 }}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: a\nwidth: 600\nheight: 300\n{attrs}\n"
    );
    let spec = parse_spec(&source, Format::Yaml)
        .expect("the spec parses")
        .spec;
    let plot = plot_at_path(&spec, "root").expect("the spec's root is its plot");
    ChannelSettings::of_plot_drawn(&spec, plot, &channels(), drawn)
}

/// The scale row of `channel`, as the plot with `attrs` reads against `drawn`.
fn scale_row(attrs: &str, drawn: &ScaleSet, channel: ShelfChannel) -> SettingRow {
    settings_of(attrs, drawn)
        .rows(channel)
        .iter()
        .find(|r| r.name == SCALE_ROW)
        .unwrap_or_else(|| panic!("{channel:?} has no scale row"))
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

/// A list on x's settings over a plot that writes `attrs`, drawn against
/// `drawn`, the cursor on the scale row.
fn list_on_scale_row(attrs: &str, drawn: &ScaleSet) -> ColumnList {
    let mut list = ColumnList::new(ColumnListRequest {
        tile: "hero".to_string(),
        channel: ShelfChannel::X,
        channels: channels(),
        columns: vec![ListColumn {
            name: "ocean_proximity".to_string(),
            kind: "VARCHAR".to_string(),
            moments: None,
        }],
    });
    list.set_settings(settings_of(attrs, drawn));
    list.feed_events(&[key_event(egui::Key::Tab)]);
    assert_eq!(list.tab(), ListTab::Settings, "Tab turned the list");
    list.feed_events(&[key_event(egui::Key::J)]);
    assert_eq!(
        list.setting_cursor().map(|r| r.name),
        Some(SCALE_ROW),
        "j moved the cursor to the scale row"
    );
    list
}

// ---------------------------------------------------------------------------
// AC1: what the row reads, and that it stays unset.
// ---------------------------------------------------------------------------

/// **An axis of names reads `band` and an axis of dates reads `time`, each from
/// the scale the chart drew for that axis, and an axis of numbers reads on.** The
/// two axes are read apart: names on x beside numbers on y leaves y at `linear`.
#[test]
fn an_axis_of_names_reads_band_and_an_axis_of_dates_reads_time() {
    let cases = [
        (names(), linear(), BAND_SCALE, "linear"),
        (time(), linear(), TIME_SCALE, "linear"),
        (linear(), names(), "linear", BAND_SCALE),
        (linear(), time(), "linear", TIME_SCALE),
        (names(), time(), BAND_SCALE, TIME_SCALE),
        (linear(), linear(), "linear", "linear"),
    ];
    for (x, y, on_x, on_y) in cases {
        let drawn = drawn_with(&x, &y);
        assert_eq!(
            scale_row("", &drawn, ShelfChannel::X).value,
            on_x,
            "x of {x:?} beside {y:?}"
        );
        assert_eq!(
            scale_row("", &drawn, ShelfChannel::Y).value,
            on_y,
            "y of {y:?} beside {x:?}"
        );
    }
}

/// **The row reads band or time and stays unset**, since band and time are
/// brightfield's own choice there and a value is the analyst's only when it
/// differs from brightfield's own. The cell marks a channel by its set rows, so
/// an unset row is also what leaves the dot off.
#[test]
fn a_band_or_time_row_is_not_set_because_it_is_brightfields_own_choice() {
    for scale in [names(), time()] {
        let drawn = drawn_with(&scale, &scale);
        for channel in [ShelfChannel::X, ShelfChannel::Y] {
            let row = scale_row("", &drawn, channel);
            assert!(!row.set, "{channel:?} over {scale:?} reads {}", row.value);
            assert!(row.reason.is_none(), "the scale row applies to every axis");
        }
    }
}

/// **A key the file writes still marks the row**: `xScale: log` over an axis of
/// names differs from brightfield's own, so it is the analyst's and the row
/// reads the scale the chart draws and carries the dot. Without the key written
/// the row is unset.
#[test]
fn a_key_the_file_writes_over_an_axis_of_names_still_marks_the_row() {
    let drawn = drawn_with(&names(), &linear());
    let row = scale_row("xScale: log", &drawn, ShelfChannel::X);
    assert_eq!(row.value, BAND_SCALE, "the chart draws a band scale");
    assert!(row.set, "the file's log differs from brightfield's own");
    assert!(
        !scale_row("", &drawn, ShelfChannel::X).set,
        "with the key out the row is unset"
    );
}

/// **A plot with no scales drawn reads as it did**: the type the plot resolves
/// to, so a list built before any chart is drawn never invents a band.
#[test]
fn with_no_scales_drawn_the_row_reads_the_type_the_plot_resolves_to() {
    let none = ScaleSet::new();
    assert_eq!(scale_row("", &none, ShelfChannel::X).value, "linear");
    assert_eq!(
        scale_row("xScale: log", &none, ShelfChannel::X).value,
        "log"
    );
}

// ---------------------------------------------------------------------------
// AC1: what `h` and `l` do on the row, and the sentence under it.
// ---------------------------------------------------------------------------

/// **`l` on a band or a time row leaves it reading band or time**, and so do
/// `h` and the arrows: the three the row steps through are for numbers, so the
/// list reports no write and the row does not step.
#[test]
fn l_on_a_band_or_time_row_leaves_it_reading_band_or_time() {
    for (scale, word) in [(names(), BAND_SCALE), (time(), TIME_SCALE)] {
        let drawn = drawn_with(&scale, &scale);
        let mut list = list_on_scale_row("", &drawn);
        for key in [
            egui::Key::L,
            egui::Key::H,
            egui::Key::ArrowRight,
            egui::Key::ArrowLeft,
        ] {
            let reports = list.feed_events(&[key_event(key)]);
            let wrote: Vec<&ListReport> = reports
                .iter()
                .filter(|r| matches!(r, ListReport::Set(_)))
                .collect();
            assert!(wrote.is_empty(), "{key:?} on a {word} row wrote {wrote:?}");
            let row = list.setting_cursor().expect("a row is under the cursor");
            assert_eq!(row.value, word, "{key:?} left the row reading {word}");
        }
    }
}

/// **A band or time row is not one that steps**, so the list draws no `←` `→`
/// chips on it, while a numeric scale row still does.
#[test]
fn a_band_or_time_row_does_not_step_and_a_numeric_scale_row_does() {
    for scale in [names(), time()] {
        let row = scale_row("", &drawn_with(&scale, &scale), ShelfChannel::X);
        assert!(!row.steps(), "{} steps", row.value);
        assert_eq!(row.stepped(1), None);
        assert_eq!(row.stepped(-1), None);
    }
    for attrs in ["", "xScale: log", "xScale: symlog"] {
        let row = scale_row(attrs, &drawn_with(&linear(), &linear()), ShelfChannel::X);
        assert!(row.steps(), "`{attrs}` reads {} and steps", row.value);
    }
}

/// **The sentence under the scale row says what auto draws on this axis.** A row
/// reading band says auto draws band for names, one reading time says time for
/// dates, and a numeric row keeps the sentence it had.
#[test]
fn the_sentence_under_the_scale_row_says_what_auto_draws_on_the_axis() {
    let said =
        |scale: &Scale| foot_sentence(&scale_row("", &drawn_with(scale, scale), ShelfChannel::X));
    assert_eq!(
        said(&names()),
        "How values are spaced along the axis, which auto draws band for names."
    );
    assert_eq!(
        said(&time()),
        "How values are spaced along the axis, which auto draws time for dates."
    );
    assert_eq!(
        said(&linear()),
        "How values are spaced along the axis, which auto draws linear."
    );
}
