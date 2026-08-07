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
//! What is still this file's own is the panel's key-hint bar, drawn under
//! whichever window hosts this view. The breadcrumb it used
//! to sit opposite is gone from here: the window has one top bar now, shared
//! with the chart view, and this view's crumbs are content in it. Neither is a
//! `Subject` — they describe the *view*, not a pane, and no pane can be held
//! to them.
//!
//! The pure interaction model ([`ProtocolModel`]) is GPU-free and unit-tested;
//! [`ProtocolDoc`] adds the canvas host the panes share, and
//! [`crate::window::MeridianApp`] wires the document to the dock.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use brightfield_protocol::contract::{SkipReason, StepState};
use brightfield_protocol::contract_graph::{AssetMeta, SeamStatus, StepView};
use brightfield_protocol::graph::{AssetGraph, AssetId, AssetKind, SeamKind, StepId};
use brightfield_protocol::layout::{Flow, Layout, LayoutConfig, Rect};
use brightfield_protocol::panel::{
    inspector_for, kind_label, outline_rows, InspectorFacts, OutlineRow,
};
use brightfield_protocol::{
    collapse_families, explode_ctes, manifest_sql, Dir, FoldOutcome, ProtocolNav, StepRow,
    StepsSheet,
};

use brightfield_render::canvas_host::{Color, PixelSize};

use brightfield_keys::BindingContext;
use brightfield_workbench::registry::{DockSide, Slot};
use brightfield_workbench::subject::RunState;
use brightfield_workbench::{
    chrome, Affordance, EmptyState, Icon, Item, ItemCtx, ItemId, ItemRegistry, ItemSpec, PaneKey,
    Subject, Tone, Verb, ViewKind,
};

use meridian_design::{control, semantic, spacing};

use crate::canvas::{CanvasSlot, EguiCanvasHost};
use crate::design::Mode;
use crate::starts;

// ---------------------------------------------------------------------------
// Offline pipeline: arcform manifest -> asset graph + steps.
// ---------------------------------------------------------------------------

/// Everything the Protocol panel needs, assembled from the offline manifest
/// path (`BRIGHTFIELD_PROTOCOL_OFFLINE=1`): the collapsed + uncollapsed graphs
/// (the fold swaps between them), the same collapsed canvas with each SQL step's
/// CTEs drawn out, and the run-ordered step rows. Measured contract maps
/// (`statuses`/`assets`/`steps`) are empty offline — the inspector then shows
/// lineage detail only.
pub struct ProtocolInputs {
    /// The protocol name (breadcrumb + window title).
    pub protocol: String,
    /// Families collapsed to tiles — the default canvas + the nav's graph.
    pub graph_collapsed: AssetGraph,
    /// The full graph — shown when a family is unfolded.
    pub graph_full: AssetGraph,
    /// The collapsed canvas with every SQL step's top-level CTEs drawn as nodes
    /// of their own — shown while the CTE fold is open. Built once, here,
    /// because both halves it needs are only in scope at load time.
    pub graph_exploded: AssetGraph,
    /// The collapsed canvas with every run of single hand-offs drawn as the one
    /// asset it ends at — shown while the chain fold is open.
    ///
    /// **Never the default**, and the reason is the same one the CTE fold has in
    /// reverse: what this absorbs on the crosswalk is the intermediate build
    /// artefacts and the hosts they came from, which is exactly the provenance
    /// the protocol view exists to show. The geometry is right (23 nodes over 10
    /// ranks become 15 over 6) and the default would be wrong.
    pub graph_contracted: AssetGraph,
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
            graph_full: graph.clone(),
            graph_exploded: graph.clone(),
            graph_contracted: graph,
            statuses: BTreeMap::new(),
            assets: BTreeMap::new(),
            steps: BTreeMap::new(),
            sheet_rows: Vec::new(),
        }
    }

    /// What a run with nobody watching reads instead of diffing pictures: one
    /// line per node that stands in for something the derivation could not
    /// produce. A degrade from the model read opens with its class — `absent`,
    /// `unreadable`, `unavailable` — and then says whose problem it is; one
    /// raised elsewhere reaches [`brightfield_protocol::Degradation::Other`],
    /// which carries the underlying message and no class tag.
    /// `examples/protocol/degrade.yaml` is the second kind.
    ///
    /// **Empty means the render is everything the manifest asked for.** That is
    /// the bit an unattended caller wants, and the other three numbers on the
    /// boot summary — collapsed nodes, full nodes, steps — do not carry it,
    /// because they move with the model's length rather than with the fault.
    /// Measured through
    /// this method and [`crate::window::Boot::describe`] in
    /// `crates/brightfield-shell/tests/protocol_degrade_channel.rs`: on the
    /// two-statement model that suite ships, readable / absent / refused each
    /// print `5 collapsed / 5 full nodes, 3 steps`; widen it to four
    /// statements and readable prints `7 collapsed / 7 full nodes` while both
    /// faults stay at 5/5, the step count 3 throughout. Neither shape lets one
    /// render in hand be compared with the render it should have been, which
    /// is why completeness has to be stated rather than inferred.
    ///
    /// Read off [`Self::graph_full`] rather than the canvas that happens to be
    /// on screen: a collapsed family tile stands in for its members, so a
    /// degrade inside one would be reported by the full graph and missed by the
    /// collapsed one. What is being answered is what the *derivation* could not
    /// do, which the fold does not change.
    ///
    /// [`crate::window::Boot::open_sampled`] prints these, which is how they
    /// reach `brightfield-shot`'s stderr; the same call is the chart path's
    /// diagnostics loop one branch over.
    #[must_use]
    pub fn degrade_report(&self) -> Vec<String> {
        brightfield_protocol::graph::degrades(&self.graph_full)
            .into_iter()
            .map(|d| match &d.step {
                Some(step) => format!("degraded step {step}: {}", d.detail),
                None => format!("degraded node {}: {}", d.node, d.detail),
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Rendering a manifest that has no run behind it
// ---------------------------------------------------------------------------

/// The opt-in that lets a Protocol **manifest** — a declaration, with no run
/// behind it — be rendered.
pub const OFFLINE_VAR: &str = "BRIGHTFIELD_PROTOCOL_OFFLINE";

/// Whether this process has opted a run-less manifest in.
#[must_use]
pub fn offline_optin() -> bool {
    std::env::var(OFFLINE_VAR).is_ok()
}

/// The refusal shown for `what`, a run-less manifest this process did not opt
/// in to.
///
/// # The rule, stated once because it had already been restated
///
/// The default input to this view is an **emitted Protocol+Run contract**: a
/// graph a run actually produced. A manifest is the other thing — the graph a
/// protocol *declares* — and nothing on the canvas distinguishes the two. So a
/// manifest may only be rendered where something else makes the difference
/// plain, and [`OFFLINE_VAR`] is that something for a path handed in from
/// outside.
///
/// The qualifier that is **not** part of it: this is about the artifact class,
/// not about who named the artifact. A defence of the form "the user named
/// that one, so it needs the gate; we shipped this one, so it does not" is a
/// narrower rule than the one recorded, and it was written down once before
/// being noticed.
///
/// # The one exemption, and what it actually is
///
/// A [`crate::starts::Start`] that sets
/// [`run_less`](crate::starts::Start::run_less) is exempt, because its label
/// carries [`crate::starts::RUN_LESS_MARK`]: the disclosure the variable
/// exists to force, made in the place the variable cannot reach — on the
/// button, at the moment of the click.
///
/// **Disclosed once at the pick, then remembered — not disclosed every time it
/// is taken.** That distinction is load-bearing rather than pedantic, because
/// the second way in is one this product ships: `MeridianApp::open_start`
/// records the start's id in
/// [`SavedLayout::opened`](brightfield_workbench::SavedLayout), and
/// [`crate::startup::opening_boot`] reopens it on the next launch — a path with
/// no button and no click, whose restored graph carries `(no run)` nowhere on
/// it. An earlier spelling of this paragraph said the exemption held "on the
/// button, at the moment of the click" full stop, which was false for the
/// launch after that click.
///
/// What makes the remembered form honest is where the memory can come from.
/// `SavedLayout::opened` is written from exactly one place in product code —
/// `MeridianApp::open_start`, checkable with
/// `git grep -n 'live_mut().opened' -- crates/brightfield-shell/src` — so it
/// can only name an id the user picked off a button that disclosed it. What
/// invalidates it: opening anything else, which overwrites it, and a build
/// that no longer ships that start, which `opening_boot` drops rather than
/// propagates.
///
/// `starts.rs` states the same thing at the declaration, and
/// `a_start_that_opens_a_run_less_manifest_says_so_on_its_own_button` holds
/// the label to the flag.
#[must_use]
pub fn run_less_manifest_refusal(what: &str) -> String {
    format!(
        "{what} is a Protocol manifest, not an emitted Protocol+Run contract. \
         To render it offline without a run, set {OFFLINE_VAR}=1."
    )
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
    Ok(inputs_from(&manifest, &sources))
}

/// The same load over manifest **text** and its models' text, rather than a
/// directory on disk.
///
/// What it exists for: the crosswalk starting point in [`crate::starts`] is
/// `include_str!`-ed into the binary, models and all, so there is no directory
/// to read them out of — and resolving `examples/` against the working
/// directory would give a start that works from the repo root and nowhere
/// else.
///
/// `models` maps each `sql:` step's model path — exactly as the manifest
/// spells it — to that model's SQL.
///
/// **A model the manifest names and this map does not carry is an error**, not
/// a degradation. [`brightfield_protocol::graph::load_model_sources`] degrades
/// an unreadable *file* to an opaque chip because a file on a user's disk can
/// legitimately be missing; nothing here can. Every source is `include_str!`-ed
/// at compile time, so the only way one is absent is a key spelled differently
/// from the manifest — and the damage is quiet: the steps sheet is unchanged,
/// every affected step degrades to a chip, and a third of the crosswalk's
/// lineage silently stops being drawn. The front door would go on offering a
/// button that resolves to a render missing the thing it is for.
///
/// # Errors
///
/// A message if the text is not a protocol manifest, does not parse as one, or
/// names a `sql:` model that is not in `models`.
pub fn load_protocol_str(text: &str, models: &[(&str, &str)]) -> Result<ProtocolInputs, String> {
    if !brightfield_protocol::is_protocol_manifest(text) {
        return Err("not a protocol manifest".to_string());
    }
    let manifest = brightfield_protocol::parse_manifest_str(text)
        .map_err(|e| format!("protocol parse error: {e}"))?;
    let mut missing: Vec<&str> = Vec::new();
    let sources: BTreeMap<StepId, Result<String, String>> = manifest
        .steps
        .iter()
        .filter_map(|step| {
            let model = step.sql.as_ref()?;
            let source = match models.iter().find(|(name, _)| name == model) {
                Some((_, sql)) => Ok((*sql).to_string()),
                None => {
                    missing.push(model);
                    Err(format!("{model}: not embedded"))
                }
            };
            Some((step.name.clone(), source))
        })
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "{} names {} model(s) no embedded source was found for: {}",
            manifest.name,
            missing.len(),
            missing.join(", ")
        ));
    }
    Ok(inputs_from(&manifest, &sources))
}

