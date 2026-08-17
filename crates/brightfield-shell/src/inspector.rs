//! The inspector — the right-hand column that replaces the old "Controls"
//! rail: it says what is selected, and what can be done with it.
//!
//! # Why this is not `app.rs::ControlsPane`, edited in place
//!
//! `ControlsPane` — its id, its `describe`, its `ui` — is declared inside
//! `app.rs`'s `chart_registry_with`, and `app.rs` is a concurrent lane's file
//! this sprint (the chart canvas taking the mode the shell is in). So the
//! swap happens one level up, in `window.rs::MeridianApp::assemble`: the
//! registry still constructs a `ControlsPane` — nothing there changed, and
//! `chart_contract.rs`'s assertions about it keep passing unmodified — and the
//! window overwrites that one map entry with an [`InspectorPane`] before the
//! first frame draws. Slot geometry (the rail's side, its share, its toggle
//! verb) stays the registry's, because those come from `ItemSpec`, not from
//! the boxed `Item`; only *what fills the slot* changed.
//!
//! # Why "what is focused" arrives through a shared cell rather than through
//! `ItemCtx`
//!
//! `Item::ui` is handed `&mut D` and an [`ItemCtx`], and neither carries a
//! sibling pane's [`Subject`] — an item is not meant to reach past its own
//! pane (see `brightfield_workbench::item`'s module docs on the aliasing
//! decision this crate is built on). The window already computes "the focused
//! pane's `Subject`" once a frame, for the status rail
//! (`window::MeridianApp::status_rail_ui`); this pane reads the *same* value
//! through a [`Selection`] handle the window writes into immediately before
//! the dock draws, so the inspector can never disagree with the rail about
//! what is focused. Focusing the inspector's own pane (clicking the checkbox,
//! say) leaves the last real selection standing rather than blanking it —
//! see [`Selection::set`]'s caller in `window.rs` for the guard.
//!
//! # What still lives here from `ControlsPane`
//!
//! The param sliders, the interval sliders and the hover-overlay checkbox are
//! ported verbatim — same document methods, same fields, same widget code —
//! because `tests/interval_slider.rs` drives them through a real window and
//! would silently stop finding them if this pane drew something else in their
//! place.

use std::cell::RefCell;
use std::rc::Rc;

use brightfield_keys::BindingContext;
use brightfield_workbench::{chrome, EmptyState, Icon, Item, ItemCtx, ItemId, Subject, Verb};
use meridian_design::{semantic, spacing};

use crate::app::{ChartDoc, CONTROLS};
use crate::design::Mode;

/// The rail's icon — unchanged from the pane it replaces, so the Meridian
/// icon set landing later is one change, not two.
const ICON_INSPECTOR: Icon = Icon("sliders");

/// What "nothing selected" says. Distinct from [`InspectorPane::empty_state`]'s
/// own text below: that one answers "no dashboard is open at all" (the whole
/// pane is empty, so the shell never calls `ui`); this one answers "a
/// dashboard is open, but nothing in it has been clicked on yet" — drawn
/// *inside* `ui`, alongside the hover-overlay checkbox, which must stay
/// reachable either way (AC5).
const NOTHING_SELECTED_HEADLINE: &str = "Nothing selected";
const NOTHING_SELECTED_BODY: &str = "Click a pane — the chart, the data grid, \
     the spec editor — and this panel names it and lists what you can do \
     with it.";

// ---------------------------------------------------------------------------
// Selection — the frame-fresh handle from the window
// ---------------------------------------------------------------------------

/// A frame-fresh snapshot of the focused pane's [`Subject`] — set by
/// `MeridianApp::draw` right before the dock draws, read by
/// [`InspectorPane::ui`] the moment after.
///
/// `Rc<RefCell<_>>` rather than a field on [`ChartDoc`]: the value belongs to
/// the *window* (it is `Workspace::focus`'s cousin, not engine state), and
/// `ChartDoc` is declared in `app.rs`, out of this lane's reach. Two clones
/// exist for the app's life — one on `MeridianApp`'s `ChartView`, one moved
/// into the boxed `InspectorPane` at construction — both aliasing the same
/// cell, read and written on the same thread within one frame, never across
/// one.
#[derive(Clone, Default)]
pub struct Selection(Rc<RefCell<Option<Subject>>>);

