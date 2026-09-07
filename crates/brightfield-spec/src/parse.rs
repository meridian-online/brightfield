//! Entry points and the Value → typed-AST walker.
//!
//! Parsing is two-stage:
//!
//! 1. Deserialise YAML/JSON into an intermediate [`serde_yaml::Value`].
//! 2. Walk the Value with vocabulary lookup, param-ref lifting at
//!    [`LIFT_SURFACE_FIELDS`] positions, SQL expression tokenisation, and
//!    Option Z handling of unknown-vs-unimplemented names.
//!
//! A textual round-trip through YAML/JSON is NOT a goal of v1; the
//! guarantee is AST idempotence: `parse → serialise → parse` yields an
//! equal Spec, with `ParamRef` canonicalised to `$name` string form.

use std::fmt;
use std::path::Path;

use indexmap::IndexMap;
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

use crate::ast::{
    AggregateFunc, Component, ConcatNode, Config, DataSource, DataSourceKind, Input, Interactor,
    LegendNode, Mark, MarkData, Meta, ParamNode, ParamRef, PlotDefaults, PlotNode, SelectionNode,
    SpaceNode, Spec, SpecValue, ValueOrParamRef,
};
use crate::error::{NameSurface, ParseError, SourceSpan};
use crate::expr;
use crate::vocab::{
    is_colour_literal, ImplStatus, InputKind, InteractorKind, LegendChannel, MarkKind,
    SelectionResolution,
};

/// Surface of field positions at which a bare `$param` string or a
/// `{param: name}` / `{selection: name}` object is lifted to a structural
/// [`ParamRef`] rather than kept as a string value.
///
/// Pinned to upstream Mosaic 0.24.x `parse-spec.js` `maybeParam` call sites.
/// The list is authoritative — fields not in this list are NEVER lifted, even
/// if they happen to carry a `$`-prefixed string.
pub const LIFT_SURFACE_FIELDS: &[&str] = &[
    // Mark-level
    "filterBy",
    "x",
    "y",
    "x1",
    "x2",
    "y1",
    "y2",
    "z",
    "r",
    "fx",
    "fy",
    "fill",
    "stroke",
    "opacity",
    "fillOpacity",
    "strokeOpacity",
    "strokeWidth",
    "strokeDasharray",
    "strokeLinecap",
    "strokeLinejoin",
    "strokeMiterlimit",
    "symbol",
    "text",
    "title",
    "href",
    "src",
    "width",
    "height",
    "rotate",
    "length",
    "shape",
    "anchor",
    "frameAnchor",
    "textAnchor",
    "lineAnchor",
    "lineWidth",
    "fontFamily",
    "fontSize",
    "fontStyle",
    "fontVariant",
    "fontWeight",
    "interval",
    "domain",
    "range",
    "offset",
    "inset",
    "insetTop",
    "insetRight",
    "insetBottom",
    "insetLeft",
    "dx",
    "dy",
    "padding",
    "paddingInner",
    "paddingOuter",
    "value",
    "order",
    "reverse",
    "sort",
    "tip",
    "channels",
    "pointerEvents",
    "ariaLabel",
    "ariaDescription",
    "ariaHidden",
    "clip",
    "facet",
    "facetAnchor",
    "curve",
    "tension",
    "marker",
    "markerStart",
    "markerMid",
    "markerEnd",
    "bend",
    "pixelSize",
    "binWidth",
    "bandwidth",
    "bins",
    "thresholds",
    "weight",
    "select",
    // Statistical mark options (density / regression)
    "normalize",
    "stack",
    "ci",
    // Interactor/input
    "as",
    // `by:` on a `highlight` interactor names the selection it CONSUMES
    // — lifted to a `Param` ref symmetric with `as:`, so a
    // downstream `HighlightBinding` reads it exactly like a producer's `as:`.
    "by",
    "field",
    "fields",
    "column",
    "columns",
    "source",
    "options",
    "label",
    "format",
    "filter",
    "peers",
    "empty",
    "cross",
    "step",
    "min",
    "max",
    // Legend / plot attributes we might see in option bags
    "for",
    "marginTop",
    "marginRight",
    "marginBottom",
    "marginLeft",
    "xDomain",
    "yDomain",
    "fxDomain",
    "fyDomain",
    "xRange",
    "yRange",
    "colorDomain",
    "colorRange",
    "colorScheme",
    "opacityDomain",
    "opacityRange",
    "symbolDomain",
    "symbolRange",
];

/// Mark channel fields that may carry a self-aggregating transform
/// (`fill: {count:}`, `fill: {avg: col}`, `r: {count:}`). Scoped to the
/// encoding channels the hexbin / self-aggregating-cell corpus uses, so a
/// single-key map at, say, `x: {bin: t}` (a positional bin transform) is not
/// mistaken for an aggregate. Pinned to what the vendored corpus exercises.
pub const AGGREGATE_CHANNEL_FIELDS: &[&str] = &["fill", "r"];

/// Single-key channel-map keys that are recognised Mosaic channel TRANSFORMS,
/// not aggregates — so a `{sql: 'POW(10, mag)'}` expression channel (the
/// vendored region-tests / earthquakes corpus uses `r: {sql: …}`) is left to
/// ordinary lifting instead of being mistaken for a typo'd aggregate and
/// warned about. Aggregates (`count`/`avg`/…) are matched by
/// [`AggregateFunc::from_wire`]; anything else on this list is a transform we
/// carry as a plain object; only a genuinely unknown key warns.
const CHANNEL_TRANSFORM_KEYS: &[&str] = &["sql"];

/// The mark channels a renderer maps to ink. A single-key map on one of these
/// is a request to *compute* the channel — `x: {bin: t}`, `y: {count:}` — and
/// no lowerer computes any of them, so each one is a hole in the drawing
/// rather than an ignored knob.
///
/// Deliberately narrower than [`crate::vocab::CONSUMED_MARK_OPTION_KEYS`],
/// which also carries scalars (`bins`, `binWidth`, `thresholds`, `geometry`,
/// `type`, `filterBy`) read as plain values that never carry a transform.
/// Warning on those would be guessing at a shape the corpus does not show.
const RENDERED_CHANNEL_FIELDS: &[&str] = &[
    "x", "y", "x1", "y1", "x2", "y2", "fill", "stroke", "size", "text",
];

/// The two positional channels a `bin` + `count` pair can occupy, in the order
/// [`binned_histogram`] tries them: `bin` on `x` counting on `y` (a `rectY`
/// histogram), then the transpose (`rectX`).
const BIN_COUNT_AXES: [(&str, &str); 2] = [("x", "y"), ("y", "x")];

/// The modifier keys a `{bin: col, …}` map may carry and still be lifted.
/// Anything else means the author asked for something the lowerer does not do,
/// and the pair is left uncomputed rather than half-honoured.
const BIN_MODIFIER_KEYS: &[&str] = &["steps"];

/// The channels that, bound to a COLUMN, split a binned rect into groups. `z`
/// is the explicit grouping channel; `fill`/`stroke` group implicitly when they
/// name a field rather than a colour constant.
const GROUPING_CHANNEL_FIELDS: &[&str] = &["z", "fill", "stroke"];

/// One mark's lifted positional `bin` + `count` pair.
struct BinnedHistogram {
    /// The positional channel carrying `{bin: …}` (`"x"` or `"y"`).
    bin_channel: &'static str,
    /// The opposite positional channel, carrying `{count:}`.
    count_channel: &'static str,
    /// The column named by the `bin`.
    column: String,
    /// The `steps:` hint, when written.
    steps: Option<i64>,
}

/// The interval channels that make a bar a PRE-EXTENTED one — the author has
/// written both ends themselves, so there is nothing to aggregate and the
/// band lift is refused. Present on a mark, any one of them is enough.
const INTERVAL_CHANNEL_FIELDS: &[&str] = &["x1", "x2", "y1", "y2"];

/// The one key a `sort:` map may carry beside its channel entry.
const SORT_MODIFIER_KEYS: &[&str] = &["limit"];

/// One mark's lifted `sort:` — the ranked half of a ranked category bar:
/// `sort: { y: -x, limit: 10 }` on a `barX`.
struct MarkSort {
    /// The channel whose order the sort sets — the mark's band axis.
    channel: &'static str,
    /// The channel the order is read from — the mark's value axis.
    by: &'static str,
    /// Whether the spec wrote the `-` prefix.
    descending: bool,
    /// `limit:`, when written.
    limit: Option<u64>,
}

/// One mark's lifted positional aggregate over a band channel — the ranked
/// category bar: `x: {sum: gold}` with `y: nationality` on a `barX`.
struct BandedAggregate {
    /// The positional channel carrying the aggregate (`"x"` on a `barX`).
    value_channel: &'static str,
    /// The aggregate written there.
    func: AggregateFunc,
    /// The column the aggregate consumes; `None` for `{count:}`.
    column: Option<String>,
}

/// Wire format of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// YAML source (`.yaml` / `.yml`).
    Yaml,
    /// JSON source (`.json`).
    Json,
}

/// Non-fatal observations collected during parsing.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseWarning {
    /// An unknown option key was encountered on a `deny_unknown_fields` head
    /// that has been narrowed to accept it silently (spec constraint #4e —
    /// only `Meta` is strict; `Config` and `PlotDefaults` are open). Retained
    /// here for future strict heads.
    UnknownOption {
        /// The dotted field path where the unknown key was found.
        path: String,
        /// The unknown key.
        key: String,
    },

    /// A known-but-unimplemented mark / interactor / input / component /
    /// legend-channel name was encountered. Parse produced an AST stub with
    /// `ImplStatus::Unimplemented` or `Planned`.
    Unimplemented {
        /// The name as it appeared in the source.
        name: String,
        /// Which vocabulary surface this name was used on.
        surface: NameSurface,
        /// Status assigned.
        status: ImplStatus,
    },

    /// `meta.version` does not match [`crate::SUPPORTED_MOSAIC_MAJOR_MINOR`]
    /// on major+minor.
    VersionMismatch {
        /// Declared version string.
        declared: String,
        /// What this parser supports.
        supported: &'static str,
    },

    /// A declared param has zero subscribers in the spec.
    DeadParam {
        /// The param name with no consumers.
        name: String,
    },

    /// An input widget's output type is provably incompatible with its
    /// target param's declared type.
    ParamTypeMismatch {
        /// The param name.
        param: String,
        /// The expected type from the param declaration.
        expected: String,
        /// The widget kind that writes to this param.
        widget_kind: String,
    },

    /// An interactor's `as:` references a param name that is not declared
    /// in `params:`. Non-fatal because Mosaic may create selections implicitly.
    InteractorBindingMissing {
        /// The undeclared param name.
        name: String,
    },

    /// An interactor's `as:` references a param that is declared as a
    /// `ParamNode::Value`, not a `ParamNode::Selection`.
    InteractorBindingNonSelection {
        /// The param name.
        name: String,
    },

    /// A legend's `as:` references a param name that is not declared in
    /// `params:`. The legend stays display-only — unlike
    /// interactors, a legend producer binding requires a declared selection
    /// because its self-exclusion contract depends on the declared
    /// resolution.
    LegendBindingMissing {
        /// The undeclared param name.
        name: String,
    },

    /// A legend's `as:` references a param that is declared as a
    /// `ParamNode::Value`, not a `ParamNode::Selection`. The legend stays
    /// display-only.
    LegendBindingNonSelection {
        /// The param name.
        name: String,
    },

    /// A mark channel carried a single-key aggregate-shaped map (`fill: {X:}`)
    /// whose key `X` is not a recognised aggregate function. The channel
    /// degrades — it is left as a plain object, NOT read as a column named `X`
    /// (no silent column lookup) — and this names the offending key so an
    /// author sees the typo (hexbin / self-aggregating cell).
    UnknownAggregate {
        /// The channel field the map appeared on (e.g. `fill`).
        field: String,
        /// The unrecognised aggregate key.
        name: String,
    },

    /// A legend's `as:` references a selection whose resolution is not
    /// `crossfilter`. Only crossfilter resolution self-excludes the
    /// contributor's own plot (`compile_selection`), so any other resolution
    /// would let the legend filter its own `for:` plot and invalidate the
    /// launch-time colour-scale snapshot; the legend stays display-only.
    LegendBindingNonCrossfilter {
        /// The param name.
        name: String,
        /// The declared resolution's wire name (e.g. `single`).
        resolution: String,
    },

    /// A plot-level inset attribute (`inset`, `xInset`, `yInset`,
    /// `xInsetLeft`/`xInsetRight`/`yInsetTop`/`yInsetBottom`) carried a value
    /// that is neither a literal number nor a `$param` reference. The attribute
    /// degrades to absent for range insetting (the axis falls back to its
    /// default inset), and this names it so an author sees the typo rather than
    /// silently losing the inset (axis-inset round).
    NonNumericInset {
        /// The offending attribute key.
        attribute: String,
    },

    /// A `highlight` interactor's `by:` references a param name that is not
    /// declared in `params:` and is not created by any `as:` binding. The
    /// highlight stays inert (no binding forms) — like a legend producer, a
    /// highlight consumer needs a real selection to dim against.
    HighlightBindingMissing {
        /// The undeclared / unbound selection name.
        name: String,
    },

    /// A `highlight` interactor's `by:` references a param that is declared as
    /// a `ParamNode::Value`, not a `ParamNode::Selection` (and is not an
    /// `as:`-bound selection). The highlight stays inert.
    HighlightBindingNonSelection {
        /// The value-param name.
        name: String,
    },

    /// A `highlight` interactor sits on a plot whose data mark AGGREGATES in
    /// SQL (a density/heatmap/contour/raster/cell/hexbin/regression kind). The
    /// per-row `__bf_selected` membership projection would evaluate against the
    /// grouped output — a predicate over a non-group-key column would SQL-error
    /// — so the highlight is guarded out (no binding, no projection) rather than
    /// risking a runtime crash. A row-level mark (dot/bar/rect/text) is
    /// unaffected: the projection there evaluates against the full source table.
    HighlightOnAggregate {
        /// The `by:` selection name.
        selection: String,
        /// The aggregating mark kind's wire name (e.g. `heatmap`).
        mark: String,
    },

    /// A plot-level axis-title / plot-title attribute (`xLabel`, `yLabel`,
    /// `title`) carried a value that is neither a string nor `null`/`$param`.
    /// The label degrades — the axis falls back to its derived field-name title,
    /// or the plot title is dropped — and this names it so an author sees the
    /// typo rather than silently losing the label (axis + plot titles).
    NonStringLabel {
        /// The offending attribute key.
        attribute: String,
    },

    /// A plot's `projectionType` carried a value outside Mosaic's
    /// `ProjectionName` vocabulary, or a non-string value. The plot then names
    /// no projection at all — it draws as a cartesian plot — and this names the
    /// value so an author sees the unsupported projection rather than silently
    /// getting a different picture.
    ///
    /// [`crate::layout::ResolvedProjection::from_wire`] is the sole judge of
    /// what is recognised, so widening the catalogue narrows this warning and
    /// widens [`crate::layout::resolve_projection`] in the same edit.
    UnknownProjection {
        /// The unrecognised projection value (or `<non-string>` for a non-string).
        value: String,
    },

    /// A mark asked for `aspectRatio: 1` on a plot that names a
    /// `projectionType`. The two
    /// are alternatives, not layers: equal-aspect widens the narrower positional
    /// domain until a degree of longitude and a degree of latitude take the same
    /// number of pixels, which is a cartesian frame's best impersonation of a
    /// map, and a projection has already answered that question — correctly, and
    /// differently at each latitude. The projection decides the frame and the
    /// `aspectRatio` is dropped.
    ///
    /// `brightfield_render::channel::ChannelMap::equal_aspect` is what makes the
    /// refusal true rather than merely announced; this is what tells the author
    /// it happened.
    AspectRatioWithProjection {
        /// The mark that asked for both.
        mark: String,
    },

    /// A plot names a `projectionType` and carries a mark whose kind cannot
    /// draw through it — see
    /// [`crate::vocab::MarkKind::draws_through_a_projection`].
    ///
    /// The plot's axes are in the projection's planar units, so this mark's raw
    /// column numbers would land somewhere arbitrary on them: a second
    /// coordinate system drawn over the first, which reads as a picture rather
    /// than as an error. **The mark is not drawn**, and this is what says so.
    MarkCannotProject {
        /// The mark kind's wire name.
        mark: String,
        /// The `projectionType` the plot named.
        projection: String,
    },

    /// A plot names a `projectionType` whose two axes do not invert separately,
    /// and carries an `intervalX` / `intervalY` / `intervalXY` interactor.
    ///
    /// A rectangle swept in pixels has a rectangle of longitudes and latitudes
    /// behind it when the planar `u` depends on the longitude alone and the
    /// planar `v` on the latitude alone, and not otherwise — see
    /// [`crate::layout::ResolvedProjection::axes_invert_separately`], enumerated
    /// by `four_of_mosaics_names_invert_per_axis`. Under a
    /// conic or an azimuthal it does not, so the `column BETWEEN lo AND hi`
    /// clause the brush would build names bounds the reader never swept.
    /// **The interactor is not installed**, and this is what says so.
    IntervalBrushUnderCurvedProjection {
        /// The interactor kind's wire name.
        interactor: String,
        /// The `projectionType` the plot named.
        projection: String,
    },

    /// A mark carried an option key that **no lowerer and no renderer reads**
    /// — see [`crate::vocab::CONSUMED_MARK_OPTION_KEYS`]. The key is parsed,
    /// held in the AST and serialised back out faithfully, and then dropped:
    /// the drawing it asks for never happens.
    ///
    /// Raised only for marks whose kind is `Implemented`. An unimplemented
    /// mark already carries [`ParseWarning::Unimplemented`], and itemising the
    /// options of something that draws nothing at all is noise, not honesty.
    UnconsumedMarkOption {
        /// The mark kind's wire name (e.g. `barX`).
        mark: String,
        /// The option key, dotted one level for a key nested inside an
        /// ignored map (`sort.limit`).
        key: String,
    },

    /// A mark wrote a `sort:` in a shape no lowerer orders by — see the
    /// private `mark_sort` resolver for each refusal and its reason. The rows
    /// arrive in whatever order the query returned them and any `limit:`
    /// inside the same map is dropped with it.
    ///
    /// Distinct from [`ParseWarning::UnconsumedMarkOption`], which is about a
    /// *key* nothing reads: `sort` is on
    /// [`crate::vocab::CONSUMED_MARK_OPTION_KEYS`], so the key-level check
    /// passes and only the value SHAPE is left to report. The same split
    /// [`ParseWarning::UnconsumedChannelTransform`] makes for a channel.
    ///
    /// Raised only for marks whose kind is `Implemented`, on the same grounds
    /// as [`ParseWarning::UnconsumedMarkOption`].
    UnconsumedSort {
        /// The mark kind's wire name (e.g. `dot`).
        mark: String,
    },

    /// A node declaring `input: slider` over an interval `select:` is missing
    /// one of the four literals the widget cannot be built without — `as:`,
    /// `column:`, `min:`, `max:` — so **no control is drawn for it at all**.
    ///
    /// The drop is deliberate (a range the spec did not declare is refused
    /// rather than guessed at from a domain query), but until this warning it
    /// was also silent: the widget simply did not appear and nothing said why.
    /// Statically decidable in full — all four are literals in the spec text,
    /// so no data is needed to know the control will be missing.
    IntervalSliderIncomplete {
        /// Component path of the slider node, so the author can find it.
        path: String,
        /// The option keys it needs and does not have as literals, in the
        /// order the collector reads them.
        missing: Vec<String>,
    },

    /// A channel the renderer maps carried a single-key **transform** map —
    /// `x: {bin: t}`, `y: {count:}` — that no lowerer computes. The channel is
    /// held in the AST as a plain object, the renderer's channel extraction
    /// finds no column in it, and the mark draws nothing at all.
    ///
    /// Distinct from [`ParseWarning::UnconsumedMarkOption`], which is about a
    /// *key* nothing reads. Here the key is one the renderer very much reads;
    /// it is the *value shape* that has no reader. That is why the key-level
    /// check passes silently and this one is needed: `x` is on
    /// [`crate::vocab::CONSUMED_MARK_OPTION_KEYS`], so an unconsumed transform
    /// sitting on it was the one degradation that cost a whole frame and said
    /// nothing.
    UnconsumedChannelTransform {
        /// The channel the transform sat on (e.g. `x`).
        channel: String,
        /// The transform key as written (e.g. `bin`, `count`).
        transform: String,
    },

    /// A binned mark's `fill`/`stroke` named a CSS colour keyword that is ALSO
    /// a column of the inline source the mark reads — `fill: gold` over rows
    /// with a `gold` column.
    ///
    /// **Advisory only — nothing about the render changes.** The colour reading
    /// stands and the histogram lifts, so its bins carry whole-row counts. All
    /// this warning does is tell an author who meant the COLUMN that the spec
    /// does not say so; see `shadowed_colour` for why that is a warning and
    /// not a refusal.
    ///
    /// **It offers no remedy, because there is none.** The obvious one —
    /// `z: {name}` to mean the column — is dead advice: `z` is in
    /// `GROUPING_CHANNEL_FIELDS`, so it refuses the lift, and nothing
    /// downstream reads it either (`brightfield-render`'s `Channel` has no `z`
    /// variant and no `"z"` arm in `from_wire`). A spec that takes the advice
    /// draws a blank frame.
    ///
    /// **The bars are painted the named colour.** `brightfield_render`'s
    /// `ChannelMap::from_mark` binds a colour-channel string this module
    /// classifies with [`is_colour_literal`] as constant ink rather than as a
    /// column name, and `resolve_colour` returns it. The message said the
    /// opposite until the keyword table landed, which was true when written and
    /// is the reason this paragraph exists rather than a bare statement.
    ///
    /// So the only thing the author loses is the SPLIT, and that is all the
    /// message now claims.
    ///
    /// Only ever raised where the shadow is provable — see
    /// `inline_source_columns`. A `file:` or `query:` source's schema is not
    /// in the document, so the same collision there stays silent.
    ColourNameShadowsColumn {
        /// The channel it sat on (`fill` or `stroke`).
        field: String,
        /// The name that is both a colour and a column.
        name: String,
        /// The data source that has a column by that name.
        source: String,
    },
}

