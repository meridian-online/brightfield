//! The steps arc generates, recognised from the model arc wrote, and the names
//! brightfield gives new ones.
//!
//! `arc operation record` writes a `filter-rows` or `sort-rows` step as a model
//! that opens with a `-- generated:` header and then follows a fixed template.
//! This module reads that template back, so a folder holding a data-file load
//! and steps arc generated opens as the chain it is. It is a parser of the
//! template and not a call to `arc sql recognise`, because opening must not
//! start the runner once per step, and because the open check must also match
//! the step's own name on the `CREATE` line, which that command does not see.
//! `parses_a_model_arc_actually_wrote` holds the parser to what arc writes.
//!
//! A model that carries the header but departs from the template (a hand edit
//! under the header, a different table in the `CREATE`) is refused with a
//! message naming the step: brightfield will not show a step as one effect
//! when its text says something else.
//!
//! **What opens this way**: a folder whose first step is a data-file load and
//! whose other steps are each a generated `filter-rows` or `sort-rows` on a
//! table an earlier step makes. It opens as a Protocol with its steps laid out
//! and starts no run; each generated step reads *not run* until a run's record
//! says otherwise, and its grid shows a labelled first-page preview (see
//! [`crate::data_grid::PREVIEW_LABEL`]). A folder with any other step after the
//! load is a hand-written Protocol, and is refused on open as it was before
//! generated steps opened, unless the offline variable is set.
//!
//! **Parsing a model yields the operation, the table it is on, and its
//! argument.** The open check reads all three, and the grid reads a
//! `sort-rows` step's order from it, because a table built with `ORDER BY`
//! keeps no order DuckDB promises on read.
//!
//! **New steps are named by their effect** ([`step_name_for`]):
//! `<table>_filtered_by_<column>` or `<table>_sorted_by_<column>`, where the
//! table is the one the data came from and the column is the one the condition
//! names, found by DuckDB's parse rather than by the cell the cursor started
//! on. A condition naming no column or several falls back to
//! `<table>_filtered`. Both parts are slugged, so the name passes arc's
//! step-name check, and a name already taken gets `_2`, `_3` and so on before
//! arc is asked, so a recording is not refused on its name.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::one_step::data_file_of_load;

/// The line arc opens a generated model with.
const HEADER: &str = "-- generated:";

/// The operations whose generated steps brightfield can read back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnownOperation {
    FilterRows,
    SortRows,
}

impl KnownOperation {
    /// The operation's long name, as arc prints it and writes it in the header.
    #[must_use]
    pub fn long_name(&self) -> &'static str {
        match self {
            Self::FilterRows => "filter-rows",
            Self::SortRows => "sort-rows",
        }
    }

    /// The word a step's name carries for what it does.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            Self::FilterRows => "filtered",
            Self::SortRows => "sorted",
        }
    }

    /// The key of the one argument the operation takes.
    #[must_use]
    pub fn argument_key(&self) -> &'static str {
        match self {
            Self::FilterRows => "where",
            Self::SortRows => "order_by",
        }
    }

    /// The operation a header's long name says, when brightfield reads it.
    #[must_use]
    pub fn from_long_name(name: &str) -> Option<Self> {
        match name {
            "filter-rows" => Some(Self::FilterRows),
            "sort-rows" => Some(Self::SortRows),
            _ => None,
        }
    }

    /// The clause keyword arc writes before the argument.
    fn clause(self) -> &'static str {
        match self {
            Self::FilterRows => "WHERE ",
            Self::SortRows => "ORDER BY ",
        }
    }
}

/// One step arc generated, as its model says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedStep {
    /// The step's name, which is also the table it makes.
    pub name: String,
    pub operation: KnownOperation,
    /// The table the step reads.
    pub on: String,
    /// The condition or the ordering, as written.
    pub argument: String,
}

/// A folder holding a data-file load and then steps arc generated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedProtocol {
    /// The data file the load reads.
    pub data: PathBuf,
    pub dir: PathBuf,
    /// The table the load makes.
    pub root_table: String,
    /// The generated steps, in manifest order.
    pub steps: Vec<GeneratedStep>,
}

