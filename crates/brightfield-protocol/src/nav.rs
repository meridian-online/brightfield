//! Spatial, flow-aware navigation over the asset graph —
//! framework-free.
//!
//! The keyboard grammar's motion layer as a pure state machine. The vim keys
//! move the cursor in their **rendered** direction ([`move_dir`](ProtocolNav::move_dir)):
//! the reading [`Flow`] decides which axis is the producer→consumer flow and
//! which stacks siblings, so the same key follows what is on screen.
//!
//! - **Vertical flow** (top→bottom): `k`/up → producer, `j`/down → consumer;
//!   `h`/left and `l`/right step to the rank sibling on that side.
//! - **Horizontal flow** (left→right): `h`/left → producer, `l`/right →
//!   consumer; `k`/up and `j`/down step to the sibling above/below.
//!
//! Where a direction is ambiguous — a fan of producers/consumers, or several
//! siblings — the move resolves to the node whose rendered centre is spatially
//! nearest in the pressed direction (fed by [`set_geometry`](ProtocolNav::set_geometry)),
//! so movement always matches the drawn layout. `za` folds/unfolds a
//! parameterised family and `Enter`/`Esc` push/pop a drill stack whose
//! breadcrumb tracks the path (no consecutive duplicates). No gpui, no vello:
//! the app's key handler calls these methods and re-reads
//! [`cursor`](ProtocolNav::cursor); the headless tests drive the same surface.
//!
//! Determinism: adjacency and sibling sets are `BTreeMap`/`Vec`-of-sorted-ids
//! and geometry ties break by node id, so a move is reproducible.

use std::collections::{BTreeMap, BTreeSet};

use crate::graph::{AssetGraph, AssetId, AssetKind};
use crate::layout::{Flow, Layout};

