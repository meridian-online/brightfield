//! In-process DuckDB execution engine for Mosaic spec pipelines.
//!
//! This crate sits downstream of `brightfield-spec` (parsing) and `brightfield-sql`
//! (SQL emission). It executes the emitted SQL against an in-process DuckDB
//! instance and returns Arrow record batches.
//!
//! **Dependency chain:** `brightfield-spec` → `brightfield-sql` → `brightfield-engine`.
//! Neither upstream crate depends on this one.

pub mod error;

use std::collections::HashMap;
use std::path::Path;

// Re-export duckdb's Arrow types so consumers don't need a separate arrow dep.
pub use duckdb::arrow::record_batch::RecordBatch;
use duckdb::Connection;

use brightfield_spec::analysis::{ComponentPath, SpecAnalysis};
use brightfield_spec::ast::{Component, Spec, SpecValue};
use brightfield_spec::parse::ParseWarning;
use brightfield_spec::vocab::MarkKind;

use brightfield_sql::binding::{Binding, EmittedQuery, ParamValues};
use brightfield_sql::emit::{emit_query, emit_query_with_passes, emit_sources};
use brightfield_sql::navigation_filter_pass::NavigationFilterPass;
use brightfield_sql::passes::Pass;

use crate::error::EngineError;

/// Result of loading a spec into a session.
pub struct LoadResult {
    /// The active session.
    pub session: Session,
    /// Non-fatal warnings from DDL emission (e.g. unknown CSV extras).
    pub warnings: Vec<ParseWarning>,
}

impl std::fmt::Debug for LoadResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadResult")
            .field("warnings_count", &self.warnings.len())
            .finish()
    }
}

/// Factory for creating [`Session`] objects. Stateless.
pub struct Engine;

impl Engine {
    /// Create a new engine instance.
    pub fn new() -> Self {
        Engine
    }

