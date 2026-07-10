//! The docked authoring workspace (card 0017) — the GPUI/gpui-component
//! translation shim.
//!
//! Views here are deliberately thin (semantic-layer rule): every decision —
//! panel visibility, save timing, load fallback, atomic writes, sidebar
//! contents, notification routing, the log model, the bottom-dock backfill
//! and presentation action — lives in the framework-free modules
//! (`shell_model`, `dock_state_file`, `spec_save`, `reload_feedback`,
//! `sidebar_model`, `log_model`); this file only executes them against
//! gpui-component's `DockArea`/`Panel`/`Root` machinery.
//!
//! - [`CanvasPanel`] — a Panel shim AROUND the untouched [`ChartView`]
//!   entity (aws_ac02): white canvas surface, workspace key context (bare
//!   `p` stays canvas-scoped), no chart event is intercepted or transformed.
//! - [`EditorPanel`] — `InputState::code_editor("yaml")`; cmd-s dispatches
//!   [`SaveSpec`], whose handler is `spec_save::save_spec_atomic` — the
//!   existing mtime watcher does everything else (aws_ac04).
//! - [`SidebarPanel`] — renders the derived [`SourceListing`]s (aws_ac06).
//! - [`LogPanel`] — the bottom-dock reload/save feedback history over the
//!   gpui-free [`FeedbackLog`] (wsc_ac02).
//! - [`WorkspaceRoot`] — hosts the `DockArea` (center canvas, right editor,
//!   left sidebar, bottom log), loads/saves the versioned layout JSON
//!   (aws_ac03), backfills pre-round layouts with the closed Log dock
//!   (wsc_ac03), and removes/rebuilds the bottom dock across presentation
//!   mode (wsc_ac04).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    actions, div, px, rgb, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement, Pixels,
    Render, SharedString, Styled, Task, WeakEntity, Window,
};
use gpui_component::dock::{
    register_panel, Dock, DockArea, DockAreaState, DockEvent, DockItem, DockPlacement, Panel,
    PanelControl, PanelEvent, PanelState, PanelView,
};
use gpui_component::input::{Input, InputState};
use gpui_component::menu::PopupMenu;
use gpui_component::notification::Notification;
use gpui_component::{ActiveTheme as _, Root};

use brightfield_ui::{ChartView, PresentationMode, TogglePresentation, WORKSPACE_KEY_CONTEXT};

use crate::dock_state_file::{
    self, LoadDecision, SaveAction, SavePolicy, DOCK_STATE_VERSION, SAVE_DEBOUNCE_MS,
};
use crate::log_model::FeedbackLog;
use crate::reload_feedback::{self, Severity};
use crate::shell_model::{
    bottom_dock_action, bottom_dock_needs_backfill, dock_closes_when_emptied, docks_open,
    layout_persistable, panel_visible, BottomDockAction, PanelRole, BOTTOM_DOCK_HEIGHT,
    CANVAS_PANEL_NAME, EDITOR_DOCK_WIDTH, EDITOR_PANEL_NAME, LOG_PANEL_NAME, SIDEBAR_DOCK_WIDTH,
    SIDEBAR_PANEL_NAME,
};
use crate::sidebar_model::SourceListing;
use crate::spec_save;

actions!(
    brightfield,
    [
        SaveSpec,
        // wsc_ac07 menu-move actions: at pin b7e63cc2 a dock's ONLY panel
        // can never start a drag (is_last_panel), so the dropdown menu is
        // the bootstrap that first pairs panels up — after which real
        // drags work. Dispatched by the panels' tab menus; handled on the
        // WorkspaceRoot render root (every dispatch path bubbles there).
        DockEditorAtBottom,
        DockEditorAtRight,
        DockSidebarAtBottom,
        DockSidebarAtLeft
    ]
);

/// Key context of the spec editor panel — the scope the cmd-s binding
/// dispatches in (nested above the Input's own context, so the binding
/// fires only while the editor has focus).
pub const EDITOR_KEY_CONTEXT: &str = "BrightfieldEditor";

/// The DockArea's stable identity (state files key on it).
pub const DOCK_AREA_ID: &str = "brightfield-workspace";

/// The editor key bindings, declared as data (aws_ac04): cmd-s →
/// [`SaveSpec`], scoped to [`EDITOR_KEY_CONTEXT`]. `main` feeds these to
/// `cx.bind_keys` alongside the workspace bindings.
pub fn editor_key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("cmd-s", SaveSpec, Some(EDITOR_KEY_CONTEXT))]
}

/// The one bit of shared shell mode: card 0016's gpui-free
/// [`PresentationMode`] (unmoved), wrapped in an entity so every panel's
/// `visible()` reads the same state the canvas's `p` handler flips.
pub struct PresentationState {
    /// Authoring ↔ presentation (the framework-free machine decides; views
    /// read).
    pub mode: PresentationMode,
}

// ---------------------------------------------------------------------------
// Canvas panel (aws_ac02)
// ---------------------------------------------------------------------------

/// The locked center panel: a thin Panel shim around the UNTOUCHED
/// [`ChartView`] entity. It adds hosting only — white surface, flex
/// centring, the canvas-scoped key context for bare `p`, a click-to-focus
/// listener — and delegates all rendering and interaction to the entity it
/// holds; no chart event is intercepted or transformed.
pub struct CanvasPanel {
    /// The dashboard view this panel wraps (the SAME entity `main` built —
    /// brush/point/slider/legend wiring rides along untouched).
    chart_view: Entity<ChartView>,
    /// Resolved dashboard title (`meta.title` or spec filename stem) — the
    /// panel's tab/title text.
    title: SharedString,
    /// Shared presentation state (the `p` handler flips it here).
    presentation: Entity<PresentationState>,
    /// The hosting dock area, for collapsing the authoring docks on toggle.
    /// Set by [`WorkspaceRoot::new`] once the dock exists.
    dock_area: Option<WeakEntity<DockArea>>,
    /// Focus handle: the workspace key context dispatches from here.
    focus_handle: FocusHandle,
}

impl CanvasPanel {
    /// Wrap `chart_view` under the resolved dashboard `title`.
    pub fn new(
        chart_view: Entity<ChartView>,
        title: impl Into<SharedString>,
        presentation: Entity<PresentationState>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            chart_view,
            title: title.into(),
            presentation,
            dock_area: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Wire the hosting dock area (called once the `DockArea` exists).
    pub fn set_dock_area(&mut self, dock_area: WeakEntity<DockArea>) {
        self.dock_area = Some(dock_area);
    }

    /// The wrapped dashboard view (shim assertion surface, aws_ac02).
    #[cfg(test)]
    pub fn chart_view(&self) -> &Entity<ChartView> {
        &self.chart_view
    }

    /// The panel's title text (shim assertion surface, aws_ac02).
    #[cfg(test)]
    pub fn title_text(&self) -> &SharedString {
        &self.title
    }

    /// `TogglePresentation` handler (bare `p`, canvas-scoped — the binding
    /// is card 0016's, unchanged): flip the shared mode, then apply the
    /// framework-free dock mapping — panels re-read `visible()` on the
    /// repaint, the left/right docks collapse/reopen (aws_ac07). The BOTTOM
    /// dock is not touched here: its closed form still paints a 29px strip,
    /// so `WorkspaceRoot` (observing the shared mode) removes and rebuilds
    /// it instead (wsc_ac04) — and the stash it takes must see the dock's
    /// open bit exactly as the author left it.
    fn toggle_presentation(
        &mut self,
        _: &TogglePresentation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.presentation.update(cx, |state, cx| {
            state.mode.toggle();
            cx.notify();
        });
        let open = docks_open(self.presentation.read(cx).mode);
        if let Some(dock_area) = self.dock_area.as_ref().and_then(WeakEntity::upgrade) {
            dock_area.update(cx, |area, cx| {
                let docks: Vec<_> = [area.left_dock(), area.right_dock()]
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect();
                for dock in docks {
                    dock.update(cx, |dock, cx| dock.set_open(open, window, cx));
                }
            });
        }
        cx.notify();
    }
}

impl EventEmitter<PanelEvent> for CanvasPanel {}

impl Focusable for CanvasPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for CanvasPanel {
    fn panel_name(&self) -> &'static str {
        CANVAS_PANEL_NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.title.clone()
    }

    fn closable(&self, _cx: &App) -> bool {
        false
    }

    fn zoomable(&self, _cx: &App) -> Option<PanelControl> {
        None
    }

    fn visible(&self, cx: &App) -> bool {
        panel_visible(self.presentation.read(cx).mode, PanelRole::Canvas)
    }
}

impl Render for CanvasPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The 0016 white canvas surface + flex centring, now inside the
        // panel (the dock owns the window background). The mouse-down
        // listener ONLY claims focus for the `p` binding — it does not
        // stop propagation, so every chart element handler below sees the
        // exact events it always did.
        div()
            .size_full()
            .bg(rgb(0xffffff))
            .key_context(WORKSPACE_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::toggle_presentation))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, window, cx| {
                    window.focus(&this.focus_handle, cx);
                }),
            )
            .flex()
            .items_center()
            .justify_center()
            .child(self.chart_view.clone())
    }
}

// ---------------------------------------------------------------------------
// Spec editor panel (aws_ac04)
// ---------------------------------------------------------------------------

/// The right-dock YAML editor: a tree-sitter code editor seeded from the
/// spec file. Save-driven, not change-driven — cmd-s writes the buffer
/// atomically to the spec path and the EXISTING mtime watcher re-renders;
/// nothing here touches the reload machinery. Every save routes through
/// the framework-free `spec_save::decide_save` guard: an external change
/// on disk warns instead of being silently overwritten (a second
/// consecutive cmd-s forces the write), and an editor whose boot seed
/// failed refuses to save at all.
pub struct EditorPanel {
    /// The code-editor state (their entity; `value()` is the buffer).
    state: Entity<InputState>,
    /// The spec file cmd-s writes to (the same path the watcher polls).
    spec_path: PathBuf,
    /// Tab title: the spec's file name.
    tab_title: SharedString,
    /// Shared presentation state (visibility mapping input).
    presentation: Entity<PresentationState>,
    /// The feedback log: every save outcome that surfaces as a workspace
    /// notification is also appended here (wsc_ac02 — the Log panel is the
    /// toasts' persistent sibling).
    log: Entity<FeedbackLog>,
    /// The file text the buffer was last synced with (the boot seed, a
    /// successful save, or a pristine reseed). `None` = the boot read
    /// failed and no reseed has landed — save refuses (truncation guard).
    last_synced: Option<String>,
    /// A conflict warning was issued and not yet resolved: the NEXT cmd-s
    /// overwrites the external change (two-step confirm).
    conflict_pending: bool,
}

