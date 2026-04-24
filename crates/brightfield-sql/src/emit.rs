//! Public entry points: data-source DDL emission (card 0004) and per-mark
//! query emission (card 0003).

use std::path::Path;

use brightfield_spec::ast::{DataSourceKind, Spec, SpecValue};
use brightfield_spec::parse::ParseWarning;
use indexmap::IndexMap;

use brightfield_spec::ast::{Component, Mark};

use crate::binding::{Binding, EmittedQuery, ParamValues};
use crate::error::EmitError;
use crate::lower::{default_lowerers, find_lowerer, LowerCtx};
use crate::passes::apply_passes;
use crate::render::render_query;
use crate::source;

/// Tag identifying which dispatch arm produced a [`SourceDdl`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKindTag {
    Parquet,
    Csv,
    Json,
    Spatial,
    DuckDb,
    InlineRows,
    Query,
}

/// One emitted DDL statement per data source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDdl {
    /// The view name (or attach alias for DuckDB).
    pub view_name: String,
    /// The full SQL statement (CREATE OR REPLACE VIEW … or ATTACH …).
    pub sql: String,
    /// Which dispatch arm produced this.
    pub source_kind: SourceKindTag,
}

/// Result of a successful emission — DDL statements plus non-fatal warnings.
#[derive(Debug, Clone, Default)]
pub struct EmitOutput {
    /// The emitted DDL statements.
    pub statements: Vec<SourceDdl>,
    /// Non-fatal observations (e.g. unknown CSV extras).
    pub warnings: Vec<ParseWarning>,
}

/// Emit data-source DDL for every entry in `spec.data`.
///
/// This is a pure function — no I/O, no DuckDB connection. File paths are
/// resolved against `base_dir` (typically the spec file's parent directory).
/// If `base_dir` is `None`, relative paths are left as-is.
///
/// # Errors
///
/// Returns [`EmitError`] if any data source cannot be emitted (unknown format,
/// inline row limit exceeded, invariant violation).
pub fn emit_sources(
    spec: &Spec,
    base_dir: Option<&Path>,
) -> Result<EmitOutput, EmitError> {
    let mut results = Vec::with_capacity(spec.data.len());
    let mut warnings = Vec::new();

    for (name, data_source) in &spec.data {
        // Collect warnings for CSV unknown extras
        if matches!(&data_source.kind, DataSourceKind::File(f) if {
            let ext = file_extension(f).unwrap_or_default();
            ext == "csv" || ext == "tsv"
        }) {
            for key in data_source.extras.keys() {
                if !CSV_ALLOW_LIST.contains(&key.as_str())
                    && key != "type"
                    && key != "where"
                    && key != "select"
                {
                    warnings.push(ParseWarning::UnknownOption {
                        path: format!("data.{name}"),
                        key: key.clone(),
                    });
                }
            }
        }

        let ddl = match &data_source.kind {
            DataSourceKind::File(file_value) => {
                source::emit_file(name, file_value, &data_source.extras, base_dir)?
            }
            DataSourceKind::Query(sql) => SourceDdl {
                view_name: name.clone(),
                sql: format!("CREATE OR REPLACE VIEW \"{}\" AS {}", name, sql),
                source_kind: SourceKindTag::Query,
            },
            DataSourceKind::Shorthand(s) => SourceDdl {
                view_name: name.clone(),
                sql: format!("CREATE OR REPLACE VIEW \"{}\" AS {}", name, s),
                source_kind: SourceKindTag::Query,
            },
            DataSourceKind::InlineRows(rows) => {
                source::emit_inline_rows(name, rows, &data_source.extras)?
            }
            DataSourceKind::Typed(type_name) => {
                // Typed without a file: key — check if there's a file in extras
                if let Some(SpecValue::String(file_value)) = data_source.extras.get("file") {
                    source::emit_file_typed(name, file_value, type_name, &data_source.extras, base_dir)?
                } else {
                    return Err(EmitError::InvariantViolation {
                        detail: format!(
                            "data source '{}' has type '{}' but no file: key — cannot emit SQL",
                            name, type_name
                        ),
                    });
                }
            }
            DataSourceKind::Opaque => {
                return Err(EmitError::InvariantViolation {
                    detail: format!(
                        "data source '{}' is opaque (no recognised keys) — cannot emit SQL",
                        name
                    ),
                });
            }
        };
        results.push(ddl);
    }

    Ok(EmitOutput {
        statements: results,
        warnings,
    })
}

/// CSV option allow-list. Extras outside this list produce a warning, not an error.
pub(crate) const CSV_ALLOW_LIST: &[&str] = &[
    "columns", "delim", "header", "nullstr", "skip", "types",
];

