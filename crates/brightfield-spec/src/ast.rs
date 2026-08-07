//! The typed Rust AST for a parsed Mosaic 0.24.x spec.
//!
//! ## Structure vs enum split
//!
//! - **Structs**: `Spec`, `Meta`, `Config`, `PlotDefaults`, `DataSource`,
//!   `ParamNode`, `SelectionNode`, `Mark`, `Interactor`, `Input`, `PlotNode`,
//!   `ExpressionNode`.
//! - **Sealed enums (composition level)**: `Component`, `ValueOrParamRef<T>`.
//!
//! Vocabulary is carried by the `*Kind` enums from [`crate::vocab`]; node
//! structs are parameterised by a `kind` field rather than being enums
//! themselves, so per-mark typed option upgrades in future cards can extend
//! the struct additively.

use indexmap::IndexMap;

use crate::vocab::{
    ComponentKind, ImplStatus, InputKind, InteractorKind, LegendChannel, MarkKind,
    SelectionResolution,
};

/// Root node: everything a Mosaic YAML/JSON spec may contain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spec {
    /// `meta:` — title, description, version.
    pub meta: Option<Meta>,
    /// `data:` — named data sources. Insertion-ordered.
    pub data: IndexMap<String, DataSource>,
    /// `params:` — named params (literal values or selection declarations).
    pub params: IndexMap<String, ParamNode>,
    /// `config:` — open bag of Mosaic-wide configuration keys.
    pub config: Config,
    /// `plotDefaults:` — open bag of plot-level defaults.
    pub plot_defaults: PlotDefaults,
    /// Root composition component (the spec's visible layout).
    /// `None` iff the spec is a bare `data+params` file with no visible root.
    pub root: Option<Component>,
}

/// Typed accessors for the `meta:` block: `title`, `description`, `version`.
///
/// **Note:** spec constraint #4e narrowed `deny_unknown_fields` to Meta only,
/// and corpus-totality evidence (20/54 specs declare `meta.credit`, one uses
/// `meta.descriptions`) narrowed it further: unknown meta keys are now
/// collected as [`crate::parse::ParseWarning::UnknownOption`] rather than
/// producing a fatal error. The typed fields below are the contract we keep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Meta {
    /// Human-readable title.
    pub title: Option<String>,
    /// Human-readable description.
    pub description: Option<String>,
    /// Declared Mosaic spec version (e.g. `0.24.0`). Compared against
    /// `SUPPORTED_MOSAIC_MAJOR_MINOR`.
    pub version: Option<String>,
}

/// `config:` — a newtype around an open bag. Spec constraint #4e narrowed
/// `deny_unknown_fields` to `Meta` only; `config:` carries Mosaic's
/// configuration library (e.g. `extensions: spatial`), not a
/// brightfield-owned schema. The wrapping struct keeps the "Config is a
/// struct" contract while the inner `IndexMap` admits the open shape.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config(pub IndexMap<String, SpecValue>);

impl Config {
    /// Construct an empty Config.
    #[must_use]
    pub fn new() -> Self {
        Self(IndexMap::new())
    }

