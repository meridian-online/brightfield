//! The Mac's menu bar, built from the command list.
//!
//! Each menu of [`brightfield_keys::menu_bar`] after the first becomes an
//! `NSMenu` on the main menu; the first is the app menu winit makes, which
//! [`crate::mac_open::route_menu_quit`] routes. A command's item carries the
//! command's place in [`crate::menu_bar::menu_rows`] as its tag. As a menu
//! opens, each item is enabled or greyed, with its sentence as the tooltip,
//! from the rows the window last published ([`MenuState`]); an item chosen
//! puts its command on the window's open queue
//! ([`OpenQueue::ask_verb`]), where the window performs or refuses it as it
//! does one chosen anywhere else.
//!
//! **The menus claim no key.** An item shows its command's key, and the
//! menus' delegate answers that no menu has a key equivalent for an event, so
//! AppKit performs no item for a key press and each key reaches the window,
//! where the registry's own handlers read it. A key typed into a text field
//! is therefore typed, and a chord the editor takes is the editor's.
//!
//! The AppKit messages are sent by selector, as [`crate::mac_open`] sends
//! them, so the crate declares no AppKit classes.

use std::sync::atomic::{AtomicBool, Ordering};

use brightfield_keys::{
    keys_label, parse_keystrokes, KeyPlatform, Keystroke, Menu, MenuItem, NativeItem, Situation,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{class, define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker};
use objc2_foundation::NSString;

use crate::menu_bar::{menu_rows, MenuRow, MenuState};
use crate::open_queue::OpenQueue;

/// `NSEventModifierFlagShift`.
const SHIFT: usize = 1 << 17;
/// `NSEventModifierFlagControl`.
const CONTROL: usize = 1 << 18;
/// `NSEventModifierFlagOption`.
const OPTION: usize = 1 << 19;
/// `NSEventModifierFlagCommand`.
const COMMAND: usize = 1 << 20;

/// The tag of an item that is not a command: a window item AppKit answers.
const NO_COMMAND: isize = -1;

/// The key equivalent AppKit shows for one keystroke, and its modifier mask:
/// a letter in lower case with the shift mask for ⇧, and AppKit's own
/// characters for the named keys. `None` for a key AppKit has no character
/// for, whose item then shows none.
fn key_equivalent(stroke: &Keystroke) -> Option<(String, usize)> {
    let key = match stroke.key {
        "enter" => "\r".to_owned(),
        "escape" => "\u{1b}".to_owned(),
        "tab" => "\t".to_owned(),
        "space" => " ".to_owned(),
        "backspace" => "\u{8}".to_owned(),
        "up" => "\u{F700}".to_owned(),
        "down" => "\u{F701}".to_owned(),
        "left" => "\u{F702}".to_owned(),
        "right" => "\u{F703}".to_owned(),
        key => {
            let function = key
                .strip_prefix('f')
                .and_then(|n| n.parse::<u32>().ok())
                .filter(|n| (1..=12).contains(n));
            if let Some(n) = function {
                char::from_u32(0xF704 + n - 1)?.to_string()
            } else if key.chars().count() == 1 {
                key.to_lowercase()
            } else {
                return None;
            }
        }
    };
    let mut mask = 0;
    for (held, flag) in [
        (stroke.shift, SHIFT),
        (stroke.control, CONTROL),
        (stroke.alt, OPTION),
        (stroke.command, COMMAND),
    ] {
        if held {
            mask |= flag;
        }
    }
    Some((key, mask))
}

/// The one keystroke of `row`'s key AppKit can show beside the title, when
/// its key is a single stroke.
fn single_stroke(row: &MenuRow) -> Option<(String, usize)> {
    match parse_keystrokes(row.keys?).as_slice() {
        [stroke] => key_equivalent(stroke),
        _ => None,
    }
}

/// The title of `row`'s item. A command whose key is a chord of strokes has
/// no key equivalent AppKit can show, so the chord is written after the
/// title: "Remove step  D D".
fn native_title(row: &MenuRow) -> String {
    match row.keys {
        Some(keys) if parse_keystrokes(keys).len() > 1 => {
            format!("{}  {}", row.title, keys_label(keys, KeyPlatform::Mac))
        }
        _ => row.title.clone(),
    }
}

/// A window item's title, and the action AppKit answers it with.
fn native_item(item: NativeItem) -> (&'static str, Sel) {
    match item {
        NativeItem::Minimize => ("Minimize", sel!(performMiniaturize:)),
        NativeItem::Zoom => ("Zoom", sel!(performZoom:)),
        NativeItem::BringAllToFront => ("Bring All to Front", sel!(arrangeInFront:)),
    }
}

struct Ivars {
    queue: OpenQueue,
    state: MenuState,
    /// Each command's longname at its tag: its place in
    /// [`crate::menu_bar::menu_rows`].
    names: Vec<&'static str>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `MenuDelegate` does not implement Drop.
    #[unsafe(super(NSObject))]
    // Named for this app, so no other class of the same runtime name can make defining it fail.
    #[name = "BrightfieldMenuDelegate"]
    #[ivars = Ivars]
    struct MenuDelegate;

    impl MenuDelegate {
        /// A menu is about to open: grey what the window cannot take now.
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: Option<&AnyObject>) {
            let Some(menu) = menu else {
                return;
            };
            let rows = self.ivars().state.rows();
            // SAFETY: `numberOfItems`, `itemAtIndex:`, `tag`, `setEnabled:` and `setToolTip:` are
            // NSMenu's and NSMenuItem's, called with the types they declare; AppKit calls this
            // method on the main thread, and each item is retained while it is used.
            unsafe {
                let items: isize = msg_send![menu, numberOfItems];
                for at in 0..items {
                    let item: Option<Retained<AnyObject>> = msg_send![menu, itemAtIndex: at];
                    let Some(item) = item else {
                        continue;
                    };
                    let tag: isize = msg_send![&*item, tag];
                    let Some(row) = usize::try_from(tag).ok().and_then(|tag| rows.get(tag)) else {
                        continue;
                    };
                    let () = msg_send![&*item, setEnabled: row.missing.is_none()];
                    let tip = row.missing.as_deref().map(NSString::from_str);
                    let () = msg_send![&*item, setToolTip: tip.as_deref()];
                }
            }
        }

        /// No menu has a key equivalent for an event, so AppKit performs no
        /// item for a key press and the key reaches the window. The last
        /// argument is AppKit's `SEL *`, taken as an untyped pointer since it
        /// is not read.
        #[unsafe(method(menuHasKeyEquivalent:forEvent:target:action:))]
        fn menu_has_key_equivalent(
            &self,
            _menu: Option<&AnyObject>,
            _event: Option<&AnyObject>,
            _target: *mut *mut AnyObject,
            _action: *mut std::ffi::c_void,
        ) -> bool {
            false
        }

        #[unsafe(method(menuItemChosen:))]
        fn menu_item_chosen(&self, sender: Option<&AnyObject>) {
            let Some(sender) = sender else {
                return;
            };
            // SAFETY: the sender of a menu item's action is the NSMenuItem, whose `tag` is an
            // NSInteger.
            let tag: isize = unsafe { msg_send![sender, tag] };
            self.choose(tag);
        }
    }
);

