# Mark breadth — text labels (card 0008)

A `text` mark draws a string label at each `(x, y)` data position — annotations
and labelled scatters. The first mark to need a non-positional encoding channel.

## What it took

- **New `Channel::Text`** (the label-content channel). Added to the enum +
  `wire_name`/`from_wire`/`all()`. Because the channel value is a string, the
  existing `from_mark` String→column path handles `text: name` with no extra
  work (it maps to the `name` column). Scale inference gives the Text channel a
  degenerate (unused) scale — harmless, the renderer never reads it.
- **`TextRenderer`** — reads x/y (position, via `resolve_position`, numeric or
  categorical like the dot mark) + the text column (strings), and draws each
  label centred on its point via the bundled-font `draw_text` (reused from the
  axis/legend label work). Registered + SimpleLowerer + vocab→Implemented;
  lowerer-count test → 14.
- **`count_scene_glyphs` test helper** — glyphs encode into
  `Scene::encoding().resources.glyphs`, NOT `n_paths`, so a text mark reports
  `count_scene_paths == 0`. The new helper counts glyphs; the test asserts one
  glyph per label character.

`examples/text.yaml` — a labelled scatter of languages by speed/ergonomics —
renders each name at its position (verified by PNG).

## Deferred

- **`textX`/`textY`** (1D variants) stay Unimplemented — like `dotX`/`lineX`,
  the basic renderer needs both x and y.
- **Label offset** (`dx`/`dy`) — labels currently centre exactly on the point, so
  a label paired with a dot overlaps it and edge labels clip at the plot
  boundary. An offset channel would place labels beside their marks.
- **String/literal text** (`text: "Total"`) — a constant label needs string
  literal channel support (ambiguous with column names; deferred with the other
  string literals).
