//! Pixel coverage for the two **live** shell surfaces, as they render today.
//!
//! The sibling `snapshot.rs` tier renders a hand-built chrome sheet — it pins
//! the design→egui bridge (fonts, visuals, widget ink) and nothing else. It
//! never touches [`brightfield_shell::window::MeridianApp`], so before this file
//! a rewrite of either surface could change every pixel a user sees with the
//! whole suite staying green. These tests close that hole: they drive the *real*
//! surfaces and diff the *real* window.
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
//! run the crate's own headless capture path (`capture::capture_png`) — the
//! same code `brightfield-shot` runs and the same code the live window runs —
//! and hand the resulting image to
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
//! component. The tests here that do move a pointer have no baseline at all:
//! each compares its captures against one another and calls no snapshot, so no
//! pointer position reaches a committed file.
//! The capture path already runs a warm-up frame (font atlas + layout settle),
//! then the scripted frames, then a final settle frame which is the one captured
//! — that is what makes a first-frame layout pass invisible to the baseline.
//!
//! Scale is **1.0** here, against the sheet tier's 2.0. The chart window is
//! 947×396 logical points and the protocol window 1978×1248; at 1.0 the five
//! committed baselines cost 0.87 MB of repo, and the same five captured at 2.0
//! cost 2.4 MB. Both figures are measured, the second by capturing them rather
//! than by scaling the first — PNG compresses the extra pixels far better than
//! the 4× area implies, and every previous version of this sentence carried a
//! projected number that was wrong. The perceptual gate is a per-pixel delta,
//! not a per-image one, so the lower raster does not buy any slack — it just
//! stores less of it.
//! (Measured sensitivity at this scale: deleting one character from a side-panel
//! label reddens the shell baselines by 15 pixels.)
//!
//! Regenerate with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell
//! --test surfaces`. Thresholds come from `kittest.toml` at the workspace root;
//! read the policy comment there before reaching for a per-test override. None
//! of the committed baselines needs one.
//!
//! Seven baselines now, not five: the two added for the CTE fold are the same
//! protocol window with the fold key pressed, and they carry the same
//! did-the-keystroke-land guard the steps-sheet capture does. The 0.87 MB
//! figure above is the measured cost of the original five and has not been
//! re-measured; see [`cte_surface`] for why the two canvas baselines could not
//! simply be re-photographed.

use std::path::PathBuf;

use brightfield_protocol::layout::Flow;
use brightfield_shell::capture::capture_png;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec_in_mode;
use brightfield_shell::protocol::load_protocol_offline;
use brightfield_shell::window::{chart_window_size, Boot, MeridianApp};
use brightfield_workbench::ViewKind;

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

/// Capture the window **booted on the charts view**: the merged top bar, and
/// the dock's two panes (the composited Vello dashboard, and the controls
/// rail), each in the header band `PaneChrome` draws from its `Subject`.
fn shell_capture(mode: Mode, name: &str, script: Vec<Vec<egui::Event>>) -> image::RgbaImage {
    // Pin the developer-diagnostics flag off so a dev shell that has
    // `BRIGHTFIELD_DEVTOOLS` set cannot bake the controls readout or the
    // top-bar renderer string into a regenerated golden. Hermetic capture
    // owns the same class of process env the offline gate does.
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let spec = fixture("examples/dashboard.yaml");
    // Composed IN the mode being photographed. `Boot::charts` is the one-shot
    // boot — it carries no session — so `ChartDoc::set_mode` has no live
    // dashboard here to re-present through, and a light composition
    // photographed under a dark window is exactly the white slab this baseline
    // pair exists to hold.
    let composed = compose_spec_in_mode(spec.to_str().expect("utf-8 fixture path"), mode)
        .unwrap_or_else(|e| panic!("compose {}: {e}", spec.display()));
    let out = scratch(name);
    let (w, h) = capture_png(Boot::charts(composed), mode, SCALE, &out, script)
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

/// A rectangle of a capture, in pixels. Half-open: `x0..x1`, `y0..y1`.
#[derive(Clone, Copy, Debug)]
struct Region {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Region {
    /// The whole of an egui rect, shrunk by `margin` logical points on every
    /// side and rounded inwards — a region strictly *inside* something the
    /// layout gave us, at [`SCALE`] 1.0 where a logical point is a pixel.
    fn inside(rect: egui::Rect, margin: f32) -> Self {
        let r = rect.shrink(margin);
        Self {
            x0: r.min.x.ceil() as u32,
            y0: r.min.y.ceil() as u32,
            x1: r.max.x.floor() as u32,
            y1: r.max.y.floor() as u32,
        }
    }
}

/// How two captures differ over one [`Region`] — a summary a human can read.
///
/// This comparison used to be an `assert_eq!` over two `Vec<u8>` of raw pixels,
/// so any failure printed 614,400 decimal bytes twice: a 5.7 MB panic message
/// whose entire content was "something in here moved". This says how much moved
/// and where it starts, which is the part a reader can act on.
#[derive(PartialEq, Eq)]
struct RegionDiff {
    /// Pixels whose RGBA differs, out of `total`.
    differing: u32,
    total: u32,
    /// The first differing pixel in raster order, in capture coordinates.
    first: (u32, u32),
}

impl std::fmt::Debug for RegionDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{} pixels differ, first at ({}, {})",
            self.differing, self.total, self.first.0, self.first.1
        )
    }
}

