//! QueryPlan intermediate representation for DuckDB SQL emission.
//!
//! The IR is a typed tree that mirrors DuckDB's grammar. Each variant maps
//! directly to a SQL clause. The tree is the substrate for optimisation passes
//! (`passes.rs`) and the rendering step (`render.rs`).

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};

/// Resolution strategy for multi-view selections.
///
/// Independent from `brightfield_spec::ast::SelectionResolution` so the IR is
/// not coupled to AST serde changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionResolution {
    Crossfilter,
    Intersect,
    Union,
    Single,
}

impl From<brightfield_spec::vocab::SelectionResolution> for SelectionResolution {
    fn from(ast: brightfield_spec::vocab::SelectionResolution) -> Self {
        match ast {
            brightfield_spec::vocab::SelectionResolution::Crossfilter => Self::Crossfilter,
            brightfield_spec::vocab::SelectionResolution::Intersect => Self::Intersect,
            brightfield_spec::vocab::SelectionResolution::Union => Self::Union,
            brightfield_spec::vocab::SelectionResolution::Single => Self::Single,
        }
    }
}

/// One selection's compiled predicates: the selection's name, paired with the
/// predicates it contributes per source table (`(table, predicate)`).
///
/// Named because the bare tuple appears in a dozen signatures across
/// `brightfield-sql` and `brightfield-engine`, where it read as noise rather
/// than as "the selection filters to apply".
pub type SelectionPredicate = (String, Vec<(String, Predicate)>);

/// A typed SQL scalar literal carried by the structured clause variants
/// ([`Predicate::Interval`] / [`Predicate::Point`]).
///
/// [`ScalarValue::to_sql_literal`] produces exactly the literal text the
/// string-predicate path produces today (bare `Display` numbers, single-quoted
/// strings with embedded quotes doubled, `make_timestamp(us)` timestamps), so
/// a structured clause and its hand-written string form render byte-identical
/// SQL.
///
/// `PartialEq`/`Eq`/`Hash` are hand-written because of the `f64` payload:
/// floats compare and hash by bit pattern (`f64::to_bits`), which keeps the
/// derived `Eq + Hash` on [`QueryPlan`] sound (reflexive even for NaN; `0.0`
/// and `-0.0` are distinct, matching their distinct SQL literals).
#[derive(Debug, Clone)]
pub enum ScalarValue {
    /// A floating-point value — bare literal via Rust `Display` (`10`, `0.5`).
    Float(f64),
    /// An exact integer value — bare literal.
    Int(i64),
    /// A string / categorical value — single-quoted, embedded quotes doubled.
    Text(String),
    /// A naive-`TIMESTAMP` microsecond epoch — `make_timestamp(us)`.
    TimestampMicros(i64),
    /// A `TIMESTAMPTZ` UTC microsecond epoch —
    /// `make_timestamp(us) AT TIME ZONE 'UTC'`.
    TimestampTzMicros(i64),
}

impl ScalarValue {
    /// Format as a SQL literal — the exact text the string-predicate path
    /// interpolates into its expression strings.
    #[must_use]
    pub fn to_sql_literal(&self) -> String {
        match self {
            Self::Float(n) => n.to_string(),
            Self::Int(i) => i.to_string(),
            Self::Text(s) => format!("'{}'", s.replace('\'', "''")),
            Self::TimestampMicros(us) => format!("make_timestamp({us})"),
            Self::TimestampTzMicros(us) => format!("make_timestamp({us}) AT TIME ZONE 'UTC'"),
        }
    }
}

impl PartialEq for ScalarValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Float(a), Self::Float(b)) => a.to_bits() == b.to_bits(),
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::TimestampMicros(a), Self::TimestampMicros(b)) => a == b,
            (Self::TimestampTzMicros(a), Self::TimestampTzMicros(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ScalarValue {}

impl Hash for ScalarValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Float(n) => n.to_bits().hash(state),
            Self::Int(i) | Self::TimestampMicros(i) | Self::TimestampTzMicros(i) => i.hash(state),
            Self::Text(s) => s.hash(state),
        }
    }
}

