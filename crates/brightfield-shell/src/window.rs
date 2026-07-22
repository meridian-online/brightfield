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
use brightfield_workbench::behavior::{TAB_BAR_HEIGHT, TILE_GAP};
use brightfield_workbench::workspace::{tabs_holding, tile_of};
use brightfield_workbench::{
    chrome, DirtyTracker, ItemMap, PaneChrome, PaneKey, Request, SavedLayout, Verb, ViewKind,
    WindowGeometry, Workspace,
};
use meridian_egui::{
    ModalChrome, ModalLayer, Notification, NotificationId, NotificationLayer, Picker, PickerEvent,
    Severity, Toast, ToastLayer,
};

use meridian_design::{semantic, spacing};

use crate::app::{chart_registry, ChartDoc, CONTROLS_SHARE};
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
/// about fitting is this crate's defect, and a window larger than the display
/// is the compositor's to resolve.
///
/// Read by the same tiers as [`chart_window_size`], and kept for the same
/// reason — see the note there.
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
        let (live, composed) = crate::pipeline::live_spec(spec)?;
        let mut boot = Self::charts(composed);
        boot.live = Some(live);
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
    #[must_use]
    pub fn window_size(&self, view: ViewKind) -> (f32, f32) {
        match view {
            ViewKind::Charts => chart_window_size(&self.composed),
            ViewKind::Protocol => {
                protocol_window_size(&ProtocolModel::boot_layout(&self.protocol, self.flow))
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
    /// comes from, and nothing sends a `ViewportCommand::Title` afterwards
    /// except the front door's own click. A title that is wrong at this call
    /// stays wrong for the session.
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
    /// Each switcher control's rect — see [`MeridianApp::switcher`].
    switcher: Vec<(ViewKind, egui::Rect)>,
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
/// need a focused plot and an applied `SpecEdit`, which is the chart view's
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
/// this frame. Only the tokens the overlay openers actually bind are mapped;
/// an unmapped token is simply never consumed, which fails safe — the
/// overlay does not open, and nothing else changes.
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
        _ => false,
    }
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
    /// Where each empty pane drew the button that resolves it, in window-space
    /// logical points — recorded for exactly the reason [`Self::switcher`] is,
    /// and read back through [`MeridianApp::affordance_rect`].
    affordances: Vec<(PaneKey, egui::Rect)>,
    /// The one modal slot — see [`Overlay`].
    overlay: Option<Overlay>,
    /// The keystrokes that open overlays, read off the registry at boot.
    overlay_keys: OverlayKeys,
    /// The per-session palette recency: verbs run from the palette rank
    /// higher on its next empty-query open. Session-scoped by design (the
    /// sanctioned v1 simplification); it resets each launch.
    recency: RecencyCounter,
    /// Persistent, id-deduplicated banners. A source that re-fails raises
    /// under the same composite id and *replaces* its banner — never stacks.
    notifications: NotificationLayer,
    /// Transient, self-expiring toasts — confirmations, not conditions.
    toasts: ToastLayer,
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

        Self {
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
            affordances: Vec::new(),
            overlay: None,
            overlay_keys: OverlayKeys::from_registry(),
            recency: RecencyCounter::new(),
            notifications: NotificationLayer::new(),
            toasts: ToastLayer::new(),
        }
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
    /// `eframe::run_native`, and the only `ViewportCommand::Title` in the
    /// workspace is in `open_start`, which a restored session never reaches. `a_restored_session_is_titled_for_the_view_it_draws` asserts
    /// the agreement rather than either answer, because a literal on both
    /// sides would go on matching itself after either drifted.
    #[must_use]
    pub fn title(&self) -> String {
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

        // The overlay-opening keys, before the grammar feed so the frame that
        // opens an overlay is already under it.
        self.overlay_open_keys(&ctx, view);

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
        if view == ViewKind::Protocol {
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
        let tabbed = self.ws().tabbed_tiles(view);
        let focused = self.ws().focus();
        let mut requests: Vec<Request> = Vec::new();
        let dock_frame = egui::Frame::new()
            .inner_margin(DOCK_INSET)
            .fill(ui.visuals().panel_fill);
        let (ws, charts, protocol, affordances) = (
            self.layout.workspace_mut(),
            &mut self.charts,
            &mut self.protocol,
            &mut self.affordances,
        );
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

        self.apply(&ctx, view, requests);
        if view == ViewKind::Protocol {
            self.read_active_tab();
            if bar.toggle_flow {
                self.protocol.doc.model.toggle_flow();
                self.protocol.doc.canvas.invalidate();
            }
        }
        if let Some(next) = bar.switch {
            self.ws_mut().set_active(next);
            ctx.request_repaint();
        }

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
    /// has no model; its verbs act on the document directly, and the match is
    /// spelled out so that a chart control declaring a verb this arm does not
    /// handle is a change to *this* line rather than a button that silently
    /// does nothing.
    fn apply(&mut self, ctx: &egui::Context, view: ViewKind, requests: Vec<Request>) {
        for request in requests {
            match request {
                Request::Verb(verb) => match view {
                    ViewKind::Charts => {
                        if verb.as_str() == "clear-selection" {
                            self.charts.doc.clear_selection();
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
            crate::starts::Opened::Charts(composed) => self.charts.doc.open(*composed),
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
