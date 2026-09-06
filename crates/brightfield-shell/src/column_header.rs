//! The grid pane's **column header band**: what a table's header row draws
//! when the pane knows what its columns are made of.
//!
//! The plain header — the column's name in muted ink, one dense row tall — is
//! [`crate::data_grid`]'s and stays. This module is the band that replaces it
//! where the pane has the column's profile to draw: per column the finetype
//! glyph and the name over a tint of the storage type, a validity band with
//! its count, a picture of the distribution, the range, and the distinct
//! count — with the finetype leaf, the storage type and the fuller run of
//! statistics reserved for the taller of the two densities.
//!
//! # Two densities, one style function
//!
//! [`column_header_frame`] is the whole of the band's ink and geometry,
//! resolved from the token set for a [`GridDensity`] and a [`Mode`]. Every
//! colour the band paints comes off it, which
//! `the_shell_spells_no_colour_or_box_model_as_a_raw_literal` is what holds;
//! every row it stacks is summed into the band's extent, which
//! `the_band_extents_are_the_sums_of_the_rows_each_density_stacks` is what
//! holds. So "is this band on the token set?" is answered by reading one
//! function.
//!
//! Which density a pane draws at follows where the pane is rather than what is
//! in it: [`GridDensity::Compact`] under the hero in the canvas's pane group,
//! where the grid has a quarter of the canvas, and [`GridDensity::Full`] where
//! the grid IS the canvas's view of the node and has the whole of it.
//!
//! # The extents are summed, never stated
//!
//! [`ColumnHeaderFrame::extent`] adds the rows the density stacks. The frames
//! the contract was drawn from carry the two totals as constants — 70 and 127
//! at 1440 by 900 — and this file reproduces them by addition, so a row that
//! changes height moves the band with it instead of leaving the widget a
//! height nothing fills. The compact total moved from 57 to 70 when the
//! compact band gained its own distinct-count row — see
//! [`GridDensity::Compact`].

use meridian_design::colour::Rgba;
use meridian_design::{semantic, spacing, typography, viz};

use brightfield_engine::{Bars, ColumnMoments};
use brightfield_workbench::chrome;

use crate::design::Mode;
use crate::one_step::ColumnFacts;

// ---------------------------------------------------------------------------
// The two densities.
// ---------------------------------------------------------------------------

/// How much of a column's profile the header band has room to state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridDensity {
    /// Beneath the hero, in a pane that is a quarter of the canvas: the glyph
    /// and the name, the validity band, a rug, the range, and — its own row,
    /// below the range — the distinct count. Not a bar distribution: the rug
    /// stays the glance-level hint that a distribution exists, and what this
    /// density gains beyond it is the summaries a number can carry at this
    /// width, the distinct count first.
    Compact,
    /// The grid as the canvas's whole view of the node: the above, plus the
    /// finetype leaf and the storage type, a bar distribution in place of the
    /// rug, and the statistics.
    Full,
}

impl GridDensity {
    /// Whether this density states the leaf, the storage type and the
    /// statistics.
    #[must_use]
    pub const fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }
}

// ---------------------------------------------------------------------------
// The rows the band stacks.
// ---------------------------------------------------------------------------

/// The band's own vertical inset, above the first row and below the last.
const INSET_Y: f32 = spacing::SPACE_2;

/// The band's horizontal inset either side of a cell's content.
const INSET_X: f32 = spacing::SPACE_4;

/// The row carrying the glyph and the column's name.
const NAME_ROW: f32 = 16.0;

/// The row carrying the validity band and its count.
const VALIDITY_ROW: f32 = 12.0;

/// How tall the validity band itself is inside its row — a rule of colour
/// rather than a bar chart, so it reads as one line at any cell width.
const VALIDITY_BAND: f32 = 3.0;

/// The row carrying the finetype leaf and the storage type, at the full
/// density only.
const TYPES_ROW: f32 = 13.0;

/// The row the rug is drawn in, at the compact density.
const RUG_ROW: f32 = 12.0;

/// How tall the rug is inside that row.
const RUG_HEIGHT: f32 = 8.0;

/// The row the bar distribution is drawn in, at the full density. It fills its
/// row, with the rule under it drawn on the row's own bottom edge.
const DISTRIBUTION_ROW: f32 = 28.0;

/// The row carrying the minimum and the maximum, at each density.
///
/// The full density's is the frame's own 11. The compact density's is 9: the
/// frame paints the compact range with no increment at all, so the rows the
/// original contract ratified sum to 48 against its constant of 57, and the
/// nine points that constant implies are this row. See the pull request for
/// the fork. That 57 was the compact total before this file's distinct row
/// existed; [`DISTINCT_ROW`] is what moved it to 70.
const RANGE_ROW_FULL: f32 = 11.0;
const RANGE_ROW_COMPACT: f32 = 9.0;

/// One caption row — *mean* and *nulls*, *median* and *sd*, *n distinct* — at
/// the full density.
const CAPTION_ROW: f32 = 13.0;

