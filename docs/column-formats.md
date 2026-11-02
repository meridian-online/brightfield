# Numbers and dates formatted from a column's type

An analyst who opens a file sees each value written the way its type is read: money as money, rates as percentages, years as `2024`, before they touch a setting. A chart that prints a year as `2,024`, or a rate as `0.045`, is a default the analyst would have to fix before they could look at their data.

Brightfield already knows what each column is: FineType labels it on open, and the dashboard generator reads that label to choose a tile. The same knowledge, with what the profile measured, chooses how each value is shown on a tile's axes, in the grid and in the hover tooltip. The choice is written into the chart file in Mosaic's own terms, so a saved chart draws the same in any Mosaic tool.

The principle behind it: make the most of the frameworks Brightfield has chosen, and where Brightfield knows more about the data than a framework's defaults do, improve the representation. Brightfield is not bound to a framework's default decisions, but whatever it decides is written in a form the framework reads.

## What it does

- A file opens with its numbers and dates formatted from each column's type, on tile axes, in the grid, and in the tooltip of a dot chart. On a generated tile the axis and the grid come from one decision per column and agree, and the tooltip follows the format the axis actually drew, so it never contradicts the axis beside it.
- The formats are written into the chart file as `xTickFormat` / `yTickFormat`, axis titles, domains and grid settings, so another Mosaic tool draws a generated chart the way Brightfield does.
- A lone CSV, TSV or Parquet file is cast on open, as a workbook sheet already was, so a currency or percentage column written as text becomes a number with a number axis.
- A column's type is remembered. Its label is kept in the source's `datapackage.json` and read back, so a cast column keeps the label that says what it was.
- A year column draws as counts per year in time order, not as the ten busiest years ranked by count.
- Every unformatted continuous number axis groups thousands (`12,000`), as Plot's default does. A band of whole numbers with no format prints its categories as written (`2024`), where Plot would group them (DEV-0019).
- A committed baseline over a fixture with currency, dollar, percentage, rate, year, count and date columns reddens when a label's format changes: its generated dashboard YAML is a snapshot, beside light and dark PNGs of the dashboard and its grid.

## What it does not do

- **No currency symbol other than `$` on axis ticks.** A Mosaic spec carries a d3-format string and no locale, and Observable Plot handles locale only through a format function, which a declarative spec cannot hold. Until Plot and Mosaic carry a locale, a sterling axis names its currency in its title.
- **No `B` for billions on ticks.** d3's `s` prints SI prefixes, so a billion is `G`. Brightfield scales in the query and names the unit in the title instead.
- **No control yet to accept a label or to set a column preference.** The record holds both and every open honours them; the controls that write them are separate work.
- **No view of stored values in the grid**, the counterpart of Excel's Show Formulas. Hovering a formatted cell shows the stored value in the meantime.
- **No change to chart furniture.** Gridlines, tick marks, tick labels and axis lines are drawn as they were; the dashboard snapshots were reviewed and they stand.
- **No change to FineType's taxonomy from this side.** The display formats and the percentage cast are FineType's, and Brightfield vendors FineType's release.

Four things are ruled out, because each would undo the point of the design:

- **Misrepresenting a value.** No format invents a convention the file did not use, and no surface shows a number without a way to reach the stored one. A scaled axis states its unit.
- **A Brightfield-only format.** Nothing is written into a chart file that a Mosaic tool would read differently, such as a private locale attribute or a `G`-to-`B` substitution.
- **A second taxonomy.** Brightfield keeps no table of how each FineType label is shown. That lives in FineType.
- **Slowing the open.** The cast happens in the in-memory copy the flat route already makes.

## Who decides what

A column's representation is derived on demand from its record, by one rule (`representation_of` in `crates/brightfield-shell/src/representation.rs`). The tile generator writes the rule's answer into the chart file; the grid and the tooltip call the same rule when they draw. Nothing derived is stored, so a format cannot drift from the label it came from, and an analyst's override of one chart lives where it always has, in that chart's axis format. Storing a format in the record instead would add a Brightfield property holding a value derivable from the label, which could disagree with the label after it changed; storing it in the Ossie model would leave a lone CSV, which has no model, without one.

