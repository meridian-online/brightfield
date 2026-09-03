//! The chart view — the composited Vello dashboard, expressed as two
//! [`Item`]s on the workbench shell contract.
//!
//! This file declares the view and nothing around it. The window, the top bar
//! and the frame loop belong to [`crate::window::MeridianApp`], which draws
//! this view and the protocol view from one `eframe::App`; what is here is the
//! document the two panes share, the registry that is the single declaration
//! of the view's shape, and the controls rail. The chart pane itself is
//! [`crate::chart_item::ChartItem`], in its own module — one implementation
//! parameterised by mark kind, with the gesture seam and the margin legend
//! beside it.
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
//! - **No bespoke selection or focus treatment.** The surface *is* selectable
//!   now — a brush is a selection — and the treatment is still nobody's own:
//!   the transient gesture wears the design system's overlay token group and
//!   keyboard focus wears `meridian-egui`'s one ring, both in
//!   [`crate::chart_item`], neither invented here.
//!
//! What this surface *did* lack entirely is an empty state — a spec that
//! composed nothing would have rendered chrome and a blank rectangle. Both panes
//! declare one now, and [`brightfield_workbench::audit`] is what makes that true
//! rather than remembered.

use std::collections::BTreeSet;

use brightfield_engine::coordinator::{Coordinator, Interaction};
use brightfield_engine::{AxisExtent, NavigationExtent};
use brightfield_keys::BindingContext;
use brightfield_render::canvas_host::{ChartSurface, Color, PixelSize};
use brightfield_spec::analysis::ComponentPath;
use brightfield_spec::ast::SpecValue;
use brightfield_spec::vocab::MarkKind;
use brightfield_workbench::item::ModuleHost;
use brightfield_workbench::registry::{ChartKindId, ChartKindRegistry, DockSide, Field, Slot};
use brightfield_workbench::{
    chrome, Activity, ActivityLog, EmptyState, Icon, Item, ItemCtx, ItemId, ItemRegistry, ItemSpec,
    PaneKey, Subject, Verb,
};

use meridian_design::{semantic, spacing};

use crate::canvas::{CanvasSlot, EguiCanvasHost, EguiChartFrame};
use crate::chart_item::ChartItem;
use crate::chart_kinds;
use crate::design::Mode;
use crate::interval_drag::IntervalDrags;
use crate::navigation::{AxisLock, NavGesture, NavOutcome};
use crate::one_step::ColumnFacts;
use crate::pipeline::{Composed, IntervalControl, LiveDashboard};
use crate::watch::FileWatcher;
use brightfield_spec::layout::Rect as SpecRect;

// ---------------------------------------------------------------------------
// ChartDoc — the state every pane in this view shares.
// ---------------------------------------------------------------------------

/// The headline over a fault the engine itself reported — a mark it would not
/// run, or a re-composite that produced nothing and left the previous picture
/// standing.
const ENGINE_REFUSED: &str = "The chart is missing data the engine refused to query";

/// The smallest box [`ChartDoc::reflow_to`] will compose a dashboard into, on
/// either axis, in logical points.
///
/// A plot's margins come off this before any data area is left;
/// `brightfield_render::layout::Margins::default` is what they are when the
/// spec declares none.
pub const MIN_CHART_EXTENT: f32 = 160.0;

/// The headline over a settled navigation whose re-query drew nothing.
///
/// A separate sentence from [`ENGINE_REFUSED`] because it is a separate event.
/// The engine did not refuse anything here: it ran the query the frame asked
/// for and the frame asked about empty space. Filed under the engine's own
/// wording it would read as a fault in the spec, which is the one thing it is
/// not.
const FRAME_OFF_THE_DATA: &str = "The frame moved off the data";

/// **What is wrong with the picture on screen** — one banner's worth of it.
///
/// A headline and the line under it, together, because they are read together
/// and a caller that could take one without the other would eventually put the
/// wrong headline over the right detail. See [`ChartDoc::chart_fault`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartFault {
    /// What happened, in the reader's terms — the banner's title.
    pub title: String,
    /// The supporting line: the engine's own words where they help, the
    /// gesture's where they help more.
    pub detail: String,
}

/// How a document's picture was chosen, when **one chart kind** chose the whole
/// of it.
///
/// A table opened with no spec gets a tile per column out of
/// [`crate::dashboard`], each tile a kind the registry chose for that column.
/// Where the walk produced exactly **one** tile, that tile's picture is the
/// document's picture: this record carries the kind, the column bound to it and
/// the block that kind builds, which is what lets the chart pane re-make the
/// [`ChartModule`](brightfield_workbench::item::ChartModule) that draws it,
/// frame after frame, out of the registry rather than out of a branch.
///
/// `None` in the two cases where no single kind built the picture: a dashboard
/// of several tiles, and a document composed from a spec someone **wrote** —
/// for a written spec because no route that opens one asks the registry
/// anything. Which routes those are is enumerated by
/// `a_chart_kinds_picture_carries_its_kind_and_a_written_spec_carries_none` in
/// `tests/data_file.rs`, and that is where a new one gets ruled on.
///
/// The module route reaches exactly the documents carrying one of these:
/// `module_of` in [`crate::chart_item`] opens with `doc.authored()?` and the
/// chart pane presents directly on its `None`. Widening it is not a matter of
/// hosting the same picture differently — a
/// [`ChartModule`](brightfield_workbench::item::ChartModule) rebuilds its spec
/// from a kind and bound columns every frame, and a document nobody bound
/// columns for has nothing to rebuild it from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authored {
    /// The kind that chose the picture.
    pub kind: ChartKindId,
    /// The columns bound to that kind, in the order they were offered.
    pub fields: Vec<Field>,
    /// The spec block that kind built for those columns — the picture this
    /// document draws, in the kind's own standalone form. It is what
    /// [`ChartModule`](brightfield_workbench::item::ChartModule) rebuilds every
    /// frame and what [`ChartDoc::draw_module`] compares against, so it is the
    /// kind's block rather than the document's whole source.
    pub block: String,
}

/// **A second view of the one composed page**, for a canvas drawing that page
/// across two panes: the box this view fills and how far the page is moved up
/// inside it.
///
/// The canvas's pane group composes one page — one engine session, one
/// crossfilter selection — and each pane shows its own part of it. The column
/// is the pane that scrolls, because the hero is bounded to the map pane's
/// height ([`crate::dashboard::HERO_BOUND`]): the page is drawn at its own
/// origin for the map and moved up by [`Self::by`] for the column, which
/// `a_wheel_over_the_column_moves_the_column_and_leaves_the_map_where_it_was`
/// reads back off a frame.
///
/// A pointer inside [`Self::clip`] is read against the moved origin, which is
/// what keeps a brush on a scrolled tile landing on the tile under it —
/// `a_brush_on_a_scrolled_tile_lands_on_the_tile_under_the_pointer`. That is a
/// per-frame answer, and a gesture spanning frames latches its own instead:
/// [`page_offset`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SecondView {
    /// The box this view fills, in window-space logical points. Nothing it
    /// paints reaches outside this, and a pointer outside it is not in this
    /// view.
    pub clip: egui::Rect,
    /// How far the page is moved **up** inside [`Self::clip`], in logical
    /// points. Zero is the same view as the first one.
    pub by: f32,
}

impl SecondView {
    /// **Whether this view holds the page's column at screen `x`** — the
    /// containment rule for a page drawn at two origins, written once and read
    /// by both callers that need it: the pointer mapping in
    /// [`crate::chart_item`] (through [`page_offset`]) and the plot readback in
    /// [`crate::window::MeridianApp::composed_plot_rects`].
    ///
    /// Membership is **horizontal**, because the pane group is a left-right
    /// split: the two views take disjoint parts of the page's width and share
    /// its vertical band. It is not a rect test, and that is the part worth
    /// stating — this is asked about a page *taller* than the pane it is drawn
    /// in, so a tile standing below the column's content bottom is still the
    /// column's tile. A rect test would hand that tile to the first view and
    /// report it at the unmoved origin, which is the readback
    /// `a_wheel_over_the_column_moves_the_column_and_leaves_the_map_where_it_was`
    /// holds to the offset the frame applied. [`page_offset`] adds the vertical
    /// clause for the one caller whose subject is a pointer rather than a plot.
    #[must_use]
    pub fn holds(self, x: f32) -> bool {
        self.clip.x_range().contains(x)
    }
}

