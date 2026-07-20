//! The docked authoring workspace — the GPUI/gpui-component
//! translation shim.
//!
//! Views here are deliberately thin (semantic-layer rule): every decision —
//! panel visibility, save timing, load fallback, atomic writes, sidebar
//! profile formatting, notification routing, the log model, the bottom-dock
//! backfill and presentation action — lives in the framework-free modules
//! (`shell_model`, `dock_state_file`, `spec_save`, `reload_feedback`,
//! `profile_model`, `log_model`); this file only executes them against
//! gpui-component's `DockArea`/`Panel`/`Root` machinery.
//!
//! - [`CanvasPanel`] — a Panel shim AROUND the untouched [`ChartView`]
//!   entity: white canvas surface, workspace key context (bare
//!   `p` stays canvas-scoped), no chart event is intercepted or transformed.
//! - [`EditorPanel`] — `InputState::code_editor("yaml")`; cmd-s dispatches
//!   [`SaveSpec`], whose handler is `spec_save::save_spec_atomic` — the
//!   existing mtime watcher does everything else.
//! - [`SidebarPanel`] — renders the engine's per-source column profiles,
//!   formatted by [`profile_model`] (sidebar profiling).
//! - [`LogPanel`] — the bottom-dock reload/save feedback history over the
//!   gpui-free [`FeedbackLog`].
//! - [`WorkspaceRoot`] — hosts the `DockArea` (center canvas, right editor,
//!   left sidebar, bottom log), loads/saves the versioned layout JSON,
//!   backfills pre-round layouts with the closed Log dock,
//!   and removes/rebuilds the bottom dock across presentation
//!   mode.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    actions, div, px, rgb, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, MouseButton,
    ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Task,
    WeakEntity, Window,
};
use gpui_component::dock::{
    register_panel, Dock, DockArea, DockAreaState, DockEvent, DockItem, DockPlacement, Panel,
    PanelControl, PanelEvent, PanelState, PanelView,
};
use gpui_component::input::{Input, InputState};
use gpui_component::menu::PopupMenu;
use gpui_component::notification::Notification;
use gpui_component::{ActiveTheme as _, Root};

use brightfield_keys::{
    focus_jump_candidates, help_sheet, palette_filter, registry, Altitude, FocusState, FocusTree,
    JumpCandidate, PaletteCandidate, RecencyCounter,
};
use brightfield_spec::analysis::{analyse_spec, ComponentPath};
use brightfield_spec::ast::{Component, Spec};
use brightfield_spec::edit::{
    apply as apply_spec_edit, classify_edit, plot_at_path, SpecEdit, UndoOutcome, UndoStack,
};
use brightfield_spec::vocab::{ImplStatus, MarkKind};
use brightfield_ui::{ChartView, PresentationMode, TogglePresentation, WORKSPACE_KEY_CONTEXT};

use crate::arg_collector::{ArgCollector, ArgOutcome, ArgStep};
use crate::command_log::CommandLog;
use crate::dock_state_file::{
    self, LoadDecision, SaveAction, SavePolicy, DOCK_STATE_VERSION, SAVE_DEBOUNCE_MS,
};
use crate::keymap::{
    action_for_longname, AddMark, ChangeMarkType, ClearSelection, CycleColourScheme, DiveIn,
    FocusJump, FocusNextSibling, FocusPrevSibling, OpenHelp, OpenPalette, PopOut, ReloadSpec,
    RemoveMark, SetChannel, ToggleFocus, Undo,
};
use crate::log_model::FeedbackLog;
use crate::profile_model::{self, ProfileOutcome, SourceProfile};
use crate::reload_feedback::{self, Severity};
use crate::shell_model::{
    bottom_dock_action, bottom_dock_needs_backfill, dock_closes_when_emptied, docks_open,
    grammar_chrome_visible, layout_persistable, panel_visible, BottomDockAction, PanelRole,
    BOTTOM_DOCK_HEIGHT, CANVAS_PANEL_NAME, CMD_LOG_PANEL_NAME, EDITOR_DOCK_WIDTH,
    EDITOR_PANEL_NAME, LOG_PANEL_NAME, SIDEBAR_DOCK_WIDTH, SIDEBAR_PANEL_NAME,
};
use crate::spec_save;

actions!(
    brightfield,
    [
        SaveSpec,
        // menu-move actions: at pin b7e63cc2 a dock's ONLY panel
        // can never start a drag (is_last_panel), so the dropdown menu is
        // the bootstrap that first pairs panels up — after which real
        // drags work. Dispatched by the panels' tab menus; handled on the
        // WorkspaceRoot render root (every dispatch path bubbles there).
        DockEditorAtBottom,
        DockEditorAtRight,
        DockSidebarAtBottom,
        DockSidebarAtLeft,
        /// Commit the accumulated command-log edits to disk: cmd-s while
        /// the CANVAS has focus. Distinct from the
        /// editor's cmd-s `SaveSpec` (which saves the hand-typed buffer);
        /// resolved by focus context.
        CommitEdits
    ]
);

/// Key context of the spec editor panel — the scope the cmd-s binding
/// dispatches in (nested above the Input's own context, so the binding
/// fires only while the editor has focus).
pub const EDITOR_KEY_CONTEXT: &str = "BrightfieldEditor";

/// The DockArea's stable identity (state files key on it).
pub const DOCK_AREA_ID: &str = "brightfield-workspace";

/// The editor key bindings, declared as data: cmd-s →
/// [`SaveSpec`], scoped to [`EDITOR_KEY_CONTEXT`]. `main` feeds these to
/// `cx.bind_keys` alongside the workspace bindings.
pub fn editor_key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("cmd-s", SaveSpec, Some(EDITOR_KEY_CONTEXT))]
}

/// The command-log commit binding: cmd-s scoped to the
/// CANVAS ([`WORKSPACE_KEY_CONTEXT`]) → [`CommitEdits`]. So cmd-s commits pending
/// structural edits while the canvas is focused, and saves the buffer while the
/// editor is focused (`editor_key_bindings`) — resolved by focus context, the
/// two never collide. Fed to `cx.bind_keys` beside the editor + grammar bindings;
/// deliberately OUTSIDE the registry-sourced set (`grammar_key_bindings`), so the
/// "cmd-s stays an editor binding" invariant on that set still holds.
pub fn workspace_command_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "cmd-s",
        CommitEdits,
        Some(WORKSPACE_KEY_CONTEXT),
    )]
}

/// The one bit of shared shell mode: the gpui-free
/// [`PresentationMode`] (unmoved), wrapped in an entity so every panel's
/// `visible()` reads the same state the canvas's `p` handler flips.
pub struct PresentationState {
    /// Authoring ↔ presentation (the framework-free machine decides; views
    /// read).
    pub mode: PresentationMode,
}

// ---------------------------------------------------------------------------
// Canvas panel
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
    /// Focus tree over the dashboard's ComponentPath structure, seeded at
    /// assembly — the nav state machine + focus-ring geometry.
    focus_tree: FocusTree,
    /// Where keyboard focus sits (the bare-verb / focus-ring target); `None` when
    /// the dashboard has no navigable structure.
    focus_state: Option<FocusState>,
    /// The keyboard command-log session: the working `Spec` the
    /// structural verbs mutate, the snapshot-undo stack, and the shared
    /// [`CommandLog`]. `None` until [`Self::set_command_session`] wires it (the
    /// dump path + a spec that failed to re-parse never do), so the verbs
    /// no-op gracefully on a session-less canvas.
    command: Option<CommandSession>,
}

/// The keyboard command-log session state riding [`CanvasPanel`].
/// Framework-bound only in that it holds a gpui `Entity<CommandLog>`; the
/// reducer target ([`Spec`]) and the [`UndoStack`] are gpui-free.
struct CommandSession {
    /// The WORKING `Spec` — the reducer target ([`apply_spec_edit`] mutates it).
    /// The live coordinator holds no `Spec` (crossfilter.rs), so it lives here.
    working_spec: Spec,
    /// The snapshot-undo stack (a whole-`Spec` clone per edit).
    undo: UndoStack,
    /// The plot path each pushed snapshot's edit targeted, PARALLEL to `undo`'s
    /// snapshots, so an undo knows which plot the reverted edit touched (v1 undo
    /// refreshes all plots, so this is retained for future targeted refresh /
    /// diagnostics rather than strictly required today).
    undo_paths: Vec<String>,
    /// The shared append-only command log the inline readout renders.
    log: Entity<CommandLog>,
}

impl CanvasPanel {
    /// Wrap `chart_view` under the resolved dashboard `title`, over `focus_tree`
    /// (the dashboard's navigable structure). Focus starts at the root; the
    /// initial focus ring is seeded on the wrapped view.
    pub fn new(
        chart_view: Entity<ChartView>,
        title: impl Into<SharedString>,
        presentation: Entity<PresentationState>,
        focus_tree: FocusTree,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_state = FocusState::new(&focus_tree);
        // Seed the initial focus ring around the focused (root) node.
        if let Some(rect) = focus_state.and_then(|s| focus_tree.rect_of(s.path(&focus_tree))) {
            chart_view.update(cx, |cv, cx| {
                cv.set_focus_ring(Some(rect));
                cx.notify();
            });
        }
        Self {
            chart_view,
            title: title.into(),
            presentation,
            dock_area: None,
            focus_handle: cx.focus_handle(),
            focus_tree,
            focus_state,
            command: None,
        }
    }

    /// Wire the command-log session AFTER construction — so
    /// [`CanvasPanel::new`]'s signature (and its many test callers) stay
    /// untouched. `working_spec` is the parsed launch spec (the reducer target);
    /// `log` is the shared [`CommandLog`] the inline readout renders and the
    /// structural verbs append to. Called once from `main` when the pipeline
    /// produced a re-parsable spec.
    pub fn set_command_session(&mut self, working_spec: Spec, log: Entity<CommandLog>) {
        self.command = Some(CommandSession {
            working_spec,
            undo: UndoStack::new(),
            undo_paths: Vec::new(),
            log,
        });
    }

    /// The focused View's plot path (`root/vconcat[0]`, …), or `None` when focus
    /// is at the Dashboard altitude (the command-log verbs are View-scoped). The
    /// path scheme matches `edit::plot_at_path` + the coordinator's `LivePlot`
    /// paths (all built from the shared `collect_plot_nodes`/`descend` walk), so
    /// an edit targeting it resolves in both the reducer and the coordinator.
    fn focused_view_path(&self) -> Option<String> {
        let s = self.focus_state.as_ref()?;
        if s.altitude(&self.focus_tree) == Altitude::View {
            Some(s.path(&self.focus_tree).0.clone())
        } else {
            None
        }
    }

    /// The next gate-clean retype for the focused View's primary mark (bare `m`
    /// CYCLES the mark kind — it takes no argument, unlike `a`/`e`).
    /// Walks the Implemented kinds after the current one and returns the first
    /// for which [`classify_edit`] is clean (so a cross-zero-baseline-class flip
    /// or a title-changing rebind is skipped, not refused mid-cycle). `None` when
    /// there is no focused View, no primary mark, or no clean retype exists.
    fn next_retype_edit(&self) -> Option<SpecEdit> {
        let session = self.command.as_ref()?;
        let path = self.focused_view_path()?;
        let plot = plot_at_path(&session.working_spec, &path)?;
        let current = plot.items.iter().find_map(|c| match c {
            Component::Mark(m) => Some(m.kind),
            _ => None,
        })?;
        let kinds: Vec<MarkKind> = MarkKind::all()
            .iter()
            .copied()
            .filter(|k| k.status() == ImplStatus::Implemented)
            .collect();
        let start = kinds.iter().position(|&k| k == current).unwrap_or(0);
        for off in 1..=kinds.len() {
            let cand = kinds[(start + off) % kinds.len()];
            if cand == current {
                continue;
            }
            let edit = SpecEdit::ChangeMarkType {
                plot: ComponentPath(path.clone()),
                mark_ordinal: 0,
                new_kind: cand,
            };
            if classify_edit(&session.working_spec, &edit).is_ok() {
                return Some(edit);
            }
        }
        None
    }