impl EditorPanel {
    /// Build the editor over `spec_path`, seeded with `seed` (the file's
    /// contents at boot; `None` when the boot read failed — the editor
    /// opens empty and refuses to save until a reseed lands).
    pub fn new(
        spec_path: PathBuf,
        seed: Option<&str>,
        presentation: Entity<PresentationState>,
        log: Entity<FeedbackLog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor("yaml")
                .line_number(true)
                .default_value(seed.unwrap_or_default().to_string())
        });
        let tab_title = spec_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "spec".to_string());
        Self {
            state,
            spec_path,
            tab_title: tab_title.into(),
            presentation,
            log,
            last_synced: seed.map(str::to_string),
            conflict_pending: false,
        }
    }

    /// Append a save outcome to the feedback log with the SAME severity +
    /// message pair the workspace notification carries (wsc_ac02).
    fn log_feedback(&self, severity: Severity, message: &str, cx: &mut Context<Self>) {
        self.log.update(cx, |log, cx| {
            log.append(severity, message);
            cx.notify();
        });
    }

    /// `SaveSpec` handler (cmd-s, editor context): `decide_save` first,
    /// then the pure atomic write. Success is quiet — the watcher's
    /// re-render (or a rejection notification, aws_ac05) is the feedback.
    /// A conflict or refusal surfaces as a workspace notification; a
    /// filesystem failure surfaces immediately.
    fn save(&mut self, _: &SaveSpec, window: &mut Window, cx: &mut Context<Self>) {
        let buffer = self.state.read(cx).value();
        let file_now = std::fs::read_to_string(&self.spec_path).ok();
        let decision = if self.conflict_pending {
            // The previous cmd-s warned about this conflict: the author
            // saved again, so the buffer wins.
            spec_save::SaveDecision::Write
        } else {
            spec_save::decide_save(buffer.as_ref(), file_now.as_deref(), self.last_synced.as_deref())
        };
        self.conflict_pending = false;
        match decision {
            spec_save::SaveDecision::Unchanged => {
                // Buffer == file: nothing to write, and the two are in
                // sync by definition.
                self.last_synced = Some(buffer.to_string());
            }
            spec_save::SaveDecision::RefuseUnseeded => {
                let message = format!(
                    "Save refused: {} could not be read when the editor opened — \
                     saving would overwrite contents the editor never held",
                    self.spec_path.display()
                );
                eprintln!("{message}");
                self.log_feedback(Severity::Error, &message, cx);
                Root::update(window, cx, |root, window, cx| {
                    root.push_notification(Notification::error(message.clone()), window, cx);
                });
            }
            spec_save::SaveDecision::ExternalConflict => {
                self.conflict_pending = true;
                let message =
                    "Spec changed on disk since it was loaded — save again to overwrite".to_string();
                eprintln!("Save deferred: {message}");
                self.log_feedback(Severity::Warning, &message, cx);
                Root::update(window, cx, |root, window, cx| {
                    root.push_notification(Notification::warning(message.clone()), window, cx);
                });
            }
            spec_save::SaveDecision::Write => {
                match spec_save::save_spec_atomic(buffer.as_ref(), &self.spec_path) {
                    Ok(_) => {
                        self.last_synced = Some(buffer.to_string());
                    }
                    Err(e) => {
                        let message = format!("Save failed: {e}");
                        eprintln!("{message}");
                        self.log_feedback(Severity::Error, &message, cx);
                        Root::update(window, cx, |root, window, cx| {
                            root.push_notification(Notification::error(message.clone()), window, cx);
                        });
                    }
                }
            }
        }
    }

    /// Adopt `contents` from disk when the buffer is pristine — the
    /// watcher taps this on every observed mtime change, so an external
    /// edit refreshes an untouched editor instead of arming the conflict
    /// guard. A dirty buffer is left alone (cmd-s then routes through the
    /// conflict path); our own save's echo is a no-op. The decision is
    /// `spec_save::should_reseed`, framework-free.
    pub fn reseed_from_disk(&mut self, contents: &str, window: &mut Window, cx: &mut Context<Self>) {
        let buffer = self.state.read(cx).value();
        if !spec_save::should_reseed(buffer.as_ref(), self.last_synced.as_deref(), contents) {
            return;
        }
        self.state
            .update(cx, |state, cx| state.set_value(contents.to_string(), window, cx));
        self.last_synced = Some(contents.to_string());
        self.conflict_pending = false;
    }
}

impl EventEmitter<PanelEvent> for EditorPanel {}

impl Focusable for EditorPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // The editor IS the panel: focusing the panel focuses the buffer.
        self.state.focus_handle(cx)
    }
}

impl Panel for EditorPanel {
    fn panel_name(&self) -> &'static str {
        EDITOR_PANEL_NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.tab_title.clone()
    }

    fn closable(&self, _cx: &App) -> bool {
        // v1 has no reopen affordance, so no panel may close.
        false
    }

    fn zoomable(&self, _cx: &App) -> Option<PanelControl> {
        None
    }

    fn visible(&self, cx: &App) -> bool {
        panel_visible(self.presentation.read(cx).mode, PanelRole::Editor)
    }

    /// Dock-placement moves (wsc_ac07): the menu is the bootstrap gesture —
    /// a dock's only panel can never start a drag at this pin, so without
    /// these items the editor could never reach the bottom dock. Both items
    /// always show (the panel cannot cheaply know its current dock); the
    /// handler is idempotent — re-landing where the panel already lives.
    fn dropdown_menu(
        &mut self,
        this: PopupMenu,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> PopupMenu {
        this.menu("Dock at Bottom", Box::new(DockEditorAtBottom))
            .menu("Dock at Right", Box::new(DockEditorAtRight))
    }
}

impl Render for EditorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .key_context(EDITOR_KEY_CONTEXT)
            .on_action(cx.listener(Self::save))
            .child(
                Input::new(&self.state)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .size_full(),
            )
    }
}

// ---------------------------------------------------------------------------
// Sidebar panel (aws_ac06)
// ---------------------------------------------------------------------------

/// The left-dock data sidebar skeleton: sources + column names, derived
/// headlessly by `sidebar_model` before the window opened. Display-only.
pub struct SidebarPanel {
    /// The derived listings (one per `data:` source, declaration order).
    listings: Vec<SourceListing>,
    /// Shared presentation state (visibility mapping input).
    presentation: Entity<PresentationState>,
    focus_handle: FocusHandle,
}

impl SidebarPanel {
    /// Host the derived `listings`.
    pub fn new(
        listings: Vec<SourceListing>,
        presentation: Entity<PresentationState>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            listings,
            presentation,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The hosted listings (shim assertion surface, aws_ac06).
    #[cfg(test)]
    pub fn listings(&self) -> &[SourceListing] {
        &self.listings
    }
}

impl EventEmitter<PanelEvent> for SidebarPanel {}

impl Focusable for SidebarPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for SidebarPanel {
    fn panel_name(&self) -> &'static str {
        SIDEBAR_PANEL_NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Data")
    }

    fn closable(&self, _cx: &App) -> bool {
        false
    }

    fn zoomable(&self, _cx: &App) -> Option<PanelControl> {
        None
    }

    fn visible(&self, cx: &App) -> bool {
        panel_visible(self.presentation.read(cx).mode, PanelRole::Sidebar)
    }

    /// Dock-placement moves (wsc_ac07) — see [`EditorPanel::dropdown_menu`];
    /// the sidebar's return move is its home left dock.
    fn dropdown_menu(
        &mut self,
        this: PopupMenu,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> PopupMenu {
        this.menu("Dock at Bottom", Box::new(DockSidebarAtBottom))
            .menu("Dock at Left", Box::new(DockSidebarAtLeft))
    }
}

impl Render for SidebarPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        div()
            .size_full()
            .p_3()
            .text_size(px(12.0))
            .children(self.listings.iter().map(|source| {
                let mut block = div()
                    .mb_3()
                    .child(div().text_color(foreground).child(SharedString::from(source.name.clone())));
                if source.columns.is_empty() {
                    block = block.child(
                        div()
                            .pl_2()
                            .text_color(muted)
                            .child(SharedString::from("(no columns known)")),
                    );
                } else {
                    block = block.children(source.columns.iter().map(|column| {
                        div()
                            .pl_2()
                            .text_color(muted)
                            .child(SharedString::from(column.clone()))
                    }));
                }
                block
            }))
    }
}

// ---------------------------------------------------------------------------
// Log panel (wsc_ac02)
// ---------------------------------------------------------------------------

/// The bottom-dock feedback log: renders the framework-free [`FeedbackLog`]
/// as simple text rows, newest at top. Permanent (closable=false) in v1 —
/// it anchors the bottom dock: an emptied dock lingers as a dead strip, and
/// a panel moved into an otherwise-empty dock would become its last panel
/// and stop being draggable. Revisit when a second bottom-dock citizen
/// exists.
pub struct LogPanel {
    /// The shared feedback history (EditorPanel saves and the reload
    /// watcher both append to it).
    log: Entity<FeedbackLog>,
    /// Shared presentation state (visibility mapping input).
    presentation: Entity<PresentationState>,
    focus_handle: FocusHandle,
}

impl LogPanel {
    /// Host the shared feedback `log`.
    pub fn new(
        log: Entity<FeedbackLog>,
        presentation: Entity<PresentationState>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            log,
            presentation,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The hosted log (shim assertion surface, wsc_ac02).
    #[cfg(test)]
    pub fn log(&self) -> &Entity<FeedbackLog> {
        &self.log
    }
}

impl EventEmitter<PanelEvent> for LogPanel {}

impl Focusable for LogPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for LogPanel {
    fn panel_name(&self) -> &'static str {
        LOG_PANEL_NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Log")
    }

    fn closable(&self, _cx: &App) -> bool {
        // v1: the permanent tab is the dock's anchor (see the type doc).
        false
    }

    fn zoomable(&self, _cx: &App) -> Option<PanelControl> {
        None
    }

    fn visible(&self, cx: &App) -> bool {
        panel_visible(self.presentation.read(cx).mode, PanelRole::Log)
    }
}

impl Render for LogPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Minimal by design (wsc_ac02): text rows, newest at top, no scroll
        // machinery. Severity is the row's colour cue.
        let danger = cx.theme().danger;
        let warning = cx.theme().warning;
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let entries = self.log.read(cx).entries().to_vec();
        let list = div().size_full().p_3().text_size(px(12.0)).overflow_hidden();
        if entries.is_empty() {
            return list.child(
                div()
                    .text_color(muted)
                    .child(SharedString::from("(no reload or save feedback yet)")),
            );
        }
        list.children(entries.into_iter().map(move |entry| {
            let (tag, tag_color) = match entry.severity {
                Severity::Error => ("error", danger),
                Severity::Warning => ("warning", warning),
            };
            div()
                .flex()
                .gap_2()
                .child(div().text_color(tag_color).child(SharedString::from(tag)))
                .child(
                    div()
                        .text_color(foreground)
                        .child(SharedString::from(entry.message)),
                )
        }))
    }
}

// ---------------------------------------------------------------------------
// Workspace root (aws_ac03)
// ---------------------------------------------------------------------------

/// The bottom dock's rebuild state, stashed while presentation mode has the
/// dock removed (wsc_ac04): the dumped panel tree (their save format — the
/// registered factories resolve it back to live entities), the dock height,
/// and the open bit. Everything `set_bottom_dock` needs to rebuild the dock
/// exactly as the author left it — closed-before stays closed, open-before
/// stays open, moved-in panels come back.
struct BottomDockStash {
    /// The dock's panel tree, dumped via the public `PanelView::dump` path.
    panel: PanelState,
    /// The dock's height (its open size; the closed strip doesn't change it).
    size: Pixels,
    /// Whether the dock was open when presentation was entered.
    open: bool,
}

