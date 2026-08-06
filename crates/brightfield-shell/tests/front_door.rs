//! The front door: what a launch that named nothing shows, and what the
//! second click does.
//!
//! The empty state itself was already shipped and already gated — every pane
//! of both views answers an empty document with one, and
//! `brightfield_workbench::audit` is what keeps that true. The mechanism that
//! made it *reachable* — `Boot::empty()` on a no-argument launch, the shipped
//! starts, `EmptyState::with_next` on the view-filling panes — landed next.
//! What is asserted here is the content built on top of it: a window with
//! nothing open draws the front door in place of the dock — Welcome, Start,
//! Continue, Explore, Learn — the Explore gallery offers every start the
//! binary ships, and taking any card lands on a rendered result rather than
//! on an instrument.
//!
//! Interaction tests are GPU-free. `MeridianApp::headless` has no device, so
//! neither canvas pane paints; every rect is the same either way, and each
//! document reports what it holds without needing a texture. The pixel
//! section at the bottom is the exception — the door's two baselines and the
//! thumbnail regeneration gate render through the real capture path, which
//! needs a wgpu adapter, exactly as `surfaces.rs` does.

use std::path::PathBuf;

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::{chart_registry, ChartDoc, CHART};
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::{self, protocol_registry, ProtocolDoc, CANVAS};
use brightfield_shell::starts::{self, Opened};
use brightfield_shell::startup::{default_layout, opening_boot};
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::{Action, PaneKey, Subject, ViewKind};

const CHART_PANE: PaneKey = PaneKey::new(ViewKind::Charts, CHART);
const CANVAS_PANE: PaneKey = PaneKey::new(ViewKind::Protocol, CANVAS);

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

    /// Reach the other view the way a person does: the top bar's switcher.
    fn switch_to(&mut self, view: ViewKind) {
        let target = self
            .app
            .switcher_rect(view)
            .expect("the top bar drew a switcher control");
        self.run(vec![click_at(target.center()), Vec::new()]);
        assert_eq!(self.app.active(), view);
    }
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
        assert_eq!(
            opened.view(),
            start.view,
            "{} is offered for the {:?} view but loads a document for {:?}",
            start.id,
            start.view,
            opened.view()
        );
        match opened {
            Opened::Charts(composed) => assert!(
                composed.width > 0 && composed.height > 0,
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
/// each decodes to the gallery card's own 16:10.
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
        let path = dir.join(format!("{}.png", start.id));
        let committed = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("{} has no committed thumbnail: {e}", start.id));
        assert_eq!(
            committed,
            start.thumbnail,
            "{}'s embedded thumbnail is not the committed {} — rebuild, or \
             fix the include path",
            start.id,
            path.display()
        );
        let decoded = image::load_from_memory(start.thumbnail)
            .unwrap_or_else(|e| panic!("{}'s thumbnail does not decode: {e}", start.id));
        assert_eq!(
            (decoded.width(), decoded.height()),
            (480, 300),
            "{}'s thumbnail is not the gallery card's 480×300",
            start.id
        );
    }
}

// ---------------------------------------------------------------------------
// Reaching it, and what the second click does
// ---------------------------------------------------------------------------

