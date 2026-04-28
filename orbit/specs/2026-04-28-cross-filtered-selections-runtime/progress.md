# Implementation progress — cfs2 (card 0006 v2)

Spec: orbit/specs/2026-04-28-cross-filtered-selections-runtime/spec.yaml
Branch: rally/cross-filtered-selections-runtime

## AC checklist

- [x] ac-01 selection_state field + current_selections() accessor on Session
- [x] ac-02 propagate_selection dispatches to subscribers
- [x] ac-03 same-contributor replacement
- [x] ac-04 parent_plot helper
- [x] ac-05 parent-plot self-exclusion (string equality on plot prefix)
- [x] ac-06 resolution strategies at runtime (intersect/union/single)
- [x] ac-07 unsubscribed selection silent
- [x] ac-08 partial failure (mirror rpw2_ac04)
- [x] ac-09 emit_query consumes param_values + selection_predicates
- [x] ac-10 brush_rect_to_predicate adapter
- [x] ac-11 on_mouse_up dispatches selection
- [x] ac-12 end-to-end against vendored crossfilter.yaml (inline shape)
- [x] ac-13 corpus regression gate green
- [x] ac-14 cargo test --workspace green
- [x] ac-15 16 cfs2_ tests (≥8 gate)

## Implementation summary

Engine (brightfield-engine):
- selection_state: HashMap<String, Vec<(ComponentPath, Predicate)>> field
- current_selections() accessor
- selection_predicates_for_emit() helper (stringifies ComponentPath)
- propagate_selection(name, contributor, predicate) → per-subscriber result vec
- All five emit_query call sites updated (execute_mark, update_param,
  propagate_param, update_extent, propagate_selection).

SQL (brightfield-sql):
- emit_query / emit_query_with_passes signatures take additive
  selection_predicates: Option<&[(String, Vec<(String, Predicate)>)]>.
- Inside emit, mark with filterBy + matching SelectionNode triggers
  compile_selection over (self_source = parent_plot(mark_path),
  contributors), wrapping the QueryPlan in QueryPlan::Filter.
- collect_marks_with_paths helper mirrors engine path format.

Spec (brightfield-spec):
- parent_plot(path: &str) -> &str helper next to ComponentPath.
  Byte-scan for `/plot[<digits>]`, returns longest matching prefix or
  the input unchanged.

UI (brightfield-ui):
- New brush.rs module: BrushKind, ChannelColumns,
  brush_rect_to_predicate, SelectionDispatcher trait.
- Session implements SelectionDispatcher (via propagate_selection).
- chart_view.rs: BrushBinding struct, on_mouse_up_with_dispatch
  method, commit_brush_release pure helper for testability.

Tests (16 cfs2_ tests):
- spec: cfs2_ac04_parent_plot_helper
- engine: cfs2_ac01..ac09 + cfs2_ac12 (9 tests)
- ui:    cfs2_ac10 × 4 (intervalX, intervalY, intervalXY, missing channel)
         cfs2_ac11 × 2 (dispatch, no-brush no-dispatch)
