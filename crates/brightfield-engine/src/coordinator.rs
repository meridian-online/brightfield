//! The push-down interaction seam: an interaction resolves to a predicate (or a
//! param value) that is pushed into DuckDB, and a re-query.
//!
//! This is the coordinator layer named by Mosaic's architecture — the thing that
//! "translates interactions into database queries." A brush, a filter, a slider
//! or a selection never filters a materialised result set client-side; it
//! resolves to a predicate the engine wraps into a SQL `WHERE`, and the
//! affected marks re-execute. That is what makes interaction latency roughly
//! independent of row count.
//!
//! Two entry points sit on the same [`Session`] re-query logic:
//!
//! - [`Coordinator`] — synchronous. Owns the live [`Session`], stamps a
//!   monotonic [`Generation`] on every applied interaction, and re-queries in
//!   place. This is what a one-window presenter drives on its own thread.
//! - [`QueryLoop`] — the off-UI-thread worker. Owns a [`Coordinator`] on a
//!   dedicated thread and is driven over channels. It **coalesces** a backlog of
//!   interactions to the latest (a superseded interaction is never executed),
//!   **interrupts** an in-flight DuckDB query when a newer interaction arrives,
//!   and stamps each result with its generation so the consumer drops any result
//!   older than the freshest it has applied. Together these guarantee a
//!   sustained drag never presents a frame from a superseded query.
//!
//! ## Off the UI thread — decided here, with its reason
//!
//! DuckDB's [`Connection`](duckdb::Connection) is `Send`, so a [`Session`] moves
//! onto a worker thread cleanly, and the connection exposes an
//! [`InterruptHandle`](duckdb::InterruptHandle) (`Send + Sync`) that cancels the
//! query in flight from another thread. The interaction requirement forces the
//! move: a sustained slider drag issues re-queries faster than a large one
//! completes, so running them on the UI thread would freeze the drag for each
//! query's duration. The engine therefore offers [`QueryLoop`] as the sanctioned
//! interaction path — the mechanism (Send session + coalescing request channel +
//! interrupt + generation-stamped results) lives here so a UI only has to pump
//! channels. The synchronous [`Coordinator`] remains for one-shot composition,
//! previews, and headless tests where there is no concurrent interaction to
//! supersede.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;

use brightfield_spec::analysis::{ComponentPath, SpecAnalysis};
use brightfield_spec::ast::{Spec, SpecValue};

use crate::error::EngineError;
use crate::{DispatchResult, Engine, RecordBatch, Session, SqlPredicate};

/// A monotonically increasing stamp identifying one applied interaction — the
/// version of the materialisation state after that interaction. The consumer
/// uses it to drop a result older than the freshest it has already presented.
pub type Generation = u64;

/// An interaction resolved to exactly what the query layer consumes: a predicate
/// (for a brush / selection), a retraction (clearing a contributor), or a scalar
/// param value (a slider). The UI resolves geometry-to-predicate; the engine
/// pushes it into DuckDB.
#[derive(Debug, Clone, PartialEq)]
pub enum Interaction {
    /// A brush / point / legend contribution to a named selection. `contributor`
    /// is the stable plot-node path that drives crossfilter self-exclusion.
    Select {
        /// The selection name (the `$brush` in `filterBy: $brush`).
        name: String,
        /// The contributing component's parent-plot path.
        contributor: ComponentPath,
        /// The predicate this contributor pushes into DuckDB.
        predicate: SqlPredicate,
    },
    /// Retract a contributor's predicate from a selection (brush released).
    ClearSelect {
        /// The selection name.
        name: String,
        /// The contributor whose predicate is retracted.
        contributor: ComponentPath,
    },
    /// A scalar parameter change (a slider drag step).
    SetParam {
        /// The parameter name.
        name: String,
        /// The new value.
        value: SpecValue,
    },
}

/// The outcome of applying one interaction: the generation it produced and the
/// re-query results of every mark it affected (partial failure is per-mark).
#[derive(Debug)]
pub struct Requery {
    /// The generation stamped on this application — strictly increasing per
    /// coordinator, so the consumer can order and drop stale results.
    pub generation: Generation,
    /// One `(mark_index, Result)` per affected mark.
    pub affected: Vec<DispatchResult>,
}

/// Owns the live [`Session`] and resolves interactions to push-down re-queries.
///
/// Every applied interaction bumps [`Coordinator::generation`]; the generation
/// is the version of the materialisation the surfaces now read from.
pub struct Coordinator {
    session: Session,
    generation: Generation,
}

impl Coordinator {
    /// Load a spec into a fresh session and hold it live for interaction.
    ///
    /// # Errors
    ///
    /// Propagates any [`Engine::load_spec`] failure (connection, DDL, emit).
    pub fn load(
        spec: Spec,
        analysis: SpecAnalysis,
        base_dir: Option<&std::path::Path>,
    ) -> Result<Self, EngineError> {
        let session = Engine::new().load_spec(spec, analysis, base_dir)?.session;
        Ok(Self::from_session(session))
    }

    /// Wrap an already-loaded session (generation starts at 0).
    #[must_use]
    pub fn from_session(session: Session) -> Self {
        Self {
            session,
            generation: 0,
        }
    }

