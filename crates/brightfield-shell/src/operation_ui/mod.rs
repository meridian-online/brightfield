//! Brightfield's side of each of arc's operations: one slot per operation.
//!
//! arc holds the catalogue: what an operation reads, the values it takes and
//! the SQL it writes ([`crate::operation`]). What arc cannot say is how an
//! operation is asked for in a window, and that differs by operation. Hiding a
//! column asks nothing; renaming one is typed in its header; a summary wants a
//! small form. So each operation has an [`OperationUi`] of its own, in a file
//! of its own under this module, which says:
//!
//! - where it starts ([`Start`]): the grid's cursor, a part of a column's
//!   header, the spine, a field of the grid's foot row, or the sources browser;
//! - what the window must hold for it to start ([`Need`]);
//! - the draft it starts as ([`OperationUi::begin`]), its values already filled
//!   from what Brightfield knows of the column;
//! - whether it is kept at once or on Enter ([`Keeps`]);
//! - how it asks: a field in place on the header ([`InPlace`]), a form in the
//!   inspector's Operation section, or neither;
//! - how its step is named ([`NameRule`]).
//!
//! [`OPERATIONS`] lists the slots in the order arc lists its operations, one
//! for each operation arc holds (`every_operation_arc_holds_has_one_slot`). A
//! slot answers `None` until its operation is built. The
//! shared code reads a slot and decides nothing about how an operation asks.
//!
//! A [`Draft`] is an operation being made: the table it is on, the other tables
//! it reads by role, and its values as text. [`Draft::request`] is what arc is
//! asked to record, so a draft holds nothing arc does not take
//! (`each_built_operations_draft_names_only_what_arc_describes`).

use brightfield_ossie::Model;
use brightfield_protocol::silver::FinetypeTransforms;

use crate::grid_foot::FootField;
use crate::one_step::ColumnFacts;
use crate::operation::{OperationDescription, StepRequest};

pub mod append_rows;
pub mod cast_column;
pub mod dedupe_rows;
pub mod derive_column;
pub mod draft;
pub mod fill_nulls;
pub mod filter_rows;
pub mod header;
pub mod hide_column;
pub mod join_tables;
pub mod limit_rows;
pub mod open;
pub mod read_file;
pub mod reads;
pub mod rename_column;
pub mod sort_rows;
pub mod summarise_rows;

pub use reads::DraftAnswer;

/// One operation as the window offers it.
pub struct OperationUi {
    /// arc's long name for the operation.
    pub long_name: &'static str,
    /// The key-registry verb that starts it.
    pub verb: &'static str,
    /// The stage of the work it belongs to.
    pub stage: Stage,
    /// Its icon's Tabler name, for the ribbon.
    pub icon: &'static str,
    /// The places it begins from.
    pub starts: &'static [Start],
    /// Its row in a column header's menu, such as "Hide column".
    pub menu_words: &'static str,
    /// What the window must hold for it to start, in the order they are said.
    pub needs: &'static [Need],
    /// The draft it starts as, with its defaults; an `Err` is a sentence saying
    /// why it cannot start here.
    pub begin: fn(&Begin<'_>) -> Result<Draft, String>,
    /// When its draft is kept.
    pub keeps: Keeps,
    /// The field it asks with in place on the header, when it asks there.
    pub field: Option<InPlace>,
    /// Its form in the inspector's Operation section, when it has one.
    pub form: Option<fn(&mut Draft, &mut egui::Ui, &FormCx<'_>) -> FormEvent>,
    /// How its step is named.
    pub name: NameRule,
}

impl std::fmt::Debug for OperationUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationUi")
            .field("long_name", &self.long_name)
            .field("verb", &self.verb)
            .finish_non_exhaustive()
    }
}

/// The stage of the work an operation belongs to: the ribbon's, so a form is
/// filed under the tab of the same name.
pub use brightfield_keys::ribbon::Stage;

