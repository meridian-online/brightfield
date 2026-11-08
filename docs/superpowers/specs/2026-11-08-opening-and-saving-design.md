# Opening and saving: Brightfield opens Protocols, not files

Design for how an analyst gets data into Brightfield and keeps what they make of it. Opening anything makes or opens a Protocol; files, folders and databases are sources added to it, through a keyboard-driven sources browser and through the Mac's own gestures; a Protocol, its sources and its joins are kept at once, and its other edits wait for Cmd+S.

## Intent

The Protocol is the native file format of Brightfield and arcform, and nearly every action applies to one (Hugh, 2026-10-28). Brightfield's opening is still file-first. A lone CSV opens as a bare file, and its Protocol appears only when Save or Run writes `arcform.yaml` beside the CSV. Only a folder, workbook or database of two or more tables becomes a Protocol on open, inside Application Support where the analyst never sees it. Cmd+S does nothing outside the YAML editor. The front door lists a Protocol only once it is saved, and it shows "never run" for a Protocol whose results the analyst has seen. There is no rename and no way to remove a step. Hugh's verdict on 0.1.7 was that the folder and file opening flow and the Protocol editing and saving flow are not right yet, and that the app is not yet one he would share.

This design serves the analyst's first hour on a Mac. The launch waits on Hugh recording that the first hour gives the feelings he named: "this is easy"; "aha! I can see it now"; "wow, this gives me more than a spreadsheet"; "results are so quick"; "I need to tell … about this". The first hour Hugh described for this flow (2026-11-08):

1. **Before the app,** the stranger meets Protocols: a Protocol page on meridian.online shared on social, in a message or by email; an agent that cites the Meridian framework as the answer to an analytics problem; or someone recommending Brightfield.
2. **They open one,** and are then encouraged to make a new one of their own.
3. **They see how easy it is to add sources.** This is the showcase.

The person is an early adopter with middling technical depth: someone who uses a GUI and learns keyboard shortcuts by browsing panes, not someone who already knows yazi. A Mac user moves files into an app without learning that app first, by double-clicking in Finder and by dragging onto the Dock or a window. Hugh, 2026-11-08: "This is important and valuable. Let's make sure this is included."

## Goals

1. **Opening makes or opens a Protocol.** Every way in ends with a Protocol open:
   - **A Protocol directory** (a folder holding `arcform.yaml`) opens where it is, whether it sits in the Protocols folder, among its data, or wherever an agent wrote it.
   - **A data file, folder, workbook or database** makes a new Protocol at once in the Protocols folder, named after it, with it as the first source. Each open makes a new one: `earthquakes_2024`, then `earthquakes_2024-2`. Duplicates are accepted, as they are in Excel.
   - **New Protocol** on the front door makes an empty Protocol and opens the sources browser.
   - **What the window shows.** Hugh likened the window to a PowerPoint view (2026-10-28): the spine on the left is the Protocol, and what is selected on it occupies the canvas. So:
     - An empty Protocol shows the sources browser over an empty canvas.
     - With one source, its table is selected and the canvas shows its grid and generated dashboard, as a single table opens today.
     - Once a second source is added, the model step is selected and the canvas shows the model, as a folder of tables opens today. Selecting a source's table on the spine shows that table's grid and dashboard.
     - The first picture after any open is drawn without a question first.
2. **The Mac's own gestures reach every way in.** Brightfield registers as an app that opens CSV, TSV, Parquet, Excel workbooks, DuckDB and SQLite files, and Protocol directories and Protocol archives. Each of these makes or opens a Protocol as in goal 1:
   - double-clicking in Finder, or using Open With;
   - dropping onto the Dock icon;
   - dropping onto the window when the front door shows;
   - Cmd+O.

   Dropping a data file or folder onto an open Protocol's window adds it as a source to that Protocol, as the browser's add does. Dropping a Protocol directory or archive there opens that Protocol instead, after the close question if a draft is pending. Opening a folder of tables by gesture makes its Protocol and opens the sources browser on that folder, so the analyst can accept its joins before anything is added.
