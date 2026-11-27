//! `join-tables` in the window: the table in front joined to a second table
//! picked in the spine, on keys proposed from what Brightfield knows, and shaped
//! in a small form in the inspector's Operation section.
//!
//! It starts from `&` on a cell, from "Join on this column" on a column
//! header's menu, or from the palette, and each of them opens the spine's pick
//! first, with the related tables marked. Once a table is taken, `begin`
//! proposes the keys, in this order:
//!
//! 1. a relationship the Protocol's model holds between the two tables: the
//!    same names on both sides become `using`, and otherwise `condition` is
//!    `<table>.<a> = <other>.<b>` for each pair of columns, joined by `AND`;
//! 2. the column the operation started on, when the other table holds it;
//! 3. the first of the table's columns the other table holds.
//!
//! A pair of tables sharing no column begins with no key, and the form takes a
//! condition. `kind` is left unset, so the join is arc's default, `left`.
//!
//! The form holds the kind, as arc's seven words; the keys, as a list of
//! column names when the tables share them and as a condition otherwise, each
//! replacing the other when typed; the prefix the other table's columns take,
//! `<other>_` until one is typed, which `semi` and `anti` joins, keeping the
//! table's columns alone, do not offer; and a line that reads the join as SQL,
//! such as `left join by_age using (house_age)`, with the count of the columns
//! the preview adds while the join is first made. Each change re-reads the rows, and Enter keeps the step,
//! named for the table joined (`<table>_joined_<other>`).

use brightfield_ossie::Model;
use brightfield_workbench::chrome;
use meridian_design::{semantic, spacing};

use super::{
    Begin, By, Draft, FormCx, FormEvent, HeaderPart, Keeps, NameRule, OperationUi, Stage, Start,
};
use crate::protocol::ui_font;

/// arc's kinds of join, in the order it lists them; the first is its default.
const KINDS: [&str; 7] = ["left", "inner", "right", "full", "semi", "anti", "cross"];

/// The id of the form's field for the columns the join is on.
pub const USING_FIELD: &str = "join-tables-using";

/// The id of the form's field for the join's condition.
pub const CONDITION_FIELD: &str = "join-tables-condition";

/// The id of the form's field for the prefix the other table's columns take.
pub const PREFIX_FIELD: &str = "join-tables-prefix";

/// The keys a join is proposed on.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Keys {
    /// Columns both tables hold under the same name, given bare.
    Using(Vec<String>),
    /// A condition naming both tables, as SQL.
    Condition(String),
}

/// The keys `model` gives the join of `table` to `other`: the columns of the
/// first relationship it holds between the two, either way round.
fn keys_from_model(model: &Model, b: &Begin<'_>, table: &str, other: &str) -> Option<Keys> {
    let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let pairs: Vec<(&String, &String)> = model.relationships.iter().find_map(|r| {
        if same(&r.from, table) && same(&r.to, other) {
            Some(r.from_columns.iter().zip(&r.to_columns).collect())
        } else if same(&r.from, other) && same(&r.to, table) {
            Some(r.to_columns.iter().zip(&r.from_columns).collect())
        } else {
            None
        }
    })?;
    if pairs.is_empty() {
        return None;
    }
    if pairs.iter().all(|(mine, theirs)| same(mine, theirs)) {
        return Some(Keys::Using(
            pairs.iter().map(|(mine, _)| (*mine).clone()).collect(),
        ));
    }
    let q = b.quote;
    let condition = pairs
        .iter()
        .map(|(mine, theirs)| format!("{}.{} = {}.{}", q(table), q(mine), q(other), q(theirs)))
        .collect::<Vec<_>>()
        .join(" AND ");
    Some(Keys::Condition(condition))
}

/// The keys proposed for joining `b`'s table to `other`, whose columns are
/// `theirs`: the model's, else the operation's column when `other` holds it,
/// else the first column both hold. `None` when the tables share no column.
fn proposed(b: &Begin<'_>, table: &str, other: &str, theirs: &[String]) -> Option<Keys> {
    if let Some(keys) = b
        .model
        .and_then(|model| keys_from_model(model, b, table, other))
    {
        return Some(keys);
    }
    let held = |column: &str| theirs.iter().any(|c| c.eq_ignore_ascii_case(column));
    b.column
        .filter(|column| held(column))
        .or_else(|| {
            b.columns
                .iter()
                .map(String::as_str)
                .find(|column| held(column))
        })
        .map(|column| Keys::Using(vec![column.to_string()]))
}

