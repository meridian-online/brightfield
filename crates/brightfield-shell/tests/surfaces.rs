//! Pixel coverage for the two **live** shell surfaces, as they render today.
//!
//! The sibling `snapshot.rs` tier renders a hand-built chrome sheet — it pins
//! the design→egui bridge (fonts, visuals, widget ink) and nothing else. It
//! never touches [`brightfield_shell::app::draw_shell`], `ShellState`, or
//! [`brightfield_shell::protocol::ProtocolShell`], so before this file a rewrite
//! of either surface could change every pixel a user sees with the whole suite
//! staying green. These tests close that hole: they drive the *real* surfaces
//! and diff the *real* window.
//!
//! # Why the pixel tier, for both
//!
//! Both surfaces present their content through the Vello canvas seam
//! (`render_to_texture` → `register_native_texture`), which reaches egui as an
//! opaque native texture id. AccessKit sees a rectangle with no children: the
//! chart's marks and axes, and the DAG's nodes, edges and selection highlight,
//! carry **zero** structural signal. A kittest accesskit assertion could pin the
//! chrome around them (headings, slider, breadcrumb text) more cheaply and less
//! brittly, and that is worth adding for the chrome-only questions — but it can
//! never answer "did the chart change", which is the question a canvas-host
//! rewrite makes urgent. So the content needs pixels, and once a pixel baseline
//! of the whole window exists it covers the chrome for free.
//!
//! # Why not `Harness::builder().wgpu()`
//!
//! kittest's wgpu harness owns its device and its `egui_wgpu::Renderer`, and
//! exposes neither. The canvas host must register its Vello texture *into the
//! same renderer* that draws the frame, so a kittest-rendered harness would show
//! the two surfaces with a blank hole where their content is — a baseline that
//! pins everything except the thing most likely to break. Instead these tests
//! run the crate's own headless capture path (`capture::capture_png` /
//! `capture_protocol_png`) — the same code `brightfield-shot` runs and the same
//! code the live window's `draw_shell` runs — and hand the resulting image to
//! egui_kittest's standalone [`egui_kittest::image_snapshot`], so the comparison,
//! the `kittest.toml` thresholds and the `UPDATE_SNAPSHOTS=1` workflow are
//! identical to the sheet tier's.
//!
//! # Determinism
//!
//! Every input is fixed: checked-in spec fixtures, a fixed logical window size
//! (each surface's own `window_size()`, which is a pure function of the fixture),
//! a fixed `pixels_per_point`, and no wall-clock or randomness anywhere on
//! either path. **No baseline capture moves a pointer**, so the chart's hover
//! crosshair overlay is never armed in one; the only scripted input a baseline
//! takes is one keypress, and it is a keypress with no cursor-position
//! component. The one test here that does move a pointer —
//! [`the_overlay_toggle_still_reaches_the_chart_pane`] — has no baseline at all:
//! it compares three captures against each other, so nothing about it is stored.
//! The capture path already runs a warm-up frame (font atlas + layout settle),
//! then the scripted frames, then a final settle frame which is the one captured
//! — that is what makes a first-frame layout pass invisible to the baseline.
//!
//! Scale is **1.0** here, against the sheet tier's 2.0. The chart window is
//! 940×380 logical points and the protocol window 1642×1250, so at 2.0 these
//! five baselines would cost roughly 3 MB of repo; at 1.0 they cost 0.8 MB for
//! the same coverage. The perceptual gate is a per-pixel delta, not a per-image
//! one, so the lower raster does not buy any slack — it just stores less of it.
//! (Measured sensitivity at this scale: deleting one character from a side-panel
//! label reddens the shell baselines by 15 pixels.)
//!
//! Regenerate with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell
//! --test surfaces`. Thresholds come from `kittest.toml` at the workspace root;
//! read the policy comment there before reaching for a per-test override. None
//! of these five needs one.

use std::path::PathBuf;

use brightfield_protocol::layout::Flow;
use brightfield_shell::capture::{capture_png, capture_protocol_png};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::load_protocol_offline;

/// Device pixels per logical point for this tier. See the module note.
const SCALE: f32 = 1.0;

/// A checked-in fixture, addressed from the crate root so the test does not
/// depend on the shell's working directory.
fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Where a capture's intermediate PNG goes. Per-test name, under the target dir
/// (already git-ignored), so concurrent tests never race on one path.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

