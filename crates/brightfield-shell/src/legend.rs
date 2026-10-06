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
//!
//! # The second placement: under the plot
//!
//! A file can put the legend under its plot instead: a `vconcat` of the named
//! plot and a standalone `legend: color` whose `for:` names it. The layout owns
//! that arrangement ([`brightfield_spec::layout::below_legends`]): it gives the
//! legend a band [`brightfield_spec::layout::BELOW_LEGEND_HEIGHT`] high, as wide
//! as the plot, and lays the plot out in the height above it, so the band is
//! carved from the pane's height and the window the shell asks for is the one it
//! asks for with no legend. [`below_blocks`] lists those legends, [`blocks`] and
//! [`band_width`] leave them out — a plot's legend is drawn once, in one place —
//! and [`draw_below`] draws each into its rect, the ramp running left to right
//! with its values under it, or the swatches in a row.

use brightfield_render::channel::Channel;
use brightfield_render::scale::{ramp_at, Scale, ScaleSet};
use brightfield_spec::edit::colour_legend_covers;
use brightfield_spec::layout::{below_legends, collect_legend_nodes, Rect};
use brightfield_spec::vocab::{LegendChannel, MarkKind};
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
        /// The value the middle of the ramp stands for.
        pivot: f64,
        /// The ramp's control points, low pole → midpoint → high pole,
        /// straight-alpha RGBA.
        stops: Vec<[f32; 4]>,
    },
    /// A stepped colour scale (`colorScale: quantize`): one flat block for each
    /// step, the highest step at the top, with the value each block begins and
    /// ends at beside it.
    Steps {
        /// The colour of each step, lowest step first, straight-alpha RGBA.
        colours: Vec<[f32; 4]>,
        /// The values that bound the steps, lowest first: one more than there are
        /// steps. The scale's own [`Scale::step_edges`], so a block's labels
        /// bound the points that wear its colour.
        edges: Vec<f64>,
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
            quantized @ Scale::Quantized { colours, .. } => Some(Self::Steps {
                colours: colours.clone(),
                edges: quantized.step_edges()?,
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
            Self::Sequential { .. } | Self::Diverging { .. } | Self::Steps { .. } => Vec::new(),
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
/// Also records, for each plot, the band its legend is drawn in when the file
/// puts the legend under it ([`PlotHandle::legend_below`]): the layout's own
/// answer for `viewport`, the box the composition was laid out in.
///
/// Called once by the composition, after the plots are placed: the no-`for:`
/// case counts the plots beside the one it marks.
pub(crate) fn declare_legends(spec: &Spec, viewport: Rect, plots: &mut [PlotHandle]) {
    let below = below_legends(spec, viewport);
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
        plot.legend_below = below
            .iter()
            .find(|b| b.plot_path == plot.path)
            .map(|b| b.rect);
    }
}

/// The legend blocks the page draws at the plot's right, as `(plot index,
/// legend)`, in plot order: one for each plot the file puts a legend on whose
/// scales call for one, and whose legend the file does not put under it
/// ([`below_blocks`] lists those).
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
        .filter(|(_, plot)| plot.legend_below.is_none())
        .filter_map(|(i, plot)| LegendSpec::of_plot(plot).map(|legend| (i, legend)))
        .collect()
}

/// The legend blocks the page draws in a band under their plot, as `(plot
/// index, legend, band)`, in plot order: one for each plot whose file puts its
/// legend there ([`PlotHandle::legend_below`]) and whose scales call for one.
/// The band is on the page plane, the layout's own rect.
#[must_use]
pub fn below_blocks(composed: &Composed) -> Vec<(usize, LegendSpec, Rect)> {
    composed
        .plots
        .iter()
        .enumerate()
        .filter_map(|(i, plot)| {
            let band = plot.legend_below?;
            LegendSpec::of_plot(plot).map(|legend| (i, legend, band))
        })
        .collect()
}

/// How far the bands under the plots reach below the raster, in logical points:
/// `0.0` when no plot has one, or when each stands inside the page because
/// something else extends past it.
///
/// The raster is the plots' bounding box and a legend is not a plot, so a band
/// under the lowest plot is outside the raster, in room the pane has to leave
/// for it. It is already part of the height the layout was offered, so this is
/// what the pane allocates under the raster and not a bite out of the offer.
/// It is deliberately not a term of the window's size
/// ([`crate::window::chart_window_size`] reads the raster alone), which is why
/// a legend under its plot asks for the window the plot asks for alone.
#[must_use]
pub fn below_overhang(composed: &Composed) -> f32 {
    composed
        .plots
        .iter()
        .filter_map(|plot| plot.legend_below)
        .map(|band| (band.y + band.height) as f32 - composed.height as f32)
        .fold(0.0, f32::max)
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

/// The reserved column a counting aggregate lands in. `fill: { count: }` binds
/// a fill channel to it, since no column of the author's holds a count, so a
/// plot whose fill is this column is a fill the transform produced. Matched as a
/// literal because the constant is private to `brightfield-render` (its title
/// code matches the same literal); the legend tests over a hexbin go red if the
/// alias is renamed.
const COUNT_COLUMN: &str = "__bf_count";

/// What a legend over a count is named: the word vgplot gives a counting
/// aggregate, which `exprLabel` in its `plot-renderer.js` makes of `count(*)`.
const COUNT_NAME: &str = "count";

/// What a legend over a heatmap is named: the name vgplot gives the grid a
/// heatmap smooths, `DENSITY` in its `Grid2DMark.js`.
const DENSITY_NAME: &str = "density";

/// The name the legend over `plot`'s fill carries over its ramp or its swatches.
///
/// A fill that is a column is named for the column. A fill the transform
/// produced has no column of the author's to name, so it is named for the
/// transform: `density` for a heatmap, and `count` for a hexbin coloured by
/// `fill: { count: }`, a raster, or any other mark whose fill is the count.
/// A raster and a heatmap set no fill channel and colour by their bins all the
/// same, so a plot with none is named for them. The first such mark in draw order
/// gives the word. `None` when neither a column nor a transform names it, which
/// is a plot whose scales call for no legend.
#[must_use]
pub fn legend_name(plot: &PlotHandle) -> Option<&str> {
    let column = plot.fill_column.as_deref();
    if let Some(column) = column.filter(|column| *column != COUNT_COLUMN) {
        return Some(column);
    }
    let transform = plot.marks.iter().find_map(|kind| match kind {
        MarkKind::Heatmap => Some(DENSITY_NAME),
        MarkKind::Hexbin | MarkKind::Raster => Some(COUNT_NAME),
        _ => None,
    });
    transform.or(column.map(|_| COUNT_NAME))
}

/// Draw every plot's legend into the reserved band beside the raster.
///
/// `band` is the rect the chart pane reserved — entirely outside the
/// presented raster — and `raster_top` is the raster rect's top in the same
/// (window-space) coordinates, so each block can sit level with the plot it
/// describes: one legend per chart, at its chart's height, scoped to what
/// that chart shows.
///
/// The band is as tall as its chart, so a block's room is what is left of the
/// band under its plot's top. A number legend's ramp gives way to that room, and
/// its value labels do not (`draw_block` says how).
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
            band.bottom(),
            &legend,
            legend_name(&composed.plots[i]),
            mode,
        );
    }
}