/// The row carrying the distinct count, at the compact density's own row
/// below the range. Sized the same as [`CAPTION_ROW`] because it is the same
/// face at the same padding, drawn solo rather than paired the way the full
/// density's rows are — the width a compact column has at its floor is not
/// wide enough to set two captions side by side without them colliding.
const DISTINCT_ROW: f32 = 13.0;

/// How many of them the full density stacks.
const CAPTION_ROWS: usize = 3;

/// The narrowest a column may be drawn at each density, in logical points. A
/// column is its natural width or this, whichever is wider.
const FLOOR_COMPACT: f32 = 96.0;
const FLOOR_FULL: f32 = 128.0;

/// The alpha the valid segment of the validity band takes over the header
/// fill, out of 255 — the mark colour at 60%.
const VALID_ALPHA: u8 = 0x99;

/// The alpha a storage tint takes over the header fill, out of 255: 7% in
/// light and 10% in dark, the two the contract names.
const TINT_ALPHA_LIGHT: u8 = 0x12;
const TINT_ALPHA_DARK: u8 = 0x1a;

/// The floor a rug column's alpha is clamped up to, so a bucket holding one
/// row still shows.
const RUG_ALPHA_FLOOR: f32 = 0.12;

/// The glyph a coordinate column carries.
const GLYPH_COORDINATE: &str = "\u{b0}";

/// The glyph a numeric column carries.
const GLYPH_NUMBER: &str = "#";

// ---------------------------------------------------------------------------
// The style function.
// ---------------------------------------------------------------------------

/// Every ink and every extent the band draws with, for one density in one
/// mode.
///
/// Built once per table draw and handed to every cell, so two columns of one
/// band cannot resolve a token differently.
#[derive(Clone, Debug)]
pub struct ColumnHeaderFrame {
    /// Which density this frame is for.
    pub density: GridDensity,
    /// The band's own fill, under the storage tint.
    pub fill: egui::Color32,
    /// The hairline between one cell and the next.
    pub separator: egui::Color32,
    /// The rule under the whole band.
    pub rule: egui::Color32,
    /// The column's name.
    pub name: egui::Color32,
    /// The finetype glyph before it.
    pub glyph: egui::Color32,
    /// The finetype leaf, at the full density.
    pub leaf: egui::Color32,
    /// The storage type beside it.
    pub storage: egui::Color32,
    /// The minimum and the maximum.
    pub range: egui::Color32,
    /// The statistics rows.
    pub caption: egui::Color32,
    /// The count at the trailing end of the validity band.
    pub count: egui::Color32,
    /// The valid share of the validity band.
    pub valid: egui::Color32,
    /// The invalid share. Declared here and drawn at whatever width a
    /// per-column invalid count gives it, which on a file whose type source
    /// reports none is zero.
    pub invalid: egui::Color32,
    /// The missing share.
    pub missing: egui::Color32,
    /// A distribution bar.
    pub bar: egui::Color32,
    /// One tint per storage type, over [`ColumnHeaderFrame::fill`].
    pub tints: [egui::Color32; 8],
    /// The mark colour as a token, which the rug takes at a per-column alpha —
    /// see [`ColumnHeaderFrame::rug_ink`]. Held as the token rather than as an
    /// `egui::Color32` because alpha is made on the token and converted once.
    mark: Rgba,
}

/// The band's style at `density` in `mode`, on the token set.
///
/// The one place this module resolves a colour. A caller that wants the rug's
/// per-bucket ink asks [`ColumnHeaderFrame::rug_ink`] rather than reaching for
/// the mark token itself.
#[must_use]
pub fn column_header_frame(density: GridDensity, mode: Mode) -> ColumnHeaderFrame {
    let dark = mode.is_dark();
    let sem = semantic(dark);
    let mark = if dark {
        viz::MARK_DEFAULT_DARK
    } else {
        viz::MARK_DEFAULT_LIGHT
    };
    let palette = if dark {
        viz::CATEGORICAL_DARK
    } else {
        viz::CATEGORICAL_LIGHT
    };
    let tint_alpha = if dark {
        TINT_ALPHA_DARK
    } else {
        TINT_ALPHA_LIGHT
    };
    let mut tints = [egui::Color32::TRANSPARENT; 8];
    for (slot, token) in tints.iter_mut().zip(palette.iter()) {
        *slot = chrome::colour(token.with_alpha_u8(tint_alpha));
    }
    ColumnHeaderFrame {
        density,
        fill: chrome::colour(sem.rows.header_background),
        separator: chrome::colour(sem.rows.row_border),
        rule: chrome::colour(sem.borders.default_),
        name: chrome::colour(sem.text.primary),
        glyph: chrome::colour(sem.text.muted),
        leaf: chrome::colour(sem.text.secondary),
        storage: chrome::colour(sem.text.muted),
        range: chrome::colour(sem.text.muted),
        caption: chrome::colour(sem.text.secondary),
        count: chrome::colour(sem.text.muted),
        valid: chrome::colour(mark.with_alpha_u8(VALID_ALPHA)),
        invalid: chrome::colour(viz::STATUS.critical),
        missing: chrome::colour(if dark {
            viz::NULL_INK_DARK
        } else {
            viz::NULL_INK_LIGHT
        }),
        bar: chrome::colour(mark),
        tints,
        mark,
    }
}

