//! Unit tests for the brightfield-sql emitter — covers ac-02 through ac-10, ac-17.

use brightfield_spec::ast::{DataSource, DataSourceKind, Spec, SpecValue};
use brightfield_sql::emit::{emit_sources, SourceKindTag};
use brightfield_sql::error::EmitError;
use indexmap::IndexMap;
use std::path::Path;

/// Helper: build a Spec with a single data source.
fn spec_with_source(name: &str, kind: DataSourceKind, extras: IndexMap<String, SpecValue>) -> Spec {
    let mut data = IndexMap::new();
    data.insert(
        name.to_string(),
        DataSource {
            kind,
            extras,
        },
    );
    Spec {
        data,
        ..Spec::default()
    }
}

// ── ac-02: EmitError variants ──────────────────────────────────────────────

#[test]
fn dfsql_emit_error_variants() {
    let e1 = EmitError::UnknownFormat {
        path: "data.xlsx".to_string(),
        extension: "xlsx".to_string(),
    };
    assert!(e1.to_string().contains("xlsx"), "Display should mention extension");
    assert!(e1.to_string().contains("data.xlsx"), "Display should mention path");

    let e2 = EmitError::InlineRowLimit { count: 1500 };
    assert!(e2.to_string().contains("1500"), "Display should mention count");

    let e3 = EmitError::InvariantViolation {
        detail: "test detail".to_string(),
    };
    assert!(e3.to_string().contains("test detail"), "Display should mention detail");
}

// ── ac-04: emit_sources entry point ────────────────────────────────────────

#[test]
fn dfsql_emit_sources_returns_one_ddl_per_data_entry() {
    let mut data = IndexMap::new();
    data.insert("flights".to_string(), DataSource {
        kind: DataSourceKind::File("flights.parquet".to_string()),
        extras: IndexMap::new(),
    });
    data.insert("weather".to_string(), DataSource {
        kind: DataSourceKind::File("weather.csv".to_string()),
        extras: IndexMap::new(),
    });
    data.insert("src".to_string(), DataSource {
        kind: DataSourceKind::Query("SELECT * FROM t".to_string()),
        extras: IndexMap::new(),
    });

    let spec = Spec { data, ..Spec::default() };
    let output = emit_sources(&spec, None).unwrap();
    assert_eq!(output.statements.len(), 3);
    assert_eq!(output.statements[0].view_name, "flights");
    assert_eq!(output.statements[1].view_name, "weather");
    assert_eq!(output.statements[2].view_name, "src");
}

// ── ac-05: Parquet emission ────────────────────────────────────────────────

