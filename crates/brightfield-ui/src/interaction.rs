//! Interaction state — tracks brush rect and hovered point for overlay
//! rendering without triggering DuckDB re-query.
//!
//! Two-tier interaction model:
//! - Immediate: overlay renders during drag (brush rect, highlight) — pure GPU, no I/O
//! - Deferred: DuckDB re-query fires on brush release via session.update_param()

use std::time::{Duration, Instant};

use kurbo::{Affine, Point, Rect};
use peniko::{Color, Fill};
use vello::Scene;

use brightfield_render::nearest::NearestHit;
use brightfield_render::scale::ViewExtent;
use brightfield_spec::vocab::InteractorKind;

/// Current interaction mode.
#[derive(Debug, Clone)]
pub enum InteractionState {
    /// No active interaction.
    Idle,
    /// User is dragging a brush selection.
    Brushing {
        /// Start point of the brush in chart coordinates.
        start: Point,
        /// Current drag point in chart coordinates.
        current: Point,
    },
    /// User is hovering over a data point.
    Hovering {
        /// The hovered point in chart coordinates.
        point: Point,
        /// Resolved nearest data point (if within max distance).
        nearest: Option<NearestHit>,
    },
    /// A committed interval selection that PERSISTS after the drag releases
    /// (Mosaic / Vega-Lite fidelity): the rectangle stays drawn until Esc, a
    /// click, or a new brush clears it. Same chart-local coordinates as
    /// `Brushing`; the dispatched data predicate rides the coordinator.
    Selected {
        /// Start corner of the committed brush, in chart coordinates.
        start: Point,
        /// Opposite corner, in chart coordinates.
        current: Point,
    },
    /// A persisted `Selected` rectangle under active direct manipulation:
    /// the pointer grabbed `region` at `origin` and `{start, current}`
    /// is the LIVE moved/resized rectangle. `anchor` is the rect at grab time,
    /// so the release can tell a real move/resize from a zero-delta click (a
    /// click on the rect) and re-dispatch only the former. Renders exactly like
    /// `Selected` (the moved rect); the grab metadata never reaches the scene.
    Dragging {
        /// Which region of the rect was grabbed (interior / edge / corner).
        region: BrushRegion,
        /// Pointer position at grab time, in chart coordinates.
        origin: Point,
        /// Live moved/resized corner (min/max normalised with `current`).
        start: Point,
        /// Live moved/resized opposite corner.
        current: Point,
        /// The rectangle at grab time — the reference the release compares the
        /// moved rect against to detect a zero-delta (inert) grab.
        anchor: Rect,
    },
}

/// The classification of a pointer position relative to a persisted `Selected`
/// brush rectangle — the single source of truth for BOTH the
/// paint-phase cursor style and the `pointer_down` grab decision + the
/// subsequent move/resize transform. Computed by the gpui-free classifier
/// [`brush_region`] with a handle tolerance band; `Corner` takes precedence
/// over `Edge`, `Edge` over `Interior`, and anything beyond `tol` outside the
/// rect is `Outside`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushRegion {
    /// Beyond the handle band on every side — not over the rect.
    Outside,
    /// Over the rect's interior — a grab translates the whole rect.
    Interior,
    /// Over one edge's handle band — a grab resizes that side.
    Edge(BrushEdge),
    /// Over one corner's handle band — a grab resizes two sides.
    Corner(BrushCorner),
}

/// One resizable edge of a brush rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushEdge {
    /// The `x0` (left) edge.
    Left,
    /// The `x1` (right) edge.
    Right,
    /// The `y0` (top) edge.
    Top,
    /// The `y1` (bottom) edge.
    Bottom,
}

/// One resizable corner of a brush rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushCorner {
    /// The `(x0, y0)` corner.
    TopLeft,
    /// The `(x1, y0)` corner.
    TopRight,
    /// The `(x0, y1)` corner.
    BottomLeft,
    /// The `(x1, y1)` corner.
    BottomRight,
}

/// The action a `pointer_down` resolves to over a plot, decided by
/// the gpui-free [`InteractionState::resolve_press`] resolver: grab a hit on
/// the persisted `Selected` rect (resolved BEFORE the plot-contains check, so a
/// handle in the inset-band overhang above `plot_area.y0` still grabs), else
/// start a fresh brush inside the plot, else ignore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerAction {
    /// The press hit the persisted `Selected` rect — enter a move/resize grab of
    /// the classified region, PRESERVING the rect (never wiping it).
    Grab(BrushRegion),
    /// The press is inside the plot but Outside the rect — start (or replace
    /// with) a fresh brush; over a persisted rect this is the click-retract path.
    StartBrush,
    /// The press is Outside the rect AND outside the plot — do nothing.
    Ignore,
}

/// Handle tolerance band (px) for [`brush_region`]: how far from an edge/corner
/// a pointer still grabs that handle (interior beyond the band = translate). The
/// `/orb:design` ~4-6px call; 6px gives a comfortable grab target without
/// swallowing a thin rect's interior.
pub const HANDLE_TOL: f64 = 6.0;

/// Normalise two corners into a `Rect` with `x0 <= x1`, `y0 <= y1`.
fn norm_rect(a: Point, b: Point) -> Rect {
    Rect::new(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
}

/// Classify a local-space pointer over a `Selected` rect into
/// interior / edge / corner / outside using a handle tolerance band `tol`.
/// Renderer-free and gpui-free. `Corner` precedence over
/// `Edge` over `Interior`; a point farther than `tol` outside any side is
/// `Outside`. The region hit is resolved BEFORE any plot-containment check so a
/// handle in the rect's inset-band overhang still grabs.
#[must_use]
pub fn brush_region(local: Point, rect: Rect, tol: f64) -> BrushRegion {
    let rect = norm_rect(Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y1));
    // Beyond the tolerance-expanded rect on any side → Outside.
    if local.x < rect.x0 - tol
        || local.x > rect.x1 + tol
        || local.y < rect.y0 - tol
        || local.y > rect.y1 + tol
    {
        return BrushRegion::Outside;
    }
    let near_left = (local.x - rect.x0).abs() <= tol;
    let near_right = (local.x - rect.x1).abs() <= tol;
    let near_top = (local.y - rect.y0).abs() <= tol;
    let near_bottom = (local.y - rect.y1).abs() <= tol;
    // Corner over Edge over Interior. (For a rect thinner than the band a mid
    // point can be near two opposite sides; it resolves to a corner, which is a
    // harmless degenerate case.)
    match (near_left, near_right, near_top, near_bottom) {
        (true, _, true, _) => BrushRegion::Corner(BrushCorner::TopLeft),
        (_, true, true, _) => BrushRegion::Corner(BrushCorner::TopRight),
        (true, _, _, true) => BrushRegion::Corner(BrushCorner::BottomLeft),
        (_, true, _, true) => BrushRegion::Corner(BrushCorner::BottomRight),
        (true, _, _, _) => BrushRegion::Edge(BrushEdge::Left),
        (_, true, _, _) => BrushRegion::Edge(BrushEdge::Right),
        (_, _, true, _) => BrushRegion::Edge(BrushEdge::Top),
        (_, _, _, true) => BrushRegion::Edge(BrushEdge::Bottom),
        _ => BrushRegion::Interior,
    }
}

