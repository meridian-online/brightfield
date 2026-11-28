# Map regions

A plot of two numeric columns, such as the earthquake start's map, a generated point map or a scatter the explorer picked, outlines each of its dense regions and names it from the rows inside it. Nobody types a query or writes a label. Zooming or panning finds the regions again over what is on screen, so finer clusters and their own names appear the way they do on a map. Clicking a region selects its rows, and the plots that follow the plot's brush filter to them.

A map that names its own clusters shows what the rows are about, not only where they are, and the explorer does nothing to get it. This note says how the regions are found, named, drawn and selected, and why each choice was made. The code is `crates/brightfield-render/src/regions.rs` (finding the regions, outlines, anchors), `crates/brightfield-sql/src/regions.rs` (the two queries), `crates/brightfield-shell/src/region_names.rs` (the naming rules) and `crates/brightfield-shell/src/regions.rs` (the worker, the overlay and the click).

## Which plots have regions

A plot has regions when one of its layers is a `dot` or a `hexbin` whose `x` and `y` each name a plain column (not a `bin:` channel, an aggregate or an expression), and the plot draws both axes on linear scales: no `xScale` or `yScale` other than `linear`, and no projection other than equirectangular or identity, which leave longitude and latitude as they are. The grid is equal-width in data units, and only a linear axis draws equal data widths as equal screen widths. When several layers qualify, the first in mark order supplies the columns and the rows. On the earthquake panel that is the grey `dot` layer over the whole table, not the brushed hexbin.

The rows are the layer's own, with the spec's parameters as they stand and no selection from any other plot. Brushing elsewhere on the page does not move the regions; they follow only the plot's own view and the parameters that filter its layer. A layer over inline rows rather than a `from:` source cannot be read off the UI thread, so its plot has no regions and no switch.

Regions are on by default and easy to turn off. Line, area, bar and rule marks, and plots with a categorical axis, never have them.

## The pipeline

Each time a plot settles at a new view, a run finds its regions in these steps.

1. **Grid.** One SQL query counts the layer's rows in a 100 × 100 equal-width grid over the plot's displayed extent, the domains of its drawn scales, which are the whole table's when nothing is zoomed. Rows outside the extent are left out. The binning arithmetic is the density lowerer's own, so a region's cells line up with the plot's other density marks.
2. **Too few rows.** If fewer than 50 rows fall in the grid, the run stops there and the plot draws no regions.
3. **KDE.** The counts are smoothed by the separable Gaussian KDE in `crates/brightfield-render/src/kde.rs`. The bandwidth on each axis is Silverman's rule, computed from the occupied cells' centres weighted by their counts, so memory does not grow with the number of rows. An axis with no spread (every row in one column of cells) is smoothed over one cell instead.
4. **Hill-climb.** Each cell points to its highest neighbour in the 8-neighbourhood, and the cells that climb to the same local maximum form an initial cluster. This is the density-map clustering of "A Scalable Approach to Clustering Embedding Projections" (arXiv 2504.07285), built from the paper rather than from any existing implementation.
5. **Merge.** Neighbouring clusters are joined in a graph whose edges carry the saddle, the highest density on their shared boundary. Clusters are merged greedily, closest first, while the saddle is at least 0.8 of the lower of the two peaks.
6. **Truncation.** Each cluster is cut to the cells at or above a tenth of its own peak, and keeps only the connected piece that holds the peak. A merged cluster joined over a saddle below the cut would otherwise fall into pieces; the cut-off pieces belong to no region.
7. **Specks.** A cluster left with fewer than 6 cells is dropped. A region holding less than 1% of the view's density mass is also dropped, neither drawn nor named.
8. **Outlines.** Each region's cells are traced into closed rings along cell edges, so outlines are axis-aligned. Outer rings come first, then holes, each starting at a fixed corner and running a fixed way. Two cells touching only at a corner stay in one ring that touches itself there.
9. **Anchor.** Each name is written at the centre of the largest square block of the region's own cells nearest the peak.
10. **Naming.** A second SQL query assigns each row in view to a region by its grid cell and summarises every candidate column per region and for the rest of the view in one scan. The summary becomes the names, under the rules below.
11. **Placement.** Names are placed in order of region share. A name whose box overlaps one already placed, or does not fit inside the plot area, is left out, and its region keeps its outline.
12. **Draw.** The overlay is drawn inside the plot's clip, after the layers and before the committed-selection band: a hairline outline per region, then the names with knockout ink so they read over dense points, in the plot's own ink tokens.

