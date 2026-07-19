//! CanvasHost / ChartSurface — the render/host boundary.
//!
//! These traits factor the chart's paint path into a framework-free seam. The
//! chart's logic (building the scene, interpreting brush/hover input, deciding
//! the overlay) is expressed against these traits; the framework that actually
//! owns a GPU device, a window, and a pointer implements them. Implementors: the
//! gpui host (`brightfield_ui::gpui_canvas`, a `RenderImage` via GPU readback —
//! the transitional present path) and the egui host (`brightfield_shell`, a
//! `vello::Scene` rendered to a texture on eframe's shared wgpu device, presented
//! zero-copy as an `egui::TextureId` — the readback deleted). The seam lets a new
//! host drop in without touching the chart logic.
//!
//! - [`CanvasHost`] owns the shared wgpu device/queue and turns a `vello::Scene`
//!   into a host-displayable handle (the gpui host: a `RenderImage` via GPU
//!   readback — the transitional present path).
//! - [`ChartSurface`] is the per-chart, per-frame boundary: present the built
//!   scene, reserve the on-screen rect, hand back an [`OverlayPainter`], and set
//!   the pointer cursor.
//! - [`OverlayPainter`] draws the interaction overlay (brush rect, hover marker,
//!   selection highlight) ON TOP of the presented scene. The chart *marks* stay
//!   in the Vello scene; only the transient overlay rides this painter, so an
//!   interaction repaint never re-runs Vello.
//!
//! Geometry is surface-local logical pixels in the crate's existing `kurbo`
//! space, so the interaction state (already `kurbo::Point`) flows through
//! unconverted. Colour is straight-alpha sRGB, framework-free — the host
//! converts to its own colour type at the boundary.

use kurbo::{Point, Vec2};
use vello::wgpu;
use vello::Scene;

/// A device-pixel size (the raster target for [`CanvasHost::present_scene`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelSize {
    /// Width in device pixels.
    pub width: u32,
    /// Height in device pixels.
    pub height: u32,
}

/// A surface-local rectangle in logical pixels: `(x, y)` origin plus size. The
/// same space the interaction state stores its brush corners in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceRect {
    /// Left edge (logical pixels, surface-local).
    pub x: f64,
    /// Top edge (logical pixels, surface-local).
    pub y: f64,
    /// Width (logical pixels).
    pub width: f64,
    /// Height (logical pixels).
    pub height: f64,
}

impl SurfaceRect {
    /// Construct a rectangle from its origin and size.
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }
}

/// A straight-alpha sRGB colour (components 0–1), framework-free. Mirrors
/// `meridian_design::Rgba`; the host converts to its own colour type at the
/// boundary (the gpui host: `gpui::Rgba` → `Hsla`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// Red (0–1).
    pub r: f32,
    /// Green (0–1).
    pub g: f32,
    /// Blue (0–1).
    pub b: f32,
    /// Straight alpha (0–1).
    pub a: f32,
}

impl Color {
    /// Fully transparent — the base colour the transitional gpui present clears
    /// to (the overlay and chart ink carry their own alpha).
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

    /// A Meridian design token, keeping the token's own alpha.
    pub fn from_token(c: meridian_design::Rgba) -> Self {
        Self { r: c.r, g: c.g, b: c.b, a: c.a }
    }

    /// A Meridian design token at an overridden alpha (a token reused at several
    /// strengths — e.g. the active drag rect wears the focus-ring token as a
    /// light fill and a stronger border).
    pub fn from_token_alpha(c: meridian_design::Rgba, a: f32) -> Self {
        Self { r: c.r, g: c.g, b: c.b, a }
    }

    /// Convert to a Vello `peniko::Color` (for the base clear colour).
    pub fn into_peniko(self) -> vello::peniko::Color {
        vello::peniko::Color::new([self.r, self.g, self.b, self.a])
    }
}

/// Keyboard modifier state at the time of a pointer event. Framework-free; the
/// host fills it from its own event. (Gathered for the boundary contract; the
/// current brush/hover overlay does not read it.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// Shift held.
    pub shift: bool,
    /// Control held.
    pub control: bool,
    /// Alt / Option held.
    pub alt: bool,
    /// Platform (Command on macOS, Super/Win elsewhere) held.
    pub platform: bool,
}

/// The up/down state of a pointer button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    /// The button is up.
    Up,
    /// The button is down.
    Down,
}

impl ButtonState {
    /// Whether the button is currently pressed.
    pub fn is_down(self) -> bool {
        matches!(self, ButtonState::Down)
    }
}

