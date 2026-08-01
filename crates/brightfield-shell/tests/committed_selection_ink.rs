//! **A committed cross-filter has ink on the plot that produced it.**
//!
//! Cross-filtering worked and pictured half of itself. The receiving plot
//! narrowed; the plot the gesture happened on drew a rectangle from the drag
//! state, which `drive_gestures` took on release — so the instant a selection
//! became real it stopped being drawn, and a reader could see that a filter had
//! happened without being able to see what it was.
//!
//! Everything below is asserted **through the rendered raster**, never through
//! the drag state, and every gesture is a real pointer sweep through the whole
//! window: press, move, release, on the raster the last frame presented. That
//! pairing is the point. A test that read `ChartItem::drag` would pass on the
//! code as it stood before this file existed, because the drag state was
//! already right — it was the picture that was missing.
//!
//! Five things are held:
//!
//! - **Rest draws no band.** The floor, asserted first so a harness that finds
//!   this ink everywhere (wrong colour, wrong export path) says so here rather
//!   than passing everything below vacuously.
//! - **A gesture in progress draws no band either.** Between press and release
//!   the chart ink is untouched — the sweep is an egui quad over the raster and
//!   the raster is what this reads. That is the distinction between the two
//!   treatments, measured rather than asserted about tokens.
//! - **Release puts the band in.** Two bound rules, both inside the plot that
//!   produced the gesture and none in the plot receiving it.
//! - **The band is the clause, not a decoration.** Brushing an adjoining range
//!   moves it, and the bound the two sweeps share lands on the same pixel.
//! - **Clearing removes it.** A click with no sweep retracts an interval
//!   contribution, and the ink goes with it.

use brightfield_render::VelloRenderer;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::live_spec;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};

use image::RgbaImage;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// The ink being measured
// ---------------------------------------------------------------------------

/// The committed band's bound rules, as sRGB bytes.
///
/// Read from the token the renderer draws with rather than transcribed, so a
/// palette bump moves the expectation with the picture.
fn bound_ink() -> [i32; 3] {
    let c = meridian_design::chrome::INK_LIGHT.focus;
    [
        (c.r * 255.0).round() as i32,
        (c.g * 255.0).round() as i32,
        (c.b * 255.0).round() as i32,
    ]
}

/// Per-channel tolerance: the rule's core and the near-opaque end of its
/// anti-aliasing.
///
/// Wide enough to catch a 1.5px rule however it lands on the pixel grid, and
/// narrow enough that nothing else in this fixture's picture is inside it —
/// the mark ink, its blend against the background, the gridlines, the axis
/// rules and the tick labels are each further than this from the focus hue.
/// `the_bands_ink_is_not_the_in_progress_gestures_ink` holds the one
/// separation that a token bump could close.
const BOUND_TOL: i32 = 24;

fn is_bound(p: [u8; 4], want: [i32; 3]) -> bool {
    (0..3).all(|c| (i32::from(p[c]) - want[c]).abs() <= BOUND_TOL)
}

// ---------------------------------------------------------------------------
// Rasterising what the window is presenting
// ---------------------------------------------------------------------------

/// The picture the document is showing, rendered off its composed scene
/// through the same Vello renderer the window and the PNG export use.
///
/// Borrows the scene rather than taking the `Composed`, which is what lets a
/// stage be read WITHOUT ending it — the mid-drag frame below could not be
/// captured by a path that consumed the document's composition.
fn raster(renderer: &Arc<Mutex<VelloRenderer>>, doc: &ChartDoc) -> RgbaImage {
    let (w, h) = (doc.composed.width, doc.composed.height);
    let pixels = renderer
        .lock()
        .expect("renderer poisoned")
        .render_to_pixels(&doc.composed.scene, w, h);
    RgbaImage::from_raw(w, h, pixels).expect("vello pixel buffer size mismatch")
}

/// Bound-ink pixels per image column, left to right.
fn bound_columns(img: &RgbaImage) -> Vec<u32> {
    let want = bound_ink();
    let (w, h) = img.dimensions();
    (0..w)
        .map(|x| (0..h).filter(|&y| is_bound(img.get_pixel(x, y).0, want)).count() as u32)
        .collect()
}