/// Translate a whole rect by `(dx, dy)`, CLAMPING THE TRANSLATION (not each
/// corner) so the rect keeps its SIZE until it hits `frame`, then slides along
/// it. Clamping each corner independently would shrink the
/// rect at the frame edge; clamping the translation preserves it.
#[must_use]
pub fn translate_brush(rect: Rect, dx: f64, dy: f64, frame: Rect) -> Rect {
    let rect = norm_rect(Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y1));
    // Translation range keeping the rect inside the frame. When the rect is
    // wider/taller than the frame the range inverts — pin that axis (no slide).
    let clamp = |d: f64, lo: f64, hi: f64| if lo <= hi { d.clamp(lo, hi) } else { 0.0 };
    let cdx = clamp(dx, frame.x0 - rect.x0, frame.x1 - rect.x1);
    let cdy = clamp(dy, frame.y0 - rect.y0, frame.y1 - rect.y1);
    Rect::new(rect.x0 + cdx, rect.y0 + cdy, rect.x1 + cdx, rect.y1 + cdy)
}

/// Resize a rect by moving the grabbed side(s) of an `Edge`/`Corner` region to
/// `pointer` (clamped to `frame`); the opposite side stays pinned and the result
/// is re-normalised so the rect NEVER inverts when the pointer crosses past the
/// pinned side. An `Interior`/`Outside` region is not a
/// resize and returns the rect unchanged.
#[must_use]
pub fn resize_brush(rect: Rect, region: BrushRegion, pointer: Point, frame: Rect) -> Rect {
    let rect = norm_rect(Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y1));
    let px = pointer.x.clamp(frame.x0, frame.x1);
    let py = pointer.y.clamp(frame.y0, frame.y1);
    let (mut x0, mut y0, mut x1, mut y1) = (rect.x0, rect.y0, rect.x1, rect.y1);
    match region {
        BrushRegion::Edge(BrushEdge::Left) => x0 = px,
        BrushRegion::Edge(BrushEdge::Right) => x1 = px,
        BrushRegion::Edge(BrushEdge::Top) => y0 = py,
        BrushRegion::Edge(BrushEdge::Bottom) => y1 = py,
        BrushRegion::Corner(BrushCorner::TopLeft) => {
            x0 = px;
            y0 = py;
        }
        BrushRegion::Corner(BrushCorner::TopRight) => {
            x1 = px;
            y0 = py;
        }
        BrushRegion::Corner(BrushCorner::BottomLeft) => {
            x0 = px;
            y1 = py;
        }
        BrushRegion::Corner(BrushCorner::BottomRight) => {
            x1 = px;
            y1 = py;
        }
        BrushRegion::Interior | BrushRegion::Outside => {}
    }
    // Re-normalise: dragging a side past the pinned one flips which coordinate is
    // min/max, but the rect stays non-inverted (the pinned side is still a bound).
    Rect::new(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1))
}

/// Brush overlay colour (semi-transparent blue).
const BRUSH_COLOUR: Color = Color::new([0.306, 0.475, 0.655, 0.251]);

/// Brush border colour.
const BRUSH_BORDER_COLOUR: Color = Color::new([0.306, 0.475, 0.655, 0.753]);

/// Committed-selection fill — semi-transparent grey, distinct from the active
/// blue drag so a persisted selection reads as settled (Vega-Lite convention).
const SELECTED_COLOUR: Color = Color::new([0.5, 0.5, 0.5, 0.18]);

/// Committed-selection border.
const SELECTED_BORDER_COLOUR: Color = Color::new([0.42, 0.42, 0.42, 0.6]);

/// Hover highlight radius.
const HOVER_RADIUS: f64 = 8.0;

/// Hover highlight colour (semi-transparent orange).
const HOVER_COLOUR: Color = Color::new([0.949, 0.557, 0.169, 0.376]);

impl InteractionState {
    /// Begin a brush at the given point.
    pub fn start_brush(point: Point) -> Self {
        Self::Brushing {
            start: point,
            current: point,
        }
    }

    /// Update the brush's current drag position.
    pub fn update_brush(&mut self, current: Point) {
        if let Self::Brushing {
            current: ref mut c, ..
        } = self
        {
            *c = current;
        }
    }

    /// Get the brush rectangle (if brushing).
    pub fn brush_rect(&self) -> Option<Rect> {
        match self {
            Self::Brushing { start, current } => {
                let x0 = start.x.min(current.x);
                let y0 = start.y.min(current.y);
                let x1 = start.x.max(current.x);
                let y1 = start.y.max(current.y);
                Some(Rect::new(x0, y0, x1, y1))
            }
            _ => None,
        }
    }

    /// The rectangle of any rect-bearing state — the active `Brushing` drag, the
    /// persisted `Selected` overlay, or an in-flight `Dragging` — as a min/max
    /// normalised `Rect`; `None` for `Idle`/`Hovering`.
    /// Unlike [`brush_rect`](Self::brush_rect) (Brushing only), this ALSO
    /// exposes the persisted `Selected` rect, so the grab hit-test
    /// ([`brush_region`]) and the `pointer_down` decision
    /// ([`resolve_press`](Self::resolve_press)) can key off it — today's
    /// `brush_rect` returning `None` for `Selected` made any such hit-test
    /// silently never fire.
    pub fn selected_rect(&self) -> Option<Rect> {
        match self {
            Self::Brushing { start, current }
            | Self::Selected { start, current }
            | Self::Dragging { start, current, .. } => Some(norm_rect(*start, *current)),
            Self::Idle | Self::Hovering { .. } => None,
        }
    }

    /// Resolve a pointer press over this state into a [`PointerAction`]:
    /// a hit on the persisted `Selected` rect (region !=
    /// `Outside`) is a `Grab` — resolved BEFORE the plot-contains gate, so a
    /// handle in the rect's inset-band overhang above `plot_area.y0` still grabs;
    /// otherwise a press inside the plot `StartBrush`es (clearing/replacing any
    /// persisted rect), and a press Outside the rect AND outside the plot is
    /// `Ignore`. Pure — the shim consumes it, so the ordering is unit-provable.
    #[must_use]
    pub fn resolve_press(&self, local: Point, plot_contains: bool, tol: f64) -> PointerAction {
        // Grab-before-brush: the Selected-rect region hit STRICTLY precedes the
        // plot-containment check (the anti-wipe invariant).
        if let Self::Selected { .. } = self {
            if let Some(rect) = self.selected_rect() {
                let region = brush_region(local, rect, tol);
                if region != BrushRegion::Outside {
                    return PointerAction::Grab(region);
                }
            }
        }
        if plot_contains {
            PointerAction::StartBrush
        } else {
            PointerAction::Ignore
        }
    }

