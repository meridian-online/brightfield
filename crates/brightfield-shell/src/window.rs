//! One window, both views.
//!
//! [`MeridianApp`] is the whole of the product's UI: a
//! [`Workspace`] holding a dock tree per
//! view, the chart view's document and items, the protocol view's document and
//! items, and the one top bar drawn above whichever of the two is active.
//! [`MeridianApp::draw`] is the single frame source every tier shares — the
//! live `eframe` window, the headless `brightfield-shot` binary and the
//! `egui_kittest` pixel tier all call it, so what an agent sees in a PNG is
//! what ships.
//!
//! # What this replaces
//!
//! Two `eframe::App` implementations over two disjoint shells, chosen at boot
//! by sniffing the spec. A dashboard opened one window; a protocol manifest
//! opened a different one. The chart and the DAG could not be on screen
//! together, and the fork was load-bearing in four places at once — the live
//! binary, both halves of the capture path, and the shot binary — so no one of
//! them could be collapsed on its own.
//!
//! # Why two documents rather than one
//!
//! [`ChartDoc`] and [`ProtocolDoc`] are unrelated types and stay that way. The
//! workbench never asked them to meet: `Workspace` carries no document at all,
//! and `egui_tiles::Tree::ui` takes `&mut dyn Behavior<PaneKey>`, so a
//! `PaneChrome<'_, ChartDoc>` and a `PaneChrome<'_, ProtocolDoc>` are both
//! accepted by the same tree method with no shared type between them. This app
//! therefore holds the two `(document, items)` pairs as sibling fields and
//! matches on the active view when it builds the behaviour.
//!
//! The alternatives were considered and rejected in the same breath. An
//! `enum Doc { Chart(..), Protocol(..) }` compiles, but every one of the six
//! `Item` impls would then have to match the enum and silently no-op on the
//! wrong variant — and `Item::subject` has no error channel, so a mismatch
//! would return a blank `Subject` rather than fail. A `dyn AnyDoc` compiles at
//! the workbench boundary (every bound there is `D: ?Sized` already) but means
//! rewriting all six impls to downcast through `Any`, which converts a
//! compile-time guarantee into a runtime panic. Both cost more than the eight
//! duplicated lines the match arms cost, and both give up the property that
//! makes the contract worth having: a pane is handed exactly one document, and
//! it is the document of its own view.
//!
//! # Both views are always loaded
//!
//! `Workspace::new` requires a tree for every [`ViewKind`], and the switcher
//! offers every view whether or not the spec on the command line described it.
//! So the view the spec did not describe boots on its *empty* document —
//! `Composed::empty()` or `ProtocolInputs::empty()` — which every pane in both
//! views already answers with a real empty state, gated by
//! [`brightfield_workbench::audit`] in the contract tests. It is a live
//! document with a canvas host behind it, not a headless one, so it can raster
//! the moment it gains content.

use std::collections::BTreeSet;

use egui::containers::{CentralPanel, Panel};
use egui_tiles::{Container, Tile};

use brightfield_protocol::layout::{Flow, Layout};
use brightfield_workbench::behavior::{TAB_BAR_HEIGHT, TILE_GAP};
use brightfield_workbench::workspace::{tabs_holding, tile_of};
use brightfield_workbench::{chrome, ItemMap, PaneChrome, PaneKey, Request, ViewKind, Workspace};

use meridian_design::{semantic, spacing};

use crate::app::{chart_registry, ChartDoc, CONTROLS_SHARE};
use crate::canvas::EguiCanvasHost;
use crate::design::{self, Mode};
use crate::pipeline::{compose_spec, Composed};
use crate::protocol::{
    hint_ui, load_protocol_offline, protocol_registry, ui_font, ProtocolDoc, ProtocolInputs,
    ProtocolModel, CANVAS, INSPECTOR_SHARE, OUTLINE_SHARE, STEPS,
};

// ---------------------------------------------------------------------------
// The window's own chrome budget.
// ---------------------------------------------------------------------------

