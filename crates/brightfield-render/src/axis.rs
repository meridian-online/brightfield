//! Axis rendering — tick computation, tick marks, labels, and axis lines.
//!
//! Tick computation is a pure function: `compute_ticks(scale, target_count) ->
//! Vec<Tick>`. The scene builder draws ticks as lines and labels as text.

use brightfield_spec::date_format::{iso_date_micros, DateFormat};
use brightfield_spec::layout::AxisFormat;
use brightfield_spec::number_format::{NumberFormat, TickFormat};
use kurbo::{Affine, Line, Point, Rect};
use vello::Scene;

use crate::ink::ChartInk;
use crate::layout::ChartLayout;
use crate::scale::Scale;
use crate::text::{
    draw_text, draw_text_rotated, measure_width, TextAnchor, LABEL_SIZE, TITLE_SIZE,
};

/// A computed tick mark with its position and label.
#[derive(Debug, Clone)]
pub struct Tick {
    /// The data value this tick represents.
    pub value: f64,
    /// Human-readable label string.
    pub label: String,
    /// Pixel position along the axis.
    pub position: f64,
}

/// How far an axis line's target reaches into the data area, in pixels. The
/// 5 px (`TICK_LENGTH`) between an x axis and its labels are too few to hit with
/// a pointer, so the line's strip takes this much of the plot above
/// it. The reach is a place a pointer may be read and draws nothing.
pub const LINE_REACH: f64 = 8.0;

/// How far below its baseline a run of text hangs, as a fraction of its size.
const DESCENT: f64 = 0.25;

/// **The three parts of one axis a pointer can land on**, each a rect in the
/// plot's own scene, whose origin is the tile's top-left corner.
///
/// Reported by the code that draws the axis, from the labels it actually drew:
/// a label thinned away, dropped to fit the tile or rotated is the extent it was
/// drawn at, and a rect restated elsewhere could not follow it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisTargets {
    /// The title's text box. `None` when the axis drew no title.
    pub title: Option<Rect>,
    /// The tick labels' strip: the union of the labels drawn. `None` when the
    /// axis drew no label.
    pub labels: Option<Rect>,
    /// The axis line with its tick marks, reaching [`LINE_REACH`] into the data
    /// area and ending where the labels begin.
    pub line: Rect,
}

/// What a plot's axes reported, one per positional channel it draws.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlotAxes {
    /// The x axis's parts. `None` when the plot draws no x axis.
    pub x: Option<AxisTargets>,
    /// The y axis's parts. `None` when the plot draws no y axis.
    pub y: Option<AxisTargets>,
}

/// `rect` joined to what `into` already covers.
fn cover(into: &mut Option<Rect>, rect: Rect) {
    *into = Some(into.map_or(rect, |held| held.union(rect)));
}

// The tick and axis inks are [`ChartInk::tick`] and [`ChartInk::axis`] — the
// mode's baseline ink. Recessive axes: the domain line and ticks sit back while
// the data ink carries the chart; tick-label TEXT stays legible via the muted
// ink ([`ChartInk::label`]), which is a step closer to the primary.

/// Tick mark length in pixels.
const TICK_LENGTH: f64 = 5.0;

/// Gap (px) between the tick-label band and an axis / plot title baseline.
const TITLE_GAP: f64 = 4.0;

/// The y-axis title baseline's x, measured from the plot's left window edge: it
/// sits in the leftmost grown-margin band, left of the (right-aligned) tick
/// labels. Fixed-band placement — a pathologically wide tick label is the
/// recorded measured-fit deferral, not handled here.
pub(crate) const Y_TITLE_X: f64 = 12.0;

/// Baseline y for the x-axis title — below the tick-label band, inside the
/// (grown) bottom margin. Exposed so the tick-clearance test can pin it.
pub(crate) fn x_title_baseline(layout: &ChartLayout) -> f64 {
    layout.plot_y_end() + TICK_LENGTH + f64::from(LABEL_SIZE) + f64::from(TITLE_SIZE) + TITLE_GAP
}

/// Baseline y for the plot title — above the frame, inside the (grown) top
/// margin, never above the window top edge.
pub(crate) fn plot_title_baseline(layout: &ChartLayout) -> f64 {
    (layout.plot_y_start() - TITLE_GAP).max(f64::from(TITLE_SIZE))
}

/// Render a per-plot title above the frame, left-aligned at the frame's left
/// edge (Observable Plot parity). Called only when the plot declares a title
/// (so the top margin has grown to make room).
pub fn render_plot_title(scene: &mut Scene, layout: &ChartLayout, title: &str, ink: ChartInk) {
    draw_text(
        scene,
        title,
        layout.plot_x_start(),
        plot_title_baseline(layout),
        TITLE_SIZE,
        ink.title,
        TextAnchor::Start,
    );
}

/// Compute ticks for a scale.
///
/// Returns tick marks with positions and labels appropriate for the scale type.
pub fn compute_ticks(scale: &Scale, target_count: usize) -> Vec<Tick> {
    compute_ticks_formatted(scale, target_count, None)
}

/// The kind of text an axis prints, which decides what a plot's `xTickFormat` /
/// `yTickFormat` can be for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisKind {
    /// A linear, log or symlog axis: numbers, and a d3-format specifier.
    Number,
    /// A timestamp axis, or a band of calendar days: dates, and a
    /// d3-time-format specifier.
    Date,
    /// A band of names. It takes no format, and a plot that sets one is told so
    /// ([`tick_format_applies`]).
    Category,
}

impl AxisKind {
    /// The word the warning banner uses for this kind.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Date => "date",
            Self::Category => "category",
        }
    }
}

/// The kind of axis a scale draws, or `None` for a colour ramp, which draws no
/// positional axis.
///
/// A `DATE` column takes a band scale whose categories are its days spelled
/// `YYYY-MM-DD`, and this build has no other record of that: a band is a date
/// axis when each of its categories is such a day and a band of names when one
/// is not, as `a_format_crosses_an_axis_of_the_other_kind_and_no_other` holds, so
/// a text column of ISO dates reads as a date axis too.
#[must_use]
pub fn axis_kind(scale: &Scale) -> Option<AxisKind> {
    match scale {
        Scale::Linear { .. } | Scale::Log { .. } | Scale::Symlog { .. } => Some(AxisKind::Number),
        Scale::Time { .. } => Some(AxisKind::Date),
        Scale::Band { categories, .. } => Some(if day_categories(categories).is_some() {
            AxisKind::Date
        } else {
            AxisKind::Category
        }),
        Scale::Colour { .. }
        | Scale::Sequential { .. }
        | Scale::Diverging { .. }
        | Scale::Quantized { .. } => None,
    }
}

/// The word the warning banner uses for the axis a scale draws, finer than
/// [`AxisKind::word`]: a linear, a log and a symlog axis read as a number axis
/// there, and an instruction that one takes and another does not has to name
/// which it met. `None` for a colour ramp, which draws no positional axis.
#[must_use]
pub fn axis_scale_word(scale: &Scale) -> Option<&'static str> {
    match scale {
        Scale::Linear { .. } => Some("linear"),
        Scale::Log { .. } => Some("log"),
        Scale::Symlog { .. } => Some("symlog"),
        Scale::Time { .. }
        | Scale::Band { .. }
        | Scale::Colour { .. }
        | Scale::Sequential { .. }
        | Scale::Diverging { .. }
        | Scale::Quantized { .. } => axis_kind(scale).map(AxisKind::word),
    }
}

/// Whether the axis `scale` draws aims its ticks at the count a plot's `xTicks`
/// / `yTicks` asks for. A linear and a time axis step toward it; a band has a
/// tick per category and a log or symlog axis a tick per decade, so a count
/// reaches none of them.
///
/// It is the one judge the composition warns through, and
/// `tick_count_applies_where_the_ticks_follow_the_count` holds it to
/// [`compute_ticks_formatted`] by drawing each kind of scale at two counts.
#[must_use]
pub fn tick_count_applies(scale: &Scale) -> bool {
    matches!(scale, Scale::Linear { .. } | Scale::Time { .. })
}

/// The instant each category names, when every category is a calendar day.
fn day_categories(categories: &[String]) -> Option<Vec<i64>> {
    if categories.is_empty() {
        return None;
    }
    categories.iter().map(|c| iso_date_micros(c)).collect()
}

/// Whether the axis `scale` draws has text a plot's `xTickFormat` /
/// `yTickFormat` can set: a number axis (linear, log or symlog) and a date axis
/// (a time scale, or a band of calendar days). A band of names prints its names,
/// whatever format is asked of it and whether or not the names read as numbers,
/// so a format on one changes nothing, and Mosaic's own band axis prints the
/// names too.
///
/// The composition warns through it, as through [`tick_count_applies`], and
/// `tick_format_applies_where_the_ticks_follow_a_format` holds it to
/// [`compute_ticks_formatted`] by drawing each kind of scale under a number
/// format and a date format.
#[must_use]
pub fn tick_format_applies(scale: &Scale) -> bool {
    matches!(axis_kind(scale), Some(AxisKind::Number | AxisKind::Date))
}

/// Whether `format` is a kind the axis `scale` draws cannot take: a number
/// format on a date axis, or a date format on a number axis. It is the one judge
/// the axis draws through and the composition warns through, so a format the axis
/// drops is a format that was named. An axis of names takes a format of neither
/// kind and is named through [`tick_format_applies`] instead, which says the
/// format has no text to set there rather than that it is the other kind; a
/// colour ramp draws no positional axis, so the question is not asked of it.
#[must_use]
pub fn tick_format_crosses_axis(scale: &Scale, format: &AxisFormat) -> bool {
    matches!(
        (axis_kind(scale), format),
        (Some(AxisKind::Date), AxisFormat::Number(_))
            | (Some(AxisKind::Number), AxisFormat::Date(_))
    )
}

/// [`compute_ticks`], with the tick text a plot's `xTickFormat` / `yTickFormat`
/// asked for.
///
/// A number format sets the text of a number axis, linear, log or symlog: a
/// linear axis takes d3-scale's precision from the step its ticks are drawn at,
/// and a log or symlog axis prints each decade as d3-format does. A date format
/// sets the text of a date axis, a timestamp's ticks or a date column's days,
/// printed in UTC. A format of the other kind, a band of names, and `None` draw
/// the text an axis drew before a format could be asked for. The tick POSITIONS
/// never follow a format: a timestamp axis still steps in 1, 2 or 5 times a power
/// of ten microseconds and a date column still has a tick per day.
pub fn compute_ticks_formatted(
    scale: &Scale,
    target_count: usize,
    format: Option<&AxisFormat>,
) -> Vec<Tick> {
    let number = match format {
        Some(AxisFormat::Number(number)) => Some(*number),
        _ => None,
    };
    let date = match format {
        Some(AxisFormat::Date(date)) => Some(date),
        _ => None,
    };
    match scale {
        Scale::Linear {
            domain_min,
            domain_max,
            range_start,
            range_end,
        } => compute_linear_ticks(
            *domain_min,
            *domain_max,
            *range_start,
            *range_end,
            target_count,
            number,
        ),
        Scale::Band {
            categories,
            range_start,
            range_end,
            padding,
        } => compute_band_ticks(categories, *range_start, *range_end, *padding, date),
        Scale::Time {
            domain_min_us,
            domain_max_us,
            range_start,
            range_end,
        } => compute_time_ticks(
            *domain_min_us,
            *domain_max_us,
            *range_start,
            *range_end,
            target_count,
            date,
        ),
        // A log axis's ticks are the DECADES, not a nice decimal step: the
        // whole point of the transform is that equal pixel distances are equal
        // ratios, and a 1/2/5 step read off the data extent labels a line at
        // the wrong place for that. `nice_step` is deliberately not reachable
        // from here.
        Scale::Log {
            domain_min,
            domain_max,
            ..
        } => positioned(
            scale,
            &log_tick_values(*domain_min, *domain_max),
            number.map(NumberFormat::decade_format),
        ),
        // Symlog's ticks are SIGNED decades with zero among them — the choice
        // recorded for this build. Zero is the value the transform exists to
        // keep, so an axis that could not label it would be hiding the reason
        // it was chosen over log.
        Scale::Symlog {
            domain_min,
            domain_max,
            ..
        } => positioned(
            scale,
            &symlog_tick_values(*domain_min, *domain_max),
            number.map(NumberFormat::decade_format),
        ),
        // Colour ramps (categorical, sequential or stepped) have no positional axis ticks.
        Scale::Colour { .. }
        | Scale::Sequential { .. }
        | Scale::Diverging { .. }
        | Scale::Quantized { .. } => Vec::new(),
    }
}

