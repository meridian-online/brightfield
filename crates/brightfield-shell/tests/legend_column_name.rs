//! **The legend at a plot's right names the column it is for, and a number
//! column's ramp runs from top to bottom with its values beside it.** A reader
//! of a coloured chart is told what the colour measures by the legend itself,
//! because a saved picture and the audience it is shown to do not see the shelf.
//!
//! The legend is drawn through `legend::draw_band` over a composed page, into a
//! headless egui context, and read back as the shapes it painted: the text a
//! reader would read and the strips of the ramp, each at the point it was placed.
//! So every fact here is a fact about where ink went, which is what a green
//! structural test about the spec would not say. The look of the whole page is
//! the baselines' (`legend_baseline.rs`).
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_shell::design::Mode;
use brightfield_shell::legend::{
    band_width, block_width, draw_band, draw_below_block, ramp_strip_colours, LegendSpec,
    RAMP_HEIGHT,
};
use brightfield_shell::pipeline::{compose_spec_str, Composed};
use brightfield_spec::layout::BELOW_LEGEND_HEIGHT;

/// The band's left edge in the headless context — well clear of the origin, so a
/// block drawn left of where it was told to go lands at a visibly different x.
const BAND_LEFT: f32 = 400.0;

/// Where the raster's top sits in the same coordinates.
const RASTER_TOP: f32 = 20.0;

/// Seven rows: a number column to colour by whose highest value is a whole
/// number, so its domain end spells the same way in the legend and here, and a
/// string column of three categories.
const VALUES: [f64; 7] = [1.5, 2.0, 3.5, 9.0, 4.0, 6.5, 5.0];

fn page(column: &str, fill: &str, attrs: &str) -> String {
    page_of(&VALUES, column, fill, attrs)
}

/// [`page`] over `values` in place of [`VALUES`].
fn page_of(values: &[f64], column: &str, fill: &str, attrs: &str) -> String {
    let rows: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            format!(
                "    - {{ x: {i}, y: {}, {column}: {v}, kind: k{} }}\n",
                i * 3,
                i % 3
            )
        })
        .collect();
    format!(
        "data:\n  t:\n{rows}plot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n    fill: {fill}\n  - legend: color\nwidth: 420\nheight: 300\n{attrs}"
    )
}

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

/// One thing the legend painted.
#[derive(Debug)]
enum Ink {
    /// Text, with the rect it fills.
    Text(String, egui::Rect),
    /// A solid rect, with its fill.
    Fill(egui::Rect, egui::Color32),
}

impl Ink {
    fn rect(&self) -> egui::Rect {
        match self {
            Self::Text(_, r) | Self::Fill(r, _) => *r,
        }
    }
}

/// What `draw_band` paints for `composed`, in paint order, with the band it was
/// given: the band starts at [`BAND_LEFT`] and is as wide as the page reserves.
fn painted(composed: &Composed) -> (egui::Rect, Vec<Ink>) {
    let band = egui::Rect::from_min_size(
        egui::pos2(BAND_LEFT, RASTER_TOP),
        egui::vec2(band_width(composed), 600.0),
    );
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        )),
        ..Default::default()
    };
    let out = ctx.run_ui(raw, |ui| {
        draw_band(ui, band, RASTER_TOP, composed, Mode::Light);
    });
    (band, ink_of(&out))
}

