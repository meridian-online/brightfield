//! The cast a lone file's columns take when it is opened on the flat route.
//!
//! A column whose trusted label FineType's projection casts is read through the label's transform,
//! as a Protocol's silver table reads it. The cast is declared in the spec itself, as plain DuckDB
//! SQL over the file's view: the file is read under [`ROWS`] and [`SOURCE`] selects its columns
//! from it (`a_files_own_columns_survive_whatever_their_names`), a cast one through its transform and, beside the columns, its text as the file wrote it
//! under [`RAW_PREFIX`]. So any session loaded from the spec (a tile switch's rebuild, the written
//! spec opened again, another Mosaic tool) reads the columns as cast.
//!
//! Whether a cast stands is decided once, on open, from the values: the in-memory copy of the
//! casting source is made, one aggregate over it compares each cast column's non-null count with
//! its text's, and a cast that turned any value into NULL is not declared. What the aggregate read
//! from a currency column's text (the symbol its values share, how its negatives are written) is
//! kept in the file's column record.

use std::collections::BTreeSet;
use std::path::Path;

use brightfield_engine::{ColumnProfile, ScanTally};
use brightfield_protocol::silver::{cast_expr_over_text, FinetypeTransforms};
use brightfield_spec::ast::{DataSourceKind, Spec};

use crate::column_record::{self, Negatives, Written};
use crate::data_file::{source_spec_deriving, OpenOptions, RAW_PREFIX, ROWS, SOURCE};
use crate::pipeline::LiveDashboard;

/// The columns a lone file's source casts as it reads them, and the SQL each is cast by.
///
/// A column is a candidate when its label is trusted (its values do not contradict it), FineType's
/// own projection casts it ([`cast_expr_over_text`], which passes through a label FineType types as
/// VARCHAR, as it does twelve currency and rate labels whose transforms return numbers), and DuckDB
/// stored it as neither a number nor the type the label casts to. A number is never cast through
/// its text: DuckDB renders a large double as `1e+16`, which a currency transform reads as `116`,
/// and rounds `123.456` to `123.46` on the way to a DECIMAL(18,2), with no NULL to show for either.
#[derive(Debug, Default)]
pub(crate) struct CastPlan {
    /// Each candidate column, and the cast that reads it, over the column read as text.
    pub(crate) casts: Vec<(String, String)>,
}

impl CastPlan {
    /// The candidates among `columns`, in the file's order.
    pub(crate) fn of(columns: &[ColumnProfile], transforms: &FinetypeTransforms) -> Self {
        let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
        let casts = columns
            .iter()
            .filter_map(|c| {
                if !c.semantic.is_usable() || is_number_type(&c.type_name) {
                    return None;
                }
                let label = c.semantic.label()?;
                let target = transforms.duckdb_type(label)?;
                if base_type(&c.type_name) == base_type(target) {
                    return None;
                }
                // A file that already has a column under the raw name keeps its own column.
                if names.contains(&raw_name(&c.name).as_str()) {
                    return None;
                }
                let expr = cast_expr_over_text(&c.name, Some(label), transforms)?;
                Some((c.name.clone(), expr))
            })
            .collect();
        Self { casts }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.casts.is_empty()
    }
}

/// The name a cast column's text is kept under.
fn raw_name(column: &str) -> String {
    format!("{RAW_PREFIX}{column}")
}

/// `name` as a SQL identifier.
fn sql_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `value` as a YAML single-quoted scalar; see `yaml_quoted` in [`crate::data_file`].
fn yaml_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// A DuckDB type name without its parameters, so `DECIMAL(18,2)` is the `DECIMAL` a label casts
/// to.
fn base_type(type_name: &str) -> String {
    type_name
        .split('(')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase()
}