/// Draw every plot's legend that sits in a band under it, each into the rect
/// the layout reserved for it.
///
/// `origin` is the raster rect's top-left in window-space coordinates, so a band
/// on the page plane lands where the layout put it. The painter is clipped to
/// the band, so a long list of categories is cut at the band's right edge and
/// never drawn over the plot or past the tile.
pub fn draw_below(ui: &egui::Ui, origin: egui::Pos2, composed: &Composed, mode: Mode) {
    for (i, legend, rect) in below_blocks(composed) {
        let band = egui::Rect::from_min_size(
            origin + egui::vec2(rect.x as f32, rect.y as f32),
            egui::vec2(rect.width as f32, rect.height as f32),
        );
        draw_below_block(
            &ui.painter_at(band),
            band,
            &legend,
            legend_name(&composed.plots[i]),
            mode,
        );
    }
}

/// The widest the ramp under a plot runs, in logical points. The ramp is one
/// strip for each colour [`ramp_strip_colours`] samples, each a whole number of
/// points wide, so it runs a little short of this where the strips do not divide it.
pub const BELOW_RAMP_MAX_WIDTH: f32 = 240.0;

/// The height of the ramp under a plot, in logical points: a swatch's height, so
/// the ramp and the categorical swatches share a row's weight.
pub const BELOW_RAMP_HEIGHT: f32 = control::ICON_XS;

