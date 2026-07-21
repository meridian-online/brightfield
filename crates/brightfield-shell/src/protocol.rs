//! The egui Protocol panel — the asset-graph view, expressed as four
//! [`Item`]s on the workbench shell contract.
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
//! # What this file no longer does
//!
//! It used to draw its own chrome. Three pane headers, spelled three different
//! ways; a second heading four pixels off the first inside the inspector body;
//! five treatments of "this thing is selected"; seven places where "there is
//! nothing here" was answered ad hoc, two of them by rendering a header and
//! silence; a colour layer hardcoded to `INK_LIGHT`, so dark mode drew light
//! ink on a dark page. All of that is gone, and none of it is replaced in
//! kind: a pane now **declares** a [`Subject`] and
//! [`brightfield_workbench::PaneChrome`] draws the header band, the empty
//! state, the selection wash and the focus ring from the token layer.
//!
//! What is still this file's own is the panel's top breadcrumb bar and its
//! bottom key-hint bar, because [`ProtocolShell`] still owns a window of its
//! own. Those move to the shell's toolbar row and status rail when the
//! one-app migration lands; they are the last chrome here that a `Subject`
//! does not describe.
//!
//! The pure interaction model ([`ProtocolModel`]) is GPU-free and unit-tested;
//! [`ProtocolDoc`] adds the canvas host the panes share, and [`ProtocolShell`]
//! wires the document to the dock.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use egui_tiles::{Container, Tile, Tree};

use brightfield_protocol::contract_graph::{AssetMeta, SeamStatus, StepView};
use brightfield_protocol::graph::{AssetGraph, AssetId, AssetKind, SeamKind, StepId};
use brightfield_protocol::layout::{Flow, Layout, LayoutConfig, Rect};
use brightfield_protocol::panel::{
    inspector_for, kind_label, outline_rows, InspectorFacts, OutlineRow,
};
use brightfield_protocol::{collapse_families, Dir, FoldOutcome, ProtocolNav, StepRow, StepsSheet};

use brightfield_render::canvas_host::{Color, PixelSize};

use brightfield_keys::BindingContext;
use brightfield_workbench::registry::{DockSide, Slot};
use brightfield_workbench::workspace::{tabbed_tiles_of, tabs_holding, tile_of};
use brightfield_workbench::{
    chrome, EmptyState, Icon, Item, ItemCtx, ItemId, ItemMap, ItemRegistry, ItemSpec, PaneChrome,
    PaneKey, Request, Subject, Tone, Verb, ViewKind,
};

use meridian_design::{control, semantic, spacing};

use crate::canvas::{CanvasSlot, EguiCanvasHost};
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

impl ProtocolInputs {
    /// Inputs with nothing in them — no assets, no seams, no steps.
    ///
    /// The document [`brightfield_workbench::audit`] runs a registry over: the
    /// gate constructs every pane and asks each for its [`Subject`] over an
    /// empty document, so "empty" has to be a value this crate can build
    /// without a manifest, a device or a window. It is also the state a first
    /// run of the panel would show if the manifest it opened declared nothing,
    /// which is exactly the case each pane's empty state is written for.
    #[must_use]
    pub fn empty() -> Self {
        let graph = AssetGraph {
            protocol: String::new(),
            nodes: BTreeMap::new(),
            seams: BTreeMap::new(),
            edges: Vec::new(),
        };
        Self {
            protocol: String::new(),
            graph_collapsed: graph.clone(),
            graph_full: graph,
            statuses: BTreeMap::new(),
            assets: BTreeMap::new(),
            steps: BTreeMap::new(),
            sheet_rows: Vec::new(),
        }
    }
}

