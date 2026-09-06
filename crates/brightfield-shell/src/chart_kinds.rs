//! The shell's chart vocabulary, as data — **the registry the running binary
//! reads**, not a fixture.
//!
//! [`brightfield_workbench::registry::ChartKind`] makes a chart kind a value:
//! an icon, a gloss, the slots it takes and a builder that turns bound columns
//! into a spec. [`registry`] is the instance this **process** reads, as opposed
//! to one a test stands up: [`crate::data_file`] chooses a first look out of it
//! and emits that kind's spec, and [`crate::app::ChartDoc`] hands it to the
//! chart pane through [`brightfield_workbench::item::ModuleHost`]. Both routes
//! are held by tests that take a kind away and watch the outcome change.
//!
//! # What a kind's spec *is* here
//!
//! Spec **source** — the body of a Brightfield YAML document, ready to have a
//! `meta:` and a `data:` block written above it. `String` is the spec type for
//! the reason [`crate::ranked_bars::chart_kind`] already chose it: the thing a
//! chart kind produces in this shell is a document the composer parses, and a
//! structured intermediate would be a second spec language to keep in step with
//! the first.
//!
//! Three consequences, and each is a contract a new kind has to keep:
//!
//! - the source is a **self-contained top-level fragment**: the picture's
//!   `plot:` or `hconcat:` key, plus whatever else that picture's instructions
//!   need declared beside them at the top level — a `params:` entry for a
//!   selection its interactors bind, say. So the caller can concatenate it
//!   under a `data:` block without knowing which kind built it;
//! - it **loads clean**: the composed document's diagnostics carry nothing
//!   advisory, because the window raises those as a banner over the picture. A
//!   block whose interactor binds an undeclared param draws a chart and tells
//!   the reader, in the same frame, that one of its instructions had no effect.
//!   `no_kinds_block_asks_for_something_the_load_says_had_no_effect` is what
//!   says so;
//! - it reads the source named [`crate::data_file::SOURCE`], because this
//!   registry's kinds are the ones the shell offers over the **one** table it
//!   synthesises a document for. A kind wanting a different table takes its
//!   own emitter, the way [`crate::ranked_bars::RankedCategoryBars::plot_yaml`]
//!   does.
//!
//! # Declaration order is the preference order
//!
//! [`ChartKindRegistry::applicable`] answers in declaration order, and a caller
//! with no opinion takes the first. So the order below is the product
//! judgement about what a table nobody has described should open as, stated
//! once, where the kinds are.

use std::fmt::Write as _;
use std::sync::OnceLock;

use brightfield_engine::ColumnProfile;
use brightfield_workbench::registry::{
    ChartKind, ChartKindId, ChartKindRegistry, Field, FieldSlot, FieldType,
};
use brightfield_workbench::Icon;

use crate::data_file::SOURCE;

/// A numeric column's distribution: `rectY` over its bin edges, counted.
pub const BINNED_HISTOGRAM: ChartKindId = ChartKindId::new("binned-histogram");
/// A dated column's rows counted per day, in time order: `barY` over a band of
/// days.
pub const COUNTS_OVER_TIME: ChartKindId = ChartKindId::new("counts-over-time");
/// Two categories crossed and counted: `cell` over a pair of band axes.
pub const COUNT_GRID: ChartKindId = ChartKindId::new("count-grid");
/// Two measures related: `dot` over a pair of quantitative axes.
pub const SCATTER: ChartKindId = ChartKindId::new("scatter");
/// A coordinate pair plotted as points on an equal-aspect frame: `dot` over
/// longitude and latitude, with `aspectRatio: 1` asking one px-per-unit of
/// both axes rather than each fitting its own domain to the tile independently.
pub const POINT_MAP: ChartKindId = ChartKindId::new("point-map");

/// The widest **category** axis this registry will cross: a `distinct ×
/// distinct` grid past this on either side is a wall of cells rather than a
/// picture.
///
/// A property of the **field list**, not of a slot — a slot declares types, and
/// "too many categories to read" is a cardinality. So it is applied by
/// [`fields_of`], which is where the column profiles are.
///
/// The number was argued for over `distinct × distinct`, and [`fields_of`]
/// applies it to the **categories** — which includes the one-axis
/// [`crate::ranked_bars`], whose bars are as unreadable past it as a grid's
/// cells are. A lone temporal column is not a category and does not meet it:
/// a year of daily readings is not a wall of cells but the ordinary shape of a
/// time series, and applying the ceiling to it is what refused a date its
/// picture. See [`counts_over_time`], which takes its column at any width.
const GRID_MAX_DISTINCT: u64 = 60;

/// The one column a histogram bins.
///
/// A single required slot, so [`ChartKind::accepts`] answers yes for any table
/// carrying a measure and no for a table of names — the applicability rule the
/// first-look chooser needs, as data rather than as a branch.
const HISTOGRAM_SLOTS: &[FieldSlot] = &[FieldSlot::required("x", &[FieldType::Quantitative])];

/// The two measures a scatter relates, x before y.
///
/// Both required, so [`ChartKind::accepts`] answers yes for a table carrying a
/// second measure and no for a table carrying one — a lone measure has a
/// distribution and no relationship, and the test over that refusal is
/// `a_table_with_one_measure_admits_no_scatter` in
/// `crates/brightfield-shell/tests/scatter_kind.rs`.
///
/// # Which column fills which slot
///
/// [`ChartKind::bind`] is first fit in slot order and [`fields_of`] hands the
/// measures over first, in the table's own order. So **x is the table's first
/// measure and y is its second** — the same ordering rule [`fields_of`] already
/// documents for the one-slot kinds, applied to two slots rather than to one,
/// which is why it is not a second rule for a reader to learn. No name, range or
/// correlation is consulted; `a_scatters_axes_are_the_tables_first_two_measures`
/// is the test, and it reorders the columns to show the rule is the order rather
/// than the names.
const SCATTER_SLOTS: &[FieldSlot] = &[
    FieldSlot::required("x", &[FieldType::Quantitative]),
    FieldSlot::required("y", &[FieldType::Quantitative]),
];

/// The two measures a point map plots, longitude before latitude.
///
/// Both required, for the reason [`SCATTER_SLOTS`] gives: one column alone has
/// no partner to pair it with. Unlike the scatter's `x`/`y`, **which column
/// fills which slot is not the table's own order** — a point map is built
/// from [`crate::dashboard::coordinate_pair`] and nowhere else, which names
/// the longitude and the latitude explicitly and binds them in that order, so
/// `bind`'s first-fit is not left to guess between two otherwise-identical
/// quantitative fields.
const POINT_MAP_SLOTS: &[FieldSlot] = &[
    FieldSlot::required("lon", &[FieldType::Quantitative]),
    FieldSlot::required("lat", &[FieldType::Quantitative]),
];

