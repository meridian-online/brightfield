//! Layout computation for Mosaic spec composition trees.
//!
//! Pure function of the AST — walks [`Component`] trees and produces positioned
//! [`LayoutNode`] trees with pixel-accurate coordinates using a simple box
//! model: sequential stacking with fixed sizes, no flex negotiation.

use crate::ast::{Component, ConcatNode, PlotNode, Spec, SpaceNode, SpecValue};

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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::*;
    use crate::parse::{parse_spec, Format};
    use indexmap::IndexMap;

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

    // ac-11: legend as: bindings in subscriber graph (verify existing behaviour)
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

        // The legends.yaml spec has params: toggle and interval.
        // Legends with `as: $toggle` and `as: $interval` should appear as subscribers.
        let toggle_subs = graph.get("toggle").expect("toggle param in graph");
        assert!(
            toggle_subs.iter().any(|cp| cp.0.contains("legend")),
            "toggle should have at least one legend subscriber, got: {toggle_subs:?}"
        );
        let interval_subs = graph.get("interval").expect("interval param in graph");
        assert!(
            interval_subs.iter().any(|cp| cp.0.contains("legend")),
            "interval should have at least one legend subscriber, got: {interval_subs:?}"
        );
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
