//! Navigation filter pass — inserts BETWEEN predicates for the visible extent.
//!
//! When the user pans or zooms, the UI layer computes a ViewExtent with the
//! new domain bounds. This pass injects a `Filter` node into the QueryPlan
//! for each navigable axis, restricting data to the visible range.
//!
//! The pass receives plain column-name + (min, max) pairs — it does NOT
//! depend on `brightfield-render` or `ViewExtent`. The caller (engine layer)
//! translates ViewExtent into these primitives.
//!
//! # Where the predicate goes, and why it is not always the top
//!
//! A navigation gesture and a brush have to mean the same thing on an
//! aggregating mark: both scope the data the aggregate is computed FROM. Wrap
//! a density plan's finished `GROUP BY` in a filter and the bins outside the
//! view disappear while every surviving bin keeps the count it had at full
//! extent — a picture that looks zoomed and is arithmetically the old one. So
//! this pass resolves each axis to one of three placements
//! ([`axis_pushdown`]):
//!
//! - [`Pushdown::Top`] — no aggregation in the plan. The mark draws rows, and
//!   filtering its rows is the whole of what navigation means.
//! - [`Pushdown::UnderGroupBy`] — the axis's column is a grouping key, so the
//!   same column exists in the aggregation's INPUT. The filter goes beneath the
//!   `GROUP BY`, exactly where the selection path puts a brush's predicate, and
//!   the aggregate is recomputed over the visible range.
//! - [`Pushdown::Decline`] — the axis's column is produced BY the aggregate
//!   (`count(*) AS …`). There is no such column below the `GROUP BY` to filter
//!   on, and filtering above would drop finished bins by their own height,
//!   which is not what "show me this range" means. The axis is dropped:
//!   nothing is emitted for it, the mark still draws, and the scale override
//!   crops it visually.
//!
//! Declining is a BAIL, not an error. A navigation gesture must never take a
//! chart down.

use crate::ir::{Predicate, QueryPlan};
use crate::passes::Pass;

/// A filter specification for one axis: column name and domain bounds.
#[derive(Debug, Clone)]
pub struct AxisFilter {
    /// Column name to filter on.
    pub column: String,
    /// Inclusive lower bound.
    pub min: f64,
    /// Inclusive upper bound.
    pub max: f64,
}

/// Where one axis's navigation predicate can honestly go in a lowered plan.
///
/// See the module docs for why the three cases exist. Resolved by
/// [`axis_pushdown`], which is pure and takes the plan it will be applied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pushdown {
    /// Wrap the plan — a row-drawing mark filters its own rows.
    Top,
    /// Insert beneath the aggregation, so the aggregate is RECOMPUTED over the
    /// visible range (bit-for-bit the placement a brush's predicate gets).
    UnderGroupBy,
    /// Nowhere honest — the column is an aggregate OUTPUT. Emit nothing for
    /// this axis.
    Decline,
}

/// Resolve where `column`'s navigation predicate belongs in `plan`.
///
/// Walks the structural wrappers a lowered mark plan puts above its
/// aggregation (`ORDER BY`, `LIMIT`, the highlight `Projection`, a selection
/// `Filter`) looking for the aggregation itself. A plan with none is
/// [`Pushdown::Top`]. A plan whose aggregation groups by this column is
/// [`Pushdown::UnderGroupBy`]. Anything else that aggregates is
/// [`Pushdown::Decline`].
#[must_use]
pub fn axis_pushdown(plan: &QueryPlan, column: &str) -> Pushdown {
    match plan {
        QueryPlan::Order { input, .. }
        | QueryPlan::Limit { input, .. }
        | QueryPlan::Projection { input, .. }
        | QueryPlan::Filter { input, .. } => axis_pushdown(input, column),
        QueryPlan::Aggregation { group_by, .. } => {
            if group_by.iter().any(|expr| group_key_provides(expr, column)) {
                Pushdown::UnderGroupBy
            } else {
                Pushdown::Decline
            }
        }
        // A scalar aggregate has no grouping keys at all — every output column
        // is an aggregate, so there is nothing a navigation extent can name.
        QueryPlan::AggregateScalar { .. } => Pushdown::Decline,
        _ => Pushdown::Top,
    }
}

