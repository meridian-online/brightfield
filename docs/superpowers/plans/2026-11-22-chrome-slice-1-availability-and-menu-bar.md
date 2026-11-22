# Chrome slice 1: availability declarations and the menu bar — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every command in the key registry declares the inputs it needs and the sentence it says when one is missing; the palette and a menu bar built from the same command list read that declaration; and every key the menus and the help sheet list is answered by a handler.

**Architecture:** The declaration lives on `VerbEntry` in `crates/brightfield-keys`, framework-free, beside a `Situation` the shell fills each frame and an `availability()` that reads one against the other. That crate also holds the menu model (the eight menus as data) and a keystroke parser that turns registry tokens into labels. The shell builds the `Situation`, greys palette rows with the declared sentence, carries a chosen menu item to its handler through the open queue, draws the menus natively on macOS (an `NSMenu` per menu that claims no keys) and from the title band's mark elsewhere. Two sweeps hold the result: every listed key is answered by a handler, and every menu item acts where it is available. Slice 2's ribbon reads the same `MeridianApp::availability_of` and `brightfield_keys::menu_bar`-style data, so nothing here is drawn twice.

**Tech Stack:** Rust; egui/eframe 0.35 (`Popup::menu`, `Ui::menu_button`, `Button::shortcut_text`, `Response::on_disabled_hover_text`); objc2 0.6 and objc2-foundation 0.3 messages sent by selector, as `crates/brightfield-shell/src/mac_open.rs` sends them.

**Spec:** `docs/superpowers/specs/2026-11-21-brightfield-chrome-design.md`: slice 1 of item 4 under "Sequencing", defined by "1. The menu bar" and the "Availability is declared by the command" bullet of "3. The ribbon's command row". Read the spec's Goals, Anti-goals and Constraints before any task.

## Global Constraints

- "Every key binding is a default that an analyst can change in a keymap file, through the key registry (`crates/brightfield-keys`)." No key is bound outside the registry. The native menus this plan adds claim no key: every key reaches the window as it does today.
- "The same command list feeds the menu bar, the ribbon, the palette and the keymap, so the four can't disagree." Menus, palette and help are projections of `brightfield_keys::registry()`; no title, key or sentence is spelled a second time in the shell.
- "The ribbon lists commands that exist. A command joins it when the feature behind it is built. Greyed means a missing input, never a missing feature." The menus list built commands only; a reserved verb is in no menu.
- The menus, in order: **Brightfield, File, Edit, View, Protocol, Data, Window, Help.**
- "macOS is the validated platform. Linux and Windows have no global menu bar, so the same menus must be reachable in the window there." On those hosts they open from the mark at the left of the title band.
- Anti-goals: "Keys that look live and do nothing. Every key the chrome shows must act." and "A greyed command that doesn't say why."
- "Slice 1 needs no new components." Draw with egui's own menu widgets and today's `PickerRow`; no stand-in of a design-system component (reason popover, menu row, ribbon command) is drawn in Brightfield.
- `brightfield-keys` takes no UI-framework dependency.
- `unsafe` is written only in the `mac_*` modules, each declared with `#[allow(unsafe_code)]` in `crates/brightfield-shell/src/lib.rs`.
- This repo is public: no issue-tracker refs, decision ids or milestone ids in code, comments, commits or docs; paths are repo-relative.
- Before each commit run `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy` and the tests the task touches; after each commit run `cargo xtask ci --step comment-citations`, which checks that a test a comment cites exists. One cargo build at a time per session, and parallel sessions only within the Mac Studio's three-sessions-building-Rust cap. No pixel baseline changes in this slice; baselines are drawn only on the Mac Studio.
- Copy: labels are sentence case, the command's action in a few words, with no trailing punctuation; a menu adds "…" to a command that asks for more before it acts. Sentences end with a full stop.

## Review Focus

1. **A native menu swallowing a key the window needs**, such as ⌘S while the YAML editor holds the keys, or a bare letter shown as a menu key typed into the grid's where field. Expected: the key reaches the editor or field as it does today. Pinned in Task 9 by `the_menus_claim_no_key_so_every_key_reaches_the_window`, and on device by Task 9's checklist.
2. **A menu item chosen behind a sheet**, such as Run chosen from the native menu while the close question is up. Expected: nothing acts behind the sheet, and every menu item reads greyed with the sheet's sentence while it is up. Pinned in Task 8 by `no_menu_command_acts_behind_the_close_question`.
3. **A menu item that went stale between the menu opening and the click**, such as Undo after the kept column was already taken back by `u`. Expected: the window says the command's sentence in a toast and does nothing. Pinned in Task 8 by `a_command_chosen_where_it_is_unavailable_says_its_sentence_and_does_nothing`.
4. **A newly wired key pressed while a text field holds the keys**, Esc above all, since it is also the field's way out. Expected: Esc with the grid foot's field open puts the field back and keeps the chart's selection. Pinned in Task 6 by `esc_with_the_grid_foot_field_open_keeps_the_selection`.
5. **The front door.** Expected: every command that needs a Protocol is greyed with its sentence, nothing acts on nothing, and on a host with no menu bar the mark is drawn on the door so the menus are reachable there. Pinned in Task 8 by `on_the_front_door_every_command_that_needs_a_protocol_is_greyed` and in Task 10 by `the_mark_is_drawn_on_the_front_door_where_it_opens_the_menus`.

---

## The code survey, verified

Each key below is listed live today by the help sheet and the palette. "No consume site" means `consume_token` in `crates/brightfield-shell/src/window.rs` maps no token for it and no other handler reads it.

| Key | Verb | Context | What the code does | Fate |
|---|---|---|---|---|
| `m` `a` `d` | `change-mark-type`, `add-mark`, `remove-mark` | Workspace | no consume site; the overlays module says the chart's editing bridge "is not landed" | reserved, needs chart editing (Task 2) |
| `c` | `cycle-colour-scheme` | Workspace | no consume site | reserved, needs chart editing (Task 2) |
| `p` | `toggle-presentation` | Workspace | no consume site; `inspector.rs`'s tests use it as the verb with no arm | reserved, needs the workspace shell (Task 2) |
| `h` `j` `k` `l` and their twins `Enter` `q` `Tab` `⇧Tab` | `dive-in`, `pop-out`, `focus-next-sibling`, `focus-prev-sibling` | Workspace | the shell keeps no focused plot; the keys crate's `FocusTree` has no caller in the shell | reserved, needs plot focus (Task 2) |
| ⌘E | `toggle-focus` | Global | no consume site | reserved, needs the workspace shell (Task 2) |
| ⌘R | `reload-spec` | Global | no consume site and no spec reload to call | reserved, needs the workspace shell (Task 2) |
| ⌘⇧P | `open-palette`'s twin | Global | `OverlayKeys` reads only the primary key, Space | wired (Task 6) |
| ⌘⇧U | `untrust-protocol` | Global | runs from the palette through `apply`; no key handler, though the README documents the key | wired (Task 6) |
| ⌘⇧R | `reload-data` (not in the survey) | Global | runs from the changed-on-disk notice through `apply`; no key handler | wired (Task 6) |
| Esc | `clear-selection` (not in the survey) | Workspace | `tile_back_key` stands down for a held selection, but no handler takes the key | wired (Task 6) |
| `e` | `set-channel` | Workspace | live: `shelf_entry_key` hands the keys to the shelf | unchanged |
| `/` | `focus-jump` | Workspace | live only with the graph on the canvas | declares `Selected::Graph` (Task 3) |

Every other context is table-driven from the registry (Protocol, Shelf, Grid, Versions, Model, Browser, Editor); the sweeps in Tasks 6 and 7 hold them all.

## The declarations

