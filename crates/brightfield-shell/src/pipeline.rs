//! Spec → composited Vello scene (framework-free).
//!
//! A focused port of the app's `build_everything` plot-composition path, using
//! only the framework-free crates (`brightfield-spec` / `-engine` / `-sql` /
//! `-render`). It parses a Mosaic spec, executes each mark's query on the
//! engine, builds one Vello scene per plot (its own axes/scales/legend, titles
//! and axis insets resolved via the same public helpers the app uses), and
//! composites them into a single dashboard scene the egui host presents.
//!
//! Scope for the loop-first phase: colour-scheme / projection / highlight /
//! explicit colorDomain and standalone-legend relocation are NOT ported (the
//! golden `dashboard.yaml` and the simple examples use none of them). Each mark
//! draws EVERY materialised chunk — its result batches are assembled into one
//! drawable batch via [`assemble_batches`], so a row-per-mark chart wider than a
//! single ~2048-row chunk draws all its rows, and an assembly that cannot
//! proceed fails loudly by name rather than silently drawing the first chunk.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use arrow::record_batch::RecordBatch;
use brightfield_conformance::LoadDiagnostics;
use brightfield_engine::coordinator::{Coordinator, Interaction};
use brightfield_engine::error::EngineError;
use brightfield_engine::facts::MarkFacts;
use brightfield_engine::nearest::{NearestProbe, NearestRead};
use brightfield_engine::{
    assemble_batches, DeclinedMark, Engine, NavigationExtent, RowsAudience, Session,
};
use brightfield_render::canvas_host::SurfaceRect;
use brightfield_render::channel::{Channel, ChannelMap};
use brightfield_render::ink::ChartInk;
use brightfield_render::inset::{resolve_insets_for_marks, DEFAULT_SCALE_INSET};
use brightfield_render::layout::{ChartLayout, Margins};
use brightfield_render::mark::{default_renderers, find_renderer, MarkRenderer};
use brightfield_render::sample_notice::{sample_band_margins, SampleFact};
use brightfield_render::sample_policy;
use brightfield_render::scale::{PinnedDomains, Scale, ScaleSet, ViewExtent};
use brightfield_render::scene::{
    build_multi_mark_scene_pinned, compose_dashboard, unrestorable_under_sampling, ChartData,
    UnsampledDomains,
};
use brightfield_render::selection::{
    committed_selection_rect, render_committed_selection, CommittedSelection, Selected,
};
use brightfield_render::{grow_margins, resolve_titles};
use brightfield_spec::analysis::{
    analyse_spec, build_brushable_bindings, BrushKind, ComponentPath,
};
use brightfield_spec::ast::{Component, MarkData, ParamNode, SpaceNode, SpecValue};
use brightfield_spec::layout::{
    collect_plot_nodes, placed_plots, resolve_fixed_domains, resolve_plot_insets, Rect,
};
use brightfield_spec::vocab::MarkKind;
use brightfield_spec::{parse_spec, parse_spec_path, Format, ParseOutput, Spec};
use brightfield_sql::emit::as_bound_selection_default;
use brightfield_sql::ir::{Predicate, SampleRate, ScalarValue};
use brightfield_sql::lower::{compile_selection, NO_SELF_EXCLUDE};
use brightfield_sql::{collect_marks, collect_plot_groups};
use brightfield_workbench::subject::RunState;
use vello::Scene;

use crate::design::Mode;

/// One placed plot of the composed dashboard, carried beside the scene so the
/// shell can act on the chart rather than merely picture it: the margin
/// legend reads the *displayed* scales, and a gesture inverts its pixels
/// through the same set — which is the only way the predicate a brush pushes
/// can mean the rectangle the user drew.
///
/// Everything here is a by-product of the composition that already happened;
/// nothing is recomputed, so a handle cannot disagree with the scene beside it.
pub struct PlotHandle {
    /// The plot node's component path (`root`, `root/hconcat[0]`, …) — the
    /// same join key `collect_plot_groups` and the brushable bindings use.
    pub path: String,
    /// The placed rect on the dashboard plane, in logical pixels.
    pub rect: Rect,
    /// The scale set this plot was drawn against. Pixel↔data inversion for
    /// gestures, and the series the margin legend is accurate to.
    pub scales: ScaleSet,
    /// The layout (margins + insets) the scales' pixel ranges live in.
    pub layout: ChartLayout,
    /// The mark kinds drawn on this plot, in declaration order. The first is
    /// the plot's presenting kind — the parameter the chart item reads.
    pub marks: Vec<MarkKind>,
    /// The plot's brush/point gesture binding, when its spec declares one.
    pub gesture: Option<GestureBinding>,
    /// **This plot's own committed selection, as its raster-local pixel
    /// rectangle** — the same box `ink_committed_selections` washes,
    /// resolved through the *displayed* scales at compose time rather than
    /// recomputed later, on the same standing as the rest of the fields here.
    ///
    /// `None` for a plot holding no selection, one whose constraint cannot be
    /// placed as a rectangle (a category pick), or a one-shot composition
    /// with no session behind it — `ink_committed_selections` writes it
    /// alone, from [`LiveDashboard::present`]. The shell's move gesture is
    /// the one reader: a press inside this rect moves it instead of starting
    /// a fresh sweep.
    pub committed_rect: Option<SurfaceRect>,
    /// The x channel's column on this plot's first mark — the column a
    /// navigation extent over its x axis names. Carried here rather than
    /// re-derived at gesture time because a plot with no brush interactor is
    /// still navigable, so `gesture` cannot be the only place columns live.
    pub x_column: Option<String>,
    /// The y channel's column on this plot's first mark. See
    /// [`PlotHandle::x_column`].
    pub y_column: Option<String>,
    /// `Some` when this plot drew a pushed-down sample — the mirror of
    /// [`ChartData::sample`](brightfield_render::scene::ChartData::sample), so a
    /// surface reading plot handles (chrome, a future export caption) can tell
    /// a sampled plot from a complete one without re-deriving it.
    pub sample: Option<SampleFact>,
    /// **The layer a pointer resting on this plot reads**, when one of its
    /// marks can be read that way — see [`HoverLayer`].
    ///
    /// `None` for a plot whose top layer summarises rather than draws them
    /// (a histogram's bars are bins, and there is no row under one), and for
    /// a plot whose positional channels are not both plain columns.
    pub hover: Option<HoverLayer>,
    /// **This plot's navigated extent has no data beneath it** — its own
    /// marks queried clean and drew zero rows apiece, and it stayed placed
    /// (see the empty-under-navigation fallback in `compose_from_results`)
    /// rather than being dropped. `false` for the overwhelming common case:
    /// a plot with real marks drawn, whether navigated or not.
    ///
    /// The one reader is the map pane's count overlay
    /// (`crate::window::count_overlay_text`): it says how many points the
    /// hero draws, and a static per-file total would say something the
    /// picture beside it does not — this is what tells it to say zero
    /// instead.
    pub navigated_empty: bool,
}

/// **Which of a plot's layers a hover reads, and what that layer encodes.**
///
/// A generated tile is two marks over one table: a ghost that never narrows,
/// drawn first, and the subset that reads the shared selection through
/// `filterBy:`, drawn over it. A reader resting the pointer on a dot is
/// pointing at the layer on top, and that layer is the filtered one — so the
/// nearest-point read has to run against *its* mark index or the row it hands
/// back can be one the brush has already excluded from the picture. Which mark
/// that is comes off the composed spec rather than off a position, and
/// `a_brush_on_a_tile_leaves_the_hover_reading_only_what_the_map_still_draws`
/// is what holds it: it brushes a tile and then asks the map about a row that
/// brush excluded.
#[derive(Clone, Debug, PartialEq)]
pub struct HoverLayer {
    /// The mark's depth-first index — the engine's own mark numbering, so it
    /// is what [`brightfield_engine::Session::nearest_row`] is asked about.
    pub mark: usize,
    /// What this layer encodes, as `(channel, column)` in readout order: x,
    /// y, colour, size. A channel the layer does not bind to a **column** is
    /// absent — a `fill:` written as a colour literal encodes nothing about
    /// the data, and a `{count:}` or a `{bin:}` names a column the step's own
    /// rows do not carry.
    pub channels: Vec<(Channel, String)>,
}

impl HoverLayer {
    /// The column this layer binds to `channel`, if any.
    #[must_use]
    pub fn column(&self, channel: Channel) -> Option<&str> {
        self.channels
            .iter()
            .find(|(c, _)| *c == channel)
            .map(|(_, col)| col.as_str())
    }
}

/// The channels this readout names, in the order it names them.
///
/// Four, and the two positional ones are load-bearing: a readout that named
/// only what the mark was coloured by would leave a reader unable to say where
/// the dot they are pointing at *is*. Colour and size follow because they are
/// the other two channels a mark can encode a column in, and a mark that binds
/// neither shows neither rather than two blank rows.
const READOUT_CHANNELS: [Channel; 4] = [Channel::X, Channel::Y, Channel::Fill, Channel::Size];

/// What one mark encodes, restricted to channels bound to a plain column.
///
/// The **raw option** is what decides, not the channel map: `ChannelMap` maps
/// `y: { count: }` onto the reserved count column and `x: { bin: c }` onto `c`,
/// both of which are columns of the *chart's* query and neither of which is a
/// column of the step's rows. Only a bare string is a column name a row read
/// can project. The channel map is then consulted anyway, because a colour
/// literal is a bare string too and that is the one place the distinction
/// already lives — a literal binds no column there.
fn hover_channels(mark: &brightfield_spec::ast::Mark, map: &ChannelMap) -> Vec<(Channel, String)> {
    READOUT_CHANNELS
        .iter()
        .filter_map(|&channel| {
            let raw = mark.options.get(channel.wire_name())?;
            if !matches!(
                raw,
                brightfield_spec::ast::ValueOrParamRef::Value(SpecValue::String(_))
            ) {
                return None;
            }
            Some((channel, map.get(channel)?.to_string()))
        })
        .collect()
}

/// Whether this mark narrows under a live selection — the `filterBy:` that
/// separates a generated tile's subset layer from the ghost behind it.
fn narrows(mark: &brightfield_spec::ast::Mark) -> bool {
    matches!(
        &mark.data,
        Some(MarkData::From {
            filter_by: Some(_),
            ..
        })
    )
}

/// **How many data units one logical pixel spans** on a scale, or `None` for a
/// scale that cannot answer.
///
/// Linear only, and that is a real limit rather than an oversight. A
/// [`Scale::Band`] has no pixels-per-unit: a category has a slot, not a
/// coordinate, and "how far is this row from the pointer" has no answer along
/// it — `a_hover_on_a_plot_that_encodes_colour_names_the_colour_column` asks
/// that of the banded plot in its fixture. A [`Scale::Time`] does have one, but its column is a DuckDB `TIMESTAMP`
/// and the nearest-point query does arithmetic on the column itself, so a time
/// axis needs an epoch conversion in the emitted SQL that this build does not
/// write — a plot with one offers no hover layer rather than offering a wrong
/// one.
///
/// One definition, two callers: this decides whether a plot *has* a hover
/// layer, and [`crate::chart_item`] uses the same number to build the probe. A
/// second spelling is how a plot could come to declare a hover the pointer
/// path then declines to serve.
#[must_use]
pub fn units_per_pixel(scale: &Scale) -> Option<f64> {
    let Scale::Linear {
        domain_min,
        domain_max,
        range_start,
        range_end,
    } = scale
    else {
        return None;
    };
    let range = range_end - range_start;
    let domain = domain_max - domain_min;
    (range.abs() > f64::EPSILON && domain.abs() > f64::EPSILON).then_some(domain / range)
}

/// The [`HoverLayer`] for a plot, given the marks it **drew** in draw order and
/// the scales it drew them against.
///
/// The layer that narrows, latest first; the topmost drawn layer when none
/// does, which is what an authored single-layer plot gets.
///
/// `None` on either of two counts, and they are different failures. The chosen
/// layer may not bind both positional channels to plain columns — there is no
/// row under a bar whose height is a count. Or the plot's positional scales may
/// not be ones a screen distance can be measured along; see
/// [`units_per_pixel`].
fn hover_layer(
    drawn: &[usize],
    marks: &[&brightfield_spec::ast::Mark],
    maps: &[ChannelMap],
    scales: &ScaleSet,
) -> Option<HoverLayer> {
    for channel in [Channel::X, Channel::Y] {
        scales.get(channel).and_then(units_per_pixel)?;
    }
    let mark = drawn
        .iter()
        .rev()
        .find(|&&mi| narrows(marks[mi]))
        .or_else(|| drawn.last())
        .copied()?;
    let channels = hover_channels(marks[mark], &maps[mark]);
    let positional = channels
        .iter()
        .filter(|(c, _)| matches!(c, Channel::X | Channel::Y))
        .count();
    (positional == 2).then_some(HoverLayer { mark, channels })
}

/// A plot's declared interaction, resolved from the spec's brushable-interactor
/// analysis to exactly what the coordinator seam consumes: which selection the
/// gesture writes, as which contributor, over which columns.
#[derive(Clone, Debug)]
pub struct GestureBinding {
    /// The selection name the gesture writes to (`as: $brush` → `"brush"`).
    pub selection: String,
    /// The contributor identity (the parent plot's node path) — crossfilter
    /// self-exclusion compares this, so it is carried, never re-derived.
    pub contributor: ComponentPath,
    /// Which gesture the interactor declared (interval axes / point toggle).
    pub kind: BrushKind,
    /// The x channel's column expression, when the first mark names one.
    pub x_column: Option<String>,
    /// The y channel's column expression, when the first mark names one.
    pub y_column: Option<String>,
}

/// One spec-declared scalar parameter with a slider widget behind it: what the
/// controls rail binds instead of its worked example, when the spec declares
/// anything to bind.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamControl {
    /// The parameter name (`$threshold` → `"threshold"`).
    pub name: String,
    /// Its current value.
    pub value: f64,
    /// Slider minimum, from the input widget's `min:` (0 when unstated).
    pub min: f64,
    /// Slider maximum, from the input widget's `max:` (1 when unstated).
    pub max: f64,
    /// Slider step, from the input widget's `step:` (`None` = continuous).
    pub step: Option<f64>,
}

