//! The model pane: the tables an own-files open found and the joins between them, shown three
//! ways — a diagram, a list, or the Ossie file the model is written as.
//!
//! The pane reads one document, [`ModelPane`]: the Protocol folder the open made, the model
//! written beside it, which of the three views is showing and what is selected. A summary line
//! heads every view, and when the open passed over files, a second line names each one with its
//! reason. `v` steps through the views (`cycle-model-view` in the keyboard registry), and the
//! palette names each view on its own.
//!
//! The diagram is the model scene ([`brightfield_render::model_scene::render_model`]) rastered
//! through Vello into a slot of the window's canvas host, as an arc Protocol's panel tiles are;
//! a click on it selects what [`brightfield_render::model_scene::hit_test`] finds under the
//! pointer. [`ModelPane::save_model`] is how a changed model reaches the folder: it writes the
//! Ossie file, rewrites the silver steps for it and runs the Protocol again. After each run, and
//! when the pane opens over a Protocol already run, each join's evidence is measured again over the
//! silver tables ([`measure_joins`]); a join the data no longer bears out carries a warning, which
//! its pill and its inspector show. Rediscover ([`REDISCOVER_MODEL`]) runs discovery again and
//! folds what it proposes around what the analyst decided.
//!
//! Discovery's full pass runs on a worker, after the open and after each Rediscover, while the
//! pane shows the sample pass's model and the analyst works on it. The window polls it each frame
//! ([`ModelPane::poll_discovery`]); when it lands, it is folded into the model as it stands then,
//! once any hand edit waiting in the file view has been read, and saved when that changes it.
//!
//! What is selected belongs to the pane, not to a view: a click on a card or a join on the
//! diagram, or on a row of the list, selects it, and in either view what is not connected to the
//! selection dims. While something is selected the inspector ([`crate::model_inspector`]) stands
//! beside the view, and the actions it offers come back to [`ModelPane::dispatch`].
//!
//! The reshape verbs ([`RESHAPE_VERBS`]) change the model from the keyboard and the palette as the
//! inspector's buttons do from the pointer: each one names the selection to
//! [`ModelPane::reshape_action`], and the action it answers goes through the same dispatch, which
//! applies the edit, writes the file, rewrites silver and runs the Protocol before the next frame
//! draws. A drag from one card to another on the diagram opens the add-join picker between them.
//! What came of the last action stands in a status line under the summary.
//!
//! The file view is an editor over the Ossie file. Each pause in typing, [`FILE_PAUSE_MS`] after
//! the last change, reads the text through [`crate::model_file::read_hand_edit`]: a text
//! Brightfield reads goes through [`ModelPane::save_model`] as an action's model does, with what
//! the edit changed marked as the analyst's; one it cannot read leaves the model standing and
//! says why, at its line, under the editor. Under it too stands a warning for each thing the file
//! holds that Brightfield does not act on. The window drives the pause with
//! [`ModelPane::tick_file_edit`]. Shown with something selected, the file scrolls to it.
//!
//! The editor's text is the analyst's while the editor holds the keyboard: an applied edit writes
//! the file as the writer emits it, and the editor takes that text only once it lets go of the
//! keyboard with no refused edit in it. A reshape reads a waiting edit first, and is refused while
//! the editor holds one that does not read; Discard edit puts the file back in the editor.

use std::path::{Path, PathBuf};

use brightfield_engine::semantic::SchemaCatalogue;
use brightfield_keys::{Altitude, BindingContext};
use brightfield_ossie::{merge_discovery, Edit, Evidence, ListRow, Model, ModelLayout};
use brightfield_render::canvas_host::{Color, PixelSize};
use brightfield_render::model_scene::{
    dataset_lit, hit_test, relationship_lit, render_model, ModelFocus, Selection,
};
use brightfield_workbench::registry::Slot;
use brightfield_workbench::{
    chrome, EmptyState, Handled, Icon, Item, ItemCtx, ItemId, ItemRegistry, ItemSpec, PaneKey,
    Subject, Verb,
};
use meridian_design::{semantic, spacing};

use crate::canvas::EguiCanvasHost;
use crate::design::Mode;
use crate::discovery::{sample_pass, write_model_file, Discovery, FullPass, StandIn, MODEL_FILE};
use crate::editor::Access;
use crate::model_file::{self, Kept};
use crate::model_inspector::{self, Inspected, InspectorDrawn, ModelAction, ModelPicker};
use crate::own_files::{self, OwnProtocol};
use crate::run::Runner;

/// The model pane's item id.
pub const MODEL: ItemId = ItemId::new("model-pane");

/// The slot the diagram's raster is filed under in the canvas host.
const DIAGRAM_SLOT: PaneKey = PaneKey::new(MODEL);

const ICON_MODEL: Icon = Icon("model");

/// The verb that steps through the three views.
pub const CYCLE_MODEL_VIEW: &str = "cycle-model-view";
/// The verbs that show one view each, in the order `v` steps through them.
const SHOW_VIEW: [(&str, ModelView); 3] = [
    ("show-model-diagram", ModelView::Diagram),
    ("show-model-list", ModelView::List),
    ("show-model-file", ModelView::File),
];

/// How long the file view's text must rest after an edit before it is read, in milliseconds: the
/// shell's typing pause, [`crate::typed_sql::TYPED_PAUSE`].
#[allow(clippy::cast_possible_truncation)] // 300 ms fits a u64
pub const FILE_PAUSE_MS: u64 = crate::typed_sql::TYPED_PAUSE.as_millis() as u64;

/// The width of the Discard edit control at the end of a refused edit's error line.
const DISCARD_WIDTH: f32 = 96.0;

/// Which way the model is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelView {
    /// The tables as cards in role bands, with the joins drawn between them.
    #[default]
    Diagram,
    /// One row per table, grouped by role, with a gutter naming its joins.
    List,
    /// The Ossie file, in an editor the analyst can change it in.
    File,
}

impl ModelView {
    /// The view `v` steps to from this one: diagram, list, file, and round again.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            ModelView::Diagram => ModelView::List,
            ModelView::List => ModelView::File,
            ModelView::File => ModelView::Diagram,
        }
    }

    /// The name the pane's view switch shows.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ModelView::Diagram => "Diagram",
            ModelView::List => "List",
            ModelView::File => "Ossie file",
        }
    }
}

/// The verbs the model pane's palette offers: every verb the registry scopes to the model
/// altitude, in the registry's order. Each of them changes the view when chosen.
#[must_use]
pub fn model_palette_verbs() -> Vec<&'static str> {
    brightfield_keys::registry()
        .iter()
        .filter(|v| v.applies_at(Altitude::Model))
        .map(|v| v.longname)
        .collect()
}

/// The verbs that change the model, each acting on the selection.
pub const RESHAPE_VERBS: [&str; 10] = [
    "keep-join",
    "drop-join",
    "add-join",
    "change-join-keys",
    "set-role",
    "fold-table",
    "unfold-table",
    "leave-out-table",
    "include-table",
    "retype-column",
];

/// The verb that runs discovery again and folds what it proposes around the analyst's decisions.
pub const REDISCOVER_MODEL: &str = "rediscover-model";

/// Whether `longname` is one of the model pane's verbs.
#[must_use]
pub fn is_model_verb(longname: &str) -> bool {
    longname == CYCLE_MODEL_VIEW
        || longname == REDISCOVER_MODEL
        || SHOW_VIEW.iter().any(|(verb, _)| *verb == longname)
        || RESHAPE_VERBS.contains(&longname)
}

/// The model pane's registry: its one pane, the centre of the model document.
#[must_use]
pub fn model_registry() -> ItemRegistry<ModelPane> {
    ItemRegistry::new(vec![ItemSpec {
        id: MODEL,
        slot: Slot::Centre,
        toggle: None,
        make: || Box::new(ModelItem),
    }])
}

