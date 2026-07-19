//! The egui Protocol panel — the asset-graph view rebuilt natively on the
//! egui/eframe shell (card 0029, the egui realisation of the retired gpui
//! `protocol_shell`).
//!
//! Structure, folding in the review of the first (gpui) cut:
//!
//! - **Real dock panes.** Outline, the DAG canvas, the Inspector, and the S
//!   steps sheet are independent [`egui_tiles`] panes in a resizable dock — not
//!   three columns nailed inside one panel. Outline · (Canvas/Steps tabs) ·
//!   Inspector is a horizontal split; `S` activates the Steps tab, `Esc` the
//!   Canvas tab.
//! - **Vertical flow by default** (more readable in a dock pane), with a toggle
//!   to the wide horizontal overview.
//! - **The keystroke grammar actually dispatches.** Raw egui key events are
//!   mapped to Protocol-context verbs *through the `brightfield-keys` registry*
//!   (the binding table is the registry's, not a duplicated map) and drive the
//!   framework-free [`ProtocolNav`] / [`StepsSheet`]: `h`/`l` producer/consumer,
//!   `j`/`k` siblings, `za` fold, `Enter`/`Esc` drill, `S` steps, `y` yank.
//! - **`za` re-lays-out the raster.** Folding the parameterised family under the
//!   cursor swaps the displayed graph (collapsed tile ⇄ expanded members),
//!   invalidates the raster cache, and re-presents through [`EguiCanvasHost`] —
//!   the family *visibly* collapses/expands in the canvas.
//!
//! The pure interaction model ([`ProtocolModel`]) is GPU-free and unit-tested;
//! [`ProtocolShell`] wires it to the dock, the Vello raster, and the chrome.

use std::collections::BTreeMap;
use std::path::Path;

use egui_tiles::{Container, SimplificationOptions, Tile, TileId, Tree, UiResponse};

use brightfield_protocol::contract_graph::{AssetMeta, SeamStatus, StepView};
use brightfield_protocol::graph::{AssetGraph, AssetId, AssetKind, SeamKind, StepId};
use brightfield_protocol::layout::{Flow, Layout, LayoutConfig, Rect};
use brightfield_protocol::panel::{
    inspector_for, kind_label, outline_rows, InspectorFacts, OutlineRow,
};
use brightfield_protocol::{collapse_families, FoldOutcome, ProtocolNav, StepRow, StepsSheet};

use brightfield_render::canvas_host::{CanvasHost, Color, PixelSize};

use meridian_design::chrome::INK_LIGHT;
use meridian_design::scales::GRAY_LIGHT;

use crate::canvas::EguiCanvasHost;
use crate::design::{self, Mode};

// ---------------------------------------------------------------------------
// Offline pipeline: arcform manifest -> asset graph + steps.
// ---------------------------------------------------------------------------

/// Everything the Protocol panel needs, assembled from the offline manifest
/// path (`BRIGHTFIELD_PROTOCOL_OFFLINE=1`): the collapsed + uncollapsed graphs
/// (the fold swaps between them), and the run-ordered step rows. Measured
/// contract maps (`statuses`/`assets`/`steps`) are empty offline — the inspector
/// then shows lineage detail only.
pub struct ProtocolInputs {
    /// The protocol name (breadcrumb + window title).
    pub protocol: String,
    /// Families collapsed to tiles — the default canvas + the nav's graph.
    pub graph_collapsed: AssetGraph,
    /// The full graph — shown when a family is unfolded.
    pub graph_full: AssetGraph,
    /// Per-step execution status (empty offline).
    pub statuses: BTreeMap<StepId, SeamStatus>,
    /// Per-asset measurements (empty offline).
    pub assets: BTreeMap<AssetId, AssetMeta>,
    /// Per-step detail (empty offline).
    pub steps: BTreeMap<StepId, StepView>,
    /// The S-sheet rows in run order.
    pub sheet_rows: Vec<StepRow>,
}

/// Load a protocol manifest (`arcform.yaml` + its `models/*.sql`) into
/// [`ProtocolInputs`] — the gated offline/fixture path, the same input
/// `brightfield-shot`/the shell consume.
///
/// # Errors
/// A human-readable message if the file cannot be read or is not a protocol
/// manifest.
pub fn load_protocol_offline(spec_path: &str) -> Result<ProtocolInputs, String> {
    let text = std::fs::read_to_string(spec_path)
        .map_err(|e| format!("read {spec_path}: {e}"))?;
    if !brightfield_protocol::is_protocol_manifest(&text) {
        return Err(format!("{spec_path} is not a protocol manifest"));
    }
    let manifest = brightfield_protocol::parse_manifest_str(&text)
        .map_err(|e| format!("protocol parse error: {e}"))?;
    let dir = Path::new(spec_path).parent().unwrap_or_else(|| Path::new("."));
    let sources = brightfield_protocol::graph::load_model_sources(&manifest, dir);
    let graph_full = brightfield_protocol::graph::build_graph(&manifest, &sources);
    let graph_collapsed = collapse_families(&graph_full);
    let sheet_rows = synth_sheet_rows(&graph_full);
    Ok(ProtocolInputs {
        protocol: manifest.name.clone(),
        graph_collapsed,
        graph_full,
        statuses: BTreeMap::new(),
        assets: BTreeMap::new(),
        steps: BTreeMap::new(),
        sheet_rows,
    })
}

