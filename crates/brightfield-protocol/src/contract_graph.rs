//! Build the crate's typed [`AssetGraph`] from the emitted Protocol+Run
//! contract — the default input, parallel to the manifest path in
//! [`graph::build_graph`](crate::graph::build_graph).
//!
//! The manifest path re-parses `arcform.yaml` + `models/*.sql` and infers
//! lineage from `with:`/`depends_on`/SQL. This path consumes the *emitted*
//! contract instead: the run already lists its assets (with `produced_by` /
//! `consumed_by`) and its steps (with per-statement `produces`/`reads`, op
//! refs, and status), so lineage is read, not inferred. What is re-derived here
//! is the crate's *finer view kind* from the contract's coarse `kind`, using
//! the same rules the manifest path applies — reusing
//! [`AssetKind`], [`Seam`], [`Edge`] and the [`GATE_OP`](crate::graph)/
//! [`EXPORT_OP`](crate::graph) vocabulary, so the same layout, family collapse,
//! and renderer draw both.
//!
//! ## Reconciliation (contract → crate model)
//! - **Ids.** Contract ids are flat (`table:widgets`, `file:build/x.parquet`).
//!   The crate namespaces by protocol: `asset.<proto>.<relation>` /
//!   `file.<proto>.<path>` / `source.<proto>.<url>`. Mapped from
//!   `run.protocol.name` + the asset kind + the flat id tail.
//! - **View kind.** The contract's coarse `table|model` becomes **Internal**
//!   when a relation is produced then read only within its own model file
//!   (from `sql.statements`) and no other step consumes it; a `parquet_export`
//!   dest that reaches no other export becomes the terminal **Dataset**;
//!   `finetype_validate` is a shield on its guarded edge, never a node.
//! - **Status.** An asset whose `produced_by` step was skipped/failed is not
//!   materialised (its `row_count` is null) — flagged, and never re-kinded to
//!   the Dataset sink. The `.json` is the authoritative complete step set; the
//!   `.jsonl` stream ([`apply_stream`]) only overlays live per-step state.
//! - **Order.** Contract assets arrive alphabetical; [`ContractView::order`]
//!   is a deterministic topological order over the edges.

use std::collections::{BTreeMap, BTreeSet};

use crate::contract::{Asset, Contract, ContractAssetKind, Outcome, Step, StepKind, StepState};
use crate::graph::{
    AssetGraph, AssetId, AssetKind, AssetNode, Edge, Seam, SeamKind, StepId, EXPORT_OP, GATE_OP,
};
use crate::stream::StreamState;

/// Run-level header for the protocol view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunView {
    /// Stable run id (also the `.jsonl` basename).
    pub run_id: String,
    /// Protocol name — the id namespace.
    pub protocol: String,
    /// Overall outcome (refreshed from the live stream when folded).
    pub outcome: Outcome,
    /// RFC3339 start timestamp, when recorded.
    pub started_at: Option<String>,
    /// RFC3339 finish timestamp, when recorded.
    pub finished_at: Option<String>,
    /// `true` once the run is complete — the contract carries a `finished_at`,
    /// or the live stream folded a `run_complete` event.
    pub complete: bool,
}

/// The execution-status channel a seam renders — the per-step run
/// status + live state, reduced to the handful of tints the
/// canvas draws. Kept beside the pure [`AssetGraph`] so the renderer can tint
/// seams without the graph carrying measurement. The honesty rule holds:
/// `Skipped` is its own tint, never `Ok`; an unrun step is never green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamStatus {
    /// No status recorded — unrun / unmeasured (never green).
    NotRun,
    /// Live stream says the step is in flight.
    Running,
    /// Terminal success.
    Ok,
    /// Skipped (fresh / hash-clean) — distinct from success.
    Skipped,
    /// Failed.
    Failed,
}