impl ColumnHeaderFrame {
    /// The band's height in logical points: the rows this density stacks, plus
    /// the inset above the first and below the last.
    #[must_use]
    pub fn extent(&self) -> f32 {
        let stacked = NAME_ROW
            + VALIDITY_ROW
            + self.plot_row()
            + self.range_row()
            + if self.density.is_full() {
                #[allow(clippy::cast_precision_loss)]
                let captions = CAPTION_ROWS as f32 * CAPTION_ROW;
                TYPES_ROW + captions
            } else {
                DISTINCT_ROW
            };
        2.0f32.mul_add(INSET_Y, stacked)
    }

    /// The row the picture of the distribution is drawn in: the rug's at the
    /// compact density, the bar chart's at the full one.
    #[must_use]
    pub const fn plot_row(&self) -> f32 {
        if self.density.is_full() {
            DISTRIBUTION_ROW
        } else {
            RUG_ROW
        }
    }

    /// The row the minimum and the maximum are drawn in.
    #[must_use]
    pub const fn range_row(&self) -> f32 {
        if self.density.is_full() {
            RANGE_ROW_FULL
        } else {
            RANGE_ROW_COMPACT
        }
    }

    /// The narrowest a column of this band may be drawn.
    #[must_use]
    pub const fn floor(&self) -> f32 {
        if self.density.is_full() {
            FLOOR_FULL
        } else {
            FLOOR_COMPACT
        }
    }

    /// The tint for storage type number `index`, wrapping at the palette's
    /// length so a table of more storage types than the palette has slots
    /// still draws.
    #[must_use]
    pub fn tint(&self, index: usize) -> egui::Color32 {
        self.tints[index % self.tints.len()]
    }

    /// The ink one column of the rug takes for `share` of the busiest
    /// bucket's count.
    ///
    /// Square-rooted so a long tail is visible beside a mode, and floored at
    /// this module's private `RUG_ALPHA_FLOOR` so a bucket holding a single row
    /// is not invisible.
    #[must_use]
    pub fn rug_ink(&self, share: f32) -> egui::Color32 {
        chrome::colour(Rgba::new(
            self.mark.r,
            self.mark.g,
            self.mark.b,
            self.rug_alpha(share),
        ))
    }

    /// The alpha [`ColumnHeaderFrame::rug_ink`] resolves, recorded beside the
    /// ink so a test reads the number rather than a quantised colour.
    #[must_use]
    pub fn rug_alpha(&self, share: f32) -> f32 {
        share.max(0.0).sqrt().clamp(RUG_ALPHA_FLOOR, 1.0)
    }
}

// ---------------------------------------------------------------------------
// The faces.
// ---------------------------------------------------------------------------

/// The glyph's face: the mono family two steps under the UI size, so a `#`
/// sits beside the name without competing with it.
fn glyph_font() -> egui::FontId {
    egui::FontId::monospace(typography::UI_SIZE - 2.0)
}

/// The column name's face: the UI sans at the UI size — the same face the
/// cells beneath it draw in.
fn name_font() -> egui::FontId {
    egui::FontId::proportional(typography::UI_SIZE)
}

/// The validity count's face: the smallest mono step the band uses, because it
/// sits inside a 12-point row beside a 3-point rule.
fn count_font() -> egui::FontId {
    egui::FontId::monospace(typography::UI_SIZE - 3.5)
}

/// The face every other line takes: the leaf, the storage type, the range and
/// the caption rows. Mono, because those lines are read against each other
/// down a column of cells and a proportional face lines nothing up.
fn detail_font() -> egui::FontId {
    egui::FontId::monospace(typography::UI_SIZE - 3.0)
}

// ---------------------------------------------------------------------------
// What a band knows about its columns.
// ---------------------------------------------------------------------------

/// The facts one table's band draws from: the profile of each column by name,
/// and the tint each storage type takes.
///
/// Built once per document rather than per frame, and looked up by column name
/// because the grid's columns come back from the engine's own rows query while
/// the facts come from the profile the file was opened with.
#[derive(Clone, Debug, Default)]
pub struct ColumnBandFacts {
    by_name: std::collections::BTreeMap<String, ColumnFacts>,
    tints: std::collections::BTreeMap<String, usize>,
}

impl ColumnBandFacts {
    /// The facts of `columns`, with a tint assigned to each distinct storage
    /// type in the order the columns first mention it — so the leftmost
    /// column's type is tint 0 whatever the table is.
    #[must_use]
    pub fn new(columns: &[ColumnFacts]) -> Self {
        let mut tints = std::collections::BTreeMap::new();
        let mut next = 0usize;
        for column in columns {
            if !tints.contains_key(&column.storage) {
                tints.insert(column.storage.clone(), next);
                next += 1;
            }
        }
        Self {
            by_name: columns
                .iter()
                .map(|c| (c.column.clone(), c.clone()))
                .collect(),
            tints,
        }
    }