/// Derive the panel's inputs from a parsed manifest and its models' sources —
/// the half both loaders share, so a start that ships inside the binary and a
/// manifest read off disk produce the same graph by construction rather than
/// by two spellings agreeing.
fn inputs_from(
    manifest: &brightfield_protocol::Manifest,
    sources: &BTreeMap<StepId, Result<String, String>>,
) -> ProtocolInputs {
    let graph_full = brightfield_protocol::graph::build_graph(manifest, sources);
    let graph_collapsed = collapse_families(&graph_full);
    // EXPLODE, THEN COLLAPSE — the order is load-bearing, not a preference.
    //
    // A CTE node carries the step that declared it, and `collapse_families`
    // retains every node whose step is a family member, so exploding first and
    // collapsing after is closed: the CTEs of a family member survive the
    // collapse with the rest of that member's assets.
    //
    // The other order is broken by construction. A family TILE carries no step
    // at all, so a CTE declared inside a collapsed step has nothing to resolve
    // its producing relation against, and the explode would skip it silently —
    // an empty fold rather than a visible failure. Composing the two the wrong
    // way round is the one bug this line cannot show on the crosswalk (whose
    // only CTE-bearing step is outside every family), so it is asserted in
    // `explode_then_collapse_keeps_the_ctes` rather than left to the surface.
    let graph_exploded = collapse_families(&explode_ctes(&graph_full, &manifest_sql(sources)));
    // CONTRACT LAST — after the explode and after the collapse, and that is the
    // same kind of fact as the line above rather than a preference.
    //
    // `explode_ctes` resolves what a CTE body reads against the relation-shaped
    // nodes of the graph it is handed. A contraction that had already absorbed
    // one of those relations leaves the explode nothing to wire from: the CTE
    // box is still drawn, its input comes from nowhere, and the direct edge it
    // should have re-routed is still there — the wrong-order failure of the
    // line above, one pass further along. Asserted in
    // `contracting_before_the_explode_orphans_the_ctes`, because the crosswalk
    // cannot show it either: the relation its one CTE-bearing step reads is a
    // fan-in, so no chain touches it and both orders are pixel-identical there.
    let graph_contracted = brightfield_protocol::contract_chains(&graph_collapsed);
    let sheet_rows = synth_sheet_rows(&graph_full);
    ProtocolInputs {
        protocol: manifest.name.clone(),
        graph_collapsed,
        graph_full,
        graph_exploded,
        graph_contracted,
        statuses: BTreeMap::new(),
        assets: BTreeMap::new(),
        steps: BTreeMap::new(),
        sheet_rows,
    }
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
    graph_exploded: AssetGraph,
    graph_contracted: AssetGraph,
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
    /// Whether the canvas draws each SQL step's CTEs as nodes of their own.
    ///
    /// **One flag over the whole canvas, not one per step.** The alternative —
    /// a set of exploded step ids — is a strictly larger interface, and on the
    /// only surface this is being judged against it is unfalsifiable: the
    /// crosswalk has exactly one SQL step that declares CTEs, so global and
    /// per-step render the same pixels. If a protocol with two such steps ever
    /// wants them opened independently, this becomes a `BTreeSet<StepId>` and
    /// two methods change — [`ProtocolModel::displayed_graph`] and
    /// [`ProtocolModel::toggle_fold`].
    cte_expanded: bool,
    /// Whether the canvas draws each run of single hand-offs as the one asset it
    /// ends at.
    ///
    /// Held to the same rule as [`ProtocolModel::cte_expanded`] and for the same
    /// reason: true **only** while `graph_contracted` is the graph
    /// [`ProtocolModel::displayed_graph`] returns. The two folds are mutually
    /// exclusive — `graph_contracted` is built over the collapsed canvas, not
    /// over the exploded one, so there is no picture in which both are open —
    /// and opening either closes the other rather than stacking a flag behind
    /// it. A combined graph is the larger increment this deliberately is not, for
    /// the reason spelled out on `cte_expanded`: on the crosswalk it would buy a
    /// state nothing reaches, because no chain touches the one CTE-bearing step.
    chain_contracted: bool,
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
        let layout = Self::boot_layout(&inputs, flow);
        let sheet = StepsSheet::from_rows(inputs.sheet_rows);
        let family_ids: Vec<AssetId> = inputs
            .graph_collapsed
            .nodes
            .iter()
            .filter(|(_, n)| n.kind == AssetKind::Family)
            .map(|(id, _)| id.clone())
            .collect();
        let mut model = Self {
            protocol: inputs.protocol,
            graph_collapsed: inputs.graph_collapsed,
            graph_full: inputs.graph_full,
            graph_exploded: inputs.graph_exploded,
            graph_contracted: inputs.graph_contracted,
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
            cte_expanded: false,
            chain_contracted: false,
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

    /// The layout the canvas opens on: the collapsed graph at `flow`, before any
    /// fold, drill or transpose.
    ///
    /// Read twice — [`ProtocolModel::new`] seeds the model with it, and
    /// [`crate::window::Boot`] sizes the window from it before the model
    /// exists — so it is declared once. Sizing a window from a second spelling
    /// of "what the canvas opens on" is exactly how the panel's rails came to be
    /// 260px wide in one place and 24% of the window in another.
    #[must_use]
    pub fn boot_layout(inputs: &ProtocolInputs, flow: Flow) -> Layout {
        let cfg = LayoutConfig {
            flow,
            ..LayoutConfig::default()
        };
        brightfield_protocol::layout(&inputs.graph_collapsed, &cfg)
    }

    /// The canvas the window is **sized for**: the componentwise largest of the
    /// states this view spends its time in, at the boot flow.
    ///
    /// # Why the boot canvas alone is the wrong thing to size a window from
    ///
    /// It was what [`crate::window::Boot::window_size`] used, and it fitted by
    /// construction and by nothing else: the crosswalk laid out at 1034×1120 into
    /// a 1034.4×1120.0 content box, under a point of slack in both axes. Every
    /// state reachable from there by one keystroke overflowed it and stayed
    /// overflowed, because a window is sized once at boot and never resized — so
    /// the CTE fold's extra rank was guaranteed scroll, measured against a
    /// configuration the user leaves immediately. There is no zoom in this binary
    /// and no fit-to-view, so scroll is the whole of the recovery.
    ///
    /// # What is sized for, and what is left to scroll
    ///
    /// Sized for: the boot canvas itself, the CTEs exploded, and the chains
    /// contracted — the states `za` reaches at the flow the window opened on
    /// whose cost is a rank or two. A drill scope is strictly smaller than the
    /// graph it is induced from, so it never sets the envelope.
    ///
    /// Left to scroll: the **flow transpose**, which is a change of reading axis
    /// rather than of detail and lays the crosswalk out 2146 points across; and
    /// the **family unfold**, which is the trade this makes and is argued below.
    ///
    /// # The family unfold: the case for sizing to it, and why it loses anyway
    ///
    /// The case is real and is kept here rather than deleted, because a reader
    /// who re-derives it from scratch will re-add the graph. It is not an
    /// unlikely state: [`ProtocolNav`] boots its cursor on the first node with no
    /// producer in id order, and on the crosswalk that node is the family tile,
    /// so `za` pressed at boot — with no navigation at all — is the unfold. It
    /// really is one keystroke away, and a window sized to spare the *second*
    /// gesture but not the first would be measuring against the same kind of
    /// assumption this function was written to replace.
    ///
    /// It loses on what it costs every other session. The unfold takes the
    /// crosswalk from 1018 points across to 1586, and so the window from 1948 to
    /// 3000: a thousand points of window, on every launch, to spare one keystroke
    /// in a state that is opened rarely and left immediately. And 3000 points is
    /// twice the width of a 1512-point laptop panel — so sizing for it does not
    /// merely waste space on a large display, it asks for a window a laptop
    /// cannot show at all, and what then arrives is whatever the compositor chose
    /// to grant. One keystroke's convenience is not worth that, in either
    /// direction, so the unfold scrolls like the transpose does.
    ///
    /// # This answers what the content wants, not what the screen will give
    ///
    /// Even at 1948 points the vertical crosswalk is wider than a laptop panel,
    /// and nothing here knows that — the two questions are separate and are
    /// answered separately. See [`crate::window::window_size_on_display`], which
    /// caps what is asked for against the monitor the window opens on.
    #[must_use]
    pub fn boot_extent(inputs: &ProtocolInputs, flow: Flow) -> (f64, f64) {
        let cfg = LayoutConfig {
            flow,
            ..LayoutConfig::default()
        };
        [
            &inputs.graph_collapsed,
            &inputs.graph_exploded,
            &inputs.graph_contracted,
        ]
        .into_iter()
        .map(|g| {
            let l = brightfield_protocol::layout(g, &cfg);
            (l.width, l.height)
        })
        .fold((0.0, 0.0), |(w, h), (lw, lh)| (w.max(lw), h.max(lh)))
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
    /// active, else the full graph when a family is unfolded, else the exploded
    /// graph when the CTE fold is open, else the contracted graph when the chain
    /// fold is open, else the collapsed graph.
    ///
    /// **These arms do not compete — they cannot both be taken.**
    /// `graph_exploded` is built over the *collapsed* canvas, so no graph in
    /// this struct is both unfolded and exploded, and an earlier cut of this
    /// method resolved that by precedence: the scope beat the family, the
    /// family beat the CTE fold, and `cte_expanded` was left standing while the
    /// canvas showed something else. That is a fold armed with nothing on the
    /// screen to show for it — the user presses `za` on the family tile, both
    /// CTEs vanish, and a later, unrelated keystroke brings them back unasked.
    ///
    /// So the flag is constrained instead of ranked. **`cte_expanded` is true
    /// only while `graph_exploded` is the graph this returns**, held by three
    /// rules in the private `toggle_fold` and `drill_in` (both documented on
    /// themselves; they are not linked here because a public item may not
    /// intra-doc-link a private one):
    ///
    /// - `za` is refused outright inside a drill scope — neither fold could be
    ///   drawn under one, so neither is armed;
    /// - the CTE half is refused while a family is unfolded (the full graph is
    ///   not an exploded graph);
    /// - unfolding a family, or drilling in, *closes* the CTE fold rather than
    ///   suspending it, so nothing returns unbidden.
    ///
    /// `display_expanded` is not held to the same rule, and the asymmetry is
    /// deliberate: it is a cache of the nav's own fold state
    /// ([`ProtocolNav::is_expanded`]), which survives a drill because the nav's
    /// cursor tree does. Clearing it here would leave this model disagreeing
    /// with the nav, and the next `za` on that tile would be the dead keystroke
    /// this is trying to prevent. `cte_expanded` has no other holder, so it can
    /// be dropped honestly.
    ///
    /// Building a fourth graph to serve unfolded-*and*-exploded is the larger
    /// increment this deliberately is not: on the crosswalk it buys a state
    /// nothing reaches, because its one CTE-bearing step belongs to no family.
    #[must_use]
    pub fn displayed_graph(&self) -> &AssetGraph {
        if let Some(scope) = &self.scope_graph {
            scope
        } else if self.display_expanded {
            &self.graph_full
        } else if self.cte_expanded {
            &self.graph_exploded
        } else if self.chain_contracted {
            &self.graph_contracted
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

    /// Make the sheet flag agree with the dock's active centre tab.
    ///
    /// The shell drives the tab strip from this flag before it draws, and reads
    /// a manual tab click back into it afterwards, so a pointer click on the
    /// Steps tab opens the sheet by the same route the `shift-S` verb does.
    pub fn set_show_sheet(&mut self, show: bool) {
        self.show_sheet = show;
    }

    /// Whether the canvas shows the unfolded family.
    #[must_use]
    pub fn is_expanded(&self) -> bool {
        self.display_expanded
    }

    /// Whether the canvas draws each SQL step's CTEs as nodes of their own.
    #[must_use]
    pub fn is_cte_expanded(&self) -> bool {
        self.cte_expanded
    }

    /// Whether the canvas draws each run of single hand-offs as the one asset it
    /// ends at.
    #[must_use]
    pub fn is_chain_contracted(&self) -> bool {
        self.chain_contracted
    }

    /// Whether `id` is a node the chain fold would absorb — the cursor positions
    /// the chain half of `za` answers to, and the ones whose ring the canvas has
    /// to redirect while the fold is open.
    ///
    /// Read off the two graphs rather than recomputed: a node the collapsed
    /// canvas has and the contracted canvas does not is, by construction,
    /// absorbed into a chain.
    #[must_use]
    pub fn is_absorbed_by_a_chain(&self, id: &AssetId) -> bool {
        self.graph_collapsed.nodes.contains_key(id) && !self.graph_contracted.nodes.contains_key(id)
    }

    /// Where the selection's ring belongs on the canvas.
    ///
    /// Normally the selection itself. While the chain fold is open the selection
    /// may name a node the fold absorbed — the outline still lists it, the nav
    /// still walks to it, the inspector still answers for it, because all three
    /// read the uncollapsed graph — and there is no rectangle under that id to
    /// ring. This resolves it to the node it was folded **into**, so a keyboard
    /// walk through an absorbed run lights the asset that run produced rather
    /// than lighting nothing at all.
    ///
    /// It is not the whole of the asymmetry — see the note on
    /// [`ProtocolModel::is_absorbed_by_a_chain`]'s callers — but it is the half
    /// that decides whether the fold is navigable.
    #[must_use]
    pub fn selection_site(&self) -> Option<AssetId> {
        let sel = self.selected.clone()?;
        if !self.chain_contracted || self.layout.positions.contains_key(&sel) {
            return Some(sel);
        }
        brightfield_protocol::chain_tails(&self.graph_collapsed)
            .get(&sel)
            .cloned()
            .or(Some(sel))
    }

    /// The invariant both canvas folds are held to: **armed only while the graph
    /// each names is the one on screen** — see
    /// [`ProtocolModel::displayed_graph`].
    ///
    /// Identity, not equality: it asks whether `displayed_graph` returned *this
    /// struct's* `graph_exploded` / `graph_contracted`, so a scope or a full
    /// graph that happened to compare equal could not satisfy it. It also
    /// rejects both flags being up at once, which no picture could show.
    #[cfg(test)]
    fn folds_are_on_screen(&self) -> bool {
        let shown = self.displayed_graph();
        match (self.cte_expanded, self.chain_contracted) {
            // Two flags, one canvas: no graph in this struct is both.
            (true, true) => false,
            (true, false) => std::ptr::eq(shown, &self.graph_exploded),
            (false, true) => std::ptr::eq(shown, &self.graph_contracted),
            (false, false) => true,
        }
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
            "toggle-fold" => self.toggle_fold(),
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
    ///
    /// **Drilling closes an open canvas fold; it does not suspend it.** The
    /// scope is induced over the collapsed graph, so the CTEs (or the contracted
    /// chains) leave the screen at this keystroke either way — the only question
    /// is whether the flag leaves with them. Left standing it would put them
    /// back on a later `Esc`, at a
    /// moment the user pressed nothing that means "explode". See
    /// [`ProtocolModel::displayed_graph`] for why `display_expanded` is not
    /// treated the same way.
    fn drill_in(&mut self) -> bool {
        if !self.nav.drill_in() {
            return false;
        }
        self.cte_expanded = false;
        self.chain_contracted = false;
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

    /// `za` — open or close the detail under the cursor, swapping the displayed
    /// graph and invalidating the layout so the canvas visibly re-lays-out.
    ///
    /// Three things fold, resolved by what the cursor is on:
    /// - a **family tile** unfolds to its members (the original behaviour);
    /// - a node **produced by a `sql:` step** opens that statement's CTEs onto
    ///   the canvas, so the joins inside the step are lineage rather than one
    ///   rectangle;
    /// - a node a **run of single hand-offs would absorb** contracts every such
    ///   run on the canvas to the asset it ends at. The cursor lands on the thing
    ///   that folds away, which is the ordinary meaning of a fold key.
    ///
    /// The three are resolved in that order, and the order is what keeps the
    /// third from shadowing the second: the crosswalk's one CTE-bearing relation
    /// is also the head of a chain, so a chain arm tried first would make the CTE
    /// fold unreachable from the only cursor position it answers to.
    ///
    /// All are the same verb, deliberately. `protocol_key_table` is a
    /// `BTreeMap` keyed by keystroke string, so a second Protocol-context verb
    /// bound to `z a` would silently overwrite the first and which one survived
    /// would depend on the order the registry happens to list them in. `z a` is
    /// also the only `z`-prefixed binding in the registry — there is no
    /// `zo`/`zc`/`zR`/`zM` family to extend, and the chord is resolved by hand
    /// through [`ProtocolModel::feed_key`]'s pending flag. So the verb is
    /// broadened, not duplicated.
    ///
    /// **Every path that reports no change leaves the model untouched**, and
    /// the list is longer than "the cursor is on nothing foldable". `za` is a
    /// no-op — no flag flipped, no re-layout, no repaint — when:
    ///
    /// - the cursor is on neither (a source file, an operator's output, a
    ///   family member);
    /// - a **drill scope** is active. The canvas is showing an induced slice,
    ///   and neither fold can be drawn under one, so neither is armed under
    ///   one. This guard runs before [`ProtocolNav::toggle_fold`], so the
    ///   nav's own fold state is not mutated either;
    /// - the cursor is on a SQL-produced node, or on one a chain would absorb,
    ///   while a **family is unfolded**. The full graph is neither an exploded
    ///   graph nor a contracted one (see [`ProtocolModel::displayed_graph`]), so
    ///   opening either canvas fold here could only arm a flag with nothing on
    ///   screen behind it.
    ///
    /// The alternative in each case is a keystroke that changes state the
    /// screen does not reflect, and surfaces it later unbidden. A refused
    /// gesture is the smaller cost.
    ///
    /// When the family arm *does* fire it closes both canvas folds as it goes:
    /// the unfolded graph is neither picture, and a flag that outlived its
    /// picture is the same defect one keystroke later.
    fn toggle_fold(&mut self) -> bool {
        if self.scope_graph.is_some() {
            return false;
        }
        if self.nav.toggle_fold() == FoldOutcome::NotAFamily {
            return self.toggle_canvas_fold();
        }
        self.display_expanded = self.family_ids.iter().any(|id| self.nav.is_expanded(id));
        self.cte_expanded = false;
        self.chain_contracted = false;
        self.selected = self.nav.cursor().cloned();
        self.recompute_layout();
        true
    }

    /// The two canvas halves of `za`: flip the whole-canvas CTE explode when the
    /// cursor is on a node a `sql:` step produced, else the whole-canvas chain
    /// contraction when it is on a node a chain would absorb — and re-lay-out so
    /// the raster cache drops the picture it was showing.
    ///
    /// Both are refused while a family is unfolded, because
    /// [`ProtocolModel::displayed_graph`] would go on drawing `graph_full`. The
    /// drill-scope half of the same rule is enforced by the caller, before the
    /// nav is touched.
    ///
    /// **Opening one closes the other.** They are two pictures of the same
    /// canvas and there is no third graph that is both, so a flag left standing
    /// behind the other's picture is precisely the armed-but-invisible state the
    /// CTE fold's rules exist to prevent.
    fn toggle_canvas_fold(&mut self) -> bool {
        debug_assert!(
            self.scope_graph.is_none(),
            "toggle_fold refuses inside a drill scope before reaching here"
        );
        if self.display_expanded {
            return false;
        }
        if self.cursor_is_sql_produced() {
            self.cte_expanded = !self.cte_expanded;
            self.chain_contracted = false;
        } else if self
            .nav
            .cursor()
            .is_some_and(|id| self.is_absorbed_by_a_chain(id))
        {
            self.chain_contracted = !self.chain_contracted;
            self.cte_expanded = false;
        } else {
            return false;
        }
        self.recompute_layout();
        true
    }

    /// Whether the node under the cursor was produced by a `sql:` step.
    ///
    /// Read off the COLLAPSED graph, which is the graph the nav walks: a cursor
    /// the nav can hold is always a node of it, exploded or not.
    fn cursor_is_sql_produced(&self) -> bool {
        let Some(cursor) = self.nav.cursor() else {
            return false;
        };
        let Some(step) = self
            .graph_collapsed
            .nodes
            .get(cursor)
            .and_then(|n| n.step.as_ref())
        else {
            return false;
        };
        self.graph_collapsed
            .seams
            .get(step)
            .is_some_and(|seam| matches!(seam.kind, SeamKind::Sql { .. }))
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
    ///
    /// # The pixels follow the LAYOUT, not the graph — read this before adding
    /// a fold
    ///
    /// `render_asset_graph_with_status` iterates `layout.positions` and looks
    /// each id up in the graph: **a graph node with no position is silently
    /// skipped, and a position with no node draws nothing.** So the displayed
    /// graph only reaches the screen through this method, and a change to
    /// [`ProtocolModel::displayed_graph`] that does not reach it is invisible
    /// on the canvas *and* invisible to the two committed CTE pixel baselines,
    /// which would stay green photographing the old picture.
    ///
    /// Nothing shipped can hit that today — every fold, drill and flow change
    /// calls this — and the boot layout is laid out over
    /// `graph_collapsed` regardless, so it is only ever the *first* frame that
    /// is layout-pinned. The trap is live for the per-step explode this note's
    /// sibling comment on `cte_expanded` sketches: flipping one step's id in a
    /// set is exactly the kind of edit that reads as "the graph changed, so the
    /// canvas changed". It does not. Re-lay-out, or nothing moves.
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
    /// The content box the canvas pane was last handed, in window-space logical
    /// points — `None` until a frame has been laid out.
    ///
    /// Written by the canvas pane before it looks for a texture, so it is
    /// observable on a *headless* document, and written *outside* the scroll
    /// area so it is the box the dock gave the pane rather than the scrolled
    /// content's. The chart view has had this since its window was caught
    /// clipping its own raster; this view had no equivalent, and its failure
    /// mode is quieter still — the canvas scrolls rather than clips, so a
    /// window a hundred points short opens the graph part-scrolled and no
    /// baseline can tell that from a graph that is simply large.
    pub viewport: Option<egui::Rect>,
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
            viewport: None,
        }
    }

    /// A document with no device behind it — the [`CanvasSlot`] holds no host.
    #[must_use]
    pub fn headless(model: ProtocolModel) -> Self {
        Self {
            model,
            canvas: CanvasSlot::headless(),
            viewport: None,
        }
    }

    /// An empty document: no assets, no seams, no steps, no device.
    ///
    /// The value [`protocol_registry`]'s audit runs against.
    #[must_use]
    pub fn empty() -> Self {
        Self::headless(ProtocolModel::new(ProtocolInputs::empty(), Flow::Vertical))
    }

    /// Replace the graph with a freshly built one, keeping the reading axis
    /// the user chose.
    ///
    /// The `invalidate` is cheaper insurance here than on the chart side —
    /// this view's [`CanvasKey`] carries a layout `generation`, which a new
    /// model resets to zero, so an equal-shaped replacement could collide with
    /// the presented key. Dropping it is one line and removes the question.
    pub fn open(&mut self, inputs: ProtocolInputs) {
        self.model = ProtocolModel::new(inputs, self.model.flow());
        self.canvas.invalidate();
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

    /// Declare which panes the frame laid out, so the host can free the canvas
    /// slot of any pane that has gone. See [`crate::window::MeridianApp`]'s
    /// sweep, which is the only caller and which explains what it hands in.
    pub(crate) fn sweep(&mut self, visible: &BTreeSet<PaneKey>) {
        if let Some(host) = self.canvas.host_mut() {
            host.end_frame(visible);
        }
    }

    /// Re-raster the DAG through the host only when [`CanvasKey`] changed, and
    /// hand the canvas pane the texture to paint.
    pub(crate) fn present(&mut self, ppp: f32, mode: Mode) {
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
/// Called at boot from [`crate::window::MeridianApp`], which is its only
/// caller, and before any layout file could be read. Idempotent, so a test
/// binary that builds two windows neither falls over nor grows the vocabulary.
/// That the window publishes *this* view's ids even when it opened on the other
/// one is asserted through the window rather than here, because the property is
/// about the window's boot and not about this function.
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
/// registry lays the dock out with it, and
/// [`protocol_window_size`](crate::window::protocol_window_size) sizes the
/// window from it. The pair of pixel constants this replaces said 260px and
/// 300px while the tiles said 24% and 22%, and the two had drifted.
pub(crate) const OUTLINE_SHARE: f32 = 0.24;
/// The inspector rail's share of the window.
pub(crate) const INSPECTOR_SHARE: f32 = 0.22;

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

    fn empty_state(&self, doc: &ProtocolDoc) -> Option<EmptyState> {
        (!doc.model.has_assets()).then(|| {
            EmptyState::new(
                ICON_OUTLINE,
                "No assets yet",
                "This protocol declares no assets, so there is nothing to list. \
                 Open a manifest whose steps build at least one table or file.",
            )
        })
    }

    fn describe(&self, _doc: &ProtocolDoc) -> Subject {
        Subject::new("Outline", ICON_OUTLINE, BindingContext::Protocol)
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

    /// The protocol view's **front door**.
    ///
    /// Reached by switching to this view on a launch that named no manifest,
    /// so its prose cannot assume one was opened. The affordance opens the
    /// crosswalk that ships with the binary — see [`crate::starts`] for why it
    /// is an `Action::Open` and not a verb.
    ///
    /// The crosswalk is a manifest with no run behind it, which is the artifact
    /// class [`run_less_manifest_refusal`] gates. It is not exempted here by
    /// being shipped: it is exempted because its label carries
    /// [`starts::RUN_LESS_MARK`], which is the disclosure
    /// [`OFFLINE_VAR`] exists to force, made where the user is looking. That
    /// label is read straight off the [`starts::Start`], so the button and the
    /// exemption cannot say different things.
    ///
    /// The disclosure is made **here**, once, and the layout file remembers the
    /// pick — a later launch reopens the crosswalk without drawing this pane at
    /// all. [`run_less_manifest_refusal`] states that half of the rule.
    fn empty_state(&self, doc: &ProtocolDoc) -> Option<EmptyState> {
        if !doc.model.displayed_graph().nodes.is_empty() {
            return None;
        }
        let mut empty = EmptyState::new(
            ICON_CANVAS,
            "Nothing to draw",
            "No protocol is open, or the graph in view holds no assets. \
             Start from the example below, or widen the drill scope.",
        );
        if let Some(start) = starts::for_view(ViewKind::Protocol) {
            empty = empty.with_next(Affordance::open(start.label, start.id));
        }
        Some(empty)
    }

    fn describe(&self, _doc: &ProtocolDoc) -> Subject {
        Subject::new("Canvas", ICON_CANVAS, BindingContext::Protocol)
    }

    fn ui(&mut self, doc: &mut ProtocolDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        // Recorded *before* the texture check and *outside* the scroll area, so
        // a headless document still reports the box the dock gave this pane.
        // See `ProtocolDoc::viewport`.
        doc.viewport = Some(ui.max_rect());
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

                // The keyboard cursor, ringed with the design system's ONE
                // focus ring — `meridian-egui`'s, the same primitive every
                // Meridian surface draws, reading its mode from the themed
                // context. This was the last shell call site of the workbench
                // chrome's second ring implementation; the hand-rolled 2px
                // stroke at a 4px radius both replaced matched nothing else
                // in the product.
                // `selection_site`, not `selected`: while the chain fold is open
                // the selection can name a node the fold absorbed, which the
                // outline and the inspector still answer for and the canvas has
                // no rectangle for. The ring goes on the node it folded into.
                if let Some(sel) = doc.model.selection_site() {
                    if let Some(node) = doc.model.layout().positions.get(&sel).cloned() {
                        let r = node_rect(rect.min, &node);
                        meridian_egui::widgets::focus_ring(ui, r, meridian_design::radius::CONTROL);
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

    fn empty_state(&self, doc: &ProtocolDoc) -> Option<EmptyState> {
        (!doc.model.has_selection()).then(|| {
            EmptyState::new(
                ICON_INSPECTOR,
                "Nothing selected",
                "Click a node in the canvas or a row in the outline, or move the \
                 cursor with h j k l.",
            )
        })
    }

    fn describe(&self, _doc: &ProtocolDoc) -> Subject {
        Subject::new("Inspector", ICON_INSPECTOR, BindingContext::Protocol)
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

    // The data-honesty channel, beside the execution channel above: what the
    // previewed data is worth against the pipeline as it stands, ingested
    // from the contract's recorded state + typed skip reason (see
    // `run_state_recorded`). Only for produced assets — an external input is
    // not run. The view is read-only today, so nothing is locally edited and
    // the recorded verdict is composed against an empty `EditOverlay`; the
    // editing bridge reports into that overlay when it lands.
    if facts.producing_step.is_some() {
        run_state_field(
            ui,
            mode,
            run_state_recorded(facts.step_state, facts.skip_reason),
        );
    }

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

    fn empty_state(&self, doc: &ProtocolDoc) -> Option<EmptyState> {
        doc.model.sheet().is_empty().then(|| {
            EmptyState::new(
                ICON_STEPS,
                "No steps yet",
                "This protocol runs nothing. A manifest with op: or sql: steps \
                 fills this list in run order.",
            )
        })
    }

    fn describe(&self, _doc: &ProtocolDoc) -> Subject {
        Subject::new("Steps", ICON_STEPS, BindingContext::Protocol)
    }

    fn ui(&mut self, doc: &mut ProtocolDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        // The one Meridian table chrome (`crate::data_grid`): sticky header,
        // the dense row rung, the cursor row under the one selection wash,
        // and the order column right-aligned in the tabular-numeral scope.
        // This retires the bespoke `egui::Grid` render that stood here — and
        // with it the mono-font order column, which was alignment-by-font,
        // the thing the density guideline forbids. The sheet CORE — rows,
        // cursor, ordering — stays in `brightfield_protocol::sheet`,
        // untouched; this pane is only its projection.
        let sheet = doc.model.sheet();
        let mut source = crate::data_grid::StepSheetRows::new(sheet.rows(), sheet.cursor());
        crate::data_grid::show_table(ui, "proto-steps-grid", cx.mode, &mut source);
    }
}

// ---------------------------------------------------------------------------
// Shared pane helpers.
// ---------------------------------------------------------------------------

/// The one UI type size. Chrome has no headings — see
/// [`brightfield_workbench::chrome`].
pub(crate) fn ui_font() -> egui::FontId {
    egui::FontId::proportional(meridian_design::typography::UI_SIZE)
}

/// The same size in the Meridian mono face, for a value the reader compares
/// character by character: an address, the key hints.
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

// ---------------------------------------------------------------------------
// Run-state: the data-honesty channel of the status/inspector region.
//
// The inspector's "Status" field is the *execution* channel — what the run
// did (ok / skipped / failed). Run-state is the other channel: what the
// previewed DATA is worth against the pipeline as it currently stands. The
// two deliberately disagree in one famous case — a hash-clean skip is
// "skipped" as execution and "fresh" as data — which is why they are two
// fields and not one.
//
// Everything here INGESTS. The run contract already records each step's
// terminal state and a typed skip reason computed by the engine's own
// staleness pass; nothing in this shell re-derives freshness from hashes.
// What the shell adds is the one thing only it can know: the edits the user
// has made since that record was written, and which previews those edits
// invalidate — a walk over the lineage the contract recorded, not a second
// staleness computation.
// ---------------------------------------------------------------------------

/// The run contract's own verdict on a step's data, ingested — never
/// recomputed — from its recorded terminal state and typed skip reason.
///
/// - a recorded success is [`RunState::Fresh`];
/// - a skip is [`RunState::Fresh`] only when its typed reason is the engine's
///   freshness proof (`hash_clean` / the precondition pair) — an unproven
///   skip refuses the claim and reads [`RunState::NeverRun`], the safe
///   direction;
/// - a recorded failure is [`RunState::Failed`];
/// - no record at all (an unrun step, the offline manifest, an external
///   input's absent producer) is [`RunState::NeverRun`] — never silently
///   dressed as fresh.
#[must_use]
pub fn run_state_recorded(state: Option<StepState>, skip: Option<SkipReason>) -> RunState {
    match state {
        None | Some(StepState::Unknown) => RunState::NeverRun,
        Some(StepState::Failed) => RunState::Failed,
        Some(StepState::Success) => RunState::Fresh,
        Some(StepState::Skipped) => match skip {
            Some(reason) if reason.proves_fresh() => RunState::Fresh,
            _ => RunState::NeverRun,
        },
    }
}

/// The view-local edit overlay: which steps have been edited since the run
/// contract was emitted, and which other steps those edits drag stale.
///
/// This is the propagation-in-the-representation piece: marking step 2 edited
/// labels steps 3..7 stale **without running anything** — the contract on
/// disk still says fresh, and stays authoritative for everything the edits do
/// not reach. The drag itself is
/// [`brightfield_protocol::downstream_steps`], a walk over the lineage the
/// contract recorded (the engine's own staleness pass drags dependents the
/// same way at run time; this is its representation-side mirror, not a rival
/// computation).
///
/// No editor surface feeds this yet — the protocol view is read-only today,
/// so the live panes compose against an empty overlay and the recorded
/// verdict is the whole truth. The type exists now so the editing bridge has
/// one honest place to report into, and so the honesty property is pinned by
/// tests before any editor ships.
#[derive(Debug, Clone, Default)]
pub struct EditOverlay {
    /// Steps edited since the contract was emitted.
    edited: BTreeSet<StepId>,
    /// Steps the edits drag stale, through the recorded lineage.
    dragged: BTreeSet<StepId>,
}

impl EditOverlay {
    /// Record an edit to `step` and re-derive the drag through `graph` — in
    /// the representation only; nothing runs.
    pub fn mark_edited(&mut self, step: impl Into<String>, graph: &AssetGraph) {
        self.edited.insert(step.into());
        self.dragged = brightfield_protocol::downstream_steps(graph, &self.edited);
    }

    /// Whether nothing has been edited (the read-only view's standing state).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.edited.is_empty()
    }

    /// The overlay's verdict for `step`, given the contract's `recorded` one.
    ///
    /// Precedence, most-recent-fact first: the user's own edit to this step
    /// beats everything (whatever the data was, the definition has left it
    /// behind); a recorded failure stays visible under an upstream edit (both
    /// are non-current, and the failure is the more actionable signal); an
    /// upstream edit drags an otherwise fresh or unrun step; everything else
    /// keeps the contract's word.
    #[must_use]
    pub fn apply(&self, step: &str, recorded: RunState) -> RunState {
        if self.edited.contains(step) {
            return RunState::StaleOwnEdit;
        }
        if recorded != RunState::Failed && self.dragged.contains(step) {
            return RunState::StaleUpstream;
        }
        recorded
    }
}

/// One inspector row for the data-honesty channel: caption, the run-state's
/// own label in its own tone ink, and its gloss. Not [`field`], deliberately —
/// a run-state is the one inspector value whose *ink* carries meaning (fresh
/// green, stale warning, never-run neutral), and the label + tone pair is
/// what keeps never-run visually distinct from fresh.
fn run_state_field(ui: &mut egui::Ui, mode: Mode, state: RunState) {
    let sem = semantic(mode.is_dark());
    ui.add_space(spacing::SPACE_4);
    ui.label(
        egui::RichText::new("DATA")
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
    ui.label(
        egui::RichText::new(state.label())
            .font(ui_font())
            .color(chrome::tone_colour(state.tone(), mode)),
    );
    ui.label(
        egui::RichText::new(state.gloss())
            .font(ui_font())
            .color(chrome::colour(sem.text.muted)),
    );
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

/// The bottom key-hint bar + a flow/state indicator (read-only — no model
/// mutation, so it is a free function).
pub(crate) fn hint_ui(ui: &mut egui::Ui, model: &ProtocolModel, mode: Mode) {
    let sem = semantic(mode.is_dark());
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
    // `horizontal_centered`, like the top bar — the band's height is
    // `BAR_HEIGHT`, pinned by the panel's `exact_size`, and this fills and
    // vertically centres within it. A content-driven `horizontal` with manual
    // `add_space` padding instead grows to whatever its row plus that padding
    // comes to, and when that exceeds `BAR_HEIGHT` egui does not clip it: the
    // panel frame reports the pinned height but is laid out past the window
    // edge, and the dock below is handed the overrun as unbudgeted slack. Filling
    // the band keeps the key-hint row exactly one `BAR_HEIGHT` tall, whatever the
    // style makes a row of labels measure.
    ui.horizontal_centered(|ui| {
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
        assert_eq!(t.get("z a").copied(), Some("toggle-fold"));
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

    // -----------------------------------------------------------------------
    // The CTE fold: `za` on a node a `sql:` step produced.
    // -----------------------------------------------------------------------

    /// The crosswalk's one relation with CTEs behind it: `sec_entities.sql`
    /// reads two files, joins them, and draws as a single rectangle until this
    /// fold is opened.
    const SQL_PRODUCED: &str = "asset.edgar_gleif.sec_entities";

    /// The two CTEs that statement declares, in the id order the canvas gains
    /// them. Pinned here as well as in the lineage suite so a change to the
    /// derivation cannot quietly change what this gesture puts on screen.
    const STEP_CTES: [&str; 2] = [
        "cte.edgar_gleif.sec_entities#ck",
        "cte.edgar_gleif.sec_entities#tk",
    ];

    /// A node no `sql:` step produced: `fetch_edgar` is an `op:` step, and it
    /// belongs to no family, so it survives the collapse as itself.
    const OP_PRODUCED: &str = "file.edgar_gleif.build/edgar.parquet";

    fn node_ids(m: &ProtocolModel) -> BTreeSet<AssetId> {
        m.displayed_graph().nodes.keys().cloned().collect()
    }

    /// Send the `z` then `a` of the chord, returning whether the second press
    /// changed anything.
    fn za(m: &mut ProtocolModel) -> bool {
        m.feed_events(&[key(egui::Key::Z)]);
        m.feed_events(&[key(egui::Key::A)])
    }

    /// `za` with the cursor on a SQL-produced relation draws that statement's
    /// CTEs — **exactly** those two ids, nothing else added and nothing
    /// removed — and re-lays-out so the raster cache cannot serve the folded
    /// picture over the unfolded one.
    #[test]
    fn za_on_a_sql_produced_node_draws_that_steps_ctes() {
        let mut m = model();
        m.select_id(SQL_PRODUCED.to_string());
        assert!(!m.is_cte_expanded(), "the fold opens closed");

        let before = node_ids(&m);
        let before_edges = m.displayed_graph().edges.len();
        let before_gen = m.layout_gen();

        assert!(za(&mut m), "za opened the CTE fold");
        assert!(m.is_cte_expanded());

        let after = node_ids(&m);
        let added: Vec<&str> = after.difference(&before).map(String::as_str).collect();
        assert_eq!(added, STEP_CTES, "exactly the CTEs of that one statement");
        let removed: Vec<&str> = before.difference(&after).map(String::as_str).collect();
        assert!(
            removed.is_empty(),
            "the fold takes nothing away: {removed:?}"
        );

        // The re-routing is visible as lineage, not just as two loose boxes:
        // each CTE feeds the relation, and the files now reach it through them.
        let edges = &m.displayed_graph().edges;
        let has = |from: &str, to: &str| edges.iter().any(|e| e.from == from && e.to == to);
        for cte in STEP_CTES {
            assert!(has(cte, SQL_PRODUCED), "{cte} feeds the relation");
        }
        assert!(has("file.edgar_gleif.build/cik_lookup.txt", STEP_CTES[0]));
        assert!(has("file.edgar_gleif.build/edgar.parquet", STEP_CTES[1]));
        assert!(
            !has("file.edgar_gleif.build/cik_lookup.txt", SQL_PRODUCED),
            "the direct read is re-routed through the CTE, not drawn beside it"
        );
        assert_eq!(
            edges.len() as i64 - before_edges as i64,
            2,
            "four edges in, two dropped — the net delta the canvas gains"
        );

        // The raster cache key moved, so the canvas repaints.
        assert_ne!(m.layout_gen(), before_gen, "a re-layout was forced");
        assert!(
            m.layout().positions.contains_key(STEP_CTES[0])
                && m.layout().positions.contains_key(STEP_CTES[1]),
            "both CTEs are laid out, not merely in the graph"
        );
    }

    /// A second `za` on the same node puts them away again — the fold is a
    /// toggle, and closing it restores the boot canvas exactly.
    #[test]
    fn za_twice_on_a_sql_produced_node_puts_the_ctes_away() {
        let mut m = model();
        m.select_id(SQL_PRODUCED.to_string());
        let closed = node_ids(&m);
        let closed_edges = m.displayed_graph().edges.clone();

        assert!(za(&mut m), "opened");
        assert!(m.is_cte_expanded());
        assert!(za(&mut m), "closed");

        assert!(!m.is_cte_expanded(), "the second press closed the fold");
        assert_eq!(node_ids(&m), closed, "back to the graph it opened on");
        assert_eq!(
            &m.displayed_graph().edges,
            &closed_edges,
            "the re-routed edges came back too"
        );
        assert!(
            !m.displayed_graph()
                .nodes
                .keys()
                .any(|id| id.starts_with("cte.")),
            "no CTE survives a close"
        );
    }

    /// `za` on a node no `sql:` step produced does nothing at all: no state
    /// change, no re-layout, and — because the dispatch reports no change — no
    /// repaint for a keystroke with nothing behind it.
    #[test]
    fn za_on_a_node_no_sql_step_produced_changes_nothing() {
        let mut m = model();
        m.select_id(OP_PRODUCED.to_string());
        let before = node_ids(&m);
        let before_gen = m.layout_gen();

        assert!(!za(&mut m), "za reported no change");
        assert!(!m.is_cte_expanded(), "the fold stayed closed");
        assert!(!m.is_expanded(), "and no family opened either");
        assert_eq!(node_ids(&m), before, "the canvas is untouched");
        assert_eq!(m.layout_gen(), before_gen, "nothing was re-laid-out");
    }

    /// **The boot canvas is unchanged by this increment.** Nothing exploded
    /// until a key is pressed, so the two committed protocol baselines
    /// photograph the same graph they always did.
    ///
    /// This is the assertion behind the fold-closed default. If it ever fails,
    /// the CTEs have leaked into the built graph and both baselines are wrong —
    /// which is a defect in this code, not a reason to regenerate a golden.
    #[test]
    fn the_boot_canvas_draws_no_cte_nodes() {
        let m = model();
        assert!(!m.is_cte_expanded());
        assert!(!m.is_expanded());
        let boot: Vec<&AssetId> = m
            .displayed_graph()
            .nodes
            .keys()
            .filter(|id| id.starts_with("cte."))
            .collect();
        assert!(boot.is_empty(), "the boot canvas is CTE-free: {boot:?}");
        assert_eq!(
            m.displayed_graph(),
            &m.graph_collapsed,
            "and it is the collapsed graph itself, not a re-derived twin"
        );
        assert!(
            !m.layout().positions.keys().any(|id| id.starts_with("cte.")),
            "nothing exploded is laid out either"
        );
    }

    // -----------------------------------------------------------------------
    // No fold is armed that the canvas does not show.
    //
    // The two folds cannot be drawn at once (`graph_exploded` is built over the
    // collapsed canvas, `graph_full` is not exploded) and neither can be drawn
    // inside a drill scope. The tests below pin what happens where they meet —
    // a refusal or a close, never a flag left standing over a canvas that
    // contradicts it. Each names the two-keystroke sequence that used to reach
    // the bad state.
    // -----------------------------------------------------------------------

    /// The family tile on the crosswalk (`stage`/`shape` over its instances) —
    /// the only cursor position from which the family half of `za` fires.
    fn family_id(m: &ProtocolModel) -> AssetId {
        m.graph_collapsed
            .nodes
            .iter()
            .find(|(_, n)| n.kind == AssetKind::Family)
            .map(|(id, _)| id.clone())
            .expect("the crosswalk declares a parameterised family")
    }

    /// Whether any CTE node is on the canvas right now.
    fn ctes_on_canvas(m: &ProtocolModel) -> bool {
        m.displayed_graph()
            .nodes
            .keys()
            .any(|id| id.starts_with("cte."))
    }

    /// **Open the CTEs, then unfold a family: the fold CLOSES, it does not
    /// hide.** Two keystrokes from the state the pixel baselines photograph.
    ///
    /// The canvas swaps to the unfolded graph, which has no CTEs in it — that
    /// much is a real limit and was always going to happen. What must not
    /// happen is the flag surviving the swap: with `cte_expanded` still true,
    /// the *next* `za` on the same tile folds the family back and the two CTEs
    /// reappear, at a keystroke that means "close this family" and nothing
    /// else. The user pressed nothing that means "explode".
    #[test]
    fn unfolding_a_family_closes_the_cte_fold_rather_than_hiding_it() {
        let mut m = model();
        m.select_id(SQL_PRODUCED.to_string());
        assert!(za(&mut m), "the CTE fold opened");
        let exploded = m.displayed_graph().nodes.len();
        assert!(ctes_on_canvas(&m));
        assert!(m.folds_are_on_screen());

        // Move to the family tile and press the same chord.
        let family = family_id(&m);
        m.select_id(family);
        assert!(za(&mut m), "the family unfolded");
        assert!(m.is_expanded(), "the canvas is on the unfolded graph");
        assert!(
            m.displayed_graph().nodes.len() > exploded,
            "the unfolded graph is the bigger one ({} > {exploded})",
            m.displayed_graph().nodes.len()
        );
        assert!(!ctes_on_canvas(&m), "the CTEs are off the canvas");
        assert!(
            !m.is_cte_expanded(),
            "and the fold went off with them — not left armed behind the family"
        );
        assert!(m.folds_are_on_screen());

        // The keystroke that used to bring them back unbidden.
        assert!(za(&mut m), "the family folded again");
        assert!(!m.is_expanded());
        assert!(
            !ctes_on_canvas(&m),
            "closing the family returns the collapsed canvas, not the exploded one"
        );
        assert_eq!(
            m.displayed_graph(),
            &m.graph_collapsed,
            "it is the collapsed graph itself"
        );
    }

    /// **The CTE fold is refused while a family is unfolded**, rather than
    /// arming a flag `displayed_graph` will ignore.
    ///
    /// The reverse order of the test above: unfold first, then put the cursor
    /// on the SQL-produced relation. `graph_full` is not an exploded graph, so
    /// there is nothing this keystroke could draw — it reports no change, the
    /// layout generation does not move (so no repaint is requested for it), and
    /// the flag stays down.
    #[test]
    fn the_cte_fold_is_refused_while_a_family_is_unfolded() {
        let mut m = model();
        let family = family_id(&m);
        m.select_id(family);
        assert!(za(&mut m), "the family unfolded");
        assert!(m.is_expanded());

        m.select_id(SQL_PRODUCED.to_string());
        let before = node_ids(&m);
        let before_gen = m.layout_gen();

        assert!(!za(&mut m), "za reported no change");
        assert!(!m.is_cte_expanded(), "no fold was armed");
        assert!(m.is_expanded(), "and the family is untouched");
        assert_eq!(node_ids(&m), before, "the canvas did not move");
        assert_eq!(
            m.layout_gen(),
            before_gen,
            "no re-layout, so no repaint for a keystroke with nothing behind it"
        );
        assert!(m.folds_are_on_screen());
    }

    /// **`za` is refused outright inside a drill scope** — both halves of it.
    ///
    /// A scope draws an induced slice of the collapsed graph, so neither fold
    /// has a picture under one. The CTE half used to flip the flag and bump the
    /// layout generation here: a forced repaint of an unchanged canvas, zero
    /// feedback, and two CTE nodes waiting to appear on a later `Esc`.
    ///
    /// The family half is refused by the same guard, and the guard runs *before*
    /// [`ProtocolNav::toggle_fold`] — so the nav's own fold state is not
    /// mutated either. Widening back out proves it: the family is still folded.
    #[test]
    fn za_is_refused_inside_a_drill_scope() {
        let mut m = model();
        m.select_id(SQL_PRODUCED.to_string());
        assert!(m.feed_events(&[key(egui::Key::Enter)]), "drilled in");
        assert!(m.is_drilled());

        let scoped = node_ids(&m);
        let before_gen = m.layout_gen();

        // The CTE half: the cursor is on the SQL-produced relation.
        assert!(!za(&mut m), "za reported no change inside the scope");
        assert!(!m.is_cte_expanded(), "no fold was armed under the scope");
        assert_eq!(node_ids(&m), scoped, "the scope is unchanged");
        assert_eq!(m.layout_gen(), before_gen, "and nothing was re-laid-out");

        // The family half: same guard, and the nav is left alone.
        let family = family_id(&m);
        m.select_id(family);
        assert!(!za(&mut m), "the family half is refused too");
        assert!(!m.is_expanded());
        assert_eq!(m.layout_gen(), before_gen, "still no re-layout");

        assert!(m.feed_events(&[key(egui::Key::Escape)]), "widened back out");
        assert!(!m.is_drilled());
        assert!(
            !m.is_expanded(),
            "the refused family fold never reached the nav, so nothing unfolds on the way out"
        );
        assert!(!ctes_on_canvas(&m), "and no CTE appears on the way out");
        assert!(m.folds_are_on_screen());
    }

    /// **Drilling in closes an open CTE fold**, so widening back out does not
    /// hand the CTEs back at a keystroke that means "widen".
    ///
    /// The CTEs leave the screen at the `Enter` either way — the scope is
    /// induced over the collapsed graph. The only question is whether the flag
    /// leaves with them, and it does.
    #[test]
    fn drilling_in_closes_the_cte_fold_rather_than_suspending_it() {
        let mut m = model();
        m.select_id(SQL_PRODUCED.to_string());
        assert!(za(&mut m), "the CTE fold opened");
        assert!(ctes_on_canvas(&m));

        assert!(m.feed_events(&[key(egui::Key::Enter)]), "drilled in");
        assert!(!m.is_cte_expanded(), "the fold closed with the drill");
        assert!(!ctes_on_canvas(&m));
        assert!(m.folds_are_on_screen());

        assert!(m.feed_events(&[key(egui::Key::Escape)]), "widened back out");
        assert!(!m.is_drilled());
        assert!(
            !ctes_on_canvas(&m),
            "Esc widened the canvas; it did not re-explode it"
        );
        assert_eq!(m.displayed_graph(), &m.graph_collapsed);
    }

    /// The invariant itself, swept over every gesture that can reach **either**
    /// canvas fold.
    ///
    /// `cte_expanded` is true only while `graph_exploded` is what
    /// `displayed_graph` returns, `chain_contracted` only while
    /// `graph_contracted` is, and never both — checked after *every* keystroke of
    /// a script that walks the two folds into each other, into the family fold,
    /// into the drill scope, through the flow transpose and back out. A single
    /// arm re-ordered in `displayed_graph`, or a guard dropped from
    /// `toggle_fold`, reddens this without anyone having to think of the sequence
    /// again.
    ///
    /// Watched redden, one mutation: dropping `self.cte_expanded = false` from
    /// the chain arm of `toggle_canvas_fold` fails at the step that opens the
    /// chain fold over an open CTE fold, with both flags up and one picture.
    #[test]
    fn no_gesture_arms_a_canvas_fold_the_canvas_does_not_show() {
        let mut m = model();
        let family = family_id(&m);
        let chain = CHAIN_ABSORBED.to_string();
        // (cursor to place first, then the chord/keys to press there)
        let script: Vec<(Option<&str>, Vec<egui::Key>)> = vec![
            (Some(SQL_PRODUCED), vec![egui::Key::Z, egui::Key::A]),
            (Some(chain.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (Some(SQL_PRODUCED), vec![egui::Key::Z, egui::Key::A]),
            (Some(family.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (Some(chain.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (Some(SQL_PRODUCED), vec![egui::Key::Z, egui::Key::A]),
            (Some(family.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (Some(SQL_PRODUCED), vec![egui::Key::Z, egui::Key::A]),
            (None, vec![egui::Key::T]),
            (Some(chain.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (Some(SQL_PRODUCED), vec![egui::Key::Enter]),
            (Some(SQL_PRODUCED), vec![egui::Key::Z, egui::Key::A]),
            (Some(chain.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (Some(family.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (None, vec![egui::Key::Escape]),
            // A chain fold OPEN, then a family unfolded on top of it, with no
            // scope in the way. The pair at steps 12-13 above looks like this
            // and is not: a drill scope is active there, so the chain fold is
            // refused and the family arm has nothing to close. Without a clean
            // adjacency the family arm's `chain_contracted = false` can be
            // deleted and this sweep stays green, which is how the guard came
            // to be unheld.
            (Some(chain.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (Some(family.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (Some(family.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (Some(chain.as_str()), vec![egui::Key::Z, egui::Key::A]),
            (Some(SQL_PRODUCED), vec![egui::Key::Z, egui::Key::A]),
            (None, vec![egui::Key::Backspace]),
        ];
        for (step, (cursor, keys)) in script.iter().enumerate() {
            if let Some(id) = cursor {
                m.select_id((*id).to_string());
            }
            for k in keys {
                m.feed_events(&[key(*k)]);
                assert!(
                    m.folds_are_on_screen(),
                    "step {step}: a canvas fold is armed but the canvas does not \
                     show it (cte={}, chain={}, drilled={}, family={})",
                    m.is_cte_expanded(),
                    m.is_chain_contracted(),
                    m.is_drilled(),
                    m.is_expanded()
                );
            }
        }
        // The sweep ended somewhere real, not on an early bail-out.
        assert!(
            m.is_cte_expanded(),
            "the last chord landed on a cursor the fold answers to"
        );
        assert!(ctes_on_canvas(&m));
    }

    /// A node the chain fold absorbs: the external host one `op:` step fetches a
    /// single file from.
    ///
    /// **No `sql:` step produced it**, which is what makes it the chain arm of
    /// `za` rather than the CTE arm — the two are resolved in that order, and
    /// four of the eight nodes this fold absorbs *are* SQL-produced and reach the
    /// CTE arm first. It is also the shape the textbook strict-linear criterion
    /// refuses outright: a host has no producer of its own.
    const CHAIN_ABSORBED: &str =
        "source.edgar_gleif.https://openlake.meridian.online/edgar.parquet";

    /// The asset [`CHAIN_ABSORBED`]'s run ends at — the node the fold keeps.
    const CHAIN_TAIL: &str = "file.edgar_gleif.build/edgar.parquet";

    /// `za` on a node a run of single hand-offs would absorb contracts every such
    /// run on the canvas to the asset it ends at — and re-lays-out, so the raster
    /// cache cannot serve the uncontracted picture.
    #[test]
    fn za_on_a_chain_absorbed_node_contracts_the_runs() {
        let mut m = model();
        assert!(
            m.is_absorbed_by_a_chain(&CHAIN_ABSORBED.to_string()),
            "the fixture no longer has a chain at this cursor"
        );
        m.select_id(CHAIN_ABSORBED.to_string());
        assert!(!m.is_chain_contracted(), "the fold opens closed");

        let before = node_ids(&m);
        let before_gen = m.layout_gen();
        assert!(za(&mut m), "za contracted the chains");
        assert!(m.is_chain_contracted());

        let after = node_ids(&m);
        let gone: BTreeSet<&AssetId> = before.difference(&after).collect();
        assert_eq!(gone.len(), 8, "eight nodes absorbed: {gone:?}");
        assert!(
            after.difference(&before).next().is_none(),
            "the contraction invents no node"
        );

        // The merged node is the TAIL, whole. A joined label would have cost
        // +294 points of canvas width on this fixture (941 -> 1235), 217 past the
        // pane — a solved vertical scroll traded for a new horizontal one.
        assert!(after.contains(CHAIN_TAIL), "the tail survives the merge");
        assert_eq!(
            m.displayed_graph().nodes[CHAIN_TAIL].label,
            m.graph_collapsed.nodes[CHAIN_TAIL].label,
            "the merged node kept the tail's own label"
        );

        // And the picture actually shrank, along the reading axis and across it.
        let contracted = m.layout().clone();
        assert_ne!(m.layout_gen(), before_gen, "a re-layout was forced");
        assert!(za(&mut m), "za put them back");
        assert!(
            contracted.height < m.layout().height && contracted.width <= m.layout().width,
            "the contracted canvas ({}x{}) is not smaller than the canvas it \
             folded ({}x{})",
            contracted.width,
            contracted.height,
            m.layout().width,
            m.layout().height
        );
    }

    /// A second `za` on the same node puts the runs back — the fold is a toggle,
    /// and closing it restores the boot canvas exactly.
    #[test]
    fn za_twice_on_a_chain_absorbed_node_puts_the_runs_back() {
        let mut m = model();
        m.select_id(CHAIN_ABSORBED.to_string());
        let closed = node_ids(&m);
        let closed_edges = m.displayed_graph().edges.clone();

        assert!(za(&mut m), "contracted");
        assert!(m.is_chain_contracted());
        assert!(za(&mut m), "restored");

        assert!(!m.is_chain_contracted(), "the second press closed the fold");
        assert_eq!(node_ids(&m), closed, "back to the graph it opened on");
        assert_eq!(&m.displayed_graph().edges, &closed_edges);
        assert_eq!(
            m.displayed_graph(),
            &m.graph_collapsed,
            "it is the collapsed graph itself, not a re-derived twin"
        );
    }

    /// **The boot canvas absorbs nothing.** The eight nodes this fold takes are
    /// the intermediate build artefacts and the hosts they came from, which is
    /// exactly the provenance the protocol view exists to show — so the geometry
    /// is right and the default would be wrong.
    ///
    /// If this ever fails, the contraction has leaked into the built graph and
    /// every committed protocol baseline is photographing a canvas that lost its
    /// provenance — which is a defect in this code, not a reason to regenerate a
    /// golden.
    #[test]
    fn the_boot_canvas_contracts_no_chain() {
        let m = model();
        assert!(!m.is_chain_contracted());
        assert_eq!(
            m.displayed_graph(),
            &m.graph_collapsed,
            "the boot canvas is the collapsed graph itself"
        );
        for id in [
            "source.edgar_gleif.https://openlake.meridian.online/edgar.parquet",
            "file.edgar_gleif.build/edgar_gleif.parquet",
            "asset.edgar_gleif.sec_entities",
        ] {
            assert!(
                m.layout().positions.contains_key(id),
                "{id} is not drawn on the boot canvas"
            );
        }
    }

    /// **The two canvas folds are mutually exclusive, by closing rather than by
    /// stacking.** There is no graph in this struct that is both exploded and
    /// contracted, so a flag left standing behind the other's picture is the
    /// armed-but-invisible state the fold rules exist to prevent.
    ///
    /// Watched redden, one mutation: dropping `self.chain_contracted = false`
    /// from the CTE arm of `toggle_canvas_fold` fails here at *"the chain fold
    /// went down with the CTEs coming up"*.
    #[test]
    fn opening_one_canvas_fold_closes_the_other() {
        let mut m = model();
        m.select_id(CHAIN_ABSORBED.to_string());
        assert!(za(&mut m), "the chain fold opened");
        assert!(m.is_chain_contracted());

        m.select_id(SQL_PRODUCED.to_string());
        assert!(za(&mut m), "the CTE fold opened");
        assert!(m.is_cte_expanded());
        assert!(
            !m.is_chain_contracted(),
            "the chain fold went down with the CTEs coming up"
        );
        assert!(m.folds_are_on_screen());

        m.select_id(CHAIN_ABSORBED.to_string());
        assert!(za(&mut m), "the chain fold opened again");
        assert!(m.is_chain_contracted());
        assert!(!m.is_cte_expanded(), "and the CTE fold went down with it");
        assert!(!ctes_on_canvas(&m), "no CTE survives the swap");
        assert!(m.folds_are_on_screen());
    }

    /// **An absorbed node stays addressable everywhere except the canvas, and on
    /// the canvas its ring moves to the node it folded into.**
    ///
    /// This is the asymmetry already recorded against exploded CTEs, in reverse:
    /// there, the canvas gained nodes the outline never listed. Here the canvas
    /// loses nodes the outline still lists — and the outline, the nav and the
    /// inspector all walk the *uncontracted* graph, so an absorbed asset is still
    /// selectable, still walked to by `hjkl`, and still has an inspector.
    ///
    /// What is **not** covered, said plainly rather than implied: an absorbed
    /// node has no rectangle of its own while the fold is open, so its selection
    /// ring lands on its chain's tail rather than on it, and a canvas *click*
    /// cannot reach it at all. Resolving that properly needs the merged tile to
    /// carry its members' identities into hit-testing, which is a larger
    /// increment than this one.
    #[test]
    fn an_absorbed_node_stays_addressable_off_the_canvas() {
        let mut m = model();
        m.select_id(CHAIN_ABSORBED.to_string());
        assert!(za(&mut m), "the chain fold opened");

        let id = CHAIN_ABSORBED.to_string();
        assert!(
            !m.displayed_graph().nodes.contains_key(&id),
            "this fixture no longer absorbs the node the test is about"
        );
        assert!(m.has_selection(), "the inspector still answers for it");
        let facts = m.inspector();
        assert!(
            facts.present,
            "the inspector went empty on an absorbed node"
        );
        assert_eq!(
            facts.address, id,
            "and it answers for that node, not its tail"
        );
        assert!(
            m.outline().iter().any(|row| row.id == id),
            "the outline dropped the absorbed node"
        );
        assert!(
            m.nav.cursor() == Some(&id),
            "the nav cursor cannot rest on the absorbed node"
        );
        // The canvas half: no rectangle of its own, and the ring goes to the
        // asset the run produced rather than nowhere.
        assert!(!m.layout().positions.contains_key(&id));
        let site = m.selection_site().expect("a selection has a site");
        assert_ne!(site, id, "the ring would have been drawn nowhere");
        assert!(
            m.layout().positions.contains_key(&site),
            "the redirected ring lands on nothing either"
        );
    }

    /// CONTRACT LAST, and why the other order is not a preference.
    ///
    /// The sibling of `explode_then_collapse_keeps_the_ctes`, one pass further
    /// along. [`brightfield_protocol::explode_ctes`] resolves what a CTE body
    /// reads against the relation-shaped nodes of the graph it is handed; a
    /// contraction that has already absorbed one of those relations leaves it
    /// nothing to wire from, so the canvas draws a CTE box fed by nothing and
    /// keeps the direct edge the explode should have re-routed.
    ///
    /// Asserted on a fixture built for it, because the crosswalk cannot show it:
    /// the relation its one CTE-bearing step reads is a fan-in, so no chain
    /// touches it and both orders are pixel-identical there.
    #[test]
    fn contracting_before_the_explode_orphans_the_ctes() {
        const MANIFEST: &str = "\
name: ordering
engine: duckdb
steps:
  - name: stage
    sql: models/stage.sql
  - name: shape
    sql: models/shape.sql
";
        // `staged` is produced by `stage` and read by exactly one consumer, and
        // `shaped`'s CTE is that consumer — a chain, and the relation the CTE
        // body resolves against.
        let models = [
            (
                "models/stage.sql",
                "CREATE OR REPLACE TABLE staged AS SELECT * FROM raw_in;",
            ),
            (
                "models/shape.sql",
                "CREATE OR REPLACE TABLE shaped AS \
                 WITH keep AS (SELECT * FROM staged) SELECT * FROM keep;",
            ),
        ];
        let inputs = load_protocol_str(MANIFEST, &models).expect("the ordering protocol loads");
        let staged = "asset.ordering.staged";
        let cte = "cte.ordering.shaped#keep";
        let has = |g: &AssetGraph, from: &str, to: &str| {
            g.edges.iter().any(|e| e.from == from && e.to == to)
        };

        // Right order — the CTE is fed by the relation its body reads.
        assert!(
            has(&inputs.graph_exploded, staged, cte),
            "the shipped order does not wire the CTE, so the wrong order below \
             proves nothing"
        );

        // Wrong order — contract first, then explode.
        let sql_by_step: BTreeMap<StepId, String> = models
            .iter()
            .map(|(model, sql)| {
                let step = model
                    .trim_start_matches("models/")
                    .trim_end_matches(".sql")
                    .to_string();
                (step, (*sql).to_string())
            })
            .collect();
        let wrong = explode_ctes(
            &brightfield_protocol::contract_chains(&collapse_families(&inputs.graph_full)),
            &sql_by_step,
        );
        assert!(
            !wrong.nodes.contains_key(staged),
            "this fixture no longer contracts the relation the CTE reads"
        );
        assert!(
            wrong.nodes.contains_key(cte),
            "the wrong order still draws the box"
        );
        assert!(
            !has(&wrong, staged, cte),
            "...but nothing feeds it: the contraction had already absorbed the \
             relation its body reads, so the explode had nothing to wire from"
        );
    }

    /// EXPLODE THEN COLLAPSE, and why the other order is not a preference.
    ///
    /// The protocol below has a parameterised family of four `sql:` steps
    /// (`stage`/`shape` over two instances) and one step outside it whose CTE
    /// reads a relation the family produces.
    ///
    /// Right order — the composition is closed. The collapse still detects the
    /// family (the explode adds nodes and edges, never seams, so detection sees
    /// the same input), the family members' CTEs fold away into the tile with
    /// the rest of their steps' assets, and the outside step's CTE survives
    /// **wired to the tile**.
    ///
    /// Wrong order — the outside CTE is orphaned. Collapsing first deletes the
    /// relation the CTE body reads, so the explode's read-resolution finds
    /// nothing to wire from, the direct tile→product edge is never re-routed,
    /// and the canvas draws a CTE box whose input came from nowhere. Asserted
    /// here, because the crosswalk cannot show it: its one CTE-bearing step
    /// belongs to no family, so both orders are pixel-identical there and the
    /// mistake would ship invisible.
    #[test]
    fn explode_then_collapse_keeps_the_ctes() {
        const MANIFEST: &str = "\
name: composition
engine: duckdb
steps:
  - name: stage_a
    sql: models/stage_a.sql
  - name: shape_a
    sql: models/shape_a.sql
  - name: stage_b
    sql: models/stage_b.sql
  - name: shape_b
    sql: models/shape_b.sql
  - name: outside
    sql: models/outside.sql
";
        let models = [
            (
                "models/stage_a.sql",
                "CREATE OR REPLACE TABLE staged_a AS \
                 WITH pick_a AS (SELECT * FROM src_a) SELECT * FROM pick_a;",
            ),
            (
                "models/shape_a.sql",
                "CREATE OR REPLACE TABLE shaped_a AS \
                 WITH trim_a AS (SELECT * FROM staged_a) SELECT * FROM trim_a;",
            ),
            (
                "models/stage_b.sql",
                "CREATE OR REPLACE TABLE staged_b AS \
                 WITH pick_b AS (SELECT * FROM src_b) SELECT * FROM pick_b;",
            ),
            (
                "models/shape_b.sql",
                "CREATE OR REPLACE TABLE shaped_b AS \
                 WITH trim_b AS (SELECT * FROM staged_b) SELECT * FROM trim_b;",
            ),
            (
                "models/outside.sql",
                "CREATE OR REPLACE TABLE outside_out AS \
                 WITH keep AS (SELECT * FROM shaped_a) SELECT * FROM keep;",
            ),
        ];
        let inputs = load_protocol_str(MANIFEST, &models).expect("the composition protocol loads");
        let tile = "family.composition.stage+shape";
        let outside_cte = "cte.composition.outside_out#keep";
        let member_cte = "cte.composition.staged_a#pick_a";

        // The collapse still detects the family over the exploded graph.
        assert_eq!(
            inputs.graph_exploded.nodes[tile].family_count,
            Some(2),
            "the explode did not disturb family detection"
        );

        // A family member's CTE folds into the tile with everything else that
        // step owns — the detail inside a closed tile stays inside it.
        assert!(
            !inputs.graph_exploded.nodes.contains_key(member_cte),
            "a collapsed member's CTE is not loose on the canvas"
        );

        // The exploded canvas is the collapsed canvas plus exactly the CTEs
        // that survived the fold.
        let collapsed: BTreeSet<&AssetId> = inputs.graph_collapsed.nodes.keys().collect();
        let exploded: BTreeSet<&AssetId> = inputs.graph_exploded.nodes.keys().collect();
        let gained: Vec<&str> = exploded
            .difference(&collapsed)
            .map(|id| id.as_str())
            .collect();
        assert_eq!(gained, [outside_cte], "only the outside CTE");
        assert!(
            collapsed.difference(&exploded).next().is_none(),
            "the explode never removes a node the collapsed canvas had"
        );

        // ...and it is real lineage: fed by the tile, feeding the product, with
        // the direct edge it re-routed gone.
        let has = |g: &AssetGraph, from: &str, to: &str| {
            g.edges.iter().any(|e| e.from == from && e.to == to)
        };
        let product = "asset.composition.outside_out";
        assert!(has(&inputs.graph_exploded, tile, outside_cte));
        assert!(has(&inputs.graph_exploded, outside_cte, product));
        assert!(
            !has(&inputs.graph_exploded, tile, product),
            "the read is re-routed through the CTE"
        );

        // The other order, on the same inputs: an orphan.
        let sql_by_step: BTreeMap<StepId, String> = models
            .iter()
            .map(|(model, sql)| {
                let step = model
                    .trim_start_matches("models/")
                    .trim_end_matches(".sql")
                    .to_string();
                (step, (*sql).to_string())
            })
            .collect();
        let wrong = explode_ctes(&collapse_families(&inputs.graph_full), &sql_by_step);
        assert!(
            wrong.nodes.contains_key(outside_cte),
            "the wrong order still draws the box"
        );
        assert!(
            !has(&wrong, tile, outside_cte),
            "...but nothing feeds it: collapsing first deleted the relation \
             its body reads, so the explode had nothing to wire from"
        );
        assert!(
            has(&wrong, tile, product),
            "...and the direct edge it should have re-routed is still there"
        );
    }

    /// The flow toggle transposes the layout: vertical bounds the canvas's width
    /// strictly below the horizontal render's, and horizontal is wider than tall.
    ///
    /// # What this used to assert, and why it stopped being true
    ///
    /// It asserted `vh > vw` — *"vertical is taller than wide"* — on this
    /// fixture, and that clause was being held up by a defect. The collapsed
    /// crosswalk is 10 ranks deep and its widest rank is 954 points of cards, so
    /// the two axes were within 86 points of each other; the 48-point thickness
    /// floor under every rank contributed 118 of those, and it was reserving room
    /// for nothing (see `LANE_EXTENT` in `brightfield-protocol`'s layout module).
    /// Take the floor away at the *old* pitch and the vertical canvas is
    /// 1034 × 1002 — already wider than it is tall, before any gap moves.
    ///
    /// So the clause was not an invariant of the transpose. It was an accident of
    /// this one graph plus 118 points of padding, and no value of `col_gap` on
    /// the design system's spacing ladder can buy it back. What the transpose
    /// actually promises is the thing that motivated it — the long axis moves
    /// onto natural scroll — and that is what is asserted here: **vertical is
    /// strictly narrower than horizontal** (1018 against 1950), and horizontal
    /// puts its long axis across. A graph whose widest rank is wider than it is
    /// deep is vertically wide, and saying otherwise would be a false claim about
    /// the picture the user sees.
    ///
    /// `vertical_bounds_width_to_the_widest_layer` in the layout crate holds the
    /// same property, and its fixture is deep enough that the taller-than-wide
    /// clause is genuinely true there; it is untouched.
    #[test]
    fn flow_toggle_transposes_layout() {
        let mut m = model();
        assert_eq!(m.flow(), Flow::Vertical);
        let (vw, vh) = (m.layout().width, m.layout().height);
        m.toggle_flow();
        assert_eq!(m.flow(), Flow::Horizontal);
        let (hw, hh) = (m.layout().width, m.layout().height);
        assert!(hw > hh, "horizontal is wider than tall: {hw}x{hh}");
        assert!(
            vw < hw,
            "vertical does not bound the width below horizontal's: \
             {vw}x{vh} against {hw}x{hh} — the transpose bought nothing"
        );
        assert!(
            vh > hh,
            "the transpose did not move extent onto the scroll axis: \
             {vw}x{vh} against {hw}x{hh}"
        );
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

    // -- Run-state: the data-honesty channel --------------------------------

    /// A seven-step chain contract, every step a recorded success except s4
    /// (a hash-clean skip — the engine's own freshness proof).
    fn chain_view() -> brightfield_protocol::ContractView {
        let mut assets = String::new();
        let mut steps = String::new();
        for i in 1..=7 {
            let consumed_by = if i < 7 {
                format!(r#"["s{}"]"#, i + 1)
            } else {
                "[]".to_string()
            };
            assets.push_str(&format!(
                r#"{}{{ "id": "table:a{i}", "name": "a{i}", "kind": "table",
                     "produced_by": "s{i}", "consumed_by": {consumed_by} }}"#,
                if i > 1 { "," } else { "" },
            ));
            let reads = if i > 1 {
                format!(r#"["a{}"]"#, i - 1)
            } else {
                "[]".to_string()
            };
            let status = if i == 4 {
                r#"{ "state": "skipped", "skip_reason": "hash_clean" }"#
            } else {
                r#"{ "state": "success" }"#
            };
            steps.push_str(&format!(
                r#"{}{{ "name": "s{i}", "kind": "sql",
                     "sql": {{ "model_path": "models/s{i}.sql",
                               "sql_hash": "h{i}",
                               "statements": [ {{ "produces": ["a{i}"], "reads": {reads} }} ] }},
                     "status": {status} }}"#,
                if i > 1 { "," } else { "" },
            ));
        }
        let json = format!(
            r#"{{ "contract_version": "b4/1",
                  "run": {{ "run_id": "r1", "protocol": {{ "name": "chain" }},
                            "outcome": "success" }},
                  "assets": [{assets}], "steps": [{steps}] }}"#
        );
        brightfield_protocol::view_from_contract_bytes(json.as_bytes()).expect("chain view")
    }

    /// The recorded run-state for a step of the chain view, composed the way
    /// the inspector composes it: state + typed skip reason, ingested.
    fn recorded(view: &brightfield_protocol::ContractView, step: &str) -> RunState {
        let s = &view.steps[step];
        run_state_recorded(Some(s.state), s.skip_reason_kind())
    }

    /// THE honesty test: editing an upstream step labels every downstream
    /// preview stale — not merely re-rendered — while the contract on disk
    /// still records success, because nothing ran. The un-edited upstream
    /// step keeps its recorded verdict.
    #[test]
    fn editing_an_upstream_step_labels_downstream_previews_stale_without_running() {
        let view = chain_view();
        let mut edits = EditOverlay::default();
        assert!(edits.is_empty(), "the read-only view starts un-edited");
        edits.mark_edited("s2", &view.graph);

        // The edited step itself: stale by its own edit.
        assert_eq!(
            edits.apply("s2", recorded(&view, "s2")),
            RunState::StaleOwnEdit
        );

        // Every dependent: the CONTRACT still says fresh (nothing ran — that
        // is the point), and the REPRESENTATION refuses to present it as
        // current anyway. This includes s4, whose hash-clean skip was proof
        // of freshness against the pipeline the run saw, not this one.
        for step in ["s3", "s4", "s5", "s6", "s7"] {
            let contract_says = recorded(&view, step);
            assert_eq!(
                contract_says,
                RunState::Fresh,
                "{step}: the recorded verdict is still fresh — nothing ran"
            );
            let labelled = edits.apply(step, contract_says);
            assert_eq!(
                labelled,
                RunState::StaleUpstream,
                "{step}: downstream of the edit, so labelled stale"
            );
            assert!(
                !labelled.is_current(),
                "{step}: a stale preview may never claim current"
            );
            assert_ne!(
                labelled.label(),
                RunState::Fresh.label(),
                "{step}: the stale label is visibly different words, not a re-render"
            );
        }

        // Upstream of the edit: untouched, and honestly still fresh.
        assert_eq!(edits.apply("s1", recorded(&view, "s1")), RunState::Fresh);
    }

    /// The ingestion rules: success reads fresh; a skip reads fresh only on
    /// the engine's typed freshness proof; an unproven skip, an unknown
    /// state and a missing record all refuse the claim; failure reads failed.
    #[test]
    fn run_state_is_ingested_from_recorded_state_and_typed_skip_reason() {
        use StepState::{Failed, Skipped, Success, Unknown};
        assert_eq!(run_state_recorded(Some(Success), None), RunState::Fresh);
        assert_eq!(
            run_state_recorded(Some(Skipped), Some(SkipReason::HashClean)),
            RunState::Fresh,
            "a hash-clean skip is the engine's own proof of freshness"
        );
        assert_eq!(
            run_state_recorded(Some(Skipped), Some(SkipReason::PreconditionFresh)),
            RunState::Fresh
        );
        assert_eq!(
            run_state_recorded(Some(Skipped), None),
            RunState::NeverRun,
            "a skip without a typed reason proves nothing — never green"
        );
        assert_eq!(
            run_state_recorded(Some(Skipped), Some(SkipReason::Other)),
            RunState::NeverRun
        );
        assert_eq!(run_state_recorded(Some(Failed), None), RunState::Failed);
        assert_eq!(run_state_recorded(Some(Unknown), None), RunState::NeverRun);
        assert_eq!(run_state_recorded(None, None), RunState::NeverRun);
    }

    /// Overlay precedence: an own edit beats everything (including a prior
    /// failure — the definition has moved on); a recorded failure stays
    /// visible under an upstream edit; an empty overlay changes nothing.
    #[test]
    fn edit_overlay_precedence_keeps_the_most_recent_fact_first() {
        let view = chain_view();
        let mut edits = EditOverlay::default();
        edits.mark_edited("s2", &view.graph);

        assert_eq!(
            edits.apply("s2", RunState::Failed),
            RunState::StaleOwnEdit,
            "the user's own edit is the newest fact about s2"
        );
        assert_eq!(
            edits.apply("s3", RunState::Failed),
            RunState::Failed,
            "a failure downstream of the edit stays visible — it is the more actionable signal"
        );
        assert_eq!(
            edits.apply("s3", RunState::NeverRun),
            RunState::StaleUpstream
        );

        let empty = EditOverlay::default();
        for state in RunState::ALL {
            assert_eq!(
                empty.apply("s3", state),
                state,
                "an empty overlay keeps the contract's word"
            );
        }
    }

    /// The inspector's facts carry the ingested pair, and composing them
    /// yields the two-channel disagreement that motivates the second field:
    /// s4 is "skipped" as execution and fresh as data.
    #[test]
    fn inspector_facts_compose_to_the_data_channel() {
        let view = chain_view();
        let graph = collapse_families(&view.graph);
        let statuses = view.seam_statuses();
        let a4 = graph
            .nodes
            .keys()
            .find(|k| k.ends_with(".a4"))
            .expect("a4 node")
            .clone();
        let facts = brightfield_protocol::inspector_for(
            &graph,
            &view.assets,
            &view.steps,
            &statuses,
            Some(&a4),
        );
        assert_eq!(
            facts.status,
            SeamStatus::Skipped,
            "execution channel: skipped"
        );
        assert_eq!(
            run_state_recorded(facts.step_state, facts.skip_reason),
            RunState::Fresh,
            "data channel: the typed hash-clean skip is proof of freshness"
        );
    }
}