/// Synthesise the flat run-ordered step rows from the graph's seams — the S
/// sheet's content on the offline path (no `ContractView`; status is unrun).
fn synth_sheet_rows(graph: &AssetGraph) -> Vec<StepRow> {
    graph
        .seams
        .values()
        .map(|seam| {
            let (kind, detail) = match &seam.kind {
                SeamKind::Op { name, .. } => ("op", name.clone()),
                SeamKind::Sql { model } => ("sql", model.clone()),
                SeamKind::Command => ("command", String::new()),
                SeamKind::Opaque => ("?", String::new()),
            };
            StepRow {
                order: seam.index,
                name: seam.step.clone(),
                label: seam.step.clone(),
                kind,
                detail,
                status: "—",
                live_state: None,
                skip_reason: None,
                gate: seam.gate,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Key dispatch — routed through the brightfield-keys registry.
// ---------------------------------------------------------------------------

/// Build the Protocol-context keystroke → verb-longname table from the
/// `brightfield-keys` registry, so the binding definitions are the single source
/// of truth (not a map duplicated here). Keyed by the registry's keystroke
/// strings (`"h"`, `"z a"`, `"shift-s"`, …).
fn protocol_key_table() -> BTreeMap<String, &'static str> {
    use brightfield_keys::registry::BindingContext;
    let mut table = BTreeMap::new();
    for verb in brightfield_keys::registry::registry() {
        for spec in &verb.binding_specs {
            if spec.context == BindingContext::Protocol {
                table.insert(spec.keystrokes.to_string(), verb.longname);
            }
        }
    }
    table
}

/// Map an egui key press to the registry keystroke token it stands for in the
/// Protocol context (`Key::H` → `"h"`, shift+`S` → `"shift-s"`). `z` is handled
/// as the `z a` chord prefix by the caller, not here.
fn key_token(key: egui::Key, mods: egui::Modifiers) -> Option<&'static str> {
    use egui::Key;
    Some(match key {
        Key::H => "h",
        Key::L => "l",
        Key::J => "j",
        Key::K => "k",
        Key::Y => "y",
        Key::Enter => "enter",
        Key::Escape => "escape",
        Key::S if mods.shift => "shift-s",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// ProtocolModel — the GPU-free interaction model (unit-tested).
// ---------------------------------------------------------------------------

/// The Protocol panel's pure state: the graphs, the nav/sheet cores, the
/// selection, and the view toggles. No GPU, no egui rendering — just the model
/// the shell renders and the key grammar drives.
pub struct ProtocolModel {
    /// Protocol name (breadcrumb).
    pub protocol: String,
    graph_collapsed: AssetGraph,
    graph_full: AssetGraph,
    statuses: BTreeMap<StepId, SeamStatus>,
    assets: BTreeMap<AssetId, AssetMeta>,
    steps: BTreeMap<StepId, StepView>,
    /// Nav over the collapsed graph (stable ids across a fold).
    nav: ProtocolNav,
    sheet: StepsSheet,
    /// The Family tile ids in the collapsed graph — a fold on any expands.
    family_ids: Vec<AssetId>,
    selected: Option<AssetId>,
    flow: Flow,
    show_sheet: bool,
    /// Whether the canvas shows the full (unfolded) graph.
    display_expanded: bool,
    /// `za` chord: a pending `z` awaiting `a`.
    pending_z: bool,
    /// The last yanked address, shown as a transient confirmation.
    yank_flash: Option<AssetId>,
    /// Drained by the shell to perform the actual clipboard copy.
    yank_request: Option<AssetId>,
    /// The current laid-out graph (matches `flow` + `display_expanded`).
    layout: Layout,
    layout_key: (bool, Flow),
    key_table: BTreeMap<String, &'static str>,
}

impl ProtocolModel {
    /// Build the model over `inputs` with the initial `flow`.
    #[must_use]
    pub fn new(inputs: ProtocolInputs, flow: Flow) -> Self {
        let nav = ProtocolNav::new(&inputs.graph_collapsed);
        let selected = nav.cursor().cloned();
        let sheet = StepsSheet::from_rows(inputs.sheet_rows);
        let family_ids: Vec<AssetId> = inputs
            .graph_collapsed
            .nodes
            .iter()
            .filter(|(_, n)| n.kind == AssetKind::Family)
            .map(|(id, _)| id.clone())
            .collect();
        // The initial canvas shows the collapsed graph.
        let cfg = LayoutConfig { flow, ..LayoutConfig::default() };
        let layout = brightfield_protocol::layout(&inputs.graph_collapsed, &cfg);
        Self {
            protocol: inputs.protocol,
            graph_collapsed: inputs.graph_collapsed,
            graph_full: inputs.graph_full,
            statuses: inputs.statuses,
            assets: inputs.assets,
            steps: inputs.steps,
            nav,
            sheet,
            family_ids,
            selected,
            flow,
            show_sheet: false,
            display_expanded: false,
            pending_z: false,
            yank_flash: None,
            yank_request: None,
            layout,
            layout_key: (false, flow),
            key_table: protocol_key_table(),
        }
    }

    /// The graph currently shown in the canvas (full when a family is unfolded).
    #[must_use]
    pub fn displayed_graph(&self) -> &AssetGraph {
        if self.display_expanded {
            &self.graph_full
        } else {
            &self.graph_collapsed
        }
    }

    /// The current layout (matches the displayed graph + flow).
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The current reading axis.
    #[must_use]
    pub fn flow(&self) -> Flow {
        self.flow
    }

    /// Whether the S steps sheet is active.
    #[must_use]
    pub fn show_sheet(&self) -> bool {
        self.show_sheet
    }

    /// Whether the canvas shows the unfolded family.
    #[must_use]
    pub fn is_expanded(&self) -> bool {
        self.display_expanded
    }

    /// The current selection (dotted asset id).
    #[must_use]
    pub fn selected(&self) -> Option<&AssetId> {
        self.selected.as_ref()
    }

    /// The last-yanked address confirmation, if any.
    #[must_use]
    pub fn yank_flash(&self) -> Option<&AssetId> {
        self.yank_flash.as_ref()
    }

    /// The steps sheet (rows + cursor).
    #[must_use]
    pub fn sheet(&self) -> &StepsSheet {
        &self.sheet
    }

    /// The outline rows in topological order (over the collapsed graph).
    #[must_use]
    pub fn outline(&self) -> Vec<OutlineRow> {
        outline_rows(&self.graph_collapsed, &self.statuses, self.selected.as_ref())
    }

    /// The inspector facts for the current selection.
    #[must_use]
    pub fn inspector(&self) -> InspectorFacts {
        inspector_for(
            &self.graph_collapsed,
            &self.assets,
            &self.steps,
            &self.statuses,
            self.selected.as_ref(),
        )
    }

    /// The drill breadcrumb labels, root → deepest.
    #[must_use]
    pub fn breadcrumb(&self) -> Vec<String> {
        self.nav
            .breadcrumb()
            .iter()
            .map(|id| {
                self.graph_collapsed
                    .nodes
                    .get(id)
                    .map_or_else(|| id.clone(), |n| n.label.clone())
            })
            .collect()
    }

    /// Point the selection (and nav cursor) at `id` — the shared entry a canvas
    /// click and an outline-row click both route through.
    pub fn select_id(&mut self, id: AssetId) {
        self.nav.focus(&id);
        self.selected = Some(id);
        self.yank_flash = None;
    }

    /// Flip the reading axis and re-lay-out.
    pub fn toggle_flow(&mut self) {
        self.flow = match self.flow {
            Flow::Vertical => Flow::Horizontal,
            Flow::Horizontal => Flow::Vertical,
        };
        self.recompute_layout();
    }

    /// Drain a pending yank (the address to copy to the clipboard), if any.
    pub fn take_yank_request(&mut self) -> Option<AssetId> {
        self.yank_request.take()
    }

    /// Feed one frame's egui events, dispatching key presses through the
    /// registry grammar. Returns whether anything changed (a repaint cue).
    pub fn feed_events(&mut self, events: &[egui::Event]) -> bool {
        let mut changed = false;
        for event in events {
            if let egui::Event::Key { key, pressed: true, modifiers, .. } = event {
                changed |= self.feed_key(*key, *modifiers);
            }
        }
        changed
    }

    /// Dispatch a single key press. Handles the `z a` fold chord.
    fn feed_key(&mut self, key: egui::Key, mods: egui::Modifiers) -> bool {
        // Resolve the `z a` chord: a pending `z` + `a` fires toggle-fold.
        if self.pending_z {
            self.pending_z = false;
            if key == egui::Key::A {
                // Resolve the `z a` chord to its verb through the registry table.
                return match self.key_table.get("z a").copied() {
                    Some(verb) => self.dispatch(verb),
                    None => false,
                };
            }
            // Otherwise fall through and treat this key normally.
        }
        if key == egui::Key::Z {
            self.pending_z = true;
            return false;
        }
        let Some(token) = key_token(key, mods) else { return false };
        match self.key_table.get(token).copied() {
            Some(verb) => self.dispatch(verb),
            None => false,
        }
    }

    /// Dispatch a resolved verb longname to the model action. Returns whether
    /// state changed.
    fn dispatch(&mut self, verb: &str) -> bool {
        match verb {
            "protocol-producer" => self.move_if(ProtocolNav::to_producer),
            "protocol-consumer" => self.move_if(ProtocolNav::to_consumer),
            "protocol-sibling-next" => {
                if self.show_sheet {
                    self.sheet.cursor_down()
                } else {
                    self.move_if(ProtocolNav::to_sibling_next)
                }
            }
            "protocol-sibling-prev" => {
                if self.show_sheet {
                    self.sheet.cursor_up()
                } else {
                    self.move_if(ProtocolNav::to_sibling_prev)
                }
            }
            "toggle-fold-family" => self.toggle_fold(),
            "protocol-drill-in" => self.move_if(ProtocolNav::drill_in),
            "protocol-drill-out" => {
                if self.show_sheet {
                    self.show_sheet = false;
                    true
                } else {
                    self.move_if(ProtocolNav::drill_out)
                }
            }
            "open-steps-sheet" => {
                let was = self.show_sheet;
                self.show_sheet = true;
                !was
            }
            "yank-address" => self.yank(),
            _ => false,
        }
    }

    /// Run a nav motion; on a real move, re-sync the selection from the cursor.
    fn move_if(&mut self, motion: fn(&mut ProtocolNav) -> bool) -> bool {
        if motion(&mut self.nav) {
            self.selected = self.nav.cursor().cloned();
            self.yank_flash = None;
            true
        } else {
            false
        }
    }

    /// `za` — fold/unfold the family under the cursor, swapping the displayed
    /// graph and invalidating the layout so the canvas visibly re-lays-out.
    fn toggle_fold(&mut self) -> bool {
        if self.nav.toggle_fold() == FoldOutcome::NotAFamily {
            return false;
        }
        self.display_expanded = self.family_ids.iter().any(|id| self.nav.is_expanded(id));
        self.selected = self.nav.cursor().cloned();
        self.recompute_layout();
        true
    }

    /// `y` — request the selected asset's dotted address be yanked (the shell
    /// performs the actual clipboard write) and flash a confirmation.
    fn yank(&mut self) -> bool {
        if let Some(id) = self.selected.clone() {
            self.yank_request = Some(id.clone());
            self.yank_flash = Some(id);
            true
        } else {
            false
        }
    }

    /// Recompute the layout for the current displayed graph + flow.
    fn recompute_layout(&mut self) {
        let cfg = LayoutConfig { flow: self.flow, ..LayoutConfig::default() };
        let graph = if self.display_expanded {
            &self.graph_full
        } else {
            &self.graph_collapsed
        };
        self.layout = brightfield_protocol::layout(graph, &cfg);
        self.layout_key = (self.display_expanded, self.flow);
    }

    /// The cache key the presented raster must match: (expanded, flow).
    #[must_use]
    fn layout_key(&self) -> (bool, Flow) {
        self.layout_key
    }
}

// ---------------------------------------------------------------------------
// Dock panes.
// ---------------------------------------------------------------------------

/// One dock citizen.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// The DAG canvas (the presented Vello raster).
    Canvas,
    /// The topological outline rail.
    Outline,
    /// The selection inspector.
    Inspector,
    /// The flat run-ordered steps sheet.
    Steps,
}

// ---------------------------------------------------------------------------
// ProtocolShell — model + dock + raster + chrome.
// ---------------------------------------------------------------------------

/// The full egui Protocol panel: the [`ProtocolModel`], the dock tree, the Vello
/// raster host, and the presented texture. [`ProtocolShell::draw`] is the single
/// frame source (live window, headless shot, snapshot) — the loop's guarantee.
pub struct ProtocolShell {
    model: ProtocolModel,
    dock: Tree<Pane>,
    tabs_id: TileId,
    canvas_id: TileId,
    steps_id: TileId,
    host: EguiCanvasHost,
    texture: Option<egui::TextureId>,
    /// (expanded, flow, dev_w, dev_h) the texture was last presented at.
    presented_key: Option<(bool, Flow, u32, u32)>,
    mode: Mode,
    fonts_installed: bool,
}

impl ProtocolShell {
    /// Build the shell over `inputs` and an [`EguiCanvasHost`] on the shell's
    /// shared device, with the initial `flow` (vertical by default).
    #[must_use]
    pub fn new(inputs: ProtocolInputs, host: EguiCanvasHost, mode: Mode, flow: Flow) -> Self {
        let model = ProtocolModel::new(inputs, flow);
        let (dock, tabs_id, canvas_id, steps_id) = build_dock();
        Self {
            model,
            dock,
            tabs_id,
            canvas_id,
            steps_id,
            host,
            texture: None,
            presented_key: None,
            mode,
            fonts_installed: false,
        }
    }

    /// The panel's natural window size in logical points — the DAG bounds plus
    /// the outline + inspector rails and the top/bottom chrome. Sized generously
    /// so the whole dock fits headlessly without clipping.
    #[must_use]
    pub fn window_size(&self) -> (f32, f32) {
        let l = self.model.layout();
        let w = (l.width as f32 + OUTLINE_W + INSPECTOR_W + 48.0).max(1100.0);
        let h = (l.height as f32 + 130.0).clamp(680.0, 1600.0);
        (w, h)
    }

    /// The window/tab title.
    #[must_use]
    pub fn title(&self) -> String {
        format!("Protocol · {}", self.model.protocol)
    }

    /// Mutable access to the model (for the shot's `--focus` seed).
    pub fn model_mut(&mut self) -> &mut ProtocolModel {
        &mut self.model
    }

    /// Re-raster the DAG through the host only when (expanded, flow, resolution)
    /// changed. Presents the current displayed graph via the T46 seam.
    fn ensure_presented(&mut self, ppp: f32) {
        let (expanded, flow) = self.model.layout_key();
        let l = self.model.layout();
        let dev = PixelSize {
            width: ((l.width as f32) * ppp).round().max(1.0) as u32,
            height: ((l.height as f32) * ppp).round().max(1.0) as u32,
        };
        let key = (expanded, flow, dev.width, dev.height);
        if self.presented_key == Some(key) && self.texture.is_some() {
            return;
        }
        let base = Color::from_token(INK_LIGHT.page);
        // Build the scene under an immutable borrow, then present (mutable host).
        let scene = {
            let mut s = vello::Scene::new();
            brightfield_render::asset_scene::render_asset_graph_with_status(
                &mut s,
                self.model.layout(),
                self.model.displayed_graph(),
                &self.model.statuses,
            );
            let mut scaled = vello::Scene::new();
            scaled.append(&s, Some(kurbo::Affine::scale(f64::from(ppp))));
            scaled
        };
        let id = self.host.present_scene(&scene, dev, base);
        self.texture = Some(id);
        self.presented_key = Some(key);
    }

    /// Before rendering: make the active Canvas/Steps tab authoritative from the
    /// model's `show_sheet` (so the `S`/`Esc` keys drive it).
    fn set_active_tab(&mut self) {
        let want = if self.model.show_sheet { self.steps_id } else { self.canvas_id };
        if let Some(Tile::Container(Container::Tabs(tabs))) = self.dock.tiles.get_mut(self.tabs_id) {
            tabs.set_active(want);
        }
    }

    /// After rendering: read a manual tab click back into `show_sheet` (so a
    /// pointer click on the Steps tab also opens the sheet, and Canvas closes it).
    fn read_active_tab(&mut self) {
        if let Some(Tile::Container(Container::Tabs(tabs))) = self.dock.tiles.get(self.tabs_id) {
            if let Some(active) = tabs.active {
                self.model.show_sheet = active == self.steps_id;
            }
        }
    }

    /// Draw one Protocol frame into `ui` — the single tier-agnostic source.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.fonts_installed {
            design::apply(&ctx, self.mode);
            self.fonts_installed = true;
        }

        // Dispatch this frame's key grammar, then perform any yank.
        let events = ctx.input(|i| i.events.clone());
        self.model.feed_events(&events);
        if let Some(addr) = self.model.take_yank_request() {
            ctx.copy_text(addr);
        }

        self.ensure_presented(ctx.pixels_per_point());
        self.set_active_tab();

        // Orientation chrome: breadcrumb (top) + key-hint bar (bottom).
        egui::containers::Panel::top("proto-breadcrumb")
            .resizable(false)
            .show(ui, |ui| self.breadcrumb_ui(ui));
        egui::containers::Panel::bottom("proto-hint")
            .resizable(false)
            .show(ui, |ui| hint_ui(ui, &self.model));

        // The dock fills the rest (disjoint borrows of model + dock through self).
        egui::containers::CentralPanel::default().show(ui, |ui| {
            let mut behavior = ProtocolBehavior {
                model: &mut self.model,
                texture: self.texture,
                canvas_id: self.canvas_id,
            };
            self.dock.ui(&mut behavior, ui);
        });

        // Read a manual Canvas/Steps tab click back into the model.
        self.read_active_tab();
    }

    /// The top breadcrumb + a flow toggle (mutates the model, so it lives on the
    /// shell rather than the free `hint_ui`).
    fn breadcrumb_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Protocol · {}", self.model.protocol))
                    .color(ink(INK_LIGHT.ink_primary))
                    .strong(),
            );
            let sel = self
                .model
                .selected()
                .and_then(|id| self.model.graph_collapsed.nodes.get(id))
                .map(|n| n.label.clone());
            if let Some(label) = sel {
                ui.label(egui::RichText::new("›").color(ink(INK_LIGHT.ink_muted)));
                ui.label(egui::RichText::new(label).color(ink(INK_LIGHT.ink_secondary)));
            }
            for crumb in self.model.breadcrumb() {
                ui.label(egui::RichText::new("»").color(ink(INK_LIGHT.ink_muted)));
                ui.label(egui::RichText::new(crumb).color(ink(INK_LIGHT.ink_secondary)).small());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (word, next) = match self.model.flow() {
                    Flow::Vertical => ("vertical", "horizontal"),
                    Flow::Horizontal => ("horizontal", "vertical"),
                };
                if ui
                    .button(egui::RichText::new(format!("flow: {word} ⇄")).small())
                    .on_hover_text(format!("switch to {next} flow"))
                    .clicked()
                {
                    self.model.toggle_flow();
                    self.presented_key = None;
                }
            });
        });
        ui.add_space(4.0);
    }
}

