//! `derive-column` in the window: an expression typed in a new header at the
//! table's end, its values filling under it at each keystroke.
//!
//! It starts from `=` with the grid's cursor on a cell, from "Add a column
//! from this one" on a column header's menu, or from the header's new-column
//! cell. It asks in place, in a header cell of its own at the table's end:
//! first the expression, then, after Tab, the new column's name. Enter keeps
//! the step ([`Keeps::OnEnter`]).
//!
//! The expression starts as the column it started on, as SQL spells it, so
//! `"median income"` is quoted; from the new-column cell it starts empty. The
//! name follows the expression until it is typed into
//! (`a_name_typed_into_stops_following_the_expression`): it is the expression's
//! letters, digits and `_`, each run of anything else made one `_`, trimmed of
//! `_` and cut at 30 characters, then made free of the table's columns with
//! `_2`, `_3`. An expression with no letter or digit names the column
//! `derived`. While the expression is in focus the field offers to complete the
//! column its last word begins, as SQL spells it.
//!
//! It needs no column of its own, since the new-column cell starts it on none.

use super::{
    Begin, By, Draft, HeaderPart, InPlace, Keeps, NameRule, Need, OperationUi, Stage, Start,
};

/// The longest name the expression gives the column, in characters.
const NAME_LENGTH: usize = 30;

/// The name `expression` gives its column, free of `columns`.
fn name_for(expression: &str, columns: &[String]) -> String {
    let mut slug = String::new();
    for c in expression.chars() {
        if c.is_alphanumeric() || c == '_' {
            slug.push(c);
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let cut: String = slug.trim_matches('_').chars().take(NAME_LENGTH).collect();
    let base = match cut.trim_end_matches('_') {
        "" => "derived",
        name => name,
    };
    let taken = |name: &str| columns.iter().any(|c| c.eq_ignore_ascii_case(name));
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|name| !taken(name))
        .expect("some suffix is free")
}

fn begin(b: &Begin<'_>) -> Result<Draft, String> {
    let from_new_column = b.start == Start::Header(HeaderPart::NewColumn);
    let expression = match b.column {
        Some(column) if !from_new_column => (b.quote)(column),
        _ => String::new(),
    };
    let mut draft = Draft::empty("derive-column");
    draft.on = b.table.map(str::to_string);
    draft.set("column", name_for(&expression, b.columns));
    draft.set("expression", expression);
    draft.focus = Some("expression");
    Ok(draft)
}

/// The column the expression's last word begins, completed as SQL spells it:
/// the whole expression with that word finished, one for each column it could
/// be. Nothing while the name is in focus, or when the word is already a
/// column's whole name.
fn suggestions(b: &Begin<'_>, draft: &Draft) -> Vec<String> {
    if draft.focus == Some("column") {
        return Vec::new();
    }
    let text = draft.value("expression").unwrap_or_default();
    let word_at = text
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map_or(text.len(), |(at, _)| at);
    let word = text[word_at..].to_lowercase();
    if word.is_empty() {
        return Vec::new();
    }
    b.columns
        .iter()
        .filter(|c| {
            let lower = c.to_lowercase();
            lower.starts_with(&word) && lower != word
        })
        .map(|c| format!("{}{}", &text[..word_at], (b.quote)(c)))
        .collect()
}

/// After the expression is edited from `before`, the name follows it while it
/// is still the one `before` gave: a name typed into, or kept from a
/// correction under another name, stays.
fn follow(b: &Begin<'_>, draft: &mut Draft, edited: &str, before: &str) {
    if edited != "expression" {
        return;
    }
    let followed = name_for(before, b.columns);
    if draft.value("column").is_none_or(|name| name == followed) {
        let now = name_for(draft.value("expression").unwrap_or_default(), b.columns);
        draft.set("column", now);
    }
}

static UI: OperationUi = OperationUi {
    long_name: "derive-column",
    verb: "derive-column",
    stage: Stage::Shape,
    icon: "math-function",
    starts: &[
        Start::Cell,
        // The menu ahead of the new-column cell: a choice from a header's menu starts from
        // the first header part listed, and from the menu the expression is that column.
        Start::Header(HeaderPart::Menu),
        Start::Header(HeaderPart::NewColumn),
    ],
    menu_words: "Add a column from this one",
    needs: &[Need::ProtocolSource, Need::Rows, Need::ViewRecorded],
    begin,
    keeps: Keeps::OnEnter,
    field: Some(InPlace {
        part: HeaderPart::NewColumn,
        keys: &["expression", "column"],
        suggestions,
        follows: Some(follow),
    }),
    form: None,
    name: NameRule {
        word: "with",
        from: By::Value("column"),
        with_by: false,
    },
};

/// `derive-column`'s slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}

#[cfg(test)]
mod tests {
    use super::{name_for, NAME_LENGTH};

    fn columns(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn an_expression_names_its_column_by_its_letters_and_digits() {
        let none = columns(&[]);
        assert_eq!(name_for("median_income * 2", &none), "median_income_2");
        assert_eq!(name_for("\"median income\"", &none), "median_income");
        assert_eq!(name_for("lower(city)", &none), "lower_city");
        assert_eq!(name_for("  a  +  b  ", &none), "a_b");
        assert_eq!(name_for("1 + 1", &none), "1_1");
        assert_eq!(name_for("", &none), "derived");
        assert_eq!(name_for("* -", &none), "derived");
        let long = name_for("a_very_long_expression_name_that_goes_on + more", &none);
        assert_eq!(long.chars().count(), NAME_LENGTH);
        // A cut that ends on a `_` drops it.
        assert_eq!(
            name_for("abcdefghijklmnopqrstuvwxyz123 + x", &none),
            "abcdefghijklmnopqrstuvwxyz123"
        );
    }

    #[test]
    fn a_name_the_table_has_takes_the_first_free_suffix() {
        let held = columns(&["median_income", "Median_Income_2", "x"]);
        assert_eq!(name_for("median_income", &held), "median_income_3");
        assert_eq!(name_for("x", &held), "x_2");
        assert_eq!(name_for("y", &held), "y");
    }
}
