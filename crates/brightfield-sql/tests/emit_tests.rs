//! Unit tests for the brightfield-sql emitter.

use brightfield_spec::ast::{DataSource, DataSourceKind, Spec, SpecValue};
use brightfield_sql::emit::{emit_sources, SourceKindTag};
use brightfield_sql::error::EmitError;
use indexmap::IndexMap;
use std::path::Path;

/// Helper: build a Spec with a single data source.
fn spec_with_source(name: &str, kind: DataSourceKind, extras: IndexMap<String, SpecValue>) -> Spec {
    let mut data = IndexMap::new();
    data.insert(name.to_string(), DataSource { kind, extras });
    Spec {
        data,
        ..Spec::default()
    }
}

// ── EmitError variants ──────────────────────────────────────────────

#[test]
fn dfsql_emit_error_variants() {
    let e1 = EmitError::UnknownFormat {
        path: "data.xlsx".to_string(),
        extension: "xlsx".to_string(),
    };
    assert!(
        e1.to_string().contains("xlsx"),
        "Display should mention extension"
    );
    assert!(
        e1.to_string().contains("data.xlsx"),
        "Display should mention path"
    );

    let e2 = EmitError::InlineRowLimit { count: 1500 };
    assert!(
        e2.to_string().contains("1500"),
        "Display should mention count"
    );

    let e3 = EmitError::InvariantViolation {
        detail: "test detail".to_string(),
    };
    assert!(
        e3.to_string().contains("test detail"),
        "Display should mention detail"
    );
}

// ── emit_sources entry point ────────────────────────────────────────

#[test]
fn dfsql_emit_sources_returns_one_ddl_per_data_entry() {
    let mut data = IndexMap::new();
    data.insert(
        "flights".to_string(),
        DataSource {
            kind: DataSourceKind::File("flights.parquet".to_string()),
            extras: IndexMap::new(),
        },
    );
    data.insert(
        "weather".to_string(),
        DataSource {
            kind: DataSourceKind::File("weather.csv".to_string()),
            extras: IndexMap::new(),
        },
    );
    data.insert(
        "src".to_string(),
        DataSource {
            kind: DataSourceKind::Query("SELECT * FROM t".to_string()),
            extras: IndexMap::new(),
        },
    );

    let spec = Spec {
        data,
        ..Spec::default()
    };
    let output = emit_sources(&spec, None).unwrap();
    assert_eq!(output.statements.len(), 3);
    assert_eq!(output.statements[0].view_name, "flights");
    assert_eq!(output.statements[1].view_name, "weather");
    assert_eq!(output.statements[2].view_name, "src");
}

// ── Parquet emission ────────────────────────────────────────────────

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
    assert!(output.statements[0]
        .sql
        .contains("read_parquet('/data/flights.parquet')"));
}

