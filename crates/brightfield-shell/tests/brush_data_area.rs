//! **An x-range brush stops above the tick labels.** Dragged through the whole
//! window, the rectangle the painter is handed spans the plot's data area top
//! to bottom and no further, however far below the plot the pointer goes: the
//! margin under the data area, where the tick labels and the x-axis title are,
//! stays clear.
//!
//! GPU-free: `ChartDoc::gesture_ink` records the rectangle the brush paints on
//! each frame a drag is in progress, which is what lets this read it without
//! a device — the standing `tests/canvas_pane_group.rs` gives it. The six
//! brush kinds, against a plot built for the question, are pinned by
//! `no_brush_kind_paints_outside_the_data_area` in `chart_item`; this is the
//! gesture end to end, on a shipped example.

use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::live_spec;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::BrushKind;

use std::path::PathBuf;

/// `examples/crossfilter.yaml`: two dot plots over temperature, each with an
/// `intervalX` brush and derived x and y titles — a temperature range,
/// whatever the power, which is what this gesture exists to select.
fn crossfilter() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/crossfilter.yaml")
}

const SCREEN: egui::Vec2 = egui::vec2(1280.0, 820.0);

/// The whole window over the example, live, with one settled frame drawn —
/// the harness `tests/committed_selection_ink.rs` uses.
fn window(ctx: &egui::Context) -> MeridianApp {
    let path = crossfilter();
    let (live, composed) =
        live_spec(path.to_str().expect("utf-8 path")).expect("the example loads live");
    let mut boot = Boot::charts(composed);
    boot.live = Some(live);
    boot.spec_path = Some(path);
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    // Three frames, the settle count `tests/canvas_pane_group.rs` uses for a
    // resizable panel's reported size to be read back: the first frames
    // reflow the plots into the pane.
    for _ in 0..3 {
        frame(&mut app, ctx, Vec::new());
    }
    app
}

fn frame(app: &mut MeridianApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)),
        events,
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// The first plot's allocation and data area in window coordinates, off the
/// composition the window is presenting now.
fn first_plot(app: &MeridianApp) -> (egui::Rect, egui::Rect) {
    let doc = app.chart_doc();
    let raster = doc
        .raster_rect
        .expect("a settled frame presented the raster");
    let handle = &doc.composed.plots[0];
    assert_eq!(
        handle.gesture.as_ref().map(|g| g.kind),
        Some(BrushKind::IntervalX),
        "the example's first plot carries an x-range brush"
    );
    let (r, l) = (handle.rect, handle.layout);
    #[allow(clippy::cast_possible_truncation)]
    let at = |x: f64, y: f64| egui::pos2(raster.min.x + x as f32, raster.min.y + y as f32);
    (
        egui::Rect::from_min_max(at(r.x, r.y), at(r.x + r.width, r.y + r.height)),
        egui::Rect::from_min_max(
            at(r.x + l.plot_x_start(), r.y + l.plot_y_start()),
            at(r.x + l.plot_x_end(), r.y + l.plot_y_end()),
        ),
    )
}

#[test]
fn an_x_range_brush_stops_above_the_tick_labels() {
    let ctx = egui::Context::default();
    let mut app = window(&ctx);

    let (plot, data) = first_plot(&app);
    assert!(
        plot.bottom() - data.bottom() > 1.0 && plot.top() < data.top(),
        "the plot {plot:?} has no margin around its data area {data:?}, so there is nothing \
         for the brush to stay out of and this test measures nothing"
    );

    // Press in the data area; drag down past the plot's bottom edge, through
    // the tick labels and the x title.
    let press = egui::pos2(data.left() + data.width() * 0.3, data.center().y);
    let to = egui::pos2(data.left() + data.width() * 0.6, plot.bottom() + 40.0);
    assert!(to.y < SCREEN.y, "the drag stays on screen");
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::PointerMoved(press),
            egui::Event::PointerButton {
                pos: press,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        ],
    );
    let midway = egui::pos2((press.x + to.x) / 2.0, (press.y + to.y) / 2.0);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(midway)]);
    frame(&mut app, &ctx, vec![egui::Event::PointerMoved(to)]);

    // Read with the button still down: the ink is an uncommitted sweep's, and
    // the data area it is measured against is the one on the frame it was
    // painted on.
    let ink = app
        .chart_doc()
        .gesture_ink
        .expect("the drag recorded its ink");
    let (plot, data) = first_plot(&app);
    assert!(
        ink.width() > 1.0,
        "the drag recorded {ink:?}, which does not follow the pointer across"
    );
    let near = |a: f32, b: f32| (a - b).abs() <= 0.5;
    assert!(
        near(ink.top(), data.top()) && near(ink.bottom(), data.bottom()),
        "the x-range brush {ink:?} spans the data area {data:?} top to bottom, no more and no \
         less — the plot's allocation is {plot:?}"
    );
    assert!(
        ink.left() >= data.left() - 0.5 && ink.right() <= data.right() + 0.5,
        "the brush {ink:?} stays across the data area {data:?}"
    );
}
