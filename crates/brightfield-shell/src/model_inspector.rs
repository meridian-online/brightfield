//! The model pane's inspector: what the analyst selected on the diagram or the list, and what can
//! be done with it.
//!
//! For a join it shows the keys and cardinality, the grade, each piece of evidence, how much of
//! the data that evidence was measured on, and a sample of the joined rows read from the
//! Protocol's database. For a table it shows the role, each column with its FineType type and how
//! many of its values failed to cast, the tables folded into it, and the first of the failed
//! values with the row each came from.
//!
//! Its buttons do nothing themselves: each one is a [`ModelAction`] the pane dispatches, so a
//! click, a test and a key reach the same change. An action that needs a choice first, such as
//! the table to fold into, opens a [`ModelPicker`] under the buttons, and the picker's answer is
//! the edit itself, as [`ModelAction::Edit`].

use std::path::{Path, PathBuf};

use brightfield_engine::semantic::SchemaCatalogue;
use brightfield_ossie::{DecidedBy, Edit, Grade, Model, Role, Verified, LINEAGE};
use brightfield_render::model_scene::Selection;
use brightfield_workbench::chrome;
use meridian_design::{control, semantic, spacing};

use crate::design::Mode;
use crate::protocol::{mono_font, ui_font};

/// How many joined rows the preview samples, and how many failed values a table lists.
pub const PREVIEW_ROWS: usize = 20;

/// The width of the inspector, beside the view it inspects.
pub const INSPECTOR_WIDTH: f32 = 360.0;

/// The folder inside the Protocol that holds the snapshots opened in the grid.
pub(crate) const GRID_DIR: &str = ".grid";

/// The roles in the order the role picker lists them.
pub const ROLES: [Role; 6] = [
    Role::Entity,
    Role::Event,
    Role::Reference,
    Role::Bridge,
    Role::Snapshot,
    Role::Unclear,
];

/// One thing the inspector offers to do to the model or with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelAction {
    /// Keep a possible join as the analyst's.
    Keep(String),
    /// Drop a join.
    Drop(String),
    /// Change a join's key columns.
    ChangeKeys(String),
    /// Show the Ossie file.
    ShowInFile,
    /// Give a table another role.
    ChangeRole { dataset: String, role: Role },
    /// Fold a table into another.
    FoldInto(String),
    /// Unfold a table from the one it is folded into.
    Unfold(String),
    /// Leave a table out of the model.
    LeaveOut(String),
    /// Bring a table left out of the model back into it.
    Include(String),
    /// Give one of a table's columns another type.
    RetypeColumn(String),
    /// Add a join from a table to another.
    AddJoin(String),
    /// Choose a table's role from a picker, as the keyboard does; the inspector's own role picker
    /// answers [`ModelAction::ChangeRole`] directly.
    PickRole(String),
    /// Open the selection's rows in the grid: a table's silver rows, or a join's joined rows.
    OpenInGrid(Selection),
    /// Make one edit, as a picker answers once its choices are made.
    Edit(Edit),
}

impl ModelAction {
    /// The words on the action's button. Unfold names the table it unfolds, since a host's
    /// inspector offers one for each table folded into it.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            ModelAction::Keep(_) => "Keep".into(),
            ModelAction::Drop(_) => "Drop".into(),
            ModelAction::ChangeKeys(_) => "Change keys\u{2026}".into(),
            ModelAction::ShowInFile => "Show in file".into(),
            ModelAction::ChangeRole { .. } => "Change role".into(),
            ModelAction::FoldInto(_) => "Fold into\u{2026}".into(),
            ModelAction::Unfold(name) => format!("Unfold {name}"),
            ModelAction::LeaveOut(_) => "Leave out".into(),
            ModelAction::Include(_) => "Include".into(),
            ModelAction::RetypeColumn(_) => "Retype column\u{2026}".into(),
            ModelAction::AddJoin(_) => "Add join\u{2026}".into(),
            ModelAction::PickRole(_) => "Change role".into(),
            ModelAction::OpenInGrid(_) => "Open in grid".into(),
            ModelAction::Edit(_) => "Apply".into(),
        }
    }
}

/// Whether `dataset` has tables of its own in the Protocol's database: it is neither left out nor
/// folded into another.
#[must_use]
pub fn materialised(model: &Model, dataset: &str) -> bool {
    model
        .datasets
        .iter()
        .any(|d| d.name == dataset && !d.meta.excluded && d.meta.folded_into.is_none())
}

/// The buttons a join's inspector offers, in order. Keep is offered for a possible join the
/// analyst has not yet settled; a likely one needs no keeping.
#[must_use]
pub fn join_actions(model: &Model, relationship: &str) -> Vec<ModelAction> {
    let Some(r) = model.relationships.iter().find(|r| r.name == relationship) else {
        return Vec::new();
    };
    let name = r.name.clone();
    let mut actions = Vec::new();
    if r.meta.grade == Grade::Possible && r.meta.decided_by != DecidedBy::Analyst {
        actions.push(ModelAction::Keep(name.clone()));
    }
    actions.push(ModelAction::Drop(name.clone()));
    actions.push(ModelAction::ChangeKeys(name.clone()));
    actions.push(ModelAction::ShowInFile);
    actions.push(ModelAction::OpenInGrid(Selection::Relationship(name)));
    actions
}