/// A launch that named nothing opens the front door: every shipped start's
/// card is on it, clickable, on either view — and nothing about it is a
/// dismissal to find, because there is nothing to dismiss.
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
        win.app.front_door_continue_rect().is_none(),
        "a first run drew a Continue zone with no history to continue"
    );
    assert!(
        win.app.front_door_help_rect().is_some(),
        "the Start zone offers nothing to do"
    );

    // The same door on the other view: the door belongs to the window, not
    // to a view, so the switcher changes the active view and nothing else.
    win.switch_to(ViewKind::Protocol);
    win.settle();
    for start in starts::STARTS {
        assert!(
            win.app.front_door_card_rect(start.id).is_some(),
            "switching views lost {}'s card",
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
    assert_eq!(
        win.app.active(),
        ViewKind::Charts,
        "opening a chart start did not land on the view it fills"
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
    assert_eq!(win.app.active(), ViewKind::Charts);
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
    assert_eq!(
        win.app.active(),
        ViewKind::Protocol,
        "the crosswalk opened without landing on the view it fills"
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
    // What a real file left by that session says: the start that was open,
    // *and* the view the window was on when it was closed. The second is not
    // derivable from the first — see `startup::opening_boot` — so the fixture
    // has to carry it rather than let a start choose it.
    layout.workspace.set_active(ViewKind::Protocol);

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

    assert_eq!(win.app.active(), ViewKind::Protocol);
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

/// The morph's other face: a door drawn while the layout remembers work grows
/// a Continue zone, and its control reopens exactly what was remembered.
///
/// Today every `main` launch with something remembered restores it before the
/// door can draw — `opening_boot`'s precedence, held by the test above — so
/// this state is reached when the two disagree: a remembered id the restore
/// dropped, or the returnable-Home path when it lands (the door reopened over
/// a session that has work). The zone is built against the layout rather than
/// against either trigger, which is why it can be held here without faking
/// one.
#[test]
fn a_door_with_history_grows_a_continue_zone_that_reopens_it() {
    let mut layout = default_layout();
    layout.opened = Some(starts::CROSSWALK.to_string());
    let mut win = Window {
        app: MeridianApp::headless_with_layout(Boot::empty(), layout, Mode::Light),
        ctx: egui::Context::default(),
        screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
    };
    win.settle();

    assert!(win.app.front_door_is_live(), "nothing open, so the door");
    let control = win
        .app
        .front_door_continue_rect()
        .expect("history exists, so the door grows a Continue zone");
    assert!(win.screen.contains_rect(control));
    win.run(vec![click_at(control.center()), Vec::new()]);
    win.settle();

    assert!(
        win.app.protocol_model().has_assets(),
        "Continue reopened nothing"
    );
    assert_eq!(win.app.active(), ViewKind::Protocol);
    assert!(!win.app.front_door_is_live());
}

/// Going home returns to the front door **without forgetting the session** —
/// the `open-home` keystroke clears both documents but leaves `layout.opened`
/// standing, so the door it lands on grows the Continue zone that reopens what
/// was left. That is the deliberate asymmetry with `open_start` (which records
/// `opened`): Home keeps your place, by design.
///
/// The trip is driven by the live cmd-shift-h keystroke rather than by calling
/// `open_home` directly, so the registry binding and its wiring are on the hook
/// too — a declared-but-unwired key would leave this at "still on the dock".
///
/// Watched redden, one mutation: having `open_home` null `layout.opened` (the
/// `open_start`-symmetric thing to do) fails here at "the door forgot the
/// session" — the Continue zone vanishes and there is no way back to the work.
#[test]
fn going_home_returns_to_the_door_but_keeps_the_session() {
    let mut layout = default_layout();
    layout.opened = Some(starts::CROSSWALK.to_string());
    layout.workspace.set_active(ViewKind::Protocol);

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

    win.run(vec![press_home(), Vec::new()]);
    win.settle();

    assert!(
        win.app.front_door_is_live(),
        "cmd-shift-h left the window on the dock — the binding is declared but not wired"
    );
    assert_eq!(
        win.app.layout().opened.as_deref(),
        Some(starts::CROSSWALK),
        "going home forgot the session — the Continue zone has nothing to reopen"
    );
    let control = win
        .app
        .front_door_continue_rect()
        .expect("the kept session should grow a Continue zone on the door");
    assert!(win.screen.contains_rect(control));

    // And Continue still reopens exactly what was left.
    win.run(vec![click_at(control.center()), Vec::new()]);
    win.settle();
    assert!(
        win.app.protocol_model().has_assets(),
        "Continue after Home reopened nothing"
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

use brightfield_shell::capture::{capture_png_at, thumbnail};

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
    );
    assert!(
        drawn >= 3,
        "only {drawn} thumbnail(s) were held against their specs — the \
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
    );
    assert!(
        drawn > 0,
        "no shipped start needs a network any more — delete this test and the \
         `remote` exemption in its sibling with it"
    );
}

/// Render each start at the window it asks for, thumbnail it, and diff against
/// the committed PNG. Returns how many were held, so a caller filtering the
/// set can refuse to pass over an empty one.
fn render_thumbnails(starts: &[&'static starts::Start]) -> usize {
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
        let size = boot.window_size(boot.view_or(ViewKind::Charts));
        let out = scratch(&format!("thumb-{}", start.id));
        let (w, h) = capture_png_at(boot, Mode::Light, 1.0, size, &out, Vec::new())
            .unwrap_or_else(|e| panic!("{} no longer renders: {e}", start.id));
        assert!(w > 0 && h > 0, "{}: empty capture", start.id);
        let thumb = thumbnail(&read_rgba(&out), 480, 300);
        if let Err(e) = egui_kittest::try_image_snapshot_options(&thumb, start.id, &options) {
            failures.push(format!("{}: {e}", start.id));
        }
    }
    assert!(
        failures.is_empty(),
        "thumbnails have drifted from what their specs render:\n{}",
        failures.join("\n")
    );
    starts.len()
}
