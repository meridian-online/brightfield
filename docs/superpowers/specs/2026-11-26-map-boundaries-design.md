# Countries behind a point map, and a choropleth from a country column — design

## Intent

A point map of longitude and latitude draws over the world's countries in a neutral grey, so the earthquake start reads as places and not as dots in space. An analyst whose table has a column of countries, as names or ISO codes, picks a choropleth in the Chart editor and gets a filled world map: every country drawn, the ones with data coloured from light to dark, the ones without in grey, the way PostHog draws its visitors-by-country map. The map and a ranked bar of the same column sit side by side and filter each other: the map gives the familiar frame, the bar gives the exact figures.

Why: alpha feedback on the California Housing file said a lat/long map shows "dots in space", and asked whether Brightfield does choropleths and what an analyst has to supply. Both hinge on one question, where the boundaries come from, and both are what the getting-started maps need.

## Rulings from the brainstorm (Hugh, 2026-11-26)

- **Boundaries are a dataset Meridian prepares, not a file the analyst brings.** arcform supports boundary data, and boundary data must be useful whatever the reader's locale. Hugh named the Mosaic examples' world-atlas and us-atlas files as too US-centric to be the answer.
- **Bundle a small canonical set, host the rest.** "If there were an open-source, canonical, broadly accepted set of boundaries with at a low size we should bundle them. Beyond that, boundary data for different resolutions and projections is valuable to host — slow moving data that's widely in demand." The hosted sets are separate work in open-analytics, not part of this design.
- **States and provinces wait for the hosted sets.** This design ships countries only.
- **FineType finds the column, the Chart editor offers the map, Relate shows the join.** "Brightfield would make this join on 'name' easy to spot firstly by identifying the type with Finetype, presenting the choropleth option in the Chart editor or perhaps in more detail with 'relate'."
- **The choropleth and the ranked bar are complementary.** Hugh's reference is PostHog's country map above its table of country, name and total: "The choropleth is a familiar frame to provide the context."
- **The defaults are configurable through Mosaic's own attributes.** Every default below is an ordinary attribute in the generated spec, which the analyst edits.
- **The join is a step in the Protocol, keyed by country code, with the shapes attached after the aggregate.** Hugh chose this over a join inside the chart's SQL and over a country-keyed `geo` mark, once it was measured not to slow a brush (below).

## Goals

1. Brightfield ships Natural Earth's 1:110m countries, published by an arcform Protocol and bundled like the earthquake start's Parquet, readable offline in both editions with or without the spatial extension.
2. The generated point map (`point_map` in `crates/brightfield-shell/src/chart_kinds.rs`) and the earthquake start's map draw the countries in grey behind their points.
3. The Chart editor offers a **choropleth** for a table with a column FineType labels `geography.location.country` or `geography.location.country_code`, or, untyped, a column whose name says country (`country`, `country_code`, `iso2`, `iso3`, `iso_a3` and the like), the same two passes `coordinate_pair` makes for a point map.
4. Choosing it adds the countries table and a join step to the Protocol, and a `geo` plot that colours each country by a count or a measure of the rows matched to it.
5. Rows whose country does not match are counted, and their values listed beside the map ("312 rows not drawn: EIRE 142, Unspecified 98, European Community 72"). They are never dropped silently.
6. The choropleth takes part in the page's crossfilter: it narrows when another plot is brushed, and clicking a country selects that country's rows for the others. Hovering a country shows its name and value.
7. Brightfield draws `colorScale: log` and `colorScale: quantile` (with `colorN`), which it parses today but draws as linear.
8. The Online Retail II start gains a choropleth of orders by country beside its "Orders, by country" bar.

## Non-goals

- States, provinces and finer divisions, and finer country boundaries: the hosted boundary sets in open-analytics.
- Natural Earth's point-of-view country files for disputed borders: hosted with the rest. The bundle carries Natural Earth's default, which draws borders as they are held on the ground.
- Line geometry (coastlines, rivers). The countries' polygons are the backdrop, so the `geo` mark's Polygon-only drawing is enough.
- An inset composite for Alaska and Hawaii (`albers-usa`), and the `sphere` and `graticule` marks.
- A backdrop on a scatter of two columns that are not a coordinate pair.

## Anti-goals

- A choropleth that leaves countries out without saying so: a country missing from the analyst's rows is drawn grey, and an analyst's row that matched no country is counted on the page.
- A backdrop that competes with the data: it is quieter than the lightest point or fill, and draws under every layer.
- A backdrop or choropleth that needs the network, or the spatial extension, to draw.
- A brush on another plot that feels slower because a map is on the page.
- A spec that only Brightfield can read: the plot is a Mosaic `geo` mark over a table, and the join is SQL in the Protocol.

## Constraints

