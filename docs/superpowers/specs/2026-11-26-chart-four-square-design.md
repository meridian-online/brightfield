# A new chart starts from the suggestion four-square, and a chart is a node of its own

Status: design, for Hugh's approval. Date: 2026-11-26.

## Intent

The Chart activity is greyed over any table or view no chart reads, with *There is no chart to show for this node yet.* That is the node a new analyst most wants a chart of: the result of their first filter or summary. Today the only chart the window can show is the held table's dashboard: one the Protocol's run contract names, or else the one generated for the table the file opened as.

Hugh's direction, in his words: "the starter dashboard is useful but it does assume a lot". A new chart opens the chart editor, with three suggested charts and one suggested dashboard in a 2×2 in the empty chart area, "a bit like the blank space under 'Sheet 1'" in Tableau. That 2×2 is the **suggestion four-square**. It shows "any time someone wants to create a new chart - regardless of if one already exists".

Behind it sits a ruling on what a chart is. **A chart is a node of the Protocol graph with its own file**, a Mosaic spec holding one plot. **A dashboard is a node that arranges charts by name**, and may also hold plots written into its own file. Charts are kept in `charts/`, dashboards in `dashboards/`, and `panels/` is kept free for the Panel, a dashboard that ends in an action. Hugh: "Yes, that matches how I see analysts working", and on the folders, "Yes, that split makes sense."

## Goals

1. **Chart works over any table or view node**:
   - No chart or dashboard reads the node: Chart opens the chart editor on a new draft, showing the four-square.
   - One reads it: Chart opens it, as today.
   - Several read it: Chart asks which, in a chooser that lists them by name and kind, with *New chart* as its last row.
2. **A New chart command** opens the editor and four-square over the table or view in focus, whether or not anything reads it.
3. **The four-square is drawn live from the node's rows**. Cells 1 to 3 are the first three chart kinds `chart_kinds::registry()` finds applicable to the node's columns, in its preference order, each bound as the first-look chooser would bind it. Cell 4 is the dashboard generated for the node, drawn small.
4. **Picking a cell starts a draft**. A chart cell makes a draft chart node: its plot is put on the shelf and the four-square gives way to it. The dashboard cell makes a draft dashboard node. The shelf stays live above the four-square throughout, so the analyst can build a chart from columns without picking anything.
5. **A draft is unsaved** (the pane reads Edited) until Cmd+S, which writes `charts/<name>.yaml` or `dashboards/<name>.yaml` in the Protocol's folder and adds the node to the graph and the spine.
6. **Leaving an unsaved draft asks first**, through the existing close question. Leaving includes moving to another node, opening another chart and closing the window.
7. **The window moves to the node in focus** (approach A, below). Chart or Grid over a node the window does not hold makes that node the held one. Its rows come from the Protocol's last run. When no run reached the node, Brightfield runs the steps up to it on its own, as a preview run (Hugh, 2026-11-26), with one exception: when a step it needs would fetch from the network, it reads only the first 1,000 rows of that source, without asking, and the pane says the rows are a sample with a control to run in full. Hugh: "fetch 1,000 rows instead of showing nothing? That's what we'd do with a grid view". Today that case draws *No rows for <node>*.
8. **Older Protocols still open as they do today.** A Protocol whose `panels/` holds an agent's files opens in the panel-tile window (`protocol_panels`), unchanged. A Brightfield-saved `panels/<name>.yaml` from before this work is still read on open as that Protocol's dashboard.

## Non-goals

- **Holding several nodes live at once** (approach B): instant switching between nodes, and charts on different nodes filtering each other.
- **Adding a saved chart to a dashboard**, the step that replaces putting a new chart straight onto the open dashboard. It is its own command and its own issue.
- **arc's side of the folder change**: `arc protocol write-file`, `arc panel check`, the analysis skills. Filed in arcform.
- **Moving an existing `panels/` folder** to the new folders.
- **What a file opens on.** A data file still opens on its generated dashboard; the four-square is for making a chart.
- **Adding a layer from another table to a chart** (outlines under points, say): a later shelf feature with its own issue. Combining two tables' columns in one mark stays a join step upstream, which makes a view the chart reads.
- **The Panel.**

## Anti-goals

- **A four-square that stands in the way of building from scratch.** The shelf is usable before any cell is picked.
- **A suggestion that needs typing before it shows.**
- **A Save that writes to `panels/`.**

