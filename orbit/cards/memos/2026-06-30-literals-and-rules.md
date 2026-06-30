# Literal channel values + rule marks (harden + card 0008)

Two intertwined increments: **literal channel values** (a harden item) and the
**rule mark family** they unlock. Brushing/baselines/thresholds like `y: 0` now
work, and `ruleX`/`ruleY` draw reference lines.

## Literal channel values (numeric)

A channel could only reference a data column (`y: count`); a numeric constant
(`y: 0`) was silently dropped. Now `ChannelMap` carries a separate
`literals: HashMap<Channel, f64>` alongside the column map — additive, so the 14
existing column-based renderer call-sites and `infer_scales` are untouched.

- `ChannelMap::from_mark` maps `SpecValue::Integer/Float` → `insert_literal`;
  strings stay columns. `literal(channel)` / `literals_iter()` read them.
- `scale.rs::extend_scales_with_literals` folds literals into the scale set after
  column inference: a literal on x/y extends an existing Linear domain to include
  the value (so an off-range constant stays on-plot), or synthesises a Linear
  scale around it when no column gave that axis a scale. Non-linear axes are left
  alone. Called from both `infer_scales` and `infer_scales_multi`.

Scope: numeric literals only. String literals (e.g. `fill: steelblue`) are
ambiguous with column names and deferred.

## Rule marks (ruleX / ruleY)

`RuleRenderer { axis }` draws thin lines spanning the plot, positioned by one
channel that may be a **literal** (one rule, e.g. `y: 30`) or a **column** (one
rule per row, e.g. thresholds from a table). Registered + SimpleLowerer +
vocab→Implemented; lowerer-count test → 13.

Constraint (tested): a rule spans the **perpendicular** axis, so that axis's
scale must exist — it does in a multi-mark plot (a sibling provides it) or when
the rule's own data gives it. A standalone single-channel rule with no
perpendicular data draws nothing (`rule_renderer_needs_perpendicular_scale`).

`examples/rules.yaml` — a scatter + a literal baseline (`y: 30`) + two
column-driven thresholds (`y: level`) — renders three horizontal reference lines
spanning x (verified by PNG).

## Deferred

- **Dataless literal marks**: a literal-only rule still needs a `data:` source to
  get a batch through the pipeline (the renderer ignores the rows). A true
  `ruleY: 0` with no data would need the engine to emit an empty batch for a
  dataless mark.
- **String/colour literals** (`fill: red`, `stroke: …`) — ambiguous with columns;
  rules currently draw in the default colour.
- **Literal positions on other marks**: only `RuleRenderer` reads literals today;
  dot/line/area/bar still require column channels (a literal `y` on a dot no-ops).
- Generic `rule` (non-X/Y) left Unimplemented; Mosaic uses ruleX/ruleY.
