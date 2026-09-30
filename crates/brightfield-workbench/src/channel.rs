//! The channels a chart's shelf names, the hue each one wears, and the extents
//! of the band that holds one cell for each.
//!
//! A shelf cell says which column a channel of the chart's mark takes. The
//! chart kind's own slots do not decide the cells: the generated map's roles
//! are `lon` and `lat`, and the dot plot it becomes when x takes a column that
//! is not a coordinate has the same four channels, so the band reads the same
//! over both. The four are the mark, x, y and colour.
//!
//! # Which hue marks which channel
//!
//! The palette, the strength of a tint and the rule that a hue marks a kind of
//! data are the design system's (`meridian_design::viz`); **which channel takes
//! which hue is this crate's, and [`hue`] is the one function that says so.**
//! The shell's band, the bar under its open cell and any marker a later surface
//! draws for a channel read it from here, so two surfaces cannot tint one
//! channel differently.
//!
//! In the design system's categorical order (blue, gold, teal, red, violet,
//! orange, plum, green) the mark is plum, x is teal, y is violet and colour is
//! orange. Blue and gold, the first two, stay with the two measures of a chart
//! whose y holds two columns, so no channel here takes either. Red stays with
//! the status inks' critical and green with the status inks' good.
//! `the_channels_take_four_distinct_hues_and_leave_blue_and_gold_to_the_measures`
//! holds that.
//!
//! # The band's extents
//!
//! [`BAND_HEIGHT`] and [`MARK_CELL_WIDTH`] are the band's own numbers, and
//! [`cell_widths`] is the split of a band's width into its four cells. The
//! split reads the width and nothing else, so a cell keeps its width when the
//! column it names changes. No egui type appears in this file.

use meridian_design::{viz, Rgba};

use crate::Mode;

/// One channel of a chart's shelf, in the order the band draws them, left to
/// right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShelfChannel {
    /// The mark: dot, line, bar.
    Mark,
    /// The x axis.
    X,
    /// The y axis.
    Y,
    /// The colour of the marks, which a spec writes as `fill`.
    Colour,
}

impl ShelfChannel {
    /// The four channels, in the order the band draws them.
    pub const ALL: [ShelfChannel; 4] = [
        ShelfChannel::Mark,
        ShelfChannel::X,
        ShelfChannel::Y,
        ShelfChannel::Colour,
    ];

    /// Where the channel's cell stands in the band, counted from the left.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            ShelfChannel::Mark => 0,
            ShelfChannel::X => 1,
            ShelfChannel::Y => 2,
            ShelfChannel::Colour => 3,
        }
    }

    /// The word an analyst calls the channel, which is not the word the file
    /// writes: the file says `fill` where the shelf says `colour`.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            ShelfChannel::Mark => "mark",
            ShelfChannel::X => "x axis",
            ShelfChannel::Y => "y axis",
            ShelfChannel::Colour => "colour",
        }
    }

    /// The channel's place in the design system's categorical order, counted
    /// from zero: plum is the seventh colour, teal the third, violet the fifth
    /// and orange the sixth.
    #[must_use]
    pub const fn palette_slot(self) -> usize {
        match self {
            ShelfChannel::Mark => 6,
            ShelfChannel::X => 2,
            ShelfChannel::Y => 4,
            ShelfChannel::Colour => 5,
        }
    }
}

/// The hue a channel wears: its slot of the design system's categorical
/// palette for `mode`.
///
/// The one function that assigns a hue to a channel. The hue is the colour's own
/// strength, which the bar under an open cell takes; a tint over a surface is
/// [`tint`].
#[must_use]
pub fn hue(channel: ShelfChannel, mode: Mode) -> Rgba {
    let palette = if mode.is_dark() {
        viz::CATEGORICAL_DARK
    } else {
        viz::CATEGORICAL_LIGHT
    };
    palette[channel.palette_slot()]
}

/// A channel's hue at the design system's tint strength for `mode`, the ground
/// of its cell.
#[must_use]
pub fn tint(channel: ShelfChannel, mode: Mode) -> Rgba {
    viz::chrome_tint(hue(channel, mode), mode.is_dark())
}

/// The band's height in logical points, the same at every width.
pub const BAND_HEIGHT: f32 = 44.0;

/// The mark's cell, in logical points. The other three share the rest of the
/// band equally.
pub const MARK_CELL_WIDTH: f32 = 88.0;

/// The widths of the band's four cells, in [`ShelfChannel::ALL`] order, at a
/// band `band_width` wide.
///
/// The mark's cell takes [`MARK_CELL_WIDTH`] and x, y and colour take a third
/// of what is left, so the four sum to the band's width. A band narrower than
/// the mark's cell gives that cell the whole band and the others none.
#[must_use]
pub fn cell_widths(band_width: f32) -> [f32; 4] {
    let mark = MARK_CELL_WIDTH.min(band_width.max(0.0));
    let rest = (band_width - mark).max(0.0) / 3.0;
    [mark, rest, rest, rest]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channels_take_four_distinct_hues_and_leave_blue_and_gold_to_the_measures() {
        for mode in [Mode::Light, Mode::Dark] {
            let palette = if mode.is_dark() {
                viz::CATEGORICAL_DARK
            } else {
                viz::CATEGORICAL_LIGHT
            };
            let hues: Vec<Rgba> = ShelfChannel::ALL.iter().map(|c| hue(*c, mode)).collect();
            for (i, a) in hues.iter().enumerate() {
                for b in &hues[i + 1..] {
                    assert_ne!(a, b, "two channels share a hue in {mode:?}");
                }
                assert_ne!(*a, palette[0], "a channel took blue in {mode:?}");
                assert_ne!(*a, palette[1], "a channel took gold in {mode:?}");
            }
        }
    }

    #[test]
    fn a_tint_is_the_hue_at_the_design_systems_strength() {
        for mode in [Mode::Light, Mode::Dark] {
            for channel in ShelfChannel::ALL {
                let (h, t) = (hue(channel, mode), tint(channel, mode));
                assert_eq!((t.r, t.g, t.b), (h.r, h.g, h.b), "{channel:?} {mode:?}");
                assert_eq!(
                    t,
                    viz::chrome_tint(h, mode.is_dark()),
                    "{channel:?} {mode:?} is not at the design system's strength"
                );
            }
        }
    }

    #[test]
    fn the_cells_sum_to_the_band_and_the_mark_takes_its_own_width() {
        let w = cell_widths(578.0);
        assert!((w[0] - 88.0).abs() < 1e-4, "{w:?}");
        for share in &w[1..] {
            assert!((share - 163.333_33).abs() < 1e-3, "{w:?}");
        }
        assert!((w.iter().sum::<f32>() - 578.0).abs() < 1e-3, "{w:?}");
    }

    #[test]
    fn a_band_narrower_than_the_mark_cell_gives_the_mark_the_whole_of_it() {
        assert_eq!(cell_widths(60.0), [60.0, 0.0, 0.0, 0.0]);
        assert_eq!(cell_widths(0.0), [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn the_cells_widths_do_not_depend_on_anything_but_the_width() {
        // The same width asked twice is the same split, which is what keeps a
        // cell where it is when the column it names changes.
        assert_eq!(cell_widths(700.0), cell_widths(700.0));
        assert!(cell_widths(700.0)[1] > cell_widths(578.0)[1]);
    }
}
