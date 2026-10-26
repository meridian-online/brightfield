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

use std::path::Path;

use brightfield_keys::{Altitude, BindingContext};
use brightfield_ossie::{ListRow, Model, ModelLayout, Role};
use brightfield_render::canvas_host::{Color, PixelSize};
use brightfield_render::model_scene::{hit_test, render_model, ModelFocus, Selection};
use brightfield_workbench::registry::Slot;
use brightfield_workbench::{
    chrome, EmptyState, Handled, Icon, Item, ItemCtx, ItemId, ItemRegistry, ItemSpec, PaneKey,
    Subject, Verb,
};
use meridian_design::{semantic, spacing};

use crate::canvas::EguiCanvasHost;
use crate::design::Mode;
use crate::discovery::MODEL_FILE;
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

        match self.view {
            ModelView::Diagram => self.diagram_ui(ui, &mut drawn),
            ModelView::List => list_ui(ui, &self.model, primary, secondary, &mut drawn),
            ModelView::File => file_ui(ui, &self.file_text, primary, &mut drawn),
        }
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

/// The list view: possible joins waiting on the analyst first, then a row per table, each with a
/// gutter naming the joins it takes part in.
fn list_ui(
    ui: &mut egui::Ui,
    model: &Model,
    primary: egui::Color32,
    secondary: egui::Color32,
    drawn: &mut ModelDrawn,
) {
    egui::ScrollArea::vertical()
        .id_salt("model-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in brightfield_ossie::list_rows(model) {
                let (gutter, text) = list_line(&row);
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [LIST_GUTTER, ui.spacing().interact_size.y],
                        egui::Label::new(egui::RichText::new(&gutter).color(secondary)),
                    );
                    ui.label(egui::RichText::new(&text).color(primary));
                });
                drawn.body.push(if gutter.is_empty() {
                    text
                } else {
                    format!("{gutter} | {text}")
                });
            }
        });
}

/// The width of the list's joins gutter, in logical points.
const LIST_GUTTER: f32 = 72.0;

/// One list row as its gutter and its text.
fn list_line(row: &ListRow) -> (String, String) {
    match row {
        ListRow::Review {
            relationship,
            edge_text,
        } => (
            "review".to_string(),
            format!("Possible join {relationship}: {edge_text}"),
        ),
        ListRow::Table {
            role,
            name,
            rows,
            key,
            fields,
            folded,
            joins,
        } => {
            let gutter = match joins.len() {
                0 => String::new(),
                1 => "1 join".to_string(),
                n => format!("{n} joins"),
            };
            let mut text = format!("{} · {name}", role_label(*role));
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
            (gutter, text)
        }
    }
}

fn role_label(role: Role) -> &'static str {
    match role {
        Role::Entity => "entity",
        Role::Event => "event",
        Role::Reference => "reference",
        Role::Bridge => "bridge",
        Role::Snapshot => "snapshot",
        Role::Unclear => "unclear",
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
