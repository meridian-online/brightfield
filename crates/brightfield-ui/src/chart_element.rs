//! Chart paint logic — framework-free, routed through the [`ChartSurface`]
//! boundary.
//!
//! This module holds the *logic* of the deepest chart shell without naming any
//! host (gpui) element or paint type. Each frame the host (see
//! [`crate::gpui_canvas`]) drives [`paint_chart`] with a [`ChartSurface`]:
//!   1. present the chart's current scene + reserve its on-screen rect,
//!   2. draw the interaction overlay (brush rect / hover marker / selection
//!      highlight) via the surface's [`OverlayPainter`], and
//!   3. set the position-dependent cursor over any persisted selection.
//!
//! Pointer input is gathered by the host into [`SurfaceInput`] and routed here
//! through [`route_pointer_down`] / [`route_pointer_move`] / [`redispatch_target`]
//! into the EXISTING interaction transitions on [`ChartState`] (unchanged) — the
//! host owns only the framework glue (window refresh, cross-filter commit,
//! per-frame listener re-registration), never the interaction semantics.

use crate::canvas_host::{
    ChartSurface, Color, OverlayPainter, PixelSize, SurfaceCursor, SurfaceInput, SurfaceRect,
};
use crate::chart_state::ChartState;
use crate::interaction::{
    redispatch_brushing_from, BrushCorner, BrushEdge, BrushRegion, InteractionState,
};
use kurbo::Point;
use meridian_design::chrome::OVERLAY_LIGHT;

/// Hover marker radius in logical pixels (mirrors `interaction::render_overlay`).
const HOVER_RADIUS: f64 = 8.0;

/// Paint one chart frame through the host surface. The host has already
/// registered this frame's input listeners; here we present, overlay, and set
/// the cursor — the paint-phase half of the lifecycle, framework-free.
///
/// A zero-size plot presents nothing (the listeners still ran) — mirroring the
/// pre-refactor early return before rasterisation.
pub fn paint_chart(
    surface: &mut dyn ChartSurface,
    interaction: &InteractionState,
    region: BrushRegion,
    size: PixelSize,
) {
    if size.width == 0 || size.height == 0 {
        return; // nothing to rasterise; the host's input listeners already ran
    }

    // Present the cached, device-resolution base raster and reserve the rect,
    // then draw the interaction overlay on top as cheap host primitives — so
    // hovering/brushing never re-run Vello.
    let _bounds = surface.present(size);
    draw_overlay(interaction, surface.overlay());

    // Position-dependent cursor over the persisted selection: the
    // grab region was tracked by the host's mouse-move listener; re-pick the
    // cursor from it each paint. No cursor over `Outside` or when nothing is
    // selected (`overlay_cursor` returns `None`).
    let dragging = matches!(interaction, InteractionState::Dragging { .. });
    surface.set_cursor(overlay_cursor(region, dragging));
}

/// Draw the interaction overlay (brush rectangle / hover marker / selection
/// highlight) via the host's [`OverlayPainter`]. Coordinates are surface-local
/// logical pixels — the same space the interaction state stores — and the host
/// offsets them by the surface origin. Drawing the overlay as host primitives
/// (rather than compositing into the Vello scene) means an interaction repaint
/// reuses the cached base raster, and keeps example PNGs byte-identical (the
/// overlay never enters the scene).
pub fn draw_overlay(interaction: &InteractionState, painter: &mut dyn OverlayPainter) {
    match interaction {
        InteractionState::Idle => {}
        InteractionState::Brushing { start, current } => {
            let rect = norm_surface_rect(*start, *current);
            // The active drag rect is interactive, so it wears Maritime (the
            // Meridian design rule: interactive/focus/selection = Maritime,
            // chrome stays warm-neutral) — the focus-ring token as a light wash
            // for the fill and stronger for the border.
            painter.fill_rect(
                rect,
                Color::from_token_alpha(OVERLAY_LIGHT.focus_ring, 0.15),
            );
            painter.stroke_rect(
                rect,
                Color::from_token_alpha(OVERLAY_LIGHT.focus_ring, 0.75),
                1.5,
            );
        }
        // A committed selection and an in-flight move/resize paint identically —
        // a neutral ink wash, so it reads as settled vs the active Maritime drag
        // (Mosaic / Vega-Lite fidelity).
        InteractionState::Selected { start, current }
        | InteractionState::Dragging { start, current, .. } => {
            let rect = norm_surface_rect(*start, *current);
            painter.fill_rect(rect, Color::from_token(OVERLAY_LIGHT.brush_fill));
            painter.stroke_rect(rect, Color::from_token(OVERLAY_LIGHT.brush_border), 1.5);
        }
        InteractionState::Hovering { point: p, .. } => {
            // Hover disc tracks categorical slot 2 (Harbour gold) so it stays an
            // accent DISTINCT from the slot-1 blue default marks it sits over —
            // the same "palette slot 2" convention the old Tableau10 orange
            // followed. Translucent, same historical alpha.
            let slot2 = meridian_design::viz::CATEGORICAL_LIGHT[1];
            painter.fill_circle(
                *p,
                HOVER_RADIUS,
                Color {
                    r: slot2.r,
                    g: slot2.g,
                    b: slot2.b,
                    a: 0.376,
                },
            );
        }
    }
}