/// The scale context a structured clause was produced under — how the brushed
/// axis maps data to pixels. Captured so a downstream consumer can derive
/// pixel-resolution binned columns from the clause; SQL emission ignores it
/// entirely.
#[derive(Debug, Clone)]
pub struct ScaleDescriptor {
    /// Scale transform kind — e.g. `"linear"`, `"band"`, `"time"`.
    pub kind: String,
    /// Data-space domain `[lo, hi]` of the axis, when continuous
    /// (a time axis carries its microsecond epochs as `f64`).
    pub domain: Option<(f64, f64)>,
    /// Pixel-space range `[lo, hi]` of the axis.
    pub range: Option<(f64, f64)>,
}

/// Bit-pattern equality for the `f64` pairs (`Eq`-sound; see [`ScalarValue`]).
fn pair_bits(pair: &Option<(f64, f64)>) -> Option<(u64, u64)> {
    pair.map(|(a, b)| (a.to_bits(), b.to_bits()))
}

impl PartialEq for ScaleDescriptor {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && pair_bits(&self.domain) == pair_bits(&other.domain)
            && pair_bits(&self.range) == pair_bits(&other.range)
    }
}

impl Eq for ScaleDescriptor {}

impl Hash for ScaleDescriptor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        pair_bits(&self.domain).hash(state);
        pair_bits(&self.range).hash(state);
    }
}

/// Optional scale/pixel metadata on a structured clause. Optional end to end:
/// a producer that cannot supply it yet passes `None` and the clause is no
/// less valid — data-unit consumers work without it; pixel-resolution
/// consumers check for it and degrade gracefully.
#[derive(Debug, Clone, Default)]
pub struct ClauseMeta {
    /// The scale that mapped the gesture's pixels to this clause's data bounds.
    pub scale: Option<ScaleDescriptor>,
    /// The interactive-pixel granularity of the selection along its axis — how
    /// many screen pixels one selection step spans (`1.0` = per-pixel).
    pub pixel_size: Option<f64>,
}

impl PartialEq for ClauseMeta {
    fn eq(&self, other: &Self) -> bool {
        self.scale == other.scale
            && self.pixel_size.map(f64::to_bits) == other.pixel_size.map(f64::to_bits)
    }
}

impl Eq for ClauseMeta {}

impl Hash for ClauseMeta {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.scale.hash(state);
        self.pixel_size.map(f64::to_bits).hash(state);
    }
}

/// A predicate in the IR — used in `Filter` and selection compilation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Predicate {
    /// A raw SQL expression string (from AST, trusted content).
    Expr(String),
    /// A parameterised placeholder — renders as `?` in prepared mode.
    Param {
        name: String,
        /// Position of this `?` in the overall statement (0-indexed).
        placeholder_index: usize,
    },
    /// Conjunction — `AND` of sub-predicates. Empty → `TRUE`.
    And(Vec<Predicate>),
    /// Disjunction — `OR` of sub-predicates. Empty → `FALSE`.
    Or(Vec<Predicate>),
    /// Literal `TRUE`.
    True,
    /// Literal `FALSE`.
    False,
    /// A structured interval clause — `column` within `[lo, hi]`, both bounds
    /// inclusive. Renders exactly as its hand-written string form
    /// `And([Expr("col >= lo"), Expr("col <= hi")])` renders —
    /// `(col >= lo AND col <= hi)` — so substituting one for the other
    /// anywhere in a predicate tree emits byte-identical SQL. Unlike that
    /// string form, the column, bounds, and scale/pixel context stay
    /// machine-readable for downstream analysis. `column` is a trusted SQL
    /// expression, exactly like `Expr`.
    Interval {
        /// The column expression the interval constrains.
        column: String,
        /// Inclusive lower bound.
        lo: ScalarValue,
        /// Inclusive upper bound.
        hi: ScalarValue,
        /// Scale/pixel context, when the producer has it (optional end to end).
        meta: Option<ClauseMeta>,
    },
    /// A structured point/membership clause — `column` equal to one of
    /// `values`. Renders exactly as its hand-written string forms render: one
    /// value as `col = v` (a bare `Expr`), several as `(col = v1 OR col = v2)`
    /// (an `Or` of equalities), none as `FALSE` (the empty `Or`).
    Point {
        /// The column expression the membership tests.
        column: String,
        /// The selected values (equality members).
        values: Vec<ScalarValue>,
        /// Scale/pixel context, when the producer has it (optional end to end).
        meta: Option<ClauseMeta>,
    },
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Expr(s) => f.write_str(s),
            Self::Param { .. } => f.write_str("?"),
            Self::And(preds) => {
                if preds.is_empty() {
                    return f.write_str("TRUE");
                }
                let parts: Vec<String> = preds.iter().map(|p| format!("{p}")).collect();
                write!(f, "({})", parts.join(" AND "))
            }
            Self::Or(preds) => {
                if preds.is_empty() {
                    return f.write_str("FALSE");
                }
                let parts: Vec<String> = preds.iter().map(|p| format!("{p}")).collect();
                write!(f, "({})", parts.join(" OR "))
            }
            Self::True => f.write_str("TRUE"),
            Self::False => f.write_str("FALSE"),
            Self::Interval { column, lo, hi, .. } => write!(
                f,
                "({column} >= {} AND {column} <= {})",
                lo.to_sql_literal(),
                hi.to_sql_literal()
            ),
            Self::Point { column, values, .. } => match values.as_slice() {
                [] => f.write_str("FALSE"),
                [v] => write!(f, "{column} = {}", v.to_sql_literal()),
                vs => {
                    let parts: Vec<String> = vs
                        .iter()
                        .map(|v| format!("{column} = {}", v.to_sql_literal()))
                        .collect();
                    write!(f, "({})", parts.join(" OR "))
                }
            },
        }
    }
}