    /// Whether any column was declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// The facts for the column the table spells `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ColumnFacts> {
        self.by_name.get(name)
    }

    /// Which tint that column's storage type takes.
    #[must_use]
    pub fn tint_index(&self, facts: &ColumnFacts) -> usize {
        self.tints.get(&facts.storage).copied().unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// The drawn record.
// ---------------------------------------------------------------------------

/// The statistics rows of one cell, at the full density: the number the engine
/// gave and the text drawn from it, so a test can hold the band to arithmetic
/// rather than to a format string.
#[derive(Clone, Debug, PartialEq)]
pub struct BandStats {
    pub mean: f64,
    pub mean_text: String,
    pub nulls: u64,
    pub nulls_text: String,
    pub median: f64,
    pub median_text: String,
    pub sd: Option<f64>,
    pub sd_text: String,
    pub distinct: u64,
    pub distinct_text: String,
}

/// What one column's band cell drew.
///
/// Recorded from the same expressions that paint, so a row that stopped being
/// drawn stops being here. The rects are window-space logical points.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnBandDrawn {
    /// The column's index in the table.
    pub column: usize,
    /// The column's name, as drawn.
    pub name: String,
    /// The finetype glyph before it — empty for a column the contract declares
    /// no glyph for.
    pub glyph: String,
    /// The density this cell drew at.
    pub density: GridDensity,
    /// The cell's own box.
    pub cell: egui::Rect,
    /// The clip it drew under.
    pub clip: egui::Rect,
    /// The band's height, summed from the rows this density stacks.
    pub extent: f32,
    /// Rows carrying a value that is neither invalid nor missing.
    pub valid: u64,
    /// Rows a per-column invalid count reports, which is zero on a file whose
    /// type source reports none.
    pub invalid: u64,
    /// Rows where the column is null.
    pub missing: u64,
    /// The validity band's own rect — the cell less the count's width.
    pub validity: egui::Rect,
    /// The three segments of it, valid then invalid then missing.
    pub segments: [egui::Rect; 3],
    /// The count at its trailing end.
    pub count_text: String,
    /// The rug's rect, at the compact density.
    pub rug: Option<egui::Rect>,
    /// One alpha per rug column, in the order they were painted.
    pub rug_alphas: Vec<f32>,
    /// The minimum and the maximum, as drawn.
    pub range: Option<(String, String)>,
    /// Where each of them was painted.
    pub range_rects: Option<(egui::Rect, egui::Rect)>,
    /// The finetype leaf, at the full density.
    pub leaf: Option<String>,
    /// The storage type beside it.
    pub storage: Option<String>,
    /// One rect per bar of the distribution, at the full density: the distinct
    /// count of them where that is at most
    /// [`VALUE_BAR_LIMIT`](brightfield_engine::profile::VALUE_BAR_LIMIT), and
    /// [`DISPLAY_BINS`](brightfield_engine::profile::DISPLAY_BINS) otherwise. A
    /// bin holding no row is a zero-height rect and is still here, so the count
    /// is the shape of the distribution rather than a count of what happened
    /// to be non-empty.
    pub bars: Vec<egui::Rect>,
    /// The statistics rows, at the full density.
    pub stats: Option<BandStats>,
    /// The distinct count, at the compact density's own row. `None` for a
    /// column with no moments, and `None` at the full density too, whose
    /// [`Self::stats`] carries the same number on its own `distinct` field
    /// instead.
    pub distinct: Option<u64>,
    /// The text [`Self::distinct`] was painted as.
    pub distinct_text: Option<String>,
    /// Where [`Self::distinct_text`] was painted — the compact density's own
    /// row, below the range and never over the rug, which
    /// `the_compact_bands_distinct_row_draws_below_the_range_and_never_over_the_rug`
    /// holds. `None` under the same conditions as [`Self::distinct`].
    pub distinct_rect: Option<egui::Rect>,
}

// ---------------------------------------------------------------------------
// Formatting.
// ---------------------------------------------------------------------------

/// A count with thousands separators.
#[must_use]
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A statistic as the band prints it.
///
/// Two decimal places below a thousand, a separated integer at or above one,
/// and an exponent where the magnitude would otherwise print as a wall of
/// digits or as `0.00`. A non-finite value prints as an em dash rather than as
/// `NaN`, which reads as a bug in the band rather than as an absent number.
#[must_use]
pub fn format_statistic(v: f64) -> String {
    if !v.is_finite() {
        return "\u{2014}".to_string();
    }
    let magnitude = v.abs();
    if magnitude >= 1e9 || (magnitude > 0.0 && magnitude < 1e-3) {
        return format!("{v:.3e}");
    }
    if magnitude >= 1000.0 {
        let rounded = v.round();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let whole = rounded.abs() as u64;
        let sign = if rounded < 0.0 { "-" } else { "" };
        return format!("{sign}{}", thousands(whole));
    }
    format!("{v:.2}")
}

/// The glyph a column carries: a degree sign for one half of a coordinate
/// pair, a hash for any other numeric column, and nothing at all for a column
/// the contract declares no glyph for — a VARCHAR, say, where drawing a hash
/// would say the column is a number.
#[must_use]
pub fn glyph_for(facts: &ColumnFacts) -> &'static str {
    if facts.paired.is_some() {
        GLYPH_COORDINATE
    } else if facts.moments.is_some() {
        GLYPH_NUMBER
    } else {
        ""
    }
}