/// The height of one of the window's own bands — the top bar, and the key-hint
/// bar the protocol view draws — in logical points: a grid row for its content,
/// with the panel's own vertical padding above and below.
///
/// **Declared, not measured.** A content-sized `Panel` is a component that does
/// not expose its height until a frame has run, and the window size is computed
/// before any frame exists — `main.rs` sizes the window from it. The previous
/// answer to that on the chart side was to guess a spacing constant that
/// happened to look close, and the guess was 17 logical points short, so the
/// window clipped the bottom of its own chart. Pinning the band with
/// [`Panel::exact_size`](egui::containers::Panel::exact_size) turns the guess
/// into a fact.
///
/// **One measure for both bands, and it has to be at least as tall as either
/// band's content.** `exact_size` does not shrink a band to fit — it clamps the
/// rect the panel *reports*, while the content goes on occupying what it wanted,
/// so a band pinned shorter than its content hands its sibling one point less
/// than the number the window arithmetic subtracted. Measured: the hint row is
/// 29 logical points, so a 28-point pin left the dock a point taller than the
/// budget said and the DAG a point of unexplained slack. A band whose content
/// grows past this clips rather than pushing the dock around, which is the safe
/// direction for a row of key hints.
pub const BAR_HEIGHT: f32 = spacing::ROW_GRID + 2.0 * spacing::SPACE_2;

/// The inset between the window edge and the dock.
///
/// The value `egui::Frame::central_panel` would have used anyway, said on the
/// spacing ladder and passed in explicitly, for the same reason as [`BAR_HEIGHT`]:
/// a term the window arithmetic has to subtract cannot be a number that lives
/// only inside egui.
pub const DOCK_INSET: f32 = spacing::SPACE_4;

/// The natural window size in logical points for a composed dashboard: exactly
/// big enough that the chart pane's content box fits the dashboard's raster,
/// with every term of the chrome budget derived from the thing that consumes it.
///
/// Reading outwards from the raster, in each axis:
///
/// - the pane's content box is its tile inset by
///   [`chrome::pane_content_inset`] on all four sides, and, vertically, below a
///   [`chrome::header_band_height`] header band;
/// - the chart's tile is `1 - CONTROLS_SHARE` of the dock's width, after the
///   [`TILE_GAP`] between it and the rail is taken out;
/// - the dock is the window inset by [`DOCK_INSET`], below the top bar.
///
/// The charts view draws no hint bar, so the window gives up one
/// [`BAR_HEIGHT`] rather than two. That is held to rather than remembered:
/// `the_window_it_asks_for_fits_the_raster_it_presents` lays a real frame out at
/// this size and reads the box the dock gave the chart pane, so a hint bar
/// appearing on this view would take its height out of that box and redden.
///
/// Every one of those is read from the component that draws it. The pair of
/// hand-tuned spacing constants this replaces (`SPACE_8` across, `SPACE_9 +
/// SPACE_8` down) were 5.6pt short across and 17pt short down, which is why the
/// presented raster overflowed the pane's clip rect and the last seventeen rows
/// of the chart never reached the window.
///
/// Rounded **up** to whole logical points: the share is an `f32` division, and a
/// window a quarter of a point short would clip a row of the raster just as
/// surely as one seventeen points short.
#[must_use]
pub fn chart_window_size(composed: &Composed) -> (f32, f32) {
    let centre = 1.0 - CONTROLS_SHARE;
    let inset = chrome::pane_content_inset();

    let tile_w = composed.width as f32 + 2.0 * inset;
    let w = (tile_w / centre + TILE_GAP + 2.0 * DOCK_INSET).ceil();

    let tile_h = composed.height as f32 + 2.0 * inset + chrome::header_band_height();
    let h = (tile_h + 2.0 * DOCK_INSET + BAR_HEIGHT).ceil();

    (w, h)
}