/// The ink the ghost layer is drawn in — the warm-gray border step of the
/// design system's generated gray scale,
/// [`meridian_design::scales::GRAY_LIGHT`].
///
/// A token rather than a hex constant, so a palette regeneration moves the
/// ghost with the rest of the chart's ink instead of leaving it behind. The
/// emitter spells it out with [`meridian_design::colour::Rgba::hex`], which
/// round-trips the scale's own 8-bit channels exactly — so the colour reaching
/// the canvas is the token's, not an approximation of it.
///
/// **This step rather than a lighter one**, and a pixel test is what decides
/// it: the reading in `crates/brightfield-shell/tests/ghosted_histogram.rs`
/// tells ghost ink from chart chrome by per-channel distance, and the plot
/// frame's own baseline is drawn from a step of this same scale. A ghost close
/// enough to that step to sit inside the tolerance would turn the reading into
/// a reading of the gridlines, so
/// `the_registrys_ghost_ink_is_not_the_charts_own_chrome` holds the separation
/// rather than this comment asserting it.
const GHOST_INK: meridian_design::colour::Rgba = meridian_design::scales::GRAY_LIGHT[7];

/// The crossfilter selection this registry's blocks drive and read.
///
/// **One name for the kinds that declare one**, and that is the point of putting
/// it here rather than in each builder: two blocks composed into one document
/// cross-filter each other only while they name the same selection. Two private
/// names would compose into a dashboard whose tiles each filtered nothing but
/// themselves — and self-exclusion means that is a dashboard where brushing does
/// nothing at all.
///
/// The name is arbitrary and the **declaration** is not: a block writing `as:
/// $sel` on an interactor has to declare `sel` under `params:`, because an
/// interactor binding a name no `params:` entry declares raises
/// [`brightfield_spec::ParseWarning::InteractorBindingMissing`] — which the
/// window puts on screen as a *"had no effect"* banner over the picture it has
/// just drawn. [`crate::ranked_bars::Dashboard::to_spec`] declares the same
/// entry for the same reason.
const SELECTION: &str = "sel";

/// The two axes a count grid crosses. Both required: one category is a bar
/// chart, not a grid.
const GRID_SLOTS: &[FieldSlot] = &[
    FieldSlot::required("x", &[FieldType::Categorical]),
    FieldSlot::required("y", &[FieldType::Categorical]),
];

/// The one column a time series counts along. A single required slot, so
/// [`ChartKind::accepts`] answers yes for a dated column and no for a category
/// — the tests are `a_table_of_names_crosses_its_two_narrowest_categories`,
/// where a date is the first look a table of names admits, and
/// `one_category_opens_on_ranked_bars`, whose exact list of applicable kinds
/// this one is absent from.
const TIME_SLOTS: &[FieldSlot] = &[FieldSlot::required("t", &[FieldType::Temporal])];

/// The shell's chart kinds, in preference order.
///
/// Built once per process. A `OnceLock` rather than a `const`: a
/// [`ChartKind`]'s `controls` is a function and its description a `&'static
/// str`, but [`ChartKindRegistry::new`] takes a `Vec` and asserts ids are
/// unique — which is a run-time check, deliberately (see its docs), so the
/// registry is a run-time value.
#[must_use]
pub fn registry() -> &'static ChartKindRegistry<String> {
    static KINDS: OnceLock<ChartKindRegistry<String>> = OnceLock::new();
    KINDS.get_or_init(|| ChartKindRegistry::new(kinds()))
}

/// The kinds [`registry`] is built from, in the order it declares them.
///
/// A function rather than the `vec![]` written inline where the registry is
/// built, so a test can stand up the same list with one kind taken out and
/// compare the two answers. `adding_the_scatter_moved_no_first_look` is that
/// test, and building its comparison registry from this list is what stops it
/// going stale the next time a kind is added: a second hand-written copy of the
/// order would keep passing while the shipped order moved underneath it.
fn kinds() -> Vec<ChartKind<String>> {
    vec![
        binned_histogram(),
        scatter(),
        point_map(),
        counts_over_time(),
        count_grid(),
        ranked_category_bars(),
    ]
}

/// The kind registered for `id`, if this build has one.
#[must_use]
pub fn find(id: ChartKindId) -> Option<&'static ChartKind<String>> {
    registry().find(id)
}

/// A numeric column's distribution, **ghosted**: the unfiltered total behind
/// the filtered subset.
///
/// Two `rectY` layers over one table and one `x: { bin: }` / `y: { count: }`
/// transform — the lift
/// [`brightfield_spec::vocab::MarkKind::bins_positionally`] recognises, so the
/// aggregation happens in SQL and the picture is of the whole table rather than
/// of a sample of it. The first layer reads [`SOURCE`] straight and never
/// narrows; the second reads it through `filterBy:` the crossfilter selection
/// and lands on top in the default mark ink.
///
/// # Why two layers rather than one filtered one
///
/// Both layers share the plot's scales, so the count axis and the pixel mapping
/// are fixed by the total. A brushed tile therefore reads as a fraction of the
/// bars behind it. One filtered layer draws a perfectly good histogram after a
/// brush — right bars, right counts, rescaled axis — and gives the reader no
/// way to see what fraction of the data it is; it reads as a chart that redrew
/// itself. `examples/rect-bin-count-ghost.yaml` is the same device authored by
/// hand, and its header comment is the long form of this paragraph.
///
/// The alternative device is `select: highlight`, which draws the selected part
/// inside a single unfiltered bar (`examples/rect-bin-count-part-of-whole.yaml`).
/// It is not interchangeable with this one: it deemphasises non-matching rows
/// within one layer, so the ghost and the subset cannot be read as two
/// separately-scaled quantities.
///
/// # Why the plot also brushes
///
/// `select: intervalX` makes the tile a contributor to [`SELECTION`] and not
/// only a subscriber to it. A sweep resolves to an interval over the binned
/// column: `x: { bin: col }` draws on an axis in `col`'s own units, so a pixel
/// range on it inverts to a `col` range. `brightfield-spec`'s
/// `positional_column` is what reads that column out of the bin transform, and
/// `tests/ghosted_histogram.rs` drives a pointer sweep through the whole path
/// to the committed clause.
///
/// Without the interactor nothing in the document this kind composes can write
/// [`SELECTION`], so the second layer's `filterBy:` never narrows, the two
/// layers stay identical and the ghost is decoration.
///
/// A sweep here does not move this tile's own bars, and that is the design
/// rather than a dead control. Crossfilter self-exclusion drops a plot's own
/// clause from its own query, so the tile keeps its whole distribution while
/// whatever else subscribes to [`SELECTION`] narrows.
fn binned_histogram() -> ChartKind<String> {
    ChartKind {
        id: BINNED_HISTOGRAM,
        icon: Icon("chart-bar"),
        description:
            "Bins a measure and counts the rows in each bin, the total behind the selection",
        slots: HISTOGRAM_SLOTS,
        controls: Vec::new,
        build: |bound, _options| {
            let column = yaml_quoted(bound.name("x").unwrap_or_default());
            let mut out = String::from("params:\n");
            let _ = writeln!(out, "  {SELECTION}: {{ select: crossfilter }}");
            out.push_str("plot:\n");
            // The ghost, first so the subset covers it: the whole table, with
            // no `filterBy:` to narrow it.
            let _ = writeln!(out, "  - mark: rectY");
            let _ = writeln!(out, "    data: {{ from: {SOURCE} }}");
            let _ = writeln!(out, "    x: {{ bin: {column} }}");
            let _ = writeln!(out, "    y: {{ count: }}");
            let _ = writeln!(out, "    fill: \"{}\"", GHOST_INK.hex());
            // The subset: the same transform, through the selection, in the
            // mark ink a layer binding no colour channel takes.
            let _ = writeln!(out, "  - mark: rectY");
            let _ = writeln!(
                out,
                "    data: {{ from: {SOURCE}, filterBy: ${SELECTION} }}"
            );
            let _ = writeln!(out, "    x: {{ bin: {column} }}");
            let _ = writeln!(out, "    y: {{ count: }}");
            let _ = writeln!(out, "  - select: intervalX");
            let _ = writeln!(out, "    as: ${SELECTION}");
            out
        },
    }
}

