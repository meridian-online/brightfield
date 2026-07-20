//! Sidebar profile formatting (sidebar profiling) —
//! framework-free.
//!
//! The engine computes the raw per-source profiles ([`SourceProfile`], its
//! types re-exported here); this module turns them into the exact strings the
//! [`SidebarPanel`] renders — the per-column stat line, the source header's
//! row-count label, the muted fallback rows, the displayed-column cap, and the
//! Warning message a failed source logs. `shell.rs` stays a rendering shim
//! that only lays these strings out. No gpui import may enter this file
//! (semantic-layer rule).
//!
//! [`SidebarPanel`]: crate::shell::SidebarPanel

pub use brightfield_engine::{ColumnProfile, ProfileOutcome, SourceProfile};

/// The most columns the sidebar renders per source before collapsing the tail
/// into a "(+N more)" row. Flat-list v1 scrolls, but a pathologically wide
/// source (hundreds of columns) would still make the panel unwieldy; the cap
/// keeps it bounded. Deferred: search/collapse/virtualization.
pub const COLUMN_DISPLAY_CAP: usize = 50;

/// Group a count with thousands separators (`231083` → `231,083`).
#[must_use]
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let len = digits.len();
    let mut out = String::with_capacity(len + len / 3);
    for (i, ch) in digits.char_indices() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The source header's row-count label — `"1 row"` / `"231,083 rows"` /
/// `"0 rows"`.
#[must_use]
pub fn row_count_label(n: u64) -> String {
    if n == 1 {
        "1 row".to_string()
    } else {
        format!("{} rows", thousands(n))
    }
}

/// Trim a DuckDB-rendered numeric bound: drop trailing zeros (and a bare
/// trailing dot) from a plain decimal so `"100.000"` → `"100"` and `"1.50"` →
/// `"1.5"`. Scientific-notation bounds (DuckDB renders DOUBLE in e-notation
/// outside ~1e15/1e-5) pass through UNTOUCHED — trimming their trailing zero
/// would corrupt the exponent (`"1.5e+20"` → `"1.5e+2"`, off by 10^18).
/// Non-numeric bounds (dates, timestamps, strings) also pass through.
#[must_use]
pub fn trim_number(s: &str) -> String {
    // Only trim a plain decimal: must parse as a float, carry a '.', and NOT
    // carry an exponent (whose trailing zeros are significant).
    if s.parse::<f64>().is_err() || !s.contains('.') || s.contains(['e', 'E']) {
        return s.to_string();
    }
    let trimmed = s.trim_end_matches('0');
    let trimmed = trimmed.strip_suffix('.').unwrap_or(trimmed);
    match trimmed {
        "" | "-" => "0".to_string(),
        other => other.to_string(),
    }
}

/// The per-column stat line the panel renders under the column name:
/// `"<distinct> distinct · <nulls> nulls"`, with `" · <min> – <max>"` appended
/// for numeric/temporal columns that carry bounds. Counts are
/// thousands-separated; numeric bounds are trimmed.
#[must_use]
pub fn stat_line(col: &ColumnProfile) -> String {
    let mut line = format!(
        "{} distinct · {} nulls",
        thousands(col.distinct),
        thousands(col.nulls)
    );
    if let (Some(min), Some(max)) = (&col.min, &col.max) {
        line.push_str(&format!(" · {} – {}", trim_number(min), trim_number(max)));
    }
    line
}

/// The muted row shown for an attached-database source (not profiled v1).
pub const UNSUPPORTED_ROW: &str = "(attached database — not profiled)";

/// The muted row shown for a source whose profiling failed, carrying the
/// reason.
#[must_use]
pub fn unavailable_row(reason: &str) -> String {
    format!("(profile unavailable: {reason})")
}

/// How many columns to display and the optional "(+N more)" tail — the cap
/// keeps a pathologically wide source bounded.
#[must_use]
pub fn column_cap(total: usize) -> (usize, Option<String>) {
    if total > COLUMN_DISPLAY_CAP {
        (
            COLUMN_DISPLAY_CAP,
            Some(format!("(+{} more)", total - COLUMN_DISPLAY_CAP)),
        )
    } else {
        (total, None)
    }
}

