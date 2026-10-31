//! How a column's values are shown: the one rule a tile's axis, the grid and the tooltip share.
//!
//! The rule splits by who knows what. FineType's display block for the column's label says what
//! the label implies: money with two decimals, a fraction shown as a percentage, a year with no
//! grouping, a size in bytes. The column's profile says what only the data shows: the scale its
//! magnitude needs and the decimals a percentage's spread needs. The column's record says the
//! currency symbol the source wrote and how it wrote negatives, and holds the analyst's
//! preferred format, which wins over both.
//!
//! Only `$` reaches a tick format, since a chart file carries a d3-format specifier and no
//! locale. Any other symbol is named in the axis title and printed by the grid and the tooltip,
//! which are Brightfield's own surfaces. No symbol is ever made up: a column whose record names
//! none (its values mixed symbols, or it has no record yet) is shown with none.

use brightfield_engine::ColumnProfile;
use brightfield_protocol::display::Display;
use brightfield_spec::date_format::DateFormat;
use brightfield_spec::number_format::NumberFormat;

use crate::chart_kinds::is_temporal_type;
use crate::column_record::{FieldRecord, Negatives};
use crate::flat_cast::is_number_type;

/// Between a number and its unit word in the tooltip, so `2.4 bn` never wraps.
const NO_BREAK_SPACE: char = '\u{a0}';

/// The sign d3-format prints in front of a negative number.
const MINUS: char = '\u{2212}';

/// The magnitude at which money is scaled, and the scales, largest first.
const MONEY_SCALED_FROM: f64 = 1e4;
const MONEY_SCALES: [Scale; 3] = [
    Scale {
        divisor: 1e9,
        unit: "bn",
    },
    Scale {
        divisor: 1e6,
        unit: "m",
    },
    Scale {
        divisor: 1e3,
        unit: "k",
    },
];

/// The magnitude at which a size in bytes is scaled, and the binary scales, largest first.
const BYTES_SCALED_FROM: f64 = 1_048_576.0;
/// No `KiB`: scaling starts at a mebibyte, where `MiB` already fits.
const BYTE_SCALES: [Scale; 2] = [
    Scale {
        divisor: 1_073_741_824.0,
        unit: "GiB",
    },
    Scale {
        divisor: 1_048_576.0,
        unit: "MiB",
    },
];

/// The most decimals a percentage is given, and how many evenly spaced ticks across its spread
/// must print distinctly at the decimals it is given.
const MOST_PERCENT_DECIMALS: usize = 3;
const SPREAD_TICKS: u32 = 5;

/// How one column's values are shown on each surface.
///
/// A column the rule has nothing to say about (an unlabelled column, a label with no display
/// block, or a text column) gets [`Representation::default`], which each surface draws as it did
/// before; `unlabelled_column_has_no_representation` and
/// `a_text_column_under_a_fraction_label_stays_text` hold it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Representation {
    /// The d3-format or d3-time-format specifier a tile writes as `xTickFormat` / `yTickFormat`.
    /// For a scaled column it formats the scaled value.
    pub tick_format: Option<String>,
    /// The divisor a tile's bin applies, and the unit it leaves the values in.
    pub scale: Option<Scale>,
    /// What the axis title names: the currency symbol the ticks do not print, the scale's unit,
    /// both (`£bn`), or `%`. A label that fixes a currency, on a column with no recorded symbol,
    /// names the currency's ISO code here and nowhere else (`CHF bn`).
    pub title_unit: Option<String>,
    /// The specifier the grid prints a full value with. A money column's carries no currency
    /// symbol; [`format_cell`] adds [`Representation::symbol`].
    pub cell_format: Option<String>,
    /// The specifier the tooltip prints a value with; [`format_compact`] adds the symbol and the
    /// unit word, and on a scaled column raises the precision for a value that needs more.
    pub compact_format: Option<String>,
    /// The currency symbol the source wrote, which the grid and the tooltip print.
    pub symbol: Option<String>,
}

/// A divisor and the unit word the divided values are in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub divisor: f64,
    /// `k`, `m` or `bn` for money; `MiB` or `GiB` for bytes.
    pub unit: &'static str,
}

