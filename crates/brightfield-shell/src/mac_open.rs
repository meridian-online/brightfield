//! The Mac's open events, onto the window's open queue.
//!
//! A file double-clicked in Finder, chosen under Open With or dropped on the Dock icon reaches an
//! app as an open-documents Apple Event, and a `brightfield://` link as a get-URL one. winit does
//! not forward either from the application delegate it installs, so this module registers its own
//! handlers with `NSAppleEventManager`. Each file the event names is pushed as
//! [`OpenRequest::Path`] and each link as [`OpenRequest::Link`], and the window opens them on its
//! next frame.
//!
//! [`install`] runs before the event loop starts, so the event a launch by double-click delivers
//! has a handler to land in. AppKit installs handlers of its own for the core events while the app
//! finishes launching, which would replace one registered earlier, so the handlers are registered
//! again when the app posts that it will finish launching and once more when it has: the
//! documents a launch opens are delivered between the two, and every later event after both.
//!
//! **Quitting comes here too.** The Dock's Quit, and a logout, reach an app as a quit Apple Event,
//! and the app menu's Quit item, on Cmd+Q, calls `terminate:` on the application, which ends the
//! process without the window's close request: neither would ask the close question over a
//! draft. The quit event's handler is replaced with one that asks the window to quit on its queue
//! ([`OpenQueue::ask_quit`]), and so is the menu item's action once the menu exists
//! ([`route_menu_quit`]). The window then asks, and closes itself once answered, which ends the
//! app as closing its one window always has. The same call adds Preferences… on Cmd+, to the app
//! menu, which asks the window to open Preferences on its queue
//! ([`OpenQueue::ask_preferences`]).
//!
//! **Becoming the default app comes here too**, since it is the crate's other Objective-C call:
//! [`ask_default_app`] asks the workspace to make the app the default for a content type, which
//! macOS confirms with the analyst itself ([`crate::default_app`] says when it is asked).
//!
//! The Apple Event methods that take a four-character code are sent by selector rather than
//! through the `objc2-foundation` methods that declare them, which need a feature whose crate the
//! dependency graph does not otherwise hold. The workspace, bundle and content-type classes are
//! sent messages by selector for the same reason.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject, Sel};
use objc2::{class, define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker};
use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSString};

use crate::open_queue::{OpenQueue, OpenRequest};

/// A four-character code, as the Apple Event Manager spells its classes, ids and types.
const fn code(c: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*c)
}

/// `kCoreEventClass`.
const CORE_EVENT_CLASS: u32 = code(b"aevt");
/// `kAEOpenDocuments`.
const OPEN_DOCUMENTS: u32 = code(b"odoc");
/// `kAEQuitApplication`.
const QUIT_APPLICATION: u32 = code(b"quit");
/// `kInternetEventClass` and `kAEGetURL`, which share their code.
const GET_URL: u32 = code(b"GURL");
/// `keyDirectObject`: the parameter an event's subject is in.
const DIRECT_OBJECT: u32 = code(b"----");
/// `typeAEList`.
const LIST: u32 = code(b"list");

/// The moment AppKit has finished launching, by which it has delivered the documents a launch
/// opens, named as AppKit posts it.
const DID_FINISH_LAUNCHING: &str = "NSApplicationDidFinishLaunchingNotification";

/// The two moments AppKit may have replaced the handlers by, named as AppKit posts them.
const LAUNCH_NOTIFICATIONS: [&str; 2] = [
    "NSApplicationWillFinishLaunchingNotification",
    DID_FINISH_LAUNCHING,
];

struct Ivars {
    queue: OpenQueue,
    wake: Box<dyn Fn() + Send + Sync>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `OpenHandler` does not implement Drop.
    #[unsafe(super(NSObject))]
    // Named for this app, so no other class of the same runtime name can make defining it fail.
    #[name = "BrightfieldOpenHandler"]
    #[ivars = Ivars]
    struct OpenHandler;

