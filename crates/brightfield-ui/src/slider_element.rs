//! SliderElement — GPUI Element that draws a slider and routes its drag into a
//! param commit. The model half (SliderBinding / SliderState /
//! value_at / commit) lives in `slider.rs` and stays gpui-free; this file is the
//! window layer, mirroring `chart_element.rs`.
//!
//! Lifecycle (per frame, created by `ChartView::render`):
//! - `request_layout()` — fixed size from the host placement
//! - `prepaint()` — hitbox over the widget bounds
//! - `paint()` — draw track + thumb as GPUI quads and wire mouse events. A drag
//!   updates the overlay (thumb) only; the release commits through the
//!   coordinator (`propagate_param` → re-render subscribers). Mid-drag never
//!   re-queries.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    fill, point, px, size, App, Bounds, Element, ElementId, Entity, GlobalElementId,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Rgba, Size, Style, Window,
};

use crate::crossfilter::CrossfilterCoordinator;
use crate::slider::{thumb_fraction, value_at, SliderBinding, SliderState};

/// Thumb radius (logical px). The track is inset by this so the thumb stays
/// within the widget bounds at the ends.
const THUMB_RADIUS: f32 = 7.0;
/// Track bar thickness (logical px).
const TRACK_THICKNESS: f32 = 4.0;

/// Slider colours — kept in sync with the headless twin
/// (brightfield-render scene.rs `SLIDER_TRACK_COLOUR`/`SLIDER_THUMB_COLOUR`)
/// so the dumped PNG matches the window. Both sides read the same
/// Meridian tokens: track = warm gray step 5 (#dcdad8), thumb = Maritime
/// focus ink (#4b7a9b).
const TRACK_TOKEN: meridian_design::colour::Rgba = meridian_design::scales::GRAY_LIGHT[4];
const THUMB_TOKEN: meridian_design::colour::Rgba = meridian_design::chrome::INK_LIGHT.focus;

/// Persistent per-slider UI state held in a gpui `Entity`: the committed
/// (resting) value and the current drag gesture. Binding + geometry live on the
/// [`SliderElement`], rebuilt each frame from the host placement.
pub struct SliderWidget {
    /// The committed value — where the thumb rests when not dragging.
    pub value: f64,
    /// The live drag gesture (`Idle` at rest).
    pub gesture: SliderState,
}

impl SliderWidget {
    /// A widget resting at `value`.
    pub fn new(value: f64) -> Self {
        Self {
            value,
            gesture: SliderState::Idle,
        }
    }

    /// The value to display: the live drag value while dragging/released, else
    /// the committed resting value.
    fn displayed(&self) -> f64 {
        match self.gesture {
            SliderState::Dragging { value } | SliderState::Released { value } => value,
            SliderState::Idle => self.value,
        }
    }
}

/// GPUI element that draws a slider (track + thumb) and routes a horizontal drag
/// into a param commit. Created fresh each frame; owns no persistent state.
pub struct SliderElement {
    state: Entity<SliderWidget>,
    binding: SliderBinding,
    width: f64,
    height: f64,
    /// Index into the coordinator's slider bindings (sliders hosted in order).
    index: usize,
    id: ElementId,
    coordinator: Option<Rc<RefCell<CrossfilterCoordinator>>>,
}

impl SliderElement {
    /// Create a slider element bound to its widget state. `index` is the stable
    /// element id and the slider index into `coordinator`.
    pub fn new(
        state: Entity<SliderWidget>,
        binding: SliderBinding,
        width: f64,
        height: f64,
        index: usize,
        coordinator: Option<Rc<RefCell<CrossfilterCoordinator>>>,
    ) -> Self {
        Self {
            state,
            binding,
            width,
            height,
            index,
            id: ElementId::from(("brightfield-slider", index)),
            coordinator,
        }
    }

    /// Track geometry in element-local pixels: `(left, width, center_y)`.
    fn track(&self) -> (f64, f64, f64) {
        let inset = THUMB_RADIUS as f64;
        (
            inset,
            (self.width - inset * 2.0).max(0.0),
            self.height / 2.0,
        )
    }
}