/// How `column`'s values are shown, from FineType's `display` block for its label, its `record`,
/// and whether the label's cast ran on it (`cast`).
///
/// `cast` matters for a percentage: the percentage cast stores a fraction (12% is 0.12), while a
/// column DuckDB already stored as a number is never cast and holds 12. The first is printed
/// through d3-format's `%`, which multiplies by 100; the second through a plain number with the
/// sign attached.
///
/// A number format is given only to a column stored as a number, and a date format only to a
/// date, time or timestamp column. A label FineType types as text keeps its values text, so a
/// text column under a rate label gets nothing.
///
/// On a money column a `$` in the analyst's preferred format is dropped from the tick format
/// unless the record holds `$`, since only a recorded `$` reaches a tick; the grid and the
/// tooltip print the recorded symbol instead.
#[must_use]
pub fn representation_of(
    column: &ColumnProfile,
    display: Option<&Display>,
    record: Option<&FieldRecord>,
    cast: bool,
) -> Representation {
    if is_temporal_type(&column.type_name) {
        let preferred = record
            .and_then(|r| r.prefer.date_format.as_deref())
            .filter(|f| DateFormat::parse(f).is_ok());
        let labelled = display
            .and_then(|d| d.date_format.as_deref())
            .filter(|f| DateFormat::parse(f).is_ok());
        return preferred
            .or(labelled)
            .map_or_else(Representation::default, |f| Representation {
                tick_format: Some(f.to_owned()),
                cell_format: Some(f.to_owned()),
                compact_format: Some(f.to_owned()),
                ..Representation::default()
            });
    }
    if !is_number_type(&column.type_name) {
        return Representation::default();
    }
    number_representation(column, display, record, cast)
}

/// The representation of a column stored as a number.
fn number_representation(
    column: &ColumnProfile,
    display: Option<&Display>,
    record: Option<&FieldRecord>,
    cast: bool,
) -> Representation {
    let preferred = record
        .and_then(|r| r.prefer.number_format.as_deref())
        .filter(|f| NumberFormat::parse(f).is_some());
    let labelled = display
        .and_then(|d| d.number_format.as_deref())
        .filter(|f| NumberFormat::parse(f).is_some());
    let block = labelled.and(display);
    let money = block.is_some_and(|d| d.money);
    let fraction = block.is_some_and(|d| d.fraction) && !money;
    let bytes = block.is_some_and(|d| d.bytes) && !money && !fraction;
    let symbol = record
        .and_then(|r| r.written.currency_symbol.clone())
        .filter(|s| money && !s.is_empty());
    let range = range_of(column);

    let (tick, cell, compact, scale) = if let Some(preferred) = preferred {
        let (tick, cell) = if money {
            let parts = Parts::of(preferred);
            (
                parts.with_dollar(symbol.as_deref() == Some("$") && parts.dollar),
                parts.with_dollar(false),
            )
        } else {
            (preferred.to_owned(), preferred.to_owned())
        };
        (tick, cell.clone(), cell, None)
    } else if let Some(labelled) = labelled {
        if fraction {
            let decimals = spread_decimals(range, if cast { 100.0 } else { 1.0 });
            let spec = if cast {
                format!(".{decimals}%")
            } else {
                format!(".{decimals}f")
            };
            (spec.clone(), spec.clone(), spec, None)
        } else if money || bytes {
            let scale = range.and_then(|r| {
                scale_of(
                    r,
                    if money {
                        &MONEY_SCALES[..]
                    } else {
                        &BYTE_SCALES[..]
                    },
                    if money {
                        MONEY_SCALED_FROM
                    } else {
                        BYTES_SCALED_FROM
                    },
                )
            });
            let mut parts = Parts::of(labelled);
            if money {
                match record.and_then(|r| r.written.negatives) {
                    Some(Negatives::Parentheses) => parts.sign = Some('('),
                    Some(Negatives::Minus) => parts.sign = None,
                    None => {}
                }
            }
            let comma = if parts.rest.contains(',') { "," } else { "" };
            let tick = Parts {
                rest: &format!("{comma}~f"),
                ..parts
            }
            .with_dollar(symbol.as_deref() == Some("$"));
            let cell = parts.with_dollar(false);
            let compact = if scale.is_some() {
                Parts {
                    rest: &format!("{comma}.1~f"),
                    ..parts
                }
                .with_dollar(false)
            } else {
                cell.clone()
            };
            (tick, cell, compact, scale)
        } else {
            (
                labelled.to_owned(),
                labelled.to_owned(),
                labelled.to_owned(),
                None,
            )
        }
    } else {
        return Representation::default();
    };

    let mut unit = String::new();
    if let Some(symbol) = &symbol {
        if !(symbol == "$" && Parts::of(&tick).dollar) {
            unit.push_str(symbol);
        }
    } else if let Some(code) = block.and_then(|d| d.currency.as_deref()).filter(|_| money) {
        unit.push_str(code);
        if scale.is_some() {
            unit.push(' ');
        }
    }
    if let Some(scale) = scale {
        unit.push_str(scale.unit);
    }
    // A preference is drawn as written, so the title says `%` only where its format prints one;
    // the rule's own formats print `%` for a fraction and attach it for percentage points.
    if fraction && preferred.is_none_or(|f| f.ends_with('%')) {
        unit.push('%');
    }
    Representation {
        tick_format: Some(tick),
        scale,
        title_unit: (!unit.is_empty()).then_some(unit),
        cell_format: Some(cell),
        compact_format: Some(compact),
        symbol,
    }
}

