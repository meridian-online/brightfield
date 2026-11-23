//! The menu bar's rows as the window reads them: each command of
//! [`brightfield_keys::menu_bar`] with its title, its key, and the sentence it
//! says when the window cannot take it now.
//!
//! The window publishes the rows late in each frame ([`MenuState`]), and a
//! host's menus read them from there: the native bar on a thread of its own
//! turn, the menus drawn in the window on the frame after. A command chosen
//! from a menu goes back to the window on the open queue
//! ([`crate::open_queue::OpenQueue::ask_verb`]).

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use brightfield_keys::{
    availability, item_title, items_without_native, keys_label, menu_bar, registry, Availability,
    BindingContext, KeyPlatform, Menu, MenuItem, Situation, VerbEntry,
};

/// What a row a sheet stops says while the sheet waits on an answer
/// ([`Sheet::stops`]).
pub const SHEET_UP: &str = "Finish what the sheet over the window asks first.";

/// One command of the menu bar, as a menu draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuRow {
    /// The title of the menu the command is on.
    pub menu: &'static str,
    /// The command's longname.
    pub longname: &'static str,
    /// The item's title, with "…" where choosing it asks for something more.
    pub title: String,
    /// The keystrokes the registry binds the command to, as it spells them:
    /// its key in the Global context where it has one, else its first.
    pub keys: Option<&'static str>,
    /// The sentence the command says while it cannot act, which greys the
    /// item. `None` where it can.
    pub missing: Option<String>,
}

/// The key a menu shows for `verb`.
fn menu_key(verb: &VerbEntry) -> Option<&'static str> {
    verb.binding_specs
        .iter()
        .find(|b| b.context == BindingContext::Global)
        .or_else(|| verb.binding_specs.first())
        .map(|b| b.keystrokes)
}

/// What stands over the window when a menu is read or a command chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sheet {
    /// Nothing that waits on an answer: no overlay, or one of the pickers.
    None,
    /// A sheet that waits on an answer: Preferences, the sources browser and
    /// the like.
    Up,
    /// The question a close asks over unsaved work.
    CloseQuestion,
}

impl Sheet {
    /// Whether the sheet stops `longname` from a menu. A sheet stops the
    /// commands its own keys would not reach: Quit's key and Save's are read
    /// under a sheet, so their menu items stay live there, and Save stands
    /// down under the close question, which asks about saving itself.
    #[must_use]
    pub fn stops(self, longname: &str) -> bool {
        match self {
            Self::None => false,
            Self::Up => !matches!(longname, "quit" | "save-protocol"),
            Self::CloseQuestion => longname != "quit",
        }
    }
}

/// The part of a row that is the command list's and does not move with what
/// the window holds, with the command's registry entry. Built once.
fn fixed_rows() -> &'static [(MenuRow, Option<VerbEntry>)] {
    static ROWS: OnceLock<Vec<(MenuRow, Option<VerbEntry>)>> = OnceLock::new();
    ROWS.get_or_init(|| {
        let reg = registry();
        let mut rows = Vec::new();
        for menu in menu_bar() {
            for item in &menu.items {
                let MenuItem::Command { longname, asks } = *item else {
                    continue;
                };
                let verb = reg.iter().find(|v| v.longname == longname).cloned();
                rows.push((
                    MenuRow {
                        menu: menu.title,
                        longname,
                        title: item_title(longname, asks),
                        keys: verb.as_ref().and_then(menu_key),
                        missing: None,
                    },
                    verb,
                ));
            }
        }
        rows
    })
}

/// Every command of the menu bar, in the bar's order, read against
/// `situation`. A row `sheet` stops says [`SHEET_UP`].
///
/// A row's place in this list is the tag a native menu item carries, so the
/// list's order is the bar's and does not depend on `situation`.
#[must_use]
pub fn menu_rows(situation: &Situation, sheet: Sheet) -> Vec<MenuRow> {
    fixed_rows()
        .iter()
        .map(|(row, verb)| MenuRow {
            missing: if sheet.stops(row.longname) {
                Some(SHEET_UP.to_owned())
            } else {
                verb.as_ref()
                    .and_then(|v| match availability(v, situation) {
                        Availability::Available => None,
                        Availability::Missing(sentence) => Some(sentence),
                    })
            },
            ..row.clone()
        })
        .collect()
}

/// The rows the window last published, shared by clone with whatever draws
/// the menus.
#[derive(Debug, Clone, Default)]
pub struct MenuState(Arc<Mutex<Vec<MenuRow>>>);

impl MenuState {
    /// Replace the rows with `rows`, where they differ from the ones held.
    pub fn publish(&self, rows: Vec<MenuRow>) {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if *held != rows {
            *held = rows;
        }
    }

    /// The rows last published. Empty before the window's first frame.
    #[must_use]
    pub fn rows(&self) -> Vec<MenuRow> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Which menus a window's host draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuHost {
    /// The host has a menu bar of its own, which draws the menus: macOS.
    Native,
    /// The host has none, and the mark in the title band opens the menus.
    Mark,
}

impl MenuHost {
    /// The host this build runs on.
    #[must_use]
    pub const fn of_this_build() -> Self {
        if cfg!(target_os = "macos") {
            Self::Native
        } else {
            Self::Mark
        }
    }
}

