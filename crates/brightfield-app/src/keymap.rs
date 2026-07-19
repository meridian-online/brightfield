//! The keymap adapter (ac-01 / ac-07): turns the gpui-free command registry
//! (`brightfield-keys`) into gpui `KeyBinding`s and actions.
//!
//! The registry is the single source of truth for the keymap. The two shipped
//! fixed points keep their original binding sites — bare `p` → `TogglePresentation`
//! (card 0016, `brightfield_ui::workspace_key_bindings`) and `cmd-s` → `SaveSpec`
//! (card 0017, `shell::editor_key_bindings`) — so their handlers are untouched;
//! the registry documents them (for the palette / help / provenance), and this
//! adapter sources every NEW grammar verb. `main` binds all three; their union is
//! exactly the registry keymap, and the dispatch-resolution table (ac-07, tested
//! headless in `brightfield-keys`) is a projection of that same registry.
//!
//! The new grammar actions are declared here; their live handlers land with the
//! focus model and surfaces (the macOS-eyeball ACs). Until a handler is attached,
//! a dispatched action simply does nothing — no crash, no render effect.

use crate::shell::EDITOR_KEY_CONTEXT;
use brightfield_keys::{keymap_bindings, registry, BindingContext, BoundKey};
use brightfield_ui::WORKSPACE_KEY_CONTEXT;
use gpui::{actions, KeyBinding};

actions!(
    brightfield,
    [
        /// Dive focus into the focused container (l / enter).
        DiveIn,
        /// Pop focus out to the parent (h / q).
        PopOut,
        /// Move focus to the next sibling view (j / tab).
        FocusNextSibling,
        /// Move focus to the previous sibling view (k / shift-tab).
        FocusPrevSibling,
        /// Toggle focus between canvas and editor (cmd-e, global).
        ToggleFocus,
        /// Fuzzy-jump focus to a component (/).
        FocusJump,
        /// Open the command palette (space; cmd-shift-p global twin).
        OpenPalette,
        /// Open the help overlay (?).
        OpenHelp,
        /// Clear the focused view's selection / run the Esc ladder (escape).
        ClearSelection,
        /// Reload the spec from disk with a dirty-buffer guard (cmd-r, global).
        ReloadSpec,
        /// Cycle the focused view's sequential colour scheme (c, transient).
        CycleColourScheme,
        /// Change the focused view's primary mark type (m, command log).
        ChangeMarkType,
        /// Add a mark to the focused view (a, argument overlay).
        AddMark,
        /// Bind a channel to a column on the focused view (e, argument overlay).
        SetChannel,
        /// Remove the focused view's primary mark (d, command log).
        RemoveMark,
        /// Undo the last uncommitted command-log edit (u).
        Undo
    ]
);

/// The gpui context predicate for the Protocol asset-graph grammar. The
/// Protocol panel now lives in the egui shell (`brightfield-shell`), so no
/// element in this gpui app declares this context — protocol bindings register
/// under it but stay inert here, never leaking into the chart grammar.
const PROTOCOL_KEY_CONTEXT: &str = "ProtocolPanel";

/// Map a framework-free [`BindingContext`] to a gpui context predicate string
/// (`None` = global, fires regardless of focus).
fn context_string(ctx: BindingContext) -> Option<&'static str> {
    match ctx {
        BindingContext::Workspace => Some(WORKSPACE_KEY_CONTEXT),
        BindingContext::Editor => Some(EDITOR_KEY_CONTEXT),
        BindingContext::Protocol => Some(PROTOCOL_KEY_CONTEXT),
        BindingContext::Global => None,
    }
}

/// Build a gpui `KeyBinding` for one registry [`BoundKey`]. The two shipped fixed
/// points (`toggle-presentation`, `save-spec`) return `None` — they keep their
/// card-0016/0017 binding sites — as does any longname without a live action, so
/// a new registry longname missing a case here is caught by the coverage test.
fn keybinding_for(bk: &BoundKey) -> Option<KeyBinding> {
    let ks = bk.keystrokes;
    let ctx = context_string(bk.context);
    Some(match bk.longname {
        "dive-in" => KeyBinding::new(ks, DiveIn, ctx),
        "pop-out" => KeyBinding::new(ks, PopOut, ctx),
        "focus-next-sibling" => KeyBinding::new(ks, FocusNextSibling, ctx),
        "focus-prev-sibling" => KeyBinding::new(ks, FocusPrevSibling, ctx),
        "toggle-focus" => KeyBinding::new(ks, ToggleFocus, ctx),
        "focus-jump" => KeyBinding::new(ks, FocusJump, ctx),
        "open-palette" => KeyBinding::new(ks, OpenPalette, ctx),
        "open-help" => KeyBinding::new(ks, OpenHelp, ctx),
        "clear-selection" => KeyBinding::new(ks, ClearSelection, ctx),
        "reload-spec" => KeyBinding::new(ks, ReloadSpec, ctx),
        "cycle-colour-scheme" => KeyBinding::new(ks, CycleColourScheme, ctx),
        // Command-log structural edits (card 0023).
        "change-mark-type" => KeyBinding::new(ks, ChangeMarkType, ctx),
        "add-mark" => KeyBinding::new(ks, AddMark, ctx),
        "set-channel" => KeyBinding::new(ks, SetChannel, ctx),
        "remove-mark" => KeyBinding::new(ks, RemoveMark, ctx),
        "undo" => KeyBinding::new(ks, Undo, ctx),
        // Shipped fixed points keep their original binding sites.
        "toggle-presentation" | "save-spec" => return None,
        // Protocol-context verbs are handled by the egui Protocol
        // panel, not this gpui adapter, and land here intentionally.
        _ => return None,
    })
}

