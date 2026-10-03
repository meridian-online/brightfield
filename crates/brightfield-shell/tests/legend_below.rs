//! **A colour legend under the plot it is for, in a `vconcat`, is drawn in a band
//! below that plot — a number column's ramp running left to right with the
//! column's name at its left and the domain's two ends under it, a string
//! column's swatches in a row — and is not also drawn at the plot's right.** The
//! band is carved from the pane's height: the plot takes the height above it,
//! the window the shell asks for is the one it asks for with no legend, and no
//! part of the legend is inside a plot's data area.
//!
//! The layout half (the band's rect, the plot taking the remainder) is
//! `crates/brightfield-spec/tests/legend_below.rs`. What is held here is the
//! shell's: what is painted into the band, read back off the shapes the painter
//! received, and where the pane puts the band, read off a headless layout. The
//! look is `legend_below_baseline.rs`'s.
//!
//! The specs carry their data inline, so the run needs no file beside it.

use brightfield_shell::design::Mode;
use brightfield_shell::legend::{
    band_width, below_blocks, below_overhang, blocks, draw_below, ramp_strip_colours, LegendSpec,
    BELOW_RAMP_HEIGHT, BELOW_RAMP_MAX_WIDTH,
};
use brightfield_shell::pipeline::{compose_spec_str, Composed, LiveDashboard};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{chart_window_size, Boot, MeridianApp};
use brightfield_spec::layout::{Rect, BELOW_LEGEND_HEIGHT};

/// The window every layout here is settled in.
const WINDOW: (f32, f32) = (1400.0, 900.0);

/// Where the raster's top-left is when a page is painted by itself.
const ORIGIN: (f32, f32) = (10.0, 20.0);

const VALUES: [f64; 5] = [0.825, 2.291, 1.5, 4.9, 3.2];

fn data() -> String {
    let rows: String = VALUES
        .iter()
        .enumerate()
        .map(|(i, v)| format!("    - {{ x: {i}, y: {}, v: {v}, g: g{} }}\n", i * 3, i % 2))
        .collect();
    format!("data:\n  t:\n{rows}")
}

/// A dot over `t` filled by `fill`, as a plot item at `indent` spaces.
fn dot(fill: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    format!(
        "{pad}- mark: dot\n{pad}  data: {{ from: t }}\n{pad}  x: x\n{pad}  y: y\n{pad}  fill: {fill}\n"
    )
}

/// A plot named `name` as a `vconcat` item, with `extra` plot items after its dot.
fn plot_item(fill: &str, name: &str, extra: &str) -> String {
    format!(
        "  - plot:\n{}{extra}    name: {name}\n    width: 420\n    height: 300\n",
        dot(fill, 6)
    )
}

/// A `vconcat` of a plot named `scatter` filled by `fill` and a colour legend
/// `for` it.
fn below(fill: &str) -> String {
    format!(
        "{}vconcat:\n{}  - legend: color\n    for: scatter\n",
        data(),
        plot_item(fill, "scatter", "")
    )
}

/// The same plot with no legend at all, as the page's one plot.
fn alone(fill: &str) -> String {
    format!("{}plot:\n{}width: 420\nheight: 300\n", data(), dot(fill, 2))
}

fn compose(source: &str) -> Composed {
    compose_spec_str(source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

/// `source` settled in [`WINDOW`] over a live document, so the pane re-lays the
/// chart into the room it has left.
fn laid_out(source: &str) -> MeridianApp {
    let mut live = LiveDashboard::load_str(source, None).expect("loads live");
    let composed = live.present().expect("first paint");
    let boot = Boot {
        live: Some(live),
        ..Boot::charts(composed)
    };
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(WINDOW.0, WINDOW.1),
        )),
        ..Default::default()
    };
    for _ in 0..4 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    app
}

/// One thing `draw_below` paints: the glyphs laid out and the rect they take, or
/// a solid rect with its fill.
#[derive(Debug, Clone)]
enum Ink {
    Text(String, egui::Rect),
    Fill(egui::Rect, egui::Color32),
}

