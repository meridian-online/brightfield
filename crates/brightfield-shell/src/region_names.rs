//! The name of each dense region on a two-column plot, read from what the region's rows share
//! and the rest of the view does not, or no name at all.
//!
//! The label query ([`brightfield_sql::regions::region_label_sql`]) summarises each candidate
//! column per region; this module decides which columns are candidates and turns the summary
//! into names. Columns are tried in the table's order, and the first that passes its test names
//! the region:
//!
//! - **Text**: the word or two-word phrase most typical of the region, by class-based TF-IDF
//!   with the regions (and the rest of the view) as the classes. It must be held by at least
//!   [`TEXT_MIN_SHARE`] of the region's rows.
//! - **Category**: the region's most common value, when at least [`CATEGORY_MIN_SHARE`] of its
//!   rows hold it and it is at least [`CATEGORY_MIN_LIFT`] times as common there as in the view.
//! - **Number**: the region's interquartile range, when it does not overlap the interquartile
//!   range of the rest of the view, written in the column's own format.
//!
//! A region no column passes for gets no name: an outline with no name is better than clutter.
//! Every choice here is deterministic, ties broken by a fixed key, because a committed baseline
//! pins the names.

use std::collections::BTreeMap;

use brightfield_engine::profile::VALUE_BAR_LIMIT;
use brightfield_engine::{ColumnProfile, RecordBatch};
use brightfield_render::axis::format_number;
use brightfield_sql::regions::label_kind;
pub use brightfield_sql::regions::{LabelColumn, LabelRole};
use duckdb::arrow::array::{Array, Float64Array, Int32Array, StringArray};

use crate::chart_kinds::is_temporal_type;
use crate::dashboard::{role_of, ColumnRole};
use crate::flat_cast::is_number_type;
use crate::representation::{format_compact, Representation};

/// The least share of a region's rows a term must appear in to name it. Half is the point past
/// which the word describes the region rather than some of it; a name true of a minority of the
/// outline would mislead.
pub const TEXT_MIN_SHARE: f64 = 0.5;

/// The least share of a region's rows its most common value must hold to name it. Higher than
/// the text figure because a category has no runner-up words to soften it: the name is the
/// value, so most of the region should be that value.
pub const CATEGORY_MIN_SHARE: f64 = 0.6;

/// How many times as common in the region as in the whole view a value must be to name it, so a
/// value common everywhere (a dataset's majority class) does not name every region the same.
pub const CATEGORY_MIN_LIFT: f64 = 2.0;

/// The least share of a region's rows that must hold a number for its range to name the region.
/// The quartiles skip nulls, so without this a range read off a few rows would stand for a
/// region whose other rows have no value at all.
pub const NUMBER_MIN_PRESENT: f64 = 0.5;

/// A column whose distinct count is at least this share of its non-null rows is an identifier
/// and never names a region. Not 1.0, because the profile's distinct count is an estimate
/// (`approx_count_distinct`) that reads a few percent either side; a column this close to
/// unique says one thing per row, which is no name for a region of many.
pub const IDENTIFIER_MIN_DISTINCT_SHARE: f64 = 0.95;

/// The thresholds the naming rules apply, so a test can change one and see the names move. The
/// default is this module's constants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NamingRules {
    pub text_min_share: f64,
    pub category_min_share: f64,
    pub category_min_lift: f64,
}

impl Default for NamingRules {
    fn default() -> Self {
        Self {
            text_min_share: TEXT_MIN_SHARE,
            category_min_share: CATEGORY_MIN_SHARE,
            category_min_lift: CATEGORY_MIN_LIFT,
        }
    }
}