/// `value` printed in full, as the grid shows it: the cell format, the currency symbol, and a
/// percent sign attached where the format does not print one.
///
/// A representation with no number cell format, a date's among them, prints d3-format's grouped
/// default.
#[must_use]
pub fn format_cell(rep: &Representation, value: f64) -> String {
    finish(rep, rep.cell_format.as_deref(), value)
}

/// `value` printed compactly, as the tooltip shows it: divided by the scale, with the currency
/// symbol, a percent sign attached, and the unit word after a no-break space (`£2.4 bn`, or
/// `(£0.3 bn)` for an accounting negative).
///
/// The scale is the column's, which the tile's bin applies, so a value far below the column's
/// magnitude is printed with the decimals it needs for two significant digits: £40m on a column
/// in billions reads `£0.04 bn`, not `£0.0 bn`.
#[must_use]
pub fn format_compact(rep: &Representation, value: f64) -> String {
    let Some(scale) = rep.scale else {
        return finish(rep, rep.compact_format.as_deref(), value);
    };
    let scaled = value / scale.divisor;
    let spec = rep.compact_format.as_deref().map(|spec| {
        let parts = Parts::of(spec);
        let comma = if parts.rest.contains(',') { "," } else { "" };
        let decimals = significant_decimals(scaled);
        Parts {
            rest: &format!("{comma}.{decimals}~f"),
            ..parts
        }
        .with_dollar(parts.dollar)
    });
    let mut text = finish(rep, spec.as_deref(), scaled);
    let closing = text.ends_with(')');
    if closing {
        text.pop();
    }
    text.push(NO_BREAK_SPACE);
    text.push_str(scale.unit);
    if closing {
        text.push(')');
    }
    text
}

/// The decimals `value` needs to print at least two significant digits, and at least one.
// The clamp keeps the float within `1..=12` before the cast.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn significant_decimals(value: f64) -> usize {
    if value == 0.0 || !value.is_finite() {
        return 1;
    }
    (1.0 - value.abs().log10().floor()).clamp(1.0, 12.0) as usize
}

/// `value` through `spec`, with the representation's symbol and percent sign.
///
/// The sign is attached only to a column of percentage points: its title says `%` and its format
/// does not print one. A fraction's format prints its own, and a preference that prints none
/// leaves `%` out of the title.
fn finish(rep: &Representation, spec: Option<&str>, value: f64) -> String {
    let format = spec
        .and_then(NumberFormat::parse)
        .or_else(|| NumberFormat::parse(","))
        .expect("`,` is a d3-format specifier");
    let mut text = format.format(value);
    if let Some(symbol) = &rep.symbol {
        text = with_symbol(&text, symbol);
    }
    let prints_percent = spec.is_some_and(|s| s.ends_with('%'));
    if rep.title_unit.as_deref() == Some("%") && !prints_percent {
        text.push('%');
    }
    text
}

