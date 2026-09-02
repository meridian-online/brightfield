//! The inspector — the right-hand column that replaces the old "Controls"
//! rail: it says what is selected, and what can be done with it.
//!
//! # Why this is not `app.rs::ControlsPane`, edited in place
//!
//! `ControlsPane` — its id, its `describe`, its `ui` — is declared inside
//! `app.rs`'s `chart_registry_with`, and `app.rs` is a concurrent lane's file
//! this sprint (the chart canvas taking the mode the shell is in). So the
//! swap happens one level up, in `window.rs::MeridianApp::assemble`: the
//! registry goes on constructing that same `ControlsPane`, unedited, and
//! `chart_contract.rs`'s assertions about it keep passing unmodified — and the
//! window overwrites that one map entry with an [`InspectorPane`] before the
//! first frame draws. Slot geometry (the rail's side, its share, its toggle
//! verb) stays the registry's, because those come from `ItemSpec`, not from
//! the boxed `Item` — what changed is what fills the slot.
//!
//! # Why "what is focused" arrives through a shared cell rather than through
//! `ItemCtx`
//!
//! `Item::ui` is handed `&mut D` and an [`ItemCtx`], and neither carries a
//! sibling pane's [`Subject`] — an item is not meant to reach past its own
//! pane (see `brightfield_workbench::item`'s module docs on the aliasing
//! decision this crate is built on). The window already computes "the focused
//! pane's `Subject`" once a frame, for the status rail
//! (`window::MeridianApp::status_rail_ui`); this pane reads that same
//! computed value through a [`Selection`] handle the window writes into
//! immediately before the dock draws, rather than recomputing its own answer.
//! Focusing the inspector's own pane (clicking the checkbox, say) leaves the
//! last real selection standing rather than blanking it — see
//! [`Selection::set`]'s caller in `window.rs` for the guard.
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
use brightfield_workbench::{
    chrome, EmptyState, Icon, Item, ItemCtx, ItemId, Subject, ToolbarEntry, Verb,
};
use meridian_design::{semantic, spacing};

use crate::app::{ChartDoc, CONTROLS};
use crate::design::Mode;
use crate::one_step::ColumnFacts;
use crate::overlays::CHART_PALETTE_VERBS;
use crate::protocol::ui_font;

/// The rail's icon — unchanged from the pane it replaces, so the Meridian
/// icon set landing later is one change, not two.
const ICON_INSPECTOR: Icon = Icon("sliders");

/// What "nothing selected" says. Distinct from [`InspectorPane::empty_state`]'s
/// own text below: that one answers "no dashboard is open" (the pane is
/// empty, so the shell skips calling `ui` for the frame); this one answers "a
/// dashboard is open, but no pane in it has been clicked on yet" — drawn
/// *inside* `ui`, alongside the hover-overlay checkbox, which has to stay
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
/// cell, read and written on the same thread, within the one frame that set
/// it.
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

/// The subset of `entries` this rail may draw: an entry passes when its verb
/// is one `MeridianApp::apply`'s Charts arm dispatches, per
/// [`CHART_PALETTE_VERBS`] — the same enumeration the chart command palette
/// (`overlays.rs`) is restricted to, so the two surfaces read one answer for
/// what is live at this altitude rather than two that could drift apart.
///
/// This is the pane's whole answer to "what can be done with it": a
/// declared-but-undispatchable entry would look live while doing nothing if
/// drawn here unfiltered, which is worse than the checkbox this rail replaced.
/// So an entry not on the allow list is dropped rather than shown disabled —
/// see `tests/inspector_contract.rs`'s
/// `every_declared_toolbar_verb_either_dispatches_or_is_filtered_out` for the
/// sweep that keeps this from widening in silence.
fn dispatchable(entries: &[ToolbarEntry]) -> Vec<ToolbarEntry> {
    entries
        .iter()
        .filter(|entry| CHART_PALETTE_VERBS.contains(&entry.verb.as_str()))
        .cloned()
        .collect()
}

