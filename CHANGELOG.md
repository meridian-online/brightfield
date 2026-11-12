# Changelog

What each Brightfield release changes, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/) with the caveat the README gives: before 1.0, any release may change spec semantics, CLI arguments or rendering output.

A release's section here is its release notes. The release workflow refuses a tag that has no section, and publishes the section at the head of the GitHub release. To cut a release, bump `version` in `crates/brightfield-shell/Cargo.toml`, move the items under `[Unreleased]` beneath a heading for that version and the date, and tag the commit that does both.

Releases before 0.1.6 predate this file; their tags and the commit history are the record of what they carried.

## [Unreleased]

### Added

- Every open makes a Protocol. A data file, folder, workbook or database becomes a Protocol in the Protocols folder, `~/Documents/Protocols` by default, named after its first source (`earthquakes_2024`, then `earthquakes_2024-2`). The folder is a setting in Preferences (`⌘,`), which offers to move the old folder's Protocols when it changes.
- Open files the Mac's ways: double-click or Open With in Finder, drop onto the Dock icon or the front door, or `⌘O`, one panel for a file, a folder or a Protocol. A `brightfield://` link says it does not open yet.
- Opening a file that already has a Protocol reopens it when that Protocol reads only that file and no other Protocol reads it; otherwise a sheet asks which.
- `⌘S` saves the Protocol from anywhere in the window. Making a Protocol, adding a source, accepting or dropping a join, and rename are kept at once; every other edit is a draft until `⌘S`. Closing the window, going Home, opening another Protocol, adding a source or quitting over a draft asks to save, close without saving or keep editing, and closing without saving keeps the edits as a version.
- Rename a Protocol from the window (`F2`) or its row on the front door, which renames its folder; move it with the Location field in the locator band. A Protocol moved in Finder is repaired when opened.
- `⌘N` makes a new, empty Protocol, asking where.
- `d d` removes a step from a Protocol's spine, and puts it back.
- The front door lists Recents and a tree of the Protocols folder, with each Protocol's run state. Brightfield's own previews are not runs: a Protocol only previewed reads "previewed, not run".
- Relaunching restores the Protocol that was open.
- The first launch from the app asks once whether to make Brightfield the default app for CSV, Parquet and DuckDB files, and Preferences does the same at any time. macOS confirms each kind of file itself.

### Changed

- A single CSV no longer opens as a bare file with its Protocol written beside it on Save. It opens as a Protocol made in the Protocols folder, and Save and Run write there.
- The front door lists a Protocol from the moment it is made, saved or not.
- The failed-values step and its table show on the spine only when they hold rows.

### Fixed

- Opening the same CSV again no longer makes another Protocol of it each time.
- A Protocol whose folder was deleted or moved in Finder says so instead of writing the folder again on Save.

## [0.1.8] - 2026-11-02

### Added

- The front door offers six starts: Palmer Penguins, the USGS earthquakes of 2025, NYC yellow taxi trips, Online Retail II, Epoch AI's notable models, and a run of the EDGAR↔GLEIF crosswalk. The first five are arc Protocols that run on the first click and open their dashboard beside its grid; the taxi, retail and Epoch AI cards fetch their data over the network on that first run and say so.
- A filter typed in a dashboard's grid narrows its charts, as a brush on a chart narrows the grid.
- Open a folder of CSV, TSV or Parquet files, an Excel workbook, or a DuckDB or SQLite database. Each table is loaded by an arc Protocol with the file, sheet and row it came from, profiled by FineType, and cleaned through FineType's casts, with the values that fail a cast kept in a table of their own. The tables open on a model pane as a diagram, a list or the Ossie file, where the analyst keeps, drops and adds joins, sets each table's role, folds reference tables in, leaves tables out and retypes columns. Until dovetail is built in, no joins are proposed, and the analyst adds them by hand.
- Copy the chart under the pointer to the clipboard as a picture (`⌘⇧C`), or save it as a PNG (`⌘⇧E`), from the window or its palette.
- The compact header band names each column's FineType type by its title, with its icon where FineType's catalogue gives one.

### Changed

- Brightfield reads Meridian's open datasets at their new addresses on `openlake.meridian.online`, each under its publisher's namespace: the EDGAR↔GLEIF crosswalk at `library/edgar_lei.parquet`, SEC's company tickers at `sec/company_tickers_exchange.parquet`, GLEIF's LEI records at `gleif/lei.parquet` and NAICS at `census/naics_2022.parquet`. The crosswalk start and the EDGAR↔GLEIF Protocol read them there. The old flat addresses (`edgar_gleif.parquet`, `edgar.parquet`, `gleif.parquet`, `naics.parquet`) no longer change and will be retired no sooner than 90 days after this release, after which 0.1.7 and earlier cannot open those starts: upgrade before then with `brew upgrade meridian-online/tap/brightfield`.
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
