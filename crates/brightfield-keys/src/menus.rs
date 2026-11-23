//! The menu bar as data: eight menus, each a list of commands from the
//! registry, separators, and the window items the platform supplies.
//!
//! A menu item names a command by its longname and says whether choosing it
//! asks for something more before it acts, which is what puts "…" after its
//! title. The title itself is the command's label from the registry, the name
//! its availability sentences use; the palette and the help sheet still write
//! its longname. A motion (a cursor step, a pan) is not a menu item: it is in
//! the help sheet and the palette.

use crate::registry::registry;

/// One menu of the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    /// The menu's title, as the bar writes it.
    pub title: &'static str,
    /// The menu's items, top to bottom.
    pub items: Vec<MenuItem>,
}

/// One row of a menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    /// A command from the registry.
    Command {
        /// The command's longname.
        longname: &'static str,
        /// Whether choosing it asks for something more before it acts. Such an
        /// item's title ends in "…".
        asks: bool,
    },
    /// A rule between groups of items.
    Separator,
    /// A window item the platform supplies and answers itself. Drawn on macOS
    /// only.
    Native(NativeItem),
}

/// A window item the platform supplies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeItem {
    /// Minimise the window.
    Minimize,
    /// Zoom the window.
    Zoom,
    /// Bring the app's windows in front of other apps'.
    BringAllToFront,
}

const fn acts(longname: &'static str) -> MenuItem {
    MenuItem::Command {
        longname,
        asks: false,
    }
}

const fn asks(longname: &'static str) -> MenuItem {
    MenuItem::Command {
        longname,
        asks: true,
    }
}

/// The menu bar: Brightfield, File, Edit, View, Protocol, Data, Window, Help.
#[must_use]
pub fn menu_bar() -> Vec<Menu> {
    use MenuItem::{Native, Separator};
    vec![
        Menu {
            title: "Brightfield",
            items: vec![asks("open-preferences"), acts("quit")],
        },
        Menu {
            title: "File",
            items: vec![
                asks("new-protocol"),
                asks("open"),
                acts("open-home"),
                Separator,
                acts("save-protocol"),
                asks("rename-protocol"),
                Separator,
                asks("export-chart-picture"),
            ],
        },
        Menu {
            title: "Edit",
            items: vec![
                acts("undo"),
                Separator,
                acts("copy-chart-picture"),
                acts("yank-address"),
                Separator,
                acts("clear-selection"),
            ],
        },
        Menu {
            title: "View",
            items: vec![
                asks("open-palette"),
                Separator,
                acts("zoom-in"),
                acts("zoom-out"),
                acts("reset-extent"),
                acts("cycle-axis-lock"),
                Separator,
                acts("show-model-diagram"),
                acts("show-model-list"),
                acts("show-model-file"),
                Separator,
                acts("open-steps-sheet"),
            ],
        },
        Menu {
            title: "Protocol",
            items: vec![
                acts("run-protocol"),
                Separator,
                asks("open-sources-browser"),
                asks("organise-joins"),
                acts("remove-step"),
                asks("focus-jump"),
                Separator,
                acts("reload-data"),
                acts("untrust-protocol"),
            ],
        },
        Menu {
            title: "Data",
            items: vec![
                asks("filter-rows-at-cursor"),
                asks("sort-rows-at-cursor"),
                Separator,
                acts("set-channel"),
                acts("put-column-on-x"),
                acts("put-column-on-y"),
                acts("put-column-on-colour"),
                Separator,
                acts("keep-join"),
                acts("drop-join"),
                asks("add-join"),
                asks("change-join-keys"),
                Separator,
                asks("set-role"),
                asks("fold-table"),
                acts("unfold-table"),
                acts("leave-out-table"),
                acts("include-table"),
                asks("retype-column"),
                Separator,
                acts("rediscover-model"),
            ],
        },
        Menu {
            title: "Window",
            items: vec![
                Native(NativeItem::Minimize),
                Native(NativeItem::Zoom),
                Separator,
                acts("toggle-outline-rail"),
                acts("move-grid"),
                Separator,
                Native(NativeItem::BringAllToFront),
            ],
        },
        Menu {
            title: "Help",
            items: vec![acts("open-help")],
        },
    ]
}