/// **How far up the page is moved for a pointer**, in logical points: zero for
/// the first view's origin and [`SecondView::by`] for the second's.
///
/// The one place a screen point becomes a page origin. `latched` is a gesture's
/// own origin, captured at its press edge, and when it is `Some` it is the
/// answer whatever the pointer has crossed since — see [`SecondView::holds`]
/// for the containment rule this shares with the plot readback.
///
/// **Why a gesture latches and a frame does not.** A press, a hover and a wheel
/// zoom are facts about one frame, so the origin the pointer is in *now* is the
/// right one for them. A brush and a pan are relations *between* frames: the
/// drag's start and the pan's previous point were taken in the origin the press
/// chose, and differencing them against a point read in the other origin
/// subtracts across two coordinate systems. Unlatched, a sweep begun on the map
/// and released in the column committed a band taller than the same screen
/// gesture on an unscrolled column by exactly `by` points of the plot's own
/// scale, and a pan crossing the boundary jumped the frame by `by` in one step.
/// The pins are `a_brush_across_the_pane_boundary_commits_what_it_swept` and
/// `a_pan_across_the_pane_boundary_moves_by_what_the_hand_moved`.
#[must_use]
pub fn page_offset(view: Option<SecondView>, latched: Option<f32>, at: Option<egui::Pos2>) -> f32 {
    if let Some(by) = latched {
        return by;
    }
    match (view, at) {
        // The vertical clause a pointer needs and a plot does not: this view
        // paints nothing outside its own box, so a pointer above or below it —
        // the pane's own header band, the chrome under the canvas — is over
        // nothing this view drew, and reads against the first origin.
        (Some(view), Some(p)) if view.holds(p.x) && view.clip.y_range().contains(p.y) => view.by,
        _ => 0.0,
    }
}