- Natural Earth is in the public domain ("Crediting the authors is unnecessary"). GADM is excluded (its licence bars commercial use), and so is OpenStreetMap's boundary data (ODbL share-alike would bind what is built on it).
- Measured 2026-11-26, zipped shapefiles from naciscdn.org: 1:110m countries 214,976 bytes, 1:110m land 69,700 bytes. Land is not bundled: the countries draw the backdrop as well.
- The download edition fetches the spatial extension the first time it is needed, so offline it may have none. The `geo` mark passes `VARCHAR` GeoJSON through without `ST_AsGeoJSON` (`crates/brightfield-sql/src/lower.rs`), so geometry stored as GeoJSON text draws in every edition. The Store edition builds spatial and GEOS in, by Hugh's ruling of the same day, so a Protocol that reads an analyst's own shapefile through `ST_Read` runs there too.
- FineType has a type for two-letter ISO codes and one for country names (locale-specific), and none for three-letter codes. That type is filed in FineType; until it lands, a three-letter column is found by its name.
- A `geo` mark today is static: it ignores `highlight`, publishes no selection, and its layer's `filterBy:` is untested (`GeoRenderer`'s doc, `crates/brightfield-render/src/mark.rs`). Goal 6 is new work in the renderer, the lowerer and the interactor.
- A `geo` mark on an unprojected plot draws the plate carrée and aspect-fits its own bbox. The earthquake panel is an unprojected cartesian plot with `xDomain: Fixed` and a `hexbin` layer. A `geo` layer added behind it must draw in that plot's x and y scales (longitude on x, latitude on y), not refit them.

## Approaches considered

Where the join between the analyst's table and the countries lives:

1. **A step in the Protocol — chosen.** The Chart editor adds the countries as a source and a join step; the chart is a plain `geo` mark. Relate shows the join and its match rate, the unmatched list is a query on the step, and other charts reuse the joined table. It costs two Protocol nodes per first choropleth over a table.
2. **Inside the chart**, as Mosaic's own unemployment example does with a named `combined` table in the spec's `data:`. Self-contained, but Relate cannot see the join, the editor cannot edit SQL embedded in a spec, and the unmatched list needs a query of its own.
3. **A country-keyed `geo` mark** (`geo` with `country: <column>`) that looks the shapes up in the renderer. Least for the author to write, but the spec stops being Mosaic. Dropped.

Measured for approach 1 in DuckDB on 2026-11-26, a million rows and 177 shapes of about 4.5 KB under a brush-like filter: grouping the row-level join by the shape took 47 ms; grouping the rows by country code and then attaching the shapes took 3 ms. The first would reach about half a second at ten million rows (scaled, not measured). Vega-Lite's county choropleth makes the same choice: the shapes drive, and a `lookup` transform brings one value per county onto them. Hence the ruling's last clause.

## The design

### The bundled dataset

An arcform Protocol in open-analytics publishes Natural Earth's 1:110m `admin_0_countries` to the lake as two tables, each with its Data Package descriptor stamped in the Parquet as the other lake datasets are:

