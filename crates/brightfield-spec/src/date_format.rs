//! d3-time-format's date specifier — `%b`, `%Y-%m-%d`, `%B %Y`, `%H:%M` — read
//! from a spec string and applied to an instant.
//!
//! A Mosaic spec's `xTickFormat` / `yTickFormat` on a date axis is a
//! d3-time-format specifier, and d3-time-format is the authority for what one
//! prints. This module ports its directives and its padding modifiers
//! (`%-d`, `%_d`, `%0d`), and prints in UTC, as Mosaic's renderer prints dates
//! (`utcFormat`, not `timeFormat`).
//!
//! One judge serves the parser and the renderer. [`DateFormat::parse`] decides
//! what a readable date format is and names the directive it cannot read, the
//! parse warning for such a value asks it, and the axis draws with what it
//! returns, so the warning and the drawing cannot disagree.
//!
//! **A directive outside d3-time-format's is not read.** d3-time-format prints
//! the character after an unknown `%` as it stands (`%K` prints `K`); this
//! build refuses the whole format instead and names the directive, because a
//! chart that prints `K` where its author asked for something else is wrong
//! without saying so. A `%` with nothing after it is the same refusal.
//!
//! The English locale is the only one: `Mon` and `March`, `AM` and `PM`, and the
//! composite directives `%c`, `%x` and `%X` read as d3-time-format's `en-US`.
//!
//! An instant is microseconds since the Unix epoch, which is what an Arrow
//! timestamp column holds. d3-time-format works in whole milliseconds, because a
//! JavaScript `Date` does, so `%L` and `%f` print the milliseconds of the second
//! (rounded down) and `%Q` and `%s` print the epoch in milliseconds and seconds.

const DAY_US: i64 = 86_400_000_000;

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The directives d3-time-format reads. Any other character after a `%` is one
/// this build refuses.
const DIRECTIVES: &str = "aAbBcdefgGHIjLmMpqQsSuUVwWxXyYZ%";

/// How a directive's number is padded: the character it fills with, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pad {
    /// The directive's own default: zero, or a space for `%e`.
    Default,
    /// `%-d` — no padding.
    None,
    /// `%_d` — a space.
    Space,
    /// `%0d` — a zero.
    Zero,
}

/// One piece of a date format: text that prints as it stands, or a directive.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Directive { code: char, pad: Pad },
}

/// A date specifier, parsed. `%b` is one directive, `%Y-%m-%d` is three
/// directives with two hyphens between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateFormat {
    segments: Vec<Segment>,
}

impl DateFormat {
    /// Read a specifier, or name the first directive that is not one
    /// d3-time-format reads, as it stands in `spec` (`%K`, `%-K`, or a bare `%`
    /// at the end).
    ///
    /// A specifier with no directive in it (`abc`) reads as text that prints as
    /// it stands; whether that is a date format at all is the caller's to
    /// judge, as [`crate::layout::read_tick_format`] does.
    ///
    /// # Errors
    ///
    /// The directive that is not read, with its `%` and any padding modifier.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut segments = Vec::new();
        let mut literal = String::new();
        let mut chars = spec.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                literal.push(c);
                continue;
            }
            let mut pad = Pad::Default;
            let mut written = String::from("%");
            let mut next = chars.next();
            if let Some(modifier) = next {
                let read = match modifier {
                    '-' => Some(Pad::None),
                    '_' => Some(Pad::Space),
                    '0' => Some(Pad::Zero),
                    _ => None,
                };
                if let Some(read) = read {
                    pad = read;
                    written.push(modifier);
                    next = chars.next();
                }
            }
            match next {
                Some(code) if DIRECTIVES.contains(code) => {
                    if !literal.is_empty() {
                        segments.push(Segment::Literal(std::mem::take(&mut literal)));
                    }
                    segments.push(Segment::Directive { code, pad });
                }
                Some(code) => {
                    written.push(code);
                    return Err(written);
                }
                None => return Err(written),
            }
        }
        if !literal.is_empty() {
            segments.push(Segment::Literal(literal));
        }
        Ok(Self { segments })
    }

    /// Whether the specifier holds at least one directive. A specifier that
    /// holds none is text, and no date format.
    #[must_use]
    pub fn has_directive(&self) -> bool {
        self.segments
            .iter()
            .any(|s| matches!(s, Segment::Directive { .. }))
    }

    /// The instant `micros` microseconds after the Unix epoch, printed in UTC.
    #[must_use]
    pub fn format(&self, micros: i64) -> String {
        let at = Instant::of(micros);
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Literal(text) => out.push_str(text),
                Segment::Directive { code, pad } => at.write(*code, *pad, &mut out),
            }
        }
        out
    }

    /// The calendar day `YYYY-MM-DD`, printed as the instant it begins in UTC,
    /// or `None` when `iso` is not such a day. A date column's tick names its
    /// day this way.
    #[must_use]
    pub fn format_iso_date(&self, iso: &str) -> Option<String> {
        iso_date_micros(iso).map(|micros| self.format(micros))
    }
}