/// Load a protocol manifest (`arcform.yaml` + its `models/*.sql`) into
/// [`ProtocolInputs`] — the gated offline/fixture path, the same input
/// `brightfield-shot`/the shell consume.
///
/// # Errors
/// A human-readable message if the file cannot be read or is not a protocol
/// manifest.
pub fn load_protocol_offline(spec_path: &str) -> Result<ProtocolInputs, String> {
    let text = std::fs::read_to_string(spec_path).map_err(|e| format!("read {spec_path}: {e}"))?;
    if !brightfield_protocol::is_protocol_manifest(&text) {
        return Err(format!("{spec_path} is not a protocol manifest"));
    }
    let manifest = brightfield_protocol::parse_manifest_str(&text)
        .map_err(|e| format!("protocol parse error: {e}"))?;
    let dir = Path::new(spec_path)
        .parent()
        .unwrap_or_else(|| Path::new("."));
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
/// the panes render and the key grammar drives.
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
    /// The drill scope: when a node is drilled into (`Enter`), the canvas shows
    /// this induced local slice instead of the whole graph; `Esc` pops it.
    scope_graph: Option<AssetGraph>,
    /// `za` chord: a pending `z` awaiting `a`.
    pending_z: bool,
    /// The last yanked address, shown as a transient confirmation.
    yank_flash: Option<AssetId>,
    /// Drained by the shell to perform the actual clipboard copy.
    yank_request: Option<AssetId>,
    /// The current laid-out graph (matches `flow` + `display_expanded` + scope).
    layout: Layout,
    layout_key: (bool, Flow),
    /// Bumps on every re-layout — the raster cache invalidates on a scope/drill
    /// change too, not just an expand/flow flip.
    layout_gen: u64,
    /// Bumps whenever a keyboard move changes the selection — the cue for the
    /// canvas to keep the freshly-selected node in frame (see `request_frame`).
    frame_gen: u64,
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
        let cfg = LayoutConfig {
            flow,
            ..LayoutConfig::default()
        };
        let layout = brightfield_protocol::layout(&inputs.graph_collapsed, &cfg);
        let mut model = Self {
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
            scope_graph: None,
            pending_z: false,
            yank_flash: None,
            yank_request: None,
            layout,
            layout_key: (false, flow),
            layout_gen: 0,
            frame_gen: 0,
            key_table: protocol_key_table(),
        };
        // Seed the nav's spatial geometry from the collapsed layout so the very
        // first keystroke moves along the drawn flow.
        model.sync_nav_geometry();
        model
    }

    /// Feed the nav the collapsed graph's rendered geometry at the current flow,
    /// so `hjkl` resolve to the on-screen producer/consumer/sibling. The nav
    /// always walks the collapsed graph, so this is its geometry regardless of a
    /// fold or drill scope; only a flow change alters it.
    fn sync_nav_geometry(&mut self) {
        let cfg = LayoutConfig {
            flow: self.flow,
            ..LayoutConfig::default()
        };
        let geom = brightfield_protocol::layout(&self.graph_collapsed, &cfg);
        self.nav.set_geometry(self.flow, &geom);
    }

    /// The graph currently shown in the canvas: the drill scope when one is
    /// active, else the full graph when a family is unfolded, else the collapsed
    /// graph.
    #[must_use]
    pub fn displayed_graph(&self) -> &AssetGraph {
        if let Some(scope) = &self.scope_graph {
            scope
        } else if self.display_expanded {
            &self.graph_full
        } else {
            &self.graph_collapsed
        }
    }

    /// Whether the protocol declares any assets at all.
    ///
    /// The outline rail's empty-state test. A count over
    /// [`ProtocolModel::outline`] would answer the same question and allocate a
    /// row vector to do it, once per pane per frame — and a `Subject` is asked
    /// for on every frame, including for panes that are not drawing.
    #[must_use]
    pub fn has_assets(&self) -> bool {
        !self.graph_collapsed.nodes.is_empty()
    }

    /// Whether the selection names an asset that is still in the graph.
    ///
    /// The inspector's empty-state test, and deliberately stricter than
    /// `selected().is_some()`: a stale id would render an inspector with every
    /// field blank, which is the failure mode the empty state exists to
    /// replace.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|id| self.graph_collapsed.nodes.contains_key(id))
    }

    /// Whether a drill scope is focusing the canvas on a local neighbourhood.
    #[must_use]
    pub fn is_drilled(&self) -> bool {
        self.scope_graph.is_some()
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
        outline_rows(
            &self.graph_collapsed,
            &self.statuses,
            self.selected.as_ref(),
        )
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

    /// Flip the reading axis and re-lay-out. The nav's spatial geometry is
    /// re-seeded so `hjkl` follow the new axis, and the selection is re-framed —
    /// a transpose is the most disruptive layout change, so (like the drill
    /// actions) it re-centres the selection rather than leaving it off-screen.
    pub fn toggle_flow(&mut self) {
        self.flow = match self.flow {
            Flow::Vertical => Flow::Horizontal,
            Flow::Horizontal => Flow::Vertical,
        };
        self.sync_nav_geometry();
        self.recompute_layout();
        self.request_frame();
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
            if let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            {
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
        // `t` transposes the reading axis — the keyboard twin of the "flow: …"
        // click control.
        if key == egui::Key::T {
            self.toggle_flow();
            return true;
        }
        // Backspace is a plain-key fallback for the widen/reset that Esc drives —
        // an ordinary hardware key that always reaches the app, so widening never
        // depends on a remapped or app-synthesized Escape arriving intact.
        if key == egui::Key::Backspace {
            return self.drill_out();
        }
        let Some(token) = key_token(key, mods) else {
            return false;
        };
        match self.key_table.get(token).copied() {
            Some(verb) => self.dispatch(verb),
            None => false,
        }
    }

    /// Dispatch a resolved verb longname to the model action. Returns whether
    /// state changed. The four motion verbs are bound to fixed vim keys; each
    /// maps to that key's **pixel direction**, and [`ProtocolNav::move_dir`]
    /// resolves the direction to a producer/consumer or sibling by the flow — so
    /// `h`/`l` and `j`/`k` always move along the drawn layout.
    ///
    /// Public because it is now the *second* way a verb reaches the model: a
    /// keystroke resolves through [`ProtocolModel::feed_events`], and a control
    /// a pane declares in its [`Subject`] arrives as a
    /// [`Request::Verb`](brightfield_workbench::Request) the shell drains after
    /// the frame. Both land here, so a click and a keystroke cannot drift into
    /// two implementations of one verb.
    pub fn dispatch(&mut self, verb: &str) -> bool {
        match verb {
            "protocol-producer" => self.move_dir(Dir::Left), // h
            "protocol-consumer" => self.move_dir(Dir::Right), // l
            "protocol-sibling-next" => {
                if self.show_sheet {
                    self.sheet.cursor_down()
                } else {
                    self.move_dir(Dir::Down) // j
                }
            }
            "protocol-sibling-prev" => {
                if self.show_sheet {
                    self.sheet.cursor_up()
                } else {
                    self.move_dir(Dir::Up) // k
                }
            }
            "toggle-fold-family" => self.toggle_fold(),
            "protocol-drill-in" => self.drill_in(),
            "protocol-drill-out" => self.drill_out(),
            "open-steps-sheet" => {
                // Toggle, so `S` both opens and closes the sheet — pressing it
                // again is the obvious way back to the canvas (Esc/Backspace also
                // close it via drill_out).
                self.show_sheet = !self.show_sheet;
                true
            }
            "yank-address" => self.yank(),
            _ => false,
        }
    }

    /// Move the cursor one node in the pressed vim direction (resolved to the
    /// on-screen producer/consumer/sibling by the flow); on a real move, re-sync
    /// the selection from the cursor.
    fn move_dir(&mut self, dir: Dir) -> bool {
        if self.nav.move_dir(dir) {
            self.selected = self.nav.cursor().cloned();
            self.yank_flash = None;
            self.request_frame();
            true
        } else {
            false
        }
    }

    /// Signal that the selection moved under keyboard control, so the canvas
    /// should bring the newly-selected node back into frame if it has scrolled
    /// out of (or near the edge of) the visible viewport.
    fn request_frame(&mut self) {
        self.frame_gen = self.frame_gen.wrapping_add(1);
    }

    /// A monotonic counter that changes each time a keyboard move re-selects a
    /// node — the canvas compares it against the last node it framed and scrolls
    /// the selection into view when it differs.
    #[must_use]
    pub fn frame_gen(&self) -> u64 {
        self.frame_gen
    }

    /// `Enter` — drill into the selected node: push the (deduped) breadcrumb and
    /// focus the canvas on that node's full transitive lineage (every ancestor
    /// upstream that feeds it + every descendant downstream it feeds + the node
    /// itself), re-laid-out and re-rastered so the scope change is visible. A
    /// repeated `Enter` on the same node is a no-op.
    fn drill_in(&mut self) -> bool {
        if !self.nav.drill_in() {
            return false;
        }
        if let Some(focus) = self.nav.cursor().cloned() {
            let keep = brightfield_protocol::graph::lineage(&self.graph_collapsed, &focus);
            self.scope_graph = Some(brightfield_protocol::graph::induced_subgraph(
                &self.graph_collapsed,
                &keep,
            ));
            self.selected = Some(focus);
            self.yank_flash = None;
        }
        self.request_frame();
        self.recompute_layout();
        true
    }

    /// `Esc` — close the steps sheet if open, else pop one drill level: re-scope
    /// the canvas to the parent crumb's neighbourhood, or clear the scope back to
    /// the whole graph at the root.
    fn drill_out(&mut self) -> bool {
        if self.show_sheet {
            self.show_sheet = false;
            return true;
        }
        if !self.nav.drill_out() {
            return false;
        }
        match self.nav.breadcrumb().last().cloned() {
            Some(parent) => {
                let keep = brightfield_protocol::graph::lineage(&self.graph_collapsed, &parent);
                self.scope_graph = Some(brightfield_protocol::graph::induced_subgraph(
                    &self.graph_collapsed,
                    &keep,
                ));
                self.selected = Some(parent);
            }
            None => {
                self.scope_graph = None;
                self.selected = self.nav.cursor().cloned();
            }
        }
        self.yank_flash = None;
        self.request_frame();
        self.recompute_layout();
        true
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

    /// Recompute the layout for the current displayed graph (scope / expand) +
    /// flow, and bump the generation so the raster cache invalidates.
    fn recompute_layout(&mut self) {
        let cfg = LayoutConfig {
            flow: self.flow,
            ..LayoutConfig::default()
        };
        let laid = {
            let graph = self.displayed_graph();
            brightfield_protocol::layout(graph, &cfg)
        };
        self.layout = laid;
        self.layout_key = (self.display_expanded, self.flow);
        self.layout_gen = self.layout_gen.wrapping_add(1);
    }

    /// The (expanded, flow) view state — the fold + flow half of the cache key.
    #[must_use]
    fn layout_key(&self) -> (bool, Flow) {
        self.layout_key
    }

    /// A monotonic counter that changes on every re-layout — the drill/scope
    /// half of the raster cache key (an expand/flow flip alone is not enough to
    /// tell a scope change apart).
    #[must_use]
    fn layout_gen(&self) -> u64 {
        self.layout_gen
    }
}

// ---------------------------------------------------------------------------
// ProtocolDoc — the state every pane in this view shares.
// ---------------------------------------------------------------------------

/// The protocol view's **document**: the interaction model, and the canvas the
/// DAG rasters into.
///
/// Every pane in the view reads this and two of them write it — clicking a node
/// on the canvas selects it, and so does clicking an outline row. No
/// [`Item`] holds a handle to it: the shell hands out exactly one `&mut
/// ProtocolDoc`, for the duration of one pane's draw. That is the aliasing rule
/// the whole contract hangs off, and it is why the canvas host lives *here*
/// rather than inside the canvas pane.
pub struct ProtocolDoc {
    /// The interaction model — GPU-free, unit-tested, and the only writer of
    /// selection, fold and drill state.
    pub model: ProtocolModel,
    /// The DAG raster: the host, the presented texture, and the key it was
    /// presented at.
    pub canvas: CanvasSlot<CanvasKey>,
}

/// Everything the DAG raster's pixels depend on.
///
/// The generation catches a drill/scope re-layout an (expanded, flow) pair
/// alone would miss; the device size catches a resize or a scale change; and
/// `dark` catches a theme switch, which since this increment changes most of
/// the raster's colours rather than only the page tone behind them — a few
/// solids (the issue badge, its glyph, the status tints) are paints and stay
/// put in both modes. Held as a named struct rather than an anonymous tuple
/// because `differs_only_by_mode` is what a test can hold the mode component
/// to — a bare tuple can lose a
/// field to a refactor and stay compiling and green.
///
/// **Deliberately not shared with the chart view's key of the same name**,
/// though [`CanvasSlot`] itself now is. A composed dashboard is composed once
/// before the window opens and never re-laid-out, so three of these six fields
/// would be constants over there — and a cache-key field nobody ever changes is
/// a cache that silently never invalidates. The same note is on the chart side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CanvasKey {
    expanded: bool,
    flow: Flow,
    generation: u64,
    dev_width: u32,
    dev_height: u32,
    dark: bool,
}

impl CanvasKey {
    /// Whether `self` and `other` differ in the mode component and nothing
    /// else — the shape a theme switch alone produces.
    #[cfg(test)]
    fn differs_only_by_mode(self, other: Self) -> bool {
        self.dark != other.dark
            && Self {
                dark: other.dark,
                ..self
            } == other
    }
}

impl ProtocolDoc {
    /// A document over `model`, rastering through `host`.
    #[must_use]
    pub fn new(model: ProtocolModel, host: EguiCanvasHost) -> Self {
        Self {
            model,
            canvas: CanvasSlot::new(host),
        }
    }

    /// A document with no device behind it — the [`CanvasSlot`] holds no host.

    #[must_use]
    pub fn headless(model: ProtocolModel) -> Self {
        Self {
            model,
            canvas: CanvasSlot::headless(),
        }
    }

    /// An empty document: no assets, no seams, no steps, no device.
    ///
    /// The value [`protocol_registry`]'s audit runs against.
    #[must_use]
    pub fn empty() -> Self {
        Self::headless(ProtocolModel::new(ProtocolInputs::empty(), Flow::Vertical))
    }

    /// The identity of the raster this document would present right now.
    fn canvas_key(&self, ppp: f32, mode: Mode) -> (CanvasKey, PixelSize) {
        let (expanded, flow) = self.model.layout_key();
        let l = self.model.layout();
        let dev = PixelSize {
            width: ((l.width as f32) * ppp).round().max(1.0) as u32,
            height: ((l.height as f32) * ppp).round().max(1.0) as u32,
        };
        (
            CanvasKey {
                expanded,
                flow,
                generation: self.model.layout_gen(),
                dev_width: dev.width,
                dev_height: dev.height,
                dark: mode.is_dark(),
            },
            dev,
        )
    }

    /// Re-raster the DAG through the host only when [`CanvasKey`] changed, and
    /// hand the canvas pane the texture to paint.
    fn ensure_presented(&mut self, ppp: f32, mode: Mode) {
        let (key, dev) = self.canvas_key(ppp, mode);
        if self.canvas.presented(&key) {
            return;
        }
        let Some(host) = self.canvas.host_mut() else {
            return;
        };
        // The raster's own page tone. The asset scene paints its own canvas
        // rectangle over this, so it is only ever seen through the scene's
        // antialiased edges — but it is a token either way, and resolved for
        // the mode like every other colour this file paints. It has to be in
        // the presented key for that to mean anything: a raster held over a
        // mode switch would keep the tone it was baked at.
        let base = Color::from_token(semantic(mode.is_dark()).surfaces.raised);
        // Build the scene under an immutable borrow, then present (mutable host).
        let scene = {
            let mut s = vello::Scene::new();
            brightfield_render::asset_scene::render_asset_graph_with_status(
                &mut s,
                self.model.layout(),
                self.model.displayed_graph(),
                &self.model.statuses,
                mode.is_dark(),
            );
            let mut scaled = vello::Scene::new();
            scaled.append(&s, Some(kurbo::Affine::scale(f64::from(ppp))));
            scaled
        };
        let id = host.present_keyed(CANVAS_PANE, &scene, dev, base);
        self.canvas.record(key, id);
    }
}

// ---------------------------------------------------------------------------
// The registry: the one declaration of this view's shape.
// ---------------------------------------------------------------------------

/// The DAG canvas — the view's centre pane.
pub const CANVAS: ItemId = ItemId::new("protocol-canvas");
/// The topological outline rail.
pub const OUTLINE: ItemId = ItemId::new("protocol-outline");
/// The selection inspector rail.
pub const INSPECTOR: ItemId = ItemId::new("protocol-inspector");
/// The flat run-ordered steps sheet, a tab beside the canvas.
pub const STEPS: ItemId = ItemId::new("protocol-steps");

/// Add this view's item ids to the process's layout vocabulary.
///
/// Called at boot from [`ProtocolShell::new`], before any layout file could be
/// read. Idempotent, so a test binary that builds two shells neither falls over
/// nor grows the vocabulary — which is why it is safe to expose as its own
/// entry point for a test that needs the vocabulary without needing a device.
///
/// The ids come from [`protocol_registry`] and nowhere else. A hand-written
/// `static [ItemId; 4]` used to stand here: a second declaration of the view's
/// shape that a fifth pane could be added to the registry without.
pub fn publish_item_ids() {
    protocol_registry().publish_ids();
}

/// The canvas pane's address — the key its Vello texture slot is filed under.
const CANVAS_PANE: PaneKey = PaneKey::new(ViewKind::Protocol, CANVAS);

/// The outline rail's share of the window. Declared once and read twice: the
/// registry lays the dock out with it, and [`ProtocolShell::window_size`] sizes
/// the window from it. The pair of pixel constants this replaces said 260px and
/// 300px while the tiles said 24% and 22%, and the two had drifted.
const OUTLINE_SHARE: f32 = 0.24;
/// The inspector rail's share of the window.
const INSPECTOR_SHARE: f32 = 0.22;

/// Every icon here is a *name*, resolved to paint at draw time. The Meridian
/// icon set has not landed in this workspace, so the chrome reserves each
/// glyph's box without painting into it.
const ICON_CANVAS: Icon = Icon("asset-graph");
const ICON_OUTLINE: Icon = Icon("list-tree");
const ICON_INSPECTOR: Icon = Icon("info-panel");
const ICON_STEPS: Icon = Icon("list-ordered");

/// The protocol view's registry: four panes, where each sits, and the verb that
/// shows and hides it.
///
/// This is the **only** declaration of the view's shape. The dock's default
/// arrangement ([`ItemRegistry::default_tree`]), the live item map
/// ([`ItemRegistry::instantiate`]) and the published id vocabulary
/// ([`ItemRegistry::publish_ids`], via [`publish_item_ids`]) are all derived
/// from this list, so a pane cannot be added to one and forgotten in another.
#[must_use]
pub fn protocol_registry() -> ItemRegistry<ProtocolDoc> {
    ItemRegistry::new(
        ViewKind::Protocol,
        vec![
            ItemSpec {
                id: OUTLINE,
                slot: Slot::Rail {
                    side: DockSide::Left,
                    share: OUTLINE_SHARE,
                },
                toggle: Some(Verb::new("toggle-outline-rail")),
                make: || Box::new(OutlinePane),
            },
            ItemSpec {
                id: CANVAS,
                slot: Slot::Centre,
                toggle: None,
                make: || Box::new(CanvasPane),
            },
            ItemSpec {
                id: STEPS,
                slot: Slot::CentreTab,
                toggle: Some(Verb::new("open-steps-sheet")),
                make: || Box::new(StepsPane),
            },
            ItemSpec {
                id: INSPECTOR,
                slot: Slot::Rail {
                    side: DockSide::Right,
                    share: INSPECTOR_SHARE,
                },
                toggle: Some(Verb::new("toggle-inspector-rail")),
                make: || Box::new(InspectorPane),
            },
        ],
    )
}

// ---------------------------------------------------------------------------
// The four panes.
// ---------------------------------------------------------------------------

/// The topological outline rail: one row per asset, in run order.
///
/// A unit struct because it has no view-local state at all — its scroll
/// position is egui's, and everything it draws is a pure function of the
/// document.
struct OutlinePane;

impl Item<ProtocolDoc> for OutlinePane {
    fn item_id(&self) -> ItemId {
        OUTLINE
    }

    fn subject(&self, doc: &ProtocolDoc) -> Subject {
        let subject = Subject::new("Outline", ICON_OUTLINE, BindingContext::Protocol);
        if doc.model.has_assets() {
            subject
        } else {
            subject.empty(EmptyState::new(
                ICON_OUTLINE,
                "No assets yet",
                "This protocol declares no assets, so there is nothing to list. \
                 Open a manifest whose steps build at least one table or file.",
            ))
        }
    }

    fn ui(&mut self, doc: &mut ProtocolDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        let rows = doc.model.outline();
        let mut clicked: Option<AssetId> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in &rows {
                    if outline_row(ui, row, cx.mode).clicked() {
                        clicked = Some(row.id.clone());
                    }
                }
            });
        if let Some(id) = clicked {
            doc.model.select_id(id);
        }
    }
}

