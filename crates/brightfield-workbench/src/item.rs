//! What occupies a pane.
//!
//! # The aliasing decision, which is the load-bearing one
//!
//! The obvious shape for an item is "a thing that holds the model it draws".
//! It does not work here and it is worth saying exactly why, because the
//! failure is not a matter of taste.
//!
//! Four panes in the protocol view all read one interaction model, and two of
//! them mutate it — clicking a node in the outline selects it, and so does
//! clicking one on the canvas. If each pane held a handle to that model, the
//! four handles would have to be four `&mut`, which is unique, or four `&`,
//! which cannot select. Wrapping the model in `Rc<RefCell<_>>` compiles and
//! converts the problem into a runtime panic the first time the shell asks a
//! pane for its [`Subject`] while another pane holds a `borrow_mut()` — and
//! the shell asking any pane for its subject at any time is precisely the
//! property this whole crate is built on.
//!
//! So: **an [`Item`] holds no document handle at all.** It holds only its own
//! view-local state — scroll memory, a pending edit, a cached raster key. The
//! state its pane reads and writes belongs to the view, and the shell hands
//! it in on every call. At any instant exactly one `&mut D` exists, created
//! for the duration of one [`Item::ui`] call and dropped at its end.
//!
//! That has a second, larger payoff. Because an item needs nothing to be
//! constructed, its constructor is a plain `fn() -> Box<dyn Item<D>>`, which
//! is what lets [`crate::ItemSpec`] be a `const`-able record and lets the
//! registry, the default layout and the empty-state gate all read from one
//! list instead of three that drift.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::subject::{Subject, Verb};
use crate::workspace::ViewKind;
use crate::Mode;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// A stable name for a kind of pane.
///
/// Serialised into the layout file, so the string is a compatibility surface
/// in a way the Rust type name is not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(&'static str);

static KNOWN: OnceLock<&'static [ItemId]> = OnceLock::new();

impl ItemId {
    /// An item id.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The id's string form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }

    /// Publish this process's item vocabulary.
    ///
    /// Called once at boot from the registries, before any layout file is
    /// read, so that deserialising an id can check it against something. This
    /// is a process global and that is a deliberate trade, not an oversight:
    /// `egui_tiles::Tree<PaneKey>`'s derived `Deserialize` offers nowhere to
    /// thread a context, and `DeserializeSeed` does not reach a nested
    /// generic parameter. If it ever bites — two workspaces in one process,
    /// say — the escape is to make the id owned and validate after load by
    /// walking the tiles, which costs `Copy` on [`PaneKey`] and nothing else.
    ///
    /// Returns whether this call was the one that published; a second call is
    /// a no-op rather than a panic, so a test binary that boots twice does
    /// not fall over.
    pub fn publish(ids: &'static [ItemId]) -> bool {
        KNOWN.set(ids).is_ok()
    }

    /// The published vocabulary, empty until [`ItemId::publish`] is called.
    #[must_use]
    pub fn known() -> &'static [ItemId] {
        KNOWN.get().copied().unwrap_or(&[])
    }
}

impl std::fmt::Display for ItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl Serialize for ItemId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for ItemId {
    /// A saved layout that names an item this build does not have is a **load
    /// failure**, not a pane to be materialised and then failed to draw.
    ///
    /// Failing here rather than later means the whole file is discarded and
    /// the default arrangement is used — a user who upgrades across a renamed
    /// pane loses their layout, which is annoying, instead of getting a
    /// window with a hole in it, which is broken.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = std::borrow::Cow::<'_, str>::deserialize(d)?;
        ItemId::known()
            .iter()
            .copied()
            .find(|i| i.0 == s)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown item id {s:?}")))
    }
}

/// A pane's address: which view it belongs to, and which item fills it.
///
/// `Copy`, small, and free of anything a document could hide inside — see the
/// size tripwire in the contract tests. A pane key that grew a model handle
/// would put the document back inside the pane identity, which is the shape
/// this crate exists to prevent.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize, //
)]
pub struct PaneKey {
    /// The view this pane belongs to.
    pub view: ViewKind,
    /// Which item fills it.
    pub item: ItemId,
}

impl PaneKey {
    /// A pane key.
    #[must_use]
    pub const fn new(view: ViewKind, item: ItemId) -> Self {
        Self { view, item }
    }
}

impl std::fmt::Display for PaneKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}/{}", self.view, self.item)
    }
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Something a pane asks the workspace to do after this frame's panes have
/// drawn.
///
/// Deferred rather than immediate because a pane is drawing inside a borrow
/// of the tile tree when it asks, and "run this verb now" would mean
/// re-entering the tree from inside itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Run a command.
    Verb(Verb),
    /// Move focus to a pane.
    Focus(PaneKey),
    /// Schedule another frame.
    Repaint,
}

