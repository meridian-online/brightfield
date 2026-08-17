//! The overlay slot, driven through real frames: the keys that open each
//! overlay, the keyboard that works inside one, and the invariant that no
//! bare verb fires underneath.
//!
//! All GPU-free, on the `front_door.rs` harness pattern: one
//! `egui::Context` for the window's whole life, real `run_ui` frames, events
//! fed as a user would press them. The delegates' own behaviour — filtering,
//! ranking, confirm semantics — is unit-tested beside them in
//! `overlays.rs`; what is held here is the *wiring*: key → open, picker
//! event → model effect, escape → closed.

use std::path::PathBuf;

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::{RecordBatch, SqlPredicate};
use brightfield_protocol::layout::Flow;
use brightfield_shell::app::ChartDoc;
use brightfield_shell::design::Mode;
use brightfield_shell::navigation::AxisLock;
use brightfield_shell::overlays::{chart_palette_candidates, CHART_PALETTE_VERBS};
use brightfield_shell::pipeline::live_spec;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_spec::analysis::ComponentPath;
use brightfield_workbench::ViewKind;

/// A window under test — see `front_door.rs` for why the context lives as
/// long as the window.
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

    /// The protocol view over the shipped crosswalk — a real graph for the
    /// grammar and the jump to act on.
    fn crosswalk() -> Self {
        let boot = Boot::start(starts::CROSSWALK, Flow::Vertical).expect("the crosswalk ships");
        let mut win = Self::open(boot);
        win.settle();
        assert_eq!(win.app.active(), ViewKind::Protocol);
        win
    }

    /// The chart view over `fixture` (a name under `examples/`), with a real
    /// DuckDB session behind it — what the dispatchability sweep needs:
    /// `pan_view` / `zoom_view` / `clear_selection` are engine queries, not
    /// field writes, and only a live session can prove one ran.
    fn live_chart(fixture: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(fixture);
        let (live, composed) = live_spec(path.to_str().expect("utf-8 fixture path"))
            .unwrap_or_else(|e| panic!("{fixture} loads live: {e}"));
        let mut boot = Boot::charts(composed);
        boot.live = Some(live);
        boot.spec_path = Some(path);
        let mut win = Self::open(boot);
        win.settle();
        assert_eq!(win.app.active(), ViewKind::Charts);
        win
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

    fn key(&mut self, key: egui::Key) {
        self.run(vec![vec![press(key)], Vec::new()]);
    }

    fn type_text(&mut self, text: &str) {
        self.run(vec![vec![egui::Event::Text(text.to_owned())], Vec::new()]);
    }
}

fn press(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::default(),
    }
}

// ---------------------------------------------------------------------------
// Opening and closing
// ---------------------------------------------------------------------------

/// `space` opens the palette on the protocol view; escape closes it. The
/// escape arrives as the picker's own dismissal — the picker inside the
/// modal consumes it first — and the wiring treats that as close.
#[test]
fn space_opens_the_palette_and_escape_closes_it() {
    let mut win = Window::crosswalk();
    assert_eq!(win.app.open_overlay(), None);

    win.key(egui::Key::Space);
    assert_eq!(win.app.open_overlay(), Some("palette"));

    win.key(egui::Key::Escape);
    assert_eq!(win.app.open_overlay(), None);
}

/// `?` opens the help sheet — on either view, because a reference sheet is
/// read-only — and escape closes it.
#[test]
fn question_mark_opens_the_help_sheet_on_both_views() {
    // The charts view (an empty boot opens on the default active view).
    let mut win = Window::open(Boot::empty());
    win.settle();
    win.key(egui::Key::Questionmark);
    assert_eq!(win.app.open_overlay(), Some("help"));
    win.key(egui::Key::Escape);
    assert_eq!(win.app.open_overlay(), None);

    // The protocol view.
    let mut win = Window::crosswalk();
    win.key(egui::Key::Questionmark);
    assert_eq!(win.app.open_overlay(), Some("help"));
    win.key(egui::Key::Escape);
    assert_eq!(win.app.open_overlay(), None);
}

/// **AC1 + AC6.** `space` opens the palette on the chart view too — the
/// gate that used to read `view == ViewKind::Protocol` for the palette key
/// now also fires on `ViewKind::Charts` — and escape closes it exactly as it
/// does on the protocol view. The candidate list it opens with is restricted
/// to [`CHART_PALETTE_VERBS`]; that restriction is proven dispatchable, verb
/// by verb, in [`every_chart_palette_candidate_actually_dispatches`] below.
#[test]
fn space_opens_the_palette_on_the_chart_view_too() {
    let mut win = Window::open(Boot::empty());
    win.settle();
    assert_eq!(win.app.active(), ViewKind::Charts);
    assert_eq!(win.app.open_overlay(), None);

    win.key(egui::Key::Space);
    assert_eq!(win.app.open_overlay(), Some("palette"));

    win.key(egui::Key::Escape);
    assert_eq!(
        win.app.open_overlay(),
        None,
        "escape did not close the chart-view palette"
    );
}