#[test]
fn dfsql_parquet_http_url_passthrough() {
    let spec = spec_with_source(
        "remote",
        DataSourceKind::File("https://example.com/data.parquet".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, Some(Path::new("/data"))).unwrap();
    assert!(output.statements[0]
        .sql
        .contains("https://example.com/data.parquet"));
}

/// A path carrying an apostrophe is escaped into the reader call rather than
/// closing its string literal early.
///
/// `spec_value_to_sql_literal` had escaped kwarg strings since it was written;
/// the path beside them had not, so `/data/Hugh's.parquet` emitted
/// `read_parquet('/data/Hugh's.parquet')` — a syntax error at best, and at
/// worst SQL of the author's choosing. It stopped being hypothetical when a
/// file dialog reached the front door: the path is now whatever the operating
/// system handed back, and `Hugh's data.csv` is an ordinary name for a file on
/// a Mac.
///
/// Every reader this crate emits is checked, because the escaping is per-call
/// site and one site left raw is the whole hole.
#[test]
fn dfsql_a_quote_in_a_path_is_escaped_not_left_to_close_the_literal() {
    for (file, needle) in [
        ("Hugh's.parquet", "read_parquet('/data/Hugh''s.parquet')"),
        ("Hugh's.csv", "read_csv('/data/Hugh''s.csv'"),
        ("Hugh's.json", "read_json_auto('/data/Hugh''s.json'"),
        ("Hugh's.duckdb", "ATTACH '/data/Hugh''s.duckdb'"),
        (
            "Hugh's.ducklake",
            "ATTACH 'ducklake:/data/Hugh''s.ducklake'",
        ),
    ] {
        let spec = spec_with_source(
            "src",
            DataSourceKind::File(file.to_string()),
            IndexMap::new(),
        );
        let output = emit_sources(&spec, Some(Path::new("/data"))).unwrap();
        let sql = &output.statements[0].sql;
        assert!(sql.contains(needle), "{file} emitted {sql}");
        // …and the raw form is gone entirely, so a partial escape cannot pass.
        assert!(
            !sql.contains("Hugh's"),
            "{file} left an unescaped quote in {sql}"
        );
    }
}

/// The spatial reader's `layer:` kwarg travels the same path as the file it
/// sits beside, and was interpolated raw for the same reason.
#[test]
fn dfsql_a_quote_in_a_spatial_layer_is_escaped() {
    let mut extras = IndexMap::new();
    extras.insert("type".to_string(), SpecValue::String("spatial".to_string()));
    extras.insert(
        "layer".to_string(),
        SpecValue::String("Hugh's layer".to_string()),
    );
    let spec = spec_with_source(
        "shapes",
        DataSourceKind::File("shapes.gpkg".to_string()),
        extras,
    );
    let output = emit_sources(&spec, Some(Path::new("/data"))).unwrap();
    let sql = &output.statements[0].sql;
    assert!(sql.contains("layer='Hugh''s layer'"), "{sql}");
}

// ── CSV emission ────────────────────────────────────────────────────

#[test]
fn dfsql_csv_emission_with_extras() {
    let mut extras = IndexMap::new();
    extras.insert("delim".to_string(), SpecValue::String("|".to_string()));
    extras.insert("skip".to_string(), SpecValue::Integer(1));

    let spec = spec_with_source("data", DataSourceKind::File("data.csv".to_string()), extras);
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
    extras.insert(
        "encoding".to_string(),
        SpecValue::String("utf8".to_string()),
    );

    let spec = spec_with_source("data", DataSourceKind::File("data.csv".to_string()), extras);
    let output = emit_sources(&spec, None).unwrap();
    // encoding should NOT appear in the SQL
    assert!(!output.statements[0].sql.contains("encoding"));
    // But a warning should be emitted
    assert_eq!(
        output.warnings.len(),
        1,
        "Expected 1 warning for unknown CSV extra"
    );
}

// ── JSON emission ───────────────────────────────────────────────────

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

// ── DuckDB ATTACH ──────────────────────────────────────────────────

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

// ── Inline rows ────────────────────────────────────────────────────

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
        SpecValue::Array(vec![
            SpecValue::Integer(1),
            SpecValue::String("a".to_string()),
        ]),
        SpecValue::Array(vec![
            SpecValue::Integer(2),
            SpecValue::String("b".to_string()),
        ]),
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

// ── Query and Shorthand emission ────────────────────────────────────

#[test]
fn dfsql_query_emission() {
    let spec = spec_with_source(
        "src",
        DataSourceKind::Query("SELECT * FROM t".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, None).unwrap();
    assert_eq!(output.statements[0].source_kind, SourceKindTag::Query);
    assert!(output.statements[0]
        .sql
        .contains("CREATE OR REPLACE VIEW \"src\" AS SELECT * FROM t"));
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
    assert!(output.statements[0]
        .sql
        .contains("CREATE OR REPLACE VIEW \"src\" AS my_table"));
}

// ── Error-path dispatch ─────────────────────────────────────────────

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

// ── multi-view: per-plot mark grouping ──────────────────────────────────────

#[test]
fn mvdash_collect_plot_groups_hconcat_two_plots() {
    use brightfield_spec::parse::Format;
    use brightfield_spec::parse_spec;
    use brightfield_sql::{collect_marks, collect_plot_groups};

    // hconcat of two plots: the first owns two marks, the second one.
    let yaml = r#"
data:
  t:
    - { x: 1, y: 2 }
hconcat:
  - plot:
      - { mark: dot, data: { from: t }, x: x, y: y }
      - { mark: line, data: { from: t }, x: x, y: y }
  - plot:
      - { mark: barY, data: { from: t }, x: x, y: y }
"#;
    let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
    let groups = collect_plot_groups(&parsed.spec);

    assert_eq!(groups.len(), 2, "two plots → two groups");
    // Marks in the SAME plot stay grouped (the per-mark plot[i] segment is an
    // item index, not the plot's identity).
    assert_eq!(
        groups[0].mark_indices,
        vec![0, 1],
        "first plot owns marks 0,1"
    );
    assert_eq!(groups[1].mark_indices, vec![2], "second plot owns mark 2");
    // Plot path = the plot node's container path (joins to the layout tree).
    assert_eq!(groups[0].plot_path, "root/hconcat[0]");
    assert_eq!(groups[1].plot_path, "root/hconcat[1]");
    // Indices align with the flat execute order.
    assert_eq!(collect_marks(&parsed.spec).len(), 3);
}

#[test]
fn mvdash_collect_plot_groups_single_top_level_plot() {
    use brightfield_spec::parse::Format;
    use brightfield_spec::parse_spec;
    use brightfield_sql::collect_plot_groups;

    let yaml = r#"
data:
  t:
    - { x: 1, y: 2 }
plot:
  - { mark: dot, data: { from: t }, x: x, y: y }
  - { mark: line, data: { from: t }, x: x, y: y }
"#;
    let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
    let groups = collect_plot_groups(&parsed.spec);

    assert_eq!(groups.len(), 1, "single top-level plot → one group");
    assert_eq!(groups[0].mark_indices, vec![0, 1]);
    assert_eq!(groups[0].plot_path, "root");
}

// ── Remote locations + DuckLake catalog attach ─────────────────────

#[test]
fn dfsql_remote_location_set_for_https_parquet_and_csv() {
    for file in [
        "https://example.com/data.parquet",
        "http://example.com/d.csv",
    ] {
        let spec = spec_with_source(
            "remote",
            DataSourceKind::File(file.to_string()),
            IndexMap::new(),
        );
        let output = emit_sources(&spec, Some(Path::new("/data"))).unwrap();
        assert_eq!(
            output.statements[0].remote_location.as_deref(),
            Some(file),
            "an http(s) URL is a remote location"
        );
    }
}

#[test]
fn dfsql_remote_location_none_for_local_inline_and_query() {
    let local = spec_with_source(
        "t",
        DataSourceKind::File("flights.parquet".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&local, Some(Path::new("/data"))).unwrap();
    assert_eq!(
        output.statements[0].remote_location, None,
        "a local file is not remote"
    );

    let query = spec_with_source(
        "q",
        DataSourceKind::Query("SELECT 1 AS x".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&query, None).unwrap();
    assert_eq!(
        output.statements[0].remote_location, None,
        "author-written SQL is not classified"
    );

    let inline = spec_with_source(
        "rows",
        DataSourceKind::InlineRows(vec![SpecValue::Object({
            let mut m = IndexMap::new();
            // A URL as a data VALUE must not classify the source as remote.
            m.insert(
                "link".to_string(),
                SpecValue::String("https://example.com/x.parquet".to_string()),
            );
            m
        })]),
        IndexMap::new(),
    );
    let output = emit_sources(&inline, None).unwrap();
    assert_eq!(
        output.statements[0].remote_location, None,
        "inline rows are never remote, whatever strings they contain"
    );
}

#[test]
fn dfsql_ducklake_uri_attaches_read_only() {
    let spec = spec_with_source(
        "meridian",
        DataSourceKind::File("ducklake:https://openlake.example/catalog/open.ducklake".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, Some(Path::new("/data"))).unwrap();
    let ddl = &output.statements[0];
    assert_eq!(ddl.source_kind, SourceKindTag::DuckLake);
    assert_eq!(
        ddl.sql,
        "ATTACH 'ducklake:https://openlake.example/catalog/open.ducklake' \
         AS \"meridian\" (READ_ONLY)"
    );
    assert_eq!(
        ddl.remote_location.as_deref(),
        Some("ducklake:https://openlake.example/catalog/open.ducklake"),
        "an https DuckLake catalog is a remote location"
    );
}

#[test]
fn dfsql_ducklake_bare_metadata_file_gets_prefix_and_base_dir() {
    let spec = spec_with_source(
        "lake",
        DataSourceKind::File("open.ducklake".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, Some(Path::new("/data"))).unwrap();
    let ddl = &output.statements[0];
    assert_eq!(ddl.source_kind, SourceKindTag::DuckLake);
    assert_eq!(
        ddl.sql,
        "ATTACH 'ducklake:/data/open.ducklake' AS \"lake\" (READ_ONLY)"
    );
    assert_eq!(
        ddl.remote_location, None,
        "a local DuckLake catalog is not remote"
    );
}

#[test]
fn dfsql_ducklake_relative_uri_resolves_inner_path() {
    // The prefix survives and the inner catalog path is base-dir-relative.
    let spec = spec_with_source(
        "lake",
        DataSourceKind::File("ducklake:meta.ducklake".to_string()),
        IndexMap::new(),
    );
    let output = emit_sources(&spec, Some(Path::new("/base"))).unwrap();
    assert!(
        output.statements[0]
            .sql
            .contains("'ducklake:/base/meta.ducklake'"),
        "inner path resolves against base_dir: {}",
        output.statements[0].sql
    );
}
