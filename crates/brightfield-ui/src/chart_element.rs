//! ChartElement — GPUI Element that paints the chart and routes mouse input.
//!
//! ChartElement is created fresh by `ChartView::render()` each frame. It holds
//! the `Entity<ChartState>` and, on paint:
//!   1. composites the interaction overlay (brush rect / hover marker) onto a
//!      clone of the current scene,
//!   2. rasterises that scene with the shared Vello renderer and paints it, and
//!   3. registers window mouse listeners that drive the chart's interaction
//!      state (brush / hover). GPUI clears per-frame listeners each frame, so
//!      they are re-registered every paint; a state change calls
//!      `window.refresh()` to repaint with the updated overlay.
//!
//! Lifecycle:
//! - `request_layout()` — fixed-size layout from ChartState dimensions
//! - `prepaint()` — registers a hitbox covering the element bounds
//! - `paint()` — composite + rasterise + paint + wire mouse events

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    fill, point, px, size, App, BorderStyle, Bounds, Corners, Element, ElementId, Entity,
    GlobalElementId, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Rgba, Size, Style, Window,
};
use kurbo::Point;

use crate::chart_state::ChartState;
use crate::crossfilter::CrossfilterCoordinator;
use crate::interaction::InteractionState;

/// GPUI element that paints a chart from its `ChartState` and routes mouse input.
///
/// Created by `ChartView::render()` each frame. Owns no chart state of its own —
/// it reads and updates the shared `Entity<ChartState>`.
pub struct ChartElement {
    /// The reactive chart state entity.
    state: Entity<ChartState>,
    /// This plot's index, both for the stable element id and as the plot index
    /// into the cross-filter coordinator (charts are hosted in plot order).
    index: usize,
    /// Stable, per-plot element id. (Sibling charts already get independent
    /// hitboxes — each `insert_hitbox` mints a unique id — so this is for GPUI's
    /// per-element state keying rather than relying on anonymous ids.)
    id: ElementId,
    /// Shared cross-filter coordinator. When present, a brush release routes
    /// through it to re-query and re-render subscriber plots; `None` means the
    /// brush is purely visual (no linked views).
    coordinator: Option<Rc<RefCell<CrossfilterCoordinator>>>,
}

impl ChartElement {
    /// Create a chart element bound to the given state entity. `index` gives the
    /// element a stable id distinguishing it from sibling plots in a dashboard,
    /// and is the plot index into `coordinator`. `coordinator` is `None` for a
    /// dashboard that doesn't cross-filter.
    pub fn new(
        state: Entity<ChartState>,
        index: usize,
        coordinator: Option<Rc<RefCell<CrossfilterCoordinator>>>,
    ) -> Self {
        Self {
            state,
            index,
            id: ElementId::from(("brightfield-plot", index)),
            coordinator,
        }
    }
}