/// What `draw_below` paints for `composed`, in paint order, with the raster
/// standing at [`ORIGIN`].
fn painted(composed: &Composed) -> Vec<Ink> {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        )),
        ..Default::default()
    };
    let out = ctx.run_ui(raw, |ui| {
        draw_below(ui, egui::pos2(ORIGIN.0, ORIGIN.1), composed, Mode::Light);
    });
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

/// The palette colour as the painter quantises it.
fn ink_of(c: [f32; 4]) -> egui::Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))
}

/// A domain end as the legend spells it: integers bare, fractions to two places.
fn spelled(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

fn texts(ink: &[Ink]) -> Vec<(&str, egui::Rect)> {
    ink.iter()
        .filter_map(|i| match i {
            Ink::Text(t, r) => Some((t.as_str(), *r)),
            Ink::Fill(..) => None,
        })
        .collect()
}

fn text_of(ink: &[Ink], text: &str) -> egui::Rect {
    texts(ink)
        .into_iter()
        .find(|(t, _)| *t == text)
        .map(|(_, r)| r)
        .unwrap_or_else(|| panic!("no text {text:?} painted: {:?}", texts(ink)))
}

/// The band on the window plane: the layout's rect moved to where the raster stands.
fn in_window(band: Rect, raster: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        raster.min + egui::vec2(band.x as f32, band.y as f32),
        egui::vec2(band.width as f32, band.height as f32),
    )
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// **AC1, a number column.** The ramp runs left to right — the low end's colour
/// at its left — at most [`BELOW_RAMP_MAX_WIDTH`] wide, with the column's name at
/// its left and the domain's two ends under it, the low one under the ramp's
/// left end and the high one under its right end. All of it inside the band.
#[test]
fn a_number_column_draws_a_ramp_running_left_to_right_with_its_ends_under_it() {
    let composed = compose(&below("v"));
    let blocks = below_blocks(&composed);
    let [(0, LegendSpec::Sequential { min, max, stops }, band)] = &blocks[..] else {
        panic!("the plot under its legend drew {blocks:?}");
    };
    let band = in_window(
        *band,
        egui::Rect::from_min_size(egui::pos2(ORIGIN.0, ORIGIN.1), egui::Vec2::ZERO),
    );
    let ink = painted(&composed);

    let strips: Vec<(egui::Rect, egui::Color32)> = ink
        .iter()
        .filter_map(|i| match i {
            Ink::Fill(r, c) if r.height() == BELOW_RAMP_HEIGHT => Some((*r, *c)),
            _ => None,
        })
        .collect();
    let expected = ramp_strip_colours(stops);
    assert_eq!(strips.len(), expected.len(), "one strip per sampled colour");
    let ramp = strips.iter().fold(strips[0].0, |all, (r, _)| all.union(*r));

    // Left to right, low end first.
    for (i, ((rect, colour), want)) in strips.iter().zip(&expected).enumerate() {
        assert_eq!(
            *colour,
            ink_of(*want),
            "strip {i} is not the ramp's colour {i}"
        );
        if i > 0 {
            assert!(
                rect.min.x >= strips[i - 1].0.min.x + 1.0,
                "strip {i} does not stand right of strip {}",
                i - 1
            );
        }
    }
    assert!(
        ramp.width() <= BELOW_RAMP_MAX_WIDTH,
        "the ramp is {} wide, more than {BELOW_RAMP_MAX_WIDTH}",
        ramp.width()
    );
    assert!(
        ramp.width() > expected.len() as f32,
        "the ramp is {} wide, narrower than a point a strip",
        ramp.width()
    );
    assert!(
        band.contains_rect(ramp),
        "the ramp {ramp:?} leaves the band {band:?}"
    );

    // The name at its left, level with the ramp.
    let name = text_of(&ink, "v");
    assert!(
        name.max.x <= ramp.min.x,
        "the name {name:?} is not at the ramp's left {ramp:?}"
    );
    assert!(
        name.center().y >= ramp.top() && name.center().y <= ramp.bottom(),
        "the name {name:?} is not level with the ramp {ramp:?}"
    );

    // The ends under it, each under the end it names.
    let low = text_of(&ink, &spelled(*min));
    let high = text_of(&ink, &spelled(*max));
    for (what, rect) in [("low", low), ("high", high)] {
        assert!(
            rect.min.y >= ramp.bottom(),
            "the {what} end {rect:?} is not under the ramp {ramp:?}"
        );
        assert!(
            band.contains_rect(rect),
            "the {what} end {rect:?} leaves the band {band:?}"
        );
    }
    assert!(
        (low.min.x - ramp.min.x).abs() <= 0.5,
        "the low end {low:?} is not under the ramp's left end {ramp:?}"
    );
    assert!(
        (high.max.x - ramp.max.x).abs() <= 0.5,
        "the high end {high:?} is not under the ramp's right end {ramp:?}"
    );
}

/// **AC1, a string column.** The swatches run in a row — one level, in the scale's
/// own order, each with its label at its right — after the column's name.
#[test]
fn a_string_column_draws_its_swatches_in_a_row() {
    let composed = compose(&below("g"));
    let blocks = below_blocks(&composed);
    let [(0, LegendSpec::Categorical { entries }, _)] = &blocks[..] else {
        panic!("the plot under its legend drew {blocks:?}");
    };
    let ink = painted(&composed);

    let swatches: Vec<egui::Rect> = entries
        .iter()
        .map(|e| {
            ink.iter()
                .find_map(|i| match i {
                    Ink::Fill(r, c) if *c == ink_of(e.colour) && r.width() == r.height() => {
                        Some(*r)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no swatch painted for {e:?}"))
        })
        .collect();
    assert_eq!(swatches.len(), 2, "two categories, two swatches");
    assert!(
        (swatches[0].center().y - swatches[1].center().y).abs() < 0.01,
        "the swatches {swatches:?} are not in one row"
    );
    assert!(
        swatches[0].max.x < swatches[1].min.x,
        "the swatches {swatches:?} are not in the scale's order, left to right"
    );

    for (entry, swatch) in entries.iter().zip(&swatches) {
        let label = text_of(&ink, &entry.label);
        assert!(
            label.min.x >= swatch.max.x,
            "{:?} is not at the right of its swatch {swatch:?}",
            entry.label
        );
        assert!(
            (label.center().y - swatch.center().y).abs() <= 1.0,
            "{:?} is not level with its swatch",
            entry.label
        );
    }
    let name = text_of(&ink, "g");
    assert!(
        name.max.x <= swatches[0].min.x,
        "the name {name:?} is not at the left of the swatches {swatches:?}"
    );
}

/// **AC2.** In that file no legend is drawn at the plot's right and no band is
/// reserved there: the plot is as wide as the same plot with no legend.
#[test]
fn nothing_is_drawn_or_reserved_at_the_right_of_a_plot_whose_legend_is_below() {
    for fill in ["v", "g"] {
        let composed = compose(&below(fill));
        assert!(
            blocks(&composed).is_empty(),
            "{fill}: the plot drew a legend at its right: {:?}",
            blocks(&composed)
        );
        assert_eq!(
            band_width(&composed),
            0.0,
            "{fill}: a band is reserved at the right"
        );

        let with = laid_out(&below(fill));
        let without = laid_out(&alone(fill));
        assert_eq!(
            with.chart_doc().legend_rect,
            None,
            "{fill}: the pane recorded a band at the right"
        );
        let (r_with, r_without) = (
            with.chart_doc().raster_rect.expect("a raster"),
            without.chart_doc().raster_rect.expect("a raster"),
        );
        assert!(
            (r_with.width() - r_without.width()).abs() <= 1.0,
            "{fill}: the plot is {} wide, the same plot with no legend {}",
            r_with.width(),
            r_without.width()
        );
    }
}

/// **AC3.** The band is [`BELOW_LEGEND_HEIGHT`] high and carved from the pane's
/// height: the plot takes the height above it, the band stands directly under the
/// raster and inside the pane, the window the shell asks for is the one it asks
/// for with no legend, and no part of the band is inside a plot's data area.
#[test]
fn the_band_is_carved_from_the_panes_height_and_the_window_is_the_one_with_no_legend() {
    let with = laid_out(&below("v"));
    let without = laid_out(&alone("v"));
    let (doc, plain) = (with.chart_doc(), without.chart_doc());
    let (raster, plain_raster) = (
        doc.raster_rect.expect("a raster"),
        plain.raster_rect.expect("a raster"),
    );

    let page = doc.composed.plots[0]
        .legend_below
        .expect("the plot has a band");
    assert_eq!(page.height, BELOW_LEGEND_HEIGHT, "the band is not 44 high");
    assert_eq!(BELOW_LEGEND_HEIGHT, 44.0);
    assert!(
        (plain_raster.height() - raster.height() - 44.0).abs() <= 1.0,
        "the plot took {} of the height the same plot with no legend takes {}: not 44 less",
        raster.height(),
        plain_raster.height()
    );

    let band = in_window(page, raster);
    assert!(
        (band.min.y - raster.max.y).abs() <= 1.0,
        "the band {band:?} does not stand directly under the raster {raster:?}"
    );
    let pane = doc.viewport.expect("the pane recorded its box");
    assert!(
        band.max.y <= pane.max.y + 0.5,
        "the band {band:?} runs past the pane {pane:?}"
    );
    assert!(
        below_overhang(&doc.composed) >= 43.0,
        "the pane reserves {} under the raster for a band 44 high",
        below_overhang(&doc.composed)
    );

    for plot in &doc.composed.plots {
        assert!(
            !overlaps(plot.data_area(), page),
            "the band {page:?} is inside the data area {:?}",
            plot.data_area()
        );
    }

    assert_eq!(
        chart_window_size(&compose(&below("v"))),
        chart_window_size(&compose(&alone("v"))),
        "the legend under the plot changed the window the shell asks for"
    );
}

/// **AC4.** A plot holding a `legend: color` item still draws its legend at the
/// right, in a file where another plot's legend is under it: the list at the right
/// holds the one plot, the list under holds the other, and neither is in both.
#[test]
fn a_plot_holding_the_item_still_draws_its_legend_at_the_right() {
    let source = format!(
        "{}vconcat:\n{}{}  - legend: color\n    for: lower\n",
        data(),
        plot_item("g", "upper", "      - legend: color\n"),
        plot_item("v", "lower", "")
    );
    let composed = compose(&source);
    match &blocks(&composed)[..] {
        [(0, LegendSpec::Categorical { .. })] => {}
        other => panic!("the plot holding the item drew {other:?} at its right"),
    }
    assert!(
        band_width(&composed) > 0.0,
        "no band at the right of the upper plot"
    );
    match &below_blocks(&composed)[..] {
        [(1, LegendSpec::Sequential { .. }, _)] => {}
        other => panic!("the lower plot drew {other:?} under it"),
    }

    let app = laid_out(&source);
    let doc = app.chart_doc();
    let (raster, legend) = (
        doc.raster_rect.expect("a raster"),
        doc.legend_rect.expect("a band at the right"),
    );
    assert!(
        legend.min.x >= raster.max.x,
        "the band {legend:?} is not at the right of the raster {raster:?}"
    );
}

/// **The pin this arrangement owes.** A standalone legend whose `for:` names a plot
/// with no colour scale is that plot's, so the file's one colour plot is not given
/// a legend by it: neither plot draws one. Taking `for:` out of the test of an
/// unnamed standalone legend would hand the legend to the colour plot, though the
/// file put it on the other.
#[test]
fn a_legend_under_a_plot_with_no_colour_scale_gives_the_colour_plot_none() {
    let source = format!(
        "{}vconcat:\n{}{}  - legend: color\n    for: plain\n",
        data(),
        plot_item("g", "coloured", ""),
        plot_item("\"#aaaaaa\"", "plain", "")
    );
    let composed = compose(&source);
    assert_eq!(composed.plots.len(), 2, "two plots placed");
    assert!(
        LegendSpec::from_scales(&composed.plots[0].scales).is_some(),
        "the first plot has a colour scale, or this holds nothing"
    );
    assert!(
        blocks(&composed).is_empty() && below_blocks(&composed).is_empty(),
        "a legend that names the other plot was drawn: right {:?}, below {:?}",
        blocks(&composed),
        below_blocks(&composed)
    );
    assert_eq!(
        band_width(&composed),
        0.0,
        "a band is reserved at the right"
    );
    assert!(
        !composed.plots[0].legend_declared,
        "the colour plot is declared a legend it was not given"
    );
    assert!(
        composed.plots[1].legend_below.is_some(),
        "the legend the file put under the plain plot is not placed there"
    );
}