/// The window root under gpui-component's `Root`: hosts the `DockArea`
/// (center canvas + right editor + left sidebar + bottom log) and owns
/// layout persistence — versioned JSON in the user config dir, saved
/// debounced on `LayoutChanged` and flushed on quit, canvas excluded, every
/// fallback decided by the framework-free `dock_state_file` module.
pub struct WorkspaceRoot {
    dock_area: Entity<DockArea>,
    presentation: Entity<PresentationState>,
    /// The movable panels (wsc_ac07): the menu-move action handlers resolve
    /// a dispatched action to the entity to move. The canvas is the fixed
    /// centre and the Log panel anchors the bottom dock — neither moves.
    editor_panel: Entity<EditorPanel>,
    sidebar_panel: Entity<SidebarPanel>,
    /// Layout file location (`None` = no config dir; persistence off).
    state_path: Option<PathBuf>,
    /// The framework-free save policy (debounce + quit-flush + skip-if-
    /// unchanged); this view supplies the clock and executes the actions.
    policy: SavePolicy,
    /// Millisecond clock origin for the policy.
    boot: Instant,
    /// The bottom dock's rebuild state while presentation has it removed
    /// (`None` whenever the dock is present).
    bottom_stash: Option<BottomDockStash>,
    /// The pending debounced save, if any (dropped saves are superseded —
    /// latest change wins, matching the policy's deadline).
    _save_task: Option<Task<()>>,
}

impl WorkspaceRoot {
    /// Assemble the dock over the four panels, restoring the saved layout
    /// when usable (missing/corrupt/version-mismatch → default), and wire
    /// the save triggers.
    pub fn new(
        canvas: Entity<CanvasPanel>,
        editor: Entity<EditorPanel>,
        sidebar: Entity<SidebarPanel>,
        log: Entity<LogPanel>,
        presentation: Entity<PresentationState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state_path = dock_state_file::dock_state_path(
            std::env::var("BRIGHTFIELD_CONFIG_DIR").ok().as_deref(),
            std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
            std::env::var("HOME").ok().as_deref(),
        );
        let raw = state_path
            .as_deref()
            .and_then(dock_state_file::read_state_file);
        Self::with_saved_layout(
            canvas, editor, sidebar, log, presentation, state_path, raw, window, cx,
        )
    }

