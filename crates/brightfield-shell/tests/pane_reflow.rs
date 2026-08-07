//! Gate: a chart draws into the box it is handed, and follows that box when it
//! moves.
//!
//! Measured off the RASTER rather than off the layout tree, for the reason
//! `banded_bar_ink.rs` gives: `compute_layout` returning the right rect and the
//! picture being drawn at the right size are two claims, and a composition that
//! carried the box only as far as a struct field would satisfy the first while
//! the window went on presenting a 640x400 texture into a 320-point pane.
//!
//! The discriminating quantity is the plot frame's inset from each edge of the
//! box. A chart re-laid-out into a smaller box keeps that inset; a chart that
//! kept its declared size and was cropped to the box loses it, because the ink
//! then runs off the crop. So the assertion is an equality between two sizes
//! rather than a constant, and no margin token is transcribed here.
//!
//! Needs a wgpu adapter, as `banded_bar_ink.rs` and the shell's snapshot suite
//! already do on the same runner.

use brightfield_render::VelloRenderer;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::{compose_spec_str, compose_spec_str_at, LiveDashboard};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::layout::Rect;
use image::RgbaImage;

/// A scatter that declares 640x400 — the size the picture must stop being when
/// it is handed a smaller box.
const DECLARED_640_400: &str = "data:
  t:
    - { x: 1, y: 3 }
    - { x: 2, y: 5 }
    - { x: 3, y: 2 }
    - { x: 4, y: 8 }
    - { x: 5, y: 4 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
width: 640
height: 400
";

/// `examples/param-threshold.yaml` — the one shipped example that declares no
/// size at all, so the composition falls through to `DEFAULT_PLOT_WIDTH` /
/// `DEFAULT_PLOT_HEIGHT`.
fn undeclared_example() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/param-threshold.yaml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Raster measurement
// ---------------------------------------------------------------------------

/// Rasterise `composed` at its own composed size, through the renderer the
/// window and the PNG export use.
fn raster(composed: &brightfield_shell::pipeline::Composed, w: u32, h: u32) -> RgbaImage {
    let renderer = VelloRenderer::new();
    let pixels = renderer
        .lock()
        .expect("renderer poisoned")
        .render_to_pixels(&composed.scene, w, h);
    RgbaImage::from_raw(w, h, pixels).expect("vello pixel buffer size mismatch")
}

/// Per-channel tolerance separating the page tone from anything drawn over it.
/// Matches the mark-ink tolerance `banded_bar_ink.rs` measures with.
const PAGE_TOL: i32 = 20;

/// Whether `p` differs from the page tone — the page fill covers the whole
/// dashboard, so alpha cannot separate drawn ink from background.
fn is_ink(p: [u8; 4], page: [u8; 4]) -> bool {
    (0..3).any(|c| (i32::from(p[c]) - i32::from(page[c])).abs() > PAGE_TOL)
}

/// The tightest box containing every pixel that is not the page tone, as
/// `(left, top, right, bottom)` in pixels, `right`/`bottom` exclusive.
fn ink_bounds(img: &RgbaImage) -> (u32, u32, u32, u32) {
    let page = img.get_pixel(0, 0).0;
    let (w, h) = img.dimensions();
    let (mut left, mut top, mut right, mut bottom) = (w, h, 0_u32, 0_u32);
    for y in 0..h {
        for x in 0..w {
            if is_ink(img.get_pixel(x, y).0, page) {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    assert!(left < right && top < bottom, "the raster has ink in it");
    (left, top, right, bottom)
}

/// Anti-aliasing on a frame line moves an edge by at most a pixel, and the two
/// rasters are laid out to different sub-pixel offsets.
const EDGE_TOL: u32 = 2;

// ---------------------------------------------------------------------------
// AC1 — a box smaller than the declared size
// ---------------------------------------------------------------------------

#[test]
fn a_chart_handed_a_smaller_box_redraws_into_it_rather_than_being_cropped() {
    let (small_w, small_h) = (320_u32, 240_u32);

    let declared = compose_spec_str(DECLARED_640_400, None).expect("the spec composes");
    assert_eq!(
        (declared.width, declared.height),
        (640, 400),
        "unoffered, the composition is the size the spec declares"
    );

    let reflowed = compose_spec_str_at(
        DECLARED_640_400,
        None,
        Rect::new(0.0, 0.0, f64::from(small_w), f64::from(small_h)),
    )
    .expect("the spec composes into the offered box");
    assert_eq!(
        (reflowed.width, reflowed.height),
        (small_w, small_h),
        "the composition takes the offered box over the declared size"
    );

    // The picture, at each size, in its own raster.
    let big_img = raster(&declared, declared.width, declared.height);
    let small_img = raster(&reflowed, reflowed.width, reflowed.height);

    let (bl, bt, br, bb) = ink_bounds(&big_img);
    let (sl, st, sr, sb) = ink_bounds(&small_img);

    // The frame's inset from each edge of the box is the same at both sizes:
    // the picture was laid out again, not cropped and not scaled.
    let big_insets = (bl, bt, declared.width - br, declared.height - bb);
    let small_insets = (sl, st, reflowed.width - sr, reflowed.height - sb);
    for (edge, big, small) in [
        ("left", big_insets.0, small_insets.0),
        ("top", big_insets.1, small_insets.1),
        ("right", big_insets.2, small_insets.2),
        ("bottom", big_insets.3, small_insets.3),
    ] {
        assert!(
            big.abs_diff(small) <= EDGE_TOL,
            "the {edge} inset is {big}px in the 640x400 picture and {small}px in the \
             {small_w}x{small_h} one — {big_insets:?} vs {small_insets:?}"
        );
    }

    // And the ink spans the small box: a picture drawn at 640 and cropped to
    // 320 would have its right-hand ink outside the raster entirely.
    assert!(
        sr > small_w / 2,
        "ink reaches x={sr} of {small_w} — the right half of the box is empty"
    );
    assert!(
        sb > small_h / 2,
        "ink reaches y={sb} of {small_h} — the bottom half of the box is empty"
    );
}

// ---------------------------------------------------------------------------
// AC2 — no declared size
// ---------------------------------------------------------------------------

#[test]
fn a_chart_with_no_declared_size_is_composed_into_the_box_it_is_given() {
    let source = undeclared_example();

    let unoffered = compose_spec_str(&source, None).expect("the example composes");
    assert_eq!(
        (unoffered.width, unoffered.height),
        (640, 400),
        "unoffered, an example that declares no size falls through to the defaults"
    );

    for (w, h) in [(320_u32, 240_u32), (900_u32, 700_u32)] {
        let composed = compose_spec_str_at(
            &source,
            None,
            Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
        )
        .expect("the example composes into the offered box");
        assert_eq!(
            (composed.width, composed.height),
            (w, h),
            "a spec with no declared size takes the box it is given"
        );

        let img = raster(&composed, composed.width, composed.height);
        let (_, _, right, bottom) = ink_bounds(&img);
        assert!(
            right > w / 2 && bottom > h / 2,
            "the picture fills the {w}x{h} box it was given — ink stops at ({right}, {bottom})"
        );
    }
}

// ---------------------------------------------------------------------------
// AC3 — the box moves under a running window
// ---------------------------------------------------------------------------

/// A live chart window at `size`, run for `frames` frames.
fn run_at(app: &mut MeridianApp, ctx: &egui::Context, size: egui::Vec2, frames: usize) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
        ..Default::default()
    };
    for _ in 0..frames {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
}

/// The composed size and the pane box the chart pane was handed, this frame.
fn composed_and_pane(app: &MeridianApp) -> ((u32, u32), egui::Rect) {
    let doc: &ChartDoc = app.chart_doc();
    (
        (doc.composed.width, doc.composed.height),
        app.chart_viewport()
            .expect("the chart pane drew, so it recorded its box"),
    )
}

#[test]
fn resizing_the_window_relays_out_the_chart_without_a_restart() {
    let mut live = LiveDashboard::load_str(DECLARED_640_400, None).expect("loads live");
    let composed = live.present().expect("first paint");
    let boot = Boot {
        live: Some(live),
        ..Boot::charts(composed)
    };

    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    let ctx = egui::Context::default();

    // Two frames per size: the first installs the font atlas and settles the
    // dock, exactly as `chart_contract.rs` describes.
    let wide = egui::vec2(1400.0, 900.0);
    let narrow = egui::vec2(900.0, 620.0);

    run_at(&mut app, &ctx, wide, 3);
    let (wide_composed, wide_pane) = composed_and_pane(&app);

    run_at(&mut app, &ctx, narrow, 3);
    let (narrow_composed, narrow_pane) = composed_and_pane(&app);

    assert!(
        narrow_pane.width() < wide_pane.width(),
        "the narrower window gives the chart pane a narrower box: \
         {} then {}",
        wide_pane.width(),
        narrow_pane.width()
    );
    assert!(
        narrow_composed.0 < wide_composed.0 && narrow_composed.1 < wide_composed.1,
        "the composition followed the window down: {wide_composed:?} then {narrow_composed:?}"
    );
    assert_ne!(
        wide_composed,
        (640, 400),
        "the chart stopped being the size its spec declares"
    );

    // And it goes back up, on the same process, with no reload.
    run_at(&mut app, &ctx, wide, 3);
    let (again, _) = composed_and_pane(&app);
    assert_eq!(
        again, wide_composed,
        "the same window size composes the same picture, whichever size it came from"
    );
}
