//! The surfaces the shell owns rather than a pane.
//!
//! A [`crate::Subject`] covers what the *focused pane* contributes to the
//! chrome. Some chrome belongs to the window instead — the view switcher, the
//! mode toggle, the connection state, a modal. Those are declared here, and
//! the crucial thing they share with a `Subject` is that they still return
//! the same plain-data [`crate::ToolbarEntry`] and [`crate::StatusEntry`]
//! types. There is exactly one vocabulary for "a control in a toolbar",
//! whether the control came from a pane or from the shell, and therefore
//! exactly one way it can look.
//!
//! Note the missing generic: these traits are doc-agnostic. A shell-owned
//! control that needed to read a view's document would be a pane's control
//! wearing the wrong hat.

use crate::item::{PaneKey, Request};
use crate::subject::{StatusEntry, ToolbarEntry};
use crate::workspace::ViewKind;
use crate::Mode;

/// What a shell-owned surface can see.
#[derive(Clone, Copy, Debug)]
pub struct WorkspaceView<'a> {
    /// Light or dark.
    pub mode: Mode,
    /// The active view.
    pub view: ViewKind,
    /// The focused pane in the active view, if any.
    pub focused: Option<PaneKey>,
    /// Borrowed so this stays a view rather than a snapshot that can go
    /// stale, and so the type can grow read-only fields without every call
    /// site changing shape.
    _marker: std::marker::PhantomData<&'a ()>,
}

impl WorkspaceView<'_> {
    /// A read view over the workspace.
    #[must_use]
    pub const fn new(mode: Mode, view: ViewKind, focused: Option<PaneKey>) -> Self {
        Self {
            mode,
            view,
            focused,
            _marker: std::marker::PhantomData,
        }
    }
}

/// What a shell-owned surface can ask for.
///
/// Mutation is by request, not by handle, for the same reason a pane's is:
/// the toolbar draws while the workspace is mid-frame, and letting a control
/// switch the active view from inside the row that is drawing it means
/// mutating the thing being iterated.
pub struct WorkspaceCtx<'a> {
    /// Light or dark.
    pub mode: Mode,
    /// The active view.
    pub view: ViewKind,
    /// The focused pane in the active view, if any.
    pub focused: Option<PaneKey>,
    requests: &'a mut Vec<Request>,
}

impl<'a> WorkspaceCtx<'a> {
    /// A mutating context over the workspace.
    pub fn new(
        mode: Mode,
        view: ViewKind,
        focused: Option<PaneKey>,
        requests: &'a mut Vec<Request>,
    ) -> Self {
        Self {
            mode,
            view,
            focused,
            requests,
        }
    }
}

impl WorkspaceCtx<'_> {
    /// The read view corresponding to this context.
    #[must_use]
    pub const fn view_only(&self) -> WorkspaceView<'_> {
        WorkspaceView::new(self.mode, self.view, self.focused)
    }

    /// Ask the workspace to run a command.
    pub fn request(&mut self, verb: crate::subject::Verb) {
        self.requests.push(Request::Verb(verb));
    }

    /// Ask the workspace to move focus.
    pub fn focus(&mut self, key: PaneKey) {
        self.requests.push(Request::Focus(key));
    }

    /// Ask for another frame.
    pub fn request_repaint(&mut self) {
        self.requests.push(Request::Repaint);
    }
}

/// A control the shell owns, drawn in the same row as the focused pane's.
pub trait ToolbarItem {
    /// What this control is, this frame.
    fn entry(&self, ws: &WorkspaceView<'_>) -> ToolbarEntry;
    /// Activate it.
    fn perform(&mut self, ws: &mut WorkspaceCtx<'_>);
}

/// A status line the shell owns, drawn in the same rail as the focused
/// pane's.
pub trait StatusItem {
    /// What this line says, this frame.
    fn entry(&self, ws: &WorkspaceView<'_>) -> StatusEntry;
}

/// What a modal reports back after a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModalOutcome {
    /// Still open.
    #[default]
    Open,
    /// The user confirmed. Close it and act.
    Accept,
    /// The user backed out. Close it and do nothing.
    Dismiss,
}

/// A modal sheet over the whole window.
pub trait ModalView {
    /// Stable id, for a test hook and for refusing to stack two of the same.
    fn modal_id(&self) -> &'static str;
    /// The title bar text.
    fn title(&self) -> String;
    /// How wide. Defaults to the design system's default modal width — a
    /// modal that picks its own width is how a product ends up with five.
    fn width(&self) -> f32 {
        meridian_design::spacing::MODAL_WIDTH_DEFAULT
    }
    /// Draw the body and report what the user did. The card, its title, its
    /// scrim and its padding are the shell's; this draws only the contents.
    fn ui(&mut self, ui: &mut egui::Ui, ws: &mut WorkspaceCtx<'_>) -> ModalOutcome;
}
