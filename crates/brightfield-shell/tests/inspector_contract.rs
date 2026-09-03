//! The inspector against the shell contract, without a GPU.
//!
//! The sibling of `chart_contract.rs`'s state-only style: this file is driven
//! through `MeridianApp::draw` and `MeridianApp::inspector_selection`, neither
//! of which needs a device — `Workspace::focus` and `Subject` are both plain
//! data. The pixel tier (`surfaces.rs`) covers what this cannot: what the
//! panel *looks* like.
//!
//! One `egui::Context` per test, created once and threaded through each
//! frame — the same shape `interval_slider.rs`'s `frame` helper uses, and for
//! the same reason: a fresh `Context::default()` per frame would leave
//! `fonts_installed` pointing at the old context and skip installing fonts on
//! the new one, which is a false economy this file does not need.
//!
//! `the_controls_rail_is_drawn_by_the_inspector_pane` is AC4's own standing
//! witness: `chart_contract.rs`'s assertions about the registry's
//! `ControlsPane` do not move when `window.rs::assemble`'s swap is reverted —
//! they read `chart_registry()` directly, never the live app — so this file
//! carries the one test that does.

use brightfield_shell::app::{ChartDoc, CHART, CONTROLS};
use brightfield_shell::design::Mode;
use brightfield_shell::editor::EDITOR;
use brightfield_shell::inspector::{InspectorPane, Selection, TableHandle};
use brightfield_shell::overlays::CHART_PALETTE_VERBS;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::{Item, PaneKey};

const DASHBOARD: &str = "../../examples/dashboard.yaml";

/// The window rect every frame here runs at.
fn screen() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0))
}