    impl OpenHandler {
        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn handle_open_documents(
            &self,
            event: &NSAppleEventDescriptor,
            _reply: Option<&NSAppleEventDescriptor>,
        ) {
            // The files of one event are one gesture, such as several chosen together in Finder,
            // so they go onto the queue together.
            if let Some(request) = OpenRequest::of_paths(documents(event)) {
                self.ivars().queue.push(request);
            }
            (self.ivars().wake)();
        }

        #[unsafe(method(handleGetURL:withReplyEvent:))]
        fn handle_get_url(
            &self,
            event: &NSAppleEventDescriptor,
            _reply: Option<&NSAppleEventDescriptor>,
        ) {
            if let Some(link) = direct_object(event).and_then(|d| d.stringValue()) {
                self.ivars().queue.push(OpenRequest::Link(link.to_string()));
            }
            (self.ivars().wake)();
        }

        #[unsafe(method(handleQuit:withReplyEvent:))]
        fn handle_quit(
            &self,
            _event: &NSAppleEventDescriptor,
            _reply: Option<&NSAppleEventDescriptor>,
        ) {
            self.ivars().queue.ask_quit();
            (self.ivars().wake)();
        }

        #[unsafe(method(quitFromMenu:))]
        fn quit_from_menu(&self, _sender: Option<&AnyObject>) {
            self.ivars().queue.ask_quit();
            (self.ivars().wake)();
        }

        #[unsafe(method(preferencesFromMenu:))]
        fn preferences_from_menu(&self, _sender: Option<&AnyObject>) {
            self.ivars().queue.ask_preferences();
            (self.ivars().wake)();
        }

        #[unsafe(method(appFinishingLaunch:))]
        fn app_finishing_launch(&self, note: &NSNotification) {
            self.register();
            self.launch_moment(&note.name().to_string());
        }
    }
);

impl OpenHandler {
    fn new(queue: OpenQueue, wake: Box<dyn Fn() + Send + Sync>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { queue, wake });
        // SAFETY: NSObject's `init` is its designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// Mark the queue launched when `name` is AppKit's post that it has finished launching: the
    /// documents a launch opens are delivered before it, so from here the queue holds every one
    /// of them. The post before it, that it will finish, leaves the queue waiting.
    fn launch_moment(&self, name: &str) {
        if name == DID_FINISH_LAUNCHING {
            self.ivars().queue.mark_launched();
            (self.ivars().wake)();
        }
    }

    /// Make this object the handler for the open, link and quit events, replacing whichever was
    /// registered.
    fn register(&self) {
        let manager = NSAppleEventManager::sharedAppleEventManager();
        let handlers: [(Sel, u32, u32); 3] = [
            (
                sel!(handleOpenDocuments:withReplyEvent:),
                CORE_EVENT_CLASS,
                OPEN_DOCUMENTS,
            ),
            (sel!(handleGetURL:withReplyEvent:), GET_URL, GET_URL),
            (
                sel!(handleQuit:withReplyEvent:),
                CORE_EVENT_CLASS,
                QUIT_APPLICATION,
            ),
        ];
        for (selector, class, id) in handlers {
            // SAFETY: `self` responds to `selector`, which takes the event and its reply as the
            // manager calls it, and the manager does not retain the handler: `install` keeps it
            // alive for the life of the process.
            let () = unsafe {
                msg_send![
                    &*manager,
                    setEventHandler: self,
                    andSelector: selector,
                    forEventClass: class,
                    andEventID: id
                ]
            };
        }
    }
}

/// The event's direct object, the parameter its subject is in.
fn direct_object(event: &NSAppleEventDescriptor) -> Option<Retained<NSAppleEventDescriptor>> {
    // SAFETY: `paramDescriptorForKeyword:` takes an `AEKeyword`, a four-character code.
    unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] }
}

/// The files an open-documents event names: its direct object, a list of file descriptors, or
/// one on its own. An entry that is not a file is skipped.
fn documents(event: &NSAppleEventDescriptor) -> Vec<std::path::PathBuf> {
    let Some(direct) = direct_object(event) else {
        return Vec::new();
    };
    // SAFETY: `descriptorType` returns a `DescType`, a four-character code.
    let kind: u32 = unsafe { msg_send![&*direct, descriptorType] };
    let items: Vec<Retained<NSAppleEventDescriptor>> = if kind == LIST {
        // A list descriptor counts its items from one.
        (1..=direct.numberOfItems())
            .filter_map(|i| direct.descriptorAtIndex(i))
            .collect()
    } else {
        vec![direct]
    };
    items
        .iter()
        // `fileURLValue` coerces an alias or bookmark to a file URL where it can.
        .filter_map(|item| item.fileURLValue())
        .filter_map(|url| url.to_file_path())
        .collect()
}

