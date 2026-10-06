//! **A tick format on an axis of names is named in the warning banner, and the
//! axis draws its names as it does without the key.**
//!
//! A band of names prints the names. Mosaic's own band axis does too, so a
//! number format or a date format on one is a promise brightfield cannot keep,
//! and until now it was dropped without a word. The banner names the key as
//! changing nothing on a category axis, whether the format is a number format or
//! a date format and whether or not the names read as numbers.
//!
//! Assertions read the TEXT that was PAINTED, decoded from the glyph runs on
//! `Composed::scene`, as `axis_vocabulary_date_format` does and for the same
//! reason: a format resolved and dropped before the draw would pass a check that
//! read the resolver. An asked-for arm is paired with the same spec asking for
//! nothing, and with an axis that does take the format.

use brightfield_render::text::{draw_text, TextAnchor, LABEL_SIZE};
use brightfield_shell::pipeline::{Composed, LiveDashboard};

/// What the load says of a key that changes nothing where it lands.
const INERT: &str = "changes nothing on a category axis";

/// Four regions on the x axis. `ATTRS` marks where the plot attributes go.
const REGIONS_ON_X: &str = r#"
data:
  d:
    query: "SELECT 'north' AS region, 30 AS n UNION ALL SELECT 'south', 50 UNION ALL SELECT 'east', 20 UNION ALL SELECT 'west', 40"
plot:
  - mark: dot
    data: { from: d }
    x: region
    y: n
name: probe
width: 600
height: 300
ATTRS
"#;

/// The same four regions on the y axis.
const REGIONS_ON_Y: &str = r#"
data:
  d:
    query: "SELECT 'north' AS region, 30 AS n UNION ALL SELECT 'south', 50 UNION ALL SELECT 'east', 20 UNION ALL SELECT 'west', 40"
plot:
  - mark: dot
    data: { from: d }
    x: n
    y: region
name: probe
width: 600
height: 300
ATTRS
"#;

/// Names that read as numbers, spelled as text, on the x axis: a column of
/// `VARCHAR`, so the axis is a band and not a number axis.
const NUMBER_NAMES_ON_X: &str = r#"
data:
  d:
    query: "SELECT CAST(i AS VARCHAR) AS id, i * 10 AS n FROM range(1, 5) t(i)"
plot:
  - mark: dot
    data: { from: d }
    x: id
    y: n
name: probe
width: 600
height: 300
ATTRS
"#;

/// Two numeric columns, so the axes are number axes.
const NUMBERS: &str = r#"
data:
  d:
    query: "SELECT i * 25 AS a, i * 10 AS n FROM range(5) t(i)"
plot:
  - mark: dot
    data: { from: d }
    x: a
    y: n
name: probe
width: 600
height: 300
ATTRS
"#;

fn compose(template: &str, attrs: &str) -> Composed {
    LiveDashboard::load_str(&template.replace("ATTRS", attrs), None)
        .expect("the spec loads")
        .present()
        .expect("the spec composes")
}

// ---------------------------------------------------------------------------
// Reading the painted text
// ---------------------------------------------------------------------------

/// The glyph id the axis's own text drawing gives `c`.
fn glyph_of(c: char) -> u32 {
    let mut scene = vello::Scene::new();
    draw_text(
        &mut scene,
        &c.to_string(),
        0.0,
        0.0,
        LABEL_SIZE,
        peniko::Color::BLACK,
        TextAnchor::Start,
    );
    let glyphs = &scene.encoding().resources.glyphs;
    assert_eq!(glyphs.len(), 1, "{c:?} paints as one glyph");
    glyphs[0].id
}

/// Every character a tick can paint in these fixtures.
const PAINTABLE: &str = "0123456789.,:+- abcdefghijklmnopqrstuvwxyz";

