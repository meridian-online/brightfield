//! One window, one arrangement, two documents behind it.
//!
//! [`MeridianApp`] is the whole of the product's UI: a [`Workspace`] holding
//! the window's one dock tree, the chart document and its items, the protocol
//! document and its items, and the one set of regions drawn over the lot.
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
//! therefore holds the two `(document, items)` pairs as sibling fields, and a
//! region hands its pane to whichever of the two owns it.
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
//! it is the document that pane reads.
//!
//! **Two documents is not two views.** The window is one Protocol: the spine
//! in the navigator rail, the step list in the ledger rail and a chart on the
//! canvas are regions of one arrangement, drawn in the same frame, and no
//! control moves between them. What the canvas holds is derived from the
//! documents rather than chosen — [`graph_takes_the_canvas`].
//!
//! # Both documents are always loaded
//!
//! The window holds both whether or not the spec on the command line described
//! them, so the one the spec did not describe boots *empty* —
//! `Composed::empty()` or `ProtocolInputs::empty()` — which every pane already
//! answers with a real empty state, gated by
//! [`brightfield_workbench::audit`] in the contract tests. It is a live
//! document with a canvas host behind it, not a headless one, so it can raster
//! the moment it gains content.

use std::collections::BTreeSet;

use egui::containers::{CentralPanel, Panel};
use egui_tiles::{Behavior, Container, Tile};

use brightfield_keys::{Altitude, RecencyCounter};
use brightfield_protocol::layout::{Flow, Layout};
use brightfield_sql::ir::SampleRate;
use brightfield_workbench::arrangement::{self, Occupant, Projection, Region, RegionId};
use brightfield_workbench::workspace::{tabs_holding, tile_of};
use brightfield_workbench::{
    chrome, Activity, ActivityIndicator, DirtyTracker, HideAffordance, ItemId, ItemMap, PaneChrome,
    PaneKey, Recent, Request, RunState, SavedLayout, StatusEntry, StatusSide, Subject, Tone,
    ToolbarEntry, Verb, WindowGeometry, Workspace,
};
use meridian_egui::{
    ModalChrome, ModalLayer, Notification, NotificationId, NotificationLayer, Picker, PickerEvent,
    Severity, Toast, ToastLayer,
};

use meridian_design::{radius, semantic, spacing};

use crate::app::{chart_registry, ChartDoc, ChartFault, CHART, CONTROLS};
use crate::canvas::EguiCanvasHost;
use crate::data_grid::DATA;
use crate::design::{self, Mode};
use crate::editor::EDITOR;
use crate::inspector::{InspectorPane, Selection};
use crate::overlays::{CommandPalette, HelpSheet, JumpTarget, JumpToNode};
use crate::pipeline::Composed;
use crate::protocol::{
    hint_ui, load_protocol_offline, mono_font, protocol_registry, ui_font, ProtocolDoc,
    ProtocolInputs, ProtocolModel, CANVAS as PROTOCOL_CANVAS, INSPECTOR as PROTOCOL_INSPECTOR,
    OUTLINE, STEPS,
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

/// The inspector rail's default width, outer — including its own frame.
///
/// One spelling of the number that lives in
/// [`brightfield_workbench::arrangement`], re-exported here because callers
/// outside this crate were reading it from this path before the arrangement
/// existed. The draw path does **not** read this: it reads the region.
pub const INSPECTOR_RAIL_WIDTH: f32 = arrangement::INSPECTOR_RAIL_WIDTH;

/// The inspector rail's minimum width, outer — the point past which
/// `Panel::resizable` refuses to narrow it further. As
/// [`INSPECTOR_RAIL_WIDTH`], one spelling of the arrangement's number.
pub const INSPECTOR_RAIL_MIN_WIDTH: f32 = arrangement::INSPECTOR_RAIL_MIN_WIDTH;

// ---------------------------------------------------------------------------
// The front door's own measures.
// ---------------------------------------------------------------------------

/// The line under the Welcome heading.
///
/// The product's own voice — chosen copy, no longer the neutral placeholder
/// this slot shipped with. Changing these words is a copy decision, not a
/// refactor.
pub const TAGLINE: &str = "Watch insight assemble.";

/// What the Start zone promises an opened data file becomes.
///
/// Every clause is a claim about behaviour, not a description of a screen: the
/// table is the Data pane reading the file's own rows, and *a chart for every
/// column it can draw one for* is [`crate::dashboard::Dashboard::of`]'s walk —
/// a tile per column, plus the columns it declines and why.
///
/// **The format names are copy, and nothing pins them to the list.** What this
/// build opens is [`crate::data_file::OPENABLE_EXTENSIONS`], and `--help` is
/// held to every spelling on it by
/// `the_help_names_every_extension_this_build_opens` — this sentence is not.
/// Whether the door should name each spelling or keep the format names a
/// reader recognises is the copy decision below, not a defect to close here.
///
/// It read *a first look drawn from it* until the generator shipped. That was
/// true of the single chart this route used to draw and understates a
/// dashboard of one tile per column, on the one screen that describes the
/// feature to a stranger. `the_door_promises_what_the_generator_draws` in
/// `tests/scripted_open.rs` runs the generator over a real table and holds
/// this sentence to what came back, so a build whose generator stopped
/// drawing a tile per column reddens rather than going on promising one.
///
/// Changing these words is a copy decision, not a refactor — as [`TAGLINE`].
pub const OPEN_FILE_PROMISE: &str = "A CSV or a Parquet on this machine. It opens as a table you \
                                     can read and a chart for every column it can draw one for — \
                                     nothing is uploaded and nothing is fetched.";

/// The front door's content column, in logical points: **every** shipped
/// start's card and the gaps between them, so the Datasets row sets the
/// measure and everything above it aligns to the gallery.
///
/// Derived from [`crate::starts::STARTS`] rather than written as a number,
/// because the number was four and the set is five: at a hard-coded
/// four-card column the fifth card wrapped to a second row, the Protocols
/// section below it fell past the bottom of the default window, and the
/// section that is empty on every first launch was invisible on every first
/// launch. `a_first_run_populates_datasets_and_states_what_protocols_will_hold`
/// is what holds it on screen — it fails if the set grows past what one row of
/// the default window can hold, which is a decision about the door rather than
/// something for this constant to absorb silently.
const DOOR_COLUMN_WIDTH: f32 = {
    let cards = crate::starts::STARTS.len();
    cards as f32 * CARD_WIDTH + (cards - 1) as f32 * spacing::SECTION_GAP
};

/// The curated section's heading: the datasets that ship with this build.
///
/// One of the door's two section names. The pair replaces the door's earlier
/// *Explore* and *Continue*, and the argument those had — verbs invite,
/// container nouns file, so spend the inviting verb on real data — stands as
/// reasoning and loses on the facts: these sections hold the product's own
/// primitives, Datasets and Protocols, and a screen naming one thing while the
/// product names another is the harder thing to unpick later.
pub const DATASETS_SECTION: &str = "Datasets";

/// The analyst's own section's heading — the other of the two.
pub const PROTOCOLS_SECTION: &str = "Protocols";

/// What the Datasets heading says about itself, beside it.
pub const DATASETS_NOTE: &str = "curated, and they ship with this build";

/// What the Protocols heading says about itself on a **first run**, when the
/// section below it is empty and has to state what it will hold rather than
/// hold it.
pub const PROTOCOLS_NOTE_EMPTY: &str = "your own work, once there is some";

/// What the Protocols heading says once there is something under it.
pub const PROTOCOLS_NOTE: &str = "pick up where you left off";

/// The empty Protocols section's own words — the first-run half of the door,
/// and the case the whole front door exists for: a section that is empty on
/// every first launch has to say what will fill it, or it reads as a feature
/// that is broken rather than one you have not used.
pub const PROTOCOLS_EMPTY_TITLE: &str = "Nothing here yet.";

/// The line under [`PROTOCOLS_EMPTY_TITLE`]: what to do about it, and what
/// the thing it produces is called.
///
/// The first clause is what this build does — taking a Datasets card records
/// it and the row appears here — and the second is the model it is naming,
/// which [`the Protocol being the container of record`] settles. Both halves
/// matter: an empty state that only names the concept teaches nothing about
/// the next click.
///
/// [`the Protocol being the container of record`]: crate::starts
pub const PROTOCOLS_EMPTY_BODY: &str =
    "Opening a Dataset above puts it here — a Protocol is what an analysis is saved as.";

/// What a Datasets card promises its click lands on, at the card's foot: the
/// door's standing promise that a starter opens onto a rendered result rather
/// than onto an instrument to fill in.
///
/// It is on the cards and not on the Protocols rows, and the asymmetry is the
/// point: a card has to say what the thing becomes when you take it, and a row
/// already *is* the thing. The promise the two sections share is not a
/// sentence printed twice but the route — both raise the same
/// [`Request::Open`] into the same private `MeridianApp::open_start`, which is
/// what `either_route_to_the_same_subject_leaves_the_same_window` holds. That
/// test walks the shipped starts and skips the one
/// [`crate::starts::Start::remote`] declares — taking its card reads an
/// `https://` source, which a hermetic suite cannot do — so what carries the
/// claim is the starts this binary opens on its own, and the remote one is
/// outside it.
pub const DOOR_ENTRY_PROMISE: &str = "opens on a rendered result";

/// One Protocols row's height, in logical points — the preview row of the
/// density ladder, which is what a row carrying a name, a state and a time
/// needs to stay readable at the door's type size.
const PROTOCOL_ROW_HEIGHT: f32 = spacing::ROW_PREVIEW;

/// How much of a Protocols row the name gets before the run state starts.
///
/// A fixed measure rather than a proportion of the column, so the three
/// columns line up down the section whatever the names are; a name longer
/// than this is clipped by its own galley rather than pushing the state
/// sideways, which is the same trade [`CARD_HEIGHT`] makes for a card's
/// label.
const PROTOCOL_ROW_NAME_WIDTH: f32 = 260.0;

/// One row the front door's Protocols section drew, with the text it drew.
///
/// Read back through [`MeridianApp::front_door_rows`]. Each string here is
/// taken off the galley the row laid out at that position — the text that
/// galley was built from, so at the one site that builds a row it is not one
/// field while the paint at that position is another. The type does not
/// enforce that; the single call site is what makes it true. A row built from the right record and painted from the
/// wrong field differs here, which is the defect this type can see and the
/// record cannot.
/// `what_open_start_records_is_what_the_opened_document_says` reads the name
/// back this way.
///
/// What it deliberately does not carry is how much of a galley the row's clip
/// rect let through: that is a matter of pixels, and the door's committed
/// baselines are what hold it.
#[derive(Clone, Debug, PartialEq)]
pub struct DoorRow {
    /// The start id a click on this row reopens — the same id a Datasets card
    /// for the same subject raises, which is what makes the two routes one.
    pub id: String,
    /// The Protocol's name, as drawn.
    pub name: String,
    /// Its run state, as drawn — [`RunState::label`]'s words rather than a
    /// second vocabulary coined here.
    ///
    /// [`RunState::label`]: brightfield_workbench::RunState::label
    pub state: String,
    /// When it was last opened, as drawn: *2m ago*, *yesterday*, *4 days ago*.
    pub when: String,
    /// Where the row was laid out, in window-space logical points — where a
    /// test aims the click.
    pub rect: egui::Rect,
}

/// One gallery card's outer width.
const CARD_WIDTH: f32 = 216.0;

/// The thumbnail's drawn height: the card's width less its two insets, at the
/// shipped thumbnails' own 16:10 — `(216 − 8) / 1.6`. Stated as the number the
/// arithmetic produces so a drift in either term is a visible edit here.
const CARD_IMAGE_HEIGHT: f32 = 130.0;

/// One gallery card's outer height: the image and its insets, room for a
/// two-line label and a three-line summary, and one grid row at the foot for
/// [`DOOR_ENTRY_PROMISE`]. A label that wraps past that is clipped by the
/// card's own rect rather than pushing the gallery apart.
const CARD_HEIGHT: f32 = 220.0 + spacing::ROW_GRID;

/// The natural window size in logical points for a composed dashboard: exactly
/// big enough that the chart pane's content box fits the dashboard's raster,
/// with every term of the chrome budget derived from the thing that consumes it.
///
/// Reading outwards from the raster, in each axis:
///
/// - the pane's content box is its tile inset by
///   [`chrome::pane_content_inset`] on all four sides, and, vertically, below a
///   [`chrome::header_band_height`] header band;
/// - the chart's tile is the whole of the dock — the inspector no longer
///   shares the dock's width with it, it is a [`Panel::right`](egui::containers::Panel::right)
///   beside the dock, at its own declared [`INSPECTOR_RAIL_WIDTH`];
/// - the dock is the window inset by [`DOCK_INSET`], below the top bar, less
///   the rail's width.
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
    let inset = chrome::pane_content_inset();
    // Every band and rail the window lays out before the canvas gets what is
    // left, read out of the arrangement rather than restated here.
    let (across, down) = chrome_budget(false);

    // The legend band is a term, not a bite: a dashboard whose scales call
    // for a margin legend gets the band's width beside the raster, and one
    // that calls for none contributes zero — read from the component that
    // draws it, like every other term here.
    let pane_w = composed.width as f32 + crate::legend::band_width(composed) + 2.0 * inset;
    let w = (pane_w + across).ceil();

    let pane_h = composed.height as f32 + chart_toolbar_band(composed) + 2.0 * inset;
    let h = (pane_h + down).ceil();

    (w, h)
}

/// What the window's own regions take out of each axis before the canvas gets
/// what is left, in logical points: `(across, down)`.
///
/// Read out of [`brightfield_workbench::arrangement`], which is what makes the
/// window the shell *asks* for follow the window it *draws*. Before the
/// arrangement existed these were literals here and literals again in the draw
/// path, and the pair that got out of step clipped the bottom seventeen rows
/// of the chart. `the_window_it_asks_for_fits_the_raster_it_presents` lays a
/// real frame out at this size and reads the box the canvas pane was handed.
///
/// `hint` is whether this window draws the key-hint band — the surface with a
/// bare-key grammar does, and the one without it does not, so it is a term of
/// the caller rather than of the arrangement.
///
/// The canvas's head band is [`chrome::rail_selector_height`] because that is
/// the split the canvas is drawn with: one measure, read from the function
/// that performs the split rather than restated as a second number.
fn chrome_budget(hint: bool) -> (f32, f32) {
    let plan = arrangement::default_arrangement();
    let across = rail_default(plan.expect_region(arrangement::NAVIGATOR_RAIL))
        + rail_default(plan.expect_region(arrangement::INSPECTOR_RAIL));
    let mut down = band_extent(plan.expect_region(arrangement::TITLE_BAND))
        + band_extent(plan.expect_region(arrangement::LOCATOR_BAND))
        + rail_default(plan.expect_region(arrangement::LEDGER_RAIL))
        + chrome::rail_selector_height();
    if hint {
        down += band_extent(plan.expect_region(arrangement::HINT_BAND));
    }
    (across, down)
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
/// One difference from the chart's, and it is a property of this surface
/// rather than an adjustment: the graph has a bare-key grammar and the chart
/// projections do not, so this window gives up the hint band as well — which
/// is the `hint` term `chrome_budget` takes.
///
/// Every other term is shared, because both windows are laid out by the same
/// arrangement: the bands and rails come out of the axes first and the canvas
/// takes what is left.
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
    let inset = chrome::pane_content_inset();
    // `true`: this window draws the key-hint band, because the graph is the
    // surface with a bare-key grammar.
    let (across, down) = chrome_budget(true);

    let w = (dag_w + 2.0 * inset + across).ceil();
    let h = (dag_h + 2.0 * inset + down).ceil();

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

/// Whether `chosen` names a file this build opens as **data** rather than
/// reads as a document.
///
/// By extension alone, and deliberately: this decides which of two loaders
/// gets the path, and it has to decide before either has touched the file.
/// Everything that could make a plausible-looking path unopenable — a URL
/// scheme, glob syntax in the name, a control character, a file that is not
/// there — is [`crate::data_file::accept`]'s to refuse, with a sentence naming
/// what it refused. A classifier that pre-empted any of those would hand a
/// `.csv` to the spec parser and get *unknown spec format* back, which is the
/// answer this route exists to stop giving.
///
/// The set is [`crate::data_file::OPENABLE_EXTENSIONS`], lowercased the way
/// `accept` lowercases it, so the two cannot disagree about what is openable —
/// `the_classifier_and_the_opener_agree_on_what_is_data` in
/// `tests/scripted_open.rs` walks the list and holds them to it.
#[must_use]
pub fn names_a_data_file(chosen: &str) -> bool {
    std::path::Path::new(chosen)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            crate::data_file::OPENABLE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())
        })
}