/// Build the dock tree: Outline · (Canvas | Steps tabs) · Inspector, a
/// horizontal split. Returns the tree and the (tabs, canvas, steps) tile ids.
fn build_dock() -> (Tree<Pane>, TileId, TileId, TileId) {
    let mut tiles = egui_tiles::Tiles::default();
    let outline = tiles.insert_pane(Pane::Outline);
    let canvas = tiles.insert_pane(Pane::Canvas);
    let steps = tiles.insert_pane(Pane::Steps);
    let inspector = tiles.insert_pane(Pane::Inspector);
    let tabs = tiles.insert_tab_tile(vec![canvas, steps]);
    // Canvas is the default active tab.
    if let Some(Tile::Container(Container::Tabs(t))) = tiles.get_mut(tabs) {
        t.set_active(canvas);
    }
    let root = tiles.insert_horizontal_tile(vec![outline, tabs, inspector]);
    // Give the rails a fixed-ish share; the canvas gets the bulk.
    if let Some(Tile::Container(Container::Linear(lin))) = tiles.get_mut(root) {
        lin.shares.set_share(outline, 0.24);
        lin.shares.set_share(tabs, 0.54);
        lin.shares.set_share(inspector, 0.22);
    }
    (Tree::new("protocol-dock", root, tiles), tabs, canvas, steps)
}

