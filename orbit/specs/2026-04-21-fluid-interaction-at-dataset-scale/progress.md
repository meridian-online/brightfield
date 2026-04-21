# Implementation Progress

Spec path: orbit/specs/2026-04-21-fluid-interaction-at-dataset-scale/spec.yaml
Spec hash: sha256:697c5cc8b19b23ba4ed8e2f9bdd313e4fb4cda4ae1949f935724a629e6677ce8
Started: 2026-04-21
Current AC: ac-16

## Hard Constraints
- [x] brightfield-sql already exists (card 0004) with emit.rs, source.rs, render.rs, error.rs — extend, do not restructure
- [x] Pure functions only — no I/O, no DuckDB connection, no filesystem access in brightfield-sql
- [x] QueryPlan IR mirrors DuckDB grammar deliberately — DuckDB is the only target
- [x] EmitError is the single error type for the crate — extend it, do not add a second
- [x] sqlparser-rs is the only new external dependency (add to brightfield-sql and brightfield-conformance)
- [x] No marks are flipped to Implemented — ImplStatus gates remain Unimplemented
- [x] Conformance layer 2 already passes for DDL (card 0004) — query conformance is additive, not breaking
- [x] ExpressionNode.spans/params interleaving invariant (spans.len() == params.len() + 1) is load-bearing for SQL rendering
- [x] All public types derive Debug, Clone, PartialEq, Eq where possible; QueryPlan must be Hash for shape-cache key

## Detours

2026-04-21: Bin.width changed from f64 to String — f64 does not implement Eq or Hash, storing as string preserves derive requirements while keeping SQL output identical.
Return to: ac-01

2026-04-21: Spec.root is Option<Component> tree, not a flat Vec<Mark> — added collect_marks helper to walk the component tree depth-first.
Return to: ac-08

## Acceptance Criteria
- [x] ac-01: QueryPlan IR in ir.rs with Source, Filter, Projection, Aggregation, Bin, Order, Limit variants — 5 tests
- [x] ac-02: Predicate type with Expr, Param, And, Or, True, False variants and Display impl — 6 tests
- [x] ac-03: MarkLower trait in lower.rs with LowerCtx and default UnsupportedMark error — 2 tests
- [x] ac-04: IR-to-SQL renderer render_query in render.rs with binding threading — 8 tests
- [x] ac-05: Selection compilation with crossfilter resolution in lower.rs — 4 tests
- [x] ac-06: Binding model — EmittedQuery, Binding enum (Scalar/Selection) — 3 tests
- [x] ac-07: Pass pipeline in passes.rs with apply_passes — 2 tests
- [x] ac-08: Public entry point emit_query in emit.rs — orchestrates lower→passes→render→hash
- [x] ac-09: EmitError extended with UnsupportedMark and SqlParseError variants
- [x] ac-10: ExpressionNode SQL rendering with BindingMode (Prepared/Interpolated) — 3 tests with invariant check
- [x] ac-11: sqlparser-rs structural conformance utility in conform.rs — 4 tests
- [x] ac-12: sqlparser 0.61 added to brightfield-sql and brightfield-conformance Cargo.toml
- [x] ac-13: lib.rs updated with pub mod binding, conform, ir, lower, passes
- [x] ac-14: QueryPlan::hash_structural method — 2 tests
- [x] ac-15 (gate): All existing tests pass — cargo test --workspace (112+ tests green)
- [x] ac-16: Test suite with 39 new dfir_ tests (threshold: >=23)