/// The text and the solid rects `out` painted, in paint order.
fn ink_of(out: &egui::FullOutput) -> Vec<Ink> {
    let mut ink = Vec::new();
    for clipped in &out.shapes {
        match &clipped.shape {
            // The glyphs laid out, not `Galley::text`, which is the whole text
            // before any of it is cut short.
            egui::Shape::Text(t) => ink.push(Ink::Text(
                t.galley.rows.iter().map(|r| r.row.text()).collect(),
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            egui::Shape::Rect(r) => ink.push(Ink::Fill(r.rect, r.fill)),
            _ => {}
        }
    }
    ink
}

fn texts(ink: &[Ink]) -> Vec<(&str, egui::Rect)> {
    ink.iter()
        .filter_map(|i| match i {
            Ink::Text(t, r) => Some((t.as_str(), *r)),
            Ink::Fill(..) => None,
        })
        .collect()
}

fn fills(ink: &[Ink]) -> Vec<(egui::Rect, egui::Color32)> {
    ink.iter()
        .filter_map(|i| match i {
            Ink::Fill(r, c) => Some((*r, *c)),
            Ink::Text(..) => None,
        })
        .collect()
}

/// A palette colour as the legend quantises it for the screen.
fn screen(c: [f32; 4]) -> egui::Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))
}

/// **AC1.** A number column on colour draws its name, then a ramp running top to
/// bottom with the domain's maximum at the top and its minimum at the foot, each
/// value beside its end of the ramp.
#[test]
fn a_number_column_draws_its_name_over_a_ramp_with_the_maximum_at_the_top() {
    let composed = compose(&page("reading", "reading", ""));
    assert_eq!(
        composed.plots[0].fill_column.as_deref(),
        Some("reading"),
        "the plot's handle names its fill column"
    );
    let Some(LegendSpec::Sequential { min, max, stops }) = LegendSpec::of_plot(&composed.plots[0])
    else {
        panic!("a number column on colour with the item draws a sequential legend");
    };
    assert_eq!(max.fract(), 0.0, "the page's maximum is a whole number");
    assert_eq!(min.fract(), 0.0, "the domain's minimum is a whole number");

    let (_, ink) = painted(&composed);
    let words = texts(&ink);
    let strips = fills(&ink);
    let spelled: Vec<&str> = words.iter().map(|(t, _)| *t).collect();
    assert_eq!(
        spelled,
        vec!["reading", &format!("{max}"), &format!("{min}")],
        "the name, then the maximum, then the minimum"
    );
    let (name, high, low) = (words[0].1, words[1].1, words[2].1);

    // The ramp is the run of strips; the one at the top wears the high end.
    assert!(!strips.is_empty(), "the ramp painted no strip");
    let top = strips.iter().map(|(r, _)| r.min.y).fold(f32::MAX, f32::min);
    let foot = strips.iter().map(|(r, _)| r.max.y).fold(f32::MIN, f32::max);
    assert_eq!(foot - top, RAMP_HEIGHT, "the ramp's height");
    assert!(
        name.max.y <= top,
        "the name {name:?} is not over the ramp, which starts at {top}"
    );
    let colour_at = |y: f32| {
        strips
            .iter()
            .find(|(r, _)| r.min.y == y)
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("no strip starts at {y}"))
    };
    assert_eq!(
        colour_at(top),
        screen(*stops.last().expect("stops")),
        "the strip at the top wears the ramp's high end"
    );
    let last_start = strips.iter().map(|(r, _)| r.min.y).fold(f32::MIN, f32::max);
    assert_eq!(
        colour_at(last_start),
        screen(stops[0]),
        "the strip at the foot wears the ramp's low end"
    );

    // Each value is beside its end, level with it, and to the right of the ramp.
    let ramp_right = strips.iter().map(|(r, _)| r.max.x).fold(f32::MIN, f32::max);
    for (what, rect) in [("maximum", high), ("minimum", low)] {
        assert!(
            rect.min.x >= ramp_right,
            "the {what} {rect:?} is not beside the ramp, which ends at {ramp_right}"
        );
    }
    assert_eq!(high.min.y, top, "the maximum is level with the ramp's top");
    assert_eq!(low.max.y, foot, "the minimum is level with the ramp's foot");
    assert!(high.min.y < low.min.y, "the maximum is under the minimum");
}

