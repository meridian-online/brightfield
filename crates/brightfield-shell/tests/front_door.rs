//! The front door: what a launch that named nothing shows, and what the
//! second click does.
//!
//! The empty state itself was already shipped and already gated — every pane
//! of both views answers an empty document with one, and
//! `brightfield_workbench::audit` is what keeps that true. It was merely
//! **unreachable**: with no argument the binary opened a hardcoded example, so
//! nobody ever saw it. What is asserted here is the half that is new — that
//! there is a way in, that a person can actually reach it, and that taking it
//! lands on a rendered result rather than on an instrument.
//!
//! All GPU-free. `MeridianApp::headless` has no device, so neither canvas pane
//! paints; every rect is the same either way, and each document reports what
//! it holds without needing a texture.

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

    /// Click the button `pane`'s empty state drew, aimed at the rect the last
    /// frame recorded rather than at a coordinate typed here.
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
/// something in it, from the bytes in the binary.
///
/// This is what `include_str!` cannot check. It proves the fixture is present
/// at compile time and nothing else — a spec that composed no plots would
/// still build and would still put a button on the front door that resolves to
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
/// root, so a start that read `examples/` relative to the process would fail
/// here.
#[test]
fn every_shipped_start_loads_into_a_document_with_something_in_it() {
    assert!(
        !starts::STARTS.is_empty(),
        "nothing ships, so nothing starts"
    );
    for start in starts::STARTS {
        let opened = starts::load(start.id)
            .unwrap_or_else(|e| panic!("the shipped start {} does not load: {e}", start.id));
        assert_eq!(
            opened.view(),
            start.view,
            "{} is offered by the {:?} view but loads a document for {:?}",
            start.id,
            start.view,
            opened.view()
        );
        match opened {
            Opened::Charts(composed) => assert!(
                composed.width > 0 && composed.height > 0,
                "{} composed no plots, so the button that opens it resolves \
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

/// The front door's button claims no keystroke, because it has none.
///
/// The chrome renders an affordance's verb's *real* keystroke beside its
/// label, straight from the keyboard registry. There is no registered command
/// meaning "open the example dashboard", so an affordance built with a
/// borrowed verb would ship a button reading `Open the example dashboard
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

// ---------------------------------------------------------------------------
// Reaching it, and what the second click does
// ---------------------------------------------------------------------------

/// A launch that named nothing opens a window, and both views offer a way in.
///
/// The state this replaces did not have a window to assert about: with no
/// argument the binary read a hardcoded `examples/dashboard.yaml`, and from any
/// directory but the repo root that is a read error before `run_native` is
/// reached.
#[test]
fn an_empty_launch_opens_a_window_that_offers_a_way_in_on_both_views() {
    let mut win = Window::open(Boot::empty());
    win.settle();

    assert!(
        win.app.chart_doc().is_empty(),
        "an empty launch composed a dashboard from somewhere"
    );
    assert!(
        !win.app.protocol_model().has_assets(),
        "an empty launch built a graph from somewhere"
    );
    assert!(
        win.app.affordance_rect(CHART_PANE).is_some(),
        "the chart view's front door offers nothing to do"
    );

    win.switch_to(ViewKind::Protocol);
    win.settle();
    assert!(
        win.app.affordance_rect(CANVAS_PANE).is_some(),
        "the protocol view's front door offers nothing to do"
    );
}

/// The second click lands on a **rendered dashboard**, and the front door is
/// gone because the pane is full rather than because anything dismissed it.
///
/// Watched redden, two mutations: dropping the `Request::Open` arm from
/// `MeridianApp::apply` — the arm the charts view used to have as `{}`, which
/// is exactly how a front door ships as chrome that does nothing — fails at
/// "the click opened nothing"; and having `open_start` set the document
/// without recording the id fails at the `opened` assertion, which is the half
/// that makes a later launch restore it.
#[test]
fn the_way_in_on_the_chart_view_lands_on_a_rendered_dashboard() {
    let mut win = Window::open(Boot::empty());
    win.settle();
    win.take_the_way_in(CHART_PANE);
    win.settle();

    assert!(
        !win.app.chart_doc().is_empty(),
        "the click opened nothing — the chart pane is still empty, which is a \
         front door that has moved the blank canvas rather than removed it"
    );
    assert_eq!(
        win.app.active(),
        ViewKind::Charts,
        "opening a chart switched the view out from under the click"
    );
    assert!(
        chart_subject(win.app.chart_doc()).empty_state.is_none(),
        "the chart pane still declares itself empty over a composed dashboard"
    );
    assert!(
        win.app.affordance_rect(CHART_PANE).is_none(),
        "the front door is still drawn over content"
    );
    assert_eq!(
        win.app.layout().opened.as_deref(),
        Some(starts::DASHBOARD),
        "nothing recorded what was opened, so the next launch cannot restore it"
    );
}

/// The same on the protocol view, reached the way a person reaches it, landing
/// on a **built asset graph**: the outline, the steps sheet and the inspector
/// all have content behind them, not just the canvas.
#[test]
fn the_way_in_on_the_protocol_view_lands_on_a_rendered_graph() {
    let mut win = Window::open(Boot::empty());
    win.settle();
    win.switch_to(ViewKind::Protocol);
    win.settle();
    win.take_the_way_in(CANVAS_PANE);
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
    assert_eq!(win.app.active(), ViewKind::Protocol);
    assert!(
        win.app.affordance_rect(CANVAS_PANE).is_none(),
        "the front door is still drawn over a graph"
    );
    assert_eq!(win.app.layout().opened.as_deref(), Some(starts::CROSSWALK));
}

/// A launch with work to restore restores it and shows **no** front door.
///
/// This is the morph, and it is why the layout has to carry what was open
/// rather than only where the panes are: restoring an arrangement alone would
/// come up with the user's splitter positions around panes that are all still
/// empty, and every one of them would go on inviting a first action. There is
/// no "don't show this again" anywhere — the surface simply stops being an
/// invitation once it has content.
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
    let boot = opening_boot(None, layout.opened.as_deref(), Flow::Vertical)
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
        win.app.affordance_rect(CANVAS_PANE).is_none(),
        "a launch that restored its work still showed the front door"
    );
    assert!(
        canvas_subject(win.app.protocol_doc()).empty_state.is_none(),
        "the canvas pane declares itself empty over a restored graph"
    );
}
