//! ChartState — one plot's mutable chart state, framework-free.
//!
//! ChartState holds all mutable chart state: the Vello scene, interaction state,
//! navigation state, transition state, layout dimensions, and a shared
//! VelloRenderer reference. The HOST owns the reactive cell it lives in and
//! addresses it through [`crate::reactive::ReactiveHandle`] (the gpui shell:
//! `Entity<ChartState<…>>`); this module names no host type.
//!
//! The one host-specific thing the state touches is its base-raster cache:
//! what a presented scene *becomes* is the host's texture handle (the
//! [`CanvasHost::Surface`] of its present path), so the state is generic over
//! that handle type `S` and [`ChartState::base_image`] is driven through
//! whichever [`CanvasHost`] the shell owns. The chart surface borrows from
//! ChartState for one paint cycle. ChartState owns all mutable state; the
//! surface owns none.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use kurbo::{Affine, Point};
use vello::Scene;

use crate::canvas_host::{CanvasHost, Color, PixelSize};
use crate::chart_layout::ChartLayout;
use crate::interaction::{
    brush_region, BrushRegion, InteractionState, NavigationState, PointerAction, HANDLE_TOL,
};
use crate::vello_renderer::VelloRenderer;
use brightfield_render::layout::{Insets, Margins};
use brightfield_render::transition::Transition;

/// One plot's mutable chart state, owned by the host's reactive cell.
///
/// Owns all mutable chart state. The chart surface borrows from this for
/// one paint cycle — it is a stateless rendering shell. `S` is the host
/// texture handle the base-raster cache holds (the host's
/// [`CanvasHost::Surface`]).
pub struct ChartState<S> {
    /// The Vello scene containing the full chart (marks + axes + grid + legend).
    scene: Scene,
    /// Current interaction state (idle, brushing, hovering).
    interaction: InteractionState,
    /// Navigation state (pan/zoom), if navigation interactors are active.
    navigation: Option<NavigationState>,
    /// Active mark transition (data animation), if any.
    transition: Option<Transition>,
    /// Chart width in pixels.
    width: u32,
    /// Chart height in pixels.
    height: u32,
    /// Shared VelloRenderer for GPU rendering.
    renderer: Arc<Mutex<VelloRenderer>>,
    /// Layout with coordinate mapping (derived from width/height + insets).
    layout: ChartLayout,
    /// Per-side range insets (axis-inset round), stored so a resize
    /// (`set_dimensions`) rebuilds the layout without dropping them. Set once at
    /// launch from the plot's resolved insets via [`ChartState::set_insets`].
    insets: Insets,
    /// Title-grown margins (axis + plot titles), stored so a resize
    /// rebuilds the layout without resetting them to `Margins::default` — a
    /// titled plot's `plot_area` (hence brush inversion / point-click) must
    /// match the grown-margin scene it was drawn against, not the default. Set
    /// once at launch via [`ChartState::set_margins_and_insets`].
    margins: Margins,
    /// Cached device-resolution raster of the current scene (without the
    /// interaction overlay), reused while the scene and target dimensions are
    /// unchanged so hovering/brushing don't re-run Vello. Interior-mutable
    /// because it is populated lazily during `paint` (a `&self` context); held
    /// per-chart, so multiple charts never share or evict each other's raster.
    base_cache: RefCell<Option<BaseRaster<S>>>,
}

/// A cached device-resolution rasterisation of [`ChartState::scene`], as the
/// host texture handle the present path produced it.
struct BaseRaster<S> {
    dev_w: u32,
    dev_h: u32,
    image: S,
}

impl<S> ChartState<S> {
    /// Create a new ChartState.
    pub fn new(scene: Scene, width: u32, height: u32, renderer: Arc<Mutex<VelloRenderer>>) -> Self {
        Self {
            scene,
            interaction: InteractionState::Idle,
            navigation: None,
            transition: None,
            width,
            height,
            renderer,
            layout: ChartLayout::new(width as f64, height as f64),
            insets: Insets::default(),
            margins: Margins::default(),
            base_cache: RefCell::new(None),
        }
    }

