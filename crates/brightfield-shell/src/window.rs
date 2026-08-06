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

use brightfield_keys::{Altitude, RecencyCounter};
use brightfield_protocol::layout::{Flow, Layout};
use brightfield_sql::ir::SampleRate;
use brightfield_workbench::behavior::{TAB_BAR_HEIGHT, TILE_GAP};
use brightfield_workbench::workspace::{tabs_holding, tile_of};
use brightfield_workbench::{
    chrome, Activity, ActivityIndicator, DirtyTracker, ItemMap, PaneChrome, PaneKey, Request,
    SavedLayout, StatusEntry, Subject, Verb, ViewKind, WindowGeometry, Workspace,
};
use meridian_egui::{
    ModalChrome, ModalLayer, Notification, NotificationId, NotificationLayer, Picker, PickerEvent,
    Severity, Toast, ToastLayer,
};

use meridian_design::{radius, semantic, spacing};

use crate::app::{chart_registry, ChartDoc, ChartFault, CHART, CONTROLS_SHARE};
use crate::canvas::EguiCanvasHost;
use crate::design::{self, Mode};
use crate::overlays::{CommandPalette, HelpSheet, JumpTarget, JumpToNode};
use crate::pipeline::Composed;
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

// ---------------------------------------------------------------------------
// The front door's own measures.
// ---------------------------------------------------------------------------

/// The line under the Welcome heading.
///
/// The product's own voice — chosen copy, no longer the neutral placeholder
/// this slot shipped with. Changing these words is a copy decision, not a
/// refactor.
pub const TAGLINE: &str = "Watch insight assemble.";

/// The front door's content column, in logical points: four gallery cards and
/// the three gaps between them, so the Explore row is what sets the measure
/// and everything above it aligns to the gallery.
const DOOR_COLUMN_WIDTH: f32 = 4.0 * CARD_WIDTH + 3.0 * spacing::SECTION_GAP;

/// One gallery card's outer width.
const CARD_WIDTH: f32 = 216.0;

/// The thumbnail's drawn height: the card's width less its two insets, at the
/// shipped thumbnails' own 16:10 — `(216 − 8) / 1.6`. Stated as the number the
/// arithmetic produces so a drift in either term is a visible edit here.
const CARD_IMAGE_HEIGHT: f32 = 130.0;

/// One gallery card's outer height: the image and its insets, plus room for a
/// two-line label and a two-line summary. A label that wraps past that is
/// clipped by the card's own rect rather than pushing the gallery apart.
const CARD_HEIGHT: f32 = 220.0;

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
///
/// # Who still reads this, now that a saved layout outranks it
///
/// Not a decoy, and worth naming the readers rather than leaving that to be
/// rediscovered. The live window consults it only on a boot with **no**
/// restored layout and a document to derive a size from; every other caller
/// has no saved layout and cannot get one — `capture::capture_png` and the
/// `brightfield-shot` binary behind it, which need a deterministic
/// content-derived size for the PNG tier, and the tests that hold this
/// arithmetic to a real laid-out frame rather than to a second copy of itself.
/// Deleting it deletes those gates, and they exist because this window was
/// once caught clipping the bottom seventeen rows of its own chart.
#[must_use]
pub fn chart_window_size(composed: &Composed) -> (f32, f32) {
    let centre = 1.0 - CONTROLS_SHARE;
    let inset = chrome::pane_content_inset();

    // The legend band is a term, not a bite: a dashboard whose scales call
    // for a margin legend gets the band's width beside the raster, and one
    // that calls for none contributes zero — read from the component that
    // draws it, like every other term here.
    let tile_w = composed.width as f32 + crate::legend::band_width(composed) + 2.0 * inset;
    let w = (tile_w / centre + TILE_GAP + 2.0 * DOCK_INSET).ceil();

    let tile_h = composed.height as f32
        + chart_toolbar_band(composed)
        + 2.0 * inset
        + chrome::header_band_height();
    let h = (tile_h + 2.0 * DOCK_INSET + BAR_HEIGHT).ceil();

    (w, h)
}

/// The height the chart pane's toolbar row consumes above the raster, in
/// logical points — `0.0` for a dashboard with no gesture-bindable plot,
/// where the collapsing `Toolbar` draws no row at all.
///
/// Like the legend band, a term read from the components that produce it: the
/// row is the toolbar button's real height — the binding's control height,
/// floored by the meridian style's `interact_size.y` (`control::HEIGHT_MD`),
/// which egui applies to every button — and the style's vertical item spacing
/// separates it from the raster. Keyed on the composition (not on liveness)
/// for the reason the pane's own declaration is — the window is sized before
/// a session could exist.
#[must_use]
pub fn chart_toolbar_band(composed: &Composed) -> f32 {
    use meridian_design::control;
    if composed.plots.iter().any(|p| p.gesture.is_some()) {
        control::binding(spacing::ROW_GRID)
            .control
            .max(control::HEIGHT_MD)
            + spacing::SPACE_2
    } else {
        0.0
    }
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
/// about fitting is this crate's defect.
///
/// This still answers only what the *content* wants. A display too small to
/// show it is a separate question with a separate answer —
/// [`window_size_on_display`] — because leaving it to the compositor was itself
/// the defect: what the compositor grants is silent, and a canvas pane that
/// scrolls looks exactly the same whether it was sized or clamped.
///
/// **No caller today**, and the note that used to sit here claimed otherwise —
/// that this was read by the same tiers as [`chart_window_size`]. That stopped
/// being true when the boot moved to [`protocol_window_size_for`] over an
/// envelope, because a single [`Layout`] is no longer what the window is sized
/// against. Kept because it is the one-graph spelling of the same arithmetic
/// and the capture tiers are its obvious next caller — but kept honestly, as a
/// convenience with no consumer rather than as load-bearing API.
#[must_use]
pub fn protocol_window_size(layout: &Layout) -> (f32, f32) {
    protocol_window_size_for(layout.width as f32, layout.height as f32)
}

/// [`protocol_window_size`] over a canvas extent that is not any one laid-out
/// graph's.
///
/// The arithmetic is identical and lives here once; the two entry points differ
/// only in what they are asked to fit. A single [`Layout`] is what the capture
/// tiers have — they photograph one picture, and it is the picture their script
/// produced. The live window has to fit something no `Layout` describes: the
/// componentwise envelope of the states it is sized for, because a window is
/// sized once at boot and this binary has neither a zoom nor a fit-to-view to
/// recover with. See
/// [`ProtocolModel::boot_extent`](crate::protocol::ProtocolModel::boot_extent),
/// which is where that envelope is defined, and where the states left to scroll
/// are named and argued.
#[must_use]
pub fn protocol_window_size_for(dag_w: f32, dag_h: f32) -> (f32, f32) {
    let centre = 1.0 - OUTLINE_SHARE - INSPECTOR_SHARE;
    let inset = chrome::pane_content_inset();

    let tile_w = dag_w + 2.0 * inset;
    let w = (tile_w / centre + 2.0 * TILE_GAP + 2.0 * DOCK_INSET).ceil();

    let tile_h = dag_h + 2.0 * inset + TAB_BAR_HEIGHT;
    let h = (tile_h + 2.0 * DOCK_INSET + 2.0 * BAR_HEIGHT).ceil();

    (w, h)
}

// ---------------------------------------------------------------------------
// What the display will actually grant.
// ---------------------------------------------------------------------------

/// `natural` capped, in each axis independently, at what `display` can show.
///
/// [`chart_window_size`] and [`protocol_window_size`] answer what the *content*
/// wants, and until this existed nothing anywhere in the boot path had a term
/// for the screen — the window size bore no relationship to the display at all.
/// That was not theoretical. The protocol view's own envelope asks for 1948
/// points across on the shipped crosswalk, and 3972 in the horizontal flow,
/// against a laptop panel 1512 points wide: the request was routinely larger
/// than the monitor, and what arrived was whatever the compositor chose to
/// grant. A window larger than the monitor is not a bigger window. It is a
/// window with its right-hand edge off the screen.
///
/// Capping is the whole of it, and the direction is the point: a display
/// smaller than the content degrades to **scrolling** — `CanvasPane` wraps the
/// raster in a `ScrollArea`, so every node is still reachable — rather than to
/// a window that cannot be brought onto the screen. A display larger than the
/// content changes nothing; this never grows a window.
///
/// # `display` is the monitor, not its work area
///
/// Said rather than implied, because the two differ by the OS's own bands — a
/// menu bar, a dock, a taskbar — and nothing reachable from here can see that
/// difference. winit 0.30, which eframe 0.35 is built on, exposes a monitor's
/// resolution, position and scale factor and **no work area, on any platform**,
/// so a work-area figure could only be a constant invented here. A window
/// capped at the monitor may therefore still overlap those bands by however
/// tall they are, and the platform resolves that as it always has: macOS
/// constrains a window's height to the visible frame itself, which is the axis
/// the menu bar and the dock take from. The axis it does *not* constrain is the
/// width — and the width is what was off the screen.
///
/// A non-positive extent is "unknown" and leaves its axis alone: a headless
/// context reports no monitor at all, and a caller that answered `0.0` would
/// otherwise ask for a window with no width.
#[must_use]
pub fn window_size_on_display(natural: (f32, f32), display: (f32, f32)) -> (f32, f32) {
    let cap = |want: f32, have: f32| if have > 0.0 { want.min(have) } else { want };
    (cap(natural.0, display.0), cap(natural.1, display.1))
}

/// What one frame's attempt to fit the window to its display found.
///
/// Three outcomes rather than a `bool`, because the caller has to be able to
/// tell "this display is bigger than the window, we are done" from "no monitor
/// has been reported yet, ask again next frame". Collapsing those two retires
/// the check on the first frame of a window that has not yet been mapped —
/// which is the frame most likely to report nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DisplayFit {
    /// No monitor was reported this frame, so nothing was decided. Ask again.
    MonitorUnknown,
    /// The window already fits the monitor it opened on, in both axes.
    Fits,
    /// The window was larger than the monitor, and this smaller size was asked
    /// for instead.
    Shrunk(egui::Vec2),
}

