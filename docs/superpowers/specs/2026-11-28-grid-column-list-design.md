# The grid's column list, chart rows and sortable fields: design

Status: draft for the owner's approval, 2026-11-28.

## Why

The grid's columns layout draws each column as a full chart row: a wide numbers block beside the column's histogram. Tried on California Housing beside two column-profiling tools that draw a column as one line and open it to a chart on a click, it was the only one of the three that can be brushed (a drag on any histogram narrows every other chart and the map, with the whole table drawn in grey behind the selection) and the least dense of the three. It also drew four of the file's nine columns as one bar on a linear axis, showed most of its numbers block empty, carried no quartiles, and had no sort anyone could find.

The owner's aim is density without giving up the brush, which is what Brightfield has that the other tools lack. He also holds that rows give a sense of the data that aggregates cannot, which is why the rows stay the grid's first layout and why the column list sits beside the rows whenever there is room. The grid's pictures are a view and stay ephemeral: nothing here writes a file or a step. A brush leads to a saved query by a later piece of work.

The work is on the path to promoting Meridian because the grid must be consistent: the same view, with the same behaviour, wherever a table is shown. That consistency matters more than any one date.

## Goals

1. **One switch, three layouts.** The grid pane's header switch becomes `rows · list · charts`. `rows` is the table's rows under the compact column band, as now. `list` and `charts` are the two modes of the columns view. The position is remembered per document as the layout is now. A fresh document opens on `rows`, and the first time the columns view is reached on a fresh document it opens on `list`.
2. **The list.** One line per column, about 30 points tall, carrying: the column's sequence index; its FineType glyph and name; a short histogram drawn as a real chart on the shared selection, so it can be brushed; its valid share; its median; its range. A click on a line opens that column's chart row (goal 3) beneath it, and a second click closes it. Several lines may be open at once.
3. **The chart row, fixed.** Each row carries the compact numbers block (goal 4) beside the column's histogram. Under the histogram are a value strip (one tick per value bucket, shaded by count) and a quartile ladder (a p5 to p95 line, a p25 to p75 bar, a median notch). The x axis follows one scale switch for the pane (goal 6). A row draws no x-axis title, since the numbers block names the column.
4. **The numbers block is a compact receipt.** The type once; then min, max, p5, p95, p25, p75, median, mean, sd, distinct count, and one valid share in place of the separate missing and null counts. Labels sit to the left of their values in two columns, in Inter with tabular figures. A range prints both ends at a resolution that fits the span: a timestamp range a year wide prints dates, not full timestamps, and the two ends never run together.
5. **Sortable fields.** The list and the chart rows each have a heading strip: index, name, valid, median, range in the list, and the same fields over the chart rows. A click on a heading sorts the rows by it, a second click reverses it, and sorting on the index restores the source order. One sort is shared by the list, the chart rows and the side list (goal 7), and it is remembered per document beside the layout.
6. **One scale switch for the pane.** `auto · linear · log · symlog` on the pane header when the layout is `list` or `charts`, in place of the per-histogram `linear · log · symlog` switches the transposed rows carry today. Under `auto`, a column whose skew passes 3 and whose values are all positive is binned on a log axis, and the row names it ("log x"); every other column is linear. `log` falls back to linear on a column with a zero or a negative, and says so on the row.
7. **The list beside the rows.** In the `rows` layout, when the grid pane is at least about 900 points wide, the list is drawn as a strip along the pane's trailing edge (about 340 points) beside the rows, on its own with no control to turn it on. Below that width the rows take the whole pane. A brush on the side list narrows the rows beside it, since the rows already read through the shared selection.
8. **FineType glyphs.** A column's glyph names its FineType family (the label's second level: `datetime.date`, `geography.coordinate`, `finance.currency`, `representation.boolean` and so on, about 45 families), drawn from the design system's icon set. When the FineType catalogue names an icon for a label (`x-finetype-icon`), that icon wins. A column with no FineType label falls back to a glyph for its storage type. The glyph is the same wherever the grid names a column: the list, the chart rows, the side list and the compact column band over the rows.
9. **Any table, the same.** The list, the chart rows, the side list and the brush behave the same for every table the grid shows: the opening table on the first screen, the same table in the spine's grid view, and any table a later step makes. The spine's grid view carries the same header switches as the first screen's grid pane.
10. **Columns that are not numbers.** A timestamp or date column shows its quartiles and median as dates or times. A text or categorical column shows its distinct count and its most frequent value where the median and range go, and its chart is the ranked bars the dashboard draws for it today. A column the chart generator gives no chart (an identifier, a column of one value, free text too wide to rank) still gets its line and its row, with its glyph, name, valid share and distinct count, an empty place where the chart would be, and the generator's reason on hover.

