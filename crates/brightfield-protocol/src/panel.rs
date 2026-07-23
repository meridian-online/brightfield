//! Framework-free derivation for the Protocol panel's outline + inspector.
//!
//! The live Protocol panel (`brightfield-shell`) and the headless
//! panel-composite render (`brightfield-render::panel_scene`) draw the SAME
//! derived rows and facts from this module — no UI framework, no vello.
//! Keeping the derivation here means the PNG proof and the live surface can
//! never drift: both read [`outline_rows`] and [`inspector_for`].
//!
//! - The **outline** is the graph's nodes in deterministic topological order
//!   (producer → consumer; a stable Kahn sort with an id-sorted ready set), the
//!   selected node marked. It is the tree the canvas selection syncs to.
//! - The **inspector** is the selected asset's S/R/C/E/D detail: its kind,
//!   dotted address, producing step + transform, execution status, and — when a
//!   contract measured them — row count / materialised / bytes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::contract::{SkipReason, StepState};
use crate::contract_graph::{AssetMeta, SeamStatus, StepView};
use crate::graph::{AssetGraph, AssetId, AssetKind, StepId};

/// One row of the outline tree — an asset placed in topological order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineRow {
    /// Stable dotted id (the outline's selection key + the yank address).
    pub id: AssetId,
    /// Display label (the node's own label).
    pub label: String,
    /// Visual class — the outline glyph/tint mirrors the canvas node.
    pub kind: AssetKind,
    /// Whether this row is the current selection (canvas ↔ outline sync).
    pub selected: bool,
    /// The producing step's execution status (`NotRun` for external inputs or
    /// an unmeasured offline manifest).
    pub status: SeamStatus,
}

/// The inspector facts for one selected asset — the detail pane's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectorFacts {
    /// `false` when nothing is selected (the empty inspector state).
    pub present: bool,
    /// The asset's display label — the inspector heading.
    pub label: String,
    /// The dotted address (the yank target; never a screen position).
    pub address: AssetId,
    /// Visual class.
    pub kind: AssetKind,
    /// The producing step name (`None` for an external input).
    pub producing_step: Option<String>,
    /// The transform: the op name, or the SQL model (`None` for an input).
    pub transform: Option<String>,
    /// Execution status of the producing step.
    pub status: SeamStatus,
    /// The producing step's recorded terminal state, verbatim from the run
    /// contract (`None` for an external input, or on the offline manifest
    /// path where no run exists). With `skip_reason`, the ingested pair a
    /// surface labels the previewed data's run-state from — the contract's
    /// own record, never a staleness re-computation.
    pub step_state: Option<StepState>,
    /// The producing step's typed skip reason, when the run recorded one.
    pub skip_reason: Option<SkipReason>,
    /// Row count when a contract measured it.
    pub row_count: Option<u64>,
    /// Whether the asset was actually materialised (`None` offline).
    pub materialized: Option<bool>,
    /// Size in bytes when measured.
    pub bytes: Option<u64>,
    /// The degrade reason on an Opaque chip.
    pub issue: Option<String>,
}

impl InspectorFacts {
    /// The empty inspector — nothing selected.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            present: false,
            label: String::new(),
            address: String::new(),
            kind: AssetKind::Table,
            producing_step: None,
            transform: None,
            status: SeamStatus::NotRun,
            step_state: None,
            skip_reason: None,
            row_count: None,
            materialized: None,
            bytes: None,
            issue: None,
        }
    }
}

/// A human tag for an [`AssetKind`] — the outline row + inspector kind chip.
#[must_use]
pub fn kind_label(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Source => "source",
        AssetKind::File => "file",
        AssetKind::Table => "table",
        AssetKind::Internal => "internal",
        AssetKind::Dataset => "dataset",
        AssetKind::Family => "family",
        AssetKind::Opaque => "opaque",
    }
}

