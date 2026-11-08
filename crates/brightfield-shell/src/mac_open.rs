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
//! The Apple Event methods that take a four-character code are sent by selector rather than
//! through the `objc2-foundation` methods that declare them, which need a feature whose crate the
//! dependency graph does not otherwise hold.

use objc2::rc::Retained;
use objc2::runtime::{NSObject, Sel};
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass};
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
/// `kInternetEventClass` and `kAEGetURL`, which share their code.
const GET_URL: u32 = code(b"GURL");
/// `keyDirectObject`: the parameter an event's subject is in.
const DIRECT_OBJECT: u32 = code(b"----");
/// `typeAEList`.
const LIST: u32 = code(b"list");

/// The two moments AppKit may have replaced the handlers by, named as AppKit posts them.
const LAUNCH_NOTIFICATIONS: [&str; 2] = [
    "NSApplicationWillFinishLaunchingNotification",
    "NSApplicationDidFinishLaunchingNotification",
];

struct Ivars {
    queue: OpenQueue,
    wake: Box<dyn Fn() + Send + Sync>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `OpenHandler` does not implement Drop.
    #[unsafe(super(NSObject))]
    #[ivars = Ivars]
    struct OpenHandler;

    impl OpenHandler {
        #[unsafe(method(handleOpenDocuments:withReplyEvent:))]
        fn handle_open_documents(
            &self,
            event: &NSAppleEventDescriptor,
            _reply: &NSAppleEventDescriptor,
        ) {
            for path in documents(event) {
                self.ivars().queue.push(OpenRequest::Path(path));
            }
            (self.ivars().wake)();
        }

        #[unsafe(method(handleGetURL:withReplyEvent:))]
        fn handle_get_url(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            if let Some(link) = direct_object(event).and_then(|d| d.stringValue()) {
                self.ivars().queue.push(OpenRequest::Link(link.to_string()));
            }
            (self.ivars().wake)();
        }

        #[unsafe(method(appFinishingLaunch:))]
        fn app_finishing_launch(&self, _note: &NSNotification) {
            self.register();
        }
    }
);

impl OpenHandler {
    fn new(queue: OpenQueue, wake: Box<dyn Fn() + Send + Sync>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { queue, wake });
        // SAFETY: NSObject's `init` is its designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    /// Make this object the handler for both events, replacing whichever was registered.
    fn register(&self) {
        let manager = NSAppleEventManager::sharedAppleEventManager();
        let handlers: [(Sel, u32, u32); 2] = [
            (
                sel!(handleOpenDocuments:withReplyEvent:),
                CORE_EVENT_CLASS,
                OPEN_DOCUMENTS,
            ),
            (sel!(handleGetURL:withReplyEvent:), GET_URL, GET_URL),
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
/// then calling `wake` so a frame runs. Call it on the main thread, before the event loop starts.
pub fn install(queue: OpenQueue, wake: impl Fn() + Send + Sync + 'static) {
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
    fn an_open_documents_event_queues_each_file_and_wakes_the_window() {
        let queue = OpenQueue::default();
        let (woken, wake) = counter();
        let handler = handler_for_test(queue.clone(), wake);
        let paths = [
            // A space survives the trip through a file URL. An accent does too, in the decomposed
            // form a file URL's path takes, which the file system reads as the same name.
            PathBuf::from("/tmp/raw data/houses 2024.csv"),
            PathBuf::from("/tmp/shop"),
        ];
        let event = open_documents(&paths);
        let reply = NSAppleEventDescriptor::nullDescriptor();
        let () =
            unsafe { msg_send![&*handler, handleOpenDocuments: &*event, withReplyEvent: &*reply] };
        assert_eq!(
            queue.drain(),
            paths
                .iter()
                .cloned()
                .map(OpenRequest::Path)
                .collect::<Vec<_>>()
        );
        assert_eq!(woken.load(Ordering::SeqCst), 1);
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

    /// The installed handlers are the ones the event manager calls: an event dispatched through
    /// it lands on the queue `install` was given. This is the manager's own routing, short of
    /// Launch Services sending the event from Finder.
    #[test]
    fn the_event_manager_routes_open_events_to_the_installed_queue() {
        let queue = OpenQueue::default();
        let (woken, wake) = counter();
        install(queue.clone(), wake);
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
