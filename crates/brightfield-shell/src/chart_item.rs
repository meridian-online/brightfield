//! The chart pane: **one** [`Item`] implementation, parameterised by mark
//! kind.
//!
//! # Three shells, one implementation
//!
//! The retired gpui side hosted a chart through three separate `Element`
//! shells — a canvas surface, a legend element, a slider element — each with
//! its own framework glue and its own idea of what "selected" looked like.
//! This module is their egui replacement expressed the other way round: one `ChartItem`
//! whose behaviour is a *function of the mark kind* it presents. The kind is
//! data ([`ChartDoc`] carries it, per plot, off the composition that actually
//! happened), so a dot plot, a bar chart and an area chart are one type with
//! one draw path and one gesture router — `mark_icon` and `gesture_for` are
//! total over [`MarkKind`], and a new mark kind is a new match arm, not a new
//! shell.
//!
//! # Mark kinds and chart kinds are two vocabularies, not one
//!
//! They are named alike and they are different lists, so the distinction is
//! stated here rather than left to be inferred. A [`MarkKind`] is what a plot
//! draws with — `dot`, `rectY`, `cell` — and it comes off the composition. A
//! **chart kind** is an entry in [`crate::chart_kinds::registry`]: an id, an
//! icon, the column slots it takes, and a builder that turns bound columns
//! into a whole spec. The two lists are declared apart — a `MarkKind` is a
//! variant of an enum in `brightfield_spec`, a chart kind's id is a string
//! literal in [`crate::chart_kinds`] — and neither is derived from the other.
//! This pane is parameterised by the first and draws a document the second
//! chose through that kind's `ChartModule`; see `module_of`.
//!
//! # One selection treatment
//!
//! Everything transient the pointer does to the chart is painted from the
//! design system's **overlay** token group (`brush_fill` / `brush_border` /
//! `focus_ring` — the "never in the data scene" inks), and keyboard focus
//! anywhere on the chart surfaces is `meridian-egui`'s one `focus_ring`. The
//! five treatments this retired were the gpui shells': the brush overlay's own
//! constants, the legend element's selected-entry dim, its hover lighten, the
//! slider's focus treatment, and the workbench's second focus ring.
//!
//! # Gestures are queries
//!
//! A brush or a click never filters pixels or batches here. It resolves —
//! through the plot's *displayed* scales — to a **structured** predicate
//! ([`SqlPredicate::Interval`] / [`SqlPredicate::Point`]) and goes through
//! [`ChartDoc::apply_interaction`] into the coordinator seam: the engine
//! pushes it into DuckDB and the affected marks re-execute. The structured
//! clause variants are preferred over flattened strings deliberately — they
//! render byte-identical SQL, and keep the column, bounds and gesture context
//! machine-readable downstream.

use brightfield_engine::coordinator::Interaction;
use brightfield_engine::SqlPredicate;
use brightfield_keys::BindingContext;
use brightfield_render::canvas_host::{Color, OverlayPainter, SurfaceCursor};
use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_spec::analysis::BrushKind;
use brightfield_spec::vocab::MarkKind;
use brightfield_sql::ir::ScalarValue;
use brightfield_workbench::item::{ChartModule, ModuleHost};
use brightfield_workbench::subject::RunState;
use brightfield_workbench::{
    chrome, Affordance, EmptyState, Icon, Item, ItemCtx, ItemId, Subject, ToolbarEntry,
    ToolbarLocation, Verb,
};
use meridian_design::chrome::{OverlayTokens, INK_DARK, INK_LIGHT, OVERLAY_DARK, OVERLAY_LIGHT};
use meridian_design::semantic::Role;

use crate::app::{ChartDoc, CHART};
use crate::canvas::{set_surface_cursor, surface_input, EguiOverlay};
use crate::design::Mode;
use crate::legend;
use crate::navigation::{self, verb::RESET_EXTENT};
use crate::pipeline::{GestureBinding, PlotHandle};
use crate::starts;

/// The predicate readout's status-entry id — the handle a headless test asserts
/// the readout by, and the name the rail records in [`chrome::StatusDrawn`]
/// when it draws the line.
///
/// **Not a replacement key.** Nothing dedups status entries by id:
/// [`chrome::status_rail`] draws every entry it is handed, in order. A redrawn
/// brush cannot stack a second readout beside the first because [`Subject`] is
/// rebuilt from scratch each time [`Item::subject`] is called — it takes
/// `&self` and `&ChartDoc`, starts from an empty [`Subject`], and adds this
/// entry at most once — so the rail's contents are a function of the document,
/// not an accumulation over frames. What the id buys is that a headless test
/// can pick this line out of the several the same rail carries without matching
/// on its text. Nothing in production reads it: dismissal routes on the
/// entry's [`brightfield_workbench::subject::Verb`], never on this.
pub const PREDICATE_READOUT: &str = "chart-predicate";

/// How many logical pixels of wheel travel double the visible span.
///
/// It is a GESTURE-SHAPE constant, not a timing one: it converts the wheel's
/// own units into a zoom factor, the way a scroll surface converts travel into
/// distance. Ln-2 scaled, so travelling this far in and then back out returns
/// the frame to exactly where it started rather than somewhere near it.
const WHEEL_ZOOM_PIXELS: f64 = 180.0 / std::f64::consts::LN_2;

/// This frame's wheel travel in logical pixels, summed from the raw events.
///
/// Line and page units are converted through egui's own configured speeds, so
/// a wheel that reports lines and a trackpad that reports points agree about
/// how far the hand moved.
fn wheel_travel(ctx: &egui::Context) -> f64 {
    let per_line = ctx.options(|o| f64::from(o.input_options.line_scroll_speed));
    // A page is a screenful. egui has no constant for one, so it is expressed
    // in lines rather than invented in pixels — twenty of them, the usual
    // terminal page.
    let per_page = per_line * 20.0;
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => Some(match unit {
                    egui::MouseWheelUnit::Point => f64::from(delta.y),
                    egui::MouseWheelUnit::Line => f64::from(delta.y) * per_line,
                    egui::MouseWheelUnit::Page => f64::from(delta.y) * per_page,
                }),
                _ => None,
            })
            .sum()
    })
}

/// The mode's overlay tokens — the transient-gesture ink group.
fn overlay_tokens(mode: Mode) -> &'static OverlayTokens {
    match mode {
        Mode::Light => &OVERLAY_LIGHT,
        Mode::Dark => &OVERLAY_DARK,
    }
}

/// The icon a mark kind wears, from the Meridian icon set — the visible half
/// of "parameterised by mark kind": one function, total over the vocabulary,
/// instead of one shell per chart shape.
#[must_use]
pub fn mark_icon(kind: Option<MarkKind>) -> Icon {
    let name = match kind {
        Some(
            MarkKind::BarY
            | MarkKind::BarX
            | MarkKind::Rect
            | MarkKind::RectX
            | MarkKind::RectY
            | MarkKind::Cell,
        ) => "chart-bar",
        Some(MarkKind::AreaY | MarkKind::AreaX) => "chart-area",
        Some(MarkKind::Line) => "chart-line",
        // Dots, densities, rasters, geo — everything mark-shaped without a
        // closer glyph presents as the dot chart.
        Some(_) | None => "chart-dots",
    };
    Icon(name)
}