The rule uses FineType rather than rebuilding it, so it splits by who knows what:

- **FineType knows what a label implies**: money with two decimals and parenthesised negatives, a fraction shown as a percent, a year with no grouping, a size in bytes. Each label that holds numbers or dates carries a display block in FineType's label definitions: a d3-format specifier for a number or a d3-time-format specifier for a date, and what a specifier cannot say (the value is money, in which currency when the label fixes one, a fraction shown as a percentage, a size in bytes). Brightfield vendors the blocks as `crates/brightfield-protocol/vendor/finetype/displays.json`, beside the `transforms.json` the casts come from, and reads them through `FinetypeDisplays` in `crates/brightfield-protocol/src/display.rs`. Every percentage-shaped label's cast stores a fraction, as Excel does, so d3's `%` prints it correctly.
- **Brightfield knows what only the data shows**: the scale (£bn), the precision a spread needs (`.1%` against `.0%`), the run of years, and the currency symbol and negative style a particular file used.
- **The analyst's preference, where one is recorded, wins over both.** It keeps the one advantage of storing a format, a column format that follows the column everywhere, as an explicit choice the analyst makes, not a stored copy of the default.

## The column record

A source's `datapackage.json` is the record of what the source taught Brightfield, in properties Frictionless and FineType already define (`crates/brightfield-shell/src/column_record.rs`).

- **Inferred**: a field carries FineType's label as `x-finetype-label`.
- **Accepted**: a field may also carry `x-finetype-nominated: true`, the marker FineType's own datapackage emitter writes for a nomination. A nominated label is taken as given: the next open does not ask FineType's classifier about that column and uses the recorded label.
- **Brightfield's own property.** Everything Brightfield adds to a field sits in one object, `x-brightfield`, so its additions stay in one namespace beside FineType's `x-finetype-*`:

  ```json
  {
    "name": "revenue",
    "type": "number",
    "x-finetype-label": "finance.currency.amount",
    "x-brightfield": {
      "written": { "currency_symbol": "£", "negatives": "parentheses" },
      "prefer": { "number_format": ",.0f" }
    }
  }
  ```

  `written` is how the source wrote the column: the currency symbol its non-null values share (none when they use more than one), and whether its negatives are in parentheses or take a minus. The cast strips both, and the label cannot say which the file used (`finance.currency.amount` accepts both `(…)` and `-`), so they are read from the raw text when the column is cast, and they live on the field because they are facts about the column. `prefer` is the analyst's preferred number and date format for the column, over FineType's default, so a preference follows the column into every chart, the grid and the tooltip; analysts have preferences beyond FineType's defaults, and those need a home.
- **What belongs in the package**: facts and preferences about the source and its columns (how it is written, its accepted labels, an analyst's column formats). What belongs to a view (a chart's layout, a dashboard's choices, a grid's column widths) stays in the chart file or the workspace, so that `datapackage.json` does not become Brightfield's settings file.
- **Other properties survive.** The package is edited as JSON, so a property the record does not model, such as `x-finetype-confidence`, survives a write. A write goes to a temporary file first, so a crash never leaves a half-written record. A Protocol's package that cannot be read is regenerated when its packages are written again. A lone file's record that cannot be read is skipped with a warning and left as it is: the file still opens, its columns profiled afresh.

### Where the record lives

