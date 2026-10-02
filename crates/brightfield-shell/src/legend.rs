//! The chart legend as a native egui **margin panel** — outside the data,
//! always.
//!
//! # The rule this module mechanises
//!
//! A legend never sits inside the plot rect, never obstructs a mark, and never
//! exists twice. The in-scene legend the compose pipeline used to bake into
//! the top-right corner of the plot broke all three at once: it was drawn *on*
//! the data (over whatever marks happened to live in that corner), and the one
//! attempt to supplement it natively — a hardcoded "Series A/B/C" swatch block
//! in the controls rail — was a second legend, fixed at three series, that
//! mislabelled every chart that was not the fixture it was written against.
//!
//! So the pipeline now composes with its inline legend **off**
//! ([`crate::pipeline`] passes `draw_inline_legend = false`), and this module
//! is the only legend there is: one block per chart, derived from the
//! [`ScaleSet`] that chart was *actually drawn against* — the same value, not
//! a re-inference — placed in a reserved band beside the raster. Accuracy is
//! structural: the swatch colours are the palette the marks were painted with,
//! byte for byte, and the labels are the scale's own categories.
//!
//! The band is outside the presented raster by construction, which is what
//! makes "no legend overlaps data" a property a headless test can hold over
//! every example spec rather than a hope about pixel placement.

use brightfield_render::channel::Channel;
use brightfield_render::scale::{ramp_at, Scale, ScaleSet};
use brightfield_spec::edit::colour_legend_covers;
use brightfield_spec::layout::collect_legend_nodes;
use brightfield_spec::vocab::LegendChannel;
use brightfield_spec::Spec;
use meridian_design::{control, semantic, spacing, typography};
use meridian_egui::Mode;

use crate::pipeline::{Composed, PlotHandle};
use crate::text_ink;

/// What one chart's legend says: the derivation from its displayed scales,
/// with no egui type in it, so accuracy can be asserted in a unit test.
#[derive(Clone, Debug, PartialEq)]
pub enum LegendSpec {
    /// A categorical colour scale: one swatch per category, in scale order.
    Categorical {
        /// The entries, in the scale's own order.
        entries: Vec<LegendEntry>,
    },
    /// A continuous colour ramp: a vertical ramp with its domain ends, the
    /// maximum at the top.
    Sequential {
        /// Domain minimum.
        min: f64,
        /// Domain maximum.
        max: f64,
        /// The ramp's control points, low → high, straight-alpha RGBA.
        stops: Vec<[f32; 4]>,
    },
    /// A colour ramp about a pivot: a vertical ramp from the high pole at the
    /// top through the midpoint colour to the low pole, with the pivot named at
    /// the middle.
    Diverging {
        /// Domain minimum — the same distance below the pivot as `max` is above.
        min: f64,
        /// Domain maximum.
        max: f64,
        /// The value the middle of the bar stands for.
        pivot: f64,
        /// The ramp's control points, low pole → midpoint → high pole,
        /// straight-alpha RGBA.
        stops: Vec<[f32; 4]>,
    },
}

/// One categorical legend entry: the category and the ink its marks wear.
#[derive(Clone, Debug, PartialEq)]
pub struct LegendEntry {
    /// The category label, exactly as the scale spells it.
    pub label: String,
    /// The mark colour, straight-alpha RGBA — the palette value itself, so
    /// the swatch cannot drift from the raster.
    pub colour: [f32; 4],
}

