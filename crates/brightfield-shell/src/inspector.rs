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
//! The param sliders and the interval sliders are ported verbatim — same
//! document methods, same fields, same widget code — because
//! `tests/interval_slider.rs` drives them through a real window and would
//! silently stop finding them if this pane drew something else in their place.
//!
//! The *hover overlay* checkbox that stood beside them is gone: the chart
//! answers the pointer with no control to arm, so there is nothing to draw.

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
/// *inside* `ui`, alongside the live document controls, which have to stay
/// reachable either way.
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
/// [`CHART_PALETTE_VERBS`] — the verbs whose meaning does not depend on which
/// window they are pressed over.
///
/// **`save-spec` is not among them, on any window**, and the reason is that
/// the name carries two writes. `EditorPane` declares a Save entry for its own
/// buffer, and `MeridianApp::apply`'s arm writes the Protocol; this rail draws
/// the *editor's* entry, so passing it here put a control on screen that is
/// dead while the buffer is clean and, when the buffer is dirty, dispatches the
/// **Protocol** save and reports success over an edit that was never written.
/// The palette is the surface that offers the Protocol save, because there the
/// row is the verb rather than a pane's button: it is listed through
/// [`crate::overlays::chart_palette_verbs`] and greyed from what the window
/// holds ([`crate::overlays::CommandPalette::with_situation`]). Splitting the verb is what would let this
/// rail offer either one honestly, and that is not this change.
///
/// This is the pane's whole answer to "what can be done with it": a
/// declared-but-undispatchable entry would look live while doing nothing if
/// drawn here unfiltered, which is worse than the checkbox this rail replaced.
/// So an entry not on the allow list is dropped rather than shown disabled —
/// see `tests/inspector_contract.rs`'s
/// `every_declared_toolbar_verb_either_dispatches_or_is_filtered_out` for the
/// sweep that keeps this from widening in silence.
pub fn dispatchable(entries: &[ToolbarEntry]) -> Vec<ToolbarEntry> {
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
/// sliders and the interval sliders), unchanged.
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
    /// The table a later step makes, while the canvas holds it: what makes it
    /// and its columns as far as they have been read. Written by the window
    /// each frame before the dock draws, as [`Selection`] is.
    inspected: InspectedHandle,
}

/// The window's handle on the table the canvas holds, when a step after the
/// load makes it — written each frame before the dock draws, read by
/// [`InspectorPane::ui`].
///
/// It also carries the inspector's ask back: the pane notes when it drew the
/// table, and the window starts reading the table's columns only then, so a
/// table on the canvas with the inspector shut or on another pane is not read
/// for nobody.
#[derive(Clone, Default)]
pub struct InspectedHandle {
    table: Rc<RefCell<Option<InspectedTable>>>,
    asked: Rc<std::cell::Cell<bool>>,
}

impl InspectedHandle {
    /// Declare this frame's table. `None` when the canvas holds no such table.
    pub fn set(&self, table: Option<InspectedTable>) {
        *self.table.borrow_mut() = table;
    }

    /// What was last declared.
    #[must_use]
    pub fn get(&self) -> Option<InspectedTable> {
        self.table.borrow().clone()
    }

    /// Note that the inspector drew the declared table this frame.
    pub fn ask(&self) {
        self.asked.set(true);
    }

    /// Whether the inspector drew the declared table since this was last
    /// taken.
    pub fn take_asked(&self) -> bool {
        self.asked.replace(false)
    }
}

/// A table a later step makes, as the inspector describes it: what makes it,
/// and its columns as far as they have been read.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectedTable {
    /// The table, the step that makes it, and where that step stands.
    pub table: ColumnTable,
    /// The table's columns, or that they are being read, or why they could
    /// not be.
    pub columns: crate::column_profiles::ProfileRead,
}

/// What the inspector says while a table's columns are being read: a line of
/// its own, so a read that takes a moment is not taken for a table with
/// nothing in it.
#[must_use]
pub fn reading_line(table: &str) -> String {
    format!("Reading the columns of {table}…")
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

/// What a column belongs to: the table's name, the step that makes it, and
/// where that step stands. Read from the Protocol's model
/// ([`crate::protocol::ProtocolModel::column_table`]), so a table a later step
/// makes names that step and not the load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnTable {
    /// The table the column is in.
    pub table: String,
    /// The step that produces it.
    pub step: String,
    /// That step's transform class, in the steps sheet's words: `sql`, `op`
    /// or `command`.
    pub kind: &'static str,
    /// The state the run record gives the step, in the steps sheet's words:
    /// [`NOT_RUN`] until a run records it.
    pub state: &'static str,
}

impl InspectorPane {
    /// An inspector reading `selection` — the handle the window updates each
    /// frame before the dock draws. See the module docs for why this is a
    /// shared cell rather than a field `Item::ui` is handed directly.
    #[must_use]
    pub fn new(selection: Selection, table: TableHandle) -> Self {
        Self {
            selection,
            table,
            inspected: InspectedHandle::default(),
        }
    }