/// A place an operation begins from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// The grid's cursor on a cell.
    Cell,
    /// A part of a column's header.
    Header(HeaderPart),
    /// The spine, picking a table for the role named.
    Spine {
        /// The role of arc's the picked table is read under.
        role: &'static str,
    },
    /// A field of the grid's foot row.
    FootField(FootField),
    /// The sources browser.
    SourcesBrowser,
}

/// A part of a column's header an operation can start from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderPart {
    /// The header's menu.
    Menu,
    /// The column's name.
    Name,
    /// The column's type.
    Type,
    /// The count of the column's missing values.
    Missing,
    /// The cell after the last column, where a new column is typed.
    NewColumn,
}

/// When a draft is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keeps {
    /// As soon as arc admits it and DuckDB has run it: it asks nothing.
    AtOnce,
    /// On Enter.
    OnEnter,
}

/// A field an operation asks with in place on a column's header.
#[derive(Clone, Copy)]
pub struct InPlace {
    /// The part of the header the field is drawn over.
    pub part: HeaderPart,
    /// The draft's values the field edits, in Tab order.
    pub keys: &'static [&'static str],
    /// What the field offers under it for the value being typed.
    pub suggestions: fn(&Begin<'_>, &Draft) -> Vec<String>,
}

/// Something the window must hold for an operation to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// A Protocol of the analyst's own, which a Save can record into.
    ProtocolSource,
    /// A Protocol Brightfield made, which takes new sources.
    TakesSources,
    /// A table's rows in front.
    Rows,
    /// The grid's cursor on a cell.
    GridCell,
    /// A column: the cursor's, or the header's the operation started from.
    Column,
    /// A column with missing values.
    ColumnHasNulls,
    /// No `where` or `order by` applied and not recorded.
    ViewRecorded,
    /// A table that has been loaded or run, so its rows can be read.
    LoadedTable,
    /// A second table in the Protocol.
    AnotherTable,
    /// A second table in the Protocol sharing a column's name with this one.
    AnotherTableSharingAColumn,
}

/// An operation being made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// arc's long name for the operation.
    pub long_name: &'static str,
    /// The table the step is recorded on, when the operation reads one.
    pub on: Option<String>,
    /// Each other table it reads, as `(role, table)`, in order.
    pub reads: Vec<(String, String)>,
    /// Its values as text, as `(key, value)`, in the order they were set.
    pub values: Vec<(&'static str, String)>,
    /// The value a field or form opens on, when one is named.
    pub focus: Option<&'static str>,
}

impl Draft {
    /// A draft of `long_name` with no table, reads or values.
    #[must_use]
    pub fn empty(long_name: &'static str) -> Self {
        Self {
            long_name,
            on: None,
            reads: Vec::new(),
            values: Vec::new(),
            focus: None,
        }
    }