/// Two measures related, **ghosted**: the unfiltered cloud behind the filtered
/// subset.
///
/// The device a scatter answers *does this move with that* with — two `dot`
/// layers over one table, `x:` naming one measure and `y:` the other, with an
/// `intervalXY` producer so a rectangle swept over the cloud narrows whatever
/// else subscribes to [`SELECTION`].
///
/// # Where it sits in the preference order, and why no file opens differently
///
/// Second, behind [`binned_histogram`]. That position is a preference — once a
/// numeric table's distribution has been offered, relating two of its measures
/// is a better second answer than crossing two of its categories — and it is
/// **not** what keeps an already-shipped file opening as it did.
///
/// What keeps that is the slots. `binned-histogram` takes one quantitative slot
/// and this kind takes two, so a field list this kind binds is one that kind
/// binds as well, and `binned-histogram` is declared ahead of it. A chooser
/// taking the first applicable kind therefore cannot reach a scatter from any
/// position after the histogram, whatever else is added later.
/// `adding_the_scatter_moved_no_first_look` is the test, and it states that as a
/// comparison against the registry with this kind removed rather than as a claim
/// about the five kinds shipped today.
///
/// The generated dashboard is a second route and is closed by the same
/// declaration: [`crate::dashboard::single_column_kinds`] admits a kind with
/// exactly one required slot, and this one has two.
///
/// # Why two layers rather than one filtered one
///
/// The argument [`binned_histogram`] makes for a count axis, in two dimensions.
/// Both layers share the plot's scales, so the extent of the cloud is fixed by
/// the whole table: a brushed scatter reads as a subset of the points behind it
/// instead of as a cloud that redrew itself at a new scale. A lone filtered
/// layer would re-derive both domains from the rows that survived, which moves
/// every remaining dot under the pointer and leaves nothing on the page to judge
/// the fraction against.
fn scatter() -> ChartKind<String> {
    ChartKind {
        id: SCATTER,
        icon: Icon("chart-dots"),
        description:
            "Relates two measures as a cloud of dots, the whole cloud behind the selection",
        slots: SCATTER_SLOTS,
        controls: Vec::new,
        build: |bound, _options| {
            let x = bound.name("x").unwrap_or_default();
            let y = bound.name("y").unwrap_or_default();
            let mut out = String::from("params:\n");
            let _ = writeln!(out, "  {SELECTION}: {{ select: crossfilter }}");
            out.push_str("hconcat:\n");
            out.push_str(&scatter_tile(x, y, 2));
            out
        },
    }
}

/// The scatter device as one entry of a concat list, indented by `indent`
/// spaces, over [`SOURCE`] and the private `SELECTION` this module declares.
///
/// **One emitter, published**, for the reason
/// [`crate::dashboard::histogram_tile`]'s own header gives about the device it
/// emits: a picture written out in two places is held in step by prose, and
/// prose does not redden. The kind's builder above wraps this body under its own
/// `params:` header, and a caller composing a scatter beside other tiles asks
/// for the same string — so there is no second copy to keep honest.
///
/// It lives here rather than beside the dashboard's two emitters because that
/// module keys a tile by `TileForm`, one per kind a **single column** can fill,
/// and this device takes two columns. It is a concat entry that no generated
/// dashboard emits.
#[must_use]
pub fn scatter_tile(x: &str, y: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let (xq, yq) = (yaml_quoted(x), yaml_quoted(y));
    let mut out = String::new();
    let _ = writeln!(out, "{pad}- plot:");
    // The ghost, first so the subset covers it: the whole table, with no
    // `filterBy:` to narrow it.
    let _ = writeln!(out, "{pad}  - mark: dot");
    let _ = writeln!(out, "{pad}    data: {{ from: {SOURCE} }}");
    let _ = writeln!(out, "{pad}    x: {xq}");
    let _ = writeln!(out, "{pad}    y: {yq}");
    let _ = writeln!(out, "{pad}    fill: \"{}\"", GHOST_INK.hex());
    // The subset: the same pair of columns, through the selection, in the mark
    // ink a layer binding no colour channel takes.
    let _ = writeln!(out, "{pad}  - mark: dot");
    let _ = writeln!(
        out,
        "{pad}    data: {{ from: {SOURCE}, filterBy: ${SELECTION} }}"
    );
    let _ = writeln!(out, "{pad}    x: {xq}");
    let _ = writeln!(out, "{pad}    y: {yq}");
    // The producer: a rectangle swept over the cloud publishes an interval on
    // each axis into the shared selection. `intervalXY` rather than two
    // one-dimensional producers because a scatter's answer is a region, and
    // both of its axes are continuous columns a pixel range inverts through.
    let _ = writeln!(out, "{pad}  - select: intervalXY");
    let _ = writeln!(out, "{pad}    as: ${SELECTION}");
    // Plot attributes are siblings of `plot:`, so they sit at its indent — one
    // level deeper and they read as more options on the last interactor, which
    // is a spec that parses and does something else.
    let _ = writeln!(out, "{pad}  xLabel: {xq}");
    let _ = writeln!(out, "{pad}  yLabel: {yq}");
    let _ = writeln!(out, "{pad}  width: {}", crate::dashboard::TILE_WIDTH);
    let _ = writeln!(out, "{pad}  height: {}", crate::dashboard::TILE_HEIGHT);
    out
}

/// A coordinate pair plotted as points, **ghosted and equal-aspect**: the
/// unfiltered cloud behind the filtered subset, on a frame where one
/// px-per-unit is shared by both axes.
///
/// [`scatter`] with two differences: the columns are longitude and latitude
/// rather than the table's first two measures, chosen by
/// [`crate::dashboard::coordinate_pair`] and not by this registry; and both
/// layers write `aspectRatio: 1`, which `brightfield_render::mark`'s
/// `DotRenderer` reads (through its `MarkRenderer::augment_scales`
/// implementation) to widen the narrower axis's domain until a degree of
/// longitude and a degree of latitude cover the same number of pixels — see
/// that implementation for why the fit needs no map projection: at this
/// scale a plate-carrée identity (`u = lon, v = lat`) and an equal-aspect
/// cartesian frame draw the same picture.
///
/// # Why two layers and a brush, same as the scatter
///
/// The device — ghost behind subset, `intervalXY` producing the shared
/// selection — is [`scatter_tile`]'s argument unchanged: both layers share the
/// plot's scales, so a brushed map reads as a subset of the points behind it,
/// and a rectangle swept over the cloud narrows whatever else subscribes to
/// [`SELECTION`]. `tests/point_map_kind.rs`'s gesture tier drives a real sweep
/// through `MeridianApp` the way `tests/scatter_kind.rs`'s does.
fn point_map() -> ChartKind<String> {
    ChartKind {
        id: POINT_MAP,
        icon: Icon("map-pin"),
        description:
            "Plots a coordinate pair on an equal-aspect map, the whole cloud behind the selection",
        slots: POINT_MAP_SLOTS,
        controls: Vec::new,
        build: |bound, _options| {
            let lon = bound.name("lon").unwrap_or_default();
            let lat = bound.name("lat").unwrap_or_default();
            let mut out = String::from("params:\n");
            let _ = writeln!(out, "  {SELECTION}: {{ select: crossfilter }}");
            out.push_str("hconcat:\n");
            out.push_str(&point_map_tile(lon, lat, 2));
            out
        },
    }
}