/// One spec-declared **interval slider**: a single-handle widget whose value is
/// the moving end of an interval pushed into a named *selection*, rather than a
/// scalar written into a param.
///
/// The vendored upstream corpus declares it as ONE node carrying BOTH
/// discriminators — `input: slider` for the widget, `select: …` for what the
/// widget writes — plus `column:` naming the column the interval is over. This
/// crate's component discriminator tests `select` before `input`, so such a
/// node parses as an [`Interactor`](brightfield_spec::ast::Component::Interactor)
/// and can never reach the scalar-param collector, which matches
/// [`Input`](brightfield_spec::ast::Component::Input) only. That is why this is
/// a second control type collected on the interactor side, not a widened
/// `ParamControl`.
///
/// The collection itself lives in the spec crate
/// ([`build_interval_sliders`](brightfield_spec::analysis::build_interval_sliders));
/// this type is the rail's view of it. It used to be collected here, and the
/// cost was that spec analysis could not see a slider's `column:` — so the
/// cross-filter column check, derived from plot channels, never looked at it.
///
/// **Deviation from the upstream node, deliberately.** Upstream writes
/// `select: interval`. In this build `interval` is registered Unimplemented —
/// in the plot-brush position nothing consumes it — so a spec declaring it
/// draws an "cannot render" banner, and promoting the name to Implemented to
/// silence the banner would publish a capability the brush position does not
/// have. The shipped example therefore declares `intervalX`, which is already
/// Implemented and already maps to a real brush kind. The axis letter is inert
/// here: a slider's interval is over the column `column:` names, not over a
/// plotted axis.
#[derive(Clone, Debug, PartialEq)]
pub struct IntervalControl {
    /// The selection this slider contributes to (`as: $window` → `"window"`).
    pub selection: String,
    /// The column the pushed interval is over (`column: elapsed`).
    pub column: String,
    /// The contributor identity crossfilter self-exclusion compares against.
    ///
    /// A standalone slider has **no plot of its own**, so unlike a brush it
    /// cannot borrow a parent plot's path — and it must not, because a slider
    /// with no picture has no picture to spare: it has to filter *every*
    /// subscriber, including a plot that declares it as a child. The path is
    /// therefore synthetic — the widget's own position with an `input[slider]`
    /// leaf — and the leaf is what makes it safe: the engine compares this
    /// string RAW against
    /// [`plot_node_path`](brightfield_spec::analysis::plot_node_path) of each
    /// subscriber mark, and that function's output always ends at a `/plot[i]`
    /// boundary (or at a `/mark[…]` leaf), so it can never equal a path ending
    /// `/input[slider]`.
    ///
    /// Held by `a_plot_that_declares_the_slider_is_still_filtered_by_it`, and
    /// by that one specifically. The structural sibling
    /// (`an_interval_sliders_contributor_matches_no_plot_in_the_spec`) reads
    /// like the guard and is not: give this field `plot_node_path(path)` —
    /// which is exactly what a brush legitimately does — and the structural
    /// test stays GREEN, because in the shipped example the slider and the
    /// plot differ by accident of layout and collide with nothing. Only the
    /// behavioural test, over a plot that declares the slider as its own
    /// child, goes red. Name the test that fails, not the one that looks
    /// like it would.
    pub contributor: ComponentPath,
    /// The widget's `label:`, when it declared one. The rail falls back to the
    /// selection name.
    pub label: Option<String>,
    /// The interval's FIXED end, from the widget's `min:` — a slider's value is
    /// one bound, so the other has to come from the declared range.
    pub min: f64,
    /// The widget's `max:` — the top of the handle's travel.
    pub max: f64,
    /// The widget's `step:` (`None` = continuous).
    pub step: Option<f64>,
    /// The handle's value as the spec declared it (`value:`, defaulting to
    /// `max:` — the whole range admitted, which is what an untouched slider
    /// should mean).
    pub value: f64,
}

impl IntervalControl {
    /// The stable key this control's UI-owned drag state is filed under. The
    /// contributor path is already unique per widget, so it is the key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.contributor.0
    }

    /// The interval `[min, value]` this slider means at `value` — the same
    /// structured clause a plot brush pushes, which is what puts a drag on the
    /// selection path (and so through the pre-aggregation cube) rather than the
    /// param path.
    #[must_use]
    pub fn predicate(&self, value: f64) -> brightfield_engine::SqlPredicate {
        brightfield_engine::SqlPredicate::Interval {
            column: self.column.clone(),
            lo: brightfield_sql::ir::ScalarValue::Float(self.min),
            hi: brightfield_sql::ir::ScalarValue::Float(value),
            meta: None,
        }
    }

    /// The whole interaction this slider means at `value`.
    #[must_use]
    pub fn interaction(&self, value: f64) -> Interaction {
        Interaction::Select {
            name: self.selection.clone(),
            contributor: self.contributor.clone(),
            predicate: self.predicate(value),
        }
    }
}

/// One composited dashboard ready to present: the merged Vello scene, its
/// logical bounding size, and the spec's declared title (for the window chrome).
pub struct Composed {
    /// The single composited Vello scene (all plots placed on the page plane).
    pub scene: Scene,
    /// Dashboard width in logical pixels.
    pub width: u32,
    /// Dashboard height in logical pixels.
    pub height: u32,
    /// The spec's `meta.title`, if declared.
    pub title: Option<String>,
    /// The placed plots, with the scales and gesture bindings each was
    /// composed against. Empty only for [`Composed::empty`].
    pub plots: Vec<PlotHandle>,
    /// The spec's slider-backed scalar params, for the controls rail.
    pub params: Vec<ParamControl>,
    /// The spec's interval sliders — the selection-writing half of the rail.
    pub intervals: Vec<IntervalControl>,
    /// What the load of this spec had to SAY: the parts of it brightfield
    /// cannot draw, and every warning the parse and the analysis produced.
    ///
    /// It rides on the composition because the composition is what the window
    /// receives, and a diagnostic that arrives separately from the picture it
    /// is about is a diagnostic that arrives at the wrong time. Empty for a
    /// spec that renders whole — which is most of them, and is why a surface
    /// can draw this unconditionally without becoming noise.
    pub diagnostics: LoadDiagnostics,
    /// The run-state of materialised data this preview shows, when it shows
    /// any — the honesty channel at the preview surface.
    ///
    /// `None` means the preview makes **no currency claim**: the compose
    /// paths in this module set `None` because they execute their queries
    /// live for this very composition (nothing previewed here outlived an
    /// edit). A caller whose spec reads output materialised by a pipeline
    /// run annotates via [`Composed::with_run_state`], **ingesting** the
    /// state from that run's contract — it is never computed here.
    ///
    /// The render is [`Composed::run_state_line`]: minimal but real, so an
    /// annotated stale preview is never presented bare. Fuller status chrome
    /// arrives with the chart-side status work and must consume this same
    /// vocabulary rather than define a second one.
    pub run_state: Option<RunState>,
    /// The marks whose query the engine REFUSED for this composition — empty
    /// for every composition that ran whole, which is nearly all of them.
    ///
    /// A mark whose query fails is dropped from the picture and the rest of
    /// the dashboard still draws, which is the right posture: one bad mark
    /// must not take a window down. What was missing is the other half. The
    /// drop went to stderr and nowhere else, so the visible result of, say, an
    /// interval slider naming a column its source does not have was a chart
    /// that emptied itself under a handle that claimed to be filtering it —
    /// indistinguishable, on screen, from a filter that matched no rows.
    ///
    /// It rides on the composition for the reason [`Self::diagnostics`] does:
    /// a diagnostic that arrives separately from the picture it is about
    /// arrives at the wrong time.
    pub mark_faults: Vec<MarkFault>,
    /// **The mode this picture's ink was resolved in.** The colours in
    /// [`Self::scene`] — the chart surface, the grid, the axes, the marks, the
    /// legend — came from this mode's tokens, and the scene is a finished
    /// raster-ready object that cannot be re-inked in place.
    /// `no_light_paint_reaches_the_dark_canvas` in `brightfield-render`'s
    /// `tests/dark_canvas.rs` is what holds the enumeration.
    ///
    /// It rides on the composition because a surface that draws the scene knows
    /// which mode the WINDOW is in and, without this, has no way to tell
    /// whether the picture it is about to paint agrees. That is the whole of
    /// the defect: the chart raster's cache key already carried `dark`, so a
    /// theme switch re-rastered — the same light scene, onto a differently
    /// toned base.
    ///
    /// [`crate::app::ChartDoc::set_mode`] is the reader: it re-presents through
    /// the live session when this disagrees with the mode in force, and the
    /// crate-private `ChartDoc::present` calls it before it rasters.
    pub mode: Mode,
    /// How many of the table's rows are currently selected, against the
    /// table's own total — both counted by the engine, never by measuring a
    /// batch this composition already fetched. `None` when the spec has no
    /// ghost/subset device for `ghost_subset_marks` to find (a hand-authored
    /// plot with one layer, say), which is the honest answer: there is no
    /// predicate seam here to read a count off.
    pub rows: Option<RowCount>,
}

/// The status band's row count: how many of [`Self::total`] the current
/// selection state admits.
///
/// Both counted the SAME way [`brightfield_engine::Session::step_rows_count`]
/// counts a mark's step — `count(*)` over the exact SQL that mark's own rows
/// query runs, not a client-side count of a fetched batch — the test
/// `computing_the_row_count_fetches_no_full_table_result` holds that. So this
/// reads the same predicate a brush pushed rather than a second compilation
/// of it. See `compute_row_count` (this module) for where the two marks come
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowCount {
    /// Rows the current selection state admits — the count under the ghost
    /// device's subset layer (`filterBy: $sel`), which is the compiled
    /// predicate a brush pushed.
    pub selected: u64,
    /// The table's own total — the count under the same device's ghost layer,
    /// which never narrows. Read from the engine, not from a file's own
    /// metadata: a Parquet's row-group header can be stale or absent, and this
    /// is the number DuckDB itself just counted.
    pub total: u64,
}

/// One mark the engine would not run, and what it said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkFault {
    /// The mark's index in the spec's mark order.
    pub mark: usize,
    /// The engine's own words — for a cross-filter onto a column the source
    /// does not expose, DuckDB's binder names the column and lists the ones
    /// that exist, which is the whole of what an author needs.
    pub message: String,
}

impl std::fmt::Display for MarkFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mark {} did not run: {}", self.mark, self.message)
    }
}

impl Composed {
    /// A dashboard with no plots on it: an empty scene, no area, no title.
    ///
    /// [`compose_spec`] never produces this — it returns `Err` when nothing
    /// rendered, and a dashboard's size is the union of its placed plots' rects,
    /// so any success has area. This exists for
    /// [`brightfield_workbench::audit`], which constructs every pane of a view
    /// and asks it what it shows over a document with nothing in it, and so
    /// needs "nothing in it" to be a value that can be built without a spec, a
    /// device or a window.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            scene: Scene::new(),
            width: 0,
            height: 0,
            title: None,
            plots: Vec::new(),
            params: Vec::new(),
            intervals: Vec::new(),
            diagnostics: LoadDiagnostics::default(),
            run_state: None,
            mark_faults: Vec::new(),
            mode: Mode::Light,
            rows: None,
        }
    }

    /// Attach what the load of this spec had to say. Consumes and returns
    /// `self` for the reason [`Composed::with_run_state`] does: the
    /// attachment happens at the compose call site rather than as a mutation
    /// something else can forget to make.
    #[must_use]
    pub fn with_diagnostics(mut self, diagnostics: LoadDiagnostics) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    /// Attach the status band's row count. Consumes and returns `self` for the
    /// reason [`Composed::with_diagnostics`] does: the compose call site is
    /// where the `Session` `compute_row_count` needs is still in scope, so
    /// the attachment has to happen there rather than as a mutation something
    /// else could forget to make.
    #[must_use]
    pub fn with_row_count(mut self, rows: Option<RowCount>) -> Self {
        self.rows = rows;
        self
    }

    /// Annotate this preview with the run-state of the materialised data it
    /// shows, read off the run's contract by the caller. Consumes and returns
    /// `self` so the annotation happens at the compose call site, not as a
    /// mutation something else can forget to make.
    #[must_use]
    pub fn with_run_state(mut self, state: RunState) -> Self {
        self.run_state = Some(state);
        self
    }

    /// The one-line run-state banner this preview draws, when it previews
    /// materialised run output at all. `None` for a live-queried dashboard —
    /// no claim is made, so no label is owed.
    ///
    /// The words and tone come from the workbench vocabulary
    /// ([`RunState::label`] / [`RunState::gloss`]), so a stale preview here
    /// and a stale step in the inspector say it the same way — and a preview
    /// annotated stale can never render the fresh line.
    #[must_use]
    pub fn run_state_line(&self) -> Option<String> {
        self.run_state
            .map(|s| format!("data {} — {}", s.label(), s.gloss()))
    }
}

/// Run a Mosaic spec at `spec_path` through parse → analyse → engine → execute →
/// per-plot scene → composite, returning the [`Composed`] dashboard.
///
/// # Errors
///
/// Returns a human-readable message if any pipeline stage fails or no mark
/// renders.
pub fn compose_spec(spec_path: &str) -> Result<Composed, String> {
    compose_spec_sampled(spec_path, None)
}

/// [`compose_spec`] with the ink resolved for `mode`.
///
/// The one-shot composition is the path with no session behind it — the capture
/// tiers and the pixel baselines — so it is the path that cannot be re-inked
/// later. A caller here says which mode it is photographing and gets that
/// picture; [`compose_spec`] is this at [`Mode::Light`], which is what the light
/// baselines are recorded against.
///
/// # Errors
///
/// As [`compose_spec`].
pub fn compose_spec_in_mode(spec_path: &str, mode: Mode) -> Result<Composed, String> {
    let parsed = parse_spec_path(spec_path).map_err(|e| format!("parse error: {e}"))?;
    compose(
        parsed,
        Some(source_name(spec_path)),
        Path::new(spec_path).parent(),
        None,
        Rect::zero(),
        mode,
    )
}

/// [`compose_spec`] at an explicit pushed-down sample rate.
///
/// `None` is [`compose_spec`] exactly. `Some(rate)` makes every row-level mark
/// draw one row in `rate.modulus()` and say so in its own ink — the switch
/// `--force-sample` turns, so that ONE spec can produce a complete PNG and a
/// sampled PNG over the SAME rows. That comparison is the point: judging the
/// treatment against a different, denser dataset would confound it with the
/// density.
///
/// # Errors
///
/// As [`compose_spec`].
pub fn compose_spec_sampled(
    spec_path: &str,
    sample: Option<SampleRate>,
) -> Result<Composed, String> {
    let parsed = parse_spec_path(spec_path).map_err(|e| format!("parse error: {e}"))?;
    compose(
        parsed,
        Some(source_name(spec_path)),
        Path::new(spec_path).parent(),
        sample,
        Rect::zero(),
        Mode::Light,
    )
}