/// Per-step detail for the step panel — the fields the pure [`AssetGraph`]
/// deliberately does not carry (SQL text, status, live state, narrative label).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepView {
    /// Step name (the seam identity).
    pub name: StepId,
    /// Display label — `narrative.label` when present, else the name.
    pub label: String,
    /// Step kind.
    pub kind: StepKind,
    /// Terminal state from the authoritative `.json`.
    pub state: StepState,
    /// Latest raw state observed on the live `.jsonl` stream, once folded.
    pub live_state: Option<String>,
    /// Reason a step was skipped, when recorded.
    pub skip_reason: Option<String>,
    /// SQL text, for `sql` steps that carry it.
    pub sql_text: Option<String>,
    /// Operator name, for `op` steps.
    pub op: Option<String>,
    /// `true` for `finetype_validate` — a shield on its edge, not a node.
    pub gate: bool,
}

/// Measured / status fields for one asset — kept beside the pure
/// [`AssetGraph`] rather than on [`AssetNode`], which stays framework- and
/// measurement-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetMeta {
    /// Row count when measured (`None` for a file, or a not-materialised
    /// relation — disambiguate with `materialized`).
    pub row_count: Option<u64>,
    /// `true` when the asset was actually produced: its producing step
    /// succeeded (or it is an external input). A skipped/failed producer leaves
    /// this `false` even though the node still shows in the lineage.
    pub materialized: bool,
    /// Size in bytes, when measured.
    pub bytes: Option<u64>,
    /// Content hash, when computed.
    pub content_hash: Option<String>,
}

/// The built protocol view: the pure [`AssetGraph`] the layout/collapse/render
/// pipeline draws, plus the contract-derived detail the panels need.
#[derive(Debug, Clone)]
pub struct ContractView {
    /// The typed asset graph — feeds `collapse_families` → `layout` →
    /// `render_asset_graph`, exactly like the manifest path's graph.
    pub graph: AssetGraph,
    /// Run header.
    pub run: RunView,
    /// A deterministic topological order over the graph's nodes (assets arrive
    /// alphabetical; this orders the DAG by its edges).
    pub order: Vec<AssetId>,
    /// Per-step detail, keyed by step name.
    pub steps: BTreeMap<StepId, StepView>,
    /// Per-asset measured/status detail, keyed by crate asset id.
    pub assets: BTreeMap<AssetId, AssetMeta>,
}

impl StepView {
    /// The seam's execution-status tint: the live stream state wins when the
    /// step is in flight, else the authoritative terminal `.json` state.
    #[must_use]
    pub fn seam_status(&self) -> SeamStatus {
        if let Some(live) = &self.live_state {
            match live.as_str() {
                "running" | "retrying" | "started" => return SeamStatus::Running,
                "succeeded" | "success" | "ok" => return SeamStatus::Ok,
                "failed" | "aborted" => return SeamStatus::Failed,
                "skipped" | "skipped_fresh" | "skipped_hash" => return SeamStatus::Skipped,
                _ => {}
            }
        }
        match self.state {
            StepState::Success => SeamStatus::Ok,
            StepState::Failed => SeamStatus::Failed,
            StepState::Skipped => SeamStatus::Skipped,
            StepState::Unknown => SeamStatus::NotRun,
        }
    }
}

impl ContractView {
    /// The per-step execution-status tint the canvas draws on each seam
    /// — surfacing the run status + live state in the
    /// interactive view. Keyed by step name, matching [`Edge::via`].
    #[must_use]
    pub fn seam_statuses(&self) -> BTreeMap<StepId, SeamStatus> {
        self.steps
            .iter()
            .map(|(name, s)| (name.clone(), s.seam_status()))
            .collect()
    }
}

/// The flat tail of a contract id (`table:widgets` → `widgets`).
fn flat_tail(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, rest)| rest)
}

/// Host portion of a URL, for SOURCE labels (mirrors the manifest path).
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split('/').next().unwrap_or(rest).to_string()
}