/// Whether a `GROUP BY` expression makes `column` available as a grouping key.
///
/// Two shapes reach here, and both are produced by the lowerers rather than
/// guessed at: a bare quoted column (`"day"`, the categorical cell path) and a
/// computed expression aliased BACK to the channel's column name
/// (`floor(…) AS "latency"`, every binning path — the alias is deliberately the
/// source column's own name so scale inference treats a bin centre like any
/// positional column). In both cases the source column exists in the
/// aggregation's input, which is what makes the push-down legal.
fn group_key_provides(expr: &str, column: &str) -> bool {
    let quoted = format!("\"{}\"", column.replace('"', "\"\""));
    let trimmed = expr.trim();
    if trimmed == quoted || trimmed == column {
        return true;
    }
    match trimmed.rfind(" AS ") {
        Some(at) => {
            let alias = trimmed[at + 4..].trim();
            alias == quoted || alias == column
        }
        None => false,
    }
}

/// Insert `predicate` immediately beneath the plan's aggregation, descending
/// through the structural wrappers above it.
///
/// Only called when [`axis_pushdown`] resolved [`Pushdown::UnderGroupBy`], so
/// an aggregation is present by construction; the fallthrough returns the plan
/// untouched rather than inventing a placement.
fn insert_under_aggregation(plan: QueryPlan, predicate: Predicate) -> QueryPlan {
    match plan {
        QueryPlan::Order { input, keys } => QueryPlan::Order {
            input: Box::new(insert_under_aggregation(*input, predicate)),
            keys,
        },
        QueryPlan::Limit {
            input,
            limit,
            offset,
        } => QueryPlan::Limit {
            input: Box::new(insert_under_aggregation(*input, predicate)),
            limit,
            offset,
        },
        QueryPlan::Projection { input, columns } => QueryPlan::Projection {
            input: Box::new(insert_under_aggregation(*input, predicate)),
            columns,
        },
        QueryPlan::Filter {
            input,
            predicate: outer,
        } => QueryPlan::Filter {
            input: Box::new(insert_under_aggregation(*input, predicate)),
            predicate: outer,
        },
        QueryPlan::Aggregation {
            input,
            group_by,
            aggregates,
        } => QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Filter { input, predicate }),
            group_by,
            aggregates,
        },
        other => other,
    }
}

/// Pass that inserts Filter nodes for the visible navigation extent.
///
/// Constructed with zero or more axis filters. Each axis resolves its own
/// placement through [`axis_pushdown`]; an axis that resolves to
/// [`Pushdown::Decline`] contributes nothing.
///
/// When no axis filters are present, the pass is a no-op (identity).
#[derive(Debug, Clone)]
pub struct NavigationFilterPass {
    filters: Vec<AxisFilter>,
}

impl NavigationFilterPass {
    /// Create a new pass with the given axis filters.
    pub fn new(filters: Vec<AxisFilter>) -> Self {
        Self { filters }
    }

    /// Create a pass from optional x/y axis specifications.
    ///
    /// This is the primary constructor used by the engine layer, translating
    /// ViewExtent's `Option<(f64, f64)>` per axis into filter specs.
    pub fn from_extents(x: Option<(&str, f64, f64)>, y: Option<(&str, f64, f64)>) -> Self {
        let mut filters = Vec::new();
        if let Some((col, min, max)) = x {
            filters.push(AxisFilter {
                column: col.to_string(),
                min,
                max,
            });
        }
        if let Some((col, min, max)) = y {
            filters.push(AxisFilter {
                column: col.to_string(),
                min,
                max,
            });
        }
        Self { filters }
    }

    /// The axes this pass would emit nothing for against `plan` — the bail
    /// list, exposed so a caller can say so rather than let a gesture look
    /// applied when it was not.
    #[must_use]
    pub fn declined(&self, plan: &QueryPlan) -> Vec<String> {
        self.filters
            .iter()
            .filter(|f| axis_pushdown(plan, &f.column) == Pushdown::Decline)
            .map(|f| f.column.clone())
            .collect()
    }
}