/// `text` with `symbol` after its sign: `(£12.00)`, `−£12.00`, `£12.00`.
fn with_symbol(text: &str, symbol: &str) -> String {
    let sign_len = text
        .chars()
        .next()
        .filter(|c| matches!(*c, '(' | MINUS | '+' | '-' | ' '))
        .map_or(0, char::len_utf8);
    let (sign, digits) = text.split_at(sign_len);
    format!("{sign}{symbol}{digits}")
}

/// The column's least and greatest values, from its moments or the profile's rendering of them.
fn range_of(column: &ColumnProfile) -> Option<(f64, f64)> {
    if let Some(m) = &column.moments {
        return Some((m.min, m.max));
    }
    let min = column.min.as_deref()?.trim().parse::<f64>().ok()?;
    let max = column.max.as_deref()?.trim().parse::<f64>().ok()?;
    (min.is_finite() && max.is_finite()).then_some((min, max))
}

/// The largest of `scales` not above the column's magnitude, once the magnitude reaches `from`.
fn scale_of((min, max): (f64, f64), scales: &[Scale], from: f64) -> Option<Scale> {
    let magnitude = min.abs().max(max.abs());
    if magnitude < from {
        return None;
    }
    scales.iter().copied().find(|s| s.divisor <= magnitude)
}

/// The fewest decimals, up to [`MOST_PERCENT_DECIMALS`], at which [`SPREAD_TICKS`] evenly
/// spaced ticks across the range print distinctly as percentage points. `points` is what one unit
/// of the stored value is in percentage points: 100 for a fraction, 1 for a column that holds
/// percentage points already. A constant column gets the decimals its one value needs.
fn spread_decimals(range: Option<(f64, f64)>, points: f64) -> usize {
    let Some((min, max)) = range else {
        return 0;
    };
    let (lo, hi) = (min * points, max * points);
    (0..=MOST_PERCENT_DECIMALS)
        .find(|&decimals| {
            let format =
                NumberFormat::parse(&format!(".{decimals}f")).expect("a fixed-point specifier");
            if hi > lo {
                let ticks: Vec<String> = (0..SPREAD_TICKS)
                    .map(|i| {
                        let at = f64::from(i) / f64::from(SPREAD_TICKS - 1);
                        format.format(lo + (hi - lo) * at)
                    })
                    .collect();
                ticks.windows(2).all(|pair| pair[0] != pair[1])
            } else {
                let shift = 10f64.powi(i32::try_from(decimals).unwrap_or(0));
                ((lo * shift).round() / shift - lo).abs() <= 1e-9 * lo.abs().max(1.0)
            }
        })
        .unwrap_or(MOST_PERCENT_DECIMALS)
}

/// A d3-format specifier split around its sign and its `$`, so the rule can set the one and
/// drop or add the other: `[[fill]align]`, `[sign]`, `[$]`, and the rest.
#[derive(Debug, Clone, Copy)]
struct Parts<'a> {
    align: &'a str,
    sign: Option<char>,
    dollar: bool,
    rest: &'a str,
}

impl<'a> Parts<'a> {
    fn of(spec: &'a str) -> Self {
        let is_align = |c: char| "<>=^".contains(c);
        let mut chars = spec.char_indices();
        let first = chars.next();
        let second = chars.next();
        let mut at = match (first, second) {
            (Some(_), Some((i, c))) if is_align(c) => i + c.len_utf8(),
            (Some((_, c)), _) if is_align(c) => c.len_utf8(),
            _ => 0,
        };
        let align = &spec[..at];
        let sign = spec[at..].chars().next().filter(|c| "-+( ".contains(*c));
        at += sign.map_or(0, char::len_utf8);
        let dollar = spec[at..].starts_with('$');
        at += usize::from(dollar);
        Self {
            align,
            sign,
            dollar,
            rest: &spec[at..],
        }
    }

    /// The specifier, with a `$` when `dollar`.
    fn with_dollar(self, dollar: bool) -> String {
        let mut spec = self.align.to_owned();
        spec.extend(self.sign);
        if dollar {
            spec.push('$');
        }
        spec.push_str(self.rest);
        spec
    }
}

#[cfg(test)]
mod tests {
    use brightfield_engine::SemanticType;
    use brightfield_protocol::display::FinetypeDisplays;
    use brightfield_spec::number_format::NumberFormat;

