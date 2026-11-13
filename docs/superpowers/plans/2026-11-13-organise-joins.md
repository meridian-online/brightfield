# `o` organises a Protocol's joins: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. This is a small change: build it inline with superpowers:executing-plans, roborev reviewing each commit.

**Goal:** `o` in the Protocol context opens a sheet over the model's joins, where `Space` ticks or unticks a join and `Enter` turns the changes into the model pane's Keep and Drop.

**Architecture:** A new registry verb, `organise-joins`, raised by the Protocol panel as `open-sources-browser` is. A small pure state module holds the sheet's rows and answers the Browser context's verbs. The sources browser's canvas drawing moves into a shared function both sheets call. The window opens the sheet as an overlay and applies the difference through `perform_model_action`.

**Tech Stack:** Rust, egui, the `brightfield-keys` registry, `brightfield-ossie` model and edits.

**Spec:** `docs/superpowers/specs/2026-11-13-organise-joins-design.md`

## Global Constraints

- Before each commit: `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, and the tests the task names. After each commit: `cargo xtask ci --step comment-citations`. Never `cargo fmt --all`.
- One cargo build at a time on this Mac; do not run implementers in parallel.
- The repo is public: no planning identifiers, task ids or private repo names in code, comments, commits or PR text.
- No new `Edit` variant; the sheet only produces `ModelAction::Keep(name)` and `ModelAction::Drop(name)`.
- Opening and closing the sheet with no toggles changes nothing in the model.

## Review Focus

1. A join on two or more columns: it must appear on the sheet with every column shown, not be filtered out as the browser's single-column `ProposedJoin` would. Pinned in Task 3.
2. Ticking a join, then unticking it, then `Enter`: nothing applied. Pinned in Task 3.
3. Dropping the only join that folds one table into another: the model refuses it, the other changes still land, and the refusal is reported. Pinned in Task 4 by `a_refused_drop_leaves_the_other_changes`: fold a table into another through `perform_model_action(ModelAction::FoldInto(..))` first (the fold's own window test in `tests/model_reshape.rs` shows how), then on the sheet untick that fold's only join and tick the proposal; after `Enter` the proposal is the analyst's and the fold's join is still in `relationships`.
4. `o` while a text field holds the keys, or while another overlay is open: no sheet. Pinned in Task 4.
5. Two joins between the same pair of tables: two rows, two curves, toggled independently. Pinned in Task 3.

---

### Task 1: The `organise-joins` verb on `o`

**Files:**
- Modify: `crates/brightfield-keys/src/registry.rs` (new `VerbEntry` after `open-sources-browser`, around line 853; the longname snapshot list in `longname_snapshot_is_stable`; the Protocol-scoped list in `protocol_altitude_verbs_are_scoped_to_protocol_only`, around line 1958)
- Modify: `crates/brightfield-keys/tests/data/keymap-research.md` (the Protocol-context table holding `open-sources-browser`, around line 264)
- Modify: `crates/brightfield-shell/src/protocol.rs` (const beside `OPEN_SOURCES_BROWSER` at 1279; a field `organise_ask: bool` beside `browser_ask` at 1467 and its init at 1613; the dispatch arm beside `OPEN_SOURCES_BROWSER =>` at 3130; `take_organise_ask` beside `take_browser_ask` at 2635; the key-table test at 6191)

**Interfaces:**
- Produces: `pub const ORGANISE_JOINS: &str = "organise-joins";` in `crate::protocol`; `pub fn take_organise_ask(&mut self) -> bool` on the Protocol panel's model, the same type `take_browser_ask` is on.

- [ ] **Step 1: Write the failing tests.** In the key-table test at `protocol.rs:6191`, add `assert_eq!(t.get("o").copied(), Some(ORGANISE_JOINS));`. Beside the existing unit test of `browser_ask` (search `take_browser_ask` in the test module), add `organise_joins_raises_the_ask_once`: dispatching `ORGANISE_JOINS` returns `true`, `take_organise_ask()` is then `true`, and a second call is `false`.
- [ ] **Step 2: Run them to see them fail.** `cargo test -p brightfield-shell --lib protocol::` — expected: compile error on `ORGANISE_JOINS`.
- [ ] **Step 3: Add the registry entry.** `longname: "organise-joins"`, `tier: CommandTier::View`, `binding_specs: vec![proto("o")]`, `scope_applicability: vec![Protocol]`, `drives: D::PaletteMeta`, `status: VerbStatus::Built`, `help: "Organise the Protocol's joins: tick the ones to keep and untick the ones to drop"`, `scores: Some(Scores { frequency: 2, mnemonic: 5, convention: 3, motor_note: "o = organise; the grid's o sorts in its own context, so the Protocol's o never meets it" })`. Add `"organise-joins"` after `"open-sources-browser"` in both test lists. Add the research row in the same table and form: `` | `organise-joins` | `o` | 2 / 5 / 3 | `o` = organise the joins with dovetail; the grid's `o` sorts rows in the Grid context, so the Protocol context keeps them apart | ``.
- [ ] **Step 4: Add the const, field, arm and `take_organise_ask`** in `protocol.rs`, mirroring `browser_ask` exactly.
- [ ] **Step 5: Run the tests.** `cargo test -p brightfield-keys` (expected: pass, including `provenance` and the snapshot) and `cargo test -p brightfield-shell --lib protocol::` (expected: pass).
- [ ] **Step 6: Commit** `registry.rs`, `keymap-research.md`, `protocol.rs`: "Give the Protocol context's o the organise-joins verb".

### Task 2: Share the canvas drawing

**Files:**
- Modify: `crates/brightfield-shell/src/sources_browser_view.rs` (the body of `canvas` from "The tables, in the order the joins first name them" at about line 891 to the end of the row loop)
- Test: `crates/brightfield-shell/tests/sources_browser_view.rs` (existing, unchanged)

**Interfaces:**
- Produces, in `sources_browser_view.rs`:
  - `pub struct CanvasJoin { pub left_table: String, pub right_table: String, pub label: String, pub accepted: bool }`
  - `pub struct CanvasDrawn { pub rows: Vec<egui::Rect>, pub labels: Vec<String>, pub clicked: Option<usize> }`
  - `pub fn draw_join_canvas(ui: &mut egui::Ui, joins: &[CanvasJoin], cursor: usize, focused: bool, accepted_word: &str) -> CanvasDrawn` — `accepted_word` is the row's word for a ticked join: `"accepted"` on the browser, `"kept"` on the organise sheet; an unticked row reads `"proposed"` on both. Each spoken label is `"{label} {word}"`, as the browser speaks today.

- [ ] **Step 1: Move the drawing** into `draw_join_canvas` with no change to what it paints. `canvas` builds its `CanvasJoin`s from `canvas_joins()` with `join_label`, calls it with `"accepted"`, pushes the rects into `sheet.drawn.joins`, sets the preview's labels from `labels`, and toggles `clicked` as before.
- [ ] **Step 2: Run the browser's tests.** `cargo test -p brightfield-shell --test sources_browser_view` and `cargo test -p brightfield-shell --lib sources_browser` — expected: pass unchanged. A failing test here means the move changed behaviour; fix the move, not the test.
- [ ] **Step 3: Commit** "Draw a join canvas from a list of joins, for any sheet".

### Task 3: The organise sheet's state

**Files:**
- Create: `crates/brightfield-shell/src/organise_joins.rs` (state and keys, no drawing, as `sources_browser.rs` is to its view)
- Modify: `crates/brightfield-shell/src/lib.rs` (`pub mod organise_joins;`)

**Interfaces:**
- Consumes: `brightfield_ossie::{Model, DecidedBy}`; the Browser-context verb consts in `crate::sources_browser` (`MOVE_DOWN`, `MOVE_UP`, `MARK_OR_ACCEPT`, `GO_IN_OR_ADD`, `ADD`, `CLOSE`); the registry's Browser-context keystroke resolution that `Browser::press` uses.
- Produces:
  - `pub struct Row { pub relationship: String, pub left_table: String, pub right_table: String, pub label: String, pub was_kept: bool, pub kept: bool }`
  - `pub enum Effect { None, Apply(Vec<crate::model_inspector::ModelAction>), Close }`
  - `pub struct Organise { pub rows: Vec<Row>, pub cursor: usize }`
  - `impl Organise { pub fn over(model: &Model) -> Option<Self>; pub fn key(&mut self, verb: &str) -> Effect; pub fn press(&mut self, keystroke: &str) -> Effect; pub fn toggle(&mut self, at: usize); pub fn canvas_joins(&self) -> Vec<crate::sources_browser_view::CanvasJoin>; }`
  - `over` returns `None` when the model has fewer than two datasets. Rows are `model.relationships` in model order; `was_kept = kept = (meta.decided_by == DecidedBy::Analyst)`; `label` is `"{from}.{from_columns joined by ", "} → {to}.{to_columns joined by ", "}"`.
  - `key`: `MOVE_DOWN`/`MOVE_UP` move the cursor within the rows; `MARK_OR_ACCEPT` toggles the row under the cursor; `GO_IN_OR_ADD` and `ADD` return `Apply` with, in row order, `ModelAction::Keep(name)` for each row with `kept && !was_kept` and `ModelAction::Drop(name)` for each with `!kept && was_kept`; `CLOSE` returns `Close`; anything else `None`.

- [ ] **Step 1: Write the failing unit tests** in the module's test block, over a `Model` built in the test with three datasets and three relationships: `a_to_b` decided by the analyst, `c_to_b` by dovetail on two columns, and a second dovetail join `a_to_b_2` between the same two tables:
  - `fewer_than_two_tables_has_nothing_to_organise`: a one-dataset model gives `over(..).is_none()`.
  - `rows_open_ticked_as_decided`: `kept` is `[true, false, false]`; the `c_to_b` row's label shows both columns on each side.
  - `two_joins_between_the_same_tables_are_two_rows`: `canvas_joins()` has three entries, two of them between `a` and `b`, and toggling `a_to_b_2` leaves `a_to_b`'s `kept` alone.
  - `space_and_enter_keep_a_proposal`: cursor to `c_to_b`, `MARK_OR_ACCEPT`, `GO_IN_OR_ADD` gives `Apply(vec![Keep("c_to_b")])`.
  - `unticking_a_kept_join_drops_it`: toggle `a_to_b`, `ADD` gives `Apply(vec![Drop("a_to_b")])`.
  - `toggling_twice_applies_nothing`: toggle `c_to_b` twice, `GO_IN_OR_ADD` gives `Apply(vec![])`.
  - `close_is_close_and_other_verbs_do_nothing`: `CLOSE` gives `Close`; `GO_UP` gives `None` and changes no row.
  - `press_resolves_in_the_browser_context`: `press("space")` toggles, `press("enter")` applies, `press("escape")` closes.
- [ ] **Step 2: Run them to see them fail.** `cargo test -p brightfield-shell --lib organise_joins` — expected: compile error, module empty.
- [ ] **Step 3: Implement** the module to the interface above. `press` resolves the keystroke with the same registry lookup `Browser::press` uses (`registry.rs:2356`), including no chord handling: none of this sheet's verbs is a chord.
- [ ] **Step 4: Run the tests.** Same command — expected: 8 passed.
- [ ] **Step 5: Commit** "Hold the organise sheet's joins and answer its keys".

### Task 4: The window opens, draws and applies the sheet

**Files:**
- Create: `crates/brightfield-shell/src/organise_joins_view.rs` (`pub fn show(ui: &mut egui::Ui, sheet: &mut Organise, mode: Mode) -> Effect`: a heading "Organise joins", the shared canvas with `"kept"`, a foot line "Space ticks or unticks · Enter applies · Esc closes", and the keys read through `Organise::press` from the frame's key events, as `sources_browser_view::show` reads the browser's)
- Modify: `crates/brightfield-shell/src/lib.rs` (`pub mod organise_joins_view;`)
- Modify: `crates/brightfield-shell/src/window.rs`: an `Overlay::OrganiseJoins(Box<crate::organise_joins::Organise>)` variant beside `SourcesBrowser` (2272); its arm in the overlay drawing beside the `SourcesBrowser` arm (9190), on the same scrim and frame, closing on `Apply`, `Close` or a backdrop click; `"organise-joins"` in `open_overlay` (10485); `if self.protocol.doc.model.take_organise_ask() { self.open_organise_joins(ctx); }` beside `take_browser_ask` (11144); a `Request::Verb` arm for `ORGANISE_JOINS` beside the `OPEN_SOURCES_BROWSER` one (10997); after the overlay frame, for an `Apply(actions)`, `self.perform_model_action(ctx, action)` for each action in order.
- Create: `crates/brightfield-shell/tests/organise_joins.rs`
- Modify: `crates/brightfield-shell/tests/shelf_column_route.rs` (an `"organise-joins"` arm in the sweep, beside `"open-sources-browser"` at 1101)

**Interfaces:**
- Consumes: `Organise`, `Effect`, `Row` from Task 3; `draw_join_canvas` and `CanvasJoin` from Task 2; `ORGANISE_JOINS` and `take_organise_ask` from Task 1.
- Produces: `pub fn open_organise_joins(&mut self, ctx: &egui::Context)` on the window, and `pub fn organise_joins_rows(&self) -> Option<Vec<(String, bool)>>` (label, kept) for tests. `open_organise_joins` does nothing while the front door is live, another overlay is open, or `ctx.egui_wants_keyboard_input()`; with no model, or `Organise::over` giving `None`, it pushes an Info toast saying organising joins needs a Protocol of two sources or more, and opens nothing.

- [ ] **Step 1: Write the failing window tests** in `tests/organise_joins.rs`, opening the retail extract fixture with its written model saved, as `tests/spine_beside_the_model.rs` does (copy its `Window::open` and `frame`; the fixture has kept joins and the proposal `customer_segments_customers`), and pressing keys as key events with the graph or spine holding the keys:
  - `o_opens_the_sheet_with_the_models_joins`: after `o`, `open_overlay() == Some("organise-joins")`; `organise_joins_rows()` lists every relationship of the model, the proposal unticked and the analyst's joins ticked.
  - `space_and_enter_keep_a_proposal_as_the_analysts`: `j` to the proposal's row, `Space`, `Enter`; the overlay is closed and the model's `customer_segments_customers` has `decided_by == DecidedBy::Analyst`.
  - `space_and_enter_drop_a_kept_join`: `Space` on a ticked row, `Enter`; that relationship is gone from `relationships` and present in `meta.rejected`.
  - `escape_changes_nothing`: `Space` on two rows, `Esc`; the model equals the model before `o`.
  - `o_over_one_table_opens_nothing_and_says_why`: over a one-CSV Protocol (the housing sample, as `shelf_column_route.rs`'s `own_protocol` makes it), `o` leaves `open_overlay() == None` and a toast says two sources are needed.
  - `o_while_a_text_field_has_the_keys_opens_nothing`: with the palette's field focused, `o` types into it and opens no sheet.
- [ ] **Step 2: Run them to see them fail.** `cargo test -p brightfield-shell --test organise_joins` — expected: compile error on `organise_joins_rows`.
- [ ] **Step 3: Implement** the view and the window wiring as listed under Files.
- [ ] **Step 4: Add the sweep arm.** `"organise-joins"`: a window over the retail extract (reuse the fixture opening from Step 1 through a helper in `shelf_column_route.rs`, or over `own_protocol` with a second CSV added), `confirm_leaving(verb) == Some("organise-joins")`, failing with "{verb} opened no organise sheet". If the sweep's own one-source fixture lists the verb, the arm still needs a two-table window: a verb the palette offers enabled must do something when chosen.
- [ ] **Step 5: Run the tests.** `cargo test -p brightfield-shell --test organise_joins` (expected: 7 passed, with `a_refused_drop_leaves_the_other_changes` from the Review Focus), `cargo test -p brightfield-shell --test shelf_column_route every_row_the_protocol_palette_offers_as_enabled_does_something_when_chosen` (expected: pass), `cargo test -p brightfield-shell --test sources_browser_view` (expected: pass).
- [ ] **Step 6: Commit** "Organise a Protocol's joins on o".

### Before the branch is ready

- [ ] Fold this spec into the repo's architecture docs (where the sources browser and model pane are described), with both rulings and their reasons, then delete this plan and its spec on the branch.
- [ ] Open the pull request; its description carries the two rulings and CI's test totals.

## Verification cost

- `cargo test -p brightfield-keys`: seconds once built; a cold build of the keys crate, a minute or two.
- `cargo test -p brightfield-shell --lib <filter>`: needs the shell crate built; minutes cold, seconds warm.
- `cargo test -p brightfield-shell --test <file>`: also builds the `brightfield-shell` binary the tests run as their runner; several minutes cold on this Mac, under a minute warm per file.
- The full gate, `cargo xtask ci`, is CI's; run locally only to reproduce a CI failure.
