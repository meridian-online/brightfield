//! Layout computation for Mosaic spec composition trees.
//!
//! Pure function of the AST — walks [`Component`] trees and produces positioned
//! [`LayoutNode`] trees with pixel-accurate coordinates.
//!
//! Two sizing regimes, chosen per axis by the viewport handed to
//! [`compute_layout`]. An axis whose viewport extent is not a positive finite
//! number is **unconstrained**: every node takes its intrinsic size on that
//! axis — a plot's declared `width:`/`height:`, or [`DEFAULT_PLOT_WIDTH`] /
//! [`DEFAULT_PLOT_HEIGHT`]. An axis with a positive extent is **constrained**:
//! the root fills it, and each container distributes what is left after its
//! fixed-size children among the ones the private `component_flexes` admits, in
//! proportion to their intrinsic sizes.

use crate::ast::{
    Component, ConcatNode, Input, Mark, ParamNode, PlotNode, SpaceNode, Spec, SpecValue,
    ValueOrParamRef,
};
use crate::date_format::DateFormat;
use crate::error::{FrameFault, FrameSide};
use crate::number_format::NumberFormat;
use crate::vocab::{InputKind, LegendChannel};
use indexmap::IndexMap;

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
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// A zero-sized rect at the origin.
    #[must_use]
    pub fn zero() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        }
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
/// Row height for a `style: radio` input's option rows (the
/// Meridian density row ladder). Shared with the vello resting twin
/// (`brightfield-render` `render_radio`), the SLIDER_* sync convention.
pub const RADIO_ROW_HEIGHT: f64 = 22.0;
/// Vertical chrome padding a radio-presented input adds around its rows —
/// `height = RADIO_ROW_HEIGHT · N + RADIO_CHROME_PAD`. Shared with the
/// render twin like [`RADIO_ROW_HEIGHT`].
pub const RADIO_CHROME_PAD: f64 = 10.0;
/// Default legend width (pixels).
pub const DEFAULT_LEGEND_WIDTH: f64 = 120.0;
/// Default legend height (pixels).
pub const DEFAULT_LEGEND_HEIGHT: f64 = 24.0;
/// The height of the band a colour legend takes under the plot it is for, in a
/// `vconcat` (pixels): the name and the ramp in one row and the ramp's values in
/// the row under it, or the swatches in a row. The band is as wide as the plot
/// above it, and the plot takes the height that is left.
pub const BELOW_LEGEND_HEIGHT: f64 = 44.0;
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

/// The size a container offers a child, per axis. `None` on an axis leaves
/// that axis to the child's intrinsic size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Avail {
    /// Offered width in pixels, or `None` for unconstrained.
    pub width: Option<f64>,
    /// Offered height in pixels, or `None` for unconstrained.
    pub height: Option<f64>,
}

impl Avail {
    /// Nothing offered on either axis — intrinsic sizing throughout.
    #[must_use]
    pub fn unconstrained() -> Self {
        Self::default()
    }

    /// The offer a `viewport` rect carries.
    ///
    /// An extent constrains its axis when it is finite and greater than zero;
    /// anything else — `0.0`, a negative, a NaN — is unconstrained, which is
    /// what [`Rect::zero`] means to every caller that measures a spec's own
    /// intrinsic size.
    #[must_use]
    pub fn from_viewport(viewport: Rect) -> Self {
        Self {
            width: offered(viewport.width),
            height: offered(viewport.height),
        }
    }
}

fn offered(extent: f64) -> Option<f64> {
    (extent.is_finite() && extent > 0.0).then_some(extent)
}

/// Compute the layout tree for a spec.
///
/// Walks `spec.root` and produces a [`LayoutTree`] with positioned nodes.
/// If the spec has no root component, returns `None`.
///
/// `viewport.x` / `viewport.y` place the root; `viewport.width` /
/// `viewport.height` are the size offered to it, read through
/// [`Avail::from_viewport`].
#[must_use]
pub fn compute_layout(spec: &Spec, viewport: Rect) -> LayoutTree {
    spec.root
        .as_ref()
        .map(|root| layout_component(root, viewport.x, viewport.y, Avail::from_viewport(viewport)))
}

/// Recursively lay out a component at the given `(x, y)` origin, taking `avail`
/// on each axis it offers and the component's intrinsic size on each it does
/// not.
fn layout_component(component: &Component, x: f64, y: f64, avail: Avail) -> LayoutNode {
    match component {
        Component::Plot(plot) => layout_plot(plot, x, y, avail),
        Component::HConcat(concat) => layout_hconcat(concat, x, y, avail),
        Component::VConcat(concat) => layout_vconcat(concat, x, y, avail),
        Component::HSpace(space) => layout_hspace(space, x, y),
        Component::VSpace(space) => layout_vspace(space, x, y),
        Component::Legend(_) => LayoutNode::Legend {
            rect: Rect::new(x, y, DEFAULT_LEGEND_WIDTH, DEFAULT_LEGEND_HEIGHT),
        },
        Component::Input(input) => LayoutNode::Input {
            rect: Rect::new(x, y, DEFAULT_INPUT_WIDTH, input_widget_height(input)),
        },
        Component::Mark(_) => LayoutNode::Mark {
            rect: Rect::new(
                x,
                y,
                avail.width.unwrap_or(DEFAULT_PLOT_WIDTH),
                avail.height.unwrap_or(DEFAULT_PLOT_HEIGHT),
            ),
        },
        Component::Interactor(_) => LayoutNode::Interactor {
            rect: Rect::new(x, y, 0.0, 0.0),
        },
    }
}

// ---------------------------------------------------------------------------
// Intrinsic measurement
// ---------------------------------------------------------------------------

/// The size a component takes when nothing is offered to it — the weight a
/// constrained container shares its residual out by, and the answer
/// [`compute_layout`] gives on an unconstrained axis.
///
/// A measurement pass rather than a second layout pass: measuring by laying the
/// subtree out unconstrained and reading its rect would double the recursion at
/// every level of a concat tree.
fn intrinsic_size(component: &Component) -> (f64, f64) {
    match component {
        Component::Plot(plot) => (plot_width(plot), plot_height(plot)),
        Component::HConcat(concat) => concat
            .items
            .iter()
            .map(intrinsic_size)
            .fold((0.0_f64, 0.0_f64), |(w, h), (cw, ch)| (w + cw, h.max(ch))),
        Component::VConcat(concat) => concat
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| match plot_above_legend(concat, i) {
                // A legend under its plot is as wide as that plot, so it adds
                // no width of its own to the column.
                Some(_) => (0.0, BELOW_LEGEND_HEIGHT),
                None => intrinsic_size(item),
            })
            .fold((0.0_f64, 0.0_f64), |(w, h), (cw, ch)| (w.max(cw), h + ch)),
        Component::HSpace(space) => (
            resolve_space_value(&space.value, DEFAULT_BASE_FONT_SIZE),
            0.0,
        ),
        Component::VSpace(space) => (
            0.0,
            resolve_space_value(&space.value, DEFAULT_BASE_FONT_SIZE),
        ),
        Component::Legend(_) => (DEFAULT_LEGEND_WIDTH, DEFAULT_LEGEND_HEIGHT),
        Component::Input(input) => (DEFAULT_INPUT_WIDTH, input_widget_height(input)),
        Component::Mark(_) => (DEFAULT_PLOT_WIDTH, DEFAULT_PLOT_HEIGHT),
        Component::Interactor(_) => (0.0, 0.0),
    }
}

/// Whether a constrained container gives this component a share of its residual
/// main-axis size, and stretches it on the cross axis. The match below is the
/// enumeration; nothing above it restates the arms.
fn component_flexes(component: &Component) -> bool {
    match component {
        Component::Plot(_) | Component::HConcat(_) | Component::VConcat(_) | Component::Mark(_) => {
            true
        }
        Component::HSpace(_)
        | Component::VSpace(_)
        | Component::Legend(_)
        | Component::Input(_)
        | Component::Interactor(_) => false,
    }
}

/// The main-axis size each item of a concat is offered, given `offer` on that
/// axis and each item's `(intrinsic, flexes)` measurement.
///
/// `None` for an item that does not flex, and for every item when the axis is
/// unconstrained. The flexing items' shares sum to the residual **exactly**:
/// the last of them absorbs the difference, so a row of tiles covers the box it
/// was given with no seam left by proportional rounding.
fn distribute(offer: Option<f64>, items: &[(f64, bool)]) -> Vec<Option<f64>> {
    let Some(offer) = offer else {
        return vec![None; items.len()];
    };
    let fixed: f64 = items
        .iter()
        .filter(|(_, flexes)| !flexes)
        .map(|(size, _)| *size)
        .sum();
    let weight: f64 = items
        .iter()
        .filter(|(_, flexes)| *flexes)
        .map(|(size, _)| *size)
        .sum();
    let residual = (offer - fixed).max(0.0);
    let last_flex = items.iter().rposition(|(_, flexes)| *flexes);

    let mut spent = 0.0_f64;
    items
        .iter()
        .enumerate()
        .map(|(i, (size, flexes))| {
            if !flexes {
                return None;
            }
            if Some(i) == last_flex {
                return Some((residual - spent).max(0.0));
            }
            let share = if weight > 0.0 {
                residual * size / weight
            } else {
                0.0
            };
            spent += share;
            Some(share)
        })
        .collect()
}

/// Per-style input widget height. Menu and checkbox
/// presentations keep the fixed `DEFAULT_INPUT_WIDTH × DEFAULT_INPUT_HEIGHT`
/// box; `style: radio` with a LITERAL N-option list reserves one
/// [`RADIO_ROW_HEIGHT`] row per option plus [`RADIO_CHROME_PAD`]. A radio
/// with DERIVED options (from/column — unknown N at layout time; this is a
/// pure spec fn with no engine in reach) is layout-sized as a menu, and app
/// assembly degrades its presentation to menu with a runtime Log Warning so
/// the widget never overflows the rect it was given.
fn input_widget_height(input: &Input) -> f64 {
    // Per-style sizing is a menu-family (`input: menu`) concern only: on any
    // other kind a stray `style: radio` + literal `options:` (inert keys for
    // that kind — e.g. a slider) must not earn a radio-tall rect.
    if input.kind != InputKind::Menu {
        return DEFAULT_INPUT_HEIGHT;
    }
    let style_is_radio = matches!(
        input.options.get("style"),
        Some(ValueOrParamRef::Value(SpecValue::String(s))) if s == "radio"
    );
    if !style_is_radio {
        return DEFAULT_INPUT_HEIGHT;
    }
    match input.options.get("options") {
        Some(ValueOrParamRef::Value(SpecValue::Array(items))) if !items.is_empty() => {
            RADIO_ROW_HEIGHT * items.len() as f64 + RADIO_CHROME_PAD
        }
        _ => DEFAULT_INPUT_HEIGHT,
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

/// A plot's four per-side insets resolved from its Mosaic inset attributes.
///
/// `None` = the attribute is absent for that side, so the caller applies its
/// own default; `Some(v)` = an explicit value that overrides the default —
/// including an explicit `Some(0.0)`, the Mosaic-exact opt-out. Values are
/// literal-only: a `$param` reference or any non-numeric value resolves to
/// `None` here (a truly non-numeric value also earns a
/// [`crate::parse::ParseWarning::NonNumericInset`]; a lifted `$param` is a
/// recorded deferral, matching hexbin's literal-only `binWidth`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SideInsets {
    /// Left (x range start) inset in pixels.
    pub left: Option<f64>,
    /// Right (x range end) inset in pixels.
    pub right: Option<f64>,
    /// Top (y range end — screen y grows downward) inset in pixels.
    pub top: Option<f64>,
    /// Bottom (y range start) inset in pixels.
    pub bottom: Option<f64>,
}

/// Resolve a plot's four per-side insets from its attributes with Observable
/// Plot most-specific-wins precedence, per side:
/// `left = xInsetLeft ?? xInset ?? inset` (and symmetrically `right`, `top`,
/// `bottom`). Only literal numeric attributes (`Integer`/`Float`) are read;
/// anything else — including a lifted `$param` — is treated as absent for that
/// key and falls through to the next-most-specific one. This is the single
/// framework-free primitive both layout models' insets derive from.
#[must_use]
pub fn resolve_plot_insets(plot: &PlotNode) -> SideInsets {
    let num = |key: &str| -> Option<f64> {
        plot.attributes.get(key).and_then(|v| match v {
            SpecValue::Integer(n) => Some(*n as f64),
            SpecValue::Float(f) => Some(*f),
            _ => None,
        })
    };
    let global = num("inset");
    let x_axis = num("xInset");
    let y_axis = num("yInset");
    SideInsets {
        left: num("xInsetLeft").or(x_axis).or(global),
        right: num("xInsetRight").or(x_axis).or(global),
        top: num("yInsetTop").or(y_axis).or(global),
        bottom: num("yInsetBottom").or(y_axis).or(global),
    }
}

/// A plot's four per-side margins as its spec declared them.
///
/// `None` = the spec left that side alone, so the caller lays its own default
/// there; `Some(v)` = a value the author wrote, including an explicit
/// `Some(0.0)`, the Mosaic-exact way to ask for no margin at all.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SideMargins {
    /// Left margin in pixels.
    pub left: Option<f64>,
    /// Right margin in pixels.
    pub right: Option<f64>,
    /// Top margin in pixels.
    pub top: Option<f64>,
    /// Bottom margin in pixels.
    pub bottom: Option<f64>,
}

/// Resolve a plot's four per-side margins from its own attributes with
/// Observable Plot most-specific-wins precedence, per side:
/// `left = marginLeft ?? margin` (and symmetrically `right`, `top`, `bottom`).
///
/// Literal numbers are read, as [`resolve_plot_insets`] reads them: a `$param`
/// reference or a non-numeric value is absent for that key and falls through
/// to the next-most-specific one. A negative number reads as `0.0`, since no
/// side has less than no margin; a number that is not finite is absent, since
/// no layout can be drawn from it. A pure reading of the plot node: the plot's
/// titles grow what this returns downstream, and `plotDefaults` is not consulted
/// (it is applied to no plot).
#[must_use]
pub fn resolve_plot_margins(plot: &PlotNode) -> SideMargins {
    let num = |key: &str| -> Option<f64> {
        plot.attributes
            .get(key)
            .and_then(|v| match v {
                SpecValue::Integer(n) => Some(*n as f64),
                SpecValue::Float(f) => Some(*f),
                _ => None,
            })
            .filter(|v| v.is_finite())
            .map(|v| v.max(0.0))
    };
    let global = num("margin");
    SideMargins {
        left: num("marginLeft").or(global),
        right: num("marginRight").or(global),
        top: num("marginTop").or(global),
        bottom: num("marginBottom").or(global),
    }
}

/// Observable Plot's default top margin, in pixels — the side a plot that
/// declares no `marginTop` is laid out at before its title grows it. The
/// render crate's `Margins::default` reads these four, so the fit check below
/// and the layout the plot is drawn in start from the same numbers.
pub const DEFAULT_MARGIN_TOP: f64 = 20.0;
/// Observable Plot's default right margin, in pixels. See [`DEFAULT_MARGIN_TOP`].
pub const DEFAULT_MARGIN_RIGHT: f64 = 20.0;
/// Observable Plot's default bottom margin, in pixels. See [`DEFAULT_MARGIN_TOP`].
pub const DEFAULT_MARGIN_BOTTOM: f64 = 30.0;
/// Observable Plot's default left margin, in pixels. See [`DEFAULT_MARGIN_TOP`].
pub const DEFAULT_MARGIN_LEFT: f64 = 40.0;

/// The band one present title adds to the margin it sits in: the left for a
/// y title, the bottom for an x title, the top for a plot title. The render
/// crate's `title::TITLE_BAND` is this constant, tied there to its title font.
pub const TITLE_BAND: f64 = 20.0;

/// **Why this plot has no data area to draw in**, or `None` when it has one.
///
/// Two faults, in the order they are checked:
///
/// 1. its `width` or `height` is NaN, infinite, zero or negative — the plot's
///    own attribute, or [`DEFAULT_PLOT_WIDTH`] / [`DEFAULT_PLOT_HEIGHT`] when
///    it declares no size;
/// 2. its left and right margins add up to more than its width, or its top
///    and bottom to more than its height, so the data area along that
///    dimension is inverted.
///
/// The margins are the ones the layout builds: each side the plot declares
/// ([`resolve_plot_margins`]) laid over Observable Plot's default, then grown
/// by [`TITLE_BAND`] on each side a title sits. A title is counted when the
/// plot names it (`xLabel: Temperature`, `title:`), not when it suppresses it
/// (`xLabel: null`), and — for a derived axis title, whose text the render
/// crate reads off the lowered channel map — when some mark in the plot binds
/// that axis to a column, a transform, an aggregate or a `$param`. The render
/// crate's channel map titles an axis bound to a column, a bin, an aggregate
/// or a `$param`, and all four are counted here. It binds nothing to title for
/// an expression, an object or a sort, or for a sum, mean, minimum or maximum
/// that names no column, and those are counted too: there the parse reserves
/// a band the layout then does not draw, which refuses a plot within one band
/// of fitting rather than drawing it inverted.
///
/// Judged on the size the spec declares. The window can still hand a plot a
/// smaller allocation than that, and the sampling notice's band is grown at
/// composition, when it is known whether the plot was sampled; neither is
/// visible from the spec.
///
/// **A plot holding no mark is not judged.** It draws no data area — the
/// composition places no chart for it — and Mosaic sizes a plot that only
/// hosts a legend to a zero-size frame on purpose: the curated `legends.yaml`
/// writes `width: 0` for exactly that.
#[must_use]
pub fn plot_frame_fault(plot: &PlotNode) -> Option<FrameFault> {
    if !plot
        .items
        .iter()
        .any(|item| matches!(item, Component::Mark(_)))
    {
        return None;
    }
    let width = plot_width(plot);
    let height = plot_height(plot);
    for (key, value) in [("width", width), ("height", height)] {
        if !(value.is_finite() && value > 0.0) {
            return Some(FrameFault::Dimension { key, value });
        }
    }

    let declared = resolve_plot_margins(plot);
    let titles = resolve_axis_titles(plot);
    let band = |present: bool| if present { TITLE_BAND } else { 0.0 };
    let side = |key, declared: Option<f64>, default, title_band| FrameSide {
        key,
        base: declared.unwrap_or(default),
        declared: declared.is_some(),
        title_band,
    };

    let left = side(
        "marginLeft",
        declared.left,
        DEFAULT_MARGIN_LEFT,
        band(draws_axis_title(plot, &titles.y, "y")),
    );
    let right = side("marginRight", declared.right, DEFAULT_MARGIN_RIGHT, 0.0);
    let top = side(
        "marginTop",
        declared.top,
        DEFAULT_MARGIN_TOP,
        band(titles.plot.is_some()),
    );
    let bottom = side(
        "marginBottom",
        declared.bottom,
        DEFAULT_MARGIN_BOTTOM,
        band(draws_axis_title(plot, &titles.x, "x")),
    );

    for (dimension, size, near, far) in [
        ("width", width, left, right),
        ("height", height, top, bottom),
    ] {
        if near.px() + far.px() > size {
            return Some(FrameFault::Margins {
                dimension,
                size,
                near,
                far,
            });
        }
    }
    None
}

/// Whether the layout reserves a title band for one positional axis — see
/// [`plot_frame_fault`] for why a derived title is counted from the marks'
/// bindings rather than from the lowered channel map the title's text comes
/// from.
fn draws_axis_title(plot: &PlotNode, decision: &AxisTitle, channel: &str) -> bool {
    match decision {
        AxisTitle::Override(_) => true,
        AxisTitle::Suppress => false,
        AxisTitle::Derive => plot.items.iter().any(|item| {
            let Component::Mark(mark) = item else {
                return false;
            };
            match mark.options.get(channel) {
                Some(ValueOrParamRef::Value(value)) => may_name_an_axis(value),
                // The lowerer projects a positional `$param` as a column named
                // for the param, and the render crate binds the axis to that
                // column and titles it with the param's name.
                Some(ValueOrParamRef::Param(_)) => true,
                None => false,
            }
        }),
    }
}

/// Whether a positional channel's value can bind the axis to something a
/// derived title is read from. Exhaustive with no wildcard, so a new
/// [`SpecValue`] variant chooses here before it compiles. A transform left out
/// would under-count: [`SpecValue::Bin`]'s binned column is one the render
/// crate titles.
fn may_name_an_axis(value: &SpecValue) -> bool {
    match value {
        SpecValue::String(_)
        | SpecValue::Object(_)
        | SpecValue::Expression(_)
        | SpecValue::Aggregate { .. }
        | SpecValue::Bin { .. }
        | SpecValue::Sort { .. } => true,
        SpecValue::Null
        | SpecValue::Bool(_)
        | SpecValue::Integer(_)
        | SpecValue::Float(_)
        | SpecValue::Array(_)
        | SpecValue::Param(_) => false,
    }
}

