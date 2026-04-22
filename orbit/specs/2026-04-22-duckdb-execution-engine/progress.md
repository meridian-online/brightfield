# Implementation Progress

Spec path: orbit/specs/2026-04-22-duckdb-execution-engine/spec.yaml
Spec hash: sha256:6f27240f0a058106d34d1ffc9c04f84b635036511d20b2cd4e33c137f8e60c06
Started: 2026-04-22
Current AC: none

## Hard Constraints
- [x] brightfield-sql stays pure — no DuckDB dependency, no I/O. The engine is a new downstream consumer.
- [x] Arrow record batch ownership: Vec<RecordBatch> per query, fully owned by the caller.
- [x] Prepared statement cache keyed by plan_hash — scalar param changes rebind without re-preparing.
- [x] EngineError is a new enum — does not extend EmitError. Wraps EmitError as EngineError::EmitFailed.
- [x] In-memory DuckDB per session — no persistent scratch database in this iteration.
- [x] The engine crate depends on brightfield-spec and brightfield-sql but neither upstream crate depends on the engine.

## Detours

## Acceptance Criteria
- [x] ac-01: New crate brightfield-engine at crates/brightfield-engine/ — Cargo.toml with bundled duckdb, workspace member added
- [x] ac-02: EngineError enum with structured variants — ConnectionFailed, DdlFailed, QueryFailed, EmitFailed; 4 unit tests
- [x] ac-03: Engine::new() and Session via load_spec with DDL execution and warnings — inline data queryable
- [x] ac-04: session.execute_mark(index) — returns EmitFailed(UnsupportedMark) for unimplemented marks
- [x] ac-05: session.execute_all() with partial failure — 2 marks both fail independently
- [x] ac-06: session.update_param with mark-only filtering and partial failure — mark_index_map filters ComponentPaths to marks
- [x] ac-07: Prepared statement cache keyed by plan_hash — cache populated via execute_emitted, cache_len() exposed for test
- [x] ac-08: Structured error context — DdlFailed carries source_name+sql+cause, QueryFailed carries mark_index+kind+sql+cause
- [x] ac-09: Session Drop closes connection cleanly — create/drop/recreate test passes
- [x] ac-10: All existing tests pass — cargo test --workspace: 189 passed, 0 failed
- [x] ac-11 (gate): No public dependency on upstream internals — only pub APIs consumed (Spec, SpecAnalysis, emit_sources, emit_query, EmittedQuery, EmitError, ParseWarning, MarkKind)