The output is the same on every run of the same file at the same view. Every tie breaks by the lower row-major cell index, regions are ordered by peak density and then by peak cell, and nothing iterates a hash map. A committed baseline pins the regions, so any run-to-run variation would make it pin noise.

## Thresholds

| Threshold | Value | Why |
|---|---|---|
| Grid size | 100 × 100 cells over the view | Fine enough for clusters a reader can see at any zoom, small enough that the steps after the grid query cost milliseconds. The two queries are the real cost. |
| Fewest rows in view | 50 | A density estimate over a handful of points spreads each one over a wide patch, so the regions it finds tile the whole view and say nothing a reader cannot see by looking at the dots. Fifty is a small table's worth, where a cluster can start to stand out. |
| Bandwidth | Silverman's rule per axis, from the binned counts as weights | A standard, parameter-free rule, and reading it from the counts keeps memory flat however many rows the view holds. |
| Merge ratio | saddle / lower peak at least 0.8 | The paper merges a cluster whose peak stands barely above its boundary with a neighbour, but states the rule in words with no constant. Measuring it as a density ratio, not a distance in cells, follows the paper's reason (a peak that does not stand out) and does not change with grid resolution. At 0.8 a peak at most a quarter above its saddle merges, while two equal Gaussians three standard deviations apart, which read as two lobes, have a ratio near 0.64 and stay apart. |
| Truncation | 0.1 of each cluster's own peak | The paper's value. It trims each cluster to its body and drops the faint skirt the smoothing spreads around it. |
| Smallest region | 6 cells | About a 2 × 3 patch, under a thousandth of the view. Anything smaller is too small to read as an outline or hold a name, and is the size of the bumps a density estimate makes around a few stray rows. |
| Smallest drawn share | 1% of the view's density mass | The smoothing spreads every row a little, so the faint tails between clusters can come out as regions holding almost nothing: an outline around empty-looking space, with a name. A hundredth keeps every cluster a reader can see at the current zoom; a smaller one is found by zooming in, where it holds more of the view. |
| Text share | a term in at least 50% of the region's rows | Past half, a word describes the region rather than part of it. A name true of a minority of the outline would mislead. |
| Category share | the top value in at least 60% of the region's rows | Higher than the text figure because a category has no runner-up words to soften it: the name is the value, so most of the region should be that value. |
| Category lift | at least 2 × the value's share of the whole view | So a value common everywhere, such as a dataset's majority class, does not name every region the same. |
| Number presence | at least 50% of the region's rows hold a number | Quartiles skip nulls, so without this a range read off a few rows could stand for a region whose other rows have no value. |
| Identifier | distinct count at least 95% of non-null rows | The distinct count is an estimate that reads a few percent either side, so the cut is not 1.0. A column this close to unique says one thing per row, which names no region of many. |
| Rest-of-view histogram | 1024 bins per region per number column | The rest of the view's quartiles come from a per-region histogram (the view minus the region) rather than a join of every region with every row, so the summary grows with regions times bins rather than regions times rows. The quartiles are then approximate to 1/1024 of the view's range, finer than the two-significant-figure rounding a printed range uses. |

These are named constants in the modules above, with the same reasons beside them. The naming thresholds are also carried as a `NamingRules` value and the job carries a bandwidth scale (1.0 in use), so tests can change one and show the baseline tables redden. Constants alone cannot be changed in a test. None of these is a spec attribute; the tuning is not exposed to authors.

## Naming

Names are the rows' own words. They never come from a language model, a network call or a shipped list of place names. Columns are tried in the table's order, skipping the plot's two axis columns, and the first column that passes its test names the region. If no column passes, the region has an outline and no name, since no name is better than clutter.

A column's role comes from its storage type and its measured distinct count first, so a session without FineType types columns the way one with FineType does on the same data. The gate's tests run with no FineType bundle, so the baselines pin this untyped path.

