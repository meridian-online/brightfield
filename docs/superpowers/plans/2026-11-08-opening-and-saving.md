# Opening and Saving Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every way into Brightfield makes or opens a Protocol in `~/Documents/Protocols`. Sources are added through a sources browser and the Mac's own gestures. A Protocol, its sources and its joins are kept at once, and its other edits wait for Cmd+S.

**Architecture:** The own-files route (`own_files.rs`) becomes the only way a new Protocol is made, and a lone CSV is a one-source own-files Protocol. All opens go through one queue, `open_queue::OpenQueue`, which the picker, the front door, egui's dropped files, macOS's Apple Events (open documents, get URL) and the `brightfield://open` link all fill. The window drains the queue once per frame. The save split rides on the dirty tracking and close question that exist today (`ChartDoc::has_unsaved_work`, `keep_unsaved`), widened to the whole window and to every way out.

**Tech Stack:** Rust; eframe/egui 0.35 over winit 0.30; `objc2` and `objc2-app-kit` (already in the graph) for the Apple Events; `zip` and the existing `remote::Fetch` for archives; `egui_kittest` for headless window tests and baselines.

**Spec:** `docs/superpowers/specs/2026-11-08-opening-and-saving-design.md`. Read it before any task; each task's requirements include the spec's goal it names.

## Global Constraints

- New Protocols live in the Protocols folder: `$BRIGHTFIELD_PROTOCOLS_DIR` if set, else the Preferences setting, else `~/Documents/Protocols`. Tests set `BRIGHTFIELD_PROTOCOLS_DIR` to a temporary folder, and **no test writes under the home directory**.
- A new Protocol is named after its first source (`earthquakes_2024`, then `earthquakes_2024-2`); the first takes the bare name, not `-1`.
- Kept at once: making a Protocol, adding a source, accepting or dropping a join, and rename. A draft until Cmd+S: every other edit.
- The link is `brightfield://open?url=<address>`, where the address is `https` on `meridian.online` or a subdomain of it, and anything else is refused with a sentence saying why.
- Every new key goes into `crates/brightfield-keys/src/registry.rs` with its context, so the help sheet, the palette and a later keymap file see it.
- Every sheet and panel takes the window's theme tokens. None draws a palette of its own.
- No private planning identifiers in code, comments, commits or pull request text.
- Before each commit: `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, and the tests the task names. After each commit: `cargo xtask ci --step comment-citations`. One cargo build at a time on Hugh's Mac.

## Review Focus

1. **A Protocol folder deleted or moved in Finder while it is listed or open.** The door drops its row on the next draw. An open window says the folder is gone, and does not crash or recreate it. Task 7 pins it.
2. **A data file opened from a read-only or removable volume, or a path holding spaces and non-ASCII characters.** It opens as a source, read where it is, with nothing written beside it. Task 2 pins it.
3. **The same CSV double-clicked twice in quick succession, as Finder sends it.** It makes two Protocols, `x` and `x-2`, never a clash or a half-written folder. Task 1 pins it.
4. **An archive whose entries escape its folder (`../`), are absolute, or unpack past a size limit.** It is refused before anything is written. Task 15 pins it.
5. **Cmd+Q with a draft pending while an arc run is in flight.** The close question waits for the answer, and the run's result is not lost. Task 5 pins it.

## File map

| File | Responsibility |
|---|---|
| `crates/brightfield-shell/src/protocols_dir.rs` (new) | Resolve the Protocols folder; claim a new Protocol's folder name; turn a refused write into the spec's sentence |
| `crates/brightfield-shell/src/open_queue.rs` (new) | `OpenRequest`, the queue every way in fills, and the routing of a path to Existing, New or Archive |
| `crates/brightfield-shell/src/mac_open.rs` (new, `cfg(target_os = "macos")`) | Install the Apple Event handlers and push their files and URLs onto the queue |
| `crates/brightfield-shell/src/protocol_archive.rs` (new) | Check a link, download an archive, check its entries, unpack it into the Protocols folder |
| `crates/brightfield-shell/src/rename.rs` (new) | Rename a Protocol's folder; repair relative source paths; move or rebuild arc's database |
| `crates/brightfield-shell/src/door_tree.rs` (new) | Read the Protocols folder as groups and Protocols; read each row's run state when drawn |
| `crates/brightfield-shell/src/sources_browser.rs` (new) | The browser's state and keys, with no drawing |
| `crates/brightfield-shell/src/sources_browser_view.rs` (new) | Draw the browser: three columns, previews, the dovetail canvas, the key foot |
| `crates/brightfield-shell/src/own_files.rs` | `create_in`, `add_source`, and one-source Protocols |
| `crates/brightfield-shell/src/window.rs` | Drain the queue; Cmd+S; the close question on Home, open and quit; the canvas following the spine |
| `crates/brightfield-shell/src/protocol.rs` | The model-first spine; removing a step |
| `crates/brightfield-workbench/src/persist.rs` | `opened` takes a Protocol path as well as a start id |
| `crates/brightfield-keys/src/registry.rs` | `save-protocol` bound window-wide; the browser's keys; `rename-protocol`, `remove-step` |
| `packaging/Info.plist` | `CFBundleDocumentTypes`, imported type declarations, `CFBundleURLTypes` for `brightfield` |

---

### Task 1: The Protocols folder and new Protocol names

**Files:**
- Create: `crates/brightfield-shell/src/protocols_dir.rs`
- Modify: `crates/brightfield-shell/src/own_files.rs:66-111` (`create` calls `create_in`), `crates/brightfield-shell/src/lib.rs`
- Test: `crates/brightfield-shell/tests/protocols_dir.rs`

**Interfaces:**
- Produces:
  - `protocols_dir::root() -> PathBuf`: `$BRIGHTFIELD_PROTOCOLS_DIR`, else `~/Documents/Protocols`.
  - `protocols_dir::claim(parent: &Path, stem: &str) -> Result<(PathBuf, String), ClaimError>`: creates `parent` if it is missing, then claims `stem`, `stem-2`, `stem-3`, … with `create_dir`.
  - `enum ClaimError { Refused(PathBuf), Io(String) }`. `Refused` is permission denied on `parent`. Its `Display` is the spec's sentence, naming System Settings → Privacy & Security → Files and Folders.
  - `own_files::create_in(parent: &Path, opened: &Path) -> Result<OwnProtocol, String>`.

- [ ] **Step 1: Write the failing tests**
  - `first_protocol_takes_the_bare_name`: `claim(tmp, "earthquakes_2024")` gives `tmp/earthquakes_2024`.
  - `a_second_takes_dash_two`: a second claim gives `earthquakes_2024-2`.
  - `two_claims_at_once_never_share_a_folder`: 8 threads claim the same stem, giving 8 distinct folders.
  - `a_refused_parent_makes_nothing_and_says_how`: on a parent with mode `0o500`, the result is `Err(ClaimError::Refused(_))`, its text contains `"Privacy & Security"`, and no folder is made.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test protocols_dir`. Expected: FAIL, module not found.
