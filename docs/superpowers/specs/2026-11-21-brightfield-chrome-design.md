# Brightfield's chrome: design

Status: draft for the owner's approval, 2026-11-21.

## What this is about

**Chrome** here is meant in Nielsen Norman Group's sense: "the visual design elements that give users information about or commands to operate on the screen's content (as opposed to being part of that content)". In Brightfield that means:

- the menu bar, the title band and anything that carries commands
- the spine of the Protocol's steps
- the pane headers
- the inspector
- the console
- the status band

It does not cover the grid, the charts, dashboards or Panels themselves.

**Why it needs redesigning.** The owner's view is that Brightfield's modules are the right mix, but they are not laid out so that an analyst can reach them. When the analyst looks away from the content for a moment, the frame should hand back context:

- what was running
- where they were up to
- what they can do next

Two further aims shape the answer:

- **The keyboard must be learnable from the screen.** VisiData is still the best grid explorer the owner has used, but it took a long time to learn its keys. Brightfield should shorten that by showing every key where its command is.
- **The window should respond to what the analyst does.** This borrows Valve's working definition of fun from Half-Life: "the degree to which the game recognizes and responds to the player's choices and actions".

**How Brightfield's chrome stands today.** The window is an IDE workbench wrapped around a pipeline:
- a title band, and a locator band holding a breadcrumb
- a spine of steps, tables and views in the left rail
- one view of one node at a time on the canvas
- a right rail split by which document owns the selection (Operator, Inspector, Operation)
- a bottom band mixing run output, rows, the spec's source and history
- a status band

Commands are hard to find:
- The menu bar holds only the macOS app menu, with Preferences and Quit (`crates/brightfield-shell/src/mac_open.rs`).
- There is no toolbar.
- The commands live in a palette opened with Space and in the `?` keyboard help.
- Some keys listed in the help have no handler.

## References, and what each contributes

- **Palantir Foundry.** Chrome in three layers, each identical wherever it appears: a platform sidebar, a resource header (breadcrumb, menus, state, Share) and a tool layer (a labelled tool strip, panel rails, a tabbed drawer). Also next steps offered where the eye is, and actions greyed or hidden unless they apply to the selection.
- **Microsoft Office's ribbon, in its one-row Simplified form.** A visible taxonomy of commands, greyed when the selection can't take them. KeyTips put a letter on every command while the modifier key is held. The ribbon collapses to its tabs.
- **Alteryx Designer's tool palette.** Category tabs over a row of tools: a ribbon of operators sorted by stage, familiar to analysts arriving from it.
- **Affinity's Personas.** One document, and a switcher that swaps the tools and panels for the job in hand.
- **Lightroom Classic's filmstrip.** The items stay in view whichever module is open, and the module acts on the picked item.
- **MotherDuck's Flight Explainer.** The pipeline as rows, with the graph drawn in a gutter, plus each node's state, output, a timeline bar and its duration, and the run log in the footer.
- **MotherDuck's Column Explorer.** The bar for what the inspector shows about a column.
- **Xcode's activity area.** The toolbar's centre says what is building or running and where.
- **macOS's unified title bar and toolbar,** and **vertical tabs in browsers.** A header kept to as few rows as possible.
- **The slim headers of VS Code, Positron, DataGrip's New UI, Zed and Beekeeper Studio.** These set the expectation of analysts arriving from developer tools. Those tools moved commands into a palette, icon strips and right-click menus, and paid for it in discoverability.

## Goals

1. **Context at a glance.** The frame always shows:
   - which node of the Protocol is in focus, and what is being done to it
   - what is running, or when the Protocol last ran and how old its data is
   - what can be done next
2. **Commands are discoverable.** Every command lives in a visible taxonomy. A command the current selection can't take shows greyed, with a sentence naming what it needs.
3. **The keys are learnable from the screen.**
   - Every visible command shows its key.
   - Holding ⌥ puts a key tip on every ribbon tab and command.
   - When a command is used with the mouse, the status band names its key.
