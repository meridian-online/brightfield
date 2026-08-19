//! The contract gate, and the fixture that proves it bites.
//!
//! Everything here is headless: no adapter, no window, no renderer. That is
//! the payoff of [`Subject`] being plain data, and it is why these assertions
//! can be cheap enough to run on every push.
//!
//! # What is being gated, and what is not
//!
//! The gate itself is [`brightfield_workbench::audit`], which ships in the
//! crate so that a view gates itself with one call rather than by
//! re-implementing the rule; later steps point it at the real registries as
//! those views move onto the contract. Right now there are no real items, so
//! it is pointed at a fixture registry. That is
//! not a self-fulfilling test: the fixture deliberately contains a *bad*
//! registry as well as a good one, and the negative cases assert the gate
//! rejects each specific defect. A gate that has never been shown to fail is
//! not a gate.

use brightfield_keys::BindingContext;
use brightfield_workbench::chrome;
use brightfield_workbench::{
    audit, Affordance, DockSide, EmptyState, Handled, HideAffordance, Icon, Item, ItemCtx, ItemId,
    ItemRegistry, ItemSpec, Mode, PaneKey, Slot, StatusEntry, StatusSide, Subject, Tone,
    ToolbarEntry, ToolbarLocation, Verb,
};

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// The gate, applied to the fixture through the crate's own [`audit`].
///
/// It used to be a private copy of the rule living here in the test file,
/// which meant every downstream view would have had to re-write it — the
/// per-surface drift the crate exists to end. It is now
/// [`brightfield_workbench::audit`], and this is a thin adapter that supplies
/// the fixture's empty document.
fn gate(reg: &ItemRegistry<Doc>) -> Result<(), String> {
    audit(reg, &Doc::default())
}

// ---------------------------------------------------------------------------
// A fixture view
// ---------------------------------------------------------------------------

/// A stand-in document. `Default` is the empty document the gate probes with.
#[derive(Default)]
struct Doc {
    rows: Vec<&'static str>,
    edits: u32,
}

const LIST: ItemId = ItemId::new("fixture-list");
const DETAIL: ItemId = ItemId::new("fixture-detail");
const NOTES: ItemId = ItemId::new("fixture-notes");

/// Real registry longnames, so the verb gate is checked against the actual
/// keyboard grammar rather than a stub of it.
fn open_verb() -> Verb {
    Verb::new("open-palette")
}
fn toggle_verb() -> Verb {
    Verb::new("toggle-focus")
}
fn reload_verb() -> Verb {
    Verb::new("reload-spec")
}

/// The centre pane.
#[derive(Default)]
struct ListItem {
    /// View-local state, which is the only kind an item is allowed to hold.
    scroll: f32,
}

impl Item<Doc> for ListItem {
    fn item_id(&self) -> ItemId {
        LIST
    }

    fn empty_state(&self, doc: &Doc) -> Option<EmptyState> {
        doc.rows.is_empty().then(|| {
            EmptyState::new(
                Icon("list"),
                "No rows yet",
                "Rows appear here once a protocol has run",
            )
            .with_next(Affordance::new("Reload", reload_verb()))
        })
    }

    fn describe(&self, doc: &Doc) -> Subject {
        Subject::new("Rows", Icon("list"), BindingContext::Workspace)
            .with_toolbar(ToolbarEntry::button("reload", "Reload", reload_verb()))
            .with_toolbar(
                ToolbarEntry::button("hidden", "Not offered", open_verb())
                    .at(ToolbarLocation::Hidden),
            )
            .with_status(StatusEntry {
                id: "count",
                side: StatusSide::Trailing,
                text: format!("{} rows", doc.rows.len()),
                tone: Tone::Neutral,
                hide: HideAffordance::WithRail,
            })
    }

