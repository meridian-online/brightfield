//! Folding a fresh discovery into the model the analyst has been editing.

use crate::edit::unique_name;
use crate::{Dataset, DecidedBy, Model, Relationship};

fn same_join(a: &Relationship, b: &Relationship) -> bool {
    a.from == b.from
        && a.from_columns == b.from_columns
        && a.to == b.to
        && a.to_columns == b.to_columns
}

/// Fold `proposed`, a fresh discovery, into `current`.
///
/// Whatever `current` holds as the analyst's stays exactly as it is, and a join the
/// analyst dropped is not proposed back. What `current` holds as Dovetail's takes
/// `proposed`'s value; a Dovetail join `proposed` no longer finds is removed. A
/// dataset the analyst decided is kept whole, fields included. A dataset `proposed` doesn't mention stays: discovery never deletes the analyst's tables.
pub fn merge_discovery(current: &Model, proposed: &Model) -> Model {
    let mut out = current.clone();
    out.meta.skipped_sources = proposed.meta.skipped_sources.clone();

    let mut datasets = Vec::new();
    for cur in &current.datasets {
        let next = match proposed.datasets.iter().find(|d| d.name == cur.name) {
            Some(prop) if cur.meta.decided_by == DecidedBy::Dovetail => merge_dataset(cur, prop),
            _ => cur.clone(),
        };
        datasets.push(next);
    }
    for prop in &proposed.datasets {
        if !current.datasets.iter().any(|d| d.name == prop.name) {
            datasets.push(prop.clone());
        }
    }
    out.datasets = datasets;

    let mut relationships = Vec::new();
    for cur in &current.relationships {
        if cur.meta.decided_by == DecidedBy::Analyst {
            relationships.push(cur.clone());
        } else if let Some(prop) = proposed.relationships.iter().find(|p| same_join(cur, p)) {
            relationships.push(prop.clone());
        }
    }
    for prop in &proposed.relationships {
        let known = current.relationships.iter().any(|c| same_join(c, prop));
        let rejected = current
            .meta
            .rejected
            .iter()
            .any(|r| r.meta.decided_by == DecidedBy::Analyst && same_join(r, prop));
        if !known && !rejected {
            let mut added = prop.clone();
            added.name = unique_name(&added.name, &relationships, &current.meta.rejected);
            relationships.push(added);
        }
    }
    out.relationships = relationships;
    out
}

/// A Dovetail dataset takes `proposed`, except its analyst-settled fields, and
/// keeps what the analyst set on it that no pass should undo: an exclusion.
fn merge_dataset(cur: &Dataset, prop: &Dataset) -> Dataset {
    let mut out = prop.clone();
    out.meta.excluded = cur.meta.excluded;
    for field in &mut out.fields {
        if let Some(mine) = cur
            .fields
            .iter()
            .find(|f| f.name == field.name && f.meta.decided_by == DecidedBy::Analyst)
        {
            *field = mine.clone();
        }
    }
    for mine in &cur.fields {
        if mine.meta.decided_by == DecidedBy::Analyst
            && !out.fields.iter().any(|f| f.name == mine.name)
        {
            out.fields.push(mine.clone());
        }
    }
    out
}
