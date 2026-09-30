//! **A spec that sets `xTickFormat` or `yTickFormat` to a date format draws the
//! tick text it asked for on a date axis.**
//!
//! Two of Mosaic's own examples set `xTickFormat: '%b'`, and no code read it: a
//! date axis drew brightfield's own text whatever the spec said, so a chart brought
//! from Mosaic's gallery read `2024-03-01` where the chart file asked for `Mar`.
//!
//! Two kinds of axis are a date axis here. A `TIMESTAMP` column takes a time scale,
//! one tick per step. A `DATE` column takes a band, one tick per day, each spelled
//! `YYYY-MM-DD` until a format says otherwise. Both are tested, since a format read
//! on one and dropped on the other is the defect.
//!
//! Assertions read the TEXT that was PAINTED, decoded from the glyph runs on
//! `Composed::scene`, which is what a viewer is shown, as
//! `axis_vocabulary_number_format` does and for the same reason: a format resolved
//! and dropped before the draw would pass a check that read the resolver.
//!
//! An asked-for arm is paired with the same spec asking for nothing, and with the
//! other axis left alone.

use brightfield_render::text::{draw_text, TextAnchor, LABEL_SIZE};
use brightfield_shell::pipeline::{Composed, LiveDashboard};

// ---------------------------------------------------------------------------
// The fixtures
// ---------------------------------------------------------------------------

/// Three days a month apart, on a `DATE` column. `ATTRS` marks where the plot
/// attributes go.
const DAYS: &str = r#"
data:
  days:
    query: "SELECT DATE '2024-03-01' AS day, 1 AS n UNION ALL SELECT DATE '2024-04-01', 2 UNION ALL SELECT DATE '2024-05-01', 3 ORDER BY day"
plot:
  - mark: dot
    data: { from: days }
    x: day
    y: n
width: 600
height: 300
ATTRS
"#;

/// Eleven instants a minute apart from two in the afternoon, on a `TIMESTAMP`
/// column, so the axis runs from 14:00 to 14:10 and a tick falls at 14:05.
const CLOCK: &str = r#"
data:
  clock:
    query: "SELECT TIMESTAMP '2024-03-01 14:00:00' + to_minutes(CAST(i AS INTEGER)) AS at, i AS n FROM range(11) t(i)"
plot:
  - mark: dot
    data: { from: clock }
    x: at
    y: n
width: 600
height: 300
ATTRS
"#;

/// Two numeric columns, so the axes are number axes.
const NUMBERS: &str = r"
data:
  pts:
    - { a: 0,   b: 0 }
    - { a: 25,  b: 60 }
    - { a: 70,  b: 30 }
    - { a: 100, b: 100 }
plot:
  - mark: dot
    data: { from: pts }
    x: a
    y: b
width: 600
height: 300
ATTRS
";

fn compose(template: &str, attrs: &str) -> Composed {
    LiveDashboard::load_str(&template.replace("ATTRS", attrs), None)
        .expect("the spec loads")
        .present()
        .expect("the spec composes")
}

fn days(attrs: &str) -> Composed {
    compose(DAYS, attrs)
}

fn clock(attrs: &str) -> Composed {
    compose(CLOCK, attrs)
}