/// How `a` and `b` differ over `r`, or `None` if they are identical there.
fn region_diff(a: &image::RgbaImage, b: &image::RgbaImage, r: Region) -> Option<RegionDiff> {
    for img in [a, b] {
        assert!(
            r.x1 <= img.width() && r.y1 <= img.height(),
            "region {r:?} is outside a {}×{} capture",
            img.width(),
            img.height()
        );
    }
    let mut differing = 0;
    let mut first = None;
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            if a.get_pixel(x, y) != b.get_pixel(x, y) {
                differing += 1;
                first.get_or_insert((x, y));
            }
        }
    }
    first.map(|first| RegionDiff {
        differing,
        total: (r.x1 - r.x0) * (r.y1 - r.y0),
        first,
    })
}

/// The rects the chart shell's layout produces at the window size it asks for:
/// the chart pane's content box, and the overlay checkbox in the controls rail.
///
/// A real layout pass over a **headless** document — no GPU, no capture — run
/// for the same two frames `capture_png` runs before the one it photographs, so
/// the rects are the settled ones. This is how the pixel test below aims: the
/// checkbox coordinate was `(800.0, 125.0)`, pinned against a layout nothing
/// derived it from. It landed, but it would have gone on being green while
/// clicking empty rail the first time the rail's share or a row height moved.
fn chart_layout(mode: Mode) -> (egui::Rect, egui::Rect) {
    let app = settled_chart_app(mode);
    (
        app.chart_viewport().expect("the chart pane drew"),
        app.overlay_checkbox()
            .expect("the controls rail drew its overlay checkbox"),
    )
}

/// A chart-view window run through a real layout pass over a **headless**
/// document — no GPU, no capture — for the same two frames `capture_png` runs
/// before the one it photographs, so every rect read back off it is settled.
fn settled_chart_app(mode: Mode) -> MeridianApp {
    let spec = fixture("examples/dashboard.yaml");
    let composed = compose_spec_in_mode(spec.to_str().expect("utf-8 fixture path"), mode)
        .unwrap_or_else(|e| panic!("compose {}: {e}", spec.display()));
    let (w, h) = chart_window_size(&composed);
    let mut app = MeridianApp::headless(Boot::charts(composed), mode);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w, h),
        )),
        ..Default::default()
    };
    for _ in 0..2 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    app
}

/// Where each plot of the composed dashboard landed **in window coordinates**:
/// the placed rect the composition gave it, which is raster-local, offset by
/// where the chart pane put the raster.
///
/// Derived from the same headless pass [`chart_layout`] runs rather than typed
/// in, for the reason recorded there — and a second time over here, because a
/// per-plot rectangle is exactly the kind of coordinate that goes on being
/// green while pointing at empty raster once a margin moves.
fn chart_plot_rects(mode: Mode) -> Vec<egui::Rect> {
    let app = settled_chart_app(mode);
    let doc = app.chart_doc();
    let raster = doc
        .raster_rect
        .expect("the chart pane reserved its raster rect");
    doc.composed
        .plots
        .iter()
        .map(|plot| {
            egui::Rect::from_min_size(
                raster.min + egui::vec2(plot.rect.x as f32, plot.rect.y as f32),
                egui::vec2(plot.rect.width as f32, plot.rect.height as f32),
            )
        })
        .collect()
}