/// One outline row: status dot, label, kind — and, when it is the selection,
/// the one selection wash.
///
/// The row rect is allocated *before* anything is painted into it, which is
/// what lets the wash sit under the content rather than beside it. The version
/// this replaces used `Ui::selectable_label`, whose wash is the framework's,
/// and then swapped the label ink on top of it — two signals for one state, one
/// of them not from the token layer at all.
fn outline_row(ui: &mut egui::Ui, row: &OutlineRow, mode: Mode) -> egui::Response {
    let sem = semantic(mode.is_dark());
    let b = control::binding(spacing::ROW_DENSE);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), b.row),
        egui::Sense::click(),
    );
    if row.selected {
        chrome::selection_wash(ui, rect, mode);
    }

    let painter = ui.painter();
    let dot = b.icon / 4.0;
    let mut x = rect.left() + b.pad_x;
    painter.circle_filled(
        egui::pos2(x + dot, rect.center().y),
        dot,
        status_colour(row.status, mode),
    );
    x += b.icon + spacing::ICON_LABEL_GAP;
    painter.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        truncate(&row.label, 26),
        ui_font(),
        chrome::colour(sem.text.primary),
    );
    painter.text(
        egui::pos2(rect.right() - b.pad_x, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        kind_label(row.kind),
        ui_font(),
        chrome::colour(sem.text.muted),
    );
    response
}