/// The three segments of the validity band, left to right: valid, invalid,
/// missing.
///
/// Widths are shares of the row count, so the three abut and together span
/// `band`. A column with no rows gives the whole width to the valid segment,
/// which is the honest shape for an empty table: nothing is missing from it.
#[must_use]
pub fn validity_segments(
    band: egui::Rect,
    valid: u64,
    invalid: u64,
    missing: u64,
) -> [egui::Rect; 3] {
    let total = valid.saturating_add(invalid).saturating_add(missing);
    #[allow(clippy::cast_precision_loss)]
    let share = |n: u64| -> f32 {
        if total == 0 {
            0.0
        } else {
            n as f32 / total as f32
        }
    };
    let a = if total == 0 { 1.0 } else { share(valid) };
    let b = a + share(invalid);
    let at = |t: f32| band.left() + band.width() * t.clamp(0.0, 1.0);
    [
        egui::Rect::from_min_max(band.min, egui::pos2(at(a), band.bottom())),
        egui::Rect::from_min_max(
            egui::pos2(at(a), band.top()),
            egui::pos2(at(b), band.bottom()),
        ),
        egui::Rect::from_min_max(egui::pos2(at(b), band.top()), band.max),
    ]
}

// ---------------------------------------------------------------------------
// The paint.
// ---------------------------------------------------------------------------

/// How wide one cell's band content wants to be, before the density's floor
/// and the cells beneath it are considered.
///
/// The header's own claim on the column's width: the glyph and the name on one
/// row, and at the full density the leaf and the storage type on another.
#[must_use]
pub fn band_content_width(
    painter: &egui::Painter,
    facts: &ColumnFacts,
    frame: &ColumnHeaderFrame,
) -> f32 {
    let width_of = |text: &str, font: egui::FontId| -> f32 {
        if text.is_empty() {
            0.0
        } else {
            painter
                .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
                .size()
                .x
        }
    };
    let glyph = glyph_for(facts);
    let mut name = width_of(glyph, glyph_font()) + width_of(&facts.column, name_font());
    if !glyph.is_empty() {
        name += spacing::SPACE_3;
    }
    let types = if frame.density.is_full() {
        width_of(&facts.leaf, detail_font())
            + spacing::SPACE_4
            + width_of(&facts.storage, detail_font())
    } else {
        0.0
    };
    2.0f32.mul_add(INSET_X, name.max(types))
}