/// The FeedbackLog message a failed source appends at `Severity::Warning`
/// (no toast) — at launch and on hot reload alike.
#[must_use]
pub fn profile_warning(source_name: &str, reason: &str) -> String {
    format!("Profile unavailable for '{source_name}': {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(
        name: &str,
        ty: &str,
        non_null: u64,
        nulls: u64,
        distinct: u64,
        min: Option<&str>,
        max: Option<&str>,
    ) -> ColumnProfile {
        ColumnProfile {
            name: name.to_string(),
            type_name: ty.to_string(),
            non_null,
            nulls,
            distinct,
            min: min.map(str::to_string),
            max: max.map(str::to_string),
        }
    }

    /// Thousands + row-count label: counts group with commas; the
    /// header label is singular at 1 and plural (incl. 0) otherwise.
    #[test]
    fn thousands_and_row_count_label() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(42), "42");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(231_083), "231,083");
        assert_eq!(thousands(1_000_000), "1,000,000");

        assert_eq!(row_count_label(0), "0 rows");
        assert_eq!(row_count_label(1), "1 row");
        assert_eq!(row_count_label(231_083), "231,083 rows");
    }

    /// Trimmed bounds: floats lose trailing zeros; ints and
    /// temporal/string bounds pass through untouched.
    #[test]
    fn trim_number_matrix() {
        assert_eq!(trim_number("100.000"), "100");
        assert_eq!(trim_number("1.50"), "1.5");
        assert_eq!(trim_number("2.500"), "2.5"); // plain decimal still trims
        assert_eq!(trim_number("0.0"), "0");
        assert_eq!(trim_number("-1.50"), "-1.5");
        assert_eq!(trim_number("42"), "42"); // int, no dot
                                             // Scientific notation passes through untouched — trimming would eat the
                                             // exponent's trailing zero and corrupt the value by orders of magnitude.
        assert_eq!(trim_number("1.5e+20"), "1.5e+20");
        assert_eq!(trim_number("1.5e-10"), "1.5e-10");
        assert_eq!(trim_number("1.5E+20"), "1.5E+20"); // uppercase E too
        assert_eq!(trim_number("2020-01-15"), "2020-01-15"); // date passes through
        assert_eq!(trim_number("2020-01-15 10:30:00"), "2020-01-15 10:30:00");
    }

    /// Stat line matrix: ints/floats/dates show a trimmed range;
    /// strings and all-null columns show only distinct/nulls.
    #[test]
    fn stat_line_by_type() {
        // Integer column with a range.
        let ints = col(
            "delay",
            "INTEGER",
            231_080,
            3,
            1_400,
            Some("-99"),
            Some("1439"),
        );
        assert_eq!(stat_line(&ints), "1,400 distinct · 3 nulls · -99 – 1439");

        // Float column: bounds are trimmed.
        let floats = col("dist", "DOUBLE", 100, 0, 100, Some("1.50"), Some("999.000"));
        assert_eq!(stat_line(&floats), "100 distinct · 0 nulls · 1.5 – 999");

        // Date column: bounds pass through.
        let dates = col(
            "day",
            "DATE",
            10,
            0,
            10,
            Some("2020-01-01"),
            Some("2020-12-31"),
        );
        assert_eq!(
            stat_line(&dates),
            "10 distinct · 0 nulls · 2020-01-01 – 2020-12-31"
        );

        // Varchar column: no min/max, just distinct/nulls.
        let strings = col("origin", "VARCHAR", 231_083, 0, 322, None, None);
        assert_eq!(stat_line(&strings), "322 distinct · 0 nulls");

        // All-null gated column: bounds are None → no range shown.
        let allnull = col("maybe", "DOUBLE", 0, 500, 0, None, None);
        assert_eq!(stat_line(&allnull), "0 distinct · 500 nulls");
    }

    /// Cap boundary: at or under the cap shows everything with no
    /// tail; over the cap shows exactly the cap plus a "(+N more)" tail.
    #[test]
    fn column_cap_boundary() {
        assert_eq!(column_cap(0), (0, None));
        assert_eq!(column_cap(COLUMN_DISPLAY_CAP), (COLUMN_DISPLAY_CAP, None));
        let (shown, tail) = column_cap(COLUMN_DISPLAY_CAP + 7);
        assert_eq!(shown, COLUMN_DISPLAY_CAP);
        assert_eq!(tail.as_deref(), Some("(+7 more)"));
    }

    /// Fallback rows + warning: the muted fallback strings and the
    /// log warning carry the source name / reason verbatim.
    #[test]
    fn fallback_rows_and_warning() {
        assert_eq!(UNSUPPORTED_ROW, "(attached database — not profiled)");
        assert_eq!(
            unavailable_row("IO Error: No files found"),
            "(profile unavailable: IO Error: No files found)"
        );
        assert_eq!(
            profile_warning("flights", "IO Error: No files found"),
            "Profile unavailable for 'flights': IO Error: No files found"
        );
    }
}