/// Build a runnable gpui [`gpui::Action`] for a verb's `longname` — the command
/// palette's pick→run constructor (ac-12). Mirrors [`keybinding_for`]'s longname
/// list, but boxes an action rather than a `KeyBinding`. `None` for anything the
/// palette cannot RUN against the focused canvas: reserved (palette-visible,
/// unbound) verbs, and `save-spec` — whose handler lives on the editor subtree,
/// unreachable from a canvas-anchored `dispatch_action` (see
/// `shell::WorkspaceRoot::run_palette`). A new registry longname missing an arm
/// here is caught by the coverage test, exactly as `keybinding_for`'s is.
#[must_use]
pub fn action_for_longname(longname: &str) -> Option<Box<dyn gpui::Action>> {
    let action: Box<dyn gpui::Action> = match longname {
        "dive-in" => Box::new(DiveIn),
        "pop-out" => Box::new(PopOut),
        "focus-next-sibling" => Box::new(FocusNextSibling),
        "focus-prev-sibling" => Box::new(FocusPrevSibling),
        "toggle-focus" => Box::new(ToggleFocus),
        "focus-jump" => Box::new(FocusJump),
        "open-palette" => Box::new(OpenPalette),
        "open-help" => Box::new(OpenHelp),
        "clear-selection" => Box::new(ClearSelection),
        "reload-spec" => Box::new(ReloadSpec),
        "cycle-colour-scheme" => Box::new(CycleColourScheme),
        "change-mark-type" => Box::new(ChangeMarkType),
        "add-mark" => Box::new(AddMark),
        "set-channel" => Box::new(SetChannel),
        "remove-mark" => Box::new(RemoveMark),
        "undo" => Box::new(Undo),
        "toggle-presentation" => Box::new(brightfield_ui::TogglePresentation),
        // save-spec's handler is on the editor subtree (unreachable from a
        // canvas-anchored dispatch); reserved verbs are unbound.
        _ => return None,
    };
    Some(action)
}