`{command}` in a sentence is replaced by the verb's label. A need's sentence is the default below unless the verb's row gives its own.

| Need | Default sentence |
|---|---|
| `Sources(2)` | "{command} needs two sources." |
| `Column(ColumnKind::Any)` | "{command} needs a column under the Outline's cursor." |
| `Selected::Protocol` | "{command} needs a Protocol open." |
| `Selected::Graph` | "{command} needs the Protocol's graph on the canvas." |
| `Selected::Chart` | "{command} needs a chart on the canvas." |
| `Selected::Rows` | "{command} needs a table's rows." |
| `Selected::GridCell` | "{command} needs the grid's cursor on a cell." |
| `Selected::Asset` | "{command} needs an asset selected on the spine." |
| `Selected::Step` | "{command} needs a step selected on the spine that can be removed." |
| `Selected::Model` | "{command} needs the model open." |
| `Selected::ModelTable` | "{command} needs a table selected in the model." |
| `Selected::ModelJoin` | "{command} needs a join selected in the model." |
| `Condition::SelectionHeld` | "{command} needs a selection on the chart." |
| `Condition::EditToTakeBack` | "{command} needs a column kept from the shelf since the last Save." |
| `Condition::UnsavedWork` | "{command} needs a change that is not saved yet." |
| `Condition::ProtocolSource` | "{command} needs a Protocol of your own: open a data file or a folder." |
| `Condition::Trusted` | "{command} needs the Protocol trusted: trust it from the band over its canvas." |
| `Condition::TakesSources` | "Sources are added to Protocols Brightfield made, and this one was not." |
| `Condition::FileToReload` | "{command} needs a data file open." |
| `Condition::ShelfDrawn` | "{command} needs the chart's shelf." |
| `Grant::FullEdition` | "{command} needs the full edition of Brightfield." |