/// One legend block in the `band` under its plot: the legend's name at the left,
/// then for a continuous scale a ramp running left to right — the low end at its
/// left — with the domain's two ends under it (and a diverging scale's pivot
/// under its middle), or for a categorical scale a swatch and its label for each
/// category, in a row. The block is centred on the band's height.
///
/// The name is cut short inside [`LABEL_COLUMN`], as it is at the plot's right,
/// and the ramp starts after what is drawn of it. `name` is [`legend_name`]'s,
/// and `None` for a plot whose fill is neither a column nor a transform's
/// output, which draws no legend through [`LegendSpec::of_plot`].
pub fn draw_below_block(
    painter: &egui::Painter,
    band: egui::Rect,
    legend: &LegendSpec,
    name: Option<&str>,
    mode: Mode,
) {
    let sem = semantic(mode.is_dark());
    let ink = crate::design::to_color32(sem.text.secondary);
    let name_ink = crate::design::to_color32(sem.text.primary);
    let font = egui::FontId::proportional(typography::UI_SIZE);
    let name = name.map(|name| text_ink::fit(painter, name, font.clone(), LABEL_COLUMN, name_ink));
    // Where the name's row is centred: the band's middle for swatches, the
    // ramp's for a ramp, whose values hang under it.
    let name_at = |painter: &egui::Painter, left: f32, centre: f32| -> f32 {
        let Some(galley) = &name else {
            return left;
        };
        let size = galley.size();
        painter.galley(
            egui::pos2(left, centre - size.y / 2.0),
            galley.clone(),
            name_ink,
        );
        left + size.x + spacing::CONTROL_GAP
    };
    match legend {
        LegendSpec::Categorical { entries } => {
            let centre = band.center().y;
            let mut x = name_at(painter, band.left(), centre);
            let swatch = control::ICON_XS;
            for entry in entries {
                if x >= band.right() {
                    break;
                }
                let rect = egui::Rect::from_center_size(
                    egui::pos2(x + swatch / 2.0, centre),
                    egui::vec2(swatch, swatch),
                );
                painter.rect_filled(rect, 0.0, chart_ink(entry.colour));
                let galley = painter.layout_no_wrap(entry.label.clone(), font.clone(), ink);
                let label_x = rect.right() + spacing::ICON_LABEL_GAP;
                painter.galley(
                    egui::pos2(label_x, centre - galley.size().y / 2.0),
                    galley.clone(),
                    ink,
                );
                x = label_x + galley.size().x + spacing::CONTROL_GAP;
            }
        }
        LegendSpec::Sequential { min, max, stops } => {
            let ramp = below_ramp(painter, band, stops, &font, &name_at);
            below_value(painter, ramp, egui::Align::Min, *min, &font, ink);
            below_value(painter, ramp, egui::Align::Max, *max, &font, ink);
        }
        LegendSpec::Diverging {
            min,
            max,
            pivot,
            stops,
        } => {
            let ramp = below_ramp(painter, band, stops, &font, &name_at);
            below_value(painter, ramp, egui::Align::Min, *min, &font, ink);
            below_value(painter, ramp, egui::Align::Center, *pivot, &font, ink);
            below_value(painter, ramp, egui::Align::Max, *max, &font, ink);
        }
        LegendSpec::Steps { colours, edges } => {
            below_steps(painter, band, colours, edges, &font, &name_at, ink);
        }
    }
}

/// Where the ramp under a plot starts: the left edge after the name, the top of
/// the ramp's row, and the width the ramp has to run in. The ramp and the row of
/// values under it are centred together on the band's height, and the name is
/// drawn first, level with the ramp.
fn below_origin(
    painter: &egui::Painter,
    band: egui::Rect,
    font: &egui::FontId,
    name_at: &dyn Fn(&egui::Painter, f32, f32) -> f32,
) -> (f32, f32, f32) {
    let value_height = painter
        .layout_no_wrap(String::from("0"), font.clone(), egui::Color32::WHITE)
        .size()
        .y;
    let rows = BELOW_RAMP_HEIGHT + spacing::SPACE_2 + value_height;
    let top = band.center().y - rows / 2.0;
    let left = name_at(painter, band.left(), top + BELOW_RAMP_HEIGHT / 2.0);
    let room = (band.right() - left).clamp(0.0, BELOW_RAMP_MAX_WIDTH);
    (left, top, room)
}