    /// This inspector, describing the table the window declares through
    /// `inspected` while the canvas holds one a later step makes.
    #[must_use]
    pub fn with_inspected(mut self, inspected: InspectedHandle) -> Self {
        self.inspected = inspected;
        self
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
///
/// The `VALUES` block below reads the rows, the bounds and the null count that
/// the grid pane's column header band now also draws, and it stays. The band
/// answers *what shape is this column* for every column at once, on sight; this
/// answers *tell me about this one column* for the column a click selected, and
/// it is the surface carrying the whole label, the tile and the reason the tile
/// was chosen. Neither is the other's abbreviation, so removing this would take
/// the answer to a question the band does not ask.
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
        // The run record's word for the step, which is `not run` until arc
        // records a run: a state derived from anything else would be a claim
        // about a run that did not happen.
        column_field(ui, mode, "status", t.state);
    }
}

/// A table a later step makes, as the inspector draws it: its name, the step
/// that makes it and where that step stands, then its columns, each with what
/// it is, once they have been read.
///
/// While the columns are being read the block says so, in [`reading_line`]'s
/// words, rather than drawing an empty list a reader would take for a table
/// with no columns.
fn table_body(ui: &mut egui::Ui, inspected: &InspectedTable, mode: Mode) {
    use crate::column_profiles::ProfileRead;
    let sem = semantic(mode.is_dark());
    let t = &inspected.table;
    ui.label(
        egui::RichText::new(&t.table)
            .font(ui_font())
            .color(chrome::colour(sem.text.primary)),
    );
    ui.label(
        egui::RichText::new("table")
            .font(ui_font())
            .color(chrome::colour(sem.text.secondary)),
    );
    ui.add_space(spacing::SPACE_4);
    ui.label(
        egui::RichText::new("PRODUCED BY")
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
    column_field(ui, mode, "step", &format!("{} · {}", t.step, t.kind));
    column_field(ui, mode, "status", t.state);

    ui.add_space(spacing::SPACE_4);
    ui.label(
        egui::RichText::new("COLUMNS · FROM THE ENGINE")
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
    match &inspected.columns {
        ProfileRead::Reading => {
            ui.label(
                egui::RichText::new(reading_line(&t.table))
                    .font(ui_font())
                    .color(chrome::colour(sem.text.secondary)),
            );
            // The worker wakes no one, so the rail looks again until the
            // answer is in.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        ProfileRead::Failed(reason) => {
            ui.label(
                egui::RichText::new(reason)
                    .font(ui_font())
                    .color(chrome::colour(sem.text.secondary)),
            );
        }
        ProfileRead::Read(columns) => {
            for column in columns.iter() {
                column_field(ui, mode, &column.column, column.full_type());
            }
        }
    }
}

/// What a step brightfield declared and nobody ran reads as: the protocol
/// view's own word for `SeamStatus::NotRun`, so the two rails cannot disagree
/// about it.
pub const NOT_RUN: &str = crate::protocol::NOT_RUN;

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
    /// answer of `Some` skips the call to `ui` for the frame, and the live
    /// document controls have to stay reachable whether or not the user has
    /// clicked on a pane yet.
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
        } else if let Some(inspected) = self.inspected.get() {
            // A table a later step makes, on the canvas: the dashboard's tiles
            // are of the file's table, so no tile can be selected for it, and
            // the table is what there is to describe.
            self.inspected.ask();
            table_body(ui, &inspected, cx.mode);
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
        // slider.
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
// The one inspector's sections
// ---------------------------------------------------------------------------

/// A section of the one inspector: a titled, collapsible part of the right
/// rail that describes one aspect of the selection.
///
/// The rail draws the sections [`sections_for`] gives, most specific first,
/// each with the design system's section header over its body. Not every
/// section has a body yet: the chart's Encoding, Reads and Fields are named
/// here so their order is fixed in one place, and [`sections_for`] lists a
/// section only once something draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    /// The operation a foot field or an operation form is asking.
    Operation,
    /// The selected column: what it is, its values, and what can be done
    /// with it.
    Column,
    /// A chart's channels and the fields on them.
    Encoding,
    /// The table a chart reads and the rows it draws.
    Reads,
    /// The fields a chart can use.
    Fields,
    /// A chart's live controls: its sliders.
    Parameters,
    /// The focused node's facts. Always last.
    Step,
}

impl Section {
    /// The section's title, as its header names it. The header draws it in
    /// capitals; the accessible name is this, as written.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Operation => "Operation",
            Self::Column => "Column",
            Self::Encoding => "Encoding",
            Self::Reads => "Reads",
            Self::Fields => "Fields",
            Self::Parameters => "Parameters",
            Self::Step => "Step",
        }
    }
}