/// The buttons a table's inspector offers, in order, besides its role picker: a folded table's
/// Unfold, or Fold into…; an Unfold for each table folded into this one and not left out; Leave
/// out, or Include for a table already left out; then Retype column…, and for a table with a
/// silver table of its own Add join…; Show in file; and, again for a table with a silver table of
/// its own, Open in grid.
#[must_use]
pub fn table_actions(model: &Model, dataset: &str) -> Vec<ModelAction> {
    let Some(d) = model.datasets.iter().find(|d| d.name == dataset) else {
        return Vec::new();
    };
    let name = d.name.clone();
    let mut actions = Vec::new();
    if d.meta.folded_into.is_some() {
        actions.push(ModelAction::Unfold(name.clone()));
    } else {
        actions.push(ModelAction::FoldInto(name.clone()));
    }
    let mut folded: Vec<&str> = model
        .datasets
        .iter()
        .filter(|f| !f.meta.excluded && f.meta.folded_into.as_deref() == Some(dataset))
        .map(|f| f.name.as_str())
        .collect();
    folded.sort_unstable();
    actions.extend(
        folded
            .into_iter()
            .map(|f| ModelAction::Unfold(f.to_string())),
    );
    if d.meta.excluded {
        actions.push(ModelAction::Include(name.clone()));
    } else {
        actions.push(ModelAction::LeaveOut(name.clone()));
    }
    actions.push(ModelAction::RetypeColumn(name.clone()));
    if materialised(model, dataset) {
        actions.push(ModelAction::AddJoin(name.clone()));
    }
    actions.push(ModelAction::ShowInFile);
    if materialised(model, dataset) {
        actions.push(ModelAction::OpenInGrid(Selection::Dataset(name)));
    }
    actions
}

/// A choice the inspector asks for before it can make an edit, drawn under its buttons. Each holds
/// what it offers, read from the model when it opened, and what is chosen so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelPicker {
    /// The columns a join matches on: one pair for each of its keys.
    Keys {
        relationship: String,
        from: String,
        to: String,
        from_choices: Vec<String>,
        to_choices: Vec<String>,
        from_columns: Vec<String>,
        to_columns: Vec<String>,
    },
    /// The table to fold `dataset` into: each table a relationship joins it to.
    Fold { dataset: String, hosts: Vec<String> },
    /// One of `dataset`'s columns, and the FineType label to give it. Choosing another column puts
    /// the label it has now in `finetype`, so the type offered is always the chosen column's.
    Retype {
        dataset: String,
        /// Each column, with the label the model gives it now.
        fields: Vec<(String, Option<String>)>,
        labels: Vec<String>,
        field: String,
        finetype: String,
        /// The column `finetype` was last set for.
        typed_for: String,
    },
    /// The role to give `dataset`.
    Role { dataset: String },
    /// A join from `from` to another table, and the column on each side.
    AddJoin {
        from: String,
        from_choices: Vec<String>,
        /// Each table the join can go to, with its columns.
        tables: Vec<(String, Vec<String>)>,
        to: String,
        from_column: String,
        to_column: String,
    },
}

impl ModelPicker {
    /// What the picker is about: the join whose keys it changes, or the table it acts on.
    #[must_use]
    pub fn subject(&self) -> Selection {
        match self {
            ModelPicker::Keys { relationship, .. } => Selection::Relationship(relationship.clone()),
            ModelPicker::Fold { dataset, .. }
            | ModelPicker::Retype { dataset, .. }
            | ModelPicker::Role { dataset } => Selection::Dataset(dataset.clone()),
            ModelPicker::AddJoin { from, .. } => Selection::Dataset(from.clone()),
        }
    }

    /// The line the picker asks its question in.
    #[must_use]
    pub fn prompt(&self) -> String {
        match self {
            ModelPicker::Keys { relationship, .. } => {
                format!("Choose the columns {relationship} matches on")
            }
            ModelPicker::Fold { dataset, .. } => format!("Fold {dataset} into"),
            ModelPicker::Retype { dataset, .. } => {
                format!("Choose a column of {dataset} and its type")
            }
            ModelPicker::Role { dataset } => format!("Choose the role of {dataset}"),
            ModelPicker::AddJoin { from, .. } => {
                format!("Join {from} to another table on a column of each")
            }
        }
    }
}