/// A boot on the protocol view over the checked-in fixture, in the default
/// vertical flow, with the boot cursor wherever the nav puts it.
fn protocol_boot() -> Boot {
    protocol_boot_focused(None)
}

/// The same boot with the cursor placed on `focus` — the state
/// `brightfield-shot --focus <dotted-id>` produces, and the state a click on
/// that node produces in the live window.
fn protocol_boot_focused(focus: Option<&str>) -> Boot {
    let spec = fixture("examples/protocol/edgar_gleif/arcform.yaml");
    let inputs = load_protocol_offline(spec.to_str().expect("utf-8 fixture path"))
        .unwrap_or_else(|e| panic!("load {}: {e}", spec.display()));
    Boot::protocol(inputs, Flow::Vertical, focus.map(str::to_string))
}

/// Capture the window **booted on the protocol view**: the outline rail, the
/// DAG canvas, the inspector rail, the steps sheet tab, and the merged top bar
/// and hint bar around them, in the default vertical flow with the nav's boot
/// cursor selected.
fn protocol_capture(mode: Mode, name: &str, script: Vec<Vec<egui::Event>>) -> image::RgbaImage {
    capture_boot(protocol_boot(), mode, name, script)
}

/// The capture itself, over a boot the caller chose.
fn capture_boot(
    boot: Boot,
    mode: Mode,
    name: &str,
    script: Vec<Vec<egui::Event>>,
) -> image::RgbaImage {
    // Hermetic capture: keep `BRIGHTFIELD_DEVTOOLS` from leaking the top-bar
    // renderer string into this golden (see `shell_capture`).
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let out = scratch(name);
    let (w, h) = capture_png(boot, mode, SCALE, &out, script)
        .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    read_rgba(&out)
}

/// The same, diffed against the committed baseline.
///
/// For a capture with a **scripted keystroke**, do not reach for this: capture
/// with [`protocol_capture`], assert the keystroke landed, and call
/// `image_snapshot` last. Under `UPDATE_SNAPSHOTS=1` this writes whatever it
/// was handed before any guard has run.
fn protocol_surface(mode: Mode, name: &str, script: Vec<Vec<egui::Event>>) {
    let img = protocol_capture(mode, name, script);
    egui_kittest::image_snapshot(&img, name);
}

