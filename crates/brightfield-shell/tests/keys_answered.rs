//! Every key the help sheet lists reaches a handler: for each binding of a
//! built command, a window in which the command can act is given the binding's
//! keystrokes, and the window records the command as the one its key performed.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_keys::{registry, BindingContext};

fn sweep(context: BindingContext) {
    let mut swept = 0;
    for verb in registry().iter().filter(|v| !v.is_reserved()) {
        for binding in verb.binding_specs.iter().filter(|b| b.context == context) {
            swept += 1;
            let mut win = scenes::scene_for(verb.longname, context);
            scenes::hold_keys(&mut win, context, verb.longname);
            win.press(binding.keystrokes);
            assert!(
                win.answered.contains(&verb.longname),
                "{} is listed for {} in {context:?} and no handler took it: the window answered \
                 {:?}",
                binding.keystrokes,
                verb.longname,
                win.answered
            );
        }
    }
    assert!(swept > 0, "no built command binds a key in {context:?}");
}

#[test]
fn every_global_key_is_answered() {
    sweep(BindingContext::Global);
}

#[test]
fn every_workspace_key_is_answered() {
    sweep(BindingContext::Workspace);
}

#[test]
fn every_protocol_key_is_answered() {
    sweep(BindingContext::Protocol);
}

#[test]
fn every_shelf_key_is_answered() {
    sweep(BindingContext::Shelf);
}

#[test]
fn every_grid_key_is_answered() {
    sweep(BindingContext::Grid);
}

#[test]
fn every_versions_key_is_answered() {
    sweep(BindingContext::Versions);
}

#[test]
fn every_browser_key_is_answered() {
    sweep(BindingContext::Browser);
}

#[test]
fn every_editor_key_is_answered() {
    sweep(BindingContext::Editor);
}

/// The contexts the sweeps cover, one a test: the eight above, and the
/// spine's pick, swept beside its own tests where a pick can be opened
/// (`tests/spine_pick.rs`, `every_spine_pick_key_is_answered`).
const SWEPT: [BindingContext; 9] = [
    BindingContext::Global,
    BindingContext::Workspace,
    BindingContext::Protocol,
    BindingContext::Shelf,
    BindingContext::Grid,
    BindingContext::Versions,
    BindingContext::Browser,
    BindingContext::Editor,
    BindingContext::SpinePick,
];

#[test]
fn every_context_a_binding_uses_is_swept() {
    for verb in registry().iter().filter(|v| !v.is_reserved()) {
        for binding in &verb.binding_specs {
            assert!(
                SWEPT.contains(&binding.context),
                "{} binds {} in {:?}, a context no sweep presses",
                verb.longname,
                binding.keystrokes,
                binding.context
            );
        }
    }
}

/// The palette's chord opens it while a text field holds the keys, where
/// Space is typed into the field.
#[test]
fn the_palette_chord_opens_it_from_a_text_field() {
    let mut win = scenes::data_file();
    scenes::hold_keys(&mut win, BindingContext::Editor, "save-spec");
    assert!(
        win.ctx.memory(|m| m.focused()).is_some(),
        "the editor's text holds the keyboard"
    );
    win.press("space");
    assert_eq!(win.app.open_overlay(), None, "Space opened the palette");
    win.press("cmd-shift-p");
    assert_eq!(win.app.open_overlay(), Some("palette"));
    assert_eq!(win.answered, ["open-palette"]);
}