3. **Open in Brightfield from a web page.** Brightfield answers the link `brightfield://open?url=<address>`, where the address is an `https` URL on `meridian.online` or one of its subdomains, pointing at a Protocol archive: a zip of a Protocol directory. Brightfield downloads it and unpacks it into the Protocols folder, named after the archive's Protocol (taking `-2` and so on on a clash). It then opens it, runs it and draws it. A link to any other host is refused with a sentence saying why. The website's button builds against this format.

   **What an archive holds.** A Protocol archive is a `.zip` whose top level is one Protocol directory. Every source it reads is either:
   - a file inside that directory, addressed relative to it; or
   - a remote address its steps fetch, as the example Protocols' `http_fetch` steps fetch from their issuers today.

   An archive whose Protocol names an absolute local path is refused, with a sentence naming the path, since it could run on no other Mac. Brightfield does not register for every `.zip` in Finder, because that would take over Finder's handling of all zips. It accepts a zip holding a Protocol through the link, Cmd+O and a drop, and checks for `arcform.yaml` before unpacking.
4. **New Protocols live in `~/Documents/Protocols`, visible in Finder** (Hugh, 2026-10-28, confirmed 2026-11-08). The Protocol's path is fixed from the first moment, which matters because arc keys a Protocol's working database by the path of its directory.
5. **The sources browser adds sources from the keyboard and the mouse alike.** It follows yazi and Hugh's prototype (the `prototype/save-models` branch, `prototypes/protocol-first.html`, at `9189fa7`):
   - three columns: places; the current folder's list; and a preview of the item under the cursor;
   - a file previews its first page of rows, and a folder previews as a dovetail canvas showing the joins dovetail proposes between its tables;
   - it opens on the Protocol's first source, in its folder, or on the folder that was dropped;
   - it takes the window's theme in light and dark, as every sheet and panel does; none draws a palette of its own.

   Every action has a key, and every key also has a click: a row is clicked to move to it and double-clicked as Enter, a join is clicked as Space on it, and the actions below have buttons. The keys that apply in the current pane are shown along the browser's foot, so a stranger learns them by reading. The keys:

   | Key | Action |
   |---|---|
   | `j` `k` | move |
   | `h` | up a folder |
   | `l` or `Enter` | the same action: into a folder, or add the file |
   | `a` | add what is highlighted: the folder (with the joins accepted on its canvas), the file, or the marked files |
   | `Space` | mark a file; on the canvas, accept or drop the highlighted join |
   | `Tab` | move between places, list and canvas |
   | `/` | type a glob |
   | `g t` | fill the browser with the preview |
   | `v` | switch a folder's preview between the canvas and its file list |
   | `Esc` | close |
   | `A` | in a Protocol, open the browser |
   | `o` | in a Protocol, organise with dovetail |
6. **The spine leads with the model.** Once sources are added, the navigator's spine lists the model's tables first, each beside the tables it joins, with the model step under them and a curve per join in the spine's gutter. The sources with no join follow under a heading.
7. **The save model is a split** (Hugh, 2026-11-03):
   - **Kept at once.** Making the Protocol, adding a source, and accepting or dropping a join, with the model step the joins make, are written to disk when they happen, with no Save.
   - **A draft until Cmd+S.** Every other edit is a draft until Cmd+S: a step kept from the grid or typed SQL, a step removed, a chart edit. The title shows the unsaved mark while one is pending.
   - **The close question.** Closing the window, going Home, or opening another Protocol with a draft pending asks first: save, close without saving, or keep editing. Enter saves.
   - **Close without saving keeps a version.** It records the unsaved edits in the Protocol's version history, so nothing is lost silently. Today only the chart's text is recorded this way; a kept grid view and kept typed SQL are recorded too.