    /// [`WorkspaceRoot::new`] with the persistence inputs injected: the
    /// layout file location (`None` = persistence off) and the raw saved
    /// layout JSON (`None` = fresh boot). `new` supplies the real env/file;
    /// the wsc_ac03/ac04 tests supply fixtures.
    #[allow(clippy::too_many_arguments)]
    fn with_saved_layout(
        canvas: Entity<CanvasPanel>,
        editor: Entity<EditorPanel>,
        sidebar: Entity<SidebarPanel>,
        log: Entity<LogPanel>,
        presentation: Entity<PresentationState>,
        state_path: Option<PathBuf>,
        raw: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let dock_area =
            cx.new(|cx| DockArea::new(DOCK_AREA_ID, Some(DOCK_STATE_VERSION), window, cx));
        canvas.update(cx, |canvas, _| canvas.set_dock_area(dock_area.downgrade()));

        // Register the factories the layout loader resolves panel names
        // against. Each returns the entity built THIS boot from live
        // pipeline state — the serialised payload is ignored, which is what
        // "the canvas is never persisted" means at load time (the payload
        // was stripped at save time too).
        register_panel(cx, CANVAS_PANEL_NAME, {
            let canvas = canvas.clone();
            move |_, _, _, _, _| Box::new(canvas.clone())
        });
        register_panel(cx, EDITOR_PANEL_NAME, {
            let editor = editor.clone();
            move |_, _, _, _, _| Box::new(editor.clone())
        });
        register_panel(cx, SIDEBAR_PANEL_NAME, {
            let sidebar = sidebar.clone();
            move |_, _, _, _, _| Box::new(sidebar.clone())
        });
        register_panel(cx, LOG_PANEL_NAME, {
            let log = log.clone();
            move |_, _, _, _, _| Box::new(log.clone())
        });

        // Restore the saved arrangement, or build the default layout. Every
        // "is this state usable?" decision is dock_state_file's; a restore
        // that fails INSIDE the dock (their loader) falls back the same way.
        let restored = match dock_state_file::decide_load(raw.as_deref(), DOCK_STATE_VERSION) {
            LoadDecision::Restore(value) => match serde_json::from_value::<DockAreaState>(value) {
                Ok(state) => {
                    let loaded = dock_area.update(cx, |area, cx| area.load(state, window, cx));
                    if let Err(e) = &loaded {
                        eprintln!("dock layout: failed to restore saved layout ({e}); using default");
                    }
                    loaded.is_ok()
                }
                Err(e) => {
                    eprintln!("dock layout: saved layout does not deserialise ({e}); using default");
                    false
                }
            },
            LoadDecision::Default(reason) => {
                if reason != "no saved layout" {
                    eprintln!("dock layout: {reason}; using default");
                }
                false
            }
        };
        if !restored {
            Self::default_layout(&dock_area, &canvas, &editor, &sidebar, &log, window, cx);
        } else {
            // Normalise (wsc_ac03 correction, review F1): pre-round saves
            // serialised bare-Tabs dock roots, which this pin's drag
            // machinery treats as locked — re-root them under StackPanels,
            // preserving the author's arrangement.
            Self::normalise_dock_roots(&dock_area, window, cx);
            if bottom_dock_needs_backfill(dock_area.read(cx).has_dock(DockPlacement::Bottom)) {
                // Backfill (wsc_ac03): every pre-round saved layout lacks a
                // bottom dock — append the same closed Log dock the default
                // layout seeds, without touching the restored arrangement.
                // DOCK_STATE_VERSION is unchanged on purpose: a version bump
                // would discard the author's layout to add one dock.
                Self::seed_bottom_dock(&dock_area, &log, window, cx);
            }
        }

        // Debounced save on layout changes…
        cx.subscribe_in(
            &dock_area,
            window,
            |this: &mut Self, _, event: &DockEvent, window, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    this.schedule_save(window, cx);
                }
            },
        )
        .detach();

        // `LayoutChanged` alone has a blind spot in their tree: `Dock::resize`
        // ends in a bare notify (no DockEvent), so dock widths would
        // otherwise persist only via the quit flush, and a crash would lose
        // them. Observe the dock entities and the center's root view
        // directly, funnelling into the same debounced policy:
        // skip-if-unchanged + debounce absorb the notify-storm a drag
        // produces. (The historical bare-Tabs blind spot — their
        // `subscribe_item` skipping tab-only items — is healed by
        // stack-rooting every dock item (review F1/F2); these direct
        // observers stay as belt-and-braces for the resize case.)
        let (edge_docks, center_tabs, center_stack) = {
            let area = dock_area.read(cx);
            let edge_docks: Vec<_> = [area.left_dock(), area.right_dock(), area.bottom_dock()]
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            let (center_tabs, center_stack) = match area.center() {
                DockItem::Tabs { view, .. } => (Some(view.clone()), None),
                DockItem::Split { view, .. } => (None, Some(view.clone())),
                _ => (None, None),
            };
            (edge_docks, center_tabs, center_stack)
        };
        for dock in edge_docks {
            Self::observe_dock_for_saves(&dock, window, cx);
        }
        if let Some(tabs) = center_tabs {
            cx.observe_in(&tabs, window, |this: &mut Self, _, window, cx| {
                this.schedule_save(window, cx);
            })
            .detach();
        }
        if let Some(stack) = center_stack {
            cx.observe_in(&stack, window, |this: &mut Self, _, window, cx| {
                this.schedule_save(window, cx);
            })
            .detach();
        }

        // The presentation round trip for the BOTTOM dock (wsc_ac04): the
        // canvas's `p` handler flips the shared mode and collapses the
        // left/right rails; this observer executes the framework-free
        // bottom-dock action — remove entirely on enter (a closed bottom
        // dock still paints a 29px strip), rebuild from the stash on exit.
        cx.observe_in(&presentation, window, |this: &mut Self, _, window, cx| {
            this.sync_bottom_dock_to_mode(window, cx);
        })
        .detach();

        // …and a flush on quit (pending debounce or not).
        cx.on_app_quit(|this: &mut Self, cx| {
            let action = if layout_persistable(this.presentation.read(cx).mode) {
                let json = this.serialised_state(cx);
                this.policy.quit(&json)
            } else {
                SaveAction::Nothing
            };
            let path = this.state_path.clone();
            async move {
                if let (SaveAction::Write(json), Some(path)) = (action, path) {
                    if let Err(e) = dock_state_file::write_state_file(&path, &json) {
                        eprintln!("dock layout: quit-flush save failed: {e}");
                    }
                }
            }
        })
        .detach();

        Self {
            dock_area,
            presentation,
            editor_panel: editor,
            sidebar_panel: sidebar,
            state_path,
            policy: SavePolicy::default(),
            boot: Instant::now(),
            bottom_stash: None,
            _save_task: None,
        }
    }

    /// A stack-rooted single-panel dock item (wsc_ac03 correction, review
    /// F1): at pin b7e63cc2 the entire drag machinery gates on TabPanels
    /// having a StackPanel parent — their `is_locked` returns true for a
    /// bare-Tabs root, killing droppable AND draggable, so a bare
    /// `DockItem::tab` dock renders no drop targets and can never source a
    /// drag. Upstream's own dock example v_split-wraps every dock item;
    /// this mirrors it.
    fn stack_rooted_tab<P: Panel>(
        panel: &Entity<P>,
        weak: &WeakEntity<DockArea>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> DockItem {
        let tab = DockItem::tab(panel.clone(), weak, window, cx);
        DockItem::v_split(vec![tab], weak, window, cx)
    }

    /// Center canvas + left sidebar + right editor at their default sizes,
    /// plus the closed bottom Log dock (wsc_ac03). Every item is
    /// stack-rooted (see [`Self::stack_rooted_tab`]).
    fn default_layout(
        dock_area: &Entity<DockArea>,
        canvas: &Entity<CanvasPanel>,
        editor: &Entity<EditorPanel>,
        sidebar: &Entity<SidebarPanel>,
        log: &Entity<LogPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = dock_area.downgrade();
        let center = Self::stack_rooted_tab(canvas, &weak, window, cx);
        let left = Self::stack_rooted_tab(sidebar, &weak, window, cx);
        let right = Self::stack_rooted_tab(editor, &weak, window, cx);
        dock_area.update(cx, |area, cx| {
            area.set_center(center, window, cx);
            area.set_left_dock(left, Some(px(SIDEBAR_DOCK_WIDTH as f32)), true, window, cx);
            area.set_right_dock(right, Some(px(EDITOR_DOCK_WIDTH as f32)), true, window, cx);
        });
        Self::seed_bottom_dock(dock_area, log, window, cx);
    }

    /// Seed the bottom dock CLOSED with the Log panel: the 29px strip is
    /// the drop/expand affordance (their drag UI only lands on existing
    /// dock surfaces — seeding is the only way drag-to-bottom can work),
    /// and closed doesn't re-carve the author's current layout. Stack-rooted
    /// so the strip's TabPanel is a real drop target (review F1).
    fn seed_bottom_dock(
        dock_area: &Entity<DockArea>,
        log: &Entity<LogPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = dock_area.downgrade();
        let bottom = Self::stack_rooted_tab(log, &weak, window, cx);
        dock_area.update(cx, |area, cx| {
            area.set_bottom_dock(bottom, Some(px(BOTTOM_DOCK_HEIGHT as f32)), false, window, cx);
        });
    }

    /// Re-root any restored dock item whose root is bare `Tabs` under a
    /// StackPanel (wsc_ac03 correction, review F1): every pre-round save
    /// serialised bare-Tabs roots, which this pin's drag machinery treats
    /// as locked (no drop targets, no drag sources) and whose panel events
    /// their `subscribe_item` never wires into the save chain (review F2).
    /// Size, open bit, and panel placement are preserved — "restores as
    /// saved" means the author's arrangement, not the serialised tree
    /// shape. The first save after a normalising boot legitimately writes
    /// the new shape. Runs BEFORE the observer wiring, so the observers
    /// attach to the re-set Dock entities.
    fn normalise_dock_roots(
        dock_area: &Entity<DockArea>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = dock_area.downgrade();

        // Center: wrapping also brings it under their subscribe_item save
        // coverage (Split arms are subscribed; bare Tabs are skipped).
        if matches!(dock_area.read(cx).center(), DockItem::Tabs { .. }) {
            let tabs = dock_area.read(cx).center().clone();
            let item = DockItem::v_split(vec![tabs], &weak, window, cx);
            dock_area.update(cx, |area, cx| area.set_center(item, window, cx));
        }

        for placement in [DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom] {
            let dock = {
                let area = dock_area.read(cx);
                match placement {
                    DockPlacement::Left => area.left_dock().cloned(),
                    DockPlacement::Right => area.right_dock().cloned(),
                    DockPlacement::Bottom => area.bottom_dock().cloned(),
                    DockPlacement::Center => None,
                }
            };
            let Some(dock) = dock else { continue };
            let (bare, size, open, tabs) = {
                let dock = dock.read(cx);
                (
                    matches!(dock.panel(), DockItem::Tabs { .. }),
                    dock.size(),
                    dock.is_open(),
                    dock.panel().clone(),
                )
            };
            if !bare {
                continue;
            }
            let item = DockItem::v_split(vec![tabs], &weak, window, cx);
            dock_area.update(cx, |area, cx| match placement {
                DockPlacement::Left => area.set_left_dock(item, Some(size), open, window, cx),
                DockPlacement::Right => area.set_right_dock(item, Some(size), open, window, cx),
                DockPlacement::Bottom => area.set_bottom_dock(item, Some(size), open, window, cx),
                DockPlacement::Center => {}
            });
        }
    }

    /// Funnel a dock entity's bare notifies (resize et al.) into the
    /// debounced save policy. Extracted because presentation-exit rebuilds
    /// the bottom Dock ENTITY (`set_bottom_dock` creates a new one), and
    /// the rebuilt dock must be re-observed — the spec's sanctioned churn.
    fn observe_dock_for_saves(dock: &Entity<Dock>, window: &mut Window, cx: &mut Context<Self>) {
        cx.observe_in(dock, window, |this: &mut Self, _, window, cx| {
            this.schedule_save(window, cx);
        })
        .detach();
    }

    /// Execute the framework-free bottom-dock action for the current mode
    /// (wsc_ac04): presentation removes the dock after stashing its rebuild
    /// state; authoring rebuilds it from the stash — contents, size, and
    /// open bit exactly as the author left them — and re-attaches the save
    /// observer to the new Dock entity.
    fn sync_bottom_dock_to_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match bottom_dock_action(self.presentation.read(cx).mode) {
            BottomDockAction::Remove => {
                let Some(dock) = self.dock_area.read(cx).bottom_dock().cloned() else {
                    return; // Already removed (repeated notify) — stash stands.
                };
                let stash = {
                    let dock = dock.read(cx);
                    BottomDockStash {
                        panel: dock.panel().view().dump(cx),
                        size: dock.size(),
                        open: dock.is_open(),
                    }
                };
                self.bottom_stash = Some(stash);
                self.dock_area.update(cx, |area, cx| {
                    area.remove_bottom_dock(window, cx);
                    cx.notify();
                });
            }
            BottomDockAction::Rebuild => {
                if self.dock_area.read(cx).has_dock(DockPlacement::Bottom) {
                    return; // Present already (boot, repeated notify).
                }
                let Some(stash) = self.bottom_stash.take() else {
                    return; // Nothing stashed — only Remove ever removes it.
                };
                let weak = self.dock_area.downgrade();
                let item = stash.panel.to_item(weak.clone(), window, cx);
                // The stash dumps the dock's stack ROOT (a StackPanel
                // PanelState), so to_item's Stack arm rebuilds it
                // stack-rooted. Belt-and-braces (review F1c): should the
                // rebuild ever collapse to bare Tabs, re-wrap it — a
                // bare-Tabs dock is locked at this pin.
                let item = match item {
                    DockItem::Tabs { .. } => {
                        DockItem::v_split(vec![item], &weak, window, cx)
                    }
                    other => other,
                };
                self.dock_area.update(cx, |area, cx| {
                    area.set_bottom_dock(item, Some(stash.size), stash.open, window, cx);
                    cx.notify();
                });
                if let Some(dock) = self.dock_area.read(cx).bottom_dock().cloned() {
                    Self::observe_dock_for_saves(&dock, window, cx);
                }
            }
        }
    }

    /// The dock entity at `placement`, if the area has one.
    fn dock_entity(&self, placement: DockPlacement, cx: &App) -> Option<Entity<Dock>> {
        let area = self.dock_area.read(cx);
        match placement {
            DockPlacement::Left => area.left_dock().cloned(),
            DockPlacement::Right => area.right_dock().cloned(),
            DockPlacement::Bottom => area.bottom_dock().cloned(),
            DockPlacement::Center => None,
        }
    }

    /// Leaf panels in a DUMPED panel tree. The dump reads the live
    /// TabPanel/StackPanel state — their `DockItem` items vecs are
    /// construction-time snapshots that go stale once panels move (an
    /// emptied TabPanel removes itself only from the live StackPanel).
    /// Panels only ever live inside TabPanels in a dock tree, so the count
    /// is the tab count of every Tabs node. (An EMPTIED StackPanel dumps
    /// with a default `Panel(Null)` info — a childless-leaf heuristic
    /// would miscount it as one panel.)
    fn dumped_panel_count(state: &PanelState) -> usize {
        match state.info {
            gpui_component::dock::PanelInfo::Tabs { .. } => state.children.len(),
            _ => state.children.iter().map(Self::dumped_panel_count).sum(),
        }
    }

    /// Live panel count of a dock (via the dump; see `dumped_panel_count`).
    /// All our panels report `visible()` in authoring mode, the only mode
    /// the move menus are reachable in, so leaf count == visible count.
    fn dock_panel_count(dock: &Entity<Dock>, cx: &App) -> usize {
        Self::dumped_panel_count(&dock.read(cx).panel().view().dump(cx))
    }

    /// Move `panel` to the dock at `destination` (wsc_ac07 — the menu-move
    /// bootstrap): at this pin their drag UI cannot reach a single-panel
    /// dock's tab (`is_last_panel` blocks the drag source), so the tab
    /// menu drives the same public machinery a drop would land on. The
    /// handler is idempotent: moving a panel to the dock it already
    /// occupies re-lands it there.
    ///
    /// Steps: detach from every dock; land at the destination (opened — a
    /// panel moved behind a closed dock's strip would land invisibly);
    /// close any other edge dock the move emptied (pure decision:
    /// `dock_closes_when_emptied` — an emptied stack renders a hollow
    /// area).
    pub fn move_panel_to_dock(
        &mut self,
        panel: Arc<dyn PanelView>,
        destination: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock_area.update(cx, |area, cx| {
            area.remove_panel_from_all_docks(panel.clone(), window, cx);
        });

        match self.dock_entity(destination, cx) {
            Some(dock) if Self::dock_panel_count(&dock, cx) > 0 => {
                // A live TabPanel exists: join it as a tab (their add
                // path; our docks host exactly one TabPanel, so the
                // Split snapshot's first Tabs is the live one).
                dock.update(cx, |dock, cx| {
                    dock.add_panel(panel.clone(), window, cx);
                    dock.set_open(true, window, cx);
                });
            }
            Some(dock) => {
                // The destination was emptied by an earlier move: its
                // live stack has no TabPanel, and their
                // `DockItem::add_panel` would land the panel in the
                // DETACHED TabPanel still in the stale Split snapshot.
                // Rebuild the dock stack-rooted at its current size, and
                // re-attach the save observer to the new Dock entity.
                let size = dock.read(cx).size();
                Self::set_edge_dock(
                    &self.dock_area,
                    destination,
                    panel.clone(),
                    Some(size),
                    window,
                    cx,
                );
            }
            None => {
                // No dock at the destination (not reachable from our
                // shell's invariants, but total): create it stack-rooted.
                Self::set_edge_dock(&self.dock_area, destination, panel.clone(), None, window, cx);
            }
        }

        for placement in [DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom] {
            if placement == destination {
                continue;
            }
            let Some(dock) = self.dock_entity(placement, cx) else {
                continue;
            };
            if dock_closes_when_emptied(Self::dock_panel_count(&dock, cx)) {
                dock.update(cx, |dock, cx| dock.set_open(false, window, cx));
            }
        }
        cx.notify();
    }

    /// (Re)build the edge dock at `placement` as a stack-rooted single-tab
    /// dock holding `panel`, opened. `set_*_dock` creates a NEW Dock
    /// entity, so the save observer is re-attached (the same sanctioned
    /// churn as the presentation rebuild).
    fn set_edge_dock(
        dock_area: &Entity<DockArea>,
        placement: DockPlacement,
        panel: Arc<dyn PanelView>,
        size: Option<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = dock_area.downgrade();
        let tab = DockItem::tabs(vec![panel], &weak, window, cx);
        let item = DockItem::v_split(vec![tab], &weak, window, cx);
        dock_area.update(cx, |area, cx| match placement {
            DockPlacement::Left => area.set_left_dock(item, size, true, window, cx),
            DockPlacement::Right => area.set_right_dock(item, size, true, window, cx),
            DockPlacement::Bottom => area.set_bottom_dock(item, size, true, window, cx),
            DockPlacement::Center => {}
        });
        let dock = {
            let area = dock_area.read(cx);
            match placement {
                DockPlacement::Left => area.left_dock().cloned(),
                DockPlacement::Right => area.right_dock().cloned(),
                DockPlacement::Bottom => area.bottom_dock().cloned(),
                DockPlacement::Center => None,
            }
        };
        if let Some(dock) = dock {
            Self::observe_dock_for_saves(&dock, window, cx);
        }
    }

    /// wsc_ac07 action handlers — registered on the render root, which is
    /// an ancestor of every dispatch path in the window (the tab menus
    /// dispatch from wherever focus sits when the item is clicked).
    fn on_dock_editor_at_bottom(
        &mut self,
        _: &DockEditorAtBottom,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel: Arc<dyn PanelView> = Arc::new(self.editor_panel.clone());
        self.move_panel_to_dock(panel, DockPlacement::Bottom, window, cx);
    }

    fn on_dock_editor_at_right(
        &mut self,
        _: &DockEditorAtRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel: Arc<dyn PanelView> = Arc::new(self.editor_panel.clone());
        self.move_panel_to_dock(panel, DockPlacement::Right, window, cx);
    }

    fn on_dock_sidebar_at_bottom(
        &mut self,
        _: &DockSidebarAtBottom,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel: Arc<dyn PanelView> = Arc::new(self.sidebar_panel.clone());
        self.move_panel_to_dock(panel, DockPlacement::Bottom, window, cx);
    }

    fn on_dock_sidebar_at_left(
        &mut self,
        _: &DockSidebarAtLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel: Arc<dyn PanelView> = Arc::new(self.sidebar_panel.clone());
        self.move_panel_to_dock(panel, DockPlacement::Left, window, cx);
    }

    /// The hosted dock area (assertion surface, wsc_ac03/ac04).
    #[cfg(test)]
    pub fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// Whether a debounced save is armed (assertion surface, wsc_ac04 —
    /// proves a change to the REBUILT dock entity still reaches the save
    /// policy through the re-attached observer).
    #[cfg(test)]
    pub fn save_pending(&self) -> bool {
        self.policy.pending()
    }

    /// Reset the save policy so `save_pending` isolates the next change
    /// (the rebuild itself legitimately schedules saves).
    #[cfg(test)]
    pub fn reset_save_probe(&mut self) {
        self.policy = SavePolicy::default();
    }

    /// The dock state as the persisted JSON: dumped, canvas-stripped,
    /// pretty-printed. (The policy compares these strings to skip
    /// unchanged writes.)
    fn serialised_state(&self, cx: &App) -> String {
        let state = self.dock_area.read(cx).dump(cx);
        let mut value = serde_json::to_value(&state).unwrap_or(serde_json::Value::Null);
        dock_state_file::strip_canvas_state(&mut value);
        serde_json::to_string_pretty(&value).unwrap_or_default()
    }

    /// Milliseconds since boot — the policy's clock.
    fn now_ms(&self) -> u64 {
        u64::try_from(self.boot.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// `LayoutChanged` → (re)schedule the debounced write. Presentation's
    /// own dock collapses are filtered out (framework-free decision).
    fn schedule_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !layout_persistable(self.presentation.read(cx).mode) {
            return;
        }
        let now = self.now_ms();
        self.policy.layout_changed(now);
        self._save_task = Some(cx.spawn_in(window, async move |this, window| {
            window
                .background_executor()
                .timer(Duration::from_millis(SAVE_DEBOUNCE_MS))
                .await;
            let _ = this.update_in(window, |this, _window, cx| {
                // Re-check persistability AT FIRE TIME: `p` may have been
                // pressed during the debounce window, and the serialised
                // state below would then be the presentation collapse. The
                // policy suppresses the write but keeps the deadline armed
                // (the quit flush in authoring writes pending changes).
                let persistable = layout_persistable(this.presentation.read(cx).mode);
                let json = this.serialised_state(cx);
                let now = this.now_ms();
                if let SaveAction::Write(json) = this.policy.timer_fired(now, &json, persistable) {
                    if let Some(path) = this.state_path.clone() {
                        if let Err(e) = dock_state_file::write_state_file(&path, &json) {
                            eprintln!("dock layout: save failed: {e}");
                        }
                    }
                }
            });
        }));
    }
}

