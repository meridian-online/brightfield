//! **The nearest drawn row to a point on a plot** — the query, and the one row
//! it comes back as.
//!
//! Resting a pointer on a mark asks a question about *data*: which row is that
//! dot. The answer this module produces is a single row read out of DuckDB by a
//! query the caller does not assemble, under the same `WHERE` the mark it is
//! hovering was drawn with. The alternative — keeping the mark's `RecordBatch`
//! on the client and scanning it — is the architecture this seam exists to
//! reject, for the same reason the tabular surface's windowed read does: a
//! step larger than memory has no client-side copy to scan, and a copy that
//! exists can drift from what the chart drew.
//!
//! # Distance is measured in pixels, and the caller supplies the conversion
//!
//! "Nearest" on a screen means nearest *as drawn*, so a plot whose x axis spans
//! six orders of magnitude and whose y axis spans two does not hand back the
//! row that is nearest in x units. The engine owns no scales, so a
//! [`NearestProbe`] carries the conversion instead: per axis, the data value
//! under the pointer ([`NearestAxis::at`]) and how many data units one logical
//! pixel spans ([`NearestAxis::per_pixel`]). Dividing a column's distance from
//! `at` by `per_pixel` puts both axes in pixels, and
//! [`NearestProbe::radius`] is then a plain screen distance.
//!
//! Both axes have to be continuous for that division to mean something, which
//! is why [`NearestAxis::per_pixel`] is a number and not a scale: a band axis
//! has no pixels-per-unit and a caller holding one cannot build a probe.

use std::fmt::Write as _;

/// The alias the distance is computed under, inside the wrapped query.
///
/// Prefixed like `brightfield_sql`'s own reserved output columns so a source
/// column of the same name is a collision somebody chose rather than one they
/// could stumble into.
const DISTANCE_COLUMN: &str = "__bf_hover_distance";

/// The subquery alias the step's rows are read through.
const ROWS_ALIAS: &str = "bf_hover_rows";

/// The subquery alias the distance is computed in.
const DISTANCE_ALIAS: &str = "bf_hover";

/// One axis of a [`NearestProbe`]: the column, where the pointer is on it, and
/// the scale that turns the two into pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct NearestAxis {
    /// The column this axis encodes, as a bare name. It is written into the
    /// query as a quoted identifier, so a column named with a space or a
    /// reserved word needs nothing from the caller.
    pub column: String,
    /// The data value under the pointer on this axis — the pointer's pixel
    /// position inverted through the scale the plot was drawn with.
    pub at: f64,
    /// How many data units one logical pixel spans on this axis. Sign is
    /// irrelevant (the distance squares it), magnitude is not: this is what
    /// makes the two axes comparable.
    pub per_pixel: f64,
}

/// **What to read, where from, and how close counts** — everything the nearest
/// read needs that is not already in the session.
#[derive(Clone, Debug, PartialEq)]
pub struct NearestProbe {
    /// The horizontal axis.
    pub x: NearestAxis,
    /// The vertical axis.
    pub y: NearestAxis,
    /// The columns to read back, in the order they should be reported. The
    /// query projects **these and nothing else**, so the caller cannot come
    /// into possession of a column it did not ask for — which is the whole
    /// difference between this and handing back a row of a `SELECT *`.
    pub read: Vec<String>,
    /// How far from the pointer a mark may be and still be the answer, in
    /// logical pixels.
    pub radius: f64,
}

/// One cell of a read row: the column, and what DuckDB spells its value.
///
/// The value is a `String` because it is on its way to a readout, and because
/// a typed cell would put the engine's Arrow types on the shell's plate for no
/// gain. `CAST(… AS VARCHAR)` is DuckDB's own rendering of the stored value —
/// not a format this crate invents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NearestCell {
    /// The column, as the probe named it.
    pub column: String,
    /// Its value in this row.
    pub value: String,
}

/// **The outcome of one nearest read.**
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NearestRead {
    /// How many rows the query returned.
    ///
    /// Carried, rather than left implicit in `row`, because the clause that
    /// bounds this read is in the SQL and a caller trusting the clause is
    /// trusting a string. `the_nearest_read_returns_one_row_from_a_cluster`
    /// asserts this over a cluster of coincident points, where a read that had
    /// lost its bound returns all of them and this number says so.
    pub rows: usize,
    /// The row, as one cell per column the probe asked for — empty when
    /// nothing was inside the radius.
    ///
    /// A cell whose value is SQL NULL is **absent** rather than blank: the
    /// readout says what the datum is, and a column with no value in this row
    /// is not something to print an empty line for.
    pub cells: Vec<NearestCell>,
}

impl NearestRead {
    /// Whether anything was found inside the radius.
    #[must_use]
    pub fn found(&self) -> bool {
        self.rows > 0
    }
}

/// `name` written as a SQL identifier: double-quoted, with an embedded quote
/// doubled.
///
/// The same unconditional rule the shell's clause builder uses, and for the
/// same reason: a column named `select`, or named with a space, is as legal in
/// a file as any other, and a list of words that need quoting has to be
/// complete to be correct.
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// A float written so DuckDB parses it back as the same number.
///
/// `{:?}` on an `f64` is Rust's round-trip form, which is what this needs —
/// `{}` truncates and a truncated pointer position moves the answer. A
/// non-finite value cannot be written as a SQL literal, and is one of the
/// cases [`nearest_row_sql`] refuses over.
fn literal(v: f64) -> String {
    format!("{v:?}")
}