impl LegendSpec {
    /// The legend `scales` calls for, or `None` when nothing on the plot maps
    /// colour. Reads the **fill** channel — what a legend over the plot would
    /// show. Whether the page draws it is the file's to say, and
    /// [`Self::of_plot`] is the one that answers for it.
    #[must_use]
    pub fn from_scales(scales: &ScaleSet) -> Option<Self> {
        match scales.get(Channel::Fill)? {
            Scale::Colour {
                categories,
                palette,
            } => Some(Self::Categorical {
                entries: categories
                    .iter()
                    .zip(palette.iter())
                    .map(|(label, colour)| LegendEntry {
                        label: label.clone(),
                        colour: *colour,
                    })
                    .collect(),
            }),
            Scale::Sequential {
                domain_min,
                domain_max,
                stops,
            } => Some(Self::Sequential {
                min: *domain_min,
                max: *domain_max,
                stops: stops.clone(),
            }),
            Scale::Diverging {
                domain_min,
                domain_max,
                pivot,
                stops,
            } => Some(Self::Diverging {
                min: *domain_min,
                max: *domain_max,
                pivot: *pivot,
                stops: stops.clone(),
            }),
            _ => None,
        }
    }

    /// The legend `plot` draws: the one its scales call for, when its file
    /// puts a legend on it ([`PlotHandle::legend_declared`]), and `None`
    /// otherwise. [`Self::from_scales`] says what a legend would show; this
    /// says whether the page draws one, and is what the band and its blocks
    /// read.
    #[must_use]
    pub fn of_plot(plot: &PlotHandle) -> Option<Self> {
        if plot.legend_declared {
            Self::from_scales(&plot.scales)
        } else {
            None
        }
    }

    /// The labels this legend shows, in order — the test hook behind
    /// "accurate to the series actually shown".
    #[must_use]
    pub fn labels(&self) -> Vec<&str> {
        match self {
            Self::Categorical { entries } => entries.iter().map(|e| e.label.as_str()).collect(),
            Self::Sequential { .. } | Self::Diverging { .. } => Vec::new(),
        }
    }
}

/// The label column a legend row reserves beside its swatch, in logical
/// points. Declared rather than measured because the *band* has to be a fact
/// before any frame exists — the window is sized around it — and a band sized
/// to its longest label would resize the window every time a category renamed.
/// A label longer than the column truncates; the swatch and the scale order
/// still identify the entry.
pub const LABEL_COLUMN: f32 = 96.0;

/// The width of one legend block: swatch, gap, label column.
#[must_use]
pub fn block_width() -> f32 {
    control::ICON_XS + spacing::ICON_LABEL_GAP + LABEL_COLUMN
}

/// Mark each plot of `plots` that the file puts a colour legend on.
///
/// A plot has one when its own items hold a `legend: color`, or a standalone
/// colour legend names it by `for:` (both read by
/// [`colour_legend_covers`]), or a standalone colour legend with no `for:`
/// stands in a file with exactly one plot whose scales call for a legend — the
/// plot such a legend can mean. A `for:` that is a `$param` names no plot, as
/// it does for the shelf's writer. The test for "calls for a legend" is the
/// scales' own ([`LegendSpec::from_scales`]), so a legend item over a plot with
/// no colour scale marks nothing and reserves no band.
///
/// Called once by the composition, after the plots are placed: the no-`for:`
/// case counts the plots beside the one it marks.
pub(crate) fn declare_legends(spec: &Spec, plots: &mut [PlotHandle]) {
    let unnamed_standalone = collect_legend_nodes(spec).iter().any(|(_, legend)| {
        legend.channel == LegendChannel::Color && !legend.options.contains_key("for")
    });
    let mut coloured = plots
        .iter()
        .enumerate()
        .filter(|(_, p)| LegendSpec::from_scales(&p.scales).is_some())
        .map(|(i, _)| i);
    let sole = match (coloured.next(), coloured.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    };
    for (i, plot) in plots.iter_mut().enumerate() {
        plot.legend_declared =
            colour_legend_covers(spec, &plot.path) || (unnamed_standalone && sole == Some(i));
    }
}

/// The legend blocks the page draws, as `(plot index, legend)`, in plot order:
/// one for each plot the file puts a legend on whose scales call for one.
///
/// [`band_width`] and [`draw_band`] both read this list, so the band is
/// reserved when a block is drawn into it, and a block is drawn for the plots
/// that reserved it; `legend_declared.rs` reads the same list.
#[must_use]
pub fn blocks(composed: &Composed) -> Vec<(usize, LegendSpec)> {
    composed
        .plots
        .iter()
        .enumerate()
        .filter_map(|(i, plot)| LegendSpec::of_plot(plot).map(|legend| (i, legend)))
        .collect()
}

