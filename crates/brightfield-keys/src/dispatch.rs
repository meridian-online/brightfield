//! The dispatch-resolution table: a PROJECTION of the same keymap-as-data
//! vec a shell feeds its key-binding registration — not a hand-maintained
//! mirror. It encodes, and is tested for, the context/overlay resolution
//! invariants.
//!
//! IMPORTANT (honesty): this proves the projection is faithful to the binding
//! vec and internally consistent. It does NOT prove a live shell's dispatch
//! conforms to these invariants — that conformance is eyeball-verified.

use crate::registry::{BindingContext, BoundKey};

/// The focus/overlay situations dispatch resolves against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchContext {
    /// The canvas holds focus (bare workspace verbs live).
    CanvasFocused,
    /// The YAML editor holds focus (its Input consumes bare letters).
    EditorFocused,
    /// An overlay (palette / help / focus-jump) is open above the workspace.
    OverlayOpen,
    /// The protocol asset-graph panel holds focus — its topological
    /// grammar resolves here, isolated from the chart canvas's bare verbs.
    ProtocolFocused,
    /// A chart's shelf holds focus — the band's cells and the open list under
    /// one. Its grammar resolves here, isolated from the chart canvas's bare
    /// verbs and from the protocol panel's.
    ShelfFocused,
    /// The table's grid holds focus — its cursor is on a cell, and its
    /// grammar resolves here, isolated from the chart canvas's bare verbs, so
    /// the arrows move the cursor rather than pan the chart.
    GridFocused,
    /// The ledger's Versions panel holds focus — its cursor is on a version's
    /// row, and its grammar resolves here, isolated from the chart canvas's
    /// bare verbs, so `Enter` steps the chart back rather than dives in and
    /// `Esc` returns to now rather than clears a selection.
    VersionsFocused,
}

/// Whether a binding in `binding` context resolves in the `dispatch` situation.
///
/// The matrix that encodes every dispatch-resolution invariant:
/// - a Global (`context = None`) binding resolves from BOTH canvas and editor;
/// - a Workspace bare verb resolves ONLY when the canvas is focused — never under
///   the editor, never under an open overlay;
/// - an Editor binding resolves only when the editor is focused;
/// - a Protocol binding resolves only when the protocol panel is focused;
/// - a Shelf binding resolves only when a shelf is focused, so `h` and `l` are
///   the cell beside there, `pop-out` and `dive-in` under the canvas, and
///   `protocol-producer` and `protocol-consumer` under the protocol panel;
/// - a Grid binding resolves only when the grid is focused, so `h` and `l`
///   move its cursor there and the arrows pan the chart only elsewhere;
/// - a Versions binding resolves only when the Versions panel is focused, so
///   `Enter` steps the chart back there and dives in only elsewhere.
#[must_use]
pub fn fires(binding: BindingContext, dispatch: DispatchContext) -> bool {
    matches!(
        (binding, dispatch),
        (BindingContext::Global, _)
            | (BindingContext::Workspace, DispatchContext::CanvasFocused)
            | (BindingContext::Editor, DispatchContext::EditorFocused)
            | (BindingContext::Protocol, DispatchContext::ProtocolFocused)
            | (BindingContext::Shelf, DispatchContext::ShelfFocused)
            | (BindingContext::Grid, DispatchContext::GridFocused)
            | (BindingContext::Versions, DispatchContext::VersionsFocused)
    )
}

/// The dispatch-resolution table — a projection of the keymap-as-data vec.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolutionTable {
    rows: Vec<BoundKey>,
}

impl ResolutionTable {
    /// The projected rows (equal to the input binding vec).
    #[must_use]
    pub fn rows(&self) -> &[BoundKey] {
        &self.rows
    }

    /// The verbs that resolve for `keystrokes` in `dispatch`.
    #[must_use]
    pub fn resolves(&self, keystrokes: &str, dispatch: DispatchContext) -> Vec<&'static str> {
        self.rows
            .iter()
            .filter(|r| r.keystrokes == keystrokes && fires(r.context, dispatch))
            .map(|r| r.longname)
            .collect()
    }

    /// The verbs that resolve in `dispatch` (any keystroke).
    #[must_use]
    pub fn resolving_in(&self, dispatch: DispatchContext) -> Vec<&'static str> {
        self.rows
            .iter()
            .filter(|r| fires(r.context, dispatch))
            .map(|r| r.longname)
            .collect()
    }
}