/// The columns a region may be named after, in table order, with the role each is asked in.
///
/// The role comes from the column's storage type and measured distinct count first, so a
/// session with no FineType types its columns the way one with FineType does on the same data:
///
/// - a number is a [`LabelRole::Number`], unless it is an integer whose distinct count is near
///   its row count (an identifier). A float or decimal is always a number: measurements are
///   often all distinct, and an identifier is stored as an integer or text;
/// - a string with at most [`VALUE_BAR_LIMIT`] distinct values, the bound under which the
///   column header draws one bar per value, is a [`LabelRole::Category`], one with more is
///   [`LabelRole::Text`], and one whose distinct count is near its row count is an identifier;
/// - a boolean is a category;
/// - a date, a time or a timestamp is skipped, since a time range rarely names a place on the
///   plot, and so is any other type (a list, a struct, a blob).
///
/// A trusted FineType label only refines that: a date label, a coordinate, and a label that
/// identifies a thing or holds a document ([`ColumnRole::Neither`], free text aside) take the
/// column out. The axis columns `x` and `y` are never candidates.
#[must_use]
pub fn label_columns(columns: &[ColumnProfile], x: &str, y: &str) -> Vec<LabelColumn> {
    columns
        .iter()
        .filter(|c| c.name != x && c.name != y)
        .filter_map(|c| {
            let role = storage_role(c)?;
            refined_by_label(c).then(|| LabelColumn {
                name: c.name.clone(),
                role,
            })
        })
        .collect()
}

/// The role `column`'s storage type and distinct count give it, or `None` to skip it.
fn storage_role(column: &ColumnProfile) -> Option<LabelRole> {
    let base = column
        .type_name
        .split('(')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase();
    #[allow(clippy::cast_precision_loss)]
    let near_unique = column.non_null > 0
        && column.distinct as f64 >= IDENTIFIER_MIN_DISTINCT_SHARE * column.non_null as f64;
    if is_temporal_type(&column.type_name) {
        return None;
    }
    if is_number_type(&column.type_name) {
        let integer = !matches!(base.as_str(), "FLOAT" | "REAL" | "DOUBLE" | "DECIMAL");
        return (!(integer && near_unique)).then_some(LabelRole::Number);
    }
    match base.as_str() {
        "BOOLEAN" | "ENUM" => Some(LabelRole::Category),
        "VARCHAR" if near_unique => None,
        "VARCHAR" if column.distinct <= VALUE_BAR_LIMIT => Some(LabelRole::Category),
        "VARCHAR" => Some(LabelRole::Text),
        _ => None,
    }
}

/// Whether `column`'s FineType label, when it has a trusted one, leaves it a candidate.
fn refined_by_label(column: &ColumnProfile) -> bool {
    let Some((label, role)) = role_of(&column.semantic) else {
        return true;
    };
    if label.starts_with("datetime.") || label.starts_with("geography.coordinate.") {
        return false;
    }
    role != ColumnRole::Neither || label.starts_with("representation.text.")
}

/// One name per region, `0..regions`, read from `summary`, the batches the label query over
/// `columns` returned, by the default [`NamingRules`].
#[must_use]
pub fn name_regions(
    columns: &[LabelColumn],
    summary: &[RecordBatch],
    regions: usize,
) -> Vec<Option<String>> {
    name_regions_with(&NamingRules::default(), columns, summary, regions)
}

/// [`name_regions`] by `rules`, with every number written in its axis's default form.
#[must_use]
pub fn name_regions_with(
    rules: &NamingRules,
    columns: &[LabelColumn],
    summary: &[RecordBatch],
    regions: usize,
) -> Vec<Option<String>> {
    name_regions_shown(rules, columns, &[], summary, regions)
}

/// [`name_regions_with`], with `shown[i]` the representation of `columns[i]`, the one its axis,
/// grid and tooltip already use ([`crate::representation::representation_of`]). A number range
/// is written as the tooltip reads a value on that axis ([`format_compact`]); a column with no
/// entry in `shown`, or the default representation, is written as an unformatted axis writes its
/// ticks ([`format_number`]).
#[must_use]
pub fn name_regions_shown(
    rules: &NamingRules,
    columns: &[LabelColumn],
    shown: &[Representation],
    summary: &[RecordBatch],
    regions: usize,
) -> Vec<Option<String>> {
    let summary = Summary::read(summary, columns.len());
    let plain = Representation::default();
    (0..regions)
        .map(|region| {
            let region = i32::try_from(region).ok()?;
            let rows = summary.rows.get(&region).copied().filter(|&n| n > 0.0)?;
            columns.iter().enumerate().find_map(|(i, column)| {
                let facts = &summary.columns[i];
                let at = Region { region, rows };
                match column.role {
                    LabelRole::Text => name_by_term(rules, &summary, facts, at),
                    LabelRole::Category => name_by_value(rules, &summary, facts, at),
                    LabelRole::Number => {
                        let rep = shown.get(i).unwrap_or(&plain);
                        name_by_range(&column.name, rep, facts, at)
                    }
                }
            })
        })
        .collect()
}

