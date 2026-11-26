//! SQL for the dense-region overlay on a two-column plot: a count of rows on a
//! square grid laid over the plot's current view, and the expression that tells
//! which cell of that grid a row falls in.
//!
//! The grid's edges are the view extent the caller passes, not the table's
//! min and max, so a zoomed plot gets a grid over what it shows. The binning is
//! the density lowerer's own arithmetic ([`crate::lower::equiwidth_bin_index`]),
//! so a region grid cell and a density cell cover the same data range.

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