    /// The text of the value `key`, when the draft holds one.
    #[must_use]
    pub fn value(&self, key: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Set the value `key` to `text`, in the place it already has or at the end.
    pub fn set(&mut self, key: &'static str, text: String) {
        match self.values.iter_mut().find(|(k, _)| *k == key) {
            Some((_, value)) => *value = text,
            None => self.values.push((key, text)),
        }
    }

    /// What arc is asked to record: the values that are not empty, in order.
    #[must_use]
    pub fn request(&self) -> StepRequest {
        StepRequest {
            long_name: self.long_name.to_string(),
            on: self.on.clone(),
            reads: self.reads.clone(),
            args: self
                .values
                .iter()
                .filter(|(_, value)| !value.trim().is_empty())
                .map(|(key, value)| ((*key).to_string(), value.clone()))
                .collect(),
        }
    }

    /// The first thing `description` requires that the draft lacks, as
    /// `"<long name> needs <key>"`: a required table, then a required value,
    /// then a value that one the draft gives needs beside it
    /// (`a_draft_says_the_first_thing_arc_requires_that_it_lacks`). `None` for a
    /// draft that holds each of them.
    #[must_use]
    pub fn missing(&self, description: &OperationDescription) -> Option<String> {
        let given = |key: &str| self.value(key).is_some_and(|v| !v.trim().is_empty());
        let needs = |what: &str| Some(format!("{} needs {what}", self.long_name));
        for role in &description.reads {
            if !role.required || role.kind != "table" {
                continue;
            }
            let held = if role.name == "table" {
                self.on.is_some()
            } else {
                self.reads.iter().any(|(name, _)| *name == role.name)
            };
            if !held {
                return needs(&role.name);
            }
        }
        for parameter in &description.parameters {
            if parameter.required && !given(&parameter.key) {
                return needs(&parameter.key);
            }
        }
        for (key, beside) in &description.needed_together {
            if given(key) {
                if let Some(lacking) = beside.iter().find(|other| !given(other)) {
                    return needs(lacking);
                }
            }
        }
        None
    }
}

/// What an operation's draft is begun from.
pub struct Begin<'a> {
    /// The place the operation started from.
    pub start: Start,
    /// The table in front, when there is one.
    pub table: Option<&'a str>,
    /// The column the operation started on, when it started on one.
    pub column: Option<&'a str>,
    /// The table's columns, in order.
    pub columns: &'a [String],
    /// What Brightfield knows of each column.
    pub facts: &'a [ColumnFacts],
    /// arc's description of the operation.
    pub description: &'a OperationDescription,
    /// The tables the spine took for the operation, each with its columns.
    pub taken: &'a [(String, Vec<String>)],
    /// The Protocol's model of its tables' relationships, when it has one.
    pub model: Option<&'a Model>,
    /// FineType's transforms, for the DuckDB type a label reads as.
    pub transforms: &'a FinetypeTransforms,
    /// A column's name as SQL spells it: bare when plain, quoted otherwise.
    pub quote: &'a dyn Fn(&str) -> String,
}

/// **What a draft was begun from, kept for as long as it is being made**: the
/// same facts as [`Begin`], owned, so the operation's suggestions and its form
/// can be asked again on a later frame.
#[derive(Clone, Debug)]
pub struct BegunWith {
    /// The place the operation started from.
    pub start: Start,
    /// The table in front, when there was one.
    pub table: Option<String>,
    /// The column the operation started on, when it started on one.
    pub column: Option<String>,
    /// The table's columns, in order.
    pub columns: Vec<String>,
    /// What Brightfield knows of each column.
    pub facts: Vec<ColumnFacts>,
    /// arc's description of the operation.
    pub description: OperationDescription,
    /// The tables the spine took for the operation, each with its columns.
    pub taken: Vec<(String, Vec<String>)>,
    /// Each column as SQL spells it, where the session could say.
    pub spelled: std::collections::BTreeMap<String, String>,
    /// The Protocol's model of its tables' relationships, when it has one and
    /// the operation started from the spine.
    pub model: Option<Model>,
}

/// FineType's vendored transforms, read once for the process.
fn transforms() -> &'static FinetypeTransforms {
    static VENDORED: std::sync::OnceLock<FinetypeTransforms> = std::sync::OnceLock::new();
    VENDORED.get_or_init(FinetypeTransforms::vendored)
}

impl BegunWith {
    /// A column as SQL spells it: as the session spelled it, and quoted for a
    /// column the session was not asked about, which costs looks and reads the
    /// right column.
    #[must_use]
    pub fn quote(&self, name: &str) -> String {
        self.spelled
            .get(name)
            .cloned()
            .unwrap_or_else(|| format!("\"{}\"", name.replace('"', "\"\"")))
    }

    /// Hand `f` these facts as a [`Begin`].
    pub fn with_begin<T>(&self, f: impl FnOnce(&Begin<'_>) -> T) -> T {
        let quote = |name: &str| self.quote(name);
        f(&Begin {
            start: self.start,
            table: self.table.as_deref(),
            column: self.column.as_deref(),
            columns: &self.columns,
            facts: &self.facts,
            description: &self.description,
            taken: &self.taken,
            model: self.model.as_ref(),
            transforms: transforms(),
            quote: &quote,
        })
    }
}

/// What an operation's form is drawn with.
pub struct FormCx<'a> {
    /// What the draft was begun from.
    pub begin: &'a Begin<'a>,
    /// The answer to the draft as it stands, when one has come.
    pub answer: Option<&'a DraftAnswer>,
    /// The theme's mode.
    pub mode: crate::design::Mode,
}