/// The horizontal, label-sized glyph runs as `(x0, baseline_y, text)`, each glyph
/// decoded back to its character. A character outside [`PAINTABLE`] decodes to
/// `?`, which no expected text holds, so it fails an equality and not a lookup.
fn label_runs(composed: &Composed) -> Vec<(f64, f64, String)> {
    let by_glyph: Vec<(u32, char)> = PAINTABLE.chars().map(|c| (glyph_of(c), c)).collect();
    let resources = &composed.scene.encoding().resources;
    resources
        .glyph_runs
        .iter()
        .filter(|run| {
            (run.font_size - LABEL_SIZE).abs() < 0.01 && run.transform.matrix[0].abs() > 0.5
        })
        .map(|run| {
            let text: String = resources.glyphs[run.glyphs.clone()]
                .iter()
                .map(|g| {
                    by_glyph
                        .iter()
                        .find(|(id, _)| *id == g.id)
                        .map_or('?', |(_, c)| *c)
                })
                .collect();
            (
                f64::from(run.transform.translation[0]),
                f64::from(run.transform.translation[1]),
                text,
            )
        })
        .collect()
}

/// A tick label sits below the axis line on the x axis and beside its tick on
/// the y axis, so the plot's bottom edge plus ten separates the two rows.
const ROW_SPLIT: f64 = 10.0;

/// The x axis's painted tick text, left to right.
fn painted_x(composed: &Composed) -> Vec<String> {
    let plot = &composed.plots[0];
    let x_row = plot.rect.y + plot.layout.plot_y_end() + ROW_SPLIT;
    let mut runs: Vec<(f64, String)> = label_runs(composed)
        .into_iter()
        .filter(|&(_, y, _)| y > x_row)
        .map(|(x, _, text)| (x, text))
        .collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    runs.into_iter().map(|(_, text)| text).collect()
}

/// The y axis's painted tick text, top to bottom.
fn painted_y(composed: &Composed) -> Vec<String> {
    let plot = &composed.plots[0];
    let x_row = plot.rect.y + plot.layout.plot_y_end() + ROW_SPLIT;
    let axis_x = plot.rect.x + plot.layout.plot_x_start();
    let mut runs: Vec<(f64, String)> = label_runs(composed)
        .into_iter()
        .filter(|&(x, y, _)| y <= x_row && x < axis_x)
        .map(|(_, y, text)| (y, text))
        .collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0));
    runs.into_iter().map(|(_, text)| text).collect()
}

/// What the load said, one line per diagnostic: what the warning banner draws.
fn said(composed: &Composed) -> Vec<String> {
    composed.diagnostics.lines()
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

// ---------------------------------------------------------------------------
// AC1 — a format on an axis of names
// ---------------------------------------------------------------------------

/// **A number format and a date format on an axis of names each draw the names
/// the axis drew without the key, and the banner names the key once, as changing
/// nothing on a category axis.** Both axes, both kinds of format.
#[test]
fn a_tick_format_on_an_axis_of_names_is_named_and_the_names_are_drawn() {
    let regions = strs(&["north", "south", "east", "west"]);
    // A y band runs from the bottom up, so the first region is the lowest.
    let regions_up = strs(&["west", "east", "south", "north"]);
    assert_eq!(painted_x(&compose(REGIONS_ON_X, "")), regions);
    assert_eq!(painted_y(&compose(REGIONS_ON_Y, "")), regions_up);

    for (key, value) in [
        ("xTickFormat", ".2f"),
        ("xTickFormat", "s"),
        ("xTickFormat", "%b"),
    ] {
        let asked = compose(REGIONS_ON_X, &format!("{key}: '{value}'"));
        assert_eq!(
            painted_x(&asked),
            regions,
            "`{key}: {value}` draws the names"
        );
        let lines = said(&asked);
        assert_eq!(
            lines.len(),
            1,
            "one line for `{key}: {value}`; got {lines:?}"
        );
        assert!(
            lines[0].contains(&format!("`{key}`"))
                && lines[0].contains("root (`probe`)")
                && lines[0].contains(INERT),
            "the line names the key, the plot and the axis; got {:?}",
            lines[0]
        );
    }
    for value in [".2f", "%b"] {
        let asked = compose(REGIONS_ON_Y, &format!("yTickFormat: '{value}'"));
        assert_eq!(
            painted_y(&asked),
            regions_up,
            "`yTickFormat: {value}` draws the names"
        );
        let lines = said(&asked);
        assert_eq!(
            lines.len(),
            1,
            "one line for `yTickFormat: {value}`; {lines:?}"
        );
        assert!(
            lines[0].contains("`yTickFormat`") && lines[0].contains(INERT),
            "{:?}",
            lines[0]
        );
    }
}

/// **Whether or not the names read as numbers.** A column of text that spells
/// `1` to `4` is a band of names and not a number axis, so a number format on it
/// does not format them as numbers, and says so.
#[test]
fn a_number_format_on_names_that_read_as_numbers_is_named_too() {
    let plain = compose(NUMBER_NAMES_ON_X, "");
    assert_eq!(painted_x(&plain), strs(&["1", "2", "3", "4"]));
    assert!(said(&plain).is_empty(), "{:?}", said(&plain));

    let asked = compose(NUMBER_NAMES_ON_X, "xTickFormat: '.2f'");
    assert_eq!(
        painted_x(&asked),
        strs(&["1", "2", "3", "4"]),
        "the names are drawn as spelled, not as `1.00`"
    );
    let lines = said(&asked);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("`xTickFormat`") && lines[0].contains(INERT),
        "{:?}",
        lines[0]
    );
}

