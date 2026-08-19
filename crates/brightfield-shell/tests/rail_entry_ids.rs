//! Gate: no two panes the window can place declare the same
//! [`StatusEntry`](brightfield_workbench::StatusEntry) id.
//!
//! The id is a stable name and nothing dedups by it. `chrome::status_rail`
//! draws each entry it is handed, in order, so two entries sharing an id draw
//! twice — `chrome_rules.rs::two_entries_sharing_an_id_both_draw` is that
//! behaviour pinned. The window collects the status lines of the panes it has
//! placed rather than the focused one's, so two panes declaring one id put the
//! same line on the rail twice.
//!
//! There is a second, quieter failure: `Activity::of_entry` recognises a
//! pane's activity report by matching its id against the ids in
//! `Activity::ALL`, and the window filters those out of the rail because the
//! merged indicator says them. A new line colliding with one of those ids
//! would be filtered out rather than drawn — the test
//! `status_rail.rs::activity_reaches_the_rail_as_the_one_indicator` holds
//! that filtering.
//!
//! # One tree, one id space
//!
//! The workspace holds **one** tree over both documents: `startup` hands
//! `window_tree` the chart registry's placements and the protocol registry's
//! together, and `MeridianApp::apply` records that both documents' panes draw
//! in the same frame. So the ids are one namespace, and this gate compares
//! across the registries rather than within each — a chart pane and a protocol
//! pane colliding is a collision.
//!
//! It cost nothing to check that today, because the protocol panel declares no
//! status entry. That is exactly why it is checked now: the reason the
//! cross-registry case is currently safe is a fact about the panel's content,
//! not about the structure, and it stops being true the first time somebody
//! adds a line there.
//!
//! # The property is asserted by running the panes, not by reading them
//!
//! This file first tried to answer the question from source text: read the
//! string literals handed to a `StatusEntry` id and report a repeat. Review
//! found four ways a real duplicate reaches the rail while such a scan says
//! the tree is clean, and they are one shape — **a source-text scan cannot
//! see through indirection.** A second caller reached through
//! `use crate::rail::run_line as owns_it;` contains no registered call
//! fragment. One pane's `describe` calling another's, folding the returned
//! `Subject`'s entries into its own, adds no `StatusEntry` text to the tree.
//! Alias, delegation, re-export, trait dispatch, macro, closure: same
//! problem, different clothes, and patching two of them buys finding the next
//! two.
//!
//! So the gate constructs the panes and asks them. [`ItemSpec::make`] is a
//! `fn() -> Box<dyn Item<D>>` per registered pane, so a test can build every
//! pane the window can place, call the real `Item::subject`, and read the ids
//! that come back. Indirection is invisible to that, because it runs the code
//! instead of reading it. It is the same move this file already made for
//! `Activity::ALL`, whose ids are match arms rather than construction-site
//! literals: link the crate and ask.
//!
//! [`ItemSpec::make`]: brightfield_workbench::registry::ItemSpec::make
//!
//! # The document matrix crosses its dimensions
//!
//! `describe` is a function of the document, and it reads three independent
//! things: the recorded run state, the work in flight, and what the watcher
//! has seen. An earlier draft swept those one at a time — every `RunState`
//! with nothing in flight, then every `Activity` pinned to `RunState::Fresh`.
//! Review broke it with a second `run-state` declaration gated on
//! `RunState::Failed` **and** an engine query in flight: a combination no
//! fixture built, so fourteen tests passed over a live duplicate.
//!
//! [`chart_states`] now walks the **product**: six run states (absent, plus
//! each of `RunState::ALL`) by eight activity subsets (each subset of
//! `Activity::ALL`, since a log holds several at once) by four watcher states
//! (neither file moved, the spec, the data, both). 192 documents, which is a
//! small finite product rather than a sample, and it is affordable because
//! one composition is mutated through the whole matrix rather than 192 being
//! built.
//!
//! # What runtime does not reach, and what covers it
//!
//! **A branch that needs a live engine session.** `chart-navigation`,
//! `chart-navigation-scope` and `chart-predicate` need a refused gesture, a
//! declined rescope or a committed selection, none of which a headless
//! document has. [`RAIL_IDS`] marks those `Reach::Declared` with the reason,
//! and the gate asserts the observed set is *exactly* the set marked
//! observed — so a fixture that stops working reddens rather than quietly
//! narrowing the check.
//!
//! **Pane-local state.** `(spec.make)()` builds a fresh pane, so an id gated
//! on state the pane accumulates is out of the registry's reach.
//! [`pane_local_states`] covers the two that exist by building the pane
//! directly and driving it, which is why the spec editor's two ids are
//! observed rather than declared.
//!
//! **A surface that is not a registered pane.** The window composes two rail
//! lines itself. [`a_booted_window_draws_no_id_twice`] covers those by booting
//! the real window — over a chart document and over a protocol one — and
//! reading `MeridianApp::rail`, which is what actually drew.
//!
//! **An entry that reaches no `Subject`.** The dev gallery builds two
//! specimens and hands them straight to `chrome::status_rail` on its own
//! surface, so no pane declares them and no pane can collide with them there.
//!
//! # What is surveyed and left open
//!
//! Each round of review found a gap one step outside whatever the mechanism
//! enumerated, so here is the accounting for this one, including where it
//! stops.
//!
//! **What `ChartItem::describe` reads, and whether it is crossed.** The run
//! state, the activity log, the watcher and `ChartDoc::is_empty` are each a
//! dimension of the product above. `nav_notice`, `nav_scope_notice` and
//! `selection_sql` are not, and cannot be: each needs a live engine session
//! that a headless document does not have. Those are the three
//! `Reach::Declared` rows owned by `chart-canvas`, so the accounting closes —
//! an input that decides a chart rail line is either crossed or named.
//!
//! **Protocol document state is not crossed.** Two documents are driven,
//! empty and the offline fixture, with no product over their internals. No
//! protocol pane declares a status entry today and none reads `self`, so
//! there is nothing there to miss; a future protocol line gated on model
//! state would sit outside this matrix, and closing it would mean giving that
//! document the treatment the chart document gets.
//!
//! **The window is driven in four frames, not a product.** A *pane* colliding
//! with one of the window's own two ids is caught by the owner column in the
//! sweep above, which is the stronger net and does not wait for a frame to
//! reach it. What is left open is a third window-composed line added on some
//! window state none of the four frames reach.
//!
//! **Two registries could declare the same `ItemId`.** `ItemRegistry::new`
//! refuses a repeat inside one registry and nothing checks across them, which
//! now matters because there is one tree. That is a pane-identity question
//! rather than a rail-id one, so it is noted here and not guarded here.
//!
//! For those, the residual at the foot of this file reads the id literals it
//! can see in `crates/*/src` and requires each to be declared in
//! [`RAIL_IDS`]. That is a rot-check on the table, **not** the uniqueness
//! property: it is the half that catches a *new* id nobody registered, on a
//! branch nothing runs. It is blind to exactly the indirection described
//! above — which is why it no longer carries the property, and why the two
//! halves divide the way they do. An alias or a delegation creates no new
//! literal, so text cannot see it and runtime can; a literal on an
//! unreachable branch runs nowhere, so runtime cannot see it and text can.
//!
//! What neither reaches: a **second** declaration of an **already-declared**
//! id, on a branch neither the matrix nor a directly-driven pane runs. Named
//! here rather than implied.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::{chart_registry_with, ChartDoc};
use brightfield_shell::design::Mode;
use brightfield_shell::editor::{EditorPane, EDITOR, RELOAD_SPINNER_HONESTY_MS};
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::{
    load_protocol_offline, protocol_registry, ProtocolDoc, ProtocolModel,
};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::registry::ItemRegistry;
use brightfield_workbench::subject::RunState;
use brightfield_workbench::{
    Activity, ActivityLog, HideAffordance, Item, StatusEntry, StatusSide, Subject, Tone,
    HONESTY_LINE_MS,
};