/// What a form did with a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormEvent {
    /// Nothing.
    None,
    /// It changed the draft.
    Changed,
    /// It asked for the draft to be kept.
    Keep,
    /// It asked for the draft to be dropped.
    Cancel,
}

/// How an operation's step is named: `<table>_<word>`, with what
/// [`NameRule::from`] names after it, behind `by_` when [`NameRule::with_by`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NameRule {
    /// The word for what the step did, such as `without` or `summarised`.
    pub word: &'static str,
    /// Where the name's last part comes from.
    pub from: By,
    /// Whether `by_` goes before the last part.
    pub with_by: bool,
}

/// Where the last part of a step's name comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// The name has no last part.
    Nothing,
    /// The draft's value of this key.
    Value(&'static str),
}

impl NameRule {
    /// The last part of the name for `draft`: the first comma-separated item
    /// of the value [`By::Value`] names, without a surrounding `"` pair and
    /// without a trailing `asc` or `desc`. `None` when the rule is [`By::Nothing`],
    /// when the draft lacks the value, when it is empty, and when what is left holds a
    /// character other than a letter, a digit or `_`, or a space outside quotes,
    /// since an expression is no name.
    #[must_use]
    pub fn last_part(&self, draft: &Draft) -> Option<String> {
        let By::Value(key) = self.from else {
            return None;
        };
        let first = draft.value(key)?.split(',').next()?.trim();
        let lower = first.to_ascii_lowercase();
        let first = [" asc", " desc"]
            .iter()
            .find(|direction| lower.ends_with(*direction))
            .map_or(first, |direction| {
                first[..first.len() - direction.len()].trim_end()
            });
        // A space is part of a name inside quotes, and SQL outside them, as
        // in `house_age desc nulls last`.
        let quoted = first
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'));
        let name = quoted.unwrap_or(first);
        let plain = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || (c == ' ' && quoted.is_some()));
        plain.then(|| name.to_string())
    }
}

/// A slot's reader: the operation's [`OperationUi`], or `None` until built.
pub type Slot = fn() -> Option<&'static OperationUi>;

/// Every operation arc holds with its slot, in the order arc lists them.
pub const OPERATIONS: [(&str, Slot); 13] = [
    ("filter-rows", filter_rows::ui),
    ("sort-rows", sort_rows::ui),
    ("hide-column", hide_column::ui),
    ("rename-column", rename_column::ui),
    ("derive-column", derive_column::ui),
    ("cast-column", cast_column::ui),
    ("summarise-rows", summarise_rows::ui),
    ("dedupe-rows", dedupe_rows::ui),
    ("limit-rows", limit_rows::ui),
    ("fill-nulls", fill_nulls::ui),
    ("join-tables", join_tables::ui),
    ("append-rows", append_rows::ui),
    ("read-file", read_file::ui),
];

/// The built operation arc calls `long_name`.
#[must_use]
pub fn spec(long_name: &str) -> Option<&'static OperationUi> {
    OPERATIONS
        .iter()
        .find(|(name, _)| *name == long_name)
        .and_then(|(_, slot)| slot())
}

/// The built operation the registry verb `verb` starts.
#[must_use]
pub fn by_verb(verb: &str) -> Option<&'static OperationUi> {
    built().find(|ui| ui.verb == verb)
}

/// Every built operation, in arc's order.
pub fn built() -> impl Iterator<Item = &'static OperationUi> {
    OPERATIONS.iter().filter_map(|(_, slot)| slot())
}

