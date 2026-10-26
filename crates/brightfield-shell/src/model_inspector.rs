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
//! click, a test and a later verb reach the same change.

use std::path::{Path, PathBuf};

use brightfield_engine::semantic::SchemaCatalogue;
use brightfield_ossie::{DecidedBy, Grade, Model, Role, Verified};
use brightfield_render::model_scene::Selection;
use brightfield_workbench::chrome;
use meridian_design::{control, semantic, spacing};

use crate::design::Mode;
use crate::protocol::{mono_font, ui_font};

/// How many joined rows the preview samples, and how many failed values a table lists.
pub const PREVIEW_ROWS: usize = 20;

/// The width of the inspector, beside the view it inspects.
pub const INSPECTOR_WIDTH: f32 = 360.0;

/// The lineage columns every loaded and silver table starts with.
const LINEAGE: [&str; 3] = ["__source_file", "__source_part", "__source_row"];

/// The folder inside the Protocol that holds the snapshots opened in the grid.
const GRID_DIR: &str = ".grid";

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
    /// Give one of a table's columns another type.
    RetypeColumn(String),
    /// Open the selection's rows in the grid: a table's silver rows, or a join's joined rows.
    OpenInGrid(Selection),
}

impl ModelAction {
    /// The words on the action's button.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            ModelAction::Keep(_) => "Keep",
            ModelAction::Drop(_) => "Drop",
            ModelAction::ChangeKeys(_) => "Change keys\u{2026}",
            ModelAction::ShowInFile => "Show in file",
            ModelAction::ChangeRole { .. } => "Change role",
            ModelAction::FoldInto(_) => "Fold into\u{2026}",
            ModelAction::Unfold(_) => "Unfold",
            ModelAction::LeaveOut(_) => "Leave out",
            ModelAction::RetypeColumn(_) => "Retype column\u{2026}",
            ModelAction::OpenInGrid(_) => "Open in grid",
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

/// The buttons a table's inspector offers, in order, besides its role picker.
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
    if !d.meta.excluded {
        actions.push(ModelAction::LeaveOut(name.clone()));
    }
    actions.push(ModelAction::RetypeColumn(name.clone()));
    actions.push(ModelAction::ShowInFile);
    if materialised(model, dataset) {
        actions.push(ModelAction::OpenInGrid(Selection::Dataset(name)));
    }
    actions
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
        Verified::Unmeasured => "Not measured: the join was written by hand",
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
                    &brightfield_ossie::join_preview_sql(model, name, PREVIEW_ROWS),
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
    /// The columns of the rows table it drew, and how many rows it held.
    pub table: Option<(Vec<String>, usize)>,
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

/// Draw the inspector for `inspected` into `ui`, offering `actions`, and push each one the
/// analyst chose onto `chosen`.
pub fn ui(
    ui: &mut egui::Ui,
    inspected: &Inspected,
    actions: &[ModelAction],
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
            drawn.actions.push(action.label().to_string());
            if ui.button(action.label()).clicked() {
                chosen.push(action.clone());
            }
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
        .label()
        .to_string(),
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