/// Listen for the Mac's open events for the rest of the process, pushing each onto `queue` and
/// then calling `wake` so a frame runs. It runs on the main thread, which `_main` proves, before
/// the event loop starts.
pub fn install(_main: MainThreadMarker, queue: OpenQueue, wake: impl Fn() + Send + Sync + 'static) {
    register_for_life(queue, wake);
}

/// [`install`] without the proof of the main thread, which a test thread cannot give. The handlers
/// are the process's, so a later call replaces an earlier one's.
fn register_for_life(queue: OpenQueue, wake: impl Fn() + Send + Sync + 'static) {
    let handler = OpenHandler::new(queue, Box::new(wake));
    handler.register();
    let center = objc2_foundation::NSNotificationCenter::defaultCenter();
    for name in LAUNCH_NOTIFICATIONS {
        let name = NSString::from_str(name);
        // SAFETY: `appFinishingLaunch:` takes the notification, and the handler outlives the
        // centre's reference to it because it is never released.
        unsafe {
            center.addObserver_selector_name_object(
                &handler,
                sel!(appFinishingLaunch:),
                Some(&name),
                None,
            );
        }
    }
    // Neither the event manager nor the notification centre keeps the handler alive, and both
    // call it for as long as the process runs.
    std::mem::forget(handler);
}

/// **Have the app menu's Quit ask the window to quit on `queue`** rather than end the process, so
/// Cmd+Q asks the close question over a draft. Call it on the main thread, which `_main` proves,
/// once the event loop has made the menu: the first frame is late enough. Answers whether a Quit
/// item was found and routed; a menu without one is left as it is.
///
/// The item is the one whose action is `terminate:`, in any of the main menu's submenus, which is
/// where winit's default menu puts it. That submenu is the app menu, and Preferences… on Cmd+, is
/// added to it after its first item, About, between separators as the Mac lays it out, unless an
/// item already has that action.
pub fn route_menu_quit(
    _main: MainThreadMarker,
    queue: OpenQueue,
    wake: impl Fn() + Send + Sync + 'static,
) -> bool {
    // SAFETY: `sharedApplication`, `mainMenu`, `numberOfItems`, `itemAtIndex:`, `submenu` and
    // `action` are NSApplication's, NSMenu's and NSMenuItem's, called with the types they declare
    // on the main thread; each object returned is retained for as long as it is used here.
    let quit_item = unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![class!(NSApplication), sharedApplication];
        let menu: Option<Retained<AnyObject>> = match &app {
            Some(app) => msg_send![&**app, mainMenu],
            None => None,
        };
        let mut found: Option<(Retained<AnyObject>, Retained<AnyObject>)> = None;
        if let Some(menu) = menu {
            let items: isize = msg_send![&*menu, numberOfItems];
            'menus: for i in 0..items {
                let item: Option<Retained<AnyObject>> = msg_send![&*menu, itemAtIndex: i];
                let sub: Option<Retained<AnyObject>> = match &item {
                    Some(item) => msg_send![&**item, submenu],
                    None => None,
                };
                let Some(sub) = sub else {
                    continue;
                };
                let entries: isize = msg_send![&*sub, numberOfItems];
                for j in 0..entries {
                    let entry: Option<Retained<AnyObject>> = msg_send![&*sub, itemAtIndex: j];
                    let Some(entry) = entry else {
                        continue;
                    };
                    let action: Option<Sel> = msg_send![&*entry, action];
                    if action == Some(sel!(terminate:)) {
                        found = Some((entry, sub.clone()));
                        break 'menus;
                    }
                }
            }
        }
        found
    };
    let Some((item, app_menu)) = quit_item else {
        return false;
    };
    let handler = OpenHandler::new(queue, Box::new(wake));
    // SAFETY: `numberOfItems`, `itemAtIndex:`, `action`, `separatorItem`, `insertItem:atIndex:`,
    // and NSMenuItem's `new`, `setTitle:`, `setAction:`, `setKeyEquivalent:` and `setTarget:` are
    // AppKit's, called with the types they declare on the main thread; the menu retains what is
    // inserted, and the handler is kept for the life of the process below.
    unsafe {
        let entries: isize = msg_send![&*app_menu, numberOfItems];
        let mut present = false;
        for j in 0..entries {
            let entry: Option<Retained<AnyObject>> = msg_send![&*app_menu, itemAtIndex: j];
            if let Some(entry) = entry {
                let action: Option<Sel> = msg_send![&*entry, action];
                present |= action == Some(sel!(preferencesFromMenu:));
            }
        }
        if !present {
            let title = NSString::from_str("Preferences\u{2026}");
            let key = NSString::from_str(",");
            let preferences: Option<Retained<AnyObject>> = msg_send![class!(NSMenuItem), new];
            if let Some(preferences) = preferences {
                let () = msg_send![&*preferences, setTitle: &*title];
                let () = msg_send![&*preferences, setAction: sel!(preferencesFromMenu:)];
                let () = msg_send![&*preferences, setKeyEquivalent: &*key];
                let () = msg_send![&*preferences, setTarget: &*handler];
                let at = entries.min(1);
                let () = msg_send![&*app_menu, insertItem: &*preferences, atIndex: at];
                let separator: Option<Retained<AnyObject>> =
                    msg_send![class!(NSMenuItem), separatorItem];
                if let Some(separator) = separator {
                    let () = msg_send![&*app_menu, insertItem: &*separator, atIndex: at];
                }
            }
        }
    }
    // SAFETY: `setTarget:` and `setAction:` are NSMenuItem's; the handler responds to
    // `quitFromMenu:`, which takes the sender, and the menu item does not retain its target, so
    // the handler is kept for the life of the process below.
    unsafe {
        let () = msg_send![&*item, setTarget: &*handler];
        let () = msg_send![&*item, setAction: sel!(quitFromMenu:)];
    }
    std::mem::forget(handler);
    true
}