/// [`compose_spec`], with the session **kept**: the live dashboard holding
/// the DuckDB session for interaction, plus its first composite. What the
/// window boots a command-line chart spec through, so a brush has something
/// to re-query; the one-shot [`compose_spec`] remains the capture tiers'
/// deterministic path.
///
/// # Errors
///
/// As [`compose_spec`].
pub fn live_spec(spec_path: &str) -> Result<(LiveDashboard, Composed), String> {
    live_spec_sampled(spec_path, None)
}

/// [`live_spec`] at an explicit pushed-down sample rate — the live window's
/// half of `--force-sample`, so the window and the captures are judged at the
/// same rate rather than one of them being taken on trust.
///
/// # Errors
///
/// As [`live_spec`].
pub fn live_spec_sampled(
    spec_path: &str,
    sample: Option<SampleRate>,
) -> Result<(LiveDashboard, Composed), String> {
    let parsed = parse_spec_path(spec_path).map_err(|e| format!("parse error: {e}"))?;
    let mut dash = LiveDashboard::load_parsed(
        parsed,
        Some(source_name(spec_path)),
        Path::new(spec_path).parent(),
    )?;
    // Only when the caller named one. The constructor has already applied the
    // policy's answer, and `set_sample(None)` would erase it — turning an
    // automatic sample off is not what "no `--force-sample` on the command
    // line" means.
    if sample.is_some() {
        dash.set_sample(sample);
    }
    let composed = dash.present()?;
    Ok((dash, composed))
}

/// The same pipeline over spec **text** rather than a file.
///
/// What it exists for: the starting points in [`crate::starts`] are
/// `include_str!`-ed into the binary, so there is no path to hand
/// [`compose_spec`] — and inventing one by resolving `examples/` relative to
/// the working directory is exactly the decoy this replaces, since it works
/// from the repo root and nowhere else.
///
/// `base_dir` is where relative `file:` paths in the spec resolve; `None` for
/// a spec whose data is inline, which every embedded start's is.
///
/// # Errors
///
/// As [`compose_spec`].
pub fn compose_spec_str(source: &str, base_dir: Option<&Path>) -> Result<Composed, String> {
    compose_spec_str_at(source, base_dir, Rect::zero())
}

/// [`compose_spec_str`] into a box the caller names, in logical pixels.
///
/// [`Rect::zero`] is [`compose_spec_str`] exactly. A positive extent on an axis
/// is the size the composition is laid out to on that axis, whatever the spec
/// declares — the one-shot half of what a resizable pane does through
/// [`LiveDashboard::set_viewport`].
///
/// # Errors
///
/// As [`compose_spec`].
pub fn compose_spec_str_at(
    source: &str,
    base_dir: Option<&Path>,
    viewport: Rect,
) -> Result<Composed, String> {
    let parsed = parse_spec(source, Format::Yaml).map_err(|e| format!("parse error: {e}"))?;
    compose(parsed, None, base_dir, None, viewport, Mode::Light)
}

/// The name a diagnostic cites for a spec loaded from a path: its file name,
/// which is what the reader has open, not the absolute path they did not type.
fn source_name(spec_path: &str) -> String {
    Path::new(spec_path).file_name().map_or_else(
        || spec_path.to_string(),
        |n| n.to_string_lossy().to_string(),
    )
}

/// Everything after the parse, shared by both entry points above.
///
/// Takes the whole [`ParseOutput`] rather than `.spec` out of it, and that is
/// the point: the previous signature took a `Spec`, so `.warnings` had to be
/// dropped at each call site to satisfy it, and all four of them did. A
/// function that cannot be called without the warnings is the only version of
/// this that stays fixed.
#[allow(clippy::too_many_arguments)]
fn compose(
    parsed: ParseOutput,
    source: Option<String>,
    spec_dir: Option<&Path>,
    sample: Option<SampleRate>,
    viewport: Rect,
    mode: Mode,
) -> Result<Composed, String> {
    let spec = parsed.spec;
    let analysis = analyse_spec(&spec).map_err(|e| format!("analysis error: {e}"))?;
    let diagnostics = LoadDiagnostics::collect(source, &spec, &parsed.warnings, &analysis.warnings);

    let engine = Engine::new();
    let load = engine
        .load_spec(spec.clone(), analysis, spec_dir)
        .map_err(|e| format!("engine error: {e}"))?;
    let mut session = load.session;
    // Resolved BEFORE `execute_all`, which is the whole of the requirement: the
    // first SQL each row-level mark issues already carries the predicate, so no
    // full result set is ever materialised and discarded.
    session.set_sample(resolved_sample(&session, sample));

    // Execute every mark; assemble all its result chunks into one drawable batch.
    let results = session.execute_all();
    let facts = unsampled_facts(&mut session, results.len());
    // A one-shot compose never navigates, so nothing can have declined; read it
    // through the same helper anyway rather than hard-coding the empty answer.
    let beyond = marks_beyond_frame(&session, &spec, results.len());
    // Read BEFORE `results` is moved into `compose_from_results` below.
    let rows = compute_row_count(&session, &spec);
    Ok(compose_from_results(
        &spec,
        results,
        &facts,
        &ViewExtents::new(),
        &beyond,
        &mut PlotPins::new(),
        viewport,
        mode,
        // A one-shot compose passes an empty `ViewExtents::new()` above, so
        // the empty-under-navigation fallback below has no navigated plot to
        // act on — passed anyway rather than `None`, so this path is
        // exercised against a real session shape here, not just the live one.
        Some(&session),
    )?
    .with_diagnostics(diagnostics)
    .with_row_count(rows))
}

/// A live, session-holding dashboard — the push-down seam at the presentation
/// layer (per the push-down architecture: interactions are queries).
///
/// The one-shot [`compose_spec`] path parses, executes once, composites, and
/// **drops the session**: there is no path for a later brush or slider to
/// re-query. [`LiveDashboard`] instead HOLDS a [`Coordinator`] — and therefore
/// the live DuckDB session — across frames. An interaction is handed to
/// [`LiveDashboard::apply`], which resolves it to a predicate/param the engine
/// pushes into DuckDB, re-queries the affected marks, and re-composites through
/// the identical layout/scene path the first paint took (`compose_from_results`).
/// No frame is ever built by filtering a materialised batch in Rust — the filter
/// is in the emitted SQL.
///
/// This is the synchronous handle a single-window presenter drives on its own
/// thread. The off-UI-thread interaction path (coalescing + interrupt +
/// generation-stamped supersession, forced by a sustained drag) is
/// [`brightfield_engine::coordinator::QueryLoop`]; wiring its channels into a
/// specific egui event loop is the chrome layer's concern, not this seam's.
pub struct LiveDashboard {
    coordinator: Coordinator,
    spec: Spec,
    diagnostics: LoadDiagnostics,
    /// What each plot's axes are DRAWN at, keyed by plot node path — the
    /// render-side half of navigation.
    ///
    /// It exists beside the session's own extent store for one reason: a pan
    /// moves the frame on every pointer sample and re-queries once, when the
    /// gesture settles. Between those two moments the axes have to be somewhere,
    /// and that somewhere cannot be the session — writing the session per sample
    /// is the per-frame re-query the settle rule exists to prevent.
    ///
    /// The two cannot drift, because [`LiveDashboard::apply`] writes this one
    /// from the settled interaction itself rather than trusting a caller to
    /// keep them in step. At rest they are equal, and
    /// `the_two_extent_stores_agree_once_a_gesture_has_settled` holds that.
    view_extents: ViewExtents,
    /// What each `Domain: Fixed` plot's axes are pinned to — the frame of
    /// reference a spec asked to hold while filters move the rows.
    ///
    /// It lives here because it is the only thing in the composition that is
    /// deliberately NOT a function of the current query: a pin is captured
    /// from the first composition and re-applied to every later one, so it has
    /// to outlive the composition that produced it. The one-shot compose path
    /// holds no equivalent and needs none.
    pins: PlotPins,
    /// The box this dashboard composes into, in logical pixels — what a
    /// surface hands over through [`LiveDashboard::set_viewport`] once it
    /// knows how much room the chart has.
    ///
    /// [`Rect::zero`] until one does, which is the offer that means "no offer":
    /// a dashboard nobody has sized composes at the spec's own declared size,
    /// so the capture tiers and the boot paint are unchanged by the existence
    /// of this field.
    viewport: Rect,
    /// Whether the session's current sample rate is [`automatic_sample`]'s
    /// answer, rather than one a caller named through [`LiveDashboard::set_sample`]
    /// (which is how `--force-sample` reaches here — see [`live_spec_sampled`]).
    ///
    /// The ceiling policy is a function of the row count INSIDE the current
    /// extent, so a settled navigation — the one interaction that changes what
    /// is inside the extent — re-derives it. But `an_explicit_rate_outranks_the_policy`
    /// holds in both directions: a rate someone named must not be quietly
    /// refined, or coarsened, the next time the reader zooms. This flag is how
    /// [`LiveDashboard::apply`] tells "the policy chose this, re-ask it" from
    /// "a caller chose this, leave it alone".
    sample_is_automatic: bool,
    /// **The mode a composition off this dashboard is inked in.**
    ///
    /// [`Mode::Light`] until a surface says otherwise through
    /// [`LiveDashboard::set_mode`], which is what keeps a caller that has not
    /// been taught the mode drawing exactly what it drew before.
    ///
    /// It lives here rather than being handed to [`LiveDashboard::present`] per
    /// call for the reason [`Self::viewport`] does: a re-present after a gesture
    /// has to carry it too, and a parameter the gesture path could forget is a
    /// dashboard that reverts to light on the first brush.
    mode: Mode,
}

/// What each plot's axes are drawn at, keyed by plot node path.
pub type ViewExtents = HashMap<String, ViewExtent>;

/// What each plot's `Domain: Fixed` axes are pinned to, keyed by plot node
/// path. Absent for a plot whose spec asks for no pin.
pub type PlotPins = HashMap<String, PinnedDomains>;

/// The render-side extent of an engine-side navigation extent — the one
/// conversion between the two, so a bound cannot be transcribed differently in
/// two places.
#[must_use]
pub fn view_extent_of(extent: &NavigationExtent) -> ViewExtent {
    ViewExtent {
        x: extent.x.as_ref().map(|a| (a.min, a.max)),
        y: extent.y.as_ref().map(|a| (a.min, a.max)),
    }
}

impl LiveDashboard {
    /// Load a spec and hold its session live for interaction. `spec_dir` is
    /// where relative `file:` paths resolve (`None` for inline-data specs).
    ///
    /// The caller here holds an already-parsed [`Spec`] and therefore no
    /// parse warnings, so the diagnostics this dashboard reports cover the
    /// preflight walk and the analysis only. A caller that still has its
    /// `ParseOutput` should use [`LiveDashboard::load_parsed`] and keep the
    /// whole picture.
    ///
    /// # Errors
    ///
    /// As [`compose_spec`]: a human-readable message on analyse / load failure.
    pub fn load(spec: Spec, spec_dir: Option<&Path>) -> Result<Self, String> {
        let analysis = analyse_spec(&spec).map_err(|e| format!("analysis error: {e}"))?;
        let diagnostics = LoadDiagnostics::collect(None, &spec, &[], &analysis.warnings);
        let coordinator = Coordinator::load(spec.clone(), analysis, spec_dir)
            .map_err(|e| format!("engine error: {e}"))?;
        Ok(Self::sampled_by_policy(Self {
            coordinator,
            spec,
            diagnostics,
            view_extents: ViewExtents::new(),
            pins: PlotPins::new(),
            viewport: Rect::zero(),
            sample_is_automatic: false,
            mode: Mode::Light,
        }))
    }

    /// Put the automatic rate on a freshly-loaded dashboard, before anything
    /// has executed.
    ///
    /// **Both constructors go through here** — [`LiveDashboard::load`] and
    /// [`LiveDashboard::load_parsed`], which is what [`LiveDashboard::load_str`]
    /// and every file-opening path resolve to — so a document opened from a
    /// file, from spec text or from an embedded start is decided the same way.
    /// A caller with a rate of its own overrides afterwards with
    /// [`LiveDashboard::set_sample`] — see [`live_spec_sampled`], which is why
    /// that call is conditional there rather than unconditional.
    fn sampled_by_policy(mut dash: Self) -> Self {
        let rate = automatic_sample(dash.coordinator.session());
        dash.coordinator.session_mut().set_sample(rate);
        dash.sample_is_automatic = true;
        dash
    }

    /// Load from a whole [`ParseOutput`], keeping its warnings.
    ///
    /// # Errors
    ///
    /// As [`LiveDashboard::load`].
    pub fn load_parsed(
        parsed: ParseOutput,
        source: Option<String>,
        spec_dir: Option<&Path>,
    ) -> Result<Self, String> {
        let spec = parsed.spec;
        let analysis = analyse_spec(&spec).map_err(|e| format!("analysis error: {e}"))?;
        let diagnostics =
            LoadDiagnostics::collect(source, &spec, &parsed.warnings, &analysis.warnings);
        let coordinator = Coordinator::load(spec.clone(), analysis, spec_dir)
            .map_err(|e| format!("engine error: {e}"))?;
        Ok(Self::sampled_by_policy(Self {
            coordinator,
            spec,
            diagnostics,
            view_extents: ViewExtents::new(),
            pins: PlotPins::new(),
            viewport: Rect::zero(),
            sample_is_automatic: false,
            mode: Mode::Light,
        }))
    }

    /// Load from spec text (mirrors [`compose_spec_str`]).
    ///
    /// # Errors
    ///
    /// As [`LiveDashboard::load`].
    pub fn load_str(source: &str, spec_dir: Option<&Path>) -> Result<Self, String> {
        let parsed = parse_spec(source, Format::Yaml).map_err(|e| format!("parse error: {e}"))?;
        Self::load_parsed(parsed, None, spec_dir)
    }

    /// What this dashboard's load had to say.
    #[must_use]
    pub fn diagnostics(&self) -> &LoadDiagnostics {
        &self.diagnostics
    }