/// A pressed vim direction. Resolved against the current [`Flow`] to a
/// producer/consumer step (along the flow axis) or a sibling step (across it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// `k`
    Up,
    /// `j`
    Down,
    /// `h`
    Left,
    /// `l`
    Right,
}

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
    /// The drill path — `Enter` pushes the cursor, `Esc` pops.
    breadcrumb: Vec<AssetId>,
    /// The reading axis the spatial keys resolve against
    /// ([`set_geometry`](ProtocolNav::set_geometry)).
    flow: Flow,
    /// Rendered node centres — the geometry `move_dir` honours. Empty until the
    /// shell calls [`set_geometry`](ProtocolNav::set_geometry); the topological
    /// primitives work without it.
    centers: BTreeMap<AssetId, (f64, f64)>,
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
                succs
                    .get_mut(&edge.from)
                    .expect("node")
                    .push(edge.to.clone());
                preds
                    .get_mut(&edge.to)
                    .expect("node")
                    .push(edge.from.clone());
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
            flow: Flow::default(),
            centers: BTreeMap::new(),
        }
    }

    /// Record the rendered geometry (node centres) + reading `flow` the spatial
    /// [`move_dir`](ProtocolNav::move_dir) keys honour. Call whenever the layout
    /// the user sees is (re)computed so a keystroke matches what is on screen.
    /// Only nodes this nav knows are kept — a scoped/expanded layout's extra
    /// nodes are ignored, so the cursor is always addressable.
    pub fn set_geometry(&mut self, flow: Flow, layout: &Layout) {
        self.flow = flow;
        self.centers = layout
            .positions
            .iter()
            .filter(|(id, _)| self.layer.contains_key(*id))
            .map(|(id, r)| (id.clone(), (r.x + r.width / 2.0, r.y + r.height / 2.0)))
            .collect();
    }

    /// The reading axis the spatial keys resolve against.
    #[must_use]
    pub fn flow(&self) -> Flow {
        self.flow
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
        let Some(cur) = &self.cursor else {
            return false;
        };
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
        let Some(cur) = &self.cursor else {
            return false;
        };
        let Some(&l) = self.layer.get(cur) else {
            return false;
        };
        let rank = &self.ranks[l];
        let Some(pos) = rank.iter().position(|id| id == cur) else {
            return false;
        };
        let next = pos as isize + delta;
        if next < 0 || next as usize >= rank.len() {
            return false; // edge of the layer — no wrap (a wall the user feels)
        }
        self.cursor = Some(rank[next as usize].clone());
        true
    }

    /// Move the cursor one node in the pressed vim direction **as rendered**.
    /// The [`Flow`] decides the axis: along the flow, `Up`/`Left` reach the
    /// producer and `Down`/`Right` the consumer; across it the same keys step to
    /// the sibling on that side. Ambiguity (a fan of neighbours, or several
    /// siblings) resolves to the node whose centre is spatially nearest in the
    /// pressed direction. Returns whether the cursor moved.
    pub fn move_dir(&mut self, dir: Dir) -> bool {
        if self.is_flow_axis(dir) {
            // Along the flow: producer towards the origin, consumer away from it.
            let upstream = matches!(dir, Dir::Up | Dir::Left);
            self.edge_move(upstream, dir)
        } else {
            self.sibling_move(dir)
        }
    }

    /// Whether `dir` runs along the flow axis (producer/consumer) rather than
    /// across it (siblings), given the current [`Flow`].
    fn is_flow_axis(&self, dir: Dir) -> bool {
        match self.flow {
            Flow::Vertical => matches!(dir, Dir::Up | Dir::Down),
            Flow::Horizontal => matches!(dir, Dir::Left | Dir::Right),
        }
    }

    /// Producer/consumer step: among the cursor's graph neighbours (`preds` when
    /// `upstream`, else `succs`), pick the one whose rendered centre is most in
    /// line with the cursor across the flow — the neighbour that reads as
    /// directly ahead. Falls back to id order when geometry is unset (headless
    /// callers that never call [`set_geometry`](ProtocolNav::set_geometry)).
    fn edge_move(&mut self, upstream: bool, dir: Dir) -> bool {
        let Some(cur) = self.cursor.clone() else {
            return false;
        };
        let table = if upstream { &self.preds } else { &self.succs };
        let neighbours = table.get(&cur).cloned().unwrap_or_default();
        if neighbours.is_empty() {
            return false;
        }
        let pick = self
            .center(&cur)
            .and_then(|origin| {
                neighbours
                    .iter()
                    .filter_map(|id| self.center(id).map(|c| (id, c)))
                    .min_by(|(_, a), (_, b)| {
                        axis_key(origin, *a, dir, true)
                            .partial_cmp(&axis_key(origin, *b, dir, true))
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(id, _)| id.clone())
            })
            .unwrap_or_else(|| neighbours[0].clone());
        self.cursor = Some(pick);
        true
    }

    /// Sibling step: among the cursor's rank siblings (same layer), pick the one
    /// nearest in the pressed pixel direction. A wall (returns false) when no
    /// sibling lies that way — so the edge of a layer is felt and id order never
    /// overrides the drawn position. Needs geometry; without an on-screen
    /// direction to honour it is a no-op.
    fn sibling_move(&mut self, dir: Dir) -> bool {
        let Some(cur) = self.cursor.clone() else {
            return false;
        };
        let Some(&l) = self.layer.get(&cur) else {
            return false;
        };
        let Some(origin) = self.center(&cur) else {
            return false;
        };
        let pick = self.ranks[l]
            .iter()
            .filter(|id| **id != cur)
            .filter_map(|id| self.center(id).map(|c| (id, c)))
            .filter(|(_, c)| in_direction(origin, *c, dir))
            .min_by(|(_, a), (_, b)| {
                axis_key(origin, *a, dir, false)
                    .partial_cmp(&axis_key(origin, *b, dir, false))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(id, _)| id.clone());
        if let Some(t) = pick {
            self.cursor = Some(t);
            true
        } else {
            false
        }
    }

    /// The rendered centre of `id`, if geometry has been set.
    fn center(&self, id: &AssetId) -> Option<(f64, f64)> {
        self.centers.get(id).copied()
    }

    /// `za` — fold/unfold the parameterised family under the cursor. A no-op
    /// (returns [`FoldOutcome::NotAFamily`]) when the cursor is not on a Family
    /// tile — folds only ever act on collapsible nodes.
    pub fn toggle_fold(&mut self) -> FoldOutcome {
        let Some(cur) = self.cursor.clone() else {
            return FoldOutcome::NotAFamily;
        };
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
    /// breadcrumb tracks the drill PATH with **no consecutive duplicates** —
    /// drilling the already-current node (a repeated `Enter`) is a no-op, so the
    /// crumb never stacks `fetch › fetch › …`. Returns whether a node was pushed.
    pub fn drill_in(&mut self) -> bool {
        let Some(cur) = self.cursor.clone() else {
            return false;
        };
        if self.breadcrumb.last() == Some(&cur) {
            return false;
        }
        self.breadcrumb.push(cur);
        true
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
    let mut layer: BTreeMap<AssetId, usize> =
        graph.nodes.keys().map(|id| (id.clone(), 0)).collect();
    let mut indegree: BTreeMap<AssetId, usize> = graph
        .nodes
        .keys()
        .map(|id| (id.clone(), preds.get(id).map_or(0, Vec::len)))
        .collect();
    let mut ready: BTreeSet<AssetId> = indegree
        .iter()
        .filter(|&(_id, d)| *d == 0)
        .map(|(id, _d)| id.clone())
        .collect();
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

/// Is `c` strictly in the pressed direction from `origin`? The gate that makes
/// a sibling step honour the drawn left/right/up/down instead of id order.
fn in_direction(origin: (f64, f64), c: (f64, f64), dir: Dir) -> bool {
    const EPS: f64 = 0.5;
    match dir {
        Dir::Up => c.1 < origin.1 - EPS,
        Dir::Down => c.1 > origin.1 + EPS,
        Dir::Left => c.0 < origin.0 - EPS,
        Dir::Right => c.0 > origin.0 + EPS,
    }
}

/// The sort key for "nearest in the pressed direction", as `(primary, secondary)`
/// distances. For an **edge** move (`align_first`) the cross-axis offset leads,
/// so the neighbour most directly ahead wins; for a **sibling** move the
/// along-axis distance leads, so the immediate neighbour wins. Each breaks ties
/// on the other component — a total, deterministic order over the candidates.
fn axis_key(origin: (f64, f64), c: (f64, f64), dir: Dir, align_first: bool) -> (f64, f64) {
    let dx = (c.0 - origin.0).abs();
    let dy = (c.1 - origin.1).abs();
    // (distance along the pressed axis, distance across it).
    let (along, cross) = match dir {
        Dir::Up | Dir::Down => (dy, dx),
        Dir::Left | Dir::Right => (dx, dy),
    };
    if align_first {
        (cross, along)
    } else {
        (along, cross)
    }
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
    fn hl_walks_producer_and_consumer_topologically() {
        let g = chain_graph();
        let mut nav = ProtocolNav::new(&g);
        // The entry cursor is a root (the source, no producer).
        let start = nav.cursor().cloned().unwrap();
        assert!(
            nav.preds[&start].is_empty(),
            "entry cursor is a root: {start}"
        );
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
    fn hl_are_walls_at_the_ends() {
        let g = chain_graph();
        let mut nav = ProtocolNav::new(&g);
        // At a root, `h` cannot move.
        assert!(!nav.to_producer(), "no producer past a root");
        // At the sink, `l` cannot move.
        while nav.to_consumer() {}
        assert!(!nav.to_consumer(), "no consumer past the sink");
    }

    #[test]
    fn jk_step_rank_siblings_without_wrapping() {
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
    with: { archive: build/a, dest: build/l, members: [p.tsv] }
  - name: right
    op: archive_extract@1
    with: { archive: build/a, dest: build/r, members: [p.tsv] }
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
    fn za_folds_only_a_family_tile() {
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
    with: { archive: build/z/q1.zip, dest: build/x/q1, members: [p.tsv] }
  - name: fetch_x_q2
    op: http_fetch@1
    with: { url: 'https://h.example/q2.zip', out: build/z/q2.zip }
  - name: extract_x_q2
    op: archive_extract@1
    with: { archive: build/z/q2.zip, dest: build/x/q2, members: [p.tsv] }
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
    fn enter_esc_push_and_pop_the_drill_stack() {
        let g = chain_graph();
        let mut nav = ProtocolNav::new(&g);
        assert!(nav.breadcrumb().is_empty());
        let root = nav.cursor().cloned().unwrap();
        assert!(nav.drill_in());
        assert_eq!(nav.breadcrumb(), std::slice::from_ref(&root));
        // Move down, drill again — the breadcrumb tracks the path.
        nav.to_consumer();
        let child = nav.cursor().cloned().unwrap();
        assert!(nav.drill_in());
        assert_eq!(nav.breadcrumb(), &[root.clone(), child.clone()]);
        // Esc pops one level and returns the cursor to the parent.
        assert!(nav.drill_out());
        assert_eq!(nav.breadcrumb(), std::slice::from_ref(&root));
        assert_eq!(nav.cursor().unwrap(), &root);
        // Esc to empty, then Esc on an empty stack is a wall.
        assert!(nav.drill_out());
        assert!(nav.breadcrumb().is_empty());
        assert!(!nav.drill_out());
    }

    #[test]
    fn nav_construction_is_deterministic() {
        let g = chain_graph();
        let a = ProtocolNav::new(&g);
        let b = ProtocolNav::new(&g);
        assert_eq!(a.cursor(), b.cursor());
        assert_eq!(a.ranks, b.ranks);
    }

    /// A diamond with two rank siblings AND a producer + consumer for each — the
    /// fixture the spatial-mapping tests need to exercise every axis.
    fn diamond_graph() -> AssetGraph {
        let yaml = r"
name: diamond
steps:
  - name: fetch
    op: http_fetch@1
    with: { url: 'https://h.example/a', out: build/a }
  - name: left
    op: archive_extract@1
    with: { archive: build/a, dest: build/l, members: [p.tsv] }
  - name: right
    op: archive_extract@1
    with: { archive: build/a, dest: build/r, members: [p.tsv] }
  - name: join
    sql: models/j.sql
    depends_on: [build/l, build/r]
";
        let manifest = parse_manifest_str(yaml).unwrap();
        let mut sources = BTreeMap::new();
        sources.insert(
            "join".to_string(),
            Ok("CREATE TABLE j AS SELECT * FROM read_csv('build/l/x.csv') \
                UNION ALL SELECT * FROM read_csv('build/r/x.csv');"
                .to_string()),
        );
        build_graph(&manifest, &sources)
    }

    fn center(l: &crate::layout::Layout, id: &str) -> (f64, f64) {
        let r = &l.positions[&id.to_string()];
        (r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    /// Vertical flow: `k`/`j` (up/down) are the producer/consumer axis and
    /// `h`/`l` (left/right) step between rank siblings — the mapping matches the
    /// drawn top→bottom flow.
    #[test]
    fn vertical_flow_maps_updown_to_flow_leftright_to_siblings() {
        use crate::layout::{layout, Flow, LayoutConfig};
        let g = diamond_graph();
        let l = layout(
            &g,
            &LayoutConfig {
                flow: Flow::Vertical,
                ..LayoutConfig::default()
            },
        );
        let (left, right) = (
            "file.diamond.build/l".to_string(),
            "file.diamond.build/r".to_string(),
        );
        let base_layer = {
            let n = ProtocolNav::new(&g);
            assert_eq!(
                n.layer[&left], n.layer[&right],
                "the two extracts are rank siblings"
            );
            n.layer[&left]
        };

        // Up = producer (a shallower layer), Down = consumer (a deeper layer).
        let mut nav = ProtocolNav::new(&g);
        nav.set_geometry(Flow::Vertical, &l);
        nav.focus(&left);
        assert!(nav.move_dir(Dir::Up), "up reaches the producer");
        assert!(
            nav.layer[nav.cursor().unwrap()] < base_layer,
            "up climbed upstream"
        );
        nav.focus(&left);
        assert!(nav.move_dir(Dir::Down), "down reaches the consumer");
        assert!(
            nav.layer[nav.cursor().unwrap()] > base_layer,
            "down descended downstream"
        );

        // Left/Right = the rank sibling (same layer), picked by rendered side.
        let (cl, cr) = (center(&l, &left), center(&l, &right));
        let to_r = if cr.0 > cl.0 { Dir::Right } else { Dir::Left };
        nav.focus(&left);
        assert!(
            nav.move_dir(to_r),
            "the horizontal key steps to the sibling"
        );
        assert_eq!(nav.cursor().unwrap(), &right);
        assert_eq!(
            nav.layer[nav.cursor().unwrap()],
            base_layer,
            "sibling stays in the layer"
        );
    }

    /// Horizontal flow rotates the axes: `h`/`l` (left/right) become the
    /// producer/consumer axis and `k`/`j` (up/down) step between siblings — the
    /// SAME graph, the keys following the drawn left→right flow.
    #[test]
    fn horizontal_flow_maps_leftright_to_flow_updown_to_siblings() {
        use crate::layout::{layout, Flow, LayoutConfig};
        let g = diamond_graph();
        let l = layout(
            &g,
            &LayoutConfig {
                flow: Flow::Horizontal,
                ..LayoutConfig::default()
            },
        );
        let (left, right) = (
            "file.diamond.build/l".to_string(),
            "file.diamond.build/r".to_string(),
        );
        let base_layer = ProtocolNav::new(&g).layer[&left];

        // Left = producer, Right = consumer.
        let mut nav = ProtocolNav::new(&g);
        nav.set_geometry(Flow::Horizontal, &l);
        nav.focus(&left);
        assert!(nav.move_dir(Dir::Left), "left reaches the producer");
        assert!(nav.layer[nav.cursor().unwrap()] < base_layer);
        nav.focus(&left);
        assert!(nav.move_dir(Dir::Right), "right reaches the consumer");
        assert!(nav.layer[nav.cursor().unwrap()] > base_layer);

        // Up/Down = the rank sibling, picked by rendered side.
        let (cl, cr) = (center(&l, &left), center(&l, &right));
        let to_r = if cr.1 > cl.1 { Dir::Down } else { Dir::Up };
        nav.focus(&left);
        assert!(nav.move_dir(to_r), "the vertical key steps to the sibling");
        assert_eq!(nav.cursor().unwrap(), &right);
        assert_eq!(nav.layer[nav.cursor().unwrap()], base_layer);
    }

    /// A repeated `Enter` on the current node does not stack a duplicate crumb.
    #[test]
    fn drill_in_dedups_the_current_node() {
        let g = chain_graph();
        let mut nav = ProtocolNav::new(&g);
        assert!(nav.drill_in(), "first drill pushes the crumb");
        assert_eq!(nav.breadcrumb().len(), 1);
        assert!(!nav.drill_in(), "a repeat on the same node is a no-op");
        assert_eq!(nav.breadcrumb().len(), 1, "no consecutive-duplicate crumb");
        // Moving to a new node, then drilling, grows the path.
        nav.to_consumer();
        assert!(nav.drill_in());
        assert_eq!(nav.breadcrumb().len(), 2);
    }
}
