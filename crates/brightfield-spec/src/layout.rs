//! Layout computation for Mosaic spec composition trees.
//!
//! Pure function of the AST — walks [`Component`] trees and produces positioned
//! [`LayoutNode`] trees with pixel-accurate coordinates using a simple box
//! model: sequential stacking with fixed sizes, no flex negotiation.

use crate::ast::{Component, ConcatNode, Input, PlotNode, Spec, SpaceNode, SpecValue};

// ---------------------------------------------------------------------------
// Rect
// ---------------------------------------------------------------------------

/// An axis-aligned rectangle with position and size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// Construct a new Rect.
    #[must_use]
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    /// A zero-sized rect at the origin.
    #[must_use]
    pub fn zero() -> Self {
        Self { x: 0.0, y: 0.0, width: 0.0, height: 0.0 }
    }
}

// ---------------------------------------------------------------------------
// Default sizes
// ---------------------------------------------------------------------------

/// Default plot width (pixels).
pub const DEFAULT_PLOT_WIDTH: f64 = 640.0;
/// Default plot height (pixels).
pub const DEFAULT_PLOT_HEIGHT: f64 = 400.0;
/// Default input widget width (pixels).
pub const DEFAULT_INPUT_WIDTH: f64 = 200.0;
/// Default input widget height (pixels).
pub const DEFAULT_INPUT_HEIGHT: f64 = 32.0;
/// Default legend width (pixels).
pub const DEFAULT_LEGEND_WIDTH: f64 = 120.0;
/// Default legend height (pixels).
pub const DEFAULT_LEGEND_HEIGHT: f64 = 24.0;
/// Default base font size for `em` unit conversion (pixels).
pub const DEFAULT_BASE_FONT_SIZE: f64 = 16.0;

// ---------------------------------------------------------------------------
// LayoutNode
// ---------------------------------------------------------------------------

/// A positioned node in the layout tree. Mirrors [`Component`] variants, each
/// carrying a [`Rect`] and children where applicable.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutNode {
    /// A plot with its computed position and size.
    Plot {
        rect: Rect,
        children: Vec<LayoutNode>,
    },
    /// Horizontal concatenation container.
    HConcat {
        rect: Rect,
        children: Vec<LayoutNode>,
    },
    /// Vertical concatenation container.
    VConcat {
        rect: Rect,
        children: Vec<LayoutNode>,
    },
    /// Horizontal spacer.
    HSpace { rect: Rect },
    /// Vertical spacer.
    VSpace { rect: Rect },
    /// A standalone legend.
    Legend { rect: Rect },
    /// A standalone input widget.
    Input { rect: Rect },
    /// A bare mark at the composition level.
    Mark { rect: Rect },
    /// A bare interactor at the composition level.
    Interactor { rect: Rect },
}

impl LayoutNode {
    /// Get the rect for any layout node variant.
    #[must_use]
    pub fn rect(&self) -> &Rect {
        match self {
            LayoutNode::Plot { rect, .. }
            | LayoutNode::HConcat { rect, .. }
            | LayoutNode::VConcat { rect, .. }
            | LayoutNode::HSpace { rect }
            | LayoutNode::VSpace { rect }
            | LayoutNode::Legend { rect }
            | LayoutNode::Input { rect }
            | LayoutNode::Mark { rect }
            | LayoutNode::Interactor { rect } => rect,
        }
    }
}

/// The result of layout computation — an optional root node (None if the spec
/// has no visible root component).
pub type LayoutTree = Option<LayoutNode>;

// ---------------------------------------------------------------------------
// Space value resolution
// ---------------------------------------------------------------------------

/// Resolve a spacer value to pixels.
///
/// - Integer and float values are treated as pixel values directly.
/// - String values ending in `em` are multiplied by `base_font_size`.
/// - Other string values and non-numeric types return 0.0.
#[must_use]
pub fn resolve_space_value(value: &SpecValue, base_font_size: f64) -> f64 {
    match value {
        SpecValue::Integer(n) => *n as f64,
        SpecValue::Float(f) => *f,
        SpecValue::String(s) => {
            let trimmed = s.trim();
            if let Some(num_str) = trimmed.strip_suffix("em") {
                num_str.trim().parse::<f64>().unwrap_or(0.0) * base_font_size
            } else {
                // Try parsing as a bare number string.
                trimmed.parse::<f64>().unwrap_or(0.0)
            }
        }
        _ => 0.0,
    }
}

// ---------------------------------------------------------------------------
// compute_layout
// ---------------------------------------------------------------------------

/// Compute the layout tree for a spec.
///
/// Walks `spec.root` and produces a [`LayoutTree`] with positioned nodes.
/// If the spec has no root component, returns `None`.
///
/// The `viewport` rect determines the origin for the root node's position
/// (typically `(0, 0)` with the desired container size).
#[must_use]
pub fn compute_layout(spec: &Spec, viewport: Rect) -> LayoutTree {
    spec.root.as_ref().map(|root| {
        layout_component(root, viewport.x, viewport.y)
    })
}