/// How a diagnostic names a plot: its component path, and its `name:` when it
/// declares one, since the name is what the author wrote and the path is not.
#[must_use]
pub fn plot_label(path: &str, plot: &PlotNode) -> String {
    match plot.attributes.get("name") {
        Some(SpecValue::String(name)) if !name.is_empty() => format!("{path} (`{name}`)"),
        _ => path.to_string(),
    }
}

/// One axis title's resolved decision, from a plot's `xLabel` / `yLabel`
/// attribute. Pure: the DERIVE case is turned into a concrete field name at the
/// render site (which holds the channel map). This resolver only decides which
/// of the three a plot asks for, mirroring [`resolve_plot_insets`]'s
/// literal-only reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AxisTitle {
    /// An explicit author string (`xLabel: Arrival Delay`) — used verbatim.
    Override(String),
    /// Explicitly suppressed (`xLabel: null` or `xLabel: ""`) — no title, no
    /// reserved band. The Mosaic-exact opt-out.
    Suppress,
    /// No label attribute (or a `$param` / non-string value that degrades) —
    /// derive the title from the encoding's column name at the render site.
    Derive,
}

/// A plot's resolved axis + plot titles — the DECISIONS. The DERIVE field names
/// and the margin growth resolve downstream (render/assembly). `x`/`y` are
/// per-axis decisions; `plot` is the optional per-plot title text (`None` =
/// absent, empty, null, or non-string).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AxisTitles {
    /// The x-axis title decision.
    pub x: AxisTitle,
    /// The y-axis title decision.
    pub y: AxisTitle,
    /// The per-plot title text, if any.
    pub plot: Option<String>,
}

/// Resolve one axis label attribute (`xLabel` / `yLabel`) into an [`AxisTitle`]
/// decision. Literal-only, mirroring [`resolve_plot_insets`]:
/// - a non-empty string → [`AxisTitle::Override`];
/// - an explicit `null` or empty string → [`AxisTitle::Suppress`];
/// - absent, a lifted `$param` (recorded deferral), or any other non-string
///   value → [`AxisTitle::Derive`]. A truly non-string value (number/boolean)
///   also earns a [`crate::parse::ParseWarning::NonStringLabel`] at PARSE time
///   (in `walk_plot`, exactly like `NonNumericInset`) — never here.
fn resolve_axis_label(plot: &PlotNode, key: &str) -> AxisTitle {
    match plot.attributes.get(key) {
        Some(SpecValue::String(s)) if !s.is_empty() => AxisTitle::Override(s.clone()),
        Some(SpecValue::String(_)) | Some(SpecValue::Null) => AxisTitle::Suppress,
        _ => AxisTitle::Derive,
    }
}

/// Resolve a plot's optional per-plot title from its `title` attribute — a
/// non-empty literal string, else `None` (absent, empty, null, or non-string).
#[must_use]
pub fn resolve_plot_title(plot: &PlotNode) -> Option<String> {
    match plot.attributes.get("title") {
        Some(SpecValue::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

/// Resolve a plot's axis + plot titles from its attributes, with the axis-inset
/// literal-only, most-specific convention. A PURE resolver: it returns the
/// Override / Suppress / Derive decision per axis (the DERIVE field name is
/// resolved at the render site from the channel map) plus the optional plot
/// title, and emits NO warnings — the non-string `ParseWarning::NonStringLabel`
/// is raised at parse time in `walk_plot`, exactly as `NonNumericInset` is. This
/// is the single framework-free primitive the render/scene path consumes.
#[must_use]
pub fn resolve_axis_titles(plot: &PlotNode) -> AxisTitles {
    AxisTitles {
        x: resolve_axis_label(plot, "xLabel"),
        y: resolve_axis_label(plot, "yLabel"),
        plot: resolve_plot_title(plot),
    }
}

/// Which of a plot's positional axes are pinned by a `Domain: Fixed`
/// attribute — the request that an axis hold its frame of reference while the
/// dashboard is filtered around it.
///
/// A pure spec reading, mirroring [`resolve_plot_insets`] and
/// [`resolve_axis_titles`]: it says what the author asked for, and holds no
/// opinion about what a scale then does with it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FixedDomains {
    /// `xDomain: Fixed` is declared on this plot.
    pub x: bool,
    /// `yDomain: Fixed` is declared on this plot.
    pub y: bool,
}

impl FixedDomains {
    /// Whether neither axis asks for a pin — the shape of every plot in the
    /// examples corpus, and the one a caller may skip work for.
    #[must_use]
    pub fn is_empty(self) -> bool {
        !self.x && !self.y
    }
}

/// The attribute value Mosaic reads as "pin this domain".
///
/// In a Mosaic spec `Fixed` is a JavaScript symbol; on the YAML/JSON wire it
/// arrives as this bare string, which is the form the vendored corpus carries.
/// Matched exactly rather than case-insensitively: a spec written for Mosaic is
/// the thing being read, and Mosaic resolves the name, not a spelling of it.
const FIXED: &str = "Fixed";

/// Resolve a plot's `xDomain` / `yDomain` pin request from its attributes.
///
/// Literal-only and per-axis, the same reading [`resolve_plot_insets`] gives
/// its keys. Any other value — a two-element array of explicit endpoints, a
/// lifted `$param`, an absent key — leaves that axis unpinned here; explicit
/// endpoints are a different instruction with a different effect, not a weaker
/// version of this one.
///
/// `xyDomain`, `fxDomain` and `fyDomain` are NOT read: see `deviations.yaml`
/// DEV-0005.
#[must_use]
pub fn resolve_fixed_domains(plot: &PlotNode) -> FixedDomains {
    let pinned =
        |key: &str| matches!(plot.attributes.get(key), Some(SpecValue::String(s)) if s == FIXED);
    FixedDomains {
        x: pinned("xDomain"),
        y: pinned("yDomain"),
    }
}

/// The tick count a renderer draws when a plot sets neither key — the count
/// the scene builder's `compute_ticks` calls drew before this resolver
/// existed, and what [`TickCounts::x_target`]/[`TickCounts::y_target`] fall
/// back to.
pub const DEFAULT_TICK_COUNT: usize = 5;

/// Which positional axes carry a `xTicks` / `yTicks` target tick count, and
/// what it is.
///
/// A pure spec reading, mirroring [`FixedDomains`] and [`AxisTitles`]: it says
/// what the author asked for, and holds no opinion about what a scale then
/// does with it. `None` covers both "the key is absent" and "the key is
/// present but not a request this build can act on" — see
/// [`tick_count_target`] for what is — which
/// [`crate::parse::ParseWarning::InvalidTickCount`] has already named at
/// parse time, exactly as [`resolve_plot_insets`]'s malformed inset is named
/// by `NonNumericInset` rather than by this resolver.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TickCounts {
    /// `xTicks`'s target count, when the plot wrote a valid one.
    pub x: Option<usize>,
    /// `yTicks`'s target count, when the plot wrote a valid one.
    pub y: Option<usize>,
}

impl TickCounts {
    /// The x axis's target tick count: what the plot asked for, or
    /// [`DEFAULT_TICK_COUNT`].
    #[must_use]
    pub fn x_target(self) -> usize {
        self.x.unwrap_or(DEFAULT_TICK_COUNT)
    }

    /// The y axis's target tick count: what the plot asked for, or
    /// [`DEFAULT_TICK_COUNT`].
    #[must_use]
    pub fn y_target(self) -> usize {
        self.y.unwrap_or(DEFAULT_TICK_COUNT)
    }
}

/// The most ticks one axis is asked to aim for.
///
/// A count reaches the tick loop as a step of `span / count` and the loop
/// draws one labelled tick per step, so a count with no upper bound is an
/// allocation with none: `xTicks: 1000000000` would try to build a billion
/// labelled ticks on the thread that draws the window. A count past this is
/// judged the malformed value it almost certainly is — a slip of extra zeros —
/// and named, rather than clamped to a number the author did not write.
pub const MAX_TICK_COUNT: usize = 1000;

/// The one judge of a `xTicks` / `yTicks` value: the target count it sets, or
/// `None` when the value is not a target.
///
/// A target is a literal whole number from 1 to [`MAX_TICK_COUNT`], written as
/// an integer or as a float with no fractional part. d3's tick rule
/// (`nice_step`, `crates/brightfield-render/src/axis.rs`) takes the count as a
/// target to aim a 1/2/5 step at, and a target of zero, a negative one or a
/// fractional one is not one it can aim at.
///
/// `None` is not itself a warning: a lifted `$param` is a recorded deferral and
/// resolves to it silently. The parser asks this same function to decide which
/// `None`s are malformed values to name, so the resolver and the warning cannot
/// disagree about what a valid count is.
#[must_use]
pub fn tick_count_target(value: &SpecValue) -> Option<usize> {
    let whole = match value {
        SpecValue::Integer(n) => usize::try_from(*n).ok()?,
        // Saturating cast: a float too large for `usize` lands on `usize::MAX`
        // and fails the range check below like any other oversized count.
        SpecValue::Float(f) if f.fract() == 0.0 => *f as usize,
        _ => return None,
    };
    (1..=MAX_TICK_COUNT).contains(&whole).then_some(whole)
}

/// Resolve a plot's `xTicks` / `yTicks` target tick count from its
/// attributes. Literal-only and per-axis, the same reading
/// [`resolve_fixed_domains`] gives its keys.
#[must_use]
pub fn resolve_tick_counts(plot: &PlotNode) -> TickCounts {
    let target = |key: &str| plot.attributes.get(key).and_then(tick_count_target);
    TickCounts {
        x: target("xTicks"),
        y: target("yTicks"),
    }
}

/// A tick format a plot asked for, read as the kind of axis text it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AxisFormat {
    /// A d3-format number specifier (`s`, `.2s`, `,d`), for a number axis.
    Number(NumberFormat),
    /// A d3-time-format date specifier (`%b`, `%Y-%m-%d`), for a date axis.
    Date(DateFormat),
}

/// Which positional axes carry an `xTickFormat` / `yTickFormat` this build
/// reads, and what it is.
///
/// A pure spec reading, like [`TickCounts`]: it says what the author asked for
/// and holds no opinion about what a scale then does with it — whether the axis
/// is one the format is for is settled where the scale is known, by
/// `brightfield_render::axis::tick_format_crosses_axis`. `None` covers the key
/// being absent, being `null` or a `$param`, and being a value that is no format
/// or names a directive this build does not read, which
/// [`crate::parse::ParseWarning`] has already named at parse time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickFormats {
    /// `xTickFormat`, when the plot wrote a format this build reads.
    pub x: Option<AxisFormat>,
    /// `yTickFormat`, when the plot wrote a format this build reads.
    pub y: Option<AxisFormat>,
}

/// What a `xTickFormat` / `yTickFormat` value is, as the one judge reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickFormatReading {
    /// A format this build reads.
    Format(AxisFormat),
    /// A date format that names a directive this build does not read, as it was
    /// written (`%K`). The axis draws its default text.
    UnreadDirective(String),
    /// `null` (Mosaic's "no format") or a lifted `$param`: a recorded deferral,
    /// not a typo. The axis draws its default text and nothing is said.
    Deferred,
    /// A value that is no format: `~~`, `.f`, a number, a list. The axis draws
    /// its default text and the parser names the value.
    Invalid,
}

/// The one judge of a `xTickFormat` / `yTickFormat` value, for the parser that
/// warns and the resolver that reads, so the two cannot disagree about what a
/// valid format is.
///
/// A string is a number format when d3-format's grammar reads it
/// ([`NumberFormat::parse`]). Otherwise it is a date format when it holds a
/// `%` directive and each directive in it is one d3-time-format reads
/// ([`DateFormat::parse`]), as `what_the_parser_warns_about_and_what_the_reader_accepts_do_not_overlap`
/// holds. A `%` ends a d3-format specifier (`%`, `+.1%`) and so
/// never has a character after it there, which is what tells a date directive
/// from a number specifier's percent sign: `abc%` is a mistyped number format, and
/// `%K` a date format with a directive this build does not read.
#[must_use]
pub fn read_tick_format(value: &SpecValue) -> TickFormatReading {
    match value {
        SpecValue::Param(_) | SpecValue::Null => TickFormatReading::Deferred,
        SpecValue::String(spec) => {
            if let Some(number) = NumberFormat::parse(spec) {
                return TickFormatReading::Format(AxisFormat::Number(number));
            }
            let dated = spec.match_indices('%').any(|(at, _)| at + 1 < spec.len());
            match DateFormat::parse(spec) {
                Ok(date) if dated && date.has_directive() => {
                    TickFormatReading::Format(AxisFormat::Date(date))
                }
                Err(directive) if dated => TickFormatReading::UnreadDirective(directive),
                _ => TickFormatReading::Invalid,
            }
        }
        _ => TickFormatReading::Invalid,
    }
}

/// Resolve a plot's `xTickFormat` / `yTickFormat` from its attributes.
/// Literal-only and per-axis, the same reading [`resolve_tick_counts`] gives
/// its keys.
#[must_use]
pub fn resolve_tick_formats(plot: &PlotNode) -> TickFormats {
    let read = |key: &str| match plot.attributes.get(key).map(read_tick_format) {
        Some(TickFormatReading::Format(format)) => Some(format),
        _ => None,
    };
    TickFormats {
        x: read("xTickFormat"),
        y: read("yTickFormat"),
    }
}

/// Which positional axes draw gridlines behind the marks.
///
/// A pure spec reading, like [`TickCounts`]: it says what the author asked for
/// and holds no opinion about what a scene then does with it. Each axis
/// resolves in three steps: the key that names the axis (`xGrid`, `yGrid`)
/// when it holds a switch, else the bare `grid` when it holds one, else the
/// default, which is to draw. A plot that sets none of the three draws the
/// gridlines it drew before these keys were read, and the axis key outranks
/// the bare one: `grid: true` with `xGrid: false` draws the horizontal rules
/// and leaves the vertical ones out.
///
/// A value that is no switch reads as absent — [`grid_switch`] is the judge,
/// and [`crate::parse::ParseWarning::InvalidGridSwitch`] names it at parse
/// time, exactly as [`resolve_tick_counts`]'s malformed count is named by
/// `InvalidTickCount` rather than by this resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridLines {
    /// Whether the x axis draws its vertical rules, one at each x tick.
    pub x: bool,
    /// Whether the y axis draws its horizontal rules, one at each y tick.
    pub y: bool,
}

impl Default for GridLines {
    /// Both axes draw: the gridlines a plot drew before `grid`, `xGrid` and
    /// `yGrid` were read.
    fn default() -> Self {
        Self { x: true, y: true }
    }
}

/// The one judge of a `grid` / `xGrid` / `yGrid` value: the switch it sets.
///
/// A literal `true` or `false` is a switch. A colour string, a number, a list,
/// `null` and a lifted `$param` are no switch this build reads. `None` is not
/// itself a warning: a lifted `$param` is a recorded deferral and resolves to
/// it silently. The parser asks this same function to decide which `None`s are
/// malformed values to name, so the resolver and the warning cannot disagree
/// about what a valid switch is.
#[must_use]
pub fn grid_switch(value: &SpecValue) -> Option<bool> {
    match value {
        SpecValue::Bool(on) => Some(*on),
        _ => None,
    }
}

/// Resolve a plot's `grid` / `xGrid` / `yGrid` from its attributes.
/// Literal-only and per-axis, the same reading [`resolve_tick_counts`] gives
/// its keys, with the axis key outranking the bare one.
#[must_use]
pub fn resolve_grid_lines(plot: &PlotNode) -> GridLines {
    let read = |key: &str| plot.attributes.get(key).and_then(grid_switch);
    let both = read("grid");
    let default = GridLines::default();
    GridLines {
        x: read("xGrid").or(both).unwrap_or(default.x),
        y: read("yGrid").or(both).unwrap_or(default.y),
    }
}

/// The name a plot's `colorScheme` gives, as the spec wrote it.
///
/// A string is the name. A `$param` is the name its value param holds *now*, so
/// a plot redrawn after the param is written draws in the scheme the param
/// then names; a param that holds anything but a string, a selection, and a
/// param nobody declared are no name. Having no name is not a warning here:
/// whether a written name is one a renderer draws is [`read_colour_scheme`]'s
/// to judge, at parse time, and a plot with no `colorScheme` draws its default.
#[must_use]
pub fn resolve_colour_scheme_name<'a>(
    plot: &'a PlotNode,
    params: &'a IndexMap<String, ParamNode>,
) -> Option<&'a str> {
    match plot.attributes.get("colorScheme")? {
        SpecValue::String(name) => Some(name),
        SpecValue::Param(param) => match params.get(&param.0) {
            Some(ParamNode::Value(SpecValue::String(name))) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// The names a plot's `colorScheme` can give and be drawn in, in the order the
/// renderer cycles them, default first.
///
/// This is the list the parser's warning judges against
/// ([`read_colour_scheme`]). `brightfield_render::scale::SequentialScheme` holds
/// its own list of every scheme it draws, and a render-side test holds the two
/// equal in both directions, so a name on one list and not the other fails
/// there. That holds for a name written as a string; a `$param` is read when
/// the plot is drawn, and a param that holds a name on neither list draws the
/// default with no warning.
pub const DRAWN_COLOUR_SCHEMES: [&str; 5] = ["viridis", "blues", "turbo", "meridian", "rdbu"];

/// What a plot's `colorScheme` value is, to the parser that warns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourSchemeReading {
    /// A name in [`DRAWN_COLOUR_SCHEMES`]: the plot draws along that ramp.
    Drawn,
    /// `null` or a lifted `$param`: a recorded deferral, not a typo. The name is
    /// read when the spec is drawn, and nothing is said here.
    Deferred,
    /// A name no renderer draws (`magma`, `Viridis`), or a value that is no name
    /// (a number, a list). The plot draws its default ramp and the parser names
    /// the value.
    Unknown,
}

/// The one judge of a plot's `colorScheme` value, for the parser that warns.
///
/// The names it accepts are [`DRAWN_COLOUR_SCHEMES`], case-exact as the
/// renderer's reading is. A `$param` is judged by the parser as a deferral
/// whatever it holds, because the parser has not yet seen the value the param
/// will hold; [`resolve_colour_scheme_name`] reads it when the plot is drawn.
#[must_use]
pub fn read_colour_scheme(value: &SpecValue) -> ColourSchemeReading {
    match value {
        SpecValue::Param(_) | SpecValue::Null => ColourSchemeReading::Deferred,
        SpecValue::String(name) if DRAWN_COLOUR_SCHEMES.contains(&name.as_str()) => {
            ColourSchemeReading::Drawn
        }
        _ => ColourSchemeReading::Unknown,
    }
}

/// The names a plot's `colorScale` can give and be drawn in: the straight ramp,
/// the one that diverges about a pivot, and the one that steps in a count
/// (`quantize`, with `colorN`). Any other Mosaic scale type (`quantile`,
/// `symlog`, `diverging-log`) is drawn as `linear`, and the parser names it.
pub const DRAWN_COLOUR_SCALES: [&str; 3] = ["linear", "diverging", "quantize"];

/// What a plot's `colorScale` value is, to the parser that warns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourScaleReading {
    /// A name in [`DRAWN_COLOUR_SCALES`].
    Drawn,
    /// `null` or a lifted `$param`: a recorded deferral, not a typo.
    Deferred,
    /// A scale type no renderer draws here, or a value that is no name. The
    /// plot draws the linear ramp and the parser names the value.
    Unknown,
}

/// The one judge of a plot's `colorScale` value, for the parser that warns.
/// Case-exact, as Mosaic's own names are.
#[must_use]
pub fn read_colour_scale(value: &SpecValue) -> ColourScaleReading {
    match value {
        SpecValue::Param(_) | SpecValue::Null => ColourScaleReading::Deferred,
        SpecValue::String(name) if DRAWN_COLOUR_SCALES.contains(&name.as_str()) => {
            ColourScaleReading::Drawn
        }
        _ => ColourScaleReading::Unknown,
    }
}

