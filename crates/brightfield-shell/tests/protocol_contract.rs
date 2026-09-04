//! The protocol view against the shell contract, without a GPU.
//!
//! Everything asserted here is a property of the view's *declaration* — its
//! registry, its four subjects, its default arrangement — and none of it needs
//! a device. That is the payoff of a [`Subject`](brightfield_workbench::Subject)
//! being plain data: the pane headers, the empty states and the key context of
//! a whole surface can be pinned in a unit test that runs in milliseconds,
//! where before the only thing that could see them was a full-window pixel
//! baseline on a GPU.
//!
//! The pixel tier (`surfaces.rs`) still covers what this cannot: what the
//! panel *looks* like. These two are complements, and the split is deliberate —
//! a rule that can be stated in data should not be paid for in a megabyte of
//! PNG.

use std::collections::BTreeMap;

use brightfield_protocol::layout::Flow;
use brightfield_shell::protocol::{
    load_protocol_offline, protocol_registry, ProtocolDoc, ProtocolInputs, ProtocolModel, CANVAS,
    INSPECTOR, OUTLINE, STEPS,
};
use brightfield_workbench::registry::Slot;
use brightfield_workbench::{audit, ItemId, PaneKey, Subject};

const EDGAR: &str = "../../examples/protocol/edgar_gleif/arcform.yaml";

/// The real fixture, as a document with no device behind it.
fn loaded() -> ProtocolDoc {
    let inputs = load_protocol_offline(EDGAR).expect("load edgar_gleif");
    ProtocolDoc::headless(ProtocolModel::new(inputs, Flow::Vertical))
}