8. **Cmd+S saves the Protocol from anywhere in the window.** In the YAML editor it still saves the editor's buffer, then the Protocol.
9. **Rename.** A Protocol can be renamed from the window and from its row on the front door. The rename renames its folder in Finder too (Hugh, 2026-11-08). Brightfield moves or rebuilds arc's working database for the new path, and the Protocol's recents entry and version history follow it. Any source path recorded relative to the Protocol's folder that points outside it is rewritten, in the data packages and the model, so the Protocol still finds its sources. The same repair runs when Brightfield opens a Protocol that was moved in Finder. A rename is a change to the Protocol itself, so it is kept at once, like adding a source.
10. **Remove a step.** A step other than a source's load and the model step can be removed from the spine. Removal is a draft until Cmd+S, like any other step edit. A step that later steps read is refused with a sentence naming them.
11. **The front door shows Recents and the tree of Protocols.**
    - **Recents** lists Protocols most recently opened, wherever they live.
    - **The tree** draws `~/Documents/Protocols` from disk. An ordinary folder is a group and a directory holding `arcform.yaml` is a Protocol. Groups nest to any depth. A Protocol is a leaf, and Brightfield never looks inside one for others.
    - **The tree is reread whenever the door is shown,** so Protocols arranged in Finder appear arranged (Hugh, 2026-11-08: Finder is enough for now). A Protocol is on the door from the moment it is made, saved or not.
12. **The door's run state is current.** A row says what the Protocol's newest run record says when the door is drawn, not a snapshot taken at the last save.
    - **What counts as run.** A run is a run of the whole Protocol through arc that writes a run record: Run in the window, or the run that follows opening from a link. Loading, profiling and dovetail's passes while sources are added are previews, and write no run record.
    - **A Protocol that has only been previewed** reads as not yet run, in words that do not suggest it failed or is empty. The builder words it, for example "previewed, not run".
13. **Quitting and relaunch.**
    - **Cmd+Q with a draft pending asks the close question,** as closing the window does, and the quit waits for the answer. Close without saving records the version, as on close.
    - **Relaunch restores the Protocol that was open, as saved,** as macOS restores windows. Drafts are not kept across launches, because the quit's question has already settled each one.
    - **Closing the window shows the front door.**

## Non-goals

- **Organising Protocols inside Brightfield.** No New Folder, no moving between groups, no folder rename on the door. Finder does it and the door shows the result (Hugh, 2026-11-08).
- **A source library shared across Protocols.** Hugh ruled that the library records where each source file is and is shared across Protocols, with each Protocol owning its profiling and transformations. In this design each Protocol records its own sources by path, as own-files Protocols do today, and the shared library comes later. Locating a moved source file, and having every Protocol that uses it follow, comes with the library.
- **Removing a source.** Not ruled on. The model's existing leave-out and include stay as they are.
- **A keymap file.** Every binding is a default an analyst will be able to change. That work is separate and reaches beyond this flow. The new keys go into the key registry so it can pick them up.
- **The website's half of Open in Brightfield:** the button on each Protocol page, its link previews, and the fallback for a visitor without Brightfield. That is the website's own work. This design sets the format it builds against.
- **Moving existing Protocols.** Protocols already made in Application Support stay there and stay on the door, in Recents. Only new ones go to `~/Documents/Protocols`.
- **Online sources and database servers in the browser.** The browser browses the Mac's disk.
- **Save As and "move to…".** A Protocol is put elsewhere by moving its folder in Finder.
- **Linux and Windows.** The gestures are the Mac's; the browser and save model are written portably, but are validated on macOS only, as for the rest of the launch.
- **The starter datasets** and the door's gallery of them. They are separate work. This design gives them the same open path: a starter opens as a Protocol like any other.

## Anti-goals

- **A question before the first picture.** No dialog asks where to save, what to name it, or which tables to join before the analyst sees their data. The one question a stranger meets is macOS's own permission for Documents, and Brightfield asks it at the moment its reason is obvious (constraint 1).
- **Work lost silently.** No path (close, Home, opening another Protocol, quitting, relaunch) drops a draft without the close question or a recorded version.
- **Two ways of opening.** A behaviour that works for a folder's Protocol and not for a CSV's, or the reverse, is a defect. The alpha review's Cmd+S and "never run" faults came from such a split.
- **A browser only yazi users can drive.** If a stranger has to read documentation to add a source, the showcase has failed.
- **The browser looking foreign to the window.** It takes the window's theme.

## Constraints