impl fmt::Display for ParseWarning {
    /// One honest line per warning — what was seen and what it cost.
    ///
    /// This exists because a warning nobody can read is a warning nobody
    /// receives: the window renders these strings, and a `{:?}` dump of a
    /// struct variant is not something to put in front of a person.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOption { path, key } => {
                write!(f, "unknown option `{key}` at {path} — ignored")
            }
            Self::Unimplemented {
                name,
                surface,
                status,
            } => write!(
                f,
                "{} `{name}` is {status} — it parses but does not render",
                surface.label()
            ),
            Self::VersionMismatch {
                declared,
                supported,
            } => write!(
                f,
                "spec declares Mosaic version {declared}; this build targets {supported}"
            ),
            Self::DeadParam { name } => {
                write!(f, "param `{name}` has no subscribers — nothing reads it")
            }
            Self::ParamTypeMismatch {
                param,
                expected,
                widget_kind,
            } => write!(
                f,
                "input `{widget_kind}` writes to param `{param}`, declared {expected}"
            ),
            Self::InteractorBindingMissing { name } => write!(
                f,
                "interactor binds `{name}`, which no `params:` entry declares"
            ),
            Self::InteractorBindingNonSelection { name } => write!(
                f,
                "interactor binds `{name}`, which is declared a value, not a selection"
            ),
            Self::LegendBindingMissing { name } => write!(
                f,
                "legend binds `{name}`, which no `params:` entry declares — legend stays display-only"
            ),
            Self::LegendBindingNonSelection { name } => write!(
                f,
                "legend binds `{name}`, which is declared a value, not a selection — legend stays display-only"
            ),
            Self::UnknownAggregate { field, name } => write!(
                f,
                "channel `{field}` names aggregate `{name}`, which is not a known aggregate — channel degrades"
            ),
            Self::LegendBindingNonCrossfilter { name, resolution } => write!(
                f,
                "legend binds `{name}`, resolved `{resolution}` rather than crossfilter — legend stays display-only"
            ),
            Self::NonNumericInset { attribute } => write!(
                f,
                "plot attribute `{attribute}` is not a number — inset falls back to the default"
            ),
            Self::HighlightBindingMissing { name } => write!(
                f,
                "highlight reads `{name}`, which is neither declared nor bound — highlight stays inert"
            ),
            Self::HighlightBindingNonSelection { name } => write!(
                f,
                "highlight reads `{name}`, which is declared a value, not a selection — highlight stays inert"
            ),
            Self::HighlightOnAggregate { selection, mark } => write!(
                f,
                "highlight on `{selection}` sits over aggregating mark `{mark}` — highlight is guarded off"
            ),
            Self::NonStringLabel { attribute } => write!(
                f,
                "plot attribute `{attribute}` is not a string — the label falls back to its derived form"
            ),
            Self::UnknownProjection { value } => write!(
                f,
                "projection `{value}` is not supported — the plot draws unprojected"
            ),
            Self::AspectRatioWithProjection { mark } => write!(
                f,
                "mark `{mark}` sets `aspectRatio` on a plot with a `projectionType` — the projection decides the frame and `aspectRatio` is dropped"
            ),
            Self::MarkCannotProject { mark, projection } => write!(
                f,
                "mark `{mark}` cannot draw through the plot's `{projection}` projection — it is not drawn, because its coordinates are not in the units the plot's axes are in"
            ),
            Self::IntervalBrushUnderCurvedProjection {
                interactor,
                projection,
            } => write!(
                f,
                "`{interactor}` cannot filter under the plot's `{projection}` projection — the axes do not invert separately, so no interval brush is installed"
            ),
            Self::UnconsumedMarkOption { mark, key } => write!(
                f,
                "mark `{mark}` sets `{key}`, which nothing in the render path reads — it has no effect"
            ),
            // The key IS read, so the line has to say that it is the SHAPE
            // that was refused — an author told "`sort` has no effect" would
            // go looking for the missing reader rather than at what they
            // wrote. It names the `limit:` too, for the reason the nested
            // `sort.limit` line did before this warning replaced it.
            Self::UnconsumedSort { mark } => write!(
                f,
                "mark `{mark}` asks for a `sort:` brightfield does not compute — the rows keep \
                 the order the query returned them in, and a `limit:` written beside it is \
                 dropped with it"
            ),
            // "no literal", not "no value". A slider may well declare
            // `min: $lo`, and telling its author they declared no `min:` sends
            // them looking for a line that is already there. The rail needs the
            // ends as numbers it can draw against; a param reference is a value
            // the spec has and this control cannot use.
            Self::IntervalSliderIncomplete { path, missing } => write!(
                f,
                "interval slider at {path} gives no literal {} — no control is drawn for it",
                missing
                    .iter()
                    .map(|k| format!("`{k}:`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            // Says the cost, not just the fact. `x: {bin: t}` does not degrade
            // the mark, it empties it — and the author is looking at a blank
            // frame, so the line has to account for the whole blank frame or
            // they will keep looking elsewhere.
            Self::UnconsumedChannelTransform { channel, transform } => write!(
                f,
                "channel `{channel}` asks for `{transform}`, which brightfield does not compute — \
                 the channel resolves to nothing and the mark draws no ink"
            ),
            // States the reading and the ONE thing the author does not get,
            // and stops. No remedy is offered; the variant doc says why.
            //
            // It said "and the bars stay the default colour" too, until
            // brightfield learned to resolve CSS keywords. That clause is now
            // false — the bars ARE the named colour — and a warning that
            // overstates the cost sends an author looking for a second problem
            // that is not there.
            Self::ColourNameShadowsColumn {
                field,
                name,
                source,
            } => write!(
                f,
                "`{field}: {name}` names a CSS colour and also a column of `{source}` — it is \
                 read as a colour constant, the same as Mosaic, so each bin counts every row in \
                 it rather than splitting by `{name}`. Brightfield cannot group a binned rect \
                 yet, so that split is not available from this spec"
            ),
        }
    }
}

/// Result of a successful parse.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParseOutput {
    /// The parsed Spec.
    pub spec: Spec,
    /// Non-fatal observations.
    pub warnings: Vec<ParseWarning>,
    /// The parent directory of the source file, if parsed from a path.
    /// `None` when parsed from a string. Used by the SQL emitter to resolve
    /// relative `file:` paths against the spec's location.
    pub base_dir: Option<std::path::PathBuf>,
}

/// Parse a Mosaic spec from source text.
///
/// # Errors
/// Returns [`ParseError::YamlSyntax`] / [`ParseError::JsonSyntax`] if the
/// input is not a well-formed document in the declared format;
/// [`ParseError::UnknownName`] for vocabulary not in the registry; and
/// various [`ParseError::SchemaViolation`] / [`ParseError::MalformedDataDef`]
/// / [`ParseError::MalformedParamDef`] variants for structural failures.
pub fn parse_spec(source: &str, format: Format) -> Result<ParseOutput, ParseError> {
    let value: serde_yaml::Value = match format {
        Format::Yaml => serde_yaml::from_str(source).map_err(|e| ParseError::YamlSyntax {
            msg: e.to_string(),
            span: e.location().map(|l| SourceSpan::point(l.index())),
        })?,
        Format::Json => {
            // Route JSON through serde_json first to get JSON-shaped error messages,
            // then convert into a serde_yaml::Value for unified walking.
            let jv: serde_json::Value =
                serde_json::from_str(source).map_err(|e| ParseError::JsonSyntax {
                    msg: e.to_string(),
                    span: None,
                })?;
            json_to_yaml_value(&jv)
        }
    };

    let mut walker = Walker::default();
    let spec = walker.walk_spec(&value)?;
    Ok(ParseOutput {
        spec,
        warnings: walker.warnings,
        base_dir: None,
    })
}

/// Parse a Mosaic spec from a path. Format is sniffed from the extension
/// (`.yaml` / `.yml` → YAML; `.json` → JSON).
///
/// # Errors
/// Returns [`ParseError::UnknownFormat`] for unrecognised extensions,
/// [`ParseError::Io`] for read failures, or any error from [`parse_spec`].
pub fn parse_spec_path(path: impl AsRef<Path>) -> Result<ParseOutput, ParseError> {
    let path = path.as_ref();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let format = match ext.as_str() {
        "yaml" | "yml" => Format::Yaml,
        "json" => Format::Json,
        other => {
            return Err(ParseError::UnknownFormat {
                ext: other.to_string(),
            })
        }
    };
    let source = std::fs::read_to_string(path)?;
    let mut output = parse_spec(&source, format)?;
    output.base_dir = path.parent().map(std::path::Path::to_path_buf);
    Ok(output)
}

// ---------------------------------------------------------------------------
// Walker: Value → typed AST
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Walker {
    warnings: Vec<ParseWarning>,
    /// Column names per INLINE data source — the only schema a spec carries in
    /// itself. Harvested from the raw document before anything is walked,
    /// because YAML key order is the author's and `plot:` may precede `data:`.
    /// See `inline_source_columns`.
    inline_columns: InlineColumns,
}

impl Walker {
    fn walk_spec(&mut self, v: &serde_yaml::Value) -> Result<Spec, ParseError> {
        let map = match v {
            serde_yaml::Value::Null => return Ok(Spec::default()),
            serde_yaml::Value::Mapping(m) => m,
            _ => {
                return Err(ParseError::SchemaViolation {
                    path: String::new(),
                    detail: "root must be a mapping".into(),
                    span: None,
                })
            }
        };

        // Before the walk, not during it: a mark's colour/column disambiguation
        // needs the schema of a source that may be declared further down the
        // file than the mark that reads it.
        self.inline_columns = inline_source_columns(map);

        let mut spec = Spec::default();
        let mut root_map = serde_yaml::Mapping::new();

        for (k, val) in map {
            let key = match k.as_str() {
                Some(s) => s,
                None => continue,
            };
            match key {
                "meta" => spec.meta = Some(self.walk_meta(val)?),
                "data" => spec.data = self.walk_data_block(val)?,
                "params" => spec.params = self.walk_params_block(val)?,
                "config" => spec.config = Config(self.walk_open_map(val, "config")?),
                "plotDefaults" => {
                    spec.plot_defaults = PlotDefaults(self.walk_open_map(val, "plotDefaults")?);
                }
                _ => {
                    // Anything else is a root-level component key (plot, vconcat,
                    // hconcat, hspace, vspace, legend, or a mark discriminator).
                    root_map.insert(k.clone(), val.clone());
                }
            }
        }

        if !root_map.is_empty() {
            let root_value = serde_yaml::Value::Mapping(root_map);
            spec.root = Some(self.walk_component(&root_value)?);
        }

        // Version mismatch warning (major.minor only).
        if let Some(meta) = &spec.meta {
            if let Some(declared) = &meta.version {
                if !version_matches(declared) {
                    self.warnings.push(ParseWarning::VersionMismatch {
                        declared: declared.clone(),
                        supported: crate::SUPPORTED_MOSAIC_VERSION,
                    });
                }
            }
        }

        Ok(spec)
    }

    fn walk_meta(&mut self, v: &serde_yaml::Value) -> Result<Meta, ParseError> {
        let map = match v {
            serde_yaml::Value::Mapping(m) => m,
            _ => {
                return Err(ParseError::SchemaViolation {
                    path: "meta".into(),
                    detail: "meta must be a mapping".into(),
                    span: None,
                })
            }
        };
        let mut meta = Meta::default();
        for (k, val) in map {
            let key = k.as_str().unwrap_or("");
            match key {
                "title" => meta.title = val.as_str().map(str::to_string),
                "description" => meta.description = val.as_str().map(str::to_string),
                "version" => meta.version = val.as_str().map(str::to_string),
                other => {
                    // Detour 2026-04-20: Corpus totality evidence (20/54 files
                    // declare `meta.credit`; one uses `meta.descriptions`)
                    // forced narrowing meta strictness further. We accept
                    // unknown meta keys as a non-fatal warning rather than
                    // fail the corpus. Typed accessors remain for
                    // title/description/version.
                    self.warnings.push(ParseWarning::UnknownOption {
                        path: "meta".into(),
                        key: other.to_string(),
                    });
                }
            }
            // Strict-context $param detection on string fields (applies to
            // both typed and unknown scalar fields under meta).
            if let Some(s) = val.as_str() {
                if let Some(name) = dollar_ident(s) {
                    return Err(ParseError::StrictContextUnresolvedRef {
                        field_path: format!("meta.{key}"),
                        name: name.to_string(),
                        span: None,
                    });
                }
            }
        }
        Ok(meta)
    }

    fn walk_open_map(
        &mut self,
        v: &serde_yaml::Value,
        head: &str,
    ) -> Result<IndexMap<String, SpecValue>, ParseError> {
        let map = match v {
            serde_yaml::Value::Mapping(m) => m,
            _ => {
                return Err(ParseError::SchemaViolation {
                    path: head.into(),
                    detail: format!("{head} must be a mapping"),
                    span: None,
                })
            }
        };
        let mut out = IndexMap::new();
        for (k, val) in map {
            let key = k
                .as_str()
                .ok_or_else(|| ParseError::SchemaViolation {
                    path: head.into(),
                    detail: "non-string key".into(),
                    span: None,
                })?
                .to_string();
            // Strict $param detection on scalar string fields under open heads
            // (constraint #4e still says strict-context detection fires on
            // string-typed fields under meta/config/plotDefaults).
            if let Some(s) = val.as_str() {
                if let Some(name) = dollar_ident(s) {
                    return Err(ParseError::StrictContextUnresolvedRef {
                        field_path: format!("{head}.{key}"),
                        name: name.to_string(),
                        span: None,
                    });
                }
            }
            out.insert(key, self.spec_value(val));
        }
        Ok(out)
    }

    fn walk_data_block(
        &mut self,
        v: &serde_yaml::Value,
    ) -> Result<IndexMap<String, DataSource>, ParseError> {
        let map = match v {
            serde_yaml::Value::Mapping(m) => m,
            _ => {
                return Err(ParseError::SchemaViolation {
                    path: "data".into(),
                    detail: "data must be a mapping".into(),
                    span: None,
                })
            }
        };
        let mut out = IndexMap::new();
        for (k, val) in map {
            let name = k
                .as_str()
                .ok_or_else(|| ParseError::MalformedDataDef {
                    span: None,
                    detail: "non-string data source name".into(),
                })?
                .to_string();
            out.insert(name, self.walk_data_source(val)?);
        }
        Ok(out)
    }