    /// True if no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl std::ops::Deref for Config {
    type Target = IndexMap<String, SpecValue>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Config {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// `plotDefaults:` — newtype around an open bag of plot-level attributes
/// applied to every plot in the spec. Same reasoning as [`Config`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlotDefaults(pub IndexMap<String, SpecValue>);

impl PlotDefaults {
    /// Construct an empty PlotDefaults.
    #[must_use]
    pub fn new() -> Self {
        Self(IndexMap::new())
    }

    /// True if no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl std::ops::Deref for PlotDefaults {
    type Target = IndexMap<String, SpecValue>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for PlotDefaults {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// A named data source declared under `data:`.
#[derive(Debug, Clone, PartialEq)]
pub struct DataSource {
    /// The kind of source this is — see [`DataSourceKind`].
    pub kind: DataSourceKind,
    /// Extra keys beyond those covered by `kind` (e.g. `select`, `where`,
    /// `type`). Preserved verbatim so downstream query generation has what
    /// it needs without re-parsing the source.
    pub extras: IndexMap<String, SpecValue>,
}

/// The shape a `data:` entry takes on the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum DataSourceKind {
    /// Entry is a string shorthand (`"SELECT ..."` or a table name).
    Shorthand(String),
    /// Entry has a `file:` key — loads from a file URL or path.
    File(String),
    /// Entry has a `query:` or inline SQL — materialised via DuckDB.
    Query(String),
    /// Entry is an inline array of row objects (or array-of-arrays).
    InlineRows(Vec<SpecValue>),
    /// Entry uses a `type:`-discriminated shape we accept but don't further
    /// classify in v1.
    Typed(String),
    /// Entry is an object with none of the above discriminators — kept in
    /// `extras` for downstream handling.
    Opaque,
}

/// A named `params:` entry. Either a literal value or a selection declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamNode {
    /// A literal value param (`params: { foo: 42 }` or
    /// `params: { domain: ['a','b'] }`).
    Value(SpecValue),
    /// A selection declaration: `{ select: <resolution>, ... }`.
    Selection(SelectionNode),
}

/// A selection declaration inside `params:`.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionNode {
    /// The resolution type (`crossfilter | intersect | single | union`).
    /// All four are implemented at the SQL-emit layer; an unrecognised name is
    /// a hard `ParseError::UnknownName`, never a stub.
    pub select: SelectionResolution,
    /// Status of the resolution name in the vocabulary registry.
    pub status: ImplStatus,
    /// Extra options (`cross`, `empty`, `include`, etc.) preserved verbatim.
    pub options: IndexMap<String, SpecValue>,
}

/// A lifted structural reference to a param or selection. Produced at parse
/// time from `"$name"` string forms and `{param: name}` object forms at
/// positions on the lift surface.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParamRef(pub String);

impl ParamRef {
    /// Construct a new ParamRef from an identifier (without the leading `$`).
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Canonical wire form: `$<name>`.
    #[must_use]
    pub fn to_wire(&self) -> String {
        format!("${}", self.0)
    }
}

/// A field-value position that may carry either a literal value or a lifted
/// param reference. This is the outermost wrapper applied to mark options,
/// interactor options, input options, and selected attribute positions.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueOrParamRef<T> {
    /// A literal value (possibly containing nested `ParamRef`s via
    /// [`SpecValue::Param`]).
    Value(T),
    /// A structural param reference at the outer position.
    Param(ParamRef),
}

impl<T> ValueOrParamRef<T> {
    /// True if this value is a lifted `ParamRef`.
    #[must_use]
    pub fn is_param(&self) -> bool {
        matches!(self, Self::Param(_))
    }
}

/// A compositional component — the universal "thing that appears at the
/// root, inside a vconcat/hconcat, or as a plot item".
///
/// Sealed enum: every Mosaic composition form brightfield recognises appears
/// as a named variant. New forms require a registry edit.
#[derive(Debug, Clone, PartialEq)]
pub enum Component {
    /// A plot block (marks + interactors + legends with plot-level attrs).
    Plot(PlotNode),
    /// Horizontal layout of sub-components.
    HConcat(ConcatNode),
    /// Vertical layout of sub-components.
    VConcat(ConcatNode),
    /// Horizontal spacer.
    HSpace(SpaceNode),
    /// Vertical spacer.
    VSpace(SpaceNode),
    /// A standalone legend (external to any plot).
    Legend(LegendNode),
    /// A bare mark at the composition level.
    Mark(Mark),
    /// A bare interactor (rare; appears inside a plot normally).
    Interactor(Interactor),
    /// A standalone input widget.
    Input(Input),
}

/// A plot node — the `plot: [...]` component with sibling plot-level
/// attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct PlotNode {
    /// The items inside `plot:` (marks, interactors, legends, nested plots).
    pub items: Vec<Component>,
    /// Plot-level attributes (`xDomain`, `width`, `height`, `colorScheme`,
    /// `name`, etc.) carried alongside the `plot:` key.
    pub attributes: IndexMap<String, SpecValue>,
}

/// An hconcat/vconcat node.
#[derive(Debug, Clone, PartialEq)]
pub struct ConcatNode {
    /// Sub-components in source order.
    pub items: Vec<Component>,
}