/// Resolve a file path against a base directory.
///
/// HTTP(S) URLs pass through verbatim. Absolute paths pass through.
/// Relative paths are joined against `base_dir` if provided.
pub(crate) fn resolve_path(file_value: &str, base_dir: Option<&Path>) -> String {
    // HTTP(S) URLs pass through verbatim
    if file_value.starts_with("http://") || file_value.starts_with("https://") {
        return file_value.to_string();
    }

    let path = Path::new(file_value);

    // Absolute paths pass through
    if path.is_absolute() {
        return file_value.to_string();
    }

    // Relative paths: join against base_dir if provided
    match base_dir {
        Some(base) => base.join(path).to_string_lossy().into_owned(),
        None => file_value.to_string(),
    }
}

/// Extract the file extension from a path, lowercased.
pub(crate) fn file_extension(file_value: &str) -> Option<String> {
    Path::new(file_value)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Format kwargs from an extras map, filtering to an allow-list and sorting
/// alphabetically. Returns pairs suitable for appending to a function call.
pub(crate) fn format_kwargs(
    extras: &IndexMap<String, SpecValue>,
    allow_list: &[&str],
) -> Vec<String> {
    let mut kwargs: Vec<(String, String)> = Vec::new();

    for key in allow_list {
        if let Some(val) = extras.get(*key) {
            kwargs.push(((*key).to_string(), spec_value_to_sql_literal(val)));
        }
    }

    kwargs.sort_by(|a, b| a.0.cmp(&b.0));
    kwargs.into_iter().map(|(k, v)| format!("{k}={v}")).collect()
}

/// Convert a `SpecValue` to a SQL literal for use in kwargs.
pub(crate) fn spec_value_to_sql_literal(val: &SpecValue) -> String {
    match val {
        SpecValue::String(s) => format!("'{s}'"),
        SpecValue::Integer(n) => format!("{n}"),
        SpecValue::Float(f) => format!("{f}"),
        SpecValue::Bool(b) => format!("{b}"),
        SpecValue::Null => "NULL".to_string(),
        SpecValue::Array(arr) => {
            let items: Vec<String> = arr.iter().map(spec_value_to_sql_literal).collect();
            format!("[{}]", items.join(", "))
        }
        SpecValue::Object(map) => {
            // Objects as kwargs are unusual — render as a struct literal
            let pairs: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{k}: {}", spec_value_to_sql_literal(v)))
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
        // Param refs and expressions in kwargs are unusual — render as placeholders
        SpecValue::Param(p) => format!("${}", p.0),
        SpecValue::Expression(e) => {
            // Render expression as its raw text — interleave spans and params
            let mut out = String::new();
            for (i, span) in e.spans.iter().enumerate() {
                out.push_str(span);
                if let Some(p) = e.params.get(i) {
                    out.push('$');
                    out.push_str(&p.0);
                }
            }
            out
        }
    }
}

/// Collect all `Mark` nodes from the spec's component tree (depth-first).
pub fn collect_marks(spec: &Spec) -> Vec<&Mark> {
    let mut marks = Vec::new();
    if let Some(root) = &spec.root {
        collect_marks_from_component(root, &mut marks);
    }
    marks
}

fn collect_marks_from_component<'a>(component: &'a Component, marks: &mut Vec<&'a Mark>) {
    match component {
        Component::Plot(plot) => {
            for item in &plot.items {
                collect_marks_from_component(item, marks);
            }
        }
        Component::HConcat(concat) | Component::VConcat(concat) => {
            for child in &concat.items {
                collect_marks_from_component(child, marks);
            }
        }
        Component::Mark(mark) => marks.push(mark),
        // Legends, interactors, inputs, spacers don't contain marks
        _ => {}
    }
}

/// Emit a query for a single mark in the spec.
///
/// Deliberate refinement of interview D6's `fn emit(spec, preflight)` signature:
/// - `mark_index` enables per-mark emission (index into depth-first mark order)
/// - `param_values` enables D4's hybrid binding mode
/// - `SupportReport` is dropped because preflight is a separate upstream phase
///
/// `param_values` carries current runtime values for `Interpolated` rendering
/// (selection re-emission); `None` uses `Prepared` mode with `?` placeholders.
///
/// # Errors
///
/// Returns `EmitError::UnsupportedMark` for marks whose `MarkKind` has no
/// registered lowering (defence-in-depth; preflight should reject first).
pub fn emit_query(
    spec: &Spec,
    mark_index: usize,
    _param_values: Option<&ParamValues>,
) -> Result<EmittedQuery, EmitError> {
    let marks = collect_marks(spec);
    let mark = marks
        .get(mark_index)
        .ok_or_else(|| EmitError::InvariantViolation {
            detail: format!(
                "mark_index {} out of bounds (spec has {} marks)",
                mark_index,
                marks.len()
            ),
        })?;

    let lowerers = default_lowerers();
    let ctx = LowerCtx {
        data_sources: &spec.data,
        params: &spec.params,
    };

    let lowerer = find_lowerer(mark.kind, &lowerers);
    let plan = lowerer.lower(mark, &ctx)?;

    // Apply optimisation passes (empty in v1)
    let passes: Vec<Box<dyn crate::passes::Pass>> = vec![];
    let plan = apply_passes(plan, &passes);

    // Compute structural hash before rendering
    let plan_hash = plan.hash_structural();

    // Render to SQL
    let mut bindings: Vec<Binding> = Vec::new();
    let sql = render_query(&plan, &mut bindings);

    Ok(EmittedQuery {
        sql,
        bindings,
        plan_hash,
    })
}

/// Emit a query for a single mark, applying the given passes to the plan.
///
/// This is the pass-aware variant of [`emit_query`]. The engine layer uses
/// this to inject navigation filters (and future optimisation passes) into
/// the query plan before SQL rendering.
pub fn emit_query_with_passes(
    spec: &Spec,
    mark_index: usize,
    _param_values: Option<&ParamValues>,
    extra_passes: &[Box<dyn crate::passes::Pass>],
) -> Result<EmittedQuery, EmitError> {
    let marks = collect_marks(spec);
    let mark = marks
        .get(mark_index)
        .ok_or_else(|| EmitError::InvariantViolation {
            detail: format!(
                "mark_index {} out of bounds (spec has {} marks)",
                mark_index,
                marks.len()
            ),
        })?;

    let lowerers = default_lowerers();
    let ctx = LowerCtx {
        data_sources: &spec.data,
        params: &spec.params,
    };

    let lowerer = find_lowerer(mark.kind, &lowerers);
    let plan = lowerer.lower(mark, &ctx)?;

    // Apply optimisation passes (built-in + caller-provided).
    let plan = apply_passes(plan, extra_passes);

    let plan_hash = plan.hash_structural();
    let mut bindings: Vec<Binding> = Vec::new();
    let sql = render_query(&plan, &mut bindings);

    Ok(EmittedQuery {
        sql,
        bindings,
        plan_hash,
    })
}

/// Emit queries for all marks in the spec.
///
/// Returns one `Result` per mark. Unimplemented marks return
/// `Err(EmitError::UnsupportedMark)`.
pub fn emit_all_queries(
    spec: &Spec,
    param_values: Option<&ParamValues>,
) -> Vec<Result<EmittedQuery, EmitError>> {
    let marks = collect_marks(spec);
    (0..marks.len())
        .map(|i| emit_query(spec, i, param_values))
        .collect()
}

#[cfg(test)]
mod query_tests {
    use super::*;
    use brightfield_spec::{parse_spec, Format};

    #[test]
    fn dfir_ac08_emit_query_unsupported_mark() {
        // Use a mark kind that SimpleLowerer is NOT registered for
        let src = "plot:\n  - mark: rect\n    data: { from: flights }\ndata:\n  flights: { file: flights.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 0, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            EmitError::UnsupportedMark { kind } => assert_eq!(kind, "rect"),
            other => panic!("expected UnsupportedMark, got {other:?}"),
        }
    }

    #[test]
    fn msv_ac01_emit_query_succeeds_for_from_data() {
        // SimpleLowerer handles line marks with data.from
        let src = "plot:\n  - mark: line\n    data: { from: flights }\ndata:\n  flights: { file: flights.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 0, None);
        assert!(result.is_ok(), "SimpleLowerer should handle line+data.from");
        let emitted = result.unwrap();
        assert!(
            emitted.sql.contains("flights"),
            "emitted SQL should reference the source table"
        );
    }

    #[test]
    fn dfir_ac08_emit_query_out_of_bounds() {
        let src = "plot:\n  - mark: dot\n    data: { from: t }\ndata:\n  t: { file: t.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 99, None);
        assert!(matches!(
            result,
            Err(EmitError::InvariantViolation { .. })
        ));
    }

    #[test]
    fn dfir_ac08_emit_all_queries_returns_per_mark_results() {
        let src = "plot:\n  - mark: line\n    data: { from: t }\n  - mark: dot\n    data: { from: t }\ndata:\n  t: { file: t.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let results = emit_all_queries(&spec, None);
        assert_eq!(results.len(), 2, "should have one result per mark");
        // SimpleLowerer handles both line and dot with data.from
        for result in &results {
            assert!(result.is_ok(), "line and dot with data.from should succeed via SimpleLowerer");
        }
    }

    #[test]
    fn dfir_ac08_emit_query_no_marks_spec() {
        // A spec with no marks — just data
        let src = "data:\n  t: { file: t.parquet }\n";
        let spec = parse_spec(src, Format::Yaml).unwrap().spec;
        let result = emit_query(&spec, 0, None);
        assert!(matches!(
            result,
            Err(EmitError::InvariantViolation { .. })
        ));
    }
}
