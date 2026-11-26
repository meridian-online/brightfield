//! `summarise-rows` in the window: the rows grouped by columns, then the
//! values each group holds, asked in a form of its own in the inspector's
//! Operation section.
//!
//! It starts from `shift-f` with the grid's cursor on a cell, or from
//! "Summarise by this column" on a header's menu, and begins with no question,
//! as VisiData's frequency table does: `group_by` is the column as SQL spells
//! it and `aggregates` is `count(*) AS n`, so the preview is a count of each of
//! the column's values at once, and Enter keeps it
//! (`summarise_rows_starts_from_a_cell_as_a_count_of_each_value`).
//!
//! The form has two parts. "Group by" lists the items grouped by, each a chip
//! that a click takes off, and *+ group* lists the table's columns not yet
//! grouped by, a category's or a date's first. "Values" lists the aggregates,
//! each with its function as a chip of its own where the item is a function of
//! one column, and *+ value* lists the columns to add, a number's first: `avg`
//! for a decimal, `sum` for an integer, and `count(DISTINCT …)` for anything
//! else, each named `<function>_<column>`. Under each part its list is the
//! select list arc records, typed into as text. arc refuses a constant grouped
//! by, and its reason stays on the status band while the last good rows fade.
//!
//! The step is named for the first item grouped by
//! (`<table>_summarised_by_<column>`), so adding a second group leaves the name
//! as it was.

use super::{
    Begin, By, Draft, FormCx, FormEvent, HeaderPart, Keeps, NameRule, Need, OperationUi, Stage,
    Start,
};
use crate::one_step::ColumnFacts;
use crate::protocol::ui_font;
use brightfield_workbench::chrome;
use meridian_design::{semantic, spacing};

/// The id of the form's field holding `group_by` as text.
pub const GROUP_BY_FIELD: &str = "summarise-rows-group-by";
/// The id of the form's field holding `aggregates` as text.
pub const AGGREGATES_FIELD: &str = "summarise-rows-aggregates";

/// The functions a value's chip offers: the call's head as SQL writes it, and
/// the word the value's name begins with.
const FUNCTIONS: [(&str, &str); 5] = [
    ("avg(", "avg"),
    ("sum(", "sum"),
    ("min(", "min"),
    ("max(", "max"),
    ("count(DISTINCT ", "count_distinct"),
];

/// [`FUNCTIONS`]' place for `count(DISTINCT …)`, the function of a column that
/// is not a number.
const DISTINCT: usize = 4;

/// The items of a select list: split at each comma outside parentheses and
/// quotes, each trimmed, with empty items dropped.
#[must_use]
pub fn items(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut current = String::new();
    for c in list.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => quote = Some(c),
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    out.push(std::mem::take(&mut current));
                    continue;
                }
                _ => {}
            },
        }
        current.push(c);
    }
    out.push(current);
    out.into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// A select list of `items`.
fn joined(items: &[String]) -> String {
    items.join(", ")
}

/// A name without the double quotes SQL spells it in, when it has them.
fn unquoted(name: &str) -> String {
    name.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .map_or_else(|| name.to_string(), |inner| inner.replace("\"\"", "\""))
}

/// The column of `columns` an item is, by its name, bare or in quotes.
fn column_of<'a>(item: &str, columns: &'a [String]) -> Option<&'a String> {
    let name = unquoted(item);
    columns.iter().find(|c| **c == name)
}

