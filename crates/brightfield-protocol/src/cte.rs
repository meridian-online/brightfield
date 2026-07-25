//! CTE explode: the joins INSIDE a SQL step, drawn as nodes of their own.
//!
//! A SQL step is otherwise one box even when its CTEs are the actual lineage —
//! `sec_entities.sql` reads two files, joins them, and shows as a single
//! rectangle. [`explode_ctes`] refines a built [`AssetGraph`] so each top-level
//! CTE of each SQL statement becomes a node under
//! `cte.<protocol>.<relation>#<name>`, where `<relation>` is the relation the
//! declaring statement produces. The reads a CTE body used to contribute to the
//! statement as a whole are re-routed THROUGH that node, so the picture gains
//! detail without inventing or losing an edge.
//!
//! **One derivation, both inputs.** The explode reads SQL text, which both
//! input paths carry: the manifest path has the model sources
//! ([`manifest_sql`]), the contract path has each step's `sql_text`
//! ([`contract_sql`]). Feed either into [`explode_ctes`] and the same CTE nodes
//! come out under the same ids.
//!
//! **Opt-in, so the folded view is the built one.** [`explode_ctes`] is a
//! transform over a finished graph, like
//! [`collapse_families`](crate::collapse_families) — the graph the builders
//! return is unchanged, which is what keeps a step folded to one chip until
//! something asks for the detail.
//!
//! **Never fabricates.** A read that resolves to no node in the graph wires
//! nothing, and a direct edge is dropped only when this pass actually re-routed
//! it. A CTE whose body failed to parse becomes an issue-badged
//! [`Opaque`](crate::AssetKind::Opaque) chip in the lineage rather than
//! vanishing from it.

use std::collections::{BTreeMap, BTreeSet};

use crate::contract_graph::ContractView;
use crate::graph::{AssetGraph, AssetId, AssetKind, AssetNode, Edge, StepId};
use crate::sql_assets::{extract_statement_assets, CteBody, StatementAssets};

/// The node id of one CTE: `cte.<protocol>.<relation>#<name>`.
///
/// `relation` is the relation the declaring statement produces — the asset the
/// CTE feeds. Both input paths speak in relation names, so the id is the same
/// whichever built the graph.
#[must_use]
pub fn cte_id(protocol: &str, relation: &str, cte: &str) -> AssetId {
    format!("cte.{protocol}.{relation}#{cte}")
}

/// The SQL text of every readable `sql:` step, from the manifest path's model
/// sources (see [`load_model_sources`](crate::graph::load_model_sources)). A
/// step whose model could not be read contributes nothing — it is already an
/// opaque chip in the graph.
#[must_use]
pub fn manifest_sql(
    sources: &BTreeMap<StepId, Result<String, String>>,
) -> BTreeMap<StepId, String> {
    sources
        .iter()
        .filter_map(|(step, source)| source.as_ref().ok().map(|sql| (step.clone(), sql.clone())))
        .collect()
}

/// The SQL text of every `sql` step in a built contract view. A step whose
/// contract carries no `sql_text` contributes nothing.
#[must_use]
pub fn contract_sql(view: &ContractView) -> BTreeMap<StepId, String> {
    view.steps
        .iter()
        .filter_map(|(step, detail)| {
            detail
                .sql_text
                .as_ref()
                .map(|sql| (step.clone(), sql.clone()))
        })
        .collect()
}

/// Relation name -> node id, over the relation-shaped nodes of a graph. Both
/// builders label a relation node with the relation's own name.
fn relations_by_name(graph: &AssetGraph) -> BTreeMap<String, AssetId> {
    let mut out = BTreeMap::new();
    for (id, node) in &graph.nodes {
        if matches!(node.kind, AssetKind::Table | AssetKind::Internal) {
            out.entry(node.label.to_lowercase()).or_insert(id.clone());
        }
    }
    out
}