    fn walk_data_source(&mut self, v: &serde_yaml::Value) -> Result<DataSource, ParseError> {
        match v {
            serde_yaml::Value::String(s) => Ok(DataSource {
                kind: DataSourceKind::Shorthand(s.clone()),
                extras: IndexMap::new(),
            }),
            serde_yaml::Value::Sequence(seq) => Ok(DataSource {
                kind: DataSourceKind::InlineRows(seq.iter().map(|x| self.spec_value(x)).collect()),
                extras: IndexMap::new(),
            }),
            serde_yaml::Value::Mapping(m) => {
                let mut extras = IndexMap::new();
                let mut kind: Option<DataSourceKind> = None;
                for (k, val) in m {
                    let key = k.as_str().unwrap_or("").to_string();
                    match key.as_str() {
                        "file" => {
                            if let Some(s) = val.as_str() {
                                kind = Some(DataSourceKind::File(s.to_string()));
                            } else {
                                return Err(ParseError::MalformedDataDef {
                                    span: None,
                                    detail: "data.file must be a string".into(),
                                });
                            }
                        }
                        "query" => {
                            let s = match val {
                                serde_yaml::Value::String(s) => s.clone(),
                                _ => {
                                    return Err(ParseError::MalformedDataDef {
                                        span: None,
                                        detail: "data.query must be a string".into(),
                                    })
                                }
                            };
                            kind = Some(DataSourceKind::Query(s));
                        }
                        "type" => {
                            if let Some(s) = val.as_str() {
                                kind.get_or_insert(DataSourceKind::Typed(s.to_string()));
                                extras.insert(key, self.spec_value(val));
                            }
                        }
                        _ => {
                            extras.insert(key, self.spec_value(val));
                        }
                    }
                }
                Ok(DataSource {
                    kind: kind.unwrap_or(DataSourceKind::Opaque),
                    extras,
                })
            }
            _ => Err(ParseError::MalformedDataDef {
                span: None,
                detail: "data source must be a string, array, or mapping".into(),
            }),
        }
    }

    fn walk_params_block(
        &mut self,
        v: &serde_yaml::Value,
    ) -> Result<IndexMap<String, ParamNode>, ParseError> {
        let map = match v {
            serde_yaml::Value::Mapping(m) => m,
            _ => {
                return Err(ParseError::SchemaViolation {
                    path: "params".into(),
                    detail: "params must be a mapping".into(),
                    span: None,
                })
            }
        };
        let mut out = IndexMap::new();
        for (k, val) in map {
            let name = k
                .as_str()
                .ok_or_else(|| ParseError::MalformedParamDef {
                    name: String::new(),
                    span: None,
                    detail: "non-string param name".into(),
                })?
                .to_string();
            out.insert(name.clone(), self.walk_param(&name, val)?);
        }
        Ok(out)
    }

    fn walk_param(&mut self, name: &str, v: &serde_yaml::Value) -> Result<ParamNode, ParseError> {
        if let serde_yaml::Value::Mapping(m) = v {
            if let Some(sel) = m.get(serde_yaml::Value::String("select".into())) {
                let resolution_name =
                    sel.as_str().ok_or_else(|| ParseError::MalformedParamDef {
                        name: name.to_string(),
                        span: None,
                        detail: "select must be a string resolution name".into(),
                    })?;
                let (kind, status) = match SelectionResolution::from_wire(resolution_name) {
                    Some(k) => (k, k.status()),
                    None => {
                        return Err(ParseError::UnknownName {
                            name: resolution_name.to_string(),
                            surface: NameSurface::Interactor,
                            span: None,
                        });
                    }
                };
                let mut options = IndexMap::new();
                for (k, val) in m {
                    let key = k.as_str().unwrap_or("").to_string();
                    if key == "select" {
                        continue;
                    }
                    options.insert(key, self.spec_value(val));
                }
                return Ok(ParamNode::Selection(SelectionNode {
                    select: kind,
                    status,
                    options,
                }));
            }
        }
        Ok(ParamNode::Value(self.spec_value(v)))
    }

    fn walk_component(&mut self, v: &serde_yaml::Value) -> Result<Component, ParseError> {
        let map = match v {
            serde_yaml::Value::Mapping(m) => m,
            _ => {
                return Err(ParseError::SchemaViolation {
                    path: "<component>".into(),
                    detail: "component must be a mapping".into(),
                    span: None,
                })
            }
        };

        // Discriminator precedence per Mosaic: plot | vconcat | hconcat |
        // hspace | vspace | legend | mark | select (interactor) | input.
        let has = |k: &str| -> Option<&serde_yaml::Value> {
            map.get(serde_yaml::Value::String(k.into()))
        };

        if let Some(items) = has("plot") {
            return Ok(Component::Plot(self.walk_plot(items, map)?));
        }
        if let Some(items) = has("vconcat") {
            return Ok(Component::VConcat(self.walk_concat(items)?));
        }
        if let Some(items) = has("hconcat") {
            return Ok(Component::HConcat(self.walk_concat(items)?));
        }
        if let Some(val) = has("hspace") {
            return Ok(Component::HSpace(SpaceNode {
                value: self.spec_value(val),
            }));
        }
        if let Some(val) = has("vspace") {
            return Ok(Component::VSpace(SpaceNode {
                value: self.spec_value(val),
            }));
        }
        if let Some(channel_val) = has("legend") {
            return Ok(Component::Legend(self.walk_legend(channel_val, map)?));
        }
        if let Some(mark_name) = has("mark") {
            return Ok(Component::Mark(self.walk_mark(mark_name, map)?));
        }
        if let Some(select_name) = has("select") {
            return Ok(Component::Interactor(
                self.walk_interactor(select_name, map)?,
            ));
        }
        if let Some(input_name) = has("input") {
            return Ok(Component::Input(self.walk_input(input_name, map)?));
        }

        Err(ParseError::SchemaViolation {
            path: "<component>".into(),
            detail: "no recognised discriminator (plot, vconcat, hconcat, hspace, vspace, legend, mark, select, input)".into(),
            span: None,
        })
    }

    fn walk_plot(
        &mut self,
        items: &serde_yaml::Value,
        parent: &serde_yaml::Mapping,
    ) -> Result<PlotNode, ParseError> {
        let seq = match items {
            serde_yaml::Value::Sequence(s) => s,
            _ => {
                return Err(ParseError::SchemaViolation {
                    path: "plot".into(),
                    detail: "plot must be a sequence".into(),
                    span: None,
                })
            }
        };
        let mut plot_items = Vec::with_capacity(seq.len());
        for item in seq {
            plot_items.push(self.walk_component(item)?);
        }
        // Plot-level inset attributes (axis-inset round). Distinct
        // from the mark-level `inset*` names in the attribute allowlist — these
        // resolve into positional-scale range insets.
        const PLOT_INSET_KEYS: [&str; 7] = [
            "inset",
            "xInset",
            "yInset",
            "xInsetLeft",
            "xInsetRight",
            "yInsetTop",
            "yInsetBottom",
        ];
        let mut attributes = IndexMap::new();
        for (k, val) in parent {
            let key = k.as_str().unwrap_or("").to_string();
            if key == "plot" {
                continue;
            }
            let value = self.spec_value(val);
            // A non-numeric inset value degrades to "absent" for range
            // insetting; name it so the author sees the typo. A lifted `$param`
            // is an intentional (recorded) deferral, not a typo — don't warn.
            if PLOT_INSET_KEYS.contains(&key.as_str())
                && !matches!(
                    value,
                    SpecValue::Integer(_) | SpecValue::Float(_) | SpecValue::Param(_)
                )
            {
                self.warnings.push(ParseWarning::NonNumericInset {
                    attribute: key.clone(),
                });
            }
            // A plot-level axis / plot title attribute. A valid
            // label is a string (override), or `null` / `""` (suppress); a
            // lifted `$param` is a recorded deferral. Anything else (number,
            // boolean, …) degrades to the derived title — name it so the author
            // sees the typo rather than silently losing the label.
            const PLOT_LABEL_KEYS: [&str; 3] = ["xLabel", "yLabel", "title"];
            if PLOT_LABEL_KEYS.contains(&key.as_str())
                && !matches!(
                    value,
                    SpecValue::String(_) | SpecValue::Null | SpecValue::Param(_)
                )
            {
                self.warnings.push(ParseWarning::NonStringLabel {
                    attribute: key.clone(),
                });
            }
            // A plot-level `projectionType` (geo) that names a
            // projection v1 can't render (or a non-string value) degrades to the
            // default equirectangular fit — name it so the author sees the
            // unsupported projection. A lifted `$param` is a recorded deferral.
            if key == "projectionType" {
                self.warn_unknown_projection(&value);
            }
            attributes.insert(key, value);
        }
        let node = PlotNode {
            items: plot_items,
            attributes,
        };
        self.warn_plot_projection(&node);
        Ok(node)
    }

    /// Name what a plot's `projectionType` costs the items inside it.
    ///
    /// A projection is a PLOT attribute and it replaces the plot's x and y
    /// scales, so it is not a per-mark decision and the marks cannot each answer
    /// it differently. Three consequences, and an author is told about each
    /// rather than shown a picture that quietly means something else:
    ///
    /// - a mark whose kind cannot project ([`MarkKind::draws_through_a_projection`])
    ///   is not drawn, because its degrees would land arbitrarily on axes in the
    ///   projection's planar units;
    /// - a mark asking for `aspectRatio: 1` has already had that question
    ///   answered, better, by the projection;
    /// - an interval brush over a projection whose axes do not invert
    ///   separately is not installed, because its `BETWEEN` bounds would be
    ///   bounds nobody swept.
    ///
    /// Runs over the built [`PlotNode`] rather than over the YAML, so it asks
    /// [`crate::layout::resolve_projection`] — the same function the renderer
    /// asks — instead of re-reading the attribute here.
    fn warn_plot_projection(&mut self, node: &PlotNode) {
        let Some(projection) = crate::layout::resolve_projection(node) else {
            return;
        };
        let name = match node.attributes.get("projectionType") {
            Some(SpecValue::String(s)) => s.clone(),
            _ => String::new(),
        };
        for item in &node.items {
            match item {
                Component::Mark(mark) => {
                    if mark.status != ImplStatus::Implemented {
                        continue;
                    }
                    if !mark.kind.draws_through_a_projection() {
                        self.warnings.push(ParseWarning::MarkCannotProject {
                            mark: mark.kind.wire_name().to_string(),
                            projection: name.clone(),
                        });
                        continue;
                    }
                    let asks_aspect = matches!(
                        mark.options.get("aspectRatio"),
                        Some(ValueOrParamRef::Value(SpecValue::Integer(1)))
                    ) || matches!(
                        mark.options.get("aspectRatio"),
                        Some(ValueOrParamRef::Value(SpecValue::Float(f))) if (*f - 1.0).abs() < f64::EPSILON
                    );
                    if asks_aspect {
                        self.warnings.push(ParseWarning::AspectRatioWithProjection {
                            mark: mark.kind.wire_name().to_string(),
                        });
                    }
                }
                Component::Interactor(interactor) => {
                    let is_interval = matches!(
                        interactor.kind,
                        InteractorKind::IntervalX
                            | InteractorKind::IntervalY
                            | InteractorKind::IntervalXY
                    );
                    if is_interval && !projection.axes_invert_separately() {
                        self.warnings
                            .push(ParseWarning::IntervalBrushUnderCurvedProjection {
                                interactor: interactor.kind.wire_name().to_string(),
                                projection: name.clone(),
                            });
                    }
                }
                _ => {}
            }
        }
    }

    fn walk_concat(&mut self, items: &serde_yaml::Value) -> Result<ConcatNode, ParseError> {
        let seq = match items {
            serde_yaml::Value::Sequence(s) => s,
            _ => {
                return Err(ParseError::SchemaViolation {
                    path: "vconcat|hconcat".into(),
                    detail: "concat value must be a sequence".into(),
                    span: None,
                })
            }
        };
        let mut out = Vec::with_capacity(seq.len());
        for item in seq {
            out.push(self.walk_component(item)?);
        }
        Ok(ConcatNode { items: out })
    }

    fn walk_legend(
        &mut self,
        channel_val: &serde_yaml::Value,
        parent: &serde_yaml::Mapping,
    ) -> Result<LegendNode, ParseError> {
        let name = channel_val
            .as_str()
            .ok_or_else(|| ParseError::SchemaViolation {
                path: "legend".into(),
                detail: "legend value must be a channel name".into(),
                span: None,
            })?;
        let channel = LegendChannel::from_wire(name).ok_or_else(|| ParseError::UnknownName {
            name: name.to_string(),
            surface: NameSurface::LegendChannel,
            span: None,
        })?;
        let status = channel.status();
        if status != ImplStatus::Implemented {
            self.warnings.push(ParseWarning::Unimplemented {
                name: name.to_string(),
                surface: NameSurface::LegendChannel,
                status,
            });
        }
        let mut options = IndexMap::new();
        for (k, val) in parent {
            let key = k.as_str().unwrap_or("").to_string();
            if key == "legend" {
                continue;
            }
            let lifted = self.lift_field(&key, val);
            options.insert(key, lifted);
        }
        Ok(LegendNode {
            channel,
            status,
            options,
        })
    }

    fn walk_mark(
        &mut self,
        mark_name_v: &serde_yaml::Value,
        parent: &serde_yaml::Mapping,
    ) -> Result<Mark, ParseError> {
        let name = mark_name_v
            .as_str()
            .ok_or_else(|| ParseError::SchemaViolation {
                path: "mark".into(),
                detail: "mark discriminator must be a string".into(),
                span: None,
            })?;
        let kind = MarkKind::from_wire(name).ok_or_else(|| ParseError::UnknownName {
            name: name.to_string(),
            surface: NameSurface::Mark,
            span: None,
        })?;
        let status = kind.status();
        if status != ImplStatus::Implemented {
            self.warnings.push(ParseWarning::Unimplemented {
                name: name.to_string(),
                surface: NameSurface::Mark,
                status,
            });
        }

        // Resolved BEFORE the per-key loop, because it is a property of the
        // PAIR: a `bin` is only computable when the opposite positional channel
        // counts, and vice versa. Deciding it per key would let `x` lift while
        // `y` warned. The shadow check sits outside [`binned_histogram`]
        // because it only reports and never changes the lift.
        let histogram = binned_histogram(kind, parent);
        if histogram.is_some() {
            if let Some(shadow) = shadowed_colour(parent, &self.inline_columns) {
                self.warnings.push(ParseWarning::ColourNameShadowsColumn {
                    field: shadow.field.to_string(),
                    name: shadow.name,
                    source: shadow.source,
                });
            }
        }

        // Resolved BEFORE the per-key loop for the same reason the histogram
        // is: an aggregate on `x` is only computable when `y` names a category
        // to group it by, so the decision belongs to the PAIR. The two idioms
        // are disjoint by mark kind — no kind both bins positionally and has a
        // band axis — but the histogram is asked first so that stays true by
        // construction rather than by coincidence.
        let banded = match histogram {
            Some(_) => None,
            None => banded_aggregate(kind, parent),
        };

        // Resolved here rather than in the per-key loop for the same reason:
        // whether `sort: {y: -x}` is computable is decided by the mark KIND,
        // which names which axis is the band and which the value.
        let sort = mark_sort(kind, parent);

        let mut data: Option<MarkData> = None;
        let mut options = IndexMap::new();
        for (k, val) in parent {
            let key = k.as_str().unwrap_or("").to_string();
            if key == "mark" {
                continue;
            }
            if key == "data" {
                data = Some(self.walk_mark_data(val)?);
                continue;
            }
            if let Some(h) = &histogram {
                if key == h.bin_channel {
                    options.insert(
                        key.clone(),
                        ValueOrParamRef::Value(SpecValue::Bin {
                            column: h.column.clone(),
                            steps: h.steps,
                        }),
                    );
                    continue;
                }
                if key == h.count_channel {
                    options.insert(
                        key.clone(),
                        ValueOrParamRef::Value(SpecValue::Aggregate {
                            func: AggregateFunc::Count,
                            column: None,
                        }),
                    );
                    continue;
                }
            }
            if let Some(b) = &banded {
                if key == b.value_channel {
                    options.insert(
                        key.clone(),
                        ValueOrParamRef::Value(SpecValue::Aggregate {
                            func: b.func,
                            column: b.column.clone(),
                        }),
                    );
                    continue;
                }
            }
            if let Some(s) = &sort {
                if key == "sort" {
                    options.insert(
                        key.clone(),
                        ValueOrParamRef::Value(SpecValue::Sort {
                            channel: s.channel.to_string(),
                            by: s.by.to_string(),
                            descending: s.descending,
                            limit: s.limit,
                        }),
                    );
                    continue;
                }
            }
            if let Some(lifted) = self.maybe_aggregate_channel(&key, val) {
                options.insert(key.clone(), lifted);
                continue;
            }
            self.warn_unconsumed_channel_transform(&key, val);
            options.insert(key.clone(), self.lift_field(&key, val));
        }
        if status == ImplStatus::Implemented {
            self.warn_unconsumed_mark_options(name, parent);
            if sort.is_none() && parent.contains_key(serde_yaml::Value::String("sort".into())) {
                self.warnings
                    .push(ParseWarning::UnconsumedSort { mark: name.into() });
            }
        }
        Ok(Mark {
            kind,
            status,
            data,
            options,
        })
    }