/// **The text of the largest tick the axis `scale` draws under `format`**: the
/// sample a settings row shows beside the format it names, so the analyst reads
/// what the axis prints before keeping it.
///
/// It is the tick [`compute_ticks_formatted`] returns with the largest value, so
/// it is the axis's own top tick (a number axis with a domain to 5,565 draws its
/// top tick at 5,000, and that is what this reads, not the domain's end), and a
/// target count or a format the plot changes moves it as it moves the axis.
/// `None` where the axis draws no text a number or date format sets (an axis of
/// names, a colour ramp), where `format` is a kind the axis drops
/// ([`tick_format_crosses_axis`]), and where the axis has no ticks.
#[must_use]
pub fn top_tick_text(
    scale: &Scale,
    target_count: usize,
    format: Option<&AxisFormat>,
) -> Option<String> {
    if !tick_format_applies(scale) || format.is_some_and(|f| tick_format_crosses_axis(scale, f)) {
        return None;
    }
    compute_ticks_formatted(scale, target_count, format)
        .into_iter()
        .max_by(|a, b| a.value.total_cmp(&b.value))
        .map(|tick| tick.label)
}

/// Turn tick VALUES into ticks, placing each one through the scale itself so
/// the label and the bar it stands under cannot be positioned by two different
/// rules. `text` is the format the plot asked for, if it asked.
fn positioned(scale: &Scale, values: &[f64], text: Option<TickFormat>) -> Vec<Tick> {
    values
        .iter()
        .map(|value| Tick {
            value: *value,
            label: tick_text(text.as_ref(), *value),
            position: scale.map_f64(*value),
        })
        .collect()
}

/// The text of the tick at `value`: the plot's format when it asked for one,
/// and the axis's own text when it did not.
fn tick_text(text: Option<&TickFormat>, value: f64) -> String {
    text.map_or_else(|| format_number(value), |format| format.format(value))
}

/// The powers of ten inside `[lo, hi]`.
///
/// Under two decades that list is one label or none, which is not an axis; the
/// fallback subdivides each decade at 1, 2 and 5, which is `d3.scaleLog`'s own
/// treatment of a short domain.
fn log_tick_values(lo: f64, hi: f64) -> Vec<f64> {
    let lo = if lo > 0.0 { lo } else { f64::MIN_POSITIVE };
    if hi <= lo {
        return Vec::new();
    }
    let decades: Vec<f64> = (lo.log10().ceil() as i32..=hi.log10().floor() as i32)
        .map(|e| 10_f64.powi(e))
        .collect();
    if decades.len() >= 2 {
        return decades;
    }
    let mut out = Vec::new();
    for e in lo.log10().floor() as i32..=hi.log10().ceil() as i32 {
        for mantissa in [1.0, 2.0, 5.0] {
            let v = mantissa * 10_f64.powi(e);
            if v >= lo && v <= hi {
                out.push(v);
            }
        }
    }
    out
}

/// Zero and the signed powers of ten inside `[lo, hi]`, ascending.
fn symlog_tick_values(lo: f64, hi: f64) -> Vec<f64> {
    if hi <= lo {
        return Vec::new();
    }
    let reach = lo.abs().max(hi.abs());
    let top = if reach >= 1.0 {
        reach.log10().floor() as i32
    } else {
        0
    };
    let mut out: Vec<f64> = Vec::new();
    for e in (0..=top).rev() {
        out.push(-10_f64.powi(e));
    }
    out.push(0.0);
    for e in 0..=top {
        out.push(10_f64.powi(e));
    }
    out.retain(|v| *v >= lo && *v <= hi);
    out
}

fn compute_linear_ticks(
    domain_min: f64,
    domain_max: f64,
    range_start: f64,
    range_end: f64,
    target_count: usize,
    format: Option<NumberFormat>,
) -> Vec<Tick> {
    let span = domain_max - domain_min;
    if span.abs() < f64::EPSILON || target_count == 0 {
        return vec![];
    }

    let step = nice_step(span, target_count);
    // The precision follows the step these ticks are DRAWN at, so the text can
    // never carry a digit the ticks do not differ by, nor lose one they do.
    let text = format.map(|f| f.tick_format(domain_min, domain_max, step));
    let first = (domain_min / step).ceil() * step;

    let mut ticks = Vec::new();
    let mut value = first;
    while value <= domain_max + step * 0.001 {
        let t = (value - domain_min) / span;
        let position = range_start + t * (range_end - range_start);
        let label = tick_text(text.as_ref(), value);
        ticks.push(Tick {
            value,
            label,
            position,
        });
        value += step;
    }
    ticks
}

/// A band's ticks: one per category, at its centre. `date` is a date format
/// the plot asked for. It sets the text when each category is a calendar day
/// (see [`axis_kind`]), and the categories print as they are when one is not,
/// as `a_date_format_prints_a_time_axis_and_a_band_of_days_and_leaves_names_alone`
/// holds.
fn compute_band_ticks(
    categories: &[String],
    range_start: f64,
    range_end: f64,
    padding: f64,
    date: Option<&DateFormat>,
) -> Vec<Tick> {
    let n = categories.len() as f64;
    if n == 0.0 {
        return vec![];
    }
    let total = range_end - range_start;
    let band = total / n;
    let days = date.and_then(|format| day_categories(categories).map(|days| (format, days)));

    categories
        .iter()
        .enumerate()
        .map(|(i, cat)| {
            let centre = range_start
                + band * (padding / 2.0)
                + band * i as f64
                + band * (1.0 - padding) / 2.0;
            Tick {
                value: i as f64,
                label: days
                    .as_ref()
                    .map_or_else(|| cat.clone(), |(format, days)| format.format(days[i])),
                position: centre,
            }
        })
        .collect()
}

fn compute_time_ticks(
    domain_min_us: i64,
    domain_max_us: i64,
    range_start: f64,
    range_end: f64,
    target_count: usize,
    date: Option<&DateFormat>,
) -> Vec<Tick> {
    let span_us = (domain_max_us - domain_min_us) as f64;
    if span_us.abs() < f64::EPSILON || target_count == 0 {
        return vec![];
    }

    let step_us = nice_step(span_us, target_count);
    let first = ((domain_min_us as f64 / step_us).ceil() * step_us) as i64;

    let mut ticks = Vec::new();
    let mut value_us = first;
    while value_us <= domain_max_us {
        let t = (value_us - domain_min_us) as f64 / span_us;
        let position = range_start + t * (range_end - range_start);
        // A plot's date format prints the instant; with none, seconds since the
        // epoch, which is all a timestamp axis drew before a format could be asked.
        let label = date.map_or_else(
            || format!("{:.1}s", value_us as f64 / 1_000_000.0),
            |format| format.format(value_us),
        );
        ticks.push(Tick {
            value: value_us as f64,
            label,
            position,
        });
        value_us += step_us as i64;
    }
    ticks
}

/// Compute a "nice" step size for tick spacing.
fn nice_step(span: f64, target_count: usize) -> f64 {
    let raw_step = span / target_count as f64;
    let magnitude = 10_f64.powf(raw_step.log10().floor());
    let residual = raw_step / magnitude;

    let nice = if residual <= 1.5 {
        1.0
    } else if residual <= 3.5 {
        2.0
    } else if residual <= 7.5 {
        5.0
    } else {
        10.0
    };

    nice * magnitude
}

/// The most times [`nice_linear_domain`] widens a domain before it stops.
/// A domain settles when widening it no longer changes the step; the cap bounds
/// the loop if a pair of ends never does, and is the cap d3's `scale.nice` puts
/// on its own.
const MAX_NICE_PASSES: usize = 10;

/// Round `x` outward to a multiple of `step`: up when `up`, else down.
///
/// A step is 1, 2 or 5 times a power of ten, so one below 1 is the reciprocal of
/// a whole number and its multiples are divided out of an integer rather than
/// built by repeated addition. A value already on a multiple is left there: the
/// quotient can land a few ulps off the whole number it should be, and rounding
/// that outward would move the end a whole step.
fn snap_to_step(x: f64, step: f64, up: bool) -> f64 {
    let inverse = (step < 1.0).then(|| (1.0 / step).round());
    let scaled = match inverse {
        Some(inverse) => x * inverse,
        None => x / step,
    };
    let nearest = scaled.round();
    let whole = if (scaled - nearest).abs() <= 1e-9 * nearest.abs().max(1.0) {
        nearest
    } else if up {
        scaled.ceil()
    } else {
        scaled.floor()
    };
    // `+ 0.0` turns a negative zero into zero, so an axis that starts at 0 does
    // not carry a sign.
    let snapped = match inverse {
        Some(inverse) => whole / inverse,
        None => whole * step,
    };
    snapped + 0.0
}

/// The linear domain `[min, max]` widened outward to round ends, as a plot's
/// `xNice` / `yNice` asks.
///
/// The ends are multiples of the step [`compute_linear_ticks`] draws its ticks
/// at for `target_count`, so each end of the axis is a tick the analyst can read
/// off: the top one is a number they can say aloud. Observable Plot rounds to
/// d3's default of ten ticks whatever the axis draws, and that step can differ
/// from the one the axis ticks at, which would leave an end between two ticks.
///
/// Widened until the step settles, so the result is a fixed point: asking again
/// of a domain this returned gives it back. That is what lets a domain a plot
/// pinned after rounding be rounded again on every later composition without
/// moving.
///
/// A span with no width, a count of zero and an end that is not a finite number
/// are returned as they came, for the reason [`compute_linear_ticks`] draws no
/// ticks on them.
pub(crate) fn nice_linear_domain(min: f64, max: f64, target_count: usize) -> (f64, f64) {
    if !min.is_finite() || !max.is_finite() || (max - min).abs() < f64::EPSILON || target_count == 0
    {
        return (min, max);
    }
    let (mut lo, mut hi) = (min, max);
    for _ in 0..MAX_NICE_PASSES {
        let step = nice_step(hi - lo, target_count);
        let (next_lo, next_hi) = (snap_to_step(lo, step, false), snap_to_step(hi, step, true));
        if next_lo == lo && next_hi == hi {
            break;
        }
        (lo, hi) = (next_lo, next_hi);
    }
    (lo, hi)
}

/// Format a number for tick labels.
pub(crate) fn format_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// Horizontal clearance a drawn tick label must keep from its neighbour's, in
/// pixels — [`labels_clear_horizontally`]'s threshold. Small on purpose: it is
/// not a design gap, it is the minimum that keeps two adjacent digits from
/// reading as one run of glyphs.
const LABEL_CLEARANCE: f64 = 4.0;

/// The vertical gap between a tick mark and a rotated label's near end, in
/// pixels — the rotation fallback in `render_x_axis` anchors the label's last
/// character here rather than at the horizontal band's baseline, since a
/// rotated run's own length is what needs the room a fixed baseline assumes a
/// horizontal one has.
const ROTATED_LABEL_GAP: f64 = 3.0;

