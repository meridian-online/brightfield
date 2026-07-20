//! Binding model for the query emitter.
//!
//! `EmittedQuery` is the output of `emit_query` — it carries the SQL string,
//! parameter bindings, and a structural plan hash for shape-cache keying.

use indexmap::IndexMap;

use brightfield_spec::ast::{ExpressionNode, SpecValue};

use crate::error::EmitError;

/// Type alias for a map of parameter names to their current values.
///
/// Used by `Interpolated` binding mode during selection re-emission.
pub type ParamValues = IndexMap<String, SpecValue>;

/// How a parameter slot is bound in the emitted SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// A scalar parameter bound as `?` — slider drag dispatches
    /// `execute(stmt, &[latest_values])`.
    Scalar {
        /// The parameter name (e.g. `"threshold"`).
        param: String,
        /// 0-indexed position of the `?` in the SQL string.
        position: usize,
    },
    /// A selection parameter — structural change triggers WHERE-clause rebuild.
    Selection {
        /// The parameter name.
        param: String,
    },
}

/// The result of emitting a query for a single mark.
#[derive(Debug, Clone)]
pub struct EmittedQuery {
    /// The full SQL statement.
    pub sql: String,
    /// Parameter bindings describing which positions are parameterised.
    pub bindings: Vec<Binding>,
    /// Structural hash of the `QueryPlan` — excludes bound param values.
    /// Populated by calling `plan.hash_structural()` after the pass pipeline.
    pub plan_hash: u64,
}

/// How parameter references are rendered in SQL.
#[derive(Debug, Clone)]
pub enum BindingMode<'a> {
    /// Emit `?` placeholders and record `Binding::Scalar`.
    Prepared,
    /// Emit literal values from the provided `ParamValues` map.
    /// Used during selection re-emission when current runtime values are known.
    Interpolated { values: &'a ParamValues },
}

/// Render an `ExpressionNode` to SQL, respecting the spans/params interleaving
/// invariant and the chosen binding mode.
///
/// # Errors
///
/// Returns `EmitError::InvariantViolation` if `spans.len() != params.len() + 1`.
pub fn expression_to_sql(
    expr: &ExpressionNode,
    bindings: &mut Vec<Binding>,
    mode: &BindingMode<'_>,
) -> Result<String, EmitError> {
    // Defensive check for constraint 8
    if expr.spans.len() != expr.params.len() + 1 {
        return Err(EmitError::InvariantViolation {
            detail: format!(
                "ExpressionNode invariant violated: spans.len()={} != params.len()+1={}",
                expr.spans.len(),
                expr.params.len() + 1
            ),
        });
    }

    let mut out = String::new();
    for (i, span) in expr.spans.iter().enumerate() {
        out.push_str(span);
        if let Some(param) = expr.params.get(i) {
            match mode {
                BindingMode::Prepared => {
                    let position = bindings.len();
                    bindings.push(Binding::Scalar {
                        param: param.0.clone(),
                        position,
                    });
                    out.push('?');
                }
                BindingMode::Interpolated { values } => {
                    if let Some(val) = values.get(&param.0) {
                        out.push_str(&crate::emit::spec_value_to_sql_literal(val));
                    } else {
                        // Missing value — fall back to param reference
                        out.push('$');
                        out.push_str(&param.0);
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::ast::ParamRef;

    fn make_expr(spans: &[&str], params: &[&str]) -> ExpressionNode {
        ExpressionNode {
            spans: spans.iter().map(|s| s.to_string()).collect(),
            params: params.iter().map(|p| ParamRef(p.to_string())).collect(),
        }
    }

    #[test]
    fn dfir_ac10_prepared_mode_emits_placeholders() {
        let expr = make_expr(&["x > ", " AND x < ", ""], &["lo", "hi"]);
        let mut bindings = Vec::new();
        let result = expression_to_sql(&expr, &mut bindings, &BindingMode::Prepared).unwrap();
        assert_eq!(result, "x > ? AND x < ?");
        assert_eq!(bindings.len(), 2);
        assert_eq!(
            bindings[0],
            Binding::Scalar {
                param: "lo".to_string(),
                position: 0,
            }
        );
        assert_eq!(
            bindings[1],
            Binding::Scalar {
                param: "hi".to_string(),
                position: 1,
            }
        );
    }

    #[test]
    fn dfir_ac10_interpolated_mode_emits_literals() {
        let expr = make_expr(&["x > ", " AND x < ", ""], &["lo", "hi"]);
        let mut values = ParamValues::new();
        values.insert("lo".to_string(), SpecValue::Integer(42));
        values.insert("hi".to_string(), SpecValue::Integer(100));
        let mut bindings = Vec::new();
        let mode = BindingMode::Interpolated { values: &values };
        let result = expression_to_sql(&expr, &mut bindings, &mode).unwrap();
        assert_eq!(result, "x > 42 AND x < 100");
        assert!(bindings.is_empty());
    }

    #[test]
    fn dfir_ac10_invariant_violation_on_bad_spans() {
        // 2 spans, 2 params — violates spans.len() == params.len() + 1
        let expr = make_expr(&["x > ", ""], &["lo", "hi"]);
        let mut bindings = Vec::new();
        let result = expression_to_sql(&expr, &mut bindings, &BindingMode::Prepared);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, EmitError::InvariantViolation { .. }),
            "expected InvariantViolation, got {err:?}"
        );
    }

    #[test]
    fn dfir_ac06_binding_scalar_fields() {
        let b = Binding::Scalar {
            param: "threshold".to_string(),
            position: 0,
        };
        assert!(format!("{b:?}").contains("threshold"));
    }

    #[test]
    fn dfir_ac06_binding_selection_fields() {
        let b = Binding::Selection {
            param: "brush".to_string(),
        };
        assert!(format!("{b:?}").contains("Selection"));
    }

    #[test]
    fn dfir_ac06_emitted_query_construction() {
        let eq = EmittedQuery {
            sql: "SELECT * FROM t".to_string(),
            bindings: vec![],
            plan_hash: 42,
        };
        assert_eq!(eq.plan_hash, 42);
        assert!(eq.bindings.is_empty());
    }
}