    /// Name a transform sitting on a channel the renderer maps but no lowerer
    /// computes, so an author who wrote `x: {bin: t}` and got a blank frame is
    /// told which channel and which transform emptied it.
    ///
    /// Fires for a map on a [`RENDERED_CHANNEL_FIELDS`] channel whose leading
    /// key is neither a lift (`{param: …}` / `{selection: …}`) nor a recognised
    /// channel transform ([`CHANNEL_TRANSFORM_KEYS`]).
    ///
    /// **Multi-key maps count.** A transform can carry modifiers beside it —
    /// `x: {bin: plddt_total, steps: 60}`, `y: {avg: cases, orderby: day, rows:
    /// $frame}` — and the corpus vendored in this crate holds five such maps
    /// across `protein-design`, `moving-average` and `window-frame`. Skipping
    /// them was worse than saying nothing: `protein-design` binds `bin` on `x`
    /// and `count` on `y` of the *same* mark, so a single-key-only check named
    /// the `count`, stayed silent on the `bin`, and left an author who fixed the
    /// named half still looking at a blank frame with nothing to explain it. A
    /// partial diagnostic on a two-part failure is a wrong diagnostic.
    ///
    /// The transform is the **leading** key in all five, so that is what gets
    /// named. If a spec ever put the modifier first the line would name the
    /// modifier — still pointing at the right channel, and still true that the
    /// channel is uncomputed, so it degrades honestly rather than silently.
    ///
    /// Aggregate-capable channels reach here only when multi-key. The caller
    /// tries [`Self::maybe_aggregate_channel`] first, and for a **single**-key
    /// non-lift non-`sql` map on `fill`/`r` that always returns `Some` —
    /// consuming a recognised aggregate, or warning
    /// [`ParseWarning::UnknownAggregate`] for an unrecognised one. Those never
    /// arrive.
    ///
    /// A multi-key one does, because `maybe_aggregate_channel` bails on
    /// `m.len() != 1`. `fill: {avg: c, orderby: t}` is therefore reported here
    /// as an uncomputed `avg` on `fill`, which is the accurate line: brightfield
    /// computes no windowed average on any channel. It is still exactly ONE
    /// line — the aggregate path returned before its own warn — so there is no
    /// guard for them here and no double report.
    fn warn_unconsumed_channel_transform(&mut self, field: &str, v: &serde_yaml::Value) {
        if !RENDERED_CHANNEL_FIELDS.contains(&field) {
            return;
        }
        // `{param: name}` / `{selection: name}` is a param lift the lowerer
        // does read — ordinary lifting handles it.
        if maybe_lift(v).is_some() {
            return;
        }
        let serde_yaml::Value::Mapping(m) = v else {
            return;
        };
        // An empty map asks for nothing and is not a transform.
        if m.is_empty() {
            return;
        }
        let Some(key) = m.iter().next().and_then(|(k, _)| k.as_str()) else {
            return;
        };
        if CHANNEL_TRANSFORM_KEYS.contains(&key) {
            return;
        }
        self.warnings
            .push(ParseWarning::UnconsumedChannelTransform {
                channel: field.to_string(),
                transform: key.to_string(),
            });
    }

    /// Raise [`ParseWarning::UnknownProjection`] when `value` is a
    /// `projectionType` this build cannot draw.
    ///
    /// ONE function for the plot attribute and the mark option, asking
    /// [`crate::layout::ResolvedProjection::from_wire`] — the same question the
    /// renderer's resolution asks. A lifted `$param` is a recorded deferral and
    /// not a bad name, so it does not warn; a value that is not a string cannot
    /// name a projection.
    fn warn_unknown_projection(&mut self, value: &SpecValue) {
        let unsupported = match value {
            SpecValue::String(s) => crate::layout::ResolvedProjection::from_wire(s).is_none(),
            SpecValue::Param(_) => false,
            _ => true,
        };
        if !unsupported {
            return;
        }
        let shown = match value {
            SpecValue::String(s) => s.clone(),
            _ => "<non-string>".to_string(),
        };
        self.warnings
            .push(ParseWarning::UnknownProjection { value: shown });
    }

    /// Name every option key on `mark_name`'s node that no lowerer and no
    /// renderer reads — see [`crate::vocab::CONSUMED_MARK_OPTION_KEYS`].
    ///
    /// Walked over the SOURCE mapping rather than the lifted option bag,
    /// because a key nested inside an ignored map (`sort: { y: -x, limit: 10 }`)
    /// is a distinct ignored knob and deserves its own line: an author who
    /// reads "`sort` has no effect" still does not know that the `limit: 10`
    /// they wrote is gone too. One level of nesting only — deeper than that
    /// and the path stops being something a person can find by eye.
    fn warn_unconsumed_mark_options(&mut self, mark_name: &str, parent: &serde_yaml::Mapping) {
        for (k, val) in parent {
            let Some(key) = k.as_str() else { continue };
            // `mark` is the discriminator and `data` is lifted into MarkData;
            // neither is an option.
            if key == "mark" || key == "data" || crate::vocab::mark_option_is_consumed(key) {
                continue;
            }
            self.warnings.push(ParseWarning::UnconsumedMarkOption {
                mark: mark_name.to_string(),
                key: key.to_string(),
            });
            if let serde_yaml::Value::Mapping(nested) = val {
                for (nk, _) in nested {
                    let Some(nested_key) = nk.as_str() else {
                        continue;
                    };
                    self.warnings.push(ParseWarning::UnconsumedMarkOption {
                        mark: mark_name.to_string(),
                        key: format!("{key}.{nested_key}"),
                    });
                }
            }
        }
    }

    fn walk_mark_data(&mut self, v: &serde_yaml::Value) -> Result<MarkData, ParseError> {
        match v {
            serde_yaml::Value::Sequence(seq) => Ok(MarkData::Inline(
                seq.iter().map(|x| self.spec_value(x)).collect(),
            )),
            serde_yaml::Value::Mapping(m) => {
                let mut source: Option<String> = None;
                let mut filter_by: Option<ParamRef> = None;
                let mut extras = IndexMap::new();
                for (k, val) in m {
                    let key = k.as_str().unwrap_or("").to_string();
                    match key.as_str() {
                        "from" => source = val.as_str().map(str::to_string),
                        "filterBy" => {
                            filter_by = maybe_lift(val);
                            if filter_by.is_none() {
                                // Preserve under extras if it wasn't liftable.
                                extras.insert(key, self.spec_value(val));
                            }
                        }
                        _ => {
                            extras.insert(key, self.spec_value(val));
                        }
                    }
                }
                match source {
                    Some(name) => Ok(MarkData::From {
                        source: name,
                        filter_by,
                        extras,
                    }),
                    None => Err(ParseError::SchemaViolation {
                        path: "mark.data".into(),
                        detail: "mark.data mapping must include `from:`".into(),
                        span: None,
                    }),
                }
            }
            _ => Err(ParseError::SchemaViolation {
                path: "mark.data".into(),
                detail: "mark.data must be a sequence or mapping".into(),
                span: None,
            }),
        }
    }

    fn walk_interactor(
        &mut self,
        kind_val: &serde_yaml::Value,
        parent: &serde_yaml::Mapping,
    ) -> Result<Interactor, ParseError> {
        let name = kind_val
            .as_str()
            .ok_or_else(|| ParseError::SchemaViolation {
                path: "select".into(),
                detail: "select discriminator must be a string".into(),
                span: None,
            })?;
        let kind = InteractorKind::from_wire(name).ok_or_else(|| ParseError::UnknownName {
            name: name.to_string(),
            surface: NameSurface::Interactor,
            span: None,
        })?;
        let status = kind.status();
        if status != ImplStatus::Implemented {
            self.warnings.push(ParseWarning::Unimplemented {
                name: name.to_string(),
                surface: NameSurface::Interactor,
                status,
            });
        }
        let mut options = IndexMap::new();
        for (k, val) in parent {
            let key = k.as_str().unwrap_or("").to_string();
            if key == "select" {
                continue;
            }
            options.insert(key.clone(), self.lift_field(&key, val));
        }
        Ok(Interactor {
            kind,
            status,
            options,
        })
    }

    fn walk_input(
        &mut self,
        kind_val: &serde_yaml::Value,
        parent: &serde_yaml::Mapping,
    ) -> Result<Input, ParseError> {
        let name = kind_val
            .as_str()
            .ok_or_else(|| ParseError::SchemaViolation {
                path: "input".into(),
                detail: "input discriminator must be a string".into(),
                span: None,
            })?;
        let kind = InputKind::from_wire(name).ok_or_else(|| ParseError::UnknownName {
            name: name.to_string(),
            surface: NameSurface::Input,
            span: None,
        })?;
        let status = kind.status();
        if status != ImplStatus::Implemented {
            self.warnings.push(ParseWarning::Unimplemented {
                name: name.to_string(),
                surface: NameSurface::Input,
                status,
            });
        }
        let mut options = IndexMap::new();
        let mut as_param: Option<ParamRef> = None;
        let mut from_source: Option<String> = None;
        let mut filter_by: Option<ParamRef> = None;
        for (k, val) in parent {
            let key = k.as_str().unwrap_or("").to_string();
            if key == "input" {
                continue;
            }
            match key.as_str() {
                "as" => {
                    let lifted = self.lift_field(&key, val);
                    if let ValueOrParamRef::Param(pr) = lifted {
                        as_param = Some(pr);
                    } else {
                        // Non-param `as:` value — store in options for compatibility
                        options.insert(key, lifted);
                    }
                }
                "filterBy" => {
                    let lifted = self.lift_field(&key, val);
                    if let ValueOrParamRef::Param(pr) = lifted {
                        filter_by = Some(pr);
                    } else {
                        options.insert(key, lifted);
                    }
                }
                "from" => {
                    if let Some(s) = val.as_str() {
                        from_source = Some(s.to_string());
                    } else {
                        let lifted = self.lift_field(&key, val);
                        options.insert(key, lifted);
                    }
                }
                _ => {
                    options.insert(key.clone(), self.lift_field(&key, val));
                }
            }
        }
        Ok(Input {
            kind,
            status,
            as_param,
            from_source,
            filter_by,
            options,
        })
    }

    /// Lift a self-aggregating channel transform (`fill: {count:}`,
    /// `fill: {avg: col}`, `r: {count:}`) at a mark channel position into a
    /// typed [`SpecValue::Aggregate`]. Returns `None` when the field is not an
    /// aggregate-capable channel or the value is not a single-key map — the
    /// caller then falls back to ordinary lifting, so plain column / literal /
    /// param channels are untouched.
    ///
    /// A single-key map on an aggregate channel whose key is NOT a recognised
    /// aggregate degrades: it warns ([`ParseWarning::UnknownAggregate`]) and is
    /// kept as a plain object (which the renderer's channel extraction ignores),
    /// so it is never silently read as a column named after the key.
    fn maybe_aggregate_channel(
        &mut self,
        field: &str,
        v: &serde_yaml::Value,
    ) -> Option<ValueOrParamRef<SpecValue>> {
        if !AGGREGATE_CHANNEL_FIELDS.contains(&field) {
            return None;
        }
        // A `{param: name}` / `{selection: name}` shorthand is a param lift, not
        // an aggregate — defer to ordinary lifting.
        if maybe_lift(v).is_some() {
            return None;
        }
        let serde_yaml::Value::Mapping(m) = v else {
            return None;
        };
        if m.len() != 1 {
            return None;
        }
        let (k, inner) = m.iter().next()?;
        let key = k.as_str()?;
        // A recognised channel transform (e.g. `{sql: …}`) is not an aggregate
        // and not a typo — defer to ordinary lifting (stored as a plain object),
        // no warning.
        if CHANNEL_TRANSFORM_KEYS.contains(&key) {
            return None;
        }
        match AggregateFunc::from_wire(key) {
            Some(func) => {
                // `{count:}` carries a null value → no column; `{avg: col}`
                // carries the source column name. A non-string, non-null inner
                // (e.g. a nested object) leaves the column `None`.
                let column = match inner {
                    serde_yaml::Value::Null => None,
                    other => other.as_str().map(str::to_string),
                };
                Some(ValueOrParamRef::Value(SpecValue::Aggregate {
                    func,
                    column,
                }))
            }
            None => {
                self.warnings.push(ParseWarning::UnknownAggregate {
                    field: field.to_string(),
                    name: key.to_string(),
                });
                // Degrade to a plain object — never a column lookup.
                Some(ValueOrParamRef::Value(self.spec_value(v)))
            }
        }
    }

    /// Produce a [`ValueOrParamRef`] for a field value. If the field name is
    /// on [`LIFT_SURFACE_FIELDS`] and the value is a lift-shaped form, the
    /// outer wrapper becomes [`ValueOrParamRef::Param`]; otherwise a
    /// [`SpecValue`] is stored.
    fn lift_field(&mut self, field: &str, v: &serde_yaml::Value) -> ValueOrParamRef<SpecValue> {
        if LIFT_SURFACE_FIELDS.contains(&field) {
            if let Some(r) = maybe_lift(v) {
                return ValueOrParamRef::Param(r);
            }
        }
        ValueOrParamRef::Value(self.spec_value(v))
    }

    /// Convert a raw YAML value to a [`SpecValue`]. Interior `"$ident"`
    /// strings lift to [`SpecValue::Param`]; SQL-like strings that contain a
    /// `$ident` outside literal contexts lift to [`SpecValue::Expression`].
    fn spec_value(&mut self, v: &serde_yaml::Value) -> SpecValue {
        match v {
            serde_yaml::Value::Null => SpecValue::Null,
            serde_yaml::Value::Bool(b) => SpecValue::Bool(*b),
            serde_yaml::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    SpecValue::Integer(i)
                } else if let Some(f) = n.as_f64() {
                    SpecValue::Float(f)
                } else {
                    SpecValue::Null
                }
            }
            serde_yaml::Value::String(s) => string_to_spec_value(s),
            serde_yaml::Value::Sequence(seq) => {
                SpecValue::Array(seq.iter().map(|x| self.spec_value(x)).collect())
            }
            serde_yaml::Value::Mapping(m) => {
                // `{param: name}` / `{selection: name}` interior shorthands
                // lift to SpecValue::Param.
                if let Some(r) = maybe_lift(v) {
                    return SpecValue::Param(r);
                }
                let mut out = IndexMap::new();
                for (k, val) in m {
                    let key = k.as_str().unwrap_or("").to_string();
                    out.insert(key, self.spec_value(val));
                }
                SpecValue::Object(out)
            }
            serde_yaml::Value::Tagged(tv) => self.spec_value(&tv.value),
        }
    }
}

/// Resolve a mark's positional `bin` + `count` pair, or `None` when the mark
/// is not the histogram idiom brightfield computes.
///
/// Every condition below is a refusal that keeps a spec's uncomputed-transform
/// diagnostic rather than drawing something wrong:
///
/// - **The mark kind** must bin positionally ([`MarkKind::bins_positionally`]).
/// - **Both halves** must be present, on opposite positional axes. A lone
///   `x: {bin: t}` has nothing to aggregate and a lone `y: {count:}` has
///   nothing to group by; either alone keeps warning.
/// - **The `bin` map's keys** must be `bin`, `steps`, or a
///   [`BIN_MODIFIER_KEYS`] entry. An unrecognised modifier refuses the lift
///   rather than dropping it: honouring the `bin` and ignoring the modifier
///   would draw a different chart from the one asked for.
/// - **No grouping channel** (`GROUPING_CHANNEL_FIELDS`). `RectLowerer`
///   groups on the bin edges alone, so lifting a grouped mark would collapse
///   its groups into one bar per bin — the right TOTAL, with the composition
///   the author asked for silently gone. A different chart, drawn confidently.
fn binned_histogram(kind: MarkKind, parent: &serde_yaml::Mapping) -> Option<BinnedHistogram> {
    if !kind.bins_positionally() || mark_is_grouped(parent) {
        return None;
    }
    BIN_COUNT_AXES
        .iter()
        .find_map(|(bin_channel, count_channel)| {
            let (column, steps) = bin_transform(channel_map_at(parent, bin_channel)?)?;
            is_count_transform(channel_map_at(parent, count_channel)?).then_some(BinnedHistogram {
                bin_channel,
                count_channel,
                column,
                steps,
            })
        })
}

/// Resolve a mark's positional aggregate over a band channel, or `None` when
/// the mark is not the ranked-category-bar idiom brightfield computes.
///
/// The shape is one aggregate on the mark's VALUE axis and a plain column on
/// its BAND axis — `x: {sum: gold}` with `y: nationality` on a `barX` — which
/// lowers to one `GROUP BY` on the band column and one aggregate call.
///
/// Like [`binned_histogram`], every condition below is a refusal that keeps a
/// spec's uncomputed-transform diagnostic rather than drawing something wrong:
///
/// - **The mark kind** must have a band axis
///   ([`MarkKind::band_aggregate_axes`]), and the aggregate must sit on the
///   VALUE half of that pair. An aggregate on the band axis is a different
///   chart from the one the kind names.
/// - **The band channel** must name a plain column. A `$param`, a map or a
///   number is not a category to group by, and inventing one would group by an
///   expression the author never wrote.
/// - **No interval channel** ([`INTERVAL_CHANNEL_FIELDS`]). A bar carrying
///   `x1`/`x2` already has both ends written; aggregating over it would throw
///   one of them away.
/// - **No grouping channel beyond the band itself**
///   ([`mark_is_grouped_beyond`]). Mosaic STACKS a bar split by a second
///   column and brightfield does not, so lifting one would collapse the
///   stacks into a single bar — the right TOTAL with the composition silently
///   gone, which is the same failure the binned rect refuses.
/// - **A column-taking aggregate must name its column.** A bare `{sum:}` has
///   nothing to sum; degrading it to a count would answer a question nobody
///   asked.
/// - **`count` must take no column.** `{count: col}` is `COUNT(col)` in
///   Mosaic and `COUNT(*)` is what this path emits — a different number
///   wherever `col` is nullable, so the pair is refused rather than
///   approximated.
fn banded_aggregate(kind: MarkKind, parent: &serde_yaml::Mapping) -> Option<BandedAggregate> {
    let (value_channel, band_channel) = kind.band_aggregate_axes()?;
    if INTERVAL_CHANNEL_FIELDS
        .iter()
        .any(|f| parent.contains_key(serde_yaml::Value::String((*f).to_string())))
    {
        return None;
    }
    let band_column = plain_column_at(parent, band_channel)?;
    if mark_is_grouped_beyond(parent, &band_column) {
        return None;
    }
    let (func, column) = aggregate_transform(channel_map_at(parent, value_channel)?)?;
    Some(BandedAggregate {
        value_channel,
        func,
        column,
    })
}

