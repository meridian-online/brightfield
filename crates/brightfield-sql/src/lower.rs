//! AST → IR lowering (ac-03, ac-05).
//!
//! The `MarkLower` trait is the extension point for per-mark lowering. V1
//! ships no concrete implementations — every `MarkKind` returns
//! `EmitError::UnsupportedMark`. As marks are implemented in future cards,
//! each gets a `MarkLower` impl registered in `default_lowerers`.

use indexmap::IndexMap;

use brightfield_spec::ast::{Mark, MarkData, ParamNode, SelectionNode};
use brightfield_spec::vocab::MarkKind;

use crate::error::EmitError;
use crate::ir::{Predicate, QueryPlan, SelectionResolution};

/// Context available during lowering — spec-level data sources, params, selections.
#[derive(Debug)]
pub struct LowerCtx<'a> {
    /// All named data sources from the spec.
    pub data_sources: &'a IndexMap<String, brightfield_spec::ast::DataSource>,
    /// All named params from the spec.
    pub params: &'a IndexMap<String, ParamNode>,
}

/// Trait for per-mark AST → IR lowering.
///
/// Each `MarkKind` that brightfield can emit SQL for gets a `MarkLower` impl.
/// The default implementation returns `EmitError::UnsupportedMark`.
pub trait MarkLower: Send + Sync {
    /// Lower a mark to a `QueryPlan`.
    fn lower(&self, mark: &Mark, ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError>;
}

/// Default lowerer that rejects all marks as unsupported.
pub struct DefaultLowerer;

impl MarkLower for DefaultLowerer {
    fn lower(&self, mark: &Mark, _ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        Err(EmitError::UnsupportedMark {
            kind: mark.kind.wire_name().to_string(),
        })
    }
}

/// Simple lowerer for marks with `data: { from: table }`.
///
/// Emits `QueryPlan::Source { table }` — equivalent to `SELECT * FROM table`.
/// Returns `UnsupportedMark` for marks without data or with inline data.
pub struct SimpleLowerer;

impl MarkLower for SimpleLowerer {
    fn lower(&self, mark: &Mark, _ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        match &mark.data {
            Some(MarkData::From { source, .. }) => Ok(QueryPlan::Source {
                table: source.clone(),
            }),
            Some(MarkData::Inline(_)) | None => Err(EmitError::UnsupportedMark {
                kind: mark.kind.wire_name().to_string(),
            }),
        }
    }
}

/// Build the registry of mark lowerers.
///
/// Registers SimpleLowerer for Dot, Line, BarX, BarY.
/// Marks not listed here fall back to DefaultLowerer (unsupported).
pub fn default_lowerers() -> Vec<(MarkKind, Box<dyn MarkLower>)> {
    vec![
        (MarkKind::Dot, Box::new(SimpleLowerer)),
        (MarkKind::Line, Box::new(SimpleLowerer)),
        (MarkKind::BarX, Box::new(SimpleLowerer)),
        (MarkKind::BarY, Box::new(SimpleLowerer)),
    ]
}

/// Look up a lowerer for a given `MarkKind`, falling back to `DefaultLowerer`.
pub fn find_lowerer<'a>(
    kind: MarkKind,
    registry: &'a [(MarkKind, Box<dyn MarkLower>)],
) -> &'a dyn MarkLower {
    registry
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, l)| l.as_ref())
        .unwrap_or(&DefaultLowerer)
}

