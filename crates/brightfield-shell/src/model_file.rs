//! Reading the Ossie file as the analyst edits it by hand in the model pane's file view.
//!
//! [`read_hand_edit`] reads the edited text against the model the pane holds. A text that is not
//! YAML, breaks the Ossie schema or is not a model Brightfield reads is refused with the reason
//! and, where the text says, the line. A text Brightfield reads becomes the next model, and each
//! dataset, field and relationship that differs from the one of the same name in the current
//! model, or is new, is marked as the analyst's; what the edit left alone keeps who decided it. A
//! relationship's evidence and warning are measured after each run, not decided: a hand edit
//! neither changes them nor is judged by them.
//!
//! What the edit deletes stays deleted when discovery runs again (`hold_deletions`). What counts
//! as deleted is what the text the edit started from held and the edited text does not: the text
//! the editor last took from disk, or the editor's own text as last read, whichever came later.
//! What the model gained since, from a discovery pass or a save, is kept as the model holds it,
//! since the analyst never saw it to delete it. A relationship the edit removed goes into the
//! model's rejected joins as the analyst's, as Drop does, unless the text still holds the same
//! join under another name, which is a rename. A rejected join leaves the rejected list when a
//! relationship of its name is in the model again, so a name typed through on the way to another
//! is not held against it. A dataset the edit removed stays in the model, left out, as the
//! analyst decided, whoever had decided it before, so the diagram shows it left out and a later
//! pass does not bring it back.
//!
//! Only what a pass could propose is held: a dataset named for one of the Protocol's source
//! tables, and a join between two of them. Anything else deleted is simply gone, such as a name
//! typed on the way to renaming a table.
//!
//! What the text names that Brightfield cannot act on is not refused. A relationship naming a
//! table the model does not hold is set aside in [`Kept`]: left out of the model the diagram is
//! drawn from and the silver steps are written for, and written back into the file by
//! [`written`], after the model's own relationships, since the analyst wrote it. A role outside
//! the six is ignored: the table keeps the role it had, in the diagram and in the file, so the
//! file always reads. Metrics ride in the model as it is, since nothing Brightfield builds reads
//! them. Each of these is one line of [`warnings`].
//!
//! The file is written as [`brightfield_ossie::write`] emits it, so YAML comments and the
//! analyst's own layout do not reach the disk; the editor keeps the text as typed until it lets
//! go of the keyboard.

use brightfield_ossie::{
    line_of, Dataset, DatasetMeta, DecidedBy, Element, Field, FieldMeta, Model, ReadError,
    Relationship, RelationshipMeta, Role, VENDOR,
};
use serde_yaml::Value;

use crate::model_inspector::role_label;

/// What an edited text holds that Brightfield does not act on.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Kept {
    /// Relationships that name a table the model does not hold, written back into the file.
    pub relationships: Vec<Relationship>,
    /// Roles that are not one of the six, as the editor's text holds them. They are only
    /// warned of: the file holds the role each table kept.
    pub roles: Vec<IgnoredRole>,
}

/// A table's role as typed, when it is not one of the six.
#[derive(Debug, Clone, PartialEq)]
pub struct IgnoredRole {
    /// The table.
    pub dataset: String,
    /// The role as typed, a JSON value.
    pub written: serde_json::Value,
    /// The role the table keeps: the one it had before the edit, or unclear.
    pub shown: Role,
}

/// A hand edit Brightfield reads: the model to act on, and what was set aside from it.
#[derive(Debug, Clone, PartialEq)]
pub struct HandEdit {
    /// The model the diagram is drawn from and the silver steps are written for.
    pub model: Model,
    /// What the file holds besides, written back with it.
    pub kept: Kept,
    /// The model as the text itself holds it, relationships set aside included, before what it
    /// deleted is held: what the next edit of the same text starts from.
    pub typed: Model,
}