- **Text**: a string column with more distinct values than the column header draws one bar each for. Its name is the word or two-word phrase most typical of the region by class-based TF-IDF, with each region and the rest of the view as the classes: the term's share of the region's rows times `ln(1 + A / f)`, where `A` is the average rows per class and `f` the rows in the whole view that hold the term. A term must clear the 50% share before it is scored. Ties go to the longer phrase, then to the term that sorts first.
- **Category**: a string column with few distinct values, or an enum. Its name is the region's most common value, when it clears the 60% share and the 2 × lift.
- **Number**: an integer, float or decimal column. Its name is `<column title> <lo>–<hi>`, the region's interquartile range, when it does not overlap or touch the interquartile range of the rest of the view and enough rows hold a number. Each end is rounded to two significant figures of the larger one, so a range reads like an axis's ticks, and printed in the column's own format as its axis prints it (money as money, percentages as percentages). The title is the column's axis title where the spec gives one, otherwise its name. A range whose two ends print the same is no range and gives no name.
- **Skipped**: identifiers (an integer or string column whose distinct count is near its row count), booleans, dates, times and timestamps, lists, structs, blobs, and the axis columns.

A float or decimal column is always a number, even when nearly every value is distinct: measurements such as a house value are naturally near-unique, and identifiers are stored as integers or text. The cost is that an identifier stored as a float would be range-named; FineType's identifier type still skips it when FineType is present.

Booleans are never candidates, because a region named `true` tells a reader nothing.

When FineType has typed a column, its label only refines this: a date label, a coordinate, or a label for something that identifies a thing or holds a document takes the column out (free text stays in). The table's profiles from the open are carried across a rebuild of the page, so names do not change, or fall back to the untyped path, because a tile switch was thrown. Profiles are cut to the columns the plot's rows actually have before the naming query runs, since a rebuild can leave the rows without a column the open's profile names.

Text is tokenised in SQL. A value is cut into runs at anything that is not a letter, an apostrophe, a hyphen or a space, so punctuation and digits end a run, and each run is split into words at spaces. A word keeps an inner apostrophe or hyphen (`Kuril'sk`, `Severo-Kuril'sk`). Words under three letters and a short list of stop words are dropped: joining English words and the distance and compass words of a place written relative to a town, so `45 km SW of Severo-Kuril'sk, Russia` is about Severo-Kuril'sk and Russia. Whole compass words such as `south` are kept, since they are often part of a name. Each kept word is a term, and so is each pair of kept words next to each other with no gap, so a phrase is always one a row actually wrote. Terms are compared lower-cased.

Using only the rows' words has a visible cost. Of the 1,851 earthquakes off Kamchatka, 3 mention "Kamchatka" in `place` and 1,809 mention "Russia", so the wide view names that cluster "Russia". Calling it "Kamchatka" would need a shipped gazetteer, a second source of names, and none ships.

A name is right when it sits where its cluster is. A name placed over empty sea or over a neighbouring cluster is wrong even if its text is right, which is why the anchor is a block of the region's own cells near its peak rather than the centre of its bounding box or its mass, which can fall in the hollow of a crescent.

## The worker

Regions never run on the UI thread or inside the pointer loop, so a settled zoom takes exactly as long as it did before regions existed, and the regions arrive after it. The model is the typed-SQL preview's.

- **One run per plot.** Each run is a thread with its own DuckDB reader, a clone of the session's connection. A run is keyed by the layer, its two columns, its rows SQL and its extent. A request for what is already answered, or already being answered, starts nothing; a request for anything else stops the plot's run in flight and counts it as superseded, and its answer never lands.
- **Cancellation.** Stopping a run sets its flag and interrupts its reader. The run reads the flag before each query, because DuckDB clears an interrupt when a statement starts, so one sent between queries would be lost. When the worker is dropped (a rebuild, an open, a quit) it re-sends the interrupt every millisecond until each run ends, for at most 250 ms, then leaves any run still going to finish alone. Joining the threads matters: a thread still running a query when the process exits reads DuckDB's state as it is torn down and crashes the process.
- **Answers.** A finished run sends its answer on a channel and asks the window for a frame; the frame takes the answer and redraws only the overlay from the batches already held. A run that fails leaves no answer. A failed naming query leaves the regions unnamed, not undrawn: the outlines are the picture.
- **Stale answers.** An answer carries the extent and the rows SQL it was found over, and is drawn only while both are the plot's. After a zoom or pan, or a parameter that changes the layer's rows, the old regions are hidden until the new ones land, rather than drawn over a view they were not found for.
- **The hold while moving.** No run starts while the plot is moving (a zoom or pan in progress) or while a slider or other input widget is being dragged. A run per pointer step would be work thrown away. A slider changes the layer's rows, so the regions hide during the drag, since regions found over other rows would misname what is drawn, and the first frame after the button comes up asks once, for where the drag left the plot. The cost is a visible blink during a drag; if that reads badly, the alternative is to keep the last answer drawn, dimmed, until the new one lands.
- **No row cap.** On a file of millions of rows the names arrive later, and nothing else slows.

