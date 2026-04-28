Findings uncovered while wiring the first end-to-end render (sprint goal for card 0001 / card 0008). Each item is follow-up work against an existing card — distill into specs, don't open new cards.

## 1. Numeric Arrow types beyond Int64/Float64 (FIXED in this commit)

`column_as_f64` in `crates/brightfield-render/src/mark.rs` only matched Float64, Int64, and Timestamp(Microsecond). DuckDB returns Int32 for inline integer literals in YAML, so every shipped mark (Dot, Line, Bar, Density, Regression) silently produced zero geometry against integer-typed columns. Fix in this PR extends the match to all signed/unsigned integer widths and Float32.

→ Spec under card 0008 (mark library) if any further numeric coverage gaps surface (Decimal? Date32? Duration?).

## 2. Mark ImplStatus drifts from runtime support

`crates/brightfield-spec/src/vocab.rs` flags `Dot`, `Line`, `Bar`, `Circle`, `DotX/Y`, `LineX/Y` as `Unimplemented` despite shipped renderers in `default_renderers()`. Produces parse warnings for every working spec. ImplStatus should be derived from the renderer registry rather than hand-maintained.

→ Spec under card 0008.

## 3. Literal channel values silently dropped

`resolve_colour` only handles column-bound channels through the Fill scale; literal `fill: orange` or `stroke: '#1f77b4'` falls through to `DEFAULT_COLOUR` (steelblue) with no warning. Same shape likely applies to literal `opacity` and `size`.

Related observation: `regressionY` with `stroke: red` produced `Binder Error: Referenced column "red" not found` from DuckDB — the SQL emitter treated the literal as a column reference. Both bugs share root cause: no clear separation between literal and column-bound channel values.

→ Spec under card 0008 (covers "each mark type draws correctly" — literal colours are part of "as declared").

## 4. densityX `width_bucket` missing from libduckdb-sys 1.10502.0

When testing densityX in multimark, DuckDB raised an opaque execution error. `width_bucket` exists in DuckDB head but not the bundled libduckdb-sys version we link against.

→ Spec under card 0012 (DuckDB execution engine) — version pinning / function-availability handling.

## 5. Window opens larger than the chart

`brightfield-app` passes `WindowOptions::default()`, which on macOS produces a full-screen window with the 640×480 chart pinned to the upper-left corner. Should size the window to chart dimensions, or have the chart fill its container responsively.

→ Spec under card 0001 (spec-driven viz integration) — width/height from spec should drive both layout and window bounds.

## 6. Tests assert SQL substrings, not real execution

The bugs above (Int32 dropping, literal-as-column, missing function) all share a class: existing tests check that the right SQL string is emitted but never run it through DuckDB and the renderer. A conformance layer that drives spec → SQL → DuckDB → renderer → "scene has more path tags than the axes-only baseline" would have caught each one.

→ Decision record (MADR) on test policy, not a card. File under `orbit/decisions/` with a proposal that every renderer in `default_renderers()` has at least one execution-conformance test.

## Diagnostic affordance shipped this PR

`BRIGHTFIELD_DUMP_PNG=<path> brightfield <spec.yaml>` now dumps the rendered scene to PNG and reports non-zero-byte coverage. Useful for headless diagnosis without GPUI complexity.
