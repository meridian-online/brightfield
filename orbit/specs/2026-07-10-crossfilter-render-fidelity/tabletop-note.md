# Tabletop note — Cross-filter render fidelity

**Date:** 2026-07-10
**Cards in scope:** 0006 (cross-filter UX), 0009 (legend click-to-filter), 0017 (authoring workspace — renderer-config seam follow-up), 0008 (density marks — dmk_ac02 correction closure)
**Output spec:** orbit/specs/2026-07-10-crossfilter-render-fidelity/spec.yaml

Closed-space note, not a full 10-question session: the solution space was
fixed by the 2026-07-10 workspace walkthrough (Hugh's product finding: a
legend click filters correctly but "it's not really obvious what's
happening") plus two already-recorded spec corrections (dmk_ac02 and the
0016 raster-scheme corner both name the missing live renderer-config seam).
The three items share one territory — the coordinator's rebuild path in
`brightfield-ui/src/crossfilter.rs` — so they ship as one round with one
review.

## Capability ambition

A cross-filter gesture reads as *filtering* — the data changes while every
frame of reference (axes, colours, legend) holds still and the legend shows
which category is active — and a live rebuild renders each mark exactly as
its first render did.

## Values

**Load-bearing: frame-of-reference stability.** The walkthrough showed the
current behaviour is *technically correct but illegible*: subscriber
rebuilds re-infer scales from the filtered batch, so the axes re-fit and
every point moves — the user can't tell filtering from redrawing. Pinning
the launch scales makes the DATA the only thing that moves. Second value:
**engine as the single source of truth** — legend selected-state derives
from `Session::contributor_predicate` per gesture, never a UI mirror (the
card 0009 F1a/F1b lesson).

## Trade-offs

- **Launch-pinned scales mean a heavy filter can leave marks huddled in a
  corner of a big domain.** Accepted: that IS the honest picture of a
  filter, and it's Mosaic's own behaviour (fixed scales unless declared
  otherwise). Escape hatch (out of scope): a spec-level `scale: fit`
  opt-in later.
- **Pinning ALL channels (Fill/Colour too), not just x/y.** Chosen over
  positional-only: a Sequential ramp that re-anchors to the filtered max
  mid-gesture recolours every cell (same illegibility, colour axis), and a
  pinned Fill scale makes the static hosted legend honest again — the
  0016-deferred "hosted-legend live refresh on gesture-driven domain
  change" item dissolves rather than needing shared mutable scale plumbing.
- **Selected-state = dim the others, no ring/geometry change.** Panel size
  and entry rects stay identical, so hit-testing, placement, and the chrome
  gate are untouched; the raster cache key gains only the selected
  category.
- **Renderer override rides `MarkInput`, built once at assembly.** One
  construction site shared by the first render and every live rebuild —
  the alternative (re-deriving renderer config inside the coordinator)
  duplicates the app-layer attribute resolution and would drift.

## Halt conditions

- Any pre-existing example PNG byte-diff → halt, revert the offending
  change. The headless path renders launch state (no selection, first
  render), so ALL of: the scene-build refactor, the legend selected-state
  variant with `None`, and the renderer-override move must be byte-neutral.
  This is the same exemption-free gate the last four rounds ran.
- The scene-build refactor (pinned-scales seam) forcing a change to
  `build_chart_scene`'s or `build_multi_mark_scene`'s public behaviour →
  halt; add a parallel entry point instead.

## Escalation triggers

- If pinning scales surfaces a mark whose renderer *requires* re-inference
  to draw at all (a scale synthesised from batch data that the launch
  batch didn't produce), surface the mark + fixture and propose per-mark
  exemption vs pin-with-fallback. [imagined — recon found none: launch
  scales are inferred from launch batches, which are a superset of every
  filtered batch's domain.]
- Any needed engine-crate change beyond read-only lookups → escalate
  (contract: `contributor_predicate` already exists; this round adds no
  engine API).

## Kill conditions

- If launch-pinned scales are judged wrong-in-product by Hugh's eyeball
  (e.g. filtered marks unreadably small in a pinned domain), the pivot is
  a per-plot re-fit opt-in — the pinning seam itself survives, only the
  default flips. The seam work is not wasted in any outcome.

## Known adjacent gap (recorded, out of scope)

The hot-reload watcher's Applied branch swaps plot scenes only — the
coordinator (session, batches, launch scales, renderer overrides, legend
bindings) is never rebuilt on reload. A data edit that hot-reloads followed
by any gesture re-executes the OLD session and reverts the plot. This
pre-dates this round and is unchanged by it; "launch-pinned" here means
"pinned when the coordinator is built at window launch". Follow-up owns:
rebuilding or refreshing the coordinator on Applied (note the
`PlacedLegend` elements hold `Rc<RefCell<CrossfilterCoordinator>>` clones,
so the refresh must swap contents inside the RefCell, not the Rc).

## Verification posture

- Scale pinning, round-trip identity, renderer-override survival, and the
  selected-state state machine: `verifies: capability` — all drive the
  Entity-free seams (`render_plot_scene`, `apply_legend_click`,
  `apply_slider`, `build_legend_scene`) against real sessions, headlessly.
- The interaction FEEL (axes visibly hold, dimming visibly reads as
  "filtered to X"): `verifies: capability` only via Hugh's in-app manual
  AC — the headless scene assertions are the stand-in
  (`verifies: stand-in (real thing is the in-app gesture), accepted
  because the walkthrough loop closes it same-week`), and the PR holds
  for that eyeball per the standing testing preference.

## Budget

One Claude working day including review round. Tripwire: if the
scene-build refactor churns beyond scene.rs + crossfilter.rs signatures,
cut item order to (3) renderer seam → (2) legend state → (1) pinning, and
ship what's green.
