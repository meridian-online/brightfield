# /orb:review-spec — Legend interaction fidelity (2026-07-13)

**Verdict: REQUEST_CHANGES → folded to spec v1.1** (same-session turnaround; blast radius confined to lif-ac05).

Method: a 4-lens adversarial workflow (`wf_ab6a3840-37a`, 23 agents) — testability, completeness, load-bearing-claim correctness, adversarial edges — each finding then independently verified as *real-and-unaddressed* vs *already-covered*. 18 raw findings → 6 survived verification (1 MEDIUM-from-HIGH, 2 MEDIUM, 3 LOW). 12 were DROPPED as already pinned by the spec's constraints — evidence the constraint set was mostly sound.

## Must-fix (all on lif-ac05, the shift-click union) — RESOLVED in v1.1

1. **Foreign-Expr fold (HIGH→MEDIUM, correctness).** The decompose rule `Expr(e) → {e}` unconditionally absorbed whatever bare Expr occupied the shared contributor slot into the OR union — including a *foreign* predicate. A legend and a same-plot point interactor share one `(selection, contributor)` slot, and `point_predicate` produces a bare `Predicate::Expr` (brush.rs:166), so shift-clicking a category while the slot held e.g. `Expr("x = 5")` would build `Or([Expr("x = 5"), Expr("species='gentoo'")])` — the exact weird Or the open_question meant to forbid, and a direct contradiction of ac08's membership-based derive (which *drops* that foreign Expr → returns `[]`). Build desynced from display.
   → **Fix:** ac05's decompose now INTERSECTS the raw member set against the legend's candidate categories' `point_predicate`s (the same membership logic as `legend_selected_categories`, ac08); any non-member is dropped and the slot collapses to a fresh REPLACE. Requires threading the candidate categories into `apply_legend_click`/`commit_legend_click` (they receive only `hit: Option<&str>` today). Added a foreign-slot fixture; broadened the open_question and marked it RESOLVED.

2. **Debug-vs-Display slot_expr (MEDIUM, testability).** ac05's `slot_expr.contains(" OR ")` could never pass: the cited helper (crossfilter.rs:2103) formats via Debug (`{p:?}`), whose union renders `Or([Expr(..), Expr(..)])` — no `" OR "` substring (that's Display-only, ir.rs:71). Worse, ac06's "never `' OR '`" regression floor was vacuously true under Debug → **toothless** (a plain-click regression wrongly emitting an Or would stay green).
   → **Fix:** ac05/ac06 now require `slot_expr` to format via Display (`{p}`) — safe, every existing assertion is a literal-substring/None check that holds under Display — or to assert structurally on `Predicate::Or(v)`.

3. **Shift + hit=None (MEDIUM, edge-case).** Unspecified. The natural impl threads `additive` into the `Some(cat)` branch only, leaving the existing `None`-branch clear (crossfilter.rs:567-575) to run under shift — so a shift-click landing in an inter-row gap (which the whole-panel PointingHand cursor *deliberately invites*) would silently wipe the entire multi-select union.
   → **Fix:** ac05 now specifies shift+hit=None as a NO-OP (gate the None-branch clear `if !additive`); only a plain empty-space click clears. Added a fixture and tightened the semantics constraint.

## Nice-to-have (LOW) — also folded into v1.1

- **ac03 background-fill was an infeasible option.** `render_swatch_legend_at` draws only the swatch rect + label — no per-entry background rect — so a "subtle background fill" would be *new geometry*, tripping both the cfr_ac06 `path_data` invariant and the byte-identical no-rebaseline floor. Struck it; constrained hover emphasis to a tint/alpha-bump of existing geometry.
- **ac04 conflated two gates.** Split the AUTOMATED at-rest scene probe (cargo test) from the MANUAL pre-merge PNG sweep (`scripts/compare-example-pngs.sh`, `cmp -s`, no committed goldens), matching the keyboard-grammar spec convention.
- **ac10 used the wrong resolution + mismatched spacing.** Legends only ever resolve Crossfilter (LegendBindingNonCrossfilter); the unit must use a contributor whose path ≠ self_source (else it's dropped → `WHERE TRUE`), and use consistent spaced Expr strings (render_predicate emits Expr verbatim). Corrected.

## Dropped (already covered — the constraints held)

hot∧dim precedence (pinned by the hover-emphasis constraint); multi-legend isolation (per-`PlacedLegend` cell, the card-0009 model); presentation-mode affordance (substance pinned by is_clickable gating); hot-reload staleness (ChromeSnapshot-gated, benign); the hover change-gate (self-hedged in the constraint); the brush cross-element case (no capture, not a gap); the hover change-gate testability.

## Strengths (verifier consensus)

- UI/render-only with an **empty-git-diff** machine gate over engine/sql/spec — an unambiguous blast-radius check.
- Regression discipline: byte-identical no-rebaseline floor + scene-probe at-rest gate + reuse of the cfr_ac06 `path_data` invariant.
- The **derive-not-mirror** invariant (ac08) is the right decoupling — and the most serious finding was precisely one ac05 arm *breaking* it, which shows the invariant is sound and legible enough to catch the desync.
- High testability: every finding was a refinement to a check that was already *present*, not a missing check.