    /// Access the current scene.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// Replace the scene (e.g. after re-render on data change).
    ///
    /// After calling this, the caller should call `cx.notify()` on the
    /// entity to trigger a repaint.
    pub fn set_scene(&mut self, scene: Scene) {
        self.scene = scene;
        // Invalidate the cached raster so the next paint re-renders.
        *self.base_cache.borrow_mut() = None;
    }

    /// The current scene rasterised at device resolution through the host's
    /// present path and returned as the host texture handle, reusing the
    /// cached result while the scene and target dimensions are unchanged.
    /// `scale_factor` is the window's device pixel ratio: the scene (in
    /// logical coordinates) is scaled up to the device pixel grid so the
    /// chart stays crisp on HiDPI displays rather than being upscaled by the
    /// compositor. The interaction overlay is NOT included — the host paints
    /// it on top — so hovering/brushing reuse this cached raster.
    pub fn base_image<H>(&self, host: &mut H, scale_factor: f32) -> S
    where
        H: CanvasHost<Surface = S>,
        S: Clone,
    {
        let sf = f64::from(scale_factor.max(1.0));
        // Match paint_image, which scales the logical bounds by the device ratio
        // and ceils the size: render at exactly that device size and scale the
        // scene to fill it, so the mapping is 1:1 (crisp) even at a fractional
        // scale factor.
        let dev_w = (f64::from(self.width) * sf).ceil().max(1.0) as u32;
        let dev_h = (f64::from(self.height) * sf).ceil().max(1.0) as u32;

        if let Some(cached) = self.base_cache.borrow().as_ref() {
            if cached.dev_w == dev_w && cached.dev_h == dev_h {
                return cached.image.clone();
            }
        }

        let scale_x = f64::from(dev_w) / f64::from(self.width.max(1));
        let scale_y = f64::from(dev_h) / f64::from(self.height.max(1));
        let mut scaled = Scene::new();
        scaled.append(
            &self.scene,
            Some(Affine::scale_non_uniform(scale_x, scale_y)),
        );

        // Present through the CanvasHost boundary: scene → the host's
        // device-resolution texture handle. The chart clears to a
        // transparent base — the overlay and ink carry their own alpha.
        let image = host.present_scene(
            &scaled,
            PixelSize {
                width: dev_w,
                height: dev_h,
            },
            Color::TRANSPARENT,
        );
        *self.base_cache.borrow_mut() = Some(BaseRaster {
            dev_w,
            dev_h,
            image: image.clone(),
        });
        image
    }

    /// Access the current interaction state.
    pub fn interaction(&self) -> &InteractionState {
        &self.interaction
    }

    /// Update the interaction state.
    pub fn set_interaction(&mut self, state: InteractionState) {
        self.interaction = state;
    }

    /// Access the navigation state.
    pub fn navigation(&self) -> Option<&NavigationState> {
        self.navigation.as_ref()
    }

    /// Set the navigation state.
    pub fn set_navigation(&mut self, nav: Option<NavigationState>) {
        self.navigation = nav;
    }

    /// Access the active transition.
    pub fn transition(&self) -> Option<&Transition> {
        self.transition.as_ref()
    }

    /// Set the active transition.
    pub fn set_transition(&mut self, transition: Option<Transition>) {
        self.transition = transition;
    }

    /// Chart width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Chart height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Rebuild the coordinate-mapping layout from the current dimensions +
    /// stored margins + insets. The single site that composes both budgets, so
    /// every mutator (resize, insets, margins) preserves the other two.
    fn rebuild_layout(&mut self) {
        self.layout = ChartLayout::with_margins_and_insets(
            self.width as f64,
            self.height as f64,
            self.margins.left,
            self.margins.top,
            self.margins.right,
            self.margins.bottom,
            self.insets,
        );
    }