/// A deterministic topological order over the graph's nodes (producer →
/// consumer). Kahn's algorithm with an id-sorted ready queue, so the outline is
/// stable across runs and independent of pixel geometry. Nodes left
/// in a cycle (there should be none) are appended in id order so every node
/// appears exactly once.
#[must_use]
pub fn outline_order(graph: &AssetGraph) -> Vec<AssetId> {
    let mut succs: BTreeMap<AssetId, BTreeSet<AssetId>> = BTreeMap::new();
    let mut indeg: BTreeMap<AssetId, usize> = BTreeMap::new();
    for id in graph.nodes.keys() {
        succs.entry(id.clone()).or_default();
        indeg.entry(id.clone()).or_insert(0);
    }
    let mut seen: BTreeSet<(AssetId, AssetId)> = BTreeSet::new();
    for edge in &graph.edges {
        if edge.from == edge.to
            || !graph.nodes.contains_key(&edge.from)
            || !graph.nodes.contains_key(&edge.to)
        {
            continue;
        }
        if seen.insert((edge.from.clone(), edge.to.clone()))
            && succs
                .get_mut(&edge.from)
                .expect("node")
                .insert(edge.to.clone())
        {
            *indeg.get_mut(&edge.to).expect("node") += 1;
        }
    }
    // Ready set: every node with no remaining producer, id-sorted (a BTreeSet
    // drains ascending — the deterministic tie-break).
    let mut ready: VecDeque<AssetId> = indeg
        .iter()
        .filter(|(_, d)| **d == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut ordered: Vec<AssetId> = Vec::with_capacity(graph.nodes.len());
    let mut placed: BTreeSet<AssetId> = BTreeSet::new();
    while let Some(id) = ready.pop_front() {
        if !placed.insert(id.clone()) {
            continue;
        }
        ordered.push(id.clone());
        // Re-collect the freshly-ready successors id-sorted so the queue stays
        // ascending — a small BTreeSet per node keeps the tie-break stable.
        let mut newly: BTreeSet<AssetId> = BTreeSet::new();
        for s in &succs[&id] {
            let d = indeg.get_mut(s).expect("node");
            *d = d.saturating_sub(1);
            if *d == 0 {
                newly.insert(s.clone());
            }
        }
        for s in newly {
            ready.push_back(s);
        }
    }
    // Any node stranded by a cycle: append id-sorted so the outline is total.
    for id in graph.nodes.keys() {
        if placed.insert(id.clone()) {
            ordered.push(id.clone());
        }
    }
    ordered
}

/// The producing step's execution status for `id` (`NotRun` when the node has
/// no producer or the status map has no entry).
fn status_of(
    graph: &AssetGraph,
    statuses: &BTreeMap<StepId, SeamStatus>,
    id: &AssetId,
) -> SeamStatus {
    graph
        .nodes
        .get(id)
        .and_then(|n| n.step.as_ref())
        .and_then(|step| statuses.get(step))
        .copied()
        .unwrap_or(SeamStatus::NotRun)
}

/// The outline rows for `graph` in topological order, marking `selected` and
/// tinting each by its producing step's `statuses` entry.
#[must_use]
pub fn outline_rows(
    graph: &AssetGraph,
    statuses: &BTreeMap<StepId, SeamStatus>,
    selected: Option<&AssetId>,
) -> Vec<OutlineRow> {
    outline_order(graph)
        .into_iter()
        .filter_map(|id| {
            let node = graph.nodes.get(&id)?;
            Some(OutlineRow {
                selected: selected == Some(&id),
                status: status_of(graph, statuses, &id),
                label: node.label.clone(),
                kind: node.kind,
                id,
            })
        })
        .collect()
}

/// The inspector detail for the selected asset `id`. `assets` / `steps` are the
/// contract-measured maps (empty on the offline manifest path — the inspector
/// then shows lineage detail only). `None` id → the empty inspector.
#[must_use]
pub fn inspector_for(
    graph: &AssetGraph,
    assets: &BTreeMap<AssetId, AssetMeta>,
    steps: &BTreeMap<StepId, StepView>,
    statuses: &BTreeMap<StepId, SeamStatus>,
    id: Option<&AssetId>,
) -> InspectorFacts {
    let Some(id) = id else {
        return InspectorFacts::empty();
    };
    let Some(node) = graph.nodes.get(id) else {
        return InspectorFacts::empty();
    };
    let step_view = node.step.as_ref().and_then(|s| steps.get(s));
    let transform = step_view.and_then(|s| {
        s.op.clone()
            .or_else(|| s.sql_text.as_ref().map(|_| "SQL model".to_string()))
    });
    let meta = assets.get(id);
    InspectorFacts {
        present: true,
        label: node.label.clone(),
        address: node.id.clone(),
        kind: node.kind,
        producing_step: node.step.clone(),
        transform,
        status: status_of(graph, statuses, id),
        step_state: step_view.map(|s| s.state),
        skip_reason: step_view.and_then(StepView::skip_reason_kind),
        row_count: meta.and_then(|m| m.row_count),
        materialized: meta.map(|m| m.materialized),
        bytes: meta.and_then(|m| m.bytes),
        issue: node.issue.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::build_graph;
    use crate::manifest::parse_manifest_str;
    use crate::{collapse_families, load_contract_with_stream};
    use std::path::Path;

    fn edgar_graph() -> AssetGraph {
        // The vendored 21-step manifest — the richest offline graph.
        let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/protocol/edgar_gleif/arcform.yaml");
        let text = std::fs::read_to_string(&manifest_path).expect("manifest");
        let manifest = parse_manifest_str(&text).expect("parse");
        let dir = manifest_path.parent().unwrap();
        let sources = crate::graph::load_model_sources(&manifest, dir);
        collapse_families(&build_graph(&manifest, &sources))
    }

    /// The outline is a total order over the nodes — every node once, producers
    /// before consumers, and stable across two builds.
    #[test]
    fn outline_is_total_topological_and_stable() {
        let graph = edgar_graph();
        let order = outline_order(&graph);
        assert_eq!(order.len(), graph.nodes.len(), "every node placed once");
        let unique: BTreeSet<_> = order.iter().collect();
        assert_eq!(unique.len(), order.len(), "no duplicates");
        // Producer precedes consumer for every real edge.
        let pos: BTreeMap<&AssetId, usize> =
            order.iter().enumerate().map(|(i, id)| (id, i)).collect();
        for edge in &graph.edges {
            if edge.from == edge.to {
                continue;
            }
            if let (Some(a), Some(b)) = (pos.get(&edge.from), pos.get(&edge.to)) {
                assert!(a <= b, "producer {} before consumer {}", edge.from, edge.to);
            }
        }
        assert_eq!(order, outline_order(&graph), "deterministic across builds");
    }

    /// Selecting a node marks exactly that outline row and populates the
    /// inspector from the same graph — the two-way-sync contract at the model
    /// layer (the windowed surface just renders these).
    #[test]
    fn selection_syncs_outline_and_inspector() {
        let graph = edgar_graph();
        let statuses = BTreeMap::new();
        let assets = BTreeMap::new();
        let steps = BTreeMap::new();
        let pick = outline_order(&graph)[graph.nodes.len() / 2].clone();

        let rows = outline_rows(&graph, &statuses, Some(&pick));
        let marked: Vec<_> = rows.iter().filter(|r| r.selected).collect();
        assert_eq!(marked.len(), 1, "exactly one row selected");
        assert_eq!(marked[0].id, pick);

        let facts = inspector_for(&graph, &assets, &steps, &statuses, Some(&pick));
        assert!(facts.present);
        assert_eq!(facts.address, pick, "inspector addresses the selection");
        assert_eq!(facts.kind, graph.nodes[&pick].kind);

        // Nothing selected → empty inspector, no marked row.
        assert!(!inspector_for(&graph, &assets, &steps, &statuses, None).present);
        assert!(outline_rows(&graph, &statuses, None)
            .iter()
            .all(|r| !r.selected));
    }

    /// On the contract path the inspector carries the measured row count +
    /// materialised flag (the S/R/C/E/D detail the offline path can't show).
    #[test]
    fn inspector_carries_contract_measurements() {
        let contract =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/sample_run.contract.json");
        let view = load_contract_with_stream(&contract, None).expect("contract");
        let graph = collapse_families(&view.graph);
        let statuses = view.seam_statuses();
        // widgets is a measured 5-row table.
        let id = graph
            .nodes
            .keys()
            .find(|k| k.ends_with("widgets"))
            .expect("widgets node")
            .clone();
        let facts = inspector_for(&graph, &view.assets, &view.steps, &statuses, Some(&id));
        assert_eq!(facts.row_count, Some(5), "measured row count surfaces");
        assert_eq!(facts.materialized, Some(true));
        assert!(facts.producing_step.is_some(), "widgets has a producer");
        // The run-state ingredients ride along verbatim: the recorded terminal
        // state and (here, none) the typed skip reason — ingested for the
        // surface to label with, never re-derived.
        assert_eq!(facts.step_state, Some(StepState::Success));
        assert_eq!(facts.skip_reason, None);
    }
}