/// The name a plot's `colorScale` gives: the literal, or a `$param` that holds a
/// string *now*, as [`resolve_colour_scheme_name`] reads its own key. A plot with
/// no `colorScale`, and a param that holds a value other than a string, give no
/// name.
fn resolve_colour_scale_name<'a>(
    plot: &'a PlotNode,
    params: &'a IndexMap<String, ParamNode>,
) -> Option<&'a str> {
    match plot.attributes.get("colorScale") {
        Some(SpecValue::String(name)) => Some(name.as_str()),
        Some(SpecValue::Param(param)) => match params.get(&param.0) {
            Some(ParamNode::Value(SpecValue::String(name))) => Some(name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// Whether a plot's `colorScale` draws about a pivot: the literal `diverging`,
/// or a `$param` that holds it *now*, as [`resolve_colour_scheme_name`] reads
/// its own key. A plot with no `colorScale`, a name no renderer draws, and a
/// param that holds some other value is not diverging and draws the linear ramp.
#[must_use]
pub fn resolve_colour_scale_diverging(
    plot: &PlotNode,
    params: &IndexMap<String, ParamNode>,
) -> bool {
    resolve_colour_scale_name(plot, params) == Some("diverging")
}

/// Whether a plot's `colorScale` draws in steps: the literal `quantize`, or a
/// `$param` that holds it *now*. A plot with no `colorScale`, a name no renderer
/// draws, and a param that holds some other value does not step and draws the
/// ramp it drew before the key was read.
#[must_use]
pub fn resolve_colour_scale_quantize(
    plot: &PlotNode,
    params: &IndexMap<String, ParamNode>,
) -> bool {
    resolve_colour_scale_name(plot, params) == Some("quantize")
}

/// How many steps a `quantize` scale draws when the plot gives no count, which is
/// the count Mosaic's renderer draws when `colorN` is absent.
pub const DEFAULT_COLOUR_STEPS: usize = 5;

/// The most steps a plot's `colorN` can ask for. A count past it is no count the
/// legend could draw a block for in the room a number legend has, and a file
/// cannot make the colour scale as long as it likes.
pub const MAX_COLOUR_STEPS: usize = 256;

/// What a plot's `colorN` value is, to the parser that warns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourStepsReading {
    /// A whole number from one to [`MAX_COLOUR_STEPS`]: the plot draws that many
    /// steps under `colorScale: quantize`.
    Steps(usize),
    /// `null` or a lifted `$param`: a recorded deferral, not a typo.
    Deferred,
    /// A value that is no count (zero, a negative, a fraction, a string, a list,
    /// a count past [`MAX_COLOUR_STEPS`]). The plot draws
    /// [`DEFAULT_COLOUR_STEPS`] and the parser names the value.
    Unknown,
}

/// The one judge of a plot's `colorN` value, for the parser that warns and the
/// resolver that draws. An integer, or a float that is a whole number, from one
/// to [`MAX_COLOUR_STEPS`].
#[must_use]
pub fn read_colour_steps(value: &SpecValue) -> ColourStepsReading {
    let most = MAX_COLOUR_STEPS as i64;
    match value {
        SpecValue::Param(_) | SpecValue::Null => ColourStepsReading::Deferred,
        SpecValue::Integer(n) if (1..=most).contains(n) => ColourStepsReading::Steps(*n as usize),
        SpecValue::Float(f)
            if f.is_finite() && f.fract() == 0.0 && (1.0..=most as f64).contains(f) =>
        {
            ColourStepsReading::Steps(*f as usize)
        }
        _ => ColourStepsReading::Unknown,
    }
}

/// The count of steps a plot's `colorN` gives, if it gives one: a count
/// [`read_colour_steps`] accepts, or a `$param` whose value param holds one
/// *now*. `None` is the plot asking for [`DEFAULT_COLOUR_STEPS`], which is what
/// a missing key, a value that is no count and a param that holds no count draw.
#[must_use]
pub fn resolve_colour_steps(
    plot: &PlotNode,
    params: &IndexMap<String, ParamNode>,
) -> Option<usize> {
    let value = match plot.attributes.get("colorN")? {
        SpecValue::Param(param) => match params.get(&param.0) {
            Some(ParamNode::Value(value)) => value,
            _ => return None,
        },
        value => value,
    };
    match read_colour_steps(value) {
        ColourStepsReading::Steps(n) => Some(n),
        ColourStepsReading::Deferred | ColourStepsReading::Unknown => None,
    }
}

/// The pivot a plot's `colorPivot` gives, if it gives one: a number, or a
/// `$param` whose value param holds a number *now*. `None` is the plot asking
/// for the pivot to be chosen from its rows; whether a written value is a number
/// is [`colour_pivot`]'s to judge at parse time.
#[must_use]
pub fn resolve_colour_pivot(plot: &PlotNode, params: &IndexMap<String, ParamNode>) -> Option<f64> {
    match plot.attributes.get("colorPivot")? {
        SpecValue::Param(param) => match params.get(&param.0) {
            Some(ParamNode::Value(value)) => colour_pivot(value),
            _ => None,
        },
        value => colour_pivot(value),
    }
}

/// A `colorPivot` value read as a number: an integer or a finite float. The
/// parser asks this same function which written values are no pivot, so the
/// resolver and the warning cannot disagree about what a pivot is.
#[must_use]
pub fn colour_pivot(value: &SpecValue) -> Option<f64> {
    match value {
        SpecValue::Integer(n) => Some(*n as f64),
        SpecValue::Float(f) if f.is_finite() => Some(*f),
        _ => None,
    }
}

/// A `colorReverse` value read as the switch it sets: a literal `true` or
/// `false`. A string, a number, a list and `null` are no switch.
///
/// The parser asks this same function which written values to name, and the
/// resolver reads the key through it, so a value the parser accepts is a value
/// the plot draws and a value it names is one the plot draws as absent.
#[must_use]
pub fn colour_reverse_switch(value: &SpecValue) -> Option<bool> {
    match value {
        SpecValue::Bool(on) => Some(*on),
        _ => None,
    }
}

/// Whether a plot's `colorReverse` runs its colour ramp or its category list the
/// other way: the literal `true`, or a `$param` whose value param holds `true`
/// *now*, as [`resolve_colour_pivot`] reads its own key. A plot with no
/// `colorReverse`, a value that is no switch, and a param that holds something
/// else or that nobody declared draws as a file without the key draws.
#[must_use]
pub fn resolve_colour_reverse(plot: &PlotNode, params: &IndexMap<String, ParamNode>) -> bool {
    let switch = match plot.attributes.get("colorReverse") {
        Some(SpecValue::Param(param)) => match params.get(&param.0) {
            Some(ParamNode::Value(value)) => colour_reverse_switch(value),
            _ => None,
        },
        Some(value) => colour_reverse_switch(value),
        None => None,
    };
    switch.unwrap_or(false)
}

/// What a plot's `colorDomain` fixes: the two ends of a number ramp, or the
/// categories of a string column in the order the legend lists them.
#[derive(Debug, Clone, PartialEq)]
pub enum ColourDomain {
    /// Two numbers, low then high: a linear or diverging ramp's ends.
    Ends(f64, f64),
    /// One or more categories, first to last.
    Categories(Vec<String>),
}

/// A `colorDomain` value read as the domain it fixes: a list of two finite
/// numbers with the low end first, or a non-empty list of strings.
///
/// A string (Mosaic's `Fixed`, which asks for the data's own domain held still
/// and which this build leaves unread), a list of any other shape, a pair of
/// numbers with the high end first or equal, and a list that mixes strings
/// with numbers are no domain, and a plot that writes one draws as a file
/// without the key does.
#[must_use]
pub fn colour_domain(value: &SpecValue) -> Option<ColourDomain> {
    let SpecValue::Array(items) = value else {
        return None;
    };
    let number = |item: &SpecValue| match item {
        SpecValue::Integer(n) => Some(*n as f64),
        SpecValue::Float(f) if f.is_finite() => Some(*f),
        _ => None,
    };
    if let [lo, hi] = items.as_slice() {
        if let (Some(lo), Some(hi)) = (number(lo), number(hi)) {
            return (lo < hi).then_some(ColourDomain::Ends(lo, hi));
        }
    }
    let categories: Option<Vec<String>> = items
        .iter()
        .map(|item| match item {
            SpecValue::String(name) => Some(name.clone()),
            _ => None,
        })
        .collect();
    categories
        .filter(|names| !names.is_empty())
        .map(ColourDomain::Categories)
}

/// A `colorRange` value read as the colours it lists, as written: a non-empty
/// list of strings. Whether each string is a colour is the renderer's to judge,
/// which has the parser for one.
#[must_use]
pub fn colour_range(value: &SpecValue) -> Option<Vec<&str>> {
    let SpecValue::Array(items) = value else {
        return None;
    };
    let names: Option<Vec<&str>> = items
        .iter()
        .map(|item| match item {
            SpecValue::String(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();
    names.filter(|names| !names.is_empty())
}

/// The value a plot's attribute `key` holds: what it wrote, or what the value
/// param it names holds *now*, so a plot redrawn after the param is written draws
/// from the value the param then holds. A selection and a param nobody declared
/// hold no value.
fn literal_attribute<'a>(
    plot: &'a PlotNode,
    params: &'a IndexMap<String, ParamNode>,
    key: &str,
) -> Option<&'a SpecValue> {
    match plot.attributes.get(key)? {
        SpecValue::Param(param) => match params.get(&param.0) {
            Some(ParamNode::Value(value)) => Some(value),
            _ => None,
        },
        value => Some(value),
    }
}

/// The domain a plot's `colorDomain` fixes, if it fixes one: a literal list, or a
/// `$param` whose value param holds a list *now*. `None` is a plot that draws the
/// domain its rows give, whether it wrote no key, wrote `Fixed`, or wrote a value
/// [`colour_domain`] reads as no domain.
#[must_use]
pub fn resolve_colour_domain(
    plot: &PlotNode,
    params: &IndexMap<String, ParamNode>,
) -> Option<ColourDomain> {
    colour_domain(literal_attribute(plot, params, "colorDomain")?)
}

/// The colours a plot's `colorRange` lists, if it lists any: a literal list, or a
/// `$param` whose value param holds a list *now*, as [`resolve_colour_domain`]
/// reads its own key.
#[must_use]
pub fn resolve_colour_range<'a>(
    plot: &'a PlotNode,
    params: &'a IndexMap<String, ParamNode>,
) -> Option<Vec<&'a str>> {
    colour_range(literal_attribute(plot, params, "colorRange")?)
}

/// What one positional axis asks of where it starts and ends: `xZero` and
/// `xNice`, or `yZero` and `yNice`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AxisEnd {
    /// The axis carries its domain to zero if the data stops short of it. It
    /// extends a domain and does not narrow one: a data range that already holds
    /// zero is left as it is.
    pub zero: bool,
    /// The axis ends on round numbers — the steps its ticks are drawn at — and
    /// widens its domain outward to reach them.
    pub nice: bool,
}

impl AxisEnd {
    /// Whether the axis asks for neither, the state in which the renderer has
    /// nothing to do to its domain.
    #[must_use]
    pub fn is_empty(self) -> bool {
        !self.zero && !self.nice
    }
}

/// Where a plot's positional axes start and end — `xZero`, `xNice`, `yZero`
/// and `yNice`.
///
/// A pure spec reading, mirroring [`GridLines`] and [`TickCounts`]: it says what
/// the author asked for and holds no opinion about the scale it lands on. The
/// default asks for nothing on either axis, which is what a plot drew before the
/// four keys were read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AxisEnds {
    /// The x axis's request.
    pub x: AxisEnd,
    /// The y axis's request.
    pub y: AxisEnd,
}

impl AxisEnds {
    /// Whether neither axis asks for anything — the shape of a plot that
    /// writes no one of the four keys, and the one a caller may skip work for.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.x.is_empty() && self.y.is_empty()
    }
}

/// The one judge of an `xZero` / `yZero` / `xNice` / `yNice` value: the switch
/// it sets.
///
/// A literal `true` or `false` is a switch. Observable Plot also reads a number
/// or an interval at a `nice` key, as the count or the step to round to; this
/// build reads neither, and neither is a switch. A string, a list, `null` and a
/// lifted `$param` are no switch either. `None` is not itself a warning: a
/// lifted `$param` is a recorded deferral and resolves to it silently. The
/// parser asks this same function to decide which `None`s are malformed values
/// to name, so the resolver and the warning cannot disagree about what a valid
/// switch is.
#[must_use]
pub fn axis_end_switch(value: &SpecValue) -> Option<bool> {
    match value {
        SpecValue::Bool(on) => Some(*on),
        _ => None,
    }
}

/// Resolve a plot's `xZero` / `xNice` / `yZero` / `yNice` from its attributes.
/// Literal-only and per-axis, the same reading [`resolve_grid_lines`] gives its
/// keys; a key that is absent, or is no switch, asks for nothing.
#[must_use]
pub fn resolve_axis_ends(plot: &PlotNode) -> AxisEnds {
    let on = |key: &str| {
        plot.attributes
            .get(key)
            .and_then(axis_end_switch)
            .unwrap_or(false)
    };
    AxisEnds {
        x: AxisEnd {
            zero: on("xZero"),
            nice: on("xNice"),
        },
        y: AxisEnd {
            zero: on("yZero"),
            nice: on("yNice"),
        },
    }
}

/// Which positional axes a plot's spec asked to draw from high to low:
/// `xReverse` and `yReverse`.
///
/// A pure spec reading, mirroring [`AxisEnds`]: it says what the author asked
/// for and holds no opinion about the scale it lands on. The default reverses
/// neither axis, which is what a plot drew before the two keys were read — x
/// from the left edge to the right, y from the bottom edge to the top.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AxisReverse {
    /// The x axis draws its lowest value at the right.
    pub x: bool,
    /// The y axis draws its lowest value at the top.
    pub y: bool,
}

impl AxisReverse {
    /// Whether neither axis is reversed — the shape of a plot that writes
    /// neither key, and the one a caller may skip work for.
    #[must_use]
    pub fn is_empty(self) -> bool {
        !self.x && !self.y
    }
}

/// The one judge of an `xReverse` / `yReverse` value: the switch it sets.
///
/// A literal `true` or `false` is a switch. A string, a number, a list, `null`
/// and a lifted `$param` are no switch. `None` is not itself a warning: a
/// lifted `$param` is a recorded deferral and resolves to it silently. The
/// parser asks this same function to decide which `None`s are malformed values
/// to name, so the resolver and the warning cannot disagree about what a valid
/// switch is.
#[must_use]
pub fn axis_reverse_switch(value: &SpecValue) -> Option<bool> {
    match value {
        SpecValue::Bool(on) => Some(*on),
        _ => None,
    }
}

/// Resolve a plot's `xReverse` / `yReverse` from its attributes. Literal-only
/// and per-axis, the same reading [`resolve_axis_ends`] gives its keys; a key
/// that is absent, or is no switch, reverses nothing.
#[must_use]
pub fn resolve_axis_reverse(plot: &PlotNode) -> AxisReverse {
    let on = |key: &str| {
        plot.attributes
            .get(key)
            .and_then(axis_reverse_switch)
            .unwrap_or(false)
    };
    AxisReverse {
        x: on("xReverse"),
        y: on("yReverse"),
    }
}

/// Which positional axis a plot attribute speaks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlotAxis {
    /// The x axis.
    X,
    /// The y axis.
    Y,
}

/// The transform a positional scale applies between a data value and its
/// pixel, as Mosaic and Observable Plot name it.
///
/// A plot with no such key takes [`ScaleType::Linear`] — the reading a spec
/// written before this key existed already had, held by
/// `a_plot_written_x_scale_log_resolves_log`.
///
/// This is a PURE spec reading — the arithmetic lives in
/// `brightfield_render::scale::Scale`, and the binning that has to happen in
/// the same space lives in `brightfield_sql`'s rect lowerer. Both convert from
/// this; neither re-parses the attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScaleType {
    /// `value` maps straight onto the pixel range. The default.
    #[default]
    Linear,
    /// `log(value)` maps onto the pixel range. Undefined at and below zero:
    /// Mosaic drops those rows rather than drawing them somewhere arbitrary.
    Log,
    /// `sign(v) * log1p(|v| / C)` — logarithmic away from the origin and
    /// linear through it, so a column holding a zero keeps that row.
    Symlog,
}

impl ScaleType {
    /// The wire name Mosaic writes, and the one this build reads back.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Log => "log",
            Self::Symlog => "symlog",
        }
    }

    /// Read a wire name, or `None` for a spelling this build does not know.
    ///
    /// Matched exactly rather than case-insensitively, for the reason the
    /// private `FIXED` literal above is: a spec written for Mosaic is the
    /// thing being read, and Mosaic resolves the name and not a spelling of
    /// it.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        match name {
            "linear" => Some(Self::Linear),
            "log" => Some(Self::Log),
            "symlog" => Some(Self::Symlog),
            _ => None,
        }
    }

    /// Whether this transform is undefined at or below zero — the question the
    /// lowerer asks before it decides which rows can be binned at all.
    #[must_use]
    pub fn drops_nonpositive(self) -> bool {
        matches!(self, Self::Log)
    }
}

/// The scale type each of a plot's positional axes resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlotScales {
    /// The x axis's transform.
    pub x: ScaleType,
    /// The y axis's transform.
    pub y: ScaleType,
}

impl PlotScales {
    /// The transform on one axis.
    #[must_use]
    pub fn axis(self, axis: PlotAxis) -> ScaleType {
        match axis {
            PlotAxis::X => self.x,
            PlotAxis::Y => self.y,
        }
    }

    /// Whether both axes are linear — the shape of every spec written before
    /// this key was read, and the one a caller may skip work for.
    #[must_use]
    pub fn is_linear(self) -> bool {
        self.x == ScaleType::Linear && self.y == ScaleType::Linear
    }
}

/// **What a stacked mark's segments are measured against** — Observable Plot's
/// stack `offset`, as a plot attribute.
///
/// Plot writes it on the stack transform (`stackY({offset: "normalize"})`);
/// brightfield writes it on the PLOT, because the control that throws it is a
/// plot's control and the switch beside it already writes `xScale` there.
/// The deviation is recorded as DEV-0007 in `deviations.yaml`.
///
/// **`normalize` here is not Mosaic's density `normalize`.** That key sits on a
/// `density` mark and divides a kernel estimate by its own sum or maximum so a
/// curve integrates to one. This divides each stacked segment by its own bin's
/// total so a bar reads as a composition. Two features, one English word, and
/// the reason this one is spelled as an OFFSET rather than as a bare
/// `normalize:` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackOffset {
    /// Segments carry their own values and the stack reaches the group's total.
    /// The default, and the reading every spec written before this key had.
    #[default]
    None,
    /// Each segment is divided by its stack's total, so every occupied stack
    /// reaches the same height and the bar reads as shares.
    Normalize,
}

impl StackOffset {
    /// The wire name this build reads back, matched exactly for the reason
    /// [`ScaleType::from_wire`] is.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Normalize => "normalize",
        }
    }

    /// Read a wire name, or `None` for a spelling this build does not know.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "normalize" => Some(Self::Normalize),
            _ => None,
        }
    }
}

/// The plot attribute [`resolve_plot_stack_offset`] reads — **the consumed
/// key**, read out of this constant at the one lookup, so a rename here is a
/// rename everywhere and `a_click_writes_the_stack_offset_into_the_canonical_spec`
/// reads back its exact wire name.
pub const STACK_OFFSET_KEY: &str = "stackOffset";

/// Resolve a plot's `stackOffset` attribute.
///
/// A pure spec reading, on the same standing as [`resolve_plot_scales`]: it
/// says what the author asked for and holds no opinion about what a lowerer
/// then does with it. A name outside the two `StackOffset::from_wire` knows
/// leaves the plot unnormalised, the same degradation an unknown scale name
/// takes: a word this build cannot draw is a reason to draw the default, not
/// a reason to blank the frame.
#[must_use]
pub fn resolve_plot_stack_offset(plot: &PlotNode) -> StackOffset {
    match plot.attributes.get(STACK_OFFSET_KEY) {
        Some(SpecValue::String(s)) => StackOffset::from_wire(s).unwrap_or_default(),
        _ => StackOffset::default(),
    }
}

/// The plot attribute naming each positional axis's scale type — **the
/// consumed list**.
///
/// [`resolve_plot_scales_in`] reads an axis's key OUT of this table rather
/// than writing the string at the lookup, so a key removed here is a key
/// nothing reads, and `a_plot_written_x_scale_log_resolves_log` says so.
const SCALE_KEYS: [(PlotAxis, &str); 2] = [(PlotAxis::X, "xScale"), (PlotAxis::Y, "yScale")];

/// The attribute key this build reads as `axis`'s scale type, or `None` when
/// that axis has no key on the consumed list.
#[must_use]
pub fn plot_scale_key(axis: PlotAxis) -> Option<&'static str> {
    SCALE_KEYS
        .iter()
        .find(|(a, _)| *a == axis)
        .map(|(_, key)| *key)
}