const DASHBOARD: &str = "../../examples/dashboard.yaml";
const EDGAR: &str = "../../examples/protocol/edgar_gleif/arcform.yaml";

// ---------------------------------------------------------------------------
// The declared id space
// ---------------------------------------------------------------------------

/// How many surfaces may put an id on the rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    /// Exactly one surface, named here. A second declaration draws the line
    /// twice, which is the defect this gate exists for.
    One(&'static str),
    /// Any pane may report it, because the window does not rail these at all:
    /// `status_rail_ui` filters out every entry `Activity::of_entry`
    /// recognises and pushes one merged indicator instead, so two panes
    /// reporting the same work say it once.
    ///
    /// **This exemption is not granted by saying so.**
    /// [`a_merged_id_is_one_the_shell_really_merges`] hands each id here to
    /// `Activity::of_entry` and requires it to be recognised, and requires
    /// every [`Owner::One`] id to be *un*recognised — so an id cannot be moved
    /// into this class to silence a real duplicate.
    Merged,
}

/// Whether the fixtures in this file place an id, or why they do not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// A fixture places it, and the gate requires that it still does — an
    /// entry that stops being observed reddens, so the matrix cannot rot into
    /// a check that asks less than it says.
    Observed,
    /// No fixture places it, and this is why.
    Declared(&'static str),
}

/// The status rail's id space: every id a surface may put on the rail, the
/// one surface allowed to put it there, and whether this file reaches it.
///
/// Before this table nothing declared the id space at all — the ids were
/// string literals at their call sites and the uniqueness requirement lived
/// in a doc comment. The owner column is what turns "this id appeared twice"
/// into "this id appeared somewhere it does not belong", which is the report
/// a reader can act on.
///
/// A surface that is not a pane is named in angle brackets, so it cannot be
/// confused with an `ItemId`.
const RAIL_IDS: &[(&str, Owner, Reach)] = &[
    ("run-state", Owner::One("chart-canvas"), Reach::Observed),
    (
        "chart-navigation",
        Owner::One("chart-canvas"),
        Reach::Declared("needs a navigation gesture refused on a live session"),
    ),
    (
        "chart-navigation-scope",
        Owner::One("chart-canvas"),
        Reach::Declared("needs a plot held at an extent whose mark declined to rescope"),
    ),
    (
        "chart-predicate",
        Owner::One("chart-canvas"),
        Reach::Declared("needs a committed selection on a live session"),
    ),
    ("activity-engine-query", Owner::Merged, Reach::Observed),
    ("activity-protocol-run", Owner::Merged, Reach::Observed),
    ("activity-file-watch", Owner::Merged, Reach::Observed),
    ("watch-spec", Owner::One("chart-canvas"), Reach::Observed),
    ("watch-data", Owner::One("chart-canvas"), Reach::Observed),
    ("editor-saved", Owner::One("spec-editor"), Reach::Observed),
    ("editor-warning", Owner::One("spec-editor"), Reach::Observed),
    ("chart-idle", Owner::One("<window>"), Reach::Observed),
    ("activity", Owner::One("<window>"), Reach::Observed),
    (
        "gallery-status-rail-predicate",
        Owner::One("<dev gallery>"),
        Reach::Declared("a specimen handed to `chrome::status_rail`, never to a Subject"),
    ),
    (
        "gallery-status-rail-idle",
        Owner::One("<dev gallery>"),
        Reach::Declared("a specimen, as above"),
    ),
];

/// What the table says about `id`, if it knows it.
fn declared(id: &str) -> Option<Owner> {
    RAIL_IDS
        .iter()
        .find(|(known, _, _)| *known == id)
        .map(|(_, owner, _)| *owner)
}

// ---------------------------------------------------------------------------
// Running the panes
// ---------------------------------------------------------------------------