/// Draw what is selected — `subject`'s title and toolbar — or, when nothing
/// is, the empty state naming what selecting something would show.
///
/// Shared between [`InspectorPane::ui`] (fed the workspace's real focused-pane
/// subject) and the gallery's specimen in `gallery.rs` (fed a standing
/// example), so a change to the demo's drawing is a change to the shipping
/// panel's too: the gallery's whole argument is that it "cannot disagree with
/// the app about what a primitive looks like, because it is the app."
///
/// Returns the verbs any drawn toolbar button activated this frame — the
/// caller's to act on ([`ItemCtx::request`] in the shipping pane; the gallery
/// specimen only checks it fired).
pub fn render_selection(ui: &mut egui::Ui, subject: Option<&Subject>, mode: Mode) -> Vec<Verb> {
    match subject {
        Some(subject) => {
            ui.label(egui::RichText::new(&subject.title).strong());
            chrome::breadcrumb(ui, &subject.breadcrumb, mode);
            let entries = dispatchable(&subject.toolbar);
            let toolbar = chrome::Toolbar::new(&entries);
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
    /// The table a selected column belongs to, and the step that produced it —
    /// the two lines the column block draws under the column's name.
    ///
    /// A shared cell rather than a field, for the reason this module's header
    /// gives about [`Selection`]: an `Item` is handed its own document and no
    /// other, and this one's is the chart's, while the table is the protocol
    /// document's. It is a cell rather than a value fixed at construction
    /// because a window outlives the document in it — opening a second data
    /// file into a window that already exists has to move this too, and by
    /// then the pane is boxed behind `dyn Item`.
    table: TableHandle,
}

/// The window's handle on which table a selected column belongs to — written
/// when a document is adopted, read by [`InspectorPane::ui`].
#[derive(Clone, Default)]
pub struct TableHandle(Rc<RefCell<Option<ColumnTable>>>);

impl TableHandle {
    /// Declare the table a selected column belongs to. `None` for a document
    /// with no Protocol behind it.
    pub fn set(&self, table: Option<ColumnTable>) {
        *self.0.borrow_mut() = table;
    }

    /// What was last declared.
    #[must_use]
    pub fn get(&self) -> Option<ColumnTable> {
        self.0.borrow().clone()
    }
}

/// What a column belongs to: the table's name, and the step that built it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnTable {
    /// The table the column is in.
    pub table: String,
    /// The step that produces it.
    pub step: String,
    /// That step's transform class — `sql` for a one-step Protocol.
    pub kind: &'static str,
}

impl InspectorPane {
    /// An inspector reading `selection` — the handle the window updates each
    /// frame before the dock draws. See the module docs for why this is a
    /// shared cell rather than a field `Item::ui` is handed directly.
    #[must_use]
    pub fn new(selection: Selection, table: TableHandle) -> Self {
        Self { selection, table }
    }
}

/// The selected column, as the inspector draws it: what it is called, what it
/// belongs to, what it means as opposed to what DuckDB stored it as, the
/// picture it was given and why, what the engine measured, and the step that
/// produced it.
///
/// The **whole** semantic label is here, not its leaf. The navigator rail
/// draws the leaf because 240 logical points do not hold
/// `representation.numeric.decimal_number` beside an eighteen-character column
/// name; this rail is the place the reader can read the rest.
fn column_body(ui: &mut egui::Ui, column: &ColumnFacts, table: Option<&ColumnTable>, mode: Mode) {
    let sem = semantic(mode.is_dark());
    ui.label(
        egui::RichText::new(&column.column)
            .font(ui_font())
            .color(chrome::colour(sem.text.primary)),
    );
    let belongs = table.map_or_else(|| "column".to_string(), |t| format!("column · {}", t.table));
    ui.label(
        egui::RichText::new(belongs)
            .font(ui_font())
            .color(chrome::colour(sem.text.secondary)),
    );

    column_field(ui, mode, "finetype", column.full_type());
    column_field(ui, mode, "storage", &column.storage);
    match &column.tile {
        Some(kind) => {
            column_field(ui, mode, "tile", kind);
            column_field(ui, mode, "chosen by", &column.because);
            if let Some(other) = &column.paired {
                // A point map is one picture of two columns, so the rail says
                // which other column is in it — without this the reader is
                // looking at a map and told about one axis of it.
                column_field(ui, mode, "drawn with", other);
            }
        }
        None => column_field(ui, mode, "tile", &column.because),
    }

    ui.add_space(spacing::SPACE_4);
    ui.label(
        egui::RichText::new("VALUES · FROM THE ENGINE")
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
    column_field(ui, mode, "rows", &column.rows.to_string());
    if let Some(min) = &column.min {
        column_field(ui, mode, "min", min);
    }
    if let Some(max) = &column.max {
        column_field(ui, mode, "max", max);
    }
    column_field(ui, mode, "nulls", &column.nulls.to_string());

    if let Some(t) = table {
        ui.add_space(spacing::SPACE_4);
        ui.label(
            egui::RichText::new("PRODUCED BY")
                .font(ui_font())
                .color(chrome::colour(sem.text.muted)),
        );
        column_field(ui, mode, "step", &format!("{} · {}", t.step, t.kind));
        // Brightfield writes the spec and never a run record, so the honest
        // answer here is always the same one and it is stated rather than
        // computed — a status derived from nothing would be a claim about a
        // run that did not happen.
        column_field(ui, mode, "status", NOT_RUN);
    }
}

/// What a step brightfield declared and nobody ran reads as. The word the
/// protocol view's own `status_word` uses for `SeamStatus::NotRun`, spelled
/// once so the two rails cannot disagree about it.
pub const NOT_RUN: &str = "not run";

/// One caption-and-value line of the column block.
fn column_field(ui: &mut egui::Ui, mode: Mode, label: &str, value: &str) {
    let sem = semantic(mode.is_dark());
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(label)
                .font(ui_font())
                .color(chrome::colour(sem.text.muted)),
        );
        ui.label(
            egui::RichText::new(value)
                .font(ui_font())
                .color(chrome::colour(sem.text.primary)),
        );
    });
}