/// Recursively lay out a component, placing it at the given (x, y) origin.
/// Returns a LayoutNode with computed position and intrinsic size.
fn layout_component(component: &Component, x: f64, y: f64) -> LayoutNode {
    match component {
        Component::Plot(plot) => layout_plot(plot, x, y),
        Component::HConcat(concat) => layout_hconcat(concat, x, y),
        Component::VConcat(concat) => layout_vconcat(concat, x, y),
        Component::HSpace(space) => layout_hspace(space, x, y),
        Component::VSpace(space) => layout_vspace(space, x, y),
        Component::Legend(_) => LayoutNode::Legend {
            rect: Rect::new(x, y, DEFAULT_LEGEND_WIDTH, DEFAULT_LEGEND_HEIGHT),
        },
        Component::Input(_) => LayoutNode::Input {
            rect: Rect::new(x, y, DEFAULT_INPUT_WIDTH, DEFAULT_INPUT_HEIGHT),
        },
        Component::Mark(_) => LayoutNode::Mark {
            rect: Rect::new(x, y, DEFAULT_PLOT_WIDTH, DEFAULT_PLOT_HEIGHT),
        },
        Component::Interactor(_) => LayoutNode::Interactor {
            rect: Rect::new(x, y, 0.0, 0.0),
        },
    }
}