impl Pass for NavigationFilterPass {
    fn apply(&self, plan: QueryPlan) -> QueryPlan {
        let mut result = plan;
        for filter in &self.filters {
            let predicate = Predicate::And(vec![
                Predicate::Expr(format!("\"{}\" >= {}", filter.column, filter.min)),
                Predicate::Expr(format!("\"{}\" <= {}", filter.column, filter.max)),
            ]);
            result = match axis_pushdown(&result, &filter.column) {
                Pushdown::Top => QueryPlan::Filter {
                    input: Box::new(result),
                    predicate,
                },
                Pushdown::UnderGroupBy => insert_under_aggregation(result, predicate),
                // The bail: this axis names an aggregate output. Emit nothing
                // rather than a predicate that would either fail to bind below
                // the GROUP BY or silently drop finished bins above it.
                Pushdown::Decline => result,
            };
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_plan() -> QueryPlan {
        QueryPlan::Source {
            table: "flights".to_string(),
        }
    }

    #[test]
    fn empty_filters_is_identity() {
        let pass = NavigationFilterPass::new(vec![]);
        let plan = source_plan();
        let result = pass.apply(plan.clone());
        assert_eq!(result, plan);
    }

    #[test]
    fn x_only_inserts_one_filter() {
        let pass = NavigationFilterPass::from_extents(Some(("x", 2.0, 4.0)), None);
        let result = pass.apply(source_plan());

        match &result {
            QueryPlan::Filter { input, predicate } => {
                // Input should be the original source.
                assert!(matches!(input.as_ref(), QueryPlan::Source { .. }));
                // Predicate should be AND of two conditions on "x".
                let pred_str = format!("{predicate}");
                assert!(pred_str.contains("\"x\" >= 2"), "got: {pred_str}");
                assert!(pred_str.contains("\"x\" <= 4"), "got: {pred_str}");
            }
            other => panic!("expected Filter, got: {other:?}"),
        }
    }

    #[test]
    fn y_only_inserts_one_filter() {
        let pass = NavigationFilterPass::from_extents(None, Some(("y", 10.0, 50.0)));
        let result = pass.apply(source_plan());

        match &result {
            QueryPlan::Filter { predicate, .. } => {
                let pred_str = format!("{predicate}");
                assert!(pred_str.contains("\"y\" >= 10"), "got: {pred_str}");
                assert!(pred_str.contains("\"y\" <= 50"), "got: {pred_str}");
            }
            other => panic!("expected Filter, got: {other:?}"),
        }
    }

    #[test]
    fn both_axes_inserts_two_nested_filters() {
        let pass =
            NavigationFilterPass::from_extents(Some(("x", 1.0, 3.0)), Some(("y", 10.0, 30.0)));
        let result = pass.apply(source_plan());

        // Outermost should be y filter (applied second).
        match &result {
            QueryPlan::Filter { input, predicate } => {
                let outer_pred = format!("{predicate}");
                assert!(
                    outer_pred.contains("\"y\""),
                    "outer should be y filter, got: {outer_pred}"
                );

                // Inner should be x filter.
                match input.as_ref() {
                    QueryPlan::Filter {
                        input: inner_input,
                        predicate: inner_pred,
                    } => {
                        let inner_pred_str = format!("{inner_pred}");
                        assert!(
                            inner_pred_str.contains("\"x\""),
                            "inner should be x filter, got: {inner_pred_str}"
                        );
                        assert!(matches!(inner_input.as_ref(), QueryPlan::Source { .. }));
                    }
                    other => panic!("expected inner Filter, got: {other:?}"),
                }
            }
            other => panic!("expected outer Filter, got: {other:?}"),
        }
    }

    #[test]
    fn predicate_uses_column_names_from_channel_map() {
        // Simulate ChannelMap providing "timestamp" for x and "price" for y.
        let pass = NavigationFilterPass::from_extents(
            Some(("timestamp", 1000.0, 2000.0)),
            Some(("price", 50.0, 150.0)),
        );
        let result = pass.apply(source_plan());

        let debug = format!("{result:?}");
        assert!(
            debug.contains("timestamp"),
            "should use channel-mapped column name 'timestamp'"
        );
        assert!(
            debug.contains("price"),
            "should use channel-mapped column name 'price'"
        );
    }

    #[test]
    fn none_extent_is_noop() {
        let pass = NavigationFilterPass::from_extents(None, None);
        let plan = source_plan();
        let result = pass.apply(plan.clone());
        assert_eq!(result, plan, "no filters should mean identity");
    }

    // -----------------------------------------------------------------------
    // Aggregating plans — the placement that makes zoom and brush agree
    // -----------------------------------------------------------------------

    use crate::ir::{AggregateCall, AggregateExpr, AggregateFunction, SortDir};

    /// The shape every binning lowerer emits: `ORDER BY` over a `GROUP BY`
    /// whose key is a computed bin centre aliased BACK to the channel column,
    /// over a not-null filter over the source.
    fn binned_density_plan() -> QueryPlan {
        QueryPlan::Order {
            input: Box::new(QueryPlan::Aggregation {
                input: Box::new(QueryPlan::Filter {
                    input: Box::new(QueryPlan::Source {
                        table: "traces".to_string(),
                    }),
                    predicate: Predicate::Expr("\"latency\" IS NOT NULL".to_string()),
                }),
                group_by: vec!["floor(\"latency\" / 0.5) * 0.5 + 0.25 AS \"latency\"".to_string()],
                aggregates: vec![AggregateExpr::Call(AggregateCall {
                    func: AggregateFunction::Count,
                    args: vec!["*".to_string()],
                    filter: None,
                    cast: Some("DOUBLE".to_string()),
                    alias: Some("__bf_count".to_string()),
                })],
            }),
            keys: vec![("\"latency\"".to_string(), SortDir::Asc)],
        }
    }

    /// The aggregation node inside a plan, whatever wraps it.
    fn aggregation_of(plan: &QueryPlan) -> &QueryPlan {
        match plan {
            QueryPlan::Order { input, .. }
            | QueryPlan::Limit { input, .. }
            | QueryPlan::Projection { input, .. }
            | QueryPlan::Filter { input, .. } => aggregation_of(input),
            other => other,
        }
    }

    /// A zoom on a grouping column lands UNDER the `GROUP BY`, so the count is
    /// recomputed over the visible range — the same placement the selection
    /// path gives a brush.
    #[test]
    fn a_grouping_column_pushes_beneath_the_group_by() {
        let pass = NavigationFilterPass::from_extents(Some(("latency", 2.0, 4.0)), None);
        let result = pass.apply(binned_density_plan());

        assert_eq!(
            axis_pushdown(&binned_density_plan(), "latency"),
            Pushdown::UnderGroupBy
        );
        // The outermost node is still the ORDER BY — the filter did not wrap
        // the finished aggregate.
        assert!(matches!(result, QueryPlan::Order { .. }), "{result:?}");
        let QueryPlan::Aggregation { input, .. } = aggregation_of(&result) else {
            panic!("expected the aggregation to survive: {result:?}");
        };
        let QueryPlan::Filter { predicate, .. } = input.as_ref() else {
            panic!("expected the navigation filter directly beneath the GROUP BY: {input:?}");
        };
        let rendered = format!("{predicate}");
        assert!(rendered.contains("\"latency\" >= 2"), "got: {rendered}");
        assert!(rendered.contains("\"latency\" <= 4"), "got: {rendered}");
    }

    /// The bail. An axis naming an aggregate OUTPUT emits nothing at all —
    /// neither a predicate below the `GROUP BY` (there is no such column) nor
    /// one above it (that would drop finished bins by their own height).
    #[test]
    fn an_aggregate_output_column_is_declined_not_filtered() {
        let plan = binned_density_plan();
        assert_eq!(axis_pushdown(&plan, "__bf_count"), Pushdown::Decline);

        let pass = NavigationFilterPass::from_extents(None, Some(("__bf_count", 10.0, 90.0)));
        assert_eq!(pass.declined(&plan), vec!["__bf_count".to_string()]);
        assert_eq!(
            pass.apply(plan.clone()),
            plan,
            "a declined axis must leave the plan untouched"
        );
    }

    /// Both axes at once on a two-key aggregation: each pushes under, and the
    /// aggregation is still the only aggregation.
    #[test]
    fn both_grouping_axes_push_beneath_one_group_by() {
        let plan = QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Source {
                table: "points".to_string(),
            }),
            group_by: vec![
                "CAST(cx AS DOUBLE) AS \"x\"".to_string(),
                "CAST(cy AS DOUBLE) AS \"y\"".to_string(),
            ],
            aggregates: vec![AggregateExpr::Raw("count(*) AS __bf_count".to_string())],
        };
        let pass =
            NavigationFilterPass::from_extents(Some(("x", 1.0, 3.0)), Some(("y", 10.0, 30.0)));
        let result = pass.apply(plan);

        let QueryPlan::Aggregation { input, .. } = &result else {
            panic!("the aggregation must stay outermost: {result:?}");
        };
        // Two nested filters beneath it, one per axis.
        let QueryPlan::Filter { input, predicate } = input.as_ref() else {
            panic!("expected a filter beneath the GROUP BY: {input:?}");
        };
        assert!(format!("{predicate}").contains("\"y\""), "{predicate}");
        let QueryPlan::Filter { input, predicate } = input.as_ref() else {
            panic!("expected the second axis filter beneath the first: {input:?}");
        };
        assert!(format!("{predicate}").contains("\"x\""), "{predicate}");
        assert!(matches!(input.as_ref(), QueryPlan::Source { .. }));
    }

