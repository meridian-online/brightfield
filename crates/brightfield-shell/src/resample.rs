//! A timestamp column counted at a calendar step, so a column of instants has
//! a picture.
//!
//! A `DATE` arrives with its bucket already chosen — the day — and
//! [`crate::chart_kinds::COUNTS_OVER_TIME`] draws it straight. A `TIMESTAMP`
//! arrives with none, and bound to a band axis it puts **0** pixels of mark ink
//! on the page: `brightfield-render`'s `positional_axis_class` reads it as
//! continuous, and a bar has no band to stand on. The measurement is the test
//! `a_timestamp_band_puts_no_ink_on_the_page` in [`crate::chart_kinds`], beside
//! the `VARCHAR` control that tells a broken harness from a real zero.
//!
//! So the column a tile draws for a timestamp is not the timestamp. It is a
//! **bucket** column derived beside it — one `strftime` over the instant — and
//! [`crate::dashboard::Dashboard::to_spec`] declares it, because the `data:`
//! block is the dashboard's to write.
//!
//! # Why an ISO string rather than a truncated timestamp
//!
//! `date_trunc('hour', t)` is itself a `TIMESTAMP` and lands back on the
//! continuous axis that draws nothing. `CAST(… AS DATE)` escapes that, and only
//! for the steps a calendar date can spell — not an hour, not a minute.
//! `strftime` spells every step of the ladder below, and its output is
//! fixed-width and zero-padded, so ascending on the text is ascending in time:
//! that is the ordering `brightfield-sql`'s `BarLowerer` gives a band
//! aggregation with no `sort:` lifted. The test that walks the ladder is
//! `each_steps_format_extends_the_one_above_it`, and the one that reads the
//! absent `sort:` off an emitted dashboard is
//! `the_timestamps_tile_is_the_dates_tile_over_the_bucket_column` in
//! [`crate::dashboard`].
//!
//! The bucket column is therefore the same picture a `DATE` already gets rather
//! than a second device. `brightfield-render`'s `column_as_string` keys a band
//! by string and answers for `Date32` in the ISO spelling, so a `DATE` band and
//! a `strftime` band reach the scale as the same values.
//!
//! # The step follows the span
//!
//! [`step_for`] reads the column's own `min` and `max` and takes the **finest**
//! step whose bucket count a tile can show apart. Nothing in the choice reads
//! the column's cardinality: two tables of the same span answer the same step
//! whether one row fell in each bucket or a million did.

use brightfield_engine::ColumnProfile;

use crate::chart_kinds::type_base;
use crate::dashboard::TILE_WIDTH;

/// The most buckets a step may produce and still be chosen: one per logical
/// point of [`TILE_WIDTH`].
///
/// A tile is that many points wide, so past one bucket per point two adjacent
/// buckets share a column of pixels and the reader cannot tell them apart. This
/// is a choice of **resolution**, which is a different thing from a cap on a
/// series: [`crate::dashboard::time_bars_tile`] writes no `limit:` and a
/// finished picture keeps every bucket it has. A `DATE` needs no such choice
/// because its source already made one. The test that reads the missing
/// `limit:` off the emitted source is
/// `the_dates_tile_counts_in_time_order_and_drops_no_date` in
/// [`crate::dashboard`].
const MAX_BUCKETS: i128 = TILE_WIDTH as i128;

/// A calendar step a timestamp column is counted at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The instant itself, to DuckDB's microsecond resolution.
    Microsecond,
    /// The second.
    Second,
    /// The minute.
    Minute,
    /// The hour.
    Hour,
    /// The calendar day.
    Day,
    /// The calendar month.
    Month,
    /// The calendar year.
    Year,
}

