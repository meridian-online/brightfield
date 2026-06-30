//! AST → IR lowering (ac-03, ac-05).
//!
//! The `MarkLower` trait is the extension point for per-mark lowering. V1
//! ships no concrete implementations — every `MarkKind` returns
//! `EmitError::UnsupportedMark`. As marks are implemented in future cards,
//! each gets a `MarkLower` impl registered in `default_lowerers`.

use indexmap::IndexMap;

use brightfield_spec::ast::{Mark, MarkData, ParamNode, SelectionNode, SpecValue, ValueOrParamRef};
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

/// Helper: extract a literal string-valued option (skips ParamRef).
fn opt_string<'a>(
    options: &'a IndexMap<String, ValueOrParamRef<SpecValue>>,
    key: &str,
) -> Option<&'a str> {
    match options.get(key)? {
        ValueOrParamRef::Value(SpecValue::String(s)) => Some(s.as_str()),
        _ => None,
    }
}

/// Helper: extract a literal numeric option as f64.
fn opt_f64(options: &IndexMap<String, ValueOrParamRef<SpecValue>>, key: &str) -> Option<f64> {
    match options.get(key)? {
        ValueOrParamRef::Value(SpecValue::Float(f)) => Some(*f),
        ValueOrParamRef::Value(SpecValue::Integer(i)) => Some(*i as f64),
        _ => None,
    }
}

/// Lowerer for regression marks (regressionY, regressionX).
///
/// Emits a one-row AggregateScalar with regr_* aggregates. When `stroke`
/// resolves to a column name, wraps in an Aggregation with that grouping
/// (one row per category). The lowerer rejects polynomial/exponential
/// regression upfront.
pub struct RegressionLowerer;

impl MarkLower for RegressionLowerer {
    fn lower(&self, mark: &Mark, _ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        // Reject non-linear regression types.
        if let Some(t) = opt_string(&mark.options, "type") {
            if t != "linear" {
                return Err(EmitError::UnsupportedMark {
                    kind: format!("{} (type='{}' — only linear is supported)", mark.kind.wire_name(), t),
                });
            }
        }

        let source = match &mark.data {
            Some(MarkData::From { source, .. }) => source.clone(),
            _ => {
                return Err(EmitError::UnsupportedMark {
                    kind: mark.kind.wire_name().to_string(),
                })
            }
        };

        let x_col = opt_string(&mark.options, "x").ok_or_else(|| EmitError::UnsupportedMark {
            kind: format!("{} (missing x)", mark.kind.wire_name()),
        })?;
        let y_col = opt_string(&mark.options, "y").ok_or_else(|| EmitError::UnsupportedMark {
            kind: format!("{} (missing y)", mark.kind.wire_name()),
        })?;

        // Filter out NULLs in x or y so DuckDB's regr_* aggregates run cleanly.
        let filtered = QueryPlan::Filter {
            input: Box::new(QueryPlan::Source { table: source }),
            predicate: Predicate::Expr(format!(
                "\"{x_col}\" IS NOT NULL AND \"{y_col}\" IS NOT NULL"
            )),
        };

        let aggregates = vec![
            format!("regr_slope(\"{y_col}\", \"{x_col}\") AS slope"),
            format!("regr_intercept(\"{y_col}\", \"{x_col}\") AS intercept"),
            format!("regr_count(\"{y_col}\", \"{x_col}\") AS n"),
            format!("regr_avgx(\"{y_col}\", \"{x_col}\") AS x_bar"),
            format!("regr_sxx(\"{y_col}\", \"{x_col}\") AS sxx"),
            format!("regr_sxy(\"{y_col}\", \"{x_col}\") AS sxy"),
            format!("regr_syy(\"{y_col}\", \"{x_col}\") AS syy"),
        ];

        // Group by stroke column when present.
        if let Some(stroke_col) = opt_string(&mark.options, "stroke") {
            let mut group_aggregates = aggregates.clone();
            // Prepend the group key so it appears in the projection.
            // (Aggregation IR variant places group_by columns first via render_query.)
            let _ = &mut group_aggregates; // keep as-is; group_by handled below
            return Ok(QueryPlan::Aggregation {
                input: Box::new(filtered),
                group_by: vec![format!("\"{stroke_col}\"")],
                aggregates,
            });
        }

        Ok(QueryPlan::AggregateScalar {
            input: Box::new(filtered),
            aggregates,
        })
    }
}