    /// The current generation — the version stamp of the live materialisation.
    #[must_use]
    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// Read access to the live session (profiling, distinct values, state).
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Mutable access to the live session, for the initial full execution and
    /// per-surface reads that are not themselves interactions.
    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    /// A cancellation handle for the session's in-flight query — see
    /// [`Session::interrupt_handle`]. Handed to the UI so a newer interaction
    /// can abort a running re-query on the worker thread.
    #[must_use]
    pub fn interrupt_handle(&self) -> Arc<duckdb::InterruptHandle> {
        self.session.interrupt_handle()
    }

    /// Apply one interaction: bump the generation, push its predicate / value
    /// into DuckDB, and re-query every affected mark. Never filters a
    /// materialised batch client-side — the predicate is in the emitted SQL.
    pub fn apply(&mut self, interaction: Interaction) -> Requery {
        self.generation += 1;
        let affected = match interaction {
            Interaction::Select {
                name,
                contributor,
                predicate,
            } => self
                .session
                .propagate_selection(&name, contributor, predicate),
            Interaction::ClearSelect { name, contributor } => {
                self.session.clear_selection(&name, contributor)
            }
            Interaction::SetParam { name, value } => self.session.propagate_param(&name, value),
        };
        Requery {
            generation: self.generation,
            affected,
        }
    }

    /// The chart surface's read of a mark at the current interaction state —
    /// the projected channel query. Shares the live selection predicate with
    /// [`Coordinator::grid_rows`] bit-for-bit.
    ///
    /// # Errors
    ///
    /// Propagates emit / query failure for the mark.
    pub fn chart_rows(&mut self, mark_index: usize) -> Result<Vec<RecordBatch>, EngineError> {
        self.session.execute_mark(mark_index)
    }

    /// The grid surface's read of the SAME step at the SAME interaction state —
    /// row-level `SELECT *` over the same materialisation, under the same
    /// `WHERE`. Because it shares one selection-compile path with
    /// [`Coordinator::chart_rows`], the two surfaces cannot resolve different
    /// rows from the same selection state.
    ///
    /// # Errors
    ///
    /// Propagates emit / query failure for the mark's step.
    pub fn grid_rows(&mut self, mark_index: usize) -> Result<Vec<RecordBatch>, EngineError> {
        self.session.execute_step_rows(mark_index)
    }

    /// Consume the coordinator, returning the owned session (to move it onto a
    /// worker thread, or to hand it back).
    #[must_use]
    pub fn into_session(self) -> Session {
        self.session
    }
}

/// Collapse a backlog of pending interactions to the newest one.
///
/// Given the first interaction already received plus a receiver, this drains
/// every interaction currently queued and returns the last — every earlier one
/// is a superseded intermediate drag step that never needs to be executed. This
/// is the "cancel the queued work" half of supersession (the in-flight query is
/// cancelled separately via the interrupt handle).
fn coalesce_latest(first: Interaction, rx: &Receiver<Interaction>) -> Interaction {
    let mut latest = first;
    while let Ok(next) = rx.try_recv() {
        latest = next;
    }
    latest
}

/// A handle to the off-UI-thread engine worker: send interactions, poll the
/// freshest result. Dropping the handle closes the request channel and lets the
/// worker thread finish and exit (join it via [`QueryLoop::join`]).
pub struct QueryLoop {
    requests: Sender<Interaction>,
    results: Receiver<Requery>,
    interrupt: Arc<duckdb::InterruptHandle>,
    last_applied: Generation,
    worker: Option<JoinHandle<Coordinator>>,
}

impl QueryLoop {
    /// Move `coordinator` onto a dedicated thread and start the interaction loop.
    ///
    /// The worker blocks on the request channel; on each wake it **coalesces**
    /// the whole backlog to the latest interaction, applies it, and sends the
    /// generation-stamped [`Requery`] back. The returned handle keeps the
    /// [`InterruptHandle`](duckdb::InterruptHandle) so [`QueryLoop::request`] can
    /// cancel an in-flight query when a newer interaction arrives.
    #[must_use]
    pub fn spawn(coordinator: Coordinator) -> Self {
        let interrupt = coordinator.interrupt_handle();
        let (req_tx, req_rx) = std::sync::mpsc::channel::<Interaction>();
        let (res_tx, res_rx) = std::sync::mpsc::channel::<Requery>();

        let worker = std::thread::Builder::new()
            .name("brightfield-query".to_string())
            .spawn(move || worker_loop(coordinator, &req_rx, &res_tx))
            .expect("spawn query worker thread");

        Self {
            requests: req_tx,
            results: res_rx,
            interrupt,
            last_applied: 0,
            worker: Some(worker),
        }
    }

    /// Submit an interaction. Interrupts whatever query is in flight so the
    /// worker abandons a now-superseded re-query and picks up the latest, then
    /// enqueues this interaction. A `false` return means the worker has stopped.
    pub fn request(&self, interaction: Interaction) -> bool {
        // Cancel the in-flight query first: a sustained drag must not wait for a
        // large re-query to finish before the newer one can start.
        self.interrupt.interrupt();
        self.requests.send(interaction).is_ok()
    }

