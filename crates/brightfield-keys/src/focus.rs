//! The focus state machine over the ComponentPath tree and the
//! path↔plot(+rect) resolver — plus focus-jump path fuzzy-matching.
//!
//! Built framework-free from the spec's layout tree: the focus tree carries the SAME
//! plot-node path scheme as `brightfield_spec::layout::placed_plots` (and thus
//! `collect_plot_groups`), so a focused plot path joins to the runtime
//! coordinator's `LivePlot.path` and to the engine's selection keys.

use crate::altitude::Altitude;
use crate::fuzzy::fuzzy_score;
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::layout::{compute_layout, LayoutNode, Rect};
use brightfield_spec::Spec;
use std::collections::HashMap;

/// The kind of a focus node. v1 has exactly two, mapping to the two altitudes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// The root or an intermediate concat container (Dashboard altitude).
    Container,
    /// A plot leaf — the focus floor (View altitude).
    Plot,
}

impl NodeKind {
    /// The altitude this node kind sits at.
    #[must_use]
    pub fn altitude(self) -> Altitude {
        match self {
            NodeKind::Container => Altitude::Dashboard,
            NodeKind::Plot => Altitude::View,
        }
    }
}

/// A node in the focus tree — an arena entry.
#[derive(Debug, Clone, PartialEq)]
pub struct FocusNode {
    /// The plot-node ComponentPath (e.g. `root`, `root/hconcat[0]`).
    pub path: ComponentPath,
    /// Container or Plot.
    pub kind: NodeKind,
    /// The node's full box in dashboard pixels (the focus-ring geometry).
    pub rect: Rect,
    /// Arena index of the parent, `None` for the root.
    pub parent: Option<usize>,
    /// Arena indices of the navigable children (plots/containers; spacers etc. skipped).
    pub children: Vec<usize>,
}

impl FocusNode {
    /// The altitude of this node (Container → Dashboard, Plot → View).
    #[must_use]
    pub fn altitude(&self) -> Altitude {
        self.kind.altitude()
    }
}

/// A framework-free tree of the dashboard's navigable structure, seeded at assembly.
/// Arena-based (indices, no `Rc`) so it stays trivially `Clone` and headless.
#[derive(Debug, Clone, PartialEq)]
pub struct FocusTree {
    nodes: Vec<FocusNode>,
    index: HashMap<ComponentPath, usize>,
    root: Option<usize>,
}

impl FocusTree {
    /// An empty focus tree (no navigable structure). `FocusState::new` returns
    /// `None`, so every nav op is a defined no-op — for a spec with no plots, or
    /// a shim/test that needs a tree value.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            index: HashMap::new(),
            root: None,
        }
    }

    /// Build the focus tree from a spec (uses the same layout walk + path scheme
    /// as `placed_plots`, so plot paths join to the runtime coordinator).
    #[must_use]
    pub fn from_spec(spec: &Spec) -> Self {
        let mut nodes = Vec::new();
        let mut index = HashMap::new();
        let root = compute_layout(spec, Rect::zero())
            .and_then(|tree| walk(&tree, "root".to_string(), None, &mut nodes, &mut index));
        Self { nodes, index, root }
    }

    /// The root node's arena index, if the spec has any navigable structure.
    #[must_use]
    pub fn root(&self) -> Option<usize> {
        self.root
    }

    /// All nodes (arena order — root first).
    #[must_use]
    pub fn nodes(&self) -> &[FocusNode] {
        &self.nodes
    }

    /// The node at `path`, if present.
    #[must_use]
    pub fn node_at(&self, path: &ComponentPath) -> Option<&FocusNode> {
        self.index.get(path).map(|&i| &self.nodes[i])
    }

    /// The focus-ring rect for `path` (the node's full box), if present.
    #[must_use]
    pub fn rect_of(&self, path: &ComponentPath) -> Option<Rect> {
        self.node_at(path).map(|n| n.rect)
    }

    /// Resolve a focused path to its plot identity (`Some` only when the path is
    /// a plot leaf), for verb-targeting a view. A container focus resolves to
    /// `None` (a dashboard-scoped verb targets the container/root directly).
    #[must_use]
    pub fn plot_at(&self, path: &ComponentPath) -> Option<&ComponentPath> {
        self.node_at(path)
            .filter(|n| n.kind == NodeKind::Plot)
            .map(|n| &n.path)
    }

    /// Whether `path` names a plot leaf.
    #[must_use]
    pub fn is_plot(&self, path: &ComponentPath) -> bool {
        self.node_at(path).is_some_and(|n| n.kind == NodeKind::Plot)
    }

    fn node(&self, i: usize) -> &FocusNode {
        &self.nodes[i]
    }
}