/// The DAG canvas: the presented Vello raster in a scroll area, with the
/// keyboard cursor ringed and click → select hit-testing.
struct CanvasPane;

impl Item<ProtocolDoc> for CanvasPane {
    fn item_id(&self) -> ItemId {
        CANVAS
    }

    fn subject(&self, doc: &ProtocolDoc) -> Subject {
        let subject = Subject::new("Canvas", ICON_CANVAS, BindingContext::Protocol);
        if doc.model.displayed_graph().nodes.is_empty() {
            subject.empty(EmptyState::new(
                ICON_CANVAS,
                "Nothing to draw",
                "The graph in view holds no assets. Widen the drill scope, or open \
                 a protocol whose steps produce something.",
            ))
        } else {
            subject
        }
    }

    fn ui(&mut self, doc: &mut ProtocolDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        let Some(texture) = doc.canvas.texture() else {
            // No device behind this document. The pane is blank rather than
            // apologetic: a headless document is a test fixture, never a state
            // a user reaches, so a message here would be chrome nobody sees.
            return;
        };
        let (w, h) = {
            let l = doc.model.layout();
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

                // The keyboard cursor, ringed with the token layer's focus ring
                // — one treatment, at the ring's own width, offset and radius.
                // The hand-rolled 2px stroke at a 4px radius it replaces matched
                // nothing else in the product.
                if let Some(sel) = doc.model.selected().cloned() {
                    if let Some(node) = doc.model.layout().positions.get(&sel).cloned() {
                        let r = node_rect(rect.min, &node);
                        chrome::focus_ring(ui, r, cx.mode);
                        frame_selection(ui, doc, cx, r);
                    }
                }

                // Click → select: hit-test canvas-local coords against the layout.
                if resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        let lx = f64::from(p.x - rect.min.x);
                        let ly = f64::from(p.y - rect.min.y);
                        if let Some(id) = hit_test(doc.model.layout(), lx, ly) {
                            doc.model.select_id(id);
                        }
                    }
                }
            });
    }
}

/// Frame follows selection: after a keyboard move, if the selected node has
/// scrolled out of the viewport (or is within a small margin of an edge), pan it
/// back into frame. A node that is already comfortably visible is left alone, so
/// manual scrolling is never fought.
fn frame_selection(ui: &mut egui::Ui, doc: &ProtocolDoc, cx: &ItemCtx<'_>, node: egui::Rect) {
    let frame_gen = doc.model.frame_gen();
    let framed_id = egui::Id::new(("proto-canvas-framed-gen", cx.tile));
    let last_framed = ui.ctx().data(|d| d.get_temp::<u64>(framed_id)).unwrap_or(0);
    if frame_gen == last_framed {
        return;
    }
    let margin = spacing::SPACE_7;
    let vis = ui.clip_rect();
    let out_of_frame = node.min.x < vis.min.x + margin
        || node.max.x > vis.max.x - margin
        || node.min.y < vis.min.y + margin
        || node.max.y > vis.max.y - margin;
    if out_of_frame {
        ui.scroll_to_rect(node.expand(margin), None);
    }
    ui.ctx().data_mut(|d| d.insert_temp(framed_id, frame_gen));
}

/// The inspector rail: the selected asset's detail, each field with a
/// plain-language explainer.
struct InspectorPane;

impl Item<ProtocolDoc> for InspectorPane {
    fn item_id(&self) -> ItemId {
        INSPECTOR
    }

    fn subject(&self, doc: &ProtocolDoc) -> Subject {
        let subject = Subject::new("Inspector", ICON_INSPECTOR, BindingContext::Protocol);
        if doc.model.has_selection() {
            subject
        } else {
            subject.empty(EmptyState::new(
                ICON_INSPECTOR,
                "Nothing selected",
                "Click a node in the canvas or a row in the outline, or move the \
                 cursor with h j k l.",
            ))
        }
    }

    fn ui(&mut self, doc: &mut ProtocolDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        let facts = doc.model.inspector();
        let mode = cx.mode;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| inspector_body(ui, &facts, mode));
    }
}