impl Render for WorkspaceRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Pushing into Root's notification/dialog/sheet lists draws NOTHING
        // unless the window's root view mounts the corresponding layers as
        // trailing children (gpui-component's workspace pattern — see their
        // story dock example). Without these, every push_notification —
        // reload rejections, editor save conflicts — and the dock
        // version-mismatch dialog are silently invisible.
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);
        div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            // wsc_ac07 menu-move actions: this root div is an ancestor of
            // every dispatch path in the window, so the tab-menu actions
            // land here regardless of where focus sat at click time.
            .on_action(cx.listener(Self::on_dock_editor_at_bottom))
            .on_action(cx.listener(Self::on_dock_editor_at_right))
            .on_action(cx.listener(Self::on_dock_sidebar_at_bottom))
            .on_action(cx.listener(Self::on_dock_sidebar_at_left))
            .child(self.dock_area.clone())
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}

// ---------------------------------------------------------------------------
// Reload-rejection notifications (aws_ac05)
// ---------------------------------------------------------------------------

/// Marker id for the reload-error notification: exactly one is live at a
/// time — a new rejection replaces it, a successful reload removes it
/// (`clear_reload_error`).
struct ReloadErrorTag;

/// Route a reload rejection to the workspace notification layer (the
/// watcher's tap). The severity/message pair comes from the framework-free
/// `reload_feedback` decision; a closed window is a silent no-op. Errors
/// are sticky (`reload_feedback::sticky`) — a transient toast is missed
/// whenever the save came from an external editor — and replace rather
/// than stack across repeated bad saves. The same pair is appended to the
/// feedback `log` (wsc_ac02): the Log panel keeps the history the toasts
/// lose.
pub fn notify_reload_rejection(
    window: &gpui::WindowHandle<Root>,
    cx: &mut gpui::AsyncApp,
    severity: Severity,
    message: String,
    log: &gpui::Entity<FeedbackLog>,
) {
    let _ = window.update(cx, |root, window, cx| {
        log.update(cx, |log, cx| {
            log.append(severity, message.clone());
            cx.notify();
        });
        let note = match severity {
            Severity::Error => {
                root.remove_notification::<ReloadErrorTag>(window, cx);
                Notification::error(message.clone())
                    .id::<ReloadErrorTag>()
                    .autohide(!reload_feedback::sticky(severity))
            }
            Severity::Warning => Notification::warning(message.clone()),
        };
        root.push_notification(note, window, cx);
    });
}

