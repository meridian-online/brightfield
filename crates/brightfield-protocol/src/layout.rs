//! Deterministic Sugiyama layout (card 0025, pds-ac06).
//!
//! Hand-ported, dependency-free: longest-path layering left -> right, dummy
//! nodes lane edges that span more than one layer, crossing reduction is
//! exactly **4 median sweeps** (fixed — never iterate-to-convergence), and
//! every tie breaks by node id (lexicographic) — never by insertion or hash
//! order. `BTreeMap`/`BTreeSet`/`Vec` end-to-end and coordinates quantised
//! to whole pixels, so the same graph always yields the identical `Layout`.
//!
//! No text measurement here: card widths come from char-count heuristics
//! (geometry before pixels) so this crate stays vello-free.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::{AssetGraph, AssetId, AssetKind, StepId};

/// A node's placed rectangle, whole pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// One routed edge: polyline waypoints from the source's right edge through
/// each dummy lane point to the target's left edge (the renderer draws the
/// orthogonal path along them).
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeRoute {
    /// Upstream node id.
    pub from: AssetId,
    /// Downstream node id.
    pub to: AssetId,
    /// The seam the flow passes through (chevron site), if any.
    pub via: Option<StepId>,
    /// Gate shield on this edge (pds-ac05).
    pub shield: bool,
    /// Waypoints, whole pixels.
    pub points: Vec<(f64, f64)>,
}

/// The layout result.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Canvas width in pixels.
    pub width: f64,
    /// Canvas height in pixels.
    pub height: f64,
    /// Placed rectangles for every REAL node (dummies stay internal).
    pub positions: BTreeMap<AssetId, Rect>,
    /// Routed edges along the dummy-node lanes.
    pub lanes: Vec<EdgeRoute>,
}

/// Tunable spacing; the default is the dump arm's configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutConfig {
    /// Canvas margin on all sides.
    pub margin: f64,
    /// Horizontal gap between layer columns.
    pub col_gap: f64,
    /// Vertical gap between nodes in a column.
    pub row_gap: f64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self { margin: 32.0, col_gap: 64.0, row_gap: 20.0 }
    }
}

/// Card width from the label's char count (the Dagster trick: geometry
/// before pixels; ~7px per char at the render module's 11px Inter).
fn node_width(kind: AssetKind, label: &str) -> f64 {
    let chars = label.chars().count().min(28) as f64;
    let base = (24.0 + chars * 7.0).clamp(64.0, 224.0);
    match kind {
        AssetKind::Family => base + 28.0, // room for the xN badge
        _ => base,
    }
}

/// Card height per node class (pds-ac05's distinguishable treatments).
fn node_height(kind: AssetKind) -> f64 {
    match kind {
        AssetKind::Source => 30.0,
        AssetKind::File => 34.0,
        AssetKind::Table => 36.0,
        AssetKind::Internal | AssetKind::Opaque => 26.0,
        AssetKind::Dataset => 42.0,
        AssetKind::Family => 46.0,
    }
}