1. **macOS permission for Documents.** A non-sandboxed app writing into `~/Documents` triggers macOS's "would like to access files in your Documents folder" prompt the first time. Brightfield creates `~/Documents/Protocols` when the analyst makes their first Protocol, so the prompt follows an action whose reason is plain. If access is refused, Brightfield says so in a sentence, names System Settings → Privacy & Security → Files and Folders, and makes nothing. It does not fall back silently to another folder.
2. **winit does not deliver the Mac's open events.** Brightfield's window stack is eframe 0.35 over winit 0.30, which does not forward the application delegate's open-files or open-URLs callbacks. The builder installs handlers for the Apple Events that carry them (open documents and get URL) through `objc2`, which is already in the dependency graph. Each event is queued for the next frame, including events that arrive before the first frame, which is how a launch by double-click delivers its file. `packaging/Info.plist` gains `CFBundleDocumentTypes` and the exported or imported type declarations for the file types in goal 2, and `CFBundleURLTypes` for the `brightfield` scheme.
3. **arc keys a working database by directory path.** A Protocol's path is fixed at birth, and a rename or a Finder move changes it. Brightfield detects a Protocol whose path no longer matches its database, and moves or rebuilds the database without asking.
4. **The app watches its Protocol.** An agent may write to an open Protocol through arc; the app already notices and shows the step beside the analyst's unsaved edits, and Save applies the analyst's edits to the file as it then stands. The kept-at-once writes (sources, joins) go through the same path so the two don't race.
5. **The gate.** `cargo xtask ci` on each push. On Hugh's Mac, one cargo build at a time.
6. **No new formats.** A Protocol archive is a zip of a Protocol directory. The model the joins make stays an Ossie model file, and sources stay recorded as they are in own-files Protocols today.

## Approaches

1. **One way in: every open makes a Protocol.** *Chosen.* Today there are two ways of opening data. The own-files way, which a folder of tables goes through, already makes a Protocol directory, records its sources by path, writes the model and runs dovetail. The one-step way opens a lone CSV as a bare file and writes its Protocol beside the CSV only on Save or Run. This approach makes the own-files way the only one. A CSV, a folder, a gesture, the browser and New Protocol all make a directory in `~/Documents/Protocols` and add sources to it. The spine, Cmd+S, the close question, rename and the door are built once on that model. An existing Protocol, including one written beside its CSV by an earlier Brightfield, opens where it is through the existing-Protocol route.
   - *Trades:* the most change up front. The one-step open is retired, and tests that assume a bare file is open change.
   - *Gains:* each later piece is built once, against the model Hugh ruled on. A door tree read from disk has no holes, because every new Protocol is in it.
2. **A Protocol-shaped surface over today's two ways.** The one-step Protocol moves to `~/Documents/Protocols/<name>` and points at its CSV, and the own-files way stays as it is. Less churn now, but every new behaviour is built and tested twice, and the faults Hugh found in the alpha review came from that kind of split. Not chosen.
3. **Approach 1 in two releases,** with the gestures, Cmd+S, rename and the door first and the browser second. It gives Hugh something to drive sooner, but the showcase he named arrives second. Not chosen as a separate design. The plan orders approach 1's tasks so that gestures, Cmd+S, rename and the door land first, which gives the same early checkpoint.

Hugh did not pick between these explicitly. He asked whether a tree of Protocols on the door changed approach 1, and whether a new Protocol should be written in the Protocols folder, both of which build on approach 1. The design proceeds on it, and his approval of this document confirms it.

## What Hugh decided, and why