/// Ask the OS to shrink this window to the monitor it opened on, when the size
/// it was created at does not fit.
///
/// **The live window's, and only the live window's.** Deliberately not part of
/// [`MeridianApp::draw`], which is the one frame source every tier shares: the
/// headless capture path and the pixel tier are sized from their content on
/// purpose — a baseline that changed with the reviewer's monitor would be no
/// baseline — and they have no viewport to command anyway.
///
/// # Why this is a frame and not a `ViewportBuilder`
///
/// Because the monitor cannot be reached before the window exists. eframe 0.35
/// offers two pre-creation hooks and neither carries one:
/// `NativeOptions::event_loop_builder` is handed a `winit::EventLoopBuilder`,
/// which has no monitor methods, and `NativeOptions::window_builder` is handed
/// an `egui::ViewportBuilder`, which has none either. winit's monitor list
/// hangs off a *built* `EventLoop`, and winit refuses to build a second one for
/// the life of the process (`EventLoopError::RecreationAttempt`), so a
/// throwaway query loop ahead of `run_native` would cost the real one. The
/// first frame is the earliest point a real monitor is readable, and a measured
/// cap one frame late is worth more than a constant that was never measured.
///
/// # eframe's own clamp is not this one
///
/// It applies `ViewportBuilder::clamp_size_to_monitor_size` at creation and
/// defaults it on, so a request is already capped — at the size of the
/// **largest monitor attached**. A laptop with an ultrawide plugged in
/// therefore has its request checked against the ultrawide, and opens on the
/// laptop panel a window the laptop panel cannot show. That case is the reason
/// this exists, and it is why the cap here is against
/// `ViewportInfo::monitor_size`, which is the monitor this window is actually
/// on.
pub fn fit_window_to_display(ctx: &egui::Context, natural: (f32, f32)) -> DisplayFit {
    let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) else {
        return DisplayFit::MonitorUnknown;
    };
    let fitted = window_size_on_display(natural, (monitor.x, monitor.y));
    if fitted == natural {
        return DisplayFit::Fits;
    }
    let size = egui::vec2(fitted.0, fitted.1);
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    DisplayFit::Shrunk(size)
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
    /// The view the window opens on, when something chose one.
    ///
    /// `None` means nothing chose, so the saved layout's own active view
    /// stands. Two boots are `None`: [`Boot::empty`], which loaded no document
    /// to have a view for, and a start put back by
    /// [`Boot::deferring_to_the_saved_view`], which loaded one but was not
    /// *asked* for.
    ///
    /// A spec on the command line does have an opinion and wins — you asked
    /// for *that*, and being shown the other view because it is where you left
    /// off last time would be the window arguing with you. So does a start the
    /// user picked off the front door, which is the same ask by a different
    /// route.
    pub view: Option<ViewKind>,
    /// The chart view's dashboard.
    pub composed: Composed,
    /// The live, session-holding dashboard behind `composed`, when this boot
    /// loaded one — what arms brushes, clicks and param sliders to re-query.
    /// `None` on the capture tiers' one-shot boots, whose stillness is the
    /// point.
    pub live: Option<crate::pipeline::LiveDashboard>,
    /// The chart spec file `composed` was composed from, when a named file
    /// composed it — what the spec editor opens. `None` for the shipped
    /// starts (embedded fixtures, no file to edit) and for [`Boot::empty`].
    pub spec_path: Option<std::path::PathBuf>,
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
            view: Some(ViewKind::Charts),
            composed,
            live: None,
            spec_path: None,
            protocol: ProtocolInputs::empty(),
            flow: Flow::Vertical,
            focus: None,
        }
    }

    /// Open on the protocol view over `inputs`, with an empty dashboard.
    #[must_use]
    pub fn protocol(inputs: ProtocolInputs, flow: Flow, focus: Option<String>) -> Self {
        Self {
            view: Some(ViewKind::Protocol),
            composed: Composed::empty(),
            live: None,
            spec_path: None,
            protocol: inputs,
            flow,
            focus,
        }
    }

    /// Open on nothing: both documents empty, no view chosen.
    ///
    /// **The no-argument launch.** What stood here was a hardcoded
    /// `examples/dashboard.yaml`, which from the repo root silently opened a
    /// dashboard nobody asked for and from anywhere else exited with a read
    /// error before a window existed. Neither is a first run of a product.
    ///
    /// Every pane of both views answers an empty document with a real empty
    /// state — that is the workbench contract, and
    /// [`audit`](brightfield_workbench::audit) is what makes it true rather
    /// than remembered — so this is not a blank window. It is the front door,
    /// and the panes that can be filled by something the binary ships offer it
    /// (see [`crate::starts`]).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            view: None,
            composed: Composed::empty(),
            live: None,
            spec_path: None,
            protocol: ProtocolInputs::empty(),
            flow: Flow::Vertical,
            focus: None,
        }
    }

    /// Open on whatever [`crate::starts`] calls `id`, in the view it fills.
    ///
    /// # Errors
    ///
    /// If `id` is not a start this build ships, or the embedded fixture fails
    /// to load.
    pub fn start(id: &str, flow: Flow) -> Result<Self, String> {
        Ok(match crate::starts::load(id)? {
            crate::starts::Opened::Charts(composed) => Self::charts(*composed),
            crate::starts::Opened::Protocol(inputs) => Self::protocol(*inputs, flow, None),
        })
    }

    /// The same documents, with the view opinion dropped.
    ///
    /// The difference between a start the user *picked* and a start the layout
    /// file *remembered*. Both load the same document, and only the first is
    /// an ask to be looking at it: the file separately records which view the
    /// window was left on, that record is the deliberate one, and a restore
    /// that overrode it would make the persisted active view dead for every
    /// returning user. See [`crate::startup::opening_boot`], which is the only
    /// caller and the place the precedence is stated.
    #[must_use]
    pub fn deferring_to_the_saved_view(mut self) -> Self {
        self.view = None;
        self
    }

    /// The view this boot's size, title and summary are answered for: the one
    /// it named, or `fallback` when it named none.
    ///
    /// **The fallback is a parameter because only the caller knows it.** This
    /// used to be a `const fn` with `ViewKind::Charts` baked in, which was
    /// harmless while every `Boot` named a view and became wrong the moment
    /// [`Boot::deferring_to_the_saved_view`] made `None` the *normal* case: a
    /// restored crosswalk answered "Brightfield" and "composed 0x0 dashboard"
    /// for a 34-node protocol graph, and since the window title is set once
    /// from that string the wrong one survived the whole session.
    ///
    /// `main` has the saved layout in hand before it asks any of the three, so
    /// it passes the view that will actually be drawn. The capture tiers build
    /// their boots through [`Boot::open`], [`Boot::charts`] or
    /// [`Boot::protocol`], all of which name a view, so what they pass is
    /// unreachable — and saying it at the call site is what makes that
    /// checkable rather than assumed.
    #[must_use]
    pub const fn view_or(&self, fallback: ViewKind) -> ViewKind {
        match self.view {
            Some(view) => view,
            None => fallback,
        }
    }

    /// Whether this boot loaded no document at all.
    ///
    /// The window arithmetic needs this: [`chart_window_size`] read outwards
    /// from a 0x0 dashboard asks for a window a few tens of points wide, which
    /// is a correct answer to the wrong question. A boot with nothing in it
    /// has no content to derive a window from, so it takes the saved geometry
    /// — or, on a first run, [`WindowGeometry`]'s default.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.composed.width == 0
            && self.composed.height == 0
            && self.protocol.graph_full.nodes.is_empty()
    }

    /// Read `spec` and load whichever document it describes.
    ///
    /// **The one place a spec is classified.** It used to be two — the live
    /// binary and the shot binary each sniffed the file and each branched into
    /// its own shell — and the two branches then had to agree about an
    /// environment gate, a window size and a summary line that neither shared.
    ///
    /// # Errors
    /// A message if the file cannot be read, if it is a run-less protocol
    /// manifest and this process has not opted in — see
    /// [`crate::protocol::run_less_manifest_refusal`], which states that rule
    /// once for both callers — or if the pipeline rejects it.
    pub fn open(spec: &str, flow: Flow, focus: Option<String>) -> Result<Self, String> {
        Self::open_sampled(spec, flow, focus, None)
    }

    /// [`Boot::open`] at an explicit pushed-down sample rate.
    ///
    /// `None` is [`Boot::open`] exactly — no clause, no extra query, the same
    /// bytes. `Some(rate)` opens the same document drawing one row in
    /// `rate.modulus()`, with the notice in the plot's own ink.
    ///
    /// # Errors
    ///
    /// As [`Boot::open`].
    pub fn open_sampled(
        spec: &str,
        flow: Flow,
        focus: Option<String>,
        sample: Option<SampleRate>,
    ) -> Result<Self, String> {
        let text = std::fs::read_to_string(spec).map_err(|e| format!("read {spec}: {e}"))?;
        if brightfield_protocol::is_protocol_manifest(&text) {
            if !crate::protocol::offline_optin() {
                return Err(crate::protocol::run_less_manifest_refusal(spec));
            }
            return Ok(Self::protocol(load_protocol_offline(spec)?, flow, focus));
        }
        // A chart spec named on the command line boots **live**: the session
        // is held behind the document, so a brush, a click or a param slider
        // resolves to a pushed predicate and a re-query rather than a still
        // frame. The capture tiers build their boots through [`Boot::charts`]
        // and stay one-shot, which is what keeps a baseline a baseline.
        let (live, composed) = crate::pipeline::live_spec_sampled(spec, sample)?;
        // The command-line surface for the same diagnostics the window raises
        // as banners. Both, not either: a headless capture never draws a
        // banner, and `brightfield-shot` rendering a chart with a mark
        // silently missing and saying nothing is the same defect one tier
        // down.
        for line in composed.diagnostics.lines() {
            eprintln!("{spec}: {line}");
        }
        let mut boot = Self::charts(composed);
        boot.live = Some(live);
        // The file the dashboard was composed from rides along, so the spec
        // editor opens on it. Only this constructor sets it: it is the one
        // place a chart document comes from a *file* rather than an embedded
        // start or a test's in-memory compose.
        boot.spec_path = Some(std::path::PathBuf::from(spec));
        Ok(boot)
    }

    /// The window this boot asks for, in logical points — `view`'s natural
    /// size over the documents this boot loaded.
    ///
    /// One window means one size, and the two views want very different ones.
    /// The opening view's is the answer because it is the only one that is a
    /// fact at the moment the window is created: the other view's document is
    /// usually empty, and sizing to the larger of the two would open a window
    /// mostly full of an empty state nobody asked for. Switching views does not
    /// resize — the user's window is theirs once it exists, and both views
    /// reflow or scroll inside whatever they are given.
    ///
    /// Answered here rather than on [`MeridianApp`] because a window has to be
    /// sized before it can be created, and the app cannot be built until eframe
    /// has handed over a device.
    ///
    /// `view` is a parameter for the reason [`Boot::view_or`] gives: a boot
    /// that named no view has no business inventing one when the caller
    /// already knows which one will be drawn.
    ///
    /// **A saved layout outranks this.** The live binary consults it only when
    /// nothing was restored, and never for a [`Boot::empty`] — see
    /// [`Boot::is_empty`]. The headless capture path and the pixel tier have
    /// no saved layout and no user to have arranged one, so this stays their
    /// only answer.
    ///
    /// **And the display outranks the answer.** *Natural* is the operative
    /// word: this is what the content wants, in a vacuum, and on the protocol
    /// view it is routinely wider than a laptop panel. The live binary caps it
    /// at the monitor the window opened on — [`window_size_on_display`] — and
    /// the capture tiers, which have no monitor and must not acquire one, do
    /// not.
    #[must_use]
    pub fn window_size(&self, view: ViewKind) -> (f32, f32) {
        match view {
            ViewKind::Charts => chart_window_size(&self.composed),
            ViewKind::Protocol => {
                // The ENVELOPE, not the boot canvas. Sizing to the boot canvas
                // fitted the graph the window opens on and nothing else: every
                // state one keystroke away overflowed and stayed overflowed,
                // because a window is sized once and never resized. See
                // `ProtocolModel::boot_extent`, which also names the two states
                // the envelope deliberately leaves to scroll.
                let (w, h) = ProtocolModel::boot_extent(&self.protocol, self.flow);
                protocol_window_size_for(w as f32, h as f32)
            }
        }
    }

    /// The window title: `view`'s subject over the documents this boot loaded.
    ///
    /// The same answer [`MeridianApp::title`] gives once the window exists,
    /// provided `view` is the view that will be drawn — which is the caller's
    /// job to supply and `main`'s reason for resolving it against the restored
    /// layout first. It matters more here than it looks: `main` hands this
    /// string to `eframe::run_native`, which is where the OS window's title
    /// comes from, and the only things that send a `ViewportCommand::Title`
    /// afterwards are opening a start (`open_start`) and going home
    /// (`open_home`) — both private, both re-titling from the documents they
    /// just changed. A title that is wrong at this call stays wrong until one
    /// of those runs.
    #[must_use]
    pub fn title(&self, view: ViewKind) -> String {
        match view {
            ViewKind::Charts => self
                .composed
                .title
                .clone()
                .unwrap_or_else(|| "Brightfield".to_string()),
            ViewKind::Protocol => format!("Protocol · {}", self.protocol.protocol),
        }
    }

    /// One line describing what `view` was given, for the binaries' stderr.
    #[must_use]
    pub fn describe(&self, view: ViewKind) -> String {
        if self.is_empty() {
            return "nothing open".to_string();
        }
        match view {
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
    /// Whether the Home button was pressed — the return to the front door.
    home: bool,
    /// Each switcher control's rect — see [`MeridianApp::switcher`].
    switcher: Vec<(ViewKind, egui::Rect)>,
    /// The Home button's rect, when the bar drew one — see
    /// [`MeridianApp::home_rect`]. `None` on the front door, which draws no
    /// Home button because it is already home.
    home_rect: Option<egui::Rect>,
}

/// Which grammar overlay is open over the workspace, holding the live
/// [`Picker`] over its delegate. In immediate mode "a modal is open" is the
/// host's state — the layer itself is stateless — so this enum *is* the
/// modal slot: `None` means no overlay, and at most one is ever open.
///
/// While one is open, no bare key reaches the protocol grammar (the
/// no-bare-under-overlay invariant): [`MeridianApp::draw`] gates the model's
/// event feed on this slot being empty.
///
/// The argument prompt ([`crate::overlays::ArgPrompt`]) and column jump are
/// deliberately absent: their opening verbs (`add-mark` / `set-channel`)
/// need a focused plot and an applied `ChartEdit`, which is the chart view's
/// editing bridge — not landed in this shell yet. The delegates are built
/// and tested; the slot grows their arms when the bridge does.
enum Overlay {
    /// The command palette (`space`): query + ranked corpus at the active
    /// altitude.
    Palette(Picker<CommandPalette>),
    /// The keyboard help sheet (`?`): grouped, read-only.
    Help(Picker<HelpSheet>),
    /// The node jump (`/`): fuzzy finder over the graph in view.
    Jump(Picker<JumpToNode>),
}

/// The registry-bound keystrokes that open overlays, resolved once at boot.
///
/// Tokens, not `egui::Key`s, because the registry speaks tokens; the one
/// token → key mapping is [`consume_token`]. Resolved from
/// `brightfield_keys::registry()` so the shell opens its overlays on the keys
/// the registry declares — the shell may not invent bindings any more than it
/// may invent verbs.
struct OverlayKeys {
    palette: Option<&'static str>,
    help: Option<&'static str>,
    jump: Option<&'static str>,
}

impl OverlayKeys {
    fn from_registry() -> Self {
        let reg = brightfield_keys::registry();
        let primary = |longname: &str| {
            reg.iter()
                .find(|v| v.longname == longname)
                .and_then(brightfield_keys::VerbEntry::primary_key)
        };
        Self {
            palette: primary("open-palette"),
            help: primary("open-help"),
            jump: primary("focus-jump"),
        }
    }
}

/// Consume the pressed key a registry keystroke token names, if it is down
/// this frame. Only the tokens the shell actually wires live — the overlay
/// openers and open-home — are mapped; an unmapped token is simply never
/// consumed, which fails safe: nothing opens, and nothing else changes.
fn consume_token(ctx: &egui::Context, token: &str) -> bool {
    use egui::{Key, Modifiers};
    match token {
        "space" => ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space)),
        "/" => ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Slash)),
        // `?` is shift-`/` on most layouts, its own key on some — egui
        // reports the logical key either way, with or without the shift.
        "?" => ctx.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::Questionmark)
                || i.consume_key(Modifiers::SHIFT, Key::Questionmark)
        }),
        "cmd-shift-h" => {
            ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::H))
        }
        // The navigation family. Bare keys, and mapped here for the same
        // reason the overlay openers are: the shell may not invent a binding,
        // so the token comes off the registry and only its egui spelling lives
        // in this table.
        "left" => ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft)),
        "right" => ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight)),
        "up" => ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)),
        "down" => ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)),
        // `=` and `+` are one key: egui reports the logical key, and a reader
        // pressing shift to say "bigger" means the same verb.
        "=" => ctx.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::Equals)
                || i.consume_key(Modifiers::SHIFT, Key::Equals)
                || i.consume_key(Modifiers::NONE, Key::Plus)
        }),
        "-" => ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Minus)),
        "x" => ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::X)),
        "0" => ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Num0)),
        _ => false,
    }
}

