//! The window that holds both views, without a GPU.
//!
//! `chart_contract.rs` and `protocol_contract.rs` each pin one view's
//! declaration. This file pins what only exists because the two are in one
//! window: which view a spec opens, the layout vocabulary a window publishes
//! being *both* views' rather than one, the size that window asks for on the
//! view it opened, and the gate that keeps a bare-key grammar belonging to one
//! view from driving it while the other is on screen.
//!
//! All GPU-free. `MeridianApp::headless` has no device, so neither canvas pane
//! paints — but both are handed the same rects either way, and each records the
//! box it was given before it looks for a texture.

use brightfield_protocol::layout::Flow;
use brightfield_shell::app::chart_registry;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::{
    load_protocol_offline, protocol_registry, ProtocolInputs, ProtocolModel,
};
use brightfield_shell::window::{protocol_window_size, Boot, MeridianApp};
use brightfield_workbench::{ItemId, PaneKey, ViewKind};

const DASHBOARD: &str = "../../examples/dashboard.yaml";
const EDGAR: &str = "../../examples/protocol/edgar_gleif/arcform.yaml";
/// The gate `Boot::open` gives a Protocol manifest with no emitted run.
const OFFLINE: &str = "BRIGHTFIELD_PROTOCOL_OFFLINE";

/// A window under test: the app, the size it asked for, and **one**
/// `egui::Context` for its whole life.
///
/// One context, not one per call. A fresh context has no memory of the widgets
/// the last frame drew, and egui resolves a click against the widget id it
/// registered on a previous frame — so driving two `run` calls through two
/// contexts silently swallows every pointer interaction, and a test that clicks
/// a control passes or fails for reasons that have nothing to do with the
/// control. It cost one false red here before it could cost a false green.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn open(boot: Boot, mode: Mode) -> Self {
        let (w, h) = boot.window_size(boot.view_or(ViewKind::Charts));
        Self::open_at(boot, mode, egui::vec2(w, h))
    }

    /// The same window at a size the boot did not ask for — which is what a
    /// user dragging the window narrow produces, and the only way the narrow
    /// case below happens.
    fn open_at(boot: Boot, mode: Mode, size: egui::Vec2) -> Self {
        Self {
            app: MeridianApp::headless(boot, mode),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, size),
        }
    }

    /// Run one frame per entry, feeding that entry's events. Two frames
    /// minimum, because the first installs the font atlas and settles the
    /// layout — exactly as the capture path does, and as the live window does.
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

    /// Two frames of nothing happening.
    fn settle(&mut self) {
        self.run(vec![Vec::new(), Vec::new()]);
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

/// One frame's worth of a key press and release.
fn press(key: egui::Key, shift: bool) -> Vec<egui::Event> {
    let modifiers = egui::Modifiers {
        shift,
        ..Default::default()
    };
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        })
        .collect()
}

fn edgar() -> ProtocolInputs {
    load_protocol_offline(EDGAR).expect("load edgar_gleif")
}

/// A boot with **both** fixtures loaded, opening on `view`.
///
/// Not a state the CLI produces — it names one spec — but the state every
/// assertion about *switching* needs. With only one document loaded the other
/// view's panes are empty, and `PaneChrome` draws an empty state *instead of*
/// the item, so the pane whose viewport a test reads never runs its `ui` at all
/// and "it did not draw" would be true for a reason that has nothing to do with
/// the view being active.
fn both(view: ViewKind) -> Boot {
    Boot {
        view: Some(view),
        composed: compose_spec(DASHBOARD).expect("compose the dashboard"),
        protocol: edgar(),
        flow: Flow::Vertical,
        focus: None,
    }
}

// ---------------------------------------------------------------------------
// Which view a spec opens
// ---------------------------------------------------------------------------

