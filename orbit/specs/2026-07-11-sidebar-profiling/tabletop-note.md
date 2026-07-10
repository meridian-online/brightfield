# Tabletop note — sidebar profiling (card 0017's next increment)

**Date:** 2026-07-11
**Cards in scope:** 0017 (authoring workspace — the "Column profiles on tap" scenario)
**Output spec:** orbit/specs/2026-07-11-sidebar-profiling/spec.yaml

Authored by the driving agent (plugin 0.4.38 blocks model-invoked /orb:tabletop).
Ground truth from a ref-pinned recon against main @ 8877311 (2026-07-11).

## Where the seam already is

The sidebar skeleton ships a v1 approximation: `sidebar_model::derive_source_listings`
reads column *names* off already-executed mark batches — no DuckDB round-trip, no
types, unconsumed sources listed empty, listings frozen at launch (the watcher never
receives the sidebar entity). This round replaces the approximation with real
DuckDB-computed per-column profiles (count, min/max, distinct, nulls — the card's
exact list) and, in doing so, un-freezes the sidebar on hot reload.

## Values

**Authoring insight at editor speed** — the MotherDuck Column Explorer benchmark:
an author opens the Data sidebar and sees what each source actually holds, typed
and profiled, without leaving the app or writing a query. Second value: **zero
render-path risk** — this is a chrome-only round; every example PNG stays
byte-identical with no exemptions, which keeps it safely parallel to the hexbin
round. Third: the **semantic-layer rule** holds — profile computation is engine
code, profile formatting is model code, the panel is a shim.

## Trade-offs

- **Threading is the design centre.** The live `Session` is non-`Send` and
  UI-thread-pinned inside the coordinator's `Rc<RefCell<...>>`; there is no
  pattern for ad-hoc background queries against it, and this round does not
  create one. Profiles come from the two sessions that already exist outside
  the gesture path: the **launch session** (profiles computed synchronously in
  the build path before the window opens — launch already runs every mark
  query) and the **watcher's throwaway session** (already built fresh on the
  background executor per reload, already `Send`-safe by dropping before
  return). The coordinator is never touched.
- **One aggregate pass per source, no sampling v1.** count/nulls/approx
  distinct/min-max for all columns in a single SELECT over the view; DuckDB is
  the engine we're demoing, a scan per source at launch/reload is the honest
  cost. approx_count_distinct, not exact. If jank shows on real corpora, the
  kill-condition pivot moves launch profiling off-thread behind a skeleton
  state.
- **min/max are type-gated** (numeric + temporal only); varchar and friends show
  count/distinct/nulls. Universal min/max is a formatting swamp (long strings,
  blobs) for marginal insight.
- **Attached-database sources (`.duckdb`/`.db` ATTACH) are not profiled v1** —
  they need table-qualified introspection (`DESCRIBE "name"` fails on an
  attached catalog); they render a muted "(attached database — not profiled)"
  row. Recorded deferred, not silently skipped.
- **Display stays flat: name + type + stat line per column, scrollable, capped**
  at a displayed-column limit with a "(+N more)" tail. Collapsible groups,
  search, histograms, virtualized lists are all deferred — the card scenario
  asks for visible profiles, not an explorer.
- **Profiles describe the SOURCE, not the current crossfilter selection** —
  full-table stats, not filtered-state stats. Deliberate and recorded; a
  selection-aware profile is a different (coordinator-coupled) feature.

## Halt conditions

- Any example PNG diff — this round has no sanctioned visual change at all.
- Suite red outside the round's own new tests.
- Borrowing the coordinator's session for profiling — that's not a bug to fix
  but a design violation to revert (UI-thread jank class).

## Escalation triggers

- The watcher's `Send` return boundary can't carry the profile set cleanly
  (e.g. Arrow types don't cross) → surface the exact type; the pivot space is
  stringly-typed profile rows vs a serialisable profile struct.
- The launch-path profiling measurably delays window-open on the example corpus
  → surface timings before reaching for the async pivot.
- Per-source failure isolation forces error-type surgery in brightfield-engine
  (new EngineError variants rippling into existing matches) → surface the
  blast radius first.

## Kill conditions

- **Claim: one aggregate pass per source is cheap at launch.** Killed by
  measurable window-open delay → pivot: launch profiles move to the background
  executor with their own throwaway session (the watcher model) behind a
  skeleton loading state; flagged to Hugh.
- **Claim: the watcher seam carries profiles across the Send boundary.** Killed
  → pivot: v1 ships launch-only profiles with the frozen-at-launch gap
  explicitly recorded on the card, and the refresh becomes its own increment
  riding the coordinator-refresh chore.
- **Claim: DESCRIBE works for every non-ATTACH view kind.** Killed for a kind →
  that kind renders the muted unsupported row and joins the deferred list;
  never a crash, never a blank sidebar.

## Verification posture

- Engine profile computation (DESCRIBE + aggregate against inline fixtures,
  mixed types, nulls, failure isolation), model formatting, panel rendering,
  watcher refresh: `verifies: capability`.
- Hugh's in-app look (profiles visible, reload updates, failure row + Log
  entry): `verifies: stand-in (real thing is Hugh's eyeball), accepted because`
  the in-app confirm loop is established and closes same-week.

## Budget

~1 Claude-day: engine profile method + tests ≈ 0.4; model + formatting ≈ 0.2;
panel render + shell tests ≈ 0.2; launch/watcher threading ≈ 0.2. Tripwire: if
the watcher seam isn't carrying profiles by mid-round, ship launch-only per
kill condition 2 rather than stalling the round.

## Sequencing

Runs parallel to the hexbin round off main @ 2bf2d03. Overlap is additive-only
(new engine method + tests; main.rs launch/watcher taps) — if hexbin merges
first, rebase is expected to be trivial; keep engine additions purely additive
(no signature changes to existing methods).