/// A region's index and its row count.
#[derive(Clone, Copy)]
struct Region {
    region: i32,
    rows: f64,
}

/// What the label query found, by column.
#[derive(Default)]
struct Summary {
    /// Rows per region, `-1` (the rest of the view) among them.
    rows: BTreeMap<i32, f64>,
    /// One entry per label column, in order.
    columns: Vec<Facts>,
}

/// What the label query found about one column.
#[derive(Default)]
struct Facts {
    /// Rows holding each term or value, per region.
    counts: BTreeMap<i32, BTreeMap<String, f64>>,
    /// A number's quartiles and present count, per region and kind.
    numbers: BTreeMap<(i32, String), f64>,
}

impl Summary {
    /// The summary in `batches`. A batch not of the label query's shape is skipped, so it names
    /// nothing rather than something wrong.
    fn read(batches: &[RecordBatch], columns: usize) -> Self {
        let mut summary = Summary {
            columns: (0..columns).map(|_| Facts::default()).collect(),
            ..Summary::default()
        };
        for batch in batches {
            if batch.num_columns() < 5 {
                continue;
            }
            let (Some(col), Some(region), Some(kind), Some(key), Some(value)) = (
                batch.column(0).as_any().downcast_ref::<Int32Array>(),
                batch.column(1).as_any().downcast_ref::<Int32Array>(),
                batch.column(2).as_any().downcast_ref::<StringArray>(),
                batch.column(3).as_any().downcast_ref::<StringArray>(),
                batch.column(4).as_any().downcast_ref::<Float64Array>(),
            ) else {
                continue;
            };
            for row in 0..batch.num_rows() {
                if region.is_null(row) || kind.is_null(row) || value.is_null(row) {
                    continue;
                }
                let (r, kind, n) = (region.value(row), kind.value(row), value.value(row));
                if kind == label_kind::ROWS {
                    summary.rows.insert(r, n);
                    continue;
                }
                let Some(facts) = (!col.is_null(row))
                    .then(|| usize::try_from(col.value(row)).ok())
                    .flatten()
                    .and_then(|i| summary.columns.get_mut(i))
                else {
                    continue;
                };
                if kind == label_kind::TERM || kind == label_kind::VALUE {
                    if !key.is_null(row) {
                        let terms = facts.counts.entry(r).or_default();
                        terms.insert(key.value(row).to_owned(), n);
                    }
                } else {
                    facts.numbers.insert((r, kind.to_owned()), n);
                }
            }
        }
        summary
    }

    /// The rows in view, across every region and the rest of the view.
    fn view_rows(&self) -> f64 {
        self.rows.values().sum()
    }
}

impl Facts {
    /// How many rows of the whole view hold `key`.
    fn in_view(&self, key: &str) -> f64 {
        self.counts.values().filter_map(|m| m.get(key)).sum()
    }
}

/// The term most typical of the region, by class-based TF-IDF: its share of the region's rows
/// times `ln(1 + A / f)`, where `A` is the average rows per class (each region and the rest of
/// the view is a class) and `f` the rows of the whole view that hold it. A term shared widely
/// scores low however common it is in the region. Only terms in at least `text_min_share` of
/// the region's rows are scored, so the share rule is a gate and the score picks among the
/// terms that pass it. Ties go to the longer phrase, which says more, and then to the term
/// that sorts first.
fn name_by_term(
    rules: &NamingRules,
    summary: &Summary,
    facts: &Facts,
    at: Region,
) -> Option<String> {
    let classes = summary.rows.values().filter(|&&n| n > 0.0).count();
    #[allow(clippy::cast_precision_loss)]
    let average = summary.view_rows() / classes.max(1) as f64;
    let mut best: Option<(f64, usize, &str)> = None;
    for (term, &n) in facts.counts.get(&at.region)? {
        let share = n / at.rows;
        if share < rules.text_min_share {
            continue;
        }
        let score = share * (1.0 + average / facts.in_view(term)).ln();
        let words = term.split(' ').count();
        let better = best.is_none_or(|(s, w, _)| {
            score.total_cmp(&s).then(words.cmp(&w)) == std::cmp::Ordering::Greater
        });
        if better {
            best = Some((score, words, term));
        }
    }
    best.map(|(_, _, term)| term.to_owned())
}