/// Paint one column's band cell into `cell`, and report what it drew.
///
/// `painter` is the header cell's own, already clipped by the widget to what
/// survives the scroll; `cell` is the whole box the column occupies whether or
/// not all of it is visible, so the record reads as *where this column is*
/// with the clip beside it saying how much of it reaches the reader.
#[allow(clippy::too_many_lines)]
pub fn draw_column_band(
    painter: &egui::Painter,
    cell: egui::Rect,
    column: usize,
    facts: &ColumnFacts,
    tint: usize,
    frame: &ColumnHeaderFrame,
) -> ColumnBandDrawn {
    painter.rect_filled(cell, 0.0, frame.fill);
    painter.rect_filled(cell, 0.0, frame.tint(tint));
    painter.line_segment(
        [cell.right_top(), cell.right_bottom()],
        egui::Stroke::new(1.0, frame.separator),
    );
    painter.line_segment(
        [cell.left_bottom(), cell.right_bottom()],
        egui::Stroke::new(1.0, frame.rule),
    );

    let inner = cell.shrink2(egui::vec2(INSET_X, INSET_Y));
    let mut y = inner.top();

    // 1. The glyph and the name.
    let glyph = glyph_for(facts);
    let mut text_left = inner.left();
    if !glyph.is_empty() {
        let at = painter.text(
            egui::pos2(inner.left(), y + NAME_ROW / 2.0),
            egui::Align2::LEFT_CENTER,
            glyph,
            glyph_font(),
            frame.glyph,
        );
        text_left = at.right() + spacing::SPACE_3;
    }
    painter.text(
        egui::pos2(text_left, y + NAME_ROW / 2.0),
        egui::Align2::LEFT_CENTER,
        &facts.column,
        name_font(),
        frame.name,
    );
    y += NAME_ROW;

    // 2. The validity band, with its count at the trailing end.
    //
    // The invalid segment is drawn at whatever width the invalid count gives
    // it. No type source in this build reports a per-column invalid count, so
    // that is zero today and the segment has no width; the count then names
    // what IS known, which is how many rows are missing.
    let invalid = 0u64;
    let missing = facts.nulls;
    let valid = facts.rows.saturating_sub(invalid).saturating_sub(missing);
    let count_text = format!("{} missing", thousands(missing));
    let count_width = painter
        .layout_no_wrap(count_text.clone(), count_font(), egui::Color32::PLACEHOLDER)
        .size()
        .x;
    let validity = egui::Rect::from_min_size(
        egui::pos2(inner.left(), (VALIDITY_ROW - VALIDITY_BAND) / 2.0 + y),
        egui::vec2(
            (inner.width() - count_width - spacing::SPACE_3).max(0.0),
            VALIDITY_BAND,
        ),
    );
    let segments = validity_segments(validity, valid, invalid, missing);
    for (rect, ink) in segments
        .iter()
        .zip([frame.valid, frame.invalid, frame.missing])
    {
        painter.rect_filled(*rect, 0.0, ink);
    }
    painter.text(
        egui::pos2(inner.right(), y + VALIDITY_ROW / 2.0),
        egui::Align2::RIGHT_CENTER,
        &count_text,
        count_font(),
        frame.count,
    );
    y += VALIDITY_ROW;

    // 3. The finetype leaf and the storage type — the full density's alone.
    let (leaf, storage) = if frame.density.is_full() {
        painter.text(
            egui::pos2(inner.left(), y + TYPES_ROW / 2.0),
            egui::Align2::LEFT_CENTER,
            &facts.leaf,
            detail_font(),
            frame.leaf,
        );
        painter.text(
            egui::pos2(inner.right(), y + TYPES_ROW / 2.0),
            egui::Align2::RIGHT_CENTER,
            &facts.storage,
            detail_font(),
            frame.storage,
        );
        y += TYPES_ROW;
        (Some(facts.leaf.clone()), Some(facts.storage.clone()))
    } else {
        (None, None)
    };

    // 4. The picture of the distribution: a bar chart at the full density, a
    //    rug at the compact one. Both are empty for a column with no moments.
    let plot = egui::Rect::from_min_size(
        egui::pos2(inner.left(), y),
        egui::vec2(inner.width(), frame.plot_row()),
    );
    let mut bars = Vec::new();
    let mut rug = None;
    let mut rug_alphas = Vec::new();
    if frame.density.is_full() {
        if let Some(moments) = facts.moments.as_ref() {
            bars = draw_bars(painter, plot, moments, frame);
        }
        painter.line_segment(
            [plot.left_bottom(), plot.right_bottom()],
            egui::Stroke::new(1.0, frame.rule),
        );
    } else if let Some(moments) = facts.moments.as_ref() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(plot.left(), plot.top() + (RUG_ROW - RUG_HEIGHT) / 2.0),
            egui::vec2(plot.width(), RUG_HEIGHT),
        );
        rug_alphas = draw_rug(painter, rect, moments, frame);
        rug = Some(rect);
    }
    y += frame.plot_row();

    // 5. The range.
    let range = facts.min.as_ref().zip(facts.max.as_ref());
    let (range_texts, range_rects) = match range {
        Some((min, max)) => {
            let lo = painter.text(
                egui::pos2(inner.left(), y + frame.range_row() / 2.0),
                egui::Align2::LEFT_CENTER,
                min,
                detail_font(),
                frame.range,
            );
            let hi = painter.text(
                egui::pos2(inner.right(), y + frame.range_row() / 2.0),
                egui::Align2::RIGHT_CENTER,
                max,
                detail_font(),
                frame.range,
            );
            (Some((min.clone(), max.clone())), Some((lo, hi)))
        }
        None => (None, None),
    };
    y += frame.range_row();

    // 6. The statistics — the full density's alone.
    let stats = if frame.density.is_full() {
        facts.moments.as_ref().map(|moments| {
            let stats = BandStats {
                mean: moments.mean,
                mean_text: format!("mean {}", format_statistic(moments.mean)),
                nulls: facts.nulls,
                nulls_text: format!("nulls {}", thousands(facts.nulls)),
                median: moments.median,
                median_text: format!("median {}", format_statistic(moments.median)),
                sd: moments.sd,
                sd_text: format!(
                    "sd {}",
                    moments
                        .sd
                        .map_or_else(|| "\u{2014}".to_string(), format_statistic)
                ),
                distinct: moments.distinct,
                distinct_text: format!("{} distinct", thousands(moments.distinct)),
            };
            for (left, right) in [
                (stats.mean_text.as_str(), stats.nulls_text.as_str()),
                (stats.median_text.as_str(), stats.sd_text.as_str()),
                (stats.distinct_text.as_str(), ""),
            ] {
                painter.text(
                    egui::pos2(inner.left(), y + CAPTION_ROW / 2.0),
                    egui::Align2::LEFT_CENTER,
                    left,
                    detail_font(),
                    frame.caption,
                );
                if !right.is_empty() {
                    painter.text(
                        egui::pos2(inner.right(), y + CAPTION_ROW / 2.0),
                        egui::Align2::RIGHT_CENTER,
                        right,
                        detail_font(),
                        frame.caption,
                    );
                }
                y += CAPTION_ROW;
            }
            stats
        })
    } else {
        None
    };

    // 7. The distinct count — the compact density's own row, below the
    //    range. The full density does not draw this a second time: its own
    //    distinct count is already inside `stats`, painted above.
    let mut distinct = None;
    let mut distinct_text = None;
    let mut distinct_rect = None;
    if !frame.density.is_full() {
        if let Some(moments) = facts.moments.as_ref() {
            let text = format!("{} distinct", thousands(moments.distinct));
            let rect = painter.text(
                egui::pos2(inner.left(), y + DISTINCT_ROW / 2.0),
                egui::Align2::LEFT_CENTER,
                &text,
                detail_font(),
                frame.caption,
            );
            distinct = Some(moments.distinct);
            distinct_text = Some(text);
            distinct_rect = Some(rect);
        }
    }

    ColumnBandDrawn {
        column,
        name: facts.column.clone(),
        glyph: glyph.to_string(),
        density: frame.density,
        cell,
        clip: painter.clip_rect(),
        extent: frame.extent(),
        valid,
        invalid,
        missing,
        validity,
        segments,
        count_text,
        rug,
        rug_alphas,
        range: range_texts,
        range_rects,
        leaf,
        storage,
        bars,
        stats,
        distinct,
        distinct_text,
        distinct_rect,
    }
}

