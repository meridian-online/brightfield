//! The front door: what a launch that named nothing shows, and what the
//! second click does.
//!
//! The empty state itself was already shipped and already gated — every pane
//! of both views answers an empty document with one, and
//! `brightfield_workbench::audit` is what keeps that true. The mechanism that
//! made it *reachable* — `Boot::empty()` on a no-argument launch, the shipped
//! starts, `EmptyState::with_next` on the view-filling panes — landed next.
//! What is asserted here is the content built on top of it: a window with
//! nothing open draws the front door in place of the dock, the door heads a
//! Datasets section over the starts the binary ships and a Protocols section
//! over what this install has opened before, and taking any card lands on a
//! rendered result rather than on an instrument. The zone names this comment
//! used to list were replaced; `the_door_heads_two_sections_and_says_neither_
//! of_the_names_it_replaced` asserts the old ones are absent from the frame.
//!
//! Interaction tests are GPU-free. `MeridianApp::headless` has no device, so
//! neither canvas pane paints; every rect is the same either way, and each
//! document reports what it holds without needing a texture. The pixel
//! section at the bottom is the exception — the door's baselines, one per
//! state per theme, and the thumbnail regeneration gate render through the
//! real capture path, which
//! needs a wgpu adapter, exactly as `surfaces.rs` does.

use std::path::PathBuf;

use brightfield_protocol::layout::Flow;
use brightfield_protocol::SeamStatus;
use brightfield_shell::app::{chart_registry, ChartDoc, CHART};
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::{
    self, protocol_registry, ProtocolDoc, ProtocolInputs, ProtocolModel, CANVAS,
};
use brightfield_shell::starts::{self, Opened};
use brightfield_shell::startup::{default_layout, opening_boot};
use brightfield_shell::window::{
    Boot, MeridianApp, DATASETS_SECTION, DOOR_ENTRY_PROMISE, PROTOCOLS_EMPTY_BODY,
    PROTOCOLS_EMPTY_TITLE, PROTOCOLS_SECTION,
};
use brightfield_workbench::{Action, PaneKey, Recent, RunState, SavedLayout, Subject};

const CHART_PANE: PaneKey = PaneKey::new(CHART);
const CANVAS_PANE: PaneKey = PaneKey::new(CANVAS);

/// A window under test: the app, and **one** `egui::Context` for its whole
/// life.
///
/// One context, not one per call, for the reason `one_window.rs` records: egui
/// resolves a click against a widget id registered on a previous frame, so two
/// `run` calls through two contexts swallow every pointer interaction and a
/// test that clicks a control passes or fails for reasons unrelated to it.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn open(boot: Boot) -> Self {
        Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        }
    }

    fn run(&mut self, frames: Vec<Vec<egui::Event>>) {
        for events in frames {
            let raw = egui::RawInput {
                screen_rect: Some(self.screen),
                events,
                ..Default::default()
            };
            let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        }
    }

    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new()]);
    }

    /// Click the button the empty **pane** offered, aimed at the rect the
    /// last frame recorded rather than at a coordinate typed here. On a door
    /// frame that rect is the gallery card of the start that fills the pane —
    /// the same affordance, drawn in the door's arrangement.
    fn take_the_way_in(&mut self, pane: PaneKey) {
        let target = self
            .app
            .affordance_rect(pane)
            .unwrap_or_else(|| panic!("{pane} drew no way in for a user to take"));
        assert!(
            self.screen.contains_rect(target),
            "{pane} drew its way in at {target:?}, outside the window — \
             nothing could click it"
        );
        self.run(vec![click_at(target.center()), Vec::new()]);
    }

    /// Click the front door's gallery card for the start `id`, where the last
    /// frame actually drew it.
    fn take_the_card(&mut self, id: &str) {
        let target = self
            .app
            .front_door_card_rect(id)
            .unwrap_or_else(|| panic!("the door drew no card for {id}"));
        assert!(
            self.screen.contains_rect(target),
            "the {id} card drew at {target:?}, outside the window — nothing \
             could click it"
        );
        self.run(vec![click_at(target.center()), Vec::new()]);
    }

    /// Every string this window's next frame draws, in no particular order.
    ///
    /// Read off the frame's own shapes rather than off a hook the door
    /// maintains, because the claim is about what a person sees: a zone name
    /// left behind in a heading, a label built from the wrong constant, and a
    /// section drawn by code nobody remembered to delete all show up here and
    /// none of them shows up in a list the door curates about itself.
    fn drawn_text(&mut self) -> Vec<String> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        let out = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
        let mut text = Vec::new();
        for clipped in &out.shapes {
            collect_text(&clipped.shape, &mut text);
        }
        text
    }

    /// Click the Protocols row for the start `id`, where the last frame drew
    /// it.
    fn take_the_row(&mut self, id: &str) {
        let target = self
            .app
            .front_door_rows()
            .iter()
            .find(|row| row.id == id)
            .unwrap_or_else(|| panic!("the door drew no Protocols row for {id}"))
            .rect;
        assert!(
            self.screen.contains_rect(target),
            "the {id} row drew at {target:?}, outside the window — nothing              could click it"
        );
        self.run(vec![click_at(target.center()), Vec::new()]);
    }

    /// Press the registry's `open-home` keystroke — the way back to the door
    /// from a window with something in it.
    fn go_home(&mut self) {
        self.run(vec![
            vec![egui::Event::Key {
                key: egui::Key::H,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            }],
            Vec::new(),
        ]);
    }
}

/// The galleys in `shape`, flattened. `Shape::Vec` nests, so a walk that reads
/// the top level and stops misses whatever a widget put inside a group.
fn collect_text(shape: &egui::epaint::Shape, into: &mut Vec<String>) {
    match shape {
        egui::epaint::Shape::Text(t) => into.push(t.galley.text().to_string()),
        egui::epaint::Shape::Vec(shapes) => {
            for s in shapes {
                collect_text(s, into);
            }
        }
        _ => {}
    }
}

/// The sections the last frame's door drew, by name and in draw order.
fn section_names(win: &Window) -> Vec<&'static str> {
    win.app
        .front_door_sections()
        .iter()
        .map(|(name, _)| *name)
        .collect()
}

/// A layout that remembers `recents`, most recent first — the returning
/// analyst's file, built here rather than by driving five opens, so a test can
/// pin what the door draws for a run state this build cannot yet produce.
///
/// The times are **relative to `now`** and land mid-bucket on purpose: a
/// fixture pinned to an absolute instant drifts into the next bucket the day
/// after it is written, and one pinned to a bucket boundary tips whenever the
/// capture takes a second longer than usual. See `relative_time`.
fn layout_remembering(recents: &[(&str, &str, RunState, u64)]) -> SavedLayout {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs();
    let mut layout = default_layout();
    layout.recents = recents
        .iter()
        .map(|(id, name, run, ago)| Recent {
            id: (*id).to_string(),
            name: (*name).to_string(),
            run: *run,
            opened_at: now - ago,
        })
        .collect();
    layout
}

/// The three recents both returning-door baselines are photographed over, and
/// the fixture the interaction tests seed.
///
/// Three run states rather than three of one, so the state column is pinned as
/// a column rather than as a repeated string — [`RunState::Fresh`] and
/// [`RunState::StaleUpstream`] are states the layout file can carry and this
/// build's own starts cannot yet reach, which is exactly why a fixture is what
/// holds the drawing of them.
fn returning_recents() -> [(&'static str, &'static str, RunState, u64); 3] {
    [
        (starts::CROSSWALK, "edgar-gleif", RunState::NeverRun, 150),
        (
            starts::DASHBOARD,
            "signals-dashboard",
            RunState::Fresh,
            30 * 60 * 60,
        ),
        (
            starts::DISTRIBUTION,
            "reading-distribution",
            RunState::StaleUpstream,
            4 * 24 * 60 * 60 + 4 * 60 * 60,
        ),
    ]
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

/// One frame's worth of the `open-home` keystroke, cmd-shift-h. `command` and
/// `shift` are what `consume_key`'s logical match reads — mac_cmd/ctrl are
/// platform detail the pattern ignores — so this fires the same on every
/// runner.
fn press_home() -> Vec<egui::Event> {
    let modifiers = egui::Modifiers {
        command: true,
        shift: true,
        ..Default::default()
    };
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::Key {
            key: egui::Key::H,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        })
        .collect()
}