/// The ladder [`step_for`] walks, finest first.
///
/// Declaration order is the search order, and the search takes the first
/// entry that fits — so a coarser step is reached only by the finer ones
/// failing [`MAX_BUCKETS`]. The test is
/// `a_spans_step_is_the_finest_a_tile_can_show_apart`, which asks a span at
/// each rung.
const LADDER: [Step; 7] = [
    Step::Microsecond,
    Step::Second,
    Step::Minute,
    Step::Hour,
    Step::Day,
    Step::Month,
    Step::Year,
];

/// Microseconds in a second, the unit DuckDB renders a `TIMESTAMP` in.
const US_PER_SECOND: i128 = 1_000_000;
/// Seconds in a day.
const SECONDS_PER_DAY: i128 = 86_400;

impl Step {
    /// The `strftime` format that spells this step.
    ///
    /// Each is a prefix of the one below it, which is what makes the text sort
    /// chronologically at every step.
    #[must_use]
    pub const fn format(self) -> &'static str {
        match self {
            Self::Microsecond => "%Y-%m-%d %H:%M:%S.%f",
            Self::Second => "%Y-%m-%d %H:%M:%S",
            Self::Minute => "%Y-%m-%d %H:%M",
            Self::Hour => "%Y-%m-%d %H",
            Self::Day => "%Y-%m-%d",
            Self::Month => "%Y-%m",
            Self::Year => "%Y",
        }
    }

    /// The step's name, as the derived column and the spec's comment spell it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Microsecond => "microsecond",
            Self::Second => "second",
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::Month => "month",
            Self::Year => "year",
        }
    }

    /// The step's length in microseconds.
    ///
    /// A month and a year are **nominal** — 30 days and 365 days — because the
    /// question the shell asks of this number is how many buckets a span holds,
    /// and that answer is compared against the private `MAX_BUCKETS` rather
    /// than reported. The two readers are [`Step::buckets`], held by
    /// `a_spans_step_is_the_finest_a_tile_can_show_apart`, and the ordering
    /// assertion inside `each_steps_format_extends_the_one_above_it`.
    #[must_use]
    pub const fn micros(self) -> i128 {
        match self {
            Self::Microsecond => 1,
            Self::Second => US_PER_SECOND,
            Self::Minute => 60 * US_PER_SECOND,
            Self::Hour => 3_600 * US_PER_SECOND,
            Self::Day => SECONDS_PER_DAY * US_PER_SECOND,
            Self::Month => 30 * SECONDS_PER_DAY * US_PER_SECOND,
            Self::Year => 365 * SECONDS_PER_DAY * US_PER_SECOND,
        }
    }

    /// How many buckets a span of `micros` microseconds holds at this step.
    #[must_use]
    pub const fn buckets(self, micros: i128) -> i128 {
        micros / self.micros() + 1
    }
}

/// Whether a DuckDB type is a timestamp this module resamples.
///
/// `DATE` is out because it is drawn as it stands. `TIME` is out because it
/// carries no calendar date, so `strftime`'s formats have nothing to spell —
/// a `TIME` column keeps the omission `chart_kinds::fields_of` gives it, with
/// the reason written into the spec. `the_resampled_types_are_the_timestamps`
/// is the test that names both exclusions.
#[must_use]
pub fn is_resampled_type(duckdb_type: &str) -> bool {
    matches!(
        type_base(duckdb_type).as_str(),
        "DATETIME" | "TIMESTAMP" | "TIMESTAMPTZ" | "TIMESTAMP_S" | "TIMESTAMP_MS" | "TIMESTAMP_NS"
    )
}

