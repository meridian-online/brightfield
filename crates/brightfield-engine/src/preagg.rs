//! Automatic pre-aggregation — the engine half of the cube layer.
//!
//! `brightfield_sql::cube` is the pure half: it decides whether a mark's plan
//! decomposes into a data cube and constructs the build / re-query SQL. This
//! module is the orchestration half, owned by the live [`Session`]:
//!
//! - **Trigger.** When a selection interaction carries a *structured* clause
//!   ([`Predicate::Interval`] / [`Predicate::Point`]), the propagation path
//!   calls `Session::preagg_prepare` before dispatching re-queries. For each
//!   subscriber mark it derives a cube, materialises it once as a
//!   session-scoped TEMP table, and registers a *serve*: the exact direct SQL
//!   the dispatch is about to execute, paired with the cube re-query that
//!   produces the identical result set.
//! - **Serving.** `execute_emitted` consults the serve table on a SQL-cache
//!   miss: when the emitted SQL matches a registered serve byte-for-byte, the
//!   cube re-query runs instead of the direct query and the batches are cached
//!   under the direct SQL key — every consumer downstream of the one execution
//!   choke point is served from the cube without knowing it exists.
//! - **Fallback.** Every bail — unstructured clause, non-decomposable plan,
//!   probe/build/serve failure — falls through to the direct query. Nothing
//!   breaks; it only slows. The spec never declares any of this.
//!
//! Cubes are **session-scoped**: TEMP tables on the session's own connection,
//! deduplicated by their build SQL, LRU-capped, and dropped when the spec is
//! reloaded. A cube can never outlive the state it was derived from.

use std::collections::{HashMap, HashSet, VecDeque};

use brightfield_spec::analysis::{plot_node_path, ComponentPath};
use brightfield_spec::ast::ParamNode;
use brightfield_sql::cube;
use brightfield_sql::emit::{emit_query, interpolate_sql, lower_mark_plan, mark_selection_name};
use brightfield_sql::ir::{Predicate, SelectionPredicate, SelectionResolution};

use crate::Session;

/// Most cubes kept materialised at once; the least recently used is dropped.
const CUBE_CAP: usize = 8;

/// Executed-SQL log capacity (diagnostics; the proof tests read it).
const LOG_CAP: usize = 256;

/// Counters describing the pre-aggregation layer's behaviour — the
/// query-count half of the proof that interactions hit the cube.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PreAggStats {
    /// Cubes materialised (one CREATE TEMP TABLE each).
    pub cubes_built: usize,
    /// Mark re-queries served from a cube instead of the base table.
    pub cube_hits: usize,
    /// Cube builds (probe or CREATE) that failed; their build SQL is
    /// remembered and never retried this session.
    pub build_failures: usize,
    /// Registered serves whose cube re-query failed at execution time and
    /// fell back to the direct query.
    pub serve_failures: usize,
}

/// One registered substitution: the direct SQL a dispatch is about to run,
/// and the cube re-query that produces the identical result.
struct CubeServe {
    direct_sql: String,
    cube_sql: String,
}

/// One materialised cube, identified by its (param-interpolated) build SQL.
struct CubeEntry {
    build_sql: String,
    table: String,
}

/// The session's pre-aggregation state.
#[derive(Default)]
pub(crate) struct PreAgg {
    /// mark index → the serve prepared for the CURRENT selection state.
    serves: HashMap<usize, CubeServe>,
    /// Materialised cubes, least recently used first.
    cubes: Vec<CubeEntry>,
    /// Build SQL that failed to materialise — never retried this session.
    failed_builds: HashSet<String>,
    /// Monotonic id for TEMP table names.
    next_id: u64,
    /// Executed-SQL log (bounded), fed by the execution choke points.
    log: VecDeque<String>,
    pub(crate) stats: PreAggStats,
}

impl PreAgg {
    pub(crate) fn log_sql(&mut self, sql: &str) {
        if self.log.len() >= LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(sql.to_string());
    }

    /// The serve registered for `mark_index` when its direct SQL is exactly
    /// `sql` — the byte-precise match that makes false substitution impossible.
    pub(crate) fn serve_for(&self, mark_index: usize, sql: &str) -> Option<String> {
        self.serves
            .get(&mark_index)
            .filter(|s| s.direct_sql == sql)
            .map(|s| s.cube_sql.clone())
    }

    pub(crate) fn drop_serve(&mut self, mark_index: usize) {
        self.serves.remove(&mark_index);
    }
}

impl Session {
    /// The pre-aggregation layer's behaviour counters.
    #[must_use]
    pub fn preagg_stats(&self) -> &PreAggStats {
        &self.preagg.stats
    }