fn numbers(attrs: &str) -> Composed {
    compose(NUMBERS, attrs)
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

/// Every character a date tick can paint in these fixtures.
const PAINTABLE: &str = "0123456789.,:+- sABCDFGJMNOSTWabcdefghijklmnoprstuvyz";

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

/// A tick label sits `LABEL_SIZE / 3` below its tick on the y axis and
/// `TICK_LENGTH + LABEL_SIZE` below the axis line on the x axis (both private to
/// `axis.rs`), so the plot's bottom edge plus ten separates the two rows.
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

/// The text a date column's axis draws with no format: each day spelled out.
fn default_days() -> Vec<String> {
    strs(&["2024-03-01", "2024-04-01", "2024-05-01"])
}

/// The text a timestamp axis draws with no format: each tick's seconds since the
/// epoch. 14:00 on 2024-03-01 is 1 709 301 600 seconds and the ticks are 100
/// seconds apart, but a label is thirteen characters wide and seven of them do not
/// fit the 600 pixels, so the axis draws every other one: 14:00:00, 14:03:20,
/// 14:06:40 and 14:10:00. (Under a format the labels are short and all seven fit.)
fn default_clock() -> Vec<String> {
    strs(&[
        "1709301600.0s",
        "1709301800.0s",
        "1709302000.0s",
        "1709302200.0s",
    ])
}

// ---------------------------------------------------------------------------
// AC1 — a date axis
// ---------------------------------------------------------------------------

/// **On a date axis, `%b` prints the tick at 2024-03-01 as `Mar`, `%Y-%m-%d`
/// prints it as `2024-03-01` and `%B %Y` prints it as `March 2024`.** The other
/// two ticks, a month apart, print as the same format prints them, and the y
/// axis, which names no format, is left as it was.
#[test]
fn a_date_format_prints_each_day_of_a_date_axis_as_d3_time_format_does() {
    let asked = days("xTickFormat: '%b'");
    assert_eq!(painted_x(&asked), strs(&["Mar", "Apr", "May"]));
    assert_eq!(painted_y(&asked), painted_y(&days("")));
    assert!(said(&asked).is_empty(), "{:?}", said(&asked));

    assert_eq!(
        painted_x(&days("xTickFormat: '%Y-%m-%d'")),
        default_days(),
        "`%Y-%m-%d` is the day's own spelling"
    );
    assert_eq!(
        painted_x(&days("xTickFormat: '%B %Y'")),
        strs(&["March 2024", "April 2024", "May 2024"])
    );
    assert_eq!(
        painted_x(&days("xTickFormat: '%-d %b'")),
        strs(&["1 Mar", "1 Apr", "1 May"]),
        "a padding modifier reaches the paint"
    );
}

/// The two keys are separate: `yTickFormat` on a date column that sits on the y
/// axis formats that axis, and leaves x alone.
#[test]
fn a_date_format_follows_the_key_of_the_axis_the_dates_are_on() {
    let spec = DAYS.replace("    x: day\n    y: n\n", "    x: n\n    y: day\n");
    let composed = compose(&spec, "yTickFormat: '%b'");
    let mut y = painted_y(&composed);
    y.sort();
    assert_eq!(y, strs(&["Apr", "Mar", "May"]));
    assert_eq!(painted_x(&composed), painted_x(&compose(&spec, "")));
    assert!(said(&composed).is_empty(), "{:?}", said(&composed));
}

// ---------------------------------------------------------------------------
// AC2 — a timestamp axis
// ---------------------------------------------------------------------------

/// **On a timestamp axis, `%H:%M` prints the tick at five past two in the
/// afternoon as `14:05`.** The ticks step every 100 seconds, from 14:00:00, so the
/// text is those instants read in UTC and the tick at 14:05:00 is among them.
#[test]
fn a_date_format_prints_each_tick_of_a_timestamp_axis_in_utc() {
    let asked = clock("xTickFormat: '%H:%M'");
    assert_eq!(
        painted_x(&asked),
        strs(&["14:00", "14:01", "14:03", "14:05", "14:06", "14:08", "14:10"]),
        "14:00:00, 14:01:40, 14:03:20, 14:05:00, 14:06:40, 14:08:20 and 14:10:00"
    );
    assert!(said(&asked).is_empty(), "{:?}", said(&asked));
    assert_eq!(painted_y(&asked), painted_y(&clock("")));

    assert_eq!(
        painted_x(&clock("xTickFormat: '%H:%M:%S'"))[1],
        "14:01:40",
        "the seconds are in the instant the tick stands at"
    );
}

// ---------------------------------------------------------------------------
// AC3 — what the reader does not handle
// ---------------------------------------------------------------------------

/// **A directive the reader does not handle is named in the warning banner with
/// its key and value, and the axis draws its default text.** On a date column and
/// on a timestamp, for `%K` alone and inside a format that is otherwise readable.
#[test]
fn a_directive_the_reader_does_not_handle_is_named_and_the_axis_draws_its_default_text() {
    for (asked, key, value, directive) in [
        (days("xTickFormat: '%K'"), "xTickFormat", "%K", "%K"),
        (
            days("xTickFormat: '%B %-K'"),
            "xTickFormat",
            "%B %-K",
            "%-K",
        ),
    ] {
        assert_eq!(painted_x(&asked), default_days());
        let lines = said(&asked);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains(key) && lines[0].contains(value) && lines[0].contains(directive),
            "the banner names the key, the value and the directive: {}",
            lines[0]
        );
    }

    let on_clock = clock("xTickFormat: '%H:%M %F'");
    assert_eq!(painted_x(&on_clock), default_clock());
    let lines = said(&on_clock);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("xTickFormat")
            && lines[0].contains("%H:%M %F")
            && lines[0].contains("%F"),
        "{}",
        lines[0]
    );
}

