# Colour's scale, domain and legend are set on its shelf entry

## Outcome

With the cursor on colour's shelf entry, `Tab` turns its list to settings, and an analyst sets colour's scheme, scale, legend, reverse, domain, pivot and steps, by key and again by click. Each row reads auto until it is set and carries a dot once it is. The chart and its legend redraw on each change. The legend row places the legend at the right, below the plot, or takes it out. After Save and reopen, the chart and the file both show what was set. A file with no legend node draws no legend.

## What is already on main

- The render crate reads `colorScheme` (viridis, the default, plus blues, turbo and meridian), `colorScale: diverging` about `colorPivot`, `colorScale: quantize` with `colorN` (five when absent), `colorReverse`, and `colorDomain` with `colorRange`. A colour key brightfield cannot draw draws as if absent, and the page's warning names it.
- A plot draws a legend where its file holds a legend node and none where it holds none: at the right as a column, below as a band 44 high when the file is a `vconcat` of the plot and a `legend: color` whose `for:` names it.
- Putting a number column on colour writes `colorScheme: viridis` and a `legend: color` item. `LegendPlacement` (`crates/brightfield-spec/src/edit.rs`) moves the legend between right, below and none.
- Colour's settings list has two head rows, scheme and reverse (`colour_rows`, `crates/brightfield-shell/src/shelf.rs`). The scheme row shows a swatch and, under the cursor, a strip of the schemes that apply. A row that does not apply is muted with its reason.

## Rulings this design works inside

- The legend's place is the chart's, with an override on the colour cell. A file says *no legend* by leaving the node out, as Mosaic draws it. Right is a `legend: color` item in the plot, so the tile stays one plot. Below is a `vconcat`.
- The scheme is picked by eye from the strip of ramps.
- The legend marker is a ring, not the bar an axis gets, because a legend is a block.
- The auto pivot is zero where the column crosses zero, and the column's median otherwise. The auto domain on diverging is even about the pivot, so one arm may use only part of its colours. A fixed domain gives each arm its full range, and no extra switch is added.
- The diverging pair is written as `colorRange`, the light theme's three stops, since Mosaic has no scheme of that name. Brightfield recognises its own pair and draws the dark theme's middle in dark mode.
- Steps are drawn in the count asked for, where Mosaic's renderer rounds to tidy thresholds.
- A legend's low end is the column's minimum, not zero, as Mosaic draws it.
- Where brightfield's default differs from Mosaic's, the saved file names it, and a row tells a set value from the default by comparing values.
- A row that does not apply is drawn muted with its reason.

## Design

### 1. The scale row

Colour's head rows become scheme, scale, legend and reverse, in that order. Domain, pivot and steps are reached by typing their names.

The scale row steps through linear, diverging and steps with `←` `→` (or `h` `l`, or a click on the value).

| Value | Keys written | Keys removed |
|---|---|---|
| linear | none | `colorScale`, `colorPivot`, `colorN`, and the diverging pair's `colorRange` |
| diverging | `colorScale: diverging`, `colorPivot: <auto pivot>`, `colorRange: <the light theme's three stops>` | `colorScheme`, `colorN` |
| steps | `colorScale: quantize`, `colorN: 5`, `colorScheme: viridis` | `colorPivot`, the diverging pair's `colorRange` |

The pivot and the count are written as soon as their scale is chosen, because Mosaic's defaults differ: its diverging pivot is 0, and its quantize thresholds are rounded.

The row carries its parameter behind the word:
- diverging reads *diverging about 1.74*;
- steps reads *steps · 5*.

The parameter is muted while it equals brightfield's auto value. Once it differs, it is in full ink with the row's dot. The auto pivot the row compares against is computed from the column's load-time profile: zero where its minimum is below zero and its maximum above, otherwise its median. The profile is the table's own, so the comparison does not move under a sweep. Values are compared at the precision the row displays.

Enter on the scale row opens the parameter's field in place, holding the current value selected. Typing previews on the chart, Enter keeps the value, and Esc drops it. A value the field cannot take (not a number; a count below 2 or past the most steps `colorN` allows) is refused in words under the row, and the field stays open. A reset (`⌫`, or a click on the dot) on the scale row puts it back to linear, which also removes its pivot or count.

The pivot and steps rows found by name show the same value as the scale row and edit the same key. Each is muted with its reason when the scale is not its own: *pivot applies to a diverging scale*, *steps applies to a stepped scale*.

The scheme strip offers the design system's blue-red pair and Mosaic's `rdbu` on diverging, and the four sequential schemes on linear and steps. On diverging, the scheme row reads the pair as auto.

The zero anchor goes in this piece. `augment_fill_ramp` (`crates/brightfield-render/src/mark.rs`) anchors a non-negative column's ramp at `[0, max]` today. It becomes `[min, max]`, as the ruling on the legend's low end has it, and the legend baselines move once for it.

### 2. The domain row and `colorDomain: Fixed`

When another tile sweeps a range, a colour re-fits to the rows in hand by default, as it does today and as Mosaic does. That shows the most detail inside the subset. An analyst who wants colours to keep their values across sweeps, to compare the subset with the whole, asks for it on the domain row. The file then says so in Mosaic's own words.