/// Recursively walk the layout tree, minting the plot-node path scheme (identical
/// to `collect_placed_plots`) and building the arena. Returns the arena index of
/// the node just added, or `None` if the node is not navigable (spacer, legend,
/// standalone input/mark/interactor).
fn walk(
    node: &LayoutNode,
    path: String,
    parent: Option<usize>,
    nodes: &mut Vec<FocusNode>,
    index: &mut HashMap<ComponentPath, usize>,
) -> Option<usize> {
    let (kind, rect, container_children, prefix) = match node {
        // A plot is a LEAF in the focus tree — the view floor (no mark descent).
        LayoutNode::Plot { rect, .. } => (NodeKind::Plot, *rect, None, ""),
        LayoutNode::HConcat { rect, children } => {
            (NodeKind::Container, *rect, Some(children), "hconcat")
        }
        LayoutNode::VConcat { rect, children } => {
            (NodeKind::Container, *rect, Some(children), "vconcat")
        }
        // Non-plot leaves are not focus nodes (mirrors collect_placed_plots).
        _ => return None,
    };

    let my_index = nodes.len();
    let path_key = ComponentPath(path.clone());
    nodes.push(FocusNode {
        path: path_key.clone(),
        kind,
        rect,
        parent,
        children: Vec::new(),
    });
    index.insert(path_key, my_index);

    if let Some(children) = container_children {
        // Enumerate ALL children so the index matches placed_plots' scheme
        // (spacers advance the index even though they are not focus nodes).
        let mut child_indices = Vec::new();
        for (i, child) in children.iter().enumerate() {
            let child_path = format!("{path}/{prefix}[{i}]");
            if let Some(ci) = walk(child, child_path, Some(my_index), nodes, index) {
                child_indices.push(ci);
            }
        }
        nodes[my_index].children = child_indices;
    }

    Some(my_index)
}

// ---------------------------------------------------------------------------
// Focus state machine
// ---------------------------------------------------------------------------

/// Where focus currently sits. Transitions are dive / pop / sibling, floored at
/// the view; pop never rises above root; dive is a no-op at a plot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusState {
    current: usize,
}

impl FocusState {
    /// Start focus at the root, if the tree has any navigable node.
    #[must_use]
    pub fn new(tree: &FocusTree) -> Option<Self> {
        tree.root.map(|current| Self { current })
    }

    /// The focused node's arena index.
    #[must_use]
    pub fn current(&self) -> usize {
        self.current
    }