/// The point-map device as one entry of a concat list, indented by `indent`
/// spaces, over [`SOURCE`] and the private `SELECTION` this module declares.
///
/// **One emitter, published** — [`scatter_tile`]'s own header gives the reason:
/// the kind's builder above wraps this body under its own `params:` header,
/// and [`crate::dashboard`] asks for the same string when it draws the joint
/// tile a coordinate pair earns, so there is no second copy to keep honest.
#[must_use]
pub fn point_map_tile(lon: &str, lat: &str, indent: usize) -> String {
    point_map_tile_sized(
        lon,
        lat,
        indent,
        crate::dashboard::TILE_WIDTH,
        crate::dashboard::TILE_HEIGHT,
    )
}

/// [`point_map_tile`] at a declared size — the weight a constrained concat
/// shares its box out by. See [`crate::dashboard::HERO_WIDTH`].
#[must_use]
pub fn point_map_tile_sized(
    lon: &str,
    lat: &str,
    indent: usize,
    width: u32,
    height: u32,
) -> String {
    let pad = " ".repeat(indent);
    let (xq, yq) = (yaml_quoted(lon), yaml_quoted(lat));
    let mut out = String::new();
    let _ = writeln!(out, "{pad}- plot:");
    // The ghost, first so the subset covers it: the whole table, with no
    // `filterBy:` to narrow it.
    let _ = writeln!(out, "{pad}  - mark: dot");
    let _ = writeln!(out, "{pad}    data: {{ from: {SOURCE} }}");
    let _ = writeln!(out, "{pad}    x: {xq}");
    let _ = writeln!(out, "{pad}    y: {yq}");
    let _ = writeln!(out, "{pad}    fill: \"{}\"", GHOST_INK.hex());
    let _ = writeln!(out, "{pad}    aspectRatio: 1");
    // The subset: the same pair of columns, through the selection, in the mark
    // ink a layer binding no colour channel takes.
    let _ = writeln!(out, "{pad}  - mark: dot");
    let _ = writeln!(
        out,
        "{pad}    data: {{ from: {SOURCE}, filterBy: ${SELECTION} }}"
    );
    let _ = writeln!(out, "{pad}    x: {xq}");
    let _ = writeln!(out, "{pad}    y: {yq}");
    let _ = writeln!(out, "{pad}    aspectRatio: 1");
    // The producer: a rectangle swept over the cloud publishes an interval on
    // each axis into the shared selection — the same two-dimensional device
    // the scatter's own tile writes.
    let _ = writeln!(out, "{pad}  - select: intervalXY");
    let _ = writeln!(out, "{pad}    as: ${SELECTION}");
    // Plot attributes are siblings of `plot:`, so they sit at its indent — one
    // level deeper and they read as more options on the last interactor, which
    // is a spec that parses and does something else.
    let _ = writeln!(out, "{pad}  xLabel: {xq}");
    let _ = writeln!(out, "{pad}  yLabel: {yq}");
    let _ = writeln!(out, "{pad}  width: {width}");
    let _ = writeln!(out, "{pad}  height: {height}");
    out
}

/// A dated column's rows counted per day, **in time order**.
///
/// [`crate::dashboard::time_bars_tile`] over this registry's source, as a
/// self-contained top-level fragment — the same arrangement
/// [`ranked_category_bars`] uses, and for the same reason: the device is
/// emitted by one function, so the tile a dashboard lays out and the block this
/// kind builds cannot describe two different pictures.
///
/// # Why this device and not a histogram
///
/// A date is not binnable here — `is_binnable_type` says why, and the reason is
/// arithmetic rather than taste. So the picture a dated column gets is a count
/// per day over a **band** of days, which is what
/// [`brightfield_spec::vocab::MarkKind::band_aggregate_axes`] computes for a
/// `barY`: one `GROUP BY` on the column, one `COUNT(*)`, ordered by the column
/// itself.
///
/// **In time order, and uncapped**, which is the whole difference between this
/// and pointing [`ranked_category_bars`] at the same column. That kind writes
/// `sort: { y: -x, limit: 10 }`, so a year of daily readings would arrive as the
/// ten busiest days in descending order of count — every one of them true, and
/// the shape of the series gone. Writing no `sort:` is what asks
/// `brightfield-sql`'s `BarLowerer` for its band ordering instead, and that
/// ordering is ascending on the band column: chronological, for a column of
/// dates. `the_dates_tile_counts_in_time_order_and_drops_no_date` in
/// [`crate::dashboard`] is the test over the two absences.
fn counts_over_time() -> ChartKind<String> {
    ChartKind {
        id: COUNTS_OVER_TIME,
        icon: Icon("chart-bar"),
        description: "Counts the rows on each date, in time order, the total behind the selection",
        slots: TIME_SLOTS,
        controls: Vec::new,
        build: |bound, _options| {
            let column = bound.name("t").unwrap_or_default();
            let mut out = String::from("params:\n");
            let _ = writeln!(out, "  {SELECTION}: {{ select: crossfilter }}");
            let _ = writeln!(out, "hconcat:");
            out.push_str(&crate::dashboard::time_bars_tile(column, 2));
            out
        },
    }
}

/// Two categories crossed and counted.
///
/// The answer for a table with no distribution to draw: `cell` over two band
/// axes with a counted fill, which is the shape a table of names admits.
fn count_grid() -> ChartKind<String> {
    ChartKind {
        id: COUNT_GRID,
        icon: Icon("chart-bar"),
        description: "Crosses two categories and counts the rows in each cell",
        slots: GRID_SLOTS,
        controls: Vec::new,
        build: |bound, _options| {
            let x = bound.name("x").unwrap_or_default();
            let y = bound.name("y").unwrap_or_default();
            let mut out = String::from("plot:\n");
            let _ = writeln!(out, "  - mark: cell");
            let _ = writeln!(out, "    data: {{ from: {SOURCE} }}");
            let _ = writeln!(out, "    x: {}", yaml_quoted(x));
            let _ = writeln!(out, "    y: {}", yaml_quoted(y));
            let _ = writeln!(out, "    fill: {{ count: }}");
            out
        },
    }
}