fn begin(b: &Begin<'_>) -> Result<Draft, String> {
    let table = b
        .table
        .ok_or_else(|| "Join tables needs a table in front.".to_string())?;
    let (other, theirs) = b
        .taken
        .first()
        .ok_or_else(|| "Join tables needs a second table, picked in the spine.".to_string())?;
    let mut draft = Draft::empty("join-tables");
    draft.on = Some(table.to_string());
    draft.reads = vec![("other".to_string(), other.clone())];
    match proposed(b, table, other, theirs) {
        Some(Keys::Using(columns)) => draft.set("using", columns.join(",")),
        Some(Keys::Condition(condition)) => draft.set("condition", condition),
        None => {}
    }
    Ok(draft)
}

/// The table the draft joins to, as it was taken.
fn other_of(draft: &Draft) -> &str {
    draft
        .reads
        .iter()
        .find(|(role, _)| role == "other")
        .map_or("", |(_, table)| table.as_str())
}

/// The draft's kind of join: the one it holds, or arc's default.
fn kind_of(draft: &Draft) -> &str {
    draft
        .value("kind")
        .map(str::trim)
        .filter(|kind| !kind.is_empty())
        .unwrap_or(KINDS[0])
}

/// Whether a join of `kind` keeps the table's columns alone, and so takes no
/// prefix.
fn keeps_table_alone(kind: &str) -> bool {
    matches!(kind, "semi" | "anti")
}

/// The join as a line of SQL reads it: `left join by_age using (house_age)`,
/// `inner join customers on <condition>` or `cross join by_age`. `other` is
/// the other table as SQL spells it.
fn as_sql(draft: &Draft, other: &str) -> String {
    let kind = kind_of(draft);
    let given = |key: &str| draft.value(key).map(str::trim).filter(|v| !v.is_empty());
    let on = match (given("using"), given("condition")) {
        _ if kind == "cross" => String::new(),
        (Some(using), _) => {
            let keys: Vec<&str> = using.split(',').map(str::trim).collect();
            format!(" using ({})", keys.join(", "))
        }
        (None, Some(condition)) => format!(" on {condition}"),
        (None, None) => String::new(),
    };
    format!("{kind} join {other}{on}")
}

/// How many columns the answer adds to the table's: `None` before an answer,
/// and for a kept join being corrected, whose grid in front is the join's own
/// rows rather than the table joined.
fn added(cx: &FormCx<'_>) -> Option<usize> {
    if cx.begin.taken.is_empty() {
        return None;
    }
    let answer = cx.answer?;
    Some(
        answer
            .page
            .columns
            .len()
            .saturating_sub(cx.begin.columns.len()),
    )
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

/// The value `key` in a field of the id named, with `hint` shown while it is
/// empty; the text when it was edited.
fn field(ui: &mut egui::Ui, draft: &Draft, key: &str, id: &str, hint: &str) -> Option<String> {
    let mut text = draft.value(key).unwrap_or_default().to_string();
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(egui::Id::new(id))
            .font(ui_font())
            .hint_text(hint)
            .desired_width(f32::INFINITY),
    )
    .changed()
    .then_some(text)
}