/// `dataset`'s columns, without the lineage columns every table carries.
fn columns(model: &Model, dataset: &str) -> Vec<String> {
    model
        .datasets
        .iter()
        .find(|d| d.name == dataset)
        .map(|d| {
            d.fields
                .iter()
                .filter(|f| !LINEAGE.contains(&f.name.as_str()))
                .map(|f| f.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The picker for the columns `relationship` matches on, opened on the ones it has now.
///
/// # Errors
///
/// When the model has no join of that name.
pub fn keys_picker(model: &Model, relationship: &str) -> Result<ModelPicker, String> {
    let r = model
        .relationships
        .iter()
        .find(|r| r.name == relationship)
        .ok_or_else(|| format!("the model has no join named {relationship}"))?;
    Ok(ModelPicker::Keys {
        relationship: r.name.clone(),
        from: r.from.clone(),
        to: r.to.clone(),
        from_choices: columns(model, &r.from),
        to_choices: columns(model, &r.to),
        from_columns: r.from_columns.clone(),
        to_columns: r.to_columns.clone(),
    })
}

/// The tables `dataset` can be folded into: each one a relationship joins it to that is in the
/// model and has a table of its own, and is not the one it is folded into already. In name order.
#[must_use]
pub fn fold_hosts(model: &Model, dataset: &str) -> Vec<String> {
    let current = model
        .datasets
        .iter()
        .find(|d| d.name == dataset)
        .and_then(|d| d.meta.folded_into.as_deref());
    let mut hosts: Vec<String> = model
        .relationships
        .iter()
        .filter_map(|r| {
            if r.from == dataset {
                Some(&r.to)
            } else if r.to == dataset {
                Some(&r.from)
            } else {
                None
            }
        })
        .filter(|host| {
            host.as_str() != dataset && Some(host.as_str()) != current && materialised(model, host)
        })
        .cloned()
        .collect();
    hosts.sort_unstable();
    hosts.dedup();
    hosts
}

/// The picker for the table to fold `dataset` into.
///
/// # Errors
///
/// When no relationship joins it to a table it could fold into, so there is nothing to offer.
pub fn fold_picker(model: &Model, dataset: &str) -> Result<ModelPicker, String> {
    let hosts = fold_hosts(model, dataset);
    if hosts.is_empty() {
        return Err(format!(
            "{dataset} can't fold into another table: no relationship joins it to one"
        ));
    }
    Ok(ModelPicker::Fold {
        dataset: dataset.to_string(),
        hosts,
    })
}

/// The picker for a column of `dataset` and its type, offering every label FineType's vendored
/// transformation contracts define, and opened on the first column and the label it has.
///
/// # Errors
///
/// When the table has no columns to retype.
pub fn retype_picker(model: &Model, dataset: &str) -> Result<ModelPicker, String> {
    let fields: Vec<(String, Option<String>)> = columns(model, dataset)
        .into_iter()
        .map(|f| {
            let label = label_of(model, dataset, &f);
            (f, label)
        })
        .collect();
    let Some(field) = fields.first().map(|(f, _)| f.clone()) else {
        return Err(format!("{dataset} has no columns to retype"));
    };
    let labels: Vec<String> = brightfield_protocol::silver::FinetypeTransforms::vendored()
        .labels()
        .map(str::to_string)
        .collect();
    let finetype = offered_label(&fields, &labels, &field);
    Ok(ModelPicker::Retype {
        dataset: dataset.to_string(),
        fields,
        labels,
        typed_for: field.clone(),
        field,
        finetype,
    })
}

/// The label the Retype picker offers for `field`: the one the model gives it, or else the first
/// label FineType defines.
fn offered_label(fields: &[(String, Option<String>)], labels: &[String], field: &str) -> String {
    fields
        .iter()
        .find(|(f, _)| f == field)
        .and_then(|(_, label)| label.clone())
        .or_else(|| labels.first().cloned())
        .unwrap_or_default()
}

/// The FineType label the model gives `field` of `dataset`, if any.
fn label_of(model: &Model, dataset: &str, field: &str) -> Option<String> {
    model
        .datasets
        .iter()
        .find(|d| d.name == dataset)?
        .fields
        .iter()
        .find(|f| f.name == field)?
        .meta
        .finetype
        .clone()
}

/// The picker for `dataset`'s role.
///
/// # Errors
///
/// When the model has no table of that name.
pub fn role_picker_for(model: &Model, dataset: &str) -> Result<ModelPicker, String> {
    if !model.datasets.iter().any(|d| d.name == dataset) {
        return Err(format!("the model has no table named {dataset}"));
    }
    Ok(ModelPicker::Role {
        dataset: dataset.to_string(),
    })
}

/// The picker for a join from `from`: to `to` when one is given, as a drag from one card to another
/// gives it, or else to the first other table. Each side opens on a column the two tables share by
/// name, or on its first column.
///
/// # Errors
///
/// When `from` has no table of its own, or there is no other table to join it to.
pub fn add_join_picker(model: &Model, from: &str, to: Option<&str>) -> Result<ModelPicker, String> {
    if !materialised(model, from) {
        return Err(format!(
            "{from} has no table of its own to join: it is left out or folded into another"
        ));
    }
    let tables: Vec<(String, Vec<String>)> = model
        .datasets
        .iter()
        .filter(|d| d.name != from && materialised(model, &d.name))
        .map(|d| (d.name.clone(), columns(model, &d.name)))
        .collect();
    let to = match to {
        Some(to) if tables.iter().any(|(t, _)| t == to) => to.to_string(),
        Some(to) => {
            return Err(format!(
                "{from} can't join {to}: {to} has no table of its own"
            ))
        }
        None => match tables.first() {
            Some((t, _)) => t.clone(),
            None => return Err(format!("there is no other table to join {from} to")),
        },
    };
    let from_choices = columns(model, from);
    let (from_column, to_column) = shared_column(&from_choices, &to_columns(&tables, &to));
    Ok(ModelPicker::AddJoin {
        from: from.to_string(),
        from_choices,
        tables,
        to,
        from_column,
        to_column,
    })
}

/// The columns of `table` among the picker's `tables`.
fn to_columns(tables: &[(String, Vec<String>)], table: &str) -> Vec<String> {
    tables
        .iter()
        .find(|(t, _)| t == table)
        .map(|(_, c)| c.clone())
        .unwrap_or_default()
}

/// The first column of `from` that `to` also has, on both sides; or each side's first column.
fn shared_column(from: &[String], to: &[String]) -> (String, String) {
    match from.iter().find(|c| to.contains(c)) {
        Some(c) => (c.clone(), c.clone()),
        None => (
            from.first().cloned().unwrap_or_default(),
            to.first().cloned().unwrap_or_default(),
        ),
    }
}

/// Draw `picker` into `ui`, and answer whether it stays open. A choice made pushes the edit it
/// makes onto `chosen` and closes it, and so does Cancel, with nothing pushed.
pub fn picker_ui(
    ui: &mut egui::Ui,
    picker: &mut ModelPicker,
    chosen: &mut Vec<ModelAction>,
    drawn: &mut InspectorDrawn,
) -> bool {
    ui.add(
        egui::Label::new(
            egui::RichText::new(picker.prompt())
                .font(ui_font())
                .strong(),
        )
        .wrap(),
    );
    drawn.lines.push(picker.prompt());
    let mut open = true;
    let button = picker_button;
    match picker {
        ModelPicker::Fold { dataset, hosts } => {
            ui.horizontal_wrapped(|ui| {
                for host in hosts.iter() {
                    if button(ui, host, drawn) {
                        chosen.push(ModelAction::Edit(Edit::Fold {
                            dataset: dataset.clone(),
                            into: host.clone(),
                        }));
                        open = false;
                    }
                }
            });
        }
        ModelPicker::Role { dataset } => {
            ui.horizontal_wrapped(|ui| {
                for role in ROLES {
                    if button(ui, role_label(role), drawn) {
                        chosen.push(ModelAction::ChangeRole {
                            dataset: dataset.clone(),
                            role,
                        });
                        open = false;
                    }
                }
            });
        }
        ModelPicker::Keys {
            relationship,
            from,
            to,
            from_choices,
            to_choices,
            from_columns,
            to_columns,
        } => {
            for (i, (f, t)) in from_columns
                .iter_mut()
                .zip(to_columns.iter_mut())
                .enumerate()
            {
                ui.horizontal(|ui| {
                    combo(ui, ("model-keys-from", i), from, f, from_choices);
                    ui.label("=");
                    combo(ui, ("model-keys-to", i), to, t, to_choices);
                });
            }
            ui.horizontal(|ui| {
                if button(ui, "Change keys", drawn) {
                    chosen.push(ModelAction::Edit(Edit::ChangeKeys {
                        relationship: relationship.clone(),
                        from_columns: from_columns.clone(),
                        to_columns: to_columns.clone(),
                    }));
                    open = false;
                }
                if button(ui, "Cancel", drawn) {
                    open = false;
                }
            });
        }
        ModelPicker::Retype {
            dataset,
            fields,
            labels,
            field,
            finetype,
            typed_for,
        } => {
            let names: Vec<String> = fields.iter().map(|(f, _)| f.clone()).collect();
            ui.horizontal(|ui| {
                combo(ui, "model-retype-field", "Column", field, &names);
            });
            // Another column chosen, by the combo or otherwise: offer the type it has now.
            if field != typed_for {
                *finetype = offered_label(fields, labels, field);
                typed_for.clone_from(field);
            }
            ui.horizontal(|ui| {
                combo(ui, "model-retype-type", "Type", finetype, labels);
            });
            ui.horizontal(|ui| {
                if button(ui, "Retype", drawn) {
                    chosen.push(ModelAction::Edit(Edit::Retype {
                        dataset: dataset.clone(),
                        field: field.clone(),
                        finetype: finetype.clone(),
                    }));
                    open = false;
                }
                if button(ui, "Cancel", drawn) {
                    open = false;
                }
            });
        }
        ModelPicker::AddJoin {
            from,
            from_choices,
            tables,
            to,
            from_column,
            to_column,
        } => {
            let names: Vec<String> = tables.iter().map(|(t, _)| t.clone()).collect();
            ui.horizontal(|ui| {
                combo(ui, "model-join-from", from, from_column, from_choices);
            });
            let before = to.clone();
            ui.horizontal(|ui| {
                combo(ui, "model-join-to-table", "to", to, &names);
            });
            let choices = to_columns(tables, to);
            if *to != before {
                *to_column = shared_column(std::slice::from_ref(from_column), &choices).1;
            }
            ui.horizontal(|ui| {
                combo(ui, "model-join-to", to.as_str(), to_column, &choices);
            });
            ui.horizontal(|ui| {
                if button(ui, "Add join", drawn) {
                    chosen.push(ModelAction::Edit(Edit::AddJoin {
                        from: from.clone(),
                        from_columns: vec![from_column.clone()],
                        to: to.clone(),
                        to_columns: vec![to_column.clone()],
                    }));
                    open = false;
                }
                if button(ui, "Cancel", drawn) {
                    open = false;
                }
            });
        }
    }
    open
}

/// One of a picker's buttons, recorded in `drawn`; answers whether it was clicked.
fn picker_button(ui: &mut egui::Ui, label: &str, drawn: &mut InspectorDrawn) -> bool {
    let response = ui.button(label);
    drawn.picker.push(label.to_string());
    drawn.picker_rects.push(response.rect);
    response.clicked()
}

/// A labelled drop-down that sets `value` to one of `choices`.
fn combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    value: &mut String,
    choices: &[String],
) {
    ui.label(egui::RichText::new(label).font(ui_font()));
    egui::ComboBox::from_id_salt(id)
        .selected_text(value.as_str())
        .show_ui(ui, |ui| {
            for c in choices {
                ui.selectable_value(value, c.clone(), c);
            }
        });
}

/// Rows read back from the Protocol's database, every value as text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rows {
    /// The columns' names, in order.
    pub columns: Vec<String>,
    /// Each row's values, `None` for a NULL.
    pub rows: Vec<Vec<Option<String>>>,
}

impl Rows {
    /// The value of `column` in row `row`, when both exist and it is not NULL.
    #[must_use]
    pub fn value(&self, row: usize, column: &str) -> Option<&str> {
        let at = self.columns.iter().position(|c| c == column)?;
        self.rows.get(row)?.get(at)?.as_deref()
    }
}

/// `name` as a SQL identifier.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `s` as a SQL string literal.
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A connection with the Protocol in `dir`'s database attached read-only as the default catalogue.
/// The database is held only while the connection lives.
fn attach(dir: &Path) -> Result<duckdb::Connection, String> {
    let db = arc::spec::protocol_db_path(dir).map_err(|e| e.to_string())?;
    if !db.is_file() {
        return Err(format!(
            "the Protocol has not been run, so {} does not exist yet",
            db.display()
        ));
    }
    let conn = duckdb::Connection::open_in_memory().map_err(|e| e.to_string())?;
    conn.execute_batch(&format!(
        "ATTACH {} AS __run (READ_ONLY); USE __run",
        literal(&db.to_string_lossy())
    ))
    .map_err(|e| format!("{}: {e}", db.display()))?;
    Ok(conn)
}

/// Run `sql` against the database of the Protocol in `dir` and read back what it selects.
///
/// # Errors
///
/// The reason when the database cannot be opened or the statement fails.
pub fn query(dir: &Path, sql: &str) -> Result<Rows, String> {
    let conn = attach(dir)?;
    let mut stmt = conn
        .prepare(&format!("SELECT COLUMNS(*)::VARCHAR FROM ({sql})"))
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    let columns = rows
        .as_ref()
        .map(duckdb::Statement::column_names)
        .unwrap_or_default();
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let mut values = Vec::with_capacity(columns.len());
        for i in 0..columns.len() {
            values.push(row.get::<_, Option<String>>(i).map_err(|e| e.to_string())?);
        }
        out.push(values);
    }
    Ok(Rows { columns, rows: out })
}

/// The statement that lists `dataset`'s silver rows without their lineage columns: what Open in
/// grid shows for a table.
#[must_use]
pub fn silver_rows_sql(dataset: &str) -> String {
    format!(
        "SELECT * EXCLUDE ({}) FROM {}",
        LINEAGE.join(", "),
        quoted(dataset)
    )
}

/// Write what `sql` selects from the Protocol in `dir`'s database to a Parquet file named `name`
/// in the Protocol's `.grid` folder, and answer its path. The grid opens that file as it opens any
/// Parquet file, so the database is free again for the next run.
///
/// # Errors
///
/// The reason when the folder cannot be made, the database cannot be opened or the copy fails.
pub fn snapshot(dir: &Path, name: &str, sql: &str) -> Result<PathBuf, String> {
    let folder = dir.join(GRID_DIR);
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let file = folder.join(format!("{name}.parquet"));
    let conn = attach(dir)?;
    conn.execute_batch(&format!(
        "COPY ({sql}) TO {} (FORMAT parquet)",
        literal(&file.to_string_lossy())
    ))
    .map_err(|e| format!("could not copy the rows to {}: {e}", file.display()))?;
    Ok(file)
}

/// One column of a table, as its inspector lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDetail {
    /// The column's name.
    pub name: String,
    /// What its chip says: the FineType title of its label, the label itself when the catalogue
    /// gives no title, or `untyped`.
    pub chip: String,
    /// How many of its values failed to cast.
    pub failed: u64,
}

