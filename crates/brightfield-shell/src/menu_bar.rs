//! The menu bar's rows as the window reads them: each command of
//! [`brightfield_keys::menu_bar`] with its title, its key, and the sentence it
//! says when the window cannot take it now.
//!
//! The window publishes the rows as each frame ends ([`MenuState`]), and a
//! host's menus read them from there: the native bar on a thread of its own
//! turn, the menus drawn in the window on the frame after. A command chosen
//! from a menu goes back to the window on the open queue
//! ([`crate::open_queue::OpenQueue::ask_verb`]).

use std::sync::{Arc, Mutex, PoisonError};

use brightfield_keys::{
    availability, item_title, menu_bar, registry, Availability, BindingContext, MenuItem,
    Situation, VerbEntry,
};

/// What every row says while a sheet over the window waits on an answer.
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

/// Every command of the menu bar, in the bar's order, read against
/// `situation`. With `sheet_up`, each row says [`SHEET_UP`].
///
/// A row's place in this list is the tag a native menu item carries, so the
/// list's order is the bar's and does not depend on `situation`.
#[must_use]
pub fn menu_rows(situation: &Situation, sheet_up: bool) -> Vec<MenuRow> {
    let reg = registry();
    let mut rows = Vec::new();
    for menu in menu_bar() {
        for item in &menu.items {
            let MenuItem::Command { longname, asks } = *item else {
                continue;
            };
            let verb = reg.iter().find(|v| v.longname == longname);
            let missing = if sheet_up {
                Some(SHEET_UP.to_owned())
            } else {
                verb.and_then(|v| match availability(v, situation) {
                    Availability::Available => None,
                    Availability::Missing(sentence) => Some(sentence),
                })
            };
            rows.push(MenuRow {
                menu: menu.title,
                longname,
                title: item_title(longname, asks),
                keys: verb.and_then(menu_key),
                missing,
            });
        }
    }
    rows
}

/// The rows the window last published, shared by clone with whatever draws
/// the menus.
#[derive(Debug, Clone, Default)]
pub struct MenuState(Arc<Mutex<Vec<MenuRow>>>);

impl MenuState {
    /// Replace the rows with `rows`.
    pub fn publish(&self, rows: Vec<MenuRow>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = rows;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_are_the_bars_commands_in_order_whatever_the_window_holds() {
        let nothing = menu_rows(&Situation::default(), false);
        let sheet = menu_rows(&Situation::default(), true);
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

    #[test]
    fn a_sheet_greys_every_row_with_one_sentence() {
        for row in menu_rows(&Situation::default(), true) {
            assert_eq!(row.missing.as_deref(), Some(SHEET_UP), "{}", row.longname);
        }
    }

    #[test]
    fn a_row_shows_the_global_key_where_the_command_has_one() {
        let rows = menu_rows(&Situation::default(), false);
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
        let rows = menu_rows(&Situation::default(), false);
        state.clone().publish(rows.clone());
        assert_eq!(state.rows(), rows);
    }
}