    /// The recent SQL statements this session sent to DuckDB through its
    /// execution choke points (bounded log, oldest first) — the surface the
    /// served-from-the-cube proof asserts against.
    #[must_use]
    pub fn executed_sql(&self) -> Vec<String> {
        self.preagg.log.iter().cloned().collect()
    }

    /// Clear the executed-SQL log (scoping a proof window).
    pub fn clear_executed_sql(&mut self) {
        self.preagg.log.clear();
    }

    /// Derive / refresh cube serves for `name`'s subscriber marks after
    /// `contributor`'s clause was stored. Called by the selection propagation
    /// path — the coordinator-side trigger; nothing in the spec declares it.
    ///
    /// Every early return leaves the affected marks WITHOUT a serve, which
    /// means the dispatch runs the direct query — the transparent fallback.
    pub(crate) fn preagg_prepare(&mut self, name: &str, contributor: &ComponentPath) {
        let marks = self.selection_subscriber_marks(name);
        if marks.is_empty() {
            return;
        }
        // A new clause for this selection supersedes every serve prepared for
        // the previous state; they are re-registered below when still derivable.
        for idx in &marks {
            self.preagg.serves.remove(idx);
        }

        let Some(entries) = self.selection_state.get(name) else {
            return;
        };
        let entries: Vec<(ComponentPath, Predicate)> = entries.clone();
        let Some((_, active)) = entries.iter().find(|(p, _)| p == contributor) else {
            return;
        };
        let active = active.clone();
        if cube::active_clauses(&active).is_none() {
            return;
        }
        let resolution = match self.spec.params.get(name) {
            Some(ParamNode::Selection(sel)) => SelectionResolution::from(sel.select),
            _ => return,
        };

        for idx in marks {
            if let Some(serve) =
                self.preagg_prepare_one(idx, name, contributor, &entries, resolution, &active)
            {
                self.preagg.serves.insert(idx, serve);
            }
        }
    }

    /// Prepare one mark's serve: resolve its view of the selection into
    /// (active clause ∧ rest), derive the cube, materialise it, and pair the
    /// re-query with the exact direct SQL the dispatch will emit. `None`
    /// bails to the direct query.
    fn preagg_prepare_one(
        &mut self,
        mark_index: usize,
        name: &str,
        contributor: &ComponentPath,
        entries: &[(ComponentPath, Predicate)],
        resolution: SelectionResolution,
        active: &Predicate,
    ) -> Option<CubeServe> {
        // The mark must subscribe to THIS selection via filterBy (a
        // highlight-only subscriber re-executes but is not filter-shaped).
        let (fb_name, mark_path) = mark_selection_name(&self.spec, mark_index)?;
        if fb_name != name {
            return None;
        }
        let self_source = plot_node_path(&mark_path);

        // The contributors this mark's compiled predicate actually sees.
        let visible: Vec<&(ComponentPath, Predicate)> = match resolution {
            SelectionResolution::Crossfilter => {
                entries.iter().filter(|(p, _)| p.0 != self_source).collect()
            }
            _ => entries.iter().collect(),
        };
        // Cross-filter-untouched mark: its predicate does not include the
        // active clause, so its result did not change — skip.
        visible.iter().find(|(p, _)| p == contributor)?;

        let others: Vec<Predicate> = visible
            .iter()
            .filter(|(p, _)| p != contributor)
            .map(|(_, pred)| pred.clone())
            .collect();
        let rest: Option<Predicate> = match resolution {
            SelectionResolution::Crossfilter | SelectionResolution::Intersect => {
                if others.is_empty() {
                    None
                } else {
                    Some(Predicate::And(others))
                }
            }
            SelectionResolution::Single => {
                // Most-recent contributor wins; serve only when that is the
                // active clause (it was just pushed to the tail).
                let (last_path, _) = visible.last()?;
                if last_path != contributor {
                    return None;
                }
                None
            }
            // OR-composition does not decompose into cube ∧ rest.
            SelectionResolution::Union => {
                if others.is_empty() {
                    None
                } else {
                    return None;
                }
            }
        };

        // Derive from the mark's pre-selection plan.
        let (plan, _) = lower_mark_plan(&self.spec, mark_index).ok()?;
        let derivation = cube::derive_cube(&plan, active, rest.as_ref())?;

        // The exact direct SQL the dispatch will execute for this mark under
        // the CURRENT state — emitted by the same function the dispatch uses,
        // so the serve can only match the query it substitutes.
        let params_owned = self.param_state.clone();
        let params_ref = if params_owned.is_empty() {
            None
        } else {
            Some(&params_owned)
        };
        let selections = self.selection_predicates_for_emit();
        let selections_ref: Option<&[SelectionPredicate]> = if selections.is_empty() {
            None
        } else {
            Some(selections.as_slice())
        };
        let direct_sql = emit_query(&self.spec, mark_index, params_ref, selections_ref)
            .ok()?
            .sql;

        // Centering probe (regression aggregates only).
        let centers: Vec<f64> = match derivation.probe_sql() {
            Some(probe) => {
                let probe_sql = self.preagg_interp(probe);
                match self.preagg_probe(&probe_sql) {
                    Some(row) => row,
                    None => {
                        self.preagg.stats.build_failures += 1;
                        return None;
                    }
                }
            }
            None => Vec::new(),
        };

        let sqls = derivation.finalize(&centers)?;
        let build_sql = self.preagg_interp(&sqls.build_select);
        let table = self.preagg_ensure_cube(&build_sql)?;
        let cube_sql = sqls.query_select(&table)?;

        Some(CubeServe {
            direct_sql,
            cube_sql,
        })
    }

