# Numbers and dates formatted from a column's type — design

## Intent

An analyst who opens a file should see each value written the way its type is read: money as money, rates as percentages, years as 2024, before they touch a setting. A chart that prints a year as 2,024, or a rate as 0.045, is a default the analyst has to fix before they can look at their data.

Brightfield already knows what each column is. FineType labels it on open, and the dashboard generator reads that label to choose a tile. This work uses the same knowledge, together with what the profile measured, to choose how each value is shown on a tile's axes, in the grid and in the hover tooltip. The choice is written into the chart file in Mosaic's own terms, so a saved chart draws the same in any Mosaic tool.

The principle, as Hugh put it: make the most of the frameworks Brightfield has chosen, and where Brightfield knows more about the data than a framework's defaults do, improve the representation. Brightfield is not bound to a framework's default decisions, but whatever it decides is written in a form the framework reads.

## What exists today

- **The tile generator** (`crates/brightfield-shell/src/dashboard.rs`). `role_of_label` maps a FineType label to Measure, Category or Neither, and `tile_for` picks a tile kind from that role and the DuckDB storage type: a histogram, counts per date (`time_bars_tile_sized`), ranked bars, or the point map. The label is used only for the role and for the comment above each tile. Generated tiles write no tick format and no grid setting.
- **Formats in a chart file.** `xTickFormat` / `yTickFormat` take a d3-format string for numbers and a d3-time-format string for dates. `crates/brightfield-spec/src/number_format.rs` and `date_format.rs` port both, in the English (US) locale only, so `$` is the only currency symbol. `crates/brightfield-render/src/axis.rs` applies a number format to linear, log and symlog axes and a date format to time axes and to a band whose categories are all ISO days. A band of anything else takes no format. With no format, an axis prints `format_number` (`axis.rs:695`): whole numbers ungrouped (2024, 12000), anything else at one decimal.
- **The inspector's axis settings** already have a **format** row that reads "auto" until the analyst types a d3-format specifier.
- **The grid** (`crates/brightfield-shell/src/data_grid.rs`) formats from Arrow storage type alone: doubles and decimals at two places, with the exact value kept for hover where rounding changed it; everything else as DuckDB's text; numeric columns right-aligned. It never sees a label.
- **The hover tooltip** (`HoverReadout` in `app.rs`, built in `chart_item.rs`) prints `column: value` from the engine's cell text.
- **Two open routes.** A lone CSV, TSV or Parquet goes straight to `data_file::open`: it is profiled with FineType (through the DuckDB extension's `ft_profile`), copied into memory when it fits the budget, and drawn. It is never cast, so a column written `$1,234.50` stays text and gets ranked bars or no tile. A folder, workbook or database goes through a Protocol (`own_files.rs`): load, profile, `datapackage.json` per source, the silver cast through FineType's vendored `transforms.json`, and, for a single sheet or table, a Parquet copy of the cast table reopened on the flat route. That reopen profiles the cast copy from scratch, so the cast currency column loses its currency label.
- **The column record.** The Protocol route writes `packages/<source>/datapackage.json` with each field's Frictionless type and FineType label as `x-finetype-label`. Nothing reads it back. There is no notion of an accepted label.

## Goals

1. **A file opens with its numbers and dates formatted from each column's type**, on tile axes, in the grid and in the tooltip, all from one decision per column so the three never disagree.
2. **The formats are written into the chart file** as `xTickFormat` / `yTickFormat`, axis titles, domains and grid settings, so another Mosaic tool draws the generated chart the way Brightfield does.
3. **A lone CSV is cast** on open, as a workbook sheet already is, so a currency or percentage column written as text becomes a number with a number axis.
4. **A column's type is remembered.** Its label is kept in the source's `datapackage.json` and read back, so a cast column keeps the label that says what it was.
5. **A year column draws as counts per year, in time order**, not as the ten busiest years ranked by count.
6. **Every unformatted number axis groups thousands.** Brightfield's own default changes from `12000` to `12,000`.
7. **A committed baseline reddens when a label's format changes.**

## Non-goals

- **A currency symbol other than `$` on axis ticks.** A Mosaic spec carries a d3-format string and no locale, and Observable Plot handles locale only through a format function, which a declarative spec cannot hold. Plot has open work on a top-level locale. Until Plot and Mosaic carry one, a sterling axis names its currency in the title. Following that work is tracked separately.
- **`B` for billions on ticks.** d3's `s` prints SI prefixes, so a billion is `G`. Brightfield scales in the query and names the unit in the title instead (below).
- **Accepting a label in the interface.** The record can hold an accepted label, and a reopen honours one, but the control an analyst uses to accept a label is separate work.
- **Setting a column preference in the interface.** The record's `prefer` is read and honoured; the control that writes it is separate work.
- **A view of stored values in the grid**, the counterpart of Excel's Show Formulas. Separate work; this design keeps the stored value reachable by hover in the meantime.
- **Changing chart furniture.** Gridlines, tick marks, tick labels and axis lines stay as they are drawn today. Hugh reviewed the current dashboard snapshots and they stand.
- **Changing the percentage cast or adding display formats to FineType's taxonomy.** Both are FineType changes this design depends on (see Dependencies). Brightfield vendors the result.

## Anti-goals

- **Misrepresenting a value.** No format invents a convention the file did not use, and no surface shows a number without a way to reach the stored one. A scaled axis states its unit.
- **A Brightfield-only format.** Nothing is written into a chart file that a Mosaic tool would read differently, such as a private locale attribute or a `G`-to-`B` substitution.
- **A second taxonomy.** Brightfield does not keep its own table of how each FineType label is shown. That lives in FineType.
- **Slowing the open.** The cast happens in the copy the flat route already makes.

## Dependencies

This design waits on two FineType changes, and Brightfield vendors FineType's release that carries them.

- **Display formats in the taxonomy.** Each FineType label definition that holds numbers or dates gains a display block: a d3-format specifier for numbers or a d3-time-format specifier for dates, plus what the specifier cannot say (the value is money, and in which currency when the label fixes one; the value is a fraction shown as a percentage). FineType exports the blocks as JSON keyed by label, beside `transforms.json`. Examples: `finance.currency.amount_accounting` → `($,.2f`, money; `datetime.component.year` → `d`; `finance.rate.yield` → `%`; `datetime.date.iso` → `%Y-%m-%d`.
- **A percentage casts to a fraction.** `representation.numeric.percentage` keeps 45 for "45%" today; `finance.rate.yield` already divides by 100. After the change every percentage-shaped label stores a fraction, as Excel does, and d3's `%` prints it correctly.

The exact JSON shape of the display export is FineType's to settle. This design needs, per label: an optional number specifier, an optional date specifier, whether the value is money, an optional fixed currency, whether the value is a fraction, and whether the value is a size in bytes.

Both changes are FineType work, made and released in FineType's repository; this design's plan covers Brightfield only. Its plan orders the work so that what needs no display block (the record, the cast, the reordered open, grouped thousands, the year tile's routing) lands first, and the step that vendors FineType's release with the display export comes before anything that reads it. Brightfield does not build against a guessed shape.

## Approaches

Three places were considered for deciding and keeping a column's representation.

- **A. Derive it on demand from the column record (chosen).** One rule takes a column's label and profile and returns its representation. The tile generator writes the result into the chart file; the grid and the tooltip call the same rule when they draw. Nothing new is stored, so a format cannot drift from the label it came from. An analyst's override lives where it does today, in the chart file's axis format. The cost: an override is per chart.
- **B. Store it in the column record.** `datapackage.json` gains a format beside the label, and every surface reads that. An override follows the column everywhere, but it adds a Brightfield-specific property holding a value derivable from the label, which can disagree with the label after it changes.
- **C. Store it in the Ossie model.** A semantic layer is a natural home for display formats, but a lone CSV has no model file, and this would make the flat route depend on one.

Hugh chose A, on the condition that Brightfield uses FineType instead of rebuilding it. That condition moved the label-dependent half of the rule into FineType's taxonomy (Dependencies), so the rule splits by who knows what:

- **FineType knows what a label implies**: money with two decimals and parenthesised negatives, a fraction shown as a percent, a year with no grouping.
- **Brightfield knows what only the data shows**: the scale (£bn), the precision the spread needs (`.1%` versus `.0%`), the run of years, and the currency symbol a particular file used.
- **The analyst's preference, where one is recorded, wins over both.** This keeps B's one advantage, a column format that follows the column everywhere, as an explicit choice the analyst makes, not a stored copy of the default.

## The design

### 1. The column record

The source's `datapackage.json` is the record of what the source taught Brightfield, using properties Frictionless and FineType already define.

- **Inferred**: a field carries `x-finetype-label`, as today.
- **Accepted**: the field also carries `x-finetype-nominated: true`, the marker FineType's own datapackage emitter writes for a nomination. A nominated label is taken as given: on the next open Brightfield does not ask `ft_profile` about that column and uses the recorded label. Nothing in this design writes the marker except tests; the control that does is separate work.
- **How the file wrote it**: a currency column's field carries what the cast strips and a label may not say: the symbol the non-null values share (`£`; none when they use more than one), and whether its negatives are written in parentheses. FineType's `finance.currency.amount` accepts both `(…)` and `-`, so the label alone cannot say which this file used. Both are read from the raw values when the column is cast. *Hugh's call: they live on the field, in Brightfield's own property (below).*
- **Brightfield's property.** Everything Brightfield adds to a field sits in one object, `x-brightfield`, so Brightfield's additions stay in one namespace beside FineType's `x-finetype-*`:

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

  `written` holds how the source wrote the column. `prefer` holds an analyst's preferred representation for the column, over FineType's default, so a preference follows the column into every chart, the grid and the tooltip. *Hugh's call: analysts have preferences beyond FineType's defaults and they need a home.* The representation rule reads `prefer` first. This design reads it and tests it; the control that sets it is separate work.
- **What belongs in the package.** Facts and preferences about the source and its columns: how it is written, its accepted labels, an analyst's column formats. What belongs to a view (a chart's layout, a dashboard's choices, a grid's column widths) stays in the chart file or the workspace. *My call, to keep `datapackage.json` from becoming Brightfield's settings file, which Hugh raised as a risk.*
- **Read back on open.** Every open reads the record before profiling. A single sheet's cast copy takes its labels from the record, not from a fresh profile, so a cast currency column stays a currency column.
- **A lone CSV's record** lives in Brightfield's own data folder, beside the own-files Protocol folders, keyed by the file's absolute path, not beside the analyst's file. *My call: Brightfield does not write into folders the analyst did not ask it to write to.*

### 2. Casting a lone CSV

The flat route's in-memory copy (`Session::materialise_source`) selects each column through its label's cast from the vendored `transforms.json`, instead of copying it as read. It already runs after the profile, so the labels are known.

- **The open is reordered** so the copy runs before the dashboard is chosen: profile, copy with casts, then one aggregate over the in-memory copy for each cast column (minimum, maximum and the spread the rule needs), then `Dashboard::of`. The aggregate scans memory, not the file. A file over the budget is profiled and drawn as today. *My call: this is the cheapest order that gives the rule the cast values' range; re-profiling the copy would repeat the classifier.*
- **A cast copy is not cast again.** A single sheet's cast copy, reopened on the flat route, is opened as already cast, so a rate is not divided by 100 twice. The Protocol route's silver cast keeps its own rule (a value that will not cast becomes NULL); aligning the two routes is out of scope. *My call.*
- **When a column is cast**: its label is trusted (not `Unusable`), FineType's own projection casts it (the rule the silver cast already follows in `cast_expr`), and every non-null value casts. Twelve currency and rate labels are typed VARCHAR in FineType's export although their transforms return numbers, so FineType's projection passes them through and they stay text until FineType corrects their type. Brightfield follows FineType here rather than second-guessing it. Otherwise it is copied as read. *My call: "every non-null value" is the high-confidence bar Hugh asked for; a cast that would turn some values into NULL is not made.*
- **Reversible**: the copy keeps each cast column's raw text beside it under a reserved name the grid does not list. Undoing a cast is copying again without it.
- **A file over the memory budget**, which opens off the view without a copy, is not cast. Its tiles behave as today.

### 3. The representation rule

One function in the shell takes a column (its label, the display block FineType gives that label, its record and its profile) and returns a representation:

- **tick format**: a d3-format or d3-time-format string for the chart file;
- **scale**: a divisor applied in the tile's query, with its unit word (`bn`, `m`, `k`, `MiB`), or none;
- **unit for the title**: the currency symbol, the scale, or both: `(£bn)`, `(%)`, `(MiB)`;
- **cell format**: the format the grid and the tooltip print a single value with.

How the data refines FineType's block:

- **Scale.** A money column, or a size in bytes, whose profiled magnitude reaches a thousand or more is divided by 10³, 10⁶ or 10⁹, chosen so most ticks have one or two leading digits. The axis is titled with the unit. File sizes use binary units (2²⁰ → `MiB`), which d3-format cannot express as a format. Other numbers are not scaled. *My call: the thresholds; the builder sets them so the fixture's ticks read naturally.*
- **Where the division lives.** In the tile's bin, as an expression: Mosaic's `bin` transform takes "a data column or expression" (`Transform.ts` in Mosaic's spec), so a Mosaic tool bins `revenue / 1e9` the same way, and a brush over it publishes a range in the same units its predicate tests. Brightfield's spec parser takes only a bare column name in `bin` today (`bin_transform` in `crates/brightfield-spec/src/parse.rs`), so it learns the expression form, in the spelling Mosaic's spec uses. The first task verifies that spelling against Mosaic's spec types before anything is written to depend on it.
- **Precision.** A percentage's precision comes from its profiled spread: the fewest decimals at which the ticks are distinct (`.0%` for 0–80%, `.1%` for 4.1–4.6%). Money keeps two decimals in cells; on axes it keeps the decimals the ticks need.
- **The tooltip** prints one form, compact, matching the axis: `revenue: £2.4 bn`, with a no-break space (U+00A0) between the number and the unit word so it never wraps. A percent sign stays attached (`45%`). The exact value is in the grid. *Hugh's call.*
- **The grid** formats only columns whose label has a display block; an unlabelled column, or one whose label has none, keeps today's display. That keeps an unlabelled identifier from being grouped as `1,234,567`. *My call.* A labelled column prints full values in the file's convention: `1,234.50`, `(1,234.50)` for an accounting column, `45%`. The header carries the unit (`revenue · £`, `share · %`). Hovering a formatted cell shows the stored value, extending what the grid already does for a rounded double.
- **Only `$` reaches a tick.** A dollar column may write `$` in its tick format. Any other currency writes the format without a symbol and names the symbol in the title. The grid and tooltip print the recorded symbol, since they are Brightfield's own surfaces.

### 4. What a generated tile writes

- `xTickFormat` / `yTickFormat` from the rule, on the axis the column is drawn on. The count axis on a histogram or dates tile writes nothing: Plot's default already groups thousands, and so will Brightfield's (section 6).
- The axis title with its unit, when the rule gives one: `xLabel: "revenue (£bn)"`.
- A scaled column's query divides by the scale, so the bins and ticks are in the unit the title names.
- `xGrid` / `yGrid` as Brightfield draws them today, because Plot draws no gridlines by default and the file should draw the same everywhere. *My call, following Hugh's ruling that a saved chart names Brightfield's default wherever Mosaic's would draw differently.*
- The comment above each tile adds which label chose the format and the scale, beside the existing sentence on which label chose the tile.

### 5. Years

A `datetime.component.year` column draws as counts per year in time order, using the existing counts-per-date tile in place of ranked bars, whatever the category ceiling says.

- The tile writes `xTickFormat: d`, because Plot's default number format groups thousands and would print 2,024.
- The tile writes the full run of years from the profiled minimum to maximum as its x domain, so a year with no rows shows as an empty slot, not as a missing one. Above a ceiling on the run's length, the domain is left to the data. *My call: the ceiling; the builder picks one that keeps a slot wide enough to see.*
- The renderer applies a number format to a band whose categories are all whole numbers, and sorts those categories numerically.

### 6. Brightfield's own default number format

`format_number` groups thousands: `12,000`, `1,234.5`. Plot's default does the same, so an unformatted axis now reads the same in Brightfield and in Mosaic. This moves most committed PNG snapshots once, in a commit that changes nothing else.

### 7. The inspector

The axis **format** row's "auto" now means "chosen from the column's type", and shows the specifier the rule chose, so the analyst can see it and type over it. A typed format is written to the chart file as today and is no longer "auto".

The file holds a specifier either way, and no marker says which kind it is. The inspector shows "auto" when the file's specifier equals what the rule gives now. The generated dashboard is generated again on each open, so it always follows the current label. A chart the analyst has saved keeps the specifier it was saved with: a label changing later shows as that row no longer reading "auto", and nothing rewrites a saved file. *My call: a saved file is the analyst's document, and a writer-only marker would be a Brightfield-only attribute.*

## Testing

- **The baseline that reddens**: a fixture CSV with a currency column (accounting negatives, `£`), a dollar column, a percentage column, a rate column, a year column, a large count column and a date column. Its generated dashboard YAML is committed as a snapshot; a change to any label's format changes the YAML and fails the test. Committed light and dark PNGs of the same dashboard, and of its grid, catch the drawing.
- **The rule**: unit tests per label family against FineType's vendored display blocks: scale thresholds, precision from spread, the title unit, the dollar-only tick symbol.
- **The record**: a reopened single sheet keeps its currency label; a nominated label skips `ft_profile`; a column with mixed symbols records none; a recorded `prefer` format wins over FineType's default on the axis, in the grid and in the tooltip.
- **The cast**: a column with one value that will not cast is copied as read; a cast column's raw text survives; a file over the budget is not cast.
- **Portability**: the generated YAML parses with no warning from Brightfield's own spec reader, and its formats are legal d3-format and d3-time-format specifiers.

## Left to the builder

- The reserved name for a cast column's raw text, and how the grid hides it.
- The scale thresholds and the year-run ceiling (within the rules above).
- How the representation rule is exposed to the grid and the tooltip, which live in different modules.
- How the record is keyed for a lone CSV (path plus size and modification time is one option), and what happens when the file moves.
- The order of work, provided the column record and cast land before representation, and the percentage format lands last since it waits on FineType's cast change.