    /// Append a message to the command log and repaint the readout.
    fn log_command(&self, f: impl FnOnce(&mut CommandLog), cx: &mut Context<Self>) {
        if let Some(session) = self.command.as_ref() {
            session.log.update(cx, |log, cx| {
                f(log);
                cx.notify();
            });
        }
    }

    /// The focused View's plot path — the argument overlay's target.
    /// `None` when there is no command session or focus sits at the
    /// Dashboard altitude (the argument verbs are View-scoped).
    pub fn command_target(&self) -> Option<ComponentPath> {
        self.command.as_ref()?;
        self.focused_view_path().map(ComponentPath)
    }

    /// The canonical YAML of the working spec, for a commit
    /// — `None` when there are no uncommitted edits (nothing to flush) or no
    /// command session. Serialises through the same canonical writer a manual
    /// save uses; the caller routes it through the editor buffer + save pipeline.
    pub fn pending_commit_yaml(&self) -> Option<String> {
        let session = self.command.as_ref()?;
        if session.undo.uncommitted_len() == 0 {
            return None;
        }
        brightfield_spec::parse::serialise_spec(&session.working_spec).ok()
    }

    /// Seal the current uncommitted edits behind a commit barrier + record the
    /// commit in the log — called after the buffer/save
    /// pipeline has flushed `pending_commit_yaml` to disk.
    pub fn mark_committed(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.command.as_mut() {
            session.undo.commit_barrier();
        }
        self.log_command(
            |log| {
                log.commit();
            },
            cx,
        );
    }

    /// Record a refused command (e.g. a dirty-buffer commit refusal) in the log.
    pub fn log_command_refusal(&self, reason: &str, cx: &mut Context<Self>) {
        self.log_command(|log| log.record_refused(reason.to_string()), cx);
    }

    /// Apply a structural [`SpecEdit`] to the working spec + live coordinator:
    /// snapshot for undo, classify+apply to the working
    /// `Spec`, and on success re-analyse + drive the coordinator's transient
    /// refresh + log the edit; on a refusal, log the reason (never mutating).
    /// TRANSIENT — no disk write (the commit is a separate deliberate action).
    /// `pub` so the argument overlay (on `WorkspaceRoot`) applies a completed
    /// `AddMark`/`SetChannel`.
    pub fn apply_command_edit(
        &mut self,
        edit: &SpecEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Phase 1: classify + apply to the working spec under a snapshot. Borrows
        // only `self.command`; produces the re-analysed spec or a refusal reason.
        let prepared: Result<(Spec, brightfield_spec::analysis::SpecAnalysis, String), String> = {
            let Some(session) = self.command.as_mut() else {
                return;
            };
            let snapshot = session.working_spec.clone();
            match apply_spec_edit(&mut session.working_spec, edit) {
                Ok(()) => match analyse_spec(&session.working_spec) {
                    Ok(analysis) => {
                        session.undo.push(snapshot);
                        session.undo_paths.push(edit.plot_path().to_string());
                        Ok((session.working_spec.clone(), analysis, edit.summary()))
                    }
                    Err(e) => {
                        // A gate-clean edit should analyse; roll back defensively.
                        session.working_spec = snapshot;
                        Err(format!("{}: re-analysis failed: {e}", edit.summary()))
                    }
                },
                Err(reason) => Err(format!("{}: {}", edit.summary(), reason.reason())),
            }
        };
        // Phase 2: drive the coordinator (disjoint field `chart_view`) + log.
        match prepared {
            Ok((spec, analysis, summary)) => {
                let coord = self.chart_view.read(cx).coordinator();
                let changed = if let Some(coord) = coord {
                    coord.borrow_mut().apply_spec_edit(edit, spec, analysis, cx)
                } else {
                    false
                };
                self.log_command(|log| log.record_edit(summary), cx);
                if changed {
                    window.refresh();
                }
            }
            Err(reason) => self.log_command(|log| log.record_refused(reason), cx),
        }
    }

    /// `ChangeMarkType` handler (bare `m`, View-scoped): cycle the
    /// focused View's primary mark to the next gate-clean kind, applied live.
    fn change_mark_type(
        &mut self,
        _: &ChangeMarkType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.next_retype_edit() {
            Some(edit) => self.apply_command_edit(&edit, window, cx),
            None => self.log_command(
                |log| log.record_refused("change-mark-type: no gate-clean retype from here"),
                cx,
            ),
        }
    }

    /// `RemoveMark` handler (bare `d`, View-scoped): drop the focused
    /// View's primary mark, applied live. Emptying a plot is refused-with-reason
    /// by the reducer (logged, never applied).
    fn remove_mark(&mut self, _: &RemoveMark, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.focused_view_path() else {
            return;
        };
        let edit = SpecEdit::RemoveMark {
            plot: ComponentPath(path),
            mark_ordinal: 0,
        };
        self.apply_command_edit(&edit, window, cx);
    }

    /// `Undo` handler (bare `u`): pop the snapshot-undo stack, restore
    /// the working spec, and reload every plot from it. A no-op past a commit
    /// barrier / on an empty stack is logged with its reason.
    fn undo(&mut self, _: &Undo, window: &mut Window, cx: &mut Context<Self>) {
        // A `Spec` + its analysis is much larger than a `String`, so
        // clippy::large_enum_variant wants the big variant boxed. This enum is a
        // function-local control-flow carrier: it is constructed once per undo
        // keystroke and matched immediately on the next line, so the size
        // difference buys one stack move that boxing would trade for a heap
        // allocation plus an indirection. Not worth obscuring the pattern.
        #[allow(clippy::large_enum_variant)]
        enum UndoAction {
            Reload(Spec, brightfield_spec::analysis::SpecAnalysis),
            Refused(String),
        }
        let action = {
            let Some(session) = self.command.as_mut() else {
                return;
            };
            match session.undo.undo() {
                UndoOutcome::Restored(prev) => {
                    session.undo_paths.pop();
                    session.working_spec = *prev;
                    match analyse_spec(&session.working_spec) {
                        Ok(a) => UndoAction::Reload(session.working_spec.clone(), a),
                        Err(e) => UndoAction::Refused(format!("undo: re-analysis failed: {e}")),
                    }
                }
                UndoOutcome::NothingToUndo => {
                    UndoAction::Refused("undo: nothing to undo".to_string())
                }
                UndoOutcome::PastCommitBarrier => {
                    UndoAction::Refused("undo: nothing to undo (past the last commit)".to_string())
                }
            }
        };
        match action {
            UndoAction::Reload(spec, analysis) => {
                let coord = self.chart_view.read(cx).coordinator();
                if let Some(coord) = coord {
                    coord.borrow_mut().reload_all_from_spec(spec, analysis, cx);
                }
                self.log_command(
                    |log| {
                        log.record_undo();
                    },
                    cx,
                );
                window.refresh();
            }
            UndoAction::Refused(reason) => self.log_command(|log| log.record_refused(reason), cx),
        }
    }

    /// Recompute the focus ring from the focus state and push it to the wrapped
    /// view (cleared in presentation mode), then repaint the breadcrumb. The
    /// single point every nav move and the presentation toggle route through.
    fn apply_focus(&mut self, cx: &mut Context<Self>) {
        let ring = if grammar_chrome_visible(self.presentation.read(cx).mode) {
            self.focus_state
                .as_ref()
                .and_then(|s| self.focus_tree.rect_of(s.path(&self.focus_tree)))
        } else {
            None
        };
        self.chart_view.update(cx, |cv, cx| {
            cv.set_focus_ring(ring);
            cx.notify();
        });
        cx.notify();
    }

    /// Focus-jump candidates: the focus tree's nodes ranked against a
    /// fuzzy `query` over their paths. The `/` overlay (on `WorkspaceRoot`) reads
    /// this; the data stays with the tree owner.
    pub fn jump_candidates(&self, query: &str) -> Vec<JumpCandidate> {
        focus_jump_candidates(&self.focus_tree, query)
    }

    /// The focused altitude — the command palette's scope input.
    /// `Dashboard` when the dashboard has no navigable structure.
    pub fn current_altitude(&self) -> Altitude {
        self.focus_state
            .as_ref()
            .map(|s| s.altitude(&self.focus_tree))
            .unwrap_or(Altitude::Dashboard)
    }

    /// Move focus to `path`: the overlay's chosen jump target. No-op if
    /// the path is not in the tree.
    pub fn jump_focus_to(&mut self, path: &ComponentPath, cx: &mut Context<Self>) {
        if self
            .focus_state
            .as_mut()
            .is_some_and(|s| s.jump_to(&self.focus_tree, path))
        {
            self.apply_focus(cx);
        }
    }

    /// The breadcrumb text for the focused node — `"<altitude> · <path>"` — or
    /// `None` when there is no navigable structure.
    fn breadcrumb_text(&self) -> Option<SharedString> {
        let state = self.focus_state.as_ref()?;
        let altitude = state.altitude(&self.focus_tree);
        let path = state.path(&self.focus_tree);
        Some(format!("{} · {}", altitude.label(), path.0).into())
    }

    /// Dispatch a nav move (`DiveIn`/`PopOut`/sibling) and refresh focus chrome.
    fn dive_in(&mut self, _: &DiveIn, _window: &mut Window, cx: &mut Context<Self>) {
        if self
            .focus_state
            .as_mut()
            .is_some_and(|s| s.dive(&self.focus_tree))
        {
            self.apply_focus(cx);
        }
    }

    fn pop_out(&mut self, _: &PopOut, _window: &mut Window, cx: &mut Context<Self>) {
        if self
            .focus_state
            .as_mut()
            .is_some_and(|s| s.pop(&self.focus_tree))
        {
            self.apply_focus(cx);
        }
    }

    fn focus_next_sibling(
        &mut self,
        _: &FocusNextSibling,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .focus_state
            .as_mut()
            .is_some_and(|s| s.next_sibling(&self.focus_tree))
        {
            self.apply_focus(cx);
        }
    }

    fn focus_prev_sibling(
        &mut self,
        _: &FocusPrevSibling,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .focus_state
            .as_mut()
            .is_some_and(|s| s.prev_sibling(&self.focus_tree))
        {
            self.apply_focus(cx);
        }
    }

    /// `ClearSelection` handler (Esc). Overlays are dismissed
    /// on `WorkspaceRoot` (they capture focus), so a canvas Esc is the Esc
    /// ladder's terminal rung: clear the focused view's selection, or every view's
    /// at the dashboard altitude. A no-op (nothing selected) skips the refresh.
    fn clear_selection(&mut self, _: &ClearSelection, window: &mut Window, cx: &mut Context<Self>) {
        let Some(coord) = self.chart_view.read(cx).coordinator() else {
            return;
        };
        let target = self.focus_state.as_ref().map(|s| {
            (
                s.altitude(&self.focus_tree),
                s.path(&self.focus_tree).0.clone(),
            )
        });
        let cleared = match target {
            Some((Altitude::View, path)) => coord.borrow_mut().clear_plot(&path, cx),
            _ => coord.borrow_mut().clear_all(cx),
        };
        if cleared {
            window.refresh();
        }
    }

