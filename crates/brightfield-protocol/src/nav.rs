//! Topological navigation over the asset graph (card 0029, AC #2) —
//! framework-free.
//!
//! The keyboard grammar's motion layer as a pure state machine: `h`/`l` walk
//! the DAG's edges to a producer/consumer, `j`/`k` step between rank siblings
//! (the nodes sharing a layer), `za` folds/unfolds a parameterised family, and
//! `Enter`/`Esc` push and pop a drill stack whose breadcrumb tracks the path.
//! Everything is **topological** — the moves walk the graph, never pixel
//! geometry — so a re-layout (or the horizontal↔vertical flip) leaves the
//! grammar unchanged (doc-25 §5). No gpui, no vello: the app's key handler
//! calls these methods and re-reads [`cursor`](ProtocolNav::cursor); the
//! headless tests drive the same surface.
//!
//! Determinism: adjacency and sibling order are `BTreeMap`/`Vec`-of-sorted-ids,
//! so `h` from a fan-in node always lands on the same producer.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::{AssetGraph, AssetId, AssetKind};

/// What a fold toggle did — a family folds/unfolds; anything else is rejected so
/// a mis-aimed `za` is a no-op, not a silent state change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldOutcome {
    /// The family under the cursor is now expanded (its members shown).
    Unfolded,
    /// The family under the cursor is now collapsed (one tile).
    Folded,
    /// The cursor is not on a collapsible family — nothing happened.
    NotAFamily,
}

/// The protocol panel's navigation + drill + fold state over one asset graph.
#[derive(Debug, Clone)]
pub struct ProtocolNav {
    /// Consumers of each node (downstream), id-sorted — `l` targets.
    succs: BTreeMap<AssetId, Vec<AssetId>>,
    /// Producers of each node (upstream), id-sorted — `h` targets.
    preds: BTreeMap<AssetId, Vec<AssetId>>,
    /// Longest-path layer per node (the `j`/`k` sibling grouping).
    layer: BTreeMap<AssetId, usize>,
    /// Nodes per layer, id-sorted — the sibling ring for `j`/`k`.
    ranks: Vec<Vec<AssetId>>,
    /// Which nodes are Family tiles (only these accept `za`).
    families: BTreeSet<AssetId>,
    /// Families currently expanded (`za` toggles membership).
    expanded: BTreeSet<AssetId>,
    /// The focused node.
    cursor: Option<AssetId>,
    /// The drill path — `Enter` pushes the cursor, `Esc` pops (doc-25 §5).
    breadcrumb: Vec<AssetId>,
}

impl ProtocolNav {
    /// Build the navigator over `graph`. The initial cursor is the first root
    /// (a node with no producer) in id order, else the first node — a stable,
    /// upstream starting point.
    #[must_use]
    pub fn new(graph: &AssetGraph) -> Self {
        let mut succs: BTreeMap<AssetId, Vec<AssetId>> = BTreeMap::new();
        let mut preds: BTreeMap<AssetId, Vec<AssetId>> = BTreeMap::new();
        for id in graph.nodes.keys() {
            succs.entry(id.clone()).or_default();
            preds.entry(id.clone()).or_default();
        }
        let mut pair_seen: BTreeSet<(AssetId, AssetId)> = BTreeSet::new();
        for edge in &graph.edges {
            if edge.from == edge.to
                || !graph.nodes.contains_key(&edge.from)
                || !graph.nodes.contains_key(&edge.to)
            {
                continue;
            }
            if pair_seen.insert((edge.from.clone(), edge.to.clone())) {
                succs.get_mut(&edge.from).expect("node").push(edge.to.clone());
                preds.get_mut(&edge.to).expect("node").push(edge.from.clone());
            }
        }
        // Sorted adjacency → deterministic `h`/`l` landing.
        for v in succs.values_mut() {
            v.sort();
        }
        for v in preds.values_mut() {
            v.sort();
        }

        let layer = longest_path_layers(graph, &succs, &preds);
        let n_layers = layer.values().copied().max().map_or(0, |m| m + 1);
        let mut ranks: Vec<Vec<AssetId>> = vec![Vec::new(); n_layers];
        for (id, l) in &layer {
            ranks[*l].push(id.clone());
        }
        for r in &mut ranks {
            r.sort();
        }

        let families: BTreeSet<AssetId> = graph
            .nodes
            .iter()
            .filter(|(_, n)| n.kind == AssetKind::Family)
            .map(|(id, _)| id.clone())
            .collect();

        // Prefer a root (no producer) as the entry cursor.
        let cursor = graph
            .nodes
            .keys()
            .find(|id| preds.get(*id).is_none_or(Vec::is_empty))
            .or_else(|| graph.nodes.keys().next())
            .cloned();

        Self {
            succs,
            preds,
            layer,
            ranks,
            families,
            expanded: BTreeSet::new(),
            cursor,
            breadcrumb: Vec::new(),
        }
    }

