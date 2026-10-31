//! The cast a lone file's columns take when it is opened on the flat route.
//!
//! A column whose trusted label FineType's projection casts is read through the label's transform,
//! as a Protocol's silver table reads it. The cast is declared in the spec itself, as plain DuckDB
//! SQL over the file's view: the file is read under [`ROWS`], and [`SOURCE`] selects its columns
//! from it, a cast one through its transform, with each cast column's text as the file wrote it
//! after them under [`RAW_PREFIX`]. A file's own columns are all kept, whatever they are called
//! (`a_files_own_columns_survive_whatever_their_names`). So any session loaded from the spec (a
//! tile switch's rebuild, the written spec opened again, another Mosaic tool) reads the columns as
//! cast.
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
/// VARCHAR, as it does twelve currency and rate labels whose transforms return numbers) to a type a
/// format is chosen for ([`is_formatted_type`]), and DuckDB stored it as neither a number nor the
/// type the label casts to.
///
/// A label cast to any other type (JSON, UUID, INTERVAL, the coordinates struct) is read as it is
/// (`a_label_no_format_needs_is_read_as_it_is`). No format reads it, and its cast may not even
/// bind: DuckDB knows `JSON` as a type when its JSON extension loads, which the bundled build does
/// not carry. `TRY` catches a value that fails to cast, not a cast that fails to bind, and one such
/// cast fails the whole copy, which then casts no column.
///
/// A number is never cast through its text: DuckDB renders a large double as `1e+16`, which a
/// currency transform reads as `116`, and rounds `123.456` to `123.46` on the way to a
/// DECIMAL(18,2), with no NULL to show for either.
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
                if !is_formatted_type(target) || base_type(&c.type_name) == base_type(target) {
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
pub(crate) fn is_number_type(type_name: &str) -> bool {
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

/// Whether a cast to `type_name` gives values a number or date format is chosen for: a number, a
/// date, a time, a timestamp, or a boolean.
fn is_formatted_type(type_name: &str) -> bool {
    is_number_type(type_name)
        || matches!(
            base_type(type_name).as_str(),
            "DATE"
                | "TIME"
                | "TIMESTAMP"
                | "TIMESTAMP WITH TIME ZONE"
                | "TIMESTAMPTZ"
                | "TIMESTAMP_S"
                | "TIMESTAMP_MS"
                | "TIMESTAMP_NS"
                | "BOOLEAN"
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
        "data:\n  {ROWS}:\n    file: {file}\n  {SOURCE}: {}",
        yaml_block(&sql, 4)
    )
}

/// `text` as a YAML literal block scalar whose lines are indented by `indent` spaces, ending in a
/// line break, so every character of it, line breaks included, reads back as written.
///
/// A quoted scalar cannot carry a line break (YAML folds one to a space), and FineType's
/// transforms hold line breaks: `identity.person.height`'s is a multi-line `CASE`, which
/// `a_multi_line_transform_round_trips_through_the_written_spec` writes and reads back. The
/// chomping indicator keeps the text's own final line breaks: none (`-`) or all (`+`). The text
/// starts with `SELECT`, not a space, so its indentation needs no indicator.
fn yaml_block(text: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let chomp = if text.ends_with('\n') { '+' } else { '-' };
    let mut out = format!("|{chomp}\n");
    for line in text.trim_end_matches('\n').split('\n') {
        if line.is_empty() {
            out.push('\n');
        } else {
            out.push_str(&pad);
            out.push_str(line);
            out.push('\n');
        }
    }
    // The last line's own break is the first of the text's final breaks; `+` keeps the rest as
    // empty lines.
    let trailing = text.len() - text.trim_end_matches('\n').len();
    for _ in 1..trailing {
        out.push('\n');
    }
    out
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

/// The two aggregate terms that read how a currency column was written from `raw`, the SQL of its
/// text as the source wrote it: the symbol every non-null value carries, when they all carry the
/// same one, and `'parentheses'` when any negative is written in parentheses, else `'minus'` when
/// any value holds a minus sign. [`written_from`] reads their answers.
pub(crate) fn written_terms(raw: &str) -> [String; 2] {
    let text = format!("CAST({raw} AS VARCHAR)");
    let symbol = format!("nullif(regexp_extract({text}, '\\p{{Sc}}', 0), '')");
    [
        format!(
            "CASE WHEN count(DISTINCT {symbol}) = 1 AND count({symbol}) = count({raw}) \
             THEN min({symbol}) END"
        ),
        format!(
            "CASE WHEN count_if(trim({text}) LIKE '(%)') > 0 THEN 'parentheses' \
             WHEN count_if(contains({text}, '-')) > 0 THEN 'minus' END"
        ),
    ]
}

/// How a column was written, from the answers to its [`written_terms`].
pub(crate) fn written_from(symbol: Option<String>, negatives: Option<String>) -> Written {
    Written {
        currency_symbol: symbol,
        negatives: match negatives.as_deref() {
            Some("parentheses") => Some(Negatives::Parentheses),
            Some("minus") => Some(Negatives::Minus),
            _ => None,
        },
    }
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
            extra.extend(written_terms(&raw));
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
            written_from(symbol, negatives),
        ));
    }
    Ok(decided)
}

/// Bring the column record of the lone file at `path` up to date: the record is checked against
/// the file ([`column_record::refresh_if_changed`]), so a changed file loses what was read from
/// its old values, and the stamp takes the file's canonical path; then each cast column takes its
/// label (unless an analyst's accepted label is already there) and how the file wrote it. Nothing
/// is written for a file with no record that cast nothing.
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
        let before = record.source.clone();
        column_record::refresh_if_changed(&mut record, path)?;
        // The stamp names the file, so a record whose file has gone can be pruned.
        if let Some(stamp) = record.source.as_mut() {
            stamp.path = Some(
                path.canonicalize()
                    .map_err(|e| format!("{}: {e}", path.display()))?,
            );
        }
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
    fn a_block_scalar_reads_back_as_written() {
        for text in [
            "SELECT 1",
            "SELECT CASE\n  WHEN x THEN 'a''b'\nEND\n",
            "SELECT\n\n  1\n\n",
        ] {
            let yaml = format!("k: {}", yaml_block(text, 2));
            let back: std::collections::BTreeMap<String, String> =
                serde_yaml::from_str(&yaml).expect("the block parses");
            assert_eq!(back["k"], text, "{yaml}");
        }
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

    #[test]
    fn only_a_type_a_format_reads_is_cast_to() {
        for ty in [
            "BIGINT",
            "SMALLINT",
            "DOUBLE",
            "DECIMAL",
            "DECIMAL(18,2)",
            "DATE",
            "TIME",
            "TIMESTAMP",
            "TIMESTAMP WITH TIME ZONE",
            "BOOLEAN",
        ] {
            assert!(is_formatted_type(ty), "{ty}");
        }
        for ty in ["JSON", "UUID", "INTERVAL", "POINT", "VARCHAR"] {
            assert!(!is_formatted_type(ty), "{ty}");
        }
        // Every target the vendored transforms name is one or the other, so a new type in a
        // FineType release is decided here rather than cast by default.
        let transforms = FinetypeTransforms::vendored();
        for label in transforms.labels() {
            let ty = transforms.duckdb_type(label).unwrap_or_default();
            assert!(
                is_formatted_type(ty)
                    || ["JSON", "UUID", "INTERVAL", "POINT", "VARCHAR"].contains(&ty),
                "{label} casts to {ty}, which neither list names"
            );
        }
    }
}