/// An `hspace:` / `vspace:` component.
#[derive(Debug, Clone, PartialEq)]
pub struct SpaceNode {
    /// The spacer value (`1em`, `35`, etc.) as authored.
    pub value: SpecValue,
}

/// A standalone legend component (external to its plot, referencing by
/// `for: <plot-name>`).
#[derive(Debug, Clone, PartialEq)]
pub struct LegendNode {
    /// Which channel this legend displays (`color | opacity | symbol`).
    pub channel: LegendChannel,
    /// Status of the channel name in the registry.
    pub status: ImplStatus,
    /// All other fields (`label`, `for`, `as`, etc.).
    pub options: IndexMap<String, ValueOrParamRef<SpecValue>>,
}

/// A mark node. `options` is a flat bag per the staged typing decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    /// The mark kind.
    pub kind: MarkKind,
    /// Status of this mark in the vocabulary registry.
    pub status: ImplStatus,
    /// Data declaration (`data: { from: ..., filterBy: ... }` or inline).
    pub data: Option<MarkData>,
    /// All other channel/option slots. Each value may be a lifted ParamRef.
    pub options: IndexMap<String, ValueOrParamRef<SpecValue>>,
}

/// An interactor node.
#[derive(Debug, Clone, PartialEq)]
pub struct Interactor {
    /// The interactor kind.
    pub kind: InteractorKind,
    /// Status of this interactor in the registry.
    pub status: ImplStatus,
    /// All option slots. Each value may be a lifted ParamRef.
    pub options: IndexMap<String, ValueOrParamRef<SpecValue>>,
}

/// An input widget node.
#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    /// The input kind.
    pub kind: InputKind,
    /// Status of this input in the registry.
    pub status: ImplStatus,
    /// `as: $param` — the param this widget writes to.
    pub as_param: Option<ParamRef>,
    /// `from:` — the data source this widget reads from (e.g. menu options).
    pub from_source: Option<String>,
    /// `filterBy: $param` — a selection or param that filters this widget's data.
    pub filter_by: Option<ParamRef>,
    /// Remaining option slots (everything except `as`, `from`, `filterBy`).
    /// Each value may be a lifted ParamRef.
    pub options: IndexMap<String, ValueOrParamRef<SpecValue>>,
}

/// A mark-level data declaration. Either references a named source via
/// `from:`, or carries inline rows.
#[derive(Debug, Clone, PartialEq)]
pub enum MarkData {
    /// References a named `data:` entry by name.
    From {
        /// The data source name.
        source: String,
        /// `filterBy:` reference, lifted from `$param` at parse time.
        filter_by: Option<ParamRef>,
        /// Any other keys on the data object (`select`, `where`, etc.).
        extras: IndexMap<String, SpecValue>,
    },
    /// Inline array of row objects / arrays / values.
    Inline(Vec<SpecValue>),
}

/// A tokenised SQL expression: interleaved literal spans and param refs.
///
/// For input `"x > $lo AND x < $hi"` the result is
/// `spans = ["x > ", " AND x < ", ""]`, `params = [ParamRef("lo"), ParamRef("hi")]`.
/// Invariant: `spans.len() == params.len() + 1`.
///
/// Spans outside literal param positions are preserved verbatim; `$` inside
/// single-quoted strings, double-quoted identifiers, line comments, and
/// block comments is treated as literal text (see [`crate::expr`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExpressionNode {
    /// Literal text between (and flanking) param refs.
    pub spans: Vec<String>,
    /// Param refs, in source order.
    pub params: Vec<ParamRef>,
}

impl ExpressionNode {
    /// A plain span with no param refs.
    #[must_use]
    pub fn literal(s: impl Into<String>) -> Self {
        Self {
            spans: vec![s.into()],
            params: Vec::new(),
        }
    }

    /// True if no param refs were found during tokenisation.
    #[must_use]
    pub fn is_literal(&self) -> bool {
        self.params.is_empty()
    }