/// Sort direction for `Order` clauses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SortDir {
    Asc,
    Desc,
}

/// An aggregate function the mark lowerers emit as a typed call.
///
/// The variant set is scoped to what the lowerers actually produce today —
/// `COUNT`/`sum`/`avg`/`min`/`max` plus the linear-regression
/// sufficient-statistics family — and grows with the lowerers. A function
/// outside this set stays a raw string ([`AggregateExpr::Raw`]).
///
/// [`Self::sql_name`] pins the exact historical spelling of each function
/// (`COUNT` uppercase, everything else lowercase), so a typed call renders
/// byte-identically to the string the lowerers used to format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AggregateFunction {
    /// `COUNT` — `args == ["*"]` is count-star (row count); a column argument
    /// counts non-NULL values of that column.
    Count,
    Sum,
    Avg,
    Min,
    Max,
    /// `regr_slope(y, x)` — slope of the least-squares fit.
    RegrSlope,
    /// `regr_intercept(y, x)` — intercept of the least-squares fit.
    RegrIntercept,
    /// `regr_count(y, x)` — rows where both arguments are non-NULL.
    RegrCount,
    /// `regr_avgx(y, x)` — mean of the independent variable.
    RegrAvgx,
    /// `regr_sxx(y, x)` — sum of squared deviations of x.
    RegrSxx,
    /// `regr_sxy(y, x)` — sum of co-deviations of x and y.
    RegrSxy,
    /// `regr_syy(y, x)` — sum of squared deviations of y.
    RegrSyy,
}

impl AggregateFunction {
    /// The exact SQL spelling the string-formatting path has always emitted.
    /// `COUNT` is uppercase (the historical density/hexbin/cell form); every
    /// other function is lowercase. Changing a spelling here changes emitted
    /// SQL bytes — don't.
    #[must_use]
    pub fn sql_name(&self) -> &'static str {
        match self {
            Self::Count => "COUNT",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
            Self::RegrSlope => "regr_slope",
            Self::RegrIntercept => "regr_intercept",
            Self::RegrCount => "regr_count",
            Self::RegrAvgx => "regr_avgx",
            Self::RegrSxx => "regr_sxx",
            Self::RegrSxy => "regr_sxy",
            Self::RegrSyy => "regr_syy",
        }
    }
}