Needs per verb, in order (the first unmet need's sentence is the one said). A verb not listed declares none; that includes every verb bound only in the Shelf, Grid-cursor, Versions and Browser contexts and the Protocol grammar's motions, since holding those keys is their need.

| Verb | Label | Needs (sentence where not the default) |
|---|---|---|
| `open-home` | Home | `Selected::Protocol` ("You are on the front door already.") |
| `rename-protocol` | Rename | `Selected::Protocol` |
| `save-protocol` | Save | `Condition::UnsavedWork` |
| `save-spec` | Save the spec | `Condition::ProtocolSource` |
| `run-protocol` | Run | `Condition::ProtocolSource`, `Condition::Trusted` |
| `untrust-protocol` | Stop trusting | `Selected::Protocol`, `Condition::Trusted` ("This Protocol is not trusted.") |
| `reload-data` | Reload data | `Condition::FileToReload` |
| `copy-chart-picture` | Copy chart as picture | `Selected::Chart` ("Open a chart to copy or export it as a picture.") |
| `export-chart-picture` | Export chart as picture | `Selected::Chart` (same sentence) |
| `move-grid` | Move grid | `Selected::Rows` |
| `focus-jump` | Jump to node | `Selected::Graph` |
| `clear-selection` | Clear selection | `Selected::Chart`, `Condition::SelectionHeld` |
| `pan-left` `pan-right` `pan-up` `pan-down` | Pan left, Pan right, Pan up, Pan down | `Selected::Chart` |
| `zoom-in` `zoom-out` `reset-extent` `cycle-axis-lock` | Zoom in, Zoom out, Full extent, Cycle axis lock | `Selected::Chart` |
| `set-channel` | Go to shelf | `Selected::Chart`, `Condition::ShelfDrawn` |
| `undo` | Undo | `Selected::Chart`, `Condition::EditToTakeBack` |
| `open-steps-sheet` | Steps sheet | `Selected::Graph` |
| `yank-address` | Copy address | `Selected::Asset` |
| `remove-step` | Remove step | `Selected::Step` |
| `open-sources-browser` | Add sources | `Selected::Protocol`, `Condition::TakesSources` |
| `organise-joins` | Organise joins | `Sources(2)` ("Organising joins needs a Protocol of two sources or more.") |
| `put-column-on-x` `-y` `-colour` | Put column on x, Put column on y, Put column on colour | `Selected::Chart`, `Condition::ShelfDrawn`, `Column(ColumnKind::Any)` |
| `filter-rows-at-cursor` `sort-rows-at-cursor` | Filter rows, Sort rows | `Selected::GridCell` |
| `cycle-model-view` `show-model-diagram` `show-model-list` `show-model-file` `rediscover-model` | Next model view, Model as diagram, Model as list, Model as Ossie file, Rediscover | `Selected::Model` |
| `keep-join` `drop-join` `change-join-keys` | Keep join, Drop join, Change join keys | `Selected::ModelJoin` |
| `add-join` `set-role` `fold-table` `unfold-table` `leave-out-table` `include-table` `retype-column` | Add join, Set role, Fold table, Unfold table, Leave table out, Include table, Retype column | `Selected::ModelTable` |

Labels for the menu commands with no needs: `open` Open, `new-protocol` New Protocol, `open-preferences` Preferences, `quit` Quit Brightfield, `open-palette` Command palette, `open-help` Keyboard shortcuts, `toggle-outline-rail` Go to spine. Every other verb gets a label by the copy rule, unique across the registry (for example `move-cursor-down` Cursor down, `go-to-x-cell` Go to x cell, `close-sources-browser` Close the browser).

## The menus

A row marked "asks" takes "…". Native items are AppKit's own and drawn on macOS only.

| Menu | Items |
|---|---|
| Brightfield | `open-preferences` (asks), `quit` |
| File | `new-protocol` (asks), `open` (asks), `open-home`, —, `save-protocol`, `rename-protocol` (asks), —, `export-chart-picture` (asks) |
| Edit | `undo`, —, `copy-chart-picture`, `yank-address`, —, `clear-selection` |
| View | `open-palette` (asks), —, `zoom-in`, `zoom-out`, `reset-extent`, `cycle-axis-lock`, —, `show-model-diagram`, `show-model-list`, `show-model-file`, —, `open-steps-sheet` |
| Protocol | `run-protocol`, —, `open-sources-browser` (asks), `organise-joins` (asks), `remove-step`, `focus-jump` (asks), —, `reload-data`, `untrust-protocol` |
| Data | `filter-rows-at-cursor` (asks), `sort-rows-at-cursor` (asks), —, `set-channel`, `put-column-on-x`, `put-column-on-y`, `put-column-on-colour`, —, `keep-join`, `drop-join`, `add-join` (asks), `change-join-keys` (asks), —, `set-role` (asks), `fold-table` (asks), `unfold-table`, `leave-out-table`, `include-table`, `retype-column` (asks), —, `rediscover-model` |
| Window | Native Minimize, Native Zoom, —, `toggle-outline-rail`, `move-grid`, —, Native Bring All to Front |
| Help | `open-help` |

A motion (a cursor step, a pan, a sibling move, the shelf's and the browser's own navigation) is not a menu item: it is in the help sheet and the palette. On macOS the Brightfield menu is the app menu winit makes, with Preferences… inserted and Quit routed by `mac_open::route_menu_quit` as today; the model still lists it so the in-window menu draws it and the tests see it.

## Calls this plan makes

The spec leaves these to the builder. Each is a call a reviewer may overturn.

- **A fifth kind of input, conditions,** beside the spec's four (sources, a column, a node, an edition or plan). Undo, Clear selection, Save, Run and Reload depend on what the window holds rather than on any of the four, and without a condition they would show live and do nothing.
- **"A plan" is read as an edition or a plan**, one `Grant` with one variant, the full edition. No command declares it yet.
- **Reserve rather than wire** the chart's mark verbs, its colour preview, presentation, plot focus, ⌘E and ⌘R: the features behind them are not built, and "a command joins it when the feature behind it is built". The four keys whose handlers already exist (⌘⇧P, ⌘⇧U, ⌘⇧R, Esc) are wired.
- **The native menus claim no keys.** Each key keeps the one path it has today, through the window's handlers. The cost: Minimize carries no ⌘M.
- **Menus hold commands, not motions.**
- **Labels are sentence case**, as the spec's mockups write them; a menu adds "…".
- **On a host with no menu bar, the mark opens the menus** and Home is reached from File and by ⇧⌘H. The spec's title band has the mark lead home, and on such a host it cannot do both.
- **The palette lists a command whose input is missing, with its sentence**, where it used to hide the row: the Outline's put rows over the graph, Remove step with nothing removable, and Save and Run on a chart start.
- **While a sheet is over the window, every menu item is greyed** with one sentence.
- **The help sheet drops reserved verbs; the palette keeps listing them, flagged,** as it did.

## File structure

- `crates/brightfield-keys/src/keystroke.rs` (new): registry tokens parsed into strokes, and their labels per platform.
- `crates/brightfield-keys/src/availability.rs` (new): `Need`, `Situation`, `availability()`, `sentence()`.
- `crates/brightfield-keys/src/menus.rs` (new): the eight menus as data.
- `crates/brightfield-keys/src/registry.rs`: `label` and `needs` on every entry; the unbuilt verbs reserved; the help sheet lists built verbs only.
- `crates/brightfield-keys/src/palette.rs`: the filter greys a row with its sentence when given a `Situation`.
- `crates/brightfield-shell/src/menu_bar.rs` (new): menu rows with their availability, the state the native menu reads, and the in-window menu's drawing.
- `crates/brightfield-shell/src/mac_menu.rs` (new, macOS): the seven native menus and their delegate.
- `crates/brightfield-shell/src/window.rs`: the `Situation` builder, the key and verb records, the four wired keys, the menu verbs' arms in `apply`, and the mark's menu.
- `crates/brightfield-shell/src/open_queue.rs`: verbs chosen outside the frame.
- `crates/brightfield-shell/tests/support/scenes.rs` (new): the windows and arrangements the sweeps choose from.
- `crates/brightfield-shell/tests/availability.rs`, `keys_answered.rs`, `menu_commands.rs`, `mark_menu.rs` (new).

---

### Task 1: Keystrokes and their labels

**Files:**
- Create: `crates/brightfield-keys/src/keystroke.rs`
- Modify: `crates/brightfield-keys/src/lib.rs` (module and re-exports)

**Interfaces:**
- Produces: `pub struct Keystroke { pub key: &'static str, pub command: bool, pub control: bool, pub alt: bool, pub shift: bool }` (derive `Debug, Clone, Copy, PartialEq, Eq`); `pub enum KeyPlatform { Mac, Other }`; `pub fn parse_keystrokes(keystrokes: &'static str) -> Vec<Keystroke>`; `impl Keystroke { pub fn label(&self, platform: KeyPlatform) -> String }`; `pub fn keys_label(keystrokes: &'static str, platform: KeyPlatform) -> String` (each stroke's label, joined by a space); `pub const KEY_NAMES: &[&str]` (every key name a binding may use).

- [ ] **Step 1: Write the failing tests** in `keystroke.rs`:

```rust
#[test]
fn modifiers_come_off_the_front_and_the_rest_is_the_key() {
    assert_eq!(parse_keystrokes("cmd-shift-p"), [Keystroke { key: "p", command: true, shift: true, ..plain("p") }]);
    assert_eq!(parse_keystrokes("-")[0].key, "-");      // zoom-out's key is the minus itself
    assert_eq!(parse_keystrokes("cmd-,")[0].key, ",");
    assert_eq!(parse_keystrokes("shift-tab")[0], Keystroke { shift: true, ..plain("tab") });
    assert_eq!(parse_keystrokes("d d").len(), 2);
}
#[test]
fn mac_labels_use_the_menu_glyphs_in_apples_order() {
    // Apple's order is ⌃ ⌥ ⇧ ⌘, then the key.
    for (keys, want) in [("cmd-shift-p", "⇧⌘P"), ("cmd-enter", "⌘↩"), ("shift-t", "⇧T"), ("escape", "⎋"),
                         ("space", "Space"), ("f2", "F2"), ("?", "?"), ("cmd-,", "⌘,"), ("d d", "D D")] {
        assert_eq!(keys_label(keys, KeyPlatform::Mac), want);
    }
}
#[test]
fn other_labels_spell_ctrl_and_the_key_names() {
    for (keys, want) in [("cmd-shift-p", "Ctrl+Shift+P"), ("cmd-enter", "Ctrl+Enter"), ("escape", "Esc"), ("shift-t", "Shift+T")] {
        assert_eq!(keys_label(keys, KeyPlatform::Other), want);
    }
}
#[test]
fn every_registry_binding_parses_to_a_known_key() {
    for v in crate::registry() { for b in &v.binding_specs { for s in parse_keystrokes(b.keystrokes) {
        assert!(KEY_NAMES.contains(&s.key), "{} binds unknown key {:?}", v.longname, s.key);
    }}}
}
```

`plain(key)` is a test helper returning a `Keystroke` with no modifier.

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-keys keystroke` Expected: FAIL, `parse_keystrokes` not found.
- [ ] **Step 3: Implement.** A stroke strips the prefixes `cmd-`, `ctrl-`, `alt-`, `shift-` while one matches; what is left is the key, so `-` alone is a key. `cmd` is ⌘ on the Mac and Ctrl elsewhere, as egui's `Modifiers::COMMAND` reads it. Letters are drawn upper-case; `enter` ↩/Enter, `escape` ⎋/Esc, `tab` ⇥/Tab, `backspace` ⌫/Backspace, `space` Space, arrows ←→↑↓ on both, `f1`–`f12` as F1–F12, punctuation as itself. `KEY_NAMES` holds `a`–`z`, `0`–`9`, those names and `/ ? = - ,`.
- [ ] **Step 4: Run them to see them pass.** Run: `cargo test -p brightfield-keys keystroke` Expected: PASS.
- [ ] **Step 5: Commit.** `git add crates/brightfield-keys/src/keystroke.rs crates/brightfield-keys/src/lib.rs && git commit -m "Parse the registry's keystrokes into labels the menus can show"`

### Task 2: Reserve the verbs the shell does not handle, and list only built verbs in the help sheet

**Files:**
- Modify: `crates/brightfield-keys/src/registry.rs` (the eleven entries in the survey table marked reserved; `ReservedReason`; `VerbStatus`; `help_sheet`; tests)
- Modify: `crates/brightfield-keys/src/dispatch.rs`, `palette.rs`, `scope.rs` (tests only)
- Modify: `crates/brightfield-shell/src/overlays.rs` (tests only)

**Interfaces:**
- Produces: `ReservedReason::NeedsChartEditing` (reason "needs editing the chart from the keys") and `ReservedReason::NeedsPlotFocus` (reason "needs a focus that moves between a chart's plots"); `help_sheet` returns built verbs only. `VerbStatus::Preview` is removed, since no verb is a preview once `cycle-colour-scheme` is reserved.

- [ ] **Step 1: Write the failing tests.** In `registry.rs`, extend `reserved_buckets_present_with_reasons` with the two new buckets, exactly `["add-mark", "change-mark-type", "cycle-colour-scheme", "remove-mark"]` and `["dive-in", "focus-next-sibling", "focus-prev-sibling", "pop-out"]`, and `NeedsWorkspaceShell` gaining `reload-spec`, `toggle-focus` and `toggle-presentation`. Add:

```rust
#[test]
fn the_help_sheet_lists_built_verbs_only() {
    let reg = registry();
    let rows = help_sheet(&reg);
    assert!(rows.iter().all(|r| r.reserved_reason.is_none() && !r.keys.is_empty()));
    assert_eq!(rows.len(), reg.iter().filter(|v| !v.is_reserved()).count());
}
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-keys registry` Expected: FAIL on both.
- [ ] **Step 3: Reserve the eleven verbs.** Each keeps its longname, scope, tier and help; gets `binding_specs: Vec::new()`, `status: VerbStatus::Reserved`, `drives: D::Reserved`, `scores: None` and its bucket from the survey table. Write them as literals, as `toggle-inspector-rail` is, since the `reserved()` helper forces the Data tier. Filter `help_sheet` to `!v.is_reserved()`. Remove `VerbStatus::Preview` and `cycle_colour_scheme_is_view_only_preview`.
- [ ] **Step 4: Move the tests that used a reserved verb's key onto built ones.** `dispatch.rs`: `global_bindings_resolve_from_both_workspace_and_editor` uses `cmd-o` and `open`; `plot_focus_resolves_a_bare_verb` uses `=` and `zoom-in`, `x` and `cycle-axis-lock`; the canvas rows of `h_and_l_are_the_cell_beside_in_the_shelf_and_keep_their_other_meanings`, `the_grid_moves_its_cursor_on_the_keys_that_pan_and_dive_elsewhere` and `the_versions_panel_steps_back_and_returns_on_the_keys_that_dive_and_clear_elsewhere` now resolve to nothing on the canvas (`is_empty()`); `the_browser_answers_its_yazi_keys_and_the_canvas_keeps_its_own` asserts `a` resolves to nothing on the canvas; `toggle_chord_is_unshadowed` becomes `the_open_chord_is_unshadowed`, over `cmd-o` and `open`. `palette.rs`: `query_colour_finds_cycle_colour_scheme_with_its_key` becomes `query_colour_finds_cycle_colour_scheme_flagged_and_keyless`; `empty_query_orders_by_frequency_then_recency` breaks its tie between two built View verbs of equal frequency. `scope.rs`: `bare_verb_off_its_altitude_is_rejected_with_reason` uses a built View-only verb such as `go-to-x-cell`. `crates/brightfield-shell/src/overlays.rs`: the help sheet's count tests compare with the built count.
- [ ] **Step 5: Run.** Run: `cargo test -p brightfield-keys` then `cargo test -p brightfield-shell --lib overlays::` Expected: PASS.
- [ ] **Step 6: Commit.** `git commit -m "Reserve the chart verbs no handler answers, so the help sheet lists only keys that act"` with the touched files.

### Task 3: The availability declaration

**Files:**
- Create: `crates/brightfield-keys/src/availability.rs`
- Modify: `crates/brightfield-keys/src/registry.rs` (`label` and `needs` on `VerbEntry`, every literal and `reserved()`; the shared requirements as `const`s above `registry()`)
- Modify: `crates/brightfield-keys/src/lib.rs`

**Interfaces:**
- Consumes: Task 2's registry.
- Produces:

```rust
pub enum Need { Sources(u8), Column(ColumnKind), Selected(Selected), Grant(Grant), Condition(Condition) }
pub enum ColumnKind { Any, Quantitative, Categorical, Temporal, Boolean }   // the workbench's FieldType words
pub enum Selected { Protocol, Graph, Chart, Rows, GridCell, Asset, Step, Model, ModelTable, ModelJoin }
pub enum Grant { FullEdition }
pub enum Condition { SelectionHeld, EditToTakeBack, UnsavedWork, ProtocolSource, Trusted, TakesSources, FileToReload, ShelfDrawn }
pub struct Requirement { pub need: Need, pub missing: &'static str }      // `missing` may hold `{command}`
impl Requirement { pub const fn new(need: Need, missing: &'static str) -> Self }
#[derive(Default)] pub struct Situation { pub sources: usize, pub selected: Vec<Selected>, pub column: Option<ColumnKind>, pub grants: Vec<Grant>, pub conditions: Vec<Condition> }
impl Situation { pub fn meeting(needs: &[Requirement]) -> Self }          // a situation every listed need holds in
pub enum Availability { Available, Missing(String) }
impl Availability { pub fn is_available(&self) -> bool }
pub fn availability(verb: &VerbEntry, situation: &Situation) -> Availability
pub fn sentence(longname: &str, need: Need) -> Option<String>            // the declared sentence, `{command}` filled
// on VerbEntry:
pub label: &'static str,
pub needs: &'static [Requirement],
```

The enums of needs and `Requirement` derive `Debug, Clone, Copy, PartialEq, Eq`; `Availability` derives `Debug, Clone, PartialEq, Eq` and `Situation` `Debug, Clone, Default, PartialEq`. `Column(Any)` is met by any `Some` column; a typed need only by that kind. The shell reports `ColumnKind::Any` until a command declares a typed column need.

- [ ] **Step 1: Write the failing tests** in `availability.rs`:

```rust
#[test] fn a_command_with_no_needs_is_available_on_the_front_door() {
    assert!(availability(&verb("open"), &Situation::default()).is_available());
}
#[test] fn the_first_missing_need_says_its_sentence() {
    let s = Situation { conditions: vec![Condition::ProtocolSource], ..Situation::default() };
    assert_eq!(availability(&verb("run-protocol"), &s),
        Availability::Missing("Run needs the Protocol trusted: trust it from the band over its canvas.".into()));
}
#[test] fn a_typed_column_need_is_met_only_by_that_kind() { /* Requirement for Column(Quantitative) against Any, Categorical, Quantitative */ }
#[test] fn a_sources_need_counts_at_least() { /* Sources(2) unmet at 1, met at 2 and 3 */ }
#[test] fn meeting_meets_every_listed_need() { /* for every registry verb, availability(v, &Situation::meeting(v.needs)) is Available */ }
#[test] fn sentence_fills_in_the_label() {
    assert_eq!(sentence("organise-joins", Need::Sources(2)).as_deref(), Some("Organising joins needs a Protocol of two sources or more."));
    assert_eq!(sentence("zoom-in", Need::Selected(Selected::Chart)).as_deref(), Some("Zoom in needs a chart on the canvas."));
}
```

In `registry.rs`: `every_verb_has_a_unique_label_in_sentence_case` (non-empty, first letter upper-case, no trailing `.` or `…`, unique) and `every_declared_sentence_reads_as_a_sentence` (after `{command}` is filled: ends with `.`, holds no `{`).

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-keys` Expected: FAIL to compile, `availability` not found.
- [ ] **Step 3: Implement** `availability.rs`, then add `label` and `needs` to every `VerbEntry` literal from the Declarations tables (`needs: &[]` where none), and a `label` argument to `reserved()`. Name a shared requirement once as a `const` (`ON_CHART`, `IN_MODEL`, …) and reuse it.
- [ ] **Step 4: Run.** Run: `cargo test -p brightfield-keys` Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Declare on each command the inputs it needs and the sentence it says without them"`

### Task 4: The menu model

**Files:**
- Create: `crates/brightfield-keys/src/menus.rs`
- Modify: `crates/brightfield-keys/src/lib.rs`

**Interfaces:**
- Consumes: Task 3's `VerbEntry::label`.
- Produces: `pub struct Menu { pub title: &'static str, pub items: Vec<MenuItem> }`; `pub enum MenuItem { Command { longname: &'static str, asks: bool }, Separator, Native(NativeItem) }`; `pub enum NativeItem { Minimize, Zoom, BringAllToFront }`; `pub fn menu_bar() -> Vec<Menu>` (the Menus table); `pub fn item_title(longname: &str, asks: bool) -> String` (the label, with "…" when it asks).

- [ ] **Step 1: Write the failing tests:** `the_menu_bar_has_the_eight_menus_in_order` (titles exactly Brightfield … Help); `every_menu_command_is_a_built_verb_with_a_key`; `no_command_is_in_two_menus`; `every_menu_holds_a_command` (each has at least one `Command`, so none is empty where natives are not drawn); `no_separator_leads_trails_or_doubles`; `item_title_adds_the_ellipsis_only_when_it_asks` (`item_title("open", true) == "Open…"`, `item_title("run-protocol", false) == "Run"`).
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-keys menus` Expected: FAIL, `menu_bar` not found.
- [ ] **Step 3: Implement** `menu_bar()` as the Menus table, data only.
- [ ] **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Lay out the eight menus as data over the command list"`

### Task 5: The window's situation, read by the palette

**Files:**
- Modify: `crates/brightfield-keys/src/palette.rs` (`palette_filter` takes `situation: Option<&Situation>`; `PaletteCandidate::missing`)
- Modify: `crates/brightfield-keys/src/lib.rs` (the cross-wiring test passes `None`)
- Modify: `crates/brightfield-shell/src/overlays.rs` (`CommandPalette::with_situation`; a missing row's detail and refusal; `protocol_palette_verbs()` and `chart_palette_verbs()` lose their flags, and `chart_offers` and `SAVEABLE_CHART_VERBS` fold into `CHART_PALETTE_VERBS`)
- Modify: `crates/brightfield-shell/src/window.rs` (`situation`, `availability_of`; the three palette openers pass the situation; the refusals listed in Step 5 say the declared sentence)
- Modify: `crates/brightfield-shell/src/grid_foot.rs` (`ChartDoc::grid_cursor_on_cell`)
- Modify: `crates/brightfield-shell/src/model_pane.rs` (`reshape_action`'s two selection errors)
- Create: `crates/brightfield-shell/tests/support/scenes.rs`, `crates/brightfield-shell/tests/availability.rs`
- Modify: `crates/brightfield-shell/tests/overlay_wiring.rs`, `crates/brightfield-shell/tests/shelf_column_route.rs`, `crates/brightfield-shell/tests/organise_joins.rs`, and each test `rg -n 'open_palette_rows' crates/brightfield-shell/tests` finds asserting a row absent that is now listed greyed (`one_step_protocol.rs` among them)

**Interfaces:**
- Consumes: Task 3's `Situation`, `availability`, `sentence`.
- Produces: `MeridianApp::situation(&self) -> brightfield_keys::Situation`; `MeridianApp::availability_of(&self, longname: &str) -> Option<Availability>` (`None` for a longname the registry lacks), which slice 2's ribbon reads; `CommandPalette::with_situation(self, situation: Situation) -> Self`; `MeridianApp::open_palette_missing(&self) -> Vec<(&'static str, String)>`, the open palette's greyed rows with their sentences, beside `open_palette_rows`; `ChartDoc::grid_cursor_on_cell(&self) -> bool`. In the test support: `pub struct Window` (as `overlay_wiring.rs`'s, with `run`, `settle`, `key`) and scene constructors `door()`, `data_file()`, `crosswalk()`, `own_files()`, `live_chart(fixture)`, each with its own scratch folders set as `tests/open_makes_a_protocol.rs`'s `scratch()` sets them.

`situation()` reads, with `door = front_door_is_live()`:

| Field | Holds when |
|---|---|
| `sources` | the model pane's table count, else 1 for a Protocol with a source (`has_protocol_to_save`), else 0 |
| `Selected::Protocol` | `!door` |
| `Selected::Graph` | `!door && graph_on_canvas()` |
| `Selected::Chart` | `!door`, the canvas holds a dashboard or a chart, the model pane is not shown, and the chart document composes a plot |
| `Selected::Rows` | `protocol.doc.model.table().is_some()` |
| `Selected::GridCell` | `drawn_grid_layout() == GridLayout::Rows && charts.doc.grid_cursor_on_cell()` |
| `Selected::Asset` | `protocol.doc.model.selected()` or `canvas_holds.node()` is some |
| `Selected::Step` | `protocol.doc.model.removable_step().is_some()` |
| `Selected::Model`, `ModelTable`, `ModelJoin` | the model pane is shown; its `focus.selected` is a dataset, a relationship |
| `column` | `Some(ColumnKind::Any)` while `protocol.doc.model.outline_column()` is some |
| `Grant::FullEdition` | `edition::current() == Edition::Full` |
| `SelectionHeld`, `EditToTakeBack`, `UnsavedWork`, `ProtocolSource` | `selection_active()`, `last_shelf_edit().is_some()`, `anything_to_save()`, `has_protocol_to_save()` |
| `Trusted` | `!door && restricted.is_none() && held_untrusted().is_none()` |
| `TakesSources`, `FileToReload`, `ShelfDrawn` | `protocol_taking_sources()`, `fetching.is_none() && data_file_to_reload()`, `charts.shelf.band` is some |

- [ ] **Step 1: Write the failing tests.** In `palette.rs`: `an_unavailable_command_is_listed_disabled_with_its_sentence` (with `Situation::default()`, `zoom-in` at View is present, `!enabled`, `missing == Some("Zoom in needs a chart on the canvas.")`; with `None`, it is enabled). In `tests/availability.rs`: `the_front_door_holds_no_protocol` (no `Selected`, sources 0); `a_data_file_protocol_holds_a_chart_its_rows_and_its_source` (Protocol, Chart, Rows; ProtocolSource, Trusted, TakesSources, FileToReload, ShelfDrawn; sources 1); `the_crosswalk_puts_its_graph_on_the_canvas` (Graph, not Chart); `an_own_files_protocol_of_two_tables_has_two_sources_and_the_model`; `a_held_selection_and_a_kept_column_are_conditions`. In `overlay_wiring.rs`, replace `a_chart_start_is_offered_no_save` with `a_chart_start_greys_save_and_run_with_their_sentences`, and add `confirming_a_greyed_row_says_its_sentence_and_runs_nothing`. In `shelf_column_route.rs`, replace `the_protocol_palette_offers_no_put_row_where_choosing_one_would_put_nothing` with `the_protocol_palette_greys_the_put_rows_over_the_graph_with_their_sentence`, keeping its check that choosing one puts no column. In `organise_joins.rs`: `organising_one_source_says_the_declared_sentence`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-keys palette` and `cargo test -p brightfield-shell --test availability --test overlay_wiring --test shelf_column_route --test organise_joins --test one_step_protocol` Expected: FAIL to compile.
- [ ] **Step 3: Implement the filter and the palette.** A candidate is `enabled` when it is not reserved and its `missing` is none. Its row's detail reads "{help} — {sentence}", as a reserved row's reads "{help} — {reason}", and confirming it sets the picker's error hint to the sentence and keeps the palette open. The rows stay drawn as rows: `PickerRow` has no disabled state yet, and drawing one is the design system's. Move the tests that passed the dropped flags onto the flagless functions: `every_chart_palette_candidate_actually_dispatches` (it chooses only enabled rows, so its window stays the saveable one), and `overlays.rs`'s `the_chart_palette_offers_save_only_where_there_is_something_to_save`, which becomes `the_chart_palette_greys_save_where_there_is_nothing_to_save`.
- [ ] **Step 4: Implement `situation` and `availability_of`** from the table above, and pass `self.situation()` to the model, Protocol and chart palettes.
- [ ] **Step 5: Say the declared sentence where a handler already refuses for a declared need:** `open_organise_joins`'s toast (`sentence(ORGANISE_JOINS, Need::Sources(2))`), `apply`'s picture toast over the graph, `untrust_open_protocol`'s two toasts, and `reshape_action`'s "Select a join first" and "Select a table first" errors. Add `the_refusals_the_window_says_are_declared` in `window.rs`'s tests, asserting `sentence` is some for each of those verb and need pairs.
- [ ] **Step 6: Run.** Run: as Step 2, then `cargo test -p brightfield-shell --lib -- overlays:: model_pane:: window::` Expected: PASS.
- [ ] **Step 7: Commit.** `git commit -m "Grey a palette row with the sentence its command declares, from what the window holds"`

### Task 6: Every Global and Workspace key is answered

**Files:**
- Modify: `crates/brightfield-shell/src/window.rs` (the key record; `consume_token` gains `cmd-shift-p`, `cmd-shift-u`, `cmd-shift-r`; `OverlayKeys::palette` becomes every binding of `open-palette`; `untrust_key`, `reload_key`, `clear_selection_key`)
- Modify: `crates/brightfield-shell/tests/support/scenes.rs` (event synthesis, arrangements, holding the keys)
- Create: `crates/brightfield-shell/tests/keys_answered.rs`

**Interfaces:**
- Consumes: Task 1's `parse_keystrokes`; Task 2's statuses; Task 5's `availability_of` and scenes.
- Produces: `MeridianApp::keys_answered(&self) -> &[&'static str]`, the verbs a key performed on the last frame, cleared at the head of `draw`. In the support: `Window::press(&mut self, keystrokes: &'static str)`, which runs `events_for`'s frames and then settles; `fn events_for(keystrokes: &'static str) -> Vec<(egui::Modifiers, Vec<egui::Event>)>` (one frame per stroke, a `Key` event and, for a printable stroke with no command, control or alt, its `Text`; the frame's `RawInput::modifiers` set to the stroke's, which `cmd-shift-c`'s copy path reads); `fn arrange(win: &mut Window, need: Need) -> bool`, an exhaustive `match` over `Need`, `Selected` and `Condition`; `fn scene_for(longname: &'static str) -> Window`, the first of `door`, `data_file`, `crosswalk`, `own_files`, `live_chart("crossfilter.yaml")` in which, after `arrange` for each need, `availability_of` answers `Available`, and a panic naming the verb when none does; `fn hold_keys(win: &mut Window, context: BindingContext, longname: &'static str)`.

Arrangements: `Column` puts the Outline's cursor on a column row (`tests/outline_list.rs`); `GridCell` clicks a grid cell in the rows layout (`tests/grid_cursor.rs`); `Asset` and `Step` select an asset, one whose step can be removed (`tests/remove_step.rs`); `ModelTable` and `ModelJoin` select through the model pane's focus (`tests/model_reshape.rs`); `SelectionHeld` applies a selection as `every_chart_palette_candidate_actually_dispatches` does; `EditToTakeBack` and `UnsavedWork` keep a column from the shelf (`tests/shelf_undo.rs`); every other need is the scene's or nothing. `hold_keys` for Global does nothing and for Workspace gives the hero pane focus when a chart is on the canvas.

- [ ] **Step 1: Write the failing tests** in `keys_answered.rs`:

```rust
fn sweep(context: BindingContext) {
    for v in registry().iter().filter(|v| !v.is_reserved()) {
        for b in v.binding_specs.iter().filter(|b| b.context == context) {
            let mut win = scene_for(v.longname);
            hold_keys(&mut win, context, v.longname);
            win.press(b.keystrokes);
            assert!(win.app.keys_answered().contains(&v.longname),
                "{} is listed for {} in {context:?} and no handler took it", b.keystrokes, v.longname);
        }
    }
}
#[test] fn every_global_key_is_answered() { sweep(BindingContext::Global) }
#[test] fn every_workspace_key_is_answered() { sweep(BindingContext::Workspace) }
#[test] fn esc_with_the_grid_foot_field_open_keeps_the_selection() {
    // A held selection, then `w` opens the where field; Esc puts the field back and
    // the selection is still held.
}
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test keys_answered` Expected: FAIL, `keys_answered` not found.
- [ ] **Step 3: Record.** Push the verb in each Global and Workspace handler where it takes its key: `overlay_open_keys` (`open-palette`, `focus-jump`, `open-help`), `home_key`, `open_key`, `save_key`, `quit_key`, `navigator_key`, `run_key`, `picture_keys`, `grid_key`, `undo_key`, `navigation_keys`, `shelf_keys` (`set-channel` on `e`; the band's own keys are Task 7's), `model_keys`, `rename_key`, `browser_key`, `new_protocol_key`, `preferences_key`.
- [ ] **Step 4: Run to see the dead keys fail.** Expected: FAIL on one of `cmd-shift-p`, `cmd-shift-u`, `cmd-shift-r` and `escape`, and on each in turn as the one before is wired.
- [ ] **Step 5: Wire the four keys.** `open-palette`'s ⌘⇧P opens what Space opens, on its gate. `untrust_key` calls `untrust_open_protocol` and `reload_key` calls `reload_data_file`, each gated as `rename_key` is and read off the registry into a binding field at boot as the others are. `clear_selection_key` takes Esc, read off the registry as `shelf_entry_key` reads `e`, only with a chart on the canvas, a held selection, no overlay, no text field holding the keys and the shelf not holding them, and runs just before `tile_back_key`, whose doc already says a selection takes Esc first.
- [ ] **Step 6: Run.** Run: `cargo test -p brightfield-shell --test keys_answered --test overlay_wiring` Expected: PASS.
- [ ] **Step 7: Commit.** `git commit -m "Answer every Global and Workspace key the help lists, wiring Esc, ⌘⇧P, ⌘⇧U and ⌘⇧R"`

### Task 7: Every key in a pane's own context is answered

**Files:**
- Modify: `crates/brightfield-shell/src/protocol.rs` (`perform`; the key path records)
- Modify: `crates/brightfield-shell/src/shelf.rs` (the band's `press` and the list's `dispatch` record what they took; `window.rs`'s `shelf_keys` records the band's `undo`)
- Modify: `crates/brightfield-shell/src/sources_browser.rs` (`Browser::key` records the verbs it answers)
- Modify: `crates/brightfield-shell/src/editor.rs`, `crates/brightfield-shell/src/app.rs` (the editor's ⌘S recorded on the chart document)
- Modify: `crates/brightfield-shell/src/window.rs` (drain those records; record in `grid_cursor_keys` and `versions_keys`)
- Modify: `crates/brightfield-shell/tests/support/scenes.rs`, `crates/brightfield-shell/tests/keys_answered.rs`

**Interfaces:**
- Consumes: Task 6's record and sweep.
- Produces: `ProtocolModel::perform(&mut self, verb: &str, canvas_node: Option<&AssetId>) -> Option<bool>` (`None` for a verb it has no arm for; `dispatch` becomes `perform(..).unwrap_or(false)`); `ProtocolModel::take_keys_answered(&mut self) -> Vec<&'static str>`, filled where `feed_key` and `feed_column_chord` dispatch a verb `perform` knows; the same `take_keys_answered` on `ShelfBand`, the column list, `Browser` and `ChartDoc`.

`hold_keys` grows to every `BindingContext`, an exhaustive `match`: Protocol, the graph on the canvas or else the Outline focused; Shelf, the hero focused, `e`, then `x` for the list's verbs and Tab first for `set-shelf-setting-to-auto`; Grid, a grid cell clicked; Versions, the panel holding the keys with its cursor on a row (`tests/versions_panel.rs`); Browser, the sources browser open over `data_file`; Editor, the editor's text holding the keys (`tests/editor_item.rs`).

- [ ] **Step 1: Write the failing tests:** `every_protocol_key_is_answered`, `every_shelf_key_is_answered`, `every_grid_key_is_answered`, `every_versions_key_is_answered`, `every_browser_key_is_answered`, `every_editor_key_is_answered`, each `sweep(context)`; `every_context_a_binding_uses_is_swept`, asserting each context any built binding uses is in a `SWEPT` list beside the sweeps; and in `protocol.rs`, `perform_answers_none_for_a_verb_it_has_no_arm_for`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test keys_answered` Expected: FAIL to compile.
- [ ] **Step 3: Implement the records** and drain each into `keys_answered` on the frame its key was pressed.
- [ ] **Step 4: Run.** Run: `cargo test -p brightfield-shell --test keys_answered`, then `cargo test -p brightfield-shell --lib -- protocol:: shelf:: sources_browser::` Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Answer every key the panes' own contexts list, and prove each context is swept"`

### Task 8: Menu commands reach the window

**Files:**
- Modify: `crates/brightfield-shell/src/open_queue.rs`
- Create: `crates/brightfield-shell/src/menu_bar.rs`
- Modify: `crates/brightfield-shell/src/lib.rs`
- Modify: `crates/brightfield-shell/src/window.rs` (the menu state, published at the end of `draw`; menu verbs drained into the frame's requests; the arms `apply` lacks; the verb record)
- Create: `crates/brightfield-shell/tests/menu_commands.rs`

**Interfaces:**
- Consumes: Task 4's `menu_bar`, `item_title`; Task 5's `situation`, `availability_of`; Task 6's scenes; Task 7's `ProtocolModel::perform`.
- Produces:

```rust
// open_queue.rs
pub fn ask_verb(&self, longname: &'static str)        // wakes the window, as ask_quit does
pub fn take_verbs(&self) -> Vec<&'static str>
// menu_bar.rs
pub const SHEET_UP: &str = "Finish what the sheet over the window asks first.";
pub struct MenuRow { pub menu: &'static str, pub longname: &'static str, pub title: String,
                     pub keys: Option<&'static str>, pub missing: Option<String> }
pub fn menu_rows(situation: &Situation, sheet_up: bool) -> Vec<MenuRow>   // every Command item, in menu_bar() order
#[derive(Clone, Default)] pub struct MenuState(/* Arc<Mutex<Vec<MenuRow>>> */);
impl MenuState { pub fn publish(&self, rows: Vec<MenuRow>); pub fn rows(&self) -> Vec<MenuRow> }
// window.rs
pub fn menu_rows(&self) -> Vec<MenuRow>
pub fn menu_state(&self) -> MenuState
pub fn verbs_applied(&self) -> &[&'static str]       // the verbs `apply` performed on the last frame
```

A row's index in `menu_rows` is its native tag (Task 9). A sheet is up while an overlay other than the palette, the help sheet or the node jump is open, or the sources browser is: every row's `missing` is then `SHEET_UP`, and a queued verb is dropped. A queued verb under a picker overlay closes the picker first. A queued verb that `availability_of` finds missing is not applied, and its sentence is said in an Info toast.

- [ ] **Step 1: Write the failing tests** in `menu_commands.rs`:

```rust
#[test] fn every_menu_command_acts_where_it_is_available() {
    for row in menu_rows(&Situation::default(), false) {
        let mut win = scene_for(row.longname);
        win.app.open_queue().ask_verb(row.longname);
        win.settle();
        assert!(win.app.verbs_applied().contains(&row.longname), "{} is on the {} menu and nothing performed it", row.longname, row.menu);
    }
}
#[test] fn a_command_chosen_where_it_is_unavailable_says_its_sentence_and_does_nothing() { /* undo with no kept column: the toast reads Undo's sentence, verbs_applied is empty */ }
#[test] fn on_the_front_door_every_command_that_needs_a_protocol_is_greyed() { /* door: each row whose verb declares Selected::Protocol, or a condition the door cannot hold, has its sentence as `missing` */ }
#[test] fn no_menu_command_acts_behind_the_close_question() { /* draft, close question up: every row reads SHEET_UP; ask_verb("run-protocol") applies nothing */ }
#[test] fn the_menu_rows_carry_each_commands_title_key_and_sentence() { /* "Open…" with keys "cmd-o"; Run's missing on a chart start */ }
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test menu_commands` Expected: FAIL to compile.
- [ ] **Step 3: Implement the queue, `menu_bar.rs` and the drain.**
- [ ] **Step 4: Add the arms `apply` lacks**, recording each verb an arm takes: `open-palette` (factor what `overlay_open_keys` opens for the canvas into one method both call), `open-help`, `focus-jump` (the jump, over the graph), `filter-rows-at-cursor` and `sort-rows-at-cursor` (factor the grid's two asks out of `grid_cursor_keys`, with the operation rail opened as they open it), `set-channel` (focus the hero pane and give the band the keys), and `remove-step` and `yank-address` sent to the Protocol model whatever the canvas holds, as a column put on a channel is. The graph arm records when `perform` answers `Some`; the chart arm records `clear-selection`, `save-spec` and a navigation verb.
- [ ] **Step 5: Run.** Run: `cargo test -p brightfield-shell --test menu_commands --test overlay_wiring` Expected: PASS.
- [ ] **Step 6: Commit.** `git commit -m "Carry a menu command to its handler through the open queue, refusing one the window cannot take"`

### Task 9: The native macOS menu bar

**Files:**
- Create: `crates/brightfield-shell/src/mac_menu.rs`
- Modify: `crates/brightfield-shell/src/lib.rs` (`#[cfg(target_os = "macos")] #[allow(unsafe_code)] pub mod mac_menu;` and the comment on the modules that write `unsafe`)
- Modify: `crates/brightfield-shell/Cargo.toml` (the `[lints.rust]` comment naming those modules)
- Modify: `crates/brightfield-shell/src/main.rs` (install the menus on the frame `route_menu_quit` routes)

**Interfaces:**
- Consumes: Task 1's `Keystroke`; Task 4's `menu_bar`; Task 8's `MenuState`, `menu_rows` order, `OpenQueue::ask_verb`.
- Produces: `pub fn install(main: MainThreadMarker, menus: &[Menu], queue: OpenQueue, state: MenuState) -> bool`; private `fn key_equivalent(stroke: &Keystroke) -> Option<(String, usize)>` and `fn native_title(row: &MenuRow) -> String`.

How it is built: every menu but the first (winit's app menu, left to `route_menu_quit`) becomes an `NSMenu` appended to the main menu, with `setAutoenablesItems: false`. A command is an `NSMenuItem` with its `title`, tag (its `menu_rows` index), action `menuItemChosen:` and a `MenuDelegate` target; a single keystroke sets `setKeyEquivalent:` and `setKeyEquivalentModifierMask:`, and a chord sets neither and is written after the title, "Remove step  D D". Natives are AppKit's `performMiniaturize:`, `performZoom:` and `arrangeInFront:` with no key equivalent and tag -1. The Window menu is passed to `setWindowsMenu:` and Help to `setHelpMenu:`. `MenuDelegate` (a `define_class!` subclass of `NSObject`, as `OpenHandler` is) is each menu's delegate: `menuNeedsUpdate:` sets each tagged item's `setEnabled:` and `setToolTip:` from `state.rows()`; `menuHasKeyEquivalent:forEvent:target:action:` answers `NO`, so AppKit claims no key and each key reaches the window; `menuItemChosen:` reads the sender's tag and calls `queue.ask_verb`. Key equivalents: a letter lower-case with the shift mask for ⇧; `enter` `\r`, `escape` `\u{1b}`, `tab` `\t`, `space` `" "`, `backspace` `\u{8}`, arrows `\u{F700}`–`\u{F703}`, `f1`–`f12` from `\u{F704}`; masks Shift `1 << 17`, Control `1 << 18`, Option `1 << 19`, Command `1 << 20`.

- [ ] **Step 1: Write the failing tests** in `mac_menu.rs`: `single_keystrokes_become_key_equivalents_with_their_modifier_mask` (`cmd-shift-p` → `("p", 1<<17 | 1<<20)`, `cmd-enter` → `("\r", 1<<20)`, `f2` → `("\u{F705}", 0)`, `w` → `("w", 0)`); `a_chord_is_written_after_the_title_and_claims_no_equivalent`; `the_menus_claim_no_key_so_every_key_reaches_the_window` (send `menuHasKeyEquivalent:forEvent:target:action:` to a delegate made on the test thread, with null arguments, and get `false`); `choosing_an_item_queues_its_verb` (the delegate's tag handler, given Run's tag, leaves `["run-protocol"]` on the queue). None of these makes an AppKit object, since a test thread is not the main thread.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --lib mac_menu::` Expected: FAIL to compile.
- [ ] **Step 3: Implement `mac_menu.rs`,** sending AppKit messages by selector as `route_menu_quit` does, with a `SAFETY` comment on each block.
- [ ] **Step 4: Install from `main.rs`:** once `route_menu_quit` answers `true`, call `install` with `brightfield_keys::menu_bar()`, the queue and `self.app.menu_state()`; on `false`, write one line to stderr saying the menus were not built.
- [ ] **Step 5: Run.** Run: `cargo test -p brightfield-shell --lib -- mac_menu:: mac_open::` Expected: PASS.
- [ ] **Step 6: Check on a Mac** with `cargo run -p brightfield-shell --release`: the menu bar reads Brightfield, File, Edit, View, Protocol, Data, Window, Help; File shows ⌘N, ⌘O, ⇧⌘H, ⌘S; on the front door Save, Run and Undo are greyed and their tooltips say their sentences; choosing Run on a data file runs it; ⌘S with the YAML editor holding the keys still saves the editor's buffer; `w` typed into the grid's where field types a `w`; ⌘Q over a draft still asks the close question; ⌘, still opens Preferences.
- [ ] **Step 7: Commit.** `git commit -m "Build the seven native menus from the command list, greyed with their sentences and claiming no keys"`

### Task 10: The menus from the mark on hosts with no menu bar

**Files:**
- Modify: `crates/brightfield-shell/src/menu_bar.rs` (the in-window menus)
- Modify: `crates/brightfield-shell/src/window.rs` (`MenuHost`; the title band's mark)
- Create: `crates/brightfield-shell/tests/mark_menu.rs`

**Interfaces:**
- Consumes: Task 1's `keys_label`; Task 8's `menu_rows` and the frame's requests.
- Produces: `pub enum MenuHost { Native, Mark }`; `MeridianApp::menu_host(&self) -> MenuHost` and `set_menu_host(&mut self, host: MenuHost)`, defaulting to `Native` on macOS and `Mark` elsewhere; `MeridianApp::mark_rect(&self) -> Option<egui::Rect>`; `MeridianApp::mark_menu_drawn(&self) -> &[DrawnRow]` with `pub struct DrawnRow { pub menu: &'static str, pub longname: Option<&'static str>, pub rect: egui::Rect, pub enabled: bool, pub hover: Option<String>, pub keys: Option<String> }` (`longname` none for a menu's own title row); `pub fn show_menus(ui: &mut egui::Ui, menus: &[Menu], rows: &[MenuRow], platform: KeyPlatform, drawn: &mut Vec<DrawnRow>) -> Option<&'static str>`.

With `MenuHost::Mark`, the mark is drawn on the front door too, and a click on it opens `egui::Popup::menu` holding the eight menus as `ui.menu_button`s; a command is `Button::new(title).shortcut_text(keys_label(keys, KeyPlatform::Other))`, added with `ui.add_enabled(missing.is_none(), …)` and `.on_disabled_hover_text(missing)`; natives are skipped; a choice becomes `Request::Verb` in the frame's requests, so it goes through the same refusal as a queued verb. Home stays the File menu's third item and ⇧⌘H. With `MenuHost::Native` the mark goes home as it does today.

- [ ] **Step 1: Write the failing tests:** `the_mark_opens_the_eight_menus_on_a_host_with_no_menu_bar`; `the_mark_is_drawn_on_the_front_door_where_it_opens_the_menus`; `a_greyed_row_says_its_sentence_and_choosing_it_does_nothing` (on a chart start, Protocol's Run row is disabled, its hover is Run's sentence, and a click on it applies nothing); `choosing_a_row_applies_its_command` (on `data_file`, View's Zoom in is applied); `keys_read_ctrl_on_a_host_with_no_menu_bar` (File's Open row reads "Ctrl+O"); `on_the_mac_host_the_mark_still_goes_home`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test mark_menu` Expected: FAIL to compile.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run.** Run: `cargo test -p brightfield-shell --test mark_menu --test front_door` Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Open the same menus from the mark where the host has no menu bar"`

### Task 11: Fold slice 1 into the README

**Files:**
- Modify: `README.md` (under Architecture, a section "Commands, menus and keys")

- [ ] **Step 1: Write the section:** each command declares its needs and sentence in the key registry, and the palette and the menus read the declaration; the menus and their order; the native menus claim no keys, so each key stays the registry's; the mark opens the menus where the host has no menu bar; every key the help sheet lists is answered, held by `tests/keys_answered.rs`, and every menu item acts where it is available, held by `tests/menu_commands.rs`; the verbs reserved for want of a handler. Include the owner's calls from the spec's brainstorm that this slice carries out (unavailable options are visible and say what they need; the keyboard is learnable from visual indicators), each with its reason, and the calls under "Calls this plan makes" below, each with its reason. The README's existing ⌘⇧U sentence is now true and stays.
- [ ] **Step 2: Check** the section against the code: each key it names is that verb's primary key in `brightfield_keys::registry()`, and each test it names exists (`rg -n 'fn every_global_key_is_answered|fn every_menu_command_acts_where_it_is_available' crates/brightfield-shell/tests`).
- [ ] **Step 3: Commit.** `git commit -m "Write the command list's availability, menus and keys into the README"`. Delete this plan when the branch is called ready; the spec stays while the later slices' plans read it.

---

## Dependencies and parallel sessions

| Task | Needs | Can run beside |
|---|---|---|
| 1 Keystrokes | — | 2, 3, 4, 5 |
| 2 Reserve unbuilt verbs | — | 1 |
| 3 Declarations | 2 (both rewrite every registry entry) | 1 |
| 4 Menu model | 3 | 5, 1 |
| 5 Situation and palette | 3 | 4, 1 |
| 6 Global and Workspace keys | 1, 2, 5 | — |
| 7 Pane-context keys | 6 | — |
| 8 Menu commands | 4, 7 (`ProtocolModel::perform`, the scenes) | — |
| 9 Native menu bar | 8 | 10 |
| 10 Mark menu | 8 | 9 |
| 11 README | 9, 10 | — |

Tasks 6, 7 and 8 each edit `window.rs`'s key handlers or `apply` and the shared test support, so they run in order in one session. Two parallel sessions are safe at three points: {1, 2}, {4, 5} and {9, 10}; `lib.rs` module lines are the only overlap, a trivial merge. Cost: the `brightfield-keys` commands build that crate alone; every `brightfield-shell` command builds the shell crate and is each shell task's slowest step, so run one at a time.

## Not in this slice

- Pressing an unavailable command's key to hear its sentence: the spec places it with the ribbon's greyed command, slice 2, which reads `availability_of`.
- A greyed palette row drawn grey, and the reason popover: both need components the design system adds.
- Narrowing the Outline's column to a typed `ColumnKind`, and a sentence naming the focused node: each lands with the first command that declares one.