/// The vertical room, in pixels, a rotated label's run has below the tick line
/// before it reaches whichever floor this axis actually reserves: the x-title
/// text's own top edge when `titled`, the tile's bottom edge otherwise.
///
/// Read off `layout` rather than assumed, because the floor is not a fixed
/// distance from the tile's bottom: `x_title_baseline` sits a FIXED offset
/// below `plot_y_end()` regardless of how large the margin is, so a titled
/// axis's room does not grow with the margin the way an untitled one's does —
/// see `the_axis_degrades_to_one_label_rather_than_clip_a_rotated_band_past_a_title`
/// (this module), which measures that floor rather than restating it.
fn rotated_label_room(layout: &ChartLayout, titled: bool) -> f64 {
    let near_end = layout.plot_y_end() + TICK_LENGTH + ROTATED_LABEL_GAP;
    let floor = if titled {
        x_title_baseline(layout) - f64::from(TITLE_SIZE) - TITLE_GAP
    } else {
        layout.height
    };
    (floor - near_end).max(0.0)
}

/// The horizontal centre a `width`-wide label drawn with `TextAnchor::Middle`
/// at `position` is nudged to so both its drawn edges stay inside the tile's
/// own `[0, tile_width]` span — [`ChartLayout::width`], the tile's full
/// extent, not the inset-adjusted x-range a mark's own scale places its ticks
/// in. A label draws in the tile's margin band below the plot area, so the
/// plot's inner range is the wrong bound to clamp against: a label near the
/// plot's own edge can already read as inside THAT narrower bound while its
/// glyphs still run past the tile the frame actually clips to — a real date
/// sliced at the tile's right edge at a real window width is what this
/// closes.
///
/// `None` when `width` on its own exceeds `tile_width`: no centre keeps both
/// edges inside a span narrower than the label itself, so the caller drops
/// the label rather than draw one that overflows regardless of where it is
/// placed — a single date wider than the whole tile, at the narrowest
/// widths the live layout still resolves a column tile to, is reachable and
/// is exactly this case. A dropped tick still draws its tick MARK
/// (`render_x_axis` draws marks independently of labels); only the text is
/// withheld.
///
/// A rotated label's footprint is [`LABEL_SIZE`] wide regardless of its text
/// length (its own glyph-height run turns crosswise on rotation), so
/// `render_x_axis`'s rotated branch calls this with that constant rather than
/// with a measured text width — same function, a different `width`.
fn contained_centre(position: f64, width: f64, tile_width: f64) -> Option<f64> {
    if width > tile_width {
        return None;
    }
    let half = width / 2.0;
    Some(position.clamp(half, tile_width - half))
}

/// Whether the labels in `ticks` — each centred (`TextAnchor::Middle`) at its
/// own tick's `position` at `size`, nudged by [`contained_centre`] to
/// `tile_width` the way `render_x_axis` actually draws them — clear
/// [`LABEL_CLEARANCE`] of their drawn neighbour's. A label that
/// [`contained_centre`] drops draws no pixel, so it clears trivially — it
/// cannot collide with a neighbour it shares no pixel with.
///
/// Sorted by drawn span start rather than read pairwise off `ticks`' own
/// order: the clamp can pull an end label in far enough that its nearest
/// drawn neighbour is no longer the tick next to it in `ticks`.
fn labels_clear_horizontally(ticks: &[&Tick], size: f32, tile_width: f64) -> bool {
    let mut spans: Vec<(f64, f64)> = ticks
        .iter()
        .filter_map(|t| {
            let width = measure_width(&t.label, size);
            let centre = contained_centre(t.position, width, tile_width)?;
            Some((centre - width / 2.0, centre + width / 2.0))
        })
        .collect();
    if spans.len() < 2 {
        return true;
    }
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    spans
        .windows(2)
        .all(|pair| pair[1].0 - pair[0].1 >= LABEL_CLEARANCE)
}

/// The widest evenly-strided subset of `ticks` whose labels clear each other
/// horizontally ([`labels_clear_horizontally`], itself bounded to
/// `tile_width`) at `size` — Observable Plot's own answer to a crowded axis:
/// try drawing each label, then try dropping alternating ones, then try a
/// wider stride still, and so on until a stride clears or the search has
/// narrowed to the two end ticks. `render_x_axis` rotates the full set
/// instead of drawing this candidate when even that pair collides AND there
/// is room to rotate into; it degrades past this candidate instead when there
/// is not — see [`rotated_label_room`].
fn thinned_x_ticks(ticks: &[Tick], size: f32, tile_width: f64) -> Vec<&Tick> {
    if ticks.len() < 2 {
        return ticks.iter().collect();
    }
    for stride in 1..ticks.len() {
        let subset: Vec<&Tick> = ticks.iter().step_by(stride).collect();
        if labels_clear_horizontally(&subset, size, tile_width) {
            return subset;
        }
    }
    vec![&ticks[0], &ticks[ticks.len() - 1]]
}

/// Render the x-axis into the scene. `title`, when `Some`, is drawn centred
/// below the tick-label band (the bottom margin has grown to make room).
///
/// A tick mark draws for each tick regardless of what its label does —
/// dropping one would misstate which values the axis carries. A tick label
/// draws thinned first (`thinned_x_ticks`, private to this module); when even
/// the sparsest horizontal set still collides, it rotates a quarter turn
/// ([`draw_text_rotated`]) IF `rotated_label_room` (private to this module)
/// says the run fits below the tick line, and degrades past the two end
/// labels to a single one
/// otherwise — a rotated run that does not fit reads as clipped digits under
/// the title rather than as an axis, which is worse than one label. This
/// module's tests pin these branches:
/// `thinning_keeps_labels_from_touching_at_various_widths`,
/// `rotation_is_the_fallback_when_thinning_cannot_clear_the_labels_and_there_is_room_to_rotate`
/// and
/// `the_axis_degrades_to_one_label_rather_than_clip_a_rotated_band_past_a_title`.
///
/// `contained_centre` (private to this module) additionally nudges a label
/// whose own drawn footprint would run past the tile's `[0, layout.width]`
/// span back inside it, or drops it when even nudging cannot fit it — applied
/// to the thinned candidate, the rotated band and the degraded single label
/// alike, so a branch above cannot hand back a rect the tile does not hold. A
/// real composed tile's own width is exercised one crate up, in the
/// brightfield-shell crate's dashboard_baseline.rs test suite, since this
/// crate carries no dependency on that composition;
/// `label_clearance_rejects_a_gap_narrower_than_the_minimum` (this module)
/// pins the neighbour-clearance floor the same nudge must not erase.
///
/// Returns the axis's [`AxisTargets`]: the title's text box, the strip the
/// labels drawn cover and the axis line's strip, each read off what this call
/// drew.
pub fn render_x_axis(
    scene: &mut Scene,
    layout: &ChartLayout,
    ticks: &[Tick],
    title: Option<&str>,
    ink: ChartInk,
) -> AxisTargets {
    let y = layout.plot_y_end();
    let tile_width = layout.width;
    let stroke = kurbo::Stroke::new(1.0);
    let mut labels: Option<Rect> = None;
    // A horizontal run of tick text, centred on `centre`, that hangs from the
    // band's top edge (the tick marks' end).
    let horizontal = |labels: &mut Option<Rect>, centre: f64, width: f64| {
        cover(
            labels,
            Rect::new(
                centre - width / 2.0,
                y + TICK_LENGTH,
                centre + width / 2.0,
                y + TICK_LENGTH + f64::from(LABEL_SIZE) * (1.0 + DESCENT),
            ),
        );
    };

    // Axis line.
    let axis_line = Line::new(
        Point::new(layout.plot_x_start(), y),
        Point::new(layout.plot_x_end(), y),
    );
    scene.stroke(&stroke, Affine::IDENTITY, ink.axis, None, &axis_line);

    // Tick marks, independent of which labels draw.
    for tick in ticks {
        let tick_line = Line::new(
            Point::new(tick.position, y),
            Point::new(tick.position, y + TICK_LENGTH),
        );
        scene.stroke(&stroke, Affine::IDENTITY, ink.tick, None, &tick_line);
    }

    // Tick labels: thin before rotating.
    let thinned = thinned_x_ticks(ticks, LABEL_SIZE, tile_width);
    if labels_clear_horizontally(&thinned, LABEL_SIZE, tile_width) {
        for tick in thinned {
            let width = measure_width(&tick.label, LABEL_SIZE);
            let Some(centre) = contained_centre(tick.position, width, tile_width) else {
                // Wider than the tile on its own: no centre keeps both
                // edges inside it, so this label is dropped rather than
                // drawn overflowing. The tick mark above already drew.
                continue;
            };
            draw_text(
                scene,
                &tick.label,
                centre,
                y + TICK_LENGTH + f64::from(LABEL_SIZE),
                LABEL_SIZE,
                ink.label,
                TextAnchor::Middle,
            );
            if !tick.label.is_empty() {
                horizontal(&mut labels, centre, width);
            }
        }
    } else {
        // Even the two end labels collide horizontally. Rotating helps when
        // the widest label's run actually fits the room below the tick line
        // — `rotated_label_room` reads that off `layout`, and a titled
        // axis's room is a small, FIXED distance (the title sits a constant
        // offset below the tick line no matter how large the margin is), so
        // this is not a check that more margin alone can satisfy.
        let widest = ticks
            .iter()
            .map(|t| measure_width(&t.label, LABEL_SIZE))
            .fold(0.0_f64, f64::max);
        if widest <= rotated_label_room(layout, title.is_some()) {
            // Rotate the full set a quarter turn so each label's OWN
            // footprint is its font size rather than its text width,
            // anchored (`TextAnchor::End`) so the label's last character
            // sits nearest the tick and the rest reaches down into the
            // margin instead of up into the plot. The rotated footprint is
            // `LABEL_SIZE` wide regardless of the text it carries, so it is
            // the pivot itself — not a measured text width — that
            // `contained_centre` nudges here; `LABEL_SIZE` fits comfortably
            // inside any tile this axis draws into in practice, so the drop
            // branch is defensive rather than reachable today.
            for tick in ticks {
                let Some(pivot) =
                    contained_centre(tick.position, f64::from(LABEL_SIZE), tile_width)
                else {
                    continue;
                };
                draw_text_rotated(
                    scene,
                    &tick.label,
                    pivot,
                    y + TICK_LENGTH + ROTATED_LABEL_GAP,
                    LABEL_SIZE,
                    ink.label,
                    TextAnchor::End,
                );
                if !tick.label.is_empty() {
                    // The run reads upward from its pivot, ascent to the
                    // pivot's left and descent to its right.
                    let near = y + TICK_LENGTH + ROTATED_LABEL_GAP;
                    cover(
                        &mut labels,
                        Rect::new(
                            pivot - f64::from(LABEL_SIZE),
                            near,
                            pivot + f64::from(LABEL_SIZE) * DESCENT,
                            near + measure_width(&tick.label, LABEL_SIZE),
                        ),
                    );
                }
            }
        } else {
            // No room to rotate into without running past the tile's own
            // bottom edge or under the title: degrade past the two end
            // labels `thinned_x_ticks` stopped at to the single label
            // nearest the domain's start, on the SAME horizontal baseline
            // the thinned case draws at. One label cannot collide with
            // itself, and that baseline is already proven clear of a title —
            // `axis_titles_render_and_clear_tick_labels`, this module.
            let solo = &ticks[0];
            let width = measure_width(&solo.label, LABEL_SIZE);
            if let Some(centre) = contained_centre(solo.position, width, tile_width) {
                draw_text(
                    scene,
                    &solo.label,
                    centre,
                    y + TICK_LENGTH + f64::from(LABEL_SIZE),
                    LABEL_SIZE,
                    ink.label,
                    TextAnchor::Middle,
                );
                if !solo.label.is_empty() {
                    horizontal(&mut labels, centre, width);
                }
            }
            // Wider than the tile on its own: dropped, same as the thinned
            // branch above — the tick marks and (when present) the title
            // still draw.
        }
    }

    // Axis title, centred below the tick-label band.
    let mut title_box = None;
    if let Some(title) = title {
        let centre = (layout.plot_x_start() + layout.plot_x_end()) / 2.0;
        let baseline = x_title_baseline(layout);
        draw_text(
            scene,
            title,
            centre,
            baseline,
            TITLE_SIZE,
            ink.title,
            TextAnchor::Middle,
        );
        if !title.is_empty() {
            let half = measure_width(title, TITLE_SIZE) / 2.0;
            title_box = Some(Rect::new(
                centre - half,
                baseline - f64::from(TITLE_SIZE),
                centre + half,
                baseline + f64::from(TITLE_SIZE) * DESCENT,
            ));
        }
    }

    AxisTargets {
        title: title_box,
        labels,
        line: Rect::new(
            layout.plot_x_start(),
            y - LINE_REACH,
            layout.plot_x_end(),
            y + TICK_LENGTH,
        ),
    }
}