/// Resolve a mark's `sort:` into the order a lowerer can emit, or `None` when
/// the shape is one nothing computes.
///
/// `sort: { y: -x, limit: 10 }` on a `barX` orders the `y` band by the `x`
/// value, descending, and keeps ten bars.
///
/// Every condition below is a refusal that keeps the mark's
/// [`ParseWarning::UnconsumedSort`] rather than ordering by something the
/// author did not ask for:
///
/// - **The mark kind** must have a band axis
///   ([`MarkKind::band_aggregate_axes`]) — the one list `BarLowerer` is
///   registered against, so a lifted sort always has a reader.
/// - **The sort key** must be that band axis. `sort: {x: -y}` on a `barX` asks
///   for the value axis to be re-ordered, which is a continuous scale with no
///   order to set.
/// - **The value** must name the mark's value axis, optionally behind a single
///   `-`. Ordering a band by a channel that is not on the mark would need a
///   column the query does not select.
/// - **`limit:`** must be a positive integer. `0` and a negative both mean a
///   chart with nothing in it, which is worth a diagnostic rather than a blank
///   frame.
/// - **No other key.** Mosaic's `sort:` also takes `reverse`, `reduce` and an
///   explicit `order`; none of them reaches a lowerer, and honouring the half
///   of a map that does would order the bars while silently dropping the rest
///   of the instruction.
fn mark_sort(kind: MarkKind, parent: &serde_yaml::Mapping) -> Option<MarkSort> {
    let (value_channel, band_channel) = kind.band_aggregate_axes()?;
    let m = channel_map_at(parent, "sort")?;
    let mut descending = None;
    let mut limit = None;
    for (k, v) in m {
        let key = k.as_str()?;
        if key == band_channel {
            let written = v.as_str()?;
            let (desc, named) = written
                .strip_prefix('-')
                .map_or((false, written), |rest| (true, rest));
            if named != value_channel {
                return None;
            }
            descending = Some(desc);
        } else if SORT_MODIFIER_KEYS.contains(&key) {
            let n = v.as_u64()?;
            if n == 0 {
                return None;
            }
            limit = Some(n);
        } else {
            return None;
        }
    }
    Some(MarkSort {
        channel: band_channel,
        by: value_channel,
        descending: descending?,
        limit,
    })
}

/// The column name written plainly at `channel` — a bare string that is not a
/// `$param` reference. `None` for every other shape.
fn plain_column_at(parent: &serde_yaml::Mapping, channel: &str) -> Option<String> {
    let s = parent
        .get(serde_yaml::Value::String(channel.to_string()))?
        .as_str()?;
    (dollar_ident(s).is_none()).then(|| s.to_string())
}

/// The `(func, column)` of a single-key aggregate map — `{count:}`,
/// `{sum: col}`, `{avg: col}`. `None` for any other shape, including a
/// recognised aggregate name carrying the wrong argument (see
/// [`banded_aggregate`] for why each of those is a refusal rather than a
/// best effort).
fn aggregate_transform(m: &serde_yaml::Mapping) -> Option<(AggregateFunc, Option<String>)> {
    let mut entries = m.iter();
    let (k, v) = entries.next()?;
    if entries.next().is_some() {
        return None;
    }
    let func = AggregateFunc::from_wire(k.as_str()?)?;
    match (func, v) {
        (AggregateFunc::Count, serde_yaml::Value::Null) => Some((func, None)),
        (AggregateFunc::Count, _) => None,
        (_, serde_yaml::Value::String(col)) if dollar_ident(col).is_none() => {
            Some((func, Some(col.clone())))
        }
        _ => None,
    }
}

/// Whether a mark carries a grouping channel that would split each band into
/// MORE than one bar.
///
/// The band column itself does not count. `observable-latency.yaml` writes
/// `fill: route` beside `y: route`: that colours one bar per route, it does
/// not stack two routes into one. A colour constant does not count either,
/// for the same reason [`mark_is_grouped`] excludes it. Anything else does.
fn mark_is_grouped_beyond(parent: &serde_yaml::Mapping, band_column: &str) -> bool {
    GROUPING_CHANNEL_FIELDS.iter().any(|field| {
        match parent.get(serde_yaml::Value::String((*field).to_string())) {
            None => false,
            Some(serde_yaml::Value::String(s)) => {
                s != band_column && (*field == "z" || !is_colour_literal(s))
            }
            // A non-string binding on a grouping channel (a map, a `$param`, a
            // number) is not the band column and not a colour constant, so it
            // may carry groups.
            Some(_) => true,
        }
    })
}

/// The mapping written at `channel`, or `None` when the channel is absent or
/// is not a map (a plain column, a literal, a `$param`).
fn channel_map_at<'a>(
    parent: &'a serde_yaml::Mapping,
    channel: &str,
) -> Option<&'a serde_yaml::Mapping> {
    match parent.get(serde_yaml::Value::String(channel.to_string()))? {
        serde_yaml::Value::Mapping(m) => Some(m),
        _ => None,
    }
}

/// The `(column, steps)` of a `{bin: col}` / `{bin: col, steps: n}` map.
/// `None` for any other shape — including a `bin` whose value is not a column
/// name, and a map carrying a modifier this lowerer does not honour.
fn bin_transform(m: &serde_yaml::Mapping) -> Option<(String, Option<i64>)> {
    let mut column: Option<String> = None;
    let mut steps: Option<i64> = None;
    for (k, v) in m {
        match k.as_str()? {
            "bin" => column = Some(v.as_str()?.to_string()),
            "steps" => steps = Some(v.as_i64()?),
            other if BIN_MODIFIER_KEYS.contains(&other) => {}
            _ => return None,
        }
    }
    Some((column?, steps))
}

/// Whether a channel map is exactly `{count:}` — the corpus writes it both
/// bare and as `{count: null}`, and both mean "count the rows in the group".
fn is_count_transform(m: &serde_yaml::Mapping) -> bool {
    let mut entries = m.iter();
    let Some((k, v)) = entries.next() else {
        return false;
    };
    entries.next().is_none() && k.as_str() == Some("count") && v.is_null()
}

/// Column names per inline data source, keyed by the `data:` entry's name.
///
/// An inline `data: { obs: [ {v: 1}, … ] }` is the only schema a spec carries
/// in itself; a `file:` or `query:` source's columns are DuckDB's and the
/// parser never asks.
type InlineColumns = std::collections::HashMap<String, std::collections::HashSet<String>>;

/// Harvest [`InlineColumns`] from the raw root mapping.
///
/// Every key of every row object counts, not just the first row's: a ragged
/// source still HAS the column its later rows carry, and the question asked of
/// this map is "could this name be a column", where a false negative is the
/// expensive answer.
fn inline_source_columns(root: &serde_yaml::Mapping) -> InlineColumns {
    let mut out = InlineColumns::new();
    let Some(serde_yaml::Value::Mapping(data)) =
        root.get(serde_yaml::Value::String("data".to_string()))
    else {
        return out;
    };
    for (name, def) in data {
        let (Some(name), serde_yaml::Value::Sequence(rows)) = (name.as_str(), def) else {
            continue;
        };
        let columns: std::collections::HashSet<String> = rows
            .iter()
            .filter_map(|row| match row {
                serde_yaml::Value::Mapping(m) => Some(m),
                _ => None,
            })
            .flat_map(|m| m.keys().filter_map(|k| k.as_str().map(str::to_string)))
            .collect();
        if !columns.is_empty() {
            out.insert(name.to_string(), columns);
        }
    }
    out
}

/// A `fill`/`stroke` string that reads as a colour constant but is also a
/// COLUMN of the mark's own source.
struct ShadowedColour {
    /// The channel it sat on (`fill` or `stroke`).
    field: &'static str,
    /// The name that is both, as written.
    name: String,
    /// The data source that has a column by that name.
    source: String,
}

/// The colour-versus-column collision, reported where the schema is knowable.
///
/// **This changes NOTHING about what is drawn — it reports, and that is all.**
/// The temptation is to refuse the histogram lift here, on the theory that a
/// real column means real groups. Refusing would blank a frame Mosaic renders:
/// unlifted, the two channels stay plain objects, which is exactly the
/// [`ParseWarning::UnconsumedChannelTransform`] case — no column reaches the
/// renderer and the mark draws no ink. Under the colour-constant rule
/// ([`is_colour_literal`]) `fill: gold` means the constant, so one bar per bin
/// carrying whole-row counts is the correct reading and there is no ambiguity
/// to protect the author from. Only their intent is in doubt, and a warning is
/// the right instrument for that.
///
/// Raised only where the shadow is PROVABLE: the mark reads an inline source
/// and that source has a column by that name.
fn shadowed_colour(parent: &serde_yaml::Mapping, inline: &InlineColumns) -> Option<ShadowedColour> {
    let columns = mark_source_columns(parent, inline)?;
    GROUPING_CHANNEL_FIELDS.iter().find_map(|field| {
        if *field == "z" {
            return None;
        }
        let name = parent
            .get(serde_yaml::Value::String((*field).to_string()))?
            .as_str()?;
        (is_colour_literal(name) && columns.contains(name)).then(|| ShadowedColour {
            field,
            name: name.to_string(),
            source: mark_source_name(parent).unwrap_or_default().to_string(),
        })
    })
}

/// The inline columns of the source a mark reads, when it reads one.
fn mark_source_columns<'a>(
    parent: &serde_yaml::Mapping,
    inline: &'a InlineColumns,
) -> Option<&'a std::collections::HashSet<String>> {
    inline.get(mark_source_name(parent)?)
}

/// The `data: { from: … }` source name a mark reads.
fn mark_source_name(parent: &serde_yaml::Mapping) -> Option<&str> {
    let serde_yaml::Value::Mapping(data) =
        parent.get(serde_yaml::Value::String("data".to_string()))?
    else {
        return None;
    };
    data.get(serde_yaml::Value::String("from".to_string()))?
        .as_str()
}

/// Whether a mark binds a channel that splits each bin into groups.
/// Conservative in both directions that matter: an explicit `z` is a grouping
/// whatever its value, and a `fill`/`stroke` that is not a recognised colour
/// constant is read as a field name (see [`is_colour_literal`]).
///
/// A name that is BOTH classifies as a colour constant here, and is reported
/// one level up by `shadowed_colour` rather than silently taken.
fn mark_is_grouped(parent: &serde_yaml::Mapping) -> bool {
    GROUPING_CHANNEL_FIELDS.iter().any(|field| {
        match parent.get(serde_yaml::Value::String((*field).to_string())) {
            None => false,
            Some(_) if *field == "z" => true,
            Some(serde_yaml::Value::String(s)) => !is_colour_literal(s),
            // A non-string binding on a colour channel (a map, a `$param`, a
            // number) is not a colour constant, so it may carry groups.
            Some(_) => true,
        }
    })
}

/// If `v` is a lift-shaped form, return the lifted ParamRef.
/// Accepts: bare `"$name"`; `{param: name}`; `{selection: name}`.
fn maybe_lift(v: &serde_yaml::Value) -> Option<ParamRef> {
    match v {
        serde_yaml::Value::String(s) => dollar_ident(s).map(|n| ParamRef::new(n.to_string())),
        serde_yaml::Value::Mapping(m) => {
            if m.len() != 1 {
                return None;
            }
            for (k, val) in m {
                let key = k.as_str().unwrap_or("");
                if (key == "param" || key == "selection") && val.as_str().is_some() {
                    return val.as_str().map(|n| ParamRef::new(n.to_string()));
                }
            }
            None
        }
        _ => None,
    }
}

/// If `s` is exactly `$ident` (nothing else), return the identifier.
fn dollar_ident(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    if b.len() < 2 || b[0] != b'$' {
        return None;
    }
    if !(b[1].is_ascii_alphabetic() || b[1] == b'_') {
        return None;
    }
    for c in &b[2..] {
        if !(c.is_ascii_alphanumeric() || *c == b'_') {
            return None;
        }
    }
    Some(&s[1..])
}

/// Decide whether a string value is a bare `$ident` (→ `SpecValue::Param`),
/// contains `$ident` outside literal contexts (→ `SpecValue::Expression`),
/// or is a plain literal string.
fn string_to_spec_value(s: &str) -> SpecValue {
    if let Some(n) = dollar_ident(s) {
        return SpecValue::Param(ParamRef::new(n.to_string()));
    }
    let tok = expr::tokenise(s);
    if tok.is_literal() {
        SpecValue::String(s.to_string())
    } else {
        SpecValue::Expression(tok)
    }
}

/// True iff `declared` shares major+minor with [`crate::SUPPORTED_MOSAIC_MAJOR_MINOR`].
/// Pre-release tags (`0.24.0-alpha.1`) and trailing components are ignored for
/// comparison purposes.
fn version_matches(declared: &str) -> bool {
    let (maj_sup, min_sup) = crate::SUPPORTED_MOSAIC_MAJOR_MINOR;
    let stem = declared.split(['-', '+']).next().unwrap_or("");
    let mut parts = stem.split('.');
    let maj = parts.next().and_then(|x| x.parse::<u16>().ok());
    let min = parts.next().and_then(|x| x.parse::<u16>().ok());
    matches!((maj, min), (Some(a), Some(b)) if a == maj_sup && b == min_sup)
}

fn json_to_yaml_value(j: &serde_json::Value) -> serde_yaml::Value {
    match j {
        serde_json::Value::Null => serde_yaml::Value::Null,
        serde_json::Value::Bool(b) => serde_yaml::Value::Bool(*b),
        serde_json::Value::Number(n) => serde_yaml::Value::Number(if let Some(i) = n.as_i64() {
            serde_yaml::Number::from(i)
        } else if let Some(u) = n.as_u64() {
            serde_yaml::Number::from(u)
        } else if let Some(f) = n.as_f64() {
            serde_yaml::Number::from(f)
        } else {
            serde_yaml::Number::from(0)
        }),
        serde_json::Value::String(s) => serde_yaml::Value::String(s.clone()),
        serde_json::Value::Array(a) => {
            serde_yaml::Value::Sequence(a.iter().map(json_to_yaml_value).collect())
        }
        serde_json::Value::Object(o) => {
            let mut m = serde_yaml::Mapping::new();
            for (k, v) in o {
                m.insert(serde_yaml::Value::String(k.clone()), json_to_yaml_value(v));
            }
            serde_yaml::Value::Mapping(m)
        }
    }
}

// ---------------------------------------------------------------------------
// Serialize: ParamRef canonicalises to "$name" string form.
// ---------------------------------------------------------------------------

/// The Meridian sequential ramp as CSS hex stops (blue-240, steps 100..=700).
/// `colorScheme: meridian` is Brightfield-local sugar a vanilla Mosaic
/// renderer would reject, so [`serialise_spec`] expands it to an explicit
/// `colorRange` of these stops on export (deviations.yaml DEV-0004). Pinned
/// byte-equal to the design crate's `viz::SEQUENTIAL_MERIDIAN` by an
/// agreement test in brightfield-render (which depends on both crates); this
/// crate stays dependency-light and carries the hex forms directly.
pub const MERIDIAN_COLOR_RANGE_HEX: [&str; 13] = [
    "#c6e4fb", "#a6d7fa", "#87c8f6", "#69baf0", "#4daae6", "#359bd9", "#238cc7", "#1d7cb2",
    "#216d9b", "#285e81", "#2d4f67", "#274154", "#1b3546",
];

/// Canonically re-serialise a [`Spec`] to a YAML string (commit).
///
/// The command-log commit re-serialises the working (edited) Spec through this
/// single canonical path (the `impl Serialize for Spec` below) into the editor
/// buffer, then lets the unchanged `set_value` -> save -> watcher pipeline carry
/// it. The write is LOSSY by design — it reformats, drops comments, and emits a
/// fixed block order — so it is confined to the deliberate commit and
/// round-trip-tested on the target within-plot-edited specs (`parse -> apply ->
/// serialise -> re-parse` yields the same AST).
pub fn serialise_spec(spec: &Spec) -> Result<String, String> {
    serde_yaml::to_string(spec).map_err(|e| e.to_string())
}

impl Serialize for Spec {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Rough field count for hint only.
        let mut count = 0;
        if self.meta.is_some() {
            count += 1;
        }
        if !self.data.is_empty() {
            count += 1;
        }
        if !self.params.is_empty() {
            count += 1;
        }
        if !self.config.is_empty() {
            count += 1;
        }
        if !self.plot_defaults.is_empty() {
            count += 1;
        }
        if self.root.is_some() {
            // Root contributes one key (plot, vconcat, hconcat, …).
            count += 1;
        }
        let mut map = s.serialize_map(Some(count))?;
        if let Some(meta) = &self.meta {
            map.serialize_entry("meta", meta)?;
        }
        if !self.data.is_empty() {
            map.serialize_entry("data", &SerData(&self.data))?;
        }
        if !self.params.is_empty() {
            map.serialize_entry("params", &SerParams(&self.params))?;
        }
        if !self.config.is_empty() {
            map.serialize_entry("config", &SerSpecValueMap(&self.config.0))?;
        }
        if !self.plot_defaults.is_empty() {
            map.serialize_entry("plotDefaults", &SerSpecValueMap(&self.plot_defaults.0))?;
        }
        if let Some(root) = &self.root {
            // Root is emitted as a nested component under its discriminator
            // keys by ComponentSer — it writes its own entries via a helper
            // that flattens into the parent map.
            emit_component_into(&mut map, root)?;
        }
        map.end()
    }
}

impl Serialize for Meta {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        if let Some(t) = &self.title {
            map.serialize_entry("title", t)?;
        }
        if let Some(d) = &self.description {
            map.serialize_entry("description", d)?;
        }
        if let Some(v) = &self.version {
            map.serialize_entry("version", v)?;
        }
        map.end()
    }
}

struct SerData<'a>(&'a IndexMap<String, DataSource>);

impl Serialize for SerData<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in self.0 {
            map.serialize_entry(k, &SerDataSource(v))?;
        }
        map.end()
    }
}

struct SerDataSource<'a>(&'a DataSource);

impl Serialize for SerDataSource<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.0.kind {
            DataSourceKind::Shorthand(x) => s.serialize_str(x),
            DataSourceKind::InlineRows(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for v in items {
                    seq.serialize_element(&SerSpecValue(v))?;
                }
                seq.end()
            }
            _ => {
                let mut map = s.serialize_map(None)?;
                match &self.0.kind {
                    DataSourceKind::File(p) => map.serialize_entry("file", p)?,
                    DataSourceKind::Query(q) => map.serialize_entry("query", q)?,
                    DataSourceKind::Typed(_) | DataSourceKind::Opaque => {}
                    _ => {}
                }
                for (k, v) in &self.0.extras {
                    map.serialize_entry(k, &SerSpecValue(v))?;
                }
                map.end()
            }
        }
    }
}

struct SerParams<'a>(&'a IndexMap<String, ParamNode>);

impl Serialize for SerParams<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in self.0 {
            map.serialize_entry(k, &SerParamNode(v))?;
        }
        map.end()
    }
}

struct SerParamNode<'a>(&'a ParamNode);

impl Serialize for SerParamNode<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            ParamNode::Value(v) => SerSpecValue(v).serialize(s),
            ParamNode::Selection(sel) => {
                let mut map = s.serialize_map(None)?;
                map.serialize_entry("select", sel.select.wire_name())?;
                for (k, v) in &sel.options {
                    map.serialize_entry(k, &SerSpecValue(v))?;
                }
                map.end()
            }
        }
    }
}

struct SerSpecValueMap<'a>(&'a IndexMap<String, SpecValue>);

