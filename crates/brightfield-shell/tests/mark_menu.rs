//! The menus on a host with no menu bar: the mark in the title band opens the
//! same eight menus, greyed and keyed from the same rows, and a command chosen
//! there is applied or refused as one chosen from a native menu is.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_keys::menu_bar;
use brightfield_shell::menu_bar::{DrawnRow, MenuHost};
use scenes::Window;

/// Say the host has no menu bar, and let a frame draw the mark as its menus'.
fn on_a_host_with_no_menu_bar(mut win: Window) -> Window {
    win.app.set_menu_host(MenuHost::Mark);
    win.settle();
    win
}

/// Click the mark, which opens its menus.
fn open_the_mark(win: &mut Window) {
    let mark = win.app.mark_rect().expect("the title band drew the mark");
    win.click(mark.center());
}

/// The row the menus drew for `longname`, or for `menu`'s own title.
fn drawn(win: &Window, menu: &str, longname: Option<&str>) -> DrawnRow {
    win.app
        .mark_menu_drawn()
        .iter()
        .find(|row| row.menu == menu && row.longname == longname)
        .unwrap_or_else(|| {
            panic!(
                "the menus drew no {longname:?} row on {menu}: {:?}",
                win.app
                    .mark_menu_drawn()
                    .iter()
                    .map(|r| (r.menu, r.longname))
                    .collect::<Vec<_>>()
            )
        })
        .clone()
}

/// Open the mark's menus and then `menu`.
fn open_menu(win: &mut Window, menu: &str) {
    open_the_mark(win);
    let title = drawn(win, menu, None);
    win.click(title.rect.center());
}

#[test]
fn the_mark_opens_the_eight_menus_on_a_host_with_no_menu_bar() {
    let mut win = on_a_host_with_no_menu_bar(scenes::data_file());
    assert!(
        win.app.mark_menu_drawn().is_empty(),
        "the menus are drawn before the mark is clicked"
    );
    open_the_mark(&mut win);
    let titles: Vec<&str> = win
        .app
        .mark_menu_drawn()
        .iter()
        .filter(|row| row.longname.is_none())
        .map(|row| row.menu)
        .collect();
    let bar: Vec<&str> = menu_bar().iter().map(|menu| menu.title).collect();
    assert_eq!(titles, bar);
}

#[test]
fn the_mark_is_drawn_on_the_front_door_where_it_opens_the_menus() {
    let native = scenes::door();
    assert_eq!(
        native.app.mark_rect().is_some(),
        native.app.menu_host() == MenuHost::Mark,
        "on a host with a menu bar the door draws no mark"
    );
    let mut win = on_a_host_with_no_menu_bar(scenes::door());
    assert!(win.app.mark_rect().is_some(), "the door drew no mark");
    open_menu(&mut win, "File");
    let home = drawn(&win, "File", Some("open-home"));
    assert!(!home.enabled);
    assert_eq!(
        home.hover.as_deref(),
        Some("You are on the front door already.")
    );
    assert!(drawn(&win, "File", Some("open")).enabled);
}

#[test]
fn a_greyed_row_says_its_sentence_and_choosing_it_does_nothing() {
    let mut win = on_a_host_with_no_menu_bar(scenes::live_chart("crossfilter.yaml"));
    open_menu(&mut win, "Protocol");
    let run = drawn(&win, "Protocol", Some("run-protocol"));
    assert!(!run.enabled, "Run is live over a chart spec");
    assert_eq!(
        run.hover.as_deref(),
        Some("Run needs a Protocol to act on: open one, or open a data file or a folder.")
    );
    win.applied.clear();
    win.click(run.rect.center());
    assert_eq!(win.applied, Vec::<&str>::new());
}

#[test]
fn choosing_a_row_applies_its_command() {
    let mut win = on_a_host_with_no_menu_bar(scenes::data_file());
    open_menu(&mut win, "View");
    let zoom = drawn(&win, "View", Some("zoom-in"));
    assert!(
        zoom.enabled,
        "Zoom in is greyed over a chart: {:?}",
        zoom.hover
    );
    win.applied.clear();
    win.click(zoom.rect.center());
    assert_eq!(win.applied, ["zoom-in"]);
}

#[test]
fn keys_read_ctrl_on_a_host_with_no_menu_bar() {
    let mut win = on_a_host_with_no_menu_bar(scenes::data_file());
    open_menu(&mut win, "File");
    assert_eq!(
        drawn(&win, "File", Some("open")).keys.as_deref(),
        Some("Ctrl+O")
    );
    assert_eq!(drawn(&win, "File", Some("open")).menu, "File");
    // The Window menu's native items are the platform's, and are not drawn.
    let title = drawn(&win, "Window", None);
    win.click(title.rect.center());
    let window: Vec<_> = win
        .app
        .mark_menu_drawn()
        .iter()
        .filter(|row| row.menu == "Window")
        .filter_map(|row| row.longname)
        .collect();
    assert_eq!(
        window,
        [
            "toggle-outline-rail",
            "toggle-inspector-rail",
            "move-grid",
            "next-place",
            "prev-place",
            "close-place"
        ]
    );
}

#[test]
fn on_a_host_with_a_menu_bar_the_mark_still_goes_home() {
    let mut win = scenes::data_file();
    win.app.set_menu_host(MenuHost::Native);
    win.settle();
    let mark = win.app.mark_rect().expect("the title band drew the mark");
    assert_eq!(win.app.home_rect(), Some(mark));
    win.click(mark.center());
    assert!(win.app.front_door_is_live(), "the mark did not go home");
    assert!(win.app.mark_menu_drawn().is_empty());
}

/// The open menus stand over the window as an overlay does: a key pressed
/// while they are open is theirs, so Esc closes them and the chart keeps its
/// selection, and a bare key the chart reads does nothing beneath them.
#[test]
fn keys_pressed_over_the_open_menus_are_not_the_windows() {
    use brightfield_keys::{Condition, Need};

    let mut win = on_a_host_with_no_menu_bar(scenes::live_chart("crossfilter.yaml"));
    assert!(
        scenes::arrange(&mut win, Need::Condition(Condition::SelectionHeld)),
        "the fixture took a selection"
    );
    open_the_mark(&mut win);
    assert!(!win.app.mark_menu_drawn().is_empty(), "the menus are open");

    win.press("=");
    assert_eq!(
        win.answered,
        Vec::<&str>::new(),
        "a key zoomed under the menus"
    );
    assert!(
        !win.app.mark_menu_drawn().is_empty(),
        "`=` closed the menus"
    );

    // A host that makes a copy of the chord in place of its key event.
    win.answered.clear();
    let held = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    win.frame(held, vec![egui::Event::Copy]);
    win.settle();
    assert_eq!(
        win.answered,
        Vec::<&str>::new(),
        "a copy acted under the menus"
    );

    win.press("escape");
    assert_eq!(win.answered, Vec::<&str>::new());
    assert!(
        win.app.chart_doc().selection_active(),
        "Esc over the open menus let go of the selection"
    );
    assert!(
        win.app.mark_menu_drawn().is_empty(),
        "Esc left the menus open"
    );

    // With the menus closed the key is the window's again.
    win.press("escape");
    assert_eq!(win.answered, ["clear-selection"]);
}