/// Which gesture class a brush binding drives on a plot of this mark kind —
/// the behavioural half of the parameterisation. Interval brushes sweep a
/// range on continuous axes; point toggles pick a category off a band axis.
/// The *spec's* interactor decides which was asked for; the mark kind decides
/// nothing extra today, and taking it as a parameter anyway is what keeps
/// this the one place a kind-specific gesture rule would land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureClass {
    /// Drag sweeps an interval (x, y, or both).
    Interval,
    /// Click toggles a categorical member.
    Point,
}

/// The gesture class of a brush kind. Total, so a new interactor kind is a
/// compile error here rather than a silent dead gesture.
#[must_use]
pub fn gesture_for(kind: BrushKind) -> GestureClass {
    match kind {
        BrushKind::IntervalX | BrushKind::IntervalY | BrushKind::IntervalXY => {
            GestureClass::Interval
        }
        BrushKind::Point | BrushKind::PointX | BrushKind::PointY => GestureClass::Point,
    }
}

// ---------------------------------------------------------------------------
// The run-state pill — the shell's render of the one status vocabulary.
// ---------------------------------------------------------------------------

/// The icon each [`RunState`] wears. Icon and label always travel together
/// (the pill draws both), so colour is never the only signal — and two states
/// never share an icon, so the icon alone is not a lie either.
#[must_use]
pub fn run_state_icon(state: RunState) -> meridian_egui::Icon {
    let name = match state {
        RunState::NeverRun => "clock",
        RunState::Fresh => "circle-check",
        RunState::StaleOwnEdit => "refresh",
        RunState::StaleUpstream => "alert-triangle",
        RunState::Failed => "circle-x",
    };
    meridian_egui::Icon::by_name(name).unwrap_or_else(|| {
        unreachable!("run-state icon {name:?} is in the Meridian icon set — pinned by test")
    })
}

/// The design-system role each [`RunState`] resolves to — the reconciliation
/// of the old gpui emitter's ad-hoc feedback roles and the viz status inks
/// into ONE mapping, routed through the state's own [`Tone`]: fresh wears
/// success, both stales wear warning, failure wears danger, and never-run
/// stays neutral — the absence of a run, not a good or bad one.
///
/// [`Tone`]: brightfield_workbench::subject::Tone
#[must_use]
pub fn run_state_role(state: RunState) -> Role {
    use brightfield_workbench::subject::Tone;
    match state.tone() {
        Tone::Good => Role::Success,
        Tone::Warning => Role::Warning,
        Tone::Critical => Role::Danger,
        Tone::Accent => Role::Accent,
        Tone::Neutral => Role::Neutral,
    }
}

/// The run-state pill: `meridian-egui`'s `status_pill` speaking the
/// [`RunState`] vocabulary — the workbench contract's five states, never a
/// second parallel set. Words from [`RunState::label`], colour from
/// [`run_state_role`], icon from [`run_state_icon`]; the gloss rides the
/// hover.
pub fn run_state_pill(ui: &mut egui::Ui, state: RunState) -> egui::Response {
    let icon = run_state_icon(state);
    meridian_egui::widgets::status_pill(ui, &icon, state.label(), run_state_role(state))
        .on_hover_text(state.gloss())
}

// ---------------------------------------------------------------------------
// The item.
// ---------------------------------------------------------------------------

/// An in-progress brush drag, in raster-local logical pixels — view-local
/// state, which is the only state an [`Item`] may hold. The *committed*
/// selection lives where it belongs: in the engine, as a pushed predicate.
#[derive(Clone, Copy, Debug)]
struct Drag {
    /// The plot index the drag started in — a drag never crosses plots.
    plot: usize,
    /// Where the primary button went down.
    start: kurbo::Point,
    /// The pointer now.
    current: kurbo::Point,
}

/// The chart pane. See the module docs for what this one type replaces.
pub struct ChartItem {
    drag: Option<Drag>,
    /// Whether the primary button was down over the raster last frame — the
    /// edge detector the drag state machine runs on.
    was_down: bool,
    /// The plot a secondary-button pan is being dragged on, and where the
    /// pointer was last frame. `None` when no pan is in progress.
    ///
    /// A pan is the SECONDARY button on purpose: the primary drag is the brush,
    /// and one button cannot mean both "select these rows" and "move the frame"
    /// without a mode nobody can see.
    pan: Option<(usize, kurbo::Point)>,
    /// Whether the secondary button was down last frame — the pan's edge
    /// detector, and its settle: the release is the gesture's end.
    was_secondary_down: bool,
    /// Whether a wheel zoom was still arriving last frame. A frame with no
    /// wheel delta after one that had some IS the gesture's end — the settle
    /// test for a gesture with no button to let go of.
    was_scrolling: bool,
}

impl ChartItem {
    /// A chart pane with no gesture in progress.
    #[must_use]
    pub fn new() -> Self {
        Self {
            drag: None,
            was_down: false,
            pan: None,
            was_secondary_down: false,
            was_scrolling: false,
        }
    }