    /// Update the chart dimensions (e.g. on window resize). Preserves the
    /// resolved range insets AND the title-grown margins so a resize doesn't
    /// silently drop them (which would desync hit-testing from the rendered
    /// scene on a titled plot).
    pub fn set_dimensions(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.rebuild_layout();
        // Dimensions changed — invalidate the cached raster.
        *self.base_cache.borrow_mut() = None;
    }

    /// Set the resolved per-side range insets (axis-inset round) and
    /// rebuild the layout so hit-testing and brush inversion use the same inset
    /// pixels as the rendered scale range. Preserves the title-grown margins.
    /// The scene raster is unaffected (it was drawn render-side with the insets
    /// already baked in), so the base cache is left intact.
    pub fn set_insets(&mut self, insets: Insets) {
        self.insets = insets;
        self.rebuild_layout();
    }

    /// Set the title-grown margins AND the range insets together,
    /// rebuilding the layout so a titled plot's `plot_area` — driving brush
    /// inversion and point-click hit-testing — matches the grown-margin scene it
    /// was drawn against. The assembly thread; a later resize preserves both.
    pub fn set_margins_and_insets(&mut self, margins: Margins, insets: Insets) {
        self.margins = margins;
        self.insets = insets;
        self.rebuild_layout();
    }

    /// Access the shared VelloRenderer.
    pub fn renderer(&self) -> &Arc<Mutex<VelloRenderer>> {
        &self.renderer
    }

    /// Access the chart layout for coordinate mapping.
    pub fn layout(&self) -> &ChartLayout {
        &self.layout
    }

    // --- Pointer interaction transitions ---
    //
    // These translate a window-space pointer position (with the element's
    // origin) into the chart's local plot coordinates and update the
    // interaction state. Each returns `true` when the state changed so the
    // caller can trigger a repaint. They are the single source of truth shared
    // by the live event wiring (GpuiChartSurface) and the ChartView handlers.

    /// Pointer pressed. A thin shim over the gpui-free grab resolver
    /// ([`InteractionState::resolve_press`]): a press ON the persisted
    /// `Selected` rect GRABS it (enters a move/resize sub-state preserving the
    /// rect) — resolved BEFORE the plot-contains gate, so a boundary handle in
    /// the inset-band overhang above `plot_area.y0` still grabs; a press inside
    /// the plot but Outside the rect starts (or replaces with) a fresh brush; a
    /// press outside both is ignored. Returns `true` when the state changed.
    pub fn pointer_down(&mut self, window_pos: Point, element_origin: Point) -> bool {
        let local = self.layout.window_to_local(window_pos, element_origin);
        let action = self
            .interaction
            .resolve_press(local, self.layout.contains(local), HANDLE_TOL);
        match action {
            PointerAction::Ignore => false,
            _ => {
                self.interaction = self.interaction.clone().on_press(action, local);
                true
            }
        }
    }