/// **The nearest-row query**, wrapped around a step's own emitted rows SQL.
///
/// `rows_sql` is the row-level query for the mark being hovered, carrying the
/// same static `data.filter` and the same live selection predicate the mark
/// was drawn under — the string `Session::execute_step_rows` runs for that
/// mark at the plot's own audience, which is the row set on screen rather than
/// the one a reader of the selection would get. Everything this adds is
/// outside it: a squared pixel distance, the radius test, the ordering, and
/// the bound.
///
/// Returns `None` when the probe cannot be expressed: a zero or non-finite
/// `per_pixel` on either axis (a degenerate scale — one where the rows are
/// equidistant, and dividing by it produces infinities rather than an
/// answer — `a_degenerate_axis_produces_no_query`), a
/// non-finite pointer position or radius, or an empty `read` list (a query
/// projecting nothing has no row to hand back).
///
/// # The shape, and why the distance is computed one level in
///
/// ```text
/// SELECT "a", "b" FROM (
///   SELECT *, <dx*dx + dy*dy> AS __bf_hover_distance FROM (<rows_sql>) AS bf_hover_rows
/// ) AS bf_hover
/// WHERE __bf_hover_distance <= <radius²>
/// ORDER BY __bf_hover_distance
/// LIMIT 1
/// ```
///
/// The distance is named in a subquery so the radius test and the ordering
/// both read the one expression rather than two copies of it, and so the
/// projection can be narrowed to the probe's columns without losing the
/// columns the distance is computed from.
///
/// A row whose x or y is SQL NULL scores NULL, `NULL <= r` is not true, and it
/// is filtered out — so a row with no position cannot be the nearest to a
/// position. That falls out of SQL's three-valued logic rather than being
/// spelled, and `a_row_with_a_null_coordinate_is_never_the_nearest` is what
/// holds it.
#[must_use]
pub fn nearest_row_sql(rows_sql: &str, probe: &NearestProbe) -> Option<String> {
    if probe.read.is_empty() {
        return None;
    }
    for axis in [&probe.x, &probe.y] {
        if !axis.at.is_finite() || !axis.per_pixel.is_finite() || axis.per_pixel == 0.0 {
            return None;
        }
    }
    if !probe.radius.is_finite() || probe.radius < 0.0 {
        return None;
    }

    let term = |axis: &NearestAxis| {
        let col = quote(&axis.column);
        let at = literal(axis.at);
        let per = literal(axis.per_pixel);
        format!("(({col} - {at}) / {per}) * (({col} - {at}) / {per})")
    };
    let distance = format!("{} + {}", term(&probe.x), term(&probe.y));

    let projection = probe
        .read
        .iter()
        .map(|c| {
            let q = quote(c);
            format!("CAST({q} AS VARCHAR) AS {q}")
        })
        .collect::<Vec<_>>()
        .join(", ");

    let mut sql = String::new();
    let _ = write!(
        sql,
        "SELECT {projection} FROM (SELECT *, {distance} AS {DISTANCE_COLUMN} \
         FROM ({rows_sql}) AS {ROWS_ALIAS}) AS {DISTANCE_ALIAS} \
         WHERE {DISTANCE_COLUMN} <= {} ORDER BY {DISTANCE_COLUMN} LIMIT 1",
        literal(probe.radius * probe.radius)
    );
    Some(sql)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe() -> NearestProbe {
        NearestProbe {
            x: NearestAxis {
                column: "lon".to_string(),
                at: 1.0,
                per_pixel: 0.5,
            },
            y: NearestAxis {
                column: "lat".to_string(),
                at: 2.0,
                per_pixel: 0.25,
            },
            read: vec!["lon".to_string(), "lat".to_string()],
            radius: 8.0,
        }
    }

    /// A degenerate axis produces no query at all, on either axis.
    ///
    /// Asked per axis rather than once: a guard written over `x` alone reads
    /// exactly like this one until the day a plot's *y* scale collapses.
    #[test]
    fn a_degenerate_axis_produces_no_query() {
        for zeroed in [0, 1] {
            let mut p = probe();
            if zeroed == 0 {
                p.x.per_pixel = 0.0;
            } else {
                p.y.per_pixel = 0.0;
            }
            assert!(
                nearest_row_sql("SELECT * FROM t", &p).is_none(),
                "axis {zeroed} has no pixels to measure in, so there is no query"
            );
        }
    }

    /// A probe asking for no columns produces no query: a projection of
    /// nothing has no row to report.
    #[test]
    fn a_probe_reading_no_columns_produces_no_query() {
        let mut p = probe();
        p.read.clear();
        assert!(nearest_row_sql("SELECT * FROM t", &p).is_none());
    }

    /// The projection is the probe's columns and nothing else — the property
    /// that stops a whole row reaching a caller that asked for two cells.
    #[test]
    fn the_projection_is_the_probes_columns_and_no_others() {
        let sql = nearest_row_sql("SELECT * FROM t", &probe()).expect("a query");
        let head = sql.split(" FROM ").next().expect("a projection");
        assert_eq!(
            head.matches("CAST(").count(),
            2,
            "two columns asked for, so two projected: {head}"
        );
        assert!(
            head.contains("\"lon\"") && head.contains("\"lat\""),
            "{head}"
        );
    }

    /// A column named with a quote is written as one identifier, not two.
    #[test]
    fn an_embedded_quote_is_doubled_in_the_identifier() {
        let mut p = probe();
        p.x.column = "od\"d".to_string();
        p.read = vec!["od\"d".to_string()];
        let sql = nearest_row_sql("SELECT * FROM t", &p).expect("a query");
        assert!(sql.contains("\"od\"\"d\""), "{sql}");
    }
}