- **A Protocol** (a folder, workbook or database) writes `packages/<source>/datapackage.json` in its own folder. When the packages are written again, each field keeps its old `prefer`, and its nomination with the nominated label, and takes `written` from the values just read, so a recorded spelling never outlives a change to the data. `written` is read from the raw table the silver cast reads, for each column the silver cast casts under a `finance.currency.` label.
- **A lone file's record** lives in Brightfield's own data folder, at `datasets/records/<key>/datapackage.json`, where `<key>` hashes the file's canonical path and nothing else, under a resource named `opened`. Brightfield does not write into folders the analyst did not ask it to write to. The resource carries `x-brightfield-source`: the file's size, its modification time, and its canonical path, which is stored there in plain text. When the size or modification time differs on open, the inferred labels and `written` are derived again, and nominated labels and `prefer` are kept, so an edited or re-exported file keeps the analyst's choices.
- **Pruning.** Once per process, on the first flat open, a lone file's record is removed when its stamped file reads as not found and the folder that held it is still there and holds something. A file on a drive that is not mounted, one behind a permission error, one in an empty folder, and a record with no stamped path all keep their records: a record is the analyst's choices, and a doubt is not grounds to delete them.

### Read back on open

Every open reads the record before profiling.

- A lone file passes its nominated labels to the profile as given labels, so FineType's classifier is not asked about those columns.
- A single sheet or table opened through a Protocol is reopened on the flat route from a Parquet copy of its cast table, under the Protocol's `.grid/` folder. The flat route recognises a file under the `.grid/` folder of a Protocol folder holding an own-files `arcform.yaml`, whose name is a resource in that Protocol's `packages/`: it takes every labelled field of that resource as given, since the copy was cast from those labels and profiling it again would lose them (a cast currency column would come back unlabelled), and it opens the file as already cast, so a rate is not divided by 100 twice. Any other file in that `.grid/` folder, such as a join preview, is built from cast tables and opens as already cast with no given labels. A `.profile/` snapshot holds the raw table, so it opens on the normal flat route and is cast. A label given this way comes back with full confidence; the confidence the package records is not carried through.

## Casting a lone file

The flat route reads a lone file through FineType's casts, as a Protocol's silver table does (`crates/brightfield-shell/src/flat_cast.rs`).

- **The open is ordered** profile, copy with casts, one aggregate over the in-memory copy, then the dashboard. The aggregate decides each cast, measures each cast column's minimum, maximum and spread, and reads `written` from the raw text; it scans memory, not the file, so the file is read once. This is the cheapest order that gives the rule the cast values' range: profiling the copy again would repeat the classifier.
- **A column is a candidate** when its label is trusted (its values do not contradict it), FineType's own projection casts it, the type it casts to is one a format reads (an integer, float or decimal type, `DATE`, `TIME`, a `TIMESTAMP` form or `BOOLEAN`), and DuckDB stored it as neither a number nor that type already.
  - **A column DuckDB already stored as a number is never cast.** The cast would gain nothing and can lose the value: DuckDB renders a large double as `1e+16`, which a currency transform reads as `116`, and rounds `123.456` on its way to a `DECIMAL(18,2)`, with no NULL to show for either. Its format comes from its label, not its storage type.
  - **A label whose cast targets any other type** (JSON, UUID, INTERVAL, the coordinates struct) is read as it is. No format reads it, and its cast may not even bind; `TRY` catches a value that fails to cast, not a cast that fails to bind, and one cast that fails to bind would leave the whole copy uncast. A Protocol's silver table still casts such a column.
  - **A label FineType types as text passes through.** Some currency and rate labels are typed VARCHAR in FineType's export although their transforms return numbers; they stay text until FineType corrects their type. Brightfield follows FineType rather than second-guessing it.
- **Every non-null value must cast.** The aggregate compares each cast column's non-null count with its text's, and a cast that turned any value into NULL is not made: the column is put back as the file wrote it. A single `n/a` among thousands of amounts keeps the column text and its tile what it was. This is the high-confidence bar: a cast that would silently empty some cells is worse than none.
- **Reversible.** Each cast column's raw text is kept beside it under the reserved prefix `__raw__` (`__raw__revenue`). The grid hides the raw columns the source declares, and shows a file's own column of such a name. Undoing a cast is reading the file again without it.
- **A file over the memory budget**, which is read through the view without a copy, is not cast, and its tiles are what they were.
- **A Protocol's silver cast keeps its own rule**: a value that will not cast becomes NULL and is kept in the table's failed values. Aligning the two routes is separate work.

