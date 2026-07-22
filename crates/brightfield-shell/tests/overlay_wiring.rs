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

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
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

/// The palette does not open on the chart view: at the chart altitudes most
/// registry verbs have no handler in this shell yet, and a palette of rows
/// that silently no-op is worse than none. It arrives with the editing
/// bridge.
#[test]
fn the_palette_does_not_open_on_the_chart_view_yet() {
    let mut win = Window::open(Boot::empty());
    win.settle();
    assert_eq!(win.app.active(), ViewKind::Charts);
    win.key(egui::Key::Space);
    assert_eq!(win.app.open_overlay(), None);
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