/// **AC1, diverging.** A diverging ramp is a number column's too: the maximum at
/// the top, the minimum at the foot and the pivot level with the middle.
#[test]
fn a_diverging_ramp_runs_from_top_to_bottom_with_the_pivot_at_its_middle() {
    let composed = compose(&page(
        "reading",
        "reading",
        "colorScale: diverging\ncolorPivot: 4\n",
    ));
    let Some(LegendSpec::Diverging {
        min, max, pivot, ..
    }) = LegendSpec::of_plot(&composed.plots[0])
    else {
        panic!("a diverging fill with the item draws a diverging legend");
    };
    let (_, ink) = painted(&composed);
    let words = texts(&ink);
    let strips = fills(&ink);
    let top = strips.iter().map(|(r, _)| r.min.y).fold(f32::MAX, f32::min);
    let foot = strips.iter().map(|(r, _)| r.max.y).fold(f32::MIN, f32::max);

    let spelled: Vec<&str> = words.iter().map(|(t, _)| *t).collect();
    let value = |v: f64| {
        if v.fract() == 0.0 {
            format!("{v:.0}")
        } else {
            format!("{v:.2}")
        }
    };
    assert_eq!(
        spelled,
        vec!["reading", &value(max), &value(pivot), &value(min)],
        "the name, then the maximum, the pivot and the minimum from the top down"
    );
    assert_eq!(words[1].1.min.y, top, "the maximum is level with the top");
    assert_eq!(words[3].1.max.y, foot, "the minimum is level with the foot");
    let middle = (top + foot) / 2.0;
    let pivot_mid = words[2].1.center().y;
    assert!(
        (pivot_mid - middle).abs() <= 0.5,
        "the pivot's label is centred at {pivot_mid}, the ramp's middle is {middle}"
    );
}

/// **AC2.** A string column on colour draws its name, then one swatch row per
/// category, each swatch beside its label, in the scale's order.
#[test]
fn a_string_column_draws_its_name_over_one_swatch_row_per_category() {
    let composed = compose(&page("reading", "kind", ""));
    assert_eq!(composed.plots[0].fill_column.as_deref(), Some("kind"));
    let Some(LegendSpec::Categorical { entries }) = LegendSpec::of_plot(&composed.plots[0]) else {
        panic!("a string column on colour with the item draws a categorical legend");
    };
    assert_eq!(entries.len(), 3, "three categories");

    let (_, ink) = painted(&composed);
    let words = texts(&ink);
    let swatches = fills(&ink);
    let spelled: Vec<&str> = words.iter().map(|(t, _)| *t).collect();
    let mut want = vec!["kind"];
    want.extend(entries.iter().map(|e| e.label.as_str()));
    assert_eq!(spelled, want, "the name, then one label per category");

    assert_eq!(swatches.len(), entries.len(), "one swatch per category");
    for (i, ((rect, colour), entry)) in swatches.iter().zip(&entries).enumerate() {
        assert_eq!(*colour, screen(entry.colour), "swatch {i}'s ink");
        let label = words[i + 1].1;
        assert!(
            label.min.x >= rect.max.x,
            "label {i} {label:?} is not beside its swatch {rect:?}"
        );
        assert!(
            (label.center().y - rect.center().y).abs() <= 0.5,
            "label {i} {label:?} is not level with its swatch {rect:?}"
        );
        if i > 0 {
            assert!(
                rect.min.y > swatches[i - 1].0.min.y,
                "swatch {i} is not below the last"
            );
        }
    }
    assert!(
        words[0].1.max.y <= swatches[0].0.min.y,
        "the name {:?} is not over the first swatch {:?}",
        words[0].1,
        swatches[0].0
    );
}

