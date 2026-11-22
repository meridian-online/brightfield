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

/// The contexts the sweeps above cover, one a test.
const SWEPT: [BindingContext; 8] = [
    BindingContext::Global,
    BindingContext::Workspace,
    BindingContext::Protocol,
    BindingContext::Shelf,
    BindingContext::Grid,
    BindingContext::Versions,
    BindingContext::Browser,
    BindingContext::Editor,
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