/// The steps under a plot: one flat block for each, equal in width, the lowest at
/// the left, with the value each begins at under its left edge and the highest
/// step's upper bound under the right end of the last.
///
/// Each block is whole points wide, so no two share a fractional edge for the
/// rasteriser to blend into a seam, and the row is at most
/// [`BELOW_RAMP_MAX_WIDTH`] wide. A value is drawn when it stands clear of the
/// one before it; the two at the ends are always drawn, and each is kept inside
/// the row's own extent, as the ramp's are. A band with less width than a point
/// for each step draws the name alone.
fn below_steps(
    painter: &egui::Painter,
    band: egui::Rect,
    colours: &[[f32; 4]],
    edges: &[f64],
    font: &egui::FontId,
    name_at: &dyn Fn(&egui::Painter, f32, f32) -> f32,
    ink: egui::Color32,
) {
    let (left, top, room) = below_origin(painter, band, font, name_at);
    let Some(block) = step_block_size(room, colours.len()) else {
        return;
    };
    let steps = colours.len();
    let row = egui::Rect::from_min_size(
        egui::pos2(left, top),
        egui::vec2(block * steps as f32, BELOW_RAMP_HEIGHT),
    );
    for (i, colour) in colours.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(row.left() + i as f32 * block, row.top()),
            egui::vec2(block, row.height()),
        );
        painter.rect_filled(rect, 0.0, chart_ink(*colour));
    }
    let galleys: Vec<_> = edges
        .iter()
        .map(|value| {
            text_ink::fit(
                painter,
                &format_domain(*value),
                font.clone(),
                row.width(),
                ink,
            )
        })
        .collect();
    // Each value's extent along the row, the ends kept inside the row.
    let spans: Vec<(f32, f32)> = galleys
        .iter()
        .enumerate()
        .map(|(i, galley)| {
            let width = galley.size().x;
            let at = row.left() + i as f32 * block;
            let start = if i == 0 {
                row.left()
            } else if i == steps {
                row.right() - width
            } else {
                at - width / 2.0
            };
            (start, start + width)
        })
        .collect();
    for i in kept_labels(&spans, spacing::CONTROL_GAP) {
        painter.galley(
            egui::pos2(spans[i].0, row.bottom() + spacing::SPACE_2),
            galleys[i].clone(),
            ink,
        );
    }
}

/// The ramp under a plot, as [`RAMP_STRIPS`] adjacent solid strips, the low end
/// at the left, and the rect it fills. The name is drawn first, level with the
/// ramp, and the ramp starts after it; the ramp and the row of values under it
/// are centred together on the band's height.
///
/// Each strip is whole points wide, so no two share a fractional edge for the
/// rasteriser to blend into a seam, and the ramp is at most
/// [`BELOW_RAMP_MAX_WIDTH`] wide.
fn below_ramp(
    painter: &egui::Painter,
    band: egui::Rect,
    stops: &[[f32; 4]],
    font: &egui::FontId,
    name_at: &dyn Fn(&egui::Painter, f32, f32) -> f32,
) -> egui::Rect {
    let (left, top, room) = below_origin(painter, band, font, name_at);
    let strip = (room / RAMP_STRIPS as f32).floor().max(1.0);
    let ramp = egui::Rect::from_min_size(
        egui::pos2(left, top),
        egui::vec2(strip * RAMP_STRIPS as f32, BELOW_RAMP_HEIGHT),
    );
    for (j, colour) in ramp_strip_colours(stops).iter().enumerate() {
        let strip_rect = egui::Rect::from_min_size(
            egui::pos2(ramp.left() + j as f32 * strip, ramp.top()),
            egui::vec2(strip, ramp.height()),
        );
        painter.rect_filled(strip_rect, 0.0, chart_ink(*colour));
    }
    ramp
}

/// A domain value under `ramp`: its left end for [`egui::Align::Min`], its middle
/// for [`egui::Align::Center`], its right end for [`egui::Align::Max`], each
/// kept inside the ramp's own extent so no value hangs past the end it names.
fn below_value(
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
        ramp.width(),
        ink,
    );
    let left = match at {
        egui::Align::Min => ramp.left(),
        egui::Align::Center => ramp.center().x - galley.size().x / 2.0,
        egui::Align::Max => ramp.right() - galley.size().x,
    };
    painter.galley(
        egui::pos2(left, ramp.bottom() + spacing::SPACE_2),
        galley,
        ink,
    );
}

