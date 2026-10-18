//! **The line-over-time kind** — a measure drawn against a date or another
//! measure as a `lineY`, one line per category when a category is bound.
//!
//! Read off the *parsed* block rather than its text, for the reason
//! `scatter_kind.rs` gives: a count of lines in a string would pass on a block
//! that named the wrong column.

use brightfield_engine::{ColumnProfile, SemanticType};
use brightfield_shell::chart_kinds;
use brightfield_spec::{parse_spec, Component, Format, Mark, MarkKind, SpecValue, ValueOrParamRef};
use brightfield_workbench::registry::{ChartKind, Field, FieldType};

fn kind() -> &'static ChartKind<String> {
    chart_kinds::find(chart_kinds::LINE_OVER_TIME).expect("this build ships a line-over-time kind")
}

fn column(name: &str, type_name: &str, distinct: u64) -> ColumnProfile {
    ColumnProfile {
        name: name.to_string(),
        type_name: type_name.to_string(),
        non_null: 100,
        nulls: 0,
        distinct,
        min: None,
        max: None,
        semantic: SemanticType::NotAsked,
        moments: None,
    }
}

/// The spec the kind builds over `fields`, parsed.
fn built(fields: &[Field]) -> String {
    let kind = kind();
    let binding = kind.bind(fields).expect("the fields fill the kind's slots");
    kind.spec(&binding, &kind.options())
        .expect("the kind builds its spec")
}

