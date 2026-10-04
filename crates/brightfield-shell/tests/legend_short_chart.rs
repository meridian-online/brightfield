//! **A number legend's ramp fits the band under its name, so a chart shorter
//! than the legend's block loses neither the ramp's foot nor the minimum.** The
//! band beside a chart is as tall as the chart, and a block begins at its plot's
//! top, so a chart shorter than the name's row, a gap and the ramp used to draw
//! the ramp past the band's foot, where the painter cut it.
//!
//! The legend is drawn through `legend::draw_band` into a headless egui context
//! over a band the height of the chart — the rect the chart pane reserves — and
//! read back as the shapes it painted, as `legend_column_name.rs` does for a
//! tall chart. Each test is about where ink went. The photographs of the short
//! charts, in both themes, are at the foot of the file.
//!
//! The specs carry their data inline, so the run needs no file beside it.
//!
//! Regenerate the photographs with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p
//! brightfield-shell --test legend_short_chart`.

use std::path::PathBuf;

use brightfield_shell::capture::capture_png;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{band_width, draw_band, ramp_floor, LegendSpec, RAMP_HEIGHT};
use brightfield_shell::pipeline::{compose_spec_str, Composed};
use brightfield_shell::window::Boot;
use meridian_design::spacing;

/// The band's left edge in the headless context.
const BAND_LEFT: f32 = 400.0;

/// Where the raster's top sits in the same coordinates, which is the band's top
/// too: the chart pane allocates the band level with the raster.
const RASTER_TOP: f32 = 20.0;

/// The column's name, which the legend names itself by.
const NAME: &str = "reading";

/// Seven rows, a number column on colour whose highest value is a whole number.
const VALUES: [f64; 7] = [1.5, 2.0, 3.5, 9.0, 4.0, 6.5, 5.0];

/// The diverging scale's attributes, with the pivot inside the domain.
const DIVERGING: &str = "colorScale: diverging\ncolorPivot: 4\n";

/// The plot's margins declared away. With the defaults a chart has to be 71
/// points tall to compose, and the legend then has room for a ramp; without them
/// a chart composes at 20, which is how a chart reaches the heights at which the
/// ramp and then the labels give way.
const NO_MARGINS: &str = "marginTop: 0\nmarginBottom: 0\n";

/// A chart `height` points tall holding a dot plot coloured by [`NAME`], with
/// its legend item, and `attrs` after the plot.
fn page(height: u32, attrs: &str) -> String {
    let rows: String = VALUES
        .iter()
        .enumerate()
        .map(|(i, v)| format!("    - {{ x: {i}, y: {}, {NAME}: {v} }}\n", i * 3))
        .collect();
    format!(
        "data:\n  t:\n{rows}plot:\n  - mark: dot\n    data: {{ from: t }}\n    x: x\n    y: y\n    fill: {NAME}\n  - legend: color\nwidth: 420\nheight: {height}\n{attrs}"
    )
}

fn compose(height: u32, attrs: &str) -> Composed {
    let source = page(height, attrs);
    compose_spec_str(&source, None)
        .unwrap_or_else(|e| panic!("the spec must compose: {e}\n{source}"))
}

/// One thing the legend painted.
#[derive(Debug, Clone, PartialEq)]
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

/// The band the chart pane reserves for `composed`: level with the raster and as
/// tall as the chart.
fn band_of(composed: &Composed) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(BAND_LEFT, RASTER_TOP),
        egui::vec2(band_width(composed), composed.height as f32),
    )
}