## Non-goals

- Turning a brush into a kept filter or a saved step. That is the next piece of work, already filed; the brush here narrows pictures and rows as it does today.
- Showing an example value in place of a list line's sparkline. Filed as its own follow-up.
- Thinning the count axis's labels on a short histogram. It stays its own issue; the chart row still draws a short count axis.
- Filling in `x-finetype-icon` in FineType's catalogue. Brightfield's own family table carries the glyphs until it does.
- A column search box, column pinning, or a "needs a look" order. The owner declined the named order in favour of the index and click-sortable fields.
- Writing any chart, spec or layout into the Protocol. The layout, the sort and the scale choice are per-machine view state, as the grid's layout is today.

## Anti-goals

- A brush that looks like an answer about the whole table when it was computed over part of it. A step nobody has run has only a 128-row preview and no table; its list and chart rows are drawn from the preview's profile, the brush is off, and the pane says why ("Run the step to brush its columns").
- Two pictures of one column that disagree: different bins, a different scale or a different filter between its list line and its chart row.
- A wide table whose every brush re-queries every column. A table of 286 columns costs, per brush, only what is on screen.
- A glyph set so large it cannot be learned at a glance, or a glyph that says a column is a number when it is not.

## Constraints

- The brush is the shared crossfilter selection the dashboard's tiles already use; nothing here adds a second brush mechanism.
- The grid only ever holds the rows in view; the column view must not change that.
- Statistics are over the whole table. The profile pass (`aggregate_source` in `brightfield-engine`) gains p5, p25, p95 and skewness (`quantile_cont` and `skewness`) in its one aggregate query, so a column costs no second scan. The valid share is the non-null share until a column's FineType label can judge validity, which is the share the validity band already draws.
- Icons come from the design system's one icon set (Tabler, per its icons guideline). Icons not yet in its desktop manifest are added there by a pull request to the design repository, and Brightfield takes them at its next bump of that dependency.
- Pixel baselines are drawn on the Mac Studio. The transposed layout's committed baselines (`dashboard_transposed_*`, `grid_in_ledger_columns_*`, `legend_ring_right_columns_*`) are redrawn on purpose and reviewed by the owner.
- This repository is public: no planning identifiers in code, comments, commits or pull request text.

## Approaches

Every brushable histogram today is a tile of the dashboard spec that `Dashboard::of` generates for the opening file: one tile per column on one `select: crossfilter` param. The spec is built in memory when the file opens and is never written to disk. Making every table brushable needs charts for tables the generator never sees.

- **A. One in-memory chart set per table, one chart per column drawn at two sizes. Chosen.** The generator learns to build its tiles for any table: the opening file's view as now, or a later step's table in the session. Each column's one tile is drawn bare and short in a list line, and in full (axis, value strip, quartile ladder) in a chart row. One query per column per brush, and only for the columns on screen. It keeps one brush across every picture, the hero map included, and makes "any table behaves the same" true by construction.
- **B. Two tiles per column**, one for the sparkline and one for the row. Simpler composition, but twice the queries per brush, and the two pictures can drift apart in bins or scale.
- **C. The grid draws its own sparklines and histograms** from its own bin queries, outside the chart system, with a brush that becomes a `where` on the grid's view. Easiest for very wide tables, but a second chart path, a brush that does not reach the hero map unless bridged, and a histogram that is no longer the chart a reader can open.