    /// The pane's toolbar, declared once and read twice — `describe` puts it
    /// on the subject (the auditable declaration) and `ui` draws the same
    /// list through the chrome's collapsing `Toolbar`.
    ///
    /// One control today: `clear-selection`. On a dashboard with no brushable
    /// plot it is **`Hidden`** — still declared, so the vocabulary stays
    /// greppable and the verb stays audited, while the row itself disappears.
    /// That is the quiet-when-nothing-to-show mechanism, not a convention.
    ///
    /// Visibility is keyed on the *composition* (does any plot declare a
    /// gesture?), not on liveness, so the pane's geometry is a pure function
    /// of [`Composed`](crate::pipeline::Composed) — which is what lets
    /// [`chart_window_size`](crate::window::chart_window_size) budget the row
    /// before any frame exists. Whether the control can *act* right now is
    /// `enabled`, which is where "can" belongs.
    fn toolbar_entries(doc: &ChartDoc) -> Vec<ToolbarEntry> {
        let bindable = doc.composed.plots.iter().any(|p| p.gesture.is_some());
        let mut entry = ToolbarEntry::button(
            "chart-clear-selection",
            "Clear selection",
            Verb::new("clear-selection"),
        )
        .enabled(doc.selection_active());
        entry.tooltip = Some("Retract every committed brush and re-query".to_string());
        if !bindable {
            entry = entry.at(ToolbarLocation::Hidden);
        }

        // The navigation reset. Declared beside the selection clear and kept
        // separate from it on purpose: a brush and a frame are different state,
        // and one control that undid both would make a zoom impossible to keep
        // while working a cross-filter.
        let navigated = doc.navigated();
        let mut reset =
            ToolbarEntry::button("chart-reset-extent", "Reset view", Verb::new(RESET_EXTENT))
                .enabled(navigated);
        reset.tooltip = Some("Return every plot to its full extent".to_string());
        if !navigated {
            // Quiet when there is nothing to undo: a chart nobody has
            // navigated has no view to reset, and a permanently greyed button
            // on every chart is chrome that means nothing.
            reset = reset.at(ToolbarLocation::Hidden);
        }
        vec![entry, reset]
    }
    /// The pointer gesture machine: the brush drag, the pan, the wheel zoom
    /// and the settled navigation's one re-query.
    ///
    /// **Lifted out of the texture branch on purpose.** It used to sit inside
    /// the arm that runs only when a wgpu device is behind the document, so a
    /// GPU-free window laid the raster out and then returned before any gesture
    /// was read — every pointer gesture on the chart was unreachable to a
    /// headless test, including the brush that shipped before this. Nothing in
    /// here needs the frame: the gesture reads input and writes the document,
    /// and only the transient overlay needs somewhere to paint.
    ///
    /// Returns whether the caller should ask for another frame.
    fn drive_gestures(
        &mut self,
        doc: &mut ChartDoc,
        ctx: &egui::Context,
        rect: egui::Rect,
    ) -> (bool, GestureFrame) {
        let mut repaint = false;
        // Gestures and the transient overlay, before the legend band so
        // the frame borrow ends inside this scope.
        let input = surface_input(ctx, rect);
        let hovered = input.hovered;
        let pointer = input.pointer_pos;
        let down = matches!(
            input.pointer_primary,
            brightfield_render::canvas_host::ButtonState::Down
        );

        // The drag state machine: press starts a brush in the plot under
        // the pointer, release commits it. Edge-triggered on the button.
        if down && !self.was_down {
            if let Some(p) = pointer {
                if let Some(plot) = plot_at(&doc.composed.plots, p) {
                    if doc.composed.plots[plot].gesture.is_some() && doc.is_live() {
                        self.drag = Some(Drag {
                            plot,
                            start: p,
                            current: p,
                        });
                    }
                }
            }
        } else if down {
            if let (Some(drag), Some(p)) = (self.drag.as_mut(), pointer) {
                drag.current = p;
            }
        }
        let released = !down && self.was_down;
        self.was_down = down;

        // ---------------------------------------------------------------
        // Navigation: the frame moves on every sample, the data re-queries
        // once the gesture has ended. See `crate::navigation`.
        // ---------------------------------------------------------------
        let secondary_down = matches!(
            input.pointer_secondary,
            brightfield_render::canvas_host::ButtonState::Down
        );
        if doc.is_live() {
            // A secondary-button drag pans the plot it started on. The
            // delta is measured against the pointer's own previous
            // position rather than against the gesture's origin, so each
            // step moves the frame by exactly what the hand moved.
            if secondary_down && !self.was_secondary_down {
                self.pan =
                    pointer.and_then(|p| plot_at(&doc.composed.plots, p).map(|plot| (plot, p)));
            } else if secondary_down {
                if let (Some((plot, last)), Some(p)) = (self.pan, pointer) {
                    let lock = doc.axis_lock;
                    let outcome = doc.composed.plots.get(plot).map(|handle| {
                        navigation::pan(&handle.scales, lock, p.x - last.x, p.y - last.y)
                    });
                    if let Some(outcome) = outcome {
                        doc.note_navigation(plot, &outcome);
                    }
                    self.pan = Some((plot, p));
                }
            }
            if !secondary_down && self.was_secondary_down {
                // Release IS the settle.
                self.pan = None;
                doc.settle_navigation();
            }

            // The wheel zooms about the pointer. Read from THIS frame's wheel
            // EVENTS rather than from the smoothed delta, and both halves of
            // that matter.
            //
            // The magnitude has to be the travel the hand actually produced:
            // the exponent below turns pixels into a multiplicative factor, so
            // zooming in and back out by the same travel returns to exactly the
            // frame you started from — a property a smoothed, lagging delta
            // does not have.
            //
            // And the SETTLE has to be a fact about the input. The smoothed
            // delta decays across frames after the wheel stops (measured: 47 →
            // 32 with no further travel), so "the delta is zero" is not the end
            // of a gesture, it is some frames after it. A frame carrying no
            // wheel event is.
            let scroll = wheel_travel(ctx);
            let scrolling = scroll.abs() > f64::EPSILON;
            if scrolling {
                if let Some(p) = pointer {
                    if let Some(plot) = plot_at(&doc.composed.plots, p) {
                        let lock = doc.axis_lock;
                        let outcome = doc.composed.plots.get(plot).map(|handle| {
                            let local = (p.x - handle.rect.x, p.y - handle.rect.y);
                            navigation::zoom(
                                &handle.scales,
                                lock,
                                Some(local),
                                (scroll / WHEEL_ZOOM_PIXELS).exp(),
                            )
                        });
                        if let Some(outcome) = outcome {
                            doc.note_navigation(plot, &outcome);
                        }
                    }
                }
            } else if self.was_scrolling {
                // A frame with no wheel travel after one that had some is
                // the end of the gesture — no clock in it.
                doc.settle_navigation();
            }
            self.was_scrolling = scrolling;
        }
        self.was_secondary_down = secondary_down;
        // Release: resolve the gesture to a structured predicate and push
        // it through the seam. A click (no sweep) on an interval binding
        // clears that plot's contribution instead — the crossfilter
        // convention — and on a point binding toggles the category.
        //
        // **This take now happens BEFORE the overlay is painted**, where it used
        // to happen after — the one shipped-rendering consequence of lifting
        // the gesture machine out of the texture branch, and a deliberate
        // choice rather than an accident of the move. The brush rectangle is
        // the transient picture of an UNCOMMITTED sweep; the frame the button
        // comes up on is the frame the sweep stops being a sweep and becomes a
        // selection. Painting it once more would draw a control that has
        // already been resolved, over a raster that is about to be replaced by
        // the result of resolving it. It costs one frame of ink (~16 ms) and no
        // golden covers it, which is why it has to be said here instead.
        if doc.pump_navigation() {
            repaint = true;
        }

        if released {
            if let Some(drag) = self.drag.take() {
                let plot = &doc.composed.plots[drag.plot];
                if let Some(binding) = plot.gesture.clone() {
                    if let Some(interaction) = resolve_gesture(&binding, plot, drag) {
                        doc.apply_interaction(interaction);
                        repaint = true;
                    }
                }
            }
        }
        (repaint, GestureFrame { hovered, pointer })
    }
}

/// The chart module this document's picture is drawn as, or `None` when no
/// chart kind chose it and none is missing.
///
/// **Rebuilt from the document every frame, not held.** A
/// [`ChartModule`]'s own state is the kind it names, the columns bound to it
/// and the state of the controls that kind hangs on itself — and every kind
/// this build ships declares no control, so all three are a function of the
/// document. `no_kind_declares_a_control_that_the_pane_would_have_to_remember`
/// in [`crate::chart_kinds`] is what says so, and it is the test that will
/// redden on the first kind that needs this pane to hold its module across
/// frames instead.
///
/// `None` also when the document's kind is not in this build's registry, which
/// is a different answer from "no kind chose this picture" and is why
/// [`Item::empty_state`] says which of the two happened rather than letting the
/// pane draw a header and silence.
fn module_of(doc: &ChartDoc) -> Option<ChartModule> {
    let authored = doc.authored()?;
    let kind = doc.chart_kinds().find(authored.kind)?;
    Some(ChartModule::new(
        CHART,
        doc.title().to_string(),
        kind,
        authored.fields.clone(),
    ))
}

/// What the gesture machine leaves for the overlay to draw with.
struct GestureFrame {
    /// Whether the pointer is over the raster.
    hovered: bool,
    /// Where it is, in raster-local logical pixels.
    pointer: Option<kurbo::Point>,
}

impl Default for ChartItem {
    fn default() -> Self {
        Self::new()
    }
}

