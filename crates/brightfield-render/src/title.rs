//! Axis + plot titles — resolve the spec's Override/Suppress/Derive decisions
//! against the plot's channel maps into concrete title text, and grow the plot
//! margins to reserve a fixed band for each present title.
//!
//! The DERIVE decision (from [`brightfield_spec::layout::resolve_axis_titles`])
//! is turned into a field name HERE — where the channel map lives — because the
//! spec crate can't see the render `ChannelMap`. The resolved titles then ride a
//! render arg + a `LivePlot` field (not `ChartLayout`, which stays `Copy`), and
//! are captured in the `ChromeSnapshot` gate.

use brightfield_spec::ast::PlotNode;
use brightfield_spec::layout::{resolve_axis_titles, AxisTitle};

use crate::channel::{Channel, ChannelMap};
use crate::layout::Margins;
use crate::text::TITLE_SIZE;

/// A plot's fully-resolved titles: the concrete text (or `None`) for each
/// positional axis and the plot, ready to render and to size margins. A
/// `Suppress`, an underivable (interval-only / augment-only), or an absent axis
/// is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedTitles {
    /// The x-axis title, if any (drawn below the tick labels).
    pub x: Option<String>,
    /// The y-axis title, if any (drawn rotated up the left margin).
    pub y: Option<String>,
    /// The per-plot title, if any (drawn above the frame).
    pub plot: Option<String>,
}

impl ResolvedTitles {
    /// True when the plot carries no title at all (no margin growth, no ink).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.x.is_none() && self.y.is_none() && self.plot.is_none()
    }
}

/// Prefix on every column a lowerer synthesises — `__bf_count`, `__bf_bin_x2`,
/// `__bf_hex_dx`. Deriving a title from one would put an internal alias in
/// front of a reader: a histogram counting rows would label its y-axis
/// `__bf_count`, which names nothing the author wrote.
const RESERVED_COLUMN_PREFIX: &str = "__bf_";

/// The reserved column a counting aggregate lands in, and the axis title it
/// earns. The same string is spelled out by a const in each crate that emits
/// it — `channel::AGGREGATE_COUNT_COL` and `mark::DENSITY_COUNT_COL` here,
/// `HEX_COUNT_COL` in `brightfield-sql` — all of them private, which is why
/// this matches on the literal rather than importing one.
///
/// Suppression is the right default for a reserved column, but it is the wrong
/// answer for this one: `__bf_count` is the ONLY synthesised column whose
/// meaning has a name in the reader's language. A histogram's y-axis is
/// counting rows, so it says `Count`.
///
/// **This is brightfield choosing a word, not matching one.** Mosaic-web draws
/// that axis untitled: it bins in SQL and hands Plot a bare column array
/// (`markPlotSpec` passes `channelOption`'s value and nothing else), so Plot
/// has no field name to label from **[V, read from the vendored checkout]**.
/// Brightfield derives axis titles itself, which is what makes any word
/// reachable here. `Count` is the obvious English for it; whether Observable
/// Plot's own `binX`/`groupX` reducers use that exact string is UNVERIFIED —
/// Plot is not vendored in this tree, and nothing here should be read as
/// claiming a match with it.
const COUNT_COLUMN: &str = "__bf_count";
/// The axis title [`COUNT_COLUMN`] resolves to.
const COUNT_TITLE: &str = "Count";

/// Resolve one axis's decision against the mark channel maps. A `Derive` axis
/// takes the FIRST map (in mark order) that binds the positional channel to a
/// COLUMN the author could have named — subsuming the `entries[0]` default and
/// covering a literal-first mark (e.g. a `ruleY` at a constant) whose bound
/// sibling names the axis. An interval-only axis (`x1`/`x2`, `y1`/`y2` — no
/// plain `x`/`y` column), an augment-only axis, and an axis bound only to a
/// reserved lowerer output ([`RESERVED_COLUMN_PREFIX`]) each name no field, so
/// `Derive` yields `None` — except [`COUNT_COLUMN`], which names a quantity
/// rather than a field and titles its axis [`COUNT_TITLE`].
fn resolve_axis(decision: &AxisTitle, channel: Channel, maps: &[&ChannelMap]) -> Option<String> {
    match decision {
        AxisTitle::Override(s) => Some(s.clone()),
        AxisTitle::Suppress => None,
        AxisTitle::Derive => maps.iter().find_map(|m| {
            let col = m.get(channel)?;
            if col == COUNT_COLUMN {
                return Some(COUNT_TITLE.to_string());
            }
            (!col.starts_with(RESERVED_COLUMN_PREFIX)).then(|| col.to_string())
        }),
    }
}