impl Selection {
    /// Overwrite this frame's focused-pane subject. `None` means no pane in
    /// this view is focused right now — not "leave whatever was there".
    pub fn set(&self, subject: Option<Subject>) {
        *self.0.borrow_mut() = subject;
    }

    /// This frame's focused-pane subject, if any pane has been focused.
    ///
    /// `pub(crate)` rather than private: `window.rs`'s `MeridianApp::
    /// inspector_selection` reads the same cell to hand a test the current
    /// answer without simulating a pointer event.
    pub(crate) fn get(&self) -> Option<Subject> {
        self.0.borrow().clone()
    }
}

// ---------------------------------------------------------------------------
// The shared body — what the shipping pane and the gallery specimen both draw
// ---------------------------------------------------------------------------

/// Draw what is selected — `subject`'s title and toolbar — or, when nothing
/// is, the empty state naming what selecting something would show.
///
/// Shared between [`InspectorPane::ui`] (fed the workspace's real focused-pane
/// subject) and the gallery's specimen in `gallery.rs` (fed a standing
/// example), so the demo can never draw something the shipping panel would
/// not: the gallery's whole argument is that it "cannot disagree with the app
/// about what a primitive looks like, because it is the app."
///
/// Returns the verbs any drawn toolbar button activated this frame — the
/// caller's to act on ([`ItemCtx::request`] in the shipping pane; the gallery
/// specimen only checks it fired).
pub fn render_selection(ui: &mut egui::Ui, subject: Option<&Subject>, mode: Mode) -> Vec<Verb> {
    match subject {
        Some(subject) => {
            ui.label(egui::RichText::new(&subject.title).strong());
            chrome::breadcrumb(ui, &subject.breadcrumb, mode);
            let toolbar = chrome::Toolbar::new(&subject.toolbar);
            if toolbar.has_something_to_say() {
                ui.add_space(spacing::CONTROL_GAP);
            }
            toolbar.show(ui, mode).activated
        }
        None => {
            let empty = EmptyState::new(
                ICON_INSPECTOR,
                NOTHING_SELECTED_HEADLINE,
                NOTHING_SELECTED_BODY,
            );
            chrome::empty_state(ui, &empty, mode);
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// The pane
// ---------------------------------------------------------------------------

/// The inspector: what is selected, and what can be done with it — plus the
/// live document controls that used to be the whole of "Controls" (the param
/// sliders, the interval sliders, the hover-overlay checkbox), unchanged.
pub struct InspectorPane {
    selection: Selection,
}

impl InspectorPane {
    /// An inspector reading `selection` — the handle the window updates each
    /// frame before the dock draws. See the module docs for why this is a
    /// shared cell rather than a field `Item::ui` is handed directly.
    #[must_use]
    pub fn new(selection: Selection) -> Self {
        Self { selection }
    }
}

impl Item<ChartDoc> for InspectorPane {
    fn item_id(&self) -> ItemId {
        CONTROLS
    }

    /// Empty under the same condition the pane it replaces used: no dashboard
    /// open at all. "Nothing selected within an open dashboard" is a
    /// *narrower* case, drawn inside [`Self::ui`] instead of here — an
    /// `Item::empty_state` answer of `Some` skips `ui` entirely, and the
    /// hover-overlay checkbox has to stay reachable whether or not anything
    /// has been clicked on yet (AC5).
    fn empty_state(&self, doc: &ChartDoc) -> Option<EmptyState> {
        doc.is_empty().then(|| {
            EmptyState::new(
                ICON_INSPECTOR,
                "No dashboard to inspect",
                "This panel names what is selected in a composed dashboard. \
                 Open one from the chart pane.",
            )
        })
    }

    fn describe(&self, _doc: &ChartDoc) -> Subject {
        Subject::new("Inspector", ICON_INSPECTOR, BindingContext::Workspace)
    }

    fn ui(&mut self, doc: &mut ChartDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        let subject = self.selection.get();
        let activated = render_selection(ui, subject.as_ref(), cx.mode);
        for verb in activated {
            cx.request(verb);
        }
        ui.add_space(spacing::SECTION_GAP);

        // Everything below is `ControlsPane::ui`, ported verbatim — see the
        // module docs for why it has to stay exactly this code.

        // The legend that used to sit above these controls is gone for good:
        // a hardcoded "Series A/B/C" swatch block duplicated the chart's own
        // legend and, being fixed at three series, mislabelled a single-series
        // bar chart. The one legend is [`crate::legend`]'s margin panel,
        // drawn by the chart pane from each chart's real series.
        //
        // The slider is two things depending on what the spec declared. A
        // spec with a slider-backed scalar param gets one slider **per
        // declared param**, labelled with the param's name, spanning the
        // widget's own range, and wired through the coordinator seam — a drag
        // is an `Interaction::SetParam`, a pushed value and a re-query, never
        // a Rust-side filter. A spec with no declared params draws no slider at
        // all — just the crosshair toggle below.
        let params = doc.composed.params.clone();
        if doc.is_live() && !params.is_empty() {
            for control in params {
                ui.label(&control.name);
                let mut value = control.value;
                let mut slider = egui::Slider::new(&mut value, control.min..=control.max);
                if let Some(step) = control.step {
                    slider = slider.step_by(step);
                }
                let response = ui.add(slider);
                if response.changed() && (value - control.value).abs() > f64::EPSILON {
                    doc.set_param(&control.name, value);
                }
            }
        }
        // The other slider: an interval slider writes a SELECTION, not a
        // param, so it dispatches an `Interaction::Select` carrying the same
        // structured interval clause a plot brush pushes — which is what puts
        // a sustained drag on the pre-aggregated path.
        //
        // Three things separate it from the param slider above. Its handle is
        // read out of the document's own drag state rather than re-seeded from
        // the composition, so under a drag it shows the POINTER's value and
        // not the last completed query's. It records rather than dispatches:
        // the pump below sends at most one value per slider per frame, newest
        // only. And it never touches the spec — no buffer, no file, no dirty
        // flag moves, because a selection is live state and not an edit.
        let intervals = doc.composed.intervals.clone();
        doc.interval_slider_rects.clear();
        if doc.is_live() && !intervals.is_empty() {
            for control in &intervals {
                ui.label(control.label.as_deref().unwrap_or(&control.selection));
                let mut value = doc.interval_drags.shown(control.key(), control.value);
                let mut slider = egui::Slider::new(&mut value, control.min..=control.max);
                if let Some(step) = control.step {
                    slider = slider.step_by(step);
                }
                let response = ui.add(slider);
                doc.interval_slider_rects
                    .push((control.key().to_string(), response.rect));
                if response.changed() {
                    doc.note_interval_drag(control, value);
                }
            }
            if doc.pump_interval_drags(&intervals) > 0 || doc.interval_drags.any_pending() {
                // A drag that outran its queries has work left; keep frames
                // coming so the next one goes out without waiting on a
                // pointer event that may never arrive.
                ui.ctx().request_repaint();
            }
        }
        doc.overlay_checkbox = Some(ui.checkbox(&mut doc.overlay, "hover overlay").rect);
        if crate::devtools::enabled() {
            ui.add_space(spacing::CONTROL_GAP);
            let sem = semantic(cx.mode.is_dark());
            ui.label(
                egui::RichText::new(format!(
                    "{}×{} logical",
                    doc.composed.width, doc.composed.height
                ))
                .monospace()
                .color(chrome::colour(sem.text.muted)),
            );
        }
    }
}