    /// Pointer moved. `button_held` is whether the (left) mouse button is still
    /// down. While brushing, extends the brush to the new point — clamped to the
    /// plot area so the rect can't spill over the axes — or ends the brush if the
    /// button is no longer held (a release that never reached us). While idle or
    /// hovering, sets a hover at the point if it is inside the plot area, or
    /// clears a hover when the pointer leaves it. Returns `true` on any change.
    pub fn pointer_move(
        &mut self,
        window_pos: Point,
        element_origin: Point,
        button_held: bool,
    ) -> bool {
        let local = self.layout.window_to_local(window_pos, element_origin);
        match &self.interaction {
            // An in-flight move/resize of a persisted selection:
            // transform the rect to the new pointer, clamped to the FRAME (not
            // the inset-pulled plot area), so a boundary brush can reach the
            // frame edge. A button-release we never got a mouse-up for finalises
            // the grab into a persisted Selected.
            InteractionState::Dragging { .. } => {
                if !button_held {
                    // A missed mouse-up during a grab (focus steal / release
                    // outside the window): this path holds NO coordinator, so it
                    // can't re-dispatch the moved range. Discard the in-flight
                    // move — revert to the anchor (pre-drag) rect — so the overlay
                    // and the live filter stay consistent at the already-
                    // dispatched pre-drag range, mirroring the Brushing arm's
                    // discard-to-Idle below. (The NORMAL release goes through the
                    // element's mouse-up listener → redispatch → pointer_up.)
                    self.interaction = self.interaction.clone().on_grab_cancel();
                    return true;
                }
                self.interaction = self
                    .interaction
                    .clone()
                    .on_grab_move(local, self.layout.frame_area());
                true
            }
            InteractionState::Brushing { .. } => {
                if !button_held {
                    // The button was released without a mouse-up reaching us
                    // (focus steal, or a release outside the window). End it.
                    self.interaction = InteractionState::Idle;
                    return true;
                }
                let clamped = self.clamp_to_frame(local);
                let mut next = self.interaction.clone();
                next.update_brush(clamped);
                self.interaction = next;
                true
            }
            InteractionState::Hovering { .. } | InteractionState::Idle => {
                if !button_held && self.layout.contains(local) {
                    // Hover is a no-button gesture; a held button means a drag is
                    // in progress (e.g. a brush in a sibling plot), so don't light
                    // up a hover marker here.
                    self.interaction = InteractionState::Hovering {
                        point: local,
                        nearest: None,
                    };
                    true
                } else if matches!(self.interaction, InteractionState::Hovering { .. }) {
                    // Pointer left the plot area (or a button went down) — drop hover.
                    self.interaction = InteractionState::Idle;
                    true
                } else {
                    false
                }
            }
            // A persisted selection stays put while merely moving/hovering over it;
            // a press grabs it (pointer_down → Dragging), a click outside or Esc
            // clears it. Suppresses the hover marker on a selected plot. (The
            // paint-phase cursor over the rect is picked from `cursor_region`,
            // tracked by the element's mouse-move listener.)
            InteractionState::Selected { .. } => false,
        }
    }

    /// Clamp a local-space point to the FRAME area (margins only, no insets),
    /// so a brush dragged into the margins keeps its rectangle within the
    /// frame while still reaching the axis-inset band to enclose a boundary dot.
    /// Retargets the pre-card `clamp_to_plot` (plot_area); `plot_area` and the
    /// axis-inset range pull are unchanged.
    fn clamp_to_frame(&self, p: Point) -> Point {
        let area = self.layout.frame_area();
        Point::new(p.x.clamp(area.x0, area.x1), p.y.clamp(area.y0, area.y1))
    }

    /// Classify the pointer over any persisted `Selected` rect for the
    /// paint-phase cursor: the grabbable region under `window_pos`,
    /// or `Outside` when there is no persisted selection. While a grab is
    /// in-flight (`Dragging`) the active region holds, so the cursor stays put.
    /// Pure over the gpui-free [`brush_region`]; the element's mouse-move
    /// listener stores the result and refreshes on change.
    pub fn cursor_region(&self, window_pos: Point, element_origin: Point) -> BrushRegion {
        let local = self.layout.window_to_local(window_pos, element_origin);
        match &self.interaction {
            InteractionState::Selected { .. } => self
                .interaction
                .selected_rect()
                .map(|r| brush_region(local, r, HANDLE_TOL))
                .unwrap_or(BrushRegion::Outside),
            InteractionState::Dragging { region, .. } => *region,
            _ => BrushRegion::Outside,
        }
    }