/// Run one frame with no input.
fn frame(app: &mut MeridianApp, ctx: &egui::Context) {
    let raw = egui::RawInput {
        screen_rect: Some(screen()),
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// A booted chart window over the real fixture, its own `egui::Context`, and
/// two settled frames — the same recipe `chart_contract.rs`'s pixel-adjacent
/// tests use, so "settled" means the same thing here it does there.
fn settled() -> (MeridianApp, egui::Context) {
    let composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    let mut app = MeridianApp::headless(Boot::charts(composed), Mode::Light);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    frame(&mut app, &ctx);
    (app, ctx)
}

/// AC4's own witness: the `CONTROLS` slot in the *live* app is drawn by
/// `InspectorPane`, not the registry's dormant `ControlsPane` — the two
/// declare different titles ("Inspector" vs "Controls"), and this is the one
/// test in the suite that asks the running window which one is actually
/// there rather than asking the registry what it declares.
#[test]
fn the_controls_rail_is_drawn_by_the_inspector_pane() {
    let (app, _ctx) = settled();
    let title = app
        .chart_pane_title(PaneKey::new(CONTROLS))
        .expect("the rail is in the default arrangement");
    assert_eq!(
        title, "Inspector",
        "the CONTROLS slot is drawn by ControlsPane, not InspectorPane — the \
         swap in window.rs::assemble did not happen"
    );
}

/// AC2: before the user has clicked on a pane, the inspector reports `None`
/// — not a stale answer from some earlier boot, because there is no earlier
/// boot.
#[test]
fn nothing_is_selected_before_any_pane_has_been_focused() {
    let (app, _ctx) = settled();
    assert_eq!(
        app.inspector_selection(),
        None,
        "a freshly booted window has focused nothing yet"
    );
}

/// AC1, the literal claim: select one pane, then another, and the panel's
/// text is not identical for both.
#[test]
fn selecting_a_different_pane_changes_what_the_inspector_shows() {
    let (mut app, ctx) = settled();

    assert!(
        app.focus_pane(PaneKey::new(CHART)),
        "the chart pane is in the default arrangement"
    );
    // Focus lands in `MeridianApp::apply`, reached at the end of the frame
    // the request was raised in — but `focus_pane` sets it directly, so the
    // very next frame's `inspector_selection` read already sees it.
    frame(&mut app, &ctx);
    let chart = app
        .inspector_selection()
        .expect("the chart pane is focused");
    assert_eq!(chart.title, "Chart");

    assert!(
        app.focus_pane(PaneKey::new(EDITOR)),
        "the editor pane is in the default arrangement"
    );
    frame(&mut app, &ctx);
    let editor = app
        .inspector_selection()
        .expect("the editor pane is focused");

    assert_ne!(
        chart.title, editor.title,
        "selecting a different pane must change what the panel's rendered \
         text says — the observable failure AC1 names"
    );
}

/// AC2's other half: clearing focus reverts the panel to "nothing selected"
/// rather than holding the last pane it named.
#[test]
fn clearing_focus_drops_the_selection_rather_than_holding_it_stale() {
    let (mut app, ctx) = settled();
    app.focus_pane(PaneKey::new(CHART));
    frame(&mut app, &ctx);
    assert!(app.inspector_selection().is_some(), "the chart is selected");

    app.clear_focus();
    frame(&mut app, &ctx);
    assert_eq!(
        app.inspector_selection(),
        None,
        "focus was cleared, so the inspector must not go on naming the chart \
         pane — that would be exactly the stale previous selection AC2 rules \
         out"
    );
}

/// Focusing the inspector's own pane — clicking its checkbox, say — must not
/// blank the selection it is itself displaying: the last real pane focused
/// stays named.
#[test]
fn focusing_the_inspector_itself_leaves_the_last_real_selection_standing() {
    let (mut app, ctx) = settled();
    app.focus_pane(PaneKey::new(EDITOR));
    frame(&mut app, &ctx);
    let editor = app
        .inspector_selection()
        .expect("the editor pane is focused");

    assert!(
        app.focus_pane(PaneKey::new(CONTROLS)),
        "the rail itself is a pane in the arrangement"
    );
    frame(&mut app, &ctx);
    assert_eq!(
        app.inspector_selection().as_ref().map(|s| &s.title),
        Some(&editor.title),
        "clicking inside the inspector's own pane must not blank what it \
         was showing"
    );
}

/// AC2's whole-pane gate, over the pane that actually ships rather than over
/// the registry's dormant `ControlsPane`: no dashboard open means the shell
/// draws the empty state instead of calling `ui`, and a real dashboard means
/// it does not.
#[test]
fn the_inspector_is_empty_only_when_the_document_is() {
    let pane = InspectorPane::new(Selection::default(), TableHandle::default());
    let empty = ChartDoc::empty();
    assert!(
        pane.empty_state(&empty).is_some(),
        "no dashboard is open, so the shipping pane must decline to draw"
    );

    let loaded = ChartDoc::headless(compose_spec(DASHBOARD).expect("compose the fixture"));
    assert!(
        pane.empty_state(&loaded).is_none(),
        "a real dashboard is open, so the shipping pane must draw its body — \
         an inverted predicate here would blank the whole rail, checkbox \
         included"
    );
}

/// Each chart-view pane's toolbar over the shipped fixture, straight from
/// `chart_registry()`'s freshly constructed, never-drawn items. That is fine
/// here specifically: `chart_item.rs`'s toolbar comes from `Self::
/// toolbar_entries(doc)`, a pure function of the document, `data_grid.rs`
/// declares an empty one, and so does the registry's own `ControlsPane` —
/// the one pane this shortcut does not hold for is the editor, which needs
/// [`editor_toolbar_verbs`] below instead.
fn registry_toolbar_verbs(doc: &ChartDoc) -> Vec<&'static str> {
    brightfield_shell::app::chart_registry()
        .specs()
        .iter()
        .flat_map(|spec| (spec.make)().subject(doc).toolbar)
        .map(|entry| entry.verb.as_str())
        .collect()
}

/// The editor's toolbar needs a file actually open — `describe` reads `self.
/// file`, a field `ui`'s first call populates from `doc.spec_path` (see
/// `editor.rs::ui`), untouched afterwards. A fresh, undrawn `EditorPane`
/// declares an empty toolbar, which is the gap that let an earlier version
/// of this sweep pass under the same mutation its doc comment claimed would
/// redden it: walking `chart_registry().specs()` alone leaves `save-spec`
/// out of the set being matched, so removing its arm left the test's
/// outcome unchanged.
fn editor_toolbar_verbs(doc: &mut ChartDoc) -> Vec<&'static str> {
    let mut pane = brightfield_shell::editor::EditorPane::new();
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(480.0, 320.0),
        )),
        ..Default::default()
    };
    let mut requests = Vec::new();
    let _ = ctx.run_ui(raw, |ui| {
        let mut icx = brightfield_workbench::ItemCtx::new(
            Mode::Light,
            PaneKey::new(EDITOR),
            egui_tiles::TileId::from_u64(1),
            true,
            &mut requests,
        );
        pane.ui(doc, ui, &mut icx);
    });
    pane.describe(doc)
        .toolbar
        .into_iter()
        .map(|entry| entry.verb.as_str())
        .collect()
}