/// Lowerer for density marks (density, densityX, densityY).
///
/// Emits a binning + group-count plan:
///   1D: SELECT width_bucket(x, ...) AS x_bin, COUNT(*) FROM source GROUP BY x_bin
///   2D: same with both x and y bins
///
/// The lowerer reads `bins` (or `thresholds`) from the mark's option bag.
/// Default is 100 to match Mosaic's reference implementation (spec 2026-04-28
/// statistical-marks ac-07 implementation note). The bin width is computed
/// from the column extent at render time — for the SQL pass we use a fixed
/// bucket count and let DuckDB compute extents via subqueries.
pub struct DensityLowerer {
    pub kind: DensityLowerKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DensityLowerKind {
    /// `densityX` — 1D density along x.
    OneDX,
    /// `densityY` — 1D density along y.
    OneDY,
    /// `density` — 2D density on both axes.
    #[allow(clippy::upper_case_acronyms)]
    TwoD,
}

impl MarkLower for DensityLowerer {
    fn lower(&self, mark: &Mark, _ctx: &LowerCtx<'_>) -> Result<QueryPlan, EmitError> {
        let source = match &mark.data {
            Some(MarkData::From { source, .. }) => source.clone(),
            _ => {
                return Err(EmitError::UnsupportedMark {
                    kind: mark.kind.wire_name().to_string(),
                })
            }
        };

        let bin_count = opt_f64(&mark.options, "thresholds")
            .or_else(|| opt_f64(&mark.options, "bins"))
            .map(|f| f as i64)
            .unwrap_or(100);

        // Resolve required columns.
        let x_col = opt_string(&mark.options, "x");
        let y_col = opt_string(&mark.options, "y");

        match self.kind {
            DensityLowerKind::OneDX => {
                let x = x_col.ok_or_else(|| EmitError::UnsupportedMark {
                    kind: "densityX (missing x)".to_string(),
                })?;
                Ok(build_density_1d(&source, x, "x_bin", bin_count))
            }
            DensityLowerKind::OneDY => {
                let y = y_col.ok_or_else(|| EmitError::UnsupportedMark {
                    kind: "densityY (missing y)".to_string(),
                })?;
                Ok(build_density_1d(&source, y, "y_bin", bin_count))
            }
            DensityLowerKind::TwoD => {
                let x = x_col.ok_or_else(|| EmitError::UnsupportedMark {
                    kind: "density (missing x)".to_string(),
                })?;
                let y = y_col.ok_or_else(|| EmitError::UnsupportedMark {
                    kind: "density (missing y)".to_string(),
                })?;
                Ok(build_density_2d(&source, x, y, bin_count))
            }
        }
    }
}

/// Portable 1-based equiwidth bucket of `col` over its `[min, max]` range into
/// `bins` buckets, cast to DOUBLE. Replaces DuckDB's `width_bucket(col, lo, hi,
/// n)`, which the bundled libduckdb lacks (first-render follow-up #4 — density
/// silently rendered nothing). `width_bucket(v, lo, hi, n)` ==
/// `floor((v - lo) / (hi - lo) * n) + 1`; `nullif` guards the all-equal case.
fn equiwidth_bucket(table: &str, col: &str, bins: i64) -> String {
    let lo = format!("(SELECT min(\"{col}\") FROM \"{table}\")");
    let hi = format!("(SELECT max(\"{col}\") FROM \"{table}\")");
    format!(
        "CAST(floor((\"{col}\" - {lo}) / nullif({hi} - {lo}, 0) * {bins}) + 1 AS DOUBLE)"
    )
}

/// Build a 1D density plan: equiwidth bucket on `col`, group by bucket, return
/// (centre, count).
fn build_density_1d(table: &str, col: &str, alias: &str, bins: i64) -> QueryPlan {
    // The render-side renderer expects `<alias>` (Float64) and `count` (Float64-ish).
    let bucket_expr = format!("{} AS \"{alias}\"", equiwidth_bucket(table, col, bins));
    let count_expr = format!("CAST(COUNT(*) AS DOUBLE) AS count");

    QueryPlan::Aggregation {
        input: Box::new(QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: table.to_string(),
            }),
            predicate: Predicate::Expr(format!("\"{col}\" IS NOT NULL")),
        }),
        group_by: vec![bucket_expr],
        aggregates: vec![count_expr],
    }
}