impl Item<ChartDoc> for ChartItem {
    fn item_id(&self) -> ItemId {
        CHART
    }

    /// The chart view's **front door**.
    ///
    /// This empty state is what a launch with no spec on the command line
    /// opens on, so its prose cannot assume a spec was ever named — the copy
    /// it replaces opened "This spec composed no plots", which is a report
    /// about a spec that does not exist. It names the one way in: a shipped
    /// start, offered as a button.
    ///
    /// The affordance is an [`Action::Open`](brightfield_workbench::Action)
    /// rather than a verb. There is no registered command that means "open the
    /// example dashboard" and inventing one would put a keystroke and a palette
    /// entry behind a fixture; worse, the chrome renders an affordance's verb's
    /// real keystroke next to its label, so borrowing an unrelated verb would
    /// ship a button that claims a key it does not have.
    fn empty_state(&self, doc: &ChartDoc) -> Option<EmptyState> {
        if !doc.is_empty() {
            // A picture a chart kind chose is drawn by that kind's module, so
            // a build whose registry no longer has the kind has nothing to
            // draw it with. Saying which kind is missing is the answer the
            // module itself would give; the pane gives it because a module
            // cannot be built without the kind it names.
            let missing = doc
                .authored()
                .filter(|a| doc.chart_kinds().find(a.kind).is_none());
            return missing.map(|authored| {
                EmptyState::new(
                    mark_icon(doc.primary_mark()),
                    "This build has no chart of that kind",
                    format!(
                        "The picture was drawn as {}, which is not in this \
                         build's chart registry.",
                        authored.kind
                    ),
                )
            });
        }
        let mut empty = EmptyState::new(
            mark_icon(None),
            "Nothing to draw",
            "No spec is open, or the one that is composed no plots. Start \
             from the example below.",
        );
        if let Some(start) = starts::for_pane(crate::app::CHART) {
            empty = empty.with_next(Affordance::open(start.label, start.id));
        }
        Some(empty)
    }

    fn describe(&self, doc: &ChartDoc) -> Subject {
        let mut subject = Subject::new(
            "Chart",
            mark_icon(doc.primary_mark()),
            BindingContext::Workspace,
        );
        for entry in Self::toolbar_entries(doc) {
            subject = subject.with_toolbar(entry);
        }
        if let Some(state) = doc.composed.run_state {
            // The one vocabulary, spelled by its own type — the same entry a
            // protocol surface rails, so a stale preview here and a stale
            // step there say it identically.
            //
            // **This pane owns the line, and the data grid does not**, though
            // both draw the same document's materialised output and both draw
            // `run_state_pill` in their own body. The rail collects the status
            // lines of each placed pane, so a document-level fact declared by
            // two panes of one view is drawn twice; one of them has to own it.
            //
            // It is this one because the chart is `Slot::Centre` with no
            // toggle verb, while the grid is a `Slot::CentreTab` the
            // `toggle-data-grid` verb closes. `ItemRegistry::new` rejects a
            // view that does not have exactly one `Slot::Centre`, so the
            // centre pane is in the tree whatever else is; `ItemSpec::toggle`
            // records why that is the slot allowed to carry no verb. The test
            // `chart_contract.rs::the_pane_that_owns_the_run_state_line_cannot_be_closed`
            // holds both halves. Had the grid owned the line, closing the Data tab would
            // take the honesty label off the rail while the chart went on
            // drawing rows from the same run — a label the user can dismiss by
            // closing an unrelated tab is worse than one that was never there,
            // because its absence reads as "nothing to report".
            //
            // The same reasoning already put the document's activity and
            // watcher entries here, below.
            subject = subject.with_status(state.status_entry(RunState::RAIL_ID));
        }
        // The document's in-flight work and file notices, reported here
        // because this pane is the view's presenting surface: activity in
        // the typed entries the shell's one indicator collects, and the
        // watcher's facts about files — which are conditions beside the
        // run-state entry above, never a re-spelling of it.
        for entry in doc.activity.entries() {
            subject = subject.with_status(entry);
        }
        for entry in doc.watch.entries() {
            subject = subject.with_status(entry);
        }
        // What a navigation gesture refused to do, and why. A categorical axis
        // cannot be panned or zoomed — there is no continuous range to move —
        // and a control that simply did nothing would read as broken. The
        // refusal is stated instead.
        if let Some(text) = doc.nav_notice() {
            subject = subject.with_status(brightfield_workbench::subject::StatusEntry {
                id: "chart-navigation",
                side: brightfield_workbench::subject::StatusSide::Trailing,
                text: text.to_string(),
                tone: brightfield_workbench::subject::Tone::Neutral,
                hide: brightfield_workbench::subject::HideAffordance::WithRail,
            });
        }
        // What the frame did NOT scope. A second entry rather than a second
        // vocabulary: the same rail, the same `StatusEntry`, and deliberately
        // not folded into the refusal above, because the two differ in what
        // they are about and in how long they last. The refusal is about one
        // gesture and stops being said when the next one succeeds; this is about
        // the extent currently in force and stands until it is reset.
        //
        // The two ids differ because the two lines do, not to keep them from
        // colliding: sharing one id would draw BOTH, one after the other under a
        // single name, since nothing dedups by id and the rail draws every entry
        // it is handed. What the separate ids buy is that a test — and the rail's
        // own `StatusDrawn` record — can tell which of the two is on screen.
        //
        // `Warning`, not `Neutral`: a declining mark is not an inert control,
        // it is a drawn quantitative claim about rows the reader cannot see.
        if let Some(text) = doc.nav_scope_notice() {
            subject = subject.with_status(brightfield_workbench::subject::StatusEntry {
                id: "chart-navigation-scope",
                side: brightfield_workbench::subject::StatusSide::Trailing,
                text,
                tone: brightfield_workbench::subject::Tone::Warning,
                hide: brightfield_workbench::subject::HideAffordance::WithRail,
            });
        }
        // **The predicate readout** — the SQL the gestures on this chart are
        // holding, said out loud instead of only executed. A chart that can
        // report how many rows are selected but never the condition that
        // selected them is asking to be trusted; this is the answer to "what
        // am I looking at", and it is the condition itself, byte for byte.
        //
        // Leading, not trailing: it is what the reader came for, not a notice
        // about the frame, and the trailing end is where the navigation
        // entries and the run state stand. The wording — `$name = clause`
        // rather than "Filter:" — is argued at [`ChartDoc::selection_sql`],
        // which is the only place that judgement should have to be made.
        //
        // Dismissed by `clear-selection`, the verb that actually retracts it.
        // `WithRail` would offer to hide a line while the state it describes
        // went on filtering the picture, which is the one dismissal a readout
        // like this must not have.
        if let Some(text) = doc.selection_sql() {
            subject = subject.with_status(brightfield_workbench::subject::StatusEntry {
                id: PREDICATE_READOUT,
                side: brightfield_workbench::subject::StatusSide::Leading,
                text,
                tone: brightfield_workbench::subject::Tone::Neutral,
                hide: brightfield_workbench::subject::HideAffordance::Verb(Verb::new(
                    "clear-selection",
                )),
            });
        }
        subject
    }