/// What the pane drew in the last frame it was drawn in: the view, and the text of each line it
/// handed the painter, in order. Recorded so a test reads what was drawn rather than what the
/// document holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelDrawn {
    /// The view the body was drawn as.
    pub view: Option<ModelView>,
    /// The summary line and, when files were passed over, the line naming them.
    pub header: Vec<String>,
    /// The body's lines: the list's rows or the file's lines. Empty for the diagram.
    pub body: Vec<String>,
    /// Where the diagram's raster was laid out, in window-space logical points.
    pub diagram: Option<egui::Rect>,
    /// Where each list row was laid out, in the order of [`ModelDrawn::body`].
    pub list_rows: Vec<egui::Rect>,
    /// The joins the list's gutter routed between its rows.
    pub routes: Vec<RouteDrawn>,
    /// Whether each list row was drawn lit, in the order of [`ModelDrawn::body`]: with a
    /// selection, a row not connected to it is drawn dimmed
    /// (`a_list_row_selects_and_the_rest_dims` in `tests/model_inspector.rs`).
    pub list_lit: Vec<bool>,
    /// What the inspector drew, when something was selected.
    pub inspector: Option<InspectorDrawn>,
    /// Where the file view's editor was laid out, when the file view was drawn.
    pub file_editor: Option<egui::Rect>,
    /// Where the Discard edit control beside a refused edit's error line was laid out.
    pub file_discard: Option<egui::Rect>,
    /// The line the file view scrolled to this frame, 1-based: the selection's, the first frame
    /// the file view shows it.
    pub file_scrolled_to: Option<usize>,
    /// The lines under the file view's editor: why the last text could not be read, then each
    /// warning.
    pub file_notes: Vec<String>,
}

/// What the diagram's raster depends on: when any of it changes, the scene is drawn again.
#[derive(Debug, Clone, PartialEq)]
struct DiagramKey {
    generation: u64,
    dev: PixelSize,
    dark: bool,
    selected: Option<Selection>,
}

/// The model document: the Protocol folder an open made, its model, and how it is shown.
pub struct ModelPane {
    /// The Protocol folder the model belongs to.
    pub protocol: OwnProtocol,
    /// The model as last read or written.
    pub model: Model,
    /// Which way the model is shown.
    pub view: ModelView,
    /// What is selected on the diagram.
    pub focus: ModelFocus,
    runner: Runner,
    layout: ModelLayout,
    /// The Ossie file's text, as it is on disk.
    file_text: String,
    /// The file view's text, as the analyst is editing it.
    file_draft: String,
    /// The text the pause last read, so each text is read once.
    file_read: String,
    /// The draft as it stood when the pause was armed, and when the pause is out.
    file_pause: Option<(String, u64)>,
    /// Why the last text read did not become the model.
    file_error: Option<String>,
    /// What the file holds that Brightfield does not act on, written back with each save.
    kept: Kept,
    /// The selection the file view last scrolled to, while the file view shows.
    file_scrolled_for: Option<Selection>,
    /// Whether the file view's editor held the keyboard when the pane was last drawn.
    file_focused: bool,
    /// Whether the last text read was refused, so it stays in the editor until put right or
    /// discarded.
    draft_refused: bool,
    /// Bumped whenever the model changes, so the diagram is drawn again.
    generation: u64,
    raster: Option<(DiagramKey, egui::TextureId)>,
    drawn: ModelDrawn,
    /// The FineType catalogue the inspector names column types from, when a bundle is staged.
    catalogue: Option<SchemaCatalogue>,
    /// What the inspector shows for the selection, read for the model generation beside it.
    inspected: Option<(Selection, u64, Option<Inspected>)>,
    /// The actions the inspector's buttons chose in the last frame, for the window to dispatch.
    actions: Vec<ModelAction>,
    /// The choice the inspector is asking for, while one is open.
    picker: Option<ModelPicker>,
    /// What came of the last action, for the status line.
    status: Option<String>,
    /// The table a drag on the diagram started on, while the drag lasts.
    drag_from: Option<String>,
    /// Discovery's full pass, while it runs on its worker.
    full_pass: Option<FullPass>,
}

/// What became of a [`ModelAction`] the pane dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatched {
    /// The action was taken; the words say what changed.
    Done(String),
    /// The action opened a picker in the inspector; the words say what it asks.
    Picking(String),
    /// The action was refused or failed; the words say why.
    Failed(String),
    /// The action opens rows in the grid, which is the window's to do.
    OpenInGrid(Selection),
}

impl ModelPane {
    /// Open the model written in `protocol`'s folder. `runner` runs the Protocol when
    /// [`Self::save_model`] changes it.
    ///
    /// # Errors
    ///
    /// When the model file cannot be read or is not an Ossie document Brightfield reads.
    pub fn open(protocol: OwnProtocol, runner: Runner) -> Result<Self, String> {
        let path = protocol.dir.join(MODEL_FILE);
        let file_text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let model =
            brightfield_ossie::read(&file_text).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut pane = Self {
            protocol,
            layout: brightfield_ossie::layout(&model),
            model,
            view: ModelView::default(),
            focus: ModelFocus::default(),
            runner,
            file_draft: file_text.clone(),
            file_read: file_text.clone(),
            file_text,
            file_pause: None,
            file_error: None,
            kept: Kept::default(),
            file_scrolled_for: None,
            file_focused: false,
            draft_refused: false,
            generation: 0,
            raster: None,
            drawn: ModelDrawn::default(),
            catalogue: match brightfield_engine::LoadOptions::packaged().type_source {
                Some(brightfield_engine::semantic::TypeSourceSpec::Bundle(dir)) => {
                    brightfield_engine::semantic::catalogue_in(&dir)
                }
                _ => None,
            },
            inspected: None,
            actions: Vec::new(),
            picker: None,
            status: None,
            drag_from: None,
            full_pass: None,
        };
        // A Protocol already run has silver tables to measure the joins over; one not yet run
        // has none, and nothing is measured.
        if let Err(e) = pane.reverify(false) {
            pane.status = Some(format!("The joins could not be measured again: {e}"));
        }
        Ok(pane)
    }

    /// Step to the next view.
    pub fn cycle_view(&mut self) {
        self.view = self.view.next();
    }

    /// Perform one of the pane's verbs, and say whether `longname` was one.
    pub fn perform(&mut self, longname: &str) -> bool {
        if longname == CYCLE_MODEL_VIEW {
            self.cycle_view();
            return true;
        }
        match SHOW_VIEW.iter().find(|(verb, _)| *verb == longname) {
            Some((_, view)) => {
                self.view = *view;
                true
            }
            None => false,
        }
    }

    /// `7 tables · 6 joins · 1 possible · 1 unclear`.
    #[must_use]
    pub fn summary_line(&self) -> String {
        brightfield_ossie::summary(&self.model)
    }

    /// The files the open passed over, each with the first line of its reason:
    /// `2 files skipped: notes.txt (not a kind of file Brightfield reads), …`. `None` when the
    /// open skipped no file.
    #[must_use]
    pub fn skipped_line(&self) -> Option<String> {
        let skipped = &self.protocol.sources.skipped;
        if skipped.is_empty() {
            return None;
        }
        let named: Vec<String> = skipped
            .iter()
            .map(|(path, reason)| {
                let reason = reason.lines().next().unwrap_or("").trim();
                format!("{} ({reason})", file_name(path))
            })
            .collect();
        Some(format!(
            "{} {} skipped: {}",
            skipped.len(),
            if skipped.len() == 1 { "file" } else { "files" },
            named.join(", ")
        ))
    }

