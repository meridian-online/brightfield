# Changelog

What each Brightfield release changes, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/) with the caveat the README gives: before 1.0, any release may change spec semantics, CLI arguments or rendering output.

A release's section here is its release notes. The release workflow refuses a tag that has no section, and publishes the section at the head of the GitHub release. To cut a release, bump `version` in `crates/brightfield-shell/Cargo.toml`, move the items under `[Unreleased]` beneath a heading for that version and the date, and tag the commit that does both.

Releases before 0.1.6 predate this file; their tags and the commit history are the record of what they carried.

## [Unreleased]

### Added

- The front door offers six starts: Palmer Penguins, the USGS earthquakes of 2025, NYC yellow taxi trips, Online Retail II, Epoch AI's notable models, and a run of the EDGAR↔GLEIF crosswalk. The first five are arc Protocols that run on the first click and open their dashboard beside its grid; the taxi, retail and Epoch AI cards fetch their data over the network on that first run and say so.
- A filter typed in a dashboard's grid narrows its charts, as a brush on a chart narrows the grid.

### Changed

- The California Housing card fetches the dataset from StatLib, where its authors published it, the first time it is opened, and says on the card that it needs the network. Brightfield no longer ships a copy, and the release's `examples/data/` holds a reduced copy of the EDGAR↔GLEIF crosswalk instead. The table now has all 20,640 block groups and the source's own nine columns, with rooms, bedrooms and households as totals rather than per-household averages.

## [0.1.7] - 2026-10-20

### Added

- An agent-written Protocol opens with its charts drawn, and its report exports to PDF, PNG and data from the window and from the command line.
- Type SQL in the chart palette: preview its result, then keep it as a step or as a filter.
- Apply a filter or a sort from the grid's cursor; it is recorded as a step of the Protocol.

### Changed

- A right-hand legend starts under its plot's title, so it no longer covers the title.
- The grid shows doubles, and the band shows its range, at two decimal places.
- The shelf's mark cell no longer shows a chevron, since it does not yet open a list.

### Fixed

- The EDGAR↔GLEIF crosswalk chart's colour legend is back, below the grid.

### Release

- A release whose packaged artifacts carry no DuckDB engine fails before it is published.
- The sealed release read-back opens a Parquet file, and refuses the artifact when it cannot.
- rustls is at 0.23.46, for four TLS advisories.
- A release tag must be `v` and three numbers matching the shell crate's version, with a section in this file; that section becomes the release notes.

## [0.1.6] - 2026-10-14

### Added

- The shelf band under the hero pane: a cell per channel showing its column, its key and its hue. Put a column on a channel from the Outline by key or by its chips, take the last one back with `u`, and Save writes each edit into the chart file as a change to one line.
- Settings lists for an axis and for colour: ticks, grid, zero, reverse, scale, range, format, title and tick count for an axis, and scheme and reverse for colour. Each part of an axis is a click target into its row, and Tab turns an axis's shelf list to its settings.
- Colour by a number column: sequential, diverging and quantized ramps, `colorScheme`, `colorDomain`, `colorRange` and `colorReverse`, with a legend placed right, below or nowhere and written into the chart file.
- Saved versions: each Save records a version of the chart file in arcform's local history, the ledger's Versions panel lists them newest first with what changed, and a chart can step back to one.
- The axis attributes `xDomain`, `yDomain`, `xyDomain`, `xZero`, `yZero`, `xNice`, `yNice`, `xReverse`, `yReverse`, `grid`, `xGrid`, `yGrid`, `xTicks`, `yTicks` and number and date tick formats.
- A cell cursor on the grid, moved with the grid's keys.
- Closing a window with an unsaved chart edit asks whether to save and close, close without saving, or keep editing.
- The design system's square look across rails, tabs, rows, the hover readout and the chart's menu and checkbox boxes.

### Changed

- The warning banner names a plot attribute, axis instruction, scheme or scale this build does not draw, rather than ignoring it.
- A numeric column's rug is drawn in the Outline's column list in place of its type.

### Fixed

- A plot's right-hand legend is drawn beside the plot, not past the edge of the page.
- An analyst's comment above a line that a reset removes is kept.
- The finetype extension stays byte-identical through a Homebrew install.
