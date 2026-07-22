//! The shell contract.
//!
//! # The problem this crate exists to solve
//!
//! A shell that grows one surface at a time grows one treatment per surface.
//! Counted in the egui shell before this crate existed: five treatments of
//! "this thing is selected", seven empty states written seven different ways
//! (two of which were simply missing, so an empty list rendered a header and
//! silence), three pane headers with three different spellings of the same
//! idea, and two top bars whose type sizes differed by four pixels because
//! nobody chose it. None of that was carelessness — each one was locally
//! reasonable at the moment it was written. The drift is structural: when any
//! surface *may* draw its own chrome, every surface eventually *does*.
//!
//! # The shape of the fix
//!
//! A pane does not draw chrome. It **declares** what its chrome should say,
//! once per frame, as plain data — a [`Subject`] — and the shell draws the
//! pane header, breadcrumb, tab title, dirty marker, toolbar row and status
//! rail from it, in one file ([`chrome`]).
//!
//! Neither of the two properties below is enforced by the compiler. Both are
//! stated here with the strength they actually have, because code written
//! against an overstated guarantee is worse off than code written against a
//! known-soft one:
//!
//! - **A pane's `Ui` is clipped below the header band — by default.**
//!   [`Item::ui`] receives a child `Ui` whose `max_rect` *and clip rect* are
//!   both the content rect reserved below the band, so a header drawn the
//!   ordinary way never reaches the pixels. A pane that means to escape can:
//!   `Ui::set_clip_rect` widens the clip on the `Ui` it was handed, and
//!   `egui::Area` / `ctx.layer_painter` take a fresh layer outright. egui
//!   offers no way to withhold either from a `&mut Ui` holder. The honest
//!   statement is *unreachable by accident, reviewable on purpose* — and the
//!   reason review suffices is that no [`Subject`] field asks for a header, so
//!   a pane wanting one has already left the contract. See
//!   [`chrome::pane_frame`].
//! - **A forgotten empty state is caught by an audit, not by the compiler.**
//!   [`Subject::empty_state`] is an `Option` and returning `None` compiles
//!   cleanly. [`audit`] catches it: it walks a registry, asks every item for
//!   its subject over an empty document, and rejects a missing empty state —
//!   along with prose that breaks the house style, an unregistered verb, and a
//!   rail with no toggle. It ships in the crate rather than as a helper each
//!   view's tests re-write, so a view gates itself with one call. That is only
//!   as good as the registry being the door every pane comes through, which is
//!   a convention this crate makes convenient rather than one it can enforce —
//!   `egui_tiles` will insert a pane for anyone who asks it.
//!
//! Because a `Subject` is plain data — no egui type, no colour, no GPU handle
//! anywhere in it — the entire visible vocabulary of a surface can be
//! asserted in a headless unit test. That is the point of the plain-data
//! constraint, and it is worth defending against the first "it would be
//! convenient to put an `egui::Color32` here".
//!
//! # Layout
//!
//! - [`subject`] — [`Subject`] and its parts, plus [`Verb`]. No egui types.
//! - [`item`] — [`Item`], [`ItemId`], [`PaneKey`], [`ItemCtx`].
//! - [`registry`] — [`Slot`], [`ItemSpec`], [`ItemRegistry`]: the only route
//!   from an item to a pane, and therefore the thing the contract tests gate;
//!   plus [`audit`], the gate itself.
//! - [`shell`] — the surfaces the *shell* owns rather than a pane:
//!   [`ToolbarItem`], [`StatusItem`], [`ModalView`].
//! - [`workspace`] — [`ViewKind`] and [`Workspace`]: one tile tree per view,
//!   plus which pane holds focus in each.
//! - [`behavior`] — [`PaneChrome`], the one `egui_tiles::Behavior`.
//! - [`persist`] — the versioned layout file and its debounced writer.
//! - [`chrome`] — the one drawing file.

pub mod behavior;
pub mod chrome;
pub mod item;
pub mod persist;
pub mod registry;
pub mod shell;
pub mod subject;
pub mod workspace;

pub use behavior::PaneChrome;
pub use item::{Handled, Item, ItemCtx, ItemId, ItemMap, PaneKey, Request};
pub use persist::{DirtyTracker, LoadOutcome, SavedLayout, WindowGeometry};
pub use registry::{audit, DockSide, ItemRegistry, ItemSpec, Slot};
pub use shell::{ModalOutcome, ModalView, StatusItem, ToolbarItem, WorkspaceCtx, WorkspaceView};
pub use subject::{
    Action, Affordance, Crumb, Dirty, EmptyState, HideAffordance, Icon, StatusEntry, StatusSide,
    Subject, Tone, ToolbarEntry, ToolbarLocation, Verb,
};
pub use workspace::{ViewKind, Workspace};

/// Light or dark chrome — re-exported from `meridian-egui`.
///
/// The theme selector and the token → egui `Style`/`Visuals`/font bridge moved
/// to the design repo's egui emitter (they belong beside the tokens they
/// translate, not in this consumer). [`chrome`] still resolves every colour it
/// paints through `meridian_design::semantic(dark)`, so it takes a [`Mode`] by
/// value; the shell holds its own copy. Re-exported here so
/// `brightfield_workbench::Mode` and `crate::Mode` resolve unchanged across the
/// crate.
pub use meridian_egui::Mode;