/// The form: the kind, the keys, the prefix, and the join read as SQL.
fn form(draft: &mut Draft, ui: &mut egui::Ui, cx: &FormCx<'_>) -> FormEvent {
    let sem = semantic(cx.mode.is_dark());
    let font = ui_font();
    let mut changed = false;
    let other = other_of(draft).to_string();

    ui.add_space(spacing::SPACE_4);
    caption(ui, cx, "JOIN");
    let now = kind_of(draft).to_string();
    let mut chosen = None;
    ui.horizontal_wrapped(|ui| {
        for kind in KINDS {
            let words = egui::RichText::new(kind).font(font.clone());
            if chrome::selectable_row(ui, now == kind, words).clicked() && now != kind {
                chosen = Some(kind);
            }
        }
    });
    if let Some(kind) = chosen {
        draft.set("kind", kind.to_string());
        // arc refuses a prefix on a join that keeps the table's columns alone.
        if keeps_table_alone(kind) {
            draft.set("prefix", String::new());
        }
        changed = true;
    }

    ui.add_space(spacing::SPACE_4);
    caption(ui, cx, "ON THE COLUMNS BOTH HOLD");
    if let Some(text) = field(ui, draft, "using", USING_FIELD, "house_age, \u{2026}") {
        draft.set("using", text);
        draft.set("condition", String::new());
        changed = true;
    }
    ui.add_space(spacing::SPACE_2);
    caption(ui, cx, "OR WHERE");
    let table = draft.on.clone().unwrap_or_default();
    let hint = format!("{table}.a = {other}.b");
    if let Some(text) = field(ui, draft, "condition", CONDITION_FIELD, &hint) {
        draft.set("condition", text);
        draft.set("using", String::new());
        changed = true;
    }

    if !keeps_table_alone(kind_of(draft)) {
        ui.add_space(spacing::SPACE_4);
        caption(ui, cx, "ITS COLUMNS START WITH");
        let hint = format!("{other}_");
        if let Some(text) = field(ui, draft, "prefix", PREFIX_FIELD, &hint) {
            draft.set("prefix", text);
            changed = true;
        }
    }

    ui.add_space(spacing::SPACE_4);
    let spelled = (cx.begin.quote)(&other);
    ui.label(
        egui::RichText::new(as_sql(draft, &spelled))
            .font(font.clone())
            .color(chrome::colour(sem.text.secondary)),
    );
    if let Some(count) = added(cx) {
        let words = match count {
            1 => "Adds 1 column".to_string(),
            n => format!("Adds {n} columns"),
        };
        ui.label(
            egui::RichText::new(words)
                .font(font)
                .color(chrome::colour(sem.text.muted)),
        );
    }

    if changed {
        FormEvent::Changed
    } else {
        FormEvent::None
    }
}

static UI: OperationUi = OperationUi {
    long_name: "join-tables",
    verb: "join-tables",
    stage: Stage::Model,
    icon: "arrows-join",
    starts: &[
        Start::Spine { role: "other" },
        Start::Cell,
        Start::Header(HeaderPart::Menu),
    ],
    menu_words: "Join on this column",
    begin,
    keeps: Keeps::OnEnter,
    field: None,
    form: Some(form),
    name: NameRule {
        word: "joined",
        from: By::Read("other"),
        with_by: false,
    },
};