/// The step `column` is counted at: the finest whose bucket count is at most
/// the private `MAX_BUCKETS` above.
///
/// Two answers are [`Step::Microsecond`] rather than a coarser step, and both
/// are the same rule read at its ends:
///
/// - a span shorter than the first step that fits — a whole column inside one
///   second — would draw a single bar, which is a true picture of nothing, so
///   the instants are counted as they stand;
/// - a column with no `min` or `max` to subtract has no span to read. The
///   engine gathers both for the temporal types (`is_min_max_type` in
///   `brightfield-engine`), so this answer is reached by a hand-built profile
///   rather than by a profiled table.
///
/// A span past every step's ceiling takes the coarsest — a picture of a
/// millennium is a picture of its years or of nothing.
#[must_use]
pub fn step_for(column: &ColumnProfile) -> Step {
    let Some(span) = span_micros(column) else {
        return Step::Microsecond;
    };
    for step in LADDER {
        if step.buckets(span) <= MAX_BUCKETS {
            return if step.buckets(span) >= 2 {
                step
            } else {
                Step::Microsecond
            };
        }
    }
    Step::Year
}

/// The name the bucket column is declared under, given the names already in
/// the table.
///
/// `taken` is every column the table has, so the derived name cannot collide
/// with one of them.
#[must_use]
pub fn derived_name(column: &str, step: Step, taken: &[String]) -> String {
    let base = format!("{column} by {}", step.label());
    let mut name = base.clone();
    let mut n = 1;
    while taken.contains(&name) {
        n += 1;
        name = format!("{base} {n}");
    }
    name
}

/// The bucket column as a SQL projection: one `strftime` over `column`, aliased
/// to `derived`.
///
/// The `CAST` is what carries the seconds/milliseconds/nanoseconds timestamps
/// and the zoned one onto the microsecond timestamp `strftime` reads.
#[must_use]
pub fn projection(column: &str, derived: &str, step: Step) -> String {
    format!(
        "strftime(CAST({} AS TIMESTAMP), '{}') AS {}",
        crate::sql_ident::quote(column),
        step.format(),
        crate::sql_ident::quote(derived)
    )
}

/// The column's span in microseconds, from the `min` and `max` the profile
/// carries.
fn span_micros(column: &ColumnProfile) -> Option<i128> {
    let lo = micros_since_epoch(column.min.as_deref()?)?;
    let hi = micros_since_epoch(column.max.as_deref()?)?;
    Some((hi - lo).max(0))
}

/// A DuckDB-rendered temporal value as microseconds from the epoch.
///
/// The shape read is `YYYY-MM-DD` with an optional ` HH:MM:SS` and an optional
/// fractional second, which is what `CAST(<temporal> AS VARCHAR)` writes. Text
/// after the seconds — a zoned column's `+01` offset — is left unread: both
/// ends of the span carry the same offset, so it cancels in the subtraction.
fn micros_since_epoch(rendered: &str) -> Option<i128> {
    let mut written = rendered.trim();
    let negative = written.starts_with('-');
    if negative {
        written = &written[1..];
    }
    let (date, rest) = written.split_once(' ').unwrap_or((written, ""));
    let mut parts = date.split('-');
    let year: i128 = parts.next()?.parse().ok()?;
    let month: i128 = parts.next()?.parse().ok()?;
    let day: i128 = parts.next()?.parse().ok()?;
    let year = if negative { -year } else { year };
    let mut micros = days_from_civil(year, month, day) * SECONDS_PER_DAY * US_PER_SECOND;

    let time = rest.trim();
    if !time.is_empty() {
        let mut clock = time.split(':');
        let hours: i128 = clock.next()?.parse().ok()?;
        let minutes: i128 = clock.next().unwrap_or("0").parse().ok()?;
        let seconds = clock.next().unwrap_or("0");
        let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
        // A zone offset rides on the end of the last field; stop at the first
        // byte that is not a digit rather than failing the parse.
        let whole: i128 = digits(whole)?;
        // The fraction is padded and then cut to microseconds, so `.5` is half
        // a second and a nanosecond column's ninth digit is dropped rather
        // than read as a thousand seconds.
        let fraction: String = format!("{:0<6}", digits_str(fraction))
            .chars()
            .take(6)
            .collect();
        let fraction = digits(&fraction)?;
        micros += (hours * 3_600 + minutes * 60 + whole) * US_PER_SECOND + fraction;
    }
    Some(micros)
}