/// A spec chooses the view the window opens on — and *only* that. Both views
/// are loaded either way.
///
/// This is what replaced a fork into two `eframe::App`s, so it reads real specs
/// through [`Boot::open`] rather than calling the two constructors it dispatches
/// to. That distinction is the whole test: the classification is the same one it
/// always was, but it now picks a field rather than a program, and a version of
/// this that called `Boot::charts` / `Boot::protocol` directly asserted nothing
/// about the classifier at all. Stubbing the sniff out — `if false &&
/// is_protocol_manifest(..)`, so no manifest can ever open — left the whole
/// crate green.
///
/// The environment gate is asserted here too, because `Boot::open` is now its
/// only home: two `fn main`s used to check it separately, and neither could be
/// reached from a test. The var is set and removed inside one test rather than
/// across two, which the mandated `--test-threads=1` makes ordered; nothing else
/// in this binary reads it, since every other fixture goes through
/// `load_protocol_offline`, which does not gate.
#[test]
fn a_spec_chooses_the_opening_view_and_both_views_are_loaded() {
    let charts = Boot::open(DASHBOARD, Flow::Vertical, None).expect("open the dashboard spec");
    assert_eq!(charts.view, Some(ViewKind::Charts));
    assert!(
        charts.protocol.graph_collapsed.nodes.is_empty(),
        "a dashboard spec loaded a protocol"
    );

    // A manifest with no emitted run needs the offline gate, and says so.
    std::env::remove_var(OFFLINE);
    let Err(refused) = Boot::open(EDGAR, Flow::Vertical, None) else {
        panic!("a bare Protocol manifest opened without the offline gate");
    };
    assert!(
        refused.contains(OFFLINE),
        "the refusal does not name the variable that lifts it: {refused}"
    );

    std::env::set_var(OFFLINE, "1");
    let protocol = Boot::open(EDGAR, Flow::Vertical, None).expect("open the manifest offline");
    std::env::remove_var(OFFLINE);

    assert_eq!(protocol.view, Some(ViewKind::Protocol));
    assert!(
        !protocol.protocol.graph_collapsed.nodes.is_empty(),
        "the protocol fixture loaded no assets, so this test proves nothing"
    );
    assert_eq!(
        protocol.composed.width, 0,
        "a protocol manifest composed a dashboard"
    );
}

/// The layout vocabulary a window publishes is **both** registries' ids and
/// nothing else.
///
/// A one-window property, and it lives here for that reason. It was written in
/// `chart_contract.rs` over the chart registry alone, which had been true when
/// only the chart shell existed in that binary; one window publishes both
/// vocabularies in `MeridianApp::assemble`, and `ItemId::known` is
/// process-global and additive, so the single-registry form was green only
/// because libtest sorts by name and it sorted before the first test that builds
/// an app. Any earlier-sorting test that built one turned it red naming four
/// protocol panes — a regression report for a thing that is correct.
///
/// Publishing both is what a `PaneKey` in a saved layout needs whichever view
/// booted: the tree of the view that did *not* boot has to deserialise too, or
/// the layout loads as corrupt. The count assertion is the other half — two
/// views sharing an id would let one view's saved pane validate as the other's.
#[test]
fn a_window_publishes_both_registries_and_nothing_else() {
    // Through the app, not through the two `publish_item_ids` entry points, so
    // this fails if `assemble` ever stops publishing the view it did not open.
    let _app = MeridianApp::headless(both(ViewKind::Charts), Mode::Light);

    let charts = chart_registry().ids();
    let protocol = protocol_registry().ids();
    let known = ItemId::known();
    for id in charts.iter().chain(protocol.iter()) {
        assert!(known.contains(id), "{id} is declared but never published");
    }
    for id in known {
        assert!(
            charts.contains(id) || protocol.contains(id),
            "{id} was published by something other than the two view registries"
        );
    }
    assert_eq!(
        known.len(),
        charts.len() + protocol.len(),
        "the two registries share an id, so a saved layout naming one view's \
         pane would validate against the other's"
    );

    // The point of the vocabulary: a saved layout naming these panes loads.
    for (view, ids) in [(ViewKind::Charts, charts), (ViewKind::Protocol, protocol)] {
        for item in ids {
            let key = PaneKey::new(view, item);
            let json = serde_json::to_string(&key).expect("a pane key serialises");
            assert_eq!(
                serde_json::from_str::<PaneKey>(&json).expect("and round trips"),
                key
            );
        }
    }
}