The region reader clones the session's connection, and a clone cannot see a connection's temporary tables. So a source read once into memory (`materialise_source`) is made as a plain table, not a temporary one. The session's database is in memory, so its lifetime is the same and nothing reaches disk. The one risk is a clash with a user table of the same generated name.

## Selecting a region

A click inside a region's outline selects it. A click is a press and release that moves less than 3 pixels on both axes; anything longer is a drag and still draws the rectangle brush. A press on a committed rectangle leaves the rectangle alone, as before.

The click publishes to the plot's own `intervalXY` selection, in place of the brush's clause, so every plot already following that selection follows the region with no new parameter. A plot with no `intervalXY` (none, or a one-axis `intervalX` or `intervalY`) draws its regions and names but offers no click, because nothing on the page listens for a two-column clause, and inventing a parameter would rewrite the explorer's spec. A plot whose brush names different columns from its regions layer also offers no click, since cells over one pair of columns say nothing about another.

The hit-test is point-in-cell on the grid, not a polygon test, and reads only the answer drawn now, so a click made while a zoom's regions are still being found is a plain click. The clause is a `Predicate::Cells` (the two columns, the grid's extent and the region's cell list) that renders to SQL as a range test bounding the scan to the grid's extent, ANDed with a membership test of each row's cell index (`y bin × 100 + x bin`, from the same arithmetic as the grid query) against the region's sorted cell list, so the same region always gives the same clause. A typed variant rather than an opaque expression lets the committed-selection band, the readable form of the clause and the equality check for clearing all read it. The selection readout writes it as its columns and its cell count over the grid (for example `longitude, latitude in 212 of 100×100 cells`), not every cell index, which would tell a reader nothing; the SQL still carries every cell.

The pre-aggregation cube declines a `Cells` clause, so the plots that follow it query the source directly. That is correct and slower, which is acceptable for a click: it is one query, not a stream of them, and its answer is exact. `docs/interaction-speed.md` records it.

Clicking the selected region again clears the selection, as a repeated point click does, and Escape clears it. The selected region's outline is drawn heavier (2.5 pixels against a 1-pixel hairline), still a line rather than a fill so its points stay visible.

When the view zooms, a held selection keeps its cells as they were selected, over the extent they were found on. The regions drawn at the new view are new regions, none of them held, so none is drawn heavier. Turning regions off, changing an axis row or undoing that change rebuilds the page and drops a held region selection with it, so the document never holds a gesture the session no longer has.

## `regions: off`

A plot that should start with its regions off says `regions: off`. It is a Brightfield extension to Mosaic, and the switch writes it only when turning regions off and removes it when turning them back on, so a spec that never touches regions stays plain Mosaic. `off` is the only value read: any other value, `regions: on` included, draws a warning, because the attribute is only ever written as `off` and anything else would be a value nothing reads.

