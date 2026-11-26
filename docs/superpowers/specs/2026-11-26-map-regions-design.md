# A region of the map says what it is — design

## Intent

An analyst opens a plot of two numeric columns, whether the earthquake start's map, a generated point map or a scatter they picked, and each dense region of it is outlined and named from the rows inside it. Nobody types a query or writes a label. Zooming in recomputes the regions over what is on screen, so finer clusters and their own names appear the way they do on a map. Clicking a region selects exactly its rows, and every other plot on the page filters to them.

Why: a map that names its own clusters is the difference between a picture of where the rows are and a picture of what they are about, and the analyst does nothing to get it. It is part of what a newcomer's first hour with Brightfield should show: the earthquake start's cluster off Kamchatka is named from its own rows without anyone asking ("Russia" at the wide view, since that is the word its rows share; zooming in refines it to the phrases its sub-clusters share, such as "Kuril Islands").

## Rulings from the brainstorm (Hugh, 2026-11-26)

- **On by default, easy to turn off.** Regions and their names appear on their own on any two-column plot that qualifies, "as long as they can be easily toggled off."
- **A name is right when it is where the cluster is.** "Kamchatka seems right — it would only be wrong if it's noticeably dislocated." So placement is part of what the baseline pins, not only the text.
- **Numbers may name a region, cleanly or not at all.** "If there was a 'neat' way to show region differences by using ranges, e.g. '30k – 40k', then that may suffice for some types of analysis. Otherwise no name is better than clutter."
- **Layers come from zooming, recomputed over the view.** Of three ways to show coarse and fine regions (recompute on zoom; one tree computed once, with its level chosen by zoom; both layers drawn at once), Hugh chose recomputing on each settled zoom. A name may change as the view narrows: "precision refines the view at lower levels."
- **A single cloud is not the end of the taxonomy.** A plot whose whole-view density has one peak still has structure inside it, which zooming in reveals. This is why the layers come from zooming rather than from a fixed set of named regions.
- **Clicking a region selects its rows**, as a crossfilter selection the other plots on the page follow.
- **Names are the rows' own words, with no place-name list.** Measured on the earthquake rows, only 3 of the 1,851 quakes off Kamchatka mention "Kamchatka" in `place`, and 1,809 mention "Russia", so the wide view names that cluster "Russia". Mapping "Petropavlovsk-Kamchatsky" to "Kamchatka" would need a shipped gazetteer, a second source of names; the spec was approved without one, so the baseline pins what the rows say.

Constraints carried from the issue, unchanged:

- Labels come from the rows, not from a language model. finetype's column types choose which columns are worth describing and how.
- No embedding and no projection are needed: any two numeric columns, including longitude and latitude.
- Apple's `density_clustering` crate (part of Embedding Atlas, MIT) may be read for understanding and not copied. Build from the published algorithm, arXiv 2504.07285, "A Scalable Approach to Clustering Embedding Projections".
- The regions are computed, not authored, so only a committed baseline shows when a change to the density estimator reorganises them. The baseline must redden when either the regions or their labels change.

## Goals

1. Any plot with a `dot` or `hexbin` layer whose `x` and `y` are both quantitative columns draws region outlines and names, with no spec change. The earthquake start (`examples/starts/usgs-earthquakes-2025/panels/dashboard.yaml`) gets them with its file untouched.
2. A settled zoom or pan recomputes the regions over the visible extent at full grid resolution.
3. A name says what sets a region's rows apart from the rest of the rows in view, from a text, category or number column, or the region carries no name.
4. A per-plot toggle shows and hides regions; a spec can start them off with `regions: off`.
5. Clicking inside a region selects its rows; the other plots filter to them; clicking it again or pressing Escape clears the selection.
6. A committed structural table per baseline fixture records every region and label, at the default view and at one zoomed view, ahead of the pixel snapshot.

## Non-goals