    /// Take the freshest available result, dropping any superseded ones.
    ///
    /// Drains every result the worker has produced since the last poll and
    /// returns only the one with the highest generation newer than the last
    /// applied — older results are discarded. Returns `None` when nothing newer
    /// is ready. This is the guarantee that a superseded query never reaches a
    /// surface: even if a stale result arrives, it is never the one returned.
    pub fn poll(&mut self) -> Option<Requery> {
        let mut freshest: Option<Requery> = None;
        while let Ok(requery) = self.results.try_recv() {
            if requery.generation <= self.last_applied {
                continue; // stale — superseded by one already applied.
            }
            match &freshest {
                Some(cur) if cur.generation >= requery.generation => {}
                _ => freshest = Some(requery),
            }
        }
        if let Some(r) = &freshest {
            self.last_applied = r.generation;
        }
        freshest
    }

    /// The generation of the most recent result [`QueryLoop::poll`] returned.
    #[must_use]
    pub fn last_applied(&self) -> Generation {
        self.last_applied
    }

    /// Close the request channel and wait for the worker to finish, recovering
    /// the [`Coordinator`] (and its session). Any results still in flight are
    /// drained by the caller beforehand if wanted.
    ///
    /// # Panics
    ///
    /// Panics if the worker thread panicked.
    #[must_use]
    pub fn join(mut self) -> Coordinator {
        // Drop the sender so the worker's recv() returns Err and it exits.
        // Replace with a fresh, immediately-closed channel to move `requests`.
        let (dead_tx, _) = std::sync::mpsc::channel::<Interaction>();
        drop(std::mem::replace(&mut self.requests, dead_tx));
        self.worker
            .take()
            .expect("worker present")
            .join()
            .expect("query worker thread panicked")
    }
}