/// The width the chart pane's legend band consumes, in logical points — `0.0`
/// when [`blocks`] is empty, which keeps a legendless dashboard's window
/// byte-identical to what it was before the band existed, and gives a plot with
/// no legend item the width the band would have held. Includes the gap between
/// the raster and the band.
///
/// Read by [`crate::window::chart_window_size`], so the band is a term of the
/// window arithmetic rather than a bite out of the raster's budget.
#[must_use]
pub fn band_width(composed: &Composed) -> f32 {
    if blocks(composed).is_empty() {
        0.0
    } else {
        spacing::CONTROL_GAP + block_width()
    }
}

/// Draw every plot's legend into the reserved band beside the raster.
///
/// `band` is the rect the chart pane reserved — entirely outside the
/// presented raster — and `raster_top` is the raster rect's top in the same
/// (window-space) coordinates, so each block can sit level with the plot it
/// describes: one legend per chart, at its chart's height, scoped to what
/// that chart shows.
pub fn draw_band(
    ui: &egui::Ui,
    band: egui::Rect,
    raster_top: f32,
    composed: &Composed,
    mode: Mode,
) {
    let painter = ui.painter_at(band);
    for (i, legend) in blocks(composed) {
        let y = raster_top + composed.plots[i].rect.y as f32;
        draw_block(
            &painter,
            egui::pos2(band.left(), y),
            &legend,
            composed.plots[i].fill_column.as_deref(),
            mode,
        );
    }
}

/// One legend block at `origin`: the column's name over the block, then a
/// swatch and its label for each category of a categorical scale, or a ramp
/// running top to bottom with its values beside it for a continuous one — the
/// domain's maximum level with the ramp's top, its minimum with the ramp's
/// foot, and for a diverging scale the pivot at the middle.
///
/// The name is cut short inside [`block_width`], the width the band was sized
/// to, so a long name leaves the block and the band as wide as they were
/// (`a_long_name_is_cut_short_inside_the_column_and_the_band_stays_as_wide`).
/// `name` is `None` for a plot whose fill names no column, which draws no
/// legend through [`LegendSpec::of_plot`].
fn draw_block(
    painter: &egui::Painter,
    origin: egui::Pos2,
    legend: &LegendSpec,
    name: Option<&str>,
    mode: Mode,
) {
    let sem = semantic(mode.is_dark());
    let ink = crate::design::to_color32(sem.text.secondary);
    let font = egui::FontId::proportional(typography::UI_SIZE);
    let swatch = control::ICON_XS;
    let row = swatch + spacing::SPACE_2;
    let mut origin = origin;
    if let Some(name) = name {
        let name_ink = crate::design::to_color32(sem.text.primary);
        let galley = text_ink::fit(painter, name, font.clone(), block_width(), name_ink);
        let height = galley.size().y;
        painter.galley(origin, galley, name_ink);
        origin.y += height + spacing::SPACE_2;
    }
    match legend {
        LegendSpec::Categorical { entries } => {
            for (i, entry) in entries.iter().enumerate() {
                let top = origin.y + i as f32 * row;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x, top),
                    egui::vec2(swatch, swatch),
                );
                painter.rect_filled(rect, 0.0, chart_ink(entry.colour));
                let galley = painter.layout(entry.label.clone(), font.clone(), ink, LABEL_COLUMN);
                painter.galley(
                    egui::pos2(
                        rect.right() + spacing::ICON_LABEL_GAP,
                        rect.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                    ink,
                );
            }
        }
        LegendSpec::Sequential { min, max, stops } => {
            let ramp = draw_ramp(painter, origin, stops);
            ramp_value(painter, ramp, egui::Align::Min, *max, &font, ink);
            ramp_value(painter, ramp, egui::Align::Max, *min, &font, ink);
        }
        LegendSpec::Diverging {
            min,
            max,
            pivot,
            stops,
        } => {
            let ramp = draw_ramp(painter, origin, stops);
            ramp_value(painter, ramp, egui::Align::Min, *max, &font, ink);
            ramp_value(painter, ramp, egui::Align::Center, *pivot, &font, ink);
            ramp_value(painter, ramp, egui::Align::Max, *min, &font, ink);
        }
    }
}

