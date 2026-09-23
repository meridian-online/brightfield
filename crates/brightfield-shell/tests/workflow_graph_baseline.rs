//! **A committed baseline, in both themes, for the workflow's second beat**: a
//! one-step Protocol's asset graph on the canvas, reached through the spine
//! head's graph chip rather than typed as a literal window state.
//!
//! The structural half mirrors `tests/dashboard_baseline.rs`, which explains
//! at length why the model-level guard runs ahead of the photograph: an image
//! diff reddens on a font bump exactly as loudly as on a dropped chip, and a
//! reviewer holding one red baseline cannot tell which of those happened. The
//! guard here reads the model the frame drew from — `canvas_holds`, the step's
//! own run status, the locator band's crumbs and counts — not a galley.
//!
//! # How the click reaches the capture
//!
//! The chip's position is read off a headless layout pass at the capture's own
//! window size, the way `tests/dashboard_baseline.rs`'s `pane_rects` reads the
//! ledger's reopen control: `MeridianApp::headless` lays out identically to
//! the device path and differs in the raster alone, so a rect read there is
//! the rect the capture lands on. The click itself is scripted in the frame
//! shape `tests/dashboard_baseline.rs`'s `reopen_the_ledger` uses — a frame
//! carrying the pointer's move, one carrying the press and the release, then
//! settle frames — handed to `capture_png_at`'s own script parameter.
//!
//! Each photograph is checked against an unscripted capture of the same boot
//! before it is trusted: `tests/surfaces.rs`'s `cte_surface` is the pattern —
//! if the two are pixel-identical the click never dispatched, and a baseline
//! that skipped this check would photograph the dashboard forever and call it
//! the graph.
//!
//! Regenerate with: `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell
//! --test workflow_graph_baseline`.

use brightfield_protocol::contract_graph::SeamStatus;
use brightfield_protocol::graph::AssetKind;
use brightfield_shell::capture::capture_png_at;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::NodeView;
use brightfield_shell::window::{Boot, CanvasHolds, MeridianApp};

/// Device pixels per logical point — `tests/dashboard_baseline.rs`'s scale.
const SCALE: f32 = 1.0;

/// The window this pair is committed at, per the milestone's condition on the
/// first screen's composition.
const WINDOW: (f32, f32) = (1440.0, 900.0);

/// The committed table this file's window is opened over: California
/// Housing, the fixture `tests/navigator_spine.rs`'s own `housing` opens.
fn housing() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv")
}

/// A boot over [`housing`], as the front door's picker and
/// `brightfield-shot --spec table.csv` both build it.
fn housing_boot() -> Boot {
    let path = housing();
    let chosen = path.to_str().expect("utf-8 fixture path");
    Boot::data_file(chosen).unwrap_or_else(|e| panic!("open {}: {e}", path.display()))
}

/// Where the capture's intermediate PNG goes — `tests/dashboard_baseline.rs`'s
/// `scratch`, under the target dir, already git-ignored.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

/// The locator band's trailing counts, spelled independently of
/// `ProtocolModel::graph_counts` — `tests/navigator_spine.rs`'s own
/// `counted_pair`, duplicated here because each integration test file is its
/// own crate and the helper is private to that one.
fn counted_pair(nodes: usize, steps: usize) -> String {
    let noun = |n: usize, word: &str| {
        if n == 1 {
            format!("{n} {word}")
        } else {
            format!("{n} {word}s")
        }
    };
    format!("{} \u{b7} {}", noun(nodes, "node"), noun(steps, "step"))
}

/// A headless window over [`housing_boot`] at [`WINDOW`], laid out through
/// three settle frames on one `egui::Context` kept for the window's whole
/// life — a click resolves against the widget id a previous frame registered.
///
/// `MeridianApp::headless` differs from the device path in the raster alone,
/// so the chip rect read off this window is the chip rect the pixel capture
/// below lands on.
fn probe() -> (MeridianApp, egui::Context) {
    let mut app = MeridianApp::headless(housing_boot(), Mode::Light);
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(WINDOW.0, WINDOW.1),
        )),
        ..Default::default()
    };
    for _ in 0..3 {
        let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
    }
    (app, ctx)
}

