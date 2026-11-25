//! The ribbon's keys move its tab and its collapse over an open Protocol, the
//! menu's and the palette's choice of them does the same, and on the front
//! door, where no ribbon is drawn, they move nothing.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_keys::ribbon::Stage;
use brightfield_shell::ribbon::{RibbonPick, RibbonState};

#[test]
fn the_tab_keys_select_their_tabs_over_a_protocol() {
    let mut win = scenes::data_file();
    assert_eq!(win.app.ribbon(), RibbonState::default());
    win.press("alt-m");
    assert_eq!(win.answered, ["show-model-tab"]);
    assert_eq!(win.app.ribbon().selected, RibbonPick::Stage(Stage::Model));
    win.press("alt-c");
    assert_eq!(win.app.ribbon().selected, RibbonPick::Stage(Stage::Connect));
    win.press("alt-p");
    assert_eq!(win.app.ribbon().selected, RibbonPick::Stage(Stage::Publish));
    win.press("alt-x");
    assert_eq!(win.app.ribbon().selected, RibbonPick::Context);
    win.press("alt-s");
    assert_eq!(win.app.ribbon().selected, RibbonPick::Stage(Stage::Shape));
}

#[test]
fn the_collapse_key_and_its_menu_command_fold_the_ribbon_and_open_it() {
    let mut win = scenes::data_file();
    win.press("alt-r");
    assert_eq!(win.answered, ["toggle-ribbon"]);
    assert!(win.app.ribbon().collapsed);
    win.choose("toggle-ribbon");
    assert_eq!(win.applied, ["toggle-ribbon"]);
    assert!(!win.app.ribbon().collapsed);
}

#[test]
fn on_the_front_door_the_ribbon_keys_move_nothing() {
    let mut win = scenes::door();
    for keys in ["alt-m", "alt-r"] {
        win.press(keys);
    }
    assert_eq!(win.app.ribbon(), RibbonState::default());
}
