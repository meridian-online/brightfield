//! ChannelMap — typed channel extraction from mark options.
//!
//! A ChannelMap maps visual encoding channels (x, y, fill, stroke, size, etc.)
//! to column names in the RecordBatch. This bridges the spec's mark options
//! and the rendering pipeline.

use std::collections::HashMap;

use brightfield_spec::ast::{AggregateFunc, Mark, PlotNode, SpecValue, ValueOrParamRef};
use brightfield_spec::vocab::is_colour_literal;
use brightfield_spec::vocab::MarkKind;
use peniko::Color;

/// Reserved output column the density / hexbin / cell lowerers alias their
/// per-group occupancy count to. Must match `brightfield-sql`'s `__bf_count`
/// alias and `brightfield-render::mark::DENSITY_COUNT_COL`; a `fill: {count:}`
/// channel is read from this column, not a user column.
const AGGREGATE_COUNT_COL: &str = "__bf_count";

/// Reserved HIGH bin-edge columns a binned rect's lowerer emits, one per axis.
/// Must match `brightfield-sql`'s `BIN_HI_X_COL` / `BIN_HI_Y_COL`. The LOW edge
/// carries the source column's own name, so it needs no reserved name here.
///
/// The two sides agree by CONVENTION rather than by construction, exactly as
/// `__bf_count` and `__bf_hex_dx` already do: [`ChannelMap::from_mark`] is
/// handed the mark AST and never the plan, so there is nothing to read the
/// emitted alias off.
const BIN_HI_X_COL: &str = "__bf_bin_x2";
const BIN_HI_Y_COL: &str = "__bf_bin_y2";

/// Visual encoding channels recognised by the rendering pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    X,
    Y,
    Fill,
    Stroke,
    Size,
    X1,
    Y1,
    X2,
    Y2,
    /// Text content channel (the label column for text marks).
    Text,
}

impl Channel {
    /// The wire name as it appears in Mosaic mark options.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::X => "x",
            Self::Y => "y",
            Self::Fill => "fill",
            Self::Stroke => "stroke",
            Self::Size => "size",
            Self::X1 => "x1",
            Self::Y1 => "y1",
            Self::X2 => "x2",
            Self::Y2 => "y2",
            Self::Text => "text",
        }
    }

    /// All known channel wire names.
    pub fn all() -> &'static [Self] {
        &[
            Self::X,
            Self::Y,
            Self::Fill,
            Self::Stroke,
            Self::Size,
            Self::X1,
            Self::Y1,
            Self::X2,
            Self::Y2,
            Self::Text,
        ]
    }

    /// Look up a channel by wire name.
    pub fn from_wire(name: &str) -> Option<Self> {
        match name {
            "x" => Some(Self::X),
            "y" => Some(Self::Y),
            "fill" => Some(Self::Fill),
            "stroke" => Some(Self::Stroke),
            "size" => Some(Self::Size),
            "x1" => Some(Self::X1),
            "y1" => Some(Self::Y1),
            "x2" => Some(Self::X2),
            "y2" => Some(Self::Y2),
            "text" => Some(Self::Text),
            _ => None,
        }
    }
}

/// The number a mark prints on its own geometry, and what it is a number OF.
///
/// Not a [`Channel`]: a channel binds a column and takes part in scale
/// inference, and this binds neither — it names an ARITHMETIC the renderer does
/// over columns the mark already carries. Binding it as a channel would put it
/// in front of `infer_scales` and give a bar chart a phantom axis.
///
/// Both forms read the same two numbers, and which two depends on the batch
/// rather than on the form:
///
/// * at rest a mark carries one number per group — the group's own aggregate;
/// * under a live `highlight` a mark that aggregates additionally carries the
///   count of rows in that group the selection accounts for
///   (`brightfield-sql`'s `__bf_selected_count`, read in `crate::mark` through
///   the private `SELECTED_COUNT_COLUMN` — a private helper, so this is a name
///   and not a link).
///
/// So a label reads `total` at rest and `selected / total` under a selection,
/// in whichever units the form names. The second is the in-bar form of the
/// part-of-whole reading the geometry already draws, which is the point: the
/// label pins the fraction the ink is showing rather than restating the axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelForm {
    /// The numbers themselves — `1234`, and `567 / 1234` under a selection.
    Count,
    /// The same two numbers as whole percentages **of the values this mark
    /// drew** — `36%`, and `17% / 36%` under a selection.
    ///
    /// Of the drawn values and not of the table: a `sort: { limit: n }` leaves
    /// the mark holding the top `n` groups, and the percentages then sum to
    /// 100% over what is on the page rather than over what was aggregated. That
    /// is the only denominator a renderer holding one batch has; a share of the
    /// whole table would need the untruncated total projected alongside.
    Percent,
}