/// [`crate::ranked_bars::chart_kind`] over this registry's source, as a
/// self-contained top-level fragment.
///
/// The declaration — id, icon, gloss, slot — is that module's, taken whole;
/// only the builder differs, and only in the three things this registry's spec
/// contract fixes that a placeholder cannot: the table the module counts over,
/// the `hconcat:` key that makes one module a document, and the `params:` entry
/// declaring the selection its interactors bind (see
/// [`SELECTION`]). Rebuilding the declaration here instead would be
/// a second copy of it, which is what a registry exists to end.
fn ranked_category_bars() -> ChartKind<String> {
    ChartKind {
        build: |bound, _options| {
            let column = bound.name("category").unwrap_or_default();
            let module = crate::ranked_bars::RankedCategoryBars::new(column);
            let mut out = String::from("params:\n");
            let _ = writeln!(out, "  {SELECTION}: {{ select: crossfilter }}");
            let _ = writeln!(out, "hconcat:");
            out.push_str(&module.plot_yaml(SOURCE, SELECTION, 2));
            out
        },
        ..crate::ranked_bars::chart_kind()
    }
}

// ---------------------------------------------------------------------------
// Columns → fields
// ---------------------------------------------------------------------------

/// The columns of a profiled table as fields a chart kind can be chosen for,
/// in the order a chooser should offer them.
///
/// Two decisions live here rather than in a slot, because a slot declares
/// *types* and both of these are about the values in a column:
///
/// - **eligibility.** A column is offered when its name survives both the
///   emitted SQL and the synthesised YAML unchanged, when it has a non-null
///   value, and when it has more than one distinct value — a constant column
///   bins to a single bar and crosses to a single row, which is a true picture
///   of nothing. A category is offered only up to the private
///   `GRID_MAX_DISTINCT` ceiling above; **a date is offered at any width**,
///   because a date is not a category and the ceiling is a bound on how many
///   categories a reader can tell apart.
/// - **order.** Measures first, in the table's own order; then dates, in the
///   table's own order; then categories, fewest distinct values first.
///   [`ChartKind::bind`] is first-fit in slot order, so this is what decides
///   *which* column fills a slot once the kind is chosen. `sort_by_key` is
///   stable, so two categories of equal width keep the table's own order rather
///   than an arbitrary one.
///
/// # What decides a column's [`FieldType`]
///
/// The DuckDB type, and in every case the question asked of it is *what can
/// this build draw*:
///
/// - the types the bin arithmetic can subtract and take a logarithm of are
///   measures — the private `is_binnable_type` below;
/// - `DATE` is [`FieldType::Temporal`], drawn by the kind registered under
///   [`COUNTS_OVER_TIME`];
/// - the **other** temporal types are offered nothing *of their own*, and that
///   is the one rule here written from a measurement rather than from an
///   argument. See `a_timestamp_band_puts_no_ink_on_the_page`: a `TIMESTAMP`
///   bound to a band axis puts **no** mark ink on the page, because
///   `brightfield-render`'s `positional_axis_class` reads it as continuous and
///   a bar has no band to stand on. The column such a table draws is a
///   different one — the bucket column [`crate::resample`] derives beside it,
///   offered as a temporal field by [`crate::dashboard`]. This function maps
///   the columns a table **has**; declaring a new one belongs to whoever writes
///   the `data:` block, which is the dashboard;
/// - everything else is a category.
#[must_use]
pub fn fields_of(columns: &[ColumnProfile]) -> Vec<Field> {
    let usable = || {
        columns
            .iter()
            .filter(|c| nameable(&c.name))
            .filter(|c| c.non_null > 0 && c.distinct > 1)
    };

    let mut fields: Vec<Field> = usable()
        .filter(|c| is_binnable_type(&c.type_name))
        .map(|c| Field::new(&c.name, FieldType::Quantitative))
        .collect();

    fields.extend(
        usable()
            .filter(|c| is_date_type(&c.type_name))
            .map(|c| Field::new(&c.name, FieldType::Temporal)),
    );

    let mut categories: Vec<&ColumnProfile> = usable()
        .filter(|c| !is_binnable_type(&c.type_name))
        .filter(|c| !is_temporal_type(&c.type_name))
        .filter(|c| c.distinct <= GRID_MAX_DISTINCT)
        .collect();
    categories.sort_by_key(|c| c.distinct);
    fields.extend(
        categories
            .into_iter()
            .map(|c| Field::new(&c.name, FieldType::Categorical)),
    );

    fields
}

/// Whether a column name can be written into both the emitted SQL and the
/// synthesised YAML without changing what it names.
///
/// The emitted SQL quotes identifiers with `"`, and the spec is written as
/// YAML, so a name carrying a `"` or a control character has no faithful
/// spelling in either — and silently drawing a *different* column would be
/// worse than drawing none.
///
/// Reachable from [`crate::dashboard`] because the bucket column it derives for
/// a timestamp is not in any profile, so it is offered as a field without
/// passing through [`fields_of`] — and the rule it still has to keep is this
/// one.
pub(crate) fn nameable(name: &str) -> bool {
    !name.is_empty() && !name.contains('"') && !name.chars().any(char::is_control)
}

/// Whether a DuckDB column type can be **binned** — strictly the numeric
/// types.
///
/// Temporal types are deliberately out, and the exclusion is load-bearing
/// rather than cautious: the bin scheme is arithmetic (`max - min`, then a
/// logarithm of the span), and subtracting two DuckDB `DATE`s yields an
/// `INTERVAL`, which has no logarithm.
fn is_binnable_type(duckdb_type: &str) -> bool {
    let base = type_base(duckdb_type);
    matches!(
        base.as_str(),
        "TINYINT"
            | "SMALLINT"
            | "INTEGER"
            | "BIGINT"
            | "HUGEINT"
            | "UTINYINT"
            | "USMALLINT"
            | "UINTEGER"
            | "UBIGINT"
            | "UHUGEINT"
            | "FLOAT"
            | "REAL"
            | "DOUBLE"
            | "DECIMAL"
            | "NUMERIC"
    )
}

/// Whether a DuckDB column type is a **calendar date** — the one temporal type
/// this build draws a picture over.
///
/// `DATE` reaches `brightfield-render` as an Arrow `Date32`, which that crate
/// collects into a band scale one category per day. Every other temporal type
/// is [`is_temporal_type`]'s business and gets no field at all — the test
/// `a_time_no_chart_here_draws_is_not_offered_a_band` names the spellings this
/// build refuses.
fn is_date_type(duckdb_type: &str) -> bool {
    type_base(duckdb_type) == "DATE"
}

/// Whether a DuckDB column type holds a **moment or a period in time**, date
/// included.
///
/// Read by [`fields_of`] to keep the temporal types it cannot draw out of the
/// category list, where they would otherwise be offered a band axis they put no
/// ink on. `DATETIME` is DuckDB's own alias for `TIMESTAMP`, and the
/// suffixed forms are its other precisions.
fn is_temporal_type(duckdb_type: &str) -> bool {
    matches!(
        type_base(duckdb_type).as_str(),
        "DATE"
            | "TIME"
            | "TIMETZ"
            | "DATETIME"
            | "TIMESTAMP"
            | "TIMESTAMPTZ"
            | "TIMESTAMP_S"
            | "TIMESTAMP_MS"
            | "TIMESTAMP_NS"
    )
}

