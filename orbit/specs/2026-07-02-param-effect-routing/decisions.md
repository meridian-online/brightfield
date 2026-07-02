# Decision Pack: Param-Effect Routing — Two-Tier Scalar Param Reactivity

**Card:** orbit/cards/0014-param-effect-routing.yaml
**Date:** 2026-07-02
**Slice:** v1 — make a scalar param actually change downstream query output, so `propagate_param` produces different results (the "data-shape" tier), verified headlessly; unblock the window-gated slider live-wiring.
**Prior art:**
- Card 0005 runtime (rpw3): `orbit/specs/2026-04-29-reactive-parameters-runtime/` — `propagate_param` topological DAG walk, `ParamDispatcher`, `commit_slider_release`. Shipped. Explicitly deferred making params affect SQL.
- Cross-filter runtime (cfs2): `Session::propagate_selection`, `selection_state`, compiled predicates. Shipped. The template for how re-executed batches flow back to the scene.

---

## Context summary

The reactive-params coordinator is built and unit-tested — `propagate_param` walks the topological order and re-dispatches every subscribing mark — but an empirical probe (this session) shows a scalar param changes **nothing** in any query:

- `params: {k: 3}` + a `dot`/`ruleY` with `y: $k`: the param subscribes the mark, `propagate_param("k", 20)` re-dispatches it, but the returned batch is byte-identical — in fact `y: $k` produces **no `y` column at all** (batch schema is just `["x"]`).
- `y: "v + $k"` (a SQL expression): the param **doesn't even subscribe** (subscriber_graph empty), so it is never re-dispatched.

Three root-cause stubs, all deliberate deferrals:

1. **`emit_query` discards `param_values`** — `let _ = param_values;` (crates/brightfield-sql/src/emit.rs:501) with a comment deferring "a proper Interpolated path to a follow-up."
2. **`execute_emitted` binds nothing** — runs with empty `duckdb::params![]` (crates/brightfield-engine/src/lib.rs:752), ignoring the `Binding::Scalar` positions `render_predicate` records (render.rs:257-263).
3. **Simple marks emit `SELECT *`** (SimpleLowerer, lower.rs:52) so a `$param` positional channel never appears in a query; `ChannelMap::from_mark` skips `$param` channels with a warning (channel.rs:147).

The engine's own comments frame the fix as "two-tier param-effect routing" that "separates pure-style param drags (no SQL re-execution needed) from data-shape param changes" (engine/lib.rs:153-160; gomb_ac12 comment). This slice builds the **data-shape tier** and shows the pure tier falls out of the cache layer for free. Five decisions follow.

---

## Decision 1: How does a param value reach the executed SQL — interpolation or prepared-binding?

### Context

`param_values` is threaded all the way to `emit_query` but discarded there (emit.rs:501). To make a param affect a query, its value must appear in what DuckDB executes. Two mechanisms are viable: inline the value into the SQL text (**interpolation**), or keep a placeholder and bind the value at execute time (**prepared-binding**). This choice is load-bearing because both existing caches key on the SQL text.

### Evidence

- `binding.rs` already models both: a `BindingMode::Interpolated { values }` path exists (binding.rs:54) and substitutes `$name → spec_value_to_sql_literal(value)` (binding.rs:82-98) — but nothing constructs it in `Interpolated` mode today.
- `render_predicate` records `Binding::Scalar { param, position }` and emits `?` (render.rs:257-263) — a prepared-binding skeleton, also unused (execute binds nothing).
- `spec_value_to_sql_literal` already escapes typed values (strings single-quoted, numbers bare) (emit.rs:193-216) — the injection-safety primitive interpolation needs.
- **The correctness-critical cache is `sql_cache: SQL-string → batches`** (engine/lib.rs:742). With interpolation the SQL differs per value → the cache keys correctly. With prepared-binding the SQL is identical (`… > ?`) for every value → the cache returns a **stale batch** unless re-keyed on (sql, values).
- `gomb_ac12` (lib.rs:1302) pins: a param change whose SQL is unchanged must hit the cache and skip DuckDB.

