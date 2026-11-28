# The open places

The navigator rail lists the places the reader has been, above the spine, the way a browser lists its tabs down the side. A place is one node in one activity: a table's grid, the chart a dashboard draws, or the Protocol's graph. Read this before changing what counts as a place, when one is added or closed, or the place keys.

## What a place is

`crate::spine::Place` is read off the canvas's latch, `CanvasHolds`, so a click on a place puts back exactly what the canvas held:

- `Place::Graph` is the asset graph. It names no one node, so its tab reads `Protocol · Graph`.
- `Place::Grid(node)` is a table or view listed as rows, `<node> · Grid`.
- `Place::Chart { node, table }` is a dashboard drawn as its chart, `<node> · Chart`. Its tab carries the unsaved-draft dot while the chart document holds unsaved work.

A bare chart opened with no Protocol behind it, and the Protocol's panel tiles, are no place, since neither is a node of the Protocol that a click could put back. The model pane shows no places either. It swaps the window's documents to open a table's grid, so a place there could not be restored through the latch, and its spine, drawn in the navigator's place, has no list above it.

## When places are added and closed

`MeridianApp::sync_places` runs at the head of each frame, straight after the canvas's latch is reconciled. It closes a place whose node the documents no longer have, then opens the place the canvas shows: a new place goes at the bottom of the list, and one already open becomes current again. So however the canvas moved (a spine row, an activity key, the graph chip, quick open), the place appears once.

Leaving the Protocol closes every place, whether for the front door or for another document opened over it (`MeridianApp::documents_changed`). The places are not kept between sessions.

Closing the current place makes the one above it current, or the one below when it was the top, and puts that place on the canvas. The last place is never closed, because the canvas always shows something: ⌘W on it leaves it, and the window stays.

## How they are drawn

The navigator's pane draws the places first, then the spine below them in its own scroll area, so the places stay put while the spine scrolls. The places take at most a third of the pane (`PLACES_MAX_SHARE`) and scroll in their own area past that, so however many tables the reader has visited, the spine keeps two thirds of the pane. When the current place changes, its tab is scrolled into view; it is not held there on every frame, which would undo the reader's own scroll. Each is the design system's `meridian_egui::vertical_tab`: the current place is the cell joined to the canvas, and its close control shows on the current place and under the pointer. The pane reports a click or a close as `PlaceAsk`, and the window, which owns what the canvas holds, acts on it, as it does a click on a spine row.

## Their keys

All three are global and sit in the Window menu. They need an open Protocol with its places drawn, which is the `PlacesShown` condition: while the model pane or the panel tiles hold the canvas the list is not shown, so the menu greys the three, and a key says why rather than walk or close a list the reader cannot see.

| Verb | Key | Does |
|---|---|---|
| `next-place` | ⇧⌘] | the place below, the top after the bottom |
| `prev-place` | ⇧⌘[ | the place above, the bottom before the top |
| `close-place` | ⌘W | closes the current place, never the last |

A US layout's Shift turns a bracket into a brace, and egui reports the logical key, so the window takes either spelling for the bracket chords.