/// Resolve a plot's [`ResolvedTitles`] from its attributes + the mark channel
/// maps. The Override/Suppress/Derive decision comes from the pure spec resolver
/// ([`resolve_axis_titles`]); the Derive field name and the plot title text are
/// filled in here.
#[must_use]
pub fn resolve_titles(plot: &PlotNode, channel_maps: &[&ChannelMap]) -> ResolvedTitles {
    let decided = resolve_axis_titles(plot);
    ResolvedTitles {
        x: resolve_axis(&decided.x, Channel::X, channel_maps),
        y: resolve_axis(&decided.y, Channel::Y, channel_maps),
        plot: decided.plot,
    }
}

/// Fixed title-band width added to a margin for one present title: the font
/// height plus padding. A title runs ALONG its axis, so the extent it consumes
/// IN the margin is this constant cross-axis band, independent of text length.
pub const TITLE_BAND: f64 = TITLE_SIZE as f64 + 8.0;

/// Grow the base margins to reserve a fixed [`TITLE_BAND`] for each present
/// title: left for a y-title, bottom for an x-title, top for a plot title.
/// Absent titles leave that side at its base (Observable-default) value; the
/// right margin never grows. Both layout models feed this the same base +
/// titles, so their grown margins agree per-side.
#[must_use]
pub fn grow_margins(base: Margins, titles: &ResolvedTitles) -> Margins {
    Margins {
        left: base.left + if titles.y.is_some() { TITLE_BAND } else { 0.0 },
        bottom: base.bottom + if titles.x.is_some() { TITLE_BAND } else { 0.0 },
        top: base.top
            + if titles.plot.is_some() {
                TITLE_BAND
            } else {
                0.0
            },
        right: base.right,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::ast::{PlotNode, SpecValue};
    use indexmap::IndexMap;

    fn plot_with(attrs: &[(&str, SpecValue)]) -> PlotNode {
        let mut attributes = IndexMap::new();
        for (k, v) in attrs {
            attributes.insert((*k).to_string(), v.clone());
        }
        PlotNode {
            items: vec![],
            attributes,
        }
    }

    fn map_cols(cols: &[(Channel, &str)]) -> ChannelMap {
        let mut m = ChannelMap::new();
        for (c, col) in cols {
            m.insert(*c, (*col).to_string());
        }
        m
    }

    #[test]
    fn derive_override_suppress_and_interval() {
        // Bare plot, column-bound x/y → derived field names.
        let cm = map_cols(&[(Channel::X, "temp"), (Channel::Y, "power")]);
        let t = resolve_titles(&plot_with(&[]), &[&cm]);
        assert_eq!(t.x.as_deref(), Some("temp"));
        assert_eq!(t.y.as_deref(), Some("power"));

        // Override wins; explicit null suppresses (→ None, no band).
        let t = resolve_titles(
            &plot_with(&[
                ("xLabel", SpecValue::String("Temperature (°C)".into())),
                ("yLabel", SpecValue::Null),
            ]),
            &[&cm],
        );
        assert_eq!(t.x.as_deref(), Some("Temperature (°C)"));
        assert_eq!(t.y, None);

        // Interval-only axis (x1/x2, no plain x) → no derived title.
        let interval = map_cols(&[
            (Channel::X1, "lo"),
            (Channel::X2, "hi"),
            (Channel::Y, "count"),
        ]);
        let t = resolve_titles(&plot_with(&[]), &[&interval]);
        assert_eq!(
            t.x, None,
            "interval-only x names no single field → no title"
        );
        assert_eq!(t.y.as_deref(), Some("count"));
    }

    /// A reserved lowerer output names no field, so it never becomes a title.
    ///
    /// The computed histogram is the case: its `x2` is bound to `__bf_bin_x2`,
    /// the high bin edge the SQL layer synthesises, and deriving from it would
    /// print an internal alias in front of a reader. Its `x` is bound to the
    /// column the author actually wrote, and must still title the axis.
    #[test]
    fn a_reserved_lowerer_column_names_no_axis() {
        // An interval-only x whose high edge is reserved: neither half may
        // title the axis, and the reserved name must not leak.
        let edges_only = map_cols(&[(Channel::X1, "lo"), (Channel::X2, "__bf_bin_x2")]);
        let t = resolve_titles(&plot_with(&[]), &[&edges_only]);
        assert_eq!(t.x, None, "an interval-only x names no single field");

        let histogram = map_cols(&[
            (Channel::X, "delay"),
            (Channel::X1, "delay"),
            (Channel::X2, "__bf_bin_x2"),
            (Channel::Y, "__bf_count"),
        ]);
        let t = resolve_titles(&plot_with(&[]), &[&histogram]);
        assert_eq!(t.x.as_deref(), Some("delay"), "the binned column titles x");

        // A reserved binding on a channel a sibling DOES bind to a real column
        // is skipped over, not treated as an answer.
        let reserved_y = map_cols(&[(Channel::Y, "__bf_hex_dy")]);
        let sibling = map_cols(&[(Channel::Y, "level")]);
        let t = resolve_titles(&plot_with(&[]), &[&reserved_y, &sibling]);
        assert_eq!(t.y.as_deref(), Some("level"));
    }

    /// The counting axis says `Count`.
    ///
    /// `__bf_count` is reserved, so the blanket suppression above would leave a
    /// computed histogram's y-axis untitled — the axis whose whole subject is
    /// how many rows fell in each bin, with no word for it. It is the one
    /// synthesised column that names a quantity a reader has a word for, so it
    /// is mapped rather than suppressed. Every other `__bf_` column stays
    /// suppressed, which the assertions above pin.
    #[test]
    fn the_count_column_titles_its_axis_count() {
        let histogram = map_cols(&[
            (Channel::X, "delay"),
            (Channel::X1, "delay"),
            (Channel::X2, "__bf_bin_x2"),
            (Channel::Y, "__bf_count"),
        ]);
        let t = resolve_titles(&plot_with(&[]), &[&histogram]);
        assert_eq!(
            t.y.as_deref(),
            Some("Count"),
            "a histogram's y-axis counts rows and must say so"
        );
        assert_eq!(t.x.as_deref(), Some("delay"), "x is unaffected");

        // The transpose: a rectX histogram counts along x.
        let transposed = map_cols(&[
            (Channel::Y, "delay"),
            (Channel::Y1, "delay"),
            (Channel::Y2, "__bf_bin_y2"),
            (Channel::X, "__bf_count"),
        ]);
        let t = resolve_titles(&plot_with(&[]), &[&transposed]);
        assert_eq!(t.x.as_deref(), Some("Count"));

        // A mark that binds a real column to the same axis still wins if it
        // comes first — the count title is a derivation, not an override.
        let sibling = map_cols(&[(Channel::Y, "level")]);
        let t = resolve_titles(&plot_with(&[]), &[&sibling, &histogram]);
        assert_eq!(t.y.as_deref(), Some("level"));

        // An explicit override still wins — the mapping is about DERIVE only.
        let t = resolve_titles(
            &plot_with(&[("yLabel", SpecValue::String("Flights".into()))]),
            &[&histogram],
        );
        assert_eq!(t.y.as_deref(), Some("Flights"));

        // …and an explicit null still suppresses it.
        let t = resolve_titles(&plot_with(&[("yLabel", SpecValue::Null)]), &[&histogram]);
        assert_eq!(t.y, None);
    }

    #[test]
    fn multimark_first_column_bound_entry_names_the_axis() {
        // First mark binds y to a literal only (no y column); the second binds
        // y to `level`. Derive must pick the bound sibling, not drop the title.
        let mut literal_first = ChannelMap::new();
        literal_first.insert(Channel::X, "date".into());
        literal_first.insert_literal(Channel::Y, 30.0);
        let bound = map_cols(&[(Channel::X, "date"), (Channel::Y, "level")]);
        let t = resolve_titles(&plot_with(&[]), &[&literal_first, &bound]);
        assert_eq!(
            t.y.as_deref(),
            Some("level"),
            "first column-bound entry names y"
        );
        assert_eq!(t.x.as_deref(), Some("date"));
    }

    #[test]
    fn grow_margins_reserves_a_band_per_present_title() {
        let base = Margins::default();
        // All three present → left/bottom/top each grow by the band; right holds.
        let all = ResolvedTitles {
            x: Some("x".into()),
            y: Some("y".into()),
            plot: Some("p".into()),
        };
        let g = grow_margins(base, &all);
        assert!((g.left - (base.left + TITLE_BAND)).abs() < f64::EPSILON);
        assert!((g.bottom - (base.bottom + TITLE_BAND)).abs() < f64::EPSILON);
        assert!((g.top - (base.top + TITLE_BAND)).abs() < f64::EPSILON);
        assert!(
            (g.right - base.right).abs() < f64::EPSILON,
            "right never grows"
        );

        // No titles → margins unchanged (an untitled plot is byte-identical).
        let none = ResolvedTitles::default();
        let g0 = grow_margins(base, &none);
        assert!((g0.left - base.left).abs() < f64::EPSILON);
        assert!((g0.bottom - base.bottom).abs() < f64::EPSILON);
        assert!((g0.top - base.top).abs() < f64::EPSILON);
        assert!(none.is_empty());
    }
}