- **`countries`**: one row per country. `iso_a3`, `iso_a2`, `name` (Natural Earth's `NAME`), `continent`, and `geom` as GeoJSON text. The ISO columns take Natural Earth's `ISO_A3_EH` / `ISO_A2_EH` where `ISO_A3` / `ISO_A2` hold `-99` (France and Norway in recent releases), so every country has a code. Antarctica is kept here and filtered in the plot (below).
- **`country_names`**: one row per spelling. `name_key` (the spelling, lower-cased, trimmed, accents folded), `iso_a3`. Built from every name field Natural Earth carries (`NAME`, `NAME_LONG`, `FORMAL_EN`, `ADMIN`, `SOVEREIGNT`, `NAME_ALT`, the `NAME_<lang>` fields in about 25 languages), the two- and three-letter codes, and a short alias list kept in the Protocol for common abbreviations Natural Earth lacks ("USA", "UK", "RSA", "EIRE"), each alias with the reason it is there. A spelling that would map to two countries is left out.

**Order, my call.** The open-analytics Protocol and its publication come first, in their own pull request. Brightfield's work does not wait on it: until the lake holds the files, its tests read a hermetic fixture of the same schema, and the bundling script and the shipped copies land once the files are published. Brightfield bundles a copy of each published Parquet with the app, fetched by a script beside `scripts/cut-usgs-earthquakes.sh` that checks size and row count, as the earthquake start's file is. Budget: under 400 KB for the two files together; a larger file goes back to Hugh.

### The backdrop behind a point map

The generated point map writes, as its first layer, a `geo` mark over `countries` (filtered to drop Antarctica unless the points reach below 60°S), filled with the design system's quietest surface tint and stroked in the page's background colour at a hairline, so borders read as gaps the way PostHog's do. It is an ordinary layer the analyst can see, restyle or delete. The countries table joins the Protocol as a source the first time a point map over it is generated.

The earthquake start's dashboard gains the same layer by hand, and its Protocol the countries source; its revision in `crates/brightfield-shell/src/starts.rs` goes up so its thumbnails are re-shot. The `geo` layer draws in the plot's cartesian scales there (Constraints).

### The choropleth

**Offered when** a table has a country column by goal 3. When the column holds names, the offer's description says so, since names match less reliably than codes.

**Where it writes, my call.** Choosing a choropleth writes into the open Protocol through the same write path the operation forms use to add a step, with the same undo. It is offered over any Protocol a form may add a step to, and nowhere a form may not. Generating a dashboard for an analyst's own file, which already writes that file's Protocol (`own_files`), adds the countries source there when the file has a coordinate pair, so the point-map tile's backdrop has a table to read. Opening someone else's Protocol never writes to it: its point maps draw without a backdrop until the analyst adds one.

**Choosing it** writes into the Protocol, through the same operations a hand-built Protocol would use:

1. the countries table and the names table as sources, unless already present;
2. a join step, a new step downstream of the analyst's table, that keeps every column and every row of it (left join) and adds `iso_a3` and `geom`: matched on the code column directly, or on `name_key` for names (the column value lower-cased, trimmed and accent-folded the same way). A row that matched nothing has a null `iso_a3` and `geom`. The step is a view, so the shapes are not copied per row. The step's Data Package schema declares `iso_a3` a foreign key to `countries.iso_a3`, and `geom` as taken from it.

**The spec stays plain Mosaic, my call.** The plot is one `geo` layer `from:` the join step, `fill: { count: }` (or a measure the analyst picks, as the other kinds take one), with the default `geom` geometry. Any Mosaic reader draws it correctly by grouping on the geometry, the 47 ms shape. Brightfield's lowering gets the 3 ms shape without changing the spec: when a `geo` layer's table declares that its geometry comes from another table through a foreign key, it groups the rows by the key under any `filterBy:` clause, then attaches each shape from the shapes table, shapes first, so every country is drawn and a country with no rows gets the backdrop's grey. A test over a million rows holds the timing. Rejected: a new attribute naming the shapes table on the mark, which other Mosaic readers would not draw; and pre-joining the aggregate in the Protocol, which a brush elsewhere could not narrow.

The rest of the plot:

- `colorScheme` a single-hue sequential ramp, `colorScale: linear`, no graticule, no axes, `projectionType: equirectangular`, Antarctica filtered.
- **Which plots it filters, my call.** A click on a country publishes a `select: toggle` clause on the analyst's own country column, as the values that matched that country (`country IN ('United Kingdom', 'UK')`), not on `iso_a3`. The join step keeps that column, so plots over the original table and plots over the join step both read the clause, and the existing plots on a page need not move to the join step. The layer takes `filterBy:` the page's selection, so a brush elsewhere recolours the map. The clause the map publishes is new work in the `geo` interactor.
- a `tip` showing the country's name and its value.
- **The unmatched list is the shell's, my call.** Below any `geo` plot whose table declares its geometry through a foreign key, Brightfield writes the count of rows with a null key and the five most frequent unmatched values of the joined column. It counts the whole table, not the brushed rows, because it reports the join's coverage and not the selection. It is hidden when every row matched. It is not a spec element, since Mosaic has none for text from a query, and another Mosaic reader simply does not show it; Relate shows the same match rate.

### FineType and Relate

The offer reads FineType's label first, then the column name. Relate shows the join step as any other join, with its match rate as evidence, because the step is ordinary Protocol SQL. Nothing is added to dovetail for this design.

### Colour scales

`log` and `quantile` join `DRAWN_COLOUR_SCALES` (`crates/brightfield-spec/src/layout.rs`), with their legends, so an analyst with lopsided data (the retail start, where the United Kingdom holds most orders) can switch to them. Mosaic's own unemployment map uses `quantile` with `colorN: 9`.

### The retail start

`examples/starts/online-retail-ii` gains the countries source, a join step over `retail_orders`, and a choropleth of orders by country beside its "Orders, by country (top eight)" bar, both on the page's selection. The bar and the other plots keep reading `retail_orders`; the bar's `toggleY` clause is on `country`, which the join step keeps, so the map follows it. Its countries are names as the shop's database writes them: most match, "EIRE", "RSA" and "USA" match through the alias list, and "Channel Islands", "European Community" and "Unspecified" are listed as unmatched.

### Tests and baselines

- A hermetic fixture table of country names and codes, including unmatched values and spellings in two languages, drives the join step, the unmatched list and a pixel baseline of the choropleth; the gate has no network and no FineType bundle, so the name pass of the offer is the one the gate holds, and an `#[ignore]`d test run with `BRIGHTFIELD_FINETYPE_BUNDLE` holds the label pass.
- A point-map baseline with the backdrop, and the earthquake start's baselines, redrawn on the Mac Studio in the same pull request.
- The timing test above: aggregate-then-attach over a million rows stays within a fixed multiple of a plain `GROUP BY` on the same rows.
- A click on a country publishes its clause, and a brush on a bar narrows the map, driven through the shell as the existing interaction tests do.

## Left to the builder

- How the bundled Parquet is resolved from a Protocol step offline (a path in the app bundle, as the earthquake start reads its own folder), and the source step's form.
- How the `geo` lowering aggregates by key and attaches the shapes, and how `filterBy:` and `select: toggle` reach a `geo` layer.
- The accent-folding and normalisation rules for `name_key`, and the alias list's first entries.
- The backdrop's exact tint and stroke within the design system's tokens, in both themes.
- Whether the Antarctica filter is a `data.filter` line or a property of the countries table read by the plot.
