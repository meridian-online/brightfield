//! LegendElement — GPUI Element that paints one standalone legend (card 0016).
//!
//! The scene half (`legend_scene::build_legend_scene`) stays gpui-free; this
//! file is the window layer, mirroring `chart_element.rs`'s raster path:
//! vello scene → device-resolution RGBA (BGRA-swapped) → `RenderImage` → one
//! `paint_image` (the chart_state.rs choke point pattern, with the same
//! scale-factor-aware cache so repaints don't re-run Vello).
//!
//! **Display-only:** no mouse listeners, no coordinator, no hitbox. Legend
//! hit-testing arrives with the click-to-filter card; until then the element
//! paints what the headless composite paints and nothing more.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{
    px, App, Bounds, Corners, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, RenderImage, Size, Style, Window,
};
use kurbo::Affine;
use vello::Scene;

use brightfield_render::scale::Scale;

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
    /// Cached device-resolution raster, shared with the per-frame elements so
    /// hovering/toggling elsewhere never re-runs Vello for a static legend.
    cache: Rc<RefCell<Option<LegendRaster>>>,
}

impl PlacedLegend {
    /// A legend descriptor at rect `(x, y, width, height)` displaying `scale`.
    pub fn new(x: f64, y: f64, width: f64, height: f64, scale: Scale) -> Self {
        Self {
            x,
            y,
            width,
            height,
            scale,
            cache: Rc::new(RefCell::new(None)),
        }
    }
}

/// A cached device-resolution rasterisation of a legend scene (mirrors
/// `chart_state::BaseRaster`).
struct LegendRaster {
    dev_w: u32,
    dev_h: u32,
    image: Arc<RenderImage>,
}

/// GPUI element that paints one standalone legend. Created fresh each frame by
/// `ChartView::render()`; owns no state beyond clones of the placement's
/// descriptor fields and the shared raster cache.
pub struct LegendElement {
    scale: Scale,
    width: f64,
    height: f64,
    /// Stable, per-legend element id (position in the hosted legend list —
    /// display-only, so it indexes nothing else).
    id: ElementId,
    cache: Rc<RefCell<Option<LegendRaster>>>,
    renderer: Arc<Mutex<VelloRenderer>>,
}

impl LegendElement {
    /// Create a legend element for `placed`, sharing its raster cache.
    /// `index` distinguishes sibling legends for GPUI's element keying;
    /// `renderer` is the window's shared Vello renderer.
    pub fn new(placed: &PlacedLegend, index: usize, renderer: Arc<Mutex<VelloRenderer>>) -> Self {
        Self {
            scale: placed.scale.clone(),
            width: placed.width,
            height: placed.height,
            id: ElementId::from(("brightfield-legend", index)),
            cache: placed.cache.clone(),
            renderer,
        }
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
    type PrepaintState = ();

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
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        // Display-only: no hitbox, no mouse events.
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if self.width <= 0.0 || self.height <= 0.0 {
            return;
        }
        // Match chart_state::base_image: render at the ceiled device size and
        // scale the scene to fill it, so the legend stays crisp on HiDPI.
        let sf = f64::from(window.scale_factor().max(1.0));
        let dev_w = (self.width * sf).ceil().max(1.0) as u32;
        let dev_h = (self.height * sf).ceil().max(1.0) as u32;

        let cached = {
            let cache = self.cache.borrow();
            cache
                .as_ref()
                .filter(|c| c.dev_w == dev_w && c.dev_h == dev_h)
                .map(|c| c.image.clone())
        };
        let image = match cached {
            Some(image) => image,
            None => {
                let Some((scene, _)) = build_legend_scene(&self.scale) else {
                    return; // non-colour scale: nothing to paint
                };
                let scale_x = f64::from(dev_w) / self.width;
                let scale_y = f64::from(dev_h) / self.height;
                let mut scaled = Scene::new();
                scaled.append(&scene, Some(Affine::scale_non_uniform(scale_x, scale_y)));

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
                    image: image.clone(),
                });
                image
            }
        };

        let _ = window.paint_image(bounds, Corners::default(), image, 0, false);
    }
}