- A geographic backdrop (coastlines, borders) behind the point map, and choropleths. A separate piece of work owns that.
- Declaring `panZoom` in a spec, or linking zooms across plots. Zoom already works per plot through the chart's own gestures and keys.
- Binned-axis zoom (a histogram's `x: { bin: … }` ignoring a zoomed extent) is a known, separate defect; regions read raw columns and do not depend on its fix.
- Exposing the algorithm's tuning (grid size, merge rule, the 0.1 truncation) as spec attributes.
- Hover on a region, and label text authored or edited by the analyst.
- Regions on line, area, bar or rule marks, or on a plot with a categorical axis.

## Anti-goals

Each of these is a failure even if the feature technically works.

- A label placed away from the cluster it names (over empty sea, or over the neighbouring cluster).
- A plot crowded with labels that say nothing (a name on every speck, or ranges that overlap the rest of the table).
- Regions that differ between two runs of the same file at the same view: the baseline would then pin noise.
- An interaction that becomes noticeably slower because regions are on: zoom settling, brushing and the first draw must stay at today's speed to the eye.
- A label from anything but the rows: no model call, no network.

## Approaches considered

For how the layers appear (the choice Hugh made):

1. **Recompute on zoom — chosen.** Each settled zoom reruns the regions over what is on screen. Fine structure appears at any depth, with one label size and the least clutter. A name can change as the view narrows, which is how maps behave.
2. **One merge tree, level chosen by zoom.** Computed once over the whole table, so names stay fixed across zooms, but fine structure is capped by a 100 × 100 grid over the whole table and turns blocky far in.
3. **Two layers drawn at once**, large regions over their sub-regions. Needs no zoom, but is the most crowded and needs two-size label collision handling.

For where it plugs in (the builder's calls, recorded here as mine):

- **A render-time overlay, not a synthesised mark.** Mark indices identify layers throughout the pipeline (lowering, the engine cache, crossfilter, hover), so injecting a mark into the spec would shift them. The committed-selection band (`ink_committed_selections`, `crates/brightfield-shell/src/pipeline.rs`) is drawn from session state after composition; regions are drawn the same way.
- **The region query rides the out-of-band path hover uses.** `Session::nearest_row` builds its SQL from `step_rows_sql(index, RowsAudience::Plot)`, so it reads exactly the rows the mark drew. Region queries are session methods on the same footing.

## The design

### Which plots qualify

A plot qualifies when one of its layers is `dot` or `hexbin`, with `x` and `y` naming plain quantitative columns (not a `bin:` channel, not an aggregate), and the plot has no band scale. When several layers qualify, the first in mark order supplies the columns and its rows are the ones regions describe. On the earthquake panel that is the grey `dot` layer over the whole table, not the brushed hexbin. **My call:** regions describe the rows the plot draws before any selection from another plot, so brushing elsewhere does not redraw them. They follow only the plot's own view (its zoom and pan).

### The pipeline, once per plot, after each draw that changes the view

1. **Grid query (SQL).** Count rows in a 100 × 100 equal-width grid over the plot's current x and y extent, not the whole table. Today the density and hexbin lowerers take bin edges from whole-table min/max subqueries, so a zoomed density shows a cropped full-table grid; the region grid takes its edges from the view extent instead (the displayed scales' domains, which are the whole table's when nothing is zoomed). Rows outside the extent are excluded. The builder puts this query builder in `brightfield-sql` beside `build_density_2d`, sharing its binning arithmetic.
2. **Smoothing (Rust, pure).** Run the existing separable Gaussian KDE (`crates/brightfield-render/src/kde.rs`) over the counts. Bandwidth is Silverman per axis computed from the binned counts as weights. A weighted 2D form (`silverman_2d_per_axis_weighted`, alongside the existing `silverman_1d_weighted`) replaces the expansion of every count back into samples, which today makes memory scale with row count.
3. **Regions (Rust, pure, new module beside `kde.rs`).** From arXiv 2504.07285:
   1. Each cell joins its highest-density neighbour (8-neighbourhood) by union-find. Ties break by lower cell index (row-major), so the result is identical run to run.
   2. Build the neighbour graph between the initial clusters, with the highest density on each shared boundary.
   3. Merge greedily, closest first, when a cluster's peak is close to the saddle it shares with a neighbour, by the paper's criterion.
   4. Truncate each cluster to cells at or above 0.1 × its own peak.
   5. Drop clusters smaller than a minimum cell count, which the builder sets so a lone speck is not a region.
   The output is a list of regions, each with its peak cell, its cells, and its share of the view's rows, in a deterministic order (by peak density, then cell index). The grid's cells map to data ranges through the grid edges.
4. **Outlines.** Trace each region's cell mask into polygons. `contour::contour_polylines` (marching squares over a 0/1 mask) is acceptable; the builder may write a cell-edge tracer if marching squares' diagonals read badly at this resolution.
5. **Label query (SQL, one statement per plot).** Assign each row in view to a region by its grid cell (the same binning arithmetic, joined against a `VALUES` list of `(bx, by, region)` triples), then compute each candidate column's per-region summary against the whole view in one scan grouped by region. Candidate columns and their tests are under *What a name says*.
6. **Placement and declutter.** Place each name at the centre of the largest square of region cells nearest the peak, so it sits inside the cluster and over its densest part. Place names in order of region share; drop a name whose box overlaps one already placed, or that does not fit inside the plot. A region whose name is dropped keeps its outline.
7. **Draw.** An overlay pass inside the plot's clip, after the layers and before the committed-selection band: a thin outline per region, then the names with knockout ink so they read over dense points, in the design system's ink and type. Use `brightfield_render::text::draw_text` and `measure_width` for text.

**Timing, my call.** The pipeline runs on a worker thread, never on the UI thread and never inside the pointer loop, so a settled zoom takes exactly as long as it does today and the regions arrive after it. The precedent is the typed-SQL preview (`crates/brightfield-shell/src/typed_sql.rs`): a read starts on its own thread, a newer request stops the one in flight and counts it as superseded, and the finished answer is taken off a channel on a later frame. Each result carries the view extent it was computed for, and a result whose extent is no longer the plot's is dropped. While a run is in flight, the previous regions stay drawn if the view is unchanged and are hidden if it moved. No row cap: on a file of millions of rows the names arrive later, and nothing else slows. A grid of 10,000 cells costs milliseconds in steps 2–4; the two queries are the cost.

### What a name says

Tried column by column, in the table's column order, skipping the two columns on the axes; the first column that passes its test names the region:

- **Text column** (finetype types it as free text or a place-like string, such as `place`): the token or two-token phrase most typical of the region against the rest of the view, by class-based TF-IDF (term frequency in the region × log of inverse frequency across regions). It must appear in at least half the region's rows. Example: "Russia" and "Tonga" at the wide view of the earthquake map.
- **Category column** (low distinct count, or typed as a category or code): the most common value, when at least 60% of the region's rows hold it and its share in the region is at least twice its share in the view. Example: "Gentoo".
- **Number column** (quantitative, not an identifier or coordinate): the region's interquartile range, when it does not overlap the interquartile range of the rest of the view. Written as `<column title> <lo>–<hi>` in the column's own format (from its finetype type, as the axes already do). Example: `median house value $300k–$450k`.
- **Skipped:** identifiers (finetype types them as unique codes, such as an LEI or an event id, or distinct count near row count), dates and timestamps (a time range rarely names a place on the plot), and the axis columns.

The builder sets the exact thresholds and records them in the module; the figures above are the starting point. **Hugh's ruling:** if no column passes, the region gets an outline and no name.

**Column roles without FineType, my call.** The gate's tests run with no FineType bundle beside them, so every column arrives untyped there (`crates/brightfield-shell/tests/dashboard_baseline.rs` says so and asserts it). The rules above therefore decide a column's role from its storage type and its measured distinct count first: a string column with few distinct values is a category, one with many is text, one whose distinct count is near the row count is an identifier and skipped; a numeric column is a number unless its distinct count is near the row count. FineType's type, when present, only refines that: it marks identifiers and codes the counts miss, and dates. So the baselines pin the untyped path, and on the fixtures it gives the same names the typed app gives. An `#[ignore]`d test beside `crates/brightfield-engine/tests/finetype_bundle.rs`, run with `BRIGHTFIELD_FINETYPE_BUNDLE`, checks that the typed path names the fixtures' regions the same.

### The toggle

A switch at the plot's top-right, drawn with the existing tile switches (`draw_scale_switch`, `draw_normalise_switch` in `crates/brightfield-shell/src/chart_item.rs`), shown only on plots that qualify. Turning it off writes `regions: off` on the plot through `set_plot_attribute`, as the scale switch does, so it is saved with the chart. Turning it on removes the attribute. `regions: off` is a Brightfield extension attribute on a plot. It must pass spec validation, and is written only when off, so a spec that never touches it stays plain Mosaic.

### Selecting a region

A click (press and release without a drag) inside a region's outline selects it; a drag still draws the rectangle brush.

**Which selection it publishes to, my call.** A region click publishes to the plot's own `intervalXY` selection, replacing the brush's clause, so the plots already filtering by that selection follow it with no new parameter. Every generated two-column tile and the earthquake panel have one. A plot with no `intervalXY` (none, or a one-axis `intervalX` / `intervalY`) draws its regions and names but offers no click, because nothing on the page listens for a two-column clause and inventing a parameter would rewrite the analyst's spec. The hit-test is point-in-region over the grid cells, which avoids polygon tests. The selection is a predicate over the plot's two columns: the OR of each cell's x and y interval, built in cell order so the same region always produces the same clause. Clicking the selected region again clears it, as a repeated toggle already does, and Escape clears it. The selected region's outline is drawn emphasised.

**My call:** add a `Predicate::Cells` variant (columns, grid edges, cell list) that renders to that SQL, rather than an opaque `Expr`. That lets the committed-selection band, the clause's readable form, and the equality check for toggling-off all read it. The pre-aggregation cube declines it and the query runs directly. That is correct and slower, which is acceptable for a click; record it in `docs/interaction-speed.md` beside the brush-while-zoomed gap.

When the view zooms, a held region selection keeps its cells as they were selected; the regions redrawn at the new view are new regions.

### Baselines

- **The region table.** A test per fixture writes, at the default view and at one fixed zoomed extent, one line per region: region index, peak cell and its data position, cell count, share of rows, bounding box in data units, label text, label anchor in data units. It compares the lines with a committed text file and fails before any pixel snapshot, as `crates/brightfield-shell/tests/dashboard_baseline.rs` checks its choices table first, so `UPDATE_SNAPSHOTS=1` cannot record a picture whose regions have moved.
- **Fixtures.** The earthquake start (the first-hour map; place names) and Palmer Penguins as a scatter of bill length against bill depth (category names), plus one numeric-only table for range names. The California Housing file the dashboard baseline already reads would serve.
- **Mutation.** A test shows the table reddens when the bandwidth rule changes, and when the label rule changes.
- **Existing snapshots.** Every committed snapshot with a qualifying plot changes once. The builder redraws them on the Mac Studio in the same pull request, after the region tables are committed, and lists them in the pull request. The research pass found them in `dashboard_baseline.rs`, `point_map_baseline.rs`, the legend baselines, `legend_short_chart.rs`, `axis_vocabulary_rows_past.rs` and `surfaces.rs`; the builder confirms the list by running the suite. Render tests that count ink on a plot are updated, or pass `regions: off` where the test is about something else.

## Left to the builder

- The merge criterion's exact constant from the paper, the minimum region size, and the label thresholds, each recorded in the module with its reason.
- Whether marching squares or a cell-edge tracer draws the outlines.
- How the overlay's state is carried between the engine session and the shell (a per-plot cache keyed by the plot and its view extent is the expected shape).
- The exact SQL for the TF-IDF tokens in DuckDB (`string_split` and `regexp_split_to_array` are both available), and tokenising rules (lower-casing, dropping stop words and numbers such as "45 km SW of").
- The outline and label styling within the design system's ink, weight and type tokens.