    /// Reconstruct the original string (round-trips exactly when the tokeniser
    /// used only the `$ident` recognition rule).
    #[must_use]
    pub fn to_wire(&self) -> String {
        let mut out = String::new();
        for i in 0..self.spans.len() {
            out.push_str(&self.spans[i]);
            if i < self.params.len() {
                out.push('$');
                out.push_str(&self.params[i].0);
            }
        }
        out
    }
}

/// A self-aggregating channel transform — the typed form of a channel value
/// like `fill: {count:}` or `fill: {avg: score_value}` (hexbin /
/// self-aggregating cell). Mosaic marks such as hexbin carry the aggregate on
/// the channel itself; the lowerer turns it into a `GROUP BY` with the matching
/// SQL aggregate. Distinct from a plain column/literal/param channel — parsing
/// lifts a recognised single-key aggregate map to this form so a downstream
/// lowerer can dispatch on it and the renderer reads the aggregated column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AggregateFunc {
    /// `{count:}` — row count per group (no source column).
    Count,
    /// `{sum: col}`.
    Sum,
    /// `{avg: col}` (also spelled `{mean: col}`).
    Avg,
    /// `{min: col}`.
    Min,
    /// `{max: col}`.
    Max,
}

impl AggregateFunc {
    /// Look up by wire key. `mean` is an accepted alias for `avg`. Returns
    /// `None` for names outside the recognised aggregate set (the caller warns).
    #[must_use]
    pub fn from_wire(wire: &str) -> Option<Self> {
        match wire {
            "count" => Some(Self::Count),
            "sum" => Some(Self::Sum),
            "avg" | "mean" => Some(Self::Avg),
            "min" => Some(Self::Min),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    /// Canonical wire key (as it re-serialises). `Avg` canonicalises to `avg`.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
        }
    }
}

/// A generic spec value — the building block of options bags and inline data.
///
/// This is analogous to `serde_json::Value` plus parser-lifted variants:
/// `Param` for `$param` references found at non-outermost positions,
/// `Expression` for tokenised SQL strings, and `Aggregate` for a
/// self-aggregating channel transform (`{count:}` / `{avg: col}`).
/// `ValueOrParamRef<SpecValue>` is used at outer option-slot positions.
#[derive(Debug, Clone, PartialEq)]
pub enum SpecValue {
    /// YAML/JSON null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Integer (fits in i64).
    Integer(i64),
    /// Floating-point.
    Float(f64),
    /// String.
    String(String),
    /// Array of values.
    Array(Vec<SpecValue>),
    /// Object (insertion-ordered map of values).
    Object(IndexMap<String, SpecValue>),
    /// Lifted `$param` reference at an interior position.
    Param(ParamRef),
    /// Tokenised SQL expression.
    Expression(ExpressionNode),
    /// A self-aggregating channel transform lifted from a recognised single-key
    /// aggregate map at a mark channel position (`{count:}`, `{avg: col}`).
    /// `column` is `None` for `count` (no source column) and `Some(col)` for
    /// the column-taking aggregates.
    Aggregate {
        /// The aggregate function.
        func: AggregateFunc,
        /// The source column (`None` for `count`).
        column: Option<String>,
    },
    /// A **positional** bin transform, lifted from `x: {bin: col}` /
    /// `y: {bin: col, steps: n}` on a rect-family mark whose OTHER positional
    /// channel counts.
    ///
    /// Deliberately a separate variant rather than a widening of
    /// [`Self::Aggregate`]'s channel list: a bin is an interval and an
    /// aggregate is a scalar, and each reaches a positional channel through
    /// its OWN pair resolver (`parse::binned_histogram`,
    /// `parse::banded_aggregate`) rather than through
    /// `AGGREGATE_CHANNEL_FIELDS`, which stays `fill`/`r`. So neither lift can
    /// quieten the other's diagnostic: `seattle-temp.yaml`'s `y1: {max: …}` is
    /// on an interval channel no lowerer computes and keeps warning, while
    /// `sorted-bars.yaml`'s `x: {sum: gold}` over `y: nationality` is the band
    /// idiom `BarLowerer` groups.
    Bin {
        /// The source column to bin.
        column: String,
        /// Mosaic's `steps:` bin-count **hint**, when the spec gave one. It
        /// selects the 1/2/5 × 10ⁿ step; it does not fix the bar count — see
        /// `binSpec` in `mosaic-sql`'s `transforms/util/bin-step.ts`.
        steps: Option<i64>,
    },
    /// A mark's **`sort:`** option, lifted from `sort: {y: -x, limit: 10}` on
    /// a kind that has a band axis — the ranked half of a ranked category bar.
    ///
    /// Lifted by the private `mark_sort` resolver in [`crate::parse`] and read
    /// by `brightfield-sql`'s `BarLowerer`, which turns it into an `ORDER BY`
    /// on the value channel's output column and a `LIMIT`.
    ///
    /// `channel` and `by` are held as written rather than re-derived from the
    /// mark kind, because serialisation sees a [`SpecValue`] and no kind: they
    /// are what lets `{y: -x}` be rebuilt from the lift, so parse → serialise
    /// → parse is idempotent.
    Sort {
        /// The channel whose order this sets — the mark's band axis (`y` on a
        /// `barX`).
        channel: String,
        /// The channel the order is read from — the mark's value axis (`x` on
        /// a `barX`).
        by: String,
        /// Whether the spec wrote the `-` prefix.
        descending: bool,
        /// `limit:`, when the spec wrote one.
        limit: Option<u64>,
    },
}

