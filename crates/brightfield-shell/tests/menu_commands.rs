//! A command chosen from a menu reaches its handler: the window takes it off
//! the open queue, performs it where it can act, and says the command's
//! sentence where it cannot. The rows a menu draws carry each command's title,
//! key and that sentence.

#[path = "support/scenes.rs"]
mod scenes;

use brightfield_keys::{registry, Availability, BindingContext, Need, Selected, Situation};
use brightfield_shell::menu_bar::{menu_rows, Sheet, SHEET_UP};

#[test]
fn every_menu_command_acts_where_it_is_available() {
    for row in menu_rows(&Situation::default(), Sheet::None) {
        let mut win = scenes::scene_for(row.longname, BindingContext::Global);
        win.choose(row.longname);
        assert!(
            win.applied.contains(&row.longname),
            "{} is on the {} menu and nothing performed it: the window applied {:?}; {}",
            row.longname,
            row.menu,
            win.applied,
            win.said()
        );
    }
}

#[test]
fn a_command_chosen_where_it_is_unavailable_says_its_sentence_and_does_nothing() {
    let mut win = scenes::data_file();
    let toasts = win.app.toasts().len();
    let Some(Availability::Missing(sentence)) = win.app.availability_of("undo") else {
        panic!("Undo is available with no column kept");
    };
    win.choose("undo");
    assert_eq!(win.applied, Vec::<&str>::new());
    assert_eq!(win.app.menu_said(), Some(sentence.as_str()));
    assert!(win.app.toasts().len() > toasts, "nothing said why");
}

#[test]
fn on_the_front_door_every_command_that_needs_a_protocol_is_greyed() {
    let win = scenes::door();
    let reg = registry();
    let rows = win.app.menu_rows();
    for row in &rows {
        let verb = reg
            .iter()
            .find(|v| v.longname == row.longname)
            .expect("a menu command is registered");
        let needs_a_protocol = verb.needs.iter().any(|r| {
            matches!(
                r.need,
                Need::Selected(_) | Need::Condition(_) | Need::Sources(_) | Need::Column(_)
            )
        });
        if needs_a_protocol {
            assert_eq!(
                Some(Availability::Missing(row.missing.clone().unwrap_or_else(
                    || panic!("{} is live on the front door", row.longname)
                ))),
                win.app.availability_of(row.longname),
                "{}",
                row.longname
            );
        } else {
            assert_eq!(row.missing, None, "{} is greyed on the door", row.longname);
        }
    }
    let row = |name: &str| rows.iter().find(|r| r.longname == name).expect("on a menu");
    assert_eq!(
        row("run-protocol").missing.as_deref(),
        Some("Run needs a Protocol to act on: open one, or open a data file or a folder.")
    );
    assert_eq!(
        row("open-home").missing.as_deref(),
        Some("You are on the front door already.")
    );
    assert!(
        verb_needs_protocol_somewhere(&reg),
        "no menu command needs a Protocol"
    );
}

fn verb_needs_protocol_somewhere(reg: &[brightfield_keys::VerbEntry]) -> bool {
    reg.iter().any(|v| {
        v.needs
            .iter()
            .any(|r| r.need == Need::Selected(Selected::Protocol))
    })
}

#[test]
fn no_menu_command_acts_behind_a_sheet() {
    let mut win = scenes::data_file();
    win.choose("open-preferences");
    assert_eq!(win.app.open_overlay(), Some("preferences"));
    for row in win.app.menu_rows() {
        // Quit's key and Save's are read under a sheet, so their items stay
        // as the window reads them.
        if matches!(row.longname, "quit" | "save-protocol") {
            assert_ne!(row.missing.as_deref(), Some(SHEET_UP), "{}", row.longname);
        } else {
            assert_eq!(row.missing.as_deref(), Some(SHEET_UP), "{}", row.longname);
        }
    }
    assert_eq!(
        win.app.menu_state().rows(),
        win.app.menu_rows(),
        "the frame published what the window reads"
    );
    win.choose("run-protocol");
    assert_eq!(win.applied, Vec::<&str>::new());
    assert_eq!(win.app.open_overlay(), Some("preferences"));
    // Quit's key is read under a sheet, and so is its menu item.
    win.choose("quit");
    assert_eq!(win.applied, ["quit"]);
}

#[test]
fn a_command_chosen_under_a_picker_closes_the_picker_first() {
    let mut win = scenes::data_file();
    win.press("space");
    assert_eq!(win.app.open_overlay(), Some("palette"));
    assert!(
        win.app.menu_rows().iter().any(|r| r.missing.is_none()),
        "a picker greys the menus"
    );
    win.choose("zoom-in");
    assert_eq!(win.applied, ["zoom-in"]);
    assert_eq!(win.app.open_overlay(), None);
}

#[test]
fn the_menu_rows_carry_each_commands_title_key_and_sentence() {
    let win = scenes::live_chart("crossfilter.yaml");
    let rows = win.app.menu_rows();
    let row = |name: &str| rows.iter().find(|r| r.longname == name).expect("on a menu");
    assert_eq!(row("open").title, "Open…");
    assert_eq!(row("open").keys, Some("cmd-o"));
    assert_eq!(row("open").menu, "File");
    assert_eq!(row("open").missing, None);
    assert_eq!(row("zoom-in").missing, None);
    // A chart spec has no Protocol for Run to act on.
    assert_eq!(
        row("run-protocol").missing.as_deref(),
        Some("Run needs a Protocol to act on: open one, or open a data file or a folder.")
    );
}