/// Resolve a plot's `xScale` / `yScale` attributes, reading a lifted `$param`
/// through its declared value.
///
/// A pure spec reading, mirroring [`resolve_fixed_domains`] and
/// [`resolve_plot_insets`]: it says what the author asked for and holds no
/// opinion about what a renderer or a lowerer then does with it.
///
/// Three values are read — `linear`, `log`, `symlog`, the set
/// [`ScaleType::from_wire`] holds. A name outside that set leaves the axis
/// linear, which `an_unknown_scale_name_degrades_to_linear` holds over a list
/// of near-misses; it is the same degradation an unreadable `xDomain` takes,
/// since a name this build cannot draw is not a reason to draw nothing.
///
/// A `$param` at the attribute position resolves through `params` — one hop,
/// not a chain, because a param whose value is another param reference is not
/// a form the spec language produces. An unresolvable reference leaves the
/// axis linear.
#[must_use]
pub fn resolve_plot_scales_in(
    plot: &PlotNode,
    params: &IndexMap<String, crate::ast::ParamNode>,
) -> PlotScales {
    let resolve = |axis: PlotAxis| -> ScaleType {
        let Some(key) = plot_scale_key(axis) else {
            return ScaleType::Linear;
        };
        let named = match plot.attributes.get(key) {
            Some(SpecValue::String(s)) => Some(s.as_str()),
            Some(SpecValue::Param(r)) => match params.get(&r.0) {
                Some(crate::ast::ParamNode::Value(SpecValue::String(s))) => Some(s.as_str()),
                _ => None,
            },
            _ => None,
        };
        named
            .and_then(ScaleType::from_wire)
            .unwrap_or(ScaleType::Linear)
    };
    PlotScales {
        x: resolve(PlotAxis::X),
        y: resolve(PlotAxis::Y),
    }
}

/// [`resolve_plot_scales_in`] with no params in scope — the literal-only
/// reading, which is what a render path that was handed a plot and not a spec
/// can answer.
#[must_use]
pub fn resolve_plot_scales(plot: &PlotNode) -> PlotScales {
    resolve_plot_scales_in(plot, &IndexMap::new())
}

/// The map projection a plot resolves to. Which projection is a PURE spec
/// decision (this resolver, reading `projectionType`); the forward MATH lives
/// render-side in `brightfield_render::mark::Projection`, converted from this.
///
/// The catalogue is **Mosaic's `ProjectionName` enum**, which is Observable
/// Plot's, which is d3-geo's — one list, so a name the spec language can ask
/// for and a name this build can draw are the same vocabulary. Sixteen names
/// are recognised, which the test
/// `every_mosaic_projection_name_resolves_to_its_own_variant` enumerates against
/// the schema; [`ResolvedProjection::from_wire`] is the single place that maps a
/// wire string to a variant.
///
/// **A projection is a PLOT attribute and nothing else**, which is Observable
/// Plot's rule and therefore Mosaic's: `projection` replaces the plot's x and y
/// scales, so every mark on the plot whose position channels are longitude and
/// latitude draws through it and no mark draws through a different one. The one
/// delivery is `brightfield_render::channel::ChannelMap::from_mark_in`, which
/// takes the plot the mark sits in.
///
/// Two stated fidelity gaps, both unchanged by the catalogue widening:
///
/// - `albers-usa`'s AK/HI composite insets are deferred — it maps to plain
///   [`ResolvedProjection::Albers`] (contiguous-US correct; AK/HI render in
///   true geographic position).
/// - `projectionRotate` / `projectionParallels` are not read, so a projection
///   here is drawn at d3's default rotation and, for the conics, d3's default
///   standard parallels. `albers` is the exception, because its rotation is
///   baked into the transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResolvedProjection {
    /// `u = lon`, `v = lat` (north-up supplied by the inverted Y scale). The
    /// default when `projectionType` is absent or unrecognised.
    #[default]
    Equirectangular,
    /// d3's `geoIdentity` — a planar passthrough. Under the renderer's
    /// aspect-preserving fit this draws the same picture as
    /// [`Self::Equirectangular`]; it is a separate variant because it is a
    /// separate name in the spec language, and because [`Self::ReflectY`] is
    /// its sibling.
    Identity,
    /// [`Self::Identity`] with the latitude axis flipped.
    ReflectY,
    /// Spherical Mercator — conformal, so local shape survives at each
    /// latitude. Undefined at the poles: beyond d3's clip latitude
    /// (±85.05113°) a coordinate has no position.
    Mercator,
    /// Transverse spherical Mercator at d3's default `rotate([0, 0, 90])`.
    TransverseMercator,
    /// Orthographic — the globe seen from infinitely far away. The far
    /// hemisphere has no position.
    Orthographic,
    /// Stereographic — conformal azimuthal. The antipode has no position.
    Stereographic,
    /// Gnomonic — great circles draw straight. Only the near hemisphere has a
    /// position, and it diverges towards the rim.
    Gnomonic,
    /// Lambert azimuthal equal-area.
    AzimuthalEqualArea,
    /// Azimuthal equidistant.
    AzimuthalEquidistant,
    /// Equal Earth (Šavrič, Patterson & Jenny, 2018) — an equal-area
    /// pseudocylindrical whole-world projection, defined at each latitude.
    EqualEarth,
    /// Albers conic equal-area at d3's default standard parallels (0°, 60°).
    ConicEqualArea,
    /// Lambert conic conformal at d3's default standard parallels (30°, 30°).
    ConicConformal,
    /// Conic equidistant at d3's default standard parallels (0°, 60°).
    ConicEquidistant,
    /// US-tuned Albers equal-area conic (fixed standard parallels 29.5°/45.5°).
    Albers,
}

impl ResolvedProjection {
    /// Recognise a `projectionType` wire value. `None` for a name outside
    /// Mosaic's vocabulary (the caller defaults + warns). `albers-usa` maps to
    /// plain `Albers` — the composite is deferred (a stated fidelity gap).
    ///
    /// This is the ONE reader of a projection name in the build: the plot
    /// attribute ([`resolve_projection`]) and the parser's
    /// [`crate::parse::ParseWarning::UnknownProjection`] check come through
    /// here, so a name the spec language accepts and a name the renderer draws
    /// cannot come apart. There is no mark-level projection key — Mosaic has no
    /// such key, and `projectionType` written on a mark reaches
    /// [`crate::parse::ParseWarning::UnconsumedMarkOption`], which
    /// `a_mark_level_projection_is_a_key_nothing_reads` holds.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        match name {
            "equirectangular" => Some(Self::Equirectangular),
            "identity" => Some(Self::Identity),
            "reflect-y" => Some(Self::ReflectY),
            "mercator" => Some(Self::Mercator),
            "transverse-mercator" => Some(Self::TransverseMercator),
            "orthographic" => Some(Self::Orthographic),
            "stereographic" => Some(Self::Stereographic),
            "gnomonic" => Some(Self::Gnomonic),
            "azimuthal-equal-area" => Some(Self::AzimuthalEqualArea),
            "azimuthal-equidistant" => Some(Self::AzimuthalEquidistant),
            "equal-earth" => Some(Self::EqualEarth),
            "conic-equal-area" => Some(Self::ConicEqualArea),
            "conic-conformal" => Some(Self::ConicConformal),
            "conic-equidistant" => Some(Self::ConicEquidistant),
            "albers" | "albers-usa" => Some(Self::Albers),
            _ => None,
        }
    }

    /// Whether a pixel on the x axis inverts to a longitude without knowing the
    /// y pixel, and a pixel on the y axis to a latitude without knowing the x
    /// pixel.
    ///
    /// True exactly for the four projections whose planar `u` is a function of
    /// longitude alone and whose `v` is a function of latitude alone —
    /// equirectangular, identity, reflect-y and Mercator —
    /// `four_of_mosaics_names_invert_per_axis` enumerates them against the
    /// catalogue. Elsewhere the two are entangled (a conic's `u` depends on the
    /// latitude, an azimuthal's on both), so a rectangle swept in pixels has no
    /// rectangle of longitudes and latitudes behind it and an `intervalX` /
    /// `intervalY` / `intervalXY` filter over it would name bounds the reader
    /// did not sweep.
    ///
    /// `brightfield_render::mark::Projection` implements the two inverses this
    /// predicate is a claim about; the test
    /// `separability_is_the_claim_the_inverses_keep` drives all sixteen names
    /// through both and fails if either side moves alone.
    #[must_use]
    pub fn axes_invert_separately(self) -> bool {
        matches!(
            self,
            Self::Equirectangular | Self::Identity | Self::ReflectY | Self::Mercator
        )
    }
}

/// The map projection a plot's `projectionType` attribute names, or `None` for a
/// plot that does not name one — a PURE resolver beside [`resolve_plot_insets`] /
/// [`resolve_axis_titles`], held by `resolve_projection_reads_projection_type`.
///
/// **The absence is meaningful and is why this returns an `Option`.** A plot
/// that names no projection is a cartesian plot: its `dot` marks draw a scatter
/// at raw column numbers and no graticule goes behind them. A plot that names
/// `equirectangular` is a map that happens to use the plate carrée, and it draws
/// a graticule. Collapsing the two — which is what returning a defaulted
/// `ResolvedProjection` did — makes a scatter indistinguishable from a world
/// map, which `an_unprojected_dot_mark_draws_no_graticule` is the contrast for.
///
/// A `$param` and an unrecognised name both read as absent — the last two cases
/// of `resolve_projection_reads_projection_type`. The unrecognised-value warning
/// is raised at PARSE time in `walk_plot` (like `NonNumericInset` /
/// `NonStringLabel`) rather than here.
#[must_use]
pub fn resolve_projection(plot: &PlotNode) -> Option<ResolvedProjection> {
    match plot.attributes.get("projectionType") {
        Some(SpecValue::String(s)) => ResolvedProjection::from_wire(s),
        _ => None,
    }
}

/// The default geometry column a geo mark reads — the name `ST_Read` (and a
/// spatial join) produces, and the name `GeoLowerer` (in `brightfield-sql`,
/// which depends on this crate, so it cannot be linked from here) wraps in
/// `ST_AsGeoJSON`.
pub const DEFAULT_GEOMETRY_COLUMN: &str = "geom";

/// Resolve a geo mark's geometry column from its `geometry:` channel, default
/// [`DEFAULT_GEOMETRY_COLUMN`] (`geom`). Literal-only, mirroring the other
/// resolvers: a `$param` or non-string value falls back to the default. The
/// lowerer reads this to decide which spatial column to wrap in `ST_AsGeoJSON`.
#[must_use]
pub fn resolve_geometry_column(mark: &Mark) -> String {
    match mark.options.get("geometry") {
        Some(ValueOrParamRef::Value(SpecValue::String(s))) if !s.is_empty() => s.clone(),
        _ => DEFAULT_GEOMETRY_COLUMN.to_string(),
    }
}

fn layout_plot(plot: &PlotNode, x: f64, y: f64, avail: Avail) -> LayoutNode {
    let w = avail.width.unwrap_or_else(|| plot_width(plot));
    let h = avail.height.unwrap_or_else(|| plot_height(plot));
    // Plot items (marks, interactors, legends) are positioned within the plot
    // and share its footprint, so the resolved box is what they are offered —
    // a bare mark inside a 300x200 plot is 300x200.
    let inner = Avail {
        width: Some(w),
        height: Some(h),
    };
    let children: Vec<LayoutNode> = plot
        .items
        .iter()
        .map(|item| layout_component(item, x, y, inner))
        .collect();
    LayoutNode::Plot {
        rect: Rect::new(x, y, w, h),
        children,
    }
}

fn layout_hconcat(concat: &ConcatNode, x: f64, y: f64, avail: Avail) -> LayoutNode {
    let measured: Vec<(f64, bool)> = concat
        .items
        .iter()
        .map(|item| (intrinsic_size(item).0, component_flexes(item)))
        .collect();
    let shares = distribute(avail.width, &measured);

    let mut children = Vec::with_capacity(concat.items.len());
    let mut cursor_x = x;
    let mut max_height: f64 = 0.0;

    for (item, share) in concat.items.iter().zip(shares) {
        let child = layout_component(
            item,
            cursor_x,
            y,
            Avail {
                width: share,
                height: component_flexes(item).then_some(avail.height).flatten(),
            },
        );
        let r = child.rect();
        cursor_x += r.width;
        max_height = max_height.max(r.height);
        children.push(child);
    }

    LayoutNode::HConcat {
        rect: Rect::new(
            x,
            y,
            avail.width.unwrap_or(cursor_x - x),
            avail.height.unwrap_or(max_height),
        ),
        children,
    }
}

/// The plot a colour legend sits under in a `vconcat`: the index of the nearest
/// earlier sibling plot whose `name:` the legend at `index` names by `for:`, or
/// `None` when the item at `index` is not such a legend.
///
/// The one place the arrangement is decided. A legend that names a plot it is
/// not beside in this `vconcat`, one that names none, one whose `for:` is a
/// `$param`, one that is not a colour legend, and one that comes before the plot
/// it names are all not under a plot, and keep the standalone legend's own rect.
fn plot_above_legend(concat: &ConcatNode, index: usize) -> Option<usize> {
    let Component::Legend(legend) = concat.items.get(index)? else {
        return None;
    };
    if legend.channel != LegendChannel::Color {
        return None;
    }
    let Some(ValueOrParamRef::Value(SpecValue::String(named))) = legend.options.get("for") else {
        return None;
    };
    concat.items[..index].iter().rposition(|item| {
        matches!(
            item,
            Component::Plot(plot)
                if matches!(plot.attributes.get("name"), Some(SpecValue::String(n)) if n == named)
        )
    })
}