/// The worker thread body: block for an interaction, coalesce the backlog to the
/// latest, apply it, and send the generation-stamped result back. Exits when the
/// request channel closes, returning the coordinator so its session can be
/// recovered by [`QueryLoop::join`].
fn worker_loop(
    mut coordinator: Coordinator,
    requests: &Receiver<Interaction>,
    results: &Sender<Requery>,
) -> Coordinator {
    // recv() returns Err only when every sender is dropped — the exit signal.
    while let Ok(first) = requests.recv() {
        let latest = coalesce_latest(first, requests);
        let requery = coordinator.apply(latest);
        if results.send(requery).is_err() {
            break; // consumer gone.
        }
    }
    coordinator
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::analysis::analyse_spec;
    use brightfield_spec::{parse_spec, Format};
    use duckdb::arrow::array::Array;

    fn coordinator_from(yaml: &str) -> Coordinator {
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        Coordinator::load(parsed.spec, analysis, None).expect("load")
    }

    const BRUSH_DASHBOARD: &str = r#"
params:
  brush:
    select: intersect
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
    - { x: 3, y: 30 }
    - { x: 4, y: 40 }
    - { x: 5, y: 50 }
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

    fn rows(batches: &[RecordBatch]) -> usize {
        batches.iter().map(RecordBatch::num_rows).sum()
    }

    // --- Session::execute_step_rows (grid path) ---

    #[test]
    fn grid_and_chart_cannot_disagree_at_rest() {
        // No selection: grid and chart both read the full materialisation.
        let mut coord = coordinator_from(BRUSH_DASHBOARD);
        let chart = coord.chart_rows(0).expect("chart");
        let grid = coord.grid_rows(0).expect("grid");
        assert_eq!(rows(&chart), 5, "chart sees all rows at rest");
        assert_eq!(rows(&grid), rows(&chart), "grid and chart agree at rest");
    }

    #[test]
    fn grid_and_chart_cannot_disagree_under_brush() {
        // A brush contributed from a THIRD plot node so neither subscriber
        // self-excludes. The predicate is pushed into DuckDB; both surfaces
        // must resolve the identical filtered row set.
        let mut coord = coordinator_from(BRUSH_DASHBOARD);
        let contributor = ComponentPath("root/vconcat[99]".to_string());
        let requery = coord.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor,
            predicate: SqlPredicate::Expr("x >= 3".to_string()),
        });
        assert_eq!(requery.generation, 1);
        assert_eq!(requery.affected.len(), 2, "both marks re-queried");

        // Mark 0 (dot) and mark 1 (line) both filtered to x in {3,4,5}.
        for mark in 0..=1 {
            let chart = coord.chart_rows(mark).expect("chart");
            let grid = coord.grid_rows(mark).expect("grid");
            assert_eq!(
                rows(&chart),
                3,
                "mark {mark} chart pushed the predicate into DuckDB"
            );
            assert_eq!(
                rows(&grid),
                rows(&chart),
                "mark {mark}: grid and chart over the same materialisation cannot disagree"
            );
            // Value agreement on the shared `x` column, sorted.
            let cx = column_i64(&chart, "x");
            let gx = column_i64(&grid, "x");
            assert_eq!(
                cx, gx,
                "mark {mark}: grid and chart carry identical x values"
            );
            assert_eq!(
                cx,
                vec![3, 4, 5],
                "the pushed predicate kept x in {{3,4,5}}"
            );
        }
    }

    fn column_i64(batches: &[RecordBatch], name: &str) -> Vec<i64> {
        use duckdb::arrow::array::Int64Array;
        use duckdb::arrow::compute::cast;
        use duckdb::arrow::datatypes::DataType;
        let mut out = Vec::new();
        for b in batches {
            let idx = b.schema().index_of(name).expect("column present");
            // Inline VALUES columns land as Int32; cast to a common width so the
            // grid/chart value comparison is width-agnostic.
            let col = cast(b.column(idx), &DataType::Int64).expect("cast to i64");
            let arr = col
                .as_any()
                .downcast_ref::<Int64Array>()
                .expect("i64 after cast");
            for i in 0..arr.len() {
                out.push(arr.value(i));
            }
        }
        out.sort_unstable();
        out
    }

    #[test]
    fn interaction_re_queries_via_pushed_predicate_not_client_filter() {
        // The re-query returns fewer rows because DuckDB filtered them — there
        // is no client-side path that could have. Clearing restores all rows.
        let mut coord = coordinator_from(BRUSH_DASHBOARD);
        assert_eq!(rows(&coord.chart_rows(0).unwrap()), 5);

        let contributor = ComponentPath("root/vconcat[99]".to_string());
        coord.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: contributor.clone(),
            predicate: SqlPredicate::Expr("x > 3".to_string()),
        });
        assert_eq!(
            rows(&coord.chart_rows(0).unwrap()),
            2,
            "DuckDB kept x in {{4,5}}"
        );

        let cleared = coord.apply(Interaction::ClearSelect {
            name: "brush".to_string(),
            contributor,
        });
        assert_eq!(cleared.generation, 2, "clearing is a new generation");
        assert_eq!(
            rows(&coord.chart_rows(0).unwrap()),
            5,
            "brush released → all rows"
        );
    }

    // --- structured predicates ride the unchanged Interaction seam ---

    #[test]
    fn structured_predicate_through_interaction_matches_string_form() {
        // The structured Interval clause flows through the SAME Interaction
        // seam (no new variants, no signature changes): applied via
        // Interaction::Select it filters exactly the rows its string form
        // filters, and the session's live slot still holds the full structure
        // — column, typed bounds, scale/pixel metadata — where the string
        // form left only an opaque expression.
        use brightfield_sql::ir::{ClauseMeta, ScalarValue, ScaleDescriptor};

        let contributor = ComponentPath("root/vconcat[99]".to_string());

        let mut coord_string = coordinator_from(BRUSH_DASHBOARD);
        coord_string.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: contributor.clone(),
            predicate: SqlPredicate::And(vec![
                SqlPredicate::Expr("x >= 2".to_string()),
                SqlPredicate::Expr("x <= 4".to_string()),
            ]),
        });

        let structured = SqlPredicate::Interval {
            column: "x".to_string(),
            lo: ScalarValue::Float(2.0),
            hi: ScalarValue::Float(4.0),
            meta: Some(ClauseMeta {
                scale: Some(ScaleDescriptor {
                    kind: "linear".to_string(),
                    domain: Some((1.0, 5.0)),
                    range: Some((40.0, 340.0)),
                }),
                pixel_size: Some(1.0),
            }),
        };
        let mut coord_structured = coordinator_from(BRUSH_DASHBOARD);
        let requery = coord_structured.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: contributor.clone(),
            predicate: structured.clone(),
        });
        assert_eq!(requery.affected.len(), 2, "both marks re-queried");

        // Identical pushed-down filtering on every mark, chart and grid alike.
        for mark in 0..=1 {
            let s = column_i64(&coord_string.chart_rows(mark).expect("chart"), "x");
            let t = column_i64(&coord_structured.chart_rows(mark).expect("chart"), "x");
            assert_eq!(t, s, "mark {mark}: structured == string filtered rows");
            assert_eq!(t, vec![2, 3, 4], "x kept in {{2,3,4}}");
            let g = column_i64(&coord_structured.grid_rows(mark).expect("grid"), "x");
            assert_eq!(g, t, "mark {mark}: grid agrees under the structured clause");
        }

        // No erasure through the seam: the live slot holds the clause whole.
        let slot = coord_structured
            .session()
            .contributor_predicate("brush", &contributor.0)
            .expect("slot written");
        assert_eq!(
            slot, &structured,
            "structure and metadata intact end to end"
        );
    }

    // --- coalescing (supersession of queued interactions) ---

    #[test]
    fn coalesce_keeps_only_the_latest() {
        let (tx, rx) = std::sync::mpsc::channel::<Interaction>();
        let mk = |v: i64| Interaction::SetParam {
            name: "threshold".to_string(),
            value: SpecValue::Integer(v),
        };
        // A burst of drag steps, all queued before we coalesce.
        for v in [10, 20, 30, 40, 50] {
            tx.send(mk(v)).unwrap();
        }
        let first = rx.recv().unwrap();
        let latest = coalesce_latest(first, &rx);
        assert_eq!(
            latest,
            mk(50),
            "coalescing drops every superseded queued interaction"
        );
        assert!(rx.try_recv().is_err(), "the backlog was fully drained");
    }

    // --- generation guard + coalescing end-to-end on the real worker ---

    #[test]
    fn worker_supersedes_to_the_last_interaction() {
        // A burst of brush steps, each tightening the predicate. However the
        // worker interleaves receiving and processing, the FINAL materialisation
        // must be the LAST interaction — never a superseded intermediate one —
        // and every result the consumer polls advances the generation.
        let coord = coordinator_from(BRUSH_DASHBOARD);
        let mut loops = QueryLoop::spawn(coord);
        let contributor = ComponentPath("root/vconcat[99]".to_string());

        for hi in [5, 4, 3, 2, 1] {
            loops.request(Interaction::Select {
                name: "brush".to_string(),
                contributor: contributor.clone(),
                predicate: SqlPredicate::Expr(format!("x <= {hi}")),
            });
        }

        // Best-effort drain: whenever a result is ready, generations never
        // regress and a stale one is never handed back.
        let mut last = 0;
        for _ in 0..200 {
            if let Some(r) = loops.poll() {
                assert!(
                    r.generation > last,
                    "poll never returns a superseded (older) generation: {last} then {}",
                    r.generation
                );
                last = r.generation;
            }
            std::thread::yield_now();
        }

        // join() closes the request channel; the worker drains the whole backlog
        // (coalescing) and applies the last interaction before returning.
        let mut coord = loops.join();
        assert_eq!(
            rows(&coord.chart_rows(0).unwrap()),
            1,
            "the presented state is the last interaction (x <= 1), never a superseded frame"
        );
    }

    // --- interaction latency vs row count (recorded, not asserted tight) ---

    #[test]
    fn interaction_latency_across_two_orders_of_magnitude() {
        // Push-down latency should track the FILTERED result, not the source
        // size. A brush keeping a constant ~5000-row window is measured over a
        // 10^4- and a 10^6-row source (two orders of magnitude). Numbers are
        // printed (run with --nocapture) and recorded in the task, not asserted
        // to a borrowed benchmark — this project measures HERE before it claims.
        for &n in &[10_000_u64, 1_000_000_u64] {
            let yaml = format!(
                r#"
params:
  brush:
    select: intersect
data:
  t: {{ query: "SELECT i AS x, (i % 100) AS y FROM range({n}) t(i)" }}
plot:
  - mark: dot
    data: {{ from: t, filterBy: $brush }}
    x: x
    y: y
"#
            );
            let mut coord = coordinator_from(&yaml);
            // Warm the connection + plan so the one-time load is excluded.
            let base = rows(&coord.chart_rows(0).unwrap());
            assert_eq!(base as u64, n, "source materialised all {n} rows");

            let contributor = ComponentPath("root/plot[99]".to_string());
            let start = std::time::Instant::now();
            let requery = coord.apply(Interaction::Select {
                name: "brush".to_string(),
                contributor,
                predicate: SqlPredicate::Expr("x < 5000".to_string()),
            });
            let elapsed = start.elapsed();
            assert!(
                requery.affected.iter().all(|(_, r)| r.is_ok()),
                "re-query succeeded for {n} rows"
            );
            let kept = rows(&coord.chart_rows(0).unwrap());
            assert_eq!(kept, 5000.min(n as usize), "constant-window result for {n}");
            eprintln!(
                "PUSHDOWN_LATENCY source_rows={n} result_rows={kept} requery_ms={:.3}",
                elapsed.as_secs_f64() * 1e3
            );
        }
    }

    // --- automatic pre-aggregation: served from the cube, proven ------------

    /// A density mark over a larger source, brushed on a DIFFERENT column —
    /// the canonical cube client. The synthetic contributor path belongs to
    /// no plot, so nothing self-excludes.
    const DENSITY_UNDER_BRUSH: &str = r#"