/// Map a contract asset to its namespaced crate id.
fn crate_asset_id(proto: &str, asset: &Asset) -> AssetId {
    let tail = flat_tail(&asset.id);
    match asset.kind {
        ContractAssetKind::Source => format!("source.{proto}.{tail}"),
        ContractAssetKind::File => format!("file.{proto}.{tail}"),
        // Every relation-shaped asset lives under `asset.` (Table/Internal); the
        // node's kind carries the finer distinction.
        _ => format!("asset.{proto}.{tail}"),
    }
}

/// The node label per kind: SOURCE = host, FILE = its path, everything else the
/// relation name.
fn node_label(asset: &Asset) -> String {
    match asset.kind {
        ContractAssetKind::Source => host_of(
            asset
                .path
                .as_deref()
                .unwrap_or_else(|| flat_tail(&asset.id)),
        ),
        ContractAssetKind::File => flat_tail(&asset.id).to_string(),
        _ => asset.name.clone(),
    }
}

/// The seam kind for a contract step.
fn seam_kind(step: &Step) -> SeamKind {
    match step.kind {
        StepKind::Op => match &step.op_ref {
            Some(op) => SeamKind::Op {
                name: op.name.clone(),
                version: op.version(),
            },
            None => SeamKind::Opaque,
        },
        StepKind::Sql => SeamKind::Sql {
            model: step
                .sql
                .as_ref()
                .and_then(|b| b.model_path.clone())
                .unwrap_or_else(|| step.name.clone()),
        },
        StepKind::Command => SeamKind::Command,
        StepKind::Hook | StepKind::Unknown => SeamKind::Opaque,
    }
}

/// A relation is **Internal** when it is produced then read only inside its own
/// model file and no other step consumes it (mirrors the manifest path's
/// statement-level INTERNAL rule, read from the contract's `statements`).
fn is_internal(asset: &Asset, step_by_name: &BTreeMap<&str, &Step>) -> bool {
    if !matches!(
        asset.kind,
        ContractAssetKind::Table | ContractAssetKind::Model
    ) {
        return false;
    }
    let Some(producer) = asset.produced_by.as_deref() else {
        return false;
    };
    // A cross-step consumer disqualifies it — it is a durable TABLE.
    if asset.consumed_by.iter().any(|c| c != producer) {
        return false;
    }
    let Some(step) = step_by_name.get(producer) else {
        return false;
    };
    let Some(sql) = &step.sql else {
        return false;
    };
    let mut produced_at: Option<usize> = None;
    for (i, stmt) in sql.statements.iter().enumerate() {
        if produced_at.is_none() && stmt.produces.iter().any(|p| *p == asset.name) {
            produced_at = Some(i);
            continue;
        }
        if let Some(pi) = produced_at {
            if i > pi && stmt.reads.iter().any(|r| *r == asset.name) {
                return true;
            }
        }
    }
    false
}

/// Add a deduplicated `from -> to via` edge, dropping self-loops and edges
/// touching a node that does not exist.
fn add_edge(
    edges: &mut Vec<Edge>,
    seen: &mut BTreeSet<(AssetId, AssetId, Option<StepId>)>,
    nodes: &BTreeMap<AssetId, AssetNode>,
    from: &AssetId,
    to: &AssetId,
    via: &str,
) {
    if from == to || !nodes.contains_key(from) || !nodes.contains_key(to) {
        return;
    }
    let key = (from.clone(), to.clone(), Some(via.to_string()));
    if seen.insert(key) {
        edges.push(Edge {
            from: from.clone(),
            to: to.clone(),
            via: Some(via.to_string()),
            shield: false,
        });
    }
}

/// Resolve a SQL `reads` entry to a node id: a known relation name, else a
/// produced file path, else `None` (never fabricate a node from a config
/// string).
fn resolve_read(
    id_of_name: &BTreeMap<&str, AssetId>,
    nodes: &BTreeMap<AssetId, AssetNode>,
    proto: &str,
    read: &str,
) -> Option<AssetId> {
    if let Some(id) = id_of_name.get(read) {
        return Some(id.clone());
    }
    let file_id = format!("file.{proto}.{read}");
    nodes.contains_key(&file_id).then_some(file_id)
}