// ---------------------------------------------------------------------------
// Behavior — renders each pane from the model.
// ---------------------------------------------------------------------------

struct ProtocolBehavior<'a> {
    model: &'a mut ProtocolModel,
    texture: Option<egui::TextureId>,
    canvas_id: TileId,
}

impl egui_tiles::Behavior<Pane> for ProtocolBehavior<'_> {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        match pane {
            Pane::Canvas => "Canvas",
            Pane::Steps => "Steps",
            Pane::Outline => "Outline",
            Pane::Inspector => "Inspector",
        }
        .into()
    }

    fn simplification_options(&self) -> SimplificationOptions {
        // Keep the authored dock shape stable (no pruning / no auto-tabbing).
        SimplificationOptions::OFF
    }

    fn pane_ui(&mut self, ui: &mut egui::Ui, _id: TileId, pane: &mut Pane) -> UiResponse {
        match pane {
            Pane::Canvas => canvas_ui(ui, self.model, self.texture, self.canvas_id),
            Pane::Outline => outline_ui(ui, self.model),
            Pane::Inspector => inspector_ui(ui, self.model),
            Pane::Steps => steps_ui(ui, self.model),
        }
        UiResponse::None
    }
}

// ---------------------------------------------------------------------------
// Pane renderers (native egui) + chrome.
// ---------------------------------------------------------------------------

