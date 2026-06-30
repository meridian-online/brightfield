# Mark breadth — areaY (card 0008, round 1)

First new mark since the original dot/line/bar/density/regression set. `areaY`
renders the filled band between the `y = 0` baseline and the value line — area
charts. Establishes the pattern for adding a mark.

## The pattern (what a new mark needs)

1. **Renderer** — a `MarkRenderer` impl in `brightfield-render/src/mark.rs`.
   `AreaRenderer` collects valid `(x, y)` pixel pairs (x-sorted, like
   `LineRenderer`), builds a closed `BezPath` baseline → value line → baseline,
   and fills it with the resolved colour softened by `AREA_FILL_ALPHA`. It
   returns `zero_baseline_channel() = Some(Channel::Y)` so the scene builder
   extends the y-domain to include 0 (the baseline sits on-plot).
2. **Lowerer** — register the kind in `brightfield-sql` `default_lowerers`.
   `areaY` reuses `SimpleLowerer` (a plain `SELECT`); the renderer sorts.
3. **Registration** — add the kind→renderer pair to `default_renderers`.
4. **Status** — flip the vocab entry to `Implemented` so it no longer emits a
   spurious `Unimplemented` parse warning.
5. **Tests + example** — a render test (`count_scene_paths == 1` for a filled
   area; `0` for <2 points) and `examples/area.yaml` (areaY + a line edge),
   verified by `BRIGHTFIELD_DUMP_PNG`.

## Note on the vocab/runtime status drift (→ harden card)

The genuinely-implemented marks `dot`/`line`/`barX`/`barY`/`regressionX/Y` are
still marked `Unimplemented` in the vocab, so every run prints false
"Unimplemented" warnings — and a test (`parse.rs:1525`) actively asserts `dot`
warns. Flipping those is a deliberate, test-coupled call about what "Implemented"
means (renders vs. fully spec-conformant), so it belongs to the harden-the-render
card with its conformance layer, not this additive change. `areaY` is set
`Implemented` because it's brand new (no test encodes the old state).

## Next marks (each follows the pattern)

- **rule / ruleX / ruleY** — reference lines. Wrinkle: a rule spans the
  *perpendicular* axis, so that axis's scale must exist. Holds in multi-mark
  plots (a sibling provides it) and where the rule's own data spans it; a
  standalone single-channel rule plot would need `infer_scales` to synthesise a
  full-range scale for the unmapped axis. Worth handling deliberately.
- **rect / rectX / rectY** — interval rectangles (histograms via `bin`); needs
  x1/x2 (or binned x) interval channels.
- **text / textX / textY** — labels at data positions; reuse `text.rs::draw_text`,
  needs a text channel.
- **areaX** — symmetric to areaY (baseline at x=0, y-sorted).