    /// The focused node.
    #[must_use]
    pub fn node<'a>(&self, tree: &'a FocusTree) -> &'a FocusNode {
        tree.node(self.current)
    }

    /// The focused ComponentPath.
    #[must_use]
    pub fn path<'a>(&self, tree: &'a FocusTree) -> &'a ComponentPath {
        &self.node(tree).path
    }

    /// The focused altitude.
    #[must_use]
    pub fn altitude(&self, tree: &FocusTree) -> Altitude {
        self.node(tree).altitude()
    }

    /// Dive into the first child. No-op at a plot (the floor) or a childless node.
    /// Returns `true` if focus moved.
    pub fn dive(&mut self, tree: &FocusTree) -> bool {
        if let Some(&first) = tree.node(self.current).children.first() {
            self.current = first;
            true
        } else {
            false
        }
    }

    /// Pop out to the parent. No-op at the root. Returns `true` if focus moved.
    pub fn pop(&mut self, tree: &FocusTree) -> bool {
        if let Some(parent) = tree.node(self.current).parent {
            self.current = parent;
            true
        } else {
            false
        }
    }

    /// Move to the next sibling (clamped at the last). Returns `true` if focus moved.
    pub fn next_sibling(&mut self, tree: &FocusTree) -> bool {
        self.sibling(tree, 1)
    }

    /// Move to the previous sibling (clamped at the first). Returns `true` if focus moved.
    pub fn prev_sibling(&mut self, tree: &FocusTree) -> bool {
        self.sibling(tree, -1)
    }

    fn sibling(&mut self, tree: &FocusTree, delta: isize) -> bool {
        let Some(parent) = tree.node(self.current).parent else {
            return false; // root has no siblings
        };
        let siblings = &tree.node(parent).children;
        let Some(pos) = siblings.iter().position(|&i| i == self.current) else {
            return false;
        };
        let target = pos as isize + delta;
        if target < 0 || target as usize >= siblings.len() {
            return false; // clamp at the ends
        }
        self.current = siblings[target as usize];
        true
    }

    /// Jump focus directly to a path (focus-jump / restore). Returns `true` if the
    /// path exists in the tree.
    pub fn jump_to(&mut self, tree: &FocusTree, path: &ComponentPath) -> bool {
        if let Some(&i) = tree.index.get(path) {
            self.current = i;
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Focus-jump path fuzzy-matching
// ---------------------------------------------------------------------------

/// A focus-jump candidate: a focusable path and its fuzzy score for `query`.
#[derive(Debug, Clone, PartialEq)]
pub struct JumpCandidate {
    /// The path to jump focus to.
    pub path: ComponentPath,
    /// Whether this node is a plot (for a leaf/branch affordance in the overlay).
    pub is_plot: bool,
    /// The fuzzy score (higher = better).
    pub score: i32,
}

/// Rank the tree's nodes against `query` for focus-jump (`/`). Reuses the palette
/// fuzzy matcher over each node's path label; returns only focusable paths (every
/// node in the focus tree is at/above the view floor by construction), best first.
#[must_use]
pub fn focus_jump_candidates(tree: &FocusTree, query: &str) -> Vec<JumpCandidate> {
    let mut out: Vec<JumpCandidate> = tree
        .nodes()
        .iter()
        .filter_map(|n| {
            fuzzy_score(query, &n.path.0).map(|score| JumpCandidate {
                path: n.path.clone(),
                is_plot: n.kind == NodeKind::Plot,
                score,
            })
        })
        .collect();
    // Best score first; ties keep arena (dashboard) order for stability.
    out.sort_by_key(|c| std::cmp::Reverse(c.score));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::parse::parse_spec;
    use brightfield_spec::Format;

    /// A vconcat of one plot over a nested hconcat of two plots (three plots,
    /// two altitudes, a nested container).
    fn multi_view_spec() -> Spec {
        let src = r#"
meta:
  title: multi-view fixture
data:
  t:
    - { a: 1, b: 2 }
    - { a: 3, b: 4 }
vconcat:
  - plot:
      - mark: dot
        data: { from: t }
        x: a
        y: b
  - hconcat:
      - plot:
          - mark: dot
            data: { from: t }
            x: a
            y: b
      - plot:
          - mark: dot
            data: { from: t }
            x: a
            y: b
"#;
        parse_spec(src, Format::Yaml).expect("fixture parses").spec
    }

    fn single_plot_spec() -> Spec {
        let src = r#"
meta:
  title: single-plot fixture
data:
  t:
    - { a: 1, b: 2 }
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
"#;
        parse_spec(src, Format::Yaml).expect("fixture parses").spec
    }

    #[test]
    fn dive_pop_sibling_land_on_expected_paths() {
        let spec = multi_view_spec();
        let tree = FocusTree::from_spec(&spec);
        let mut st = FocusState::new(&tree).expect("has root");

        // Root is the vconcat container (Dashboard altitude).
        assert_eq!(st.path(&tree).0, "root");
        assert_eq!(st.altitude(&tree), Altitude::Dashboard);

        // Dive → first child = the first plot (View).
        assert!(st.dive(&tree));
        assert_eq!(st.path(&tree).0, "root/vconcat[0]");
        assert_eq!(st.altitude(&tree), Altitude::View);

        // Next sibling → the hconcat container.
        assert!(st.next_sibling(&tree));
        assert_eq!(st.path(&tree).0, "root/vconcat[1]");
        assert_eq!(st.altitude(&tree), Altitude::Dashboard);

        // Dive into the hconcat → its first plot.
        assert!(st.dive(&tree));
        assert_eq!(st.path(&tree).0, "root/vconcat[1]/hconcat[0]");
        assert!(st.next_sibling(&tree));
        assert_eq!(st.path(&tree).0, "root/vconcat[1]/hconcat[1]");

        // Pop twice → back to root.
        assert!(st.pop(&tree));
        assert_eq!(st.path(&tree).0, "root/vconcat[1]");
        assert!(st.pop(&tree));
        assert_eq!(st.path(&tree).0, "root");
    }

    #[test]
    fn dive_at_plot_and_pop_at_root_are_defined_noops() {
        let spec = multi_view_spec();
        let tree = FocusTree::from_spec(&spec);
        let mut st = FocusState::new(&tree).unwrap();

        // Pop at root → no-op.
        assert!(!st.pop(&tree));
        assert_eq!(st.path(&tree).0, "root");

        // Dive to a plot, then dive again → floored no-op.
        assert!(st.dive(&tree));
        assert_eq!(st.altitude(&tree), Altitude::View);
        assert!(
            !st.dive(&tree),
            "dive at a plot must be a no-op (view floor)"
        );
        assert_eq!(st.path(&tree).0, "root/vconcat[0]");
    }

    #[test]
    fn sibling_clamps_at_ends() {
        let spec = multi_view_spec();
        let tree = FocusTree::from_spec(&spec);
        let mut st = FocusState::new(&tree).unwrap();
        st.dive(&tree); // vconcat[0]
        assert!(!st.prev_sibling(&tree), "prev at first sibling is a no-op");
        assert!(st.next_sibling(&tree)); // vconcat[1]
        assert!(!st.next_sibling(&tree), "next at last sibling is a no-op");
    }

    #[test]
    fn resolver_returns_plot_and_rect_per_path() {
        let spec = multi_view_spec();
        let tree = FocusTree::from_spec(&spec);

        // A plot path resolves to itself + a non-zero rect.
        let plot = ComponentPath("root/vconcat[0]".into());
        assert!(tree.is_plot(&plot));
        assert_eq!(tree.plot_at(&plot), Some(&plot));
        let rect = tree.rect_of(&plot).expect("plot has a rect");
        assert!(rect.width > 0.0 && rect.height > 0.0);

        // A container path resolves to no plot (dashboard-scoped) but has a rect.
        let container = ComponentPath("root".into());
        assert!(!tree.is_plot(&container));
        assert_eq!(tree.plot_at(&container), None);
        assert!(tree.rect_of(&container).is_some());

        // The tree's plot paths match the layout's placed_plots paths exactly.
        let placed: Vec<String> = brightfield_spec::layout::placed_plots(&spec, Rect::zero())
            .into_iter()
            .map(|p| p.path)
            .collect();
        let tree_plots: Vec<String> = tree
            .nodes()
            .iter()
            .filter(|n| n.kind == NodeKind::Plot)
            .map(|n| n.path.0.clone())
            .collect();
        assert_eq!(tree_plots, placed);
    }

    #[test]
    fn single_plot_degenerate_leaves_nav_defined() {
        let spec = single_plot_spec();
        let tree = FocusTree::from_spec(&spec);
        let mut st = FocusState::new(&tree).expect("root plot");
        // Root IS the plot (View altitude); every nav op is a defined no-op.
        assert_eq!(st.path(&tree).0, "root");
        assert_eq!(st.altitude(&tree), Altitude::View);
        assert!(!st.dive(&tree));
        assert!(!st.pop(&tree));
        assert!(!st.next_sibling(&tree));
        assert!(!st.prev_sibling(&tree));
    }

    #[test]
    fn focus_jump_ranks_nodes_and_returns_focusable_paths() {
        let spec = multi_view_spec();
        let tree = FocusTree::from_spec(&spec);

        // Empty query returns every node (all focusable, at/above the view floor).
        let all = focus_jump_candidates(&tree, "");
        assert_eq!(all.len(), tree.nodes().len());

        // A query ranks matching paths; "hconcat" surfaces the hconcat subtree.
        let hits = focus_jump_candidates(&tree, "hconcat");
        assert!(hits.iter().all(|c| c.score >= 0));
        assert!(hits.first().is_some_and(|c| c.path.0.contains("hconcat")));

        // jump_to moves focus to a returned path.
        let mut st = FocusState::new(&tree).unwrap();
        let target = ComponentPath("root/vconcat[1]/hconcat[1]".into());
        assert!(st.jump_to(&tree, &target));
        assert_eq!(st.path(&tree), &target);
        // A path not in the tree does not move focus.
        assert!(!st.jump_to(&tree, &ComponentPath("root/nope[9]".into())));
        assert_eq!(st.path(&tree), &target);
    }
}