    /// Pointer released. A DRAG commits to a persistent `Selected` rectangle
    /// (Mosaic / Vega-Lite fidelity — the selection stays drawn until cleared); a
    /// click (zero-area) returns to idle. Returns `true` when a brush was in
    /// progress. Selection dispatch (re-query) is handled separately by the app
    /// shell via `commit_brush`.
    pub fn pointer_up(&mut self) -> bool {
        // Finalise an in-flight move/resize: the moved `Dragging` collapses to a
        // persisted `Selected` at its new corners. The cross-filter
        // re-dispatch from those corners is driven by the element's mouse-up
        // listener via `redispatch_brushing_from`, BEFORE this transition.
        if matches!(self.interaction, InteractionState::Dragging { .. }) {
            self.interaction = self.interaction.clone().on_grab_release();
            return true;
        }
        let (start, current) = match &self.interaction {
            InteractionState::Brushing { start, current } => (*start, *current),
            _ => return false,
        };
        let is_drag = (start.x - current.x).abs() >= crate::chart_view::ZERO_AREA_EPSILON
            || (start.y - current.y).abs() >= crate::chart_view::ZERO_AREA_EPSILON;
        self.interaction = if is_drag {
            InteractionState::Selected { start, current }
        } else {
            InteractionState::Idle
        };
        true
    }

    /// Drop a persistent selection overlay — a committed `Selected` OR an
    /// in-flight `Dragging` (Esc / cross-filter clear). Returns `true` if an
    /// overlay was cleared.
    pub fn clear_persistent_selection(&mut self) -> bool {
        // Drop an in-flight `Dragging` overlay too: a clear arriving
        // mid-drag retracts the filter, so leaving the grey rect drawn would be a
        // transient visual/data mismatch.
        if self.interaction.has_persistent_selection() {
            self.interaction = InteractionState::Idle;
            true
        } else {
            false
        }
    }
}

// ChartState must be Send (with a Send surface handle) for hosts whose
// reactive cells require it (gpui's Entity does).
// This is safe because all remaining fields are Send:
// - Scene is Send
// - InteractionState is Send (Point, NearestHit are Send)
// - NavigationState is Send
// - Transition is Send
// - VelloRenderer contains wgpu types which are Send
// Compile-time assertion, generic in the surface handle.
fn _assert_chart_state_send<S: Send>() {
    fn _assert<T: Send>() {}
    _assert::<ChartState<S>>();
}

#[cfg(all(test, feature = "gpu-tests"))]
mod tests {
    use super::*;
    use vello::Scene;

    // --- ChartState struct ---

    #[test]
    fn chart_state_construction() {
        let renderer = VelloRenderer::new();
        let scene = Scene::new();
        let state = ChartState::<()>::new(scene, 640, 480, renderer);

        assert_eq!(state.width(), 640);
        assert_eq!(state.height(), 480);
        assert!(matches!(state.interaction(), InteractionState::Idle));
        assert!(state.navigation().is_none());
        assert!(state.transition().is_none());
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn chart_state_scene_update() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::<()>::new(Scene::new(), 640, 480, renderer);

        let mut new_scene = Scene::new();
        use kurbo::{Affine, Circle};
        use peniko::{Color, Fill};
        let circle = Circle::new((100.0, 100.0), 5.0);
        new_scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::new([1.0, 0.0, 0.0, 1.0]),
            None,
            &circle,
        );
        state.set_scene(new_scene);

