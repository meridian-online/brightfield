//! LegendElement — GPUI Element that paints one standalone legend (card 0016).
//!
//! The scene half (`legend_scene::build_legend_scene`) stays gpui-free; this
//! file is the window layer, mirroring `chart_element.rs`'s raster path:
//! vello scene → device-resolution RGBA (BGRA-swapped) → `RenderImage` → one
//! `paint_image` (the chart_state.rs choke point pattern, with the same
//! scale-factor-aware cache so repaints don't re-run Vello). The raster is
//! padded by the panel border's 0.25 logical-px overhang on every side (see
//! [`legend_raster_geometry`]) so the window paints the full stroke the PNG
//! composite shows, instead of clipping it at the buffer edge.
//!
//! **Display-only by default:** a legend without a selection binding has no
//! mouse listeners, no coordinator, no hitbox — exactly as card 0016 shipped.
//! A legend BOUND `as:` a selection (card 0009, click-to-filter) additionally
//! registers a hitbox and a mouse-up hit-test: the click maps to a swatch
//! entry via [`swatch_entry_rects`] and commits through
//! [`CrossfilterCoordinator::commit_legend_click`]. Sequential (gradient)
//! legends never bind — they have no discrete entries.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{
    px, App, Bounds, Corners, Element, ElementId, GlobalElementId, HitboxBehavior,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    RenderImage, Size, Style, Window,
};
use kurbo::{Affine, Point};
use vello::Scene;

use brightfield_render::legend::swatch_entry_rects;
use brightfield_render::scale::Scale;

use crate::crossfilter::CrossfilterCoordinator;
use crate::legend_scene::build_legend_scene;
use crate::vello_renderer::VelloRenderer;

/// One standalone legend positioned in the dashboard: its rect (in dashboard
/// pixels) and the colour scale it displays. A plain descriptor — no gpui
/// entity, no renderer — so the app's placement mapping is headlessly
/// testable; the raster cache rides along as an initially-empty cell.
pub struct PlacedLegend {
    /// Left edge within the dashboard, in pixels.
    pub x: f64,
    /// Top edge within the dashboard, in pixels.
    pub y: f64,
    /// Legend panel width in pixels.
    pub width: f64,
    /// Legend panel height in pixels.
    pub height: f64,
    /// The colour scale displayed (categorical swatches or sequential bar).
    pub scale: Scale,
    /// The legend's binding index into the coordinator's legend bindings plus
    /// the shared coordinator, present only for a legend bound `as:` a
    /// selection (card 0009). `None` = display-only, exactly as 0016 shipped.
    pub binding: Option<(usize, Rc<RefCell<CrossfilterCoordinator>>)>,
    /// Whether the current mouse press originated inside this legend's panel
    /// — the press-origin half of the click decision (card 0009 F6). Lives on
    /// the placement (not the per-frame element) so it survives element
    /// recreation between the press and the release; a drag that starts on a
    /// plot and releases over the legend must not dispatch.
    pressed: Rc<Cell<bool>>,
    /// Cached device-resolution raster, shared with the per-frame elements so
    /// hovering/toggling elsewhere never re-runs Vello for a static legend.
    cache: Rc<RefCell<Option<LegendRaster>>>,
}

impl PlacedLegend {
    /// A display-only legend descriptor at rect `(x, y, width, height)`
    /// displaying `scale`.
    pub fn new(x: f64, y: f64, width: f64, height: f64, scale: Scale) -> Self {
        Self {
            x,
            y,
            width,
            height,
            scale,
            binding: None,
            pressed: Rc::new(Cell::new(false)),
            cache: Rc::new(RefCell::new(None)),
        }
    }

    /// Bind this legend to the shared coordinator as its legend binding
    /// `index` (card 0009): a swatch click on the hosted element then commits
    /// through [`CrossfilterCoordinator::commit_legend_click`]. Only
    /// meaningful for a categorical [`Scale::Colour`] legend — the element
    /// arms its hit-test for bound Colour legends exclusively.
    #[must_use]
    pub fn with_binding(
        mut self,
        index: usize,
        coordinator: Rc<RefCell<CrossfilterCoordinator>>,
    ) -> Self {
        self.binding = Some((index, coordinator));
        self
    }
}

