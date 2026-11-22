//! The window's side of an operation: starting it, reading it as it is made,
//! keeping it, dropping it, and saying what it needs when it cannot start.
//!
//! **One entry point.** A key in the grid, a palette row, a header's gesture
//! and a menu item each come to [`MeridianApp::start_operation`], which works
//! in order and says the first thing in its way: no arc to record through, no
//! description from arc yet, a need the window does not meet
//! ([`availability`]), or the operation's own refusal to begin. The sentence
//! goes on the status band and stays until the next key or pointer press.
//!
//! **An operation that asks in the foot row** (`filter-rows`, `sort-rows`)
//! opens its field there, as its key did before. Any other becomes the
//! operation being made ([`Making`], on the document): its draft is read at
//! each edit ([`DraftReads`]), the grid shows the answer's page under the
//! preview label, and the Editor shows arc's model for it.
//!
//! **While an operation is being made**, Enter keeps it and Esc drops it. An
//! operation that asks for no value ([`Keeps::AtOnce`]) is kept on its first answer.
//! Keeping needs each value arc requires and an answer for the draft as it
//! stands; otherwise the status band says what is missing, or arc's or
//! DuckDB's reason, and the draft stays open. A kept draft is an unsaved step
//! ([`MeridianApp::keep_draft`]), and no file is written before a Save. A Save
//! pressed while a draft is open keeps it first when Enter would, and
//! otherwise saves the rest and leaves the draft open, saying why.
//!
//! **A column is spelled for the value it goes into**: bare for a value arc
//! takes as a name, and as SQL spells it for a value arc takes as SQL
//! ([`MeridianApp::quoted_columns`]).

use std::collections::{BTreeMap, BTreeSet};

use brightfield_workbench::{HideAffordance, StatusEntry, StatusSide, Tone};

use super::{CanvasHolds, MeridianApp, NodeView};
use crate::app::Making;
use crate::grid_foot::FootField;
use crate::operation::{DescriptionState, Descriptions, OperationDescription};
use crate::operation_ui::header::HeaderHit;
use crate::operation_ui::open::{availability, Availability, InFront, OperationContext};
use crate::operation_ui::reads::DraftReads;
use crate::operation_ui::{self, BegunWith, Draft, HeaderPart, Keeps, OperationUi, Start};
use crate::spine_pick::{CandidateSource, ColumnReads, SpinePick};
use brightfield_workbench::PaneKey;

/// The stable id of the status band's line for an operation: the handle a
/// test reads the line by.
pub const OPERATION_STATUS_ID: &str = "operation";

/// What an operation that removes duplicate rows says of a table that has no such rows.
pub const NO_DUPLICATE_ROWS: &str = "The table has no duplicate rows, so no step was kept.";

/// A value's key as a `'static` name, which a [`Draft`] holds its keys as. A
/// draft begun from a kept step's request takes its keys from arc's
/// description, which is read at run time, so each distinct key is kept for
/// the life of the process: a few dozen short names at most.
fn static_key(key: &str) -> &'static str {
    static KEYS: std::sync::Mutex<BTreeSet<&'static str>> = std::sync::Mutex::new(BTreeSet::new());
    let mut keys = KEYS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(held) = keys.get(key) {
        return held;
    }
    let leaked: &'static str = Box::leak(key.to_string().into_boxed_str());
    keys.insert(leaked);
    leaked
}

/// The stable id of the status band's line while the spine picks a table.
pub const SPINE_PICK_STATUS_ID: &str = "spine-pick";

/// **The spine's ask for an operation's table, while it is open**: the
/// operation waiting on it, the pick itself, and how the window stood before
/// the rail took the keys, so backing out puts it back.
pub(super) struct Picking {
    /// The operation the table is picked for.
    spec: &'static OperationUi,
    /// Where it started from.
    start: Start,
    /// The column it started on.
    column: Option<String>,
    /// The pick.
    pick: SpinePick,
    /// The reads of the candidates' columns.
    columns: ColumnReads,
    /// The pane that held the keys before the rail took them.
    focus_before: Option<PaneKey>,
    /// Whether the navigator rail was collapsed.
    rail_was_collapsed: bool,
    /// What the canvas held when the pick opened: the grid it is made over.
    began_over: CanvasHolds,
}

/// A table the spine handed an operation, with its columns.
type Taken = (String, Vec<String>);

/// The spine pick's `(keystroke token, longname)` pairs: every binding the
/// registry declares in its SpinePick context, the arrow twins included.
fn spine_pick_bindings() -> Vec<(&'static str, &'static str)> {
    brightfield_keys::registry()
        .iter()
        .flat_map(|verb| {
            verb.binding_specs
                .iter()
                .filter(|spec| spec.context == brightfield_keys::BindingContext::SpinePick)
                .map(move |spec| (spec.keystrokes, verb.longname))
        })
        .collect()
}

/// Consume the press of the key a SpinePick binding's token names, if it is
/// down this frame. The five tokens are spelled here, since the pick is the
/// one caller that takes `enter`, `escape` and `space` together.
fn consume_pick_token(ctx: &egui::Context, token: &str) -> bool {
    use egui::{Key, Modifiers};
    let key = match token {
        "j" => Key::J,
        "k" => Key::K,
        "down" => Key::ArrowDown,
        "up" => Key::ArrowUp,
        "space" => Key::Space,
        "enter" => Key::Enter,
        "escape" => Key::Escape,
        _ => return false,
    };
    ctx.input_mut(|i| {
        let pressed = i.consume_key(Modifiers::NONE, key);
        if pressed {
            // The text the key made goes with it, so `j` typed over the pick
            // reaches no field and no other handler.
            i.events.retain(|e| !matches!(e, egui::Event::Text(_)));
        }
        pressed
    })
}

/// The keys the registry binds to `longname` while the spine picks, as the
/// status band names them: `j`, or `j/down` for a verb with a twin.
fn pick_keys_of(longname: &str) -> String {
    spine_pick_bindings()
        .into_iter()
        .filter(|(_, name)| *name == longname)
        .map(|(token, _)| token)
        .collect::<Vec<_>>()
        .join("/")
}