    /// A bare quoted grouping key (the categorical cell path) also resolves as
    /// a grouping column — the alias form is not the only one the lowerers emit.
    #[test]
    fn a_bare_quoted_grouping_key_is_recognised() {
        let plan = QueryPlan::Aggregation {
            input: Box::new(QueryPlan::Source {
                table: "activity".to_string(),
            }),
            group_by: vec!["\"day\"".to_string(), "\"hour\"".to_string()],
            aggregates: vec![AggregateExpr::Raw("sum(value) AS value".to_string())],
        };
        assert_eq!(axis_pushdown(&plan, "day"), Pushdown::UnderGroupBy);
        assert_eq!(axis_pushdown(&plan, "value"), Pushdown::Decline);
    }

    /// A scalar aggregate (regression statistics) has no grouping keys, so
    /// every axis declines rather than filtering a one-row summary.
    #[test]
    fn a_scalar_aggregate_declines_every_axis() {
        let plan = QueryPlan::AggregateScalar {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            aggregates: vec![AggregateExpr::Raw("regr_slope(y, x) AS slope".to_string())],
        };
        assert_eq!(axis_pushdown(&plan, "x"), Pushdown::Decline);
        assert_eq!(pass_over(&plan, "x"), plan);
    }

    fn pass_over(plan: &QueryPlan, column: &str) -> QueryPlan {
        NavigationFilterPass::from_extents(Some((column, 0.0, 1.0)), None).apply(plan.clone())
    }

    /// A row-drawing plan still wraps at the top — the aggregate-aware walk
    /// must not change what a scatter's navigation emits.
    #[test]
    fn a_row_level_plan_still_wraps_at_the_top() {
        let plan = QueryPlan::Projection {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            columns: vec!["\"x\"".to_string(), "\"y\"".to_string()],
        };
        assert_eq!(axis_pushdown(&plan, "x"), Pushdown::Top);
        let result = pass_over(&plan, "x");
        let QueryPlan::Filter { input, .. } = &result else {
            panic!("expected a top-level filter: {result:?}");
        };
        assert!(matches!(input.as_ref(), QueryPlan::Projection { .. }));
    }
}