    use super::*;
    use crate::column_record::{Negatives, Prefer, Written};

    /// A profiled column of `type_name` whose values run from `min` to `max`, as the profile
    /// renders them.
    fn column(type_name: &str, min: f64, max: f64) -> ColumnProfile {
        ColumnProfile {
            name: "value".to_string(),
            type_name: type_name.to_string(),
            non_null: 100,
            nulls: 0,
            distinct: 100,
            min: Some(min.to_string()),
            max: Some(max.to_string()),
            semantic: SemanticType::NotAsked,
            moments: None,
        }
    }

    /// FineType's vendored display block for `label`.
    fn display(label: &str) -> Display {
        FinetypeDisplays::vendored()
            .get(label)
            .cloned()
            .unwrap_or_else(|| panic!("{label} has a display block"))
    }

    /// A record of a column the source wrote with `symbol`.
    fn written(symbol: &str) -> FieldRecord {
        FieldRecord {
            written: Written {
                currency_symbol: Some(symbol.to_string()),
                negatives: None,
            },
            ..FieldRecord::default()
        }
    }

    /// The text of the tick at `value` on an axis drawn with `rep`'s tick format from `start` to
    /// `stop` by `step`, as the renderer and d3-scale build it.
    fn tick(rep: &Representation, start: f64, stop: f64, step: f64, value: f64) -> String {
        let spec = rep.tick_format.as_deref().expect("a tick format");
        NumberFormat::parse(spec)
            .expect("the tick format parses")
            .tick_format(start, stop, step)
            .format(value)
    }

    fn formats(rep: &Representation) -> Vec<&str> {
        [&rep.tick_format, &rep.cell_format, &rep.compact_format]
            .into_iter()
            .filter_map(|f| f.as_deref())
            .collect()
    }

    #[test]
    fn sterling_billions() {
        let rep = representation_of(
            &column("DOUBLE", 2e8, 3.1e9),
            Some(&display("finance.currency.amount")),
            Some(&written("£")),
            true,
        );
        assert_eq!(
            rep.scale,
            Some(Scale {
                divisor: 1e9,
                unit: "bn"
            })
        );
        assert_eq!(rep.title_unit.as_deref(), Some("£bn"));
        assert!(!rep.tick_format.as_deref().unwrap_or("$").contains('$'));
        assert_eq!(rep.symbol.as_deref(), Some("£"));
        assert_eq!(format_compact(&rep, 2.412e9), "£2.4\u{a0}bn");
        assert_eq!(format_cell(&rep, 2_412_345_000.0), "£2,412,345,000.00");
        assert_eq!(format_compact(&rep, 4e7), "£0.04\u{a0}bn");
        assert_eq!(format_compact(&rep, -3e7), "\u{2212}£0.03\u{a0}bn");
        assert_eq!(format_compact(&rep, 0.0), "£0\u{a0}bn");
    }

    #[test]
    fn a_scaled_accounting_tooltip_keeps_its_unit_inside_the_parentheses() {
        let record = FieldRecord {
            written: Written {
                currency_symbol: Some("£".to_string()),
                negatives: Some(Negatives::Parentheses),
            },
            ..FieldRecord::default()
        };
        let rep = representation_of(
            &column("DOUBLE", -3e8, 2.1e9),
            Some(&display("finance.currency.amount")),
            Some(&record),
            true,
        );
        assert_eq!(format_compact(&rep, -3e8), "(£0.3\u{a0}bn)");
        assert_eq!(format_compact(&rep, 2.1e9), "£2.1\u{a0}bn");
    }

    #[test]
    fn a_label_that_fixes_its_currency_names_it_only_in_the_title() {
        let rep = representation_of(
            &column("DOUBLE", 2e8, 3.1e9),
            Some(&display("finance.currency.amount_apostrophe")),
            None,
            true,
        );
        assert_eq!(rep.title_unit.as_deref(), Some("CHF bn"));
        assert_eq!(rep.symbol, None);
        for format in formats(&rep) {
            assert!(!format.contains("CHF"), "{format:?} names the currency");
        }
        assert_eq!(format_compact(&rep, 2.412e9), "2.4\u{a0}bn");
        assert_eq!(format_cell(&rep, 12.0), "12.00");
        let unscaled = representation_of(
            &column("DOUBLE", 1.0, 900.0),
            Some(&display("finance.currency.amount_apostrophe")),
            None,
            true,
        );
        assert_eq!(unscaled.title_unit.as_deref(), Some("CHF"));
        let recorded = representation_of(
            &column("DOUBLE", 1.0, 900.0),
            Some(&display("finance.currency.amount_apostrophe")),
            Some(&written("Fr.")),
            true,
        );
        assert_eq!(recorded.title_unit.as_deref(), Some("Fr."));
    }