    /// Interpolate live scalar params into cube SQL — the same substitution
    /// the direct emission applies, so both sides agree on param handling.
    fn preagg_interp(&self, sql: &str) -> String {
        if self.param_state.is_empty() {
            sql.to_string()
        } else {
            interpolate_sql(sql, &self.param_state)
        }
    }

    /// Run the centering probe: one row of DOUBLE columns (NULL → 0.0).
    fn preagg_probe(&mut self, sql: &str) -> Option<Vec<f64>> {
        use duckdb::arrow::array::{Array, Float64Array};
        use duckdb::arrow::compute::cast;
        use duckdb::arrow::datatypes::DataType;

        self.preagg.log_sql(sql);
        let batches = self.query_arrow_raw(sql).ok()?;
        let batch = batches.iter().find(|b| b.num_rows() > 0)?;
        let mut out = Vec::with_capacity(batch.num_columns());
        for i in 0..batch.num_columns() {
            let col = cast(batch.column(i), &DataType::Float64).ok()?;
            let arr = col.as_any().downcast_ref::<Float64Array>()?;
            out.push(if arr.is_null(0) { 0.0 } else { arr.value(0) });
        }
        Some(out)
    }

    /// The TEMP table holding `build_sql`'s materialisation — reused when
    /// already built this session, created otherwise (evicting the least
    /// recently used cube beyond the cap). `None` when this build has already
    /// failed once, or fails now.
    fn preagg_ensure_cube(&mut self, build_sql: &str) -> Option<String> {
        if self.preagg.failed_builds.contains(build_sql) {
            return None;
        }
        if let Some(pos) = self
            .preagg
            .cubes
            .iter()
            .position(|c| c.build_sql == build_sql)
        {
            // Reuse; refresh LRU position.
            let entry = self.preagg.cubes.remove(pos);
            let table = entry.table.clone();
            self.preagg.cubes.push(entry);
            return Some(table);
        }

        while self.preagg.cubes.len() >= CUBE_CAP {
            let evicted = self.preagg.cubes.remove(0);
            let drop_sql = format!("DROP TABLE IF EXISTS \"{}\"", evicted.table);
            self.preagg.log_sql(&drop_sql);
            let _ = self.conn.execute_batch(&drop_sql);
        }

        let table = format!("__bf_preagg_{}", self.preagg.next_id);
        self.preagg.next_id += 1;
        let create = format!("CREATE TEMP TABLE \"{table}\" AS {build_sql}");
        self.preagg.log_sql(&create);
        match self.conn.execute_batch(&create) {
            Ok(()) => {
                self.preagg.stats.cubes_built += 1;
                self.preagg.cubes.push(CubeEntry {
                    build_sql: build_sql.to_string(),
                    table: table.clone(),
                });
                Some(table)
            }
            Err(_) => {
                self.preagg.stats.build_failures += 1;
                self.preagg.failed_builds.insert(build_sql.to_string());
                None
            }
        }
    }

    /// Retire the whole pre-aggregation state: drop every cube table and
    /// forget serves and failure memory. Called on spec reload — a cube never
    /// survives the state it was derived from.
    pub(crate) fn preagg_retire_all(&mut self) {
        let tables: Vec<String> = self.preagg.cubes.iter().map(|c| c.table.clone()).collect();
        for table in tables {
            let drop_sql = format!("DROP TABLE IF EXISTS \"{table}\"");
            self.preagg.log_sql(&drop_sql);
            let _ = self.conn.execute_batch(&drop_sql);
        }
        self.preagg.cubes.clear();
        self.preagg.serves.clear();
        self.preagg.failed_builds.clear();
    }
}