/// Compile a selection into a predicate.
///
/// Crossfilter resolution drops predicates whose source matches `self_source`.
/// Empty selections render as `Predicate::True` (WHERE TRUE).
/// Union combines with OR; Intersect combines with AND.
pub fn compile_selection(
    selection: &SelectionNode,
    self_source: &str,
    predicates: &[(String, Predicate)], // (source_name, predicate)
) -> Predicate {
    let resolution = SelectionResolution::from(selection.select);

    let active: Vec<Predicate> = match resolution {
        SelectionResolution::Crossfilter => {
            // Drop predicates from self_source
            predicates
                .iter()
                .filter(|(source, _)| source != self_source)
                .map(|(_, p)| p.clone())
                .collect()
        }
        _ => predicates.iter().map(|(_, p)| p.clone()).collect(),
    };

    if active.is_empty() {
        return Predicate::True;
    }

    match resolution {
        SelectionResolution::Union => Predicate::Or(active),
        SelectionResolution::Intersect | SelectionResolution::Crossfilter => {
            Predicate::And(active)
        }
        SelectionResolution::Single => {
            // Single: take the last predicate
            active.into_iter().last().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::ast::MarkData;
    use brightfield_spec::vocab::{ImplStatus, SelectionResolution as AstRes};

    fn make_mark(kind: MarkKind) -> Mark {
        Mark {
            kind,
            status: ImplStatus::Unimplemented,
            data: Some(MarkData::From {
                source: "flights".to_string(),
                filter_by: None,
                extras: IndexMap::new(),
            }),
            options: IndexMap::new(),
        }
    }

    fn make_ctx() -> LowerCtx<'static> {
        // Use leaked static for test convenience
        let data_sources: &'static IndexMap<String, brightfield_spec::ast::DataSource> =
            Box::leak(Box::new(IndexMap::new()));
        let params: &'static IndexMap<String, ParamNode> =
            Box::leak(Box::new(IndexMap::new()));
        LowerCtx {
            data_sources,
            params,
        }
    }

    #[test]
    fn dfir_ac03_default_lowerer_returns_unsupported() {
        let mark = make_mark(MarkKind::Line);
        let ctx = make_ctx();
        let result = DefaultLowerer.lower(&mark, &ctx);
        assert!(result.is_err());
        match result.unwrap_err() {
            EmitError::UnsupportedMark { kind } => assert_eq!(kind, "line"),
            other => panic!("expected UnsupportedMark, got {other:?}"),
        }
    }

    #[test]
    fn dfir_ac03_find_lowerer_falls_back_to_default() {
        let registry = default_lowerers();
        // Rect is not registered — should fall back to DefaultLowerer
        let lowerer = find_lowerer(MarkKind::Rect, &registry);
        let mark = make_mark(MarkKind::Rect);
        let ctx = make_ctx();
        let result = lowerer.lower(&mark, &ctx);
        assert!(matches!(result, Err(EmitError::UnsupportedMark { .. })));
    }

    // --- AC-01 tests: SimpleLowerer ---

    #[test]
    fn msv_ac01_simple_lowerer_produces_source_for_from() {
        let mark = make_mark(MarkKind::Dot);
        let ctx = make_ctx();
        let result = SimpleLowerer.lower(&mark, &ctx);
        assert_eq!(
            result.unwrap(),
            QueryPlan::Source {
                table: "flights".to_string()
            }
        );
    }

    #[test]
    fn msv_ac01_simple_lowerer_rejects_no_data() {
        let mark = Mark {
            kind: MarkKind::Dot,
            status: ImplStatus::Unimplemented,
            data: None,
            options: IndexMap::new(),
        };
        let ctx = make_ctx();
        let result = SimpleLowerer.lower(&mark, &ctx);
        assert!(matches!(result, Err(EmitError::UnsupportedMark { .. })));
    }

    #[test]
    fn msv_ac01_simple_lowerer_rejects_inline_data() {
        let mark = Mark {
            kind: MarkKind::Line,
            status: ImplStatus::Unimplemented,
            data: Some(MarkData::Inline(vec![])),
            options: IndexMap::new(),
        };
        let ctx = make_ctx();
        let result = SimpleLowerer.lower(&mark, &ctx);
        match result.unwrap_err() {
            EmitError::UnsupportedMark { kind } => assert_eq!(kind, "line"),
            other => panic!("expected UnsupportedMark, got {other:?}"),
        }
    }

    #[test]
    fn msv_ac01_default_lowerers_contains_all_registered_kinds() {
        let registry = default_lowerers();
        let kinds: Vec<MarkKind> = registry.iter().map(|(k, _)| *k).collect();
        assert!(kinds.contains(&MarkKind::Dot));
        assert!(kinds.contains(&MarkKind::Line));
        assert!(kinds.contains(&MarkKind::BarX));
        assert!(kinds.contains(&MarkKind::BarY));
        assert_eq!(kinds.len(), 4);
    }

    #[test]
    fn msv_ac01_find_lowerer_returns_simple_for_registered() {
        let registry = default_lowerers();
        let lowerer = find_lowerer(MarkKind::Dot, &registry);
        let mark = make_mark(MarkKind::Dot);
        let ctx = make_ctx();
        // SimpleLowerer should succeed for a mark with data.from
        let result = lowerer.lower(&mark, &ctx);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            QueryPlan::Source {
                table: "flights".to_string()
            }
        );
    }

    #[test]
    fn dfir_ac05_crossfilter_excludes_self() {
        let selection = SelectionNode {
            select: AstRes::Crossfilter,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates = vec![
            (
                "view_a".to_string(),
                Predicate::Expr("x > 1".to_string()),
            ),
            (
                "view_b".to_string(),
                Predicate::Expr("y < 10".to_string()),
            ),
        ];
        let result = compile_selection(&selection, "view_a", &predicates);
        // view_a's predicate should be excluded
        assert_eq!(
            result,
            Predicate::And(vec![Predicate::Expr("y < 10".to_string())])
        );
    }

    #[test]
    fn dfir_ac05_crossfilter_includes_in_other_view() {
        let selection = SelectionNode {
            select: AstRes::Crossfilter,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates = vec![
            (
                "view_a".to_string(),
                Predicate::Expr("x > 1".to_string()),
            ),
            (
                "view_b".to_string(),
                Predicate::Expr("y < 10".to_string()),
            ),
        ];
        let result = compile_selection(&selection, "view_b", &predicates);
        // view_b's predicate excluded, view_a's included
        assert_eq!(
            result,
            Predicate::And(vec![Predicate::Expr("x > 1".to_string())])
        );
    }

    #[test]
    fn dfir_ac05_empty_selection_is_true() {
        let selection = SelectionNode {
            select: AstRes::Intersect,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates: Vec<(String, Predicate)> = vec![];
        let result = compile_selection(&selection, "view_a", &predicates);
        assert_eq!(result, Predicate::True);
    }

    #[test]
    fn dfir_ac05_union_combines_with_or() {
        let selection = SelectionNode {
            select: AstRes::Union,
            status: ImplStatus::Unimplemented,
            options: IndexMap::new(),
        };
        let predicates = vec![
            (
                "view_a".to_string(),
                Predicate::Expr("x > 1".to_string()),
            ),
            (
                "view_b".to_string(),
                Predicate::Expr("y < 10".to_string()),
            ),
        ];
        let result = compile_selection(&selection, "view_a", &predicates);
        assert_eq!(
            result,
            Predicate::Or(vec![
                Predicate::Expr("x > 1".to_string()),
                Predicate::Expr("y < 10".to_string()),
            ])
        );
    }
}