    /// The box the next composite will be laid out into.
    #[must_use]
    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    /// The mode the next composite will be inked in.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Tell this dashboard which mode it is being drawn in, and say whether
    /// that is news — the [`LiveDashboard::set_viewport`] shape, for the same
    /// reason: a surface reports the mode once a frame and must not re-query
    /// once a frame.
    ///
    /// The re-query is unavoidable when it IS news. A composed scene is a
    /// finished list of drawing commands with their brushes already resolved;
    /// there is no in-place re-ink, so a new mode means a new composition, and
    /// a new composition means the batches it draws. No code path switches mode
    /// mid-process today — the window takes a mode at boot and
    /// `crate::app::CanvasKey`'s own note records the same — so in practice this
    /// fires at most once, on the first frame of a dark boot.
    pub fn set_mode(&mut self, mode: Mode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        true
    }

    /// Offer this dashboard a box to compose into, and say whether that is
    /// news. A caller re-presents on `true` and does nothing on `false`, which
    /// is what keeps a surface that reports its size every frame from
    /// re-querying every frame.
    pub fn set_viewport(&mut self, viewport: Rect) -> bool {
        if self.viewport == viewport {
            return false;
        }
        self.viewport = viewport;
        true
    }

    /// Hold the hero `points` short of the composed page's height, and say
    /// whether that is news — the [`LiveDashboard::set_viewport`] shape, for
    /// the same reason and on the same frame.
    ///
    /// The page a generated dashboard composes onto is as tall as its **column**
    /// needs, and an `hconcat` offers every item that flexes its whole height,
    /// so the hero would be composed at the column's height and reach below the
    /// map pane it is drawn in. The generator emits it under a `vspace` for
    /// this: a spacer does not flex, so what is written here comes off the
    /// hero's share rather than the column's. See
    /// [`crate::dashboard::HERO_BOUND`], and
    /// `the_hero_is_composed_whole_inside_the_map_pane` for the frame that
    /// reads the result back.
    ///
    /// `false`, and no write, for a spec whose root is not the shape
    /// [`crate::dashboard::Dashboard::to_spec`] emits. An authored spec has no
    /// such spacer and must not acquire one.
    pub fn set_hero_bound(&mut self, points: f64) -> bool {
        let Some(space) = hero_bound_spacer(&mut self.spec) else {
            return false;
        };
        let next = SpecValue::Float(points);
        if space.value == next {
            return false;
        }
        space.value = next;
        true
    }

    /// Composite the CURRENT materialisation into a dashboard scene — the first
    /// paint and every post-interaction re-paint go through here.
    ///
    /// Every composite carries the load's diagnostics, including the ones
    /// after an interaction: the spec did not become renderable because
    /// someone dragged a brush, so a re-present that dropped the warnings
    /// would let a single gesture silence them.
    ///
    /// The committed selections are inked here for the same reason, and it is
    /// the same obligation: the picture is rebuilt from scratch on every
    /// gesture, so a path that composed without them would erase the band on
    /// the next brush and leave a filtered dashboard looking unfiltered.
    ///
    /// # Errors
    ///
    /// As [`compose_spec`] (returns `Err` when nothing renders).
    pub fn present(&mut self) -> Result<Composed, String> {
        let results = self.coordinator.session_mut().execute_all();
        // Gathered on the SAME path as the first paint, not only there: a
        // re-present after a brush that dropped the facts would erase the
        // notice on the first gesture and leave a sampled picture looking
        // complete.
        let facts = unsampled_facts(self.coordinator.session_mut(), results.len());
        // Same reasoning, same seam: a mark that could not be narrowed to the
        // frame has to keep saying so on every repaint, not only the first.
        let beyond = marks_beyond_frame(self.coordinator.session(), &self.spec, results.len());
        // Read on each re-present, the same as the first: this IS the seam
        // that makes the band move under a brush — an interaction lands on
        // the session before `present` is called (see `LiveDashboard::apply`),
        // so the count read here is already under the settled predicate. The
        // test `a_brush_moves_selected_to_the_compiled_predicates_count_and_leaves_total`
        // drives a brush through exactly this second call and reads a moved
        // count back.
        let rows = compute_row_count(self.coordinator.session(), &self.spec);
        let mut composed = compose_from_results(
            &self.spec,
            results,
            &facts,
            &self.view_extents,
            &beyond,
            &mut self.pins,
            self.viewport,
            self.mode,
            Some(self.coordinator.session()),
        )?
        .with_diagnostics(self.diagnostics.clone())
        .with_row_count(rows);
        ink_committed_selections(&mut composed, self.coordinator.session());
        Ok(composed)
    }

    /// Apply one interaction — push its predicate/param into DuckDB, re-query,
    /// and re-composite. This is the seam: an interaction resolves to a query.
    ///
    /// A [`Interaction::SetParam`] also lands in this handle's spec copy, so
    /// the [`ParamControl`]s the next composition surfaces carry the value the
    /// slider was just dragged to — otherwise every re-present would snap the
    /// control back to the spec's boot value while the query ran at the new
    /// one, a lie in whichever direction the reader trusted.
    ///
    /// # Errors
    ///
    /// As [`LiveDashboard::present`].
    pub fn apply(&mut self, interaction: Interaction) -> Result<Composed, String> {
        if let Interaction::SetParam { name, value } = &interaction {
            if let Some(node @ ParamNode::Value(_)) = self.spec.params.get_mut(name) {
                *node = ParamNode::Value(value.clone());
            }
        }
        // A settled navigation writes BOTH stores from the one value it
        // carries. Leaving the render side to the caller is how a settled zoom
        // ends up querying one range and drawing another.
        //
        // The QUERY store is written speculatively and rolled back below if the
        // extent turns out to draw nothing: see the note on that arm.
        let navigated = matches!(interaction, Interaction::Navigate { .. });
        let rollback = match &interaction {
            Interaction::Navigate { plot, extent } => {
                let key = plot.0.clone();
                let previous = self
                    .coordinator
                    .session()
                    .navigation_extent(&key)
                    .cloned()
                    .unwrap_or_default();
                // The automatic rate is a function of the row count inside
                // THIS extent, so it is speculative in exactly the same way
                // the query store's write below is, and for the same reason:
                // an extent that turns out to draw nothing must not leave a
                // rate chosen for it in force.
                let previous_sample = self.coordinator.session().sample();
                self.set_view_extent(&key, view_extent_of(extent));
                Some((key, previous, previous_sample))
            }
            _ => None,
        };
        self.coordinator.apply(interaction);
        // The ceiling policy is asked again HERE, after the extent has landed
        // on the session but before the re-query it governs: a settled
        // navigation is the gesture that changes what is inside the frame,
        // and that count is what `sample_policy::sample_exponent` — the
        // smallest modulus that brings the plot back under the ceiling — is a
        // function of. `Select`, `ClearSelect` and `SetParam` change which
        // rows pass, not how many are in the frame to begin with, so those
        // are deliberately left off this path.
        //
        // Skipped once `sample_is_automatic` is false: an explicit rate
        // (`--force-sample`, or a caller's own `set_sample`) outranks the
        // policy in both directions, so a zoom must not quietly refine OR
        // coarsen a rate someone named.
        //
        // Writing through the session directly (not `Self::set_sample`) is
        // deliberate: that setter is the caller's declaration that they are
        // choosing the rate, and going through it here would flip
        // `sample_is_automatic` to false on the very re-derivation meant to
        // keep it true — the next zoom would then find the flag off and stop
        // re-asking, which is the one-way ratchet AC3 exists to rule out.
        if navigated && self.sample_is_automatic {
            let rate = automatic_sample(self.coordinator.session());
            self.coordinator.session_mut().set_sample(rate);
        }
        let composed = self.present();

        // A gesture onto empty space composes nothing ("no marks rendered
        // successfully"), and the caller keeps the picture it already had. The
        // query store must go back with it: left holding the new extent it
        // would claim the drawn rows are scoped to a range that returned none
        // of them, and every later re-query — a brush, a slider, a full
        // `execute_all` — would re-emit at a range the reader never saw a
        // picture of. The sample rate follows it for the same reason: it was
        // derived from the same rejected extent.
        //
        // The RENDER store is deliberately NOT rolled back. The axes did move,
        // on this step and on every step of the gesture before it; the frame on
        // screen is the moved one, `navigated()` is true because it is, and the
        // reset affordance is offered because pressing it does something. So
        // after a failed settle the two stores disagree ON PURPOSE, and each is
        // true of the half it governs: the render store says where the axes
        // are, the query store says what the rows are. They re-converge on the
        // next settle that draws, or on a reset.
        if let (Err(_), Some((key, previous, previous_sample))) = (&composed, rollback) {
            self.coordinator
                .session_mut()
                .set_navigation_extent(&key, previous);
            self.coordinator.session_mut().set_sample(previous_sample);
        }
        composed
    }

    /// Draw `plot`'s axes at `extent` from the next composite on, WITHOUT
    /// re-querying — what a pan or zoom in progress writes on every step.
    ///
    /// A full extent removes the override, so a reset leaves the map exactly as
    /// a never-navigated dashboard's.
    pub fn set_view_extent(&mut self, plot: &str, extent: ViewExtent) {
        if extent.x.is_none() && extent.y.is_none() {
            self.view_extents.remove(plot);
        } else {
            self.view_extents.insert(plot.to_string(), extent);
        }
    }

    /// What each plot's axes are currently drawn at.
    #[must_use]
    pub fn view_extents(&self) -> &ViewExtents {
        &self.view_extents
    }

    /// What each plot's QUERIES are currently filtered to — the session's own
    /// store, the durable half. Read by the gate that holds the two in step.
    #[must_use]
    pub fn query_extents(&self) -> &std::collections::HashMap<String, NavigationExtent> {
        self.coordinator.session().navigation_extents()
    }

    /// The marks of `plot` that did NOT rescope under the extent it is held
    /// at — see [`brightfield_engine::Session::declined_navigation`].
    ///
    /// Empty for a plot at full extent. Read live rather than remembered: it is
    /// a property of the extent currently in force, and a cached copy would go
    /// on claiming a mark is unscoped after a reset widened the frame back out.
    #[must_use]
    pub fn declined_navigation(&self, plot: &str) -> Vec<DeclinedMark> {
        self.coordinator.session().declined_navigation(plot)
    }

    /// Set the pushed-down sample rate every later query carries — including
    /// every re-query a brush, a click or a slider triggers, which is the
    /// point of holding it on the session rather than on the call.
    ///
    /// A settled navigation is the one exception: it re-asks the policy for
    /// the row count inside its (possibly narrower, possibly wider) extent
    /// rather than carrying this rate forward unchanged — see [`Self::apply`].
    /// Calling this method is what turns that re-asking OFF: from here on the
    /// rate is the caller's, in both directions, until this is called again.
    pub fn set_sample(&mut self, sample: Option<SampleRate>) {
        self.sample_is_automatic = false;
        self.coordinator.session_mut().set_sample(sample);
    }

    /// The live coordinator, for surfaces that read the session directly (a grid
    /// at a step, distinct-value option lists) or hold the interrupt handle.
    pub fn coordinator(&mut self) -> &mut Coordinator {
        &mut self.coordinator
    }

    /// **The nearest drawn row to a point on one of this dashboard's marks.**
    ///
    /// Straight to the session, deliberately not through
    /// [`Coordinator::apply`]: a hover is a question, not an interaction. It
    /// pushes no predicate, it produces no [`Interaction`], and it leaves
    /// [`Coordinator::generation`] where it found it, so no downstream reader
    /// re-composites and none treats the answer as a new state of the picture —
    /// `a_hover_is_not_an_interaction` holds the three of them.
    ///
    /// # Errors
    ///
    /// As [`brightfield_engine::Session::nearest_row`].
    // The Err variant is the engine's own error type, at the engine's own size,
    // on the same terms as `data_grid::fetch_page`: boxing it at this one seam
    // would leave the hover as its single boxed consumer, and its one caller
    // unwraps it into an `Option` on the next line.
    #[allow(clippy::result_large_err)]
    pub fn nearest_row(
        &mut self,
        mark: usize,
        probe: &NearestProbe,
    ) -> Result<NearestRead, brightfield_engine::error::EngineError> {
        self.coordinator.session_mut().nearest_row(mark, probe)
    }

    /// How many DuckDB executes this dashboard's session has performed.
    ///
    /// Surfaced here because the counter is how a test asks "did that frame
    /// issue a query", and the question is asked of the *dashboard* by the
    /// pair that hold the pointer-stillness gate —
    /// `a_sweep_across_a_plot_issues_no_query` and
    /// `a_rest_issues_exactly_one_query_however_long_it_lasts`. Reading it
    /// needs no `&mut`, which is what lets a test take it around a frame it is
    /// also drawing.
    #[must_use]
    pub fn executes(&self) -> usize {
        self.coordinator.session().duckdb_execute_count()
    }

    /// How many distinct SQL strings the session's renderer-side cache holds.
    ///
    /// The other half of the pair above. A hover read that went through the
    /// caching path would move this, and a stream of one-shot pointer
    /// positions would evict the chart's own results out of the LRU —
    /// `a_hover_read_raises_the_execute_count_without_touching_the_cache`.
    #[must_use]
    pub fn sql_cache_len(&self) -> usize {
        self.coordinator.session().sql_cache_len()
    }

    /// **What each live selection currently HOLDS**, as `(name, clause)` pairs
    /// **The mark index this dashboard's rows are read at** by a surface that
    /// is not a plot — [`presenting_rows_mark`] over the spec this dashboard
    /// was composed from.
    ///
    /// It lives here because the spec does: the rows pane holds a `ChartDoc`,
    /// the `ChartDoc` holds this, and the alternative — the pane reaching for
    /// the spec itself — is how a second copy of the rule gets written. There
    /// is one rule and this is the only way to it from a pane.
    #[must_use]
    pub fn rows_mark(&self) -> usize {
        presenting_rows_mark(&self.spec)
    }

