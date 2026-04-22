# Decision Pack — Card 0012: DuckDB Execution Engine

Rally: **Layer 3 execution**.
Card: `orbit/cards/0012-duckdb-execution-engine.yaml`.
Scope: deciding how brightfield instantiates an in-process DuckDB connection, executes emitted SQL (both source DDL from card 0004 and per-mark queries from the SQL IR), returns Arrow record batches to the rendering layer, and re-executes on parameter changes.

## What is already fixed (not up for debate here)

These are inherited from cards 0004 (data source loading) and the SQL emission layer:

- `emit_sources(&Spec, Option<&Path>) -> Result<EmitOutput, EmitError>` — pure-function DDL emission for data sources. Each source becomes a `CREATE OR REPLACE VIEW` or `ATTACH` statement.
- `emit_query(&Spec, mark_index, Option<&ParamValues>) -> Result<EmittedQuery, EmitError>` — per-mark query emission producing SQL with `?` placeholders (prepared mode) or interpolated literals.
- `EmittedQuery { sql, bindings: Vec<Binding>, plan_hash: u64 }` — the emitter output carrying both the SQL string and parameter binding metadata.
- `Binding::Scalar { param, position }` and `Binding::Selection { param }` — two binding modes distinguishing slider-style scalars from structural selection changes.
- `SpecAnalysis { subscriber_graph, topological_order, selection_subscribers, ... }` — static analysis output from `analyse_spec()` identifying which marks subscribe to which params.
- The `brightfield-sql` crate is pure string generation with no DuckDB dependency. The execution engine is a new consumer of its output.

What this pack decides: how the DuckDB connection is managed, how emitted SQL is executed, how results flow as Arrow, how parameter changes trigger re-execution, how errors are surfaced, and where this new crate sits in the workspace.

---

## Decision 1 — Crate placement and DuckDB binding

### Context
The execution engine must hold a DuckDB connection and execute SQL — it is inherently stateful and has a native dependency (libduckdb). The existing `brightfield-sql` crate is explicitly documented as "no I/O, no DuckDB connection" (lib.rs line 8). Mixing a native C library dependency into the pure-emission crate would violate this contract, slow compilation for all downstream consumers, and muddy the conformance boundary. The `duckdb` Rust crate (`duckdb-rs`) bundles libduckdb and returns `arrow::RecordBatch` natively.

### Options
- **A. New crate `brightfield-engine` at `crates/brightfield-engine/`.** Depends on `brightfield-spec`, `brightfield-sql`, `duckdb` (with `bundled` feature), and `arrow`. The emission crate stays pure; the engine crate owns the DuckDB connection and Arrow transport.
- **B. Add DuckDB to `brightfield-sql` behind a cargo feature flag.** `brightfield-sql` gains `features = ["runtime"]` gating `duckdb` + `arrow` deps. The execution code lives in `brightfield-sql/src/engine.rs`, compiled only with the feature.
- **C. Add DuckDB to the workspace root crate.** No new crate; the binary crate at the workspace root owns the connection.

### Trade-offs
- **A (new crate)** — cleanest separation. `brightfield-sql` stays pure (its conformance tests never touch libduckdb). Compilation: `cargo test -p brightfield-sql` remains fast; only consumers that need execution pay the libduckdb build cost. Cost: one more workspace member, one more Cargo.toml.
- **B (feature flag)** — fewer crates. Loses: feature flags leak — any downstream crate enabling `runtime` forces all `brightfield-sql` consumers to link libduckdb. The "pure" invariant becomes conditional rather than structural. Feature-gated code paths are a testing blind spot (CI must test both with and without).
- **C (root crate)** — simplest plumbing but the workspace root today is empty (just a virtual workspace manifest). Putting the engine there conflates the binary entry point with the engine library, making it untestable by other crates.

### Recommendation
**Option A.** Create `crates/brightfield-engine/` with dependencies: `brightfield-spec` (path), `brightfield-sql` (path), `duckdb = { version = "1", features = ["bundled"] }`, `arrow` (version matched to what `duckdb-rs` re-exports). This preserves the pure/stateful boundary structurally. The crate name `brightfield-engine` mirrors the card title and the brief's "execution engine" terminology. Add to workspace `members` in root `Cargo.toml`.

---

## Decision 2 — DuckDB connection lifecycle

### Context
The engine must create a DuckDB instance, execute setup DDL (views from card 0004), run per-mark queries, and tear down on spec unload. DuckDB supports in-memory databases (`:memory:`) and persistent file-backed databases. The card's scope is read-only analytical exploration — the brief says "in-process DuckDB instance executes all analytical SQL." The `ATTACH` path from card 0004 (Decision 5: `READ_ONLY`) already handles connecting to existing database files; the engine's own instance is for executing the emitted SQL pipeline.