/// Render the selected asset's fields with explainers.
///
/// The asset's label used to be a `heading()` here — a second type size inside
/// a pane whose header band is already its name. It is the pane's content now,
/// at the one UI size, in primary ink.
fn inspector_body(ui: &mut egui::Ui, facts: &InspectorFacts, mode: Mode) {
    let sem = semantic(mode.is_dark());
    ui.label(
        egui::RichText::new(&facts.label)
            .font(ui_font())
            .color(chrome::colour(sem.text.primary)),
    );
    ui.label(
        egui::RichText::new(kind_gloss(facts.kind))
            .font(ui_font())
            .color(chrome::colour(sem.text.secondary)),
    );

    field(
        ui,
        mode,
        "Address",
        &facts.address,
        "Stable dotted id for this asset — press y to copy it.",
        true,
    );

    match &facts.producing_step {
        Some(step) => field(
            ui,
            mode,
            "Produced by",
            step,
            "The step / operator that builds this asset.",
            false,
        ),
        None => field(
            ui,
            mode,
            "Produced by",
            "external input",
            "Fetched from outside the build — nothing in the protocol produces it.",
            false,
        ),
    }

    if let Some(t) = &facts.transform {
        field(
            ui,
            mode,
            "Transform",
            t,
            "How that step derives the asset (operator or SQL model).",
            false,
        );
    }

    field(
        ui,
        mode,
        "Status",
        status_word(facts.status),
        status_gloss(facts.status),
        false,
    );

    // Measured values exist only after a real run; the offline manifest carries
    // lineage only, so say that rather than show blank rows. This is a *field*
    // that is not yet measured, not a pane with nothing in it — the pane's
    // empty state is the `Subject`'s, and it says something else entirely.
    let measured =
        facts.row_count.is_some() || facts.bytes.is_some() || facts.materialized.is_some();
    if measured {
        if let Some(rc) = facts.row_count {
            field(
                ui,
                mode,
                "Rows",
                &rc.to_string(),
                "Row count measured on the last run.",
                false,
            );
        }
        if let Some(b) = facts.bytes {
            field(
                ui,
                mode,
                "Size",
                &human_bytes(b),
                "Bytes written on the last run.",
                false,
            );
        }
        if let Some(m) = facts.materialized {
            field(
                ui,
                mode,
                "Materialized",
                if m { "yes" } else { "no" },
                "Whether the run actually wrote this asset.",
                false,
            );
        }
    } else {
        ui.add_space(spacing::SPACE_4);
        ui.label(
            egui::RichText::new(
                "No measured values yet — this is the offline manifest, which carries lineage \
                 only. Row count, size, and content hash appear once the protocol is run.",
            )
            .font(ui_font())
            .color(chrome::colour(sem.text.muted))
            .italics(),
        );
    }

    if let Some(issue) = &facts.issue {
        field(
            ui,
            mode,
            "Issue",
            issue,
            "Why this step is degraded / opaque.",
            false,
        );
    }
}

/// One inspector field: a muted caption, the value, and a one-line explainer.
///
/// `mono` picks the face the *value* is set in. It is true for a value the
/// reader compares character by character — the address — and false for prose.
/// See [`mono_font`] for why it selects a `FontId` rather than chaining
/// `RichText::monospace`.
fn field(ui: &mut egui::Ui, mode: Mode, label: &str, value: &str, explain: &str, mono: bool) {
    let sem = semantic(mode.is_dark());
    ui.add_space(spacing::SPACE_4);
    ui.label(
        egui::RichText::new(label.to_uppercase())
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
    ui.label(
        egui::RichText::new(value)
            .font(if mono { mono_font() } else { ui_font() })
            .color(chrome::colour(sem.text.primary)),
    );
    ui.label(
        egui::RichText::new(explain)
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
}

/// The S steps sheet: the flat run-ordered step list as a grid.
struct StepsPane;

impl Item<ProtocolDoc> for StepsPane {
    fn item_id(&self) -> ItemId {
        STEPS
    }

    fn subject(&self, doc: &ProtocolDoc) -> Subject {
        let subject = Subject::new("Steps", ICON_STEPS, BindingContext::Protocol);
        if doc.model.sheet().is_empty() {
            subject.empty(EmptyState::new(
                ICON_STEPS,
                "No steps yet",
                "This protocol runs nothing. A manifest with op: or sql: steps \
                 fills this list in run order.",
            ))
        } else {
            subject
        }
    }

    fn ui(&mut self, doc: &mut ProtocolDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        let sem = semantic(cx.mode.is_dark());
        let cursor = doc.model.sheet().cursor();
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let grid = egui::Grid::new("proto-steps-grid")
                    .striped(true)
                    .num_columns(6)
                    .spacing(egui::vec2(spacing::SPACE_6, spacing::SPACE_2))
                    .show(ui, |ui| {
                        for h in brightfield_protocol::sheet::COLUMNS {
                            ui.label(
                                egui::RichText::new(h)
                                    .font(ui_font())
                                    .color(chrome::colour(sem.text.muted)),
                            );
                        }
                        ui.end_row();
                        let mut wash = None;
                        for (i, row) in doc.model.sheet().rows().iter().enumerate() {
                            if let Some(w) = steps_row(ui, row, i == cursor, cx.mode) {
                                wash = Some(w);
                            }
                        }
                        wash
                    });
                // The wash is filled in *after* the grid closes, because that
                // is the first moment the row's full width is known. Sizing it
                // from the union of the row's own cells fell ~40px short of the
                // zebra stripe beside it: the stripe spans the grid's full
                // width, while the last column is empty on an offline row and
                // contributes a zero-width rect. So the x range comes from the
                // grid and only the y range from the row.
                if let Some((idx, rows)) = grid.inner {
                    let rect = egui::Rect::from_x_y_ranges(grid.response.rect.x_range(), rows)
                        .expand(spacing::SPACE_1);
                    ui.painter()
                        .set(idx, chrome::selection_wash_shape(rect, cx.mode));
                }
            });
    }
}

/// One steps row, with the cursor row wearing the one selection wash.
///
/// Returns the reserved wash slot and the row's vertical extent when this is
/// the cursor row, for the caller to fill in once the grid's width is known.
/// The wash has to be *reserved* before the cells are laid out, because a grid
/// row's rect is not known until its widest cell has been measured. That is
/// what [`chrome::selection_wash_shape`] exists for. The version this replaces
/// marked the cursor with a ▸ prefix and an ink swap and no wash at all — a
/// third spelling of "this row is selected".
fn steps_row(
    ui: &mut egui::Ui,
    row: &StepRow,
    selected: bool,
    mode: Mode,
) -> Option<(egui::layers::ShapeIdx, egui::Rangef)> {
    let sem = semantic(mode.is_dark());
    let wash = selected.then(|| ui.painter().add(egui::Shape::Noop));
    let ink = chrome::colour(sem.text.primary);
    let quiet = chrome::colour(sem.text.secondary);

    let name = if row.gate {
        format!("{} ◈", row.name)
    } else {
        row.name.clone()
    };
    let first = ui.label(
        egui::RichText::new(row.order.to_string())
            .font(mono_font())
            .color(ink),
    );
    ui.label(egui::RichText::new(name).font(ui_font()).color(ink));
    ui.label(egui::RichText::new(row.kind).font(ui_font()).color(quiet));
    ui.label(
        egui::RichText::new(truncate(&row.detail, 40))
            .font(ui_font())
            .color(quiet),
    );
    ui.label(egui::RichText::new(row.status).font(ui_font()).color(quiet));
    let live = row
        .live_state
        .clone()
        .or_else(|| row.skip_reason.clone())
        .unwrap_or_default();
    let last = ui.label(
        egui::RichText::new(live)
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
    ui.end_row();

    wash.map(|idx| (idx, first.rect.union(last.rect).y_range()))
}

// ---------------------------------------------------------------------------
// Shared pane helpers.
// ---------------------------------------------------------------------------

/// The one UI type size. Chrome has no headings — see
/// [`brightfield_workbench::chrome`].
fn ui_font() -> egui::FontId {
    egui::FontId::proportional(meridian_design::typography::UI_SIZE)
}

/// The same size in the Meridian mono face, for a value the reader compares
/// character by character: an address, a run ordinal, the key hints.
///
/// It has to be a [`egui::FontId`] rather than `RichText::monospace`, and that
/// is the whole reason this exists. `RichText::font` sets *size and family*
/// together, while `RichText::monospace` only sets the text style — so
/// `.font(ui_font()).monospace()` resolves the style first and then overwrites
/// its family with `ui_font`'s proportional one. The `.monospace()` is inert
/// and the value silently renders proportional. Three call sites in this file
/// were written that way and lost their mono face without a compiler word.
///
/// `FontFamily::Monospace` is the family [`crate::design::install_fonts`] maps
/// to the design system's [`MONO_FAMILY`](meridian_design::typography::MONO_FAMILY),
/// so this reaches the token layer's face rather than egui's fallback.
fn mono_font() -> egui::FontId {
    egui::FontId::monospace(meridian_design::typography::UI_SIZE)
}

/// A seam status as ink.
///
/// The three real statuses take the reserved Meridian status inks through the
/// [`Tone`] vocabulary, so the panel cannot reach past them for an accent.
/// Skipped and not-run are not statuses so much as their absence, and they take
/// quiet ink from the semantic layer: two raw gray rungs used to stand here,
/// which meant a dark-mode panel drew light-mode grays.
fn status_colour(s: SeamStatus, mode: Mode) -> egui::Color32 {
    let sem = semantic(mode.is_dark());
    match s {
        SeamStatus::Ok => chrome::tone_colour(Tone::Good, mode),
        SeamStatus::Running => chrome::tone_colour(Tone::Warning, mode),
        SeamStatus::Failed => chrome::tone_colour(Tone::Critical, mode),
        SeamStatus::Skipped => chrome::colour(sem.text.disabled),
        SeamStatus::NotRun => chrome::colour(sem.text.placeholder),
    }
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

/// A plain-language gloss of an [`AssetKind`] for the inspector subtitle.
fn kind_gloss(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Source => "source · an external origin fetched into the build",
        AssetKind::File => "file · a file on disk a step writes or reads",
        AssetKind::Table => "table · a durable relation other steps read",
        AssetKind::Internal => "internal · a statement intermediate inside one model",
        AssetKind::Dataset => "dataset · the published output artefact (the sink)",
        AssetKind::Family => "family · a collapsed group of parameterised steps",
        AssetKind::Opaque => "opaque · a degraded or unreadable step",
    }
}

/// A one-line explanation of a run status for the inspector.
fn status_gloss(s: SeamStatus) -> &'static str {
    match s {
        SeamStatus::Ok => "Ran successfully on the last run.",
        SeamStatus::Running => "Currently running.",
        SeamStatus::Skipped => "Skipped — already up to date, or gated off.",
        SeamStatus::Failed => "Failed on the last run — check the run log.",
        SeamStatus::NotRun => "Not run in this view (the offline manifest has no run status).",
    }
}