/// Whether DuckDB stored a column of `type_name` as a number: an integer, a float or a decimal.
fn is_number_type(type_name: &str) -> bool {
    matches!(
        base_type(type_name).as_str(),
        "TINYINT"
            | "SMALLINT"
            | "INTEGER"
            | "BIGINT"
            | "HUGEINT"
            | "UTINYINT"
            | "USMALLINT"
            | "UINTEGER"
            | "UBIGINT"
            | "UHUGEINT"
            | "FLOAT"
            | "REAL"
            | "DOUBLE"
            | "DECIMAL"
    )
}

/// The select that reads [`ROWS`] with `casts` applied: the file's columns (`SELECT *`) in its
/// own order, a cast one replaced by its cast, and each cast column's text after them under its raw
/// name.
fn cast_select(casts: &[(String, String)]) -> String {
    let replaced: Vec<String> = casts
        .iter()
        .map(|(column, expr)| format!("{expr} AS {}", sql_ident(column)))
        .collect();
    let raw: Vec<String> = casts
        .iter()
        .map(|(column, _)| format!("{} AS {}", sql_ident(column), sql_ident(&raw_name(column))))
        .collect();
    format!(
        "SELECT * REPLACE ({}), {} FROM {}",
        replaced.join(", "),
        raw.join(", "),
        sql_ident(ROWS)
    )
}

/// The root-less spec that declares `path` as a source with `casts` applied and `derived` columns
/// projected beside its own.
///
/// With no casts this is [`source_spec_deriving`]'s spec. With casts, the file is read
/// under [`ROWS`] and [`SOURCE`] is [`cast_select`] over it, with the derived columns projected
/// over the cast ones, so a column derived from a cast one sees it cast.
#[must_use]
pub(crate) fn source_spec_casting(
    path: &Path,
    casts: &[(String, String)],
    derived: &[String],
) -> String {
    if casts.is_empty() {
        return source_spec_deriving(path, derived);
    }
    let file = yaml_quoted(&path.to_string_lossy());
    let mut sql = cast_select(casts);
    if !derived.is_empty() {
        sql = format!("SELECT *, {} FROM ({sql})", derived.join(", "));
    }
    format!(
        "data:\n  {ROWS}:\n    file: {file}\n  {SOURCE}: {}\n",
        yaml_quoted(&sql)
    )
}

/// The raw columns the sources of `spec` declare: each name a source's SQL selects a column as
/// (`AS "…"`) that starts with [`RAW_PREFIX`], which is how [`cast_select`] keeps a cast
/// column's text. A file's own column of such a name is not selected under an alias, so it is not
/// among them.
#[must_use]
pub(crate) fn raw_columns_declared(spec: &Spec) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for source in spec.data.values() {
        let (DataSourceKind::Query(sql) | DataSourceKind::Shorthand(sql)) = &source.kind else {
            continue;
        };
        let marker = format!("AS \"{RAW_PREFIX}");
        let mut rest = sql.as_str();
        while let Some(at) = rest.find(&marker) {
            let name = &rest[at + 3..];
            if let Some((ident, after)) = quoted_ident(name) {
                out.insert(ident);
                rest = after;
            } else {
                rest = &rest[at + marker.len()..];
            }
        }
    }
    out
}

/// The double-quoted SQL identifier `text` starts with, unescaped, and the text after it.
fn quoted_ident(text: &str) -> Option<(String, &str)> {
    let body = text.strip_prefix('"')?;
    let mut ident = String::new();
    let mut chars = body.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c == '"' {
            if chars.peek().is_some_and(|(_, next)| *next == '"') {
                ident.push('"');
                chars.next();
            } else {
                return Some((ident, &body[at + 1..]));
            }
        } else {
            ident.push(c);
        }
    }
    None
}

/// A column that was cast, the label it was cast for, and how the file wrote it.
pub(crate) type CastWritten = (String, String, Written);

