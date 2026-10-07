//! Counting the rows that lie past an axis end the file fixed, and saying so at
//! that end.
//!
//! A plot whose file writes `xDomain: [0, 100]` draws its x axis from 0 to 100
//! whatever its rows span, and a dot at 150 is clipped at the frame and drawn
//! nowhere. Without a count the reader cannot tell a picture that holds every
//! row from one with a hundred off the end. So each end of a fixed axis that
//! rows fell past carries a mark in the warning ink and the number of those
//! rows, outside the data area: a triangle pointing past the end, and the count
//! beside it.
//!
//! # Where it is drawn
//!
//! In the axis's title row: the x counts on the x title's baseline, at the
//! plot's left and right edges, and the y counts in the y title's column,
//! rotated as the title is, at the plot's top and bottom edges. The title is
//! centred on its axis and the counts sit at its ends, so the two share the row.
//! An axis whose title is suppressed has no row there, and
//! [`rows_past_band_margins`] reserves one for it, the band a title would take,
//! when the file fixes that axis's ends and a mark that counts is drawn. That
//! reservation reads only the file, so a brush that moves the count does not
//! move the frame.
//!
//! # Which rows are counted
//!
//! The rows of the marks the caller hands in, read from each mark's own batch
//! on the channel the axis draws: a value below the axis's low end is counted
//! at that end, and one above its high end at the other. A null or a NaN is not
//! past either end. A mark whose batch holds bins, groups or summary rows has
//! no rows to count, and the caller leaves it out: the shell hands in the dot
//! marks whose plan returns rows one for one
//! (`brightfield_sql::emit::plan_returns_rows`).
//!
//! The count is of the rows the plot then draws: after a brush on another tile
//! filters this plot, it counts what is left, and under a sample it counts the
//! sample, which the sampling notice below the plot says the picture is.

use brightfield_spec::vocab::MarkKind;
use kurbo::{Affine, BezPath, Point};
use peniko::{Color, Fill};
use vello::Scene;

use crate::axis::{x_title_baseline, Y_TITLE_X};
use crate::channel::Channel;
use crate::ink::ChartInk;
use crate::layout::{ChartLayout, Margins};
use crate::mark::column_as_f64;
use crate::scale::{Scale, ScaleSet};
use crate::scene::ChartData;
use crate::text::{draw_text, draw_text_rotated, TextAnchor, LABEL_SIZE};
use crate::title::{ResolvedTitles, TITLE_BAND};

/// Which positional axes hold the ends the file wrote as two numbers.
///
/// For the margins, this is what the file says; for the count, it is what the
/// draw held: an axis the reader has panned or zoomed, or one whose scale two
/// numbers do not fix (a date axis, an axis of names), is not held, and draws
/// no count (`crate::scene::written_ends_held`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FixedEnds {
    /// The x axis's ends are the file's.
    pub x: bool,
    /// The y axis's ends are the file's.
    pub y: bool,
}

impl FixedEnds {
    /// Neither axis is fixed by the file.
    #[must_use]
    pub fn is_empty(self) -> bool {
        !self.x && !self.y
    }
}

/// How many rows lie past each end of one axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EndCounts {
    /// Rows whose value is below the axis's low end.
    pub low: u64,
    /// Rows whose value is above the axis's high end.
    pub high: u64,
}

/// How many rows lie past each end of each positional axis the file fixed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PastEnds {
    /// The x axis's ends.
    pub x: EndCounts,
    /// The y axis's ends.
    pub y: EndCounts,
}

