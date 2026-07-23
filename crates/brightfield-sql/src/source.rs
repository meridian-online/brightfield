//! Per-`DataSourceKind` dispatch functions for DDL emission.

use std::path::Path;

use brightfield_spec::ast::SpecValue;
use indexmap::IndexMap;

use crate::emit::{self, SourceDdl, SourceKindTag};
use crate::error::EmitError;

/// Emit DDL for a `DataSourceKind::File` source, dispatching by extension.
pub(crate) fn emit_file(
    name: &str,
    file_value: &str,
    extras: &IndexMap<String, SpecValue>,
    base_dir: Option<&Path>,
) -> Result<SourceDdl, EmitError> {
    // Check if extras has type: spatial — that takes precedence
    if let Some(SpecValue::String(type_name)) = extras.get("type") {
        if type_name == "spatial" {
            return emit_spatial(name, file_value, extras, base_dir);
        }
    }

    let ext = emit::file_extension(file_value).unwrap_or_default();
    let resolved = emit::resolve_path(file_value, base_dir);

    // A DuckLake catalog — the `ducklake:` URI form (per the DuckDB docs)
    // or a bare `.ducklake` metadata file — attaches read-only. Checked
    // before extension dispatch: a `ducklake:…` URI's "extension" is
    // whatever follows the last dot of its inner location.
    if file_value.starts_with("ducklake:") || ext == "ducklake" {
        return Ok(emit_ducklake_attach(name, &resolved));
    }

    match ext.as_str() {
        "parquet" => Ok(emit_parquet(name, &resolved)),
        "csv" | "tsv" => Ok(emit_csv(name, &resolved, extras)),
        "json" | "ndjson" => Ok(emit_json(name, &resolved)),
        "geojson" => Err(EmitError::UnknownFormat {
            path: file_value.to_string(),
            extension: ext,
        }),
        "duckdb" | "db" => Ok(emit_duckdb_attach(name, &resolved)),
        _ => Err(EmitError::UnknownFormat {
            path: file_value.to_string(),
            extension: ext,
        }),
    }
}

/// Emit DDL for a `DataSourceKind::Typed` source that also has a file: key.
pub(crate) fn emit_file_typed(
    name: &str,
    file_value: &str,
    type_name: &str,
    extras: &IndexMap<String, SpecValue>,
    base_dir: Option<&Path>,
) -> Result<SourceDdl, EmitError> {
    if type_name == "spatial" {
        return emit_spatial(name, file_value, extras, base_dir);
    }

    // For non-spatial typed sources, fall through to extension dispatch
    emit_file(name, file_value, extras, base_dir)
}

fn emit_parquet(name: &str, resolved_path: &str) -> SourceDdl {
    SourceDdl {
        view_name: name.to_string(),
        sql: format!(
            "CREATE OR REPLACE VIEW \"{}\" AS SELECT * FROM read_parquet('{}')",
            name, resolved_path
        ),
        source_kind: SourceKindTag::Parquet,
        remote_location: emit::remote_location(resolved_path),
    }
}

fn emit_csv(name: &str, resolved_path: &str, extras: &IndexMap<String, SpecValue>) -> SourceDdl {
    let mut kwargs = vec!["auto_detect=true".to_string()];
    kwargs.extend(emit::format_kwargs(extras, emit::CSV_ALLOW_LIST));

    SourceDdl {
        view_name: name.to_string(),
        sql: format!(
            "CREATE OR REPLACE VIEW \"{}\" AS SELECT * FROM read_csv('{}', {})",
            name,
            resolved_path,
            kwargs.join(", ")
        ),
        source_kind: SourceKindTag::Csv,
        remote_location: emit::remote_location(resolved_path),
    }
}

fn emit_json(name: &str, resolved_path: &str) -> SourceDdl {
    SourceDdl {
        view_name: name.to_string(),
        sql: format!(
            "CREATE OR REPLACE VIEW \"{}\" AS SELECT * FROM read_json_auto('{}', format='auto')",
            name, resolved_path
        ),
        source_kind: SourceKindTag::Json,
        remote_location: emit::remote_location(resolved_path),
    }
}