/// The NEW grammar verbs as gpui bindings, sourced from the registry. `main` feeds
/// this alongside the two shipped fixed-point binding vecs; the union is the
/// registry keymap. Each binding carries its own context predicate.
#[must_use]
pub fn grammar_key_bindings() -> Vec<KeyBinding> {
    keymap_bindings(&registry()).iter().filter_map(keybinding_for).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Keystroke;

    /// The two shipped fixed points the adapter deliberately does not re-source.
    const SHIPPED_FIXED_POINTS: [&str; 2] = ["toggle-presentation", "save-spec"];

    fn matches_complete(bindings: &[KeyBinding], keystrokes: &str) -> Vec<KeyBinding> {
        let k = Keystroke::parse(keystrokes).expect("keystroke parses");
        bindings
            .iter()
            .filter(|b| b.match_keystrokes(std::slice::from_ref(&k)) == Some(false))
            .cloned()
            .collect()
    }

    #[test]
    fn kbg_ac01_adapter_covers_every_new_registry_binding() {
        // The adapter produces exactly the registry's chart-grammar bindings minus
        // the two shipped fixed points — no other longname is silently dropped.
        // Protocol-context verbs are excluded: that grammar is
        // dispatched by the egui Protocol panel (`brightfield-shell::protocol`)
        // through the registry directly, not by this gpui adapter.
        let want: usize = keymap_bindings(&registry())
            .iter()
            .filter(|bk| bk.context != BindingContext::Protocol)
            .filter(|bk| !SHIPPED_FIXED_POINTS.contains(&bk.longname))
            .count();
        assert_eq!(grammar_key_bindings().len(), want, "adapter dropped a new registry binding");
        // Card 0023 added m/a/e/d/u (5 keys): 23 registry keys − p − cmd-s = 21.
        assert_eq!(want, 21, "21 new grammar bindings (23 registry keys − p − cmd-s)");
    }

    #[test]
    fn kbg_ac12_action_for_longname_covers_runnable_verbs_only() {
        // Every runnable, canvas-reachable verb maps to its CORRECT action (a
        // wrong arm — e.g. pop-out => DiveIn — fails, not just a missing one); the
        // two non-runnable classes map to None: reserved (palette-visible, unbound)
        // and save-spec (editor-subtree handler, unreachable from canvas dispatch).
        let expected: &[(&str, &str)] = &[
            ("dive-in", "brightfield::DiveIn"),
            ("pop-out", "brightfield::PopOut"),
            ("focus-next-sibling", "brightfield::FocusNextSibling"),
            ("focus-prev-sibling", "brightfield::FocusPrevSibling"),
            ("toggle-focus", "brightfield::ToggleFocus"),
            ("focus-jump", "brightfield::FocusJump"),
            ("open-palette", "brightfield::OpenPalette"),
            ("open-help", "brightfield::OpenHelp"),
            ("clear-selection", "brightfield::ClearSelection"),
            ("reload-spec", "brightfield::ReloadSpec"),
            ("cycle-colour-scheme", "brightfield::CycleColourScheme"),
            ("change-mark-type", "brightfield::ChangeMarkType"),
            ("add-mark", "brightfield::AddMark"),
            ("set-channel", "brightfield::SetChannel"),
            ("remove-mark", "brightfield::RemoveMark"),
            ("undo", "brightfield::Undo"),
            ("toggle-presentation", "brightfield::TogglePresentation"),
        ];
        for &(longname, action_name) in expected {
            assert_eq!(
                action_for_longname(longname).unwrap().name(),
                action_name,
                "{longname} maps to the wrong action",
            );
        }
        // Completeness/drift: every runnable registry verb is in the table above,
        // and every non-runnable (reserved OR save-spec) maps to None.
        let table: std::collections::HashSet<&str> = expected.iter().map(|(l, _)| *l).collect();
        let reg = registry();
        for v in &reg {
            let is_protocol = v
                .binding_specs
                .iter()
                .any(|s| s.context == BindingContext::Protocol);
            if v.is_reserved() || v.longname == "save-spec" || is_protocol {
                // Reserved (unbound), save-spec (editor subtree), and the Protocol
                // grammar (served by the egui Protocol panel, not this gpui adapter)
                // are all deliberately not palette-runnable here.
                assert!(
                    action_for_longname(v.longname).is_none(),
                    "{} must not be palette-runnable",
                    v.longname
                );
            } else {
                assert!(
                    table.contains(v.longname),
                    "runnable verb {} is missing from the expected-action table",
                    v.longname
                );
            }
        }
        // A string that is not a registry verb → None.
        assert!(action_for_longname("not-a-verb").is_none());
    }

    #[test]
    fn kbg_ac07_fixed_points_are_left_to_their_shipped_binding_sites() {
        // The adapter does NOT re-bind p or cmd-s (covered by fww_ac07 / aws_ac04).
        let b = grammar_key_bindings();
        assert!(matches_complete(&b, "p").is_empty(), "p stays a card-0016 binding");
        assert!(matches_complete(&b, "cmd-s").is_empty(), "cmd-s stays a card-0017 binding");
    }

    #[test]
    fn kbg_ac09_focus_toggle_is_global_and_unshadowed() {
        let b = grammar_key_bindings();
        // cmd-e → ToggleFocus, GLOBAL (no context predicate → fires from anywhere).
        let claimers = matches_complete(&b, "cmd-e");
        assert_eq!(claimers.len(), 1, "exactly one binding claims cmd-e");
        assert_eq!(claimers[0].action().name(), "brightfield::ToggleFocus");
        assert!(claimers[0].predicate().is_none(), "toggle-focus is global (context=None)");
    }

    #[test]
    fn kbg_ac13_bare_c_binds_cycle_colour_scheme_in_workspace() {
        let b = grammar_key_bindings();
        let c = matches_complete(&b, "c");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].action().name(), "brightfield::CycleColourScheme");
        assert!(c[0].predicate().is_some(), "c is canvas-scoped, not global");
    }

    #[test]
    fn kbg_ac10_nav_keys_are_workspace_scoped_with_alternates() {
        let b = grammar_key_bindings();
        // Both the home-row key and its alternate bind the same nav action.
        for (key, alt, action) in [
            ("l", "enter", "brightfield::DiveIn"),
            ("h", "q", "brightfield::PopOut"),
            ("j", "tab", "brightfield::FocusNextSibling"),
            ("k", "shift-tab", "brightfield::FocusPrevSibling"),
        ] {
            for ks in [key, alt] {
                let m = matches_complete(&b, ks);
                assert_eq!(m.len(), 1, "one binding for {ks}");
                assert_eq!(m[0].action().name(), action, "{ks} → {action}");
                assert!(m[0].predicate().is_some(), "{ks} is canvas-scoped");
            }
        }
    }
}