### Options

**A. String interpolation — inline the param value into the emitted SQL at emit time.**
- Gains: aligns with `sql_cache` (distinct value → distinct SQL → distinct key → never stale). Reuses `binding.rs`'s existing `Interpolated` mode and `spec_value_to_sql_literal` escaping. Concrete SQL is debuggable and cache-transparent. Localizes the change to `emit_query` (close the stub) + the cache key (decision 3). No execution-layer change.
- Loses: `plan_hash` (structural) no longer uniquely identifies the SQL — must fold interpolated values into the key (decision 3). Requires disciplined use of `spec_value_to_sql_literal` for every inlined value (injection surface, mitigated by typed `SpecValue`s).

**B. Prepared-statement binding — keep `?`, bind values in `execute_emitted`.**
- Gains: no injection surface (DuckDB binds typed values); `plan_hash` stays purely structural; one compiled statement reused across values.
- Loses: **breaks `sql_cache`** — identical `?`-SQL across values collides in the cache, returning stale batches. Fixing it means re-keying `sql_cache` on `(sql, bound-values)` and converting `SpecValue → duckdb::types::Value` in the execute path. More surgery in the hottest code path, and the plan-hash statement cache (`self.cache`) would still need a values-aware notion to avoid reusing a stale prepared plan mapping.

**C. Hybrid — bind `?` predicates, interpolate projections/expressions.**
- Gains: none material over A.
- Loses: two mechanisms, two cache-key stories, more code. The projection path (decision 2) can't use `?` cleanly anyway (projections are raw strings, not predicates).

### Trade-offs

Interpolation trades a fold-values-into-the-key step for cache correctness that is otherwise free; binding trades injection-safety-by-construction for a re-keyed hot cache and type-conversion plumbing. The codebase already leans interpolation (the unused `Interpolated` mode) and the correctness-critical cache is SQL-string-keyed, which interpolation satisfies natively.

### Recommendation

**Option A (interpolation).** Close the `emit_query` stub by substituting param values into the emitted SQL — reuse `binding.rs`'s `Interpolated` mode for `ExpressionNode`s and a bounded `$name` substitution for lowerer-emitted projection strings, all via `spec_value_to_sql_literal`. Leave `render_predicate`'s `?`/`Binding` recording in place (it becomes the substitution plan); `execute_emitted` continues to run parameter-free SQL. Cite: sql_cache (lib.rs:742); binding.rs:54-98; gomb_ac12.

---

## Decision 2: Which param→SQL surfaces land in this slice?

### Context

Because simple marks emit `SELECT *`, no channel/expression currently reaches the SQL. To make *any* param reactive, at least one surface must lower a param into the query. There are three candidate surfaces, with escalating reach and cost.

### Evidence

- **Positional `$param` channel** (`y: $k`): already subscribes (probe: subscriber_graph has the mark) via `collect_value_or_param_ref_subscribers` matching `ValueOrParamRef::Param` (analysis.rs:318). But it never reaches SQL (SELECT *) and `ChannelMap` drops it (channel.rs:147).
- **Expression channel / data filter** (`y: "v*$k"`, `filter: "x > $k"`): does **not** subscribe — `collect_spec_value_subscribers` recurses only `Object`/`Array` and ignores `Expression`/`Param` (analysis.rs:330-344, `_ => {}`). `MarkData::From.extras` can already carry a `where`/`filter` (ast.rs:346) but no lowerer consumes it.
- **Aggregation option** (`bins: $n`, regression `ci: $ci`): lowerers read options as literals only (`opt_f64` skips `ParamRef`, lower.rs:76-81), so an option param is silently ignored.

### Options

