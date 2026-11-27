//! SQL for the dense-region overlay on a two-column plot: a count of rows on a
//! square grid laid over the plot's current view, the expression that tells
//! which cell of that grid a row falls in, and the summary of each region's
//! rows its name is read from.
//!
//! The grid's edges are the view extent the caller passes, not the table's
//! min and max, so a zoomed plot gets a grid over what it shows. The binning is
//! the density lowerer's own arithmetic (`lower::equiwidth_bin_index`),
//! so a region grid cell and a density cell cover the same data range.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use crate::lower::{equiwidth_bin_index, quoted_name};

/// A square grid over a view: the x and y ranges as `(lo, hi)` and the number
/// of equal-width bins along each axis.
#[derive(Clone, Debug)]
pub struct GridExtent {
    pub x: (f64, f64),
    pub y: (f64, f64),
    pub bins: u32,
}

impl GridExtent {
    fn bits(&self) -> [u64; 4] {
        [
            self.x.0.to_bits(),
            self.x.1.to_bits(),
            self.y.0.to_bits(),
            self.y.1.to_bits(),
        ]
    }

    /// Both ranges are finite and have positive width.
    pub(crate) fn is_usable(&self) -> bool {
        let ok = |(lo, hi): (f64, f64)| lo.is_finite() && hi.is_finite() && lo < hi;
        self.bins > 0 && ok(self.x) && ok(self.y)
    }
}

// Bit-pattern equality, as `ScalarValue` does, so `Eq` and `Hash` agree and a
// `Predicate` holding an extent can derive them.
impl PartialEq for GridExtent {
    fn eq(&self, other: &Self) -> bool {
        self.bins == other.bins && self.bits() == other.bits()
    }
}

impl Eq for GridExtent {}

impl Hash for GridExtent {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bits().hash(state);
        self.bins.hash(state);
    }
}

/// A float as a SQL literal that reads back to the same value.
fn literal(v: f64) -> String {
    format!("{v:?}")
}

/// `x` and `y` within the extent, both ends inclusive. A null fails both tests.
pub(crate) fn in_extent_sql(x_col: &str, y_col: &str, extent: &GridExtent) -> String {
    let (x, y) = (quoted_name(x_col), quoted_name(y_col));
    format!(
        "{x} BETWEEN {} AND {} AND {y} BETWEEN {} AND {}",
        literal(extent.x.0),
        literal(extent.x.1),
        literal(extent.y.0),
        literal(extent.y.1),
    )
}

/// The 0-based bin of `col` along one axis of the extent, as an INTEGER.
fn bin_sql(col: &str, (lo, hi): (f64, f64), bins: u32) -> String {
    let index = equiwidth_bin_index(
        &quoted_name(col),
        &literal(lo),
        &literal(hi),
        i64::from(bins),
    );
    format!("CAST({index} AS INTEGER)")
}

/// The index of the grid cell a row's point falls in: `by * bins + bx`, with
/// `by` from `y_col` and `bx` from `x_col`.
///
/// This is row-major with the row taken from y, the layout of
/// `brightfield_render::regions::DensityGrid` (cell = row * cols + col), so a
/// cell index found on that grid selects the same rows here. The two crates do
/// not depend on each other; the agreement is this convention.
///
/// The expression is meaningful only for a row with both coordinates inside the
/// extent: `least` skips a null, so a null coordinate would land in the last
/// bin. [`region_grid_sql`] and [`Predicate::Cells`](crate::ir::Predicate::Cells)
/// bound the rows by the extent first.
#[must_use]
pub fn cell_index_sql(x_col: &str, y_col: &str, extent: &GridExtent) -> String {
    let bx = bin_sql(x_col, extent.x, extent.bins);
    let by = bin_sql(y_col, extent.y, extent.bins);
    format!("({by} * {} + {bx})", extent.bins)
}