/// Resolve which category a click at `local` (element-local logical px — the
/// panel's top-left is the element origin) lands on: the index of the
/// containing [`swatch_entry_rects`] entry, mapped to its category. `None`
/// for a click between/outside entries or a non-Colour scale.
///
/// Coordinate space: the painted raster is inflated by the border-pad fringe
/// (see [`legend_raster_geometry`]), but the paint offsets the image by the
/// same pad so the panel INTERIOR stays exactly at the element's layout
/// bounds — the pad cancels, and element-local coordinates map straight onto
/// entry-rect space at origin (0, 0) with no offset. Card 0009, lcf ac-05.
#[must_use]
pub fn swatch_hit_category(local: Point, scale: &Scale) -> Option<String> {
    let categories = match scale {
        Scale::Colour { categories, .. } => categories,
        _ => return None,
    };
    swatch_entry_rects(0.0, 0.0, scale)
        .iter()
        .position(|r| r.contains(local))
        .and_then(|i| categories.get(i).cloned())
}

/// The press/release decision for a bound legend's click (card 0009 F6):
/// consume the recorded press-origin flag — so a stale press can never leak
/// into a later gesture — and commit only when the press originated inside
/// the panel AND the release lands inside it. A drag that starts on a plot
/// (e.g. a brush) and releases over the legend, or starts on the legend and
/// releases elsewhere, never dispatches.
fn legend_click_decision(pressed: &Cell<bool>, released_inside: bool) -> bool {
    pressed.replace(false) && released_inside
}

/// Raster-buffer geometry for a legend panel of `width` × `height` logical px
/// at `scale_factor`: the composite draws the panel's 0.5px border stroke
/// centred on the panel rect edge, so 0.25 logical px of stroke overhangs each
/// side. In the PNG the surrounding canvas absorbs that overhang; a buffer
/// starting exactly at the rect edge would clip it. Pad the device buffer by
/// the overhang (ceiled to whole device pixels) per side, so interior
/// alignment is untouched and the border renders whole.
///
/// Returns `(dev_w, dev_h, pad_dev)`: the padded buffer size and the per-side
/// pad, all in device pixels. Kept gpui-free so the geometry is headlessly
/// testable.
fn legend_raster_geometry(width: f64, height: f64, scale_factor: f64) -> (u32, u32, u32) {
    let pad_dev = (0.25 * scale_factor).ceil() as u32;
    let dev_w = (width * scale_factor).ceil().max(1.0) as u32 + 2 * pad_dev;
    let dev_h = (height * scale_factor).ceil().max(1.0) as u32 + 2 * pad_dev;
    (dev_w, dev_h, pad_dev)
}

/// A cached device-resolution rasterisation of a legend scene (mirrors
/// `chart_state::BaseRaster`). The `selected` category is part of the cache key
/// (card 0006 selected-state): a bound categorical legend re-rasterises when the
/// active category changes, but a static (unbound / Sequential) legend keeps
/// `None` and so never re-runs Vello for a gesture elsewhere.
struct LegendRaster {
    dev_w: u32,
    dev_h: u32,
    selected: Option<String>,
    image: Arc<RenderImage>,
}

/// Whether a cached legend raster is still valid for the requested device size
/// AND selected category. A pure predicate so the cache key — dims *and*
/// selected-state — is unit-testable without a window (card 0006, cfr_ac06).
fn raster_cache_hit(
    cached_dims: (u32, u32),
    cached_selected: &Option<String>,
    dev_w: u32,
    dev_h: u32,
    selected: &Option<String>,
) -> bool {
    cached_dims == (dev_w, dev_h) && cached_selected == selected
}

/// GPUI element that paints one standalone legend. Created fresh each frame by
/// `ChartView::render()`; owns no state beyond clones of the placement's
/// descriptor fields and the shared raster cache.
pub struct LegendElement {
    scale: Scale,
    width: f64,
    height: f64,
    /// Stable, per-legend element id (position in the hosted legend list —
    /// distinct from the coordinator binding index below).
    id: ElementId,
    /// Coordinator + binding index for a bound legend (card 0009); `None`
    /// keeps the element display-only.
    binding: Option<(usize, Rc<RefCell<CrossfilterCoordinator>>)>,
    /// Shared press-origin flag (see [`PlacedLegend::pressed`]) — set on
    /// mouse-down, consumed by the mouse-up decision (card 0009 F6).
    pressed: Rc<Cell<bool>>,
    cache: Rc<RefCell<Option<LegendRaster>>>,
    renderer: Arc<Mutex<VelloRenderer>>,
}