    /// `CycleColourScheme` handler (bare `c`, canvas-scoped):
    /// cycle the FOCUSED VIEW's sequential colour scheme, transiently (no spec
    /// write). View-scoped (registry scope = [View]): a clean no-op at the
    /// dashboard altitude, unlike `clear_selection` which falls through to
    /// clear_all. The coordinator recolours the plot's launch Fill ramp and
    /// re-renders that one plot's scene; `window.refresh()` repaints.
    fn cycle_colour_scheme(
        &mut self,
        _: &CycleColourScheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(coord) = self.chart_view.read(cx).coordinator() else {
            return;
        };
        let target = self.focus_state.as_ref().map(|s| {
            (
                s.altitude(&self.focus_tree),
                s.path(&self.focus_tree).0.clone(),
            )
        });
        let changed = match target {
            Some((Altitude::View, path)) => coord.borrow_mut().cycle_scheme(&path, cx),
            _ => false,
        };
        if changed {
            window.refresh();
        }
    }

    /// Wire the hosting dock area (called once the `DockArea` exists).
    pub fn set_dock_area(&mut self, dock_area: WeakEntity<DockArea>) {
        self.dock_area = Some(dock_area);
    }

    /// The wrapped dashboard view (shim assertion surface).
    #[cfg(test)]
    pub fn chart_view(&self) -> &Entity<ChartView> {
        &self.chart_view
    }

    /// The panel's title text (shim assertion surface).
    #[cfg(test)]
    pub fn title_text(&self) -> &SharedString {
        &self.title
    }

    /// `TogglePresentation` handler (bare `p`, canvas-scoped — the original
    /// binding, unchanged): flip the shared mode, then apply the
    /// framework-free dock mapping — panels re-read `visible()` on the
    /// repaint, the left/right docks collapse/reopen. The BOTTOM
    /// dock is not touched here: its closed form still paints a 29px strip,
    /// so `WorkspaceRoot` (observing the shared mode) removes and rebuilds
    /// it instead — and the stash it takes must see the dock's
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
        // Show/hide the focus ring + breadcrumb with the authoring chrome.
        self.apply_focus(cx);
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
        // The white canvas surface + flex centring, now inside the
        // panel (the dock owns the window background). The mouse-down
        // listener ONLY claims focus for the `p` binding — it does not
        // stop propagation, so every chart element handler below sees the
        // exact events it always did.
        // The breadcrumb: an absolute top-left readout of the
        // focused altitude + path, hidden under presentation. Absolute so
        // it never disturbs the centred canvas layout.
        let breadcrumb = grammar_chrome_visible(self.presentation.read(cx).mode)
            .then(|| self.breadcrumb_text())
            .flatten();
        // A lightweight inline "uncommitted edits" badge, authoring-
        // only — an at-a-glance cue right where the author works. The full
        // history lives in the dedicated bottom-dock CommandLog panel.
        let command_readout: Option<usize> =
            (grammar_chrome_visible(self.presentation.read(cx).mode))
                .then_some(self.command.as_ref())
                .flatten()
                .map(|s| s.log.read(cx).uncommitted())
                .filter(|n| *n > 0);
        div()
            .relative()
            .size_full()
            .bg(rgb(0xffffff))
            .key_context(WORKSPACE_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::toggle_presentation))
            .on_action(cx.listener(Self::dive_in))
            .on_action(cx.listener(Self::pop_out))
            .on_action(cx.listener(Self::focus_next_sibling))
            .on_action(cx.listener(Self::focus_prev_sibling))
            .on_action(cx.listener(Self::clear_selection))
            .on_action(cx.listener(Self::cycle_colour_scheme))
            .on_action(cx.listener(Self::change_mark_type))
            .on_action(cx.listener(Self::remove_mark))
            .on_action(cx.listener(Self::undo))
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
            .children(breadcrumb.map(|text| {
                div()
                    .absolute()
                    .left(px(8.0))
                    .top(px(8.0))
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(rgb(0x1f2430))
                    .text_color(rgb(0xd7dae0))
                    .text_size(px(12.0))
                    .child(text)
            }))
            .children(command_readout.map(|uncommitted| {
                div()
                    .absolute()
                    .right(px(8.0))
                    .bottom(px(8.0))
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(rgb(0x161a22))
                    .border_1()
                    .border_color(rgb(0x2b3242))
                    .text_size(px(11.0))
                    .text_color(rgb(0x8a93a6))
                    .child(format!("{uncommitted} uncommitted · cmd-s to commit"))
            }))
    }
}

// ---------------------------------------------------------------------------
// Spec editor panel
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
    /// notification is also appended here (the Log panel is the
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
    /// message pair the workspace notification carries.
    fn log_feedback(&self, severity: Severity, message: &str, cx: &mut Context<Self>) {
        self.log.update(cx, |log, cx| {
            log.append(severity, message);
            cx.notify();
        });
    }

    /// `SaveSpec` handler (cmd-s, editor context): `decide_save` first,
    /// then the pure atomic write. Success is quiet — the watcher's
    /// re-render (or a rejection notification) is the feedback.
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
            spec_save::decide_save(
                buffer.as_ref(),
                file_now.as_deref(),
                self.last_synced.as_deref(),
            )
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
                let message = "Spec changed on disk since it was loaded — save again to overwrite"
                    .to_string();
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
                            root.push_notification(
                                Notification::error(message.clone()),
                                window,
                                cx,
                            );
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
    pub fn reseed_from_disk(
        &mut self,
        contents: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer = self.state.read(cx).value();
        if !spec_save::should_reseed(buffer.as_ref(), self.last_synced.as_deref(), contents) {
            return;
        }
        self.state.update(cx, |state, cx| {
            state.set_value(contents.to_string(), window, cx)
        });
        self.last_synced = Some(contents.to_string());
        self.conflict_pending = false;
    }

    /// Commit a command-log flush THROUGH the editor buffer:
    /// the PRISTINE-BUFFER gate first (a DIRTY buffer refuses — never `set_value`
    /// over hand-typed edits), then render the canonical `yaml` into the buffer
    /// and write it atomically, letting the watcher reload it. `Ok(())` on a
    /// successful flush; `Err(reason)` when the buffer is dirty (the author saves
    /// or discards first) or the write fails. Routes through the SAME buffer +
    /// `save_spec_atomic` path a manual save uses (no out-of-band write).
    pub fn commit_buffer(
        &mut self,
        yaml: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let buffer = self.state.read(cx).value();
        if !spec_save::commit_is_allowed(buffer.as_ref(), self.last_synced.as_deref()) {
            return Err(spec_save::DIRTY_BUFFER_COMMIT_REFUSAL.to_string());
        }
        self.state.update(cx, |state, cx| {
            state.set_value(yaml.to_string(), window, cx)
        });
        match spec_save::save_spec_atomic(yaml, &self.spec_path) {
            Ok(_) => {
                self.last_synced = Some(yaml.to_string());
                Ok(())
            }
            Err(e) => {
                let message = format!("Commit save failed: {e}");
                eprintln!("{message}");
                self.log_feedback(Severity::Error, &message, cx);
                Err(message)
            }
        }
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

    /// Dock-placement moves: the menu is the bootstrap gesture —
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
// Sidebar panel
// ---------------------------------------------------------------------------

/// The left-dock Data sidebar: real DuckDB-computed per-source column profiles.
/// Profiles are computed OFF the UI thread — on the launch
/// session before the window opens, and refreshed from the watcher's throwaway
/// session on hot reload — and handed here as pure data; this panel only lays
/// out the strings [`profile_model`] formatted. Display-only.
pub struct SidebarPanel {
    /// One profile per `data:` source, declaration order.
    profiles: Vec<SourceProfile>,
    /// Shared presentation state (visibility mapping input).
    presentation: Entity<PresentationState>,
    focus_handle: FocusHandle,
}

impl SidebarPanel {
    /// Host the computed `profiles`.
    pub fn new(
        profiles: Vec<SourceProfile>,
        presentation: Entity<PresentationState>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            profiles,
            presentation,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Replace the hosted profiles and repaint — the hot-reload refresh tap
    /// (the watcher hands fresh profiles computed on its throwaway session,
    /// closing the frozen-at-launch gap).
    pub fn set_profiles(&mut self, profiles: Vec<SourceProfile>, cx: &mut Context<Self>) {
        self.profiles = profiles;
        cx.notify();
    }

    /// The hosted profiles — the shim assertion surface, and the
    /// set-channel COLUMN pick-list's source (delta finding 6).
    pub fn profiles(&self) -> &[SourceProfile] {
        &self.profiles
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

    /// Dock-placement moves — see [`EditorPanel::dropdown_menu`];
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
        // Scrolls so a tall (many-column) or wide (long-name) source stays
        // usable; the flat name + type + stat-line layout is v1 (collapse /
        // search / histograms are deferred). A zero-source spec renders an
        // empty container — the existing empty state.
        div()
            .id("sidebar-scroll")
            .size_full()
            .overflow_y_scroll()
            .p_3()
            .text_size(px(12.0))
            .children(self.profiles.iter().map(|source| {
                // Header: source name, plus the row count for profiled sources.
                let mut header = div().flex().gap_2().child(
                    div()
                        .text_color(foreground)
                        .child(SharedString::from(source.name.clone())),
                );
                if let ProfileOutcome::Profiled { row_count, .. } = &source.outcome {
                    header = header.child(div().text_color(muted).child(SharedString::from(
                        profile_model::row_count_label(*row_count),
                    )));
                }
                let mut block = div().mb_3().child(header);
                match &source.outcome {
                    ProfileOutcome::Profiled { columns, .. } => {
                        // Per column: name (foreground), muted type, muted
                        // stat line — capped with a "(+N more)" tail.
                        let (shown, more) = profile_model::column_cap(columns.len());
                        block = block.children(columns.iter().take(shown).map(|col| {
                            div()
                                .pl_2()
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_color(foreground)
                                                .child(SharedString::from(col.name.clone())),
                                        )
                                        .child(
                                            div()
                                                .text_color(muted)
                                                .child(SharedString::from(col.type_name.clone())),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_color(muted)
                                        .child(SharedString::from(profile_model::stat_line(col))),
                                )
                        }));
                        if let Some(tail) = more {
                            block = block.child(
                                div()
                                    .pl_2()
                                    .text_color(muted)
                                    .child(SharedString::from(tail)),
                            );
                        }
                    }
                    ProfileOutcome::Unsupported => {
                        block = block.child(
                            div()
                                .pl_2()
                                .text_color(muted)
                                .child(SharedString::from(profile_model::UNSUPPORTED_ROW)),
                        );
                    }
                    ProfileOutcome::Failed(reason) => {
                        block = block.child(
                            div()
                                .pl_2()
                                .text_color(muted)
                                .child(SharedString::from(profile_model::unavailable_row(reason))),
                        );
                    }
                }
                block
            }))
    }
}

// ---------------------------------------------------------------------------
// Log panel
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

    /// The hosted log (shim assertion surface).
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
        // Minimal by design: text rows, newest at top, no scroll
        // machinery. Severity is the row's colour cue.
        let danger = cx.theme().danger;
        let warning = cx.theme().warning;
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let entries = self.log.read(cx).entries().to_vec();
        let list = div()
            .size_full()
            .p_3()
            .text_size(px(12.0))
            .overflow_hidden();
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

/// The bottom-dock command-log panel — the SECOND bottom
/// citizen, rendering the framework-free [`CommandLog`] (the structural edits /
/// commits / refusals a keyboard author runs), newest at top, with the
/// uncommitted count in its header. DISTINCT from [`LogPanel`], which stays the
/// reload/save diagnostics log. Its arrival is what unlocks the dock-drag (a
/// single-panel dock can never source a drag).
pub struct CommandLogPanel {
    log: Entity<CommandLog>,
    presentation: Entity<PresentationState>,
    focus_handle: FocusHandle,
}

impl CommandLogPanel {
    /// Host the shared command `log` (the SAME entity the canvas appends to).
    pub fn new(
        log: Entity<CommandLog>,
        presentation: Entity<PresentationState>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Repaint on ANY log change (delta finding 7). The canvas appends to the
        // log from ITS own context — a bare-verb refusal (`d`/`m`/undo) or an edit
        // notifies the log entity's observers, not this panel — so without this
        // observe the new row only surfaces on an unrelated later frame.
        cx.observe(&log, |_, _, cx| cx.notify()).detach();
        Self {
            log,
            presentation,
            focus_handle: cx.focus_handle(),
        }
    }
}

impl EventEmitter<PanelEvent> for CommandLogPanel {}

impl Focusable for CommandLogPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for CommandLogPanel {
    fn panel_name(&self) -> &'static str {
        CMD_LOG_PANEL_NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Commands")
    }

    fn closable(&self, _cx: &App) -> bool {
        false
    }

    fn zoomable(&self, _cx: &App) -> Option<PanelControl> {
        None
    }

    fn visible(&self, cx: &App) -> bool {
        // Same visibility rule as the diagnostics Log (authoring-only, a bottom
        // citizen): reuse PanelRole::Log rather than adding a new role.
        panel_visible(self.presentation.read(cx).mode, PanelRole::Log)
    }
}