/// One legend block at `origin`: the legend's name over the block, then a
/// swatch and its label for each category of a categorical scale, or a ramp
/// running top to bottom with its values beside it for a continuous one — the
/// domain's maximum level with the ramp's top, its minimum with the ramp's
/// foot, and for a diverging scale the pivot at the middle.
///
/// The name is cut short inside [`block_width`], the width the band was sized
/// to, so a long name leaves the block and the band as wide as they were
/// (`a_long_name_is_cut_short_inside_the_column_and_the_band_stays_as_wide`).
/// `name` is [`legend_name`]'s, and `None` for a plot whose fill is neither a
/// column nor a transform's output, which draws no legend through
/// [`LegendSpec::of_plot`].
///
/// `bottom` is the foot of the room the block draws in, in the same
/// coordinates as `origin`: the band's. A number column's ramp is drawn as tall
/// as the room under the name allows, up to [`RAMP_HEIGHT`] ([`number_fit`]).
fn draw_block(
    painter: &egui::Painter,
    origin: egui::Pos2,
    bottom: f32,
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
            let values = RampValues {
                stops,
                max: *max,
                pivot: None,
                min: *min,
            };
            number_legend(painter, origin, bottom, &values, &font, ink);
        }
        LegendSpec::Diverging {
            min,
            max,
            pivot,
            stops,
        } => {
            let values = RampValues {
                stops,
                max: *max,
                pivot: Some(*pivot),
                min: *min,
            };
            number_legend(painter, origin, bottom, &values, &font, ink);
        }
        LegendSpec::Steps { colours, edges } => {
            steps_legend(painter, origin, bottom, colours, edges, &font, ink);
        }
    }
}

/// What a number column's legend says beside its ramp: the ramp's stops and the
/// domain's ends, with the pivot of a diverging scale.
struct RampValues<'a> {
    stops: &'a [[f32; 4]],
    max: f64,
    pivot: Option<f64>,
    min: f64,
}

/// A number column's legend under its name, at `origin`, in the room down to
/// `bottom`: the ramp with its values beside it, or as much of that as the room
/// holds ([`number_fit`]).
///
/// The labels keep the font's size whatever the room; it is the ramp that
/// gives way, down to the height that stands the two end labels one over the
/// other, and then it is not drawn. Below that the labels stand in the label
/// column on their own, the maximum over the minimum from `origin` down, and a
/// label the room cannot hold whole is not drawn (a band no taller than the
/// name and one label's row draws the name alone).
fn number_legend(
    painter: &egui::Painter,
    origin: egui::Pos2,
    bottom: f32,
    values: &RampValues,
    font: &egui::FontId,
    ink: egui::Color32,
) {
    let label = |value: f64| {
        text_ink::fit(
            painter,
            &format_domain(value),
            font.clone(),
            LABEL_COLUMN,
            ink,
        )
    };
    let (high, low) = (label(values.max), label(values.min));
    let label_height = high.size().y;
    match number_fit(bottom - origin.y, label_height, values.pivot.is_some()) {
        NumberFit::Ramp { strips, pivot } => {
            let ramp = draw_ramp(painter, origin, values.stops, strips);
            ramp_value(painter, ramp, egui::Align::Min, high, ink);
            if let Some(pivot) = values.pivot.filter(|_| pivot) {
                ramp_value(painter, ramp, egui::Align::Center, label(pivot), ink);
            }
            ramp_value(painter, ramp, egui::Align::Max, low, ink);
        }
        NumberFit::Labels => end_labels(painter, origin, bottom, [high, low], ink),
    }
}

/// The two end labels alone, the maximum over the minimum from `origin` down, in
/// the label column; a label the room down to `bottom` cannot hold whole is not
/// drawn.
fn end_labels(
    painter: &egui::Painter,
    origin: egui::Pos2,
    bottom: f32,
    labels: [std::sync::Arc<egui::Galley>; 2],
    ink: egui::Color32,
) {
    let column = origin.x + control::ICON_XS + spacing::ICON_LABEL_GAP;
    let label_height = labels[0].size().y;
    for (i, galley) in labels.into_iter().enumerate() {
        let top = origin.y + i as f32 * (label_height + LABEL_GAP);
        if top + label_height <= bottom {
            painter.galley(egui::pos2(column, top), galley, ink);
        }
    }
}