    /// The Ossie file's text as it is on disk.
    #[must_use]
    pub fn file_text(&self) -> &str {
        &self.file_text
    }

    /// The file view's text, as the analyst is editing it.
    #[must_use]
    pub fn file_draft(&self) -> &str {
        &self.file_draft
    }

    /// Put `text` in the file view's editor, as typing there does. It is read once the pause
    /// after it is out ([`Self::tick_file_edit`]).
    pub fn set_file_draft(&mut self, text: String) {
        self.file_draft = text;
    }

    /// Tick the file view's pause at `now_ms`: an edit since the last read arms it, a further
    /// edit arms it again, and once the text has rested [`FILE_PAUSE_MS`] it is read. Answers
    /// what came of a read when one was made: `Err` with why the text was refused or its run
    /// failed, which [`Self::file_error`] then says too.
    ///
    /// An editor that has let go of the keyboard, with no refused edit in it, takes the file's
    /// text here.
    pub fn tick_file_edit(&mut self, now_ms: u64) -> Option<Result<(), String>> {
        if !self.file_focused
            && !self.draft_refused
            && self.file_draft == self.file_read
            && self.file_draft != self.file_text
        {
            self.resync_draft();
        }
        if self.file_draft == self.file_read {
            self.file_pause = None;
            return None;
        }
        match &self.file_pause {
            Some((armed, due)) if *armed == self.file_draft && now_ms >= *due => {
                self.file_pause = None;
                Some(self.read_file_draft())
            }
            Some((armed, _)) if *armed == self.file_draft => None,
            _ => {
                self.file_pause = Some((self.file_draft.clone(), now_ms + FILE_PAUSE_MS));
                None
            }
        }
    }

    /// Whether an edit in the file view is waiting out its pause, so the window draws again to
    /// read it.
    #[must_use]
    pub fn file_edit_armed(&self) -> bool {
        self.file_draft != self.file_read
    }

    /// Why the last text the file view read did not become the model: `Line 3: …`.
    #[must_use]
    pub fn file_error(&self) -> Option<&str> {
        self.file_error.as_deref()
    }

    /// A line for each thing the file holds that Brightfield does not act on.
    #[must_use]
    pub fn file_warnings(&self) -> Vec<String> {
        model_file::warnings(&self.model, &self.kept)
    }

    /// Put the file back in the editor, dropping the edit in it: what Discard edit does.
    pub fn discard_file_edit(&mut self) {
        self.resync_draft();
        self.file_error = None;
    }

    /// The editor takes the file's text, and what an earlier text held but the file does not
    /// is let go.
    fn resync_draft(&mut self) {
        self.file_draft.clone_from(&self.file_text);
        self.file_read.clone_from(&self.file_text);
        self.file_pause = None;
        self.draft_refused = false;
        self.kept.roles.clear();
    }

    /// Read the file view's text and, when Brightfield reads it and it changes anything, save
    /// it as the model. The editor keeps the text as typed.
    fn read_file_draft(&mut self) -> Result<(), String> {
        self.file_read = self.file_draft.clone();
        self.file_pause = None;
        let result = match model_file::read_hand_edit(&self.model, &self.file_draft) {
            // Nothing the file holds changed, only the text's layout or a role it ignores:
            // nothing to write or run.
            Ok(edit)
                if edit.model == self.model
                    && edit.kept.relationships == self.kept.relationships =>
            {
                self.kept = edit.kept;
                self.draft_refused = false;
                Ok(())
            }
            Ok(edit) => {
                let saved = self.save_with(edit.model, edit.kept, true);
                self.let_go_of_what_is_gone();
                saved
            }
            Err(e) => {
                self.draft_refused = true;
                Err(e)
            }
        };
        self.file_error = result.as_ref().err().cloned();
        result
    }

    /// Before a reshape saves: read an edit still waiting out its pause, and refuse when the
    /// editor holds one that does not read.
    fn settle_file_edit(&mut self) -> Result<(), String> {
        if self.file_draft != self.file_read {
            // Its outcome stands in the file view; what matters here is whether it was refused.
            let _ = self.read_file_draft();
        }
        if self.draft_refused {
            Err("The file has an edit that doesn't read — fix or discard it first".to_string())
        } else {
            Ok(())
        }
    }

    /// Let the selection go when the model no longer holds it.
    fn let_go_of_what_is_gone(&mut self) {
        let held = match &self.focus.selected {
            Some(Selection::Dataset(name)) => self.model.datasets.iter().any(|d| d.name == *name),
            Some(Selection::Relationship(name)) => {
                self.model.relationships.iter().any(|r| r.name == *name)
            }
            None => true,
        };
        if !held {
            self.focus.selected = None;
        }
    }

    /// The diagram's layout, as last laid out from the model.
    #[must_use]
    pub fn layout(&self) -> &ModelLayout {
        &self.layout
    }

    /// The choice the inspector is asking for, while one is open.
    #[must_use]
    pub fn picker(&self) -> Option<&ModelPicker> {
        self.picker.as_ref()
    }

    /// The open picker, to change what it has chosen.
    pub fn picker_mut(&mut self) -> Option<&mut ModelPicker> {
        self.picker.as_mut()
    }

    /// What came of the last action: what changed, what a picker asks, or why it was refused.
    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// The action reshape verb `longname` takes on the selection, or why it takes none: a join's
    /// verb with no join selected, say. `None` when `longname` is not a reshape verb.
    #[must_use]
    pub fn reshape_action(&self, longname: &str) -> Option<Result<ModelAction, String>> {
        if !RESHAPE_VERBS.contains(&longname) {
            return None;
        }
        let join = match &self.focus.selected {
            Some(Selection::Relationship(name)) => Ok(name.clone()),
            _ => Err(format!("Select a join first: {longname} acts on a join")),
        };
        let table = match &self.focus.selected {
            Some(Selection::Dataset(name)) => Ok(name.clone()),
            _ => Err(format!("Select a table first: {longname} acts on a table")),
        };
        Some(match longname {
            "keep-join" => join.map(ModelAction::Keep),
            "drop-join" => join.map(ModelAction::Drop),
            "change-join-keys" => join.map(ModelAction::ChangeKeys),
            "add-join" => table.map(ModelAction::AddJoin),
            "set-role" => table.map(ModelAction::PickRole),
            "fold-table" => table.map(ModelAction::FoldInto),
            "unfold-table" => table.and_then(|name| {
                let folded = self
                    .model
                    .datasets
                    .iter()
                    .any(|d| d.name == name && d.meta.folded_into.is_some());
                if folded {
                    Ok(ModelAction::Unfold(name))
                } else {
                    Err(format!("{name} is not folded into another table"))
                }
            }),
            // Each of these two refuses a table already as it would leave it, so a stray key
            // changes nothing and marks nothing the analyst's.
            "leave-out-table" => table.and_then(|name| {
                if self.is_excluded(&name) {
                    Err(format!("{name} is already left out of the model"))
                } else {
                    Ok(ModelAction::LeaveOut(name))
                }
            }),
            "include-table" => table.and_then(|name| {
                if self.is_excluded(&name) {
                    Ok(ModelAction::Include(name))
                } else {
                    Err(format!("{name} is not left out of the model"))
                }
            }),
            _ => table.map(ModelAction::RetypeColumn),
        })
    }

    /// Whether the model leaves the table `name` out.
    fn is_excluded(&self, name: &str) -> bool {
        self.model
            .datasets
            .iter()
            .any(|d| d.name == name && d.meta.excluded)
    }

