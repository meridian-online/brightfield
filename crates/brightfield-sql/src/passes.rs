//! Optimisation pass pipeline.
//!
//! V1 ships the pipeline shape with no passes registered. Each future
//! optimisation (pre-aggregation, M4) adds a `Pass` impl.

use crate::ir::QueryPlan;

/// A single optimisation pass over a `QueryPlan`.
pub trait Pass: Send + Sync {
    /// Transform the plan. Passes must preserve semantics.
    fn apply(&self, plan: QueryPlan) -> QueryPlan;
}

/// Fold-left application of a pass sequence.
///
/// With an empty `passes` slice this is the identity function.
#[must_use]
pub fn apply_passes(mut plan: QueryPlan, passes: &[Box<dyn Pass>]) -> QueryPlan {
    for pass in passes {
        plan = pass.apply(plan);
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::Predicate;

    struct NoOpPass;
    impl Pass for NoOpPass {
        fn apply(&self, plan: QueryPlan) -> QueryPlan {
            plan
        }
    }

    #[test]
    fn empty_passes_is_identity() {
        let plan = QueryPlan::Source {
            table: "t".to_string(),
        };
        let passes: Vec<Box<dyn Pass>> = vec![];
        let result = apply_passes(plan.clone(), &passes);
        assert_eq!(result, plan);
    }

    #[test]
    fn noop_pass_composes() {
        let plan = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: "t".to_string(),
            }),
            predicate: Predicate::True,
        };
        let passes: Vec<Box<dyn Pass>> = vec![Box::new(NoOpPass), Box::new(NoOpPass)];
        let result = apply_passes(plan.clone(), &passes);
        assert_eq!(result, plan);
    }
}
