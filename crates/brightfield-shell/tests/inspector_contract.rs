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
//! AC4's revert-and-witness is recorded in the card, not duplicated as a
//! standing test: pinning "the registry still constructs `ControlsPane`" here
//! would just restate `chart_contract.rs`'s existing
//! `each_pane_names_itself_once_and_binds_in_its_own_context`, which already
//! reddens the moment that changes.

use brightfield_shell::app::{ChartDoc, CHART, CONTROLS};
use brightfield_shell::design::Mode;
use brightfield_shell::editor::EDITOR;
use brightfield_shell::inspector::{InspectorPane, Selection};
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::{Item, PaneKey, ViewKind};

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
        app.focus_pane(PaneKey::new(ViewKind::Charts, CHART)),
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
        app.focus_pane(PaneKey::new(ViewKind::Charts, EDITOR)),
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
    app.focus_pane(PaneKey::new(ViewKind::Charts, CHART));
    frame(&mut app, &ctx);
    assert!(app.inspector_selection().is_some(), "the chart is selected");

    app.clear_chart_focus();
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
    app.focus_pane(PaneKey::new(ViewKind::Charts, EDITOR));
    frame(&mut app, &ctx);
    let editor = app
        .inspector_selection()
        .expect("the editor pane is focused");

    assert!(
        app.focus_pane(PaneKey::new(ViewKind::Charts, CONTROLS)),
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

/// AC5: the checkbox draws — with area to click — whether or not a pane is
/// selected, because the whole-pane empty state this pane declares is gated
/// on `doc.is_empty()`, not on selection. Proven at the document level,
/// without a GPU, so the pixel-tier `the_overlay_toggle_still_reaches_the_
/// chart_pane` (in `surfaces.rs`) is proving the seam still reaches the
/// canvas, not proving the checkbox is there to click.
#[test]
fn the_hover_overlay_checkbox_draws_whether_or_not_anything_is_selected() {
    let (app, _ctx) = settled();
    assert_eq!(
        app.inspector_selection(),
        None,
        "nothing was focused, so the panel's own empty state applies to the \
         selection half"
    );
    let checkbox = app
        .overlay_checkbox()
        .expect("the hover-overlay checkbox drew even with nothing selected");
    assert!(
        checkbox.width() > 0.0 && checkbox.height() > 0.0,
        "the checkbox drew with no area to click: {checkbox:?}"
    );
    assert!(app.chart_doc().overlay, "the overlay starts armed");
}

/// AC2's whole-pane gate, over the pane that actually ships rather than over
/// the registry's dormant `ControlsPane`: no dashboard open means the shell
/// draws the empty state instead of calling `ui`, and a real dashboard means
/// it does not.
#[test]
fn the_inspector_is_empty_only_when_the_document_is() {
    let pane = InspectorPane::new(Selection::default());
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