### Options
- **A. In-memory database per spec session.** `Connection::open_in_memory()`. Views are created at spec-mount, queries run against them, connection drops when the spec is unloaded. No persistent state.
- **B. Persistent scratch database.** Write a temporary `.duckdb` file per session, enabling DuckDB's disk spill for large datasets. Clean up on drop.
- **C. Connection pool.** Multiple connections to the same in-memory database, enabling concurrent query execution across marks.

### Trade-offs
- **A (in-memory per session)** — simplest, stateless across spec loads, no cleanup needed. DuckDB's in-memory mode handles multi-GB datasets on modern hardware (the brief targets analyst laptops). Loses: no disk spill — a 10GB Parquet scan may OOM. But: the card's scenario uses "a parquet data source" (singular, typical analyst file), and DuckDB's streaming execution means views over `read_parquet()` don't materialise the whole file.
- **B (persistent scratch)** — enables disk spill. Loses: file management (temp dir, cleanup on crash), and the emitted `READ_ONLY` ATTACH from card 0004 would need careful interaction with a read-write scratch DB. Premature for v1 — optimise when evidence demands it.
- **C (connection pool)** — enables parallel mark execution. Loses: DuckDB's in-process mode uses a single-writer model; concurrent reads are fine but the setup phase (DDL) is inherently serial. Added complexity for marginal gain when most specs have 2-5 marks.

### Recommendation
**Option A.** `Connection::open_in_memory()` per spec session. The engine struct owns the connection; `Drop` closes it. This matches the card's "in-process DuckDB instance" language literally. If a future card needs disk spill or concurrency, the engine struct's API (`execute_ddl`, `execute_query`) does not change — only the connection factory does. Ship the loop before optimising it.

---

## Decision 3 — Arrow record batch transport

