# `o` organises a Protocol's joins

## Intent

The Protocol context's key table gives `o` the job of organising a Protocol's joins with dovetail: a canvas over the Protocol's own tables, with the joins the model holds, where the analyst ticks the ones to keep and unticks the ones to let go, and applies the lot with one key. Every other key in that table is built; `o` is not, and the palette has no row for it. The sources browser already has a canvas of this shape for a folder being added, and the model pane already has Keep and Drop. This change puts the two together for a Protocol that already has its sources.

## Goals

- `o`, in the Protocol context, opens an organise sheet over a Protocol whose model has two tables or more. With fewer, it opens nothing and a toast says the Protocol needs two sources to have joins to organise.
- The sheet draws the Protocol's tables and one curve per join in the model between two of them, with a row per join. A join the analyst has decided on is ticked and reads "kept"; one dovetail proposes and nobody has decided on is unticked and reads "proposed".
- `j` and `k` move between joins, `Space` (or a click on a row) ticks or unticks the join under the cursor. `Enter` applies and closes. `Esc`, or a click on the backdrop, closes and changes nothing.
- Apply turns the difference into the model pane's own edits, one per changed join: a proposal ticked becomes Keep, a kept join unticked becomes Drop. Joins left as they opened are not touched. Each edit goes through the window's `perform_model_action`, as the browser's add already does, so it is logged and undoable like the same edit made in the model pane.
- An edit the model refuses (Drop on the only join folding one table into another) does not stop the others, and is reported as the model pane reports the same refusal: `perform_model_action` already turns each dispatch's outcome into the window's report.
- The palette lists the verb, and its sweep test proves that choosing it opens the sheet over a Protocol of two tables. The keymap research table scores it.

## Non-goals

- No new edit. The sheet only Keeps and Drops; it does not add a join, change a join's keys, or bring back one the analyst dropped before (those stay in the model's rejected list and do not appear on the sheet).
- No new discovery run. The sheet shows the joins the model holds now; it does not ask dovetail for more.
- No change to the sources browser's own canvas behaviour, beyond its drawing moving into a function the two sheets share.

## Rulings taken while Hugh was away

These were the two calls the work left open. Both can be undone without changing what else gets built.

1. **Enter drops only the joins the analyst unticked, not every unticked one.** Unticked is the state a dovetail proposal opens in, so dropping every unticked join would reject each proposal the analyst merely looked past, and a rejected join is kept out of later discovery. Applying only the difference matches what the browser's add does over a folder (joins left unticked stay dovetail's proposals) and keeps the rule that a sheet opened and closed with no keys changes nothing. Rejecting a proposal outright stays the model pane's Drop.
2. **It is a sheet of its own, not a mode of the sources browser.** The browser's canvas is the preview of the folder under its list's cursor, keyed by that folder's path, and a Protocol's tables can come from several folders and single files. A mode would need a folder that does not exist and would carry the places, list, glob and add keys that mean nothing here. The new sheet reuses the canvas drawing and the browser's key verbs instead.

## Approaches considered

- **A sheet of its own sharing the browser's canvas drawing and key verbs** (chosen). Small state of its own (the joins, each with how it opened and how it stands, and a cursor); the drawing is the browser's, moved into a shared function; the keys resolve in the existing Browser context, so `j`, `k`, `Space`, `Enter`, `Esc` need no new bindings and stay where the browser has them.
- **A mode of the sources browser**, opened on a synthetic "Protocol" entry. Reuses the most code, but ties the Protocol's joins to a folder path and leaves the browser's other columns live and meaningless. Ruled out above.
- **Reuse the browser's existing "made of" path** (the browser opened on a folder just added, its model joins on the canvas). That path only keeps ticked joins and only covers one folder's tables; widening it to drops and to every table is the first approach with a folder in the way.

## Design

- **Key and verb.** A new registry verb, `organise-joins`, bound to `o` in the Protocol context, a View verb (opening the sheet writes nothing; the edits it applies are the model's own), driven by the palette's meta path as `open-sources-browser` is. `o` is free there: the grid's `o` is the Grid context's.
- **Dispatch.** The Protocol panel answers the verb by raising a request the window takes, as it does for `A`; the window's verb dispatch answers it too, so the palette and the key reach the same function. That function reads the window's model, lists its relationships between tables of the Protocol, and opens the sheet, or says why it cannot.
- **Rows.** Each row is the relationship's name, its tables, and a label in the browser's form (`orders.customer_id → customers.id`, with every key column on each side joined by commas, so a join on two columns is shown and not left out). Ticked when `decided_by` is the analyst.
- **Shared drawing.** The table boxes, curves and rows of the browser's canvas move into one function that takes the rows to draw, the cursor and whether the canvas holds the keys, and returns the rows' rects, their spoken labels and the row clicked. The browser's canvas and the organise sheet both call it.
- **Keys.** The sheet resolves keystrokes in the Browser context and answers its verbs as the browser's canvas does: move down and up, mark-or-accept toggles, go-in-or-add and add apply (so `l`, `Enter` and `a` apply, as they add on the browser's canvas), close closes. Every other Browser verb does nothing on this sheet.
- **Apply.** On apply the window takes the changed joins, performs Keep for each newly ticked one and Drop for each newly unticked one, in the sheet's row order, and closes the sheet.
- **Test hooks.** The window names the open overlay `organise-joins` and exposes the sheet's rows (label, ticked) for tests, as it does for the sources browser.

## Left to the builder

- Where the sheet's state and view live (a module beside `sources_browser_view.rs`, by the look of the code).
- Its size and frame: the sources browser's scrim and frame, sized for the canvas alone.
- The exact wording of the toast for a Protocol of fewer than two tables.
- Which fixture the window tests use; the retail extract fixture already has kept joins and one dovetail proposal waiting on the analyst.

## Verification

- A window test over a Protocol of two tables or more: `o` opens the sheet with the model's joins, ticked as decided; `Space` on a proposal and `Enter` leave that join decided by the analyst; `Space` on a kept join and `Enter` move it to the model's rejected joins; `Esc` after toggles changes nothing.
- A window test that `o` over a Protocol of one table opens no sheet and says why.
- The palette sweep's arm for `organise-joins`.
- A keymap research table row, which the keys crate's tests check against the registry.
- The browser's existing canvas tests still pass after the drawing moves.
