Findings from the adversarial review of the rect mark family (card 0008 — rect / rectX / rectY). Each item is follow-up work against an existing card — distill into specs, don't open new cards. The rect family ships with the load-bearing path (shared-axis synthesis, zero baselines, degenerate/NaN skipping) covered and eyeball-confirmed; these are the deferred edges.

## 1. Ranged-axis synthesis is Linear-only — non-linear siblings clip, time bins render as µs

`RectRenderer::augment_scales` synthesises the shared `Channel::X`/`Y` a ranged rect needs via `merge_linear_scale`, which only ever produces a `Scale::Linear`. Two consequences, both inherited from the shared synthesis path (`regression`/`density` have the identical constraint):

- **Non-linear sibling clobbers the rect's contribution.** When another mark in the same plot already established a non-linear `Channel::X`/`Y` — a `line` over a `Timestamp` (→ `Scale::Time`) or a `bar` over a `Band` — `merge_linear_scale` early-returns on the non-linear scale (`scale.rs` `Some(_) => return`), so the rect's `x1`/`x2` (`y1`/`y2`) extent never widens that axis. Bins past the sibling's domain map beyond `range_end` and are silently clipped by the plot-area layer. Realistic trigger: a `line` (Jan–Jun) overlaid with a time-binned `rectY` volume histogram (Jan–Dec) — roughly half the bars vanish with no error. (major)

- **Standalone time-binned rect shows microsecond integers.** A lone `rectY` over `Timestamp` bin edges synthesises a plain `Linear` `Channel::X` over raw-microsecond f64 (`column_as_f64` casts `Timestamp → µs`), so axis ticks read `1.71e15` rather than a `Time`-formatted label. Geometry is correct; only the axis labels degrade. Note the current `Time` axis is itself only "v1 seconds" formatting, so the payoff is bounded until time-tick formatting improves too. (minor)

The fix is **time/band-aware ranged-axis synthesis**: detect the type of the `X1`/`X2` (`Y1`/`Y2`) scales `infer_scales` already built and synthesise a matching `Scale::Time` (unioning domains) instead of forcing `Linear`; for an incompatible `Band` sibling, warn rather than silently clip. This is a shared improvement (rect + regression + density), not rect-specific.

→ Spec under card 0008 (mark library — "each mark type draws correctly on top of scales, axes"). Sequence alongside the `Scale::Time` tick-formatting improvement.

## 2. NaN in a bound column (FIXED in this PR)

A genuine (non-null) `NaN` in a rect's edge column survived the null guard and, when BOTH endpoints of an axis were NaN, bypassed the zero-area skip (`NaN < EPSILON` is false) — handing `Rect::new(NaN, …)` to Vello. Fixed by an explicit `is_finite` check on all four pixel edges before the zero-area test, which also rejects the ±inf a zero-span synthesised scale can produce. Covered by `rect_skips_non_finite_edges`.

→ No further action; noted for the record.