/// A stepped scale's legend under its name, at `origin`, in the room down to
/// `bottom`: a stack of flat blocks, one for each step, the highest at the top,
/// with the value at each boundary beside the stack.
///
/// The stack is as tall as the ramp a number column would draw in the same room
/// ([`number_fit`]), to within the rounding: the blocks are the same whole number
/// of points ([`step_block_size`]), so a block does not share a fractional edge
/// with the next. A boundary's label is level with the line between its two blocks, the
/// highest level with the stack's top and the lowest with its foot, and one that
/// would stand within [`LABEL_GAP`] of the one above it is not drawn
/// ([`kept_labels`]); the two at the ends always are. In a room that holds no
/// stack, or one a point high for each step cannot fill, the end labels stand
/// alone, as a ramp's do.
fn steps_legend(
    painter: &egui::Painter,
    origin: egui::Pos2,
    bottom: f32,
    colours: &[[f32; 4]],
    edges: &[f64],
    font: &egui::FontId,
    ink: egui::Color32,
) {
    let steps = colours.len();
    let label = |value: f64| {
        text_ink::fit(
            painter,
            &format_domain(value),
            font.clone(),
            LABEL_COLUMN,
            ink,
        )
    };
    let (Some(&high), Some(&low)) = (edges.last(), edges.first()) else {
        return;
    };
    let label_height = label(high).size().y;
    let block = match number_fit(bottom - origin.y, label_height, false) {
        NumberFit::Ramp { strips, .. } => step_block_size(strips as f32 * STRIP_HEIGHT, steps),
        NumberFit::Labels => None,
    };
    let Some(block) = block else {
        end_labels(painter, origin, bottom, [label(high), label(low)], ink);
        return;
    };
    for (i, colour) in colours.iter().rev().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(origin.x, origin.y + i as f32 * block),
            egui::vec2(control::ICON_XS, block),
        );
        painter.rect_filled(rect, 0.0, chart_ink(*colour));
    }
    // The boundaries from the top: boundary `i` is `i` blocks down the stack.
    let galleys: Vec<_> = edges.iter().rev().map(|value| label(*value)).collect();
    let spans: Vec<(f32, f32)> = (0..=steps)
        .map(|i| {
            let line = origin.y + i as f32 * block;
            let start = if i == 0 {
                line
            } else if i == steps {
                line - label_height
            } else {
                line - label_height / 2.0
            };
            (start, start + label_height)
        })
        .collect();
    let column = origin.x + control::ICON_XS + spacing::ICON_LABEL_GAP;
    for i in kept_labels(&spans, LABEL_GAP) {
        painter.galley(egui::pos2(column, spans[i].0), galleys[i].clone(), ink);
    }
}

/// How many whole points each of `steps` blocks is across `extent` points, or
/// `None` when a block would be under a point: the stack gives way to the end
/// labels rather than draw blocks that share a fractional edge.
///
/// Every block is the same, so the stack is `steps` times this: as long as
/// `extent` to within `steps - 1` points.
#[must_use]
pub fn step_block_size(extent: f32, steps: usize) -> Option<f32> {
    if steps == 0 {
        return None;
    }
    let block = (extent / steps as f32).floor();
    (block >= 1.0).then_some(block)
}

/// Which of a legend's value labels are drawn, as indices into `spans`: each
/// label's extent along the legend, `(start, end)`, in reading order.
///
/// The first and the last are always drawn, since they name the ends the legend
/// runs between. One between them is drawn when it stands `gap` clear of the one
/// drawn before it and of the last, so a legend with many steps and little room
/// draws the labels that fit and none that overlap.
#[must_use]
pub fn kept_labels(spans: &[(f32, f32)], gap: f32) -> Vec<usize> {
    let Some(last) = spans.len().checked_sub(1) else {
        return Vec::new();
    };
    if last == 0 {
        return vec![0];
    }
    let mut kept = vec![0];
    let mut edge = spans[0].1;
    for (i, &(start, end)) in spans.iter().enumerate().take(last).skip(1) {
        if start >= edge + gap && end + gap <= spans[last].0 {
            kept.push(i);
            edge = end;
        }
    }
    kept.push(last);
    kept
}