/// **A format of the other kind takes the same path.** A number format on a date
/// axis and a date format on a number axis each draw the axis's default text and
/// are named in the banner with the key and the value, on either kind of date axis
/// and on either axis of a number plot.
#[test]
fn a_format_of_the_other_kind_is_named_and_the_axis_draws_its_default_text() {
    let names = |composed: &Composed, key: &str, value: &str, format: &str, axis: &str| {
        let lines = said(composed);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains(key)
                && lines[0].contains(&format!("`{value}`"))
                && lines[0].contains(&format!("a {format} format on a {axis} axis")),
            "{}",
            lines[0]
        );
    };

    let on_days = days("xTickFormat: s");
    assert_eq!(painted_x(&on_days), default_days());
    names(&on_days, "xTickFormat", "s", "number", "date");

    let on_clock = clock("xTickFormat: '.2f'");
    assert_eq!(painted_x(&on_clock), default_clock());
    names(&on_clock, "xTickFormat", ".2f", "number", "date");

    let on_x = numbers("xTickFormat: '%b'");
    assert_eq!(
        painted_x(&on_x),
        painted_x(&numbers("")),
        "a date format on a number axis draws the default text"
    );
    names(&on_x, "xTickFormat", "%b", "date", "number");

    let on_y = numbers("yTickFormat: '%Y-%m-%d'");
    assert_eq!(painted_y(&on_y), painted_y(&numbers("")));
    names(&on_y, "yTickFormat", "%Y-%m-%d", "date", "number");

    // A format of the right kind on the same plot says nothing.
    assert!(said(&numbers("xTickFormat: s")).is_empty());
}

/// What the banner says survives a repaint once and is not said twice: a
/// re-present rebuilds the composition, and the load's diagnostics are attached
/// to each, so a line found by the composition must neither be lost nor doubled.
#[test]
fn a_repaint_says_a_crossed_format_once() {
    let mut live = LiveDashboard::load_str(&DAYS.replace("ATTRS", "xTickFormat: s"), None)
        .expect("the spec loads");
    let first = live.present().expect("composes");
    let second = live.present().expect("composes again");
    assert_eq!(said(&first).len(), 1, "{:?}", said(&first));
    assert_eq!(said(&second), said(&first));
}

// ---------------------------------------------------------------------------
// AC4 — no format
// ---------------------------------------------------------------------------

/// **A date axis whose spec sets no format draws the text it drew before a date
/// format could be asked for**, on both kinds of date axis: each day spelled out,
/// and each tick's seconds since the epoch. The load says nothing.
#[test]
fn a_date_axis_with_no_format_draws_the_text_it_always_drew() {
    let on_days = days("");
    assert_eq!(painted_x(&on_days), default_days());
    assert!(said(&on_days).is_empty(), "{:?}", said(&on_days));

    let on_clock = clock("");
    assert_eq!(painted_x(&on_clock), default_clock());
    assert!(said(&on_clock).is_empty(), "{:?}", said(&on_clock));

    // `null` is Mosaic's "no format", and a date format does not change the
    // text of an axis it is not asked of.
    assert_eq!(painted_x(&days("xTickFormat: null")), default_days());
    assert_eq!(painted_x(&days("yTickFormat: '%b'"))[0], "2024-03-01");
}

/// A date format under `plotDefaults` reaches a plot that names no format of its
/// own, through the same resolver a format on the plot does.
#[test]
fn a_plot_defaults_date_format_reaches_the_plot() {
    let composed = days("plotDefaults:\n  xTickFormat: '%b'");
    assert_eq!(painted_x(&composed), strs(&["Mar", "Apr", "May"]));
    assert!(said(&composed).is_empty(), "{:?}", said(&composed));
}
