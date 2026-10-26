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
//! Ossie file, rewrites the silver steps for it and runs the Protocol again.
//!
//! What is selected belongs to the pane, not to a view: a click on a card or a join on the
//! diagram, or on a row of the list, selects it, and in either view what is not connected to the
//! selection dims. While something is selected the inspector ([`crate::model_inspector`]) stands
//! beside the view, and the actions it offers come back to [`ModelPane::dispatch`].

use std::path::{Path, PathBuf};

use brightfield_engine::semantic::SchemaCatalogue;
use brightfield_keys::{Altitude, BindingContext};
use brightfield_ossie::{Edit, ListRow, Model, ModelLayout};
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
use crate::discovery::MODEL_FILE;
use crate::model_inspector::{self, Inspected, InspectorDrawn, ModelAction};
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

/// Which way the model is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelView {
    /// The tables as cards in role bands, with the joins drawn between them.
    #[default]
    Diagram,
    /// One row per table, grouped by role, with a gutter naming its joins.
    List,
    /// The Ossie file as written.
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

/// Whether `longname` is one of the model pane's verbs.
#[must_use]
pub fn is_model_verb(longname: &str) -> bool {
    longname == CYCLE_MODEL_VIEW || SHOW_VIEW.iter().any(|(verb, _)| *verb == longname)
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
}

/// What became of a [`ModelAction`] the pane dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatched {
    /// The action was taken; the words say what changed.
    Done(String),
    /// The action needs a picker this build does not have yet; the words say so.
    NotYet(String),
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
        Ok(Self {
            protocol,
            layout: brightfield_ossie::layout(&model),
            model,
            view: ModelView::default(),
            focus: ModelFocus::default(),
            runner,
            file_text,
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
        })
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
    /// back for the window to open. The actions that need a picker say they are not available
    /// yet.
    pub fn dispatch(&mut self, action: ModelAction) -> Dispatched {
        let edit = match action {
            ModelAction::Keep(relationship) => Edit::KeepJoin { relationship },
            ModelAction::Drop(relationship) => Edit::DropJoin { relationship },
            ModelAction::ChangeRole { dataset, role } => Edit::SetRole { dataset, role },
            ModelAction::Unfold(dataset) => Edit::Unfold { dataset },
            ModelAction::LeaveOut(dataset) => Edit::Exclude { dataset },
            ModelAction::ShowInFile => {
                self.view = ModelView::File;
                return Dispatched::Done("Showing the Ossie file".to_string());
            }
            ModelAction::OpenInGrid(selection) => return Dispatched::OpenInGrid(selection),
            ModelAction::ChangeKeys(_) => {
                return Dispatched::NotYet("Changing a join's keys isn't available yet".into())
            }
            ModelAction::FoldInto(_) => {
                return Dispatched::NotYet(
                    "Folding a table into another isn't available yet".into(),
                )
            }
            ModelAction::RetypeColumn(_) => {
                return Dispatched::NotYet("Retyping a column isn't available yet".into())
            }
            ModelAction::Include(_) => {
                return Dispatched::NotYet(
                    "Bringing a left-out table back isn't available yet".into(),
                )
            }
        };
        let said = edit_said(&edit);
        let model = match brightfield_ossie::apply(&self.model, &edit) {
            Ok(model) => model,
            Err(e) => return Dispatched::Failed(e.0),
        };
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
    /// for it, and run the Protocol.
    ///
    /// The run executes only the steps whose SQL changed, which are the silver and failed-value
    /// steps: arc skips a step whose SQL file and inputs hash as they did after its last success,
    /// and the load steps' are untouched. The pane takes `model` once the file is written, so a
    /// failed run leaves it showing what the folder holds.
    ///
    /// # Errors
    ///
    /// The reason the file or the silver steps could not be written, or what arc printed when
    /// the run failed.
    pub fn save_model(&mut self, model: Model) -> Result<(), String> {
        let path = self.protocol.dir.join(MODEL_FILE);
        let text = brightfield_ossie::write(&model);
        std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
        self.layout = brightfield_ossie::layout(&model);
        self.model = model;
        self.file_text = text;
        self.generation += 1;
        own_files::write_silver(&self.protocol, &self.model)?;
        own_files::run_loads(&self.protocol, &self.runner)
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
            ModelView::File => file_ui(&mut view_ui, &self.file_text, primary, &mut drawn),
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
                mode,
                &mut self.actions,
                &mut said,
            );
            drawn.inspector = Some(said);
        }
        ui.allocate_rect(area, egui::Sense::hover());
        self.drawn = drawn;
    }

    fn diagram_ui(&mut self, ui: &mut egui::Ui, drawn: &mut ModelDrawn) {
        let size = egui::vec2(self.layout.width as f32, self.layout.height as f32);
        let mut clicked = None;
        egui::ScrollArea::both()
            .id_salt("model-diagram")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                drawn.diagram = Some(rect);
                if let Some((_, texture)) = &self.raster {
                    ui.painter().image(
                        *texture,
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                if response.clicked() {
                    if let Some(at) = response.interact_pointer_pos() {
                        let local = at - rect.min;
                        clicked = Some(hit_test(
                            &self.layout,
                            f64::from(local.x),
                            f64::from(local.y),
                        ));
                    }
                }
            });
        if let Some(selected) = clicked {
            self.focus.selected = selected;
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
        other => format!("{other:?}"),
    }
}

/// The file view: the Ossie file as written, in the mono face. Read-only here.
fn file_ui(ui: &mut egui::Ui, text: &str, primary: egui::Color32, drawn: &mut ModelDrawn) {
    egui::ScrollArea::both()
        .id_salt("model-file")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(text)
                        .font(crate::protocol::mono_font())
                        .color(primary),
                )
                .extend(),
            );
        });
    drawn.body = text.lines().map(str::to_string).collect();
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
        if doc.perform(verb.as_str()) {
            Handled::Yes
        } else {
            Handled::No
        }
    }
}