    fn ui(&mut self, doc: &mut Doc, ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {
        self.scroll = ui.max_rect().top();
        for row in &doc.rows {
            ui.label(*row);
        }
    }

    fn perform(&mut self, doc: &mut Doc, verb: Verb, _cx: &mut ItemCtx<'_>) -> Handled {
        if verb == reload_verb() {
            doc.edits += 1;
            Handled::Yes
        } else {
            Handled::No
        }
    }
}

/// A right rail.
#[derive(Default)]
struct DetailItem;

impl Item<Doc> for DetailItem {
    fn item_id(&self) -> ItemId {
        DETAIL
    }
    fn empty_state(&self, doc: &Doc) -> Option<EmptyState> {
        doc.rows.is_empty().then(|| {
            EmptyState::new(
                Icon("inspector"),
                "Nothing selected",
                "Pick a row to see what it is made of",
            )
        })
    }
    fn describe(&self, _doc: &Doc) -> Subject {
        Subject::new("Detail", Icon("inspector"), BindingContext::Workspace)
    }
    fn ui(&mut self, _doc: &mut Doc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        if ui.button("focus me").clicked() {
            cx.take_focus();
        }
    }
}

/// A centre tab.
#[derive(Default)]
struct NotesItem;

impl Item<Doc> for NotesItem {
    fn item_id(&self) -> ItemId {
        NOTES
    }
    fn empty_state(&self, _doc: &Doc) -> Option<EmptyState> {
        Some(EmptyState::new(
            Icon("note"),
            "No notes",
            "Notes you write against a run are kept here",
        ))
    }
    fn describe(&self, _doc: &Doc) -> Subject {
        Subject::new("Notes", Icon("note"), BindingContext::Editor)
    }
    fn ui(&mut self, _doc: &mut Doc, _ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {}
}

fn good_registry() -> ItemRegistry<Doc> {
    ItemRegistry::new(vec![
        ItemSpec {
            id: LIST,
            slot: Slot::Centre,
            toggle: None,
            make: || Box::new(ListItem::default()),
        },
        ItemSpec {
            id: DETAIL,
            slot: Slot::Rail {
                side: DockSide::Right,
                share: 0.22,
            },
            toggle: Some(toggle_verb()),
            make: || Box::new(DetailItem),
        },
        ItemSpec {
            id: NOTES,
            slot: Slot::CentreTab,
            toggle: Some(toggle_verb()),
            make: || Box::new(NotesItem),
        },
    ])
}

// ---------------------------------------------------------------------------
// The gate passes on a well-formed view
// ---------------------------------------------------------------------------

#[test]
fn a_well_formed_view_passes_the_gate() {
    assert_eq!(gate(&good_registry()), Ok(()));
}

// ---------------------------------------------------------------------------
// …and bites on each specific defect
// ---------------------------------------------------------------------------

/// The defect the whole empty-state-is-required design exists to catch: a
/// pane that renders a header and silence when it has nothing to show.
/// `Item::empty_state` being a required method forces the *question* at
/// compile time; answering `None` over an empty document is the wrong
/// *answer*, and it is the audit that rejects it.
#[derive(Default)]
struct NoEmptyState;
impl Item<Doc> for NoEmptyState {
    fn item_id(&self) -> ItemId {
        LIST
    }
    fn empty_state(&self, _doc: &Doc) -> Option<EmptyState> {
        None
    }
    fn describe(&self, _doc: &Doc) -> Subject {
        Subject::new("Rows", Icon("list"), BindingContext::Workspace)
    }
    fn ui(&mut self, _doc: &mut Doc, _ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {}
}

#[test]
fn a_pane_with_no_empty_state_fails_the_gate() {
    let reg = ItemRegistry::new(vec![ItemSpec {
        id: LIST,
        slot: Slot::Centre,
        toggle: None,
        make: || Box::new(NoEmptyState),
    }]);
    let err = gate(&reg).unwrap_err();
    assert!(err.contains("shows no empty state"), "{err}");
}

#[derive(Default)]
struct SloppyProse;
impl Item<Doc> for SloppyProse {
    fn item_id(&self) -> ItemId {
        LIST
    }
    fn empty_state(&self, _doc: &Doc) -> Option<EmptyState> {
        Some(EmptyState::new(Icon("list"), "no rows yet.", ""))
    }
    fn describe(&self, _doc: &Doc) -> Subject {
        Subject::new("Rows", Icon("list"), BindingContext::Workspace)
    }
    fn ui(&mut self, _doc: &mut Doc, _ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {}
}

#[test]
fn empty_state_prose_is_held_to_the_house_style() {
    let reg = ItemRegistry::new(vec![ItemSpec {
        id: LIST,
        slot: Slot::Centre,
        toggle: None,
        make: || Box::new(SloppyProse),
    }]);
    // The gate reports the first rule that bites; the terminal period is
    // checked before the case, and both before the body.
    let err = gate(&reg).unwrap_err();
    assert!(err.contains("terminal period"), "{err}");
}

#[test]
fn a_rail_with_no_toggle_verb_fails_the_gate() {
    let reg = ItemRegistry::new(vec![
        ItemSpec {
            id: LIST,
            slot: Slot::Centre,
            toggle: None,
            make: || Box::new(ListItem::default()),
        },
        ItemSpec {
            id: DETAIL,
            slot: Slot::Rail {
                side: DockSide::Left,
                share: 0.2,
            },
            toggle: None,
            make: || Box::new(DetailItem),
        },
    ]);
    let err = gate(&reg).unwrap_err();
    assert!(err.contains("no toggle verb"), "{err}");
}

// ---------------------------------------------------------------------------
// Verbs
// ---------------------------------------------------------------------------

/// Note what this does and does not prove. It walks the fixture's declared
/// verbs and finds them registered — but every one of them came from
/// [`Verb::new`], which already asserted as much in this (debug) build, so it
/// cannot fail while `is_registered` is intact. Its real job is to pin
/// [`Subject::declared_verbs`]'s reach: that the walk visits toolbar entries,
/// the empty state's affordance and a status entry's dismiss verb, so a new
/// verb-bearing field cannot escape the gate by being forgotten here. The
/// check that `is_registered` still *works* is in `subject.rs`'s unit tests.
#[test]
fn every_verb_the_fixture_declares_is_a_real_command() {
    let reg = good_registry();
    let doc = Doc::default();
    let mut seen = 0;
    for spec in reg.specs() {
        for verb in (spec.make)().subject(&doc).declared_verbs() {
            assert!(verb.is_registered(), "{:?}", verb.as_str());
            seen += 1;
        }
    }
    assert!(
        seen > 0,
        "the fixture declares no verbs, so this proves nothing"
    );
}

/// The *negative* half of the verb gate — a `Verb` that never went through
/// [`Verb::new`] and names nothing in the registry — cannot be built from
/// here: `Verb`'s field is private, so an integration test can only reach
/// verbs that already passed the constructor's debug assertion and therefore
/// cannot fail `is_registered()`. It lives in `subject.rs`'s unit tests,
/// which are inside the module and can construct it.
///
/// What this test can honestly claim is the assumption that one rests on:
/// the longname it smuggles in really is absent from the keyboard registry.
#[test]
fn the_longname_the_unit_test_smuggles_in_is_genuinely_unregistered() {
    let reg = brightfield_keys::registry();
    assert!(!reg.iter().any(|v| v.longname == "summon-a-pony"));
}

#[test]
fn a_verbs_keystroke_comes_from_the_registry_not_from_a_stored_copy() {
    let verb = reload_verb();
    let expected = brightfield_keys::registry()
        .iter()
        .find(|v| v.longname == verb.as_str())
        .and_then(brightfield_keys::VerbEntry::primary_key);
    assert_eq!(verb.keys(), expected);
}

// ---------------------------------------------------------------------------
// Structural invariants
// ---------------------------------------------------------------------------

#[test]
fn a_registry_needs_exactly_one_centre_pane() {
    let two_centres = std::panic::catch_unwind(|| {
        ItemRegistry::new(vec![
            ItemSpec {
                id: LIST,
                slot: Slot::Centre,
                toggle: None,
                make: || Box::new(ListItem::default()) as Box<dyn Item<Doc>>,
            },
            ItemSpec {
                id: DETAIL,
                slot: Slot::Centre,
                toggle: None,
                make: || Box::new(DetailItem) as Box<dyn Item<Doc>>,
            },
        ])
    });
    assert!(two_centres.is_err());
}

#[test]
fn duplicate_item_ids_are_refused() {
    let dup = std::panic::catch_unwind(|| {
        ItemRegistry::new(vec![
            ItemSpec {
                id: LIST,
                slot: Slot::Centre,
                toggle: None,
                make: || Box::new(ListItem::default()) as Box<dyn Item<Doc>>,
            },
            ItemSpec {
                id: LIST,
                slot: Slot::CentreTab,
                toggle: Some(toggle_verb()),
                make: || Box::new(NotesItem) as Box<dyn Item<Doc>>,
            },
        ])
    });
    assert!(dup.is_err());
}

/// A crude but effective tripwire against someone putting a model handle in
/// the pane identity. `PaneKey` is a view discriminant and a `&'static str`;
/// anything that pushes it past a couple of words is a document sneaking back
/// in, which is the shape this crate exists to prevent.
#[test]
fn a_pane_key_stays_small_and_copy() {
    assert!(
        std::mem::size_of::<PaneKey>() <= 24,
        "PaneKey grew to {} bytes",
        std::mem::size_of::<PaneKey>()
    );
    fn assert_copy<T: Copy>() {}
    assert_copy::<PaneKey>();
}

#[test]
fn the_registry_instantiates_one_item_per_spec_keyed_by_pane() {
    let reg = good_registry();
    let items = reg.instantiate();
    assert_eq!(items.len(), reg.specs().len());
    for spec in reg.specs() {
        let key = PaneKey::new(spec.id);
        assert_eq!(items[&key].item_id(), spec.id);
    }
}

#[test]
fn the_default_tree_holds_exactly_the_registered_panes() {
    let reg = good_registry();
    let tree = reg.default_tree();
    let mut panes: Vec<PaneKey> = tree.tiles.tiles().filter_map(tile_pane).copied().collect();
    panes.sort_unstable();
    let mut want: Vec<PaneKey> = reg.ids().into_iter().map(|i| reg.pane_key(i)).collect();
    want.sort_unstable();
    assert_eq!(panes, want);
}

fn tile_pane(tile: &egui_tiles::Tile<PaneKey>) -> Option<&PaneKey> {
    match tile {
        egui_tiles::Tile::Pane(p) => Some(p),
        egui_tiles::Tile::Container(_) => None,
    }
}

/// A window built from several registries lays every one of their panes into
/// **one** tree, with each document's own centre pane sharing the centre
/// strip.
///
/// The property `ItemRegistry::default_tree` cannot show on its own, and the
/// one the shell depends on: `window_tree` refuses more than one
/// `Slot::Centre` inside a registry and expects them across registries, so a
/// second document's main surface has to land in the strip rather than
/// displace the first or be dropped.
///
/// Would catch: a `window_tree` that took just the first centre — which
/// compiles, draws the first document perfectly, and leaves the second's
/// canvas pane with no tile at all, so its region draws nothing.
#[test]
fn one_window_tree_holds_every_registry_s_panes_and_every_centre() {
    let a = good_registry();
    let b = ItemRegistry::new(vec![ItemSpec {
        id: NOTES,
        slot: Slot::Centre,
        toggle: None,
        make: || Box::new(ListItem::default()),
    }]);
    let mut placements = a.placements();
    placements.extend(b.placements());
    let tree = brightfield_workbench::window_tree(&placements);

    let mut panes: Vec<PaneKey> = tree.tiles.tiles().filter_map(tile_pane).copied().collect();
    panes.sort_unstable();
    let mut want: Vec<PaneKey> = placements.iter().map(|(id, _)| PaneKey::new(*id)).collect();
    want.sort_unstable();
    assert_eq!(
        panes, want,
        "a placement was dropped from the window's tree"
    );

    // Both centres are under one tab strip, and the first placed is active —
    // a window whose first sight is the second document's surface would be
    // opening on the thing the command line did not name.
    let tabbed = brightfield_workbench::workspace::tabbed_tiles_of(&tree);
    for centre in [LIST, NOTES] {
        let tile = brightfield_workbench::workspace::tile_of(&tree, PaneKey::new(centre))
            .expect("a centre pane has a tile");
        assert!(
            tabbed.contains(&tile),
            "{centre} is a centre pane and is not in the centre strip"
        );
    }
}

// ---------------------------------------------------------------------------
// Serialisation
// ---------------------------------------------------------------------------

/// Deliberately **one** test covering both the before-publish and
/// after-publish behaviour, rather than the two it obviously wants to be.
///
/// The published vocabulary is a process global (see `ItemId::publish` for
/// why: `Tree<PaneKey>`'s derived `Deserialize` has nowhere to thread a
/// context). Two tests in one binary would therefore share it, and which one
/// passed would depend on which ran first — green under the default
/// thread-per-test scheduler and red, or worse intermittently green, under
/// `--test-threads=1`. Ordering-dependent tests are a way of learning nothing
/// slowly, so the ordering is made explicit instead.
#[test]
fn an_item_id_deserialises_only_from_the_published_vocabulary() {
    // Before publishing, the vocabulary is empty and every id is unknown —
    // which is the safe direction: a layout read before boot finishes is
    // discarded rather than trusted.
    let json = r#""fixture-detail""#;
    assert!(serde_json::from_str::<PaneKey>(json).is_err());

    static VOCAB: &[ItemId] = &[LIST, DETAIL, NOTES];
    ItemId::publish(VOCAB);
    assert_eq!(
        ItemId::known(),
        VOCAB,
        "nothing else in this binary may have published"
    );

    let key = PaneKey::new(DETAIL);
    assert_eq!(serde_json::to_string(&key).unwrap(), json);
    assert_eq!(serde_json::from_str::<PaneKey>(json).unwrap(), key);

    // An id this build does not have is a load failure, not a pane nothing
    // can draw.
    let unknown = r#""a-pane-from-the-future""#;
    assert!(serde_json::from_str::<PaneKey>(unknown).is_err());
}

// ---------------------------------------------------------------------------
// Tone, and the reserved accent
// ---------------------------------------------------------------------------

/// The one-accent rule, as an assertion rather than a comment: the focus
/// border is what `Tone::Accent` resolves to, and no other tone may reach it.
#[test]
fn only_the_accent_tone_paints_the_interaction_colour() {
    for mode in [Mode::Light, Mode::Dark] {
        let accent = chrome::tone_colour(Tone::Accent, mode);
        let focus = chrome::colour(meridian_design::semantic(mode.is_dark()).borders.focus);
        assert_eq!(accent, focus);
        for tone in [Tone::Neutral, Tone::Good, Tone::Warning, Tone::Critical] {
            assert_ne!(
                chrome::tone_colour(tone, mode),
                accent,
                "{tone:?} reaches the interaction accent in {mode:?}"
            );
        }
    }
}