fn emit_spatial(
    name: &str,
    file_value: &str,
    extras: &IndexMap<String, SpecValue>,
    base_dir: Option<&Path>,
) -> Result<SourceDdl, EmitError> {
    let resolved = emit::resolve_path(file_value, base_dir);

    let layer_kwarg = if let Some(SpecValue::String(layer)) = extras.get("layer") {
        format!(", layer='{layer}'")
    } else {
        String::new()
    };

    Ok(SourceDdl {
        view_name: name.to_string(),
        sql: format!(
            "CREATE OR REPLACE VIEW \"{}\" AS SELECT * FROM ST_Read('{}'{})",
            name, resolved, layer_kwarg
        ),
        source_kind: SourceKindTag::Spatial,
        remote_location: emit::remote_location(&resolved),
    })
}

fn emit_duckdb_attach(name: &str, resolved_path: &str) -> SourceDdl {
    SourceDdl {
        view_name: name.to_string(),
        sql: format!("ATTACH '{}' AS \"{}\" (READ_ONLY)", resolved_path, name),
        source_kind: SourceKindTag::DuckDb,
        remote_location: emit::remote_location(resolved_path),
    }
}

/// `ATTACH 'ducklake:…' AS "name" (READ_ONLY)` — a DuckLake catalog. The
/// `ducklake:` prefix is added when the source was a bare `.ducklake`
/// metadata file. Read-only is not optional here: brightfield is a
/// consumer of published catalogs, never a writer.
fn emit_ducklake_attach(name: &str, resolved_path: &str) -> SourceDdl {
    let uri = if resolved_path.starts_with("ducklake:") {
        resolved_path.to_string()
    } else {
        format!("ducklake:{resolved_path}")
    };
    SourceDdl {
        view_name: name.to_string(),
        sql: format!("ATTACH '{}' AS \"{}\" (READ_ONLY)", uri, name),
        source_kind: SourceKindTag::DuckLake,
        remote_location: emit::remote_location(&uri),
    }
}

/// Emit DDL for inline row data via a VALUES clause.
pub(crate) fn emit_inline_rows(
    name: &str,
    rows: &[SpecValue],
    _extras: &IndexMap<String, SpecValue>,
) -> Result<SourceDdl, EmitError> {
    if rows.len() > 1000 {
        return Err(EmitError::InlineRowLimit { count: rows.len() });
    }

    if rows.is_empty() {
        return Err(EmitError::InvariantViolation {
            detail: format!("data source '{name}' has zero inline rows"),
        });
    }

    // Determine column names from the first row
    let (col_names, value_rows) = match &rows[0] {
        SpecValue::Object(first_row) => {
            // Object-per-row: column names from first row's keys (insertion order)
            let names: Vec<String> = first_row.keys().cloned().collect();
            let mut value_rows = Vec::with_capacity(rows.len());

            for row in rows {
                if let SpecValue::Object(obj) = row {
                    let vals: Vec<String> = names
                        .iter()
                        .map(|k| {
                            obj.get(k)
                                .map(emit::spec_value_to_sql_literal)
                                .unwrap_or_else(|| "NULL".to_string())
                        })
                        .collect();
                    value_rows.push(format!("({})", vals.join(", ")));
                } else {
                    return Err(EmitError::InvariantViolation {
                        detail: format!(
                            "data source '{name}': mixed row shapes (expected object, got other)"
                        ),
                    });
                }
            }

            (names, value_rows)
        }
        SpecValue::Array(first_arr) => {
            // Array-per-row: synthesise column names c0, c1, …
            let names: Vec<String> = (0..first_arr.len()).map(|i| format!("c{i}")).collect();
            let mut value_rows = Vec::with_capacity(rows.len());

            for row in rows {
                if let SpecValue::Array(arr) = row {
                    let vals: Vec<String> =
                        arr.iter().map(emit::spec_value_to_sql_literal).collect();
                    value_rows.push(format!("({})", vals.join(", ")));
                } else {
                    return Err(EmitError::InvariantViolation {
                        detail: format!(
                            "data source '{name}': mixed row shapes (expected array, got other)"
                        ),
                    });
                }
            }

            (names, value_rows)
        }
        _ => {
            return Err(EmitError::InvariantViolation {
                detail: format!(
                    "data source '{name}': inline rows must be objects or arrays, got scalar"
                ),
            });
        }
    };

    let quoted_cols: Vec<String> = col_names.iter().map(|c| format!("\"{c}\"")).collect();
    let values_clause = value_rows.join(", ");

    Ok(SourceDdl {
        view_name: name.to_string(),
        sql: format!(
            "CREATE OR REPLACE VIEW \"{}\" AS SELECT * FROM (VALUES {}) AS t({})",
            name,
            values_clause,
            quoted_cols.join(", ")
        ),
        source_kind: SourceKindTag::InlineRows,
        remote_location: None,
    })
}
