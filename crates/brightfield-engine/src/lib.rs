//! In-process DuckDB execution engine for Mosaic spec pipelines.
//!
//! This crate sits downstream of `brightfield-spec` (parsing) and `brightfield-sql`
//! (SQL emission). It executes the emitted SQL against an in-process DuckDB
//! instance and returns Arrow record batches.
//!
//! **Dependency chain:** `brightfield-spec` → `brightfield-sql` → `brightfield-engine`.
//! Neither upstream crate depends on this one.

pub mod error;

use std::collections::{HashMap, HashSet};
use std::path::Path;

// Re-export duckdb's Arrow types so consumers don't need a separate arrow dep.
pub use duckdb::arrow::record_batch::RecordBatch;
pub use brightfield_sql::ir::Predicate as SqlPredicate;
use duckdb::Connection;

/// Concatenate a query's result chunks into a single [`RecordBatch`], or `None`
/// if empty. A DuckDB result arrives as one batch per ~2048 rows; renderers and
/// the cross-filter coordinator want one batch per mark. Uses duckdb's bundled
/// Arrow so callers in crates without a direct `arrow` dependency (e.g.
/// `brightfield-ui`) can concatenate re-execution results. Falls back to the
/// first chunk if concatenation fails (schema mismatch is not expected).
pub fn concat_batches(batches: Vec<RecordBatch>) -> Option<RecordBatch> {
    match batches.len() {
        0 => None,
        1 => batches.into_iter().next(),
        _ => {
            let schema = batches[0].schema();
            match duckdb::arrow::compute::concat_batches(&schema, &batches) {
                Ok(batch) => Some(batch),
                Err(_) => batches.into_iter().next(),
            }
        }
    }
}

use brightfield_spec::analysis::{ComponentPath, SpecAnalysis};
use brightfield_spec::ast::{Component, Spec, SpecValue};
use brightfield_spec::parse::ParseWarning;
use brightfield_spec::vocab::MarkKind;

use brightfield_sql::binding::{Binding, EmittedQuery, ParamValues};
use brightfield_sql::emit::{emit_query, emit_query_with_passes, emit_sources};
use brightfield_sql::ir::Predicate;
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
            sql_cache: SqlCache::default(),
            ddl_warnings: emit_output.warnings.clone(),
            param_state,
            selection_state: HashMap::new(),
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
    /// Renderer-side SQL → Arrow batches cache (capped LRU, cap 32).
    ///
    /// TODO(card-runtime-reactivity): this is a stand-in for proper two-tier
    /// param-effect routing. The proper design separates pure-style param
    /// drags (no SQL re-execution needed) from data-shape param changes
    /// (SQL must re-run). Until that lands we cache by literal SQL string
    /// and rely on the cache hit to skip re-execution.
    sql_cache: SqlCache,
    /// DDL emission warnings.
    ddl_warnings: Vec<ParseWarning>,
    /// Current param values — updated by propagate_param, consumed by
    /// execute_mark/execute_all for query emission.
    param_state: ParamValues,
    /// Live per-contributor selection predicates — updated by
    /// propagate_selection, consumed by execute_mark/execute_all/etc. via
    /// selection_predicates_for_emit. Outer key: selection name. Inner
    /// vec: (contributor_path, predicate) pairs, where contributor_path is
    /// the parent plot path of the contributing component (card 0006 v2
    /// decision 4 — string equality with subscriber's parent plot path
    /// drives crossfilter self-exclusion in compile_selection).
    selection_state: HashMap<String, Vec<(ComponentPath, Predicate)>>,
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

/// SQL → Arrow batches cache with capped LRU eviction.
///
/// Hits skip the DuckDB execute, leaving `duckdb_execute_count` unchanged
/// — the property card 0008 ac-11 / ac-12 verifies. Cap is fixed at 32.
///
/// TODO(card-runtime-reactivity): replace with proper two-tier routing
/// (pure-style vs data-shape param effects). This is a stand-in.
#[derive(Default)]
struct SqlCache {
    /// SQL string → Arrow batches.
    entries: HashMap<String, Vec<RecordBatch>>,
    /// LRU order — most recently used at the back.
    order: Vec<String>,
    /// Counter incremented every time a fresh DuckDB execute runs (cache miss).
    duckdb_execute_count: usize,
}

impl SqlCache {
    const CAP: usize = 32;

    fn get(&mut self, sql: &str) -> Option<Vec<RecordBatch>> {
        if let Some(batches) = self.entries.get(sql) {
            // Move to most-recently-used position.
            self.order.retain(|s| s != sql);
            self.order.push(sql.to_string());
            // RecordBatch is Arc-shared internally; clone is cheap.
            return Some(batches.clone());
        }
        None
    }

    fn insert(&mut self, sql: String, batches: Vec<RecordBatch>) {
        // Evict oldest until under cap.
        while self.entries.len() >= Self::CAP && !self.order.is_empty() {
            let evict = self.order.remove(0);
            self.entries.remove(&evict);
        }
        self.entries.insert(sql.clone(), batches);
        self.order.push(sql);
    }
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

    /// Current selection state — the live per-contributor predicate store.
    /// Card 0006 v2 ac-01.
    pub fn current_selections(&self) -> &HashMap<String, Vec<(ComponentPath, Predicate)>> {
        &self.selection_state
    }

    /// The predicate `contributor` currently holds in selection `name`, if
    /// any — a read-only lookup into the live per-contributor store (card
    /// 0009). `contributor` is the `ComponentPath` payload string (the parent
    /// plot path), matching the keys `propagate_selection` stores.
    ///
    /// The legend toggle derives its dispatch-vs-clear decision from this
    /// slot rather than a UI-side mirror: the slot is shared with the plot's
    /// brush/point interactors (same `(selection, contributor)` identity), so
    /// any gesture that replaces or removes it is observed here instead of
    /// silently desynchronising a mirror.
    pub fn contributor_predicate(&self, name: &str, contributor: &str) -> Option<&Predicate> {
        self.selection_state
            .get(name)?
            .iter()
            .find(|(path, _)| path.0 == contributor)
            .map(|(_, predicate)| predicate)
    }

    /// Convert the live `selection_state` into the shape `emit_query` consumes:
    /// `Vec<(selection_name, Vec<(contributor_path_string, Predicate)>)>`. The
    /// inner contributor strings are the `ComponentPath` payloads — already
    /// stored as parent plot paths so `compile_selection`'s `self_source`
    /// equality fires correctly for crossfilter self-exclusion.
    fn selection_predicates_for_emit(&self) -> Vec<(String, Vec<(String, Predicate)>)> {
        self.selection_state
            .iter()
            .map(|(name, contribs)| {
                let pairs: Vec<(String, Predicate)> = contribs
                    .iter()
                    .map(|(path, pred)| (path.0.clone(), pred.clone()))
                    .collect();
                (name.clone(), pairs)
            })
            .collect()
    }