/// What a table's inspector shows.
#[derive(Debug, Clone, PartialEq)]
pub struct TableDetails {
    /// The table's name.
    pub dataset: String,
    /// Its role.
    pub role: Role,
    /// The table it is folded into, if any.
    pub folded_into: Option<String>,
    /// Whether it is left out of the model.
    pub excluded: bool,
    /// Its columns.
    pub columns: Vec<ColumnDetail>,
    /// The tables folded into it.
    pub folded: Vec<String>,
    /// The first of its failed values, with their lineage, or why they could not be read. `None`
    /// for a table with no tables of its own.
    pub failed: Option<Result<Rows, String>>,
}

/// What a join's inspector shows.
#[derive(Debug, Clone, PartialEq)]
pub struct JoinDetails {
    /// The relationship's name.
    pub relationship: String,
    /// `transactions (customer_id) → customers (customer_id) · N:1`.
    pub keys: String,
    /// Its grade.
    pub grade: Grade,
    /// Who settled it.
    pub decided_by: DecidedBy,
    /// Each piece of evidence, as a line.
    pub evidence: Vec<String>,
    /// `Verified on the full data`, `Sample only`, or that nothing was measured.
    pub verified: String,
    /// Its warning, if it carries one.
    pub warning: Option<String>,
    /// A sample of the joined rows, or why they could not be read.
    pub preview: Result<Rows, String>,
}

