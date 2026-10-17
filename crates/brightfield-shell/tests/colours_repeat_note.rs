//! **A plot whose colour scale has more categories than the palette has colours
//! says so, once; a plot that fits the palette, or writes its own colours, says
//! nothing.**
//!
//! These run the whole composition over inline data and read the diagnostics the
//! page is handed.

use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_shell::pipeline::{compose_spec_str, Composed};

/// A line over `series` categories, one stroke colour each; `attrs` is the
/// plot's attributes.
fn spec(series: usize, attrs: &str) -> String {
    with_mark(series, "line", "stroke", attrs)
}

/// The same over `mark`, coloured through `channel`. Only a dot reads a
/// `colorRange` the plot writes.
fn with_mark(series: usize, mark: &str, channel: &str, attrs: &str) -> String {
    let rows: String = (0..series)
        .flat_map(|s| (0..3).map(move |x| format!("    - {{ x: {x}, y: {}, s: s{s} }}\n", x + s)))
        .collect();
    format!(
        "data:\n  t:\n{rows}plot:\n  - mark: {mark}\n    data: {{ from: t }}\n    x: x\n    y: y\n    {channel}: s\nwidth: 420\nheight: 300\n{attrs}"
    )
}

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

fn palette_size(composed: &Composed) -> usize {
    match composed.plots[0].scales.get(Channel::Stroke) {
        Some(Scale::Colour { palette, .. }) => palette.len(),
        other => panic!("the stroke is not a colour scale: {other:?}"),
    }
}

#[test]
fn nine_series_note_that_colours_repeat() {
    let composed = compose(&spec(9, ""));
    let advisory = composed.diagnostics.advisory();
    let notes: Vec<&str> = advisory
        .iter()
        .map(|d| d.message.as_str())
        .filter(|m| m.contains("colours repeat"))
        .collect();
    assert_eq!(notes.len(), 1, "{:?}", composed.diagnostics.lines());
    let p = palette_size(&composed);
    assert_eq!(p, 8, "the palette has eight colours today");
    assert_eq!(
        notes[0],
        format!(
            "9 series share {p} colours, so colours repeat; highlight the series that matter, \
             fold the rest into \"Other\", or split the chart"
        )
    );
    assert_eq!(composed.diagnostics.colours_repeat(), Some(notes[0]));
}

#[test]
fn eight_series_carry_no_note() {
    let composed = compose(&spec(8, ""));
    assert_eq!(
        composed.diagnostics.colours_repeat(),
        None,
        "{:?}",
        composed.diagnostics.lines()
    );
}

#[test]
fn nine_dot_categories_note_that_colours_repeat_without_a_range() {
    let composed = compose(&with_mark(9, "dot", "fill", ""));
    assert!(
        composed.diagnostics.colours_repeat().is_some(),
        "{:?}",
        composed.diagnostics.lines()
    );
}

#[test]
fn an_explicit_colour_range_carries_no_note() {
    let range: Vec<String> = (0..9).map(|i| format!("\"#{i}0{i}0{i}0\"")).collect();
    let attrs = format!("colorRange: [{}]\n", range.join(", "));
    let composed = compose(&with_mark(9, "dot", "fill", &attrs));
    assert_eq!(
        composed.diagnostics.colours_repeat(),
        None,
        "{:?}",
        composed.diagnostics.lines()
    );
}