/// Build the [`ContractView`] from a parsed contract.
#[must_use]
pub fn build_contract_view(contract: &Contract) -> ContractView {
    let proto = contract.run.protocol.name.clone();

    let step_by_name: BTreeMap<&str, &Step> = contract
        .steps
        .iter()
        .map(|s| (s.name.as_str(), s))
        .collect();

    // Contract id -> crate id, and relation name -> crate id (statements speak
    // in relation names). First asset for a name wins (deterministic order).
    let mut id_of: BTreeMap<&str, AssetId> = BTreeMap::new();
    let mut id_of_name: BTreeMap<&str, AssetId> = BTreeMap::new();
    for asset in &contract.assets {
        let id = crate_asset_id(&proto, asset);
        id_of.insert(asset.id.as_str(), id.clone());
        id_of_name.entry(asset.name.as_str()).or_insert(id);
    }

    // Nodes + per-asset meta. Base kind from the coarse contract kind, refined
    // to INTERNAL here; the DATASET re-kind waits until edges exist (below).
    let mut nodes: BTreeMap<AssetId, AssetNode> = BTreeMap::new();
    let mut assets: BTreeMap<AssetId, AssetMeta> = BTreeMap::new();
    for asset in &contract.assets {
        let id = id_of[asset.id.as_str()].clone();
        let mut kind = match asset.kind {
            ContractAssetKind::Source => AssetKind::Source,
            ContractAssetKind::File => AssetKind::File,
            ContractAssetKind::Table | ContractAssetKind::Model => AssetKind::Table,
            // Out-of-domain viz artifacts — surface them as produced files.
            ContractAssetKind::ChartSpec
            | ContractAssetKind::Metrics
            | ContractAssetKind::Dashboard
            | ContractAssetKind::Unknown => AssetKind::File,
        };
        if is_internal(asset, &step_by_name) {
            kind = AssetKind::Internal;
        }
        // Producer status: materialised only when the producing step succeeded
        // (or there is no producer — an external input). A skipped/failed
        // producer leaves the asset unmaterialised, flagged, and out of the
        // Dataset re-kind below.
        let producer_state = asset
            .produced_by
            .as_deref()
            .and_then(|p| step_by_name.get(p))
            .map(|s| s.status.state);
        let materialized = !matches!(producer_state, Some(StepState::Skipped | StepState::Failed));
        let issue = match producer_state {
            Some(StepState::Skipped) => Some(format!(
                "not materialised: step {} skipped",
                asset.produced_by.as_deref().unwrap_or("?")
            )),
            Some(StepState::Failed) => Some(format!(
                "not materialised: step {} failed",
                asset.produced_by.as_deref().unwrap_or("?")
            )),
            _ => None,
        };
        nodes.insert(
            id.clone(),
            AssetNode {
                id: id.clone(),
                kind,
                label: node_label(asset),
                step: asset.produced_by.clone(),
                family_count: None,
                issue,
            },
        );
        assets.insert(
            id,
            AssetMeta {
                row_count: asset.row_count,
                materialized,
                bytes: asset.bytes,
                content_hash: asset.content_hash.clone(),
            },
        );
    }

    // Seams — one per step, in contract order (collapse needs the order).
    let mut seams: BTreeMap<StepId, Seam> = BTreeMap::new();
    for (index, step) in contract.steps.iter().enumerate() {
        seams.insert(
            step.name.clone(),
            Seam {
                step: step.name.clone(),
                index,
                kind: seam_kind(step),
                gate: step.op_name() == Some(GATE_OP),
            },
        );
    }

    // Edges: asset -> asset via the step between them. SQL steps use the
    // per-statement reads/produces for intra-step lineage (staged -> cleaned);
    // op/command steps wire every consumed asset to every produced asset.
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen: BTreeSet<(AssetId, AssetId, Option<StepId>)> = BTreeSet::new();
    for step in &contract.steps {
        if seams[&step.name].gate {
            continue; // a gate is a shield on an edge, never a lineage seam
        }
        let produced: BTreeSet<AssetId> = contract
            .assets
            .iter()
            .filter(|a| a.produced_by.as_deref() == Some(step.name.as_str()))
            .map(|a| id_of[a.id.as_str()].clone())
            .collect();
        let consumed: BTreeSet<AssetId> = contract
            .assets
            .iter()
            .filter(|a| a.consumed_by.iter().any(|c| *c == step.name))
            .map(|a| id_of[a.id.as_str()].clone())
            .collect();

        if let (StepKind::Sql, Some(sql)) = (step.kind, step.sql.as_ref()) {
            let mut covered: BTreeSet<AssetId> = BTreeSet::new();
            let mut first_target: Option<AssetId> = None;
            for stmt in &sql.statements {
                for produce in &stmt.produces {
                    let Some(target) = id_of_name.get(produce.as_str()).cloned() else {
                        continue;
                    };
                    for read in &stmt.reads {
                        if let Some(from) = resolve_read(&id_of_name, &nodes, &proto, read) {
                            add_edge(&mut edges, &mut seen, &nodes, &from, &target, &step.name);
                            covered.insert(from);
                        }
                    }
                    if first_target.is_none() {
                        first_target = Some(target.clone());
                    }
                }
            }
            // Cross-step inputs the statements didn't already read (depends_on-
            // style file/table inputs) wire into the step's first product.
            if let Some(target) = &first_target {
                for from in &consumed {
                    if !covered.contains(from) {
                        add_edge(&mut edges, &mut seen, &nodes, from, target, &step.name);
                    }
                }
            }
        } else {
            for from in &consumed {
                for to in &produced {
                    add_edge(&mut edges, &mut seen, &nodes, from, to, &step.name);
                }
            }
        }
    }

    // Gates: shield every edge into a guarded (gate-consumed) asset.
    for step in &contract.steps {
        if !seams[&step.name].gate {
            continue;
        }
        let guarded: BTreeSet<AssetId> = contract
            .assets
            .iter()
            .filter(|a| a.consumed_by.iter().any(|c| *c == step.name))
            .map(|a| id_of[a.id.as_str()].clone())
            .collect();
        for edge in &mut edges {
            if guarded.contains(&edge.to) {
                edge.shield = true;
            }
        }
    }

    // Dataset sink: a parquet_export dest that reaches no OTHER export dest
    // through the lineage (a describe/validate sidecar produces no export, so
    // it leaves the terminal the sink) and whose producer succeeded — the same
    // transitive rule the manifest path applies.
    let export_steps: BTreeSet<&str> = contract
        .steps
        .iter()
        .filter(|s| s.op_name() == Some(EXPORT_OP))
        .map(|s| s.name.as_str())
        .collect();
    let export_dests: BTreeSet<AssetId> = contract
        .assets
        .iter()
        .filter(|a| {
            a.produced_by
                .as_deref()
                .is_some_and(|p| export_steps.contains(p))
        })
        .map(|a| id_of[a.id.as_str()].clone())
        .collect();
    let mut forward: BTreeMap<AssetId, BTreeSet<AssetId>> = BTreeMap::new();
    for edge in &edges {
        forward
            .entry(edge.from.clone())
            .or_default()
            .insert(edge.to.clone());
    }
    let reaches_other_export = |start: &AssetId| -> bool {
        let mut stack = vec![start.clone()];
        let mut visited: BTreeSet<AssetId> = BTreeSet::new();
        while let Some(node) = stack.pop() {
            for succ in forward.get(&node).into_iter().flatten() {
                if succ != start && export_dests.contains(succ) {
                    return true;
                }
                if visited.insert(succ.clone()) {
                    stack.push(succ.clone());
                }
            }
        }
        false
    };
    for asset in &contract.assets {
        let Some(producer) = asset.produced_by.as_deref() else {
            continue;
        };
        if !export_steps.contains(producer) {
            continue;
        }
        // note (a): a skipped/failed export is not a materialised sink.
        if step_by_name.get(producer).map(|s| s.status.state) != Some(StepState::Success) {
            continue;
        }
        let id = id_of[asset.id.as_str()].clone();
        if !reaches_other_export(&id) {
            if let Some(node) = nodes.get_mut(&id) {
                node.kind = AssetKind::Dataset;
            }
        }
    }

    // Per-step detail.
    let steps: BTreeMap<StepId, StepView> = contract
        .steps
        .iter()
        .map(|step| {
            (
                step.name.clone(),
                StepView {
                    name: step.name.clone(),
                    label: step.label(),
                    kind: step.kind,
                    state: step.status.state,
                    live_state: None,
                    skip_reason: step.status.skip_reason.clone(),
                    sql_text: step.sql.as_ref().and_then(|b| b.sql_text.clone()),
                    op: step.op_name().map(str::to_string),
                    gate: step.op_name() == Some(GATE_OP),
                },
            )
        })
        .collect();

    let order = topological_order(&nodes, &edges);

    let graph = AssetGraph {
        protocol: proto.clone(),
        nodes,
        seams,
        edges,
    };

    let run = RunView {
        run_id: contract.run.run_id.clone(),
        protocol: proto,
        outcome: contract.run.outcome,
        started_at: contract.run.started_at.clone(),
        finished_at: contract.run.finished_at.clone(),
        complete: contract.run.finished_at.is_some(),
    };

    ContractView {
        graph,
        run,
        order,
        steps,
        assets,
    }
}

