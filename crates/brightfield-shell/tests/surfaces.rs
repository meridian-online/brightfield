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
//! either path. No pointer is ever moved, so the chart's hover crosshair overlay
//! is never armed; the only scripted input in this file is one keypress, and it
//! is a keypress with no cursor-position component. The capture path already runs
//! a warm-up frame (font atlas + layout settle), then the scripted frames, then a
//! final settle frame which is the one captured — that is what makes a
//! first-frame layout pass invisible to the baseline. Measured on the reference
//! machine, five consecutive runs of all five tests produced byte-identical PNGs.
//!
//! Scale is **1.0** here, against the sheet tier's 2.0. The chart window is
//! 934×360 logical points and the protocol window 1642×1250, so at 2.0 these
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

/// Capture the **chart shell** — `draw_shell` over `ShellState`: the top header
/// band, the right controls panel, and the composited Vello dashboard in the
/// central panel.
fn shell_surface(mode: Mode, name: &str) {
    let spec = fixture("examples/dashboard.yaml");
    let composed = compose_spec(spec.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("compose {}: {e}", spec.display()));
    let out = scratch(name);
    let (w, h) = capture_png(composed, mode, SCALE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    egui_kittest::image_snapshot(&read_rgba(&out), name);
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
/// Light only, deliberately. The sheet is pure egui chrome — the same
/// hardcoded-`INK_LIGHT` problem the dark canvas baseline already documents
/// would be the only thing a dark twin showed, and it would cost another
/// full-window image for that. Covered here: the surface exists and renders.
/// Not covered: the sheet's dark ink. Say so rather than imply otherwise.
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

/// Pins the protocol panel in dark mode **including what is wrong with it**.
///
/// The panel hardcodes `meridian_design::chrome::INK_LIGHT` throughout and never
/// reads `INK_DARK`, so this baseline shows light-mode ink on a dark page:
/// low-contrast and, in places, unreadable. That is the surface as it ships
/// today, and pinning it is the point — the increment that threads the mode
/// through will land as a deliberate, reviewable baseline diff instead of
/// disappearing into an untested surface. Do not "fix" this baseline by
/// regenerating it; fix the panel, and let the regeneration be the evidence.
#[test]
fn protocol_dark_surface() {
    protocol_surface(Mode::Dark, "protocol_dark", Vec::new());
}
