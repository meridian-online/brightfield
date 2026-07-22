//! The chart view against the shell contract, without a GPU.
//!
//! The sibling of `protocol_contract.rs`, and deliberately its shape:
//! everything asserted here is a property of the view's *declaration* — its
//! registry, its two subjects, its default arrangement, the window it asks for —
//! and none of it needs a device. That is the payoff of a
//! [`Subject`](brightfield_workbench::Subject) being plain data: the pane
//! headers, the empty states and the key context of a whole surface can be
//! pinned in a unit test that runs in milliseconds, where before the only thing
//! on this surface that could see them was a full-window pixel baseline on a
//! GPU.
//!
//! The pixel tier (`surfaces.rs`) still covers what this cannot: what the shell
//! *looks* like.

use std::collections::BTreeMap;

use brightfield_shell::app::{chart_registry, ChartDoc, CHART, CONTROLS};
use brightfield_shell::design::Mode;
use brightfield_shell::editor::EDITOR;
use brightfield_shell::pipeline::{compose_spec, Composed};
use brightfield_shell::window::{
    chart_toolbar_band, chart_window_size, Boot, MeridianApp, BAR_HEIGHT, DOCK_INSET,
};
use brightfield_workbench::behavior::TILE_GAP;
use brightfield_workbench::registry::{DockSide, Slot};
use brightfield_workbench::subject::{RunState, ToolbarLocation};
use brightfield_workbench::{audit, chrome, ItemId, PaneKey, Subject, ViewKind};

const DASHBOARD: &str = "../../examples/dashboard.yaml";
const CROSSFILTER: &str = "../../examples/crossfilter.yaml";