const OUTLINE_W: f32 = 260.0;
const INSPECTOR_W: f32 = 300.0;
const ROW_H: f32 = 22.0;

/// Meridian token → egui colour (light chrome — the DAG raster is light-first).
fn ink(c: meridian_design::colour::Rgba) -> egui::Color32 {
    design::to_color32(c)
}

fn status_color(s: SeamStatus) -> egui::Color32 {
    ink(match s {
        SeamStatus::Ok => meridian_design::viz::STATUS.good,
        SeamStatus::Running => meridian_design::viz::STATUS.warning,
        SeamStatus::Skipped => GRAY_LIGHT[6],
        SeamStatus::Failed => meridian_design::viz::STATUS.critical,
        SeamStatus::NotRun => GRAY_LIGHT[5],
    })
}

fn status_word(s: SeamStatus) -> &'static str {
    match s {
        SeamStatus::Ok => "ok",
        SeamStatus::Running => "running",
        SeamStatus::Skipped => "skipped",
        SeamStatus::Failed => "failed",
        SeamStatus::NotRun => "not run",
    }
}

/// The DAG canvas pane: the presented Vello raster in a scroll area, with a
/// selection highlight and click → select hit-testing.
fn canvas_ui(
    ui: &mut egui::Ui,
    model: &mut ProtocolModel,
    texture: Option<egui::TextureId>,
    _canvas_id: TileId,
) {
    let Some(texture) = texture else {
        ui.centered_and_justified(|ui| ui.label("presenting…"));
        return;
    };
    let (w, h) = {
        let l = model.layout();
        (l.width as f32, l.height as f32)
    };
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click_and_drag());
            egui::Image::new((texture, rect.size()))
                .tint(egui::Color32::WHITE)
                .paint_at(ui, rect);

            // Selection highlight over the selected node's layout rect.
            if let Some(sel) = model.selected().cloned() {
                if let Some(node) = model.layout().positions.get(&sel).cloned() {
                    let r = node_rect(rect.min, &node);
                    ui.painter().rect_stroke(
                        r,
                        4.0,
                        egui::Stroke::new(2.0, ink(INK_LIGHT.focus)),
                        egui::StrokeKind::Outside,
                    );
                }
            }

            // Click → select: hit-test canvas-local coords against the layout.
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let lx = f64::from(p.x - rect.min.x);
                    let ly = f64::from(p.y - rect.min.y);
                    if let Some(id) = hit_test(model.layout(), lx, ly) {
                        model.select_id(id);
                    }
                }
            }
        });
}