/// What the window holds that decides which sections the inspector draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InspectorSelection {
    /// An operation is being asked: a foot field or an operation form is open.
    pub operation: bool,
    /// A column is selected, or a table a later step makes is on the canvas.
    pub column: bool,
    /// A chart document is open.
    pub chart: bool,
}

/// The sections the inspector draws for `selection`, in the order they stack:
/// Operation, then the column's, then the chart's, then Step, which is always
/// there and always last.
///
/// The chart's pane body draws its live controls under whatever it describes,
/// so a chart with a column selected is described by the Column section, and
/// one with nothing selected by Parameters.
#[must_use]
pub fn sections_for(selection: &InspectorSelection) -> Vec<Section> {
    let mut sections = Vec::new();
    if selection.operation {
        sections.push(Section::Operation);
    }
    if selection.chart {
        sections.push(if selection.column {
            Section::Column
        } else {
            Section::Parameters
        });
    }
    sections.push(Section::Step);
    sections
}

/// Whether each of `sections` is open, from what the reader set in `open`.
///
/// Every section is open until the reader closes it, except Step: it is open
/// only while no section above it is, so the focused node's facts give way to
/// whatever is more specific about the selection.
#[must_use]
pub fn sections_open(
    sections: &[Section],
    open: &std::collections::BTreeMap<Section, bool>,
) -> Vec<(Section, bool)> {
    let set = |s: Section| open.get(&s).copied().unwrap_or(true);
    let above_open = sections.iter().any(|s| *s != Section::Step && set(*s));
    sections
        .iter()
        .map(|s| {
            let is_open = if *s == Section::Step {
                set(Section::Step) && !above_open
            } else {
                set(*s)
            };
            (*s, is_open)
        })
        .collect()
}

/// Flip `section`, as a click or Enter on its header does, given how the
/// sections stood when it was drawn.
///
/// Opening Step closes the sections above it, since Step is open only while
/// none of them is.
pub fn toggle_section(
    section: Section,
    drawn: &[(Section, bool)],
    open: &mut std::collections::BTreeMap<Section, bool>,
) {
    let was_open = drawn
        .iter()
        .find(|(s, _)| *s == section)
        .is_some_and(|(_, o)| *o);
    open.insert(section, !was_open);
    if section == Section::Step && !was_open {
        for (s, _) in drawn {
            if *s != Section::Step {
                open.insert(*s, false);
            }
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
    /// the one it cannot — not the reverse, and not both or neither.
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

    /// **This rail never offers `save-spec`, whatever the window is.**
    ///
    /// The entry it would draw is the *editor's* — its own buffer's Save,
    /// declared on `EditorPane`'s subject — while the verb, dispatched, writes
    /// the **Protocol**. On a chart-spec window that is a dead button; on a
    /// data-file window with a dirty buffer it is worse, because the click
    /// saves the Protocol and reports success over an edit that was never
    /// written. One name, two writes, and this rail cannot tell a reader which
    /// it is about — so it draws neither, and the palette carries the Protocol
    /// save instead.
    #[test]
    fn dispatchable_never_offers_save_because_the_verb_means_two_writes() {
        let save = ToolbarEntry::button("editor-save", "Save", Verb::new("save-spec"));
        assert_eq!(
            dispatchable(std::slice::from_ref(&save)),
            Vec::new(),
            "the rail drew a Save button; the entry is the editor's and the \
             dispatch is the Protocol's, so whichever the reader meant, one of \
             the two silently does not happen"
        );
    }

    #[test]
    fn the_sections_stack_most_specific_first_with_step_last() {
        let all = InspectorSelection {
            operation: true,
            column: true,
            chart: true,
        };
        assert_eq!(
            sections_for(&all),
            [Section::Operation, Section::Column, Section::Step]
        );
        assert_eq!(
            sections_for(&InspectorSelection {
                chart: true,
                ..Default::default()
            }),
            [Section::Parameters, Section::Step]
        );
        assert_eq!(
            sections_for(&InspectorSelection::default()),
            [Section::Step]
        );
    }

    #[test]
    fn step_is_open_only_while_no_section_above_it_is() {
        let mut open = std::collections::BTreeMap::new();
        let sections = [Section::Parameters, Section::Step];
        let drawn = sections_open(&sections, &open);
        assert_eq!(drawn, [(Section::Parameters, true), (Section::Step, false)]);
        assert_eq!(
            sections_open(&[Section::Step], &open),
            [(Section::Step, true)]
        );

        // Opening Step closes the section above it.
        toggle_section(Section::Step, &drawn, &mut open);
        let drawn = sections_open(&sections, &open);
        assert_eq!(drawn, [(Section::Parameters, false), (Section::Step, true)]);

        // Opening the section above again folds Step away.
        toggle_section(Section::Parameters, &drawn, &mut open);
        assert_eq!(
            sections_open(&sections, &open),
            [(Section::Parameters, true), (Section::Step, false)]
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