/// Human-readable byte size (1 KiB steps), for the inspector's measured Size.
fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if b < 1024 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// ProtocolShell — the document, the dock, and the two bars it still owns.
// ---------------------------------------------------------------------------

/// The full egui Protocol panel: the [`ProtocolDoc`], the four live items, and
/// the dock tree the registry laid out. [`ProtocolShell::draw`] is the single
/// frame source (live window, headless shot, snapshot) — the loop's guarantee.
///
/// It still owns a window, a top breadcrumb bar and a bottom key-hint bar, all
/// three of which the one-app shell takes over later. What it no longer owns is
/// any pane's chrome: every pane header, empty state, selection wash and focus
/// ring on this surface is drawn by [`PaneChrome`] from a [`Subject`].
pub struct ProtocolShell {
    doc: ProtocolDoc,
    items: ItemMap<ProtocolDoc>,
    dock: Tree<PaneKey>,
    /// The focused pane. Tracked because [`PaneChrome`] reports focus moves and
    /// hands each pane its own focus state; nothing paints it yet — the pane
    /// focus ring lands with the one-app shell, which is also where a focused
    /// pane starts to *mean* something (its subject becomes the window chrome).
    focus: Option<PaneKey>,
    mode: Mode,
    fonts_installed: bool,
}

impl ProtocolShell {
    /// Build the shell over `inputs` and an [`EguiCanvasHost`] on the shell's
    /// shared device, with the initial `flow` (vertical by default).
    #[must_use]
    pub fn new(inputs: ProtocolInputs, host: EguiCanvasHost, mode: Mode, flow: Flow) -> Self {
        publish_item_ids();
        let registry = protocol_registry();
        Self {
            doc: ProtocolDoc::new(ProtocolModel::new(inputs, flow), host),
            items: registry.instantiate(),
            dock: registry.default_tree(),
            focus: None,
            mode,
            fonts_installed: false,
        }
    }

    /// The panel's natural window size in logical points: wide enough that the
    /// canvas pane's *share* of it fits the DAG, plus the top and bottom bars.
    ///
    /// Derived from the rail shares rather than from a second pair of pixel
    /// constants. The constants said the rails were 560px wide while the tiles
    /// gave them 46% of the window, so on the fixture the canvas pane was ~9%
    /// narrower than the DAG it had to show and the graph opened part-scrolled.
    #[must_use]
    pub fn window_size(&self) -> (f32, f32) {
        let l = self.doc.model.layout();
        let centre = 1.0 - OUTLINE_SHARE - INSPECTOR_SHARE;
        let w = (l.width as f32 / centre + spacing::SPACE_9).max(1100.0);
        let h = (l.height as f32 + 130.0).clamp(680.0, 1600.0);
        (w, h)
    }

    /// The window/tab title.
    #[must_use]
    pub fn title(&self) -> String {
        format!("Protocol · {}", self.doc.model.protocol)
    }

    /// Mutable access to the model (for the shot's `--focus` seed).
    pub fn model_mut(&mut self) -> &mut ProtocolModel {
        &mut self.doc.model
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
        self.doc.model.feed_events(&events);
        if let Some(addr) = self.doc.model.take_yank_request() {
            ctx.copy_text(addr);
        }

        self.doc.ensure_presented(ctx.pixels_per_point(), self.mode);
        self.set_active_tab();

        // Orientation chrome: breadcrumb (top) + key-hint bar (bottom). Still
        // the shell's own, still not a `Subject` — see the module docs.
        egui::containers::Panel::top("proto-breadcrumb")
            .resizable(false)
            .show(ui, |ui| self.breadcrumb_ui(ui));
        egui::containers::Panel::bottom("proto-hint")
            .resizable(false)
            .show(ui, |ui| hint_ui(ui, &self.doc.model, self.mode));

        // The dock fills the rest. Every pane's chrome comes from its subject,
        // through the one `egui_tiles::Behavior` in the product.
        let tabbed = tabbed_tiles_of(&self.dock);
        let mut requests: Vec<Request> = Vec::new();
        egui::containers::CentralPanel::default().show(ui, |ui| {
            let mut behavior = PaneChrome::new(
                &mut self.doc,
                &mut self.items,
                self.mode,
                self.focus,
                &tabbed,
                &mut requests,
            );
            self.dock.ui(&mut behavior, ui);
        });
        self.apply(&ctx, requests);

        // Read a manual Canvas/Steps tab click back into the model.
        self.read_active_tab();
        self.sweep_canvas();
    }

    /// Perform the requests the frame's panes raised, now that the tile tree's
    /// borrow is over.
    ///
    /// A verb is dispatched to the model, which is where a keystroke's verb
    /// lands too — so a control and its keystroke cannot become two
    /// implementations of one command.
    fn apply(&mut self, ctx: &egui::Context, requests: Vec<Request>) {
        for request in requests {
            match request {
                Request::Verb(verb) => {
                    self.doc.model.dispatch(verb.as_str());
                }
                Request::Focus(key) => self.focus = Some(key),
                Request::Repaint => ctx.request_repaint(),
            }
        }
    }

    /// Before rendering: make the active Canvas/Steps tab authoritative from the
    /// model's `show_sheet` (so the `S`/`Esc` keys drive it).
    fn set_active_tab(&mut self) {
        let Some(canvas) = tile_of(&self.dock, CANVAS_PANE) else {
            return;
        };
        let Some(steps) = tile_of(&self.dock, PaneKey::new(ViewKind::Protocol, STEPS)) else {
            return;
        };
        let want = if self.doc.model.show_sheet {
            steps
        } else {
            canvas
        };
        let Some(tabs_id) = tabs_holding(&self.dock, canvas) else {
            return;
        };
        if let Some(Tile::Container(Container::Tabs(tabs))) = self.dock.tiles.get_mut(tabs_id) {
            tabs.set_active(want);
        }
    }

    /// After rendering: read a manual tab click back into `show_sheet` (so a
    /// pointer click on the Steps tab also opens the sheet, and Canvas closes it).
    fn read_active_tab(&mut self) {
        let Some(steps) = tile_of(&self.dock, PaneKey::new(ViewKind::Protocol, STEPS)) else {
            return;
        };
        let Some(tabs_id) = tabs_holding(&self.dock, steps) else {
            return;
        };
        if let Some(Tile::Container(Container::Tabs(tabs))) = self.dock.tiles.get(tabs_id) {
            if let Some(active) = tabs.active {
                self.doc.model.show_sheet = active == steps;
            }
        }
    }