- [ ] **Step 3:** Implement `protocols_dir.rs`. Make `own_files::create` a wrapper over `create_in(&protocols_dir::root(), opened)`, so own-files Protocols move from `<datasets>/protocols/own` to the Protocols folder. Existing ones stay listed: see Task 7.
- [ ] **Step 4:** Run the same test. Expected: PASS. Also run `cargo test -p brightfield-shell --test own_files` (its fixtures now set `BRIGHTFIELD_PROTOCOLS_DIR`) and expect PASS.
- [ ] **Step 5:** Commit: "Make new Protocols in the Protocols folder, named after their first source".

### Task 2: One way in: a lone file is a one-source Protocol

**Files:**
- Create: `crates/brightfield-shell/src/open_queue.rs`
- Modify: `crates/brightfield-shell/src/data_file.rs:1230-1282` (`open_any_with`), `crates/brightfield-shell/src/window.rs:9676-9792` (`open_path`, `open_path_now`, `open_model_pane`), `crates/brightfield-shell/src/one_step.rs` (no new writes beside a CSV)
- Test: `crates/brightfield-shell/tests/open_makes_a_protocol.rs`

**Interfaces:**
- Consumes: `own_files::create_in` (Task 1).
- Produces:
  - `enum OpenRequest { Path(PathBuf), Link(String), AddSource(PathBuf) }`
  - `enum Route { Existing(PathBuf), New(PathBuf), Archive(PathBuf) }`
  - `open_queue::route(path: &Path) -> Route`: Existing when `path` or its parent holds `arcform.yaml`; Archive for a `.zip` whose single top-level folder holds `arcform.yaml`; else New.
  - `struct OpenQueue` (an `Arc<Mutex<VecDeque<OpenRequest>>>`) with `push(&self, OpenRequest)` and `drain(&self) -> Vec<OpenRequest>`. `MeridianApp::open_queue(&self) -> OpenQueue` returns a clone.