/// The bound rules in a picture, as the pixel centre of each run of adjacent
/// heavily-inked columns.
///
/// A rule 1.5px wide lands on one or two columns depending on where its centre
/// falls between pixels, so runs are grouped before being counted — otherwise
/// the same rule reads as one bound or two according to sub-pixel luck.
///
/// "Heavily inked" is a third of the image height: a bound rule spans the whole
/// plot area, and nothing else in this fixture puts this hue down a column at
/// all.
fn rules(img: &RgbaImage) -> Vec<f64> {
    let floor = img.height() / 3;
    let columns = bound_columns(img);
    let mut found = Vec::new();
    let mut run: Option<(usize, usize)> = None;
    for (x, &count) in columns.iter().enumerate() {
        match (count > floor, run) {
            (true, None) => run = Some((x, x)),
            (true, Some((start, _))) => run = Some((start, x)),
            (false, Some((start, end))) => {
                found.push((start + end) as f64 / 2.0);
                run = None;
            }
            (false, None) => {}
        }
    }
    if let Some((start, end)) = run {
        found.push((start + end) as f64 / 2.0);
    }
    found
}

// ---------------------------------------------------------------------------
// The fixture and its gestures
// ---------------------------------------------------------------------------

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

/// The whole window over an example, live, with a real DuckDB session behind
/// it and one settled frame drawn.
fn window(name: &str, ctx: &egui::Context) -> MeridianApp {
    let path = example(name);
    let path_str = path.to_str().expect("utf-8 path");
    let (live, composed) = live_spec(path_str).expect("the fixture loads live");
    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    boot.spec_path = Some(path.clone());
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    frame(&mut app, ctx, Vec::new());
    app
}

fn frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 820.0),
        )),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// A pointer position on `plot`, at `fraction` across its width and halfway
/// down it — where an `intervalX` sweep is read.
fn at(app: &MeridianApp, plot: usize, fraction: f32) -> egui::Pos2 {
    let raster = app
        .chart_doc()
        .raster_rect
        .expect("a settled frame presented the raster");
    let rect = app.chart_doc().composed.plots[plot].rect;
    egui::pos2(
        raster.min.x + rect.x as f32 + rect.width as f32 * fraction,
        raster.min.y + rect.y as f32 + rect.height as f32 * 0.5,
    )
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// Press at `from` and drag to `to` WITHOUT releasing — the frames a gesture
/// occupies while it is still a gesture.
fn sweep(app: &mut MeridianApp, ctx: &egui::Context, plot: usize, from: f32, to: f32) {
    let start = at(app, plot, from);
    frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(start), button(start, true)],
    );
    let end = at(app, plot, to);
    frame(app, ctx, vec![egui::Event::PointerMoved(end)]);
}

/// Release at `to`, committing whatever [`sweep`] is holding.
fn release(app: &mut MeridianApp, ctx: &egui::Context, plot: usize, to: f32) {
    let end = at(app, plot, to);
    frame(app, ctx, vec![button(end, false)]);
    frame(app, ctx, Vec::new());
}

/// A real drag, pressed and released on the raster.
fn brush(app: &mut MeridianApp, ctx: &egui::Context, plot: usize, from: f32, to: f32) {
    sweep(app, ctx, plot, from, to);
    release(app, ctx, plot, to);
}

/// A press and release on the same pixel — a click, which an interval binding
/// reads as "retract this plot's contribution".
fn click(app: &mut MeridianApp, ctx: &egui::Context, plot: usize, fraction: f32) {
    let pos = at(app, plot, fraction);
    frame(
        app,
        ctx,
        vec![egui::Event::PointerMoved(pos), button(pos, true)],
    );
    frame(app, ctx, vec![button(pos, false)]);
    frame(app, ctx, Vec::new());
}