/// Read a capture back as RGBA8. PNG is lossless, so this round-trip is
/// pixel-exact — the file is only how `capture_*` hands its result over.
fn read_rgba(path: &PathBuf) -> image::RgbaImage {
    image::open(path)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", path.display()))
        .to_rgba8()
}

/// Capture the **chart shell** — `draw_shell` over `ShellState`: the top bar,
/// and the dock's two panes (the composited Vello dashboard, and the controls
/// rail), each in the header band `PaneChrome` draws from its `Subject`.
fn shell_capture(mode: Mode, name: &str, script: Vec<Vec<egui::Event>>) -> image::RgbaImage {
    let spec = fixture("examples/dashboard.yaml");
    let composed = compose_spec(spec.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("compose {}: {e}", spec.display()));
    let out = scratch(name);
    let (w, h) = capture_png(composed, mode, SCALE, &out, script)
        .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    read_rgba(&out)
}

/// The same, diffed against the committed baseline.
fn shell_surface(mode: Mode, name: &str) {
    egui_kittest::image_snapshot(&shell_capture(mode, name, Vec::new()), name);
}

/// One frame that moves the pointer to a logical position.
fn move_to(x: f32, y: f32) -> Vec<egui::Event> {
    vec![egui::Event::PointerMoved(egui::pos2(x, y))]
}

/// One frame that moves the pointer and clicks the primary button there — the
/// same event triple `capture::parse_script` synthesises for a `{"click":[x,y]}`
/// line.
fn click_at(x: f32, y: f32) -> Vec<egui::Event> {
    let pos = egui::pos2(x, y);
    let mut events = move_to(x, y);
    for pressed in [true, false] {
        events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
    events
}

/// A rectangle of an image's pixels, as bytes — for comparing one region of two
/// captures while ignoring everything outside it.
fn region(img: &image::RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<u8> {
    assert!(
        x1 <= img.width() && y1 <= img.height(),
        "region {x1}×{y1} is outside a {}×{} capture",
        img.width(),
        img.height()
    );
    let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            out.extend_from_slice(&img.get_pixel(x, y).0);
        }
    }
    out
}

/// Capture the **protocol shell** — `ProtocolShell::draw`: the outline rail, the
/// DAG canvas, the inspector rail, the steps sheet tab and the breadcrumb/hint
/// bars, in the default vertical flow with the nav's boot cursor selected.
fn protocol_surface(mode: Mode, name: &str, script: Vec<Vec<egui::Event>>) -> image::RgbaImage {
    let spec = fixture("examples/protocol/edgar_gleif/arcform.yaml");
    let inputs = load_protocol_offline(spec.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("load {}: {e}", spec.display()));
    let out = scratch(name);
    let (w, h) = capture_protocol_png(inputs, mode, Flow::Vertical, None, SCALE, &out, script)
        .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    let img = read_rgba(&out);
    egui_kittest::image_snapshot(&img, name);
    img
}

/// One frame of a keypress, the same event pair `capture::parse_script`
/// synthesises for a `{"key":"…","shift":…}` script line.
fn press(key: egui::Key, shift: bool) -> Vec<egui::Event> {
    let modifiers = egui::Modifiers {
        shift,
        ..Default::default()
    };
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        })
        .collect()
}

#[test]
fn shell_light_surface() {
    shell_surface(Mode::Light, "shell_light");
}

#[test]
fn shell_dark_surface() {
    shell_surface(Mode::Dark, "shell_dark");
}

#[test]
fn protocol_light_surface() {
    protocol_surface(Mode::Light, "protocol_light", Vec::new());
}

