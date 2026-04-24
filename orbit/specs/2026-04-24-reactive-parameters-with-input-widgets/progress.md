# Implementation Progress

Spec path: orbit/specs/2026-04-24-reactive-parameters-with-input-widgets/spec.yaml
Spec hash: sha256:0a93a849a944ced49ad7910ed1da9b2cc2053698f9f886743bb770bdee3fe672
Started: 2026-04-24
Current AC: none

## Hard Constraints
- [x] Coordinator lives in brightfield-engine, extending Session
- [x] Direct propagation only — chained DAG walking deferred
- [x] Partial failure: one mark's failure must not prevent others
- [x] Existing Session API unchanged in signature and behaviour
- [x] All existing tests must pass
- [x] Session tracks current param values for subsequent queries
- [x] Unsubscribed param changes produce empty results with no error

## Detours

## Acceptance Criteria
- [x] ac-01: param_state field + current_params() — initialised from spec.params defaults, Selection params excluded
- [x] ac-02: propagate_param() dispatches to subscribers — updates param_state, re-executes subscribing marks with full state
- [x] ac-03: unsubscribed param returns empty — param_state updated, no queries fire
- [x] ac-04: partial failure handling — param_state always updated regardless of mark results
- [x] ac-05: unknown param permissive — dynamic param injection into param_state
- [x] ac-06: execute_mark/execute_all use param_state — param_state plumbing verified
- [x] ac-07: end-to-end integration test — parse, load, propagate, verify state consistency
- [x] ac-08 (gate): all existing tests pass — 320/320 passed, 0 failed
- [x] ac-09: at least 8 rpw2_ prefixed tests — 10 rpw2_ tests