/// The ramp as `strips` adjacent solid strips, the ramp's high end at the top
/// and its low end at the foot, and the rect it fills.
///
/// Each strip is [`STRIP_HEIGHT`] tall, whole points, so no two strips share a
/// fractional edge for the rasteriser to blend into a seam.
fn draw_ramp(
    painter: &egui::Painter,
    origin: egui::Pos2,
    stops: &[[f32; 4]],
    strips: usize,
) -> egui::Rect {
    let ramp = egui::Rect::from_min_size(
        origin,
        egui::vec2(control::ICON_XS, strips as f32 * STRIP_HEIGHT),
    );
    for (j, colour) in strip_colours(stops, strips).iter().rev().enumerate() {
        let top = ramp.top() + j as f32 * STRIP_HEIGHT;
        let rect = egui::Rect::from_min_size(
            egui::pos2(ramp.left(), top),
            egui::vec2(ramp.width(), STRIP_HEIGHT),
        );
        painter.rect_filled(rect, 0.0, chart_ink(*colour));
    }
    ramp
}

/// A domain value beside `ramp`, in the label column: level with the ramp's top
/// for [`egui::Align::Min`], its middle for [`egui::Align::Center`], its foot
/// for [`egui::Align::Max`]. `galley` is the value laid out and cut short inside
/// [`LABEL_COLUMN`].
fn ramp_value(
    painter: &egui::Painter,
    ramp: egui::Rect,
    at: egui::Align,
    galley: std::sync::Arc<egui::Galley>,
    ink: egui::Color32,
) {
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

/// The height of a number column's ramp, in logical points, when the room under
/// its name holds it: `RAMP_STRIPS` strips of `STRIP_HEIGHT` each. The design
/// gives the legend column a vertical ramp and no figure for its height; this is
/// read off the accepted frame of the legend at the plot's right, where the ramp
/// runs about this far. A chart shorter than the name's row, a gap and this
/// draws a shorter ramp, down to [`ramp_floor`].
pub const RAMP_HEIGHT: f32 = RAMP_STRIPS as f32 * STRIP_HEIGHT;

/// The least space between two value labels beside a ramp, in logical points.
const LABEL_GAP: f32 = spacing::SPACE_1;

/// What a number legend draws in the room under its name.
#[derive(Debug, Clone, Copy, PartialEq)]
enum NumberFit {
    /// The ramp in `strips` strips, with the pivot's label at its middle when
    /// `pivot`: a diverging legend's, kept while it stands clear of both ends.
    Ramp { strips: usize, pivot: bool },
    /// No ramp; the two end labels alone.
    Labels,
}

/// The fewest strips a ramp is drawn in for value labels `label` points tall:
/// odd, so the middle strip is still the midpoint colour, and tall enough that
/// the maximum's label and the minimum's stand one over the other with
/// [`LABEL_GAP`] between them.
fn floor_strips(label: f32) -> usize {
    (((2.0 * label + LABEL_GAP) / STRIP_HEIGHT).ceil() as usize) | 1
}

/// The shortest a number legend's ramp is drawn, in logical points, for value
/// labels `label` points tall; a legend with less room than this draws the
/// labels alone. A ramp is drawn in whole strips, so this is a little over the
/// two labels' height and the gap between them.
#[must_use]
pub fn ramp_floor(label: f32) -> f32 {
    floor_strips(label) as f32 * STRIP_HEIGHT
}

/// What a number legend draws in `room` points under its name, for value labels
/// `label` points tall, with a pivot's label to place when `has_pivot`.
///
/// The ramp is as tall as the room allows up to [`RAMP_HEIGHT`], in whole odd
/// strips; under [`ramp_floor`] it is not drawn. The pivot's label stays while
/// the three labels stand [`LABEL_GAP`] apart along the ramp and is the first to
/// go when they cannot; the labels at its ends stay at the font's size.
fn number_fit(room: f32, label: f32, has_pivot: bool) -> NumberFit {
    let whole = ((room / STRIP_HEIGHT).floor() as usize).min(RAMP_STRIPS);
    let strips = if whole.is_multiple_of(2) {
        whole.saturating_sub(1)
    } else {
        whole
    };
    if strips < floor_strips(label) {
        return NumberFit::Labels;
    }
    let pivot = has_pivot && strips as f32 * STRIP_HEIGHT >= 3.0 * label + 2.0 * LABEL_GAP;
    NumberFit::Ramp { strips, pivot }
}

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
    strip_colours(stops, RAMP_STRIPS)
}

