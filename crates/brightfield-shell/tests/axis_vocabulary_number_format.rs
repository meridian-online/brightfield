//! **A spec that sets `xTickFormat` or `yTickFormat` draws the tick text it
//! asked for.**
//!
//! Neither key was read by any code, so an axis drew a whole number as an
//! integer and any other value to one decimal place whatever the spec said: an
//! analyst charting revenue read `2000000` where the chart file asked for `2M`.
//!
//! Assertions read the TEXT that was PAINTED, decoded from the glyph runs on
//! `Composed::scene`, which is what a viewer is shown. A format resolved
//! correctly and dropped before the draw would pass a check that read the
//! resolver, and a label with the right glyph count but the wrong characters
//! (`1.5k` against `1.5M`) would pass a check that counted glyphs. Each glyph
//! is decoded back to a character by drawing that character through the same
//! `draw_text` the axis draws with and matching glyph ids.
//!
//! An asked-for arm is paired with the same spec asking for nothing, and with
//! the other axis left alone. A text that changed could be a fixture whose
//! default is the text asked for, and an axis that followed the wrong key
//! would pass a test that set one key on one axis.

use brightfield_render::text::{draw_text, TextAnchor, LABEL_SIZE};
use brightfield_shell::pipeline::{Composed, LiveDashboard};

// ---------------------------------------------------------------------------
// The fixture
// ---------------------------------------------------------------------------

/// Two columns that each run from 0 to `MAX`, so both axes infer the same
/// `[0, MAX]` domain and a spec that formats one axis can be read against the
/// other left alone. `ATTRS` marks where the plot attributes go.
const TEMPLATE: &str = r"
data:
  pts:
    - { a: 0,       b: 0 }
    - { a: QUARTER, b: SIXTY }
    - { a: SEVENTY, b: THIRTY }
    - { a: MAX,     b: MAX }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
width: 600
height: 300
ATTRS
";