/// A typed aggregate call — function identity and argument expressions carried
/// as data, not as unparsed text.
///
/// # The decomposition contract
///
/// This type exists so a later automatic pre-aggregation pass can decompose an
/// aggregate into sufficient statistics over the cells of a derived cube
/// (global `sum` = sum of per-cell sums; `avg` = summed sums over summed
/// counts; `min`/`max` = min/max of per-cell extrema; the `regr_*` family from
/// per-cell moment sums). What that pass needs, and what this type guarantees:
///
/// - **`func` is an enum**, so decomposability is a `match`, not a string
///   parse.
/// - **`args` are individually addressable expressions** (each element is
///   trusted raw SQL, exactly like [`Predicate::Expr`]; `["*"]` is
///   count-star), so the derivation can re-aggregate the same argument
///   expression at cube-cell grain.
/// - **`filter`**, when present, is the predicate text of a
///   `FILTER (WHERE …)` clause and must propagate onto *every* statistic the
///   aggregate decomposes into.
/// - **`cast` / `alias` are carried, not interpreted** — they reproduce
///   today's emission byte-exactly ([`Display`](fmt::Display) renders
///   `CAST(func(args) FILTER (WHERE …) AS cast) AS alias`, omitting the
///   absent parts) and the output column keeps its name across a rewrite.
///
/// An expression that does not fit this shape stays an
/// [`AggregateExpr::Raw`] string: opaque to analysis, which simply bails
/// derivation for that query and falls back to the direct query — the
/// designed fallback, not a failure.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AggregateCall {
    /// The aggregate function.
    pub func: AggregateFunction,
    /// Argument expressions, in call order. Each is a trusted SQL expression
    /// (typically a quoted column like `"delay"`); `["*"]` is count-star.
    pub args: Vec<String>,
    /// `FILTER (WHERE …)` predicate text, when present. No lowerer emits one
    /// today; the field is carried for derivation, which must propagate it
    /// onto every decomposed statistic.
    pub filter: Option<String>,
    /// `CAST(… AS <this>)` wrapped around the call (e.g. `DOUBLE`), when
    /// present.
    pub cast: Option<String>,
    /// Output alias — the exact text after ` AS `, including any quoting
    /// (`__bf_count`, `"score"`, `slope`).
    pub alias: Option<String>,
}

impl fmt::Display for AggregateCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut call = format!("{}({})", self.func.sql_name(), self.args.join(", "));
        if let Some(pred) = &self.filter {
            call = format!("{call} FILTER (WHERE {pred})");
        }
        if let Some(ty) = &self.cast {
            call = format!("CAST({call} AS {ty})");
        }
        f.write_str(&call)?;
        if let Some(alias) = &self.alias {
            write!(f, " AS {alias}")?;
        }
        Ok(())
    }
}

/// One aggregate output expression in an [`QueryPlan::Aggregation`] /
/// [`QueryPlan::AggregateScalar`] — either a typed, analyzable call or the
/// raw-string escape hatch.
///
/// The two forms render identically wherever both can express the same SQL
/// (see [`AggregateCall`]'s `Display`), so migrating a lowerer from `Raw` to
/// `Call` never changes emitted bytes — only what downstream analysis can see.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AggregateExpr {
    /// A raw SQL expression string, rendered verbatim. The escape hatch for
    /// anything unanalyzable (or not yet analyzed) — e.g. hexbin's
    /// constant-per-group geometry columns, which are scalar expressions, not
    /// aggregate calls. Derivation treats `Raw` as opaque and bails to the
    /// direct query.
    Raw(String),
    /// A typed aggregate call — see [`AggregateCall`] for the contract.
    Call(AggregateCall),
}

impl fmt::Display for AggregateExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Raw(s) => f.write_str(s),
            Self::Call(call) => write!(f, "{call}"),
        }
    }
}

impl From<String> for AggregateExpr {
    fn from(raw: String) -> Self {
        Self::Raw(raw)
    }
}

impl From<&str> for AggregateExpr {
    fn from(raw: &str) -> Self {
        Self::Raw(raw.to_string())
    }
}