    /// ordered by name — the store's own values, resolved through the very
    /// [`compile_selection`] every emitted query resolves them through, so a
    /// surface that shows one of these clauses is showing the string DuckDB
    /// was handed rather than a second rendering of it.
    ///
    /// Sorted because the store is a `HashMap`: iteration order is arbitrary
    /// and a readout that reordered itself between frames would read as
    /// flicker.
    ///
    /// **Held, not applied** — the distinction the wording of any readout over
    /// this has to keep:
    ///
    /// - Under `select: crossfilter` each consumer drops a *different* clause
    ///   (its own), so no single consumer's WHERE can stand for the value.
    ///   [`NO_SELF_EXCLUDE`] drops none, which is the value itself.
    /// - A static `data.filter`, a navigation extent pushed into a plot's
    ///   query, and a pushed-down sample rate all add to the executed WHERE
    ///   and appear in no selection store, so this is a floor on what ran, not
    ///   the whole of it.
    /// - Under `highlight, by: $sel` the clause is projected as a per-row
    ///   boolean and DIMS rather than filtering. The clause is the same string;
    ///   what it does to the rows is not.
    ///
    /// A selection with no live contributor, or one whose contributors compile
    /// to `TRUE` (nothing constrained), is absent — the empty state is an empty
    /// list, never a `WHERE TRUE` nobody drew.
    ///
    /// **`TRUE` is the only value dropped, and `FALSE` is deliberately not.**
    /// A `Predicate::Point` with no values and an empty `Predicate::Or` both
    /// render as `FALSE`, so the question is whether a gesture can make one:
    /// it cannot. Every structured point a click produces carries exactly one
    /// value — `chart_item`'s band-scale resolution and `brightfield_ui`'s
    /// `point_to_structured` each build a one-element `values` — nothing
    /// removes a value from a held clause, and `compile_selection` returns
    /// `TRUE` rather than an empty `Or`/`And` when a selection has no live
    /// contributor. If some future producer ever did hold one, `$name = FALSE`
    /// is the honest line for it: the selection genuinely admits no rows,
    /// which is a fact to show rather than a rest state to hide. `TRUE` is
    /// filtered because it is the one value that means "nothing is held".
    ///
    /// A selection created only by an `as:` binding and never declared in
    /// `params:` — `highlight.yaml`'s `$range` — resolves through
    /// [`as_bound_selection_default`], the same fallback the highlight emit
    /// path takes. Skipping those names would go silent on exactly the specs
    /// whose selection is born from the gesture rather than declared ahead of
    /// it. A declared VALUE param of the same name is not a selection and is
    /// skipped, as it is at emit.
    #[must_use]
    pub fn selection_clauses(&self) -> Vec<(String, Predicate)> {
        let as_bound = as_bound_selection_default();
        let mut held: Vec<(String, Predicate)> = self
            .coordinator
            .session()
            .current_selections()
            .iter()
            .filter_map(|(name, contributors)| {
                let node = match self.spec.params.get(name) {
                    Some(ParamNode::Selection(node)) => node,
                    Some(ParamNode::Value(_)) => return None,
                    None => &as_bound,
                };
                let by_source: Vec<(String, Predicate)> = contributors
                    .iter()
                    .map(|(path, predicate)| (path.0.clone(), predicate.clone()))
                    .collect();
                let clause = compile_selection(node, NO_SELF_EXCLUDE, &by_source);
                (clause != Predicate::True).then(|| (name.clone(), clause))
            })
            .collect();
        held.sort_by(|(a, _), (b, _)| a.cmp(b));
        held
    }

    /// The local files this dashboard's spec reads through `file:` data
    /// sources — see [`spec_data_files`], which this delegates to over the
    /// held spec.
    #[must_use]
    pub fn data_files(&self, spec_dir: Option<&Path>) -> Vec<PathBuf> {
        spec_data_files(&self.spec, spec_dir)
    }
}

/// The spacer that holds the hero short of the page's height, in a spec shaped
/// as [`crate::dashboard::Dashboard::to_spec`] writes one: the last item of the
/// `vconcat` that is the first item of the root `hconcat`.
///
/// `None` for a spec of another shape, and the match is the whole of that
/// judgement — an authored `hconcat` whose first item happens to be a `vconcat`
/// ending in a `vspace` is a spec that asked for exactly this and gets it. The
/// shape is matched here rather than a marker being written into the emitted
/// source, because a spec is a file a reader edits and a magic comment they
/// could delete would take the map's axis with it.
fn hero_bound_spacer(spec: &mut Spec) -> Option<&mut SpaceNode> {
    let Some(Component::HConcat(row)) = spec.root.as_mut() else {
        return None;
    };
    let Some(Component::VConcat(column)) = row.items.first_mut() else {
        return None;
    };
    match column.items.last_mut() {
        Some(Component::VSpace(space)) => Some(space),
        _ => None,
    }
}

/// **Draw each plot's own committed selection onto the dashboard scene** — the
/// half of a cross-filter that had no picture.
///
/// The receiving plot narrows and says so by narrowing. The plot the gesture
/// happened on says nothing once the pointer is up: the brush rectangle is an
/// egui quad painted from the drag state, and a committed selection lived only
/// as a predicate in the engine. This is where it acquires ink.
///
/// Applied to the composed dashboard rather than threaded through
/// [`compose_from_results`] because a [`PlotHandle`] already carries every
/// input the band needs — the placed rect, the layout, and the *displayed*
/// scales the gesture inverted through.
fn ink_committed_selections(composed: &mut Composed, session: &Session) {
    for plot in &mut composed.plots {
        let held = plot_selection(session, plot);
        // Raster-local, so the shell's press-inside-the-committed-rectangle
        // test — `chart_item::drive_gestures` — can compare it directly
        // against a raw pointer point without knowing this plot's own
        // margin offset.
        plot.committed_rect =
            committed_selection_rect(&plot.layout, &plot.scales, &held).map(|r| {
                SurfaceRect::new(
                    r.x0 + plot.rect.x,
                    r.y0 + plot.rect.y,
                    r.width(),
                    r.height(),
                )
            });
        if held.is_empty() {
            continue;
        }
        let mut band = Scene::new();
        render_committed_selection(&mut band, &plot.layout, &plot.scales, &held);
        composed.scene.append(
            &band,
            Some(kurbo::Affine::translate((plot.rect.x, plot.rect.y))),
        );
    }
}

/// What `plot`'s own gesture is holding right now, read from the live
/// per-contributor selection store.
///
/// Keyed on the contributor path, which is the plot's own node path — so a
/// plot draws the clause IT contributed and never a sibling's, whatever the
/// selection's resolution mode does with the two downstream. That is what
/// makes the band true under `crossfilter`, where the brushed plot's own query
/// omits its own clause and the rows under the band are therefore unfiltered.
fn plot_selection(session: &Session, plot: &PlotHandle) -> CommittedSelection {
    let mut held = CommittedSelection::default();
    for contributors in session.current_selections().values() {
        for (contributor, predicate) in contributors {
            if contributor.0 == plot.path {
                gather_selected(predicate, plot, &mut held);
            }
        }
    }
    held
}

/// Fold one contributor's clause into the channels it constrains.
///
/// `And` is walked because an `intervalXY` sweep dispatches one clause per
/// swept axis, `And`-ed. `Or` is deliberately NOT walked: a disjunction of
/// intervals is several disjoint regions, and drawing one of them as though it
/// were the selection is a picture that reads as a narrower filter than the one
/// in force. Nothing is drawn for a shape this cannot represent.
///
/// The first clause naming a channel is the one drawn.
fn gather_selected(predicate: &Predicate, plot: &PlotHandle, held: &mut CommittedSelection) {
    match predicate {
        Predicate::And(parts) => {
            for part in parts {
                gather_selected(part, plot, held);
            }
        }
        Predicate::Interval { column, lo, hi, .. } => {
            let (Some(lo), Some(hi)) = (bound_position(lo), bound_position(hi)) else {
                return;
            };
            if let Some(slot) = channel_slot(plot, column, held) {
                *slot = Some(Selected::Interval(lo, hi));
            }
        }
        Predicate::Point { column, values, .. } => {
            let categories: Vec<String> = values
                .iter()
                .filter_map(|v| match v {
                    ScalarValue::Text(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
            if categories.is_empty() || categories.len() != values.len() {
                return;
            }
            if let Some(slot) = channel_slot(plot, column, held) {
                *slot = Some(Selected::Categories(categories));
            }
        }
        Predicate::Expr(_)
        | Predicate::Param { .. }
        | Predicate::Or(_)
        | Predicate::True
        | Predicate::False => {}
    }
}

/// The channel slot `column` occupies on this plot, when the plot draws that
/// column on a positional channel and has not already taken the slot.
///
/// Matched against the plot's OWN channel columns rather than the gesture
/// binding's, so a clause a plot did not draw an axis for gets no band.
///
/// On the column's NAME rather than on its spelling. A clause carries the
/// column as a SQL identifier (`"observed by hour"`) because that is what a
/// WHERE has to say; a plot handle carries the name the mark was bound to
/// (`observed by hour`). Comparing the two strings directly matched neither the
/// gesture's own clause nor a hand-written one — [`crate::sql_ident`] is the
/// one place that knows they are the same column.
fn channel_slot<'a>(
    plot: &PlotHandle,
    column: &str,
    held: &'a mut CommittedSelection,
) -> Option<&'a mut Option<Selected>> {
    let name = crate::sql_ident::name_of(column);
    if plot.x_column.as_deref() == Some(name.as_ref()) && held.x.is_none() {
        return Some(&mut held.x);
    }
    if plot.y_column.as_deref() == Some(name.as_ref()) && held.y.is_none() {
        return Some(&mut held.y);
    }
    None
}

/// A bound's position in the units its scale reads — microseconds for the two
/// timestamp forms, matching what `Scale::inverse_f64` returned to build them.
/// `None` for a text bound, which no continuous scale can place.
fn bound_position(value: &ScalarValue) -> Option<f64> {
    match value {
        ScalarValue::Float(n) => Some(*n),
        ScalarValue::Int(i) => Some(*i as f64),
        ScalarValue::TimestampMicros(us) | ScalarValue::TimestampTzMicros(us) => Some(*us as f64),
        ScalarValue::Text(_) => None,
    }
}

/// The local files `spec` reads through `file:` data sources, resolved
/// against `spec_dir` — the list the document's file watcher watches. URLs
/// are not files and are skipped; whether a listed path exists is the
/// watcher's business (a file appearing where the spec expects one is a
/// change worth noticing).
#[must_use]
pub fn spec_data_files(spec: &Spec, spec_dir: Option<&Path>) -> Vec<PathBuf> {
    use brightfield_spec::ast::DataSourceKind;
    spec.data
        .values()
        .filter_map(|source| match &source.kind {
            DataSourceKind::File(f) if !f.contains("://") => Some(match spec_dir {
                Some(dir) => dir.join(f),
                None => PathBuf::from(f),
            }),
            _ => None,
        })
        .collect()
}

/// **The rate a session will run at when nobody named one.**
///
/// This is the driver the sampling mechanism shipped without. `--force-sample`
/// is a switch someone has to already know to reach for.
///
/// Answered from [`brightfield_engine::Session::drawn_primitive_estimate`] —
/// counted before any mark is executed — and decided by
/// [`sample_policy::sample_exponent`], which owns the ceiling and the choice of
/// modulus.
///
/// `None` for a spec that draws complete, and `None` when the estimate cannot
/// be taken: a session that cannot count its own rows has no basis to sample,
/// and drawing complete is what this driver replaced.
fn automatic_sample(session: &Session) -> Option<SampleRate> {
    let estimate = match session.drawn_primitive_estimate() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("warning: cannot estimate drawn primitives, drawing complete: {e}");
            return None;
        }
    };
    let exponent = sample_policy::sample_exponent(estimate)?;
    let rate = SampleRate::from_exponent(exponent);
    if rate.is_none() {
        eprintln!(
            "warning: {estimate} drawn primitives needs a 1-in-2^{exponent} sample, which is \
             past the largest representable rate — drawing complete"
        );
    }
    rate
}

/// The rate to run at: the caller's if they named one, otherwise
/// [`automatic_sample`]'s.
///
/// An explicit `--force-sample` outranks the policy in both directions — it is
/// how one spec is rendered complete and sampled over the same rows for
/// comparison, and a policy that overrode it would take that away.
fn resolved_sample(session: &Session, explicit: Option<SampleRate>) -> Option<SampleRate> {
    match explicit {
        Some(rate) => Some(rate),
        None => automatic_sample(session),
    }
}

/// The unsampled facts for every mark, or `None` per mark when the session is
/// not sampling.
///
/// No query at all for a mark the rate did not reach, which is what keeps an
/// unsampled chart's query count byte-unchanged by this feature's existence.
fn unsampled_facts(session: &mut Session, marks: usize) -> Vec<Option<MarkFacts>> {
    (0..marks)
        .map(|i| match session.unsampled_mark_facts(i) {
            None => None,
            Some(Ok(f)) => Some(f),
            Some(Err(e)) => {
                // A failed facts query must not take the picture down with it:
                // the chart still draws, it just cannot say what it is a
                // sample of, and says so by not claiming.
                eprintln!("warning: unsampled facts for mark {i}: {e}");
                None
            }
        })
        .collect()
}

/// Which marks still summarise rows their plot's frame excludes, by flat mark
/// index.
///
/// Read off [`brightfield_engine::Session::declined_navigation`] — the ONE
/// detection, resolved against the very plan the emitter ran — and gathered
/// here beside [`unsampled_facts`] for the same reason that one is: the live
/// re-present after a gesture takes this path too, so a fact gathered only on
/// the first paint would be silently dropped by the first drag.
///
/// Costs nothing on an unnavigated dashboard: `declined_navigation` returns
/// empty for any plot with no extent in force, without planning a thing.
fn marks_beyond_frame(session: &Session, spec: &Spec, marks: usize) -> Vec<bool> {
    let mut out = vec![false; marks];
    for group in collect_plot_groups(spec) {
        for declined in session.declined_navigation(&group.plot_path) {
            if let Some(slot) = out.get_mut(declined.index) {
                *slot = true;
            }
        }
    }
    out
}