The domain row reads one of three values:
- *auto · 0.50 – 15.00*: the ends drawn, muted;
- *fixed*: with the dot;
- *0 – 10*: two typed ends, with the dot.

`←` `→` step between auto and fixed. Enter opens a two-ends field, the editor the axis range row uses, and Enter keeps it.
- Fixed writes `colorDomain: Fixed`.
- Typed ends write `colorDomain: [lo, hi]`.
- A reset removes `colorDomain`.

The domain row is muted with its reason when colour paints names. Putting a different column on colour removes a set domain and a set pivot, since both were values of the old column.

The reader:

- `brightfield-spec` reads `colorDomain: Fixed` as a pin request beside `xDomain` and `yDomain`. It is no longer a key brightfield cannot draw, so it leaves the page's warning.
- `PinnedDomains` (`crates/brightfield-render/src/scale.rs`) gains a colour pin: the colour scale's two ends, plus the pivot when the scale is diverging. It is captured from the scales of the plot's first composition, as the positional pins are, and applied where `apply_colour_override` runs. Re-domaining keeps the scale's own stops and kind. A pin and a scale of different kinds (the colour changed from a ramp to categories) leave the fresh scale standing, as the positional pins do.
- The capture moment is Mosaic's: a file opened with a selection already filtering its plot pins the filtered domain. A *fixed* set from the row is captured unfiltered, because an edit rebuilds the dashboard and a rebuild drops the committed selection.
- `Fixed` on a colour that paints category names is not read in this work. It draws as today, and the page's warning names it.
- Under an automatic sample, the pin holds the ends of the sample's first draw. A ramp anchored to the unsampled column is a separate piece of work.

### 3. The legend row and its ring

The legend row steps through right, below and none, writing through `LegendPlacement`. It reads the placement the file holds. The row's auto is right, which is what the colour edit writes. Below and none carry the dot.

While colour's list is open on either tab, a ring 1 px in colour's hue is drawn round the legend block, expanded by 4 px. Where the expansion would reach into the plot's data rect, it is cut back on that side, so the ring stays outside the data area. At rest the ring is gone. With no legend there is nothing to ring, and the colour cell's dot is the sign colour is set.

`draw_band`, `draw_below` and `draw_below_block` (`crates/brightfield-shell/src/legend.rs`) return the rect each block was drawn in, keyed by plot. `chart_item.rs` draws the ring from those rects when the tile's colour list is open. When a tile's plot sits in a `vconcat` with its legend below, the shelf band reads its cells from the first plot in the concat.

### 4. The reverse row's hand-off, held by a test

The window hands colour's settings the marks the hero drew (`of_plot_marked(.., &hero.marks)` in `hero_axis_settings`), which is what mutes reverse on a heatmap, raster or cell chart. No test holds it today, because every window test runs a hero that draws dots.

A window test over a hero that draws no dot asserts that the reverse row is muted with its reason. Replacing `of_plot_marked` with `of_plot_drawn` must turn that test red. Comments near colour's settings that describe the list as an axis's alone are corrected in the same change: in `shelf.rs`, `window.rs` and `tests/shelf_fallback.rs`.

## Order of work

1. The scale row, with pivot and steps by name, and the zero anchor's removal.
2. The domain row and the colour pin.
3. The legend row and the ring.
4. The reverse hand-off test and the stale comments.

The scale and legend rows both edit `shelf.rs` and `window.rs`, so they go in sequence. The domain row comes after the scale row because its auto reading and the pin's pivot both depend on the scale.

## Testing

- `brightfield-spec` edit tests for each key the rows write and remove, including a round trip through the chart-text writer and back through the parser.
- `brightfield-spec` reader tests: `colorDomain: Fixed` is a pin request and raises no warning; `Fixed` beside a categorical colour still warns.
- `brightfield-render` tests:
  - the colour pin captures on the first composition and holds the ends across a later composition over fewer rows;
  - on diverging it holds the pivot too;
  - a kind change leaves the fresh scale;
  - a non-negative ramp starts at the column's minimum.
- `brightfield-shell` shelf tests:
  - each row's value, auto/set state and mute reason;
  - the pivot's auto/set comparison against the profile;
  - a reset on the scale row removes the pivot or count;
  - a column change removes a set domain and pivot.
- Window tests: the field opens on Enter, previews and keeps; the legend row moves the legend; the ring's rect lies outside the data rect at right and below and is absent at rest; the reverse hand-off test above.
- kittest baselines for the scale row on each value, the pivot field open, the domain row, and the ring at right and below, in both themes. A baseline the change moves is redrawn with `UPDATE_SNAPSHOTS=1`, read, and named in the pull request with what moved. `kittest.toml`'s thresholds are not loosened.
- The gate is `cargo xtask ci`. Before a push, the whole suite runs with `cargo +1.95.0 test --locked --workspace --no-fail-fast -- --test-threads=1`, and its totals are quoted in the pull request. Two `remote_start` tests fail on a Mac on `main` and are named as such.

## Out of scope

- `colorDomain: Fixed` on a colour that paints category names.
- Anchoring a sampled plot's ramp to the unsampled column.
- Keeping a comment on the standalone legend's own lines across a move between right and below.
