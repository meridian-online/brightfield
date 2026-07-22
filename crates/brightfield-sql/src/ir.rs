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
        /// Aggregate expressions (e.g. `"SUM(y) AS total"`).
        aggregates: Vec<String>,
    },

    /// Scalar aggregation — produces a single row, no `GROUP BY`.
    ///
    /// Renders as `SELECT <aggregates> FROM (<input>)`. Used for
    /// regression statistics (regr_slope, regr_intercept, ...) where
    /// the output is a single row of summary statistics.
    AggregateScalar {
        input: Box<QueryPlan>,
        /// Aggregate expressions (e.g. `"regr_slope(y, x) AS slope"`).
        aggregates: Vec<String>,
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
            aggregates: vec!["SUM(y)".to_string()],
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
