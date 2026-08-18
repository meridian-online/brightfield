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

use brightfield_protocol::layout::{Flow, LayoutConfig};
use brightfield_shell::app::chart_registry_with;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::{
    load_protocol_offline, protocol_registry, ProtocolInputs, ProtocolModel,
};
use brightfield_shell::window::{
    fit_window_to_display, protocol_window_size_for, window_size_on_display, Boot, DisplayFit,
    MeridianApp,
};
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
        live: None,
        spec_path: Some(DASHBOARD.into()),
        authored: None,
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
///
/// The chart side is compared in its gallery-*inclusive* form
/// (`chart_registry_with(true)`), because that is deliberately what
/// `publish_item_ids` publishes whatever the dev flag says: a layout saved
/// while the gallery flag was on names its pane, and an id that stops being
/// published makes that whole file unloadable. The *item* stays flag-gated;
/// only the vocabulary is a superset.
#[test]
fn a_window_publishes_both_registries_and_nothing_else() {
    // Through the app, not through the two `publish_item_ids` entry points, so
    // this fails if `assemble` ever stops publishing the view it did not open.
    let _app = MeridianApp::headless(both(ViewKind::Charts), Mode::Light);

    let charts = chart_registry_with(true).ids();
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

/// Two real displays, in logical points, that this window has to open onto.
///
/// Not thresholds and not policy — [`window_size_on_display`] takes whatever it
/// is handed. They are here so the assertions below are about a screen someone
/// actually uses rather than a number chosen to make them pass: `LAPTOP` is the
/// built-in Retina panel of the machine this product is developed and demoed
/// on, and `ULTRAWIDE` is the external display beside it, which is where the
/// protocol view's screenshots have been taken. The defect these tests exist
/// for is exactly the gap between them — a window sized on the second and
/// unopenable on the first.
const LAPTOP: (f32, f32) = (1512.0, 982.0);
const ULTRAWIDE: (f32, f32) = (3440.0, 1440.0);

/// The window `protocol_window_size` asks for really does fit **every state it
/// is sized for** — checked by laying a **real frame** out, not by re-running
/// the same arithmetic.
///
/// The chart view has had this assertion since its window was caught clipping
/// the bottom seventeen rows of its own raster. The protocol view had nothing
/// like it, and its failure mode is quieter: `CanvasPane` wraps the raster in a
/// `ScrollArea`, so a window several points short opens the graph part-scrolled
/// and every pixel baseline photographs that perfectly happily. The window it
/// asked for was `layout.height + 130.0`, floored and clamped, with nothing
/// deriving the 130 and nothing able to see it.
///
/// # What moved: the thing the window is measured against
///
/// It used to be measured against the **boot** canvas, and it fitted that by
/// construction and by nothing else — 1034×1120 into a 1034.4×1120.0 content
/// box, under a point of slack in both axes. Every state one fold gesture away
/// overflowed it and stayed overflowed, because a window is sized once at boot
/// and nothing resizes it, and this binary has neither a zoom nor a fit-to-view
/// to recover with. So the measure is now [`ProtocolModel::boot_extent`], the
/// envelope of the states this view spends its time in.
///
/// The no-fudge-factor rule is unchanged and is what keeps this honest: the
/// slack against the envelope must still be under a logical point.
///
/// # What this one cannot see, and which test does
///
/// It asserts that the window brightfield *requests* fits the envelope, and
/// that is true by construction — both sides are `protocol_window_size_for` of
/// the same extent. The moment the OS grants something smaller the canvas pane
/// is short again and every assertion here still passes. That gap is the whole
/// of `the_window_a_small_display_grants_leaves_the_canvas_short` below,
/// which lays its frame out at the **granted** size instead.
///
/// Watched redden, two mutations. Dropping the ledger rail's term from
/// `chrome_budget` — the rail the canvas sits above — leaves the canvas pane
/// 180 points short and fails the fit by that much. Reverting
/// `Boot::window_size` to `boot_layout` fails at *"the boot sized the window
/// differently"*, which is what measuring the envelope removed.
#[test]
fn the_protocol_window_it_asks_for_fits_the_states_it_is_sized_for() {
    let inputs = edgar();
    let (env_w, env_h) = ProtocolModel::boot_extent(&inputs, Flow::Vertical);
    #[allow(clippy::cast_possible_truncation)]
    let (env_w, env_h) = (env_w as f32, env_h as f32);
    assert!(
        env_w > 0.0 && env_h > 0.0,
        "the fixture laid out nothing, so this test proves nothing"
    );
    let (w, h) = protocol_window_size_for(env_w, env_h);

    let mut win = Window::open(Boot::protocol(inputs, Flow::Vertical, None), Mode::Light);
    assert_eq!(
        (win.screen.width(), win.screen.height()),
        (w, h),
        "the boot sized the window differently from `protocol_window_size_for` \
         over the envelope"
    );
    win.settle();

    let box_ = win
        .app
        .canvas_viewport()
        .expect("the canvas pane drew, so it recorded the box it was given");
    assert!(
        box_.width() >= env_w && box_.height() >= env_h,
        "a {w}x{h} window gives the canvas pane a {:.2}x{:.2} content box, \
         and one keystroke lays a {env_w:.0}x{env_h:.0} DAG into it — the graph \
         opens part-scrolled the moment it is folded, which no baseline can tell \
         from a graph that is large",
        box_.width(),
        box_.height(),
    );

    // And in neither axis is the leftover a fudge factor. Every term of
    // `protocol_window_size_for` is read from the component that consumes it, so
    // the only slack it may have is its rounding up to whole logical points. An
    // inequality any positive number satisfies is what let the chart view's
    // height budget be 95 points short while its own test stayed green.
    for (axis, slack) in [
        ("across", box_.width() - env_w),
        ("down", box_.height() - env_h),
    ] {
        assert!(
            slack < 1.0,
            "the canvas pane's content box has {slack:.2}pt of slack {axis} — \
             more than the sub-point rounding `protocol_window_size_for` is \
             allowed, so some of the budget is a fudge factor rather than a \
             component"
        );
    }
}

/// The envelope covers each state the window is sized for, and deliberately
/// does **not** cover the family unfold — which is the trade, stated where a
/// reader will trip over it.
///
/// The other half of the test above, and the half that would otherwise be
/// asserted by nothing. `boot_extent` could return the boot layout's own size
/// and the fit assertion would pass perfectly — it would just be the original
/// defect restated as a green test. So this one enumerates the pictures rather
/// than trusting the fold.
///
/// # Why the family unfold is asserted *out*
///
/// It was in the envelope for one increment, on an argument that is not wrong:
/// the nav boots its cursor on the crosswalk's family tile, so `za` at boot —
/// with no navigation at all — is the unfold. It came out because sizing for it
/// costs 1052 points of window width on **every** launch, taking the vertical
/// window from 1948 across to 3000, against a laptop panel 1512 points wide. A
/// window twice the width of the display is not a convenience.
///
/// So the exclusion is asserted, not merely implemented: putting `graph_full`
/// back reddens this test rather than silently costing a thousand points again.
///
/// Watched redden, two mutations. Putting `graph_full` back into
/// `boot_extent`'s candidate list fails at *"the envelope grew to cover the
/// unfolded family"*. Dropping `graph_exploded` fails at *"the envelope is 68pt
/// short down for the exploded CTEs"*.
#[test]
fn the_boot_envelope_covers_what_it_sizes_for_and_not_the_family_unfold() {
    let inputs = edgar();
    let (env_w, env_h) = ProtocolModel::boot_extent(&inputs, Flow::Vertical);
    let cfg = LayoutConfig {
        flow: Flow::Vertical,
        ..LayoutConfig::default()
    };
    let laid_out = |graph| {
        let l: brightfield_protocol::layout::Layout = brightfield_protocol::layout(graph, &cfg);
        (l.width, l.height)
    };

    let mut covers_something_bigger = false;
    for (name, graph) in [
        ("the boot canvas", &inputs.graph_collapsed),
        ("the exploded CTEs", &inputs.graph_exploded),
        ("the contracted chains", &inputs.graph_contracted),
    ] {
        let (w, h) = laid_out(graph);
        assert!(
            w <= env_w,
            "the envelope is {}pt short across for {name}",
            w - env_w
        );
        assert!(
            h <= env_h,
            "the envelope is {}pt short down for {name}",
            h - env_h
        );
        covers_something_bigger |= w > 0.0 && (w, h) != (env_w, env_h);
    }
    assert!(covers_something_bigger, "the fixture has only one picture");

    let (full_w, full_h) = laid_out(&inputs.graph_full);
    assert!(
        full_w > env_w || full_h > env_h,
        "the envelope grew to cover the unfolded family ({full_w}x{full_h} \
         inside {env_w}x{env_h}) — that is a thousand points of window width on \
         every launch, on a display that may be 1512 points wide, to spare one \
         keystroke in a state that is left immediately"
    );

    let boot = ProtocolModel::boot_layout(&inputs, Flow::Vertical);
    assert!(
        (env_w, env_h) != (boot.width, boot.height),
        "the envelope is the boot canvas itself ({}x{}) — the window is still \
         measured against the configuration the user leaves immediately",
        boot.width,
        boot.height
    );
}

// ---------------------------------------------------------------------------
// The window the display grants
// ---------------------------------------------------------------------------

/// Whatever the content asks for, the window asked of the OS fits the monitor —
/// on both of the displays this product is used on, in both flows.
///
/// **The assertion that had no home before.** Nothing in the boot path had a
/// term for the screen at all: the size was read outwards from the graph and
/// handed to `run_native`, and a request larger than the display was left to
/// the compositor. On the shipped crosswalk that request is 1948 points across
/// vertically and 3972 horizontally — the first is wider than a laptop panel
/// and the second is wider than either display here — so "larger than the
/// monitor" was the normal case, not the edge one.
///
/// Watched redden, one mutation: making `window_size_on_display` return
/// `natural` unchanged fails at *"Vertical asks for (1948.0, 910.0) and this
/// leaves a 1948x910 window on the laptop panel (1512x982)"*.
#[test]
fn the_window_asked_of_the_os_never_exceeds_the_display() {
    for flow in [Flow::Vertical, Flow::Horizontal] {
        let natural = Boot::protocol(edgar(), flow, None).window_size(ViewKind::Protocol);
        for (screen, display) in [("the laptop panel", LAPTOP), ("the ultrawide", ULTRAWIDE)] {
            let (w, h) = window_size_on_display(natural, display);
            assert!(
                w <= display.0 && h <= display.1,
                "{flow:?} asks for {natural:?} and this leaves a {w}x{h} window \
                 on {screen} ({}x{}) — the part of it past the edge cannot be \
                 read, scrolled to, or dragged back",
                display.0,
                display.1,
            );
            assert!(
                w <= natural.0 && h <= natural.1,
                "the display cap grew the window from {natural:?} to {w}x{h}; it \
                 may only ever cap"
            );
        }
    }
}

/// The cap works one axis at a time: a display short in one dimension leaves
/// the other alone, and an unknown extent caps nothing.
///
/// The extents here are **not** displays anyone owns — that is the point. The
/// two real displays above are both wider than they are tall and both wider
/// than the window, so between them they exercise the width cap twice and the
/// height cap never; a version of `window_size_on_display` that capped the
/// height against the *width's* extent passed every assertion in this file.
/// These are chosen instead to make each axis bite on its own, and to bite the
/// wrong way round if the two are ever crossed.
///
/// The `0.0` case is the headless one, and it is a behaviour rather than an
/// omission: egui reports no monitor at all before a window is mapped, and a
/// cap that read that as a zero-width display would ask for a window with no
/// width in it.
///
/// Watched redden, two mutations. Capping the height against `display.0`
/// (transposing the two calls) fails at *"a 2000x900 window on a 3000x500
/// display came out 2000x900, not 2000x500"*. Treating a `0.0` extent as a real
/// one fails at *"a 2000x900 window on a 0x0 display came out 0x0"*.
#[test]
fn the_display_cap_bites_one_axis_at_a_time() {
    let natural = (2000.0, 900.0);
    for (display, want) in [
        // Narrow and tall: the width is capped, the height is untouched.
        ((1000.0, 2000.0), (1000.0, 900.0)),
        // Wide and short: the height is capped, the width is untouched.
        ((3000.0, 500.0), (2000.0, 500.0)),
        // Larger in both: nothing is capped.
        ((3000.0, 2000.0), natural),
        // Smaller in both: both are capped.
        ((1000.0, 500.0), (1000.0, 500.0)),
        // Unknown in both, and in each singly: the unknown axis is left alone.
        ((0.0, 0.0), natural),
        ((0.0, 500.0), (2000.0, 500.0)),
        ((1000.0, 0.0), (1000.0, 900.0)),
    ] {
        let got = window_size_on_display(natural, display);
        assert_eq!(
            got, want,
            "a {}x{} window on a {}x{} display came out {}x{}, not {}x{}",
            natural.0, natural.1, display.0, display.1, got.0, got.1, want.0, want.1,
        );
    }
}

/// The size a display **grants** is not the size the boot **requested**, and
/// the difference is a canvas that no longer covers its own envelope — asserted
/// on two real frames, one laid out at each size.
///
/// # The hole this closes
///
/// `the_protocol_window_it_asks_for_fits_the_states_it_is_sized_for` asserts
/// that the requested window fits the envelope, and it always will: both sides
/// of that comparison are the same arithmetic over the same extent. It lays its
/// frame out at the request, so it passes identically whether the OS granted
/// that request or halved it. The failure this branch was opened for — a window
/// twice the width of the laptop panel, silently truncated to the panel, with
/// the graph scrolled — was therefore invisible to the one test whose job was
/// watching the window.
///
/// So this one asks the question the other cannot: lay the frame out at what
/// the display *gives*, and does the canvas still cover the graph? On the
/// laptop panel it does not, and saying so is the point — the degradation is
/// named and measured rather than being a scrollbar nobody attributed. On the
/// ultrawide it does, which is the positive control that stops a cap that
/// shrank everything from satisfying the first half.
///
/// The loss is asserted as a **bound**, not an equality, and deliberately: the
/// canvas is one share of the dock between two rails, so a window short by *n*
/// points hands the canvas somewhere between nothing and *n* of them. Restating
/// that share here would be re-deriving `protocol_window_size_for` against
/// itself, which is the failure mode this whole file exists to avoid.
///
/// Watched redden, one mutation: making `window_size_on_display` return
/// `natural` unchanged fails at *"a display that clamps nothing proves
/// nothing"*.
#[test]
fn the_window_a_small_display_grants_leaves_the_canvas_short() {
    let (env_w, env_h) = ProtocolModel::boot_extent(&edgar(), Flow::Vertical);
    #[allow(clippy::cast_possible_truncation)]
    let (env_w, env_h) = (env_w as f32, env_h as f32);

    let natural = Boot::protocol(edgar(), Flow::Vertical, None).window_size(ViewKind::Protocol);
    let granted = window_size_on_display(natural, LAPTOP);
    assert!(
        granted.0 < natural.0,
        "a display that clamps nothing proves nothing: the laptop panel granted \
         the whole {natural:?} window, so this test would pass on a build with \
         no cap at all"
    );
    let clamped_away = natural.0 - granted.0;

    // The canvas the window ASKED for, and the canvas the display GRANTED.
    let canvas_at = |size: (f32, f32)| {
        let mut win = Window::open_at(
            Boot::protocol(edgar(), Flow::Vertical, None),
            Mode::Light,
            egui::vec2(size.0, size.1),
        );
        win.settle();
        win.app
            .canvas_viewport()
            .expect("the canvas pane drew, so it recorded the box it was given")
    };

    // The positive control: a display bigger than the window in both axes grants
    // it whole, and the canvas covers the graph.
    assert_eq!(
        window_size_on_display(natural, ULTRAWIDE),
        natural,
        "the ultrawide is larger than this window in both axes, so it must grant \
         it whole"
    );
    let asked = canvas_at(natural);
    assert!(
        asked.width() >= env_w,
        "the canvas is already {:.2}pt short across at the size the boot asked \
         for, before any display has clamped anything",
        env_w - asked.width()
    );

    let given = canvas_at(granted);
    assert!(
        given.width() < env_w,
        "the laptop panel clamped {clamped_away:.0}pt off a {}pt window and the \
         canvas still covers the {env_w:.0}pt graph — either the cap did nothing \
         or the canvas is not where the window's width goes, and in both cases \
         this test has stopped watching what it claims to",
        natural.0
    );

    let lost = asked.width() - given.width();
    assert!(
        lost > 0.0 && lost <= clamped_away,
        "the clamp took {clamped_away:.0}pt off the window and {lost:.2}pt off \
         the canvas — the canvas is one share of the dock between two rails, so \
         its loss must be positive and no larger than the window's"
    );
    // Down, the same claim in the same shape. It used to be an unconditional
    // "the canvas still covers the graph downwards, because the panel is taller
    // than the window": that premise died when the arrangement gained its
    // ledger rail and its locator band and the window grew past the panel's
    // 982 points. Asserting a bound rather than restoring the premise is the
    // honest reading — the degradation is named and measured, exactly as it is
    // across.
    let clamped_down = natural.1 - granted.1;
    let lost_down = asked.height() - given.height();
    assert!(
        lost_down >= 0.0 && lost_down <= clamped_down,
        "the clamp took {clamped_down:.0}pt off the window's height and \
         {lost_down:.2}pt off the canvas's — the canvas's loss must be positive \
         and no larger than the window's"
    );
    assert!(
        given.height() + clamped_down >= env_h,
        "the laptop panel clamped {clamped_down:.0}pt off this window's height \
         and the canvas came up {:.2}pt short down — more than the clamp \
         explains, so some of the height budget is unaccounted for",
        env_h - given.height() - clamped_down
    );
}

/// The first frame of a live window measures the monitor it landed on and asks
/// the OS to shrink to it — and asks for nothing when it already fits.
///
/// The wiring, not the arithmetic: `window_size_on_display` is a pure function
/// three tests above already hold, and this asserts that a running window
/// actually *sends* its answer. It reads the monitor the way the live binary
/// does, out of `ViewportInfo`, and reads the emitted
/// `ViewportCommand::InnerSize` back off the frame's own output.
///
/// The `MonitorUnknown` arm is the one with teeth. egui reports no monitor at
/// all in a headless context and on a window that is not yet mapped, and a
/// check that treated "don't know" as "fits" would retire itself on frame one
/// and never cap anything — green here, and dead in the product.
///
/// Watched redden, two mutations. Returning `DisplayFit::Fits` instead of
/// `MonitorUnknown` for an unreported monitor fails at *"a context with no
/// monitor answered Fits"*. Dropping the `send_viewport_cmd` fails at *"the
/// frame emitted no InnerSize"*.
#[test]
fn the_first_frame_shrinks_a_window_bigger_than_its_monitor() {
    let natural = (1948.0, 910.0);

    let ask = |monitor: Option<(f32, f32)>| {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(natural.0, natural.1),
            )),
            ..Default::default()
        };
        let id = raw.viewport_id;
        raw.viewports
            .get_mut(&id)
            .expect("egui's own RawInput default carries the root viewport")
            .monitor_size = monitor.map(|(w, h)| egui::vec2(w, h));

        let mut fit = None;
        let out = ctx.run_ui(raw, |_ui| fit = Some(fit_window_to_display(&ctx, natural)));
        let sent: Vec<egui::Vec2> = out
            .viewport_output
            .get(&id)
            .map(|v| {
                v.commands
                    .iter()
                    .filter_map(|c| match c {
                        egui::ViewportCommand::InnerSize(size) => Some(*size),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        (fit.expect("the frame ran"), sent)
    };

    let (fit, sent) = ask(None);
    assert_eq!(
        fit,
        DisplayFit::MonitorUnknown,
        "a context with no monitor answered {fit:?} — a window that has not been \
         mapped yet reports none, so answering anything else retires the cap on \
         the one frame it is most likely to be wrong about"
    );
    assert!(
        sent.is_empty(),
        "a frame that could not read the monitor resized the window anyway, to {sent:?}"
    );

    let (fit, sent) = ask(Some(ULTRAWIDE));
    assert_eq!(
        fit,
        DisplayFit::Fits,
        "the ultrawide fits this window whole"
    );
    assert!(
        sent.is_empty(),
        "a window that already fits its display was resized to {sent:?} anyway"
    );

    let (fit, sent) = ask(Some(LAPTOP));
    let want = egui::vec2(LAPTOP.0, natural.1);
    assert_eq!(
        fit,
        DisplayFit::Shrunk(want),
        "the laptop panel is narrower than this window, so the frame should have \
         asked for {want:?}"
    );
    assert_eq!(
        sent,
        vec![want],
        "the frame emitted no InnerSize the integration could act on — the cap \
         was computed and thrown away"
    );
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
/// The navigator rail's toggle reaches the protocol spine and comes back.
///
/// AC1's round trip, and the reason the verb is a dock toggle rather than a
/// view switch: one keystroke reaches the spine, the same keystroke returns
/// focus to the work, and the cursor is never left parked in a rail. A window
/// that treated the second press as a second move would leave focus on the
/// outline and fail the last assertion.
///
/// The keystroke comes off `brightfield_keys::registry()` rather than being
/// typed here, because the shell wires the binding the registry declares and
/// invents none — a test that typed `cmd-b` would go on pressing a key the
/// registry had stopped naming.
///
/// Watched redden, one mutation: dropping the `focus_return` restore in
/// `MeridianApp::toggle_navigator_focus` (setting the rail's key on both
/// presses) leaves the first three assertions green and fails the last with
/// focus still on the outline rail.
#[test]
fn pressing_the_navigator_toggle_twice_returns_focus() {
    let mut win = Window::open(both(ViewKind::Charts), Mode::Light);
    win.settle();

    let started = PaneKey::new(ViewKind::Charts, brightfield_shell::app::CHART);
    assert!(
        win.app.focus_pane(started),
        "the chart pane is placed, so focus can be put on it"
    );
    win.settle();

    let rail = PaneKey::new(ViewKind::Protocol, brightfield_shell::protocol::OUTLINE);
    win.run(vec![navigator_toggle(), Vec::new()]);
    assert_eq!(
        win.app.focused_pane(),
        Some(rail),
        "the toggle did not reach the navigator rail"
    );

    win.run(vec![navigator_toggle(), Vec::new()]);
    assert_eq!(
        win.app.focused_pane(),
        Some(started),
        "the second press did not return focus to where it started — the \
         toggle is a one-way trip, not a round trip"
    );
}

/// The increment-7 view switcher is gone, not restyled.
///
/// Asserted through what a person can reach rather than by reading the source:
/// the two `selectable_label`s were controls in the title band, so a pointer
/// swept across that band used to be able to change which document the canvas
/// draws. Nothing there does that now — the protocol is the navigator rail,
/// and reaching it is `toggle-outline-rail`.
///
/// The sweep is every four points across the band's width rather than one
/// aimed click, because a switcher put back anywhere in the band is the thing
/// this refuses, not a switcher put back where the old one was.
///
/// Watched redden, one mutation: restoring the pair of `selectable_label`s at
/// the head of `MeridianApp::title_band` fails at the first click that lands
/// on one.
#[test]
fn no_control_in_the_title_band_changes_which_document_the_canvas_draws() {
    use brightfield_workbench::arrangement;

    let mut win = Window::open(both(ViewKind::Charts), Mode::Light);
    win.settle();
    let before = win.app.active();
    let band = win
        .app
        .region_rect(arrangement::TITLE_BAND)
        .expect("the title band drew");
    assert!(
        band.width() > 200.0,
        "the title band drew {}pt wide, so this sweep covers almost none of it",
        band.width()
    );

    // Home is skipped, and it is the one control the band is supposed to
    // carry: it returns to the front door, which empties both documents, and
    // every click after it would then be landing on a door rather than on the
    // band this test is about.
    let home = win.app.home_rect().expect("the band drew a Home button");
    assert!(
        win.app.region_rect(arrangement::NAVIGATOR_RAIL).is_some(),
        "the navigator rail did not draw, so the protocol has nowhere to be a \
         dock and the sweep below proves nothing"
    );
    let mut x = band.left() + 2.0;
    while x < band.right() - 2.0 {
        // Expanded by HOME_CLEARANCE: egui snaps a click that misses every
        // widget to the nearest one within its aim radius, and a click 3.5
        // points clear of the Home button was measured still going to it.
        if home
            .expand(HOME_CLEARANCE)
            .contains(egui::pos2(x, band.center().y))
        {
            x += 4.0;
            continue;
        }
        win.run(vec![click_at(egui::pos2(x, band.center().y)), Vec::new()]);
        assert_eq!(
            win.app.active(),
            before,
            "a click at x={x} in the title band moved the canvas to the other \
             document — the peer switcher is back"
        );
        x += 4.0;
    }
}

/// How far the sweep in
/// `no_control_in_the_title_band_changes_which_document_the_canvas_draws`
/// stays clear of the Home button, in logical points.
///
/// egui gives a click that lands on nothing to the nearest widget within its
/// aim radius, so "outside the rect" is not far enough: measured on this band,
/// a click 3.5 points clear of the button was still given to it. Wide enough
/// that the snap cannot reach, narrow enough that the sweep still covers the
/// rest of a 1386-point band.
const HOME_CLEARANCE: f32 = 20.0;

/// One frame's worth of the registry's `toggle-outline-rail` keystroke.
fn navigator_toggle() -> Vec<egui::Event> {
    let token = brightfield_keys::registry()
        .iter()
        .find(|v| v.longname == brightfield_shell::window::NAVIGATOR_TOGGLE)
        .and_then(brightfield_keys::VerbEntry::primary_key)
        .expect("the registry binds the navigator rail's toggle");
    assert_eq!(
        token, "cmd-b",
        "the registry moved the navigator toggle to {token}; this test's event \
         spelling has to move with it"
    );
    let modifiers = egui::Modifiers {
        command: true,
        mac_cmd: cfg!(target_os = "macos"),
        ..Default::default()
    };
    [true, false]
        .into_iter()
        .map(|pressed| egui::Event::Key {
            key: egui::Key::B,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        })
        .collect()
}

/// The top bar's Home button actually returns to the front door — clicked
/// where it drew itself. Sibling to the switcher test above, and asserted the
/// same way: `open_home` can be called from a test all day without a person
/// being able to reach it, so the click is aimed at the rect the bar recorded
/// on the previous frame, and the claim is that the window *went home* — both
/// documents emptied, so `front_door_is_live` again.
///
/// The button lives in the always-drawn left group, not the right-to-left
/// group that a narrow window drops, and it shows only off the door — so this
/// opens with both fixtures loaded, and after the trip the door draws no Home
/// button at all, because there is nowhere left to go.
///
/// Watched redden, one mutation: dropping the `if bar.home` handling after the
/// top panel closes, so the recorded click reaches nothing, fails here at
/// "clicking Home left the window on the dock".
#[test]
fn the_top_bar_home_button_returns_to_the_front_door() {
    let mut win = Window::open(both(ViewKind::Charts), Mode::Light);
    win.settle();
    assert!(
        !win.app.front_door_is_live(),
        "both fixtures loaded, so the dock — not the door"
    );

    let target = win
        .app
        .home_rect()
        .expect("off the door, the top bar draws a Home button");
    assert!(
        win.screen.contains_rect(target),
        "the Home button drew at {target:?}, outside the window — nothing \
         could click it"
    );
    win.run(vec![click_at(target.center()), Vec::new()]);
    win.settle();

    assert!(
        win.app.front_door_is_live(),
        "clicking Home left the window on the dock — the button is chrome \
         that does nothing"
    );
    assert!(
        win.app.home_rect().is_none(),
        "the front door still drew a Home button — there is nowhere to go \
         home from"
    );
}