impl LegendElement {
    /// Create a legend element for `placed`, sharing its raster cache and
    /// selection binding. `index` distinguishes sibling legends for GPUI's
    /// element keying; `renderer` is the window's shared Vello renderer.
    pub fn new(placed: &PlacedLegend, index: usize, renderer: Arc<Mutex<VelloRenderer>>) -> Self {
        Self {
            scale: placed.scale.clone(),
            width: placed.width,
            height: placed.height,
            id: ElementId::from(("brightfield-legend", index)),
            binding: placed.binding.clone(),
            pressed: placed.pressed.clone(),
            cache: placed.cache.clone(),
            renderer,
        }
    }

    /// Whether this element arms the click hit-test: bound AND categorical.
    /// Sequential (gradient) and unbound legends stay display-only — no
    /// hitbox, no listeners (lcf ac-05).
    fn is_clickable(&self) -> bool {
        self.binding.is_some() && matches!(self.scale, Scale::Colour { .. })
    }
}

impl IntoElement for LegendElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LegendElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<gpui::Hitbox>;

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
        // A bound categorical legend registers a hitbox so the mouse-up
        // listener can scope clicks to this element (card 0009). Unbound or
        // Sequential legends stay display-only: no hitbox, no mouse events.
        self.is_clickable()
            .then(|| window.insert_hitbox(bounds, HitboxBehavior::Normal))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        // Bound categorical legend: register the press-origin recorder and
        // the mouse-up hit-test (card 0009). GPUI clears per-frame listeners
        // each frame, so both are re-registered every paint (the
        // chart_element.rs pattern). The hitbox scopes the gesture to this
        // element; the element origin maps the window-space release to
        // panel-local coordinates, which feed the swatch entry-rect hit-test
        // directly — the raster's border-pad inflation cancels out because
        // paint offsets the image by the same pad (see `swatch_hit_category`).
        if let (Some((legend_index, coordinator)), Some(hitbox)) = (&self.binding, &*prepaint) {
            if matches!(self.scale, Scale::Colour { .. }) {
                // Press-origin recorder (F6): every left press stamps whether
                // it landed on this panel; the flag rides the placement so it
                // survives element recreation until the release consumes it.
                window.on_mouse_event({
                    let pressed = self.pressed.clone();
                    let hitbox = hitbox.clone();
                    move |event: &MouseDownEvent, phase, window, _cx| {
                        if phase.bubble() && event.button == MouseButton::Left {
                            pressed.set(hitbox.is_hovered(window));
                        }
                    }
                });
                window.on_mouse_event({
                    let coordinator = coordinator.clone();
                    let hitbox = hitbox.clone();
                    let pressed = self.pressed.clone();
                    let scale = self.scale.clone();
                    let legend_index = *legend_index;
                    let origin = Point::new(bounds.origin.x.to_f64(), bounds.origin.y.to_f64());
                    move |event: &MouseUpEvent, phase, window, cx| {
                        if phase.bubble()
                            && event.button == MouseButton::Left
                            && legend_click_decision(&pressed, hitbox.is_hovered(window))
                        {
                            let local = Point::new(
                                event.position.x.to_f64() - origin.x,
                                event.position.y.to_f64() - origin.y,
                            );
                            let hit = swatch_hit_category(local, &scale);
                            let committed = coordinator
                                .borrow_mut()
                                .commit_legend_click(legend_index, hit.as_deref(), cx);
                            if committed {
                                window.refresh();
                            }
                        }
                    }
                });
            }
        }

        if self.width <= 0.0 || self.height <= 0.0 {
            return;
        }

        // The active category for a bound categorical legend, read from the
        // engine each paint (card 0006 selected-state) so the swatch dimming
        // tracks the contributor slot — no new invalidation plumbing: the same
        // gesture that changes the slot already triggers this repaint, and the
        // borrow is free because commits release theirs before the refresh. An
        // unbound or Sequential legend stays `None` (zero behaviour change).
        let selected: Option<String> = match (&self.binding, &self.scale) {
            (Some((legend_index, coordinator)), Scale::Colour { categories, .. }) => coordinator
                .borrow()
                .legend_selected_category(*legend_index, categories),
            _ => None,
        };

        // Match chart_state::base_image: render at the ceiled device size and
        // scale the scene to fill it, so the legend stays crisp on HiDPI —
        // padded by the border's overhang so the 0.5px edge stroke isn't
        // clipped (see legend_raster_geometry).
        let sf = f64::from(window.scale_factor().max(1.0));
        let (dev_w, dev_h, pad_dev) = legend_raster_geometry(self.width, self.height, sf);
        let pad_logical = f64::from(pad_dev) / sf;

        let cached = {
            let cache = self.cache.borrow();
            cache
                .as_ref()
                .filter(|c| raster_cache_hit((c.dev_w, c.dev_h), &c.selected, dev_w, dev_h, &selected))
                .map(|c| c.image.clone())
        };
        let image = match cached {
            Some(image) => image,
            None => {
                let Some((scene, _)) = build_legend_scene(&self.scale, selected.as_deref()) else {
                    return; // non-colour scale: nothing to paint
                };
                let scale_x = f64::from(dev_w - 2 * pad_dev) / self.width;
                let scale_y = f64::from(dev_h - 2 * pad_dev) / self.height;
                let mut scaled = Scene::new();
                // Whole-device-pixel translate applied AFTER the scale: the
                // content lands exactly pad_dev px in from each buffer edge,
                // so interior alignment matches the unpadded raster.
                scaled.append(
                    &scene,
                    Some(
                        Affine::translate((f64::from(pad_dev), f64::from(pad_dev)))
                            * Affine::scale_non_uniform(scale_x, scale_y),
                    ),
                );

                let mut pixels = self
                    .renderer
                    .lock()
                    .expect("VelloRenderer mutex poisoned")
                    .render_to_pixels(&scaled, dev_w, dev_h);
                // RenderImage expects BGRA.
                for px in pixels.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
                let buffer = image::RgbaImage::from_raw(dev_w, dev_h, pixels)
                    .expect("pixel buffer size mismatch");
                let image = Arc::new(RenderImage::new(smallvec::SmallVec::from_elem(
                    image::Frame::new(buffer),
                    1,
                )));
                *self.cache.borrow_mut() = Some(LegendRaster {
                    dev_w,
                    dev_h,
                    selected: selected.clone(),
                    image: image.clone(),
                });
                image
            }
        };

        // Paint at bounds inflated by the same logical pad per side, so the
        // panel's interior stays at its layout rect and only the border's
        // overhang spills into the (previously clipped) quarter-pixel fringe.
        let pad_px = px(pad_logical as f32);
        let padded = Bounds {
            origin: gpui::point(bounds.origin.x - pad_px, bounds.origin.y - pad_px),
            size: gpui::size(
                bounds.size.width + pad_px + pad_px,
                bounds.size.height + pad_px + pad_px,
            ),
        };
        let _ = window.paint_image(padded, Corners::default(), image, 0, false);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        legend_click_decision, legend_raster_geometry, raster_cache_hit, swatch_hit_category,
    };
    use brightfield_render::legend::swatch_entry_rects;
    use brightfield_render::scale::Scale;
    use kurbo::Point;
    use std::cell::Cell;

    /// cfr_ac06 (cache key): a bound categorical legend's raster cache keys on
    /// the selected category as well as the device dims, so a gesture that
    /// changes the slot repaints the legend on the refresh it already triggers.
    /// Same dims + same selection hits; a different (or newly-present) selection
    /// misses; a static legend (`None` both) always hits regardless of the slot.
    #[test]
    fn cfr_ac06_raster_cache_keys_on_dims_and_selected() {
        let g = Some("gentoo".to_string());
        let a = Some("adelie".to_string());

        // Same dims + same selection → hit.
        assert!(raster_cache_hit((100, 40), &g, 100, 40, &g));
        // Same dims + different selection → miss.
        assert!(!raster_cache_hit((100, 40), &g, 100, 40, &a));
        // No selection either side (unbound / Sequential) → hit.
        assert!(raster_cache_hit((100, 40), &None, 100, 40, &None));
        // A selection where there was none → miss.
        assert!(!raster_cache_hit((100, 40), &None, 100, 40, &g));
        // Different dims → miss regardless of selection.
        assert!(!raster_cache_hit((100, 40), &g, 200, 40, &g));
    }

    /// fww_ac04 (post-review, sub-pixel border parity): the raster buffer is
    /// padded by the panel border's 0.25 logical-px overhang, ceiled to whole
    /// device pixels per side, so the window legend paints the full stroke
    /// the PNG composite shows.
    #[test]
    fn fww_ac04_raster_buffer_pads_for_border_overhang() {
        // sf = 1: a 0.25px overhang ceils to a 1-device-px pad per side.
        assert_eq!(legend_raster_geometry(120.0, 24.0, 1.0), (122, 26, 1));
        // sf = 2 (Retina): 0.5 device px of overhang still ceils to 1.
        assert_eq!(legend_raster_geometry(120.0, 24.0, 2.0), (242, 50, 1));
        // sf = 8: exactly 2 device px per side.
        assert_eq!(legend_raster_geometry(120.0, 24.0, 8.0), (964, 196, 2));
        // Fractional logical sizes keep the ceil-then-pad order (content
        // ceils first, pad is added outside it).
        assert_eq!(legend_raster_geometry(120.5, 24.5, 2.0), (243, 51, 1));
    }

    // --- lcf_ac05 (card 0009): element-local click → swatch category ---

    fn colour_scale() -> Scale {
        Scale::Colour {
            categories: vec!["adelie".into(), "gentoo".into(), "chinstrap".into()],
            palette: vec![
                [0.306, 0.475, 0.655, 1.0],
                [0.949, 0.557, 0.169, 1.0],
                [0.882, 0.341, 0.349, 1.0],
            ],
        }
    }

    /// lcf_ac05: the hit-test maps element-local coordinates straight onto
    /// the entry rects at origin — the panel interior sits exactly at the
    /// element's layout bounds (the raster's border-pad inflation cancels in
    /// paint), so a click at each entry's centre resolves its category, a
    /// click in the panel padding between/around entries resolves None, and
    /// a Sequential scale never resolves (no discrete entries).
    #[test]
    fn lcf_ac05_swatch_hit_category_maps_local_click_to_entry() {
        let scale = colour_scale();
        let rects = swatch_entry_rects(0.0, 0.0, &scale);
        assert_eq!(rects.len(), 3);

        // Each entry's centre hits its own category, in order.
        for (rect, expected) in rects.iter().zip(["adelie", "gentoo", "chinstrap"]) {
            let hit = swatch_hit_category(rect.center(), &scale);
            assert_eq!(hit.as_deref(), Some(expected), "centre of {expected}'s row");
        }

        // The panel's top-left padding corner is outside every entry rect.
        assert_eq!(swatch_hit_category(Point::new(1.0, 1.0), &scale), None);
        // Between row 0 and row 1 (below the swatch, above the next row).
        let gap_y = (rects[0].y1 + rects[1].y0) / 2.0;
        assert_eq!(swatch_hit_category(Point::new(rects[0].center().x, gap_y), &scale), None);
        // Far outside the panel.
        assert_eq!(swatch_hit_category(Point::new(1e4, 1e4), &scale), None);

        // A Sequential (gradient) scale has no entries — inert.
        let sequential = Scale::Sequential {
            domain_min: 0.0,
            domain_max: 1.0,
            stops: vec![[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]],
        };
        assert_eq!(swatch_hit_category(Point::new(10.0, 10.0), &sequential), None);
    }

    /// Card 0009 F6: the click decision requires the press to have
    /// originated inside the panel AND the release to land inside it — all
    /// four press-in/out × release-in/out combinations — and always consumes
    /// the press flag so no stale press leaks into a later gesture (a brush
    /// drag released over the legend must not dispatch a click).
    #[test]
    fn lcf_f6_click_decision_requires_press_and_release_inside() {
        let pressed = Cell::new(false);

        // press-in × release-in → commits, flag consumed.
        pressed.set(true);
        assert!(legend_click_decision(&pressed, true));
        assert!(!pressed.get(), "the press flag is consumed by the decision");

        // press-in × release-out → no commit (started here, let go elsewhere).
        pressed.set(true);
        assert!(!legend_click_decision(&pressed, false));
        assert!(!pressed.get(), "consumed even when the release misses");

        // press-out × release-in → no commit (a drag arriving from a plot).
        pressed.set(false);
        assert!(!legend_click_decision(&pressed, true));

        // press-out × release-out → no commit.
        pressed.set(false);
        assert!(!legend_click_decision(&pressed, false));

        // The consumed flag means a release-only gesture later (no new
        // press on the panel) still cannot commit.
        assert!(!legend_click_decision(&pressed, true));
    }
}