/// A DuckDB type name reduced to the name the predicates above match on:
/// upper-cased, trimmed, without a width or precision, and with the spelled-out
/// time-zone suffix folded onto the short one DuckDB also accepts.
///
/// One function rather than a copy of the same calls per predicate. Two tests
/// between them hold the case, the width and the fold:
/// `a_columns_field_type_follows_what_can_be_binned` asks for `DECIMAL(10,2)`
/// and for ` integer `, which is the case and the width;
/// `a_time_no_chart_here_draws_is_not_offered_a_band` asks for `TIMESTAMP WITH
/// TIME ZONE`, which is the fold. The first alone leaves the fold unheld.
///
/// **The trim is held as a behaviour, not per call site.** Deleting either
/// `.trim()` below on its own leaves
/// `a_columns_field_type_follows_what_can_be_binned` green — measured, one at a
/// time — and deleting both reddens it, on ` integer `.
/// The outer one is subsumed — the inner runs after the split and strips the
/// same ends, so removing it changes this function's answer on no input at all
/// and no test could hold it. The inner one is not subsumed: it is what a type
/// written `DECIMAL (10,2)`, with a space before the paren, would need, and
/// nothing here asks for one.
pub(crate) fn type_base(duckdb_type: &str) -> String {
    let upper = duckdb_type.trim().to_ascii_uppercase();
    upper
        .split('(')
        .next()
        .unwrap_or(&upper)
        .trim()
        .replace(" WITH TIME ZONE", "TZ")
}

/// `value` as a YAML single-quoted scalar.
///
/// Single-quoted rather than double-quoted because the only escape a
/// single-quoted YAML scalar has is a doubled quote — no backslashes, no
/// interpretation — so a column name full of punctuation survives verbatim.
/// What single-quoting cannot carry is a **line break**, which [`nameable`]
/// keeps out of the field list above.
fn yaml_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::{parse_spec, Format};
    use brightfield_workbench::registry::audit_chart_kinds;

    fn column(name: &str, type_name: &str, distinct: u64) -> ColumnProfile {
        ColumnProfile {
            name: name.to_string(),
            type_name: type_name.to_string(),
            non_null: 100,
            nulls: 0,
            distinct,
            min: None,
            max: None,
            semantic: brightfield_engine::SemanticType::NotAsked,
            moments: None,
        }
    }

    /// The registry the running binary reads passes the workbench's own
    /// conformance gate — every kind's icon, gloss, slots and builder.
    #[test]
    fn the_shipped_registry_passes_the_audit() {
        audit_chart_kinds(registry()).expect("the shell's chart kinds are well-formed");
    }

    /// **The kinds this build ships, named.** A registry is a list, and a list
    /// nobody enumerates grows an entry that nothing downstream expects — so
    /// the set is pinned here and adding one is a deliberate edit of this line.
    #[test]
    fn the_registry_ships_these_kinds_in_this_order() {
        assert_eq!(
            registry().ids(),
            vec![
                BINNED_HISTOGRAM,
                SCATTER,
                POINT_MAP,
                COUNTS_OVER_TIME,
                COUNT_GRID,
                crate::ranked_bars::KIND_ID
            ],
            "declaration order is the preference order a chooser reads"
        );
    }

    /// **Adding the scatter moved no table's first look**, and no applicable
    /// list beyond gaining an entry.
    ///
    /// The comparison is against [`registry`]'s own list with the scatter taken
    /// out, over the field lists of up to three columns that can be drawn from
    /// the three field types — 40 of them, generated rather than chosen, so a
    /// shape nobody thought of is covered by construction.
    ///
    /// Two claims, and the second is what the first rests on. The first look is
    /// unchanged, so a shipped file or start opens on the picture it opened on
    /// before; and the rest of the list is unchanged as a subsequence, so a
    /// chooser offering more than one kind offers them in the order it did.
    ///
    /// Placing [`scatter`] ahead of [`binned_histogram`] in [`kinds`] reddens
    /// the first assertion here on any list carrying two measures — measured —
    /// and that is why this test is a comparison rather than a restatement of
    /// the shipped order.
    #[test]
    fn adding_the_scatter_moved_no_first_look() {
        let without =
            ChartKindRegistry::new(kinds().into_iter().filter(|k| k.id != SCATTER).collect());
        for fields in field_lists(3) {
            let mut shipped = registry().applicable(&fields);
            assert_eq!(
                shipped.first().copied(),
                without.applicable(&fields).first().copied(),
                "the first look over {fields:?} moved when the scatter was added"
            );
            shipped.retain(|id| *id != SCATTER);
            assert_eq!(
                shipped,
                without.applicable(&fields),
                "the kinds offered for {fields:?}, scatter aside, are not the \
                 ones offered before it"
            );
        }
    }

    /// The field lists of up to `max` columns over the three field types, one
    /// per combination, each column named for its position so
    /// [`ChartKind::bind`] can tell them apart.
    fn field_lists(max: usize) -> Vec<Vec<Field>> {
        let types = [
            FieldType::Quantitative,
            FieldType::Temporal,
            FieldType::Categorical,
        ];
        let mut out = vec![Vec::new()];
        let mut frontier = vec![Vec::<Field>::new()];
        for _ in 0..max {
            let mut next = Vec::new();
            for prefix in &frontier {
                for ty in types {
                    let mut list = prefix.clone();
                    list.push(Field::new(format!("f{}", list.len()), ty));
                    next.push(list);
                }
            }
            out.extend(next.iter().cloned());
            frontier = next;
        }
        out
    }

    /// **A pair of measures reaches the scatter, and a lone measure does not.**
    ///
    /// The registry-level half of the slot declaration: what `accepts` answers
    /// is what decides whether the kind is offered at all, and the refusal is
    /// the half a kind with one required slot would pass while drawing nothing.
    #[test]
    fn a_second_measure_is_what_the_scatter_waits_for() {
        let one = fields_of(&[column("amount", "DOUBLE", 900)]);
        assert!(!registry().applicable(&one).contains(&SCATTER), "{one:?}");
        let two = fields_of(&[
            column("amount", "DOUBLE", 900),
            column("weight", "BIGINT", 400),
        ]);
        assert!(registry().applicable(&two).contains(&SCATTER), "{two:?}");
        assert_eq!(
            registry().applicable(&two).first().copied(),
            Some(BINNED_HISTOGRAM),
            "a pair of measures still opens on the first one's distribution"
        );
    }

    /// Every kind declares **no** control.
    ///
    /// Not decoration: the chart pane rebuilds its
    /// [`brightfield_workbench::item::ChartModule`] from the document each
    /// frame, which is only sound while a module's own state is a function of
    /// the document. A [`brightfield_workbench::registry::ModuleControl`] is
    /// state a *user* sets, so the first kind to declare one has to be met by
    /// the pane holding its module across frames — and this is the test that
    /// says so at the moment it happens.
    #[test]
    fn no_kind_declares_a_control_that_the_pane_would_have_to_remember() {
        for kind in registry().kinds() {
            assert!(
                (kind.controls)().is_empty(),
                "{}: declares a control, so the chart pane can no longer rebuild \
                 its module every frame — hold the module instead",
                kind.id
            );
        }
    }

    /// Every kind builds **a document**: the source it emits parses on its
    /// own once a `data:` block is written above it. That is the spec contract
    /// this registry states, and it is what lets a caller concatenate a block
    /// without knowing which kind built it.
    #[test]
    fn every_kind_builds_a_block_that_parses_under_a_data_header() {
        for kind in registry().kinds() {
            let source = document_of(kind, FILE_HEADER);
            let parsed = parse_spec(&source, Format::Yaml);
            assert!(parsed.is_ok(), "{}: {parsed:?}\n{source}", kind.id);
        }
    }

    /// **No kind's block asks for something the load then says had no
    /// effect.** A picture the reader is shown must not arrive under a sentence
    /// saying part of it did nothing.
    ///
    /// Held on the composed document's [`LoadDiagnostics`], because that is the
    /// object the window turns into a banner: `MeridianApp::say_load_diagnostics`
    /// raises one `Severity::Warning` over the advisories and one
    /// `Severity::Error` over the blocking ones, so an advisory a kind's own
    /// builder earns is a sentence a user reads about a file they merely
    /// opened, with no spec of theirs to correct.
    ///
    /// **Composed, not parsed** — the binding checks that produce these live in
    /// `analyse_spec`, which `parse_spec` does not run. A version of this test
    /// written on `parse_spec`'s warnings stayed green on the very block that
    /// prompted it: the ranked-bars block's `toggleY` and `highlight` bind
    /// `$sel`, and until the builder declared `sel` under `params:` a
    /// one-category CSV opened under *"1 instruction … had no effect"*.
    #[test]
    fn no_kinds_block_asks_for_something_the_load_says_had_no_effect() {
        for kind in registry().kinds() {
            let source = document_of(kind, INLINE_ROWS);
            let composed = crate::pipeline::compose_spec_str(&source, None)
                .unwrap_or_else(|e| panic!("{}: {e}\n{source}", kind.id));
            let found: Vec<String> = composed
                .diagnostics
                .advisory()
                .iter()
                .map(ToString::to_string)
                .collect();
            assert!(
                found.is_empty(),
                "{}: the block it builds earns an advisory, and the window puts \
                 every one of these over the picture: {found:?}\n{source}",
                kind.id
            );
        }
    }

    /// Rows carrying the columns [`document_of`] binds, under the name every
    /// kind reads. `c0` is numeric so it fills a quantitative slot; both
    /// columns serve as band axes.
    const INLINE_ROWS: &str = "\
data:
  opened:
    - { c0: 1, c1: north }
    - { c0: 4, c1: north }
    - { c0: 9, c1: south }
    - { c0: 16, c1: east }
