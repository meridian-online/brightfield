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
use brightfield_render::canvas_host::{ChartSurface, Color, PixelSize, SurfaceCursor};
use brightfield_render::channel::Channel;
use brightfield_render::scale::Scale;
use brightfield_spec::analysis::BrushKind;
use brightfield_spec::vocab::MarkKind;
use brightfield_sql::ir::ScalarValue;
use brightfield_workbench::subject::RunState;
use brightfield_workbench::{
    chrome, Affordance, EmptyState, Icon, Item, ItemCtx, ItemId, Subject, ToolbarEntry,
    ToolbarLocation, Verb,
};
use meridian_design::chrome::{OverlayTokens, INK_DARK, INK_LIGHT, OVERLAY_DARK, OVERLAY_LIGHT};
use meridian_design::semantic::Role;

use crate::app::{ChartDoc, CHART};
use crate::canvas::{surface_input, EguiChartFrame};
use crate::design::Mode;
use crate::legend;
use crate::navigation::{self, verb::RESET_EXTENT};
use crate::pipeline::{GestureBinding, PlotHandle};
use crate::starts;

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
            return None;
        }
        let mut empty = EmptyState::new(
            mark_icon(None),
            "Nothing to draw",
            "No spec is open, or the one that is composed no plots. Start \
             from the example below.",
        );
        if let Some(start) = starts::for_view(brightfield_workbench::ViewKind::Charts) {
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
            subject = subject.with_status(state.status_entry("run-state"));
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
        // gesture and is replaced by the next; this is about the extent
        // currently in force and stands until it is reset. Sharing one id would
        // let whichever was written last silence the other, and the pair can be
        // true at once.
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

        let (w, h) = (doc.composed.width, doc.composed.height);
        let band = legend::band_width(&doc.composed);

        // The raster and, when any plot calls for one, the legend band beside
        // it — OUTSIDE the plot rect, in the chart's margin, by layout rather
        // than by hope.
        let overlay_on = doc.overlay;
        let ctx = ui.ctx().clone();
        let texture = doc.canvas_texture();
        let mut raster_rect = None;
        let mut legend_rect = None;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let Some(texture) = texture else {
                // No device behind this document. The raster is blank rather
                // than apologetic — a headless document is a test fixture —
                // but the *layout* still happens: the geometry the exercise
                // tests hold is produced with and without a GPU alike.
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(w as f32, h as f32),
                    egui::Sense::click_and_drag(),
                );
                raster_rect = Some(rect);
                // Same gestures, no device. The overlay has nowhere to paint,
                // which is the only thing a headless document loses.
                let (repaint, _) = self.drive_gestures(doc, &ctx, rect);
                if repaint {
                    cx.request_repaint();
                }
                if band > 0.0 {
                    ui.add_space(meridian_design::spacing::CONTROL_GAP);
                    let (band_rect, _) = ui.allocate_exact_size(
                        egui::vec2(legend::block_width(), h as f32),
                        egui::Sense::hover(),
                    );
                    legend_rect = Some(band_rect);
                }
                return;
            };

            let mut frame = EguiChartFrame::new(ui, texture);
            frame.present(PixelSize {
                width: w,
                height: h,
            });
            let Some(rect) = frame.reserved() else {
                return;
            };
            raster_rect = Some(rect);

            let (repaint, gesture) = self.drive_gestures(doc, &ctx, rect);
            if repaint {
                cx.request_repaint();
            }
            let (hovered, pointer) = (gesture.hovered, gesture.pointer);

            // The one transient-gesture treatment: the overlay token group.
            // `drive_gestures` above has already taken a released drag, so the
            // rectangle is gone on the release frame rather than one frame
            // later — see the note at that take.
            if let Some(drag) = self.drag {
                let tokens = overlay_tokens(mode);
                let r = drag_rect(&doc.composed.plots[drag.plot], drag);
                let painter = frame.overlay();
                painter.fill_rect(r, Color::from_token(tokens.brush_fill));
                painter.stroke_rect(r, Color::from_token(tokens.brush_border), 1.0);
            } else if overlay_on && hovered {
                // The hover crosshair — the chart's own ink layer, matched to
                // the raster's palette rather than the chrome's.
                if let Some(p) = pointer {
                    let focus = match mode {
                        Mode::Light => INK_LIGHT.focus,
                        Mode::Dark => INK_DARK.focus,
                    };
                    let ink = Color::from_token_alpha(focus, 0.9);
                    let painter = frame.overlay();
                    painter.line(
                        kurbo::Point::new(p.x, 0.0),
                        kurbo::Point::new(p.x, f64::from(h)),
                        ink,
                        1.0,
                    );
                    painter.line(
                        kurbo::Point::new(0.0, p.y),
                        kurbo::Point::new(f64::from(w), p.y),
                        ink,
                        1.0,
                    );
                    painter.fill_circle(p, 3.0, ink);
                }
                frame.set_cursor(Some(SurfaceCursor::Grab));
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
                legend::draw_band(ui, band_rect, rect.top(), &doc.composed, mode);
            }
        });
        doc.raster_rect = raster_rect;
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