/// What casting a lone file's copy produced.
pub(crate) struct CastCopy {
    /// The session holding the copy, under the spec that casts every candidate.
    pub(crate) live: LiveDashboard,
    /// The file's columns, a cast that stands profiled as cast.
    pub(crate) columns: Vec<ColumnProfile>,
    /// The casts that stand, each a column and its cast: what the dashboard's spec declares.
    pub(crate) stood: Vec<(String, String)>,
    /// Each cast that did not stand, as `(column, raw column)`: the copy reads the raw text in the
    /// column's place ([`brightfield_engine::Session::reshape_copy`]).
    pub(crate) put_back: Vec<(String, String)>,
    /// The copy and the aggregates over it, when the open counts.
    pub(crate) tally: ScanTally,
    /// The copy's clock, milliseconds.
    pub(crate) ms: f64,
    /// Each cast that stands, its label and how its source wrote it.
    pub(crate) written: Vec<CastWritten>,
}

/// Read `path` once into memory through a source that casts each of `plan`'s candidates, and
/// decide which casts stand.
///
/// One aggregate over the copy decides each candidate (a cast that turned a value into NULL has
/// fewer non-null values than the text it was read from), profiles the cast columns, and reads
/// from a currency column's text the symbol its values share and how its negatives are written.
///
/// # Errors
///
/// Why the copy or the aggregate failed; the session is then the caller's to drop.
pub(crate) fn cast_copy(
    path: &Path,
    columns: &[ColumnProfile],
    plan: &CastPlan,
    options: &OpenOptions,
) -> Result<CastCopy, String> {
    let mut live = LiveDashboard::load_str(&source_spec_casting(path, &plan.casts, &[]), None)?;
    if options.count_scans {
        live.coordinator().session().begin_scan_tally();
    }
    let at = std::time::Instant::now();
    let copied = live.materialise_source(SOURCE, options.materialise_budget_bytes);
    let ms = at.elapsed().as_secs_f64() * 1000.0;
    let decided = copied.and_then(|()| decide_casts(&mut live, columns, plan));
    let tally = live.coordinator().session().take_scan_tally();
    let decided = decided?;
    Ok(CastCopy {
        live,
        columns: decided.columns,
        stood: decided.stood,
        put_back: decided.put_back,
        tally,
        ms,
        written: decided.written,
    })
}

/// What [`decide_casts`] found.
struct Decided {
    columns: Vec<ColumnProfile>,
    stood: Vec<(String, String)>,
    put_back: Vec<(String, String)>,
    written: Vec<CastWritten>,
}

/// Decide each of `plan`'s candidates in the copy `live` holds, from one aggregate over it.
fn decide_casts(
    live: &mut LiveDashboard,
    columns: &[ColumnProfile],
    plan: &CastPlan,
) -> Result<Decided, String> {
    let label_of = |column: &str| {
        columns
            .iter()
            .find(|c| c.name == column)
            .and_then(|c| c.semantic.label())
            .unwrap_or_default()
            .to_string()
    };
    // Per candidate: the raw text's non-null count, then, for a currency column, the symbol its
    // values share and how its negatives are written.
    let mut extra = Vec::new();
    let mut asked = Vec::new();
    for (column, _) in &plan.casts {
        let raw = sql_ident(&raw_name(column));
        extra.push(format!("count({raw})"));
        let currency = label_of(column).starts_with("finance.currency.");
        if currency {
            let text = format!("CAST({raw} AS VARCHAR)");
            let symbol = format!("nullif(regexp_extract({text}, '\\p{{Sc}}', 0), '')");
            extra.push(format!(
                "CASE WHEN count(DISTINCT {symbol}) = 1 AND count({symbol}) = count({raw}) \
                 THEN min({symbol}) END"
            ));
            extra.push(format!(
                "CASE WHEN count_if(trim({text}) LIKE '(%)') > 0 THEN 'parentheses' \
                 WHEN count_if(contains({text}, '-')) > 0 THEN 'minus' END"
            ));
        }
        asked.push(currency);
    }
    let names: Vec<String> = plan.casts.iter().map(|(c, _)| c.clone()).collect();
    let (profiled, answers) = live
        .coordinator()
        .session()
        .profile_columns_with(SOURCE, &names, &extra)?;
    let mut answers = answers.into_iter();
    let mut decided = Decided {
        columns: columns.to_vec(),
        stood: Vec::new(),
        put_back: Vec::new(),
        written: Vec::new(),
    };
    for ((column, expr), currency) in plan.casts.iter().zip(asked) {
        let raw_count: Option<u64> = answers.next().flatten().and_then(|n| n.parse().ok());
        let (symbol, negatives) = if currency {
            (answers.next().flatten(), answers.next().flatten())
        } else {
            (None, None)
        };
        let cast = profiled.iter().find(|p| &p.name == column);
        let stands = matches!((cast, raw_count), (Some(p), Some(n)) if p.non_null == n);
        let (Some(cast), true, Some(at)) = (
            cast,
            stands,
            decided.columns.iter().position(|c| &c.name == column),
        ) else {
            decided.put_back.push((column.clone(), raw_name(column)));
            continue;
        };
        decided.columns[at] = ColumnProfile {
            semantic: decided.columns[at].semantic.clone(),
            ..cast.clone()
        };
        decided.stood.push((column.clone(), expr.clone()));
        decided.written.push((
            column.clone(),
            label_of(column),
            Written {
                currency_symbol: symbol,
                negatives: match negatives.as_deref() {
                    Some("parentheses") => Some(Negatives::Parentheses),
                    Some("minus") => Some(Negatives::Minus),
                    _ => None,
                },
            },
        ));
    }
    Ok(decided)
}

