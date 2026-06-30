# Harden — execution-conformance layer (first-render follow-up #6)

A test layer that drives each mark through the REAL pipeline (parse → analyse →
engine → execute → render) and asserts the **mark** drew geometry — closing the
gap where tests checked the emitted SQL *string* but never ran it. MADR 0002.

## What it immediately caught

Three "Implemented" marks render **nothing** end-to-end:
`regressionY`/`regressionX`, `density`, `densityX`. Their lowerers emit
aggregate/binned output columns (`x_bin`/`y_bin`/`count`, regression
coefficients), but the renderers read the raw `x`/`y` channel columns — so the
executed batch has no column the renderer or `infer_scales` can find, and the
renderer returns early (blank). Verified end-to-end via the app PNG path too.

These passed every prior test because the tests asserted SQL substrings, and a
naive `paths > 0` conformance check is *also* fooled — `build_multi_mark_scene`
always paints a white background (1 path). So the layer renders the mark's
renderer onto a **fresh scene** (no background) and counts only mark geometry.

## Shipped

- `crates/brightfield-ui/tests/execution_conformance.rs` — one case per renderer.
  **7 active and passing** (dot, line, barY, areaY, areaX, ruleY, text);
  **3 `#[ignore]`d executable repros** (the broken statistical marks).
- **width_bucket fix** — density's SQL used DuckDB's `width_bucket`, missing from
  the bundled libduckdb (it *errored* before returning rows). Replaced with the
  portable `floor((v - lo) / (hi - lo) * n) + 1` (`equiwidth_bucket` in
  `lower.rs`). Necessary but not sufficient — the column-contract mismatch
  remains, so density still doesn't render. The SQL-shape tests now assert the
  `floor` binning.
- MADR `orbit/decisions/0002-execution-conformance-per-renderer.md`.
- Removed the `examples/density.yaml` I'd added prematurely — density renders
  blank until the contract fix.

## Follow-up (task #7)

Reconcile the renderer↔SQL output-column contract for the statistical marks
(alias the SQL output to the channel names, or read the binned/coefficient
columns), un-`#[ignore]` the repros, and realign their vocab status (they're
currently over-reported `Implemented`; demote until they render, watching the
curated `deviations.yaml`).