";

    /// A file-backed `data:` header — enough to parse against, and it opens no
    /// file because parsing does not read one.
    const FILE_HEADER: &str = "data:\n  opened:\n    file: 'rows.csv'\n";

    /// The document `kind` builds over its own required slots, under `data`.
    fn document_of(kind: &ChartKind<String>, data: &str) -> String {
        let fields: Vec<Field> = kind
            .slots
            .iter()
            .filter(|s| s.required)
            .enumerate()
            .map(|(i, s)| Field::new(format!("c{i}"), s.accepts[0]))
            .collect();
        let binding = kind.bind(&fields).expect("its own required slots bind");
        let block = kind
            .spec(&binding, &kind.options())
            .expect("its own builder runs");
        format!("{data}{block}")
    }

    /// A measure beats a cross-tabulation, and the field order decides which
    /// column fills the slot.
    #[test]
    fn a_table_with_a_measure_opens_on_its_distribution() {
        let fields = fields_of(&[
            column("region", "VARCHAR", 4),
            column("amount", "DOUBLE", 900),
        ]);
        assert_eq!(
            registry().applicable(&fields).first().copied(),
            Some(BINNED_HISTOGRAM)
        );
        let kind = find(BINNED_HISTOGRAM).expect("shipped");
        let block = kind
            .spec(&kind.bind(&fields).expect("binds"), &kind.options())
            .expect("builds");
        assert!(block.contains("x: { bin: 'amount' }"), "{block}");
    }

    /// A table of names with no distribution crosses its two narrowest
    /// categories — narrowest first, which is what the field order carries.
    ///
    /// The `day` column is here to be **left out of the grid**: it is a date,
    /// so it takes a temporal field and a picture of its own rather than a band
    /// on somebody else's axis.
    #[test]
    fn a_table_of_names_crosses_its_two_narrowest_categories() {
        let fields = fields_of(&[
            column("city", "VARCHAR", 40),
            column("region", "VARCHAR", 4),
            column("day", "DATE", 12),
        ]);
        assert_eq!(
            registry().applicable(&fields).first().copied(),
            Some(COUNTS_OVER_TIME),
            "a dated column is the first look this table admits"
        );
        assert!(registry().applicable(&fields).contains(&COUNT_GRID));
        let kind = find(COUNT_GRID).expect("shipped");
        let block = kind
            .spec(&kind.bind(&fields).expect("binds"), &kind.options())
            .expect("builds");
        assert!(block.contains("x: 'region'"), "{block}");
        assert!(block.contains("y: 'city'"), "{block}");
    }

    /// One category and nothing else is the ranked bars' case — a table that
    /// used to admit no first look at all.
    #[test]
    fn one_category_opens_on_ranked_bars() {
        let fields = fields_of(&[column("tag", "VARCHAR", 9)]);
        assert_eq!(
            registry().applicable(&fields),
            vec![crate::ranked_bars::KIND_ID]
        );
    }

    /// The eligibility rules, each on its own column: a constant, an all-null,
    /// a name that cannot be written, and a category with too many values are
    /// each offered to no kind.
    #[test]
    fn a_column_that_cannot_be_drawn_is_not_offered() {
        let constant = column("flat", "DOUBLE", 1);
        let all_null = ColumnProfile {
            non_null: 0,
            ..column("blank", "DOUBLE", 900)
        };
        let unwritable = column("we\"ird", "DOUBLE", 900);
        let too_many = column("id", "VARCHAR", GRID_MAX_DISTINCT + 1);
        for profile in [constant, all_null, unwritable, too_many] {
            let name = profile.name.clone();
            assert!(
                fields_of(&[profile]).is_empty(),
                "{name} was offered to a chart kind"
            );
        }
        // …and the boundary is inclusive: one fewer distinct value is offered.
        assert_eq!(
            fields_of(&[column("id", "VARCHAR", GRID_MAX_DISTINCT)]).len(),
            1
        );
    }

    /// A numeric type is a measure, a date is temporal and everything else is a
    /// category — stated over the types themselves.
    #[test]
    fn a_columns_field_type_follows_what_can_be_binned() {
        for numeric in ["BIGINT", "DOUBLE", "DECIMAL(10,2)", " integer "] {
            assert_eq!(
                fields_of(&[column("v", numeric, 900)])
                    .first()
                    .map(|f| f.ty),
                Some(FieldType::Quantitative),
                "{numeric}"
            );
        }
        for dated in ["DATE", " date "] {
            assert_eq!(
                fields_of(&[column("v", dated, 9)]).first().map(|f| f.ty),
                Some(FieldType::Temporal),
                "{dated}"
            );
        }
        for other in ["VARCHAR", "BOOLEAN"] {
            assert_eq!(
                fields_of(&[column("v", other, 9)]).first().map(|f| f.ty),
                Some(FieldType::Categorical),
                "{other}"
            );
        }
    }

    /// **The temporal types this build cannot draw are offered to nothing** —
    /// not handed to a band axis they would put no ink on.
    ///
    /// The list is the exclusion, and `a_timestamp_band_puts_no_ink_on_the_page`
    /// below is the measurement it rests on.
    #[test]
    fn a_time_no_chart_here_draws_is_not_offered_a_band() {
        for undrawable in [
            "TIME",
            "TIMETZ",
            "TIMESTAMP",
            "TIMESTAMPTZ",
            "TIMESTAMP WITH TIME ZONE",
            "TIMESTAMP_NS",
            "DATETIME",
        ] {
            assert!(
                fields_of(&[column("t", undrawable, 9)]).is_empty(),
                "{undrawable} was offered a field, and nothing here draws one"
            );
        }
    }

    /// **A daily series past the grid ceiling gets a field and a kind.**
    ///
    /// Ninety days is three months of readings, which is the shape that used to
    /// be refused: `GRID_MAX_DISTINCT` was applied to every non-binnable
    /// column, so a date crossed it at two months and the generator recorded an
    /// omission. The ceiling bounds a grid's axes, and this column is not one —
    /// `the_grid_ceiling_still_refuses_a_wide_pair_of_categories` is the test
    /// below that keeps it where it was argued for.
    #[test]
    fn a_daily_series_past_the_grid_ceiling_is_offered_a_picture() {
        let daily = column("day", "DATE", 90);
        assert!(daily.distinct > GRID_MAX_DISTINCT, "the case, or it is not");
        let fields = fields_of(std::slice::from_ref(&daily));
        assert_eq!(
            fields,
            vec![Field::new("day", FieldType::Temporal)],
            "a date is offered at any width"
        );
        assert_eq!(
            registry().applicable(&fields),
            vec![COUNTS_OVER_TIME],
            "and exactly one kind draws it"
        );
        // The device follows from the column being a date, not from how many
        // days it holds: the same kind answers for a week and for a decade.
        for width in [2, GRID_MAX_DISTINCT, 3_650] {
            assert_eq!(
                registry().applicable(&fields_of(&[column("day", "DATE", width)])),
                vec![COUNTS_OVER_TIME],
                "{width} distinct days"
            );
        }
    }

    /// **The ceiling still refuses a wide category, and the grid with it.** The
    /// number was argued for over `distinct × distinct`, and that argument is
    /// untouched: two categories past it cross into a wall of cells, so neither
    /// is offered and `count-grid` has nothing to bind.
    #[test]
    fn the_grid_ceiling_still_refuses_a_wide_pair_of_categories() {
        let wide = [
            column("city", "VARCHAR", GRID_MAX_DISTINCT + 1),
            column("street", "VARCHAR", 4_000),
        ];
        assert!(
            fields_of(&wide).is_empty(),
            "a category past the ceiling is offered to nothing"
        );
        assert!(
            registry().applicable(&fields_of(&wide)).is_empty(),
            "so no kind — the count grid least of all — has an axis to stand on"
        );
        // …and one wide category beside one narrow one still cannot cross.
        let mixed = fields_of(&[
            column("city", "VARCHAR", GRID_MAX_DISTINCT + 1),
            column("region", "VARCHAR", 4),
        ]);
        assert_eq!(mixed, vec![Field::new("region", FieldType::Categorical)]);
        assert!(!registry().applicable(&mixed).contains(&COUNT_GRID));
    }

    /// **The measurement [`fields_of`]'s temporal split rests on**: of the
    /// types DuckDB hands back for a column of dates, a `DATE` and a text
    /// spelling both put mark ink on the page as a band, and a `TIMESTAMP` puts
    /// no ink.
    ///
    /// This is a renderer fact and it is asserted here because it is the reason
    /// the split exists at all — a rule written from an argument would have
    /// offered the raw timestamp and shipped a tile with axes and no bars,
    /// which is exactly the failure `tests/bar_orientation.rs` exists for.
    ///
    /// The `VARCHAR` reading is doing two jobs. It tells a broken harness from
    /// a real zero — if it ever reads 0 the measurement is meaningless rather
    /// than damning — and it is the reading [`crate::resample`] stands on: the
    /// bucket column that module derives is `strftime` text, so this is the ink
    /// a resampled timestamp draws. It is compared against the `DATE` reading
    /// rather than to a number, because the two spellings are the same bands
    /// and a figure in a comment goes stale in silence.
    #[test]
    fn a_timestamp_band_puts_no_ink_on_the_page() {
        let text = band_mark_ink("VARCHAR");
        assert!(
            text > 1_000,
            "the harness draws nothing at all, so nothing below is evidence"
        );
        assert_eq!(
            band_mark_ink("DATE"),
            text,
            "a DATE band and the same dates as text are the same bands"
        );
        assert_eq!(
            band_mark_ink("TIMESTAMP"),
            0,
            "a TIMESTAMP band draws bars now, so `fields_of` can offer it a \
             field and `resample` has nothing left to do"
        );
    }

    /// Pixels of default mark ink in a counting `barY` whose band is one column
    /// of six dates, cast to `cast`.
    ///
    /// Counted as pixels rather than scene ops for the reason
    /// `tests/bar_orientation.rs` gives at length: geometry that never reached
    /// a pixel satisfies every structural check there is.
    fn band_mark_ink(cast: &str) -> u64 {
        let source = format!(
            "data:\n  {SOURCE}: \"SELECT CAST(d AS {cast}) AS d FROM (VALUES \
             ('2020-01-01'),('2020-01-01'),('2020-01-02'),('2020-01-03'),\
             ('2020-01-03'),('2020-01-04')) AS v(d)\"\nhconcat:\n\
             {}",
            crate::dashboard::time_bars_tile("d", 2)
        );
        let composed = crate::pipeline::compose_spec_str(&source, None)
            .unwrap_or_else(|e| panic!("{cast}: {e}\n{source}"));
        let png = std::env::temp_dir().join(format!("bf-band-ink-{cast}.png"));
        crate::capture::capture_vello_only(composed, 1.0, &png).expect("export");
        let want = meridian_design::viz::MARK_DEFAULT_LIGHT;
        let want = [
            (want.r * 255.0).round() as i32,
            (want.g * 255.0).round() as i32,
            (want.b * 255.0).round() as i32,
        ];
        let img = image::open(&png).expect("open png").to_rgba8();
        img.pixels()
            .filter(|p| (0..3).all(|c| (i32::from(p.0[c]) - want[c]).abs() <= 20))
            .count() as u64
    }
}