/// Whether the canvas region holds the **asset graph** rather than a chart
/// projection, given what each document has in it.
///
/// The window's one derived answer to *which document the canvas belongs to*,
/// and the reason it is a free function taking two bools rather than a method:
/// two callers need it at different moments and must not be able to disagree.
/// [`MeridianApp::draw`] asks it once per frame from the live documents, and
/// [`Boot::window_size`] asks it before a window exists, from documents that
/// have been loaded and not yet handed over. When those two disagreed, the
/// window was sized for one surface and drew the other.
///
/// The asymmetry — chart wins a tie — is the arrangement's, not a preference.
/// The protocol has three regions of its own (the spine in the navigator rail,
/// the steps in the ledger rail, an operator in the inspector rail), so it is
/// readable without the canvas. The chart has the canvas and nothing else, so
/// a window that gives the canvas away has put the chart nowhere.
///
/// `(false, false)` — nothing loaded — answers `false`, which is the front
/// door's case and costs nothing either way: the door replaces the regions
/// entirely.
#[must_use]
pub const fn graph_takes_the_canvas(has_graph: bool, has_chart: bool) -> bool {
    has_graph && !has_chart
}

/// What a window opens with: both documents' contents.
///
/// One value rather than two entry points, because the window holds both
/// documents whichever spec was named. The one the spec did not describe is
/// empty, and that is a state the product has to render correctly anyway — it
/// is what a first run over a spec that declares nothing looks like.
///
/// **It carries no opinion about what to look at.** There is one arrangement
/// and no switcher, and which document the canvas holds is derived from these
/// contents by [`graph_takes_the_canvas`] — so a boot that loaded a graph is
/// looking at a graph because that is what it loaded, not because it said so.
pub struct Boot {
    /// The chart document's dashboard.
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
    /// How this document's picture was chosen, when one chart kind chose the
    /// whole of it — see [`crate::app::Authored`].
    ///
    /// Set by [`Boot::data_file`] and by nothing else: a chart kind chooses a
    /// picture only on the route that opens a table with no spec, and only
    /// where the walk produced a single tile. It rides on the boot rather than
    /// being re-derived by each constructor because the *other* route into an
    /// opened file — [`MeridianApp::open_data_file`], reached from the front
    /// door's picker — builds its document from a `Boot` too. One record, one
    /// place it is made, so the two entry points cannot hand the chart pane
    /// different documents for one file.
    pub authored: Option<crate::app::Authored>,
    /// The protocol document's graph and steps.
    pub protocol: ProtocolInputs,
    /// The protocol document's reading axis.
    pub flow: Flow,
    /// A dotted asset id to select before the first frame, for a scripted
    /// capture that needs a cursor to act on. The graph's, and nothing else's.
    pub focus: Option<String>,
}

impl Boot {
    /// Open on `composed`, with an empty protocol.
    #[must_use]
    pub fn charts(composed: Composed) -> Self {
        Self {
            composed,
            live: None,
            spec_path: None,
            authored: None,
            protocol: ProtocolInputs::empty(),
            flow: Flow::Vertical,
            focus: None,
        }
    }

    /// Open on `inputs`, with an empty dashboard.
    #[must_use]
    pub fn protocol(inputs: ProtocolInputs, flow: Flow, focus: Option<String>) -> Self {
        Self {
            composed: Composed::empty(),
            live: None,
            spec_path: None,
            authored: None,
            protocol: inputs,
            flow,
            focus,
        }
    }