impl Item<ChartDoc> for InspectorPane {
    fn item_id(&self) -> ItemId {
        CONTROLS
    }

    /// Empty under the same condition the pane it replaces used: no dashboard
    /// open. An open dashboard with an empty selection is a *narrower* case,
    /// drawn inside [`Self::ui`] instead of here — an `Item::empty_state`
    /// answer of `Some` skips the call to `ui` for the frame, and the
    /// hover-overlay checkbox has to stay reachable whether or not the user
    /// has clicked on a pane yet (AC5).
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
        // A selected COLUMN outranks the focused pane, and that is a statement
        // about what the reader just did rather than a preference. Clicking a
        // tile is the only gesture that reaches this rail carrying a subject of
        // its own; the pane subject is what the rail falls back to when nobody
        // has pointed at anything in the picture yet.
        if let Some(column) = doc.selected_column().cloned() {
            column_body(ui, &column, self.table.get().as_ref(), cx.mode);
            ui.add_space(spacing::SECTION_GAP);
        } else {
            let subject = self.selection.get();
            let activated = render_selection(ui, subject.as_ref(), cx.mode);
            for verb in activated {
                cx.request(verb);
            }
            ui.add_space(spacing::SECTION_GAP);
        }

        // The rest of this method is `ControlsPane::ui`, ported verbatim —
        // see the module docs for why it has to stay exactly this code.

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
        // is an `Interaction::SetParam`, a pushed value and a re-query rather
        // than a Rust-side filter. A spec with no declared params draws no
        // slider — just the crosshair toggle below.
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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A dispatchable verb (drawn on the real chart pane's toolbar) and one
    /// that is not: `toggle-presentation` is a real registry verb that
    /// `MeridianApp::apply`'s Charts branch has no arm for, so it is what an
    /// entry the rail must drop looks like.
    fn sample_entries() -> (ToolbarEntry, ToolbarEntry) {
        (
            ToolbarEntry::button(
                "clear-selection",
                "Clear selection",
                Verb::new("clear-selection"),
            ),
            ToolbarEntry::button(
                "toggle-presentation",
                "Present",
                Verb::new("toggle-presentation"),
            ),
        )
    }

    /// The filter's whole job: keep the verb the Charts view can run, drop
    /// the one it cannot — never the reverse, and never both or neither.
    #[test]
    fn dispatchable_keeps_only_verbs_the_charts_view_can_run() {
        let (keep, drop) = sample_entries();
        let filtered = dispatchable(&[keep.clone(), drop]);
        assert_eq!(
            filtered,
            vec![keep],
            "dispatchable() let an undispatchable verb through, or dropped a \
             real one — either way the inspector would draw a button that \
             lies about what it does"
        );
    }

    /// `dispatchable`'s promise is membership in `CHART_PALETTE_VERBS`, not
    /// the order that list happens to declare them in — the input's own
    /// declaration order survives the filter instead.
    #[test]
    fn dispatchable_preserves_declaration_order() {
        let (keep, _drop) = sample_entries();
        let reset = ToolbarEntry::button("reset-extent", "Reset view", Verb::new("reset-extent"));
        let filtered = dispatchable(&[reset.clone(), keep.clone()]);
        assert_eq!(filtered, vec![reset, keep]);
    }
}
