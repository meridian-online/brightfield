# Implementation Progress

Spec path: orbit/specs/2026-04-22-multi-view-dashboard-composition/spec.yaml
Spec hash: sha256:fed12cde42a7259a0e8299b9be47d3d346fcba6c103bed8f9cc43907720cc511
Started: 2026-04-22
Current AC: none

## Hard Constraints
- [x] Layout is a pure function of the AST — no I/O, no DuckDB, no runtime data
- [x] Simple box model — sequential stacking with fixed sizes, no flex negotiation
- [x] No new crate — layout lives in brightfield-spec alongside analysis.rs
- [x] No new dependencies — layout is pure computation on existing AST types
- [x] Default sizes: plot 640x400, input 200x32, legend 120x24

## Detours

## Acceptance Criteria
- [x] ac-01: Rect struct with x, y, width, height (all f64) exists in layout module — tests mvdc_ac01_rect_fields, mvdc_ac01_rect_zero
- [x] ac-02: LayoutNode enum mirrors Component variants — tests mvdc_ac02_layout_node_exhaustive_match, mvdc_ac02_layout_node_rect_accessor
- [x] ac-03: compute_layout(spec, viewport) walks spec.root and returns a LayoutTree — tests mvdc_ac03_single_plot, mvdc_ac03_no_root
- [x] ac-04: hconcat stacks children left-to-right — test mvdc_ac04_hconcat_two_plots
- [x] ac-05: vconcat stacks children top-to-bottom — test mvdc_ac05_vconcat_two_plots
- [x] ac-06: hspace and vspace insert fixed pixel gaps — tests mvdc_ac06_hspace_gap, mvdc_ac06_vspace_gap
- [x] ac-07: resolve_space_value parses numeric and em values — tests mvdc_ac07_numeric_pixels, mvdc_ac07_em_units, mvdc_ac07_invalid_returns_zero
- [x] ac-08: Nested composition produces grid-like layouts — test mvdc_ac08_nested_grid
- [x] ac-09: Plots, inputs, and legends all receive layout positions — test mvdc_ac09_mixed_types
- [x] ac-10: Plot width/height read from attributes with fallback — tests mvdc_ac10_plot_declared_size, mvdc_ac10_plot_partial_override
- [x] ac-11: Legend as: bindings in subscriber graph verified — test mvdc_ac11_legend_subscriber_graph
- [x] ac-12: layout module publicly exported from lib.rs — pub mod layout added
- [x] ac-13: Vendored corpus specs produce valid layout trees — test mvdc_ac13_corpus_layout
