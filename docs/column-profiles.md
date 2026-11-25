# Column profiles of any table

The window's session holds one table: the one the data file opened as. Its columns are profiled when the file opens, and the dashboard's tiles and the grid's column header band read that profile. A table that a later step makes, such as a filter or a limit recorded through arc, has no profile until something asks for one. This note says how such a table is profiled, and what the inspector says about it.

## What is measured

A column's moments are its mean, median, upper quartile, sample deviation, exact distinct count, bounds and counted shape. The upper quartile is `quantile_cont(col, 0.75)`, which interpolates as DuckDB's median does, so 1 to 100 gives 75.25. The chrome spec asks for the "upper quantile", which is not a defined term; this reads it as the upper quartile.

The profile pass (`Profiler` in `brightfield-engine`) runs on any DuckDB connection. A session profiles its own views through it, labelled by its type source and counted by its scan tally. A preview profiles a step's table on its own connection through the same pass (`preview_profile_stoppable`), so the two cannot drift apart in what they measure.

## How a step's table is read

The read is a preview's. The data file is loaded on a fresh in-memory connection, the steps that make the table are run on it, and the table is profiled there. It is held to the preview's size, memory and time limits, and nothing reaches the window's session. A refusal comes back in the same words a preview page's would: the deadline, the stop, or running out of memory.

No type source rides the preview, so a step table's column has no semantic label. Its "finetype" line shows its storage type until the type source can be asked on a preview connection.

The window keeps up to eight profiles (`ColumnProfiles`), each against the table's name, the content hash the run record gives it, and the steps it is made through. A run that changes the table changes its hash, and an edit to one of its steps changes the steps, so either reads the table again. A read that ran out of time is said to have, and is read again on the next fresh ask: the canvas coming to the table again, or a caller asking for it by name. The inspector's own asks on each frame it draws the table are not fresh, so a table whose profile keeps running out of time is not read again behind the reader's back. A refusal that asking again cannot change, such as DuckDB rejecting a statement, is kept.

## What the inspector says

When the canvas holds a table a later step makes, the inspector describes that table. It shows the table's name; the step that makes it, with its kind and its state in the steps sheet's words (`not run` until a run records it); and its columns.

- The step and state come from the table's own node in the Protocol's model, not from the spine's selection, and not from the load.
- The inspector asks for the columns as it draws the table, and the window starts the read only then. A table on the canvas with the inspector shut, or showing another pane, is not read.
- A read the inspector started is stopped when the canvas moves off the table, so the grid's own reads of the next table do not wait behind it. The preview gate runs one read at a time.
- While the columns are being read, the inspector says "Reading the columns of *table*…" rather than drawing an empty list.

The column's full sections (what it is, its values and what can be done with it) are drawn on top of this profile by a later change.