/// The leading digits of `s`, as a number. `None` when it starts with a byte
/// that is not a digit.
fn digits(s: &str) -> Option<i128> {
    let taken = digits_str(s);
    if taken.is_empty() {
        return None;
    }
    taken.parse().ok()
}

/// The leading digits of `s`.
fn digits_str(s: &str) -> String {
    s.chars().take_while(char::is_ascii_digit).collect()
}

/// Days from 1970-01-01 to the given civil date, by Howard Hinnant's
/// `days_from_civil` — the algorithm `chrono` and `libstdc++` both implement,
/// written out here rather than pulling a date crate into the shell for one
/// subtraction.
///
/// Valid for a proleptic Gregorian calendar, which is the calendar DuckDB
/// renders a timestamp in.
fn days_from_civil(year: i128, month: i128, day: i128) -> i128 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_engine::SemanticType;

    fn stamped(min: &str, max: &str) -> ColumnProfile {
        ColumnProfile {
            name: "t".to_string(),
            type_name: "TIMESTAMP".to_string(),
            non_null: 100,
            nulls: 0,
            distinct: 100,
            min: Some(min.to_string()),
            max: Some(max.to_string()),
            semantic: SemanticType::NotAsked,
        }
    }

    /// **The step a span answers, at every rung of the ladder**, so the rule is
    /// a table a reader can check rather than a walk they have to simulate.
    #[test]
    fn a_spans_step_is_the_finest_a_tile_can_show_apart() {
        let cases: [(&str, &str, Step); 8] = [
            // Under a second: no coarser step has two buckets to draw.
            (
                "2020-01-01 00:00:00",
                "2020-01-01 00:00:00.5",
                Step::Microsecond,
            ),
            // Five minutes is 301 seconds, which fits; six minutes is 361.
            ("2020-01-01 00:00:00", "2020-01-01 00:05:00", Step::Second),
            ("2020-01-01 00:00:00", "2020-01-01 00:06:00", Step::Minute),
            ("2020-01-01 00:00:00", "2020-01-01 04:00:00", Step::Minute),
            ("2020-01-01 00:00:00", "2020-01-02 00:00:00", Step::Hour),
            // The case the card was filed for, arriving as instants: three
            // months of readings are counted per day.
            ("2020-01-01 00:00:00", "2020-04-01 00:00:00", Step::Day),
            ("2020-01-01 00:00:00", "2024-01-01 00:00:00", Step::Month),
            ("1020-01-01 00:00:00", "2020-01-01 00:00:00", Step::Year),
        ];
        for (min, max, want) in cases {
            assert_eq!(step_for(&stamped(min, max)), want, "{min} .. {max}");
        }
    }

    /// No step is chosen that a tile cannot show apart, and none is chosen that
    /// draws a single bar — the two bounds the walk is written to hold.
    #[test]
    fn the_chosen_step_lands_between_one_bar_and_one_per_point() {
        for (min, max) in [
            ("2020-01-01 00:00:00", "2020-01-01 00:00:00.000004"),
            ("2020-01-01 00:00:00", "2020-01-01 00:00:03"),
            ("2020-01-01 00:00:00", "2020-01-01 04:00:00"),
            ("2020-01-01 00:00:00", "2020-02-14 00:00:00"),
            ("2020-01-01 00:00:00", "2021-06-01 00:00:00"),
            ("2020-01-01 00:00:00", "2400-01-01 00:00:00"),
        ] {
            let column = stamped(min, max);
            let step = step_for(&column);
            let span = span_micros(&column).expect("a span");
            assert!(
                step.buckets(span) >= 2,
                "{min} .. {max} draws one bar at the {} step",
                step.label()
            );
            // The coarsest step is the ladder's floor: a span past every
            // ceiling has nowhere finer to go.
            assert!(
                step.buckets(span) <= MAX_BUCKETS || step == Step::Year,
                "{min} .. {max} asks for {} buckets at the {} step",
                step.buckets(span),
                step.label()
            );
        }
    }

    /// A profile carrying no span answers the instant, which is the one step
    /// that cannot collapse a column with two distinct values into one bar.
    #[test]
    fn a_column_with_no_span_is_counted_as_it_stands() {
        let mut column = stamped("2020-01-01", "2020-06-01");
        column.min = None;
        assert_eq!(step_for(&column), Step::Microsecond);
        let mut column = stamped("2020-01-01", "2020-06-01");
        column.max = None;
        assert_eq!(step_for(&column), Step::Microsecond);
    }

    /// **The rendering DuckDB writes is the rendering this parses**, including
    /// the fractional second and a zoned column's offset.
    #[test]
    fn a_rendered_timestamp_parses_back_to_the_instant_it_names() {
        assert_eq!(micros_since_epoch("1970-01-01 00:00:00"), Some(0));
        assert_eq!(micros_since_epoch("1970-01-02"), Some(86_400_000_000));
        assert_eq!(micros_since_epoch("1969-12-31"), Some(-86_400_000_000));
        assert_eq!(
            micros_since_epoch("1970-01-01 00:00:01.000123"),
            Some(1_000_123)
        );
        // Milliseconds are three digits and mean three digits: `.5` is half a
        // second, not five microseconds.
        assert_eq!(micros_since_epoch("1970-01-01 00:00:00.5"), Some(500_000));
        // A zoned column renders its offset onto the last field. Both ends of
        // a span carry it, so it is read past rather than failed on.
        assert_eq!(
            micros_since_epoch("1970-01-01 00:00:02+01"),
            Some(2_000_000)
        );
        assert_eq!(micros_since_epoch("not a timestamp"), None);
    }

    /// The types this build resamples, named — and the two temporal types it
    /// does not, each for its own reason.
    #[test]
    fn the_resampled_types_are_the_timestamps() {
        for t in [
            "TIMESTAMP",
            "DATETIME",
            "TIMESTAMPTZ",
            "TIMESTAMP WITH TIME ZONE",
            "TIMESTAMP_S",
            "TIMESTAMP_MS",
            "TIMESTAMP_NS",
            "timestamp",
        ] {
            assert!(is_resampled_type(t), "{t}");
        }
        for t in ["DATE", "TIME", "TIMETZ", "VARCHAR", "BIGINT"] {
            assert!(!is_resampled_type(t), "{t}");
        }
    }

    /// The bucket column takes a name the table does not already carry.
    #[test]
    fn a_derived_name_steps_aside_for_a_column_that_owns_it() {
        let taken = vec!["t".to_string(), "t by day".to_string()];
        assert_eq!(derived_name("t", Step::Day, &taken), "t by day 2");
        assert_eq!(derived_name("t", Step::Day, &[]), "t by day");
        assert_eq!(derived_name("t", Step::Hour, &taken), "t by hour");
    }

    /// Every step's format is a prefix of the next finer one's, which is what
    /// makes the emitted text sort chronologically at each of them.
    #[test]
    fn each_steps_format_extends_the_one_above_it() {
        for pair in LADDER.windows(2) {
            let (fine, coarse) = (pair[0], pair[1]);
            assert!(
                fine.format().starts_with(coarse.format()),
                "{} does not extend {}",
                fine.label(),
                coarse.label()
            );
            assert!(
                fine.micros() < coarse.micros(),
                "{} is not finer than {}",
                fine.label(),
                coarse.label()
            );
        }
    }

    /// A name SQL would misread survives into the projection quoted.
    #[test]
    fn the_projection_quotes_both_names() {
        assert_eq!(
            projection("seen at", "seen at by hour", Step::Hour),
            "strftime(CAST(\"seen at\" AS TIMESTAMP), '%Y-%m-%d %H') AS \"seen at by hour\""
        );
    }
}