/// Read `text`, the Ossie file as the analyst edited it, against `current`, the model the pane
/// holds, `base`, the model of the text the edit started from, and `sources`, the names of the
/// Protocol's source tables (see the module).
///
/// # Errors
///
/// Why the text is not a model Brightfield reads, headed with its line where the text gives one:
/// `Line 3: …`.
pub fn read_hand_edit(
    current: &Model,
    base: &Model,
    sources: &[&str],
    text: &str,
) -> Result<HandEdit, String> {
    let mut doc: Value = serde_yaml::from_str(text).map_err(|e| yaml_error(text, &e))?;
    if let Some(issue) = brightfield_ossie::validate(text).into_iter().next() {
        return Err(match issue_line(text, &doc, &issue.path) {
            Some(line) => format!("Line {line}: {}", issue.message),
            None => issue.message,
        });
    }
    let roles = set_aside_roles(current, &mut doc);
    let readable = if roles.is_empty() {
        text.to_string()
    } else {
        match roles_replaced_in_place(text, &roles) {
            Some(replaced) => replaced,
            // A layout the line-for-line swap does not follow: read the whole document again,
            // whose lines may then differ from the editor's.
            None => serde_yaml::to_string(&doc).map_err(|e| e.to_string())?,
        }
    };
    let mut model = brightfield_ossie::read(&readable).map_err(|e| read_error(&e))?;
    let typed = model.clone();
    let tables: Vec<String> = model.datasets.iter().map(|d| d.name.clone()).collect();
    let (acted, unjoined): (Vec<Relationship>, Vec<Relationship>) =
        std::mem::take(&mut model.relationships)
            .into_iter()
            .partition(|r| tables.contains(&r.from) && tables.contains(&r.to));
    model.relationships = acted;
    keep_measurements(current, &mut model);
    mark_changed(current, &mut model);
    hold_deletions(current, base, sources, &mut model, &unjoined);
    // A table the edit deleted is held, left out, so a join to it names a table of the model
    // again and is acted on rather than set aside
    // (`a_table_deleted_by_hand_keeps_its_joins_in_the_model`).
    let tables: Vec<String> = model.datasets.iter().map(|d| d.name.clone()).collect();
    let (rejoined, unjoined): (Vec<Relationship>, Vec<Relationship>) = unjoined
        .into_iter()
        .partition(|r| tables.contains(&r.from) && tables.contains(&r.to));
    if !rejoined.is_empty() {
        model.relationships.extend(rejoined);
        // Both read `current` alone, so running them again leaves what they already did as it is.
        keep_measurements(current, &mut model);
        mark_changed(current, &mut model);
    }
    Ok(HandEdit {
        model,
        kept: Kept {
            relationships: unjoined,
            roles,
        },
        typed,
    })
}

/// What of `kept` still stands beside `model` once the file is written for it and the editor
/// holds the file: each relationship set aside that no relationship of the model now shares a
/// name with. No ignored role stands, since the file holds none.
#[must_use]
pub fn still_kept(model: &Model, kept: &Kept) -> Kept {
    Kept {
        relationships: kept
            .relationships
            .iter()
            .filter(|k| !model.relationships.iter().any(|r| r.name == k.name))
            .cloned()
            .collect(),
        roles: Vec::new(),
    }
}

/// The Ossie file for `model` with the relationships `kept` sets aside written back after the
/// model's own.
#[must_use]
pub fn written(model: &Model, kept: &Kept) -> String {
    let mut full = model.clone();
    full.relationships
        .extend(kept.relationships.iter().cloned());
    brightfield_ossie::write(&full)
}

/// One line for each thing the file holds that Brightfield does not act on: the relationships
/// and roles set aside in `kept`, then the metrics in `model`.
#[must_use]
pub fn warnings(model: &Model, kept: &Kept) -> Vec<String> {
    let mut said = Vec::new();
    for r in &kept.relationships {
        let mut missing: Vec<&str> = [r.from.as_str(), r.to.as_str()]
            .into_iter()
            .filter(|t| !model.datasets.iter().any(|d| d.name == *t))
            .collect();
        missing.dedup();
        let tables = match missing.as_slice() {
            [one] => format!("{one}, which is not a table"),
            _ => format!("{}, which are not tables", missing.join(" and ")),
        };
        said.push(format!(
            "{} joins {tables} in the model: it stays in the file, and is left out of the \
             diagram and the silver tables",
            r.name
        ));
    }
    for role in &kept.roles {
        let typed = match &role.written {
            serde_json::Value::String(typed) => typed.clone(),
            other => other.to_string(),
        };
        said.push(format!(
            "role '{typed}' is not one of the six; {} stays {}",
            role.dataset,
            role_label(role.shown)
        ));
    }
    for metric in &model.metrics {
        said.push(format!(
            "The metric {} stays in the file: Brightfield does not act on metrics",
            metric.name
        ));
    }
    for d in &model.datasets {
        for metric in &d.metrics {
            said.push(format!(
                "The metric {} on {} stays in the file: Brightfield does not act on metrics",
                metric.name, d.name
            ));
        }
    }
    said
}