/// A menu's items on a host that draws no native window items: the
/// [`MenuItem::Native`] rows dropped, and with them any separator left leading,
/// trailing or doubled.
#[must_use]
pub fn items_without_native(menu: &Menu) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = Vec::new();
    for item in &menu.items {
        match item {
            MenuItem::Native(_) => {}
            MenuItem::Separator => {
                if items
                    .last()
                    .is_some_and(|last| *last != MenuItem::Separator)
                {
                    items.push(*item);
                }
            }
            MenuItem::Command { .. } => items.push(*item),
        }
    }
    if items.last() == Some(&MenuItem::Separator) {
        items.pop();
    }
    items
}

/// A menu item's title: the command's label, with "…" when choosing it asks
/// for something more. A longname the registry lacks is written as itself.
#[must_use]
pub fn item_title(longname: &str, asks: bool) -> String {
    let reg = registry();
    let label = reg
        .iter()
        .find(|v| v.longname == longname)
        .map_or(longname, |v| v.label);
    if asks {
        format!("{label}…")
    } else {
        label.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn commands(menu: &Menu) -> Vec<&'static str> {
        menu.items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Command { longname, .. } => Some(*longname),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_menu_bar_has_the_eight_menus_in_order() {
        let titles: Vec<&str> = menu_bar().iter().map(|m| m.title).collect();
        assert_eq!(
            titles,
            [
                "Brightfield",
                "File",
                "Edit",
                "View",
                "Protocol",
                "Data",
                "Window",
                "Help"
            ]
        );
    }

    #[test]
    fn every_menu_command_is_a_built_verb_with_a_key() {
        let reg = registry();
        let mut seen = 0;
        for menu in menu_bar() {
            for longname in commands(&menu) {
                seen += 1;
                let verb = reg
                    .iter()
                    .find(|v| v.longname == longname)
                    .unwrap_or_else(|| panic!("{}: {longname} is not in the registry", menu.title));
                assert!(!verb.is_reserved(), "{longname} is reserved");
                assert!(!verb.binding_specs.is_empty(), "{longname} binds no key");
            }
        }
        assert!(seen > 0, "the menu bar holds no command");
    }

    #[test]
    fn no_command_is_in_two_menus() {
        let mut seen = BTreeSet::new();
        for menu in menu_bar() {
            for longname in commands(&menu) {
                assert!(seen.insert(longname), "{longname} is in two menus");
            }
        }
    }

    #[test]
    fn every_menu_holds_a_command() {
        // So no menu is empty on a host where the native items are not drawn.
        for menu in menu_bar() {
            assert!(!commands(&menu).is_empty(), "{} is empty", menu.title);
        }
    }

    fn assert_rules_sit_between_items(title: &str, items: &[MenuItem]) {
        let is_rule = |item: &MenuItem| *item == MenuItem::Separator;
        assert!(
            !items.first().is_some_and(is_rule),
            "{title} leads with a separator"
        );
        assert!(
            !items.last().is_some_and(is_rule),
            "{title} trails with a separator"
        );
        assert!(
            !items
                .windows(2)
                .any(|pair| is_rule(&pair[0]) && is_rule(&pair[1])),
            "{title} doubles a separator"
        );
    }

    #[test]
    fn no_separator_leads_trails_or_doubles() {
        for menu in menu_bar() {
            assert_rules_sit_between_items(menu.title, &menu.items);
        }
    }

    #[test]
    fn no_separator_leads_trails_or_doubles_without_the_native_items() {
        for menu in menu_bar() {
            let items = items_without_native(&menu);
            assert!(
                !items.iter().any(|i| matches!(i, MenuItem::Native(_))),
                "{} keeps a native item",
                menu.title
            );
            assert_rules_sit_between_items(menu.title, &items);
        }
        let window = menu_bar()
            .into_iter()
            .find(|m| m.title == "Window")
            .expect("a Window menu");
        assert_eq!(
            items_without_native(&window),
            [acts("toggle-outline-rail"), acts("move-grid")]
        );
    }

    #[test]
    fn item_title_adds_the_ellipsis_only_when_it_asks() {
        assert_eq!(item_title("open", true), "Open…");
        assert_eq!(item_title("run-protocol", false), "Run");
    }
}