/// One rail line a surface placed: which surface, which id.
type Placed = (String, &'static str);

/// The rail lines the registered panes of `registry` place over `doc`.
///
/// Registered rather than currently placed, which is the conservative
/// direction: `window_tree` gives each placement a tile, and a pane's toggle
/// verb hides a pane the registry already holds rather than introducing one,
/// so the registry is a superset of any arrangement the user can reach.
fn placed<D: ?Sized>(registry: &ItemRegistry<D>, doc: &D) -> Vec<Placed> {
    let mut out = Vec::new();
    for spec in registry.specs() {
        let pane: Box<dyn Item<D>> = (spec.make)();
        for entry in pane.subject(doc).status {
            out.push((spec.id.as_str().to_string(), entry.id));
        }
    }
    out
}

/// Everything wrong with what one window state placed.
///
/// `lines` is the union across both registries and the editor's own state,
/// because the window holds one tree and every pane in it draws in the same
/// frame.
fn complaints(state: &str, lines: &[Placed]) -> Vec<String> {
    let mut out = Vec::new();
    let mut first: BTreeMap<&str, &str> = BTreeMap::new();
    for (owner, id) in lines {
        match declared(id) {
            None => out.push(format!(
                "{state}: {owner} declares `{id}`, which RAIL_IDS does not know — add it \
                 with the surface that owns it"
            )),
            // A merged id is a report, not a rail line: the window filters
            // every one of them out and says the work once through the
            // indicator, so a second reporter is the design.
            Some(Owner::Merged) => {}
            Some(Owner::One(declared_owner)) => {
                if let Some(prev) = first.insert(id, owner) {
                    out.push(format!(
                        "{state}: `{id}` is declared by {prev} and by {owner} — the rail \
                         draws each entry it is handed, so that line appears twice"
                    ));
                }
                if declared_owner != owner {
                    out.push(format!(
                        "{state}: {owner} declares `{id}`, which RAIL_IDS gives to \
                         {declared_owner}"
                    ));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The document matrix
// ---------------------------------------------------------------------------

/// A scratch directory unique to this test binary.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("bf-rail-ids-{}", std::process::id()))
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// A spec and a data file for the watcher to hold an opinion about.
struct Watched {
    spec: PathBuf,
    data: PathBuf,
}

impl Watched {
    fn new() -> Self {
        let dir = scratch("watched");
        let spec = dir.join("spec.yaml");
        let data = dir.join("rows.csv");
        fs::write(&spec, include_str!("../../../examples/dashboard.yaml")).expect("spec");
        fs::write(&data, "a,b\n1,2\n").expect("data");
        Self { spec, data }
    }

    /// Re-baseline the watcher, then move whichever files this state says have
    /// changed and let it see them.
    ///
    /// `nonce` keeps each move to a timestamp no baseline has recorded, which
    /// is what makes the change detectable without sleeping through the
    /// filesystem's mtime granularity.
    fn apply(&self, doc: &mut ChartDoc, spec_moved: bool, data_moved: bool, nonce: u64) {
        doc.watch
            .watch(Some(self.spec.clone()), vec![self.data.clone()]);
        let mut moved = false;
        for (path, wanted) in [(&self.spec, spec_moved), (&self.data, data_moved)] {
            if !wanted {
                continue;
            }
            let f = fs::File::options().write(true).open(path).expect("reopen");
            f.set_modified(SystemTime::now() - Duration::from_secs(nonce + 1))
                .expect("move the mtime somewhere the baseline is not");
            moved = true;
        }
        if moved {
            assert!(
                doc.watch.poll_now(),
                "the watcher saw no change, so this state reaches no file notice"
            );
        }
    }
}

/// One log per subset of `Activity::ALL` — the power set, since a log holds
/// several kinds at once — each aged past the honesty line so it owes its
/// entries the moment it is read.
///
/// Aged once and swapped in rather than begun per state: `ActivityLog::entries`
/// withholds work younger than [`HONESTY_LINE_MS`], so beginning it inside the
/// matrix would mean a sleep per document.
fn aged_activity_logs() -> Vec<(String, ActivityLog)> {
    let mut out = Vec::new();
    for mask in 0..(1u8 << Activity::ALL.len()) {
        let mut log = ActivityLog::new();
        let mut names = Vec::new();
        for (bit, kind) in Activity::ALL.into_iter().enumerate() {
            if mask & (1 << bit) != 0 {
                log.begin(kind);
                names.push(format!("{kind:?}"));
            }
        }
        let name = if names.is_empty() {
            "nothing in flight".to_string()
        } else {
            format!("{} in flight", names.join("+"))
        };
        out.push((name, log));
    }
    std::thread::sleep(Duration::from_millis(
        u64::try_from(HONESTY_LINE_MS).expect("small") + 40,
    ));
    out
}

/// Drive `body` over the product of emptiness, run state, work in flight and
/// watcher state — 384 documents, two compositions mutated through all of them.
///
/// Emptiness is a crossed dimension rather than a point beside the product.
/// An earlier draft evaluated `ChartDoc::empty()` once, with no run state, no
/// work in flight and no watcher notice, so a line gated on an empty document
/// *and* anything else was unreachable. That is the same shape as the gap
/// review found between run state and activity, one dimension over.
fn chart_states(mut body: impl FnMut(&str, &ChartDoc)) {
    let files = Watched::new();
    let mut logs = aged_activity_logs();

    let composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    let mut loaded = ChartDoc::headless(composed);
    loaded.spec_path = Some(DASHBOARD.into());
    let mut bases = [
        ("an empty document", ChartDoc::empty()),
        ("the dashboard", loaded),
    ];

    let mut nonce = 0u64;
    let run_states: Vec<Option<RunState>> = std::iter::once(None)
        .chain(RunState::ALL.into_iter().map(Some))
        .collect();
    for (base, doc) in &mut bases {
        for run in &run_states {
            doc.composed.run_state = *run;
            for (work, log) in &mut logs {
                std::mem::swap(&mut doc.activity, log);
                for (spec_moved, data_moved) in
                    [(false, false), (true, false), (false, true), (true, true)]
                {
                    nonce += 1;
                    files.apply(doc, spec_moved, data_moved, nonce);
                    let name = format!(
                        "{base} recorded {run:?}, {work}, watcher spec={spec_moved} \
                         data={data_moved}"
                    );
                    body(&name, doc);
                }
                std::mem::swap(&mut doc.activity, log);
            }
        }
    }
}

/// The Protocol documents. The panel declares no status entry today; driving
/// it anyway is what makes the first one somebody adds arrive through this
/// gate rather than past it.
fn protocol_documents() -> Vec<(String, ProtocolDoc)> {
    let inputs = load_protocol_offline(EDGAR).expect("load the offline protocol fixture");
    vec![
        ("an empty document".to_string(), ProtocolDoc::empty()),
        (
            "the offline protocol fixture".to_string(),
            ProtocolDoc::headless(ProtocolModel::new(inputs, Flow::Vertical)),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Panes the registry cannot reach, driven directly
// ---------------------------------------------------------------------------

/// The rail lines the spec editor places once it has accumulated state of its
/// own, one entry per state it can be in.
///
/// `(spec.make)()` hands back a fresh pane, so an id gated on what a pane has
/// been *given* is invisible to the registry sweep. `EditorPane::describe` is
/// the only `describe` in this workspace that reads `self` at all — surveyed,
/// and held by [`only_the_editor_reads_its_own_state_in_describe`] — so
/// driving it closes the whole pane-local class rather than one instance of
/// it.
///
/// The third state is the one review found. A reload the editor cannot
/// complete leaves `reload_pending_since` set, and past the honesty line the
/// pane reports `Activity::FileWatch` — the same id `ChartItem` reports from
/// the *document's* activity log. Two panes, one id, from independent
/// triggers. The read is made to fail by replacing the file with a directory
/// of the same name, which is deterministic and needs no privilege games: the
/// mtime moves so the poll proceeds, and `read_to_string` cannot succeed on a
/// directory for any user.
fn editor_states() -> Vec<(String, Vec<Placed>)> {
    let dir = scratch("editor");
    let doc = ChartDoc::empty();
    let ids = |subject: Subject| -> Vec<Placed> {
        subject
            .status
            .into_iter()
            .map(|entry| (EDITOR.as_str().to_string(), entry.id))
            .collect()
    };

    let saved = dir.join("saved.yaml");
    fs::write(&saved, "plot: {}\n").expect("seed");
    let mut pane = EditorPane::new();
    pane.open_file(&saved);
    pane.buffer_mut()
        .expect("a seeded buffer")
        .push_str("a: 1\n");
    pane.note_buffer_edited();
    pane.save_now();
    let after_save = ids(pane.subject(&doc));

    let conflict = dir.join("conflict.yaml");
    fs::write(&conflict, "seeded: 1\n").expect("seed");
    let mut pane = EditorPane::new();
    pane.open_file(&conflict);
    pane.buffer_mut()
        .expect("a seeded buffer")
        .push_str("mine: 2\n");
    fs::write(&conflict, "external: 3\n").expect("an external write");
    pane.save_now();
    let after_conflict = ids(pane.subject(&doc));

    let stuck = dir.join("stuck.yaml");
    fs::write(&stuck, "seeded: 1\n").expect("seed");
    let mut pane = EditorPane::new();
    pane.open_file(&stuck);
    fs::remove_file(&stuck).expect("remove the file");
    fs::create_dir(&stuck).expect("put a directory where the file was");
    pane.poll_disk_now();
    std::thread::sleep(Duration::from_millis(
        u64::try_from(RELOAD_SPINNER_HONESTY_MS).expect("small") + 40,
    ));
    let mid_reload = ids(pane.subject(&doc));
    assert!(
        mid_reload
            .iter()
            .any(|(_, id)| *id == Activity::FileWatch.id()),
        "the editor was meant to be mid-reload and reporting file-watch work; \
         it placed {mid_reload:?}"
    );

    vec![
        ("the spec editor just after a save".to_string(), after_save),
        (
            "the spec editor after the two-writer guard refused a save".to_string(),
            after_conflict,
        ),
        (
            "the spec editor with a reload it cannot complete".to_string(),
            mid_reload,
        ),
    ]
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

#[test]
fn no_two_panes_declare_the_same_rail_id() {
    let protocol = protocol_documents();
    let protocol_registry = protocol_registry();
    // The protocol panes' lines do not vary with the chart document, so they
    // are read once and unioned into each window state below.
    let protocol_lines: Vec<(String, Vec<Placed>)> = protocol
        .iter()
        .map(|(name, doc)| (name.clone(), placed(&protocol_registry, doc)))
        .collect();

    // Nor do the editor's, which read `self` and not the document — so they
    // are driven once too, and substituted for the fresh editor the registry
    // sweep builds. Substituted rather than added: the fresh pane's lines come
    // out first, so a pane appears once per state whichever state it is in.
    let editor = editor_states();

    let registries = [
        ("", chart_registry_with(false)),
        (" with the dev gallery", chart_registry_with(true)),
    ];

    let mut found = Vec::new();
    let mut observed: BTreeSet<&'static str> = BTreeSet::new();
    let mut states = 0usize;

    chart_states(|name, doc| {
        for (gallery, registry) in &registries {
            let chart_lines = placed(registry, doc);
            observed.extend(chart_lines.iter().map(|(_, id)| *id));
            for (protocol_name, protocol_lines) in &protocol_lines {
                // One tree, so both documents' panes are compared together.
                let mut base = chart_lines.clone();
                base.extend(protocol_lines.iter().cloned());
                observed.extend(protocol_lines.iter().map(|(_, id)| *id));
                states += 1;
                found.extend(complaints(
                    &format!("{name}{gallery}, protocol showing {protocol_name}"),
                    &base,
                ));

                // And the same window with the editor in each state it can
                // accumulate. A pane's own state is a dimension of the window
                // like the document's, so it is crossed rather than checked
                // beside it: `activity-file-watch` reaches the rail from the
                // editor's own pending reload and from the document's activity
                // log, and only the union of the two shows the collision.
                for (editor_name, editor_lines) in &editor {
                    let mut lines: Vec<Placed> = base
                        .iter()
                        .filter(|(owner, _)| owner != EDITOR.as_str())
                        .cloned()
                        .collect();
                    lines.extend(editor_lines.iter().cloned());
                    observed.extend(editor_lines.iter().map(|(_, id)| *id));
                    states += 1;
                    found.extend(complaints(
                        &format!(
                            "{name}{gallery}, protocol showing {protocol_name}, \
                             {editor_name}"
                        ),
                        &lines,
                    ));
                }
            }
        }
    });

    // The window's own two lines, from the windows that actually drew them.
    observed.extend(window_drew().into_iter().flat_map(|(_, ids)| ids));

    assert!(
        found.is_empty(),
        "status-rail id violations ({}):\n{}",
        found.len(),
        found.join("\n")
    );

    // The matrix is a product, not a sample. A draft that swept each dimension
    // singly missed a duplicate gated on `RunState::Failed` AND an engine query
    // in flight, so the shape is asserted: emptiness by run state by activity
    // subset by watcher state, each crossed with both gallery arrangements,
    // both protocol documents, and the editor's own states.
    let bases = 2; // an empty document and the dashboard
    let run_states = 1 + RunState::ALL.len(); // absent, plus each recorded state
    let activity_subsets = 1 << Activity::ALL.len(); // the power set
    let watcher_states = 4; // neither file moved, the spec, the data, both
    let arrangements = 2; // without and with the dev gallery
    let protocol_documents = 2;
    let editor_variants = 1 + editor.len(); // the fresh pane, plus each state
    assert_eq!(
        states,
        bases
            * run_states
            * activity_subsets
            * watcher_states
            * arrangements
            * protocol_documents
            * editor_variants,
        "the matrix no longer crosses its dimensions"
    );

    // And it still reaches what the table says it reaches. Without this the
    // check narrows silently: a fixture that stopped placing its entry would
    // leave the assertion above green over less and less.
    let want: BTreeSet<&str> = RAIL_IDS
        .iter()
        .filter(|(_, _, reach)| *reach == Reach::Observed)
        .map(|(id, _, _)| *id)
        .collect();
    let got: BTreeSet<&str> = observed.iter().copied().collect();
    let missing: Vec<&&str> = want.difference(&got).collect();
    let extra: Vec<&&str> = got.difference(&want).collect();
    assert!(
        missing.is_empty(),
        "RAIL_IDS marks these Observed but no fixture placed them, so the gate \
         is checking less than it claims: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "these ids were placed but RAIL_IDS calls them unreachable — mark them \
         Observed: {extra:?}"
    );
}

/// What the rail drew, frame by frame, in two booted windows — one over a
/// chart document, one over a protocol one. This is where the lines the window
/// composes itself, the idle line and the merged activity indicator, can
/// collide with a pane's.
///
/// Per frame, not accumulated: the same id drawn in two different frames is
/// the rail doing its job, and only one frame carrying it twice is a defect.
fn window_drew() -> Vec<(String, Vec<&'static str>)> {
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0));
    let frame = |app: &mut MeridianApp| {
        let raw = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| app.draw(ui));
    };

    let mut drew = Vec::new();

    let composed = compose_spec(DASHBOARD).expect("compose examples/dashboard.yaml");
    let mut boot = Boot::charts(composed);
    boot.spec_path = Some(DASHBOARD.into());
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    frame(&mut app);
    frame(&mut app);
    drew.push((
        "a chart window at rest".to_string(),
        app.rail().drawn.clone(),
    ));

    // A run state recorded, which is the branch the chart pane rails.
    app.chart_doc_mut().composed.run_state = Some(RunState::StaleUpstream);
    frame(&mut app);
    drew.push((
        "a chart window with a run state recorded".to_string(),
        app.rail().drawn.clone(),
    ));

    // Work in flight, which is what summons the indicator.
    app.chart_doc_mut().activity.begin(Activity::EngineQuery);
    std::thread::sleep(Duration::from_millis(
        u64::try_from(HONESTY_LINE_MS).expect("small") + 40,
    ));
    frame(&mut app);
    drew.push((
        "a chart window with a run state and an engine query in flight".to_string(),
        app.rail().drawn.clone(),
    ));

    // And the same window booted on the protocol document, because the one
    // tree holds both documents' panes and the rail is theirs together.
    let inputs = load_protocol_offline(EDGAR).expect("load the offline protocol fixture");
    let boot = Boot::protocol(inputs, Flow::Vertical, None);
    let mut app = MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light);
    frame(&mut app);
    frame(&mut app);
    drew.push((
        "a protocol window at rest".to_string(),
        app.rail().drawn.clone(),
    ));

    drew
}

#[test]
fn a_booted_window_draws_no_id_twice() {
    let frames = window_drew();
    assert!(
        frames.iter().any(|(_, ids)| !ids.is_empty()),
        "no frame drew a rail at all, so this proves nothing"
    );
    for (state, ids) in &frames {
        let mut seen = BTreeSet::new();
        let repeated: Vec<&&str> = ids.iter().filter(|id| !seen.insert(**id)).collect();
        assert!(
            repeated.is_empty(),
            "{state} drew {repeated:?} more than once; it drew {ids:?}"
        );
        for id in ids {
            assert!(
                declared(id).is_some(),
                "{state} drew `{id}`, which RAIL_IDS does not know; it drew {ids:?}"
            );
        }
    }
}

#[test]
fn the_declared_id_space_names_each_id_once() {
    let mut seen = BTreeSet::new();
    for (id, owner, reach) in RAIL_IDS {
        assert!(
            seen.insert(*id),
            "RAIL_IDS names `{id}` twice, so it cannot say who owns it"
        );
        if let Owner::One(who) = owner {
            assert!(!who.trim().is_empty(), "`{id}` has no owner");
        }
        if let Reach::Declared(why) = reach {
            assert!(
                !why.trim().is_empty(),
                "`{id}` is out of the fixtures' reach and does not say why"
            );
        }
    }
}

/// `Owner::Merged` is the one exemption from the uniqueness rule, so it is
/// granted by the shell's own recognition rather than by this table saying so.
///
/// `status_rail_ui` drops every entry `Activity::of_entry` recognises and
/// pushes one merged indicator instead. An id that function does not know is
/// railed as written, so calling it merged would silence a real duplicate;
/// an id it does know cannot collide, so calling it owned would report one
/// that cannot happen. Both directions are checked, which is what stops the
/// exemption being reachable by editing a string.
#[test]
fn a_merged_id_is_one_the_shell_really_merges() {
    for (id, owner, _) in RAIL_IDS {
        let entry = StatusEntry {
            id,
            side: StatusSide::Trailing,
            text: String::new(),
            tone: Tone::Neutral,
            hide: HideAffordance::WithRail,
        };
        let merged_by_the_shell = Activity::of_entry(&entry).is_some();
        match owner {
            Owner::Merged => assert!(
                merged_by_the_shell,
                "RAIL_IDS calls `{id}` merged, but `Activity::of_entry` does not know it, \
                 so the window rails it as written and a second declaration would draw twice"
            ),
            Owner::One(who) => assert!(
                !merged_by_the_shell,
                "RAIL_IDS gives `{id}` to {who}, but the window merges it into the one \
                 indicator, so a second reporter is the design rather than a defect"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// The residual: id literals the runtime matrix never runs
// ---------------------------------------------------------------------------
//
// Everything below is a rot-check on RAIL_IDS, not the uniqueness property.
// It reads the id literals it can see in `crates/*/src` and requires each to
// be declared. Its job is the one thing running the panes cannot do: notice a
// NEW id added on a branch no fixture reaches.
//
// It is blind to indirection, by construction and on purpose. An alias, a
// delegation, a re-export or a macro creates no new literal, so there is
// nothing here for it to see — and nothing it needs to see, because those
// reuse an id that already exists and the runtime half catches the reuse. The
// division is deliberate: this half is for literals that never run, the
// runtime half is for code that runs however it was reached.

/// A site whose id the scan cannot follow, and why that is safe:
/// `(path suffix, expression as written, why)`.
type Allow = (&'static str, &'static str, &'static str);

/// How many unreadable sites may be excused.
const ALLOW_CAP: usize = 5;

/// The sites whose id expression the scan cannot follow.
const UNREADABLE_ALLOW: &[Allow] = &[
    (
        "crates/brightfield-workbench/src/subject.rs",
        "id",
        "RunState::status_entry forwards the id its caller passed, so the \
         literal is at the call and the scan reads it there.",
    ),
    (
        "crates/brightfield-workbench/src/activity.rs",
        "self.id()",
        "Activity::id is a match over Activity::ALL. Those ids reach RAIL_IDS \
         through the runtime half, which links the crate rather than reading \
         the arms.",
    ),
];

/// How far past a `StatusEntry {` the `id` field may sit before the scan gives
/// up and reports.
const FIELD_SCAN: usize = 12;

/// The fewest id literals the workspace may hold before the scan is assumed to
/// have stopped reading rather than the rail to have gone quiet.
const SITE_FLOOR: usize = 10;

/// One place production code hands a `StatusEntry` an id.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Site {
    /// Workspace-relative path of the file the site is in.
    rel: String,
    /// Workspace-relative path and 1-based line.
    at: String,
    /// The id expression exactly as written.
    expr: String,
}

/// If `lines[i]` opens a `#[cfg(test)]` **module** in column 0, the index just
/// past that module's closing brace.
///
/// `None` when the attribute sits on a bare item — a `const`, a `use`, a `fn`
/// — because such an item opens no module and closes no brace of its own. An
/// earlier draft skipped to the next column-0 `}` regardless, which on a bare
/// item ran past every declaration up to whatever brace came next and took
/// them with it. `brightfield-bench/src/main.rs` carries four consecutive bare
/// `#[cfg(test)] const` items, so the shape is in this tree;
/// `a_bare_cfg_test_item_does_not_hide_what_follows_it` holds it.
fn test_module_end(lines: &[&str], i: usize) -> Option<usize> {
    if lines[i] != "#[cfg(test)]" {
        return None;
    }
    let mut j = i + 1;
    while j < lines.len() {
        let trimmed = lines[j].trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("#[") {
            j += 1;
            continue;
        }
        break;
    }
    let decl = lines.get(j)?.trim_start();
    // `mod x;` names a file and opens no body, so the next brace is not its.
    let opens_a_body = decl.ends_with('{');
    if !(opens_a_body && (decl.starts_with("mod ") || decl.starts_with("pub mod "))) {
        return None;
    }
    let mut k = j;
    while k < lines.len() && lines[k] != "}" {
        k += 1;
    }
    Some(k + 1)
}

/// The argument of a `…status_entry(<expr>)` call on `line`.
///
/// The needle is anchored at the front: a name that merely *ends* in
/// `status_entry` is a different function. `idle_status_entry` and
/// `draw_status_entry` are both in this workspace, and an unanchored needle
/// read their first argument as an id.
fn call_argument(line: &str) -> Option<String> {
    if line.contains("fn status_entry(") {
        return None;
    }
    let idx = line
        .match_indices("status_entry(")
        .find(|(at, _)| {
            line[..*at]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
        })
        .map(|(at, _)| at)?;
    let rest = &line[idx + "status_entry(".len()..];
    let close = rest.find(')')?;
    let arg = rest[..close].trim();
    (!arg.is_empty()).then(|| arg.to_string())
}

/// The `id` field belonging to the `StatusEntry {` that opens at `open`.
fn field_site(rel: &str, lines: &[&str], open: usize) -> Site {
    let end = (open + 1 + FIELD_SCAN).min(lines.len());
    for (n, line) in lines.iter().enumerate().take(end).skip(open + 1) {
        let trimmed = line.trim();
        let expr = if trimmed == "id," {
            // Field shorthand: the id is whatever local named `id` holds.
            "id".to_string()
        } else if let Some(rest) = trimmed.strip_prefix("id:") {
            rest.trim().trim_end_matches(',').trim().to_string()
        } else {
            continue;
        };
        return Site {
            rel: rel.to_string(),
            at: format!("{rel}:{}", n + 1),
            expr,
        };
    }
    Site {
        rel: rel.to_string(),
        at: format!("{rel}:{}", open + 1),
        expr: format!("<no id field within {FIELD_SCAN} lines of the construction>"),
    }
}

/// Every id-supplying site in one file's text.
fn sites(rel: &str, text: &str) -> Vec<Site> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(past) = test_module_end(&lines, i) {
            i = past;
            continue;
        }
        let line = lines[i];
        if line.trim_start().starts_with("//") {
            i += 1;
            continue;
        }
        // A return type opens a function body, not a construction, and the
        // construction it returns is on a line of its own. The test for it is
        // an arrow anywhere to the left rather than the literal
        // `-> StatusEntry {`, because the type is spelled in full at some
        // sites — `-> brightfield_workbench::subject::StatusEntry {` — and the
        // narrower form read those signatures as constructions naming no id.
        // A construction has no arrow before it:
        // `subject.with_status(StatusEntry {` and `.map(|e| StatusEntry {`
        // both pass. Held by
        // `a_fully_qualified_return_type_is_not_a_construction`.
        if let Some(idx) = line.find("StatusEntry {") {
            if !line[..idx].contains("->") && !line.contains("struct StatusEntry") {
                out.push(field_site(rel, &lines, i));
            }
        }
        if let Some(expr) = call_argument(line) {
            out.push(Site {
                rel: rel.to_string(),
                at: format!("{rel}:{}", i + 1),
                expr,
            });
        }
        i += 1;
    }
    out
}

/// The name and value of a `const NAME: &str = "…";` on `line`.
fn constant_decl(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let trimmed = trimmed
        .strip_prefix("pub(crate) ")
        .or_else(|| trimmed.strip_prefix("pub "))
        .unwrap_or(trimmed);
    let rest = trimmed.strip_prefix("const ")?;
    let (name, rest) = rest.split_once(':')?;
    let (ty, value) = rest.split_once('=')?;
    if !matches!(ty.trim(), "&str" | "&'static str") {
        return None;
    }
    let inner = value.trim().strip_prefix('"')?;
    let end = inner.find('"')?;
    if !inner[end + 1..].trim().starts_with(';') {
        return None;
    }
    Some((name.trim().to_string(), inner[..end].to_string()))
}

/// Every readable string constant, as `name -> (file -> value)`.
fn constants(files: &[(String, String)]) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (rel, text) in files {
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if let Some(past) = test_module_end(&lines, i) {
                i = past;
                continue;
            }
            if let Some((name, value)) = constant_decl(lines[i]) {
                out.entry(name).or_default().insert(rel.clone(), value);
            }
            i += 1;
        }
    }
    out
}

/// What an id expression turned out to be.
enum Read {
    /// The id itself.
    Id(String),
    /// Why the scan could not follow the expression.
    Unreadable(String),
}

/// Follow one site's expression to the id it supplies.
fn read_id(site: &Site, consts: &BTreeMap<String, BTreeMap<String, String>>) -> Read {
    if let Some(inner) = site.expr.strip_prefix('"') {
        return match inner.strip_suffix('"') {
            Some(body) if !body.contains('"') => Read::Id(body.to_string()),
            _ => Read::Unreadable(format!("not a plain string literal: `{}`", site.expr)),
        };
    }
    let name = site.expr.rsplit("::").next().unwrap_or(&site.expr);
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Read::Unreadable(format!(
            "neither a string literal nor a constant name: `{}`",
            site.expr
        ));
    }
    let Some(declared) = consts.get(name) else {
        return Read::Unreadable(format!(
            "`{}` is not a string constant this scan read",
            site.expr
        ));
    };
    if let Some(value) = declared.get(&site.rel) {
        return Read::Id(value.clone());
    }
    let distinct: BTreeSet<&String> = declared.values().collect();
    match distinct.len() {
        1 => Read::Id((*distinct.iter().next().expect("one value")).clone()),
        _ => Read::Unreadable(format!(
            "`{}` names {} different constants, so this scan cannot say which \
             one the site means",
            site.expr,
            distinct.len()
        )),
    }
}

/// Every id literal in `files` that RAIL_IDS does not declare, plus every site
/// whose expression the scan could not follow and no entry in `allow` excuses.
fn undeclared(files: &[(String, String)], allow: &[Allow]) -> Vec<String> {
    let consts = constants(files);
    let mut out = Vec::new();
    let mut excuses_used: BTreeSet<usize> = BTreeSet::new();
    for (rel, text) in files {
        for site in sites(rel, text) {
            match read_id(&site, &consts) {
                Read::Id(id) => {
                    if declared(&id).is_none() {
                        out.push(format!(
                            "{}: `{id}` is handed to a StatusEntry and RAIL_IDS does not \
                             know it — declare it with the surface that owns it",
                            site.at
                        ));
                    }
                }
                Read::Unreadable(why) => {
                    if let Some(index) = allow
                        .iter()
                        .position(|(s, e, _)| site.rel.ends_with(s) && site.expr == *e)
                    {
                        excuses_used.insert(index);
                        continue;
                    }
                    out.push(format!("{}: unreadable status-entry id — {why}", site.at));
                }
            }
        }
    }
    for (index, (suffix, expr, _)) in allow.iter().enumerate() {
        if !excuses_used.contains(&index) {
            out.push(format!(
                "UNREADABLE_ALLOW entry ({suffix}, {expr}) excused nothing — the site \
                 it was written for is gone, so the entry is too"
            ));
        }
    }
    out.sort();
    out
}

fn rs_files(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.map(|e| e.expect("dir entry").path()).collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .expect("a read entry has a name")
            .to_string_lossy()
            .to_string();
        if path.is_dir() {
            rs_files(&path, &format!("{prefix}/{name}"), out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).expect("source file is UTF-8");
            out.push((format!("{prefix}/{name}"), text));
        }
    }
}

/// Every production source file in the workspace, as `(relative path, text)`.
fn production_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("this crate sits at <workspace>/crates/<name>")
        .to_path_buf();
    let mut out = Vec::new();
    let mut crates: Vec<PathBuf> = fs::read_dir(root.join("crates"))
        .expect("crates/ is readable")
        .map(|e| e.expect("dir entry").path())
        .collect();
    crates.sort();
    for dir in crates {
        let src = dir.join("src");
        if !src.is_dir() {
            continue;
        }
        let name = dir
            .file_name()
            .expect("a crate directory has a name")
            .to_string_lossy()
            .to_string();
        rs_files(&src, &format!("crates/{name}/src"), &mut out);
    }
    out
}

#[test]
fn every_id_literal_in_the_workspace_is_declared() {
    let files = production_sources();
    assert!(
        files.len() > 50,
        "the scan found {} source files, which is too few to have walked the workspace",
        files.len()
    );
    let all: Vec<Site> = files
        .iter()
        .flat_map(|(rel, text)| sites(rel, text))
        .collect();
    assert!(
        all.len() >= SITE_FLOOR,
        "the scan found {} id sites in the workspace, under the floor of \
         {SITE_FLOOR} — its needles have stopped matching",
        all.len()
    );

    let found = undeclared(&files, UNREADABLE_ALLOW);
    assert!(
        found.is_empty(),
        "undeclared status-rail ids ({}):\n{}",
        found.len(),
        found.join("\n")
    );
}

/// The files holding an `Item::describe` that decides a **status line** from
/// state of the pane's own, rather than from the document's.
///
/// Such a pane is invisible to the registry sweep, which builds it fresh
/// through `(spec.make)()`. There is one, and `editor_states` drives it.
const SELF_READING_DESCRIBES: &[&str] = &["crates/brightfield-shell/src/editor.rs"];

/// The pane-local class is closed by driving one pane, so this is the check
/// that the class still has one member.
///
/// A `describe` that reads `self` *and* declares a status line can place a
/// rail line the document cannot summon, and `(spec.make)()` hands back a pane
/// with none of that state — so a new one would be a branch the matrix cannot
/// reach and nothing would say so. Surveyed when `editor_states` was written:
/// ten `Item` implementations in this workspace, and `EditorPane` is the one
/// whose `describe` reads `self` at all. `ModuleHost::describe` reads `self`
/// for its title and icon and declares no status line, which is why the check
/// asks for both.
///
/// Line-based, and blind to the same indirection the rest of the residual is:
/// a `describe` that delegates to a method which reads `self` is not seen
/// here. It is a rot-check on the survey, not a proof.
#[test]
fn only_the_editor_reads_its_own_state_in_describe() {
    let mut reading: Vec<String> = Vec::new();
    for (rel, text) in production_sources() {
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if let Some(past) = test_module_end(&lines, i) {
                i = past;
                continue;
            }
            let indent = lines[i].len() - lines[i].trim_start().len();
            // `fn describe(...) -> Subject;` in the trait declares no body,
            // and running past it swept in the default `subject()` glue below.
            let opens_a_body = lines[i].trim_end().ends_with('{');
            if opens_a_body && lines[i].trim_start().starts_with("fn describe(") {
                let close = format!("{}}}", " ".repeat(indent));
                let end = (i + 1..lines.len())
                    .find(|j| lines[*j] == close)
                    .unwrap_or(lines.len());
                let body = lines[i + 1..end].join("\n");
                // Both, not either: `ModuleHost::describe` reads `self` for
                // its title and icon and declares no status line at all, so it
                // places nothing the registry sweep could miss.
                if body.contains("self.") && body.contains("with_status") {
                    reading.push(rel.clone());
                }
            }
            i += 1;
        }
    }
    reading.sort();
    reading.dedup();
    let known: Vec<String> = SELF_READING_DESCRIBES
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert_eq!(
        reading, known,
        "the set of panes whose `describe` reads their own state has changed. A pane \
         like that can place a rail line no document summons, and the registry sweep \
         builds it fresh — so drive it in `editor_states` and add it here, or say in \
         RAIL_IDS why its ids are out of reach"
    );
}

#[test]
fn the_allowlist_stays_small_and_justified() {
    assert!(
        UNREADABLE_ALLOW.len() <= ALLOW_CAP,
        "UNREADABLE_ALLOW holds {} entries, past the cap of {ALLOW_CAP}",
        UNREADABLE_ALLOW.len()
    );
    for (suffix, expr, why) in UNREADABLE_ALLOW {
        assert!(
            !why.trim().is_empty(),
            "UNREADABLE_ALLOW entry ({suffix}, {expr}) carries no reason"
        );
    }
}

// ---------------------------------------------------------------------------
// The residual proves it can still fail, on synthetic source
// ---------------------------------------------------------------------------

fn synthetic(files: &[(&str, &str)]) -> Vec<String> {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(rel, text)| ((*rel).to_string(), (*text).to_string()))
        .collect();
    undeclared(&owned, &[])
}

#[test]
fn a_bare_cfg_test_item_does_not_hide_what_follows_it() {
    let bare = r#"
#[cfg(test)]
const FIXTURE: &str = "a fixture compiled in for tests";

impl Item<ChartDoc> for ChartItem {
    fn describe(&self, doc: &ChartDoc) -> Subject {
        Subject::new().with_status(StatusEntry {
            id: "not-a-declared-id",
            side: StatusSide::Trailing,
        })
    }
}
"#;
    let found = synthetic(&[("crates/a/src/chart.rs", bare)]);
    assert_eq!(
        found.len(),
        1,
        "a bare `#[cfg(test)]` item hid the declaration after it; got {found:?}"
    );
    assert!(found[0].contains("`not-a-declared-id`"), "{}", found[0]);
}

#[test]
fn a_fixture_inside_a_test_module_is_not_a_declaration() {
    let with_tests = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn a_fixture() {
        let _ = StatusEntry {
            id: "not-a-declared-id",
            side: StatusSide::Trailing,
        };
    }
}
"#;
    assert!(
        synthetic(&[("crates/a/src/chart.rs", with_tests)]).is_empty(),
        "a fixture in a test module was read as a declaration"
    );
}