    /// Perform reshape verb `longname` on the selection, or Rediscover on the whole model, and
    /// answer what came of it; `None` when `longname` is neither.
    pub fn perform_reshape(&mut self, longname: &str) -> Option<Dispatched> {
        if longname == REDISCOVER_MODEL {
            // The stand-in is the discoverer the open uses ([`crate::data_file::open_any`]).
            return Some(self.rediscover(StandIn));
        }
        Some(match self.reshape_action(longname)? {
            Ok(action) => self.dispatch(action),
            Err(e) => {
                self.status = Some(e.clone());
                Dispatched::Failed(e)
            }
        })
    }

    /// What the pane drew in the last frame it was drawn in.
    #[must_use]
    pub fn drawn(&self) -> &ModelDrawn {
        &self.drawn
    }

    /// What the inspector shows for the selection, read from the model and the Protocol's
    /// database once for each selection and model, and kept until either changes. `None` with
    /// no selection, or a selection the model no longer holds.
    pub fn inspected(&mut self) -> Option<&Inspected> {
        let selected = self.focus.selected.clone()?;
        let fresh = self
            .inspected
            .as_ref()
            .is_some_and(|(s, generation, _)| *s == selected && *generation == self.generation);
        if !fresh {
            let found = model_inspector::inspect(
                &self.protocol.dir,
                &self.model,
                &selected,
                self.catalogue.as_ref(),
            );
            self.inspected = Some((selected, self.generation, found));
        }
        self.inspected.as_ref()?.2.as_ref()
    }

    /// The actions the inspector offers for the selection, in the order its buttons stand.
    #[must_use]
    pub fn offered_actions(&self) -> Vec<ModelAction> {
        match &self.focus.selected {
            Some(Selection::Relationship(name)) => model_inspector::join_actions(&self.model, name),
            Some(Selection::Dataset(name)) => model_inspector::table_actions(&self.model, name),
            None => Vec::new(),
        }
    }

    /// Take the actions the inspector's buttons chose since the last call.
    pub fn take_actions(&mut self) -> Vec<ModelAction> {
        std::mem::take(&mut self.actions)
    }

    /// Carry out `action`: an edit goes through [`brightfield_ossie::apply`] and then
    /// [`Self::save_model`], Show in file switches to the file view, and Open in grid is handed
    /// back for the window to open. The actions that need a choice first open a picker in the
    /// inspector, whose answer comes back here as [`ModelAction::Edit`]. What came of it stands in
    /// the status line.
    pub fn dispatch(&mut self, action: ModelAction) -> Dispatched {
        let dispatched = self.carry_out(action);
        self.say(&dispatched);
        dispatched
    }

    fn carry_out(&mut self, action: ModelAction) -> Dispatched {
        let edit = match action {
            ModelAction::Keep(relationship) => Edit::KeepJoin { relationship },
            ModelAction::Drop(relationship) => Edit::DropJoin { relationship },
            ModelAction::ChangeRole { dataset, role } => Edit::SetRole { dataset, role },
            ModelAction::Unfold(dataset) => Edit::Unfold { dataset },
            ModelAction::LeaveOut(dataset) => Edit::Exclude { dataset },
            ModelAction::Include(dataset) => Edit::Include { dataset },
            ModelAction::Edit(edit) => edit,
            ModelAction::ShowInFile => {
                self.view = ModelView::File;
                return Dispatched::Done("Showing the Ossie file".to_string());
            }
            ModelAction::OpenInGrid(selection) => return Dispatched::OpenInGrid(selection),
            ModelAction::ChangeKeys(relationship) => {
                let picker = model_inspector::keys_picker(&self.model, &relationship);
                return self.open_picker(picker);
            }
            ModelAction::FoldInto(dataset) => {
                let picker = model_inspector::fold_picker(&self.model, &dataset);
                return self.open_picker(picker);
            }
            ModelAction::RetypeColumn(dataset) => {
                let picker = model_inspector::retype_picker(&self.model, &dataset);
                return self.open_picker(picker);
            }
            ModelAction::AddJoin(dataset) => {
                let picker = model_inspector::add_join_picker(&self.model, &dataset, None);
                return self.open_picker(picker);
            }
            ModelAction::PickRole(dataset) => {
                let picker = model_inspector::role_picker_for(&self.model, &dataset);
                return self.open_picker(picker);
            }
        };
        if let Err(e) = self.settle_file_edit() {
            return Dispatched::Failed(e);
        }
        let said = edit_said(&edit);
        let model = match brightfield_ossie::apply(&self.model, &edit) {
            Ok(model) => model,
            Err(e) => return Dispatched::Failed(e.0),
        };
        self.picker = None;
        let saved = self.save_model(model);
        // A dropped join is gone from the model, and so is anything selected on it.
        if let Some(Selection::Relationship(name)) = &self.focus.selected {
            if !self.model.relationships.iter().any(|r| r.name == *name) {
                self.focus.selected = None;
            }
        }
        match saved {
            Ok(()) => Dispatched::Done(said),
            Err(e) => Dispatched::Failed(e),
        }
    }

    /// Run `discovery` over the Protocol again and fold what it proposes into the model: what the
    /// analyst decided stays as it is, and what Dovetail decided takes the fresh pass. The sample
    /// pass is folded in and saved, as an action's model is, before this answers; the full pass
    /// then runs on a worker, in place of any still running, which is told to stop, and lands
    /// through [`Self::poll_discovery`]. The full pass starts only once the sample's save is
    /// written, so it reads the folder as the save left it. What came of it stands in the status
    /// line.
    pub fn rediscover(&mut self, discovery: impl Discovery + Send + 'static) -> Dispatched {
        let dispatched = self.rediscovered(Box::new(discovery));
        self.say(&dispatched);
        dispatched
    }

    fn rediscovered(&mut self, discovery: Box<dyn Discovery + Send>) -> Dispatched {
        if let Err(e) = self.settle_file_edit() {
            return Dispatched::Failed(e);
        }
        // A pass still running answers for a model this one replaces.
        self.full_pass = None;
        let model = match sample_pass(&*discovery, &self.protocol, Some(&self.model)) {
            Ok(model) => model,
            Err(e) => return Dispatched::Failed(format!("Discovery failed: {e}")),
        };
        let dispatched = if model == self.model {
            Dispatched::Done("The sample pass found nothing to change".to_string())
        } else {
            self.picker = None;
            let saved = self.save_model(model);
            self.let_go_of_what_is_gone();
            match saved {
                Ok(()) => Dispatched::Done(
                    "Discovered the model again from a sample, around what you decided".to_string(),
                ),
                Err(e) => return Dispatched::Failed(e),
            }
        };
        self.full_pass = Some(FullPass::start(discovery, &self.protocol));
        dispatched
    }

    /// Run `discovery`'s full pass over the Protocol on a worker, in place of any still running,
    /// which is told to stop, to land through [`Self::poll_discovery`]. The window starts one when the pane opens, whose
    /// model is the sample pass's.
    pub fn start_full_pass(&mut self, discovery: impl Discovery + Send + 'static) {
        self.full_pass = Some(FullPass::start(Box::new(discovery), &self.protocol));
    }

    /// Whether discovery's full pass is still to land, so the window keeps polling.
    #[must_use]
    pub fn discovering(&self) -> bool {
        self.full_pass.is_some()
    }

    /// Fold discovery's full pass into the model once it has landed, and answer what came of it;
    /// `None` while it runs, or while it waits for a hand edit in the file view to be read or
    /// discarded, so the fold sees the model the edit made. A fold that changes the model saves
    /// it, as an action's model is, and says so in the status line, as a failed pass does; what
    /// the analyst decided stays as it is.
    pub fn poll_discovery(&mut self) -> Option<Dispatched> {
        if self.file_edit_armed() || self.draft_refused {
            return None;
        }
        let answer = self.full_pass.as_ref()?.try_take()?;
        self.full_pass = None;
        Some(self.fold_full_pass(answer))
    }