- [ ] **Step 1: Write the failing tests** (headless window via `egui_kittest`, with `BRIGHTFIELD_PROTOCOLS_DIR` set to a temporary folder):
  - `a_csv_opens_as_a_protocol_in_the_protocols_folder`: push `Path(housing.csv)` and run frames. `<protocols>/california_housing/arcform.yaml` exists, its one load step reads the CSV where it is, the canvas shows the grid and dashboard of `california_housing`, and nothing is written in the CSV's folder.
  - `a_csv_with_spaces_and_accents_in_a_read_only_folder_opens`: a copy at `Données brutes/houses 2024.csv` in a `0o555` folder opens, and the folder is unchanged.
  - `a_folder_of_two_tables_opens_the_model`: as today's own-files test, but in the Protocols folder, with the model step selected.
  - `an_existing_protocol_opens_where_it_is`: a fixture Protocol folder opens with no new folder made.
  - `a_protocol_saved_beside_its_csv_by_an_earlier_brightfield_still_opens`: a one-step fixture with `arcform.yaml` beside a CSV opens through the existing route with its panels.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test open_makes_a_protocol`. Expected: FAIL.
- [ ] **Step 3:** Route every caller of `open_path` through `OpenQueue::push` and drain it at the start of `MeridianApp::update`. `Route::New` calls `create_in`, then `run_loads` and `profile`. With one table it opens the table's grid and dashboard, `Opened::OneTable` carrying the `OwnProtocol` as its Protocol. With two or more it opens the model pane as today. One-step's `save_to` is no longer reached for a new open; delete its new-open callers, not the type, which reopening uses.
- [ ] **Step 4:** Run the test. Expected: PASS. Then run `cargo test -p brightfield-shell` and update tests that assumed a bare file had no Protocol behind it, each change preserving the behaviour the test protects.
- [ ] **Step 5:** Commit: "Open a lone data file as a Protocol of one source, through one queue".

### Task 3: The Mac's gestures: Finder, the Dock, the window and Cmd+O

**Files:**
- Create: `crates/brightfield-shell/src/mac_open.rs`
- Modify: `crates/brightfield-shell/src/main.rs` (install before the event loop runs), `crates/brightfield-shell/src/window.rs` (egui `raw.dropped_files` on the door pushes `Path`; Cmd+O opens the picker), `crates/brightfield-keys/src/registry.rs` (`open` on `global("cmd-o")`), `packaging/Info.plist`
- Test: `crates/brightfield-shell/tests/open_gestures.rs`, plus `scripts/package.sh`'s plist lint; add `plist` (MIT) as a dev-dependency of `brightfield-shell`

**Interfaces:**
- Consumes: `OpenQueue` (Task 2).
- Produces: `mac_open::install(queue: OpenQueue, wake: impl Fn() + Send + Sync + 'static)`. It registers `NSAppleEventManager` handlers for `kCoreEventClass/kAEOpenDocuments` (each file pushes `Path`) and `kInternetEventClass/kAEGetURL` (pushes `Link`), then calls `wake` so a frame runs. It is installed before `eframe::run_native`, so a launch by double-click delivers its file.

- [ ] **Step 1: Write the failing tests:**
  - `a_file_dropped_on_the_door_opens_as_a_protocol`: inject `RawInput.dropped_files = [housing.csv]` on the door, and a Protocol opens as in Task 2.
  - `requests_queued_before_the_first_frame_are_opened`: push before the harness's first frame, and the Protocol opens on the first frame.
  - `cmd_o_raises_the_picker`: the key event sets `pick_requested`.
  - `info_plist_declares_the_documents_and_the_scheme`: parse `packaging/Info.plist` with the `plist` crate. It declares `CFBundleURLSchemes` containing `brightfield`, and `CFBundleDocumentTypes` covering `public.comma-separated-values-text`, `public.tab-separated-values-text`, `org.apache.parquet`, `org.openxmlformats.spreadsheetml.sheet`, `org.duckdb.database`, `public.database` (SQLite) and `public.folder`. It does not register `public.zip-archive`.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test open_gestures`. Expected: FAIL.
- [ ] **Step 3:** Implement `mac_open.rs` with `objc2` (declare a handler class whose methods read the event's direct object, a file list or a URL string). Add the plist entries, importing the Parquet and DuckDB types (`UTImportedTypeDeclarations`) since macOS ships no UTType for them.
- [ ] **Step 4:** Run the tests. Expected: PASS. Hand check, recorded in the pull request's checklist: on a packaged build, double-click a CSV after Open With → Always, drop a folder on the Dock icon, and drop a CSV on the door.
- [ ] **Step 5:** Commit: "Open files from Finder, the Dock and the window, and Cmd+O from anywhere".

### Task 4: Cmd+S saves the Protocol from anywhere

**Files:**
- Modify: `crates/brightfield-keys/src/registry.rs:507-517` (`save-protocol` on `global("cmd-s")`; the editor's `save-spec` keeps `editor("cmd-s")` and then raises `save-protocol`), `crates/brightfield-shell/src/window.rs:2195-2240` (`consume_token` maps `cmd-s`), `crates/brightfield-shell/src/editor.rs:1217`
- Test: `crates/brightfield-shell/tests/cmd_s_saves_the_protocol.rs`

**Interfaces:**
- Consumes: `MeridianApp::save_protocol` (exists, `window.rs:10000`).

- [ ] **Step 1: Write the failing tests:**
  - `cmd_s_on_the_dashboard_records_a_kept_filter`: keep a grid filter, press Cmd+S with the canvas focused. `arcform.yaml` gains the filter step and the title loses `UNSAVED_MARK`.
  - `cmd_s_in_the_editor_saves_the_buffer_then_the_protocol`: both files change.
  - `cmd_s_with_nothing_pending_writes_nothing`: the modification times are unchanged.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test cmd_s_saves_the_protocol`. Expected: FAIL on the first.
- [ ] **Step 3:** Bind and dispatch as listed under Files. Also: after a save into a one-source own-files Protocol, the spine shows the recorded step and the run strip adopts the run record (today re-adoption reads the old one-step shape and goes stale); add `cmd_s_on_a_one_source_protocol_shows_the_step_on_the_spine`.
- [ ] **Step 4:** Run the test, then `cargo test -p brightfield-keys`. Expected: PASS.
- [ ] **Step 5:** Commit: "Save the Protocol with Cmd+S wherever the focus is".

### Task 5: The close question on every way out, and a discard keeps every draft

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs:8062-8206` (the close question), `:9896` and `:11057` (Home and open ask rather than block), `crates/brightfield-shell/src/app.rs:2193-2221` (`keep_unsaved_beside` records the grid view and kept SQL too), `crates/brightfield-protocol/src/` (`record_unsaved` takes the Protocol's pending steps)
- Test: `crates/brightfield-shell/tests/close_question_every_way_out.rs`

**Interfaces:**
- Produces: `enum Leaving { Close, Home, Open(OpenRequest), Quit }`. `MeridianApp::leave(&mut self, ctx, to: Leaving)` asks when `has_unsaved_work()`, and otherwise acts at once.

- [ ] **Step 1: Write the failing tests:**
  - `home_with_a_draft_asks`
  - `opening_another_protocol_with_a_draft_asks`, where the queued open happens after "Close without saving"
  - `quit_with_a_draft_asks_and_waits`: a `ViewportCommand::Close` from Cmd+Q is cancelled until answered.
  - `quit_during_a_run_keeps_the_run`: start a run, quit, press Esc (keep editing), and the run's record still lands.
  - `discard_records_the_kept_filter_and_typed_sql`: the Versions panel lists a `closed unsaved` entry naming both.
  - `enter_saves_and_leaves`
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test close_question_every_way_out`. Expected: FAIL.
- [ ] **Step 3:** Implement `leave` and route Home, open, close and quit through it.
- [ ] **Step 4:** Run the test, then `close_question_keeps_unsaved` and `chart_save_close_asks`. Expected: PASS.
- [ ] **Step 5:** Commit: "Ask before Home, another open or quit loses a draft, and keep every draft a discard drops".

### Task 6: Rename, Location, and a Protocol moved in Finder

**Files:**
- Create: `crates/brightfield-shell/src/rename.rs`
- Modify: `crates/brightfield-shell/src/window.rs` (the rename field in the locator band and on a door row), `crates/brightfield-keys/src/registry.rs` (`rename-protocol` on a chord free in its contexts: `cmd-r` is taken by `reload-spec`, so use F2 if the key parser takes function keys, else a free `cmd-shift` chord; plus a registry test that fails when two verbs share a chord in overlapping contexts), `crates/brightfield-workbench/src/persist.rs` (`Recent` repointed)
- Test: `crates/brightfield-shell/tests/rename_protocol.rs`

**Interfaces:**
- Produces:
  - `rename::rename(dir: &Path, new_name: &str) -> Result<PathBuf, String>`. It renames the folder within its parent (refusing a name that exists, empty, or holding `/`), rewrites `name:` in `arcform.yaml`, calls `repair(&new_dir, dir)`, and moves arc's database for the new path, or deletes it so the next run rebuilds it.
  - `rename::repair(dir: &Path, was: &Path) -> Result<usize, String>`. It rewrites relative source paths that leave the folder, in `packages/*/datapackage.json`, `.sources.json` and `model.ossie.yaml`, and returns the count rewritten.
  - `rename::relocate(dir: &Path, new_parent: &Path) -> Result<PathBuf, String>`: moves the folder to another parent (claiming `-2` on a clash), sharing rename's manifest, repair and database steps, and refusing a move into the Protocol itself.
  - `rename::moved_since(dir: &Path) -> Option<PathBuf>`: the path the Protocol was last opened at, read from `.sources.json`'s recorded `dir`, when it differs from `dir`.

- [ ] **Step 1: Write the failing tests:**
  - `rename_renames_the_folder_and_the_protocol_still_runs`: after a rename, a run succeeds and the old folder is gone.
  - `rename_keeps_recents_and_versions`: the recents row and the version history are under the new path.
  - `rename_to_a_taken_name_is_refused`: the folder is unchanged.
  - `a_protocol_moved_in_finder_finds_its_sources`: move the folder into a subfolder, open it, and its loads run with the paths repaired.
  - `rename_is_kept_at_once`: no unsaved mark after a rename.
  - `relocate_moves_the_protocol_and_it_still_runs`, and `the_location_field_moves_it`: the locator band's Location field calls `relocate`.
  - `relocate_across_volumes_copies_then_removes`: when `rename` fails with a cross-device error, it copies, checks, then removes the original.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test rename_protocol`. Expected: FAIL.
- [ ] **Step 3:** Implement. The window's open path calls `moved_since` and `repair` before `reopen`.
- [ ] **Step 4:** Run the test. Expected: PASS.
- [ ] **Step 5:** Commit: "Rename a Protocol and its folder, and find a moved Protocol's sources".

### Task 7: The front door: Recents and the tree, with current run state

**Files:**
- Create: `crates/brightfield-shell/src/door_tree.rs`
- Modify: `crates/brightfield-shell/src/window.rs:11198-11214`, `:12444-12474` (the door's rows), `crates/brightfield-workbench/src/subject.rs:365-418` (a `Previewed` state and its words)
- Test: `crates/brightfield-shell/tests/door_tree.rs`, with baselines `door_tree_light.png` and `door_tree_dark.png`

**Interfaces:**
- Produces:
  - `enum Node { Group { name: String, children: Vec<Node> }, Protocol { name: String, dir: PathBuf } }`
  - `door_tree::read(root: &Path) -> Vec<Node>`: groups first, then by name. It does not descend into a Protocol, skips hidden folders, and returns an empty list for a missing root.
  - `door_tree::run_state(dir: &Path) -> RunState`, read from the newest run record when drawn. `RunState::Previewed` is the state of a Protocol with no run record whose loads have computed; it reads "previewed, not run".

- [ ] **Step 1: Write the failing tests:**
  - `groups_nest_and_protocols_are_leaves`: a `<root>/Work/2026/q1` Protocol, plus a Protocol holding a subfolder with its own `arcform.yaml`, gives one leaf for the latter.
  - `a_new_protocol_is_on_the_door_before_any_save`
  - `a_run_updates_the_row_without_a_save`
  - `a_previewed_protocol_reads_previewed_not_run`
  - `a_folder_deleted_in_finder_drops_its_row_and_an_open_window_says_so`
  - `old_own_protocols_stay_in_recents`: a fixture under `<datasets>/protocols/own` is listed in Recents.
  - The baselines in both themes.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test door_tree`. Expected: FAIL.
- [ ] **Step 3:** Implement. The door draws Recents (existing rows, their run state now from `run_state`) and then the tree, reread on each door frame that follows a hidden-to-shown change.
- [ ] **Step 4:** Run the test with `UPDATE_SNAPSHOTS=1` once, look at both pictures, and run it again without. Expected: PASS.
- [ ] **Step 5:** Commit: "Show Recents and the Protocols folder's tree on the door, with each row's run state as it is".

### Task 8: Relaunch restores the open Protocol

**Files:**
- Modify: `crates/brightfield-workbench/src/persist.rs:216` (`opened` holds `Opened::Start(String)` or `Opened::Protocol(PathBuf)`, reading the old string form as `Start`), `crates/brightfield-shell/src/startup.rs:218-249`, `crates/brightfield-shell/src/window.rs` (opening any Protocol sets it; closing to the door clears it)
- Test: `crates/brightfield-shell/tests/relaunch_restores.rs`

- [ ] **Step 1: Write the failing tests:**
  - `relaunch_reopens_the_protocol_as_saved`: open, keep a filter without saving, drop the app, and boot a new one from the same config. The Protocol opens without the filter.
  - `closing_to_the_door_relaunches_on_the_door`
  - `an_old_layout_file_with_a_start_id_still_restores_it`
  - `a_restored_protocol_that_was_deleted_opens_the_door`
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test relaunch_restores`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the test, then `cargo test -p brightfield-workbench`. Expected: PASS.
- [ ] **Step 5:** Commit: "Reopen the Protocol that was open at the last quit".

### Task 9: Remove a step

**Files:**
- Modify: `crates/brightfield-shell/src/protocol.rs` (`remove-step` on a spine row), `crates/brightfield-shell/src/app.rs` (a pending removal is unsaved work), `crates/brightfield-shell/src/window.rs` (`save_protocol` applies it through `arc`), `crates/brightfield-keys/src/registry.rs` (`remove-step`, `protocol("d d")`)
- Test: `crates/brightfield-shell/tests/remove_step.rs`

**Interfaces:**
- Produces: `ProtocolModel::readers_of(&self, step: &str) -> Vec<String>`, the later steps that read its assets.

- [ ] **Step 1: Write the failing tests:**
  - `removing_a_typed_sql_step_is_a_draft_until_save`: the row is struck through and the title is marked. Cmd+S removes it from `arcform.yaml` and deletes its model file.
  - `a_load_step_and_the_model_step_cannot_be_removed`: the verb is not offered.
  - `a_step_others_read_is_refused_naming_them`: the message names each reader.
  - `close_without_saving_keeps_the_step`
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test remove_step`. Expected: FAIL.
- [ ] **Step 3:** Implement. Apply the removal with arc's own step removal if arc has one; otherwise edit `arcform.yaml` by patching, never by regenerating it, and keep its comments.
- [ ] **Step 4:** Run the test. Expected: PASS.
- [ ] **Step 5:** Commit: "Remove a step from the spine, kept as a draft until Save".

### Task 10: New Protocol, and adding a source to an open one

**Files:**
- Modify: `crates/brightfield-shell/src/own_files.rs` (an empty Protocol; `add_source`), `crates/brightfield-shell/src/window.rs` (New Protocol on the door; `AddSource` from the queue; canvas selection follows the spine)
- Test: `crates/brightfield-shell/tests/new_protocol.rs`

**Interfaces:**
- Consumes: `protocols_dir::claim`, `rename::rename`.
- Produces:
  - `own_files::create_empty(parent: &Path) -> Result<OwnProtocol, String>`, named `Untitled`, `Untitled-2`, …
  - `own_files::add_source(p: &mut OwnProtocol, path: &Path, joins: &[AcceptedJoin]) -> Result<(), String>`. It appends load steps, rewrites `.sources.json`, reruns discovery when there are two or more tables, writes the accepted joins into the model, and is written at once.
  - `struct AcceptedJoin { left: (String, String), right: (String, String) }`, each side a table and a column.

- [ ] **Step 1: Write the failing tests:**
  - `new_protocol_asks_where_with_the_protocols_folder_preselected`: the chooser lists the Protocols folder, its groups, and Other…; the Protocol is made where chosen.
  - `new_protocol_makes_untitled_and_opens_the_browser`
  - `the_first_source_names_an_untitled_protocol`: the folder becomes `earthquakes_2024`.
  - `a_renamed_untitled_keeps_its_name_when_a_source_is_added`
  - `a_second_source_switches_the_canvas_to_the_model`
  - `selecting_a_source_on_the_spine_shows_its_grid`
  - `adding_a_source_leaves_no_unsaved_mark`
  - `adding_writes_no_run_record`
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test new_protocol`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the test. Expected: PASS.
- [ ] **Step 5:** Commit: "Make an empty Protocol from the door and add sources to an open one, kept at once".

### Task 10b: Preferences, with the Protocols folder

**Files:**
- Create: `crates/brightfield-shell/src/preferences.rs` (the sheet, in the window's theme tokens)
- Modify: `crates/brightfield-shell/src/protocols_dir.rs` (`root()` reads the env var, then the setting, then the default), `crates/brightfield-workbench/src/persist.rs` or the config the window already keeps (the setting), `crates/brightfield-keys/src/registry.rs` (`open-preferences` on `global("cmd-,")`), `crates/brightfield-shell/src/window.rs`
- Test: `crates/brightfield-shell/tests/preferences.rs`

**Interfaces:**
- Consumes: `rename::relocate` (Task 6), `door_tree::read` (Task 7).
- Produces: `protocols_dir::set_root(path: &Path) -> Result<(), String>` and `protocols_dir::move_all(from: &Path, to: &Path) -> MoveReport { moved: Vec<PathBuf>, failed: Option<(PathBuf, String)>, left_open: Option<PathBuf> }`.

- [ ] **Step 1: Write the failing tests:**
  - `cmd_comma_opens_preferences`
  - `the_setting_sends_new_protocols_to_the_new_folder`
  - `the_env_var_still_wins_over_the_setting`
  - `changing_the_folder_offers_to_move_and_declining_moves_nothing`: old Protocols stay, still in Recents.
  - `accepting_moves_each_protocol_and_they_still_run`
  - `a_failed_move_stops_and_names_what_moved`
  - `an_open_protocol_is_moved_after_the_close_question_or_left`
  - `the_door_tree_reads_the_new_folder`
  - `a_refused_new_folder_says_how_and_keeps_the_old`: the Privacy & Security sentence, as in Task 1.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test preferences`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the test. Expected: PASS.
- [ ] **Step 5:** Commit: "Choose the Protocols folder in Preferences, and offer to move what was in the old one".

**Checkpoint.** Tasks 1 to 10b cover the gestures, Cmd+S, rename, the door, relaunch, step removal and adding sources by drop. Push, and offer Hugh a build to drive before the browser.

### Task 11: The sources browser's state and keys

**Files:**
- Create: `crates/brightfield-shell/src/sources_browser.rs`
- Modify: `crates/brightfield-keys/src/registry.rs` (a `BindingContext::Browser` with the spec's table of keys, goal 5, and `A` and `o` in `Protocol`)
- Test: unit tests in `sources_browser.rs`

**Interfaces:**
- Produces:
  - `struct Browser { place: usize, dir: PathBuf, cursor: usize, marks: BTreeSet<PathBuf>, focus: Pane, glob: Option<String>, preview: PreviewMode, full: bool }`
  - `enum Pane { Places, List, Canvas }`
  - `enum PreviewMode { Canvas, Files }`
  - `Browser::open_at(first_source: Option<&Path>) -> Browser`: the first source's folder with it under the cursor, or the home folder.
  - `Browser::key(&mut self, verb: &str, fs: &dyn Listing) -> Effect`
  - `enum Effect { None, Add(Vec<PathBuf>, Vec<AcceptedJoin>), Close }`
  - `trait Listing { fn list(&self, dir: &Path) -> Vec<Entry>; }`, so tests use an in-memory disk.
  - The canvas's join cursor and accepted set: `Browser::joins: Vec<(ProposedJoin, bool)>`, `Browser::join_cursor: usize`.

- [ ] **Step 1: Write the failing tests**, one per row of the spec's key table:
  - `l_and_enter_go_into_a_folder`
  - `l_and_enter_add_a_file`
  - `h_goes_up`
  - `a_adds_the_highlighted_folder_with_its_accepted_joins`
  - `a_adds_the_marked_files_across_folders` (marks survive a change of folder)
  - `space_marks_a_file`
  - `space_on_the_canvas_toggles_the_join`
  - `j_k_on_the_canvas_move_between_joins`
  - `tab_cycles_places_list_canvas`
  - `slash_glob_counts_matches`
  - `g_t_fills`
  - `v_switches_preview`
  - `enter_on_the_canvas_adds_the_folder`
  - `esc_closes`
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --lib sources_browser`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the same test, then `cargo test -p brightfield-keys`. Expected: PASS.
- [ ] **Step 5:** Commit: "The sources browser's state and keys".

### Task 12: Drawing the sources browser

**Files:**
- Create: `crates/brightfield-shell/src/sources_browser_view.rs`
- Modify: `crates/brightfield-shell/src/window.rs` (show it from New Protocol, `A`, and a dropped folder; `Effect::Add` goes to `own_files::add_source`)
- Test: `crates/brightfield-shell/tests/sources_browser_view.rs`, with baselines for a CSV preview and a folder's canvas, each in light and dark

**Interfaces:**
- Consumes: `Browser` (Task 11), `discovery::Dovetail` (the sample pass that exists today) for a folder's proposed joins, and `own_files::add_source` (Task 10).
- Produces: `sources_browser_view::show(ui: &mut egui::Ui, b: &mut Browser, theme: &Theme) -> Effect`.

- [ ] **Step 1: Write the failing tests:**
  - `a_click_moves_and_a_double_click_acts_as_enter`
  - `a_click_on_a_join_toggles_it`
  - `the_foot_shows_the_keys_for_the_focused_pane` (the list's and the canvas's differ)
  - `a_file_preview_shows_its_first_rows_without_blocking_the_frame`: the frame returns before the read finishes, and the rows appear on a later frame.
  - `a_folder_preview_draws_dovetails_joins`
  - `it_takes_the_window_theme`: the background and ink are read from the theme tokens in both modes.
  - `a_folder_with_a_proposed_join_opens_the_browser_on_it_before_adding`: on every way in (queue `Path` from the picker and from a drop alike), `Route::New` for a folder whose tables dovetail proposes a join between makes the Protocol and opens the browser on its canvas instead of drawing the model.
  - `a_folder_of_one_table_or_no_joins_is_added_at_once`: it opens as Task 2 opens a folder or a lone file, with no browser.
  - The four baselines.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test sources_browser_view`. Expected: FAIL.
- [ ] **Step 3:** Implement. Previews read off the frame thread as the grid's summaries do.
- [ ] **Step 4:** Run the test with `UPDATE_SNAPSHOTS=1` once, look at the four pictures, and run it again without. Expected: PASS.
- [ ] **Step 5:** Commit: "Draw the sources browser, by click and by key, in the window's theme".

### Task 13: The spine leads with the model

**Files:**
- Modify: `crates/brightfield-shell/src/protocol.rs:1729-1872` (`spine()` order and the gutter curves)
- Test: `crates/brightfield-shell/tests/navigator_spine.rs` (extend), with baselines `spine_model_first_light.png` and `spine_model_first_dark.png`

**Interfaces:**
- Produces: `ProtocolModel::spine()` returns the joined tables first, each followed by the tables it joins (breadth-first from the table with the most joins, ties by name), then the model step, then a heading row `SpineRole::Unjoined` and the sources with no join. `SpineRow::join_curves: Vec<(usize, usize)>` gives the row index pairs a curve joins.

- [ ] **Step 1: Write the failing tests:**
  - `joined_tables_lead_and_each_sits_beside_its_partner`: on a three-table star plus two loose CSVs, the order is fact, dimension, dimension, model step, Unjoined, loose, loose.
  - `one_curve_per_join`
  - `a_protocol_with_no_joins_keeps_todays_order`
  - The two baselines.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test navigator_spine`. Expected: the new tests FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the test with `UPDATE_SNAPSHOTS=1` once, look at both pictures, and run it again without. Expected: PASS.
- [ ] **Step 5:** Commit: "Lead the spine with the model's tables and draw a curve per join".

### Task 14: Drops onto an open Protocol

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs` (`dropped_files` with a Protocol open: data pushes `AddSource`, a folder in which dovetail proposes a join opens the browser on it and any other folder is added at once, and a Protocol or archive pushes `Path` through `leave`)
- Test: `crates/brightfield-shell/tests/open_gestures.rs` (extend)

- [ ] **Step 1: Write the failing tests:**
  - `a_csv_dropped_on_an_open_protocol_is_added_as_a_source`
  - `a_folder_with_a_join_dropped_on_an_open_protocol_opens_the_browser_on_it`
  - `a_folder_of_one_table_dropped_on_an_open_protocol_is_added_at_once`
  - `a_protocol_dropped_on_an_open_protocol_opens_it_after_the_question`
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test open_gestures`. Expected: the new tests FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the test. Expected: PASS.
- [ ] **Step 5:** Commit: "Add a dropped file to the open Protocol, or open a dropped Protocol".

### Task 15: Open in Brightfield: the link, the archive and trust

**Files:**
- Create: `crates/brightfield-shell/src/protocol_archive.rs`
- Modify: `crates/brightfield-shell/src/window.rs` (`OpenRequest::Link` and `Route::Archive`; a progress line while downloading)
- Test: `crates/brightfield-shell/tests/protocol_archive.rs`

**Interfaces:**
- Consumes: `remote::Fetch::begin` (exists), `protocols_dir::claim`, and the workspace's `zip` (version 6, as `Cargo.toml` pins it).
- Produces:
  - `protocol_archive::check_link(link: &str) -> Result<url::Url, String>`. Accepted: scheme `brightfield`, host `open`, one `url` parameter that is `https` with host `meridian.online` or ending `.meridian.online`.
  - `protocol_archive::unpack(zip: &Path, into: &Path) -> Result<PathBuf, String>`. It checks every entry first: one top-level folder holding `arcform.yaml`; no absolute path, `..`, or symlink; at most 10,000 entries and 2 GiB unpacked by the sizes the headers declare. It then claims a name and unpacks, counting the bytes actually written and stopping at the same 2 GiB, removing the partial folder, since headers can understate sizes; and refuses a Protocol whose `.sources.json` or load SQL names an absolute local path.

- [ ] **Step 1: Write the failing tests:**
  - `a_meridian_link_is_accepted`: `brightfield://open?url=https://meridian.online/protocols/usgs/earthquakes_2025.zip`.
  - `a_subdomain_link_is_accepted`: `https://openlake.meridian.online/x.zip`.
  - `other_hosts_and_http_are_refused_with_a_sentence`: `https://evil.example/x.zip`, `http://meridian.online/x.zip`, `https://meridian.online.evil.example/x.zip`.
  - `an_archive_with_dotdot_or_absolute_entries_writes_nothing`
  - `an_archive_past_the_limits_writes_nothing`
  - `an_archive_that_understates_its_sizes_is_stopped_and_removed`: declared sizes under the limit, real contents over it; nothing is left behind.
  - `an_archive_naming_an_absolute_local_path_is_refused_naming_it`
  - `a_clashing_archive_takes_dash_two`
  - `a_source_path_escaping_the_folder_is_refused`: a relative `../..` source in `.sources.json` or a package.
  - `a_redirect_off_meridian_online_is_refused`: the host is checked again after each redirect.
  - `an_archived_protocol_opens_restricted_and_draws_its_shipped_results`: no run record is written, no fetch happens, the banner names the host, and the charts draw from the archive's results.
  - `trust_unlocks_running_and_is_remembered`: Trust, then Run writes a run record; reopening the folder is not restricted.
  - `a_protocol_folder_brightfield_did_not_make_opens_restricted`, wherever it sits; one Brightfield made opens trusted.
  - `an_unpacked_archive_in_the_protocols_folder_is_not_trusted_by_location`
  - `a_brightfield_made_protocol_in_recents_from_before_trust_opens_trusted`; a foreign folder in Recents opens restricted.
  - `a_brightfield_made_protocol_moved_in_finder_stays_trusted`
  - `trusting_a_folder_trusts_protocols_beneath_it_including_later_ones`: trust the Protocols folder, then unpack an archive into it; it opens trusted.
  - `removing_a_trusted_folder_restricts_what_it_covered`
  - `restricted_drawing_reads_only_shipped_results`: with DuckDB external access and extension autoload off.
  - `a_good_archive_unpacks_and_draws`: serve a fixture zip from a local stand-in HTTP server so the fetch path runs (`Fetched::stand_in` copies files without the network and does not exercise it); it unpacks, opens restricted and draws its shipped results.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test protocol_archive`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the test. Expected: PASS. Hand check on a packaged build, in the pull request's checklist: `open 'brightfield://open?url=…'` from Terminal opens the Protocol.
- [ ] **Step 5:** Commit: "Open a Protocol from a meridian.online link".

### Task 16: Fold the design into the docs, and retire the spec and plan

**Files:**
- Modify: `README.md` (the opening section: Protocols folder, gestures, the browser, the save split, the link format for the website), `CHANGELOG.md`
- Delete: `docs/superpowers/specs/2026-11-08-opening-and-saving-design.md`, `docs/superpowers/plans/2026-11-08-opening-and-saving.md`

- [ ] **Step 1:** Write the README section with each of Hugh's rulings and its reason, in plain words: the spec's "What Hugh decided, and why" and the calls he approved. State the link and archive format exactly, since the website builds against it.
- [ ] **Step 2:** Add a CHANGELOG entry.
- [ ] **Step 3:** Delete the spec and plan.
- [ ] **Step 4:** Run `cargo xtask ci --step comment-citations` and the docs steps `cargo xtask ci --list` names. Expected: PASS.
- [ ] **Step 5:** Commit: "Document opening and saving, and retire the spec and plan".

## Verification costs

| Command | What it covers | Cost on Hugh's Mac |
|---|---|---|
| `cargo test -p brightfield-shell --test <name>` | one task's tests | 1 to 4 minutes after the first build; the first build after a clean takes about 15 |
| `cargo test -p brightfield-shell --lib sources_browser` | Task 11 | under a minute after a build |
| `cargo xtask ci --step fmt` / `--step clippy` | before each commit | seconds / 2 to 5 minutes |
| `cargo xtask ci --step comment-citations` | after each commit | seconds |
| `cargo xtask ci` | the full gate; CI runs it on each push | 4 to 55 minutes locally; run only to reproduce a CI failure |
| Packaged-build hand checks | Tasks 3 and 15 | about 10 minutes, Hugh driving |

Dependencies: Task 2 needs 1. Tasks 3, 4, 5, 6 and 7 need 2. Task 8 needs 7. Task 9 needs 5. Task 10 needs 1, 2 and 6. Task 10b needs 6 and 7. Task 11 needs nothing. Task 12 needs 10 and 11. Task 13 needs 10. Task 14 needs 3, 5 and 12. Task 15 needs 2 and 3. Task 16 is last.