    /// Apply a resolved press: `Grab` enters `Dragging`
    /// PRESERVING the rect corners (never a wipe to zero-area); `StartBrush`
    /// begins a fresh `Brushing` at the press (replacing any `Selected`);
    /// `Ignore` leaves the state unchanged. `local` is the press point.
    #[must_use]
    pub fn on_press(self, action: PointerAction, local: Point) -> Self {
        match action {
            PointerAction::Grab(region) => match self.selected_rect() {
                Some(rect) => Self::Dragging {
                    region,
                    origin: local,
                    start: Point::new(rect.x0, rect.y0),
                    current: Point::new(rect.x1, rect.y1),
                    anchor: rect,
                },
                None => self,
            },
            PointerAction::StartBrush => Self::start_brush(local),
            PointerAction::Ignore => self,
        }
    }

    /// Apply a pointer move during a grab: transform the
    /// `Dragging` rect to `pointer` — an `Interior` grab translates the anchor by
    /// the pointer delta, an `Edge`/`Corner` grab resizes the grabbed side(s) —
    /// each clamped to `frame`. Non-`Dragging` states pass through unchanged.
    #[must_use]
    pub fn on_grab_move(self, pointer: Point, frame: Rect) -> Self {
        if let Self::Dragging {
            region,
            origin,
            anchor,
            ..
        } = self
        {
            let moved = match region {
                BrushRegion::Interior => {
                    translate_brush(anchor, pointer.x - origin.x, pointer.y - origin.y, frame)
                }
                BrushRegion::Edge(_) | BrushRegion::Corner(_) => {
                    resize_brush(anchor, region, pointer, frame)
                }
                // Not a live-manipulable region; hold the rect.
                BrushRegion::Outside => anchor,
            };
            Self::Dragging {
                region,
                origin,
                start: Point::new(moved.x0, moved.y0),
                current: Point::new(moved.x1, moved.y1),
                anchor,
            }
        } else {
            self
        }
    }

    /// Finalise a grab: a `Dragging` collapses to a
    /// persisted `Selected` at its current (moved/resized) rect — the end-state
    /// the release re-dispatches from. Non-`Dragging` states pass through.
    #[must_use]
    pub fn on_grab_release(self) -> Self {
        if let Self::Dragging { start, current, .. } = self {
            Self::Selected { start, current }
        } else {
            self
        }
    }

    /// Cancel an in-flight grab: a MISSED mouse-up during a
    /// move/resize (a focus steal, or a release outside the window — the normal
    /// release goes through the element's mouse-up listener) reaches only
    /// `pointer_move`, which holds NO coordinator handle and so cannot
    /// re-dispatch the moved range. Collapsing to the moved `Selected` would draw
    /// the grey overlay at the new range while the live filter stayed at the
    /// pre-move range (the silent-no-op class). Instead revert to the
    /// `anchor` (pre-drag) rect — discarding the undispatched move so the overlay
    /// and the filter stay consistent at the already-dispatched pre-drag range,
    /// mirroring the `Brushing` arm's discard-to-`Idle`. Non-`Dragging` passes
    /// through.
    #[must_use]
    pub fn on_grab_cancel(self) -> Self {
        if let Self::Dragging { anchor, .. } = self {
            Self::Selected {
                start: Point::new(anchor.x0, anchor.y0),
                current: Point::new(anchor.x1, anchor.y1),
            }
        } else {
            self
        }
    }

    /// Whether this state carries a persisted (or in-flight) selection overlay an
    /// Esc / cross-filter clear should drop: a committed `Selected`
    /// OR an in-flight `Dragging` — so a clear arriving mid-drag doesn't retract
    /// the filter while leaving the grey overlay drawn (a transient
    /// visual/data mismatch). `Idle`/`Brushing`/`Hovering` carry no such overlay.
    #[must_use]
    pub fn has_persistent_selection(&self) -> bool {
        matches!(self, Self::Selected { .. } | Self::Dragging { .. })
    }

    /// Render the interaction overlay into the scene.
    ///
    /// This is pure rendering — no DuckDB query, no I/O. The overlay
    /// renders immediately during drag at frame rate.
    pub fn render_overlay(&self, scene: &mut Scene) {
        match self {
            Self::Idle => {}
            Self::Brushing { start, current } => {
                let rect = Rect::new(
                    start.x.min(current.x),
                    start.y.min(current.y),
                    start.x.max(current.x),
                    start.y.max(current.y),
                );
                // Semi-transparent fill.
                scene.fill(Fill::NonZero, Affine::IDENTITY, BRUSH_COLOUR, None, &rect);
                // Border stroke.
                let stroke = kurbo::Stroke::new(1.5);
                scene.stroke(&stroke, Affine::IDENTITY, BRUSH_BORDER_COLOUR, None, &rect);
            }
            // A committed selection and an in-flight move/resize render identically
            // (the grey settled rect at its current corners) — the grab metadata
            // never reaches the scene.
            Self::Selected { start, current } | Self::Dragging { start, current, .. } => {
                let rect = Rect::new(
                    start.x.min(current.x),
                    start.y.min(current.y),
                    start.x.max(current.x),
                    start.y.max(current.y),
                );
                scene.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    SELECTED_COLOUR,
                    None,
                    &rect,
                );
                let stroke = kurbo::Stroke::new(1.5);
                scene.stroke(
                    &stroke,
                    Affine::IDENTITY,
                    SELECTED_BORDER_COLOUR,
                    None,
                    &rect,
                );
            }
            Self::Hovering { point, .. } => {
                let circle = kurbo::Circle::new(*point, HOVER_RADIUS);
                scene.fill(Fill::NonZero, Affine::IDENTITY, HOVER_COLOUR, None, &circle);
            }
        }
    }
}

/// Synthesise a pixel-space `Brushing` from a move/resize end-state so the
/// moved cross-filter re-dispatches on release — the SOLE
/// synthesis site of the silent-no-op defence, a pure
/// gpui-free production fn the release path DRIVES.
///
/// `commit_brush` reads ONLY `Brushing`, so a gesture ending in `Dragging`
/// re-dispatches nothing unless converted here. Returns:
/// - `Some(Brushing { new corners })` for a `Dragging` whose rect actually moved
///   from its `anchor` — the corners `invert_pixel_brush` then inverts downstream;
/// - `None` for a zero-delta grab (a click on the rect, the moved rect within
///   [`ZERO_AREA_EPSILON`](crate::chart_view::ZERO_AREA_EPSILON) of the anchor)
///   — so a click never fires a redundant re-query, the selection intact;
/// - `None` for every other state (a persisted `Selected`, a fresh `Brushing`
///   which already dispatches through the unchanged path, `Idle`, `Hovering`) —
///   in particular a persisted `Selected` on an untouched sibling plot yields
///   `None`, so a release there never re-dispatches its selection.
#[must_use]
pub fn redispatch_brushing_from(end_state: &InteractionState) -> Option<InteractionState> {
    match end_state {
        InteractionState::Dragging {
            start,
            current,
            anchor,
            ..
        } => {
            let moved = norm_rect(*start, *current);
            let unmoved = (moved.x0 - anchor.x0).abs() < crate::chart_view::ZERO_AREA_EPSILON
                && (moved.y0 - anchor.y0).abs() < crate::chart_view::ZERO_AREA_EPSILON
                && (moved.x1 - anchor.x1).abs() < crate::chart_view::ZERO_AREA_EPSILON
                && (moved.y1 - anchor.y1).abs() < crate::chart_view::ZERO_AREA_EPSILON;
            if unmoved {
                None
            } else {
                Some(InteractionState::Brushing {
                    start: *start,
                    current: *current,
                })
            }
        }
        _ => None,
    }
}