#[test]
fn a_declared_id_is_not_a_finding() {
    let declared = r#"
impl Item<ChartDoc> for ChartItem {
    fn describe(&self, doc: &ChartDoc) -> Subject {
        Subject::new().with_status(state.status_entry("run-state"))
    }
}
"#;
    assert!(synthetic(&[("crates/a/src/chart.rs", declared)]).is_empty());
}

#[test]
fn an_id_reached_through_a_constant_is_followed() {
    let by_const = r#"
const MADE_UP: &str = "not-a-declared-id";

impl Item<ChartDoc> for ChartItem {
    fn describe(&self, doc: &ChartDoc) -> Subject {
        Subject::new().with_status(StatusEntry {
            id: MADE_UP,
            side: StatusSide::Trailing,
        })
    }
}
"#;
    let found = synthetic(&[("crates/a/src/chart.rs", by_const)]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(found[0].contains("`not-a-declared-id`"), "{}", found[0]);
}

#[test]
fn a_fully_qualified_return_type_is_not_a_construction() {
    // Found by the round-3 sweep: a helper returning the type by its full path
    // was read as a construction that names no id, and reported. Loud rather
    // than silent, but wrong.
    let helper = r#"
pub fn run_line(state: RunState) -> brightfield_workbench::subject::StatusEntry {
    state.status_entry("run-state")
}
"#;
    assert_eq!(
        synthetic(&[("crates/a/src/rail.rs", helper)]),
        Vec::<String>::new(),
        "a signature returning the type by its full path was read as a construction"
    );
}

#[test]
fn a_longer_name_ending_in_status_entry_is_a_different_call() {
    // Both of these are real lines in this workspace, and the first version of
    // this scan read the argument of each as a status-entry id.
    let neighbours = r#"
fn idle_status_entry(composed: &Composed) -> Option<StatusEntry> {
    None
}

fn rail(&mut self) {
    if let Some(idle) = idle_status_entry(&self.charts.doc.composed) {
        entries.push(idle);
    }
    draw_status_entry(ui, entry, mode, &mut out);
}
"#;
    assert_eq!(
        synthetic(&[("crates/a/src/window.rs", neighbours)]),
        Vec::<String>::new(),
        "a function whose name merely ends in `status_entry` was read as a declaration"
    );
}

#[test]
fn an_id_the_scan_cannot_follow_is_reported() {
    let computed = r#"
impl Item<ChartDoc> for ChartItem {
    fn describe(&self, doc: &ChartDoc) -> Subject {
        Subject::new().with_status(StatusEntry {
            id: leak(&format!("pane-{n}")),
            side: StatusSide::Trailing,
        })
    }
}
"#;
    let found = synthetic(&[("crates/a/src/chart.rs", computed)]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(
        found[0].contains("unreadable status-entry id"),
        "an id built at run time went unreported: {}",
        found[0]
    );
}

#[test]
fn a_construction_with_no_id_field_is_reported() {
    let headless = r#"
impl Item<ChartDoc> for ChartItem {
    fn describe(&self, doc: &ChartDoc) -> Subject {
        Subject::new().with_status(StatusEntry {
            side: StatusSide::Trailing,
        })
    }
}
"#;
    let found = synthetic(&[("crates/a/src/chart.rs", headless)]);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(found[0].contains("no id field"), "{}", found[0]);
}

#[test]
fn an_allowlist_entry_that_excuses_nothing_is_reported() {
    let owned = vec![("crates/a/src/chart.rs".to_string(), String::new())];
    let stale: &[Allow] = &[("crates/a/src/gone.rs", "self.id()", "a site since deleted")];
    let found = undeclared(&owned, stale);
    assert_eq!(found.len(), 1, "expected one finding, got {found:?}");
    assert!(found[0].contains("excused nothing"), "{}", found[0]);
}