fn node_rect(origin: egui::Pos2, r: &Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(origin.x + r.x as f32, origin.y + r.y as f32),
        egui::vec2(r.width as f32, r.height as f32),
    )
}

/// The topmost node rect containing `(lx, ly)` (id-descending tie-break).
fn hit_test(layout: &Layout, lx: f64, ly: f64) -> Option<AssetId> {
    layout
        .positions
        .iter()
        .rev()
        .find(|(_, r)| lx >= r.x && lx <= r.x + r.width && ly >= r.y && ly <= r.y + r.height)
        .map(|(id, _)| id.clone())
}

/// The outline rail: topological rows, status dot + label + kind, click to
/// select.
fn outline_ui(ui: &mut egui::Ui, model: &mut ProtocolModel) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("OUTLINE").color(ink(INK_LIGHT.ink_muted)).small());
    ui.separator();
    let rows = model.outline();
    let mut clicked: Option<AssetId> = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for row in rows {
            ui.horizontal(|ui| {
                let (dot, _) =
                    ui.allocate_exact_size(egui::vec2(12.0, ROW_H), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot.center(), 3.5, status_color(row.status));
                let ink_c = if row.selected {
                    ink(INK_LIGHT.ink_primary)
                } else {
                    ink(INK_LIGHT.ink_secondary)
                };
                let resp = ui.selectable_label(
                    row.selected,
                    egui::RichText::new(truncate(&row.label, 26)).color(ink_c),
                );
                if resp.clicked() {
                    clicked = Some(row.id.clone());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(kind_label(row.kind))
                            .color(ink(INK_LIGHT.ink_muted))
                            .small(),
                    );
                });
            });
        }
    });
    if let Some(id) = clicked {
        model.select_id(id);
    }
}