/// Typed axis-lock and pan/zoom capability derived from `InteractorKind`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavigationConfig {
    /// Whether panning is enabled.
    pub pan: bool,
    /// Whether zooming is enabled.
    pub zoom: bool,
    /// Whether the x-axis is navigable.
    pub x_navigable: bool,
    /// Whether the y-axis is navigable.
    pub y_navigable: bool,
}

impl NavigationConfig {
    /// Derive a `NavigationConfig` from an `InteractorKind`.
    ///
    /// Returns `Some` for the six pan/zoom interactor kinds, `None` for all others.
    pub fn from_interactor_kind(kind: InteractorKind) -> Option<Self> {
        match kind {
            InteractorKind::Pan => Some(Self {
                pan: true,
                zoom: false,
                x_navigable: true,
                y_navigable: true,
            }),
            InteractorKind::PanX => Some(Self {
                pan: true,
                zoom: false,
                x_navigable: true,
                y_navigable: false,
            }),
            InteractorKind::PanY => Some(Self {
                pan: true,
                zoom: false,
                x_navigable: false,
                y_navigable: true,
            }),
            InteractorKind::PanZoom => Some(Self {
                pan: true,
                zoom: true,
                x_navigable: true,
                y_navigable: true,
            }),
            InteractorKind::PanZoomX => Some(Self {
                pan: true,
                zoom: true,
                x_navigable: true,
                y_navigable: false,
            }),
            InteractorKind::PanZoomY => Some(Self {
                pan: true,
                zoom: true,
                x_navigable: false,
                y_navigable: true,
            }),
            _ => None,
        }
    }
}

/// Mutable navigation state for pan/zoom interaction.
#[derive(Debug, Clone)]
pub struct NavigationState {
    /// Current view extent (None = full data extent).
    pub view_extent: ViewExtent,
    /// Navigation configuration (axis lock, pan/zoom capabilities).
    pub config: NavigationConfig,
    /// Timestamp of the last pan/zoom event (for debounce).
    last_event: Option<Instant>,
    /// Debounce duration (default 150ms).
    debounce_duration: Duration,
    /// Whether a re-query is pending (settle not yet fired).
    pub requery_pending: bool,
}

impl NavigationState {
    /// Create a new navigation state with the given config.
    pub fn new(config: NavigationConfig) -> Self {
        Self {
            view_extent: ViewExtent::default(),
            config,
            last_event: None,
            debounce_duration: Duration::from_millis(150),
            requery_pending: false,
        }
    }

    /// Create a navigation state with a custom debounce duration.
    pub fn with_debounce(config: NavigationConfig, debounce: Duration) -> Self {
        Self {
            debounce_duration: debounce,
            ..Self::new(config)
        }
    }

    /// Apply a pan gesture (normalised pixel delta).
    ///
    /// `dx_norm` and `dy_norm` are normalised deltas: `px_delta / range_width`.
    /// Axis lock is respected: non-navigable axes are ignored.
    pub fn apply_pan(
        &mut self,
        dx_norm: f64,
        dy_norm: f64,
        x_domain: (f64, f64),
        y_domain: (f64, f64),
    ) {
        if self.config.pan && self.config.x_navigable {
            let x_span = x_domain.1 - x_domain.0;
            let (x_min, x_max) = self.view_extent.x.unwrap_or(x_domain);
            let new_min = x_min - dx_norm * x_span;
            let new_max = x_max - dx_norm * x_span;
            self.view_extent.x = Some((new_min, new_max));
        }
        if self.config.pan && self.config.y_navigable {
            let y_span = y_domain.1 - y_domain.0;
            let (y_min, y_max) = self.view_extent.y.unwrap_or(y_domain);
            let new_min = y_min - dy_norm * y_span;
            let new_max = y_max - dy_norm * y_span;
            self.view_extent.y = Some((new_min, new_max));
        }
        self.last_event = Some(Instant::now());
        self.requery_pending = true;
    }

    /// Apply a zoom gesture around a cursor position.
    ///
    /// `cursor_norm_x` and `cursor_norm_y` are the cursor's normalised position
    /// within the range [0, 1]. `zoom_factor` > 1.0 zooms in, < 1.0 zooms out.
    pub fn apply_zoom(
        &mut self,
        cursor_norm_x: f64,
        cursor_norm_y: f64,
        zoom_factor: f64,
        x_domain: (f64, f64),
        y_domain: (f64, f64),
    ) {
        if self.config.zoom && self.config.x_navigable {
            let (x_min, x_max) = self.view_extent.x.unwrap_or(x_domain);
            let x_span = x_max - x_min;
            let cursor_data = x_min + cursor_norm_x * x_span;
            let new_span = x_span / zoom_factor;
            let new_min = cursor_data - cursor_norm_x * new_span;
            let new_max = cursor_data + (1.0 - cursor_norm_x) * new_span;
            self.view_extent.x = Some((new_min, new_max));
        }
        if self.config.zoom && self.config.y_navigable {
            let (y_min, y_max) = self.view_extent.y.unwrap_or(y_domain);
            let y_span = y_max - y_min;
            let cursor_data = y_min + cursor_norm_y * y_span;
            let new_span = y_span / zoom_factor;
            let new_min = cursor_data - cursor_norm_y * new_span;
            let new_max = cursor_data + (1.0 - cursor_norm_y) * new_span;
            self.view_extent.y = Some((new_min, new_max));
        }
        self.last_event = Some(Instant::now());
        self.requery_pending = true;
    }

    /// Reset the view extent to None (full data extent).
    pub fn reset(&mut self) {
        if self.config.x_navigable {
            self.view_extent.x = None;
        }
        if self.config.y_navigable {
            self.view_extent.y = None;
        }
        self.last_event = None;
        self.requery_pending = true;
    }

    /// Check if the debounce timer has settled (enough time since last event).
    ///
    /// Returns `true` if a re-query should fire. Calling this also clears the
    /// pending flag.
    pub fn check_settle(&mut self) -> bool {
        if !self.requery_pending {
            return false;
        }
        if let Some(last) = self.last_event {
            if last.elapsed() >= self.debounce_duration {
                self.requery_pending = false;
                return true;
            }
        }
        false
    }