    #[test]
    fn dollars_keep_their_symbol_on_ticks() {
        let rep = representation_of(
            &column("DOUBLE", 12.0, 980.0),
            Some(&display("finance.currency.amount")),
            Some(&written("$")),
            true,
        );
        assert!(rep
            .tick_format
            .as_deref()
            .is_some_and(|f| f.starts_with('$')));
        assert_eq!(rep.scale, None);
        assert_eq!(tick(&rep, 0.0, 1000.0, 200.0, 400.0), "$400");
        assert_eq!(rep.title_unit, None, "the ticks already say dollars");
        assert_eq!(format_cell(&rep, 1234.5), "$1,234.50");
    }

    #[test]
    fn accounting_negatives() {
        let record = FieldRecord {
            written: Written {
                currency_symbol: Some("£".to_string()),
                negatives: Some(Negatives::Parentheses),
            },
            ..FieldRecord::default()
        };
        let rep = representation_of(
            &column("DOUBLE", -40.0, 900.0),
            Some(&display("finance.currency.amount")),
            Some(&record),
            true,
        );
        assert_eq!(format_cell(&rep, -12.0), "(£12.00)");
        assert_eq!(format_cell(&rep, 12.0), "£12.00");
    }

    #[test]
    fn minus_negatives_override_an_accounting_label() {
        let record = FieldRecord {
            written: Written {
                currency_symbol: Some("£".to_string()),
                negatives: Some(Negatives::Minus),
            },
            ..FieldRecord::default()
        };
        let rep = representation_of(
            &column("DOUBLE", -40.0, 900.0),
            Some(&display("finance.currency.amount_accounting")),
            Some(&record),
            true,
        );
        assert_eq!(format_cell(&rep, -12.0), "\u{2212}£12.00");
    }

    #[test]
    fn negative_scaled_axis() {
        let rep = representation_of(
            &column("DOUBLE", -3e8, 2.1e9),
            Some(&display("finance.currency.amount")),
            Some(&written("£")),
            true,
        );
        assert_eq!(
            rep.scale,
            Some(Scale {
                divisor: 1e9,
                unit: "bn"
            })
        );
        assert_eq!(tick(&rep, -0.3, 2.1, 0.1, -0.3), "\u{2212}0.3");
        assert_eq!(tick(&rep, -0.3, 2.1, 0.1, 0.0), "0");
        assert_eq!(tick(&rep, -0.5, 2.0, 0.5, 0.0), "0");
        assert_eq!(tick(&rep, -0.5, 2.0, 0.5, 1.5), "1.5");
        assert_eq!(format_compact(&rep, -3e8), "\u{2212}£0.3\u{a0}bn");
    }

    #[test]
    fn percentage_precision_from_spread() {
        let percentage = display("representation.numeric.percentage");
        let wide = representation_of(&column("DOUBLE", 0.12, 0.78), Some(&percentage), None, true);
        assert_eq!(wide.tick_format.as_deref(), Some(".0%"));
        assert_eq!(wide.title_unit.as_deref(), Some("%"));
        assert_eq!(format_cell(&wide, 0.45), "45%");
        assert_eq!(format_compact(&wide, 0.45), "45%");
        let narrow = representation_of(
            &column("DOUBLE", 0.041, 0.046),
            Some(&percentage),
            None,
            true,
        );
        assert_eq!(narrow.tick_format.as_deref(), Some(".1%"));
        assert_eq!(format_cell(&narrow, 0.0442), "4.4%");
    }