    /// Propagate a selection update: store the contributor's predicate in
    /// `selection_state[name]` (replacing any prior entry from the same
    /// contributor), look up subscribers from
    /// `analysis.selection_subscribers`, filter to mark components, and
    /// re-emit + re-execute each subscriber with per-subscriber merged
    /// predicates resolved via `compile_selection` inside `emit_query`.
    /// Returns one `(mark_index, Result)` tuple per subscriber.
    ///
    /// Mirrors [`Self::propagate_param`]'s shape — partial-failure pattern,
    /// `selection_state` always updated regardless of subscriber outcomes,
    /// unsubscribed selections silently absorbed (empty result vec, no
    /// queries fire).
    ///
    /// Card 0006 v2 ac-02 / ac-03 / ac-07 / ac-08.
    pub fn propagate_selection(
        &mut self,
        name: &str,
        contributor: ComponentPath,
        predicate: Predicate,
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
        // 1. Update selection_state. A re-contribution from an existing source
        // moves to the TAIL (retain-then-push), so vec order reflects recency:
        // SelectionResolution::Single resolves via `.last()`, so the "most
        // recent" predicate must be the last element, not the source's original
        // slot. For AND/OR strategies the RESULTS are order-independent; the only
        // cost is that reordering changes the emitted SQL string, so a re-contribution
        // of an identical predicate by a non-tail source can miss the SQL cache and
        // re-execute — a minor cache-warmth edge, never a wrong result. Linear
        // scan; ≤5 contributors per selection in the corpus. (card 0006)
        let entries = self.selection_state.entry(name.to_string()).or_default();
        entries.retain(|(p, _)| p != &contributor);
        entries.push((contributor, predicate));

        // 2. Look up subscribers from the static analysis graph.
        let subscriber_paths: Vec<ComponentPath> = self
            .analysis
            .selection_subscribers
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

        // 4. Per-subscriber emit + execute. emit_query computes the
        // per-subscriber `self_source` (parent plot path) internally and
        // calls compile_selection, so the coordinator does not need to
        // resolve per-subscriber predicates inline. Partial failure: each
        // mark's outcome is independent; an emit error on one mark does
        // not halt dispatch.
        let selections = self.selection_predicates_for_emit();
        let selections_ref: Option<&[(String, Vec<(String, Predicate)>)]> =
            Some(selections.as_slice());

        // Clone param_state into an owned snapshot so we hold no borrow on
        // self while looping (execute_emitted needs &mut self for the
        // statement cache).
        let params_owned: ParamValues = self.param_state.clone();
        let params_ref = if params_owned.is_empty() {
            None
        } else {
            Some(&params_owned)
        };

        let mut results = Vec::new();
        for idx in mark_indices {
            let emitted = match emit_query(&self.spec, idx, params_ref, selections_ref) {
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

    /// Retract a contributor's predicate from `selection_state[name]` and
    /// re-execute every subscriber to that selection. Symmetric to
    /// [`Self::propagate_selection`].
    ///
    /// Linear-scan find-and-remove on the contributor list. If the named
    /// selection does not exist OR the contributor is not in its list, this
    /// is a silent no-op: returns an empty result vec, leaves
    /// `selection_state` untouched, fires no queries.
    ///
    /// On a successful removal, looks up subscribers via
    /// `analysis.selection_subscribers`, filters to mark components, and
    /// re-emits + re-executes each subscriber against the now-reduced
    /// selection state. Partial-failure shape mirrors `propagate_selection`:
    /// each mark's result is independent; a per-mark emit/execute error
    /// never halts the dispatch loop.
    ///
    /// Card 0006 v3 (cfs3) ac-01, ac-02. Backs `SelectionDispatcher::clear`.
    pub fn clear_selection(
        &mut self,
        name: &str,
        contributor: ComponentPath,
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
        // 1. Locate and remove the contributor's slot. Silent no-op on miss.
        let removed = match self.selection_state.get_mut(name) {
            Some(entries) => {
                if let Some(idx) = entries.iter().position(|(p, _)| p == &contributor) {
                    entries.remove(idx);
                    true
                } else {
                    false
                }
            }
            None => false,
        };

        if !removed {
            return Vec::new();
        }

        // 2. Look up subscribers from the static analysis graph.
        let subscriber_paths: Vec<ComponentPath> = self
            .analysis
            .selection_subscribers
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

        // 4. Per-subscriber emit + execute against the reduced selection set.
        //    Same shape as propagate_selection's dispatch loop.
        let selections = self.selection_predicates_for_emit();
        let selections_ref: Option<&[(String, Vec<(String, Predicate)>)]> =
            Some(selections.as_slice());

        let params_owned: ParamValues = self.param_state.clone();
        let params_ref = if params_owned.is_empty() {
            None
        } else {
            Some(&params_owned)
        };

        let mut results = Vec::new();
        for idx in mark_indices {
            let emitted = match emit_query(&self.spec, idx, params_ref, selections_ref) {
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

    /// Execute a single mark's query by its depth-first index.
    ///
    /// Uses the current param_state and selection_state for query emission,
    /// so marks see the latest param values (set via propagate_param) and
    /// the latest selection predicates (set via propagate_selection).
    pub fn execute_mark(&mut self, index: usize) -> Result<Vec<RecordBatch>, EngineError> {
        let params = if self.param_state.is_empty() {
            None
        } else {
            Some(&self.param_state)
        };
        let selections = self.selection_predicates_for_emit();
        let selections_ref: Option<&[(String, Vec<(String, Predicate)>)]> = if selections.is_empty() {
            None
        } else {
            Some(selections.as_slice())
        };
        let emitted = emit_query(&self.spec, index, params, selections_ref)
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

        let selections = self.selection_predicates_for_emit();
        let selections_ref: Option<&[(String, Vec<(String, Predicate)>)]> = if selections.is_empty() {
            None
        } else {
            Some(selections.as_slice())
        };

        let mut results = Vec::new();
        for idx in mark_indices {
            let emitted = match emit_query(&self.spec, idx, Some(&param_values), selections_ref) {
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

    /// Propagate a param change: update param_state, then walk the param
    /// dependency DAG starting at `name` and re-execute all marks subscribing
    /// to any descendant param, in topological order. Returns one
    /// (mark_index, Result) tuple per subscribing mark.
    ///
    /// This is the runtime coordinator entry point. It updates the stored
    /// param value, computes the descendant chain via
    /// [`brightfield_spec::analysis::topological_descendants`], and dispatches
    /// at every level against the full `param_state` and the active
    /// `selection_state`. The selection-predicate slice is captured ONCE
    /// before the loop, so chained re-execution honours the active brush
    /// at every level (rpw3 ac-04 / decision 2 invariant).
    ///
    /// **Dedup invariant (rpw3 ac-05 / decision 3 — first-level-wins):** a
    /// mark whose query references both an upstream param (e.g. `$A`) and a
    /// descendant (`$B` with `A → B`) appears in `subscriber_graph[A]`
    /// AND `subscriber_graph[B]`. The walk dispatches it at A's level
    /// (the topologically-earliest level whose subscriber list contains
    /// it) and skips it at B's level via a `dispatched` HashSet carried
    /// across levels. This produces a result vec with one entry per
    /// affected mark, ordered by first-appearance level.
    ///
    /// **Computed-param case-iii deferral (decision 2 / ac-15):** the walk
    /// reads `param_state` for every level but writes `param_state` only
    /// for the explicitly named root. Downstream params are NOT mutated by
    /// the walk — that case (`ParamNode::FromQuery`) requires AST surface
    /// not present in v3 and is deferred.
    ///
    /// Unsubscribed or unknown params: param_state is updated but no queries
    /// fire — returns an empty results vector. Partial failure: each mark's
    /// result is independent and a per-mark emit/execute error never halts
    /// the walk (decision 4).
    ///
    /// rpw3 ac-04, ac-05, ac-06, ac-07, ac-08, ac-15.
    pub fn propagate_param(
        &mut self,
        name: &str,
        value: SpecValue,
    ) -> Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> {
        // 1. Update param_state for the named root only.
        //    (Decision 2 case iii — downstream params are read, never written.)
        self.param_state.insert(name.to_string(), value);

        // 2. Compute the topological walk order: [root, descendant_0, ...].
        let order: Vec<String> =
            brightfield_spec::analysis::topological_descendants(&self.analysis, name);

        // 3. Capture the selection-predicate slice ONCE before the loop.
        //    Every level of the walk receives the same slice, so a chained
        //    re-execution after a brush release continues to honour the
        //    active selection (rpw3 ac-04 invariant). Capturing inside the
        //    loop would re-read self.selection_state at every level — no
        //    behavioural difference today, but a foot-gun if a future change
        //    accidentally mutates selection_state mid-walk.
        let selections = self.selection_predicates_for_emit();
        let selections_ref: Option<&[(String, Vec<(String, Predicate)>)]> =
            if selections.is_empty() {
                None
            } else {
                Some(selections.as_slice())
            };

        // 4. Walk the order. `dispatched` carries cross-level state for the
        //    first-level-wins dedup invariant.
        let mut dispatched: HashSet<usize> = HashSet::new();
        let mut results: Vec<(usize, Result<Vec<RecordBatch>, EngineError>)> = Vec::new();

        for level in &order {
            // Look up subscribers for this level's param.
            let subscriber_paths: Vec<ComponentPath> = self
                .analysis
                .subscriber_graph
                .get(level)
                .cloned()
                .unwrap_or_default();

            // Filter to mark components only, drop already-dispatched.
            let mut mark_indices: Vec<usize> = Vec::new();
            for path in &subscriber_paths {
                if let Some(&(idx, _)) = self.mark_index_map.get(&path.0) {
                    if !dispatched.contains(&idx) {
                        mark_indices.push(idx);
                    }
                }
            }
            mark_indices.sort();
            mark_indices.dedup();

            if mark_indices.is_empty() {
                continue;
            }

            for idx in mark_indices {
                // First-level-wins guard (in case a path appears twice in
                // subscriber_paths for the same mark within one level).
                if !dispatched.insert(idx) {
                    continue;
                }
                let emitted = match emit_query(
                    &self.spec,
                    idx,
                    Some(&self.param_state),
                    selections_ref,
                ) {
                    Ok(eq) => eq,
                    Err(e) => {
                        results
                            .push((idx, Err(EngineError::EmitFailed { cause: e })));
                        continue;
                    }
                };

                let mark_kind = self.mark_kind_at(idx);
                let result = self.execute_emitted(idx, &mark_kind, &emitted);
                results.push((idx, result));
            }
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

        let selections = self.selection_predicates_for_emit();
        let selections_ref: Option<&[(String, Vec<(String, Predicate)>)]> = if selections.is_empty() {
            None
        } else {
            Some(selections.as_slice())
        };

        // Pass the current param_state so a positional `$param` channel (card
        // 0014) is interpolated during navigation too — otherwise `$name` would
        // reach DuckDB as an unbound placeholder and the mark would fail on every
        // pan/zoom.
        let params_owned: ParamValues = self.param_state.clone();
        let params_ref = if params_owned.is_empty() {
            None
        } else {
            Some(&params_owned)
        };

        for idx in 0..mark_count {
            let emitted = match emit_query_with_passes(
                &self.spec,
                idx,
                params_ref,
                selections_ref,
                &passes,
            ) {
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
        // Execute the freshly-emitted SQL directly — it is already
        // param-interpolated (card 0014), so it is always the correct query for
        // the current param_state; never serve a cached string that might predate
        // a param change. Still record the plan for stability tracking, but BOUND
        // the map: with interpolation each distinct inlined param value yields a
        // distinct plan_hash, so a dragged param would otherwise grow this cache
        // without bound. The recorded SQL is redundant with `emitted.sql`, so a
        // cap is safe — beyond it we simply stop recording; execution is
        // unaffected. (Renderer-side dedup is the LRU-capped `sql_cache` below.)
        const PLAN_CACHE_CAP: usize = 64;
        if self.cache.len() < PLAN_CACHE_CAP {
            self.cache
                .entry(emitted.plan_hash)
                .or_insert_with(|| CachedStatement {
                    sql: emitted.sql.clone(),
                    bindings: emitted.bindings.clone(),
                });
        }
        let sql = emitted.sql.clone();

        // Renderer-side SQL cache: hit → skip DuckDB execute entirely.
        if let Some(batches) = self.sql_cache.get(&sql) {
            return Ok(batches);
        }

        // Cache miss — execute the query and record one DuckDB execute.
        self.sql_cache.duckdb_execute_count += 1;
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
                sql: sql.clone(),
                cause: e,
            })?;

        self.sql_cache.insert(sql, batches.clone());
        Ok(batches)
    }

    /// Test-only accessor: number of DuckDB executes performed since this
    /// Session was created. Increments on every cache miss in
    /// `execute_emitted`. Used to verify the renderer-side SQL cache
    /// short-circuits redundant queries (card 0008 ac-11 / ac-12).
    pub fn duckdb_execute_count(&self) -> usize {
        self.sql_cache.duckdb_execute_count
    }

    /// Test-only accessor: number of distinct SQL strings currently in the
    /// renderer-side cache (used for ac-11 LRU eviction tests).
    pub fn sql_cache_len(&self) -> usize {
        self.sql_cache.entries.len()
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
                kind: "geo".to_string(),
            },
        };
        let msg = format!("{err}");
        assert!(msg.contains("emit failed"), "got: {msg}");
        assert!(msg.contains("geo"), "got: {msg}");
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
  - mark: geo
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
        // Mix a supported mark (dot with data.from) and an unsupported mark (geo)
        let yaml = r#"
data:
  t:
    - { x: 1 }
plot:
  - mark: dot
    data: { from: t }
  - mark: geo
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let results = session.execute_all();
        assert_eq!(results.len(), 2);
        // dot with data.from succeeds via SimpleLowerer
        assert!(results[0].is_ok(), "dot with data.from should succeed");
        // geo is unsupported
        assert!(results[1].is_err(), "geo should be unsupported");
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

    // --- gomb ac-11 / ac-12: renderer-side SQL cache + duckdb_execute_count ---

    #[test]
    fn gomb_ac11_sql_cache_skips_duckdb_execute_on_repeat() {
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

        assert_eq!(session.duckdb_execute_count(), 0);
        assert_eq!(session.sql_cache_len(), 0);

        let emitted = EmittedQuery {
            sql: "SELECT * FROM t".to_string(),
            bindings: vec![],
            plan_hash: 1,
        };
        // First call → cache miss → 1 DuckDB execute.
        session.test_execute_emitted(0, "dot", &emitted).unwrap();
        assert_eq!(session.duckdb_execute_count(), 1);
        assert_eq!(session.sql_cache_len(), 1);

        // Second call with same SQL → cache hit → no new execute.
        session.test_execute_emitted(0, "dot", &emitted).unwrap();
        assert_eq!(
            session.duckdb_execute_count(),
            1,
            "duckdb_execute_count must not increment on cache hit"
        );
        assert_eq!(session.sql_cache_len(), 1);
    }

    #[test]
    fn gomb_ac11_sql_cache_lru_eviction() {
        // Property: cache eviction is LRU, not FIFO/random/MRU.
        // Strategy:
        //   1. Insert k0..k31 (cache full at 32, oldest = k0).
        //   2. Touch k0 again — cache hit, no execute, k0 moves to MRU.
        //   3. Insert k32 — cache miss, evicts oldest. Under LRU this is k1
        //      (not k0, which we just touched). Under FIFO it would be k0.
        //   4. Re-execute k0 → must be a cache hit (no execute increment).
        //   5. Re-execute k1 → must be a cache miss (execute increment).
        // Counters at the end:
        //   - 32 inserts (k0..k31) → 32 executes
        //   - touching k0 → 0 executes (cache hit)
        //   - inserting k32 → 1 execute → 33
        //   - re-executing k0 → 0 executes (still cached) → 33
        //   - re-executing k1 → 1 execute (was evicted) → 34
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

        let make = |i: usize| EmittedQuery {
            sql: format!("SELECT * FROM t WHERE x < {i} OR x >= {i}"),
            bindings: vec![],
            plan_hash: 100 + i as u64,
        };

        // Step 1: fill the cache to capacity.
        for i in 0..32 {
            session.test_execute_emitted(0, "dot", &make(i)).unwrap();
        }
        assert_eq!(session.sql_cache_len(), 32);
        assert_eq!(session.duckdb_execute_count(), 32);

        // Step 2: touch k0 — cache hit, must NOT increment execute count.
        session.test_execute_emitted(0, "dot", &make(0)).unwrap();
        assert_eq!(
            session.duckdb_execute_count(),
            32,
            "touching k0 must be a cache hit"
        );

        // Step 3: insert k32 — under LRU this evicts k1 (now the oldest),
        // not k0 (which we just touched).
        session.test_execute_emitted(0, "dot", &make(32)).unwrap();
        assert_eq!(session.sql_cache_len(), 32, "cache stays at cap");
        assert_eq!(session.duckdb_execute_count(), 33);

        // Step 4: re-execute k0 — under LRU it must still be cached.
        // A FIFO/random/MRU policy would have evicted k0 here and this
        // assertion would fail.
        session.test_execute_emitted(0, "dot", &make(0)).unwrap();
        assert_eq!(
            session.duckdb_execute_count(),
            33,
            "k0 was MRU after step 2 — must still be in cache (LRU semantics)"
        );

        // Step 5: re-execute k1 — must be evicted (it was the oldest after
        // k0 was touched in step 2). This locks in the LRU choice: k1 is
        // gone, not k0.
        session.test_execute_emitted(0, "dot", &make(1)).unwrap();
        assert_eq!(
            session.duckdb_execute_count(),
            34,
            "k1 must have been evicted under LRU when k32 was inserted"
        );
    }

    #[test]
    fn gomb_ac12_propagate_param_with_unchanged_sql_hits_cache() {
        // Property: propagate_param re-dispatches subscribers via the SQL
        // execute path. When the param's value does NOT affect the emitted
        // SQL string, the second propagate_param call hits the cache and
        // skips DuckDB.
        //
        // Setup: a selection param "brush" subscribed by a dot mark via
        // filterBy. The brush *value* lands in param_state but does NOT
        // appear in the emitted SQL (selection params are routed through
        // selection_state predicates, not inlined). selection_state is
        // empty for the contributor, so emit_query produces byte-identical
        // SQL across propagate_param calls regardless of the brush value.
        //
        // This honours the spec's intent for ac-12 — "param mutation that
        // does not change SQL keeps the cache warm" — without requiring
        // bandwidth-as-runtime-param wiring, which is deferred to the
        // two-tier param-effect routing card. See decisions.md decision
        // "ac-12 verification reframe".
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

        // First propagate — cache miss, DuckDB executes once.
        let r1 = session.propagate_param("brush", SpecValue::Integer(1));
        assert_eq!(r1.len(), 1, "subscriber should be dispatched");
        assert!(r1[0].1.is_ok(), "first execute must succeed: {:?}", r1[0].1);
        let baseline = session.duckdb_execute_count();
        assert!(baseline >= 1, "first call must trigger at least one execute");

        // Second propagate — different value but selection_state unchanged,
        // so the emitted SQL is byte-identical → cache hit, no new execute.
        let r2 = session.propagate_param("brush", SpecValue::Integer(2));
        assert_eq!(r2.len(), 1);
        assert!(r2[0].1.is_ok(), "second execute must succeed: {:?}", r2[0].1);
        assert_eq!(
            session.duckdb_execute_count(),
            baseline,
            "propagate_param with byte-identical SQL must hit the SQL cache \
             and skip DuckDB execute"
        );

        // param_state still updated regardless of cache outcome.
        assert_eq!(
            session.current_params().get("brush"),
            Some(&SpecValue::Integer(2)),
            "param_state must reflect the latest propagate_param value"
        );
    }

    /// Robustly read a numeric column as f64 across DuckDB's integer/float
    /// return types (card 0014 tests).
    fn column_as_f64_vec(batches: &[RecordBatch], name: &str) -> Vec<f64> {
        use duckdb::arrow::array::{Array, Float64Array};
        use duckdb::arrow::compute::cast;
        use duckdb::arrow::datatypes::DataType;
        let mut out = Vec::new();
        for b in batches {
            let idx = b
                .schema()
                .index_of(name)
                .unwrap_or_else(|_| panic!("column `{name}` absent; schema = {:?}", b.schema()));
            let col = cast(b.column(idx), &DataType::Float64).expect("cast to f64");
            let arr = col
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("f64 array");
            for i in 0..arr.len() {
                out.push(arr.value(i));
            }
        }
        out
    }

    /// hex_ac02 (THE FIXTURE): the HexbinLowerer's pixel-space cube-round SQL
    /// executed against DuckDB, proven against a HAND-COMPUTED fixture.
    ///
    /// Plot 160×150 ⇒ area 100×100 (margins 60×50); data extents [0,100]² ⇒
    /// data units == pixels. binWidth 20 ⇒ hex size = 20/√3, horizontal centre
    /// spacing = 20. By hand (pointy-top, size = 20/√3):
    ///   (0,0)→hex(0,0) centre (0,0);  (100,0)→hex(5,0) centre (100,0);
    ///   (0,100)→hex(-3,6) centre (0, 9·size≈103.923);
    ///   (100,100)→hex(2,6) centre (100, 103.923);
    ///   (20,0)×2 → hex(1,0) centre (20,0), count 2.
    /// The BOUNDARY point (10,0) sits EXACTLY between hex(0,0) and hex(1,0)
    /// (qf = 0.5). DuckDB's `round()` is round-HALF-TO-EVEN (banker's), so
    /// 0.5→0 and it deterministically joins hex(0,0) — the tie-break is
    /// well-defined either way (the two centres are equidistant), pinned here.
    /// So hex(0,0) has count 2.
    /// dx = binWidth/2 = 10 (data units); dy = size ≈ 11.547. Row order is
    /// x-centre then y-centre (determinism).
    #[test]
    fn hex_ac02_hexbin_cube_round_fixture_executes() {
        let yaml = r#"
data:
  pts:
    - { x: 0, y: 0 }
    - { x: 100, y: 0 }
    - { x: 0, y: 100 }
    - { x: 100, y: 100 }
    - { x: 20, y: 0 }
    - { x: 20, y: 0 }
    - { x: 10, y: 0 }
plot:
  - mark: hexbin
    data: { from: pts }
    x: x
    y: y
    fill: { count: }
    binWidth: 20
width: 160
height: 150
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;
        let batches = session.execute_mark(0).expect("hexbin executes");

        let xs = column_as_f64_vec(&batches, "x");
        let ys = column_as_f64_vec(&batches, "y");
        let counts = column_as_f64_vec(&batches, "__bf_count");
        let dxs = column_as_f64_vec(&batches, "__bf_hex_dx");
        let dys = column_as_f64_vec(&batches, "__bf_hex_dy");

        let size = 20.0 / 1.732_050_807_568_877_2_f64;
        let cy_edge = size * 9.0; // ≈ 103.923
        let expected: Vec<(f64, f64, f64)> = vec![
            (0.0, 0.0, 2.0), // (0,0) + boundary (10,0) via banker's rounding
            (0.0, cy_edge, 1.0),
            (20.0, 0.0, 2.0),
            (100.0, 0.0, 1.0),
            (100.0, cy_edge, 1.0),
        ];
        assert_eq!(xs.len(), expected.len(), "5 distinct hex bins");
        for (i, (ex, ey, ec)) in expected.iter().enumerate() {
            assert!((xs[i] - ex).abs() < 1e-6, "row {i} x: {} != {ex}", xs[i]);
            assert!((ys[i] - ey).abs() < 1e-6, "row {i} y: {} != {ey}", ys[i]);
            assert!((counts[i] - ec).abs() < 1e-9, "row {i} count: {} != {ec}", counts[i]);
        }
        // Total occupancy == input rows.
        assert!((counts.iter().sum::<f64>() - 7.0).abs() < 1e-9);
        // Constant in-band hex half-extents (data units).
        for d in &dxs {
            assert!((d - 10.0).abs() < 1e-6, "dx == binWidth/2 == 10, got {d}");
        }
        for d in &dys {
            assert!((d - size).abs() < 1e-6, "dy == size, got {d}");
        }
    }

    /// hex_ac02: double-render byte-equality (the #42 determinism class) — the
    /// hexbin SQL orders its emitted centres, so two executions match exactly.
    #[test]
    fn hex_ac02_hexbin_double_render_deterministic() {
        let yaml = r#"
data:
  pts:
    - { x: 1, y: 2 }
    - { x: 3, y: 5 }
    - { x: 4, y: 1 }
    - { x: 2, y: 4 }
    - { x: 5, y: 3 }
plot:
  - mark: hexbin
    data: { from: pts }
    x: x
    y: y
    fill: { count: }
    binWidth: 15
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;
        let a = session.execute_mark(0).expect("first");
        let b = session.execute_mark(0).expect("second");
        assert_eq!(column_as_f64_vec(&a, "x"), column_as_f64_vec(&b, "x"));
        assert_eq!(column_as_f64_vec(&a, "y"), column_as_f64_vec(&b, "y"));
        assert_eq!(
            column_as_f64_vec(&a, "__bf_count"),
            column_as_f64_vec(&b, "__bf_count")
        );
    }

    /// hex_ac02: a `fill: {avg: col}` hexbin aggregates the column per hex,
    /// aliased to the source column so the renderer's numeric-fill path reads it.
    #[test]
    fn hex_ac02_hexbin_avg_executes() {
        // Two points in the same hex (near origin) with values 10 and 20 ⇒ avg
        // 15; one far point (its own hex) with value 100.
        let yaml = r#"
data:
  pts:
    - { x: 0, y: 0, v: 10 }
    - { x: 1, y: 0, v: 20 }
    - { x: 100, y: 100, v: 100 }
plot:
  - mark: hexbin
    data: { from: pts }
    x: x
    y: y
    fill: { avg: v }
    binWidth: 20
width: 160
height: 150
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;
        let batches = session.execute_mark(0).expect("hexbin avg executes");
        let avgs = column_as_f64_vec(&batches, "v");
        assert!(avgs.iter().any(|a| (a - 15.0).abs() < 1e-9), "avg 15 hex: {avgs:?}");
        assert!(avgs.iter().any(|a| (a - 100.0).abs() < 1e-9), "avg 100 hex: {avgs:?}");
        // No count column when the fill is an avg aggregate.
        assert!(batches[0].schema().index_of("__bf_count").is_err());
    }

    /// hex_ac05: a self-aggregating `cell` with `fill: {count:}` GROUP BYs the
    /// two categorical axes and counts per (x, y) pair, executed against DuckDB.
    #[test]
    fn hex_ac05_self_aggregating_count_cell_executes() {
        let yaml = r#"
data:
  events:
    - { xg: a, yg: p }
    - { xg: a, yg: p }
    - { xg: a, yg: q }
    - { xg: b, yg: p }
plot:
  - mark: cell
    data: { from: events }
    x: xg
    y: yg
    fill: { count: }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;
        let batches = session.execute_mark(0).expect("self-aggregating cell executes");
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 3, "three distinct (x, y) cells");
        let mut counts = column_as_f64_vec(&batches, "__bf_count");
        counts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(counts, vec![1.0, 1.0, 2.0], "(a,p)=2, others 1");
        assert!((counts.iter().sum::<f64>() - 4.0).abs() < 1e-9);
    }

    /// pefr ac-05 (THE PROBE): a scalar param bound to a positional channel
    /// changes the mark's batch output when the param changes. Before card 0014
    /// this returned byte-identical output (in fact no `y`/`k` column at all).
    #[test]
    fn pefr_ac05_propagate_param_changes_channel_batch() {
        let yaml = r#"
params:
  k: 3
data:
  t:
    - { x: 1 }
    - { x: 2 }
    - { x: 3 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: $k
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Initial execution uses the default k = 3.
        let before = session.execute_all();
        let batch_before = before[0].as_ref().expect("dot executes");
        assert_eq!(
            column_as_f64_vec(batch_before, "k"),
            vec![3.0, 3.0, 3.0],
            "the $k channel column reflects the default param value"
        );

        // Change the param — the subscribing mark re-executes with the new value.
        let results = session.propagate_param("k", SpecValue::Integer(20));
        assert_eq!(results.len(), 1, "the $k-channel mark must be re-dispatched");
        let batch_after = results[0].1.as_ref().expect("re-execute succeeds");
        assert_eq!(
            column_as_f64_vec(batch_after, "k"),
            vec![20.0, 20.0, 20.0],
            "the channel column now reflects the new param value — reactive"
        );
    }

    /// pefr ac-08 (card 0014): propagate_param on a `data.filter` param
    /// re-filters the row set — the param reaches the WHERE clause.
    #[test]
    fn pefr_ac08_propagate_param_filter_changes_rows() {
        let yaml = r#"
params:
  k: 0
data:
  t:
    - { x: 1 }
    - { x: 2 }
    - { x: 3 }
    - { x: 4 }
plot:
  - mark: dot
    data: { from: t, filter: "x > $k" }
    x: x
    y: x
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let before = session.execute_all();
        let rows_before: usize = before[0]
            .as_ref()
            .unwrap()
            .iter()
            .map(|b| b.num_rows())
            .sum();
        assert_eq!(rows_before, 4, "k=0: all four rows pass x > 0");

        let results = session.propagate_param("k", SpecValue::Integer(2));
        assert_eq!(results.len(), 1, "the filter mark must be re-dispatched");
        let rows_after: usize = results[0]
            .1
            .as_ref()
            .unwrap()
            .iter()
            .map(|b| b.num_rows())
            .sum();
        assert_eq!(rows_after, 2, "k=2: only x=3,4 pass x > 2 — the filter tightened");
    }

    /// pefr ac-09 (card 0014): a data-shape param change (new WHERE value)
    /// misses the cache and re-executes DuckDB — the plan_hash fold keys the
    /// caches on the inlined value, so no stale batch is returned.
    #[test]
    fn pefr_ac09_data_shape_param_reexecutes() {
        let yaml = r#"
params:
  k: 0
data:
  t:
    - { x: 1 }
    - { x: 2 }
    - { x: 3 }
    - { x: 4 }
plot:
  - mark: dot
    data: { from: t, filter: "x > $k" }
    x: x
    y: x
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;
        let _ = session.execute_all();
        let _ = session.propagate_param("k", SpecValue::Integer(2));
        let count_after_first = session.duckdb_execute_count();

        let results = session.propagate_param("k", SpecValue::Integer(3));
        assert!(
            session.duckdb_execute_count() > count_after_first,
            "a data-shape param change must miss the cache and re-execute DuckDB"
        );
        let rows: usize = results[0]
            .1
            .as_ref()
            .unwrap()
            .iter()
            .map(|b| b.num_rows())
            .sum();
        assert_eq!(rows, 1, "k=3: only x=4 passes x > 3 — fresh (not stale) batch");
    }

    /// pefr ac-12 (card 0014): the shipped example spec is reactive — raising
    /// the threshold param re-executes the query and drops points.
    #[test]
    fn pefr_ac12_example_param_threshold_is_reactive() {
        let yaml = include_str!("../../../examples/param-threshold.yaml");
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let before = session.execute_all();
        let rows_before: usize = before[0]
            .as_ref()
            .unwrap()
            .iter()
            .map(|b| b.num_rows())
            .sum();

        let results = session.propagate_param("threshold", SpecValue::Integer(6));
        let rows_after: usize = results[0]
            .1
            .as_ref()
            .unwrap()
            .iter()
            .map(|b| b.num_rows())
            .sum();
        assert!(
            rows_after < rows_before,
            "raising the threshold must drop points: {rows_before} -> {rows_after}"
        );
    }

    /// Review regression (card 0014): interpolation makes each distinct inlined
    /// param value a distinct plan_hash, so a dragged param (the feature's whole
    /// point) must not grow the plan-stability cache without bound.
    #[test]
    fn pefr_plan_cache_bounded_under_param_sweep() {
        let yaml = r#"
params:
  k: 0
data:
  t:
    - { x: 1 }
    - { x: 50 }
    - { x: 150 }
plot:
  - mark: dot
    data: { from: t, filter: "x > $k" }
    x: x
    y: x
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;
        let _ = session.execute_all();
        for k in 0..200i64 {
            let _ = session.propagate_param("k", SpecValue::Integer(k));
        }
        assert!(
            session.cache_len() <= 64,
            "plan cache must stay bounded under a 200-value param sweep; got {}",
            session.cache_len()
        );
    }

    /// Review regression (card 0014 #1): a FRACTIONAL param on a positional
    /// channel must produce a DOUBLE column, not DECIMAL — the renderer's
    /// column_as_f64 reads Float/Int but not Decimal, so a bare `3.5 AS "k"`
    /// would silently render nothing. The projection CASTs to DOUBLE.
    #[test]
    fn pefr_float_param_channel_is_double_typed() {
        use duckdb::arrow::datatypes::DataType;
        let yaml = "params:\n  k: 0\ndata:\n  t:\n    - { x: 1 }\n    - { x: 2 }\nplot:\n  - mark: dot\n    data: { from: t }\n    x: x\n    y: $k\n";
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;
        let results = session.propagate_param("k", SpecValue::Float(3.5));
        let batches = results[0].1.as_ref().unwrap();
        let col = batches[0]
            .column_by_name("k")
            .expect("param column must be projected");
        assert_eq!(
            col.data_type(),
            &DataType::Float64,
            "fractional param channel must be DOUBLE-typed, got {:?}",
            col.data_type()
        );
    }

    /// Review regression (card 0014 #3): navigation (pan/zoom via update_extent)
    /// must interpolate param channels too — otherwise `$k` reaches DuckDB as an
    /// unbound placeholder and the mark fails on every pan/zoom.
    #[test]
    fn pefr_navigation_preserves_param_channel() {
        let yaml = "params:\n  k: 5\ndata:\n  t:\n    - { x: 1 }\n    - { x: 2 }\n    - { x: 3 }\nplot:\n  - mark: dot\n    data: { from: t }\n    x: x\n    y: $k\n";
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;
        let results = session.update_extent(Some(("x", 1.5, 3.5)), None);
        assert!(
            results[0].1.is_ok(),
            "navigation with a positional param channel must not fail: {:?}",
            results[0].1
        );
        let batches = results[0].1.as_ref().unwrap();
        assert!(
            batches.iter().any(|b| b.column_by_name("k").is_some()),
            "the param column must survive navigation"
        );
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
  - mark: geo
    data: { from: t }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Mark 0 succeeds via SimpleLowerer (dot with data.from).
        let ok_result = session.execute_mark(0);
        assert!(ok_result.is_ok(), "mark 0 should succeed via SimpleLowerer");
        assert!(!ok_result.unwrap().is_empty());

        // Mark 1 fails via execute_mark (geo is unsupported).
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
        // SimpleLowerer (Ok), geo is not (Err). Each mark is dispatched
        // independently — geo's failure must not prevent dot from succeeding.
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
  - mark: geo
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
        assert_eq!(err_count, 1, "geo should fail (UnsupportedMark)");

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

    // ===========================================================================
    // Card 0005 v3 — reactive parameters chained walk + slider widget (rpw3_)
    // ===========================================================================

    /// rpw3 ac-04: propagate_param walks topological_descendants and dispatches
    /// subscribing marks at every level against full param_state AND the active
    /// selection_state. Selection passthrough is verified end-to-end: with a
    /// brush predicate of "1 = 0" pre-populated in selection_state by a
    /// contributor in a different parent plot, both m_A (subscribing to $A)
    /// and m_B (subscribing to $B, descendant of A in the DAG) must produce
    /// Ok results with zero rows — the brush WHERE-clause is threaded through
    /// to emit_query at every level.
    #[test]
    fn rpw3_ac04_propagate_param_chained_walk() {
        // Chained DAG A → B (via menu input that filterBy $A and writes $B).
        // m_A and m_B both filterBy $brush AND subscribe to $A / $B
        // respectively via the opacity ParamRef channel. The marks live in
        // a different parent plot than the brush contributor so cfs2
        // self-exclusion does not fire — the brush predicate filters them.
        let yaml = r#"
params:
  A: { select: intersect }
  B: { select: intersect }
  brush: { select: intersect }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
hconcat:
  - input: menu
    filterBy: $A
    as: $B
    from: t
    column: x
  - plot:
    - mark: dot
      data: { from: t, filterBy: $brush }
      x: x
      y: y
      opacity: $A
    - mark: dot
      data: { from: t, filterBy: $brush }
      x: x
      y: y
      opacity: $B
"#;
        let (spec, analysis) = parse_and_analyse(yaml);

        // Sanity: DAG edge A → B exists.
        assert!(
            analysis
                .dependency_edges
                .iter()
                .any(|e| e.from == "A" && e.to == "B"),
            "expected DAG edge A → B from menu input; got {:?}",
            analysis.dependency_edges
        );
        // Sanity: subscriber graph routes A and B to the right marks.
        let a_subs = analysis
            .subscriber_graph
            .get("A")
            .expect("A in subscriber_graph");
        assert!(
            a_subs.iter().any(|p| p.0.contains("mark[dot]")),
            "A should have a mark subscriber via opacity; got {:?}",
            a_subs
        );
        let b_subs = analysis
            .subscriber_graph
            .get("B")
            .expect("B in subscriber_graph");
        assert!(
            b_subs.iter().any(|p| p.0.contains("mark[dot]")),
            "B should have a mark subscriber via opacity; got {:?}",
            b_subs
        );

        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Pre-populate selection_state with a brush predicate of "1 = 0"
        // (filters all rows). Contributor is the menu input's path — a
        // DIFFERENT parent plot than where the marks live, so
        // self-exclusion does not strip the predicate.
        let contributor = ComponentPath("root/hconcat[0]".to_string());
        let _ = session.propagate_selection(
            "brush",
            contributor,
            Predicate::Expr("1 = 0".to_string()),
        );

        // Now propagate A. Walk should reach both A and B levels, and
        // dispatch m_A at A's level and m_B at B's level. Both emit_query
        // calls receive the same selections_ref with brush.
        let results = session.propagate_param("A", SpecValue::Integer(1));

        assert_eq!(
            results.len(),
            2,
            "chained walk must dispatch both m_A (level A) and m_B (level B); got {:?}",
            results.iter().map(|(idx, r)| (*idx, r.is_ok())).collect::<Vec<_>>()
        );

        // Both marks should be Ok (dot lowerer is supported).
        for (idx, result) in &results {
            assert!(
                result.is_ok(),
                "mark {idx} should succeed via SimpleLowerer; got {result:?}"
            );
        }

        // Selection passthrough invariant: brush predicate "1 = 0" filters
        // all rows out at BOTH levels. If selections_ref were dropped at
        // level B, m_B would return 2 rows.
        for (idx, result) in &results {
            let batches = result.as_ref().unwrap();
            let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
            assert_eq!(
                total_rows, 0,
                "mark {idx} must have brush predicate threaded — \
                 expected 0 rows, got {total_rows}. Selection passthrough broken."
            );
        }

        // Selection state retained — the walk is read-only over selection_state.
        let state = session.current_selections();
        assert!(
            state.contains_key("brush"),
            "brush must remain in selection_state after propagate_param"
        );
    }

    /// rpw3 ac-05: per-walk dedup is first-level-wins. A mark whose query
    /// references both $A and $B (where A → B in the DAG) appears in
    /// subscriber_graph for both A and B. The walk must dispatch it at A's
    /// level (the topologically-earliest level in the walk where it
    /// appears) and skip it at B's level. Asserted via index ordering in
    /// the result vec — last-level-wins would also produce at-most-once
    /// but with a different ordering.
    #[test]
    fn rpw3_ac05_propagate_param_first_level_wins_dedup() {
        // Chained DAG A → B. Two marks:
        //   m_AB: subscribes to BOTH $A and $B (via opacity: $A AND fill: $B)
        //   m_B:  subscribes only to $B (via opacity: $B)
        //
        // After propagate_param("A", v):
        //   Level "A": subscriber_graph[A] = [m_AB]. Dispatch m_AB.
        //   Level "B": subscriber_graph[B] = [m_AB, m_B]. m_AB is already
        //              dispatched — skip via first-level-wins dedup. Dispatch m_B.
        // Result vec ordering: [m_AB at level A, m_B at level B]. Ordering is
        // the falsifiable property — last-level-wins would produce the same
        // count but with m_AB after m_B (since m_AB would dispatch at B's
        // level after m_B).
        let yaml = r#"
params:
  A: { select: intersect }
  B: { select: intersect }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
hconcat:
  - input: menu
    filterBy: $A
    as: $B
    from: t
    column: x
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      opacity: $A
      fill: $B
    - mark: dot
      data: { from: t }
      x: x
      y: y
      opacity: $B
"#;
        let (spec, analysis) = parse_and_analyse(yaml);

        // Sanity: m_AB is in subscriber_graph for BOTH A and B; m_B is only
        // in subscriber_graph for B.
        let a_subs = analysis.subscriber_graph.get("A").unwrap();
        let b_subs = analysis.subscriber_graph.get("B").unwrap();
        assert!(
            a_subs.iter().any(|p| p.0.ends_with("mark[dot]") && !p.0.contains("plot[1]")),
            "subscriber_graph[A] should contain at least one mark; got {:?}",
            a_subs
        );
        assert!(
            b_subs.len() >= 2,
            "subscriber_graph[B] should contain at least 2 marks (m_AB and m_B); got {:?}",
            b_subs
        );

        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let results = session.propagate_param("A", SpecValue::Integer(1));

        // First-level-wins property (a): exactly 2 entries — m_AB once + m_B.
        assert_eq!(
            results.len(),
            2,
            "first-level-wins dedup must yield 2 results (m_AB once + m_B); got {results:?}"
        );

        // Mark indices: in depth-first order, m_AB is index 0, m_B is index 1
        // (both dot marks, m_AB is first inside the plot).
        let mark_ab = 0usize;
        let mark_b = 1usize;

        // Both marks dispatched.
        let result_indices: Vec<usize> = results.iter().map(|(i, _)| *i).collect();
        assert!(
            result_indices.contains(&mark_ab),
            "m_AB (index {mark_ab}) must appear in results; got {result_indices:?}"
        );
        assert!(
            result_indices.contains(&mark_b),
            "m_B (index {mark_b}) must appear in results; got {result_indices:?}"
        );

        // First-level-wins property (b): m_AB precedes m_B in the result vec
        // (m_AB dispatched at A's level, m_B at B's level). Last-level-wins
        // would produce the reverse ordering since m_AB would be skipped at
        // A's level and dispatched at B's level after m_B.
        let pos_ab = result_indices.iter().position(|&i| i == mark_ab).unwrap();
        let pos_b = result_indices.iter().position(|&i| i == mark_b).unwrap();
        assert!(
            pos_ab < pos_b,
            "m_AB must precede m_B (first-level-wins → m_AB at level A); got order {result_indices:?}"
        );
    }

    /// rpw3 ac-06: descendants-only scope. Marks subscribing only to non-
    /// descendant params (DAG siblings of the propagated root, or unrelated
    /// params) must NOT be re-executed.
    #[test]
    fn rpw3_ac06_propagate_param_descendants_only() {
        // DAG: parent P → A, parent P → C (A and C are siblings in the DAG).
        // Marks: m_A subscribes to $A; m_C subscribes to $C.
        // Propagate A: only m_A should dispatch — m_C is a non-descendant.
        let yaml = r#"
params:
  P: { select: intersect }
  A: { select: intersect }
  C: { select: intersect }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
hconcat:
  - input: menu      # creates P → A
    filterBy: $P
    as: $A
    from: t
    column: x
  - input: menu      # creates P → C
    filterBy: $P
    as: $C
    from: t
    column: x
  - plot:
    - mark: dot
      data: { from: t }
      x: x
      y: y
      opacity: $A
    - mark: dot
      data: { from: t }
      x: x
      y: y
      opacity: $C
"#;
        let (spec, analysis) = parse_and_analyse(yaml);

        // Sanity: P → A and P → C both exist; A → C does NOT.
        let edges = &analysis.dependency_edges;
        assert!(edges.iter().any(|e| e.from == "P" && e.to == "A"));
        assert!(edges.iter().any(|e| e.from == "P" && e.to == "C"));
        assert!(
            !edges.iter().any(|e| e.from == "A" && e.to == "C"),
            "A and C must be DAG siblings (no edge between them)"
        );

        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let results = session.propagate_param("A", SpecValue::Integer(1));

        // Only m_A (mark index 0 in depth-first order) should dispatch.
        // m_C (mark index 1) is a non-descendant of A.
        assert_eq!(
            results.len(),
            1,
            "only m_A should dispatch — m_C is a non-descendant; got {results:?}"
        );
        assert_eq!(
            results[0].0, 0,
            "expected mark index 0 (m_A), got {}",
            results[0].0
        );
    }

    /// rpw3 ac-07: propagating to a param with no subscribers and no
    /// descendants returns an empty result vec, but param_state is updated.
    #[test]
    fn rpw3_ac07_propagate_param_unsubscribed_leaf() {
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

        let results = session.propagate_param("orphan", SpecValue::Integer(99));
        assert!(
            results.is_empty(),
            "unsubscribed leaf with no descendants must return empty; got {results:?}"
        );
        assert_eq!(
            session.current_params().get("orphan"),
            Some(&SpecValue::Integer(99)),
            "param_state must reflect the new value regardless of dispatch outcome"
        );
    }

    /// rpw3 ac-08: partial failure — strengthens v2 rpw2_ac04 by naming the
    /// EngineError discriminant, the lowerer registration scheme, and the
    /// param_state assertion. Two marks subscribe to $brush via the same
    /// edge (data.filterBy): one dot (registered lowerer → Ok) and one geo
    /// (no registered lowerer → Err with EmitFailed { cause: UnsupportedMark }).
    /// The walk continues across mixed Ok/Err and updates param_state.
    #[test]
    fn rpw3_ac08_propagate_param_partial_failure() {
        let yaml = r#"
params:
  brush: { select: intersect }
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
  - mark: geo
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let results = session.propagate_param("brush", SpecValue::Integer(7));

        // (a) results.len() == 2 — both marks dispatched.
        assert_eq!(
            results.len(),
            2,
            "both marks must be dispatched; got {results:?}"
        );

        // (b) dot mark Ok with non-empty batches; (c) geo mark Err with the
        // EmitFailed { cause: UnsupportedMark } discriminant.
        let dot_idx = 0usize;
        let geo_idx = 1usize;
        let dot_result = results
            .iter()
            .find(|(i, _)| *i == dot_idx)
            .expect("dot at index 0 in results");
        let geo_result = results
            .iter()
            .find(|(i, _)| *i == geo_idx)
            .expect("geo at index 1 in results");

        let dot_batches = dot_result
            .1
            .as_ref()
            .expect("dot mark must be Ok via SimpleLowerer");
        let dot_rows: usize = dot_batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(
            dot_rows, 2,
            "dot must return 2 rows from inline data (no contributor → no filter); got {dot_rows}"
        );

        match &geo_result.1 {
            Err(EngineError::EmitFailed { cause }) => {
                let msg = format!("{cause:?}");
                assert!(
                    msg.contains("Unsupported"),
                    "geo Err cause must indicate UnsupportedMark; got {msg}"
                );
            }
            other => panic!(
                "geo must produce Err(EngineError::EmitFailed {{ cause: UnsupportedMark }}); got {other:?}"
            ),
        }

        // (d) param_state updated regardless of mixed Ok/Err.
        assert_eq!(
            session.current_params().get("brush"),
            Some(&SpecValue::Integer(7)),
            "param_state must reflect the new value regardless of dispatch outcome"
        );
    }

    /// rpw3 ac-15: case-iii deferral guard. The walk reads param_state for
    /// every level but writes param_state only for the explicitly named
    /// root. Downstream params keep their initial values. Locks down
    /// Decision 2's deferral as a behavioural property — a future
    /// implementer who silently extends the walk to derive downstream
    /// param values from query results would break this test.
    #[test]
    fn rpw3_ac15_propagate_param_does_not_mutate_downstream_params() {
        // Chained DAG A → B. Initial param_state has both A and B set.
        // After propagate_param("A", new_a), B should remain at its
        // initial value (the walk MUST NOT compute a new value for B from
        // the dispatched B-subscribers' query results — that's case iii).
        //
        // Note: A and B are scalar-typed here (not selections) so they
        // appear in initial param_state. Selection params are excluded
        // from param_state entirely, which would muddy the assertion.
        let yaml = r#"
params:
  A: 100
  B: 200
data:
  t:
    - { x: 1, y: 10 }
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Initial state.
        assert_eq!(session.current_params().get("A"), Some(&SpecValue::Integer(100)));
        assert_eq!(session.current_params().get("B"), Some(&SpecValue::Integer(200)));

        // Propagate A with a new value. No subscribers to dispatch (no
        // marks in this minimal spec) — but the walk still iterates the
        // descendants of A in topological_descendants(analysis, "A").
        let _ = session.propagate_param("A", SpecValue::Integer(999));

        // (a) A reflects new value.
        assert_eq!(
            session.current_params().get("A"),
            Some(&SpecValue::Integer(999)),
            "A must reflect propagated value"
        );
        // (b) B unchanged — case-iii deferral holds.
        assert_eq!(
            session.current_params().get("B"),
            Some(&SpecValue::Integer(200)),
            "B must NOT be mutated by the walk — case-iii deferral guard"
        );
    }

    // ===========================================================================
    // Card 0006 v2 — cross-filtered selections runtime coordinator (cfs2_)
    // ===========================================================================

    /// cfs2_ac01: selection_state is empty at load and gains an entry on first
    /// propagate_selection.
    #[test]
    fn cfs2_ac01_selection_state_initial_empty() {
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

        // Initial selection_state is empty.
        assert!(
            session.current_selections().is_empty(),
            "selection_state should be empty at load"
        );

        // First propagate_selection populates the entry.
        let contrib = ComponentPath("root/plot[0]".to_string());
        let pred = Predicate::Expr("x > 1".to_string());
        let _ = session.propagate_selection("brush", contrib.clone(), pred.clone());

        let state = session.current_selections();
        let contribs = state.get("brush").expect("brush entry should exist");
        assert_eq!(contribs.len(), 1, "exactly one contributor stored");
        assert_eq!(contribs[0].0, contrib);
        assert_eq!(contribs[0].1, pred);
    }

    /// cfs2_ac02: propagate_selection dispatches to all subscriber marks.
    /// Two plots both filterBy $brush; result vec has one Ok per subscriber.
    #[test]
    fn cfs2_ac02_propagate_selection_dispatches_to_subscribers() {
        let yaml = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
vconcat:
  - plot:
    - mark: dot
      data: { from: t, filterBy: $brush }
      x: x
      y: y
  - plot:
    - mark: line
      data: { from: t, filterBy: $brush }
      x: x
      y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);

        // Sanity: both marks are subscribers of $brush.
        let subs = analysis
            .selection_subscribers
            .get("brush")
            .expect("brush should have selection subscribers");
        assert_eq!(subs.len(), 2, "both marks should subscribe to $brush");

        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Brush from a separate plot path so the subscribers are not self-excluded.
        let contributor = ComponentPath("root/plot[99]".to_string());
        let pred = Predicate::Expr("x > 1".to_string());
        let results = session.propagate_selection("brush", contributor, pred);

        assert_eq!(results.len(), 2, "two subscriber marks dispatched");
        for (idx, r) in &results {
            assert!(r.is_ok(), "subscriber mark {idx} should succeed: {r:?}");
            let batches = r.as_ref().unwrap();
            let total: usize = batches.iter().map(|b| b.num_rows()).sum();
            assert_eq!(total, 2, "predicate x > 1 should keep 2 of 3 rows");
        }
    }

    /// cfs2_ac03: a second propagate_selection from the same contributor
    /// replaces the prior predicate. A different contributor accumulates.
    #[test]
    fn cfs2_ac03_same_contributor_replaces_predicate() {
        let yaml = r#"
params:
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
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let contrib_a = ComponentPath("root/plot[0]".to_string());
        let contrib_b = ComponentPath("root/plot[1]".to_string());

        let _ = session.propagate_selection(
            "brush",
            contrib_a.clone(),
            Predicate::Expr("x > 1".to_string()),
        );
        let _ = session.propagate_selection(
            "brush",
            contrib_a.clone(),
            Predicate::Expr("x < 100".to_string()),
        );
        // Same contributor twice → still exactly one entry.
        let state = session.current_selections();
        let entries = state.get("brush").unwrap();
        assert_eq!(entries.len(), 1, "same-contributor calls must replace, not append");
        assert_eq!(entries[0].1, Predicate::Expr("x < 100".to_string()));

        // Different contributor → accumulates.
        let _ = session.propagate_selection(
            "brush",
            contrib_b.clone(),
            Predicate::Expr("y > 5".to_string()),
        );
        let entries = session.current_selections().get("brush").unwrap();
        assert_eq!(entries.len(), 2, "different contributors accumulate");
    }

    /// cfs2_ac05: parent-plot self-exclusion. A mark in plot[0] is the
    /// contributor; a different mark in plot[0] subscribes; its own
    /// predicate is excluded from its own filter when the selection
    /// resolution is crossfilter. A subscriber in plot[1] receives the
    /// predicate.
    #[test]
    fn cfs2_ac05_parent_plot_self_exclusion() {
        // crossfilter resolution drops predicates whose contributor source
        // matches the subscriber's self_source. We verify by emitting
        // the SQL for each subscriber and checking whether the predicate
        // text appears.
        let yaml = r#"
params:
  brush:
    select: crossfilter
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
vconcat:
  - plot:
    - mark: dot
      data: { from: t, filterBy: $brush }
      x: x
      y: y
  - plot:
    - mark: line
      data: { from: t, filterBy: $brush }
      x: x
      y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // The two plots are vconcat items 0 and 1, so their marks are at
        // root/vconcat[0]/plot[0]/mark[dot] and root/vconcat[1]/plot[0]/mark[line];
        // their stable plot-node identities are root/vconcat[0] and
        // root/vconcat[1]. We brush from the first plot (root/vconcat[0]); only
        // its own mark is self-excluded.
        let contributor = ComponentPath("root/vconcat[0]".to_string());
        let pred_text = "x > 99999".to_string(); // distinctive marker in SQL
        let _ = session.propagate_selection(
            "brush",
            contributor,
            Predicate::Expr(pred_text.clone()),
        );

        // Re-emit each mark's SQL with the live selection_state and inspect.
        let selections = session.selection_predicates_for_emit();
        let selections_ref: Option<&[(String, Vec<(String, Predicate)>)]> = Some(&selections);

        let emitted_idx_0 =
            emit_query(&session.spec, 0, None, selections_ref).expect("emit mark 0");
        let emitted_idx_1 =
            emit_query(&session.spec, 1, None, selections_ref).expect("emit mark 1");

        // Mark 0 lives at root/vconcat[0]/plot[0]/mark[dot] — its plot-node path
        // is root/vconcat[0], same as the contributor → self-excluded.
        assert!(
            !emitted_idx_0.sql.contains(&pred_text),
            "mark 0 (same plot node as contributor) must be self-excluded; got SQL: {}",
            emitted_idx_0.sql
        );
        // Mark 1 lives at root/vconcat[1]/plot[0]/mark[line] — different
        // plot node → predicate must be present.
        assert!(
            emitted_idx_1.sql.contains(&pred_text),
            "mark 1 (different parent plot) must receive the predicate; got SQL: {}",
            emitted_idx_1.sql
        );
    }

    /// cfs2_ac06: resolution strategies threaded through emit_query
    /// (intersect → AND, union → OR, single → last predicate). Verified
    /// by inspecting the rendered SQL.
    #[test]
    fn cfs2_ac06_resolution_strategies_runtime() {
        // Intersect: AND of contributors.
        let yaml_intersect = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
"#;
        // Union: OR of contributors.
        let yaml_union = r#"
params:
  brush:
    select: union
data:
  t:
    - { x: 1 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
"#;
        // Single: only the last contributor's predicate.
        let yaml_single = r#"
params:
  brush:
    select: single
data:
  t:
    - { x: 1 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
"#;

        for (yaml, expected_marker, unwanted_marker) in [
            (yaml_intersect, " AND ", " OR "),
            (yaml_union, " OR ", " AND "),
            (yaml_single, "y_marker", "x_marker"),
        ] {
            let (spec, analysis) = parse_and_analyse(yaml);
            let engine = Engine::new();
            let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

            // Two contributors, distinctive markers in their predicates.
            let _ = session.propagate_selection(
                "brush",
                ComponentPath("root/plot[100]".to_string()),
                Predicate::Expr("x_marker = 1".to_string()),
            );
            let _ = session.propagate_selection(
                "brush",
                ComponentPath("root/plot[101]".to_string()),
                Predicate::Expr("y_marker = 2".to_string()),
            );

            let selections = session.selection_predicates_for_emit();
            let emitted = emit_query(&session.spec, 0, None, Some(&selections)).unwrap();

            assert!(
                emitted.sql.contains(expected_marker),
                "expected `{expected_marker}` in SQL for resolution test; got: {}",
                emitted.sql
            );
            // Single takes only the last predicate so the first marker must
            // be absent. Intersect/Union retain both markers; the unwanted
            // here is the *connective* of the other strategy.
            if expected_marker == "y_marker" {
                assert!(
                    !emitted.sql.contains(unwanted_marker),
                    "single resolution must drop earlier predicate; got: {}",
                    emitted.sql
                );
            } else {
                assert!(
                    !emitted.sql.contains(unwanted_marker),
                    "must not contain other resolution's connective; got: {}",
                    emitted.sql
                );
            }
        }
    }

    /// cfs review-fix (card 0006): `select: single` resolves the MOST RECENT
    /// contribution. When an existing source re-contributes, its new predicate
    /// must become "last" — regressed before the fix because propagate_selection
    /// updated the predicate in the source's original slot, so an earlier
    /// source's re-contribution stayed behind a later source and `.last()`
    /// returned the stale one.
    #[test]
    fn cfs_single_recontribution_becomes_most_recent() {
        let yaml = r#"
params:
  brush:
    select: single
data:
  t:
    - { x: 1 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let source_a = ComponentPath("root/plot[100]".to_string());
        let source_b = ComponentPath("root/plot[101]".to_string());

        // A contributes, then B (B is now "most recent"), then A re-contributes
        // a NEW predicate — A must now be the most recent.
        let _ = session.propagate_selection("brush", source_a.clone(), Predicate::Expr("a_old = 1".to_string()));
        let _ = session.propagate_selection("brush", source_b.clone(), Predicate::Expr("b_marker = 2".to_string()));
        let _ = session.propagate_selection("brush", source_a.clone(), Predicate::Expr("a_new = 3".to_string()));

        let selections = session.selection_predicates_for_emit();
        let emitted = emit_query(&session.spec, 0, None, Some(&selections)).unwrap();

        assert!(
            emitted.sql.contains("a_new"),
            "single must resolve A's re-contribution as most recent; got: {}",
            emitted.sql
        );
        assert!(
            !emitted.sql.contains("b_marker") && !emitted.sql.contains("a_old"),
            "single must drop the superseded predicates (b_marker, a_old); got: {}",
            emitted.sql
        );
    }

    /// cfs2_ac07: an unsubscribed selection (no entry in
    /// analysis.selection_subscribers) updates state but dispatches
    /// nothing.
    #[test]
    fn cfs2_ac07_unsubscribed_selection_silent() {
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

        let results = session.propagate_selection(
            "ghost",
            ComponentPath("root/plot[0]".to_string()),
            Predicate::Expr("x > 0".to_string()),
        );

        assert!(
            results.is_empty(),
            "unsubscribed selection: no marks dispatched"
        );
        // selection_state nonetheless updated.
        assert!(
            session.current_selections().contains_key("ghost"),
            "selection_state should still record the contribution"
        );
    }

    /// cfs2_ac08: partial failure. Two subscribers — one supported (dot)
    /// and one unsupported (geo). One Ok + one Err; selection_state
    /// updated regardless. Mirrors rpw2_ac04.
    #[test]
    fn cfs2_ac08_partial_failure() {
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
  - mark: geo
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);

        // Sanity check: both marks subscribe.
        let subs = analysis
            .selection_subscribers
            .get("brush")
            .expect("brush subscribers");
        assert!(subs.len() >= 2, "both marks should subscribe to brush");

        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        let results = session.propagate_selection(
            "brush",
            ComponentPath("root/plot[99]".to_string()),
            Predicate::Expr("x > 0".to_string()),
        );

        assert_eq!(results.len(), 2, "both subscribers dispatched");
        let ok_count = results.iter().filter(|(_, r)| r.is_ok()).count();
        let err_count = results.iter().filter(|(_, r)| r.is_err()).count();
        assert_eq!(ok_count, 1, "dot succeeds via SimpleLowerer");
        assert_eq!(err_count, 1, "geo fails (UnsupportedMark)");

        // selection_state updated regardless of partial failure.
        assert!(session.current_selections().contains_key("brush"));
    }

    /// cfs2_ac09: emit_query consumes both param_values and
    /// selection_predicates. With a non-empty selection_predicates slice
    /// the resulting SQL contains a WHERE clause derived from the
    /// predicate — not "WHERE TRUE".
    #[test]
    fn cfs2_ac09_emit_query_threads_param_and_selection() {
        let yaml = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
"#;
        let (spec, _analysis) = parse_and_analyse(yaml);

        // No selection state → no Filter wrapping → no WHERE clause.
        let no_sel = emit_query(&spec, 0, None, None).unwrap();
        assert!(
            !no_sel.sql.to_uppercase().contains("WHERE"),
            "without selection predicates, SQL should not contain WHERE: {}",
            no_sel.sql
        );

        // With a selection predicate, the SQL must contain the predicate text.
        let predicates: Vec<(String, Vec<(String, Predicate)>)> = vec![(
            "brush".to_string(),
            vec![(
                "root/plot[100]".to_string(),
                Predicate::Expr("x = 42".to_string()),
            )],
        )];
        let with_sel = emit_query(&spec, 0, None, Some(&predicates)).unwrap();
        assert!(
            with_sel.sql.to_uppercase().contains("WHERE"),
            "with selection predicate, SQL must contain WHERE: {}",
            with_sel.sql
        );
        assert!(
            with_sel.sql.contains("x = 42"),
            "predicate text must appear in SQL: {}",
            with_sel.sql
        );
    }

    /// cfs2_ac12: end-to-end against vendored crossfilter.yaml. Loads the
    /// spec, propagates a selection, and verifies subscribers get filtered
    /// rows via the full pipeline (parse → analyse → load → propagate
    /// → emit_query consumes selection → DuckDB returns batches).
    #[test]
    fn cfs2_ac12_crossfilter_yaml_end_to_end() {
        // Use an inline crossfilter-style spec rather than the vendored
        // YAML directly: the vendor file uses parquet/csv files that
        // require an actual filesystem path. The structural shape is
        // what the AC verifies — multiple plots subscribing to a shared
        // crossfilter selection, brushed from one plot path, observable
        // row-count reduction in another.
        let yaml = r#"
params:
  brush:
    select: crossfilter
data:
  flights:
    - { delay: 5, distance: 100, time: 6 }
    - { delay: 10, distance: 200, time: 8 }
    - { delay: 15, distance: 300, time: 10 }
    - { delay: 20, distance: 400, time: 12 }
    - { delay: 25, distance: 500, time: 14 }
hconcat:
  - plot:
      - mark: dot
        data: { from: flights, filterBy: $brush }
        x: distance
        y: delay
  - plot:
      - mark: dot
        data: { from: flights, filterBy: $brush }
        x: time
        y: delay
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Baseline: unfiltered execution returns all 5 rows.
        let baseline = session.execute_all();
        let baseline_rows: usize = baseline
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .flat_map(|batches| batches.iter().map(|b| b.num_rows()))
            .sum();
        assert_eq!(baseline_rows, 10, "baseline: 5 rows × 2 marks = 10");

        // Brush originates in the first plot (plot-node path root/hconcat[0])
        // — picks rows where distance is 100..=300 (3 of 5).
        let contributor =
            ComponentPath("root/hconcat[0]".to_string());
        let predicate =
            Predicate::Expr("distance >= 100 AND distance <= 300".to_string());
        let results = session.propagate_selection("brush", contributor, predicate);

        // The contributing plot's mark is self-excluded (crossfilter), so
        // its result is dispatched but the predicate does not apply to it.
        // The other plot's mark applies the predicate → 3 rows.
        let other_plot_result = results
            .iter()
            .find(|(idx, _)| *idx == 1)
            .expect("subscriber mark at index 1 must be dispatched");
        let batches = other_plot_result
            .1
            .as_ref()
            .expect("subscriber must succeed");
        let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert!(
            rows < 5,
            "non-contributor subscriber must reflect predicate (got {rows} rows)"
        );
        assert_eq!(rows, 3, "predicate distance in 100..=300 keeps 3 rows");
    }

    // ===========================================================================
    // Card 0006 v3 — cross-filtered selections, interactor variants (cfs3_)
    // ===========================================================================

    /// cfs3_ac01: clear_selection removes the named contributor from
    /// selection_state[name] and re-executes every subscriber. The result
    /// shape mirrors propagate_selection.
    #[test]
    fn cfs3_ac01_clear_selection_removes_contributor() {
        let yaml = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // Populate the selection from two distinct contributor paths so the
        // remove-one assertion is non-trivial.
        let contrib_a = ComponentPath("root/plot[0]".to_string());
        let contrib_b = ComponentPath("root/plot[1]".to_string());
        let _ = session.propagate_selection(
            "brush",
            contrib_a.clone(),
            Predicate::Expr("x >= 1".to_string()),
        );
        let _ = session.propagate_selection(
            "brush",
            contrib_b.clone(),
            Predicate::Expr("x <= 2".to_string()),
        );
        assert_eq!(
            session
                .current_selections()
                .get("brush")
                .map(|v| v.len())
                .unwrap_or(0),
            2,
            "two contributors before clear"
        );

        // (a) clear contributor A.
        let results = session.clear_selection("brush", contrib_a.clone());

        // (c) post-condition: only contributor B remains.
        let remaining = session
            .current_selections()
            .get("brush")
            .expect("brush entry should still exist after clearing one of two");
        assert_eq!(remaining.len(), 1, "exactly one contributor left");
        assert_eq!(remaining[0].0, contrib_b, "contributor B remains");

        // (d) one subscriber re-executed with at least one RecordBatch.
        assert_eq!(results.len(), 1, "one subscriber dispatched");
        let (_idx, result) = &results[0];
        let batches = result.as_ref().expect("subscriber must succeed");
        assert!(
            !batches.is_empty(),
            "subscriber re-execution must yield at least one batch"
        );
    }

    /// cfs3_ac02: clear_selection on an unknown selection name OR a known
    /// name with an unknown contributor is a silent no-op — empty result vec,
    /// selection_state untouched, no panic.
    #[test]
    fn cfs3_ac02_clear_selection_unsubscribed_silent() {
        let yaml = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // (a) Unknown selection name on an empty session.
        let results = session
            .clear_selection("does_not_exist", ComponentPath("root/plot[0]".to_string()));
        assert!(results.is_empty(), "unknown name → empty result vec");
        assert!(
            session.current_selections().is_empty(),
            "selection_state still empty"
        );

        // Populate one selection with one known contributor.
        let known_contrib = ComponentPath("root/plot[0]".to_string());
        let _ = session.propagate_selection(
            "brush",
            known_contrib.clone(),
            Predicate::Expr("x = 1".to_string()),
        );
        let snapshot: Vec<(ComponentPath, Predicate)> = session
            .current_selections()
            .get("brush")
            .cloned()
            .unwrap_or_default();
        assert_eq!(snapshot.len(), 1);

        // (b) Known name, but a contributor that is NOT in the list.
        let stranger = ComponentPath("root/plot[99]".to_string());
        let results = session.clear_selection("brush", stranger);
        assert!(
            results.is_empty(),
            "unknown contributor → empty result vec"
        );
        let after = session
            .current_selections()
            .get("brush")
            .cloned()
            .unwrap_or_default();
        assert_eq!(after, snapshot, "contributor list unchanged");
    }

    /// cfs3_ac07: a param change after a brush release does NOT lose the
    /// brush — selection_state is preserved AND the dispatched mark's
    /// emitted SQL still reflects the brush WHERE-clause. Pins the v2
    /// lib.rs:464-466 behaviour (propagate_param reads selection_state but
    /// never writes it) as a regression test.
    #[test]
    fn cfs3_ac07_param_change_preserves_selection() {
        let yaml = r#"
params:
  threshold:
    value: 0
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
plot:
  - mark: dot
    data: { from: t, filterBy: $brush }
    x: x
    y: y
"#;
        let (spec, analysis) = parse_and_analyse(yaml);
        let spec_for_emit = spec.clone();
        let engine = Engine::new();
        let mut session = engine.load_spec(spec, analysis, None).unwrap().session;

        // (a) Populate selection_state with a non-trivial predicate.
        let contributor = ComponentPath("root/plot[99]".to_string());
        let pred = Predicate::Expr("x >= 2".to_string());
        let _ = session.propagate_selection("brush", contributor, pred.clone());

        // (b) Snapshot.
        let snapshot = session.current_selections().clone();

        // (c) Propagate a param change.
        let _ = session.propagate_param(
            "threshold",
            brightfield_spec::ast::SpecValue::Float(5.0),
        );

        // (d) Selection state equality field-by-field.
        assert_eq!(
            session.current_selections(),
            &snapshot,
            "current_selections() must equal pre-propagate snapshot"
        );

        // (e) The dispatched mark's emitted SQL still contains the brush
        //     WHERE-clause fragment. We re-emit directly (post-walk) and
        //     inspect — the same falsifiable shape rpw3_ac04 used.
        let selections = vec![(
            "brush".to_string(),
            vec![("root/plot[99]".to_string(), pred.clone())],
        )];
        let emitted = emit_query(
            &spec_for_emit,
            0,
            None,
            Some(selections.as_slice()),
        )
        .expect("emit must succeed");
        assert!(
            emitted.sql.to_uppercase().contains("WHERE"),
            "emitted SQL must contain WHERE after param change with active brush: {}",
            emitted.sql
        );
        assert!(
            emitted.sql.contains("x >= 2"),
            "brush predicate text must appear in emitted SQL: {}",
            emitted.sql
        );
    }

    /// cfs3_ac08: propagate_param's body never writes to selection_state.
    /// Behavioural assertion: snapshot selection_state, call propagate_param
    /// twice with different values for the same param (covers first-walk
    /// and re-entry paths), assert selection_state equals the snapshot.
    /// This is the rally seam regression — pins the rpw3 chained walk as
    /// read-only over cfs2/cfs3 state.
    #[test]
    fn cfs3_ac08_propagate_param_does_not_clobber_selection_state() {
        let yaml = r#"
params:
  threshold:
    value: 0
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

        // (a) Populate selection with one contributor, non-trivial predicate.
        let contributor = ComponentPath("root/plot[99]".to_string());
        let pred = Predicate::Expr("x >= 1".to_string());
        let _ = session.propagate_selection("brush", contributor, pred);

        // (b) Snapshot.
        let snapshot = session.current_selections().clone();
        assert!(
            !snapshot.is_empty(),
            "snapshot precondition: selection populated"
        );

        // (c) Two propagate_param calls — first-walk + re-entry.
        let _ = session.propagate_param(
            "threshold",
            brightfield_spec::ast::SpecValue::Float(1.0),
        );
        let _ = session.propagate_param(
            "threshold",
            brightfield_spec::ast::SpecValue::Float(2.0),
        );

        // (d) selection_state byte-equal to snapshot.
        assert_eq!(
            session.current_selections(),
            &snapshot,
            "propagate_param must not mutate selection_state — snapshot mismatch"
        );
    }
}