    fn ui(&mut self, doc: &mut ChartDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        // Recorded *before* the texture check, so a headless document still
        // reports the box the dock gave this pane. See `ChartDoc::viewport`.
        doc.viewport = Some(ui.max_rect());
        let mode = cx.mode;

        // The toolbar row — through the collapsing Toolbar, so a document
        // with nothing to offer draws no row at all rather than a blank band.
        let entries = Self::toolbar_entries(doc);
        let drawn = chrome::Toolbar::new(&entries).show(ui, mode);
        for verb in drawn.activated {
            cx.request(verb);
        }

        // The run-state banner, only when this preview shows materialised run
        // output at all: icon + label + tone, never colour alone. A live or
        // one-shot composition claims nothing and draws nothing.
        if let Some(state) = doc.composed.run_state {
            run_state_pill(ui, state);
        }

        // The dashboard is laid out into the room this pane has left, so the
        // raster below is a re-layout of the chart at the pane's size rather
        // than a fixed picture the window was sized around. The legend band is
        // drawn beside the raster rather than inside it, so it comes off the
        // offer — [`legend::band_width`] is the whole of what the band takes,
        // gap included, and it is the same term `chart_window_size` uses.
        let reserved = legend::band_width(&doc.composed);
        let room = ui.available_size();
        // A changed composition needs a frame to be rastered in:
        // [`ChartDoc::present`] runs at the top of `MeridianApp::draw`, before
        // the dock reaches this pane, so the texture painted below this call is
        // the previous composition stretched into the new rect. The gesture
        // paths further down act on the same signal for the same reason.
        if doc.reflow_to(egui::vec2(room.x - reserved, room.y)) {
            cx.request_repaint();
        }

        let h = doc.composed.height;
        let band = legend::band_width(&doc.composed);

        // The raster and, when any plot calls for one, the legend band beside
        // it — OUTSIDE the plot rect, in the chart's margin, by layout rather
        // than by hope.
        let overlay_on = doc.overlay;
        let ctx = ui.ctx().clone();
        let textured = doc.canvas_texture().is_some();
        let mut legend_rect = None;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;

            // **The raster, drawn through the chart-kind registry.** A picture
            // a chart kind chose is drawn by that kind's `ChartModule`: the
            // module resolves its kind out of the document's registry, builds
            // the spec from the columns bound to it, and hands that spec back
            // to the document — which is where the composer and the canvas
            // host live. A document carrying no `Authored` record has no kind
            // to build a module from (see `Authored` for which routes record
            // one) and presents directly.
            //
            // `raster_rect` is cleared first because the module route can draw
            // nothing at all — a kind this build no longer has, columns that
            // no longer fit — and a rect left standing from the last frame
            // would aim this frame's gestures at a raster that is not there.
            doc.raster_rect = None;
            let rect = match module_of(doc) {
                Some(mut module) => {
                    Item::ui(&mut module, doc, ui, cx);
                    // The module reserved its raster inside the child `Ui`
                    // `module_frame` hands it, and a child's allocations do not
                    // move the parent's cursor. Unadvanced, the legend band
                    // below would be laid out at the raster's own x and sit on
                    // the data — which is the one thing the band's whole
                    // geometry exists to prevent.
                    if let Some(rect) = doc.raster_rect {
                        ui.advance_cursor_after_rect(rect);
                    }
                    doc.raster_rect
                }
                None => doc.present_raster(ui),
            };
            let Some(rect) = rect else {
                return;
            };

            // Same gestures with a device and without one. The overlay is the
            // only thing a headless document loses: it has nowhere to paint.
            let (repaint, gesture) = self.drive_gestures(doc, &ctx, rect);
            if repaint {
                cx.request_repaint();
            }
            let (hovered, pointer) = (gesture.hovered, gesture.pointer);

            // The one transient-gesture treatment: the overlay token group.
            // `drive_gestures` above has already taken a released drag, so the
            // rectangle is gone on the release frame rather than one frame
            // later — see the note at that take.
            if textured {
                if let Some(drag) = self.drag {
                    let tokens = overlay_tokens(mode);
                    let r = drag_rect(&doc.composed.plots[drag.plot], drag);
                    let mut painter = EguiOverlay::new(ui, rect);
                    painter.fill_rect(r, Color::from_token(tokens.brush_fill));
                    painter.stroke_rect(r, Color::from_token(tokens.brush_border), 1.0);
                } else if overlay_on && hovered {
                    // The hover crosshair — the chart's own ink layer, matched
                    // to the raster's palette rather than the chrome's, and
                    // bounded by the plot the pointer is in. See
                    // `crosshair_segments`.
                    if let Some(p) = pointer {
                        if let Some(segments) = crosshair_segments(&doc.composed.plots, p) {
                            let focus = match mode {
                                Mode::Light => INK_LIGHT.focus,
                                Mode::Dark => INK_DARK.focus,
                            };
                            let ink = Color::from_token_alpha(focus, 0.9);
                            let mut painter = EguiOverlay::new(ui, rect);
                            for (a, b) in segments {
                                painter.line(a, b, ink, 1.0);
                            }
                            painter.fill_circle(p, 3.0, ink);
                        }
                    }
                    set_surface_cursor(ui.ctx(), SurfaceCursor::Grab);
                }
            }

            // The legend band, drawn from the same scales the raster was
            // composed against. The gap is allocated space, so the band's
            // rect and the raster's are disjoint by geometry, not by paint.
            if band > 0.0 {
                ui.add_space(meridian_design::spacing::CONTROL_GAP);
                let (band_rect, _) = ui.allocate_exact_size(
                    egui::vec2(legend::block_width(), h as f32),
                    egui::Sense::hover(),
                );
                legend_rect = Some(band_rect);
                if textured {
                    legend::draw_band(ui, band_rect, rect.top(), &doc.composed, mode);
                }
            }
        });
        doc.legend_rect = legend_rect;
    }
}

// ---------------------------------------------------------------------------
// Gesture geometry — pure, and unit-tested as such.
// ---------------------------------------------------------------------------

/// The index of the plot whose placed rect contains `p` (raster-local
/// logical pixels), if any.
fn plot_at(plots: &[PlotHandle], p: kurbo::Point) -> Option<usize> {
    plots.iter().position(|plot| {
        p.x >= plot.rect.x
            && p.x <= plot.rect.x + plot.rect.width
            && p.y >= plot.rect.y
            && p.y <= plot.rect.y + plot.rect.height
    })
}

/// The two segments a hover crosshair draws at `p` (raster-local logical
/// pixels): the vertical one first, then the horizontal one. Each spans the
/// placed rect of the plot **under the pointer** and stops there.
///
/// `None` when the pointer is over the raster but over no plot. A crosshair
/// reads out a position within a plot, so there is nothing to read out there.
///
/// A dashboard is one chart document rendered to one raster, so the plot rect
/// — not the raster — is the extent a reader can attribute a line to.
fn crosshair_segments(
    plots: &[PlotHandle],
    p: kurbo::Point,
) -> Option<[(kurbo::Point, kurbo::Point); 2]> {
    let rect = plots.get(plot_at(plots, p)?)?.rect;
    Some([
        (
            kurbo::Point::new(p.x, rect.y),
            kurbo::Point::new(p.x, rect.y + rect.height),
        ),
        (
            kurbo::Point::new(rect.x, p.y),
            kurbo::Point::new(rect.x + rect.width, p.y),
        ),
    ])
}