/// Render the y-axis into the scene. `title`, when `Some`, is drawn rotated a
/// quarter-turn up the (grown) left margin, left of the tick labels.
///
/// Returns the axis's [`AxisTargets`], read off what this call drew.
pub fn render_y_axis(
    scene: &mut Scene,
    layout: &ChartLayout,
    ticks: &[Tick],
    title: Option<&str>,
    ink: ChartInk,
) -> AxisTargets {
    let x = layout.plot_x_start();
    let stroke = kurbo::Stroke::new(1.0);
    let mut labels: Option<Rect> = None;

    // Axis line.
    let axis_line = Line::new(
        Point::new(x, layout.plot_y_start()),
        Point::new(x, layout.plot_y_end()),
    );
    scene.stroke(&stroke, Affine::IDENTITY, ink.axis, None, &axis_line);

    // Tick marks and labels.
    for tick in ticks {
        let tick_line = Line::new(
            Point::new(x - TICK_LENGTH, tick.position),
            Point::new(x, tick.position),
        );
        scene.stroke(&stroke, Affine::IDENTITY, ink.tick, None, &tick_line);

        // Label, right-aligned in the left margin and vertically centred on the tick.
        let right = x - TICK_LENGTH - 3.0;
        let baseline = tick.position + f64::from(LABEL_SIZE) / 3.0;
        draw_text(
            scene,
            &tick.label,
            right,
            baseline,
            LABEL_SIZE,
            ink.label,
            TextAnchor::End,
        );
        if !tick.label.is_empty() {
            cover(
                &mut labels,
                Rect::new(
                    right - measure_width(&tick.label, LABEL_SIZE),
                    baseline - f64::from(LABEL_SIZE),
                    right,
                    baseline + f64::from(LABEL_SIZE) * DESCENT,
                ),
            );
        }
    }

    // Axis title, rotated bottom-to-top and centred on the plot height.
    let mut title_box = None;
    if let Some(title) = title {
        let centre = (layout.plot_y_start() + layout.plot_y_end()) / 2.0;
        draw_text_rotated(
            scene,
            title,
            Y_TITLE_X,
            centre,
            TITLE_SIZE,
            ink.title,
            TextAnchor::Middle,
        );
        if !title.is_empty() {
            // The run reads upward from its pivot: ascent to the pivot's left,
            // descent to its right, the run centred on the plot's height.
            let half = measure_width(title, TITLE_SIZE) / 2.0;
            title_box = Some(Rect::new(
                Y_TITLE_X - f64::from(TITLE_SIZE),
                centre - half,
                Y_TITLE_X + f64::from(TITLE_SIZE) * DESCENT,
                centre + half,
            ));
        }
    }

    AxisTargets {
        title: title_box,
        labels,
        line: Rect::new(
            x - TICK_LENGTH - 3.0,
            layout.plot_y_start(),
            x + LINE_REACH,
            layout.plot_y_end(),
        ),
    }
}