/// **A format reaches a plot from `plotDefaults:` and is named at the plot it
/// lands on**, as a count or an end set there is.
#[test]
fn a_format_under_plot_defaults_is_named_at_the_plot_whose_names_it_lands_on() {
    let asked = compose(REGIONS_ON_X, "plotDefaults:\n  xTickFormat: '.2f'");
    let lines = said(&asked);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("`xTickFormat`") && lines[0].contains(INERT),
        "{:?}",
        lines[0]
    );
}

// ---------------------------------------------------------------------------
// AC3 — no new line
// ---------------------------------------------------------------------------

/// **A plot that sets no format on an axis of names says nothing, and a format
/// on an axis that takes it says nothing**: the number axis of the same plot, and
/// a `null`, which is Mosaic's "no format".
#[test]
fn an_axis_that_takes_the_format_or_is_asked_for_none_says_nothing() {
    for attrs in [
        "",
        "xTickFormat: null",
        "yTickFormat: s",
        "yTickFormat: '.1f'",
    ] {
        let composed = compose(REGIONS_ON_X, attrs);
        assert!(
            said(&composed).is_empty(),
            "`{attrs}` on names: {:?}",
            said(&composed)
        );
    }
    let numbers = compose(NUMBERS, "xTickFormat: s\nyTickFormat: '.1f'");
    assert!(said(&numbers).is_empty(), "{:?}", said(&numbers));
}

/// **A format that is no format at all is named once, as such, and not again as
/// changing nothing**: the typo is the lead and the axis of names is not asked.
#[test]
fn a_format_that_is_no_format_is_named_once_as_such_on_an_axis_of_names() {
    let asked = compose(REGIONS_ON_X, "xTickFormat: '~~'");
    let lines = said(&asked);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("`~~`") && !lines[0].contains(INERT),
        "{:?}",
        lines[0]
    );
}

/// A re-present rebuilds the composition and the load's diagnostics are attached
/// to each: the line must be said once, on the first paint and on the repaint.
#[test]
fn a_repaint_says_a_format_on_names_once() {
    let mut live =
        LiveDashboard::load_str(&REGIONS_ON_X.replace("ATTRS", "xTickFormat: '.2f'"), None)
            .expect("the spec loads");
    let first = live.present().expect("composes");
    let second = live.present().expect("composes again");
    assert_eq!(said(&first).len(), 1, "{:?}", said(&first));
    assert_eq!(said(&second), said(&first));
}