fn chart_subject(doc: &ChartDoc) -> Subject {
    chart_registry()
        .specs()
        .iter()
        .find(|s| s.id == CHART)
        .map(|spec| (spec.make)().subject(doc))
        .expect("the chart pane is in the registry")
}

fn canvas_subject(doc: &ProtocolDoc) -> Subject {
    protocol_registry()
        .specs()
        .iter()
        .find(|s| s.id == CANVAS)
        .map(|spec| (spec.make)().subject(doc))
        .expect("the canvas pane is in the registry")
}

// ---------------------------------------------------------------------------
// What ships
// ---------------------------------------------------------------------------

/// Every shipped starting point loads, all the way to a document with
/// something in it, from the bytes in the binary — and the set stays the
/// curated size: between three and five, anchored by the crosswalk.
///
/// This is what `include_str!` cannot check. It proves the fixture is present
/// at compile time and nothing else — a spec that composed no plots would
/// still build and would still put a card in the gallery that resolves to
/// an apology.
///
/// A manifest whose models are keyed under names it does not use is the other
/// half, and the assertions here could not see it: mis-keying all four of
/// `starts::CROSSWALK_MODELS` leaves `sheet_rows` identical and only drops the
/// graph from 34 nodes / 40 edges to 30 / 27 — a third of the lineage gone,
/// quietly. So it is not asserted here; it is refused at the source, by
/// `protocol::load_protocol_str`, and the refusal surfaces at the
/// `does not load` panic below. Watched redden, one mutation: all four
/// `"models/…"` keys in `starts.rs` changed to `"WRONGDIR/…"`.
///
/// It also asserts the load touches no path: nothing here sets a working
/// directory, and the suite runs from the crate root rather than the repo
/// root, so a start that read a file relative to the process would fail
/// here.
///
/// **What it deliberately does not cover.** A start that declares
/// [`starts::Start::remote`] reads a source this binary does not carry, so
/// loading it here would put an https fetch of someone else's server inside a
/// hermetic suite — green or red for reasons that have nothing to do with this
/// repo. Those are skipped, the skip is counted so it cannot quietly swallow
/// the set, and the same assertions are made over them by the `#[ignore]`d
/// `the_crosswalk_chart_start_opens_over_the_network_drawing_every_row` in
/// `tests/crosswalk_chart.rs`. The trade is stated rather than hidden: that
/// start's load gate runs on demand, not on every push.
#[test]
fn every_shipped_start_loads_into_a_document_with_something_in_it() {
    assert!(
        (3..=5).contains(&starts::STARTS.len()),
        "the curated set is {} starts — few enough to choose from at a \
         glance, enough to read as a gallery, is the recorded size",
        starts::STARTS.len()
    );
    assert!(
        starts::find(starts::CROSSWALK).is_some(),
        "the crosswalk anchors the set; a gallery without it is a different \
         product decision, not a refactor"
    );
    let local: Vec<&starts::Start> = starts::STARTS.iter().filter(|s| !s.remote).collect();
    assert!(
        local.len() >= 3,
        "only {} start(s) can be loaded without a network, so this gate has \
         been hollowed out by the `remote` exemption rather than narrowed by it",
        local.len()
    );
    for start in local {
        let opened = starts::load(start.id)
            .unwrap_or_else(|e| panic!("the shipped start {} does not load: {e}", start.id));
        let loads_a_chart = matches!(opened, Opened::Charts(_));
        assert_eq!(
            loads_a_chart,
            start.fills == CHART,
            "{} declares it fills the {} pane and loads a {} document",
            start.id,
            start.fills,
            if loads_a_chart { "chart" } else { "protocol" }
        );
        match opened {
            Opened::Charts(chart) => assert!(
                chart.composed.width > 0 && chart.composed.height > 0,
                "{} composed no plots, so the card that opens it resolves \
                 one empty state into another",
                start.id
            ),
            Opened::Protocol(inputs) => {
                assert!(
                    !inputs.graph_collapsed.nodes.is_empty(),
                    "{} built no assets",
                    start.id
                );
                assert!(
                    !inputs.sheet_rows.is_empty(),
                    "{} built no steps, so the steps sheet opens empty behind \
                     the click",
                    start.id
                );
                assert!(
                    !inputs.graph_full.edges.is_empty(),
                    "{} built assets with no lineage between them, which is \
                     the one thing this view is for",
                    start.id
                );
            }
        }
    }
}

/// A start that opens a run-less Protocol manifest says so on its own button.
///
/// The **pick** half of the exemption from
/// `protocol::run_less_manifest_refusal` — not the whole of it, which this
/// once claimed. The other half is the restore:
/// `a_launch_with_something_to_restore_shows_no_front_door` reopens the same
/// crosswalk with no button and no click in the path, and nothing on that
/// surface carries the mark. That is the rule the code implements — disclosed
/// once at the pick, then remembered in the layout file — and
/// `run_less_manifest_refusal` is where it is stated, including where the
/// memory can come from and what invalidates it. Nothing here holds that half;
/// saying so is cheaper than a claim that has to be read as narrower than it
/// sounds.
///
/// The gate exists because this view's default input is an emitted
/// Protocol+Run contract, a manifest is the same shape without a run behind it, and nothing
/// on the canvas tells them apart — so `BRIGHTFIELD_PROTOCOL_OFFLINE` is made
/// to carry the difference for a path handed in from outside. The crosswalk on
/// the front door is exactly that artifact class, and one click reaches it
/// without any variable being set.
///
/// What makes that honest rather than a hole is that the disclosure is made in
/// the place the variable cannot reach: on the button. This is the assertion
/// that the two stay together — a `run_less` start whose label drops the mark
/// fails here, and so does a label that claims the mark without the flag.
///
/// The gate itself is asserted through its message rather than through the
/// environment, because a test that sets or clears a process-wide variable is
/// a test that changes what its neighbours in the same binary see.
#[test]
fn a_start_that_opens_a_run_less_manifest_says_so_on_its_own_button() {
    for start in starts::STARTS {
        assert_eq!(
            start.run_less,
            start.label.contains(starts::RUN_LESS_MARK),
            "{}'s label {:?} and its run_less flag ({}) disagree — the flag is \
             what exempts it from the offline gate, and the label is the only \
             reason that exemption is honest",
            start.id,
            start.label,
            start.run_less
        );
    }

    assert!(
        starts::STARTS.iter().any(|s| s.run_less),
        "no shipped start is run-less, so this test is holding nothing — if \
         that is now true, delete it and the exemption with it"
    );

    // And the rule the exemption is from still names its opt-in, so the two
    // halves cannot drift into different vocabularies.
    let refusal = protocol::run_less_manifest_refusal("some/arcform.yaml");
    assert!(refusal.contains(protocol::OFFLINE_VAR), "{refusal}");
    assert!(refusal.contains("some/arcform.yaml"), "{refusal}");
}

/// The front door's controls claim no keystroke, because they have none.
///
/// The chrome renders an affordance's verb's *real* keystroke beside its
/// label, straight from the keyboard registry. There is no registered command
/// meaning "open the signals dashboard", so an affordance built with a
/// borrowed verb would ship a button reading `Open the signals dashboard
/// cmd-r` and pressing that key would do something else entirely. Declaring an
/// `Action::Open` is what makes that unrepresentable rather than merely
/// avoided.
///
/// Watched redden, one mutation: building the chart pane's affordance with
/// `Affordance::new(label, Verb::new("reload-spec"))` instead fails here at
/// "declares a verb".
#[test]
fn the_way_in_declares_no_verb_and_therefore_no_keystroke() {
    for (what, subject) in [
        ("the chart pane", chart_subject(&ChartDoc::empty())),
        ("the canvas pane", canvas_subject(&ProtocolDoc::empty())),
    ] {
        let empty = subject
            .empty_state
            .as_ref()
            .expect("an empty document shows an empty state");
        let next = empty
            .next
            .as_ref()
            .unwrap_or_else(|| panic!("{what} offers no way in"));
        assert!(
            matches!(next.action, Action::Open(_)),
            "{what} offers a way in that declares a verb: {:?}",
            next.action
        );
        // Every verb the subject declares comes from its toolbar and only
        // its toolbar — the way in itself contributes none, so the chrome
        // cannot print a keystroke on the button. (The chart pane's toolbar
        // legitimately declares `clear-selection`, mostly Hidden; that is a
        // control, not a way in.)
        let toolbar_verbs: Vec<_> = subject.toolbar.iter().map(|t| t.verb).collect();
        assert_eq!(
            subject.declared_verbs(),
            toolbar_verbs,
            "{what}'s empty state declares a verb, so the chrome will print \
             that verb's keystroke on the button"
        );
    }
}

