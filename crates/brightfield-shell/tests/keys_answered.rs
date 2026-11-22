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
            let mut win = scenes::scene_for(verb.longname);
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