**A. Positional `$param` channels only.** Lower a bare `$param` positional channel into the SELECT as `<param> AS "<alias>"`; map it in `ChannelMap`. Reuses existing subscription. Minimal. Demo: a constant channel that moves (dots/line/threshold-rule at `y: $k`).

**B. A + expression channels & data filters.** Also lower channel `Expression`s and a `data.filter` expression into SELECT/WHERE, and extend subscription to expression params (analysis.rs:342). Demo: scale/shift (`y: "v*$k"`) or **filter rows by a threshold** (`filter: "x > $k"`) — the canonical, most compelling slider use.

**C. A + B + aggregation-option params.** Also make per-lowerer options (`bins`, `ci`, bandwidth) param-driven. Touches every specialised lowerer.

### Trade-offs

A is the smallest genuinely-reactive slice and is fully headless-verifiable via the probe, but the demo (a constant channel) is artificial. B adds the subscription fix + one filter/expression lowering and yields the demo the sprint goal implies ("filter downstream queries by a swept variable"). C multiplies per-lowerer work for domain-specific gains with no corpus pressure yet.

### Recommendation

**Option B, staged: land A first (probe-green, the reactivity spine), then the data-filter expression path within the same slice.** Concretely: (1) positional `$param` channel projection + `ChannelMap` mapping; (2) `data.filter` expression → `QueryPlan::Filter` WHERE with param interpolation; (3) extend `collect_spec_value_subscribers` to subscribe params embedded in `Expression`s. Defer expression *channels* beyond filters, and all aggregation-option params (C), to a follow-up — record the boundary in the spec's implementation notes. Cite: analysis.rs:318 vs 342; ast.rs:346.

---

## Decision 3: Cache-key reconciliation — keep results correct across param changes.

### Context

`execute_emitted` has two caches that both assume SQL is param-independent: `self.cache: plan_hash → {sql, bindings}` reuses the SQL string when `plan_hash` matches (lib.rs:725-739), and `self.sql_cache: sql → batches` skips DuckDB on an SQL hit (lib.rs:742). Interpolation (decision 1) makes the SQL value-dependent, so `plan_hash` (structural) can now map to *different* SQL across param values — the `self.cache` hit at lib.rs:727 would hand back **stale SQL**.

### Evidence

- `plan_hash = plan.hash_structural()` (emit.rs:489) — structural only; identical for `k=3` and `k=20`.
- The `self.cache` contract (lib.rs:712-714) is literally "same plan_hash ⇒ reuse the cached SQL string" — the invariant that interpolation breaks.
- `gomb_ac12` relies on a **selection** param (routed through predicates, never inlined) → SQL genuinely identical across values → must stay a cache hit.
- `gomb_ac11` pins `sql_cache` LRU + `duckdb_execute_count` behaviour on unchanged SQL.

### Options