/// What the inspector shows for the selection.
#[derive(Debug, Clone, PartialEq)]
pub enum Inspected {
    Join(JoinDetails),
    Table(TableDetails),
}

/// The words for how much of the data a join's evidence was measured on.
#[must_use]
pub fn verified_line(verified: Verified) -> &'static str {
    match verified {
        Verified::Full => "Verified on the full data",
        Verified::Sample => "Sample only",
        Verified::Unmeasured => "Added by hand: no discovery pass has verified it",
    }
}

/// Read what the inspector shows for `selection` from `model` and the Protocol's database in
/// `dir`. `None` when the model holds no table or join of the selection's name.
#[must_use]
pub fn inspect(
    dir: &Path,
    model: &Model,
    selection: &Selection,
    catalogue: Option<&SchemaCatalogue>,
) -> Option<Inspected> {
    match selection {
        Selection::Relationship(name) => {
            let r = model.relationships.iter().find(|r| r.name == *name)?;
            let e = &r.meta.evidence;
            let types = match (&e.child_type, &e.parent_type) {
                (Some(c), Some(p)) => format!("{c} \u{2192} {p}"),
                _ => "not recorded".to_string(),
            };
            let evidence = vec![
                format!("Value overlap: {:.0}%", e.value_overlap * 100.0),
                format!("Orphans: {} of {} {} rows", e.orphans, e.child_rows, r.from),
                format!(
                    "Key types: {types}, {}",
                    if e.types_agree { "agree" } else { "differ" }
                ),
                format!("Name similarity: {:.2}", e.name_similarity),
            ];
            Some(Inspected::Join(JoinDetails {
                relationship: r.name.clone(),
                keys: format!(
                    "{} ({}) \u{2192} {} ({}) \u{b7} {}",
                    r.from,
                    r.from_columns.join(", "),
                    r.to,
                    r.to_columns.join(", "),
                    r.meta.cardinality
                ),
                grade: r.meta.grade,
                decided_by: r.meta.decided_by,
                evidence,
                verified: verified_line(r.meta.verified).to_string(),
                warning: r.meta.warning.clone(),
                preview: query(
                    dir,
                    &brightfield_protocol::preview::join_preview_sql(model, name, PREVIEW_ROWS),
                ),
            }))
        }
        Selection::Dataset(name) => {
            let d = model.datasets.iter().find(|d| d.name == *name)?;
            let failed_table = quoted(&format!("{name}__failed"));
            let (counts, failed) = if materialised(model, name) {
                let counts = query(
                    dir,
                    &format!(
                        "SELECT \"column\", count(*) AS n FROM {failed_table} GROUP BY \"column\""
                    ),
                )
                .unwrap_or_default();
                let failed = query(
                    dir,
                    &format!(
                        "SELECT {}, \"column\", \"value\" FROM {failed_table} \
                         ORDER BY \"__source_file\", \"__source_part\", \"__source_row\", \"column\" \
                         LIMIT {PREVIEW_ROWS}",
                        LINEAGE.join(", ")
                    ),
                );
                (counts, Some(failed))
            } else {
                (Rows::default(), None)
            };
            let failed_in = |column: &str| {
                counts
                    .rows
                    .iter()
                    .find(|row| row.first().and_then(Option::as_deref) == Some(column))
                    .and_then(|row| row.get(1)?.as_deref()?.parse().ok())
                    .unwrap_or(0)
            };
            let columns = d
                .fields
                .iter()
                .filter(|f| !LINEAGE.contains(&f.name.as_str()))
                .map(|f| ColumnDetail {
                    name: f.name.clone(),
                    chip: match f.meta.finetype.as_deref() {
                        Some(label) => catalogue
                            .and_then(|c| c.title(label))
                            .unwrap_or(label)
                            .to_string(),
                        None => "untyped".to_string(),
                    },
                    failed: failed_in(&f.name),
                })
                .collect();
            Some(Inspected::Table(TableDetails {
                dataset: d.name.clone(),
                role: d.meta.role,
                folded_into: d.meta.folded_into.clone(),
                excluded: d.meta.excluded,
                columns,
                folded: model
                    .datasets
                    .iter()
                    .filter(|f| f.meta.folded_into.as_deref() == Some(name.as_str()))
                    .map(|f| f.name.clone())
                    .collect(),
                failed,
            }))
        }
    }
}