/// The brush rectangle a drag paints, clamped to its plot's DATA AREA and
/// axis-locked to the binding's brush kind (an x-interval sweeps the data
/// area's full height, a y-interval its full width).
///
/// Full span across the locked axis is what the gesture *means* — an x-range
/// brush selects that range at every y — but the span stops at the data frame,
/// not at the placed allocation. `plot.rect` is the whole allocation, margins
/// included, and the margins are where the tick labels and the axis titles are
/// drawn; clamping to it paints the brush over its own axis furniture. The
/// data area is this plot's layout offset into raster-local coordinates — the
/// same `plot_x_start`..`plot_y_end` rect the renderer clips the frame and
/// draws the axis lines against, so there is one notion of "the plot area"
/// here, not a second one invented beside it.
fn drag_rect(plot: &PlotHandle, drag: Drag) -> brightfield_render::canvas_host::SurfaceRect {
    use brightfield_render::canvas_host::SurfaceRect;
    let kind = plot.gesture.as_ref().map(|g| g.kind);
    let (x0, x1) = min_max(drag.start.x, drag.current.x);
    let (y0, y1) = min_max(drag.start.y, drag.current.y);
    let (px0, px1) = (
        plot.rect.x + plot.layout.plot_x_start(),
        plot.rect.x + plot.layout.plot_x_end(),
    );
    let (py0, py1) = (
        plot.rect.y + plot.layout.plot_y_start(),
        plot.rect.y + plot.layout.plot_y_end(),
    );
    // BOTH corners clamp, not just the near one. `x0.max(px0)` alone leaves the
    // ORIGIN untouched when the whole drag lies past the far edge: x0 stays out
    // in the margin, x1 pulls back to the frame, and the `.max(0.0)` below only
    // collapses the width — so the rect is pinned at an origin the plot does not
    // own and strokes a zero-width line across the tick labels it is supposed to
    // stop above. Clamping both ends degenerates the rect AT the frame edge
    // instead, which is where a gesture that left the plot belongs.
    let (x0, x1, y0, y1) = match kind {
        Some(BrushKind::IntervalX | BrushKind::PointX) => {
            (x0.clamp(px0, px1), x1.clamp(px0, px1), py0, py1)
        }
        Some(BrushKind::IntervalY | BrushKind::PointY) => {
            (px0, px1, y0.clamp(py0, py1), y1.clamp(py0, py1))
        }
        _ => (
            x0.clamp(px0, px1),
            x1.clamp(px0, px1),
            y0.clamp(py0, py1),
            y1.clamp(py0, py1),
        ),
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
///   displayed scales — from the corners of the rectangle [`drag_rect`] paints,
///   so what the sweep selects is what the sweep drew.
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
            // Resolve the bounds from the rectangle that was PAINTED, not from
            // the raw pointer. `drag_rect` is the one place the drag is clamped
            // to the data area, and this is the same call the overlay makes, so
            // the SQL and the ink cannot disagree. Inverting the raw pointer
            // instead feeds `Scale::inverse_f64` a pixel outside the scale's own
            // range, and it extrapolates rather than clamping — a sweep begun in
            // the margin then selects a band of data no part of the rectangle
            // ever covered, which on a `view_extent`-overridden domain is real
            // rows rather than empty space.
            let painted = drag_rect(plot, drag);
            let predicate = interval_predicate(
                binding,
                plot,
                kurbo::Point::new(painted.x, painted.y),
                kurbo::Point::new(painted.x + painted.width, painted.y + painted.height),
            )?;
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
///
/// `a` and `b` are the opposite corners of the rectangle the overlay PAINTS —
/// [`drag_rect`]'s output, already clamped to the data area — not the raw
/// pointer. Callers must keep it that way: the corners are inverted through
/// scales whose pixel range is the data area, and `Scale::inverse_f64`
/// extrapolates outside it rather than clamping, so a corner from beyond the
/// frame resolves to a bound the painted rectangle never reached.
fn interval_predicate(
    binding: &GestureBinding,
    plot: &PlotHandle,
    a: kurbo::Point,
    b: kurbo::Point,
) -> Option<SqlPredicate> {
    let mut clauses = Vec::new();
    if matches!(binding.kind, BrushKind::IntervalX | BrushKind::IntervalXY) {
        let column = binding.x_column.clone()?;
        let scale = plot.scales.get(Channel::X)?;
        clauses.push(axis_interval(
            column,
            scale,
            a.x - plot.rect.x,
            b.x - plot.rect.x,
        )?);
    }
    if matches!(binding.kind, BrushKind::IntervalY | BrushKind::IntervalXY) {
        let column = binding.y_column.clone()?;
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
fn axis_interval(column: String, scale: &Scale, p0: f64, p1: f64) -> Option<SqlPredicate> {
    let (v0, v1) = (scale.inverse_f64(p0)?, scale.inverse_f64(p1)?);
    let (lo, hi) = min_max(v0, v1);
    let bound = |v: f64| match scale {
        Scale::Time { .. } => ScalarValue::TimestampMicros(v.round() as i64),
        _ => ScalarValue::Float(v),
    };
    Some(SqlPredicate::Interval {
        column,
        lo: bound(lo),
        hi: bound(hi),
        meta: None,
    })
}

/// The structured point clause a click at `p` (raster-local) means: the
/// category whose band slot contains the pointer, on the binding's axis.
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
        column,
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
        assert_eq!(column, "x");
        assert_eq!(*lo, ScalarValue::Float(2.0));
        assert_eq!(*hi, ScalarValue::Float(8.0));
        // The structured clause renders byte-identically to the string form.
        assert_eq!(predicate.to_string(), "(x >= 2 AND x <= 8)");
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
        assert_eq!(column, "y");
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
                column: "x".to_string(),
                values: vec![ScalarValue::Text("South".to_string())],
                meta: None,
            }
        );
        assert_eq!(predicate.to_string(), "x = 'South'");
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

    // -- The brush rectangle stops at the data frame -----------------------

    /// A plot placed away from the raster origin, so a test that passes on a
    /// rect at (0, 0) cannot pass here by accident: the data area is the
    /// PLACED origin plus the layout's margins, and getting either term wrong
    /// moves the brush.
    ///
    /// 300×200 at (200, 100) with Observable Plot's default margins puts the
    /// data area at x 240..480 and y 120..270 in raster-local pixels.
    const PLACED: Rect = Rect {
        x: 200.0,
        y: 100.0,
        width: 300.0,
        height: 200.0,
    };
    const DATA_LEFT: f64 = 240.0; // 200 + margin.left 40
    const DATA_RIGHT: f64 = 480.0; // 200 + 300 - margin.right 20
    const DATA_TOP: f64 = 120.0; // 100 + margin.top 20
    const DATA_BOTTOM: f64 = 270.0; // 100 + 200 - margin.bottom 30

    /// The `plot` helper's handle, re-placed at [`PLACED`] with the layout
    /// that placement implies, carrying `scales`.
    fn placed_with(scales: ScaleSet, kind: BrushKind) -> PlotHandle {
        let mut plot = plot(scales, kind);
        plot.rect = PLACED;
        plot.layout = ChartLayout::new(PLACED.width, PLACED.height);
        plot
    }

    /// The same handle with no scales at all. Scales are irrelevant to
    /// `drag_rect` — it is pure geometry — so the set stays empty.
    fn placed(kind: BrushKind) -> PlotHandle {
        placed_with(ScaleSet::new(), kind)
    }

    fn drag(start: (f64, f64), current: (f64, f64)) -> Drag {
        Drag {
            plot: 0,
            start: kurbo::Point::new(start.0, start.1),
            current: kurbo::Point::new(current.0, current.1),
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// AC1. An x-range brush spans the DATA AREA's full height — the whole of
    /// it, because that is the gesture's meaning — and stops at its bottom
    /// edge, a clear margin above the allocation's own bottom where the tick
    /// labels and the axis title are drawn.
    #[test]
    fn an_x_brush_spans_the_data_area_and_clears_the_axis_furniture() {
        let plot = placed(BrushKind::IntervalX);
        let r = drag_rect(&plot, drag((300.0, 150.0), (400.0, 160.0)));

        // Swept on x, exactly as dragged — the sweep is inside the data area.
        assert!(close(r.x, 300.0), "x {r:?}");
        assert!(close(r.width, 100.0), "width {r:?}");

        // Full height of the data area, top and bottom.
        assert!(close(r.y, DATA_TOP), "top {r:?}");
        assert!(close(r.y + r.height, DATA_BOTTOM), "bottom {r:?}");
        assert!(close(r.height, plot.layout.plot_height()), "height {r:?}");

        // And that bottom is a whole bottom margin clear of the allocation —
        // the band the tick labels and the axis title live in.
        let allocation_bottom = PLACED.y + PLACED.height;
        assert!(
            close(allocation_bottom - (r.y + r.height), 30.0),
            "the brush reaches into the {}px axis band: {r:?}",
            allocation_bottom - (r.y + r.height)
        );
        assert!(
            r.y > PLACED.y,
            "the brush reaches into the top margin: {r:?}"
        );
    }

    /// AC1, the other half: the swept axis clamps to the DATA AREA too, so a
    /// drag begun out in the left margin starts at the frame, not at the
    /// allocation's edge.
    #[test]
    fn an_x_brush_clamps_its_sweep_to_the_data_area_not_the_allocation() {
        let plot = placed(BrushKind::IntervalX);
        let r = drag_rect(&plot, drag((205.0, 150.0), (900.0, 160.0)));
        assert!(close(r.x, DATA_LEFT), "left {r:?}");
        assert!(close(r.x + r.width, DATA_RIGHT), "right {r:?}");
    }

    /// AC2. The y-range case is the same rule on the other axis: full data-area
    /// WIDTH, stopping clear of the left margin the y tick labels occupy, with
    /// the swept axis left as dragged.
    #[test]
    fn a_y_brush_spans_the_data_area_width_and_clears_the_tick_labels() {
        let plot = placed(BrushKind::IntervalY);
        let r = drag_rect(&plot, drag((300.0, 140.0), (310.0, 200.0)));

        // Swept on y, exactly as dragged.
        assert!(close(r.y, 140.0), "y {r:?}");
        assert!(close(r.height, 60.0), "height {r:?}");

        // Full width of the data area — not of the allocation.
        assert!(close(r.x, DATA_LEFT), "left {r:?}");
        assert!(close(r.x + r.width, DATA_RIGHT), "right {r:?}");
        assert!(close(r.width, plot.layout.plot_width()), "width {r:?}");
        assert!(
            close(r.x - PLACED.x, 40.0),
            "the brush reaches into the y-label margin: {r:?}"
        );
    }

    /// AC2. The two-dimensional case clamps BOTH axes to the data area: a
    /// drag that overshoots the allocation on every side paints the data
    /// frame exactly, and no part of the margins.
    #[test]
    fn an_xy_brush_clamps_both_axes_to_the_data_area() {
        let plot = placed(BrushKind::IntervalXY);
        let r = drag_rect(&plot, drag((0.0, 0.0), (900.0, 900.0)));
        assert!(close(r.x, DATA_LEFT), "left {r:?}");
        assert!(close(r.x + r.width, DATA_RIGHT), "right {r:?}");
        assert!(close(r.y, DATA_TOP), "top {r:?}");
        assert!(close(r.y + r.height, DATA_BOTTOM), "bottom {r:?}");

        // An in-bounds XY sweep is left exactly as dragged.
        let r = drag_rect(&plot, drag((400.0, 200.0), (300.0, 150.0)));
        assert!(close(r.x, 300.0) && close(r.width, 100.0), "x {r:?}");
        assert!(close(r.y, 150.0) && close(r.height, 50.0), "y {r:?}");
    }

    /// AC1 at the FAR edges — the case a near-corner-only clamp gets wrong.
    ///
    /// A drag lying ENTIRELY past the right or bottom edge used to keep its
    /// unclamped origin: `x1` pulled back to the frame, `x0` did not, and the
    /// width collapsed to zero — leaving a full-height stroke standing in the
    /// margin, which is the sighted symptom (ink over the axis furniture) in its
    /// most literal form. Degenerate is fine; degenerate OUTSIDE the frame is not.
    #[test]
    fn a_sweep_wholly_past_the_far_edge_degenerates_at_the_frame_not_in_the_margin() {
        // Entirely right of the data area, and short of the allocation's edge.
        let plot = placed(BrushKind::IntervalX);
        let r = drag_rect(&plot, drag((DATA_RIGHT + 6.0, 150.0), (DATA_RIGHT + 16.0, 160.0)));
        assert!(
            close(r.x, DATA_RIGHT) && close(r.width, 0.0),
            "an x sweep past the right edge must degenerate AT it: {r:?}"
        );
        assert!(
            r.x + r.width <= DATA_RIGHT + 1e-9,
            "the brush reaches into the right margin: {r:?}"
        );

        // Entirely below the data area, where the tick labels and title live.
        let plot = placed(BrushKind::IntervalXY);
        let r = drag_rect(
            &plot,
            drag((300.0, DATA_BOTTOM + 10.0), (500.0, DATA_BOTTOM + 25.0)),
        );
        assert!(
            close(r.y, DATA_BOTTOM) && close(r.height, 0.0),
            "a y sweep below the bottom edge must degenerate AT it: {r:?}"
        );
        assert!(
            r.y + r.height <= DATA_BOTTOM + 1e-9,
            "the brush reaches into the axis band: {r:?}"
        );

        // And the near side, for symmetry — wholly left of the frame.
        let plot = placed(BrushKind::IntervalX);
        let r = drag_rect(&plot, drag((DATA_LEFT - 30.0, 150.0), (DATA_LEFT - 5.0, 160.0)));
        assert!(
            close(r.x, DATA_LEFT) && close(r.width, 0.0),
            "an x sweep left of the frame must degenerate AT it: {r:?}"
        );
    }

    // The claim "the brush clamps to the renderer's own plot rect" is made by
    // the three tests above, in hardcoded raster constants. It was also made by
    // a fourth test that asserted the output against
    // `plot.rect.x + plot.layout.plot_x_start()` — the implementation
    // expression, copied. Both sides moved together, so it was blind: adding
    // 7px to `plot_x_start()` left it green while all three constant-based
    // tests reddened. It is deleted rather than rewritten because the renderer
    // has no independent statement of the frame to check against — its
    // `plot_area_rect` (scene.rs) is private and computes the identical
    // expression from the identical accessors, so routing the assertion
    // through it would swap one re-derivation for another. The constants are
    // the independent statement.

    // -- The predicate is the painted rectangle ----------------------------

    /// The SQL and the ink describe the same rectangle.
    ///
    /// A drag begun out in the left margin paints from the frame edge — that
    /// is the clamp above — so it must SELECT from the frame edge too.
    /// Resolving the bound from the raw pointer instead inverts a pixel that
    /// lies outside the scale's own range, and `inverse_f64` extrapolates
    /// rather than clamps: the bound lands outside the domain, and on a plot
    /// whose `view_extent` has overridden that domain there is real data out
    /// there for it to select — rows the rectangle never covered.
    #[test]
    fn a_sweep_begun_in_the_margin_selects_only_what_it_painted() {
        let mut scales = ScaleSet::new();
        // A plot's positional range IS its data area, in plot-local pixels:
        // 40..280 across a 300-wide plot at the default margins.
        scales.insert(Channel::X, linear((40.0, 280.0), (0.0, 100.0)));
        let plot = placed_with(scales, BrushKind::IntervalX);
        let binding = plot.gesture.clone().expect("bound");

        // Down 35px inside the left margin, released well within the frame.
        let swept = drag((205.0, 150.0), (360.0, 200.0));

        // What the user saw: a rectangle that starts at the frame edge,
        // 35px right of where the pointer went down.
        let painted = drag_rect(&plot, swept);
        assert!(close(painted.x, DATA_LEFT), "painted left {painted:?}");

        let Some(Interaction::Select { predicate, .. }) = resolve_gesture(&binding, &plot, swept)
        else {
            panic!("a sweep selects");
        };
        let SqlPredicate::Interval { lo, hi, .. } = &predicate else {
            panic!("an x sweep is one interval, got {predicate:?}");
        };
        let (ScalarValue::Float(lo), ScalarValue::Float(hi)) = (lo, hi) else {
            panic!("linear bounds are floats");
        };

        // The painted left edge is raster 240 = plot-local 40 = the domain's
        // own start, 0. NOT the -14.583 the raw pointer at raster 205
        // extrapolates to — 14.6 units of data the rectangle never covered.
        assert!(
            close(*lo, 0.0),
            "lo {lo}: the predicate ran past the painted rectangle"
        );
        // The release is inside the frame, so it is taken as dragged: raster
        // 360 = plot-local 160, two thirds along a 40..280 range = 50.
        assert!(close(*hi, 50.0), "hi {hi}");
    }

    /// The same rule on the other axis and the other corner, so a fix that
    /// picks the wrong edge off the rect cannot pass: a y sweep begun above
    /// the frame selects from the frame's TOP, which on a downward pixel
    /// range is the domain's maximum.
    #[test]
    fn a_y_sweep_begun_above_the_frame_selects_only_what_it_painted() {
        let mut scales = ScaleSet::new();
        // y runs downward: plot-local 170 (bottom) is the domain min, 20 the max.
        scales.insert(Channel::Y, linear((170.0, 20.0), (0.0, 100.0)));
        let plot = placed_with(scales, BrushKind::IntervalY);
        let binding = plot.gesture.clone().expect("bound");

        // Down 15px above the frame, in the top margin.
        let swept = drag((300.0, 105.0), (310.0, 195.0));

        let painted = drag_rect(&plot, swept);
        assert!(close(painted.y, DATA_TOP), "painted top {painted:?}");

        let Some(Interaction::Select { predicate, .. }) = resolve_gesture(&binding, &plot, swept)
        else {
            panic!("a sweep selects");
        };
        let SqlPredicate::Interval { lo, hi, .. } = &predicate else {
            panic!("a y sweep is one interval, got {predicate:?}");
        };
        let (ScalarValue::Float(lo), ScalarValue::Float(hi)) = (lo, hi) else {
            panic!("linear bounds are floats");
        };

        // Raster 195 = plot-local 95, halfway up a 170..20 range = 50.
        assert!(close(*lo, 50.0), "lo {lo}");
        // The painted top edge is raster 120 = plot-local 20 = the domain's
        // own maximum, 100. NOT the 110 the raw pointer at raster 105 reaches.
        assert!(
            close(*hi, 100.0),
            "hi {hi}: the predicate ran past the painted rectangle"
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