/// The bar distribution: one bar per distinct value where there are few
/// enough, and the binned branch otherwise. Returns one rect per bar, in the
/// order they were painted, an empty bin included as a bar of no height.
fn draw_bars(
    painter: &egui::Painter,
    plot: egui::Rect,
    moments: &ColumnMoments,
    frame: &ColumnHeaderFrame,
) -> Vec<egui::Rect> {
    let bars = moments.bars();
    #[allow(clippy::cast_precision_loss)]
    let peak = bars.peak() as f32;
    let mut out = Vec::with_capacity(bars.len());
    match &bars {
        Bars::PerValue(values) => {
            #[allow(clippy::cast_precision_loss)]
            let width = (plot.width() / values.len().max(1) as f32).max(1.0);
            for (t, n) in values {
                #[allow(clippy::cast_precision_loss)]
                let height = plot.height() * (*n as f32 / peak);
                #[allow(clippy::cast_possible_truncation)]
                let left = (*t as f32).mul_add(plot.width() - width, plot.left());
                let rect = egui::Rect::from_min_max(
                    egui::pos2(left, plot.bottom() - height),
                    egui::pos2(left + (width - 0.5).max(0.5), plot.bottom()),
                );
                painter.rect_filled(rect, 0.0, frame.bar);
                out.push(rect);
            }
        }
        Bars::Binned(counts) => {
            #[allow(clippy::cast_precision_loss)]
            let width = plot.width() / counts.len().max(1) as f32;
            for (b, n) in counts.iter().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let height = plot.height() * (*n as f32 / peak);
                #[allow(clippy::cast_precision_loss)]
                let left = (b as f32).mul_add(width, plot.left());
                let rect = egui::Rect::from_min_max(
                    egui::pos2(left + 0.5, plot.bottom() - height),
                    egui::pos2((left + width - 0.5).max(left + 0.5), plot.bottom()),
                );
                painter.rect_filled(rect, 0.0, frame.bar);
                out.push(rect);
            }
        }
    }
    out
}