/// Extract a plot's width from its attributes, falling back to the default.
fn plot_width(plot: &PlotNode) -> f64 {
    plot.attributes
        .get("width")
        .and_then(|v| match v {
            SpecValue::Integer(n) => Some(*n as f64),
            SpecValue::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap_or(DEFAULT_PLOT_WIDTH)
}

/// Extract a plot's height from its attributes, falling back to the default.
fn plot_height(plot: &PlotNode) -> f64 {
    plot.attributes
        .get("height")
        .and_then(|v| match v {
            SpecValue::Integer(n) => Some(*n as f64),
            SpecValue::Float(f) => Some(*f),
            _ => None,
        })
        .unwrap_or(DEFAULT_PLOT_HEIGHT)
}

fn layout_plot(plot: &PlotNode, x: f64, y: f64) -> LayoutNode {
    let w = plot_width(plot);
    let h = plot_height(plot);
    // Plot items (marks, interactors, legends) are positioned within the plot
    // but for layout purposes they share the plot's footprint.
    let children: Vec<LayoutNode> = plot
        .items
        .iter()
        .map(|item| layout_component(item, x, y))
        .collect();
    LayoutNode::Plot {
        rect: Rect::new(x, y, w, h),
        children,
    }
}

fn layout_hconcat(concat: &ConcatNode, x: f64, y: f64) -> LayoutNode {
    let mut children = Vec::with_capacity(concat.items.len());
    let mut cursor_x = x;
    let mut max_height: f64 = 0.0;

    for item in &concat.items {
        let child = layout_component(item, cursor_x, y);
        let r = child.rect();
        cursor_x += r.width;
        max_height = max_height.max(r.height);
        children.push(child);
    }

    LayoutNode::HConcat {
        rect: Rect::new(x, y, cursor_x - x, max_height),
        children,
    }
}

fn layout_vconcat(concat: &ConcatNode, x: f64, y: f64) -> LayoutNode {
    let mut children = Vec::with_capacity(concat.items.len());
    let mut cursor_y = y;
    let mut max_width: f64 = 0.0;

    for item in &concat.items {
        let child = layout_component(item, x, cursor_y);
        let r = child.rect();
        cursor_y += r.height;
        max_width = max_width.max(r.width);
        children.push(child);
    }

    LayoutNode::VConcat {
        rect: Rect::new(x, y, max_width, cursor_y - y),
        children,
    }
}

fn layout_hspace(space: &SpaceNode, x: f64, y: f64) -> LayoutNode {
    let w = resolve_space_value(&space.value, DEFAULT_BASE_FONT_SIZE);
    LayoutNode::HSpace {
        rect: Rect::new(x, y, w, 0.0),
    }
}

fn layout_vspace(space: &SpaceNode, x: f64, y: f64) -> LayoutNode {
    let h = resolve_space_value(&space.value, DEFAULT_BASE_FONT_SIZE);
    LayoutNode::VSpace {
        rect: Rect::new(x, y, 0.0, h),
    }
}

// ---------------------------------------------------------------------------
// Plot placement (multi-view)
// ---------------------------------------------------------------------------

/// A plot leaf with its component-path identity and positioned rect.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedPlot {
    /// Component path of the plot node (e.g. `root`, `root/hconcat[0]`).
    /// Matches `brightfield_sql::collect_plot_groups`' `plot_path`, so a
    /// positioned plot joins back to its marks' data.
    pub path: String,
    /// The plot's position and size within the viewport.
    pub rect: Rect,
}

/// The positioned plot leaves of a spec, each with its component-path identity.
///
/// This is the multi-view consumer's view of the layout — "where does each plot
/// go, and which plot is it?" The `path` join key matches the per-plot mark
/// grouping, so a renderer can place each plot's scene at its rect.
#[must_use]
pub fn placed_plots(spec: &Spec, viewport: Rect) -> Vec<PlacedPlot> {
    let mut out = Vec::new();
    if let Some(tree) = compute_layout(spec, viewport) {
        collect_placed_plots(&tree, "root", &mut out);
    }
    out
}

fn collect_placed_plots(node: &LayoutNode, path: &str, out: &mut Vec<PlacedPlot>) {
    match node {
        LayoutNode::Plot { rect, .. } => out.push(PlacedPlot {
            path: path.to_string(),
            rect: *rect,
        }),
        LayoutNode::HConcat { children, .. } => {
            for (i, child) in children.iter().enumerate() {
                collect_placed_plots(child, &format!("{path}/hconcat[{i}]"), out);
            }
        }
        LayoutNode::VConcat { children, .. } => {
            for (i, child) in children.iter().enumerate() {
                collect_placed_plots(child, &format!("{path}/vconcat[{i}]"), out);
            }
        }
        // Non-plot leaves (spacers, standalone legends/inputs/marks) are not
        // plot render units; ignored here.
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Input placement (widgets — card 0005)
// ---------------------------------------------------------------------------

/// A composition-level input widget (e.g. a slider) with its component-path
/// identity and positioned rect — the input analogue of [`PlacedPlot`].
///
/// The `path` uses the same scheme as [`PlacedPlot`] / `collect_plot_groups`
/// (`root`, `root/hconcat[0]`, …), so it joins to the input's AST node via
/// [`collect_input_nodes`]. See [`placed_input_nodes`] for the joined view.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedInput {
    /// Component path of the input node.
    pub path: String,
    /// The widget's position and reserved size within the viewport.
    pub rect: Rect,
}

/// The positioned composition-level input leaves of a spec.
///
/// Mirrors [`placed_plots`]: walks the layout tree and emits one [`PlacedInput`]
/// per `input:` node placed in an hconcat/vconcat. Inputs nested inside a plot's
/// items are not composition widgets and are ignored (matching
/// [`collect_placed_plots`]'s stop-at-plot behaviour).
#[must_use]
pub fn placed_inputs(spec: &Spec, viewport: Rect) -> Vec<PlacedInput> {
    let mut out = Vec::new();
    if let Some(tree) = compute_layout(spec, viewport) {
        collect_placed_inputs(&tree, "root", &mut out);
    }
    out
}

fn collect_placed_inputs(node: &LayoutNode, path: &str, out: &mut Vec<PlacedInput>) {
    match node {
        LayoutNode::Input { rect } => out.push(PlacedInput {
            path: path.to_string(),
            rect: *rect,
        }),
        LayoutNode::HConcat { children, .. } => {
            for (i, child) in children.iter().enumerate() {
                collect_placed_inputs(child, &format!("{path}/hconcat[{i}]"), out);
            }
        }
        LayoutNode::VConcat { children, .. } => {
            for (i, child) in children.iter().enumerate() {
                collect_placed_inputs(child, &format!("{path}/vconcat[{i}]"), out);
            }
        }
        // Stop at plots (a slider is a composition sibling, not a plot item);
        // ignore spacers/legends/marks/interactors.
        _ => {}
    }
}

/// The composition-level input AST nodes of a spec, each paired with its
/// component path — the node-side of the [`placed_inputs`] join.
///
/// Walks the [`Component`] tree with the *same* path scheme as
/// [`collect_placed_inputs`] (which walks the layout tree). Because
/// [`compute_layout`] maps each Component to exactly one LayoutNode, the paths
/// align, so a placed rect joins to its `Input` node by path.
#[must_use]
pub fn collect_input_nodes(spec: &Spec) -> Vec<(String, &Input)> {
    let mut out = Vec::new();
    if let Some(root) = &spec.root {
        collect_input_nodes_in(root, "root", &mut out);
    }
    out
}

fn collect_input_nodes_in<'a>(
    component: &'a Component,
    path: &str,
    out: &mut Vec<(String, &'a Input)>,
) {
    match component {
        Component::Input(input) => out.push((path.to_string(), input)),
        Component::HConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                collect_input_nodes_in(item, &format!("{path}/hconcat[{i}]"), out);
            }
        }
        Component::VConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                collect_input_nodes_in(item, &format!("{path}/vconcat[{i}]"), out);
            }
        }
        // Stop at plots; ignore spacers/legends/marks/interactors.
        _ => {}
    }
}

/// Positioned input widgets joined to their AST nodes: `(rect, &Input)` per
/// composition-level input. This is the app-facing view — build a
/// [`SliderBinding`] from each `&Input` and host a widget at its `rect`.
///
/// [`SliderBinding`]: (see brightfield-ui)
#[must_use]
pub fn placed_input_nodes<'a>(spec: &'a Spec, viewport: Rect) -> Vec<(Rect, &'a Input)> {
    let placed = placed_inputs(spec, viewport);
    let nodes = collect_input_nodes(spec);
    placed
        .into_iter()
        .filter_map(|p| {
            nodes
                .iter()
                .find(|(path, _)| path == &p.path)
                .map(|(_, node)| (p.rect, *node))
        })
        .collect()
}