// ---------------------------------------------------------------------------
// No bare verb underneath
// ---------------------------------------------------------------------------

/// While an overlay is open, the protocol grammar is not fed: a `j` typed at
/// the palette must never also walk the DAG under it. The same key moves the
/// cursor again the moment the overlay closes.
#[test]
fn no_bare_verb_fires_under_an_open_overlay() {
    let mut win = Window::crosswalk();

    // The grammar works before: j moves the cursor off its boot seed.
    let booted = win.app.protocol_model().selected().cloned();
    assert!(booted.is_some(), "the nav seeds a cursor at boot");
    win.key(egui::Key::J);
    let selected = win.app.protocol_model().selected().cloned();
    assert_ne!(selected, booted, "j moved nothing — the fixture is inert");

    // Open the palette; j now belongs to the overlay's keyboard.
    win.key(egui::Key::Space);
    assert_eq!(win.app.open_overlay(), Some("palette"));
    win.key(egui::Key::J);
    assert_eq!(
        win.app.protocol_model().selected().cloned(),
        selected,
        "a bare verb fired underneath the open overlay"
    );

    // Close; the grammar is live again.
    win.key(egui::Key::Escape);
    win.key(egui::Key::J);
    assert_ne!(
        win.app.protocol_model().selected().cloned(),
        selected,
        "the grammar did not come back after the overlay closed"
    );
}

// ---------------------------------------------------------------------------
// Confirming
// ---------------------------------------------------------------------------

/// Typing a query and confirming runs the verb through the same dispatch a
/// keystroke uses: `steps` finds `open-steps-sheet`, enter runs it, the
/// sheet opens, the overlay closes.
#[test]
fn the_palette_runs_the_confirmed_verb() {
    let mut win = Window::crosswalk();
    assert!(!win.app.protocol_model().show_sheet());

    win.key(egui::Key::Space);
    win.settle(); // the query line takes focus
    win.type_text("steps");
    win.key(egui::Key::Enter);

    assert_eq!(win.app.open_overlay(), None, "a run closes the palette");
    assert!(
        win.app.protocol_model().show_sheet(),
        "the confirmed verb did not dispatch"
    );
}

/// `/` opens the node jump over the outline; arrow + enter moves the
/// model's selection to the confirmed row.
#[test]
fn the_jump_moves_the_selection_to_the_confirmed_node() {
    let mut win = Window::crosswalk();
    let outline: Vec<_> = win.app.protocol_model().outline();
    assert!(outline.len() > 1, "the fixture is too small to jump within");

    win.key(egui::Key::Slash);
    assert_eq!(win.app.open_overlay(), Some("jump"));

    // An empty query keeps topological order, so the second row is a known
    // target.
    win.key(egui::Key::ArrowDown);
    win.key(egui::Key::Enter);

    assert_eq!(win.app.open_overlay(), None);
    assert_eq!(
        win.app.protocol_model().selected(),
        Some(&outline[1].id),
        "the selection did not land on the confirmed row"
    );
}

// ---------------------------------------------------------------------------
// AC2 + AC3: the chart palette lists nothing it cannot run
// ---------------------------------------------------------------------------

/// How many rows a mark's own query holds — the un-fakeable read: it comes
/// off the live DuckDB session, not off a field the navigation code writes,
/// so a zoom that changed nothing real still moves this number.
fn mark_rows(doc: &mut ChartDoc, mark: usize) -> usize {
    doc.live_coordinator()
        .expect("a live document")
        .chart_rows(mark)
        .expect("the mark queries")
        .iter()
        .map(RecordBatch::num_rows)
        .sum()
}

/// The addressed plot's x/y span — `(x_max - x_min, y_max - y_min)`. Pan
/// moves the frame without changing this; zoom does not. Reading it (rather
/// than only checking [`ChartDoc::navigated`]) is what tells a pan verb
/// apart from a verb that accidentally zoomed instead.
fn domain_span(doc: &ChartDoc) -> (f64, f64) {
    use brightfield_render::channel::Channel;
    let plot = doc.composed.plots.first().expect("the fixture has a plot");
    let span = |channel| {
        let scale = plot.scales.get(channel).expect("a continuous scale");
        scale.domain_max().expect("continuous") - scale.domain_min().expect("continuous")
    };
    (span(Channel::X), span(Channel::Y))
}

/// Open the chart palette, type `longname` verbatim (an exact match ranks
/// first) and confirm — the same path a person takes, not a shortcut around
/// it.
fn confirm_chart_verb(win: &mut Window, longname: &str) {
    win.key(egui::Key::Space);
    assert_eq!(
        win.app.open_overlay(),
        Some("palette"),
        "space did not open the chart palette"
    );
    win.settle(); // the query line takes focus
    win.type_text(longname);
    win.key(egui::Key::Enter);
    assert_eq!(
        win.app.open_overlay(),
        None,
        "confirming {longname} did not close the palette"
    );
}