/// What the inspector drew in the last frame it was drawn in, for a test to read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InspectorDrawn {
    /// Its heading.
    pub title: String,
    /// Each line of text it drew under the heading, in order.
    pub lines: Vec<String>,
    /// The words on each button it drew, in order.
    pub actions: Vec<String>,
    /// Where each of those buttons was laid out, in the order of [`InspectorDrawn::actions`]. The
    /// role picker, which [`InspectorDrawn::actions`] names first as "Change role", is not a button
    /// and has no rect here.
    pub button_rects: Vec<egui::Rect>,
    /// The columns of the rows table it drew, and how many rows it held.
    pub table: Option<(Vec<String>, usize)>,
    /// The words on each of the open picker's buttons, in order.
    pub picker: Vec<String>,
    /// Where each of those buttons was laid out, in the order of [`InspectorDrawn::picker`].
    pub picker_rects: Vec<egui::Rect>,
}

/// The name a role is written as.
#[must_use]
pub fn role_label(role: Role) -> &'static str {
    match role {
        Role::Entity => "entity",
        Role::Event => "event",
        Role::Reference => "reference",
        Role::Bridge => "bridge",
        Role::Snapshot => "snapshot",
        Role::Unclear => "unclear",
    }
}

/// Draw the inspector for `inspected` into `ui`, offering `actions` and, under them, the open
/// `picker`, and push each one the analyst chose onto `chosen`. A picker that is answered or
/// cancelled is taken out of `picker`.
pub fn ui(
    ui: &mut egui::Ui,
    inspected: &Inspected,
    actions: &[ModelAction],
    picker: &mut Option<ModelPicker>,
    mode: Mode,
    chosen: &mut Vec<ModelAction>,
    drawn: &mut InspectorDrawn,
) {
    let sem = semantic(mode.is_dark());
    let primary = chrome::colour(sem.text.primary);
    let secondary = chrome::colour(sem.text.secondary);
    let warning = chrome::colour(sem.role(meridian_design::Role::Warning).foreground.base);
    let line = |ui: &mut egui::Ui, drawn: &mut InspectorDrawn, text: String, ink| {
        ui.add(egui::Label::new(egui::RichText::new(&text).font(ui_font()).color(ink)).wrap());
        drawn.lines.push(text);
    };
    egui::ScrollArea::vertical()
        .id_salt("model-inspector")
        .auto_shrink([false, false])
        .show(ui, |ui| match inspected {
            Inspected::Join(j) => {
                drawn.title = format!("Join {}", j.relationship);
                heading(ui, &drawn.title, primary);
                line(ui, drawn, j.keys.clone(), primary);
                let grade = match j.grade {
                    Grade::Likely => "likely",
                    Grade::Possible => "possible",
                };
                let who = match j.decided_by {
                    DecidedBy::Dovetail => "found by discovery",
                    DecidedBy::Analyst => "settled by the analyst",
                };
                line(ui, drawn, format!("Grade: {grade}, {who}"), primary);
                line(ui, drawn, j.verified.clone(), secondary);
                for e in &j.evidence {
                    line(ui, drawn, e.clone(), secondary);
                }
                if let Some(w) = &j.warning {
                    line(ui, drawn, w.clone(), warning);
                }
                ui.add_space(spacing::SPACE_2);
                buttons(ui, actions, chosen, drawn);
                open_picker(ui, picker, chosen, drawn);
                ui.add_space(spacing::SPACE_2);
                line(
                    ui,
                    drawn,
                    format!("Joined rows, a sample of up to {PREVIEW_ROWS}"),
                    primary,
                );
                rows_ui(ui, "join-preview", &j.preview, mode, drawn);
            }
            Inspected::Table(t) => {
                drawn.title = format!("Table {}", t.dataset);
                heading(ui, &drawn.title, primary);
                role_picker(ui, t, chosen, drawn);
                if let Some(host) = &t.folded_into {
                    line(ui, drawn, format!("Folded into {host}"), secondary);
                }
                if t.excluded {
                    line(ui, drawn, "Left out of the model".to_string(), secondary);
                }
                if !t.folded.is_empty() {
                    line(
                        ui,
                        drawn,
                        format!("Folded in: {}", t.folded.join(", ")),
                        secondary,
                    );
                }
                ui.add_space(spacing::SPACE_2);
                for c in &t.columns {
                    column_row(ui, c, mode, drawn);
                }
                ui.add_space(spacing::SPACE_2);
                buttons(ui, actions, chosen, drawn);
                open_picker(ui, picker, chosen, drawn);
                if let Some(failed) = &t.failed {
                    ui.add_space(spacing::SPACE_2);
                    line(
                        ui,
                        drawn,
                        format!("Values that did not cast, the first {PREVIEW_ROWS}"),
                        primary,
                    );
                    rows_ui(ui, "failed-rows", failed, mode, drawn);
                }
            }
        });
}