/// Build the composited dashboard from a spec and its per-mark execution
/// results. Shared by the one-shot [`compose`] path and the live
/// [`LiveDashboard`] re-query seam, so a re-composite after an interaction takes
/// the identical layout and scene path as the first paint.
///
/// `pins` is the one piece of state that must survive between compositions:
/// a `Domain: Fixed` axis is pinned to the scales its FIRST composition drew
/// against, so this reads a plot's pin from the store before drawing and
/// captures it back afterwards. A caller with nowhere to keep the store passes
/// a fresh one — a composition that is never repeated cannot observe a pin, so
/// the one-shot path is unaffected by construction.
#[allow(clippy::too_many_arguments)]
fn compose_from_results(
    spec: &Spec,
    results: Vec<Result<Vec<RecordBatch>, EngineError>>,
    facts: &[Option<MarkFacts>],
    extents: &ViewExtents,
    beyond_frame: &[bool],
    pins: &mut PlotPins,
    viewport: Rect,
    mode: Mode,
    // The live session, for the one thing this function cannot get from its
    // other arguments: a mark's real column TYPES when its query drew no
    // rows to infer them from. `None` for a caller with no session behind it
    // — every unit test below — which simply leaves the empty-under-navigation
    // fallback unreachable, same as today.
    session: Option<&Session>,
) -> Result<Composed, String> {
    // One canvas for the whole composition, resolved once here: every plot's
    // scene and the page they are placed on take the same answer, so a plot
    // cannot end up a different mode from the dashboard behind it.
    let ink = ChartInk::for_mode(mode.is_dark());
    let marks = collect_marks(spec);
    let mut batches: Vec<Option<RecordBatch>> = Vec::with_capacity(marks.len());
    let mut channel_maps: Vec<ChannelMap> = Vec::with_capacity(marks.len());
    let mut kinds = Vec::with_capacity(marks.len());
    let mut mark_faults: Vec<MarkFault> = Vec::new();
    for (i, result) in results.into_iter().enumerate() {
        // Assemble EVERY materialised chunk into the one batch this mark draws,
        // not just the first ~2048-row chunk. A row-per-mark chart wider than one
        // chunk must draw all its rows; taking `bs.into_iter().next()` silently
        // dropped the rest. Assembly failure is a loud, NAMED error, surfaced
        // here rather than masked by drawing a partial batch.
        let batch = match result {
            Ok(bs) => assemble_batches(bs).map_err(|e| format!("mark {i}: {e}"))?,
            Err(e) => {
                // Kept out of the picture, and no longer kept quiet: the fault
                // rides out on the composition so the window can say which
                // mark the engine refused and why. See [`Composed::mark_faults`].
                eprintln!("warning: skipping mark {i}: {e}");
                mark_faults.push(MarkFault {
                    mark: i,
                    message: e.to_string(),
                });
                None
            }
        };
        batches.push(batch);
        channel_maps.push(ChannelMap::from_mark(marks[i]));
        kinds.push(marks[i].kind);
    }
    // Whether ANYTHING in this composition actually drew a row — the gate the
    // empty-under-navigation fallback below reads before it keeps a plot
    // placed on nothing. A composition with no real content anywhere (a
    // single-plot spec panned or zoomed past its own data, say) must still
    // fail with "no marks rendered successfully" and let the caller roll the
    // gesture back, exactly as before: `a_settled_gesture_that_drew_nothing_rolls_the_query_store_back`
    // (`tests/navigation_extent.rs`) is a single plot navigated to empty and
    // depends on that failure to know to restore the picture it had. It is
    // only a plot alongside OTHER plots that still drew something for which
    // staying placed, empty, keeps `plots`/`groups` at one index apiece.
    let any_real_batch = batches.iter().any(Option::is_some);

    let placed = placed_plots(spec, viewport);
    let groups = collect_plot_groups(spec);
    let plot_nodes = collect_plot_nodes(spec);
    let registry = default_renderers();
    let brushable = build_brushable_bindings(spec);

    // Own each plot's scene; place them below.
    let mut placements: Vec<(f64, f64, Scene)> = Vec::new();
    let mut plots: Vec<PlotHandle> = Vec::new();
    for plot in &placed {
        let Some(group) = groups.iter().find(|g| g.plot_path == plot.path) else {
            continue;
        };

        // The extent this plot is navigated to, if any — read from the ONE
        // store the engine also filters on, so the axes and the numbers cannot
        // describe different ranges.
        let plot_extent = extents.get(&plot.path);

        // Backs `chart_data` in the empty-under-navigation fallback below —
        // declared here, ahead of `chart_data`, so it outlives the borrows
        // `chart_data` takes of it (Rust drops locals in reverse declaration
        // order, and a `ChartData` is exactly as long-lived as the `&Scene`
        // built from it needs).
        let mut synthetic_batches: Vec<RecordBatch> = Vec::new();
        let mut chart_data: Vec<ChartData<'_>> = Vec::new();
        // Set by the empty-under-navigation fallback below, and there alone,
        // once it has actually populated `chart_data` rather than merely
        // attempted to — see [`PlotHandle::navigated_empty`].
        let mut navigated_empty = false;
        let mut plot_marks: Vec<MarkKind> = Vec::new();
        let mut plot_domains = UnsampledDomains::default();
        let mut plot_x_column: Option<String> = None;
        let mut plot_y_column: Option<String> = None;
        // The marks this plot DREW, in draw order — the candidates a hover can
        // read. A mark the engine refused is not on screen, so a pointer
        // cannot be resting on it, and offering it here would hand a reader a
        // row from a layer they cannot see.
        let mut drawn: Vec<usize> = Vec::new();
        for &mi in &group.mark_indices {
            let Some(batch) = batches.get(mi).and_then(|b| b.as_ref()) else {
                continue;
            };
            let renderer: &dyn MarkRenderer = match find_renderer(&registry, kinds[mi]) {
                Some(r) => r,
                None => {
                    eprintln!("warning: no renderer for mark {mi} — skipping");
                    continue;
                }
            };
            plot_marks.push(kinds[mi]);
            drawn.push(mi);
            if plot_x_column.is_none() {
                plot_x_column = channel_maps[mi].get(Channel::X).map(str::to_string);
            }
            if plot_y_column.is_none() {
                plot_y_column = channel_maps[mi].get(Channel::Y).map(str::to_string);
            }
            // A mark is sampled exactly when the session produced unsampled
            // facts for it; `drawn` is what actually arrived, `of` is what the
            // same query returns without the clause.
            let sample = facts.get(mi).and_then(Option::as_ref).map(|f| SampleFact {
                drawn: batch.num_rows() as u64,
                of: f.rows,
            });
            if let Some(f) = facts.get(mi).and_then(Option::as_ref) {
                // Unioned across the plot's marks for the reason the categories
                // below are, and it bites harder here: the colour channel has a
                // containment test that refuses a domain it cannot trust, and
                // the positional one has none, so one mark's extent installed
                // over the plot clips its siblings out of the frame entirely.
                for (channel, measured) in [(Channel::X, f.x_domain), (Channel::Y, f.y_domain)] {
                    if let Some(measured) = measured {
                        plot_domains.merge_extent(channel, measured);
                    }
                }
                // Keyed by the channel's Mosaic wire name on the way out of the
                // engine, which owns no view vocabulary. Unioned across the
                // plot's marks, because the scale this is checked against is: a
                // plot draws one scale per channel and its drawn domain is the
                // union of what its marks drew, so the measured set has to be
                // the union of what its marks measured or the comparison is
                // between two differently-assembled lists.
                for (wire, cats) in &f.categories {
                    let Some(channel) = Channel::from_wire(wire) else {
                        continue;
                    };
                    plot_domains.merge_categories(channel, cats);
                }
                // The band channels' unsampled ORDER, keyed and unioned the
                // same way, and kept apart from the sets above because it is a
                // different quantity: a band scale reads the order, a colour
                // scale reads only the membership.
                for (wire, cats) in &f.band_categories {
                    let Some(channel) = Channel::from_wire(wire) else {
                        continue;
                    };
                    plot_domains.merge_band_categories(channel, cats);
                }
            }
            chart_data.push(ChartData {
                batch,
                channel_map: &channel_maps[mi],
                renderer,
                layout: ChartLayout::new(plot.rect.width, plot.rect.height),
                view_extent: plot_extent,
                highlight: None,
                sample,
                beyond_frame: beyond_frame.get(mi).copied().unwrap_or(false),
            });
        }
        if chart_data.is_empty() {
            // A plot whose marks each queried clean and came back with no
            // rows — no engine refusal touched this group (`mark_faults`
            // carries no entry for any of its marks) and the plot itself has
            // a navigated extent in force. That second condition is what
            // makes this the empty-under-NAVIGATION case rather than an
            // ordinary empty result: an unnavigated plot with no marks to
            // draw keeps today's behaviour (dropped below), because there is
            // no "held frame" for it to stay honest about.
            let no_faults = !group
                .mark_indices
                .iter()
                .any(|mi| mark_faults.iter().any(|f| f.mark == *mi));
            if let (true, Some(session)) = (
                plot_extent.is_some() && no_faults && any_real_batch,
                session,
            ) {
                // Fetch each mark's real column types first, with no
                // `ChartData` borrowing `synthetic_batches` yet — the second
                // pass below is what turns them into entries, once this
                // plot's own marks have a home apiece. A schema miss on any
                // of them (the source has since vanished, say) abandons the
                // whole plot rather than drawing one layer's axes and not the
                // other's, so `chart_data` stays empty and the plot is
                // dropped exactly as it would be without this fallback.
                let renderers: Vec<(usize, &(dyn MarkRenderer + Send + Sync))> = group
                    .mark_indices
                    .iter()
                    .filter_map(|&mi| find_renderer(&registry, kinds[mi]).map(|r| (mi, r)))
                    .collect();
                if renderers.len() == group.mark_indices.len() {
                    for &(mi, _) in &renderers {
                        if let Ok(schema) = session.mark_schema(mi) {
                            synthetic_batches.push(RecordBatch::new_empty(schema));
                        }
                    }
                    if synthetic_batches.len() == renderers.len() {
                        for (k, &(mi, renderer)) in renderers.iter().enumerate() {
                            plot_marks.push(kinds[mi]);
                            drawn.push(mi);
                            if plot_x_column.is_none() {
                                plot_x_column =
                                    channel_maps[mi].get(Channel::X).map(str::to_string);
                            }
                            if plot_y_column.is_none() {
                                plot_y_column =
                                    channel_maps[mi].get(Channel::Y).map(str::to_string);
                            }
                            chart_data.push(ChartData {
                                batch: &synthetic_batches[k],
                                channel_map: &channel_maps[mi],
                                renderer,
                                layout: ChartLayout::new(plot.rect.width, plot.rect.height),
                                view_extent: plot_extent,
                                highlight: None,
                                // Zero rows either way, and no sampling
                                // clause dropped any of them — the frame is
                                // empty because the reader navigated it there.
                                sample: None,
                                beyond_frame: beyond_frame.get(mi).copied().unwrap_or(false),
                            });
                        }
                        navigated_empty = true;
                    } else {
                        synthetic_batches.clear();
                    }
                }
            }
            if chart_data.is_empty() {
                continue;
            }
        }

        // Axis + plot titles, then grow the margins to reserve their band.
        let title_maps: Vec<&ChannelMap> = chart_data.iter().map(|d| d.channel_map).collect();
        let titles = plot_nodes
            .iter()
            .find(|(p, _)| *p == plot.path)
            .map(|(_, node)| resolve_titles(node, &title_maps))
            .unwrap_or_default();
        drop(title_maps);

        // Axis insets so edge marks render whole inside the frame clip.
        let explicit_insets = plot_nodes
            .iter()
            .find(|(p, _)| *p == plot.path)
            .map(|(_, node)| resolve_plot_insets(node))
            .unwrap_or_default();
        let inset_entries: Vec<_> = chart_data
            .iter()
            .map(|d| (d.batch, d.channel_map, d.renderer))
            .collect();
        let insets = resolve_insets_for_marks(explicit_insets, &inset_entries, DEFAULT_SCALE_INSET);
        drop(inset_entries);

        // Two bands, reserved the same way: one for the axis titles, one for
        // the sampling notice. Growing the margin is what makes the device
        // removable later without disturbing anything else's geometry.
        let plot_sample = chart_data.iter().find_map(|d| d.sample);
        let margins = sample_band_margins(
            grow_margins(Margins::default(), &titles),
            plot_sample.is_some(),
        );
        let layout = ChartLayout::with_margins_and_insets(
            plot.rect.width,
            plot.rect.height,
            margins,
            insets,
        );
        for d in &mut chart_data {
            d.layout = layout;
        }

        // What this plot's spec asked to hold still, and what it is holding
        // still so far. The pin is READ before the draw and CAPTURED after, so
        // the first composition draws against its own inference (there is
        // nothing to hold to yet) and every later one draws against that.
        let fixed = plot_nodes
            .iter()
            .find(|(p, _)| *p == plot.path)
            .map(|(_, node)| resolve_fixed_domains(node))
            .unwrap_or_default();
        let plot_pins = pins.get(&plot.path).cloned().unwrap_or_default();

        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        // `draw_inline_legend = false`: the legend is NOT baked into the data
        // scene. The shell draws it as a native margin panel outside the plot
        // rect, from the scales returned here — one legend per chart, one
        // source of truth, and no in-plot swatch block a margin copy could
        // drift from or that could sit on top of the marks.
        let (scene, scales) =
            build_multi_mark_scene_pinned(&refs, false, &titles, &plot_domains, &plot_pins, ink);
        drop(refs);
        drop(chart_data);

        if !fixed.is_empty() {
            let mut held = plot_pins;
            held.capture(&scales, fixed);
            pins.insert(plot.path.clone(), held);
        }

        // REFUSE rather than draw a confidently wrong picture. A sampled plot
        // whose scale set carries a channel `apply_unsampled_domains` cannot
        // restore renders a value in a different place, or a different colour,
        // than the complete one — under a notice that says only that rows were
        // dropped. Checked here because here is where the scales exist — one
        // seam covering `--force-sample` on `brightfield-shot`, on the live
        // window, and every re-present after a gesture, rather than three
        // argument parsers each remembering.
        //
        // The list this reads is derived from the scales and the domains, so it
        // narrows on its own as restorations become available rather than being
        // edited: a colour channel whose set `plot_domains` carries is no longer
        // in it.
        if plot_sample.is_some() {
            let unrestorable = unrestorable_under_sampling(&scales, &plot_domains);
            if !unrestorable.is_empty() {
                let names: Vec<&str> = unrestorable.iter().map(|(c, _)| c.wire_name()).collect();
                let faults: Vec<String> = unrestorable
                    .iter()
                    .map(|(c, why)| format!("{} is {}", c.wire_name(), why.reason()))
                    .collect();
                let names = names.join(", ");
                return Err(format!(
                    "refusing to sample plot {}: {}. The sampled render would place or colour \
                     the same value differently from the complete one, and the sampling notice \
                     would not say so. Drop {names} from this plot, or bind {names} to a column \
                     whose domain the unsampled query can put back.",
                    plot.path,
                    faults.join("; "),
                ));
            }
        }

        placements.push((plot.rect.x, plot.rect.y, scene));

        let gesture = brushable
            .iter()
            .find(|b| b.parent_plot.0 == plot.path)
            .map(|b| GestureBinding {
                selection: b.selection.clone(),
                contributor: b.parent_plot.clone(),
                kind: b.kind,
                x_column: b.channels.x.clone(),
                y_column: b.channels.y.clone(),
            });
        let hover = hover_layer(&drawn, &marks, &channel_maps, &scales);
        plots.push(PlotHandle {
            path: plot.path.clone(),
            rect: plot.rect,
            scales,
            layout,
            marks: plot_marks,
            gesture,
            x_column: plot_x_column,
            y_column: plot_y_column,
            sample: plot_sample,
            hover,
            navigated_empty,
            // Set by `ink_committed_selections` alone — a one-shot
            // composition has no session to hold a selection and does not
            // call it, so it stays `None` here.
            committed_rect: None,
        });
    }

    if placements.is_empty() {
        // Carry the reasons out with the failure. When EVERY mark is refused
        // there is no composition to hang [`Composed::mark_faults`] on, and
        // this is exactly the case where the reasons matter most — a bare "no
        // marks rendered successfully" is the sentence a user got instead of
        // DuckDB naming the column it could not bind.
        return Err(if mark_faults.is_empty() {
            "no marks rendered successfully".to_string()
        } else {
            format!(
                "no marks rendered successfully — {}",
                mark_faults
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        });
    }

    let width = placed
        .iter()
        .map(|p| p.rect.x + p.rect.width)
        .fold(0.0_f64, f64::max)
        .ceil() as u32;
    let height = placed
        .iter()
        .map(|p| p.rect.y + p.rect.height)
        .fold(0.0_f64, f64::max)
        .ceil() as u32;

    let refs2: Vec<(f64, f64, &Scene)> = placements.iter().map(|(x, y, s)| (*x, *y, s)).collect();
    let scene = compose_dashboard(f64::from(width), f64::from(height), &refs2, ink);

    let title = spec.meta.as_ref().and_then(|m| m.title.clone());
    Ok(Composed {
        scene,
        width,
        height,
        title,
        plots,
        params: param_controls(spec),
        intervals: interval_controls(spec),
        // Attached by the load path, which is the only place that holds the
        // ParseOutput. `compose_from_results` is also reached on every
        // re-present after an interaction, where re-deriving diagnostics from
        // the spec alone would silently lose the parse warnings.
        diagnostics: LoadDiagnostics::default(),
        // Live-queried this very composition — no materialised run output is
        // being previewed, so no currency claim is made (or owed). A caller
        // previewing run output annotates with `with_run_state`, ingesting
        // from the run's contract.
        run_state: None,
        mark_faults,
        mode,
        // Attached by the caller with `with_row_count`, at the call site
        // that still holds the `Session` a count is read from —
        // `compose_from_results` runs after `execute_all`, over batches
        // already fetched, and has no session to query.
        rows: None,
    })
}

/// The `(ghost, subset)` mark indices of the first ghost/subset device this
/// spec's marks contain, `None` when there is no such pair.
///
/// **This is the device three of the generated tiles draw** —
/// [`crate::dashboard::histogram_tile`], [`crate::chart_kinds::point_map_tile`]
/// and the scatter's own tile: two adjacent marks over the SAME `from:`
/// source, the first with no `filterBy:` (the ghost, the whole table, does
/// not narrow) and the second `filterBy:`-bound to a selection (the subset,
/// what a brush leaves). The other generated tiles
/// (`crate::dashboard::time_bars_tile`, `crate::ranked_bars::RankedCategoryBars`)
/// use one mark and `highlight` instead, so a dashboard built from just
/// those tiles gets no row count from this seam. That shape is what this
/// reads back,
/// rather than re-deriving it from [`crate::dashboard::SELECTION`] by name: a
/// hand-authored spec can name its selection whatever it likes, and the
/// pairing the status band needs is structural, not nominal.
///
/// The FIRST such pair, because every tile a generated dashboard writes reads
/// the one opened table through the one shared crossfilter selection — a
/// brush on any tile narrows the same predicate, so any one tile's pair
/// answers for the whole document. A hand-authored spec with several
/// unrelated sources is not this card's scope; see the card for why one
/// figure is what the band owes.
pub(crate) fn ghost_subset_marks(spec: &Spec) -> Option<(usize, usize)> {
    let marks = collect_marks(spec);
    for i in 0..marks.len().checked_sub(1)? {
        let (
            Some(MarkData::From {
                source: ghost_source,
                filter_by: None,
                ..
            }),
            Some(MarkData::From {
                source: subset_source,
                filter_by: Some(_),
                ..
            }),
        ) = (&marks[i].data, &marks[i + 1].data)
        else {
            continue;
        };
        if ghost_source == subset_source {
            return Some((i, i + 1));
        }
    }
    None
}

/// **The mark index a surface that is not a plot reads the presenting step's
/// rows at** — the layer that carries `filterBy:`, resolved from the composed
/// spec rather than written down as a literal.
///
/// The subset mark of the first ghost/subset device `ghost_subset_marks`
/// finds, and mark `0` for a spec that declares no such device. A one-mark spec
/// therefore resolves to its only mark, and a hand-authored single layer bound
/// `filterBy:` resolves to itself — in both cases because there is no ghost to
/// pass over, not because `0` is a safe default.
///
/// # Why a literal was wrong, and why the obvious repair is also wrong
///
/// A generated tile writes its ghost first and its subset second — see
/// [`crate::chart_kinds::point_map_tile`] and the histogram and scatter tiles
/// beside it — so mark `0` of a generated dashboard is the hero's **ghost** —
/// `data: { from: opened }` and no `filterBy:`. A surface reading it
/// reads the whole table whatever anybody brushes, and that is what the rows
/// pane did: it listed 240 of 240 rows beside a status band saying 45 were
/// selected.
///
/// Moving to the subset mark is necessary and not sufficient, and this is the
/// half that is easy to miss. The generated selection is `select: crossfilter`,
/// under which every consumer drops the clause its own plot published, so the
/// hero's subset layer answers 240 under a brush **on the hero** — measured, at
/// the fixture in `tests/canvas_pane_group.rs`: after one interval on
/// `longitude` the sixteen marks count
/// `[240, 240, 240, 45, 240, 45, 240, 45, 240, 45, 240, 45, 240, 45, 240, 45]`,
/// and marks 0 and 1 are the hero's pair. So the mark index answers *which
/// materialisation*, and
/// [`RowsAudience`] answers *whose clause is
/// dropped*; a reader needs both and passes `Reader` for the second.
#[must_use]
pub fn presenting_rows_mark(spec: &Spec) -> usize {
    ghost_subset_marks(spec).map_or(0, |(_ghost, subset)| subset)
}

/// The status band's row count, read off `session` under its CURRENT
/// interaction state — the one query this seam owes, at the ghost/subset
/// device [`ghost_subset_marks`] finds.
///
/// Both figures ride [`brightfield_engine::Session::step_rows_count`], the
/// same `count(*)`-over-the-emitted-SQL seam the data grid sizes its scroll
/// range with: `count(*)` inside DuckDB, not a materialised batch counted on
/// this side — `computing_the_row_count_fetches_no_full_table_result` is the
/// test that holds that. The subset mark's own `filterBy:` is what makes its
/// count move under a brush — the SAME predicate the mark's own query is
/// filtered by, not a second compilation of it — and the ghost mark declares
/// no `filterBy:` at all, so its count is the table's total regardless of
/// what is currently held.
///
/// `None` when the spec has no such device, or when either count fails (the
/// mark's source has since vanished, say) — a band that cannot ask the engine
/// stays quiet rather than say something it did not check.
fn compute_row_count(session: &Session, spec: &Spec) -> Option<RowCount> {
    let (ghost, subset) = ghost_subset_marks(spec)?;
    let total = session.step_rows_count(ghost, RowsAudience::Reader).ok()?;
    let selected = session.step_rows_count(subset, RowsAudience::Reader).ok()?;
    Some(RowCount { selected, total })
}

/// The spec's slider-backed scalar params: every `input: slider` widget bound
/// `as: $param` whose param currently holds a number, with the widget's own
/// `min:`/`max:`/`step:` range. Read off the spec, never invented — a rail
/// slider over a range the spec did not declare would be a control whose ends
/// mean nothing.
fn param_controls(spec: &Spec) -> Vec<ParamControl> {
    use brightfield_spec::ast::{Component, Input, ParamNode, SpecValue, ValueOrParamRef};
    use brightfield_spec::vocab::InputKind;

    fn numeric(v: &SpecValue) -> Option<f64> {
        match v {
            SpecValue::Integer(i) => Some(*i as f64),
            SpecValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    fn collect<'s>(component: &'s Component, out: &mut Vec<&'s Input>) {
        match component {
            Component::Input(input) => out.push(input),
            Component::Plot(node) => {
                for item in &node.items {
                    collect(item, out);
                }
            }
            Component::HConcat(node) | Component::VConcat(node) => {
                for item in &node.items {
                    collect(item, out);
                }
            }
            _ => {}
        }
    }

    let mut inputs = Vec::new();
    if let Some(root) = &spec.root {
        collect(root, &mut inputs);
    }

    let mut out = Vec::new();
    for input in inputs {
        if input.kind != InputKind::Slider {
            continue;
        }
        let Some(param) = &input.as_param else {
            continue;
        };
        let name = param.0.clone();
        let Some(ParamNode::Value(value)) = spec.params.get(&name) else {
            continue;
        };
        let Some(value) = numeric(value) else {
            continue;
        };
        let option = |key: &str| match input.options.get(key) {
            Some(ValueOrParamRef::Value(v)) => numeric(v),
            _ => None,
        };
        out.push(ParamControl {
            name,
            value,
            min: option("min").unwrap_or(0.0),
            max: option("max").unwrap_or(1.0),
            step: option("step"),
        });
    }
    out
}

/// The spec's interval sliders as the rail's control type.
///
/// A thin map over [`brightfield_spec::analysis::build_interval_sliders`],
/// which owns the walk and the acceptance rule. It used to own them here, and
/// the cost was that the spec layer could not see a slider's `column:` — so
/// the cross-filter column check, which is derived from plot channels, never
/// looked at it and a typo'd column reached the user as a chart that never
/// filtered. One collector, one rule, and both consumers read it.
///
/// The warnings for nodes that asked for a slider and will not get one are
/// raised by `analyse_spec` (they ride to the surface on the composition's
/// diagnostics), so the sink here is correct: this call re-walks for the
/// widgets, not for the diagnosis.
fn interval_controls(spec: &Spec) -> Vec<IntervalControl> {
    let mut sink = Vec::new();
    brightfield_spec::analysis::build_interval_sliders(spec, &mut sink)
        .into_iter()
        .map(|s| IntervalControl {
            selection: s.selection,
            column: s.column,
            contributor: s.path,
            label: s.label,
            min: s.min,
            max: s.max,
            step: s.step,
            value: s.value,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_engine::SqlPredicate;
    use brightfield_spec::analysis::ComponentPath;

    const SPEC: &str = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;

    #[test]
    fn live_dashboard_holds_session_and_re_queries_on_interaction() {
        // The seam at the presentation layer: the session is held across frames
        // and a brush resolves to a pushed predicate + a re-composite, rather
        // than the one-shot compose_spec path that drops the session.
        let mut dash = LiveDashboard::load_str(SPEC, None).expect("load");
        let first = dash.present().expect("first paint");
        assert!(first.width > 0 && first.height > 0, "first paint has area");
        assert_eq!(dash.coordinator().generation(), 0);

        let after = dash
            .apply(Interaction::Select {
                name: "brush".to_string(),
                contributor: ComponentPath("root/plot[99]".to_string()),
                predicate: SqlPredicate::Expr("x > 2".to_string()),
            })
            .expect("re-paint after brush");
        assert!(after.width > 0 && after.height > 0, "re-paint has area");
        assert_eq!(
            dash.coordinator().generation(),
            1,
            "the interaction advanced the materialisation generation"
        );

        // The re-composite drew from a DuckDB-filtered batch: 2 rows kept, and
        // no Rust-side path filtered a materialised batch.
        let rows: usize = dash
            .coordinator()
            .chart_rows(0)
            .expect("chart rows")
            .iter()
            .map(RecordBatch::num_rows)
            .sum();
        assert_eq!(rows, 2, "brush kept x in {{3,4}} via a pushed predicate");
    }

    /// The same seam, driven with the STRUCTURED clause the chart gestures
    /// prefer: a `Predicate::Interval` keeps exactly the rows its hand-written
    /// string form would — the variants render byte-identical SQL — while the
    /// column and bounds stay machine-readable end to end.
    #[test]
    fn a_structured_interval_selects_the_same_rows_as_its_string_form() {
        use brightfield_sql::ir::ScalarValue;
        let mut dash = LiveDashboard::load_str(SPEC, None).expect("load");
        let _ = dash.present().expect("first paint");

        let interval = SqlPredicate::Interval {
            column: "x".to_string(),
            lo: ScalarValue::Float(2.0),
            hi: ScalarValue::Float(3.0),
            meta: None,
        };
        assert_eq!(
            interval.to_string(),
            "(x >= 2 AND x <= 3)",
            "the structured clause renders exactly the string form"
        );
        let after = dash
            .apply(Interaction::Select {
                name: "brush".to_string(),
                contributor: ComponentPath("root/plot[99]".to_string()),
                predicate: interval,
            })
            .expect("re-paint after structured brush");
        assert!(after.width > 0 && after.height > 0);
        let rows: usize = dash
            .coordinator()
            .chart_rows(0)
            .expect("chart rows")
            .iter()
            .map(RecordBatch::num_rows)
            .sum();
        assert_eq!(rows, 2, "the interval kept x in {{2,3}} in DuckDB");
    }

    /// A live-queried dashboard makes no currency claim — its queries ran for
    /// this very composition, so there is no materialised run output whose
    /// staleness could be misrepresented, and no banner is owed.
    #[test]
    fn a_live_queried_preview_makes_no_run_state_claim() {
        let mut dash = LiveDashboard::load_str(SPEC, None).expect("load");
        let composed = dash.present().expect("paint");
        assert_eq!(composed.run_state, None);
        assert_eq!(composed.run_state_line(), None);
        assert_eq!(Composed::empty().run_state, None, "empty claims nothing");
    }

    /// A preview annotated with run output's state renders that state's own
    /// words: a stale annotation can never produce the fresh line, and a
    /// never-run annotation is not the fresh line either — the preview
    /// surface cannot show materialised data as though it were current.
    #[test]
    fn an_annotated_preview_is_labelled_not_merely_rendered() {
        let stale = Composed::empty().with_run_state(RunState::StaleUpstream);
        let line = stale.run_state_line().expect("an annotated preview labels");
        assert!(line.contains("stale"), "the stale line says stale: {line}");
        assert!(
            !stale.run_state.expect("annotated").is_current(),
            "a stale preview may never claim current"
        );

        let fresh_line = Composed::empty()
            .with_run_state(RunState::Fresh)
            .run_state_line()
            .expect("labelled");
        let never_line = Composed::empty()
            .with_run_state(RunState::NeverRun)
            .run_state_line()
            .expect("labelled");
        assert_ne!(line, fresh_line, "stale and fresh are different words");
        assert_ne!(
            never_line, fresh_line,
            "never-run is visibly distinct from fresh"
        );
    }

    // --- Drawing every materialised chunk (the row-per-mark fix) -----------

    /// A one-mark dot spec whose inline data is irrelevant — these tests feed
    /// the execution results directly, so `compose_from_results` never queries.
    const DOT_SPEC: &str = r#"
data:
  t:
    - { x: 0, y: 0 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;

    fn xy_batch(xs: Vec<f64>, ys: Vec<f64>) -> RecordBatch {
        use arrow::array::Float64Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;
        let schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Float64Array::from(xs)),
                Arc::new(Float64Array::from(ys)),
            ],
        )
        .unwrap()
    }

    /// Two chunks that, between them, span more than one DuckDB vector
    /// (>2048 rows). The FIRST chunk already carries the domain endpoints (0
    /// and 100 on both axes), so the inferred scales — and therefore the grid
    /// and axis ink — are byte-identical whether the composite sees only the
    /// first chunk or both. That pins every non-mark path constant, so a
    /// path-count DIFFERENCE between the two composites is exactly the extra
    /// dots drawn.
    fn two_chunk_dot_results(
        first_rows: usize,
        second_rows: usize,
    ) -> (Vec<Result<Vec<RecordBatch>, EngineError>>, usize) {
        let first_x: Vec<f64> = (0..first_rows).map(|i| (i % 101) as f64).collect();
        let first_y: Vec<f64> = (0..first_rows).map(|i| (i % 101) as f64).collect();
        let first = xy_batch(first_x, first_y);
        // Second chunk stays strictly inside [0, 100] so it cannot widen the
        // domain — every value is the midpoint.
        let second = xy_batch(vec![50.0; second_rows], vec![50.0; second_rows]);
        let total = first_rows + second_rows;
        (vec![Ok(vec![first, second])], total)
    }

    /// A row-per-mark chart over more than one ~2048-row chunk draws EVERY
    /// materialised row, not just the first chunk. Proven two ways: the whole
    /// materialisation composites to strictly more mark ink than the first
    /// chunk alone, by exactly the dropped-row count; and chunking is
    /// invisible — one 3052-row batch draws identically to 2000 + 1052.
    #[test]
    fn a_chart_over_2048_rows_draws_every_materialised_row() {
        use brightfield_render::mark::count_scene_paths;
        let spec = parse_spec(DOT_SPEC, Format::Yaml).expect("parse").spec;

        let first_rows = 2000;
        let second_rows = 1052;
        let materialised = first_rows + second_rows;
        assert!(
            materialised > 2048,
            "the fixture must exceed one DuckDB vector"
        );

        // The whole materialisation vs the first chunk only (what the old code
        // drew). Domains are pinned equal, so the frame ink is identical and
        // the path-count delta is purely the extra dots.
        let (full_results, total) = two_chunk_dot_results(first_rows, second_rows);
        assert_eq!(total, materialised);
        let full = compose_from_results(
            &spec,
            full_results,
            &[],
            &ViewExtents::new(),
            &[],
            &mut PlotPins::new(),
            Rect::zero(),
            Mode::Light,
            None,
        )
        .expect("compose full");

        let first_only: Vec<Result<Vec<RecordBatch>, EngineError>> = vec![Ok(vec![xy_batch(
            (0..first_rows).map(|i| (i % 101) as f64).collect(),
            (0..first_rows).map(|i| (i % 101) as f64).collect(),
        )])];
        let first = compose_from_results(
            &spec,
            first_only,
            &[],
            &ViewExtents::new(),
            &[],
            &mut PlotPins::new(),
            Rect::zero(),
            Mode::Light,
            None,
        )
        .expect("compose first");

        let drawn_delta = count_scene_paths(&full.scene) - count_scene_paths(&first.scene);
        assert_eq!(
            drawn_delta, second_rows,
            "the composite drew exactly the rows the first-chunk cap used to drop"
        );

        // Chunking is invisible: one batch of all rows draws the same as two.
        let single: Vec<Result<Vec<RecordBatch>, EngineError>> = vec![Ok(vec![xy_batch(
            (0..materialised).map(|i| (i % 101) as f64).collect(),
            (0..materialised).map(|i| (i % 101) as f64).collect(),
        )])];
        let single = compose_from_results(
            &spec,
            single,
            &[],
            &ViewExtents::new(),
            &[],
            &mut PlotPins::new(),
            Rect::zero(),
            Mode::Light,
            None,
        )
        .expect("compose single");
        assert_eq!(
            count_scene_paths(&full.scene),
            count_scene_paths(&single.scene),
            "a chart draws the same whether its rows arrive in one chunk or many"
        );
    }

    /// Hitting a real assembly limit — a mark's chunks whose schemas disagree —
    /// fails the compose loudly, by NAME, naming the mark, instead of silently
    /// drawing only the first chunk.
    #[test]
    fn an_unassemblable_mark_fails_loudly_by_name() {
        use arrow::array::{Float64Array, Int64Array};
        use arrow::datatypes::{DataType, Field, Schema};
        use std::sync::Arc;
        let parsed = parse_spec(DOT_SPEC, Format::Yaml).expect("parse");
        let spec = parsed.spec;

        let int_schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Int64, false),
            Field::new("y", DataType::Int64, false),
        ]));
        let a = RecordBatch::try_new(
            int_schema,
            vec![
                Arc::new(Int64Array::from(vec![1_i64, 2])),
                Arc::new(Int64Array::from(vec![1_i64, 2])),
            ],
        )
        .unwrap();
        // Same column names, a different x type — the chunks cannot concatenate.
        let drift_schema = Arc::new(Schema::new(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Int64, false),
        ]));
        let b = RecordBatch::try_new(
            drift_schema,
            vec![
                Arc::new(Float64Array::from(vec![3.0_f64])),
                Arc::new(Int64Array::from(vec![3_i64])),
            ],
        )
        .unwrap();

        let results: Vec<Result<Vec<RecordBatch>, EngineError>> = vec![Ok(vec![a, b])];
        // `.err()` rather than `.expect_err()` — `Composed` is deliberately not
        // `Debug` (it holds a Vello scene), so we inspect the error side directly.
        let err = compose_from_results(
            &spec,
            results,
            &[],
            &ViewExtents::new(),
            &[],
            &mut PlotPins::new(),
            Rect::zero(),
            Mode::Light,
            None,
        )
        .err()
        .expect("must fail loudly");
        assert!(err.contains("mark 0"), "the failure names the mark: {err}");
        assert!(
            err.contains("batch-assembly limit"),
            "the failure names the limit: {err}"
        );
    }

    // -----------------------------------------------------------------
    // The status band's row count — the ghost/subset device
    // -----------------------------------------------------------------

    /// A minimal ghost/subset device over ten known rows: mark 0 the ghost
    /// (`data: { from: t }`, no `filterBy:`), mark 1 the subset
    /// (`filterBy: $sel`) — the shape `dashboard::histogram_tile` and
    /// `chart_kinds::point_map_tile` write, over a table small enough to
    /// hand-count.
    const ROW_COUNT_DEVICE: &str = r#"
