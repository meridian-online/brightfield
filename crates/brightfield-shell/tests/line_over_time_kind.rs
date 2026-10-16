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
