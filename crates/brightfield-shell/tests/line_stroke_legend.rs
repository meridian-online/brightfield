//! **A plot's legend is named for the column its legend scale is built from.**
//!
//! The legend scale is the plot's fill scale, or its stroke scale when no mark
//! fills. The name follows the same order across every mark of the plot, not
//! the first mark that colours anything.

use brightfield_shell::legend::LegendSpec;
use brightfield_shell::pipeline::{compose_spec_str, Composed};

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

const ROWS: &str = "data:
  t:
    - { x: 0, y: 1, country: a, kind: p }
    - { x: 1, y: 3, country: a, kind: q }
    - { x: 2, y: 2, country: a, kind: p }
    - { x: 0, y: 4, country: b, kind: q }
    - { x: 1, y: 6, country: b, kind: p }
    - { x: 2, y: 5, country: b, kind: q }
";

const LINE: &str = "  - mark: lineY
    data: { from: t }
    x: x
    y: y
    stroke: country
";

const DOTS: &str = "  - mark: dot
    data: { from: t }
    x: x
    y: y
    fill: kind
";

fn page(marks: &[&str]) -> String {
    format!(
        "{ROWS}plot:\n{}  - legend: color\nwidth: 420\nheight: 300\n",
        marks.concat()
    )
}

fn labels(composed: &Composed) -> Vec<String> {
    match LegendSpec::of_plot(&composed.plots[0]) {
        Some(LegendSpec::Categorical { entries }) => entries.into_iter().map(|e| e.label).collect(),
        other => panic!("a categorical legend, got {other:?}"),
    }
}

#[test]
fn a_stroked_line_alone_names_its_stroke_column() {
    let composed = compose(&page(&[LINE]));
    assert_eq!(composed.plots[0].fill_column.as_deref(), Some("country"));
    assert_eq!(labels(&composed), ["a", "b"]);
}

#[test]
fn a_fill_wins_the_legend_name_whichever_mark_comes_first() {
    for marks in [[LINE, DOTS], [DOTS, LINE]] {
        let composed = compose(&page(&marks));
        assert_eq!(
            composed.plots[0].fill_column.as_deref(),
            Some("kind"),
            "the legend draws the fill scale, so it is named for the fill column"
        );
        assert_eq!(labels(&composed), ["p", "q"]);
    }
}
