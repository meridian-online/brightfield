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

use brightfield_shell::app::{
    chart_registry, publish_item_ids, window_size_for, ChartDoc, CHART, CONTROLS,
};
use brightfield_shell::pipeline::{compose_spec, Composed};
use brightfield_workbench::registry::{DockSide, Slot};
use brightfield_workbench::{audit, ItemId, PaneKey, Subject, ViewKind};

const DASHBOARD: &str = "../../examples/dashboard.yaml";

/// The real fixture, as a document with no device behind it.
fn loaded() -> ChartDoc {
    ChartDoc::headless(compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml"))
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

/// Each pane names itself once, and resolves its keys in the chart grammar's
/// context.
///
/// The title is the whole of the pane's name now. Before this increment the
/// controls rail's own body opened with a bold `Controls` label and the chart
/// had no name at all; the only heading on the surface was the window's.
#[test]
fn each_pane_names_itself_once_and_binds_in_the_workspace_context() {
    let names: BTreeMap<ItemId, String> = subjects(&loaded())
        .into_iter()
        .map(|(id, s)| {
            assert_eq!(
                s.key_context,
                brightfield_keys::BindingContext::Workspace,
                "{id} resolves keys somewhere other than the chart context"
            );
            (id, s.title)
        })
        .collect();
    assert_eq!(names[&CHART], "Chart");
    assert_eq!(names[&CONTROLS], "Controls");
}

/// A pane declares no chrome it cannot be held to: no toolbar control and no
/// status line, because the shell's top bar is still its own.
///
/// A statement about *this* increment rather than a rule for all time — the top
/// bar's mode line becomes a status entry when the one-app shell lands. Until
/// then, an entry appearing here would be an entry nothing draws, which is worse
/// than none.
#[test]
fn no_pane_declares_chrome_the_shell_does_not_yet_draw() {
    for (id, subject) in subjects(&loaded()) {
        assert!(
            subject.toolbar.is_empty(),
            "{id} declares a toolbar control"
        );
        assert!(subject.status.is_empty(), "{id} declares a status line");
    }
}

// ---------------------------------------------------------------------------
// The default arrangement
// ---------------------------------------------------------------------------

/// The registry is the single declaration of the view's shape: the chart is the
/// centre pane and the controls are a right rail.
///
/// Neither pane is under a tab strip, which is the load-bearing part: a pane in
/// a tab strip has its header band suppressed because the strip already names
/// it, so if either of these became a tab it would silently lose its name rather
/// than grow a second one.
#[test]
fn the_default_dock_is_a_chart_with_a_controls_rail() {
    let registry = chart_registry();
    let slots: BTreeMap<ItemId, Slot> = registry
        .specs()
        .iter()
        .map(|spec| (spec.id, spec.slot))
        .collect();
    assert_eq!(slots[&CHART], Slot::Centre, "the chart is the centre pane");
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
    for item in [CHART, CONTROLS] {
        let tile =
            brightfield_workbench::workspace::tile_of(&tree, PaneKey::new(ViewKind::Charts, item))
                .unwrap_or_else(|| panic!("{item} is in the default tree"));
        assert!(
            !tabbed.contains(&tile),
            "{item} is under a tab strip, so its header band is suppressed"
        );
    }
}

/// The published id vocabulary is derived from the registry, not written beside
/// it.
///
/// The protocol view shipped a hand-written `static [ItemId; 4]` next to its
/// registry, which is a second declaration by definition: a pane added to the
/// registry and forgotten in the array compiled, ran, and produced a pane whose
/// saved layout could never load. This asserts the two agree — and, because
/// `publish` is additive and process-global, that every id this binary has
/// published came from the registry.
#[test]
fn the_published_vocabulary_is_the_registry_and_nothing_else() {
    publish_item_ids();
    let declared = chart_registry().ids();
    let known = ItemId::known();
    for id in &declared {
        assert!(known.contains(id), "{id} is declared but never published");
    }
    for id in known {
        assert!(
            declared.contains(id),
            "{id} was published by something other than the chart registry"
        );
    }

    // The point of the vocabulary: a saved layout naming these panes loads.
    for item in declared {
        let key = PaneKey::new(ViewKind::Charts, item);
        let json = serde_json::to_string(&key).expect("a pane key serialises");
        assert_eq!(
            serde_json::from_str::<PaneKey>(&json).expect("and round trips"),
            key
        );
    }
}

/// The window the shell asks for is sized from the *same* share the dock lays
/// the rail out with, and it is big enough for the dashboard.
///
/// This is the test for the deletion that mattered most here. Three numbers used
/// to describe one layout — a side panel pinned at 180 logical points, a
/// `window_size` that budgeted 214 for it, and a `main.rs` that budgeted 200 —
/// and no test could see that they disagreed, because each was correct on its
/// own terms. Now the share is the single declaration and this walks the
/// arithmetic that depends on it.
#[test]
fn the_window_is_sized_from_the_rail_share_it_lays_out() {
    let composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    let (w, h) = window_size_for(&composed);
    let centre = 1.0 - controls_share();

    let chart_tile = w * centre;
    let dashboard = composed.width as f32;
    assert!(
        chart_tile >= dashboard,
        "the chart tile is {chart_tile:.1}pt for a {dashboard:.1}pt dashboard — it will clip"
    );
    // And not absurdly wide: the slack is pane chrome, not a fudge factor.
    assert!(
        chart_tile - dashboard < 64.0,
        "the chart tile has {:.1}pt of slack over the dashboard",
        chart_tile - dashboard
    );
    assert!(
        h > composed.height as f32,
        "the window is shorter than the dashboard"
    );
}