params:
  brush:
    select: intersect
data:
  events_base: { query: "SELECT (i % 200) AS x, (i % 100) AS y FROM range(20000) t(i)" }
plot:
  - mark: densityX
    data: { from: events_base, filterBy: $brush }
    x: x
"#;

    fn structured_y_interval(lo: f64, hi: f64) -> SqlPredicate {
        use brightfield_sql::ir::ScalarValue;
        SqlPredicate::Interval {
            column: "y".to_string(),
            lo: ScalarValue::Float(lo),
            hi: ScalarValue::Float(hi),
            meta: None,
        }
    }

    fn string_y_interval(lo: f64, hi: f64) -> SqlPredicate {
        SqlPredicate::And(vec![
            SqlPredicate::Expr(format!("y >= {lo}")),
            SqlPredicate::Expr(format!("y <= {hi}")),
        ])
    }

    fn column_f64(batches: &[RecordBatch], name: &str) -> Vec<f64> {
        use duckdb::arrow::array::Float64Array;
        use duckdb::arrow::compute::cast;
        use duckdb::arrow::datatypes::DataType;
        let mut out = Vec::new();
        for b in batches {
            let idx = b.schema().index_of(name).expect("column present");
            let col = cast(b.column(idx), &DataType::Float64).expect("numeric column");
            let arr = col.as_any().downcast_ref::<Float64Array>().expect("f64");
            for i in 0..arr.len() {
                out.push(arr.value(i));
            }
        }
        out
    }

    /// THE MERGE GATE for the cube layer: an interaction over the source is
    /// served from the automatically derived pre-aggregate, not a scan of the
    /// base table — proven by the executed-statement targets (query-count and
    /// query-plan assertions), not by timing.
    #[test]
    fn structured_interaction_is_served_from_the_cube_not_the_base_table() {
        let mut coord = coordinator_from(DENSITY_UNDER_BRUSH);
        let contributor = ComponentPath("root/vconcat[99]".to_string());

        // Warm the initial render (direct, unfiltered — no interaction yet).
        let at_rest = coord.chart_rows(0).expect("initial render");
        assert!(rows(&at_rest) > 0);
        coord.session_mut().clear_executed_sql();

        // First brush step: builds the cube once, serves from it.
        let requery = coord.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: contributor.clone(),
            predicate: structured_y_interval(10.0, 49.0),
        });
        assert!(requery.affected.iter().all(|(_, r)| r.is_ok()));
        let stats = coord.session().preagg_stats().clone();
        assert_eq!(stats.cubes_built, 1, "one cube materialised");
        assert_eq!(stats.cube_hits, 1, "the re-query was served from it");
        assert_eq!(stats.serve_failures, 0);
        assert_eq!(stats.build_failures, 0);

        let log = coord.session().executed_sql();
        assert!(
            log.iter()
                .any(|s| s.starts_with("CREATE TEMP TABLE \"__bf_preagg_")),
            "the build statement ran: {log:#?}"
        );
        let serving: Vec<&String> = log
            .iter()
            .filter(|s| !s.starts_with("CREATE TEMP TABLE"))
            .collect();
        assert!(
            serving.iter().all(|s| !s.contains("\"events_base\"")),
            "after the one-time build, no executed statement touches the base table: {log:#?}"
        );
        assert!(
            serving.iter().any(|s| s.contains("__bf_preagg_")),
            "the serving query reads the cube: {log:#?}"
        );

        // Query-plan assertion: DuckDB's own plan for the serving query scans
        // the pre-aggregate, never the base table.
        let cube_query = serving
            .iter()
            .find(|s| s.contains("__bf_preagg_"))
            .expect("serving query logged")
            .to_string();
        let plan_batches = coord
            .session()
            .execute_raw_sql(&format!("EXPLAIN {cube_query}"))
            .expect("EXPLAIN runs");
        let mut plan_text = String::new();
        for b in &plan_batches {
            for c in 0..b.num_columns() {
                use duckdb::arrow::array::StringArray;
                if let Some(col) = b.column(c).as_any().downcast_ref::<StringArray>() {
                    for r in 0..col.len() {
                        plan_text.push_str(col.value(r));
                        plan_text.push('\n');
                    }
                }
            }
        }
        assert!(
            plan_text.contains("__bf_preagg_"),
            "plan scans the cube: {plan_text}"
        );
        assert!(
            !plan_text.contains("events_base"),
            "plan never scans the base table: {plan_text}"
        );

        // Second brush step: the SAME cube serves — no rebuild, exactly one
        // more DuckDB execute, still zero base-table statements.
        coord.session_mut().clear_executed_sql();
        let execs_before = coord.session().duckdb_execute_count();
        let requery2 = coord.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor,
            predicate: structured_y_interval(20.0, 59.0),
        });
        assert!(requery2.affected.iter().all(|(_, r)| r.is_ok()));
        let stats2 = coord.session().preagg_stats().clone();
        assert_eq!(stats2.cubes_built, 1, "cube reused across steps");
        assert_eq!(stats2.cube_hits, 2);
        assert_eq!(
            coord.session().duckdb_execute_count(),
            execs_before + 1,
            "one drag step costs exactly one cube query"
        );
        let log2 = coord.session().executed_sql();
        assert!(
            log2.iter().all(|s| !s.contains("\"events_base\"")),
            "no drag step rescans the base table: {log2:#?}"
        );
    }

    /// Transparency: the cube-served result IS the direct result. The control
    /// coordinator applies the equivalent STRING predicate (unstructured →
    /// derivation bails → direct query); schemas and values must match.
    #[test]
    fn cube_served_result_equals_the_direct_result() {
        let contributor = ComponentPath("root/vconcat[99]".to_string());

        let mut served = coordinator_from(DENSITY_UNDER_BRUSH);
        served.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: contributor.clone(),
            predicate: structured_y_interval(10.0, 49.0),
        });
        assert_eq!(served.session().preagg_stats().cube_hits, 1, "served");

        let mut direct = coordinator_from(DENSITY_UNDER_BRUSH);
        direct.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor,
            predicate: string_y_interval(10.0, 49.0),
        });
        assert_eq!(
            direct.session().preagg_stats().cubes_built,
            0,
            "the string predicate is not derivable — direct query"
        );

        let a = served.chart_rows(0).expect("served rows");
        let b = direct.chart_rows(0).expect("direct rows");
        assert_eq!(rows(&a), rows(&b), "same row count");
        assert!(rows(&a) > 0, "non-vacuous comparison");
        assert_eq!(
            a[0].schema(),
            b[0].schema(),
            "identical schema — names and types"
        );
        // Density orders by bin centre, so rows align positionally.
        assert_eq!(column_f64(&a, "x"), column_f64(&b, "x"), "bin centres");
        assert_eq!(
            column_f64(&a, "__bf_count"),
            column_f64(&b, "__bf_count"),
            "per-bin counts"
        );
    }

    /// Fallback matrix: shapes the cube must NOT serve still work, directly.
    #[test]
    fn non_derivable_shapes_fall_back_to_the_direct_query() {
        // (a) A row-level mark (dot) has nothing to pre-aggregate.
        let mut coord = coordinator_from(BRUSH_DASHBOARD);
        let contributor = ComponentPath("root/vconcat[99]".to_string());
        let requery = coord.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: contributor.clone(),
            predicate: structured_y_interval(20.0, 40.0),
        });
        assert!(requery.affected.iter().all(|(_, r)| r.is_ok()));
        assert_eq!(coord.session().preagg_stats().cubes_built, 0);
        assert_eq!(coord.session().preagg_stats().cube_hits, 0);
        assert_eq!(rows(&coord.chart_rows(0).unwrap()), 3, "y in {{20,30,40}}");

        // (b) An unstructured predicate over the aggregate mark: direct.
        let mut coord = coordinator_from(DENSITY_UNDER_BRUSH);
        let requery = coord.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor,
            predicate: SqlPredicate::Expr("y < 50".to_string()),
        });
        assert!(requery.affected.iter().all(|(_, r)| r.is_ok()));
        assert_eq!(coord.session().preagg_stats().cubes_built, 0);
        assert!(
            coord
                .session()
                .executed_sql()
                .iter()
                .any(|s| s.contains("\"events_base\"")),
            "the direct query ran against the base table"
        );
    }

    /// The grid surface (row-level SELECT *) is never served from a cube —
    /// exact-SQL matching cannot confuse the two read shapes — and both
    /// surfaces agree on the filtered row count.
    #[test]
    fn grid_reads_stay_row_level_under_a_cube_serve() {
        let mut coord = coordinator_from(DENSITY_UNDER_BRUSH);
        let contributor = ComponentPath("root/vconcat[99]".to_string());
        coord.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor,
            predicate: structured_y_interval(0.0, 49.0),
        });
        assert_eq!(coord.session().preagg_stats().cube_hits, 1);
        let grid = coord.grid_rows(0).expect("grid rows");
        assert_eq!(
            rows(&grid),
            10_000,
            "row-level read: the 10k source rows with y in [0,49]"
        );
        assert_eq!(
            coord.session().preagg_stats().cube_hits,
            1,
            "the grid read did not (wrongly) hit the cube"
        );
    }

    /// A spec reload retires every cube and serve: the TEMP tables are
    /// dropped and the next interaction derives afresh.
    #[test]
    fn spec_reload_retires_the_cubes() {
        let parsed = parse_spec(DENSITY_UNDER_BRUSH, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        let mut coord = coordinator_from(DENSITY_UNDER_BRUSH);
        let contributor = ComponentPath("root/vconcat[99]".to_string());
        coord.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor: contributor.clone(),
            predicate: structured_y_interval(10.0, 49.0),
        });
        assert_eq!(coord.session().preagg_stats().cubes_built, 1);

        coord.session_mut().clear_executed_sql();
        coord.session_mut().reload_spec(parsed.spec, analysis);
        assert!(
            coord
                .session()
                .executed_sql()
                .iter()
                .any(|s| s.starts_with("DROP TABLE IF EXISTS \"__bf_preagg_")),
            "reload dropped the cube tables"
        );

        // The next interaction builds a fresh cube and still serves.
        let requery = coord.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor,
            predicate: structured_y_interval(10.0, 49.0),
        });
        assert!(requery.affected.iter().all(|(_, r)| r.is_ok()));
        assert_eq!(coord.session().preagg_stats().cubes_built, 2);
    }

    /// A self-aggregating cell under a POINT clause: served from the cube,
    /// equal to the direct result.
    #[test]
    fn cell_point_selection_served_from_cube_matches_direct() {
        use brightfield_sql::ir::ScalarValue;
        const CELL_UNDER_BRUSH: &str = r#"
params:
  pick:
    select: intersect
data:
  events_base:
    - { xg: a, yg: p, region: north }
    - { xg: a, yg: p, region: south }
    - { xg: a, yg: q, region: north }
    - { xg: b, yg: p, region: north }
    - { xg: b, yg: q, region: south }
plot:
  - mark: cell
    data: { from: events_base, filterBy: $pick }
    x: xg
    y: yg
    fill: { count: }
"#;
        let contributor = ComponentPath("root/vconcat[99]".to_string());
        let point = SqlPredicate::Point {
            column: "region".to_string(),
            values: vec![ScalarValue::Text("north".to_string())],
            meta: None,
        };

        let mut served = coordinator_from(CELL_UNDER_BRUSH);
        served.apply(Interaction::Select {
            name: "pick".to_string(),
            contributor: contributor.clone(),
            predicate: point,
        });
        assert_eq!(served.session().preagg_stats().cube_hits, 1, "served");

        let mut direct = coordinator_from(CELL_UNDER_BRUSH);
        direct.apply(Interaction::Select {
            name: "pick".to_string(),
            contributor,
            predicate: SqlPredicate::Expr("region = 'north'".to_string()),
        });
        assert_eq!(direct.session().preagg_stats().cubes_built, 0);

        let a = served.chart_rows(0).expect("served");
        let b = direct.chart_rows(0).expect("direct");
        assert_eq!(rows(&a), rows(&b));
        assert_eq!(rows(&a), 3, "three cells hold a north row");
        assert_eq!(a[0].schema(), b[0].schema());
        assert_eq!(
            column_f64(&a, "__bf_count"),
            column_f64(&b, "__bf_count"),
            "identical per-cell counts (cell orders by category)"
        );
    }

    /// The regression mark (scalar regr_* family) served from a cube matches
    /// the direct fit to floating-point tolerance — the mean-centered
    /// sufficient statistics reassemble the same coefficients.
    #[test]
    fn regression_served_from_cube_matches_direct_fit() {
        const REGRESSION_UNDER_BRUSH: &str = r#"
params:
  brush:
    select: intersect
data:
  events_base: { query: "SELECT (i % 40) AS g, CAST(i % 97 AS DOUBLE) AS w, CAST(2.5 * (i % 97) + 3 + (i % 7) AS DOUBLE) AS h FROM range(5000) t(i)" }
plot:
  - mark: regressionY
    data: { from: events_base, filterBy: $brush }
    x: w
    y: h
"#;
        let contributor = ComponentPath("root/vconcat[99]".to_string());
        let brush = |mut c: Coordinator, pred: SqlPredicate| {
            c.apply(Interaction::Select {
                name: "brush".to_string(),
                contributor: contributor.clone(),
                predicate: pred,
            });
            c
        };
        let served = brush(
            coordinator_from(REGRESSION_UNDER_BRUSH),
            SqlPredicate::Interval {
                column: "g".to_string(),
                lo: brightfield_sql::ir::ScalarValue::Float(5.0),
                hi: brightfield_sql::ir::ScalarValue::Float(30.0),
                meta: None,
            },
        );
        let mut served = served;
        assert_eq!(served.session().preagg_stats().cube_hits, 1, "served");
        assert_eq!(served.session().preagg_stats().build_failures, 0);

        let direct = brush(
            coordinator_from(REGRESSION_UNDER_BRUSH),
            SqlPredicate::And(vec![
                SqlPredicate::Expr("g >= 5".to_string()),
                SqlPredicate::Expr("g <= 30".to_string()),
            ]),
        );
        let mut direct = direct;
        assert_eq!(direct.session().preagg_stats().cubes_built, 0);

        let a = served.chart_rows(0).expect("served");
        let b = direct.chart_rows(0).expect("direct");
        assert_eq!(rows(&a), 1);
        assert_eq!(rows(&b), 1);
        for col in [
            "slope",
            "intercept",
            "x_bar",
            "x_min",
            "x_max",
            "y_min",
            "y_max",
        ] {
            let va = column_f64(&a, col)[0];
            let vb = column_f64(&b, col)[0];
            assert!(
                (va - vb).abs() <= 1e-9 * vb.abs().max(1.0),
                "{col}: cube {va} vs direct {vb}"
            );
        }
        assert_eq!(
            column_f64(&a, "n"),
            column_f64(&b, "n"),
            "pair count is exact"
        );
    }

    // --- offline, file-backed spec (no network) ---

    #[test]
    fn local_file_spec_renders_and_re_queries_offline() {
        // A local CSV over a file: source must load, execute, and push-down
        // re-query with no network access and no extension download.
        let dir = std::env::temp_dir();
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let fname = format!("bf_pushdown_offline_{}_{unique}.csv", std::process::id());
        let path = dir.join(&fname);
        std::fs::write(&path, "x,y\n1,10\n2,20\n3,30\n4,40\n").expect("write csv");

        let yaml = format!(
            r#"
params:
  brush:
    select: intersect
data:
  t: {{ file: {fname} }}
plot:
  - mark: dot
    data: {{ from: t, filterBy: $brush }}
    x: x
    y: y
"#
        );
        let parsed = parse_spec(&yaml, Format::Yaml).expect("parse");
        let analysis = analyse_spec(&parsed.spec).expect("analyse");
        let mut coord =
            Coordinator::load(parsed.spec, analysis, Some(dir.as_path())).expect("load offline");

        assert_eq!(
            rows(&coord.chart_rows(0).unwrap()),
            4,
            "local CSV rows loaded and rendered offline"
        );

        let contributor = ComponentPath("root/plot[99]".to_string());
        coord.apply(Interaction::Select {
            name: "brush".to_string(),
            contributor,
            predicate: SqlPredicate::Expr("x > 2".to_string()),
        });
        assert_eq!(
            rows(&coord.chart_rows(0).unwrap()),
            2,
            "offline re-query pushed the predicate into DuckDB (x in {{3,4}})"
        );

        std::fs::remove_file(&path).ok();
    }

    // --- in-flight cancellation via the interrupt handle ---

    #[test]
    fn interrupt_cancels_an_in_flight_query() {
        // A genuinely expensive query on one thread; interrupt it from another.
        // It must return an error rather than a full result, and the connection
        // must remain usable afterward. sum-of-products cannot be shortcut to a
        // cardinality, so DuckDB must actually iterate ~10^10 pairs.
        let coord = coordinator_from(BRUSH_DASHBOARD);
        let session = coord.into_session();
        let handle = session.interrupt_handle();

        let worker = std::thread::spawn(move || {
            // ~10^10 predicate evaluations, count-only so nothing overflows: it
            // cannot finish inside the 150ms window, so a non-error result would
            // mean the interrupt did nothing. count(*) < i64::MAX, no overflow.
            let sql = "SELECT count(*) FROM range(100000) a, range(100000) b \
                       WHERE (a.\"range\" + b.\"range\") % 2 = 0";
            let res = session.execute_raw_sql(sql);
            (session, res)
        });

        // Give the query a moment to start, then cancel it.
        std::thread::sleep(std::time::Duration::from_millis(150));
        handle.interrupt();

        let (session, res) = worker.join().expect("worker joined");
        assert!(
            res.is_err(),
            "an interrupted in-flight query must not return a full result"
        );
        // The session survives the interrupt: a trivial query still runs.
        let ok = session.execute_raw_sql("SELECT 1 AS one");
        assert!(ok.is_ok(), "connection remains usable after interrupt");
        assert_eq!(
            ok.unwrap().iter().map(RecordBatch::num_rows).sum::<usize>(),
            1
        );
    }
}