#[test]
fn dfsql_parquet_emission() {
    let spec = spec_with_source(
        "flights",
        DataSourceKind::File("flights.parquet".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, Some(Path::new("/data"))).unwrap();
    assert_eq!(output.statements.len(), 1);
    assert_eq!(output.statements[0].source_kind, SourceKindTag::Parquet);
    assert!(output.statements[0].sql.contains("read_parquet('/data/flights.parquet')"));
}

#[test]
fn dfsql_parquet_http_url_passthrough() {
    let spec = spec_with_source(
        "remote",
        DataSourceKind::File("https://example.com/data.parquet".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, Some(Path::new("/data"))).unwrap();
    assert!(output.statements[0].sql.contains("https://example.com/data.parquet"));
}

// ── ac-06: CSV emission ────────────────────────────────────────────────────

#[test]
fn dfsql_csv_emission_with_extras() {
    let mut extras = IndexMap::new();
    extras.insert("delim".to_string(), SpecValue::String("|".to_string()));
    extras.insert("skip".to_string(), SpecValue::Integer(1));

    let spec = spec_with_source(
        "data",
        DataSourceKind::File("data.csv".to_string()),
        extras,
    );
    let output = emit_sources(&spec, None).unwrap();
    assert_eq!(output.statements[0].source_kind, SourceKindTag::Csv);
    assert!(output.statements[0].sql.contains("auto_detect=true"));
    assert!(output.statements[0].sql.contains("delim='|'"));
    assert!(output.statements[0].sql.contains("skip=1"));
}

#[test]
fn dfsql_csv_unknown_extra_warns() {
    // Unknown extras like 'encoding' produce a ParseWarning::UnknownOption
    let mut extras = IndexMap::new();
    extras.insert("encoding".to_string(), SpecValue::String("utf8".to_string()));

    let spec = spec_with_source(
        "data",
        DataSourceKind::File("data.csv".to_string()),
        extras,
    );
    let output = emit_sources(&spec, None).unwrap();
    // encoding should NOT appear in the SQL
    assert!(!output.statements[0].sql.contains("encoding"));
    // But a warning should be emitted
    assert_eq!(output.warnings.len(), 1, "Expected 1 warning for unknown CSV extra");
}

// ── ac-07: JSON emission ───────────────────────────────────────────────────

#[test]
fn dfsql_json_emission() {
    let spec = spec_with_source(
        "events",
        DataSourceKind::File("events.json".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, None).unwrap();
    assert_eq!(output.statements[0].source_kind, SourceKindTag::Json);
    assert!(output.statements[0].sql.contains("read_json_auto("));
}

#[test]
fn dfsql_geojson_requires_spatial_type() {
    let spec = spec_with_source(
        "geo",
        DataSourceKind::File("areas.geojson".to_string()),
        IndexMap::new(),
    );
    let result = emit_sources(&spec, None);
    assert!(result.is_err());
    match result.unwrap_err() {
        EmitError::UnknownFormat { extension, .. } => {
            assert_eq!(extension, "geojson");
        }
        other => panic!("Expected UnknownFormat, got: {other}"),
    }
}

#[test]
fn dfsql_spatial_emission() {
    let mut extras = IndexMap::new();
    extras.insert("type".to_string(), SpecValue::String("spatial".to_string()));

    let spec = spec_with_source(
        "geo",
        DataSourceKind::File("areas.geojson".to_string()),
        extras,
    );
    let output = emit_sources(&spec, None).unwrap();
    assert_eq!(output.statements[0].source_kind, SourceKindTag::Spatial);
    assert!(output.statements[0].sql.contains("ST_Read("));
}

// ── ac-08: DuckDB ATTACH ──────────────────────────────────────────────────

#[test]
fn dfsql_duckdb_attach_emission() {
    let spec = spec_with_source(
        "analytics",
        DataSourceKind::File("analytics.duckdb".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, Some(Path::new("/data"))).unwrap();
    assert_eq!(output.statements[0].source_kind, SourceKindTag::DuckDb);
    assert!(output.statements[0].sql.contains("ATTACH"));
    assert!(output.statements[0].sql.contains("AS \"analytics\""));
}

#[test]
fn dfsql_duckdb_read_only_enforced() {
    let spec = spec_with_source(
        "db",
        DataSourceKind::File("my.db".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, None).unwrap();
    assert!(output.statements[0].sql.contains("READ_ONLY"));
}

// ── ac-09: Inline rows ────────────────────────────────────────────────────

#[test]
fn dfsql_inline_object_rows() {
    let rows = vec![
        SpecValue::Object({
            let mut m = IndexMap::new();
            m.insert("x".to_string(), SpecValue::Integer(1));
            m.insert("y".to_string(), SpecValue::Integer(2));
            m
        }),
        SpecValue::Object({
            let mut m = IndexMap::new();
            m.insert("x".to_string(), SpecValue::Integer(3));
            m.insert("y".to_string(), SpecValue::Integer(4));
            m
        }),
        SpecValue::Object({
            let mut m = IndexMap::new();
            m.insert("x".to_string(), SpecValue::Integer(5));
            m.insert("y".to_string(), SpecValue::Integer(6));
            m
        }),
    ];

    let spec = spec_with_source("points", DataSourceKind::InlineRows(rows), IndexMap::new());
    let output = emit_sources(&spec, None).unwrap();
    assert_eq!(output.statements[0].source_kind, SourceKindTag::InlineRows);
    assert!(output.statements[0].sql.contains("VALUES"));
    assert!(output.statements[0].sql.contains("\"x\""));
    assert!(output.statements[0].sql.contains("\"y\""));
    assert!(output.statements[0].sql.contains("(1, 2)"));
}

#[test]
fn dfsql_inline_array_rows() {
    let rows = vec![
        SpecValue::Array(vec![SpecValue::Integer(1), SpecValue::String("a".to_string())]),
        SpecValue::Array(vec![SpecValue::Integer(2), SpecValue::String("b".to_string())]),
    ];

    let spec = spec_with_source("data", DataSourceKind::InlineRows(rows), IndexMap::new());
    let output = emit_sources(&spec, None).unwrap();
    assert!(output.statements[0].sql.contains("\"c0\""));
    assert!(output.statements[0].sql.contains("\"c1\""));
}

#[test]
fn dfsql_inline_row_limit() {
    let rows: Vec<SpecValue> = (0..1001)
        .map(|i| {
            SpecValue::Object({
                let mut m = IndexMap::new();
                m.insert("v".to_string(), SpecValue::Integer(i));
                m
            })
        })
        .collect();

    let spec = spec_with_source("big", DataSourceKind::InlineRows(rows), IndexMap::new());
    let result = emit_sources(&spec, None);
    assert!(result.is_err());
    match result.unwrap_err() {
        EmitError::InlineRowLimit { count } => assert_eq!(count, 1001),
        other => panic!("Expected InlineRowLimit, got: {other}"),
    }
}

// ── ac-10: Query and Shorthand emission ────────────────────────────────────

#[test]
fn dfsql_query_emission() {
    let spec = spec_with_source(
        "src",
        DataSourceKind::Query("SELECT * FROM t".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, None).unwrap();
    assert_eq!(output.statements[0].source_kind, SourceKindTag::Query);
    assert!(output.statements[0].sql.contains("CREATE OR REPLACE VIEW \"src\" AS SELECT * FROM t"));
}

#[test]
fn dfsql_shorthand_emission() {
    let spec = spec_with_source(
        "src",
        DataSourceKind::Shorthand("my_table".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, None).unwrap();
    assert_eq!(output.statements[0].source_kind, SourceKindTag::Query);
    assert!(output.statements[0].sql.contains("CREATE OR REPLACE VIEW \"src\" AS my_table"));
}

// ── ac-17: Error-path dispatch ─────────────────────────────────────────────

#[test]
fn dfsql_typed_without_file_is_invariant_violation() {
    let spec = spec_with_source(
        "geo",
        DataSourceKind::Typed("spatial".to_string()),
        IndexMap::new(),
    );
    let result = emit_sources(&spec, None);
    assert!(result.is_err());
    match result.unwrap_err() {
        EmitError::InvariantViolation { .. } => {}
        other => panic!("Expected InvariantViolation, got: {other}"),
    }
}

#[test]
fn dfsql_opaque_is_invariant_violation() {
    let spec = spec_with_source("mystery", DataSourceKind::Opaque, IndexMap::new());
    let result = emit_sources(&spec, None);
    assert!(result.is_err());
    match result.unwrap_err() {
        EmitError::InvariantViolation { .. } => {}
        other => panic!("Expected InvariantViolation, got: {other}"),
    }
}

#[test]
fn dfsql_unknown_extension_errors() {
    let spec = spec_with_source(
        "data",
        DataSourceKind::File("data.xlsx".to_string()),
        IndexMap::new(),
    );
    let result = emit_sources(&spec, None);
    assert!(result.is_err());
    match result.unwrap_err() {
        EmitError::UnknownFormat { extension, .. } => {
            assert_eq!(extension, "xlsx");
        }
        other => panic!("Expected UnknownFormat, got: {other}"),
    }
}