/// `name` as arc writes a quoted identifier.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// A table as arc's `FROM` line spells it, bare or double-quoted, read back.
fn unspell_table(spelled: &str) -> Option<String> {
    match spelled.strip_prefix('"') {
        Some(rest) => {
            let inner = rest.strip_suffix('"')?;
            let mut out = String::new();
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c == '"' && chars.next() != Some('"') {
                    return None;
                }
                out.push(c);
            }
            Some(out)
        }
        None => {
            let plain = !spelled.is_empty()
                && !spelled.starts_with(|c: char| c.is_ascii_digit())
                && spelled
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_');
            plain.then(|| spelled.to_string())
        }
    }
}

/// Split a table spelling off the front of `text`: a double-quoted name up to
/// its closing quote (a doubled quote inside is part of the name), or a bare
/// name up to the first space. Gives the spelling and what follows it.
fn split_spelling(text: &str) -> Option<(&str, &str)> {
    if !text.starts_with('"') {
        return Some(text.split_at(text.find(' ').unwrap_or(text.len())));
    }
    let bytes = text.as_bytes();
    let mut i = 1;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if bytes.get(i + 1) == Some(&b'"') {
                i += 2;
                continue;
            }
            return Some(text.split_at(i + 1));
        }
        i += 1;
    }
    None
}

/// Read the model arc wrote for the step `step_name`.
///
/// # Errors
///
/// A message naming the step and the first point where the text departs from
/// arc's template.
pub fn parse_generated(step_name: &str, model_sql: &str) -> Result<GeneratedStep, String> {
    let refuse = |what: &str| {
        Err(format!(
            "step {step_name:?} carries the generated header but {what}"
        ))
    };
    let model_sql = model_sql.replace("\r\n", "\n");
    let text = model_sql.strip_suffix('\n').unwrap_or(&model_sql);
    let mut lines = text.splitn(5, '\n');
    let header = lines.next().unwrap_or("");
    let Some(said) = header.strip_prefix(HEADER) else {
        return Err(format!(
            "step {step_name:?} is not a generated step: its model does not open with `{HEADER}`"
        ));
    };
    let Some((long, on)) = said.trim_start().split_once(" on ") else {
        return refuse("its header does not say `<operation> on <table>`");
    };
    let Some(operation) = KnownOperation::from_long_name(long) else {
        return refuse(&format!("{long:?} is not an operation brightfield reads"));
    };
    let want_create = format!("CREATE OR REPLACE TABLE {} AS", quoted(step_name));
    if lines.next() != Some(want_create.as_str()) {
        return refuse(&format!("its second line is not `{want_create}`"));
    }
    if lines.next() != Some("SELECT *") {
        return refuse("its third line is not `SELECT *`");
    }
    let from = lines.next().unwrap_or("");
    let Some(spelled) = from.strip_prefix("FROM ") else {
        return refuse("its fourth line is not `FROM <table>`");
    };
    if unspell_table(spelled).as_deref() != Some(on) {
        return refuse(&format!("it reads {spelled} where its header says {on:?}"));
    }
    let rest = lines.next().unwrap_or("");
    let Some(tail) = rest.strip_prefix(operation.clause()) else {
        return refuse(&format!(
            "its clause does not open with `{}`",
            operation.clause().trim_end()
        ));
    };
    let Some(argument) = tail.trim_end().strip_suffix(';') else {
        return refuse("its statement does not end with `;`");
    };
    if argument.trim().is_empty() {
        return refuse("its clause is empty");
    }
    // DuckDB ignores an empty statement after the last, so `x;;` and `x; -- n`
    // count as one on their own. Each argument is therefore probed in the form
    // the engine later reads it in: a condition inside `(\n…\n)`, as the
    // engine's filtered read wraps it, and an order followed by `\n, 1`, as the
    // window read follows it with its tie-break columns. A `;` left in either
    // then cuts the probe short and fails to parse, and the probe adds no
    // operand an unfinished clause (`BETWEEN 1`) could take as its own.
    let statements = match operation {
        KnownOperation::FilterRows => {
            brightfield_engine::condition_statement_count(&format!("(\n{argument}\n)"))
        }
        KnownOperation::SortRows => {
            brightfield_engine::order_statement_count(&format!("{argument}\n, 1"))
        }
    };
    match statements {
        Ok(1) => {}
        Ok(n) => return refuse(&format!("its clause reads as {n} statements, not one")),
        Err(e) => return refuse(&format!("DuckDB cannot read its clause: {e}")),
    }
    // The probe above judges the argument in the engine's form; this judges
    // the statement the model is, as arc wrote it. A clause can parse as one
    // statement and still turn the model into something else, such as a
    // `UNION` that returns the table twice, or close the engine's parenthesis
    // and open another (`1) OR (1`), so the whole query must read as a plain
    // `SELECT *` from `on` with the one clause the operation writes and no
    // other, as
    // `a_clause_that_turns_the_query_into_something_else_is_refused` reads.
    let query = format!(
        "SELECT *\nFROM {spelled}\n{}{argument}\n",
        operation.clause()
    );
    let shape = match brightfield_engine::plain_select(&query) {
        Ok(shape) => shape,
        Err(e) => return refuse(&format!("its query is not a plain `SELECT *`: {e}")),
    };
    if shape.table != on {
        return refuse(&format!("its query reads {:?}, not {on:?}", shape.table));
    }
    let clause_only = match operation {
        KnownOperation::FilterRows => shape.has_where && shape.modifiers.is_empty(),
        KnownOperation::SortRows => !shape.has_where && shape.modifiers == ["ORDER_MODIFIER"],
    };
    if !clause_only {
        return refuse(&format!(
            "its query carries more than its `{}` clause",
            operation.clause().trim_end()
        ));
    }
    Ok(GeneratedStep {
        name: step_name.to_string(),
        operation,
        on: on.to_string(),
        argument: argument.to_string(),
    })
}