/// The natural window size in logical points for a laid-out asset graph, read
/// outwards from the DAG exactly as [`chart_window_size`] is read outwards from
/// the dashboard.
///
/// The two differences from the chart's are both properties of this view's
/// declared shape rather than adjustments:
///
/// - the canvas sits between **two** rails, so two [`TILE_GAP`]s come out of the
///   dock's width and the centre share is what both rails leave;
/// - the canvas is a centre *tab*, so its header band is suppressed — the strip
///   already names it — and the strip's [`TAB_BAR_HEIGHT`] takes that band's
///   place in the vertical budget;
/// - this view draws a key-hint bar under the dock as well as the top bar over
///   it, so the window gives up two [`BAR_HEIGHT`]s rather than one.
///
/// What this replaces was a different kind of number altogether: `l.height +
/// 130.0`, floored at `1100.0` across and clamped to `680.0..=1600.0` down.
/// Nothing derived the 130, no test could see it, and the canvas pane scrolls
/// rather than clips, so a budget several points short opened the graph
/// part-scrolled in silence. The clamps are gone with it: a window that lies
/// about fitting is this crate's defect, and a window larger than the display
/// is the compositor's to resolve.
#[must_use]
pub fn protocol_window_size(layout: &Layout) -> (f32, f32) {
    let centre = 1.0 - OUTLINE_SHARE - INSPECTOR_SHARE;
    let inset = chrome::pane_content_inset();

    let tile_w = layout.width as f32 + 2.0 * inset;
    let w = (tile_w / centre + 2.0 * TILE_GAP + 2.0 * DOCK_INSET).ceil();

    let tile_h = layout.height as f32 + 2.0 * inset + TAB_BAR_HEIGHT;
    let h = (tile_h + 2.0 * DOCK_INSET + 2.0 * BAR_HEIGHT).ceil();

    (w, h)
}

// ---------------------------------------------------------------------------
// Boot — which document the spec describes, and which view opens on it.
// ---------------------------------------------------------------------------

/// What a window opens with: both documents' contents, and which view is drawn
/// first.
///
/// One value rather than two entry points, because the window holds both views
/// whichever spec was named. The view the spec did not describe gets its empty
/// document, and that is a state the product has to render correctly anyway —
/// it is what a first run over a spec that declares nothing looks like.
pub struct Boot {
    /// The view the window opens on.
    pub view: ViewKind,
    /// The chart view's dashboard.
    pub composed: Composed,
    /// The protocol view's graph and steps.
    pub protocol: ProtocolInputs,
    /// The protocol view's reading axis.
    pub flow: Flow,
    /// A dotted asset id to select before the first frame, for a scripted
    /// capture that needs a cursor to act on. Protocol view only.
    pub focus: Option<String>,
}

impl Boot {
    /// Open on the charts view over `composed`, with an empty protocol.
    #[must_use]
    pub fn charts(composed: Composed) -> Self {
        Self {
            view: ViewKind::Charts,
            composed,
            protocol: ProtocolInputs::empty(),
            flow: Flow::Vertical,
            focus: None,
        }
    }

    /// Open on the protocol view over `inputs`, with an empty dashboard.
    #[must_use]
    pub fn protocol(inputs: ProtocolInputs, flow: Flow, focus: Option<String>) -> Self {
        Self {
            view: ViewKind::Protocol,
            composed: Composed::empty(),
            protocol: inputs,
            flow,
            focus,
        }
    }

    /// Read `spec` and load whichever document it describes.
    ///
    /// **The one place a spec is classified.** It used to be two — the live
    /// binary and the shot binary each sniffed the file and each branched into
    /// its own shell — and the two branches then had to agree about an
    /// environment gate, a window size and a summary line that neither shared.
    ///
    /// # Errors
    /// A message if the file cannot be read, if it is a protocol manifest
    /// without `BRIGHTFIELD_PROTOCOL_OFFLINE` set, or if the pipeline rejects
    /// it.
    pub fn open(spec: &str, flow: Flow, focus: Option<String>) -> Result<Self, String> {
        let text = std::fs::read_to_string(spec).map_err(|e| format!("read {spec}: {e}"))?;
        if brightfield_protocol::is_protocol_manifest(&text) {
            if std::env::var("BRIGHTFIELD_PROTOCOL_OFFLINE").is_err() {
                return Err(format!(
                    "{spec} is a Protocol manifest, not an emitted Protocol+Run contract. \
                     To render it offline without a run, set BRIGHTFIELD_PROTOCOL_OFFLINE=1."
                ));
            }
            return Ok(Self::protocol(load_protocol_offline(spec)?, flow, focus));
        }
        Ok(Self::charts(compose_spec(spec)?))
    }