/// The instant a calendar day `YYYY-MM-DD` begins at, in UTC, in microseconds
/// since the Unix epoch, or `None` when the text is not a day of that shape
/// (four-digit year, two-digit month and day, a day the month has).
#[must_use]
pub fn iso_date_micros(iso: &str) -> Option<i64> {
    let bytes = iso.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let text = &iso[range];
        if text.bytes().all(|b| b.is_ascii_digit()) {
            text.parse().ok()
        } else {
            None
        }
    };
    let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some(days_from_civil(year, month, day) * DAY_US)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date of a day count since 1970-01-01 (Howard
/// Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// One instant, broken into what the directives read.
struct Instant {
    micros: i64,
    days: i64,
    year: i64,
    /// 1 to 12.
    month: i64,
    /// 1 to 31.
    day: i64,
    /// 0 (Sunday) to 6.
    weekday: i64,
    /// 0-based day of the year.
    yday: i64,
    hour: i64,
    minute: i64,
    second: i64,
    milli: i64,
}

impl Instant {
    fn of(micros: i64) -> Self {
        let days = micros.div_euclid(DAY_US);
        let of_day = micros.rem_euclid(DAY_US);
        let (year, month, day) = civil_from_days(days);
        Self {
            micros,
            days,
            year,
            month,
            day,
            weekday: (days + 4).rem_euclid(7),
            yday: days - days_from_civil(year, 1, 1),
            hour: of_day / 3_600_000_000,
            minute: of_day / 60_000_000 % 60,
            second: of_day / 1_000_000 % 60,
            milli: of_day / 1_000 % 1_000,
        }
    }

    /// The Thursday of the ISO 8601 week this instant is in, as a day count.
    /// Monday to Wednesday take the Thursday after and Thursday to Sunday the
    /// Thursday before, which is d3-time-format's `dISO`.
    fn iso_thursday(&self) -> i64 {
        let back = (self.weekday - 4).rem_euclid(7);
        if self.weekday >= 4 || self.weekday == 0 {
            self.days - back
        } else {
            self.days + (4 - self.weekday)
        }
    }

    fn iso_year(&self) -> i64 {
        civil_from_days(self.iso_thursday()).0
    }

    fn iso_week(&self) -> i64 {
        let thursday = self.iso_thursday();
        let (year, _, _) = civil_from_days(thursday);
        (thursday - days_from_civil(year, 1, 1)) / 7 + 1
    }

    fn write(&self, code: char, pad: Pad, out: &mut String) {
        let zero = |value: i64, width: usize| padded(value, pad, '0', width);
        match code {
            'a' => out.push_str(&WEEKDAYS[self.weekday as usize][..3]),
            'A' => out.push_str(WEEKDAYS[self.weekday as usize]),
            'b' => out.push_str(&MONTHS[(self.month - 1) as usize][..3]),
            'B' => out.push_str(MONTHS[(self.month - 1) as usize]),
            'c' => {
                self.write_all("%x, %X", out);
            }
            'd' => out.push_str(&zero(self.day, 2)),
            'e' => out.push_str(&padded(self.day, pad, ' ', 2)),
            'f' => {
                out.push_str(&zero(self.milli, 3));
                out.push_str("000");
            }
            'g' => out.push_str(&zero(self.iso_year() % 100, 2)),
            'G' => out.push_str(&zero(self.iso_year() % 10_000, 4)),
            'H' => out.push_str(&zero(self.hour, 2)),
            'I' => out.push_str(&zero(
                if self.hour % 12 == 0 {
                    12
                } else {
                    self.hour % 12
                },
                2,
            )),
            'j' => out.push_str(&zero(self.yday + 1, 3)),
            'L' => out.push_str(&zero(self.milli, 3)),
            'm' => out.push_str(&zero(self.month, 2)),
            'M' => out.push_str(&zero(self.minute, 2)),
            'p' => out.push_str(if self.hour < 12 { "AM" } else { "PM" }),
            'q' => out.push_str(&(1 + (self.month - 1) / 3).to_string()),
            'Q' => out.push_str(&self.micros.div_euclid(1_000).to_string()),
            's' => out.push_str(&self.micros.div_euclid(1_000_000).to_string()),
            'S' => out.push_str(&zero(self.second, 2)),
            'u' => out.push_str(&(if self.weekday == 0 { 7 } else { self.weekday }).to_string()),
            'U' => out.push_str(&zero((self.yday + 7 - self.weekday) / 7, 2)),
            'V' => out.push_str(&zero(self.iso_week(), 2)),
            'w' => out.push_str(&self.weekday.to_string()),
            'W' => out.push_str(&zero((self.yday + 7 - (self.weekday + 6) % 7) / 7, 2)),
            'x' => self.write_all("%-m/%-d/%Y", out),
            'X' => self.write_all("%-I:%M:%S %p", out),
            'y' => out.push_str(&zero(self.year % 100, 2)),
            'Y' => out.push_str(&zero(self.year % 10_000, 4)),
            'Z' => out.push_str("+0000"),
            '%' => out.push('%'),
            other => unreachable!("`%{other}` is not a directive `DateFormat::parse` lets through"),
        }
    }

    /// A composite directive's expansion: fixed text that is a specifier of this
    /// module's own.
    fn write_all(&self, spec: &str, out: &mut String) {
        let format =
            DateFormat::parse(spec).expect("a composite directive is a readable specifier");
        for segment in &format.segments {
            match segment {
                Segment::Literal(text) => out.push_str(text),
                Segment::Directive { code, pad } => self.write(*code, *pad, out),
            }
        }
    }
}

/// d3-time-format's `pad`: the sign, then the digits filled out to `width` with
/// the directive's fill. `default` is the fill a directive takes when the
/// specifier names none.
fn padded(value: i64, pad: Pad, default: char, width: usize) -> String {
    let fill = match pad {
        Pad::Default => Some(default),
        Pad::None => None,
        Pad::Space => Some(' '),
        Pad::Zero => Some('0'),
    };
    let sign = if value < 0 { "-" } else { "" };
    let digits = value.unsigned_abs().to_string();
    match fill {
        Some(fill) if digits.len() < width => {
            let mut out = String::from(sign);
            out.extend(std::iter::repeat(fill).take(width - digits.len()));
            out.push_str(&digits);
            out
        }
        _ => format!("{sign}{digits}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Microseconds since the epoch of a UTC instant.
    fn at(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
        days_from_civil(year, month, day) * DAY_US
            + hour * 3_600_000_000
            + minute * 60_000_000
            + second * 1_000_000
    }

    fn print(spec: &str, micros: i64) -> String {
        DateFormat::parse(spec)
            .unwrap_or_else(|d| panic!("`{spec}` should read; `{d}` was refused"))
            .format(micros)
    }

    /// The three formats the card's first criterion names, on the day it names.
    #[test]
    fn a_day_prints_as_its_month_its_iso_date_and_its_month_and_year() {
        let march = at(2024, 3, 1, 0, 0, 0);
        assert_eq!(print("%b", march), "Mar");
        assert_eq!(print("%Y-%m-%d", march), "2024-03-01");
        assert_eq!(print("%B %Y", march), "March 2024");
    }

    /// The card's second criterion: five past two in the afternoon.
    #[test]
    fn five_past_two_in_the_afternoon_prints_as_fourteen_oh_five() {
        assert_eq!(print("%H:%M", at(2024, 3, 1, 14, 5, 0)), "14:05");
        assert_eq!(print("%I:%M %p", at(2024, 3, 1, 14, 5, 0)), "02:05 PM");
    }

    /// The instants a port of a calendar goes wrong at.
    #[test]
    fn the_calendar_holds_at_a_leap_day_a_year_end_and_before_the_epoch() {
        assert_eq!(print("%Y-%m-%d", at(2024, 2, 29, 0, 0, 0)), "2024-02-29");
        assert_eq!(print("%Y-%m-%d", at(1900, 3, 1, 0, 0, 0)), "1900-03-01");
        assert_eq!(print("%Y-%m-%d", at(2000, 2, 29, 0, 0, 0)), "2000-02-29");
        assert_eq!(print("%j", at(2024, 12, 31, 0, 0, 0)), "366");
        assert_eq!(print("%Y-%m-%d %H:%M:%S", -1), "1969-12-31 23:59:59");
        assert_eq!(print("%a %A", at(1970, 1, 1, 0, 0, 0)), "Thu Thursday");
    }

    /// `d3-time-format`'s padding modifiers: `%-d` drops the padding, `%_d`
    /// pads with a space, `%e` is a space-padded day and `%0e` a zero-padded one.
    #[test]
    fn a_padding_modifier_changes_the_fill() {
        let day = at(2024, 3, 5, 0, 0, 0);
        assert_eq!(print("%d|%-d|%_d|%e|%0e|%-e", day), "05|5| 5| 5|05|5");
    }

    /// A microsecond is below what d3-time-format can hold: `%L` and `%f` print
    /// the whole milliseconds of the second.
    #[test]
    fn milliseconds_round_down_and_microseconds_are_not_carried() {
        let instant = at(2024, 3, 1, 0, 0, 1) + 234_567;
        assert_eq!(print("%L", instant), "234");
        assert_eq!(print("%f", instant), "234000");
        assert_eq!(print("%Q", instant), "1709251201234");
        assert_eq!(print("%s", instant), "1709251201");
    }

    /// A directive outside d3-time-format's is named as written, with its
    /// modifier, and the whole specifier is refused.
    #[test]
    fn an_unknown_directive_is_named_as_it_was_written() {
        assert_eq!(DateFormat::parse("%K").unwrap_err(), "%K");
        assert_eq!(DateFormat::parse("%Y %-K").unwrap_err(), "%-K");
        assert_eq!(DateFormat::parse("%Y-%").unwrap_err(), "%");
        assert_eq!(DateFormat::parse("%_").unwrap_err(), "%_");
        assert!(DateFormat::parse("%Y %B").is_ok());
    }

    #[test]
    fn a_specifier_with_no_directive_is_text() {
        let format = DateFormat::parse("abc").expect("text reads");
        assert!(!format.has_directive());
        assert!(DateFormat::parse("%b").expect("reads").has_directive());
        assert_eq!(format.format(0), "abc");
    }

    #[test]
    fn a_day_is_read_only_as_a_real_iso_date() {
        assert_eq!(iso_date_micros("2024-03-01"), Some(at(2024, 3, 1, 0, 0, 0)));
        for not in [
            "2024-3-01",
            "2024-02-30",
            "2023-02-29",
            "2024-13-01",
            "2024-00-10",
            "2024-03-00",
            "24-03-01",
            "2024/03/01",
            "2024-03-01T00:00:00",
            "March",
            "",
            "+024-03-01",
        ] {
            assert_eq!(iso_date_micros(not), None, "`{not}` is no day");
        }
        let b = DateFormat::parse("%b").expect("reads");
        assert_eq!(b.format_iso_date("2024-03-01").as_deref(), Some("Mar"));
        assert_eq!(b.format_iso_date("Mar"), None);
    }
}