/// Clear the outstanding sticky reload error after a successful reload
/// (`reload_feedback::clears_errors` decided) — the file is good again and
/// a stale error toast would misreport it. Closed window: silent no-op.
pub fn clear_reload_error(window: &gpui::WindowHandle<Root>, cx: &mut gpui::AsyncApp) {
    let _ = window.update(cx, |root, window, cx| {
        root.remove_notification::<ReloadErrorTag>(window, cx);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    /// aws_ac02 (shim): the canvas panel HOLDS the ChartView entity (same
    /// entity id — delegation, not a copy), carries the stable panel name
    /// and the resolved dashboard title, refuses closing, and stays
    /// visible in BOTH presentation modes (the canvas remains). Together
    /// with the untouched chart_view/element sources (ac-09's diff gate),
    /// that is the whole shim contract: hosting added, nothing transformed.
    #[gpui::test]
    fn aws_ac02_canvas_panel_is_a_shim_around_the_chart_view(cx: &mut TestAppContext) {
        let (chart, presentation, panel) = cx.update(|cx| {
            let chart =
                cx.new(|_| ChartView::new(320.0, 240.0, Vec::new(), Vec::new(), Vec::new()));
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            let panel = cx.new(|cx| {
                CanvasPanel::new(chart.clone(), "Flight Delays", presentation.clone(), cx)
            });
            (chart, presentation, panel)
        });

        cx.update(|cx| {
            let p = panel.read(cx);
            assert_eq!(p.panel_name(), CANVAS_PANEL_NAME, "stable serialisation name");
            assert!(!p.closable(cx), "the canvas is locked");
            assert_eq!(p.title_text().as_ref(), "Flight Delays", "resolved dashboard title");
            assert_eq!(
                p.chart_view().entity_id(),
                chart.entity_id(),
                "the panel holds the SAME ChartView entity — render delegates to it"
            );
            assert!(p.visible(cx), "visible while authoring");
        });

        cx.update(|cx| {
            presentation.update(cx, |state, _| state.mode = PresentationMode::Presentation);
        });
        cx.update(|cx| {
            assert!(
                panel.read(cx).visible(cx),
                "the canvas REMAINS under presentation (aws_ac07's mapping)"
            );
        });
    }

    /// aws_ac06 (shim): the sidebar panel hosts the derived listings
    /// verbatim and hides under presentation per the mapping.
    #[gpui::test]
    fn aws_ac06_sidebar_panel_hosts_listings_and_hides_in_presentation(cx: &mut TestAppContext) {
        let listings = vec![SourceListing {
            name: "flights".to_string(),
            columns: vec!["delay".to_string(), "distance".to_string()],
        }];
        let (presentation, panel) = cx.update(|cx| {
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            let panel =
                cx.new(|cx| SidebarPanel::new(listings.clone(), presentation.clone(), cx));
            (presentation, panel)
        });

        cx.update(|cx| {
            let p = panel.read(cx);
            assert_eq!(p.panel_name(), SIDEBAR_PANEL_NAME);
            assert_eq!(p.listings(), &listings[..], "hosts the derivation verbatim");
            assert!(p.visible(cx));
        });
        cx.update(|cx| {
            presentation.update(cx, |state, _| state.mode = PresentationMode::Presentation);
        });
        cx.update(|cx| {
            assert!(!panel.read(cx).visible(cx), "authoring panels hide");
        });
    }

    /// wsc_ac02 (shim): the Log panel carries the stable panel name, hosts
    /// the SAME FeedbackLog entity, is permanent (closable=false — it
    /// anchors the bottom dock), never zoomable, and follows the authoring
    /// chrome: visible while authoring, hidden under presentation.
    #[gpui::test]
    fn wsc_ac02_log_panel_is_a_permanent_shim_over_the_feedback_log(cx: &mut TestAppContext) {
        let (feedback_log, presentation, panel) = cx.update(|cx| {
            let feedback_log = cx.new(|_| FeedbackLog::default());
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            let panel =
                cx.new(|cx| LogPanel::new(feedback_log.clone(), presentation.clone(), cx));
            (feedback_log, presentation, panel)
        });

        cx.update(|cx| {
            let p = panel.read(cx);
            assert_eq!(p.panel_name(), LOG_PANEL_NAME, "stable serialisation name");
            assert!(!p.closable(cx), "permanent in v1 — the dock's anchor");
            assert!(p.zoomable(cx).is_none(), "never zoomable");
            assert_eq!(
                p.log().entity_id(),
                feedback_log.entity_id(),
                "the panel renders the SAME log entity the taps append to"
            );
            assert!(p.visible(cx), "visible while authoring");
        });
        cx.update(|cx| {
            presentation.update(cx, |state, _| state.mode = PresentationMode::Presentation);
        });
        cx.update(|cx| {
            assert!(!panel.read(cx).visible(cx), "hidden under presentation");
        });
    }

    /// wsc_ac02 (reload-feedback tap): driving the SAME path the watcher
    /// uses — `notify_reload_rejection` — lands the identical severity +
    /// message pair in the feedback log that the notification carried
    /// (both come from the one reload_feedback decision).
    #[gpui::test]
    fn wsc_ac02_reload_rejection_reaches_the_log_with_the_notification_message(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let (feedback_log, presentation) = cx.update(|cx| {
            let feedback_log = cx.new(|_| FeedbackLog::default());
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            (feedback_log, presentation)
        });
        let panel_log = feedback_log.clone();
        let window: gpui::WindowHandle<Root> = cx.add_window(move |window, cx| {
            let panel = cx.new(|cx| LogPanel::new(panel_log.clone(), presentation.clone(), cx));
            Root::new(panel, window, cx)
        });
        cx.run_until_parked();

        let (severity, message) = reload_feedback::reload_notification(
            &reload_feedback::ReloadOutcome::PipelineFailed("mapping values are not allowed"),
        )
        .expect("rejections surface");
        let mut async_cx = cx.to_async();
        notify_reload_rejection(&window, &mut async_cx, severity, message.clone(), &feedback_log);
        cx.run_until_parked();

        cx.update(|cx| {
            let log = feedback_log.read(cx);
            assert_eq!(log.entries().len(), 1, "one outcome, one entry");
            assert_eq!(log.entries()[0].severity, severity, "same severity");
            assert_eq!(log.entries()[0].message, message, "same message, verbatim");
        });
    }

    /// A Root window hosting an EditorPanel over `spec_path` seeded with
    /// `seed`, plus the shared feedback log — the harness for the
    /// editor-save tap tests (wsc_ac02, review F5).
    fn build_editor_shell(
        cx: &mut TestAppContext,
        spec_path: PathBuf,
        seed: Option<&str>,
    ) -> (gpui::WindowHandle<Root>, Entity<EditorPanel>, Entity<FeedbackLog>) {
        cx.update(gpui_component::init);
        let (feedback_log, presentation) = cx.update(|cx| {
            let feedback_log = cx.new(|_| FeedbackLog::default());
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            (feedback_log, presentation)
        });
        let editor_slot: std::rc::Rc<std::cell::RefCell<Option<Entity<EditorPanel>>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        let slot = editor_slot.clone();
        let log_for_editor = feedback_log.clone();
        let seed = seed.map(str::to_string);
        let window: gpui::WindowHandle<Root> = cx.add_window(move |window, cx| {
            let editor = cx.new(|cx| {
                EditorPanel::new(
                    spec_path,
                    seed.as_deref(),
                    presentation.clone(),
                    log_for_editor.clone(),
                    window,
                    cx,
                )
            });
            *slot.borrow_mut() = Some(editor.clone());
            Root::new(editor, window, cx)
        });
        cx.run_until_parked();
        let editor = editor_slot.borrow().clone().expect("editor built");
        (window, editor, feedback_log)
    }

    /// Drive cmd-s from window scope WITHOUT a lease on Root (the handler
    /// itself calls Root::update, as the real action dispatch does).
    fn drive_save(cx: &mut TestAppContext, window: gpui::WindowHandle<Root>, editor: &Entity<EditorPanel>) {
        let editor = editor.clone();
        cx.update_window(window.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| editor.save(&SaveSpec, window, cx));
        })
        .expect("drive cmd-s");
        cx.run_until_parked();
    }

    /// wsc_ac02 (editor-save tap, refusal arm): a refused save (unseeded
    /// editor) appends the notification's exact message to the log at
    /// error severity.
    #[gpui::test]
    fn wsc_ac02_editor_save_refusal_reaches_the_log(cx: &mut TestAppContext) {
        let (window, editor, feedback_log) = build_editor_shell(
            cx,
            PathBuf::from("/nonexistent/brightfield-test-spec.yaml"),
            None, // Boot read failed: saving must refuse (truncation guard).
        );
        drive_save(cx, window, &editor);

        cx.update(|cx| {
            let log = feedback_log.read(cx);
            assert_eq!(log.entries().len(), 1, "the refusal was logged");
            assert_eq!(log.entries()[0].severity, Severity::Error);
            assert!(
                log.entries()[0].message.contains("Save refused"),
                "the notification's message, verbatim: {}",
                log.entries()[0].message
            );
        });
    }

    /// wsc_ac02 (editor-save tap, conflict arm — review F5): an external
    /// change on disk under a dirty buffer defers the save with a warning,
    /// and the log receives the same warning message the toast carried.
    #[gpui::test]
    fn wsc_ac02_editor_save_conflict_reaches_the_log(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("bf-wsc-conflict-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join("spec.yaml");
        std::fs::write(&path, "original: 1\n").expect("seed file");

        let (window, editor, feedback_log) =
            build_editor_shell(cx, path.clone(), Some("original: 1\n"));

        // The author edits the buffer…
        cx.update_window(window.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| {
                editor.state.update(cx, |state, cx| {
                    state.set_value("edited: 2\n".to_string(), window, cx);
                });
            });
        })
        .expect("dirty the buffer");
        // …while the file changes externally under it.
        std::fs::write(&path, "external: 3\n").expect("external change");

        drive_save(cx, window, &editor);

        cx.update(|cx| {
            let log = feedback_log.read(cx);
            assert_eq!(log.entries().len(), 1, "the conflict was logged");
            assert_eq!(log.entries()[0].severity, Severity::Warning, "warning, like the toast");
            assert_eq!(
                log.entries()[0].message,
                "Spec changed on disk since it was loaded — save again to overwrite",
                "the toast's message, verbatim"
            );
        });
        assert_eq!(
            std::fs::read_to_string(&path).expect("file intact"),
            "external: 3\n",
            "the deferred save wrote nothing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// wsc_ac02 (editor-save tap, write-failure arm — review F5): a save
    /// whose atomic write fails (the spec path is a DIRECTORY, so the
    /// final rename cannot land) logs the same "Save failed" error the
    /// toast carried.
    #[gpui::test]
    fn wsc_ac02_editor_save_write_failure_reaches_the_log(cx: &mut TestAppContext) {
        let dir = std::env::temp_dir().join(format!("bf-wsc-writeerr-{}", std::process::id()));
        let spec_as_dir = dir.join("spec.yaml");
        std::fs::create_dir_all(&spec_as_dir).expect("dir standing where the file should be");

        // Seeded (so the save is not refused) with a buffer that differs
        // (so it is not Unchanged); the unreadable "file" routes decide_save
        // to Write, whose atomic rename then fails against the directory.
        let (window, editor, feedback_log) =
            build_editor_shell(cx, spec_as_dir.clone(), Some("original: 1\n"));
        cx.update_window(window.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| {
                editor.state.update(cx, |state, cx| {
                    state.set_value("edited: 2\n".to_string(), window, cx);
                });
            });
        })
        .expect("dirty the buffer");

        drive_save(cx, window, &editor);

        cx.update(|cx| {
            let log = feedback_log.read(cx);
            assert_eq!(log.entries().len(), 1, "the write failure was logged");
            assert_eq!(log.entries()[0].severity, Severity::Error, "error, like the toast");
            assert!(
                log.entries()[0].message.starts_with("Save failed: "),
                "the toast's message shape: {}",
                log.entries()[0].message
            );
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// wsc_ac02 (no-clear-on-recovery, review F4): recovery clears the
    /// sticky error NOTIFICATION but never the log — driven through the
    /// same notify_reload_rejection + clear_reload_error pair the
    /// watcher's rejection and recovery arms use.
    #[gpui::test]
    fn wsc_ac02_recovery_clears_the_notification_not_the_log(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (feedback_log, presentation) = cx.update(|cx| {
            let feedback_log = cx.new(|_| FeedbackLog::default());
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            (feedback_log, presentation)
        });
        let panel_log = feedback_log.clone();
        let window: gpui::WindowHandle<Root> = cx.add_window(move |window, cx| {
            let panel = cx.new(|cx| LogPanel::new(panel_log.clone(), presentation.clone(), cx));
            Root::new(panel, window, cx)
        });
        cx.run_until_parked();

        // A rejection: notification pushed (sticky error) + log entry.
        let (severity, message) = reload_feedback::reload_notification(
            &reload_feedback::ReloadOutcome::PipelineFailed("boom"),
        )
        .expect("rejections surface");
        let mut async_cx = cx.to_async();
        notify_reload_rejection(&window, &mut async_cx, severity, message.clone(), &feedback_log);
        cx.run_until_parked();

        let notifications = window
            .update(cx, |root, _, _| root.notification.clone())
            .expect("read notification list");
        cx.update(|cx| {
            assert_eq!(
                notifications.read(cx).notifications().len(),
                1,
                "the rejection raised a notification"
            );
            assert_eq!(feedback_log.read(cx).entries().len(), 1, "and a log entry");
        });

        // The recovery arm (a successful reload): main.rs consults
        // clears_errors(Applied) and calls clear_reload_error.
        assert!(reload_feedback::clears_errors(&reload_feedback::ReloadOutcome::Applied));
        let mut async_cx = cx.to_async();
        clear_reload_error(&window, &mut async_cx);
        // Their dismissal is animated: the entry leaves the list 0.15s
        // after the close — advance the test clock past it.
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();

        cx.update(|cx| {
            assert_eq!(
                notifications.read(cx).notifications().len(),
                0,
                "recovery cleared the sticky error toast"
            );
            let log = feedback_log.read(cx);
            assert_eq!(
                log.entries().len(),
                1,
                "the log is history — recovery never clears it"
            );
            assert_eq!(log.entries()[0].message, message, "entry intact, verbatim");
        });
    }

    /// aws_ac04 (binding data): the editor keymap is one binding — cmd-s →
    /// SaveSpec — scoped to the editor context, mirroring fww_ac07's shape
    /// for the workspace `p` binding.
    #[test]
    fn aws_ac04_cmd_s_binds_save_spec_in_editor_context() {
        let bindings = editor_key_bindings();
        assert_eq!(bindings.len(), 1, "one action, one binding");
        let binding = &bindings[0];

        let cmd_s = gpui::Keystroke::parse("cmd-s").expect("parses");
        assert_eq!(
            binding.match_keystrokes(std::slice::from_ref(&cmd_s)),
            Some(false),
            "cmd-s is a complete match"
        );
        let plain_s = gpui::Keystroke::parse("s").expect("parses");
        assert_eq!(binding.match_keystrokes(std::slice::from_ref(&plain_s)), None);

        assert_eq!(binding.action().name(), "brightfield::SaveSpec");
        assert!(
            binding.predicate().is_some(),
            "the binding is editor-scoped, not global"
        );
    }

    // -----------------------------------------------------------------------
    // wsc_ac03/ac04 — bottom dock seed/backfill + presentation round trip,
    // against the real WorkspaceRoot in a test window (aws_ac03 precedent).
    // -----------------------------------------------------------------------

    struct TestShell {
        window: gpui::WindowHandle<Root>,
        workspace: Entity<WorkspaceRoot>,
        presentation: Entity<PresentationState>,
        editor: Entity<EditorPanel>,
    }

    /// Assemble the full four-panel WorkspaceRoot in a test window, with the
    /// persistence inputs injected (`raw` = the saved layout JSON; state
    /// path off so no file I/O races other tests).
    fn build_shell(cx: &mut TestAppContext, raw: Option<String>) -> TestShell {
        cx.update(gpui_component::init);
        let (feedback_log, presentation) = cx.update(|cx| {
            let feedback_log = cx.new(|_| FeedbackLog::default());
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            (feedback_log, presentation)
        });
        let slots: std::rc::Rc<
            std::cell::RefCell<Option<(Entity<WorkspaceRoot>, Entity<EditorPanel>)>>,
        > = std::rc::Rc::new(std::cell::RefCell::new(None));
        let slot = slots.clone();
        let presentation_in = presentation.clone();
        let window: gpui::WindowHandle<Root> = cx.add_window(move |window, cx| {
            let chart =
                cx.new(|_| ChartView::new(320.0, 240.0, Vec::new(), Vec::new(), Vec::new()));
            let canvas =
                cx.new(|cx| CanvasPanel::new(chart, "Test", presentation_in.clone(), cx));
            let editor = cx.new(|cx| {
                EditorPanel::new(
                    PathBuf::from("/tmp/brightfield-wsc-test-spec.yaml"),
                    Some(""),
                    presentation_in.clone(),
                    feedback_log.clone(),
                    window,
                    cx,
                )
            });
            let sidebar = cx.new(|cx| SidebarPanel::new(Vec::new(), presentation_in.clone(), cx));
            let log_panel =
                cx.new(|cx| LogPanel::new(feedback_log.clone(), presentation_in.clone(), cx));
            let workspace = cx.new(|cx| {
                WorkspaceRoot::with_saved_layout(
                    canvas,
                    editor.clone(),
                    sidebar,
                    log_panel,
                    presentation_in.clone(),
                    None,
                    raw,
                    window,
                    cx,
                )
            });
            *slot.borrow_mut() = Some((workspace.clone(), editor));
            Root::new(workspace, window, cx)
        });
        cx.run_until_parked();
        let (workspace, editor) = slots.borrow().clone().expect("workspace built");
        TestShell {
            window,
            workspace,
            presentation,
            editor,
        }
    }

    /// Flip the shared presentation mode (the same entity notify the canvas
    /// `p` handler produces) and flush the observers.
    fn toggle_presentation_mode(
        cx: &mut TestAppContext,
        presentation: &Entity<PresentationState>,
    ) {
        presentation.update(cx, |state, cx| {
            state.mode.toggle();
            cx.notify();
        });
        cx.run_until_parked();
    }

    /// (has_bottom_dock, is_open, size) of the workspace's bottom dock.
    fn bottom_dock_state(
        cx: &mut TestAppContext,
        workspace: &Entity<WorkspaceRoot>,
    ) -> (bool, Option<bool>, Option<gpui::Pixels>) {
        cx.update(|cx| {
            let area = workspace.read(cx).dock_area().read(cx);
            match area.bottom_dock() {
                Some(dock) => {
                    let dock = dock.read(cx);
                    (true, Some(dock.is_open()), Some(dock.size()))
                }
                None => (false, None, None),
            }
        })
    }

    /// The leaf panel names under a dumped panel tree, in tab order. Reads
    /// the DUMP (live TabPanel/StackPanel state) rather than the DockItem
    /// snapshot: their `DockItem::add_panel` Split arm updates only the
    /// live view, so snapshots go stale after panel moves. Panels only
    /// ever live inside TabPanels, so leaves are the children of Tabs
    /// nodes (an emptied StackPanel dumps with a default Panel(Null) info
    /// — a childless-leaf heuristic would misread it as a panel).
    fn dump_leaf_names(state: &gpui_component::dock::PanelState) -> Vec<String> {
        match state.info {
            gpui_component::dock::PanelInfo::Tabs { .. } => state
                .children
                .iter()
                .map(|child| child.panel_name.clone())
                .collect(),
            _ => state.children.iter().flat_map(dump_leaf_names).collect(),
        }
    }

    /// The dock entity at `placement`, if any.
    fn dock_at(
        cx: &mut TestAppContext,
        workspace: &Entity<WorkspaceRoot>,
        placement: DockPlacement,
    ) -> Option<Entity<Dock>> {
        cx.update(|cx| {
            let area = workspace.read(cx).dock_area().read(cx);
            match placement {
                DockPlacement::Left => area.left_dock().cloned(),
                DockPlacement::Right => area.right_dock().cloned(),
                DockPlacement::Bottom => area.bottom_dock().cloned(),
                DockPlacement::Center => None,
            }
        })
    }

    /// The panel names hosted by the dock at `placement`, in tab order.
    fn dock_panel_names(
        cx: &mut TestAppContext,
        workspace: &Entity<WorkspaceRoot>,
        placement: DockPlacement,
    ) -> Vec<String> {
        let dock = dock_at(cx, workspace, placement).expect("dock present");
        cx.update(|cx| dump_leaf_names(&dock.read(cx).panel().view().dump(cx)))
    }

    /// The panel names hosted by the bottom dock's tab set, in tab order.
    fn bottom_dock_panel_names(
        cx: &mut TestAppContext,
        workspace: &Entity<WorkspaceRoot>,
    ) -> Vec<String> {
        dock_panel_names(cx, workspace, DockPlacement::Bottom)
    }

    /// Whether the dock at `placement` has a stack-rooted item — the
    /// droppability-relevant shape (review F1: a bare-Tabs root is locked:
    /// no drop targets, no drag sources).
    fn dock_root_is_stack_rooted(
        cx: &mut TestAppContext,
        workspace: &Entity<WorkspaceRoot>,
        placement: DockPlacement,
    ) -> bool {
        let dock = dock_at(cx, workspace, placement).expect("dock present");
        cx.update(|cx| matches!(dock.read(cx).panel(), DockItem::Split { .. }))
    }

    /// Whether the center item is stack-rooted.
    fn center_is_stack_rooted(cx: &mut TestAppContext, workspace: &Entity<WorkspaceRoot>) -> bool {
        cx.update(|cx| {
            matches!(
                workspace.read(cx).dock_area().read(cx).center(),
                DockItem::Split { .. }
            )
        })
    }

    /// Every dock root (left/right/bottom) AND the center are stack-rooted.
    fn assert_all_roots_stack_rooted(cx: &mut TestAppContext, workspace: &Entity<WorkspaceRoot>) {
        for placement in [DockPlacement::Left, DockPlacement::Right, DockPlacement::Bottom] {
            assert!(
                dock_root_is_stack_rooted(cx, workspace, placement),
                "{placement:?} dock root must be stack-rooted (bare Tabs are locked at this pin)"
            );
        }
        assert!(
            center_is_stack_rooted(cx, workspace),
            "center must be stack-rooted (their subscribe_item skips bare Tabs)"
        );
    }

    /// A pre-round saved layout (DOCK_STATE_VERSION, canvas center, sidebar
    /// left, editor right — NO bottom dock), in their serde shape with the
    /// BARE-Tabs dock roots every pre-round save serialised. The
    /// observables are deliberately NON-default (left 250px and CLOSED,
    /// right 300px) so "restored, not defaulted" is falsifiable — the
    /// original fixture's default-valued observables made that assertion
    /// tautological (review F3).
    fn saved_layout_without_bottom() -> serde_json::Value {
        serde_json::json!({
            "version": DOCK_STATE_VERSION,
            "center": {
                "panel_name": "TabPanel",
                "children": [
                    { "panel_name": CANVAS_PANEL_NAME, "children": [], "info": { "panel": null } }
                ],
                "info": { "tabs": { "active_index": 0 } }
            },
            "left_dock": {
                "panel": {
                    "panel_name": "TabPanel",
                    "children": [
                        { "panel_name": SIDEBAR_PANEL_NAME, "children": [], "info": { "panel": null } }
                    ],
                    "info": { "tabs": { "active_index": 0 } }
                },
                "placement": "left",
                "size": 250.0,
                "open": false
            },
            "right_dock": {
                "panel": {
                    "panel_name": "TabPanel",
                    "children": [
                        { "panel_name": EDITOR_PANEL_NAME, "children": [], "info": { "panel": null } }
                    ],
                    "info": { "tabs": { "active_index": 0 } }
                },
                "placement": "right",
                "size": 300.0,
                "open": true
            }
        })
    }

    /// wsc_ac03 (fresh boot): no saved layout → the default layout seeds
    /// the bottom dock CLOSED with the Log panel at the default height,
    /// and every dock item is stack-rooted (review F1: bare-Tabs roots
    /// render no drop targets and cannot source drags at this pin).
    #[gpui::test]
    fn wsc_ac03_fresh_boot_seeds_closed_bottom_log_dock(cx: &mut TestAppContext) {
        let shell = build_shell(cx, None);

        let (has, open, size) = bottom_dock_state(cx, &shell.workspace);
        assert!(has, "fresh boot has a bottom dock");
        assert_eq!(open, Some(false), "seeded CLOSED — the strip is the affordance");
        assert_eq!(size, Some(px(BOTTOM_DOCK_HEIGHT as f32)), "default open height");
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string()],
            "the Log panel anchors it"
        );
        assert_all_roots_stack_rooted(cx, &shell.workspace);
    }

    /// wsc_ac03 (backfill + normalise): restoring a pre-round layout (no
    /// bottom dock, bare-Tabs roots, NON-default sizes/open bits) appends
    /// the same closed Log dock post-load and re-roots every dock under a
    /// StackPanel, while the author's arrangement — a 300px right dock, a
    /// 250px CLOSED left dock — survives exactly. DOCK_STATE_VERSION is
    /// unchanged — the layout restores, it is not discarded.
    #[gpui::test]
    fn wsc_ac03_pre_round_layout_is_backfilled_with_closed_bottom_dock(cx: &mut TestAppContext) {
        let raw = serde_json::to_string_pretty(&saved_layout_without_bottom()).unwrap();
        let shell = build_shell(cx, Some(raw));

        // The restored arrangement survived, at values default_layout could
        // not have produced (review F3): right 300px (default 380), left
        // 250px and CLOSED (default 220, open).
        cx.update(|cx| {
            let area = shell.workspace.read(cx).dock_area().read(cx);
            let right = area.right_dock().expect("restored right dock").read(cx);
            assert_eq!(right.size(), px(300.0), "right width restored, not defaulted");
            assert!(right.is_open(), "right open bit restored");
            let left = area.left_dock().expect("restored left dock").read(cx);
            assert_eq!(left.size(), px(250.0), "left width restored, not defaulted");
            assert!(!left.is_open(), "left CLOSED bit restored, not forced open");
        });
        assert_eq!(
            dock_panel_names(cx, &shell.workspace, DockPlacement::Right),
            vec![EDITOR_PANEL_NAME.to_string()],
            "editor stayed in the right dock through normalisation"
        );
        assert_eq!(
            dock_panel_names(cx, &shell.workspace, DockPlacement::Left),
            vec![SIDEBAR_PANEL_NAME.to_string()],
            "sidebar stayed in the left dock through normalisation"
        );
        // …the bottom dock was backfilled, closed, with the Log panel…
        let (has, open, size) = bottom_dock_state(cx, &shell.workspace);
        assert!(has, "backfilled bottom dock");
        assert_eq!(open, Some(false), "backfilled CLOSED");
        assert_eq!(size, Some(px(BOTTOM_DOCK_HEIGHT as f32)));
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string()]
        );
        // …and every restored bare-Tabs root was normalised (review F1).
        assert_all_roots_stack_rooted(cx, &shell.workspace);
    }

    /// wsc_ac03 (already present): a saved layout that carries a bottom
    /// dock restores exactly as saved — its open state and size are kept,
    /// and no second dock (or forced state) is introduced. Its bare-Tabs
    /// root is normalised to the stack-rooted shape (arrangement, not tree
    /// shape, is what "as saved" pins).
    #[gpui::test]
    fn wsc_ac03_saved_bottom_dock_restores_as_saved(cx: &mut TestAppContext) {
        let mut layout = saved_layout_without_bottom();
        layout["bottom_dock"] = serde_json::json!({
            "panel": {
                "panel_name": "TabPanel",
                "children": [
                    { "panel_name": LOG_PANEL_NAME, "children": [], "info": { "panel": null } }
                ],
                "info": { "tabs": { "active_index": 0 } }
            },
            "placement": "bottom",
            "size": 240.0,
            "open": true
        });
        let raw = serde_json::to_string_pretty(&layout).unwrap();
        let shell = build_shell(cx, Some(raw));

        let (has, open, size) = bottom_dock_state(cx, &shell.workspace);
        assert!(has);
        assert_eq!(open, Some(true), "saved OPEN state kept — not forced closed");
        assert_eq!(size, Some(px(240.0)), "saved height kept — not the default");
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string()],
            "one Log panel — no double dock, no duplicate seed"
        );
        assert_all_roots_stack_rooted(cx, &shell.workspace);
    }

    /// wsc_ac04 (closed-before): entering presentation removes the bottom
    /// dock ENTIRELY (no 29px strip — there is no dock to paint one);
    /// exiting rebuilds it closed at the same size with the Log panel.
    #[gpui::test]
    fn wsc_ac04_round_trip_preserves_a_closed_bottom_dock(cx: &mut TestAppContext) {
        let shell = build_shell(cx, None);
        assert_eq!(bottom_dock_state(cx, &shell.workspace).1, Some(false));

        toggle_presentation_mode(cx, &shell.presentation);
        let (has, _, _) = bottom_dock_state(cx, &shell.workspace);
        assert!(!has, "presentation renders ZERO bottom-dock chrome");

        toggle_presentation_mode(cx, &shell.presentation);
        let (has, open, size) = bottom_dock_state(cx, &shell.workspace);
        assert!(has, "rebuilt on exit");
        assert_eq!(open, Some(false), "closed-before stays closed");
        assert_eq!(size, Some(px(BOTTOM_DOCK_HEIGHT as f32)), "size preserved");
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string()]
        );
        assert!(
            dock_root_is_stack_rooted(cx, &shell.workspace, DockPlacement::Bottom),
            "the rebuild round-trips the stack-rooted shape (review F1c)"
        );
    }

    /// wsc_ac04 (open-before, moved-in panel): a bottom dock the author
    /// opened, resized, and moved the editor into comes back exactly —
    /// open, same size, both panels — and a change to the REBUILT dock
    /// entity still schedules a layout save (the observer re-attached).
    #[gpui::test]
    fn wsc_ac04_round_trip_preserves_open_dock_with_moved_in_panel(cx: &mut TestAppContext) {
        let shell = build_shell(cx, None);

        // Author's arrangement: open the dock, resize it, move the editor
        // in (remove from its source dock first, as a drag would).
        shell
            .window
            .update(cx, |_root, window, cx| {
                let (right, bottom) = {
                    let area = shell.workspace.read(cx).dock_area().read(cx);
                    (
                        area.right_dock().expect("right dock").clone(),
                        area.bottom_dock().expect("bottom dock").clone(),
                    )
                };
                right.update(cx, |dock, cx| {
                    dock.remove_panel(std::sync::Arc::new(shell.editor.clone()), window, cx);
                });
                bottom.update(cx, |dock, cx| {
                    dock.set_open(true, window, cx);
                    dock.set_size(px(240.0), window, cx);
                    dock.add_panel(std::sync::Arc::new(shell.editor.clone()), window, cx);
                });
            })
            .expect("arrange bottom dock");
        cx.run_until_parked();
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string(), EDITOR_PANEL_NAME.to_string()],
            "editor moved into the bottom dock"
        );

        toggle_presentation_mode(cx, &shell.presentation);
        assert!(
            !bottom_dock_state(cx, &shell.workspace).0,
            "presentation removes the dock, moved-in panel and all"
        );

        toggle_presentation_mode(cx, &shell.presentation);
        let (has, open, size) = bottom_dock_state(cx, &shell.workspace);
        assert!(has, "rebuilt on exit");
        assert_eq!(open, Some(true), "open-before stays open");
        assert_eq!(size, Some(px(240.0)), "author's size preserved");
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string(), EDITOR_PANEL_NAME.to_string()],
            "contents preserved, including the moved-in editor"
        );
        assert!(
            dock_root_is_stack_rooted(cx, &shell.workspace, DockPlacement::Bottom),
            "the rebuild round-trips the stack-rooted shape (review F1c)"
        );

        // The save observer re-attached to the NEW dock entity: an isolated
        // post-round-trip Dock-entity change (resize) reaches the policy.
        shell.workspace.update(cx, |workspace, _| workspace.reset_save_probe());
        shell
            .window
            .update(cx, |_root, window, cx| {
                let bottom = shell
                    .workspace
                    .read(cx)
                    .dock_area()
                    .read(cx)
                    .bottom_dock()
                    .expect("rebuilt dock")
                    .clone();
                bottom.update(cx, |dock, cx| dock.set_size(px(300.0), window, cx));
            })
            .expect("resize rebuilt dock");
        cx.run_until_parked();
        cx.update(|cx| {
            assert!(
                shell.workspace.read(cx).save_pending(),
                "a resize of the rebuilt dock still schedules a save"
            );
        });

        // And a PANEL-TREE change (review F2, the ac-04 correction):
        // removing a panel from the rebuilt dock's TabPanel must reach the
        // policy through their StackPanel-mediated PanelEvent → DockEvent
        // chain — wiring that only exists because the rebuild is
        // stack-rooted (their subscribe_item skips bare-Tabs items, and
        // nothing else observes the TabPanel). Resize alone no longer
        // counts: it rides the direct dock-entity observer above.
        shell.workspace.update(cx, |workspace, _| workspace.reset_save_probe());
        shell
            .window
            .update(cx, |_root, window, cx| {
                let bottom = shell
                    .workspace
                    .read(cx)
                    .dock_area()
                    .read(cx)
                    .bottom_dock()
                    .expect("rebuilt dock")
                    .clone();
                bottom.update(cx, |dock, cx| {
                    dock.remove_panel(std::sync::Arc::new(shell.editor.clone()), window, cx);
                });
            })
            .expect("panel-tree change on rebuilt dock");
        cx.run_until_parked();
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string()],
            "the panel-tree change landed"
        );
        cx.update(|cx| {
            assert!(
                shell.workspace.read(cx).save_pending(),
                "a panel-tree change to the rebuilt dock schedules a save (F2)"
            );
        });
    }

    /// Drive the wsc_ac07 move handler (the body of the menu actions)
    /// without a lease on Root, as the real dispatch would.
    fn move_panel(
        cx: &mut TestAppContext,
        shell: &TestShell,
        panel: std::sync::Arc<dyn gpui_component::dock::PanelView>,
        destination: DockPlacement,
    ) {
        cx.update_window(shell.window.into(), |_, window, cx| {
            shell.workspace.update(cx, |workspace, cx| {
                workspace.move_panel_to_dock(panel, destination, window, cx);
            });
        })
        .expect("drive menu move");
        cx.run_until_parked();
    }

    /// wsc_ac07 (menu-move handler): the bootstrap that makes the drag
    /// gestures reachable — moving the editor to the bottom dock joins the
    /// Log panel's tab set (two tabs = real drag sources) and closes the
    /// emptied right dock; moving it back restores an open, stack-rooted
    /// right dock at its previous width and leaves the Log alone below.
    #[gpui::test]
    fn wsc_ac07_menu_move_editor_to_bottom_and_back(cx: &mut TestAppContext) {
        let shell = build_shell(cx, None);
        let editor: std::sync::Arc<dyn gpui_component::dock::PanelView> =
            std::sync::Arc::new(shell.editor.clone());

        // "Dock at Bottom": the editor joins the Log tab set, the
        // destination opens, and the emptied source dock closes.
        move_panel(cx, &shell, editor.clone(), DockPlacement::Bottom);
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string(), EDITOR_PANEL_NAME.to_string()],
            "editor docked at the bottom beside the Log"
        );
        assert_eq!(
            bottom_dock_state(cx, &shell.workspace).1,
            Some(true),
            "the destination dock opened (a move behind the closed strip would be invisible)"
        );
        let right = dock_at(cx, &shell.workspace, DockPlacement::Right).expect("right dock");
        cx.update(|cx| {
            assert!(
                !right.read(cx).is_open(),
                "the emptied source dock closed (wsc_ac07's pure decision)"
            );
        });
        assert!(
            dock_panel_names(cx, &shell.workspace, DockPlacement::Right).is_empty(),
            "the editor left the right dock"
        );

        // Idempotent: repeating the move re-lands the editor in place.
        move_panel(cx, &shell, editor.clone(), DockPlacement::Bottom);
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string(), EDITOR_PANEL_NAME.to_string()],
            "repeating the move is a no-op, not a duplicate"
        );

        // "Dock at Right": the return move rebuilds the emptied right dock
        // stack-rooted at its previous width, opened; the Log stays below.
        move_panel(cx, &shell, editor, DockPlacement::Right);
        assert_eq!(
            dock_panel_names(cx, &shell.workspace, DockPlacement::Right),
            vec![EDITOR_PANEL_NAME.to_string()],
            "editor restored to the right dock"
        );
        let right = dock_at(cx, &shell.workspace, DockPlacement::Right).expect("right dock");
        cx.update(|cx| {
            let right = right.read(cx);
            assert!(right.is_open(), "the rebuilt right dock is open");
            assert_eq!(
                right.size(),
                px(EDITOR_DOCK_WIDTH as f32),
                "the dock kept its width across the round trip"
            );
        });
        assert!(
            dock_root_is_stack_rooted(cx, &shell.workspace, DockPlacement::Right),
            "the rebuilt dock is stack-rooted (a bare-Tabs rebuild would be locked)"
        );
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string()],
            "the Log remains the bottom dock's anchor"
        );
        assert_eq!(
            bottom_dock_state(cx, &shell.workspace).1,
            Some(true),
            "the bottom dock was not emptied, so it stays open"
        );
    }
}