/// What `draw_band` paints for `composed` into `band`, in paint order.
fn painted(composed: &Composed, band: egui::Rect) -> Vec<Ink> {
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 900.0),
        )),
        ..Default::default()
    };
    let out = ctx.run_ui(raw, |ui| {
        draw_band(ui, band, RASTER_TOP, composed, Mode::Light);
    });
    let mut ink = Vec::new();
    for clipped in &out.shapes {
        match &clipped.shape {
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

/// What the legend painted, taken apart: the name, the value labels in paint
/// order (the maximum, the pivot when there is one, the minimum) and the strips.
struct Legend {
    name: egui::Rect,
    labels: Vec<(String, egui::Rect)>,
    strips: Vec<egui::Rect>,
}

impl Legend {
    fn read(ink: &[Ink]) -> Self {
        let mut texts = ink.iter().filter_map(|i| match i {
            Ink::Text(t, r) => Some((t.clone(), *r)),
            Ink::Fill(..) => None,
        });
        let (name, name_rect) = texts.next().expect("the legend painted no name");
        assert_eq!(name, NAME, "the first text is the column's name");
        Self {
            name: name_rect,
            labels: texts.collect(),
            strips: ink
                .iter()
                .filter_map(|i| match i {
                    Ink::Fill(r, _) => Some(*r),
                    Ink::Text(..) => None,
                })
                .collect(),
        }
    }

    /// The ramp's top and foot, when it was drawn.
    fn ramp(&self) -> Option<(f32, f32)> {
        let top = self.strips.iter().map(|r| r.min.y).reduce(f32::min)?;
        let foot = self.strips.iter().map(|r| r.max.y).reduce(f32::max)?;
        Some((top, foot))
    }

    /// The room under the name that the legend was drawn in, down to `band`'s foot.
    fn room(&self, band: egui::Rect) -> f32 {
        band.max.y - (self.name.max.y + spacing::SPACE_2)
    }
}

/// The height of a value label at the font's size, read off a chart tall enough
/// for the whole ramp.
fn normal_label_height() -> f32 {
    let composed = compose(400, "");
    let ink = painted(&composed, band_of(&composed));
    let legend = Legend::read(&ink);
    assert_eq!(legend.labels.len(), 2, "a tall chart draws both ends");
    legend.labels[0].1.height()
}

/// Every fact that holds of the legend at every chart height at which it is
/// drawn: nothing is outside the band, no two labels meet, and a label is the
/// font's size.
fn assert_fits(what: &str, legend: &Legend, band: egui::Rect, label_height: f32) {
    let mut every = vec![("name".to_owned(), legend.name)];
    every.extend(legend.labels.iter().cloned());
    every.extend(legend.strips.iter().map(|r| ("strip".to_owned(), *r)));
    for (part, rect) in &every {
        assert!(
            band.contains_rect(*rect),
            "{what}: the {part} {rect:?} is outside the band {band:?}"
        );
    }
    for (i, (a, ra)) in legend.labels.iter().enumerate() {
        assert_eq!(
            ra.height(),
            label_height,
            "{what}: the label {a:?} is not the font's size"
        );
        for (b, rb) in &legend.labels[i + 1..] {
            assert!(
                !ra.intersects(*rb),
                "{what}: the labels {a:?} {ra:?} and {b:?} {rb:?} meet"
            );
        }
    }
}

/// **A chart 120 points tall draws its whole legend inside its band.** The name,
/// the ramp and each value label lie inside the band, the labels do not meet and
/// they are the font's size; the ramp is shorter than the one a tall chart draws,
/// and a diverging scale keeps the pivot's label while the three stand clear.
#[test]
fn a_chart_120_tall_draws_its_legend_inside_its_band() {
    let label_height = normal_label_height();
    for (attrs, labels) in [("", 2), (DIVERGING, 3)] {
        let composed = compose(120, attrs);
        assert!(
            LegendSpec::of_plot(&composed.plots[0]).is_some(),
            "{attrs:?}: a number column with the item draws a legend"
        );
        let band = band_of(&composed);
        let legend = Legend::read(&painted(&composed, band));
        assert_fits(attrs, &legend, band, label_height);

        let (top, foot) = legend
            .ramp()
            .unwrap_or_else(|| panic!("{attrs:?}: a ramp is drawn at 120"));
        assert!(
            foot - top < RAMP_HEIGHT,
            "{attrs:?}: the ramp is {} tall, the whole ramp is {RAMP_HEIGHT}",
            foot - top
        );
        assert_eq!(
            legend.labels.len(),
            labels,
            "{attrs:?}: the labels at 120: {:?}",
            legend.labels
        );
        // The ends sit level with the ramp's, as they do on a tall chart.
        assert_eq!(legend.labels[0].1.min.y, top, "{attrs:?}: the maximum");
        assert_eq!(
            legend.labels[labels - 1].1.max.y,
            foot,
            "{attrs:?}: the minimum"
        );
    }
}

/// How much of a diverging legend a chart's height leaves, in the order it is
/// lost as the chart shortens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kept {
    /// The ramp, the pivot's label and the two ends'.
    RampWithPivot,
    /// The ramp and the two ends' labels.
    RampWithoutPivot,
    /// No ramp: the maximum's label alone, the one the room holds.
    Labels,
    /// Not even a label.
    NameOnly,
}

/// **As a chart shortens, the pivot's label goes first and the ramp second, and
/// what is drawn stays inside the band as the height falls.** Each height from 400
/// points down to 20, the shortest a chart with its margins declared away
/// composes at, composes the chart and reads its legend: no part of it is outside
/// the band, no two labels meet, a label is the font's size, the pivot's label
/// stays on a ramp that holds the three labels clear of one another and is gone
/// from a ramp that does not,
/// and the ramp is drawn exactly where the room under the name holds
/// [`ramp_floor`] — below it no strip is drawn and the labels that fit are, the
/// maximum first.
#[test]
fn the_pivot_goes_first_and_the_ramp_goes_at_its_floor_as_the_chart_shortens() {
    let label_height = normal_label_height();
    let floor = ramp_floor(label_height);
    assert!(
        floor > 2.0 * label_height,
        "the floor {floor} does not stand two labels of {label_height} apart"
    );
    let attrs = format!("{DIVERGING}{NO_MARGINS}");
    let mut last = Kept::RampWithPivot;
    let mut seen = std::collections::BTreeSet::new();
    for height in (20..=400).rev() {
        let composed = compose(height, &attrs);
        let band = band_of(&composed);
        let legend = Legend::read(&painted(&composed, band));
        let what = format!("a chart {height} tall");
        assert_fits(&what, &legend, band, label_height);

        let room = legend.room(band);
        let kept = match legend.ramp() {
            Some((top, foot)) => {
                assert!(
                    room >= floor,
                    "{what}: a ramp in a room of {room}, under the floor {floor}"
                );
                let ramp = foot - top;
                let pivot = legend.labels.len() == 3;
                if pivot {
                    assert!(
                        ramp > 3.0 * label_height,
                        "{what}: the pivot's label is on a ramp {ramp} tall"
                    );
                } else {
                    assert_eq!(legend.labels.len(), 2, "{what}: {:?}", legend.labels);
                    assert!(
                        ramp < 3.0 * label_height + 2.0 * spacing::SPACE_1,
                        "{what}: the pivot's label is gone from a ramp {ramp} tall"
                    );
                }
                if pivot {
                    Kept::RampWithPivot
                } else {
                    Kept::RampWithoutPivot
                }
            }
            None => {
                assert!(
                    room < floor,
                    "{what}: no ramp in a room of {room}, the floor is {floor}"
                );
                // What fits is drawn from the top: the maximum, then the minimum.
                let drawn: Vec<&str> = legend.labels.iter().map(|(t, _)| t.as_str()).collect();
                assert!(
                    ["9", "-1"].starts_with(&drawn),
                    "{what}: {drawn:?} is not the maximum then the minimum"
                );
                if legend.labels.is_empty() {
                    Kept::NameOnly
                } else {
                    Kept::Labels
                }
            }
        };
        assert!(
            kept >= last,
            "{what}: {kept:?} after {last:?}, so something came back as the chart shortened"
        );
        last = kept;
        seen.insert(kept);
    }
    for kept in [
        Kept::RampWithPivot,
        Kept::RampWithoutPivot,
        Kept::Labels,
        Kept::NameOnly,
    ] {
        assert!(
            seen.contains(&kept),
            "no height from 400 to 20 draws {kept:?}"
        );
    }
}

/// **A chart taller than the block draws as it did.** The whole ramp, at the
/// height it has always had, and every shape where it was: the same ink as in a
/// band 600 points tall, which no chart here fills.
#[test]
fn a_chart_taller_than_the_block_draws_the_whole_ramp_where_it_always_did() {
    for height in [300, 360, 400] {
        for attrs in ["", DIVERGING] {
            let composed = compose(height, attrs);
            let band = band_of(&composed);
            let ink = painted(&composed, band);
            let tall = band.with_max_y(band.min.y + 600.0);
            assert_eq!(
                ink,
                painted(&composed, tall),
                "{height} {attrs:?}: the ink moved with the band's height"
            );
            let (top, foot) = Legend::read(&ink).ramp().expect("a ramp is drawn");
            assert_eq!(foot - top, RAMP_HEIGHT, "{height} {attrs:?}: the ramp");
        }
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

/// The short chart as a booted page, photographed: the structural fact first, so
/// a reader of a red image can tell whether the legend or only the ink moved.
fn baseline(name: &str, mode: Mode, attrs: &str) {
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let composed = compose(120, attrs);
    let band = band_of(&composed);
    let legend = Legend::read(&painted(&composed, band));
    assert_fits(name, &legend, band, normal_label_height());
    let (top, foot) = legend.ramp().expect("a ramp is drawn at 120");
    assert!(
        foot - top < RAMP_HEIGHT,
        "{name}: the ramp is not shortened"
    );

    let out = scratch(name);
    let (w, h) = capture_png(Boot::charts(composed), mode, 1.0, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    egui_kittest::image_snapshot(&image, name);
}

/// **The short chart's legend, a number column on colour — light.** A ramp
/// shortened to the band, its two ends' labels at its top and foot.
#[test]
fn the_short_chart_legend_light_baseline() {
    baseline("legend_short_chart_light", Mode::Light, "");
}

/// **The same chart in dark.**
#[test]
fn the_short_chart_legend_dark_baseline() {
    baseline("legend_short_chart_dark", Mode::Dark, "");
}

/// **The short chart's legend on a diverging scale — light.** The pivot's label
/// at the middle while the three stand clear.
#[test]
fn the_short_diverging_chart_legend_light_baseline() {
    baseline("legend_short_diverging_light", Mode::Light, DIVERGING);
}

/// **The same chart in dark.**
#[test]
fn the_short_diverging_chart_legend_dark_baseline() {
    baseline("legend_short_diverging_dark", Mode::Dark, DIVERGING);
}
