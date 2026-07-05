//! WorkspaceView — the window-root GPUI shim over the workspace shell model
//! (card 0016).
//!
//! Hosts the shell chrome — a header strip carrying the resolved dashboard
//! title, and a padded content area centring the existing [`ChartView`] —
//! and translates the `TogglePresentation` action into a
//! [`PresentationMode`] flip. All decisions live in the gpui-free
//! `workspace` module (semantic-layer rule); this view only reads them.
//! Deliberately thin so a gpui-component `DockArea` can absorb it at the
//! editor milestone without touching `ChartView`.

use gpui::{
    actions, div, px, rgb, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Render, SharedString, Styled, Window,
};

use crate::chart_view::ChartView;
use crate::workspace::{PresentationMode, CONTENT_PADDING, HEADER_HEIGHT};

actions!(brightfield, [TogglePresentation]);

/// Key context of the workspace root — the scope the `p` binding dispatches
/// in. A future editor pane gets its own context (VisiData-style bare-letter
/// grammar, per the tabletop's locked pick).
pub const WORKSPACE_KEY_CONTEXT: &str = "BrightfieldWorkspace";

/// The workspace key bindings, declared as data (remappable config), not a
/// hardcoded key match: bare `p` → [`TogglePresentation`], scoped to
/// [`WORKSPACE_KEY_CONTEXT`]. `main` feeds these to `cx.bind_keys`.
pub fn workspace_key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "p",
        TogglePresentation,
        Some(WORKSPACE_KEY_CONTEXT),
    )]
}

/// Window root: header strip (dashboard title) + padded content area mounting
/// the dashboard's [`ChartView`]. Presentation mode hides the chrome and
/// re-centres the canvas in the unchanged window.
pub struct WorkspaceView {
    /// Resolved dashboard title (the same string the native titlebar shows).
    title: SharedString,
    /// The hosted dashboard view.
    dashboard: Entity<ChartView>,
    /// Authoring ↔ presentation state (gpui-free machine; the view reads it).
    mode: PresentationMode,
    /// Focus handle so the workspace key context receives key dispatch.
    focus_handle: FocusHandle,
}

impl WorkspaceView {
    /// Create the workspace root hosting `dashboard` under `title`.
    pub fn new(
        title: impl Into<SharedString>,
        dashboard: Entity<ChartView>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            title: title.into(),
            dashboard,
            mode: PresentationMode::default(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// The current surface mode.
    pub fn mode(&self) -> PresentationMode {
        self.mode
    }

    /// `TogglePresentation` handler: flip the state machine and repaint.
    fn toggle_presentation(
        &mut self,
        _: &TogglePresentation,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode.toggle();
        cx.notify();
    }
}

impl Focusable for WorkspaceView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The white-bg/centring shell (previously ChartView's) lives up here
        // now: ChartView keeps only the fixed-size relative canvas. The
        // content area centres the canvas, so presentation mode re-centres in
        // the freed space with NO window resize (flex reclaims the chrome).
        let root = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .key_context(WORKSPACE_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::toggle_presentation));

        let canvas = div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .child(self.dashboard.clone());

        if self.mode.chrome_visible() {
            root.child(
                // Header strip: the resolved title in a thin bar above the canvas.
                div()
                    .w_full()
                    .h(px(HEADER_HEIGHT as f32))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(CONTENT_PADDING as f32))
                    .bg(rgb(0xf7f7f7))
                    .border_b_1()
                    .border_color(rgb(0xe2e2e2))
                    .text_size(px(13.0))
                    .text_color(rgb(0x333333))
                    .child(self.title.clone()),
            )
            .child(canvas.p(px(CONTENT_PADDING as f32)))
        } else {
            root.child(canvas)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Keystroke;

    /// fww_ac07 (binding data): the workspace keymap is declared as data —
    /// exactly one binding, bare `p`, dispatching `TogglePresentation` inside
    /// the workspace key context. (The handler's flip is fww_ac03's tested
    /// state machine; physical keypress delivery is ac-11's manual eyeball.)
    #[test]
    fn fww_ac07_p_binds_toggle_presentation_in_workspace_context() {
        let bindings = workspace_key_bindings();
        assert_eq!(bindings.len(), 1, "one action, one binding");
        let binding = &bindings[0];

        // Bare `p` matches the binding completely (Some(false) = no pending
        // keystrokes), and nothing else does.
        let p = Keystroke::parse("p").expect("parses");
        assert_eq!(
            binding.match_keystrokes(std::slice::from_ref(&p)),
            Some(false),
            "bare p is a complete match"
        );
        let q = Keystroke::parse("q").expect("parses");
        assert_eq!(binding.match_keystrokes(std::slice::from_ref(&q)), None);

        // It dispatches TogglePresentation, scoped to the workspace context.
        assert_eq!(binding.action().name(), "brightfield::TogglePresentation");
        assert!(
            binding.predicate().is_some(),
            "the binding is canvas-scoped, not global"
        );
    }
}