/// A deterministic topological order over the graph's nodes (Kahn's algorithm,
/// ties by id). Assets arrive alphabetical in the contract; this orders them by
/// the edges so the renderer/consumer can walk the DAG upstream → downstream.
fn topological_order(nodes: &BTreeMap<AssetId, AssetNode>, edges: &[Edge]) -> Vec<AssetId> {
    let mut succ: BTreeMap<AssetId, BTreeSet<AssetId>> = BTreeMap::new();
    let mut indegree: BTreeMap<AssetId, usize> =
        nodes.keys().map(|k| (k.clone(), 0usize)).collect();
    let mut pair_seen: BTreeSet<(AssetId, AssetId)> = BTreeSet::new();
    for edge in edges {
        if edge.from == edge.to || !nodes.contains_key(&edge.from) || !nodes.contains_key(&edge.to)
        {
            continue;
        }
        if pair_seen.insert((edge.from.clone(), edge.to.clone())) {
            succ.entry(edge.from.clone())
                .or_default()
                .insert(edge.to.clone());
            *indegree.get_mut(&edge.to).expect("edge target is a node") += 1;
        }
    }
    let mut ready: BTreeSet<AssetId> = indegree
        .iter()
        .filter_map(|(k, d)| (*d == 0).then(|| k.clone()))
        .collect();
    let mut out: Vec<AssetId> = Vec::with_capacity(nodes.len());
    while let Some(id) = ready.iter().next().cloned() {
        ready.remove(&id);
        out.push(id.clone());
        if let Some(succs) = succ.get(&id) {
            for s in succs {
                let d = indegree.get_mut(s).expect("successor counted");
                *d -= 1;
                if *d == 0 {
                    ready.insert(s.clone());
                }
            }
        }
    }
    // Nodes caught in a cycle (malformed input) fall in deterministically.
    for k in nodes.keys() {
        if !out.contains(k) {
            out.push(k.clone());
        }
    }
    out
}

/// Fold a run's live `.jsonl` [`StreamState`] onto a built view: each step in
/// the stream gets its `live_state` set (last-line-wins), and the run outcome /
/// completion are refreshed. Steps absent from the thinner stream keep their
/// authoritative `.json` state.
pub fn apply_stream(view: &mut ContractView, stream: &StreamState) {
    for (name, step) in &mut view.steps {
        if let Some(entry) = stream.steps.get(name) {
            step.live_state = Some(entry.state.clone());
        }
    }
    if let Some(outcome) = stream.outcome {
        view.run.outcome = outcome;
    }
    if stream.complete {
        view.run.complete = true;
    }
}