/// A plot whose axes run from 0 to `max`, with `attrs` as its plot attributes.
fn compose_to(max: f64, attrs: &str) -> Composed {
    let spec = TEMPLATE
        .replace("QUARTER", &format!("{}", max * 0.25))
        .replace("SIXTY", &format!("{}", max * 0.6))
        .replace("SEVENTY", &format!("{}", max * 0.7))
        .replace("THIRTY", &format!("{}", max * 0.3))
        .replace("MAX", &format!("{max}"))
        .replace("ATTRS", attrs);
    LiveDashboard::load_str(&spec, None)
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

/// The horizontal, label-sized glyph runs as `(x0, baseline_y, text)`, each
/// glyph decoded back to its character. The axis titles are a different size
/// and the y title is rotated, so neither is in this list.
fn label_runs(composed: &Composed) -> Vec<(f64, f64, String)> {
    let by_glyph: Vec<(u32, char)> = "0123456789.,%+-kMG"
        .chars()
        .map(|c| (glyph_of(c), c))
        .collect();
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

/// A tick label is placed `LABEL_SIZE / 3` below its tick on the y axis and
/// `TICK_LENGTH + LABEL_SIZE` below the axis line on the x axis (both private
/// to `axis.rs`), so the plot's bottom edge plus ten separates the two rows
/// without naming either constant.
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

/// The y axis's painted tick text, top to bottom: they sit left of the axis
/// line, above the x row.
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

/// What the load said, one line per diagnostic.
fn said(composed: &Composed) -> Vec<String> {
    composed.diagnostics.lines()
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

// ---------------------------------------------------------------------------
// AC5 — the text an axis draws with no format
// ---------------------------------------------------------------------------

/// **A spec that sets neither key draws the tick text it drew before.** A whole
/// number reads as an integer and a fraction to one decimal place, on both
/// axes, and the load says nothing. The committed dashboard baselines carry the
/// same claim at the whole-dashboard level, `dashboard_baseline`.
#[test]
fn a_spec_that_sets_no_tick_format_draws_the_text_it_always_drew() {
    let hundred = compose_to(100.0, "");
    assert_eq!(
        painted_x(&hundred),
        strs(&["0", "20", "40", "60", "80", "100"])
    );
    assert_eq!(
        painted_y(&hundred),
        strs(&["100", "80", "60", "40", "20", "0"])
    );
    assert!(said(&hundred).is_empty(), "{:?}", said(&hundred));

    let unit = compose_to(1.0, "");
    assert_eq!(
        painted_x(&unit),
        strs(&["0", "0.2", "0.4", "0.6", "0.8", "1"])
    );
}

// ---------------------------------------------------------------------------
// AC1 — a precision in the format
// ---------------------------------------------------------------------------

/// **With a precision in the format, tick text reads as d3-format prints it.**
/// Under `.2s` the ticks of a 0 to 2000000 axis read `0.00M`, `0.50M`, `1.00M`,
/// `1.50M` and `2.00M`: the analyst's two decimals, and the one prefix the
/// largest tick takes, as d3-scale shares it. The `s` of AC2 differs only in
/// where the decimals come from.
#[test]
fn a_precision_in_the_format_prints_each_tick_as_d3_format_does() {
    let composed = compose_to(2_000_000.0, "xTickFormat: '.2s'");
    assert_eq!(
        painted_x(&composed),
        strs(&["0.00M", "0.50M", "1.00M", "1.50M", "2.00M"])
    );
    // The y axis names no format and is left as it was.
    assert_eq!(
        painted_y(&composed),
        strs(&["2000000", "1500000", "1000000", "500000", "0"])
    );

    let grouped = compose_to(2_000_000.0, "yTickFormat: ',d'");
    assert_eq!(
        painted_y(&grouped),
        strs(&["2,000,000", "1,500,000", "1,000,000", "500,000", "0"])
    );
    assert_eq!(
        painted_x(&grouped),
        strs(&["0", "500000", "1000000", "1500000", "2000000"])
    );

    let percent = compose_to(1.0, "xTickFormat: '.0%'");
    assert_eq!(
        painted_x(&percent),
        strs(&["0%", "20%", "40%", "60%", "80%", "100%"])
    );

    let signed = compose_to(5.0, "yTickFormat: '+.1f'");
    assert_eq!(
        painted_y(&signed),
        strs(&["+5.0", "+4.0", "+3.0", "+2.0", "+1.0", "+0.0"])
    );
}

// ---------------------------------------------------------------------------
// AC2 — no precision in the format
// ---------------------------------------------------------------------------

/// **With no precision in the format, the precision follows the tick step.**
/// Under `s` the ticks of a 0 to 2000 axis read `0.0k` … `2.0k`, one decimal
/// because the step is 500, and under `%` the ticks of a 0 to 1 axis read `0%`
/// … `100%`, none because the step is 0.2.
#[test]
fn a_format_with_no_precision_follows_the_tick_step() {
    let composed = compose_to(2000.0, "xTickFormat: s");
    assert_eq!(
        painted_x(&composed),
        strs(&["0.0k", "0.5k", "1.0k", "1.5k", "2.0k"])
    );
    assert_eq!(
        painted_y(&composed),
        strs(&["2000", "1500", "1000", "500", "0"]),
        "the y axis names no format"
    );

    let percent = compose_to(1.0, "yTickFormat: '%'");
    assert_eq!(
        painted_y(&percent),
        strs(&["100%", "80%", "60%", "40%", "20%", "0%"])
    );
    assert_eq!(
        painted_x(&percent),
        strs(&["0", "0.2", "0.4", "0.6", "0.8", "1"]),
        "the x axis names no format"
    );

    // The same `%` over a step ten times finer reads a decimal more, which is
    // the inference and not a fixed precision: 0.02 is 2%, not 0%.
    let fine = compose_to(0.1, "xTickFormat: '%'");
    assert_eq!(
        painted_x(&fine),
        strs(&["0%", "2%", "4%", "6%", "8%", "10%"])
    );
}

/// A format under `plotDefaults` reaches a plot that names no format of its
/// own, through the same resolver a format on the plot does.
#[test]
fn a_plot_defaults_tick_format_reaches_the_plot() {
    let composed = compose_to(2000.0, "plotDefaults:\n  xTickFormat: s");
    assert_eq!(
        painted_x(&composed),
        strs(&["0.0k", "0.5k", "1.0k", "1.5k", "2.0k"])
    );
}

// ---------------------------------------------------------------------------
// AC3 — the corpus's number formats draw with no warning
// ---------------------------------------------------------------------------

/// **Each of the four number formats the vendored corpus carries, `s`, `d`, `%`
/// and `+f`, draws with no warning**, on either axis, and draws something other
/// than the default text where the format changes it.
#[test]
fn each_number_format_the_corpus_carries_draws_with_no_warning() {
    for format in ["s", "d", "%", "+f"] {
        for key in ["xTickFormat", "yTickFormat"] {
            let composed = compose_to(100.0, &format!("{key}: '{format}'"));
            assert!(
                said(&composed).is_empty(),
                "`{key}: {format}` must draw with no warning; said {:?}",
                said(&composed)
            );
        }
    }
    // And each is read, not merely tolerated: `d` leaves whole numbers as they
    // are and `+f` signs them.
    let signed = compose_to(100.0, "xTickFormat: '+f'");
    assert_eq!(
        painted_x(&signed),
        strs(&["+0", "+20", "+40", "+60", "+80", "+100"])
    );
}

// ---------------------------------------------------------------------------
// AC4 — a format the reader cannot parse
// ---------------------------------------------------------------------------

/// **A format the reader cannot parse is named in the warning banner with its
/// key and value, and the axis draws its default text.** `xTickFormat: "~~"` is
/// the card's own example.
#[test]
fn a_format_the_reader_cannot_parse_is_named_and_the_axis_draws_its_default_text() {
    let composed = compose_to(100.0, "xTickFormat: \"~~\"");
    assert_eq!(
        painted_x(&composed),
        strs(&["0", "20", "40", "60", "80", "100"]),
        "the axis draws the text it draws with no format"
    );
    let lines = said(&composed);
    assert_eq!(lines.len(), 1, "one warning for one bad key: {lines:?}");
    assert!(
        lines[0].contains("xTickFormat") && lines[0].contains("~~"),
        "the banner names the key and the value: {}",
        lines[0]
    );

    // The other axis's format is still read: one bad key does not take the good
    // one down with it.
    let mixed = compose_to(2000.0, "xTickFormat: \"~~\"\nyTickFormat: s");
    assert_eq!(
        painted_y(&mixed),
        strs(&["2.0k", "1.5k", "1.0k", "0.5k", "0.0k"])
    );
    assert_eq!(said(&mixed).len(), 1, "{:?}", said(&mixed));
}