    /// [`Self::poll_discovery`], waiting for the full pass to land: `None` when no pass is out,
    /// or a hand edit in the file view waits to be read.
    pub fn finish_discovery(&mut self) -> Option<Dispatched> {
        if self.file_edit_armed() || self.draft_refused {
            return None;
        }
        let answer = self.full_pass.take()?.wait();
        Some(self.fold_full_pass(answer))
    }

    /// Fold the full pass's `answer` into the model as it stands.
    fn fold_full_pass(&mut self, answer: Result<Model, String>) -> Dispatched {
        let full = match answer {
            Ok(full) => full,
            Err(e) => {
                let failed = Dispatched::Failed(format!("The full discovery pass failed: {e}"));
                self.say(&failed);
                return failed;
            }
        };
        let model = merge_discovery(&self.model, &full);
        if model == self.model {
            // The status line keeps what the analyst last did.
            return Dispatched::Done("The full pass found nothing to change".to_string());
        }
        let saved = self.save_model(model);
        self.let_go_of_what_is_gone();
        let dispatched = match saved {
            Ok(()) => Dispatched::Done("Folded in the full pass, around what you decided".into()),
            Err(e) => Dispatched::Failed(e),
        };
        self.say(&dispatched);
        dispatched
    }

    /// Put what came of `dispatched` in the status line.
    fn say(&mut self, dispatched: &Dispatched) {
        match dispatched {
            Dispatched::Done(said) | Dispatched::Picking(said) | Dispatched::Failed(said) => {
                self.status = Some(said.clone());
            }
            Dispatched::OpenInGrid(_) => {}
        }
    }

    /// Open `picker` over its subject, selecting it so the inspector shows it, or say why it
    /// cannot open.
    fn open_picker(&mut self, picker: Result<ModelPicker, String>) -> Dispatched {
        match picker {
            Ok(picker) => {
                let said = picker.prompt();
                self.focus.selected = Some(picker.subject());
                self.picker = Some(picker);
                Dispatched::Picking(said)
            }
            Err(e) => Dispatched::Failed(e),
        }
    }

    /// Write the rows `target` names to a Parquet file in the Protocol folder for the grid to
    /// open, and answer its path: a table's silver rows, or every joined row of a join, each
    /// without their lineage columns.
    ///
    /// # Errors
    ///
    /// The reason when the target has no rows of its own in the database, or the copy fails.
    pub fn grid_snapshot(&self, target: &Selection) -> Result<PathBuf, String> {
        let (name, sql) = match target {
            Selection::Dataset(name) => {
                if !model_inspector::materialised(&self.model, name) {
                    return Err(format!(
                        "{name} has no table of its own: it is left out or folded into another"
                    ));
                }
                (name.clone(), model_inspector::silver_rows_sql(name))
            }
            Selection::Relationship(name) => {
                let sql = brightfield_protocol::preview::join_rows_sql(&self.model, name);
                if sql.is_empty() {
                    return Err(format!("the model has no join named {name}"));
                }
                (name.clone(), sql)
            }
        };
        model_inspector::snapshot(&self.protocol.dir, &name, &sql)
    }

    /// Forget the diagram's raster, so the next frame that shows the diagram draws it again: the
    /// window calls this when the pane comes back from the grid, whose document replaced the
    /// canvas host's textures.
    pub fn forget_raster(&mut self) {
        self.raster = None;
    }

    /// The canvas-host slot the diagram rasters into, for the window's sweep.
    #[must_use]
    pub fn raster_key(&self) -> PaneKey {
        DIAGRAM_SLOT
    }

    /// Make `model` the Protocol's model: write it as the Ossie file, rewrite the silver steps
    /// for it, and run the Protocol. What the file holds that Brightfield does not act on is
    /// written back with it, while it still stands beside `model` ([`model_file::still_kept`]),
    /// and the file view's text becomes the file's.
    ///
    /// The run executes only the steps whose SQL changed, which are the silver and failed-value
    /// steps: arc skips a step whose SQL file and inputs hash as they did after its last success,
    /// and the load steps' are untouched. A table left out or folded into another loses its
    /// silver tables from the database before the run ([`own_files::drop_unmade_silver`]). The
    /// pane takes `model` once the file is written, so a failed run leaves it showing what the
    /// folder holds.
    ///
    /// After a run that succeeds, each join's evidence is measured again over the silver tables
    /// ([`measure_joins`]), and when that changes the model it is written again, without a run:
    /// evidence is what the file says about the data, not a step of the Protocol.
    ///
    /// # Errors
    ///
    /// The reason the file or the silver steps could not be written, or what arc printed when
    /// the run failed.
    pub fn save_model(&mut self, model: Model) -> Result<(), String> {
        let kept = model_file::still_kept(&model, &self.kept);
        self.save_with(model, kept, false)
    }

    /// Write `model` with `kept`, as [`Self::save_model`] says. A save from the editor's own
    /// text (`from_editor`), or one while the editor holds the keyboard, leaves the editor's text
    /// as it stands, to take the file's once the editor lets go of the keyboard.
    fn save_with(&mut self, model: Model, kept: Kept, from_editor: bool) -> Result<(), String> {
        self.write_model(model, kept, from_editor)?;
        own_files::write_silver(&self.protocol, &self.model)?;
        own_files::drop_unmade_silver(&self.protocol, &self.model)?;
        own_files::run_loads(&self.protocol, &self.runner)?;
        self.reverify(from_editor)
    }

    /// Write `model` with `kept` as the Ossie file and make it the pane's model, without touching
    /// the Protocol's steps or running it.
    fn write_model(&mut self, model: Model, kept: Kept, from_editor: bool) -> Result<(), String> {
        let text = model_file::written(&model, &kept);
        write_model_file(&self.protocol.dir, &text)?;
        self.layout = brightfield_ossie::layout(&model);
        self.model = model;
        self.kept = kept;
        self.file_text = text;
        if !(from_editor || self.file_focused) {
            self.resync_draft();
        }
        self.draft_refused = false;
        self.file_pause = None;
        self.file_error = None;
        self.generation += 1;
        Ok(())
    }

    /// Measure each join's evidence again over the Protocol's silver tables, and write the model
    /// when that changed it. `from_editor` is as [`Self::save_with`] takes it.
    fn reverify(&mut self, from_editor: bool) -> Result<(), String> {
        let measured = measure_joins(&self.protocol.dir, &self.model)?;
        if measured == self.model {
            return Ok(());
        }
        // Measuring renames and removes nothing, so what the file kept beside the model stands.
        let kept = self.kept.clone();
        self.write_model(measured, kept, from_editor)
    }

    /// Raster the diagram through `host` when it is showing and what it depends on changed.
    /// A window with no device passes `None`, and the pane lays out the same without a raster.
    pub fn present(&mut self, host: Option<&mut EguiCanvasHost>, ppp: f32, mode: Mode) {
        if self.view != ModelView::Diagram {
            return;
        }
        let Some(host) = host else {
            return;
        };
        let key = DiagramKey {
            generation: self.generation,
            dev: PixelSize {
                width: ((self.layout.width as f32) * ppp).round().max(1.0) as u32,
                height: ((self.layout.height as f32) * ppp).round().max(1.0) as u32,
            },
            dark: mode.is_dark(),
            selected: self.focus.selected.clone(),
        };
        if self.raster.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let mut scene = vello::Scene::new();
        render_model(
            &mut scene,
            &self.model,
            &self.layout,
            &self.focus,
            mode.is_dark(),
        );
        let mut scaled = vello::Scene::new();
        scaled.append(&scene, Some(kurbo::Affine::scale(f64::from(ppp))));
        let base = Color::from_token(semantic(mode.is_dark()).surfaces.app);
        let id = host.present_keyed(DIAGRAM_SLOT, &scaled, key.dev, base);
        self.raster = Some((key, id));
    }

