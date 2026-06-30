# 2. Execution-conformance test per renderer

Date: 2026-06-30

## Status

Accepted

## Context

The first-render follow-ups (`orbit/cards/memos/2026-04-29-first-render-followups.md`,
item #6) catalogued a class of bugs that all passed the existing tests:

- Int32 columns silently dropped (`column_as_f64` didn't match the type).
- A literal `stroke: red` treated as a column reference (`Binder Error`).
- A missing DuckDB function — `width_bucket` — used by the density lowerer.

Each passed CI because the tests asserted the emitted SQL **string** but never ran
it through DuckDB and the renderer. The `width_bucket` gap actually shipped:
`density`/`densityX` were marked `Implemented` and emitted valid-looking SQL, but
the bundled libduckdb lacks `width_bucket`, so every density spec failed at
execution and rendered nothing — silently.

## Decision

Every renderer in `default_renderers()` that is genuinely implemented (a
registered renderer **and** a working lowerer) has at least one
**execution-conformance** test that drives the REAL pipeline end to end —
parse → analyse → engine → execute → render — and asserts the **mark's** geometry
(a path or a glyph). Crucially it renders the mark's renderer onto a fresh scene,
NOT via `build_multi_mark_scene` — which always paints a white background + grid,
so a `paths > 0` check there passes even when the mark draws nothing (the
"axes-only baseline" the follow-up memo warned about).

The tests live in `crates/brightfield-ui/tests/execution_conformance.rs` — the
lowest crate that depends on both the engine and the renderer. They use integer
inline data deliberately (DuckDB types YAML integers as Int32, the column type
that once silently dropped).

## Consequences

- **Caught three silently-broken marks on day one.** `regressionY`, `densityX`
  and `density` are marked `Implemented` but render NOTHING end-to-end: their
  lowerers emit aggregate/binned columns (`x_bin`/`y_bin`/`count`, regression
  coefficients) while the renderers read the raw `x`/`y` channel columns, so the
  executed batch has no column the renderer or scale inference can find. They're
  kept as `#[ignore]`d executable repros; fixing the renderer/SQL output-column
  contract — and demoting their status until then — is a tracked follow-up.
- **Fixed the first density blocker.** Density's SQL used DuckDB's `width_bucket`,
  absent from the bundled libduckdb (it errored before even returning rows).
  Replaced with the portable `floor((v - lo) / (hi - lo) * n) + 1`. Necessary but
  not sufficient — the column-contract mismatch above remains.
- A new mark that ships a renderer + lowerer must add a conformance case. This is
  the end-to-end backstop; it does not replace unit tests for renderer geometry
  or SQL shape.
- `PARTIAL` marks (renderer but no lowerer — `dotX`/`lineX`/…) are excluded:
  they don't render end-to-end, which is why they stay `Unimplemented`.