/// Normalise two brush corners into a surface rectangle (min corner + extent),
/// the same min/max the pre-refactor overlay computed.
fn norm_surface_rect(start: Point, current: Point) -> SurfaceRect {
    let x0 = start.x.min(current.x);
    let y0 = start.y.min(current.y);
    let w = start.x.max(current.x) - x0;
    let h = start.y.max(current.y) - y0;
    SurfaceRect::new(x0, y0, w, h)
}

/// Map a grab region to its surface cursor: an open hand
/// over the interior (closed while dragging), a horizontal/vertical resize on an
/// edge, a diagonal resize on a corner. `Outside` sets no cursor (the plot
/// default). The host maps [`SurfaceCursor`] to its own cursor type.
pub fn overlay_cursor(region: BrushRegion, dragging: bool) -> Option<SurfaceCursor> {
    match region {
        BrushRegion::Interior => Some(if dragging {
            SurfaceCursor::Grabbing
        } else {
            SurfaceCursor::Grab
        }),
        BrushRegion::Edge(BrushEdge::Left | BrushEdge::Right) => {
            Some(SurfaceCursor::ResizeHorizontal)
        }
        BrushRegion::Edge(BrushEdge::Top | BrushEdge::Bottom) => {
            Some(SurfaceCursor::ResizeVertical)
        }
        BrushRegion::Corner(BrushCorner::TopLeft | BrushCorner::BottomRight) => {
            Some(SurfaceCursor::ResizeNwSe)
        }
        BrushRegion::Corner(BrushCorner::TopRight | BrushCorner::BottomLeft) => {
            Some(SurfaceCursor::ResizeNeSw)
        }
        BrushRegion::Outside => None,
    }
}

// --- Pointer input routing -------------------------------------------------
//
// The host gathers each native pointer event into a `SurfaceInput` and calls
// these; they translate it into the EXISTING interaction transitions on
// `ChartState` (which delegate to `interaction.rs`). Only the framework glue —
// entity update, window refresh, cross-filter commit — stays host-side.

/// The result of routing a pointer-move: whether the interaction changed, and
/// the (possibly new) grab region under the pointer for the paint-phase cursor.
#[derive(Clone, Copy, Debug)]
pub struct MoveOutcome {
    /// Whether the interaction state changed (host should refresh).
    pub changed: bool,
    /// The grab region under the pointer, for the paint-phase cursor.
    pub region: BrushRegion,
}

/// Route a pointer-down. A press over the hitbox begins a brush / grabs a
/// persisted selection (the `pointer_down` resolver decides which). Returns
/// `true` when the interaction changed. A non-primary press, or one off the
/// hitbox, is ignored — matching the pre-refactor left-button + hovered gate.
pub fn route_pointer_down(input: &SurfaceInput, state: &mut ChartState, origin: Point) -> bool {
    if input.pointer_primary.is_down() && input.hovered {
        if let Some(pos) = input.pointer_pos {
            return state.pointer_down(pos, origin);
        }
    }
    false
}

/// Route a pointer-move: extend the brush / move-resize the grab while the
/// primary button is held, or update hover; and re-classify the pointer over any
/// persisted selection so the paint-phase cursor tracks the region under it.
pub fn route_pointer_move(
    input: &SurfaceInput,
    state: &mut ChartState,
    origin: Point,
) -> MoveOutcome {
    let Some(pos) = input.pointer_pos else {
        return MoveOutcome {
            changed: false,
            region: BrushRegion::Outside,
        };
    };
    let held = input.pointer_primary.is_down();
    let changed = state.pointer_move(pos, origin, held);
    let region = state.cursor_region(pos, origin);
    MoveOutcome { changed, region }
}

/// The interaction to commit on release. A move/resize that ends in `Dragging`
/// is synthesised into a pixel-space `Brushing` from its NEW corners so the moved
/// range re-dispatches (the drag defence); every other state passes through
/// unchanged. The host feeds the result into the cross-filter coordinator BEFORE
/// `ChartState::pointer_up` clears the gesture.
pub fn redispatch_target(state: &ChartState) -> InteractionState {
    let interaction = state.interaction().clone();
    redispatch_brushing_from(&interaction).unwrap_or(interaction)
}

#[cfg(test)]
mod tests {
    use super::{draw_overlay, overlay_cursor};
    use crate::canvas_host::{Color, OverlayPainter, SurfaceCursor, SurfaceRect};
    use crate::interaction::{BrushCorner, BrushEdge, BrushRegion, InteractionState};
    use kurbo::Point;