impl PastEnds {
    /// No row lies past any end.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// Whether a mark of `kind` counts its rows past a fixed axis end: the dots,
/// `dot`, `dotX`, `dotY` and `circle`, which draw one mark per row.
///
/// The kind is half the judgement. A dot whose channels aggregate or bin draws
/// one mark per group, so its caller also asks whether the mark's plan returns
/// rows. Every other kind draws no count in this build: a binned, raster or
/// aggregating kind because its batch holds no rows to count, and a line, a
/// text, a rule or a tick, which do draw rows, because the count was built for
/// the dot and the others wait on a change of their own. `deviations.yaml`
/// records the list.
#[must_use]
pub fn mark_counts_rows_past(kind: MarkKind) -> bool {
    matches!(
        kind,
        MarkKind::Dot | MarkKind::DotX | MarkKind::DotY | MarkKind::Circle
    )
}

/// The length of the triangle along its axis, in logical pixels.
const MARK_LENGTH: f64 = 6.0;
/// Half the triangle's width across its axis.
const MARK_HALF_WIDTH: f64 = 3.5;
/// The space between the triangle and the number.
const MARK_GAP: f64 = 3.0;
/// How far the middle of the count's figures sits from its baseline: half the
/// cap height of the label face at [`LABEL_SIZE`], so the triangle is centred
/// on the figures.
const FIGURE_MIDDLE: f64 = 4.0;

/// Count, for each axis `fixed` names, the rows of `entries` past that axis's
/// ends as `scales` draws them.
///
/// An axis `fixed` does not name counts nothing, and so does one whose scale
/// has no numeric domain. Each entry is read on the channel the axis draws; an
/// entry that maps no column there adds nothing to that axis.
#[must_use]
pub fn count_rows_past(
    entries: &[&ChartData<'_>],
    scales: &ScaleSet,
    fixed: FixedEnds,
) -> PastEnds {
    let axis = |on: bool, channel: Channel| {
        if !on {
            return EndCounts::default();
        }
        scales
            .get(channel)
            .map(|scale| count_axis(entries, scale, channel))
            .unwrap_or_default()
    };
    PastEnds {
        x: axis(fixed.x, Channel::X),
        y: axis(fixed.y, Channel::Y),
    }
}

fn count_axis(entries: &[&ChartData<'_>], scale: &Scale, channel: Channel) -> EndCounts {
    let (Some(lo), Some(hi)) = (scale.domain_min(), scale.domain_max()) else {
        return EndCounts::default();
    };
    let mut counts = EndCounts::default();
    for entry in entries {
        let Some(column) = entry.channel_map.get(channel) else {
            continue;
        };
        let Some(values) = column_as_f64(entry.batch, column) else {
            continue;
        };
        for value in values.into_iter().flatten() {
            if value < lo {
                counts.low += 1;
            } else if value > hi {
                counts.high += 1;
            }
        }
    }
    counts
}

/// Grow `margins` to reserve the title row an axis's counts are drawn in, for
/// each axis `fixed` names that has no title to have reserved it already.
///
/// Called with what the file fixes, not with what the rows do, so the frame is
/// the same with a count drawn and without one. A plot `fixed` names no axis of
/// keeps its margins as they are.
#[must_use]
pub fn rows_past_band_margins(
    margins: Margins,
    titles: &ResolvedTitles,
    fixed: FixedEnds,
) -> Margins {
    let band = |needed: bool| if needed { TITLE_BAND } else { 0.0 };
    Margins {
        bottom: margins.bottom + band(fixed.x && titles.x.is_none()),
        left: margins.left + band(fixed.y && titles.y.is_none()),
        ..margins
    }
}

/// Draw each non-zero count in `counts` at the end of its axis the rows fell
/// past, in the warning ink, outside the data area.
///
/// The low end of an axis is wherever its scale puts its low value, so on an
/// axis that runs from high to low the low end's count is drawn at the right,
/// or at the top. With no row past an end, nothing is drawn there.
pub fn render_rows_past(
    scene: &mut Scene,
    layout: &ChartLayout,
    scales: &ScaleSet,
    counts: PastEnds,
    ink: ChartInk,
) {
    if let Some(scale) = scales.get(Channel::X) {
        let low_at_left = scale.range_start() <= scale.range_end();
        for (n, low) in [(counts.x.low, true), (counts.x.high, false)] {
            if n > 0 {
                draw_x_count(scene, layout, n, low == low_at_left, ink.warning);
            }
        }
    }
    if let Some(scale) = scales.get(Channel::Y) {
        let low_at_bottom = scale.range_start() >= scale.range_end();
        for (n, low) in [(counts.y.low, true), (counts.y.high, false)] {
            if n > 0 {
                draw_y_count(scene, layout, n, low == low_at_bottom, ink.warning);
            }
        }
    }
}

/// One x count, on the x title's baseline, at the plot's left or right edge:
/// `◂ n` at the left and `n ▸` at the right, the triangle's point on the edge.
fn draw_x_count(scene: &mut Scene, layout: &ChartLayout, n: u64, at_left: bool, ink: Color) {
    let baseline = x_title_baseline(layout);
    let middle = baseline - FIGURE_MIDDLE;
    let (edge, inward) = if at_left {
        (layout.plot_x_start(), 1.0)
    } else {
        (layout.plot_x_end(), -1.0)
    };
    let base = edge + inward * MARK_LENGTH;
    triangle(
        scene,
        Point::new(edge, middle),
        Point::new(base, middle - MARK_HALF_WIDTH),
        Point::new(base, middle + MARK_HALF_WIDTH),
        ink,
    );
    let (at, anchor) = if at_left {
        (base + MARK_GAP, TextAnchor::Start)
    } else {
        (base - MARK_GAP, TextAnchor::End)
    };
    draw_text(scene, &grouped(n), at, baseline, LABEL_SIZE, ink, anchor);
}

/// One y count, in the y title's column and rotated as the title is, at the
/// plot's top or bottom edge, the triangle's point on the edge.
fn draw_y_count(scene: &mut Scene, layout: &ChartLayout, n: u64, at_bottom: bool, ink: Color) {
    // The rotated figures stand to the left of their baseline, so their middle
    // is that far left of the title's baseline x.
    let middle = Y_TITLE_X - FIGURE_MIDDLE;
    let (edge, inward) = if at_bottom {
        (layout.plot_y_end(), -1.0)
    } else {
        (layout.plot_y_start(), 1.0)
    };
    let base = edge + inward * MARK_LENGTH;
    triangle(
        scene,
        Point::new(middle, edge),
        Point::new(middle - MARK_HALF_WIDTH, base),
        Point::new(middle + MARK_HALF_WIDTH, base),
        ink,
    );
    // The run reads bottom to top: at the bottom edge it starts above the
    // triangle and runs up, and at the top edge it ends below the triangle.
    let (at, anchor) = if at_bottom {
        (base - MARK_GAP, TextAnchor::Start)
    } else {
        (base + MARK_GAP, TextAnchor::End)
    };
    draw_text_rotated(scene, &grouped(n), Y_TITLE_X, at, LABEL_SIZE, ink, anchor);
}

fn triangle(scene: &mut Scene, tip: Point, a: Point, b: Point, ink: Color) {
    let mut path = BezPath::new();
    path.move_to(tip);
    path.line_to(a);
    path.line_to(b);
    path.close_path();
    scene.fill(Fill::NonZero, Affine::IDENTITY, ink, None, &path);
}

/// `n` with its thousands grouped by commas: `1,234,567`.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_groups_its_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(12_345), "12,345");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    /// The band is reserved only where a fixed axis has no title row to share:
    /// a titled axis keeps its margin, and an axis the file does not fix keeps
    /// its margin whether or not it has a title.
    #[test]
    fn a_band_is_reserved_only_for_a_fixed_axis_with_no_title() {
        let base = Margins::default();
        let titled = ResolvedTitles {
            x: Some("a".into()),
            y: Some("b".into()),
            plot: None,
        };
        let untitled = ResolvedTitles::default();
        let both = FixedEnds { x: true, y: true };

        let kept = rows_past_band_margins(base, &titled, both);
        assert_eq!((kept.bottom, kept.left), (base.bottom, base.left));

        let grown = rows_past_band_margins(base, &untitled, both);
        assert_eq!(grown.bottom, base.bottom + TITLE_BAND);
        assert_eq!(grown.left, base.left + TITLE_BAND);
        assert_eq!((grown.top, grown.right), (base.top, base.right));

        let only_x = rows_past_band_margins(base, &untitled, FixedEnds { x: true, y: false });
        assert_eq!(only_x.bottom, base.bottom + TITLE_BAND);
        assert_eq!(only_x.left, base.left);

        let none = rows_past_band_margins(base, &untitled, FixedEnds::default());
        assert_eq!((none.bottom, none.left), (base.bottom, base.left));
    }
}