4. **Focus is the resting state; composition comes on request.** One activity fills the canvas by default. A second pane, and several open places, are available when asked for.
5. **Consistency.** Each part of the chrome is in the same place in every activity, so what the analyst learns once keeps working.
6. **Both expectations are met.** Analysts from Excel, Power BI, Tableau and Alteryx find a ribbon. Analysts from VS Code and Positron can collapse it to a one-row header and work from keys and the palette.

## Non-goals

- **The contents of the canvas:** the grid, chart design, dashboards, Panels and their actions. A Panel appears here only as an activity that is greyed until an action exists.
- **Defining the components.** This design names the components it needs; the design system defines them (see Sequencing).
- **Colour, type, density and icons.** The design system settles these: square chrome, dense at 12px with no density modes, Tabler icons, the token set.
- **A platform shell around the app**, Foundry's sidebar of apps and files. Brightfield is one application, and the outer level would have nothing to switch between. The owner may reopen this later; nothing here depends on it.
- **Navigation by object.** An object, the business noun a Protocol's data describes, is a later way in. The quick-open below leaves room for it.

## Anti-goals: what would make this a failure even if it works

- **A header that grows.** Open, it is two rows. Collapsed, it is one.
- **Keys that look live and do nothing.** Every key the chrome shows must act.
- **Chrome that comes and goes unpredictably.** Anything hidden comes back by one reliable key or click, in the same place each time.
- **Two rows of tabs that look alike.** The activity switcher, the open places and the ribbon's tabs must not be mistaken for one another.
- **A greyed command that doesn't say why.**

## Constraints

- **The region kinds are settled:** bands, rails, one canvas and overlays. A band has a fixed height and full width and is never resized. A rail has a default and minimum extent, resizes, and collapses. The canvas takes what remains. An overlay floats over the canvas.
- **The look comes from `meridian-online/design`:** its tokens, square chrome (ADR 0013), Tabler icons (ADR 0009), line tabs, keycaps, and hard shadows on overlays only.
- **Every key binding is a default** that an analyst can change in a keymap file, through the key registry (`crates/brightfield-keys`).
- **The Protocol is the document,** and its graph is the spine for every view: sources lead to views, then grids and charts, then dashboards, then Panels.
- **macOS is the validated platform.** Linux and Windows have no global menu bar, so the same menus must be reachable in the window there.
- **The same command list feeds** the menu bar, the ribbon, the palette and the keymap, so the four can't disagree.

## Approaches considered

| | Where the commands live | Header height | Strength | Weakness |
|---|---|---|---|---|
| **A · A single-line ribbon across the window (chosen)** | A row of stage tabs over one row of commands, each with its key; key tips while ⌥ is held | two rows open, one collapsed | The whole taxonomy in one place that never moves | Reopens an earlier ruling against a toolbar strip, at window scale |
| B · A command strip in each pane | The focused pane's strip carries its commands, with the stages as menus beside them | one row per pane | Commands where the eye is | The taxonomy sits behind clicks, and two panes side by side carry two strips |
| C · Keys first, the ribbon on demand | A slim bar of suggested next moves with their keys, and everything summoned over the canvas | one row | Fewest pixels, fastest for the fluent | Most for a newcomer to discover, which works against learning the keys |

**The owner chose A:** "the consistency would reinforce the learning". Analysts from Excel, Power BI and Alteryx expect a ribbon. The collapse below meets the expectation of those from developer tools.

**This reopens an earlier ruling.** In August the owner turned down a separate toolbar strip above each pane, for the 26 pixels each would cost; toolbars were to live in a pane's header. A is one window-wide row rather than a row per pane, and the owner reopened the question in this brainstorm.

## The design

The arrangement, open and collapsed:

```
open
┌ ● ● ●  ≡ brewtrend   Bring in · Shape · Model · Publish │ Grid · ranking    ◐ ran 23 Oct · 28 days old    ▶ Run ⌘↩ ┐  title band
│ [Filter rows W] [Sort O] [Columns K] │ [Derive =] [Retype ⇧T] [Summarise ⇧S]                       hold ⌥ for key tips │  command row
├ open places ───────┬ rank › ranking   [Grid G] Chart C  Dashboard D  Panel N  ┬ inspector ───────────────┤
│ ranking · Grid     │                                                          │ column installs_30d      │
│ measures · Chart   │            one activity, full attention                  │ count · INTEGER          │
├ spine ─────────────┤                                                          │ distribution             │
│ ● fetch ×6   2.3s  │                                                          │ min · median · max       │
│ ● load      1.4s   │                                                          │ do with this column      │
│ ● trending  1.8s   │                                                          │                          │
│ ▪ ranking 19,028   │                                                          │                          │
├────────────────────┴ Run log · Problems · Versions ─ rank · ok · 1.2s ─────────┴──────────── ⌘J ───────────┤  console
└ ranking · Grid · column installs_30d    Filter rows · next time, press W         19,028 rows · idle ────────┘  status band

collapsed
┌ ● ● ●  ≡ brewtrend   Bring in · Shape · Model · Publish │ Grid · ranking    ◐ ran 23 Oct · 28 days old    ▶ Run ⌘↩ ┐
```

### 1. The menu bar

On macOS it is the native menu bar: Brightfield, File, Edit, View, Protocol, Data, Window and Help.
- It is built from the same command list as the ribbon and palette.
- Each item shows its key.
- Items the selection can't take are greyed, as in the ribbon.
- On Linux and Windows the same menus open from the mark at the left of the title band.

### 2. The title band

One band, in the macOS unified style, holding from left to right:

1. **The window's controls.**
2. **The mark,** which leads home to the front door.
3. **The Protocol's name and where it is kept.**
4. **The ribbon's stage tabs, then the contextual tab.** The stage tabs are Bring in, Shape, Model and Publish. They read arcform's operation stages (acquire, shape, model, publish) as the reasons an analyst reaches for an operator: bring data in, work it out, act on it. The contextual tab, set apart by a divider and a tint, is named for the focused activity and node, such as `Grid · ranking`.
5. **The activity area, in the centre.** While a run is in progress it shows the step running, how many steps are done and a progress bar. Otherwise it shows when the Protocol last ran and how old its data is, with a state mark: fresh, stale or failed.
6. **Run, with its key ⌘↩.**

The stage tabs, the activity switcher and the open places must each look unlike the other two.

### 3. The ribbon's command row

One row, under the title band, for the selected tab.

- **Each command is an icon, a label and its key,** gathered into groups separated by a rule.
- **A greyed command keeps its place and its key.** Pointing at it opens a popover naming what it needs, for example:
  - "Relate needs two sources."
  - "Join: pick a second table to join ranking to."
  - "Forecast needs a date or time column. ranking has none."
  - "Schedule needs Meridian Pro."

  Pressing its key shows the same sentence.
- **Key tips.** While ⌥ is held, a letter sits on each tab and each command. Releasing ⌥ removes them.
- **Collapse.** Double-clicking the selected tab hides the command row; so does a key bound in the registry. The header is then one row. The tabs still open their commands as a dropdown row, and key tips still work. The collapsed state is remembered per user.
- **Availability is declared by the command.** Each command states the inputs it needs: a count of sources, a column of some type, a selected node of some kind, a plan. The ribbon, the menus and the palette all read that declaration, so a command is greyed with the same sentence everywhere.

### 4. The spine and the open places, in the left rail

- **The spine stays on screen in focus.** It has three widths, which one key cycles:
  - **Thin:** a column of state marks, one per step, with the focused node marked.
  - **List,** after Flight Explainer: one row per step and table. Each row carries the graph's edges in a gutter, a state mark (not run, running, ok, failed, stale), what it produced (a row count, a table count) and, for steps, a timeline bar and duration from the last run. Grouped steps, such as six fetches, fold.
  - **Graph:** the DAG.
- **Picking a node in the spine makes it the focus.** The activity switcher then acts on it.
- **The open places sit above the spine,** as a vertical list. A place is a node paired with an activity, such as `ranking · Grid` or `measures · Chart`. Opening a place adds it; closing removes it. The list holds several places without a tab strip across the header, the way vertical tabs work in a browser.
- **Quick open by name.** A key opens a search over every table and view by its DuckDB name. Choosing one focuses it in the current activity. Objects can join this search later.

### 5. The canvas and its pane header