impl Serialize for SerSpecValueMap<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in self.0 {
            map.serialize_entry(k, &SerSpecValue(v))?;
        }
        map.end()
    }
}

struct SerSpecValue<'a>(&'a SpecValue);

impl Serialize for SerSpecValue<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            SpecValue::Null => s.serialize_unit(),
            SpecValue::Bool(b) => s.serialize_bool(*b),
            SpecValue::Integer(i) => s.serialize_i64(*i),
            SpecValue::Float(f) => s.serialize_f64(*f),
            SpecValue::String(x) => s.serialize_str(x),
            SpecValue::Array(a) => {
                let mut seq = s.serialize_seq(Some(a.len()))?;
                for v in a {
                    seq.serialize_element(&SerSpecValue(v))?;
                }
                seq.end()
            }
            SpecValue::Object(m) => {
                let mut map = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    map.serialize_entry(k, &SerSpecValue(v))?;
                }
                map.end()
            }
            SpecValue::Param(r) => s.serialize_str(&r.to_wire()),
            SpecValue::Expression(e) => s.serialize_str(&e.to_wire()),
            // Re-serialise the aggregate transform to its single-key map form:
            // `{count: null}` / `{avg: "col"}`, so parse → serialise → parse is
            // idempotent.
            SpecValue::Aggregate { func, column } => {
                let mut map = s.serialize_map(Some(1))?;
                match column {
                    Some(col) => map.serialize_entry(func.wire_name(), col)?,
                    None => map.serialize_entry(func.wire_name(), &())?,
                }
                map.end()
            }
            // Likewise for the positional bin: back to `{bin: col}` — plus
            // `{steps: n}` only when the spec wrote one, so a spec that took
            // the default does not gain a key it never had.
            SpecValue::Bin { column, steps } => {
                let mut map = s.serialize_map(Some(1 + usize::from(steps.is_some())))?;
                map.serialize_entry("bin", column)?;
                if let Some(n) = steps {
                    map.serialize_entry("steps", n)?;
                }
                map.end()
            }
            // And likewise back to `{y: -x, limit: n}`. The `-` is written
            // only when the lift read one, and `limit:` only when the spec
            // carried one, so a re-serialised spec gains no key it never had.
            SpecValue::Sort {
                channel,
                by,
                descending,
                limit,
            } => {
                let mut map = s.serialize_map(Some(1 + usize::from(limit.is_some())))?;
                let sign = if *descending { "-" } else { "" };
                map.serialize_entry(channel, &format!("{sign}{by}"))?;
                if let Some(n) = limit {
                    map.serialize_entry("limit", n)?;
                }
                map.end()
            }
        }
    }
}

struct SerValueOrParamRef<'a>(&'a ValueOrParamRef<SpecValue>);

impl Serialize for SerValueOrParamRef<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            ValueOrParamRef::Value(v) => SerSpecValue(v).serialize(s),
            ValueOrParamRef::Param(r) => s.serialize_str(&r.to_wire()),
        }
    }
}

fn emit_component_into<S>(map: &mut S, c: &Component) -> Result<(), S::Error>
where
    S: SerializeMap,
{
    match c {
        Component::Plot(p) => {
            let items: Vec<SerComponent> = p.items.iter().map(SerComponent).collect();
            map.serialize_entry("plot", &items)?;
            for (k, v) in &p.attributes {
                // `colorScheme: meridian` is Brightfield-local sugar
                // (DEV-0004): export expands it to explicit `colorRange`
                // stops so the emitted spec stays vanilla-Mosaic-portable.
                // With an explicit colorRange ALSO present the sugar is
                // dropped instead (never emit a duplicate key; the explicit
                // range already wins consumption-side).
                if k == "colorScheme" && matches!(v, SpecValue::String(s) if s == "meridian") {
                    if !p.attributes.contains_key("colorRange") {
                        map.serialize_entry("colorRange", &MERIDIAN_COLOR_RANGE_HEX)?;
                    }
                    continue;
                }
                map.serialize_entry(k, &SerSpecValue(v))?;
            }
        }
        Component::HConcat(c) => {
            let items: Vec<SerComponent> = c.items.iter().map(SerComponent).collect();
            map.serialize_entry("hconcat", &items)?;
        }
        Component::VConcat(c) => {
            let items: Vec<SerComponent> = c.items.iter().map(SerComponent).collect();
            map.serialize_entry("vconcat", &items)?;
        }
        Component::HSpace(sp) => {
            map.serialize_entry("hspace", &SerSpecValue(&sp.value))?;
        }
        Component::VSpace(sp) => {
            map.serialize_entry("vspace", &SerSpecValue(&sp.value))?;
        }
        Component::Legend(l) => {
            map.serialize_entry("legend", l.channel.wire_name())?;
            for (k, v) in &l.options {
                map.serialize_entry(k, &SerValueOrParamRef(v))?;
            }
        }
        Component::Mark(m) => {
            map.serialize_entry("mark", m.kind.wire_name())?;
            if let Some(data) = &m.data {
                map.serialize_entry("data", &SerMarkData(data))?;
            }
            for (k, v) in &m.options {
                map.serialize_entry(k, &SerValueOrParamRef(v))?;
            }
        }
        Component::Interactor(i) => {
            map.serialize_entry("select", i.kind.wire_name())?;
            for (k, v) in &i.options {
                map.serialize_entry(k, &SerValueOrParamRef(v))?;
            }
        }
        Component::Input(inp) => {
            map.serialize_entry("input", inp.kind.wire_name())?;
            if let Some(ref pr) = inp.as_param {
                map.serialize_entry("as", &pr.to_wire())?;
            }
            if let Some(ref src) = inp.from_source {
                map.serialize_entry("from", src)?;
            }
            if let Some(ref pr) = inp.filter_by {
                map.serialize_entry("filterBy", &pr.to_wire())?;
            }
            for (k, v) in &inp.options {
                map.serialize_entry(k, &SerValueOrParamRef(v))?;
            }
        }
    }
    Ok(())
}

struct SerComponent<'a>(&'a Component);

impl Serialize for SerComponent<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(None)?;
        emit_component_into(&mut map, self.0)?;
        map.end()
    }
}

struct SerMarkData<'a>(&'a MarkData);