/// The table a load model makes: the target of its `CREATE OR REPLACE TABLE`.
fn load_table(model: &str) -> Option<String> {
    let rest = model
        .trim_start()
        .strip_prefix("CREATE OR REPLACE TABLE ")?;
    let (name, after) = split_spelling(rest)?;
    after.starts_with(" AS").then_some(())?;
    unspell_table(name)
}

/// Whether `model` opens with arc's generated header.
fn has_header(model: &str) -> bool {
    model
        .lines()
        .next()
        .is_some_and(|l| l.trim_start().starts_with(HEADER))
}

/// The folder at `dir`, whose manifest reads `manifest_text`, as a data-file
/// load followed by steps arc generated.
///
/// `Ok(None)` is a folder that is not that shape, so the caller goes on as it
/// did before: a manifest that is not a Protocol, fewer than two steps, no
/// generated header in any step after the first, or a first step that is not a
/// data-file load.
///
/// # Errors
///
/// A message naming the step, when the folder has a generated header and a step
/// does not match the template, or reads a table no earlier step makes.
pub fn generated_protocol_in(
    manifest_text: &str,
    dir: &Path,
) -> Result<Option<GeneratedProtocol>, String> {
    if !brightfield_protocol::is_protocol_manifest(manifest_text) {
        return Ok(None);
    }
    let Ok(manifest) = brightfield_protocol::parse_manifest_str(manifest_text) else {
        return Ok(None);
    };
    let Some((load, rest)) = manifest.steps.split_first() else {
        return Ok(None);
    };
    if rest.is_empty() {
        return Ok(None);
    }
    let models: Vec<Option<String>> = rest
        .iter()
        .map(|s| {
            s.sql
                .as_ref()
                .and_then(|p| std::fs::read_to_string(dir.join(p)).ok())
        })
        .collect();
    if !models.iter().flatten().any(|m| has_header(m)) {
        return Ok(None);
    }
    let Some(data) = data_file_of_load(load, dir) else {
        return Ok(None);
    };
    let root_table = load
        .sql
        .as_ref()
        .and_then(|p| std::fs::read_to_string(dir.join(p)).ok())
        .and_then(|m| load_table(&m))
        .ok_or_else(|| format!("step {:?} makes no table brightfield can name", load.name))?;
    let mut made: BTreeSet<String> = BTreeSet::from([root_table.clone()]);
    let mut steps = Vec::new();
    for (step, model) in rest.iter().zip(models) {
        let Some(model) = model else {
            return Err(format!(
                "step {:?} has no model file to read beside the generated steps",
                step.name
            ));
        };
        let parsed = parse_generated(&step.name, &model)?;
        if !made.contains(&parsed.on) {
            return Err(format!(
                "step {:?} reads {:?}, which no earlier step makes",
                step.name, parsed.on
            ));
        }
        made.insert(step.name.clone());
        steps.push(parsed);
    }
    Ok(Some(GeneratedProtocol {
        data,
        dir: dir.to_path_buf(),
        root_table,
        steps,
    }))
}