/// The rects the protocol view's layout produces at the window size it asks
/// for: the DAG canvas pane's content box, and the top bar's switcher control
/// for each view.
///
/// The headless twin of [`chart_layout`], derived for the same reason: the
/// round-trip test below has to click the *switcher*, and a coordinate typed
/// against a bar nothing derived it from would go on clicking empty bar the
/// first time a label or a padding moved.
fn protocol_layout(mode: Mode) -> (egui::Rect, egui::Rect, egui::Rect) {
    let boot = protocol_boot();
    let (w, h) = boot.window_size(ViewKind::Protocol);
    let mut app = MeridianApp::headless(boot, mode);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w, h),
        )),
        ..Default::default()
    };
    for _ in 0..2 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    (
        app.canvas_viewport().expect("the DAG canvas pane drew"),
        app.switcher_rect(ViewKind::Charts)
            .expect("the top bar drew a charts switcher control"),
        app.switcher_rect(ViewKind::Protocol)
            .expect("the top bar drew a protocol switcher control"),
    )
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
///
/// The guard runs **before** the snapshot, for the reason spelled out on
/// [`cte_surface`]: under `UPDATE_SNAPSHOTS=1` a snapshot call writes whatever
/// it is handed, so a guard behind it would author a golden of the wrong tab
/// and only complain about it afterwards. This test had them the other way
/// round until the CTE captures arrived; the committed bytes are unchanged by
/// the reorder.
#[test]
fn protocol_steps_light_surface() {
    let steps = protocol_capture(
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
    egui_kittest::image_snapshot(&steps, "protocol_steps_light");
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

/// The node `build_sec_entities` produces — the crosswalk's one relation with
/// CTEs behind it, and therefore the only cursor position on this fixture from
/// which the CTE fold does anything.
const CTE_FOCUS: &str = "asset.edgar_gleif.sec_entities";

/// The **CTE fold open**: the joins inside a SQL step drawn as lineage of their
/// own, rather than folded into the one rectangle that step is by default.
///
/// # Why these are new captures and not a re-photograph of the two above
///
/// The two committed canvas baselines are taken with an empty script and a
/// fold-closed default, so they show the built graph — and a correct
/// implementation leaves them **byte-identical**. Demanding a diff on them
/// would have been a demand that this feature break its own default. So the
/// opened state gets baselines of its own, and the two existing ones are
/// evidence that nothing leaked into the boot canvas.
///
/// # The guard, and why it runs before the snapshot
///
/// A capture of a keystroke that did nothing would be perfectly stable and
/// perfectly green forever — the failure the steps-sheet baseline above had on
/// its first cut. The same guard applies here, against a reference capture of
/// the **same boot** with **no script**: same window, same size, same focused
/// node, differing only by what `za` did. If the chord ever stops dispatching,
/// the two are identical and this fails.
///
/// It is asserted *before* `image_snapshot`. Under `UPDATE_SNAPSHOTS=1` a
/// snapshot call writes whatever it was handed, so a guard that runs after it
/// would author the wrong golden and only complain afterwards. Guard first,
/// then commit the pixels. (The steps-sheet test above had them the other way
/// round until this one arrived; it is now ordered the same way.)
///
/// # What these two baselines constrain: the LAYOUT, not the graph
///
/// **A pixel here is evidence about `layout.positions`, not about
/// `displayed_graph()`.** `render_asset_graph_with_status` walks the layout's
/// positions and looks each id up in the graph, so a graph node with no
/// position is silently skipped — and the window is *sized* from
/// `ProtocolModel::boot_layout`, which always lays out the collapsed graph.
/// A change that puts nodes into the displayed graph without re-laying-out
/// moves nothing on this image, and these two goldens would stay green
/// photographing the picture they were authored against.
///
/// Nothing shipped can reach that today — every fold, drill and flow change
/// calls `recompute_layout` — so this is a note for the next author, not a hole
/// in the current cover. It is live for the per-step explode (one CTE fold per
/// `sql:` step rather than one over the canvas): flipping an id in a set reads
/// like a change to what is drawn, and it is not one until the layout is
/// recomputed. If a new fold ever appears with these two baselines still green,
/// suspect the layout before believing the pixels.
fn cte_surface(mode: Mode, name: &str) {
    let opened = capture_boot(
        protocol_boot_focused(Some(CTE_FOCUS)),
        mode,
        name,
        vec![press(egui::Key::Z, false), press(egui::Key::A, false)],
    );
    let folded = capture_boot(
        protocol_boot_focused(Some(CTE_FOCUS)),
        mode,
        &format!("{name}_folded_reference"),
        Vec::new(),
    );
    assert_ne!(
        opened.as_raw(),
        folded.as_raw(),
        "{name} is pixel-identical to the same window with no keystroke — \
         the fold chord did not dispatch, so this baseline photographs the \
         folded canvas and would pass forever"
    );
    egui_kittest::image_snapshot(&opened, name);
}

#[test]
fn protocol_cte_light_surface() {
    cte_surface(Mode::Light, "protocol_cte_light");
}

#[test]
fn protocol_cte_dark_surface() {
    cte_surface(Mode::Dark, "protocol_cte_dark");
}

/// The DAG is still there after the user has been to the other view and back.
///
/// One window means one canvas host per document and two documents alive at
/// once, and the rule that keeps them alive is a single line: `MeridianApp` has
/// to name **every pane of every view** to the hosts every frame, not just the
/// panes the drawn view laid out. `EguiCanvasHost::end_frame` frees any slot
/// that neither presented this frame nor appears in the set it is handed, and
/// `present` returns early on an unchanged key — so naming only the drawn
/// view's panes frees the other view's texture while leaving the pane holding
/// its id, and the DAG comes back as an empty box the instant the user
/// switches to it. Nothing about that is visible without a device: the layout,
/// the rects, the scroll offsets and every accesskit node are identical either
/// way. It is a pixel question, so it is asked in pixels.
///
/// Two captures of the same window, compared over a rectangle strictly inside
/// the DAG canvas: one that never leaves the protocol view, one that clicks
/// through to the charts view and back. The pointer ends on the top bar in
/// both readings that matter, so nothing inside the canvas is hovered.
///
/// Watched redden, one mutation: sweeping both documents with the *drawn*
/// view's pane set (`self.charts.doc.sweep(&live); self.protocol.doc.sweep(&live)`)
/// leaves every other test in the workspace green and fails here with the whole
/// canvas differing.
#[test]
fn the_dag_survives_a_round_trip_through_the_charts_view() {
    let (canvas, to_charts, to_protocol) = protocol_layout(Mode::Light);
    // Strictly inside the raster, clear of the pane frame and the tab strip.
    let inside_dag = Region::inside(canvas, 20.0);

    let stayed = protocol_capture(Mode::Light, "roundtrip_stayed", Vec::new());
    let returned = protocol_capture(
        Mode::Light,
        "roundtrip_returned",
        vec![
            click_at(to_charts.center().x, to_charts.center().y),
            click_at(to_protocol.center().x, to_protocol.center().y),
        ],
    );

    // Guard the guard: if either click missed, the second capture never left
    // the protocol view and the assertion below would pass for the wrong
    // reason. The charts view is a different window entirely — a chart pane and
    // a controls rail where the outline rail and the DAG are — so a capture
    // taken with only the first click is bound to differ over this rectangle,
    // and does not if the click missed the switcher.
    let switched = protocol_capture(
        Mode::Light,
        "roundtrip_switched",
        vec![click_at(to_charts.center().x, to_charts.center().y)],
    );
    assert!(
        region_diff(&stayed, &switched, inside_dag).is_some(),
        "clicking the charts control at {:?} changed nothing inside the DAG \
         canvas — the click did not land on the switcher, so this test proves \
         nothing about the round trip",
        to_charts.center(),
    );

    assert_eq!(
        region_diff(&stayed, &returned, inside_dag),
        None,
        "the DAG canvas differs after a round trip through the charts view \
         (and the trip did happen — see above), so the protocol document's \
         texture did not survive the frames its view was not drawn"
    );
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
///
/// Both rectangles and the click are derived from a headless layout pass
/// ([`chart_layout`]) rather than typed in, and the click is *verified to have
/// landed* before the chart region is read: a miss and a broken seam produce the
/// same failure otherwise, and the message would have to guess between them.
#[test]
fn the_overlay_toggle_still_reaches_the_chart_pane() {
    let (chart, checkbox) = chart_layout(Mode::Light);
    // Strictly inside the Vello raster, clear of the pane's frame, its header
    // band and the rail beside it.
    let inside_chart = Region::inside(chart, 20.0);
    // The checkbox and its label, and nothing else in the rail.
    let on_checkbox = Region::inside(checkbox, 1.0);
    // The middle of the chart, so both crosshair lines cross `inside_chart`.
    let over_chart = chart.center();

    let idle = shell_capture(Mode::Light, "overlay_idle", Vec::new());
    let hovered = shell_capture(
        Mode::Light,
        "overlay_on",
        vec![move_to(over_chart.x, over_chart.y)],
    );
    let toggled_off = shell_capture(
        Mode::Light,
        "overlay_off",
        vec![
            click_at(checkbox.center().x, checkbox.center().y),
            move_to(over_chart.x, over_chart.y),
        ],
    );

    // The click landed on the checkbox and changed it. Asserted first, and over
    // the checkbox alone, so that the two assertions below mean exactly one
    // thing each: a missed click cannot masquerade as a broken overlay seam.
    assert!(
        region_diff(&idle, &toggled_off, on_checkbox).is_some(),
        "the checkbox at {:?} looks identical before and after being clicked at \
         {:?} — the click did not land on it, so this test proves nothing about \
         the overlay seam",
        checkbox,
        checkbox.center(),
    );

    assert!(
        region_diff(&idle, &hovered, inside_chart).is_some(),
        "hovering the chart drew nothing — the overlay seam no longer reaches the pane"
    );
    assert_eq!(
        region_diff(&idle, &toggled_off, inside_chart),
        None,
        "the crosshair drew inside the chart with the overlay checkbox off (and \
         the click did land — see above), so the flag the rail writes is not the \
         flag the chart pane reads"
    );
}

/// One pointer, one crosshair, on the plot the pointer is actually over.
///
/// A dashboard is **one** chart document rendered to **one** raster, and
/// `examples/dashboard.yaml` puts two plots on it side by side. Ink spanning
/// the raster therefore crosses a plot the pointer is nowhere near, and the
/// reading is a crosshair on a plot that has no pointer in it.
///
/// Asked in pixels because that is where the claim lives: the segments are
/// painted through the overlay painter onto the presented texture's rect, so
/// nothing about which plot they cross is visible to an accesskit query or to a
/// layout assertion. `crosshair_segments` in `chart_item.rs` holds the geometry
/// as arithmetic; this holds it as photographs of the shipped surface.
///
/// Four captures of the chart window, compared over rectangles strictly inside
/// each plot's placed rect ([`chart_plot_rects`], derived from a headless
/// layout pass rather than typed in):
///
/// - pointer nowhere — the idle reference,
/// - pointer at the centre of the left plot,
/// - pointer at the centre of the right plot,
/// - pointer at the centre of the left plot, then gone from the window.
///
/// For each hover: the hovered plot must differ from idle (the crosshair drew,
/// so the reading below is not vacuous) and the *other* plot must be byte-
/// identical to idle. Both directions, because a fix that clipped to
/// `plots[0]` rather than to the plot under the pointer passes one of them.
/// Then the departure: with the pointer gone, both plots return to idle.
///
/// Light only, for the reason [`the_overlay_toggle_still_reaches_the_chart_pane`]
/// gives: the geometry is mode-independent and a dark twin would cost four more
/// GPU captures to re-photograph the same arithmetic.
#[test]
fn the_crosshair_is_bounded_by_the_plot_under_the_pointer() {
    let plots = chart_plot_rects(Mode::Light);
    assert_eq!(
        plots.len(),
        2,
        "this test reads one plot against another, so the fixture has to place \
         two — examples/dashboard.yaml is an hconcat of a scatter and a bar chart"
    );
    // Clear of each plot's own boundary, so a segment stopping exactly on the
    // shared edge cannot redden the neighbour through one antialiased pixel,
    // while the rest of the neighbour — including the row the raster-wide line
    // would have crossed — stays inside the region.
    let inside: Vec<Region> = plots.iter().map(|r| Region::inside(*r, 2.0)).collect();

    let idle = shell_capture(Mode::Light, "crosshair_idle", Vec::new());

    for (hovered, other) in [(0usize, 1usize), (1, 0)] {
        let at = plots[hovered].center();
        let over = shell_capture(
            Mode::Light,
            &format!("crosshair_over_plot_{hovered}"),
            vec![move_to(at.x, at.y)],
        );
        assert!(
            region_diff(&idle, &over, inside[hovered]).is_some(),
            "hovering the centre of plot {hovered} at {at:?} drew nothing inside \
             it — the pointer missed the raster or the crosshair is gone, so the \
             neighbour reading below would prove nothing"
        );
        assert_eq!(
            region_diff(&idle, &over, inside[other]),
            None,
            "hovering plot {hovered} changed pixels inside plot {other} (and the \
             hover did land — see above), so the crosshair is drawn across the \
             raster rather than across the plot the pointer is in"
        );
    }

    // The departure. The overlay is painted immediate-mode into the frame, so
    // this cannot regress without a retained layer appearing — which is exactly
    // the mechanism to be told about if one ever does.
    let at = plots[0].center();
    let left = shell_capture(
        Mode::Light,
        "crosshair_pointer_gone",
        vec![move_to(at.x, at.y), vec![egui::Event::PointerGone]],
    );
    for (i, region) in inside.iter().enumerate() {
        assert_eq!(
            region_diff(&idle, &left, *region),
            None,
            "plot {i} still differs from idle after the pointer left the window, \
             so ink from a frame the pointer was in survived it"
        );
    }
}