/// The region's most common value (ties to the value that sorts first), when it is held by at
/// least `category_min_share` of the region's rows and is at least `category_min_lift` times as
/// common there as in the view.
fn name_by_value(
    rules: &NamingRules,
    summary: &Summary,
    facts: &Facts,
    at: Region,
) -> Option<String> {
    let (value, &n) = facts
        .counts
        .get(&at.region)?
        .iter()
        .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(a.0)))?;
    let share = n / at.rows;
    let view_share = facts.in_view(value) / summary.view_rows();
    (share >= rules.category_min_share && share >= rules.category_min_lift * view_share)
        .then(|| value.clone())
}

/// `<column> <lo>–<hi>`, the region's interquartile range, when at least
/// [`NUMBER_MIN_PRESENT`] of its rows hold a number and the range does not overlap (or touch)
/// the interquartile range of the rest of the view.
fn name_by_range(name: &str, rep: &Representation, facts: &Facts, at: Region) -> Option<String> {
    let get = |kind: &str| facts.numbers.get(&(at.region, kind.to_owned())).copied();
    let (q1, q3) = (get(label_kind::Q1)?, get(label_kind::Q3)?);
    let (rest_q1, rest_q3) = (get(label_kind::REST_Q1)?, get(label_kind::REST_Q3)?);
    let present = get(label_kind::PRESENT)? / at.rows;
    let apart = q3 < rest_q1 || q1 > rest_q3;
    (present >= NUMBER_MIN_PRESENT && apart).then(|| format!("{name} {}", range_text(rep, q1, q3)))
}

/// `lo–hi` (an en dash), each end rounded to two significant figures of the larger one, so a
/// range reads as neatly as an axis's ticks (`300,000–450,000`, not `312,456.3–447,812.8`), and
/// printed as `rep` prints a value. One value when both ends print the same.
fn range_text(rep: &Representation, lo: f64, hi: f64) -> String {
    let largest = lo.abs().max(hi.abs());
    let (lo, hi) = if largest > 0.0 && largest.is_finite() {
        let step = 10_f64.powf(largest.log10().floor() - 1.0);
        ((lo / step).round() * step, (hi / step).round() * step)
    } else {
        (lo, hi)
    };
    let print = |v: f64| {
        if *rep == Representation::default() {
            format_number(v)
        } else {
            format_compact(rep, v)
        }
    };
    let (lo, hi) = (print(lo), print(hi));
    if lo == hi {
        lo
    } else {
        format!("{lo}\u{2013}{hi}")
    }
}

#[cfg(test)]
mod tests {
    use brightfield_engine::{SemanticType, ValueCheck};
    use brightfield_protocol::display::FinetypeDisplays;
    use brightfield_sql::regions::{region_label_sql, GridExtent};
    use duckdb::Connection;

    use super::*;
    use crate::column_record::{FieldRecord, Written};
    use crate::representation::representation_of;

    /// The grid every fixture's rows lie on: ten bins over `(0, 10)` on both axes.
    fn extent() -> GridExtent {
        GridExtent {
            x: (0.0, 10.0),
            y: (0.0, 10.0),
            bins: 10,
        }
    }

    /// Region 0 is every cell with `bx` in 0..3 and region 1 every cell with `bx` in 6..9, so a
    /// row at `x = 1.5` is in region 0, one at `x = 7.5` in region 1 and one at `x = 4.5` in
    /// neither.
    fn banded_cells() -> Vec<(u32, u32)> {
        (0..10)
            .flat_map(|by| {
                (0..3)
                    .map(move |bx| (by * 10 + bx, 0))
                    .chain((6..9).map(move |bx| (by * 10 + bx, 1)))
            })
            .collect()
    }

    /// The rows of `table` (which has `x` and `y`) summarised by the label query.
    fn summarise(table: &str, columns: &[LabelColumn]) -> Vec<RecordBatch> {
        let c = Connection::open_in_memory().expect("duckdb");
        c.execute_batch(table).expect("fixture table");
        let sql = region_label_sql(
            "SELECT * FROM t",
            "x",
            "y",
            &extent(),
            &banded_cells(),
            columns,
        )
        .expect("usable extent");
        let mut stmt = c.prepare(&sql).expect("label sql prepares");
        stmt.query_arrow([]).expect("label sql runs").collect()
    }