/// An operation's name as a command reads: `hide-column` is "Hide column".
#[must_use]
pub fn label(long_name: &str) -> String {
    let words = long_name.replace('-', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => words,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(values: &[(&'static str, &str)]) -> Draft {
        let mut draft = Draft::empty("limit-rows");
        for (key, value) in values {
            draft.set(key, (*value).to_string());
        }
        draft
    }

    #[test]
    fn a_command_reads_as_its_long_name_in_words() {
        assert_eq!(label("hide-column"), "Hide column");
        assert_eq!(label("summarise-rows"), "Summarise rows");
        assert_eq!(label(""), "");
    }

    #[test]
    fn a_draft_asks_arc_for_the_values_it_holds_and_no_empty_one() {
        let mut made = draft(&[("count", "10"), ("order_by", " ")]);
        made.on = Some("housing".to_string());
        made.set("count", "5".to_string());
        assert_eq!(made.value("count"), Some("5"));
        let request = made.request();
        assert_eq!(request.long_name, "limit-rows");
        assert_eq!(request.on.as_deref(), Some("housing"));
        assert_eq!(request.args, [("count".to_string(), "5".to_string())]);
    }

    #[test]
    fn a_draft_says_the_first_thing_arc_requires_that_it_lacks() {
        use crate::operation::{Parameter, Role, ValueShape};
        let role = |name: &str, kind: &str, required: bool| Role {
            name: name.to_string(),
            kind: kind.to_string(),
            required,
            description: String::new(),
            list: false,
            min_items: 0,
        };
        let parameter = |key: &str, required: bool| Parameter {
            key: key.to_string(),
            description: String::new(),
            offered: Vec::new(),
            shape: ValueShape::Text,
            required,
        };
        let description = OperationDescription {
            long_name: "limit-rows".to_string(),
            summary: String::new(),
            reads: vec![
                role("table", "table", true),
                role("column", "column", false),
                role("other", "table", true),
            ],
            parameters: vec![
                parameter("count", true),
                parameter("key", false),
                parameter("order_by", false),
            ],
            needed_together: vec![("key".to_string(), vec!["order_by".to_string()])],
        };
        let needs = |what: &str| Some(format!("limit-rows needs {what}"));
        // The table first, then the other table, then a required value.
        let mut made = draft(&[]);
        assert_eq!(made.missing(&description), needs("table"));
        made.on = Some("housing".to_string());
        assert_eq!(made.missing(&description), needs("other"));
        made.reads.push(("other".to_string(), "by_age".to_string()));
        assert_eq!(made.missing(&description), needs("count"));
        // Spaces are no value.
        made.set("count", "  ".to_string());
        assert_eq!(made.missing(&description), needs("count"));
        made.set("count", "10".to_string());
        assert_eq!(made.missing(&description), None);
        // `key` needs `order_by` beside it, and `order_by` alone needs no `key`.
        made.set("order_by", "house_age".to_string());
        assert_eq!(made.missing(&description), None);
        made.set("order_by", String::new());
        made.set("key", "house_age".to_string());
        assert_eq!(made.missing(&description), needs("order_by"));
    }

    #[test]
    fn a_step_is_named_by_a_plain_column_and_by_no_expression() {
        let rule = NameRule {
            word: "first",
            from: By::Value("order_by"),
            with_by: true,
        };
        let part = |text: &str| rule.last_part(&draft(&[("order_by", text)]));
        assert_eq!(part("house_age"), Some("house_age".to_string()));
        assert_eq!(part("house_age DESC, rooms"), Some("house_age".to_string()));
        assert_eq!(
            part("\"median income\" asc"),
            Some("median income".to_string())
        );
        assert_eq!(part("lower(note)"), None);
        assert_eq!(part("house_age desc nulls last"), None);
        assert_eq!(part("note collate nocase"), None);
        assert_eq!(part(",rooms"), None);
        assert_eq!(part(""), None);
        assert_eq!(rule.last_part(&Draft::empty("limit-rows")), None);
        let nothing = NameRule {
            word: "appended",
            from: By::Nothing,
            with_by: false,
        };
        assert_eq!(nothing.last_part(&draft(&[("order_by", "a")])), None);
    }
}