/// Whether an open panel [`begin_open_panel`] raised is still up, so a second Cmd+O while it is
/// does not raise another over it.
static PANEL_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `NSModalResponseOK`, the answer an open panel gives when the analyst chose Open.
const MODAL_RESPONSE_OK: isize = 1;

/// **Raise the Mac's open panel, choosing files and folders in the one panel, and return at
/// once.** The panel is attached to the key window as a sheet when there is one, and runs on its
/// own otherwise; either way AppKit runs it from the event loop the window's frames run in, so
/// the window goes on drawing behind it and nothing beachballs. When the analyst chooses Open,
/// `answer` is called on the main thread with the chosen paths; a cancel calls nothing.
///
/// Files are offered only with one of `extensions`, and every folder can be chosen. Answers
/// whether a panel was raised: `false` when one is already up, or off the main thread.
pub fn begin_open_panel(
    message: &str,
    extensions: &[&str],
    answer: impl Fn(Vec<std::path::PathBuf>) + 'static,
) -> bool {
    use std::sync::atomic::Ordering;
    if MainThreadMarker::new().is_none() || PANEL_UP.swap(true, Ordering::SeqCst) {
        return false;
    }
    // SAFETY: each message below is NSOpenPanel's, NSSavePanel's, NSMutableArray's or
    // NSApplication's, sent on the main thread (checked above) with the argument types AppKit
    // declares: `BOOL` setters, an `NSString`, an array of strings, an `NSWindow` and a block
    // taking an `NSModalResponse`.
    unsafe {
        let panel: Option<Retained<AnyObject>> = msg_send![class!(NSOpenPanel), openPanel];
        let Some(panel) = panel else {
            PANEL_UP.store(false, Ordering::SeqCst);
            return false;
        };
        let () = msg_send![&*panel, setCanChooseFiles: true];
        let () = msg_send![&*panel, setCanChooseDirectories: true];
        let () = msg_send![&*panel, setAllowsMultipleSelection: true];
        let () = msg_send![&*panel, setResolvesAliases: true];
        let message = NSString::from_str(message);
        let () = msg_send![&*panel, setMessage: &*message];
        let types: Retained<AnyObject> = msg_send![class!(NSMutableArray), array];
        for ext in extensions {
            let ext = NSString::from_str(ext);
            let () = msg_send![&*types, addObject: &*ext];
        }
        // Deprecated in favour of content types, which need a framework the graph does not
        // hold; it still filters, and leaves folders choosable.
        let () = msg_send![&*panel, setAllowedFileTypes: &*types];

        let chosen = panel.clone();
        let done = block2::RcBlock::new(move |response: isize| {
            PANEL_UP.store(false, Ordering::SeqCst);
            if response != MODAL_RESPONSE_OK {
                return;
            }
            let paths = panel_paths(&chosen);
            if !paths.is_empty() {
                answer(paths);
            }
        });
        let app: Option<Retained<AnyObject>> = msg_send![class!(NSApplication), sharedApplication];
        let window: Option<Retained<AnyObject>> = match &app {
            Some(app) => msg_send![&**app, keyWindow],
            None => None,
        };
        match window {
            Some(window) => {
                let () = msg_send![
                    &*panel,
                    beginSheetModalForWindow: &*window,
                    completionHandler: &*done
                ];
            }
            None => {
                let () = msg_send![&*panel, beginWithCompletionHandler: &*done];
            }
        }
    }
    true
}