**The cast is declared in the spec.** The chart file reads the file under `opened_rows` and declares the source `opened` as plain DuckDB SQL over it: `SELECT * REPLACE (TRY(<transform>) AS "c", …), "c" AS "__raw__c", … FROM "opened_rows"`, written as a YAML block scalar so a multi-line transform reads back exactly. SQL holding a control character other than a tab or a line feed, such as a carriage return in a CSV header, is written as a double-quoted scalar with the character escaped, since a block scalar cannot hold it. Any session loaded from the spec, whether a tile switch's rebuild, the written spec opened again or another Mosaic tool, reads the columns as cast.

> **Note: a rebuild re-runs the casts.** Because a lone file's casts are declared in its spec as `TRY` casts over the file's view, a rebuild (a tile switch, the written spec opened again) runs them again on each query over the file. Whether a cast stands was decided once, on open, from the values. A file that gains a bad value after it was opened reads that value as NULL until the next open decides again and puts the column back as text.

## The representation rule

`representation_of` takes a column's profile, the display block FineType gives its label, its record, and whether the label's cast ran on it, and returns:

- **a tick format**: a d3-format or d3-time-format specifier for the chart file;
- **a scale**: a divisor the tile's bin applies, with its unit word, or none;
- **a title unit**: the currency symbol, the scale, or both (`£bn`), or `%`;
- **a cell format** the grid prints a full value with, and **a compact format** the tooltip prints with;
- **the symbol** the source wrote, which the grid and the tooltip print.

A column the rule has nothing to say about (unlabelled, a label with no display block, or a text column) gets an empty representation, and every surface draws it as it did before.

How the data refines FineType's block:

- **Storage decides which kind of format applies.** A number format is given only to a column stored as a number, and a date format only to a date, time or timestamp column. A text column under a rate label gets nothing, since text cannot take a number format.
- **Scale.** A money column whose larger end, by magnitude, reaches 10⁴ is divided by the largest of 10³, 10⁶ and 10⁹ not above it (`k`, `m`, `bn`). A size in bytes from 2²⁰ up is divided by 2²⁰ or 2³⁰ (`MiB`, `GiB`), which d3-format cannot express. Other numbers are not scaled. Negative and zero values keep d3's minus sign and print `0`.
- **The division lives in the bin.** Mosaic's `bin` transform takes a data column or an expression, so a tile bins `{ sql: "\"revenue\" / 1e9" }` and a Mosaic tool bins it the same way, and a brush over it publishes a range in the same units its predicate tests.
- **Precision.** A fraction's precision is the fewest decimals, from 0 to 3, at which five evenly spaced ticks across its spread print distinctly (`.0%` for 12–78%, `.1%` for 4.1–4.6%). Money keeps two decimals in cells; on axes it keeps the decimals the ticks need.
- **A percentage the cast did not store as a fraction.** A column DuckDB already stored as a number is never cast, so a percentage held as `45.0` is 45, not the 0.45 the percentage cast stores. The rule is told whether the label's cast ran on the column (from the session's cast list on the flat route, or from FineType's projection for a Protocol's copy, which holds the cast table), and an uncast percentage gets a plain fixed-precision format with `%` in its title and attached to its cells (`45%`), never d3's `%`, which would print `4500%`.
- **Only `$` reaches a tick.** A column whose recorded symbol is `$` writes `$` in its tick format. A recorded symbol is the currency sign with the letters written against it, so `A$`, `HK$` and `US$` are recorded whole and are not `$`, and a column that mixes `US$` and `A$` records none. Any other currency writes the format without a symbol and names the symbol in the title (`revenue (£bn)`). A label that fixes a currency, on a column with no recorded symbol, names the currency's ISO code in the title only (`CHF bn`): the label fixes the currency, so this invents nothing. A money column with no recorded symbol, because its values mixed symbols or it has no record, is shown with none, and never with `$`.
- **Accounting negatives.** A column whose record says its negatives are in parentheses prints them so (`(£12.00)`); one whose record says minus prints a minus whatever the label's default.
- **A preference wins.** A recorded `prefer` format is used as written on the axis, in the grid and in the tooltip, and the column is not scaled. On a money column a `$` in the preference reaches the tick only when the record holds `$`.