/// The window built from a boot opens on the view the boot named, and **only
/// that view is laid out**.
///
/// Both documents are loaded and both dock trees exist, so the cheap mistake
/// would be to run both trees every frame and show one — which costs a full
/// second layout pass and a second raster, and makes "which view is active" a
/// question about visibility rather than about work. The second assertion is
/// what rules that out: the chart pane records the box it was handed inside its
/// own `ui`, so a `None` there means nobody called it.
///
/// It says nothing about switching. That is
/// `the_top_bar_switcher_switches_the_view_the_dock_draws`, below.
#[test]
fn the_window_opens_on_the_view_the_boot_named_and_lays_out_only_that_view() {
    let mut win = Window::open(both(ViewKind::Protocol), Mode::Light);
    assert_eq!(win.app.active(), ViewKind::Protocol);
    win.settle();
    assert!(
        win.app.canvas_viewport().is_some(),
        "the DAG canvas pane never drew"
    );
    assert!(
        win.app.chart_viewport().is_none(),
        "the chart pane drew while the protocol view was active — both views \
         are laid out every frame, which is not what one window means"
    );
}

// ---------------------------------------------------------------------------
// The window the protocol view asks for
// ---------------------------------------------------------------------------

/// The window `protocol_window_size` asks for really does fit the DAG the
/// canvas pane lays out — checked by laying a **real frame** out, not by
/// re-running the same arithmetic.
///
/// The chart view has had this assertion since its window was caught clipping
/// the bottom seventeen rows of its own raster. The protocol view had nothing
/// like it, and its failure mode is quieter: `CanvasPane` wraps the raster in a
/// `ScrollArea`, so a window several points short opens the graph part-scrolled
/// and every pixel baseline photographs that perfectly happily. The window it
/// asked for was `layout.height + 130.0`, floored and clamped, with nothing
/// deriving the 130 and nothing able to see it.
///
/// Watched redden, one mutation: dropping the `TAB_BAR_HEIGHT` term from
/// `protocol_window_size` — the centre tab strip the canvas sits under — leaves
/// the canvas pane 24 points short of the DAG and fails here by that much.
#[test]
fn the_protocol_window_it_asks_for_fits_the_dag_it_lays_out() {
    let inputs = edgar();
    let layout = ProtocolModel::boot_layout(&inputs, Flow::Vertical);
    let (dag_w, dag_h) = (layout.width as f32, layout.height as f32);
    assert!(
        dag_w > 0.0 && dag_h > 0.0,
        "the fixture laid out nothing, so this test proves nothing"
    );
    let (w, h) = protocol_window_size(&layout);

    let mut win = Window::open(Boot::protocol(inputs, Flow::Vertical, None), Mode::Light);
    assert_eq!(
        (win.screen.width(), win.screen.height()),
        (w, h),
        "the boot sized the window differently from `protocol_window_size`"
    );
    win.settle();

    let box_ = win
        .app
        .canvas_viewport()
        .expect("the canvas pane drew, so it recorded the box it was given");
    assert!(
        box_.width() >= dag_w && box_.height() >= dag_h,
        "a {w}x{h} window gives the canvas pane a {:.2}x{:.2} content box, \
         and it lays out a {dag_w:.0}x{dag_h:.0} DAG into it — the graph opens \
         part-scrolled, which no baseline can tell from a graph that is large",
        box_.width(),
        box_.height(),
    );

    // And in neither axis is the leftover a fudge factor. Every term of
    // `protocol_window_size` is read from the component that consumes it, so the
    // only slack it may have is its rounding up to whole logical points. An
    // inequality any positive number satisfies is what let the chart view's
    // height budget be 95 points short while its own test stayed green.
    for (axis, slack) in [
        ("across", box_.width() - dag_w),
        ("down", box_.height() - dag_h),
    ] {
        assert!(
            slack < 1.0,
            "the canvas pane's content box has {slack:.2}pt of slack {axis} — \
             more than the sub-point rounding `protocol_window_size` is allowed, \
             so some of the budget is a fudge factor rather than a component"
        );
    }
}

// ---------------------------------------------------------------------------
// Key routing
// ---------------------------------------------------------------------------

