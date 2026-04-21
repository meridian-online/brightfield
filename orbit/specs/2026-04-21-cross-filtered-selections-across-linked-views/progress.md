# Implementation Progress

Spec path: orbit/specs/2026-04-21-cross-filtered-selections-across-linked-views/spec.yaml
Spec hash: sha256:ac510a2269c2e4e0eb03e70d60b90a003d440ab3a7ba402739fad14e6cfa83da
Started: 2026-04-21
Current AC: none

## Hard Constraints
- [x] All 54+ vendored Mosaic specs must continue to parse and round-trip without error
- [x] filterBy validation is a hard error (ParseError), not a warning — fast failure on broken wiring
- [x] Validation runs in analyse_spec, not during the parse walk — the walker builds AST, analysis validates it
- [x] Self-exclusion structure is per-view, not per-interactor (D3 decision)
- [x] Resolution strategy is structural and fixed at parse time — no runtime mutation (D5 decision)
- [x] Empty selection semantics (Predicate::True) are already correct in lower.rs — this card documents, not reimplements

## Detours
2026-04-21: Self-loop DAG edges — wnba-shots.yaml has input with filterBy: $filter and as: $filter on same widget. Fixed by skipping self-referential edges in collect_dag_edges.
Return to: ac-10

2026-04-21: Implicit selection creation — many vendored specs use filterBy: $name where $name is created by interactor or input as: binding, not declared in params. Fixed by collecting all as:-bound names as known selections.
Return to: ac-10

## Acceptance Criteria
- [x] ac-01: filterBy on mark data referencing a missing param → ParseError::SchemaViolation — test: cfs_ac01_filterby_mark_missing_param
- [x] ac-02: filterBy on mark data referencing a value param (not selection) → ParseError::SchemaViolation — test: cfs_ac02_filterby_mark_value_param
- [x] ac-03: filterBy on input referencing a missing param → ParseError::SchemaViolation — test: cfs_ac03_filterby_input_missing_param
- [x] ac-04: filterBy on input referencing a value param → ParseError::SchemaViolation — test: cfs_ac04_filterby_input_value_param
- [x] ac-05: interactor as: $name where name missing → ParseWarning::InteractorBindingMissing — test: cfs_ac05_interactor_binding_missing
- [x] ac-06: interactor as: $name where name is value param → ParseWarning::InteractorBindingNonSelection — test: cfs_ac06_interactor_binding_non_selection
- [x] ac-07: build_selection_subscriber_graph returns selection→subscriber map — tests: cfs_ac07_selection_subscriber_graph, cfs_ac07_selection_subscriber_graph_excludes_value_params
- [x] ac-08: build_interactor_bindings returns interactor→selection pairs — test: cfs_ac08_interactor_bindings
- [x] ac-09: analyse_spec integrates selection validation; SpecAnalysis gains new fields — test: cfs_ac09_analyse_spec_integration
- [x] ac-10 (gate): vendored corpus passes parse + analyse with no new errors — test: cfs_ac10_vendored_corpus_passes_analyse
- [x] ac-11: round-trip property preserved — test: cfs_ac11_round_trip_with_selection_filterby
