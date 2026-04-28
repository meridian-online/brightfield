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
    /// Leaf node referencing a named view (from card 0004's DDL).
    Source {
        table: String,
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
    fn dfir_ac01_source_variant_constructs() {
        let plan = QueryPlan::Source {
            table: "flights".to_string(),
        };
        assert!(format!("{plan:?}").contains("flights"));
    }

    #[test]
    fn dfir_ac01_filter_variant_constructs() {
        let plan = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            predicate: Predicate::Expr("x > 1".to_string()),
        };
        assert!(format!("{plan:?}").contains("Filter"));
    }

    #[test]
    fn dfir_ac01_all_variants_construct() {
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
    fn dfir_ac01_hash_stability() {
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
    fn dfir_ac01_selection_resolution_from_ast() {
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
    fn dfir_ac02_predicate_display_expr() {
        let p = Predicate::Expr("x > 1".to_string());
        assert_eq!(format!("{p}"), "x > 1");
    }

    #[test]
    fn dfir_ac02_predicate_display_param() {
        let p = Predicate::Param {
            name: "lo".to_string(),
            placeholder_index: 0,
        };
        assert_eq!(format!("{p}"), "?");
    }

    #[test]
    fn dfir_ac02_predicate_display_and() {
        let p = Predicate::And(vec![
            Predicate::Expr("x > 1".to_string()),
            Predicate::Expr("x < 10".to_string()),
        ]);
        assert_eq!(format!("{p}"), "(x > 1 AND x < 10)");
    }

    #[test]
    fn dfir_ac02_predicate_display_or() {
        let p = Predicate::Or(vec![
            Predicate::Expr("a = 1".to_string()),
            Predicate::Expr("b = 2".to_string()),
        ]);
        assert_eq!(format!("{p}"), "(a = 1 OR b = 2)");
    }

    #[test]
    fn dfir_ac02_predicate_empty_and_is_true() {
        assert_eq!(format!("{}", Predicate::And(vec![])), "TRUE");
    }

    #[test]
    fn dfir_ac02_predicate_empty_or_is_false() {
        assert_eq!(format!("{}", Predicate::Or(vec![])), "FALSE");
    }

    #[test]
    fn dfir_ac14_hash_structural_same_structure_same_hash() {
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
    fn dfir_ac14_hash_structural_different_structure_different_hash() {
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
}