/// The file paths an open panel's chosen URLs name, in the order it lists them.
fn panel_paths(panel: &AnyObject) -> Vec<std::path::PathBuf> {
    // SAFETY: `URLs` is NSOpenPanel's, an array of `NSURL`; `count` and `objectAtIndex:` are
    // NSArray's, the index below the count.
    unsafe {
        let urls: Option<Retained<AnyObject>> = msg_send![panel, URLs];
        let Some(urls) = urls else {
            return Vec::new();
        };
        let count: usize = msg_send![&*urls, count];
        (0..count)
            .filter_map(|i| {
                let url: Option<Retained<objc2_foundation::NSURL>> =
                    msg_send![&*urls, objectAtIndex: i];
                url.and_then(|u| u.to_file_path())
            })
            .collect()
    }
}

// The content-type class the default-app call takes, `UTType`, is in this framework. AppKit loads
// it on the systems that have the call, and linking it says so rather than relying on that.
#[link(name = "UniformTypeIdentifiers", kind = "framework")]
extern "C" {}

/// The selector of the workspace's default-app call, which macOS 12 added.
fn set_default_selector() -> Sel {
    sel!(setDefaultApplicationAtURL:toOpenContentType:completionHandler:)
}

/// The app this process runs from and the shared workspace, when it can ask macOS to make the app
/// a default: it runs from an app bundle, and the system has the call.
fn default_app_parts() -> Result<(Retained<objc2_foundation::NSURL>, Retained<AnyObject>), String> {
    let not_the_app = || {
        "Brightfield is not running from its app, so macOS cannot make it the default. Open it \
         from Applications to do this."
            .to_owned()
    };
    let bundle = AnyClass::get(c"NSBundle").ok_or_else(not_the_app)?;
    // SAFETY: `mainBundle` is NSBundle's, returning the bundle or nil, and `bundleURL` is
    // NSBundle's, returning a file URL.
    let app: Option<Retained<objc2_foundation::NSURL>> = unsafe {
        let main: Option<Retained<AnyObject>> = msg_send![bundle, mainBundle];
        match main {
            Some(main) => msg_send![&*main, bundleURL],
            None => None,
        }
    };
    let app = app
        .filter(|url| {
            url.to_file_path()
                .is_some_and(|path| path.extension().is_some_and(|ext| ext == "app"))
        })
        .ok_or_else(not_the_app)?;
    let too_old = || "Making Brightfield the default app needs macOS 12 or later.".to_owned();
    let workspace = AnyClass::get(c"NSWorkspace").ok_or_else(too_old)?;
    // SAFETY: `sharedWorkspace` is NSWorkspace's, and `respondsToSelector:` NSObject's, taking a
    // selector and returning a BOOL.
    let workspace: Retained<AnyObject> = unsafe {
        let shared: Option<Retained<AnyObject>> = msg_send![workspace, sharedWorkspace];
        let shared = shared.ok_or_else(too_old)?;
        let can: bool = msg_send![&*shared, respondsToSelector: set_default_selector()];
        if !can || AnyClass::get(c"UTType").is_none() {
            return Err(too_old());
        }
        shared
    };
    Ok((app, workspace))
}