/// Where the last frame `app` ran drew the spine's graph chip — panicking by
/// name when the head carries no chip, so a chip dropped off the head fails
/// here with a sentence rather than with `unwrap` on a `None`.
fn chip_pos(app: &MeridianApp) -> egui::Pos2 {
    let rows = app.spine_rows();
    let head = rows.first().unwrap_or_else(|| {
        panic!("the rail drew no rows at all, so it drew no head to carry a chip")
    });
    head.chip
        .unwrap_or_else(|| {
            panic!(
                "the spine's head row {:?} carries no graph chip",
                head.label
            )
        })
        .rect
        .center()
}

/// One frame's worth of a pointer move and a primary click at `pos`.
fn click_at(pos: egui::Pos2) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::PointerMoved(pos)];
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

/// The probe, driven through one click on the graph chip — the window the
/// structural guard below reads, ahead of any photograph.
fn driven_graph_window() -> MeridianApp {
    let (mut app, ctx) = probe();
    let at = chip_pos(&app);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(WINDOW.0, WINDOW.1));
    for events in [click_at(at), Vec::new(), Vec::new()] {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    }
    app
}

/// **AC1's structural half.** The model the frame drew from, read back
/// without a pixel — proving the chip actually put the graph on the canvas,
/// with its chips, its seam's run state, the spine head marked, and both
/// rails closed to their strips, before the photograph is trusted.
fn assert_workflow_graph_structure(app: &MeridianApp) {
    assert_eq!(
        app.canvas_holds(),
        &CanvasHolds::Graph,
        "the chip did not put the graph on the canvas"
    );
    assert!(
        app.graph_on_canvas(),
        "graph_on_canvas() disagrees with canvas_holds() over the same frame"
    );
    assert!(
        app.canvas_panes().panes.is_empty(),
        "the pane group is still drawn over the graph: {:?}",
        app.canvas_panes()
            .panes
            .iter()
            .map(|p| p.name)
            .collect::<Vec<_>>()
    );

    let model = app.protocol_model();
    let nodes = &model.displayed_graph().nodes;
    let kinds: std::collections::BTreeSet<AssetKind> = nodes.values().map(|n| n.kind).collect();
    assert_eq!(
        kinds,
        [AssetKind::File, AssetKind::Table, AssetKind::Dashboard]
            .into_iter()
            .collect(),
        "the housing fixture's one-step Protocol should draw the file, its \
         table and the table's own generated dashboard, and this frame draws \
         {:?}",
        nodes
            .values()
            .map(|n| (n.kind, n.label.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        model.node_count(),
        3,
        "the housing fixture's one-step Protocol should draw three nodes — \
         the file, the table and the table's own generated dashboard — and \
         this one draws {}",
        model.node_count()
    );
    assert_eq!(
        model.step_count(),
        1,
        "the housing fixture declares one step, and this graph counts {}",
        model.step_count()
    );

    let states = model.step_states();
    assert_eq!(
        states.len(),
        1,
        "one seam is declared, so step_states() should carry one entry, and \
         it carries {}",
        states.len()
    );
    for (step, status) in &states {
        assert_eq!(
            *status,
            SeamStatus::NotRun,
            "the housing fixture's step {step:?} has not been run, so its \
             seam status should read not-run and reads {status:?}"
        );
    }

    assert_eq!(
        app.locator_crumbs(),
        vec!["Protocol".to_string()],
        "the locator band's crumb line over the graph should be the one entry Protocol"
    );
    let counts = app
        .locator_counts()
        .expect("the graph holds the canvas, so the band has counts to say");
    assert_eq!(
        counts,
        counted_pair(model.node_count(), model.step_count()),
        "the locator band's trailing counts are not the graph's own node and \
         step counts"
    );

    let rows = app.spine_rows();
    let head = rows
        .first()
        .expect("the spine drew no rows, so it drew no head to check");
    let chip = head
        .chip
        .expect("the spine's head row carries no graph chip");
    assert!(
        chip.filled,
        "the chip should read filled with the graph on the canvas"
    );
    assert!(
        head.on_canvas.is_some(),
        "the spine's head row does not carry the on-canvas bar"
    );
    for view in NodeView::ALL {
        let row = rows
            .iter()
            .find(|r| r.label == view.label())
            .unwrap_or_else(|| panic!("the spine drew no {} row", view.label()));
        assert!(
            row.on_canvas.is_none(),
            "the {} row carries the on-canvas bar while the graph holds the canvas",
            view.label()
        );
    }

    let chips = app.canvas_chips();
    assert_eq!(
        chips.len(),
        1,
        "the table node's one declared view (grid) should draw one chip on \
         the graph, and this frame draws {}",
        chips.len()
    );
    assert_eq!(
        chips[0].view,
        NodeView::Grid,
        "the table node's chip is not the grid view's"
    );

    assert!(
        app.rail_is_collapsed(brightfield_workbench::arrangement::LEDGER_RAIL),
        "a one-step Protocol's ledger rail should open collapsed to its strip"
    );
    assert!(
        app.rail_is_collapsed(brightfield_workbench::arrangement::INSPECTOR_RAIL),
        "a one-step Protocol's inspector rail should open collapsed to its strip"
    );
    assert!(
        app.rail_summary_rect(brightfield_workbench::arrangement::LEDGER_RAIL)
            .is_some(),
        "the collapsed ledger rail drew no trailing summary in its strip"
    );

    let canvas = app
        .region_rect(brightfield_workbench::arrangement::CANVAS)
        .expect("the canvas region drew");
    let pane = app
        .canvas_viewport()
        .expect("the DAG canvas pane drew, so it recorded the box it was given");
    assert!(
        canvas.expand(0.5).contains_rect(pane),
        "the DAG pane drew at {pane:?}, which is not inside the canvas region {canvas:?}"
    );
}

/// The frames that click the graph chip at `at` — `tests/dashboard_baseline.rs`'s
/// `reopen_the_ledger` shape, handed to [`capture_png_at`]'s script.
fn click_the_chip(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    vec![
        vec![egui::Event::PointerMoved(at)],
        vec![button(true), button(false)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ]
}

/// [`housing_boot`] captured at [`WINDOW`] under `script`, read back as
/// pixels — `tests/dashboard_baseline.rs`'s `capture_short`, hermetic of the
/// developer's renderer string the same way.
fn capture(mode: Mode, script: Vec<Vec<egui::Event>>, name: &str) -> image::RgbaImage {
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let out = scratch(name);
    let (w, h) = capture_png_at(housing_boot(), mode, SCALE, WINDOW, &out, script)
        .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    image::open(&out)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", out.display()))
        .to_rgba8()
}

/// The photograph half, shared by both themes: click the chip, prove the
/// click actually moved pixels against an unscripted capture of the same
/// boot, then check the image against its committed baseline.
fn workflow_graph_capture(mode: Mode, name: &str) {
    let (probe_app, _ctx) = probe();
    let at = chip_pos(&probe_app);

    let clicked = capture(mode, click_the_chip(at), name);
    let unclicked = capture(mode, Vec::new(), &format!("{name}_unclicked_reference"));
    assert_ne!(
        clicked.as_raw(),
        unclicked.as_raw(),
        "{name} is pixel-identical to the same window with no click — the \
         chip's click did not dispatch, so this baseline photographs the \
         dashboard rather than the graph and would pass forever"
    );

    egui_kittest::image_snapshot(&clicked, name);
}

/// **AC1.** The graph reached through the spine head's chip, committed in
/// light — the structural guard runs first, on a headless drive of the same
/// click, for the reason this file's header gives.
#[test]
fn the_workflow_graph_light_baseline() {
    assert_workflow_graph_structure(&driven_graph_window());
    workflow_graph_capture(Mode::Light, "workflow_graph_light");
}

/// **AC1's dark twin.** The model facts the light test above checks are
/// mode-independent — read off `canvas_holds`, the step map and the locator
/// band, and ink moves neither — so they are not restated here;
/// `tests/dashboard_baseline.rs`'s header gives the same reason for its own
/// pair.
#[test]
fn the_workflow_graph_dark_baseline() {
    workflow_graph_capture(Mode::Dark, "workflow_graph_dark");
}