/// A typed intermediate representation for DuckDB query emission.
///
/// Variants compose via `Box<QueryPlan>` nesting — a `Filter` wraps its
/// `input` source, a `Projection` wraps its input, etc.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum QueryPlan {
    /// Leaf node referencing a named view (from the DDL).
    Source { table: String },

    /// A constant single-row select with no `FROM` — the minimal named
    /// dataless-mark pathway (hexgrid). Renders `SELECT <columns>`, yielding one
    /// row so a decorative mark that draws from the plot extent (not data) still
    /// produces a batch and is not skipped downstream.
    Singleton {
        /// Column expressions (e.g. `"1 AS __bf_hexgrid"`).
        columns: Vec<String>,
    },

    /// `WHERE` clause with a predicate tree.
    Filter {
        input: Box<QueryPlan>,
        predicate: Predicate,
    },

    /// `SELECT` column list.
    Projection {
        input: Box<QueryPlan>,
        /// Column expressions (e.g. `"x"`, `"SUM(y)"`, `"*"`).
        columns: Vec<String>,
    },

    /// `GROUP BY` with aggregate expressions.
    Aggregation {
        input: Box<QueryPlan>,
        /// Group-by column expressions.
        group_by: Vec<String>,
        /// Aggregate output expressions — typed calls where the lowerer's
        /// output is analyzable ([`AggregateExpr::Call`]), raw strings
        /// otherwise ([`AggregateExpr::Raw`], e.g. `"SUM(y) AS total"`).
        aggregates: Vec<AggregateExpr>,
    },

    /// Scalar aggregation — produces a single row, no `GROUP BY`.
    ///
    /// Renders as `SELECT <aggregates> FROM (<input>)`. Used for
    /// regression statistics (regr_slope, regr_intercept, ...) where
    /// the output is a single row of summary statistics.
    AggregateScalar {
        input: Box<QueryPlan>,
        /// Aggregate output expressions — same convention as the
        /// [`QueryPlan::Aggregation`] `aggregates` field (e.g. a typed
        /// `regr_slope("y", "x") AS slope` call).
        aggregates: Vec<AggregateExpr>,
    },

    /// Binning via `width_bucket()` or `CASE` expression.
    Bin {
        input: Box<QueryPlan>,
        /// The column to bin.
        column: String,
        /// Bin width (or bucket count) as a SQL-safe string representation.
        /// Stored as string to preserve `Eq + Hash` on `QueryPlan`.
        width: String,
        /// Alias for the binned column.
        alias: String,
    },

    /// `ORDER BY` clause.
    Order {
        input: Box<QueryPlan>,
        /// (column_expr, direction) pairs.
        keys: Vec<(String, SortDir)>,
    },

    /// `LIMIT` / `OFFSET` clause.
    Limit {
        input: Box<QueryPlan>,
        limit: usize,
        offset: Option<usize>,
    },
}

impl QueryPlan {
    /// Compute a structural hash that excludes bound parameter values.
    ///
    /// `?` placeholder positions are included in the hash; their current
    /// runtime values are not. Two plans differing only in scalar param
    /// values produce identical structural hashes.
    pub fn hash_structural(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_variant_constructs() {
        let plan = QueryPlan::Source {
            table: "flights".to_string(),
        };
        assert!(format!("{plan:?}").contains("flights"));
    }

    #[test]
    fn filter_variant_constructs() {
        let plan = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            predicate: Predicate::Expr("x > 1".to_string()),
        };
        assert!(format!("{plan:?}").contains("Filter"));
    }

    #[test]
    fn all_variants_construct() {
        let src = QueryPlan::Source {
            table: "t".to_string(),
        };
        let _filter = QueryPlan::Filter {
            input: Box::new(src.clone()),
            predicate: Predicate::True,
        };
        let _proj = QueryPlan::Projection {
            input: Box::new(src.clone()),
            columns: vec!["x".to_string()],
        };
        let _agg = QueryPlan::Aggregation {
            input: Box::new(src.clone()),
            group_by: vec!["x".to_string()],
            aggregates: vec![AggregateExpr::Raw("SUM(y)".to_string())],
        };
        let _bin = QueryPlan::Bin {
            input: Box::new(src.clone()),
            column: "x".to_string(),
            width: "10".to_string(),
            alias: "x_bin".to_string(),
        };
        let _order = QueryPlan::Order {
            input: Box::new(src.clone()),
            keys: vec![("x".to_string(), SortDir::Asc)],
        };
        let _limit = QueryPlan::Limit {
            input: Box::new(src),
            limit: 100,
            offset: Some(10),
        };
    }