/// A name as SQL spells it: bare when it is ASCII letters, digits and `_` and
/// does not begin with a digit, and quoted otherwise.
fn spelled(name: &str) -> String {
    let plain = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if plain {
        name.to_string()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// The value `<function>(<column>) AS <function>_<column>`, with the column
/// written as `column_sql`.
fn value_item(function: usize, column: &str, column_sql: &str) -> String {
    let (head, word) = FUNCTIONS[function];
    format!(
        "{head}{column_sql}) AS {}",
        spelled(&format!("{word}_{column}"))
    )
}

/// The function a value of a column opens on: `avg` for a decimal, `sum` for
/// an integer and `count(DISTINCT …)` for anything else. Only a column stored
/// as a number is averaged or summed, since DuckDB does neither to text, which
/// FineType may still label a number (`45%`, `1,5`). For one, FineType's label
/// decides between the two, and a label that is not a number, such as a year,
/// counts it distinct; without a label, the storage type decides.
fn function_for(label: Option<&str>, storage: &str) -> usize {
    let upper = storage.trim().to_ascii_uppercase();
    let stored = match upper.split('(').next().unwrap_or_default().trim() {
        "FLOAT" | "REAL" | "DOUBLE" | "DECIMAL" | "NUMERIC" => 0,
        "TINYINT" | "SMALLINT" | "INTEGER" | "INT" | "BIGINT" | "HUGEINT" | "UTINYINT"
        | "USMALLINT" | "UINTEGER" | "UBIGINT" | "UHUGEINT" => 1,
        _ => return DISTINCT,
    };
    match label {
        None => stored,
        Some(label) if label.starts_with("representation.numeric.integer") => 1,
        Some(label) if label.starts_with("representation.numeric.") => 0,
        Some(_) => DISTINCT,
    }
}

/// [`function_for`] a column, or `count(DISTINCT …)` for one Brightfield has
/// no facts of.
fn function_of(facts: Option<&ColumnFacts>) -> usize {
    facts.map_or(DISTINCT, |f| function_for(f.label.as_deref(), &f.storage))
}

/// A value item read as a function of one column: the function's place in
/// [`FUNCTIONS`], the column as written, and whether the item's name is the one
/// [`value_item`] gives it. `None` for any other item, `count(*)` among them.
fn as_function(item: &str) -> Option<(usize, String, bool)> {
    let (call, alias) = match item.rsplit_once(" AS ") {
        Some((call, alias)) => (call.trim(), Some(alias.trim())),
        None => (item.trim(), None),
    };
    let lower = call.to_ascii_lowercase();
    let at = FUNCTIONS
        .iter()
        .position(|(head, _)| lower.starts_with(&head.to_ascii_lowercase()))?;
    let column = call.get(FUNCTIONS[at].0.len()..)?.strip_suffix(')')?.trim();
    // One column: no call, operator or second argument inside.
    let one = !column.is_empty()
        && (column.starts_with('"') && column.ends_with('"') && column.len() > 1
            || column.chars().all(|c| c.is_alphanumeric() || c == '_'));
    if !one {
        return None;
    }
    // A name the function gave, or that name made free with `_2`, `_3`.
    let given = format!("{}_{}", FUNCTIONS[at].1, unquoted(column));
    let named = alias.is_some_and(|alias| {
        let alias = unquoted(alias);
        alias == given
            || alias
                .strip_prefix(&format!("{given}_"))
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    });
    Some((at, column.to_string(), named))
}

/// `item` with its function changed to `function`: renamed for it where its
/// name was the old function's, and with its own name kept otherwise.
fn with_function(item: &str, function: usize) -> Option<String> {
    let (_, column, named) = as_function(item)?;
    let made = value_item(function, &unquoted(&column), &column);
    if named {
        return Some(made);
    }
    let call = made.rsplit_once(" AS ").map_or(made.as_str(), |(c, _)| c);
    Some(match item.rsplit_once(" AS ") {
        Some((_, alias)) => format!("{call} AS {}", alias.trim()),
        None => call.to_string(),
    })
}

/// The name of the column an item makes: its alias, or the item itself when
/// it is a column given bare or in quotes; without quotes either way. `None`
/// for an expression with no alias.
fn name_of(item: &str) -> Option<String> {
    if let Some((_, alias)) = item.rsplit_once(" AS ") {
        return Some(unquoted(alias.trim()));
    }
    let item = item.trim();
    let quoted = item.len() > 1 && item.starts_with('"') && item.ends_with('"');
    let bare = !item.is_empty() && item.chars().all(|c| c.is_alphanumeric() || c == '_');
    (quoted || bare).then(|| unquoted(item))
}

/// `item` under a name free of the columns the items `taken` make, compared
/// without case, with `_2`, `_3`.
fn free_alias(item: String, taken: &[String]) -> String {
    let Some((call, alias)) = item.rsplit_once(" AS ") else {
        return item;
    };
    // DuckDB reads names alike in all but case as one, quoted or not.
    let names: Vec<String> = taken
        .iter()
        .filter_map(|t| name_of(t))
        .map(|name| name.to_lowercase())
        .collect();
    let bare = unquoted(alias.trim());
    let taken = |name: &str| names.contains(&name.to_lowercase());
    if !taken(&bare) {
        return item;
    }
    (2..)
        .map(|n| format!("{bare}_{n}"))
        .find(|candidate| !taken(candidate))
        .map_or(item.clone(), |free| format!("{call} AS {}", spelled(&free)))
}

/// Whether a column reads as a category or a date: by its chart tile, or by
/// FineType's label.
fn groups_well(facts: &ColumnFacts) -> bool {
    matches!(
        facts.tile.as_deref(),
        Some("ranked-category-bars" | "counts-over-time" | "count-grid")
    ) || facts
        .label
        .as_deref()
        .is_some_and(|label| label.starts_with("datetime.") || label.contains("categor"))
}

/// What Brightfield knows of `column`.
fn facts<'a>(b: &'a Begin<'_>, column: &str) -> Option<&'a ColumnFacts> {
    b.facts.iter().find(|f| f.column == column)
}

/// The table's columns the draft does not group by.
fn ungrouped<'a>(b: &'a Begin<'_>, draft: &Draft) -> Vec<&'a String> {
    let grouped: Vec<&String> = items(draft.value("group_by").unwrap_or_default())
        .iter()
        .filter_map(|item| column_of(item, b.columns))
        .collect();
    b.columns.iter().filter(|c| !grouped.contains(c)).collect()
}

/// The columns *+ group* offers: those not grouped by, a category's or a
/// date's first, each part in the table's order.
#[must_use]
pub fn group_offers(b: &Begin<'_>, draft: &Draft) -> Vec<String> {
    let open = ungrouped(b, draft);
    let well = |c: &&&String| facts(b, c).is_some_and(groups_well);
    open.iter()
        .filter(well)
        .chain(open.iter().filter(|c| !well(c)))
        .map(|c| (*c).clone())
        .collect()
}

/// The values *+ value* offers, as the items they add: one per column not
/// grouped by, a number's first, each part in the table's order.
#[must_use]
pub fn value_offers(b: &Begin<'_>, draft: &Draft) -> Vec<String> {
    let open = ungrouped(b, draft);
    let number = |c: &&&String| function_of(facts(b, c)) != DISTINCT;
    open.iter()
        .filter(number)
        .chain(open.iter().filter(|c| !number(c)))
        .map(|c| value_item(function_of(facts(b, c)), c, &(b.quote)(c)))
        .collect()
}

/// Put `made` in place of the value at `at`, under a name free of the other
/// values' and of the items `groups` makes, as an added value's is.
fn retype_value(list: &mut [String], at: usize, made: String, groups: &[String]) {
    let others: Vec<String> = list
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != at)
        .map(|(_, item)| item.clone())
        .chain(groups.iter().cloned())
        .collect();
    list[at] = free_alias(made, &others);
}

/// Add the value `item` to the draft's aggregates, under a name free of theirs
/// and of the columns it groups by.
pub fn add_value(draft: &mut Draft, item: String) {
    let mut list = items(draft.value("aggregates").unwrap_or_default());
    let groups = items(draft.value("group_by").unwrap_or_default());
    let taken: Vec<String> = list.iter().chain(&groups).cloned().collect();
    let item = free_alias(item, &taken);
    list.push(item);
    draft.set("aggregates", joined(&list));
}

/// Add `column`, written as `column_sql`, to what the draft groups by.
pub fn add_group(draft: &mut Draft, column_sql: String) {
    let mut list = items(draft.value("group_by").unwrap_or_default());
    list.push(column_sql);
    draft.set("group_by", joined(&list));
}

fn begin(b: &Begin<'_>) -> Result<Draft, String> {
    let column = b
        .column
        .ok_or_else(|| "Summarise rows needs a column.".to_string())?;
    let mut draft = Draft::empty("summarise-rows");
    draft.on = b.table.map(str::to_string);
    let group = (b.quote)(column);
    // The count is `n`, or `n_2` over a column already named `n`.
    let count = free_alias("count(*) AS n".to_string(), std::slice::from_ref(&group));
    draft.set("group_by", group);
    draft.set("aggregates", count);
    Ok(draft)
}

/// A small heading over a part of the form.
fn caption(ui: &mut egui::Ui, cx: &FormCx<'_>, text: &str) {
    let sem = semantic(cx.mode.is_dark());
    ui.label(
        egui::RichText::new(text)
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
}

/// Text in the form's face and ink, for a button.
fn word(cx: &FormCx<'_>, text: &str) -> egui::RichText {
    let sem = semantic(cx.mode.is_dark());
    egui::RichText::new(text)
        .font(ui_font())
        .color(chrome::colour(sem.text.primary))
}

/// The value `key` as a select list of text, in a field of the id named;
/// whether it was edited.
fn list_field(ui: &mut egui::Ui, draft: &mut Draft, key: &'static str, id: &str) -> bool {
    let mut text = draft.value(key).unwrap_or_default().to_string();
    let edited = ui
        .add(
            egui::TextEdit::singleline(&mut text)
                .id(egui::Id::new(id))
                .font(ui_font())
                .desired_width(f32::INFINITY),
        )
        .changed();
    if edited {
        draft.set(key, text);
    }
    edited
}

/// The form: what the rows are grouped by, then the values each group holds.
fn form(draft: &mut Draft, ui: &mut egui::Ui, cx: &FormCx<'_>) -> FormEvent {
    let b = cx.begin;
    let mut changed = false;

    ui.add_space(spacing::SPACE_4);
    caption(ui, cx, "GROUP BY");
    let groups = items(draft.value("group_by").unwrap_or_default());
    let offers = group_offers(b, draft);
    let mut dropped = None;
    let mut added = None;
    ui.horizontal_wrapped(|ui| {
        for (at, item) in groups.iter().enumerate() {
            if ui
                .button(word(cx, &format!("{item} \u{00d7}")))
                .on_hover_text("Stop grouping by this")
                .clicked()
            {
                dropped = Some(at);
            }
        }
        if !offers.is_empty() {
            ui.menu_button(word(cx, "+ group"), |ui| {
                for column in &offers {
                    if ui.button(word(cx, &format!("by {column}"))).clicked() {
                        added = Some((b.quote)(column));
                        ui.close();
                    }
                }
            });
        }
    });
    if let Some(at) = dropped {
        let mut left = groups;
        left.remove(at);
        draft.set("group_by", joined(&left));
        changed = true;
    }
    if let Some(column_sql) = added {
        add_group(draft, column_sql);
        changed = true;
    }
    changed |= list_field(ui, draft, "group_by", GROUP_BY_FIELD);

    ui.add_space(spacing::SPACE_4);
    caption(ui, cx, "VALUES");
    let values = items(draft.value("aggregates").unwrap_or_default());
    let mut dropped = None;
    let mut retyped = None;
    for (at, item) in values.iter().enumerate() {
        ui.horizontal_wrapped(|ui| {
            if let Some((function, ..)) = as_function(item) {
                ui.menu_button(word(cx, FUNCTIONS[function].1), |ui| {
                    for (other, (_, name)) in FUNCTIONS.iter().enumerate() {
                        if other != function && ui.button(word(cx, name)).clicked() {
                            retyped = with_function(item, other).map(|made| (at, made));
                            ui.close();
                        }
                    }
                });
            }
            if ui
                .button(word(cx, &format!("{item} \u{00d7}")))
                .on_hover_text("Take this value off")
                .clicked()
            {
                dropped = Some(at);
            }
        });
    }
    let offers = value_offers(b, draft);
    let mut added = None;
    if !offers.is_empty() {
        ui.menu_button(word(cx, "+ value"), |ui| {
            for item in &offers {
                let call = item.rsplit_once(" AS ").map_or(item.as_str(), |(c, _)| c);
                if ui.button(word(cx, call)).clicked() {
                    added = Some(item.clone());
                    ui.close();
                }
            }
        });
    }
    if dropped.is_some() || retyped.is_some() {
        let mut list = values;
        if let Some((at, made)) = retyped {
            let groups = items(draft.value("group_by").unwrap_or_default());
            retype_value(&mut list, at, made, &groups);
        }
        if let Some(at) = dropped {
            list.remove(at);
        }
        draft.set("aggregates", joined(&list));
        changed = true;
    }
    if let Some(item) = added {
        add_value(draft, item);
        changed = true;
    }
    changed |= list_field(ui, draft, "aggregates", AGGREGATES_FIELD);

    if changed {
        FormEvent::Changed
    } else {
        FormEvent::None
    }
}

static UI: OperationUi = OperationUi {
    long_name: "summarise-rows",
    verb: "summarise-rows",
    stage: Stage::Shape,
    icon: "sum",
    starts: &[Start::Cell, Start::Header(HeaderPart::Menu)],
    menu_words: "Summarise by this column",
    needs: &[
        Need::ProtocolSource,
        Need::Rows,
        Need::Column,
        Need::ViewRecorded,
    ],
    begin,
    keeps: Keeps::OnEnter,
    field: None,
    form: Some(form),
    name: NameRule {
        word: "summarised",
        from: By::Value("group_by"),
        with_by: true,
    },
};

/// `summarise-rows`'s slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_select_list_splits_at_its_own_commas_only() {
        assert_eq!(
            items("a, round(b, 2) AS r ,'x, y', \"c, d\",, "),
            ["a", "round(b, 2) AS r", "'x, y'", "\"c, d\""]
        );
        assert!(items("  ").is_empty());
    }

    #[test]
    fn a_value_is_named_for_its_function_and_column() {
        assert_eq!(
            value_item(0, "median_income", "median_income"),
            "avg(median_income) AS avg_median_income"
        );
        assert_eq!(
            value_item(DISTINCT, "median income", "\"median income\""),
            "count(DISTINCT \"median income\") AS \"count_distinct_median income\""
        );
    }

    #[test]
    fn a_values_function_changes_and_its_name_follows_unless_typed() {
        assert_eq!(
            with_function("avg(median_income) AS avg_median_income", 1).as_deref(),
            Some("sum(median_income) AS sum_median_income")
        );
        assert_eq!(
            with_function("AVG(median_income) AS income", 3).as_deref(),
            Some("max(median_income) AS income")
        );
        assert_eq!(
            with_function("count(DISTINCT \"a b\") AS \"count_distinct_a b\"", 2).as_deref(),
            Some("min(\"a b\") AS \"min_a b\"")
        );
        assert_eq!(with_function("count(*) AS n", 0), None);
        assert_eq!(with_function("avg(a + 1) AS x", 0), None);
        assert_eq!(with_function("avg(a) + 1 AS x", 0), None);
    }

    #[test]
    fn a_value_added_twice_takes_a_free_name() {
        let mut draft = Draft::empty("summarise-rows");
        draft.set("aggregates", "count(*) AS n".to_string());
        add_value(&mut draft, "avg(a) AS avg_a".to_string());
        add_value(&mut draft, "avg(a) AS avg_a".to_string());
        assert_eq!(
            draft.value("aggregates"),
            Some("count(*) AS n, avg(a) AS avg_a, avg(a) AS avg_a_2")
        );
        // A column grouped by is a name taken too, bare or quoted.
        draft.set("group_by", "\"sum_b\"".to_string());
        add_value(&mut draft, "sum(b) AS sum_b".to_string());
        assert!(draft
            .value("aggregates")
            .unwrap_or_default()
            .ends_with("sum(b) AS sum_b_2"));
    }

    #[test]
    fn a_values_new_function_takes_a_name_free_of_the_other_values() {
        let mut list = vec!["sum(a) AS sum_a".to_string(), "avg(a) AS avg_a".to_string()];
        let made = with_function(&list[1], 1).expect("a function of one column");
        retype_value(&mut list, 1, made, &[]);
        assert_eq!(list, ["sum(a) AS sum_a", "sum(a) AS sum_a_2"]);
        // A name made free is still the function's, so it follows the next change.
        let made = with_function(&list[1], 0).expect("a function of one column");
        retype_value(&mut list, 1, made, &[]);
        assert_eq!(list, ["sum(a) AS sum_a", "avg(a) AS avg_a"]);
        // A name typed stays.
        assert_eq!(
            with_function("avg(a) AS total", 1).as_deref(),
            Some("sum(a) AS total")
        );
        // Its own name is no clash; a column grouped by is.
        let mut list = vec!["avg(a) AS avg_a".to_string()];
        retype_value(&mut list, 0, "avg(a) AS avg_a".to_string(), &[]);
        assert_eq!(list, ["avg(a) AS avg_a"]);
        retype_value(
            &mut list,
            0,
            "sum(a) AS sum_a".to_string(),
            &["sum_a".to_string()],
        );
        assert_eq!(list, ["sum(a) AS sum_a_2"]);
        // A name alike in all but case is taken.
        assert_eq!(
            free_alias("count(*) AS n".to_string(), &["\"N\"".to_string()]),
            "count(*) AS n_2"
        );
    }

    #[test]
    fn a_value_opens_on_the_function_its_column_suits() {
        let decimal = Some("representation.numeric.decimal_number");
        let integer = Some("representation.numeric.integer_number");
        assert_eq!(function_for(decimal, "DOUBLE"), 0);
        // Text FineType reads as a number is not averaged.
        assert_eq!(
            function_for(Some("representation.numeric.percentage"), "VARCHAR"),
            DISTINCT
        );
        assert_eq!(function_for(decimal, "VARCHAR"), DISTINCT);
        // On a number, the label decides between the two.
        assert_eq!(function_for(integer, "DOUBLE"), 1);
        assert_eq!(function_for(integer, "BIGINT"), 1);
        assert_eq!(
            function_for(Some("representation.text.word"), "VARCHAR"),
            DISTINCT
        );
        // A label decides over the storage type.
        assert_eq!(
            function_for(Some("datetime.component.year"), "BIGINT"),
            DISTINCT
        );
        assert_eq!(function_for(None, "DOUBLE"), 0);
        assert_eq!(function_for(None, "BIGINT"), 1);
        assert_eq!(function_for(None, "DATE"), DISTINCT);
        assert_eq!(function_of(None), DISTINCT);
    }
}