/// The block's marks, wherever it nests them.
fn marks(source: &str) -> Vec<Mark> {
    let spec = parse_spec(source, Format::Yaml)
        .unwrap_or_else(|e| panic!("the block must parse: {e}\n{source}"))
        .spec;
    fn walk(c: &Component, out: &mut Vec<Mark>) {
        match c {
            Component::Mark(m) => out.push(m.clone()),
            Component::Plot(p) => p.items.iter().for_each(|i| walk(i, out)),
            Component::HConcat(h) => h.items.iter().for_each(|i| walk(i, out)),
            Component::VConcat(v) => v.items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(spec.root.as_ref().expect("a root"), &mut out);
    out
}

fn bound(mark: &Mark, channel: &str) -> Option<String> {
    match mark.options.get(channel) {
        Some(ValueOrParamRef::Value(SpecValue::String(name))) => Some(name.clone()),
        _ => None,
    }
}

#[test]
fn builds_a_line_per_category() {
    let source = built(&[
        Field::new("year", FieldType::Temporal),
        Field::new("value", FieldType::Quantitative),
        Field::new("country", FieldType::Categorical),
    ]);
    let marks = marks(&source);
    assert_eq!(marks.len(), 1, "one mark: {source}");
    let line = &marks[0];
    assert_eq!(line.kind, MarkKind::LineY);
    assert_eq!(bound(line, "x").as_deref(), Some("year"));
    assert_eq!(bound(line, "y").as_deref(), Some("value"));
    assert_eq!(bound(line, "stroke").as_deref(), Some("country"));
    assert!(source.contains("legend: color"), "a stroke gets a legend");
}

#[test]
fn without_a_category_it_is_one_unlegended_line() {
    let source = built(&[
        Field::new("year", FieldType::Temporal),
        Field::new("value", FieldType::Quantitative),
    ]);
    let marks = marks(&source);
    assert_eq!(marks.len(), 1);
    assert_eq!(bound(&marks[0], "stroke"), None);
    assert!(!source.contains("legend"), "no stroke, no legend");
}

#[test]
fn a_date_wins_the_x_axis_over_a_measure() {
    // Measures come first in a profiled table's field order, so a first fit
    // would put a measure on x and strand the date.
    let source = built(&[
        Field::new("amount", FieldType::Quantitative),
        Field::new("weight", FieldType::Quantitative),
        Field::new("day", FieldType::Temporal),
    ]);
    let marks = marks(&source);
    assert_eq!(bound(&marks[0], "x").as_deref(), Some("day"));
    assert_eq!(bound(&marks[0], "y").as_deref(), Some("amount"));
}

#[test]
fn it_is_offered_for_a_date_and_a_measure_and_not_for_either_alone() {
    let kind = kind();
    let offered = |cols: &[ColumnProfile]| kind.accepts(&chart_kinds::fields_of(cols));
    assert!(offered(&[
        column("day", "DATE", 30),
        column("amount", "DOUBLE", 900)
    ]));
    assert!(!offered(&[column("day", "DATE", 30)]));
    assert!(!offered(&[column("amount", "DOUBLE", 900)]));
    assert!(!offered(&[
        column("day", "DATE", 30),
        column("region", "VARCHAR", 4)
    ]));
}

#[test]
fn it_sits_after_counts_over_time_and_moves_no_first_look() {
    let ids = chart_kinds::registry().ids();
    let at = |id| ids.iter().position(|k| *k == id).expect("registered");
    assert_eq!(
        at(chart_kinds::LINE_OVER_TIME),
        at(chart_kinds::COUNTS_OVER_TIME) + 1
    );
}

// ---------------------------------------------------------------------------
// The picture: a DATE axis, and a split by category
// ---------------------------------------------------------------------------

/// Three countries over three dates, on a DuckDB table whose x is a `DATE`, y a
/// `DOUBLE` and category a `VARCHAR` (or a `BOOLEAN`), with the rows out of date
/// order.
fn table(category: &str) -> String {
    format!(
        "data:\n  {}:\n    query: \"SELECT * FROM (VALUES \
         (DATE '2024-03-01', 1.0, {c}0), (DATE '2024-01-01', 5.0, {c}0), (DATE '2024-02-01', 2.0, {c}0), \
         (DATE '2024-03-01', 4.0, {c}1), (DATE '2024-01-01', 9.0, {c}1), (DATE '2024-02-01', 6.5, {c}1)) \
         t(day, v, g)\"\n",
        brightfield_shell::data_file::SOURCE,
        c = category
    )
}

/// The paths drawn for the kind's spec over `table`, after `edit` is applied to
/// the built text.
fn paths(table: &str, edit: impl Fn(String) -> String) -> u32 {
    let spec = built(&[
        Field::new("day", FieldType::Temporal),
        Field::new("v", FieldType::Quantitative),
        Field::new("g", FieldType::Categorical),
    ]);
    let source = format!("{table}{}", edit(spec));
    let c = brightfield_shell::pipeline::LiveDashboard::load_str(&source, None)
        .unwrap_or_else(|e| panic!("loads: {e}\n{source}"))
        .present()
        .unwrap_or_else(|e| panic!("composes: {e}\n{source}"));
    assert!(c.mark_faults.is_empty(), "{:?}", c.mark_faults);
    c.scene.encoding().n_paths
}

#[test]
fn a_date_axis_draws_one_series_per_category() {
    let t = table("'k");
    let table = t.replace("'k0", "'a'").replace("'k1", "'b'");
    let no_legend = |s: String| s.replace("  - legend: color\n", "");
    let split = paths(&table, no_legend);
    let one_series = paths(&table, |s| no_legend(s).replace("    stroke: 'g'\n", ""));
    // Two series of three points are four segments; one series through all six
    // rows is five. The lines only exist if the DATE axis drew them.
    assert_eq!(one_series - split, 1, "split {split}, single {one_series}");
}

#[test]
fn a_boolean_category_splits_the_line_too() {
    let table = table("")
        .replace(", 0)", ", true)")
        .replace(", 1)", ", false)");
    let no_legend = |s: String| s.replace("  - legend: color\n", "");
    let split = paths(&table, no_legend);
    let one_series = paths(&table, |s| no_legend(s).replace("    stroke: 'g'\n", ""));
    assert_eq!(one_series - split, 1, "split {split}, single {one_series}");
}

#[test]
fn an_integer_category_splits_the_line_too() {
    // `stroke: year`, the one-line-per-year chart: a number column splits the
    // rows as a text one does, rather than joining every year into one line.
    let table = table("");
    let no_legend = |s: String| s.replace("  - legend: color\n", "");
    let split = paths(&table, no_legend);
    let one_series = paths(&table, |s| no_legend(s).replace("    stroke: 'g'\n", ""));
    assert_eq!(one_series - split, 1, "split {split}, single {one_series}");
    // With its legend it still composes without a fault.
    paths(&table, |s| s);
}

// ---------------------------------------------------------------------------
// A DATE axis is a band of its days, so a line says when they are uneven
// ---------------------------------------------------------------------------

/// The notes the kind's spec, with no category, gives over one series at
/// `days`.
fn notes_over(days: &[&str]) -> Vec<String> {
    let rows: Vec<String> = days
        .iter()
        .enumerate()
        .map(|(i, d)| format!("(DATE '{d}', {i}.0)"))
        .collect();
    let source = format!(
        "data:\n  {}:\n    query: \"SELECT * FROM (VALUES {}) t(day, v)\"\n{}",
        brightfield_shell::data_file::SOURCE,
        rows.join(", "),
        built(&[
            Field::new("day", FieldType::Temporal),
            Field::new("v", FieldType::Quantitative),
        ])
    );
    let c = brightfield_shell::pipeline::LiveDashboard::load_str(&source, None)
        .unwrap_or_else(|e| panic!("loads: {e}\n{source}"))
        .present()
        .unwrap_or_else(|e| panic!("composes: {e}\n{source}"));
    c.diagnostics
        .notes()
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn says_uneven(notes: &[String]) -> bool {
    notes.iter().any(|n| n.contains("not evenly spaced"))
}

#[test]
fn a_line_over_unevenly_spaced_days_says_they_are_drawn_evenly() {
    // Weekdays only: the weekend gap is three days, drawn as wide as one.
    let notes = notes_over(&["2024-01-04", "2024-01-05", "2024-01-08", "2024-01-09"]);
    assert!(says_uneven(&notes), "{notes:?}");
    assert!(
        notes
            .iter()
            .any(|n| n.contains("`lineY`") && n.contains("`x`")),
        "the note names the mark and its axis: {notes:?}"
    );
}

#[test]
fn a_line_over_daily_or_monthly_days_says_nothing_of_spacing() {
    let daily = notes_over(&["2024-01-01", "2024-01-02", "2024-01-03", "2024-01-04"]);
    assert!(!says_uneven(&daily), "{daily:?}");
    // A month apart is one band apart, whatever the month's length.
    let monthly = notes_over(&["2024-01-01", "2024-02-01", "2024-03-01", "2024-04-01"]);
    assert!(!says_uneven(&monthly), "{monthly:?}");
    let month_ends = notes_over(&["2024-01-31", "2024-02-29", "2024-03-31", "2024-04-30"]);
    assert!(!says_uneven(&month_ends), "{month_ends:?}");
}

/// The colours a `lineY` split by an integer column draws over [`table`], with
/// `keys` written as plot attributes.
fn stroke_colours(keys: &str) -> std::collections::HashSet<u32> {
    let source = format!(
        "{}plot:\n  - mark: lineY\n    data: {{ from: {} }}\n    x: day\n    y: v\n    stroke: g\n{keys}",
        table(""),
        brightfield_shell::data_file::SOURCE
    );
    let c = brightfield_shell::pipeline::LiveDashboard::load_str(&source, None)
        .unwrap_or_else(|e| panic!("loads: {e}\n{source}"))
        .present()
        .unwrap_or_else(|e| panic!("composes: {e}\n{source}"));
    assert!(c.mark_faults.is_empty(), "{:?}", c.mark_faults);
    c.scene.encoding().draw_data.iter().copied().collect()
}

#[test]
fn a_number_stroke_line_takes_the_plots_colour_keys() {
    // Premultiplied little-endian RGBA8, as the scene encodes a solid colour.
    let (red, blue) = (0xFF00_00FF_u32, 0xFFFF_0000_u32);
    let written = stroke_colours("colorRange: [red, blue]\n");
    assert!(
        written.contains(&red) && written.contains(&blue),
        "the two series wear the written colours"
    );
    let default = stroke_colours("");
    assert!(!default.contains(&red) && !default.contains(&blue));
    assert_ne!(
        stroke_colours("colorScheme: turbo\n"),
        default,
        "a written scheme paints the series along it"
    );
}
