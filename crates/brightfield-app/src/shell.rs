//! The docked authoring workspace (card 0017) — the GPUI/gpui-component
//! translation shim.
//!
//! Views here are deliberately thin (semantic-layer rule): every decision —
//! panel visibility, save timing, load fallback, atomic writes, sidebar
//! contents, notification routing — lives in the framework-free modules
//! (`shell_model`, `dock_state_file`, `spec_save`, `reload_feedback`,
//! `sidebar_model`); this file only executes them against gpui-component's
//! `DockArea`/`Panel`/`Root` machinery.
//!
//! - [`CanvasPanel`] — a Panel shim AROUND the untouched [`ChartView`]
//!   entity (aws_ac02): white canvas surface, workspace key context (bare
//!   `p` stays canvas-scoped), no chart event is intercepted or transformed.
//! - [`EditorPanel`] — `InputState::code_editor("yaml")`; cmd-s dispatches
//!   [`SaveSpec`], whose handler is `spec_save::save_spec_atomic` — the
//!   existing mtime watcher does everything else (aws_ac04).
//! - [`SidebarPanel`] — renders the derived [`SourceListing`]s (aws_ac06).
//! - [`WorkspaceRoot`] — hosts the `DockArea` (center canvas, right editor,
//!   left sidebar), loads/saves the versioned layout JSON (aws_ac03).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{
    actions, div, px, rgb, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement, Render,
    SharedString, Styled, Task, WeakEntity, Window,
};
use gpui_component::dock::{
    register_panel, DockArea, DockAreaState, DockEvent, DockItem, Panel, PanelControl, PanelEvent,
};
use gpui_component::input::{Input, InputState};
use gpui_component::notification::Notification;
use gpui_component::{ActiveTheme as _, Root};

use brightfield_ui::{ChartView, PresentationMode, TogglePresentation, WORKSPACE_KEY_CONTEXT};

use crate::dock_state_file::{
    self, LoadDecision, SaveAction, SavePolicy, DOCK_STATE_VERSION, SAVE_DEBOUNCE_MS,
};
use crate::reload_feedback::{self, Severity};
use crate::shell_model::{
    docks_open, layout_persistable, panel_visible, PanelRole, CANVAS_PANEL_NAME,
    EDITOR_DOCK_WIDTH, EDITOR_PANEL_NAME, SIDEBAR_DOCK_WIDTH, SIDEBAR_PANEL_NAME,
};
use crate::sidebar_model::SourceListing;
use crate::spec_save;