/// The ramp as [`RAMP_STRIPS`] adjacent solid strips, the ramp's high end at
/// the top and its low end at the foot, and the rect it fills.
///
/// Each strip is [`STRIP_HEIGHT`] tall, whole points, so no two strips share a
/// fractional edge for the rasteriser to blend into a seam.
fn draw_ramp(painter: &egui::Painter, origin: egui::Pos2, stops: &[[f32; 4]]) -> egui::Rect {
    let ramp = egui::Rect::from_min_size(origin, egui::vec2(control::ICON_XS, RAMP_HEIGHT));
    for (j, colour) in ramp_strip_colours(stops).iter().rev().enumerate() {
        let top = ramp.top() + j as f32 * STRIP_HEIGHT;
        let rect = egui::Rect::from_min_size(
            egui::pos2(ramp.left(), top),
            egui::vec2(ramp.width(), STRIP_HEIGHT),
        );
        painter.rect_filled(rect, 0.0, chart_ink(*colour));
    }
    ramp
}

/// A domain value beside `ramp`, in the label column and cut short inside it:
/// level with the ramp's top for [`egui::Align::Min`], its middle for
/// [`egui::Align::Center`], its foot for [`egui::Align::Max`].
fn ramp_value(
    painter: &egui::Painter,
    ramp: egui::Rect,
    at: egui::Align,
    value: f64,
    font: &egui::FontId,
    ink: egui::Color32,
) {
    let galley = text_ink::fit(
        painter,
        &format_domain(value),
        font.clone(),
        LABEL_COLUMN,
        ink,
    );
    let top = match at {
        egui::Align::Min => ramp.top(),
        egui::Align::Center => ramp.center().y - galley.size().y / 2.0,
        egui::Align::Max => ramp.bottom() - galley.size().y,
    };
    painter.galley(
        egui::pos2(ramp.right() + spacing::ICON_LABEL_GAP, top),
        galley,
        ink,
    );
}

/// How many strips a ramp is drawn in. Odd, so the middle strip is the ramp's
/// midpoint colour and stands at the ramp's centre, where a diverging ramp's
/// pivot is labelled.
const RAMP_STRIPS: usize = 71;

/// The height of one strip of a ramp, in logical points. Whole, so strips meet
/// on pixel edges at a scale of one.
const STRIP_HEIGHT: f32 = 2.0;

/// The height of a number column's ramp, in logical points: `RAMP_STRIPS`
/// strips of `STRIP_HEIGHT` each. The design gives the legend column a vertical
/// ramp and no figure for its height; this is read off the accepted frame of the
/// legend at the plot's right, where the ramp runs about this far.
pub const RAMP_HEIGHT: f32 = RAMP_STRIPS as f32 * STRIP_HEIGHT;

/// The colour of each strip of a ramp, **low end first**: strip `i` samples the
/// ramp at `i / (n - 1)`, so the first strip is the low end's colour, the last
/// is the high end's, and for an odd count the middle one is the midpoint. The
/// drawing runs them from the foot of the ramp to its top.
///
/// Sampling the ramp rather than painting one strip per stop is what puts both
/// ends on a ramp that has to show both: a strip per stop leaves the last
/// stop's without a height.
#[must_use]
pub fn ramp_strip_colours(stops: &[[f32; 4]]) -> Vec<[f32; 4]> {
    (0..RAMP_STRIPS)
        .map(|i| ramp_at(stops, i as f64 / (RAMP_STRIPS - 1) as f64))
        .collect()
}

