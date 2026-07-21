//! The chart view — the composited Vello dashboard, expressed as two
//! [`Item`]s on the workbench shell contract.
//!
//! [`draw_shell`] is still the single source of this surface's UI — the live
//! eframe window (`main.rs`), the headless `brightfield-shot` binary, and the
//! `egui_kittest` snapshot tests all call it, so what an agent sees in a PNG is
//! exactly what ships. What changed is what it draws: the dashboard and the
//! controls are two panes of an `egui_tiles` dock, and every pixel of chrome
//! around them comes from [`PaneChrome`] reading each pane's [`Subject`].
//!
//! # What this file no longer does
//!
//! - **It no longer draws a pane header.** The right-hand panel used to open
//!   with a hand-written `RichText::new("Controls").strong()` — a pane naming
//!   itself, in a treatment nothing else in the product used. The pane declares
//!   the name now and the shell draws the band.
//! - **It no longer hand-places a side panel.** `Panel::right(…).exact_size(180.0)`
//!   is a [`Slot::Rail`] with a share, so the dock lays it out, the user can
//!   resize it, and — once the workspace shell can perform the verb — close and
//!   reopen it.
//! - **It no longer declares its window's geometry twice.** `window_size` said
//!   the rail cost 214 logical points while `main.rs`'s `run_mosaic_window` said
//!   200, and both were pixel constants beside a panel declared at 180. All
//!   three are gone: [`window_size_for`] derives the window from
//!   the controls rail's declared share, and both callers read it.
//! - **It no longer guesses what its own chrome costs.** The first cut of
//!   [`window_size_for`] replaced those three constants with two more —
//!   `SPACE_8` across and `SPACE_9 + SPACE_8` down — hand-tuned to look about
//!   right and 5.6pt and 17pt short respectively, so the window clipped the
//!   bottom seventeen rows of its own chart raster and the re-photographed
//!   baseline preserved the clipping. Every term of the budget is read from the
//!   component that consumes it now: [`chrome::header_band_height`],
//!   [`chrome::pane_content_inset`], [`TILE_GAP`], and this file's own
//!   [`TOP_BAR`] and [`DOCK_INSET`] — the two egui would otherwise decide for
//!   us, pinned so that they can be subtracted.
//! - **It no longer spells its own spacing.** Two bare `add_space(6.0)` calls in
//!   the rail are gone (the pane frame's padding comes from the spacing ladder),
//!   and so are the two that padded the top bar — the band has a declared height
//!   and centres its row in it.
//! - **The top bar is no longer a heading.** `ui.heading` was a second type size
//!   on a surface whose other text is the 12px UI size — the four-pixel drift
//!   the workbench exists to end. It is the UI size in chrome ink now, which is
//!   what the protocol panel's breadcrumb bar already was.
//!
//! Two categories the protocol increment deleted **do not exist here**, and
//! saying so is more useful than inventing a deletion to match its shape:
//!
//! - **No hardcoded light-mode ink.** Both colours this file resolves — the
//!   raster's base tone and the crosshair ink — were already `match mode` arms
//!   over `INK_LIGHT`/`INK_DARK`. (The chart *raster* is a different matter:
//!   `brightfield-render`'s `axis`, `grid`, `legend`, `scene`, `text` and `mark`
//!   name `INK_LIGHT` unconditionally, so the plotted chart is light ink in both
//!   modes. That is the chart-side twin of the DAG raster fix, it is fourteen
//!   call sites in another crate, and it is not this file.)
//! - **No bespoke selection or focus treatment**, because this surface has
//!   neither: nothing on it is selectable, and nothing tracked focus.
//!
//! What this surface *did* lack entirely is an empty state — a spec that
//! composed nothing would have rendered chrome and a blank rectangle. Both panes
//! declare one now, and [`brightfield_workbench::audit`] is what makes that true
//! rather than remembered.
//!
//! `ShellApp` still owns its own window and its own top bar, exactly as
//! [`crate::protocol::ProtocolShell`] still owns its breadcrumb: those become
//! the workspace's toolbar row when the one-app migration lands.