    /// Declare which panes this frame laid out, so the host can free the canvas
    /// slot of any pane that has gone.
    ///
    /// Every pane in the tree is declared, including a canvas that is tabbed out
    /// of sight this frame. That is deliberate and it is the safe direction:
    /// `ensure_presented` caches its texture id across frames and returns early
    /// on an unchanged view, so a slot freed while the canvas was behind the
    /// steps tab would leave a dangling id the moment the user tabbed back.
    fn sweep_canvas(&mut self) {
        let Some(host) = self.doc.canvas.host_mut() else {
            return;
        };
        let visible: BTreeSet<PaneKey> = self
            .dock
            .tiles
            .tiles()
            .filter_map(|tile| match tile {
                Tile::Pane(key) => Some(*key),
                Tile::Container(_) => None,
            })
            .collect();
        host.end_frame(&visible);
    }

    /// The top breadcrumb + a flow toggle (mutates the model, so it lives on the
    /// shell rather than the free `hint_ui`).
    ///
    /// The selection used to be restated here as a `› label` hop — a fifth way
    /// of saying "this is selected", one pane away from the wash that already
    /// says it. The drill crumbs stay: they are scope, not selection.
    fn breadcrumb_ui(&mut self, ui: &mut egui::Ui) {
        let sem = semantic(self.mode.is_dark());
        ui.add_space(spacing::SPACE_2);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Protocol · {}", self.doc.model.protocol))
                    .font(ui_font())
                    .color(chrome::colour(sem.text.primary)),
            );
            for crumb in self.doc.model.breadcrumb() {
                ui.label(
                    egui::RichText::new("»")
                        .font(ui_font())
                        .color(chrome::colour(sem.text.muted)),
                );
                ui.label(
                    egui::RichText::new(crumb)
                        .font(ui_font())
                        .color(chrome::colour(sem.text.secondary)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (word, next) = match self.doc.model.flow() {
                    Flow::Vertical => ("vertical", "horizontal"),
                    Flow::Horizontal => ("horizontal", "vertical"),
                };
                if ui
                    .button(egui::RichText::new(format!("flow: {word} ⇄")).font(ui_font()))
                    .on_hover_text(format!("switch to {next} flow"))
                    .clicked()
                {
                    self.doc.model.toggle_flow();
                    self.doc.canvas.invalidate();
                }
            });
        });
        ui.add_space(spacing::SPACE_2);
    }
}

