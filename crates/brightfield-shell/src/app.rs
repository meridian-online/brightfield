//! The chart view — the composited Vello dashboard, expressed as two
//! [`Item`]s on the workbench shell contract.
//!
//! This file declares the view and nothing around it. The window, the top bar
//! and the frame loop belong to [`crate::window::MeridianApp`], which draws
//! this view and the protocol view from one `eframe::App`; what is here is the
//! document the two panes share, the registry that is the single declaration of
//! the view's shape, and the panes themselves.
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
//! - **It no longer declares its window's geometry at all.** `window_size` said
//!   the rail cost 214 logical points while `main.rs` said 200, and both were
//!   pixel constants beside a panel declared at 180. All three went when
//!   [`chart_window_size`](crate::window::chart_window_size) derived the window
//!   from the controls rail's declared share; the derivation itself now lives
//!   with the window it
//!   sizes, next to the protocol view's, because one window has one answer.
//! - **It no longer owns a shell.** `ShellState` and `draw_shell` were this
//!   view's half of a two-`eframe::App` fork that made it structurally
//!   impossible for the chart and the DAG to share a window. Both are gone.
//! - **It no longer spells its own spacing.** Two bare `add_space(6.0)` calls in
//!   the rail are gone — the pane frame's padding comes from the spacing ladder.
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

use std::collections::BTreeSet;

use brightfield_keys::BindingContext;
use brightfield_render::canvas_host::{ChartSurface, Color, PixelSize, SurfaceCursor};
use brightfield_workbench::registry::{DockSide, Slot};
use brightfield_workbench::{
    chrome, EmptyState, Icon, Item, ItemCtx, ItemId, ItemRegistry, ItemSpec, PaneKey, Subject,
    Verb, ViewKind,
};

use meridian_design::chrome::{INK_DARK, INK_LIGHT};
use meridian_design::{semantic, spacing};

use crate::canvas::{surface_input, CanvasSlot, EguiCanvasHost, EguiChartFrame};
use crate::design::Mode;
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
    /// hold [`chart_window_size`](crate::window::chart_window_size) to the box
    /// the dock actually produces, rather than to a second copy of the same
    /// arithmetic — which is the only kind of assertion that could have caught
    /// this window clipping its own raster.
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
/// Nothing switches mode mid-process today — the window takes a [`Mode`] at boot
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

    /// The dashboard's title, or the product's if the spec named none.
    #[must_use]
    pub fn title(&self) -> &str {
        self.composed.title.as_deref().unwrap_or("Brightfield")
    }

    /// Declare which panes the frame laid out, so the host can free the canvas
    /// slot of any pane that has gone. See [`crate::window::MeridianApp`]'s
    /// sweep, which is the only caller and which explains what it hands in.
    pub(crate) fn sweep(&mut self, visible: &BTreeSet<PaneKey>) {
        if let Some(host) = self.canvas.host_mut() {
            host.end_frame(visible);
        }
    }

    /// Rasterise the Vello dashboard onto a shared-device texture at the current
    /// HiDPI scale and register it for zero-copy egui sampling — only when
    /// [`CanvasKey`] actually changed.
    ///
    /// The composited scene is in logical coordinates, so it is scaled by `ppp`
    /// onto the device-resolution texture (the same scale-the-scene step the
    /// app's dump path uses) — otherwise the logical-sized scene would fill only
    /// the top-left corner of the larger texture.
    pub(crate) fn present(&mut self, ppp: f32, mode: Mode) {
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
/// Called at boot from [`crate::window::MeridianApp`], before any layout file
/// could be read. Idempotent, so a test binary that builds two windows neither
/// falls over nor grows the vocabulary.
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
/// registry lays the dock out with it, and
/// [`chart_window_size`](crate::window::chart_window_size) sizes the window from
/// it. It replaces three numbers that disagreed — a panel pinned at 180 logical
/// points, a `window_size` that budgeted 214 for it, and a `main.rs` that
/// budgeted 200.
pub(crate) const CONTROLS_SHARE: f32 = 0.2;

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