/// The plot AST nodes of a spec, each paired with its component path — the
/// same path scheme as [`placed_plots`]. Lets a consumer join a positioned plot
/// back to its `PlotNode` (e.g. to read the `name` attribute a standalone
/// legend's `for:` references).
#[must_use]
pub fn collect_plot_nodes(spec: &Spec) -> Vec<(String, &PlotNode)> {
    let mut out = Vec::new();
    if let Some(root) = &spec.root {
        collect_plot_nodes_in(root, "root", &mut out);
    }
    out
}

fn collect_plot_nodes_in<'a>(
    component: &'a Component,
    path: &str,
    out: &mut Vec<(String, &'a PlotNode)>,
) {
    match component {
        Component::Plot(plot) => out.push((path.to_string(), plot)),
        Component::HConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                collect_plot_nodes_in(item, &format!("{path}/hconcat[{i}]"), out);
            }
        }
        Component::VConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                collect_plot_nodes_in(item, &format!("{path}/vconcat[{i}]"), out);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Legend placement (standalone colour legends)
// ---------------------------------------------------------------------------

/// A composition-level standalone `legend:` node with its component-path
/// identity and positioned rect — the legend analogue of [`PlacedInput`].
///
/// Uses the same path scheme as [`PlacedPlot`] / [`placed_inputs`], so a placed
/// legend joins to its [`LegendNode`] via [`collect_legend_nodes`]. See
/// [`placed_legend_nodes`] for the joined view an app hosts.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedLegend {
    /// Component path of the legend node.
    pub path: String,
    /// The legend's position and reserved size within the viewport.
    pub rect: Rect,
}

/// The positioned composition-level standalone legends of a spec.
///
/// Mirrors [`placed_inputs`]: walks the layout tree and emits one
/// [`PlacedLegend`] per `legend:` node placed in an hconcat/vconcat. A legend
/// nested inside a plot's items is that plot's own inline legend (drawn in the
/// plot scene), not a composition node, so it is ignored here.
#[must_use]
pub fn placed_legends(spec: &Spec, viewport: Rect) -> Vec<PlacedLegend> {
    let mut out = Vec::new();
    if let Some(tree) = compute_layout(spec, viewport) {
        collect_placed_legends(&tree, "root", &mut out);
    }
    out
}

fn collect_placed_legends(node: &LayoutNode, path: &str, out: &mut Vec<PlacedLegend>) {
    match node {
        LayoutNode::Legend { rect } => out.push(PlacedLegend {
            path: path.to_string(),
            rect: *rect,
        }),
        LayoutNode::HConcat { children, .. } => {
            for (i, child) in children.iter().enumerate() {
                collect_placed_legends(child, &format!("{path}/hconcat[{i}]"), out);
            }
        }
        LayoutNode::VConcat { children, .. } => {
            for (i, child) in children.iter().enumerate() {
                collect_placed_legends(child, &format!("{path}/vconcat[{i}]"), out);
            }
        }
        // Stop at plots (an inline legend is a plot item, not a composition
        // node); ignore spacers/inputs/marks/interactors.
        _ => {}
    }
}

/// The composition-level standalone legend AST nodes, each paired with its
/// component path — the node-side of the [`placed_legends`] join. Walks the
/// [`Component`] tree with the same path scheme as [`collect_placed_legends`].
#[must_use]
pub fn collect_legend_nodes(spec: &Spec) -> Vec<(String, &crate::ast::LegendNode)> {
    let mut out = Vec::new();
    if let Some(root) = &spec.root {
        collect_legend_nodes_in(root, "root", &mut out);
    }
    out
}

fn collect_legend_nodes_in<'a>(
    component: &'a Component,
    path: &str,
    out: &mut Vec<(String, &'a crate::ast::LegendNode)>,
) {
    match component {
        Component::Legend(legend) => out.push((path.to_string(), legend)),
        Component::HConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                collect_legend_nodes_in(item, &format!("{path}/hconcat[{i}]"), out);
            }
        }
        Component::VConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                collect_legend_nodes_in(item, &format!("{path}/vconcat[{i}]"), out);
            }
        }
        // Stop at plots; ignore spacers/inputs/marks/interactors.
        _ => {}
    }
}