/// Draw a map's graticule labels where a cartesian plot's tick labels sit: the
/// `meridians` below the plot area on the x-axis labels' baseline, the
/// `parallels` right-aligned in the left margin on the y-axis labels' — the
/// same size, the same offsets and [`ChartInk::label`], so a projected plot
/// reads its coordinates the way the axes it replaced did. Each tick's
/// `position` is where its line meets that edge
/// ([`crate::mark::PlotGraticule::edge_ticks`]).
///
/// No axis line and no tick marks: the lines themselves reach the edge, so a
/// tick mark would restate one and an axis line would frame two sides of four.
///
/// A label that would crowd the one drawn before it — within
/// [`LABEL_CLEARANCE`] along the edge — is skipped, and a meridian label is
/// nudged inside the tile by `contained_centre`, as a tick label is.
pub(crate) fn render_graticule_labels(
    scene: &mut Scene,
    layout: &ChartLayout,
    meridians: &[Tick],
    parallels: &[Tick],
    ink: ChartInk,
) {
    fn by_position(ticks: &[Tick]) -> Vec<&Tick> {
        let mut sorted: Vec<&Tick> = ticks.iter().collect();
        sorted.sort_by(|a, b| a.position.total_cmp(&b.position));
        sorted
    }

    let baseline = layout.plot_y_end() + TICK_LENGTH + f64::from(LABEL_SIZE);
    let mut drawn_to = f64::NEG_INFINITY;
    for tick in by_position(meridians) {
        let width = measure_width(&tick.label, LABEL_SIZE);
        let Some(centre) = contained_centre(tick.position, width, layout.width) else {
            continue;
        };
        if centre - width / 2.0 < drawn_to + LABEL_CLEARANCE {
            continue;
        }
        drawn_to = centre + width / 2.0;
        draw_text(
            scene,
            &tick.label,
            centre,
            baseline,
            LABEL_SIZE,
            ink.label,
            TextAnchor::Middle,
        );
    }

    let x = layout.plot_x_start() - TICK_LENGTH - 3.0;
    let mut drawn_to = f64::NEG_INFINITY;
    for tick in by_position(parallels) {
        if tick.position < drawn_to + LABEL_CLEARANCE {
            continue;
        }
        drawn_to = tick.position + f64::from(LABEL_SIZE);
        draw_text(
            scene,
            &tick.label,
            x,
            tick.position + f64::from(LABEL_SIZE) / 3.0,
            LABEL_SIZE,
            ink.label,
            TextAnchor::End,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{ChartLayout, Insets, Margins};
    use crate::scale::Scale;

    #[test]
    fn linear_scale_ticks_stay_within_range_and_are_labelled() {
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: 40.0,
            range_end: 600.0,
        };
        let ticks = compute_ticks(&scale, 5);
        assert!(!ticks.is_empty(), "should produce ticks");
        // All tick positions should be within the range.
        for tick in &ticks {
            assert!(
                tick.position >= 40.0 - 0.1,
                "tick at {:.1} below range start",
                tick.position
            );
            assert!(
                tick.position <= 600.0 + 0.1,
                "tick at {:.1} above range end",
                tick.position
            );
        }
        // Labels should be numeric strings.
        for tick in &ticks {
            assert!(!tick.label.is_empty(), "tick should have a label");
        }
    }

    #[test]
    fn band_scale_yields_one_ordered_tick_per_category() {
        let scale = Scale::Band {
            categories: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            range_start: 40.0,
            range_end: 600.0,
            padding: 0.1,
        };
        let ticks = compute_ticks(&scale, 3);
        assert_eq!(ticks.len(), 3, "should produce one tick per category");
        assert_eq!(ticks[0].label, "a");
        assert_eq!(ticks[1].label, "b");
        assert_eq!(ticks[2].label, "c");
        // Positions should be in order.
        assert!(ticks[0].position < ticks[1].position);
        assert!(ticks[1].position < ticks[2].position);
    }

    /// **A log axis spanning less than two decades is still labelled**, at
    /// 1, 2 and 5 of each decade it touches.
    ///
    /// Two domains, because the short case has two shapes: `[2, 50]` holds one
    /// power of ten (10), and `[3, 8]` holds none. Read as decades alone the
    /// first is one label and the second is an empty axis — a tile whose
    /// column spans a narrow positive range, thrown to log, would draw no
    /// numbers at all.
    #[test]
    fn a_log_axis_under_two_decades_is_labelled_at_one_two_and_five() {
        let log = |min, max| Scale::Log {
            domain_min: min,
            domain_max: max,
            range_start: 40.0,
            range_end: 600.0,
        };
        let labels = |scale: &Scale| -> Vec<String> {
            compute_ticks(scale, 10)
                .into_iter()
                .map(|tick| tick.label)
                .collect()
        };
        assert_eq!(
            labels(&log(2.0, 50.0)),
            ["2", "5", "10", "20", "50"],
            "[2, 50] is one decade and a half: 2, 5, 10, 20 and 50 lie inside it"
        );
        assert_eq!(
            labels(&log(3.0, 8.0)),
            ["5"],
            "[3, 8] holds no power of ten, and 5 is the one 1-2-5 step inside it"
        );
    }

    /// A number axis's tick text under a plot's format, from the step its ticks
    /// are drawn at: the same two axes d3-scale's `tickFormat` is given in its
    /// own tests of the rule.
    fn labels_under(scale: &Scale, spec: Option<&str>) -> Vec<String> {
        let format =
            spec.map(|s| AxisFormat::Number(NumberFormat::parse(s).expect("a number format")));
        compute_ticks_formatted(scale, 5, format.as_ref())
            .into_iter()
            .map(|tick| tick.label)
            .collect()
    }

    fn linear(min: f64, max: f64) -> Scale {
        Scale::Linear {
            domain_min: min,
            domain_max: max,
            range_start: 40.0,
            range_end: 600.0,
        }
    }

    /// **The top tick's text is the text of the largest tick the axis draws, not
    /// of the domain's end**: a domain to 5,565 stops its ticks at 5,000, and the
    /// sample reads that tick under the format, with no format, and reads nothing
    /// for a format of the wrong kind or an axis of names.
    #[test]
    fn the_top_tick_text_is_the_largest_drawn_tick_under_the_format() {
        let scale = linear(0.0, 5565.0);
        let number = |s: &str| AxisFormat::Number(NumberFormat::parse(s).expect("a number format"));
        let drawn = labels_under(&scale, Some("%"));
        assert_eq!(
            top_tick_text(&scale, 5, Some(&number("%"))).as_deref(),
            drawn.last().map(String::as_str),
            "the sample is the last tick the axis draws under percent"
        );
        assert_eq!(
            top_tick_text(&scale, 5, Some(&number(",f"))).as_deref(),
            Some("5,000"),
            "the top tick is 5,000, below the domain's end of 5,565"
        );
        assert_eq!(
            top_tick_text(&scale, 5, None).as_deref(),
            compute_ticks_formatted(&scale, 5, None)
                .last()
                .map(|t| t.label.as_str()),
            "with no format the sample is the axis's own text"
        );
        let date = AxisFormat::Date(DateFormat::parse("%b").expect("a date format"));
        assert_eq!(
            top_tick_text(&scale, 5, Some(&date)),
            None,
            "a date format on a number axis is dropped"
        );
        let names = Scale::Band {
            categories: vec!["a".into(), "b".into()],
            range_start: 0.0,
            range_end: 100.0,
            padding: 0.1,
        };
        assert_eq!(
            top_tick_text(&names, 5, Some(&number(",f"))),
            None,
            "names carry no number text"
        );
    }

    /// AC2: with no precision named, the precision follows the tick step. An
    /// SI axis shares the one prefix its larger end takes, so its zero reads
    /// `0.0k` and not `0`.
    #[test]
    fn a_format_with_no_precision_follows_the_ticks_own_step() {
        assert_eq!(
            labels_under(&linear(0.0, 2000.0), Some("s")),
            ["0.0k", "0.5k", "1.0k", "1.5k", "2.0k"]
        );
        assert_eq!(
            labels_under(&linear(0.0, 1.0), Some("%")),
            ["0%", "20%", "40%", "60%", "80%", "100%"]
        );
        // The same format over a wider step reads fewer decimals.
        assert_eq!(
            labels_under(&linear(0.0, 100.0), Some("+f")),
            ["+0", "+20", "+40", "+60", "+80", "+100"]
        );
    }

    /// AC1: a format that names its precision prints each value as d3-format
    /// does, whatever the step. The one exception is an SI axis, which keeps
    /// the analyst's decimals and still shares the prefix of its largest tick.
    #[test]
    fn a_format_with_a_precision_prints_each_tick_as_written() {
        assert_eq!(
            labels_under(&linear(0.0, 2_000_000.0), Some(".2s")),
            ["0.00M", "0.50M", "1.00M", "1.50M", "2.00M"]
        );
        assert_eq!(
            labels_under(&linear(0.0, 2_000_000.0), Some(",d")),
            ["0", "500,000", "1,000,000", "1,500,000", "2,000,000"]
        );
        assert_eq!(
            labels_under(&linear(0.0, 1.0), Some(".0%")),
            ["0%", "20%", "40%", "60%", "80%", "100%"]
        );
    }

    /// AC5 at the axis: no format draws the text every axis drew before a
    /// format could be asked for, integers as integers and other values to one
    /// decimal place.
    #[test]
    fn no_format_draws_the_text_an_axis_always_drew() {
        for scale in [linear(0.0, 2000.0), linear(0.0, 1.0), linear(-3.0, 4.0)] {
            for tick in compute_ticks_formatted(&scale, 5, None) {
                assert_eq!(
                    tick.label,
                    format_number(tick.value),
                    "with no format a tick reads as the axis has always read it"
                );
            }
        }
        assert_eq!(
            labels_under(&linear(0.0, 1.0), None),
            ["0", "0.2", "0.4", "0.6", "0.8", "1"]
        );
    }

    /// A log or symlog axis sits on decades, not on a step, so each decade
    /// prints as the format says with its zeros trimmed, and a negative decade
    /// leads with the minus sign d3-format prints.
    #[test]
    fn a_log_or_symlog_axis_prints_each_decade_under_the_format() {
        let log = Scale::Log {
            domain_min: 1.0,
            domain_max: 10_000.0,
            range_start: 40.0,
            range_end: 600.0,
        };
        assert_eq!(
            labels_under(&log, Some("s")),
            ["1", "10", "100", "1k", "10k"]
        );
        assert_eq!(
            labels_under(&log, Some(".1f")),
            ["1.0", "10.0", "100.0", "1000.0", "10000.0"]
        );
        let symlog = Scale::Symlog {
            domain_min: -100.0,
            domain_max: 100.0,
            range_start: 40.0,
            range_end: 600.0,
        };
        assert_eq!(
            labels_under(&symlog, Some("d")),
            [
                "\u{2212}100",
                "\u{2212}10",
                "\u{2212}1",
                "0",
                "1",
                "10",
                "100"
            ]
        );
    }

    /// A band axis prints its categories and a time axis its seconds, whatever
    /// number format the plot names: neither is a number axis. (The plot is told
    /// so: [`tick_format_crosses_axis`] names a number format on a time axis, and
    /// [`tick_format_applies`] a format of either kind on a band of names.)
    #[test]
    fn a_band_or_time_axis_ignores_a_number_format() {
        let band = Scale::Band {
            categories: vec!["a".to_string(), "b".to_string()],
            range_start: 40.0,
            range_end: 600.0,
            padding: 0.1,
        };
        assert_eq!(labels_under(&band, Some("s")), ["a", "b"]);
        let time = Scale::Time {
            domain_min_us: 1_000_000,
            domain_max_us: 4_000_000,
            range_start: 40.0,
            range_end: 600.0,
        };
        assert_eq!(labels_under(&time, Some("s")), labels_under(&time, None));
    }

    fn band(categories: &[&str]) -> Scale {
        Scale::Band {
            categories: categories.iter().map(|c| (*c).to_string()).collect(),
            range_start: 40.0,
            range_end: 600.0,
            padding: 0.1,
        }
    }

    fn date(spec: &str) -> AxisFormat {
        AxisFormat::Date(DateFormat::parse(spec).expect("a date format"))
    }

    /// A date format prints a time axis's instants and a band of days, in UTC, at
    /// the ticks the axis already had; a band of names, or of days with one name
    /// among them, prints its categories as it did.
    #[test]
    fn a_date_format_prints_a_time_axis_and_a_band_of_days_and_leaves_names_alone() {
        // 2024-03-01T14:00:00Z to 14:10:00Z, in microseconds.
        let start = 1_709_301_600_000_000_i64;
        let time = Scale::Time {
            domain_min_us: start,
            domain_max_us: start + 600_000_000,
            range_start: 40.0,
            range_end: 600.0,
        };
        let format = date("%H:%M");
        let drawn = compute_ticks_formatted(&time, 5, Some(&format));
        let bare = compute_ticks_formatted(&time, 5, None);
        assert_eq!(
            drawn.iter().map(|t| t.position).collect::<Vec<_>>(),
            bare.iter().map(|t| t.position).collect::<Vec<_>>(),
            "a format changes the text and never the place"
        );
        assert!(
            drawn.iter().any(|t| t.label == "14:05"),
            "{:?}",
            drawn.iter().map(|t| &t.label).collect::<Vec<_>>()
        );

        let days = band(&["2024-03-01", "2024-04-01"]);
        let month = date("%b");
        let labels = |scale: &Scale, format: Option<&AxisFormat>| -> Vec<String> {
            compute_ticks_formatted(scale, 5, format)
                .into_iter()
                .map(|t| t.label)
                .collect()
        };
        assert_eq!(labels(&days, Some(&month)), ["Mar", "Apr"]);
        assert_eq!(labels(&days, None), ["2024-03-01", "2024-04-01"]);

        let names = band(&["north", "south"]);
        assert_eq!(labels(&names, Some(&month)), ["north", "south"]);
        let mixed = band(&["2024-03-01", "south"]);
        assert_eq!(
            labels(&mixed, Some(&month)),
            ["2024-03-01", "south"],
            "one name among the days makes it a band of names, not a half-formatted one"
        );
    }

    /// The judge the axis draws through and the composition warns through: a
    /// number format crosses a date axis and a date format crosses a number
    /// axis, and nothing else crosses: not a format on its own kind of axis, not
    /// an axis of names, not a colour ramp.
    #[test]
    fn a_format_crosses_an_axis_of_the_other_kind_and_no_other() {
        let number = AxisFormat::Number(NumberFormat::parse("s").expect("number"));
        let month = date("%b");
        let time = Scale::Time {
            domain_min_us: 0,
            domain_max_us: 1_000_000,
            range_start: 0.0,
            range_end: 1.0,
        };
        let days = band(&["2024-03-01"]);
        let names = band(&["a"]);
        let linear = Scale::Linear {
            domain_min: 0.0,
            domain_max: 1.0,
            range_start: 0.0,
            range_end: 1.0,
        };

        assert!(tick_format_crosses_axis(&time, &number));
        assert!(tick_format_crosses_axis(&days, &number));
        assert!(tick_format_crosses_axis(&linear, &month));

        assert!(!tick_format_crosses_axis(&time, &month));
        assert!(!tick_format_crosses_axis(&days, &month));
        assert!(!tick_format_crosses_axis(&linear, &number));
        assert!(!tick_format_crosses_axis(&names, &number));
        assert!(!tick_format_crosses_axis(&names, &month));

        assert_eq!(axis_kind(&time), Some(AxisKind::Date));
        assert_eq!(axis_kind(&days), Some(AxisKind::Date));
        assert_eq!(axis_kind(&names), Some(AxisKind::Category));
        assert_eq!(axis_kind(&linear), Some(AxisKind::Number));
    }

    /// The banner's word for an axis tells a log axis from a linear one, which
    /// `axis_kind` does not: both are a number axis there.
    #[test]
    fn the_word_for_an_axis_tells_a_log_axis_from_a_linear_one() {
        let span = |make: fn(f64, f64, f64, f64) -> Scale| make(1.0, 100.0, 0.0, 1.0);
        let linear = span(
            |domain_min, domain_max, range_start, range_end| Scale::Linear {
                domain_min,
                domain_max,
                range_start,
                range_end,
            },
        );
        let log = span(
            |domain_min, domain_max, range_start, range_end| Scale::Log {
                domain_min,
                domain_max,
                range_start,
                range_end,
            },
        );
        let symlog = span(
            |domain_min, domain_max, range_start, range_end| Scale::Symlog {
                domain_min,
                domain_max,
                range_start,
                range_end,
            },
        );
        let time = Scale::Time {
            domain_min_us: 0,
            domain_max_us: 1_000_000,
            range_start: 0.0,
            range_end: 1.0,
        };
        assert_eq!(axis_scale_word(&linear), Some("linear"));
        assert_eq!(axis_scale_word(&log), Some("log"));
        assert_eq!(axis_scale_word(&symlog), Some("symlog"));
        assert_eq!(axis_scale_word(&time), Some("date"));
        assert_eq!(axis_scale_word(&band(&["2024-03-01"])), Some("date"));
        assert_eq!(axis_scale_word(&band(&["north"])), Some("category"));
    }

    /// The judge the composition warns through is the axis's own behaviour: a
    /// scale takes the count exactly when drawing it at a count of two and at a
    /// count of twenty puts its ticks in different places or words. Each kind of
    /// positional scale is drawn both ways, so a scale that learns to follow the
    /// count, or stops, fails here rather than drifting from the warning.
    #[test]
    fn tick_count_applies_where_the_ticks_follow_the_count() {
        let scales = [
            (
                "linear",
                Scale::Linear {
                    domain_min: 0.0,
                    domain_max: 1000.0,
                    range_start: 40.0,
                    range_end: 600.0,
                },
            ),
            (
                "time",
                Scale::Time {
                    domain_min_us: 0,
                    domain_max_us: 86_400_000_000,
                    range_start: 40.0,
                    range_end: 600.0,
                },
            ),
            (
                "log",
                Scale::Log {
                    domain_min: 1.0,
                    domain_max: 1_000_000.0,
                    range_start: 40.0,
                    range_end: 600.0,
                },
            ),
            (
                "symlog",
                Scale::Symlog {
                    domain_min: -1000.0,
                    domain_max: 1000.0,
                    range_start: 40.0,
                    range_end: 600.0,
                },
            ),
            ("band of names", band(&["north", "south", "east", "west"])),
            (
                "band of days",
                band(&["2024-03-01", "2024-04-01", "2024-05-01"]),
            ),
        ];
        for (name, scale) in &scales {
            let drawn = |count: usize| -> Vec<(u64, String, u64)> {
                compute_ticks(scale, count)
                    .iter()
                    .map(|t| (t.value.to_bits(), t.label.clone(), t.position.to_bits()))
                    .collect()
            };
            let follows_the_count = drawn(2) != drawn(20);
            assert_eq!(
                tick_count_applies(scale),
                follows_the_count,
                "{name}: the judge says the count applies = {}, the ticks say {follows_the_count}",
                tick_count_applies(scale)
            );
        }
        assert!(
            scales
                .iter()
                .filter(|(_, scale)| tick_count_applies(scale))
                .count()
                == 2,
            "the linear and the time axis take the count"
        );
    }

    /// The judge the composition warns through is the axis's own behaviour: a
    /// scale takes a tick format exactly when drawing it under a number format
    /// or a date format puts different words on its ticks than drawing it under
    /// none. Each kind of positional scale is drawn all three ways, and a band of
    /// names is one whose labels read as numbers as well as one whose do not, so
    /// a scale that learns to take a format, or stops, fails here rather than
    /// drifting from the warning.
    #[test]
    fn tick_format_applies_where_the_ticks_follow_a_format() {
        let scales = [
            ("linear", linear(0.0, 1000.0)),
            (
                "time",
                Scale::Time {
                    domain_min_us: 1_709_301_600_000_000,
                    domain_max_us: 1_709_301_600_000_000 + 600_000_000,
                    range_start: 40.0,
                    range_end: 600.0,
                },
            ),
            (
                "log",
                Scale::Log {
                    domain_min: 1.0,
                    domain_max: 1_000_000.0,
                    range_start: 40.0,
                    range_end: 600.0,
                },
            ),
            (
                "symlog",
                Scale::Symlog {
                    domain_min: -1000.0,
                    domain_max: 1000.0,
                    range_start: 40.0,
                    range_end: 600.0,
                },
            ),
            ("band of names", band(&["north", "south", "east", "west"])),
            (
                "band of names that read as numbers",
                band(&["1", "2", "3", "4"]),
            ),
            (
                "band of days",
                band(&["2024-03-01", "2024-04-01", "2024-05-01"]),
            ),
        ];
        let number = AxisFormat::Number(NumberFormat::parse(".2f").expect("number"));
        let month = date("%b");
        for (name, scale) in &scales {
            let words = |format: Option<&AxisFormat>| -> Vec<String> {
                compute_ticks_formatted(scale, 5, format)
                    .into_iter()
                    .map(|tick| tick.label)
                    .collect()
            };
            let plain = words(None);
            let follows_a_format = words(Some(&number)) != plain || words(Some(&month)) != plain;
            assert_eq!(
                tick_format_applies(scale),
                follows_a_format,
                "{name}: the judge says a format applies = {}, the ticks say {follows_a_format}",
                tick_format_applies(scale)
            );
        }
        assert_eq!(
            scales
                .iter()
                .filter(|(_, scale)| !tick_format_applies(scale))
                .map(|(name, _)| *name)
                .collect::<Vec<_>>(),
            ["band of names", "band of names that read as numbers"],
            "only a band of names takes no format"
        );
    }

    #[test]
    fn time_scale_ticks_stay_within_range() {
        let scale = Scale::Time {
            domain_min_us: 1_000_000,
            domain_max_us: 4_000_000,
            range_start: 40.0,
            range_end: 600.0,
        };
        let ticks = compute_ticks(&scale, 5);
        assert!(!ticks.is_empty(), "should produce time ticks");
        for tick in &ticks {
            assert!(tick.position >= 40.0 - 0.1);
            assert!(tick.position <= 600.0 + 0.1);
            assert!(
                tick.label.contains('s'),
                "time tick label should contain 's': {}",
                tick.label
            );
        }
    }

    /// True if any glyph run in the scene carries a quarter-turn (±90°) rotation
    /// — a rotation has a ~zero diagonal and ~±1 off-diagonal, whereas
    /// horizontal text (tick labels, x-title, plot title) uses an
    /// identity/translate run transform ([1,0,0,1]). Reads the public
    /// `vello_encoding` glyph-run transform matrices, no GPU.
    fn scene_has_quarter_turn(scene: &Scene) -> bool {
        scene.encoding().resources.glyph_runs.iter().any(|r| {
            let m = r.transform.matrix;
            m[0].abs() < 1e-3 && m[3].abs() < 1e-3 && m[1].abs() > 0.5 && m[2].abs() > 0.5
        })
    }

    #[test]
    fn render_y_axis_rotates_its_title_but_x_does_not() {
        // Pinned AT THE RENDER SITE: render_y_axis must draw its title
        // rotated (a draw_text_rotated → draw_text refactor would ship a
        // horizontal, tick-colliding y-title otherwise). render_x_axis's title
        // is horizontal; neither axis rotates without a title.
        let layout = ChartLayout::new(400.0, 300.0);
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: layout.plot_y_end(),
            range_end: layout.plot_y_start(),
        };
        let ticks = compute_ticks(&scale, 5);

        let mut y_titled = Scene::new();
        render_y_axis(
            &mut y_titled,
            &layout,
            &ticks,
            Some("Travelers"),
            ChartInk::LIGHT,
        );
        assert!(
            scene_has_quarter_turn(&y_titled),
            "render_y_axis must rotate its title (bottom-to-top)"
        );

        let mut y_plain = Scene::new();
        render_y_axis(&mut y_plain, &layout, &ticks, None, ChartInk::LIGHT);
        assert!(
            !scene_has_quarter_turn(&y_plain),
            "no rotation without a y-title"
        );

        let mut x_titled = Scene::new();
        render_x_axis(
            &mut x_titled,
            &layout,
            &ticks,
            Some("weight"),
            ChartInk::LIGHT,
        );
        assert!(
            !scene_has_quarter_turn(&x_titled),
            "the x-axis title is horizontal, not rotated"
        );
    }

    #[test]
    fn axis_titles_render_and_clear_tick_labels() {
        use crate::text::measure_width;

        // Grown margins (left +band for a y-title, bottom +band for an x-title).
        let margins = Margins {
            left: 60.0,
            right: 20.0,
            bottom: 50.0,
            top: 20.0,
        };
        let layout = ChartLayout::with_margins_and_insets(400.0, 300.0, margins, Insets::default());
        let ticks = vec![
            Tick {
                value: 10.0,
                label: "10".into(),
                position: layout.plot_y_end(),
            },
            Tick {
                value: 20.0,
                label: "20".into(),
                position: layout.plot_y_start(),
            },
        ];

        // Drawing WITH a title adds ink over drawing without.
        let mut with_t = Scene::new();
        render_x_axis(
            &mut with_t,
            &layout,
            &ticks,
            Some("Arrival Delay"),
            ChartInk::LIGHT,
        );
        let mut no_t = Scene::new();
        render_x_axis(&mut no_t, &layout, &ticks, None, ChartInk::LIGHT);
        assert!(
            with_t.encoding().draw_tags.len() > no_t.encoding().draw_tags.len(),
            "an x-axis title adds ink"
        );

        // x-title top edge sits BELOW the x tick-label baseline (no overlap).
        let tick_label_baseline = layout.plot_y_end() + TICK_LENGTH + f64::from(LABEL_SIZE);
        assert!(
            x_title_baseline(&layout) - f64::from(TITLE_SIZE) > tick_label_baseline,
            "x-title top edge is below the tick labels"
        );

        // y-title right edge sits LEFT of the widest y tick label's left edge.
        let widest = ticks
            .iter()
            .map(|t| measure_width(&t.label, LABEL_SIZE))
            .fold(0.0_f64, f64::max);
        let label_left = layout.plot_x_start() - TICK_LENGTH - 3.0 - widest;
        assert!(
            Y_TITLE_X + f64::from(TITLE_SIZE) < label_left,
            "y-title right edge {} must be left of the widest y label at {label_left}",
            Y_TITLE_X + f64::from(TITLE_SIZE),
        );

        let mut yt = Scene::new();
        render_y_axis(&mut yt, &layout, &ticks, Some("Travelers"), ChartInk::LIGHT);
        let mut yn = Scene::new();
        render_y_axis(&mut yn, &layout, &ticks, None, ChartInk::LIGHT);
        assert!(
            yt.encoding().draw_tags.len() > yn.encoding().draw_tags.len(),
            "a y-axis title adds ink"
        );
    }

    #[test]
    fn render_x_axis_produces_scene_content() {
        let layout = ChartLayout::new(640.0, 480.0);
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: layout.plot_x_start(),
            range_end: layout.plot_x_end(),
        };
        let ticks = compute_ticks(&scale, 5);

        let mut scene = Scene::new();
        render_x_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "x-axis should produce scene content"
        );
    }

    #[test]
    fn render_y_axis_produces_scene_content() {
        let layout = ChartLayout::new(640.0, 480.0);
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: layout.plot_y_end(),
            range_end: layout.plot_y_start(),
        };
        let ticks = compute_ticks(&scale, 5);

        let mut scene = Scene::new();
        render_y_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "y-axis should produce scene content"
        );
    }

    #[test]
    fn nice_step_produces_human_readable_intervals() {
        // 0-100 with ~5 ticks should give step=20
        let step = nice_step(100.0, 5);
        assert!(
            (step - 20.0).abs() < f64::EPSILON,
            "expected step 20, got {step}"
        );

        // 0-1000 with ~5 ticks should give step=200
        let step = nice_step(1000.0, 5);
        assert!(
            (step - 200.0).abs() < f64::EPSILON,
            "expected step 200, got {step}"
        );
    }

    // -----------------------------------------------------------------
    // Round ends — `nice_linear_domain`
    // -----------------------------------------------------------------

    /// Domains the rounding is asked about: whole and fractional, negative and
    /// straddling zero, narrow beside their offset, and wide. The fractional
    /// ones are where a quotient lands a few ulps off the whole number it should
    /// be, so a rounding that used a bare `floor` or `ceil` would move an end a
    /// whole step.
    const NICE_DOMAINS: [(f64, f64); 14] = [
        (3.0, 97.0),
        (7.0, 43.0),
        (40.0, 90.0),
        (0.3, 0.9),
        (0.12, 0.87),
        (0.07, 0.43),
        (-3.7, 12.2),
        (-97.0, -3.0),
        (-0.4, 0.4),
        (1234.5, 1299.5),
        (0.000_31, 0.000_77),
        (1.0e6, 1.0e6 + 4321.0),
        (0.0, 47.0),
        (14.0, 26.0),
    ];

    /// **A rounded domain contains the data, and asking again moves nothing.**
    /// The second is what lets a domain a plot pinned after rounding be rounded
    /// again on every later composition without drifting a step each time.
    #[test]
    fn a_rounded_domain_holds_the_data_and_is_a_fixed_point() {
        for (min, max) in NICE_DOMAINS {
            for target in [2, 3, 5, 10] {
                let (lo, hi) = nice_linear_domain(min, max, target);
                assert!(
                    lo <= min && hi >= max,
                    "({min}, {max}) at {target} ticks rounds to ({lo}, {hi}), which cuts the data"
                );
                assert_eq!(
                    nice_linear_domain(lo, hi, target),
                    (lo, hi),
                    "({min}, {max}) at {target} ticks rounds to ({lo}, {hi}), and rounding that \
                     again moves it"
                );
            }
        }
    }

    /// **A rounded end is a tick.** The axis draws a tick at each end of the
    /// domain `nice_linear_domain` returns, at the count it rounded for, so the
    /// top of the axis is a labelled number and the bottom is too.
    #[test]
    fn each_end_of_a_rounded_domain_is_a_tick() {
        for (min, max) in NICE_DOMAINS {
            for target in [2, 3, 5, 10] {
                let (lo, hi) = nice_linear_domain(min, max, target);
                let ticks = compute_linear_ticks(lo, hi, 0.0, 100.0, target, None);
                let (first, last) = (
                    ticks.first().expect("the domain has ticks").value,
                    ticks.last().expect("the domain has ticks").value,
                );
                let tolerance = (hi - lo) * 1e-9;
                assert!(
                    (first - lo).abs() <= tolerance && (last - hi).abs() <= tolerance,
                    "({min}, {max}) at {target} ticks rounds to ({lo}, {hi}) but its ticks run \
                     from {first} to {last}"
                );
            }
        }
    }

    /// **A value already on a step is not moved a whole step.** `1.12 * 50` is
    /// `56.00000000000001`, so a bare `ceil` of it would carry the top of 0.8 to
    /// 1.12 out to 1.14 at ten ticks, and a sum like `0.1 + 0.2` would add a step
    /// at five. The ends are left on the multiple they were a few ulps from.
    #[test]
    fn a_value_already_on_a_step_is_not_moved_a_whole_step() {
        assert_eq!(nice_linear_domain(0.8, 1.12, 10), (0.8, 1.12));
        assert_eq!(nice_linear_domain(0.1, 0.1 + 0.2, 5), (0.1, 0.3));
        assert_eq!(
            nice_linear_domain(0.34, 1.400_000_000_000_000_1, 5),
            (0.2, 1.4)
        );
    }

    /// The domains rounding leaves as it found them: a span with no width, a
    /// count of zero, and an end that is not a finite number.
    #[test]
    fn rounding_leaves_a_domain_it_cannot_step_as_it_found_it() {
        assert_eq!(nice_linear_domain(5.0, 5.0, 5), (5.0, 5.0));
        assert_eq!(nice_linear_domain(3.0, 97.0, 0), (3.0, 97.0));
        let (lo, hi) = nice_linear_domain(f64::NEG_INFINITY, 97.0, 5);
        assert!(lo.is_infinite() && hi == 97.0);
        let (lo, hi) = nice_linear_domain(3.0, f64::NAN, 5);
        assert!(lo == 3.0 && hi.is_nan());
    }

    /// An axis that ends at zero ends at zero with no sign on it: rounding a
    /// small negative top up lands on `ceil(-0.02)`, which is a negative zero,
    /// and a negative zero would print as `-0` on a label that reads the sign.
    #[test]
    fn a_rounded_end_at_zero_carries_no_sign() {
        let (lo, hi) = nice_linear_domain(-47.0, -0.2, 5);
        assert!(
            lo < 0.0,
            "fixture check: the domain is below zero, got {lo:?}"
        );
        assert!(hi == 0.0 && hi.is_sign_positive(), "got {hi:?}");
        let (lo, _) = nice_linear_domain(0.2, 47.0, 5);
        assert!(lo == 0.0 && lo.is_sign_positive(), "got {lo:?}");
    }

    // -----------------------------------------------------------------
    // Thin before you rotate — a time axis at a dashboard tile's width
    // -----------------------------------------------------------------

    /// The six real dates `crates/brightfield-shell/tests/data/dashboard_baseline.csv`'s
    /// `day` column carries, in file order. Restated rather than read off the
    /// CSV, because this crate carries no CSV reader and no dependency on
    /// `brightfield-shell`; the six strings are what ties the test below to
    /// that fixture rather than to an invented one.
    const FIXTURE_DAYS: &[&str] = &[
        "2026-01-05",
        "2026-01-06",
        "2026-01-07",
        "2026-01-08",
        "2026-01-09",
        "2026-01-10",
    ];

    /// A [`Scale::Band`] over [`FIXTURE_DAYS`], ranged across `layout`'s own
    /// (inset-adjusted) x range — the same range [`crate::scale::infer_scales_in`]
    /// would resolve a `day` column's scale against.
    fn fixture_day_scale(layout: &ChartLayout) -> Scale {
        let (range_start, range_end) = layout.x_range();
        Scale::Band {
            categories: FIXTURE_DAYS.iter().map(|s| (*s).to_string()).collect(),
            range_start,
            range_end,
            padding: 0.1,
        }
    }

    /// Matches each horizontal glyph run in `scene` back to whichever `ticks`
    /// entry its draw position ([`TextAnchor::Middle`], `render_x_axis`'s own
    /// anchor) is nearest, then asserts no two runs' `[x, x + width]`
    /// intervals intersect — the width read with [`measure_width`], the same
    /// shaping `render_x_axis` measured it with, rather than estimated from
    /// the run's raw glyph count. A rotated run (its transform carries a
    /// quarter turn) is skipped: this checks the horizontal branch.
    fn assert_no_tick_label_overlap(scene: &Scene, ticks: &[Tick], size: f32) {
        let candidates: Vec<(f64, &str)> = ticks
            .iter()
            .map(|t| {
                (
                    t.position - measure_width(&t.label, size) / 2.0,
                    t.label.as_str(),
                )
            })
            .collect();
        let mut spans: Vec<(f64, f64)> = Vec::new();
        for run in &scene.encoding().resources.glyph_runs {
            let m = run.transform.matrix;
            let rotated = m[0].abs() < 1e-3 && m[3].abs() < 1e-3;
            if rotated {
                continue;
            }
            let x0 = f64::from(run.transform.translation[0]);
            let (_, label) = candidates
                .iter()
                .min_by(|a, b| (a.0 - x0).abs().partial_cmp(&(b.0 - x0).abs()).unwrap())
                .expect("fixture check: at least one candidate tick");
            spans.push((x0, x0 + measure_width(label, size)));
        }
        spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for pair in spans.windows(2) {
            assert!(
                pair[0].1 <= pair[1].0,
                "two drawn tick labels overlap: {pair:?} (all spans: {spans:?})"
            );
        }
    }

    /// The counts_over_time tile's real dates stay legible at a narrow tile
    /// (240 points) and a wide one (720), so the thinning rule is a rule
    /// rather than a constant tuned to one width. At both, `thinned_x_ticks`
    /// finds a stride that clears, so the labels stay horizontal — see
    /// `rotation_is_the_fallback_when_thinning_cannot_clear_the_labels_and_there_is_room_to_rotate`
    /// and `the_axis_degrades_to_one_label_rather_than_clip_a_rotated_band_past_a_title`
    /// for widths where it cannot.
    #[test]
    fn thinning_keeps_labels_from_touching_at_various_widths() {
        for width in [240.0_f64, 720.0_f64] {
            let layout = ChartLayout::new(width, 300.0);
            let scale = fixture_day_scale(&layout);
            let ticks = compute_ticks(&scale, 5);
            assert_eq!(
                ticks.len(),
                FIXTURE_DAYS.len(),
                "fixture check: one tick per date"
            );

            let mut scene = Scene::new();
            render_x_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);
            assert!(
                !scene_has_quarter_turn(&scene),
                "at {width} points wide thinning should have kept the labels \
                 horizontal rather than rotating them"
            );
            assert_no_tick_label_overlap(&scene, &ticks, LABEL_SIZE);
        }
    }

    /// The rotation fallback, isolated, over a margin with room to rotate
    /// into — the same six real dates crowded past what dropping labels can
    /// fix, at a width picked to force it (132 points: even the two end
    /// dates' labels do not clear each other there), with a bottom margin
    /// grown past [`rotated_label_room`]'s floor for one date's own width.
    /// `render_x_axis` should still fall back to `draw_text_rotated` and draw
    /// a label for each tick rather than a thinned subset, and the rotated
    /// labels themselves should not collide either — 132 was chosen so the
    /// band between ticks still clears one rotated label's own width even
    /// though it cannot clear the unrotated text. See
    /// `the_axis_degrades_to_one_label_rather_than_clip_a_rotated_band_past_a_title`
    /// for the same crowding with no such room.
    #[test]
    fn rotation_is_the_fallback_when_thinning_cannot_clear_the_labels_and_there_is_room_to_rotate()
    {
        let margins = Margins {
            bottom: 100.0,
            ..Margins::default()
        };
        let layout = ChartLayout::with_margins_and_insets(132.0, 300.0, margins, Insets::default());
        let scale = fixture_day_scale(&layout);
        let ticks = compute_ticks(&scale, 5);
        let widest = ticks
            .iter()
            .map(|t| measure_width(&t.label, LABEL_SIZE))
            .fold(0.0_f64, f64::max);
        assert!(
            widest <= rotated_label_room(&layout, false),
            "fixture check: the margin this test grew ({margins:?}) is meant \
             to leave room to rotate a real date ({widest} points wide) into"
        );

        let mut scene = Scene::new();
        render_x_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);

        assert!(
            scene_has_quarter_turn(&scene),
            "thinning cannot clear real dates at 132 points wide and there is \
             room to rotate into, so the axis should have rotated its labels \
             instead of drawing them horizontal"
        );
        let glyph_runs = scene.encoding().resources.glyph_runs.len();
        assert_eq!(
            glyph_runs,
            ticks.len(),
            "a rotated axis draws a label for each tick ({} ticks) rather than \
             the thinned subset a horizontal axis would ({glyph_runs} runs drawn)",
            ticks.len(),
        );

        // The rotated labels themselves must not collide: consecutive runs'
        // pivots (`TextAnchor::End`, so translation is each label's END, the
        // point nearest its tick) sit at least `LABEL_SIZE` apart along the x
        // axis, which is a rotated label's own footprint.
        let mut xs: Vec<f64> = scene
            .encoding()
            .resources
            .glyph_runs
            .iter()
            .map(|r| f64::from(r.transform.translation[0]))
            .collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in xs.windows(2) {
            assert!(
                pair[1] - pair[0] >= f64::from(LABEL_SIZE) - 0.01,
                "two rotated tick labels sit closer than one label's own \
                 width apart: {pair:?}"
            );
        }
    }

    /// **When there is no room to rotate a real date past a title, the axis
    /// degrades to a single label rather than clip one under it.**
    /// [`FIXTURE_DAYS`] crowded at 130 points — narrow enough that
    /// `thinned_x_ticks` cannot clear even its two end ticks — with an
    /// x title present. [`rotated_label_room`] measures the floor a title
    /// leaves as a small, FIXED distance below the tick line regardless of
    /// how large the margin is (`x_title_baseline` is a constant offset from
    /// `ChartLayout::plot_y_end`, not from the margin's own size), so growing
    /// the margin further would not have bought a 65-point date any more
    /// room here — unlike
    /// `rotation_is_the_fallback_when_thinning_cannot_clear_the_labels_and_there_is_room_to_rotate`,
    /// which has no title and grows past its floor instead.
    #[test]
    fn the_axis_degrades_to_one_label_rather_than_clip_a_rotated_band_past_a_title() {
        let unfitted = ChartLayout::new(130.0, 300.0);
        let scale = fixture_day_scale(&unfitted);
        let ticks = compute_ticks(&scale, 5);
        let thinned = thinned_x_ticks(&ticks, LABEL_SIZE, unfitted.width);
        assert!(
            !labels_clear_horizontally(&thinned, LABEL_SIZE, unfitted.width),
            "fixture check: 130 points is meant to be a width where even the \
             two end dates collide, which is what forces the choice this \
             test is about"
        );

        // A bottom margin generous enough that an UNTITLED axis at this same
        // width would have room to rotate a real date into — grown by hand
        // rather than through `grow_margins`, because the point of what
        // follows is to hold the width and the margin FIXED and change just
        // whether a title is present. Finding: a ten-character date used to
        // overrun BOTH floors at once at the margin `grow_margins` actually
        // produces for a titled axis here (bottom 50), which left the titled
        // and untitled floors indistinguishable — this margin (bottom 100)
        // is chosen so the fixture check below can tell them apart.
        let margins = Margins {
            bottom: 100.0,
            ..Margins::default()
        };
        let layout = ChartLayout::with_margins(130.0, 300.0, margins);
        let scale = fixture_day_scale(&layout);
        let ticks = compute_ticks(&scale, 5);
        let widest = ticks
            .iter()
            .map(|t| measure_width(&t.label, LABEL_SIZE))
            .fold(0.0_f64, f64::max);
        assert!(
            widest <= rotated_label_room(&layout, false),
            "fixture check: at this margin an UNTITLED axis is meant to have \
             room to rotate a real date ({widest} points wide) into — the \
             control this test needs so the degrade below reads as the \
             title's doing rather than the layout being too narrow outright"
        );
        assert!(
            widest > rotated_label_room(&layout, true),
            "fixture check: a real date ({widest} points wide) is meant to \
             overrun the room a title leaves, at the SAME margin the line \
             above just proved has room without one"
        );

        let mut scene = Scene::new();
        render_x_axis(&mut scene, &layout, &ticks, Some("day"), ChartInk::LIGHT);

        assert!(
            !scene_has_quarter_turn(&scene),
            "there is no room below the title to rotate a real date into, so \
             the axis should have degraded to a single label rather than \
             clip a rotated one past the title"
        );
        let glyph_runs = &scene.encoding().resources.glyph_runs;
        assert_eq!(
            glyph_runs.len(),
            2,
            "a degraded titled axis draws one tick label plus the title, not \
             {} runs",
            glyph_runs.len(),
        );

        // The one label drawn sits at the ordinary horizontal tick-label
        // baseline (`axis_titles_render_and_clear_tick_labels` pins that row
        // clear of the title already) and the other run is the title itself
        // — nothing draws anywhere else, in particular not past either.
        let label_y = layout.plot_y_end() + TICK_LENGTH + f64::from(LABEL_SIZE);
        let title_y = x_title_baseline(&layout);
        let mut saw_label_row = false;
        for run in glyph_runs {
            let y = f64::from(run.transform.translation[1]);
            assert!(
                (y - label_y).abs() < 0.5 || (y - title_y).abs() < 0.5,
                "a glyph run drew at y={y}, neither the tick-label baseline \
                 {label_y} nor the title baseline {title_y} — it drew \
                 somewhere a rotated run would have, clipped or not"
            );
            if (y - label_y).abs() < 0.5 {
                saw_label_row = true;
            }
        }
        assert!(
            saw_label_row,
            "no run drew at the ordinary tick-label baseline {label_y}"
        );

        // The control the fixture checks above set up: the SAME width and
        // margin, with no title, rotates. If this degraded too, the title
        // would not be what forced the degrade above, and the two fixture
        // checks at the top of this test would not actually have
        // distinguished a titled floor from an untitled one.
        let mut untitled_scene = Scene::new();
        render_x_axis(&mut untitled_scene, &layout, &ticks, None, ChartInk::LIGHT);
        assert!(
            scene_has_quarter_turn(&untitled_scene),
            "the same width and margin, with no title, should have rotated \
             — a degrade here would mean the title above was not what forced \
             the degrade, and this test would be pinning sheer narrowness \
             rather than the claim its name makes"
        );
    }

    /// **A gap inside [`LABEL_CLEARANCE`] reads as NOT clear, even though the
    /// two labels do not yet overlap.** Containment alone cannot pin this —
    /// a tile wide enough leaves both labels contained whatever the gap
    /// between them — so this reads [`labels_clear_horizontally`] directly,
    /// on two synthetic ticks placed so their reach (half of each label's
    /// width) leaves exactly half of [`LABEL_CLEARANCE`] between them:
    /// comfortably short of overlapping, and just as comfortably short of
    /// the minimum. A mutation that zeroed `LABEL_CLEARANCE` would read this
    /// pair as clear, which is the gap this test closes.
    #[test]
    fn label_clearance_rejects_a_gap_narrower_than_the_minimum() {
        let label = "22";
        let width = measure_width(label, LABEL_SIZE);
        let reach = width; // two equal-width labels: half + half = the full width
        let tile_width = 1000.0; // wide enough that containment moves neither label

        let inside_minimum = LABEL_CLEARANCE / 2.0;
        let too_close = [
            Tick {
                value: 0.0,
                label: label.to_string(),
                position: 100.0,
            },
            Tick {
                value: 1.0,
                label: label.to_string(),
                position: 100.0 + reach + inside_minimum,
            },
        ];
        let refs: Vec<&Tick> = too_close.iter().collect();
        assert!(
            !labels_clear_horizontally(&refs, LABEL_SIZE, tile_width),
            "a {inside_minimum}px gap is half of LABEL_CLEARANCE and should \
             not read as clear — a mutation that zeroed LABEL_CLEARANCE \
             would accept this pair, which is what this test pins"
        );

        let at_minimum = [
            Tick {
                value: 0.0,
                label: label.to_string(),
                position: 100.0,
            },
            Tick {
                value: 1.0,
                label: label.to_string(),
                position: 100.0 + reach + LABEL_CLEARANCE,
            },
        ];
        let refs_ok: Vec<&Tick> = at_minimum.iter().collect();
        assert!(
            labels_clear_horizontally(&refs_ok, LABEL_SIZE, tile_width),
            "a gap exactly at LABEL_CLEARANCE should read as clear"
        );
    }

    // ------------------------------------------------------------------
    // Where each axis part is — the extents the draw reports.
    // ------------------------------------------------------------------

    /// The point each glyph run was drawn from, as the scene encodes it: a run's
    /// translation is where its first glyph's baseline starts (a rotated run,
    /// its pivot).
    fn drawn_run_origins(scene: &Scene) -> Vec<Point> {
        scene
            .encoding()
            .resources
            .glyph_runs
            .iter()
            .map(|run| {
                Point::new(
                    f64::from(run.transform.translation[0]),
                    f64::from(run.transform.translation[1]),
                )
            })
            .collect()
    }

    fn linear_x(layout: &ChartLayout) -> Scale {
        Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: layout.plot_x_start(),
            range_end: layout.plot_x_end(),
        }
    }

    /// The x axis reports its line reaching `LINE_REACH` into the data area and
    /// ending where its labels begin, its labels in the strip under the tick
    /// marks and its title under them, centred and as wide as it was drawn; and
    /// every run the scene holds was drawn from inside one of the rects.
    #[test]
    fn the_x_axis_reports_where_it_drew_its_line_labels_and_title() {
        let layout = ChartLayout::new(640.0, 480.0);
        let ticks = compute_ticks(&linear_x(&layout), 5);
        let mut scene = Scene::new();
        let axis = render_x_axis(
            &mut scene,
            &layout,
            &ticks,
            Some("Arrival Delay"),
            ChartInk::LIGHT,
        );

        let y = layout.plot_y_end();
        assert_eq!(
            axis.line,
            Rect::new(
                layout.plot_x_start(),
                y - 8.0,
                layout.plot_x_end(),
                y + TICK_LENGTH
            ),
            "the line's strip reaches 8 px up into the data area and down to the labels"
        );
        let labels = axis.labels.expect("the ticks were labelled");
        let title = axis.title.expect("the axis was titled");
        assert_eq!(
            labels.y0, axis.line.y1,
            "the labels begin where the line's strip ends"
        );
        assert!(
            title.y0 >= labels.y1,
            "the title ({title:?}) is under the labels ({labels:?})"
        );
        let centre = (layout.plot_x_start() + layout.plot_x_end()) / 2.0;
        assert!((title.center().x - centre).abs() < 1e-9, "centred");
        assert!(
            (title.width() - measure_width("Arrival Delay", TITLE_SIZE)).abs() < 1e-9,
            "as wide as the text drew"
        );

        let origins = drawn_run_origins(&scene);
        assert_eq!(
            origins.len(),
            ticks.len() + 1,
            "a run per label and the title"
        );
        for at in &origins {
            assert!(
                labels.contains(*at) || title.contains(*at),
                "a run drawn from {at:?} lies in neither the labels {labels:?} nor the title {title:?}"
            );
        }
        assert_eq!(
            origins.iter().filter(|at| labels.contains(**at)).count(),
            ticks.len(),
            "every label was drawn inside the strip reported for them"
        );
    }

    /// The y axis reports its line reaching `LINE_REACH` into the data area on
    /// one side and ending at its labels on the other, its labels to the left of
    /// the tick marks and its rotated title left of them.
    #[test]
    fn the_y_axis_reports_where_it_drew_its_line_labels_and_title() {
        // The left margin a titled plot grows, so the title has its own band.
        let margins = Margins {
            left: 64.0,
            ..Margins::default()
        };
        let layout = ChartLayout::with_margins_and_insets(640.0, 480.0, margins, Insets::default());
        let scale = Scale::Linear {
            domain_min: 0.0,
            domain_max: 100.0,
            range_start: layout.plot_y_end(),
            range_end: layout.plot_y_start(),
        };
        let ticks = compute_ticks(&scale, 5);
        let mut scene = Scene::new();
        let axis = render_y_axis(
            &mut scene,
            &layout,
            &ticks,
            Some("Travelers"),
            ChartInk::LIGHT,
        );

        let x = layout.plot_x_start();
        assert_eq!(
            axis.line,
            Rect::new(
                x - TICK_LENGTH - 3.0,
                layout.plot_y_start(),
                x + 8.0,
                layout.plot_y_end()
            ),
            "the line's strip reaches 8 px right into the data area and left to the labels"
        );
        let labels = axis.labels.expect("the ticks were labelled");
        let title = axis.title.expect("the axis was titled");
        assert_eq!(
            labels.x1, axis.line.x0,
            "the labels end where the line's strip begins"
        );
        assert!(
            title.x1 <= labels.x0,
            "the title ({title:?}) is left of the labels ({labels:?})"
        );
        let widest = ticks
            .iter()
            .map(|t| measure_width(&t.label, LABEL_SIZE))
            .fold(0.0_f64, f64::max);
        assert!(
            (labels.width() - widest).abs() < 1e-9,
            "the strip is as wide as the widest label"
        );
        assert!(
            (title.height() - measure_width("Travelers", TITLE_SIZE)).abs() < 1e-9,
            "a rotated title runs as long as the text is wide"
        );

        let origins = drawn_run_origins(&scene);
        assert_eq!(
            origins.len(),
            ticks.len() + 1,
            "a run per label and the title"
        );
        for at in &origins {
            assert!(
                labels.contains(*at) || title.contains(*at),
                "a run drawn from {at:?} lies in neither the labels {labels:?} nor the title {title:?}"
            );
        }
    }

    /// An axis that drew no title or no labels reports none, and an empty title
    /// is none: there is no text to click on.
    #[test]
    fn an_axis_reports_no_part_it_did_not_draw() {
        let layout = ChartLayout::new(640.0, 480.0);
        let mut scene = Scene::new();
        let bare = render_x_axis(&mut scene, &layout, &[], None, ChartInk::LIGHT);
        assert_eq!((bare.title, bare.labels), (None, None));
        let empty = render_y_axis(&mut scene, &layout, &[], Some(""), ChartInk::LIGHT);
        assert_eq!((empty.title, empty.labels), (None, None));
        assert!(
            bare.line.area() > 0.0 && empty.line.area() > 0.0,
            "the line is there to click even with nothing else drawn"
        );
    }

    /// Labels the axis thinned away are not in the strip it reports: the strip
    /// is the union of the labels drawn, which `thinned_x_ticks` picks.
    #[test]
    fn a_thinned_x_axis_reports_only_the_labels_it_kept() {
        let layout = ChartLayout::new(400.0, 300.0);
        let ticks: Vec<Tick> = (0..10)
            .map(|i| Tick {
                value: f64::from(i),
                label: "1,000,000".into(),
                position: layout.plot_x_start() + 30.0 * f64::from(i),
            })
            .collect();
        let kept = thinned_x_ticks(&ticks, LABEL_SIZE, layout.width);
        assert!(
            kept.len() < ticks.len(),
            "fixture check: this crowding thins the axis"
        );

        let mut scene = Scene::new();
        let axis = render_x_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);
        let labels = axis.labels.expect("labels drawn");
        let width = measure_width("1,000,000", LABEL_SIZE);
        let first = kept.first().expect("kept").position;
        let last = kept.last().expect("kept").position;
        assert!((labels.x0 - (first - width / 2.0)).abs() < 1e-9);
        assert!((labels.x1 - (last + width / 2.0)).abs() < 1e-9);
        assert_eq!(drawn_run_origins(&scene).len(), kept.len());
    }

    /// An axis that rotates its labels reports the rotated runs' extent: as tall
    /// as the longest label is wide, not one line of text.
    #[test]
    fn a_rotated_x_axis_reports_the_height_of_its_rotated_labels() {
        let margins = Margins {
            bottom: 100.0,
            ..Margins::default()
        };
        let layout = ChartLayout::with_margins_and_insets(132.0, 300.0, margins, Insets::default());
        let ticks = compute_ticks(&fixture_day_scale(&layout), 5);
        let mut scene = Scene::new();
        let axis = render_x_axis(&mut scene, &layout, &ticks, None, ChartInk::LIGHT);
        assert!(scene_has_quarter_turn(&scene), "fixture check: rotated");

        let labels = axis.labels.expect("labels drawn");
        let widest = ticks
            .iter()
            .map(|t| measure_width(&t.label, LABEL_SIZE))
            .fold(0.0_f64, f64::max);
        assert!(
            (labels.height() - widest).abs() < 1e-9,
            "{labels:?} is not as tall as the widest label, {widest}"
        );
        for at in drawn_run_origins(&scene) {
            assert!(labels.contains(at), "{at:?} is outside {labels:?}");
        }
    }
}
