# Spec Review

**Date:** 2026-04-22
**Reviewer:** Context-separated agent (fresh session)
**Spec:** orbit/specs/2026-04-22-duckdb-execution-engine/spec.yaml
**Verdict:** REQUEST_CHANGES

---

## Review Depth

```
| Pass | Triggered by | Findings |
|------|-------------|----------|
| 1 — Structural scan | always | 3 |
| 2 — Assumption & failure | cross-system boundary content signal (consumes brightfield-spec + brightfield-sql APIs) | 3 |
| 3 — Adversarial | not triggered | — |
```

## Findings

### [MEDIUM] Subscriber graph maps to component paths, not mark indices
**Category:** assumption
**Pass:** 1
**Description:** AC-06 states that `update_param` uses `SpecAnalysis.subscriber_graph` to look up which marks subscribe to a named parameter. However, the actual `SubscriberGraph` type (in `crates/brightfield-spec/src/analysis.rs`) maps param names to `Vec<ComponentPath>` where `ComponentPath` is a string like `"root/plot[0]/mark[dot]"`. These are not mark indices. The engine needs to map from component paths back to depth-first mark indices to know which `execute_mark(index)` calls to make. The spec does not describe how this mapping works, and the upstream API does not provide it.
**Evidence:** `pub type SubscriberGraph = HashMap<String, Vec<ComponentPath>>` at analysis.rs:97. AC-06 says "using SpecAnalysis.subscriber_graph for lookup" but the graph yields paths, not indices.
**Recommendation:** Either (a) add an implementation note describing how the engine builds a `ComponentPath -> mark_index` reverse map during `load_spec`, or (b) specify that the engine builds its own param-to-mark-index map by walking the spec's component tree. This is an internal detail but the current spec implies a direct lookup that does not exist.

### [LOW] AC-06 return type loses error granularity on partial failure
**Category:** missing-requirement
**Pass:** 1
**Description:** `update_param` returns `Result<Vec<(usize, Vec<RecordBatch>)>, EngineError>`. If re-executing three subscribing marks and only one fails, the entire call returns `Err`. This is inconsistent with `execute_all()` (AC-05) which returns `Vec<Result<...>>` to allow partial failure. The interview (Q4) does not discuss partial failure during param update.
**Evidence:** AC-05: "Partial failure is possible (some marks succeed, others fail)." AC-06 return type is a single `Result`, not `Vec<Result<...>>`.
**Recommendation:** Decide whether `update_param` should mirror `execute_all`'s partial-failure semantics (return `Vec<(usize, Result<Vec<RecordBatch>, EngineError>)>`) or document that partial failure is intentionally fatal for param updates. Either choice is valid -- just make it explicit.

### [LOW] AC-07 verification does not specify how to observe cache hits
**Category:** test-gap
**Pass:** 1
**Description:** AC-07's verification says "Test that calling update_param twice with different scalar values for the same param reuses the same plan_hash entry." The plan_hash being the same is a property of the emitter, not the cache. The test does not specify how to observe that the prepared statement was actually reused (no re-prepare) versus re-prepared with the same hash. Without internal observability (a counter, a log, or exposing cache stats), the test proves the hash is stable but not that the cache is hit.
**Evidence:** AC-07 verification text.
**Recommendation:** Add an implementation note about how the test will observe cache reuse -- e.g., an internal `cache_hits` counter on Session, a `#[cfg(test)]` method exposing cache size, or asserting that the DuckDB prepare count does not increase. Alternatively, accept that stable plan_hash is a sufficient proxy and note that explicitly.

### [MEDIUM] Subscriber graph includes non-mark components
**Category:** assumption
**Pass:** 2
**Description:** `update_param` is defined to re-execute marks, but `SubscriberGraph` tracks subscriptions from all component types -- marks, inputs, interactors, and legends. The engine must filter the subscriber list to only mark components. If it does not filter, it will attempt to "execute" input or legend component paths as mark queries, producing confusing errors or panics.
**Evidence:** `collect_subscribers` in analysis.rs walks all component types (Mark, Input, Interactor, Legend, Plot, HConcat, VConcat). AC-06 says "re-executes all marks that subscribe to the named parameter."
**Recommendation:** Add an implementation note: "Filter subscriber_graph entries to mark components only (paths containing `/mark[`) before dispatching to execute_mark." Or build a dedicated mark-only subscriber index during load_spec.

### [LOW] emit_sources warnings are surfaced but behaviour is unspecified
**Category:** missing-requirement
**Pass:** 2
**Description:** Implementation note 6 says "emit_sources may return warnings -- surface these but don't fail the session load." No AC covers how warnings are surfaced. Are they stored on Session? Logged? Returned from load_spec? The caller has no way to access them.
**Evidence:** Implementation note 6. No AC mentions warnings from DDL emission.
**Recommendation:** Either add a minor AC or implementation note specifying that `load_spec` returns or stores `EmitOutput.warnings` (e.g., `session.ddl_warnings() -> &[ParseWarning]`), or drop the implementation note if warnings are out of scope for this iteration.

### [LOW] Arrow version coupling is acknowledged but not pinned
**Category:** failure-mode
**Pass:** 2
**Description:** Implementation note 2 says "Arrow version must match what duckdb-rs re-exports -- check duckdb-rs Cargo.toml." If the versions mismatch, RecordBatch types will be incompatible at compile time (different crate versions = different types). This is a known Rust pitfall with re-exported types.
**Evidence:** Implementation note 2. Constraint: "Arrow record batch ownership: Vec<RecordBatch> per query, fully owned by the caller."
**Recommendation:** Pin the arrow dependency to use `duckdb-rs`'s re-export rather than adding an independent `arrow` dependency. Add an implementation note: "Use `duckdb::arrow` re-export for RecordBatch to guarantee type compatibility, or pin arrow to the exact version re-exported by duckdb-rs." This avoids a class of compile errors that would be confusing to diagnose.

---

## Honest Assessment

This spec is well-structured with clear separation of concerns, good error modelling, and testable ACs. The biggest risk is the gap between `SubscriberGraph`'s component-path-based API and the engine's need to map param changes to mark indices -- this is the core of the reactive re-execution path (AC-06, AC-07) and if the mapping strategy is not thought through before implementation, it will lead to either a brittle string-parsing approach or a late redesign. The Arrow version coupling is a known footgun that is easy to prevent. I recommend addressing the two MEDIUM findings (subscriber graph mapping and non-mark filtering) before implementation, and making a quick decision on update_param's partial-failure semantics. The remaining LOW findings can be resolved during implementation.
