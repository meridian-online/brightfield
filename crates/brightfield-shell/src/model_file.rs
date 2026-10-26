//! Reading the Ossie file as the analyst edits it by hand in the model pane's file view.
//!
//! [`read_hand_edit`] reads the edited text against the model the pane holds. A text that is not
//! YAML, breaks the Ossie schema or is not a model Brightfield reads is refused with the reason
//! and, where the text says, the line. A text Brightfield reads becomes the next model, and each
//! dataset, field and relationship that differs from the one of the same name in the current
//! model, or is new, is marked as the analyst's; what the edit left alone keeps who decided it. A
//! relationship the edit removed is kept as rejected, as dropping it from the inspector keeps it,
//! so discovery does not propose it back.
//!
//! What the text names that Brightfield cannot act on is not refused. A relationship naming a
//! table the model does not hold, and a role outside the six, are set aside in [`Kept`]: left out
//! of the model the diagram is drawn from and the silver steps are written for, and written back
//! into the file by [`written`], since the analyst wrote them. Metrics ride in the model as it is,
//! since nothing Brightfield builds reads them. Each of these is one line of [`warnings`].

use brightfield_ossie::{
    line_of, Dataset, DatasetMeta, DecidedBy, Element, Field, FieldMeta, Model, ReadError,
    Relationship, RelationshipMeta, Role, VENDOR,
};
use serde_yaml::Value;

use crate::model_inspector::role_label;

/// What an edited file holds that Brightfield does not act on, set aside so the file goes on
/// holding it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Kept {
    /// Relationships that name a table the model does not hold.
    pub relationships: Vec<Relationship>,
    /// Roles that are not one of the six.
    pub roles: Vec<KeptRole>,
}

/// A table's role as written, when it is not one of the six.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptRole {
    /// The table.
    pub dataset: String,
    /// The role as the file writes it, a JSON value.
    pub written: serde_json::Value,
    /// The role the table is shown with instead: the one it had before the edit, or unclear.
    pub shown: Role,
}

/// A hand edit Brightfield reads: the model to act on, and what was set aside from it.
#[derive(Debug, Clone, PartialEq)]
pub struct HandEdit {
    /// The model the diagram is drawn from and the silver steps are written for.
    pub model: Model,
    /// What the file holds besides, written back with it.
    pub kept: Kept,
}

/// Read `text`, the Ossie file as the analyst edited it, against `current`, the model the pane
/// holds.
///
/// # Errors
///
/// Why the text is not a model Brightfield reads, headed with its line where the text gives one:
/// `Line 3: …`.
pub fn read_hand_edit(current: &Model, text: &str) -> Result<HandEdit, String> {
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
        serde_yaml::to_string(&doc).map_err(|e| e.to_string())?
    };
    let mut model = brightfield_ossie::read(&readable).map_err(|e| read_error(&e))?;
    let tables: Vec<String> = model.datasets.iter().map(|d| d.name.clone()).collect();
    let (acted, unjoined): (Vec<Relationship>, Vec<Relationship>) =
        std::mem::take(&mut model.relationships)
            .into_iter()
            .partition(|r| tables.contains(&r.from) && tables.contains(&r.to));
    model.relationships = acted;
    mark_changed(current, &mut model);
    reject_removed(current, &mut model, &unjoined);
    Ok(HandEdit {
        model,
        kept: Kept {
            relationships: unjoined,
            roles,
        },
    })
}

/// What of `kept` still stands beside `model`: a relationship set aside that no relationship of
/// the model now shares a name with, and a role set aside on a table that still has the role it
/// was shown with. A reshape that sets the table's role lets the written one go.
#[must_use]
pub fn still_kept(model: &Model, kept: &Kept) -> Kept {
    Kept {
        relationships: kept
            .relationships
            .iter()
            .filter(|k| !model.relationships.iter().any(|r| r.name == k.name))
            .cloned()
            .collect(),
        roles: kept
            .roles
            .iter()
            .filter(|k| {
                model
                    .datasets
                    .iter()
                    .any(|d| d.name == k.dataset && d.meta.role == k.shown)
            })
            .cloned()
            .collect(),
    }
}

/// The Ossie file for `model` with `kept` written back into it: the relationships set aside after
/// the model's own, and each role set aside in place of the one its table is shown with.
#[must_use]
pub fn written(model: &Model, kept: &Kept) -> String {
    let mut full = model.clone();
    full.relationships
        .extend(kept.relationships.iter().cloned());
    let text = brightfield_ossie::write(&full);
    if kept.roles.is_empty() {
        return text;
    }
    let Ok(mut doc) = serde_yaml::from_str::<Value>(&text) else {
        return text;
    };
    for role in &kept.roles {
        let Some(data) = meridian_data(&mut doc, &role.dataset) else {
            continue;
        };
        let Some(Value::String(json)) = data else {
            continue;
        };
        // `write` emits the entry as compact JSON, so the role stands exactly so in it.
        let shown = format!("\"role\":\"{}\"", role_label(role.shown));
        *json = json.replacen(&shown, &format!("\"role\":{}", role.written), 1);
    }
    serde_yaml::to_string(&doc).unwrap_or(text)
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
        said.push(format!(
            "{} has the role {}, which is not entity, event, reference, bridge, snapshot or \
             unclear: it stays in the file, and the table is shown as {}",
            role.dataset,
            role.written,
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

/// Set aside each role in `doc` that is not one of the six, putting in its place the one its
/// table has in `current` (or unclear, for a table `current` does not hold), so the rest reads.
fn set_aside_roles(current: &Model, doc: &mut Value) -> Vec<KeptRole> {
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
        kept.push(KeptRole {
            dataset: name,
            written,
            shown,
        });
    }
    kept
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

/// Keep each relationship of `current` the edit removed as rejected by the analyst, unless the
/// edit already lists it so or set it aside.
fn reject_removed(current: &Model, edited: &mut Model, set_aside: &[Relationship]) {
    for r in &current.relationships {
        let held = edited
            .relationships
            .iter()
            .chain(set_aside)
            .chain(&edited.meta.rejected)
            .any(|e| e.name == r.name);
        if !held {
            let mut dropped = r.clone();
            dropped.meta.decided_by = DecidedBy::Analyst;
            edited.meta.rejected.push(dropped);
        }
    }
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

fn same_relationship(a: &Relationship, b: &Relationship) -> bool {
    let bare = |r: &Relationship| Relationship {
        meta: RelationshipMeta {
            decided_by: DecidedBy::Analyst,
            ..r.meta.clone()
        },
        ..r.clone()
    };
    bare(a) == bare(b)
}