    /// The window this boot asks for, in logical points — the **boot view's**
    /// natural size.
    ///
    /// One window means one size, and the two views want very different ones.
    /// The boot view's is the answer because it is the only one that is a fact
    /// at the moment the window is created: the other view's document is
    /// usually empty, and sizing to the larger of the two would open a window
    /// mostly full of an empty state nobody asked for. Switching views does not
    /// resize — the user's window is theirs once it exists, and both views
    /// reflow or scroll inside whatever they are given.
    ///
    /// Answered here rather than on [`MeridianApp`] because a window has to be
    /// sized before it can be created, and the app cannot be built until eframe
    /// has handed over a device.
    #[must_use]
    pub fn window_size(&self) -> (f32, f32) {
        match self.view {
            ViewKind::Charts => chart_window_size(&self.composed),
            ViewKind::Protocol => {
                protocol_window_size(&ProtocolModel::boot_layout(&self.protocol, self.flow))
            }
        }
    }

    /// The window title: the boot view's subject.
    #[must_use]
    pub fn title(&self) -> String {
        match self.view {
            ViewKind::Charts => self
                .composed
                .title
                .clone()
                .unwrap_or_else(|| "Brightfield".to_string()),
            ViewKind::Protocol => format!("Protocol · {}", self.protocol.protocol),
        }
    }