## What a generated tile writes

- `xTickFormat` / `yTickFormat` from the rule, quoted, on the axis the column is drawn on. The count axis of a histogram or a counts-per-date tile writes nothing, since Plot's default and Brightfield's both group thousands. A column drawn through a resampled date bucket writes no tick format, since its label's format does not describe the bucket.
- The axis title with its unit, when the rule gives one: `xLabel: "revenue (£bn)"`.
- A scaled column's bin over its expression, so the bins and the ticks are in the unit the title names.
- `xGrid` / `yGrid` as Brightfield draws them, because Plot draws no gridlines by default and the file should draw the same everywhere. A saved chart names Brightfield's default wherever Mosaic's would draw differently.
- The comment above each tile says which label, or the column's recorded preference, chose the format and the scale, beside the existing sentence on which label chose the tile.

A column put on a generated tile's axis through the shelf takes out the axis's tick format, its title and its listed domain where each still holds what the tile wrote for the old column, in the same edit, so one undo puts them all back. Each describes the old column only: `$` on a count would read `$1,200`, `d` would round a fraction, `revenue (£bn)` would name a unit the new column does not have, and a band pinned to a run of years would match no value of the new column and draw no bar. A format or title the analyst typed stays.

### Years

A `datetime.component.year` column draws as counts per year in time order, through the counts-per-date tile, whatever the category ceiling says: a year is a member of an ordered set, and ranking the busiest ten loses the series.

- The tile writes `xTickFormat: "d"`, because Plot's default number format groups thousands and would print `2,024`.
- The tile writes the full run of years from the profiled minimum to the maximum as a listed `xDomain`, so a year with no rows shows as an empty slot rather than a missing one; a band takes its order from a listed domain. A run of more than 120 years is left to the data: at a tile's width, 120 slots is about the narrowest a year's bar can be and still be seen. A year column stored as text writes no domain, since another Mosaic tool matches a listed domain by value, and the number 2001 is not the word `"2001"`, so every bar would go.
- A band whose categories are all whole numbers, spelled the way an integer prints (no leading zero, no `+`), is a number axis for its text and takes a number format. Text that merely parses (`02134`, `+7`) is an identifier and stays a name, so a format never rewrites it.
- A band over a numeric column is laid out in value order by the renderer whatever `sort:` says. The counts-per-date tile relies on that order. It also means that ranked bars over a numeric category column (integer codes, say) draw in value order rather than by count; a text column's ranked bars keep their count order, which comes from the query. Making a numeric category keep its count order is separate work.

> **Note: an unformatted band of whole numbers prints ungrouped.** A band of whole numbers with no format prints its category text as it is (`2024`), while Plot's band prints `toLocaleString` (`2,024`). Generated year tiles avoid the difference by writing `d`; a spec over a numeric band without a format, such as a hand-written one, draws differently in the two. `DEVIATIONS.md` records it as DEV-0019.

### Brightfield's own default number format

With no format, a continuous number axis groups thousands (`12,000`, `1,234.5`), as Plot's default does, so an unformatted axis reads the same in Brightfield and in Mosaic. Each committed chart that puts a year on a number axis writes `xTickFormat: d`: the file says what it means, as Mosaic needs it to, and the renderer does not guess years from a hand-written chart.

## The grid