/// What the window holds of an operation being made, beside the draft on the
/// document: its reads, and what of them the grid has been handed.
pub(super) struct OperationHost {
    /// The reads of the draft being made.
    reads: Option<DraftReads>,
    /// The sentence on the status band, and whether a press has been seen
    /// since it was said.
    said: Option<String>,
    /// Whether the sentence was said on this frame, so the press that caused
    /// it does not clear it.
    said_now: bool,
    /// What the grid was last handed: the answer's name, whether it was faded,
    /// and how many answers the reads had taken.
    in_grid: Option<(String, bool, u64)>,
    /// How many times the reads' answers have changed.
    answers: u64,
    /// What the canvas held when the draft began: the grid it is made over.
    began_over: Option<CanvasHolds>,
    /// The spine's ask for the operation's table, while one is open.
    picking: Option<Picking>,
    /// The model's tables the spine has handed an operation, each with the
    /// grid copy its rows are read from, which the operation's preview loads
    /// and so does the preview of a step kept from it.
    model_tables: Vec<(String, std::path::PathBuf)>,
}

impl OperationHost {
    pub(super) const fn new() -> Self {
        Self {
            reads: None,
            said: None,
            said_now: false,
            in_grid: None,
            answers: 0,
            began_over: None,
            picking: None,
            model_tables: Vec::new(),
        }
    }
}