/// Compute the deterministic layout for `graph`.
#[must_use]
pub fn layout(graph: &AssetGraph, config: &LayoutConfig) -> Layout {
    // Unique (from, to) pairs drive layering + routing; parallel edges merge
    // into one lane that keeps the FIRST seam via and ORs any shield. A None
    // via never shadows a Some (collapse re-targeting can emit a via-less edge
    // alongside a seam-bearing one), so the merged lane always draws its
    // chevron when any parallel edge carries a seam.
    let mut unique: BTreeMap<(AssetId, AssetId), (Option<StepId>, bool)> = BTreeMap::new();
    for edge in &graph.edges {
        if edge.from == edge.to
            || !graph.nodes.contains_key(&edge.from)
            || !graph.nodes.contains_key(&edge.to)
        {
            continue;
        }
        let entry = unique
            .entry((edge.from.clone(), edge.to.clone()))
            .or_insert((edge.via.clone(), edge.shield));
        if entry.0.is_none() {
            entry.0 = edge.via.clone();
        }
        entry.1 = entry.1 || edge.shield;
    }

    let mut succs: BTreeMap<&AssetId, BTreeSet<&AssetId>> = BTreeMap::new();
    let mut preds: BTreeMap<&AssetId, BTreeSet<&AssetId>> = BTreeMap::new();
    for (from, to) in unique.keys() {
        succs.entry(from).or_default().insert(to);
        preds.entry(to).or_default().insert(from);
    }

    // Longest-path layering via Kahn's algorithm in id order. Nodes caught in
    // a cycle (malformed input) fall deterministically into layer 0.
    let mut layer: BTreeMap<&AssetId, usize> = graph.nodes.keys().map(|id| (id, 0)).collect();
    let mut indegree: BTreeMap<&AssetId, usize> =
        graph.nodes.keys().map(|id| (id, preds.get(id).map_or(0, BTreeSet::len))).collect();
    let mut ready: BTreeSet<&AssetId> = indegree
        .iter()
        .filter_map(|(id, d)| (*d == 0).then_some(*id))
        .collect();
    while let Some(&id) = ready.iter().next() {
        ready.remove(&id);
        let l = layer[&id];
        for &succ in succs.get(&id).into_iter().flatten() {
            if layer[&succ] < l + 1 {
                *layer.get_mut(&succ).expect("all nodes layered") = l + 1;
            }
            let d = indegree.get_mut(&succ).expect("all nodes counted");
            *d = d.saturating_sub(1);
            if *d == 0 {
                ready.insert(succ);
            }
        }
    }
    let n_layers = layer.values().copied().max().unwrap_or(0) + 1;

    // Ordering slots: real nodes plus dummy chains for edges spanning >1
    // layer. Dummy ids derive from the (from, to) pair — deterministic.
    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
    enum Slot {
        Real(AssetId),
        Dummy { edge: (AssetId, AssetId), hop: usize },
    }
    let slot_id = |slot: &Slot| -> String {
        match slot {
            Slot::Real(id) => id.clone(),
            Slot::Dummy { edge, hop } => format!("dummy.{}->{}#{hop}", edge.0, edge.1),
        }
    };

    let mut layers: Vec<Vec<Slot>> = vec![Vec::new(); n_layers];
    for (id, l) in &layer {
        layers[*l].push(Slot::Real((*id).clone()));
    }
    // Dummy chains + the slot-level adjacency used by the median sweeps.
    let mut slot_succs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut slot_preds: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let link = |a: &Slot, b: &Slot, succs: &mut BTreeMap<String, BTreeSet<String>>,
                    preds: &mut BTreeMap<String, BTreeSet<String>>| {
        succs.entry(slot_id(a)).or_default().insert(slot_id(b));
        preds.entry(slot_id(b)).or_default().insert(slot_id(a));
    };
    let mut routes_chain: BTreeMap<(AssetId, AssetId), Vec<Slot>> = BTreeMap::new();
    for (from, to) in unique.keys() {
        let (lf, lt) = (layer[from], layer[to]);
        let mut chain = Vec::new();
        if lt > lf + 1 {
            for (hop, l) in (lf + 1..lt).enumerate() {
                let dummy = Slot::Dummy { edge: (from.clone(), to.clone()), hop };
                layers[l].push(dummy.clone());
                chain.push(dummy);
            }
        }
        let mut prev = Slot::Real(from.clone());
        for d in &chain {
            link(&prev, d, &mut slot_succs, &mut slot_preds);
            prev = d.clone();
        }
        link(&prev, &Slot::Real(to.clone()), &mut slot_succs, &mut slot_preds);
        routes_chain.insert((from.clone(), to.clone()), chain);
    }
    for l in &mut layers {
        l.sort_by_key(|s| slot_id(s));
    }

    // Crossing reduction: exactly 4 median sweeps (down, up, down, up), ties
    // by node id — the fixed-iteration rule that pins pds-ac06.
    for sweep in 0..4 {
        let downward = sweep % 2 == 0;
        let range: Vec<usize> = if downward {
            (1..n_layers).collect()
        } else {
            (0..n_layers.saturating_sub(1)).rev().collect()
        };
        for l in range {
            let neighbour_layer = if downward { l - 1 } else { l + 1 };
            let neighbour_index: BTreeMap<String, usize> = layers[neighbour_layer]
                .iter()
                .enumerate()
                .map(|(i, s)| (slot_id(s), i))
                .collect();
            let lookup = if downward { &slot_preds } else { &slot_succs };
            // 2x the median of neighbour indices keeps the key integral.
            let keys: Vec<(i64, String)> = layers[l]
                .iter()
                .enumerate()
                .map(|(current, slot)| {
                    let id = slot_id(slot);
                    let mut idx: Vec<i64> = lookup
                        .get(&id)
                        .into_iter()
                        .flatten()
                        .filter_map(|n| neighbour_index.get(n).map(|i| *i as i64))
                        .collect();
                    idx.sort_unstable();
                    let median2 = if idx.is_empty() {
                        2 * current as i64 // no neighbours: hold position
                    } else if idx.len() % 2 == 1 {
                        2 * idx[idx.len() / 2]
                    } else {
                        idx[idx.len() / 2 - 1] + idx[idx.len() / 2]
                    };
                    (median2, id)
                })
                .collect();
            let mut order: Vec<usize> = (0..layers[l].len()).collect();
            order.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
            let reordered: Vec<Slot> = order.into_iter().map(|i| layers[l][i].clone()).collect();
            layers[l] = reordered;
        }
    }

    // Coordinates: per-layer columns sized to the widest card, stacks
    // vertically centred on the tallest column, whole pixels throughout.
    let size_of = |slot: &Slot| -> (f64, f64) {
        match slot {
            Slot::Real(id) => {
                let node = &graph.nodes[id];
                (node_width(node.kind, &node.label), node_height(node.kind))
            }
            Slot::Dummy { .. } => (0.0, 10.0),
        }
    };
    let col_widths: Vec<f64> = layers
        .iter()
        .map(|l| l.iter().map(|s| size_of(s).0).fold(48.0, f64::max))
        .collect();
    let col_heights: Vec<f64> = layers
        .iter()
        .map(|l| {
            let heights: f64 = l.iter().map(|s| size_of(s).1).sum();
            heights + config.row_gap * (l.len().saturating_sub(1)) as f64
        })
        .collect();
    let tallest = col_heights.iter().copied().fold(0.0, f64::max);

    let mut positions: BTreeMap<AssetId, Rect> = BTreeMap::new();
    let mut lane_points: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut x = config.margin;
    for (l, slots) in layers.iter().enumerate() {
        let col_w = col_widths[l];
        let mut y = config.margin + ((tallest - col_heights[l]) / 2.0).max(0.0);
        for slot in slots {
            let (w, h) = size_of(slot);
            match slot {
                Slot::Real(id) => {
                    positions.insert(
                        id.clone(),
                        Rect {
                            x: (x + (col_w - w) / 2.0).round(),
                            y: y.round(),
                            width: w.round(),
                            height: h.round(),
                        },
                    );
                }
                Slot::Dummy { .. } => {
                    lane_points.insert(
                        slot_id(slot),
                        ((x + col_w / 2.0).round(), (y + h / 2.0).round()),
                    );
                }
            }
            y += h + config.row_gap;
        }
        x += col_w + config.col_gap;
    }
    let width = (x - config.col_gap + config.margin).round();
    let height = (tallest + 2.0 * config.margin).round();

    // Routes: from-right-edge -> dummy lane points -> to-left-edge.
    let lanes: Vec<EdgeRoute> = unique
        .iter()
        .map(|((from, to), (via, shield))| {
            let f = &positions[from];
            let t = &positions[to];
            let mut points = vec![(f.x + f.width, (f.y + f.height / 2.0).round())];
            for d in &routes_chain[&(from.clone(), to.clone())] {
                points.push(lane_points[&slot_id(d)]);
            }
            points.push((t.x, (t.y + t.height / 2.0).round()));
            EdgeRoute {
                from: from.clone(),
                to: to.clone(),
                via: via.clone(),
                shield: *shield,
                points,
            }
        })
        .collect();

    Layout { width, height, positions, lanes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{build_graph, AssetGraph, AssetNode, Edge};
    use crate::manifest::parse_manifest_str;

    /// A two-node graph reached by parallel edges: a via-less one (as collapse
    /// re-targeting emits) FIRST, then a seam-bearing one.
    fn parallel_edges(via_first: Option<&str>, via_second: Option<&str>) -> AssetGraph {
        let node = |id: &str| AssetNode {
            id: id.to_string(),
            kind: AssetKind::File,
            label: id.to_string(),
            step: None,
            family_count: None,
            issue: None,
        };
        let edge = |via: Option<&str>| Edge {
            from: "file.p.a".to_string(),
            to: "file.p.b".to_string(),
            via: via.map(str::to_string),
            shield: false,
        };
        AssetGraph {
            protocol: "p".to_string(),
            nodes: [("file.p.a".to_string(), node("file.p.a")), ("file.p.b".to_string(), node("file.p.b"))]
                .into_iter()
                .collect(),
            seams: BTreeMap::new(),
            edges: vec![edge(via_first), edge(via_second)],
        }
    }

    #[test]
    fn pds_parallel_edge_merge_keeps_the_seam_via() {
        // A None-via edge listed FIRST must not shadow the Some-via sibling —
        // the merged lane keeps the chevron.
        let g = parallel_edges(None, Some("transform"));
        let l = layout(&g, &LayoutConfig::default());
        assert_eq!(l.lanes.len(), 1);
        assert_eq!(l.lanes[0].via.as_deref(), Some("transform"), "the seam via survives the merge");
    }

    fn diamond() -> AssetGraph {
        // fetch feeds two extracts which both feed a loader — plus one edge
        // spanning two layers (fetch's file read directly by the loader), so
        // a dummy lane exists.
        let yaml = r"
name: d
steps:
  - name: fetch
    op: http_fetch@1
    with: { url: 'https://h.example/a', out: build/a }
  - name: left
    op: archive_extract@1
    with: { archive: build/a, dest: build/l }
  - name: right
    op: archive_extract@1
    with: { archive: build/a, dest: build/r }
  - name: join
    sql: models/j.sql
    depends_on: [build/l, build/r, build/a]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = std::collections::BTreeMap::new();
        sources.insert(
            "join".to_string(),
            Ok("CREATE TABLE j AS SELECT * FROM read_csv('build/l/x.csv') \
                UNION ALL SELECT * FROM read_csv('build/r/x.csv') \
                UNION ALL SELECT * FROM read_csv('build/a');"
                .to_string()),
        );
        build_graph(&manifest, &sources)
    }

    #[test]
    fn pds_ac06_layout_repeated_call_equality() {
        let g = diamond();
        let cfg = LayoutConfig::default();
        assert_eq!(layout(&g, &cfg), layout(&g, &cfg));
    }

    #[test]
    fn pds_ac06_whole_pixel_quantisation() {
        let g = diamond();
        let l = layout(&g, &LayoutConfig::default());
        assert_eq!(l.width.fract(), 0.0);
        assert_eq!(l.height.fract(), 0.0);
        for r in l.positions.values() {
            for v in [r.x, r.y, r.width, r.height] {
                assert_eq!(v.fract(), 0.0, "whole pixels only: {r:?}");
            }
        }
        for lane in &l.lanes {
            for (px, py) in &lane.points {
                assert_eq!(px.fract(), 0.0);
                assert_eq!(py.fract(), 0.0);
            }
        }
    }

    #[test]
    fn pds_layout_flows_left_to_right() {
        let g = diamond();
        let l = layout(&g, &LayoutConfig::default());
        for lane in &l.lanes {
            let f = &l.positions[&lane.from];
            let t = &l.positions[&lane.to];
            assert!(
                f.x + f.width <= t.x,
                "downstream sits strictly right: {} -> {}",
                lane.from,
                lane.to
            );
        }
        // Every real node is placed; the multi-layer span routed through a
        // dummy lane (3+ waypoints).
        assert_eq!(l.positions.len(), g.nodes.len());
        assert!(
            l.lanes.iter().any(|lane| lane.points.len() > 2),
            "the layer-spanning edge routes through a dummy lane"
        );
    }
}