- **One pane by default, with the analyst's full attention.**
- **The pane header names the node and the step that made it,** for example `rank › ranking`, followed by the activity switcher: Grid, Chart, Dashboard and Panel, each with its key.
  - The switcher belongs to the node. Switching keeps the node and the selection, so going from a chart to the grid to find a field and back loses neither.
  - An activity the node can't take is greyed with its reason. Panel stays greyed until an action has been defined.
- **On request, the canvas splits into two panes.** The activity keys act on whichever pane has focus, and each pane keeps its own header.
- **The header's right end** carries the node's kind and size, for example "table · 19,028 rows".

### 6. The inspector, in the right rail

It describes the selection, at the level Column Explorer sets for a column:
- **What the column is:** its FineType type, its storage type, and the step that made it.
- **Its values:** a distribution (bars for numbers, top values for categories), minimum, median, upper quantile, maximum and missing count.
- **What can be done with it:** its commands, with their keys.

For a chart, it shows the encoding, the table it reads with the rows drawn, and the fields available to it.

### 7. The console, in the bottom rail

- Tabs for the run log, problems and versions.
- Collapsed, it is one line showing the latest log entry. One key opens and closes it.
- During a run, it receives each step's line as the step finishes.

### 8. The status band

- **The left end** names the focused node, the activity and the selection.
- **After a command is used with the mouse,** the status band says, for a few seconds, the command's name and "next time, press" its key. This is the window responding to what the analyst did: the shortest route to learning the keys.
- **The right end** carries counts and whether the engine is idle or working.

### 9. The palette

Space still opens every command, grouped by stage, with keys and greyed reasons, and typing SQL still works as it does today. The palette is the fallback for whatever the ribbon hides when collapsed.

## The calls made in the brainstorm, and whose they are

**The owner's:**
- the chrome's job is context (what ran, where you are, what you can do)
- the modules stay, laid out better
- focus is the resting state; side by side and several open places are on request
- the activities are bound to the graph, not floating free of it
- unavailable options are visible and say what they need
- the spine stays on screen, in a compact form and two expanded ones
- a console and an inspector
- the ribbon is a taxonomy of commands in a current look, with Tabler icons
- the keyboard is learnable from visual indicators
- approach A

**The assistant's, not yet tested by the owner beyond agreeing they looked right:**
- the ribbon collapses to its tabs
- the stage tabs sit in the title band
- open places are a vertical list above the spine
- the status band teaches a command's key after mouse use
- the menu bar's menu names
- the availability declaration on each command

**Left to the builder:**
- each key (the mockups' keys are placeholders, and every key must be a registry default)
- the extents within the region rules
- the order of commands within a group
- how the native menu is built on macOS
- how the in-window menu works on Linux and Windows
- how a command's availability is expressed in the registry

## Sequencing

The design system already has the tokens this chrome needs. What it lacks are the components that compose them. The owner's point is that building those components first will make this work easier.

1. **This design is approved.**
2. **The design system defines the components it names,** in its `meridian-egui` crate, as part of a design review raising its component detail to Foundry's level:
   - a ribbon tab and a contextual tab
   - a ribbon command (icon, label, keycap, and a greyed state carrying its reason)
   - a key tip badge
   - a reason popover
   - an activity switcher, distinct from tabs
   - a spine row (graph gutter, state mark, output, timeline bar, duration) and its thin and graph forms
   - an open place in a vertical list
   - the activity area
   - inspector sections with a title and a count, and a distribution bar
   - a menu row with its key
   - the status band's teaching message
3. **Brightfield assembles the chrome from those components,** in slices that each leave the app working:
   1. the command list's availability declarations, and the native menu bar built from it
   2. the unified title band and the ribbon, with key tips, collapse and greyed reasons
   3. the spine's three widths, with the open places and quick open
   4. the activity switcher bound to the node, and the split
   5. the inspector
   6. the console and the teaching status band

## Open for the plan

- Whether the existing locator band survives once the pane header names the node and the title band names the Protocol.
- How the right rail's three tabs today (Operator, Inspector, Operation) fold into one inspector.
- How the bottom band's tabs today (Log, Quality, Rows, Editor, Versions) map onto the console.
