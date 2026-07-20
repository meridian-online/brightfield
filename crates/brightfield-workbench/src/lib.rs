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
//! Two properties make that a contract rather than a convention:
//!
//! - **A pane has no `Ui` to draw a header into.** [`Item::ui`] receives a
//!   child `Ui` whose `max_rect` is the content rect the shell reserved
//!   *below* the header band. Drawing a header is not discouraged; it is
//!   unreachable.
//! - **An empty state cannot be skipped.** [`Subject::empty_state`] is an
//!   `Option`, and when it is `Some` the shell paints it and does **not**
//!   call [`Item::ui`] at all. Forgetting one is not a silent blank pane; it
//!   is a test failure, because [`EmptyState`] is a required struct with
//!   required prose and the registry is the only route a pane has into a
//!   layout.
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
//!   from an item to a pane, and therefore the thing the contract tests gate.
//! - [`shell`] — the surfaces the *shell* owns rather than a pane:
//!   [`ToolbarItem`], [`StatusItem`], [`ModalView`].
//! - [`workspace`] — [`ViewKind`], the view a pane belongs to.
//! - [`chrome`] — the one drawing file.

pub mod chrome;
pub mod item;
pub mod registry;
pub mod shell;
pub mod subject;
pub mod workspace;

pub use item::{Handled, Item, ItemCtx, ItemId, ItemMap, PaneKey, Request};
pub use registry::{DockSide, ItemRegistry, ItemSpec, Slot};
pub use shell::{ModalOutcome, ModalView, StatusItem, ToolbarItem, WorkspaceCtx, WorkspaceView};
pub use subject::{
    Affordance, Crumb, Dirty, EmptyState, HideAffordance, Icon, StatusEntry, StatusSide, Subject,
    Tone, ToolbarEntry, ToolbarLocation, Verb,
};
pub use workspace::ViewKind;

/// Light or dark chrome.
///
/// This lives here, not in the egui shell's design bridge, because [`chrome`]
/// resolves every colour it paints through `meridian_design::semantic(dark)`
/// and therefore needs to know the mode before any shell type exists. The
/// shell re-exports it so nothing downstream had to change when it moved.
///
/// [`Mode::Light`] remains the default: the Vello chart canvas is light-first
/// this phase, and dark chrome around a white chart reads as broken until
/// dark chart ink lands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Light chrome (the current default).
    #[default]
    Light,
    /// Dark chrome (tokens ready; chart ink follows in a later phase).
    Dark,
}

impl Mode {
    /// Whether this is the dark mode — the argument every design-token
    /// accessor takes (`semantic(dark)`, `Elevation::shadow(dark)`).
    #[must_use]
    pub const fn is_dark(self) -> bool {
        matches!(self, Mode::Dark)
    }

    /// The other mode.
    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Mode::Light => Mode::Dark,
            Mode::Dark => Mode::Light,
        }
    }
}
