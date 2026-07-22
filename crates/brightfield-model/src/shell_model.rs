//! Workspace-shell model — the framework-free half of the
//! docked authoring workspace.
//!
//! Panel identities, default dock geometry, the initial-window-size formula,
//! and the presentation-mode → panel-visibility mapping all live
//! here as plain data and arithmetic; `shell.rs` is the thin GPUI/
//! gpui-component translation shim over this module. No gpui import may
//! enter this file (semantic-layer rule).
//!
//! [`PresentationMode`] itself stays in `brightfield_ui::workspace` (the
//! gpui-free machine, deliberately unmoved); this module only maps it
//! onto the dock shell: which panels report `visible()` and whether the
//! authoring docks are open.

use brightfield_ui::{framed_window_size, PresentationMode};

/// The canvas panel's stable serialisation name. Once persisted layouts
/// exist this must never change (it keys panel deserialisation).
pub const CANVAS_PANEL_NAME: &str = "BrightfieldCanvas";

/// The YAML spec editor panel's stable serialisation name.
pub const EDITOR_PANEL_NAME: &str = "BrightfieldSpecEditor";

/// The data-sources sidebar panel's stable serialisation name.
pub const SIDEBAR_PANEL_NAME: &str = "BrightfieldSidebar";

/// The reload/save feedback Log panel's stable serialisation name.
pub const LOG_PANEL_NAME: &str = "BrightfieldLog";

/// Default width of the left (sidebar) dock, in logical pixels.
pub const SIDEBAR_DOCK_WIDTH: f64 = 220.0;

/// Default width of the right (editor) dock, in logical pixels.
pub const EDITOR_DOCK_WIDTH: f64 = 380.0;

/// Default height of the bottom (log) dock when opened, in logical pixels.
/// The dock is seeded CLOSED (a 29px strip advertises the drop/expand
/// affordance without re-carving the author's layout); this is the height
/// it opens to.
pub const BOTTOM_DOCK_HEIGHT: f64 = 180.0;

/// The role a panel plays in the workspace — the input (with the mode) to
/// the presentation-visibility mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelRole {
    /// The chart canvas (center, locked, never hidden).
    Canvas,
    /// The YAML spec editor (right dock).
    Editor,
    /// The data-sources sidebar (left dock).
    Sidebar,
    /// The reload/save feedback log (bottom dock).
    Log,
}

/// Whether a panel of `role` reports `visible()` under `mode`.
///
/// Presentation hides every authoring panel; the canvas remains in both
/// modes. A pure function of (mode, role) — the GPUI shell reads it, never
/// decides it.
#[must_use]
pub fn panel_visible(mode: PresentationMode, role: PanelRole) -> bool {
    match role {
        PanelRole::Canvas => true,
        PanelRole::Editor | PanelRole::Sidebar | PanelRole::Log => mode.chrome_visible(),
    }
}

/// Whether the keyboard-grammar chrome (the breadcrumb + focus ring) is
/// visible under `mode`. Mirrors [`panel_visible`]'s shape for chrome
/// that is NOT a dock panel: presentation hides all authoring chrome; authoring
/// shows it. A pure function of `mode` — the canvas reads it, never decides. The
/// byte gate is unaffected either way (this chrome never reaches the
/// headless render path).
#[must_use]
pub fn grammar_chrome_visible(mode: PresentationMode) -> bool {
    mode.chrome_visible()
}

/// Whether a just-loaded layout needs the bottom Log dock appended
/// (the backfill decision): every pre-round saved layout lacks a
/// bottom dock and gets the same closed Log dock the default layout seeds;
/// a layout that already carries one restores exactly as saved — no double
/// dock, no forced state. Pure so the shell executes, never decides.
#[must_use]
pub fn bottom_dock_needs_backfill(has_bottom_dock: bool) -> bool {
    !has_bottom_dock
}

/// What the shell does with the BOTTOM dock when the presentation mode
/// changes. Unlike the left/right rails — whose closed form is
/// invisible, so `set_open(false)` suffices — a closed bottom dock still
/// renders a 29px title-bar strip, which would break presentation's
/// consumer-preview promise. It must be removed entirely, then rebuilt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BottomDockAction {
    /// Entering presentation: stash the dock's rebuild state, then remove
    /// the dock — zero bottom-dock chrome remains.
    Remove,
    /// Back in authoring: rebuild the dock from the stash — contents, size,
    /// and open/closed state exactly as the author left them.
    Rebuild,
}

