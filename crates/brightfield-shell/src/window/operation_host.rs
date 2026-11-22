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
use crate::operation_ui::open::{availability, Availability, InFront, OperationContext};
use crate::operation_ui::reads::DraftReads;
use crate::operation_ui::{self, Begin, Draft, Keeps, OperationUi, Start};

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
}

impl OperationHost {
    pub(super) const fn new() -> Self {
        Self {
            reads: None,
            said: None,
            said_now: false,
            in_grid: None,
            answers: 0,
        }
    }
}

impl MeridianApp {
    /// What the window holds now, as far as starting an operation asks.
    pub(crate) fn operation_context(&self) -> OperationContext {
        let model = &self.protocol.doc.model;
        let doc = &self.charts.doc;
        let source = model.source();
        let root = source.map(|s| s.name.clone());
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
        let in_front = (!table.is_empty() && !self.graph_on_canvas()).then(|| InFront {
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
    pub(crate) fn start_operation(
        &mut self,
        ctx: &egui::Context,
        spec: &'static OperationUi,
        start: Start,
        column: Option<String>,
    ) -> bool {
        // One operation is made at a time; starting another drops the first.
        self.drop_making(ctx);
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
            None => {
                let spelled = self.quoted_columns(&front.columns);
                let quote = |name: &str| {
                    spelled
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| format!("\"{}\"", name.replace('"', "\"\"")))
                };
                let transforms = brightfield_protocol::silver::FinetypeTransforms::vendored();
                let begin = Begin {
                    start,
                    table: Some(front.table.as_str()).filter(|t| !t.is_empty()),
                    column: column.as_deref(),
                    columns: &front.columns,
                    facts: &front.facts,
                    description: &description,
                    taken: &[],
                    model: None,
                    transforms: &transforms,
                    quote: &quote,
                };
                (spec.begin)(&begin)
            }
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
        self.charts.doc.set_making(Making {
            spec,
            draft,
            start,
            column,
            corrects: corrects.map(|step| step.name),
        });
        if spec.form.is_some() {
            self.open_rail_on_operation();
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
        self.drop_making(ctx);
        self.keep_draft(ctx, answer.name, answer.request, answer.model.model);
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
        let mut text = making.draft.value(focus).unwrap_or_default().to_string();
        let before = text.clone();
        let mut next = None;
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Tab) {
                let at = field.keys.iter().position(|k| *k == focus).unwrap_or(0);
                next = field.keys.get((at + 1) % field.keys.len()).copied();
            }
            if i.consume_key(Modifiers::NONE, Key::Backspace) {
                text.pop();
            }
            i.events.retain(|e| match e {
                egui::Event::Text(typed) => {
                    text.push_str(typed);
                    false
                }
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
        if let Some(runner) = self.runner.clone() {
            let wake = ctx.clone();
            let descriptions = self
                .descriptions
                .get_or_insert_with(|| Descriptions::with_wake(move || wake.request_repaint()));
            for spec in operation_ui::built() {
                let _ = descriptions.get(&runner, spec.long_name);
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