impl LabelForm {
    /// The spec spelling.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Percent => "percent",
        }
    }

    /// Read a spec spelling. `None` for anything else, which leaves the mark
    /// unlabelled — the same silence an unrecognised channel value gets.
    #[must_use]
    pub fn from_wire(name: &str) -> Option<Self> {
        match name {
            "count" => Some(Self::Count),
            "percent" => Some(Self::Percent),
            _ => None,
        }
    }
}

/// Maps visual encoding channels to column names in the RecordBatch, plus any
/// channels bound to a constant **literal** value (e.g. `y: 0` for a baseline
/// rule, or `fill: steelblue` for constant ink). Columns and literals are kept
/// in separate maps so existing column-based renderers are unaffected;
/// renderers that accept constants read [`ChannelMap::literal`] or
/// [`ChannelMap::colour`].
///
/// It also carries the mark options that are not channels and that a
/// RENDERER (rather than a lowerer) reads — see [`ChannelMap::label`] and
/// [`ChannelMap::equal_aspect`]. This is the route such an option has:
/// [`crate::mark::MarkRenderer::render`] and
/// [`crate::mark::MarkRenderer::augment_scales`] are handed a batch, a channel
/// map and a scale set, not the mark itself.
#[derive(Debug, Clone, Default)]
pub struct ChannelMap {
    map: HashMap<Channel, String>,
    literals: HashMap<Channel, f64>,
    colours: HashMap<Channel, Color>,
    label: Option<LabelForm>,
    equal_aspect: bool,
    projection: MarkProjection,
}

/// What the plot's map projection means for one mark on it.
///
/// A projection is a plot attribute in Mosaic, as it is in Observable Plot, and
/// it replaces the plot's x and y scales. So the question is never "which
/// projection did this mark ask for" — it is "what does the plot's projection do
/// to this mark", and there are exactly three answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarkProjection {
    /// The plot names no projection. The mark draws at its raw column numbers
    /// on cartesian axes — every scatter, bar and line in the build.
    #[default]
    None,
    /// The plot names a projection and this mark draws through it.
    Through(crate::mark::Projection),
    /// The plot names a projection and this mark's kind cannot draw through it
    /// ([`MarkKind::draws_through_a_projection`](brightfield_spec::vocab::MarkKind::draws_through_a_projection)).
    ///
    /// **The mark is not drawn.** The plot's axes are in the projection's planar
    /// units — a Mercator `v` of 1.47 where the column says 64 — so drawing this
    /// mark's degrees against them puts a second coordinate system on top of the
    /// first, which reads as a picture rather than as an error. `render_entry`
    /// (`crate::scene`) is the one place that acts on this, and
    /// `ParseWarning::MarkCannotProject` is what tells the author.
    Undrawable(crate::mark::Projection),
}

impl MarkProjection {
    /// What `plot`'s projection means for a mark of kind `kind`.
    ///
    /// **The one delivery of a projection to a mark in the build.** `plot` is
    /// `None` for a mark with no owning plot node.
    ///
    /// A `geo` mark is the one kind that projects even when the plot names
    /// nothing: its column holds longitude/latitude geometry and there is no
    /// cartesian reading of it, so an unnamed projection means the plate carrée
    /// rather than no projection at all — which is what this build has always
    /// drawn for a `geo` mark on a bare plot.
    #[must_use]
    pub fn of(kind: MarkKind, plot: Option<&PlotNode>) -> Self {
        Self::of_resolved(
            kind,
            plot.and_then(brightfield_spec::layout::resolve_projection),
        )
    }