    /// The focused node.
    #[must_use]
    pub fn cursor(&self) -> Option<&AssetId> {
        self.cursor.as_ref()
    }

    /// Point the cursor at `id` (a click / outline-sync entry point). Ignored
    /// when `id` is not a node.
    pub fn focus(&mut self, id: &AssetId) {
        if self.layer.contains_key(id) {
            self.cursor = Some(id.clone());
        }
    }

    /// `h` — move to the focused node's producer (topological upstream). The
    /// FIRST producer in id order on a fan-in. Returns whether the cursor moved.
    pub fn to_producer(&mut self) -> bool {
        self.step_edge(true)
    }

    /// `l` — move to the focused node's consumer (topological downstream).
    pub fn to_consumer(&mut self) -> bool {
        self.step_edge(false)
    }

    fn step_edge(&mut self, upstream: bool) -> bool {
        let Some(cur) = &self.cursor else { return false };
        let table = if upstream { &self.preds } else { &self.succs };
        if let Some(next) = table.get(cur).and_then(|v| v.first()) {
            self.cursor = Some(next.clone());
            true
        } else {
            false
        }
    }

    /// `j` — move to the next rank sibling (down the layer, id order).
    pub fn to_sibling_next(&mut self) -> bool {
        self.step_sibling(1)
    }

    /// `k` — move to the previous rank sibling (up the layer, id order).
    pub fn to_sibling_prev(&mut self) -> bool {
        self.step_sibling(-1)
    }

    fn step_sibling(&mut self, delta: isize) -> bool {
        let Some(cur) = &self.cursor else { return false };
        let Some(&l) = self.layer.get(cur) else { return false };
        let rank = &self.ranks[l];
        let Some(pos) = rank.iter().position(|id| id == cur) else { return false };
        let next = pos as isize + delta;
        if next < 0 || next as usize >= rank.len() {
            return false; // edge of the layer — no wrap (a wall the user feels)
        }
        self.cursor = Some(rank[next as usize].clone());
        true
    }

    /// `za` — fold/unfold the parameterised family under the cursor. A no-op
    /// (returns [`FoldOutcome::NotAFamily`]) when the cursor is not on a Family
    /// tile — folds only ever act on collapsible nodes.
    pub fn toggle_fold(&mut self) -> FoldOutcome {
        let Some(cur) = self.cursor.clone() else { return FoldOutcome::NotAFamily };
        if !self.families.contains(&cur) {
            return FoldOutcome::NotAFamily;
        }
        if self.expanded.remove(&cur) {
            FoldOutcome::Folded
        } else {
            self.expanded.insert(cur);
            FoldOutcome::Unfolded
        }
    }

    /// Whether the family under `id` is currently expanded.
    #[must_use]
    pub fn is_expanded(&self, id: &AssetId) -> bool {
        self.expanded.contains(id)
    }

    /// `Enter` — drill into the focused node: push it onto the drill stack. The
    /// breadcrumb grows by one; returns whether a node was pushed.
    pub fn drill_in(&mut self) -> bool {
        if let Some(cur) = self.cursor.clone() {
            self.breadcrumb.push(cur);
            true
        } else {
            false
        }
    }

    /// `Esc` — pop one level off the drill stack, returning the cursor to the
    /// parent level. Returns whether a level was popped.
    pub fn drill_out(&mut self) -> bool {
        if self.breadcrumb.pop().is_none() {
            return false;
        }
        // The cursor returns to the now-deepest breadcrumb entry (the parent),
        // or is left where it is when the stack empties.
        if let Some(parent) = self.breadcrumb.last() {
            self.cursor = Some(parent.clone());
        }
        true
    }