/// Build a 2D density plan: equiwidth bucket on both x and y, group by both.
fn build_density_2d(table: &str, x_col: &str, y_col: &str, bins: i64) -> QueryPlan {
    let x_bucket = format!("{} AS x_bin", equiwidth_bucket(table, x_col, bins));
    let y_bucket = format!("{} AS y_bin", equiwidth_bucket(table, y_col, bins));
    let count_expr = format!("CAST(COUNT(*) AS DOUBLE) AS count");

    QueryPlan::Aggregation {
        input: Box::new(QueryPlan::Filter {
            input: Box::new(QueryPlan::Source {
                table: table.to_string(),
            }),
            predicate: Predicate::Expr(format!(
                "\"{x_col}\" IS NOT NULL AND \"{y_col}\" IS NOT NULL"
            )),
        }),
        group_by: vec![x_bucket, y_bucket],
        aggregates: vec![count_expr],
    }
}

/// Build the registry of mark lowerers.
///
/// Registers SimpleLowerer for Dot, Line, BarX, BarY; the statistical-mark
/// lowerers (RegressionLowerer, DensityLowerer) for regression and density
/// kinds.
/// Marks not listed here fall back to DefaultLowerer (unsupported).
pub fn default_lowerers() -> Vec<(MarkKind, Box<dyn MarkLower>)> {
    vec![
        (MarkKind::Dot, Box::new(SimpleLowerer)),
        (MarkKind::Line, Box::new(SimpleLowerer)),
        (MarkKind::AreaY, Box::new(SimpleLowerer)),
        (MarkKind::AreaX, Box::new(SimpleLowerer)),
        (MarkKind::RuleX, Box::new(SimpleLowerer)),
        (MarkKind::RuleY, Box::new(SimpleLowerer)),
        (MarkKind::Text, Box::new(SimpleLowerer)),
        (MarkKind::BarX, Box::new(SimpleLowerer)),
        (MarkKind::BarY, Box::new(SimpleLowerer)),
        (MarkKind::RegressionY, Box::new(RegressionLowerer)),
        (MarkKind::RegressionX, Box::new(RegressionLowerer)),
        (
            MarkKind::DensityX,
            Box::new(DensityLowerer {
                kind: DensityLowerKind::OneDX,
            }),
        ),
        (
            MarkKind::DensityY,
            Box::new(DensityLowerer {
                kind: DensityLowerKind::OneDY,
            }),
        ),
        (
            MarkKind::Density,
            Box::new(DensityLowerer {
                kind: DensityLowerKind::TwoD,
            }),
        ),
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
        assert!(kinds.contains(&MarkKind::AreaY));
        assert!(kinds.contains(&MarkKind::AreaX));
        assert!(kinds.contains(&MarkKind::RuleX));
        assert!(kinds.contains(&MarkKind::RuleY));
        assert!(kinds.contains(&MarkKind::Text));
        assert!(kinds.contains(&MarkKind::BarX));
        assert!(kinds.contains(&MarkKind::BarY));
        assert!(kinds.contains(&MarkKind::RegressionY));
        assert!(kinds.contains(&MarkKind::RegressionX));
        assert!(kinds.contains(&MarkKind::DensityX));
        assert!(kinds.contains(&MarkKind::DensityY));
        assert!(kinds.contains(&MarkKind::Density));
        assert_eq!(kinds.len(), 14);
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

    // -----------------------------------------------------------------------
    // gomb ac-06 / ac-07 — statistical-mark lowerers
    // -----------------------------------------------------------------------

    fn make_mark_with_options(
        kind: MarkKind,
        opts: Vec<(&str, SpecValue)>,
    ) -> Mark {
        let mut options: IndexMap<String, ValueOrParamRef<SpecValue>> = IndexMap::new();
        for (k, v) in opts {
            options.insert(k.to_string(), ValueOrParamRef::Value(v));
        }
        Mark {
            kind,
            status: ImplStatus::Unimplemented,
            data: Some(MarkData::From {
                source: "athletes".to_string(),
                filter_by: None,
                extras: IndexMap::new(),
            }),
            options,
        }
    }

    #[test]
    fn gomb_ac06_regression_lowerer_emits_aggregate_scalar() {
        let mark = make_mark_with_options(
            MarkKind::RegressionY,
            vec![
                ("x", SpecValue::String("weight".to_string())),
                ("y", SpecValue::String("height".to_string())),
            ],
        );
        let ctx = make_ctx();
        let plan = RegressionLowerer.lower(&mark, &ctx).expect("lowers");
        match plan {
            QueryPlan::AggregateScalar { aggregates, .. } => {
                assert!(aggregates.iter().any(|a| a.contains("regr_slope")));
                assert!(aggregates.iter().any(|a| a.contains("regr_intercept")));
                assert!(aggregates.iter().any(|a| a.contains("regr_count")));
                assert!(aggregates.iter().any(|a| a.contains("regr_avgx")));
                assert!(aggregates.iter().any(|a| a.contains("regr_sxx")));
                assert!(aggregates.iter().any(|a| a.contains("regr_sxy")));
                assert!(aggregates.iter().any(|a| a.contains("regr_syy")));
            }
            other => panic!("expected AggregateScalar, got {other:?}"),
        }
    }

    #[test]
    fn gomb_ac06_regression_lowerer_with_stroke_groups_by() {
        let mark = make_mark_with_options(
            MarkKind::RegressionY,
            vec![
                ("x", SpecValue::String("weight".to_string())),
                ("y", SpecValue::String("height".to_string())),
                ("stroke", SpecValue::String("sport".to_string())),
            ],
        );
        let ctx = make_ctx();
        let plan = RegressionLowerer.lower(&mark, &ctx).expect("lowers");
        match plan {
            QueryPlan::Aggregation { group_by, .. } => {
                assert_eq!(group_by.len(), 1);
                assert!(group_by[0].contains("sport"));
            }
            other => panic!("expected grouped Aggregation, got {other:?}"),
        }
    }

    #[test]
    fn gomb_ac06_regression_lowerer_rejects_polynomial() {
        let mark = make_mark_with_options(
            MarkKind::RegressionY,
            vec![
                ("x", SpecValue::String("a".to_string())),
                ("y", SpecValue::String("b".to_string())),
                ("type", SpecValue::String("polynomial".to_string())),
            ],
        );
        let ctx = make_ctx();
        let result = RegressionLowerer.lower(&mark, &ctx);
        match result {
            Err(EmitError::UnsupportedMark { kind }) => {
                assert!(kind.contains("polynomial"));
            }
            other => panic!("expected UnsupportedMark with polynomial, got {other:?}"),
        }
    }

    #[test]
    fn gomb_ac07_density_lowerer_1d_x_uses_equiwidth_bucket() {
        let mark = make_mark_with_options(
            MarkKind::DensityX,
            vec![("x", SpecValue::String("weight".to_string()))],
        );
        let ctx = make_ctx();
        let plan = DensityLowerer {
            kind: DensityLowerKind::OneDX,
        }
        .lower(&mark, &ctx)
        .expect("lowers");
        match plan {
            QueryPlan::Aggregation {
                group_by,
                aggregates,
                ..
            } => {
                assert_eq!(group_by.len(), 1);
                // Portable equiwidth binning (no width_bucket — see follow-up #4).
                assert!(group_by[0].contains("floor"));
                assert!(group_by[0].contains("x_bin"));
                assert_eq!(aggregates.len(), 1);
                assert!(aggregates[0].contains("COUNT"));
            }
            other => panic!("expected Aggregation, got {other:?}"),
        }
    }

    #[test]
    fn gomb_ac07_density_lowerer_2d_uses_two_buckets() {
        let mark = make_mark_with_options(
            MarkKind::Density,
            vec![
                ("x", SpecValue::String("weight".to_string())),
                ("y", SpecValue::String("height".to_string())),
                ("thresholds", SpecValue::Integer(16)),
            ],
        );
        let ctx = make_ctx();
        let plan = DensityLowerer {
            kind: DensityLowerKind::TwoD,
        }
        .lower(&mark, &ctx)
        .expect("lowers");
        match plan {
            QueryPlan::Aggregation { group_by, .. } => {
                assert_eq!(group_by.len(), 2);
                assert!(group_by[0].contains("x_bin"));
                assert!(group_by[1].contains("y_bin"));
                // honours thresholds=16 in the bucket count
                assert!(group_by[0].contains("16") || group_by[1].contains("16"));
            }
            other => panic!("expected 2D Aggregation, got {other:?}"),
        }
    }

    #[test]
    fn gomb_ac06_default_lowerers_includes_statistical_kinds() {
        let registry = default_lowerers();
        let kinds: Vec<MarkKind> = registry.iter().map(|(k, _)| *k).collect();
        assert!(kinds.contains(&MarkKind::RegressionY));
        assert!(kinds.contains(&MarkKind::RegressionX));
        assert!(kinds.contains(&MarkKind::Density));
        assert!(kinds.contains(&MarkKind::DensityX));
        assert!(kinds.contains(&MarkKind::DensityY));
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