    /// [`MarkProjection::of`] when the plot's projection has already been
    /// resolved — the live-rebuild path, which reads the plot node before it
    /// swaps the spec and cannot hold the borrow across it. The one decision
    /// lives here; `of` is a thin wrapper that resolves first.
    #[must_use]
    pub fn of_resolved(
        kind: MarkKind,
        named: Option<brightfield_spec::layout::ResolvedProjection>,
    ) -> Self {
        match (kind, named) {
            (MarkKind::Geo, named) => {
                Self::Through(named.map(crate::mark::Projection::from).unwrap_or_default())
            }
            (_, None) => Self::None,
            (kind, Some(named)) if kind.draws_through_a_projection() => {
                Self::Through(crate::mark::Projection::from(named))
            }
            (_, Some(named)) => Self::Undrawable(crate::mark::Projection::from(named)),
        }
    }

    /// The projection this mark DRAWS through, or `None` when it draws
    /// cartesian or is not drawn at all.
    #[must_use]
    pub fn drawn(self) -> Option<crate::mark::Projection> {
        match self {
            Self::Through(p) => Some(p),
            Self::None | Self::Undrawable(_) => None,
        }
    }

    /// Whether the plot's projection leaves this mark undrawable.
    #[must_use]
    pub fn is_undrawable(self) -> bool {
        matches!(self, Self::Undrawable(_))
    }

    /// The projection the PLOT names, whether or not this mark can draw through
    /// it. What the plot's axes are in.
    #[must_use]
    pub fn on_the_plot(self) -> Option<crate::mark::Projection> {
        match self {
            Self::Through(p) | Self::Undrawable(p) => Some(p),
            Self::None => None,
        }
    }
}

impl ChannelMap {
    /// Create an empty channel map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a channel -> column mapping.
    pub fn insert(&mut self, channel: Channel, column: String) {
        self.map.insert(channel, column);
    }

    /// Insert a channel -> constant literal value (e.g. `y: 0`).
    pub fn insert_literal(&mut self, channel: Channel, value: f64) {
        self.literals.insert(channel, value);
    }

    /// Insert a channel -> constant COLOUR (e.g. `fill: steelblue`).
    pub fn insert_colour(&mut self, channel: Channel, colour: Color) {
        self.colours.insert(channel, colour);
    }

    /// The constant colour bound to a channel, if the spec wrote a colour
    /// literal there rather than a column name.
    ///
    /// A channel is never in both this map and [`ChannelMap::get`]'s — a
    /// colour-channel string is one or the other, decided lexically by
    /// [`is_colour_literal`]. A renderer that consults this FIRST and the
    /// column path second therefore cannot double-resolve.
    pub fn colour(&self, channel: Channel) -> Option<Color> {
        self.colours.get(&channel).copied()
    }

    /// Look up the column name for a channel.
    pub fn get(&self, channel: Channel) -> Option<&str> {
        self.map.get(&channel).map(|s| s.as_str())
    }

    /// Look up the constant literal value bound to a channel, if any.
    pub fn literal(&self, channel: Channel) -> Option<f64> {
        self.literals.get(&channel).copied()
    }