/// [`ramp_strip_colours`] for a ramp drawn in `strips` strips, `strips` at
/// least two.
fn strip_colours(stops: &[[f32; 4]], strips: usize) -> Vec<[f32; 4]> {
    (0..strips)
        .map(|i| ramp_at(stops, i as f64 / (strips - 1) as f64))
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

    /// A sequential scale derives a ramp legend with its domain ends.
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
            draw_block(
                &painter,
                egui::pos2(10.0, 10.0),
                200.0,
                &spec,
                None,
                Mode::Light,
            );
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

    /// A label height near the font's: the fit is a function of it, and the
    /// rooms below sweep it at half a point.
    const LABEL: f32 = 15.0;

    fn rooms() -> impl Iterator<Item = f32> {
        (0..=320).map(|i| i as f32 * 0.5)
    }

    /// **A room that holds the ramp draws it whole**, the pivot's label with it
    /// when there is one, so a chart taller than the block draws as it did.
    #[test]
    fn a_room_that_holds_the_ramp_draws_it_whole() {
        for room in [RAMP_HEIGHT, RAMP_HEIGHT + 0.5, 400.0] {
            assert_eq!(
                number_fit(room, LABEL, false),
                NumberFit::Ramp {
                    strips: RAMP_STRIPS,
                    pivot: false
                },
                "a sequential legend in {room}"
            );
            assert_eq!(
                number_fit(room, LABEL, true),
                NumberFit::Ramp {
                    strips: RAMP_STRIPS,
                    pivot: true
                },
                "a diverging legend in {room}"
            );
        }
    }

    /// **The ramp is never taller than its room, is always odd, and stands the
    /// labels apart.** A ramp of an odd count keeps its middle strip the
    /// midpoint; one no taller than its room is inside the band; and the two end
    /// labels, and the pivot's when it stays, do not meet.
    #[test]
    fn a_ramp_in_a_short_room_fits_it_and_clears_its_labels() {
        for room in rooms() {
            for has_pivot in [false, true] {
                let NumberFit::Ramp { strips, pivot } = number_fit(room, LABEL, has_pivot) else {
                    continue;
                };
                let height = strips as f32 * STRIP_HEIGHT;
                assert_eq!(strips % 2, 1, "{strips} strips in {room} is not odd");
                assert!(height <= room, "a ramp {height} tall in a room of {room}");
                assert!(
                    height >= 2.0 * LABEL + LABEL_GAP,
                    "the end labels meet on a ramp {height} tall"
                );
                assert!(
                    !pivot || height >= 3.0 * LABEL + 2.0 * LABEL_GAP,
                    "the pivot's label meets an end's on a ramp {height} tall"
                );
                assert!(has_pivot || !pivot, "a pivot's label for a sequential ramp");
            }
        }
    }

    /// **The pivot's label is the first to go, and the ramp the second.** Taking
    /// the room down from the block's height, the pivot leaves while the ramp is
    /// still drawn, and the ramp leaves at [`ramp_floor`] and not before; a room
    /// that has lost the ramp has not kept the pivot.
    #[test]
    fn the_pivot_goes_first_and_the_ramp_goes_at_its_floor() {
        let floor = ramp_floor(LABEL);
        let mut pivot_gone_at = None;
        for room in rooms().collect::<Vec<_>>().into_iter().rev() {
            match number_fit(room, LABEL, true) {
                NumberFit::Ramp { pivot: true, .. } => {
                    assert!(pivot_gone_at.is_none(), "the pivot is back at {room}");
                }
                NumberFit::Ramp { pivot: false, .. } => {
                    pivot_gone_at.get_or_insert(room);
                    assert!(room >= floor, "a ramp in {room}, under its floor {floor}");
                }
                NumberFit::Labels => {
                    assert!(
                        pivot_gone_at.is_some(),
                        "the ramp went at {room} with the pivot still on it"
                    );
                    assert!(room < floor, "labels alone in {room}, the floor is {floor}");
                }
            }
        }
        let gone = pivot_gone_at.expect("a room too short for the pivot, with a ramp");
        assert!(
            gone > floor,
            "the pivot went at {gone}, with the ramp at {floor}"
        );
        assert_eq!(
            number_fit(floor, LABEL, false),
            NumberFit::Ramp {
                strips: floor_strips(LABEL),
                pivot: false
            },
            "the floor itself draws the ramp"
        );
        assert_eq!(
            number_fit(floor - 0.5, LABEL, false),
            NumberFit::Labels,
            "under the floor draws the labels alone"
        );
    }
}