use std::collections::BTreeSet;

use egui::containers::{CentralPanel, Panel};
use egui_tiles::{Tile, Tree};

use brightfield_keys::BindingContext;
use brightfield_render::canvas_host::{ChartSurface, Color, PixelSize, SurfaceCursor};
use brightfield_workbench::behavior::TILE_GAP;
use brightfield_workbench::registry::{DockSide, Slot};
use brightfield_workbench::workspace::tabbed_tiles_of;
use brightfield_workbench::{
    chrome, EmptyState, Icon, Item, ItemCtx, ItemId, ItemMap, ItemRegistry, ItemSpec, PaneChrome,
    PaneKey, Request, Subject, Verb, ViewKind,
};

use meridian_design::chrome::{INK_DARK, INK_LIGHT};
use meridian_design::{semantic, spacing};

use crate::canvas::{surface_input, CanvasSlot, EguiCanvasHost, EguiChartFrame};
use crate::design::{self, Mode};
use crate::pipeline::Composed;

// ---------------------------------------------------------------------------
// ChartDoc — the state every pane in this view shares.
// ---------------------------------------------------------------------------

/// The chart view's **document**: the composited dashboard, the canvas it
/// rasters into, and the chart state the panes read.
///
/// No [`Item`] holds a handle to it — the shell hands out exactly one
/// `&mut ChartDoc`, for the duration of one pane's draw. That is why the canvas
/// host lives here rather than inside the canvas pane, and why the parameter and
/// the overlay flag live here rather than inside the controls pane: the controls
/// rail writes both and the chart pane reads one of them, so they belong to the
/// view, not to either pane.
pub struct ChartDoc {
    /// The composited Vello dashboard and its logical size.
    pub composed: Composed,
    /// The parameter the controls rail's slider drives.
    ///
    /// Nothing downstream re-executes on it today: the compose pipeline runs
    /// once, before the window opens, and this value reaches no query. It is the
    /// shell's worked example of a native egui control over shared view state,
    /// and it is named for what it is rather than for what it will be.
    pub param: f32,
    /// Whether the hover crosshair overlay is armed — the worked example that
    /// keeps the overlay seam exercised end to end.
    pub overlay: bool,
    /// The content box the chart pane was last handed, in window-space logical
    /// points — `None` until a frame has been laid out.
    ///
    /// Written by the chart pane before it looks for a texture, so it is
    /// observable on a *headless* document. That is what lets a GPU-free test
    /// hold [`window_size_for`] to the box the dock actually produces, rather
    /// than to a second copy of the same arithmetic — which is the only kind of
    /// assertion that could have caught this window clipping its own raster.
    pub viewport: Option<egui::Rect>,
    /// The rect the controls rail's overlay checkbox last occupied, in
    /// window-space logical points — `None` until a frame has been laid out.
    ///
    /// Recorded for the same reason as [`Self::viewport`], and it buys the same
    /// thing one level in. The pixel test that proves the overlay seam still
    /// crosses the dock has to *click* this checkbox, and it used to aim at a
    /// coordinate typed against a layout nothing derived it from: it landed
    /// today, and would have silently stopped landing the first time the rail's
    /// share or a row height moved. It aims from a headless layout pass now.
    pub overlay_checkbox: Option<egui::Rect>,
    canvas: CanvasSlot<CanvasKey>,
}

/// Everything the dashboard raster's pixels depend on.
///
/// The device size catches a resize or a HiDPI-scale change. `dark` catches a
/// theme switch: the base tone the scene is composited over is resolved for the
/// mode, so a raster held across a switch would keep the tone it was baked at.
/// Nothing switches mode mid-process today — [`ShellState::new`] takes a [`Mode`]
/// and no code path changes it — so that field is correctness kept ahead of the
/// control that will exercise it, not a bug being fixed.
///
/// The scene itself is not in the key because it is composed once, before the
/// window opens, and never rebuilt.
///
/// **Deliberately not shared with the protocol view's key of the same name**,
/// though [`CanvasSlot`] itself now is. That one carries `expanded`, `flow` and
/// a layout `generation` as well, because a DAG re-lays-out under the user's
/// hands and a composed dashboard does not. Merging them would hand this view
/// three fields it has to remember to hold constant forever, and a cache-key
/// field nobody sets is a cache that silently never invalidates. The same note
/// is on the protocol side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CanvasKey {
    dev_width: u32,
    dev_height: u32,
    dark: bool,
}