The owner asked whether A means a YAML file per grid. It does not: the chart set is plumbing held in memory, as the opening file's dashboard spec is today, and is dropped when the grid moves on.

## What the owner decided

- Both modes, list and chart row, behind a switch; neither alone. (26 November, after the comparison.)
- The numbers block as a compact receipt: p5 to p95, quartiles, one valid share. (26 November.)
- A sequence index and click-sortable fields, not an order picker: "The name 'needs a look' is unclear, and sometimes data isn't loaded from a 'file'. can we just add a sequence index then make the fields click sortable?" (26 November.)
- The columns view opens on the list. The first screen still opens on rows, because rows give a sense of the data aggregates cannot. (28 November.)
- The list sits beside the rows whenever the pane is wide enough, with no control to turn it on. (28 November.)
- Every grid behaves the same for every table: "It's not clear to me why we'd treat any table differently." (28 November.)
- The glyph is the FineType glyph, by family, falling back to the storage type: a two-way choice "misrepresents the diversity of the data, undermines our capability and falls short of 'spark joy'." (28 November.)
- Build "any table" first, so every grid is consistent from the first merge, rather than the opening table first for an earlier look: "this on the critical path for the promote milestone because the grid view needs to be consistent." (28 November.)
- The grid's pictures are ephemeral; the generated charts are not an artefact a user sees. Turning a brush into a saved query comes later, built before Meridian is promoted. (28 November.)

## Calls made for the owner

- The one switch reads `rows · list · charts`, rather than a second switch that appears only in the columns view.
- The side list appears at a pane width of about 900 points and is about 340 points wide; the builder tunes both to the rows still showing about five columns.
- A list line opens the same chart row that charts mode draws, so there is one full picture per column, not two.
- The chart rows get the same heading strip as the list, so the sort has something to click in both modes.
- One sort, shared and remembered per document.
- The scale switch keeps `symlog` beside `auto · linear · log`, for columns with zeros or negatives.
- Auto picks log past a skew of 3, on columns whose values are all positive.
- A text column's list line shows its distinct count and most frequent value in place of median and range.
- The glyph is chosen per FineType family (second level), not per top-level group, because the top level puts numbers, booleans and free text under one group.
- A step nobody has run gets its list without the brush. A step whose table exists but ran before an earlier step changed is brushed as the table it is, and the pane names its state in the spine's words, since a brush over that table is a true answer about that table.
- The side list is the list at its narrower width: its lines and its heading strip, sharing the pane's remembered sort and scale (`auto` until the scale is changed in `list` or `charts`). A click opens the chart row inline, with the receipt stacked above the histogram, as the grid already stacks them in a narrow pane.
- A ran step's table is read from the Protocol's last run (a Parquet snapshot of the run's table) and loaded with the opening file's FineType type source, so its columns are labelled as the opening table's are, the columns a step makes included. Today a later step's grid shows only its 128-row preview, so reading the run's table is part of this work, built on the separate piece that reads any node's rows from the last run.
- Sorting: the median and range headings group columns by kind (numbers, then dates and times, then text), sort within a kind by the median value or by the range's minimum, and put columns with no value last. Text columns sort by their most frequent value. What is remembered is the heading and its direction, not the rule.
- A document saved in today's `columns` layout opens on `charts`, which is what it draws now.
- The glyph task lands last in the build, after the design repository's icons are taken, so the baselines are redrawn once for the glyphs; until then the grid keeps today's glyphs.

## Left to the builder

- How the composer draws one tile at two sizes and looks (bare sparkline, full row) without a second query, and how tiles off screen are left unqueried.
- How the generator is pointed at a step's table in the session rather than at the opening file, and how its in-memory chart set is cached and dropped as the grid moves between tables.
- The family-to-icon table's exact icons, drawn from Tabler and agreed in the design repository's pull request.
- The heading strip's look, sort indicator and keyboard reach, within the design system's existing controls.
- The exact receipt layout within its narrower width, and the narrower width itself.
- Test names and which baselines are redrawn.