    fn names(table: &str, columns: &[LabelColumn]) -> Vec<Option<String>> {
        name_regions(columns, &summarise(table, columns), 2)
    }

    fn column(name: &str, role: LabelRole) -> LabelColumn {
        LabelColumn {
            name: name.into(),
            role,
        }
    }

    /// A table of `x`, `y` and one VARCHAR column `c`, from `(x, value)` pairs; `None` is NULL.
    fn text_table(rows: &[(f64, Option<&str>)]) -> String {
        let values: Vec<String> = rows
            .iter()
            .enumerate()
            .map(|(i, (x, v))| {
                let v = v.map_or("NULL".to_owned(), |v| {
                    format!("'{}'", v.replace('\'', "''"))
                });
                format!("({x}, {}, {v})", 0.5 + (i % 9) as f64)
            })
            .collect();
        format!(
            "CREATE TABLE t (x DOUBLE, y DOUBLE, c VARCHAR); INSERT INTO t VALUES {};",
            values.join(", ")
        )
    }

    /// A table of `x`, `y` and one DOUBLE column `c`.
    fn number_table(name: &str, rows: &[(f64, Option<f64>)]) -> String {
        let values: Vec<String> = rows
            .iter()
            .enumerate()
            .map(|(i, (x, v))| {
                let v = v.map_or("NULL".to_owned(), |v| format!("{v:?}"));
                format!("({x}, {}, {v})", 0.5 + (i % 9) as f64)
            })
            .collect();
        format!(
            "CREATE TABLE t (x DOUBLE, y DOUBLE, \"{name}\" DOUBLE); INSERT INTO t VALUES {};",
            values.join(", ")
        )
    }

    const LEFT: f64 = 1.5;
    const RIGHT: f64 = 7.5;
    const BETWEEN: f64 = 4.5;

    fn places() -> String {
        text_table(&[
            (LEFT, Some("45 km SW of Severo-Kuril'sk, Russia")),
            (LEFT, Some("120 km E of Petropavlovsk-Kamchatsky, Russia")),
            (LEFT, Some("88 km SSE of Ust'-Kamchatsk Staryy, Russia")),
            (LEFT, Some("Komandorskiye Ostrova, Russia")),
            (RIGHT, Some("100 km NNE of Neiafu, Tonga")),
            (RIGHT, Some("65 km WSW of Pangai, Tonga")),
            (RIGHT, Some("Tonga Islands region")),
            (RIGHT, Some("south of the Fiji Islands")),
            (RIGHT, Some("12 km S of Hihifo, TONGA")),
            (BETWEEN, Some("Fiji region")),
            (BETWEEN, Some("Kermadec Islands, New Zealand")),
        ])
    }

    #[test]
    fn a_shared_place_word_names_its_region() {
        assert_eq!(
            names(&places(), &[column("c", LabelRole::Text)]),
            [Some("Russia".to_owned()), Some("Tonga".to_owned())]
        );
    }

    fn species(left: &[&str], right: &[&str], between: &[&str]) -> String {
        let rows: Vec<(f64, Option<&str>)> = left
            .iter()
            .map(|v| (LEFT, Some(*v)))
            .chain(right.iter().map(|v| (RIGHT, Some(*v))))
            .chain(between.iter().map(|v| (BETWEEN, Some(*v))))
            .collect();
        text_table(&rows)
    }

    #[test]
    fn a_dominant_category_names_its_region() {
        // Gentoo is every row of region 0 and a third of the view. Adelie is 60% of region 1
        // but over half the view, so it does not stand out there.
        let table = species(
            &["Gentoo"; 5],
            &["Adelie", "Adelie", "Adelie", "Chinstrap", "Chinstrap"],
            &["Adelie"; 5],
        );
        assert_eq!(
            names(&table, &[column("c", LabelRole::Category)]),
            [Some("Gentoo".to_owned()), None]
        );
    }

    #[test]
    fn a_category_common_everywhere_names_nothing() {
        let mixed = ["Adelie", "Adelie", "Adelie", "Adelie", "Gentoo"];
        let table = species(&mixed, &mixed, &mixed);
        assert_eq!(
            names(&table, &[column("c", LabelRole::Category)]),
            [None, None]
        );
    }