/// `text` as a slug: a character outside `[A-Za-z0-9_]` becomes `_`, a run of
/// `_` collapses to one, and a leading or trailing `_` is trimmed.
/// `a_column_arc_would_refuse_in_a_name_is_slugged` holds the cases.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let c = if c.is_ascii_alphanumeric() { c } else { '_' };
        if c != '_' || !out.ends_with('_') {
            out.push(c);
        }
    }
    out.trim_matches('_').to_string()
}

/// A name for a new step by its effect: `<root>_<word>_by_<column>` when a
/// filter's condition names exactly one column or a sort has any (its first),
/// `<root>_<word>` otherwise, and a
/// numeric suffix when the name is taken.
///
/// The column and the root are slugged first, so the name is one arc's step-name
/// check admits.
#[must_use]
pub fn step_name_for(
    operation: KnownOperation,
    root_table: &str,
    columns: &[String],
    taken: &BTreeSet<String>,
) -> String {
    let root = match slug(root_table) {
        s if s.is_empty() => "table".to_string(),
        s => s,
    };
    let named = match (operation, columns) {
        (KnownOperation::FilterRows, [one]) => Some(slug(one)),
        (KnownOperation::SortRows, [first, ..]) => Some(slug(first)),
        _ => None,
    };
    let base = match named {
        Some(col) if !col.is_empty() => format!("{root}_{}_by_{col}", operation.word()),
        _ => format!("{root}_{}", operation.word()),
    };
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|c| !taken.contains(c))
        .unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILTER: &str = "-- generated: filter-rows on california_housing\nCREATE OR REPLACE TABLE \"x\" AS\nSELECT *\nFROM california_housing\nWHERE house_age > 28;\n";

    #[test]
    fn parses_arcs_filter_template() {
        let s = parse_generated("x", FILTER).expect("arc's own text parses");
        assert_eq!(s.operation, KnownOperation::FilterRows);
        assert_eq!(s.on, "california_housing");
        assert_eq!(s.argument, "house_age > 28");
        assert_eq!(s.name, "x");
    }

    #[test]
    fn parses_arcs_sort_template() {
        let text = "-- generated: sort-rows on my table\nCREATE OR REPLACE TABLE \"y\" AS\nSELECT *\nFROM \"my table\"\nORDER BY a DESC,\n  b;\n";
        let s = parse_generated("y", text).expect("the sort template parses");
        assert_eq!(s.operation, KnownOperation::SortRows);
        assert_eq!(s.on, "my table");
        assert_eq!(s.argument, "a DESC,\n  b");
    }

    #[test]
    fn a_hand_edited_body_under_the_header_is_refused() {
        let edited = FILTER.replace("SELECT *", "SELECT house_age");
        let err = parse_generated("x", &edited).expect_err("a hand edit is refused");
        assert!(err.contains("\"x\""), "{err}");
    }

    #[test]
    fn a_quoted_name_that_is_not_the_steps_own_is_refused() {
        let err = parse_generated("other", FILTER).expect_err("the CREATE names x, not other");
        assert!(err.contains("\"other\""), "{err}");
    }

    #[test]
    fn an_unknown_operation_in_the_header_is_refused() {
        let text = FILTER.replace("filter-rows", "join-rows");
        let err = parse_generated("x", &text).expect_err("join-rows is not read");
        assert!(err.contains("join-rows"), "{err}");
    }

    fn taken(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    fn cols(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn names_carry_the_one_column_the_condition_names() {
        let name = step_name_for(
            KnownOperation::FilterRows,
            "california_housing",
            &cols(&["house_age"]),
            &taken(&[]),
        );
        assert_eq!(name, "california_housing_filtered_by_house_age");
    }

    #[test]
    fn names_fall_back_without_a_single_column() {
        for columns in [cols(&[]), cols(&["a", "b"])] {
            let name = step_name_for(
                KnownOperation::FilterRows,
                "california_housing",
                &columns,
                &taken(&[]),
            );
            assert_eq!(name, "california_housing_filtered");
        }
    }

    #[test]
    fn a_column_arc_would_refuse_in_a_name_is_slugged() {
        let by = |c: &str| {
            step_name_for(
                KnownOperation::FilterRows,
                "california_housing",
                &cols(&[c]),
                &taken(&[]),
            )
        };
        assert_eq!(by("#rooms"), "california_housing_filtered_by_rooms");
        assert_eq!(by(" price "), "california_housing_filtered_by_price");
        assert_eq!(by("a - b"), "california_housing_filtered_by_a_b");
        assert_eq!(by("###"), "california_housing_filtered");
        let rooted = step_name_for(
            KnownOperation::SortRows,
            "my data!",
            &cols(&[]),
            &taken(&[]),
        );
        assert_eq!(rooted, "my_data_sorted");
    }

    #[test]
    fn a_taken_name_gets_a_suffix() {
        let name = step_name_for(
            KnownOperation::SortRows,
            "x",
            &cols(&[]),
            &taken(&["x_sorted", "x_sorted_2"]),
        );
        assert_eq!(name, "x_sorted_3");
    }

    #[test]
    fn a_second_statement_smuggled_into_the_clause_is_refused() {
        let text = FILTER.replace("28;", "28;\nDELETE FROM california_housing;");
        let err = parse_generated("x", &text).expect_err("two statements are refused");
        assert!(err.contains("\"x\""), "{err}");
        for sneaky in [
            "x = E'\\''; DELETE FROM california_housing; SELECT 'x'",
            "x = $$'$$; DELETE FROM california_housing; SELECT 'x'",
        ] {
            let text = FILTER.replace("house_age > 28", sneaky);
            let err = parse_generated("x", &text).expect_err("a second statement is refused");
            assert!(err.contains("\"x\""), "{sneaky}: {err}");
        }
        let text = FILTER.replace("house_age > 28", "name = 'a;b' AND \"c;d\" > 1 /* ; */");
        let s = parse_generated("x", &text).expect("a `;` in a literal or comment is fine");
        assert_eq!(s.argument, "name = 'a;b' AND \"c;d\" > 1 /* ; */");
    }

    #[test]
    fn a_trailing_empty_statement_is_refused() {
        for tail in ["28;;", "28; -- note;"] {
            let text = FILTER.replace("28;", tail);
            let err = parse_generated("x", &text).expect_err("a trailing `;` is refused");
            assert!(err.contains("\"x\""), "{tail}: {err}");
        }
        let sort = "-- generated: sort-rows on t\nCREATE OR REPLACE TABLE \"y\" AS\nSELECT *\nFROM t\nORDER BY a;;\n";
        assert!(parse_generated("y", sort).is_err());
    }

    #[test]
    fn an_unfinished_between_is_refused() {
        for unfinished in [
            "house_age BETWEEN 1",
            "house_age NOT BETWEEN 1",
            "house_age BETWEEN SYMMETRIC 1",
        ] {
            let text = FILTER.replace("house_age > 28", unfinished);
            let err = parse_generated("x", &text).expect_err("an unfinished clause is refused");
            assert!(err.contains("\"x\""), "{unfinished}: {err}");
        }
    }

    #[test]
    fn a_condition_that_breaks_out_of_the_engines_parentheses_is_refused() {
        let text = FILTER.replace("house_age > 28", "1) OR (1");
        let err = parse_generated("x", &text).expect_err("an unbalanced condition is refused");
        assert!(err.contains("\"x\""), "{err}");
    }

    #[test]
    fn a_clause_that_turns_the_query_into_something_else_is_refused() {
        let text = FILTER.replace(
            "house_age > 28",
            "(SELECT true) UNION ALL SELECT * FROM california_housing",
        );
        let err = parse_generated("x", &text).expect_err("a UNION is refused");
        assert!(err.contains("\"x\""), "{err}");
        for order in [
            "a UNION SELECT * FROM t",
            "a LIMIT 5",
            "(SELECT 1) UNION ALL SELECT * FROM t ORDER BY a",
        ] {
            let text = format!(
                "-- generated: sort-rows on t\nCREATE OR REPLACE TABLE \"y\" AS\nSELECT *\nFROM t\nORDER BY {order};\n"
            );
            let err = parse_generated("y", &text).expect_err("more than an order is refused");
            assert!(err.contains("\"y\""), "{order}: {err}");
        }
        for condition in ["a > 1 GROUP BY ALL", "a > 1 LIMIT 5", "a > 1 QUALIFY true"] {
            let text = FILTER.replace("house_age > 28", condition);
            let err = parse_generated("x", &text).expect_err("more than a condition is refused");
            assert!(err.contains("\"x\""), "{condition}: {err}");
        }
    }

    #[test]
    fn a_clause_ending_in_a_line_comment_parses() {
        let text = FILTER.replace("house_age > 28", "house_age > 28 -- keep old");
        let s = parse_generated("x", &text).expect("a trailing line comment is fine");
        assert_eq!(s.argument, "house_age > 28 -- keep old");
        let sort = "-- generated: sort-rows on t\nCREATE OR REPLACE TABLE \"y\" AS\nSELECT *\nFROM t\nORDER BY a DESC -- newest first;\n";
        let s = parse_generated("y", sort).expect("a trailing line comment in a sort is fine");
        assert_eq!(s.argument, "a DESC -- newest first");
    }

    #[test]
    fn a_multi_line_sort_and_crlf_both_parse() {
        let text = "-- generated: sort-rows on t\r\nCREATE OR REPLACE TABLE \"y\" AS\r\nSELECT *\r\nFROM t\r\nORDER BY a DESC,\r\n  b;\r\n";
        let s = parse_generated("y", text).expect("CRLF is tolerated");
        assert_eq!(s.argument, "a DESC,\n  b");
    }

    #[test]
    fn a_bare_from_must_be_a_plain_identifier() {
        let text = FILTER
            .replace("on california_housing", "on my table")
            .replace("FROM california_housing", "FROM my table");
        assert!(parse_generated("x", &text).is_err());
    }

    #[test]
    fn a_load_table_name_is_split_at_its_closing_quote() {
        let got = load_table("CREATE OR REPLACE TABLE \"a AS b\" AS SELECT 1;\n");
        assert_eq!(got.as_deref(), Some("a AS b"));
        assert_eq!(
            load_table("CREATE OR REPLACE TABLE t AS SELECT 1;").as_deref(),
            Some("t")
        );
    }

    #[test]
    fn a_non_generated_step_is_not_called_a_generated_one() {
        let err = parse_generated("x", "SELECT 1;").expect_err("no header");
        assert!(err.contains("not a generated step"), "{err}");
    }

    #[test]
    fn a_sort_over_several_columns_names_the_first() {
        let name = step_name_for(
            KnownOperation::SortRows,
            "t",
            &cols(&["median_income", "house_age"]),
            &taken(&[]),
        );
        assert_eq!(name, "t_sorted_by_median_income");
        let empty_root = step_name_for(KnownOperation::SortRows, "!!", &cols(&["a"]), &taken(&[]));
        assert_eq!(empty_root, "table_sorted_by_a");
    }

    #[test]
    fn a_sort_names_its_first_column() {
        let name = step_name_for(
            KnownOperation::SortRows,
            "california_housing",
            &cols(&["median_income"]),
            &taken(&[]),
        );
        assert_eq!(name, "california_housing_sorted_by_median_income");
    }
}
