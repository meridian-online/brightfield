//! Framework-free semantic layer for the selection-first keyboard grammar (card 0018).
//!
//! Scope is where focus sits on the ComponentPath tree; a bare argumentless verb
//! acts on the focused node; a fuzzy Space palette carries discoverability so
//! single keys stay terse. This crate is the gpui-FREE half of that grammar —
//! the command registry, focus state machine, path↔plot resolver, scope resolver,
//! palette filter, Esc ladder, and dispatch-resolution projection all live here as
//! plain data and state machines. "Could this run headless? yes" — enforced by
//! construction: this crate has NO gpui dependency. The GPUI adapter (in
//! `brightfield-app` / `brightfield-ui`) turns a [`BindingSpec`] into a
//! `gpui::KeyBinding`, maps a `longname` to its action, and adapts these
//! decisions into views; nothing here knows about gpui.
//!
//! Verification split: everything here is headless-unit-tested (verifies:
//! capability). The live GPUI wiring that consumes it is macOS-eyeball verified,
//! because keyboard interaction is not headless-testable.

pub mod altitude;
pub mod cmdlog;
pub mod dispatch;
pub mod esc;
pub mod focus;
pub mod fuzzy;
pub mod palette;
pub mod registry;
pub mod scope;

pub use altitude::Altitude;
pub use cmdlog::{DataCommand, ProtocolCmdLog, RecordRejection};
pub use dispatch::{fires, resolution_table, DispatchContext, ResolutionTable};
pub use esc::{esc_action, EscAction, EscState};
pub use focus::{focus_jump_candidates, FocusNode, FocusState, FocusTree, JumpCandidate, NodeKind};
pub use palette::{palette_filter, PaletteCandidate, RecencyCounter};
pub use registry::{
    help_sheet, keymap_bindings, palette_corpus, registry, BindingContext, BindingSpec, BoundKey,
    CommandTier, Drives, HelpRow, PaletteEntry, ReservedReason, Scores, VerbEntry, VerbStatus,
};
pub use scope::{bare_applicable, resolve_scope, RejectReason, ScopeContext, ScopeResolution};

#[cfg(test)]
mod tests {
    //! Cross-module wiring tests (ac-04 shared predicate; producers feed each other).
    use super::*;

    #[test]
    fn kbg_ac04_bare_resolver_and_palette_share_one_altitude_predicate() {
        // The verbs a bare key FIRES at an altitude equal the verbs the palette
        // ENABLES there (enabled = not-reserved, applicable) — one predicate.
        let reg = registry();
        let recency = RecencyCounter::new();
        for altitude in [Altitude::Dashboard, Altitude::View] {
            let bare_fires: std::collections::HashSet<&str> =
                reg.iter().filter(|v| bare_applicable(v, altitude)).map(|v| v.longname).collect();
            let palette_enables: std::collections::HashSet<&str> = palette_filter(&reg, altitude, "", &recency)
                .into_iter()
                .filter(|c| c.enabled)
                .map(|c| c.longname)
                .collect();
            assert_eq!(bare_fires, palette_enables, "predicate diverged at {altitude:?}");
        }
    }

    #[test]
    fn kbg_registry_is_the_sole_input_to_every_producer() {
        // Smoke: the three producers + the resolution table all derive from one registry().
        let reg = registry();
        let _keys = keymap_bindings(&reg);
        let _corpus = palette_corpus(&reg);
        let _help = help_sheet(&reg);
        let table = resolution_table(&keymap_bindings(&reg));
        assert_eq!(table.rows().len(), keymap_bindings(&reg).len());
    }
}
