//! The dispatch-resolution table (ac-07): a PROJECTION of the same keymap-as-data
//! vec `main` feeds to `cx.bind_keys` — not a hand-maintained mirror. It encodes,
//! and is tested for, the context/overlay resolution invariants.
//!
//! IMPORTANT (honesty): this proves the projection is faithful to the binding vec
//! and internally consistent. It does NOT prove GPUI's live dispatch conforms to
//! these invariants — that conformance is eyeball-verified by ac-09 / ac-12.

use crate::registry::{BindingContext, BoundKey};

/// The three focus/overlay situations dispatch resolves against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchContext {
    /// The canvas holds focus (bare workspace verbs live).
    CanvasFocused,
    /// The YAML editor holds focus (its Input consumes bare letters).
    EditorFocused,
    /// An overlay (palette / help / focus-jump) is open above the workspace.
    OverlayOpen,
}

/// Whether a binding in `binding` context resolves in the `dispatch` situation.
///
/// The matrix that encodes every ac-07 invariant:
/// - a Global (`context = None`) binding resolves from BOTH canvas and editor;
/// - a Workspace bare verb resolves ONLY when the canvas is focused — never under
///   the editor, never under an open overlay;
/// - an Editor binding resolves only when the editor is focused.
#[must_use]
pub fn fires(binding: BindingContext, dispatch: DispatchContext) -> bool {
    match (binding, dispatch) {
        (BindingContext::Global, _) => true,
        (BindingContext::Workspace, DispatchContext::CanvasFocused) => true,
        (BindingContext::Editor, DispatchContext::EditorFocused) => true,
        _ => false,
    }
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
        self.rows.iter().filter(|r| fires(r.context, dispatch)).map(|r| r.longname).collect()
    }
}

/// Project a dispatch-resolution table from the keymap-as-data vec.
#[must_use]
pub fn resolution_table(bound: &[BoundKey]) -> ResolutionTable {
    ResolutionTable { rows: bound.to_vec() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{keymap_bindings, registry, BindingContext};

    fn table() -> ResolutionTable {
        resolution_table(&keymap_bindings(&registry()))
    }

    #[test]
    fn kbg_ac07_table_is_a_faithful_projection_of_the_binding_vec() {
        let bound = keymap_bindings(&registry());
        let table = resolution_table(&bound);
        assert_eq!(table.rows(), bound.as_slice(), "table must equal the shipped binding vec");
    }

    #[test]
    fn kbg_ac07_global_bindings_resolve_from_both_workspace_and_editor() {
        let t = table();
        // toggle-focus (cmd-e) is Global — fires from both contexts.
        assert!(t.resolves("cmd-e", DispatchContext::CanvasFocused).contains(&"toggle-focus"));
        assert!(t.resolves("cmd-e", DispatchContext::EditorFocused).contains(&"toggle-focus"));
    }

    #[test]
    fn kbg_ac07_editor_suppresses_every_workspace_bare_verb() {
        let bound = keymap_bindings(&registry());
        let t = resolution_table(&bound);
        // No workspace-scoped binding resolves while the editor is focused.
        for b in &bound {
            if b.context == BindingContext::Workspace {
                assert!(
                    !t.resolves(b.keystrokes, DispatchContext::EditorFocused).contains(&b.longname),
                    "workspace verb {} leaked into editor focus",
                    b.longname
                );
            }
        }
    }

    #[test]
    fn kbg_ac07_no_bare_verb_resolves_under_an_open_overlay() {
        let bound = keymap_bindings(&registry());
        let t = resolution_table(&bound);
        for b in &bound {
            if b.context == BindingContext::Workspace {
                assert!(
                    !t.resolves(b.keystrokes, DispatchContext::OverlayOpen).contains(&b.longname),
                    "workspace verb {} resolved under an overlay",
                    b.longname
                );
            }
        }
    }

    #[test]
    fn kbg_ac07_plot_focus_resolves_a_bare_verb() {
        let t = table();
        // A canvas-focused bare verb (p) resolves.
        assert!(t.resolves("p", DispatchContext::CanvasFocused).contains(&"toggle-presentation"));
        assert!(t.resolves("c", DispatchContext::CanvasFocused).contains(&"cycle-colour-scheme"));
    }

    #[test]
    fn kbg_ac07_toggle_chord_is_unshadowed() {
        let bound = keymap_bindings(&registry());
        // Exactly one binding claims cmd-e, and it is the focus toggle.
        let claimers: Vec<_> = bound.iter().filter(|b| b.keystrokes == "cmd-e").collect();
        assert_eq!(claimers.len(), 1);
        assert_eq!(claimers[0].longname, "toggle-focus");
    }
}