    #[test]
    fn year_has_no_grouping() {
        let rep = representation_of(
            &column("BIGINT", 1990.0, 2024.0),
            Some(&display("datetime.component.year")),
            None,
            false,
        );
        assert_eq!(rep.tick_format.as_deref(), Some("d"));
        assert_eq!(rep.scale, None);
        assert_eq!(format_cell(&rep, 2024.0), "2024");
    }

    #[test]
    fn bytes_scale_binary() {
        let rep = representation_of(
            &column("BIGINT", 0.0, 5e9),
            Some(&display("representation.file.file_size")),
            None,
            false,
        );
        assert_eq!(
            rep.scale,
            Some(Scale {
                divisor: 1_073_741_824.0,
                unit: "GiB"
            })
        );
        assert_eq!(rep.title_unit.as_deref(), Some("GiB"));
        assert_eq!(format_compact(&rep, 2_684_354_560.0), "2.5\u{a0}GiB");
        assert_eq!(format_compact(&rep, 41_943_040.0), "0.039\u{a0}GiB");
        assert_eq!(format_cell(&rep, 5e9), "5,000,000,000");
    }

    #[test]
    fn bytes_below_a_mebibyte_are_not_scaled() {
        let rep = representation_of(
            &column("BIGINT", 0.0, 900_000.0),
            Some(&display("representation.file.file_size")),
            None,
            false,
        );
        assert_eq!(rep.scale, None);
    }

    #[test]
    fn money_below_ten_thousand_is_not_scaled() {
        let rep = representation_of(
            &column("DOUBLE", -9_999.0, 10.0),
            Some(&display("finance.currency.amount")),
            Some(&written("£")),
            true,
        );
        assert_eq!(rep.scale, None);
        assert_eq!(rep.title_unit.as_deref(), Some("£"));
        let thousands = representation_of(
            &column("DOUBLE", -10_000.0, 10.0),
            Some(&display("finance.currency.amount")),
            Some(&written("£")),
            true,
        );
        assert_eq!(
            thousands.scale,
            Some(Scale {
                divisor: 1e3,
                unit: "k"
            })
        );
    }

    #[test]
    fn preference_wins() {
        let record = FieldRecord {
            prefer: Prefer {
                number_format: Some(",.0f".to_string()),
                date_format: None,
            },
            ..written("£")
        };
        let rep = representation_of(
            &column("DOUBLE", 2e8, 3.1e9),
            Some(&display("finance.currency.amount")),
            Some(&record),
            true,
        );
        assert_eq!(rep.tick_format.as_deref(), Some(",.0f"));
        assert_eq!(rep.cell_format.as_deref(), Some(",.0f"));
        assert_eq!(rep.scale, None, "a preferred format is drawn as written");
        assert_eq!(format_cell(&rep, 2_412_345_000.4), "£2,412,345,000");
    }

    #[test]
    fn a_preferred_date_format_wins() {
        let record = FieldRecord {
            prefer: Prefer {
                number_format: None,
                date_format: Some("%d %b %Y".to_string()),
            },
            ..FieldRecord::default()
        };
        let mut date = column("DATE", 0.0, 0.0);
        date.min = Some("2024-01-01".to_string());
        date.max = Some("2024-12-31".to_string());
        let labelled = representation_of(&date, Some(&display("datetime.date.iso")), None, true);
        assert_eq!(labelled.tick_format.as_deref(), Some("%Y-%m-%d"));
        assert_eq!(labelled.cell_format.as_deref(), Some("%Y-%m-%d"));
        let preferred = representation_of(
            &date,
            Some(&display("datetime.date.iso")),
            Some(&record),
            true,
        );
        assert_eq!(preferred.tick_format.as_deref(), Some("%d %b %Y"));
        assert_eq!(preferred.cell_format.as_deref(), Some("%d %b %Y"));
    }

    #[test]
    fn unlabelled_column_has_no_representation() {
        let rep = representation_of(&column("BIGINT", 1.0, 1_234_567.0), None, None, false);
        assert_eq!(rep, Representation::default());
    }