/// Positioned standalone legends joined to their AST nodes: `(rect, &LegendNode)`
/// per composition-level legend. The app-facing view — resolve each legend's
/// `for:` plot and draw its colour scale at `rect`.
#[must_use]
pub fn placed_legend_nodes<'a>(
    spec: &'a Spec,
    viewport: Rect,
) -> Vec<(Rect, &'a crate::ast::LegendNode)> {
    let placed = placed_legends(spec, viewport);
    let nodes = collect_legend_nodes(spec);
    placed
        .into_iter()
        .filter_map(|p| {
            nodes
                .iter()
                .find(|(path, _)| path == &p.path)
                .map(|(_, node)| (p.rect, *node))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;
    use crate::parse::{parse_spec, Format};
    use indexmap::IndexMap;

    /// A vconcat>hconcat>[plot, input:slider] spec used by the slider tests.
    const SLIDER_SPEC: &str = r#"
data:
  t:
    - { x: 1, y: 2 }
vconcat:
  - hconcat:
      - plot:
          - { mark: dot, data: { from: t }, x: x, y: y }
        width: 300
        height: 200
      - input: slider
        as: $threshold
        min: 0
        max: 10
        step: 1
"#;

    // slw ac-01 (card 0005): placed_inputs surfaces a composition-level input's
    // rect + path (the rect the multi-view extraction used to drop).
    #[test]
    fn slw_ac01_placed_inputs_path_and_rect() {
        let parsed = parse_spec(SLIDER_SPEC, Format::Yaml).expect("parse");
        let inputs = placed_inputs(&parsed.spec, Rect::new(0.0, 0.0, 0.0, 0.0));
        assert_eq!(inputs.len(), 1, "one composition-level input");
        assert_eq!(inputs[0].path, "root/vconcat[0]/hconcat[1]");
        // Reserved 200x32, stacked right of the 300-wide plot.
        assert_eq!(inputs[0].rect, Rect::new(300.0, 0.0, 200.0, 32.0));

        // A plots-only spec yields no placed inputs.
        let plots_only = r#"
data:
  t: [{ x: 1, y: 2 }]
plot:
  - { mark: dot, data: { from: t }, x: x, y: y }
"#;
        let p2 = parse_spec(plots_only, Format::Yaml).expect("parse");
        assert!(placed_inputs(&p2.spec, Rect::zero()).is_empty());
    }

    // slw ac-02: collect_input_nodes paths match placed_inputs, and
    // placed_input_nodes joins each rect to the Input that writes the param.
    #[test]
    fn slw_ac02_collect_input_nodes_and_join() {
        let parsed = parse_spec(SLIDER_SPEC, Format::Yaml).expect("parse");
        let spec = &parsed.spec;

        let nodes = collect_input_nodes(spec);
        let placed = placed_inputs(spec, Rect::zero());
        assert_eq!(nodes.len(), 1);
        assert_eq!(placed.len(), 1);
        assert_eq!(nodes[0].0, placed[0].path, "node path matches placed path");
        assert_eq!(
            nodes[0].1.as_param.as_ref().map(|p| p.0.as_str()),
            Some("threshold"),
            "the input writes $threshold"
        );

        let joined = placed_input_nodes(spec, Rect::zero());
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].0, Rect::new(300.0, 0.0, 200.0, 32.0));
        assert_eq!(
            joined[0].1.as_param.as_ref().map(|p| p.0.as_str()),
            Some("threshold")
        );
    }

    // multi-view: placed_plots joins identity to positioned rects
    #[test]
    fn mvdash_placed_plots_hconcat_paths_and_rects() {
        let yaml = r#"
data:
  t:
    - { x: 1, y: 2 }
hconcat:
  - plot:
      - { mark: dot, data: { from: t }, x: x, y: y }
    width: 300
    height: 200
  - plot:
      - { mark: line, data: { from: t }, x: x, y: y }
    width: 300
    height: 200
"#;
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
        let plots = placed_plots(&parsed.spec, Rect::new(0.0, 0.0, 0.0, 0.0));

        assert_eq!(plots.len(), 2, "two plots placed");
        // Paths match collect_plot_groups' plot_path so data joins to position.
        assert_eq!(plots[0].path, "root/hconcat[0]");
        assert_eq!(plots[1].path, "root/hconcat[1]");
        // hconcat stacks left-to-right with declared sizes.
        assert_eq!(plots[0].rect, Rect::new(0.0, 0.0, 300.0, 200.0));
        assert_eq!(plots[1].rect, Rect::new(300.0, 0.0, 300.0, 200.0));
    }

    // Spacer hosting: an `hspace:` between two plots offsets the right plot by
    // the gap. placed_plots drives both the window and the composite, so the
    // offset is the whole of "hosting" a spacer (a gap renders nothing).
    #[test]
    fn hspace_offsets_subsequent_plot() {
        let yaml = r#"
data:
  t:
    - { x: 1, y: 2 }
hconcat:
  - plot:
      - { mark: dot, data: { from: t }, x: x, y: y }
    width: 300
    height: 200
  - hspace: 64
  - plot:
      - { mark: line, data: { from: t }, x: x, y: y }
    width: 300
    height: 200
"#;
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
        let plots = placed_plots(&parsed.spec, Rect::zero());
        assert_eq!(plots.len(), 2, "spacers are not plots");
        assert_eq!(plots[0].rect.x, 0.0);
        // Second plot pushed right by plot-0 width (300) + the 64px spacer.
        assert_eq!(
            plots[1].rect.x, 364.0,
            "the 64px hspace offsets the right plot"
        );
    }

    // Standalone legend placement: a composition-level `legend:` node is emitted
    // by placed_legends (an inline plot legend is not), joined to its LegendNode.
    #[test]
    fn placed_legend_nodes_join_rect_to_node() {
        let yaml = r#"
data:
  t:
    - { x: 1, y: 2, grp: a }
hconcat:
  - plot:
      - { mark: dot, data: { from: t }, x: x, y: y, fill: grp }
    name: scatter
    width: 300
    height: 200
  - legend: color
    for: scatter
"#;
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
        let legends = placed_legends(&parsed.spec, Rect::zero());
        assert_eq!(legends.len(), 1, "one standalone legend");
        assert_eq!(legends[0].path, "root/hconcat[1]");
        // Positioned to the right of the 300px plot.
        assert_eq!(legends[0].rect.x, 300.0);

        let joined = placed_legend_nodes(&parsed.spec, Rect::zero());
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].1.channel, crate::vocab::LegendChannel::Color);
    }

    // ac-01: Rect struct
    #[test]
    fn mvdc_ac01_rect_fields() {
        let r = Rect::new(10.0, 20.0, 300.0, 200.0);
        assert_eq!(r.x, 10.0);
        assert_eq!(r.y, 20.0);
        assert_eq!(r.width, 300.0);
        assert_eq!(r.height, 200.0);
    }

    #[test]
    fn mvdc_ac01_rect_zero() {
        let r = Rect::zero();
        assert_eq!(r.x, 0.0);
        assert_eq!(r.y, 0.0);
        assert_eq!(r.width, 0.0);
        assert_eq!(r.height, 0.0);
    }

    // ac-02: LayoutNode enum is exhaustive over Component variants
    #[test]
    fn mvdc_ac02_layout_node_exhaustive_match() {
        fn discriminator(n: &LayoutNode) -> &'static str {
            match n {
                LayoutNode::Plot { .. } => "plot",
                LayoutNode::HConcat { .. } => "hconcat",
                LayoutNode::VConcat { .. } => "vconcat",
                LayoutNode::HSpace { .. } => "hspace",
                LayoutNode::VSpace { .. } => "vspace",
                LayoutNode::Legend { .. } => "legend",
                LayoutNode::Input { .. } => "input",
                LayoutNode::Mark { .. } => "mark",
                LayoutNode::Interactor { .. } => "interactor",
            }
        }
        let node = LayoutNode::Plot {
            rect: Rect::zero(),
            children: vec![],
        };
        assert_eq!(discriminator(&node), "plot");
    }

    #[test]
    fn mvdc_ac02_layout_node_rect_accessor() {
        let node = LayoutNode::Legend {
            rect: Rect::new(5.0, 10.0, 120.0, 24.0),
        };
        assert_eq!(node.rect().x, 5.0);
        assert_eq!(node.rect().width, 120.0);
    }

    // ac-03: compute_layout basic
    #[test]
    fn mvdc_ac03_single_plot() {
        let spec = Spec {
            root: Some(Component::Plot(PlotNode {
                items: vec![],
                attributes: IndexMap::new(),
            })),
            ..Default::default()
        };
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let tree = compute_layout(&spec, viewport);
        let node = tree.expect("should have layout");
        match &node {
            LayoutNode::Plot { rect, .. } => {
                assert_eq!(rect.x, 0.0);
                assert_eq!(rect.y, 0.0);
                assert_eq!(rect.width, DEFAULT_PLOT_WIDTH);
                assert_eq!(rect.height, DEFAULT_PLOT_HEIGHT);
            }
            _ => panic!("expected Plot node"),
        }
    }

    #[test]
    fn mvdc_ac03_no_root() {
        let spec = Spec::default();
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let tree = compute_layout(&spec, viewport);
        assert!(tree.is_none());
    }

    // ac-04: hconcat stacks left-to-right
    #[test]
    fn mvdc_ac04_hconcat_two_plots() {
        let spec = Spec {
            root: Some(Component::HConcat(ConcatNode {
                items: vec![
                    Component::Plot(PlotNode {
                        items: vec![],
                        attributes: IndexMap::new(),
                    }),
                    Component::Plot(PlotNode {
                        items: vec![],
                        attributes: IndexMap::new(),
                    }),
                ],
            })),
            ..Default::default()
        };
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 1600.0, 600.0)).unwrap();
        if let LayoutNode::HConcat { children, rect, .. } = &tree {
            assert_eq!(children.len(), 2);
            assert_eq!(children[0].rect().x, 0.0);
            assert_eq!(children[1].rect().x, DEFAULT_PLOT_WIDTH);
            assert_eq!(rect.width, DEFAULT_PLOT_WIDTH * 2.0);
        } else {
            panic!("expected HConcat");
        }
    }

    // ac-05: vconcat stacks top-to-bottom
    #[test]
    fn mvdc_ac05_vconcat_two_plots() {
        let spec = Spec {
            root: Some(Component::VConcat(ConcatNode {
                items: vec![
                    Component::Plot(PlotNode {
                        items: vec![],
                        attributes: IndexMap::new(),
                    }),
                    Component::Plot(PlotNode {
                        items: vec![],
                        attributes: IndexMap::new(),
                    }),
                ],
            })),
            ..Default::default()
        };
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 800.0, 1200.0)).unwrap();
        if let LayoutNode::VConcat { children, rect, .. } = &tree {
            assert_eq!(children.len(), 2);
            assert_eq!(children[0].rect().y, 0.0);
            assert_eq!(children[1].rect().y, DEFAULT_PLOT_HEIGHT);
            assert_eq!(rect.height, DEFAULT_PLOT_HEIGHT * 2.0);
        } else {
            panic!("expected VConcat");
        }
    }

    // ac-06: hspace and vspace gaps
    #[test]
    fn mvdc_ac06_hspace_gap() {
        let spec = Spec {
            root: Some(Component::HConcat(ConcatNode {
                items: vec![
                    Component::Plot(PlotNode {
                        items: vec![],
                        attributes: IndexMap::new(),
                    }),
                    Component::HSpace(SpaceNode {
                        value: SpecValue::Integer(35),
                    }),
                    Component::Plot(PlotNode {
                        items: vec![],
                        attributes: IndexMap::new(),
                    }),
                ],
            })),
            ..Default::default()
        };
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 1600.0, 600.0)).unwrap();
        if let LayoutNode::HConcat { children, .. } = &tree {
            assert_eq!(children.len(), 3);
            let plot1_x = children[0].rect().x;
            assert_eq!(plot1_x, 0.0);
            let space_x = children[1].rect().x;
            assert_eq!(space_x, DEFAULT_PLOT_WIDTH);
            assert_eq!(children[1].rect().width, 35.0);
            let plot2_x = children[2].rect().x;
            assert_eq!(plot2_x, DEFAULT_PLOT_WIDTH + 35.0);
        } else {
            panic!("expected HConcat");
        }
    }

    #[test]
    fn mvdc_ac06_vspace_gap() {
        let spec = Spec {
            root: Some(Component::VConcat(ConcatNode {
                items: vec![
                    Component::Plot(PlotNode {
                        items: vec![],
                        attributes: IndexMap::new(),
                    }),
                    Component::VSpace(SpaceNode {
                        value: SpecValue::String("1em".to_string()),
                    }),
                    Component::Plot(PlotNode {
                        items: vec![],
                        attributes: IndexMap::new(),
                    }),
                ],
            })),
            ..Default::default()
        };
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 800.0, 1200.0)).unwrap();
        if let LayoutNode::VConcat { children, .. } = &tree {
            assert_eq!(children.len(), 3);
            assert_eq!(children[0].rect().y, 0.0);
            assert_eq!(children[1].rect().y, DEFAULT_PLOT_HEIGHT);
            assert_eq!(children[1].rect().height, 16.0); // 1em = 16px
            assert_eq!(children[2].rect().y, DEFAULT_PLOT_HEIGHT + 16.0);
        } else {
            panic!("expected VConcat");
        }
    }

    // ac-07: resolve_space_value
    #[test]
    fn mvdc_ac07_numeric_pixels() {
        assert_eq!(resolve_space_value(&SpecValue::Integer(35), 16.0), 35.0);
        assert_eq!(resolve_space_value(&SpecValue::Float(2.5), 16.0), 2.5);
    }

    #[test]
    fn mvdc_ac07_em_units() {
        assert_eq!(
            resolve_space_value(&SpecValue::String("1em".to_string()), 16.0),
            16.0
        );
        assert_eq!(
            resolve_space_value(&SpecValue::String("2.5em".to_string()), 16.0),
            40.0
        );
    }

    #[test]
    fn mvdc_ac07_invalid_returns_zero() {
        assert_eq!(
            resolve_space_value(&SpecValue::String("bogus".to_string()), 16.0),
            0.0
        );
        assert_eq!(resolve_space_value(&SpecValue::Null, 16.0), 0.0);
    }

    // ac-08: nested composition (grid)
    #[test]
    fn mvdc_ac08_nested_grid() {
        // hconcat [ vconcat [A, B], vconcat [C, D] ]
        // A is at (0,0), B at (0, 400)
        // C is at (640, 0), D at (640, 400)
        let make_plot = || {
            Component::Plot(PlotNode {
                items: vec![],
                attributes: IndexMap::new(),
            })
        };
        let spec = Spec {
            root: Some(Component::HConcat(ConcatNode {
                items: vec![
                    Component::VConcat(ConcatNode {
                        items: vec![make_plot(), make_plot()],
                    }),
                    Component::VConcat(ConcatNode {
                        items: vec![make_plot(), make_plot()],
                    }),
                ],
            })),
            ..Default::default()
        };
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 1600.0, 1200.0)).unwrap();
        if let LayoutNode::HConcat { children, .. } = &tree {
            // First column (vconcat)
            if let LayoutNode::VConcat { children: col1, rect: col1_rect, .. } = &children[0] {
                assert_eq!(col1[0].rect().x, 0.0);
                assert_eq!(col1[0].rect().y, 0.0);
                assert_eq!(col1[1].rect().y, DEFAULT_PLOT_HEIGHT);
                assert_eq!(col1_rect.width, DEFAULT_PLOT_WIDTH);
            } else {
                panic!("expected VConcat for first column");
            }
            // Second column (vconcat) — C.x equals first column width
            if let LayoutNode::VConcat { children: col2, .. } = &children[1] {
                assert_eq!(col2[0].rect().x, DEFAULT_PLOT_WIDTH);
                assert_eq!(col2[0].rect().y, 0.0);
                assert_eq!(col2[1].rect().x, DEFAULT_PLOT_WIDTH);
                assert_eq!(col2[1].rect().y, DEFAULT_PLOT_HEIGHT);
            } else {
                panic!("expected VConcat for second column");
            }
        } else {
            panic!("expected HConcat");
        }
    }

    // ac-09: mixed component types
    #[test]
    fn mvdc_ac09_mixed_types() {
        use crate::vocab::{InputKind, LegendChannel, ImplStatus};
        let spec = Spec {
            root: Some(Component::HConcat(ConcatNode {
                items: vec![
                    Component::Plot(PlotNode {
                        items: vec![],
                        attributes: IndexMap::new(),
                    }),
                    Component::Input(Input {
                        kind: InputKind::Menu,
                        status: ImplStatus::Implemented,
                        as_param: None,
                        from_source: None,
                        filter_by: None,
                        options: IndexMap::new(),
                    }),
                    Component::Legend(LegendNode {
                        channel: LegendChannel::Color,
                        status: ImplStatus::Implemented,
                        options: IndexMap::new(),
                    }),
                ],
            })),
            ..Default::default()
        };
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 1600.0, 600.0)).unwrap();
        if let LayoutNode::HConcat { children, .. } = &tree {
            assert_eq!(children.len(), 3);
            // Plot at x=0
            assert_eq!(children[0].rect().x, 0.0);
            assert!(children[0].rect().width > 0.0);
            // Input at x=plot_width
            assert_eq!(children[1].rect().x, DEFAULT_PLOT_WIDTH);
            assert!(children[1].rect().width > 0.0);
            // Legend at x=plot_width+input_width
            assert_eq!(children[2].rect().x, DEFAULT_PLOT_WIDTH + DEFAULT_INPUT_WIDTH);
            assert!(children[2].rect().width > 0.0);
        } else {
            panic!("expected HConcat");
        }
    }

    // ac-10: plot attributes override defaults
    #[test]
    fn mvdc_ac10_plot_declared_size() {
        let mut attrs = IndexMap::new();
        attrs.insert("height".to_string(), SpecValue::Integer(200));
        attrs.insert("width".to_string(), SpecValue::Integer(500));
        let spec = Spec {
            root: Some(Component::Plot(PlotNode {
                items: vec![],
                attributes: attrs,
            })),
            ..Default::default()
        };
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 800.0, 600.0)).unwrap();
        assert_eq!(tree.rect().width, 500.0);
        assert_eq!(tree.rect().height, 200.0);
    }

    #[test]
    fn mvdc_ac10_plot_partial_override() {
        let mut attrs = IndexMap::new();
        attrs.insert("height".to_string(), SpecValue::Integer(200));
        // No width declared — should use default
        let spec = Spec {
            root: Some(Component::Plot(PlotNode {
                items: vec![],
                attributes: attrs,
            })),
            ..Default::default()
        };
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 800.0, 600.0)).unwrap();
        assert_eq!(tree.rect().width, DEFAULT_PLOT_WIDTH);
        assert_eq!(tree.rect().height, 200.0);
    }

    // ac-11 — REVISED (card 0009, lcf ac-01): `as:` on a legend is a selection
    // PRODUCER binding (clicking a swatch WRITES the selection), so the
    // corpus legends with `as: $toggle` / `as: $interval` must NOT appear in
    // the subscriber graph. The original ac-11 pinned the backwards wiring
    // (legend-as-subscriber); the fixed analysis arm skips the `as:` key and
    // surfaces the binding via `legend_bindings` instead.
    #[test]
    fn mvdc_ac11_legend_subscriber_graph() {
        use std::path::PathBuf;
        let legends_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("vendor")
            .join("mosaic-specs")
            .join("yaml")
            .join("legends.yaml");
        let source = std::fs::read_to_string(&legends_path).expect("read legends.yaml");
        let out = parse_spec(&source, Format::Yaml).expect("parse legends.yaml");

        let graph = crate::analysis::build_subscriber_graph(&out.spec);

        // The params stay in the graph (declared params are seeded), but no
        // legend subscribes to the selection it produces.
        for param in ["toggle", "interval"] {
            let subs = graph.get(param).expect("declared param in graph");
            assert!(
                subs.iter().all(|cp| !cp.0.contains("legend")),
                "`as: ${param}` is a producer binding — no legend subscriber, got: {subs:?}"
            );
        }
    }

    // ac-13: vendored corpus specs with composition
    #[test]
    fn mvdc_ac13_corpus_layout() {
        use std::path::PathBuf;
        let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("vendor")
            .join("mosaic-specs")
            .join("yaml");
        let viewport = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let mut tested = 0;
        for entry in std::fs::read_dir(&corpus).expect("corpus dir").flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read");
            let out = parse_spec(&source, Format::Yaml)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));

            // Only test specs that have composition (hconcat/vconcat).
            let has_composition = source.contains("hconcat") || source.contains("vconcat");
            if !has_composition {
                continue;
            }

            // Should not panic.
            let tree = compute_layout(&out.spec, viewport);
            if out.spec.root.is_some() {
                assert!(tree.is_some(), "{}: spec has root but layout returned None", path.display());
            }
            tested += 1;
        }
        assert!(tested > 0, "no composition specs found in corpus");
    }
}
