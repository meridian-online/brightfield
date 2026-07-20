//! Navigation filter pass — inserts BETWEEN predicates for the visible extent.
//!
//! When the user pans or zooms, the UI layer computes a ViewExtent with the
//! new domain bounds. This pass injects a `Filter` node into the QueryPlan
//! for each navigable axis, restricting data to the visible range.
//!
//! The pass receives plain column-name + (min, max) pairs — it does NOT
//! depend on `brightfield-render` or `ViewExtent`. The caller (engine layer)
//! translates ViewExtent into these primitives.

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

/// Pass that inserts Filter nodes for the visible navigation extent.
///
/// Constructed with zero or more axis filters. Each non-empty axis produces
/// a `Filter { predicate: And(col >= min, col <= max) }` wrapping the plan.
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
    pub fn from_extents(
        x: Option<(&str, f64, f64)>,
        y: Option<(&str, f64, f64)>,
    ) -> Self {
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
}

impl Pass for NavigationFilterPass {
    fn apply(&self, plan: QueryPlan) -> QueryPlan {
        let mut result = plan;
        for filter in &self.filters {
            let predicate = Predicate::And(vec![
                Predicate::Expr(format!("\"{}\" >= {}", filter.column, filter.min)),
                Predicate::Expr(format!("\"{}\" <= {}", filter.column, filter.max)),
            ]);
            result = QueryPlan::Filter {
                input: Box::new(result),
                predicate,
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
        let pass = NavigationFilterPass::from_extents(
            Some(("x", 2.0, 4.0)),
            None,
        );
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
        let pass = NavigationFilterPass::from_extents(
            None,
            Some(("y", 10.0, 50.0)),
        );
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
        let pass = NavigationFilterPass::from_extents(
            Some(("x", 1.0, 3.0)),
            Some(("y", 10.0, 30.0)),
        );
        let result = pass.apply(source_plan());

        // Outermost should be y filter (applied second).
        match &result {
            QueryPlan::Filter { input, predicate } => {
                let outer_pred = format!("{predicate}");
                assert!(outer_pred.contains("\"y\""), "outer should be y filter, got: {outer_pred}");

                // Inner should be x filter.
                match input.as_ref() {
                    QueryPlan::Filter { input: inner_input, predicate: inner_pred } => {
                        let inner_pred_str = format!("{inner_pred}");
                        assert!(inner_pred_str.contains("\"x\""), "inner should be x filter, got: {inner_pred_str}");
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
        assert!(debug.contains("timestamp"), "should use channel-mapped column name 'timestamp'");
        assert!(debug.contains("price"), "should use channel-mapped column name 'price'");
    }

    #[test]
    fn none_extent_is_noop() {
        let pass = NavigationFilterPass::from_extents(None, None);
        let plan = source_plan();
        let result = pass.apply(plan.clone());
        assert_eq!(result, plan, "no filters should mean identity");
    }
}