/// The bottom key-hint bar + a flow/state indicator (read-only — no model
/// mutation, so it is a free function).
fn hint_ui(ui: &mut egui::Ui, model: &ProtocolModel, mode: Mode) {
    let sem = semantic(mode.is_dark());
    ui.add_space(spacing::SPACE_1);
    // The motion keys follow the drawn flow: along it, produce/consume; across
    // it, siblings. Spell out which keys are which for the current axis.
    let hint = if model.show_sheet() {
        // The sheet takes j/k for row motion, so spell out the way back to the canvas.
        "j/k rows   S / Esc / ⌫ back to canvas   y yank"
    } else {
        match model.flow() {
            Flow::Vertical => {
                "k/j producer·consumer   h/l siblings   za fold   t flip   Enter lineage   Esc/⌫ widen   S steps   y yank"
            }
            Flow::Horizontal => {
                "h/l producer·consumer   j/k siblings   za fold   t flip   Enter lineage   Esc/⌫ widen   S steps   y yank"
            }
        }
    };
    ui.horizontal(|ui| {
        // Small and proportional — what this bar has always rendered. `main`
        // wrote `.monospace().small()`, but `.small()` came last and both call
        // `text_style()`, which overwrites unconditionally, so Small won and the
        // mono never took effect. Making it mono would be a new design decision
        // rather than a restoration, so it keeps the face it has today.
        ui.label(
            egui::RichText::new(hint)
                .small()
                .color(chrome::colour(sem.text.muted)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(addr) = model.yank_flash() {
                ui.label(
                    egui::RichText::new(format!("yanked {addr}"))
                        .font(ui_font())
                        .color(chrome::tone_colour(Tone::Accent, mode)),
                );
            } else if model.is_drilled() {
                ui.label(
                    egui::RichText::new("focused — Esc to widen")
                        .font(ui_font())
                        .color(chrome::tone_colour(Tone::Accent, mode)),
                );
            } else {
                let state = if model.is_expanded() {
                    "family: expanded"
                } else {
                    "family: collapsed"
                };
                ui.label(
                    egui::RichText::new(state)
                        .font(ui_font())
                        .color(chrome::colour(sem.text.muted)),
                );
            }
        });
    });
    ui.add_space(spacing::SPACE_1);
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

    /// A theme switch invalidates the cached DAG raster, and does so through
    /// the key rather than by luck.
    ///
    /// The raster is presented once and re-presented from the slot on every
    /// later frame, so a key blind to the mode leaves a light-ink DAG on the
    /// screen after a switch to dark — the same bug, one frame later. The
    /// assertion is deliberately two-sided: the two keys differ, and they
    /// differ in the mode component *only*, so it cannot be satisfied by a key
    /// that happens to churn for an unrelated reason.
    #[test]
    fn a_theme_switch_changes_the_canvas_key_and_nothing_else() {
        let doc = ProtocolDoc::headless(model());
        let (light, light_dev) = doc.canvas_key(2.0, Mode::Light);
        let (dark, dark_dev) = doc.canvas_key(2.0, Mode::Dark);
        assert_ne!(light, dark, "a theme switch left the raster key unchanged");
        assert!(
            light.differs_only_by_mode(dark),
            "the two keys differ by more than the mode: {light:?} vs {dark:?}"
        );
        assert_eq!(
            light_dev, dark_dev,
            "the mode does not change the raster size"
        );
        // The other components still bite — a key that only ever varied by mode
        // would pass the assertions above and cache a stale layout instead.
        let (finer, _) = doc.canvas_key(3.0, Mode::Light);
        assert_ne!(light, finer, "a scale change must re-raster too");
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
            modifiers: egui::Modifiers {
                shift: true,
                ..Default::default()
            },
        }
    }

    /// The centre of a node's rendered rect in the model's current layout — the
    /// on-screen geometry a spatial move is asserted against.
    fn centre(m: &ProtocolModel, id: &AssetId) -> (f64, f64) {
        let r = &m.layout().positions[id];
        (r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    /// An edgar_gleif model at a chosen reading flow.
    fn model_flow(flow: Flow) -> ProtocolModel {
        ProtocolModel::new(
            load_protocol_offline(EDGAR).expect("load edgar_gleif"),
            flow,
        )
    }

    /// Vertical flow, from the default selection: `j`/`k` run the
    /// producer/consumer axis and `h`/`l` step siblings — and each lands on a
    /// node whose RENDERED CENTRE is genuinely in the pressed screen direction,
    /// not merely a topological neighbour. This is the spatial-nav behaviour,
    /// asserted against the drawn geometry: `j` goes strictly
    /// below, `k` is a wall at the top row (or strictly above), `l`/`h` step to a
    /// same-row sibling strictly right/left.
    #[test]
    fn vertical_keys_move_in_the_pressed_screen_direction() {
        let mut m = model_flow(Flow::Vertical);
        assert_eq!(m.flow(), Flow::Vertical);
        let start = m.selected().cloned().expect("a boot selection");
        let (sx, sy) = centre(&m, &start);

        // j = down: the new selection sits strictly BELOW the start (a consumer).
        assert!(m.feed_events(&[key(egui::Key::J)]), "j moved down the flow");
        let down = m.selected().cloned().unwrap();
        assert_ne!(down, start, "the selection advanced");
        assert!(
            centre(&m, &down).1 > sy + 0.5,
            "j landed strictly below: {down}"
        );
        assert_eq!(
            m.outline().iter().filter(|r| r.selected).count(),
            1,
            "one row marked"
        );

        // k = up from the top row: a wall (nothing above), else strictly above.
        let mut m = model_flow(Flow::Vertical);
        if m.feed_events(&[key(egui::Key::K)]) {
            let up = m.selected().cloned().unwrap();
            assert!(
                centre(&m, &up).1 < sy - 0.5,
                "k landed strictly above: {up}"
            );
        }

        // l = right: a same-row sibling strictly to the RIGHT.
        let mut m = model_flow(Flow::Vertical);
        assert!(
            m.feed_events(&[key(egui::Key::L)]),
            "l stepped a sibling right"
        );
        let right = m.selected().cloned().unwrap();
        assert!(
            centre(&m, &right).0 > sx + 0.5,
            "l landed strictly right: {right}"
        );

        // h = left: a same-row sibling strictly to the LEFT.
        let mut m = model_flow(Flow::Vertical);
        assert!(
            m.feed_events(&[key(egui::Key::H)]),
            "h stepped a sibling left"
        );
        let left = m.selected().cloned().unwrap();
        assert!(
            centre(&m, &left).0 < sx - 0.5,
            "h landed strictly left: {left}"
        );
    }

    /// Horizontal flow rotates the axes: `l`/`h` become producer/consumer and
    /// `j`/`k` step siblings. The SAME default node, the keys following the drawn
    /// left→right flow — again asserted against the rendered centres: `l` lands
    /// strictly right (a consumer), `j` strictly below (a sibling).
    #[test]
    fn horizontal_keys_rotate_with_the_flow() {
        let mut m = model_flow(Flow::Horizontal);
        assert_eq!(m.flow(), Flow::Horizontal);
        let start = m.selected().cloned().expect("a boot selection");
        let (sx, sy) = centre(&m, &start);

        // l = right: a consumer down the flow, strictly to the RIGHT.
        assert!(
            m.feed_events(&[key(egui::Key::L)]),
            "l consumed down the flow"
        );
        let right = m.selected().cloned().unwrap();
        assert!(
            centre(&m, &right).0 > sx + 0.5,
            "l landed strictly right: {right}"
        );

        // j = down: a sibling across the flow, strictly BELOW.
        let mut m = model_flow(Flow::Horizontal);
        assert!(
            m.feed_events(&[key(egui::Key::J)]),
            "j stepped a sibling down"
        );
        let down = m.selected().cloned().unwrap();
        assert!(
            centre(&m, &down).1 > sy + 0.5,
            "j landed strictly below: {down}"
        );
    }

    /// `S` opens the steps sheet; `Esc` closes it.
    #[test]
    fn shift_s_opens_sheet_esc_closes() {
        let mut m = model();
        assert!(!m.show_sheet());
        m.feed_events(&[key_shift(egui::Key::S)]);
        assert!(m.show_sheet(), "S opened the steps sheet");
        // S again toggles it closed (previously a no-op — the way back was mouse-only).
        m.feed_events(&[key_shift(egui::Key::S)]);
        assert!(!m.show_sheet(), "S again toggled the steps sheet closed");
        // Esc and Backspace also close it (Backspace is the Hyperkey-independent path).
        m.feed_events(&[key_shift(egui::Key::S)]);
        m.feed_events(&[key(egui::Key::Escape)]);
        assert!(!m.show_sheet(), "Esc closed the steps sheet");
        m.feed_events(&[key_shift(egui::Key::S)]);
        m.feed_events(&[key(egui::Key::Backspace)]);
        assert!(!m.show_sheet(), "Backspace closed the steps sheet");
    }

    /// `Enter` drills into a visible local scope, a repeat never stacks a
    /// duplicate crumb, and `Esc` widens back out.
    #[test]
    fn enter_drills_to_a_scope_without_duplicate_crumbs() {
        let mut m = model();
        assert!(!m.is_drilled());
        let full = m.graph_collapsed.nodes.len();
        let before_gen = m.layout_gen();

        // Enter focuses the canvas on the selection's full transitive lineage.
        assert!(m.feed_events(&[key(egui::Key::Enter)]), "Enter drilled in");
        assert!(m.is_drilled(), "the canvas is now scoped");
        assert_eq!(m.breadcrumb().len(), 1);
        assert!(
            m.displayed_graph().nodes.len() <= full,
            "the lineage slice fits the graph"
        );
        assert_ne!(
            m.layout_gen(),
            before_gen,
            "the raster cache key changed (a re-layout)"
        );

        // A repeat Enter on the same node is a no-op — no duplicate crumb.
        assert!(
            !m.feed_events(&[key(egui::Key::Enter)]),
            "a repeat Enter does nothing"
        );
        assert_eq!(m.breadcrumb().len(), 1, "no consecutive-duplicate crumb");

        // Esc widens back to the whole graph.
        assert!(m.feed_events(&[key(egui::Key::Escape)]), "Esc drilled out");
        assert!(!m.is_drilled());
        assert!(m.breadcrumb().is_empty());
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

    /// `Enter` drills into the selected node's FULL lineage: when the selection
    /// has both an upstream producer and a downstream consumer, the drilled
    /// scope keeps a two-hop chain — strictly wider than a one-hop slice.
    #[test]
    fn enter_scopes_the_full_transitive_lineage() {
        let mut m = model();
        // Walk to a node that has a producer (so it is not the top source) and
        // a consumer (so it is not the sink): step down once, then the lineage
        // must reach back up to the origin and forward to the dataset.
        assert!(
            m.feed_events(&[key(egui::Key::J)]),
            "j advanced off the top row"
        );
        let sel = m.selected().cloned().expect("a selection");
        let want = brightfield_protocol::graph::lineage(&m.graph_collapsed, &sel);
        assert!(m.feed_events(&[key(egui::Key::Enter)]), "Enter drilled in");
        // The drilled scope is exactly the induced lineage — every kept node is
        // a lineage member and the count matches.
        assert_eq!(
            m.displayed_graph().nodes.len(),
            want.len(),
            "the drilled scope is the selection's full lineage"
        );
        assert!(
            m.displayed_graph().nodes.keys().all(|id| want.contains(id)),
            "every drilled node is on the selection's lineage"
        );
    }

    /// `t` transposes the reading axis — the keyboard twin of the flow-toggle
    /// click control — and re-seeds the layout each way.
    #[test]
    fn t_key_transposes_the_flow() {
        let mut m = model();
        assert_eq!(m.flow(), Flow::Vertical);
        assert!(m.feed_events(&[key(egui::Key::T)]), "t flipped the axis");
        assert_eq!(m.flow(), Flow::Horizontal);
        assert!(m.feed_events(&[key(egui::Key::T)]), "t flipped back");
        assert_eq!(m.flow(), Flow::Vertical);
    }

    /// Backspace is a plain-key fallback for Esc's widen/reset, so widening never
    /// depends on a remapped or synthesized Escape.
    #[test]
    fn backspace_widens_like_esc() {
        let mut m = model();
        assert!(m.feed_events(&[key(egui::Key::Enter)]), "Enter drilled in");
        assert!(m.is_drilled());
        assert!(
            m.feed_events(&[key(egui::Key::Backspace)]),
            "Backspace widened"
        );
        assert!(
            !m.is_drilled(),
            "Backspace popped the drill exactly as Esc does"
        );
        // And it still closes the steps sheet, matching Esc's dual role.
        m.feed_events(&[key_shift(egui::Key::S)]);
        assert!(m.show_sheet());
        assert!(
            m.feed_events(&[key(egui::Key::Backspace)]),
            "Backspace closed the sheet"
        );
        assert!(!m.show_sheet());
    }

    /// A keyboard move bumps the reframe cue so the canvas keeps the freshly
    /// selected node in view.
    #[test]
    fn keyboard_move_requests_a_reframe() {
        let mut m = model_flow(Flow::Vertical);
        let before = m.frame_gen();
        assert!(m.feed_events(&[key(egui::Key::J)]), "j moved the selection");
        assert!(
            m.frame_gen() > before,
            "a keyboard move asks the canvas to reframe"
        );
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