/// A query counting the rows of `rows_sql` in each occupied cell of the grid:
/// columns `bx INTEGER, by INTEGER, n DOUBLE`, ordered by `by, bx`. (`by` is a
/// keyword in DuckDB, so the query quotes it.)
///
/// Rows outside the extent and rows with a null in either column are left out;
/// a row on the top edge falls in the last bin. `None` when an axis has no
/// positive finite width, since no grid exists over it.
#[must_use]
pub fn region_grid_sql(
    rows_sql: &str,
    x_col: &str,
    y_col: &str,
    extent: &GridExtent,
) -> Option<String> {
    if !extent.is_usable() {
        return None;
    }
    let bx = bin_sql(x_col, extent.x, extent.bins);
    let by = bin_sql(y_col, extent.y, extent.bins);
    let in_view = in_extent_sql(x_col, y_col, extent);
    Some(format!(
        "SELECT bx, \"by\", CAST(count(*) AS DOUBLE) AS n \
         FROM (SELECT {bx} AS bx, {by} AS \"by\" FROM ({rows_sql}) AS __bf_rows WHERE {in_view}) \
         GROUP BY bx, \"by\" ORDER BY \"by\", bx"
    ))
}

/// What the label query asks of a column, and so how a region may be named
/// after it: by a word its rows share, by a value most of them hold, or by the
/// range most of them fall in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelRole {
    Text,
    Category,
    Number,
}

/// A column the label query summarises per region, with the role it is asked
/// in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelColumn {
    pub name: String,
    pub role: LabelRole,
}

/// The `kind` of each row [`region_label_sql`] returns.
pub mod label_kind {
    /// How many rows of the view are in the region. `col` and `key` are null.
    pub const ROWS: &str = "rows";
    /// A text column's word or two-word phrase, in the capitalisation the
    /// view's rows use most, and how many of the region's rows hold it.
    pub const TERM: &str = "term";
    /// A category column's value and how many of the region's rows hold it.
    pub const VALUE: &str = "value";
    /// A number column's lower quartile in the region.
    pub const Q1: &str = "q1";
    /// A number column's upper quartile in the region.
    pub const Q3: &str = "q3";
    /// How many of the region's rows hold a number in the column.
    pub const PRESENT: &str = "present";
    /// A number column's smallest value in the view, on a row with region `-1`
    /// whatever region holds it: the low edge of its histogram.
    pub const LO: &str = "lo";
    /// A number column's largest value in the view, on a row with region `-1`:
    /// the high edge of its histogram.
    pub const HI: &str = "hi";
    /// How many of the region's rows have a number in one bin of the column's
    /// histogram; `key` is the bin's index, `0..NUMBER_BINS`, as text.
    ///
    /// [`NUMBER_BINS`]: super::NUMBER_BINS
    pub const BIN: &str = "bin";
    /// The least number the region's rows hold in one bin; `key` as for
    /// [`BIN`].
    pub const BIN_MIN: &str = "bin_min";
    /// The greatest number the region's rows hold in one bin; `key` as for
    /// [`BIN`].
    pub const BIN_MAX: &str = "bin_max";
}

/// How many equal-width bins span a number column's range in the view, for the
/// histogram the rest of the view's quartiles are read from. Each bin carries
/// its count and the least and greatest value in it, so a quartile read off
/// the histogram always lies between two values of its bin that the rows
/// hold, and is exact when that bin holds at most two values. A bin's width
/// only decides which values share it: one outlier widens every bin, but the
/// interpolation stays inside the values each bin holds. The summary stays at
/// most this many bins per region per number column, however many rows the
/// view holds.
pub const NUMBER_BINS: u32 = 1024;

/// Words a text column's terms leave out, besides any word of fewer than three
/// letters: English words that join a description rather than say what it is
/// about, and the distance and compass words of a place written relative to a
/// town, so `45 km SW of Severo-Kuril'sk, Russia` is about Severo-Kuril'sk and
/// Russia. Compass points of one and two letters, `km` and `of` fall to the
/// length rule. Whole compass words (`south`) are kept, since they are often
/// part of a name (`South Sandwich Islands`).
pub const STOP_WORDS: &[&str] = &[
    // English.
    "the",
    "and",
    "for",
    "from",
    "with",
    "near",
    "off",
    "into",
    "onto",
    "over",
    "under",
    "about",
    "this",
    "that",
    "are",
    "was",
    "were",
    "has",
    "have",
    "had",
    "not",
    "but",
    "its",
    "per",
    "via",
    "than",
    "then",
    "there",
    "their",
    "they",
    "them",
    "who",
    "what",
    "when",
    "where",
    "which",
    "will",
    "would",
    "also",
    "any",
    "all",
    "can",
    // Distance.
    "kms",
    "mile",
    "miles",
    "kilometre",
    "kilometres",
    "kilometer",
    "kilometers",
    "metre",
    "metres",
    "meter",
    "meters",
    "feet",
    // Three-letter compass points.
    "nne",
    "ene",
    "ese",
    "sse",
    "ssw",
    "wsw",
    "wnw",
    "nnw",
];