The grid (`crates/brightfield-shell/src/data_grid.rs`) formats only columns whose representation has a cell format: a labelled column with a display block, stored as a number or a date. An unlabelled column, or one whose label has no display block, keeps its old display, so an unlabelled identifier is never grouped as `1,234,567`.

- A labelled number prints its full value in the file's convention: `£1,234.50`, `(£12.00)` for an accounting column, `45%`. A date prints through its date format. A timestamp is shown as stored, since its text carries a time of day the date format does not read back.
- The header names the unit after the column, `revenue · £` or `share · %`, or a fixed currency's ISO code. The grid shows full values, so it never names a scale or a byte unit.
- Hovering a formatted cell shows the stored value wherever the two differ.
- A cell is widened to keep the stored value's own decimals, up to four, for fixed and percent formats, so a fraction stored as `0.455` reads `45.5%`, not `46%`. Ticks and tooltips are not widened.
- The typed-filter preview reads the file through the casts the session's copy stood on, so a condition reads a cast column as the applied filter does (`share > 0.4` keeps the same rows in both), and it prints its rows the same way, so it never shows stored values under a unit header.

## The tooltip

The hover tooltip formats the dot readout (`hover_line` in `crates/brightfield-shell/src/chart_item.rs`). It names the column, never the bin's SQL: a line over an expression bin is named for the one column the expression reads. It follows the axis the chart drew, so it never contradicts the axis beside it:

- **The rule's own tick format on an unscaled column** prints as the column's compact format does, matching the axis, with the symbol and a percent sign attached (`45%`).
- **The rule's own tick format on a scaled column** is drawn over the raw values on a dot chart (only a generated bin is divided by the scale, and bars and histograms have no hover), so it is read like any other written format: `revenue: £2,412,345,000`, beside ticks that read `2,500,000,000`. The compact reading, `£2.4 bn` with a no-break space (U+00A0) before the unit word, belongs to the scaled bin; its small values keep at least two significant digits (`£0.04 bn`, not `£0.0 bn`).
- **Any other written format** (a chart the analyst wrote, or a format they typed) prints as written, with the currency symbol added where the format prints none. A stored fraction under a format with no `%` reads as that format reads it, with nothing appended.
- **An axis with no format** prints the grouped default, unscaled, with the symbol. A stored fraction is multiplied by 100 and given its `%`; a column of percentage points has `%` appended. A column whose own tick format is ungrouped (a year's `d`) prints by that format.

Bars and histograms have no hover yet: hover reads rows, not bins. Their tooltip, including a bar's range line in the column's units, is future work, and will follow the same rule.

## The inspector

An axis's **format** row (`crates/brightfield-shell/src/shelf.rs`) shows where the axis's format came from.

- A file holding exactly the rule's specifier reads `auto`, with the specifier beside it.
- A file holding no specifier reads `default`, the axis's own tick text, with the rule's specifier beside it as a suggestion where the rule has one. The unset row reads `default` on every column, labelled or not.
- Any other specifier is shown as typed, or as the preset it equals.

The stepper's `default` preset takes the specifier out, and its `auto` preset writes the rule's specifier for the row's column. Where the rule has no specifier (an unlabelled column) the stepper skips `auto`, and where the rule's specifier equals a later preset's (a preference of `~s`, say), it skips that preset too, so every remaining preset stays reachable. Reset means back to Brightfield's choice: on a column the rule gives a specifier, resetting the row writes it, whatever the row held (a typed format, or none), so the row reads `auto`; where the rule gives none, reset takes the key out and the row reads `default`. An axis drawn from an expression, such as a scaled bin, takes the rule's format for the tile's column only when the expression names that column and no other.

The file holds a specifier either way, and no marker says which kind it is: a writer-only marker would be a Brightfield-only attribute, and a saved file is the analyst's document. The generated dashboard is generated again on each open, so it always follows the current label. A chart the analyst has saved keeps the specifier it was saved with: a label changing later shows as that row no longer reading `auto`, and nothing rewrites a saved file.