fn layout_vconcat(concat: &ConcatNode, x: f64, y: f64, avail: Avail) -> LayoutNode {
    let under: Vec<Option<usize>> = (0..concat.items.len())
        .map(|i| plot_above_legend(concat, i))
        .collect();
    let measured: Vec<(f64, bool)> = concat
        .items
        .iter()
        .zip(&under)
        .map(|(item, under)| match under {
            Some(_) => (BELOW_LEGEND_HEIGHT, false),
            None => (intrinsic_size(item).1, component_flexes(item)),
        })
        .collect();
    let shares = distribute(avail.height, &measured);

    let mut children: Vec<LayoutNode> = Vec::with_capacity(concat.items.len());
    let mut cursor_y = y;
    let mut max_width: f64 = 0.0;

    for ((item, share), under) in concat.items.iter().zip(shares).zip(under) {
        let child = match under {
            // The band under a plot: as wide as the plot above it, and
            // [`BELOW_LEGEND_HEIGHT`] high whatever the column is offered.
            Some(plot) => LayoutNode::Legend {
                rect: Rect::new(
                    x,
                    cursor_y,
                    children[plot].rect().width,
                    BELOW_LEGEND_HEIGHT,
                ),
            },
            None => layout_component(
                item,
                x,
                cursor_y,
                Avail {
                    width: component_flexes(item).then_some(avail.width).flatten(),
                    height: share,
                },
            ),
        };
        let r = child.rect();
        cursor_y += r.height;
        max_width = max_width.max(r.width);
        children.push(child);
    }

    LayoutNode::VConcat {
        rect: Rect::new(
            x,
            y,
            avail.width.unwrap_or(max_width),
            avail.height.unwrap_or(cursor_y - y),
        ),
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
// Input placement (widgets)
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
/// the private `collect_placed_plots`'s stop-at-plot behaviour).
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
/// the private `collect_placed_inputs` (which walks the layout tree). Because
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
/// `SliderBinding` (in `brightfield-ui`, which depends on this crate, so it
/// cannot be linked from here) from each `&Input` and host a widget at its
/// `rect`.
#[must_use]
pub fn placed_input_nodes(spec: &Spec, viewport: Rect) -> Vec<(Rect, &Input)> {
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
/// legend joins to its [`LegendNode`](crate::ast::LegendNode) via
/// [`collect_legend_nodes`]. See [`placed_legend_nodes`] for the joined view an
/// app hosts.
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
/// [`Component`] tree with the same path scheme as the private
/// `collect_placed_legends`.
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
pub fn placed_legend_nodes(spec: &Spec, viewport: Rect) -> Vec<(Rect, &crate::ast::LegendNode)> {
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

/// A standalone colour legend placed in the band under the plot it is for: the
/// plot, the legend and the band's rect.
#[derive(Debug, Clone, PartialEq)]
pub struct BelowLegend {
    /// Component path of the plot the legend is for — the join key
    /// [`PlacedPlot::path`] carries.
    pub plot_path: String,
    /// Component path of the legend node.
    pub legend_path: String,
    /// The band, on the same plane as [`PlacedPlot::rect`]: [`BELOW_LEGEND_HEIGHT`]
    /// high, as wide as the plot, directly under it.
    pub rect: Rect,
}

/// The standalone colour legends of a spec that sit under the plot they are
/// for: a legend in a `vconcat` after a sibling plot whose `name:` its `for:`
/// names. A standalone legend placed any other way — in an `hconcat`, for a plot
/// that is not its sibling, with no `for:` — is not here, and the shell draws the
/// plot's legend at its right.
///
/// The rect is [`placed_legends`]'s, so it is the one the layout reserved: the
/// plot above it was laid out in the height that was left.
#[must_use]
pub fn below_legends(spec: &Spec, viewport: Rect) -> Vec<BelowLegend> {
    let placed = placed_legends(spec, viewport);
    let mut out = Vec::new();
    if let Some(root) = &spec.root {
        collect_below_legends(root, "root", &placed, &mut out);
    }
    out
}

fn collect_below_legends(
    component: &Component,
    path: &str,
    placed: &[PlacedLegend],
    out: &mut Vec<BelowLegend>,
) {
    match component {
        Component::HConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                collect_below_legends(item, &format!("{path}/hconcat[{i}]"), placed, out);
            }
        }
        Component::VConcat(concat) => {
            for (i, item) in concat.items.iter().enumerate() {
                let item_path = format!("{path}/vconcat[{i}]");
                match plot_above_legend(concat, i) {
                    Some(plot) => {
                        if let Some(legend) = placed.iter().find(|p| p.path == item_path) {
                            out.push(BelowLegend {
                                plot_path: format!("{path}/vconcat[{plot}]"),
                                legend_path: item_path,
                                rect: legend.rect,
                            });
                        }
                    }
                    None => collect_below_legends(item, &item_path, placed, out),
                }
            }
        }
        _ => {}
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

    /// A viewport that offers nothing on either axis: every assertion made
    /// under it is about the spec's own intrinsic size.
    const UNCONSTRAINED: Rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

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

    // placed_inputs surfaces a composition-level input's
    // rect + path (the rect the multi-view extraction used to drop).
    #[test]
    fn placed_inputs_path_and_rect() {
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

    // collect_input_nodes paths match placed_inputs, and
    // placed_input_nodes joins each rect to the Input that writes the param.
    #[test]
    fn collect_input_nodes_and_join() {
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

    // The vspace twin of the above, and the evidence behind
    // `ComponentKind::VSpace`'s status: a `vspace:` between two stacked plots
    // pushes the lower plot down through the SAME `placed_plots` call the
    // window and the composite both take. `sorted-bars.yaml` (vendored,
    // unmodified) declares exactly this shape.
    #[test]
    fn vspace_offsets_subsequent_plot() {
        let yaml = r#"
data:
  t:
    - { x: 1, y: 2 }
vconcat:
  - plot:
      - { mark: dot, data: { from: t }, x: x, y: y }
    width: 300
    height: 200
  - vspace: 40
  - plot:
      - { mark: line, data: { from: t }, x: x, y: y }
    width: 300
    height: 200
"#;
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
        let plots = placed_plots(&parsed.spec, Rect::zero());
        assert_eq!(plots.len(), 2, "spacers are not plots");
        assert_eq!(plots[0].rect.y, 0.0);
        // Second plot pushed down by plot-0 height (200) + the 40px spacer.
        assert_eq!(
            plots[1].rect.y, 240.0,
            "the 40px vspace offsets the lower plot"
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

    // Rect struct
    #[test]
    fn rect_fields() {
        let r = Rect::new(10.0, 20.0, 300.0, 200.0);
        assert_eq!(r.x, 10.0);
        assert_eq!(r.y, 20.0);
        assert_eq!(r.width, 300.0);
        assert_eq!(r.height, 200.0);
    }

    #[test]
    fn rect_zero() {
        let r = Rect::zero();
        assert_eq!(r.x, 0.0);
        assert_eq!(r.y, 0.0);
        assert_eq!(r.width, 0.0);
        assert_eq!(r.height, 0.0);
    }

    // LayoutNode enum is exhaustive over Component variants
    #[test]
    fn layout_node_exhaustive_match() {
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
    fn layout_node_rect_accessor() {
        let node = LayoutNode::Legend {
            rect: Rect::new(5.0, 10.0, 120.0, 24.0),
        };
        assert_eq!(node.rect().x, 5.0);
        assert_eq!(node.rect().width, 120.0);
    }

    // compute_layout basic
    #[test]
    fn single_plot() {
        let spec = Spec {
            root: Some(Component::Plot(PlotNode {
                items: vec![],
                attributes: IndexMap::new(),
            })),
            ..Default::default()
        };
        let tree = compute_layout(&spec, UNCONSTRAINED);
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
    fn no_root() {
        let spec = Spec::default();
        let viewport = Rect::new(0.0, 0.0, 800.0, 600.0);
        let tree = compute_layout(&spec, viewport);
        assert!(tree.is_none());
    }

    // hconcat stacks left-to-right
    #[test]
    fn hconcat_two_plots() {
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
        let tree = compute_layout(&spec, UNCONSTRAINED).unwrap();
        if let LayoutNode::HConcat { children, rect, .. } = &tree {
            assert_eq!(children.len(), 2);
            assert_eq!(children[0].rect().x, 0.0);
            assert_eq!(children[1].rect().x, DEFAULT_PLOT_WIDTH);
            assert_eq!(rect.width, DEFAULT_PLOT_WIDTH * 2.0);
        } else {
            panic!("expected HConcat");
        }
    }

    // vconcat stacks top-to-bottom
    #[test]
    fn vconcat_two_plots() {
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
        let tree = compute_layout(&spec, UNCONSTRAINED).unwrap();
        if let LayoutNode::VConcat { children, rect, .. } = &tree {
            assert_eq!(children.len(), 2);
            assert_eq!(children[0].rect().y, 0.0);
            assert_eq!(children[1].rect().y, DEFAULT_PLOT_HEIGHT);
            assert_eq!(rect.height, DEFAULT_PLOT_HEIGHT * 2.0);
        } else {
            panic!("expected VConcat");
        }
    }

    // hspace and vspace gaps
    #[test]
    fn hspace_gap() {
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
        let tree = compute_layout(&spec, UNCONSTRAINED).unwrap();
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
    fn vspace_gap() {
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
        let tree = compute_layout(&spec, UNCONSTRAINED).unwrap();
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

    // resolve_space_value
    #[test]
    fn numeric_pixels() {
        assert_eq!(resolve_space_value(&SpecValue::Integer(35), 16.0), 35.0);
        assert_eq!(resolve_space_value(&SpecValue::Float(2.5), 16.0), 2.5);
    }

    #[test]
    fn em_units() {
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
    fn invalid_returns_zero() {
        assert_eq!(
            resolve_space_value(&SpecValue::String("bogus".to_string()), 16.0),
            0.0
        );
        assert_eq!(resolve_space_value(&SpecValue::Null, 16.0), 0.0);
    }

    // nested composition (grid)
    #[test]
    fn nested_grid() {
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
        let tree = compute_layout(&spec, UNCONSTRAINED).unwrap();
        if let LayoutNode::HConcat { children, .. } = &tree {
            // First column (vconcat)
            if let LayoutNode::VConcat {
                children: col1,
                rect: col1_rect,
                ..
            } = &children[0]
            {
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

    // mixed component types
    #[test]
    fn mixed_types() {
        use crate::vocab::{ImplStatus, InputKind, LegendChannel};
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
        let tree = compute_layout(&spec, UNCONSTRAINED).unwrap();
        if let LayoutNode::HConcat { children, .. } = &tree {
            assert_eq!(children.len(), 3);
            // Plot at x=0
            assert_eq!(children[0].rect().x, 0.0);
            assert!(children[0].rect().width > 0.0);
            // Input at x=plot_width
            assert_eq!(children[1].rect().x, DEFAULT_PLOT_WIDTH);
            assert!(children[1].rect().width > 0.0);
            // Legend at x=plot_width+input_width
            assert_eq!(
                children[2].rect().x,
                DEFAULT_PLOT_WIDTH + DEFAULT_INPUT_WIDTH
            );
            assert!(children[2].rect().width > 0.0);
        } else {
            panic!("expected HConcat");
        }
    }

    // plot attributes override defaults
    #[test]
    fn plot_declared_size() {
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
        let tree = compute_layout(&spec, UNCONSTRAINED).unwrap();
        assert_eq!(tree.rect().width, 500.0);
        assert_eq!(tree.rect().height, 200.0);
    }

    #[test]
    fn plot_partial_override() {
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
        let tree = compute_layout(&spec, UNCONSTRAINED).unwrap();
        assert_eq!(tree.rect().width, DEFAULT_PLOT_WIDTH);
        assert_eq!(tree.rect().height, 200.0);
    }

    // --- constrained layout: the rect the caller hands in wins ---

    fn bare_plot() -> Component {
        Component::Plot(PlotNode {
            items: vec![],
            attributes: IndexMap::new(),
        })
    }

    fn sized_plot(width: i64, height: i64) -> Component {
        let mut attrs = IndexMap::new();
        attrs.insert("width".to_string(), SpecValue::Integer(width));
        attrs.insert("height".to_string(), SpecValue::Integer(height));
        Component::Plot(PlotNode {
            items: vec![],
            attributes: attrs,
        })
    }

    fn spec_of(root: Component) -> Spec {
        Spec {
            root: Some(root),
            ..Default::default()
        }
    }

    #[test]
    fn a_constrained_plot_takes_the_offered_box_over_its_declared_size() {
        let spec = spec_of(sized_plot(500, 200));

        let smaller = compute_layout(&spec, Rect::new(0.0, 0.0, 320.0, 180.0)).unwrap();
        assert_eq!(*smaller.rect(), Rect::new(0.0, 0.0, 320.0, 180.0));

        let larger = compute_layout(&spec, Rect::new(0.0, 0.0, 900.0, 700.0)).unwrap();
        assert_eq!(*larger.rect(), Rect::new(0.0, 0.0, 900.0, 700.0));
    }

    #[test]
    fn a_plot_with_no_declared_size_takes_the_offered_box() {
        let spec = spec_of(bare_plot());
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 320.0, 180.0)).unwrap();
        assert_eq!(*tree.rect(), Rect::new(0.0, 0.0, 320.0, 180.0));
    }

    #[test]
    fn each_axis_is_constrained_on_its_own() {
        let spec = spec_of(sized_plot(500, 200));

        let width_only = compute_layout(&spec, Rect::new(0.0, 0.0, 900.0, 0.0)).unwrap();
        assert_eq!(width_only.rect().width, 900.0);
        assert_eq!(width_only.rect().height, 200.0);

        let height_only = compute_layout(&spec, Rect::new(0.0, 0.0, 0.0, 700.0)).unwrap();
        assert_eq!(height_only.rect().width, 500.0);
        assert_eq!(height_only.rect().height, 700.0);
    }

    #[test]
    fn a_constrained_hconcat_splits_by_intrinsic_weight() {
        let spec = spec_of(Component::HConcat(ConcatNode {
            items: vec![sized_plot(300, 200), sized_plot(600, 200)],
        }));

        // 1:2 at the declared sizes, so 900 splits 300/600 and 450 splits
        // 150/300.
        for (offer, left, right) in [(900.0, 300.0, 600.0), (450.0, 150.0, 300.0)] {
            let tree = compute_layout(&spec, Rect::new(0.0, 0.0, offer, 400.0)).unwrap();
            let LayoutNode::HConcat { children, rect } = &tree else {
                panic!("expected HConcat");
            };
            assert_eq!(rect.width, offer);
            assert_eq!(children[0].rect().width, left);
            assert_eq!(children[1].rect().width, right);
            // Tiled with no seam: the second starts where the first ends, and
            // the pair covers the offer exactly.
            assert_eq!(children[1].rect().x, children[0].rect().width);
            assert_eq!(
                children[1].rect().x + children[1].rect().width,
                offer,
                "children tile the offered width exactly"
            );
            // Stretched on the cross axis rather than left at 200.
            assert_eq!(children[0].rect().height, 400.0);
            assert_eq!(children[1].rect().height, 400.0);
        }
    }

    #[test]
    fn a_spacer_keeps_its_gap_out_of_the_residual() {
        let spec = spec_of(Component::HConcat(ConcatNode {
            items: vec![
                bare_plot(),
                Component::HSpace(SpaceNode {
                    value: SpecValue::Integer(64),
                }),
                bare_plot(),
            ],
        }));

        // 1064 - 64 = 1000, halved; and 264 - 64 = 200, halved.
        for (offer, plot_w) in [(1064.0, 500.0), (264.0, 100.0)] {
            let tree = compute_layout(&spec, Rect::new(0.0, 0.0, offer, 400.0)).unwrap();
            let LayoutNode::HConcat { children, .. } = &tree else {
                panic!("expected HConcat");
            };
            assert_eq!(children[0].rect().width, plot_w);
            assert_eq!(children[1].rect().width, 64.0, "the gap is not shared out");
            assert_eq!(children[2].rect().width, plot_w);
            assert_eq!(children[2].rect().x + children[2].rect().width, offer);
        }
    }

    #[test]
    fn an_offer_smaller_than_the_fixed_children_clamps_at_zero() {
        let spec = spec_of(Component::HConcat(ConcatNode {
            items: vec![
                bare_plot(),
                Component::HSpace(SpaceNode {
                    value: SpecValue::Integer(64),
                }),
                bare_plot(),
            ],
        }));
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 40.0, 400.0)).unwrap();
        let LayoutNode::HConcat { children, .. } = &tree else {
            panic!("expected HConcat");
        };
        assert_eq!(children[0].rect().width, 0.0);
        assert_eq!(children[2].rect().width, 0.0);
    }

    #[test]
    fn a_constrained_vconcat_reserves_its_inputs_height() {
        let parsed = parse_spec(SLIDER_SPEC, Format::Yaml).expect("parse");
        // vconcat > hconcat > [plot 300x200, slider 200x32].
        let tree = compute_layout(&parsed.spec, Rect::new(0.0, 0.0, 800.0, 500.0)).unwrap();
        let LayoutNode::VConcat { children, .. } = &tree else {
            panic!("expected VConcat");
        };
        let LayoutNode::HConcat { children: row, .. } = &children[0] else {
            panic!("expected HConcat");
        };
        assert_eq!(row[1].rect().width, DEFAULT_INPUT_WIDTH);
        assert_eq!(row[1].rect().height, DEFAULT_INPUT_HEIGHT);
        assert_eq!(row[0].rect().width, 800.0 - DEFAULT_INPUT_WIDTH);
        assert_eq!(row[0].rect().height, 500.0);

        // The joined view the menu resolver and the rail read is placed to
        // match.
        let inputs = placed_inputs(&parsed.spec, Rect::new(0.0, 0.0, 800.0, 500.0));
        assert_eq!(inputs.len(), 1);
        assert_eq!(
            inputs[0].rect,
            Rect::new(800.0 - DEFAULT_INPUT_WIDTH, 0.0, 200.0, 32.0)
        );
    }

    #[test]
    fn a_plots_items_are_offered_the_plots_box() {
        let spec = spec_of(Component::Plot(PlotNode {
            items: vec![Component::Mark(Mark {
                kind: crate::vocab::MarkKind::Dot,
                status: crate::vocab::ImplStatus::Implemented,
                data: None,
                options: IndexMap::new(),
            })],
            attributes: IndexMap::new(),
        }));
        let tree = compute_layout(&spec, Rect::new(0.0, 0.0, 320.0, 180.0)).unwrap();
        let LayoutNode::Plot { children, rect } = &tree else {
            panic!("expected Plot");
        };
        assert_eq!(*rect, Rect::new(0.0, 0.0, 320.0, 180.0));
        assert_eq!(*children[0].rect(), Rect::new(0.0, 0.0, 320.0, 180.0));
    }

    #[test]
    fn a_constrained_grid_tiles_the_offered_box() {
        let spec = spec_of(Component::HConcat(ConcatNode {
            items: vec![
                Component::VConcat(ConcatNode {
                    items: vec![bare_plot(), bare_plot()],
                }),
                Component::VConcat(ConcatNode {
                    items: vec![bare_plot(), bare_plot()],
                }),
            ],
        }));
        let plots = placed_plots(&spec, Rect::new(0.0, 0.0, 1000.0, 600.0));
        let rects: Vec<Rect> = plots.iter().map(|p| p.rect).collect();
        assert_eq!(
            rects,
            vec![
                Rect::new(0.0, 0.0, 500.0, 300.0),
                Rect::new(0.0, 300.0, 500.0, 300.0),
                Rect::new(500.0, 0.0, 500.0, 300.0),
                Rect::new(500.0, 300.0, 500.0, 300.0),
            ]
        );
    }

    #[test]
    fn the_origin_still_offsets_a_constrained_root() {
        let spec = spec_of(sized_plot(500, 200));
        let tree = compute_layout(&spec, Rect::new(12.0, 34.0, 320.0, 180.0)).unwrap();
        assert_eq!(*tree.rect(), Rect::new(12.0, 34.0, 320.0, 180.0));
    }

    // REVISED: `as:` on a legend is a selection
    // PRODUCER binding (clicking a swatch WRITES the selection), so the
    // corpus legends with `as: $toggle` / `as: $interval` must NOT appear in
    // the subscriber graph. The original assertion pinned the backwards wiring
    // (legend-as-subscriber); the fixed analysis arm skips the `as:` key and
    // surfaces the binding via `legend_bindings` instead.
    #[test]
    fn legend_subscriber_graph() {
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

    // vendored corpus specs with composition
    #[test]
    fn corpus_layout() {
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
                assert!(
                    tree.is_some(),
                    "{}: spec has root but layout returned None",
                    path.display()
                );
            }
            tested += 1;
        }
        assert!(tested > 0, "no composition specs found in corpus");
    }

    // --- plot inset attribute resolution (most-specific-wins) ---

    fn plot_with(attrs: &[(&str, SpecValue)]) -> PlotNode {
        let mut attributes = IndexMap::new();
        for (k, v) in attrs {
            attributes.insert((*k).to_string(), v.clone());
        }
        PlotNode {
            items: vec![],
            attributes,
        }
    }

    #[test]
    fn global_inset_sets_all_four_sides() {
        let p = plot_with(&[("inset", SpecValue::Integer(5))]);
        assert_eq!(
            resolve_plot_insets(&p),
            SideInsets {
                left: Some(5.0),
                right: Some(5.0),
                top: Some(5.0),
                bottom: Some(5.0),
            }
        );
    }

    #[test]
    fn per_axis_overrides_global() {
        // xInset governs left+right; yInset governs top+bottom; global fills gaps.
        let p = plot_with(&[
            ("inset", SpecValue::Integer(2)),
            ("xInset", SpecValue::Float(8.0)),
        ]);
        assert_eq!(
            resolve_plot_insets(&p),
            SideInsets {
                left: Some(8.0),
                right: Some(8.0),
                top: Some(2.0),
                bottom: Some(2.0),
            }
        );
    }

    #[test]
    fn per_side_is_most_specific() {
        // Per-side beats per-axis beats global, independently on each side.
        let p = plot_with(&[
            ("inset", SpecValue::Integer(1)),
            ("xInset", SpecValue::Integer(4)),
            ("xInsetLeft", SpecValue::Integer(10)),
            ("yInsetTop", SpecValue::Integer(7)),
        ]);
        assert_eq!(
            resolve_plot_insets(&p),
            SideInsets {
                left: Some(10.0),  // xInsetLeft
                right: Some(4.0),  // xInset
                top: Some(7.0),    // yInsetTop
                bottom: Some(1.0), // global inset
            }
        );
    }

    #[test]
    fn explicit_zero_is_preserved_not_dropped() {
        // Explicit 0 is Some(0.0) — the Mosaic-exact opt-out — not "absent".
        let p = plot_with(&[("xInsetRight", SpecValue::Integer(0))]);
        let got = resolve_plot_insets(&p);
        assert_eq!(got.right, Some(0.0));
        assert_eq!(
            got.left, None,
            "unspecified side stays absent (default applies)"
        );
    }

    #[test]
    fn absent_is_none_and_nonnumeric_falls_through() {
        // No inset attrs at all → all None. A non-numeric per-side value degrades
        // to absent for that key and falls through to the next-most-specific.
        assert_eq!(resolve_plot_insets(&plot_with(&[])), SideInsets::default());
        let p = plot_with(&[
            ("xInset", SpecValue::Integer(6)),
            ("xInsetLeft", SpecValue::String("nope".into())),
        ]);
        // xInsetLeft is non-numeric → falls through to xInset(6).
        assert_eq!(resolve_plot_insets(&p).left, Some(6.0));
    }

    // --- plot margin attribute resolution (most-specific-wins) ---

    #[test]
    fn each_margin_key_reaches_its_own_side_and_only_that_side() {
        // Four distinct values, so a swapped pair of sides cannot pass.
        let p = plot_with(&[
            ("marginTop", SpecValue::Integer(1)),
            ("marginRight", SpecValue::Integer(2)),
            ("marginBottom", SpecValue::Integer(3)),
            ("marginLeft", SpecValue::Float(4.5)),
        ]);
        assert_eq!(
            resolve_plot_margins(&p),
            SideMargins {
                top: Some(1.0),
                right: Some(2.0),
                bottom: Some(3.0),
                left: Some(4.5),
            }
        );
        // One declared side leaves the other three absent, so the caller's
        // default still applies to them.
        let only_left = resolve_plot_margins(&plot_with(&[("marginLeft", SpecValue::Integer(0))]));
        assert_eq!(
            only_left.left,
            Some(0.0),
            "an explicit 0 is a value, not absent"
        );
        assert_eq!(
            (only_left.top, only_left.right, only_left.bottom),
            (None, None, None)
        );
    }

    #[test]
    fn the_margin_shorthand_sets_every_side_and_a_side_key_overrides_it() {
        let p = plot_with(&[
            ("margin", SpecValue::Integer(7)),
            ("marginBottom", SpecValue::Integer(9)),
        ]);
        assert_eq!(
            resolve_plot_margins(&p),
            SideMargins {
                top: Some(7.0),
                right: Some(7.0),
                bottom: Some(9.0),
                left: Some(7.0),
            }
        );
    }

    #[test]
    fn a_margin_that_is_not_a_usable_number_is_not_a_declared_margin() {
        // No margin attributes at all → nothing declared.
        assert_eq!(
            resolve_plot_margins(&plot_with(&[])),
            SideMargins::default()
        );
        // A non-numeric side key falls through to the shorthand, as insets do.
        let p = plot_with(&[
            ("margin", SpecValue::Integer(6)),
            ("marginLeft", SpecValue::String("wide".into())),
        ]);
        assert_eq!(resolve_plot_margins(&p).left, Some(6.0));
        // A negative number reads as no margin; a non-finite one is absent.
        let p = plot_with(&[
            ("marginLeft", SpecValue::Integer(-12)),
            ("marginRight", SpecValue::Float(f64::INFINITY)),
            ("marginTop", SpecValue::Float(f64::NAN)),
        ]);
        let got = resolve_plot_margins(&p);
        assert_eq!(got.left, Some(0.0));
        assert_eq!((got.right, got.top), (None, None));
    }

    #[test]
    fn resolve_axis_titles_override_suppress_derive() {
        // Override: a non-empty string is used verbatim.
        let p = plot_with(&[
            ("xLabel", SpecValue::String("Arrival Delay".into())),
            ("yLabel", SpecValue::Null),
        ]);
        let t = resolve_axis_titles(&p);
        assert_eq!(t.x, AxisTitle::Override("Arrival Delay".into()));
        assert_eq!(t.y, AxisTitle::Suppress, "explicit null suppresses");

        // Empty string also suppresses (the card's wording; corpus uses null).
        assert_eq!(
            resolve_axis_titles(&plot_with(&[("xLabel", SpecValue::String(String::new()))])).x,
            AxisTitle::Suppress,
        );

        // Absent → Derive; a $param → Derive (recorded deferral, treated absent);
        // a number/boolean → Derive (the warning is a parse-time concern).
        assert_eq!(resolve_axis_titles(&plot_with(&[])).x, AxisTitle::Derive);
        assert_eq!(
            resolve_axis_titles(&plot_with(&[("yLabel", SpecValue::Integer(42))])).y,
            AxisTitle::Derive,
            "a non-string label degrades to Derive here (no warning in the resolver)",
        );
        assert_eq!(
            resolve_axis_titles(&plot_with(&[(
                "xLabel",
                SpecValue::Param(crate::ast::ParamRef::new("p"))
            )]))
            .x,
            AxisTitle::Derive,
        );
    }

    #[test]
    fn resolve_projection_reads_projection_type() {
        let named = |name: &str| {
            resolve_projection(&plot_with(&[(
                "projectionType",
                SpecValue::String(name.into()),
            )]))
        };
        // Absent → the plot names NO projection. Not "equirectangular": a plot
        // that names nothing is a cartesian plot, and its dot marks draw a
        // scatter with no graticule behind it.
        assert_eq!(resolve_projection(&plot_with(&[])), None);
        // Recognised names.
        assert_eq!(
            named("equirectangular"),
            Some(ResolvedProjection::Equirectangular)
        );
        assert_eq!(named("albers"), Some(ResolvedProjection::Albers));
        // albers-usa → plain Albers (composite deferred, a stated gap).
        assert_eq!(named("albers-usa"), Some(ResolvedProjection::Albers));
        // The rest of Mosaic's names resolve through the same attribute — the
        // catalogue widened without a second mechanism beside `resolve_projection`.
        assert_eq!(named("mercator"), Some(ResolvedProjection::Mercator));
        assert_eq!(
            named("orthographic"),
            Some(ResolvedProjection::Orthographic)
        );
        // A name outside Mosaic's vocabulary, and a non-string → no projection
        // (no panic; the warning is parse-time).
        assert_eq!(named("mollweide"), None);
        assert_eq!(
            resolve_projection(&plot_with(&[("projectionType", SpecValue::Integer(3))])),
            None
        );
    }

    /// **The separability claim, over Mosaic's sixteen names.**
    /// `ResolvedProjection::axes_invert_separately` is a spec-side
    /// assertion about render-side behaviour: `build_brushable_bindings` reads it
    /// to decide whether an interval brush is installed, and
    /// `brightfield-shell`'s `axis_interval` then relies on the inverses
    /// existing. Two ways for that to be wrong, and this test rules out both — a
    /// projection declared separable whose inverses are missing is a brush that
    /// silently stops filtering, and one declared curved whose inverses exist is
    /// a brush refused for nothing.
    ///
    /// The render-side half lives in `brightfield-render`'s
    /// `separability_is_the_claim_the_inverses_keep`, which drives the same
    /// sixteen names through `Projection::invert_lon` / `invert_lat`; this half
    /// pins WHICH names the claim covers, so the list cannot quietly widen.
    #[test]
    fn four_of_mosaics_names_invert_per_axis() {
        let mut separable: Vec<&str> = MOSAIC_PROJECTION_NAMES
            .iter()
            .map(|(n, _)| *n)
            .filter(|n| {
                ResolvedProjection::from_wire(n)
                    .expect("a Mosaic name resolves")
                    .axes_invert_separately()
            })
            .collect();
        separable.sort_unstable();
        assert_eq!(
            separable,
            vec!["equirectangular", "identity", "mercator", "reflect-y"],
            "the separable set is the four whose u depends on the longitude \
             alone and whose v depends on the latitude alone"
        );
    }

    /// Mosaic's `ProjectionName` enum, verbatim from its published JSON schema
    /// (`idl.uw.edu/mosaic/schema/latest.json`, `definitions/ProjectionName`).
    /// Sixteen names; ONE enumeration, so a test about the vocabulary and a test
    /// about what the vocabulary can do cannot be about different lists.
    const MOSAIC_PROJECTION_NAMES: [(&str, ResolvedProjection); 16] = [
        ("albers-usa", ResolvedProjection::Albers),
        ("albers", ResolvedProjection::Albers),
        (
            "azimuthal-equal-area",
            ResolvedProjection::AzimuthalEqualArea,
        ),
        (
            "azimuthal-equidistant",
            ResolvedProjection::AzimuthalEquidistant,
        ),
        ("conic-conformal", ResolvedProjection::ConicConformal),
        ("conic-equal-area", ResolvedProjection::ConicEqualArea),
        ("conic-equidistant", ResolvedProjection::ConicEquidistant),
        ("equal-earth", ResolvedProjection::EqualEarth),
        ("equirectangular", ResolvedProjection::Equirectangular),
        ("gnomonic", ResolvedProjection::Gnomonic),
        ("identity", ResolvedProjection::Identity),
        ("reflect-y", ResolvedProjection::ReflectY),
        ("mercator", ResolvedProjection::Mercator),
        ("orthographic", ResolvedProjection::Orthographic),
        ("stereographic", ResolvedProjection::Stereographic),
        (
            "transverse-mercator",
            ResolvedProjection::TransverseMercator,
        ),
    ];

    /// Each of Mosaic's sixteen names resolves, and to a DISTINCT variant apart
    /// from the one pair that is deliberately shared: `albers`/`albers-usa`,
    /// whose composite insets are deferred.
    #[test]
    fn every_mosaic_projection_name_resolves_to_its_own_variant() {
        let names = MOSAIC_PROJECTION_NAMES;
        for (wire, expected) in names {
            assert_eq!(
                ResolvedProjection::from_wire(wire),
                Some(expected),
                "`{wire}` must resolve to {expected:?}"
            );
        }
        // Distinctness, so a widening that mapped several names onto one variant
        // could not pass the loop above: sixteen names, fifteen variants, and the
        // one collision is the deferred composite.
        let mut variants: Vec<ResolvedProjection> = names.iter().map(|(_, v)| *v).collect();
        variants.sort_by_key(|v| format!("{v:?}"));
        variants.dedup();
        assert_eq!(
            variants.len(),
            names.len() - 1,
            "only `albers`/`albers-usa` may share a variant; got {variants:?}"
        );
    }

    #[test]
    fn resolve_geometry_column_defaults_to_geom() {
        use crate::ast::{Mark, ValueOrParamRef};
        use crate::vocab::{ImplStatus, MarkKind};

        let mark_with_geometry = |geom: Option<SpecValue>| {
            let mut options: IndexMap<String, ValueOrParamRef<SpecValue>> = IndexMap::new();
            if let Some(v) = geom {
                options.insert("geometry".to_string(), ValueOrParamRef::Value(v));
            }
            Mark {
                kind: MarkKind::Geo,
                status: ImplStatus::Implemented,
                data: None,
                options,
            }
        };
        // Absent → default "geom".
        assert_eq!(resolve_geometry_column(&mark_with_geometry(None)), "geom");
        // Explicit column name.
        assert_eq!(
            resolve_geometry_column(&mark_with_geometry(Some(SpecValue::String("shape".into())))),
            "shape"
        );
        // Empty / non-string → default.
        assert_eq!(
            resolve_geometry_column(&mark_with_geometry(Some(SpecValue::String(String::new())))),
            "geom"
        );
    }

    #[test]
    fn resolve_plot_title_string_only() {
        assert_eq!(
            resolve_plot_title(&plot_with(&[(
                "title",
                SpecValue::String("Weather".into())
            )])),
            Some("Weather".to_string()),
        );
        // Absent, empty, null, and non-string all → None (no title, no band).
        assert_eq!(resolve_plot_title(&plot_with(&[])), None);
        assert_eq!(
            resolve_plot_title(&plot_with(&[("title", SpecValue::String(String::new()))])),
            None,
        );
        assert_eq!(
            resolve_plot_title(&plot_with(&[("title", SpecValue::Null)])),
            None
        );
        assert_eq!(
            resolve_plot_title(&plot_with(&[("title", SpecValue::Integer(3))])),
            None,
        );
    }

    // -----------------------------------------------------------------------
    // per-style input widget sizing at the Input arm.
    // -----------------------------------------------------------------------

    /// Parse a single composition-level input spec and return its layout rect.
    fn input_rect(yaml: &str) -> Rect {
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
        let inputs = placed_inputs(&parsed.spec, Rect::new(0.0, 0.0, 0.0, 0.0));
        assert_eq!(inputs.len(), 1, "one composition-level input");
        inputs[0].rect
    }

    /// `style: radio` with a literal N-option list reserves
    /// `RADIO_ROW_HEIGHT · N + RADIO_CHROME_PAD` — pinned against the SHARED
    /// constants (the SLIDER_* sync convention), not a recomputed value.
    #[test]
    fn radio_literal_height_formula() {
        let rect = input_rect(
            r#"
vconcat:
  - input: menu
    style: radio
    as: $shape
    options: [circle, square, triangle]
"#,
        );
        assert_eq!(rect.width, DEFAULT_INPUT_WIDTH);
        assert_eq!(
            rect.height,
            RADIO_ROW_HEIGHT * 3.0 + RADIO_CHROME_PAD,
            "three literal options → three 22px rows + the shared chrome pad"
        );
    }

    /// a radio with DERIVED options (from/column — unknown N at
    /// layout time) is layout-sized as a menu; assembly degrades the
    /// presentation to match.
    #[test]
    fn radio_derived_options_menu_sized() {
        let rect = input_rect(
            r#"
data:
  t: [{ region: east }]
vconcat:
  - input: menu
    style: radio
    as: $region
    from: t
    column: region
"#,
        );
        assert_eq!(
            rect,
            Rect::new(0.0, 0.0, DEFAULT_INPUT_WIDTH, DEFAULT_INPUT_HEIGHT),
            "derived radio keeps the menu box — the degrade rule keeps geometry honest"
        );
    }

    /// menu and checkbox presentations keep the fixed 200×32 box.
    #[test]
    fn menu_and_checkbox_unchanged_200x32() {
        let menu = input_rect(
            r#"
vconcat:
  - input: menu
    as: $region
    options: [east, west]
"#,
        );
        assert_eq!(menu, Rect::new(0.0, 0.0, 200.0, 32.0));

        let checkbox = input_rect(
            r#"
vconcat:
  - input: menu
    style: checkbox
    as: $flag
"#,
        );
        assert_eq!(checkbox, Rect::new(0.0, 0.0, 200.0, 32.0));
    }

    /// sizing gates on `InputKind::Menu` — an `input: slider`
    /// carrying a stray `style: radio` + literal `options:` (both inert keys
    /// on a slider) keeps the fixed 200×32 box, never a radio-tall rect.
    #[test]
    fn non_menu_kind_ignores_style_keys() {
        let slider = input_rect(
            r#"
vconcat:
  - input: slider
    style: radio
    as: $threshold
    options: [a, b, c]
"#,
        );
        assert_eq!(
            slider,
            Rect::new(0.0, 0.0, DEFAULT_INPUT_WIDTH, DEFAULT_INPUT_HEIGHT),
            "non-menu kinds ignore the menu-family style keys"
        );
    }

    // --- positional scale type (`xScale` / `yScale`) ---

    /// The key is read, per axis, and a plot that never mentions it is linear.
    ///
    /// The second half is what makes the first a measurement: every spec in
    /// the corpus predates this attribute, so a resolver that answered `Log`
    /// unconditionally would also pass a test that only looked at the `log`
    /// plot.
    #[test]
    fn a_plot_written_x_scale_log_resolves_log() {
        let log = || SpecValue::String("log".to_string());
        assert_eq!(
            resolve_plot_scales(&plot_with(&[("xScale", log())])),
            PlotScales {
                x: ScaleType::Log,
                y: ScaleType::Linear
            },
        );
        assert_eq!(
            resolve_plot_scales(&plot_with(&[("yScale", log())])),
            PlotScales {
                x: ScaleType::Linear,
                y: ScaleType::Log
            },
        );
        let none = resolve_plot_scales(&plot_with(&[]));
        assert!(
            none.is_linear(),
            "a plot without the key is linear on both axes, got {none:?}"
        );
    }

    /// `symlog` is its own resolved kind and not a spelling of `log` — the
    /// two differ exactly at zero, which is the reason both are offered.
    #[test]
    fn symlog_resolves_to_its_own_kind() {
        let p = plot_with(&[("xScale", SpecValue::String("symlog".to_string()))]);
        assert_eq!(resolve_plot_scales(&p).x, ScaleType::Symlog);
        assert!(
            !ScaleType::Symlog.drops_nonpositive(),
            "symlog is defined at zero; only log drops those rows"
        );
        assert!(ScaleType::Log.drops_nonpositive());
    }

    /// A name this build cannot draw degrades to linear rather than to
    /// nothing, and the degradation is per axis.
    #[test]
    fn an_unknown_scale_name_degrades_to_linear() {
        for name in ["band", "sqrt", "LOG", "", "pow"] {
            let p = plot_with(&[("xScale", SpecValue::String(name.to_string()))]);
            assert!(
                resolve_plot_scales(&p).is_linear(),
                "xScale: {name:?} is not a name this build reads and must not leave linear"
            );
        }
    }

    /// The attribute may be a lifted `$param`, and it resolves through the
    /// param's declared value.
    #[test]
    fn a_lifted_param_scale_resolves_through_its_declaration() {
        let p = plot_with(&[("xScale", SpecValue::Param(ParamRef::new("s")))]);
        let mut params = IndexMap::new();
        params.insert(
            "s".to_string(),
            ParamNode::Value(SpecValue::String("log".to_string())),
        );
        assert_eq!(resolve_plot_scales_in(&p, &params).x, ScaleType::Log);
        // Undeclared, and a selection rather than a value, both degrade.
        assert!(resolve_plot_scales_in(&p, &IndexMap::new()).is_linear());
    }

    /// The resolver reads the axis's key out of the consumed list, so a key
    /// dropped from that list is a key nothing reads.
    #[test]
    fn each_axis_has_its_key_on_the_consumed_list() {
        assert_eq!(plot_scale_key(PlotAxis::X), Some("xScale"));
        assert_eq!(plot_scale_key(PlotAxis::Y), Some("yScale"));
    }

    /// The attribute survives a real parse: `xScale: log` written in a YAML
    /// document reaches `PlotNode::attributes` as the string the resolver
    /// reads, rather than being dropped or lifted into something else.
    #[test]
    fn the_scale_attribute_survives_a_parse() {
        let spec = parse_spec(
            r#"
plot:
  - mark: rectY
    data: { from: t }
    x: { bin: v }
    y: { count: }
xScale: log
"#,
            Format::Yaml,
        )
        .expect("parses");
        let plots = collect_plot_nodes(&spec.spec);
        assert_eq!(resolve_plot_scales(plots[0].1).x, ScaleType::Log);
    }

    // --- positional domain pinning (`Domain: Fixed`) ---

    /// Each positional axis is read on its own key, and neither reaches across.
    #[test]
    fn fixed_is_read_per_axis() {
        let fixed = || SpecValue::String("Fixed".to_string());
        assert_eq!(
            resolve_fixed_domains(&plot_with(&[("xDomain", fixed())])),
            FixedDomains { x: true, y: false }
        );
        assert_eq!(
            resolve_fixed_domains(&plot_with(&[("yDomain", fixed())])),
            FixedDomains { x: false, y: true }
        );
        assert_eq!(
            resolve_fixed_domains(&plot_with(&[("xDomain", fixed()), ("yDomain", fixed())])),
            FixedDomains { x: true, y: true }
        );
        assert!(resolve_fixed_domains(&plot_with(&[])).is_empty());
    }

    /// **Every other value at these keys leaves the axis unpinned.** A
    /// two-element domain and a `$param` are different instructions with
    /// different effects, and the positions `deviations.yaml` DEV-0005 records
    /// as unread have to stay unread — a resolver that treated any `xDomain` as
    /// a pin would silently freeze `mark-types.yaml`'s explicit `[-1, 8]`.
    #[test]
    fn only_the_fixed_literal_pins_an_axis() {
        for value in [
            SpecValue::Array(vec![SpecValue::Integer(0), SpecValue::Integer(100)]),
            SpecValue::Param(ParamRef::new("domain")),
            SpecValue::String("fixed".to_string()),
            SpecValue::String("FIXED".to_string()),
            SpecValue::Null,
            SpecValue::Bool(true),
        ] {
            let p = plot_with(&[("xDomain", value.clone())]);
            assert!(
                resolve_fixed_domains(&p).is_empty(),
                "xDomain: {value:?} is not the Fixed literal and must not pin the axis"
            );
        }
    }

    /// The both-axes shorthand and the facet axes are recorded as unread
    /// (DEV-0005), so the resolver must not answer for them.
    #[test]
    fn the_shorthand_and_facet_keys_are_not_read() {
        for key in ["xyDomain", "fxDomain", "fyDomain"] {
            let p = plot_with(&[(key, SpecValue::String("Fixed".to_string()))]);
            assert!(
                resolve_fixed_domains(&p).is_empty(),
                "{key} is not read here; DEV-0005 is what says so"
            );
        }
    }

    /// A pin written under `plotDefaults` reaches a plot that sets no
    /// `xDomain`/`yDomain` of its own: `Walker::walk_plot` merges the whole
    /// `plotDefaults` bag into a plot's attributes before `resolve_fixed_domains`
    /// ever runs, so the resolver — which reads `plot.attributes` alone and
    /// cannot tell where an entry came from — sees the default exactly as it
    /// would a value written on the plot directly. This closes the
    /// `plotDefaults` clause of `deviations.yaml` DEV-0005.
    #[test]
    fn a_plot_defaults_pin_reaches_a_plot_that_does_not_set_its_own() {
        let parsed = parse_spec(
            r"
data:
  t:
    - { x: 1, y: 2 }
plotDefaults:
  xDomain: Fixed
plot:
  - { mark: dot, data: { from: t }, x: x, y: y }
",
            Format::Yaml,
        )
        .expect("parse");
        let nodes = collect_plot_nodes(&parsed.spec);
        assert_eq!(nodes.len(), 1, "one plot");
        assert!(
            resolve_fixed_domains(nodes[0].1).x,
            "the plot sets no xDomain of its own; the plotDefaults pin should reach it"
        );
    }

    /// **The override, not just the reach.** The merge only fills a key the
    /// plot left unset (`attributes.entry(key).or_insert(default)`), so a
    /// plot's own, different instruction at the same key is not overwritten
    /// by the default sitting next to it — the first of AC1's three
    /// precedence pairs, "a per-plot value of the same attribute wins over
    /// the default".
    #[test]
    fn a_plots_own_xdomain_wins_over_the_same_key_under_plot_defaults() {
        let parsed = parse_spec(
            r"
data:
  t:
    - { x: 1, y: 2 }
plotDefaults:
  xDomain: Fixed
plot:
  - { mark: dot, data: { from: t }, x: x, y: y }
xDomain: [0, 100]
",
            Format::Yaml,
        )
        .expect("parse");
        let nodes = collect_plot_nodes(&parsed.spec);
        assert_eq!(nodes.len(), 1, "one plot");
        assert!(
            !resolve_fixed_domains(nodes[0].1).x,
            "the plot's own xDomain is an explicit two-element domain, not \
             Fixed; the plotDefaults entry must not overwrite it with a pin"
        );
    }

    // --- tick count (`xTicks` / `yTicks`) ---

    /// Each positional axis is read on its own key, and neither reaches
    /// across — mirroring [`fixed_is_read_per_axis`].
    #[test]
    fn tick_count_is_read_per_axis() {
        assert_eq!(
            resolve_tick_counts(&plot_with(&[("xTicks", SpecValue::Integer(2))])),
            TickCounts {
                x: Some(2),
                y: None
            }
        );
        assert_eq!(
            resolve_tick_counts(&plot_with(&[("yTicks", SpecValue::Integer(4))])),
            TickCounts {
                x: None,
                y: Some(4)
            }
        );
        assert_eq!(
            resolve_tick_counts(&plot_with(&[
                ("xTicks", SpecValue::Integer(10)),
                ("yTicks", SpecValue::Integer(4)),
            ])),
            TickCounts {
                x: Some(10),
                y: Some(4)
            }
        );
        assert_eq!(resolve_tick_counts(&plot_with(&[])), TickCounts::default());
    }

    /// A plot that sets neither key is drawn at [`DEFAULT_TICK_COUNT`], and that
    /// is five: the target the scene builder's `compute_ticks` calls drew
    /// before either key was read. The drawn ticks are pinned by the shell's default-arm tests
    /// on domains where four and six would draw differently; this pins the
    /// number they are pinning.
    #[test]
    fn a_plot_that_asks_for_nothing_is_drawn_at_five() {
        assert_eq!(DEFAULT_TICK_COUNT, 5);
        let none = resolve_tick_counts(&plot_with(&[]));
        assert_eq!((none.x_target(), none.y_target()), (5, 5));
        let one = resolve_tick_counts(&plot_with(&[("xTicks", SpecValue::Integer(7))]));
        assert_eq!((one.x_target(), one.y_target()), (7, 5));
    }

    /// A whole float (`xTicks: 2.0`) sets the same target an integer would —
    /// the AST's numeric split by literal syntax, not by the author's
    /// intent, is not a second instruction.
    #[test]
    fn a_whole_float_sets_a_target_same_as_the_integer() {
        assert_eq!(
            resolve_tick_counts(&plot_with(&[("xTicks", SpecValue::Float(2.0))])).x,
            Some(2)
        );
    }

    /// **A value that is not a target leaves that axis at the default —
    /// AC4's warned case.** Zero, negative, fractional and non-numeric are
    /// each a value `nice_step` cannot aim a step at; a `$param` is a
    /// recorded deferral (`resolve_fixed_domains`'s own exclusion), not
    /// a bad request, and defers the same way.
    #[test]
    fn only_a_positive_whole_number_sets_a_target() {
        for value in [
            SpecValue::Integer(0),
            SpecValue::Integer(-3),
            SpecValue::Float(2.5),
            SpecValue::Float(-1.0),
            SpecValue::Float(f64::NAN),
            SpecValue::Float(f64::INFINITY),
            SpecValue::String("10".to_string()),
            SpecValue::Bool(true),
            SpecValue::Null,
            SpecValue::Param(ParamRef::new("n")),
        ] {
            let p = plot_with(&[("xTicks", value.clone())]);
            assert_eq!(
                resolve_tick_counts(&p).x,
                None,
                "xTicks: {value:?} is not a positive whole number and must not set a target"
            );
        }
    }

    /// **The bound is inclusive at the top and exact.** [`MAX_TICK_COUNT`] is
    /// a count an axis can be asked for and one past it is not, as an integer
    /// and as a whole float — so the bound is where it is written, not a
    /// neighbouring number.
    #[test]
    fn the_bound_is_inclusive_and_one_past_it_is_not_a_target() {
        let at = MAX_TICK_COUNT;
        let past = i64::try_from(MAX_TICK_COUNT).expect("the bound fits an i64") + 1;
        assert_eq!(tick_count_target(&SpecValue::Integer(past - 1)), Some(at));
        assert_eq!(tick_count_target(&SpecValue::Integer(past)), None);
        assert_eq!(
            tick_count_target(&SpecValue::Float((past - 1) as f64)),
            Some(at)
        );
        assert_eq!(tick_count_target(&SpecValue::Float(past as f64)), None);
        assert_eq!(tick_count_target(&SpecValue::Integer(1)), Some(1));
    }

    /// A tick count written under `plotDefaults` reaches a plot that sets no
    /// `xTicks`/`yTicks` of its own, exactly as
    /// [`a_plot_defaults_pin_reaches_a_plot_that_does_not_set_its_own`]
    /// verifies for `xDomain`: `Walker::walk_plot` merges the whole
    /// `plotDefaults` bag key-agnostically, before either resolver ever
    /// runs.
    #[test]
    fn a_plot_defaults_tick_count_reaches_a_plot_that_does_not_set_its_own() {
        let parsed = parse_spec(
            r"
data:
  t:
    - { x: 1, y: 2 }
plotDefaults:
  xTicks: 3
plot:
  - { mark: dot, data: { from: t }, x: x, y: y }
",
            Format::Yaml,
        )
        .expect("parse");
        let nodes = collect_plot_nodes(&parsed.spec);
        assert_eq!(nodes.len(), 1, "one plot");
        assert_eq!(
            resolve_tick_counts(nodes[0].1).x,
            Some(3),
            "the plot sets no xTicks of its own; the plotDefaults value should reach it"
        );
    }

    // --- where an axis starts and ends (`xZero` / `xNice` / `yZero` / `yNice`) ---

    /// A plot that writes none of the four keys asks for nothing, which is what
    /// it drew before the keys were read, and `false` asks for the same.
    #[test]
    fn a_plot_that_sets_no_axis_end_key_asks_for_nothing() {
        assert!(resolve_axis_ends(&plot_with(&[])).is_empty());
        let all_false: Vec<(&str, SpecValue)> = ["xZero", "xNice", "yZero", "yNice"]
            .into_iter()
            .map(|key| (key, SpecValue::Bool(false)))
            .collect();
        assert!(
            resolve_axis_ends(&plot_with(&all_false)).is_empty(),
            "`false` on each key asks for what an unset plot gets"
        );
    }

    /// Each key sets its own switch on its own axis, and neither reaches across:
    /// `yZero` does not set `yNice`, and `xNice` does not set `yNice`.
    #[test]
    fn each_axis_end_key_sets_its_own_switch_on_its_own_axis() {
        let only = |key: &str| resolve_axis_ends(&plot_with(&[(key, SpecValue::Bool(true))]));
        let zero = AxisEnd {
            zero: true,
            nice: false,
        };
        let nice = AxisEnd {
            zero: false,
            nice: true,
        };
        let none = AxisEnd::default();
        assert_eq!(only("xZero"), AxisEnds { x: zero, y: none });
        assert_eq!(only("xNice"), AxisEnds { x: nice, y: none });
        assert_eq!(only("yZero"), AxisEnds { x: none, y: zero });
        assert_eq!(only("yNice"), AxisEnds { x: none, y: nice });
    }

    /// A value that is no switch reads as absent, so the axis asks for nothing.
    /// The judge is [`axis_end_switch`], the same one the parser's warning asks.
    /// A number at a `nice` key is a count to Observable Plot and is no switch
    /// here.
    #[test]
    fn a_value_that_is_no_axis_end_switch_reads_as_absent() {
        let no_switches = [
            SpecValue::String("true".to_string()),
            SpecValue::Integer(1),
            SpecValue::Integer(5),
            SpecValue::Null,
            SpecValue::Array(vec![]),
            SpecValue::Param(ParamRef::new("z")),
        ];
        for value in no_switches {
            assert_eq!(axis_end_switch(&value), None, "the judge on {value:?}");
            for key in ["xZero", "xNice", "yZero", "yNice"] {
                assert!(
                    resolve_axis_ends(&plot_with(&[(key, value.clone())])).is_empty(),
                    "`{key}: {value:?}` is read as absent"
                );
            }
        }
        assert_eq!(axis_end_switch(&SpecValue::Bool(true)), Some(true));
        assert_eq!(axis_end_switch(&SpecValue::Bool(false)), Some(false));
    }

    // --- which way an axis runs (`xReverse` / `yReverse`) ---

    /// A plot that writes neither key reverses nothing, which is what it drew
    /// before the keys were read, and `false` asks for the same.
    #[test]
    fn a_plot_that_sets_no_reverse_key_reverses_nothing() {
        assert!(resolve_axis_reverse(&plot_with(&[])).is_empty());
        let both_false = [
            ("xReverse", SpecValue::Bool(false)),
            ("yReverse", SpecValue::Bool(false)),
        ];
        assert!(
            resolve_axis_reverse(&plot_with(&both_false)).is_empty(),
            "`false` on each key asks for what an unset plot gets"
        );
    }

    /// Each key reverses its own axis and neither reaches across: `yReverse`
    /// does not reverse x, and `xReverse` does not reverse y.
    #[test]
    fn each_reverse_key_reverses_its_own_axis_only() {
        let only = |key: &str| resolve_axis_reverse(&plot_with(&[(key, SpecValue::Bool(true))]));
        assert_eq!(only("xReverse"), AxisReverse { x: true, y: false });
        assert_eq!(only("yReverse"), AxisReverse { x: false, y: true });
    }

    /// A value that is no switch reads as absent, so no axis is reversed. The
    /// judge is [`axis_reverse_switch`], the same one the parser's warning asks.
    #[test]
    fn a_value_that_is_no_reverse_switch_reads_as_absent() {
        let no_switches = [
            SpecValue::String("true".to_string()),
            SpecValue::Integer(1),
            SpecValue::Null,
            SpecValue::Array(vec![]),
            SpecValue::Param(ParamRef::new("z")),
        ];
        for value in no_switches {
            assert_eq!(axis_reverse_switch(&value), None, "the judge on {value:?}");
            for key in ["xReverse", "yReverse"] {
                assert!(
                    resolve_axis_reverse(&plot_with(&[(key, value.clone())])).is_empty(),
                    "`{key}: {value:?}` is read as absent"
                );
            }
        }
        assert_eq!(axis_reverse_switch(&SpecValue::Bool(true)), Some(true));
        assert_eq!(axis_reverse_switch(&SpecValue::Bool(false)), Some(false));
    }

    // --- gridlines (`grid` / `xGrid` / `yGrid`) ---

    /// A plot that sets none of the three keys draws gridlines on both axes,
    /// the reading it had before the keys were read. A plot that only asks for
    /// them on is asking for what it already got.
    #[test]
    fn a_plot_that_sets_no_grid_key_draws_gridlines_on_both_axes() {
        let both = GridLines { x: true, y: true };
        assert_eq!(GridLines::default(), both);
        assert_eq!(resolve_grid_lines(&plot_with(&[])), both);
        assert_eq!(
            resolve_grid_lines(&plot_with(&[("grid", SpecValue::Bool(true))])),
            both
        );
        assert_eq!(
            resolve_grid_lines(&plot_with(&[("yGrid", SpecValue::Bool(true))])),
            both,
            "`yGrid: true` alone leaves x at its default rather than turning it off"
        );
    }

    /// Each axis is read on its own key, and neither reaches across.
    #[test]
    fn an_axis_grid_key_is_read_on_its_own_axis() {
        assert_eq!(
            resolve_grid_lines(&plot_with(&[("yGrid", SpecValue::Bool(false))])),
            GridLines { x: true, y: false }
        );
        assert_eq!(
            resolve_grid_lines(&plot_with(&[("xGrid", SpecValue::Bool(false))])),
            GridLines { x: false, y: true }
        );
        assert_eq!(
            resolve_grid_lines(&plot_with(&[
                ("xGrid", SpecValue::Bool(false)),
                ("yGrid", SpecValue::Bool(false)),
            ])),
            GridLines { x: false, y: false }
        );
    }

    /// **The key that names an axis outranks the bare `grid`.** `grid: true`
    /// with `xGrid: false` draws the horizontal rules and not the vertical
    /// ones, and the mirror case turns one axis on under a bare `grid: false`.
    #[test]
    fn the_key_that_names_an_axis_outranks_the_bare_grid() {
        let on = SpecValue::Bool(true);
        let off = SpecValue::Bool(false);
        let cases: [(&[(&str, &SpecValue)], GridLines); 5] = [
            (&[("grid", &off)], GridLines { x: false, y: false }),
            (
                &[("grid", &on), ("xGrid", &off)],
                GridLines { x: false, y: true },
            ),
            (
                &[("grid", &on), ("yGrid", &off)],
                GridLines { x: true, y: false },
            ),
            (
                &[("grid", &off), ("xGrid", &on)],
                GridLines { x: true, y: false },
            ),
            (
                &[("grid", &off), ("xGrid", &on), ("yGrid", &on)],
                GridLines { x: true, y: true },
            ),
        ];
        for (attrs, expected) in cases {
            let owned: Vec<(&str, SpecValue)> =
                attrs.iter().map(|(k, v)| (*k, (*v).clone())).collect();
            assert_eq!(
                resolve_grid_lines(&plot_with(&owned)),
                expected,
                "the reading of {attrs:?}"
            );
        }
    }

    /// A value that is no switch reads as absent, so the axis falls through to
    /// the bare `grid` and then to the default. The judge is
    /// [`grid_switch`], the same one the parser's warning asks.
    #[test]
    fn a_value_that_is_no_grid_switch_reads_as_absent() {
        let draws = GridLines { x: true, y: true };
        let no_switches = [
            SpecValue::String("off".to_string()),
            SpecValue::String("false".to_string()),
            SpecValue::Integer(0),
            SpecValue::Null,
            SpecValue::Array(vec![]),
            SpecValue::Param(ParamRef::new("g")),
        ];
        for value in no_switches {
            assert_eq!(grid_switch(&value), None, "the judge on {value:?}");
            assert_eq!(
                resolve_grid_lines(&plot_with(&[("yGrid", value.clone())])),
                draws,
                "`yGrid: {value:?}` is read as absent, so the default draws"
            );
            assert_eq!(
                resolve_grid_lines(&plot_with(&[
                    ("grid", SpecValue::Bool(false)),
                    ("yGrid", value.clone()),
                ])),
                GridLines { x: false, y: false },
                "`yGrid: {value:?}` is read as absent, so the bare `grid: false` decides"
            );
        }
        assert_eq!(grid_switch(&SpecValue::Bool(true)), Some(true));
        assert_eq!(grid_switch(&SpecValue::Bool(false)), Some(false));
    }

    /// A `plotDefaults` switch reaches a plot that does not write its own, and
    /// a switch the plot writes wins over it — the merge in `walk_plot` is
    /// key-agnostic and keeps the plot's own value.
    #[test]
    fn a_plot_defaults_grid_key_reaches_a_plot_that_does_not_set_its_own() {
        let parsed = parse_spec(
            r"
data:
  t:
    - { x: 1, y: 2 }
plotDefaults:
  yGrid: false
vconcat:
  - plot:
      - { mark: dot, data: { from: t }, x: x, y: y }
  - plot:
      - { mark: dot, data: { from: t }, x: x, y: y }
    yGrid: true
",
            Format::Yaml,
        )
        .expect("parse");
        let nodes = collect_plot_nodes(&parsed.spec);
        assert_eq!(nodes.len(), 2, "two plots");
        assert_eq!(
            resolve_grid_lines(nodes[0].1),
            GridLines { x: true, y: false },
            "the first plot sets no yGrid of its own; the plotDefaults value should reach it"
        );
        assert_eq!(
            resolve_grid_lines(nodes[1].1),
            GridLines { x: true, y: true },
            "the second plot writes yGrid: true, which wins over the default"
        );
    }

    // --- xTickFormat / yTickFormat ---

    fn format_attr(key: &'static str, spec: &str) -> PlotNode {
        plot_with(&[(key, SpecValue::String(spec.to_string()))])
    }

    /// Each axis reads its own key, and a key the plot does not write resolves
    /// to nothing rather than to a default: what an axis then draws is the
    /// renderer's to say.
    #[test]
    fn a_tick_format_is_read_per_axis() {
        let x = resolve_tick_formats(&format_attr("xTickFormat", "s"));
        assert_eq!(x.x, NumberFormat::parse("s").map(AxisFormat::Number));
        assert!(
            x.x.is_some() && x.y.is_none(),
            "only x wrote a format: {x:?}"
        );

        let y = resolve_tick_formats(&format_attr("yTickFormat", "+.1f"));
        assert_eq!(y.y, NumberFormat::parse("+.1f").map(AxisFormat::Number));
        assert!(
            y.x.is_none() && y.y.is_some(),
            "only y wrote a format: {y:?}"
        );

        assert_eq!(
            resolve_tick_formats(&plot_with(&[])),
            TickFormats::default()
        );
    }

    /// A value the judge does not accept resolves to none, whether it is a
    /// typo, a date format with a directive this build does not read, or not a
    /// string; and a date format it does read resolves to a date format.
    #[test]
    fn a_value_that_is_no_format_resolves_to_none() {
        for spec in ["~~", ".f", "%K", "%Y-%K", "abc"] {
            assert_eq!(
                resolve_tick_formats(&format_attr("xTickFormat", spec)).x,
                None,
                "`{spec}` is no format this build reads"
            );
        }
        for value in [
            SpecValue::Integer(3),
            SpecValue::Bool(true),
            SpecValue::Null,
        ] {
            assert_eq!(
                resolve_tick_formats(&plot_with(&[("xTickFormat", value.clone())])).x,
                None,
                "{value:?} is no format"
            );
        }
        for spec in ["%b", "%Y-%m-%d", "%B %Y", "%H:%M"] {
            assert_eq!(
                resolve_tick_formats(&format_attr("xTickFormat", spec)).x,
                DateFormat::parse(spec).ok().map(AxisFormat::Date),
                "`{spec}` is a date format"
            );
        }
    }

    /// The parser stays silent about exactly the values the reader accepts and
    /// the values this build defers, and speaks about the rest. The reader and
    /// the warning are one judgement asked twice, so a value the reader accepts
    /// is never one the warning names.
    #[test]
    fn what_the_parser_warns_about_and_what_the_reader_accepts_do_not_overlap() {
        let s = |v: &str| SpecValue::String(v.to_string());
        let unread = |d: &str| TickFormatReading::UnreadDirective(d.to_string());
        let number = |v: &str| {
            TickFormatReading::Format(AxisFormat::Number(NumberFormat::parse(v).expect("number")))
        };
        let date = |v: &str| {
            TickFormatReading::Format(AxisFormat::Date(DateFormat::parse(v).expect("date")))
        };
        let cases = [
            (s("s"), number("s")),
            (s(".2s"), number(".2s")),
            (s("+f"), number("+f")),
            (s("%"), number("%")),
            (s("+.1%"), number("+.1%")),
            (s("d"), number("d")),
            (s(""), number("")),
            (s("%b"), date("%b")),
            (s("%Y-%m-%d"), date("%Y-%m-%d")),
            (s("%-d %B"), date("%-d %B")),
            (SpecValue::Null, TickFormatReading::Deferred),
            (s("%K"), unread("%K")),
            (s("%Y-%-K"), unread("%-K")),
            (s("%Y-%"), unread("%")),
            (s("~~"), TickFormatReading::Invalid),
            (s(".f"), TickFormatReading::Invalid),
            (s("ss"), TickFormatReading::Invalid),
            (s("abc%"), TickFormatReading::Invalid),
            (SpecValue::Integer(5), TickFormatReading::Invalid),
            (SpecValue::Bool(false), TickFormatReading::Invalid),
            (SpecValue::Array(vec![]), TickFormatReading::Invalid),
        ];
        for (value, reading) in cases {
            assert_eq!(read_tick_format(&value), reading, "the judge on {value:?}");
        }
    }

    /// A `plotDefaults` format reaches a plot that writes no format of its own,
    /// exactly as a `plotDefaults` tick count does: `Walker::walk_plot` merges
    /// the whole bag key-agnostically before either resolver runs.
    #[test]
    fn a_plot_defaults_tick_format_reaches_a_plot_that_does_not_set_its_own() {
        let parsed = parse_spec(
            r"
data:
  t:
    - { x: 1, y: 2 }
plotDefaults:
  yTickFormat: '%'
plot:
  - { mark: dot, data: { from: t }, x: x, y: y }
",
            Format::Yaml,
        )
        .expect("parse");
        let nodes = collect_plot_nodes(&parsed.spec);
        assert_eq!(nodes.len(), 1, "one plot");
        assert_eq!(
            resolve_tick_formats(nodes[0].1).y,
            NumberFormat::parse("%").map(AxisFormat::Number),
            "the plot sets no yTickFormat of its own; the plotDefaults value should reach it"
        );
    }

    /// **AC5 — a spec that declares no `plotDefaults` resolves the same plot
    /// attributes and the same `FixedDomains` as before this change.** One
    /// line per plot, across the vendored and curated specs whose
    /// `plotDefaults` bag is empty, captured against the tree at
    /// `origin/main` = `cd7a4c6` — before `Walker::walk_plot` touched a
    /// plot's attributes. The merge in `walk_plot` iterates
    /// `self.plot_defaults`, so an empty bag leaves a plot's own attributes
    /// untouched; this pins that down as an exact,
    /// reviewable value rather than an assumption. A bug that let a default
    /// leak onto the wrong plot, or that mutated attributes even off an
    /// empty bag, would redden this — an omitted spec here is a gap in the
    /// baseline, not a passing case, so a corpus-membership change (a vendor
    /// bump, a new curated fixture) that silently drops a line would also
    /// redden it.
    #[test]
    fn a_spec_with_no_plot_defaults_resolves_the_same_plot_attributes_and_fixed_domains_as_before()
    {
        const BASELINE: &str = r#"aeromagnetic-survey.yaml::root/vconcat[2] FixedDomains { x: false, y: false } {"colorScale": String("diverging"), "colorDomain": String("Fixed")}
airline-travelers.yaml::root FixedDomains { x: false, y: false } {"yGrid": Bool(true), "yLabel": String("↑ Travelers per day"), "yTickFormat": String("s")}
area-sine.yaml::root/vconcat[0] FixedDomains { x: false, y: true } {"yDomain": String("Fixed"), "colorDomain": String("Fixed"), "xLabel": Null, "width": Integer(680), "height": Integer(180)}
area-sine.yaml::root/vconcat[2] FixedDomains { x: false, y: true } {"yDomain": String("Fixed"), "colorDomain": String("Fixed"), "xLabel": Null, "width": Integer(680), "height": Integer(180)}
area-sine.yaml::root/vconcat[4] FixedDomains { x: false, y: true } {"yDomain": String("Fixed"), "width": Integer(680), "height": Integer(90)}
athlete-birth-waffle.yaml::root/vconcat[2] FixedDomains { x: false, y: false } {"xLabel": Null, "xTickSize": Integer(0), "xTickFormat": String("d")}
athlete-height.yaml::root/hconcat[0]/vconcat[1] FixedDomains { x: false, y: true } {"name": String("heights"), "xDomain": Array([Float(1.5), Float(2.1)]), "yDomain": String("Fixed"), "yGrid": Bool(true), "yLabel": Null, "marginTop": Integer(5), "marginLeft": Integer(105), "marginRight": Integer(30), "height": Integer(420)}
athletes.yaml::root/hconcat[0]/vconcat[2] FixedDomains { x: false, y: false } {"xyDomain": String("Fixed"), "colorDomain": String("Fixed"), "margins": Object({"left": Integer(35), "top": Integer(20), "right": Integer(1)}), "width": Integer(570), "height": Integer(350)}
axes.yaml::root FixedDomains { x: false, y: false } {"xDomain": Array([Integer(0), Integer(100)]), "yDomain": Array([Integer(0), Integer(100)]), "xInsetLeft": Integer(36), "marginLeft": Integer(0), "marginRight": Integer(35), "width": Integer(680)}
bias.yaml::root/vconcat[1] FixedDomains { x: false, y: false } {"width": Integer(680), "height": Integer(200)}
contours.yaml::root/vconcat[1] FixedDomains { x: false, y: false } {"xAxis": String("bottom"), "xLabelAnchor": String("center"), "yAxis": String("right"), "yLabelAnchor": String("center"), "margins": Object({"top": Integer(5), "bottom": Integer(30), "left": Integer(5), "right": Integer(50)}), "width": Integer(700), "height": Integer(480)}
crossfilter.yaml::root/vconcat[0] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Arrival Delay (min)"), "xLabelAnchor": String("center"), "yTickFormat": String("s"), "height": Integer(200)}
crossfilter.yaml::root/vconcat[1] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Departure Time (hour)"), "xLabelAnchor": String("center"), "yTickFormat": String("s"), "height": Integer(200)}
density-groups.yaml::root/vconcat[1] FixedDomains { x: false, y: false } {"marginLeft": Integer(50), "height": Integer(200)}
density1d.yaml::root/vconcat[1] FixedDomains { x: true, y: false } {"yAxis": Null, "xDomain": String("Fixed"), "width": Integer(600), "marginLeft": Integer(10), "height": Integer(200)}
density1d.yaml::root/vconcat[2] FixedDomains { x: true, y: false } {"yAxis": Null, "xScale": String("log"), "xDomain": String("Fixed"), "width": Integer(600), "marginLeft": Integer(10), "height": Integer(200)}
density2d.yaml::root/vconcat[1] FixedDomains { x: false, y: false } {"rRange": Array([Integer(0), Integer(16)]), "xAxis": String("bottom"), "xLabelAnchor": String("center"), "yAxis": String("right"), "yLabelAnchor": String("center"), "margins": Object({"top": Integer(5), "bottom": Integer(30), "left": Integer(5), "right": Integer(50)}), "width": Integer(700), "height": Integer(480)}
driving-shifts.yaml::root FixedDomains { x: false, y: false } {"inset": Integer(10), "grid": Bool(true), "xLabel": String("Miles driven (per person-year)"), "yLabel": String("Cost of gasoline ($ per gallon)")}
earthquakes-feed.yaml::root FixedDomains { x: false, y: false } {"margin": Integer(2), "projectionType": String("equirectangular")}
earthquakes-globe.yaml::root/vconcat[1] FixedDomains { x: false, y: false } {"margin": Integer(10), "style": String("overflow: visible;"), "projectionType": String("orthographic"), "projectionRotate": Param(ParamRef("rotate"))}
facet-interval.yaml::root/hconcat[0] FixedDomains { x: true, y: true } {"name": String("plot"), "grid": Bool(true), "marginRight": Integer(60), "xDomain": String("Fixed"), "yDomain": String("Fixed"), "fxDomain": String("Fixed"), "fyDomain": String("Fixed"), "fxLabel": Null, "fyLabel": Null}
flights-10m.yaml::root/vconcat[0] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Arrival Delay (min)"), "yTickFormat": String("s"), "width": Integer(600), "height": Integer(200)}
flights-10m.yaml::root/vconcat[1] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Departure Time (hour)"), "yTickFormat": String("s"), "width": Integer(600), "height": Integer(200)}
flights-10m.yaml::root/vconcat[2] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Flight Distance (miles)"), "yTickFormat": String("s"), "width": Integer(600), "height": Integer(200)}
flights-200k.yaml::root/vconcat[0] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Arrival Delay (min)"), "yTickFormat": String("s"), "width": Integer(600), "height": Integer(200)}
flights-200k.yaml::root/vconcat[1] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Departure Time (hour)"), "yTickFormat": String("s"), "width": Integer(600), "height": Integer(200)}
flights-200k.yaml::root/vconcat[2] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Flight Distance (miles)"), "yTickFormat": String("s"), "width": Integer(600), "height": Integer(200)}
flights-density.yaml::root/vconcat[1] FixedDomains { x: false, y: false } {"colorScale": String("symlog"), "colorScheme": String("ylgnbu"), "xAxis": String("top"), "xLabelAnchor": String("center"), "xZero": Bool(true), "yAxis": String("right"), "yLabelAnchor": String("center"), "marginTop": Integer(30), "marginLeft": Integer(5), "marginRight": Integer(40), "width": Integer(700), "height": Integer(500)}
flights-hexbin.yaml::root/vconcat[1]/hconcat[0] FixedDomains { x: true, y: false } {"margins": Object({"left": Integer(5), "right": Integer(5), "top": Integer(30), "bottom": Integer(0)}), "xDomain": String("Fixed"), "xAxis": String("top"), "yAxis": Null, "xLabelAnchor": String("center"), "width": Integer(605), "height": Integer(70)}
flights-hexbin.yaml::root/vconcat[2]/hconcat[0] FixedDomains { x: false, y: false } {"name": String("hexbins"), "colorScheme": String("ylgnbu"), "colorScale": Param(ParamRef("scale")), "margins": Object({"left": Integer(5), "right": Integer(0), "top": Integer(0), "bottom": Integer(5)}), "xAxis": Null, "yAxis": Null, "xyDomain": String("Fixed"), "width": Integer(600), "height": Integer(455)}
flights-hexbin.yaml::root/vconcat[2]/hconcat[1] FixedDomains { x: false, y: false } {"margins": Object({"left": Integer(0), "right": Integer(50), "top": Integer(4), "bottom": Integer(5)}), "yDomain": Array([Integer(-60), Integer(180)]), "xAxis": Null, "yAxis": String("right"), "yLabelAnchor": String("center"), "width": Integer(80), "height": Integer(455)}
gaia.yaml::root/hconcat[0]/vconcat[0] FixedDomains { x: false, y: false } {"xyDomain": String("Fixed"), "colorScale": Param(ParamRef("scaleType")), "colorScheme": String("viridis"), "width": Integer(440), "height": Integer(250), "marginLeft": Integer(25), "marginTop": Integer(20), "marginRight": Integer(1)}
gaia.yaml::root/hconcat[0]/vconcat[1]/hconcat[0] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "yScale": Param(ParamRef("scaleType")), "yGrid": Bool(true), "width": Integer(220), "height": Integer(120), "marginLeft": Integer(65)}
gaia.yaml::root/hconcat[0]/vconcat[1]/hconcat[1] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "yScale": Param(ParamRef("scaleType")), "yGrid": Bool(true), "width": Integer(220), "height": Integer(120), "marginLeft": Integer(65)}
gaia.yaml::root/hconcat[2] FixedDomains { x: false, y: false } {"xyDomain": String("Fixed"), "colorScale": Param(ParamRef("scaleType")), "colorScheme": String("viridis"), "yReverse": Bool(true), "width": Integer(230), "height": Integer(370), "marginLeft": Integer(25), "marginTop": Integer(20), "marginRight": Integer(1)}
line-density.yaml::root/vconcat[2] FixedDomains { x: false, y: false } {"colorScheme": Param(ParamRef("schemeColor")), "colorScale": Param(ParamRef("scaleColor")), "yLabel": String("Close (Normalized) ↑"), "yNice": Bool(true), "margins": Object({"left": Integer(30), "top": Integer(20), "right": Integer(0)}), "width": Integer(680), "height": Integer(240)}
line-density.yaml::root/vconcat[3] FixedDomains { x: false, y: false } {"colorScheme": Param(ParamRef("schemeColor")), "colorScale": Param(ParamRef("scaleColor")), "yLabel": String("Close (Unnormalized) ↑"), "yNice": Bool(true), "margins": Object({"left": Integer(30), "top": Integer(20), "right": Integer(0)}), "width": Integer(680), "height": Integer(240)}
line-multi-series.yaml::root FixedDomains { x: false, y: false } {"marginLeft": Integer(24), "xLabel": Null, "xTicks": Integer(10), "yLabel": String("Unemployment (%)"), "yGrid": Bool(true), "style": String("overflow: visible;"), "width": Integer(680)}
line.yaml::root FixedDomains { x: false, y: false } {"width": Integer(680), "height": Integer(200)}
linear-regression-10m.yaml::root/vconcat[2] FixedDomains { x: false, y: false } {"xDomain": Array([Integer(0), Integer(24)]), "yDomain": Array([Integer(-60), Integer(180)]), "colorScale": String("symlog"), "colorScheme": String("blues"), "colorDomain": String("Fixed")}
linear-regression.yaml::root FixedDomains { x: false, y: false } {"xyDomain": String("Fixed"), "colorDomain": String("Fixed")}
moving-average.yaml::root/vconcat[0] FixedDomains { x: false, y: false } {"xLabel": String("day"), "width": Integer(680), "height": Integer(300)}
normalize.yaml::root FixedDomains { x: false, y: false } {"yScale": String("log"), "yDomain": Array([Float(0.2), Integer(6)]), "yGrid": Bool(true), "xLabel": Null, "yLabel": Null, "yTickFormat": String("%"), "width": Integer(680), "height": Integer(400), "marginRight": Integer(35)}
nyc-taxi-rides.yaml::root/vconcat[0]/hconcat[0] FixedDomains { x: false, y: false } {"width": Integer(335), "height": Integer(550), "margin": Integer(0), "xAxis": Null, "yAxis": Null, "xDomain": Array([Float(975000.0), Float(1005000.0)]), "yDomain": Array([Float(190000.0), Float(240000.0)]), "colorScale": String("symlog"), "colorScheme": String("blues")}
nyc-taxi-rides.yaml::root/vconcat[0]/hconcat[2] FixedDomains { x: false, y: false } {"width": Integer(335), "height": Integer(550), "margin": Integer(0), "xAxis": Null, "yAxis": Null, "xDomain": Array([Float(975000.0), Float(1005000.0)]), "yDomain": Array([Float(190000.0), Float(240000.0)]), "colorScale": String("symlog"), "colorScheme": String("oranges")}
nyc-taxi-rides.yaml::root/vconcat[2] FixedDomains { x: false, y: false } {"yTickFormat": String("s"), "xLabel": String("Pickup Hour →"), "width": Integer(680), "height": Integer(100)}
observable-latency.yaml::root/vconcat[0] FixedDomains { x: false, y: false } {"colorDomain": String("Fixed"), "colorScheme": String("observable10"), "opacityDomain": Array([Integer(0), Integer(25)]), "opacityClamp": Bool(true), "yScale": String("log"), "yLabel": String("↑ Duration (ms)"), "yDomain": Array([Float(0.5), Integer(10000)]), "yTickFormat": String("s"), "xScale": String("utc"), "xLabel": Null, "xDomain": Array([Integer(1706227200000), Integer(1706832000000)]), "width": Integer(680), "height": Integer(300), "margins": Object({"left": Integer(35), "top": Integer(20), "bottom": Integer(30), "right": Integer(20)})}
observable-latency.yaml::root/vconcat[1] FixedDomains { x: false, y: false } {"colorDomain": String("Fixed"), "xLabel": String("Routes by Total Requests"), "xTickFormat": String("s"), "yLabel": Null, "width": Integer(680), "height": Integer(300), "marginTop": Integer(5), "marginLeft": Integer(220), "marginBottom": Integer(35)}
overview-detail.yaml::root/vconcat[0] FixedDomains { x: false, y: false } {"width": Integer(680), "height": Integer(200)}
overview-detail.yaml::root/vconcat[1] FixedDomains { x: false, y: true } {"yDomain": String("Fixed"), "width": Integer(680), "height": Integer(200)}
pan-zoom.yaml::root/hconcat[0]/vconcat[0] FixedDomains { x: false, y: false } {"width": Integer(320), "height": Integer(240)}
pan-zoom.yaml::root/hconcat[0]/vconcat[2] FixedDomains { x: false, y: false } {"width": Integer(320), "height": Integer(240)}
pan-zoom.yaml::root/hconcat[2]/vconcat[0] FixedDomains { x: false, y: false } {"width": Integer(320), "height": Integer(240)}
pan-zoom.yaml::root/hconcat[2]/vconcat[2] FixedDomains { x: false, y: false } {"width": Integer(320), "height": Integer(240)}
population-arrows.yaml::root/vconcat[1] FixedDomains { x: false, y: false } {"name": String("arrows"), "grid": Bool(true), "inset": Integer(10), "xScale": String("log"), "xLabel": String("Population →"), "yLabel": String("↑ Inequality"), "yTicks": Integer(4), "colorScheme": String("BuRd"), "colorTickFormat": String("+f")}
presidential-opinion.yaml::root/vconcat[0] FixedDomains { x: false, y: false } {"xInset": Integer(20), "xLabel": String("First inauguration date →"), "yInsetTop": Integer(4), "yGrid": Bool(true), "yLabel": String("↑ Opinion (%)"), "yTickFormat": String("+f")}
protein-design.yaml::root/vconcat[2]/hconcat[0] FixedDomains { x: false, y: false } {"width": Integer(600), "height": Integer(55), "xAxis": Null, "yAxis": Null, "xDomain": Param(ParamRef("plddt_domain")), "colorDomain": String("Fixed"), "colorScheme": Param(ParamRef("scheme")), "marginLeft": Integer(40), "marginRight": Integer(0), "marginTop": Integer(0), "marginBottom": Integer(0)}
protein-design.yaml::root/vconcat[3]/hconcat[0] FixedDomains { x: false, y: false } {"name": String("scatter"), "opacityDomain": Array([Integer(0), Integer(2)]), "opacityClamp": Bool(true), "colorDomain": String("Fixed"), "colorScheme": Param(ParamRef("scheme")), "xDomain": Param(ParamRef("plddt_domain")), "yDomain": Param(ParamRef("pae_domain")), "xLabelAnchor": String("center"), "yLabelAnchor": String("center"), "marginTop": Integer(0), "marginLeft": Integer(40), "marginRight": Integer(0), "width": Integer(600), "height": Integer(450)}
protein-design.yaml::root/vconcat[3]/hconcat[1] FixedDomains { x: false, y: false } {"width": Integer(55), "height": Integer(450), "xAxis": Null, "yAxis": Null, "marginTop": Integer(0), "marginLeft": Integer(0), "marginRight": Integer(0), "yDomain": Param(ParamRef("pae_domain")), "colorDomain": String("Fixed"), "colorScheme": Param(ParamRef("scheme"))}
region-tests.yaml::root/vconcat[0] FixedDomains { x: false, y: false } {"marginLeft": Integer(24), "xLabel": Null, "xTicks": Integer(10), "xLine": Bool(true), "yLine": Bool(true), "yLabel": String("Unemployment (%)"), "yGrid": Bool(true), "marginRight": Integer(0)}
region-tests.yaml::root/vconcat[2] FixedDomains { x: false, y: false } {"margin": Integer(2), "projectionType": String("equirectangular")}
region-tests.yaml::root/vconcat[4] FixedDomains { x: false, y: false } {"margin": Integer(0), "projectionType": String("albers")}
seattle-temp.yaml::root FixedDomains { x: false, y: false } {"xTickFormat": String("%b"), "yLabel": String("Temperature Range (°C)"), "width": Integer(680), "height": Integer(300)}
sorted-bars.yaml::root/vconcat[2] FixedDomains { x: false, y: false } {"xLabel": String("Gold Medals"), "yLabel": String("Nationality"), "yLabelAnchor": String("top"), "marginTop": Integer(15)}
symbols.yaml::root/vconcat[2]/hconcat[0] FixedDomains { x: false, y: false } {"name": String("stroked"), "grid": Bool(true), "xLabel": String("Body mass (g) →"), "yLabel": String("↑ Flipper length (mm)")}
symbols.yaml::root/vconcat[4]/hconcat[0] FixedDomains { x: false, y: false } {"name": String("filled"), "grid": Bool(true), "xLabel": String("Body mass (g) →"), "yLabel": String("↑ Flipper length (mm)")}
triangle-wave.yaml::root/vconcat[0] FixedDomains { x: false, y: false } {"xLabel": Null, "width": Integer(680), "height": Integer(150)}
triangle-wave.yaml::root/vconcat[2] FixedDomains { x: false, y: true } {"yDomain": String("Fixed"), "colorDomain": String("Fixed"), "xLabel": Null, "width": Integer(680), "height": Integer(150)}
triangle-wave.yaml::root/vconcat[4] FixedDomains { x: false, y: true } {"yDomain": String("Fixed"), "colorDomain": String("Fixed"), "xLabel": Null, "width": Integer(680), "height": Integer(150)}
unemployment.yaml::root/vconcat[1] FixedDomains { x: false, y: false } {"name": String("county-map"), "margin": Integer(0), "colorScale": String("quantile"), "colorN": Integer(9), "colorScheme": String("blues"), "projectionType": String("albers-usa")}
us-county-map.yaml::root FixedDomains { x: false, y: false } {"margin": Integer(0), "projectionType": String("albers")}
us-state-map.yaml::root FixedDomains { x: false, y: false } {"margin": Integer(0), "projectionType": String("albers")}
voronoi.yaml::root/vconcat[0] FixedDomains { x: false, y: false } {"inset": Integer(10), "width": Integer(680)}
walmart-openings.yaml::root/vconcat[0] FixedDomains { x: false, y: false } {"margin": Integer(0), "fyLabel": Null, "projectionType": String("albers")}
weather.yaml::root/vconcat[0]/hconcat[0] FixedDomains { x: false, y: false } {"xyDomain": String("Fixed"), "xTickFormat": String("%b"), "colorDomain": Param(ParamRef("domain")), "colorRange": Param(ParamRef("colors")), "rDomain": String("Fixed"), "rRange": Array([Integer(2), Integer(10)]), "width": Integer(680), "height": Integer(300)}
weather.yaml::root/vconcat[1] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "yDomain": Param(ParamRef("domain")), "yLabel": Null, "colorDomain": Param(ParamRef("domain")), "colorRange": Param(ParamRef("colors")), "width": Integer(680)}
wind-map.yaml::root/vconcat[1] FixedDomains { x: false, y: false } {"name": String("wind-map"), "lengthScale": String("identity"), "colorZero": Bool(true), "inset": Integer(10), "aspectRatio": Integer(1), "width": Integer(680)}
window-frame.yaml::root FixedDomains { x: false, y: false } {"yLabel": String("Close"), "width": Integer(680), "height": Integer(200)}
wnba-shots.yaml::root/vconcat[2] FixedDomains { x: false, y: false } {"name": String("shot-chart"), "xAxis": Null, "yAxis": Null, "margin": Integer(5), "xDomain": Array([Integer(0), Integer(50)]), "yDomain": Array([Integer(0), Integer(40)]), "colorDomain": String("Fixed"), "colorScheme": String("YlOrRd"), "colorScale": String("linear"), "colorLabel": String("Avg. Shot Value"), "rScale": String("log"), "rRange": Array([Integer(3), Integer(9)]), "rLabel": String("Shot Count"), "aspectRatio": Integer(1), "width": Integer(510)}
crossfilter.yaml::root/vconcat[0] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Arrival Delay (min)"), "xLabelAnchor": String("center"), "yTickFormat": String("s"), "height": Integer(200)}
crossfilter.yaml::root/vconcat[1] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Departure Time (hour)"), "xLabelAnchor": String("center"), "yTickFormat": String("s"), "height": Integer(200)}
facet-interval.yaml::root/hconcat[0] FixedDomains { x: true, y: true } {"name": String("plot"), "grid": Bool(true), "marginRight": Integer(60), "xDomain": String("Fixed"), "yDomain": String("Fixed"), "fxDomain": String("Fixed"), "fyDomain": String("Fixed"), "fxLabel": Null, "fyLabel": Null}
flights-200k.yaml::root/vconcat[0] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Arrival Delay (min)"), "yTickFormat": String("s"), "width": Integer(600), "height": Integer(200)}
flights-200k.yaml::root/vconcat[1] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Departure Time (hour)"), "yTickFormat": String("s"), "width": Integer(600), "height": Integer(200)}
flights-200k.yaml::root/vconcat[2] FixedDomains { x: true, y: false } {"xDomain": String("Fixed"), "xLabel": String("Flight Distance (miles)"), "yTickFormat": String("s"), "width": Integer(600), "height": Integer(200)}
line.yaml::root FixedDomains { x: false, y: false } {"width": Integer(680), "height": Integer(200)}
overview-detail.yaml::root/vconcat[0] FixedDomains { x: false, y: false } {"width": Integer(680), "height": Integer(200)}
overview-detail.yaml::root/vconcat[1] FixedDomains { x: false, y: true } {"yDomain": String("Fixed"), "width": Integer(680), "height": Integer(200)}
seattle-temp.yaml::root FixedDomains { x: false, y: false } {"xTickFormat": String("%b"), "yLabel": String("Temperature Range (°C)"), "width": Integer(680), "height": Integer(300)}
sorted-bars.yaml::root/vconcat[2] FixedDomains { x: false, y: false } {"xLabel": String("Gold Medals"), "yLabel": String("Nationality"), "yLabelAnchor": String("top"), "marginTop": Integer(15)}"#;

        let vendored =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/mosaic-specs/yaml");
        let curated = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../brightfield-conformance/vendor/curated/yaml");

        let mut lines = Vec::new();
        for dir in [vendored, curated] {
            let mut entries: Vec<_> = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml"))
                .collect();
            entries.sort();
            for path in entries {
                let src =
                    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
                let Ok(parsed) = parse_spec(&src, Format::Yaml) else {
                    continue; // corpus_totality is the gate for parse failures
                };
                if !parsed.spec.plot_defaults.is_empty() {
                    continue; // this card's whole point is that these DO change
                }
                for (at, plot) in collect_plot_nodes(&parsed.spec) {
                    lines.push(format!(
                        "{}::{at} {:?} {:?}",
                        path.file_name().unwrap().to_str().unwrap(),
                        resolve_fixed_domains(plot),
                        plot.attributes
                    ));
                }
            }
        }
        let actual = lines.join("\n");
        assert_eq!(
            actual, BASELINE,
            "a spec declaring no plotDefaults resolved differently than it did \
             before this change; the merge should be a no-op on an empty bag"
        );
    }
}