/// The query that summarises each of `columns` per region of the view, for
/// naming the regions: one statement that reads the rows of `rows_sql` once.
///
/// A row is in the view when both coordinates are inside `extent` (the
/// [`region_grid_sql`] guard, which [`cell_index_sql`] needs), and in the
/// region its grid cell is paired with in `cell_regions`, `(cell index,
/// region index)`; a row in view whose cell is in no region is in region `-1`,
/// the rest of the view. A cell listed twice keeps its first region.
///
/// The result is one long table, ordered by `col`, `region`, `kind` and `key`:
///
/// - `col INTEGER`: the column's index in `columns`, null on a
///   [`label_kind::ROWS`] row;
/// - `region INTEGER`: the region, or `-1`;
/// - `kind VARCHAR`: one of [`label_kind`];
/// - `key VARCHAR`: the term, the category value or the bin, null for the
///   others;
/// - `value DOUBLE`: the count, the quartile or the edge.
///
/// A text column is cut into runs at anything that is not a letter, an
/// apostrophe, a hyphen or a space, so punctuation and digits end a run, and
/// each run is split into words at spaces. A word keeps an apostrophe or a
/// hyphen inside it (`Kuril'sk`, a transliteration's soft sign;
/// `Severo-Kuril'sk`, one name) and loses either at its ends, and a typographic
/// apostrophe reads as a plain one. Words of fewer than three letters and the
/// [`STOP_WORDS`] are dropped and leave a gap. Each kept word is a term, and so
/// is each pair of kept words next to each other in a run with no gap between,
/// so a phrase is always one the row wrote (`Fiji Islands` from `south of the
/// Fiji Islands`, but no `Gulf California` from `Gulf of California` and no
/// `Kuril'sk Russia` across a comma). Terms are compared lower-cased and a row
/// counts once per term. Null and blank values hold no term and no category
/// value, and a category value is trimmed, so `Gentoo ` is `Gentoo`; empty
/// values count against a share without ever being named.
///
/// A number column reads NaN and infinities as missing, like a null: they have
/// no place on a range and no bin. Its quartiles in each region are DuckDB's
/// `quantile_cont` over its finite numbers, exact because they are what a name
/// prints. The
/// rest of the view's are left to the reader: the query counts each region's
/// numbers in [`NUMBER_BINS`] equal-width bins between the column's [`LO`] and
/// [`HI`] in the view, with each bin's count, least and greatest value, so the
/// rest of a region is the view's histogram less the
/// region's, and the summary grows with regions times bins rather than with
/// regions times rows. The rows are materialised once first, so the source is
/// read once.
///
/// [`LO`]: label_kind::LO
/// [`HI`]: label_kind::HI
///
/// `None` when `extent` has no usable grid or `cell_regions` is empty, since
/// then there is no region to name.
#[must_use]
pub fn region_label_sql(
    rows_sql: &str,
    x_col: &str,
    y_col: &str,
    extent: &GridExtent,
    cell_regions: &[(u32, u32)],
    columns: &[LabelColumn],
) -> Option<String> {
    if !extent.is_usable() || cell_regions.is_empty() {
        return None;
    }
    let mut cells = BTreeMap::new();
    for &(cell, region) in cell_regions {
        cells.entry(cell).or_insert(region);
    }
    let pairs: Vec<String> = cells.iter().map(|(c, r)| format!("({c}, {r})")).collect();
    let picked: String = columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let name = quoted_name(&c.name);
            match c.role {
                LabelRole::Number => format!(
                    ", CASE WHEN isfinite(TRY_CAST({name} AS DOUBLE)) \
                     THEN TRY_CAST({name} AS DOUBLE) END AS __bf_c{i}"
                ),
                LabelRole::Text => format!(", CAST({name} AS VARCHAR) AS __bf_c{i}"),
                LabelRole::Category => format!(", trim(CAST({name} AS VARCHAR)) AS __bf_c{i}"),
            }
        })
        .collect();
    let kept: String = (0..columns.len()).map(|i| format!(", __bf_c{i}")).collect();
    let cell = cell_index_sql(x_col, y_col, extent);
    let in_view = in_extent_sql(x_col, y_col, extent);

    let mut ctes = vec![
        format!(
            "__bf_cells(__bf_cell, __bf_region) AS (VALUES {})",
            pairs.join(", ")
        ),
        format!(
            "__bf_view AS MATERIALIZED (\
             SELECT row_number() OVER () AS __bf_row, \
             CAST(coalesce(__bf_cells.__bf_region, -1) AS INTEGER) AS __bf_region{kept} \
             FROM (SELECT {cell} AS __bf_cell{picked} FROM ({rows_sql}) AS __bf_rows \
             WHERE {in_view}) AS __bf_in \
             LEFT JOIN __bf_cells USING (__bf_cell))"
        ),
    ];
    let mut branches = vec![format!(
        "SELECT CAST(NULL AS INTEGER) AS col, __bf_region AS region, '{}' AS kind, \
         CAST(NULL AS VARCHAR) AS key, CAST(count(*) AS DOUBLE) AS value \
         FROM __bf_view GROUP BY __bf_region",
        label_kind::ROWS
    )];
    for (i, c) in columns.iter().enumerate() {
        let v = format!("__bf_c{i}");
        match c.role {
            LabelRole::Category => branches.push(format!(
                "SELECT {i}, __bf_region, '{}', {v}, CAST(count(*) AS DOUBLE) \
                 FROM __bf_view WHERE {v} <> '' GROUP BY __bf_region, {v}",
                label_kind::VALUE
            )),
            LabelRole::Text => {
                ctes.push(text_terms_cte(i));
                ctes.push(format!(
                    "__bf_forms{i} AS (SELECT term, form FROM (\
                     SELECT term, form, row_number() OVER \
                     (PARTITION BY term ORDER BY count(*) DESC, form) AS rank \
                     FROM __bf_terms{i} GROUP BY term, form) WHERE rank = 1)"
                ));
                branches.push(format!(
                    "SELECT {i}, t.__bf_region, '{}', f.form, \
                     CAST(count(DISTINCT t.__bf_row) AS DOUBLE) \
                     FROM __bf_terms{i} AS t JOIN __bf_forms{i} AS f USING (term) \
                     GROUP BY t.__bf_region, f.form",
                    label_kind::TERM
                ));
            }
            LabelRole::Number => {
                branches.push(format!(
                    "SELECT {i}, region, kind, CAST(NULL AS VARCHAR), value FROM (\
                     SELECT __bf_region AS region, \
                     quantile_cont({v}, 0.25) AS {q1}, quantile_cont({v}, 0.75) AS {q3}, \
                     CAST(count({v}) AS DOUBLE) AS {present} \
                     FROM __bf_view GROUP BY __bf_region) \
                     UNPIVOT (value FOR kind IN ({q1}, {q3}, {present}))",
                    q1 = label_kind::Q1,
                    q3 = label_kind::Q3,
                    present = label_kind::PRESENT,
                ));
                let bin = equiwidth_bin_index(&v, "b.lo", "b.hi", i64::from(NUMBER_BINS));
                branches.push(format!(
                    "SELECT {i}, -1, kind, CAST(NULL AS VARCHAR), value FROM (\
                     SELECT min({v}) AS {lo}, max({v}) AS {hi} FROM __bf_view) \
                     UNPIVOT (value FOR kind IN ({lo}, {hi}))",
                    lo = label_kind::LO,
                    hi = label_kind::HI,
                ));
                branches.push(format!(
                    "SELECT {i}, region, kind, __bf_bin, value FROM (\
                     SELECT v.__bf_region AS region, \
                     CAST(CAST({bin} AS INTEGER) AS VARCHAR) AS __bf_bin, \
                     CAST(count(*) AS DOUBLE) AS {count}, \
                     min(v.{v}) AS {least}, max(v.{v}) AS {greatest} \
                     FROM __bf_view AS v, \
                     (SELECT min({v}) AS lo, max({v}) AS hi FROM __bf_view) AS b \
                     WHERE v.{v} IS NOT NULL GROUP BY v.__bf_region, __bf_bin) \
                     UNPIVOT (value FOR kind IN ({count}, {least}, {greatest}))",
                    count = label_kind::BIN,
                    least = label_kind::BIN_MIN,
                    greatest = label_kind::BIN_MAX,
                ));
            }
        }
    }
    Some(format!(
        "WITH {} SELECT * FROM ({}) ORDER BY col NULLS FIRST, region, kind, key NULLS FIRST",
        ctes.join(", "),
        branches.join(" UNION ALL ")
    ))
}