/// **AC3.** A name longer than the legend column is cut short inside it — the
/// text ends in an ellipsis and its right edge is inside the block — and the band
/// is as wide as it is for a short name, so the plot's width is unchanged.
#[test]
fn a_long_name_is_cut_short_inside_the_column_and_the_band_stays_as_wide() {
    let long = "median_house_value_over_the_whole_of_the_surveyed_period_in_current_dollars";
    let short = compose(&page("reading", "reading", ""));
    let composed = compose(&page(long, long, ""));
    assert_eq!(composed.plots[0].fill_column.as_deref(), Some(long));
    assert_eq!(
        band_width(&composed),
        band_width(&short),
        "a long name widened the band"
    );

    let (band, ink) = painted(&composed);
    let words = texts(&ink);
    let (cut, rect) = words[0];
    assert!(
        cut.ends_with('\u{2026}') && cut.len() < long.len(),
        "the name was not cut short: {cut:?}"
    );
    assert!(
        rect.max.x <= BAND_LEFT + block_width(),
        "the name {rect:?} runs past the block, which ends at {}",
        BAND_LEFT + block_width()
    );
    assert!(band.max.x >= rect.max.x, "the name leaves the band");
}

/// Whole numbers of sixteen digits, each below 2^53 so that an `f64` holds it
/// exactly, at the ends of the domain: spelled out they outrun any room a legend
/// gives a label.
const SIXTEEN_DIGITS: [f64; 4] = [
    1_234_567_890_123_456.0,
    5_000_000_000_000_000.0,
    7_000_000_000_000_000.0,
    8_765_432_109_876_543.0,
];

/// The scales a sixteen-digit domain is drawn on, with how many of the legend's
/// values are then of sixteen digits: a sequential scale starts at zero, so only
/// its maximum is; a diverging one about a pivot of sixteen digits has all three.
const SIXTEEN_DIGIT_SCALES: [(&str, usize); 2] = [
    ("", 1),
    ("colorScale: diverging\ncolorPivot: 5000000000000000\n", 3),
];

/// The values `legend` labels, in the order a ramp is read from its top: the
/// maximum, the pivot of a diverging scale, the minimum.
fn values_top_down(legend: &LegendSpec) -> Vec<f64> {
    match legend {
        LegendSpec::Sequential { min, max, .. } => vec![*max, *min],
        LegendSpec::Diverging {
            min, max, pivot, ..
        } => vec![*max, *pivot, *min],
        LegendSpec::Categorical { .. } => panic!("a number fill draws a ramp"),
    }
}

/// The value as the legend spells it: a whole number to no decimals.
fn spelled(value: f64) -> String {
    format!("{value:.0}")
}

/// **AC1.** A domain end of sixteen digits at a plot's right is cut short inside
/// the label column: its text ends in an ellipsis and its right edge is inside
/// the column, which ends where the block does. A value of one digit beside it
/// is spelled whole.
#[test]
fn a_domain_end_of_sixteen_digits_is_cut_short_inside_the_label_column() {
    for (attrs, long) in SIXTEEN_DIGIT_SCALES {
        let composed = compose(&page_of(&SIXTEEN_DIGITS, "reading", "reading", attrs));
        let legend = LegendSpec::of_plot(&composed.plots[0]).expect("a number fill draws a legend");
        let values = values_top_down(&legend);
        let (_, ink) = painted(&composed);
        let words = texts(&ink);
        assert_eq!(
            words.len(),
            1 + values.len(),
            "{attrs:?}: the name, then each value: {words:?}"
        );
        let mut cut_short = 0;
        for ((text, rect), value) in words[1..].iter().zip(&values) {
            let whole = spelled(*value);
            if whole.chars().count() < 16 {
                assert_eq!(*text, whole, "{attrs:?}: a short value was changed");
                continue;
            }
            cut_short += 1;
            assert!(
                text.ends_with('\u{2026}') && text.chars().count() < whole.chars().count(),
                "{attrs:?}: {whole:?} was not cut short: {text:?}"
            );
            assert!(
                rect.max.x <= BAND_LEFT + block_width(),
                "{attrs:?}: {text:?} {rect:?} runs past the label column, which ends at {}",
                BAND_LEFT + block_width()
            );
        }
        assert_eq!(cut_short, long, "{attrs:?}: values of sixteen digits");
    }
}

/// How wide the band under a plot is for the next test: narrow enough that the
/// ramp is the least it is drawn, so sixteen digits are wider than the ramp.
const BELOW_BAND_WIDTH: f32 = 190.0;