impl ChartDoc {
    /// A document over `composed`, rastering through `host`.
    #[must_use]
    pub fn new(composed: Composed, host: EguiCanvasHost) -> Self {
        Self {
            composed,
            param: 0.5,
            overlay: true,
            viewport: None,
            overlay_checkbox: None,
            canvas: CanvasSlot::new(host),
        }
    }

    /// A document with no device behind it.
    #[must_use]
    pub fn headless(composed: Composed) -> Self {
        Self {
            composed,
            param: 0.5,
            overlay: true,
            viewport: None,
            overlay_checkbox: None,
            canvas: CanvasSlot::headless(),
        }
    }

    /// An empty document: a dashboard with no plots on it, and no device.
    ///
    /// The value [`chart_registry`]'s audit runs against.
    #[must_use]
    pub fn empty() -> Self {
        Self::headless(Composed::empty())
    }

    /// Whether there is a dashboard to draw.
    ///
    /// A composed dashboard's size is the union of its placed plots' rects, and
    /// [`crate::pipeline::compose_spec`] fails outright rather than returning a
    /// dashboard with none, so zero area means zero plots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.composed.width == 0 || self.composed.height == 0
    }

    /// Rasterise the Vello dashboard onto a shared-device texture at the current
    /// HiDPI scale and register it for zero-copy egui sampling — only when
    /// [`CanvasKey`] actually changed.
    ///
    /// The composited scene is in logical coordinates, so it is scaled by `ppp`
    /// onto the device-resolution texture (the same scale-the-scene step the
    /// app's dump path uses) — otherwise the logical-sized scene would fill only
    /// the top-left corner of the larger texture.
    fn ensure_presented(&mut self, ppp: f32, mode: Mode) {
        let dev = PixelSize {
            width: ((self.composed.width as f32) * ppp).round().max(1.0) as u32,
            height: ((self.composed.height as f32) * ppp).round().max(1.0) as u32,
        };
        let key = CanvasKey {
            dev_width: dev.width,
            dev_height: dev.height,
            dark: mode.is_dark(),
        };
        if self.canvas.presented(&key) {
            return;
        }
        let Some(host) = self.canvas.host_mut() else {
            return;
        };
        // The raster's base tone, under the composited scene. It used to be
        // `INK_LIGHT.page`/`INK_DARK.page` — the page behind a window. The
        // raster sits inside a pane now, so it takes the pane's surface: the
        // same token `chrome::pane_frame` fills the pane with, and the same one
        // the DAG raster resolves. `compose_dashboard` paints its own background
        // across the whole extent, so this shows through the scene's antialiased
        // edges rather than as an area of its own.
        let base = Color::from_token(semantic(mode.is_dark()).surfaces.raised);
        let mut scaled = vello::Scene::new();
        scaled.append(
            &self.composed.scene,
            Some(kurbo::Affine::scale(f64::from(ppp))),
        );
        let id = host.present_keyed(CHART_PANE, &scaled, dev, base);
        self.canvas.record(key, id);
    }
}

// ---------------------------------------------------------------------------
// The registry: the one declaration of this view's shape.
// ---------------------------------------------------------------------------

/// The composited dashboard — the view's centre pane.
pub const CHART: ItemId = ItemId::new("chart-canvas");
/// The controls rail.
pub const CONTROLS: ItemId = ItemId::new("chart-controls");