/// **AC2 + AC3.** Every verb the chart palette lists is dispatchable at the
/// chart altitude.
///
/// This walks the REAL candidate list — [`chart_palette_candidates`], not a
/// hand-copied mirror of it — and for each longname opens the chart palette
/// through real key events, types it, confirms with enter, and checks the
/// one piece of live state that verb is documented to change. An
/// unrecognised longname panics rather than being skipped: that is what
/// stops a verb added to [`CHART_PALETTE_VERBS`] later from riding through
/// this sweep unproven, which is the exact silent-no-op AC3 exists to catch.
///
/// Proof this sweep can fail: deleting the `"reset-extent" => { ... }` arm
/// below (so it falls to the `other => panic!` case) reddens this test with
/// "reset-extent is listed on the chart palette with no dispatch proof" —
/// the sweep does not pass an unproven verb by omission.
#[test]
fn every_chart_palette_candidate_actually_dispatches() {
    let candidates = chart_palette_candidates();
    assert!(!candidates.is_empty(), "the chart palette lists nothing");
    assert_eq!(
        candidates.len(),
        CHART_PALETTE_VERBS.len(),
        "the chart palette's candidate count does not match its own allow list: {candidates:?}"
    );

    for longname in candidates {
        match longname {
            "clear-selection" => {
                let mut win = Window::live_chart("crossfilter.yaml");
                assert!(
                    win.app
                        .chart_doc_mut()
                        .apply_interaction(Interaction::Select {
                            name: "brush".to_string(),
                            contributor: ComponentPath("root/hconcat[0]".to_string()),
                            predicate: SqlPredicate::Expr("temp > 5".to_string()),
                        }),
                    "the fixture accepted a selection"
                );
                assert!(win.app.chart_doc().selection_active());
                confirm_chart_verb(&mut win, longname);
                assert!(
                    !win.app.chart_doc().selection_active(),
                    "clear-selection did not clear the held selection"
                );
            }
            "pan-left" | "pan-right" | "pan-up" | "pan-down" => {
                let mut win = Window::live_chart("scatter.yaml");
                assert!(
                    !win.app.chart_doc().navigated(),
                    "fixture starts unnavigated"
                );
                let before = domain_span(win.app.chart_doc());
                confirm_chart_verb(&mut win, longname);
                assert!(
                    win.app.chart_doc().navigated(),
                    "{longname} did not navigate the frame"
                );
                let after = domain_span(win.app.chart_doc());
                assert!(
                    (before.0 - after.0).abs() < 1e-9 && (before.1 - after.1).abs() < 1e-9,
                    "{longname} changed the frame's span ({before:?} -> {after:?}) — \
                     that is a zoom, not a pan"
                );
            }
            "zoom-in" => {
                let mut win = Window::live_chart("scatter.yaml");
                let before = mark_rows(win.app.chart_doc_mut(), 0);
                confirm_chart_verb(&mut win, longname);
                let after = mark_rows(win.app.chart_doc_mut(), 0);
                assert!(
                    after < before,
                    "zoom-in did not narrow the picture: {after} vs {before}"
                );
            }
            "zoom-out" => {
                let mut win = Window::live_chart("scatter.yaml");
                assert!(
                    win.app.chart_doc_mut().zoom_view(2.0),
                    "priming a zoom to zoom back out of"
                );
                let zoomed = mark_rows(win.app.chart_doc_mut(), 0);
                confirm_chart_verb(&mut win, longname);
                let after = mark_rows(win.app.chart_doc_mut(), 0);
                assert!(
                    after > zoomed,
                    "zoom-out did not widen the picture: {after} vs {zoomed}"
                );
            }
            "cycle-axis-lock" => {
                let mut win = Window::live_chart("scatter.yaml");
                assert_eq!(win.app.chart_doc().axis_lock, AxisLock::Both);
                confirm_chart_verb(&mut win, longname);
                assert_eq!(
                    win.app.chart_doc().axis_lock,
                    AxisLock::XOnly,
                    "cycle-axis-lock did not cycle the lock"
                );
            }
            "reset-extent" => {
                let mut win = Window::live_chart("scatter.yaml");
                assert!(
                    win.app.chart_doc_mut().zoom_view(2.0),
                    "priming a zoom to reset"
                );
                assert!(win.app.chart_doc().navigated());
                confirm_chart_verb(&mut win, longname);
                assert!(
                    !win.app.chart_doc().navigated(),
                    "reset-extent did not clear the navigated frame"
                );
            }
            "open-home" => {
                let mut win = Window::live_chart("scatter.yaml");
                assert!(!win.app.chart_doc().is_empty(), "fixture starts loaded");
                confirm_chart_verb(&mut win, longname);
                assert!(
                    win.app.chart_doc().is_empty(),
                    "open-home did not return to the front door"
                );
            }
            other => panic!(
                "{other} is listed on the chart palette with no dispatch proof in \
                 this sweep — add a case above before adding it to CHART_PALETTE_VERBS"
            ),
        }
    }
}