/// **AC1.** A domain end of sixteen digits under a plot is cut short inside the
/// ramp's width: its text ends in an ellipsis and lies between the ramp's two
/// ends, so none hangs past the end it names.
#[test]
fn a_domain_end_of_sixteen_digits_under_a_plot_is_cut_short_inside_the_ramp() {
    for (attrs, long) in SIXTEEN_DIGIT_SCALES {
        let composed = compose(&page_of(&SIXTEEN_DIGITS, "reading", "reading", attrs));
        let legend = LegendSpec::of_plot(&composed.plots[0]).expect("a number fill draws a legend");
        // Read from the left, as the band draws them.
        let mut values = values_top_down(&legend);
        values.reverse();
        let band = egui::Rect::from_min_size(
            egui::pos2(BAND_LEFT, RASTER_TOP),
            egui::vec2(BELOW_BAND_WIDTH, BELOW_LEGEND_HEIGHT as f32),
        );
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 700.0),
            )),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            draw_below_block(
                &ui.painter_at(band),
                band,
                &legend,
                composed.plots[0].fill_column.as_deref(),
                Mode::Light,
            );
        });
        let ink = ink_of(&out);
        let words = texts(&ink);
        let strips = fills(&ink);
        assert!(!strips.is_empty(), "{attrs:?}: the ramp painted no strip");
        let left = strips.iter().map(|(r, _)| r.min.x).fold(f32::MAX, f32::min);
        let right = strips.iter().map(|(r, _)| r.max.x).fold(f32::MIN, f32::max);
        assert_eq!(
            words.len(),
            1 + values.len(),
            "{attrs:?}: the name, then each value: {words:?}"
        );
        let mut cut_short = 0;
        for ((text, rect), value) in words[1..].iter().zip(&values) {
            let whole = spelled(*value);
            if whole.chars().count() < 16 {
                assert_eq!(*text, whole, "{attrs:?}: a short value was changed");
                continue;
            }
            cut_short += 1;
            assert!(
                text.ends_with('\u{2026}') && text.chars().count() < whole.chars().count(),
                "{attrs:?}: {whole:?} was not cut short: {text:?}"
            );
            assert!(
                rect.min.x >= left - 0.01 && rect.max.x <= right + 0.01,
                "{attrs:?}: {text:?} {rect:?} is outside the ramp, which spans {left} to {right}"
            );
        }
        assert_eq!(cut_short, long, "{attrs:?}: values of sixteen digits");
    }
}

/// **AC4.** No part of the legend is inside the plot's data area: everything the
/// legend paints, of either kind of column, lies inside the band, which starts at
/// or after the raster's right edge.
#[test]
fn nothing_the_legend_paints_is_outside_its_band() {
    for fill in ["reading", "kind"] {
        let composed = compose(&page("reading", fill, ""));
        let (band, ink) = painted(&composed);
        assert!(!ink.is_empty(), "{fill}: the legend painted nothing");
        for painted in &ink {
            let r = painted.rect();
            assert!(
                r.min.x >= band.min.x && r.max.x <= band.max.x,
                "{fill}: {painted:?} is outside the band {band:?}"
            );
        }
    }
}

/// A plot whose fill is not a column has no fill column, and no legend to name
/// one for.
#[test]
fn a_plot_with_no_fill_column_names_none() {
    let source = page("reading", "steelblue", "");
    let composed = compose(&source);
    assert_eq!(composed.plots[0].fill_column, None);
    assert_eq!(LegendSpec::of_plot(&composed.plots[0]), None);
}

/// The strips the ramp is drawn in run low end first and the drawing reverses
/// them, which this holds for the one function both legends read.
#[test]
fn the_strips_run_from_the_low_end_to_the_high_end() {
    let stops = [[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]];
    let strips = ramp_strip_colours(&stops);
    assert_eq!(strips[0], stops[0]);
    assert_eq!(strips[strips.len() - 1], stops[1]);
}