impl IntoElement for ChartElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ChartElement {
    type RequestLayoutState = ();
    type PrepaintState = gpui::Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let (width, height) = {
            let state = self.state.read(cx);
            (state.width(), state.height())
        };
        let mut style = Style::default();
        style.size = Size {
            width: gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                gpui::AbsoluteLength::Pixels(px(width as f32)),
            )),
            height: gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                gpui::AbsoluteLength::Pixels(px(height as f32)),
            )),
        };
        let layout_id = window.request_layout(style, [], cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        // Register a hitbox covering the full element bounds for mouse events.
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Register window mouse listeners FIRST, so input survives even a
        // transient zero-size frame (the raster below may early-return, but the
        // listeners only need bounds.origin + the hitbox). The element origin
        // maps window-space positions to chart-local coordinates; the hitbox
        // restricts presses to this element. GPUI clears per-frame listeners, so
        // they are re-registered every paint, and window.refresh() is the
        // repaint trigger when the interaction state changes.
        let element_origin = Point::new(bounds.origin.x.to_f64(), bounds.origin.y.to_f64());

        // Mouse down — begin a brush if the press is over the chart.
        window.on_mouse_event({
            let state = self.state.clone();
            let hitbox = prepaint.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase.bubble()
                    && event.button == MouseButton::Left
                    && hitbox.is_hovered(window)
                {
                    let pos = Point::new(event.position.x.to_f64(), event.position.y.to_f64());
                    let changed = state.update(cx, |s, _| s.pointer_down(pos, element_origin));
                    if changed {
                        window.refresh();
                    }
                }
            }
        });

        // Mouse move — extend the brush while the button is held, or update hover.
        window.on_mouse_event({
            let state = self.state.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase.bubble() {
                    let pos = Point::new(event.position.x.to_f64(), event.position.y.to_f64());
                    let held = event.pressed_button == Some(MouseButton::Left);
                    let changed = state.update(cx, |s, _| s.pointer_move(pos, element_origin, held));
                    if changed {
                        window.refresh();
                    }
                }
            }
        });

        // Mouse up — commit the brush (cross-filter the linked views, if wired)
        // then end the visual brush. The commit reads this plot's Brushing rect
        // BEFORE pointer_up clears it, dispatches the selection through the
        // coordinator's live Session, and swaps the re-queried subscriber scenes.
        window.on_mouse_event({
            let state = self.state.clone();
            let coordinator = self.coordinator.clone();
            let plot_index = self.index;
            move |event: &MouseUpEvent, phase, window, cx| {
                if phase.bubble() && event.button == MouseButton::Left {
                    // `committed` is true only for the plot that was actually
                    // brushed, so an untouched sibling's listener doesn't trigger
                    // a redundant window refresh.
                    let committed = match &coordinator {
                        Some(coord) => {
                            let interaction = state.read(cx).interaction().clone();
                            coord.borrow_mut().commit_brush(plot_index, &interaction, cx)
                        }
                        None => false,
                    };
                    let changed = state.update(cx, |s, _| s.pointer_up());
                    if changed || committed {
                        window.refresh();
                    }
                }
            }
        });

        // Fetch the cached, device-resolution base raster (re-rendered only when
        // the scene changes) and paint it; then draw the interaction overlay as
        // cheap GPUI quads on top, so hovering/brushing never re-run Vello.
        let sf = window.scale_factor();
        let (base, interaction) = {
            let state = self.state.read(cx);
            if state.width() == 0 || state.height() == 0 {
                return; // nothing to rasterise; mouse listeners already registered
            }
            (state.base_image(sf), state.interaction().clone())
        };

        let _ = window.paint_image(bounds, Corners::default(), base, 0, false);
        paint_overlay(window, bounds, &interaction);
    }
}

/// Convert a straight-alpha RGBA tuple (0–1) to a GPUI colour.
fn rgba(r: f32, g: f32, b: f32, a: f32) -> Hsla {
    Rgba { r, g, b, a }.into()
}

/// Hover marker radius in logical pixels (mirrors `interaction::render_overlay`).
const HOVER_RADIUS: f64 = 8.0;

/// Paint the interaction overlay (brush rectangle / hover marker) as GPUI quads
/// over the chart image. Coordinates are element-local logical pixels — the same
/// space the interaction state stores — offset by the element origin. Drawing
/// the overlay as quads (rather than compositing into the Vello scene) means an
/// interaction repaint reuses the cached base raster instead of re-rendering.
fn paint_overlay(window: &mut Window, bounds: Bounds<Pixels>, interaction: &InteractionState) {
    let ox = bounds.origin.x;
    let oy = bounds.origin.y;
    match interaction {
        InteractionState::Idle => {}
        InteractionState::Brushing { start, current } => {
            let x0 = start.x.min(current.x);
            let y0 = start.y.min(current.y);
            let w = (start.x.max(current.x) - x0) as f32;
            let h = (start.y.max(current.y) - y0) as f32;
            let rect = Bounds {
                origin: point(ox + px(x0 as f32), oy + px(y0 as f32)),
                size: size(px(w), px(h)),
            };
            let mut q = fill(rect, rgba(0.306, 0.475, 0.655, 0.251));
            q.border_widths = (1.5).into();
            q.border_color = rgba(0.306, 0.475, 0.655, 0.753);
            q.border_style = BorderStyle::Solid;
            window.paint_quad(q);
        }
        InteractionState::Hovering { point: p, .. } => {
            let d = (HOVER_RADIUS * 2.0) as f32;
            let rect = Bounds {
                origin: point(
                    ox + px((p.x - HOVER_RADIUS) as f32),
                    oy + px((p.y - HOVER_RADIUS) as f32),
                ),
                size: size(px(d), px(d)),
            };
            let mut q = fill(rect, rgba(0.949, 0.557, 0.169, 0.376));
            q.corner_radii = (HOVER_RADIUS as f32).into(); // round the quad into a circle
            window.paint_quad(q);
        }
    }
}