/// The **steps sheet**, which the two canvas-tab baselines above cannot see: the
/// dock boots on the Canvas tab, so the sheet is an unrendered sibling until the
/// steps verb activates it. One scripted keypress, then the settle frame, then
/// capture.
///
/// The verb is **shift-S**, not a bare `s`: `key_token` maps `Key::S if
/// mods.shift` and nothing else, so a bare press falls through and this test
/// would silently re-photograph the Canvas tab — which is exactly what it did on
/// the first cut. The hint bar reads `S steps`, which is why that is easy to get
/// wrong; the assertion below is what catches it.
///
/// Light only, deliberately. The sheet is pure egui chrome, so its ink comes
/// from the same `semantic(dark)` resolution `chrome_dark` and the dark canvas
/// tab already cover; a dark twin would cost another full-window image to
/// re-photograph that. Covered here: the surface exists and renders. Not
/// covered: this particular sheet in dark. Say so rather than imply otherwise.
#[test]
fn protocol_steps_light_surface() {
    let steps = protocol_surface(
        Mode::Light,
        "protocol_steps_light",
        vec![press(egui::Key::S, true)],
    );
    // Guard the guard: a baseline of the wrong tab would still be perfectly
    // stable and perfectly green forever. The canvas-tab baseline is the same
    // window at the same size, so if the keypress did nothing these two files
    // are identical — assert they are not.
    let canvas = image::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/protocol_light.png"),
    )
    .expect("canvas-tab baseline is committed next to this one")
    .to_rgba8();
    assert_ne!(
        steps.as_raw(),
        canvas.as_raw(),
        "the steps capture is pixel-identical to the canvas capture — \
         the steps verb did not dispatch, so this baseline photographs the wrong tab"
    );
}

/// Pins the protocol panel in dark mode, chrome **and** DAG raster.
///
/// This baseline used to pin the panel's dark mode *including what was wrong
/// with it*: `asset_scene` hardcoded `meridian_design::chrome::INK_LIGHT`, so
/// the DAG rendered as a white sheet inside a dark window. Both halves now
/// resolve through `semantic(dark)` — the chrome through
/// `brightfield_workbench::chrome`, the raster through
/// `asset_scene::AssetInk` — and this photograph is the evidence the earlier
/// note asked for.
///
/// What it does **not** cover: the chart side of `brightfield-render` (axis,
/// grid, legend, marks, `scene.rs`) still names light tokens. None of it is on
/// this surface — the protocol panel draws no charts — so a green run here says
/// nothing about it either way.
#[test]
fn protocol_dark_surface() {
    protocol_surface(Mode::Dark, "protocol_dark", Vec::new());
}

/// The overlay toggle still reaches the canvas across the dock.
///
/// The chart pane and the controls rail are two `egui_tiles` panes now, and the
/// flag one writes and the other reads lives on the document between them. That
/// is exactly the kind of wiring a re-expression can drop in silence: the
/// checkbox would still tick, the crosshair would simply never appear again, and
/// no baseline in this file would notice — none of the five moves a pointer.
///
/// Three captures, compared over a rectangle **strictly inside the chart raster**
/// so the controls rail's own change of state is out of frame:
///
/// - pointer nowhere (the baseline capture's input),
/// - pointer over the chart, overlay armed as it boots,
/// - the "hover overlay" checkbox clicked off, *then* the pointer over the chart.
///
/// The first two must differ — that is the crosshair being drawn. The first and
/// third must be identical — that is the toggle actually suppressing it. Light
/// only: the plumbing is mode-independent and a dark twin would cost three more
/// GPU captures to re-photograph the same wire.
#[test]
fn the_overlay_toggle_still_reaches_the_chart_pane() {
    // Inside the Vello raster and clear of the pane's frame, its header band and
    // the rail beside it.
    const X0: u32 = 60;
    const Y0: u32 = 100;
    const X1: u32 = 700;
    const Y1: u32 = 340;
    // Over the chart, so both crosshair lines cross the region above.
    const OVER_CHART: (f32, f32) = (400.0, 220.0);
    // The "hover overlay" checkbox in the controls rail.
    const CHECKBOX: (f32, f32) = (800.0, 125.0);

    let idle = shell_capture(Mode::Light, "overlay_idle", Vec::new());
    let hovered = shell_capture(
        Mode::Light,
        "overlay_on",
        vec![move_to(OVER_CHART.0, OVER_CHART.1)],
    );
    let toggled_off = shell_capture(
        Mode::Light,
        "overlay_off",
        vec![
            click_at(CHECKBOX.0, CHECKBOX.1),
            move_to(OVER_CHART.0, OVER_CHART.1),
        ],
    );

    assert_ne!(
        region(&idle, X0, Y0, X1, Y1),
        region(&hovered, X0, Y0, X1, Y1),
        "hovering the chart drew nothing — the overlay seam no longer reaches the pane"
    );
    assert_eq!(
        region(&idle, X0, Y0, X1, Y1),
        region(&toggled_off, X0, Y0, X1, Y1),
        "the crosshair drew with the overlay checkbox off — either the click \
         missed the checkbox, or the flag the rail writes is not the flag the \
         chart pane reads"
    );
}