- **The Protocol is the document; sources are added to an open one** (2026-10-28). "Protocols are the Native File Format of Brightfield and Arcform - so almost actions apply to a Protocol."
- **New Protocols live in one folder Brightfield manages, as Tableau kept its workbooks; an existing Protocol folder is worked on where it is** (2026-10-28). A Protocol is a directory of files, and "we don't want too much unmanaged change within the protocol directory". `~/Documents/Protocols`, visible in Finder, "still seems to be the best fit" (2026-11-08).
- **A Protocol exists from the first open, named after its first source** (2026-10-28), and **each open makes a new one**: "If you keep double clicking CSVs you'll end up with duplicates. This is a reality of the user behaviour."
- **The save split** (2026-11-03): the Protocol, its sources and its joins are kept at once, and other edits wait for Cmd+S with the close question. It keeps the Protocol real from the first open, and keeps an unwanted filter or chart edit out of it when the analyst closes without saving.
- **The browser follows yazi.** It takes the window's theme ("the theme should always be consistent"), accepts a join by key, treats l and Enter as one action as Hugh's own yazi binds l, adds a whole folder with a, and opens on the Protocol's first source (2026-11-02 and 2026-11-03).
- **The spine leads with the model** (2026-11-03): Hugh wanted "a column graph that is weighted towards the interactions".
- **Every key binding is a default an analyst can change** (2026-11-03); the keymap file is separate work.
- **The Mac's own gestures are in** (2026-11-08): "This is important and valuable."
- **Open in Brightfield must be visible** (2026-11-08); whether the website's half was separate work was left to this design, which made it so.
- **Rename renames the folder** (2026-11-08).
- **The door shows Recents and a tree of Protocols, nested in directories; organising is Finder's for now** (2026-11-08). A new Protocol is written in the Protocols folder, with no option at creation.

## Calls made on Hugh's behalf

- The link format: `brightfield://open?url=<https address of a Protocol archive>`, limited to `meridian.online` and its subdomains, with the archive unpacked into the Protocols folder. The website builds against this.
- An archive holds one Protocol directory whose sources are inside it or fetched from remote addresses. One that names an absolute local path is refused. Brightfield takes archives through the link, Cmd+O and a drop, not by registering for every zip in Finder.
- What the canvas shows follows the spine's selection: a lone source's grid and dashboard, or the model once there are two sources.
- A run is a whole-Protocol arc run that writes a run record; adding sources only previews.
- Cmd+Q asks the close question; drafts are not kept across launches.
- Rename, and opening a Protocol moved in Finder, repair source paths recorded relative to the folder.
- Dropping a Protocol onto an open Protocol's window opens it rather than adding it.
- The browser works by click as well as key, and shows the keys that apply along its foot.
- Dropping onto an open Protocol's window adds a source; dropping onto the front door, the Dock or Finder's Open With makes or opens a Protocol.
- The Documents permission is asked at the first new Protocol, and a refusal makes nothing and says how to grant it.
- Existing Protocols in Application Support stay there.
- Rename is kept at once; removing a step is a draft until Cmd+S.
- Going Home and opening another Protocol ask the close question, as closing the window does.
- The door's run state is read from the newest run record when drawn, and "never run" is reworded where previews computed.
- From the prototype's list of undecided calls:
  - Enter on a folder's canvas adds the folder with its accepted joins.
  - Marks survive going into another folder.
  - A dropped folder makes a Protocol named after it and opens the browser on it.
  - New Protocol starts as "Untitled" and takes the name of its first source, renaming its folder, so long as the analyst has not renamed it.
  - A glob comes in as one grouped row on the spine.
  - Relaunch restores the open Protocol.
  - A glob's row count fills in after its file count.

## What's left to the builder

- How the Apple Event handlers are installed alongside winit's delegate, and how events reach the frame loop.
- Which UTType identifiers are declared or imported for each file type, and whether a Protocol directory is declared as a package or a plain folder (a plain folder keeps it browsable in Finder, which the tree relies on).
- How the one-step tests are migrated, and the order in which the one-step code is retired.
- The wording of every sentence the analyst reads, within the writing rules of the repo.
- How the database is moved or rebuilt on a rename, and how a Finder move is detected (on open, by comparing paths, is enough).
- The archive's size and entry limits on download, and refusing an archive whose paths escape its folder.
- How the browser's preview reads a file's first page of rows without blocking the frame, as the grid's summaries do.

## Verification

Each goal has an integration test in `crates/brightfield-shell/tests/` driving the window headlessly. The open events are injected at the queue the Apple Event handlers fill, since a test cannot double-click in Finder. The browser, the spine's model-first order and the door's tree each get a committed baseline picture in both themes. The Mac gestures, the Documents prompt and the link are checked by hand on a packaged build, in a short checklist the pull request carries for Hugh to drive.