/// The real fixture, as a document with no device behind it.
///
/// Carries the path it was composed from, exactly as [`Boot::open`] records
/// it on a live boot — the spec editor reads it, so a fixture that dropped
/// the path would test a document no boot produces.
///
/// [`Boot::open`]: brightfield_shell::window::Boot::open
fn loaded() -> ChartDoc {
    let mut doc =
        ChartDoc::headless(compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml"));
    doc.spec_path = Some(DASHBOARD.into());
    doc
}

/// Every pane's subject over one document, keyed by item id.
fn subjects(doc: &ChartDoc) -> BTreeMap<ItemId, Subject> {
    chart_registry()
        .specs()
        .iter()
        .map(|spec| (spec.id, (spec.make)().subject(doc)))
        .collect()
}

/// The controls rail's declared share, read out of the registry.
fn controls_share() -> f32 {
    let registry = chart_registry();
    let spec = registry
        .specs()
        .iter()
        .find(|s| s.id == CONTROLS)
        .expect("the controls rail is in the registry");
    match spec.slot {
        Slot::Rail { side, share } => {
            assert_eq!(side, DockSide::Right, "the controls rail docks right");
            share
        }
        other => panic!("the controls rail is not a rail: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// The workbench audit, over the chart view.
///
/// This is the one assertion that replaces "somebody remembered to write an
/// empty state" — and on this surface nobody had: before this increment a spec
/// that composed nothing rendered a header, a side panel and a blank rectangle.
/// It constructs both panes, asks each for its subject over an empty document,
/// and rejects a missing empty state, prose that breaks the house style, a verb
/// the keyboard registry does not have, and a rail that names no verb to show
/// and hide it.
#[test]
fn every_chart_pane_passes_the_contract_audit() {
    audit(&chart_registry(), &ChartDoc::empty()).expect("the chart view is on the contract");
}

/// An empty document really is empty, so the audit above is asserting on the
/// branch it thinks it is.
///
/// Without this, `Composed::empty` could quietly grow area and every
/// empty-state assertion in the file would pass by never reaching one.
#[test]
fn the_empty_document_has_nothing_in_it() {
    let composed = Composed::empty();
    assert_eq!(composed.width, 0, "an empty dashboard has width");
    assert_eq!(composed.height, 0, "an empty dashboard has height");
    assert!(composed.title.is_none(), "an empty dashboard has a title");

    let doc = ChartDoc::empty();
    assert!(doc.is_empty(), "an empty document reports content");
    for (id, subject) in subjects(&doc) {
        assert!(
            subject.empty_state.is_some(),
            "{id} shows content over an empty document"
        );
    }
}

/// The mirror of the audit, and the half that actually catches an inverted
/// predicate: over the **real** fixture, no pane is empty.
///
/// An `empty_state` that is always `Some` passes the audit perfectly and blanks
/// the whole window — the shell draws the empty state *instead of* the pane's
/// own body, so `doc.is_empty()` written without the `!` on the other branch
/// would ship two panes with two apologies in them and no chart at all.
#[test]
fn no_pane_is_empty_over_a_real_dashboard() {
    let doc = loaded();
    assert!(
        !doc.is_empty(),
        "the fixture composed nothing, so this test proves nothing"
    );
    for (id, subject) in subjects(&doc) {
        assert!(
            subject.empty_state.is_none(),
            "{id} claims to be empty over examples/dashboard.yaml: {:?}",
            subject.empty_state
        );
    }
}

// ---------------------------------------------------------------------------
// What the panes declare
// ---------------------------------------------------------------------------

/// Each pane names itself once, and resolves its keys in its own context:
/// the chart grammar's for the chart and its controls, the editor's for the
/// editor — a text buffer that resolved `cmd-c` through chart bindings
/// would copy the wrong thing.
///
/// The title is the whole of the pane's name now. Before this increment the
/// controls rail's own body opened with a bold `Controls` label and the chart
/// had no name at all; the only heading on the surface was the window's.
/// The editor's fresh-pane title is `Editor`; the file names the tab from
/// the first drawn frame, and `editor_item.rs` holds that half.
#[test]
fn each_pane_names_itself_once_and_binds_in_its_own_context() {
    let names: BTreeMap<ItemId, String> = subjects(&loaded())
        .into_iter()
        .map(|(id, s)| {
            let expected = if id == EDITOR {
                brightfield_keys::BindingContext::Editor
            } else {
                brightfield_keys::BindingContext::Workspace
            };
            assert_eq!(
                s.key_context, expected,
                "{id} resolves keys somewhere other than its own context"
            );
            (id, s.title)
        })
        .collect();
    assert_eq!(names[&CHART], "Chart");
    assert_eq!(names[&CONTROLS], "Controls");
    assert_eq!(names[&EDITOR], "Editor");
}

/// The chart pane declares its toolbar — and over a dashboard with no
/// brushable plot, declares it **withheld**, so the collapsing `Toolbar`
/// draws no row at all. Quiet-when-nothing-to-show as a mechanism: the
/// declaration stays (greppable, and its verb goes through the audit's
/// registry gate), the row does not exist.
///
/// The controls rail still declares nothing: the merged top bar describes the
/// window, and `chrome::status_rail` is still drawn by nothing, so a status
/// entry here would be an entry nothing draws.
#[test]
fn the_toolbar_is_declared_but_the_row_vanishes_when_nothing_can_act() {
    let subjects = subjects(&loaded());
    let chart = &subjects[&CHART];
    assert_eq!(
        chart.toolbar.len(),
        1,
        "the chart pane declares exactly its clear-selection control"
    );
    assert_eq!(chart.toolbar[0].verb.as_str(), "clear-selection");
    assert_eq!(
        chart.toolbar[0].location,
        ToolbarLocation::Hidden,
        "no plot of examples/dashboard.yaml declares a gesture, so the \
         control is declared-but-withheld"
    );
    assert!(
        !chrome::Toolbar::new(&chart.toolbar).has_something_to_say(),
        "a withheld-only toolbar summons no row"
    );
    assert!(
        chart.status.is_empty(),
        "a live-queried preview makes no run-state claim, so no status line"
    );

    let controls = &subjects[&CONTROLS];
    assert!(controls.toolbar.is_empty(), "the rail declares no toolbar");
    assert!(controls.status.is_empty(), "the rail declares no status");
}

/// Over a spec that *does* declare a brush, the same control is offered —
/// same declaration, different location — and the row exists.
#[test]
fn a_brushable_dashboard_offers_the_clear_selection_control() {
    let doc = ChartDoc::headless(compose_spec(CROSSFILTER).expect("compose crossfilter"));
    let chart = &subjects(&doc)[&CHART];
    assert_eq!(chart.toolbar.len(), 1);
    assert_eq!(
        chart.toolbar[0].location,
        ToolbarLocation::Leading,
        "a brushable plot offers the control in the row"
    );
    assert!(
        !chart.toolbar[0].enabled,
        "nothing is committed yet, so the control is offered but disabled"
    );
    assert!(chrome::Toolbar::new(&chart.toolbar).has_something_to_say());
}

/// A preview annotated with materialised run output rails that state — the
/// one vocabulary, spelled by its own type, never a second set.
#[test]
fn an_annotated_preview_rails_its_run_state() {
    let mut composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    composed = composed.with_run_state(RunState::StaleUpstream);
    let doc = ChartDoc::headless(composed);
    let chart = &subjects(&doc)[&CHART];
    assert_eq!(chart.status.len(), 1, "one standing run-state entry");
    assert_eq!(chart.status[0].id, "run-state");
    assert_eq!(chart.status[0].text, RunState::StaleUpstream.label());
}

// ---------------------------------------------------------------------------
// The default arrangement
// ---------------------------------------------------------------------------

/// The registry is the single declaration of the view's shape: the chart is
/// the centre pane, the editor is a tab beside it, and the controls are a
/// right rail.
///
/// The strip placement is the load-bearing part, in both directions. The
/// chart and the editor sit under one tab strip, whose tabs name them — so
/// their header bands are suppressed and neither grows a second name. The
/// rail is *not* under a strip: tabbing it would silently swallow its own
/// header band, and its name with it.
#[test]
fn the_default_dock_is_a_tabbed_chart_and_editor_with_a_controls_rail() {
    let registry = chart_registry();
    let slots: BTreeMap<ItemId, Slot> = registry
        .specs()
        .iter()
        .map(|spec| (spec.id, spec.slot))
        .collect();
    assert_eq!(slots[&CHART], Slot::Centre, "the chart is the centre pane");
    assert_eq!(
        slots[&EDITOR],
        Slot::CentreTab,
        "the editor is a tab beside the centre"
    );
    assert!(
        matches!(
            slots[&CONTROLS],
            Slot::Rail {
                side: DockSide::Right,
                ..
            }
        ),
        "the controls are a right rail, got {:?}",
        slots[&CONTROLS]
    );

    let tree = registry.default_tree();
    let tabbed = brightfield_workbench::workspace::tabbed_tiles_of(&tree);
    for item in [CHART, EDITOR] {
        let tile =
            brightfield_workbench::workspace::tile_of(&tree, PaneKey::new(ViewKind::Charts, item))
                .unwrap_or_else(|| panic!("{item} is in the default tree"));
        assert!(
            tabbed.contains(&tile),
            "{item} is a centre tab: the strip names it, its header band is \
             suppressed"
        );
    }
    let rail =
        brightfield_workbench::workspace::tile_of(&tree, PaneKey::new(ViewKind::Charts, CONTROLS))
            .expect("the controls rail is in the default tree");
    assert!(
        !tabbed.contains(&rail),
        "the rail is not under a tab strip, so it keeps its header band"
    );
}

/// The window the shell asks for is sized from the *same* share the dock lays
/// the rail out with, and the chart pane's content box it produces fits the
/// dashboard — **in both axes**.
///
/// This is the test for the deletion that mattered most here. Three numbers used
/// to describe one layout — a side panel pinned at 180 logical points, a
/// `window_size` that budgeted 214 for it, and a `main.rs` that budgeted 200 —
/// and no test could see that they disagreed, because each was correct on its
/// own terms. Now the share is the single declaration and this walks the
/// arithmetic that depends on it.
///
/// It walked the *width* only, and said in its own doc that it walked "the
/// arithmetic". The height line was `h > composed.height`, which one point of
/// chrome budget satisfies — and one point of chrome budget for a top bar, a
/// header band and two pane frames is 95 points short. It was mutated to
/// `composed.height + 1.0` and all eight tests here stayed green while the
/// window clipped the bottom seventeen rows of its own raster. Both axes are
/// walked the same way now: outward from the raster through every component
/// that consumes space, with the leftover slack named and bounded.
#[test]
fn the_window_is_sized_from_the_rail_share_it_lays_out() {
    let composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    let (w, h) = chart_window_size(&composed);
    let centre = 1.0 - controls_share();
    let inset = chrome::pane_content_inset();

    // Across: the window insets to the dock, the dock gives up the gap between
    // the two tiles, the chart takes `centre` of what is left, and the pane
    // frame insets it again on both sides. The content box holds the raster
    // AND the legend margin band beside it — dashboard.yaml's scatter maps
    // fill, so its band is real here, and a `band_width` nobody budgeted
    // would come straight out of the raster's pixels.
    let content_w = (w - 2.0 * DOCK_INSET - TILE_GAP) * centre - 2.0 * inset;
    let need_w = composed.width as f32 + brightfield_shell::legend::band_width(&composed);
    assert!(
        brightfield_shell::legend::band_width(&composed) > 0.0,
        "dashboard.yaml maps fill, so this walk must include a real band"
    );
    assert!(
        content_w >= need_w,
        "the chart pane's content box is {content_w:.2}pt across for a \
         {need_w:.0}pt raster+legend — something will be clipped"
    );

    // Down: the window gives up its one band — the charts view draws no hint
    // bar — insets to the dock, and the pane frame takes its header band and
    // its padding above and below. The toolbar band is a term too, and for
    // this gesture-less dashboard it must be exactly zero — quiet means no
    // row, and no row means no budget.
    let content_h = h - BAR_HEIGHT - 2.0 * DOCK_INSET - chrome::header_band_height() - 2.0 * inset;
    assert_eq!(
        chart_toolbar_band(&composed),
        0.0,
        "no plot of dashboard.yaml declares a gesture, so no toolbar band"
    );
    let need_h = composed.height as f32 + chart_toolbar_band(&composed);
    assert!(
        content_h >= need_h,
        "the chart pane's content box is {content_h:.2}pt tall for a \
         {need_h:.0}pt raster — the raster will be clipped"
    );

    // And in neither axis is the leftover a fudge factor. Every term above is
    // read from the component that consumes it, so the *only* slack
    // `chart_window_size` may have is its rounding up to whole logical points:
    // strictly less than one point, per axis. An inequality that any positive
    // number satisfies is what let the height budget be 95 points short.
    for (axis, slack) in [("across", content_w - need_w), ("down", content_h - need_h)] {
        assert!(
            slack < 1.0,
            "the chart pane's content box has {slack:.2}pt of slack {axis} — \
             more than the sub-point rounding `chart_window_size` is allowed, so \
             some of the budget is a fudge factor rather than a component"
        );
    }
}

/// The window `chart_window_size` asks for really does fit the raster the chart
/// pane presents — checked by laying a **real frame** out, not by re-running the
/// same arithmetic.
///
/// The sibling above walks the budget term by term, and a walk can only ever be
/// as right as its author's model of the dock. This one has no model: it runs
/// the window through `egui::Context::run_ui` at exactly the window size the
/// shell asks for, and reads back the content box `egui_tiles` and
/// `chrome::pane_frame` between them actually handed the chart pane. If any of
/// the four components in that budget changes its height, this reddens whether
/// or not anyone remembered to update the arithmetic.
///
/// GPU-free: `MeridianApp::headless` has no device, so the pane paints nothing —
/// but it is handed the same rect either way, and `ChartPane::ui` records it
/// before it looks for a texture. Two frames, because the first installs the
/// font atlas and settles the layout, exactly as the capture path does.
///
/// It also holds the charts view to drawing no key-hint bar, without naming
/// one: a hint bar is one [`BAR_HEIGHT`] band — 32 logical points — and
/// `chart_window_size` budgets for a single band, so a hint bar on this view
/// would take those points out of the box read back below.
#[test]
fn the_window_it_asks_for_fits_the_raster_it_presents() {
    let composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    let (dash_w, dash_h) = (composed.width as f32, composed.height as f32);
    let (w, h) = chart_window_size(&composed);

    let mut app = MeridianApp::headless(Boot::charts(composed), Mode::Light);
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

    let box_ = app
        .chart_viewport()
        .expect("the chart pane drew, so it recorded the box it was given");
    assert!(
        box_.width() >= dash_w && box_.height() >= dash_h,
        "a {w}×{h} window gives the chart pane a {:.2}×{:.2} content box, \
         and it presents a {dash_w:.0}×{dash_h:.0} raster into it — \
         {:.2}pt of it is outside the pane's clip rect and never reaches the window",
        box_.width(),
        box_.height(),
        (dash_h - box_.height()).max(dash_w - box_.width()),
    );
}
