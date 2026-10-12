//! **The ring round the hero's legend while colour's list is open: committed
//! baselines, in both themes, with the legend at the plot's right and with it
//! below, and a pixel check that the ring is one point wide, stands 4 points off
//! the block, and keeps out of the plot's data area.** The legend is gone in a
//! third picture, which is checked for the ring's absence and not photographed.
//!
//! The chart is the one `legend_below_baseline.rs` photographs, coloured by the
//! `reading` column. The shelf band is drawn (a capture draws none) and the keys
//! `e` then `c` open colour's list, so the picture is the window as it reads
//! with that list open.
//!
//! Regenerate with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell
//! --test legend_ring_baseline`.

use std::path::PathBuf;

use brightfield_shell::app::{GridLayout, CHART};
use brightfield_shell::capture::capture_png_staged;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::{self, RING_GAP};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::channel::{hue, ShelfChannel};
use brightfield_workbench::{chrome, PaneKey};

/// Device pixels per logical point.
const SCALE: f32 = 1.0;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/california_housing_sample.csv")
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Placement {
    Right,
    Below,
    None,
}

/// Where the grid pane's layout switch draws its columns state, read off a
/// window of the same size that has run three frames, as `dashboard_baseline.rs`
/// reads it. The picture with the legend at the right is taken in that layout:
/// in the rows layout the hero's pane shows the page cut at the hero's own width,
/// and the legend at the page's right edge lies beyond it.
fn layout_switch(size: (f32, f32), mode: Mode) -> egui::Pos2 {
    let boot = Boot::data_file(fixture().to_str().expect("utf-8 path")).expect("the sample opens");
    let mut app = MeridianApp::headless(boot, mode);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(size.0, size.1),
        )),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    app.chart_doc()
        .grid_layout_switch
        .clone()
        .expect("the grid pane's header band drew a layout switch")
        .states
        .iter()
        .find(|(state, _)| *state == GridLayout::Columns)
        .expect("the switch offers a columns state")
        .1
        .center()
}