## Approaches

**A. The window moves to the node in focus (chosen by Hugh).** The window keeps one held table and one chart document. Going to another node loads that node's rows and its chart, and going back reloads the earlier one from its file. It is the smallest change to a window that assumes one table and one chart throughout (`ProtocolModel::table()`, the chart document, the Save path), and B can be built on top of it later.

**B. The window holds several nodes live (not taken).** Instant switching and cross-node filtering, at several times A's cost: everything singular about the window becomes per node. Nothing in this work needs it.

Mosaic itself is no limit either way: a spec's `data` block names as many tables as it needs, and the engine loads each entry (`crates/brightfield-engine/src/lib.rs:3262`). A dashboard may arrange charts that read different tables. Cross-filtering between them works where the column names line up.

## What the design fixes

- **A new node kind.** `AssetKind::Chart` joins `AssetKind::Dashboard` in `brightfield-protocol`'s graph. Each saved chart is a spine row under the tables or views it reads, as a dashboard is today. Edges: table or view to chart (one for each table the chart's `data` block names, since a plot's marks may each read a different table), chart to dashboard, and table or view to dashboard for plots a dashboard holds directly.
- **What "reads the node" means.** `ProtocolModel::charts_reading` widens to list chart nodes and dashboard nodes that read a table or view, directly or through a chart. Its order, the graph's edge order, is the chooser's order.
- **The four-square is the empty chart's content.** A draft chart starts with no column on any channel, its shelf cells reading *add a column*; the chart pane under the shelf draws the four-square until a cell is picked or a column lands. There is no empty-chart state in the code today, so this is new, not a replacement.
- **A chart may read several tables.** The four-square starts a chart from the node in focus, so a new chart reads one. Opening a chart that reads several makes sure each has rows (goal 7) before it draws; the window still holds one node, the one in focus.
- **A draft chart is named** after its node and kind, made unique in `charts/`, and renamed through the existing rename path.

## Calls settled after the builder check

- **A saved chart or dashboard comes back on reopen** because Brightfield reads the Protocol folder's `charts/` and `dashboards/` when it opens a Protocol and adds a node for each file, with an edge from each table its `data` block reads. It does not wait on arc.
- **Save of the generated dashboard writes `dashboards/<name>.yaml`**, and the model's tables and joins copies move with it, to `dashboards/tables/` and `dashboards/joins/`. `agent_panels_in` keeps recognising Brightfield's old `panels/<name>.yaml` as Brightfield's own, so a Protocol saved before this does not open in tile mode.
- **A generated dashboard exists only for the table the file opened as**, as today. Moving to another node does not add one, so Chart over that node finds nothing reading it and opens the four-square. Making the held node one the window moved to goes through a path that does not call `add_generated_dashboard`.
- **A saved chart reads the Protocol's tables by name**, as agent panels read the run's tables, not an absolute path to a copy. The file is portable, survives a rerun, and arc can check it. The registry's kinds read `data_file::SOURCE`; the four-square binds each cell's spec to the node's table name instead.

## Left to the builder

- **How a dashboard file names a chart.** Mosaic has no include. Brightfield's composer must put together a dashboard whose plots are given by chart name. The syntax must be one Brightfield's YAML dialect can parse, and a dashboard naming a missing chart draws that plot's place with a sentence saying which chart is missing.
- **The preview run's mechanics** for a node no run has reached: reusing `run_is_preview` and the run records, what the pane shows while it runs, and how the 1,000-row limit is pushed into a remote read so that only those rows are fetched. A chart saved over a sample reads its table by name, so it draws every row once a full run has made them. The grid's own preview of an unrun step reads one page (`data_grid::PREVIEW_ROWS`, 128 rows); whether it moves to 1,000 is not part of this work.
- **Fewer than three applicable kinds.** The unused cells say why, in one line each (my call: never a blank cell).
- **The New chart verb's name and key**, in `brightfield-keys`, scored as other keys are.
- **How small the four-square's cells draw**, and whether they draw through the chart picture path or the live canvas.
- **The draft name's exact form.**

## Calls made in Hugh's place

- A data file still opens on its generated dashboard.
- The chooser lists New chart as its last row, so the four-square is one step from any node.
- Agent-written `panels/` files keep the panel-tile window and are not made graph nodes here; Brightfield never moves them.
- A cell the registry cannot fill shows a reason, not a blank.