    /// Draw the pane's body: the summary, the skipped files, the view switch and the view.
    pub fn ui(&mut self, ui: &mut egui::Ui, mode: Mode) {
        let sem = semantic(mode.is_dark());
        let primary = chrome::colour(sem.text.primary);
        let secondary = chrome::colour(sem.text.secondary);
        let mut drawn = ModelDrawn {
            view: Some(self.view),
            ..ModelDrawn::default()
        };

        let summary = self.summary_line();
        ui.label(egui::RichText::new(&summary).color(primary));
        drawn.header.push(summary);
        if let Some(skipped) = self.skipped_line() {
            ui.label(egui::RichText::new(&skipped).color(secondary));
            drawn.header.push(skipped);
        }
        if let Some(status) = &self.status {
            ui.label(egui::RichText::new(status).color(secondary));
            drawn.header.push(status.clone());
        }
        ui.horizontal(|ui| {
            for (_, view) in SHOW_VIEW {
                if ui
                    .selectable_label(self.view == view, view.label())
                    .clicked()
                {
                    self.view = view;
                }
            }
        });
        ui.add_space(spacing::SPACE_2);

        // With a selection the inspector stands to the right of the view, and the view has what
        // is left.
        let area = ui.available_rect_before_wrap();
        // A picker asks about what was selected when it opened, and closes once that is let go.
        if self
            .picker
            .as_ref()
            .is_some_and(|p| self.focus.selected.as_ref() != Some(&p.subject()))
        {
            self.picker = None;
        }
        let inspector = self.inspected().cloned().map(|inspected| {
            let rect = egui::Rect::from_min_max(
                egui::pos2(
                    (area.right() - model_inspector::INSPECTOR_WIDTH).max(area.left()),
                    area.top(),
                ),
                area.max,
            );
            (rect, inspected)
        });
        let view_rect = match &inspector {
            Some((rect, _)) => egui::Rect::from_min_max(
                area.min,
                egui::pos2(rect.left() - spacing::SPACE_2, area.bottom()),
            ),
            None => area,
        };
        let mut view_ui = ui.new_child(egui::UiBuilder::new().max_rect(view_rect));
        if self.view != ModelView::File {
            self.file_scrolled_for = None;
            self.file_focused = false;
        }
        match self.view {
            ModelView::Diagram => self.diagram_ui(&mut view_ui, &mut drawn),
            ModelView::List => {
                if let Some(selected) = list_ui(
                    &mut view_ui,
                    &self.model,
                    self.focus.selected.as_ref(),
                    mode,
                    &mut drawn,
                ) {
                    self.focus.selected = selected;
                }
            }
            ModelView::File => self.file_ui(&mut view_ui, mode, &mut drawn),
        }
        if let Some((rect, inspected)) = inspector {
            let sem = semantic(mode.is_dark());
            ui.painter().line_segment(
                [rect.left_top(), rect.left_bottom()],
                egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
            );
            let mut inspector_ui = ui.new_child(
                egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(spacing::SPACE_2, 0.0))),
            );
            let actions = self.offered_actions();
            let mut said = InspectorDrawn::default();
            model_inspector::ui(
                &mut inspector_ui,
                &inspected,
                &actions,
                &mut self.picker,
                mode,
                &mut self.actions,
                &mut said,
            );
            drawn.inspector = Some(said);
        }
        ui.allocate_rect(area, egui::Sense::hover());
        self.drawn = drawn;
    }

    /// The file view: the Ossie file in the code editor, and under it why the last text could not
    /// be read and what the file holds that Brightfield does not act on, a line each. The first
    /// frame it shows a selection, it scrolls to the selection's `name:` line.
    fn file_ui(&mut self, ui: &mut egui::Ui, mode: Mode, drawn: &mut ModelDrawn) {
        let sem = semantic(mode.is_dark());
        let danger = chrome::colour(sem.role(meridian_design::Role::Danger).foreground.base);
        let warning = chrome::colour(sem.role(meridian_design::Role::Warning).foreground.base);
        let notes: Vec<(String, egui::Color32)> = self
            .file_error
            .iter()
            .map(|e| (e.clone(), danger))
            .chain(self.file_warnings().into_iter().map(|w| (w, warning)))
            .collect();
        let area = ui.available_rect_before_wrap();
        let note_h = ui.text_style_height(&egui::TextStyle::Body) + spacing::SPACE_1;
        #[allow(clippy::cast_precision_loss)] // a handful of notes
        let notes_h = note_h * notes.len() as f32;
        let editor_rect = egui::Rect::from_min_max(
            area.min,
            egui::pos2(area.max.x, (area.max.y - notes_h).max(area.min.y)),
        );
        let scroll_to = match &self.focus.selected {
            Some(selected) if self.file_scrolled_for.as_ref() != Some(selected) => {
                model_file::selection_line(&self.file_draft, selected)
            }
            _ => None,
        };
        self.file_scrolled_for.clone_from(&self.focus.selected);
        let mut editor_ui = ui.new_child(egui::UiBuilder::new().max_rect(editor_rect));
        let surface = crate::editor::code_surface_at(
            &mut editor_ui,
            "model-file",
            &mut self.file_draft,
            Access::Editable,
            mode,
            scroll_to,
        );
        self.file_focused = surface.focused;
        for (i, (note, ink)) in notes.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)] // a handful of notes
            let y = editor_rect.bottom() + note_h * (i as f32 + 0.5);
            ui.painter().text(
                egui::pos2(area.left(), y),
                egui::Align2::LEFT_CENTER,
                note,
                crate::protocol::ui_font(),
                *ink,
            );
        }
        // A refused edit's error line ends in the way back to the file.
        if self.draft_refused && self.file_error.is_some() {
            let row = egui::Rect::from_min_max(
                egui::pos2(area.left(), editor_rect.bottom()),
                egui::pos2(area.right(), editor_rect.bottom() + note_h),
            );
            let rect = egui::Rect::from_min_max(
                egui::pos2((row.right() - DISCARD_WIDTH).max(row.left()), row.top()),
                row.max,
            );
            if ui
                .put(rect, egui::Button::new("Discard edit").small())
                .clicked()
            {
                self.discard_file_edit();
            }
            drawn.file_discard = Some(rect);
        }
        drawn.file_editor = Some(editor_rect);
        drawn.file_scrolled_to = scroll_to;
        drawn.file_notes = notes.into_iter().map(|(note, _)| note).collect();
        drawn.body = self.file_draft.lines().map(str::to_string).collect();
    }

    fn diagram_ui(&mut self, ui: &mut egui::Ui, drawn: &mut ModelDrawn) {
        let size = egui::vec2(self.layout.width as f32, self.layout.height as f32);
        let mut clicked = None;
        let mut drag_from = self.drag_from.clone();
        let mut dragged = None;
        egui::ScrollArea::both()
            .id_salt("model-diagram")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                drawn.diagram = Some(rect);
                if let Some((_, texture)) = &self.raster {
                    ui.painter().image(
                        *texture,
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                let under = |at: egui::Pos2| {
                    let local = at - rect.min;
                    hit_test(&self.layout, f64::from(local.x), f64::from(local.y))
                };
                let table_under = |at: Option<egui::Pos2>| match at.and_then(under) {
                    Some(Selection::Dataset(name)) => Some(name),
                    _ => None,
                };
                if response.clicked() {
                    if let Some(at) = response.interact_pointer_pos() {
                        clicked = Some(under(at));
                    }
                }
                // A drag from one card to another asks for the join between them.
                if response.drag_started() {
                    drag_from = table_under(ui.input(|i| i.pointer.press_origin()));
                }
                if response.drag_stopped() {
                    let to = table_under(ui.input(|i| i.pointer.latest_pos()));
                    if let (Some(from), Some(to)) = (drag_from.take(), to) {
                        if from != to {
                            dragged = Some((from, to));
                        }
                    }
                }
            });
        self.drag_from = drag_from;
        if let Some(selected) = clicked {
            self.focus.selected = selected;
        }
        if let Some((from, to)) = dragged {
            let picker = model_inspector::add_join_picker(&self.model, &from, Some(&to));
            let dispatched = self.open_picker(picker);
            if let Dispatched::Picking(said) | Dispatched::Failed(said) = dispatched {
                self.status = Some(said);
            }
        }
    }
}

