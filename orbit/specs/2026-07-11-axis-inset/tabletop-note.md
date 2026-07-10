# Tabletop note — axis inset (edge-point trim)

**Date:** 2026-07-11
**Cards in scope:** 0008-adjacent render fidelity (the product finding queued from the
crossfilter-render-fidelity round, spec.yaml:53 there); touches 0006's gesture seam
**Output spec:** orbit/specs/2026-07-11-axis-inset/spec.yaml

Authored by the driving agent (plugin 0.4.38 blocks model-invoked /orb:tabletop).
Ground truth from a ref-pinned recon against main @ 8877311 (2026-07-11).

## The finding and the mechanism

Hugh's retest finding, recorded verbatim in the crossfilter spec: *"edge points at
domain bounds clip to slivers (pre-existing — no domain padding/inset) — queued as
the axis-inset round."* Recon confirms the full chain: scale ranges land domain
edges exactly on the plot-frame pixels (`layout.rs:87-95`, `scale.rs:57-70` —
value at domain_max maps to range_end exactly), and a single clip layer at that
same frame (`scene.rs:84-91`, pushed at `:170-174` and `:307-318`) trims the
overhang. A dot at domain min spans [36,44] around frame-x 40 and loses its left
half. No inset attribute is consumed anywhere — the Mosaic keywords exist only as
`$param`-lift positions (`parse.rs:85-94`) and sit unread in `PlotNode.attributes`.

## Values

**Author-honest rendering by default.** An analyst's scatter must not trim data
at the domain extremes — the author declared the data, not an inset; the default
must render it whole. Second value: **Mosaic parity in the vocabulary** — the
plot-level inset attribute family resolves with Plot's most-specific-wins
semantics, so corpus specs that carry insets (presidential-opinion, axes,
wind-map, voronoi, driving-shifts, population-arrows) become honest citizens.

## Trade-offs

- **The load-bearing deviation: a nonzero default.** Mosaic/Plot default inset is
  0 — but Plot doesn't clip marks by default, Brightfield does (the frame clip is
  deliberate and stays; it's what keeps gesture-widened data tidy). With default 0
  the finding stays open. Brightfield defaults `DEFAULT_SCALE_INSET = 5.0` px
  (DOT_RADIUS 4 + 1 breathing) on continuous positional scale ends. Explicit spec
  insets — including explicit 0 — always win, so Mosaic-exact rendering is one
  attribute away. Deviation recorded here and reviewable by Hugh in the gallery;
  he vetoes by gallery ack, not by archaeology.
- **Zero-baseline ends are exempt from the default** — bars and areas stay flush
  on their axis baseline instead of floating 5px. The exemption keys on the hook
  that already exists: `renderer.zero_baseline_channel()` (`scene.rs:136-141`) —
  on that channel, the end pinned to 0 by `extend_domain_to_zero` gets no default
  inset. Multi-mark plots exempt a channel if any mark in the plot declares it.
- **Band scales get no default inset** (band `padding` already owns categorical
  edge spacing); explicit insets still apply to the range extent.
- **Two layout models move together, not merged.** Render (`layout.rs`) and
  interaction (`chart_layout.rs`) duplicate the margin model and agree by luck
  today — `crossfilter.rs:32` imports the render one, `chart_state.rs:18` the ui
  one. This round mirrors the inset into both and pins agreement with a
  cross-model test; unification is held in reserve (lateral approach), not
  attempted — the simplest cut that holds the value.
- **Every example PNG re-baselines.** Position is `range_start + t*(range_end -
  range_start)`; any inset moves every mark. This is the round's declared cost —
  the before/after gallery (new tooling; none exists, prior "galleries" were
  Hugh eyeballing dumped PNGs) is how the cost stays reviewable. The before set
  is captured from main at branch point (post-hexbin), not the stale cfr set.

## Halt conditions

- A re-baseline diff **not explainable by the inset** (anything beyond marks/axes
  shifting by the resolved inset amounts — e.g. text reflow, colour change) —
  halt, investigate before re-baselining that file.
- Vendored 54-spec corpus regression after the attrs become consumed — halt.
- Suite red outside the sanctioned churn (layout.rs `gpu_layout_*`,
  chart_layout.rs `gmr_ac08_*` re-pin to inset-aware values) — halt.

## Escalation triggers

- The per-channel exemption can't reach range computation cleanly (ranges are
  built in layout before scales exist; the exemption needs the renderer set) —
  surface mechanism options (single resolution fn over attributes + mark set,
  threaded to both models; vs layout-owned with a renderer parameter) before
  committing to a refactor.
- Mirroring insets into both layout models starts forcing de-facto unification —
  stop and surface; unification is a reserve decision, not a drive-by.
- Brush/point inversion can't be made consistent without touching the
  coordinator's launch-scale stash — surface with the exact call path.

## Kill conditions

- **Claim: inset composes cleanly with widen-only anchoring** (anchor preserves
  launch `range_start/range_end` verbatim — `scale.rs:359-398` — so a launch-baked
  inset survives every fold). Killed if an anchored rebuild shifts pixels with a
  nonzero inset → pivot: bake insets into launch-scale construction only, and
  re-derive displayed ranges from launch; flag to Hugh.
- **Claim: the zero-baseline exemption is derivable where scales are built.**
  Killed if the hook can't distinguish the baseline end → pivot: default inset
  applies everywhere (bars float 5px, visible in the gallery) OR default drops to
  0 and our own examples gain explicit insets; Hugh decides via gallery.
- **Claim: all 27+ diffs are inset-explainable.** Killed by an unexplained diff
  class → the round stops at the halt and the diff class becomes its own
  investigation before any re-baseline lands.

## Verification posture

- Attribute resolution, range insetting, cross-model agreement, inversion
  round-trip, widen-only composition, edge-dot scene probe:
  `verifies: capability`.
- The gallery review of every re-baselined PNG:
  `verifies: stand-in (real thing is Hugh's eyeball over the full before/after
  set), accepted because` the PNG gallery loop is established and the whole
  round's cost is visual — the gallery IS the review artifact.

## Budget

~1 Claude-day: resolution fn + both layout models + defaults/exemption ≈ 0.5;
inversion/widen-only probes ≈ 0.25; re-baseline + gallery tooling ≈ 0.25.
Tripwire: if the exemption mechanism isn't settled by mid-round, escalate per
kill condition 2 rather than improvising a mark-type whitelist.

## Sequencing

Implementation is held until the hexbin round merges — this round re-baselines
every example PNG including hexbin's new ones, and the before set must be
captured from post-hexbin main. cfr-baselines retires as canonical when this
round's after set lands.