/// The terms of text column `i`: one row per row of the view, term and the
/// spelling the row wrote it in. See [`region_label_sql`] for the rules.
fn text_terms_cte(i: usize) -> String {
    let stop: Vec<String> = STOP_WORDS.iter().map(|w| format!("'{w}'")).collect();
    let stop = stop.join(", ");
    // Each run's words, with a dropped word as '' and a '' after the run, so
    // two words pair only when both are kept and nothing came between them.
    let word = "trim(w, '''-')";
    let words = format!(
        "flatten(list_transform(\
         regexp_split_to_array(replace(__bf_c{i}, '\u{2019}', ''''), '[^\\p{{L}}''\\s-]+'), \
         lambda g: list_append(list_transform(regexp_split_to_array(g, '\\s+'), \
         lambda w: CASE WHEN length(regexp_replace({word}, '[''-]', '', 'g')) >= 3 \
         AND NOT list_contains([{stop}], lower({word})) THEN {word} ELSE '' END), '')))"
    );
    format!(
        "__bf_terms{i} AS MATERIALIZED (\
         SELECT DISTINCT __bf_row, __bf_region, lower(form) AS term, form FROM (\
         SELECT __bf_row, __bf_region, unnest(list_concat(\
         list_filter(ws, lambda w: w <> ''), \
         list_filter(list_transform(range(1, len(ws)), lambda j: \
         CASE WHEN ws[j] <> '' AND ws[j + 1] <> '' THEN ws[j] || ' ' || ws[j + 1] END), \
         lambda p: p IS NOT NULL))) AS form \
         FROM (SELECT __bf_row, __bf_region, {words} AS ws FROM __bf_view \
         WHERE __bf_c{i} IS NOT NULL)))"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Predicate;

    fn extent() -> GridExtent {
        GridExtent {
            x: (0.0, 10.0),
            y: (-5.0, 5.0),
            bins: 100,
        }
    }

    #[test]
    fn a_degenerate_extent_yields_no_grid_sql() {
        for (x, y) in [
            ((1.0, 1.0), (0.0, 1.0)),
            ((0.0, 1.0), (2.0, 1.0)),
            ((0.0, f64::NAN), (0.0, 1.0)),
            ((0.0, 1.0), (f64::NEG_INFINITY, 1.0)),
        ] {
            let e = GridExtent { x, y, bins: 100 };
            assert_eq!(region_grid_sql("SELECT 1", "x", "y", &e), None);
        }
        assert!(region_grid_sql("SELECT 1", "x", "y", &extent()).is_some());
    }

    #[test]
    fn quoted_column_names_round_trip() {
        let name = "a \"b\"";
        let quoted = "\"a \"\"b\"\"\"";
        let grid = region_grid_sql("SELECT 1", name, "y", &extent()).unwrap();
        assert!(grid.contains(quoted), "{grid}");
        assert!(cell_index_sql(name, "y", &extent()).contains(quoted));
    }

    #[test]
    fn a_degenerate_extent_yields_no_cells_predicate() {
        let nan = GridExtent {
            x: (f64::NAN, 1.0),
            ..extent()
        };
        let flat = GridExtent {
            y: (2.0, 2.0),
            ..extent()
        };
        let no_bins = GridExtent {
            bins: 0,
            ..extent()
        };
        for e in [nan, flat, no_bins] {
            assert_eq!(Predicate::cells("x", "y", e, vec![1]), None);
        }
    }

    #[test]
    fn cells_predicate_is_canonical() {
        let a = Predicate::cells("x", "y", extent(), vec![7, 3, 7, 1]).unwrap();
        let b = Predicate::cells("x", "y", extent(), vec![1, 3, 7]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.to_string(), b.to_string());
        assert!(a.to_string().contains("IN (1, 3, 7)"), "{a}");
    }
}