impl MeridianApp {
    /// What the window holds now, as far as starting an operation asks.
    pub(crate) fn operation_context(&self) -> OperationContext {
        let model = &self.protocol.doc.model;
        let doc = &self.charts.doc;
        let source = model.source();
        // The table the grid lists: the Protocol's own, or, in a window with no Protocol of
        // its own, the one the session read.
        let root = source
            .map(|s| s.name.clone())
            .or_else(|| doc.operation_table().map(str::to_string));
        let preview = doc.grid_preview();
        // The table whose grid is in front: a step's, when the canvas holds
        // its grid, and otherwise the loaded table's.
        let step_in_front = match self.canvas_holds() {
            CanvasHolds::View {
                node,
                view: NodeView::Grid,
            } => model
                .label_of(node)
                .filter(|label| {
                    root.as_deref().map(str::to_lowercase) != Some(label.to_lowercase())
                })
                .map(str::to_string),
            _ => None,
        };
        let loaded = step_in_front.is_none() && preview.is_none_or(|p| p.is_typed());
        let table = match &step_in_front {
            Some(label) => model
                .generated()
                .and_then(|g| {
                    g.steps
                        .iter()
                        .find(|s| s.name.to_lowercase() == label.to_lowercase())
                })
                .map_or_else(|| label.clone(), |s| s.name.clone()),
            None => root.clone().unwrap_or_default(),
        };
        let columns: Vec<String> = if loaded {
            doc.grid_foot().columns.clone()
        } else {
            preview
                .and_then(|p| p.page())
                .map(|page| page.columns.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default()
        };
        let drew = doc.grid_drawn().is_some();
        let rows = drew && self.drawn_grid_layout() == crate::app::GridLayout::Rows;
        let cursor_column = doc.grid_cursor.and_then(|at| columns.get(at.col)).cloned();
        let in_front = ((!table.is_empty() || !columns.is_empty()) && !self.graph_on_canvas())
            .then(|| InFront {
                table: table.clone(),
                loaded,
                rows,
                cell: rows && cursor_column.is_some(),
                view_unrecorded: loaded && doc.grid_view_unrecorded(),
                facts: if loaded {
                    model.columns().to_vec()
                } else {
                    Vec::new()
                },
                cursor_column,
                step_offer: preview
                    .filter(|p| !p.is_typed())
                    .map(crate::data_grid::GridPreview::offer),
                columns,
            });
        let mut tables: Vec<(String, Option<Vec<String>>)> = Vec::new();
        if let Some(source) = source {
            tables.push((
                source.name.clone(),
                Some(source.columns.iter().map(|c| c.column.clone()).collect()),
            ));
        }
        for step in model.generated().into_iter().flat_map(|g| &g.steps) {
            tables.push((step.name.clone(), None));
        }
        // While the grid holds a table of a model, the model's other tables are the
        // Protocol's too: the ones the spine offers an operation that reads a second.
        for table in self.model_tables_beside_the_grid() {
            if !tables
                .iter()
                .any(|(held, _)| held.eq_ignore_ascii_case(&table))
            {
                tables.push((table, None));
            }
        }
        OperationContext {
            runner: self.runner.is_some(),
            protocol: source.map(|s| s.dir.clone()),
            takes_sources: self.protocol_taking_sources().is_some(),
            in_front,
            tables,
        }
    }

    /// Each column of the table in front as SQL spells it: bare when it is a
    /// plain name DuckDB does not read as a word, and quoted otherwise. A
    /// column this cannot ask the session about is quoted, which costs looks
    /// and reads the right column.
    fn quoted_columns(&mut self, columns: &[String]) -> BTreeMap<String, String> {
        let mut spelled = BTreeMap::new();
        if let Some(coordinator) = self.charts.doc.live_coordinator() {
            let session = coordinator.session();
            for column in columns {
                spelled.insert(
                    column.clone(),
                    crate::grid_foot::column_sql(session, column),
                );
            }
        }
        spelled
    }

    /// arc's description of `long_name`, once it has come: `Err` with the
    /// sentence to say while it has not, or when arc gave no description.
    fn description_of(&mut self, long_name: &str) -> Result<OperationDescription, String> {
        let Some(runner) = self.runner.clone() else {
            return Err(format!(
                "{} records through arc, and this window was started without it.",
                operation_ui::label(long_name)
            ));
        };
        let descriptions = self.descriptions.get_or_insert_with(Descriptions::default);
        match descriptions.get(&runner, long_name) {
            DescriptionState::Ready(description) => Ok(description.clone()),
            DescriptionState::Pending => {
                Err(format!("Reading arc's description of {long_name}\u{2026}"))
            }
            DescriptionState::Failed(why) => Err(format!("No description of {long_name}: {why}")),
        }
    }

    /// Put `sentence` on the status band until the next key or pointer press.
    fn say_operation(&mut self, ctx: &egui::Context, sentence: String) {
        self.operation.said = Some(sentence);
        self.operation.said_now = true;
        ctx.request_repaint();
    }

    /// What the status band says of an operation that could not start, could
    /// not be kept, or kept no step.
    #[must_use]
    pub fn operation_said(&self) -> Option<&str> {
        self.operation.said.as_deref()
    }

    /// The operation being made, while one is.
    #[must_use]
    pub fn making(&self) -> Option<&Making> {
        self.charts.doc.making()
    }

    /// The reads of the operation being made, while one is.
    #[doc(hidden)]
    #[must_use]
    pub fn draft_reads(&self) -> Option<&DraftReads> {
        self.operation.reads.as_ref()
    }

    /// Start the operation the registry verb `verb` names, from the grid's
    /// cursor: what a key, a palette row and a menu item come to. Whether it
    /// started; when it did not, [`Self::operation_said`] says why.
    pub(crate) fn start_operation_verb(&mut self, ctx: &egui::Context, verb: &str) -> bool {
        let Some(spec) = operation_ui::by_verb(verb) else {
            return false;
        };
        let start = spec.starts.first().copied().unwrap_or(Start::Cell);
        self.start_operation(ctx, spec, start, None)
    }

    /// [`Self::start_operation`] for a spec a test holds, on `column`.
    #[doc(hidden)]
    pub fn start_operation_spec(
        &mut self,
        ctx: &egui::Context,
        spec: &'static OperationUi,
        start: Start,
        column: Option<&str>,
    ) -> bool {
        self.start_operation(ctx, spec, start, column.map(str::to_string))
    }

    /// **Start `spec` from `start`**, on `column` when it is started on one and
    /// on the cursor's column otherwise. Whether it started.
    ///
    /// An operation that asks in the foot row opens its field there once the
    /// window meets its needs. For any other, in order, saying the first that
    /// fails: the window has arc; arc's description of the operation has come;
    /// the window meets the operation's needs; the operation begins its draft.
    /// It then becomes the operation being made, with its first edit sent to
    /// be read.
    ///
    /// An operation that starts from the spine opens the spine's pick in place
    /// of its draft, and answers `true`: the draft begins once a table is taken
    /// ([`Self::take_pick`]).
    pub(crate) fn start_operation(
        &mut self,
        ctx: &egui::Context,
        spec: &'static OperationUi,
        start: Start,
        column: Option<String>,
    ) -> bool {
        self.start_operation_taking(ctx, spec, start, column, None)
    }

    /// [`Self::start_operation`], with the tables the spine took for it once
    /// it has: `None` for a start that has not asked the spine.
    fn start_operation_taking(
        &mut self,
        ctx: &egui::Context,
        spec: &'static OperationUi,
        start: Start,
        column: Option<String>,
        taken: Option<Vec<Taken>>,
    ) -> bool {
        // One operation is made at a time; starting another drops the first,
        // and an ask the spine has open for one.
        self.drop_making(ctx);
        self.end_pick(ctx);
        // A field of the foot row writes its own text and a Save records it, so it opens
        // with no description from arc, and in a window with no arc to ask: the operation
        // panel beside it says what arc gave, or that it gave no description.
        if let Start::FootField(field) = start {
            let cx = self.operation_context();
            if let Availability::Unavailable(why) = availability(spec, &cx, column.as_deref()) {
                // Over a step's preview the foot row has its own line for the offer, which
                // the ask below raises; said here too it would stand on the band twice.
                let the_previews = cx
                    .in_front
                    .as_ref()
                    .and_then(|front| front.step_offer.as_deref())
                    == Some(why.as_str());
                if !the_previews {
                    self.say_operation(ctx, why);
                    return false;
                }
            }
            let asked = match field {
                FootField::Where => self.charts.doc.ask_where_at_cursor(),
                FootField::OrderBy => self.charts.doc.ask_order_at_cursor(),
            };
            if asked {
                self.open_rail_on_operation();
            }
            // A field that did not open says why on the foot row's own line.
            ctx.request_repaint();
            return asked;
        }
        let description = match self.description_of(spec.long_name) {
            Ok(description) => description,
            Err(why) => {
                self.say_operation(ctx, why);
                return false;
            }
        };
        let cx = self.operation_context();
        if let Availability::Unavailable(why) = availability(spec, &cx, column.as_deref()) {
            self.say_operation(ctx, why);
            return false;
        }
        let front = cx.in_front.clone().unwrap_or_default();
        let column = column.or_else(|| front.cursor_column.clone());

        // A kept step of the same operation in front is corrected: the draft
        // begins from what was kept, on the table that step is on.
        let corrects = self
            .typed_step_in_front()
            .filter(|step| {
                step.operation
                    .as_ref()
                    .is_some_and(|request| request.long_name == spec.long_name)
            })
            .cloned();
        // An operation that reads a second table asks for it in the spine first. A kept
        // step being corrected holds its tables already.
        if let (Start::Spine { role }, None, None) = (start, &taken, &corrects) {
            return self.open_pick(ctx, spec, (start, role), column, &description, &cx);
        }
        let model = taken.as_ref().and_then(|_| self.relationship_model());
        let with = BegunWith {
            start,
            table: Some(front.table.clone()).filter(|t| !t.is_empty()),
            column: column.clone(),
            spelled: self.quoted_columns(&front.columns),
            columns: front.columns.clone(),
            facts: front.facts.clone(),
            description,
            taken: taken.unwrap_or_default(),
            model,
        };
        let draft = match &corrects {
            Some(step) => {
                let request = step.operation.as_ref().expect("filtered on it");
                Ok(Draft {
                    long_name: spec.long_name,
                    on: request.on.clone(),
                    reads: request.reads.clone(),
                    values: request
                        .args
                        .iter()
                        .map(|(key, value)| (static_key(key), value.clone()))
                        .collect(),
                    focus: spec.field.as_ref().and_then(|f| f.keys.last().copied()),
                })
            }
            None => with.with_begin(|begin| (spec.begin)(begin)),
        };
        let draft = match draft {
            Ok(draft) => draft,
            Err(why) => {
                self.say_operation(ctx, why);
                return false;
            }
        };
        let Some(source) = self.typed_source(corrects.as_ref().map(|s| s.name.as_str())) else {
            self.say_operation(
                ctx,
                operation_ui::open::sentence(
                    operation_ui::Need::ProtocolSource,
                    &operation_ui::label(spec.long_name),
                    None,
                ),
            );
            return false;
        };
        let reads_tables = crate::typed_sql::tables_read(&draft.request());
        let wake = ctx.clone();
        self.operation.reads = Some(DraftReads::start_behind(
            source,
            reads_tables,
            self.charts.doc.preview_hooks().clone(),
            move || wake.request_repaint(),
        ));
        self.operation.in_grid = None;
        self.operation.began_over = Some(self.canvas_holds().clone());
        self.charts.doc.set_making(Making {
            spec,
            draft,
            start,
            column,
            corrects: corrects.map(|step| step.name),
            with,
            suggestions: Vec::new(),
            suggestion: None,
            answer: None,
            form_event: operation_ui::FormEvent::None,
        });
        if spec.form.is_some() {
            self.open_rail_on_form();
        }
        self.send_draft();
        ctx.request_repaint();
        true
    }

    /// The name the draft being made would be kept as: the kept step's it
    /// corrects, or the operation's rule over the draft, free of the names the
    /// Protocol and the kept steps hold.
    fn draft_name(&self, making: &Making) -> String {
        if let Some(name) = &making.corrects {
            return name.clone();
        }
        let model = &self.protocol.doc.model;
        let mut taken: BTreeSet<String> = model
            .generated()
            .map(|g| g.steps.iter().map(|s| s.name.clone()).collect())
            .unwrap_or_default();
        taken.extend(model.source().map(|s| s.name.clone()));
        taken.extend(self.charts.doc.typed_steps().iter().map(|s| s.name.clone()));
        let rule = &making.spec.name;
        crate::generated::effect_step_name(
            rule.word,
            making.draft.on.as_deref(),
            rule.last_part(&making.draft).as_deref(),
            rule.with_by,
            &taken,
        )
    }

    /// Send the draft as it stands to be read. The reads take the same
    /// request twice as one.
    fn send_draft(&mut self) {
        let Some(making) = self.charts.doc.making() else {
            return;
        };
        let (name, request) = (self.draft_name(making), making.draft.request());
        if let Some(reads) = &mut self.operation.reads {
            reads.edit(&name, request);
        }
    }

    /// Stop making the operation and keep no step: the reads end, the grid
    /// goes back to what it showed, and the rail a form moved goes back.
    fn drop_making(&mut self, ctx: &egui::Context) {
        if self.charts.doc.take_making().is_none() {
            return;
        }
        self.operation.reads = None;
        self.operation.in_grid = None;
        self.operation.began_over = None;
        self.typed_in_grid = None;
        self.restore_rail_after_ask();
        ctx.request_repaint();
    }

    /// **Keep the operation being made**, when it can be kept: each value arc
    /// requires is given, and the reads hold arc's model for the draft as it
    /// stands. Whether it was kept; when it was not, the status band says what
    /// is missing or why it was refused, and the draft stays open.
    pub(crate) fn keep_making(&mut self, ctx: &egui::Context) -> bool {
        let Some(making) = self.charts.doc.making().cloned() else {
            return false;
        };
        self.send_draft();
        if let Some(reads) = &mut self.operation.reads {
            reads.poll();
        }
        let missing = match self.description_of(making.spec.long_name) {
            Ok(description) => making.draft.missing(&description),
            Err(why) => Some(why),
        };
        if let Some(what) = missing {
            self.say_operation(ctx, what);
            return false;
        }
        let Some(reads) = &self.operation.reads else {
            return false;
        };
        let Some(answer) = reads.current().cloned() else {
            let why = reads.refusal().map_or_else(
                || "Still reading the step: press Enter again once it is shown.".to_string(),
                str::to_string,
            );
            self.say_operation(ctx, why);
            return false;
        };
        // A step the window refuses to keep, as over a join's rows, leaves the draft open
        // with what was typed; the banner says why.
        if !self.keep_draft(ctx, answer.name, answer.request, answer.model.model) {
            return false;
        }
        self.drop_making(ctx);
        true
    }

    /// **The keys of an operation being made**, ahead of the grid's foot row
    /// and the handlers after it: Enter keeps and Esc drops. An operation that
    /// asks in place on a header takes the typing too: text goes into the
    /// value in focus, Backspace takes a character back, and Tab moves to the
    /// next value its field edits.
    ///
    /// A sentence on the status band goes at the first key or pointer press
    /// after the frame it was said on.
    pub(super) fn operation_keys(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let pressed = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::PointerButton { pressed: true, .. }
                        | egui::Event::Text(_)
                )
            })
        });
        if pressed && !std::mem::take(&mut self.operation.said_now) {
            self.operation.said = None;
        }
        if !pressed {
            self.operation.said_now = false;
        }
        if self.overlay.is_some() || self.charts.doc.making().is_none() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.drop_making(ctx);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            // A suggestion the arrow keys are on is taken first, and Enter then keeps as it
            // would have.
            if let Some(making) = self.charts.doc.making_mut() {
                let taken = making
                    .suggestion
                    .take()
                    .and_then(|at| making.suggestions.get(at).cloned());
                let focus = making
                    .draft
                    .focus
                    .or_else(|| making.spec.field.and_then(|f| f.keys.first().copied()));
                if let (Some(taken), Some(focus)) = (taken, focus) {
                    making.draft.set(focus, taken);
                }
            }
            self.keep_making(ctx);
            return;
        }
        // A form's own text fields hold the keyboard while one is edited.
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let Some(making) = self.charts.doc.making_mut() else {
            return;
        };
        let Some(field) = making.spec.field else {
            return;
        };
        let Some(focus) = making.draft.focus.or_else(|| field.keys.first().copied()) else {
            return;
        };
        // egui reads Tab at the head of a pass as a request to move focus to the next widget
        // that takes it, which would then hold the keyboard and take the typing from the
        // field in place. The request is withdrawn ahead of the pass's widgets.
        if ctx.input(|i| i.events.iter().any(super::is_bare_tab)) {
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        let mut text = making.draft.value(focus).unwrap_or_default().to_string();
        let before = text.clone();
        let mut next = None;
        let offered = making.suggestions.len();
        ctx.input_mut(|i| {
            // Down and Up move through what the field offers; Tab takes the one they are on,
            // or moves to the next value the field edits.
            if offered > 0 && i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                making.suggestion = Some(making.suggestion.map_or(0, |at| (at + 1) % offered));
            }
            if offered > 0 && i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                making.suggestion = Some(
                    making
                        .suggestion
                        .map_or(offered - 1, |at| (at + offered - 1) % offered),
                );
            }
            if i.consume_key(Modifiers::NONE, Key::Tab) {
                match making
                    .suggestion
                    .take()
                    .and_then(|at| making.suggestions.get(at))
                {
                    Some(taken) => text.clone_from(taken),
                    None => {
                        let at = field.keys.iter().position(|k| *k == focus).unwrap_or(0);
                        next = field.keys.get((at + 1) % field.keys.len()).copied();
                    }
                }
            }
            if i.consume_key(Modifiers::NONE, Key::Backspace) {
                text.pop();
            }
            // A keystroke that makes text is the field's and no other handler's: its text
            // goes into the value, and its key press goes with it, so `o` typed into a name
            // does not also sort the rows. A key held with Cmd, Ctrl or Alt is the window's.
            i.events.retain(|e| match e {
                egui::Event::Text(typed) => {
                    text.push_str(typed);
                    false
                }
                egui::Event::Key {
                    pressed: true,
                    modifiers,
                    ..
                } => modifiers.command || modifiers.ctrl || modifiers.alt || modifiers.mac_cmd,
                _ => true,
            });
        });
        if text != before {
            making.draft.set(focus, text);
        }
        making.draft.focus = Some(next.unwrap_or(focus));
        if next.is_some() {
            ctx.request_repaint();
        }
    }

    /// **An operation's frame**: ask arc for the description of each built
    /// operation once the window has arc; send the draft being made to be read
    /// when it changed; take the answers; hand the grid the page to show; and
    /// keep an operation that asks for no value on its first answer.
    pub(super) fn operation_frame(&mut self, ctx: &egui::Context) {
        self.spine_pick_frame(ctx);
        if let Some(runner) = self.runner.clone() {
            let wake = ctx.clone();
            let descriptions = self
                .descriptions
                .get_or_insert_with(|| Descriptions::with_wake(move || wake.request_repaint()));
            for spec in operation_ui::built() {
                let _ = descriptions.get(&runner, spec.long_name);
            }
        }
        // A header's menu greys a row by what the window holds, told to it each frame an
        // operation starts from a header.
        let context = self
            .charts
            .doc
            .header_has_operations()
            .then(|| self.operation_context());
        self.charts.doc.set_header_context(context);
        // A gesture on a part of a header, or a choice from its menu, starts its operation.
        if let Some(hit) = self.charts.doc.take_header_hit() {
            self.header_hit(ctx, &hit);
        }
        // What a form asked for on the frame it was last drawn.
        let asked =
            self.charts.doc.making_mut().map(|making| {
                std::mem::replace(&mut making.form_event, operation_ui::FormEvent::None)
            });
        match asked {
            Some(operation_ui::FormEvent::Keep) => {
                self.keep_making(ctx);
            }
            Some(operation_ui::FormEvent::Cancel) => self.drop_making(ctx),
            _ => {}
        }
        if let Some(making) = self.charts.doc.making_mut() {
            // What the field in place offers for the draft as it stands.
            if let Some(field) = making.spec.field {
                let offered = making
                    .with
                    .with_begin(|begin| (field.suggestions)(begin, &making.draft));
                if offered != making.suggestions {
                    making.suggestions = offered;
                    making.suggestion = None;
                }
            }
        }
        // **A draft is made over the grid it began on, and goes when that grid does.** Its
        // keys and its page are that grid's: once the canvas holds another view, the graph, or
        // a field of the foot row takes the keyboard, the typing would go to a draft no longer
        // on screen, so the draft is dropped as Esc drops it
        // (`a_draft_goes_when_the_grid_it_began_over_does` in `tests/operation_start.rs`).
        if self.charts.doc.making().is_some() {
            let moved = self.operation.began_over.as_ref() != Some(self.canvas_holds())
                || self.graph_on_canvas()
                || self.charts.doc.foot_asking().is_some();
            if moved {
                self.drop_making(ctx);
            }
        }
        let Some(making) = self.charts.doc.making().cloned() else {
            self.operation.reads = None;
            return;
        };
        self.send_draft();
        let Some(reads) = &mut self.operation.reads else {
            return;
        };
        if reads.poll() {
            self.operation.answers += 1;
            let answer = reads.current().cloned();
            if let Some(making) = self.charts.doc.making_mut() {
                making.answer = answer;
            }
            ctx.request_repaint();
        }
        let (refusal, current) = (
            reads.refusal().map(str::to_string),
            reads.current().cloned(),
        );
        if making.spec.keeps == Keeps::AtOnce {
            if let Some(why) = refusal {
                self.drop_making(ctx);
                self.say_operation(ctx, why);
                return;
            }
            if let Some(answer) = current {
                self.drop_making(ctx);
                // As many rows out as in: no duplicate was found.
                if making.spec.long_name == "dedupe-rows"
                    && answer.unread.is_none()
                    && answer.rows == answer.rows_in
                {
                    self.say_operation(ctx, NO_DUPLICATE_ROWS.to_string());
                } else {
                    // A refusal to keep is on a banner; the action asked for no value, so
                    // there is no draft to leave open.
                    self.keep_draft(ctx, answer.name, answer.request, answer.model.model);
                }
            }
            return;
        }
        // The grid is handed the last good page when it, or its fade, changes.
        let Some(reads) = &self.operation.reads else {
            return;
        };
        let Some(shown) = reads.shown() else {
            return;
        };
        let stamp = (shown.name.clone(), reads.faded(), self.operation.answers);
        if self.operation.in_grid.as_ref() != Some(&stamp) {
            let preview = crate::data_grid::GridPreview::of_draft(
                shown.name.clone(),
                shown.page.clone(),
                shown.missing.clone(),
                reads.faded(),
            );
            self.charts.doc.show_typed_preview(preview);
            self.typed_in_grid = Some(format!("operation {}", shown.name));
            self.operation.in_grid = Some(stamp);
            ctx.request_repaint();
        }
    }

    /// **Start the operation a header's gesture names**: the one built operation that starts
    /// from the part, on the header's column, or the one chosen from the header's menu. A
    /// secondary click starts no operation: it opened the menu.
    fn header_hit(&mut self, ctx: &egui::Context, hit: &HeaderHit) {
        let ops = self.charts.doc.header_operations();
        let spec = match hit.verb {
            Some(verb) => ops.iter().copied().find(|op| op.verb == verb),
            None if hit.part == HeaderPart::Menu => None,
            None => operation_ui::header::the_one_from(&ops, hit.part),
        };
        let Some(spec) = spec else {
            return;
        };
        // An operation chosen from the menu starts from the part it names, when it names one.
        let start = spec
            .starts
            .iter()
            .copied()
            .find(|s| matches!(s, Start::Header(part) if *part == hit.part || hit.verb.is_some()))
            .unwrap_or(Start::Header(hit.part));
        self.start_operation(ctx, spec, start, hit.column.clone());
    }

    /// Open the Inspector rail on the operation panel for a form, remembering how the rail
    /// stood so that keeping or dropping the operation puts it back.
    fn open_rail_on_form(&mut self) {
        let Some(at) = Self::operation_pane_index() else {
            return;
        };
        self.rail_before_ask = Some(super::RailBeforeAsk {
            collapsed: self
                .collapsed
                .remove(&brightfield_workbench::arrangement::INSPECTOR_RAIL),
            panel: self.inspector_panel,
        });
        self.inspector_panel = at;
    }

    /// The step the grid shows a draft's page under, while it shows one.
    pub(super) fn making_grid_step(&self) -> Option<String> {
        self.charts.doc.making()?;
        self.operation
            .in_grid
            .as_ref()
            .map(|(name, ..)| name.clone())
    }

    /// The SQL of the draft being made, for the Editor: the name it would be
    /// kept as and arc's model for the last edit that read.
    pub(super) fn making_sql(&self) -> Option<(String, String)> {
        self.charts.doc.making()?;
        let shown = self.operation.reads.as_ref()?.shown()?;
        Some((shown.name.clone(), shown.model.model.clone()))
    }

    /// **The status band's line for an operation.** A sentence said leads it.
    /// While an operation is being made: arc's or DuckDB's refusal of the
    /// draft as it stands, in the warning tone; else arc's notes; else why the
    /// rows were not read; else the operation, its table and how many rows it
    /// reads and makes.
    pub(super) fn operation_status_entry(&self) -> Option<StatusEntry> {
        let entry = |text: String, tone: Tone| StatusEntry {
            id: OPERATION_STATUS_ID,
            side: StatusSide::Leading,
            text,
            tone,
            hide: HideAffordance::WithRail,
        };
        if let Some(said) = &self.operation.said {
            return Some(entry(said.clone(), Tone::Warning));
        }
        if let Some(picking) = self.spine_pick_status() {
            return Some(StatusEntry {
                id: SPINE_PICK_STATUS_ID,
                ..entry(picking, Tone::Neutral)
            });
        }
        let making = self.charts.doc.making()?;
        let reads = self.operation.reads.as_ref()?;
        if let Some(why) = reads.refusal() {
            return Some(entry(why.to_string(), Tone::Warning));
        }
        let answer = reads.current()?;
        if !answer.model.notes.is_empty() {
            return Some(entry(answer.model.notes.join(" "), Tone::Neutral));
        }
        if let Some(unread) = &answer.unread {
            return Some(entry(unread.clone(), Tone::Neutral));
        }
        let on = making.draft.on.as_deref().unwrap_or_default();
        Some(entry(
            format!(
                "{} on {on} \u{b7} {} \u{2192} {} rows",
                making.spec.long_name,
                crate::column_header::thousands(answer.rows_in),
                crate::column_header::thousands(answer.rows)
            ),
            Tone::Neutral,
        ))
    }

    /// **The Protocol's model of its tables' relationships**: the model pane's,
    /// while a table of its Protocol is in the grid, and otherwise the model
    /// file in the Protocol's folder, when it holds one Brightfield reads.
    fn relationship_model(&self) -> Option<brightfield_ossie::Model> {
        if self.model_behind_grid() {
            return self.model.as_ref().map(|m| m.doc.model.clone());
        }
        let dir = &self.protocol.doc.model.source()?.dir;
        let text = std::fs::read_to_string(dir.join(crate::discovery::MODEL_FILE)).ok()?;
        brightfield_ossie::read(&text).ok()
    }

    /// The model's tables that have rows to read, while the grid holds one of
    /// them: none while it holds a join's rows, whose spine the rail does not
    /// draw, and none in a window with no model pane.
    fn model_tables_beside_the_grid(&self) -> Vec<String> {
        let Some(model) = self.model.as_ref().filter(|m| m.grid_table.is_some()) else {
            return Vec::new();
        };
        let pane = &model.doc;
        pane.model
            .datasets
            .iter()
            .filter(|d| crate::model_inspector::materialised(&pane.model, &d.name))
            .map(|d| d.name.clone())
            .collect()
    }

    /// The Protocol whose model's table the grid holds, when it holds one.
    fn model_protocol_beside_the_grid(&self) -> Option<&crate::own_files::OwnProtocol> {
        self.model
            .as_ref()
            .filter(|m| m.grid_table.is_some())
            .map(|m| &m.doc.protocol)
    }

    /// **The tables the spine lists for an operation to read**, each with its
    /// columns where they are known and where its rows are read from: the
    /// tables `cx` holds, which are the ones the operation's needs were checked
    /// against (`availability`). A table of the model the grid's table belongs
    /// to is read from its grid copy; any other is the loaded table or a step
    /// after it.
    fn pick_tables(&self, cx: &OperationContext) -> Vec<crate::spine_pick::Listed> {
        let of_model = self.model_tables_beside_the_grid();
        let protocol = self.model_protocol_beside_the_grid();
        cx.tables
            .iter()
            .map(|(table, columns)| {
                let source = match protocol.filter(|_| of_model.contains(table)) {
                    Some(protocol) => CandidateSource::ModelTable {
                        grid_copy: crate::own_files::grid_copy_path(protocol, table),
                    },
                    None => CandidateSource::Step,
                };
                (table.clone(), columns.clone(), source)
            })
            .collect()
    }

    /// The model's tables the spine has handed an operation, each with the
    /// grid copy it is read from: those picked in the Protocol whose table the
    /// grid holds, and never the table the grid itself holds.
    pub(super) fn picked_model_tables(&self) -> Vec<(String, std::path::PathBuf)> {
        let Some(model) = self.model.as_ref().filter(|m| m.grid_table.is_some()) else {
            return Vec::new();
        };
        let in_grid = model.grid_table.clone().unwrap_or_default();
        let folder = model
            .doc
            .protocol
            .dir
            .join(crate::model_inspector::GRID_DIR);
        self.operation
            .model_tables
            .iter()
            // A table picked in another Protocol is that Protocol's, whatever its name.
            .filter(|(table, path)| {
                path.starts_with(&folder) && !table.eq_ignore_ascii_case(&in_grid)
            })
            .cloned()
            .collect()
    }

    /// **Open the spine's pick for `spec`**: the rail takes the keys, with its
    /// focus ring, and lists the tables the operation can read. Whether it
    /// opened; a Protocol with no other table says so and opens nothing.
    fn open_pick(
        &mut self,
        ctx: &egui::Context,
        spec: &'static OperationUi,
        (start, role): (Start, &'static str),
        column: Option<String>,
        description: &OperationDescription,
        cx: &OperationContext,
    ) -> bool {
        let front = cx.in_front.clone().unwrap_or_default();
        let tables = self.pick_tables(cx);
        let copies = self.model_protocol_beside_the_grid().cloned();
        let relationships: Vec<(String, String)> = self
            .relationship_model()
            .map(|model| {
                model
                    .relationships
                    .iter()
                    .map(|r| (r.from.clone(), r.to.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let found = crate::spine_pick::candidates(&tables, &front.table, &relationships);
        if found.is_empty() {
            self.say_operation(
                ctx,
                operation_ui::open::sentence(
                    operation_ui::Need::AnotherTable,
                    &operation_ui::label(spec.long_name),
                    None,
                ),
            );
            return false;
        }
        let list = description
            .reads
            .iter()
            .find(|r| r.name == role)
            .is_some_and(|r| r.list);
        let pick = SpinePick::new(role, list, &front.table, column.as_deref(), found);
        let wake = ctx.clone();
        let columns = crate::spine_pick::read_columns(
            self.typed_source(None),
            copies,
            pick.candidates(),
            self.charts.doc.preview_hooks().clone(),
            move || wake.request_repaint(),
        );
        // The band's line is the pick's now: a sentence said by an earlier
        // ask, as while arc's description was on its way, is over.
        self.operation.said = None;
        let rail = PaneKey::new(crate::protocol::OUTLINE);
        let focus_before = self.focused_pane();
        self.ws_mut().set_focus(rail);
        self.operation.picking = Some(Picking {
            spec,
            start,
            column,
            pick,
            columns,
            focus_before,
            rail_was_collapsed: self
                .collapsed
                .remove(&brightfield_workbench::arrangement::NAVIGATOR_RAIL),
            began_over: self.canvas_holds().clone(),
        });
        ctx.request_repaint();
        true
    }

    /// The spine's pick, while it is open.
    #[must_use]
    pub fn spine_pick(&self) -> Option<&SpinePick> {
        self.operation.picking.as_ref().map(|p| &p.pick)
    }

    /// **Close the spine's pick and begin nothing**: the keys go back to the
    /// pane that held them, and the rail is put away again if it was.
    fn end_pick(&mut self, ctx: &egui::Context) {
        let Some(picking) = self.operation.picking.take() else {
            return;
        };
        let rail = PaneKey::new(crate::protocol::OUTLINE);
        // The keys go back only if the rail still holds them: a press on
        // another pane since has already said where they go.
        if self.focused_pane() == Some(rail) {
            self.ws_mut().clear_focus();
            if let Some(before) = picking.focus_before {
                self.ws_mut().set_focus(before);
            }
        }
        if picking.rail_was_collapsed {
            self.collapsed
                .insert(brightfield_workbench::arrangement::NAVIGATOR_RAIL);
        }
        self.protocol.doc.spine_pick = None;
        ctx.request_repaint();
    }

    /// **Take the picked tables and begin the operation with them**: the added
    /// tables in order, or the one under the cursor. A table whose columns are
    /// still being read is not taken yet, and the status band says to press
    /// Enter again; the operation's `begin` is handed each table with its
    /// columns.
    fn take_pick(&mut self, ctx: &egui::Context) {
        let Some(picking) = self.operation.picking.as_mut() else {
            return;
        };
        let tables = picking.pick.would_take();
        if let Some(waiting) = tables
            .iter()
            .find(|table| picking.pick.columns_of(table).is_none())
        {
            let why =
                format!("Still reading {waiting}'s columns: press Enter again once they are read.");
            self.say_operation(ctx, why);
            return;
        }
        let taken: Vec<Taken> = tables
            .into_iter()
            .map(|table| {
                let columns = picking
                    .pick
                    .columns_of(&table)
                    .map(<[String]>::to_vec)
                    .unwrap_or_default();
                (table, columns)
            })
            .collect();
        let from_model: Vec<(String, std::path::PathBuf)> = taken
            .iter()
            .filter_map(|(table, _)| match &picking.pick.candidate(table)?.source {
                CandidateSource::ModelTable { grid_copy } => {
                    Some((table.clone(), grid_copy.clone()))
                }
                CandidateSource::Step => None,
            })
            .collect();
        let (spec, start, column) = (picking.spec, picking.start, picking.column.clone());
        // A model's table is read from a copy of its rows, written now for the tables
        // taken and no other, as opening a table's grid writes its own.
        if let Some(protocol) = self.model_protocol_beside_the_grid().cloned() {
            for (table, _) in &from_model {
                if let Err(why) = crate::own_files::grid_copy(&protocol, table) {
                    self.say_operation(ctx, format!("{table} could not be read: {why}"));
                    return;
                }
            }
        }
        for (table, path) in from_model {
            self.operation
                .model_tables
                .retain(|(held, _)| *held != table);
            self.operation.model_tables.push((table, path));
        }
        self.end_pick(ctx);
        self.start_operation_taking(ctx, spec, start, column, Some(taken));
    }

    /// **The spine pick's keys**, ahead of every other handler: `j` and `k`,
    /// and the arrows, move the cursor a table; `Space` adds the table under it
    /// to a list; `Enter` takes; `Esc` backs out. Each binding comes off the
    /// registry's SpinePick context, so the keys the status band names are the
    /// keys that act. A key is consumed whether or not the cursor could move,
    /// so a move off the list's end does not fall through to another pane's
    /// verb.
    pub(super) fn spine_pick_keys(&mut self, ctx: &egui::Context) {
        if self.operation.picking.is_none()
            || self.overlay.is_some()
            || ctx.egui_wants_keyboard_input()
        {
            return;
        }
        for (token, longname) in spine_pick_bindings() {
            if !consume_pick_token(ctx, token) {
                continue;
            }
            // A sentence said by the key before stays until this one.
            self.operation.said = None;
            match longname {
                "take-picked-table" => {
                    self.take_pick(ctx);
                    return;
                }
                "back-out-of-pick" => {
                    self.end_pick(ctx);
                    return;
                }
                _ => {}
            }
            let Some(picking) = self.operation.picking.as_mut() else {
                return;
            };
            match longname {
                "pick-next-table" => picking.pick.next(),
                "pick-previous-table" => picking.pick.previous(),
                "add-table-to-pick" if !picking.pick.add() => {
                    let why = format!(
                        "{} reads one table: press Enter to take the one under the cursor.",
                        operation_ui::label(picking.spec.long_name)
                    );
                    self.say_operation(ctx, why);
                }
                _ => {}
            }
            ctx.request_repaint();
        }
    }

    /// **The spine pick's frame**: the columns read since the last are noted,
    /// a click on a table's row puts the cursor on it, and the spine is told
    /// what to show. The pick closes, beginning nothing, once the grid it
    /// opened over is no longer on the canvas or another pane has taken the
    /// keys: its keys are the rail's, and the table it is for is that grid's.
    fn spine_pick_frame(&mut self, ctx: &egui::Context) {
        let clicked = self.protocol.doc.take_spine_pick_click();
        let Some(picking) = self.operation.picking.as_mut() else {
            self.protocol.doc.spine_pick = None;
            return;
        };
        for (table, columns) in picking.columns.poll() {
            picking.pick.set_columns(&table, columns);
            ctx.request_repaint();
        }
        if let Some(table) = clicked {
            picking.pick.put_cursor_on(&table);
        }
        let began_over = picking.began_over.clone();
        let moved = began_over != *self.canvas_holds()
            || self.graph_on_canvas()
            || self.charts.doc.foot_asking().is_some()
            || self.focused_pane() != Some(PaneKey::new(crate::protocol::OUTLINE));
        if moved {
            self.end_pick(ctx);
            return;
        }
        let Some(picking) = self.operation.picking.as_ref() else {
            return;
        };
        let pick = &picking.pick;
        self.protocol.doc.spine_pick = Some(crate::protocol::SpinePickRows {
            in_front: pick.in_front().to_string(),
            candidates: pick
                .candidates()
                .iter()
                .map(|c| (c.table.clone(), c.trail()))
                .collect(),
            cursor: pick.under_cursor().map(|c| c.table.clone()),
            added: pick.added().to_vec(),
        });
    }

    /// The status band's line while the spine picks: the operation, and the
    /// pick's five keys as the registry binds them.
    fn spine_pick_status(&self) -> Option<String> {
        let picking = self.operation.picking.as_ref()?;
        Some(format!(
            "{}: pick a table in the spine \u{b7} {} next \u{b7} {} previous \u{b7} {} add to a list \u{b7} {} take \u{b7} {} back",
            operation_ui::label(picking.spec.long_name),
            pick_keys_of("pick-next-table"),
            pick_keys_of("pick-previous-table"),
            pick_keys_of("add-table-to-pick"),
            pick_keys_of("take-picked-table"),
            pick_keys_of("back-out-of-pick"),
        ))
    }

    /// Draw frames' worth of the operation's reads until the draft as it
    /// stands is answered or refused, for at most `patience`. Whether it
    /// settled.
    #[doc(hidden)]
    pub fn finish_operation_reads(&mut self, patience: std::time::Duration) -> bool {
        let until = std::time::Instant::now() + patience;
        loop {
            self.send_draft();
            let Some(reads) = &mut self.operation.reads else {
                return true;
            };
            if reads.poll() {
                self.operation.answers += 1;
            }
            if reads.settled() {
                return true;
            }
            if std::time::Instant::now() >= until {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}