    #[test]
    fn hash_stability() {
        let plan_a = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            predicate: Predicate::Expr("x > 1".to_string()),
        };
        let plan_b = plan_a.clone();
        assert_eq!(plan_a.hash_structural(), plan_b.hash_structural());
    }

    #[test]
    fn selection_resolution_from_ast() {
        use brightfield_spec::vocab::SelectionResolution as AstRes;
        assert_eq!(
            SelectionResolution::from(AstRes::Crossfilter),
            SelectionResolution::Crossfilter
        );
        assert_eq!(
            SelectionResolution::from(AstRes::Union),
            SelectionResolution::Union
        );
    }

    #[test]
    fn predicate_display_expr() {
        let p = Predicate::Expr("x > 1".to_string());
        assert_eq!(format!("{p}"), "x > 1");
    }

    #[test]
    fn predicate_display_param() {
        let p = Predicate::Param {
            name: "lo".to_string(),
            placeholder_index: 0,
        };
        assert_eq!(format!("{p}"), "?");
    }

    #[test]
    fn predicate_display_and() {
        let p = Predicate::And(vec![
            Predicate::Expr("x > 1".to_string()),
            Predicate::Expr("x < 10".to_string()),
        ]);
        assert_eq!(format!("{p}"), "(x > 1 AND x < 10)");
    }

    #[test]
    fn predicate_display_or() {
        let p = Predicate::Or(vec![
            Predicate::Expr("a = 1".to_string()),
            Predicate::Expr("b = 2".to_string()),
        ]);
        assert_eq!(format!("{p}"), "(a = 1 OR b = 2)");
    }

    #[test]
    fn predicate_empty_and_is_true() {
        assert_eq!(format!("{}", Predicate::And(vec![])), "TRUE");
    }

    #[test]
    fn predicate_empty_or_is_false() {
        assert_eq!(format!("{}", Predicate::Or(vec![])), "FALSE");
    }

    #[test]
    fn hash_structural_same_structure_same_hash() {
        let plan_a = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            predicate: Predicate::Param {
                name: "lo".to_string(),
                placeholder_index: 0,
            },
        };
        // Same structure, same param name/position — should hash identically
        let plan_b = plan_a.clone();
        assert_eq!(plan_a.hash_structural(), plan_b.hash_structural());
    }

    #[test]
    fn hash_structural_different_structure_different_hash() {
        let plan_a = QueryPlan::Source {
            table: "t".to_string(),
        };
        let plan_b = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            predicate: Predicate::True,
        };
        assert_ne!(plan_a.hash_structural(), plan_b.hash_structural());
    }

    // --- structured clause variants (Interval / Point) ---

    fn hash_of(p: &Predicate) -> u64 {
        let mut h = DefaultHasher::new();
        p.hash(&mut h);
        h.finish()
    }

    /// Every ScalarValue variant formats the exact literal text the
    /// string-predicate path interpolates (bare Display numbers, quoted +
    /// doubled strings, make_timestamp forms).
    #[test]
    fn scalar_value_literals_match_string_path_formatting() {
        assert_eq!(ScalarValue::Float(10.0).to_sql_literal(), "10");
        assert_eq!(ScalarValue::Float(0.5).to_sql_literal(), "0.5");
        assert_eq!(ScalarValue::Int(42).to_sql_literal(), "42");
        assert_eq!(
            ScalarValue::Text("O'Hara".to_string()).to_sql_literal(),
            "'O''Hara'"
        );
        assert_eq!(
            ScalarValue::TimestampMicros(1_700_000_000_000_000).to_sql_literal(),
            "make_timestamp(1700000000000000)"
        );
        assert_eq!(
            ScalarValue::TimestampTzMicros(1_700_000_000_000_000).to_sql_literal(),
            "make_timestamp(1700000000000000) AT TIME ZONE 'UTC'"
        );
    }

    /// Float equality/hashing is by bit pattern: reflexive for NaN (Eq-sound
    /// inside the derived Eq on QueryPlan) and distinguishing 0.0 from -0.0
    /// (their SQL literals differ).
    #[test]
    fn scalar_value_float_bitwise_eq_and_hash() {
        let nan_a = ScalarValue::Float(f64::NAN);
        let nan_b = ScalarValue::Float(f64::NAN);
        assert_eq!(nan_a, nan_b, "NaN == NaN by bit pattern (Eq reflexivity)");
        assert_ne!(
            ScalarValue::Float(0.0),
            ScalarValue::Float(-0.0),
            "0.0 and -0.0 are distinct values with distinct literals"
        );
        assert_ne!(
            ScalarValue::Int(5),
            ScalarValue::TimestampMicros(5),
            "same payload, different variant: not equal"
        );
    }

    /// Display of a structured Interval is byte-identical to the Display of
    /// its hand-written string form (the two-clause And).
    #[test]
    fn interval_display_matches_string_form() {
        let structured = Predicate::Interval {
            column: "speed".to_string(),
            lo: ScalarValue::Float(10.0),
            hi: ScalarValue::Float(90.0),
            meta: None,
        };
        let string_form = Predicate::And(vec![
            Predicate::Expr("speed >= 10".to_string()),
            Predicate::Expr("speed <= 90".to_string()),
        ]);
        assert_eq!(format!("{structured}"), format!("{string_form}"));
        assert_eq!(format!("{structured}"), "(speed >= 10 AND speed <= 90)");
    }

    /// Display of a structured Point matches its string forms across all three
    /// cardinalities: none → FALSE (empty Or), one → the bare equality Expr,
    /// several → the Or of equalities.
    #[test]
    fn point_display_matches_string_forms() {
        let none = Predicate::Point {
            column: "sport".to_string(),
            values: vec![],
            meta: None,
        };
        assert_eq!(format!("{none}"), format!("{}", Predicate::Or(vec![])));

        let one = Predicate::Point {
            column: "sport".to_string(),
            values: vec![ScalarValue::Text("Athletics".to_string())],
            meta: None,
        };
        assert_eq!(
            format!("{one}"),
            format!("{}", Predicate::Expr("sport = 'Athletics'".to_string()))
        );

        let many = Predicate::Point {
            column: "sport".to_string(),
            values: vec![
                ScalarValue::Text("Athletics".to_string()),
                ScalarValue::Text("Rowing".to_string()),
            ],
            meta: None,
        };
        let string_form = Predicate::Or(vec![
            Predicate::Expr("sport = 'Athletics'".to_string()),
            Predicate::Expr("sport = 'Rowing'".to_string()),
        ]);
        assert_eq!(format!("{many}"), format!("{string_form}"));
        assert_eq!(
            format!("{many}"),
            "(sport = 'Athletics' OR sport = 'Rowing')"
        );
    }

    /// Metadata is carried (participating in Eq/Hash — two clauses differing
    /// only in scale context are different values) but never rendered.
    #[test]
    fn clause_meta_participates_in_identity_but_not_display() {
        let meta = ClauseMeta {
            scale: Some(ScaleDescriptor {
                kind: "linear".to_string(),
                domain: Some((0.0, 100.0)),
                range: Some((40.0, 340.0)),
            }),
            pixel_size: Some(1.0),
        };
        let with_meta = Predicate::Interval {
            column: "x".to_string(),
            lo: ScalarValue::Float(3.0),
            hi: ScalarValue::Float(7.0),
            meta: Some(meta),
        };
        let without_meta = Predicate::Interval {
            column: "x".to_string(),
            lo: ScalarValue::Float(3.0),
            hi: ScalarValue::Float(7.0),
            meta: None,
        };
        assert_ne!(with_meta, without_meta, "meta is part of the clause value");
        assert_ne!(hash_of(&with_meta), hash_of(&without_meta));
        assert_eq!(
            format!("{with_meta}"),
            format!("{without_meta}"),
            "meta never leaks into the rendered SQL"
        );
        assert_eq!(
            with_meta,
            with_meta.clone(),
            "clone preserves meta identity"
        );
    }

    // --- typed aggregates (AggregateExpr / AggregateCall) ---

    fn call(
        func: AggregateFunction,
        args: &[&str],
        cast: Option<&str>,
        alias: Option<&str>,
    ) -> AggregateCall {
        AggregateCall {
            func,
            args: args.iter().map(ToString::to_string).collect(),
            filter: None,
            cast: cast.map(ToString::to_string),
            alias: alias.map(ToString::to_string),
        }
    }

    /// Every typed-call shape a lowerer emits today renders byte-identically
    /// to the string it used to format — the byte-stability contract at the
    /// single-expression level.
    #[test]
    fn aggregate_call_renders_historical_strings() {
        // density / hexbin / cell count-star.
        assert_eq!(
            call(
                AggregateFunction::Count,
                &["*"],
                Some("DOUBLE"),
                Some("__bf_count")
            )
            .to_string(),
            "CAST(COUNT(*) AS DOUBLE) AS __bf_count"
        );
        // hexbin / cell column aggregate aliased to its source column.
        assert_eq!(
            call(
                AggregateFunction::Avg,
                &["\"score\""],
                Some("DOUBLE"),
                Some("\"score\"")
            )
            .to_string(),
            "CAST(avg(\"score\") AS DOUBLE) AS \"score\""
        );
        // regression sufficient statistics (two-argument family, no cast).
        assert_eq!(
            call(
                AggregateFunction::RegrSlope,
                &["\"height\"", "\"weight\""],
                None,
                Some("slope")
            )
            .to_string(),
            "regr_slope(\"height\", \"weight\") AS slope"
        );
        // regression data extents (cast, plain alias).
        assert_eq!(
            call(
                AggregateFunction::Min,
                &["\"weight\""],
                Some("DOUBLE"),
                Some("x_min")
            )
            .to_string(),
            "CAST(min(\"weight\") AS DOUBLE) AS x_min"
        );
        // bare call: no cast, no alias.
        assert_eq!(
            call(AggregateFunction::Sum, &["\"y\""], None, None).to_string(),
            "sum(\"y\")"
        );
    }

    /// A FILTER clause renders inside the CAST (it attaches to the aggregate
    /// call itself) and before the alias.
    #[test]
    fn aggregate_call_renders_filter_clause() {
        let mut c = call(
            AggregateFunction::Sum,
            &["\"y\""],
            Some("DOUBLE"),
            Some("total"),
        );
        c.filter = Some("\"y\" > 0".to_string());
        assert_eq!(
            c.to_string(),
            "CAST(sum(\"y\") FILTER (WHERE \"y\" > 0) AS DOUBLE) AS total"
        );
        let mut bare = call(AggregateFunction::Count, &["*"], None, Some("n"));
        bare.filter = Some("\"ok\"".to_string());
        assert_eq!(bare.to_string(), "COUNT(*) FILTER (WHERE \"ok\") AS n");
    }

    /// Raw is a verbatim pass-through, and the From impls build it.
    #[test]
    fn aggregate_expr_raw_passthrough() {
        let raw: AggregateExpr = "SUM(y) AS total".into();
        assert_eq!(raw.to_string(), "SUM(y) AS total");
        assert_eq!(raw, AggregateExpr::Raw("SUM(y) AS total".to_string()));
        let owned: AggregateExpr = String::from("COUNT(*)").into();
        assert_eq!(owned.to_string(), "COUNT(*)");
    }

    /// A typed call and the raw string that renders the same SQL are
    /// DIFFERENT IR values (analysis can tell them apart) even though their
    /// rendered bytes are identical.
    #[test]
    fn aggregate_expr_typed_and_raw_render_same_but_differ() {
        let typed = AggregateExpr::Call(call(
            AggregateFunction::Count,
            &["*"],
            Some("DOUBLE"),
            Some("__bf_count"),
        ));
        let raw = AggregateExpr::Raw("CAST(COUNT(*) AS DOUBLE) AS __bf_count".to_string());
        assert_eq!(typed.to_string(), raw.to_string(), "same rendered bytes");
        assert_ne!(typed, raw, "different IR values");
        let mut h1 = DefaultHasher::new();
        let mut h2 = DefaultHasher::new();
        typed.hash(&mut h1);
        raw.hash(&mut h2);
        assert_ne!(h1.finish(), h2.finish(), "hash follows the IR value");
    }

    /// A plan whose aggregates are typed still hashes structurally, and the
    /// hash moves when the aggregate value changes.
    #[test]
    fn hash_structural_covers_typed_aggregates() {
        let plan = |alias: &str| QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            group_by: vec!["\"x\"".to_string()],
            aggregates: vec![AggregateExpr::Call(call(
                AggregateFunction::Sum,
                &["\"y\""],
                None,
                Some(alias),
            ))],
        };
        assert_eq!(plan("a").hash_structural(), plan("a").hash_structural());
        assert_ne!(plan("a").hash_structural(), plan("b").hash_structural());
    }

    /// A QueryPlan containing a structured clause still hashes structurally
    /// (the derived Eq + Hash stay sound with the f64 payloads).
    #[test]
    fn hash_structural_covers_structured_clauses() {
        let plan = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            predicate: Predicate::Interval {
                column: "x".to_string(),
                lo: ScalarValue::Float(1.5),
                hi: ScalarValue::Float(2.5),
                meta: None,
            },
        };
        assert_eq!(plan.hash_structural(), plan.clone().hash_structural());
    }
}