/// The brush rectangle a drag paints, clamped to its plot and axis-locked to
/// the binding's brush kind (an x-interval sweeps full plot height, a
/// y-interval full width).
fn drag_rect(plot: &PlotHandle, drag: Drag) -> brightfield_render::canvas_host::SurfaceRect {
    use brightfield_render::canvas_host::SurfaceRect;
    let kind = plot.gesture.as_ref().map(|g| g.kind);
    let (x0, x1) = min_max(drag.start.x, drag.current.x);
    let (y0, y1) = min_max(drag.start.y, drag.current.y);
    let (px0, px1) = (plot.rect.x, plot.rect.x + plot.rect.width);
    let (py0, py1) = (plot.rect.y, plot.rect.y + plot.rect.height);
    let (x0, x1, y0, y1) = match kind {
        Some(BrushKind::IntervalX | BrushKind::PointX) => (x0.max(px0), x1.min(px1), py0, py1),
        Some(BrushKind::IntervalY | BrushKind::PointY) => (px0, px1, y0.max(py0), y1.min(py1)),
        _ => (x0.max(px0), x1.min(px1), y0.max(py0), y1.min(py1)),
    };
    SurfaceRect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

fn min_max(a: f64, b: f64) -> (f64, f64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Resolve a finished drag to the interaction it means, or `None` when it
/// means nothing (no channel column, a degenerate scale).
///
/// - An **interval** sweep becomes [`SqlPredicate::Interval`] per swept axis
///   (both, `And`-ed, for `intervalXY`), bounds inverted through the plot's
///   displayed scales.
/// - An interval **click** (no sweep) retracts this plot's contribution —
///   the click-clears convention.
/// - A **point** click becomes [`SqlPredicate::Point`] over the category the
///   band scale places under the pointer.
fn resolve_gesture(binding: &GestureBinding, plot: &PlotHandle, drag: Drag) -> Option<Interaction> {
    /// A drag shorter than this on both axes is a click, not a sweep.
    const CLICK_SLOP: f64 = 3.0;
    let swept = (drag.current.x - drag.start.x).abs() > CLICK_SLOP
        || (drag.current.y - drag.start.y).abs() > CLICK_SLOP;
    match gesture_for(binding.kind) {
        GestureClass::Interval => {
            if !swept {
                return Some(Interaction::ClearSelect {
                    name: binding.selection.clone(),
                    contributor: binding.contributor.clone(),
                });
            }
            let predicate = interval_predicate(binding, plot, drag.start, drag.current)?;
            Some(Interaction::Select {
                name: binding.selection.clone(),
                contributor: binding.contributor.clone(),
                predicate,
            })
        }
        GestureClass::Point => {
            if swept {
                return None;
            }
            let predicate = point_predicate(binding, plot, drag.current)?;
            Some(Interaction::Select {
                name: binding.selection.clone(),
                contributor: binding.contributor.clone(),
                predicate,
            })
        }
    }
}

/// The structured interval clause(s) a sweep from `a` to `b` (raster-local)
/// means on this plot: per-axis `Interval`s, `And`-ed when the binding
/// sweeps both.
fn interval_predicate(
    binding: &GestureBinding,
    plot: &PlotHandle,
    a: kurbo::Point,
    b: kurbo::Point,
) -> Option<SqlPredicate> {
    let mut clauses = Vec::new();
    if matches!(binding.kind, BrushKind::IntervalX | BrushKind::IntervalXY) {
        let column = binding.x_column.as_deref()?;
        let scale = plot.scales.get(Channel::X)?;
        clauses.push(axis_interval(
            column,
            scale,
            a.x - plot.rect.x,
            b.x - plot.rect.x,
        )?);
    }
    if matches!(binding.kind, BrushKind::IntervalY | BrushKind::IntervalXY) {
        let column = binding.y_column.as_deref()?;
        let scale = plot.scales.get(Channel::Y)?;
        clauses.push(axis_interval(
            column,
            scale,
            a.y - plot.rect.y,
            b.y - plot.rect.y,
        )?);
    }
    match clauses.len() {
        0 => None,
        1 => clauses.pop(),
        _ => Some(SqlPredicate::And(clauses)),
    }
}

/// One axis's structured interval: two plot-local pixels inverted through the
/// displayed scale, ordered into inclusive `[lo, hi]` bounds.
///
/// The clause names the column as a SQL **identifier** rather than by the
/// spelling the binding carries — see [`crate::sql_ident`], and
/// `point_predicate` below for the failure that made it matter. Both clause
/// producers on this path quote, because a file column named with a space is as
/// legal in an interval as in a point.
fn axis_interval(column: &str, scale: &Scale, p0: f64, p1: f64) -> Option<SqlPredicate> {
    let (v0, v1) = (scale.inverse_f64(p0)?, scale.inverse_f64(p1)?);
    let (lo, hi) = min_max(v0, v1);
    let bound = |v: f64| match scale {
        Scale::Time { .. } => ScalarValue::TimestampMicros(v.round() as i64),
        _ => ScalarValue::Float(v),
    };
    Some(SqlPredicate::Interval {
        column: crate::sql_ident::quote(column),
        lo: bound(lo),
        hi: bound(hi),
        meta: None,
    })
}

/// The structured point clause a click at `p` (raster-local) means: the
/// category whose band slot contains the pointer, on the binding's axis.
///
/// The column is written as a SQL identifier. A [`SqlPredicate::Point`] renders
/// its column verbatim, and the names reaching here are file column names and
/// [`crate::resample`]'s bucket columns — which always carry spaces
/// (`observed by hour`), so the unquoted form was a parser error that took
/// every OTHER tile's query down with it. The spaces are the ` by ` a bucket
/// name is built around, pinned by the test
/// `a_derived_name_steps_aside_for_a_column_that_owns_it`, and the gesture that
/// takes the whole page down without the quoting is
/// `a_click_on_a_timestamp_tile_leaves_its_sibling_tiles_on_the_page` in
/// `tests/data_file.rs`.
fn point_predicate(
    binding: &GestureBinding,
    plot: &PlotHandle,
    p: kurbo::Point,
) -> Option<SqlPredicate> {
    let (column, channel, pixel) = match binding.kind {
        BrushKind::PointY => (binding.y_column.clone()?, Channel::Y, p.y - plot.rect.y),
        _ => (binding.x_column.clone()?, Channel::X, p.x - plot.rect.x),
    };
    let category = band_category(plot.scales.get(channel)?, pixel)?;
    Some(SqlPredicate::Point {
        column: crate::sql_ident::quote(&column),
        values: vec![ScalarValue::Text(category)],
        meta: None,
    })
}

/// The category whose band slot contains `pixel`, on a band scale.
fn band_category(scale: &Scale, pixel: f64) -> Option<String> {
    let Scale::Band {
        categories,
        range_start,
        range_end,
        ..
    } = scale
    else {
        return None;
    };
    if categories.is_empty() {
        return None;
    }
    let span = range_end - range_start;
    if span.abs() < f64::EPSILON {
        return None;
    }
    let slot = span / categories.len() as f64;
    let idx = ((pixel - range_start) / slot).floor();
    if idx < 0.0 || idx >= categories.len() as f64 {
        return None;
    }
    Some(categories[idx as usize].clone())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_render::layout::ChartLayout;
    use brightfield_render::scale::ScaleSet;
    use brightfield_spec::analysis::ComponentPath;
    use brightfield_spec::layout::Rect;

    fn linear(range: (f64, f64), domain: (f64, f64)) -> Scale {
        Scale::Linear {
            domain_min: domain.0,
            domain_max: domain.1,
            range_start: range.0,
            range_end: range.1,
        }
    }

    fn plot(scales: ScaleSet, kind: BrushKind) -> PlotHandle {
        PlotHandle {
            path: "root".to_string(),
            rect: Rect::new(0.0, 0.0, 100.0, 100.0),
            scales,
            layout: ChartLayout::new(100.0, 100.0),
            marks: vec![MarkKind::Dot],
            gesture: Some(GestureBinding {
                selection: "brush".to_string(),
                contributor: ComponentPath("root".to_string()),
                kind,
                x_column: Some("x".to_string()),
                y_column: Some("y".to_string()),
            }),
            x_column: Some("x".to_string()),
            y_column: Some("y".to_string()),
            sample: None,
        }
    }

    /// The wave constraint in one assertion: a sweep resolves to the
    /// STRUCTURED interval clause — column, typed bounds — not a flattened
    /// string, and it renders exactly the SQL its hand-written form would.
    #[test]
    fn an_x_sweep_resolves_to_a_structured_interval_clause() {
        let mut scales = ScaleSet::new();
        scales.insert(Channel::X, linear((0.0, 100.0), (0.0, 10.0)));
        let plot = plot(scales, BrushKind::IntervalX);
        let binding = plot.gesture.clone().expect("bound");
        let predicate = interval_predicate(
            &binding,
            &plot,
            kurbo::Point::new(20.0, 5.0),
            kurbo::Point::new(80.0, 60.0),
        )
        .expect("a sweep inverts");
        let SqlPredicate::Interval { column, lo, hi, .. } = &predicate else {
            panic!("expected the structured Interval variant, got {predicate:?}");
        };
        assert_eq!(
            column, "\"x\"",
            "the clause names the column as an identifier"
        );
        assert_eq!(*lo, ScalarValue::Float(2.0));
        assert_eq!(*hi, ScalarValue::Float(8.0));
        // The structured clause renders byte-identically to the string form.
        assert_eq!(predicate.to_string(), "(\"x\" >= 2 AND \"x\" <= 8)");
    }

    /// A reversed drag still yields ordered inclusive bounds.
    #[test]
    fn a_right_to_left_sweep_orders_its_bounds() {
        let mut scales = ScaleSet::new();
        scales.insert(Channel::X, linear((0.0, 100.0), (0.0, 10.0)));
        let plot = plot(scales, BrushKind::IntervalX);
        let binding = plot.gesture.clone().expect("bound");
        let SqlPredicate::Interval { lo, hi, .. } = interval_predicate(
            &binding,
            &plot,
            kurbo::Point::new(80.0, 0.0),
            kurbo::Point::new(20.0, 0.0),
        )
        .expect("inverts") else {
            panic!("structured clause");
        };
        assert_eq!(lo, ScalarValue::Float(2.0));
        assert_eq!(hi, ScalarValue::Float(8.0));
    }

    /// A 2D sweep is one `And` of two structured intervals — the y axis
    /// inverted through the y scale's flipped pixel range.
    #[test]
    fn an_xy_sweep_ands_two_intervals() {
        let mut scales = ScaleSet::new();
        scales.insert(Channel::X, linear((0.0, 100.0), (0.0, 10.0)));
        // y pixel range runs top-down: pixel 0 is the domain max.
        scales.insert(Channel::Y, linear((100.0, 0.0), (0.0, 50.0)));
        let plot = plot(scales, BrushKind::IntervalXY);
        let binding = plot.gesture.clone().expect("bound");
        let predicate = interval_predicate(
            &binding,
            &plot,
            kurbo::Point::new(10.0, 10.0),
            kurbo::Point::new(90.0, 90.0),
        )
        .expect("inverts");
        let SqlPredicate::And(clauses) = &predicate else {
            panic!("an XY sweep is a conjunction, got {predicate:?}");
        };
        assert_eq!(clauses.len(), 2);
        let SqlPredicate::Interval { column, lo, hi, .. } = &clauses[1] else {
            panic!("y clause is structured");
        };
        assert_eq!(column, "\"y\"");
        // Pixels 10 and 90 on a flipped 100→0 range are domain 45 and 5.
        let (ScalarValue::Float(lo), ScalarValue::Float(hi)) = (lo, hi) else {
            panic!("linear bounds are floats");
        };
        assert!((lo - 5.0).abs() < 1e-9, "lo inverted to {lo}");
        assert!((hi - 45.0).abs() < 1e-9, "hi inverted to {hi}");
    }

    /// A click on a band axis resolves the category under the pointer to a
    /// structured Point clause — a quoted equality, not a between.
    #[test]
    fn a_band_click_resolves_to_a_structured_point_clause() {
        let mut scales = ScaleSet::new();
        scales.insert(
            Channel::X,
            Scale::Band {
                categories: vec!["North".into(), "South".into(), "East".into()],
                range_start: 0.0,
                range_end: 90.0,
                padding: 0.1,
            },
        );
        let plot = plot(scales, BrushKind::PointX);
        let binding = plot.gesture.clone().expect("bound");
        let predicate = point_predicate(&binding, &plot, kurbo::Point::new(45.0, 50.0))
            .expect("the middle band is under the pointer");
        assert_eq!(
            predicate,
            SqlPredicate::Point {
                column: "\"x\"".to_string(),
                values: vec![ScalarValue::Text("South".to_string())],
                meta: None,
            }
        );
        assert_eq!(predicate.to_string(), "\"x\" = 'South'");
    }

    /// A click outside every band resolves to nothing rather than to the
    /// nearest category — a miss is a miss.
    #[test]
    fn a_click_off_the_band_axis_selects_nothing() {
        let scale = Scale::Band {
            categories: vec!["a".into(), "b".into()],
            range_start: 10.0,
            range_end: 50.0,
            padding: 0.0,
        };
        assert_eq!(band_category(&scale, 5.0), None);
        assert_eq!(band_category(&scale, 55.0), None);
        assert_eq!(band_category(&scale, 15.0), Some("a".to_string()));
    }

    /// An interval click (no sweep) retracts the plot's contribution — the
    /// click-clears convention — and a point sweep does nothing at all.
    #[test]
    fn clicks_and_sweeps_route_by_gesture_class() {
        let mut scales = ScaleSet::new();
        scales.insert(Channel::X, linear((0.0, 100.0), (0.0, 10.0)));
        let interval = plot(scales.clone(), BrushKind::IntervalX);
        let click = Drag {
            plot: 0,
            start: kurbo::Point::new(40.0, 40.0),
            current: kurbo::Point::new(41.0, 41.0),
        };
        let binding = interval.gesture.clone().expect("bound");
        assert!(matches!(
            resolve_gesture(&binding, &interval, click),
            Some(Interaction::ClearSelect { .. })
        ));

        let point = plot(scales, BrushKind::PointX);
        let sweep = Drag {
            plot: 0,
            start: kurbo::Point::new(10.0, 10.0),
            current: kurbo::Point::new(90.0, 90.0),
        };
        let binding = point.gesture.clone().expect("bound");
        assert_eq!(resolve_gesture(&binding, &point, sweep), None);
    }

    /// Two plots placed side by side on one raster, as a root `hconcat` of
    /// two places them: same height, edge-adjacent, the first at the raster's
    /// origin.
    fn two_placed_plots() -> Vec<PlotHandle> {
        let mut left = plot(ScaleSet::new(), BrushKind::IntervalX);
        left.rect = Rect::new(0.0, 0.0, 360.0, 300.0);
        let mut right = plot(ScaleSet::new(), BrushKind::IntervalX);
        right.path = "root/hconcat[1]".to_string();
        right.rect = Rect::new(360.0, 0.0, 360.0, 300.0);
        vec![left, right]
    }

    /// The crosshair spans the plot the pointer is in, and stops at its edges.
    #[test]
    fn a_crosshair_spans_the_hovered_plot_and_no_more() {
        let plots = two_placed_plots();
        let at = kurbo::Point::new(100.0, 70.0);
        let [(v0, v1), (h0, h1)] =
            crosshair_segments(&plots, at).expect("the pointer is on a plot");

        // Vertical: held at the pointer's x, spanning the plot's own height.
        assert_eq!((v0.x, v1.x), (100.0, 100.0));
        assert_eq!((v0.y, v1.y), (0.0, 300.0));
        // Horizontal: held at the pointer's y, spanning the plot's own width.
        assert_eq!((h0.y, h1.y), (70.0, 70.0));
        assert_eq!((h0.x, h1.x), (0.0, 360.0));
    }

    /// The neighbour is untouched: a segment drawn for a pointer on one plot
    /// stays out of the other's interior. These two share a boundary, so an
    /// endpoint landing on it is placement; ink past it is the sighting this
    /// fix answers — one pointer, crosshairs on both plots of a two-plot
    /// dashboard.
    #[test]
    fn a_crosshair_stays_out_of_the_neighbouring_plots_interior() {
        let plots = two_placed_plots();
        for (hovered, neighbour) in [(0usize, 1usize), (1, 0)] {
            let r = plots[hovered].rect;
            let at = kurbo::Point::new(r.x + r.width / 2.0, r.y + r.height / 2.0);
            let segments = crosshair_segments(&plots, at).expect("the pointer is on a plot");
            let n = plots[neighbour].rect;
            for (a, b) in segments {
                let (x0, x1) = min_max(a.x, b.x);
                let (y0, y1) = min_max(a.y, b.y);
                assert!(
                    x1 <= n.x || x0 >= n.x + n.width || y1 <= n.y || y0 >= n.y + n.height,
                    "a segment drawn for plot {hovered} runs {a:?} to {b:?}, \
                     through plot {neighbour}'s interior"
                );
            }
        }
    }

    /// A pointer the plot rects do not contain draws no crosshair, rather than
    /// one attributed to whichever plot comes first.
    #[test]
    fn a_pointer_on_no_plot_draws_no_crosshair() {
        let plots = two_placed_plots();
        assert_eq!(
            crosshair_segments(&plots, kurbo::Point::new(800.0, 150.0)),
            None
        );
        assert_eq!(
            crosshair_segments(&plots, kurbo::Point::new(100.0, 400.0)),
            None
        );
        assert_eq!(
            crosshair_segments(&[], kurbo::Point::new(100.0, 70.0)),
            None
        );

        // A ragged `vconcat`: the row takes the widest child's width, so the
        // area beside a narrower plot is on the raster and on no plot.
        let mut wide = plot(ScaleSet::new(), BrushKind::IntervalX);
        wide.path = "root/vconcat[0]".to_string();
        wide.rect = Rect::new(0.0, 0.0, 720.0, 300.0);
        let mut narrow = plot(ScaleSet::new(), BrushKind::IntervalX);
        narrow.path = "root/vconcat[1]".to_string();
        narrow.rect = Rect::new(0.0, 300.0, 360.0, 300.0);
        assert_eq!(
            crosshair_segments(&[wide, narrow], kurbo::Point::new(540.0, 450.0)),
            None
        );
    }

    /// One implementation, every mark kind: the parameterisation is total —
    /// each implemented kind resolves an icon from the Meridian set, so a
    /// dot, a bar and an area chart are match arms, not shells.
    #[test]
    fn the_mark_kind_parameterisation_is_total_over_the_vocabulary() {
        for kind in [
            MarkKind::Dot,
            MarkKind::BarY,
            MarkKind::BarX,
            MarkKind::AreaY,
            MarkKind::AreaX,
            MarkKind::Line,
            MarkKind::Rect,
            MarkKind::Cell,
        ] {
            let icon = mark_icon(Some(kind));
            assert!(
                meridian_egui::Icon::by_name(icon.as_str()).is_some(),
                "{kind:?} resolves to {:?}, which is not in the icon set",
                icon.as_str()
            );
        }
        assert_eq!(mark_icon(None).as_str(), "chart-dots");
    }

    /// Every brush kind routes to exactly one gesture class — a new
    /// interactor kind must choose here before it can compile.
    #[test]
    fn every_brush_kind_has_a_gesture_class() {
        for (kind, class) in [
            (BrushKind::IntervalX, GestureClass::Interval),
            (BrushKind::IntervalY, GestureClass::Interval),
            (BrushKind::IntervalXY, GestureClass::Interval),
            (BrushKind::Point, GestureClass::Point),
            (BrushKind::PointX, GestureClass::Point),
            (BrushKind::PointY, GestureClass::Point),
        ] {
            assert_eq!(gesture_for(kind), class);
        }
    }

    // -- The run-state pill: the one vocabulary, rendered -------------------

    /// The pill speaks the workbench vocabulary and nothing else: five
    /// states, five distinct icons that all exist in the icon set, and the
    /// role mapping is the tone reconciliation — never-run is neutral, not
    /// an error and not success.
    #[test]
    fn the_run_state_pill_consumes_the_one_vocabulary() {
        let mut seen = std::collections::BTreeSet::new();
        for state in RunState::ALL {
            let icon = run_state_icon(state);
            assert!(seen.insert(format!("{icon:?}")), "{state:?} shares an icon");
        }
        assert_eq!(run_state_role(RunState::Fresh), Role::Success);
        assert_eq!(run_state_role(RunState::StaleOwnEdit), Role::Warning);
        assert_eq!(run_state_role(RunState::StaleUpstream), Role::Warning);
        assert_eq!(run_state_role(RunState::Failed), Role::Danger);
        assert_eq!(run_state_role(RunState::NeverRun), Role::Neutral);
    }

    /// The toolbar declaration follows the document: hidden (row disappears)
    /// with nothing to act on, offered-but-disabled with a live bindable
    /// dashboard and no committed brush. Declared either way, so the verb
    /// stays greppable and audited.
    #[test]
    fn the_clear_selection_control_is_hidden_exactly_when_it_cannot_act() {
        let doc = ChartDoc::empty();
        let entries = ChartItem::toolbar_entries(&doc);
        // Both controls are declared — the vocabulary stays greppable — and
        // both are hidden on a dashboard with nothing to brush and no plot to
        // navigate.
        assert_eq!(entries.len(), 2, "both controls stay declared");
        assert!(
            entries
                .iter()
                .all(|e| e.location == ToolbarLocation::Hidden),
            "{entries:#?}"
        );
        assert!(
            !chrome::Toolbar::new(&entries).has_something_to_say(),
            "a hidden-only toolbar summons no row"
        );
    }
}