impl MenuDelegate {
    fn new(queue: OpenQueue, state: MenuState, names: Vec<&'static str>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            queue,
            state,
            names,
        });
        // SAFETY: NSObject's `init` is its designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// Put the command at `tag` on the window's queue. A tag that names no
    /// command asks nothing.
    fn choose(&self, tag: isize) {
        let name = usize::try_from(tag)
            .ok()
            .and_then(|tag| self.ivars().names.get(tag));
        if let Some(name) = name {
            self.ivars().queue.ask_verb(name);
        }
    }
}

/// Whether the menus have been built, so a second call builds no second set.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// **Build the menu bar's menus after the app menu** from `menus`, greyed from
/// `state` as each opens, with a chosen command put on `queue`. Call it on the
/// main thread, which `_main` proves, once the event loop has made the main
/// menu. Answers whether the menus are built: `false` when there is no main
/// menu to add them to yet.
pub fn install(
    _main: MainThreadMarker,
    menus: &[Menu],
    queue: OpenQueue,
    state: MenuState,
) -> bool {
    if INSTALLED.load(Ordering::SeqCst) {
        return true;
    }
    // The titles and keys are the command list's and do not move with what the
    // window holds, so they are read once, against nothing held.
    let rows = menu_rows(&Situation::default(), false);
    let names: Vec<&'static str> = rows.iter().map(|row| row.longname).collect();
    // SAFETY: `sharedApplication` and `mainMenu` are NSApplication's, called on the main thread.
    let (app, bar) = unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![class!(NSApplication), sharedApplication];
        let bar: Option<Retained<AnyObject>> = match &app {
            Some(app) => msg_send![&**app, mainMenu],
            None => None,
        };
        (app, bar)
    };
    let (Some(app), Some(bar)) = (app, bar) else {
        return false;
    };
    let delegate = MenuDelegate::new(queue, state, names);
    // A command's tag is its place among every menu's commands, the app
    // menu's included, which is its place in `menu_rows`.
    let mut tag = 0_usize;
    for (at, menu) in menus.iter().enumerate() {
        let commands = menu
            .items
            .iter()
            .filter(|item| matches!(item, MenuItem::Command { .. }))
            .count();
        if at == 0 {
            tag += commands;
            continue;
        }
        // SAFETY: NSMenu's and NSMenuItem's `new`, `setTitle:`, `setAutoenablesItems:`,
        // `setDelegate:`, `addItem:`, `setSubmenu:`, `separatorItem`, `setAction:`, `setTarget:`,
        // `setTag:`, `setKeyEquivalent:` and `setKeyEquivalentModifierMask:`, and NSApplication's
        // `setWindowsMenu:` and `setHelpMenu:`, are AppKit's, called with the types they declare
        // on the main thread. A menu retains the items added to it and an item its submenu; the
        // delegate, which a menu and an item hold without retaining, is kept for the life of the
        // process below.
        unsafe {
            let title = NSString::from_str(menu.title);
            let native: Option<Retained<AnyObject>> = msg_send![class!(NSMenu), new];
            let holder: Option<Retained<AnyObject>> = msg_send![class!(NSMenuItem), new];
            let (Some(native), Some(holder)) = (native, holder) else {
                tag += commands;
                continue;
            };
            let () = msg_send![&*native, setTitle: &*title];
            let () = msg_send![&*native, setAutoenablesItems: false];
            let () = msg_send![&*native, setDelegate: &*delegate];
            for item in &menu.items {
                let entry: Option<Retained<AnyObject>> = match item {
                    MenuItem::Separator => msg_send![class!(NSMenuItem), separatorItem],
                    MenuItem::Native(_) | MenuItem::Command { .. } => {
                        msg_send![class!(NSMenuItem), new]
                    }
                };
                let Some(entry) = entry else {
                    if matches!(item, MenuItem::Command { .. }) {
                        tag += 1;
                    }
                    continue;
                };
                match item {
                    MenuItem::Separator => {}
                    MenuItem::Native(which) => {
                        let (words, action) = native_item(*which);
                        let words = NSString::from_str(words);
                        let () = msg_send![&*entry, setTitle: &*words];
                        let () = msg_send![&*entry, setAction: action];
                        let () = msg_send![&*entry, setTag: NO_COMMAND];
                    }
                    MenuItem::Command { .. } => {
                        if let Some(row) = rows.get(tag) {
                            let words = NSString::from_str(&native_title(row));
                            let () = msg_send![&*entry, setTitle: &*words];
                            if let Some((key, mask)) = single_stroke(row) {
                                let key = NSString::from_str(&key);
                                let () = msg_send![&*entry, setKeyEquivalent: &*key];
                                let () = msg_send![&*entry, setKeyEquivalentModifierMask: mask];
                            }
                        }
                        let () = msg_send![&*entry, setAction: sel!(menuItemChosen:)];
                        let () = msg_send![&*entry, setTarget: &*delegate];
                        let () = msg_send![&*entry, setTag: tag.cast_signed()];
                        tag += 1;
                    }
                }
                let () = msg_send![&*native, addItem: &*entry];
            }
            let () = msg_send![&*holder, setTitle: &*title];
            let () = msg_send![&*holder, setSubmenu: &*native];
            let () = msg_send![&*bar, addItem: &*holder];
            match menu.title {
                "Window" => {
                    let () = msg_send![&*app, setWindowsMenu: &*native];
                }
                "Help" => {
                    let () = msg_send![&*app, setHelpMenu: &*native];
                }
                _ => {}
            }
        }
    }
    // The menus and their items hold the delegate without retaining it, for the life of the
    // process.
    std::mem::forget(delegate);
    INSTALLED.store(true, Ordering::SeqCst);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(keys: &'static str) -> Keystroke {
        let strokes = parse_keystrokes(keys);
        assert_eq!(strokes.len(), 1, "{keys} is one stroke");
        strokes[0]
    }

    fn row(longname: &str) -> MenuRow {
        menu_rows(&Situation::default(), false)
            .into_iter()
            .find(|r| r.longname == longname)
            .unwrap_or_else(|| panic!("{longname} is on no menu"))
    }

    #[test]
    fn single_keystrokes_become_key_equivalents_with_their_modifier_mask() {
        assert_eq!(
            key_equivalent(&stroke("cmd-shift-p")),
            Some(("p".to_owned(), SHIFT | COMMAND))
        );
        assert_eq!(
            key_equivalent(&stroke("cmd-enter")),
            Some(("\r".to_owned(), COMMAND))
        );
        assert_eq!(
            key_equivalent(&stroke("f2")),
            Some(("\u{F705}".to_owned(), 0))
        );
        assert_eq!(key_equivalent(&stroke("w")), Some(("w".to_owned(), 0)));
        assert_eq!(
            key_equivalent(&stroke("shift-d")),
            Some(("d".to_owned(), SHIFT))
        );
        assert_eq!(key_equivalent(&stroke("pageup")), None);
    }

    #[test]
    fn a_chord_is_written_after_the_title_and_claims_no_equivalent() {
        let remove = row("remove-step");
        assert_eq!(remove.keys, Some("d d"));
        assert_eq!(native_title(&remove), "Remove step  D D");
        assert_eq!(single_stroke(&remove), None);

        let open = row("open");
        assert_eq!(native_title(&open), "Open…");
        assert_eq!(single_stroke(&open), Some(("o".to_owned(), COMMAND)));
    }

    #[test]
    fn the_menus_claim_no_key_so_every_key_reaches_the_window() {
        let delegate = MenuDelegate::new(OpenQueue::default(), MenuState::default(), Vec::new());
        let no_object: Option<&AnyObject> = None;
        // SAFETY: the selector is the delegate's own method above, sent the argument types it
        // declares; it reads none of them.
        let claimed: bool = unsafe {
            msg_send![
                &*delegate,
                menuHasKeyEquivalent: no_object,
                forEvent: no_object,
                target: std::ptr::null_mut::<*mut AnyObject>(),
                action: std::ptr::null_mut::<std::ffi::c_void>(),
            ]
        };
        assert!(!claimed);
    }

    #[test]
    fn choosing_an_item_queues_its_verb() {
        let rows = menu_rows(&Situation::default(), false);
        let names: Vec<&'static str> = rows.iter().map(|r| r.longname).collect();
        let run = names
            .iter()
            .position(|name| *name == "run-protocol")
            .expect("Run is on a menu");
        let queue = OpenQueue::default();
        let delegate = MenuDelegate::new(queue.clone(), MenuState::default(), names);
        delegate.choose(run.cast_signed());
        delegate.choose(NO_COMMAND);
        delegate.choose(isize::MAX);
        assert_eq!(queue.take_verbs(), ["run-protocol"]);
    }

    /// The tags `install` gives the items count the app menu's commands first,
    /// so a tag is a row's place in `menu_rows`.
    #[test]
    fn the_first_menu_after_the_app_menu_starts_past_the_app_menus_commands() {
        let menus = brightfield_keys::menu_bar();
        let rows = menu_rows(&Situation::default(), false);
        let app_commands = menus[0]
            .items
            .iter()
            .filter(|item| matches!(item, MenuItem::Command { .. }))
            .count();
        assert_eq!(rows[app_commands].menu, menus[1].title);
        assert_eq!(rows[app_commands - 1].menu, menus[0].title);
    }
}