    /// Get the current view extent (for passing to build_chart_scene).
    pub fn current_extent(&self) -> Option<&ViewExtent> {
        if self.view_extent.x.is_some() || self.view_extent.y.is_some() {
            Some(&self.view_extent)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    #[test]
    fn brush_state_tracks_rect() {
        let mut state = InteractionState::start_brush(Point::new(10.0, 20.0));
        state.update_brush(Point::new(100.0, 200.0));

        let rect = state.brush_rect().expect("should have brush rect");
        assert!((rect.x0 - 10.0).abs() < f64::EPSILON);
        assert!((rect.y0 - 20.0).abs() < f64::EPSILON);
        assert!((rect.x1 - 100.0).abs() < f64::EPSILON);
        assert!((rect.y1 - 200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn brush_overlay_renders_without_query() {
        let state = InteractionState::Brushing {
            start: Point::new(10.0, 20.0),
            current: Point::new(100.0, 200.0),
        };

        let mut scene = Scene::new();
        state.render_overlay(&mut scene);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "brush overlay should produce scene content"
        );
        // Key assertion: this test proves overlay renders without any engine
        // dependency — no DuckDB, no execute_mark call. Pure scene rendering.
    }

    #[test]
    fn hover_overlay_renders() {
        let state = InteractionState::Hovering {
            point: Point::new(50.0, 50.0),
            nearest: None,
        };

        let mut scene = Scene::new();
        state.render_overlay(&mut scene);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "hover overlay should produce scene content"
        );
    }

    // --- NavigationConfig ---

    #[test]
    fn pan_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::Pan).unwrap();
        assert!(cfg.pan);
        assert!(!cfg.zoom);
        assert!(cfg.x_navigable);
        assert!(cfg.y_navigable);
    }

    #[test]
    fn pan_x_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::PanX).unwrap();
        assert!(cfg.pan);
        assert!(!cfg.zoom);
        assert!(cfg.x_navigable);
        assert!(!cfg.y_navigable);
    }

    #[test]
    fn pan_y_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::PanY).unwrap();
        assert!(cfg.pan);
        assert!(!cfg.zoom);
        assert!(!cfg.x_navigable);
        assert!(cfg.y_navigable);
    }

    #[test]
    fn pan_zoom_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::PanZoom).unwrap();
        assert!(cfg.pan);
        assert!(cfg.zoom);
        assert!(cfg.x_navigable);
        assert!(cfg.y_navigable);
    }

    #[test]
    fn pan_zoom_x_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::PanZoomX).unwrap();
        assert!(cfg.pan);
        assert!(cfg.zoom);
        assert!(cfg.x_navigable);
        assert!(!cfg.y_navigable);
    }

    #[test]
    fn pan_zoom_y_config() {
        let cfg = NavigationConfig::from_interactor_kind(InteractorKind::PanZoomY).unwrap();
        assert!(cfg.pan);
        assert!(cfg.zoom);
        assert!(!cfg.x_navigable);
        assert!(cfg.y_navigable);
    }

    #[test]
    fn non_navigation_returns_none() {
        assert!(NavigationConfig::from_interactor_kind(InteractorKind::Nearest).is_none());
        assert!(NavigationConfig::from_interactor_kind(InteractorKind::Highlight).is_none());
        assert!(NavigationConfig::from_interactor_kind(InteractorKind::Toggle).is_none());
    }

    // --- Pan gesture handler ---

    #[test]
    fn pan_x_only() {
        let config = NavigationConfig {
            pan: true,
            zoom: false,
            x_navigable: true,
            y_navigable: false,
        };
        let mut state = NavigationState::new(config);
        let x_domain = (0.0, 100.0);
        let y_domain = (0.0, 50.0);

        // Pan right by 10% of the range
        state.apply_pan(0.1, 0.1, x_domain, y_domain);

        // X should have shifted
        assert!(state.view_extent.x.is_some());
        let (x_min, x_max) = state.view_extent.x.unwrap();
        assert!((x_min - (-10.0)).abs() < f64::EPSILON);
        assert!((x_max - 90.0).abs() < f64::EPSILON);

        // Y should be unchanged (axis locked)
        assert!(state.view_extent.y.is_none());
    }

    #[test]
    fn pan_both_axes() {
        let config = NavigationConfig {
            pan: true,
            zoom: false,
            x_navigable: true,
            y_navigable: true,
        };
        let mut state = NavigationState::new(config);
        state.apply_pan(0.2, 0.3, (0.0, 100.0), (0.0, 50.0));

        assert!(state.view_extent.x.is_some());
        assert!(state.view_extent.y.is_some());
        let (x_min, _) = state.view_extent.x.unwrap();
        let (y_min, _) = state.view_extent.y.unwrap();
        assert!((x_min - (-20.0)).abs() < f64::EPSILON);
        assert!((y_min - (-15.0)).abs() < f64::EPSILON);
    }

    // --- Zoom gesture handler ---

    #[test]
    fn zoom_in_center_narrows_symmetrically() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: true,
        };
        let mut state = NavigationState::new(config);

        // Zoom 2x at center (cursor_norm = 0.5)
        state.apply_zoom(0.5, 0.5, 2.0, (0.0, 100.0), (0.0, 50.0));

        let (x_min, x_max) = state.view_extent.x.unwrap();
        assert!((x_min - 25.0).abs() < f64::EPSILON);
        assert!((x_max - 75.0).abs() < f64::EPSILON);

        let (y_min, y_max) = state.view_extent.y.unwrap();
        assert!((y_min - 12.5).abs() < f64::EPSILON);
        assert!((y_max - 37.5).abs() < f64::EPSILON);
    }

    #[test]
    fn zoom_y_locked() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: false,
        };
        let mut state = NavigationState::new(config);
        state.apply_zoom(0.5, 0.5, 2.0, (0.0, 100.0), (0.0, 50.0));

        assert!(state.view_extent.x.is_some());
        assert!(state.view_extent.y.is_none());
    }

    // --- Reset ---

    #[test]
    fn reset_clears_extent() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: true,
        };
        let mut state = NavigationState::new(config);
        state.view_extent.x = Some((25.0, 75.0));
        state.view_extent.y = Some((10.0, 40.0));

        state.reset();

        assert!(state.view_extent.x.is_none());
        assert!(state.view_extent.y.is_none());
    }

    // --- Debounce ---

    #[test]
    fn debounce_not_settled_immediately() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: true,
        };
        let mut state = NavigationState::with_debounce(config, Duration::from_millis(100));
        state.apply_zoom(0.5, 0.5, 2.0, (0.0, 100.0), (0.0, 50.0));

        // Immediately after zoom, settle should not fire
        assert!(!state.check_settle());
        assert!(state.requery_pending);
    }

    #[test]
    fn debounce_settles_after_duration() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: true,
        };
        // Use 1ms debounce for test speed
        let mut state = NavigationState::with_debounce(config, Duration::from_millis(1));
        state.apply_zoom(0.5, 0.5, 2.0, (0.0, 100.0), (0.0, 50.0));

        // Wait for debounce
        std::thread::sleep(Duration::from_millis(5));

        assert!(state.check_settle());
        // After settle, pending should be cleared
        assert!(!state.requery_pending);
    }

    #[test]
    fn debounce_resets_on_new_event() {
        let config = NavigationConfig {
            pan: true,
            zoom: true,
            x_navigable: true,
            y_navigable: true,
        };
        let mut state = NavigationState::with_debounce(config, Duration::from_millis(50));
        state.apply_zoom(0.5, 0.5, 2.0, (0.0, 100.0), (0.0, 50.0));

        // Fire a new event — timer should reset
        state.apply_pan(0.1, 0.0, (0.0, 100.0), (0.0, 50.0));

        // Immediately after new event, not settled
        assert!(!state.check_settle());
    }

    #[test]
    fn idle_overlay_is_empty() {
        let state = InteractionState::Idle;
        let mut scene = Scene::new();
        state.render_overlay(&mut scene);

        let encoding = scene.encoding();
        assert_eq!(
            encoding.path_tags.len(),
            0,
            "idle state should not produce overlay content"
        );
    }

    // --- Hovering with NearestHit ---

    #[test]
    fn hovering_with_nearest_hit() {
        use brightfield_render::nearest::NearestHit;

        let hit = NearestHit {
            row: 3,
            point: kurbo::Point::new(100.0, 200.0),
            distance: 5.0,
        };
        let state = InteractionState::Hovering {
            point: Point::new(102.0, 198.0),
            nearest: Some(hit.clone()),
        };

        match &state {
            InteractionState::Hovering { nearest, .. } => {
                let hit = nearest.as_ref().unwrap();
                assert_eq!(hit.row, 3);
                assert!((hit.distance - 5.0).abs() < f64::EPSILON);
            }
            _ => panic!("expected Hovering state"),
        }
    }

    #[test]
    fn hovering_without_nearest_backward_compatible() {
        let state = InteractionState::Hovering {
            point: Point::new(50.0, 50.0),
            nearest: None,
        };
        // Should still render overlay without panicking
        let mut scene = Scene::new();
        state.render_overlay(&mut scene);

        let encoding = scene.encoding();
        assert!(
            !encoding.path_tags.is_empty(),
            "hovering without nearest should still render highlight circle"
        );
    }

    // --- (draggable / resizable persisted brush) ---

    const TOL: f64 = HANDLE_TOL;

    /// the gpui-free brush-region classifier resolves a pointer over a
    /// Selected rect into Interior / Edge / Corner / Outside with corner-over-edge
    /// precedence and a `tol` handle band.
    #[test]
    fn brush_region_classifier() {
        let rect = Rect::new(100.0, 100.0, 200.0, 150.0);

        // Centre → Interior.
        assert_eq!(
            brush_region(Point::new(150.0, 125.0), rect, TOL),
            BrushRegion::Interior
        );

        // Each corner (within tol) → the matching Corner.
        assert_eq!(
            brush_region(Point::new(100.0, 100.0), rect, TOL),
            BrushRegion::Corner(BrushCorner::TopLeft)
        );
        assert_eq!(
            brush_region(Point::new(200.0, 100.0), rect, TOL),
            BrushRegion::Corner(BrushCorner::TopRight)
        );
        assert_eq!(
            brush_region(Point::new(100.0, 150.0), rect, TOL),
            BrushRegion::Corner(BrushCorner::BottomLeft)
        );
        assert_eq!(
            brush_region(Point::new(200.0, 150.0), rect, TOL),
            BrushRegion::Corner(BrushCorner::BottomRight)
        );

        // Each edge midpoint (within tol, away from a corner) → the matching Edge.
        assert_eq!(
            brush_region(Point::new(150.0, 100.0), rect, TOL),
            BrushRegion::Edge(BrushEdge::Top)
        );
        assert_eq!(
            brush_region(Point::new(150.0, 150.0), rect, TOL),
            BrushRegion::Edge(BrushEdge::Bottom)
        );
        assert_eq!(
            brush_region(Point::new(100.0, 125.0), rect, TOL),
            BrushRegion::Edge(BrushEdge::Left)
        );
        assert_eq!(
            brush_region(Point::new(200.0, 125.0), rect, TOL),
            BrushRegion::Edge(BrushEdge::Right)
        );

        // Beyond tol on every side → Outside.
        assert_eq!(
            brush_region(Point::new(300.0, 300.0), rect, TOL),
            BrushRegion::Outside
        );
        assert_eq!(
            brush_region(Point::new(150.0, 50.0), rect, TOL),
            BrushRegion::Outside
        );
        // Just inside the band above the top edge is still the Top edge, not Outside.
        assert_eq!(
            brush_region(Point::new(150.0, 100.0 - TOL + 0.5), rect, TOL),
            BrushRegion::Edge(BrushEdge::Top)
        );

        // A point in BOTH a corner's and an edge's band resolves the Corner
        // (precedence): near the top-left corner but slightly down the left edge.
        assert_eq!(
            brush_region(Point::new(101.0, 103.0), rect, TOL),
            BrushRegion::Corner(BrushCorner::TopLeft)
        );
    }

    /// the Selected-aware rect accessor returns Some(normalised) for
    /// Selected / Brushing / Dragging and None for Idle / Hovering — while the
    /// legacy `brush_rect` stays Brushing-only (the regression).
    #[test]
    fn selected_rect_accessor() {
        // Selected → Some, min/max normalised (x0<x1, y0<y1) even given reversed
        // corners.
        let sel = InteractionState::Selected {
            start: Point::new(100.0, 200.0),
            current: Point::new(10.0, 20.0),
        };
        let r = sel.selected_rect().expect("Selected exposes its rect");
        assert!(r.x0 < r.x1 && r.y0 < r.y1);
        assert!((r.x0 - 10.0).abs() < f64::EPSILON && (r.x1 - 100.0).abs() < f64::EPSILON);

        // Brushing → Some (regression: unchanged), and brush_rect still agrees.
        let brushing = InteractionState::Brushing {
            start: Point::new(10.0, 20.0),
            current: Point::new(100.0, 200.0),
        };
        assert!(brushing.selected_rect().is_some());
        assert!(
            brushing.brush_rect().is_some(),
            "brush_rect Brushing semantics unchanged"
        );

        // Dragging → Some (the live moved rect).
        let dragging = InteractionState::Dragging {
            region: BrushRegion::Interior,
            origin: Point::new(0.0, 0.0),
            start: Point::new(10.0, 20.0),
            current: Point::new(100.0, 200.0),
            anchor: Rect::new(10.0, 20.0, 100.0, 200.0),
        };
        assert!(dragging.selected_rect().is_some());

        // Idle / Hovering → None (both accessors). brush_rect stays None for
        // Selected (the documented gap — a hit-test on it never fired).
        assert!(InteractionState::Idle.selected_rect().is_none());
        assert!(InteractionState::Hovering {
            point: Point::new(1.0, 1.0),
            nearest: None
        }
        .selected_rect()
        .is_none());
        assert!(
            sel.brush_rect().is_none(),
            "legacy brush_rect is still None for Selected"
        );
    }

    /// the translate transform moves all four corners by the delta,
    /// preserving SIZE, and clamps the TRANSLATION (not each corner) at the frame.
    #[test]
    fn translate_transform() {
        let rect = Rect::new(100.0, 100.0, 200.0, 150.0);
        let frame = Rect::new(0.0, 0.0, 640.0, 480.0);

        // Well inside → every corner shifts by exactly (dx, dy); size unchanged.
        let moved = translate_brush(rect, 30.0, 20.0, frame);
        assert_eq!(moved, Rect::new(130.0, 120.0, 230.0, 170.0));
        assert!((moved.width() - rect.width()).abs() < f64::EPSILON);
        assert!((moved.height() - rect.height()).abs() < f64::EPSILON);

        // A translation that would push past the frame butts against it, size
        // PRESERVED (not shrunk): dx = 1000 → x1 pinned at 640, width still 100.
        let butted = translate_brush(rect, 1000.0, 0.0, frame);
        assert!((butted.x1 - 640.0).abs() < f64::EPSILON);
        assert!(
            (butted.width() - 100.0).abs() < f64::EPSILON,
            "size preserved at the frame edge"
        );

        // Zero delta → identity.
        assert_eq!(translate_brush(rect, 0.0, 0.0, frame), rect);
    }

    /// the resize transform moves only the grabbed side(s); the
    /// opposite side stays pinned; the result never inverts.
    #[test]
    fn resize_transform() {
        let rect = Rect::new(100.0, 100.0, 200.0, 150.0);
        let frame = Rect::new(0.0, 0.0, 640.0, 480.0);

        // Right edge dragged right → widens x1 only.
        let r = resize_brush(
            rect,
            BrushRegion::Edge(BrushEdge::Right),
            Point::new(260.0, 400.0),
            frame,
        );
        assert_eq!(r, Rect::new(100.0, 100.0, 260.0, 150.0));

        // TopLeft corner → moves x0, y0 only (x1, y1 pinned).
        let r = resize_brush(
            rect,
            BrushRegion::Corner(BrushCorner::TopLeft),
            Point::new(80.0, 90.0),
            frame,
        );
        assert_eq!(r, Rect::new(80.0, 90.0, 200.0, 150.0));

        // Right edge dragged LEFT past x0 → normalised, x0<x1, the pinned side
        // (old x0 = 100) is now the right bound.
        let r = resize_brush(
            rect,
            BrushRegion::Edge(BrushEdge::Right),
            Point::new(50.0, 125.0),
            frame,
        );
        assert!(r.x0 < r.x1);
        assert!(
            (r.x1 - 100.0).abs() < f64::EPSILON,
            "the pinned side is still old x0"
        );
        assert!((r.x0 - 50.0).abs() < f64::EPSILON);

        // A corner dragged past its diagonal opposite normalises likewise.
        let r = resize_brush(
            rect,
            BrushRegion::Corner(BrushCorner::BottomRight),
            Point::new(50.0, 50.0),
            frame,
        );
        assert_eq!(r, Rect::new(50.0, 50.0, 100.0, 100.0));
    }

    /// the grab-before-brush resolver maps (local, contains) to
    /// Grab / StartBrush / Ignore, resolving a Selected-rect hit BEFORE the
    /// plot-contains check (case b — the inset-band overhang), and a Grab carries
    /// the pre-press rect intact (the anti-wipe invariant).
    #[test]
    fn pointer_down_grab_resolver() {
        let sel = InteractionState::Selected {
            start: Point::new(100.0, 100.0),
            current: Point::new(200.0, 150.0),
        };

        // (a) A press inside the rect → Grab(region); the resulting sub-state
        // preserves the rect corners (anti-wipe).
        let action = sel.resolve_press(Point::new(150.0, 125.0), true, TOL);
        assert_eq!(action, PointerAction::Grab(BrushRegion::Interior));
        let grabbed = sel.clone().on_press(action, Point::new(150.0, 125.0));
        let r = grabbed
            .selected_rect()
            .expect("the grab preserves the rect");
        assert_eq!(
            r,
            Rect::new(100.0, 100.0, 200.0, 150.0),
            "a Grab never wipes the rect"
        );

        // (b) A press on a Selected rect whose TOP EDGE is above plot_area.y0 —
        // in the inset band where contains() is FALSE — still Grabs, NOT Ignore:
        // the region hit is resolved BEFORE the plot-containment check.
        let band = InteractionState::Selected {
            start: Point::new(100.0, 20.0), // top edge at y=20
            current: Point::new(200.0, 120.0),
        };
        // plot_contains = false (the press sits in the inset-band overhang).
        let action = band.resolve_press(Point::new(150.0, 20.0), false, TOL);
        assert!(
            matches!(action, PointerAction::Grab(_)),
            "a handle in the inset-band overhang grabs, not Ignore: {action:?}"
        );

        // (c) A press inside the plot but Outside the rect → StartBrush (clears /
        // replaces the persisted rect on commit).
        assert_eq!(
            sel.resolve_press(Point::new(400.0, 300.0), true, TOL),
            PointerAction::StartBrush
        );

        // (d) A press with no Selected rect inside the plot → StartBrush.
        assert_eq!(
            InteractionState::Idle.resolve_press(Point::new(400.0, 300.0), true, TOL),
            PointerAction::StartBrush
        );

        // (e) A press Outside the rect AND outside the plot → Ignore.
        assert_eq!(
            sel.resolve_press(Point::new(5.0, 5.0), false, TOL),
            PointerAction::Ignore
        );
    }

    /// the pure release re-dispatch resolver — a MOVED/RESIZED Dragging
    /// end-state → Some(Brushing) with the new corners; a zero-delta grab and
    /// every non-grab state → None. Driven from the translate/resize transforms, not
    /// hand-built corners.
    #[test]
    fn release_redispatch_resolves_moved_and_resized_grabs() {
        let anchor = Rect::new(100.0, 100.0, 200.0, 150.0);
        let frame = Rect::new(0.0, 0.0, 640.0, 480.0);

        // A Dragging moved by the translate → Some(Brushing) whose
        // corners equal the moved rect and DIFFER from the anchor.
        let moved = translate_brush(anchor, 30.0, 20.0, frame);
        let dragged = InteractionState::Dragging {
            region: BrushRegion::Interior,
            origin: Point::new(150.0, 125.0),
            start: Point::new(moved.x0, moved.y0),
            current: Point::new(moved.x1, moved.y1),
            anchor,
        };
        match redispatch_brushing_from(&dragged) {
            Some(InteractionState::Brushing { start, current }) => {
                assert_eq!(norm_rect(start, current), moved);
                assert_ne!(
                    norm_rect(start, current),
                    anchor,
                    "moved corners differ from the original"
                );
            }
            other => panic!("expected Some(Brushing) for a moved grab, got {other:?}"),
        }

        // A Dragging resized by the resize → Some(Brushing) with the
        // resized corners.
        let resized = resize_brush(
            anchor,
            BrushRegion::Edge(BrushEdge::Right),
            Point::new(260.0, 400.0),
            frame,
        );
        let dragged = InteractionState::Dragging {
            region: BrushRegion::Edge(BrushEdge::Right),
            origin: Point::new(200.0, 125.0),
            start: Point::new(resized.x0, resized.y0),
            current: Point::new(resized.x1, resized.y1),
            anchor,
        };
        match redispatch_brushing_from(&dragged) {
            Some(InteractionState::Brushing { start, current }) => {
                assert_eq!(norm_rect(start, current), resized);
            }
            other => panic!("expected Some(Brushing) for a resized grab, got {other:?}"),
        }

        // A zero-delta grab (a click on the rect — the moved rect equals the
        // anchor) → None: no redundant re-query, selection intact.
        let click = InteractionState::Dragging {
            region: BrushRegion::Interior,
            origin: Point::new(150.0, 125.0),
            start: Point::new(anchor.x0, anchor.y0),
            current: Point::new(anchor.x1, anchor.y1),
            anchor,
        };
        assert!(
            redispatch_brushing_from(&click).is_none(),
            "a zero-delta grab re-dispatches nothing"
        );

        // Every non-grab state → None (a persisted Selected on an untouched
        // sibling never re-dispatches; a fresh Brushing dispatches through the
        // unchanged path; Idle / Hovering are inert).
        assert!(redispatch_brushing_from(&InteractionState::Selected {
            start: Point::new(100.0, 100.0),
            current: Point::new(200.0, 150.0),
        })
        .is_none());
        assert!(redispatch_brushing_from(&InteractionState::Brushing {
            start: Point::new(100.0, 100.0),
            current: Point::new(200.0, 150.0),
        })
        .is_none());
        assert!(redispatch_brushing_from(&InteractionState::Idle).is_none());
        assert!(redispatch_brushing_from(&InteractionState::Hovering {
            point: Point::new(1.0, 1.0),
            nearest: None,
        })
        .is_none());
    }

    /// the pointer-shim transitions are pure InteractionState → state
    /// functions provable headless in the default gate (the gpu-tests pointer
    /// fixtures do NOT run there). A Grab at pointer_down preserves the rect
    /// (anti-wipe); a move at pointer_move applies the transform; pointer_up
    /// yields the moved Selected end-state the re-dispatch reads.
    #[test]
    fn pointer_shim_state_transitions() {
        let frame = Rect::new(0.0, 0.0, 640.0, 480.0);
        let sel = InteractionState::Selected {
            start: Point::new(100.0, 100.0),
            current: Point::new(200.0, 150.0),
        };

        // pointer_down: a Grab preserves the rect corners (no wipe to zero-area).
        let action = sel.resolve_press(Point::new(150.0, 125.0), true, TOL);
        let dragging = sel.clone().on_press(action, Point::new(150.0, 125.0));
        assert!(
            matches!(dragging, InteractionState::Dragging { .. }),
            "a Grab enters the Dragging sub-state"
        );
        assert_eq!(
            dragging.selected_rect().unwrap(),
            Rect::new(100.0, 100.0, 200.0, 150.0),
            "the grab preserves the rect (anti-wipe)"
        );

        // pointer_move: the transform arm moves the rect (interior translate by
        // the pointer delta (180-150, 145-125) = (30, 20)).
        let moved = dragging.on_grab_move(Point::new(180.0, 145.0), frame);
        assert_eq!(
            moved.selected_rect().unwrap(),
            Rect::new(130.0, 120.0, 230.0, 170.0),
            "pointer_move applies the translate"
        );

        // pointer_up: the moved sub-state yields the moved Selected end-state.
        let released = moved.clone().on_grab_release();
        assert!(matches!(released, InteractionState::Selected { .. }));
        assert_eq!(
            released.selected_rect().unwrap(),
            Rect::new(130.0, 120.0, 230.0, 170.0)
        );

        // The end-state feeds the re-dispatch: the in-flight Dragging read at
        // release re-dispatches the moved corners (Some), a zero-delta would not.
        assert!(
            redispatch_brushing_from(&moved).is_some(),
            "a moved grab re-dispatches on release"
        );
    }

    /// Review finding 1: a MISSED mouse-up during a grab (the `!button_held`
    /// pointer_move arm, which holds no coordinator and can't re-dispatch) must
    /// DISCARD the in-flight move — reverting to the ANCHOR (pre-drag) rect — so
    /// the overlay and the live filter stay consistent, NOT collapse to the moved
    /// range (a silent-no-op overlay). `on_grab_cancel` is the pure transition the
    /// shim calls.
    #[test]
    fn drb_grab_cancel_reverts_to_anchor_on_missed_release() {
        let anchor = Rect::new(100.0, 100.0, 200.0, 150.0);
        // A grab moved well away from the anchor.
        let dragging = InteractionState::Dragging {
            region: BrushRegion::Interior,
            origin: Point::new(150.0, 125.0),
            start: Point::new(230.0, 220.0),
            current: Point::new(330.0, 270.0),
            anchor,
        };
        // Sanity: the live (moved) rect differs from the anchor.
        assert_ne!(dragging.selected_rect().unwrap(), anchor);

        // A missed release cancels to a persisted Selected at the ANCHOR corners
        // (the pre-drag range), NOT the moved corners.
        let cancelled = dragging.clone().on_grab_cancel();
        assert!(matches!(cancelled, InteractionState::Selected { .. }));
        assert_eq!(
            cancelled.selected_rect().unwrap(),
            anchor,
            "a missed release reverts to the anchor, not the undispatched moved range"
        );
        // Contrast: the NORMAL release (on_grab_release) keeps the moved range —
        // because the mouse-up listener re-dispatches it first.
        let released = dragging.on_grab_release();
        assert_eq!(
            released.selected_rect().unwrap(),
            Rect::new(230.0, 220.0, 330.0, 270.0)
        );
        // Non-Dragging states pass through unchanged.
        assert!(matches!(
            InteractionState::Idle.on_grab_cancel(),
            InteractionState::Idle
        ));
    }

    /// Review finding 2: an Esc / cross-filter clear arriving MID-DRAG must drop
    /// the in-flight `Dragging` overlay too (else the filter retracts while the
    /// grey rect stays drawn). `has_persistent_selection` — the predicate
    /// `clear_persistent_selection` delegates to — is true for Selected AND
    /// Dragging, false for the overlay-less states.
    #[test]
    fn drb_has_persistent_selection_covers_dragging() {
        let sel = InteractionState::Selected {
            start: Point::new(100.0, 100.0),
            current: Point::new(200.0, 150.0),
        };
        let dragging = InteractionState::Dragging {
            region: BrushRegion::Interior,
            origin: Point::new(150.0, 125.0),
            start: Point::new(120.0, 110.0),
            current: Point::new(220.0, 160.0),
            anchor: Rect::new(100.0, 100.0, 200.0, 150.0),
        };
        assert!(
            sel.has_persistent_selection(),
            "a committed Selected clears"
        );
        assert!(
            dragging.has_persistent_selection(),
            "an in-flight Dragging also clears"
        );
        assert!(!InteractionState::Idle.has_persistent_selection());
        assert!(!InteractionState::Brushing {
            start: Point::new(1.0, 1.0),
            current: Point::new(2.0, 2.0),
        }
        .has_persistent_selection());
        assert!(!InteractionState::Hovering {
            point: Point::new(1.0, 1.0),
            nearest: None,
        }
        .has_persistent_selection());
    }
}