    /// One line describing what was loaded, for the binaries' stderr.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.view {
            ViewKind::Charts => format!(
                "composed {}x{} dashboard",
                self.composed.width, self.composed.height
            ),
            ViewKind::Protocol => format!(
                "protocol {} ({} collapsed / {} full nodes, {} steps, {:?} flow)",
                self.protocol.protocol,
                self.protocol.graph_collapsed.nodes.len(),
                self.protocol.graph_full.nodes.len(),
                self.protocol.sheet_rows.len(),
                self.flow,
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// MeridianApp
// ---------------------------------------------------------------------------

/// What one frame of the top bar was asked to do, and where it put the controls
/// that could ask.
#[derive(Default)]
struct TopBar {
    /// The view the switcher was clicked to.
    switch: Option<ViewKind>,
    /// Whether the protocol view's flow toggle was pressed.
    toggle_flow: bool,
    /// Each switcher control's rect — see [`MeridianApp::switcher`].
    switcher: Vec<(ViewKind, egui::Rect)>,
}

/// The chart view's half: its document and its live items.
struct ChartView {
    doc: ChartDoc,
    items: ItemMap<ChartDoc>,
}

/// The protocol view's half: its document and its live items.
struct ProtocolView {
    doc: ProtocolDoc,
    items: ItemMap<ProtocolDoc>,
}

/// The window.
pub struct MeridianApp {
    ws: Workspace,
    charts: ChartView,
    protocol: ProtocolView,
    mode: Mode,
    fonts_installed: bool,
    /// Where the top bar's view switcher drew each view's control, in
    /// window-space logical points — empty until a frame has been laid out.
    ///
    /// Recorded for the reason `ChartDoc::overlay_checkbox` is: the test that
    /// proves a user can actually reach the other view has to *click* this
    /// control, and a coordinate typed against a layout nothing derived it from
    /// lands today and goes on being green while clicking empty bar the first
    /// time a label or a padding moves.
    switcher: Vec<(ViewKind, egui::Rect)>,
}

impl MeridianApp {
    /// Build the window over `boot`, rastering each view through its own host.
    ///
    /// Two hosts rather than one, one per document, because a document owns the
    /// canvas it rasters into — that is the rule the whole item contract hangs
    /// off. Both are built from the same wgpu device: `EguiCanvasHost` holds
    /// `Arc` handles, so a second one costs a `VelloRenderer` and nothing else.
    #[must_use]
    pub fn new(
        boot: Boot,
        chart_host: EguiCanvasHost,
        protocol_host: EguiCanvasHost,
        mode: Mode,
    ) -> Self {
        let doc = ChartDoc::new(boot.composed, chart_host);
        let model = ProtocolModel::new(boot.protocol, boot.flow);
        Self::assemble(
            boot.view,
            boot.focus,
            doc,
            ProtocolDoc::new(model, protocol_host),
            mode,
        )
    }

    /// The same window over documents with no device behind them.
    ///
    /// Everything except the two rasters is a pure function of the loaded
    /// documents, so this lays out identically to [`Self::new`] — each canvas
    /// pane reserves and paints nothing, and every rect around it is the same.
    /// That is what makes the window arithmetic assertable without a GPU: a
    /// test can run a real frame through `egui::Context::run_ui` and read the
    /// box the dock gave a canvas pane back out of the document.
    #[must_use]
    pub fn headless(boot: Boot, mode: Mode) -> Self {
        let doc = ChartDoc::headless(boot.composed);
        let model = ProtocolModel::new(boot.protocol, boot.flow);
        Self::assemble(
            boot.view,
            boot.focus,
            doc,
            ProtocolDoc::headless(model),
            mode,
        )
    }

    fn assemble(
        view: ViewKind,
        focus: Option<String>,
        chart_doc: ChartDoc,
        mut protocol_doc: ProtocolDoc,
        mode: Mode,
    ) -> Self {
        // Both views' vocabularies, published before any layout file could be
        // read. Idempotent, and both are needed whichever view boots: a
        // `PaneKey` naming a pane of the view that did not boot has to
        // deserialise too, or a saved layout loads as corrupt.
        crate::app::publish_item_ids();
        crate::protocol::publish_item_ids();

        if let Some(id) = focus {
            protocol_doc.model.select_id(id);
        }

        let charts = chart_registry();
        let protocol = protocol_registry();
        let trees = [
            (ViewKind::Charts, charts.default_tree()),
            (ViewKind::Protocol, protocol.default_tree()),
        ]
        .into_iter()
        .collect();

        let mut ws = Workspace::new(trees);
        ws.set_active(view);

        Self {
            ws,
            charts: ChartView {
                doc: chart_doc,
                items: charts.instantiate(),
            },
            protocol: ProtocolView {
                doc: protocol_doc,
                items: protocol.instantiate(),
            },
            mode,
            fonts_installed: false,
            switcher: Vec::new(),
        }
    }

    /// The view currently drawn.
    #[must_use]
    pub fn active(&self) -> ViewKind {
        self.ws.active()
    }

    /// The window title: the active view's subject. [`Boot::title`] is the same
    /// question answered before the window exists.
    #[must_use]
    pub fn title(&self) -> String {
        match self.ws.active() {
            ViewKind::Charts => self.charts.doc.title().to_string(),
            ViewKind::Protocol => format!("Protocol · {}", self.protocol.doc.model.protocol),
        }
    }

    /// The content box the chart pane was handed by the last frame this window
    /// drew, or `None` if it has not drawn one.
    #[must_use]
    pub fn chart_viewport(&self) -> Option<egui::Rect> {
        self.charts.doc.viewport
    }

    /// The rect the controls rail's overlay checkbox occupied in the last frame
    /// this window drew.
    #[must_use]
    pub fn overlay_checkbox(&self) -> Option<egui::Rect> {
        self.charts.doc.overlay_checkbox
    }

    /// The content box the DAG canvas pane was handed by the last frame this
    /// window drew, or `None` if it has not drawn one.
    #[must_use]
    pub fn canvas_viewport(&self) -> Option<egui::Rect> {
        self.protocol.doc.viewport
    }

    /// The rect the top bar's switcher control for `view` occupied in the last
    /// frame this window drew.
    #[must_use]
    pub fn switcher_rect(&self, view: ViewKind) -> Option<egui::Rect> {
        self.switcher
            .iter()
            .find(|(v, _)| *v == view)
            .map(|(_, r)| *r)
    }

    /// The protocol view's interaction model, read-only.
    ///
    /// The window is the only thing that feeds it keys, and it feeds it keys
    /// only while the protocol view is drawn — so this is how a test asks
    /// whether a keystroke reached the DAG, which is the half of that gate that
    /// cannot be seen from outside.
    #[must_use]
    pub fn protocol_model(&self) -> &ProtocolModel {
        &self.protocol.doc.model
    }

    /// Draw one frame into the root `ui` (egui 0.35's Ui-rooted model — the same
    /// `ui` eframe hands `App::ui` and `Context::run_ui` yields). Idempotent and
    /// tier-agnostic.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.fonts_installed {
            design::apply(&ctx, self.mode);
            self.fonts_installed = true;
        }
        let view = self.ws.active();
        let mode = self.mode;

        // The protocol grammar is bare-key — `h j k l y t Enter Esc ⌫ shift-S`
        // with no modifier to disambiguate it — so it is fed only while its own
        // view is drawn. Gating on the active view rather than on the focused
        // pane's `Subject::key_context` is deliberate: the grammar drives the
        // *view's* model, not one pane's, and every pane of this view declares
        // the same context anyway. A per-pane gate would be a second answer to
        // a question the view already answers.
        if view == ViewKind::Protocol {
            let events = ctx.input(|i| i.events.clone());
            self.protocol.doc.model.feed_events(&events);
            if let Some(addr) = self.protocol.doc.model.take_yank_request() {
                ctx.copy_text(addr);
            }
            self.set_active_tab();
        }

        // Only the drawn view rasters. The other view's texture is not freed —
        // see `sweep`.
        match view {
            ViewKind::Charts => self.charts.doc.present(ctx.pixels_per_point(), mode),
            ViewKind::Protocol => self.protocol.doc.present(ctx.pixels_per_point(), mode),
        }

        let mut bar = TopBar::default();
        Panel::top("bf-top-bar")
            .resizable(false)
            .exact_size(BAR_HEIGHT)
            .show(ui, |ui| bar = self.top_bar(ui));
        self.switcher = std::mem::take(&mut bar.switcher);

        // The key-hint bar belongs to the protocol grammar, so it is drawn on
        // the view that has one. The charts view has no key grammar at all, and
        // `chart_window_size` has no term for a hint bar —
        // `the_window_it_asks_for_fits_the_raster_it_presents` is the assertion
        // that keeps it honest about that: it lays a real frame out at the size
        // that function asks for and reads the box the chart pane was handed, so
        // a hint bar appearing on this view would take BAR_HEIGHT out of that
        // box and redden.
        if view == ViewKind::Protocol {
            let model = &self.protocol.doc.model;
            Panel::bottom("bf-hint-bar")
                .resizable(false)
                .exact_size(BAR_HEIGHT)
                .show(ui, |ui| hint_ui(ui, model, mode));
        }

        // The dock fills the rest. Every pane's chrome comes from its subject,
        // through the one `egui_tiles::Behavior` in the product. The frame is
        // `Frame::central_panel`'s, restated with `DOCK_INSET` in place of
        // egui's internal `8` — same pixels, one declaration the window
        // arithmetic reads.
        let tabbed = self.ws.tabbed_tiles(view);
        let focused = self.ws.focus();
        let mut requests: Vec<Request> = Vec::new();
        let dock_frame = egui::Frame::new()
            .inner_margin(DOCK_INSET)
            .fill(ui.visuals().panel_fill);
        let (ws, charts, protocol) = (&mut self.ws, &mut self.charts, &mut self.protocol);
        CentralPanel::default().frame(dock_frame).show(ui, |ui| {
            // The two arms are the whole cost of keeping the documents apart.
            match view {
                ViewKind::Charts => {
                    let mut behavior = PaneChrome::new(
                        &mut charts.doc,
                        &mut charts.items,
                        mode,
                        focused,
                        &tabbed,
                        &mut requests,
                    );
                    ws.tree_mut(view).ui(&mut behavior, ui);
                }
                ViewKind::Protocol => {
                    let mut behavior = PaneChrome::new(
                        &mut protocol.doc,
                        &mut protocol.items,
                        mode,
                        focused,
                        &tabbed,
                        &mut requests,
                    );
                    ws.tree_mut(view).ui(&mut behavior, ui);
                }
            }
        });

        self.apply(&ctx, view, requests);
        if view == ViewKind::Protocol {
            self.read_active_tab();
            if bar.toggle_flow {
                self.protocol.doc.model.toggle_flow();
                self.protocol.doc.canvas.invalidate();
            }
        }
        if let Some(next) = bar.switch {
            self.ws.set_active(next);
            ctx.request_repaint();
        }
        self.sweep();
    }