/// Every pane's subject over one document, keyed by item id.
fn subjects(doc: &ProtocolDoc) -> BTreeMap<ItemId, Subject> {
    protocol_registry()
        .specs()
        .iter()
        .map(|spec| (spec.id, (spec.make)().subject(doc)))
        .collect()
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// The workbench audit, over the protocol view.
///
/// This is the one assertion that replaces "somebody remembered to write an
/// empty state". It constructs all four panes, asks each for its subject over
/// an empty document, and rejects a missing empty state, prose that breaks the
/// house style, a verb the keyboard registry does not have, and a rail or tab
/// that names no verb to show and hide it.
///
/// Watched redden, one mutation each: making the outline pane's
/// `empty_state` answer `None` gives *"protocol-outline: shows no empty
/// state on an empty document"* (forgetting the method entirely no longer
/// compiles — it is required on `Item`);
/// putting a full stop on the steps headline gives *"protocol-steps: headline
/// takes no terminal period"*; dropping the inspector rail's `toggle` gives
/// *"protocol-inspector: a rail or tab with no toggle verb"*. Each of those is
/// a real defect the panel shipped with before this increment, and none of
/// them is visible to a pixel baseline of the fixture, because the fixture is
/// never empty.
#[test]
fn every_protocol_pane_passes_the_contract_audit() {
    let empty = ProtocolDoc::empty();
    audit(&protocol_registry(), &empty).expect("the protocol view is on the contract");
}

/// An empty document really is empty, so the audit above is asserting on the
/// branch it thinks it is.
///
/// Without this, `ProtocolInputs::empty` could quietly grow a node and every
/// empty-state assertion in the file would pass by never reaching one.
#[test]
fn the_empty_document_has_nothing_in_it() {
    let doc = ProtocolDoc::empty();
    assert!(!doc.model.has_assets(), "an empty document declares assets");
    assert!(
        !doc.model.has_selection(doc.canvas_holds.node()),
        "an empty document has a selection"
    );
    assert!(doc.model.sheet().is_empty(), "an empty document has steps");
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
/// the whole panel — the shell draws the empty state *instead of* the pane's
/// own body, so `!doc.model.has_assets()` written without the `!` would ship a
/// four-pane window with four apologies in it. Watched redden: dropping that
/// `!` fails here on `protocol-outline`.
#[test]
fn no_pane_is_empty_over_a_real_protocol() {
    for (id, subject) in subjects(&loaded()) {
        assert!(
            subject.empty_state.is_none(),
            "{id} claims to be empty over edgar_gleif: {:?}",
            subject.empty_state
        );
    }
}

/// **The Operator's empty-state body names no pane "the outline", and reads
/// true whichever the canvas holds.**
///
/// Built with `ProtocolDoc::headless` directly rather than through a window,
/// so `canvas_holds` stays at its unreconciled default (`Graph`) instead of
/// the `View` a real housing-fixture window latches before its first frame —
/// the one construction that leaves this pane's own fallback with nothing to
/// fall back to, and so the one that still reaches this text at all. A real
/// window over this same fixture never sees it: `navigator_spine.rs`'s
/// `switching_to_operator_on_a_fresh_open_describes_the_canvas_held_table` is
/// the pin that a live window's fallback keeps this pane out of its empty
/// state in the first place.
#[test]
fn the_operators_empty_state_over_the_housing_fixture_names_no_outline() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/california_housing_sample.csv");
    let opened =
        brightfield_shell::data_file::open(&path.to_string_lossy()).expect("the CSV opens");
    let inputs = opened.protocol.inputs().expect("the Protocol builds");
    let doc = ProtocolDoc::headless(ProtocolModel::new(inputs, Flow::Vertical));

    assert!(
        !doc.model.has_selection(doc.canvas_holds.node()),
        "this test is about the branch where nothing is selected and the \
         canvas holds no node either — construct it differently if this \
         starts failing"
    );

    let empty = subjects(&doc)[&INSPECTOR]
        .empty_state
        .clone()
        .expect("the Operator has nothing to describe here");
    assert!(
        !empty.body.to_lowercase().contains("outline"),
        "the Operator's empty state still names the pane \"outline\": {:?}",
        empty.body
    );
    assert!(
        empty.body.to_lowercase().contains("rail"),
        "the Operator's empty state offers no way to reach content from the \
         rail: {:?}",
        empty.body
    );
}

// ---------------------------------------------------------------------------
// What the panes declare
// ---------------------------------------------------------------------------

/// Each pane names itself once, and resolves its keys in the Protocol context.
///
/// The title is the whole of the pane's name now: the shell draws it as a
/// header band for a rail and as a tab title for a centre tab, never both, and
/// no pane draws either. Before this increment the outline rail's own header
/// said `OUTLINE`, the inspector's said `INSPECTOR` in a different treatment,
/// the steps sheet's said `STEPS — the flat run-ordered list` while its tab
/// said `Steps`, and the canvas had none at all.
#[test]
fn each_pane_names_itself_once_and_binds_in_the_protocol_context() {
    let names: BTreeMap<ItemId, String> = subjects(&loaded())
        .into_iter()
        .map(|(id, s)| {
            assert_eq!(
                s.key_context,
                brightfield_keys::BindingContext::Protocol,
                "{id} resolves keys somewhere other than the protocol context"
            );
            (id, s.title)
        })
        .collect();
    // The item id is still `protocol-outline` — a saved layout files a pane
    // under it — while the pane's name is what it draws: the Protocol, as a
    // spine of what it reads, does and makes, with the outline's columns
    // beneath.
    assert_eq!(names[&OUTLINE], "Protocol");
    assert_eq!(names[&CANVAS], "Canvas");
    assert_eq!(names[&INSPECTOR], "Operator");
    assert_eq!(names[&STEPS], "Steps");
}

/// A pane declares no chrome it cannot be held to: no toolbar control and no
/// status line.
///
/// The one-app window has landed and this still holds, which is worth saying
/// plainly because the note here used to promise the opposite. The hint bar and
/// the flow toggle describe the *view* — its key grammar, its reading axis —
/// and neither is any one pane's to declare; they are drawn by the window, from
/// the view's model. `chrome::toolbar_row` is still drawn by nothing here, so a
/// toolbar control would be a control nothing draws. The status rail *is*
/// drawn now — the window rails the focused pane's entries and the one
/// activity indicator — so the status half of this assertion changed meaning
/// without changing truth: these panes still declare no status line because
/// their status spellings (the node status words, the inspector gloss) live
/// in the canvas and the inspector, and a rail entry would be a second
/// spelling of the same facts.
#[test]
fn no_pane_declares_chrome_the_panel_does_not_yet_draw() {
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

/// The registry is the single declaration of the view's shape, and the shape
/// is the one the panel has always had: outline · (canvas | steps) · inspector.
///
/// The centre tab strip is the load-bearing part. It is what suppresses the
/// canvas's and the steps sheet's header bands — a pane under a tab strip is
/// already named by the strip — so if the steps sheet ever stopped being a
/// centre tab it would silently grow a second name.
#[test]
fn the_default_dock_is_outline_canvas_steps_inspector() {
    let registry = protocol_registry();
    let slots: BTreeMap<ItemId, Slot> = registry
        .specs()
        .iter()
        .map(|spec| (spec.id, spec.slot))
        .collect();
    assert_eq!(
        slots[&CANVAS],
        Slot::Centre,
        "the canvas is the centre pane"
    );
    assert_eq!(slots[&STEPS], Slot::CentreTab, "the steps sheet is a tab");
    assert!(
        matches!(slots[&OUTLINE], Slot::Rail { .. }),
        "the outline is a rail"
    );
    assert!(
        matches!(slots[&INSPECTOR], Slot::Rail { .. }),
        "the inspector is a rail"
    );

    // And the tree the registry builds really does put the two centre panes
    // under one tab container and leave the rails outside it.
    let tree = registry.default_tree();
    let tabbed = brightfield_workbench::workspace::tabbed_tiles_of(&tree);
    let tile = |item| {
        brightfield_workbench::workspace::tile_of(&tree, PaneKey::new(item))
            .unwrap_or_else(|| panic!("{item} is in the default tree"))
    };
    assert!(tabbed.contains(&tile(CANVAS)), "the canvas is tabbed");
    assert!(tabbed.contains(&tile(STEPS)), "the steps sheet is tabbed");
    assert!(
        !tabbed.contains(&tile(OUTLINE)),
        "the outline rail is tabbed"
    );
    assert!(
        !tabbed.contains(&tile(INSPECTOR)),
        "the inspector rail is tabbed"
    );
}

/// The empty inputs helper is public because the audit needs it; keep it
/// honest about being a *protocol* with nothing in it rather than a
/// half-built one.
#[test]
fn empty_inputs_describe_a_protocol_with_nothing_in_it() {
    let inputs = ProtocolInputs::empty();
    assert!(inputs.graph_collapsed.nodes.is_empty());
    assert!(inputs.graph_full.nodes.is_empty());
    assert!(inputs.sheet_rows.is_empty());
}