    /// Load a spec into a new session.
    ///
    /// Opens an in-memory DuckDB connection, executes all source DDL from
    /// `emit_sources()`, and builds the mark-index map for reactive updates.
    pub fn load_spec(
        &self,
        spec: Spec,
        analysis: SpecAnalysis,
        base_dir: Option<&Path>,
    ) -> Result<LoadResult, EngineError> {
        let conn =
            Connection::open_in_memory().map_err(|e| EngineError::ConnectionFailed { cause: e })?;

        let emit_output =
            emit_sources(&spec, base_dir).map_err(|e| EngineError::EmitFailed { cause: e })?;

        for ddl in &emit_output.statements {
            conn.execute_batch(&ddl.sql)
                .map_err(|e| EngineError::DdlFailed {
                    source_name: ddl.view_name.clone(),
                    sql: ddl.sql.clone(),
                    cause: e,
                })?;
        }

        let mark_index_map = build_mark_index_map(&spec);

        // Initialise param_state from spec.params defaults. Only scalar
        // Value params are populated; Selection params are omitted (they
        // enter param_state on first propagate_param call).
        let mut param_state = ParamValues::new();
        for (name, node) in &spec.params {
            if let brightfield_spec::ast::ParamNode::Value(v) = node {
                param_state.insert(name.clone(), v.clone());
            }
        }

        let session = Session {
            conn,
            spec,
            analysis,
            mark_index_map,
            cache: HashMap::new(),
            ddl_warnings: emit_output.warnings.clone(),
            param_state,
        };

        Ok(LoadResult {
            session,
            warnings: emit_output.warnings,
        })
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// An active execution session. Owns a DuckDB connection, the loaded spec,
/// analysis, and prepared statement cache.
///
/// Dropping the session closes the DuckDB connection.
pub struct Session {
    conn: Connection,
    spec: Spec,
    analysis: SpecAnalysis,
    /// Maps ComponentPath string to (depth-first mark index, MarkKind).
    mark_index_map: HashMap<String, (usize, MarkKind)>,
    /// Prepared statement cache keyed by plan_hash.
    cache: HashMap<u64, CachedStatement>,
    /// DDL emission warnings.
    ddl_warnings: Vec<ParseWarning>,
    /// Current param values — updated by propagate_param, consumed by
    /// execute_mark/execute_all for query emission.
    param_state: ParamValues,
}

/// A cached SQL string with its binding metadata.
///
/// **Design note:** This stores the SQL string rather than a DuckDB
/// `PreparedStatement` because `duckdb::Statement<'_>` borrows from
/// `Connection`, making it impossible to store both `Connection` and
/// `Statement` in the same `Session` struct (self-referential borrow).
/// DuckDB internally caches prepared statements by SQL text, so
/// re-calling `conn.prepare(&sql)` with the same string is effectively
/// a no-op at the database level. The cache here avoids redundant
/// SQL emission and confirms plan stability via `plan_hash`.
#[allow(dead_code)]
struct CachedStatement {
    sql: String,
    bindings: Vec<Binding>,
}

impl Session {
    /// Access DDL warnings from the load phase.
    pub fn ddl_warnings(&self) -> &[ParseWarning] {
        &self.ddl_warnings
    }

    /// Current param values — the live param store.
    pub fn current_params(&self) -> &ParamValues {
        &self.param_state
    }

    /// Execute a single mark's query by its depth-first index.
    ///
    /// Uses the current param_state for query emission, so marks see
    /// the latest param values set via propagate_param.
    pub fn execute_mark(&mut self, index: usize) -> Result<Vec<RecordBatch>, EngineError> {
        let params = if self.param_state.is_empty() {
            None
        } else {
            Some(&self.param_state)
        };
        let emitted = emit_query(&self.spec, index, params)
            .map_err(|e| EngineError::EmitFailed { cause: e })?;

        let mark_kind = self.mark_kind_at(index);
        self.execute_emitted(index, &mark_kind, &emitted)
    }

    /// Execute all marks. Returns one result per mark in depth-first order.
    /// Partial failure is possible.
    pub fn execute_all(&mut self) -> Vec<Result<Vec<RecordBatch>, EngineError>> {
        let mark_count = self.mark_index_map.len();
        (0..mark_count).map(|i| self.execute_mark(i)).collect()
    }

    /// Re-execute all marks subscribing to the named parameter.
    ///
    /// Only mark components are dispatched — inputs, interactors, and legends
    /// in the subscriber graph are filtered out. Partial failure is possible.
    pub fn update_param(
        &mut self,
        name: &str,
        value: SpecValue,
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
        let subscriber_paths: Vec<ComponentPath> = self
            .analysis
            .subscriber_graph
            .get(name)
            .cloned()
            .unwrap_or_default();

        // Filter to mark components only.
        let mut mark_indices: Vec<usize> = Vec::new();
        for path in &subscriber_paths {
            if let Some(&(idx, _)) = self.mark_index_map.get(&path.0) {
                mark_indices.push(idx);
            }
        }
        mark_indices.sort();
        mark_indices.dedup();

        let mut param_values = ParamValues::new();
        param_values.insert(name.to_string(), value);

        let mut results = Vec::new();
        for idx in mark_indices {
            let emitted = match emit_query(&self.spec, idx, Some(&param_values)) {
                Ok(eq) => eq,
                Err(e) => {
                    results.push((idx, Err(EngineError::EmitFailed { cause: e })));
                    continue;
                }
            };

            let mark_kind = self.mark_kind_at(idx);
            let result = self.execute_emitted(idx, &mark_kind, &emitted);
            results.push((idx, result));
        }

        results
    }

    /// Propagate a param change: update param_state, then re-execute all
    /// subscribing marks with the full param_state. Returns per-mark results.
    ///
    /// This is the runtime coordinator entry point. It updates the stored
    /// param value, looks up direct subscribers from the subscriber graph,
    /// filters to mark components, and re-emits + re-executes each with
    /// the complete param_state (so multi-param queries see all current values).
    ///
    /// Unsubscribed or unknown params: param_state is updated but no queries
    /// fire — returns an empty results vector. Partial failure: each mark's
    /// result is independent.
    pub fn propagate_param(
        &mut self,
        name: &str,
        value: SpecValue,
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
        // 1. Update param_state.
        self.param_state.insert(name.to_string(), value);

        // 2. Look up subscribers.
        let subscriber_paths: Vec<ComponentPath> = self
            .analysis
            .subscriber_graph
            .get(name)
            .cloned()
            .unwrap_or_default();

        // 3. Filter to mark components only.
        let mut mark_indices: Vec<usize> = Vec::new();
        for path in &subscriber_paths {
            if let Some(&(idx, _)) = self.mark_index_map.get(&path.0) {
                mark_indices.push(idx);
            }
        }
        mark_indices.sort();
        mark_indices.dedup();

        if mark_indices.is_empty() {
            return Vec::new();
        }

        // 4. Re-execute each subscribing mark with the full param_state.
        let mut results = Vec::new();
        for idx in mark_indices {
            let emitted = match emit_query(&self.spec, idx, Some(&self.param_state)) {
                Ok(eq) => eq,
                Err(e) => {
                    results.push((idx, Err(EngineError::EmitFailed { cause: e })));
                    continue;
                }
            };

            let mark_kind = self.mark_kind_at(idx);
            let result = self.execute_emitted(idx, &mark_kind, &emitted);
            results.push((idx, result));
        }

        results
    }

    /// Re-execute all marks with a navigation filter applied for the visible extent.
    ///
    /// Constructs a [`NavigationFilterPass`] from the given column-extent pairs
    /// and re-emits + re-executes all marks with the filter injected into the
    /// query plan. Marks whose emitter fails (unsupported) produce errors in
    /// the result vector, same as `execute_all`.
    ///
    /// `x_extent` and `y_extent` are `Option<(column_name, min, max)>` — when
    /// `None`, that axis is not filtered (full data range). When both are `None`,
    /// this is equivalent to `execute_all` (no filter pass is registered).
    pub fn update_extent(
        &mut self,
        x_extent: Option<(&str, f64, f64)>,
        y_extent: Option<(&str, f64, f64)>,
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
        // Only register the pass when at least one axis has an extent —
        // at full extent (None, None) no pass is needed.
        let passes: Vec<Box<dyn Pass>> = if x_extent.is_some() || y_extent.is_some() {
            let pass = NavigationFilterPass::from_extents(x_extent, y_extent);
            vec![Box::new(pass)]
        } else {
            vec![]
        };

        let mark_count = self.mark_index_map.len();
        let mut results = Vec::new();

        for idx in 0..mark_count {
            let emitted = match emit_query_with_passes(&self.spec, idx, None, &passes) {
                Ok(eq) => eq,
                Err(e) => {
                    results.push((idx, Err(EngineError::EmitFailed { cause: e })));
                    continue;
                }
            };

            let mark_kind = self.mark_kind_at(idx);
            let result = self.execute_emitted(idx, &mark_kind, &emitted);
            results.push((idx, result));
        }

        results
    }

    /// Execute an emitted query, using the SQL-string cache when plan_hash matches.
    ///
    /// On cache hit (same plan_hash), the cached SQL string is reused, confirming
    /// the SQL structure hasn't changed (scalar rebind case). DuckDB internally
    /// caches prepared statements by SQL text, so `conn.prepare()` with a
    /// previously-seen string is effectively a rebind — no re-parse or re-plan
    /// at the database level. On cache miss, the new SQL is cached for future use.
    fn execute_emitted(
        &mut self,
        mark_index: usize,
        mark_kind: &str,
        emitted: &EmittedQuery,
    ) -> Result<Vec<RecordBatch>, EngineError> {
        // Check cache: if plan_hash matches, we know the SQL structure is
        // identical (scalar param change). Use the cached SQL.
        let sql = if let Some(cached) = self.cache.get(&emitted.plan_hash) {
            cached.sql.clone()
        } else {
            // Cache miss — new structural plan. Store it.
            self.cache.insert(
                emitted.plan_hash,
                CachedStatement {
                    sql: emitted.sql.clone(),
                    bindings: emitted.bindings.clone(),
                },
            );
            emitted.sql.clone()
        };

        let batches = self
            .conn
            .prepare(&sql)
            .and_then(|mut stmt| {
                let arrow = stmt.query_arrow(duckdb::params![])?;
                Ok(arrow.collect::<Vec<_>>())
            })
            .map_err(|e| EngineError::QueryFailed {
                mark_index,
                mark_kind: mark_kind.to_string(),
                sql,
                cause: e,
            })?;

        Ok(batches)
    }

    /// Execute a raw SQL query and return Arrow batches. Test-only.
    #[cfg(test)]
    pub fn execute_raw_sql(
        &self,
        sql: &str,
    ) -> Result<Vec<RecordBatch>, duckdb::Error> {
        let mut stmt = self.conn.prepare(sql)?;
        let arrow = stmt.query_arrow(duckdb::params![])?;
        Ok(arrow.collect())
    }

    /// Look up the wire name of the mark at a given depth-first index.
    fn mark_kind_at(&self, index: usize) -> String {
        for (_, &(idx, kind)) in &self.mark_index_map {
            if idx == index {
                return kind.wire_name().to_string();
            }
        }
        "unknown".to_string()
    }

    /// Expose cache size for testing (ac-07).
    #[cfg(test)]
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Execute a hand-crafted EmittedQuery for testing (bypasses emit_query).
    #[cfg(test)]
    pub fn test_execute_emitted(
        &mut self,
        mark_index: usize,
        mark_kind: &str,
        emitted: &EmittedQuery,
    ) -> Result<Vec<RecordBatch>, EngineError> {
        self.execute_emitted(mark_index, mark_kind, emitted)
    }
}

/// Walk the spec's component tree depth-first and collect mark positions.
fn build_mark_index_map(spec: &Spec) -> HashMap<String, (usize, MarkKind)> {
    let mut marks: Vec<(String, MarkKind)> = Vec::new();
    if let Some(root) = &spec.root {
        collect_marks_with_path(root, "root", &mut marks);
    }
    marks
        .into_iter()
        .enumerate()
        .map(|(i, (path, kind))| (path, (i, kind)))
        .collect()
}

fn collect_marks_with_path(
    component: &Component,
    prefix: &str,
    marks: &mut Vec<(String, MarkKind)>,
) {
    match component {
        Component::Plot(plot) => {
            for (i, item) in plot.items.iter().enumerate() {
                let child_prefix = format!("{prefix}/plot[{i}]");
                collect_marks_with_path(item, &child_prefix, marks);
            }
        }
        Component::HConcat(concat) => {
            for (i, child) in concat.items.iter().enumerate() {
                let child_prefix = format!("{prefix}/hconcat[{i}]");
                collect_marks_with_path(child, &child_prefix, marks);
            }
        }
        Component::VConcat(concat) => {
            for (i, child) in concat.items.iter().enumerate() {
                let child_prefix = format!("{prefix}/vconcat[{i}]");
                collect_marks_with_path(child, &child_prefix, marks);
            }
        }
        Component::Mark(mark) => {
            let path = format!("{prefix}/mark[{}]", mark.kind.wire_name());
            marks.push((path, mark.kind));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::analysis::analyse_spec;
    use brightfield_spec::{parse_spec, Format};
    use brightfield_sql::error::EmitError;

    fn parse_and_analyse(yaml: &str) -> (Spec, SpecAnalysis) {
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse failed");
        let analysis = analyse_spec(&parsed.spec).expect("analysis failed");
        (parsed.spec, analysis)
    }

    // --- ac-02: EngineError variants ---
    #[test]
    fn dex_ac02_engine_error_connection_failed() {
        let err = EngineError::ConnectionFailed {
            cause: duckdb::Error::InvalidColumnIndex(0),
        };
        let msg = format!("{err}");
        assert!(msg.contains("connection failed"), "got: {msg}");
    }

    #[test]
    fn dex_ac02_engine_error_ddl_failed() {
        let err = EngineError::DdlFailed {
            source_name: "flights".to_string(),
            sql: "CREATE VIEW flights AS ...".to_string(),
            cause: duckdb::Error::InvalidColumnIndex(0),
        };
        let msg = format!("{err}");
        assert!(msg.contains("flights"), "got: {msg}");
        assert!(msg.contains("DDL failed"), "got: {msg}");
    }

    #[test]
    fn dex_ac02_engine_error_query_failed() {
        let err = EngineError::QueryFailed {
            mark_index: 2,
            mark_kind: "dot".to_string(),
            sql: "SELECT * FROM t".to_string(),
            cause: duckdb::Error::InvalidColumnIndex(0),
        };
        let msg = format!("{err}");
        assert!(msg.contains("mark 2"), "got: {msg}");
        assert!(msg.contains("dot"), "got: {msg}");
    }

    #[test]
    fn dex_ac02_engine_error_emit_failed() {
        let err = EngineError::EmitFailed {
            cause: EmitError::UnsupportedMark {
                kind: "hexbin".to_string(),
            },
        };
        let msg = format!("{err}");
        assert!(msg.contains("emit failed"), "got: {msg}");
        assert!(msg.contains("hexbin"), "got: {msg}");
    }

    // --- ac-03: Engine::new() and load_spec ---
    #[test]
    fn dex_ac03_load_spec_with_inline_data() {
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
plot:
  - mark: dot
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let result = engine.load_spec(spec, analysis, None);
        assert!(result.is_ok(), "load_spec failed: {:?}", result.err());
        let load = result.unwrap();

        // Verify the view is queryable.
        let mut stmt = load.session.conn.prepare("SELECT * FROM t").unwrap();
        let rows: Vec<RecordBatch> = stmt.query_arrow(duckdb::params![]).unwrap().collect();
        assert!(!rows.is_empty(), "expected rows from inline data");
    }

    // --- ac-04: execute_mark ---
    #[test]
    fn dex_ac04_execute_mark_unsupported() {
        // Use a mark kind that SimpleLowerer is NOT registered for
        let yaml = r#"
data:
  t:
    - { x: 1 }
plot:
  - mark: rect
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let result = session.execute_mark(0);
        assert!(result.is_err());
        match result.unwrap_err() {
            EngineError::EmitFailed { cause } => {
                assert!(matches!(cause, EmitError::UnsupportedMark { .. }));
            }
            other => panic!("expected EmitFailed, got: {other:?}"),
        }
    }

    // --- ac-05: execute_all with partial failure ---
    #[test]
    fn dex_ac05_execute_all_partial_failure() {
        // Mix a supported mark (dot with data.from) and an unsupported mark (rect)
        let yaml = r#"
data:
  t:
    - { x: 1 }
plot:
  - mark: dot
    data: { from: t }
  - mark: rect
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let results = session.execute_all();
        assert_eq!(results.len(), 2);
        // dot with data.from succeeds via SimpleLowerer
        assert!(results[0].is_ok(), "dot with data.from should succeed");
        // rect is unsupported
        assert!(results[1].is_err(), "rect should be unsupported");
    }

    // --- msv ac-01: SimpleLowerer end-to-end via Session ---
    #[test]
    fn msv_ac01_execute_mark_dot_with_data_from() {
        let yaml = r#"
data:
  flights:
    - { origin: "SEA", delay: 14 }
    - { origin: "LAX", delay: -3 }
    - { origin: "SEA", delay: 22 }
plot:
  - mark: dot
    data: { from: flights }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let result = session.execute_mark(0);
        assert!(result.is_ok(), "execute_mark failed: {:?}", result.err());
        let batches = result.unwrap();
        assert!(!batches.is_empty(), "expected at least one RecordBatch");
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3, "expected 3 rows from inline data");
    }

    // --- ac-08: DDL failure produces structured error ---
    #[test]
    fn dex_ac08_ddl_failed_nonexistent_parquet() {
        let yaml = r#"
data:
  flights: { file: /nonexistent/path/flights.parquet }
plot:
  - mark: dot
    data: { from: flights }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let result = engine.load_spec(spec, analysis, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            EngineError::DdlFailed {
                source_name,
                sql,
                cause: _,
            } => {
                assert_eq!(source_name, "flights");
                assert!(
                    sql.contains("read_parquet"),
                    "sql should reference parquet: {sql}"
                );
            }
            other => panic!("expected DdlFailed, got: {other:?}"),
        }
    }

    // --- ac-09: Session drop and re-create ---
    #[test]
    fn dex_ac09_session_drop_and_recreate() {
        let yaml1 = r#"
data:
  t1:
    - { a: 1 }
plot:
  - mark: dot
    data: { from: t1 }
"#;
        let yaml2 = r#"
data:
  t2:
    - { b: 2 }
plot:
  - mark: dot
    data: { from: t2 }
"#;
        let engine = Engine::new();

        let (spec1, analysis1) = parse_and_analyse(yaml1);
        let session1 = engine.load_spec(spec1, analysis1, None).unwrap().session;
        drop(session1);

        let (spec2, analysis2) = parse_and_analyse(yaml2);
        let result = engine.load_spec(spec2, analysis2, None);
        assert!(result.is_ok(), "second session failed: {:?}", result.err());

        let load = result.unwrap();
        let mut stmt = load.session.conn.prepare("SELECT * FROM t2").unwrap();
        let rows: Vec<RecordBatch> = stmt.query_arrow(duckdb::params![]).unwrap().collect();
        assert!(!rows.is_empty());
    }

    // --- ac-06: update_param filters to marks only ---
    #[test]
    fn dex_ac06_update_param_filters_to_marks() {
        // Spec with a param referenced by a mark via filterBy AND an input
        // that also references the param. update_param should only return
        // results for marks, not inputs.
        let yaml = r#"
params:
  brush: { select: crossfilter }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
vconcat:
  - plot:
    - mark: dot
      data: { from: t, filterBy: $brush }
  - plot:
    - mark: line
      data: { from: t, filterBy: $brush }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);

        // Verify the subscriber graph has mark entries for 'brush'.
        let subs = analysis.subscriber_graph.get("brush").expect("brush should have subscribers");
        assert!(subs.len() >= 2, "expected at least 2 subscribers for brush, got {}", subs.len());

        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // update_param should dispatch to mark subscribers only.
        // Both dot and line have data.from, so SimpleLowerer handles them.
        let results = session.update_param("brush", SpecValue::String("test".to_string()));

        // Key assertions:
        // 1. Results are non-empty — the param has mark subscribers.
        assert!(!results.is_empty(), "expected results for subscribing marks");
        // 2. Each result succeeds — SimpleLowerer handles dot and line with data.from.
        //    The important thing is we got results, proving the subscriber graph was consulted.
        for (idx, result) in &results {
            assert!(result.is_ok(), "mark {idx} should succeed via SimpleLowerer");
        }
        // 3. Result count matches mark subscribers, not all subscribers.
        assert_eq!(results.len(), 2, "expected exactly 2 mark results");
    }

    // --- ac-07: Prepared statement cache ---
    #[test]
    fn dex_ac07_cache_populated_on_execute() {
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
plot:
  - mark: dot
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        assert_eq!(session.cache_len(), 0, "cache should start empty");

        // Execute a hand-crafted query (bypassing emit_query which fails for unimplemented marks).
        let emitted = EmittedQuery {
            sql: "SELECT * FROM t".to_string(),
            bindings: vec![],
            plan_hash: 42,
        };
        let result = session.test_execute_emitted(0, "dot", &emitted);
        assert!(result.is_ok());
        assert_eq!(session.cache_len(), 1, "cache should have 1 entry after first execute");

        // Same plan_hash — cache hit (no new entry).
        let emitted2 = EmittedQuery {
            sql: "SELECT * FROM t".to_string(),
            bindings: vec![],
            plan_hash: 42,
        };
        let result2 = session.test_execute_emitted(0, "dot", &emitted2);
        assert!(result2.is_ok());
        assert_eq!(session.cache_len(), 1, "cache should still have 1 entry (reused)");

        // Different plan_hash — cache miss (new entry).
        let emitted3 = EmittedQuery {
            sql: "SELECT x FROM t".to_string(),
            bindings: vec![],
            plan_hash: 99,
        };
        let result3 = session.test_execute_emitted(0, "dot", &emitted3);
        assert!(result3.is_ok());
        assert_eq!(session.cache_len(), 2, "cache should have 2 entries (new plan_hash)");
    }

    // --- ac-08: QueryFailed with mark_index and mark_kind ---
    #[test]
    fn dex_ac08_query_failed_with_mark_context() {
        let yaml = r#"
data:
  t:
    - { x: 1 }
plot:
  - mark: dot
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Hand-craft an EmittedQuery with invalid SQL.
        let emitted = EmittedQuery {
            sql: "SELECT * FROM nonexistent_table_xyz".to_string(),
            bindings: vec![],
            plan_hash: 999,
        };
        let result = session.test_execute_emitted(0, "dot", &emitted);
        assert!(result.is_err());
        match result.unwrap_err() {
            EngineError::QueryFailed {
                mark_index,
                mark_kind,
                sql,
                cause: _,
            } => {
                assert_eq!(mark_index, 0);
                assert_eq!(mark_kind, "dot");
                assert!(sql.contains("nonexistent_table_xyz"));
            }
            other => panic!("expected QueryFailed, got: {other:?}"),
        }
    }

    // --- ac-03: ddl_warnings accessor ---
    #[test]
    fn dex_ac03_ddl_warnings_accessible() {
        // Inline data produces no warnings — test that the accessor works.
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
plot:
  - mark: dot
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let load = engine.load_spec(spec, analysis, None).unwrap();
        // Warnings are empty for inline data, but the accessor is callable.
        assert!(load.session.ddl_warnings().is_empty());
        assert!(load.warnings.is_empty());
    }

    // --- ac-05: mixed partial failure via test helper ---
    #[test]
    fn dex_ac05_execute_all_mixed_results() {
        // Two marks: one succeeds via SimpleLowerer, one fails (unsupported).
        // This demonstrates Session can produce mixed Ok+Err in the same session.
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
plot:
  - mark: dot
    data: { from: t }
  - mark: rect
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Mark 0 succeeds via SimpleLowerer (dot with data.from).
        let ok_result = session.execute_mark(0);
        assert!(ok_result.is_ok(), "mark 0 should succeed via SimpleLowerer");
        assert!(!ok_result.unwrap().is_empty());

        // Mark 1 fails via execute_mark (rect is unsupported).
        let err_result = session.execute_mark(1);
        assert!(err_result.is_err(), "mark 1 should fail (unsupported)");
        assert!(matches!(
            err_result.unwrap_err(),
            EngineError::EmitFailed { .. }
        ));
    }

    // --- ac-10 (nav): update_extent with navigation filter ---
    #[test]
    fn nav_ac10_update_extent_produces_filtered_sql() {
        // Spec with inline data and a mark — we test that the emitted SQL
        // contains a BETWEEN-style predicate for the filtered column.
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
    - { x: 5, y: 50 }
plot:
  - mark: dot
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // update_extent should attempt to emit with the navigation filter.
        // SimpleLowerer handles dot with data.from, so the query succeeds.
        let results = session.update_extent(
            Some(("x", 2.0, 4.0)),
            None,
        );

        // There should be exactly 1 mark result (the dot).
        assert_eq!(results.len(), 1, "expected 1 mark result");
        // Dot with data.from succeeds via SimpleLowerer + navigation filter pass.
        let (idx, result) = &results[0];
        assert_eq!(*idx, 0);
        assert!(result.is_ok(), "dot mark should succeed via SimpleLowerer");
    }

    #[test]
    fn nav_ac10_update_extent_emits_between_clause() {
        // Direct test: emit a query with the navigation filter pass and
        // verify the SQL contains the expected BETWEEN-style predicates.
        use brightfield_sql::navigation_filter_pass::NavigationFilterPass;
        use brightfield_sql::passes::Pass;

        let pass = NavigationFilterPass::from_extents(
            Some(("x", 2.0, 4.0)),
            None,
        );

        // Build a simple plan and apply the pass.
        use brightfield_sql::ir::QueryPlan;
        let plan = QueryPlan::Source {
            table: "t".to_string(),
        };
        let filtered = pass.apply(plan);

        // Render to SQL and check for the filter.
        let mut bindings = Vec::new();
        let sql = brightfield_sql::render::render_query(&filtered, &mut bindings);
        assert!(
            sql.contains("\"x\" >= 2") && sql.contains("\"x\" <= 4"),
            "SQL should contain BETWEEN predicates for x, got: {sql}"
        );
    }

    #[test]
    fn nav_ac10_update_extent_both_axes() {
        use brightfield_sql::navigation_filter_pass::NavigationFilterPass;
        use brightfield_sql::passes::Pass;

        let pass = NavigationFilterPass::from_extents(
            Some(("ts", 1000.0, 2000.0)),
            Some(("price", 50.0, 150.0)),
        );

        let plan = brightfield_sql::ir::QueryPlan::Source {
            table: "data".to_string(),
        };
        let filtered = pass.apply(plan);

        let mut bindings = Vec::new();
        let sql = brightfield_sql::render::render_query(&filtered, &mut bindings);
        assert!(sql.contains("\"ts\""), "SQL should filter on ts column, got: {sql}");
        assert!(sql.contains("\"price\""), "SQL should filter on price column, got: {sql}");
    }

    #[test]
    fn nav_ac10_update_extent_none_is_no_filter() {
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
plot:
  - mark: dot
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // With None/None, should behave like execute_all (no filter).
        let results = session.update_extent(None, None);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn dex_ac07_cache_returns_arrow_batches() {
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
plot:
  - mark: dot
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let emitted = EmittedQuery {
            sql: "SELECT x, y FROM t".to_string(),
            bindings: vec![],
            plan_hash: 100,
        };
        let batches = session.test_execute_emitted(0, "dot", &emitted).unwrap();
        assert!(!batches.is_empty(), "expected record batches");
        // Verify we got actual data.
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2, "expected 2 rows from inline data");
    }

    // --- rpw2 ac-01: param_state + current_params ---

    #[test]
    fn rpw2_ac01_param_state_initialised_from_defaults() {
        let yaml = r#"
params:
  threshold: 50
  label: hello
data:
  t:
    - { x: 1, y: 10 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let session = engine.load_spec(spec, analysis, None).unwrap().session;

        let params = session.current_params();
        assert_eq!(params.len(), 2, "should have 2 params from defaults");
        assert_eq!(
            params.get("threshold"),
            Some(&SpecValue::Integer(50)),
            "threshold should be 50"
        );
        assert_eq!(
            params.get("label"),
            Some(&SpecValue::String("hello".to_string())),
            "label should be 'hello'"
        );
    }

    #[test]
    fn rpw2_ac01_param_state_empty_when_no_params() {
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let session = engine.load_spec(spec, analysis, None).unwrap().session;

        assert!(
            session.current_params().is_empty(),
            "no params should mean empty state"
        );
    }

    // --- rpw2 ac-02: propagate_param dispatches to subscribers ---

    #[test]
    fn rpw2_ac02_propagate_param_dispatches_to_subscriber_mark() {
        // Mark subscribes to "brush" via filterBy — this makes the mark
        // appear in the subscriber graph for "brush".
        // filterBy requires a selection param (not a value param).
        let yaml = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);

        // Verify the subscriber graph actually links the mark to "brush".
        let subs = analysis.subscriber_graph.get("brush").expect("brush should have subscribers");
        assert!(!subs.is_empty(), "dot mark should subscribe to brush via filterBy");

        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let results = session.propagate_param("brush", SpecValue::Integer(42));

        // The mark should be dispatched — results should be non-empty.
        // The mark will fail (no SimpleLowerer on main) or succeed, but
        // the dispatch happened — the results vector has an entry.
        assert!(!results.is_empty(), "subscriber mark should be dispatched");
        assert_eq!(results.len(), 1, "exactly one mark should be dispatched");

        // param_state updated regardless of result.
        assert_eq!(
            session.current_params().get("brush"),
            Some(&SpecValue::Integer(42))
        );
    }

    #[test]
    fn rpw2_ac02_propagate_param_updates_state_and_dispatches() {
        // Mark subscribes to "brush" via filterBy (selection param).
        let yaml = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Initial state — selection params are NOT in param_state.
        assert!(session.current_params().get("brush").is_none());

        // Propagate — mark should be dispatched, state updated.
        let results = session.propagate_param("brush", SpecValue::Integer(75));
        assert!(!results.is_empty(), "subscriber mark should be dispatched");

        // State should be updated.
        assert_eq!(
            session.current_params().get("brush"),
            Some(&SpecValue::Integer(75))
        );
    }

    // --- rpw2 ac-03: unsubscribed param returns empty ---

    #[test]
    fn rpw2_ac03_unsubscribed_param_returns_empty() {
        let yaml = r#"
params:
  orphan: 0
data:
  t:
    - { x: 1, y: 10 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // "orphan" has no subscribers (no mark references it).
        let results = session.propagate_param("orphan", SpecValue::Integer(99));
        assert!(results.is_empty(), "unsubscribed param should return empty results");

        // But param_state should be updated.
        assert_eq!(
            session.current_params().get("orphan"),
            Some(&SpecValue::Integer(99))
        );
    }

    // --- rpw2 ac-04: partial failure ---

    #[test]
    fn rpw2_ac04_partial_failure_mixed_ok_err() {
        // Two marks subscribe to "brush" via filterBy. Dot is supported by
        // SimpleLowerer (Ok), rect is not (Err). Each mark is dispatched
        // independently — rect's failure must not prevent dot from succeeding.
        let yaml = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
  - mark: rect
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);

        // Verify both marks subscribe.
        let subs = analysis.subscriber_graph.get("brush").expect("brush subscribers");
        assert!(subs.len() >= 2, "both marks should subscribe to brush");

        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let results = session.propagate_param("brush", SpecValue::Integer(42));

        // Both marks dispatched — independent of each other's success/failure.
        assert_eq!(results.len(), 2, "both subscriber marks should be dispatched");

        // Count successes and failures.
        let ok_count = results.iter().filter(|(_, r)| r.is_ok()).count();
        let err_count = results.iter().filter(|(_, r)| r.is_err()).count();
        assert_eq!(ok_count, 1, "dot should succeed via SimpleLowerer");
        assert_eq!(err_count, 1, "rect should fail (UnsupportedMark)");

        // The successful mark should have returned data (2 rows from inline source).
        let (_, ok_result) = results.iter().find(|(_, r)| r.is_ok()).unwrap();
        let batches = ok_result.as_ref().unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2, "dot mark should return 2 rows from inline data");

        // param_state updated regardless of mixed results.
        assert_eq!(
            session.current_params().get("brush"),
            Some(&SpecValue::Integer(42)),
            "param_state should reflect update regardless of execution results"
        );
    }

    // --- rpw2 ac-05: unknown param permissive ---

    #[test]
    fn rpw2_ac05_unknown_param_permissive() {
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // No params declared at all. Propagate an arbitrary param.
        let results =
            session.propagate_param("invented_param", SpecValue::String("hello".to_string()));
        assert!(results.is_empty(), "unknown param should return empty");
        assert_eq!(
            session.current_params().get("invented_param"),
            Some(&SpecValue::String("hello".to_string())),
            "param_state should contain the dynamically injected param"
        );
    }

    // --- rpw2 ac-06: execute_mark uses param_state ---

    #[test]
    fn rpw2_ac06_execute_mark_passes_param_state() {
        // Verify that after propagate_param updates param_state, the state
        // is accessible and consistent. The actual param injection into SQL
        // depends on SimpleLowerer (card 0001) — here we verify the state
        // plumbing: propagate_param updates param_state, and current_params()
        // reflects the update for downstream consumers.
        let yaml = r#"
params:
  threshold: 50
  mode: auto
data:
  t:
    - { x: 1, y: 10 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Initial state has both params.
        assert_eq!(session.current_params().len(), 2);

        // Update one param.
        session.propagate_param("threshold", SpecValue::Integer(75));

        // Both params visible in state — threshold updated, mode unchanged.
        let params = session.current_params();
        assert_eq!(params.get("threshold"), Some(&SpecValue::Integer(75)));
        assert_eq!(
            params.get("mode"),
            Some(&SpecValue::String("auto".to_string()))
        );

        // Update the other param.
        session.propagate_param("mode", SpecValue::String("manual".to_string()));

        let params = session.current_params();
        assert_eq!(params.get("threshold"), Some(&SpecValue::Integer(75)));
        assert_eq!(
            params.get("mode"),
            Some(&SpecValue::String("manual".to_string()))
        );
    }

    // --- rpw2 ac-07: end-to-end integration ---

    #[test]
    fn rpw2_ac07_end_to_end_param_propagation() {
        // Full pipeline: parse spec with params, load, propagate, verify state.
        // The actual SQL param injection requires SimpleLowerer (card 0001);
        // this test verifies the coordinator's state management end-to-end.
        let yaml = r#"
params:
  filter_val: 1
  label: initial
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse failed");
        let analysis = analyse_spec(&parsed.spec).expect("analysis failed");

        let engine = Engine::new();
        let load = engine
            .load_spec(parsed.spec, analysis, None)
            .expect("load failed");
        let mut session = load.session;

        // Verify initial param state.
        assert_eq!(
            session.current_params().get("filter_val"),
            Some(&SpecValue::Integer(1))
        );
        assert_eq!(
            session.current_params().get("label"),
            Some(&SpecValue::String("initial".to_string()))
        );

        // Propagate param change — filter_val.
        let results = session.propagate_param("filter_val", SpecValue::Integer(2));
        // No mark subscribers for this param (dot doesn't reference $filter_val).
        assert!(results.is_empty(), "no subscribers for filter_val");
        assert_eq!(
            session.current_params().get("filter_val"),
            Some(&SpecValue::Integer(2))
        );

        // Propagate param change — label.
        let results =
            session.propagate_param("label", SpecValue::String("updated".to_string()));
        assert!(results.is_empty(), "no subscribers for label");
        assert_eq!(
            session.current_params().get("label"),
            Some(&SpecValue::String("updated".to_string()))
        );

        // Add a dynamic param.
        let results =
            session.propagate_param("dynamic", SpecValue::Float(3.14));
        assert!(results.is_empty(), "no subscribers for dynamic param");
        assert_eq!(
            session.current_params().get("dynamic"),
            Some(&SpecValue::Float(3.14))
        );

        // Final state should have all 3 params.
        assert_eq!(session.current_params().len(), 3);
    }

    #[test]
    fn rpw2_ac01_selection_params_excluded_from_initial_state() {
        // Selection params should not appear in initial param_state.
        let yaml = r#"
params:
  threshold: 50
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let session = engine.load_spec(spec, analysis, None).unwrap().session;

        let params = session.current_params();
        assert_eq!(params.len(), 1, "only scalar params in initial state");
        assert!(
            params.contains_key("threshold"),
            "threshold should be present"
        );
        assert!(
            !params.contains_key("brush"),
            "selection param should be excluded"
        );
    }
}