### Context
Card scenario: "results arrive as Arrow record batches in shared memory — no serialisation boundary." The `duckdb` Rust crate's `Statement::query_arrow()` returns `Arrow<'_>`, an iterator over `arrow::RecordBatch`. The rendering layer (card 0014+, not yet specified) will consume these batches. The question is how the engine exposes results: raw iterator, collected Vec, or a streaming channel.

### Options
- **A. Return `Vec<RecordBatch>` per query.** Fully materialise results before returning. Simple, owned, no lifetime issues.
- **B. Return a streaming iterator (`Box<dyn Iterator<Item = Result<RecordBatch>>>`).** Zero-copy streaming from DuckDB through to the renderer.
- **C. Return a single concatenated `RecordBatch`.** Use `arrow::compute::concat_batches` to merge all chunks into one batch.

### Trade-offs
- **A (Vec<RecordBatch>)** — simplest API. DuckDB typically returns results in a small number of batches (often 1-3 for analytical queries). The Vec is owned, so the caller can pass it across threads or store it. Cost: if a query returns 1M rows, all batches are in memory simultaneously — but this is already true inside DuckDB's result set. The rendering layer needs all rows anyway (it draws the full mark).
- **B (streaming iterator)** — lowest latency to first batch. Loses: the iterator borrows the DuckDB `Statement`, which borrows the `Connection` — lifetime entanglement prevents running the next query until the current iterator is drained. This blocks the "execute all marks, return all results" pattern. Would require `unsafe` or connection cloning to work around.
- **C (single batch)** — simplest downstream consumption (one schema, one batch, no iteration). Loses: concat allocates a new buffer copying all data. For small results (<100K rows, typical of binned/aggregated analytical queries), the copy cost is negligible. For large scatter plots, it doubles peak memory briefly.

### Recommendation
**Option A.** Return `Vec<RecordBatch>` per mark query. This matches duckdb-rs's natural output (`query_arrow().collect()`), avoids lifetime entanglement, and gives the rendering layer owned data it can pass to WebGPU/canvas without borrowing the engine. The Vec is typically 1-3 batches. If a future card needs streaming (e.g. progressive rendering of large datasets), the API can add a streaming variant alongside — the Vec variant remains the simple default.

---

## Decision 4 — Parameter re-execution strategy

### Context
Card scenario: "the parameter value changes → the query re-executes with the new value and returns updated results." The `SpecAnalysis.subscriber_graph` maps param names to subscribing component paths. The `EmittedQuery.bindings` distinguish `Binding::Scalar` (slider values — `?` placeholder) from `Binding::Selection` (crossfilter brush — structural WHERE change). The question is how the engine handles these two re-execution paths.

### Options
- **A. Re-emit and re-execute every affected query from scratch.** On param change, call `emit_query()` again with updated `ParamValues`, then execute the new SQL. Stateless — no prepared statement caching.
- **B. Prepared statement cache keyed by `plan_hash`.** For scalar param changes, reuse the prepared statement and rebind `?` values. For selection changes (structural), re-emit SQL and prepare a new statement. Cache eviction on spec reload.
- **C. Fully reactive dataflow graph.** Build a dependency DAG at the query level (not just param level), incrementally re-execute only the changed subplan.

### Trade-offs
- **A (re-emit + re-execute)** — simplest. The emission layer is already pure and fast (microseconds). DuckDB re-prepares each query, but for 2-5 mark queries this is negligible. Loses: a slider drag generating 60 param updates/second would prepare 60 statements/second. DuckDB handles this fine for simple queries but it is wasteful.
- **B (prepared cache)** — optimises the hot path. `plan_hash` (already computed in `EmittedQuery`) keys the cache. Scalar param changes rebind `?` without re-preparing — DuckDB's prepared statements support this natively via `execute()` with parameter arrays. Selection changes invalidate the cache entry (structural SQL change). Loses: cache management complexity, but the cache is small (one entry per mark, 2-5 entries typical).
- **C (incremental dataflow)** — maximal efficiency. Loses: massive implementation complexity for marginal gain. The card's scope is "queries re-execute" not "queries incrementally update." Mosaic-web itself re-executes full queries on param change; incremental is a research topic.

### Recommendation
**Option B.** Prepared statement cache keyed by `plan_hash`. The `EmittedQuery` already computes `plan_hash` for exactly this purpose (ir.rs line 153: "structural hash that excludes bound parameter values"). On scalar param change: look up cached prepared statement by `plan_hash`, rebind `?` values, execute. On selection change: re-emit SQL (the WHERE clause changes structurally), prepare new statement, update cache. This gives slider-drag performance (rebind is sub-millisecond) without the complexity of incremental dataflow. The cache is bounded by mark count and evicted on spec reload.

---

## Decision 5 — Error surface for execution failures

### Context
Card scenario: "the error identifies the failing query, the missing column, and the data source." DuckDB returns errors as `duckdb::Error` which wraps a string message from libduckdb. The engine must translate these into structured errors the UI can display with context (which mark, which data source, what went wrong). The existing `EmitError` in `brightfield-sql` covers emission failures; execution failures are a new category.

### Options
- **A. New `EngineError` enum in `brightfield-engine`.** Variants: `DdlFailed { source_name, sql, detail }`, `QueryFailed { mark_index, sql, detail }`, `ConnectionFailed { detail }`. Wraps `duckdb::Error` as a source.
- **B. Extend `EmitError` with execution variants.** Add `ExecutionFailed { ... }` to the existing error enum.
- **C. Return `duckdb::Error` directly.** Let callers pattern-match on DuckDB's error types.

### Trade-offs
- **A (new enum)** — clean separation between emission errors (string generation bugs) and execution errors (runtime failures from DuckDB). The engine crate owns its error type; callers match on engine-specific variants with structured context. Cost: one more error type.
- **B (extend EmitError)** — fewer types. Loses: `brightfield-sql` would need a `duckdb` dependency (even if just for the error type), breaking its "no DuckDB" invariant. Alternatively, the execution variant would carry a `String` detail rather than `duckdb::Error`, losing type information.
- **C (raw duckdb::Error)** — zero wrapping. Loses: no structured context — "Column 'delya' not found" without knowing which mark or data source triggered it. The card's scenario explicitly demands this context.

### Recommendation
**Option A.** Define `EngineError` in `brightfield-engine/src/error.rs` with variants that carry structured context:
- `DdlFailed { source_name: String, sql: String, cause: duckdb::Error }` — a data source view/attach failed.
- `QueryFailed { mark_index: usize, mark_kind: String, sql: String, cause: duckdb::Error }` — a mark query failed.
- `ConnectionFailed { cause: duckdb::Error }` — DuckDB connection setup failed.
- `EmitFailed { cause: EmitError }` — upstream emission failed (wraps the existing error).

This directly satisfies the card's scenario: "the error identifies the failing query, the missing column, and the data source" — `QueryFailed` carries `mark_index` (which query), `cause` contains DuckDB's column-level message, and the engine can look up the data source from the mark's `data.from` field.

---

## Decision 6 — Execution orchestration API

### Context
The engine sits between SQL emission (Layer 2) and rendering (Layer 4). It must coordinate: (1) spec loading (parse + analyse + emit DDL + execute DDL), (2) initial query execution (emit + execute per-mark queries), (3) parameter updates (re-execute affected queries). The question is what the public API looks like — a single `Engine` struct with methods, or a set of free functions, or a trait.

### Options
- **A. `Engine` struct with session methods.** `Engine::new() -> Engine`, `engine.load_spec(spec, base_dir) -> Result<Session>`, `session.execute_all() -> Vec<Result<MarkResult>>`, `session.update_param(name, value) -> Vec<Result<MarkResult>>`. `MarkResult` wraps `(mark_index, Vec<RecordBatch>)`.
- **B. Free functions taking a connection.** `load_spec(conn, spec, base_dir) -> Result<()>`, `execute_mark(conn, spec, index) -> Result<Vec<RecordBatch>>`. Caller manages the connection.
- **C. `Executor` trait with an in-memory impl.** Trait methods `execute_ddl`, `execute_query`. Enables future alternative backends (remote DuckDB, MotherDuck).

### Trade-offs
- **A (struct with session)** — highest-level API. The `Session` owns the connection, the loaded spec, the analysis, and the prepared statement cache (Decision 4). Callers interact at the domain level ("load this spec", "this param changed") not the SQL level. Cost: the struct carries state, but this state is inherent to the execution model.
- **B (free functions)** — maximally flexible. Loses: caller must orchestrate the sequence (DDL before queries, param tracking, cache management). The "session" concept leaks into every caller.
- **C (trait)** — most extensible. Loses: premature abstraction. There is exactly one backend today (in-process DuckDB). A trait boundary adds indirection and forces all error types through a generic. When a second backend appears, extracting the trait from a concrete struct is a straightforward refactor.

### Recommendation
**Option A.** A concrete `Engine` struct that creates `Session` objects. The session owns: the DuckDB `Connection` (Decision 2), the parsed `Spec` and `SpecAnalysis`, the emitted DDL and per-mark queries, and the prepared statement cache (Decision 4). Public methods:
- `Engine::new() -> Engine` — constructs the engine (no connection yet).
- `engine.load_spec(spec, analysis, base_dir) -> Result<Session, EngineError>` — opens a connection, executes DDL, prepares initial queries.
- `session.execute_mark(index) -> Result<Vec<RecordBatch>, EngineError>` — executes one mark's query.
- `session.execute_all() -> Vec<Result<Vec<RecordBatch>, EngineError>>` — executes all marks.
- `session.update_param(name, value) -> Result<Vec<(usize, Vec<RecordBatch>)>, EngineError>` — re-executes affected marks, returns updated results with their indices.

This API maps directly to the card's scenarios: "the engine processes the spec" = `load_spec` + `execute_all`; "the parameter value changes" = `update_param`. The session pattern makes the lifecycle explicit — spec reload means dropping the session and creating a new one, which drops the connection and clears the cache.

---

## Summary table

```
| #  | Decision                          | Recommendation                                                           |
|----|-----------------------------------|--------------------------------------------------------------------------|
| 1  | Crate placement and DuckDB binding| New `brightfield-engine` crate; `brightfield-sql` stays pure                 |
| 2  | DuckDB connection lifecycle       | In-memory database per spec session; `open_in_memory()`                  |
| 3  | Arrow record batch transport      | `Vec<RecordBatch>` per query; owned, no lifetime entanglement            |
| 4  | Parameter re-execution strategy   | Prepared statement cache keyed by `plan_hash`; rebind on scalar change   |
| 5  | Error surface                     | New `EngineError` enum with structured context per failure site          |
| 6  | Execution orchestration API       | `Engine` struct creating `Session` objects with domain-level methods     |
```

## Cross-cutting notes

- **Dependency chain**: `brightfield-spec` (parse) -> `brightfield-sql` (emit) -> `brightfield-engine` (execute). Each crate adds one concern. The engine depends on both upstream crates but neither upstream crate depends on the engine.
- **Interaction with card 0004**: The engine consumes `emit_sources()` output directly. Card 0004's Decision 6 (source registration lifecycle: views at spec-mount) is the exact setup phase this engine's `load_spec` executes. The DDL is already designed for this two-phase pattern.
- **Interaction with SQL emission IR**: The `QueryPlan.hash_structural()` (ir.rs) and `Binding` model (binding.rs) were designed with Decision 4's caching strategy in mind — the `plan_hash` excludes parameter values so scalar changes produce identical hashes, enabling prepared statement reuse.
- **Mark lowering gap**: The `default_lowerers()` registry in `lower.rs` currently has zero concrete implementations — all marks return `EmitError::UnsupportedMark`. The engine will surface these as `EngineError::EmitFailed` until mark-lowering cards (e.g. dot, line, rectY) land. This is acceptable: the engine's infrastructure (connection, DDL, caching, param tracking) can be built and tested with the DDL path alone, and mark queries can be tested with hand-written SQL or a test-only lowerer.
- **Out of scope for this card**: rendering/UI layer (Layer 4), WebSocket transport for browser clients, spatial extension auto-loading, concurrent multi-session support.