/// The colour of each strip of a diverging legend's ramp, low pole first —
/// [`ramp_strip_colours`], under the name the diverging legend's tests read it
/// by. The middle strip is the midpoint colour, level with the pivot's label.
#[must_use]
pub fn diverging_strip_colours(stops: &[[f32; 4]]) -> Vec<[f32; 4]> {
    ramp_strip_colours(stops)
}

/// A domain end, spelled the short way: integers bare, fractions to two
/// places.
fn format_domain(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

/// A palette colour to egui ink — straight alpha, same quantisation as the
/// chrome's one colour boundary. The value is the **chart's** ink, used raw:
/// remapping it through a semantic token would let the swatch and the raster
/// disagree.
fn chart_ink(c: [f32; 4]) -> egui::Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    egui::Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colour_scale() -> ScaleSet {
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::Fill,
            Scale::Colour {
                categories: vec!["A".into(), "B".into(), "C".into()],
                palette: vec![
                    [0.1, 0.2, 0.3, 1.0],
                    [0.4, 0.5, 0.6, 1.0],
                    [0.7, 0.8, 0.9, 1.0],
                ],
            },
        );
        scales
    }

    /// Accuracy is structural: the legend is the scale's own categories in the
    /// scale's own order, wearing the scale's own palette — not a re-inference
    /// that could drift from what the marks were painted with.
    #[test]
    fn a_categorical_legend_is_the_scale_verbatim() {
        let legend = LegendSpec::from_scales(&colour_scale()).expect("a fill scale has a legend");
        assert_eq!(legend.labels(), vec!["A", "B", "C"]);
        let LegendSpec::Categorical { entries } = legend else {
            panic!("a colour scale derives a categorical legend");
        };
        assert_eq!(entries[1].colour, [0.4, 0.5, 0.6, 1.0]);
    }

    /// No fill scale, no legend — and therefore no band: a legendless
    /// dashboard gives up no width at all.
    #[test]
    fn no_fill_scale_means_no_legend_and_no_band() {
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Linear {
                domain_min: 0.0,
                domain_max: 1.0,
                range_start: 0.0,
                range_end: 100.0,
            },
        );
        assert_eq!(LegendSpec::from_scales(&scales), None);
        assert_eq!(band_width(&Composed::empty()), 0.0);
    }

    /// A sequential ramp derives the gradient form with its domain ends.
    #[test]
    fn a_sequential_scale_derives_a_ramp_legend() {
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::Fill,
            Scale::Sequential {
                domain_min: 2.0,
                domain_max: 9.0,
                stops: vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
            },
        );
        let legend = LegendSpec::from_scales(&scales).expect("a ramp has a legend");
        assert_eq!(
            legend,
            LegendSpec::Sequential {
                min: 2.0,
                max: 9.0,
                stops: vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
            }
        );
        assert!(legend.labels().is_empty(), "a ramp has ends, not entries");
    }
    /// **A categorical swatch is drawn square** — the corner is the token's
    /// zero and not a literal of the call site's.
    #[test]
    fn a_categorical_swatch_is_drawn_without_a_corner() {
        let spec = LegendSpec::from_scales(&colour_scale()).expect("a colour scale has a legend");
        let LegendSpec::Categorical { entries } = &spec else {
            panic!("a categorical scale drew {spec:?}");
        };
        let inks: Vec<egui::Color32> = entries.iter().map(|e| chart_ink(e.colour)).collect();
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(300.0, 200.0),
            )),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            let painter = ui.painter().clone();
            draw_block(&painter, egui::pos2(10.0, 10.0), &spec, None, Mode::Light);
        });
        let mut swatches = Vec::new();
        for clipped in &out.shapes {
            if let egui::Shape::Rect(r) = &clipped.shape {
                if inks.contains(&r.fill) {
                    swatches.push(r.clone());
                }
            }
        }
        assert_eq!(
            swatches.len(),
            inks.len(),
            "one swatch per entry: {swatches:?}"
        );
        for swatch in &swatches {
            assert_eq!(
                swatch.corner_radius,
                egui::CornerRadius::ZERO,
                "the swatch at {:?} carries a corner",
                swatch.rect
            );
        }
    }
}