    /// The one top bar: the view switcher, the active view's subject, and what
    /// this is being rendered by.
    ///
    /// Returns what the bar's controls were asked to do rather than doing it:
    /// this runs inside the top panel's closure, and switching views mid-frame
    /// would leave the dock below drawing a tree the bar above has already
    /// stopped describing.
    ///
    /// The switcher is a pair of plain `selectable_label`s rather than a
    /// `Subject` toolbar entry. A toolbar entry carries a [`Verb`], every verb
    /// is checked against the `brightfield-keys` registry, and there is no
    /// registered verb for switching views — inventing one is a keyboard-grammar
    /// decision, not a consequence of putting two views in one window.
    ///
    /// **The right-hand group is dropped rather than allowed to spill.** A
    /// right-to-left layout draws from the window's right edge leftwards and
    /// does not stop at the cursor the left-hand content left behind, so on a
    /// narrow window the renderer line lands *on top of* the switcher. egui
    /// gives a click to the last widget drawn over a point, so the switcher goes
    /// on drawing, goes on recording a rect, and stops switching — and with no
    /// keyboard verb for switching views, that leaves the other view with no
    /// reachable affordance at all. Measured before this gate went in, sweeping
    /// the protocol view's window width from 240 to 700 logical points: every
    /// width from 376 up switched, and all but two of the sampled widths below
    /// it did not. [`right_group_width`] asks what the group needs before any of
    /// it is drawn, and
    /// `the_top_bar_switcher_switches_the_view_the_dock_draws` clicks the
    /// switcher at a window narrow enough to have failed.
    ///
    /// [`Verb`]: brightfield_workbench::Verb
    fn top_bar(&mut self, ui: &mut egui::Ui) -> TopBar {
        let sem = semantic(self.mode.is_dark());
        let active = self.ws.active();
        // Read everything the bar says before drawing it, so the closure below
        // borrows no more of `self` than the switcher state it writes.
        let title = self.title();
        let crumbs = match active {
            ViewKind::Charts => Vec::new(),
            ViewKind::Protocol => self.protocol.doc.model.breadcrumb(),
        };
        let flow = self.protocol.doc.model.flow();
        let theme = match self.mode {
            Mode::Light => "light",
            Mode::Dark => "dark",
        };

        let mut bar = TopBar::default();
        ui.horizontal_centered(|ui| {
            for view in ViewKind::ALL {
                let control = ui.selectable_label(view == active, view.label());
                bar.switcher.push((view, control.rect));
                if control.clicked() && view != active {
                    bar.switch = Some(view);
                }
            }
            ui.label(egui::RichText::new(title).color(chrome::colour(sem.text.primary)));
            for crumb in crumbs {
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
            let renderer = format!("egui · Vello · wgpu 29  —  {theme}");
            let toggle = (active == ViewKind::Protocol).then(|| match flow {
                Flow::Vertical => ("flow: vertical ⇄".to_string(), "horizontal"),
                Flow::Horizontal => ("flow: horizontal ⇄".to_string(), "vertical"),
            });
            let wanted = right_group_width(ui, &renderer, toggle.as_ref().map(|(t, _)| t.as_str()));
            if ui.available_width() >= wanted {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(renderer)
                            .monospace()
                            .color(chrome::colour(sem.text.muted)),
                    );
                    if let Some((label, next)) = toggle {
                        bar.toggle_flow = ui
                            .button(egui::RichText::new(label).font(ui_font()))
                            .on_hover_text(format!("switch to {next} flow"))
                            .clicked();
                    }
                });
            }
        });
        bar
    }

    /// Perform the requests the frame's panes raised, now that the tile tree's
    /// borrow is over.
    ///
    /// A verb goes to the model of the view that raised it — only the active
    /// view's panes drew, so that is the active view's model. The charts view
    /// has no model to dispatch into and nothing on it declares a verb-bearing
    /// control; the arm is spelled out so that adding one is a change to *this*
    /// line rather than a control that silently does nothing.
    fn apply(&mut self, ctx: &egui::Context, view: ViewKind, requests: Vec<Request>) {
        for request in requests {
            match request {
                Request::Verb(verb) => match view {
                    ViewKind::Charts => {}
                    ViewKind::Protocol => {
                        self.protocol.doc.model.dispatch(verb.as_str());
                    }
                },
                Request::Focus(key) => {
                    self.ws.set_focus(key);
                }
                Request::Repaint => ctx.request_repaint(),
            }
        }
    }

    /// Before rendering: make the active Canvas/Steps tab authoritative from the
    /// model's sheet flag (so the `shift-S`/`Esc` keys drive it).
    fn set_active_tab(&mut self) {
        let show_sheet = self.protocol.doc.model.show_sheet();
        let tree = self.ws.tree_mut(ViewKind::Protocol);
        let Some(canvas) = tile_of(tree, PaneKey::new(ViewKind::Protocol, CANVAS)) else {
            return;
        };
        let Some(steps) = tile_of(tree, PaneKey::new(ViewKind::Protocol, STEPS)) else {
            return;
        };
        let want = if show_sheet { steps } else { canvas };
        let Some(tabs_id) = tabs_holding(tree, canvas) else {
            return;
        };
        if let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get_mut(tabs_id) {
            tabs.set_active(want);
        }
    }

    /// After rendering: read a manual tab click back into the model (so a
    /// pointer click on the Steps tab also opens the sheet, and Canvas closes it).
    fn read_active_tab(&mut self) {
        let tree = self.ws.tree(ViewKind::Protocol);
        let Some(steps) = tile_of(tree, PaneKey::new(ViewKind::Protocol, STEPS)) else {
            return;
        };
        let Some(tabs_id) = tabs_holding(tree, steps) else {
            return;
        };
        if let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get(tabs_id) {
            if let Some(active) = tabs.active {
                self.protocol.doc.model.set_show_sheet(active == steps);
            }
        }
    }

    /// Declare which panes this frame laid out, so each host can free the canvas
    /// slot of any pane that has gone.
    ///
    /// **Every pane of every view, every frame** — including the whole of the
    /// view that was not drawn, and including a canvas tabbed out of sight.
    /// `end_frame` frees any slot that neither presented this frame nor appears
    /// in the set it is handed, while `present` caches its texture id across
    /// frames and returns early on an unchanged key. Naming only the drawn
    /// view's panes would therefore free the other view's texture and leave a
    /// dangling id the instant the user switched back. The failure in the other
    /// direction is a leak, which is the safe one to be wrong in.
    fn sweep(&mut self) {
        let charts: BTreeSet<PaneKey> = self.ws.panes(ViewKind::Charts).into_iter().collect();
        self.charts.doc.sweep(&charts);
        let protocol: BTreeSet<PaneKey> = self.ws.panes(ViewKind::Protocol).into_iter().collect();
        self.protocol.doc.sweep(&protocol);
    }
}

/// How wide the top bar's right-hand group would be if it drew: the renderer
/// line, and the flow toggle when the protocol view supplies one.
///
/// Asked *before* the group is drawn, which is the whole point — a
/// right-to-left layout that does not fit overlaps what is already on the bar
/// instead of shrinking, and the thing already on the bar is the only way to
/// reach the other view. See [`MeridianApp::top_bar`].
///
/// The one leading item spacing is included because egui inserts it when it
/// places the group, and `available_width` is measured before it exists.
fn right_group_width(ui: &egui::Ui, renderer: &str, toggle: Option<&str>) -> f32 {
    let spacing = ui.spacing().item_spacing.x;
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let mut wanted = spacing + text_width(ui, renderer, mono);
    if let Some(label) = toggle {
        wanted += spacing + text_width(ui, label, ui_font()) + 2.0 * ui.spacing().button_padding.x;
    }
    wanted
}

/// The width `text` lays out to in `font`, unwrapped.
fn text_width(ui: &egui::Ui, text: &str, font: egui::FontId) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x
}