/// One join in the list's gutter: a run down its own lane from one table's row to the other's,
/// with a tick into each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GutterRoute {
    /// The relationship's name.
    pub relationship: String,
    /// The list row of the relationship's `from` table.
    pub from_row: usize,
    /// The list row of its `to` table.
    pub to_row: usize,
    /// Its lane, counted from the gutter's left edge.
    pub lane: usize,
    /// Whether the join is only possible, drawn dashed in the proposal ink.
    pub possible: bool,
}

/// The gutter's routes for `rows` (the model's [`brightfield_ossie::list_rows`]): one per join
/// between two placed tables, each on the leftmost lane no other route overlapping it holds. Two
/// routes share a lane only when the rows they span do not meet, so no tick lands on another
/// route's run.
#[must_use]
pub fn gutter_routes(model: &Model, rows: &[ListRow]) -> Vec<GutterRoute> {
    let row_of = |table: &str| {
        rows.iter()
            .position(|r| matches!(r, ListRow::Table { name, .. } if name == table))
    };
    let mut joins: Vec<&str> = rows
        .iter()
        .filter_map(|r| match r {
            ListRow::Table { joins, .. } => Some(joins.iter().map(String::as_str)),
            ListRow::Review { .. } | ListRow::Folded { .. } | ListRow::LeftOut { .. } => None,
        })
        .flatten()
        .collect();
    joins.sort_unstable();
    joins.dedup();
    let mut routes: Vec<GutterRoute> = joins
        .into_iter()
        .filter_map(|name| {
            let r = model.relationships.iter().find(|r| r.name == name)?;
            Some(GutterRoute {
                relationship: r.name.clone(),
                from_row: row_of(&r.from)?,
                to_row: row_of(&r.to)?,
                lane: 0,
                possible: brightfield_ossie::is_proposal(r),
            })
        })
        .collect();
    let span = |r: &GutterRoute| (r.from_row.min(r.to_row), r.from_row.max(r.to_row));
    routes.sort_by(|a, b| {
        span(a)
            .cmp(&span(b))
            .then(a.relationship.cmp(&b.relationship))
    });
    // The last row each lane's routes reach.
    let mut lane_ends: Vec<usize> = Vec::new();
    for route in &mut routes {
        let (top, bottom) = span(route);
        route.lane = match lane_ends.iter().position(|&end| end < top) {
            Some(lane) => lane,
            None => {
                lane_ends.push(0);
                lane_ends.len() - 1
            }
        };
        lane_ends[route.lane] = bottom;
    }
    routes
}

/// A route as the list drew it, in window-space logical points.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteDrawn {
    /// The route it draws.
    pub route: GutterRoute,
    /// Where its run stands.
    pub x: f32,
    /// The heights of its ticks: into the `from` row, then into the `to` row.
    pub ticks: (f32, f32),
    /// Where its ticks end, at the gutter's right edge.
    pub tick_end: f32,
}

/// The height of one list row, in logical points.
const LIST_ROW: f32 = 24.0;
/// The width of one gutter lane.
const LANE: f32 = 8.0;
/// Space either side of the gutter's lanes.
const GUTTER_PAD: f32 = 6.0;
/// The gutter's dashes, for a possible join: the dash and the gap.
const GUTTER_DASH: (f32, f32) = (4.0, 3.0);

/// The selection a list row stands for: its table, or the possible join it asks about.
fn row_selection(row: &ListRow) -> Selection {
    match row {
        ListRow::Review { relationship, .. } => Selection::Relationship(relationship.clone()),
        ListRow::Table { name, .. }
        | ListRow::Folded { name, .. }
        | ListRow::LeftOut { name, .. } => Selection::Dataset(name.clone()),
    }
}

/// Whether `selected` leaves a list row lit, by the rule the diagram dims by.
fn row_lit(model: &Model, selected: Option<&Selection>, row: &ListRow) -> bool {
    match row {
        ListRow::Review { relationship, .. } => relationship_lit(model, selected, relationship),
        ListRow::Table { name, .. }
        | ListRow::Folded { name, .. }
        | ListRow::LeftOut { name, .. } => dataset_lit(model, selected, name),
    }
}

/// The list view: possible joins waiting on the analyst first, then a row per table with its
/// metadata, and the joins routed in a narrow gutter on the left linking the rows they join.
///
/// A click on a row selects what it stands for, and a click on the selected row lets it go: the
/// answer is the new selection when a row was clicked. With a selection, the rows and routes not
/// connected to it are drawn in the dimmed ink.
fn list_ui(
    ui: &mut egui::Ui,
    model: &Model,
    selected: Option<&Selection>,
    mode: Mode,
    drawn: &mut ModelDrawn,
) -> Option<Option<Selection>> {
    let sem = semantic(mode.is_dark());
    let primary = chrome::colour(sem.text.primary);
    let secondary = chrome::colour(sem.text.secondary);
    let likely = chrome::colour(sem.borders.default_);
    let proposal = chrome::colour(sem.role(meridian_design::Role::Neutral).foreground.base);
    let dimmed = chrome::colour(sem.text.disabled);
    let chosen_fill = chrome::colour(sem.rows.selected_background);
    let rows = brightfield_ossie::list_rows(model);
    let routes = gutter_routes(model, &rows);
    let lanes = routes.iter().map(|r| r.lane + 1).max().unwrap_or(0);
    let gutter = GUTTER_PAD * 2.0 + LANE * lanes as f32;
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("model-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width();
            let (area, _) = ui.allocate_exact_size(
                egui::vec2(width, LIST_ROW * rows.len() as f32),
                egui::Sense::hover(),
            );
            let row_rect = |i: usize| {
                egui::Rect::from_min_size(
                    area.min + egui::vec2(0.0, LIST_ROW * i as f32),
                    egui::vec2(width, LIST_ROW),
                )
            };
            for (i, row) in rows.iter().enumerate() {
                let rect = row_rect(i);
                let this = row_selection(row);
                if ui
                    .interact(
                        rect,
                        ui.id().with(("model-list-row", i)),
                        egui::Sense::click(),
                    )
                    .clicked()
                {
                    clicked = Some((selected != Some(&this)).then(|| this.clone()));
                }
            }
            let painter = ui.painter();
            for (i, row) in rows.iter().enumerate() {
                let rect = row_rect(i);
                let lit = row_lit(model, selected, row);
                if selected == Some(&row_selection(row)) {
                    painter.rect_filled(rect, 0.0, chosen_fill);
                }
                let (text, ink) = match row {
                    ListRow::Review { .. } => (list_line(row), proposal),
                    ListRow::Table { .. } => (list_line(row), primary),
                    ListRow::Folded { .. } | ListRow::LeftOut { .. } => (list_line(row), secondary),
                };
                let ink = if lit { ink } else { dimmed };
                // A folded table stands indented under its host's row.
                let indent = if matches!(row, ListRow::Folded { .. }) {
                    spacing::SPACE_6
                } else {
                    0.0
                };
                painter.text(
                    egui::pos2(
                        rect.left() + gutter + spacing::SPACE_2 + indent,
                        rect.center().y,
                    ),
                    egui::Align2::LEFT_CENTER,
                    &text,
                    crate::protocol::ui_font(),
                    ink,
                );
                drawn.list_rows.push(rect);
                drawn.list_lit.push(lit);
                drawn.body.push(text);
            }
            let tick_end = area.left() + gutter - GUTTER_PAD / 2.0;
            for route in routes {
                let x = area.left() + GUTTER_PAD + LANE * (route.lane as f32 + 0.5);
                let ticks = (
                    row_rect(route.from_row).center().y,
                    row_rect(route.to_row).center().y,
                );
                let ink = if !relationship_lit(model, selected, &route.relationship) {
                    dimmed
                } else if route.possible {
                    proposal
                } else {
                    likely
                };
                let stroke = egui::Stroke::new(1.5, ink);
                let path = [
                    egui::pos2(tick_end, ticks.0),
                    egui::pos2(x, ticks.0),
                    egui::pos2(x, ticks.1),
                    egui::pos2(tick_end, ticks.1),
                ];
                if route.possible {
                    painter.extend(egui::Shape::dashed_line(
                        &path,
                        stroke,
                        GUTTER_DASH.0,
                        GUTTER_DASH.1,
                    ));
                } else {
                    painter.add(egui::Shape::line(path.to_vec(), stroke));
                }
                drawn.routes.push(RouteDrawn {
                    route,
                    x,
                    ticks,
                    tick_end,
                });
            }
        });
    clicked
}