    /// The drill breadcrumb, root → deepest — the bottom-bar drill path.
    #[must_use]
    pub fn breadcrumb(&self) -> &[AssetId] {
        &self.breadcrumb
    }
}

/// Longest-path layering (Kahn's algorithm in id order) — the same rule
/// `layout` uses, so the sibling grouping matches the drawn columns. Cyclic
/// nodes (malformed input) fall into layer 0 deterministically.
fn longest_path_layers(
    graph: &AssetGraph,
    succs: &BTreeMap<AssetId, Vec<AssetId>>,
    preds: &BTreeMap<AssetId, Vec<AssetId>>,
) -> BTreeMap<AssetId, usize> {
    let mut layer: BTreeMap<AssetId, usize> = graph.nodes.keys().map(|id| (id.clone(), 0)).collect();
    let mut indegree: BTreeMap<AssetId, usize> =
        graph.nodes.keys().map(|id| (id.clone(), preds.get(id).map_or(0, Vec::len))).collect();
    let mut ready: BTreeSet<AssetId> =
        indegree.iter().filter_map(|(id, d)| (*d == 0).then(|| id.clone())).collect();
    while let Some(id) = ready.iter().next().cloned() {
        ready.remove(&id);
        let l = layer[&id];
        for succ in succs.get(&id).into_iter().flatten() {
            if layer[succ] < l + 1 {
                *layer.get_mut(succ).expect("node layered") = l + 1;
            }
            let d = indegree.get_mut(succ).expect("node counted");
            *d = d.saturating_sub(1);
            if *d == 0 {
                ready.insert(succ.clone());
            }
        }
    }
    layer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collapse::collapse_families;
    use crate::graph::build_graph;
    use crate::manifest::parse_manifest_str;
    use std::collections::BTreeMap;

    /// fetch → transform → export, a straight chain, for motion tests.
    fn chain_graph() -> AssetGraph {
        let yaml = r"
name: chain
steps:
  - name: fetch
    op: http_fetch@1
    with: { url: 'https://example.com/a.csv', out: build/a.csv }
  - name: transform
    sql: models/t.sql
    depends_on: [build/a.csv]
  - name: export
    op: parquet_export@1
    with: { input: t_out, dest: build/t.parquet }
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "transform".to_string(),
            Ok("CREATE TABLE t_out AS SELECT * FROM read_csv('build/a.csv');".to_string()),
        );
        build_graph(&manifest, &sources)
    }

    #[test]
    fn t29_hl_walks_producer_and_consumer_topologically() {
        let g = chain_graph();
        let mut nav = ProtocolNav::new(&g);
        // The entry cursor is a root (the source, no producer).
        let start = nav.cursor().cloned().unwrap();
        assert!(nav.preds[&start].is_empty(), "entry cursor is a root: {start}");
        // Walking `l` reaches strictly deeper layers; each hop moves.
        let mut prev_layer = nav.layer[&start];
        let mut hops = 0;
        while nav.to_consumer() {
            let cur = nav.cursor().cloned().unwrap();
            assert!(nav.layer[&cur] > prev_layer, "`l` descends: {cur}");
            prev_layer = nav.layer[&cur];
            hops += 1;
        }
        assert!(hops >= 2, "the chain has several downstream hops");
        // From the sink, `h` walks back upstream to the root.
        while nav.to_producer() {}
        let back = nav.cursor().cloned().unwrap();
        assert!(nav.preds[&back].is_empty(), "`h` returns to a root: {back}");
    }

    #[test]
    fn t29_hl_are_walls_at_the_ends() {
        let g = chain_graph();
        let mut nav = ProtocolNav::new(&g);
        // At a root, `h` cannot move.
        assert!(!nav.to_producer(), "no producer past a root");
        // At the sink, `l` cannot move.
        while nav.to_consumer() {}
        assert!(!nav.to_consumer(), "no consumer past the sink");
    }

    #[test]
    fn t29_jk_step_rank_siblings_without_wrapping() {
        // A fan-out layer: two files produced from one source feed one join, so
        // the middle layer holds two rank siblings.
        let yaml = r"
name: fan
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
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let g = build_graph(&manifest, &BTreeMap::new());
        let mut nav = ProtocolNav::new(&g);
        // Move to a node in the two-sibling layer (build/l and build/r share it).
        nav.focus(&"file.fan.build/l".to_string());
        let l = nav.layer[&"file.fan.build/l".to_string()];
        assert_eq!(nav.ranks[l].len(), 2, "two rank siblings in this layer");
        // `j` moves to the next sibling; `j` again is a wall (no wrap).
        assert!(nav.to_sibling_next());
        assert_eq!(nav.cursor().unwrap(), "file.fan.build/r");
        assert!(!nav.to_sibling_next(), "no wrap at the layer edge");
        // `k` steps back.
        assert!(nav.to_sibling_prev());
        assert_eq!(nav.cursor().unwrap(), "file.fan.build/l");
        assert!(!nav.to_sibling_prev(), "no wrap at the other edge");
    }

    #[test]
    fn t29_za_folds_only_a_family_tile() {
        // The N-CEN-style family collapses to a Family tile; `za` toggles it and
        // is a no-op anywhere else.
        let yaml = r"
name: fam
steps:
  - name: fetch_x_q1
    op: http_fetch@1
    with: { url: 'https://h.example/q1.zip', out: build/z/q1.zip }
  - name: extract_x_q1
    op: archive_extract@1
    with: { archive: build/z/q1.zip, dest: build/x/q1 }
  - name: fetch_x_q2
    op: http_fetch@1
    with: { url: 'https://h.example/q2.zip', out: build/z/q2.zip }
  - name: extract_x_q2
    op: archive_extract@1
    with: { archive: build/z/q2.zip, dest: build/x/q2 }
  - name: load
    sql: models/load.sql
    depends_on: [build/x/q1, build/x/q2]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "load".to_string(),
            Ok("CREATE TABLE loaded AS SELECT * FROM read_csv('build/x/*/data.csv');".to_string()),
        );
        let g = collapse_families(&build_graph(&manifest, &sources));
        let family_id = g
            .nodes
            .iter()
            .find(|(_, n)| n.kind == AssetKind::Family)
            .map(|(id, _)| id.clone())
            .expect("a family tile exists");
        let mut nav = ProtocolNav::new(&g);
        // On a non-family node, `za` is a no-op.
        nav.focus(&g.nodes.keys().find(|id| **id != family_id).unwrap().clone());
        assert_eq!(nav.toggle_fold(), FoldOutcome::NotAFamily);
        // On the family tile, `za` toggles expand/collapse.
        nav.focus(&family_id);
        assert_eq!(nav.toggle_fold(), FoldOutcome::Unfolded);
        assert!(nav.is_expanded(&family_id));
        assert_eq!(nav.toggle_fold(), FoldOutcome::Folded);
        assert!(!nav.is_expanded(&family_id));
    }

    #[test]
    fn t29_enter_esc_push_and_pop_the_drill_stack() {
        let g = chain_graph();
        let mut nav = ProtocolNav::new(&g);
        assert!(nav.breadcrumb().is_empty());
        let root = nav.cursor().cloned().unwrap();
        assert!(nav.drill_in());
        assert_eq!(nav.breadcrumb(), &[root.clone()]);
        // Move down, drill again — the breadcrumb tracks the path.
        nav.to_consumer();
        let child = nav.cursor().cloned().unwrap();
        assert!(nav.drill_in());
        assert_eq!(nav.breadcrumb(), &[root.clone(), child.clone()]);
        // Esc pops one level and returns the cursor to the parent.
        assert!(nav.drill_out());
        assert_eq!(nav.breadcrumb(), &[root.clone()]);
        assert_eq!(nav.cursor().unwrap(), &root);
        // Esc to empty, then Esc on an empty stack is a wall.
        assert!(nav.drill_out());
        assert!(nav.breadcrumb().is_empty());
        assert!(!nav.drill_out());
    }

    #[test]
    fn t29_nav_construction_is_deterministic() {
        let g = chain_graph();
        let a = ProtocolNav::new(&g);
        let b = ProtocolNav::new(&g);
        assert_eq!(a.cursor(), b.cursor());
        assert_eq!(a.ranks, b.ranks);
    }
}