        let encoding = state.scene().encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "updated scene should have content"
        );
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn chart_state_dimensions_update() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::<()>::new(Scene::new(), 640, 480, renderer);

        state.set_dimensions(1024, 768);
        assert_eq!(state.width(), 1024);
        assert_eq!(state.height(), 768);
        assert!((state.layout().width - 1024.0).abs() < f64::EPSILON);
        assert!((state.layout().height - 768.0).abs() < f64::EPSILON);
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn resize_preserves_grown_title_margins() {
        // The review HIGH: a resize must NOT reset title-grown margins to
        // Margins::default (which would desync hit-testing from the titled
        // scene). set_margins_and_insets stores them; set_dimensions preserves.
        let mut state = ChartState::<()>::new(Scene::new(), 640, 480, VelloRenderer::new());
        let grown = Margins {
            left: 60.0,
            top: 40.0,
            right: 20.0,
            bottom: 50.0,
        };
        state.set_margins_and_insets(grown, Insets::default());
        assert!((state.layout().margin_left - 60.0).abs() < f64::EPSILON);

        state.set_dimensions(1024, 768);
        assert!(
            (state.layout().margin_left - 60.0).abs() < f64::EPSILON,
            "resize preserved the grown left margin"
        );
        assert!(
            (state.layout().margin_bottom - 50.0).abs() < f64::EPSILON,
            "resize preserved the grown bottom margin"
        );
        assert!((state.layout().width - 1024.0).abs() < f64::EPSILON);
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn chart_state_interaction_update() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::<()>::new(Scene::new(), 640, 480, renderer);

        state.set_interaction(InteractionState::start_brush(kurbo::Point::new(10.0, 20.0)));
        assert!(matches!(
            state.interaction(),
            InteractionState::Brushing { .. }
        ));
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn pointer_down_inside_starts_brush_outside_does_not() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::<()>::new(Scene::new(), 640, 480, renderer);

        // Inside the plot area (default margins 40/20/20/30) at origin (0,0).
        assert!(state.pointer_down(Point::new(300.0, 200.0), Point::new(0.0, 0.0)));
        assert!(matches!(
            state.interaction(),
            InteractionState::Brushing { .. }
        ));

        // In the left margin (x=10 < 40) — no brush.
        let mut state2 = ChartState::<()>::new(Scene::new(), 640, 480, VelloRenderer::new());
        assert!(!state2.pointer_down(Point::new(10.0, 200.0), Point::new(0.0, 0.0)));
        assert!(matches!(state2.interaction(), InteractionState::Idle));
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn pointer_move_drags_brush_and_hovers() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::<()>::new(Scene::new(), 640, 480, renderer);

        // Hover inside the plot area (no button held).
        assert!(state.pointer_move(Point::new(300.0, 200.0), Point::new(0.0, 0.0), false));
        assert!(matches!(
            state.interaction(),
            InteractionState::Hovering { .. }
        ));

        // Begin a brush and drag with the button held — the rect tracks the pointer.
        state.pointer_down(Point::new(100.0, 100.0), Point::new(0.0, 0.0));
        assert!(state.pointer_move(Point::new(250.0, 300.0), Point::new(0.0, 0.0), true));
        let rect = state.interaction().brush_rect().expect("brushing");
        assert!((rect.x1 - 250.0).abs() < f64::EPSILON);

        // A move with the button no longer held ends the brush.
        assert!(state.pointer_move(Point::new(260.0, 310.0), Point::new(0.0, 0.0), false));
        assert!(matches!(state.interaction(), InteractionState::Idle));

        // pointer_up on an already-idle state is a no-op.
        assert!(!state.pointer_up());
    }

    #[cfg(feature = "gpu-tests")]
    #[test]
    fn pointer_up_persists_a_drag_and_a_click_stays_idle() {
        let renderer = VelloRenderer::new();
        let mut state = ChartState::<()>::new(Scene::new(), 640, 480, renderer);

        // A drag commits to a persistent Selected rectangle (Mosaic fidelity).
        state.pointer_down(Point::new(120.0, 120.0), Point::new(0.0, 0.0));
        state.pointer_move(Point::new(280.0, 300.0), Point::new(0.0, 0.0), true);
        assert!(state.pointer_up());
        assert!(
            matches!(state.interaction(), InteractionState::Selected { .. }),
            "a drag persists as a committed selection"
        );

        // Esc / cross-filter clear drops the committed selection.
        assert!(state.clear_persistent_selection());
        assert!(matches!(state.interaction(), InteractionState::Idle));
        assert!(!state.clear_persistent_selection(), "nothing left to clear");

        // A zero-area click inside the plot does NOT persist.
        state.pointer_down(Point::new(300.0, 200.0), Point::new(0.0, 0.0));
        assert!(state.pointer_up());
        assert!(
            matches!(state.interaction(), InteractionState::Idle),
            "a click clears rather than persisting"
        );
    }
}