The attribute is plot-only. Under `plotDefaults:` it draws a warning (``regions: off` under `plotDefaults:` is ignored``) and is not passed down to the plots. An inherited key cannot be removed by a plot's switch, and saving a chart is all or nothing, so a switch that tried to turn on a plot whose `off` came from `plotDefaults:` would queue an edit that could not be placed, and every other edit would then fail to save. The cost is that a chart cannot turn regions off for all its plots in one line.

## The switch

Each plot that can have regions carries a two-state switch, `regions · off`, in the scale switch's chrome, with a hover naming the plot's two columns so two switches on one page read differently. Turning it off writes `regions: off` on the plot through `set_plot_attribute`, as the scale switch does, so it is saved with the chart.

The switch sits at the tile's bottom-right, in the row the x axis's title is written in, below the tick labels, right-aligned and clear of the title. It does not sit at the top-right, in the tile's head, because authored plots write their titles there and a long title runs the full width, so a switch there was painted over it. It does not sit in the corner of the data area either, because the first screen's map keeps its data area free of chrome, by an earlier rule its tests hold. The right end of the axis-title row was empty on every fixture. Where that row has no room clear of the title, or a count of rows past the end of the x axis is written at the right of it, the plot draws no switch. The cost is that the switch is less prominent than a top-right one would be.

The switch is offered by the plot's spec and scales, not by its row count, so a plot under the 50-row floor still carries it. The count changes with every zoom, and a switch that came and went with the view would be worse than one that does nothing visible on a tiny table.

## Design decisions

- **On by default.** Regions appear on every qualifying plot with no spec change, as long as they are easy to turn off, which the switch makes them.
- **Layers come from zooming, recomputed over the view.** Three ways to show coarse and fine regions were weighed: recompute on each settled zoom, compute one merge tree over the whole table and pick a level by zoom, or draw both layers at once. Recomputing shows fine structure at any depth with one label size and the least clutter. A name may change as the view narrows, which is how maps behave: precision refines the view at lower levels. One merge tree would keep names fixed, but a 100 × 100 grid over the whole table turns blocky far in. Two layers at once would be the most crowded and need collision handling across two label sizes.
- **A single cloud still has structure.** A plot whose whole view has one peak still has clusters inside it that zooming reveals, which is why layers come from zooming rather than from a fixed set of named regions.
- **Numbers name a region cleanly or not at all.** A neat range such as `$300k–$450k` can name a region, but only when it stands apart from the rest of the view; otherwise no name is better than clutter.
- **A render-time overlay, not a synthesised mark.** Mark indices identify layers throughout the pipeline (lowering, the engine cache, crossfilter, hover), so a mark injected into the spec would shift them. The regions are drawn from the worker's state after composition, as the committed-selection band is drawn from the session.
- **Regions describe the plot's own rows**, before any selection from another plot, so brushing elsewhere does not redraw them.
- **Small tables get no regions.** Below 50 rows in view, a density estimate partitions the space rather than finding clusters.
- **The baseline comes before the picture.** The regions are computed, not authored, so a change to the bandwidth, the merge rule or a naming threshold can reorganise them with no spec changing. `crates/brightfield-shell/tests/region_baseline.rs` writes one line per region (peak, cells, share, bounding box, name, anchor) for the earthquake start at its wide view and at one zoomed view, Palmer Penguins as bill depth against bill length, and the California Housing point map, and compares them with the files under `crates/brightfield-shell/tests/regions/`. Nothing orders it before the snapshot targets, and the gate leaves the shell's targets to the nightly, so whoever redraws a snapshot of these plots runs and reads the region tables first: a run that includes them names what moved, where a picture alone would only show that something did. Regenerate the tables with `UPDATE_REGION_TABLES=1 cargo test -p brightfield-shell --test region_baseline` and read every line of the diff. Two tests show the tables redden when the bandwidth changes and when a naming threshold changes. No fixture pins a range name; range wording is pinned by the naming module's unit tests. An ignored test, `region_names_match_with_finetype`, run with `BRIGHTFIELD_FINETYPE_BUNDLE`, checks that the typed path names the fixtures' regions the same.

## Where the results are rough

The wide view of the earthquake map is coarse. Silverman's bandwidth assumes one roughly normal spread, and a global map of earthquakes is many narrow arcs spread over the whole planet, so the bandwidth comes out wide and the regions come out large. At the wide view the largest region, named "Russia", spans longitude 15 to 184, and its name sits near the Kuril side of the cluster rather than over Kamchatka itself. The second-largest, around Indonesia and the western Pacific, holds about 30% of the view and has no name, because no column passes its test there. The Aleutian region is named "moment (Mw)", a value of the `magnitude_scale` category column, rather than a place: that column comes before `place` in the table, and at least 60% of the region's quakes were measured on that scale, at least twice its share across the view. These stand as the algorithm gives them. Zooming in gives the fine names: over longitude 140 to 170 and latitude 35 to 60 the regions are "Russia" (the Kamchatka cluster), "Japan", "Shikotan" and "Kuril'sk". A bandwidth narrower than Silverman's for multimodal spreads is the lever if the wide view should read finer.

Other rough edges:

- The California Housing point map finds two regions, Los Angeles and the Bay Area, and names neither: no text column passes, and no number's interquartile range stands clear of the rest of the view's.
- A text term held by half of every region's rows would name them all alike; there is no lift test for text as there is for categories. The baselines would show it.
- On the earthquake panel, the hexbin layer pins the map's frame through a zoom, an existing bug separate from regions. The zoomed baseline therefore takes its map without the hexbin layer, as the overlay tests do; regions follow the drawn extent either way.
- A genuine cluster under 1% of the view's mass is unmarked until zoomed into.
- A plot whose brush and regions layer read different columns has regions but no click.