/// The chart view's **document**: the composited dashboard, the canvas it
/// rasters into, and the chart state the panes read.
///
/// No [`Item`] holds a handle to it — the shell hands out exactly one
/// `&mut ChartDoc`, for the duration of one pane's draw. That is why the canvas
/// host lives here rather than inside the canvas pane, and why the overlay flag
/// lives here rather than inside the controls pane: the controls rail writes it
/// and the chart pane reads it, so it belongs to the view, not to either pane.
pub struct ChartDoc {
    /// The composited Vello dashboard and its logical size.
    pub composed: Composed,
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
    /// **The second pane's view of this document's page**, when a canvas is
    /// drawing one page across two panes — see [`SecondView`].
    ///
    /// `None` for a document drawn in one view, which is what an authored
    /// spec gets — `an_authored_spec_still_draws_one_pane`. Written by the
    /// canvas each frame *before* the pane draws, because it is a fact about
    /// the layout the frame chose rather than about the document.
    pub second_view: Option<SecondView>,
    /// **Whether this frame's wheel travel already has a consumer.**
    ///
    /// The canvas takes the wheel when the pointer is over the pane that
    /// scrolls, and a wheel event with two consumers is one gesture doing two
    /// things — it scrolled the column and zoomed the plot under the cursor at
    /// the same time. Written by the canvas each frame, read by the chart
    /// pane's gesture machine, and false on every frame nobody claims it.
    pub wheel_taken: bool,
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
    /// The rect the raster was presented into last frame, in window-space
    /// logical points — the box the legend must never enter. Recorded for the
    /// reason [`Self::viewport`] is: the no-legend-overlaps-data exercise
    /// holds disjointness over rects a real layout pass produced.
    pub raster_rect: Option<egui::Rect>,
    /// The rect the legend band occupied last frame — `None` when the
    /// dashboard calls for no legend, which is itself an assertable fact.
    pub legend_rect: Option<egui::Rect>,
    /// Where each interval slider's track was drawn last frame, as
    /// `(control key, rect)` in window-space logical points — empty until a
    /// frame has laid the rail out, and empty for a spec that declares none.
    ///
    /// Recorded for the reason [`Self::overlay_checkbox`] is: the only
    /// assertion worth making about a drag is one that aims a real pointer at
    /// the widget a person would grab, and a coordinate typed by hand against
    /// a layout nothing derived it from stops landing the first time a row
    /// height moves without anything going red.
    pub interval_slider_rects: Vec<(String, egui::Rect)>,
    /// **How many tiles stand in the column beside the hero**, when this
    /// document is a generated dashboard laid out as a hero and a column —
    /// `None` for a document that is one picture, which the canvas draws as
    /// one pane.
    ///
    /// It rides on the document because the canvas has to decide, before it
    /// draws anything, whether the region is one pane or a pane group, and the
    /// only body that knows is the generator that laid the page out. Deriving
    /// it from the composed plots' rects instead would be reading the answer
    /// off the geometry the answer produced.
    stacked_tiles: Option<usize>,
    /// The floor the composed page's height is held at, in logical points.
    ///
    /// Zero for every document but a hero-and-column dashboard, whose stacked
    /// tiles have a height floor: the page grows past the pane and the canvas
    /// scrolls it. See [`crate::dashboard::stack_extent`].
    min_page_height: f32,
    /// How this document's picture was chosen, when a chart kind chose it —
    /// see [`Authored`]. Written by the open-a-data-file path, cleared by
    /// [`ChartDoc::open`] with the rest of the outgoing document's state.
    authored: Option<Authored>,
    /// The spec file this dashboard was composed from, when a named file
    /// composed it — what the spec editor opens beside the live chart.
    /// `None` for the shipped starts and the in-memory composes of the test
    /// and capture tiers, whose dashboards have no file behind them.
    pub spec_path: Option<std::path::PathBuf>,
    /// The work this document has in flight — engine queries mark themselves
    /// here, and the shell's one activity indicator reports from it. See
    /// `brightfield_workbench::activity` for why a synchronous query never
    /// draws a cue (and why that is the honest answer, not a gap).
    pub activity: ActivityLog,
    /// The mtime poll over this document's claimed files — the spec it was
    /// composed from and the `file:` data sources that spec names. Wired by
    /// [`ChartDoc::wire_watch`] when a named file composed the document;
    /// watches nothing otherwise.
    pub watch: FileWatcher,
    /// The live, session-holding dashboard behind this document, when the
    /// boot path loaded one. `None` for a one-shot composition (captures, the
    /// pixel tier, the shipped starts): every gesture entry point checks, so
    /// a still frame is still a still frame.
    live: Option<LiveDashboard>,
    /// Selections currently holding a committed gesture, as
    /// `(selection name, contributor)` — what `clear-selection` retracts.
    active_selections: Vec<(String, ComponentPath)>,
    /// Where each interval slider's handle is *right now*, and which of a
    /// drag's values are still owed a query. Public because the rail writes it
    /// and a headless test drives it; see [`crate::interval_drag`] for why the
    /// handle position cannot live in [`Self::composed`].
    pub interval_drags: IntervalDrags,
    /// Why the last gesture failed to change the picture, if it did — read
    /// through [`Self::chart_fault`].
    ///
    /// Written by [`Self::apply_interaction`], which knows only that the
    /// engine refused, and RESTATED by [`Self::pump_navigation`], which knows
    /// what the refused gesture meant. A pan onto empty space fails as
    /// `no marks rendered successfully`; that is the mechanism, not the event,
    /// and the caller is the only place with enough context to say the event.
    interaction_fault: Option<ChartFault>,
    /// The pan/zoom gesture in progress and the settle rule that decides when
    /// it becomes a query. Public because a headless test drives it through the
    /// same entry points the chart pane uses.
    pub nav: NavGesture,
    /// Which axes a navigation gesture may move — cycled from the keyboard, a
    /// property of the view rather than of the spec.
    pub axis_lock: AxisLock,
    /// Which plot the keyboard navigation verbs address. Follows the last plot
    /// a pointer gesture navigated, so `zoom-in` after a wheel zoom keeps
    /// working on the plot the hand was on; 0 until one has.
    nav_plot: usize,
    /// The last thing a navigation gesture REFUSED to do, and why — surfaced
    /// on the chart pane's subject so a categorical axis that will not pan says
    /// so instead of reading as a dead control. Cleared by the next gesture
    /// that does something.
    ///
    /// A **refusal**, never a failure: the gesture was understood and declined
    /// before anything was queried. A gesture that ran and could not be drawn
    /// is a different event with a different lifetime, and it goes to
    /// [`Self::interaction_fault`] and the window's banner instead.
    nav_notice: Option<String>,
    /// What each **tile** of a generated dashboard is of, in the composition's
    /// own plot order — one entry per plot, or empty for a document that was
    /// not generated from a data file.
    ///
    /// The order is the join. `Dashboard::to_spec` lays the tiles out in the
    /// order it chose them and the composition places its plots in the order
    /// the spec declares them, so plot *n* draws tile *n*'s column; this is
    /// that correspondence recorded once, at the point both are in scope,
    /// rather than re-derived from a plot's channel expression at click time.
    tile_columns: Vec<ColumnFacts>,
    /// Which of them the inspector is showing, by index into
    /// [`Self::tile_columns`]. Set by a click on a tile, cleared with the
    /// document.
    selected_tile: Option<usize>,
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
            overlay: true,
            viewport: None,
            second_view: None,
            wheel_taken: false,
            overlay_checkbox: None,
            raster_rect: None,
            legend_rect: None,
            interval_slider_rects: Vec::new(),
            stacked_tiles: None,
            min_page_height: 0.0,
            authored: None,
            spec_path: None,
            activity: ActivityLog::new(),
            watch: FileWatcher::new(),
            live: None,
            active_selections: Vec::new(),
            interval_drags: IntervalDrags::new(),
            interaction_fault: None,
            nav: NavGesture::new(),
            axis_lock: AxisLock::default(),
            nav_plot: 0,
            nav_notice: None,
            tile_columns: Vec::new(),
            selected_tile: None,
            canvas: CanvasSlot::new(host),
        }
    }

    /// A document with no device behind it.
    #[must_use]
    pub fn headless(composed: Composed) -> Self {
        Self {
            composed,
            overlay: true,
            viewport: None,
            second_view: None,
            wheel_taken: false,
            overlay_checkbox: None,
            raster_rect: None,
            legend_rect: None,
            interval_slider_rects: Vec::new(),
            stacked_tiles: None,
            min_page_height: 0.0,
            authored: None,
            spec_path: None,
            activity: ActivityLog::new(),
            watch: FileWatcher::new(),
            live: None,
            active_selections: Vec::new(),
            interval_drags: IntervalDrags::new(),
            interaction_fault: None,
            nav: NavGesture::new(),
            axis_lock: AxisLock::default(),
            nav_plot: 0,
            nav_notice: None,
            tile_columns: Vec::new(),
            selected_tile: None,
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

    /// Replace the dashboard with a freshly composed one.
    ///
    /// The `invalidate` is not belt and braces. This document's canvas key is
    /// `{dev_width, dev_height, dark}` and nothing else — the scene is not in
    /// it, because until this existed a dashboard was composed once before the
    /// window opened and never rebuilt. `CanvasSlot::present` returns early on
    /// an unchanged key, so a new dashboard that happens to be the same pixel
    /// size as the old one would leave the *old* raster on screen with no
    /// error anywhere: a stale picture that reads as a GPU fault. Dropping the
    /// presented key is what makes the next `present` actually raster.
    ///
    /// This is the **different-document** entry: it drops any live session,
    /// any committed selections, and the composing file's path, because all
    /// three belong to the spec that is being replaced. A caller holding a
    /// session for the *incoming* document puts it on afterwards, through
    /// [`ChartDoc::attach_live`] — `open_start` and `open_data_file` both do.
    /// A re-composite of the *same* live document goes through
    /// [`ChartDoc::apply_interaction`], which keeps them.
    pub fn open(&mut self, composed: Composed) {
        self.composed = composed;
        self.live = None;
        self.active_selections.clear();
        // The handle positions described the replaced document's sliders.
        self.interval_drags.clear();
        // …and the fault described the replaced document's gesture. Left
        // standing it would put the outgoing spec's banner over the incoming
        // spec's chart — the defect `open_chart` exists to prevent, re-made
        // one field down.
        self.interaction_fault = None;
        // …and the extent described the replaced document's plots.
        self.nav.clear();
        self.nav_notice = None;
        self.nav_plot = 0;
        self.authored = None;
        self.spec_path = None;
        // …and the columns described the replaced document's table.
        self.tile_columns = Vec::new();
        self.selected_tile = None;
        // …and the pane group described the replaced document's layout. A
        // document that arrives as one picture must not be drawn in the
        // outgoing dashboard's two panes.
        self.stacked_tiles = None;
        self.min_page_height = 0.0;
        self.second_view = None;
        // The watch list described the replaced document's files, and any
        // in-flight marks belonged to its session — both go with it.
        self.watch.watch(None, Vec::new());
        self.activity = ActivityLog::new();
        self.canvas.invalidate();
    }

    /// Point the file watcher at this document's claimed files: the spec it
    /// was composed from and the `file:` data sources that spec names. A
    /// document with no file behind it (a shipped start, an in-memory
    /// compose) watches nothing.
    ///
    /// Called by the boot path after [`Self::spec_path`] and the live session
    /// are in place — the two fields the list is derived from.
    pub fn wire_watch(&mut self) {
        let Some(spec) = self.spec_path.clone() else {
            self.watch.watch(None, Vec::new());
            return;
        };
        let dir = spec.parent().map(std::path::Path::to_path_buf);
        let data = self
            .live
            .as_ref()
            .map(|live| live.data_files(dir.as_deref()))
            .unwrap_or_default();
        self.watch.watch(Some(spec), data);
    }

    /// Put a live, session-holding dashboard behind this document — the boot
    /// path calls this when it loaded one, and it is what arms every gesture.
    pub fn attach_live(&mut self, live: LiveDashboard) {
        self.live = Some(live);
    }

    /// Whether a live session is behind this document — whether gestures can
    /// re-query at all.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.live.is_some()
    }

    /// Lay the dashboard out into a box of `size` logical points and re-present
    /// it, returning whether the picture changed.
    ///
    /// `false` on a document with no live session, on a box this one is already
    /// composed into, and on a re-composite the engine refused — the previous
    /// picture stands in that last case, as it does for a refused gesture.
    ///
    /// The offered box is floored to whole points and held at or above
    /// [`MIN_CHART_EXTENT`] on each axis, so a pane reported at a fractional
    /// or vanishing size cannot re-query once per frame or ask for a scene
    /// with no range in it.
    ///
    /// **The page's height and the hero's are two numbers here, not one.** The
    /// box offered is the taller of the room and [`Self::set_min_page_height`]'s
    /// floor, because the column's tiles do not compress past their own; what
    /// the floor added is then handed to
    /// [`LiveDashboard::set_hero_bound`](crate::pipeline::LiveDashboard::set_hero_bound)
    /// so the hero is composed at the room it was offered and the growth is the
    /// column's alone. Both are written before the re-present, and either being
    /// news is what makes one happen.
    pub fn reflow_to(&mut self, size: egui::Vec2) -> bool {
        let room = size.y.floor().max(MIN_CHART_EXTENT);
        let page = size
            .y
            .max(self.min_page_height)
            .floor()
            .max(MIN_CHART_EXTENT);
        let box_ = SpecRect::new(
            0.0,
            0.0,
            f64::from(size.x.floor().max(MIN_CHART_EXTENT)),
            f64::from(page),
        );
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        let bound = live.set_hero_bound(f64::from((page - room).max(0.0)));
        if !live.set_viewport(box_) && !bound {
            return false;
        }
        self.activity.begin(Activity::EngineQuery);
        let presented = live.present();
        self.activity.end(Activity::EngineQuery);
        match presented {
            Ok(composed) => {
                self.composed = composed;
                self.canvas.invalidate();
                true
            }
            Err(e) => {
                eprintln!("warning: reflow re-composite failed: {e}");
                self.interaction_fault = Some(ChartFault {
                    title: ENGINE_REFUSED.to_string(),
                    detail: e.to_string(),
                });
                false
            }
        }
    }

    /// The live coordinator behind this document, when one is attached — the
    /// data-grid pane's read path: the windowed step-rows seam, plus the
    /// interaction generation those reads are cache-keyed to. `None` for a
    /// one-shot composition, whose stillness has nothing to read back.
    ///
    /// Public because "did the picture actually change" has exactly one
    /// honest answer — the rows the session now returns — and a gate that
    /// asserted it from a field the code under test writes would be asserting
    /// against itself. [`LiveDashboard::coordinator`] is public for the same
    /// reason one level down.
    pub fn live_coordinator(&mut self) -> Option<&mut Coordinator> {
        self.live.as_mut().map(LiveDashboard::coordinator)
    }

    /// The live dashboard behind this document, read-only — the extent stores
    /// and the spec it was composed from.
    ///
    /// Public for the reason [`Self::live_coordinator`] is: "do the axes and
    /// the rows describe the same range" has exactly one honest answer, and a
    /// gate that asserted it from a field the code under test writes would be
    /// asserting against itself.
    #[must_use]
    pub fn live_dashboard(&self) -> Option<&LiveDashboard> {
        self.live.as_ref()
    }

    /// Whether any selection currently holds a committed gesture.
    #[must_use]
    pub fn selection_active(&self) -> bool {
        !self.active_selections.is_empty()
    }

    /// **The SQL the gestures on this chart are currently holding**, as one
    /// line — `$brush = ("temp" >= 8.6 AND "temp" <= 15.2)`, several selections
    /// joined by ` · `. `None` when nothing is held, which is what makes a
    /// cleared selection clear the readout rather than leave `WHERE TRUE`
    /// standing.
    ///
    /// The clause text is [`LiveDashboard::selection_clauses`]'s, verbatim: the
    /// same `Display` of the same [`SqlPredicate`](brightfield_engine::SqlPredicate)
    /// value that goes into the
    /// emitted query. **Not rounded and not prettified** — a brush lands on
    /// `8.600000000000001` because inverting a pixel through a linear scale
    /// lands there, Mosaic's `literalToSQL` renders a number as bare
    /// `${value}` exactly the same way, and a readout that tidied the digits
    /// would be showing a string nothing executed. Shown *is* executed, or the
    /// readout is a second opinion about the filter rather than a view of it.
    ///
    /// # Why `$name = clause` and not "Filter:"
    ///
    /// Four things make "the filter" ambiguous, and this phrasing is chosen to
    /// be true under all of them — it reports the VALUE a selection holds, and
    /// claims nothing about which rows any particular query dropped.
    ///
    /// 1. **Crossfilter self-exclusion.** Under `select: crossfilter` a plot's
    ///    own query omits its own clause, so "this is your filter" is false for
    ///    exactly the plot the reader is looking at. What every consumer *does*
    ///    agree on is what `$name` holds.
    /// 2. **The executed WHERE is wider than this.** A static `data.filter`, a
    ///    navigation extent pushed into a plot's query and a pushed-down sample
    ///    are all in the SQL and in no selection store. This line is a floor on
    ///    what ran; saying `$name =` rather than `WHERE` is what keeps it from
    ///    reading as the whole clause.
    /// 3. **Highlight does not filter.** Under `highlight, by: $sel` the same
    ///    clause is projected as a per-row boolean and DIMS rows instead of
    ///    removing them, so the word "filter" would be wrong there. "`$sel`
    ///    holds this" is right in both modes, because it is a statement about
    ///    the selection rather than about the rows.
    /// 4. **Nothing held.** No entry at all — not "no filter", which would be a
    ///    claim about the query, and not `TRUE`, which is a clause nobody drew.
    ///
    /// [`LiveDashboard::selection_clauses`]: crate::pipeline::LiveDashboard::selection_clauses
    #[must_use]
    pub fn selection_sql(&self) -> Option<String> {
        let held = self.live.as_ref()?.selection_clauses();
        if held.is_empty() {
            return None;
        }
        Some(
            held.iter()
                .map(|(name, clause)| format!("${name} = {clause}"))
                .collect::<Vec<_>>()
                .join(" · "),
        )
    }

    /// Push one interaction through the coordinator seam and present the
    /// re-composite: the predicate goes into DuckDB, the affected marks
    /// re-query, and the identical layout/scene path repaints. Returns whether
    /// anything was applied (`false` on a document with no live session).
    ///
    /// A failed re-composite keeps the previous picture rather than blanking a
    /// window over one gesture, and records what went wrong for
    /// [`Self::chart_fault`] — which is where the window gets the banner it
    /// raises. It used to report to stderr and nowhere else, which for a
    /// windowed application is the same as not reporting: a control that
    /// quietly stops filtering, under a handle that says it does, is exactly
    /// the failure nobody sees.
    pub fn apply_interaction(&mut self, interaction: Interaction) -> bool {
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        match &interaction {
            Interaction::Select {
                name, contributor, ..
            } => {
                let entry = (name.clone(), contributor.clone());
                if !self.active_selections.contains(&entry) {
                    self.active_selections.push(entry);
                }
            }
            Interaction::ClearSelect { name, contributor } => {
                self.active_selections
                    .retain(|(n, c)| !(n == name && c == contributor));
            }
            Interaction::SetParam { .. } | Interaction::Navigate { .. } => {}
        }
        // The engine seam marks its work. Synchronous today, so begin/end
        // resolve inside one frame and no cue is drawn — see the activity
        // module for why that is the honest answer; the mark is what lights
        // the indicator up unchanged when this moves off the UI thread.
        self.activity.begin(Activity::EngineQuery);
        let applied = live.apply(interaction);
        self.activity.end(Activity::EngineQuery);
        match applied {
            Ok(composed) => {
                self.composed = composed;
                self.canvas.invalidate();
                // A gesture that lands clears the last one that did not: the
                // fault describes the picture on screen, and this is a new one.
                self.interaction_fault = None;
                true
            }
            Err(e) => {
                eprintln!("warning: interaction re-composite failed: {e}");
                self.interaction_fault = Some(ChartFault {
                    title: ENGINE_REFUSED.to_string(),
                    detail: e.to_string(),
                });
                false
            }
        }
    }

    /// What is wrong with the picture on screen, if anything — the words the
    /// window puts in a banner.
    ///
    /// Two sources, folded here because a reader has one question. A whole
    /// re-composite that failed leaves the previous picture standing and is
    /// recorded by [`Self::apply_interaction`]; a composition that ran but had
    /// marks the engine refused carries them on
    /// [`Composed::mark_faults`](crate::pipeline::Composed::mark_faults), and
    /// that is the common case — one bad mark does not fail a composition, it
    /// silently leaves a hole in it.
    ///
    /// This is the runtime half of a defence whose other half is static. A
    /// cross-filter column a subscriber's INLINE source cannot bind is refused
    /// at load by
    /// [`validate_crossfilter_columns`](brightfield_spec::analysis::validate_crossfilter_columns),
    /// because there the schema is in the spec text. Against a `query:` source
    /// there is no schema until DuckDB has one, so the same typo is not
    /// decidable at load by anything — it can only be caught when the binder
    /// rejects it, and the only question left is whether anyone is told.
    #[must_use]
    pub fn chart_fault(&self) -> Option<ChartFault> {
        if let Some(fault) = &self.interaction_fault {
            return Some(fault.clone());
        }
        if self.composed.mark_faults.is_empty() {
            return None;
        }
        Some(ChartFault {
            title: ENGINE_REFUSED.to_string(),
            detail: self
                .composed
                .mark_faults
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }

    /// Retract every committed gesture — the `clear-selection` verb's chart
    /// arm. A no-op on a document with nothing committed.
    pub fn clear_selection(&mut self) {
        for (name, contributor) in std::mem::take(&mut self.active_selections) {
            self.apply_interaction(Interaction::ClearSelect { name, contributor });
        }
    }

    /// Record that an interval slider's handle was dragged to `value`.
    ///
    /// Nothing is queried here. The value becomes what the handle renders at
    /// and the newest value owed a query; [`Self::pump_interval_drags`] is
    /// what turns at most one of them into an interaction. Splitting the two
    /// is the coalescing — see [`crate::interval_drag`].
    pub fn note_interval_drag(&mut self, control: &IntervalControl, value: f64) {
        self.interval_drags.note(control.key(), value);
    }

    /// Dispatch at most one owed value per interval slider, newest only.
    ///
    /// Each dispatch is an `Interaction::Select` carrying the structured
    /// interval clause — the SELECTION path, the one a plot brush takes, so a
    /// drag gets the pre-aggregation cube. It is deliberately not
    /// `set_param`: a param change is a re-emit with a new literal and no cube
    /// behind it, which is what made a slider feel like a series of stalls.
    ///
    /// Returns the number of sliders that dispatched (0 or more), so the frame
    /// loop can tell a settled rail from a moving one.
    pub fn pump_interval_drags(&mut self, controls: &[IntervalControl]) -> usize {
        let mut dispatched = 0;
        for control in controls {
            let Some(value) = self.interval_drags.take_dispatch(control.key()) else {
                continue;
            };
            // Whether the re-query succeeded or failed, the dispatch is over:
            // a handle left permanently outstanding would take the slider out
            // of service for the rest of the session.
            self.apply_interaction(control.interaction(value));
            self.interval_drags.finish(control.key());
            dispatched += 1;
        }
        dispatched
    }

    // -----------------------------------------------------------------------
    // Navigation — the frame moves continuously, the data re-queries once.
    // -----------------------------------------------------------------------

    /// Which plot the keyboard navigation verbs address.
    #[must_use]
    pub fn nav_plot(&self) -> usize {
        self.nav_plot
    }

    /// What the last navigation gesture refused to do, if it refused anything.
    #[must_use]
    pub fn nav_notice(&self) -> Option<&str> {
        self.nav_notice.as_deref()
    }

    /// Whether any plot is currently held at a navigation extent — what the
    /// reset affordance keys its enabled state on.
    ///
    /// Read off the RENDER store, which is where the axes are: a settled
    /// gesture whose re-query drew nothing rolls the query store back and
    /// leaves the axes moved (see [`LiveDashboard::apply`]), and in that state
    /// there is still a frame to reset.
    #[must_use]
    pub fn navigated(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|live| !live.view_extents().is_empty())
    }

    /// **What the frame does not scope.** The marks on navigated plots whose
    /// query the extent could not be pushed into, named by kind — the sentence
    /// the chart pane rails for as long as such a plot is held at an extent.
    ///
    /// This is the honesty channel for a bail that is otherwise invisible.
    /// `examples/regression.yaml` is the case: a `dot` scatter and a
    /// `regressionY` fit share one plot and one pair of columns, the scatter
    /// narrows to the frame, and the fit — a scalar aggregate with no grouping
    /// key to filter beneath — returns the byte-identical row it returned at
    /// full extent. The reader is then shown an ordinary-least-squares line
    /// computed from points that are not on screen, spanning an x range wider
    /// than the frame. Nothing about the drawing says so, so the pane does.
    ///
    /// Derived, never stored: it is a fact about the extent in force, and it
    /// has to stop being said the moment a reset widens the frame back out.
    /// `None` at full extent, on a still document, and when every mark
    /// rescoped.
    #[must_use]
    pub fn nav_scope_notice(&self) -> Option<String> {
        let live = self.live.as_ref()?;
        if live.view_extents().is_empty() {
            return None;
        }
        let mut kinds: Vec<String> = Vec::new();
        for plot in &self.composed.plots {
            for declined in live.declined_navigation(&plot.path) {
                let name = declined.kind.to_string();
                if !kinds.contains(&name) {
                    kinds.push(name);
                }
            }
        }
        if kinds.is_empty() {
            return None;
        }
        // Plural agreement done rather than dodged: this line is read by
        // someone deciding whether to trust a number on the screen, and
        // "the regressionY, heatmap mark still summarises" reads as a bug.
        let (subject, verb, them) = if kinds.len() == 1 {
            (format!("the {} mark", kinds[0]), "summarises", "it")
        } else {
            (
                format!("the {} marks", kinds.join(", ")),
                "summarise",
                "them",
            )
        };
        Some(format!(
            "{subject} still {verb} data outside the frame — navigation cannot rescope {them}"
        ))
    }

    /// Cycle the axis lock and say so.
    pub fn cycle_axis_lock(&mut self) {
        self.axis_lock = self.axis_lock.cycle();
        self.nav_notice = Some(format!("navigation moves {}", self.axis_lock.label()));
    }

    /// Record one step of a navigation gesture on `plot`: the axes move NOW,
    /// and nothing is queried.
    ///
    /// Returns whether the frame actually moved. A gesture every axis refused
    /// leaves the extent alone and files the reason, so the pane can say why
    /// rather than look broken.
    pub fn note_navigation(&mut self, plot: usize, outcome: &NavOutcome) -> bool {
        self.nav_plot = plot;
        self.nav_notice = outcome.refused.first().map(|(axis, why)| why.message(axis));
        if outcome.extent.x.is_none() && outcome.extent.y.is_none() {
            return false;
        }
        let Some(path) = self.composed.plots.get(plot).map(|p| p.path.clone()) else {
            return false;
        };
        self.nav.moved(plot, outcome.extent.clone());
        let Some(live) = self.live.as_mut() else {
            return true;
        };
        live.set_view_extent(&path, outcome.extent.clone());
        // Re-composite at the new extent WITHOUT re-querying. Every mark's SQL
        // is unchanged — the session's extent moves only on settle — so this
        // is served from the batches already materialised, and a sampled
        // plot's unsampled facts from the measurement keyed to that same
        // unchanged SQL. The axes track the hand while the data waits for the
        // gesture to stop. A failed re-composite (a frame moved onto empty
        // space) keeps the previous picture, the same posture an interaction
        // takes.
        match live.present() {
            Ok(composed) => {
                self.composed = composed;
                self.canvas.invalidate();
            }
            Err(e) => eprintln!("warning: navigation re-composite failed: {e}"),
        }
        true
    }

    /// The gesture on the plot has ended — the extent it stopped at is now owed
    /// exactly one query.
    pub fn settle_navigation(&mut self) {
        self.nav.settle();
    }

    /// Issue the settled gesture's re-query, if one is owed. Returns whether a
    /// query was issued — **at most one per settled gesture, ever**.
    ///
    /// The extent is translated to the engine's own form here, against the
    /// plot's channel columns: an axis whose plot names no column for it is
    /// dropped, because a bound with no column to bind to is not a filter.
    pub fn pump_navigation(&mut self) -> bool {
        let Some((plot, extent)) = self.nav.take_settled() else {
            return false;
        };
        let Some(handle) = self.composed.plots.get(plot) else {
            return false;
        };
        let (path, x_col, y_col) = (
            handle.path.clone(),
            handle.x_column.clone(),
            handle.y_column.clone(),
        );
        let engine_extent = NavigationExtent {
            x: extent
                .x
                .zip(x_col)
                .map(|((lo, hi), col)| AxisExtent::new(col, lo, hi)),
            y: extent
                .y
                .zip(y_col)
                .map(|((lo, hi), col)| AxisExtent::new(col, lo, hi)),
        };
        let applied = self.apply_interaction(Interaction::Navigate {
            plot: ComponentPath(path),
            extent: engine_extent,
        });
        if !applied {
            // The settled frame composed nothing — a gesture that landed off
            // the data. The dashboard has rolled its query store back, so the
            // rows on screen are honest; what would still be missing is anyone
            // saying why the picture stopped changing.
            //
            // It is said ONCE, on the banner, and deliberately not also on the
            // rail. The two surfaces are not two audiences, they are two
            // lifetimes: the rail carries what is true of the extent currently
            // in force — `nav_scope_notice`'s declining mark, which stands
            // until a reset — and the banner carries what one gesture just did,
            // which the reader can dismiss because it is over. A dead end is
            // the second kind. Saying it in both places would put a permanent
            // rail entry under a dismissable banner about the same instant, and
            // the reader would have to work out that they are one event.
            //
            // `apply_interaction` has already filed the engine's own account of
            // it — `no marks rendered successfully`, which names the mechanism
            // and leaves the reader to guess the cause. This replaces it,
            // because this is the frame that knows the gesture was a pan.
            self.interaction_fault = Some(ChartFault {
                title: FRAME_OFF_THE_DATA.to_string(),
                detail: "Nothing to draw in this range. The rows on screen are the \
                         ones from before the gesture; the axes are where the gesture \
                         left them. Reset the frame to bring the two back together."
                    .to_string(),
            });
        }
        applied
    }

    /// Return every plot to full extent — the **explicit reset**, and the only
    /// thing that clears a navigation extent.
    ///
    /// Deliberately not folded into `clear-selection`: a brush and a frame are
    /// different state, and one key that silently undid both would make a zoom
    /// impossible to keep while working a cross-filter.
    pub fn reset_navigation(&mut self) -> bool {
        let paths: Vec<String> = self
            .live
            .as_ref()
            .map(|live| live.view_extents().keys().cloned().collect())
            .unwrap_or_default();
        self.nav.clear();
        self.nav_notice = None;
        if paths.is_empty() {
            return false;
        }
        let mut applied = false;
        for path in paths {
            if let Some(live) = self.live.as_mut() {
                live.set_view_extent(&path, brightfield_render::scale::ViewExtent::default());
            }
            applied |= self.apply_interaction(Interaction::Navigate {
                plot: ComponentPath(path),
                extent: NavigationExtent::default(),
            });
        }
        applied
    }

    /// A keyboard pan of the addressed plot, by a fraction of the frame's
    /// width/height — one discrete gesture, so it settles the moment it happens.
    pub fn pan_view(&mut self, fx: f64, fy: f64) -> bool {
        let plot = self.nav_plot;
        let Some(handle) = self.composed.plots.get(plot) else {
            return false;
        };
        let (dx, dy) = (handle.rect.width * fx, handle.rect.height * fy);
        let outcome = crate::navigation::pan(&handle.scales, self.axis_lock, dx, dy);
        self.discrete_navigation(plot, &outcome)
    }

    /// A keyboard zoom of the addressed plot about its centre — one discrete
    /// gesture. `factor` above 1 zooms in.
    pub fn zoom_view(&mut self, factor: f64) -> bool {
        let plot = self.nav_plot;
        let Some(handle) = self.composed.plots.get(plot) else {
            return false;
        };
        let outcome = crate::navigation::zoom(&handle.scales, self.axis_lock, None, factor);
        self.discrete_navigation(plot, &outcome)
    }

    /// A gesture that begins and ends in the same instant (a keystroke): step,
    /// settle, query — one of each.
    fn discrete_navigation(&mut self, plot: usize, outcome: &NavOutcome) -> bool {
        if !self.note_navigation(plot, outcome) {
            return false;
        }
        self.settle_navigation();
        self.pump_navigation()
    }

    /// Drive one spec-declared scalar parameter — the controls rail's slider,
    /// bound to the seam instead of to a field nothing reads.
    pub fn set_param(&mut self, name: &str, value: f64) -> bool {
        self.apply_interaction(Interaction::SetParam {
            name: name.to_string(),
            value: SpecValue::Float(value),
        })
    }

    /// The dashboard's presenting mark kind: the first mark of the first
    /// plot. `None` over an empty document.
    #[must_use]
    pub fn primary_mark(&self) -> Option<MarkKind> {
        self.composed
            .plots
            .first()
            .and_then(|p| p.marks.first())
            .copied()
    }

    /// How this document's picture was chosen, when a chart kind chose it.
    #[must_use]
    pub const fn authored(&self) -> Option<&Authored> {
        self.authored.as_ref()
    }

    /// Record that a chart kind chose this document's picture.
    ///
    /// Called by the open-a-data-file path after [`ChartDoc::open`] has taken
    /// the composed dashboard, for the reason [`ChartDoc::attach_live`] is
    /// called there: `open` is the different-document entry and clears
    /// everything that belonged to the outgoing spec, this included.
    pub fn set_authored(&mut self, authored: Authored) {
        self.authored = Some(authored);
    }

    /// Declare what each tile of this dashboard is of, in plot order — the
    /// open-a-data-file path's, and nothing else's.
    /// Declare that this document is a generated dashboard laid out as a hero
    /// beside a column of `tiles`, or (with `None`) that it is one picture.
    ///
    /// Set by the open-a-data-file path from
    /// [`Dashboard::hero_index`](crate::dashboard::Dashboard::hero_index),
    /// which is where the layout is decided.
    pub fn set_stacked_tiles(&mut self, tiles: Option<usize>) {
        self.stacked_tiles = tiles;
    }

    /// How many tiles stand in the column beside the hero — see
    /// [`Self::set_stacked_tiles`].
    #[must_use]
    pub const fn stacked_tiles(&self) -> Option<usize> {
        self.stacked_tiles
    }

    /// Hold the composed page at `height` logical points or taller.
    ///
    /// The column's tiles have a height floor, so a canvas too short to give
    /// them one composes a page taller than the pane and is scrolled. See
    /// [`crate::dashboard::stack_extent`].
    pub fn set_min_page_height(&mut self, height: f32) {
        self.min_page_height = height;
    }

    pub fn set_tile_columns(&mut self, columns: Vec<ColumnFacts>) {
        self.tile_columns = columns;
        self.selected_tile = None;
    }

    /// What each tile is of, in plot order.
    #[must_use]
    pub fn tile_columns(&self) -> &[ColumnFacts] {
        &self.tile_columns
    }

    /// Select the column tile `plot` draws. Out-of-range indices select
    /// nothing rather than panicking: a document with no declared tile columns
    /// is the ordinary case for every dashboard that came from a spec.
    pub fn select_tile(&mut self, plot: usize) {
        self.selected_tile = (plot < self.tile_columns.len()).then_some(plot);
    }

    /// Select by column name — what an outline row's click resolves to.
    /// A name this document draws no tile for selects nothing.
    pub fn select_column(&mut self, column: &str) {
        self.selected_tile = self.tile_columns.iter().position(|c| c.column == column);
    }

    /// The column the inspector is showing, if one is selected.
    #[must_use]
    pub fn selected_column(&self) -> Option<&ColumnFacts> {
        self.selected_tile.and_then(|i| self.tile_columns.get(i))
    }

    /// Reserve the raster's rect in `ui` and paint the composited dashboard
    /// into it, returning the rect — `None` when the surface reserved nothing.
    ///
    /// **The one place the chart's picture reaches the screen**, reached the
    /// two ways the chart pane's `match module_of(doc)` has arms for: through
    /// [`ModuleHost::draw_module`] for a document carrying an [`Authored`]
    /// record, and directly for a document carrying none. One routine rather
    /// than two, so the two documents cannot drift into two pictures — what
    /// differs between them is which of them a registry gets to decide, not
    /// how the pixels arrive.
    ///
    /// With no device behind the document the raster is blank rather than
    /// apologetic — a headless document is a test fixture — but the *layout*
    /// still happens: the geometry the exercise tests hold is produced with
    /// and without a GPU alike.
    pub fn present_raster(&mut self, ui: &mut egui::Ui) -> Option<egui::Rect> {
        let (w, h) = (self.composed.width, self.composed.height);
        let rect = match self.canvas_texture() {
            Some(texture) => {
                let mut frame = EguiChartFrame::new(ui, texture);
                frame.present(PixelSize {
                    width: w,
                    height: h,
                });
                frame.reserved()
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(w as f32, h as f32),
                    egui::Sense::click_and_drag(),
                );
                Some(rect)
            }
        };
        self.raster_rect = rect;
        rect
    }

    /// The presented texture, if a device is behind this document and a frame
    /// has rastered. The chart pane's read — the canvas slot itself stays
    /// private to this module.
    #[must_use]
    pub(crate) fn canvas_texture(&self) -> Option<egui::TextureId> {
        self.canvas.texture()
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

    /// Draw this document's picture in `mode`, re-composing it if it was drawn
    /// in the other one — and say whether the picture changed.
    ///
    /// A composed scene is a finished list of drawing commands whose brushes
    /// are already resolved, so there is no in-place re-ink: a mode the picture
    /// does not agree with means composing again. That is only possible with a
    /// live session behind the document; a one-shot composition (the capture
    /// tiers, the shipped starts) keeps the mode it was composed at, which is
    /// why the compose entry points take one.
    ///
    /// `false` when the picture already agrees, when there is no session, and
    /// when the engine refused the re-composite — the previous picture stands
    /// in that last case, as it does for a refused gesture and a refused
    /// reflow.
    pub fn set_mode(&mut self, mode: Mode) -> bool {
        if self.composed.mode == mode {
            return false;
        }
        let Some(live) = self.live.as_mut() else {
            return false;
        };
        if !live.set_mode(mode) {
            return false;
        }
        self.activity.begin(Activity::EngineQuery);
        let presented = live.present();
        self.activity.end(Activity::EngineQuery);
        match presented {
            Ok(composed) => {
                self.composed = composed;
                self.canvas.invalidate();
                true
            }
            Err(e) => {
                eprintln!("warning: re-composite for the {mode:?} theme failed: {e}");
                false
            }
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
    ///
    /// The mode reaches the SCENE here, through [`Self::set_mode`], rather than
    /// reaching the base tone under it and stopping. That ordering is the fix:
    /// the key below has carried `dark` since the theme work landed, so a dark
    /// window already re-rastered — over a scene whose colours were still
    /// resolved from light tokens, which is what put a white slab where the
    /// chart is. `the_generated_dashboard_dark_baseline` in
    /// `tests/dashboard_baseline.rs` holds it: remove this line and it counts
    /// 210639 light-surface pixels in a dark window.
    pub(crate) fn present(&mut self, ppp: f32, mode: Mode) {
        self.set_mode(mode);
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
// The document as a chart-module host.
// ---------------------------------------------------------------------------

/// The chart document hosts the shell's chart kinds.
///
/// This is the seam [`brightfield_workbench`] draws between "a chart kind is
/// data" and "something has to rasterise it": a kind builds a spec and stops,
/// and the spec comes back to the document, which owns the composer, the canvas
/// host and the GPU handle that crate has none of.
impl ModuleHost for ChartDoc {
    /// Spec **source** — see [`crate::chart_kinds`] for why a chart kind in
    /// this shell builds a document rather than a structured intermediate.
    type Spec = String;

    fn chart_kinds(&self) -> &ChartKindRegistry<String> {
        chart_kinds::registry()
    }

    /// Present the picture this module's spec asked for.
    ///
    /// **What this checks, and what it does not.** The spec a module hands over
    /// is rebuilt from its kind and its columns every frame, and the block in
    /// [`Authored`] is what the kind built when the document was opened — so an
    /// equal pair means the module still names the picture on screen, over the
    /// same columns. It is not the source the raster was composed from: that is
    /// the whole generated dashboard, and the tile form of a kind is not its
    /// standalone form (`dashboard`'s header says why). Presenting under a
    /// module whose spec has changed would put one chart under another chart's
    /// module, so a disagreement draws nothing rather than something wrong. It
    /// does not compose the incoming spec — a document whose module has moved on
    /// stays blank until something re-opens it, and composing here is the
    /// follow-on that would fix that.
    ///
    /// No shipped kind can reach the disagreement today, and the two reasons
    /// are each checkable rather than remembered: no kind declares a control
    /// (`chart_kinds`'s
    /// `no_kind_declares_a_control_that_the_pane_would_have_to_remember`), and
    /// `ChartModule::set_fields` has no call site in the workspace, so a
    /// module's columns are whatever the document handed it. It is reachable
    /// from a test, which is where it is held.
    fn draw_module(&mut self, spec: &String, ui: &mut egui::Ui) {
        if self.authored.as_ref().map(|a| a.block.as_str()) != Some(spec.as_str()) {
            return;
        }
        self.present_raster(ui);
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
///
/// Published from the gallery-*inclusive* registry, whatever the dev flag
/// says: a layout saved while the gallery flag was on names its pane, and an
/// id that stops being published makes that whole file unloadable. The pane
/// itself stays flag-gated — an unpublished id corrupts the file, an
/// uninstantiated item merely draws the orphan-pane treatment.
pub fn publish_item_ids() {
    chart_registry_with(true).publish_ids();
}

/// The chart pane's address — the key its Vello texture slot is filed under.
const CHART_PANE: PaneKey = PaneKey::new(CHART);

/// The controls rail's share of the window. Declared once and read twice: the
/// registry lays the dock out with it, and
/// [`chart_window_size`](crate::window::chart_window_size) sizes the window from
/// it. It replaces three numbers that disagreed — a panel pinned at 180 logical
/// points, a `window_size` that budgeted 214 for it, and a `main.rs` that
/// budgeted 200.
pub(crate) const CONTROLS_SHARE: f32 = 0.2;

/// The rail's icon is a *name*, resolved to paint at draw time. The Meridian
/// icon set has not landed in this workspace's chrome, so the chrome reserves
/// the glyph's box without painting into it. The chart pane's icon is the
/// mark kind's own, named by [`ChartItem`].
const ICON_CONTROLS: Icon = Icon("sliders");

/// The chart document's registry: the chart canvas, its data-grid peer, the
/// controls rail, and the spec editor, where each sits, and the verbs that
/// show and hide them.
///
/// This is the **only** declaration of this document's panes. The window's
/// default arrangement ([`window_tree`](brightfield_workbench::window_tree)),
/// the live item map ([`ItemRegistry::instantiate`]) and the published id
/// vocabulary ([`ItemRegistry::publish_ids`], via [`publish_item_ids`]) are
/// derived from this list, so a pane cannot be added to one and forgotten in
/// another.
#[must_use]
pub fn chart_registry() -> ItemRegistry<ChartDoc> {
    chart_registry_with(crate::gallery::enabled())
}

/// [`chart_registry`] with the gallery decision explicit — the form the
/// contract tests hold both arrangements through, without touching the
/// process environment. `gallery: true` appends the dev gallery tab
/// ([`crate::gallery::gallery_spec`]); `false` is the shipping arrangement.
#[must_use]
pub fn chart_registry_with(gallery: bool) -> ItemRegistry<ChartDoc> {
    let mut specs = vec![
        ItemSpec {
            id: CHART,
            slot: Slot::Centre,
            toggle: None,
            make: || Box::new(ChartItem::new()),
        },
        crate::data_grid::data_grid_spec(),
        ItemSpec {
            id: CONTROLS,
            slot: Slot::Rail {
                side: DockSide::Right,
                share: CONTROLS_SHARE,
            },
            toggle: Some(Verb::new("toggle-controls-rail")),
            make: || Box::new(ControlsPane),
        },
        crate::editor::editor_spec(),
    ];
    if gallery {
        specs.push(crate::gallery::gallery_spec());
    }
    ItemRegistry::new(specs)
}

// ---------------------------------------------------------------------------
// The controls rail. (The chart pane is [`ChartItem`], in its own module.)
// ---------------------------------------------------------------------------

/// The controls rail: the native egui widgets the render trait does not cover.
///
/// A unit struct for the same reason [`ChartItem`] holds no document handle —
/// the values these widgets drive belong to the document, because the chart
/// pane reads them.
struct ControlsPane;

impl Item<ChartDoc> for ControlsPane {
    fn item_id(&self) -> ItemId {
        CONTROLS
    }

    /// No affordance here on purpose: this rail sits beside the front door,
    /// and two buttons offering different things on a first launch is a
    /// choice nobody asked to make. It says what fills it and points at the
    /// pane that offers the way in.
    fn empty_state(&self, doc: &ChartDoc) -> Option<EmptyState> {
        doc.is_empty().then(|| {
            EmptyState::new(
                ICON_CONTROLS,
                "No dashboard to control",
                "These controls act on a composed dashboard. Open one from the \
                 chart pane.",
            )
        })
    }

    fn describe(&self, _doc: &ChartDoc) -> Subject {
        Subject::new("Controls", ICON_CONTROLS, BindingContext::Workspace)
    }

    fn ui(&mut self, doc: &mut ChartDoc, ui: &mut egui::Ui, cx: &mut ItemCtx<'_>) {
        // The legend that used to sit above these controls is gone for good:
        // a hardcoded "Series A/B/C" swatch block duplicated the chart's own
        // legend and, being fixed at three series, mislabelled a single-series
        // bar chart. The one legend is [`crate::legend`]'s margin panel,
        // drawn by the chart pane from each chart's real series.
        //
        // The slider is two things depending on what the spec declared. A
        // spec with a slider-backed scalar param gets one slider **per
        // declared param**, labelled with the param's name, spanning the
        // widget's own range, and wired through the coordinator seam — a drag
        // is an `Interaction::SetParam`, a pushed value and a re-query, never
        // a Rust-side filter. A spec with no declared params draws no slider at
        // all — just the crosshair toggle below.
        let params = doc.composed.params.clone();
        if doc.is_live() && !params.is_empty() {
            for control in params {
                ui.label(&control.name);
                let mut value = control.value;
                let mut slider = egui::Slider::new(&mut value, control.min..=control.max);
                if let Some(step) = control.step {
                    slider = slider.step_by(step);
                }
                let response = ui.add(slider);
                if response.changed() && (value - control.value).abs() > f64::EPSILON {
                    doc.set_param(&control.name, value);
                }
            }
        }
        // The other slider: an interval slider writes a SELECTION, not a
        // param, so it dispatches an `Interaction::Select` carrying the same
        // structured interval clause a plot brush pushes — which is what puts
        // a sustained drag on the pre-aggregated path.
        //
        // Three things separate it from the param slider above. Its handle is
        // read out of the document's own drag state rather than re-seeded from
        // the composition, so under a drag it shows the POINTER's value and
        // not the last completed query's. It records rather than dispatches:
        // the pump below sends at most one value per slider per frame, newest
        // only. And it never touches the spec — no buffer, no file, no dirty
        // flag moves, because a selection is live state and not an edit.
        let intervals = doc.composed.intervals.clone();
        doc.interval_slider_rects.clear();
        if doc.is_live() && !intervals.is_empty() {
            for control in &intervals {
                ui.label(control.label.as_deref().unwrap_or(&control.selection));
                let mut value = doc.interval_drags.shown(control.key(), control.value);
                let mut slider = egui::Slider::new(&mut value, control.min..=control.max);
                if let Some(step) = control.step {
                    slider = slider.step_by(step);
                }
                let response = ui.add(slider);
                doc.interval_slider_rects
                    .push((control.key().to_string(), response.rect));
                if response.changed() {
                    doc.note_interval_drag(control, value);
                }
            }
            if doc.pump_interval_drags(&intervals) > 0 || doc.interval_drags.any_pending() {
                // A drag that outran its queries has work left; keep frames
                // coming so the next one goes out without waiting on a
                // pointer event that may never arrive.
                ui.ctx().request_repaint();
            }
        }
        doc.overlay_checkbox = Some(ui.checkbox(&mut doc.overlay, "hover overlay").rect);
        if crate::devtools::enabled() {
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
}

// ---------------------------------------------------------------------------
// Unit tests — the document's live seam, which only this module can drive
// through the private fields.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::LiveDashboard;
    use brightfield_engine::SqlPredicate;
    use brightfield_sql::ir::ScalarValue;

    /// A brushable spec with a slider-backed param — both live seams in one
    /// fixture.
    const SPEC: &str = r#"
params:
  brush:
    select: intersect
  threshold: 0
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush, filter: "x > $threshold" }
    x: x
    y: y
"#;

    fn live_doc() -> ChartDoc {
        let mut live = LiveDashboard::load_str(SPEC, None).expect("load");
        let composed = live.present().expect("first paint");
        let mut doc = ChartDoc::headless(composed);
        doc.attach_live(live);
        doc
    }

    /// The chart-side gesture path end to end: a STRUCTURED interval clause
    /// through `apply_interaction` re-queries and re-presents, the committed
    /// selection is tracked, and `clear_selection` retracts it — the
    /// clear-selection verb's whole arm, without a window anywhere.
    #[test]
    fn a_structured_interval_brush_applies_and_clears_through_the_document() {
        let mut doc = live_doc();
        assert!(doc.is_live());
        assert!(!doc.selection_active(), "nothing committed at boot");

        let applied = doc.apply_interaction(Interaction::Select {
            name: "brush".to_string(),
            contributor: ComponentPath("root/plot[9]".to_string()),
            predicate: SqlPredicate::Interval {
                column: "x".to_string(),
                lo: ScalarValue::Float(2.0),
                hi: ScalarValue::Float(3.0),
                meta: None,
            },
        });
        assert!(applied, "a live document applies");
        assert!(doc.selection_active(), "the commitment is tracked");
        assert!(
            doc.composed.width > 0 && doc.composed.height > 0,
            "the re-composite replaced the picture"
        );

        doc.clear_selection();
        assert!(!doc.selection_active(), "clear-selection retracts");
    }

    /// A one-shot document refuses every interaction — a still frame is a
    /// still frame, and the capture tiers depend on that.
    #[test]
    fn a_one_shot_document_applies_nothing() {
        let mut doc = ChartDoc::empty();
        assert!(!doc.apply_interaction(Interaction::SetParam {
            name: "threshold".to_string(),
            value: SpecValue::Float(2.0),
        }));
        assert!(!doc.set_param("threshold", 2.0));
        assert!(!doc.is_live());
    }

    /// The param seam over a spec with a real slider widget: `set_param`
    /// pushes the value into DuckDB, and the next composition's surfaced
    /// control carries the value the slider was dragged to — not the spec's
    /// boot value, which is what would make the control snap back under the
    /// user's pointer.
    #[test]
    fn set_param_re_queries_and_the_surfaced_control_keeps_the_new_value() {
        const SLIDER_SPEC: &str = r#"
params:
  threshold: 1
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
vconcat:
  - plot:
      - { mark: dot, data: { from: t, filter: "x > $threshold" }, x: x, y: y }
  - input: slider
    label: Threshold
    as: $threshold
    min: 0
    max: 3
    step: 1
"#;
        let mut live = LiveDashboard::load_str(SLIDER_SPEC, None).expect("load");
        let composed = live.present().expect("first paint");
        assert_eq!(
            composed.params,
            vec![crate::pipeline::ParamControl {
                name: "threshold".to_string(),
                value: 1.0,
                min: 0.0,
                max: 3.0,
                step: Some(1.0),
            }],
            "the widget's own range is what the rail binds"
        );
        let mut doc = ChartDoc::headless(composed);
        doc.attach_live(live);

        assert!(doc.set_param("threshold", 2.0));
        let control = doc
            .composed
            .params
            .iter()
            .find(|p| p.name == "threshold")
            .expect("the control survives the re-composite");
        assert!(
            (control.value - 2.0).abs() < f64::EPSILON,
            "the surfaced control snapped back to {}",
            control.value
        );
    }
}