/// The bottom-dock action for `mode` — the pure half of the presentation
/// round trip; the GPUI shell executes it against the DockArea.
#[must_use]
pub fn bottom_dock_action(mode: PresentationMode) -> BottomDockAction {
    if mode.chrome_visible() {
        BottomDockAction::Rebuild
    } else {
        BottomDockAction::Remove
    }
}

/// Whether the authoring docks' LEFT/RIGHT rails are open under `mode`:
/// presentation collapses them so the canvas runs full-bleed. The BOTTOM
/// dock is deliberately NOT covered by this mapping — its closed form
/// still paints a 29px title-bar strip, so presentation removes it
/// entirely instead; see [`bottom_dock_action`] (review F6 correction).
#[must_use]
pub fn docks_open(mode: PresentationMode) -> bool {
    mode.chrome_visible()
}

/// Whether a dock that a menu-move just emptied of visible panels should
/// close: an emptied dock's stack renders a hollow area (their
/// TabPanel warns it is "visually empty and undroppable" once its last
/// panel leaves), so the shell collapses it rather than leaving dead
/// chrome. Pure so the shell executes, never decides.
#[must_use]
pub fn dock_closes_when_emptied(remaining_visible_panels: usize) -> bool {
    remaining_visible_panels == 0
}

/// Whether layout events should reach the persistence policy under `mode`:
/// presentation's dock collapses are the TOGGLE's own doing, not the
/// author's arrangement, so they must never overwrite the saved authoring
/// layout (and a quit while presenting keeps the last authoring state).
#[must_use]
pub fn layout_persistable(mode: PresentationMode) -> bool {
    mode.chrome_visible()
}

/// Initial window content size for a dashboard bounding box hosted in the
/// docked workspace: the framed size (canvas + chrome margins) plus the
/// default authoring dock widths, so the first boot shows the canvas
/// unsqueezed beside the editor and sidebar.
///
/// Initial size ONLY — the "window never resizes on toggle" invariant
/// is superseded: once open, DockArea owns
/// the layout and the user owns the window size.
#[must_use]
pub fn initial_window_size(dashboard_width: f64, dashboard_height: f64) -> (f64, f64) {
    let (w, h) = framed_window_size(dashboard_width, dashboard_height);
    (w + SIDEBAR_DOCK_WIDTH + EDITOR_DOCK_WIDTH, h)
}