impl Serialize for SerMarkData<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            MarkData::From {
                source,
                filter_by,
                extras,
            } => {
                let mut map = s.serialize_map(None)?;
                map.serialize_entry("from", source)?;
                if let Some(r) = filter_by {
                    map.serialize_entry("filterBy", &r.to_wire())?;
                }
                for (k, v) in extras {
                    map.serialize_entry(k, &SerSpecValue(v))?;
                }
                map.end()
            }
            MarkData::Inline(rows) => {
                let mut seq = s.serialize_seq(Some(rows.len()))?;
                for r in rows {
                    seq.serialize_element(&SerSpecValue(r))?;
                }
                seq.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_spec_yaml_entry() {
        let src = "meta:\n  title: hello\n";
        let out = parse_spec(src, Format::Yaml).expect("parses");
        assert_eq!(
            out.spec.meta.as_ref().unwrap().title.as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn parse_spec_json_entry() {
        let src = r#"{"meta":{"title":"hi"}}"#;
        let out = parse_spec(src, Format::Json).expect("parses");
        assert_eq!(out.spec.meta.as_ref().unwrap().title.as_deref(), Some("hi"));
    }

    #[test]
    fn parse_spec_path_unknown_ext() {
        let p = std::path::PathBuf::from("/tmp/nope.toml");
        let err = parse_spec_path(&p).unwrap_err();
        assert!(matches!(err, ParseError::UnknownFormat { .. }));
    }

    #[test]
    fn dollar_ident_detection() {
        assert_eq!(dollar_ident("$brush"), Some("brush"));
        assert_eq!(dollar_ident("$snake_case"), Some("snake_case"));
        assert_eq!(dollar_ident("$a1"), Some("a1"));
        assert_eq!(dollar_ident("$1bad"), None);
        assert_eq!(dollar_ident("$"), None);
        assert_eq!(dollar_ident("plain"), None);
        assert_eq!(dollar_ident("$a b"), None);
    }

    #[test]
    fn maybe_lift_string_and_object_shorthands() {
        let s = serde_yaml::Value::String("$foo".into());
        assert_eq!(maybe_lift(&s), Some(ParamRef::new("foo")));

        let src = "{ param: bar }";
        let v: serde_yaml::Value = serde_yaml::from_str(src).unwrap();
        assert_eq!(maybe_lift(&v), Some(ParamRef::new("bar")));

        let src = "{ selection: sel }";
        let v: serde_yaml::Value = serde_yaml::from_str(src).unwrap();
        assert_eq!(maybe_lift(&v), Some(ParamRef::new("sel")));

        let src = "{ other: x }";
        let v: serde_yaml::Value = serde_yaml::from_str(src).unwrap();
        assert_eq!(maybe_lift(&v), None);
    }

    #[test]
    fn unknown_mark_errors() {
        let src = r#"
plot:
  - mark: fooBar
"#;
        let err = parse_spec(src, Format::Yaml).unwrap_err();
        match err {
            ParseError::UnknownName { name, surface, .. } => {
                assert_eq!(name, "fooBar");
                assert_eq!(surface, NameSurface::Mark);
            }
            other => panic!("expected UnknownName, got {other:?}"),
        }
    }

    #[test]
    fn unimplemented_mark_warns_with_stub() {
        // `voronoi` is a genuinely-unimplemented mark (no renderer/lowerer), so
        // it still warns. (cell was this test's stub until the density
        // marks promoted it — the swap keeps exactly one always-unimplemented
        // stand-in exercising the warning path.)
        let src = r#"
plot:
  - mark: voronoi
    x: a
    y: b
"#;
        let out = parse_spec(src, Format::Yaml).expect("parses");
        assert!(out
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::Unimplemented { name, .. } if name == "voronoi")));
    }

    #[test]
    fn version_match_rules() {
        assert!(version_matches("0.24.0"));
        assert!(version_matches("0.24.1"));
        assert!(version_matches("0.24"));
        assert!(version_matches("0.24.0-alpha.1"));
        assert!(!version_matches("0.23.9"));
        assert!(!version_matches("1.0.0"));
    }

    #[test]
    fn version_mismatch_warns() {
        let src = "meta:\n  version: 0.23.0\n";
        let out = parse_spec(src, Format::Yaml).expect("parses");
        assert!(out
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::VersionMismatch { .. })));
    }

    #[test]
    fn version_match_no_warning() {
        let src = "meta:\n  version: 0.24.2\n";
        let out = parse_spec(src, Format::Yaml).expect("parses");
        assert!(!out
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::VersionMismatch { .. })));
    }

    #[test]
    fn version_absent_no_warning() {
        let src = "meta:\n  title: no version\n";
        let out = parse_spec(src, Format::Yaml).expect("parses");
        assert!(!out
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::VersionMismatch { .. })));
    }

    #[test]
    fn version_invalid_warns() {
        let src = "meta:\n  version: bogus\n";
        let out = parse_spec(src, Format::Yaml).expect("parses");
        assert!(out
            .warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::VersionMismatch { .. })));
    }

    #[test]
    fn strict_context_reject_dollar() {
        let src = "meta:\n  title: $dynamic\n";
        let err = parse_spec(src, Format::Yaml).unwrap_err();
        assert!(matches!(err, ParseError::StrictContextUnresolvedRef { .. }));
    }

    #[test]
    fn paramref_serialises_to_dollar_form() {
        // Round-trip a SpecValue::Param through YAML and confirm "$foo" shape.
        let v = SpecValue::Param(ParamRef::new("foo"));
        let s = serde_yaml::to_string(&SerSpecValue(&v)).unwrap();
        assert!(s.contains("$foo"));
    }

    #[test]
    fn ast_round_trip_idempotent() {
        let src = r#"
meta:
  title: rt
params:
  brush:
    select: crossfilter
plot:
  - mark: dot
    x: a
    y: b
    filterBy: $brush
"#;
        let a = parse_spec(src, Format::Yaml).expect("first parse");
        let serialised = serde_yaml::to_string(&a.spec).expect("serialise");
        let b = parse_spec(&serialised, Format::Yaml).expect("second parse");
        assert_eq!(a.spec, b.spec);
    }

    /// verification (spec-mandated): for every entry in
    /// [`LIFT_SURFACE_FIELDS`], placing that field on a Mark's option bag
    /// with a `"$foo"` string form produces `ValueOrParamRef::Param`.
    /// Parametrised — omissions surface as failures, not silent
    /// under-coverage. Mark is chosen as the universal parent because
    /// `Walker::lift_field` is uniformly called for all option-bag walks.
    #[test]
    fn lift_surface_parametrised_string_form() {
        for field in LIFT_SURFACE_FIELDS {
            let src = format!("mark: dot\n{field}: $foo\n");
            let out = parse_spec(&src, Format::Yaml)
                .unwrap_or_else(|e| panic!("parse failed for field `{field}`: {e}"));
            let root = out
                .spec
                .root
                .unwrap_or_else(|| panic!("no root for field `{field}`"));
            let m = match root {
                Component::Mark(m) => m,
                other => panic!("root was not a Mark for field `{field}`: {other:?}"),
            };
            let entry = m
                .options
                .get(*field)
                .unwrap_or_else(|| panic!("mark options has no `{field}` entry"));
            match entry {
                ValueOrParamRef::Param(r) => {
                    assert_eq!(
                        r.to_wire(),
                        "$foo",
                        "field `{field}` lifted, but wrong name"
                    );
                }
                ValueOrParamRef::Value(v) => {
                    panic!("field `{field}` did not lift from string form: kept as Value {v:?}")
                }
            }
        }
    }

    /// verification: same contract as the string-form parametrisation
    /// but with the object shorthand `{param: foo}` at every lift position.
    #[test]
    fn lift_surface_parametrised_object_form() {
        for field in LIFT_SURFACE_FIELDS {
            let src = format!("mark: dot\n{field}: {{ param: foo }}\n");
            let out = parse_spec(&src, Format::Yaml)
                .unwrap_or_else(|e| panic!("parse failed for field `{field}`: {e}"));
            let root = out
                .spec
                .root
                .unwrap_or_else(|| panic!("no root for field `{field}`"));
            let m = match root {
                Component::Mark(m) => m,
                other => panic!("root was not a Mark for field `{field}`: {other:?}"),
            };
            let entry = m
                .options
                .get(*field)
                .unwrap_or_else(|| panic!("mark options has no `{field}` entry"));
            match entry {
                ValueOrParamRef::Param(r) => {
                    assert_eq!(
                        r.to_wire(),
                        "$foo",
                        "field `{field}` lifted, but wrong name"
                    );
                }
                ValueOrParamRef::Value(v) => {
                    panic!("field `{field}` did not lift from object form: kept as Value {v:?}")
                }
            }
        }
    }

    /// unknown keys under `meta` parse successfully and emit
    /// exactly one `ParseWarning::UnknownOption { path: "meta", key }`.
    /// Post-D2 contract (narrowed from fatal SchemaViolation).
    #[test]
    fn meta_unknown_key_warns() {
        let src = "meta:\n  credit: Observable\n  title: x\n";
        let out = parse_spec(src, Format::Yaml).expect("parses despite unknown meta key");
        let matches: Vec<_> = out
            .warnings
            .iter()
            .filter(|w| {
                matches!(
                    w,
                    ParseWarning::UnknownOption { path, key }
                        if path == "meta" && key == "credit"
                )
            })
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "expected one UnknownOption warning for meta.credit; got {:?}",
            out.warnings
        );
        // Typed accessor still filled.
        assert_eq!(out.spec.meta.as_ref().unwrap().title.as_deref(), Some("x"));
    }

    #[test]
    fn nonnumeric_plot_inset_warns_but_param_defers() {
        // A non-numeric plot inset degrades to absent AND names itself (not a
        // silent drop) — the "malformed" case.
        let bad = "data:\n  t:\n    - { x: 1, y: 2 }\nplot:\n  - { mark: dot, data: { from: t }, x: x, y: y }\ninset: nope\n";
        let out = parse_spec(bad, Format::Yaml).expect("parses despite a bad inset");
        let n = out
            .warnings
            .iter()
            .filter(|w| matches!(w, ParseWarning::NonNumericInset { attribute } if attribute == "inset"))
            .count();
        assert_eq!(
            n, 1,
            "one NonNumericInset naming `inset`; got {:?}",
            out.warnings
        );

        // Numeric inset: silent.
        let good = "data:\n  t:\n    - { x: 1, y: 2 }\nplot:\n  - { mark: dot, data: { from: t }, x: x, y: y }\ninset: 5\n";
        let out2 = parse_spec(good, Format::Yaml).expect("parses");
        assert!(
            !out2
                .warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::NonNumericInset { .. })),
            "a numeric inset must not warn; got {:?}",
            out2.warnings
        );

        // A lifted $param is a recorded deferral, not a typo — no warning.
        let param = "params:\n  pad: 5\ndata:\n  t:\n    - { x: 1, y: 2 }\nplot:\n  - { mark: dot, data: { from: t }, x: x, y: y }\nxInset: $pad\n";
        let out3 = parse_spec(param, Format::Yaml).expect("parses");
        assert!(
            !out3
                .warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::NonNumericInset { .. })),
            "a $param inset defers silently; got {:?}",
            out3.warnings
        );
    }

    #[test]
    fn nonstring_label_warns_but_string_null_param_defer() {
        // A number for an axis label degrades to the derived title AND names
        // itself — mirroring the NonNumericInset parse-time check.
        let bad = "data:\n  t:\n    - { x: 1, y: 2 }\nplot:\n  - { mark: dot, data: { from: t }, x: x, y: y }\nxLabel: 42\n";
        let out = parse_spec(bad, Format::Yaml).expect("parses despite a bad label");
        let n = out
            .warnings
            .iter()
            .filter(|w| matches!(w, ParseWarning::NonStringLabel { attribute } if attribute == "xLabel"))
            .count();
        assert_eq!(
            n, 1,
            "one NonStringLabel naming `xLabel`; got {:?}",
            out.warnings
        );

        // A string override, an explicit null (suppress), and a lifted $param
        // (recorded deferral) all pass silently — for xLabel/yLabel AND title.
        // The `$p` cases exercise the `SpecValue::Param(_)` exclusion arm: a
        // lifted param defers without a warning (they'd spuriously warn if the
        // exclusion were dropped).
        for ok in [
            "yLabel: Travelers",
            "yLabel: null",
            "title: Weather",
            "title: null",
            "xLabel: $p",
            "title: $p",
        ] {
            let src = format!(
                "params:\n  p: 1\ndata:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - {{ mark: dot, data: {{ from: t }}, x: x, y: y }}\n{ok}\n"
            );
            let o = parse_spec(&src, Format::Yaml).expect("parses");
            assert!(
                !o.warnings
                    .iter()
                    .any(|w| matches!(w, ParseWarning::NonStringLabel { .. })),
                "`{ok}` must not warn; got {:?}",
                o.warnings
            );
        }
        // A boolean title degrades and warns, naming `title`.
        let bt = "data:\n  t:\n    - { x: 1, y: 2 }\nplot:\n  - { mark: dot, data: { from: t }, x: x, y: y }\ntitle: true\n";
        let obt = parse_spec(bt, Format::Yaml).expect("parses");
        assert!(
            obt.warnings.iter().any(
                |w| matches!(w, ParseWarning::NonStringLabel { attribute } if attribute == "title")
            ),
            "a boolean title warns naming `title`; got {:?}",
            obt.warnings
        );
    }

    #[test]
    fn unknown_projection_warns_but_supported_defer() {
        // A name outside Mosaic's `ProjectionName` vocabulary degrades to the
        // default equirectangular fit AND names itself — mirroring the
        // NonStringLabel check. `mollweide` is a real d3 EXTENSION projection
        // rather than a typo, which is the case worth naming: it is the shape of
        // request this warning exists to answer.
        let bad = "data:\n  t:\n    - { x: 1, y: 2 }\nplot:\n  - { mark: dot, data: { from: t }, x: x, y: y }\nprojectionType: mollweide\n";
        let out = parse_spec(bad, Format::Yaml).expect("parses despite unsupported projection");
        assert!(
            out.warnings.iter().any(
                |w| matches!(w, ParseWarning::UnknownProjection { value } if value == "mollweide")
            ),
            "one UnknownProjection naming `mollweide`; got {:?}",
            out.warnings
        );

        // Supported projections and a lifted $param pass silently.
        for ok in [
            "projectionType: albers",
            "projectionType: albers-usa",
            "projectionType: equirectangular",
            "projectionType: mercator",
            "projectionType: orthographic",
            "projectionType: equal-earth",
            "projectionType: transverse-mercator",
            "projectionType: $p",
        ] {
            let src = format!(
                "params:\n  p: 1\ndata:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - {{ mark: dot, data: {{ from: t }}, x: x, y: y }}\n{ok}\n"
            );
            let o = parse_spec(&src, Format::Yaml).expect("parses");
            assert!(
                !o.warnings
                    .iter()
                    .any(|w| matches!(w, ParseWarning::UnknownProjection { .. })),
                "`{ok}` must not warn; got {:?}",
                o.warnings
            );
        }
    }

    /// **`projectionType` on a MARK is a key no lowerer and no renderer reads.**
    /// Mosaic has no mark-level projection: a projection is a plot attribute and
    /// it replaces the plot's x and y scales, so a mark cannot ask for a
    /// different one. This
    /// build read one for a while, which made the mark option and the plot
    /// attribute two mechanisms with nothing arbitrating between them.
    ///
    /// It now reports as `UnconsumedMarkOption` like `curve` does — including
    /// when the value IS one of Mosaic's names, which is the case a reader would
    /// otherwise expect to work.
    #[test]
    fn a_mark_level_projection_is_a_key_nothing_reads() {
        let mark = |opts: &str| {
            format!("data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - {{ mark: dot, data: {{ from: t }}, x: x, y: y, {opts} }}\n")
        };
        for value in ["mercator", "equal-earth", "mollweide"] {
            let out = parse_spec(&mark(&format!("projectionType: {value}")), Format::Yaml)
                .expect("parses");
            assert!(
                out.warnings.iter().any(|w| matches!(
                    w,
                    ParseWarning::UnconsumedMarkOption { mark, key }
                        if mark == "dot" && key == "projectionType"
                )),
                "`projectionType: {value}` on a mark must report as unconsumed; got {:?}",
                out.warnings
            );
            // And it is NOT judged as a projection name — the vocabulary check
            // is the plot attribute's alone, so a mark-level name neither
            // resolves nor warns about resolving.
            assert!(
                !out.warnings
                    .iter()
                    .any(|w| matches!(w, ParseWarning::UnknownProjection { .. })),
                "a mark-level value is not judged as a projection name; got {:?}",
                out.warnings
            );
        }
    }

    /// A mark that asks for `aspectRatio: 1` on a plot that names a
    /// `projectionType` is refused the combination. The warning is what tells the
    /// author; `ChannelMap::equal_aspect` is what makes it true, and
    /// `crates/brightfield-render/tests/projected_point_map.rs` holds that half.
    #[test]
    fn aspect_ratio_alongside_a_projection_warns() {
        let plot = |mark_opts: &str, attrs: &str| {
            format!("data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - {{ mark: dot, data: {{ from: t }}, x: x, y: y{mark_opts} }}\n{attrs}")
        };
        let out = parse_spec(
            &plot(", aspectRatio: 1", "projectionType: mercator\n"),
            Format::Yaml,
        )
        .expect("parses");
        assert!(
            out.warnings.iter().any(
                |w| matches!(w, ParseWarning::AspectRatioWithProjection { mark } if mark == "dot")
            ),
            "asking for both warns naming the mark; got {:?}",
            out.warnings
        );

        // Each alone is silent — the warning is about the COMBINATION, and
        // without these two a warning that fired on `aspectRatio` alone would
        // pass the assertion above.
        for (mark_opts, attrs) in [(", aspectRatio: 1", ""), ("", "projectionType: mercator\n")] {
            let out = parse_spec(&plot(mark_opts, attrs), Format::Yaml).expect("parses");
            assert!(
                !out.warnings
                    .iter()
                    .any(|w| matches!(w, ParseWarning::AspectRatioWithProjection { .. })),
                "`{mark_opts}` / `{attrs}` alone must not warn; got {:?}",
                out.warnings
            );
        }

        // An `aspectRatio` on a plot naming a projection this build cannot draw
        // is not the refusal — the plot names no projection, so the mark keeps
        // its equal-aspect frame and hears about the name instead.
        let out = parse_spec(
            &plot(", aspectRatio: 1", "projectionType: mollweide\n"),
            Format::Yaml,
        )
        .expect("parses");
        assert!(
            !out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::AspectRatioWithProjection { .. })),
            "an unrecognised projection does not displace `aspectRatio`; got {:?}",
            out.warnings
        );
        assert!(
            out.warnings.iter().any(
                |w| matches!(w, ParseWarning::UnknownProjection { value } if value == "mollweide")
            ),
            "the unrecognised name is what the author hears about; got {:?}",
            out.warnings
        );
    }

    /// **A mark whose kind cannot project is not drawn, and is named.** A plot
    /// that carries a projection has axes in the projection's planar units; a
    /// `barY` or a `line` on it would draw its raw columns against those axes,
    /// which is a second coordinate system laid over the first.
    ///
    /// `crates/brightfield-render`'s `scene::render_entry` is what makes the
    /// "not drawn" half true; this is the half that tells the author.
    #[test]
    fn a_mark_that_cannot_project_is_named_rather_than_drawn() {
        let plot = |kind: &str, attrs: &str| {
            format!("data:\n  t:\n    - {{ x: 1, y: 2 }}\nplot:\n  - {{ mark: {kind}, data: {{ from: t }}, x: x, y: y }}\n{attrs}")
        };
        // `dot` and `geo` draw through a projection; nothing else does.
        for kind in ["dot", "geo"] {
            let out = parse_spec(&plot(kind, "projectionType: orthographic\n"), Format::Yaml)
                .expect("parses");
            assert!(
                !out.warnings
                    .iter()
                    .any(|w| matches!(w, ParseWarning::MarkCannotProject { .. })),
                "`{kind}` draws through a projection; got {:?}",
                out.warnings
            );
        }
        for kind in ["line", "barY", "rectY", "text"] {
            let out = parse_spec(&plot(kind, "projectionType: orthographic\n"), Format::Yaml)
                .expect("parses");
            assert!(
                out.warnings.iter().any(|w| matches!(
                    w,
                    ParseWarning::MarkCannotProject { mark, projection }
                        if mark == kind && projection == "orthographic"
                )),
                "`{kind}` cannot project and must be named; got {:?}",
                out.warnings
            );
            // The control: the SAME mark on a plot naming no projection is
            // silent, so the warning is about the projection and not the kind.
            let bare = parse_spec(&plot(kind, ""), Format::Yaml).expect("parses");
            assert!(
                !bare
                    .warnings
                    .iter()
                    .any(|w| matches!(w, ParseWarning::MarkCannotProject { .. })),
                "`{kind}` on an unprojected plot is fine; got {:?}",
                bare.warnings
            );
        }
    }

    /// **An interval brush over a curved projection is refused, and named.** The
    /// warning is here; `analysis::build_brushable_bindings` is what stops the
    /// interactor being installed, held by
    /// `an_interval_brush_is_not_installed_over_a_curved_projection`.
    #[test]
    fn an_interval_brush_over_a_curved_projection_warns() {
        let plot = |interactor: &str, projection: &str| {
            format!(
                "params:\n  sel: {{ select: crossfilter }}\ndata:\n  t:\n    - {{ x: 1, y: 2 }}\n\
                 plot:\n  - {{ mark: dot, data: {{ from: t }}, x: x, y: y }}\n  \
                 - {{ select: {interactor}, as: $sel }}\nprojectionType: {projection}\n"
            )
        };
        for interactor in ["intervalX", "intervalY", "intervalXY"] {
            // Separable: the per-axis inverse is exact, so the brush stands.
            for ok in ["equirectangular", "mercator", "identity", "reflect-y"] {
                let out = parse_spec(&plot(interactor, ok), Format::Yaml).expect("parses");
                assert!(
                    !out.warnings.iter().any(|w| matches!(
                        w,
                        ParseWarning::IntervalBrushUnderCurvedProjection { .. }
                    )),
                    "`{interactor}` under `{ok}` inverts per axis; got {:?}",
                    out.warnings
                );
            }
            // Curved: no rectangle of degrees stands behind the swept pixels.
            for curved in ["orthographic", "albers", "equal-earth", "conic-conformal"] {
                let out = parse_spec(&plot(interactor, curved), Format::Yaml).expect("parses");
                assert!(
                    out.warnings.iter().any(|w| matches!(
                        w,
                        ParseWarning::IntervalBrushUnderCurvedProjection { interactor: i, projection: p }
                            if i == interactor && p == curved
                    )),
                    "`{interactor}` under `{curved}` must be refused; got {:?}",
                    out.warnings
                );
            }
        }
    }

    #[test]
    fn dollar_in_label_text_degrades_to_derive_round_trip() {
        // A label whose text contains a bare `$ident` (a currency/unit literal
        // like "Cost in $usd") is lifted to an Expression at parse time, so it
        // can't be used verbatim — it warns NonStringLabel and the axis falls
        // back to its derived field-name title. Same substrate as a lifted
        // $param label (recorded deferral); pinned here as a parse→resolve round
        // trip so the documented degrade can't silently change.
        use crate::ast::Component;
        use crate::layout::{resolve_axis_titles, AxisTitle};
        let src = "data:\n  t:\n    - { x: 1, y: 2 }\nplot:\n  - { mark: dot, data: { from: t }, x: x, y: y }\nxLabel: Cost in $usd\n";
        let out = parse_spec(src, Format::Yaml).expect("parses");
        assert!(
            out.warnings.iter().any(
                |w| matches!(w, ParseWarning::NonStringLabel { attribute } if attribute == "xLabel")
            ),
            "a $-in-text label warns; got {:?}",
            out.warnings
        );
        let plot = match out.spec.root {
            Some(Component::Plot(p)) => p,
            other => panic!("expected a plot root, got {other:?}"),
        };
        assert_eq!(
            resolve_axis_titles(&plot).x,
            AxisTitle::Derive,
            "a $-in-text xLabel degrades to the derived title"
        );
    }

    /// unknown keys on mark option bags are accepted silently
    /// (open bag — no warning, no error).
    #[test]
    fn mark_unknown_option_is_accepted() {
        let src = "mark: dot\nweirdKey: 42\n";
        let out = parse_spec(src, Format::Yaml).expect("parses with unknown mark option");
        // No UnknownOption warnings for mark option bags — they are open.
        assert!(
            !out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::UnknownOption { .. })),
            "mark options are open bags; no UnknownOption warnings expected, got {:?}",
            out.warnings
        );
        // Value is present in the options.
        let m = match out.spec.root.unwrap() {
            Component::Mark(m) => m,
            _ => panic!("not a mark"),
        };
        assert!(m.options.contains_key("weirdKey"));
    }

    /// Statistical-mark options pass parser cleanly
    /// (no SchemaViolation, no UnknownOption warning).
    #[test]
    fn statistical_mark_options_accepted() {
        let cases = [
            ("density", "bandwidth: 0.5"),
            ("density", "normalize: \"max\""),
            ("density", "stack: true"),
            ("densityX", "thresholds: 32"),
            ("regressionY", "ci: 0.95"),
            ("regressionY", "stroke: \"red\""),
        ];
        for (mark, opt) in cases {
            let src = format!("mark: {mark}\n{opt}\n");
            let out = parse_spec(&src, Format::Yaml).expect("parses statistical mark");
            assert!(
                !out.warnings
                    .iter()
                    .any(|w| matches!(w, ParseWarning::UnknownOption { .. })),
                "no UnknownOption warning for {mark}/{opt}; got {:?}",
                out.warnings
            );
        }
    }

    // -----------------------------------------------------------------------
    // Self-aggregating channel transforms (fill/r {count:}/{avg:})
    // -----------------------------------------------------------------------

    use crate::ast::AggregateFunc;

    fn mark_channel(src: &str, channel: &str) -> ValueOrParamRef<SpecValue> {
        let out = parse_spec(src, Format::Yaml).expect("parses");
        let m = match out.spec.root.expect("root") {
            Component::Mark(m) => m,
            other => panic!("expected mark, got {other:?}"),
        };
        m.options.get(channel).expect("channel present").clone()
    }

    /// `fill: {count:}` (flights-hexbin / mark-types) parses to a
    /// typed count aggregate with no source column.
    #[test]
    fn fill_count_parses_to_aggregate() {
        let entry = mark_channel("mark: hexbin\nfill: { count: }\n", "fill");
        assert_eq!(
            entry,
            ValueOrParamRef::Value(SpecValue::Aggregate {
                func: AggregateFunc::Count,
                column: None,
            })
        );
    }

    /// `fill: {avg: score_value}` (wnba-shots) parses to a typed avg
    /// aggregate carrying its source column.
    #[test]
    fn fill_avg_parses_to_aggregate_with_column() {
        let entry = mark_channel("mark: hexbin\nfill: { avg: score_value }\n", "fill");
        assert_eq!(
            entry,
            ValueOrParamRef::Value(SpecValue::Aggregate {
                func: AggregateFunc::Avg,
                column: Some("score_value".to_string()),
            })
        );
    }

    /// `r: {count:}` (wnba-shots) parses to an aggregate on the r
    /// channel — recorded, deferred at execution, NOT a parse error.
    #[test]
    fn r_count_parses_to_aggregate() {
        let out = parse_spec("mark: hexbin\nr: { count: }\n", Format::Yaml).expect("parses");
        let m = match out.spec.root.unwrap() {
            Component::Mark(m) => m,
            _ => panic!("mark"),
        };
        assert_eq!(
            m.options.get("r"),
            Some(&ValueOrParamRef::Value(SpecValue::Aggregate {
                func: AggregateFunc::Count,
                column: None,
            }))
        );
        assert!(
            !out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::UnknownAggregate { .. })),
            "count is a recognised aggregate — no warning"
        );
    }

    /// `mean` is an accepted alias for `avg`.
    #[test]
    fn mean_aliases_avg() {
        let entry = mark_channel("mark: hexbin\nfill: { mean: v }\n", "fill");
        assert_eq!(
            entry,
            ValueOrParamRef::Value(SpecValue::Aggregate {
                func: AggregateFunc::Avg,
                column: Some("v".to_string()),
            })
        );
    }

    /// an UNKNOWN aggregate name warns (naming it) and degrades to a
    /// plain object — never a silent column lookup for a column named after the
    /// key. The renderer's channel extraction ignores the object.
    #[test]
    fn unknown_aggregate_warns_and_degrades() {
        let out = parse_spec("mark: hexbin\nfill: { stdev: v }\n", Format::Yaml).expect("parses");
        assert!(
            out.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::UnknownAggregate { field, name }
                    if field == "fill" && name == "stdev"
            )),
            "expected UnknownAggregate for fill.stdev, got {:?}",
            out.warnings
        );
        let m = match out.spec.root.unwrap() {
            Component::Mark(m) => m,
            _ => panic!("mark"),
        };
        // Degraded to an object, NOT an aggregate and NOT a column ref.
        assert!(matches!(
            m.options.get("fill"),
            Some(ValueOrParamRef::Value(SpecValue::Object(_)))
        ));
    }

    // -----------------------------------------------------------------------
    // Positional bin + count (the rect histogram idiom)
    // -----------------------------------------------------------------------

    /// A `rectY` binding `{bin: col}` on one positional channel and `{count:}`
    /// on the other lifts BOTH, as a pair — with the `steps:` hint carried.
    #[test]
    fn a_rect_bin_and_count_pair_lifts_together() {
        let src = "mark: rectY\nx: { bin: delay, steps: 60 }\ny: { count: }\nfill: steelblue\n";
        assert_eq!(
            mark_channel(src, "x"),
            ValueOrParamRef::Value(SpecValue::Bin {
                column: "delay".to_string(),
                steps: Some(60),
            })
        );
        assert_eq!(
            mark_channel(src, "y"),
            ValueOrParamRef::Value(SpecValue::Aggregate {
                func: AggregateFunc::Count,
                column: None,
            })
        );
        // The transpose, and the bare `{bin: col}` with no hint.
        assert_eq!(
            mark_channel("mark: rectX\nx: { count: }\ny: { bin: delay }\n", "y"),
            ValueOrParamRef::Value(SpecValue::Bin {
                column: "delay".to_string(),
                steps: None,
            })
        );
    }

    /// Every refusal, in one place. Each leaves the channels as plain objects,
    /// which is what keeps the uncomputed-transform diagnostic firing.
    #[test]
    fn the_bin_lift_refuses_everything_it_cannot_compute() {
        let refused = |src: &str, why: &str| {
            let entry = mark_channel(src, if src.contains("y: { bin") { "y" } else { "x" });
            assert!(
                !matches!(entry, ValueOrParamRef::Value(SpecValue::Bin { .. })),
                "{why}: lifted anyway ({entry:?})"
            );
        };
        // A mark kind with no binning lowerer.
        refused(
            "mark: dot\nx: { bin: delay }\ny: { count: }\n",
            "only the rect family bins positionally",
        );
        // Half the idiom: a bin with nothing to aggregate.
        refused(
            "mark: rectY\nx: { bin: delay }\ny: value\n",
            "a bin with no count opposite it has nothing to group",
        );
        // A column-valued fill: a grouping, not a plain histogram.
        refused(
            "mark: rectY\nx: { bin: delay }\ny: { count: }\nfill: version\n",
            "a column-valued fill is a stack, not a histogram",
        );
        // An explicit `z`, whatever the fill.
        refused(
            "mark: rectY\nx: { bin: delay }\ny: { count: }\nz: version\nfill: steelblue\n",
            "`z` is Mosaic's grouping channel",
        );
        // A modifier the lowerer does not honour. Honouring the `bin` and
        // ignoring the modifier would draw a chart nobody asked for.
        refused(
            "mark: rectY\nx: { bin: delay, interval: day }\ny: { count: }\n",
            "an unknown modifier beside the bin",
        );
        // A bin whose value is not a column name.
        refused(
            "mark: rectY\nx: { bin: { sql: 'a + b' } }\ny: { count: }\n",
            "the bin must name a column",
        );
    }

    /// A CSS colour keyword that is also a COLUMN of the mark's own source
    /// still draws, and says so.
    ///
    /// **The histogram must still lift.** Refusing it would blank a frame
    /// Mosaic renders, over a grouping the spec never asked for. Only the
    /// author's intent is in doubt, which no amount of parsing can settle —
    /// hence a warning and not a refusal. This pins both halves at once,
    /// because getting the warning right while quietly dropping the picture is
    /// the failure that shipped here first.
    ///
    /// Two sources make it a test of the RESOLUTION rather than of the keyword:
    /// identical mark text, one source with a `gold` column and one without,
    /// same lifted result, different diagnostic.
    #[test]
    fn a_colour_name_that_is_also_a_column_still_lifts_and_says_so() {
        let spec = |rows: &str| {
            format!(
                "data:\n  obs: {rows}\nplot:\n  - mark: rectY\n    data: {{ from: obs }}\n    \
                 x: {{ bin: v }}\n    y: {{ count: }}\n    fill: gold\n"
            )
        };
        let parse = |src: &str| parse_spec(src, Format::Yaml).expect("parses");

        // No `gold` column: `fill: gold` is unambiguously the colour, the pair
        // lifts, and nothing is said.
        let clean = parse(&spec("[ { v: 1 }, { v: 2 } ]"));
        let mark = match &clean.spec.root {
            Some(Component::Plot(p)) => match &p.items[0] {
                Component::Mark(m) => m.clone(),
                other => panic!("expected mark, got {other:?}"),
            },
            other => panic!("expected plot, got {other:?}"),
        };
        assert!(
            matches!(
                mark.options.get("x"),
                Some(ValueOrParamRef::Value(SpecValue::Bin { .. }))
            ),
            "an unshadowed colour keyword still lifts: {:?}",
            mark.options.get("x")
        );
        assert!(
            !clean
                .warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::ColourNameShadowsColumn { .. })),
            "nothing to shadow, so nothing to say: {:?}",
            clean.warnings
        );

        // Same mark, a source that HAS a `gold` column. The lift is identical;
        // the only difference is that the collision is named.
        let shadowed = parse(&spec("[ { v: 1, gold: 3 }, { v: 2, gold: 4 } ]"));
        let mark = match &shadowed.spec.root {
            Some(Component::Plot(p)) => match &p.items[0] {
                Component::Mark(m) => m.clone(),
                other => panic!("expected mark, got {other:?}"),
            },
            other => panic!("expected plot, got {other:?}"),
        };
        assert!(
            matches!(
                mark.options.get("x"),
                Some(ValueOrParamRef::Value(SpecValue::Bin { .. }))
            ),
            "a shadowed colour name must not blank a frame Mosaic renders: {:?}",
            mark.options.get("x")
        );
        let uncomputed: Vec<String> = shadowed
            .warnings
            .iter()
            .filter(|w| matches!(w, ParseWarning::UnconsumedChannelTransform { .. }))
            .map(ToString::to_string)
            .collect();
        assert!(
            uncomputed.is_empty(),
            "the pair lifted, so neither channel is uncomputed: {uncomputed:?}"
        );

        // And the collision names itself, so an author who meant the column
        // learns that the spec does not say so.
        let said = shadowed
            .warnings
            .iter()
            .find(|w| matches!(w, ParseWarning::ColourNameShadowsColumn { .. }))
            .map(ToString::to_string)
            .expect("the shadow must be reported");
        assert!(
            said.contains("fill: gold"),
            "names what was written: {said}"
        );
        assert!(said.contains('`'), "quotes the names: {said}");
        assert!(
            said.contains("obs"),
            "names the source that has the column: {said}"
        );

        // The three above pin the message's SHAPE — which names it mentions —
        // and the false message this line replaced satisfied every one, so they
        // are not enough on their own. What follows pins CLAIMS.
        //
        // Both are stated as bans on saying something FALSE, never as a
        // required phrase. An earlier attempt asserted the exact wording and
        // rejected a truer rewrite for not matching it; a message can be
        // rephrased freely, it just may not assert either of these.
        assert!(
            !said.contains("z: "),
            "must not offer `z:` as the remedy — it is dead advice. `z` is a \
             GROUPING_CHANNEL_FIELD, so it refuses the lift, and nothing in the \
             render path reads it: a spec that takes the advice draws a blank \
             frame. Measured, not reasoned — {said}"
        );
        assert!(
            !said.contains("uncomputed"),
            "must not say the pair was left uncomputed: it lifted, and the \
             assertion above proves it — {said}"
        );
        assert!(
            !said.contains("default colour"),
            "must not say the bars stay the default colour: a colour keyword \
             now reaches the canvas, and this clause outlived the gap it \
             described. `crates/brightfield-shell/tests/colour_literals.rs` \
             renders `fill: gold` over a source WITH a `gold` column and reads \
             #ffd700 off the picture — {said}"
        );
    }

    /// The shadow check reads the mark's OWN source, reads the whole file, and
    /// reads every row of it.
    ///
    /// Four failure modes it would be easy to ship: keying off any inline
    /// source rather than the one the mark names (which would report a
    /// collision over a source that does not have the column), depending on
    /// `data:` preceding `plot:` in the document, which is the author's key
    /// order and not a guarantee, harvesting only the first row's keys, which
    /// misses a ragged source, and mistaking the `file:` gap for coverage.
    #[test]
    fn the_shadow_is_resolved_against_the_marks_own_source_whatever_the_key_order() {
        let quiet = |src: &str| {
            let out = parse_spec(src, Format::Yaml).expect("parses");
            !out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::ColourNameShadowsColumn { .. }))
        };
        // `plot:` written first, `data:` last: the harvest is a pre-pass, so
        // the column is still found.
        assert!(
            !quiet(
                "plot:\n  - mark: rectY\n    data: { from: obs }\n    x: { bin: v }\n    \
                 y: { count: }\n    fill: gold\ndata:\n  obs: [ { v: 1, gold: 3 } ]\n"
            ),
            "a source declared after the mark that reads it is still its source"
        );
        // A `gold` column on a DIFFERENT source is not this mark's problem.
        assert!(
            quiet(
                "data:\n  obs: [ { v: 1 } ]\n  other: [ { gold: 3 } ]\nplot:\n  - mark: rectY\n    \
                 data: { from: obs }\n    x: { bin: v }\n    y: { count: }\n    fill: gold\n"
            ),
            "the shadow is per-source, not per-document"
        );
        // A RAGGED source: `gold` appears only on the second row. Inline rows
        // are free-form mappings, so the column set is the union of every
        // row's keys and not the first row's. Harvesting one row would go
        // quiet here, which is the expensive direction to be wrong in — the
        // author who meant the column would never learn they lost it.
        assert!(
            !quiet(
                "data:\n  obs: [ { v: 1 }, { v: 2, gold: 3 } ]\nplot:\n  - mark: rectY\n    \
                 data: { from: obs }\n    x: { bin: v }\n    y: { count: }\n    fill: gold\n"
            ),
            "the column set is the union of every row's keys, not the first row's"
        );
        // A `file:` source carries no schema here, so the same spec is
        // unresolvable and keeps the reference reading. This is the honest
        // limit of the check, and pinning it stops it being mistaken for
        // coverage it does not have.
        assert!(
            quiet(
                "data:\n  obs: { file: obs.parquet }\nplot:\n  - mark: rectY\n    \
                 data: { from: obs }\n    x: { bin: v }\n    y: { count: }\n    fill: gold\n"
            ),
            "nothing in the parse path knows a parquet's columns"
        );
    }

    /// The lifted pair re-serialises to the shape it was written in, so
    /// `parse → serialise → parse` stays idempotent. The `steps:` key appears
    /// only when the spec wrote one.
    #[test]
    fn a_lifted_bin_re_serialises_to_its_wire_form() {
        for src in [
            "mark: rectY\nx: { bin: delay }\ny: { count: }\n",
            "mark: rectY\nx: { bin: delay, steps: 60 }\ny: { count: }\n",
        ] {
            let first = parse_spec(src, Format::Yaml).expect("parses");
            let wire = serialise_spec(&first.spec).expect("serialises");
            let second = parse_spec(&wire, Format::Yaml).expect("re-parses");
            assert_eq!(
                first.spec, second.spec,
                "round trip changed the spec; wire form was:\n{wire}"
            );
        }
        let wire = serialise_spec(
            &parse_spec(
                "mark: rectY\nx: { bin: delay }\ny: { count: }\n",
                Format::Yaml,
            )
            .expect("parses")
            .spec,
        )
        .expect("serialises");
        assert!(
            !wire.contains("steps"),
            "a spec that took the default must not gain a `steps` key: {wire}"
        );
    }

    /// plain column/literal/param fill channels are untouched by
    /// aggregate detection.
    #[test]
    fn plain_fill_channels_untouched() {
        // String column.
        assert_eq!(
            mark_channel("mark: dot\nfill: species\n", "fill"),
            ValueOrParamRef::Value(SpecValue::String("species".to_string()))
        );
        // Literal colour.
        assert_eq!(
            mark_channel("mark: dot\nfill: steelblue\n", "fill"),
            ValueOrParamRef::Value(SpecValue::String("steelblue".to_string()))
        );
        // Param ref still lifts to the outer Param wrapper.
        assert!(mark_channel("mark: dot\nfill: $c\n", "fill").is_param());
        // `{param: c}` shorthand at fill still lifts to a ParamRef, not an aggregate.
        assert!(mark_channel("mark: dot\nfill: { param: c }\n", "fill").is_param());
    }

    /// the three vendored corpus specs with aggregate channels parse
    /// cleanly (no error) after the aggregate form lands.
    #[test]
    fn vendored_hexbin_corpus_parses() {
        for name in ["flights-hexbin", "wnba-shots", "mark-types"] {
            let path = format!(
                "{}/vendor/mosaic-specs/yaml/{name}.yaml",
                env!("CARGO_MANIFEST_DIR")
            );
            let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
            parse_spec(&src, Format::Yaml)
                .unwrap_or_else(|e| panic!("corpus {name} failed to parse: {e}"));
        }
    }

    /// F2 (review): a Mosaic `{sql: …}` channel-transform expression is NOT a
    /// typo'd aggregate — the vendored specs that carry `r: {sql: 'POW(10, mag)'}`
    /// must parse with ZERO `UnknownAggregate` warnings (the warning is
    /// author-facing via the app's stderr).
    #[test]
    fn f2_sql_channel_expression_emits_no_unknown_aggregate_warning() {
        for name in ["region-tests", "earthquakes-feed", "earthquakes-globe"] {
            let path = format!(
                "{}/vendor/mosaic-specs/yaml/{name}.yaml",
                env!("CARGO_MANIFEST_DIR")
            );
            let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
            let out = parse_spec(&src, Format::Yaml)
                .unwrap_or_else(|e| panic!("corpus {name} failed to parse: {e}"));
            let unknown: Vec<_> = out
                .warnings
                .iter()
                .filter(|w| matches!(w, ParseWarning::UnknownAggregate { .. }))
                .collect();
            assert!(
                unknown.is_empty(),
                "corpus {name}: sql channel transform must not warn, got {unknown:?}"
            );
        }
    }

    /// an aggregate channel round-trips through serialise → parse.
    #[test]
    fn aggregate_channel_round_trips() {
        let src = "mark: hexbin\nfill: { avg: score_value }\nr: { count: }\n";
        let a = parse_spec(src, Format::Yaml).expect("first parse");
        let serialised = serde_yaml::to_string(&a.spec).expect("serialise");
        let b = parse_spec(&serialised, Format::Yaml).expect("second parse");
        assert_eq!(a.spec, b.spec, "serialised:\n{serialised}");
    }

    /// case (c) — post-D2: a `meta:` unknown field is not a
    /// `SchemaViolation`; it is a `ParseWarning::UnknownOption`. Locking
    /// the D2 adjustment against regression.
    #[test]
    fn meta_unknown_field_is_warning_not_error() {
        let src = "meta:\n  bogus: x\n  title: t\n";
        let out = parse_spec(src, Format::Yaml).expect("parses under D2");
        assert!(
            out.warnings.iter().any(|w| matches!(
                w,
                ParseWarning::UnknownOption { path, key } if path == "meta" && key == "bogus"
            )),
            "expected UnknownOption warning for meta.bogus, got {:?}",
            out.warnings
        );
    }

    /// a `highlight` interactor's `by:` lifts to a `Param`
    /// ref — symmetric with a producer's `as:` — while its literal `opacity`
    /// override stays a plain value. No `Unimplemented` warning fires (Highlight
    /// is already vocab `Implemented`).
    #[test]
    fn highlight_by_lifts_to_param() {
        let yaml = r#"
params:
  brush: { select: single }
plot:
  - mark: dot
    data: { from: t }
    x: a
    y: b
  - select: intervalXY
    as: $brush
  - select: highlight
    by: $brush
    opacity: 0.1
"#;
        let out = parse_spec(yaml, Format::Yaml).expect("parses");
        let Some(Component::Plot(plot)) = &out.spec.root else {
            panic!("expected a plot root");
        };
        let hl = plot
            .items
            .iter()
            .find_map(|it| match it {
                Component::Interactor(i) if i.kind == InteractorKind::Highlight => Some(i),
                _ => None,
            })
            .expect("highlight interactor present");
        assert!(
            matches!(hl.options.get("by"), Some(ValueOrParamRef::Param(pr)) if pr.0 == "brush"),
            "`by:` must lift to a Param ref, got {:?}",
            hl.options.get("by")
        );
        assert!(
            matches!(
                hl.options.get("opacity"),
                Some(ValueOrParamRef::Value(SpecValue::Float(f))) if (*f - 0.1).abs() < 1e-9
            ),
            "literal opacity stays a plain value"
        );
        assert!(
            !out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::Unimplemented { .. })),
            "highlight is Implemented — no Unimplemented warning"
        );
    }

    // --- design phase 4 PR B: `colorScheme: meridian` export expansion ---

    /// `colorScheme: meridian` is Brightfield-local sugar (DEV-0004): export
    /// expands it to the explicit 13-stop `colorRange`, so the emitted spec
    /// stays vanilla-Mosaic-portable. Portable scheme names pass through.
    #[test]
    fn dsb_serialise_expands_meridian_scheme_to_color_range() {
        let src = "\
data:
  t: { file: data/t.parquet }
plot:
  - mark: raster
    data: { from: t }
    x: u
    y: v
colorScheme: meridian
";
        let spec = parse_spec(src, Format::Yaml).expect("parses").spec;
        let out = serialise_spec(&spec).expect("serialises");
        assert!(
            !out.contains("colorScheme"),
            "the Brightfield-local scheme name must not be exported:\n{out}"
        );
        assert!(out.contains("colorRange"), "expanded to colorRange:\n{out}");
        for stop in [MERIDIAN_COLOR_RANGE_HEX[0], MERIDIAN_COLOR_RANGE_HEX[12]] {
            assert!(
                out.contains(stop),
                "colorRange carries the ramp stop {stop}:\n{out}"
            );
        }
        // The expansion re-parses cleanly (the export is consumable).
        let reparsed = parse_spec(&out, Format::Yaml).expect("expanded export re-parses");
        let plots = crate::layout::collect_plot_nodes(&reparsed.spec);
        let (_, node) = plots.first().expect("one plot");
        assert!(
            matches!(node.attributes.get("colorRange"), Some(SpecValue::Array(a)) if a.len() == 13),
            "re-parsed export carries the 13 explicit stops"
        );

        // A portable scheme name is exported unchanged.
        let portable = src.replace("meridian", "viridis");
        let spec2 = parse_spec(&portable, Format::Yaml).expect("parses").spec;
        let out2 = serialise_spec(&spec2).expect("serialises");
        assert!(
            out2.contains("colorScheme: viridis") && !out2.contains("colorRange"),
            "portable scheme names pass through untouched:\n{out2}"
        );
    }

    /// With an explicit `colorRange` ALSO present, the sugar is dropped
    /// instead of expanded — never a duplicate `colorRange` key.
    #[test]
    fn dsb_serialise_meridian_with_explicit_color_range_drops_sugar() {
        let src = "\
data:
  t: { file: data/t.parquet }
plot:
  - mark: raster
    data: { from: t }
    x: u
    y: v
colorScheme: meridian
colorRange: ['#000000', '#ffffff']
";
        let spec = parse_spec(src, Format::Yaml).expect("parses").spec;
        let out = serialise_spec(&spec).expect("serialises");
        assert!(!out.contains("colorScheme"), "sugar dropped:\n{out}");
        assert_eq!(
            out.matches("colorRange").count(),
            1,
            "exactly one colorRange key:\n{out}"
        );
        assert!(
            out.contains("#000000") && !out.contains(MERIDIAN_COLOR_RANGE_HEX[0]),
            "the author's explicit range wins:\n{out}"
        );
    }
}