params:
  sel:
    select: intersect
data:
  t:
    - { x: 1 }
    - { x: 2 }
    - { x: 3 }
    - { x: 4 }
    - { x: 5 }
    - { x: 6 }
    - { x: 7 }
    - { x: 8 }
    - { x: 9 }
    - { x: 10 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: x
  - mark: dot
    data: { from: t, filterBy: $sel }
    x: x
    y: x
"#;

    /// [`ghost_subset_marks`] finds the pair by SHAPE — no `filterBy:` beside
    /// a `filterBy:` over the same source, adjacent in mark order — not by
    /// the selection's name, so a spec naming its selection whatever it
    /// likes still earns a row count.
    #[test]
    fn ghost_subset_marks_finds_the_pair_by_shape_not_by_name() {
        let spec = parse_spec(ROW_COUNT_DEVICE, Format::Yaml)
            .expect("parse")
            .spec;
        assert_eq!(ghost_subset_marks(&spec), Some((0, 1)));
    }

    /// A spec with one layer and no `filterBy:` anywhere has no predicate
    /// seam for the band to read a count off — `None`, not a guess.
    #[test]
    fn a_spec_with_no_filter_by_mark_has_no_row_count_device() {
        let spec = parse_spec(DOT_SPEC, Format::Yaml).expect("parse").spec;
        assert_eq!(ghost_subset_marks(&spec), None);
    }

    /// AC1: a freshly opened document with a ghost/subset device reads its
    /// row count straight off the engine — selected equal to total, because
    /// nobody has brushed yet, over a fixture with no file metadata at all
    /// (the rows are inline literals) to prove the number cannot have come
    /// from one.
    #[test]
    fn a_freshly_opened_document_reads_selected_equal_to_total() {
        let mut dash = LiveDashboard::load_str(ROW_COUNT_DEVICE, None).expect("load");
        let composed = dash.present().expect("first paint");
        assert_eq!(
            composed.rows,
            Some(RowCount {
                selected: 10,
                total: 10
            }),
            "ten inline rows, nobody has brushed"
        );
    }

    /// AC2: a brush moves the selected figure to exactly the compiled
    /// predicate's `COUNT(*)` and leaves the total where it was — proof that
    /// the total is the table's own count, not re-derived under the brush.
    #[test]
    fn a_brush_moves_selected_to_the_compiled_predicates_count_and_leaves_total() {
        let mut dash = LiveDashboard::load_str(ROW_COUNT_DEVICE, None).expect("load");
        let _ = dash.present().expect("first paint");

        let after = dash
            .apply(Interaction::Select {
                name: "sel".to_string(),
                contributor: ComponentPath("root/plot[99]".to_string()),
                predicate: SqlPredicate::Expr("x > 5".to_string()),
            })
            .expect("re-paint after brush");
        assert_eq!(
            after.rows,
            Some(RowCount {
                selected: 5,
                total: 10
            }),
            "x > 5 admits {{6,7,8,9,10}} — five of the table's ten rows — and \
             the total must not move under a brush that never touches it"
        );
    }

    /// The same device under `select: crossfilter`, brushed from **its own
    /// plot** — the shape every generated dashboard writes, and the one the
    /// `intersect` fixture above cannot see.
    ///
    /// Under crossfilter a consumer drops the clause its own plot published, so
    /// asking the subset layer as the plot answers ten under this brush: the
    /// figure a status band would report is `10 of 10 rows` beside a chart in
    /// which everything outside the brush has gone grey. That was live until
    /// this round. `compute_row_count` asks as a [`RowsAudience::Reader`] — no
    /// plot, so no contribution of its own to drop — and the first assertion
    /// here holds the fixture's self-exclusion so this cannot go green by the
    /// selection quietly ceasing to be a crossfilter.
    #[test]
    fn a_crossfilter_brush_from_the_plot_itself_still_moves_the_selected_figure() {
        let src = ROW_COUNT_DEVICE.replace("select: intersect", "select: crossfilter");
        let mut dash = LiveDashboard::load_str(&src, None).expect("load");
        let composed = dash.present().expect("first paint");
        let plot = ComponentPath(composed.plots[0].path.clone());

        let after = dash
            .apply(Interaction::Select {
                name: "sel".to_string(),
                contributor: plot,
                predicate: SqlPredicate::Expr("x > 5".to_string()),
            })
            .expect("re-paint after brush");

        let spec = parse_spec(&src, Format::Yaml).expect("parse").spec;
        let (_, subset) = ghost_subset_marks(&spec).expect("the device");
        assert_eq!(
            dash.coordinator()
                .session()
                .step_rows_count(subset, RowsAudience::Plot)
                .expect("subset as the plot"),
            10,
            "the subset layer asked as its own plot must still drop this \
             plot's clause, or the fixture is no longer a crossfilter and \
             nothing below is a claim about one"
        );
        assert_eq!(
            after.rows,
            Some(RowCount {
                selected: 5,
                total: 10
            }),
            "the band reports the selection's own count — reading the subset \
             layer as its plot would report ten of ten under this brush"
        );
    }

    /// AC3: the count is issued to the engine and not computed from a batch
    /// this side already fetched. `Session::duckdb_execute_count` advances on
    /// the cached mark-execution path (`execute_mark`/`execute_emitted`) and
    /// not on the raw query path `step_rows_count` rides (see its own doc),
    /// so reading the row count off a session that has executed no mark yet
    /// leaves that counter at zero — which this test asserts directly, at the
    /// crosswalk's own magnitude (207,099 rows, `crosswalk_chart.rs`'s
    /// `CROSSWALK_ROWS`), where a client-side count would mean fetching and
    /// counting that many rows rather than asking DuckDB for one number.
    #[test]
    fn computing_the_row_count_fetches_no_full_table_result() {
        let spec_src = "params:\n  sel:\n    select: intersect\ndata:\n  t:\n    query: \
             \"SELECT i AS x FROM range(207099) t(i)\"\nplot:\n  - mark: dot\n    \
             data: { from: t }\n    x: x\n    y: x\n  - mark: dot\n    data: \
             { from: t, filterBy: $sel }\n    x: x\n    y: x\n";
        let parsed = parse_spec(spec_src, Format::Yaml).expect("parse").spec;
        let analysis = analyse_spec(&parsed).expect("analyse");
        let session = Engine::new()
            .load_spec(parsed.clone(), analysis, None)
            .expect("load")
            .session;
        assert_eq!(session.duckdb_execute_count(), 0, "nothing executed yet");

        let rows = compute_row_count(&session, &parsed).expect("device found");
        assert_eq!(rows.total, 207_099);
        assert_eq!(rows.selected, 207_099, "nothing brushed yet");
        assert_eq!(
            session.duckdb_execute_count(),
            0,
            "the row count must ride the raw count(*) path — a materialised \
             fetch of any of the 207,099 rows would show up here"
        );
    }
}