impl IntoElement for SliderElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SliderElement {
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
        let mut style = Style::default();
        style.size = Size {
            width: gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                gpui::AbsoluteLength::Pixels(px(self.width as f32)),
            )),
            height: gpui::Length::Definite(gpui::DefiniteLength::Absolute(
                gpui::AbsoluteLength::Pixels(px(self.height as f32)),
            )),
        };
        (window.request_layout(style, [], cx), ())
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
        let (track_left, track_w, cy) = self.track();
        // Track origin in window space, so a drag pixel maps to a value.
        let track_left_win = bounds.origin.x.to_f64() + track_left;

        // Mouse down — begin a drag if over the widget; the thumb jumps to the
        // press position.
        window.on_mouse_event({
            let state = self.state.clone();
            let hitbox = prepaint.clone();
            let binding = self.binding.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if phase.bubble() && event.button == MouseButton::Left && hitbox.is_hovered(window)
                {
                    let v = value_at(event.position.x.to_f64(), track_left_win, track_w, &binding);
                    state.update(cx, |w, _| w.gesture = SliderState::start_drag(v));
                    window.refresh();
                }
            }
        });

        // Mouse move — update the live drag value (overlay only; NO re-query).
        window.on_mouse_event({
            let state = self.state.clone();
            let binding = self.binding.clone();
            move |event: &MouseMoveEvent, phase, window, cx| {
                if phase.bubble() && event.pressed_button == Some(MouseButton::Left) {
                    let v = value_at(event.position.x.to_f64(), track_left_win, track_w, &binding);
                    let changed = state.update(cx, |w, _| {
                        if matches!(w.gesture, SliderState::Dragging { .. }) {
                            w.gesture.update_drag(v);
                            true
                        } else {
                            false
                        }
                    });
                    if changed {
                        window.refresh();
                    }
                }
            }
        });

        // Mouse up — commit the released value: re-execute subscribers and repaint.
        // Only the slider that owns the active drag commits (window-level listener
        // fires for every slider).
        window.on_mouse_event({
            let state = self.state.clone();
            let coordinator = self.coordinator.clone();
            let index = self.index;
            move |event: &MouseUpEvent, phase, window, cx| {
                if phase.bubble() && event.button == MouseButton::Left {
                    let released = state.update(cx, |w, _| {
                        if matches!(w.gesture, SliderState::Dragging { .. }) {
                            w.gesture.release();
                            match w.gesture {
                                SliderState::Released { value } => Some(value),
                                _ => None,
                            }
                        } else {
                            None
                        }
                    });
                    if let Some(value) = released {
                        if let Some(coord) = &coordinator {
                            let gesture = SliderState::Released { value };
                            coord.borrow_mut().commit_slider(index, &gesture, cx);
                        }
                        // Settle: the thumb rests at the committed value.
                        state.update(cx, |w, _| {
                            w.value = value;
                            w.gesture = SliderState::Idle;
                        });
                        window.refresh();
                    }
                }
            }
        });

        // Draw the track, then the thumb at the displayed value.
        let displayed = self.state.read(cx).displayed();
        let ox = bounds.origin.x;
        let oy = bounds.origin.y;

        let track_rect = Bounds {
            origin: point(
                ox + px(track_left as f32),
                oy + px(cy as f32) - px(TRACK_THICKNESS / 2.0),
            ),
            size: size(px(track_w as f32), px(TRACK_THICKNESS)),
        };
        let mut tq = fill(track_rect, token(TRACK_TOKEN));
        tq.corner_radii = (TRACK_THICKNESS / 2.0).into();
        window.paint_quad(tq);

        let frac = thumb_fraction(displayed, &self.binding) as f32;
        let thumb_cx = track_left as f32 + track_w as f32 * frac;
        let thumb_rect = Bounds {
            origin: point(
                ox + px(thumb_cx) - px(THUMB_RADIUS),
                oy + px(cy as f32) - px(THUMB_RADIUS),
            ),
            size: size(px(THUMB_RADIUS * 2.0), px(THUMB_RADIUS * 2.0)),
        };
        let mut thq = fill(thumb_rect, token(THUMB_TOKEN));
        thq.corner_radii = (THUMB_RADIUS).into();
        window.paint_quad(thq);
    }
}

/// Meridian design token → GPUI colour (straight-alpha sRGB on both sides;
/// mirrors chart_element::rgba's boundary conversion).
fn token(c: meridian_design::colour::Rgba) -> Hsla {
    Rgba {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
    .into()
}