/// Add this view's item ids to the process's layout vocabulary.
///
/// Called at boot from [`ShellState::new`], before any layout file could be
/// read. Idempotent, so a test binary that builds two shells neither falls over
/// nor grows the vocabulary.
///
/// The ids come from [`chart_registry`] and nowhere else. The protocol view
/// learned that the hard way: a hand-written `static [ItemId; 4]` beside its
/// registry was a second declaration of the view's shape, one a fifth pane could
/// be added to the registry without.
pub fn publish_item_ids() {
    chart_registry().publish_ids();
}

/// The chart pane's address — the key its Vello texture slot is filed under.
const CHART_PANE: PaneKey = PaneKey::new(ViewKind::Charts, CHART);

/// The controls rail's share of the window. Declared once and read twice: the
/// registry lays the dock out with it, and [`window_size_for`] sizes the window
/// from it. It replaces three numbers that disagreed — a panel pinned at 180
/// logical points, a `window_size` that budgeted 214 for it, and a `main.rs`
/// that budgeted 200.
const CONTROLS_SHARE: f32 = 0.2;

/// Every icon here is a *name*, resolved to paint at draw time. The Meridian
/// icon set has not landed in this workspace, so the chrome reserves each
/// glyph's box without painting into it.
const ICON_CHART: Icon = Icon("chart");
const ICON_CONTROLS: Icon = Icon("sliders");

/// The chart view's registry: two panes, where each sits, and the verb that
/// shows and hides the rail.
///
/// This is the **only** declaration of the view's shape. The dock's default
/// arrangement ([`ItemRegistry::default_tree`]), the live item map
/// ([`ItemRegistry::instantiate`]) and the published id vocabulary
/// ([`ItemRegistry::publish_ids`], via [`publish_item_ids`]) are all derived
/// from this list, so a pane cannot be added to one and forgotten in another.
#[must_use]
pub fn chart_registry() -> ItemRegistry<ChartDoc> {
    ItemRegistry::new(
        ViewKind::Charts,
        vec![
            ItemSpec {
                id: CHART,
                slot: Slot::Centre,
                toggle: None,
                make: || Box::new(ChartPane),
            },
            ItemSpec {
                id: CONTROLS,
                slot: Slot::Rail {
                    side: DockSide::Right,
                    share: CONTROLS_SHARE,
                },
                toggle: Some(Verb::new("toggle-controls-rail")),
                make: || Box::new(ControlsPane),
            },
        ],
    )
}

// ---------------------------------------------------------------------------
// The two panes.
// ---------------------------------------------------------------------------

/// The composited dashboard, presented as a zero-copy egui texture, with the
/// hover crosshair overlay drawn on top of it.
///
/// A unit struct because it has no view-local state at all — everything it draws
/// is a function of the document.
struct ChartPane;

impl Item<ChartDoc> for ChartPane {
    fn item_id(&self) -> ItemId {
        CHART
    }

    fn subject(&self, doc: &ChartDoc) -> Subject {
        let subject = Subject::new("Chart", ICON_CHART, BindingContext::Workspace);
        if doc.is_empty() {
            subject.empty(EmptyState::new(
                ICON_CHART,
                "Nothing to draw",
                "This spec composed no plots. Open a spec whose marks resolve \
                 against their data, so that at least one plot is placed.",
            ))
        } else {
            subject
        }
    }