/// Whether a dispatched verb was consumed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Handled {
    /// Consumed here. Stop.
    Yes,
    /// Not ours. Bubble to the workspace.
    #[default]
    No,
}

impl Handled {
    /// Whether the verb was consumed.
    #[must_use]
    pub const fn is_handled(self) -> bool {
        matches!(self, Handled::Yes)
    }
}

/// What a pane is told about the frame it is drawing in, and how it talks
/// back.
///
/// Carries **no** GPU handle and no scene type. That is what keeps this crate
/// free of wgpu: the canvas host is owned by the *document*, which is a
/// shell-side type, so a canvas pane reaches its renderer through `&mut D`
/// like any other state rather than through a `dyn Any` in the context.
pub struct ItemCtx<'a> {
    /// Light or dark.
    pub mode: Mode,
    /// This pane's address.
    pub key: PaneKey,
    /// This pane's tile, for deriving stable widget ids.
    pub tile: egui_tiles::TileId,
    /// Whether this pane holds focus this frame.
    ///
    /// Items never paint the focus ring themselves — the shell does — but a
    /// list may legitimately want to show its cursor row differently when the
    /// pane is not focused.
    pub focused: bool,
    requests: &'a mut Vec<Request>,
}

impl<'a> ItemCtx<'a> {
    /// Build a context for one pane's draw.
    ///
    /// Public because the shell that hosts the panes is the caller, and it
    /// may live outside this crate. The `requests` queue behind it stays
    /// private: an item can push onto it through [`ItemCtx::request`] and
    /// friends, and can neither read nor drain it — a pane that could cancel
    /// another pane's request would be reaching outside its own borrow.
    pub fn new(
        mode: Mode,
        key: PaneKey,
        tile: egui_tiles::TileId,
        focused: bool,
        requests: &'a mut Vec<Request>,
    ) -> Self {
        Self {
            mode,
            key,
            tile,
            focused,
            requests,
        }
    }
}

impl ItemCtx<'_> {
    /// Ask the workspace to run `verb` once this frame's panes have drawn.
    pub fn request(&mut self, verb: Verb) {
        self.requests.push(Request::Verb(verb));
    }

    /// Ask for focus.
    pub fn take_focus(&mut self) {
        self.requests.push(Request::Focus(self.key));
    }

    /// Ask for another frame.
    pub fn request_repaint(&mut self) {
        self.requests.push(Request::Repaint);
    }
}

// ---------------------------------------------------------------------------
// Item
// ---------------------------------------------------------------------------

/// Something that occupies a pane, reads one document, and declares its own
/// chrome.
///
/// `D` is the view's **document**: the state every pane in that view shares.
/// An item never holds a handle to it — see the module docs for why that is
/// the decision the rest of the design hangs off.
pub trait Item<D: ?Sized> {
    /// This item's stable id. Must match the [`crate::ItemSpec`] that
    /// constructs it; the registry asserts as much.
    fn item_id(&self) -> ItemId;

    /// What the shell should draw around this pane, this frame.
    ///
    /// `&self` and `&D` on purpose: building a subject cannot mutate
    /// anything, so it can never quietly become a second update path, and the
    /// shell can ask any pane for its subject at any time — including a pane
    /// that is not being drawn, which is what a tab title needs.
    fn subject(&self, doc: &D) -> Subject;

    /// Draw the body.
    ///
    /// `ui` is a child `Ui` whose `max_rect` is the content rect the shell
    /// reserved *below* the header band. That is the enforcement of "panes do
    /// not draw their own headers", and it is structural rather than a
    /// convention: an item is handed no `Ui` that covers the header, so it
    /// cannot draw one there even if it tries.
    ///
    /// Not called at all when `subject(doc).empty_state` is `Some`.
    fn ui(&mut self, doc: &mut D, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>);

    /// Perform a verb dispatched to this item.
    ///
    /// [`Handled::No`] — the default — bubbles it to the workspace.
    fn perform(&mut self, _doc: &mut D, _verb: Verb, _cx: &mut ItemCtx<'_>) -> Handled {
        Handled::No
    }
}

/// The live items of one view, addressed by pane.
///
/// A `BTreeMap` rather than a `HashMap` so iteration order is the pane key's
/// order and therefore stable — a test that walks every pane should not
/// depend on hash seeding.
pub type ItemMap<D> = BTreeMap<PaneKey, Box<dyn Item<D>>>;