/// The open picker, if any, under the buttons; closed once it is answered or cancelled.
fn open_picker(
    ui: &mut egui::Ui,
    picker: &mut Option<ModelPicker>,
    chosen: &mut Vec<ModelAction>,
    drawn: &mut InspectorDrawn,
) {
    if let Some(p) = picker {
        ui.add_space(spacing::SPACE_2);
        if !picker_ui(ui, p, chosen, drawn) {
            *picker = None;
        }
    }
}

fn heading(ui: &mut egui::Ui, text: &str, ink: egui::Color32) {
    ui.label(
        egui::RichText::new(text)
            .font(ui_font())
            .strong()
            .color(ink),
    );
    ui.add_space(spacing::SPACE_1);
}

fn buttons(
    ui: &mut egui::Ui,
    actions: &[ModelAction],
    chosen: &mut Vec<ModelAction>,
    drawn: &mut InspectorDrawn,
) {
    ui.horizontal_wrapped(|ui| {
        for action in actions {
            let label = action.label();
            let response = ui.button(&label);
            if response.clicked() {
                chosen.push(action.clone());
            }
            drawn.button_rects.push(response.rect);
            drawn.actions.push(label);
        }
    });
}

/// The role, as a picker of the six: choosing another is a Change role action.
fn role_picker(
    ui: &mut egui::Ui,
    t: &TableDetails,
    chosen: &mut Vec<ModelAction>,
    drawn: &mut InspectorDrawn,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Role").font(ui_font()));
        let mut role = t.role;
        egui::ComboBox::from_id_salt(("model-role", &t.dataset))
            .selected_text(role_label(role))
            .show_ui(ui, |ui| {
                for r in ROLES {
                    ui.selectable_value(&mut role, r, role_label(r));
                }
            });
        if role != t.role {
            chosen.push(ModelAction::ChangeRole {
                dataset: t.dataset.clone(),
                role,
            });
        }
    });
    drawn.lines.push(format!("Role: {}", role_label(t.role)));
    drawn.actions.push(
        ModelAction::ChangeRole {
            dataset: t.dataset.clone(),
            role: t.role,
        }
        .label(),
    );
}