/// `join-tables`' slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::OperationDescription;

    fn description() -> OperationDescription {
        OperationDescription {
            long_name: "join-tables".to_string(),
            summary: String::new(),
            reads: Vec::new(),
            parameters: Vec::new(),
            needed_together: Vec::new(),
        }
    }

    /// A model of two tables, `orders` and `customers`, with one relationship
    /// from `from` to `to` on the columns given, each side's in order.
    fn model(from: &str, to: &str, from_columns: &[&str], to_columns: &[&str]) -> Model {
        let list = |columns: &[&str]| {
            columns
                .iter()
                .map(|c| format!("  - {c}\n"))
                .collect::<String>()
        };
        let dataset = |name: &str| {
            format!(
                "- name: {name}\n  source: {name}.csv\n  primary_key:\n  - id\n  fields:\n  - name: \
                 id\n    expression:\n      sql: id\n    custom_extensions:\n    - vendor_name: \
                 MERIDIAN\n      data: '{{\"decided_by\":\"dovetail\"}}'\n  custom_extensions:\n  - \
                 vendor_name: MERIDIAN\n    data: '{{\"role\":\"entity\",\"role_evidence\":[],\
                 \"excluded\":false,\"decided_by\":\"dovetail\",\"source_file\":\"{name}.csv\",\
                 \"rows\":5}}'\n"
            )
        };
        let text = format!(
            "version: 0.2.0.dev0\nname: shop\ndatasets:\n{}{}relationships:\n- name: r\n  from: \
             {from}\n  to: {to}\n  from_columns:\n{}  to_columns:\n{}  custom_extensions:\n  - \
             vendor_name: MERIDIAN\n    data: \
             '{{\"grade\":\"likely\",\"evidence\":{{\"value_overlap\":1.0,\"orphans\":0,\
             \"child_rows\":8,\"child_type\":\"BIGINT\",\"parent_type\":\"BIGINT\",\
             \"types_agree\":true,\"name_similarity\":1.0}},\"cardinality\":\"N:1\",\
             \"verified\":\"full\",\"decided_by\":\"analyst\"}}'\n",
            dataset("orders"),
            dataset("customers"),
            list(from_columns),
            list(to_columns),
        );
        brightfield_ossie::read(&text).unwrap_or_else(|e| panic!("the model reads: {e:?}"))
    }

    /// The draft `begin` makes over `orders`, with `customers` taken holding
    /// the columns `theirs`, `model` given and the operation started on `column`.
    fn begun(model: Option<&Model>, column: Option<&str>, theirs: &[&str]) -> Draft {
        let columns: Vec<String> = ["order_id", "customer_id", "region", "total name"]
            .iter()
            .map(|c| (*c).to_string())
            .collect();
        let taken = vec![(
            "customers".to_string(),
            theirs.iter().map(|c| (*c).to_string()).collect(),
        )];
        let description = description();
        let quote = |name: &str| {
            if name.contains(' ') {
                format!("\"{name}\"")
            } else {
                name.to_string()
            }
        };
        begin(&Begin {
            start: Start::Spine { role: "other" },
            table: Some("orders"),
            column,
            columns: &columns,
            facts: &[],
            description: &description,
            taken: &taken,
            model,
            transforms: super::super::transforms(),
            quote: &quote,
        })
        .expect("the join begins")
    }

    #[test]
    fn join_keys_come_from_the_models_relationship() {
        let related = model("orders", "customers", &["customer_id"], &["id"]);
        let draft = begun(Some(&related), Some("region"), &["id", "region"]);
        assert_eq!(draft.on.as_deref(), Some("orders"));
        assert_eq!(
            draft.reads,
            [("other".to_string(), "customers".to_string())]
        );
        assert_eq!(
            draft.value("condition"),
            Some("orders.customer_id = customers.id")
        );
        assert_eq!(draft.value("using"), None);
        assert_eq!(draft.value("kind"), None);

        // Read the other way round, the columns still pair the table's first.
        let reversed = model("customers", "orders", &["id"], &["customer_id"]);
        let draft = begun(Some(&reversed), None, &["id"]);
        assert_eq!(
            draft.value("condition"),
            Some("orders.customer_id = customers.id")
        );

        // Several columns join by AND, each spelled as SQL spells it.
        let two = model(
            "orders",
            "customers",
            &["customer_id", "total name"],
            &["id", "total name 2"],
        );
        let draft = begun(Some(&two), None, &["id"]);
        assert_eq!(
            draft.value("condition"),
            Some(
                "orders.customer_id = customers.id AND orders.\"total name\" = \
                 customers.\"total name 2\""
            )
        );
    }

    #[test]
    fn a_relationship_on_the_same_names_joins_using_them() {
        let same = model("orders", "customers", &["region"], &["REGION"]);
        let draft = begun(Some(&same), None, &["REGION"]);
        assert_eq!(draft.value("using"), Some("region"));
        assert_eq!(draft.value("condition"), None);
    }

    #[test]
    fn without_a_relationship_the_operations_column_then_the_first_shared() {
        // The model relates neither table here.
        let unrelated = model("customers", "elsewhere", &["id"], &["id"]);
        let draft = begun(Some(&unrelated), Some("region"), &["customer_id", "region"]);
        assert_eq!(draft.value("using"), Some("region"));
        // A column the other table lacks gives way to the first both hold.
        let draft = begun(None, Some("order_id"), &["region", "customer_id"]);
        assert_eq!(draft.value("using"), Some("customer_id"));
        // No shared column: no key, and the form takes a condition.
        let draft = begun(None, Some("order_id"), &["id"]);
        assert_eq!(draft.value("using"), None);
        assert_eq!(draft.value("condition"), None);
    }

    #[test]
    fn the_join_reads_as_a_line_of_sql() {
        let mut draft = Draft::empty("join-tables");
        draft.set("using", "house_age, rooms".to_string());
        assert_eq!(
            as_sql(&draft, "by_age"),
            "left join by_age using (house_age, rooms)"
        );
        draft.set("kind", "inner".to_string());
        draft.set("using", String::new());
        draft.set("condition", "a.x = by_age.y".to_string());
        assert_eq!(
            as_sql(&draft, "by_age"),
            "inner join by_age on a.x = by_age.y"
        );
        draft.set("kind", "cross".to_string());
        assert_eq!(as_sql(&draft, "by_age"), "cross join by_age");
    }
}