    /// Iterator over channels bound to a literal value.
    pub fn literals_iter(&self) -> impl Iterator<Item = (Channel, f64)> + '_ {
        self.literals.iter().map(|(c, v)| (*c, *v))
    }

    /// True if the channel is mapped to a column.
    pub fn has(&self, channel: Channel) -> bool {
        self.map.contains_key(&channel)
    }

    /// Set the in-bar label form (the `label:` mark option).
    pub fn set_label(&mut self, label: LabelForm) {
        self.label = Some(label);
    }

    /// The label form this mark asked to print on its own geometry, or `None`
    /// when it wrote no `label:` — which is every mark in the corpus that
    /// predates the option, so an unlabelled mark draws exactly as before.
    #[must_use]
    pub fn label(&self) -> Option<LabelForm> {
        self.label
    }

    /// Set whether this mark asked for an equal-aspect frame — see
    /// [`ChannelMap::equal_aspect`].
    pub fn set_equal_aspect(&mut self, on: bool) {
        self.equal_aspect = on;
    }

    /// Set what the plot's projection means for this mark — see
    /// [`MarkProjection`]. Called by [`ChannelMap::from_mark_in`] and by tests
    /// that build a map without a spec behind it.
    pub fn set_projection(&mut self, projection: MarkProjection) {
        self.projection = projection;
    }

    /// The map projection this mark draws through, or `None` for a cartesian
    /// mark and for one the plot's projection leaves undrawable.
    ///
    /// `crate::mark::DotRenderer` reads this in both halves of its work: its
    /// `MarkRenderer::augment_scales` fits the PROJECTED coordinates rather than
    /// the raw ones, and its `MarkRenderer::render` places each point through
    /// the projection and draws a graticule behind them.
    #[must_use]
    pub fn projection(&self) -> Option<crate::mark::Projection> {
        self.projection.drawn()
    }

    /// What the plot's projection means for this mark, undrawability included.
    #[must_use]
    pub fn mark_projection(&self) -> MarkProjection {
        self.projection
    }

    /// Whether this mark wrote `aspectRatio: 1`, asking the positional axes it
    /// binds to share one px-per-unit rather than each fitting its own domain
    /// to the plot rect independently — **and is not also projected**.
    ///
    /// A mark option rather than a channel for the reason [`Self::label`]
    /// already is one: there is no column behind it, so `infer_scales` reads
    /// no channel from it. `crate::mark::DotRenderer`'s
    /// `MarkRenderer::augment_scales` implementation is the one reader — a
    /// plain `dot` mark that does not write `aspectRatio: 1` draws exactly as
    /// before, held by `augment_scales_without_the_flag_leaves_scales_untouched`
    /// in that crate's test module.
    ///
    /// # Why a projection refuses it rather than composing with it
    ///
    /// Equal-aspect exists because a point map had no projection: it widens the
    /// narrower of two positional domains until a degree of longitude and a
    /// degree of latitude occupy the same number of pixels, which is the best a
    /// cartesian frame can do at impersonating a map. A projection has already
    /// answered that question — correctly, and differently at every latitude —
    /// and the renderer aspect-fits its output, so widening on top of it would
    /// stretch a map that was right. The two are alternatives, not layers.
    ///
    /// The refusal is HERE, in the accessor a reader goes through, rather than in
    /// the setters or in the renderer — the test
    /// `equal_aspect_and_a_projection_cannot_both_apply` drives both write orders
    /// against it. A setter-side refusal would depend on which of the two was
    /// written first, and a renderer-side one would have to be repeated by each
    /// renderer that grows a projection.
    #[must_use]
    pub fn equal_aspect(&self) -> bool {
        self.equal_aspect && self.projection.on_the_plot().is_none()
    }

    /// Extract a ChannelMap from a mark's options.
    ///
    /// Scans the mark's options for known channel names (x, y, fill, etc.)
    /// and maps them to column name strings. ParamRef channels are skipped
    /// with a warning — they require reactive parameter resolution which is
    /// not yet implemented.
    ///
    /// **A colour channel is the one place a string is not always a column.**
    /// `fill: steelblue` is constant ink and `fill: species` is a column, and
    /// the string itself is the only discriminator — the same overload
    /// [`is_colour_literal`] exists to settle, and the same call the parser
    /// makes when it decides whether a binned rect carries groups. Reading both
    /// through one predicate is what keeps the two sides from disagreeing about
    /// which one a spec wrote.
    pub fn from_mark(mark: &Mark) -> Self {
        let mut cm = Self::new();
        for ch in Channel::all() {
            if let Some(val) = mark.options.get(ch.wire_name()) {
                match val {
                    ValueOrParamRef::Value(SpecValue::String(col))
                        if matches!(ch, Channel::Fill | Channel::Stroke)
                            && is_colour_literal(col) =>
                    {
                        // A colour constant is NOT a column name, so it must not
                        // be bound as one: doing that sends the renderer looking
                        // for a column no batch carries, and the fall-through
                        // paints the default mark ink — which is the whole
                        // defect this arm exists to close.
                        //
                        // Unresolvable-but-recognised forms (`currentColor`,
                        // `rgb(…)`) bind NOTHING and keep the default. That is a
                        // gap; see `mark::parse_colour_literal`.
                        if let Some(colour) = crate::mark::parse_colour_literal(col) {
                            cm.insert_colour(*ch, colour);
                        }
                    }
                    ValueOrParamRef::Value(SpecValue::String(col)) => {
                        cm.insert(*ch, col.clone());
                    }
                    // Numeric literals bind the channel to a constant for all
                    // rows (e.g. `y: 0` for a baseline rule).
                    ValueOrParamRef::Value(SpecValue::Integer(i)) => {
                        cm.insert_literal(*ch, *i as f64);
                    }
                    ValueOrParamRef::Value(SpecValue::Float(f)) => {
                        cm.insert_literal(*ch, *f);
                    }
                    // A self-aggregating channel (`fill: {count:}` / `{avg:
                    // col}`) maps to the column the lowerer emits: the reserved
                    // count column for `count`, or the source column for the
                    // column-taking aggregates (aliased to itself in SQL). The
                    // renderer then reads it like any numeric fill column.
                    ValueOrParamRef::Value(SpecValue::Aggregate { func, column }) => {
                        let col = match func {
                            AggregateFunc::Count => AGGREGATE_COUNT_COL.to_string(),
                            AggregateFunc::Sum
                            | AggregateFunc::Avg
                            | AggregateFunc::Min
                            | AggregateFunc::Max => match column {
                                Some(c) => c.clone(),
                                None => continue,
                            },
                        };
                        cm.insert(*ch, col);
                    }
                    // A positional bin binds THREE channels off one key. The
                    // two interval channels are what the rect actually draws
                    // (its lowerer emits the low edge under the source column's
                    // own name and the high edge under a reserved one); the
                    // bare positional channel is bound as well, so the axis
                    // still derives its title from the column the author named
                    // and `infer_scales` sees a quantitative axis there.
                    ValueOrParamRef::Value(SpecValue::Bin { column, .. }) => {
                        let (lo, hi, hi_col) = match ch {
                            Channel::X => (Channel::X1, Channel::X2, BIN_HI_X_COL),
                            Channel::Y => (Channel::Y1, Channel::Y2, BIN_HI_Y_COL),
                            // A bin on a non-positional channel has no interval
                            // to draw and no lowerer; leave it unbound rather
                            // than invent a column the batch does not carry.
                            _ => continue,
                        };
                        cm.insert(lo, column.clone());
                        cm.insert(hi, hi_col.to_string());
                        cm.insert(*ch, column.clone());
                    }
                    ValueOrParamRef::Param(param_ref) => {
                        // A positional channel bound to a `$param` is projected
                        // into the query as `$param AS "<param>"` by the lowerer
                        // (Decision 2), sourced from param_state at
                        // emit time. Map it to that param-named column so the
                        // renderer reads the interpolated value — and so a
                        // param change flows through to the render. Non-positional
                        // channels (fill/stroke/size/text) bound to a param are
                        // the deferred render-only case (Decision 5).
                        if matches!(
                            ch,
                            Channel::X
                                | Channel::Y
                                | Channel::X1
                                | Channel::Y1
                                | Channel::X2
                                | Channel::Y2
                        ) {
                            cm.insert(*ch, param_ref.0.clone());
                        } else {
                            eprintln!(
                                "warning: skipping non-positional channel `{}` \
                                 bound to param `{}` — render-only param channels \
                                 are not yet supported",
                                ch.wire_name(),
                                param_ref.to_wire()
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        // The non-channel options read here. Both are scanned outside the
        // channel loop because neither binds a column: there is no column for
        // `infer_scales` to see and no scale to be built over either.
        if let Some(ValueOrParamRef::Value(SpecValue::String(form))) = mark.options.get("label") {
            if let Some(form) = LabelForm::from_wire(form) {
                cm.set_label(form);
            }
        }
        // `aspectRatio: 1` — the one ratio this build honours. A different
        // number is not a request this renderer refuses; it is a request
        // nothing here reads yet, so it is left unbound rather than rejected,
        // the same silence an unrecognised `label:` value gets.
        let asks_equal_aspect = match mark.options.get("aspectRatio") {
            Some(ValueOrParamRef::Value(SpecValue::Integer(1))) => true,
            Some(ValueOrParamRef::Value(SpecValue::Float(f))) => (*f - 1.0).abs() < f64::EPSILON,
            _ => false,
        };
        if asks_equal_aspect {
            cm.set_equal_aspect(true);
        }
        cm
    }

    /// [`ChannelMap::from_mark`] for a mark inside a plot — the plot's map
    /// projection applied to it.
    ///
    /// **This is the delivery seam.** `from_mark` alone cannot see the plot, and
    /// a projection is a plot attribute: the mark's own options never mention
    /// one, and Mosaic has no key with which they could. Every composition path
    /// that has a plot node calls this; a caller with no plot (a bare
    /// composition-level mark, a unit test with no spec) calls `from_mark` and
    /// gets the cartesian reading, which is what such a mark has.
    #[must_use]
    pub fn from_mark_in(mark: &Mark, plot: Option<&PlotNode>) -> Self {
        let mut cm = Self::from_mark(mark);
        cm.set_projection(MarkProjection::of(mark.kind, plot));
        cm
    }

    /// Iterator over all mapped channels.
    pub fn iter(&self) -> impl Iterator<Item = (&Channel, &String)> {
        self.map.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_channel_from_wire_round_trips() {
        for ch in Channel::all() {
            assert_eq!(Channel::from_wire(ch.wire_name()), Some(*ch));
        }
        assert_eq!(Channel::from_wire("unknown"), None);
    }

    #[test]
    fn gpu_channel_map_insert_and_get() {
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "col_x".to_string());
        cm.insert(Channel::Fill, "species".to_string());
        assert_eq!(cm.get(Channel::X), Some("col_x"));
        assert_eq!(cm.get(Channel::Fill), Some("species"));
        assert_eq!(cm.get(Channel::Y), None);
        assert!(cm.has(Channel::X));
        assert!(!cm.has(Channel::Y));
    }

    /// a POSITIONAL channel bound to a `$param` maps to
    /// the param-named column (the lowerer projects `$param AS "<param>"`), so
    /// the renderer reads the interpolated value. A NON-positional channel
    /// (fill/stroke/size/text) bound to a param is still skipped (the deferred
    /// render-only case). Supersedes the old skip-everything behaviour.
    #[test]
    fn from_mark_maps_positional_param_channel() {
        use brightfield_spec::ast::{Mark, ParamRef, SpecValue, ValueOrParamRef};
        use brightfield_spec::vocab::{ImplStatus, MarkKind};

        let mut options: indexmap::IndexMap<String, ValueOrParamRef<SpecValue>> =
            Default::default();
        // y (positional) is a ParamRef — now mapped to the param-named column.
        options.insert(
            "y".to_string(),
            ValueOrParamRef::Param(ParamRef::new("threshold")),
        );
        // x is a literal column reference.
        options.insert(
            "x".to_string(),
            ValueOrParamRef::Value(SpecValue::String("col_x".to_string())),
        );
        // fill (non-positional) is a ParamRef — still skipped (render-only case).
        options.insert(
            "fill".to_string(),
            ValueOrParamRef::Param(ParamRef::new("colour")),
        );

        let mark = Mark {
            kind: MarkKind::Dot,
            status: ImplStatus::Implemented,
            data: None,
            options,
        };

        let cm = ChannelMap::from_mark(&mark);
        // Positional param channel → mapped to the param name.
        assert_eq!(
            cm.get(Channel::Y),
            Some("threshold"),
            "positional $param channel maps to the param-named column"
        );
        // Column ref unchanged.
        assert_eq!(cm.get(Channel::X), Some("col_x"));
        // Non-positional param channel → still absent (deferred render-only).
        assert!(
            !cm.has(Channel::Fill),
            "non-positional $param channel is still skipped"
        );
    }

    #[test]
    fn from_mark_captures_numeric_literal_channel() {
        use brightfield_spec::ast::{Mark, SpecValue, ValueOrParamRef};
        use brightfield_spec::vocab::{ImplStatus, MarkKind};

        let mut options: indexmap::IndexMap<String, ValueOrParamRef<SpecValue>> =
            Default::default();
        // x is a column reference; y is a numeric literal (e.g. a baseline).
        options.insert(
            "x".to_string(),
            ValueOrParamRef::Value(SpecValue::String("col_x".to_string())),
        );
        options.insert(
            "y".to_string(),
            ValueOrParamRef::Value(SpecValue::Integer(0)),
        );

        let mark = Mark {
            kind: MarkKind::RuleY,
            status: ImplStatus::Implemented,
            data: None,
            options,
        };

        let cm = ChannelMap::from_mark(&mark);
        assert_eq!(cm.get(Channel::X), Some("col_x"), "x is a column");
        assert_eq!(cm.get(Channel::Y), None, "a literal y is not a column");
        assert_eq!(
            cm.literal(Channel::Y),
            Some(0.0),
            "literal y captured as 0.0"
        );
        assert_eq!(cm.literal(Channel::X), None, "x has no literal");
    }

    /// A positional bin binds THREE channels off one key, and the columns it
    /// names are the ones `RectLowerer` emits.
    ///
    /// The interval pair is what `RectRenderer` actually draws — a `rectY`
    /// reads `X1`/`X2` and never `X` for its edges — so binding only the bare
    /// channel would leave `axis_edges` returning `None` and the mark blank.
    /// Binding `X` as well is what gives the axis its derived title and puts a
    /// quantitative scale on it.
    #[test]
    fn from_mark_synthesises_the_interval_pair_for_a_positional_bin() {
        use brightfield_spec::ast::{AggregateFunc, Mark, PlotNode, SpecValue, ValueOrParamRef};
        use brightfield_spec::vocab::MarkKind;
        use brightfield_spec::vocab::{ImplStatus, MarkKind};

        let mut options: indexmap::IndexMap<String, ValueOrParamRef<SpecValue>> =
            Default::default();
        options.insert(
            "x".to_string(),
            ValueOrParamRef::Value(SpecValue::Bin {
                column: "delay".to_string(),
                steps: None,
            }),
        );
        options.insert(
            "y".to_string(),
            ValueOrParamRef::Value(SpecValue::Aggregate {
                func: AggregateFunc::Count,
                column: None,
            }),
        );

        let cm = ChannelMap::from_mark(&Mark {
            kind: MarkKind::RectY,
            status: ImplStatus::Implemented,
            data: None,
            options: options.clone(),
        });
        assert_eq!(
            cm.get(Channel::X1),
            Some("delay"),
            "the LOW edge is emitted under the source column's own name"
        );
        assert_eq!(cm.get(Channel::X2), Some(BIN_HI_X_COL));
        assert_eq!(
            cm.get(Channel::X),
            Some("delay"),
            "the bare channel names the field, so the axis has a title"
        );
        assert_eq!(
            cm.get(Channel::Y),
            Some(AGGREGATE_COUNT_COL),
            "the count half needs no new code — the aggregate arm already maps it"
        );

        // The transpose keys its high edge to the other axis, so the two
        // orientations cannot read each other's column.
        let mut transposed: indexmap::IndexMap<String, ValueOrParamRef<SpecValue>> =
            Default::default();
        transposed.insert("y".to_string(), options["x"].clone());
        transposed.insert("x".to_string(), options["y"].clone());
        let cm = ChannelMap::from_mark(&Mark {
            kind: MarkKind::RectX,
            status: ImplStatus::Implemented,
            data: None,
            options: transposed,
        });
        assert_eq!(cm.get(Channel::Y1), Some("delay"));
        assert_eq!(cm.get(Channel::Y2), Some(BIN_HI_Y_COL));
        assert_eq!(cm.get(Channel::X), Some(AGGREGATE_COUNT_COL));
        assert!(
            !cm.has(Channel::X1) && !cm.has(Channel::X2),
            "a rectX binds no x interval — its x is the counted value"
        );
    }
}