/// Clamp an initial window content size to the display's visible bounds
/// (menu bar / taskbar excluded): a dashboard plus both dock widths can
/// exceed a laptop display, and centring an oversized content rect pushes
/// the titlebar off-screen. `None` (headless, or no display information)
/// passes the size through unclamped — centring then falls back to the
/// platform's own behaviour.
#[must_use]
pub fn clamp_to_display(size: (f64, f64), display: Option<(f64, f64)>) -> (f64, f64) {
    match display {
        Some((display_w, display_h)) => (size.0.min(display_w), size.1.min(display_h)),
        None => size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the mode → visibility mapping over both modes and all
    /// three panel roles — authoring shows everything; presentation keeps
    /// ONLY the canvas. The docks-open bit follows the same flip.
    ///
    /// Layout invariance (the AC's other clause) rides on this mapping
    /// being a pure function of (mode, role): the PNG dump path returns
    /// before any shell construction (the seam), and the
    /// spec-derived layout takes no shell state (the pinned
    /// invariance), so toggling presentation cannot affect PNG output.
    /// Purity is a property of the signature (no hidden state to probe) —
    /// the BEHAVIOURAL pin is dump_seam.rs's run-twice byte-identity test
    /// against the real binary, not a repeated-call assertion here.
    #[test]
    fn presentation_hides_authoring_panels_keeps_canvas() {
        use PanelRole::*;
        let authoring = PresentationMode::Authoring;
        let presentation = PresentationMode::Presentation;

        for role in [Canvas, Editor, Sidebar] {
            assert!(
                panel_visible(authoring, role),
                "authoring shows every panel ({role:?})"
            );
        }
        assert!(panel_visible(presentation, Canvas), "the canvas remains");
        assert!(!panel_visible(presentation, Editor), "editor hides");
        assert!(!panel_visible(presentation, Sidebar), "sidebar hides");

        assert!(docks_open(authoring), "authoring docks open");
        assert!(
            !docks_open(presentation),
            "presentation collapses the docks"
        );

        // Toggle symmetry rides PresentationMode's own tested machine: the
        // mapping is pure, so toggling back restores the exact same bits.
        let mut mode = presentation;
        mode.toggle();
        assert!(panel_visible(mode, Editor) && docks_open(mode));
    }

    /// Visibility role: the Log panel follows the authoring
    /// chrome — visible while authoring, hidden under presentation (where
    /// the whole bottom dock is removed anyway; the mapping is belt and
    /// braces for the panel's own `visible()`).
    #[test]
    fn log_panel_visible_in_authoring_hidden_in_presentation() {
        assert!(panel_visible(PresentationMode::Authoring, PanelRole::Log));
        assert!(!panel_visible(
            PresentationMode::Presentation,
            PanelRole::Log
        ));
    }

    /// the keyboard-grammar chrome (breadcrumb + focus ring) follows
    /// the authoring chrome — shown while authoring, hidden under presentation.
    #[test]
    fn grammar_chrome_hidden_in_presentation() {
        assert!(
            grammar_chrome_visible(PresentationMode::Authoring),
            "shown while authoring"
        );
        assert!(
            !grammar_chrome_visible(PresentationMode::Presentation),
            "hidden in presentation"
        );
    }

    /// Backfill decision: a restored layout without a bottom
    /// dock gets one backfilled; a layout that already carries one is left
    /// exactly as saved.
    #[test]
    fn backfill_only_when_bottom_dock_missing() {
        assert!(
            bottom_dock_needs_backfill(false),
            "pre-round layouts backfill"
        );
        assert!(
            !bottom_dock_needs_backfill(true),
            "saved bottom dock restores as-is"
        );
    }

    /// Presentation dock action: presentation removes the bottom
    /// dock (its closed form still paints a 29px strip — set_open is not
    /// enough); authoring rebuilds it from the stash. Toggle symmetry rides
    /// PresentationMode's tested machine.
    #[test]
    fn presentation_removes_bottom_dock_authoring_rebuilds() {
        assert_eq!(
            bottom_dock_action(PresentationMode::Presentation),
            BottomDockAction::Remove
        );
        assert_eq!(
            bottom_dock_action(PresentationMode::Authoring),
            BottomDockAction::Rebuild
        );

        let mut mode = PresentationMode::Presentation;
        mode.toggle();
        assert_eq!(bottom_dock_action(mode), BottomDockAction::Rebuild);
    }

    /// Emptied-dock decision: a menu-move that empties its
    /// source dock closes it; any remaining visible panel keeps it open.
    #[test]
    fn emptied_source_dock_closes() {
        assert!(dock_closes_when_emptied(0), "hollow docks collapse");
        assert!(!dock_closes_when_emptied(1), "an occupied dock stays");
        assert!(!dock_closes_when_emptied(3));
    }

    /// Presentation guard: layout events persist only while
    /// authoring — presentation's own dock collapses never overwrite the
    /// saved authoring arrangement.
    #[test]
    fn presentation_layout_is_not_persisted() {
        assert!(layout_persistable(PresentationMode::Authoring));
        assert!(!layout_persistable(PresentationMode::Presentation));
    }

    /// Geometry: the initial window adds both default dock
    /// widths to the framed size — initial size only, per the
    /// superseded-invariant note above.
    #[test]
    fn initial_window_adds_dock_widths_to_framed_size() {
        let (framed_w, framed_h) = framed_window_size(800.0, 600.0);
        let (w, h) = initial_window_size(800.0, 600.0);
        assert_eq!(w, framed_w + SIDEBAR_DOCK_WIDTH + EDITOR_DOCK_WIDTH);
        assert_eq!(h, framed_h);
    }

    /// Geometry clamp: the initial content size never exceeds
    /// the display's visible bounds — an oversized dashboard clamps per
    /// axis (so `Bounds::centered` cannot push the titlebar off-screen),
    /// a fitting window is untouched, and headless (no display) passes
    /// through unclamped.
    #[test]
    fn initial_window_clamps_to_visible_display_bounds() {
        let laptop = Some((1512.0, 944.0));

        // Oversized on both axes → exactly the display size.
        assert_eq!(clamp_to_display((2600.0, 1300.0), laptop), (1512.0, 944.0));
        // One oversized axis clamps independently.
        assert_eq!(clamp_to_display((2600.0, 700.0), laptop), (1512.0, 700.0));
        assert_eq!(clamp_to_display((900.0, 1300.0), laptop), (900.0, 944.0));
        // A window that fits is untouched.
        assert_eq!(clamp_to_display((900.0, 700.0), laptop), (900.0, 700.0));
        // No display information → unclamped passthrough.
        assert_eq!(clamp_to_display((2600.0, 1300.0), None), (2600.0, 1300.0));

        // The real caller feeds initial_window_size through the clamp: the
        // result is always <= the display on both axes.
        let (w, h) = clamp_to_display(initial_window_size(2400.0, 1200.0), laptop);
        assert!(
            w <= 1512.0 && h <= 944.0,
            "clamped ({w}, {h}) fits the display"
        );
    }
}