    /// Open on nothing: both documents empty.
    ///
    /// **The no-argument launch.** What stood here was a hardcoded
    /// `examples/dashboard.yaml`, which from the repo root silently opened a
    /// dashboard nobody asked for and from anywhere else exited with a read
    /// error before a window existed. Neither is a first run of a product.
    ///
    /// A pane answers an empty document with a real empty state — that is the
    /// workbench contract, held by `every_chart_pane_passes_the_contract_audit`
    /// and `every_protocol_pane_passes_the_contract_audit` through
    /// [`audit`](brightfield_workbench::audit) rather than remembered — so this
    /// is not a blank window. It is the front door,
    /// and the panes that can be filled by something the binary ships offer it
    /// (see [`crate::starts`]).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            composed: Composed::empty(),
            live: None,
            spec_path: None,
            authored: None,
            protocol: ProtocolInputs::empty(),
            flow: Flow::Vertical,
            focus: None,
        }
    }

    /// Open on whatever [`crate::starts`] calls `id`.
    ///
    /// # Errors
    ///
    /// If `id` is not a start this build ships, or the embedded fixture fails
    /// to load.
    pub fn start(id: &str, flow: Flow) -> Result<Self, String> {
        Ok(match crate::starts::load(id)? {
            // The session comes with it, so the pane can re-composite into the
            // box the dock gives it. See [`crate::starts::OpenedChart`].
            crate::starts::Opened::Charts(chart) => Self {
                live: Some(chart.live),
                ..Self::charts(chart.composed)
            },
            crate::starts::Opened::Protocol(inputs) => Self::protocol(*inputs, flow, None),
        })
    }

    /// Whether the canvas of a window over these documents holds the asset
    /// graph — [`graph_takes_the_canvas`] over what this boot loaded.
    ///
    /// The size, the title and the summary line are all answered for the
    /// surface this names, and they are asked before a window exists. It used
    /// to be a field the caller had to resolve against the saved layout, and
    /// the resolution had a default in it: a restored crosswalk answered
    /// "Brightfield" and "composed 0x0 dashboard" for a 34-node graph, and
    /// since the window title is set once from that string the wrong one
    /// survived the whole session. Derived from the documents, there is
    /// nothing left to default.
    #[must_use]
    pub fn graph_on_canvas(&self) -> bool {
        graph_takes_the_canvas(!self.protocol.graph_full.nodes.is_empty(), self.has_chart())
    }

    /// Whether the chart document this boot loaded has a picture in it.
    ///
    /// The same question [`ChartDoc::is_empty`](crate::app::ChartDoc::is_empty)
    /// answers once the document exists, asked of the composition before it
    /// does.
    #[must_use]
    pub const fn has_chart(&self) -> bool {
        self.composed.width != 0 || self.composed.height != 0
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

    /// Open the data file at `chosen` as the dashboard generated for it.
    ///
    /// **The scripted twin of the front door's picker.** Both routes call
    /// [`crate::data_file::open`] and both hand the result to
    /// the private `Boot::of_opened_file`; the only difference is where the
    /// path came from — a command line, or an operating-system modal. That is
    /// what makes the generated dashboard something a script, a capture or a
    /// demo can reach, rather than something only a finger can.
    ///
    /// # Errors
    ///
    /// Whatever [`crate::data_file::open`] refuses: a URL rather than a local
    /// path, an extension this build does not read, a path the reader would
    /// resolve as a glob, a file DuckDB will not read, or a table no column of
    /// which admits a picture. Each carries the path and the reason.
    pub fn data_file(chosen: &str) -> Result<Self, String> {
        Ok(Self::of_opened_file(crate::data_file::open(chosen)?))
    }

    /// What an opened data file becomes, for **both** routes that open one.
    ///
    /// The document, the session behind it, the generated spec the editor pane
    /// opens, and the [`crate::app::Authored`] record — assembled once, here.
    /// [`MeridianApp::open_data_file`] takes the same `Boot` into a window that
    /// already exists; [`MeridianApp::with_layout`] takes it into a window
    /// being built. Two callers, one assembly, so the two entry points cannot
    /// drift into two documents for one file — which
    /// `the_two_routes_into_a_data_file_produce_one_document` in
    /// `tests/scripted_open.rs` holds by comparing what each leaves behind.
    fn of_opened_file(opened: crate::data_file::OpenedFile) -> Self {
        let crate::data_file::OpenedFile {
            live,
            composed,
            dashboard,
            spec_file,
        } = opened;
        // Only for a dashboard of ONE tile: that is the case where a tile's
        // picture is the document's picture, so the chart pane can host it
        // through that kind's module. A dashboard of several tiles is one
        // picture no single kind built — see [`crate::app::Authored`].
        let authored = dashboard.sole_tile().map(|tile| crate::app::Authored {
            kind: tile.kind(),
            fields: vec![tile.field().clone()],
            block: tile.block().to_string(),
        });
        Self {
            live: Some(live),
            spec_path: spec_file,
            authored,
            ..Self::charts(composed)
        }
    }

    /// Read `spec` and load whichever document it describes.
    ///
    /// **The one place a document named on the command line is classified.** It
    /// used to be two — the live binary and the shot binary each sniffed the
    /// file and each branched into its own shell — and the two branches then
    /// had to agree about an environment gate, a window size and a summary line
    /// that neither shared.
    ///
    /// Three kinds now, not two: a chart spec, a Protocol manifest, and a
    /// **data file**, which is decided first and by extension alone — see
    /// [`names_a_data_file`]. It has to come first because the other two are
    /// classified by reading the file as text, and a Parquet is not text.
    ///
    /// # Errors
    /// A message if the file cannot be read, if it is a run-less protocol
    /// manifest and this process has not opted in — see
    /// [`crate::protocol::run_less_manifest_refusal`], which states that rule
    /// once for both callers — or if the pipeline rejects it. A data file
    /// refuses through [`Boot::data_file`].
    pub fn open(spec: &str, flow: Flow, focus: Option<String>) -> Result<Self, String> {
        Self::open_sampled(spec, flow, focus, None)
    }

    /// [`Boot::open`] at an explicit pushed-down sample rate.
    ///
    /// `None` is [`Boot::open`] exactly — no clause, no extra query, the same
    /// bytes. `Some(rate)` opens the same document drawing one row in
    /// `rate.modulus()`, with the notice in the plot's own ink.
    ///
    /// A **data file** is refused a rate rather than quietly opened without
    /// one. The rate is pushed into a plot's own query, and a generated
    /// dashboard has no authored plot to push it into — so honouring the flag
    /// is not possible here and ignoring it would draw the complete table under
    /// a command line that asked for a sample of it.
    ///
    /// # Errors
    ///
    /// As [`Boot::open`], plus a data file named alongside a sample rate.
    pub fn open_sampled(
        spec: &str,
        flow: Flow,
        focus: Option<String>,
        sample: Option<SampleRate>,
    ) -> Result<Self, String> {
        if names_a_data_file(spec) {
            if sample.is_some() {
                return Err(format!(
                    "{spec}: a sample rate is pushed into a plot's own query, and the dashboard \
                     generated for a data file has no authored plot to push it into. Open it \
                     without the flag."
                ));
            }
            return Self::data_file(spec);
        }
        let text = std::fs::read_to_string(spec).map_err(|e| format!("read {spec}: {e}"))?;
        if brightfield_protocol::is_protocol_manifest(&text) {
            if !crate::protocol::offline_optin() {
                return Err(crate::protocol::run_less_manifest_refusal(spec));
            }
            let inputs = load_protocol_offline(spec)?;
            // The protocol half of the chart path's diagnostics loop below, and
            // it is here for the same reason: a headless capture draws no
            // banner, so a run with nobody watching has nothing but this. It
            // needs its own emission rather than sharing one because a degraded
            // protocol is the harder case — the chart diagnostic reports a mark
            // that is missing from a picture somebody is looking at, and this
            // reports a render that looks finished and is not.
            //
            // The PNG is still written and the exit code is still 0. The
            // degrade is deliberate (draw what you can); what was missing was
            // any way to know it happened.
            for line in inputs.degrade_report() {
                eprintln!("{spec}: {line}");
            }
            return Ok(Self::protocol(inputs, flow, focus));
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

    /// The window this boot asks for, in logical points — the natural size of
    /// whatever the canvas will hold.
    ///
    /// One window means one size, and the graph and a dashboard want very
    /// different ones. The canvas occupant decides it because the canvas is
    /// the region that cannot reflow away its content: the rails carry lists
    /// and scroll, and sizing to the larger of the two would open a window
    /// mostly full of an empty state nobody asked for. Nothing resizes
    /// afterwards — the user's window is theirs once it exists.
    ///
    /// Answered here rather than on [`MeridianApp`] because a window has to be
    /// sized before it can be created, and the app cannot be built until eframe
    /// has handed over a device. [`Boot::graph_on_canvas`] is what keeps that
    /// answer the same as the one the first frame draws.
    ///
    /// **A saved layout outranks this.** The live binary consults it only when
    /// nothing was restored, and never for a [`Boot::empty`] — see
    /// [`Boot::is_empty`]. The headless capture path and the pixel tier have
    /// no saved layout and no user to have arranged one, so this stays their
    /// only answer.
    ///
    /// **And the display outranks the answer.** *Natural* is the operative
    /// word: this is what the content wants, in a vacuum, and for the graph it
    /// is routinely wider than a laptop panel. The live binary caps it at the
    /// monitor the window opened on — [`window_size_on_display`] — and the
    /// capture tiers, which have no monitor and must not acquire one, do not.
    #[must_use]
    pub fn window_size(&self) -> (f32, f32) {
        if self.graph_on_canvas() {
            // The ENVELOPE, not the boot canvas. Sizing to the boot canvas
            // fitted the graph the window opens on and nothing else: every
            // state one keystroke away overflowed and stayed overflowed,
            // because a window is sized once and never resized. See
            // `ProtocolModel::boot_extent`, which also names the two states
            // the envelope deliberately leaves to scroll.
            let (w, h) = ProtocolModel::boot_extent(&self.protocol, self.flow);
            protocol_window_size_for(w as f32, h as f32)
        } else {
            chart_window_size(&self.composed)
        }
    }

    /// The window title: the subject of whatever the canvas will hold.
    ///
    /// The same answer [`MeridianApp::title`] gives once the window exists,
    /// because both read [`graph_takes_the_canvas`] over the same documents.
    /// It matters more here than it looks: `main` hands this string to
    /// `eframe::run_native`, which is where the OS window's title comes from,
    /// and a `ViewportCommand::Title` is sent afterwards by opening a start
    /// (`open_start`) and by going home (`open_home`) — both private, both
    /// re-titling from the documents they just changed. A title that is wrong
    /// at this call stays wrong until one of those runs.
    #[must_use]
    pub fn title(&self) -> String {
        if self.graph_on_canvas() {
            format!("Protocol · {}", self.protocol.protocol)
        } else {
            self.composed
                .title
                .clone()
                .unwrap_or_else(|| "Brightfield".to_string())
        }
    }

    /// One line describing what this boot loaded, for the binaries' stderr.
    ///
    /// **The protocol form carries a degrade count when there is one to
    /// carry**, because the other three numbers on that line — collapsed nodes,
    /// full nodes, steps — do not report whether the render is complete. A
    /// complete render prints no such clause. Measured through this method in
    /// `crates/brightfield-shell/tests/protocol_degrade_channel.rs`, over the
    /// three-step manifest that suite builds: with its two-statement model,
    /// readable / absent / refused each produce the same
    /// `5 collapsed / 5 full nodes, 3 steps` behind three different pictures.
    /// Widen the model to four statements and readable moves to
    /// `7 collapsed / 7 full nodes` while both faults stay at 5/5, the step
    /// count 3 throughout. So neither a match nor a mismatch settles anything
    /// for a caller holding a single render: nothing in that render says which
    /// model it should have had. The count is stated rather than inferred.
    #[must_use]
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return "nothing open".to_string();
        }
        if self.graph_on_canvas() {
            format!(
                "protocol {} ({} collapsed / {} full nodes, {} steps, {:?} flow{})",
                self.protocol.protocol,
                self.protocol.graph_collapsed.nodes.len(),
                self.protocol.graph_full.nodes.len(),
                self.protocol.sheet_rows.len(),
                self.flow,
                match self.protocol.degrade_report().len() {
                    0 => String::new(),
                    n => format!(", {n} degraded"),
                },
            )
        } else {
            format!(
                "composed {}x{} dashboard",
                self.composed.width, self.composed.height
            )
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
    /// Whether the flow toggle was pressed.
    toggle_flow: bool,
    /// Whether the Home button was pressed — the return to the front door.
    home: bool,
    /// The Home button's rect, when the band drew one — see
    /// [`MeridianApp::home_rect`]. `None` on the front door, which draws no
    /// Home button because it is already home.
    home_rect: Option<egui::Rect>,
}

/// What one frame's region controls were asked for: the canvas toggle and the
/// two rails' selector strips, answered after their closures have returned.
#[derive(Default)]
struct RegionPicks {
    /// The projection the canvas toggle was clicked to.
    projection: Option<usize>,
    /// The pane the ledger rail's strip was clicked to.
    ledger: Option<usize>,
    /// The pane the inspector rail's strip was clicked to.
    inspector: Option<usize>,
    /// The rails whose collapse control was clicked this frame.
    collapse: Vec<RegionId>,
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
        "cmd-b" => ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::B)),
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

/// The chart half of the window: its document and its live items.
struct ChartView {
    doc: ChartDoc,
    items: ItemMap<ChartDoc>,
    /// What the inspector rail says is selected — written by [`MeridianApp::draw`]
    /// from [`Workspace::focus`] right before the dock draws, read by the
    /// [`InspectorPane`] boxed inside `items` at the [`CONTROLS`] key. See
    /// `crate::inspector`'s module docs for why this is not a field on
    /// [`ChartDoc`].
    inspector_selection: Selection,
}

/// The protocol half of the window: its document and its live items.
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
    /// Where each region of the arrangement was drawn in the last frame this
    /// window drew, in window-space logical points — empty until a frame has
    /// been laid out, and holding only the regions that drew.
    ///
    /// Recorded for the reason `ChartDoc::overlay_checkbox` is: the assertion
    /// that a rail is the width it declares has to read the rect the rail was
    /// *drawn* at, and a test comparing the declared constant with itself is
    /// green whatever the window does. Read back through
    /// [`MeridianApp::region_rect`].
    regions: Vec<(RegionId, egui::Rect)>,
    /// The regions the user has put away this session, by the id the
    /// arrangement declares them under.
    ///
    /// # Why this is session state and not in the saved layout
    ///
    /// It joins [`Self::projection`], [`Self::ledger_panel`] and
    /// [`Self::inspector_panel`] rather than [`SavedLayout`], and those three
    /// are the argument: a collapsed rail is set from the same strip, at the
    /// same granularity, by the same click as which pane that strip is
    /// showing, and the other three are dropped at the next launch. Restoring a
    /// collapsed ledger while the pane inside it came back at whatever this
    /// build defaults to would be a half-restored rail, which reads as a bug
    /// rather than as a restore. The day the window persists how a region is
    /// presented, it persists the four together, at one [`LAYOUT_VERSION`]
    /// bump — and because the collapsed extent is declared in
    /// [`brightfield_workbench::arrangement`], [`chrome_budget`] gains a term
    /// that reads it rather than a number of its own.
    ///
    /// The comparison that settles it the other way is focus, which
    /// [`Workspace`] excludes from its own equality *because* it is
    /// transient. That is not this: a rail the user put away is visible, and
    /// they know they put it there.
    ///
    /// [`LAYOUT_VERSION`]: brightfield_workbench::persist::LAYOUT_VERSION
    collapsed: BTreeSet<RegionId>,
    /// What each rail's strip drew in the last frame this window drew — the
    /// names and the collapse control, in window-space logical points.
    ///
    /// Recorded for the reason [`Self::regions`] is: the test that a collapsed
    /// rail can be reopened has to click the control where it was laid out,
    /// and the control is drawn by the chrome, so nothing else knows. Read
    /// back through [`MeridianApp::rail_collapse_rect`] and
    /// [`MeridianApp::rail_name_rect`].
    strips: Vec<(RegionId, chrome::StripDrawn)>,
    /// Which of the canvas's projections is showing — an index into the
    /// arrangement's declared projections.
    projection: usize,
    /// Which of the ledger rail's panes is showing.
    ledger_panel: usize,
    /// Which of the inspector rail's panes is showing.
    inspector_panel: usize,
    /// Where each segment of the canvas toggle was drawn, in window-space
    /// logical points — the hook a test aims a click at, and counts to find a
    /// third projection. Empty on a frame that drew no toggle.
    canvas_toggle: Vec<egui::Rect>,
    /// Where focus was before the navigator rail's toggle took it, so pressing
    /// that toggle again puts it back. `None` when the rail does not hold
    /// focus — see [`MeridianApp::toggle_navigator_focus`].
    focus_return: Option<PaneKey>,
    /// Where the top bar drew the Home button in the last frame this window
    /// drew, in window-space logical points — recorded for the reason
    /// [`Self::regions`] is, and read back through [`MeridianApp::home_rect`].
    /// `None` on a frame the bar drew no Home button (the front door).
    home_button: Option<egui::Rect>,
    /// Where each empty pane drew the button that resolves it, in window-space
    /// logical points — recorded for exactly the reason [`Self::regions`] is,
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
    /// Which sections the front door drew, in the order it drew them, and
    /// where each one's heading landed.
    ///
    /// The order is the morph: a first run leads with Datasets because there
    /// is nothing else to lead with, and a door with recents leads with
    /// Protocols. Recorded rather than re-derived so a test asks the door
    /// what it drew instead of asking the layout what it should have drawn.
    door_sections: Vec<(&'static str, egui::Rect)>,
    /// The rows the front door's Protocols section drew, most recent first —
    /// each carrying the text of the galleys that row painted, so a test reads
    /// what was handed to the painter rather than what the layout holds.
    door_rows: Vec<DoorRow>,
    /// Where the front door drew the Start zone's keyboard-help control.
    door_help: Option<egui::Rect>,
    /// Where the front door drew the Start zone's open-a-file control —
    /// recorded for the reason [`Self::door_cards`] is, so the test that proves
    /// the verb is reachable clicks it where it was actually laid out.
    door_open_file: Option<egui::Rect>,
    /// Whether this frame's front door asked for the file dialog.
    ///
    /// A flag rather than a call inside the door's closure: `pick` blocks on an
    /// operating-system modal, and blocking inside the layout pass would hold
    /// the `Ui` borrow across a window this process does not own. It is read
    /// after the frame's requests are applied — see [`MeridianApp::draw`].
    pick_requested: bool,
    /// Whether this app may raise an operating-system dialog at all.
    ///
    /// **Off unless the live window turns it on**, which is the safe default
    /// rather than the polite one. Three tiers build a `MeridianApp` and only
    /// one of them has a person in front of it: `capture_png` and the
    /// `brightfield-shot` binary drive real frames through real wgpu devices
    /// with no window and no user, and `brightfield-shot --script` feeds
    /// synthetic pointer events — so a scripted click that happened to land on
    /// the door's open control would raise a modal on a CI runner and block
    /// there until the job timed out. The headless test tiers are the same
    /// case one step further down.
    ///
    /// So the permission is granted at exactly one call site, by
    /// [`MeridianApp::allowing_dialogs`], and `main` is the only caller.
    dialogs_allowed: bool,
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
    /// The `toggle-outline-rail` keystroke token, read off the registry at
    /// boot — same rule as [`Self::home_binding`]: the shell wires whichever
    /// binding the registry declares, and does not invent one of its own.
    navigator_binding: Option<&'static str>,
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
    /// dismissals — recorded for the reason [`Self::regions`] is: a headless
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
        if let Some(authored) = boot.authored {
            doc.set_authored(authored);
        }
        let model = ProtocolModel::new(boot.protocol, boot.flow);
        Self::assemble(
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
        if let Some(authored) = boot.authored {
            doc.set_authored(authored);
        }
        let model = ProtocolModel::new(boot.protocol, boot.flow);
        Self::assemble(boot.focus, layout, doc, ProtocolDoc::headless(model), mode)
    }

    fn assemble(
        focus: Option<String>,
        mut layout: SavedLayout,
        chart_doc: ChartDoc,
        mut protocol_doc: ProtocolDoc,
        mode: Mode,
    ) -> Self {
        // Both registries' vocabularies, published before any layout file
        // could be read. Idempotent, and both are needed whichever document
        // the boot filled: the window's one tree carries the panes of both,
        // so a `PaneKey` naming a pane of the empty one has to deserialise
        // too, or a saved layout loads as corrupt. `startup` publishes them
        // again ahead of the read that happens before this window exists;
        // these stay because the headless tiers never go through `startup` at
        // all.
        crate::app::publish_item_ids();
        crate::protocol::publish_item_ids();

        if let Some(id) = focus {
            protocol_doc.model.select_id(id);
        }

        let charts = chart_registry();
        let protocol = protocol_registry();

        // The registry still lays CONTROLS out as a rail of the dock's own
        // `Slot::Rail` — that declaration stays app.rs's, untouched, so
        // `chart_contract.rs`'s registry-level assertions about it keep
        // passing (see `crate::inspector`'s module docs). What draws there
        // now is a genuine `Panel::right` (`Self::draw`), sized from
        // `INSPECTOR_RAIL_WIDTH` rather than from the dock's proportional
        // share — so the tile the tree still carries for it is hidden here
        // rather than removed: `egui_tiles::Tiles::set_visible` keeps its
        // place in the persisted tree (a saved arrangement still deserialises
        // against it) while excluding it from `Linear::layout`, so the centre
        // tile takes the whole of what the dock is given instead of sharing
        // it with an invisible sibling.
        //
        // On the raw `layout`, before `DirtyTracker::new` below starts
        // comparing: a launch that leaves this pane untouched must write
        // it back unchanged, for the same reason `steps_tab_is_active`'s
        // seeding a few lines down reads the document rather than the tree.
        if let Some(controls_tile) = layout.workspace.tile_of(charts.pane_key(CONTROLS)) {
            layout
                .workspace
                .tree_mut()
                .tiles
                .set_visible(controls_tile, false);
        }

        let layout = DirtyTracker::new(layout);

        // The restored tab strip is the authority over the model's default,
        // not the other way round. `ProtocolModel` boots with its sheet shut,
        // `draw` makes the strip authoritative from that flag on every frame
        // the graph holds the canvas, and the strip's active tab is part of
        // the serialised tree — so without this line a file that recorded the
        // Steps sheet is overwritten with Canvas before any frame can read it,
        // and the overwrite is a tile-tree mutation, so a launch nobody
        // touched goes dirty and the debounce writes the reverted arrangement
        // back. The restore was one-way lossy and it rewrote the file to say
        // so. Held by `a_restored_steps_tab_survives_the_first_frame`.
        if let Some(show) = steps_tab_is_active(layout.live().workspace.tree()) {
            protocol_doc.model.set_show_sheet(show);
        }

        // The registry still builds a `ControlsPane` at `CONTROLS` — that
        // declaration lives in `app.rs` and stays untouched, so the
        // registry-level assertions in `chart_contract.rs` keep passing. What
        // actually draws is swapped here, one map entry, right after
        // construction: an `InspectorPane` sharing a `Selection` cell with
        // the `ChartView` it sits in. See `crate::inspector`'s module docs.
        let inspector_selection = Selection::default();
        let mut chart_items = charts.instantiate();
        chart_items.insert(
            charts.pane_key(CONTROLS),
            Box::new(InspectorPane::new(inspector_selection.clone())),
        );

        // The inspector rail opens on the pane belonging to whichever document
        // is leading the window — the operator when the graph has the canvas,
        // the chart's own controls otherwise. Derived from the documents, by
        // the same rule the canvas is, so the rail cannot open on the chart's
        // controls over a window with no chart in it. Read before the two
        // documents are moved into the fields below.
        let inspector_panel = usize::from(!graph_takes_the_canvas(
            protocol_doc.model.has_assets(),
            !chart_doc.is_empty(),
        ));

        let mut app = Self {
            layout,
            charts: ChartView {
                doc: chart_doc,
                items: chart_items,
                inspector_selection,
            },
            protocol: ProtocolView {
                doc: protocol_doc,
                items: protocol.instantiate(),
            },
            mode,
            fonts_installed: false,
            regions: Vec::new(),
            collapsed: BTreeSet::new(),
            strips: Vec::new(),
            // The chart's drawn reading, not its tabular one: a window that
            // opens on a grid of numbers has buried the picture it exists to
            // present. The index is into the arrangement's declared
            // projections, which name the grid first and the chart second.
            projection: 1,
            ledger_panel: 0,
            inspector_panel,
            canvas_toggle: Vec::new(),
            focus_return: None,
            home_button: None,
            affordances: Vec::new(),
            door_thumbs: Vec::new(),
            door_cards: Vec::new(),
            door_sections: Vec::new(),
            door_rows: Vec::new(),
            door_help: None,
            door_open_file: None,
            pick_requested: false,
            dialogs_allowed: false,
            overlay: None,
            overlay_keys: OverlayKeys::from_registry(),
            home_binding: brightfield_keys::registry()
                .iter()
                .find(|v| v.longname == "open-home")
                .and_then(brightfield_keys::VerbEntry::primary_key),
            navigator_binding: brightfield_keys::registry()
                .iter()
                .find(|v| v.longname == NAVIGATOR_TOGGLE)
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
    /// document nobody had open any more. A chart-document swap that reaches
    /// past this is the same bug again, so the reviewer's question about any
    /// new one is "does it call this".
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

    /// Whether the canvas is drawing the asset graph rather than a chart
    /// projection — [`graph_takes_the_canvas`] over this window's documents.
    ///
    /// Recomputed on each call rather than latched, which is what makes it
    /// the same answer [`MeridianApp::draw`] uses and the same answer
    /// [`Boot::graph_on_canvas`] gave before the window existed.
    #[must_use]
    pub fn graph_on_canvas(&self) -> bool {
        graph_takes_the_canvas(
            self.protocol.doc.model.has_assets(),
            !self.charts.doc.is_empty(),
        )
    }

    /// The pane the window's chrome is reading from, if any.
    ///
    /// One record for the window, because both documents' panes are drawn in
    /// the same frame and two records would mean two panes wearing the focus
    /// ring. Read back by the tests that press the navigator rail's toggle.
    #[must_use]
    pub fn focused_pane(&self) -> Option<PaneKey> {
        self.ws().focus()
    }

    /// The window title: the subject of whatever the canvas holds.
    ///
    /// [`Boot::title`] is the same question answered before the window exists,
    /// and the two agree because both read [`graph_takes_the_canvas`] over the
    /// same documents. It matters that they agree: `main` hands `Boot::title`'s
    /// answer to `eframe::run_native`, and the runtime `ViewportCommand::Title`s
    /// in the workspace — in `open_start` and `open_home` — re-title from this
    /// same method, so a restored session that reaches neither keeps
    /// `Boot::title`'s answer.
    /// `a_restored_session_is_titled_for_the_surface_it_draws` asserts the
    /// agreement rather than either answer, because a literal on both sides
    /// would go on matching itself after either drifted.
    #[must_use]
    pub fn title(&self) -> String {
        // The front door replaces the regions, so it has no document subject
        // to name — and the top bar draws this unconditionally. Reaching the
        // door from a graph would otherwise show "Protocol · " (the emptied
        // graph's blank name) in the visible chrome for the whole visit. A
        // cold-boot door is already this string (an empty `ChartDoc` titles
        // "Brightfield"), so this fixes the reached-from-a-graph case.
        if self.front_door_is_live() {
            return "Brightfield".to_string();
        }
        if self.graph_on_canvas() {
            format!("Protocol · {}", self.protocol.doc.model.protocol)
        } else {
            self.charts.doc.title().to_string()
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

    /// What the inspector currently says is selected — the focused pane's
    /// [`Subject`], recomputed each frame from [`Workspace::focus`]
    /// immediately before the dock draws. `None` before anything in the
    /// charts view has been focused, or once focus is cleared — never a
    /// stale previous answer. A test hook, for the reason
    /// [`Self::overlay_checkbox`] is one: proving the inspector tracks
    /// selection should not have to pay for a pixel capture per pane.
    #[must_use]
    pub fn inspector_selection(&self) -> Option<Subject> {
        self.charts.inspector_selection.get()
    }

    /// Move focus to `key` — the same effect the
    /// click-anywhere-in-a-pane rule in `PaneChrome::pane_ui` has, without
    /// simulating a pointer event. Returns whether the move was accepted
    /// (see [`Workspace::set_focus`]). A test hook, for the reason
    /// [`Self::inspector_selection`] is one.
    pub fn focus_pane(&mut self, key: PaneKey) -> bool {
        self.ws_mut().set_focus(key)
    }

    /// Drop the window's focus record, as if the focused pane had just closed.
    /// The other half of [`Self::focus_pane`] — proving the inspector reverts
    /// to its empty-selection state rather than holding a stale one.
    pub fn clear_focus(&mut self) {
        self.ws_mut().clear_focus();
    }

    /// What the item actually occupying `key` in the *live* item map declares
    /// about itself — as opposed to `chart_registry()`'s declared shape,
    /// which `chart_contract.rs` already covers and which `assemble` swaps
    /// one entry away from at boot. A test hook: this is how a test tells
    /// `InspectorPane` from the registry's dormant `ControlsPane` without a
    /// pixel capture — their titles differ ("Inspector" vs "Controls").
    #[must_use]
    pub fn chart_pane_title(&self, key: PaneKey) -> Option<String> {
        self.charts
            .items
            .get(&key)
            .map(|item| item.describe(&self.charts.doc).title)
    }

    /// The toolbar `key`'s live item declares right now — the other half of
    /// [`Self::chart_pane_title`], and for the same reason: a pane's
    /// declared toolbar (e.g. the editor's `save-spec`, once a file is open)
    /// only exists after `describe` has run against a document that has
    /// actually drawn a frame, so a test that wants it has to ask the live
    /// app rather than `chart_registry()`'s freshly constructed, never-drawn
    /// items.
    #[must_use]
    pub fn chart_pane_toolbar(&self, key: PaneKey) -> Vec<ToolbarEntry> {
        self.charts
            .items
            .get(&key)
            .map(|item| item.describe(&self.charts.doc).toolbar)
            .unwrap_or_default()
    }

    /// One pane's own [`Subject`] title, whichever document owns it.
    ///
    /// The words a region's selector strip offers its panes under come from
    /// here rather than from a second table beside the arrangement, for the
    /// reason [`chrome::pane_frame`] takes its header from the subject: a
    /// strip and a pane header saying different things about one pane is the
    /// drift the workbench exists to end.
    fn pane_title_of(&self, item: ItemId) -> String {
        let key = PaneKey::new(item);
        if let Some(pane) = self.protocol.items.get(&key) {
            return pane.subject(&self.protocol.doc).title;
        }
        if let Some(pane) = self.charts.items.get(&key) {
            return pane.subject(&self.charts.doc).title;
        }
        item.to_string()
    }

    /// [`Self::pane_title_of`] over a region's declared panes, in declaration
    /// order.
    fn pane_titles(&self, items: &[ItemId]) -> Vec<String> {
        items.iter().map(|item| self.pane_title_of(*item)).collect()
    }

    /// Where the document on the canvas came from, for the locator band's
    /// right-hand note — the spec file's **name**, or `None` for a document
    /// with no file behind it (a shipped start, an opened data file, an empty
    /// window).
    ///
    /// The name rather than the path, and it is not a shortening for taste: a
    /// path is where this machine keeps the file, so a window drawn on two
    /// machines would say two different things and every pixel baseline that
    /// photographs this band would be a baseline of one developer's home
    /// directory. The file name is the part the reader wrote.
    fn document_source(&self) -> Option<String> {
        self.charts
            .doc
            .spec_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
    }

    /// The breadcrumb the locator band draws: where the subject sits, most
    /// general first.
    ///
    /// The protocol's own drill trail when the graph holds the canvas, and the
    /// window's title otherwise — a chart has no drill state, and a locator
    /// band that went blank on the surface a stranger meets first would be a
    /// row of empty chrome.
    fn crumb_line(&self) -> Vec<String> {
        let mut crumbs = vec![self.title()];
        if self.graph_on_canvas() {
            crumbs.extend(self.protocol.doc.model.breadcrumb());
        }
        crumbs
    }

    /// The content box the DAG canvas pane was handed by the last frame this
    /// window drew, or `None` if it has not drawn one.
    #[must_use]
    pub fn canvas_viewport(&self) -> Option<egui::Rect> {
        self.protocol.doc.viewport
    }

    /// The rect region `id` was drawn at in the last frame this window drew,
    /// or `None` on a frame it did not draw.
    ///
    /// The *drawn* rect, which is the whole point: a resizable panel reports
    /// its content's extent rather than its declared one unless the content
    /// claims the space, and it persists the narrower number across frames.
    /// A test comparing a declared constant with itself cannot see that, and
    /// `the_drawn_regions_match_the_declared_arrangement` reads this instead.
    #[must_use]
    pub fn region_rect(&self, id: RegionId) -> Option<egui::Rect> {
        self.regions
            .iter()
            .find(|(r, _)| *r == id)
            .map(|(_, rect)| *rect)
    }

    /// Every region the last frame drew, in draw order.
    #[must_use]
    pub fn drawn_regions(&self) -> &[(RegionId, egui::Rect)] {
        &self.regions
    }

    /// Where the collapse control of rail `id` drew in the last frame this
    /// window drew, or `None` on a frame that rail did not draw one.
    ///
    /// The gesture that puts a rail away and brings it back, at the place a
    /// pointer would find it — recorded and read back for the reason
    /// [`MeridianApp::home_rect`] is.
    #[must_use]
    pub fn rail_collapse_rect(&self, id: RegionId) -> Option<egui::Rect> {
        self.strips
            .iter()
            .find(|(r, _)| *r == id)
            .and_then(|(_, strip)| strip.control)
    }

    /// Where the `index`-th name in rail `id`'s strip drew in the last frame,
    /// or `None` where that rail drew no such name.
    #[must_use]
    pub fn rail_name_rect(&self, id: RegionId, index: usize) -> Option<egui::Rect> {
        self.strips
            .iter()
            .find(|(r, _)| *r == id)
            .and_then(|(_, strip)| strip.names.get(index).copied())
    }

    /// Where the canvas toggle drew each of its segments in the last frame,
    /// left to right — empty on a frame that drew no toggle.
    ///
    /// The hook a test aims a click at, and the one it counts to find a third
    /// projection: the toggle is built from the arrangement's declared
    /// projections, so this is that list as it reached the screen.
    #[must_use]
    pub fn canvas_toggle_segments(&self) -> &[egui::Rect] {
        &self.canvas_toggle
    }

    /// Which of the canvas's projections is showing — an index into the
    /// declared projections.
    #[must_use]
    pub const fn projection(&self) -> usize {
        self.projection
    }

    /// The rect the top bar's Home button occupied in the last frame this
    /// window drew, or `None` on a frame it drew none — the front door draws
    /// no Home button, because it is already home. Recorded and read back for
    /// the reason [`MeridianApp::region_rect`] is: the test that proves Home
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
    /// open in either document, so the window's answer is an invitation rather
    /// than a set of empty instruments.
    ///
    /// Not a mode and not a setting — a fact about the documents, recomputed
    /// every frame, which is why the door needs no dismissal: it is
    /// outcompeted by content the moment either document has any.
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

    /// Which sections the last frame's front door drew, in the order it drew
    /// them, and where each heading landed.
    ///
    /// Empty on a frame the door did not draw — the same answer
    /// [`MeridianApp::front_door_card_rect`] gives, and for the same reason:
    /// a test asserting the door is gone asks this too.
    #[must_use]
    pub fn front_door_sections(&self) -> &[(&'static str, egui::Rect)] {
        &self.door_sections
    }

    /// The rows the last frame's front door drew in its Protocols section,
    /// most recent first.
    ///
    /// The strings are the galleys' own, not the [`Recent`] fields the row
    /// was built from: a row painted from the wrong field, or labelled by
    /// recomputing a run state the door has no business computing, differs
    /// here.
    #[must_use]
    pub fn front_door_rows(&self) -> &[DoorRow] {
        &self.door_rows
    }

    /// The rect of the front door's keyboard-help control, when the last
    /// frame drew the door.
    #[must_use]
    pub fn front_door_help_rect(&self) -> Option<egui::Rect> {
        self.door_help
    }

    /// The rect of the front door's open-a-data-file control, when the last
    /// frame drew the door.
    ///
    /// `None` on a frame the door did not draw, exactly as
    /// [`MeridianApp::front_door_card_rect`] answers — the door's controls are
    /// its own, not the window's.
    #[must_use]
    pub fn front_door_open_file_rect(&self) -> Option<egui::Rect> {
        self.door_open_file
    }

    /// Whether the last frame's door asked for the file dialog — the test hook
    /// that proves the control is wired without opening a dialog on anyone's
    /// desktop.
    #[must_use]
    pub fn pick_requested(&self) -> bool {
        self.pick_requested
    }

    /// Permit this app to raise operating-system dialogs — the live window's
    /// declaration that there is a person in front of it.
    ///
    /// `main` is the only caller, and the default is off for the reason the
    /// `dialogs_allowed` field records: the capture tiers and the pixel tier
    /// drive the same frames with no window and no user, and a modal raised
    /// there blocks until something kills the process.
    #[must_use]
    pub fn allowing_dialogs(mut self) -> Self {
        self.dialogs_allowed = true;
        self
    }

    /// The protocol view's interaction model, read-only.
    ///
    /// The window is the only thing that feeds it keys, and it feeds it keys
    /// only while the graph holds the canvas — so this is how a test asks
    /// whether a keystroke reached the DAG, which is the half of that gate that
    /// cannot be seen from outside.
    #[must_use]
    pub fn protocol_model(&self) -> &ProtocolModel {
        &self.protocol.doc.model
    }

    /// The chart document, read-only — what a test asks whether the front
    /// door's second click actually filled.
    #[must_use]
    pub fn chart_doc(&self) -> &ChartDoc {
        &self.charts.doc
    }

    /// The protocol document, read-only. The twin of [`Self::chart_doc`].
    #[must_use]
    pub fn protocol_doc(&self) -> &ProtocolDoc {
        &self.protocol.doc
    }

    /// The chart document, mutably — the seam an embedder (or a test)
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

        // Which document the canvas draws — content decides it, and content
        // alone. A chart on the canvas whenever there is a chart; the graph
        // only where there is no chart for the canvas to hold. See
        // `graph_takes_the_canvas` for why the tie goes that way.
        //
        // Derived rather than latched, and that is what closes a dead end.
        // The window used to carry a recorded active view that a boot or an
        // open could set, which made the canvas a function of the order the
        // two documents were opened in: open a chart start and then a protocol
        // start and the canvas went to the graph and stayed there. There is no
        // control that moves the canvas between the two — the graph at canvas
        // size is a different arrangement, reached by a toggle a build
        // supporting it would declare, and inventing one here would be the
        // peer switcher again under a new name — so a state reached by history
        // was a state with no way out of it. Every exit was Home, which
        // empties both documents.
        //
        // `a_protocol_opened_over_a_chart_leaves_the_chart_on_the_canvas` is
        // the pin. It reaches this state through two direct `open_start`
        // calls, not through the UI — `Boot::open` takes exactly one spec,
        // and no shipped control opens a second document into a window that
        // already holds one.
        let graph_on_canvas = self.graph_on_canvas();

        // The overlay-opening keys, before the grammar feed so the frame that
        // opens an overlay is already under it.
        self.overlay_open_keys(&ctx, graph_on_canvas);
        // Return-home, on the same gate: no overlay may own the keyboard, and
        // it is deliberately not an overlay-opener (so the registry cross-ref
        // that pins those three stays pinned).
        self.home_key(&ctx);
        // The navigator rail's round-trip focus toggle, on the same gate.
        self.navigator_key(&ctx);
        // The frame verbs, on the same gate and only where the chart holds the
        // canvas: they are bare keys, so an overlay or a text field must own
        // the keyboard first.
        self.navigation_keys(&ctx, graph_on_canvas);

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
        // with no modifier to disambiguate it — so it is fed only while the
        // graph is the thing on the canvas. Gating on that rather than on the
        // focused pane's `Subject::key_context` is deliberate: the grammar
        // drives the *document's* model, not one pane's, and the panes
        // reading that document declare the same context anyway. A per-pane
        // gate would be a second answer to a question the canvas already
        // answers.
        //
        // And it is fed only while no overlay is open — the
        // no-bare-under-overlay invariant. An open picker owns the keyboard;
        // a `j` typed into its query line must never also walk the DAG
        // underneath it.
        //
        // Nor while the front door is drawn: the grammar drives the DAG, and
        // the door is standing where the DAG would be.
        if graph_on_canvas && !door {
            if self.overlay.is_none() {
                let events = ctx.input(|i| i.events.clone());
                self.protocol.doc.model.feed_events(&events);
            }
            if let Some(addr) = self.protocol.doc.model.take_yank_request() {
                ctx.copy_text(addr);
            }
            self.set_active_tab();
        }

        // Only the document on the canvas rasters. The other one's texture is
        // not freed — see `sweep`.
        if graph_on_canvas {
            self.protocol.doc.present(ctx.pixels_per_point(), mode);
        } else {
            self.charts.doc.present(ctx.pixels_per_point(), mode);
        }

        // The window's arrangement, read once and read from nowhere else.
        // Every extent below comes out of it rather than out of a literal in
        // this draw path — see `brightfield_workbench::arrangement`, and
        // `the_drawn_regions_match_the_declared_arrangement`, which lays a real
        // frame out and compares each region's drawn rect with what is
        // declared there.
        let plan = arrangement::default_arrangement();
        self.regions.clear();
        self.strips.clear();
        // Cleared once per frame rather than per `PaneChrome`: several regions
        // build one each, and clearing on construction meant the last one wiped
        // what the earlier ones recorded.
        self.affordances.clear();

        let mut bar = TopBar::default();
        let title = plan.expect_region(arrangement::TITLE_BAND);
        let drawn = Panel::top("bf-title-band")
            .resizable(false)
            .exact_size(band_extent(title))
            .show(ui, |ui| bar = self.title_band(ui));
        self.regions.push((title.id, drawn.response.rect));
        self.home_button = bar.home_rect;

        let mut requests: Vec<Request> = Vec::new();
        let dock_frame = egui::Frame::new()
            .inner_margin(DOCK_INSET)
            .fill(ui.visuals().panel_fill);
        let rail_frame = egui::Frame::new().fill(ui.visuals().panel_fill);

        if door {
            // The front door, instead of every region below the title band: a
            // window of empty instruments is the surface the research warned
            // against, and each of its regions would be inviting the same
            // first action from a different corner. What a card click *does*
            // is the same `Request::Open` an empty pane's button raises — the
            // door is a different arrangement of the same way in, not a second
            // route.
            CentralPanel::default().frame(dock_frame).show(ui, |ui| {
                self.front_door_ui(ui, &mut requests);
            });
        } else {
            // Content somewhere, so the regions — and no stale door geometry: a
            // test that asks where a card was after the door has gone must
            // hear "nowhere", exactly as `affordances` answers for panes.
            self.door_cards.clear();
            self.door_sections.clear();
            self.door_rows.clear();
            self.door_help = None;
            self.door_open_file = None;

            // The inspector's own read of "what is selected" — the same value
            // `status_rail_ui` reads for the rail's status lines, computed here
            // rather than there because the inspector needs it *before* its
            // rail draws, not after. Skipped when focus has landed on the
            // inspector's own pane (clicking its checkbox, say): that would
            // otherwise blank the panel it is itself part of the moment
            // someone touches it.
            match self.ws().focus() {
                Some(key) if key.item != CONTROLS && self.charts.items.contains_key(&key) => {
                    let subject = self
                        .charts
                        .items
                        .get(&key)
                        .map(|item| item.describe(&self.charts.doc));
                    self.charts.inspector_selection.set(subject);
                }
                Some(_) => {}
                None => self.charts.inspector_selection.set(None),
            }

            let locator = plan.expect_region(arrangement::LOCATOR_BAND);
            let crumbs = self.crumb_line();
            let source = self.document_source();
            let drawn = Panel::top("bf-locator-band")
                .resizable(false)
                .exact_size(band_extent(locator))
                .show(ui, |ui| {
                    locator_band_ui(ui, &crumbs, source.as_deref(), mode)
                });
            self.regions.push((locator.id, drawn.response.rect));

            // The key-hint band belongs to a key grammar, so it is drawn where
            // there is one. The chart projections have no bare-key grammar, and
            // `chart_window_size` has no term for this band —
            // `the_window_it_asks_for_fits_the_raster_it_presents` is the
            // assertion that keeps it honest about that.
            if graph_on_canvas {
                let hint = plan.expect_region(arrangement::HINT_BAND);
                let model = &self.protocol.doc.model;
                let drawn = Panel::bottom("bf-hint-band")
                    .resizable(false)
                    .exact_size(band_extent(hint))
                    .show(ui, |ui| hint_ui(ui, model, mode));
                self.regions.push((hint.id, drawn.response.rect));
            }

            let ledger = plan.expect_region(arrangement::LEDGER_RAIL);
            let navigator = plan.expect_region(arrangement::NAVIGATOR_RAIL);
            let inspector = plan.expect_region(arrangement::INSPECTOR_RAIL);
            let canvas = plan.expect_region(arrangement::CANVAS);
            let ledger_panes = region_panes(ledger);
            let navigator_panes = region_panes(navigator);
            let inspector_panes = region_panes(inspector);
            let (projections, graph) = canvas_occupants(canvas);

            let projection = self.projection.min(projections.len() - 1);
            let ledger_panel = self.ledger_panel.min(ledger_panes.len() - 1);
            let inspector_panel = self.inspector_panel.min(inspector_panes.len() - 1);

            // Read before the borrows below take this app apart field by
            // field, so each rail's block has a plain `bool` rather than a
            // second borrow of `self`.
            let ledger_collapsed = self.collapsed.contains(&ledger.id);
            let navigator_collapsed = self.collapsed.contains(&navigator.id);
            let inspector_collapsed = self.collapsed.contains(&inspector.id);

            // Each strip's words are the panes' own `Subject` titles, read
            // before the closures below take their borrows of the documents.
            let ledger_labels = self.pane_titles(ledger_panes);
            let navigator_labels = self.pane_titles(navigator_panes);
            let inspector_labels = self.pane_titles(inspector_panes);
            let projection_labels: Vec<&str> = projections.iter().map(|p| p.label).collect();
            // What the canvas is showing, which is the leaf of the locator
            // rather than the pane's own kind: the toggle beside it already
            // says "Grid" and "Chart", so a head band repeating the pane's
            // title would name the reading twice and the subject not at all.
            let canvas_name = crumbs
                .last()
                .cloned()
                .unwrap_or_else(|| self.pane_title_of(graph));

            let mut regions = std::mem::take(&mut self.regions);
            let mut strips = std::mem::take(&mut self.strips);
            let mut canvas_toggle: Vec<egui::Rect> = Vec::new();
            let mut picks = RegionPicks::default();
            let (ws, charts, protocol, affordances) = (
                self.layout.workspace_mut(),
                &mut self.charts,
                &mut self.protocol,
                &mut self.affordances,
            );
            // A region draws its occupant under its own strip, so the pane's
            // header band is suppressed the same way a tab strip suppresses
            // it — `PaneChrome::pane_ui` takes that as its `tabbed` set.
            let mut headed: std::collections::HashSet<egui_tiles::TileId> =
                std::collections::HashSet::new();
            for item in [
                OUTLINE,
                PROTOCOL_INSPECTOR,
                STEPS,
                PROTOCOL_CANVAS,
                CONTROLS,
                EDITOR,
                CHART,
                DATA,
            ] {
                if let Some(tile) = ws.tile_of(PaneKey::new(item)) {
                    headed.insert(tile);
                }
            }
            let focused = ws.focus();

            // ---- the ledger rail, before the side rails so it spans the
            // window's width and they stop above it.
            //
            // Collapsed, it is a second `Panel` under a second id rather than
            // the same one at a smaller size, and the id is the mechanism:
            // egui stores a panel's size per id, so drawing the open rail's id
            // at the collapsed measure would overwrite the extent the user
            // dragged it to and reopening would come back at that measure.
            // Two ids keep the two sizes apart, which is what
            // `a_rail_reopens_at_the_extent_it_was_dragged_to` measures.
            // egui's own `Panel::show_switched` carries the same rule as a
            // `debug_assert`.
            let ledger_panel_widget = if ledger_collapsed {
                Panel::bottom("bf-ledger-rail-collapsed")
                    .resizable(false)
                    .exact_size(rail_collapsed(ledger))
            } else {
                Panel::bottom("bf-ledger-rail")
                    .default_size(rail_default(ledger))
                    .min_size(rail_min(ledger))
                    .resizable(true)
            };
            let mut ledger_strip = None;
            let drawn = ledger_panel_widget.frame(rail_frame).show(ui, |ui| {
                let caret = collapse_caret(ledger.edge, ledger_collapsed);
                if ledger_collapsed {
                    // Collapsed, this rail is its own strip drawn by the same
                    // call that draws it open — names live, and no body under
                    // it to put a pane in. Split rather than handed the whole
                    // rect: what the arrangement declares this rail collapses
                    // to is the strip *plus* clearance for the status rail's
                    // floating band, and the strip is the head of that.
                    let (strip, _) = chrome::rail_split(ui.max_rect());
                    ledger_strip = Some(chrome::rail_selector(
                        ui,
                        strip,
                        &pane_labels(&ledger_labels),
                        ledger_panel,
                        Some(caret),
                        mode,
                    ));
                    return;
                }
                // `Panel::resizable`'s own doc: a resizable panel whose
                // content does not claim the available space shrinks to
                // content instead of holding its declared size, and egui
                // *persists* the narrower number for next frame. This is
                // the claim, on the axis this rail is resizable along.
                ui.set_min_height(ui.available_height());
                ui.set_min_width(ui.available_width());
                let (strip, body) = chrome::rail_split(ui.max_rect());
                ledger_strip = Some(chrome::rail_selector(
                    ui,
                    strip,
                    &pane_labels(&ledger_labels),
                    ledger_panel,
                    Some(caret),
                    mode,
                ));
                let item = ledger_panes[ledger_panel];
                if item == STEPS {
                    draw_protocol_pane(
                        ui,
                        body,
                        protocol,
                        ws,
                        item,
                        mode,
                        focused,
                        &headed,
                        &mut requests,
                        affordances,
                    );
                } else {
                    draw_chart_pane(
                        ui,
                        body,
                        charts,
                        ws,
                        item,
                        mode,
                        focused,
                        &headed,
                        &mut requests,
                        affordances,
                    );
                }
            });
            if let Some(strip) = ledger_strip {
                picks.ledger = strip.picked;
                if strip.toggled {
                    picks.collapse.push(ledger.id);
                }
                strips.push((ledger.id, strip));
            }
            regions.push((ledger.id, drawn.response.rect));

            // ---- the navigator rail: the protocol, as an ordered spine.
            //
            // Two ids for the two states, as the ledger's block explains.
            let navigator_panel_widget = if navigator_collapsed {
                Panel::left("bf-navigator-rail-collapsed")
                    .resizable(false)
                    .exact_size(rail_collapsed(navigator))
            } else {
                Panel::left("bf-navigator-rail")
                    .default_size(rail_default(navigator))
                    .min_size(rail_min(navigator))
                    .resizable(true)
            };
            let mut navigator_strip = None;
            let drawn = navigator_panel_widget.frame(rail_frame).show(ui, |ui| {
                let caret = collapse_caret(navigator.edge, navigator_collapsed);
                if navigator_collapsed {
                    // A side rail collapses along its width, so what is left
                    // is that measure of *width* — a stub with room for the
                    // control that reopens it and none for a name.
                    navigator_strip = Some(chrome::rail_stub(ui, ui.max_rect(), caret, mode));
                    return;
                }
                ui.set_min_width(ui.available_width());
                let (strip, body) = chrome::rail_split(ui.max_rect());
                navigator_strip = Some(chrome::rail_selector(
                    ui,
                    strip,
                    &pane_labels(&navigator_labels),
                    0,
                    Some(caret),
                    mode,
                ));
                draw_protocol_pane(
                    ui,
                    body,
                    protocol,
                    ws,
                    navigator_panes[0],
                    mode,
                    focused,
                    &headed,
                    &mut requests,
                    affordances,
                );
            });
            if let Some(strip) = navigator_strip {
                if strip.toggled {
                    picks.collapse.push(navigator.id);
                }
                strips.push((navigator.id, strip));
            }
            regions.push((navigator.id, drawn.response.rect));

            // ---- the inspector rail: what the selection is, from whichever
            // document owns the selection.
            //
            // Two ids for the two states, as the ledger's block explains.
            let inspector_panel_widget = if inspector_collapsed {
                Panel::right("bf-inspector-rail-collapsed")
                    .resizable(false)
                    .exact_size(rail_collapsed(inspector))
            } else {
                Panel::right("bf-inspector-rail")
                    .default_size(rail_default(inspector))
                    .min_size(rail_min(inspector))
                    .resizable(true)
            };
            let mut inspector_strip = None;
            let drawn = inspector_panel_widget.frame(rail_frame).show(ui, |ui| {
                let caret = collapse_caret(inspector.edge, inspector_collapsed);
                if inspector_collapsed {
                    inspector_strip = Some(chrome::rail_stub(ui, ui.max_rect(), caret, mode));
                    return;
                }
                // Measured before this line existed: the rail's reported
                // rect was 200pt wide — its declared floor — by the second
                // frame rather than the declared 280, because a quiet
                // inspector does not ask for more. Watched redden without
                // it: `the_overlay_toggle_still_reaches_the_chart_pane`
                // fails, the scripted click aimed at the wider prediction
                // landing outside the shrunken panel.
                ui.set_min_width(ui.available_width());
                let (strip, body) = chrome::rail_split(ui.max_rect());
                inspector_strip = Some(chrome::rail_selector(
                    ui,
                    strip,
                    &pane_labels(&inspector_labels),
                    inspector_panel,
                    Some(caret),
                    mode,
                ));
                let item = inspector_panes[inspector_panel];
                if item == PROTOCOL_INSPECTOR {
                    draw_protocol_pane(
                        ui,
                        body,
                        protocol,
                        ws,
                        item,
                        mode,
                        focused,
                        &headed,
                        &mut requests,
                        affordances,
                    );
                } else {
                    draw_chart_pane(
                        ui,
                        body,
                        charts,
                        ws,
                        item,
                        mode,
                        focused,
                        &headed,
                        &mut requests,
                        affordances,
                    );
                }
            });
            if let Some(strip) = inspector_strip {
                picks.inspector = strip.picked;
                if strip.toggled {
                    picks.collapse.push(inspector.id);
                }
                strips.push((inspector.id, strip));
            }
            regions.push((inspector.id, drawn.response.rect));

            // ---- the canvas: the remainder, and it comes last because a
            // `CentralPanel` takes what the panels before it left.
            let drawn = CentralPanel::default().frame(rail_frame).show(ui, |ui| {
                let (head, body) = chrome::rail_split(ui.max_rect());
                if graph_on_canvas {
                    canvas_head(ui, head, &canvas_name, None, mode);
                    draw_protocol_pane(
                        ui,
                        body,
                        protocol,
                        ws,
                        graph,
                        mode,
                        focused,
                        &headed,
                        &mut requests,
                        affordances,
                    );
                } else {
                    let toggle = canvas_head(
                        ui,
                        head,
                        &canvas_name,
                        Some((&projection_labels, projection)),
                        mode,
                    );
                    if let Some(toggle) = toggle {
                        canvas_toggle = toggle.segments;
                        picks.projection = toggle.picked;
                    }
                    draw_chart_pane(
                        ui,
                        body,
                        charts,
                        ws,
                        projections[projection].item,
                        mode,
                        focused,
                        &headed,
                        &mut requests,
                        affordances,
                    );
                }
            });
            regions.push((canvas.id, drawn.response.rect));

            self.regions = regions;
            self.strips = strips;
            self.canvas_toggle = canvas_toggle;
            if let Some(next) = picks.projection {
                self.projection = next;
            }
            // A name picked in a collapsed rail's strip reopens it. Picking a
            // pane you cannot see is a gesture with no result, and the strip
            // stays live while the rail is down precisely so it is a way back
            // in — `the_collapsed_ledger_keeps_its_selector_strip_and_reopens_from_it`.
            if let Some(next) = picks.ledger {
                self.ledger_panel = next;
                self.collapsed.remove(&arrangement::LEDGER_RAIL);
            }
            if let Some(next) = picks.inspector {
                self.inspector_panel = next;
                self.collapsed.remove(&arrangement::INSPECTOR_RAIL);
            }
            for id in picks.collapse {
                if !self.collapsed.remove(&id) {
                    self.collapsed.insert(id);
                }
            }
        }

        self.status_rail_ui(&ctx, graph_on_canvas, &mut requests);

        self.apply(&ctx, graph_on_canvas, requests);

        // The file dialog, after the frame and outside every borrow it took:
        // it blocks the thread on a window this process did not lay out, so it
        // may not run inside the layout pass. A cancelled dialog is not an
        // error and says nothing — the user changed their mind.
        //
        // `dialogs_allowed` is the gate, not an afterthought: every other tier
        // that drives this method has no user in front of it, and a modal
        // raised there hangs until something kills it. The flag is left set so
        // a headless test can still assert the control asked; the door rewrites
        // it on its next frame.
        if self.pick_requested && self.dialogs_allowed {
            self.pick_requested = false;
            if let Some(path) = crate::data_file::pick() {
                self.open_data_file(&ctx, &path.to_string_lossy());
            }
        }
        if graph_on_canvas {
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
        // After the panes have drawn, because the controls rail dispatches a
        // slider's queued value inside its own draw — so this frame's gesture
        // is answered in this frame's banner, not the next one's.
        self.say_interaction_fault();

        // The overlay plane, over everything the frame drew, then the two
        // notification layers over that. All three draw nothing when empty,
        // so a frame with no overlay, no banner and no toast is
        // pixel-identical to one drawn before they existed.
        self.overlay_ui(&ctx, graph_on_canvas);
        self.notifications.show(&ctx);
        self.toasts.show(&ctx);

        self.sweep();
    }

    // -----------------------------------------------------------------------
    // The overlay slot
    // -----------------------------------------------------------------------

    /// Open an overlay if its registry-declared key was pressed this frame.
    ///
    /// The palette opens either way, but the candidate list differs: with the
    /// graph on the canvas, `Altitude::Protocol`'s raw registry scope already
    /// dispatches through the model, so the raw scope IS the candidate list.
    /// With a chart there, most `Altitude::View` verbs have no handler in this
    /// shell yet — the editing bridge that would let `add-mark`,
    /// `set-channel` and the rest apply a `ChartEdit` is not landed — so
    /// [`Self::open_chart_palette`] restricts the list to
    /// [`CHART_PALETTE_VERBS`](crate::overlays::CHART_PALETTE_VERBS): exactly
    /// what [`Self::apply`]'s chart arm dispatches. A palette of rows that
    /// silently no-op would be worse than none. The node jump stays with the
    /// graph — it has no chart equivalent yet — and the help sheet is
    /// read-only and opens anywhere.
    fn overlay_open_keys(&mut self, ctx: &egui::Context, graph_on_canvas: bool) {
        if self.overlay.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        let pressed = |token: Option<&'static str>| token.is_some_and(|t| consume_token(ctx, t));
        if graph_on_canvas && pressed(self.overlay_keys.palette) {
            self.open_palette(Altitude::Protocol);
        } else if !graph_on_canvas && pressed(self.overlay_keys.palette) {
            self.open_chart_palette();
        } else if graph_on_canvas && pressed(self.overlay_keys.jump) {
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

    /// Move focus to the navigator rail, or put it back, if the registry's
    /// `toggle-outline-rail` keystroke is down this frame.
    ///
    /// Gated exactly as [`Self::home_key`] is: no overlay open, no widget
    /// holding the keyboard.
    fn navigator_key(&mut self, ctx: &egui::Context) {
        if self.overlay.is_some() || ctx.egui_wants_keyboard_input() {
            return;
        }
        if self
            .navigator_binding
            .is_some_and(|t| consume_token(ctx, t))
        {
            self.toggle_navigator_focus(ctx);
        }
    }

    /// The navigator rail's round trip: the same verb reaches the protocol
    /// spine and returns focus to where it came from.
    ///
    /// The round trip is the whole of the semantics, and it is what a dock
    /// toggle has that a view switch does not: a glance at the protocol costs
    /// one key each way and never leaves the cursor parked in a rail. Held by
    /// `pressing_the_navigator_toggle_twice_returns_focus`, which breaks if
    /// the second press is treated as a second move rather than as a return.
    ///
    /// "Where it came from" includes **nowhere**: a window nobody has clicked
    /// in has no focused pane, and putting focus back there is a state the
    /// window can be in rather than a missing answer.
    fn toggle_navigator_focus(&mut self, ctx: &egui::Context) {
        let rail = PaneKey::new(OUTLINE);
        if self.ws().focus() == Some(rail) {
            let back = self.focus_return.take();
            self.ws_mut().clear_focus();
            if let Some(key) = back {
                self.ws_mut().set_focus(key);
            }
        } else {
            self.focus_return = self.focused_pane();
            // One record for the window, so moving it is a move rather than a
            // move plus a tidy-up: both documents' panes are drawn in the same
            // frame, and two live records would mean two panes wearing the
            // focus ring.
            self.ws_mut().set_focus(rail);
        }
        ctx.request_repaint();
    }

    /// Perform whichever navigation verb's key is down this frame.
    ///
    /// Gated exactly as [`Self::home_key`] is — no overlay open, no widget
    /// holding the keyboard — plus the chart being the thing on the canvas,
    /// because a frame verb over the asset graph has no frame to move and its
    /// bare keys would shadow the graph's own grammar.
    fn navigation_keys(&mut self, ctx: &egui::Context, graph_on_canvas: bool) {
        if graph_on_canvas || self.overlay.is_some() || ctx.egui_wants_keyboard_input() {
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

    /// Open the command palette at the chart altitude, restricted to
    /// [`crate::overlays::CHART_PALETTE_VERBS`] — see [`Self::overlay_open_keys`]
    /// for why the chart view cannot simply reuse [`Self::open_palette`] with
    /// [`Altitude::View`].
    fn open_chart_palette(&mut self) {
        self.overlay = Some(Overlay::Palette(Picker::new(
            CommandPalette::new_restricted(
                Altitude::View,
                self.recency.clone(),
                crate::overlays::CHART_PALETTE_VERBS,
            ),
        )));
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
    fn overlay_ui(&mut self, ctx: &egui::Context, graph_on_canvas: bool) {
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
                            self.apply(ctx, graph_on_canvas, vec![Request::Verb(Verb::new(name))]);
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
    /// [`MeridianApp::region_rect`].
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

    /// The title band: the way back to the front door, the subject, and what
    /// this is being rendered by.
    ///
    /// Returns what the band's controls were asked to do rather than doing it:
    /// this runs inside the band's own panel closure, and acting on a control
    /// mid-frame would leave the regions below drawing a state the band above
    /// has already stopped describing.
    ///
    /// **No view switcher.** The pair of plain `selectable_label`s this band
    /// carried modelled the protocol as a peer of the chart, and the protocol
    /// is the container the chart sits inside — it is the navigator rail, and
    /// its toggle is `toggle-outline-rail` off the keyboard registry rather
    /// than a control invented here. There is nothing left for such a control
    /// to switch: one window, one tile tree, one arrangement.
    ///
    /// **The right-hand group is dropped rather than allowed to spill.** A
    /// right-to-left layout draws from the window's right edge leftwards and
    /// does not stop at the cursor the left-hand content left behind, so on a
    /// narrow window the renderer line lands on top of the Home button. egui
    /// gives a click to the last widget drawn over a point, so Home would go
    /// on drawing, go on recording a rect, and stop working.
    /// [`right_group_width`] asks what the group needs before any of it is
    /// drawn.
    ///
    /// [`Verb`]: brightfield_workbench::Verb
    fn title_band(&mut self, ui: &mut egui::Ui) -> TopBar {
        let sem = semantic(self.mode.is_dark());
        let graph_on_canvas = self.graph_on_canvas();
        // Read everything the band says before drawing it, so the closure
        // below borrows no more of `self` than the state it writes.
        let title = self.title();
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
            ui.label(
                egui::RichText::new("Meridian")
                    .font(ui_font())
                    .color(chrome::colour(sem.text.secondary)),
            );
            if !door {
                let home = ui.button(egui::RichText::new("Home").font(ui_font()));
                bar.home_rect = Some(home.rect);
                if home.clicked() {
                    bar.home = true;
                }
            }
            ui.label(egui::RichText::new(title).color(chrome::colour(sem.text.primary)));
            // The renderer line is a developer diagnostic — a stranger's first
            // launch should not read "egui · Vello · wgpu 29". It appears only
            // under the devtools flag; the flow toggle is a real affordance and
            // always draws when the protocol view supplies one.
            let renderer =
                crate::devtools::enabled().then(|| format!("egui · Vello · wgpu 29  —  {theme}"));
            let toggle = graph_on_canvas.then(|| match flow {
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
    /// - the status lines of **every pane reading the document the canvas
    ///   holds**, as declared on its own [`Subject`] — minus its activity
    ///   reports. Not only the focused pane's: a window nobody has clicked in
    ///   still has to say what its panes declare, or every honesty affordance
    ///   living on the rail (a navigation refusal, an unrescoped-mark notice)
    ///   is declared and never seen until the user happens to click inside the
    ///   pane that raised it — the test
    ///   `a_panes_notice_reaches_the_rail_before_anything_is_focused` holds
    ///   this;
    /// - **one** activity indicator, composed from *every* pane's subject
    ///   across *both* documents — in-flight work anywhere in the window is
    ///   the window's to report, and two panes querying at once say
    ///   "querying…" once;
    /// - when neither of those has anything to say and a chart is open, the
    ///   idle line [`idle_status_entry`] composes from the loaded dashboard —
    ///   so an idle window with a chart open is never a silent rail. Live
    ///   activity always wins the slot: the branch below only runs when the
    ///   indicator composed to `None` — the test
    ///   `an_idle_chart_window_names_what_it_loaded` holds the idle line,
    ///   and `activity_reaches_the_rail_as_the_one_indicator` holds
    ///   precedence.
    ///
    /// Dismissal verbs the rail's entries declare are routed into the same
    /// request queue as pane requests — the rail can offer nothing a pane
    /// could not.
    fn status_rail_ui(
        &mut self,
        ctx: &egui::Context,
        graph_on_canvas: bool,
        requests: &mut Vec<Request>,
    ) {
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
        for key in self.ws().panes() {
            let subject = if graph_on_canvas {
                self.protocol
                    .items
                    .get(&key)
                    .map(|item| item.subject(&self.protocol.doc))
            } else {
                self.charts
                    .items
                    .get(&key)
                    .map(|item| item.subject(&self.charts.doc))
            };
            if let Some(subject) = subject {
                // Every placed pane's own lines, focused or not. Typed
                // activity entries are filtered out here because the
                // indicator below says them — once, merged with everyone
                // else's, never per-pane and never twice.
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
        } else if entries.is_empty() && !graph_on_canvas {
            if let Some(idle) = idle_status_entry(&self.charts.doc.composed) {
                entries.push(idle);
            }
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
    /// A verb goes to the model of the document the canvas holds. Both
    /// documents' panes draw in the same frame, so this is a routing choice
    /// rather than a deduction: the canvas is the region a verb typed with no
    /// pane focused is most plausibly aimed at, and `ProtocolModel::dispatch`
    /// silently no-ops a verb it does not know, so a verb sent to the wrong
    /// one would vanish rather than misfire. The chart side has no model; its
    /// verbs act on the document directly, and the branch is spelled out so
    /// that a chart control declaring a verb it does not handle is a change to
    /// *this* line rather than a button that silently does nothing.
    ///
    /// Two verbs are the **window's** rather than either document's and are
    /// intercepted above the branch — see the comments on each.
    fn apply(&mut self, ctx: &egui::Context, graph_on_canvas: bool, requests: Vec<Request>) {
        for request in requests {
            match request {
                // open-home is the window's verb, not either document's, so
                // it is handled before the branch rather than inside it. The
                // chart arm string-matches only clear-selection, and the graph
                // arm forwards to `model.dispatch`, which would silently no-op
                // an unknown verb — so without this intercept the verb would
                // reach neither handler.
                Request::Verb(verb) if verb.as_str() == "open-home" => {
                    self.open_home(ctx);
                }
                // The navigator rail's toggle spans both documents for the
                // same reason open-home does — it is the window's verb, not
                // one document's — so it is intercepted here rather than
                // inside the branch, which would send it to a model that
                // silently no-ops an unknown verb.
                Request::Verb(verb) if verb.as_str() == NAVIGATOR_TOGGLE => {
                    self.toggle_navigator_focus(ctx);
                }
                Request::Verb(verb) if graph_on_canvas => {
                    self.protocol.doc.model.dispatch(verb.as_str());
                }
                Request::Verb(verb) => {
                    if verb.as_str() == "clear-selection" {
                        self.charts.doc.clear_selection();
                        ctx.request_repaint();
                    } else if navigation_verb(&mut self.charts.doc, verb.as_str()) {
                        ctx.request_repaint();
                    }
                }
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
    ///
    /// Nothing else has to be reset, and that is the point of deriving what
    /// the canvas holds rather than recording it: emptying the two documents
    /// *is* the state change. A recorded active view used to survive this call
    /// and leave `title_band` drawing the protocol's flow toggle for the
    /// `ProtocolModel` that had just been emptied, so a click on it dispatched
    /// `toggle_flow` to a document with no assets left. Held by
    /// `going_home_from_a_protocol_leaves_no_protocol_control_behind`.
    fn open_home(&mut self, ctx: &egui::Context) {
        if self.front_door_is_live() {
            return;
        }
        self.open_chart(Composed::empty());
        self.protocol.doc.open(ProtocolInputs::empty());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
        ctx.request_repaint();
    }

    /// What **this window** calls itself — the noun a Protocols row draws, and
    /// the same string [`MeridianApp::title`] builds the window title from,
    /// without its `Protocol · ` prefix.
    fn subject_name(&self) -> String {
        if self.graph_on_canvas() {
            self.protocol.doc.model.protocol.clone()
        } else {
            self.charts.doc.title().to_string()
        }
    }

    /// The run state **this window** was last seen in — what a Protocols row
    /// draws beside the name.
    ///
    /// The window's, not the opened start's, and that is the difference that
    /// stops the front door's Protocols heading lying. A window is one
    /// Protocol whose panes may be a chart, a grid or the asset graph, so the
    /// run state of the row it leaves behind is the run state of that
    /// Protocol: whatever the run contract behind this window recorded, folded
    /// by [`ProtocolModel::recorded_run_state`]. Reading it off the start
    /// instead answered [`RunState::NeverRun`] whenever a chart start opened
    /// it — including a chart opened into a window whose protocol *had*
    /// run, where the row then contradicted the window it came from.
    ///
    /// A window with no protocol behind it folds to
    /// [`RunState::NeverRun`] anyway, which is the honest answer for a
    /// Protocol that has not been run: a chart's tables are materialised by the
    /// engine as it composes and no run contract is written, so there is no
    /// run to report, and reporting one would be the shell computing what the
    /// engine is the thing that computes.
    fn recorded_run_state(&self) -> RunState {
        self.protocol.doc.model.recorded_run_state()
    }

    /// Open the shipped starting point `id` into the document it fills.
    ///
    /// **The second click.** It has to land on a rendered result, not on an
    /// instrument: the document is replaced with a *composed* dashboard or a
    /// *built* graph and its canvas is invalidated so the next frame rasters
    /// the new one. What the window then shows follows from the documents —
    /// see [`graph_takes_the_canvas`]. There is no editor in between, no path
    /// to type and no buffer to fill.
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
    ///
    /// What it records beside the id — the name and the run state — is read
    /// off the document this call just opened, through
    /// [`Self::subject_name`] and [`Self::recorded_run_state`], and
    /// `what_open_start_records_is_what_the_opened_document_says` reads both
    /// back against the opened document itself.
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
        match opened {
            // A new chart document is a new set of things to say, and a
            // reason to stop saying the last one's.
            // `open_chart` drops whatever session was behind the outgoing
            // document; this one's goes on straight after, as the file-open
            // path does at [`MeridianApp::open_data_file`].
            crate::starts::Opened::Charts(chart) => {
                self.open_chart(chart.composed);
                self.charts.doc.attach_live(chart.live);
            }
            crate::starts::Opened::Protocol(inputs) => self.protocol.doc.open(*inputs),
        }
        self.notifications.dismiss(banner);
        self.layout.live_mut().opened = Some(id.to_string());
        // …and remembered as a Protocol, which is the other half: `opened` is
        // what the next launch restores, and this is what the door lists. The
        // name and the run state are read off **this window** rather than off
        // the start — a start's label is a verb for a button, its run state is
        // not a property of a start at all, and the row the door draws is a
        // row for the Protocol the window is.
        let name = self.subject_name();
        let run = self.recorded_run_state();
        self.layout.live_mut().remember(id, &name, run, now_secs());
        self.toasts.push(Toast::new(
            Severity::Success,
            format!("Opened {}", self.title()),
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));
        ctx.request_repaint();
    }

    /// Take the charts half of `boot` into a window that already exists.
    ///
    /// The counterpart of what [`MeridianApp::with_layout`] does while building
    /// one, in the same order and with the same fields — a boot is a document
    /// plus what travels with it, and which of the two constructors receives it
    /// is not the document's business. Keeping the sequence in one place is
    /// what stops a field being added to a boot and wired into only the
    /// constructor whose test happened to be written.
    ///
    /// [`MeridianApp::open_chart`] leads, because it is the different-document
    /// entry: it clears the session, the spec path, the authored record and
    /// everything else that belonged to the outgoing document, so each of them
    /// has to be put back **after** it rather than before.
    fn adopt_chart_boot(&mut self, boot: Boot) {
        self.open_chart(boot.composed);
        if let Some(live) = boot.live {
            self.charts.doc.attach_live(live);
        }
        // Where the editor pane opens the document — for a generated dashboard,
        // the scratch file holding the bytes that composed it. The same field a
        // dashboard composed from a spec someone wrote carries, so the pane
        // needs no arm of its own for a dashboard nobody wrote.
        self.charts.doc.spec_path = boot.spec_path;
        self.charts.doc.wire_watch();
        if let Some(authored) = boot.authored {
            self.charts.doc.set_authored(authored);
        }
    }

    /// Open the data file at `chosen` into the chart document: the file as a
    /// live DuckDB view, the dashboard generated over it, and the Data pane
    /// beside the chart reading the file's own rows back.
    ///
    /// **Public, and taking a string rather than a dialog result.** The dialog
    /// is one line in [`crate::data_file`] and a headless test may not open
    /// one; every decision worth gating — what is refused, what is drawn, what
    /// a failure says — is on this side of that line, so this is the entry
    /// point a test drives and the entry point the door's button reaches
    /// through.
    ///
    /// A failure raises a banner and changes nothing else. That is the whole of
    /// the contract: the window stays up with whatever was on it, and the
    /// banner carries the path and the engine's own words — a file that will
    /// not read must never present as an empty window. The banner id is fixed
    /// rather than composite over the path, so a second attempt replaces the
    /// first attempt's message instead of stacking a history of what did not
    /// open.
    ///
    /// Nothing is recorded in the layout, deliberately: `SavedLayout::opened`
    /// holds a **start id**, and an id cannot name a file that has since been
    /// deleted or moved. Reopening files across launches is a recent-files
    /// list, which is its own piece of work.
    pub fn open_data_file(&mut self, ctx: &egui::Context, chosen: &str) {
        let banner = NotificationId::new("open-data-file");
        let boot = match crate::data_file::open(chosen) {
            Ok(opened) => Boot::of_opened_file(opened),
            Err(e) => {
                eprintln!("could not open {chosen}: {e}");
                self.notifications.raise(
                    Notification::new(banner, Severity::Error, "Could not open that data file")
                        .body(e),
                );
                ctx.request_repaint();
                return;
            }
        };
        self.adopt_chart_boot(boot);
        self.notifications.dismiss(banner);
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
    /// **Welcome**, the **Start** verb spine, and then two sections:
    /// [`DATASETS_SECTION`] — the curated starts this build ships — and
    /// [`PROTOCOLS_SECTION`] — the analyst's own recent work, drawn from
    /// [`SavedLayout::recents`]. The greeting never flips; the morph is in
    /// which section leads and what is under the Protocols heading. There is
    /// no dismissal anywhere because there is nothing to dismiss: content
    /// outcompetes the door, and the door comes back only when the window is
    /// emptied.
    ///
    /// **The empty half is the one that matters.** A first launch has no
    /// recents, which is the blank state the front door exists to prevent, so
    /// the section is drawn *present and speaking* rather than omitted —
    /// [`PROTOCOLS_EMPTY_TITLE`] and [`PROTOCOLS_EMPTY_BODY`]. A section that
    /// appears once it has content and not before teaches a stranger nothing
    /// about what the product saves.
    /// `a_first_run_populates_datasets_and_states_what_protocols_will_hold`
    /// is what holds the heading and both sentences on a first-run frame.
    ///
    /// Everything a click does here goes out through `requests` as the same
    /// [`Request::Open`] an empty pane's button raises, into the same
    /// [`MeridianApp::open_start`]. The door owns no route of its own, and
    /// that is what makes a Protocols row and the Datasets card for the same
    /// subject leave the window in the same state rather than in two states a
    /// test can tell apart.
    ///
    /// [`SavedLayout::recents`]: brightfield_workbench::SavedLayout::recents
    fn front_door_ui(&mut self, ui: &mut egui::Ui, requests: &mut Vec<Request>) {
        self.door_cards.clear();
        self.door_sections.clear();
        self.door_rows.clear();
        self.door_help = None;
        self.door_open_file = None;
        self.affordances.clear();
        self.ensure_door_thumbs(ui.ctx());

        let sem = semantic(self.mode.is_dark());
        let help_key = self.overlay_keys.help;
        // The rows this door will draw, read out of the layout before the
        // closure below borrows `self` mutably — and filtered to what this
        // build can reopen. A recent naming a start this binary no longer
        // ships would draw a row whose click does nothing, which is worse
        // than a shorter list.
        let recents: Vec<Recent> = self
            .layout
            .live()
            .recents
            .iter()
            .filter(|r| crate::starts::find(&r.id).is_some())
            .cloned()
            .collect();
        let now = now_secs();
        let mut open_help = false;
        let mut open_file = false;

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

                        // Welcome — invariant, whatever the sections below do.
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
                        //
                        // **Open leads it**, and it is the one verb here that
                        // does not open something this binary already carries.
                        // Everything else on this door — every Datasets card,
                        // every Protocols row — resolves to a start compiled
                        // into the executable, which is a fine first click and
                        // a poor second one: the product's own promise is that
                        // you open a file and the picture is already there.
                        // This control is the pointing route to that promise
                        // and `names_a_data_file` is the typed one; both end in
                        // `data_file::open` and neither is the other's
                        // shorthand. It is deliberately NOT a `Request::Open`,
                        // because that carries a `&'static str` start id and a
                        // file the user chose is neither static nor a start.
                        door_zone_heading(ui, "Start", sem);
                        let open =
                            ui.button(egui::RichText::new("Open a data file…").font(ui_font()));
                        self.door_open_file = Some(open.rect);
                        if open.clicked() {
                            open_file = true;
                        }
                        ui.add_space(spacing::SPACE_2);
                        ui.label(
                            egui::RichText::new(OPEN_FILE_PROMISE)
                                .font(ui_font())
                                .color(chrome::colour(sem.text.secondary)),
                        );
                        ui.add_space(spacing::CONTROL_GAP);
                        let help_label = match help_key {
                            Some(k) => format!("Keyboard help  {k}"),
                            None => "Keyboard help".to_string(),
                        };
                        let help = ui.button(egui::RichText::new(help_label).font(ui_font()));
                        self.door_help = Some(help.rect);
                        if help.clicked() {
                            open_help = true;
                        }

                        // The morph, and the whole of it: which section leads.
                        // A returning analyst's own work is what they came
                        // for, so it goes above the catalogue; a first run has
                        // nothing to lead with, so the catalogue carries the
                        // door and Protocols states what it will hold. The
                        // greeting above is the same either way: it does not
                        // flip to "Welcome back", which is a deliberate
                        // departure from the Zed precedent the rest of this
                        // door follows.
                        if recents.is_empty() {
                            self.datasets_section(ui, width, sem, requests);
                            self.protocols_section(ui, width, sem, &recents, now, requests);
                        } else {
                            self.protocols_section(ui, width, sem, &recents, now, requests);
                            self.datasets_section(ui, width, sem, requests);
                        }
                        ui.add_space(spacing::SECTION_GAP);
                    });
                });
            });

        if open_help {
            self.overlay = Some(Overlay::Help(Picker::new(HelpSheet::new())));
        }
        // Latched, not acted on: the dialog blocks on the operating system and
        // the `Ui` borrow is still live here. `draw` takes it after the frame.
        self.pick_requested = open_file;
    }

    /// The Datasets section: the curated starts, as a wrapping row of cards.
    ///
    /// A card carries [`DOOR_ENTRY_PROMISE`] under its summary, which
    /// `a_first_run_populates_datasets_and_states_what_protocols_will_hold`
    /// reads back off a rendered first-run frame.
    fn datasets_section(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        sem: &semantic::Semantic,
        requests: &mut Vec<Request>,
    ) {
        self.door_section_heading(ui, DATASETS_SECTION, DATASETS_NOTE, width, sem);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(spacing::SECTION_GAP, spacing::SECTION_GAP);
            for start in crate::starts::STARTS {
                self.door_card(ui, start, sem, requests);
            }
        });
    }

    /// The Protocols section: the analyst's own recent work, most recent
    /// first — or, on a first run, what it will hold.
    ///
    /// The empty arm is not a fallback. It is the state this section is in on
    /// every first launch of every install, so it is drawn as deliberately as
    /// the populated one, and the section's heading is present in both — a
    /// Protocols heading that is absent until there is something under it is
    /// the failure this arm exists to make impossible.
    fn protocols_section(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        sem: &semantic::Semantic,
        recents: &[Recent],
        now: u64,
        requests: &mut Vec<Request>,
    ) {
        let note = if recents.is_empty() {
            PROTOCOLS_NOTE_EMPTY
        } else {
            PROTOCOLS_NOTE
        };
        self.door_section_heading(ui, PROTOCOLS_SECTION, note, width, sem);
        if recents.is_empty() {
            door_empty_protocols(ui, width, sem);
            return;
        }
        for recent in recents {
            self.door_row(ui, width, recent, now, sem, requests);
        }
    }

    /// One section heading: the noun, what it says about itself beside it, and
    /// a rule across the column.
    ///
    /// Records the heading's rect and its position in the draw order, which is
    /// what [`MeridianApp::front_door_sections`] answers — so *which sections
    /// the door drew, and in what order* is read off the frame rather than
    /// recomputed from the layout the frame was drawn from.
    fn door_section_heading(
        &mut self,
        ui: &mut egui::Ui,
        name: &'static str,
        note: &str,
        width: f32,
        sem: &semantic::Semantic,
    ) {
        ui.add_space(spacing::SECTION_GAP);
        let heading = ui
            .horizontal(|ui| {
                let label = ui.label(
                    egui::RichText::new(name)
                        .font(ui_font())
                        .color(chrome::colour(sem.text.primary)),
                );
                ui.add_space(spacing::SPACE_3);
                ui.label(
                    egui::RichText::new(note)
                        .text_style(egui::TextStyle::Small)
                        .color(chrome::colour(sem.text.muted)),
                );
                label.rect
            })
            .inner;
        self.door_sections.push((name, heading));
        ui.add_space(spacing::SPACE_2);
        let (rule, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
        ui.painter().hline(
            rule.x_range(),
            rule.center().y,
            (1.0, chrome::colour(sem.borders.subtle)),
        );
        ui.add_space(spacing::SPACE_4);
    }

    /// One Protocols row: the Protocol's name, the run state it was last seen
    /// in, and when that was — the whole row clickable, raising the same
    /// [`Request::Open`] its Datasets card raises.
    fn door_row(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        recent: &Recent,
        now: u64,
        sem: &semantic::Semantic,
        requests: &mut Vec<Request>,
    ) {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, PROTOCOL_ROW_HEIGHT), egui::Sense::click());
        // Laid out ahead of the visibility test rather than inside it, because
        // these three galleys are what the row records: a row scrolled past
        // the bottom of the column still has to answer
        // [`MeridianApp::front_door_rows`] with the text it put on the screen,
        // and a galley built inside the branch would leave that answer to
        // whichever field the push below happened to name. egui keys its
        // galley cache on the layout job, so a row nobody sees costs a lookup
        // rather than a fresh shaping on the frames after the first.
        let tone = chrome::tone_colour(recent.run.tone(), self.mode);
        let name = ui.painter().layout(
            recent.name.clone(),
            ui_font(),
            chrome::colour(sem.text.primary),
            PROTOCOL_ROW_NAME_WIDTH - spacing::SPACE_4,
        );
        // The run state in the reader's own comparison face, and in
        // `RunState::label`'s words rather than a second vocabulary coined
        // here — `a_door_with_recents_lists_every_one_of_them_most_recent_first`
        // compares this string against the enum's own label.
        let state = ui.painter().layout(
            recent.run.label().to_string(),
            mono_font(),
            tone,
            PROTOCOL_ROW_NAME_WIDTH,
        );
        let when = ui.painter().layout_no_wrap(
            relative_time(now, recent.opened_at),
            mono_font(),
            chrome::colour(sem.text.muted),
        );
        // Off the galleys, not off `recent` — which is what makes [`DoorRow`]'s
        // claim about itself true. The strings a test reads back are the ones
        // handed to the painter at these three positions, so painting one
        // field and recording another is a difference the read-back can see.
        self.door_rows.push(DoorRow {
            id: recent.id.clone(),
            name: name.text().to_string(),
            state: state.text().to_string(),
            when: when.text().to_string(),
            rect,
        });
        if ui.is_rect_visible(rect) {
            let painter = ui.painter().with_clip_rect(rect);
            if response.hovered() {
                painter.rect_filled(rect, radius::CONTROL, chrome::colour(sem.surfaces.raised));
            }
            let mid = rect.center().y;
            painter.galley(
                egui::pos2(rect.min.x + spacing::SPACE_4, mid - name.size().y / 2.0),
                name,
                chrome::colour(sem.text.primary),
            );
            painter.galley(
                egui::pos2(
                    rect.min.x + PROTOCOL_ROW_NAME_WIDTH,
                    mid - state.size().y / 2.0,
                ),
                state,
                tone,
            );
            painter.galley(
                egui::pos2(
                    rect.max.x - spacing::SPACE_4 - when.size().x,
                    mid - when.size().y / 2.0,
                ),
                when,
                chrome::colour(sem.text.muted),
            );
        }
        if response.clicked() {
            // Leaked into the request queue as a `&'static str`, which is what
            // `Request::Open` carries: the id came off a shipped start (the
            // filter in `front_door_ui` drops a recent that does not resolve),
            // so this raises the same static the card beside it raises —
            // `either_route_to_the_same_subject_leaves_the_same_window` walks
            // the shipped set and compares the two windows.
            if let Some(start) = crate::starts::find(&recent.id) {
                requests.push(Request::Open(start.id));
            }
        }
    }

    /// One gallery card: the start's pre-rendered thumbnail, its label and
    /// its one-line summary, the whole card clickable.
    ///
    /// A card is also recorded under the pane its start fills in
    /// [`Self::affordances`] — on a door frame it *is* where the way in that
    /// fills that pane was drawn, so [`MeridianApp::affordance_rect`] keeps
    /// one answer across both arrangements of the same affordance.
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
            // The promise, at the card's foot: one constant, shared with the
            // Protocols section, so a reader comparing the two sections is
            // comparing one sentence with itself.
            let promise = painter.layout_no_wrap(
                DOOR_ENTRY_PROMISE.to_string(),
                egui::FontId::monospace(meridian_design::typography::CHART_LABEL_SIZE),
                chrome::colour(sem.text.muted),
            );
            painter.galley(
                egui::pos2(text_left, rect.max.y - spacing::SPACE_4 - promise.size().y),
                promise,
                chrome::colour(sem.text.muted),
            );
        }

        self.door_cards.push((start.id, rect));
        // The card is recorded under the pane it fills when it is the one that
        // pane's own empty state would offer — `for_pane` hands an empty
        // pane the FIRST start that fills it, so a second start for the same
        // pane is on the gallery and not on that button.
        if crate::starts::for_pane(start.fills).map(|s| s.id) == Some(start.id) {
            self.affordances.push((PaneKey::new(start.fills), rect));
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
    ///
    /// Picks each start's thumbnail for `self.mode` — see
    /// [`crate::starts::Start::thumbnail_for`] — rather than the light one
    /// unconditionally. Safe to do once, here: `mode` is a private field
    /// [`MeridianApp::assemble`] writes once, from a `Mode` its four public
    /// constructors all take as a parameter and none reassign afterwards;
    /// this file declares no submodule but `mod tests`; and the workspace's
    /// one `.mode = …` assignment outside test code is
    /// [`LiveDashboard::set_mode`](crate::pipeline::LiveDashboard::set_mode),
    /// on a different struct entirely. So there is no later mode change this
    /// cache could go stale against.
    fn ensure_door_thumbs(&mut self, ctx: &egui::Context) {
        if !self.door_thumbs.is_empty() {
            return;
        }
        for start in crate::starts::STARTS {
            let Ok(decoded) = image::load_from_memory(start.thumbnail_for(self.mode)) else {
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
        let tree = self.ws_mut().tree_mut();
        let Some(canvas) = tile_of(tree, PaneKey::new(PROTOCOL_CANVAS)) else {
            return;
        };
        let Some(steps) = tile_of(tree, PaneKey::new(STEPS)) else {
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
        if let Some(show) = steps_tab_is_active(self.ws().tree()) {
            self.protocol.doc.model.set_show_sheet(show);
        }
    }

    /// Declare which panes this frame laid out, so each host can free the canvas
    /// slot of any pane that has gone.
    ///
    /// **Every pane in the window, every frame**, handed to both documents —
    /// including panes the other document owns, and including a canvas tabbed
    /// out of sight. `end_frame` frees any slot that neither presented this
    /// frame nor appears in the set it is handed, while `present` caches its
    /// texture id across frames and returns early on an unchanged key. Naming
    /// only the panes of the document on the canvas would therefore free the
    /// other's texture and leave a dangling id the instant the canvas changed
    /// hands. A superset frees nothing it should not: a document has no slot
    /// filed under another's pane key, so the extra names match nothing. The
    /// failure in the other direction is a leak, which is the safe one to be
    /// wrong in.
    fn sweep(&mut self) {
        let panes: BTreeSet<PaneKey> = self.ws().panes().into_iter().collect();
        self.charts.doc.sweep(&panes);
        self.protocol.doc.sweep(&panes);
    }
}

/// Whether the window's tab strip has the steps sheet in front, or `None`
/// when this tree has no such strip.
///
/// The one reader of the strip, so the restore path and the click-back path
/// cannot disagree about which tab means what:
/// [`MeridianApp::assemble`] seeds the model from it before the first frame,
/// and [`MeridianApp::read_active_tab`] reads a user's click out of it after
/// each one.
fn steps_tab_is_active(tree: &egui_tiles::Tree<PaneKey>) -> Option<bool> {
    let steps = tile_of(tree, PaneKey::new(STEPS))?;
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

/// The stable id [`idle_status_entry`] writes, and the one
/// [`status_rail_ui`](MeridianApp::status_rail_ui) reads back for the test
/// that pins it — distinct from the rail's other declared ids, so a rail
/// carrying this entry is unambiguous about what it is.
const IDLE_STATUS_ID: &str = "chart-idle";

/// What a settled chart window says about itself when nothing is running and
/// no pane has anything more specific to report — the answer to *"is the app
/// finished thinking, and what did it load?"*, said once the rail would
/// otherwise be silent.
///
/// Built from `Composed`, the data the window already holds from composing
/// the spec — no live query, no new vocabulary: the same [`StatusEntry`]
/// carrier the rail's other declared lines use, and the same rows a
/// sampled plot already carries in [`crate::pipeline::PlotHandle::sample`].
/// `None` for an empty document (`Composed::empty()`) — the front door's own
/// empty state already says the document is empty, and a second empty-state
/// line here would repeat it in fainter ink.
fn idle_status_entry(composed: &Composed) -> Option<StatusEntry> {
    if composed.plots.is_empty() {
        return None;
    }
    let marks: usize = composed.plots.iter().map(|plot| plot.marks.len()).sum();
    let text = match composed.plots.iter().find_map(|plot| plot.sample.as_ref()) {
        Some(sample) if sample.of > sample.drawn => {
            format!("loaded · {} of {} rows sampled", sample.drawn, sample.of)
        }
        _ => format!("loaded · {}", plural(marks, "mark", "marks")),
    };
    Some(StatusEntry {
        id: IDLE_STATUS_ID,
        side: StatusSide::Trailing,
        text,
        tone: Tone::Neutral,
        hide: HideAffordance::WithRail,
    })
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

/// The empty Protocols section: a bordered box saying what will fill it.
///
/// A box rather than two bare lines, because the section has to read as
/// *present and empty* rather than as the page having stopped — the whole
/// point of drawing the heading on a first run at all.
fn door_empty_protocols(ui: &mut egui::Ui, width: f32, sem: &semantic::Semantic) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(width, EMPTY_PROTOCOLS_HEIGHT),
        egui::Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_stroke(
        rect,
        radius::CONTROL,
        egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
        egui::StrokeKind::Inside,
    );
    let left = rect.min.x + spacing::SPACE_5;
    let wrap = width - 2.0 * spacing::SPACE_5;
    let title = painter.layout(
        PROTOCOLS_EMPTY_TITLE.to_string(),
        ui_font(),
        chrome::colour(sem.text.primary),
        wrap,
    );
    let title_pos = egui::pos2(left, rect.min.y + spacing::SPACE_5);
    painter.galley(title_pos, title.clone(), chrome::colour(sem.text.primary));
    let body = painter.layout(
        PROTOCOLS_EMPTY_BODY.to_string(),
        egui::TextStyle::Small.resolve(ui.style()),
        chrome::colour(sem.text.secondary),
        wrap,
    );
    painter.galley(
        egui::pos2(left, title_pos.y + title.size().y + spacing::SPACE_3),
        body,
        chrome::colour(sem.text.secondary),
    );
}

/// The empty Protocols box's height: two lines and the padding around them,
/// stated as the arithmetic rather than as a tuned number so a change to
/// either term is a visible edit here.
const EMPTY_PROTOCOLS_HEIGHT: f32 = 2.0 * spacing::SPACE_5 + 2.0 * spacing::ROW_GRID;

/// Whole seconds since the Unix epoch, as [`Recent::opened_at`] records them.
///
/// A clock set before 1970 reads as the epoch rather than panicking: a wrong
/// clock is a row that says *just now*, which is a cosmetic wrong answer, and
/// a window that will not draw its own front door is not.
///
/// [`Recent::opened_at`]: brightfield_workbench::Recent::opened_at
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// How long ago `then` was, from `now`, in the words a Protocols row draws:
/// *just now*, *17m ago*, *5h ago*, *yesterday*, *4 days ago*, *3 weeks ago*,
/// *2 months ago*.
///
/// **`now` is a parameter, not a call.** A relative time computed from the
/// process clock inside the draw path cannot be pinned by a committed
/// baseline, and a golden that photographs a moving string is a gate that
/// flaps. The door reads [`now_secs`] once per frame and hands it down; a
/// test hands down whatever it likes.
///
/// A `then` in the future — a clock that went backwards between two launches,
/// which is ordinary on a machine that has just synchronised — saturates to
/// *just now* rather than underflowing.
///
/// The buckets are exclusive upper bounds in seconds: 60, 3,600, 86,400, then
/// two days, seven days, thirty days. `the_relative_time_says_what_it_means`
/// walks every boundary either side.
fn relative_time(now: u64, then: u64) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    const MONTH: u64 = 30 * DAY;
    let ago = now.saturating_sub(then);
    match ago {
        _ if ago < MINUTE => "just now".to_string(),
        _ if ago < HOUR => format!("{}m ago", ago / MINUTE),
        _ if ago < DAY => format!("{}h ago", ago / HOUR),
        _ if ago < 2 * DAY => "yesterday".to_string(),
        _ if ago < WEEK => format!("{} days ago", ago / DAY),
        _ if ago < MONTH => format!("{} ago", plural((ago / WEEK) as usize, "week", "weeks")),
        _ => format!("{} ago", plural((ago / MONTH) as usize, "month", "months")),
    }
}

/// The verb that reaches the navigator rail and returns from it.
///
/// The registry's longname, said once here rather than spelled at each of the
/// three sites that need it — the boot lookup, the request intercept and the
/// test that presses it.
pub const NAVIGATOR_TOGGLE: &str = "toggle-outline-rail";

// ---------------------------------------------------------------------------
// Reading the arrangement
// ---------------------------------------------------------------------------

/// The extent a band was declared at.
///
/// # Panics
///
/// If the region is not a band. A region the draw path lays out as a fixed
/// band while the arrangement calls it something else is a structural mistake,
/// and honouring it silently is how the two answers drift apart.
fn band_extent(region: &Region) -> f32 {
    match region.extent {
        arrangement::Extent::Band(size) => size,
        other => panic!("{} is drawn as a band but declared {other:?}", region.id),
    }
}

/// What a rail opens at.
///
/// # Panics
///
/// If the region is not a rail — as [`band_extent`].
fn rail_default(region: &Region) -> f32 {
    match region.extent {
        arrangement::Extent::Rail { default, .. } => default,
        other => panic!("{} is drawn as a rail but declared {other:?}", region.id),
    }
}

/// What a rail refuses to narrow past.
///
/// # Panics
///
/// If the region is not a rail — as [`band_extent`].
fn rail_min(region: &Region) -> f32 {
    match region.extent {
        arrangement::Extent::Rail { min, .. } => min,
        other => panic!("{} is drawn as a rail but declared {other:?}", region.id),
    }
}

/// What a rail takes while it is collapsed.
///
/// # Panics
///
/// If the region declares no collapsed extent — as [`band_extent`]. The draw
/// path reaches this for a region the arrangement says can collapse, so a
/// `None` here is the declaration and the draw path disagreeing about which
/// regions those are.
fn rail_collapsed(region: &Region) -> f32 {
    region.collapsed_extent().unwrap_or_else(|| {
        panic!(
            "{} is drawn collapsed but declares no collapsed extent",
            region.id
        )
    })
}

/// Which way a rail's collapse control points: the direction the rail moves
/// when it is clicked, so an open rail's caret points at the edge it folds
/// into and a collapsed one's points back at the room it would take.
///
/// # Panics
///
/// If the region takes no edge. A region that is the remainder has no
/// direction to fold in, and the arrangement's audit refuses a collapsed
/// extent on one — `the_audit_refuses_a_collapsed_extent_on_something_that_is_not_a_rail`.
const fn collapse_caret(edge: arrangement::Edge, collapsed: bool) -> chrome::Caret {
    match (edge, collapsed) {
        (arrangement::Edge::Left, false) | (arrangement::Edge::Right, true) => chrome::Caret::Left,
        (arrangement::Edge::Left, true) | (arrangement::Edge::Right, false) => chrome::Caret::Right,
        (arrangement::Edge::Top, false) | (arrangement::Edge::Bottom, true) => chrome::Caret::Up,
        (arrangement::Edge::Top, true) | (arrangement::Edge::Bottom, false) => chrome::Caret::Down,
        (arrangement::Edge::Centre, _) => {
            panic!("a region with no edge has no direction to fold in")
        }
    }
}

/// The panes a rail was declared to hold.
///
/// # Panics
///
/// If the region holds something other than panes — as [`band_extent`].
fn region_panes(region: &Region) -> &'static [ItemId] {
    match region.occupant {
        Occupant::Panes(panes) => panes,
        other => panic!("{} is drawn as a rail but declared {other:?}", region.id),
    }
}

/// The canvas's declared projections and the graph behind them.
///
/// # Panics
///
/// If the region is not the canvas — as [`band_extent`].
fn canvas_occupants(region: &Region) -> (&'static [Projection], ItemId) {
    match region.occupant {
        Occupant::Canvas { projections, graph } => (projections, graph),
        other => panic!(
            "{} is drawn as the canvas but declared {other:?}",
            region.id
        ),
    }
}

/// The words a rail's selector strip offers its panes under — each pane's own
/// [`Subject`] title, so the strip and the pane cannot say different things
/// about the same pane.
fn pane_labels(labels: &[String]) -> Vec<&str> {
    labels.iter().map(String::as_str).collect()
}

// ---------------------------------------------------------------------------
// Drawing a region's occupant
// ---------------------------------------------------------------------------

/// Draw one pane of the protocol document into `body`, through the same
/// [`PaneChrome`] a dock would have used.
///
/// A no-op for a pane this build's tree does not carry: the tile is what
/// `pane_ui` addresses its focus and its item context by, and a pane with no
/// tile has no address.
#[allow(clippy::too_many_arguments)]
fn draw_protocol_pane(
    ui: &mut egui::Ui,
    body: egui::Rect,
    protocol: &mut ProtocolView,
    ws: &Workspace,
    item: ItemId,
    mode: Mode,
    focused: Option<PaneKey>,
    headed: &std::collections::HashSet<egui_tiles::TileId>,
    requests: &mut Vec<Request>,
    affordances: &mut Vec<(PaneKey, egui::Rect)>,
) {
    let mut key = PaneKey::new(item);
    let Some(tile) = ws.tile_of(key) else {
        return;
    };
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.shrink_clip_rect(body);
    let mut behavior = PaneChrome::new(
        &mut protocol.doc,
        &mut protocol.items,
        mode,
        focused,
        headed,
        requests,
        affordances,
    );
    let _ = behavior.pane_ui(&mut child, tile, &mut key);
}

/// [`draw_protocol_pane`] over the chart document.
#[allow(clippy::too_many_arguments)]
fn draw_chart_pane(
    ui: &mut egui::Ui,
    body: egui::Rect,
    charts: &mut ChartView,
    ws: &Workspace,
    item: ItemId,
    mode: Mode,
    focused: Option<PaneKey>,
    headed: &std::collections::HashSet<egui_tiles::TileId>,
    requests: &mut Vec<Request>,
    affordances: &mut Vec<(PaneKey, egui::Rect)>,
) {
    let mut key = PaneKey::new(item);
    let Some(tile) = ws.tile_of(key) else {
        return;
    };
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.shrink_clip_rect(body);
    let mut behavior = PaneChrome::new(
        &mut charts.doc,
        &mut charts.items,
        mode,
        focused,
        headed,
        requests,
        affordances,
    );
    let _ = behavior.pane_ui(&mut child, tile, &mut key);
}

/// The canvas's head band: what is on the canvas at its left, the one toggle
/// between the step's projections after it, and a rule under the lot.
///
/// `toggle` is `None` on a canvas showing the graph, which has no second
/// reading to offer — the toggle is drawn where there is something to toggle
/// rather than drawn disabled, because a control that cannot act is a control
/// a reader has to work out the state of.
fn canvas_head(
    ui: &mut egui::Ui,
    head: egui::Rect,
    name: &str,
    toggle: Option<(&[&str], usize)>,
    mode: Mode,
) -> Option<chrome::ToggleDrawn> {
    let sem = semantic(mode.is_dark());
    ui.painter()
        .rect_filled(head, radius::NONE, chrome::colour(sem.surfaces.header));
    let galley =
        ui.painter()
            .layout_no_wrap(name.to_owned(), ui_font(), chrome::colour(sem.text.primary));
    let name_width = galley.size().x;
    ui.painter().galley(
        egui::pos2(
            head.left() + spacing::SPACE_4,
            head.center().y - galley.size().y / 2.0,
        ),
        galley,
        chrome::colour(sem.text.primary),
    );
    let drawn = toggle.map(|(labels, active)| {
        chrome::projection_toggle(
            ui,
            egui::pos2(
                head.left() + spacing::SPACE_4 + name_width + spacing::SPACE_6,
                head.center().y,
            ),
            labels,
            active,
            mode,
        )
    });
    ui.painter().line_segment(
        [head.left_bottom(), head.right_bottom()],
        egui::Stroke::new(1.0, chrome::colour(sem.borders.subtle)),
    );
    drawn
}

/// The locator band: where the subject sits, said as a breadcrumb, and where
/// the document came from at its right.
///
/// The right-hand note is what stops this band repeating the title band on a
/// window whose subject has no trail below it: a chart has no drill state, so
/// its crumb line is one entry, and the file it was composed from is the thing
/// a locator can say that the title cannot.
fn locator_band_ui(ui: &egui::Ui, crumbs: &[String], source: Option<&str>, mode: Mode) {
    let sem = semantic(mode.is_dark());
    let rect = ui.max_rect();
    if let Some(source) = source {
        ui.painter().text(
            egui::pos2(rect.right() - spacing::SPACE_4, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            source,
            egui::FontId::monospace(meridian_design::typography::UI_SIZE - 1.0),
            chrome::colour(sem.text.muted),
        );
    }
    let mut x = rect.left() + spacing::SPACE_4;
    for (i, crumb) in crumbs.iter().enumerate() {
        if i > 0 {
            let sep = ui.painter().layout_no_wrap(
                "\u{203a}".to_owned(),
                ui_font(),
                chrome::colour(sem.text.muted),
            );
            let width = sep.size().x;
            ui.painter().galley(
                egui::pos2(x, rect.center().y - sep.size().y / 2.0),
                sep,
                chrome::colour(sem.text.muted),
            );
            x += width + spacing::SPACE_3;
        }
        let ink = if i + 1 == crumbs.len() {
            sem.text.primary
        } else {
            sem.text.secondary
        };
        let galley = ui
            .painter()
            .layout_no_wrap(crumb.clone(), ui_font(), chrome::colour(ink));
        let width = galley.size().x;
        ui.painter().galley(
            egui::pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            chrome::colour(ink),
        );
        x += width + spacing::SPACE_3;
    }
}

/// How wide the top bar's right-hand group would be if it drew: the renderer
/// line when developer diagnostics are on, and the flow toggle when the
/// protocol view supplies one. Either or both may be absent — an absent item
/// counts no width, and a group with nothing in it is not drawn at all.
///
/// Asked *before* the group is drawn, which is the whole point — a
/// right-to-left layout that does not fit overlaps what is already on the bar
/// instead of shrinking, and the thing already on the bar is the only way to
/// reach the other view. See [`MeridianApp::title_band`].
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

    /// Every bucket of a Protocols row's relative time, walked from both
    /// sides of every boundary.
    ///
    /// The boundaries are where this goes wrong: an inclusive `<=` at the day
    /// mark turns *23h ago* into *yesterday* an hour early, and nothing on a
    /// door row makes that visible. The pairs below are the second either
    /// side of each edge, so an off-by-one in any comparison changes exactly
    /// one of them.
    #[test]
    fn the_relative_time_says_what_it_means() {
        const MINUTE: u64 = 60;
        const HOUR: u64 = 60 * MINUTE;
        const DAY: u64 = 24 * HOUR;
        // A fixed "now" well past every bucket, so no case saturates.
        let now = 400 * DAY;
        let cases: [(u64, &str); 16] = [
            (0, "just now"),
            (MINUTE - 1, "just now"),
            (MINUTE, "1m ago"),
            (2 * MINUTE + 30, "2m ago"),
            (HOUR - 1, "59m ago"),
            (HOUR, "1h ago"),
            (DAY - 1, "23h ago"),
            (DAY, "yesterday"),
            (2 * DAY - 1, "yesterday"),
            (2 * DAY, "2 days ago"),
            (7 * DAY - 1, "6 days ago"),
            (7 * DAY, "1 week ago"),
            (14 * DAY, "2 weeks ago"),
            (30 * DAY - 1, "4 weeks ago"),
            (30 * DAY, "1 month ago"),
            (400 * DAY, "13 months ago"),
        ];
        for (ago, said) in cases {
            assert_eq!(
                relative_time(now, now - ago),
                said,
                "{ago}s ago should read {said:?}"
            );
        }
        // A clock that went backwards between two launches — ordinary on a
        // machine that has just synchronised — reads as the present rather
        // than underflowing.
        assert_eq!(relative_time(now, now + 10 * DAY), "just now");
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

    /// The clicked-card path leaves the document able to re-lay-out.
    ///
    /// Driven through `open_start` rather than [`Boot::start`] because they are
    /// two openers: this one replaces the document in a running window, and it
    /// is the one a person reaches by clicking a card on the front door.
    #[test]
    fn a_start_opened_from_the_door_re_lays_out_into_a_box_it_is_handed() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.open_start(&ctx, crate::starts::DASHBOARD);
        let doc = app.chart_doc();
        let declared = (doc.composed.width, doc.composed.height);
        let half = egui::vec2(declared.0 as f32 / 2.0, declared.1 as f32 / 2.0);

        assert!(
            app.chart_doc_mut().reflow_to(half),
            "the start reports no re-layout for a box half its declared size"
        );
        let doc = app.chart_doc();
        assert_ne!(
            (doc.composed.width, doc.composed.height),
            declared,
            "the start held its declared size in a box half that wide"
        );
    }

    /// A protocol opened over a chart leaves the chart on the canvas.
    ///
    /// Reached by two direct `open_start` calls, not by two clicks: no
    /// shipped control opens a second document into a window already holding
    /// one — `Boot::open` takes exactly one spec, and every `Request::Open` a
    /// person can raise is gated behind the receiving document being empty
    /// first. This pins the order that used to trap the code handling that
    /// state regardless: `open_start` records the view its start fills, its
    /// `Opened::Protocol` arm leaves the chart document alone, and the frame
    /// then read `_ => view` for a window holding both — so the canvas went
    /// to the graph and stayed there. Nothing on the window moves it back.
    /// Measured before this test existed: a 12-point grid of clicks over the
    /// whole window, re-settling after each, found no control that returns
    /// the canvas to the chart, and the only exit was Home, which discards
    /// both documents.
    ///
    /// Asserted off a drawn frame rather than off `active()`: the branch that
    /// puts the graph on the canvas draws no toggle, so two segments coming
    /// back are the screen saying which document it gave the canvas to.
    /// Then one of them is clicked, because a canvas that draws the chart and
    /// answers no pointer is the same dead end with a picture of the way out.
    #[test]
    fn a_protocol_opened_over_a_chart_leaves_the_chart_on_the_canvas() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.open_start(&ctx, crate::starts::DASHBOARD);
        app.open_start(&ctx, crate::starts::CROSSWALK);
        assert!(
            !app.chart_doc().is_empty(),
            "opening the protocol emptied the chart document, so this window \
             never reached the state under test"
        );
        assert!(
            app.protocol_doc().model.has_assets(),
            "the protocol start built no assets, so this window never reached \
             the state under test"
        );

        let (w, h) = chart_window_size(&app.chart_doc().composed);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w, h),
            )),
            ..Default::default()
        };
        // Three frames, for the reason `tests/arrangement.rs` gives: a
        // resizable panel's reported size is read back on the frame after.
        for _ in 0..3 {
            let _ = ctx.run_ui(raw.clone(), |ui| app.draw(ui));
        }

        let segments = app.canvas_toggle_segments().to_vec();
        assert_eq!(
            segments.len(),
            2,
            "the canvas drew {} toggle segments over a window holding both \
             documents. The toggle draws where the canvas holds the chart, so \
             none means the graph took the canvas — and there is no control \
             that gives it back",
            segments.len()
        );

        // The window opens on the chart, which is the second projection; the
        // grid is the first. Clicking it is the click a person makes.
        let opened_on = app.projection();
        assert_eq!(opened_on, 1, "the window did not open on the chart");
        let grid = segments[0].center();
        let mut events = vec![egui::Event::PointerMoved(grid)];
        for pressed in [true, false] {
            events.push(egui::Event::PointerButton {
                pos: grid,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        let clicked = egui::RawInput {
            events,
            ..raw.clone()
        };
        let _ = ctx.run_ui(clicked, |ui| app.draw(ui));
        assert_eq!(
            app.projection(),
            0,
            "clicking the grid segment at {grid:?} left the canvas on \
             projection {opened_on}, so the toggle drew over a canvas that \
             does not answer it"
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