/// The protocol grammar reaches the DAG when the protocol view is drawn.
///
/// The positive control for the test below it. Without this one, a gate that
/// swallowed *every* keystroke would pass the negative case perfectly.
#[test]
fn the_protocol_grammar_reaches_the_dag_on_its_own_view() {
    let mut win = Window::open(Boot::protocol(edgar(), Flow::Vertical, None), Mode::Light);
    assert!(
        !win.app.protocol_model().show_sheet(),
        "the sheet boots open"
    );
    win.run(vec![Vec::new(), press(egui::Key::S, true), Vec::new()]);
    assert!(
        win.app.protocol_model().show_sheet(),
        "shift-S did not open the steps sheet on the protocol view"
    );
}

/// The protocol grammar does **not** reach the DAG while the charts view is
/// drawn.
///
/// This is the one behaviour the merge had to invent rather than move. The
/// grammar is bare-key — `h j k l y t Enter Esc ⌫ shift-S`, no modifier to
/// disambiguate it — and the panel used to read `ctx.input` unconditionally
/// because it owned a whole window. In one window that would fold a family,
/// drill a scope or open a steps sheet under a user who is looking at a chart.
///
/// Watched redden, one mutation: removing the `view == ViewKind::Protocol`
/// guard around `feed_events` in `MeridianApp::draw` fails here with *"shift-S
/// reached the DAG while the charts view was drawn"*.
#[test]
fn the_protocol_grammar_does_not_reach_the_dag_from_the_charts_view() {
    let mut win = Window::open(both(ViewKind::Charts), Mode::Light);
    win.run(vec![Vec::new(), press(egui::Key::S, true), Vec::new()]);
    assert_eq!(win.app.active(), ViewKind::Charts);
    assert!(
        !win.app.protocol_model().show_sheet(),
        "shift-S reached the DAG while the charts view was drawn"
    );
}

// ---------------------------------------------------------------------------
// The switcher
// ---------------------------------------------------------------------------

/// The top bar's switcher actually switches — clicked where it drew itself, at
/// the window's natural size **and at a window narrow enough to have broken
/// it**.
///
/// This is the increment's user-facing claim, and it is the one that is easiest
/// to assert and never check: `Workspace::set_active` can be called from a test
/// all day without there being any way for a person to reach it. So the click
/// is aimed at the rect the bar recorded on the previous frame rather than at a
/// coordinate typed here, and the assertion is that the *dock* followed — the
/// chart pane records a content box only on a frame the charts view drew.
///
/// The narrow case is not decoration. Aiming at the recorded rect is not enough
/// on its own: the bar's right-to-left group draws from the window's right edge
/// leftwards, so on a narrow window it lands over a switcher that is still
/// exactly where it says it is, and egui hands the click to the label on top.
/// The switcher went on drawing and stopped switching under about 376 logical
/// points of window width, with a green suite — this test at its old single
/// width could not fail that way, because the protocol boot only ever ran it at
/// 1978.
///
/// Watched redden, one mutation: dropping the `available_width` gate in
/// `MeridianApp::top_bar` so the right-hand group always draws fails the 240pt
/// case with *"the switcher is chrome that does nothing"* and leaves the 1978pt
/// case green.
#[test]
fn the_top_bar_switcher_switches_the_view_the_dock_draws() {
    let natural = both(ViewKind::Protocol).window_size(ViewKind::Protocol);
    for size in [egui::vec2(natural.0, natural.1), egui::vec2(240.0, 400.0)] {
        let mut win = Window::open_at(both(ViewKind::Protocol), Mode::Light, size);
        win.settle();
        assert_eq!(win.app.active(), ViewKind::Protocol);
        assert!(
            win.app.chart_viewport().is_none(),
            "at {size:?} the chart pane drew before the view was switched to it"
        );

        let target = win
            .app
            .switcher_rect(ViewKind::Charts)
            .expect("the top bar drew a switcher control for the charts view");
        assert!(
            win.screen.contains_rect(target),
            "at {size:?} the charts control drew at {target:?}, outside the \
             window — nothing could click it"
        );
        win.run(vec![click_at(target.center()), Vec::new()]);

        assert_eq!(
            win.app.active(),
            ViewKind::Charts,
            "at {size:?}, clicking the charts control at {target:?} left the \
             window on the protocol view — the switcher is chrome that does \
             nothing"
        );
        assert!(
            win.app.chart_viewport().is_some(),
            "at {size:?} the window says it is on the charts view but the \
             chart pane never drew"
        );
    }
}