/// The rug: one pixel column per point of the cell's width, each inked at the
/// alpha its share of the busiest bucket earns. Returns one alpha per column
/// painted, in order — the empty ones skipped, and recorded as zero so the
/// record's length is the rug's width rather than its density.
fn draw_rug(
    painter: &egui::Painter,
    rug: egui::Rect,
    moments: &ColumnMoments,
    frame: &ColumnHeaderFrame,
) -> Vec<f32> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let columns = (rug.width().floor().max(1.0)) as usize;
    let counts = moments.rug(columns);
    #[allow(clippy::cast_precision_loss)]
    let peak = counts.iter().copied().max().unwrap_or(1).max(1) as f32;
    let mut alphas = Vec::with_capacity(counts.len());
    for (k, n) in counts.iter().enumerate() {
        if *n == 0 {
            alphas.push(0.0);
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let share = *n as f32 / peak;
        let alpha = frame.rug_alpha(share);
        #[allow(clippy::cast_precision_loss)]
        let left = k as f32 + rug.left();
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(left, rug.top()), egui::vec2(1.0, rug.height())),
            0.0,
            frame.rug_ink(share),
        );
        alphas.push(alpha);
    }
    alphas
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The two extents are the rows the density stacks** — summed here and
    /// stated in the ratified frames as 70 and 127.
    ///
    /// The frames hard-code those two numbers; this file adds its rows up. The
    /// assertion is what keeps the two answers the same one, and it is why a
    /// row changing height is a change this reports rather than one it
    /// absorbs. The compact total is 70, not the original contract's 57: this
    /// card gave the compact density its own distinct-count row, and 57 + 13
    /// (`DISTINCT_ROW`) is 70.
    #[test]
    fn the_band_extents_are_the_sums_of_the_rows_each_density_stacks() {
        let compact = column_header_frame(GridDensity::Compact, Mode::Light);
        let full = column_header_frame(GridDensity::Full, Mode::Light);
        assert!(
            (compact.extent() - 70.0).abs() < f32::EPSILON,
            "the compact band is {} points, and the ratified frame is 70",
            compact.extent()
        );
        assert!(
            (full.extent() - 127.0).abs() < f32::EPSILON,
            "the full band is {} points, and the ratified frame is 127",
            full.extent()
        );
        // What the full density adds over the compact one: the leaf-and-storage
        // row, the bar chart over the rug, two more points of range row, and
        // three caption rows — less the one row the compact density has that
        // the full density does not, its own solo distinct-count row.
        #[allow(clippy::cast_precision_loss)]
        let added = TYPES_ROW
            + (DISTRIBUTION_ROW - RUG_ROW)
            + (RANGE_ROW_FULL - RANGE_ROW_COMPACT)
            + CAPTION_ROWS as f32 * CAPTION_ROW
            - DISTINCT_ROW;
        assert!(
            (full.extent() - compact.extent() - added).abs() < f32::EPSILON,
            "full {} less compact {} is not the {added} points the contract adds",
            full.extent(),
            compact.extent()
        );
    }

    /// **The invalid segment is declared and drawn, at whatever width its
    /// count gives it.**
    ///
    /// Nothing in this build reports a per-column invalid count, so the band a
    /// reader sees has a zero-width invalid segment — and a test that only ever
    /// passed zero would leave the segment's arithmetic unexercised, which is
    /// how a declared-but-dead branch ships. This drives the arithmetic
    /// directly at both.
    #[test]
    fn the_validity_bands_three_segments_abut_and_span_the_band() {
        let band = egui::Rect::from_min_size(egui::pos2(10.0, 4.0), egui::vec2(100.0, 3.0));

        let none = validity_segments(band, 240, 0, 0);
        assert!(
            (none[0].width() - 100.0).abs() < 1e-3,
            "240 valid rows and nothing else fill the band: {none:?}"
        );
        assert!(
            none[1].width().abs() < 1e-3 && none[2].width().abs() < 1e-3,
            "no invalid and no missing rows draw no width: {none:?}"
        );

        let mixed = validity_segments(band, 50, 25, 25);
        assert!((mixed[0].width() - 50.0).abs() < 1e-3, "{mixed:?}");
        assert!((mixed[1].width() - 25.0).abs() < 1e-3, "{mixed:?}");
        assert!((mixed[2].width() - 25.0).abs() < 1e-3, "{mixed:?}");
        assert!(
            (mixed[0].right() - mixed[1].left()).abs() < 1e-3
                && (mixed[1].right() - mixed[2].left()).abs() < 1e-3,
            "the segments do not abut: {mixed:?}"
        );
        assert!(
            (mixed[2].right() - band.right()).abs() < 1e-3,
            "the segments do not reach the band's end: {mixed:?}"
        );

        let empty = validity_segments(band, 0, 0, 0);
        assert!(
            (empty[0].width() - 100.0).abs() < 1e-3,
            "a table with no rows is not a table that is all missing: {empty:?}"
        );
    }

    /// The rug's alpha is the square root of a share, floored so a single row
    /// is visible and capped at opaque.
    #[test]
    fn a_rug_columns_alpha_is_the_square_root_of_its_share() {
        let frame = column_header_frame(GridDensity::Compact, Mode::Light);
        assert!((frame.rug_alpha(1.0) - 1.0).abs() < 1e-6);
        assert!((frame.rug_alpha(0.25) - 0.5).abs() < 1e-6);
        assert!(
            (frame.rug_alpha(0.001) - RUG_ALPHA_FLOOR).abs() < 1e-6,
            "a share under the floor is drawn at the floor, not invisibly"
        );
    }

    /// The statistic format: two decimals below a thousand, separated digits
    /// above it, an exponent at the extremes, an em dash for a non-number.
    #[test]
    fn a_statistic_is_formatted_by_its_magnitude() {
        assert_eq!(format_statistic(3.870_671), "3.87");
        assert_eq!(format_statistic(1_425.476_744), "1,425");
        assert_eq!(format_statistic(-12.5), "-12.50");
        assert_eq!(format_statistic(0.0), "0.00");
        assert_eq!(format_statistic(f64::NAN), "\u{2014}");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    /// Two storage types on one table take two tints, and a column of the same
    /// type as the first takes the first's.
    #[test]
    fn a_storage_type_takes_one_tint_across_the_table() {
        let facts = |name: &str, storage: &str| ColumnFacts {
            column: name.to_string(),
            label: None,
            leaf: storage.to_string(),
            storage: storage.to_string(),
            tile: None,
            because: String::new(),
            paired: None,
            rows: 0,
            nulls: 0,
            min: None,
            max: None,
            moments: None,
        };
        let columns = vec![
            facts("a", "DOUBLE"),
            facts("b", "BIGINT"),
            facts("c", "DOUBLE"),
        ];
        let band = ColumnBandFacts::new(&columns);
        let index = |n: &str| band.tint_index(band.get(n).expect("declared"));
        assert_eq!(index("a"), 0, "the leftmost column's type is tint 0");
        assert_eq!(index("b"), 1, "a second storage type is a second tint");
        assert_eq!(index("c"), 0, "the same type is the same tint");
        let frame = column_header_frame(GridDensity::Compact, Mode::Light);
        assert_ne!(
            frame.tint(0),
            frame.tint(1),
            "two storage types that read as one tint say the table is uniform \
             when it is not"
        );
    }
}
