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
use std::sync::{PoisonError, RwLock};

use brightfield_keys::BindingContext;
use serde::{Deserialize, Serialize};

use crate::registry::{ChartKind, ChartKindId, ChartKindRegistry, Field, ModuleOptions};
use crate::subject::{EmptyState, Icon, Subject, Verb};
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

/// The process's item vocabulary, accumulated across every registry that
/// publishes into it.
///
/// A `RwLock` rather than a `OnceLock` because there is one registry *per
/// view* and [`ViewKind::ALL`] has more than one, so "publish" is inherently
/// plural. The first draft of this was a `OnceLock` whose `set` result was
/// dropped, which meant the second view's ids were discarded in silence and
/// its saved layout failed to load forever — the exact failure the custom
/// [`Deserialize`] was written to make loud.
static KNOWN: RwLock<&'static [ItemId]> = RwLock::new(&[]);

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

    /// Add a registry's ids to this process's item vocabulary.
    ///
    /// Called at boot **once per view registry** — [`ViewKind::ALL`] has more
    /// than one — before any layout file is read, so that deserialising an id
    /// can check it against something. Calls accumulate: every id ever
    /// published stays published, and re-publishing an id already present is
    /// a no-op, so a test binary that boots twice neither falls over nor
    /// grows.
    ///
    /// This is a process global and that is a deliberate trade, not an
    /// oversight: `egui_tiles::Tree<PaneKey>`'s derived `Deserialize` offers
    /// nowhere to thread a context, and `DeserializeSeed` does not reach a
    /// nested generic parameter. If it ever bites — two workspaces in one
    /// process wanting *different* vocabularies, say — the escape is to make
    /// the id owned and validate after load by walking the tiles, which costs
    /// `Copy` on [`PaneKey`] and nothing else.
    ///
    /// # Why this leaks
    ///
    /// Merging two `&'static [ItemId]` needs a new allocation that outlives
    /// both, so a merge leaks one `Vec`. That is bounded by the number of
    /// `publish` calls — one per view, at boot — and buys [`ItemId::known`]
    /// its `&'static [ItemId]` return, which is what lets `Deserialize` look
    /// an id up without allocating on every pane in a layout file. Publishing
    /// the first registry does not allocate at all.
    pub fn publish(ids: &'static [ItemId]) {
        let mut known = KNOWN.write().unwrap_or_else(PoisonError::into_inner);
        if known.is_empty() {
            *known = ids;
            return;
        }
        if ids.iter().all(|id| known.contains(id)) {
            return;
        }
        let mut merged: Vec<ItemId> = known.to_vec();
        for id in ids {
            if !merged.contains(id) {
                merged.push(*id);
            }
        }
        *known = Vec::leak(merged);
    }

    /// The published vocabulary, empty until [`ItemId::publish`] is called.
    #[must_use]
    pub fn known() -> &'static [ItemId] {
        *KNOWN.read().unwrap_or_else(PoisonError::into_inner)
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
    /// Open the shipped starting point with this id — see
    /// [`Action::Open`](crate::subject::Action::Open). The id is opaque here;
    /// a shell that does not recognise it does nothing.
    Open(&'static str),
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

    /// What this pane shows when it has nothing to show — and whether that is
    /// now. `Some` means the pane is empty this frame: the shell draws the
    /// returned state *instead of* calling [`Item::ui`]. `None` means the
    /// pane has content.
    ///
    /// **Required, with no default, and that is the whole design.** The two
    /// worst empty states in the shell this crate replaces were the two
    /// nobody had written — an outline and a steps sheet that rendered a
    /// header and silence. An optional field can be forgotten; a required
    /// method makes "what does this pane show when it is empty?" a question
    /// every pane author answers before their pane compiles. The audit
    /// ([`crate::registry::audit`]) still checks the *answer*: over an empty
    /// document this must be `Some`, with prose to the house style.
    ///
    /// `&self` and `&D` for the reason [`Item::describe`] takes them:
    /// answering cannot mutate anything.
    fn empty_state(&self, doc: &D) -> Option<EmptyState>;

    /// What the shell should draw around this pane, this frame — everything
    /// **except** the empty state, which is [`Item::empty_state`]'s to
    /// answer. Implementations must not set [`Subject::empty_state`] here;
    /// [`Item::subject`] composes the two and asserts as much in debug.
    ///
    /// `&self` and `&D` on purpose: building a subject cannot mutate
    /// anything, so it can never quietly become a second update path, and the
    /// shell can ask any pane for its subject at any time — including a pane
    /// that is not being drawn, which is what a tab title needs.
    fn describe(&self, doc: &D) -> Subject;

    /// The full subject: [`Item::describe`]'s chrome with
    /// [`Item::empty_state`]'s answer folded in. This is what the shell and
    /// the audit read.
    ///
    /// Provided, and **not to be overridden**: the composition is what makes
    /// the two channels unable to disagree — an override would reopen the gap
    /// between "the pane says it is empty" and "the shell draws it empty",
    /// and the audit rejects an item whose `subject` and `empty_state`
    /// answers diverge.
    fn subject(&self, doc: &D) -> Subject {
        let mut subject = self.describe(doc);
        debug_assert!(
            subject.empty_state.is_none(),
            "{}: declare emptiness by implementing Item::empty_state, not by \
             setting Subject::empty_state in describe()",
            self.item_id()
        );
        subject.empty_state = self.empty_state(doc);
        subject
    }

    /// Draw the body.
    ///
    /// `ui` is a child `Ui` whose `max_rect` *and clip rect* are both the
    /// content rect the shell reserved **below** the header band, so drawing
    /// the ordinary way cannot put a header where the shell's goes.
    ///
    /// That is a default, not a capability. `Ui::set_clip_rect` widens the
    /// clip on this very `Ui`, and `egui::Area` / `egui::Window` /
    /// `ctx.layer_painter` take a fresh layer from the `Context`. egui gives
    /// no way to withhold any of those from a `&mut Ui` holder, so against a
    /// pane that sets out to bypass the contract the backstop is review, not
    /// the type system. See [`crate::chrome::pane_frame`].
    ///
    /// Not called at all when [`Item::empty_state`] answers `Some` — the
    /// shell draws that instead.
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

// ---------------------------------------------------------------------------
// Modules: one component, many kinds
// ---------------------------------------------------------------------------

/// A view document that can draw the charts its own registry declares.
///
/// The seam between "a chart kind is data" and "something has to rasterise
/// it". [`ChartKind`] builds a spec and stops; the spec goes back to the
/// document, which owns the renderer, the GPU handle and everything else this
/// crate has none of. That is the same division [`ItemCtx`] already makes, and
/// it is why a chart-kind registry can live in a crate with no wgpu in it.
pub trait ModuleHost {
    /// The spec type this view's chart kinds build.
    ///
    /// Named once, by the view, and never by this crate — see [`ChartKind`]
    /// for why the registry is generic over it.
    type Spec;

    /// This view's chart vocabulary.
    fn chart_kinds(&self) -> &ChartKindRegistry<Self::Spec>;

    /// Draw one module's spec into `ui`.
    ///
    /// `ui` is the module's *body*: the rect left after the module's own
    /// declared chrome has taken what it needs, which is nothing at all when
    /// the kind declares no controls.
    fn draw_module(&mut self, spec: &Self::Spec, ui: &mut egui::Ui);
}

/// One chart module: a registered [`ChartKind`], the columns bound to it, and
/// the state of the controls that kind hangs on itself.
///
/// **This is the only component a chart kind gets, and adding a kind does not
/// add another.** A kind is a value in [`ChartKindRegistry`]; the pane that
/// draws it is this type, unchanged. What varies between two kinds is their
/// data — icon, description, slots, controls, builder — and nothing else.
///
/// It is an [`Item`] like any other, so it inherits the whole pane contract:
/// the shell draws its header from [`Item::describe`], draws its empty state
/// *instead of* its body when the columns do not fit the kind, and clips its
/// draw below the header band.
///
/// # What it deliberately does not put in its [`Subject`]
///
/// Its kind's [`ModuleControl`](crate::registry::ModuleControl)s. A
/// `Subject::toolbar` entry is drawn by the surface *around* the pane, and a
/// module's own controls are the case that must not be: a log/linear toggle
/// belongs to the module it rescales, not to the canvas the module sits on. So
/// they are drawn inside the module's own rect by
/// [`module_frame`](crate::chrome::module_frame), performed here — [`perform`]
/// answers [`Handled::Yes`] for them, so the verb never reaches the workspace
/// — and held to the registered-verb rule by
/// [`audit_chart_kinds`](crate::registry::audit_chart_kinds) rather than by
/// [`audit`](crate::registry::audit), which only ever sees a subject.
///
/// [`perform`]: Item::perform
#[derive(Clone, Debug)]
pub struct ChartModule {
    item: ItemId,
    kind: ChartKindId,
    icon: Icon,
    title: String,
    key_context: BindingContext,
    fields: Vec<Field>,
    options: ModuleOptions,
}

impl ChartModule {
    /// A module of `kind`, over `fields`.
    ///
    /// Takes the kind by reference rather than by id so the module starts with
    /// that kind's declared control states rather than with everything off —
    /// a control whose declared default is *on* would otherwise be off for one
    /// frame, or forever if nobody noticed.
    ///
    /// The icon is copied from the kind here, and the copy is the point: a
    /// module whose kind this build no longer has still has an icon to draw,
    /// while the kind stays the only place an icon is *declared*.
    #[must_use]
    pub fn new<S>(
        item: ItemId,
        title: impl Into<String>,
        kind: &ChartKind<S>,
        fields: Vec<Field>,
    ) -> Self {
        Self {
            item,
            kind: kind.id,
            icon: kind.icon,
            title: title.into(),
            key_context: BindingContext::Workspace,
            fields,
            options: kind.options(),
        }
    }

    /// Resolve bindings in a different keyboard context.
    #[must_use]
    pub const fn in_context(mut self, context: BindingContext) -> Self {
        self.key_context = context;
        self
    }

    /// Which kind this module draws.
    #[must_use]
    pub const fn kind(&self) -> ChartKindId {
        self.kind
    }

    /// The columns this module was handed.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Hand this module a different set of columns.
    pub fn set_fields(&mut self, fields: Vec<Field>) {
        self.fields = fields;
    }

    /// The state of this module's own controls.
    #[must_use]
    pub const fn options(&self) -> &ModuleOptions {
        &self.options
    }

    /// The spec this module would hand its host right now, or `None` when its
    /// kind is not in this build or its columns do not fill that kind's
    /// required slots.
    ///
    /// The same value [`Item::ui`] draws, reachable without a frame — which is
    /// what lets a migration be checked by comparing specs rather than by
    /// comparing screenshots.
    pub fn spec<D: ModuleHost + ?Sized>(&self, doc: &D) -> Option<D::Spec> {
        let kind = doc.chart_kinds().find(self.kind)?;
        let binding = kind.bind(&self.fields).ok()?;
        kind.spec(&binding, &self.options).ok()
    }

    /// Flip the control this verb belongs to, and report whether one did.
    fn apply<D: ModuleHost + ?Sized>(&mut self, doc: &D, verb: Verb) -> bool {
        let Some(kind) = doc.chart_kinds().find(self.kind) else {
            return false;
        };
        let Some(id) = (kind.controls)()
            .iter()
            .find(|c| c.verb == verb)
            .map(|c| c.id)
        else {
            return false;
        };
        self.options.toggle(id);
        true
    }
}

impl<D: ModuleHost + ?Sized> Item<D> for ChartModule {
    fn item_id(&self) -> ItemId {
        self.item
    }

    /// Empty when there is no chart to draw: the kind is not in this build, or
    /// the columns do not fill its required slots.
    ///
    /// The second case is the ordinary one and the message says which slot and
    /// what it wanted, because "this module is empty" is not an answer anyone
    /// can act on — [`ChartKind::bind`]'s error is.
    fn empty_state(&self, doc: &D) -> Option<EmptyState> {
        let Some(kind) = doc.chart_kinds().find(self.kind) else {
            return Some(EmptyState::new(
                self.icon,
                "This build has no chart of that kind",
                format!(
                    "The module asks for {}, which is not in this build's chart registry.",
                    self.kind
                ),
            ));
        };
        match kind.bind(&self.fields) {
            Ok(_) => None,
            Err(why) => Some(EmptyState::new(
                self.icon,
                "Nothing here to chart yet",
                format!("{why}. Bind a column that fits and it draws."),
            )),
        }
    }

    fn describe(&self, _doc: &D) -> Subject {
        Subject::new(self.title.clone(), self.icon, self.key_context)
    }

    /// The module's own chrome, then the module.
    ///
    /// Reached only when [`Item::empty_state`] answered `None`, so the kind is
    /// present and the binding holds; the early returns below are the release
    /// behaviour for a host that answers differently on two calls in one frame
    /// — a blank body rather than a panic.
    fn ui(&mut self, doc: &mut D, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        // Scoped so the borrow of `*doc` this takes ends before `draw_module`
        // needs `&mut D`. Everything carried out is owned.
        let (entries, spec) = {
            let Some(kind) = doc.chart_kinds().find(self.kind) else {
                return;
            };
            let Ok(binding) = kind.bind(&self.fields) else {
                return;
            };
            let Ok(spec) = kind.spec(&binding, &self.options) else {
                return;
            };
            (kind.toolbar(&self.options), spec)
        };

        let (mut body, drawn) = crate::chrome::module_frame(ui, &entries, cx.mode);
        let mut flipped = false;
        for verb in drawn.activated {
            flipped |= self.apply(&*doc, verb);
        }
        if flipped {
            // The spec above was built from the options as they were when the
            // row drew. The flip lands on the next frame, which is asked for
            // here rather than left to whatever else happens to repaint.
            cx.request_repaint();
        }
        doc.draw_module(&spec, &mut body);
    }

    /// A verb one of this module's own controls declares is **this module's**,
    /// however it arrived — the control's click and its keystroke are the same
    /// command, so a keyboard user and a pointer user get the same effect.
    fn perform(&mut self, doc: &mut D, verb: Verb, cx: &mut ItemCtx<'_>) -> Handled {
        if self.apply(&*doc, verb) {
            cx.request_repaint();
            return Handled::Yes;
        }
        Handled::No
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const CHARTS_CANVAS: ItemId = ItemId::new("test-charts-canvas");
    const PROTOCOL_OUTLINE: ItemId = ItemId::new("test-protocol-outline");

    /// Deliberately **one** test covering the whole publish lifecycle rather
    /// than the four it wants to be. The vocabulary is a process global (see
    /// [`ItemId::publish`] for why), so separate tests in this binary would
    /// share it and pass or fail depending on which ran first. Ordering-
    /// dependent tests are a way of learning nothing slowly, so the ordering
    /// is made explicit here instead.
    #[test]
    fn publishing_each_view_in_turn_keeps_every_view_loadable() {
        // Nothing published: every id is unknown, which is the safe
        // direction — a layout read before boot finishes is discarded rather
        // than trusted.
        assert!(ItemId::known().is_empty());

        ItemId::publish(&[CHARTS_CANVAS]);
        assert_eq!(ItemId::known(), &[CHARTS_CANVAS]);

        // The second view. This is the case the first draft lost in silence:
        // a `OnceLock::set` whose result was dropped kept Charts and threw
        // Protocol away, so the protocol view's saved layout could never load
        // again.
        ItemId::publish(&[PROTOCOL_OUTLINE]);
        assert!(
            ItemId::known().contains(&CHARTS_CANVAS),
            "publishing the second view discarded the first"
        );
        assert!(
            ItemId::known().contains(&PROTOCOL_OUTLINE),
            "publishing the second view was silently dropped"
        );

        // Both views' pane keys survive a round trip, which is the thing the
        // vocabulary exists to make possible.
        for (view, item) in [
            (ViewKind::Charts, CHARTS_CANVAS),
            (ViewKind::Protocol, PROTOCOL_OUTLINE),
        ] {
            let key = PaneKey::new(view, item);
            let json = serde_json::to_string(&key).expect("a pane key serialises");
            assert_eq!(
                serde_json::from_str::<PaneKey>(&json).expect("and round trips"),
                key
            );
        }

        // Re-publishing is idempotent rather than a panic or a duplicate, so
        // a binary that boots twice neither falls over nor grows.
        let before = ItemId::known().len();
        ItemId::publish(&[CHARTS_CANVAS]);
        ItemId::publish(&[CHARTS_CANVAS, PROTOCOL_OUTLINE]);
        assert_eq!(ItemId::known().len(), before);

        // An id no view published is still a load failure.
        let unknown = r#"{"view":"Charts","item":"a-pane-from-the-future"}"#;
        assert!(serde_json::from_str::<PaneKey>(unknown).is_err());
    }
}

// ---------------------------------------------------------------------------
// Unit tests — the migration, and the chrome a module owns
// ---------------------------------------------------------------------------

#[cfg(test)]
mod module_tests {
    use super::*;
    use crate::registry::{FieldSlot, FieldType, ModuleControl};

    /// A stand-in for a view's spec type — see
    /// [`ChartKind`](crate::registry::ChartKind) for why the registry never
    /// names a real one.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Spec {
        mark: &'static str,
        x: String,
        y: String,
        y_scale: &'static str,
    }

    const BARS: ChartKindId = ChartKindId::new("bars");
    const POINTS: ChartKindId = ChartKindId::new("points");
    const LOG_SCALE: &str = "log-scale";

    const BAR_SLOTS: &[FieldSlot] = &[
        FieldSlot::required("x", &[FieldType::Categorical]),
        FieldSlot::required("y", &[FieldType::Quantitative]),
    ];
    const POINT_SLOTS: &[FieldSlot] = &[
        FieldSlot::required("x", &[FieldType::Quantitative]),
        FieldSlot::required("y", &[FieldType::Quantitative]),
    ];

    /// The kind under test: what [`HandWrittenBars`] below does, expressed as
    /// data. It declares no controls, because the stand-in it is compared
    /// against declares none.
    fn bars() -> ChartKind<Spec> {
        ChartKind {
            id: BARS,
            icon: Icon("chart-bar"),
            description: "Ranks a category by a measure",
            slots: BAR_SLOTS,
            controls: Vec::new,
            build: |binding, _| Spec {
                mark: "bar",
                x: binding.name("x").unwrap_or_default().to_string(),
                y: binding.name("y").unwrap_or_default().to_string(),
                y_scale: "linear",
            },
        }
    }

    /// The second kind. Adding it is this function and its entry in the
    /// registry below — no component, no trait implementation, no branch
    /// anywhere else. It also carries a control of its own, which is what
    /// makes "a kind hangs its own chrome" a property of adding a kind rather
    /// than a feature of one.
    fn points() -> ChartKind<Spec> {
        ChartKind {
            id: POINTS,
            icon: Icon("chart-dots"),
            description: "Shows one measure against another",
            slots: POINT_SLOTS,
            controls: || {
                vec![
                    ModuleControl::new(LOG_SCALE, "Log", Verb::new("cycle-axis-lock"))
                        .with_tooltip("Log scale the measure axis"),
                ]
            },
            build: |binding, options| Spec {
                mark: "dot",
                x: binding.name("x").unwrap_or_default().to_string(),
                y: binding.name("y").unwrap_or_default().to_string(),
                y_scale: if options.is_on(LOG_SCALE) {
                    "log"
                } else {
                    "linear"
                },
            },
        }
    }

    /// The ink a module's body paints. Unique in the frame, so its bounding
    /// rect is findable in the tessellated output and "did this draw, and
    /// where" is answerable without a GPU.
    const BODY_INK: egui::Color32 = egui::Color32::from_rgb(7, 11, 13);
    /// The ink a log-scaled module's body paints instead, so a spec change is
    /// visible in the triangles rather than only in a struct comparison.
    const LOG_INK: egui::Color32 = egui::Color32::from_rgb(13, 11, 7);

    /// The block a bar module's body paints, and the one a dot module paints.
    /// Named rather than typed at the call site so a geometry assertion can
    /// state the size it expects.
    const BAR_INK: egui::Vec2 = egui::vec2(40.0, 12.0);
    const DOT_INK: egui::Vec2 = egui::vec2(20.0, 12.0);

    /// A view document: the chart vocabulary, and the one place a spec is
    /// drawn.
    struct Doc {
        kinds: ChartKindRegistry<Spec>,
        drawn: Vec<Spec>,
    }

    impl Doc {
        fn new() -> Self {
            Self::with_kinds(vec![bars(), points()])
        }

        fn with_kinds(kinds: Vec<ChartKind<Spec>>) -> Self {
            Self {
                kinds: ChartKindRegistry::new(kinds),
                drawn: Vec::new(),
            }
        }
    }

    impl ModuleHost for Doc {
        type Spec = Spec;

        fn chart_kinds(&self) -> &ChartKindRegistry<Spec> {
            &self.kinds
        }

        /// Paints the spec, so identical triangles mean an identical spec
        /// drawn into an identical rect — the two things "draws unchanged"
        /// has to cover.
        fn draw_module(&mut self, spec: &Spec, ui: &mut egui::Ui) {
            self.drawn.push(spec.clone());
            let ink = if spec.y_scale == "log" {
                LOG_INK
            } else {
                BODY_INK
            };
            let rect = ui.max_rect();
            let size = if spec.mark == "bar" { BAR_INK } else { DOT_INK };
            ui.painter()
                .rect_filled(egui::Rect::from_min_size(rect.min, size), 0.0, ink);
        }
    }

    /// A chart pane whose shape is a component per chart kind: it holds its
    /// own columns and builds its own spec inline, and a second kind would be
    /// a second type beside it.
    ///
    /// **A stand-in, not something this tree contains.** `brightfield-shell`'s
    /// `ChartItem` is already one item over a mark-kind enum — a new mark
    /// there is a new match arm, not a new shell. What a registry moves is
    /// where a kind is *declared*: an arm inside the component, or a value in
    /// a list outside it. This is the first of those, written small, so that
    /// "the kind draws what the component drew" is a comparison against
    /// something rather than an assertion about nothing.
    struct HandWrittenBars {
        item: ItemId,
        x: String,
        y: String,
    }

    impl Item<Doc> for HandWrittenBars {
        fn item_id(&self) -> ItemId {
            self.item
        }

        fn empty_state(&self, _doc: &Doc) -> Option<EmptyState> {
            (self.x.is_empty() || self.y.is_empty()).then(|| {
                EmptyState::new(
                    Icon("chart-bar"),
                    "Nothing here to chart yet",
                    "Bind a category and a measure and it draws.",
                )
            })
        }

        fn describe(&self, _doc: &Doc) -> Subject {
            Subject::new(
                "Revenue by sector",
                Icon("chart-bar"),
                BindingContext::Workspace,
            )
        }

        fn ui(&mut self, doc: &mut Doc, ui: &mut egui::Ui, _cx: &mut ItemCtx<'_>) {
            let spec = Spec {
                mark: "bar",
                x: self.x.clone(),
                y: self.y.clone(),
                y_scale: "linear",
            };
            doc.draw_module(&spec, ui);
        }
    }

    const PANE: egui::Rect = egui::Rect {
        min: egui::pos2(0.0, 0.0),
        max: egui::pos2(400.0, 300.0),
    };

    fn category(name: &str) -> Field {
        Field::new(name, FieldType::Categorical)
    }

    fn measure(name: &str) -> Field {
        Field::new(name, FieldType::Quantitative)
    }

    /// Draw one item the way the shell draws a pane — the real
    /// [`pane_frame`](crate::chrome::pane_frame), the real header rule, the
    /// real empty-state branch — and hand back the triangles that would reach
    /// the GPU.
    fn draw_pane(doc: &mut Doc, item: &mut dyn Item<Doc>) -> Vec<egui::ClippedPrimitive> {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(PANE),
            ..Default::default()
        };
        let mut requests = Vec::new();
        let full = ctx.run_ui(input, |ui| {
            let subject = item.subject(doc);
            let mut body = crate::chrome::pane_frame(ui, &subject, true, Mode::Light);
            if let Some(empty) = &subject.empty_state {
                crate::chrome::empty_state(&mut body, empty, Mode::Light);
            } else {
                let mut cx = ItemCtx::new(
                    Mode::Light,
                    PaneKey::new(ViewKind::Charts, item.item_id()),
                    egui_tiles::TileId::from_u64(1),
                    false,
                    &mut requests,
                );
                item.ui(doc, &mut body, &mut cx);
            }
        });
        ctx.tessellate(full.shapes, full.pixels_per_point)
    }

    /// The triangles of a frame, as comparable text: clip rect, then every
    /// vertex and index of every mesh, in draw order.
    fn triangles(primitives: &[egui::ClippedPrimitive]) -> String {
        let mut out = String::new();
        for primitive in primitives {
            out.push_str(&format!("clip {:?}\n", primitive.clip_rect));
            if let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive {
                for vertex in &mesh.vertices {
                    out.push_str(&format!("  v {:?}\n", vertex));
                }
                out.push_str(&format!("  i {:?}\n", mesh.indices));
            }
        }
        out
    }

    /// Where [`pane_frame`](crate::chrome::pane_frame) hands an item's body
    /// over inside [`PANE`]: below the header band, and inside the content
    /// inset on every side.
    ///
    /// Derived from the chrome's own published measurements and from nothing
    /// the tests below draw with. That independence is the point — an
    /// assertion against this says *where* ink landed, where a pane-to-pane
    /// comparison can only say that two routines agreed on a place, and both
    /// panes below paint through the one `Doc::draw_module`.
    fn body_origin() -> egui::Pos2 {
        let inset = crate::chrome::pane_content_inset();
        egui::pos2(
            PANE.left() + inset,
            PANE.top() + crate::chrome::header_band_height() + inset,
        )
    }

    /// The anti-aliasing inset a vertex position carries. The tessellator
    /// feathers a filled rect outward with transparent vertices and pulls the
    /// opaque ones in by half the feather width, so a rect read back out of a
    /// tessellated frame sits this far inside the rect that was asked for.
    /// Measured on this egui rather than assumed: at 0.0, `ink_at` reports the
    /// exact gap it found.
    const INK_FEATHER: f32 = 0.5;

    /// Whether the ink `found` in a frame is the rect that was `asked` for.
    ///
    /// Allows for [`INK_FEATHER`] and for float error in the layout that
    /// produced it, and for nothing a misplaced paint could hide behind: the
    /// slack is 0.01pt, against displacements measured in whole points.
    fn ink_at(found: egui::Rect, asked: egui::Rect) -> bool {
        let want = asked.shrink(INK_FEATHER);
        (found.min - want.min).abs().max_elem() < 0.01
            && (found.max - want.max).abs().max_elem() < 0.01
    }

    /// The bounding rect of every vertex painted in exactly `ink`, or `None`
    /// when nothing in the frame was — a clipped shape contributes no
    /// vertices, so absence is an assertion.
    fn painted(primitives: &[egui::ClippedPrimitive], ink: egui::Color32) -> Option<egui::Rect> {
        let mut found: Option<egui::Rect> = None;
        for primitive in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive else {
                continue;
            };
            for vertex in mesh.vertices.iter().filter(|v| v.color == ink) {
                let point = egui::Rect::from_min_max(vertex.pos, vertex.pos);
                found = Some(found.map_or(point, |r| r.union(point)));
            }
        }
        found
    }

    // -----------------------------------------------------------------------
    // The migration
    // -----------------------------------------------------------------------

    /// The migrated kind builds the **same spec** the hand-written component
    /// built, over the same columns.
    ///
    /// A spec is the whole of what reaches the renderer, so an equal spec is
    /// an equal chart — this is the half of "draws unchanged" that does not
    /// depend on any drawing at all.
    #[test]
    fn the_migrated_kind_builds_the_spec_the_hand_written_component_built() {
        let doc = Doc::new();
        let module = ChartModule::new(
            ItemId::new("module-bars"),
            "Revenue by sector",
            &bars(),
            vec![category("sector"), measure("revenue")],
        );
        assert_eq!(
            module.spec(&doc),
            Some(Spec {
                mark: "bar",
                x: "sector".to_string(),
                y: "revenue".to_string(),
                y_scale: "linear",
            })
        );
    }

    /// …and drawn through the real pane frame, the registry-driven module puts
    /// the **same triangles** on the screen as the hand-written stand-in — at
    /// the place the pane frame hands the body over, not merely at the same
    /// place as each other.
    ///
    /// Two assertions, and the first is what makes the second worth anything.
    /// Both panes paint through the one `Doc::draw_module`, so a fault *there*
    /// moves both bodies together and an equality between them stays green
    /// through it: shared wrongness is the only way these two panes can differ
    /// from correct, and it is exactly what comparing them cannot see. So each
    /// body's ink is pinned to an absolute rect first, built by
    /// [`body_origin`] from [`PANE`], the header band and the content inset —
    /// none of which the painting under test contributes to.
    ///
    /// The triangle comparison then covers what an absolute rect cannot: the
    /// chrome around the body. This kind declares no controls, so
    /// [`module_frame`](crate::chrome::module_frame) takes no height and
    /// paints nothing, and the pane renders exactly as one holding an item
    /// with no module chrome at all. A single pixel of strip, a single point
    /// of reserved gap, and that half goes red.
    #[test]
    fn the_migrated_kind_draws_the_same_triangles_as_the_hand_written_stand_in() {
        let fields = vec![category("sector"), measure("revenue")];

        let mut before_doc = Doc::new();
        let mut before = HandWrittenBars {
            item: ItemId::new("module-bars"),
            x: "sector".to_string(),
            y: "revenue".to_string(),
        };
        let before_frame = draw_pane(&mut before_doc, &mut before);

        let mut after_doc = Doc::new();
        let mut after = ChartModule::new(
            ItemId::new("module-bars"),
            "Revenue by sector",
            &bars(),
            fields,
        );
        let after_frame = draw_pane(&mut after_doc, &mut after);

        let want = egui::Rect::from_min_size(body_origin(), BAR_INK);
        for (whose, frame) in [
            ("the stand-in", &before_frame),
            ("the registry-driven module", &after_frame),
        ] {
            let ink = painted(frame, BODY_INK)
                .unwrap_or_else(|| panic!("{whose}: no body ink reached the frame at all"));
            assert!(
                ink_at(ink, want),
                "{whose} painted its body at {ink:?}; the pane frame hands the \
                 body over at {want:?}, less a {INK_FEATHER}pt feather"
            );
        }

        assert_eq!(before_doc.drawn, after_doc.drawn, "the same spec was drawn");
        assert_eq!(
            triangles(&before_frame),
            triangles(&after_frame),
            "the registry-driven module drew different triangles from the \
             hand-written stand-in"
        );
    }

    /// The empty-state branch survives the migration too: a module whose
    /// columns do not fill its kind's required slots renders the chrome's
    /// empty state, not a header and silence.
    #[test]
    fn a_module_whose_columns_do_not_fit_renders_an_empty_state() {
        let doc = Doc::new();
        let module = ChartModule::new(
            ItemId::new("module-bars"),
            "Revenue by sector",
            &bars(),
            vec![measure("revenue")],
        );
        let empty = Item::subject(&module, &doc)
            .empty_state
            .expect("one measure does not fill a category slot");
        assert_eq!(empty.headline, "Nothing here to chart yet");
        assert!(
            empty.body.contains("\"x\""),
            "the body says which slot is unfilled: {}",
            empty.body
        );
        assert_eq!(module.spec(&doc), None);
    }

    // -----------------------------------------------------------------------
    // A second kind, and no second component
    // -----------------------------------------------------------------------

    /// Every kind the registry declares draws through the one component.
    ///
    /// Stated over the whole registry rather than over the kind that happened
    /// to be added, so "adding a kind needs no new component" is a property of
    /// the set: a kind that needed one would have to appear here as a second
    /// type, and there is nowhere for it to go.
    #[test]
    fn every_registered_kind_draws_through_the_one_component() {
        let ids = Doc::new().kinds.ids();
        assert_eq!(ids, vec![BARS, POINTS], "the fixture declares two kinds");

        for id in ids {
            let mut doc = Doc::new();
            let kind = doc.kinds.find(id).expect("just enumerated");
            let fields = match id {
                BARS => vec![category("sector"), measure("revenue")],
                _ => vec![measure("revenue"), measure("margin")],
            };
            let mut module: ChartModule =
                ChartModule::new(ItemId::new("module"), "A module", kind, fields);
            let frame = draw_pane(&mut doc, &mut module);
            assert_eq!(doc.drawn.len(), 1, "{id}: drew no spec");
            assert!(
                painted(&frame, BODY_INK).is_some(),
                "{id}: the module's body did not reach the frame"
            );
        }
    }

    /// The second kind picks itself out of the registry by its declared type
    /// constraints, with no caller-side branch on which kind it is.
    #[test]
    fn the_registry_chooses_between_the_two_kinds_by_column_type() {
        let doc = Doc::new();
        assert_eq!(
            doc.kinds
                .applicable(&[category("sector"), measure("revenue")]),
            vec![BARS]
        );
        assert_eq!(
            doc.kinds
                .applicable(&[measure("revenue"), measure("margin")]),
            vec![POINTS]
        );
    }

    // -----------------------------------------------------------------------
    // The chrome a module owns
    // -----------------------------------------------------------------------

    /// A module's own controls are **not** in its subject, which is what stops
    /// the surface around the pane from drawing them.
    #[test]
    fn a_modules_own_controls_are_not_the_panes_toolbar() {
        let doc = Doc::new();
        let kind = points();
        let module = ChartModule::new(
            ItemId::new("module-points"),
            "Margin against revenue",
            &kind,
            vec![measure("revenue"), measure("margin")],
        );
        assert_eq!(
            kind.toolbar(module.options())
                .iter()
                .map(|e| e.id)
                .collect::<Vec<_>>(),
            vec![LOG_SCALE],
            "the kind declares a control"
        );
        assert!(
            Item::subject(&module, &doc).toolbar.is_empty(),
            "and the pane's toolbar — the canvas's chrome — stays empty"
        );
    }

    /// The declared chrome draws **inside the module**: it takes space out of
    /// the module's own rect and pushes the module's body down, rather than
    /// appearing anywhere the pane's own chrome lives.
    #[test]
    fn a_declared_control_takes_space_from_the_module_and_not_from_the_pane() {
        let fields = vec![measure("revenue"), measure("margin")];

        // The same kind with its controls removed. It has to go in a registry
        // of its own, because a module resolves its kind through the document
        // at draw time rather than holding the value it was built from — the
        // whole reason a kind can be changed without touching a module.
        let bare = ChartKind {
            controls: Vec::new,
            ..points()
        };

        let mut plain_doc = Doc::with_kinds(vec![ChartKind {
            controls: Vec::new,
            ..points()
        }]);
        let mut plain = ChartModule::new(
            ItemId::new("module-points"),
            "Margin against revenue",
            &bare,
            fields.clone(),
        );
        let plain_frame = draw_pane(&mut plain_doc, &mut plain);
        let plain_body = painted(&plain_frame, BODY_INK).expect("the plain module drew");

        let mut chromed_doc = Doc::new();
        let mut chromed = ChartModule::new(
            ItemId::new("module-points"),
            "Margin against revenue",
            &points(),
            fields,
        );
        let chromed_frame = draw_pane(&mut chromed_doc, &mut chromed);
        let chromed_body = painted(&chromed_frame, BODY_INK).expect("the chromed module drew");

        assert!(
            chromed_body.top() > plain_body.top(),
            "the declared control reserved no space in the module: body at {} \
             either way",
            plain_body.top()
        );
        assert!(
            chromed_body.top() < PANE.bottom(),
            "the control strip pushed the module's body off the pane"
        );
        assert_eq!(
            plain_doc.drawn, chromed_doc.drawn,
            "the two modules drew the same spec, so the shift is the strip and \
             not a different chart"
        );
    }

    /// The control's verb is the module's: it flips that module's option,
    /// changes that module's spec, and is reported handled so it never reaches
    /// the workspace.
    #[test]
    fn the_control_verb_is_performed_by_the_module_and_stops_there() {
        let mut doc = Doc::new();
        let mut module = ChartModule::new(
            ItemId::new("module-points"),
            "Margin against revenue",
            &points(),
            vec![measure("revenue"), measure("margin")],
        );
        assert_eq!(
            module.spec(&doc).expect("the columns fit").y_scale,
            "linear"
        );

        let mut requests = Vec::new();
        let mut cx = ItemCtx::new(
            Mode::Light,
            PaneKey::new(ViewKind::Charts, ItemId::new("module-points")),
            egui_tiles::TileId::from_u64(1),
            true,
            &mut requests,
        );
        assert_eq!(
            module.perform(&mut doc, Verb::new("cycle-axis-lock"), &mut cx),
            Handled::Yes,
            "a control's verb belongs to the module that declared it"
        );
        assert_eq!(module.spec(&doc).expect("the columns fit").y_scale, "log");

        // A verb no control declares still bubbles.
        assert_eq!(
            module.perform(&mut doc, Verb::new("reset-extent"), &mut cx),
            Handled::No
        );

        // And the flip reaches the pixels, not only the struct.
        let frame = draw_pane(&mut doc, &mut module);
        assert!(
            painted(&frame, LOG_INK).is_some(),
            "the module redrew at the old scale"
        );
        assert!(painted(&frame, BODY_INK).is_none());
    }

    /// A module whose kind this build does not have says so, rather than
    /// drawing a header and silence.
    #[test]
    fn a_module_naming_an_absent_kind_says_which_kind_is_missing() {
        let doc = Doc::new();
        let stranger = ChartKind::<Spec> {
            id: ChartKindId::new("sunburst"),
            ..bars()
        };
        let module = ChartModule::new(
            ItemId::new("module-stranger"),
            "A stranger",
            &stranger,
            vec![category("sector"), measure("revenue")],
        );
        let empty = Item::subject(&module, &doc)
            .empty_state
            .expect("the registry has no sunburst");
        assert!(
            empty.body.contains("sunburst"),
            "the body names the missing kind: {}",
            empty.body
        );
    }
}