    fn ui(&mut self, doc: &mut ChartDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        // Recorded *before* the texture check, so a headless document still
        // reports the box the dock gave this pane. See `ChartDoc::viewport`.
        doc.viewport = Some(ui.max_rect());
        let Some(texture) = doc.canvas.texture() else {
            // No device behind this document. The pane is blank rather than
            // apologetic: a headless document is a test fixture, never a state a
            // user reaches, so a message here would be chrome nobody sees.
            return;
        };
        let (w, h) = (doc.composed.width, doc.composed.height);
        let overlay_on = doc.overlay;
        let mode = cx.mode;
        let ctx = ui.ctx().clone();

        let mut frame = EguiChartFrame::new(ui, texture);
        frame.present(PixelSize {
            width: w,
            height: h,
        });

        // Drive the overlay/hit-test seam from egui pointer input: a crosshair
        // marker at the pointer while it's over the chart, with a grab cursor.
        let Some(rect) = frame.reserved() else {
            return;
        };
        let input = surface_input(&ctx, rect);
        if !overlay_on || !input.hovered {
            return;
        }
        if let Some(p) = input.pointer_pos {
            // The chart's own ink layer, not the chrome's: this line is painted
            // through the render seam over a Vello raster whose colours all come
            // from `chrome::INK_*`, so it stays there rather than moving to
            // `semantic()` and quietly becoming a different colour.
            let focus = match mode {
                Mode::Light => INK_LIGHT.focus,
                Mode::Dark => INK_DARK.focus,
            };
            let ink = Color::from_token_alpha(focus, 0.9);
            let painter = frame.overlay();
            painter.line(
                kurbo::Point::new(p.x, 0.0),
                kurbo::Point::new(p.x, f64::from(h)),
                ink,
                1.0,
            );
            painter.line(
                kurbo::Point::new(0.0, p.y),
                kurbo::Point::new(f64::from(w), p.y),
                ink,
                1.0,
            );
            painter.fill_circle(p, 3.0, ink);
        }
        frame.set_cursor(Some(SurfaceCursor::Grab));
    }
}

/// The controls rail: the native egui widgets the render trait does not cover.
///
/// A unit struct for the same reason [`ChartPane`] is one — the values these
/// widgets drive belong to the document, because the chart pane reads one of
/// them.
struct ControlsPane;

impl Item<ChartDoc> for ControlsPane {
    fn item_id(&self) -> ItemId {
        CONTROLS
    }

    fn subject(&self, doc: &ChartDoc) -> Subject {
        let subject = Subject::new("Controls", ICON_CONTROLS, BindingContext::Workspace);
        if doc.is_empty() {
            subject.empty(EmptyState::new(
                ICON_CONTROLS,
                "No dashboard to control",
                "These controls act on a composed dashboard. Open a spec that \
                 places at least one plot.",
            ))
        } else {
            subject
        }
    }