/// The inspector: the selected asset's S/R/C/E/D detail.
fn inspector_ui(ui: &mut egui::Ui, model: &ProtocolModel) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("INSPECTOR").color(ink(INK_LIGHT.ink_muted)).small());
    ui.separator();
    let facts = model.inspector();
    if !facts.present {
        ui.label(egui::RichText::new("no selection").color(ink(INK_LIGHT.ink_muted)));
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.label(
            egui::RichText::new(facts.label.clone())
                .color(ink(INK_LIGHT.ink_primary))
                .heading(),
        );
        ui.label(
            egui::RichText::new(kind_label(facts.kind))
                .color(ink(INK_LIGHT.ink_secondary))
                .small(),
        );
        ui.add_space(6.0);
        kv(ui, "address", &facts.address);
        kv(
            ui,
            "produced by",
            facts.producing_step.as_deref().unwrap_or("— external input"),
        );
        if let Some(t) = &facts.transform {
            kv(ui, "transform", t);
        }
        kv(ui, "status", status_word(facts.status));
        if let Some(rc) = facts.row_count {
            kv(ui, "rows", &rc.to_string());
        }
        if let Some(m) = facts.materialized {
            kv(ui, "materialized", if m { "yes" } else { "no" });
        }
        if let Some(b) = facts.bytes {
            kv(ui, "bytes", &b.to_string());
        }
        if let Some(issue) = &facts.issue {
            kv(ui, "issue", issue);
        }
    });
}

fn kv(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(label).color(ink(INK_LIGHT.ink_muted)).small());
    ui.label(egui::RichText::new(value).color(ink(INK_LIGHT.ink_secondary)));
}

/// The S steps sheet: the flat run-ordered step list as a grid.
fn steps_ui(ui: &mut egui::Ui, model: &ProtocolModel) {
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("STEPS — the flat run-ordered list")
            .color(ink(INK_LIGHT.ink_muted))
            .small(),
    );
    ui.separator();
    let cursor = model.sheet().cursor();
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("proto-steps-grid")
            .striped(true)
            .num_columns(6)
            .spacing(egui::vec2(16.0, 4.0))
            .show(ui, |ui| {
                for h in brightfield_protocol::sheet::COLUMNS {
                    ui.label(egui::RichText::new(h).color(ink(INK_LIGHT.ink_muted)).strong().small());
                }
                ui.end_row();
                for (i, row) in model.sheet().rows().iter().enumerate() {
                    let sel = i == cursor;
                    let c = if sel { ink(INK_LIGHT.ink_primary) } else { ink(INK_LIGHT.ink_secondary) };
                    let name = if row.gate {
                        format!("{} ◈", row.name)
                    } else {
                        row.name.clone()
                    };
                    let mark = if sel { "▸ " } else { "  " };
                    ui.label(egui::RichText::new(format!("{mark}{}", row.order)).color(c).monospace());
                    ui.label(egui::RichText::new(name).color(c));
                    ui.label(egui::RichText::new(row.kind).color(ink(INK_LIGHT.ink_secondary)).small());
                    ui.label(egui::RichText::new(truncate(&row.detail, 40)).color(ink(INK_LIGHT.ink_secondary)).small());
                    ui.label(egui::RichText::new(row.status).color(ink(INK_LIGHT.ink_secondary)).small());
                    let live = row.live_state.clone().or_else(|| row.skip_reason.clone()).unwrap_or_default();
                    ui.label(egui::RichText::new(live).color(ink(INK_LIGHT.ink_muted)).small());
                    ui.end_row();
                }
            });
    });
}

/// The bottom key-hint bar + a flow/state indicator (read-only — no model
/// mutation, so it is a free function).
fn hint_ui(ui: &mut egui::Ui, model: &ProtocolModel) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(
                "h/l producer·consumer   j/k siblings   za fold   Enter/Esc drill   S steps   y yank",
            )
            .color(ink(INK_LIGHT.ink_muted))
            .monospace()
            .small(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(addr) = model.yank_flash() {
                ui.label(
                    egui::RichText::new(format!("yanked {addr}"))
                        .color(ink(INK_LIGHT.focus))
                        .small(),
                );
            } else {
                let state = if model.is_expanded() { "family: expanded" } else { "family: collapsed" };
                ui.label(egui::RichText::new(state).color(ink(INK_LIGHT.ink_muted)).small());
            }
        });
    });
    ui.add_space(2.0);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// A convenience the shot/live binaries share: build the host on a device.