**A. Fold the interpolated `(param, value)` pairs into `plan_hash`.** Restore the invariant "plan_hash uniquely identifies the emitted SQL." A data-shape change → different plan_hash + different SQL → `self.cache` miss + `sql_cache` miss → re-execute. An SQL-invariant change (selection params, or a param not inlined into this mark's SQL) → plan_hash unchanged → both caches warm.
- Gains: preserves both caches' existing semantics and every cache test; the "pure tier" (no re-execution) falls out automatically for params that don't touch a given mark's SQL. Minimal, local to `emit_query`.
- Loses: `plan_hash` is no longer purely structural — its name/doc must be updated to "plan + inlined-param identity."

**B. Use `emitted.sql` directly; retire the `plan_hash → sql` reuse.** Since interpolation already yields concrete SQL, skip `self.cache`'s SQL-reuse and rely on `sql_cache` (SQL-keyed) alone.
- Gains: conceptually simpler — one cache.
- Loses: drops the plan-stability cache the codebase built and its tests assert (`cache_len`), a larger blast radius than A.

**C. Re-key `sql_cache` on `(sql, param snapshot)`.** Redundant once SQL is concrete (the SQL already encodes the params).

### Recommendation

**Option A.** Fold only the *actually-inlined* `(param, value)` pairs into `plan_hash` (selection params, which aren't inlined, stay out of the key → `gomb_ac12` unchanged). Update the `plan_hash` doc comment to note it now identifies the concrete SQL. Cite: lib.rs:712-739; gomb_ac12; gomb_ac11.

---

## Decision 4: How does a projected `$param` channel name its column (the cross-crate alias)?

### Context

Approach A projects `y: $k` as `<value> AS "<alias>"` in `brightfield-sql`, and `ChannelMap::from_mark` in `brightfield-render` must map `Channel::Y → "<alias>"` so the renderer reads it. Both crates derive the alias independently from the same `ParamRef`, so they must agree by construction.

### Options

**A. Alias = the param name** (`y: $k` → column `"k"`). Meaningful axis labels; both sides read `ParamRef.0`. Collision only if a real table column is *also* named `k` (then `SELECT *, $k AS "k"` duplicates `k`).

**B. Alias = a reserved name** (`"__bf_param_y"` or `"__bf_param_k"`). Collision-proof (mirrors the `__bf_count` precedent from card 0008 follow-ups), but the axis label becomes the reserved token unless labels are sourced separately.

**C. Alias = the channel name** (`"y"`). Collides with a real `y` column under `SELECT *`; semantically the param *overrides* `y`, but produces a duplicate-named batch column.

### Recommendation

**Option A (param name), with the collision documented as a known edge** (consistent with how card 0008 handled the analogous `count` collision — reserve later if a corpus spec hits it). The param name gives correct axis labels for free and is deterministic across crates. Note in implementation notes: if `SELECT *` already yields a column of the param's name, prefer that real column (the param binding is shadowed) or reserve-prefix — resolve when first observed. Cite: card 0008 `__bf_count` precedent.

---

## Decision 5: Where does the "two-tier" split live — explicit routing or emergent from the cache?

### Context

The card and the engine comments call for distinguishing "pure-style param drags (no SQL re-execution)" from "data-shape param changes." The naive reading is an explicit router that classifies each param and takes a render-only fast path for pure params.

### Evidence

- In this slice, every in-scope param effect goes through SQL (decisions 1-2) — there is no render-only param channel yet (`ChannelMap` param channels become SQL columns, not render constants).
- Decision 3 makes `propagate_param` re-emit for every subscriber, but a subscriber whose SQL is unchanged by the param hits `sql_cache` and **skips DuckDB** — i.e. the pure/cheap path already exists at the cache layer, keyed on whether the SQL actually changed.

### Options

**A. No explicit router; the cache layer *is* the tiering.** A data-shape param → SQL changes → cache miss → re-execute. A param that doesn't alter a given mark's SQL → cache hit → no DuckDB work. The distinction is computed, not declared.

**B. Explicit two-tier classifier + a render-only param path** (params bound to visual channels like `fill`/`opacity`/`r` resolved from `param_state` at render, no SQL).

### Recommendation

**Option A for this slice; defer B.** Realize the two tiers through decision 3's cache keying — it gives pure params a DuckDB-free update and data-shape params a correct re-execution with no new routing surface. Explicit render-only param channels (color/opacity/radius bound to `$param`, resolved render-side) are a clean follow-up card once a corpus spec needs a purely-visual param. Record the deferral. Cite: engine/lib.rs:153-160; decision 3.

---

## Deferred (record in the spec's implementation notes)

- Expression *channels* beyond `data.filter` (e.g. `y: "v*$k"`), and aggregation-option params (`bins`/`ci`/bandwidth) — decision 2 C.
- Explicit render-only param channels (visual-channel params with no SQL effect) — decision 5 B.
- Computed-param chains (`ParamNode::FromQuery`) — already deferred by card 0005 rpw3 (decisions.md case iii).
- Multi-param projection dedup / alias collision hardening — decision 4 edge.