/// One list row's text: a possible join's review line, a table's metadata, a folded table under
/// its host, or a table left out.
fn list_line(row: &ListRow) -> String {
    match row {
        ListRow::Folded { name, into } => format!("\u{21b3} {name} · folded into {into}"),
        ListRow::LeftOut { name, role } => {
            format!("left out · {name} · {}", model_inspector::role_label(*role))
        }
        ListRow::Review {
            relationship,
            edge_text,
        } => format!("Possible join {relationship}: {edge_text}"),
        ListRow::Table {
            role,
            name,
            rows,
            key,
            fields,
            folded,
            joins,
        } => {
            let mut text = format!("{} · {name}", model_inspector::role_label(*role));
            if let Some(rows) = rows {
                text.push_str(&format!(" · {rows} rows"));
            }
            if !key.is_empty() {
                text.push_str(&format!(" · key {}", key.join(", ")));
            }
            text.push_str(&format!(
                " · {fields} {}",
                if *fields == 1 { "field" } else { "fields" }
            ));
            if !folded.is_empty() {
                text.push_str(&format!(" · + {}", folded.join(", ")));
            }
            if !joins.is_empty() {
                text.push_str(&format!(" · joins {}", joins.join(", ")));
            }
            text
        }
    }
}

/// `model` with each join's evidence measured again over the silver tables of the Protocol in
/// `dir` ([`brightfield_ossie::evidence_sql`]) and written onto it with whatever warning that
/// calls for ([`brightfield_ossie::apply_evidence`]).
///
/// A join is measured when both its tables are in the model and each has a silver table in the
/// database; a folded table has none, and its join is left as it is. A join whose statement
/// fails, keys whose types do not compare, say, is left as it is too. The evidence a statement
/// does not measure (the key types and how alike the names are) stays as it was. A Protocol that
/// has not been run has no database, and `model` comes back as it is.
///
/// # Errors
///
/// The reason when the database cannot be opened.
pub fn measure_joins(dir: &Path, model: &Model) -> Result<Model, String> {
    let db = arc::spec::protocol_db_path(dir).map_err(|e| e.to_string())?;
    if !db.is_file() || model.relationships.is_empty() {
        return Ok(model.clone());
    }
    let tables = model_inspector::query(dir, "SELECT table_name FROM duckdb_tables()")?;
    let has_table = |name: &str| {
        tables
            .rows
            .iter()
            .any(|row| row.first().and_then(Option::as_deref) == Some(name))
    };
    let included = |name: &str| {
        model
            .datasets
            .iter()
            .any(|d| d.name == name && !d.meta.excluded)
    };
    let mut out = model.clone();
    for r in &model.relationships {
        if !(included(&r.from) && included(&r.to) && has_table(&r.from) && has_table(&r.to)) {
            continue;
        }
        let sql = brightfield_ossie::evidence_sql(model, &r.name);
        if sql.is_empty() {
            continue;
        }
        let Ok(rows) = model_inspector::query(dir, &sql) else {
            continue;
        };
        let count = |column: &str| rows.value(0, column).and_then(|v| v.parse::<u64>().ok());
        let overlap = rows
            .value(0, "value_overlap")
            .and_then(|v| v.parse::<f64>().ok());
        let (Some(value_overlap), Some(orphans), Some(child_rows), Some(parent_duplicates)) = (
            overlap,
            count("orphans"),
            count("child_rows"),
            count("parent_duplicates"),
        ) else {
            continue;
        };
        let measured = Evidence {
            value_overlap,
            orphans,
            child_rows,
            parent_duplicates,
            ..r.meta.evidence.clone()
        };
        out = brightfield_ossie::apply_evidence(&out, &r.name, measured);
    }
    Ok(out)
}

/// What a toast says once `edit` is saved.
fn edit_said(edit: &Edit) -> String {
    match edit {
        Edit::KeepJoin { relationship } => format!("Kept {relationship}"),
        Edit::DropJoin { relationship } => format!("Dropped {relationship}"),
        Edit::SetRole { dataset, role } => {
            format!("{dataset} is now {}", model_inspector::role_label(*role))
        }
        Edit::Unfold { dataset } => format!("Unfolded {dataset}"),
        Edit::Exclude { dataset } => format!("Left {dataset} out of the model"),
        Edit::Include { dataset } => format!("Brought {dataset} back into the model"),
        Edit::AddJoin {
            from,
            from_columns,
            to,
            to_columns,
        } => format!(
            "Joined {from} ({}) to {to} ({})",
            from_columns.join(", "),
            to_columns.join(", ")
        ),
        Edit::ChangeKeys {
            relationship,
            from_columns,
            to_columns,
        } => format!(
            "{relationship} now matches {} to {}",
            from_columns.join(", "),
            to_columns.join(", ")
        ),
        Edit::Fold { dataset, into } => format!("Folded {dataset} into {into}"),
        Edit::Retype {
            dataset,
            field,
            finetype,
        } => format!("{dataset}.{field} is now {finetype}"),
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The one pane of the model document.
struct ModelItem;

impl Item<ModelPane> for ModelItem {
    fn item_id(&self) -> ItemId {
        MODEL
    }

    fn empty_state(&self, doc: &ModelPane) -> Option<EmptyState> {
        doc.model.datasets.is_empty().then(|| {
            EmptyState::new(
                ICON_MODEL,
                "No tables in this model",
                "The model names no tables, so there is nothing to draw. \
                 Open a folder, workbook or database that holds at least one.",
            )
        })
    }

    fn describe(&self, _doc: &ModelPane) -> Subject {
        Subject::new("Model", ICON_MODEL, BindingContext::Workspace)
    }

    fn ui(&mut self, doc: &mut ModelPane, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        doc.ui(ui, cx.mode);
    }

    fn perform(&mut self, doc: &mut ModelPane, verb: Verb, _cx: &mut ItemCtx<'_>) -> Handled {
        if doc.perform(verb.as_str()) || doc.perform_reshape(verb.as_str()).is_some() {
            Handled::Yes
        } else {
            Handled::No
        }
    }
}
