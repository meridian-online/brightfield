# The inspector

The right rail is one inspector. It describes the selection in titled, collapsible sections stacked top to bottom, rather than behind a strip of three tabs, one of which the reader had to guess. Read this before changing what the rail draws or which section opens when.

## Its sections

`crate::inspector::sections_for` chooses the sections from what the window holds, most specific first:

1. **Operation**, while a foot field or an operation form is asking. Its body is the operation panel (`operation_panel.rs`). A key that asks a field, or a form that opens, opens this section at the top of the rail, opening the rail if it was shut; Esc, or keeping or dropping the operation, puts the rail and the section back as they stood.
2. **Column**, while a column is selected or the canvas holds a table a later step makes. Its body is the chart's inspector pane (`inspector.rs`), which describes the column or the table.
3. **Parameters**, for a chart with nothing selected. Its body is the same pane, which names the focused pane and draws the chart's sliders.
4. **Step**, always, and always last. Its body is the operator's facts for the focused node (`protocol.rs`). It is open only while no section above it is, so the node's facts give way to whatever is more specific; opening it folds the sections above.

The chart's Encoding, Reads and Fields are named in `Section` so that their place in the order is fixed, and are listed once something draws them. Until then the chart's pane draws its sliders under the Column section when a column is selected.

Every other section is open until the reader closes it. What the reader opened and closed is session state, as which pane a rail showed was, and goes back to the defaults when the window adopts another document.

## How it is drawn

The rail declares one pane, `inspector`, so its strip names it and offers no choice. The window draws each section with the design system's `meridian_egui::section` header, and under it the body of the pane that drew it when the rail held three, through `PaneChrome::body_ui`: the pane's own drawing, empty state and focus on a press, with no frame and no header band, since the section header already names it. The three panes' ids (`protocol-inspector`, `chart-controls`, `operation-inspector`) stay in their documents' registries and stay published, so a layout saved when the rail held three panes loads whole and opens on the one inspector.

Each open body may grow until the headers below it would leave the rail, so every header stays where a click can reach it. The operation panel and the operator's facts scroll inside that room.

## Its key

⌘⌥B (`toggle-inspector-rail`, in the Window menu) hides the inspector and brings it back, over an open Protocol, as ⌘B moves to the spine on the left. The chart's own rail toggle (`toggle-controls-rail`) is gone, since there is no longer a chart rail apart from the inspector.