actions!(brightfield, [SaveSpec]);

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
    /// repaint, docks collapse/reopen (aws_ac07).
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
                let docks: Vec<_> = [area.left_dock(), area.right_dock(), area.bottom_dock()]
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
            last_synced: seed.map(str::to_string),
            conflict_pending: false,
        }
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
                Root::update(window, cx, |root, window, cx| {
                    root.push_notification(Notification::error(message.clone()), window, cx);
                });
            }
            spec_save::SaveDecision::ExternalConflict => {
                self.conflict_pending = true;
                let message =
                    "Spec changed on disk since it was loaded — save again to overwrite".to_string();
                eprintln!("Save deferred: {message}");
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
// Workspace root (aws_ac03)
// ---------------------------------------------------------------------------

/// The window root under gpui-component's `Root`: hosts the `DockArea`
/// (center canvas + right editor + left sidebar) and owns layout
/// persistence — versioned JSON in the user config dir, saved debounced on
/// `LayoutChanged` and flushed on quit, canvas excluded, every fallback
/// decided by the framework-free `dock_state_file` module.
pub struct WorkspaceRoot {
    dock_area: Entity<DockArea>,
    presentation: Entity<PresentationState>,
    /// Layout file location (`None` = no config dir; persistence off).
    state_path: Option<PathBuf>,
    /// The framework-free save policy (debounce + quit-flush + skip-if-
    /// unchanged); this view supplies the clock and executes the actions.
    policy: SavePolicy,
    /// Millisecond clock origin for the policy.
    boot: Instant,
    /// The pending debounced save, if any (dropped saves are superseded —
    /// latest change wins, matching the policy's deadline).
    _save_task: Option<Task<()>>,
}

impl WorkspaceRoot {
    /// Assemble the dock over the three panels, restoring the saved layout
    /// when usable (missing/corrupt/version-mismatch → default), and wire
    /// the save triggers.
    pub fn new(
        canvas: Entity<CanvasPanel>,
        editor: Entity<EditorPanel>,
        sidebar: Entity<SidebarPanel>,
        presentation: Entity<PresentationState>,
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

        let state_path = dock_state_file::dock_state_path(
            std::env::var("BRIGHTFIELD_CONFIG_DIR").ok().as_deref(),
            std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
            std::env::var("HOME").ok().as_deref(),
        );

        // Restore the saved arrangement, or build the default layout. Every
        // "is this state usable?" decision is dock_state_file's; a restore
        // that fails INSIDE the dock (their loader) falls back the same way.
        let raw = state_path
            .as_deref()
            .and_then(dock_state_file::read_state_file);
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
            Self::default_layout(&dock_area, &canvas, &editor, &sidebar, window, cx);
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

        // `LayoutChanged` alone has blind spots in their tree: `Dock::resize`
        // ends in a bare notify (no DockEvent), and a bare center
        // `DockItem::tab` is skipped by their `subscribe_item` (it assumes
        // StackPanel parents) — so dock widths and center tab changes would
        // otherwise persist only via the quit flush, and a crash would lose
        // them. Observe the dock entities and the center TabPanel directly,
        // funnelling into the same debounced policy: skip-if-unchanged +
        // debounce absorb the notify-storm a drag produces. (A split center
        // — the restored-layout shape — IS covered by their subscription.)
        let (edge_docks, center_tabs) = {
            let area = dock_area.read(cx);
            let edge_docks: Vec<_> = [area.left_dock(), area.right_dock(), area.bottom_dock()]
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            let center_tabs = match area.center() {
                DockItem::Tabs { view, .. } => Some(view.clone()),
                _ => None,
            };
            (edge_docks, center_tabs)
        };
        for dock in edge_docks {
            cx.observe_in(&dock, window, |this: &mut Self, _, window, cx| {
                this.schedule_save(window, cx);
            })
            .detach();
        }
        if let Some(tabs) = center_tabs {
            cx.observe_in(&tabs, window, |this: &mut Self, _, window, cx| {
                this.schedule_save(window, cx);
            })
            .detach();
        }

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
            state_path,
            policy: SavePolicy::default(),
            boot: Instant::now(),
            _save_task: None,
        }
    }

    /// Center canvas + left sidebar + right editor at their default sizes.
    fn default_layout(
        dock_area: &Entity<DockArea>,
        canvas: &Entity<CanvasPanel>,
        editor: &Entity<EditorPanel>,
        sidebar: &Entity<SidebarPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = dock_area.downgrade();
        let center = DockItem::tab(canvas.clone(), &weak, window, cx);
        let left = DockItem::tab(sidebar.clone(), &weak, window, cx);
        let right = DockItem::tab(editor.clone(), &weak, window, cx);
        dock_area.update(cx, |area, cx| {
            area.set_center(center, window, cx);
            area.set_left_dock(left, Some(px(SIDEBAR_DOCK_WIDTH as f32)), true, window, cx);
            area.set_right_dock(right, Some(px(EDITOR_DOCK_WIDTH as f32)), true, window, cx);
        });
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
/// than stack across repeated bad saves.
pub fn notify_reload_rejection(
    window: &gpui::WindowHandle<Root>,
    cx: &mut gpui::AsyncApp,
    severity: Severity,
    message: String,
) {
    let _ = window.update(cx, |root, window, cx| {
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
}