/// Explode every SQL step's top-level CTEs into nodes of their own.
///
/// `sql_by_step` maps a step name to that step's SQL text — build it with
/// [`manifest_sql`] or [`contract_sql`]. Steps absent from the graph's seams,
/// statements that produce no relation, and statements with no `WITH` clause
/// are all passed over untouched.
///
/// For each statement that declares CTEs and produces a relation:
/// - each CTE gains a node under [`cte_id`], `Internal` when its body parsed
///   and `Opaque` (issue-badged) when it did not;
/// - a CTE body's read of a sibling CTE becomes an intra-step edge between two
///   CTE nodes; a read of anything else becomes an edge from that asset's node
///   into the CTE node;
/// - each CTE the MAIN body reads gains an edge into the statement's produced
///   relation;
/// - the direct edge a re-routed read used to have into that relation is
///   dropped, unless the main body reads it too.
///
/// The result is a plain [`AssetGraph`]: it lays out, collapses and renders
/// like any other. Deterministic — same input, same nodes, same edge order.
#[must_use]
pub fn explode_ctes(graph: &AssetGraph, sql_by_step: &BTreeMap<StepId, String>) -> AssetGraph {
    let proto = graph.protocol.clone();
    let by_name = relations_by_name(graph);
    let mut out = graph.clone();
    let mut add: Vec<Edge> = Vec::new();
    let mut drop: BTreeSet<(AssetId, AssetId, Option<StepId>)> = BTreeSet::new();

    for (step, sql) in sql_by_step {
        if !graph.seams.contains_key(step) {
            continue;
        }
        for statement in extract_statement_assets(sql) {
            let StatementAssets::Parsed {
                produced: Some(relation),
                ctes,
                main_reads,
                ..
            } = statement
            else {
                continue;
            };
            if ctes.is_empty() {
                continue;
            }
            // The statement's own product: prefer the node this step produces,
            // so a name reused across steps cannot cross-wire.
            let Some(target) = graph
                .nodes
                .values()
                .find(|n| {
                    n.label.to_lowercase() == relation
                        && n.step.as_deref() == Some(step.as_str())
                        && matches!(n.kind, AssetKind::Table | AssetKind::Internal)
                })
                .map(|n| n.id.clone())
                .or_else(|| by_name.get(&relation).cloned())
            else {
                continue;
            };
            let declared: BTreeSet<&str> = ctes.iter().map(|c| c.name.as_str()).collect();

            for cte in &ctes {
                let (kind, issue) = match &cte.body {
                    CteBody::Parsed(_) => (AssetKind::Internal, None),
                    CteBody::Opaque { error } => (AssetKind::Opaque, Some(error.clone())),
                };
                let id = cte_id(&proto, &relation, &cte.name);
                out.nodes.entry(id.clone()).or_insert(AssetNode {
                    id,
                    kind,
                    label: cte.name.clone(),
                    step: Some(step.clone()),
                    family_count: None,
                    issue,
                });
            }

            // What each CTE body reads, wired into that CTE's node.
            let mut rerouted: BTreeSet<AssetId> = BTreeSet::new();
            for cte in &ctes {
                let CteBody::Parsed(reads) = &cte.body else {
                    continue;
                };
                let to = cte_id(&proto, &relation, &cte.name);
                for sibling in &reads.ctes {
                    if declared.contains(sibling.as_str()) {
                        add.push(edge(cte_id(&proto, &relation, sibling), to.clone(), step));
                    }
                }
                for read in &reads.relations {
                    if let Some(from) = by_name.get(read) {
                        rerouted.insert(from.clone());
                        add.push(edge(from.clone(), to.clone(), step));
                    }
                }
                for path in &reads.files {
                    let from = format!("file.{proto}.{path}");
                    if graph.nodes.contains_key(&from) {
                        rerouted.insert(from.clone());
                        add.push(edge(from, to.clone(), step));
                    }
                }
            }

            // The CTEs the main body reads feed the statement's product.
            for name in &main_reads.ctes {
                if declared.contains(name.as_str()) {
                    add.push(edge(cte_id(&proto, &relation, name), target.clone(), step));
                }
            }

            // A source the main body ALSO reads keeps its direct edge; one now
            // reached only through a CTE loses it.
            let mut direct: BTreeSet<AssetId> = BTreeSet::new();
            for read in &main_reads.relations {
                if let Some(id) = by_name.get(read) {
                    direct.insert(id.clone());
                }
            }
            for path in &main_reads.files {
                direct.insert(format!("file.{proto}.{path}"));
            }
            for from in rerouted.difference(&direct) {
                drop.insert((from.clone(), target.clone(), Some(step.clone())));
            }
        }
    }

    out.edges
        .retain(|e| !drop.contains(&(e.from.clone(), e.to.clone(), e.via.clone())));
    for candidate in add {
        if candidate.from == candidate.to
            || !out.nodes.contains_key(&candidate.from)
            || !out.nodes.contains_key(&candidate.to)
        {
            continue;
        }
        if !out
            .edges
            .iter()
            .any(|e| e.from == candidate.from && e.to == candidate.to && e.via == candidate.via)
        {
            out.edges.push(candidate);
        }
    }
    out
}

/// An un-shielded lineage edge through `step`.
fn edge(from: AssetId, to: AssetId, step: &StepId) -> Edge {
    Edge {
        from,
        to,
        via: Some(step.clone()),
        shield: false,
    }
}