    fn ui(&mut self, doc: &mut ChartDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        // The legend that used to sit above these controls is gone and is not
        // coming back here: a hardcoded "Series A/B/C" swatch block duplicated
        // the chart's own in-scene legend and, being fixed at three series,
        // mislabelled a single-series bar chart. Accurate, one-per-chart legends
        // belong in the plot margin, derived from each chart's real series — a
        // follow-up that needs the compose pipeline to surface series metadata
        // (it is currently baked into the Vello scene by `build_multi_mark_scene`).
        ui.label("param");
        ui.add(egui::Slider::new(&mut doc.param, 0.0..=1.0));
        doc.overlay_checkbox = Some(ui.checkbox(&mut doc.overlay, "hover overlay").rect);
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

// ---------------------------------------------------------------------------
// ShellState — the document, the dock, and the top bar it still owns.
// ---------------------------------------------------------------------------

/// The chart view's shell: the [`ChartDoc`], the two live items, and the dock
/// tree the registry laid out. [`draw_shell`] is the single frame source (live
/// window, headless shot, snapshot) — the loop's guarantee.
///
/// It still owns a window and a top bar, both of which the one-app shell takes
/// over later. What it no longer owns is any pane's chrome.
pub struct ShellState {
    doc: ChartDoc,
    items: ItemMap<ChartDoc>,
    dock: Tree<PaneKey>,
    /// The focused pane. Tracked because [`PaneChrome`] reports focus moves and
    /// hands each pane its own focus state; nothing paints it yet — the pane
    /// focus ring lands with the one-app shell.
    focus: Option<PaneKey>,
    mode: Mode,
    fonts_installed: bool,
}

impl ShellState {
    /// Build the shell around a composited dashboard and its egui host.
    #[must_use]
    pub fn new(composed: Composed, host: EguiCanvasHost, mode: Mode) -> Self {
        publish_item_ids();
        let registry = chart_registry();
        Self {
            doc: ChartDoc::new(composed, host),
            items: registry.instantiate(),
            dock: registry.default_tree(),
            focus: None,
            mode,
            fonts_installed: false,
        }
    }

    /// The same shell over a document with no device behind it.
    ///
    /// Everything except the raster is a pure function of the composed
    /// dashboard, so this lays out identically to [`Self::new`] — the chart pane
    /// reserves and paints nothing, and every rect around it is the same. That
    /// is what makes the window arithmetic assertable without a GPU: a test can
    /// run a real frame through `egui::Context::run_ui` and read the box the
    /// dock gave the chart pane back out of [`ChartDoc::viewport`].
    #[must_use]
    pub fn headless(composed: Composed, mode: Mode) -> Self {
        publish_item_ids();
        let registry = chart_registry();
        Self {
            doc: ChartDoc::headless(composed),
            items: registry.instantiate(),
            dock: registry.default_tree(),
            focus: None,
            mode,
            fonts_installed: false,
        }
    }

    /// The shell's natural window size in logical points.
    #[must_use]
    pub fn window_size(&self) -> (f32, f32) {
        window_size_for(&self.doc.composed)
    }

    /// The content box the chart pane was handed by the last frame this shell
    /// drew, or `None` if it has not drawn one. See [`ChartDoc::viewport`].
    #[must_use]
    pub fn chart_viewport(&self) -> Option<egui::Rect> {
        self.doc.viewport
    }

    /// The rect the controls rail's overlay checkbox occupied in the last frame
    /// this shell drew. See [`ChartDoc::overlay_checkbox`].
    #[must_use]
    pub fn overlay_checkbox(&self) -> Option<egui::Rect> {
        self.doc.overlay_checkbox
    }

    /// The dashboard this shell is showing, in logical points.
    #[must_use]
    pub fn dashboard_size(&self) -> (u32, u32) {
        (self.doc.composed.width, self.doc.composed.height)
    }

    /// The window/spec title.
    #[must_use]
    pub fn title(&self) -> &str {
        self.doc.composed.title.as_deref().unwrap_or("Brightfield")
    }

    /// The top bar: the dashboard's title, and what it is being rendered by.
    ///
    /// At the UI size in chrome ink, not `ui.heading`. A heading here was a
    /// second type size on a surface whose every other string is 12px, which is
    /// the drift `brightfield_workbench::chrome` names in its own docs.
    fn top_bar(&self, ui: &mut egui::Ui) {
        let sem = semantic(self.mode.is_dark());
        // Centred in the band rather than pushed down it with `add_space`: the
        // band's height is `TOP_BAR` now, so the padding above and below is
        // whatever that leaves, and two hand-placed gaps cannot disagree with it.
        ui.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new(self.title().to_string())
                    .color(chrome::colour(sem.text.primary)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mode = match self.mode {
                    Mode::Light => "light",
                    Mode::Dark => "dark",
                };
                ui.label(
                    egui::RichText::new(format!("egui · Vello · wgpu 29  —  {mode}"))
                        .monospace()
                        .color(chrome::colour(sem.text.muted)),
                );
            });
        });
    }

    /// Perform the requests the frame's panes raised, now that the tile tree's
    /// borrow is over.
    fn apply(&mut self, ctx: &egui::Context, requests: Vec<Request>) {
        for request in requests {
            match request {
                // Nothing on this surface declares a verb-bearing control: no
                // pane has a toolbar entry, and neither empty state names a
                // resolving action, so nothing raises one. The arm is here so
                // that adding one is a change to *this* line rather than a
                // control that silently does nothing.
                Request::Verb(_) => {}
                Request::Focus(key) => self.focus = Some(key),
                Request::Repaint => ctx.request_repaint(),
            }
        }
    }

    /// Declare which panes this frame laid out, so the host can free the canvas
    /// slot of any pane that has gone.
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
}