    /// Mapping: the region→cursor mapping is a pure fn — open hand over
    /// the interior (closed while dragging), horizontal/vertical resize on edges,
    /// diagonal resize on corners, no cursor over Outside. (The host maps each
    /// `SurfaceCursor` to its own glyph — pinned in `gpui_canvas`; the live glyph
    /// and its change-on-motion are Hugh's in-app eyeball.)
    #[test]
    fn region_cursor_mapping() {
        assert_eq!(
            overlay_cursor(BrushRegion::Interior, false),
            Some(SurfaceCursor::Grab)
        );
        assert_eq!(
            overlay_cursor(BrushRegion::Interior, true),
            Some(SurfaceCursor::Grabbing)
        );
        assert_eq!(
            overlay_cursor(BrushRegion::Edge(BrushEdge::Left), false),
            Some(SurfaceCursor::ResizeHorizontal)
        );
        assert_eq!(
            overlay_cursor(BrushRegion::Edge(BrushEdge::Right), false),
            Some(SurfaceCursor::ResizeHorizontal)
        );
        assert_eq!(
            overlay_cursor(BrushRegion::Edge(BrushEdge::Top), false),
            Some(SurfaceCursor::ResizeVertical)
        );
        assert_eq!(
            overlay_cursor(BrushRegion::Edge(BrushEdge::Bottom), false),
            Some(SurfaceCursor::ResizeVertical)
        );
        assert_eq!(
            overlay_cursor(BrushRegion::Corner(BrushCorner::TopLeft), false),
            Some(SurfaceCursor::ResizeNwSe)
        );
        assert_eq!(
            overlay_cursor(BrushRegion::Corner(BrushCorner::BottomRight), false),
            Some(SurfaceCursor::ResizeNwSe)
        );
        assert_eq!(
            overlay_cursor(BrushRegion::Corner(BrushCorner::TopRight), false),
            Some(SurfaceCursor::ResizeNeSw)
        );
        assert_eq!(
            overlay_cursor(BrushRegion::Corner(BrushCorner::BottomLeft), false),
            Some(SurfaceCursor::ResizeNeSw)
        );
        // Outside → no cursor (the plot default), whether or not "dragging".
        assert_eq!(overlay_cursor(BrushRegion::Outside, false), None);
        assert_eq!(overlay_cursor(BrushRegion::Outside, true), None);
    }

    /// Records overlay primitive calls so the pure overlay-decision logic can be
    /// asserted without a live window (the host paint is Hugh's in-app eyeball).
    #[derive(Default)]
    struct RecordingPainter {
        rects: Vec<(SurfaceRect, Color)>,
        strokes: Vec<(SurfaceRect, Color, f32)>,
        circles: Vec<(Point, f64, Color)>,
    }
    impl OverlayPainter for RecordingPainter {
        fn fill_rect(&mut self, r: SurfaceRect, c: Color) {
            self.rects.push((r, c));
        }
        fn stroke_rect(&mut self, r: SurfaceRect, c: Color, w: f32) {
            self.strokes.push((r, c, w));
        }
        fn fill_circle(&mut self, center: Point, radius: f64, c: Color) {
            self.circles.push((center, radius, c));
        }
        fn line(&mut self, _a: Point, _b: Point, _c: Color, _w: f32) {}
        fn text(&mut self, _at: Point, _s: &str, _c: Color, _size: f32) {}
    }

    /// The overlay decision per interaction state: `Idle` draws nothing; a brush
    /// draws a normalised fill + 1.5px border; a selection/drag draws the settled
    /// wash; a hover draws an 8px-radius disc — the same primitives the
    /// pre-refactor `paint_overlay` emitted, now framework-free.
    #[test]
    fn draw_overlay_emits_expected_primitives() {
        // Idle → nothing.
        let mut p = RecordingPainter::default();
        draw_overlay(&InteractionState::Idle, &mut p);
        assert!(p.rects.is_empty() && p.strokes.is_empty() && p.circles.is_empty());

        // Brushing → one normalised fill + one 1.5px border (corners normalised).
        let mut p = RecordingPainter::default();
        draw_overlay(
            &InteractionState::Brushing {
                start: Point::new(120.0, 40.0),
                current: Point::new(20.0, 90.0),
            },
            &mut p,
        );
        assert_eq!(p.rects.len(), 1);
        assert_eq!(p.strokes.len(), 1);
        let (rect, _) = p.rects[0];
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (20.0, 40.0, 100.0, 50.0)
        );
        assert_eq!(p.strokes[0].2, 1.5);
        assert!(p.circles.is_empty());

        // Selected → the settled fill + border (same shape as a drag).
        let mut p = RecordingPainter::default();
        draw_overlay(
            &InteractionState::Selected {
                start: Point::new(10.0, 10.0),
                current: Point::new(30.0, 50.0),
            },
            &mut p,
        );
        assert_eq!(p.rects.len(), 1);
        assert_eq!(p.strokes.len(), 1);

        // Hovering → one 8px-radius disc at the point, alpha 0.376.
        let mut p = RecordingPainter::default();
        draw_overlay(
            &InteractionState::Hovering {
                point: Point::new(64.0, 48.0),
                nearest: None,
            },
            &mut p,
        );
        assert_eq!(p.circles.len(), 1);
        let (center, radius, color) = p.circles[0];
        assert_eq!(center, Point::new(64.0, 48.0));
        assert_eq!(radius, 8.0);
        assert!((color.a - 0.376).abs() < 1e-6);
        assert!(p.rects.is_empty() && p.strokes.is_empty());
    }
}