/// Bring the column record of the lone file at `path` up to date: the record is checked against
/// the file ([`column_record::refresh_if_changed`]), so a changed file loses what was read from
/// its old values; then each cast column takes its label (unless an analyst's accepted label is
/// already there) and how the file wrote it. Nothing is written for a file with no record that
/// cast nothing.
///
/// A record that cannot be read or written is reported and otherwise ignored: the open does not
/// depend on it.
pub(crate) fn keep_record(path: &Path, written: &[CastWritten]) {
    let kept = (|| {
        let package = column_record::flat_record_path(path)?;
        let existing = column_record::read_record(&package, SOURCE)?;
        if existing.is_none() && written.is_empty() {
            return Ok(());
        }
        let mut record = existing.unwrap_or_default();
        let before = record.source;
        column_record::refresh_if_changed(&mut record, path)?;
        if written.is_empty() && record.source == before {
            return Ok(());
        }
        for (column, label, how) in written {
            let field = record.fields.entry(column.clone()).or_default();
            if !field.nominated {
                field.label = Some(label.clone());
            }
            field.written = how.clone();
        }
        column_record::write_record(&package, SOURCE, &record)
    })();
    if let Err(e) = kept {
        eprintln!(
            "warning: {}: its column record was not brought up to date: {e}",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_columns_are_the_aliases_a_casting_source_declares() {
        let casts = vec![
            ("revenue".to_string(), "TRY(1)".to_string()),
            ("odd \"name\"".to_string(), "TRY(2)".to_string()),
        ];
        let text = source_spec_casting(Path::new("/tmp/x.csv"), &casts, &[]);
        let spec = brightfield_spec::parse_spec(&text, brightfield_spec::Format::Yaml)
            .expect("the source spec parses")
            .spec;
        let raw: Vec<String> = raw_columns_declared(&spec).into_iter().collect();
        assert_eq!(raw, ["__raw__odd \"name\"", "__raw__revenue"]);
        // A file read as it is declares none, whatever its columns are called.
        let plain = brightfield_spec::parse_spec(
            &source_spec_deriving(Path::new("/tmp/__raw__x.csv"), &[]),
            brightfield_spec::Format::Yaml,
        )
        .expect("the plain spec parses")
        .spec;
        assert!(raw_columns_declared(&plain).is_empty());
    }

    #[test]
    fn a_stored_number_is_not_a_candidate() {
        for ty in ["BIGINT", "DOUBLE", "DECIMAL(18,2)", "SMALLINT", "FLOAT"] {
            assert!(is_number_type(ty), "{ty}");
        }
        for ty in ["VARCHAR", "DATE", "TIMESTAMP"] {
            assert!(!is_number_type(ty), "{ty}");
        }
    }
}