impl Render for CommandLogPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;
        let accent = cx.theme().info;
        let danger = cx.theme().danger;
        let log = self.log.read(cx);
        let uncommitted = log.uncommitted();
        let entries = log.entries().to_vec();
        let header = div()
            .text_size(px(11.0))
            .text_color(muted)
            .child(SharedString::from(format!("{uncommitted} uncommitted")));
        let list = div()
            .flex()
            .flex_col()
            .gap_1()
            .size_full()
            .p_3()
            .text_size(px(12.0))
            .overflow_hidden()
            .child(header);
        if entries.is_empty() {
            return list.child(div().text_color(muted).child(SharedString::from(
                "(no command-log edits yet — try m / a / e / d / u)",
            )));
        }
        list.children(entries.into_iter().map(move |entry| {
            use crate::command_log::CommandLogEntry;
            let (tag, tag_color) = match &entry {
                CommandLogEntry::Edit(_) => ("edit", foreground),
                CommandLogEntry::Commit(_) => ("commit", accent),
                CommandLogEntry::Refused(_) => ("refused", danger),
            };
            div()
                .flex()
                .gap_2()
                .child(div().text_color(tag_color).child(SharedString::from(tag)))
                .child(
                    div()
                        .text_color(foreground)
                        .child(SharedString::from(entry.text().to_string())),
                )
        }))
    }
}

// ---------------------------------------------------------------------------
// Workspace root
// ---------------------------------------------------------------------------

/// The bottom dock's rebuild state, stashed while presentation mode has the
/// dock removed: the dumped panel tree (their save format — the
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
/// Which grammar overlay is open over the workspace. Rendered by
/// [`WorkspaceRoot`] as a trailing child OUTSIDE the workspace key-context, so no
/// bare verb fires underneath it; the open overlay captures focus.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Overlay {
    /// No overlay.
    Closed,
    /// The `?` help sheet (static, read-only).
    Help,
    /// The `/` focus-jump finder (fuzzy query + selection).
    Jump,
    /// The command palette (space / cmd-shift-p): fuzzy verb-finder that
    /// dispatches the chosen verb against the refocused canvas.
    Palette,
    /// The argument-prompt overlay for `a`/`e`: a running
    /// [`ArgCollector`] drives a step-by-step pick (mark KIND, or CHANNEL then
    /// COLUMN); the completed `SpecEdit` applies against the focused plot.
    Arg,
}

/// The command palette renders at most this many candidate rows; the down-arrow
/// clamp, the highlight, and `run_palette` all agree on this window so a
/// selection can't escape the visible rows (review fix).
const PALETTE_MAX_ROWS: usize = 12;

/// Whether a palette candidate can be RUN from the palette (review fix):
/// enabled (not reserved) AND backed by an action reachable from the canvas-
/// anchored dispatch. `save-spec` is enabled but its handler lives on the editor
/// subtree — unreachable from the canvas — so it renders greyed (with its key
/// shown) instead of silently no-opping on Enter.
fn palette_runnable(c: &PaletteCandidate) -> bool {
    c.enabled && action_for_longname(c.longname).is_some()
}