/// The top bar's height in logical points: a grid row for its labels, with the
/// panel's own vertical padding above and below.
///
/// **Declared, not measured.** A content-sized `Panel::top` is a component that
/// does not expose its height until a frame has run, and [`window_size_for`] is
/// called before any frame exists — `main.rs` sizes the window from it. The
/// previous answer to that was to guess (a spacing constant that happened to be
/// close), and the guess was 17 logical points short, so the window clipped the
/// bottom of its own chart. Pinning the band with
/// [`Panel::exact_size`](egui::containers::Panel::exact_size) turns the guess
/// into a fact: the bar is this tall, and the arithmetic below can subtract it.
pub const TOP_BAR: f32 = spacing::ROW_GRID + 2.0 * spacing::SPACE_2;

/// The inset between the window edge and the dock.
///
/// The value `egui::Frame::central_panel` would have used anyway, said on the
/// spacing ladder and passed in explicitly, for the same reason as [`TOP_BAR`]:
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
/// - the dock is the window inset by [`DOCK_INSET`], below the [`TOP_BAR`].
///
/// Every one of those is read from the component that draws it. The pair of
/// hand-tuned spacing constants this replaces (`SPACE_8` across, `SPACE_9 +
/// SPACE_8` down) were 5.6pt short across and 17pt short down, which is why the
/// presented raster overflowed the pane's clip rect and the last seventeen rows
/// of the chart never reached the window.
///
/// Rounded **up** to whole logical points: the share is an `f32` division, and a
/// window a quarter of a point short would clip a row of the raster just as
/// surely as one seventeen points short. `the_window_it_asks_for_fits_the_raster_it_presents`
/// lays a real frame out and holds this to the box the dock actually produces.
///
/// A free function because `main.rs` sizes the window before it can build a
/// [`ShellState`] — and because it having been open-coded there, with different
/// numbers, is the drift this replaces.
#[must_use]
pub fn window_size_for(composed: &Composed) -> (f32, f32) {
    let centre = 1.0 - CONTROLS_SHARE;
    let inset = chrome::pane_content_inset();

    let tile_w = composed.width as f32 + 2.0 * inset;
    let w = (tile_w / centre + TILE_GAP + 2.0 * DOCK_INSET).ceil();

    let tile_h = composed.height as f32 + 2.0 * inset + chrome::header_band_height();
    let h = (tile_h + 2.0 * DOCK_INSET + TOP_BAR).ceil();

    (w, h)
}

/// Draw one shell frame into the root `ui` (egui 0.35's Ui-rooted model — the
/// same `ui` eframe hands `App::ui` and `Context::run_ui` yields). Idempotent
/// and tier-agnostic.
pub fn draw_shell(ui: &mut egui::Ui, state: &mut ShellState) {
    let ctx = ui.ctx().clone();
    if !state.fonts_installed {
        design::apply(&ctx, state.mode);
        state.fonts_installed = true;
    }

    state
        .doc
        .ensure_presented(ctx.pixels_per_point(), state.mode);

    // Orientation chrome: the window's own top bar. Still the shell's own, still
    // not a `Subject` — see the module docs. Pinned to `TOP_BAR` rather than
    // sized by its text, so that `window_size_for` can subtract it.
    Panel::top("bf-shell-header")
        .resizable(false)
        .exact_size(TOP_BAR)
        .show(ui, |ui| state.top_bar(ui));

    // The dock fills the rest. Every pane's chrome comes from its subject,
    // through the one `egui_tiles::Behavior` in the product. The frame is
    // `Frame::central_panel`'s, restated with `DOCK_INSET` in place of egui's
    // internal `8` — same pixels, one declaration the window arithmetic reads.
    let tabbed = tabbed_tiles_of(&state.dock);
    let mut requests: Vec<Request> = Vec::new();
    let dock_frame = egui::Frame::new()
        .inner_margin(DOCK_INSET)
        .fill(ui.visuals().panel_fill);
    CentralPanel::default().frame(dock_frame).show(ui, |ui| {
        let mut behavior = PaneChrome::new(
            &mut state.doc,
            &mut state.items,
            state.mode,
            state.focus,
            &tabbed,
            &mut requests,
        );
        state.dock.ui(&mut behavior, ui);
    });
    state.apply(&ctx, requests);
    state.sweep_canvas();
}