    fn house_values() -> String {
        let rows: Vec<(f64, Option<f64>)> = [200e3, 300e3, 375e3, 450e3, 550e3]
            .map(|v| (LEFT, Some(v)))
            .into_iter()
            .chain([100e3, 150e3, 200e3, 250e3, 300e3].map(|v| (RIGHT, Some(v))))
            .chain([50e3, 100e3, 150e3, 200e3, 250e3].map(|v| (BETWEEN, Some(v))))
            .collect();
        number_table("median_house_value", &rows)
    }

    #[test]
    fn a_clean_range_names_a_numeric_region() {
        // Region 0's quartiles are 300k and 450k, above the rest of the view's upper quartile.
        // Region 1's sit inside the rest of the view's, so it gets no name.
        let columns = [column("median_house_value", LabelRole::Number)];
        let summary = summarise(&house_values(), &columns);
        assert_eq!(
            name_regions(&columns, &summary, 2),
            [
                Some("median_house_value 300,000\u{2013}450,000".to_owned()),
                None
            ],
            "an unformatted column reads as its axis's default ticks"
        );

        // A money column the source wrote in dollars is written as its axis and tooltip take it.
        let profile = ColumnProfile {
            name: "median_house_value".into(),
            type_name: "DOUBLE".into(),
            non_null: 15,
            nulls: 0,
            distinct: 12,
            min: Some("50000".into()),
            max: Some("550000".into()),
            semantic: SemanticType::NotAsked,
            moments: None,
        };
        let display = FinetypeDisplays::vendored()
            .get("finance.currency.amount")
            .cloned()
            .expect("a money display block");
        let record = FieldRecord {
            written: Written {
                currency_symbol: Some("$".into()),
                negatives: None,
            },
            ..FieldRecord::default()
        };
        let money = representation_of(&profile, Some(&display), Some(&record), true);
        assert_eq!(
            name_regions_shown(&NamingRules::default(), &columns, &[money], &summary, 2),
            [
                Some("median_house_value $300\u{a0}k\u{2013}$450\u{a0}k".to_owned()),
                None
            ]
        );
    }

    #[test]
    fn overlapping_ranges_name_nothing() {
        let spread = [10.0, 20.0, 30.0, 40.0, 50.0];
        let rows: Vec<(f64, Option<f64>)> = [LEFT, RIGHT, BETWEEN]
            .into_iter()
            .flat_map(|x| spread.map(|v| (x, Some(v))))
            .collect();
        assert_eq!(
            names(
                &number_table("depth", &rows),
                &[column("depth", LabelRole::Number)]
            ),
            [None, None]
        );
    }

    #[test]
    fn nulls_and_blank_text_never_name_a_region() {
        // In region 0, three of five rows are null or blank. What the other two share is under
        // every threshold once the empty rows are counted.
        let text = text_table(&[
            (LEFT, None),
            (LEFT, Some("")),
            (LEFT, Some("   ")),
            (LEFT, Some("Gentoo")),
            (LEFT, Some("Gentoo")),
            (RIGHT, Some("Adelie")),
            (BETWEEN, Some("Adelie")),
            (BETWEEN, Some("Adelie")),
        ]);
        for role in [LabelRole::Text, LabelRole::Category] {
            assert_eq!(names(&text, &[column("c", role)])[0], None, "{role:?}");
        }

        // A region whose rows are all blank or null has no value to name it by, though a blank
        // counted as a value would be in 80% of its rows and three times as common as in the
        // view.
        let mut rows = vec![(LEFT, Some(" ")); 4];
        rows.push((LEFT, None));
        rows.extend([(RIGHT, Some("Adelie")); 5]);
        rows.extend([(BETWEEN, Some("Adelie")); 10]);
        let blank = text_table(&rows);
        for role in [LabelRole::Text, LabelRole::Category] {
            assert_eq!(names(&blank, &[column("c", role)])[0], None, "{role:?}");
        }

        // A clean range read off two of five rows does not say what the region is.
        let numbers = number_table(
            "value",
            &[
                (LEFT, None),
                (LEFT, None),
                (LEFT, None),
                (LEFT, Some(900.0)),
                (LEFT, Some(950.0)),
                (RIGHT, Some(10.0)),
                (BETWEEN, Some(20.0)),
                (BETWEEN, Some(30.0)),
            ],
        );
        assert_eq!(
            names(&numbers, &[column("value", LabelRole::Number)])[0],
            None
        );
    }