/// The defect this lane would otherwise have shipped: a
/// declared-but-undispatchable verb — one `MeridianApp::apply`'s Charts arm
/// has no case for — would look live if this rail drew it unfiltered. This
/// sweep requires a verdict, from [`registry_toolbar_verbs`] and
/// [`editor_toolbar_verbs`] combined: dispatches, so the inspector may draw
/// it, or does not, so it must stay filtered. An unrecognised verb panics
/// rather than passing over unproven, the same shape
/// `overlay_wiring.rs::every_chart_palette_candidate_actually_dispatches`
/// uses for the command palette.
///
/// `save-spec` stays filtered here, and the reason changed. It used to be
/// this file's worked example of a verb `MeridianApp::apply`'s Charts arm had
/// no case for. The arm exists now — it writes the Protocol a data file opened
/// as — but the entry this rail would draw for that verb is `EditorPane`'s,
/// which means the editor's own buffer. One name, two writes: drawn here it is
/// a dead button over a clean buffer and a wrong one over a dirty buffer,
/// because the click saves the Protocol and reports success over an edit that
/// was never written. The palette carries the Protocol save instead —
/// `overlay_wiring.rs`'s sweep picks it off there and reads the written spec
/// back.
///
/// Proof this sweep can fail: dropping the `"save-spec"` arm below (so it
/// falls to the `other => panic!` case) reddens this test with "save-spec is
/// declared on a chart-view pane's toolbar with no verdict in this sweep".
#[test]
fn every_declared_toolbar_verb_either_dispatches_or_is_filtered_out() {
    let mut doc =
        ChartDoc::headless(compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml"));
    doc.spec_path = Some(DASHBOARD.into());

    let mut verbs = registry_toolbar_verbs(&doc);
    verbs.extend(editor_toolbar_verbs(&mut doc));

    for verb in &verbs {
        let dispatches = CHART_PALETTE_VERBS.contains(verb);
        match *verb {
            "clear-selection" | "reset-extent" => assert!(
                dispatches,
                "{verb} is expected to dispatch at the chart altitude — \
                 CHART_PALETTE_VERBS moved and \
                 inspector::dispatchable's verdict for it needs re-checking"
            ),
            "save-spec" => assert!(
                !dispatches,
                "save-spec is drawable in the inspector rail again. The entry \
                 there is the editor's buffer Save and the dispatch is the \
                 Protocol's, so one of the two writes silently does not \
                 happen. The palette is where the Protocol save is offered — \
                 see overlays::chart_offers."
            ),
            other => panic!(
                "{other} is declared on a chart-view pane's toolbar with no \
                 verdict in this sweep — decide whether the inspector may \
                 draw it (it dispatches: add a case here) or must keep \
                 dropping it, and extend this match"
            ),
        }
    }
    assert!(
        verbs.contains(&"save-spec"),
        "the editor's toolbar declared no save-spec entry — the file did \
         not open, which would make this sweep vacuous for it: saw {verbs:?}"
    );
}