impl SpecValue {
    /// If this value is a string, return it.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// If this value is an object, return its map.
    #[must_use]
    pub fn as_object(&self) -> Option<&IndexMap<String, SpecValue>> {
        match self {
            Self::Object(m) => Some(m),
            _ => None,
        }
    }

    /// If this value is an array, return its items.
    #[must_use]
    pub fn as_array(&self) -> Option<&[SpecValue]> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
}

// Re-exports so downstream code that imports only from crate::ast doesn't
// also need to reach into crate::vocab.
pub use crate::vocab::{
    ComponentKind as ComponentKindRef, InputKind as InputKindRef,
    InteractorKind as InteractorKindRef, LegendChannel as LegendChannelRef,
    MarkKind as MarkKindRef, SelectionResolution as SelectionResolutionRef,
};

// Unused imports are unused; silence the trivial warning deterministically.
#[allow(unused_imports)]
use self::{
    ComponentKind as _, InputKind as _, InteractorKind as _, LegendChannel as _, MarkKind as _,
    SelectionResolution as _,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// verification: `Component` is a sealed enum with no `Other`
    /// variant. Construction of each named variant compiles; no catch-all is
    /// available. The match below is exhaustive — adding a new variant
    /// without naming it here fails compilation.
    #[test]
    fn component_enum_is_sealed() {
        fn discriminator(c: &Component) -> &'static str {
            match c {
                Component::Plot(_) => "plot",
                Component::HConcat(_) => "hconcat",
                Component::VConcat(_) => "vconcat",
                Component::HSpace(_) => "hspace",
                Component::VSpace(_) => "vspace",
                Component::Legend(_) => "legend",
                Component::Mark(_) => "mark",
                Component::Interactor(_) => "interactor",
                Component::Input(_) => "input",
            }
        }
        let plot = Component::Plot(PlotNode {
            items: vec![],
            attributes: IndexMap::new(),
        });
        assert_eq!(discriminator(&plot), "plot");
    }

    /// verification: Mark / Interactor / Input are *structs* keyed by
    /// their respective `*Kind` enums — constructable with named fields here
    /// which compiles only if they are structs, not enums.
    #[test]
    fn mark_interactor_input_are_structs() {
        let _m = Mark {
            kind: MarkKind::Line,
            status: ImplStatus::Unimplemented,
            data: None,
            options: IndexMap::new(),
        };
        let _i = Interactor {
            kind: InteractorKind::IntervalX,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let _u = Input {
            kind: InputKind::Table,
            status: ImplStatus::Unimplemented,
            as_param: None,
            from_source: None,
            filter_by: None,
            options: IndexMap::new(),
        };
    }

    #[test]
    fn paramref_to_wire() {
        assert_eq!(ParamRef::new("brush").to_wire(), "$brush");
    }
}