/// A column's row: its name, its type as a chip, and how many of its values failed.
fn column_row(ui: &mut egui::Ui, c: &ColumnDetail, mode: Mode, drawn: &mut InspectorDrawn) {
    let sem = semantic(mode.is_dark());
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(&c.name)
                .font(mono_font())
                .color(chrome::colour(sem.text.primary)),
        );
        let font = ui_font();
        let width = chrome::chip_width(ui, &c.chip, font.clone());
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(width, control::HEIGHT_XS), egui::Sense::hover());
        chrome::chip(ui, rect, &c.chip, font, false, egui::Sense::hover(), mode);
        if c.failed > 0 {
            ui.label(
                egui::RichText::new(format!("{} failed", c.failed))
                    .font(ui_font())
                    .color(chrome::colour(
                        sem.role(meridian_design::Role::Warning).foreground.base,
                    )),
            );
        }
    });
    let mut text = format!("{} [{}]", c.name, c.chip);
    if c.failed > 0 {
        text.push_str(&format!(" {} failed", c.failed));
    }
    drawn.lines.push(text);
}

/// Rows read from the database as a grid of mono text, or why they could not be read.
fn rows_ui(
    ui: &mut egui::Ui,
    id: &str,
    rows: &Result<Rows, String>,
    mode: Mode,
    drawn: &mut InspectorDrawn,
) {
    let sem = semantic(mode.is_dark());
    let primary = chrome::colour(sem.text.primary);
    let secondary = chrome::colour(sem.text.secondary);
    match rows {
        Err(e) => {
            let text = format!("Could not read the rows: {e}");
            ui.add(
                egui::Label::new(egui::RichText::new(&text).font(ui_font()).color(secondary))
                    .wrap(),
            );
            drawn.lines.push(text);
        }
        Ok(rows) if rows.rows.is_empty() => {
            ui.label(egui::RichText::new("None").font(ui_font()).color(secondary));
            drawn.table = Some((rows.columns.clone(), 0));
        }
        Ok(rows) => {
            egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
                egui::Grid::new(id).striped(true).show(ui, |ui| {
                    for c in &rows.columns {
                        ui.label(egui::RichText::new(c).font(mono_font()).color(secondary));
                    }
                    ui.end_row();
                    for row in &rows.rows {
                        for value in row {
                            ui.label(
                                egui::RichText::new(value.as_deref().unwrap_or("NULL"))
                                    .font(mono_font())
                                    .color(primary),
                            );
                        }
                        ui.end_row();
                    }
                });
            });
            drawn.table = Some((rows.columns.clone(), rows.rows.len()));
        }
    }
}