/// The navigation family's `(keystroke token, longname)` pairs, off the
/// registry. A verb the registry leaves unbound simply does not appear, so it
/// stays reachable from the palette and unreachable by key — which is what
/// "unbound" means.
fn navigation_bindings() -> Vec<(&'static str, &'static str)> {
    let reg = brightfield_keys::registry();
    crate::navigation::verb::ALL
        .iter()
        .filter_map(|longname| {
            let entry = reg.iter().find(|v| v.longname == *longname)?;
            Some((entry.primary_key()?, entry.longname))
        })
        .collect()
}

/// Perform a navigation verb on the chart document, or report that this is not
/// one. The one place a longname becomes a frame movement.
fn navigation_verb(doc: &mut crate::app::ChartDoc, longname: &str) -> bool {
    use crate::navigation::{verb, KEY_PAN_FRACTION, KEY_ZOOM_FACTOR};
    let f = KEY_PAN_FRACTION;
    match longname {
        verb::PAN_LEFT => doc.pan_view(f, 0.0),
        verb::PAN_RIGHT => doc.pan_view(-f, 0.0),
        verb::PAN_UP => doc.pan_view(0.0, f),
        verb::PAN_DOWN => doc.pan_view(0.0, -f),
        verb::ZOOM_IN => doc.zoom_view(KEY_ZOOM_FACTOR),
        verb::ZOOM_OUT => doc.zoom_view(1.0 / KEY_ZOOM_FACTOR),
        verb::CYCLE_AXIS_LOCK => {
            doc.cycle_axis_lock();
            true
        }
        verb::RESET_EXTENT => {
            doc.reset_navigation();
            true
        }
        _ => return false,
    };
    true
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
    /// The live layout — the arrangement, the window geometry, and what is
    /// open — alongside a clone of what is durably on disk.
    ///
    /// The workspace lives *inside* the tracker rather than beside it because
    /// the tracker's whole design is that `live` **is** the thing the UI
    /// mutates: the change signal is a plain `live != saved` compare, and a
    /// workspace held next to the tracker would have to be copied in and out
    /// every frame for that compare to mean anything.
    layout: DirtyTracker,
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
    /// Where the top bar drew the Home button in the last frame this window
    /// drew, in window-space logical points — recorded for the reason
    /// [`Self::switcher`] is, and read back through [`MeridianApp::home_rect`].
    /// `None` on a frame the bar drew no Home button (the front door).
    home_button: Option<egui::Rect>,
    /// Where each empty pane drew the button that resolves it, in window-space
    /// logical points — recorded for exactly the reason [`Self::switcher`] is,
    /// and read back through [`MeridianApp::affordance_rect`].
    ///
    /// On a frame the front door drew instead of the dock, the door records
    /// the view-filling starts' cards here under the pane keys those starts
    /// fill — see [`MeridianApp::front_door_ui`] — so "where is the way in
    /// that fills this pane" has one answer wherever it was drawn.
    affordances: Vec<(PaneKey, egui::Rect)>,
    /// The front door's gallery textures, decoded once from the shipped
    /// thumbnail bytes on the first frame the door draws. Kept for the app's
    /// life: a `TextureHandle` is an `Arc`, and four small thumbnails are not
    /// worth a re-decode if the user comes back to an emptied window.
    door_thumbs: Vec<(&'static str, egui::TextureHandle)>,
    /// Where the front door drew each start's gallery card, by start id —
    /// the test hook the door's clicks are aimed through, exactly as
    /// [`Self::affordances`] is for pane empty states. Cleared on frames the
    /// door did not draw.
    door_cards: Vec<(&'static str, egui::Rect)>,
    /// Where the front door drew the Continue zone's resolving button, when
    /// the layout remembered work to continue. `None` on a first run — the
    /// zone is the morph, not a fixture.
    door_continue: Option<egui::Rect>,
    /// Where the front door drew the Start zone's keyboard-help control.
    door_help: Option<egui::Rect>,
    /// The one modal slot — see [`Overlay`].
    overlay: Option<Overlay>,
    /// The keystrokes that open overlays, read off the registry at boot.
    overlay_keys: OverlayKeys,
    /// The `open-home` keystroke token, read off the registry at boot — the
    /// shell invents no binding, it only wires the one the registry declares.
    /// Kept out of [`OverlayKeys`] on purpose: open-home is a runtime
    /// dispatch, not an overlay opener, and that struct's contents are pinned
    /// by `the_overlay_keys_come_from_the_registry`.
    home_binding: Option<&'static str>,
    /// The navigation family's keystroke tokens paired with their verb
    /// longnames, read off the registry at boot — same rule as
    /// [`Self::home_binding`]: the shell wires the binding the registry
    /// declares and invents none. Empty for a verb the registry leaves unbound.
    nav_bindings: Vec<(&'static str, &'static str)>,
    /// The per-session palette recency: verbs run from the palette rank
    /// higher on its next empty-query open. Session-scoped by design (the
    /// sanctioned v1 simplification); it resets each launch.
    recency: RecencyCounter,
    /// Persistent, id-deduplicated banners. A source that re-fails raises
    /// under the same composite id and *replaces* its banner — never stacks.
    notifications: NotificationLayer,
    /// The chart fault the interaction banner was last raised for, so it is
    /// raised on the frame the fault appears and not on every frame after.
    ///
    /// `raise` replaces in place, so re-raising an unchanged fault every frame
    /// silently undoes the dismiss the user just clicked — the × works and the
    /// banner is back before the next paint. For a fault that cannot clear
    /// without editing the spec, which is exactly the mistyped column this
    /// banner exists to report, that makes it permanent furniture. A fault that
    /// *changes* still re-raises, because it is new information — and the
    /// headline counts as part of it, so a pan onto empty space still speaks
    /// after a mistyped column has been dismissed.
    last_chart_fault: Option<ChartFault>,
    /// The banner ids the CURRENT chart document's load diagnostics raised.
    ///
    /// Held so opening a different document can take the previous document's
    /// diagnostics down. Without it the banners would be sticky in the one way
    /// that matters: a user who opens a spec with an unrenderable mark, reads
    /// the banner, then opens a clean one would go on being warned about a
    /// document they are no longer looking at.
    diagnostic_banners: Vec<NotificationId>,
    /// Transient, self-expiring toasts — confirmations, not conditions.
    toasts: ToastLayer,
    /// What the status rail drew last frame — the ids in draw order and any
    /// dismissals — recorded for the reason [`Self::switcher`] is: a headless
    /// test that asks "did the rail say it?" reads this rather than a second
    /// copy of the composing logic. Empty on a frame the rail drew nothing,
    /// which is most frames — the rail is quiet when there is nothing to say.
    rail: chrome::StatusDrawn,
}

impl MeridianApp {
    /// Build the window over `boot`, rastering each view through its own host.
    ///
    /// Two hosts rather than one, one per document, because a document owns the
    /// canvas it rasters into — that is the rule the whole item contract hangs
    /// off. Both are built from the same wgpu device: `EguiCanvasHost` holds
    /// `Arc` handles, so a second one costs a `VelloRenderer` and nothing else.
    /// **Never reads the layout file.** The default arrangement is built here,
    /// which is what keeps the headless capture path — `capture_png`, and so
    /// `brightfield-shot` and the whole pixel tier — off the developer's real
    /// `workspace-layout.json`. The live window uses
    /// [`MeridianApp::with_layout`] and passes one in.
    #[must_use]
    pub fn new(
        boot: Boot,
        chart_host: EguiCanvasHost,
        protocol_host: EguiCanvasHost,
        mode: Mode,
    ) -> Self {
        Self::with_layout(
            boot,
            crate::startup::default_layout(),
            chart_host,
            protocol_host,
            mode,
        )
    }

    /// The window over `boot`, arranged as `layout` says.
    ///
    /// The one constructor that takes a restored layout, and the layout is a
    /// **parameter** rather than something read here: a constructor that read
    /// the file itself would read and write the developer's real config
    /// directory on every `cargo test`.
    #[must_use]
    pub fn with_layout(
        boot: Boot,
        layout: SavedLayout,
        chart_host: EguiCanvasHost,
        protocol_host: EguiCanvasHost,
        mode: Mode,
    ) -> Self {
        let mut doc = ChartDoc::new(boot.composed, chart_host);
        if let Some(live) = boot.live {
            doc.attach_live(live);
        }
        doc.spec_path = boot.spec_path;
        doc.wire_watch();
        let model = ProtocolModel::new(boot.protocol, boot.flow);
        Self::assemble(
            boot.view,
            boot.focus,
            layout,
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
    ///
    /// Like [`MeridianApp::new`], it never reads the layout file.
    #[must_use]
    pub fn headless(boot: Boot, mode: Mode) -> Self {
        Self::headless_with_layout(boot, crate::startup::default_layout(), mode)
    }

    /// The device-free window over a layout that was restored from somewhere.
    ///
    /// The GPU-free twin of [`MeridianApp::with_layout`], and the constructor
    /// the layout wiring is asserted through: it is the only way to ask what a
    /// restored arrangement does to a real frame without a device or a config
    /// directory.
    #[must_use]
    pub fn headless_with_layout(boot: Boot, layout: SavedLayout, mode: Mode) -> Self {
        let mut doc = ChartDoc::headless(boot.composed);
        if let Some(live) = boot.live {
            doc.attach_live(live);
        }
        doc.spec_path = boot.spec_path;
        doc.wire_watch();
        let model = ProtocolModel::new(boot.protocol, boot.flow);
        Self::assemble(
            boot.view,
            boot.focus,
            layout,
            doc,
            ProtocolDoc::headless(model),
            mode,
        )
    }

    fn assemble(
        view: Option<ViewKind>,
        focus: Option<String>,
        layout: SavedLayout,
        chart_doc: ChartDoc,
        mut protocol_doc: ProtocolDoc,
        mode: Mode,
    ) -> Self {
        // Both views' vocabularies, published before any layout file could be
        // read. Idempotent, and both are needed whichever view boots: a
        // `PaneKey` naming a pane of the view that did not boot has to
        // deserialise too, or a saved layout loads as corrupt. `startup`
        // publishes them again ahead of the read that happens before this
        // window exists; these stay because the headless tiers never go
        // through `startup` at all.
        crate::app::publish_item_ids();
        crate::protocol::publish_item_ids();

        if let Some(id) = focus {
            protocol_doc.model.select_id(id);
        }

        let charts = chart_registry();
        let protocol = protocol_registry();

        let mut layout = DirtyTracker::new(layout);
        // Something that asked for a view wins; a boot with no opinion keeps
        // the view the saved layout was left on. See `Boot::view` for which
        // boots have an opinion — notably a *restored* start does not.
        //
        // Deliberately after `DirtyTracker::new`: a boot that overrides the
        // active view has genuinely moved the window off what the file says,
        // and that difference should be written. A boot with no opinion
        // touches nothing here, so it is clean from construction and a launch
        // that restores a session writes nothing until the user moves
        // something.
        if let Some(view) = view {
            layout.workspace_mut().set_active(view);
        }

        // The restored tab strip is the authority over the model's default,
        // not the other way round. `ProtocolModel` boots with its sheet shut,
        // `draw` makes the strip authoritative from that flag on every frame
        // it draws this view, and the strip's active tab is part of the
        // serialised tree — so without this line a file that recorded the
        // Steps sheet is overwritten with Canvas before any frame can read it,
        // and the overwrite is a tile-tree mutation, so a launch nobody
        // touched goes dirty and the debounce writes the reverted arrangement
        // back. The restore was one-way lossy and it rewrote the file to say
        // so. Held by `a_restored_steps_tab_survives_the_first_frame`.
        if let Some(show) = steps_tab_is_active(layout.live().workspace.tree(ViewKind::Protocol)) {
            protocol_doc.model.set_show_sheet(show);
        }

        let mut app = Self {
            layout,
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
            home_button: None,
            affordances: Vec::new(),
            door_thumbs: Vec::new(),
            door_cards: Vec::new(),
            door_continue: None,
            door_help: None,
            overlay: None,
            overlay_keys: OverlayKeys::from_registry(),
            home_binding: brightfield_keys::registry()
                .iter()
                .find(|v| v.longname == "open-home")
                .and_then(brightfield_keys::VerbEntry::primary_key),
            nav_bindings: navigation_bindings(),
            recency: RecencyCounter::new(),
            notifications: NotificationLayer::new(),
            last_chart_fault: None,
            diagnostic_banners: Vec::new(),
            toasts: ToastLayer::new(),
            rail: chrome::StatusDrawn::default(),
        };
        // Say what this document's load found, before its first frame. A
        // diagnostic that waits for the user to go looking is a diagnostic
        // that does not exist.
        app.say_load_diagnostics();
        app
    }

    /// Raise a banner for everything the chart document's load had to say,
    /// and take down whatever the previous document's load had said.
    ///
    /// One banner per distinct unrenderable feature, titled with its Mosaic
    /// **wire name** — the string the reader wrote in their own file, so the
    /// banner names something they can search for — and bodied with where it
    /// appeared and how often. Nine `voronoi` marks are one problem, so nine
    /// occurrences make one banner; two different unrenderable features are
    /// two problems and make two.
    ///
    /// Then one banner carrying every remaining diagnostic: the parse and
    /// analysis warnings that four spec-load entry points used to drop on the
    /// floor. Grouped rather than one-per-line because these are degradations
    /// rather than failures and a wall of separate banners would bury the
    /// blockers above them — but every distinct line is in the body, because
    /// a warning summarised into a count has not been told.
    fn say_load_diagnostics(&mut self) {
        for id in std::mem::take(&mut self.diagnostic_banners) {
            self.notifications.dismiss(id);
        }
        // The dismissed-fault memory belongs to the document that raised it.
        // Two documents can produce a byte-identical fault, and without this a
        // fault dismissed on the first would open the second already silenced —
        // narrow, since any fault-free frame resets it, but free to close here
        // beside the banners it travels with.
        self.last_chart_fault = None;
        let diagnostics = self.charts.doc.composed.diagnostics.clone();
        let document = diagnostics
            .source
            .clone()
            .unwrap_or_else(|| "this spec".to_string());

        for name in diagnostics.blocking_names() {
            let occurrences: Vec<&brightfield_conformance::Diagnostic> = diagnostics
                .blocking()
                .into_iter()
                .filter(|d| d.wire_name == name)
                .collect();
            let mut surfaces: Vec<&str> = Vec::new();
            for d in &occurrences {
                if !surfaces.contains(&d.surface) {
                    surfaces.push(d.surface);
                }
            }
            let id = NotificationId::composite("spec-diagnostics", &name);
            self.notifications.raise(
                Notification::new(id, Severity::Error, format!("Cannot render `{name}`")).body(
                    format!(
                        "{} in {document}, {} — brightfield does not draw it, so that part of \
                         the chart is missing.",
                        plural(occurrences.len(), "occurrence", "occurrences"),
                        surfaces.join(" / "),
                    ),
                ),
            );
            self.diagnostic_banners.push(id);
        }

        let mut lines: Vec<String> = Vec::new();
        for d in diagnostics.advisory() {
            let line = d.to_string();
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
        if !lines.is_empty() {
            let id = NotificationId::new("spec-advisories");
            self.notifications.raise(
                Notification::new(
                    id,
                    Severity::Warning,
                    format!(
                        "{} in {document} had no effect",
                        plural(lines.len(), "instruction", "instructions")
                    ),
                )
                .body(lines.join("\n")),
            );
            self.diagnostic_banners.push(id);
        }
    }

    /// Raise a banner for a chart the engine would not fully draw, and take it
    /// down again the moment it would.
    ///
    /// A load diagnostic is what a spec had to say before anyone touched it;
    /// this is what a spec had to say only once someone did. The two cannot be
    /// merged, because the failure this reports is **not decidable at load**:
    /// a cross-filter column that a `query:` source does not expose has no
    /// schema to check against until DuckDB has one, so the binder's rejection
    /// is the first moment the fact exists. It used to go to stderr, which for
    /// a windowed application means the chart quietly emptied itself under a
    /// control that claimed to be filtering it and nothing on screen said so.
    ///
    /// One banner, replaced in place while the fault persists — a drag emits a
    /// value per frame and a stack of identical banners would bury the picture
    /// it is about.
    ///
    /// The headline comes from the fault rather than from here. A mark the
    /// binder rejected and a pan onto empty space are both "the gesture did not
    /// change the picture" and are not the same news, and the document is where
    /// the difference is known — see [`ChartFault`].
    fn say_interaction_fault(&mut self) {
        let id = NotificationId::new("chart-interaction-fault");
        match self.charts.doc.chart_fault() {
            // Raise on the frame the fault appears, and on any frame it says
            // something different. NOT on every frame it persists: `raise`
            // replaces in place, so that would put the banner back one frame
            // after the user dismissed it, and a mistyped column does not clear
            // until the spec is edited.
            Some(fault) if self.last_chart_fault.as_ref() != Some(&fault) => {
                self.last_chart_fault = Some(fault.clone());
                self.notifications
                    .raise(Notification::new(id, Severity::Error, fault.title).body(fault.detail));
            }
            Some(_) => {}
            None => {
                self.last_chart_fault = None;
                self.notifications.dismiss(id);
            }
        }
    }

    /// What the current chart document's load had to say — the read-only hook
    /// a headless test asks "did the window receive it?" through.
    #[must_use]
    pub fn load_diagnostics(&self) -> &brightfield_conformance::LoadDiagnostics {
        &self.charts.doc.composed.diagnostics
    }

    /// Replace the chart document with `composed`, and say what its load
    /// found.
    ///
    /// **The only sanctioned way to swap the chart document at runtime.**
    /// `ChartDoc::open` alone cannot do it: the document does not own the
    /// banner layer, so a caller reaching past this to open a document
    /// directly inherits the previous spec's banners and publishes none of
    /// its own — the original defect, re-made one level down.
    ///
    /// That is not a hypothetical. `open_home` did exactly that: it emptied
    /// the chart document through `ChartDoc::open` and left the outgoing
    /// spec's `Cannot render …` banner standing on the front door, for a
    /// document nobody had open any more. There are two callers now
    /// (`open_start` and `open_home`) and both go through here; a third that
    /// does not is the same bug a third time, so the reviewer's question about
    /// any new chart-document swap is "does it call this".
    pub fn open_chart(&mut self, composed: Composed) {
        self.charts.doc.open(composed);
        self.say_load_diagnostics();
    }

    /// The arrangement the UI reads.
    fn ws(&self) -> &Workspace {
        &self.layout.live().workspace
    }

    /// The arrangement the UI mutates.
    fn ws_mut(&mut self) -> &mut Workspace {
        self.layout.workspace_mut()
    }

    /// The view currently drawn.
    #[must_use]
    pub fn active(&self) -> ViewKind {
        self.ws().active()
    }

    /// The window title: the active view's subject.
    ///
    /// [`Boot::title`] is the same question answered before the window exists,
    /// and the two agree exactly when it is asked for the view this window will
    /// draw — which is why `Boot::title` takes that view rather than assuming
    /// one. It matters that they agree: `main` hands `Boot::title`'s answer to
    /// `eframe::run_native`, and the runtime `ViewportCommand::Title`s in the
    /// workspace — in `open_start` and `open_home` — re-title from this same
    /// method, so a restored session that reaches neither keeps `Boot::title`'s
    /// answer. `a_restored_session_is_titled_for_the_view_it_draws` asserts
    /// the agreement rather than either answer, because a literal on both
    /// sides would go on matching itself after either drifted.
    #[must_use]
    pub fn title(&self) -> String {
        // The front door spans both views, so it has no view subject to name —
        // and the top bar draws this unconditionally. Reaching the door from
        // the Protocol view would otherwise show "Protocol · " (the emptied
        // graph's blank name) in the visible chrome for the whole visit. A
        // cold-boot door is already this string (an empty `ChartDoc` titles
        // "Brightfield"), so this only fixes the reached-from-Protocol case.
        if self.front_door_is_live() {
            return "Brightfield".to_string();
        }
        match self.ws().active() {
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

    /// The rect the top bar's Home button occupied in the last frame this
    /// window drew, or `None` on a frame it drew none — the front door draws
    /// no Home button, because it is already home. Recorded and read back for
    /// the reason [`MeridianApp::switcher_rect`] is: the test that proves Home
    /// is reachable has to click it where it was actually laid out.
    #[must_use]
    pub fn home_rect(&self) -> Option<egui::Rect> {
        self.home_button
    }

    /// The rect the empty state of `pane` drew its resolving button at, in the
    /// last frame this window drew.
    ///
    /// `None` for a pane that is not empty, or whose empty state offers no
    /// action. This is how the test that a front door is *reachable* aims its
    /// click — and the empty state is drawn by the chrome, not by the pane, so
    /// no `Item` can record it.
    #[must_use]
    pub fn affordance_rect(&self, pane: PaneKey) -> Option<egui::Rect> {
        self.affordances
            .iter()
            .find(|(k, _)| *k == pane)
            .map(|(_, r)| *r)
    }

    /// Whether the next frame this window draws is the front door: nothing
    /// open in either view, so the window's answer is an invitation rather
    /// than a dock of empty instruments.
    ///
    /// Not a mode and not a setting — a fact about the documents, recomputed
    /// every frame, which is why the door needs no dismissal: it is
    /// outcompeted by content the moment either view has any.
    #[must_use]
    pub fn front_door_is_live(&self) -> bool {
        self.charts.doc.is_empty() && !self.protocol.doc.model.has_assets()
    }

    /// The rect the front door drew the gallery card for the start `id` at,
    /// in the last frame this window drew.
    ///
    /// `None` when the door did not draw — which is the morph's other half,
    /// so a test that asserts the door is *gone* asks this too. Recorded for
    /// the reason [`MeridianApp::affordance_rect`] records pane buttons: the
    /// test that proves a card opens something has to click it where it was
    /// actually laid out.
    #[must_use]
    pub fn front_door_card_rect(&self, id: &str) -> Option<egui::Rect> {
        self.door_cards
            .iter()
            .find(|(card, _)| *card == id)
            .map(|(_, r)| *r)
    }

    /// The rect of the front door's Continue button, when the last frame drew
    /// one — it exists only once the layout remembers work to continue.
    #[must_use]
    pub fn front_door_continue_rect(&self) -> Option<egui::Rect> {
        self.door_continue
    }

    /// The rect of the front door's keyboard-help control, when the last
    /// frame drew the door.
    #[must_use]
    pub fn front_door_help_rect(&self) -> Option<egui::Rect> {
        self.door_help
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

    /// The chart view's document, read-only — what a test asks whether the
    /// front door's second click actually filled.
    #[must_use]
    pub fn chart_doc(&self) -> &ChartDoc {
        &self.charts.doc
    }

    /// The protocol view's document, read-only. The twin of
    /// [`Self::chart_doc`].
    #[must_use]
    pub fn protocol_doc(&self) -> &ProtocolDoc {
        &self.protocol.doc
    }

    /// The chart view's document, mutably — the seam an embedder (or a test)
    /// reaches the document's own state through between frames: its activity
    /// log, its file watcher, a param. Never handed to an [`Item`] — the
    /// no-document-handle rule is about panes, and this is not one.
    ///
    /// [`Item`]: brightfield_workbench::Item
    pub fn chart_doc_mut(&mut self) -> &mut ChartDoc {
        &mut self.charts.doc
    }

    // -----------------------------------------------------------------------
    // The layout file
    // -----------------------------------------------------------------------

    /// The live layout: the arrangement, the window geometry, and what is open.
    #[must_use]
    pub fn layout(&self) -> &SavedLayout {
        self.layout.live()
    }

    /// Record the window's current size and position into the live layout.
    ///
    /// Called by the **host** and not from [`Self::draw`], so the headless
    /// tiers cannot accidentally write a test harness's screen rect into a
    /// layout: a headless context reports no viewport rect at all, and the
    /// fallback below would take the whole screen for the window.
    ///
    /// `inner_rect`/`outer_rect` are `Option` and are `None` on some platforms,
    /// so neither is unwrapped. A position that cannot be read simply is not
    /// recorded, which the layout file already permits.
    pub fn observe_window(&mut self, ctx: &egui::Context) {
        let (inner, outer, viewport_rect) = ctx.input(|i| {
            (
                i.viewport().inner_rect,
                i.viewport().outer_rect,
                i.viewport_rect(),
            )
        });
        let size = inner.map_or(viewport_rect, |r| r).size();
        self.layout.live_mut().window = WindowGeometry {
            size: [size.x, size.y],
            position: outer.map(|r| [r.min.x, r.min.y]),
        };
    }

    /// Tick the layout's debounced save at `now_ms`, writing to `path` when
    /// the layout has been changed and still long enough.
    ///
    /// Returns the write's result when one was attempted. Never a hard
    /// failure: a failed write leaves the tracker dirty so the next tick, or
    /// the exit flush, retries it.
    pub fn poll_layout(
        &mut self,
        now_ms: u64,
        path: &std::path::Path,
    ) -> Option<Result<(), String>> {
        self.layout.poll(now_ms, path)
    }

    /// Whether a debounced save is counting down.
    ///
    /// The host has to ask, because eframe paints on input rather than
    /// continuously: without a [`request_repaint_after`](egui::Context::request_repaint_after)
    /// while this is true, a user who drags a splitter and then leaves the
    /// window alone generates no further frames, the countdown never fires,
    /// and the change survives only as far as the exit flush.
    #[must_use]
    pub fn layout_armed(&self) -> bool {
        self.layout.is_armed()
    }

    /// Write the layout now if it has changed, debounce or not — the quit path.
    pub fn flush_layout(&mut self, path: &std::path::Path) -> Option<Result<(), String>> {
        self.layout.flush(path)
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
        let view = self.ws().active();
        let mode = self.mode;

        // The document's file watcher: poll on its own cadence, keep frames
        // coming while anything is watched (a poll nobody runs watches
        // nothing), and repaint immediately on news so the notice lands next
        // frame rather than at the next keystroke.
        if self.charts.doc.watch.has_watches() {
            if self.charts.doc.watch.poll() {
                ctx.request_repaint();
            }
            ctx.request_repaint_after(crate::watch::WATCH_POLL);
        }

        // The overlay-opening keys, before the grammar feed so the frame that
        // opens an overlay is already under it.
        self.overlay_open_keys(&ctx, view);
        // Return-home, on the same gate: no overlay may own the keyboard, and
        // it is deliberately not an overlay-opener (so the registry cross-ref
        // that pins those three stays pinned).
        self.home_key(&ctx);
        // The frame verbs, on the same gate and only over the chart view: they
        // are bare keys, so an overlay or a text field must own the keyboard
        // first.
        self.navigation_keys(&ctx, view);

        // Whether this frame is the front door. Decided once, **after**
        // `home_key`, because three branches below have to agree which frame
        // this is — the grammar feed, the hint bar and the central panel — and
        // a cmd-shift-h this frame just emptied the documents: latched before
        // it, they would draw the dock over a door-that-should-be for one
        // frame. The door replaces the *dock* and nothing else — the top bar,
        // the overlays and the notification layers stay exactly where they are,
        // so the door is a state of the window's content plane rather than a
        // mode of the window.
        let door = self.front_door_is_live();

        // The protocol grammar is bare-key — `h j k l y t Enter Esc ⌫ shift-S`
        // with no modifier to disambiguate it — so it is fed only while its own
        // view is drawn. Gating on the active view rather than on the focused
        // pane's `Subject::key_context` is deliberate: the grammar drives the
        // *view's* model, not one pane's, and every pane of this view declares
        // the same context anyway. A per-pane gate would be a second answer to
        // a question the view already answers.
        //
        // And it is fed only while no overlay is open — the
        // no-bare-under-overlay invariant. An open picker owns the keyboard;
        // a `j` typed into its query line must never also walk the DAG
        // underneath it.
        //
        // Nor while the front door is drawn: the grammar drives the DAG, and
        // the door is standing where the DAG would be.
        if view == ViewKind::Protocol && !door {
            if self.overlay.is_none() {
                let events = ctx.input(|i| i.events.clone());
                self.protocol.doc.model.feed_events(&events);
            }
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
        self.home_button = bar.home_rect;

        // The key-hint bar belongs to the protocol grammar, so it is drawn on
        // the view that has one. The charts view has no key grammar at all, and
        // `chart_window_size` has no term for a hint bar —
        // `the_window_it_asks_for_fits_the_raster_it_presents` is the assertion
        // that keeps it honest about that: it lays a real frame out at the size
        // that function asks for and reads the box the chart pane was handed, so
        // a hint bar appearing on this view would take BAR_HEIGHT out of that
        // box and redden.
        // And not on the front door: a row of grammar hints for a DAG that is
        // not on screen would be the window instructing rather than inviting.
        if view == ViewKind::Protocol && !door {
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
        let tabbed = self.ws().tabbed_tiles(view);
        let focused = self.ws().focus();
        let mut requests: Vec<Request> = Vec::new();
        let dock_frame = egui::Frame::new()
            .inner_margin(DOCK_INSET)
            .fill(ui.visuals().panel_fill);
        if door {
            // The front door, instead of the dock: a dock of empty
            // instruments is the surface the research warned against, and
            // every one of its panes would be inviting the same first action
            // from a different corner. What a card click *does* is the same
            // `Request::Open` an empty pane's button raises — the door is a
            // different arrangement of the same way in, not a second route.
            CentralPanel::default().frame(dock_frame).show(ui, |ui| {
                self.front_door_ui(ui, &mut requests);
            });
        } else {
            // Content somewhere, so the dock — and no stale door geometry: a
            // test that asks where a card was after the door has gone must
            // hear "nowhere", exactly as `affordances` answers for panes.
            self.door_cards.clear();
            self.door_continue = None;
            self.door_help = None;
            let (ws, charts, protocol, affordances) = (
                self.layout.workspace_mut(),
                &mut self.charts,
                &mut self.protocol,
                &mut self.affordances,
            );
            CentralPanel::default().frame(dock_frame).show(ui, |ui| {
                // The two arms are the whole cost of keeping the documents
                // apart.
                match view {
                    ViewKind::Charts => {
                        let mut behavior = PaneChrome::new(
                            &mut charts.doc,
                            &mut charts.items,
                            mode,
                            focused,
                            &tabbed,
                            &mut requests,
                            affordances,
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
                            affordances,
                        );
                        ws.tree_mut(view).ui(&mut behavior, ui);
                    }
                }
            });
        }

        self.status_rail_ui(&ctx, &mut requests);

        self.apply(&ctx, view, requests);
        if view == ViewKind::Protocol {
            if !door {
                self.read_active_tab();
            }
            if bar.toggle_flow {
                self.protocol.doc.model.toggle_flow();
                self.protocol.doc.canvas.invalidate();
            }
        }
        if bar.home {
            self.open_home(&ctx);
        }
        if let Some(next) = bar.switch {
            self.ws_mut().set_active(next);
            ctx.request_repaint();
        }

        // After the panes have drawn, because the controls rail dispatches a
        // slider's queued value inside its own draw — so this frame's gesture
        // is answered in this frame's banner, not the next one's.
        self.say_interaction_fault();

        // The overlay plane, over everything the frame drew, then the two
        // notification layers over that. All three draw nothing when empty,
        // so a frame with no overlay, no banner and no toast is
        // pixel-identical to one drawn before they existed.
        self.overlay_ui(&ctx, view);
        self.notifications.show(&ctx);
        self.toasts.show(&ctx);

        self.sweep();
    }

    // -----------------------------------------------------------------------
    // The overlay slot
    // -----------------------------------------------------------------------

    /// Open an overlay if its registry-declared key was pressed this frame.
    ///
    /// The palette and the node jump open on the **protocol** view only: the
    /// palette's candidate list is altitude-scoped and every verb it offers
    /// at [`Altitude::Protocol`] genuinely dispatches through the model,
    /// while at the chart altitudes most verbs have no handler in this shell
    /// yet — a palette of rows that silently no-op would be worse than none.
    /// Both go live on the chart view with its editing bridge. The help
    /// sheet is read-only and opens anywhere.
    fn overlay_open_keys(&mut self, ctx: &egui::Context, view: ViewKind) {
        if self.overlay.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed = |token: Option<&'static str>| token.is_some_and(|t| consume_token(ctx, t));
        if view == ViewKind::Protocol && pressed(self.overlay_keys.palette) {
            self.open_palette(Altitude::Protocol);
        } else if view == ViewKind::Protocol && pressed(self.overlay_keys.jump) {
            self.open_jump();
        } else if pressed(self.overlay_keys.help) {
            self.overlay = Some(Overlay::Help(Picker::new(HelpSheet::new())));
        }
    }

    /// Return home if the registry's `open-home` keystroke was pressed this
    /// frame and no overlay owns the keyboard.
    ///
    /// A sibling of [`overlay_open_keys`](Self::overlay_open_keys) rather than
    /// a case inside it: `open-home` is a runtime dispatch, not an overlay
    /// opener, and [`OverlayKeys`] is pinned to exactly the three that are. It
    /// still invents no binding — the token comes off the registry
    /// ([`Self::home_binding`]) and runs through the same
    /// [`consume_token`]. `open_home` is a no-op on the front door, so an
    /// idle press there costs nothing.
    fn home_key(&mut self, ctx: &egui::Context) {
        if self.overlay.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        if self.home_binding.is_some_and(|t| consume_token(ctx, t)) {
            self.open_home(ctx);
        }
    }

    /// Perform whichever navigation verb's key is down this frame.
    ///
    /// Gated exactly as [`Self::home_key`] is — no overlay open, no widget
    /// holding the keyboard — plus the chart view being the one on screen,
    /// because a frame verb over the protocol graph has no frame to move and
    /// its bare keys would shadow that view's own grammar.
    fn navigation_keys(&mut self, ctx: &egui::Context, view: ViewKind) {
        if view != ViewKind::Charts || self.overlay.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        for (token, longname) in self.nav_bindings.clone() {
            if consume_token(ctx, token) && navigation_verb(&mut self.charts.doc, longname) {
                ctx.request_repaint();
            }
        }
    }

    /// Open the command palette at `altitude`, over a snapshot of the
    /// session's recency.
    fn open_palette(&mut self, altitude: Altitude) {
        self.overlay = Some(Overlay::Palette(Picker::new(CommandPalette::new(
            altitude,
            self.recency.clone(),
        ))));
    }

    /// Open the node jump over the outline — the graph in view, in its
    /// topological order, which is what an empty query shows.
    fn open_jump(&mut self) {
        let targets = self
            .protocol
            .doc
            .model
            .outline()
            .into_iter()
            .map(|row| JumpTarget {
                detail: (row.id != row.label).then(|| row.id.clone()),
                id: row.id,
                label: row.label,
            })
            .collect();
        self.overlay = Some(Overlay::Jump(Picker::new(JumpToNode::new(targets))));
    }

    /// Draw the open overlay, if any, and act on what it reports.
    ///
    /// Dismissal arrives two ways and both mean close: a [`Picker`] inside a
    /// [`ModalLayer`] consumes escape first and reports it as its own
    /// [`PickerEvent::Dismissed`], while a backdrop click surfaces as the
    /// layer's `dismissed` flag.
    fn overlay_ui(&mut self, ctx: &egui::Context, view: ViewKind) {
        let Some(mut overlay) = self.overlay.take() else {
            return;
        };
        let close;
        match &mut overlay {
            Overlay::Palette(picker) => {
                let chrome = ModalChrome::new().title("Commands").enter_hint("run");
                let shown =
                    ModalLayer::show(ctx, "bf-overlay-palette", &chrome, |ui| picker.show(ui));
                match shown.inner.event {
                    Some(PickerEvent::Confirmed) => {
                        if let Some(name) = picker.delegate.take_picked() {
                            self.recency.record(name);
                            self.apply(ctx, view, vec![Request::Verb(Verb::new(name))]);
                        }
                        close = true;
                    }
                    Some(PickerEvent::Dismissed) => close = true,
                    None => close = shown.dismissed,
                }
            }
            Overlay::Help(picker) => {
                let chrome = ModalChrome::new().title("Keyboard help");
                let shown = ModalLayer::show(ctx, "bf-overlay-help", &chrome, |ui| picker.show(ui));
                // Read-only: enter is another way out (the delegate is not
                // confirmable), so every event is a close.
                close = shown.inner.event.is_some() || shown.dismissed;
            }
            Overlay::Jump(picker) => {
                let chrome = ModalChrome::new().title("Jump to asset").enter_hint("jump");
                let shown = ModalLayer::show(ctx, "bf-overlay-jump", &chrome, |ui| picker.show(ui));
                match shown.inner.event {
                    Some(PickerEvent::Confirmed) => {
                        if let Some(id) = picker.delegate.take_picked() {
                            self.protocol.doc.model.select_id(id);
                        }
                        close = true;
                    }
                    Some(PickerEvent::Dismissed) => close = true,
                    None => close = shown.dismissed,
                }
            }
        }
        if close {
            ctx.request_repaint();
        } else {
            self.overlay = Some(overlay);
        }
    }

    /// Which overlay is open, named — a test hook, like
    /// [`MeridianApp::switcher_rect`].
    #[must_use]
    pub fn open_overlay(&self) -> Option<&'static str> {
        self.overlay.as_ref().map(|o| match o {
            Overlay::Palette(_) => "palette",
            Overlay::Help(_) => "help",
            Overlay::Jump(_) => "jump",
        })
    }

    /// The persistent banner layer, read-only — what a test holds the
    /// replace-not-stack contract against.
    #[must_use]
    pub fn notifications(&self) -> &NotificationLayer {
        &self.notifications
    }

    /// Dismiss the chart-fault banner, as clicking its × does.
    ///
    /// Exists so the dismissal can be driven from outside a real pointer: the ×
    /// is handled inside the notification layer's own draw, and a banner that
    /// comes straight back is indistinguishable from one that was never
    /// dismissed unless a test can perform the gesture. Returns whether a
    /// banner was showing to dismiss.
    pub fn dismiss_chart_fault(&mut self) -> bool {
        self.notifications
            .dismiss(NotificationId::new("chart-interaction-fault"))
    }

    /// The transient toast layer, read-only.
    #[must_use]
    pub fn toasts(&self) -> &ToastLayer {
        &self.toasts
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
        let active = self.ws().active();
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
        // The Home button belongs in the always-drawn left group, not the
        // right-to-left group that is dropped on a narrow window — a return
        // to the front door that vanishes when the window is small is a return
        // you cannot rely on. It shows only off the door: on the door there is
        // nothing to return from, and `open_home` would no-op anyway.
        let door = self.front_door_is_live();

        let mut bar = TopBar::default();
        ui.horizontal_centered(|ui| {
            for view in ViewKind::ALL {
                let control = ui.selectable_label(view == active, view.label());
                bar.switcher.push((view, control.rect));
                if control.clicked() && view != active {
                    bar.switch = Some(view);
                }
            }
            if !door {
                let home = ui.button(egui::RichText::new("Home").font(ui_font()));
                bar.home_rect = Some(home.rect);
                if home.clicked() {
                    bar.home = true;
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
            // The renderer line is a developer diagnostic — a stranger's first
            // launch should not read "egui · Vello · wgpu 29". It appears only
            // under the devtools flag; the flow toggle is a real affordance and
            // always draws when the protocol view supplies one.
            let renderer =
                crate::devtools::enabled().then(|| format!("egui · Vello · wgpu 29  —  {theme}"));
            let toggle = (active == ViewKind::Protocol).then(|| match flow {
                Flow::Vertical => ("flow: vertical ⇄".to_string(), "horizontal"),
                Flow::Horizontal => ("flow: horizontal ⇄".to_string(), "vertical"),
            });
            let wanted = right_group_width(
                ui,
                renderer.as_deref(),
                toggle.as_ref().map(|(t, _)| t.as_str()),
            );
            if (renderer.is_some() || toggle.is_some()) && ui.available_width() >= wanted {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(renderer) = renderer {
                        ui.label(
                            egui::RichText::new(renderer)
                                .monospace()
                                .color(chrome::colour(sem.text.muted)),
                        );
                    }
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

    /// The status rail: the workbench widget, drawn at last — the contract
    /// tests spent several releases noting it was "drawn by nothing", and
    /// this is the something.
    ///
    /// The float itself is [`chrome::status_rail_overlay`]'s — the bottom
    /// edge, a foreground layer, nothing when there is nothing to say (and
    /// the note there on why floating rather than a bottom panel). What this
    /// method owns is the *content*:
    ///
    /// - the **focused pane's** status lines, as declared on its
    ///   [`Subject`] — minus its activity reports;
    /// - **one** activity indicator, composed from *every* pane's subject in
    ///   *both* views — in-flight work anywhere in the window is the window's
    ///   to report, and two panes querying at once say "querying…" once.
    ///
    /// Dismissal verbs the rail's entries declare are routed into the same
    /// request queue as pane requests — the rail can offer nothing a pane
    /// could not.
    fn status_rail_ui(&mut self, ctx: &egui::Context, requests: &mut Vec<Request>) {
        let mode = self.mode;
        let chart_subjects: Vec<Subject> = self
            .charts
            .items
            .values()
            .map(|item| item.subject(&self.charts.doc))
            .collect();
        let protocol_subjects: Vec<Subject> = self
            .protocol
            .items
            .values()
            .map(|item| item.subject(&self.protocol.doc))
            .collect();

        let mut entries: Vec<StatusEntry> = Vec::new();
        if let Some(key) = self.ws().focus() {
            let focused = match self.ws().active() {
                ViewKind::Charts => self
                    .charts
                    .items
                    .get(&key)
                    .map(|item| item.subject(&self.charts.doc)),
                ViewKind::Protocol => self
                    .protocol
                    .items
                    .get(&key)
                    .map(|item| item.subject(&self.protocol.doc)),
            };
            if let Some(subject) = focused {
                // The pane's own lines. Its typed activity entries are
                // filtered out here because the indicator below says them —
                // once, merged with everyone else's, never twice.
                entries.extend(
                    subject
                        .status
                        .into_iter()
                        .filter(|entry| Activity::of_entry(entry).is_none()),
                );
            }
        }
        if let Some(indicator) =
            ActivityIndicator::compose(chart_subjects.iter().chain(protocol_subjects.iter()))
        {
            entries.push(indicator);
        }

        self.rail = chrome::status_rail_overlay(ctx, &entries, mode);
        for verb in self.rail.dismissed.clone() {
            requests.push(Request::Verb(verb));
        }
    }

    /// What the status rail drew last frame, read-only — the test hook.
    #[must_use]
    pub fn rail(&self) -> &chrome::StatusDrawn {
        &self.rail
    }

    /// Perform the requests the frame's panes raised, now that the tile tree's
    /// borrow is over.
    ///
    /// A verb goes to the model of the view that raised it — only the active
    /// view's panes drew, so that is the active view's model. The charts view
    /// has no model; its verbs act on the document directly, and the match is
    /// spelled out so that a chart control declaring a verb this arm does not
    /// handle is a change to *this* line rather than a button that silently
    /// does nothing.
    fn apply(&mut self, ctx: &egui::Context, view: ViewKind, requests: Vec<Request>) {
        for request in requests {
            match request {
                // open-home spans both views — it is not a per-view dispatch,
                // so it is handled before the view match rather than inside it.
                // The Charts arm string-matches only clear-selection, and the
                // Protocol arm forwards to `model.dispatch`, which would
                // silently no-op an unknown verb — so without this intercept
                // the verb would reach neither handler.
                Request::Verb(verb) if verb.as_str() == "open-home" => {
                    self.open_home(ctx);
                }
                Request::Verb(verb) => match view {
                    ViewKind::Charts => {
                        if verb.as_str() == "clear-selection" {
                            self.charts.doc.clear_selection();
                            ctx.request_repaint();
                        } else if navigation_verb(&mut self.charts.doc, verb.as_str()) {
                            ctx.request_repaint();
                        }
                    }
                    ViewKind::Protocol => {
                        self.protocol.doc.model.dispatch(verb.as_str());
                    }
                },
                Request::Open(id) => self.open_start(ctx, id),
                Request::Focus(key) => {
                    self.ws_mut().set_focus(key);
                }
                Request::Repaint => ctx.request_repaint(),
            }
        }
    }

    /// Return to the front door, **keeping the session** so the door's
    /// Continue zone still offers the way back in.
    ///
    /// [`open_start`](Self::open_start)'s mirror, with one deliberate
    /// asymmetry: this clears both documents but leaves `layout.opened`
    /// untouched. `open_start` records the id there so a later launch restores
    /// rather than greets; going Home wants the greeting *and* the standing
    /// offer to resume, and the Continue card is drawn from that id — so it has
    /// to survive the trip home. The product owner locked that: Home keeps your
    /// place.
    ///
    /// Both documents are emptied **in place** rather than rebuilt, which keeps
    /// the wgpu host this window rasters through. A fresh `ChartDoc::empty()` /
    /// `ProtocolDoc::empty()` would drop the device the next dashboard needs.
    /// A window already on the front door is left exactly as it is — the guard
    /// is what makes cmd-shift-h on the door a no-op rather than a needless
    /// re-clear that would flash the same empty pixels.
    ///
    /// The chart side goes through [`open_chart`](Self::open_chart), not
    /// through `ChartDoc::open`: going Home is a document swap like any other,
    /// and the empty document has nothing to say — so the outgoing spec's
    /// banners come down with it. Calling `ChartDoc::open` here instead left
    /// a `Cannot render …` banner raised on the front door, about a document
    /// that was no longer open.
    fn open_home(&mut self, ctx: &egui::Context) {
        if self.front_door_is_live() {
            return;
        }
        self.open_chart(Composed::empty());
        self.protocol.doc.open(ProtocolInputs::empty());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
        ctx.request_repaint();
    }

    /// Open the shipped starting point `id` into the view it fills.
    ///
    /// **The second click.** It has to land on a rendered result, not on an
    /// instrument: the document is replaced with a *composed* dashboard or a
    /// *built* graph, its canvas is invalidated so the next frame rasters the
    /// new one, and the view holding it is made active. There is no editor in
    /// between, no path to type and no buffer to fill.
    ///
    /// It also records the id in the live layout, which is what lets a later
    /// launch restore this rather than show the front door again — the whole
    /// reason [`SavedLayout::opened`] exists. Recording it also makes the
    /// layout dirty, so the debounce writes it.
    ///
    /// An id this build does not ship, or a fixture that will not load, is a
    /// build-time defect rather than a user's circumstance, and not worth
    /// taking a window down for — it logs for the headless tiers and raises
    /// a banner where a user is looking. The banner's id is composite over
    /// the start, so the same start failing again **replaces** its banner in
    /// place rather than stacking a second; a later success dismisses it.
    fn open_start(&mut self, ctx: &egui::Context, id: &'static str) {
        let banner = NotificationId::composite("open-start", id);
        let opened = match crate::starts::load(id) {
            Ok(opened) => opened,
            Err(e) => {
                eprintln!("could not open {id}: {e}");
                self.notifications.raise(
                    Notification::new(banner, Severity::Error, "Could not open the starting point")
                        .body(format!("{id}: {e}")),
                );
                ctx.request_repaint();
                return;
            }
        };
        let view = opened.view();
        match opened {
            // A new chart document is a new set of things to say, and a
            // reason to stop saying the last one's.
            crate::starts::Opened::Charts(composed) => self.open_chart(*composed),
            crate::starts::Opened::Protocol(inputs) => self.protocol.doc.open(*inputs),
        }
        self.notifications.dismiss(banner);
        self.ws_mut().set_active(view);
        self.layout.live_mut().opened = Some(id.to_string());
        self.toasts.push(Toast::new(
            Severity::Success,
            format!("Opened {}", self.title()),
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
        ctx.request_repaint();
    }

    // -----------------------------------------------------------------------
    // The front door
    // -----------------------------------------------------------------------

    /// Draw the front door into the central panel: what the window shows when
    /// nothing is open anywhere, in place of a dock of empty instruments.
    ///
    /// Five zones, one voice: **Welcome** (the invariant greeting — the
    /// content below it morphs, the greeting never flips), **Start** (the
    /// verb spine), **Continue** (recent work, present only once there is
    /// any), **Explore** (the gallery of shipped starts — the flagship), and
    /// **Learn** (the placeholder for walkthroughs, which are their own
    /// work). Continue appearing and disappearing *is* the morph: there is no
    /// dismissal anywhere because there is nothing to dismiss — content
    /// outcompetes the door, and the door comes back only when the window is
    /// emptied.
    ///
    /// Everything a click does here goes out through `requests` as the same
    /// [`Request::Open`] an empty pane's button raises, into the same
    /// [`MeridianApp::open_start`]. The door owns no route of its own.
    fn front_door_ui(&mut self, ui: &mut egui::Ui, requests: &mut Vec<Request>) {
        self.door_cards.clear();
        self.door_continue = None;
        self.door_help = None;
        self.affordances.clear();
        self.ensure_door_thumbs(ui.ctx());

        let sem = semantic(self.mode.is_dark());
        let help_key = self.overlay_keys.help;
        // The remembered start, when the layout carries one this build
        // recognises. The door only draws with nothing open, so anything
        // remembered here is by definition work that was *not* restored —
        // which is exactly what Continue is for.
        let remembered = self
            .layout
            .live()
            .opened
            .as_deref()
            .and_then(crate::starts::find);
        let mut open_help = false;

        egui::ScrollArea::vertical()
            .id_salt("bf-front-door")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = DOOR_COLUMN_WIDTH.min(ui.available_width());
                let pad = ((ui.available_width() - width) / 2.0).max(0.0);
                ui.horizontal(|ui| {
                    ui.add_space(pad);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        ui.add_space(spacing::SECTION_GAP);

                        // Welcome — invariant, whatever the zones below do.
                        ui.label(
                            egui::RichText::new("Welcome")
                                .text_style(egui::TextStyle::Heading)
                                .color(chrome::colour(sem.text.primary)),
                        );
                        ui.add_space(spacing::SPACE_2);
                        ui.label(
                            egui::RichText::new(TAGLINE)
                                .font(ui_font())
                                .color(chrome::colour(sem.text.muted)),
                        );

                        // Start — the verb spine. Only verbs that work from
                        // here: the help sheet opens on any view, and its
                        // keystroke is the registry's, printed rather than
                        // claimed.
                        door_zone_heading(ui, "Start", sem);
                        let help_label = match help_key {
                            Some(k) => format!("Keyboard help  {k}"),
                            None => "Keyboard help".to_string(),
                        };
                        let help = ui.button(egui::RichText::new(help_label).font(ui_font()));
                        self.door_help = Some(help.rect);
                        if help.clicked() {
                            open_help = true;
                        }

                        // Continue — the morph. Absent on a first run; a
                        // remembered start renders its own opening control,
                        // raising the request a pane's button would.
                        if let Some(start) = remembered {
                            door_zone_heading(ui, "Continue", sem);
                            let control =
                                ui.button(egui::RichText::new(start.label).font(ui_font()));
                            self.door_continue = Some(control.rect);
                            if control.clicked() {
                                requests.push(Request::Open(start.id));
                            }
                        }

                        // Explore — the gallery, and the flagship: every card
                        // opens onto a drawn result. The second sentence is
                        // the narrowing a fetched start forced: the SPECS all
                        // ship inside the binary, but one of them reads a
                        // table that does not, and the sentence that used to
                        // end at "rendered result" read as a promise it could
                        // not keep on a plane. Its card carries
                        // `starts::REMOTE_MARK`; this says what that means.
                        door_zone_heading(ui, "Explore", sem);
                        ui.label(
                            egui::RichText::new(
                                "Starting points that ship with the binary — \
                                 each opens on a rendered result. A card \
                                 marked over the network fetches its data \
                                 when you open it.",
                            )
                            .font(ui_font())
                            .color(chrome::colour(sem.text.secondary)),
                        );
                        ui.add_space(spacing::CONTROL_GAP);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing =
                                egui::vec2(spacing::SECTION_GAP, spacing::SECTION_GAP);
                            for start in crate::starts::STARTS {
                                self.door_card(ui, start, sem, requests);
                            }
                        });

                        // Learn — the placeholder zone; walkthrough content
                        // is its own work and arrives as such.
                        door_zone_heading(ui, "Learn", sem);
                        ui.label(
                            egui::RichText::new("Guided walkthroughs will live here.")
                                .font(ui_font())
                                .color(chrome::colour(sem.text.muted)),
                        );
                        ui.add_space(spacing::SECTION_GAP);
                    });
                });
            });

        if open_help {
            self.overlay = Some(Overlay::Help(Picker::new(HelpSheet::new())));
        }
    }

    /// One gallery card: the start's pre-rendered thumbnail, its label and
    /// its one-line summary, the whole card clickable.
    ///
    /// A card whose start fills a view is also recorded under that view's
    /// canvas pane key in [`Self::affordances`] — on a door frame it *is*
    /// where the way in that fills that pane was drawn, so
    /// [`MeridianApp::affordance_rect`] keeps one answer across both
    /// arrangements of the same affordance.
    fn door_card(
        &mut self,
        ui: &mut egui::Ui,
        start: &'static crate::starts::Start,
        sem: &semantic::Semantic,
        requests: &mut Vec<Request>,
    ) {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(CARD_WIDTH, CARD_HEIGHT), egui::Sense::click());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter().with_clip_rect(rect);
            painter.rect_filled(rect, radius::CONTROL, chrome::colour(sem.surfaces.raised));
            let edge = if response.hovered() {
                sem.borders.strong
            } else {
                sem.borders.subtle
            };
            painter.rect_stroke(
                rect,
                radius::CONTROL,
                egui::Stroke::new(1.0, chrome::colour(edge)),
                egui::StrokeKind::Inside,
            );

            let img_rect = egui::Rect::from_min_size(
                rect.min + egui::vec2(spacing::SPACE_2, spacing::SPACE_2),
                egui::vec2(CARD_WIDTH - 2.0 * spacing::SPACE_2, CARD_IMAGE_HEIGHT),
            );
            if let Some(tex) = self
                .door_thumbs
                .iter()
                .find(|(id, _)| *id == start.id)
                .map(|(_, t)| t.id())
            {
                painter.image(
                    tex,
                    img_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }

            let text_left = rect.min.x + spacing::SPACE_4;
            let wrap = CARD_WIDTH - 2.0 * spacing::SPACE_4;
            let title = painter.layout(
                start.label.to_string(),
                ui_font(),
                chrome::colour(sem.text.primary),
                wrap,
            );
            let title_pos = egui::pos2(text_left, img_rect.max.y + spacing::SPACE_4);
            painter.galley(title_pos, title.clone(), chrome::colour(sem.text.primary));
            let summary = painter.layout(
                start.summary.to_string(),
                egui::TextStyle::Small.resolve(ui.style()),
                chrome::colour(sem.text.muted),
                wrap,
            );
            painter.galley(
                egui::pos2(text_left, title_pos.y + title.size().y + spacing::SPACE_1),
                summary,
                chrome::colour(sem.text.muted),
            );
        }

        self.door_cards.push((start.id, rect));
        let fills_its_view = crate::starts::for_view(start.view).map(|s| s.id) == Some(start.id);
        if fills_its_view {
            let pane = match start.view {
                ViewKind::Charts => PaneKey::new(ViewKind::Charts, CHART),
                ViewKind::Protocol => PaneKey::new(ViewKind::Protocol, CANVAS),
            };
            self.affordances.push((pane, rect));
        }
        if response.clicked() {
            requests.push(Request::Open(start.id));
        }
    }

    /// Decode the shipped thumbnails into textures, once.
    ///
    /// Bytes that will not decode are a build defect the thumbnail
    /// regeneration test exists to catch — a card without its picture is the
    /// honest degradation here, not a reason to take the door down.
    fn ensure_door_thumbs(&mut self, ctx: &egui::Context) {
        if !self.door_thumbs.is_empty() {
            return;
        }
        for start in crate::starts::STARTS {
            let Ok(decoded) = image::load_from_memory(start.thumbnail) else {
                debug_assert!(false, "{}'s shipped thumbnail is not decodable", start.id);
                continue;
            };
            let rgba = decoded.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
            let tex = ctx.load_texture(
                format!("bf-door-thumb-{}", start.id),
                pixels,
                egui::TextureOptions::LINEAR,
            );
            self.door_thumbs.push((start.id, tex));
        }
    }

    /// Before rendering: make the active Canvas/Steps tab authoritative from the
    /// model's sheet flag (so the `shift-S`/`Esc` keys drive it).
    fn set_active_tab(&mut self) {
        let show_sheet = self.protocol.doc.model.show_sheet();
        let tree = self.ws_mut().tree_mut(ViewKind::Protocol);
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
        if let Some(show) = steps_tab_is_active(self.ws().tree(ViewKind::Protocol)) {
            self.protocol.doc.model.set_show_sheet(show);
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
        let charts: BTreeSet<PaneKey> = self.ws().panes(ViewKind::Charts).into_iter().collect();
        self.charts.doc.sweep(&charts);
        let protocol: BTreeSet<PaneKey> = self.ws().panes(ViewKind::Protocol).into_iter().collect();
        self.protocol.doc.sweep(&protocol);
    }
}

/// Whether the protocol view's tab strip has the steps sheet in front, or
/// `None` when this tree has no such strip.
///
/// The one reader of the strip, so the restore path and the click-back path
/// cannot disagree about which tab means what:
/// [`MeridianApp::assemble`] seeds the model from it before the first frame,
/// and [`MeridianApp::read_active_tab`] reads a user's click out of it after
/// each one.
fn steps_tab_is_active(tree: &egui_tiles::Tree<PaneKey>) -> Option<bool> {
    let steps = tile_of(tree, PaneKey::new(ViewKind::Protocol, STEPS))?;
    let tabs_id = tabs_holding(tree, steps)?;
    let Some(Tile::Container(Container::Tabs(tabs))) = tree.tiles.get(tabs_id) else {
        return None;
    };
    Some(tabs.active? == steps)
}

/// `3 occurrences` / `1 occurrence`. A banner that says "1 occurrences" is a
/// banner the reader trusts a shade less than the one before it.
fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// One front-door zone heading: the settled name, set apart from the zone's
/// content by weight and tone rather than by size — the door has exactly one
/// large word on it, and it is Welcome.
fn door_zone_heading(ui: &mut egui::Ui, name: &str, sem: &semantic::Semantic) {
    ui.add_space(spacing::SECTION_GAP);
    ui.label(
        egui::RichText::new(name)
            .font(ui_font())
            .strong()
            .color(chrome::colour(sem.text.secondary)),
    );
    ui.add_space(spacing::SPACE_2);
}

/// How wide the top bar's right-hand group would be if it drew: the renderer
/// line when developer diagnostics are on, and the flow toggle when the
/// protocol view supplies one. Either or both may be absent — an absent item
/// counts no width, and a group with nothing in it is not drawn at all.
///
/// Asked *before* the group is drawn, which is the whole point — a
/// right-to-left layout that does not fit overlaps what is already on the bar
/// instead of shrinking, and the thing already on the bar is the only way to
/// reach the other view. See [`MeridianApp::top_bar`].
///
/// The one leading item spacing is included because egui inserts it when it
/// places the group, and `available_width` is measured before it exists.
fn right_group_width(ui: &egui::Ui, renderer: Option<&str>, toggle: Option<&str>) -> f32 {
    let spacing = ui.spacing().item_spacing.x;
    let mut wanted = 0.0;
    if let Some(renderer) = renderer {
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        wanted += spacing + text_width(ui, renderer, mono);
    }
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

// ---------------------------------------------------------------------------
// Unit tests — the notification wiring, which only this module can drive
// through its private entry points.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> MeridianApp {
        MeridianApp::headless(Boot::empty(), Mode::Light)
    }

    /// The id-dedup contract at this shell's real raise site: the same start
    /// failing again replaces its banner; a different start is a different
    /// condition with its own.
    #[test]
    fn a_start_that_refails_replaces_its_banner_instead_of_stacking() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.open_start(&ctx, "no-such-start");
        app.open_start(&ctx, "no-such-start");
        assert_eq!(app.notifications().len(), 1, "one source, one banner");
        app.open_start(&ctx, "also-no-such-start");
        assert_eq!(
            app.notifications().len(),
            2,
            "a different start is a different condition"
        );
    }

    /// A success is a toast (a moment), never a banner (a condition) — and it
    /// clears only its own start's banner.
    #[test]
    fn a_successful_open_toasts_and_clears_only_its_own_banner() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.open_start(&ctx, "no-such-start");
        app.open_start(&ctx, crate::starts::DASHBOARD);
        assert!(!app.chart_doc().is_empty(), "the open did not land");
        assert_eq!(app.toasts().len(), 1, "a confirmation is a toast");
        assert_eq!(
            app.notifications().len(),
            1,
            "the unrelated failure banner is not swept up by someone else's success"
        );
    }

    /// The overlay openers are read off the registry, not invented here — if
    /// a binding moves in `brightfield-keys`, the shell follows it.
    #[test]
    fn the_overlay_keys_come_from_the_registry() {
        let keys = OverlayKeys::from_registry();
        let reg = brightfield_keys::registry();
        let primary = |name: &str| {
            reg.iter()
                .find(|v| v.longname == name)
                .and_then(brightfield_keys::VerbEntry::primary_key)
        };
        assert_eq!(keys.palette, primary("open-palette"));
        assert_eq!(keys.help, primary("open-help"));
        assert_eq!(keys.jump, primary("focus-jump"));
        // …and every token an opener binds today is one `consume_token` can
        // map to a key, so none of the three is silently unwired.
        for token in [keys.palette, keys.help, keys.jump].into_iter().flatten() {
            assert!(
                matches!(token, "space" | "/" | "?"),
                "registry token {token:?} has no key mapping in consume_token — \
                 the opener would be dead"
            );
        }
    }
}