/// Whether this process can ask macOS to make it the default app for a content type: it runs from
/// an app bundle, on macOS 12 or later. `Err` says why not, in a sentence.
///
/// # Errors
///
/// Why it cannot ask.
pub fn default_app_available() -> Result<(), String> {
    default_app_parts().map(|_| ())
}

/// **Ask macOS to make this app the default for each of `types`**, a content type each, in turn:
/// each is asked once macOS has answered for the one before, so the system's confirmations come one
/// at a time. `answered` is called with each type's index in `types` and macOS's answer, `Ok` when
/// the app is now the default for it, else macOS's reason. It is called off the main thread, from
/// the workspace's completion handler.
///
/// # Errors
///
/// Why this process cannot ask, as [`default_app_available`] says it; nothing is asked then.
pub fn ask_default_app(
    types: &'static [&'static str],
    answered: impl Fn(usize, Result<(), String>) + Send + Sync + 'static,
) -> Result<(), String> {
    let (app, workspace) = default_app_parts()?;
    ask_default_from(app, workspace, types, 0, std::sync::Arc::new(answered));
    Ok(())
}

/// Ask for the content type at `at` in `types`, and from its answer the next.
fn ask_default_from(
    app: Retained<objc2_foundation::NSURL>,
    workspace: Retained<AnyObject>,
    types: &'static [&'static str],
    at: usize,
    answered: std::sync::Arc<dyn Fn(usize, Result<(), String>) + Send + Sync>,
) {
    let Some(identifier) = types.get(at) else {
        return;
    };
    let class = AnyClass::get(c"UTType");
    let identifier = NSString::from_str(identifier);
    // SAFETY: `typeWithIdentifier:` is UTType's, taking a string and returning the type or nil.
    let content_type: Option<Retained<AnyObject>> = match class {
        Some(class) => unsafe { msg_send![class, typeWithIdentifier: &*identifier] },
        None => None,
    };
    let Some(content_type) = content_type else {
        answered(
            at,
            Err(format!("macOS does not know the type {identifier}.")),
        );
        ask_default_from(app, workspace, types, at + 1, answered);
        return;
    };
    let (next_app, next_workspace) = (app.clone(), workspace.clone());
    let done = block2::RcBlock::new(move |error: *mut AnyObject| {
        // SAFETY: the completion handler is given an NSError, or nil when the change was made.
        let answer = match unsafe { error.as_ref() } {
            None => Ok(()),
            Some(error) => Err(error_text(error)),
        };
        answered(at, answer);
        ask_default_from(
            next_app.clone(),
            next_workspace.clone(),
            types,
            at + 1,
            answered.clone(),
        );
    });
    // SAFETY: the call is NSWorkspace's, checked present by `default_app_parts`, taking the app's
    // file URL, a UTType and a block taking an NSError or nil; the workspace copies the block.
    unsafe {
        let () = msg_send![
            &*workspace,
            setDefaultApplicationAtURL: &*app,
            toOpenContentType: &*content_type,
            completionHandler: &*done
        ];
    }
}

/// An NSError's description, in the system's words.
fn error_text(error: &AnyObject) -> String {
    // SAFETY: `localizedDescription` is NSError's, returning a string.
    let text: Option<Retained<NSString>> = unsafe { msg_send![error, localizedDescription] };
    text.map_or_else(|| "macOS gave no reason.".to_owned(), |t| t.to_string())
}

/// The handler `install` would register, kept by the caller, so a test can send it an event.
#[cfg(test)]
fn handler_for_test(
    queue: OpenQueue,
    wake: impl Fn() + Send + Sync + 'static,
) -> Retained<OpenHandler> {
    OpenHandler::new(queue, Box::new(wake))
}

