//! **The ring round the hero's legend while colour's list is open: committed
//! baselines, in both themes, with the legend under the plot, and with it at the
//! plot's right in each of the grid's two layouts, and pixel checks that the
//! ring runs down each side of the band, stands 4 points off it, and keeps out
//! of the plot's data area.** With the list closed, or with no legend, the
//! pictures are checked for the ring's absence and not photographed.
//!
//! The legend at the right is the shelf's default, and on a generated dashboard
//! it stands in a band the hero gives up out of its own box, so it is beside the
//! hero in the hero's pane, whether the grid beside it is drawn as rows or as
//! columns of tiles. The pictures at the right hold that: the band lies inside
//! the hero's pane, and the ring round it does too.
//!
//! The fixture is the housing sample with `median_income` kept on colour, so its
//! page has a single coloured plot and these tests do not tell the hero's own
//! legend from the first one the page draws. The shelf band is drawn (a capture
//! draws none) and the keys `e` then `c` open colour's list.
//!
//! Regenerate with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell
//! --test legend_ring_baseline`.

use std::path::PathBuf;

use brightfield_shell::app::{GridLayout, CHART};
use brightfield_shell::capture::capture_png_staged;
use brightfield_shell::design::Mode;
use brightfield_shell::legend::RING_GAP;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::Boot;
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
    /// The legend at the plot's right, colour's list open on its settings, with
    /// the grid drawn in this layout.
    Right(GridLayout),
    /// The legend under the plot, colour's list open on its settings.
    Below,
    /// The legend under the plot, colour's list opened and then closed again.
    BelowClosed,
    /// No legend, colour's list open.
    None,
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
    /// The band under the plot.
    below: Option<egui::Rect>,
    /// The band at the hero's right.
    right: Option<egui::Rect>,
    /// The box the hero's pane draws the page in.
    hero_pane: Option<egui::Rect>,
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
    // colour's settings, at the right until it is set, and `l` steps it from right
    // to below, and again to none.
    let mut script = vec![Vec::new(), Vec::new()];
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
        Placement::Right(_) => 0,
        Placement::Below | Placement::BelowClosed => 1,
        Placement::None => 2,
    };
    {
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
    if placement == Placement::BelowClosed {
        // Esc leaves the list for the band and the band for the pane.
        for _ in 0..2 {
            script.push(vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            script.push(Vec::new());
        }
        script.extend((0..3).map(|_| Vec::new()));
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
            if let Placement::Right(layout) = placement {
                app.set_grid_layout(layout);
            }
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
            seen.right = doc.legend_rect;
            seen.hero_pane = doc.pane_views.map(|views| views.first);
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
/// bounding rect, or nothing when no pixel is. "Near" is a per-channel
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

/// Where the ring is looked for under the plot: the band grown past the ring's
/// gap, below the data area.
fn under_region(band: egui::Rect, data: egui::Rect, image: &image::RgbaImage) -> egui::Rect {
    band.expand(RING_GAP + 3.0)
        .intersect(egui::Rect::from_min_max(
            egui::pos2(0.0, data.bottom()),
            egui::pos2(image.width() as f32, image.height() as f32),
        ))
}

fn baseline(name: &str, mode: Mode) {
    let (image, seen) = capture(name, mode, Placement::Below);
    assert!(seen.open, "{name}: colour's list is not open");
    let data = seen.data.expect("the hero has a data area");
    let band = seen.below.expect("a band under the plot");
    // Colour's hue stands in the plot's data area in its marks, which the pixel
    // rule cannot tell apart, so the ring is looked for under the plot.
    let ring = hue_pixels(&image, under_region(band, data, &image), mode)
        .expect("a ring stands under the plot");
    // The band is the last thing in its pane, so the ring is cut back at the
    // pane's edge on its left, right and foot and stands the gap off it on top.
    assert!(
        (ring.top() - (band.top() - RING_GAP)).abs() <= 1.5,
        "{name}: ring {ring:?}, band {band:?}"
    );
    assert!(
        ring.left() <= band.left() + 1.5
            && ring.right() >= band.right() - 2.5
            && ring.bottom() >= band.bottom() - 2.5,
        "{name}: ring {ring:?} does not run round the band {band:?}"
    );
    // Each side is drawn down its length, not only met by the top edge: a strip
    // along each side, clear of the top and foot, holds colour's hue.
    let side = |x: f32| {
        egui::Rect::from_min_max(
            egui::pos2(x - 2.0, band.top() + 4.0),
            egui::pos2(x + 2.0, band.bottom() - 4.0),
        )
    };
    for (edge, x) in [("left", ring.left() + 0.5), ("right", ring.right() - 0.5)] {
        let found = hue_pixels(&image, side(x), mode)
            .unwrap_or_else(|| panic!("{name}: no ring down the band's {edge} side"));
        assert!(
            found.height() >= band.height() - 10.0,
            "{name}: the {edge} side runs {found:?}, short of the band {band:?}"
        );
    }
    egui_kittest::image_snapshot(&image, name);
}

/// **The ring round the legend under the plot — light.**
#[test]
fn the_ring_below_light_baseline() {
    baseline("legend_ring_below_light", Mode::Light);
}

/// **The same in dark.**
#[test]
fn the_ring_below_dark_baseline() {
    baseline("legend_ring_below_dark", Mode::Dark);
}

/// The right-hand baseline: the band beside the hero, inside the hero's pane,
/// and the ring round the legend in it, clear of the data area and of the pane
/// beside it.
fn right_baseline(name: &str, mode: Mode, layout: GridLayout) {
    let (image, seen) = capture(name, mode, Placement::Right(layout));
    assert!(seen.open, "{name}: colour's list is not open");
    let data = seen.data.expect("the hero has a data area");
    let band = seen.right.expect("a band at the hero's right");
    let pane = seen.hero_pane.expect("the canvas drew the hero's pane");
    assert!(
        band.left() >= data.right(),
        "{name}: the band {band:?} is not at the right of the data area {data:?}"
    );
    assert!(
        pane.expand(0.5).contains_rect(band),
        "{name}: the band {band:?} leaves the hero's pane {pane:?}"
    );
    // The ring is looked for round the band, past the data area: the block in
    // the band is the legend, and the ring stands the gap off it.
    let region = band
        .expand(RING_GAP + 3.0)
        .intersect(egui::Rect::from_min_max(
            egui::pos2(data.right(), 0.0),
            egui::pos2(image.width() as f32, image.height() as f32),
        ));
    let ring = hue_pixels(&image, region, mode).expect("a ring stands round the legend");
    assert!(
        ring.left() >= data.right() - 0.5,
        "{name}: the ring {ring:?} reaches into the data area {data:?}"
    );
    assert!(
        pane.expand(0.5).contains_rect(ring),
        "{name}: the ring {ring:?} leaves the hero's pane {pane:?}"
    );
    assert!(
        ring.height() > 2.0 * RING_GAP && ring.width() > 2.0 * RING_GAP,
        "{name}: the ring {ring:?} is not round a block"
    );
    egui_kittest::image_snapshot(&image, name);
}

/// **The ring round the legend at the hero's right, the grid on its rows —
/// light.**
#[test]
fn the_ring_at_the_right_on_rows_light_baseline() {
    right_baseline(
        "legend_ring_right_rows_light",
        Mode::Light,
        GridLayout::Rows,
    );
}

/// **The same in dark.**
#[test]
fn the_ring_at_the_right_on_rows_dark_baseline() {
    right_baseline("legend_ring_right_rows_dark", Mode::Dark, GridLayout::Rows);
}

/// **The ring round the legend at the hero's right, the grid drawn as columns of
/// tiles — light.** The layout the legend used to be drawn over a tile in.
#[test]
fn the_ring_at_the_right_on_columns_light_baseline() {
    right_baseline(
        "legend_ring_right_columns_light",
        Mode::Light,
        GridLayout::Columns,
    );
}

/// **The same in dark.**
#[test]
fn the_ring_at_the_right_on_columns_dark_baseline() {
    right_baseline(
        "legend_ring_right_columns_dark",
        Mode::Dark,
        GridLayout::Columns,
    );
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

/// **With colour's list closed there is no ring**, though the legend is where it
/// was: no pixel of colour's hue stands round the band under the plot.
#[test]
fn with_the_list_closed_no_ring_is_drawn() {
    let (image, seen) = capture(
        "legend_ring_closed_light",
        Mode::Light,
        Placement::BelowClosed,
    );
    assert!(!seen.open, "colour's list is still open");
    let data = seen.data.expect("the hero has a data area");
    let band = seen.below.expect("the legend is still under the plot");
    assert_eq!(
        hue_pixels(&image, under_region(band, data, &image), Mode::Light),
        None
    );
}
