//! Public entry point and the `SourceDdl` output type.

use std::path::Path;

use brightfield_spec::ast::{DataSourceKind, Spec, SpecValue};
use brightfield_spec::parse::ParseWarning;
use indexmap::IndexMap;

use crate::error::EmitError;
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