/// An `AEDesc`, the record an `NSAppleEventDescriptor` wraps, as its encoding names it.
#[cfg(test)]
#[repr(C)]
struct AEDesc {
    descriptor_type: u32,
    data_handle: *mut std::ffi::c_void,
}

#[cfg(test)]
// SAFETY: the encoding is the one the runtime records for `AEDesc`: a `DescType` and an
// `AEDataStorage`, which is a pointer to a pointer to an opaque struct.
unsafe impl objc2::encode::RefEncode for AEDesc {
    const ENCODING_REF: objc2::encode::Encoding =
        objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct(
            "AEDesc",
            &[
                objc2::encode::Encoding::UInt,
                objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Pointer(
                    &objc2::encode::Encoding::Struct("OpaqueAEDataStorageType", &[]),
                )),
            ],
        ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSURL;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// An Apple Event of `class` and `id` whose direct object is `direct`.
    fn event(
        class: u32,
        id: u32,
        direct: &NSAppleEventDescriptor,
    ) -> Retained<NSAppleEventDescriptor> {
        let target = NSAppleEventDescriptor::nullDescriptor();
        // SAFETY: the selector takes an event class and id, a target descriptor, a return id and a
        // transaction id, with the types given; -1 and 0 are `kAutoGenerateReturnID` and
        // `kAnyTransactionID`.
        let event: Retained<NSAppleEventDescriptor> = unsafe {
            msg_send![
                NSAppleEventDescriptor::class(),
                appleEventWithEventClass: class,
                eventID: id,
                targetDescriptor: &*target,
                returnID: -1i16,
                transactionID: 0i32
            ]
        };
        // SAFETY: `setParamDescriptor:forKeyword:` takes a descriptor and an `AEKeyword`.
        let () =
            unsafe { msg_send![&*event, setParamDescriptor: direct, forKeyword: DIRECT_OBJECT] };
        event
    }

    /// An open-documents event naming `paths`, as Finder sends one: a list of file URLs.
    fn open_documents(paths: &[PathBuf]) -> Retained<NSAppleEventDescriptor> {
        let list = NSAppleEventDescriptor::listDescriptor();
        for path in paths {
            let url = NSURL::from_file_path(path).expect("a file URL");
            // Index 0 appends.
            list.insertDescriptor_atIndex(&NSAppleEventDescriptor::descriptorWithFileURL(&url), 0);
        }
        event(CORE_EVENT_CLASS, OPEN_DOCUMENTS, &list)
    }

    fn counter() -> (Arc<AtomicUsize>, impl Fn() + Send + Sync + 'static) {
        let woken = Arc::new(AtomicUsize::new(0));
        let w = woken.clone();
        (woken, move || {
            w.fetch_add(1, Ordering::SeqCst);
        })
    }

    use objc2::ClassType as _;

    #[test]
    fn an_open_documents_event_queues_its_files_together_and_wakes_the_window() {
        let queue = OpenQueue::default();
        let (woken, wake) = counter();
        let handler = handler_for_test(queue.clone(), wake);
        let paths = [
            PathBuf::from("/tmp/Données brutes/houses 2024.csv"),
            PathBuf::from("/tmp/shop"),
        ];
        // A space survives the trip through a file URL. An accent does too, in the decomposed form
        // a file URL's path takes, which the Mac's file systems read as the same name.
        let delivered = [
            PathBuf::from("/tmp/Donne\u{301}es brutes/houses 2024.csv"),
            PathBuf::from("/tmp/shop"),
        ];
        let event = open_documents(&paths);
        let reply = NSAppleEventDescriptor::nullDescriptor();
        let () =
            unsafe { msg_send![&*handler, handleOpenDocuments: &*event, withReplyEvent: &*reply] };
        assert_eq!(
            queue.drain(),
            vec![OpenRequest::Paths(delivered.to_vec())],
            "the files of one event are one request"
        );
        assert_eq!(woken.load(Ordering::SeqCst), 1);
    }