/// Build an [`EguiCanvasHost`] on a device + queue with its own Vello renderer
/// and egui-wgpu renderer registration (the shot's offscreen path; the live
/// window reuses eframe's shared renderer instead).
#[must_use]
pub fn host_on_device(
    device: vello::wgpu::Device,
    queue: vello::wgpu::Queue,
    egui_renderer: crate::canvas::SharedEguiRenderer,
) -> EguiCanvasHost {
    let vello = brightfield_render::vello_renderer::VelloRenderer::from_shared(
        device.clone(),
        queue.clone(),
    );
    EguiCanvasHost::new(device, queue, vello, egui_renderer)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDGAR: &str = "../../examples/protocol/edgar_gleif/arcform.yaml";

    fn model() -> ProtocolModel {
        let inputs = load_protocol_offline(EDGAR).expect("load edgar_gleif");
        ProtocolModel::new(inputs, Flow::Vertical)
    }

    /// The offline pipeline builds both graphs; the collapsed one has exactly one
    /// Family tile (the ncen fetch/extract cycle), and the full one is larger.
    #[test]
    fn pipeline_builds_family_collapsed_and_full() {
        let inputs = load_protocol_offline(EDGAR).expect("load");
        let families = inputs
            .graph_collapsed
            .nodes
            .values()
            .filter(|n| n.kind == AssetKind::Family)
            .count();
        assert_eq!(families, 1, "edgar_gleif has one parameterised family");
        assert!(
            inputs.graph_full.nodes.len() > inputs.graph_collapsed.nodes.len(),
            "the full graph has more nodes than the collapsed one"
        );
        assert!(!inputs.sheet_rows.is_empty(), "the steps sheet has rows");
    }

    /// The key table is sourced from the registry and covers the Protocol verbs.
    #[test]
    fn key_table_covers_protocol_grammar() {
        let t = protocol_key_table();
        assert_eq!(t.get("h").copied(), Some("protocol-producer"));
        assert_eq!(t.get("l").copied(), Some("protocol-consumer"));
        assert_eq!(t.get("j").copied(), Some("protocol-sibling-next"));
        assert_eq!(t.get("k").copied(), Some("protocol-sibling-prev"));
        assert_eq!(t.get("z a").copied(), Some("toggle-fold-family"));
        assert_eq!(t.get("enter").copied(), Some("protocol-drill-in"));
        assert_eq!(t.get("escape").copied(), Some("protocol-drill-out"));
        assert_eq!(t.get("shift-s").copied(), Some("open-steps-sheet"));
        assert_eq!(t.get("y").copied(), Some("yank-address"));
    }

    fn key(k: egui::Key) -> egui::Event {
        egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn key_shift(k: egui::Key) -> egui::Event {
        egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers { shift: true, ..Default::default() },
        }
    }

    /// `l` moves the selection downstream and syncs the outline mark.
    #[test]
    fn consumer_key_moves_selection() {
        let mut m = model();
        let start = m.selected().cloned();
        let moved = m.feed_events(&[key(egui::Key::L)]);
        assert!(moved, "l dispatched a consumer move");
        assert_ne!(m.selected().cloned(), start, "the selection advanced");
        assert_eq!(
            m.outline().iter().filter(|r| r.selected).count(),
            1,
            "exactly one outline row is marked"
        );
    }

    /// `S` opens the steps sheet; `Esc` closes it.
    #[test]
    fn shift_s_opens_sheet_esc_closes() {
        let mut m = model();
        assert!(!m.show_sheet());
        m.feed_events(&[key_shift(egui::Key::S)]);
        assert!(m.show_sheet(), "S opened the steps sheet");
        m.feed_events(&[key(egui::Key::Escape)]);
        assert!(!m.show_sheet(), "Esc closed the steps sheet");
    }

    /// `za` on the family re-lays-out: the displayed graph and the layout both
    /// change (collapsed tile ⇄ expanded members). Sending `z` then `a`.
    #[test]
    fn za_chord_folds_and_relayouts() {
        let mut m = model();
        // Focus the family tile (a click / --focus would do this live).
        let family = m
            .graph_collapsed
            .nodes
            .iter()
            .find(|(_, n)| n.kind == AssetKind::Family)
            .map(|(id, _)| id.clone())
            .expect("family");
        m.select_id(family.clone());
        assert!(!m.is_expanded());
        let before_nodes = m.layout().positions.len();
        let before_key = m.layout_key();

        // The chord: z then a.
        m.feed_events(&[key(egui::Key::Z)]);
        let changed = m.feed_events(&[key(egui::Key::A)]);
        assert!(changed, "za toggled the fold");
        assert!(m.is_expanded(), "the family is now expanded");
        assert_ne!(m.layout_key(), before_key, "the layout cache key changed");
        assert!(
            m.layout().positions.len() > before_nodes,
            "the expanded raster has more nodes ({} > {})",
            m.layout().positions.len(),
            before_nodes
        );

        // za again collapses back.
        m.feed_events(&[key(egui::Key::Z)]);
        m.feed_events(&[key(egui::Key::A)]);
        assert!(!m.is_expanded(), "za again folded the family");
        assert_eq!(m.layout().positions.len(), before_nodes, "back to the tile");
    }

    /// The flow toggle transposes the layout (vertical taller than wide →
    /// horizontal wider than tall).
    #[test]
    fn flow_toggle_transposes_layout() {
        let mut m = model();
        assert_eq!(m.flow(), Flow::Vertical);
        let (vw, vh) = (m.layout().width, m.layout().height);
        assert!(vh > vw, "vertical is taller than wide: {vw}x{vh}");
        m.toggle_flow();
        assert_eq!(m.flow(), Flow::Horizontal);
        let (hw, hh) = (m.layout().width, m.layout().height);
        assert!(hw > hh, "horizontal is wider than tall: {hw}x{hh}");
    }

    /// `y` requests a yank of the selected dotted address (never a screen
    /// position) and flashes it.
    #[test]
    fn yank_requests_the_dotted_address() {
        let mut m = model();
        let sel = m.selected().cloned().expect("a boot selection");
        m.feed_events(&[key(egui::Key::Y)]);
        assert_eq!(m.take_yank_request(), Some(sel.clone()));
        assert_eq!(m.yank_flash(), Some(&sel));
    }
}