/// The 1-based line of the `name:` entry the text gives `selection`, when it gives one.
#[must_use]
pub fn selection_line(
    text: &str,
    selection: &brightfield_render::model_scene::Selection,
) -> Option<usize> {
    use brightfield_render::model_scene::Selection;
    match selection {
        Selection::Dataset(name) => line_of(text, Element::Dataset(name)),
        Selection::Relationship(name) => line_of(text, Element::Relationship(name)),
    }
}

/// A YAML error, at its line. A text cut off mid-value is reported one line past its end, so the
/// line is held to the text's last, as [`brightfield_ossie::read`] holds it.
fn yaml_error(text: &str, e: &serde_yaml::Error) -> String {
    match e.location() {
        Some(at) => format!("Line {}: {e}", at.line().min(text.lines().count().max(1))),
        None => e.to_string(),
    }
}

fn read_error(e: &ReadError) -> String {
    if e.line == 0 {
        e.message.clone()
    } else {
        format!("Line {}: {}", e.line, e.message)
    }
}

/// The line a schema issue at JSON pointer `path` stands on: the `name:` entry of the dataset,
/// field or relationship it is in, or the top-level key it is under.
fn issue_line(text: &str, doc: &Value, path: &str) -> Option<usize> {
    let at: Vec<&str> = path.split('/').skip(1).collect();
    let item = |list: &str, i: &str| doc.get(list)?.get(i.parse::<usize>().ok()?);
    let name = |v: &Value| v.get("name")?.as_str().map(str::to_string);
    match at.as_slice() {
        ["datasets", i, "fields", j, ..] => {
            let dataset = item("datasets", i)?;
            let field = dataset.get("fields")?.get(j.parse::<usize>().ok()?)?;
            line_of(text, Element::Field(&name(dataset)?, &name(field)?))
        }
        ["datasets", i, ..] => line_of(text, Element::Dataset(&name(item("datasets", i)?)?)),
        ["relationships", i, ..] => line_of(
            text,
            Element::Relationship(&name(item("relationships", i)?)?),
        ),
        [key, ..] => text
            .lines()
            .position(|l| l.starts_with(&format!("{key}:")))
            .map(|i| i + 1),
        [] => None,
    }
}

/// The `data` of the Meridian entry of dataset `dataset` in `doc`, if it has one.
fn meridian_data<'a>(doc: &'a mut Value, dataset: &str) -> Option<Option<&'a mut Value>> {
    let entry = doc
        .get_mut("datasets")?
        .as_sequence_mut()?
        .iter_mut()
        .find(|d| d.get("name").and_then(Value::as_str) == Some(dataset))?
        .get_mut("custom_extensions")?
        .as_sequence_mut()?
        .iter_mut()
        .find(|e| e.get("vendor_name").and_then(Value::as_str) == Some(VENDOR))?;
    Some(entry.get_mut("data"))
}