/// Project a dispatch-resolution table from the keymap-as-data vec.
#[must_use]
pub fn resolution_table(bound: &[BoundKey]) -> ResolutionTable {
    ResolutionTable {
        rows: bound.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{keymap_bindings, registry, BindingContext};

    fn table() -> ResolutionTable {
        resolution_table(&keymap_bindings(&registry()))
    }

    #[test]
    fn table_is_a_faithful_projection_of_the_binding_vec() {
        let bound = keymap_bindings(&registry());
        let table = resolution_table(&bound);
        assert_eq!(
            table.rows(),
            bound.as_slice(),
            "table must equal the shipped binding vec"
        );
    }

    #[test]
    fn global_bindings_resolve_from_both_workspace_and_editor() {
        let t = table();
        // toggle-focus (cmd-e) is Global — fires from both contexts.
        assert!(t
            .resolves("cmd-e", DispatchContext::CanvasFocused)
            .contains(&"toggle-focus"));
        assert!(t
            .resolves("cmd-e", DispatchContext::EditorFocused)
            .contains(&"toggle-focus"));
    }

    #[test]
    fn editor_suppresses_every_workspace_bare_verb() {
        let bound = keymap_bindings(&registry());
        let t = resolution_table(&bound);
        // No workspace-scoped binding resolves while the editor is focused.
        for b in &bound {
            if b.context == BindingContext::Workspace {
                assert!(
                    !t.resolves(b.keystrokes, DispatchContext::EditorFocused)
                        .contains(&b.longname),
                    "workspace verb {} leaked into editor focus",
                    b.longname
                );
            }
        }
    }

    #[test]
    fn no_bare_verb_resolves_under_an_open_overlay() {
        let bound = keymap_bindings(&registry());
        let t = resolution_table(&bound);
        for b in &bound {
            if b.context == BindingContext::Workspace {
                assert!(
                    !t.resolves(b.keystrokes, DispatchContext::OverlayOpen)
                        .contains(&b.longname),
                    "workspace verb {} resolved under an overlay",
                    b.longname
                );
            }
        }
    }

    #[test]
    fn plot_focus_resolves_a_bare_verb() {
        let t = table();
        // A canvas-focused bare verb (p) resolves.
        assert!(t
            .resolves("p", DispatchContext::CanvasFocused)
            .contains(&"toggle-presentation"));
        assert!(t
            .resolves("c", DispatchContext::CanvasFocused)
            .contains(&"cycle-colour-scheme"));
    }

    #[test]
    fn h_and_l_are_the_cell_beside_in_the_shelf_and_keep_their_other_meanings() {
        let t = table();
        // Exact vectors, not `contains`: a Workspace or Protocol binding that
        // leaked into the shelf's dispatch context would add a second verb.
        assert_eq!(
            t.resolves("h", DispatchContext::ShelfFocused),
            vec!["move-shelf-left"]
        );
        assert_eq!(
            t.resolves("l", DispatchContext::ShelfFocused),
            vec!["move-shelf-right"]
        );
        assert_eq!(
            t.resolves("h", DispatchContext::CanvasFocused),
            vec!["pop-out"]
        );
        assert_eq!(
            t.resolves("l", DispatchContext::CanvasFocused),
            vec!["dive-in"]
        );
        assert_eq!(
            t.resolves("h", DispatchContext::ProtocolFocused),
            vec!["protocol-producer"]
        );
        assert_eq!(
            t.resolves("l", DispatchContext::ProtocolFocused),
            vec!["protocol-consumer"]
        );
    }

    #[test]
    fn the_grid_moves_its_cursor_on_the_keys_that_pan_and_dive_elsewhere() {
        let t = table();
        // Exact vectors: a Workspace binding leaking into the grid's dispatch
        // context would add a second verb, and the arrow would both move the
        // cursor and pan the chart.
        for (keys, verb) in [
            ("j", "move-cursor-down"),
            ("down", "move-cursor-down"),
            ("k", "move-cursor-up"),
            ("up", "move-cursor-up"),
            ("h", "move-cursor-left"),
            ("left", "move-cursor-left"),
            ("l", "move-cursor-right"),
            ("right", "move-cursor-right"),
        ] {
            assert_eq!(
                t.resolves(keys, DispatchContext::GridFocused),
                vec![verb],
                "`{keys}` with the grid focused"
            );
        }
        assert_eq!(
            t.resolves("left", DispatchContext::CanvasFocused),
            vec!["pan-left"]
        );
        assert_eq!(
            t.resolves("l", DispatchContext::CanvasFocused),
            vec!["dive-in"]
        );
        // And the zoom, axis-lock and reset keys resolve to nothing there.
        for keys in ["=", "-", "x", "0"] {
            assert!(
                t.resolves(keys, DispatchContext::GridFocused).is_empty(),
                "`{keys}` resolves with the grid focused"
            );
        }
    }

    #[test]
    fn the_versions_panel_steps_back_and_returns_on_the_keys_that_dive_and_clear_elsewhere() {
        let t = table();
        // Exact vectors: a Workspace binding leaking into the panel's dispatch
        // context would add a second verb, and Enter would both step the chart
        // back and dive into the focused container.
        for (keys, verb) in [
            ("j", "move-version-cursor-down"),
            ("down", "move-version-cursor-down"),
            ("k", "move-version-cursor-up"),
            ("up", "move-version-cursor-up"),
            ("enter", "step-back-to-version"),
            ("escape", "return-to-now"),
        ] {
            assert_eq!(
                t.resolves(keys, DispatchContext::VersionsFocused),
                vec![verb],
                "`{keys}` with the Versions panel focused"
            );
        }
        assert_eq!(
            t.resolves("enter", DispatchContext::CanvasFocused),
            vec!["dive-in"]
        );
        assert!(t
            .resolves("enter", DispatchContext::GridFocused)
            .is_empty());
    }

    #[test]
    fn each_binding_context_fires_in_its_own_dispatch_context_and_global_in_all() {
        use BindingContext::{Editor, Global, Grid, Protocol, Shelf, Versions, Workspace};
        use DispatchContext::{
            CanvasFocused, EditorFocused, GridFocused, OverlayOpen, ProtocolFocused, ShelfFocused,
            VersionsFocused,
        };
        // Written as a `match`, so a new binding context does not compile until
        // this test says which dispatch context it fires in.
        let own = |binding: BindingContext| match binding {
            Workspace => Some(CanvasFocused),
            Editor => Some(EditorFocused),
            Protocol => Some(ProtocolFocused),
            Shelf => Some(ShelfFocused),
            Grid => Some(GridFocused),
            Versions => Some(VersionsFocused),
            Global => None,
        };
        for binding in [Workspace, Editor, Protocol, Shelf, Grid, Versions, Global] {
            for dispatch in [
                CanvasFocused,
                EditorFocused,
                OverlayOpen,
                ProtocolFocused,
                ShelfFocused,
                GridFocused,
                VersionsFocused,
            ] {
                let expected = own(binding).is_none_or(|own| own == dispatch);
                assert_eq!(
                    fires(binding, dispatch),
                    expected,
                    "{binding:?} binding under {dispatch:?}"
                );
            }
        }
    }

    #[test]
    fn undo_fires_on_u_in_the_shelf_as_in_the_workspace_and_on_cmd_z_in_the_shelf() {
        let t = table();
        assert!(t
            .resolves("u", DispatchContext::ShelfFocused)
            .contains(&"undo"));
        assert!(t
            .resolves("u", DispatchContext::CanvasFocused)
            .contains(&"undo"));
        assert!(t
            .resolves("cmd-z", DispatchContext::ShelfFocused)
            .contains(&"undo"));
    }

    #[test]
    fn z_then_a_channel_puts_the_outlines_column_and_z_a_still_folds() {
        let t = table();
        for (keys, verb) in [
            ("z x", "put-column-on-x"),
            ("z y", "put-column-on-y"),
            ("z c", "put-column-on-colour"),
            ("z a", "toggle-fold"),
        ] {
            assert_eq!(
                t.resolves(keys, DispatchContext::ProtocolFocused),
                vec![verb],
                "{keys} in the Protocol panel"
            );
            // The chord belongs to the panel: the canvas has no `z` binding.
            assert!(
                t.resolves(keys, DispatchContext::CanvasFocused).is_empty(),
                "{keys} leaked into the canvas"
            );
        }
    }

    #[test]
    fn toggle_chord_is_unshadowed() {
        let bound = keymap_bindings(&registry());
        // Exactly one binding claims cmd-e, and it is the focus toggle.
        let claimers: Vec<_> = bound.iter().filter(|b| b.keystrokes == "cmd-e").collect();
        assert_eq!(claimers.len(), 1);
        assert_eq!(claimers[0].longname, "toggle-focus");
    }
}