/// The frames that click `at`.
fn throw_the_switch(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    vec![
        vec![egui::Event::PointerMoved(at)],
        vec![egui::Event::PointerMoved(at), button(true)],
        vec![egui::Event::PointerMoved(at), button(false)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ]
}

fn enter() -> egui::Event {
    key(egui::Key::Enter, "").remove(0)
}

fn tab() -> egui::Event {
    key(egui::Key::Tab, "").remove(0)
}

fn key(key: egui::Key, text: &str) -> Vec<egui::Event> {
    vec![
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::Text(text.to_owned()),
    ]
}

/// Where the legend sits when the script has run, in window space.
#[derive(Default)]
struct Seen {
    data: Option<egui::Rect>,
    /// The legend's band: at the right, the strip reserved beside the raster,
    /// with the block's top where the plot's is; below, the band under the plot.
    block_left_top: Option<egui::Pos2>,
    below: Option<egui::Rect>,
    open: bool,
}

/// Capture the window with colour's list open and the legend at `placement`, and
/// give back the picture with what the window said about where things stood.
fn capture(name: &str, mode: Mode, placement: Placement) -> (image::RgbaImage, Seen) {
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let boot = Boot::data_file(fixture().to_str().expect("utf-8 path")).expect("the sample opens");
    let size = boot.window_size();
    let out = scratch(name);
    let mut seen = Seen::default();
    // `e` hands the keys to the shelf band, `c` opens colour's cell: its list.
    // One `j` puts the list's cursor on `median_income`, the first column, and
    // Enter keeps it on colour and closes the list. The legend row is the third of
    // colour's settings, and `l` steps it from right to below, and again to none.
    let mut script = vec![Vec::new(), Vec::new()];
    if placement == Placement::Right {
        script.extend(throw_the_switch(layout_switch(size, mode)));
    }
    script.extend([
        key(egui::Key::E, "e"),
        Vec::new(),
        key(egui::Key::C, "c"),
        Vec::new(),
        key(egui::Key::J, "j"),
        Vec::new(),
        vec![enter()],
        Vec::new(),
        // Enter closes the list; `e` and `c` open it again, now over a coloured chart.
        key(egui::Key::E, "e"),
        Vec::new(),
        key(egui::Key::C, "c"),
        Vec::new(),
    ]);
    // Frames for the chart to be drawn again over the colour it was given.
    script.extend((0..6).map(|_| Vec::new()));
    let steps = match placement {
        Placement::Right => 0,
        Placement::Below => 1,
        Placement::None => 2,
    };
    if steps > 0 {
        script.push(vec![tab()]);
        script.push(Vec::new());
        for _ in 0..2 {
            script.push(key(egui::Key::J, "j"));
            script.push(Vec::new());
        }
        for _ in 0..steps {
            script.push(key(egui::Key::L, "l"));
            script.push(Vec::new());
        }
    }
    capture_png_staged(
        boot,
        default_layout(),
        mode,
        SCALE,
        size,
        &out,
        script,
        |app| {
            app.set_shelf_band_drawn(true);
            assert!(app.focus_pane(PaneKey::new(CHART)));
        },
        |app| {
            let doc = app.chart_doc();
            let hero = app.composed_plot_rects()[0];
            let layout = &doc.composed.plots[0].layout;
            #[allow(clippy::cast_possible_truncation)]
            {
                seen.data = Some(egui::Rect::from_min_max(
                    egui::pos2(
                        hero.left() + layout.plot_x_start() as f32,
                        hero.top() + layout.plot_y_start() as f32,
                    ),
                    egui::pos2(
                        hero.left() + layout.plot_x_end() as f32,
                        hero.top() + layout.plot_y_end() as f32,
                    ),
                ));
                seen.block_left_top = doc
                    .legend_rect
                    .map(|band| egui::pos2(band.left(), hero.top()));
                seen.below =
                    doc.raster_rect
                        .zip(doc.composed.plots[0].legend_below)
                        .map(|(raster, r)| {
                            egui::Rect::from_min_size(
                                raster.min + egui::vec2(r.x as f32, r.y as f32),
                                egui::vec2(r.width as f32, r.height as f32),
                            )
                        });
            }
            seen.open = doc.colour_list_open;
        },
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    let image = image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8();
    (image, seen)
}

/// The pixels of `image` within `region` that are near colour's hue, as a
/// bounding rect, or `None` when there are none. "Near" is a per-channel
/// distance of 40 from the hue, which a one-point stroke on a fractional edge
/// still lands inside and the legend's ramp and the plot's marks do not.
fn hue_pixels(image: &image::RgbaImage, region: egui::Rect, mode: Mode) -> Option<egui::Rect> {
    let c = chrome::colour(hue(ShelfChannel::Colour, mode));
    let near = |p: &image::Rgba<u8>| {
        [(p[0], c.r()), (p[1], c.g()), (p[2], c.b())]
            .iter()
            .all(|(a, b)| a.abs_diff(*b) <= 40)
    };
    let mut bounds: Option<egui::Rect> = None;
    for y in region.top().max(0.0) as u32..(region.bottom().max(0.0) as u32).min(image.height()) {
        for x in region.left().max(0.0) as u32..(region.right().max(0.0) as u32).min(image.width())
        {
            if near(image.get_pixel(x, y)) {
                let p = egui::pos2(x as f32, y as f32);
                bounds = Some(bounds.map_or(egui::Rect::from_min_max(p, p), |b| {
                    b.union(egui::Rect::from_min_max(p, p))
                }));
            }
        }
    }
    bounds
}

fn baseline(name: &str, mode: Mode, placement: Placement) {
    let (image, seen) = capture(name, mode, placement);
    assert!(seen.open, "{name}: colour's list is not open");
    let data = seen.data.expect("the hero has a data area");
    let (w, h) = (image.width() as f32, image.height() as f32);
    let everywhere = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(w, h));
    // Colour's hue stands nowhere in the plot's data area but in its marks, which
    // the pixel rule cannot tell apart, so the ring is looked for beside it.
    let ring = match placement {
        Placement::Right => {
            let origin = seen.block_left_top.expect("a band at the right");
            let region = egui::Rect::from_min_max(
                egui::pos2(origin.x - RING_GAP - 3.0, origin.y - RING_GAP - 3.0),
                egui::pos2(origin.x + legend::block_width() + RING_GAP + 3.0, h),
            )
            .intersect(egui::Rect::from_min_max(
                egui::pos2(data.right(), 0.0),
                everywhere.max,
            ));
            hue_pixels(&image, region, mode)
        }
        Placement::Below => {
            let band = seen.below.expect("a band under the plot");
            let region = band
                .expand(RING_GAP + 3.0)
                .intersect(egui::Rect::from_min_max(
                    egui::pos2(0.0, data.bottom()),
                    everywhere.max,
                ));
            hue_pixels(&image, region, mode)
        }
        Placement::None => None,
    };
    match placement {
        Placement::Right => {
            let ring = ring.expect("a ring stands at the right of the plot");
            let origin = seen.block_left_top.expect("a band at the right");
            assert!(
                ring.left() >= data.right(),
                "{name}: {ring:?} reaches into {data:?}"
            );
            // The block's left edge less the gap, to within a pixel, or the data
            // area's right edge where the gap would reach it.
            let want = (origin.x - RING_GAP).max(data.right());
            assert!(
                (ring.left() - want).abs() <= 1.5,
                "{name}: ring left {} want {want}",
                ring.left()
            );
        }
        Placement::Below => {
            let ring = ring.expect("a ring stands under the plot");
            let band = seen.below.expect("a band under the plot");
            assert!(
                ring.top() >= data.bottom(),
                "{name}: {ring:?} reaches into {data:?}"
            );
            assert!(
                (ring.left() - (band.left() - RING_GAP)).abs() <= 1.5,
                "{name}: {ring:?} vs {band:?}"
            );
            assert!(
                (ring.right() - (band.right() + RING_GAP)).abs() <= 1.5,
                "{name}: {ring:?} vs {band:?}"
            );
            assert!(
                (ring.bottom() - (band.bottom() + RING_GAP)).abs() <= 1.5,
                "{name}: {ring:?} vs {band:?}"
            );
        }
        Placement::None => {}
    }
    egui_kittest::image_snapshot(&image, name);
}

/// **The ring round the legend at the plot's right — light.**
#[test]
fn the_ring_at_the_right_light_baseline() {
    baseline("legend_ring_right_light", Mode::Light, Placement::Right);
}

/// **The same in dark.**
#[test]
fn the_ring_at_the_right_dark_baseline() {
    baseline("legend_ring_right_dark", Mode::Dark, Placement::Right);
}

/// **The ring round the legend under the plot — light.**
#[test]
fn the_ring_below_light_baseline() {
    baseline("legend_ring_below_light", Mode::Light, Placement::Below);
}

/// **The same in dark.**
#[test]
fn the_ring_below_dark_baseline() {
    baseline("legend_ring_below_dark", Mode::Dark, Placement::Below);
}

/// **With no legend there is nothing to ring**, though colour's list is open:
/// no pixel of colour's hue stands where a ring would, beside or under the plot.
#[test]
fn with_no_legend_no_ring_is_drawn() {
    let (image, seen) = capture("legend_ring_none_light", Mode::Light, Placement::None);
    assert!(seen.open, "colour's list is not open");
    let data = seen.data.expect("the hero has a data area");
    let (w, h) = (image.width() as f32, image.height() as f32);
    let beside = egui::Rect::from_min_max(
        egui::pos2(data.right(), data.top()),
        egui::pos2(w, data.bottom()),
    );
    let under = egui::Rect::from_min_max(
        egui::pos2(data.left(), data.bottom()),
        egui::pos2(data.right(), h),
    );
    assert_eq!(hue_pixels(&image, beside, Mode::Light), None);
    // Under the plot are the axis' labels and the grid pane's chrome, none of
    // them colour's hue.
    assert_eq!(hue_pixels(&image, under, Mode::Light), None);
}
