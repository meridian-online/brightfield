# The Protocol's spine beside the model

## Intent

A Protocol of two or more sources opens on the model pane. Today that pane takes the whole canvas region in place of the dock, so no rail is drawn and the Protocol's spine, which the opening-and-saving work orders model-first, never shows on a Protocol opened, made or added to in the app. It shows only where a Protocol's manifest is loaded with a model file beside it, as an agent Protocol's folder is.

The first screen is meant to read as a workflow: the spine on the left is the Protocol, and what is selected on it occupies the canvas. With two or more sources the model step is selected and the canvas shows the model; selecting a source's table on the spine shows that table's grid and dashboard, with the spine still the Protocol's and a way back to the model. `MeridianApp::select_source` does the second half already, but no spine row calls it.

## Goals

1. While the model pane is on the canvas, the navigator rail is drawn to its left, as it is beside the dock, holding the Protocol's spine: the joined tables first, the model step, then the heading over the sources with no join, with one join curve per join in the gutter.
2. The spine is built from the Protocol's manifest with its newest run record adopted, as an agent Protocol's rails are, and with the joins of the pane's model as it is in memory (`ModelJoins::of`), not as last read from disk.
3. The model step's row is drawn selected while the model is on the canvas.
4. A click on a source table's row calls `select_source` for that table: its rows open in the grid with its dashboard, and the model waits behind them.
5. While a source table's rows are in the grid, the navigator rail's spine is still the Protocol's, with that table's row selected. A click on the model step's row goes back to the model. A click on another source table's row goes back to the model and opens that table's rows instead. The existing way back (the locator's crumb) still works.
6. The spine is rebuilt when the model changes (a join accepted or dropped, a hand edit to the file read, a full discovery pass landing), when a run started from the pane lands, and when a source is added (which already reopens the pane).
7. The rail collapses and reopens as it does beside the dock, under the same panel ids, so its dragged width carries between the two.

## Non-goals

- Keyboard navigation of the spine beside the model pane. The spine's bare-key grammar is fed only while the graph is on the canvas, and that stays so.
- The ledger and inspector rails beside the model pane. The model pane has its own inspector; the ledger's run record for a multi-source Protocol is separate work.
- The locator band beside the model pane.
- The column outline under the spine while the model is on the canvas. It lists one table's columns, and no single table is selected then.
- Changing the spine's ordering rule, its row drawing, or the join curves' routing.

## Anti-goals

- Two derivations of the spine. The rail beside the model and the rail beside the dock draw through one function from one `Spine` value, and the test hooks (`spine_rows`, `spine_curves`) read what was drawn, not a second derivation.
- A spine that goes stale: a join accepted on the diagram that does not move the tables on the rail on the next frame is a failure even if every row draws.
- Losing the grid's own document while the Protocol's spine is shown over it: the grid, its dashboard and its Save behave exactly as they do now.

## Constraints

- `window.rs` is very large; new code goes beside what it extends, and the spine list drawing is extracted from `OutlinePane::ui` into one function both rails call, not copied.
- The headless harness is how the change is verified; the render baselines must stay byte-identical except where a frame now draws the rail beside the model pane, and any baseline that changes is regenerated deliberately and named in the pull request.
- One cargo build at a time on Hugh's Mac.

## Approaches

1. **A Protocol spine held by the model pane's document, drawn by a shared spine-list function (chosen).** `ModelDoc` keeps a `Spine` and its caption, built from `agent_protocol_inputs(dir)` with `model_joins` set from the pane's model, and rebuilt when the pane's generation or its last run changes. The model branch of the central draw draws the navigator rail with that spine. While a table is in the grid, the window hands the same spine to the Protocol document for the frame, and `OutlinePane` draws it in place of the opened file's spine, with clicks returned as a request the window drains, as `browser_ask` is. Trades a small override path in `OutlinePane` for leaving the grid's own document untouched.
2. **Route the model pane through the dock as one more canvas occupant.** The rails would then draw beside it for free. Rejected: the dock's canvas branch reads the chart and Protocol documents throughout (the grid's spot, the hero, the view chips), and the model pane is neither; making it an occupant touches most of the central draw for a rail.
3. **Keep the Protocol's manifest loaded as the window's Protocol document while the model is open, instead of `Boot::empty()`.** The navigator would read it naturally. Rejected: once a table's rows open in the grid, `adopt_boot(Boot::of_opened_file(..))` replaces that document with the opened file's, which the grid needs, so the Protocol's spine would still need holding somewhere else; and a non-empty Protocol document behind the model pane changes what the graph latch, the run control and the key grammar think is open.

## Decided

- The table rows a click acts on are the asset rows whose label is one of the model's tables. Clicks on step rows, file rows and the heading do nothing beside the model pane (they select an asset on a graph that is not on the canvas).
- While a table is in the grid, the head row's graph chip is a readout, not a control, since the Protocol's graph is not what it would put on the canvas.
- The column outline under the spine while a table is in the grid stays the opened table's, as now.
- `ModelDoc` keeps which table the grid holds, as the model's table name, alongside the words it already keeps for the crumb.

- Every route into the model pane (a folder, workbook or database opened, a Protocol made or added to) goes through `own_files`, which writes the Protocol's `arcform.yaml` with one load step per table before the pane opens, so a manifest is always on disk when the spine is first built.
- Which group a source falls in is decided by `model_joins` alone: `model_first` matches each of the model's tables to the spine unit whose label equals it, and the manifest's graph lists every source table whether or not it is joined. Overriding `model_joins` with the in-memory model is enough to move a table between groups.
- A table row's label is the table's name exactly as the model names it; that same exact match is what `model_first` already relies on, so the click handler uses it too.
- The rows are marked selected by the window, not by the Protocol model's own selection: only the model step's row while the model shows, only the grid's table's row while a table is in the grid.
- A run does not rewrite the manifest; the rebuild re-reads it anyway, since that is the one loader, and adopts the newest run record.

## Left to the builder

- The exact rebuild key (generation plus some identity of the last run).
- Whether the shared function takes a closure for "is this row selected" or the rows arrive pre-marked.
- Where the spine's caption comes from when the manifest cannot be read (an empty spine is acceptable; the model pane still draws).

## Verification

A headless window over a folder of joined tables (the joined-tables fixtures the model pane's tests already use) opens on the model pane, and:

- `spine_rows` lists the joined tables, then the model step, then the no-join heading and its sources, drawn left of the model pane's rect; `spine_curves` has one curve per join; the model step's row is selected.
- A click on a table's row puts that table's grid on the canvas (`model_behind_grid`), and `spine_rows` lists the same rows in the same order, with that table's row selected.
- A click on the model step's row from there brings the model pane back.
- Accepting a proposed join on the model pane moves the rows on the next frame and adds a curve.
