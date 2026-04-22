# Design Interview — Card 0012: DuckDB Execution Engine

Rally: mark lowering and DuckDB execution
Card: orbit/cards/0012-duckdb-execution-engine.yaml
Date: 2026-04-22
Mode: rally decision-pack (agent-proposed, author-approved)

---

## Q1: Where does the execution engine live in the workspace?

**Decision:** New crate `crates/brightfield-engine/`.

Dependencies: `brightfield-spec` (path), `brightfield-sql` (path), `duckdb = { version = "1", features = ["bundled"] }`, `arrow` (version matched to duckdb-rs re-export).

Dependency chain: `brightfield-spec` → `brightfield-sql` → `brightfield-engine`. Each crate adds one concern. `brightfield-sql` stays pure (no I/O, no DuckDB — invariant preserved structurally, not by feature flag).

---

## Q2: How is the DuckDB connection managed?

**Decision:** In-memory database per spec session.

`Connection::open_in_memory()` per session. The `Session` struct owns the connection; `Drop` closes it. Stateless across spec loads, no cleanup needed.

DuckDB streaming execution means views over `read_parquet()` don't materialise the whole file. If a future card needs disk spill, only the connection factory changes — the API surface stays the same.

---

## Q3: How do query results flow to the renderer?

**Decision:** `Vec<RecordBatch>` per query.

Matches duckdb-rs natural output (`query_arrow().collect()`). Owned data — no lifetime entanglement with the DuckDB Connection. Typically 1-3 batches per analytical query. Caller can pass to rendering layer or store without borrowing the engine.

Streaming variant can be added alongside later if progressive rendering demands it.

---

## Q4: How does the engine handle parameter changes?

**Decision:** Prepared statement cache keyed by `plan_hash`.

- Scalar param change (slider drag): look up cached prepared statement by `plan_hash` (structural hash excluding parameter values — already computed in `EmittedQuery`). Rebind `?` values. Execute. Sub-millisecond.
- Selection change (crossfilter brush): re-emit SQL (WHERE clause changes structurally). Prepare new statement. Update cache entry.
- Cache bounded by mark count (typically 2-5 entries). Evicted on spec reload (session drop).

Evidence: `EmittedQuery.plan_hash` and `Binding::Scalar`/`Binding::Selection` distinction were designed for exactly this pattern.

---

## Q5: How are execution errors surfaced?

**Decision:** New `EngineError` enum in `brightfield-engine/src/error.rs`.

Variants:
- `DdlFailed { source_name: String, sql: String, cause: duckdb::Error }` — data source view/attach failed.
- `QueryFailed { mark_index: usize, mark_kind: String, sql: String, cause: duckdb::Error }` — mark query failed.
- `ConnectionFailed { cause: duckdb::Error }` — DuckDB connection setup failed.
- `EmitFailed { cause: EmitError }` — upstream emission failed.

Directly satisfies card scenario: "the error identifies the failing query, the missing column, and the data source."

---

## Q6: What is the public API shape?

**Decision:** `Engine` struct creating `Session` objects.

```
Engine::new() -> Engine
engine.load_spec(spec, analysis, base_dir) -> Result<Session, EngineError>
session.execute_mark(index) -> Result<Vec<RecordBatch>, EngineError>
session.execute_all() -> Vec<Result<Vec<RecordBatch>, EngineError>>
session.update_param(name, value) -> Result<Vec<(usize, Vec<RecordBatch>)>, EngineError>
```

Session owns: DuckDB Connection, parsed Spec + SpecAnalysis, emitted DDL + per-mark queries, prepared statement cache.

Maps to card scenarios:
- "the engine processes the spec" = `load_spec` + `execute_all`
- "the parameter value changes" = `update_param`
- Spec reload = drop session, create new one (drops connection, clears cache)

---

## Interaction with Card 0004

The engine consumes `emit_sources()` output directly. Card 0004's DDL emission (CREATE OR REPLACE VIEW, ATTACH READ_ONLY) is the exact setup phase that `load_spec` executes.

## Interaction with Card 0008

The engine executes SQL from mark lowerers. Until mark lowerers are implemented, `default_lowerers()` returns `EmitError::UnsupportedMark`. The engine surfaces this as `EngineError::EmitFailed`. Engine infrastructure (connection, DDL, caching, param tracking) can be built and tested with DDL alone + hand-written SQL or test-only lowerers.

## Key References

- brightfield-brief.md (execution model, Arrow transport)
- crates/brightfield-sql/src/lib.rs (emit_sources, emit_query — pure emission API)
- crates/brightfield-sql/src/ir.rs (QueryPlan, plan_hash)
- crates/brightfield-sql/src/binding.rs (Binding::Scalar, Binding::Selection)
- crates/brightfield-spec/src/analysis.rs (SpecAnalysis, subscriber_graph)
- orbit/specs/2026-04-21-direct-data-source-loading/ (card 0004 spec)