pub struct WorkspaceRoot {
    dock_area: Entity<DockArea>,
    presentation: Entity<PresentationState>,
    /// The movable panels: the menu-move action handlers resolve
    /// a dispatched action to the entity to move. The canvas is the fixed
    /// centre and the Log panel anchors the bottom dock — neither moves.
    editor_panel: Entity<EditorPanel>,
    sidebar_panel: Entity<SidebarPanel>,
    /// The canvas panel — held so the global focus-toggle (cmd-e) can move focus
    /// to it (it is not otherwise reachable from a root-level handler)
    /// and so the `/` overlay can drive its focus jump.
    canvas: Entity<CanvasPanel>,
    /// Which grammar overlay is open.
    overlay: Overlay,
    /// The focus handle the open overlay captures — so bare canvas verbs do not
    /// fire underneath it (the no-bare-under-overlay invariant, live).
    overlay_focus: FocusHandle,
    /// The `/` focus-jump query and selected row.
    jump_query: String,
    jump_selected: usize,
    /// The command-palette query, selected row, and per-session recency.
    /// The recency lifts recently-run verbs under an empty query; it resets each
    /// launch (the spec's sanctioned v1 simplification).
    palette_query: String,
    palette_selected: usize,
    palette_recency: RecencyCounter,
    /// The argument-prompt overlay's running collection:
    /// `Some` while `a`/`e` is collecting; the query + selected row filter the
    /// current step's option list. `None` whenever the overlay is not `Arg`.
    arg: Option<ArgCollector>,
    arg_query: String,
    arg_selected: usize,
    /// Shared force-reload flag the cmd-r handler flips; the spec watcher polls
    /// it and runs one identical reload pass when set.
    reload_trigger: Arc<AtomicBool>,
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
    // 9 arguments: the five panels this root docks, the presentation state they
    // share, the reload flag, and gpui's `window`/`cx` — which every gpui
    // constructor carries and which alone put this two over clippy's threshold.
    // There is no grouping of the panels that is not just a struct named
    // "the arguments to new".
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        canvas: Entity<CanvasPanel>,
        editor: Entity<EditorPanel>,
        sidebar: Entity<SidebarPanel>,
        log: Entity<LogPanel>,
        command_log: Entity<CommandLogPanel>,
        presentation: Entity<PresentationState>,
        reload_trigger: Arc<AtomicBool>,
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
        // with_saved_layout defaults the trigger (its test callers
        // never drives the watcher); the live wiring is injected here.
        let mut this = Self::with_saved_layout(
            canvas,
            editor,
            sidebar,
            log,
            command_log,
            presentation,
            state_path,
            raw,
            window,
            cx,
        );
        this.reload_trigger = reload_trigger;
        this
    }

    /// [`WorkspaceRoot::new`] with the persistence inputs injected: the
    /// layout file location (`None` = persistence off) and the raw saved
    /// layout JSON (`None` = fresh boot). `new` supplies the real env/file;
    /// the dock-layout tests supply fixtures.
    #[allow(clippy::too_many_arguments)]
    fn with_saved_layout(
        canvas: Entity<CanvasPanel>,
        editor: Entity<EditorPanel>,
        sidebar: Entity<SidebarPanel>,
        log: Entity<LogPanel>,
        command_log: Entity<CommandLogPanel>,
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
        register_panel(cx, CMD_LOG_PANEL_NAME, {
            let command_log = command_log.clone();
            move |_, _, _, _, _| Box::new(command_log.clone())
        });

        // Restore the saved arrangement, or build the default layout. Every
        // "is this state usable?" decision is dock_state_file's; a restore
        // that fails INSIDE the dock (their loader) falls back the same way.
        let restored = match dock_state_file::decide_load(raw.as_deref(), DOCK_STATE_VERSION) {
            LoadDecision::Restore(value) => match serde_json::from_value::<DockAreaState>(value) {
                Ok(state) => {
                    let loaded = dock_area.update(cx, |area, cx| area.load(state, window, cx));
                    if let Err(e) = &loaded {
                        eprintln!(
                            "dock layout: failed to restore saved layout ({e}); using default"
                        );
                    }
                    loaded.is_ok()
                }
                Err(e) => {
                    eprintln!(
                        "dock layout: saved layout does not deserialise ({e}); using default"
                    );
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
            Self::default_layout(
                &dock_area,
                &canvas,
                &editor,
                &sidebar,
                &log,
                &command_log,
                window,
                cx,
            );
        } else {
            // Normalise (correction): pre-round saves
            // serialised bare-Tabs dock roots, which this pin's drag
            // machinery treats as locked — re-root them under StackPanels,
            // preserving the author's arrangement.
            Self::normalise_dock_roots(&dock_area, window, cx);
            if bottom_dock_needs_backfill(dock_area.read(cx).has_dock(DockPlacement::Bottom)) {
                // Backfill: a restored layout with no bottom dock —
                // append the same closed bottom dock the default layout seeds
                // (Log + CommandLog tabs), without touching the restored
                // arrangement. (A saved layout that predates the CommandLog panel
                // is discarded by the v2 version bump before reaching here.)
                Self::seed_bottom_dock(&dock_area, &log, &command_log, window, cx);
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

        // The presentation round trip for the BOTTOM dock: the
        // canvas's `p` handler flips the shared mode and collapses the
        // left/right rails; this observer executes the framework-free
        // bottom-dock action — remove entirely on enter (a closed bottom
        // dock still paints a 29px strip), rebuild from the stash on exit.
        cx.observe_in(&presentation, window, |this: &mut Self, _, window, cx| {
            this.sync_bottom_dock_to_mode(window, cx);
            // Presentation hides authoring chrome — dismiss any open overlay.
            if !grammar_chrome_visible(this.presentation.read(cx).mode)
                && this.overlay != Overlay::Closed
            {
                this.overlay = Overlay::Closed;
                cx.notify();
            }
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
            canvas,
            overlay: Overlay::Closed,
            overlay_focus: cx.focus_handle(),
            jump_query: String::new(),
            jump_selected: 0,
            palette_query: String::new(),
            palette_selected: 0,
            palette_recency: RecencyCounter::new(),
            arg: None,
            arg_query: String::new(),
            arg_selected: 0,
            // Defaulted here; `new` injects the live trigger the watcher shares.
            reload_trigger: Arc::new(AtomicBool::new(false)),
            state_path,
            policy: SavePolicy::default(),
            boot: Instant::now(),
            bottom_stash: None,
            _save_task: None,
        }
    }

    /// `ToggleFocus` handler (cmd-e, global): move focus
    /// between the canvas and the editor. Registered on the render root, which is
    /// an ancestor of every dispatch path, so it fires regardless of focus.
    fn on_toggle_focus(&mut self, _: &ToggleFocus, window: &mut Window, cx: &mut Context<Self>) {
        // Entity implements both PanelView::focus_handle and Focusable::focus_handle;
        // the focus target is the Focusable one.
        let canvas = Focusable::focus_handle(&self.canvas, cx);
        if canvas.is_focused(window) {
            let editor = Focusable::focus_handle(&self.editor_panel, cx);
            window.focus(&editor, cx);
        } else {
            window.focus(&canvas, cx);
        }
        cx.notify();
    }

    /// `ReloadSpec` handler (cmd-r, global): flip the shared
    /// force-reload flag; the watcher polls it and runs one identical reload pass
    /// (dirty-safe reseed, panic-guarded pipeline, layout/chrome gates, scene
    /// swap, rejection routing — all reused, not reimplemented). Registered on
    /// the render root like `on_toggle_focus`, and deliberately NOT gated on
    /// `overlays_allowed` — the watcher reloads in presentation mode too, and a
    /// consumer-preview refresh is legitimate. A dirty editor buffer is preserved
    /// by the watcher's `should_reseed` tap (standard revert-to-disk semantics),
    /// and cmd-r only ever reads FROM disk, so it can never clobber the spec.
    fn on_reload(&mut self, _: &ReloadSpec, _window: &mut Window, cx: &mut Context<Self>) {
        self.reload_trigger.store(true, Ordering::Release);
        cx.notify();
    }

    /// Whether authoring chrome — and thus overlays — may show.
    fn overlays_allowed(&self, cx: &App) -> bool {
        grammar_chrome_visible(self.presentation.read(cx).mode)
    }

    /// `OpenHelp` handler (`?`): open the help sheet. No-op in presentation.
    fn on_open_help(&mut self, _: &OpenHelp, window: &mut Window, cx: &mut Context<Self>) {
        if !self.overlays_allowed(cx) {
            return;
        }
        self.overlay = Overlay::Help;
        window.focus(&self.overlay_focus, cx);
        cx.notify();
    }

    /// `FocusJump` handler (`/`): open the focus-jump finder. No-op in
    /// presentation.
    fn on_focus_jump(&mut self, _: &FocusJump, window: &mut Window, cx: &mut Context<Self>) {
        if !self.overlays_allowed(cx) {
            return;
        }
        self.overlay = Overlay::Jump;
        self.jump_query.clear();
        self.jump_selected = 0;
        window.focus(&self.overlay_focus, cx);
        cx.notify();
    }

    /// `OpenPalette` handler (space / cmd-shift-p): open the command
    /// palette. No-op in presentation. Mirrors `on_focus_jump`.
    fn on_open_palette(&mut self, _: &OpenPalette, window: &mut Window, cx: &mut Context<Self>) {
        if !self.overlays_allowed(cx) {
            return;
        }
        self.overlay = Overlay::Palette;
        self.palette_query.clear();
        self.palette_selected = 0;
        window.focus(&self.overlay_focus, cx);
        cx.notify();
    }

    /// `AddMark` handler (bare `a`, View-scoped): open the
    /// argument overlay collecting a mark KIND, targeting the focused plot. A
    /// no-op when there is no focused View / command session (bubbled from the
    /// canvas; handled here because the overlay lives on `WorkspaceRoot`).
    fn on_add_mark(&mut self, _: &AddMark, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.canvas.read(cx).command_target() else {
            return;
        };
        self.open_arg_overlay(ArgCollector::add_mark(target), window, cx);
    }

    /// `SetChannel` handler (bare `e`, View-scoped): open
    /// the argument overlay collecting a CHANNEL then a COLUMN for the focused
    /// plot's primary mark.
    fn on_set_channel(&mut self, _: &SetChannel, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.canvas.read(cx).command_target() else {
            return;
        };
        self.open_arg_overlay(ArgCollector::set_channel(target, 0), window, cx);
    }

    /// Open the argument-prompt overlay with a fresh collector (shared by `a`/`e`).
    fn open_arg_overlay(
        &mut self,
        collector: ArgCollector,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.overlays_allowed(cx) {
            return;
        }
        self.arg = Some(collector);
        self.arg_query.clear();
        self.arg_selected = 0;
        self.overlay = Overlay::Arg;
        window.focus(&self.overlay_focus, cx);
        cx.notify();
    }

    /// `CommitEdits` handler (cmd-s, canvas-focused): flush
    /// the accumulated transient edits to disk THROUGH the editor buffer behind
    /// the pristine-buffer gate. Nothing pending → a quiet no-op; a DIRTY editor
    /// buffer → refused-with-reason (the hand-typed text survives). On success the
    /// canvas seals the undo barrier + logs the commit and the watcher reloads.
    fn on_commit(&mut self, _: &CommitEdits, window: &mut Window, cx: &mut Context<Self>) {
        let Some(yaml) = self.canvas.read(cx).pending_commit_yaml() else {
            return; // nothing to commit
        };
        let result = self
            .editor_panel
            .update(cx, |editor, cx| editor.commit_buffer(&yaml, window, cx));
        match result {
            Ok(()) => self.canvas.update(cx, |c, cx| c.mark_committed(cx)),
            Err(reason) => self
                .canvas
                .update(cx, |c, cx| c.log_command_refusal(&reason, cx)),
        }
        cx.notify();
    }

    /// Close any open overlay and return focus to the canvas (the live
    /// realisation of the Esc ladder's dismiss-overlay rung).
    fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = Overlay::Closed;
        // Drop any running argument collection (the Esc "cancel to Idle" leg).
        self.arg = None;
        self.arg_query.clear();
        self.arg_selected = 0;
        window.focus(&Focusable::focus_handle(&self.canvas, cx), cx);
        cx.notify();
    }

    /// The current step's fuzzy-filtered option list: the
    /// enumerable kind / channel picks, and — for the COLUMN step — the profiled
    /// source columns (the `SourceProfile`), each case-insensitively
    /// filtered by `arg_query`. The column list is a convenience: `run_arg` still
    /// falls back to the raw query, so a column absent from the profile (or an
    /// unprofiled source) stays typeable.
    fn arg_options(&self, cx: &App) -> Vec<String> {
        let Some(collector) = self.arg.as_ref() else {
            return Vec::new();
        };
        let columns = match collector.step() {
            ArgStep::Column { .. } => self.arg_column_options(cx),
            _ => Vec::new(),
        };
        let all = collector.options(&columns);
        let q = self.arg_query.to_lowercase();
        all.into_iter()
            .filter(|o| o.to_lowercase().contains(&q))
            .collect()
    }

    /// The distinct, non-internal column names across every PROFILED source in
    /// the Data sidebar, in first-seen order — the enumerable pick
    /// list the set-channel COLUMN step offers (delta finding 6). v1
    /// offers the UNION across sources; narrowing to the focused plot's own
    /// source is a follow-up (the free-text fallback covers the multi-source
    /// case in the meantime).
    fn arg_column_options(&self, cx: &App) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for source in self.sidebar_panel.read(cx).profiles() {
            if let ProfileOutcome::Profiled { columns, .. } = &source.outcome {
                for col in columns {
                    if !seen.iter().any(|c| c == &col.name) {
                        seen.push(col.name.clone());
                    }
                }
            }
        }
        seen
    }

    /// Run the current argument pick (Enter): the highlighted option, or — on the
    /// free-text COLUMN step — the raw query. `Pending` advances the step (clear
    /// the query); `Ready` applies the edit against the canvas and closes;
    /// `Invalid` leaves the overlay open.
    fn run_arg(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filtered = self.arg_options(cx);
        let is_column = matches!(
            self.arg.as_ref().map(ArgCollector::step),
            Some(ArgStep::Column { .. })
        );
        let choice = filtered
            .get(self.arg_selected)
            .cloned()
            .or_else(|| is_column.then(|| self.arg_query.clone()))
            .unwrap_or_default();
        let Some(collector) = self.arg.as_mut() else {
            return;
        };
        match collector.pick(&choice) {
            ArgOutcome::Pending => {
                self.arg_query.clear();
                self.arg_selected = 0;
                cx.notify();
            }
            ArgOutcome::Ready(edit) => {
                // Refocus the canvas BEFORE applying (the run_palette ordering
                // note): apply_command_edit reads the focused plot's coordinator.
                self.close_overlay(window, cx);
                self.canvas
                    .update(cx, |c, cx| c.apply_command_edit(&edit, window, cx));
            }
            ArgOutcome::Invalid => {
                cx.notify();
            }
        }
    }

    /// Move the argument-overlay selection within the filtered options (down =
    /// `true`), bound to ↓/↑ + Ctrl-j/k like the palette.
    fn arg_nav(&mut self, down: bool, cx: &mut Context<Self>) {
        let n = self.arg_options(cx).len();
        if down {
            self.arg_selected = (self.arg_selected + 1).min(n.saturating_sub(1));
        } else {
            self.arg_selected = self.arg_selected.saturating_sub(1);
        }
        cx.notify();
    }

    /// Run the selected focus-jump candidate: move canvas focus there, then close.
    fn run_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self
            .canvas
            .read(cx)
            .jump_candidates(&self.jump_query)
            .get(self.jump_selected)
            .map(|c| c.path.clone());
        if let Some(path) = target {
            self.canvas.update(cx, |c, cx| c.jump_focus_to(&path, cx));
        }
        self.close_overlay(window, cx);
    }

    /// Run the selected command-palette verb. A reserved (greyed) row is
    /// non-runnable and keeps the palette open; an empty result dismisses.
    ///
    /// CRITICAL ORDER: close the overlay FIRST — `close_overlay` refocuses the
    /// canvas synchronously — THEN `dispatch_action`, which snapshots the focused
    /// node at call time. A canvas-subtree verb (dive-in / clear-selection /
    /// cycle-colour-scheme / toggle-presentation) dispatched while the overlay
    /// still held focus would resolve against the overlay→root path and silently
    /// no-op. So this INVERTS `run_jump` (which acts, then closes).
    fn run_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let altitude = self.canvas.read(cx).current_altitude();
        let reg = registry();
        let selected = {
            let cands = palette_filter(&reg, altitude, &self.palette_query, &self.palette_recency);
            cands.get(self.palette_selected).cloned()
        };
        let Some(cand) = selected else {
            self.close_overlay(window, cx);
            return;
        };
        if !palette_runnable(&cand) {
            // Reserved, or enabled-but-not-canvas-runnable (save-spec): greyed and
            // non-runnable — leave the palette open rather than silently closing
            // with no effect.
            return;
        }
        let longname = cand.longname;
        // Refocus the canvas BEFORE dispatch (the ordering note above).
        self.close_overlay(window, cx);
        if let Some(action) = action_for_longname(longname) {
            window.dispatch_action(action, cx);
        }
        self.palette_recency.record(longname);
    }

    /// Move the `/` jump selection down (`true`) or up. Bound to ↓/↑ AND
    /// Ctrl-j/Ctrl-k (vim-style, matching the canvas's own h/j/k/l nav) — Ctrl so
    /// bare j/k stay free to type into the fuzzy query (the fzf / Telescope way).
    fn jump_nav(&mut self, down: bool, cx: &mut Context<Self>) {
        if down {
            let n = self.canvas.read(cx).jump_candidates(&self.jump_query).len();
            self.jump_selected = (self.jump_selected + 1).min(n.saturating_sub(1));
        } else {
            self.jump_selected = self.jump_selected.saturating_sub(1);
        }
        cx.notify();
    }

    /// Move the command-palette selection within the rendered window, down
    /// (`true`) or up — bound to ↓/↑ AND Ctrl-j/Ctrl-k (see [`Self::jump_nav`]).
    fn palette_nav(&mut self, down: bool, cx: &mut Context<Self>) {
        if down {
            let altitude = self.canvas.read(cx).current_altitude();
            let n = palette_filter(
                &registry(),
                altitude,
                &self.palette_query,
                &self.palette_recency,
            )
            .len();
            self.palette_selected =
                (self.palette_selected + 1).min(n.min(PALETTE_MAX_ROWS).saturating_sub(1));
        } else {
            self.palette_selected = self.palette_selected.saturating_sub(1);
        }
        cx.notify();
    }

    /// Key handling for the focused overlay: Esc dismisses, and — for the
    /// `/` finder — up/down move the selection, Enter runs it, and printable keys
    /// edit the query. Bare canvas verbs cannot fire here (the overlay holds
    /// focus).
    fn on_overlay_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        match self.overlay {
            Overlay::Closed => {}
            Overlay::Help => {
                if key == "escape" {
                    self.close_overlay(window, cx);
                }
            }
            Overlay::Jump => match key {
                "escape" => self.close_overlay(window, cx),
                "enter" => self.run_jump(window, cx),
                "up" => self.jump_nav(false, cx),
                "down" => self.jump_nav(true, cx),
                "k" if event.keystroke.modifiers.control => self.jump_nav(false, cx),
                "j" if event.keystroke.modifiers.control => self.jump_nav(true, cx),
                "backspace" => {
                    self.jump_query.pop();
                    self.jump_selected = 0;
                    cx.notify();
                }
                "space" => {
                    self.jump_query.push(' ');
                    self.jump_selected = 0;
                    cx.notify();
                }
                k if k.chars().count() == 1 => {
                    if let Some(c) = k.chars().next().filter(|c| !c.is_control()) {
                        self.jump_query.push(c);
                        self.jump_selected = 0;
                        cx.notify();
                    }
                }
                _ => {}
            },
            Overlay::Palette => match key {
                "escape" => self.close_overlay(window, cx),
                "enter" => self.run_palette(window, cx),
                "up" => self.palette_nav(false, cx),
                "down" => self.palette_nav(true, cx),
                "k" if event.keystroke.modifiers.control => self.palette_nav(false, cx),
                "j" if event.keystroke.modifiers.control => self.palette_nav(true, cx),
                "backspace" => {
                    self.palette_query.pop();
                    self.palette_selected = 0;
                    cx.notify();
                }
                "space" => {
                    self.palette_query.push(' ');
                    self.palette_selected = 0;
                    cx.notify();
                }
                k if k.chars().count() == 1 => {
                    if let Some(c) = k.chars().next().filter(|c| !c.is_control()) {
                        self.palette_query.push(c);
                        self.palette_selected = 0;
                        cx.notify();
                    }
                }
                _ => {}
            },
            Overlay::Arg => match key {
                "escape" => self.close_overlay(window, cx),
                "enter" => self.run_arg(window, cx),
                "up" => self.arg_nav(false, cx),
                "down" => self.arg_nav(true, cx),
                "k" if event.keystroke.modifiers.control => self.arg_nav(false, cx),
                "j" if event.keystroke.modifiers.control => self.arg_nav(true, cx),
                "backspace" => {
                    self.arg_query.pop();
                    self.arg_selected = 0;
                    cx.notify();
                }
                k if k.chars().count() == 1 => {
                    if let Some(c) = k.chars().next().filter(|c| !c.is_control()) {
                        self.arg_query.push(c);
                        self.arg_selected = 0;
                        cx.notify();
                    }
                }
                _ => {}
            },
        }
    }

    /// The help overlay body: every verb grouped by scope, with its bound
    /// key(s), one-line help, and a reserved flag.
    fn help_overlay_body(&self) -> gpui::Div {
        let reg = registry();
        let sheet = help_sheet(&reg);
        let mut sections = div().flex().flex_col().gap_3();
        for altitude in [Altitude::Dashboard, Altitude::View] {
            let rows: Vec<_> = sheet
                .iter()
                .filter(|r| r.altitudes.contains(&altitude))
                .collect();
            let mut section = div().flex().flex_col().gap_1().child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x8a93a6))
                    .child(altitude.label().to_uppercase()),
            );
            for r in rows {
                let keys = if r.keys.is_empty() {
                    "—".to_string()
                } else {
                    r.keys.join(" / ")
                };
                let mut line = format!("{keys}   {}", r.help);
                if let Some(reason) = r.reserved_reason {
                    line.push_str(&format!("   · reserved: {}", reason.reason()));
                }
                let colour = if r.reserved_reason.is_some() {
                    rgb(0x6b7280)
                } else {
                    rgb(0xd7dae0)
                };
                section = section.child(div().text_size(px(13.0)).text_color(colour).child(line));
            }
            sections = sections.child(section);
        }
        div()
            .w(px(560.0))
            .max_h(px(620.0))
            .overflow_hidden()
            .p_4()
            .rounded(px(8.0))
            .bg(rgb(0x161a22))
            .border_1()
            .border_color(rgb(0x2b3242))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(15.0))
                    .text_color(rgb(0xf0f2f6))
                    .child("Keyboard grammar — help  (Esc to close)"),
            )
            .child(sections)
    }

    /// The focus-jump overlay body: a query line + the ranked candidate
    /// paths, the selected row highlighted.
    fn jump_overlay_body(&self, cx: &App) -> gpui::Div {
        let candidates = self.canvas.read(cx).jump_candidates(&self.jump_query);
        let mut list = div().flex().flex_col().gap_0();
        for (i, c) in candidates.iter().take(12).enumerate() {
            let selected = i == self.jump_selected;
            let marker = if c.is_plot { "▪" } else { "▸" };
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(if selected {
                        rgb(0x2f6feb)
                    } else {
                        rgb(0x161a22)
                    })
                    .text_size(px(13.0))
                    .text_color(if selected {
                        rgb(0xffffff)
                    } else {
                        rgb(0xd7dae0)
                    })
                    .child(format!("{marker}  {}", c.path.0)),
            );
        }
        div()
            .w(px(480.0))
            .max_h(px(420.0))
            .overflow_hidden()
            .p_3()
            .rounded(px(8.0))
            .bg(rgb(0x161a22))
            .border_1()
            .border_color(rgb(0x2b3242))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(rgb(0x0e1017))
                    .text_size(px(14.0))
                    .text_color(rgb(0xf0f2f6))
                    .child(format!("/ {}", self.jump_query)),
            )
            .child(list)
    }

    /// The command-palette overlay body: a query line + the ranked verb
    /// rows (longname, bound key inline, one-line help), the selected row
    /// highlighted, reserved rows greyed with their bucket reason.
    fn palette_overlay_body(&self, cx: &App) -> gpui::Div {
        let altitude = self.canvas.read(cx).current_altitude();
        let reg = registry();
        let cands = palette_filter(&reg, altitude, &self.palette_query, &self.palette_recency);
        let mut list = div().flex().flex_col().gap_0();
        for (i, c) in cands.iter().take(PALETTE_MAX_ROWS).enumerate() {
            let selected = i == self.palette_selected;
            let runnable = palette_runnable(c);
            let key = c
                .primary_key
                .map(|k| format!("  [{k}]"))
                .unwrap_or_default();
            let mut label = format!("{}{}   {}", c.longname, key, c.help);
            if let Some(reason) = c.reserved_reason {
                label.push_str(&format!("   · reserved: {}", reason.reason()));
            } else if !runnable {
                // Enabled but not runnable from here (save-spec): point at its key.
                label.push_str("   · run with its key");
            }
            // Selected wins the highlight; an unselected non-runnable row is greyed.
            let colour = if selected {
                rgb(0xffffff)
            } else if runnable {
                rgb(0xd7dae0)
            } else {
                rgb(0x6b7280)
            };
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(if selected {
                        rgb(0x2f6feb)
                    } else {
                        rgb(0x161a22)
                    })
                    .text_size(px(13.0))
                    .text_color(colour)
                    .child(label),
            );
        }
        div()
            .w(px(520.0))
            .max_h(px(460.0))
            .overflow_hidden()
            .p_3()
            .rounded(px(8.0))
            .bg(rgb(0x161a22))
            .border_1()
            .border_color(rgb(0x2b3242))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(rgb(0x0e1017))
                    .text_size(px(14.0))
                    .text_color(rgb(0xf0f2f6))
                    .child(format!("> {}", self.palette_query)),
            )
            .child(list)
    }

    /// The argument-prompt overlay body: a prompt line for
    /// the current step (mark kind / channel / column) + the fuzzy-filtered
    /// option rows, the selected row highlighted. The free-text COLUMN step shows
    /// the typed query as the pick (Enter takes it verbatim).
    fn arg_overlay_body(&self, cx: &App) -> gpui::Div {
        let (verb, prompt, is_column) = match self.arg.as_ref() {
            Some(c) => {
                let v = match c.step() {
                    ArgStep::Kind => "add-mark",
                    ArgStep::Channel | ArgStep::Column { .. } => "set-channel",
                };
                (
                    v,
                    c.step().prompt(),
                    matches!(c.step(), ArgStep::Column { .. }),
                )
            }
            None => ("", "", false),
        };
        let options = self.arg_options(cx);
        let mut list = div().flex().flex_col().gap_0();
        if is_column && options.is_empty() {
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .text_size(px(12.0))
                    .text_color(rgb(0x8a93a6))
                    .child("type a column name, then Enter"),
            );
        }
        for (i, o) in options.iter().take(PALETTE_MAX_ROWS).enumerate() {
            let selected = i == self.arg_selected;
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(if selected {
                        rgb(0x2f6feb)
                    } else {
                        rgb(0x161a22)
                    })
                    .text_size(px(13.0))
                    .text_color(if selected {
                        rgb(0xffffff)
                    } else {
                        rgb(0xd7dae0)
                    })
                    .child(o.clone()),
            );
        }
        div()
            .w(px(480.0))
            .max_h(px(460.0))
            .overflow_hidden()
            .p_3()
            .rounded(px(8.0))
            .bg(rgb(0x161a22))
            .border_1()
            .border_color(rgb(0x2b3242))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x8a93a6))
                    .child(format!("{verb} · pick a {prompt}  (Esc to cancel)")),
            )
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(4.0))
                    .bg(rgb(0x0e1017))
                    .text_size(px(14.0))
                    .text_color(rgb(0xf0f2f6))
                    .child(format!("> {}", self.arg_query)),
            )
            .child(list)
    }

    /// A stack-rooted single-panel dock item (correction, review
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
    /// plus the closed bottom Log dock. Every item is
    /// stack-rooted (see [`Self::stack_rooted_tab`]).
    // 8 arguments, for the same reason as `new`: the dock area, the five panels
    // being placed into it, and gpui's `window`/`cx`.
    #[allow(clippy::too_many_arguments)]
    fn default_layout(
        dock_area: &Entity<DockArea>,
        canvas: &Entity<CanvasPanel>,
        editor: &Entity<EditorPanel>,
        sidebar: &Entity<SidebarPanel>,
        log: &Entity<LogPanel>,
        command_log: &Entity<CommandLogPanel>,
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
        Self::seed_bottom_dock(dock_area, log, command_log, window, cx);
    }

    /// Seed the bottom dock CLOSED with the Log + CommandLog panels as tabs
    /// (makes the CommandLog the SECOND citizen — the pair is what unlocks the
    /// dock drag, since a single-panel dock can never source a drag). The 29px
    /// strip is the drop/expand affordance, closed doesn't re-carve the author's
    /// layout, and the tabs are stack-rooted so the strip's TabPanel is a real
    /// drop target (review F1).
    fn seed_bottom_dock(
        dock_area: &Entity<DockArea>,
        log: &Entity<LogPanel>,
        command_log: &Entity<CommandLogPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = dock_area.downgrade();
        let tabs = DockItem::tabs(
            vec![
                std::sync::Arc::new(log.clone()) as std::sync::Arc<dyn PanelView>,
                std::sync::Arc::new(command_log.clone()),
            ],
            &weak,
            window,
            cx,
        );
        let bottom = DockItem::v_split(vec![tabs], &weak, window, cx);
        dock_area.update(cx, |area, cx| {
            area.set_bottom_dock(
                bottom,
                Some(px(BOTTOM_DOCK_HEIGHT as f32)),
                false,
                window,
                cx,
            );
        });
    }

    /// Re-root any restored dock item whose root is bare `Tabs` under a
    /// StackPanel (correction): every pre-round save
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

        for placement in [
            DockPlacement::Left,
            DockPlacement::Right,
            DockPlacement::Bottom,
        ] {
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

    /// Execute the framework-free bottom-dock action for the current mode:
    /// presentation removes the dock after stashing its rebuild
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
                    DockItem::Tabs { .. } => DockItem::v_split(vec![item], &weak, window, cx),
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

    /// Move `panel` to the dock at `destination` (the menu-move
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
                Self::set_edge_dock(
                    &self.dock_area,
                    destination,
                    panel.clone(),
                    None,
                    window,
                    cx,
                );
            }
        }

        for placement in [
            DockPlacement::Left,
            DockPlacement::Right,
            DockPlacement::Bottom,
        ] {
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

    /// action handlers — registered on the render root, which is
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

    /// The hosted dock area (assertion surface).
    #[cfg(test)]
    pub fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// Whether a debounced save is armed (assertion surface —
    /// proves a change to the REBUILT dock entity still reaches the save
    /// policy through the re-attached observer).
    #[cfg(test)]
    pub fn save_pending(&self) -> bool {
        self.policy.pending()
    }

    /// Whether a cmd-r force-reload is pending on the shared flag (assertion
    /// surface).
    #[cfg(test)]
    pub fn reload_requested(&self) -> bool {
        self.reload_trigger.load(Ordering::Acquire)
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
        // The grammar overlay: a focus-capturing modal
        // OUTSIDE the workspace key-context, so no bare verb fires underneath it.
        let overlay_body = match self.overlay {
            Overlay::Closed => None,
            Overlay::Help => Some(self.help_overlay_body()),
            Overlay::Jump => Some(self.jump_overlay_body(cx)),
            Overlay::Palette => Some(self.palette_overlay_body(cx)),
            Overlay::Arg => Some(self.arg_overlay_body(cx)),
        };
        let overlay = overlay_body.map(|body| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x0a0d1466))
                .track_focus(&self.overlay_focus)
                .on_key_down(cx.listener(Self::on_overlay_key))
                .child(body)
        });
        div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            // menu-move actions: this root div is an ancestor of
            // every dispatch path in the window, so the tab-menu actions
            // land here regardless of where focus sat at click time.
            .on_action(cx.listener(Self::on_dock_editor_at_bottom))
            .on_action(cx.listener(Self::on_dock_editor_at_right))
            .on_action(cx.listener(Self::on_dock_sidebar_at_bottom))
            .on_action(cx.listener(Self::on_dock_sidebar_at_left))
            // The global focus toggle (cmd-e) + the overlay openers (?, /) land
            // here too — bubbling from the focused canvas.
            .on_action(cx.listener(Self::on_toggle_focus))
            .on_action(cx.listener(Self::on_open_help))
            .on_action(cx.listener(Self::on_focus_jump))
            .on_action(cx.listener(Self::on_open_palette))
            // Command-log argument verbs (a/e) + commit (cmd-s on the canvas):
            // bubble from the focused canvas to this root.
            .on_action(cx.listener(Self::on_add_mark))
            .on_action(cx.listener(Self::on_set_channel))
            .on_action(cx.listener(Self::on_commit))
            // cmd-r (global, ungated): flips the shared force-reload flag.
            .on_action(cx.listener(Self::on_reload))
            .child(self.dock_area.clone())
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
            .children(overlay)
    }
}

