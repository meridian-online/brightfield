//! Workspace actions and key bindings (hosted by the
//! shell).
//!
//! Declares the [`TogglePresentation`] action, the workspace key context it
//! dispatches in, and the `p` binding as remappable data. The `WorkspaceView`
//! window root this file once hosted was superseded by the gpui-component
//! `DockArea` shell (#45) and has been deleted; the action's live
//! handler is the app crate's `CanvasPanel`.

use gpui::{actions, KeyBinding};

actions!(brightfield, [TogglePresentation]);

/// Key context of the workspace root — the scope the `p` binding dispatches
/// in. A future editor pane gets its own context (VisiData-style bare-letter
/// grammar, per the tabletop's locked pick).
pub const WORKSPACE_KEY_CONTEXT: &str = "BrightfieldWorkspace";

/// The workspace key bindings, declared as data (remappable config), not a
/// hardcoded key match: bare `p` → [`TogglePresentation`], scoped to
/// [`WORKSPACE_KEY_CONTEXT`]. `main` feeds these to `cx.bind_keys`.
pub fn workspace_key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "p",
        TogglePresentation,
        Some(WORKSPACE_KEY_CONTEXT),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Keystroke;

    /// Binding data: the workspace keymap is declared as data —
    /// exactly one binding, bare `p`, dispatching `TogglePresentation` inside
    /// the workspace key context. (The handler's flip is the tested
    /// state machine; physical keypress delivery is the manual eyeball.)
    #[test]
    fn p_binds_toggle_presentation_in_workspace_context() {
        let bindings = workspace_key_bindings();
        assert_eq!(bindings.len(), 1, "one action, one binding");
        let binding = &bindings[0];

        // Bare `p` matches the binding completely (Some(false) = no pending
        // keystrokes), and nothing else does.
        let p = Keystroke::parse("p").expect("parses");
        assert_eq!(
            binding.match_keystrokes(std::slice::from_ref(&p)),
            Some(false),
            "bare p is a complete match"
        );
        let q = Keystroke::parse("q").expect("parses");
        assert_eq!(binding.match_keystrokes(std::slice::from_ref(&q)), None);

        // It dispatches TogglePresentation, scoped to the workspace context.
        assert_eq!(binding.action().name(), "brightfield::TogglePresentation");
        assert!(
            binding.predicate().is_some(),
            "the binding is canvas-scoped, not global"
        );
    }
}