    /// AppKit's panels belong to the main thread, and a test's thread is not it: the ask is
    /// declined there rather than raising a panel AppKit would refuse, and it leaves no panel
    /// counted as up.
    #[test]
    fn the_open_panel_is_not_raised_off_the_main_thread() {
        assert!(!begin_open_panel("Open", &["csv"], |_| {}));
        assert!(!PANEL_UP.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn finishing_launching_marks_the_queue_launched() {
        let queue = OpenQueue::awaiting_launch();
        let (woken, wake) = counter();
        let handler = handler_for_test(queue.clone(), wake);

        handler.launch_moment(LAUNCH_NOTIFICATIONS[0]);
        assert!(
            !queue.has_launched(),
            "will-finish leaves the queue waiting"
        );

        handler.launch_moment(DID_FINISH_LAUNCHING);
        assert!(queue.has_launched(), "did-finish marks it launched");
        assert_eq!(woken.load(Ordering::SeqCst), 1, "and wakes the window");
    }

    #[test]
    fn a_get_url_event_queues_the_link() {
        let queue = OpenQueue::default();
        let (woken, wake) = counter();
        let handler = handler_for_test(queue.clone(), wake);
        let link = "brightfield://open?url=https://meridian.online/p.zip";
        let event = event(
            GET_URL,
            GET_URL,
            &NSAppleEventDescriptor::descriptorWithString(&NSString::from_str(link)),
        );
        let reply = NSAppleEventDescriptor::nullDescriptor();
        let () = unsafe { msg_send![&*handler, handleGetURL: &*event, withReplyEvent: &*reply] };
        assert_eq!(queue.drain(), vec![OpenRequest::Link(link.to_string())]);
        assert_eq!(woken.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_quit_event_asks_the_window_to_quit() {
        let queue = OpenQueue::default();
        let (woken, wake) = counter();
        let handler = handler_for_test(queue.clone(), wake);
        let event = event(
            CORE_EVENT_CLASS,
            QUIT_APPLICATION,
            &NSAppleEventDescriptor::nullDescriptor(),
        );
        let reply = NSAppleEventDescriptor::nullDescriptor();
        let () = unsafe { msg_send![&*handler, handleQuit: &*event, withReplyEvent: &*reply] };
        assert!(queue.take_quit(), "the quit reached the queue");
        assert!(queue.drain().is_empty(), "a quit opens nothing");
        assert_eq!(woken.load(Ordering::SeqCst), 1);
    }

    /// The installed handlers are the ones the event manager calls: an event dispatched through
    /// it lands on the queue `install` was given. This is the manager's own routing, short of
    /// Launch Services sending the event from Finder. The handlers it registers are the whole
    /// process's, so this is the one test that registers any; the others send their events to a
    /// handler of their own.
    #[test]
    fn the_event_manager_routes_open_events_to_the_installed_queue() {
        let queue = OpenQueue::default();
        let (woken, wake) = counter();
        register_for_life(queue.clone(), wake);
        let manager = NSAppleEventManager::sharedAppleEventManager();
        let path = PathBuf::from("/tmp/dispatched.csv");
        let link = "brightfield://open?url=https://meridian.online/q.zip";
        let events = [
            open_documents(std::slice::from_ref(&path)),
            event(
                GET_URL,
                GET_URL,
                &NSAppleEventDescriptor::descriptorWithString(&NSString::from_str(link)),
            ),
        ];
        for event in &events {
            let reply = NSAppleEventDescriptor::nullDescriptor();
            // SAFETY: both descriptors are live for the call, and `aeDesc` points into them.
            let err: i16 = unsafe {
                let raw: *const AEDesc = msg_send![&**event, aeDesc];
                let raw_reply: *const AEDesc = msg_send![&*reply, aeDesc];
                msg_send![
                    &*manager,
                    dispatchRawAppleEvent: raw,
                    withRawReply: raw_reply.cast_mut(),
                    handlerRefCon: std::ptr::null_mut::<std::ffi::c_void>()
                ]
            };
            assert_eq!(err, 0, "the manager dispatched the event");
        }
        assert_eq!(
            queue.drain(),
            vec![OpenRequest::Path(path), OpenRequest::Link(link.to_string())]
        );
        assert_eq!(woken.load(Ordering::SeqCst), 2);
    }
}