/// One row the in-window menus drew on a frame: a menu's own title, or a
/// command in a menu that was open.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawnRow {
    /// The title of the menu the row belongs to.
    pub menu: &'static str,
    /// The command's longname. `None` for the menu's own title row.
    pub longname: Option<&'static str>,
    /// Where the row was drawn.
    pub rect: egui::Rect,
    /// Whether the row can be chosen.
    pub enabled: bool,
    /// What a greyed row says under the pointer: its command's sentence.
    pub hover: Option<String>,
    /// The key the row shows, as the host writes keys.
    pub keys: Option<String>,
}

/// **Draw `menus` as a column of menu buttons**, each opening its commands
/// from `rows`, and answer the command chosen this frame. A command is greyed
/// while its row says a sentence, which it shows under the pointer; the window
/// items a platform supplies are left out. What was drawn is pushed to `drawn`.
pub fn show_menus(
    ui: &mut egui::Ui,
    menus: &[Menu],
    rows: &[MenuRow],
    platform: KeyPlatform,
    drawn: &mut Vec<DrawnRow>,
) -> Option<&'static str> {
    let mut chosen = None;
    for menu in menus {
        let items = items_without_native(menu);
        let opened = ui.menu_button(menu.title, |ui| {
            for item in &items {
                let longname = match *item {
                    MenuItem::Separator => {
                        ui.separator();
                        continue;
                    }
                    MenuItem::Native(_) => continue,
                    MenuItem::Command { longname, .. } => longname,
                };
                let Some(row) = rows.iter().find(|r| r.longname == longname) else {
                    continue;
                };
                let keys = row.keys.map(|keys| keys_label(keys, platform));
                let mut button = egui::Button::new(row.title.as_str());
                if let Some(keys) = &keys {
                    button = button.shortcut_text(keys.as_str());
                }
                let mut response = ui.add_enabled(row.missing.is_none(), button);
                if let Some(sentence) = &row.missing {
                    response = response.on_disabled_hover_text(sentence.as_str());
                }
                drawn.push(DrawnRow {
                    menu: menu.title,
                    longname: Some(longname),
                    rect: response.rect,
                    enabled: row.missing.is_none(),
                    hover: row.missing.clone(),
                    keys,
                });
                if response.clicked() {
                    chosen = Some(longname);
                }
            }
        });
        drawn.push(DrawnRow {
            menu: menu.title,
            longname: None,
            rect: opened.response.rect,
            enabled: true,
            hover: None,
            keys: None,
        });
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_are_the_bars_commands_in_order_whatever_the_window_holds() {
        let nothing = menu_rows(&Situation::default(), Sheet::None);
        let sheet = menu_rows(&Situation::default(), Sheet::Up);
        let names = |rows: &[MenuRow]| rows.iter().map(|r| r.longname).collect::<Vec<_>>();
        assert_eq!(names(&nothing), names(&sheet));
        let commands: usize = menu_bar()
            .iter()
            .flat_map(|m| &m.items)
            .filter(|i| matches!(i, MenuItem::Command { .. }))
            .count();
        assert_eq!(nothing.len(), commands);
        assert_eq!(nothing[0].menu, "Brightfield");
    }

    /// A sheet greys the rows its keys would not reach: Quit's key, and
    /// Save's outside the close question, are read under a sheet.
    #[test]
    fn a_sheet_greys_each_row_but_the_two_whose_keys_it_leaves() {
        let holding_work = Situation {
            conditions: vec![brightfield_keys::Condition::UnsavedWork],
            ..Situation::default()
        };
        for (sheet, live) in [
            (Sheet::Up, &["quit", "save-protocol"][..]),
            (Sheet::CloseQuestion, &["quit"][..]),
        ] {
            for row in menu_rows(&holding_work, sheet) {
                if live.contains(&row.longname) {
                    assert_eq!(row.missing, None, "{} under {sheet:?}", row.longname);
                } else {
                    assert_eq!(
                        row.missing.as_deref(),
                        Some(SHEET_UP),
                        "{} under {sheet:?}",
                        row.longname
                    );
                }
            }
        }
    }

    #[test]
    fn a_row_shows_the_global_key_where_the_command_has_one() {
        let rows = menu_rows(&Situation::default(), Sheet::None);
        let row = |name: &str| {
            rows.iter()
                .find(|r| r.longname == name)
                .unwrap_or_else(|| panic!("{name} is on no menu"))
        };
        assert_eq!(row("open-palette").keys, Some("cmd-shift-p"));
        assert_eq!(row("open").keys, Some("cmd-o"));
        assert_eq!(row("open").title, "Open…");
        assert_eq!(row("zoom-in").keys, Some("="));
        assert_eq!(
            row("zoom-in").missing.as_deref(),
            Some("Zoom in needs a chart on the canvas.")
        );
        assert_eq!(row("open").missing, None);
    }

    #[test]
    fn the_state_hands_back_what_was_published() {
        let state = MenuState::default();
        assert!(state.rows().is_empty());
        let rows = menu_rows(&Situation::default(), Sheet::None);
        state.clone().publish(rows.clone());
        assert_eq!(state.rows(), rows);
    }
}