/// The x pixel where the second plot starts — the divider a band drawn on the
/// first plot must stay left of.
fn second_plot_start(app: &MeridianApp) -> f64 {
    app.chart_doc().composed.plots[1].rect.x
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// **The whole claim, in one pass over one document.**
///
/// Written as a single test rather than five because each stage is the
/// previous stage's document: the resting picture, the picture mid-sweep, the
/// picture the release produced, the picture a second sweep produced, and the
/// picture left after a click retracts it. Splitting them would either re-load
/// the fixture five times or assert about five different documents.
#[test]
fn a_committed_brush_is_drawn_on_the_plot_that_produced_it_and_goes_when_it_is_cleared() {
    let ctx = egui::Context::default();
    let renderer = VelloRenderer::new();
    let mut app = window("crossfilter.yaml", &ctx);

    // The floor. Nothing is held, so nothing is drawn — and if this ever reads
    // non-zero the measurement is wrong and every assertion below is worthless.
    let resting = rules(&raster(&renderer, app.chart_doc()));
    assert!(
        resting.is_empty(),
        "a dashboard nobody has brushed draws no selection band (found {resting:?})"
    );

    // A gesture still in progress. The pointer is down and the sweep is live,
    // and the chart ink has not moved: the rectangle a reader sees here is an
    // egui quad over the raster, and this IS the raster.
    sweep(&mut app, &ctx, 0, 0.25, 0.6);
    assert!(
        !app.chart_doc().selection_active(),
        "fixture check: a sweep that has not been released has committed nothing, \
         so the picture below is genuinely the in-progress one"
    );
    let mid_sweep = rules(&raster(&renderer, app.chart_doc()));
    assert!(
        mid_sweep.is_empty(),
        "a gesture still in progress leaves the chart ink alone — it is drawn on the \
         overlay, not in the scene (found {mid_sweep:?})"
    );

    // Release. The selection is real, and now it is drawn.
    release(&mut app, &ctx, 0, 0.6);
    assert!(
        app.chart_doc().selection_active(),
        "fixture check: the released sweep committed a selection"
    );
    let first = rules(&raster(&renderer, app.chart_doc()));
    assert_eq!(
        first.len(),
        2,
        "a committed x interval is drawn as its two bounds (found {first:?})"
    );
    let divider = second_plot_start(&app);
    assert!(
        first.iter().all(|&x| x < divider),
        "the band is on the plot that produced the gesture, and the plot being \
         filtered carries none of it (bounds {first:?}, second plot starts at {divider})"
    );

    // The band is the clause. An adjoining sweep starting where the first one
    // ended must draw its low bound where the first drew its high bound —
    // ink that ignored the bounds could not do that.
    brush(&mut app, &ctx, 0, 0.6, 0.85);
    let second = rules(&raster(&renderer, app.chart_doc()));
    assert_eq!(
        second.len(),
        2,
        "the second committed interval is drawn as its two bounds (found {second:?})"
    );
    assert!(
        (second[0] - first[1]).abs() <= 2.0,
        "the two sweeps share a bound, so the band's edges land on the same pixel: \
         first {first:?}, second {second:?}"
    );
    assert!(
        second[1] > first[1],
        "the second sweep reaches further right, and so does its band: \
         first {first:?}, second {second:?}"
    );

    // Clearing. A click with no sweep retracts an interval contribution.
    click(&mut app, &ctx, 0, 0.5);
    assert!(
        !app.chart_doc().selection_active(),
        "fixture check: the click retracted the contribution"
    );
    let cleared = rules(&raster(&renderer, app.chart_doc()));
    assert!(
        cleared.is_empty(),
        "clearing the selection removes its ink (found {cleared:?})"
    );
}

/// **The band's ink is not the in-progress gesture's ink.**
///
/// The two treatments are told apart at a glance by hue, and the test above
/// measures the band in exactly one of them. This pins the separation the
/// measurement depends on: the design system's overlay group is the neutral
/// wash and border the transient rectangle is painted with, and the band is
/// the chart's own focus ink. Should a token bump ever bring the two within
/// `BOUND_TOL` of each other, the picture stops distinguishing them and this
/// says so — before the test above starts counting one as the other.
#[test]
fn the_bands_ink_is_not_the_in_progress_gestures_ink() {
    let want = bound_ink();
    for (name, token) in [
        ("brush_fill", meridian_design::chrome::OVERLAY_LIGHT.brush_fill),
        (
            "brush_border",
            meridian_design::chrome::OVERLAY_LIGHT.brush_border,
        ),
    ] {
        let rgba = [
            (token.r * 255.0).round() as u8,
            (token.g * 255.0).round() as u8,
            (token.b * 255.0).round() as u8,
            255,
        ];
        assert!(
            !is_bound(rgba, want),
            "the transient gesture's {name} is within the band's measurement \
             tolerance of the band's own ink — the two treatments no longer \
             read as different ink"
        );
    }
}