/// One frame of surface-local pointer input — exactly the events the brush /
/// crossfilter / interaction logic needs, in the surface's own coordinate space.
/// The host gathers each of its native pointer events into this shape before
/// routing it into the (framework-free) interaction transitions.
#[derive(Clone, Copy, Debug)]
pub struct SurfaceInput {
    /// Pointer position in surface-local logical pixels, or `None` when the
    /// pointer is absent.
    pub pointer_pos: Option<Point>,
    /// The primary (left) button.
    pub pointer_primary: ButtonState,
    /// The secondary (right) button.
    pub pointer_secondary: ButtonState,
    /// Drag delta since the previous frame (logical pixels).
    pub drag_delta: Vec2,
    /// Scroll-wheel delta since the previous frame (logical pixels).
    pub scroll_delta: Vec2,
    /// Keyboard modifiers at event time.
    pub modifiers: Modifiers,
    /// Whether the pointer is over this surface's hitbox.
    pub hovered: bool,
}

impl Default for SurfaceInput {
    fn default() -> Self {
        Self {
            pointer_pos: None,
            pointer_primary: ButtonState::Up,
            pointer_secondary: ButtonState::Up,
            drag_delta: Vec2::ZERO,
            scroll_delta: Vec2::ZERO,
            modifiers: Modifiers::default(),
            hovered: false,
        }
    }
}

/// A pointer cursor over the surface, framework-free. The host maps each variant
/// to its own cursor type (the gpui host: an open/closed hand over the interior,
/// axis/diagonal resize handles on edges/corners).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceCursor {
    /// Over a grabbable interior (translate) — open hand.
    Grab,
    /// Actively translating — closed hand.
    Grabbing,
    /// Resize along the horizontal axis (a left/right edge).
    ResizeHorizontal,
    /// Resize along the vertical axis (a top/bottom edge).
    ResizeVertical,
    /// Diagonal resize, top-left ↔ bottom-right corner.
    ResizeNwSe,
    /// Diagonal resize, top-right ↔ bottom-left corner.
    ResizeNeSw,
}

/// The response a [`ChartSurface`] produces for a frame — the reserved on-screen
/// rect plus the surface-local input. (The overlay pass is borrowed from the
/// surface via [`ChartSurface::overlay`] rather than owned here, because the gpui
/// overlay painter borrows the live window for the duration of the pass.)
#[derive(Clone, Copy, Debug)]
pub struct SurfaceResponse {
    /// The rect the surface reserved on screen (window-space logical pixels).
    pub bounds: SurfaceRect,
    /// This frame's surface-local pointer input.
    pub input: SurfaceInput,
}

/// Owns the shared wgpu device/queue; turns a `vello::Scene` into a
/// host-displayable handle. The gpui host's `Surface` is a `RenderImage`
/// produced by GPU readback (the transitional present path).
pub trait CanvasHost {
    /// The host-displayable handle a presented scene becomes.
    type Surface;

    /// The wgpu device the host renders on (a cheap handle clone).
    fn device(&self) -> wgpu::Device;

    /// The wgpu queue the host submits on (a cheap handle clone).
    fn queue(&self) -> wgpu::Queue;

    /// Rasterise `scene` at `size` over the `base` clear colour and return a
    /// host-displayable handle.
    fn present_scene(&mut self, scene: &Scene, size: PixelSize, base: Color) -> Self::Surface;
}

/// Draws the interaction overlay on top of the presented scene. Coordinates are
/// surface-local logical pixels (the space the interaction state stores); the
/// host offsets them by the surface origin. The chart marks stay in the Vello
/// scene — only the transient overlay rides this painter.
pub trait OverlayPainter {
    /// A filled rectangle.
    fn fill_rect(&mut self, r: SurfaceRect, c: Color);
    /// A rectangle outline of stroke width `w`.
    fn stroke_rect(&mut self, r: SurfaceRect, c: Color, w: f32);
    /// A filled disc of radius `radius` centred at `center` (the hover marker).
    fn fill_circle(&mut self, center: Point, radius: f64, c: Color);
    /// A line segment of stroke width `w`.
    fn line(&mut self, a: Point, b: Point, c: Color, w: f32);
    /// A run of text at `at`, of the given font `size`.
    fn text(&mut self, at: Point, s: &str, c: Color, size: f32);
}

/// The per-chart render/host boundary. Each frame the chart hands over the built
/// scene + logical size; the surface presents it, reserves the on-screen rect,
/// exposes an [`OverlayPainter`] for the transient overlay, and sets the pointer
/// cursor. Input is delivered separately (see the module docs on the gpui host):
/// the host's native pointer events are gathered into [`SurfaceInput`] and routed
/// into the interaction transitions.
pub trait ChartSurface {
    /// Present the chart's current scene at `size` and reserve its on-screen
    /// rect. Returns the reserved rect (window-space logical pixels).
    fn present(&mut self, size: PixelSize) -> SurfaceRect;

    /// The overlay painter for this frame.
    fn overlay(&mut self) -> &mut dyn OverlayPainter;

    /// Set (or, with `None`, leave unset) the pointer cursor over the reserved
    /// rect.
    fn set_cursor(&mut self, cursor: Option<SurfaceCursor>);
}
