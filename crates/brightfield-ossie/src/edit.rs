//! The analyst's edits to a model. Each one returns a new model and marks what it
//! touched as the analyst's, so discovery and verification leave it alone.

use std::fmt;

use crate::merge::same_join;
use crate::{DecidedBy, Evidence, Grade, Model, Relationship, RelationshipMeta, Role, Verified};

/// One change the analyst makes to the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    KeepJoin {
        relationship: String,
    },
    DropJoin {
        relationship: String,
    },
    AddJoin {
        from: String,
        from_columns: Vec<String>,
        to: String,
        to_columns: Vec<String>,
    },
    ChangeKeys {
        relationship: String,
        from_columns: Vec<String>,
        to_columns: Vec<String>,
    },
    SetRole {
        dataset: String,
        role: Role,
    },
    Fold {
        dataset: String,
        into: String,
    },
    Unfold {
        dataset: String,
    },
    Exclude {
        dataset: String,
    },
    Include {
        dataset: String,
    },
    Retype {
        dataset: String,
        field: String,
        finetype: String,
    },
}

/// Why an edit was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditError(pub String);

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EditError {}

fn err<T>(message: String) -> Result<T, EditError> {
    Err(EditError(message))
}

/// Apply one edit to a copy of `model`.
pub fn apply(model: &Model, edit: &Edit) -> Result<Model, EditError> {
    let mut m = model.clone();
    match edit {
        Edit::KeepJoin { relationship } => {
            relationship_mut(&mut m, relationship)?.meta.decided_by = DecidedBy::Analyst;
        }
        Edit::DropJoin { relationship } => {
            let at = relationship_index(&m, relationship)?;
            let r = &m.relationships[at];
            let joins = |a: &str, b: &str| {
                m.relationships
                    .iter()
                    .filter(|x| (x.from == a && x.to == b) || (x.from == b && x.to == a))
                    .count()
            };
            for (child, host) in [(&r.from, &r.to), (&r.to, &r.from)] {
                let folded = m
                    .datasets
                    .iter()
                    .any(|d| d.name == *child && d.meta.folded_into.as_deref() == Some(host));
                if folded && joins(child, host) == 1 {
                    return err(format!(
                        "{relationship} is the only join folding {child} into {host}: unfold {child} first"
                    ));
                }
            }
            let mut dropped = m.relationships.remove(at);
            dropped.meta.decided_by = DecidedBy::Analyst;
            m.meta.rejected.push(dropped);
        }
        Edit::AddJoin {
            from,
            from_columns,
            to,
            to_columns,
        } => {
            dataset_index(&m, from)?;
            dataset_index(&m, to)?;
            if from_columns.is_empty() || from_columns.len() != to_columns.len() {
                return err(format!(
                    "a join from {from} to {to} needs the same, non-zero number of columns on each side"
                ));
            }
            if m.relationships.iter().any(|r| {
                r.from == *from
                    && r.to == *to
                    && r.from_columns == *from_columns
                    && r.to_columns == *to_columns
            }) {
                return err(format!(
                    "the model already joins {from} to {to} on those columns"
                ));
            }
            let name = unique_name(&format!("{from}_{to}"), &m.relationships, &m.meta.rejected);
            m.relationships.push(Relationship {
                name,
                from: from.clone(),
                to: to.clone(),
                from_columns: from_columns.clone(),
                to_columns: to_columns.clone(),
                meta: RelationshipMeta {
                    grade: Grade::Possible,
                    evidence: Evidence {
                        value_overlap: 0.0,
                        orphans: 0,
                        child_rows: 0,
                        child_type: None,
                        parent_type: None,
                        types_agree: false,
                        name_similarity: 0.0,
                    },
                    cardinality: "N:1".to_string(),
                    verified: Verified::Unmeasured,
                    decided_by: DecidedBy::Analyst,
                    warning: None,
                },
                extras: Default::default(),
                other_extensions: Vec::new(),
            });
        }
        Edit::ChangeKeys {
            relationship,
            from_columns,
            to_columns,
        } => {
            if from_columns.is_empty() || from_columns.len() != to_columns.len() {
                return err(format!(
                    "{relationship} needs the same, non-zero number of columns on each side"
                ));
            }
            let at = relationship_index(&m, relationship)?;
            let mut changed = m.relationships[at].clone();
            changed.from_columns = from_columns.clone();
            changed.to_columns = to_columns.clone();
            // Keep the join discovery found as a rejected one, so a rediscovery
            // doesn't add it back beside the corrected keys.
            let original = &m.relationships[at];
            if original.meta.decided_by == DecidedBy::Dovetail && !same_join(original, &changed) {
                let mut kept = original.clone();
                kept.name = unique_name(&kept.name, &m.relationships, &m.meta.rejected);
                kept.meta.decided_by = DecidedBy::Analyst;
                m.meta.rejected.push(kept);
            }
            // Changing back to a rejected join takes it out of the rejected list.
            m.meta.rejected.retain(|x| !same_join(x, &changed));
            let r = &mut m.relationships[at];
            r.from_columns = from_columns.clone();
            r.to_columns = to_columns.clone();
            r.meta.verified = Verified::Unmeasured;
            r.meta.decided_by = DecidedBy::Analyst;
        }
        Edit::SetRole { dataset, role } => {
            let d = dataset_mut(&mut m, dataset)?;
            d.meta.role = *role;
            d.meta.decided_by = DecidedBy::Analyst;
        }
        Edit::Fold { dataset, into } => {
            dataset_index(&m, dataset)?;
            dataset_index(&m, into)?;
            let joined = m.relationships.iter().any(|r| {
                (r.from == *dataset && r.to == *into) || (r.from == *into && r.to == *dataset)
            });
            if !joined {
                return err(format!(
                    "{dataset} can't fold into {into}: no relationship joins them"
                ));
            }
            // The join the fold rests on is the analyst's now, so discovery keeps it.
            for r in &mut m.relationships {
                if (r.from == *dataset && r.to == *into) || (r.from == *into && r.to == *dataset) {
                    r.meta.decided_by = DecidedBy::Analyst;
                }
            }
            let d = dataset_mut(&mut m, dataset)?;
            d.meta.folded_into = Some(into.clone());
            d.meta.role = Role::Reference;
            d.meta.decided_by = DecidedBy::Analyst;
        }
        Edit::Unfold { dataset } => {
            let d = dataset_mut(&mut m, dataset)?;
            d.meta.folded_into = None;
            d.meta.decided_by = DecidedBy::Analyst;
        }
        Edit::Exclude { dataset } => {
            let d = dataset_mut(&mut m, dataset)?;
            d.meta.excluded = true;
            d.meta.decided_by = DecidedBy::Analyst;
        }
        Edit::Include { dataset } => {
            let d = dataset_mut(&mut m, dataset)?;
            d.meta.excluded = false;
            d.meta.decided_by = DecidedBy::Analyst;
        }
        Edit::Retype {
            dataset,
            field,
            finetype,
        } => {
            let d = dataset_mut(&mut m, dataset)?;
            let Some(f) = d.fields.iter_mut().find(|f| f.name == *field) else {
                return err(format!("{dataset} has no field {field}"));
            };
            f.meta.finetype = Some(finetype.clone());
            f.meta.decided_by = DecidedBy::Analyst;
        }
    }
    Ok(m)
}

/// `base`, or `base_2`, `base_3`... when a kept or rejected relationship already has the name.
pub(crate) fn unique_name(base: &str, kept: &[Relationship], rejected: &[Relationship]) -> String {
    let taken = |n: &str| kept.iter().chain(rejected).any(|r| r.name == n);
    let mut name = base.to_string();
    let mut n = 2;
    while taken(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

fn dataset_index(m: &Model, name: &str) -> Result<usize, EditError> {
    match m.datasets.iter().position(|d| d.name == name) {
        Some(i) => Ok(i),
        None => err(format!("no dataset named {name}")),
    }
}

fn dataset_mut<'a>(m: &'a mut Model, name: &str) -> Result<&'a mut crate::Dataset, EditError> {
    let i = dataset_index(m, name)?;
    Ok(&mut m.datasets[i])
}

fn relationship_index(m: &Model, name: &str) -> Result<usize, EditError> {
    match m.relationships.iter().position(|r| r.name == name) {
        Some(i) => Ok(i),
        None => err(format!("no relationship named {name}")),
    }
}

fn relationship_mut<'a>(m: &'a mut Model, name: &str) -> Result<&'a mut Relationship, EditError> {
    let i = relationship_index(m, name)?;
    Ok(&mut m.relationships[i])
}
