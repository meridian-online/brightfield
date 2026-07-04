Findings from the adversarial review of the live point-click gesture (card 0006 — the pixel→datum increment that finishes cross-filter point selection). The gesture works for the common case — a click on a NUMERIC scatter axis snaps to the nearest datum and dispatches its exact value. These are the deferred edges; distill into specs under card 0006 (or a follow-up card) rather than opening new cards ad hoc.

## 1. Type-aware POINT predicates — DONE (categorical + temporal point selection)

**Resolved** by the typed-predicates increment. Point selection is no longer
numeric-only: `commit_click_multi`/`resolve_point_value` produce a
`SelectionValue { Number | Int | Text | Timestamp }` (`brightfield_render::nearest`)
whose `literal()` formats a type-correct SQL literal, and `point_to_predicate`
takes it.

- **Categorical (string / Band axis)** — the canonical Mosaic "click a bar to
  filter" now works: `band_category_at` resolves the clicked category from the
  band scale (a pixel→category inverse — no datum scan), and the predicate is a
  quoted, escaped `col = 'value'`. (`examples/point-select-categorical.yaml`.)
  Because a band axis tiles the plot (no empty space to click), the **clear**
  affordance is a click in the axis margin (outside the band pixel range).
  Mosaic-style **toggle-off** (click the same bar again to deselect) needs the
  current selection state at click time and is a follow-up.
- **Temporal (Timestamp axis)** — a time-axis click emits `col = make_timestamp(us)`
  for a naive `TIMESTAMP`, and `col = make_timestamp(us) AT TIME ZONE 'UTC'` for a
  `TIMESTAMPTZ` column (the tz field distinguishes them). The plain `make_timestamp`
  is a naive value that DuckDB shifts through the session TimeZone, so the
  tz-anchored form is required to match a `TIMESTAMPTZ` column under the default
  (machine-local) session — otherwise the selection silently matched zero rows.

**Still open — temporal INTERVAL brushes.** The systemic half remains: the
interval-brush range predicates (`x_range_predicate`/`y_range_predicate` in
`brush.rs`) still emit bare-integer microsecond bounds on a temporal axis, so a
DRAG (not a click) on a time axis errors the same way point selection used to.
Applying the same `make_timestamp` formatting to `brush_rect_to_predicate`
(threading the axis's scale type through the invert→predicate path in
`crossfilter.rs`) is the remaining follow-up.

## 2. Int64/UInt64 keys above 2^53 — DONE for point selection

**Resolved.** `column_typed_value_at` reads integer columns as `SelectionValue::Int(i64)`
and formats the exact stored integer, so a large key (snowflake id, epoch-nanos)
selects the right row instead of a rounded one. (Note `find_nearest` still snaps
via `f64` — fine, that only picks the nearest datum; the *value* read is exact.
UInt64 above `i64::MAX` is the one remaining lossy edge.)

## 3. `find_nearest` requires BOTH axes even for a single-axis mode

`find_nearest` unconditionally requires `Channel::X` **and** `Channel::Y` columns+scales regardless of `NearestMode`. So a mode-X point selection (or hover) on a 1-D mark — `ruleX`, `tickX`, or a dot with a literal `y: 0` — finds nothing and every click clears. A mode-X search needs only the x column/scale. Fix: make the channel/scale requirement mode-aware (X needs X, Y needs Y, XY needs both), computing the perpendicular pixel only when available. Low-risk but touches the hover path, so verify hover on 2-D marks is unchanged.

→ Spec under card 0006 (or the hover card) — small, self-contained.

## 4. Same-selection point + interval stomp — GUARDED in this PR

A plot carrying BOTH a `toggleX` and an `intervalX` that write the SAME selection (`as: $sel`) produces two bindings with an identical `(selection, contributor)`. On a click, the point binding selects and the interval binding would then clear the same target. This PR guards it: `commit_click_multi` tracks the `(selection, contributor)` pairs a point binding selected this click and skips a sibling clear on the same target. (Distinct selection names were always fine.) An unusual config, but the guard makes it order-independent.

→ No further action; noted for the record. A cleaner long-term fix is to dedup bindings by `(selection, contributor, axis)` upstream in `build_brushable_bindings`.
