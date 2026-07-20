//! The Esc precedence ladder: a pure decision over the current surface
//! state. Strict order — dismiss an open overlay > cancel a pending prefix >
//! clear the focused view's selection. It does NOT auto-pop the focus surface
//! (pop is `h` / `q`).
//!
//! Cancelling a pending prefix relies on GPUI's replay-on-non-match (a key that
//! does not extend the prefix resolves it before the 1s timer), NOT
//! `window.clear_pending_keystrokes` (which is `pub(crate)` at our pin). The
//! cancel-pending rung is inert in v1 (no `g`-sequences are wired) but the
//! decision is modelled and tested.

/// The surface state Esc resolves against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EscState {
    /// An overlay (palette / help / focus-jump) is open.
    pub overlay_open: bool,
    /// A key-sequence prefix is pending (e.g. after `g`).
    pub prefix_pending: bool,
    /// The focused view has a live selection.
    pub selection_present: bool,
}

/// What Esc resolves to, in strict precedence order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscAction {
    /// Dismiss the open overlay.
    DismissOverlay,
    /// Cancel the pending key-sequence prefix.
    CancelPending,
    /// Clear the focused view's selection.
    ClearSelection,
    /// Nothing to do (Esc does not auto-pop focus).
    Noop,
}

/// Resolve the Esc ladder for `state`. Higher rungs win.
#[must_use]
pub fn esc_action(state: EscState) -> EscAction {
    if state.overlay_open {
        EscAction::DismissOverlay
    } else if state.prefix_pending {
        EscAction::CancelPending
    } else if state.selection_present {
        EscAction::ClearSelection
    } else {
        EscAction::Noop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(overlay: bool, pending: bool, selection: bool) -> EscState {
        EscState { overlay_open: overlay, prefix_pending: pending, selection_present: selection }
    }

    #[test]
    fn ladder_resolves_in_strict_order() {
        // Overlay wins over everything.
        assert_eq!(esc_action(state(true, true, true)), EscAction::DismissOverlay);
        // Then pending prefix.
        assert_eq!(esc_action(state(false, true, true)), EscAction::CancelPending);
        // Then clear selection.
        assert_eq!(esc_action(state(false, false, true)), EscAction::ClearSelection);
        // Idle → no-op (never auto-pops focus).
        assert_eq!(esc_action(state(false, false, false)), EscAction::Noop);
    }

    #[test]
    fn overlay_beats_selection_and_pending_beats_selection() {
        assert_eq!(esc_action(state(true, false, true)), EscAction::DismissOverlay);
        assert_eq!(esc_action(state(false, true, false)), EscAction::CancelPending);
    }
}