    #[test]
    fn the_first_passing_column_in_table_order_wins() {
        let rows = [
            (LEFT, "Adelie", "Russia", "Gentoo"),
            (LEFT, "Adelie", "Kuril Islands, Russia", "Gentoo"),
            (LEFT, "Adelie", "near Russia", "Gentoo"),
            (RIGHT, "Adelie", "Tonga", "Chinstrap"),
            (BETWEEN, "Adelie", "Fiji", "Chinstrap"),
            (BETWEEN, "Adelie", "Fiji", "Chinstrap"),
        ];
        let values: Vec<String> = rows
            .iter()
            .enumerate()
            .map(|(i, (x, a, b, c))| format!("({x}, {}, '{a}', '{b}', '{c}')", 0.5 + i as f64))
            .collect();
        let table = format!(
            "CREATE TABLE t (x DOUBLE, y DOUBLE, everywhere VARCHAR, place VARCHAR, \
             species VARCHAR); INSERT INTO t VALUES {};",
            values.join(", ")
        );
        let everywhere = column("everywhere", LabelRole::Category);
        let place = column("place", LabelRole::Text);
        let species = column("species", LabelRole::Category);

        // The first column names nothing (it is everywhere), so the next one that passes does.
        let first = names(
            &table,
            &[everywhere.clone(), place.clone(), species.clone()],
        );
        assert_eq!(first[0].as_deref(), Some("Russia"));
        let second = names(&table, &[everywhere, species, place]);
        assert_eq!(second[0].as_deref(), Some("Gentoo"));
    }

    fn profile(name: &str, type_name: &str, rows: u64, distinct: u64) -> ColumnProfile {
        ColumnProfile {
            name: name.into(),
            type_name: type_name.into(),
            non_null: rows,
            nulls: 0,
            distinct,
            min: None,
            max: None,
            semantic: SemanticType::NotAsked,
            moments: None,
        }
    }

    fn labelled(mut profile: ColumnProfile, label: &str) -> ColumnProfile {
        profile.semantic = SemanticType::Labelled {
            label: label.into(),
            confidence: 0.9,
            check: ValueCheck::Given,
        };
        profile
    }

    #[test]
    fn identifiers_and_axis_columns_are_never_tried() {
        let rows = 8_502;
        let columns = [
            profile("event_id", "BIGINT", rows, rows),
            profile("time", "TIMESTAMP", rows, rows),
            profile("week", "DATE", rows, 52),
            profile("longitude", "DOUBLE", rows, 8_000),
            profile("latitude", "DOUBLE", rows, 8_000),
            profile("depth", "DOUBLE", rows, 8_400),
            profile("mag", "DOUBLE", rows, 40),
            profile("magType", "VARCHAR", rows, 9),
            profile("code", "VARCHAR", rows, rows - 10),
            profile("place", "VARCHAR", rows, 5_469),
            profile("reviewed", "BOOLEAN", rows, 2),
            labelled(
                profile("contact", "VARCHAR", rows, 300),
                "identity.person.email",
            ),
            labelled(
                profile("year", "BIGINT", rows, 3),
                "datetime.component.year",
            ),
            labelled(
                profile("station_lat", "DOUBLE", rows, 60),
                "geography.coordinate.latitude",
            ),
            labelled(
                profile("summary", "VARCHAR", rows, 30),
                "representation.text.plain_text",
            ),
            profile("payload", "STRUCT(a INTEGER)", rows, 10),
        ];
        assert_eq!(
            label_columns(&columns, "longitude", "latitude"),
            [
                column("depth", LabelRole::Number),
                column("mag", LabelRole::Number),
                column("magType", LabelRole::Category),
                column("place", LabelRole::Text),
                column("reviewed", LabelRole::Category),
                column("summary", LabelRole::Category),
            ]
        );
    }

    #[test]
    fn the_naming_rules_decide_the_names() {
        let columns = [column("c", LabelRole::Text)];
        let summary = summarise(&places(), &columns);
        let strict = NamingRules {
            text_min_share: 0.99,
            ..NamingRules::default()
        };
        // Russia is in every row of region 0; Tonga in four of five rows of region 1.
        assert_eq!(
            name_regions_with(&strict, &columns, &summary, 2),
            [Some("Russia".to_owned()), None]
        );
    }
}