/// Every start's embedded thumbnail is byte-for-byte the committed file, and
/// each decodes to the gallery card's own 16:10 — for **both** [`Mode`]s.
///
/// The binary ships `include_bytes!` and the regeneration gate at the bottom
/// of this file holds the *file* against the bundled spec — this is the strut
/// between them, so a thumbnail regenerated on disk cannot quietly diverge
/// from the bytes a stale build embedded, and an include path pointed at the
/// wrong file fails as itself rather than as a pixel diff.
#[test]
fn every_shipped_thumbnail_is_the_committed_file() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/starts");
    for start in starts::STARTS {
        for (mode, suffix, embedded) in [
            (Mode::Light, "", start.thumbnail),
            (Mode::Dark, "-dark", start.thumbnail_dark),
        ] {
            let path = dir.join(format!("{}{suffix}.png", start.id));
            let committed = std::fs::read(&path).unwrap_or_else(|e| {
                panic!("{} has no committed {mode:?} thumbnail: {e}", start.id)
            });
            assert_eq!(
                committed,
                embedded,
                "{}'s embedded {mode:?} thumbnail is not the committed {} — \
                 rebuild, or fix the include path",
                start.id,
                path.display()
            );
            let decoded = image::load_from_memory(embedded).unwrap_or_else(|e| {
                panic!("{}'s {mode:?} thumbnail does not decode: {e}", start.id)
            });
            assert_eq!(
                (decoded.width(), decoded.height()),
                (480, 300),
                "{}'s {mode:?} thumbnail is not the gallery card's 480×300",
                start.id
            );
            assert_eq!(
                start.thumbnail_for(mode),
                embedded,
                "{}'s thumbnail_for({mode:?}) did not select the {mode:?} slice",
                start.id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Reaching it, and what the second click does
// ---------------------------------------------------------------------------

/// A launch that named nothing opens the front door: every shipped start's
/// card is on it and clickable, and nothing about it is a dismissal to find,
/// because there is nothing to dismiss.
///
/// The state this replaces twice over: first a hardcoded example nobody asked
/// for, then a dock of empty instruments each inviting the same first action
/// from a different corner.
#[test]
fn an_empty_launch_opens_the_front_door_with_every_start_on_it() {
    let mut win = Window::open(Boot::empty());
    win.settle();

    assert!(
        win.app.front_door_is_live(),
        "an empty launch did not land on the front door"
    );
    assert!(
        win.app.chart_doc().is_empty(),
        "an empty launch composed a dashboard from somewhere"
    );
    assert!(
        !win.app.protocol_model().has_assets(),
        "an empty launch built a graph from somewhere"
    );
    for start in starts::STARTS {
        let card = win
            .app
            .front_door_card_rect(start.id)
            .unwrap_or_else(|| panic!("the door drew no card for {}", start.id));
        assert!(
            win.screen.contains_rect(card),
            "{}'s card drew at {card:?}, outside the window",
            start.id
        );
    }
    assert!(
        win.app.front_door_rows().is_empty(),
        "a first run listed recent work it has never had"
    );
    assert!(
        win.app.front_door_help_rect().is_some(),
        "the Start zone offers nothing to do"
    );

    // The door belongs to the **window**, not to a document: reached back
    // from a graph — the document that is not the chart, and the one whose
    // canvas the door stands where — it draws the same cards.
    //
    // Through the shipped route rather than a test hook, because there is no
    // control that moves the canvas between the two: the protocol is the
    // navigator rail rather than a peer view, and the pair of
    // `selectable_label`s that used to switch between them is gone.
    win.take_the_card(starts::CROSSWALK);
    win.settle();
    assert!(
        win.app.graph_on_canvas(),
        "the crosswalk did not put its graph on the canvas, so going home \
         from it proves nothing about the door"
    );
    win.go_home();
    win.settle();
    assert!(
        win.app.front_door_is_live(),
        "cmd-shift-h from a graph did not return to the door"
    );
    for start in starts::STARTS {
        assert!(
            win.app.front_door_card_rect(start.id).is_some(),
            "the door reached back from a graph lost {}'s card",
            start.id
        );
    }
}

/// On a door frame, the pane's recorded way in *is* the gallery card of the
/// start that fills it — one answer to "where is the way in" across both
/// arrangements of the same affordance.
///
/// This is the compatibility other suites lean on (`layout_wiring.rs` clicks
/// the chart pane's affordance on an empty boot), so it is pinned here as its
/// own claim rather than left as a side effect two files happen to agree on.
#[test]
fn the_pane_way_in_is_the_doors_card_for_the_start_that_fills_it() {
    let mut win = Window::open(Boot::empty());
    win.settle();

    assert_eq!(
        win.app.affordance_rect(CHART_PANE),
        win.app.front_door_card_rect(starts::DASHBOARD),
        "the chart pane's way in and the dashboard card disagree about where \
         the same affordance is"
    );
    assert_eq!(
        win.app.affordance_rect(CANVAS_PANE),
        win.app.front_door_card_rect(starts::CROSSWALK),
        "the canvas pane's way in and the crosswalk card disagree about where \
         the same affordance is"
    );

    // And the recorded way in is not merely the same rectangle — taking it
    // does what taking the card does, which is the interaction the other
    // suites replay.
    win.take_the_way_in(CHART_PANE);
    win.settle();
    assert!(
        !win.app.chart_doc().is_empty(),
        "the pane's recorded way in opened nothing"
    );
    assert_eq!(win.app.layout().opened.as_deref(), Some(starts::DASHBOARD));
}

/// The second click lands on a **rendered dashboard**, and the front door is
/// gone because the window has content rather than because anything dismissed
/// it.
///
/// Watched redden, two mutations: dropping the `Request::Open` arm from
/// `MeridianApp::apply` — the arm the charts view used to have as `{}`, which
/// is exactly how a front door ships as chrome that does nothing — fails at
/// "the click opened nothing"; and having `open_start` set the document
/// without recording the id fails at the `opened` assertion, which is the half
/// that makes a later launch restore it.
#[test]
fn a_dashboard_card_lands_on_a_rendered_dashboard() {
    let mut win = Window::open(Boot::empty());
    win.settle();
    win.take_the_card(starts::DASHBOARD);
    win.settle();

    assert!(
        !win.app.chart_doc().is_empty(),
        "the click opened nothing — the chart pane is still empty, which is a \
         front door that has moved the blank canvas rather than removed it"
    );
    assert!(
        !win.app.graph_on_canvas(),
        "opening a chart start did not put its chart on the canvas"
    );
    assert!(
        chart_subject(win.app.chart_doc()).empty_state.is_none(),
        "the chart pane still declares itself empty over a composed dashboard"
    );
    assert!(
        !win.app.front_door_is_live(),
        "the front door is still live over content"
    );
    for start in starts::STARTS {
        assert!(
            win.app.front_door_card_rect(start.id).is_none(),
            "{}'s card is still recorded over content",
            start.id
        );
    }
    assert!(
        win.app.affordance_rect(CHART_PANE).is_none(),
        "a way in is still recorded over content"
    );
    assert_eq!(
        win.app.layout().opened.as_deref(),
        Some(starts::DASHBOARD),
        "nothing recorded what was opened, so the next launch cannot restore it"
    );
}

/// The crosswalk's card, taken from the **charts** view, lands on a **built
/// asset graph** in the protocol view: the outline, the steps sheet and the
/// inspector all have content behind them, not just the canvas.
///
/// From the charts view deliberately: the gallery's promise is the same
/// wherever the switcher happens to be, so a card whose start fills the other
/// view carries the click across — the door is the window's, and the view
/// switch is `open_start`'s ordinary behaviour, not a special case.
#[test]
fn the_crosswalk_card_lands_on_a_rendered_graph() {
    let mut win = Window::open(Boot::empty());
    win.settle();
    assert!(!win.app.graph_on_canvas());
    win.take_the_card(starts::CROSSWALK);
    win.settle();

    let model = win.app.protocol_model();
    assert!(model.has_assets(), "the click built no assets");
    assert!(
        !model.displayed_graph().nodes.is_empty(),
        "the click left the canvas with nothing to draw"
    );
    assert!(
        !model.sheet().is_empty(),
        "the click left the steps sheet empty"
    );
    assert!(
        win.app.graph_on_canvas(),
        "the crosswalk opened without putting its graph on the canvas"
    );
    assert!(
        win.app.front_door_card_rect(starts::CROSSWALK).is_none(),
        "the front door is still drawn over a graph"
    );
    assert_eq!(win.app.layout().opened.as_deref(), Some(starts::CROSSWALK));
}

/// The Start zone's one working verb works: the keyboard-help control opens
/// the help sheet, the same overlay `?` opens — printed with the registry's
/// keystroke, never claiming one of its own.
#[test]
fn the_start_zone_opens_the_help_sheet() {
    let mut win = Window::open(Boot::empty());
    win.settle();
    let help = win
        .app
        .front_door_help_rect()
        .expect("the Start zone drew its help control");
    win.run(vec![click_at(help.center()), Vec::new()]);
    assert_eq!(
        win.app.open_overlay(),
        Some("help"),
        "the Start zone's help control opened nothing"
    );
}

/// A launch with work to restore restores it and shows **no** front door.
///
/// This is the morph's sharp edge, and it is why the layout has to carry what
/// was open rather than only where the panes are: restoring an arrangement
/// alone would come up with the user's splitter positions around panes that
/// are all still empty, and the door would rightly go on inviting a first
/// action. There is no "don't show this again" anywhere — the surface simply
/// stops being an invitation once it has content.
///
/// This is also the path that takes the run-less exemption **without** a
/// click: the crosswalk is a manifest with no run behind it, no
/// `BRIGHTFIELD_PROTOCOL_OFFLINE` is set, and no surface here carries
/// `starts::RUN_LESS_MARK`. That is the remembered form of the exemption
/// rather than a hole in it — `protocol::run_less_manifest_refusal` states
/// where the memory comes from — and it is named here because this test is the
/// path, and a reader arriving at it should not have to reconstruct that.
///
/// Watched redden, one mutation: having `startup::opening_boot` ignore its
/// `opened` argument and always return `Boot::empty()` — which is all a shell
/// that persisted only the arrangement can do — fails here at "restored
/// nothing".
#[test]
fn a_launch_with_something_to_restore_shows_no_front_door() {
    let mut layout = default_layout();
    layout.opened = Some(starts::CROSSWALK.to_string());

    // Through the same function `main` calls, with the same two arguments it
    // has: no spec on the command line, and whatever the layout remembered.
    let boot = opening_boot(None, layout.opened.as_deref(), Flow::Vertical, None)
        .expect("an unnamed launch cannot fail");
    let mut win = Window {
        app: MeridianApp::headless_with_layout(boot, layout, Mode::Light),
        ctx: egui::Context::default(),
        screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
    };
    win.settle();

    assert!(win.app.graph_on_canvas());
    assert!(
        win.app.protocol_model().has_assets(),
        "the launch restored nothing"
    );
    assert!(
        !win.app.front_door_is_live(),
        "a launch that restored its work is still on the front door"
    );
    for start in starts::STARTS {
        assert!(
            win.app.front_door_card_rect(start.id).is_none(),
            "a launch that restored its work still drew {}'s card",
            start.id
        );
    }
    assert!(
        win.app.affordance_rect(CANVAS_PANE).is_none(),
        "a launch that restored its work still offered a way in"
    );
    assert!(
        canvas_subject(win.app.protocol_doc()).empty_state.is_none(),
        "the canvas pane declares itself empty over a restored graph"
    );
}

/// The door heads exactly two sections — Datasets and Protocols — and says
/// neither of the two names they replaced anywhere on the screen.
///
/// *Explore* and *Continue* were this door's earlier names for these two
/// zones, and the sections now hold the product's own primitives. The reason
/// this is asserted over the **rendered text** rather than over the door's own
/// list of section names: a zone name is not only a heading, and a build that
/// renamed the heading while leaving the old word in a body line would pass a
/// headings-only check and still put the retired vocabulary in front of a
/// stranger.
///
/// Watched redden, two mutations, one for each half. Passing `"Explore"` to
/// `door_section_heading` in `datasets_section` instead of `DATASETS_SECTION`
/// fails at the section-order assertion (`["Explore", "Protocols"]`). Putting
/// the retired word in a body line instead — `DATASETS_NOTE` rewritten as
/// *"the Explore gallery, curated and shipped with this build"* — leaves that
/// assertion green and fails at "still says Explore", which is the half a
/// headings-only check would not have.
#[test]
fn the_door_heads_two_sections_and_says_neither_of_the_names_it_replaced() {
    let mut win = Window::open(Boot::empty());
    win.settle();
    assert!(win.app.front_door_is_live());

    assert_eq!(
        section_names(&win),
        vec![DATASETS_SECTION, PROTOCOLS_SECTION],
        "the door's sections are not the two the ruling names, or not in the \
         order a first run draws them"
    );

    let text = win.drawn_text();
    for retired in ["Explore", "Continue", "Learn"] {
        assert!(
            !text.iter().any(|drawn| drawn.contains(retired)),
            "a headless render of the first-run door still says {retired}: {text:?}"
        );
    }
    for kept in [DATASETS_SECTION, PROTOCOLS_SECTION] {
        assert!(
            text.iter().any(|drawn| drawn.contains(kept)),
            "a headless render of the first-run door never says {kept}: {text:?}"
        );
    }
}

/// A first run: Datasets is present **and populated**, and Protocols is
/// present **and says what will fill it**.
///
/// This is the blank-page state the front door exists to prevent, in the one
/// arrangement every install passes through exactly once. The two halves fail
/// differently and are asserted separately: a Datasets section with no cards
/// is a catalogue that shipped empty, and an absent Protocols heading is a
/// section that appears from nowhere on the second launch, which teaches a
/// stranger nothing about what the product saves.
///
/// Watched redden, one mutation: returning early from `protocols_section`
/// before `door_section_heading` when `recents.is_empty()` — the shape of
/// "draw the section once it has content and not before" — fails here at "the
/// Protocols heading is absent on a first run".
///
/// The on-screen assertion below was watched fail on real code rather than on
/// a mutation: with `DOOR_COLUMN_WIDTH` written as four cards, the fifth start
/// wrapped the gallery to a second row and this heading drew at y ≈ 900 in an
/// 820-point window.
#[test]
fn a_first_run_populates_datasets_and_states_what_protocols_will_hold() {
    let mut win = Window::open(Boot::empty());
    win.settle();

    assert!(
        win.app
            .front_door_sections()
            .iter()
            .any(|(name, _)| *name == DATASETS_SECTION),
        "the Datasets heading is absent on a first run"
    );
    assert!(
        win.app
            .front_door_sections()
            .iter()
            .any(|(name, _)| *name == PROTOCOLS_SECTION),
        "the Protocols heading is absent on a first run — the section appears \
         from nowhere on the second launch"
    );
    for start in starts::STARTS {
        assert!(
            win.app.front_door_card_rect(start.id).is_some(),
            "the Datasets section drew no card for {} — a catalogue that \
             shipped empty",
            start.id
        );
    }
    assert!(
        win.app.front_door_rows().is_empty(),
        "a first run drew a Protocols row out of a layout that remembers \
         nothing"
    );

    // Present in the frame's records is not the same as present on the
    // screen, and the difference is the whole defect: with the gallery
    // wrapping to a second row this section drew, recorded its heading, and
    // landed past the bottom of the default window — so the first-run
    // baseline could not see it and neither could a first-run user. A window
    // that cannot hold the door's two sections at its default size is a
    // decision about the door, and this is where it comes up for one.
    let (_, heading) = win
        .app
        .front_door_sections()
        .iter()
        .find(|(name, _)| *name == PROTOCOLS_SECTION)
        .copied()
        .expect("the Protocols heading is drawn on a first run");
    assert!(
        win.screen.contains_rect(heading),
        "the Protocols heading drew at {heading:?}, past the bottom of a \
         {:?} window — the empty section nobody ever sees",
        win.screen.size()
    );

    let text = win.drawn_text();
    for said in [PROTOCOLS_EMPTY_TITLE, PROTOCOLS_EMPTY_BODY] {
        assert!(
            text.iter().any(|drawn| drawn.contains(said)),
            "the empty Protocols section never says {said:?}: {text:?}"
        );
    }
    assert!(
        text.iter().any(|drawn| drawn.contains(DOOR_ENTRY_PROMISE)),
        "no entry on the first-run door carries the promise {DOOR_ENTRY_PROMISE:?}: {text:?}"
    );
}

/// A door whose layout remembers three Protocols draws **three rows**, most
/// recent first, each carrying its name and its run state — and Protocols
/// leads, above the catalogue.
///
/// The count is the assertion the old Continue zone could not pass: it was
/// built from `SavedLayout::opened`, one id, so a layout that remembered three
/// sessions drew one control and the other two were unreachable from the one
/// screen whose whole job is to put you back in them.
///
/// The run states are three different ones on purpose. Two of them are states
/// this build's own starts cannot reach, because `open_start` records what
/// `ProtocolModel::recorded_run_state` folds and a shipped start carries no
/// run contract for it to fold. Seeding them is what proves the row *draws
/// what was recorded* rather than a constant that happens to be right for
/// `RunState::NeverRun`.
///
/// Watched redden, two mutations. Having `door_row` build its label from
/// `RunState::NeverRun.label()` instead of `recent.run.label()` fails at
/// "signals-dashboard's row does not carry the run state the layout recorded"
/// (`"never run"` against `"fresh"`). Drawing `recents.iter().take(1)` — which
/// is as much as the `SavedLayout::opened` door could do — fails at "the door
/// drew 1 row(s) for 3 remembered Protocols".
#[test]
fn a_door_with_recents_lists_every_one_of_them_most_recent_first() {
    let recents = returning_recents();
    let mut win = Window {
        app: MeridianApp::headless_with_layout(
            Boot::empty(),
            layout_remembering(&recents),
            Mode::Light,
        ),
        ctx: egui::Context::default(),
        screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
    };
    win.settle();

    assert!(win.app.front_door_is_live(), "nothing open, so the door");
    assert_eq!(
        section_names(&win),
        vec![PROTOCOLS_SECTION, DATASETS_SECTION],
        "a returning analyst's own work does not lead the door"
    );

    let drawn = win.app.front_door_rows().to_vec();
    assert_eq!(
        drawn.len(),
        recents.len(),
        "the door drew {} row(s) for {} remembered Protocols",
        drawn.len(),
        recents.len()
    );
    for (row, (id, name, run, _)) in drawn.iter().zip(recents.iter()) {
        assert_eq!(row.id, *id, "the rows are not in most-recent-first order");
        assert_eq!(
            row.name, *name,
            "{id}'s row is not named after its Protocol"
        );
        assert_eq!(
            row.state,
            run.label(),
            "{id}'s row does not carry the run state the layout recorded"
        );
        assert!(
            win.screen.contains_rect(row.rect),
            "{id}'s row drew at {:?}, outside the window",
            row.rect
        );
    }
    assert_eq!(
        drawn.iter().map(|r| r.when.as_str()).collect::<Vec<_>>(),
        vec!["2m ago", "yesterday", "4 days ago"],
        "the rows do not say how long ago each was opened"
    );

    // And a row is a way back in, not a label: clicking the third one — the
    // one an `opened`-shaped door could never have offered — reopens it.
    win.take_the_row(starts::DISTRIBUTION);
    win.settle();
    assert!(
        !win.app.chart_doc().is_empty(),
        "a Protocols row reopened nothing"
    );
    assert!(!win.app.front_door_is_live());
}

/// Either route to the same subject leaves the window in the same state.
///
/// The door owns no route of its own: a Protocols row and the Datasets card
/// beside it raise the same `Request::Open` into the same `open_start`, so
/// there is nothing for the two to disagree about. That is the property, and
/// this walks **every** shipped start rather than one, because a route that
/// diverges for one document kind and not the other is exactly the divergence
/// that would survive a single-case test.
///
/// The remote start is skipped, for the reason its siblings in this file skip
/// it: taking its card composes its spec, and that spec reads an `https://`
/// source, which would put a fetch of someone else's server inside a hermetic
/// suite.
///
/// Watched redden, one mutation: having `door_row` push
/// `Request::Focus(PaneKey::new(PROTOCOL_CANVAS))` after
/// its `Request::Open` — a plausible "put the cursor where the work is" —
/// fails at "edgar-gleif-crosswalk: the row and the card leave focus on
/// different surfaces", `Some(protocol-canvas)` against `None`.
#[test]
fn either_route_to_the_same_subject_leaves_the_same_window() {
    for start in starts::STARTS {
        if start.remote {
            continue;
        }
        let recents = [(start.id, "remembered", RunState::NeverRun, 60)];

        let mut by_row = Window {
            app: MeridianApp::headless_with_layout(
                Boot::empty(),
                layout_remembering(&recents),
                Mode::Light,
            ),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        };
        by_row.settle();
        by_row.take_the_row(start.id);
        by_row.settle();

        let mut by_card = Window::open(Boot::empty());
        by_card.settle();
        by_card.take_the_card(start.id);
        by_card.settle();

        assert_eq!(
            by_row.app.graph_on_canvas(),
            by_card.app.graph_on_canvas(),
            "{}: the row and the card put different documents on the canvas",
            start.id
        );
        assert_eq!(
            by_row.app.focused_pane(),
            by_card.app.focused_pane(),
            "{}: the row and the card leave focus on different surfaces",
            start.id
        );
        assert_eq!(
            by_row.app.front_door_is_live(),
            by_card.app.front_door_is_live(),
            "{}: one route left the door up and the other did not",
            start.id
        );
        assert_eq!(
            by_row.app.layout().opened.as_deref(),
            by_card.app.layout().opened.as_deref(),
            "{}: the two routes recorded different work to restore",
            start.id
        );
    }
}

/// Opening a start puts it at the head of the door's own list, and opening a
/// second does not lose the first.
///
/// The half `a_door_with_recents_lists_every_one_of_them_most_recent_first`
/// cannot see: that test seeds the file, and a build that drew a seeded file
/// beautifully and never wrote one would pass it and ship a Protocols section
/// that is empty forever.
///
/// Watched redden, two mutations. Dropping the `remember` call from
/// `open_start`, which leaves the `opened` line that was already there, fails
/// here with an empty recents list against `["signals-dashboard"]`. Having
/// `MeridianApp::subject_name` return `String::new()` for both views fails at
/// the names, `["", ""]` against the two documents' own titles.
#[test]
fn opening_a_start_remembers_it_and_keeps_what_was_remembered_before() {
    let mut win = Window::open(Boot::empty());
    win.settle();
    assert!(win.app.layout().recents.is_empty());

    win.take_the_card(starts::DASHBOARD);
    win.settle();
    assert_eq!(
        win.app
            .layout()
            .recents
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec![starts::DASHBOARD],
        "opening a start recorded nothing for the door to list"
    );

    // Home, so the door draws again over a session that has history, and the
    // second card is reachable.
    win.run(vec![press_home(), Vec::new()]);
    win.settle();
    win.take_the_card(starts::DISTRIBUTION);
    win.settle();
    assert_eq!(
        win.app
            .layout()
            .recents
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec![starts::DISTRIBUTION, starts::DASHBOARD],
        "the second open replaced the first instead of joining it"
    );
    // What each record is *named* after, which the ids above cannot see: the
    // document that was opened, not the start it was reached through. The
    // start's own label is a verb — "Open the reading distribution" — and a
    // build that recorded that, or a constant, or nothing at all, keeps every
    // assertion above green.
    assert_eq!(
        win.app
            .layout()
            .recents
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Distribution of readings", "A year of signals"],
        "the two records are not named after the two documents that were opened"
    );
    assert_eq!(
        win.app.layout().recents[0].name,
        win.app.chart_doc().title(),
        "the head record is not named after the document still open behind it"
    );
}

/// What `open_start` records beside the id is what the opened document says —
/// its own name, and its own run state.
///
/// The half `a_door_with_recents_lists_every_one_of_them_most_recent_first`
/// cannot see, in the other direction from the test above: that one seeds a
/// layout file and reads back what the door drew from it, so a build that
/// recorded a constant name and a constant run state would draw the fixture
/// beautifully and record neither. Both fields are produced at one site each —
/// `MeridianApp::subject_name` and `MeridianApp::recorded_run_state` — and
/// each has one caller, which is this write.
///
/// Both arms of both matches are walked. The crosswalk fills the Protocol
/// view and the dashboard fills Charts; the two documents name themselves
/// differently, so a name read off the wrong document is a difference here
/// too, and the two windows are separate so that neither open is asked to
/// find a card under the other one's rows.
///
/// The run state a *shipped* start records is `RunState::NeverRun` in this
/// build, for the reason `MeridianApp::recorded_run_state` gives: a chart
/// composes with no run contract behind it, and the crosswalk manifest is
/// loaded with no run. So the protocol half is asserted against the document's
/// own fold rather than against a literal alone, and the fold itself — which
/// this build's starts cannot exercise — is held by
/// `a_failed_step_anywhere_beats_a_success_anywhere`.
///
/// Watched redden, two mutations. `MeridianApp::subject_name` returning
/// `String::new()` for both views fails at "the record is not named after
/// what the opened Protocol calls itself", `""` against `"edgar_gleif"`.
/// `MeridianApp::recorded_run_state` returning `RunState::Fresh` for both
/// views fails at "the record carries a run state the opened Protocol does not
/// report", `Fresh` against `NeverRun`.
#[test]
fn what_open_start_records_is_what_the_opened_document_says() {
    // The Protocol arm.
    let mut by_protocol = Window::open(Boot::empty());
    by_protocol.settle();
    by_protocol.take_the_card(starts::CROSSWALK);
    by_protocol.settle();

    let protocol_name = by_protocol.app.protocol_model().protocol.clone();
    assert!(
        !protocol_name.is_empty(),
        "the opened Protocol document names itself nothing, so nothing below \
         this line pins anything"
    );
    let recorded = by_protocol.app.layout().recents[0].clone();
    assert_eq!(
        recorded.name, protocol_name,
        "the record is not named after what the opened Protocol calls itself"
    );
    assert_eq!(
        recorded.run,
        by_protocol.app.protocol_model().recorded_run_state(),
        "the record carries a run state the opened Protocol does not report"
    );
    assert_eq!(
        recorded.run,
        RunState::NeverRun,
        "the crosswalk manifest is loaded with no run behind it, so never run \
         is what there is to record"
    );

    // The Charts arm, in a window of its own.
    let mut by_chart = Window::open(Boot::empty());
    by_chart.settle();
    by_chart.take_the_card(starts::DASHBOARD);
    by_chart.settle();

    let chart_title = by_chart.app.chart_doc().title().to_string();
    let recorded = by_chart.app.layout().recents[0].clone();
    assert_eq!(
        recorded.name, chart_title,
        "the record is named nothing the opened dashboard calls itself"
    );
    assert_ne!(
        chart_title, protocol_name,
        "the two documents call themselves the same thing, so this test could \
         not tell a name read off the wrong one"
    );
    assert_eq!(
        recorded.run,
        RunState::NeverRun,
        "a composed dashboard has no run contract, so never run is what there \
         is to record"
    );

    // And what the analyst reads off the row is that same recorded name, taken
    // off the galley the row painted rather than off the record behind it.
    for (win, expected) in [
        (&mut by_protocol, protocol_name.as_str()),
        (&mut by_chart, chart_title.as_str()),
    ] {
        win.run(vec![press_home(), Vec::new()]);
        win.settle();
        let rows = win.app.front_door_rows();
        assert_eq!(rows.len(), 1, "one start was taken, so one row");
        assert_eq!(
            rows[0].name, expected,
            "the row the analyst reads is not named after the document that \
             was opened"
        );
        assert_eq!(
            rows[0].state,
            RunState::NeverRun.label(),
            "the row states a run this build never recorded"
        );
    }
}

/// A failure anywhere in a Protocol's run beats a success anywhere else.
///
/// `ProtocolModel::recorded_run_state` is the fold `open_start` records for a
/// Protocol document and the door draws beside its name — the one place the
/// per-step statuses a run emitted become the single word a row has room for.
/// Its precedence is a product claim rather than an implementation detail: one
/// step that did not produce its data is the fact worth carrying to a surface
/// with room for one word.
///
/// Asserted here rather than through a shipped start because no shipped start
/// can reach it — each is loaded with no run contract, which is the empty case
/// below. That case is the safe direction and worth its own line: a manifest
/// with no run behind it folds to `RunState::NeverRun` rather than to a state
/// a preview may present as current.
///
/// Both orders of the two-step cases are asserted. `statuses` is a
/// `BTreeMap`, so it is walked in key order, and a fold that stopped at the
/// first status it read would pass one order and fail the other.
///
/// Watched redden, one mutation: the whole fold deleted from
/// `ProtocolModel::recorded_run_state`, leaving it `RunState::NeverRun`, fails
/// at "a step that succeeded folds to something other than fresh", `NeverRun`
/// against `Fresh`.
#[test]
fn a_failed_step_anywhere_beats_a_success_anywhere() {
    fn folded(statuses: &[(&str, SeamStatus)]) -> RunState {
        let mut inputs = ProtocolInputs::empty();
        inputs.statuses = statuses
            .iter()
            .map(|(step, status)| ((*step).to_string(), *status))
            .collect();
        ProtocolModel::new(inputs, Flow::Vertical).recorded_run_state()
    }

    assert_eq!(
        folded(&[]),
        RunState::NeverRun,
        "a manifest with no run behind it reads as something other than never run"
    );
    assert_eq!(
        folded(&[("fetch", SeamStatus::NotRun)]),
        RunState::NeverRun,
        "a step the contract recorded no status for reads as a run"
    );
    assert_eq!(
        folded(&[("fetch", SeamStatus::Ok)]),
        RunState::Fresh,
        "a step that succeeded folds to something other than fresh"
    );
    assert_eq!(
        folded(&[("fetch", SeamStatus::Skipped)]),
        RunState::Fresh,
        "a skip the engine recorded as fresh folds to something other than fresh"
    );
    assert_eq!(
        folded(&[("fetch", SeamStatus::Failed)]),
        RunState::Failed,
        "a step that failed reads as something other than failed"
    );
    assert_eq!(
        folded(&[("a-fetch", SeamStatus::Failed), ("z-load", SeamStatus::Ok),]),
        RunState::Failed,
        "a success after a failure buried the failure"
    );
    assert_eq!(
        folded(&[("a-fetch", SeamStatus::Ok), ("z-load", SeamStatus::Failed),]),
        RunState::Failed,
        "a failure after a success was not carried"
    );
    assert_eq!(
        folded(&[
            ("a-fetch", SeamStatus::Skipped),
            ("z-load", SeamStatus::Failed),
        ]),
        RunState::Failed,
        "a failure beside a skip was not carried"
    );
}

/// Going home returns to the front door **without forgetting the session** —
/// the `open-home` keystroke clears both documents but leaves `layout.opened`
/// standing and the recents list intact, so the door it lands on lists the
/// work that was left and a row reopens it. That is the deliberate asymmetry
/// with `open_start` (which records both): Home keeps your place, by design.
///
/// The trip is driven by the live cmd-shift-h keystroke rather than by calling
/// `open_home` directly, so the registry binding and its wiring are on the hook
/// too — a declared-but-unwired key would leave this at "still on the dock".
///
/// Watched redden, one mutation: having `open_home` clear `layout.recents`
/// (the `open_start`-symmetric thing to do) fails here at "the door forgot the
/// session", with an empty row list against `["edgar-gleif-crosswalk"]`.
#[test]
fn going_home_returns_to_the_door_but_keeps_the_session() {
    let mut layout = default_layout();
    layout.opened = Some(starts::CROSSWALK.to_string());

    let boot = opening_boot(None, layout.opened.as_deref(), Flow::Vertical, None)
        .expect("an unnamed launch cannot fail");
    let mut win = Window {
        app: MeridianApp::headless_with_layout(boot, layout, Mode::Light),
        ctx: egui::Context::default(),
        screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
    };
    win.settle();
    assert!(
        !win.app.front_door_is_live(),
        "the launch restored its work, so the dock — not the door"
    );
    // A restore is not an open, so it records no recent: the layout this
    // window came up on remembered a start and nothing else. The work has to
    // be *taken* for the door to have a row, which is what the card click
    // below does.
    assert!(win.app.layout().recents.is_empty());

    win.run(vec![press_home(), Vec::new()]);
    win.settle();

    assert!(
        win.app.front_door_is_live(),
        "cmd-shift-h left the window on the dock — the binding is declared but not wired"
    );
    assert_eq!(
        win.app.layout().opened.as_deref(),
        Some(starts::CROSSWALK),
        "going home forgot what to restore next launch"
    );

    // Take the crosswalk from the door, come Home again, and the row for it is
    // there: `open_start` records and `open_home` does not un-record.
    win.take_the_card(starts::CROSSWALK);
    win.settle();
    win.run(vec![press_home(), Vec::new()]);
    win.settle();
    assert_eq!(
        win.app
            .front_door_rows()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec![starts::CROSSWALK],
        "the door forgot the session — the Protocols section has nothing to reopen"
    );

    // And the row still reopens exactly what was left.
    win.take_the_row(starts::CROSSWALK);
    win.settle();
    assert!(
        win.app.protocol_model().has_assets(),
        "a Protocols row after Home reopened nothing"
    );
    assert!(!win.app.front_door_is_live());
}

// ---------------------------------------------------------------------------
// Pixels: the door's baselines, and the thumbnail regeneration gate
// ---------------------------------------------------------------------------
//
// Everything below renders through the crate's real capture path
// (`capture::capture_png_at`) on a real wgpu adapter, exactly as
// `surfaces.rs` does, and diffs with the same `kittest.toml` thresholds and
// `UPDATE_SNAPSHOTS=1` workflow. It lives here rather than there because the
// door and its gallery are this file's subject.

use brightfield_shell::capture::{capture_png_at, capture_png_at_with_layout, thumbnail};

/// The size the front door is photographed at: the default window geometry,
/// because an empty boot is the one boot with no content to derive a size
/// from — that is `WindowGeometry::default()`'s job at runtime, and the trap
/// the explicit-size capture entry point exists for (an empty boot's content
/// self-measures a few tens of points).
const DOOR_SIZE: (f32, f32) = (1280.0, 820.0);

/// Where a capture's intermediate PNG goes: per-name, under the target dir,
/// as `surfaces.rs` does it.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.capture.png"))
}

fn read_rgba(path: &PathBuf) -> image::RgbaImage {
    image::open(path)
        .unwrap_or_else(|e| panic!("read capture {}: {e}", path.display()))
        .to_rgba8()
}

/// Photograph the front door — the whole window over `Boot::empty()` at the
/// default geometry — and diff it against the committed baseline.
fn door_surface(mode: Mode, name: &str) {
    // Hermetic capture: a dev shell with `BRIGHTFIELD_DEVTOOLS` set must not
    // bake the top-bar renderer string into this golden. Same class of process
    // env the offline gate owns.
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let out = scratch(name);
    let (w, h) = capture_png_at(Boot::empty(), mode, 1.0, DOOR_SIZE, &out, Vec::new())
        .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    egui_kittest::image_snapshot(&read_rgba(&out), name);
}

#[test]
fn front_door_light_surface() {
    door_surface(Mode::Light, "front_door_light");
}

#[test]
fn front_door_dark_surface() {
    door_surface(Mode::Dark, "front_door_dark");
}

/// Photograph the **returning** door — the same window over the same empty
/// boot, with a layout that remembers three Protocols — and diff it against
/// the committed baseline.
///
/// The second of the door's two states, and the one the interaction tests
/// above cannot photograph: a rect hook says a row was laid out somewhere, and
/// only pixels say the name, the run state and the time landed in three
/// columns that line up rather than on top of each other. The first-run pair
/// beside it pins the other state, so the four together are a baseline in both
/// themes for both of the door's states.
///
/// The two differ only in the layout handed in, which is what makes a
/// difference between them attributable to the recents.
fn returning_door_surface(mode: Mode, name: &str) {
    // Hermetic capture, as `door_surface`: a dev shell with
    // `BRIGHTFIELD_DEVTOOLS` set must not bake the top-bar renderer string
    // into this golden.
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let out = scratch(name);
    let (w, h) = capture_png_at_with_layout(
        Boot::empty(),
        layout_remembering(&returning_recents()),
        mode,
        1.0,
        DOOR_SIZE,
        &out,
        Vec::new(),
    )
    .unwrap_or_else(|e| panic!("capture {name}: {e}"));
    assert!(w > 0 && h > 0, "{name}: empty capture");
    egui_kittest::image_snapshot(&read_rgba(&out), name);
}

#[test]
fn front_door_return_light_surface() {
    returning_door_surface(Mode::Light, "front_door_return_light");
}

#[test]
fn front_door_return_dark_surface() {
    returning_door_surface(Mode::Dark, "front_door_return_dark");
}

/// Every bundled start still renders, and its committed thumbnail is what the
/// bundled spec still renders to.
///
/// Two gates in one pass, deliberately inseparable. The capture is the render
/// gate: a start whose spec stops composing, or whose window comes up blank,
/// fails right here — bundled starts are shipped product surface, so a start
/// that stops rendering is a shipped defect, not an example gone stale. The
/// diff is the drift gate: the committed `assets/starts/*.png` is regenerated
/// from the spec and held to the same perceptual thresholds as every other
/// baseline, so the gallery can never show a picture of something the click
/// no longer opens. Regenerate with `UPDATE_SNAPSHOTS=1` after a deliberate
/// change to a starter spec or to the renderer, and re-commit the PNGs —
/// they are pre-rendered on purpose; the door never renders them live.
///
/// A start that declares [`starts::Start::remote`] is rendered by the
/// `#[ignore]`d sibling below instead, for the reason
/// `every_shipped_start_loads_into_a_document_with_something_in_it` gives: its
/// picture cannot be redrawn without fetching someone else's server, and this
/// suite renders on every push. Both halves share [`render_thumbnails`], so the
/// two gates cannot drift into different definitions of "current".
#[test]
fn every_shipped_start_still_renders_and_its_thumbnail_is_current() {
    let drawn = render_thumbnails(
        &starts::STARTS
            .iter()
            .filter(|s| !s.remote)
            .collect::<Vec<_>>(),
        Mode::Light,
    );
    assert!(
        drawn >= 3,
        "only {drawn} thumbnail(s) were held against their specs — the \
         `remote` exemption has taken over the gate"
    );
}

/// The same gate, over the dark thumbnails: [`Start::thumbnail_dark`] is a
/// second, independently rendered PNG per start, not the light one recoloured
/// at draw time, so it needs its own render-and-diff pass held against the
/// same specs — over [`capture::capture_png_at`]'s dark path rather than a
/// palette swap.
///
/// [`Start::thumbnail_dark`]: starts::Start::thumbnail_dark
#[test]
fn every_shipped_start_still_renders_and_its_dark_thumbnail_is_current() {
    let drawn = render_thumbnails(
        &starts::STARTS
            .iter()
            .filter(|s| !s.remote)
            .collect::<Vec<_>>(),
        Mode::Dark,
    );
    assert!(
        drawn >= 3,
        "only {drawn} dark thumbnail(s) were held against their specs — the \
         `remote` exemption has taken over the gate"
    );
}

/// The same gate for the starts whose picture cannot be redrawn offline.
///
/// `cargo +1.95.0 test -p brightfield-shell --test front_door -- --ignored`,
/// and with `UPDATE_SNAPSHOTS=1` to regenerate. Run it after touching
/// `examples/remote/**` or the renderer, and commit the PNG it writes.
#[test]
#[ignore = "network: renders a start whose spec fetches over https"]
fn every_remote_start_still_renders_and_its_thumbnail_is_current() {
    let drawn = render_thumbnails(
        &starts::STARTS
            .iter()
            .filter(|s| s.remote)
            .collect::<Vec<_>>(),
        Mode::Light,
    );
    assert!(
        drawn > 0,
        "no shipped start needs a network any more — delete this test and the \
         `remote` exemption in its sibling with it"
    );
}

/// The dark half of the network-gated gate above — same starts, same
/// [`UPDATE_SNAPSHOTS`] workflow, [`Mode::Dark`] instead.
#[test]
#[ignore = "network: renders a start whose spec fetches over https"]
fn every_remote_start_still_renders_and_its_dark_thumbnail_is_current() {
    let drawn = render_thumbnails(
        &starts::STARTS
            .iter()
            .filter(|s| s.remote)
            .collect::<Vec<_>>(),
        Mode::Dark,
    );
    assert!(
        drawn > 0,
        "no shipped start needs a network any more — delete this test and the \
         `remote` exemption in its sibling with it"
    );
}

/// Render each start at the window it asks for, thumbnail it, and diff against
/// the committed PNG for `mode` — `{id}.png` for [`Mode::Light`], `{id}-dark.png`
/// for [`Mode::Dark`], so the two halves of the set stay apart by name.
/// Returns how many were held, so a caller filtering the set can refuse to
/// pass over an empty one.
fn render_thumbnails(starts: &[&'static starts::Start], mode: Mode) -> usize {
    // Hermetic capture: keep `BRIGHTFIELD_DEVTOOLS` from baking the top-bar
    // renderer string into a regenerated thumbnail (see `door_surface`).
    std::env::remove_var(brightfield_shell::devtools::DEVTOOLS_VAR);
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/starts");
    let options = egui_kittest::SnapshotOptions::default().output_path(&assets);
    let mut failures = Vec::new();
    for start in starts {
        let boot = Boot::start(start.id, Flow::Vertical)
            .unwrap_or_else(|e| panic!("{} no longer loads: {e}", start.id));
        // The window the start itself asks for — its content's own natural
        // size, the same answer the live click gives.
        let size = boot.window_size();
        let out = scratch(&format!("thumb-{}-{mode:?}", start.id));
        let (w, h) = capture_png_at(boot, mode, 1.0, size, &out, Vec::new())
            .unwrap_or_else(|e| panic!("{} no longer renders: {e}", start.id));
        assert!(w > 0 && h > 0, "{}: empty capture", start.id);
        let thumb = thumbnail(&read_rgba(&out), 480, 300);
        let name = match mode {
            Mode::Light => start.id.to_string(),
            Mode::Dark => format!("{}-dark", start.id),
        };
        if let Err(e) = egui_kittest::try_image_snapshot_options(&thumb, name, &options) {
            failures.push(format!("{}: {e}", start.id));
        }
    }
    assert!(
        failures.is_empty(),
        "{mode:?} thumbnails have drifted from what their specs render:\n{}",
        failures.join("\n")
    );
    starts.len()
}

/// No pixel in any of the five cards' **image area** is within [`TOLERANCE`]
/// of `INK_LIGHT.surface`, in the committed dark baseline — the defect this
/// card exists to fix (`front_door_dark.png` used to draw its chrome
/// correctly dark and then show five light cards).
///
/// Scans each card's image sub-rect pixel by pixel — `card.min + (SPACE_2,
/// SPACE_2)`, sized `card.width() - 2*SPACE_2` by `CARD_IMAGE_HEIGHT` — not
/// one sampled point. A single point is not enough: `capture::thumbnail`
/// (`src/capture.rs:606`) letterboxes a capture that is not exactly 16:10
/// with **transparent** padding, and `door_card` composites that padding
/// over the card's own fill (`sem.surfaces.raised`), not over the
/// thumbnail's ink (`capture.rs:600`, `window.rs:2881-2884`) — so a point
/// chosen inside that band reads the card's fill instead of shipped
/// thumbnail content. `edgar-gleif-crosswalk-dark.png`,
/// `signals-dashboard-dark.png` and `edgar-gleif-crosswalk-chart-dark.png`
/// each carry a letterbox band tall enough to swallow a single point sampled
/// a few pixels below the image's top edge; a full-area scan cannot miss the
/// same way, wherever that band happens to fall.
///
/// `SPACE_2` is [`meridian_design::spacing::SPACE_2`] — the same shared
/// design token `door_card`'s own `img_rect` is built from
/// (`window.rs:2882`), read here rather than duplicated. `card.width()` comes
/// from the rect [`front_door_card_rect`] already returns, leaving
/// `CARD_IMAGE_HEIGHT` (130.0) as the one number duplicated from `window.rs`,
/// which has no public accessor to read instead; scanning a shade short of
/// the true image height costs a thin strip of the card's own fill at the
/// bottom (still not light), and scanning a shade past it stops short of
/// where the card's label text starts (`SPACE_4` further down), so an
/// approximate value here cannot turn a real defect invisible.
///
/// Measured on the real committed baseline: 0 of 27,040 pixels per card
/// (208 × 130) are within [`TOLERANCE`] of `INK_LIGHT.surface`, on each of
/// the five cards. Measured under the mutation below (each card drawn from
/// its light slice, in `starts::STARTS` order): 68.2% / 39.6% / 67.7% /
/// 68.1% / 49.4% of each card's pixels are — the margin either side of
/// [`TOLERANCE`] is not a close call.
///
/// [`front_door_card_rect`]: brightfield_shell::window::MeridianApp::front_door_card_rect
///
/// Watched redden, one mutation: pointing `ensure_door_thumbs` at
/// `start.thumbnail` (the light slice) instead of
/// `start.thumbnail_for(self.mode)` and regenerating the baseline — each
/// card is now wrong, and the loop panics at `edgar-gleif-crosswalk`, the
/// FIRST entry in `starts::STARTS`, because the scan finds a bad pixel
/// wherever one falls rather than where one particular sampled point
/// happened to land.
const TOLERANCE: i32 = 20;

/// The image sub-rect `door_card` draws each thumbnail into — see the test
/// above for why this, and not the whole card, is what gets scanned.
fn card_image_rect(card: egui::Rect) -> egui::Rect {
    const CARD_IMAGE_HEIGHT: f32 = 130.0;
    let pad = meridian_design::spacing::SPACE_2;
    egui::Rect::from_min_size(
        card.min + egui::vec2(pad, pad),
        egui::vec2(card.width() - 2.0 * pad, CARD_IMAGE_HEIGHT),
    )
}

#[test]
fn no_front_door_card_shows_the_light_surface_family_in_the_dark_baseline() {
    let mut win = Window::open(Boot::empty());
    win.settle();

    let baseline =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/front_door_dark.png");
    let img = image::open(&baseline)
        .unwrap_or_else(|e| panic!("read {}: {e}", baseline.display()))
        .to_rgba8();

    let s = meridian_design::chrome::INK_LIGHT.surface;
    let light_rgb = [
        (s.r * 255.0).round() as i32,
        (s.g * 255.0).round() as i32,
        (s.b * 255.0).round() as i32,
    ];

    for start in starts::STARTS {
        let card = win
            .app
            .front_door_card_rect(start.id)
            .unwrap_or_else(|| panic!("the door drew no card for {}", start.id));
        let image = card_image_rect(card);
        let x0 = image.min.x.round() as u32;
        let y0 = image.min.y.round() as u32;
        let x1 = image.max.x.round() as u32;
        let y1 = image.max.y.round() as u32;

        let mut near_light = 0u32;
        let mut scanned = 0u32;
        let mut first_bad = None;
        for y in y0..y1 {
            for x in x0..x1 {
                let px = img.get_pixel(x, y);
                let sampled = [i32::from(px[0]), i32::from(px[1]), i32::from(px[2])];
                scanned += 1;
                let within = sampled
                    .iter()
                    .zip(light_rgb)
                    .all(|(&channel, light)| (channel - light).abs() <= TOLERANCE);
                if within {
                    near_light += 1;
                    first_bad.get_or_insert((x, y, sampled));
                }
            }
        }
        assert_eq!(
            near_light, 0,
            "{}'s card has {near_light} of {scanned} image pixels within \
             {TOLERANCE} of INK_LIGHT.surface {light_rgb:?} — first at \
             {first_bad:?} — so the dark front door is still showing (at \
             least partly) a light thumbnail",
            start.id
        );
    }
}