/// `text` with each of `roles` swapped, on the line its table's Meridian entry stands on, for
/// the role the table keeps, so the text reads and every other line stays where the editor shows
/// it. `None` when an entry is not on one line of its own.
fn roles_replaced_in_place(text: &str, roles: &[IgnoredRole]) -> Option<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for role in roles {
        let start = line_of(text, Element::Dataset(&role.dataset))? - 1;
        // `- name: …` opens the item at its own indent; a `name:` under the dash stands two in.
        let item_indent = if lines[start].trim_start().starts_with("- ") {
            indent(&lines[start])
        } else {
            indent(&lines[start]).saturating_sub(2)
        };
        let end = (start + 1..lines.len())
            .find(|&i| {
                let l = lines[i].trim_start();
                !l.is_empty()
                    && !l.starts_with('#')
                    && (indent(&lines[i]) < item_indent
                        || (indent(&lines[i]) == item_indent && l.starts_with("- ")))
            })
            .unwrap_or(lines.len());
        let swapped = (start..end).find_map(|i| {
            let line = &lines[i];
            let lead = line.len() - line.trim_start().len();
            let body = line.trim_start();
            let (dash, entry) = match body.strip_prefix("- ") {
                Some(rest) => ("- ", rest),
                None => ("", body),
            };
            if !entry.starts_with("data:") {
                return None;
            }
            let parsed: Value = serde_yaml::from_str(entry).ok()?;
            let json = parsed.get("data")?.as_str()?;
            let serde_json::Value::Object(mut meta) = serde_json::from_str(json).ok()? else {
                return None;
            };
            if meta.get("role") != Some(&role.written) {
                return None;
            }
            meta.insert(
                "role".to_string(),
                serde_json::Value::String(role_label(role.shown).to_string()),
            );
            let mut swapped = serde_yaml::Mapping::new();
            swapped.insert(
                Value::String("data".to_string()),
                Value::String(serde_json::Value::Object(meta).to_string()),
            );
            let entry = serde_yaml::to_string(&swapped).ok()?;
            let entry = entry.trim_end();
            // A value the writer folds over several lines would move every line after it.
            (!entry.contains('\n')).then(|| (i, format!("{}{dash}{entry}", &line[..lead])))
        });
        let (i, line) = swapped?;
        lines[i] = line;
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Set aside each role in `doc` that is not one of the six, putting in its place the one its
/// table has in `current` (or unclear, for a table `current` does not hold), so the rest reads.
fn set_aside_roles(current: &Model, doc: &mut Value) -> Vec<IgnoredRole> {
    let names: Vec<String> = doc
        .get("datasets")
        .and_then(Value::as_sequence)
        .map(|ds| {
            ds.iter()
                .filter_map(|d| d.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut kept = Vec::new();
    for name in names {
        let Some(Some(Value::String(json))) = meridian_data(doc, &name) else {
            continue;
        };
        // Data that is not JSON at all is the reader's to refuse.
        let Ok(serde_json::Value::Object(mut meta)) = serde_json::from_str(json) else {
            continue;
        };
        let Some(written) = meta.get("role").cloned() else {
            continue;
        };
        if serde_json::from_value::<Role>(written.clone()).is_ok() {
            continue;
        }
        let shown = current
            .datasets
            .iter()
            .find(|d| d.name == name)
            .map_or(Role::Unclear, |d| d.meta.role);
        meta.insert(
            "role".to_string(),
            serde_json::Value::String(role_label(shown).to_string()),
        );
        *json = serde_json::Value::Object(meta).to_string();
        kept.push(IgnoredRole {
            dataset: name,
            written,
            shown,
        });
    }
    kept
}

/// Give each relationship of `edited` that `current` also holds `current`'s evidence and warning.
/// Those are measured over the data after each run, not decided, so a hand edit never claims
/// them: an editor whose text predates the last measurement would otherwise write the old
/// numbers back and mark every join they differ on as the analyst's.
fn keep_measurements(current: &Model, edited: &mut Model) {
    for r in &mut edited.relationships {
        if let Some(c) = current.relationships.iter().find(|c| c.name == r.name) {
            r.meta.evidence.clone_from(&c.meta.evidence);
            r.meta.warning.clone_from(&c.meta.warning);
        }
    }
}

/// Mark as the analyst's each element of `edited` that `current` does not hold as it is.
fn mark_changed(current: &Model, edited: &mut Model) {
    for d in &mut edited.datasets {
        let before = current.datasets.iter().find(|c| c.name == d.name);
        if before.is_none_or(|c| !same_dataset(c, d)) {
            d.meta.decided_by = DecidedBy::Analyst;
        }
        for f in &mut d.fields {
            let was = before.and_then(|c| c.fields.iter().find(|cf| cf.name == f.name));
            if was.is_none_or(|cf| !same_field(cf, f)) {
                f.meta.decided_by = DecidedBy::Analyst;
            }
        }
    }
    for r in &mut edited.relationships {
        let was = current.relationships.iter().find(|c| c.name == r.name);
        if was.is_none_or(|c| !same_relationship(c, r)) {
            r.meta.decided_by = DecidedBy::Analyst;
        }
    }
}

/// Keep what the edit deleted from coming back with a later discovery pass, as the module says.
/// An element is deleted when `base`, the model of the text the edit started from, held it and
/// `edited` does not; one `current` gained since `base` and `edited` lacks is kept as `current`
/// holds it. Only a deleted dataset named in `sources`, or a join between two of them, is held.
/// `set_aside` is what the edit holds but the model does not act on, which was not deleted. The
/// rejected joins `current` holds are carried into `edited` too, so an editor whose text predates
/// a drop does not take the drop back.
fn hold_deletions(
    current: &Model,
    base: &Model,
    sources: &[&str],
    edited: &mut Model,
    set_aside: &[Relationship],
) {
    let sourced = |name: &str| sources.contains(&name);
    for c in &current.datasets {
        if edited.datasets.iter().any(|d| d.name == c.name) {
            continue;
        }
        if !base.datasets.iter().any(|b| b.name == c.name) {
            edited.datasets.push(c.clone());
        } else if sourced(&c.name) {
            let mut left_out = c.clone();
            left_out.meta.excluded = true;
            left_out.meta.decided_by = DecidedBy::Analyst;
            edited.datasets.push(left_out);
        }
    }
    for r in &current.meta.rejected {
        if !edited.meta.rejected.iter().any(|e| e.name == r.name) {
            edited.meta.rejected.push(r.clone());
        }
    }
    for c in &current.relationships {
        let held = edited
            .relationships
            .iter()
            .chain(set_aside)
            .any(|r| r.name == c.name || same_join(r, c));
        if held {
            continue;
        }
        if !base.relationships.iter().any(|b| b.name == c.name) {
            edited.relationships.push(c.clone());
        } else if sourced(&c.from)
            && sourced(&c.to)
            && !edited.meta.rejected.iter().any(|e| e.name == c.name)
        {
            let mut dropped = c.clone();
            dropped.meta.decided_by = DecidedBy::Analyst;
            edited.meta.rejected.push(dropped);
        }
    }
    let active: Vec<String> = edited
        .relationships
        .iter()
        .chain(set_aside)
        .map(|r| r.name.clone())
        .collect();
    edited.meta.rejected.retain(|r| !active.contains(&r.name));
}

/// Whether two relationships join the same tables on the same columns, whatever they are called.
fn same_join(a: &Relationship, b: &Relationship) -> bool {
    a.from == b.from
        && a.from_columns == b.from_columns
        && a.to == b.to
        && a.to_columns == b.to_columns
}

/// Whether two datasets are the same but for who decided them and their fields, which are
/// compared one by one.
fn same_dataset(a: &Dataset, b: &Dataset) -> bool {
    let bare = |d: &Dataset| Dataset {
        fields: Vec::new(),
        meta: DatasetMeta {
            decided_by: DecidedBy::Analyst,
            ..d.meta.clone()
        },
        ..d.clone()
    };
    bare(a) == bare(b)
}

fn same_field(a: &Field, b: &Field) -> bool {
    let bare = |f: &Field| Field {
        meta: FieldMeta {
            decided_by: DecidedBy::Analyst,
            ..f.meta.clone()
        },
        ..f.clone()
    };
    bare(a) == bare(b)
}

/// Whether two relationships are the same but for who decided them and what was measured of
/// them, their evidence and warning.
fn same_relationship(a: &Relationship, b: &Relationship) -> bool {
    let bare = |r: &Relationship| Relationship {
        meta: RelationshipMeta {
            decided_by: DecidedBy::Analyst,
            evidence: b.meta.evidence.clone(),
            warning: b.meta.warning.clone(),
            ..r.meta.clone()
        },
        ..r.clone()
    };
    bare(a) == bare(b)
}
