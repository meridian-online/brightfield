# The inspector

The right rail is one inspector. It describes the selection in titled, collapsible sections stacked top to bottom, rather than behind a strip of three tabs, one of which the reader had to guess. Read this before changing what the rail draws or which section opens when.

## Its sections

`crate::inspector::sections_for` chooses the sections from what the window holds, most specific first:

1. **Operation**, while a foot field or an operation form is asking. Its body is the operation panel (`operation_panel.rs`). A key that asks a field, or a form that opens, opens this section at the top of the rail, opening the rail if it was shut; Esc, or keeping or dropping the operation, puts the rail and the section back as they stood.
2. **Column**, in a window with a chart, while a column is selected or the canvas holds a table a later step makes. Its body is the chart's inspector pane (`inspector.rs`), which describes the column or the table. A window with no chart draws no Column section, as the chart's pane drew only its empty state there before the rail was one inspector.
3. For a chart, always, in this order:
   - **Encoding**: the mark and each of the shelf's channels with what is on it, one `meridian_egui::menu_item` row each, carrying the key of the shelf's cell for that channel (`shelf::cell_key`). A click on a row opens that cell as its key does: the chart's pane is focused, the shelf takes the keys and the cell's list opens. The shelf in the chart pane stays the place the encoding is edited; this section reads it and points at it. The window opens the cell after the frame's requests, because the click also asks to focus the inspector.
   - **Reads**: the table the chart reads and the rows it draws, as `ranking · 19,028 of 19,028 rows`. The table is named as the Protocol names it where the chart's columns belong to one, and by the spec's `data:` name otherwise. Where the spec draws a faded layer of every row under the selected ones, the window has already counted both layers, and the line says how many of the table's rows are drawn. Otherwise it gives the presenting mark's count alone (`points · 6 rows`), counted once per change of the session rather than once a frame. The total for that case would need a count of the source table, which the engine does not offer the shell.
   - **Fields**: the columns the shelf's list offers, in the table's order, each with its semantic type's leaf, and their count in the header.
   - **Parameters**: what the focused pane offers when no column is selected, and the chart's sliders.
4. **Step**, always, and always last. Its body is the operator's facts for the focused node (`protocol.rs`). It is open only while no section above it is, so the node's facts give way to whatever is more specific; opening it folds the sections above.

One pane draws the Column, Encoding, Reads, Fields and Parameters bodies, so the window tells it through `SectionHandle` which section it is drawing before each body, and reads back what the chart's sections drew and an Encoding row's click. A chart drawn with no live session, as a still picture is, says so in Encoding and Reads rather than claiming it has no mark or no table.

Every other section is open until the reader closes it. What the reader opened and closed is session state, as which pane a rail showed was, and goes back to the defaults when the window adopts another document.

## How it is drawn

The rail declares one pane, `inspector`, so its strip names it and offers no choice. The window draws each section with the design system's `meridian_egui::section` header, and under it the body of the pane that drew it when the rail held three, through `PaneChrome::body_ui`: the pane's own drawing, empty state and focus on a press, with no frame and no header band, since the section header already names it. The three panes' ids (`protocol-inspector`, `chart-controls`, `operation-inspector`) stay in their documents' registries and stay published, so a layout saved when the rail held three panes loads whole and opens on the one inspector.

Each open body scrolls within the room the headers below it leave, so those headers stay in the rail where a click can reach them. That room also leaves each open section below its body's inset above and below, which it keeps however little room its body is given, so a chart's seven sections keep the Step header in a short rail.

## Its key

⌘⌥B (`toggle-inspector-rail`, in the Window menu) hides the inspector and brings it back, over an open Protocol, as ⌘B moves to the spine on the left. The chart's own rail toggle (`toggle-controls-rail`) is gone, since there is no longer a chart rail apart from the inspector.
