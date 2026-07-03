Findings from the adversarial review of the live point-click gesture (card 0006 — the pixel→datum increment that finishes cross-filter point selection). The gesture works for the common case — a click on a NUMERIC scatter axis snaps to the nearest datum and dispatches its exact value. These are the deferred edges; distill into specs under card 0006 (or a follow-up card) rather than opening new cards ad hoc.

## 1. Point selection is numeric-axis-only (categorical + temporal deferred) — GUARDED in this PR

`point_to_predicate` forms a bare `col = value` numeric literal, and `find_nearest` resolves the datum via `column_as_f64` (numeric only). So point selection is scoped to continuous numeric axes. This PR **guards** the two out-of-scope axis types so a click is a clean no-op rather than misbehaving (`is_numeric_point_column` gates the binding in `commit_click_multi`):

- **Categorical (string / Band axis)** — the canonical Mosaic "click a bar to filter". Needs: a categorical nearest-resolution path (map a category to its band-centre pixel; `find_nearest` today can't read a Utf8 column), and a **quoted, escaped string predicate** (`col = 'value'`). Without the guard, clicking a bar silently CLEARED the whole dashboard (find_nearest → None → deselect).
- **Temporal (Timestamp axis)** — clicking a time-axis datum read the value as microseconds-f64 and emitted `ts = 1700000000000000`; DuckDB has no implicit `BIGINT → TIMESTAMP` cast, so `propagate_selection` errored and `absorb` swallowed it (subscriber kept its old batch). Needs a **temporal literal predicate** (e.g. `ts = make_timestamp(us)`). NOTE this is **systemic**: the interval-brush predicates (`x_range_predicate`/`y_range_predicate` in `brush.rs`) have the identical bare-integer flaw on temporal axes, so a temporal-literal predicate builder fixes brushing too.

→ Spec under card 0006: **type-aware selection predicates** (string equality + escaping, temporal literals) + categorical nearest-resolution. Shared with interval brushing for the temporal half.

## 2. Int64/UInt64 keys above 2^53 lose precision through `as f64`

`column_value_at` reads the datum via `column_as_f64`, which coerces every integer through `arr.value(i) as f64`. For an Int64/UInt64 value beyond 2^53 (~9.007e15 — snowflake ids, epoch-nanoseconds), the f64 rounds, so `col = <rounded>` matches zero rows despite the "exact stored value" promise. The common inline-YAML Int32 case is exact. Fix: an integer-preserving value path (read the raw i64/u64 and format it directly) once the predicate builder is type-aware (folds into #1).

→ Spec under card 0006 (same type-aware-predicate increment).

## 3. `find_nearest` requires BOTH axes even for a single-axis mode

`find_nearest` unconditionally requires `Channel::X` **and** `Channel::Y` columns+scales regardless of `NearestMode`. So a mode-X point selection (or hover) on a 1-D mark — `ruleX`, `tickX`, or a dot with a literal `y: 0` — finds nothing and every click clears. A mode-X search needs only the x column/scale. Fix: make the channel/scale requirement mode-aware (X needs X, Y needs Y, XY needs both), computing the perpendicular pixel only when available. Low-risk but touches the hover path, so verify hover on 2-D marks is unchanged.

→ Spec under card 0006 (or the hover card) — small, self-contained.

## 4. Same-selection point + interval stomp — GUARDED in this PR

A plot carrying BOTH a `toggleX` and an `intervalX` that write the SAME selection (`as: $sel`) produces two bindings with an identical `(selection, contributor)`. On a click, the point binding selects and the interval binding would then clear the same target. This PR guards it: `commit_click_multi` tracks the `(selection, contributor)` pairs a point binding selected this click and skips a sibling clear on the same target. (Distinct selection names were always fine.) An unusual config, but the guard makes it order-independent.

→ No further action; noted for the record. A cleaner long-term fix is to dedup bindings by `(selection, contributor, axis)` upstream in `build_brushable_bindings`.