    #[test]
    fn money_with_no_recorded_symbol_never_prints_dollars() {
        for record in [None, Some(FieldRecord::default())] {
            let rep = representation_of(
                &column("DOUBLE", 2e8, 3.1e9),
                Some(&display("finance.currency.amount")),
                record.as_ref(),
                true,
            );
            assert_eq!(rep.symbol, None);
            for format in formats(&rep) {
                assert!(!format.contains('$'), "{format:?} prints dollars");
            }
            assert_eq!(rep.title_unit.as_deref(), Some("bn"));
            assert_eq!(format_compact(&rep, 2.412e9), "2.4\u{a0}bn");
            assert_eq!(format_cell(&rep, 2_412_345_000.0), "2,412,345,000.00");
        }
    }

    #[test]
    fn a_symbol_other_than_dollars_stays_off_the_ticks() {
        for symbol in ["€", "¥", "CHF"] {
            let rep = representation_of(
                &column("DOUBLE", 1.0, 900.0),
                Some(&display("finance.currency.amount")),
                Some(&written(symbol)),
                true,
            );
            assert!(!rep.tick_format.as_deref().unwrap_or("$").contains('$'));
            assert_eq!(rep.title_unit.as_deref(), Some(symbol));
            assert_eq!(format_cell(&rep, 5.0), format!("{symbol}5.00"));
        }
    }

    #[test]
    fn uncast_percentage_is_not_multiplied() {
        let percentage = display("representation.numeric.percentage");
        let uncast = representation_of(
            &column("DOUBLE", 12.0, 78.0),
            Some(&percentage),
            None,
            false,
        );
        assert_eq!(uncast.tick_format.as_deref(), Some(".0f"));
        assert_eq!(uncast.title_unit.as_deref(), Some("%"));
        for format in formats(&uncast) {
            assert!(!format.ends_with('%'), "{format:?} multiplies by 100");
        }
        assert_eq!(format_cell(&uncast, 45.0), "45%");
        assert_eq!(format_compact(&uncast, 45.0), "45%");
        let narrow = representation_of(&column("DOUBLE", 4.1, 4.6), Some(&percentage), None, false);
        assert_eq!(narrow.tick_format.as_deref(), Some(".1f"));
        assert_eq!(format_cell(&narrow, 4.42), "4.4%");

        let cast = representation_of(&column("DOUBLE", 0.12, 0.78), Some(&percentage), None, true);
        assert_eq!(cast.cell_format.as_deref(), Some(".0%"));
        assert_eq!(format_cell(&cast, 0.45), "45%");
    }

    #[test]
    fn a_preference_on_a_fraction_attaches_no_percent_sign() {
        let prefer = |format: &str| FieldRecord {
            prefer: Prefer {
                number_format: Some(format.to_string()),
                date_format: None,
            },
            ..FieldRecord::default()
        };
        let percentage = display("representation.numeric.percentage");
        let plain = representation_of(
            &column("DOUBLE", 0.12, 0.78),
            Some(&percentage),
            Some(&prefer(",.2f")),
            true,
        );
        assert_eq!(plain.title_unit, None);
        assert_eq!(format_cell(&plain, 0.45), "0.45");
        assert_eq!(format_compact(&plain, 0.45), "0.45");
        let percent = representation_of(
            &column("DOUBLE", 0.12, 0.78),
            Some(&percentage),
            Some(&prefer(".1%")),
            true,
        );
        assert_eq!(percent.title_unit.as_deref(), Some("%"));
        assert_eq!(format_cell(&percent, 0.45), "45.0%");
        let points = representation_of(
            &column("DOUBLE", 12.0, 78.0),
            Some(&percentage),
            Some(&prefer(",.1f")),
            false,
        );
        assert_eq!(points.title_unit, None);
        assert_eq!(format_cell(&points, 45.0), "45.0");
    }

    #[test]
    fn a_text_column_under_a_fraction_label_stays_text() {
        let mut text = column("VARCHAR", 0.0, 0.0);
        text.min = None;
        text.max = None;
        for label in ["finance.rate.basis_points", "finance.rate.yield"] {
            let rep = representation_of(&text, Some(&display(label)), None, false);
            assert_eq!(rep, Representation::default(), "{label}");
        }
    }

    #[test]
    fn a_date_label_on_a_number_column_gives_no_format() {
        let rep = representation_of(
            &column("BIGINT", 1.0, 2.0),
            Some(&display("datetime.date.iso")),
            None,
            false,
        );
        assert_eq!(rep, Representation::default());
    }
}