// ---------------------------------------------------------------------------
// Reload-rejection notifications
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
/// feedback `log`: the Log panel keeps the history the toasts
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

    /// Shim: the canvas panel HOLDS the ChartView entity (same
    /// entity id — delegation, not a copy), carries the stable panel name
    /// and the resolved dashboard title, refuses closing, and stays
    /// visible in BOTH presentation modes (the canvas remains). Together
    /// with the untouched chart_view/element sources (the diff gate),
    /// that is the whole shim contract: hosting added, nothing transformed.
    #[gpui::test]
    fn canvas_panel_is_a_shim_around_the_chart_view(cx: &mut TestAppContext) {
        let (chart, presentation, panel) = cx.update(|cx| {
            let chart = cx.new(|_| {
                ChartView::new(320.0, 240.0, Vec::new(), Vec::new(), Vec::new(), Vec::new())
            });
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            let panel = cx.new(|cx| {
                CanvasPanel::new(
                    chart.clone(),
                    "Flight Delays",
                    presentation.clone(),
                    FocusTree::empty(),
                    cx,
                )
            });
            (chart, presentation, panel)
        });

        cx.update(|cx| {
            let p = panel.read(cx);
            assert_eq!(
                p.panel_name(),
                CANVAS_PANEL_NAME,
                "stable serialisation name"
            );
            assert!(!p.closable(cx), "the canvas is locked");
            assert_eq!(
                p.title_text().as_ref(),
                "Flight Delays",
                "resolved dashboard title"
            );
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
                "the canvas REMAINS under presentation"
            );
        });
    }

    /// Shim: the sidebar panel hosts the computed profiles
    /// verbatim — profiled/failed/unsupported alike — the set-profiles refresh
    /// tap replaces them, and it hides under presentation per the mapping.
    #[gpui::test]
    fn sidebar_panel_hosts_profiles_and_hides_in_presentation(cx: &mut TestAppContext) {
        use profile_model::ColumnProfile;

        let profiles = vec![
            SourceProfile {
                name: "flights".to_string(),
                outcome: ProfileOutcome::Profiled {
                    row_count: 231_083,
                    columns: vec![
                        ColumnProfile {
                            name: "delay".to_string(),
                            type_name: "INTEGER".to_string(),
                            non_null: 231_080,
                            nulls: 3,
                            distinct: 1_400,
                            min: Some("-99".to_string()),
                            max: Some("1439".to_string()),
                        },
                        ColumnProfile {
                            name: "origin".to_string(),
                            type_name: "VARCHAR".to_string(),
                            non_null: 231_083,
                            nulls: 0,
                            distinct: 322,
                            min: None,
                            max: None,
                        },
                    ],
                },
            },
            SourceProfile {
                name: "warehouse".to_string(),
                outcome: ProfileOutcome::Unsupported,
            },
            SourceProfile {
                name: "broken".to_string(),
                outcome: ProfileOutcome::Failed("IO Error: No files found".to_string()),
            },
        ];
        let (presentation, panel) = cx.update(|cx| {
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            let panel = cx.new(|cx| SidebarPanel::new(profiles.clone(), presentation.clone(), cx));
            (presentation, panel)
        });

        cx.update(|cx| {
            let p = panel.read(cx);
            assert_eq!(p.panel_name(), SIDEBAR_PANEL_NAME);
            assert_eq!(p.profiles(), &profiles[..], "hosts the profiles verbatim");
            // The failed and unsupported variants ride through as rows.
            assert!(matches!(
                p.profiles()[1].outcome,
                ProfileOutcome::Unsupported
            ));
            assert!(matches!(p.profiles()[2].outcome, ProfileOutcome::Failed(_)));
            assert!(p.visible(cx));
        });

        // The refresh tap swaps in a fresh set (the hot-reload path).
        let refreshed = vec![SourceProfile {
            name: "only".to_string(),
            outcome: ProfileOutcome::Profiled {
                row_count: 1,
                columns: vec![],
            },
        }];
        cx.update(|cx| {
            panel.update(cx, |p, cx| p.set_profiles(refreshed.clone(), cx));
        });
        cx.update(|cx| {
            assert_eq!(
                panel.read(cx).profiles(),
                &refreshed[..],
                "refresh replaced the set"
            );
        });

        cx.update(|cx| {
            presentation.update(cx, |state, _| state.mode = PresentationMode::Presentation);
        });
        cx.update(|cx| {
            assert!(!panel.read(cx).visible(cx), "authoring panels hide");
        });
    }

    /// Empty state: a zero-source spec renders without panicking —
    /// the existing empty-state placeholder survives.
    #[gpui::test]
    fn sidebar_panel_handles_zero_sources(cx: &mut TestAppContext) {
        let (_presentation, panel) = cx.update(|cx| {
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            let panel = cx.new(|cx| SidebarPanel::new(Vec::new(), presentation.clone(), cx));
            (presentation, panel)
        });
        cx.update(|cx| {
            assert!(panel.read(cx).profiles().is_empty(), "no sources hosted");
            assert!(panel.read(cx).visible(cx));
        });
    }

    /// Shim: the Log panel carries the stable panel name, hosts
    /// the SAME FeedbackLog entity, is permanent (closable=false — it
    /// anchors the bottom dock), never zoomable, and follows the authoring
    /// chrome: visible while authoring, hidden under presentation.
    #[gpui::test]
    fn log_panel_is_a_permanent_shim_over_the_feedback_log(cx: &mut TestAppContext) {
        let (feedback_log, presentation, panel) = cx.update(|cx| {
            let feedback_log = cx.new(|_| FeedbackLog::default());
            let presentation = cx.new(|_| PresentationState {
                mode: PresentationMode::default(),
            });
            let panel = cx.new(|cx| LogPanel::new(feedback_log.clone(), presentation.clone(), cx));
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

    /// Reload-feedback tap: driving the SAME path the watcher
    /// uses — `notify_reload_rejection` — lands the identical severity +
    /// message pair in the feedback log that the notification carried
    /// (both come from the one reload_feedback decision).
    #[gpui::test]
    fn reload_rejection_reaches_the_log_with_the_notification_message(cx: &mut TestAppContext) {
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
        notify_reload_rejection(
            &window,
            &mut async_cx,
            severity,
            message.clone(),
            &feedback_log,
        );
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
    /// editor-save tap tests.
    fn build_editor_shell(
        cx: &mut TestAppContext,
        spec_path: PathBuf,
        seed: Option<&str>,
    ) -> (
        gpui::WindowHandle<Root>,
        Entity<EditorPanel>,
        Entity<FeedbackLog>,
    ) {
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
    fn drive_save(
        cx: &mut TestAppContext,
        window: gpui::WindowHandle<Root>,
        editor: &Entity<EditorPanel>,
    ) {
        let editor = editor.clone();
        cx.update_window(window.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| editor.save(&SaveSpec, window, cx));
        })
        .expect("drive cmd-s");
        cx.run_until_parked();
    }

    /// Editor-save tap, refusal arm: a refused save (unseeded
    /// editor) appends the notification's exact message to the log at
    /// error severity.
    #[gpui::test]
    fn editor_save_refusal_reaches_the_log(cx: &mut TestAppContext) {
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

    /// Editor-save tap, conflict arm: an external
    /// change on disk under a dirty buffer defers the save with a warning,
    /// and the log receives the same warning message the toast carried.
    #[gpui::test]
    fn editor_save_conflict_reaches_the_log(cx: &mut TestAppContext) {
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
            assert_eq!(
                log.entries()[0].severity,
                Severity::Warning,
                "warning, like the toast"
            );
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

    /// Editor-save tap, write-failure arm: a save
    /// whose atomic write fails (the spec path is a DIRECTORY, so the
    /// final rename cannot land) logs the same "Save failed" error the
    /// toast carried.
    #[gpui::test]
    fn editor_save_write_failure_reaches_the_log(cx: &mut TestAppContext) {
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
            assert_eq!(
                log.entries()[0].severity,
                Severity::Error,
                "error, like the toast"
            );
            assert!(
                log.entries()[0].message.starts_with("Save failed: "),
                "the toast's message shape: {}",
                log.entries()[0].message
            );
        });
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No-clear-on-recovery: recovery clears the
    /// sticky error NOTIFICATION but never the log — driven through the
    /// same notify_reload_rejection + clear_reload_error pair the
    /// watcher's rejection and recovery arms use.
    #[gpui::test]
    fn recovery_clears_the_notification_not_the_log(cx: &mut TestAppContext) {
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
        notify_reload_rejection(
            &window,
            &mut async_cx,
            severity,
            message.clone(),
            &feedback_log,
        );
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
        assert!(reload_feedback::clears_errors(
            &reload_feedback::ReloadOutcome::Applied
        ));
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

    /// Binding data: the editor keymap is one binding — cmd-s →
    /// SaveSpec — scoped to the editor context, mirroring the shape
    /// for the workspace `p` binding.
    #[test]
    fn cmd_s_binds_save_spec_in_editor_context() {
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
        assert_eq!(
            binding.match_keystrokes(std::slice::from_ref(&plain_s)),
            None
        );

        assert_eq!(binding.action().name(), "brightfield::SaveSpec");
        assert!(
            binding.predicate().is_some(),
            "the binding is editor-scoped, not global"
        );
    }

    // -----------------------------------------------------------------------
    // Bottom dock seed/backfill + presentation round trip,
    // against the real WorkspaceRoot in a test window.
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
    // The `Rc<RefCell<Option<(Entity<_>, Entity<_>)>>>` slot below is the plain
    // spelling of "a cell two gpui closures write their handles into". Naming it
    // via a `type` would hide the shape that makes the pattern legible, in a
    // test helper with one call site for it.
    #[allow(clippy::type_complexity)]
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
            let chart = cx.new(|_| {
                ChartView::new(320.0, 240.0, Vec::new(), Vec::new(), Vec::new(), Vec::new())
            });
            let canvas = cx.new(|cx| {
                CanvasPanel::new(
                    chart,
                    "Test",
                    presentation_in.clone(),
                    FocusTree::empty(),
                    cx,
                )
            });
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
            let command_log_model = cx.new(|_| crate::command_log::CommandLog::new());
            let command_log_panel =
                cx.new(|cx| CommandLogPanel::new(command_log_model, presentation_in.clone(), cx));
            let workspace = cx.new(|cx| {
                WorkspaceRoot::with_saved_layout(
                    canvas,
                    editor.clone(),
                    sidebar,
                    log_panel,
                    command_log_panel,
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

    /// cmd-r (`on_reload`) flips the shared force-reload flag the
    /// watcher polls — the whole end-to-end reload is macOS-eyeball, but the
    /// handler's flag flip is headless. Ungated by presentation by design.
    #[gpui::test]
    fn cmd_r_flips_the_shared_reload_trigger(cx: &mut TestAppContext) {
        let shell = build_shell(cx, None);
        assert!(
            !cx.update(|cx| shell.workspace.read(cx).reload_requested()),
            "no force-reload pending at rest"
        );
        cx.update_window(shell.window.into(), |_, window, cx| {
            shell
                .workspace
                .update(cx, |w, cx| w.on_reload(&ReloadSpec, window, cx));
        })
        .expect("drive cmd-r");
        assert!(
            cx.update(|cx| shell.workspace.read(cx).reload_requested()),
            "cmd-r set the shared force-reload flag"
        );
    }

    /// Flip the shared presentation mode (the same entity notify the canvas
    /// `p` handler produces) and flush the observers.
    fn toggle_presentation_mode(cx: &mut TestAppContext, presentation: &Entity<PresentationState>) {
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
        for placement in [
            DockPlacement::Left,
            DockPlacement::Right,
            DockPlacement::Bottom,
        ] {
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

    /// Fresh boot: no saved layout → the default layout seeds
    /// the bottom dock CLOSED with the Log panel at the default height,
    /// and every dock item is stack-rooted (review F1: bare-Tabs roots
    /// render no drop targets and cannot source drags at this pin).
    #[gpui::test]
    fn fresh_boot_seeds_closed_bottom_log_dock(cx: &mut TestAppContext) {
        let shell = build_shell(cx, None);

        let (has, open, size) = bottom_dock_state(cx, &shell.workspace);
        assert!(has, "fresh boot has a bottom dock");
        assert_eq!(
            open,
            Some(false),
            "seeded CLOSED — the strip is the affordance"
        );
        assert_eq!(
            size,
            Some(px(BOTTOM_DOCK_HEIGHT as f32)),
            "default open height"
        );
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string(), CMD_LOG_PANEL_NAME.to_string()],
            "the Log + Commands panels anchor it"
        );
        assert_all_roots_stack_rooted(cx, &shell.workspace);
    }

    /// Backfill + normalise: restoring a pre-round layout (no
    /// bottom dock, bare-Tabs roots, NON-default sizes/open bits) appends
    /// the same closed Log dock post-load and re-roots every dock under a
    /// StackPanel, while the author's arrangement — a 300px right dock, a
    /// 250px CLOSED left dock — survives exactly. DOCK_STATE_VERSION is
    /// unchanged — the layout restores, it is not discarded.
    #[gpui::test]
    fn pre_round_layout_is_backfilled_with_closed_bottom_dock(cx: &mut TestAppContext) {
        let raw = serde_json::to_string_pretty(&saved_layout_without_bottom()).unwrap();
        let shell = build_shell(cx, Some(raw));

        // The restored arrangement survived, at values default_layout could
        // not have produced (review F3): right 300px (default 380), left
        // 250px and CLOSED (default 220, open).
        cx.update(|cx| {
            let area = shell.workspace.read(cx).dock_area().read(cx);
            let right = area.right_dock().expect("restored right dock").read(cx);
            assert_eq!(
                right.size(),
                px(300.0),
                "right width restored, not defaulted"
            );
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
            vec![LOG_PANEL_NAME.to_string(), CMD_LOG_PANEL_NAME.to_string()]
        );
        // …and every restored bare-Tabs root was normalised (review F1).
        assert_all_roots_stack_rooted(cx, &shell.workspace);
    }

    /// Already present: a saved layout that carries a bottom
    /// dock restores exactly as saved — its open state and size are kept,
    /// and no second dock (or forced state) is introduced. Its bare-Tabs
    /// root is normalised to the stack-rooted shape (arrangement, not tree
    /// shape, is what "as saved" pins).
    #[gpui::test]
    fn saved_bottom_dock_restores_as_saved(cx: &mut TestAppContext) {
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
        assert_eq!(
            open,
            Some(true),
            "saved OPEN state kept — not forced closed"
        );
        assert_eq!(size, Some(px(240.0)), "saved height kept — not the default");
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![LOG_PANEL_NAME.to_string()],
            "one Log panel — no double dock, no duplicate seed"
        );
        assert_all_roots_stack_rooted(cx, &shell.workspace);
    }

    /// Closed-before: entering presentation removes the bottom
    /// dock ENTIRELY (no 29px strip — there is no dock to paint one);
    /// exiting rebuilds it closed at the same size with the Log panel.
    #[gpui::test]
    fn round_trip_preserves_a_closed_bottom_dock(cx: &mut TestAppContext) {
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
            vec![LOG_PANEL_NAME.to_string(), CMD_LOG_PANEL_NAME.to_string()]
        );
        assert!(
            dock_root_is_stack_rooted(cx, &shell.workspace, DockPlacement::Bottom),
            "the rebuild round-trips the stack-rooted shape (review F1c)"
        );
    }

    /// Open-before, moved-in panel: a bottom dock the author
    /// opened, resized, and moved the editor into comes back exactly —
    /// open, same size, both panels — and a change to the REBUILT dock
    /// entity still schedules a layout save (the observer re-attached).
    #[gpui::test]
    fn round_trip_preserves_open_dock_with_moved_in_panel(cx: &mut TestAppContext) {
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
            vec![
                LOG_PANEL_NAME.to_string(),
                CMD_LOG_PANEL_NAME.to_string(),
                EDITOR_PANEL_NAME.to_string()
            ],
            "editor moved into the bottom dock beside Log + Commands"
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
            vec![
                LOG_PANEL_NAME.to_string(),
                CMD_LOG_PANEL_NAME.to_string(),
                EDITOR_PANEL_NAME.to_string()
            ],
            "contents preserved, including the moved-in editor"
        );
        assert!(
            dock_root_is_stack_rooted(cx, &shell.workspace, DockPlacement::Bottom),
            "the rebuild round-trips the stack-rooted shape (review F1c)"
        );

        // The save observer re-attached to the NEW dock entity: an isolated
        // post-round-trip Dock-entity change (resize) reaches the policy.
        shell
            .workspace
            .update(cx, |workspace, _| workspace.reset_save_probe());
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

        // And a PANEL-TREE change (review F2, the correction):
        // removing a panel from the rebuilt dock's TabPanel must reach the
        // policy through their StackPanel-mediated PanelEvent → DockEvent
        // chain — wiring that only exists because the rebuild is
        // stack-rooted (their subscribe_item skips bare-Tabs items, and
        // nothing else observes the TabPanel). Resize alone no longer
        // counts: it rides the direct dock-entity observer above.
        shell
            .workspace
            .update(cx, |workspace, _| workspace.reset_save_probe());
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
            vec![LOG_PANEL_NAME.to_string(), CMD_LOG_PANEL_NAME.to_string()],
            "the panel-tree change landed"
        );
        cx.update(|cx| {
            assert!(
                shell.workspace.read(cx).save_pending(),
                "a panel-tree change to the rebuilt dock schedules a save (F2)"
            );
        });
    }

    /// Drive the move handler (the body of the menu actions)
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

    /// Menu-move handler: the bootstrap that makes the drag
    /// gestures reachable — moving the editor to the bottom dock joins the
    /// Log panel's tab set (two tabs = real drag sources) and closes the
    /// emptied right dock; moving it back restores an open, stack-rooted
    /// right dock at its previous width and leaves the Log alone below.
    #[gpui::test]
    fn menu_move_editor_to_bottom_and_back(cx: &mut TestAppContext) {
        let shell = build_shell(cx, None);
        let editor: std::sync::Arc<dyn gpui_component::dock::PanelView> =
            std::sync::Arc::new(shell.editor.clone());

        // "Dock at Bottom": the editor joins the Log tab set, the
        // destination opens, and the emptied source dock closes.
        move_panel(cx, &shell, editor.clone(), DockPlacement::Bottom);
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![
                LOG_PANEL_NAME.to_string(),
                CMD_LOG_PANEL_NAME.to_string(),
                EDITOR_PANEL_NAME.to_string()
            ],
            "editor docked at the bottom beside the Log + Commands"
        );
        assert_eq!(
            bottom_dock_state(cx, &shell.workspace).1,
            Some(true),
            "the destination dock opened (a move behind the closed strip would be invisible)"
        );
        let right = dock_at(cx, &shell.workspace, DockPlacement::Right).expect("right dock");
        cx.update(|cx| {
            assert!(!right.read(cx).is_open(), "the emptied source dock closed");
        });
        assert!(
            dock_panel_names(cx, &shell.workspace, DockPlacement::Right).is_empty(),
            "the editor left the right dock"
        );

        // Idempotent: repeating the move re-lands the editor in place.
        move_panel(cx, &shell, editor.clone(), DockPlacement::Bottom);
        assert_eq!(
            bottom_dock_panel_names(cx, &shell.workspace),
            vec![
                LOG_PANEL_NAME.to_string(),
                CMD_LOG_PANEL_NAME.to_string(),
                EDITOR_PANEL_NAME.to_string()
            ],
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
            vec![LOG_PANEL_NAME.to_string(), CMD_LOG_PANEL_NAME.to_string()],
            "the Log + Commands remain the bottom dock's anchors"
        );
        assert_eq!(
            bottom_dock_state(cx, &shell.workspace).1,
            Some(true),
            "the bottom dock was not emptied, so it stays open"
        );
    }
}
