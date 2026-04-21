# Implementation Progress

Spec path: orbit/specs/2026-04-21-reactive-parameters-with-input-widgets/spec.yaml
Spec hash: sha256:1beb7678c29c6295642eefcc6ad32b0ed9efd3ed728d087bc92a172ce92a2d46
Started: 2026-04-21
Current AC: none

## Hard Constraints
- [x] Input.as_param, Input.from, and Input.filter_by are typed fields — not convention over the options bag
- [x] ParseWarning variants are non-fatal — do not abort parsing or produce Err
- [x] Cycle detection rejects specs with circular param dependencies as SchemaViolation
- [x] Existing brightfield-spec and brightfield-sql tests must continue to pass unchanged
- [x] No runtime coordinator or query execution — static analysis at parse/load time only
- [x] Subscriber graph and DAG are pure functions of parsed Spec — no I/O, no DuckDB
- [x] All new public types live in brightfield-spec; DAG builder may live in brightfield-spec or brightfield-sql

## Detours

## Acceptance Criteria
- [x] ac-01: Input struct gains typed fields (as_param, from_source, filter_by) — typed fields added to Input, parser extracts as/from/filterBy before options bag, serialiser re-emits them
- [x] ac-02: Vendored specs parse successfully with new Input struct — round-trip test passes, explicit vendored spec test confirms no as/from/filterBy in options
- [x] ac-03: Subscriber graph maps param name to subscriber component paths — build_subscriber_graph walks component tree collecting param refs from marks, inputs, interactors, legends
- [x] ac-04: ParseWarning::DeadParam emitted for zero-subscriber params — analyse_spec checks subscriber graph for empty subscriber lists
- [x] ac-05: Param dependency DAG with topological ordering — Kahn's algorithm over adjacency list built from input filter_by→as_param edges
- [x] ac-06: Circular param dependencies detected as SchemaViolation — Kahn's detects cycle when topological sort is incomplete
- [x] ac-07: ParseWarning::ParamTypeMismatch for provably incompatible pairs — slider→selection and table→scalar detected
- [x] ac-08: WidgetOutputType and ParamDeclaredType enums with constructors — from_input_kind and from_param_node implemented
- [x] ac-09: Analysis integrated into parse pipeline (SpecAnalysis) — analyse_spec returns SpecAnalysis with graph, edges, order, warnings
- [x] ac-10: Specs with no params produce empty analysis, no warnings — verified with minimal spec test
- [x] ac-11 (gate): All existing tests pass — cargo test --workspace: all green
- [x] ac-12: At least 15 rpw_ prefixed tests — 16 rpw_ tests
