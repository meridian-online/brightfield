# The operation forms Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Phase 1 is built** (Tasks 1 to 7). What was built, the decisions made on the owner's behalf while building it, and how to add an operation are in `docs/operation-forms.md`, which is the reference for Tasks 8 to 20 where it differs from the interfaces written below. The differences that matter to an operation task: a draft's begin facts are kept on the operation being made (`Making::with`, a `BegunWith`), so `suggestions` and a form are handed a `Begin` on any frame; a header's gesture reaches the window as a `HeaderHit` from `operation_ui/header.rs`; a test offers its own operations from a header with `ChartDoc::set_header_operations`; a preview states its missing values in a line above its rows; and `filter-rows` and `sort-rows` need no Protocol.

**Goal:** Each of arcform's 13 operations reachable in Brightfield's window through the interaction that suits it. For some that is a form; for others it is a direct action on what is on screen. Each previews the result at each keystroke, each records its step through arc on ⌘S, and each shows the step's SQL in the Editor tab.

**Architecture:** Every operation has a slot in Brightfield's operation registry (`crates/brightfield-shell/src/operation_ui/`) and in the key registry (`crates/brightfield-keys/src/operations.rs`). Its own task fills that slot in its own files, so the per-operation tasks run side by side. The shared part is only what every operation needs:
- the places an operation can start from: a column header, the grid's cursor, the palette, the spine, and the inspector's Operation section, where an operation that needs a form draws its own;
- one preview path: arc records the draft into a throwaway Protocol of stub tables, and arc's model runs on a warm preview connection that holds the table whole;
- one way of keeping a draft as an unsaved step, and recording it on ⌘S;
- one availability check.

**Tech Stack:** Rust; egui/eframe 0.35; DuckDB 2.0 through the bundled `duckdb` 1.20000.0; arc (arcform, rev-pinned) run as this binary's child; dovetail's Ossie model (`brightfield-ossie`); FineType's vendored transforms (`brightfield_protocol::silver::FinetypeTransforms`); `meridian-design` and `meridian-egui`.

**Spec:** `docs/superpowers/specs/2026-11-21-brightfield-chrome-design.md`: read "The operation forms", "Sequencing", the availability bullets of "3. The ribbon's command row", and the Goals and Anti-goals. The owner's rulings of 2026-11-22 on the forms, which this plan follows where they differ from the spec's wording, are under Global Constraints.

## How the work fans out

| Phase | Tasks | Run side by side | Waits on |
|---|---|---|---|
| 1. The shared parts, one branch and one session | 1 and 2, then 3, 4, 5, 6, 7 in that order | 1 with 2 | nothing |
| 2. The fan-out, a branch and a session each | 8 (the spine picks a table); 9 to 16 (Shape: `hide-column`, `rename-column`, `derive-column`, `cast-column`, `fill-nulls`, `dedupe-rows`, `limit-rows`, `summarise-rows`); 17 (`read-file`, through the sources browser) | all ten | phase 1 merged |
| 3. Model | 18 (`join-tables`), 19 (`append-rows`) | both, and any phase 2 task still open | 8 merged |
| 4. Closing | 20 (availability through the registry; the fold) | — | every other task, and the chrome's slice 1 |

**What keeps phases 2 and 3 apart.** Phase 1 creates every slot up front:
- a stub file per operation under `operation_ui/`;
- the table that lists all 13;
- a key-registry entry per operation behind its own `*_BUILT` constant, with its key already chosen;
- a place for an operation to start from each part of a column header;
- a place in the Operation section for an operation's own form.

An operation task then touches exactly three files:
- its own `crates/brightfield-shell/src/operation_ui/<op>.rs`, which holds its defaults, its in-place field and its form;
- its own `*_BUILT` line in `crates/brightfield-keys/src/operations.rs`, each in a block of its own, so two flips never touch neighbouring lines;
- its own new test file.

Task 17 needs no `*_BUILT` line, since its command is the sources browser's. Task 8 owns the spine's picking mode, which Tasks 18 and 19 use.

## Calls this plan makes on the owner's behalf

Each is reversible by a sentence, and each goes into `docs/operation-forms.md` when Task 6 writes it.

1. **Each operation gets the interaction that suits it**, from the owner's ruling. The foot row stays for `where` and `order by` alone. Shared code is what every operation needs, and nothing that decides how one asks.
2. **A kept operation, other than filter and sort, is a new node**: an unsaved step at the spine's foot with the not-recorded mark and its table in front, as a statement kept from the palette is. Filter and sort stay views of the node in front.
3. **The preview runs arc's own model on each edit.** The connection holds the table and the steps upstream of the draft from the moment the draft starts, so an edit runs only the draft. arc's round trip, about 150 ms a call when this plan was written, is the floor until arc can render a model in-process (see "What arc lacks").
4. **Fade, don't clear.** While an edit fails, the grid keeps the last good rows faded, with the reason on the status band; it is cleared only when nothing can be shown.
5. **A preview's header follows the preview.** Each column's missing count, and the table's row count, come with each page, so filling a column empties its validity band as the value is typed.
6. **Direct actions keep at once.** `hide-column` and `dedupe-rows` ask nothing and keep their step as soon as arc admits it and DuckDB has run it. `remove-step` takes back a kept draft, and a kept draft in front is corrected in place from its own form.
7. **The defaults come from what Brightfield already knows:**
   - `cast-column` from FineType's DuckDB type for the column's label;
   - `fill-nulls` from the column's storage type (`0`, `'unknown'`, `false`), with its median offered;
   - `summarise-rows` from `count(*)` by the column;
   - `limit-rows` from the top 10 by the column, largest first;
   - `derive-column`'s name from its expression;
   - `join-tables`' keys from the Protocol's model of relationships (dovetail's proposals, or the analyst's), else from the cursor's column, else from the first column both tables share.
8. **`read-file` is the sources browser.** The browser already adds a file with its FineType profile and data package. An arc `read-file` step an agent records still opens and previews.
9. **Names.** The approved rule `<table>_<word>_by_<column>` stays for row operations (`_deduped_by_`, `_summarised_by_`). Column operations read `<table>_<word>_<column>` (`_without_`, `_renamed_`, `_with_`, `_cast_`, `_filled_`). The rest:
   - `limit-rows` is `<table>_first_<count>`;
   - `join-tables` is `<table>_joined_<other>`;
   - `append-rows` is `<table>_appended`.
10. **Keys** take VisiData's where it has one: `-` hide, `=` derive, `f` fill, `shift-f` summarise (its frequency table), `&` join. The rest are `shift-r` rename, `shift-t` cast, `shift-u` dedupe, `shift-l` limit and `shift-a` append. A double-click on a header's name renames, a click on its type casts, and a click on its missing count fills.
11. **An operation does not start over a `where` or `order by` that is applied and not recorded**, and says why: it would read the table, not the rows on screen.
12. **`fill-nulls` is greyed on a column with no missing values**, and `dedupe-rows` on a table with no duplicates says so and keeps nothing.
13. **`append-rows` needs another table sharing a column name**, as the spec's sentence "Append rows needs a second table with matching columns" says.
14. **The Editor tab shows the step's SQL read-only**: arc's model, live while the step is being made. Editing it there into a SQL step is the later work the owner split off ("an edit past the operation makes the node a SQL node").

## Global Constraints

- This repository is public. Put no tracker refs, decision ids, milestone ids or card ids, and no absolute local paths, in code, comments, commits, pull request text or branch names. State a ruling in its own words.
- **The owner's ruling on the forms (2026-11-22).** The foot row is only for simple `where` statements and `order by` clauses, a convenience borrowed from DataGrip. "I'm leaning towards a relevant form per operation. Some operations may not need one." His worry about "a sea of dials and dropdowns" is the settings pages of cloud consoles, not forms as such. The aim is a window that recognises and responds to what the analyst does: operations transparent, with sensible configurations, and feeling like magic, as Instant SQL does.
- **arc holds the catalogue.** Brightfield writes no operation's SQL. The preview runs the model arc writes. The step a Save records is `arc operation record`'s, byte for byte what any other way of recording the same operation writes.
- An operation's values, required-ness and words come from `arc operation describe`, run through the window's runner. The words Brightfield adds are its own: labels, menus and sentences.
- **Previews follow each keystroke**, after Instant SQL:
  - an edit runs at once, with no pause;
  - a newer edit supersedes an older one's answer, which is never shown;
  - a failing edit leaves the last good rows faded, with DuckDB's or arc's reason on the status band;
  - a preview is labelled as one, and its header's counts are the preview's own.
- A join's second table, and a union's list, are picked in the spine. A file is brought in through the sources browser.
- Live edit, explicit save: nothing is written to the Protocol before ⌘S.
- Save asks for no name. A name is the rule's, slugged, with `_2`, `_3` for a name taken, and it says only what that one step did. It is fixed when the draft is kept.
- Brightfield holds the stage mapping: Bring in `read-file`; Shape `filter-rows`, `sort-rows`, `hide-column`, `rename-column`, `derive-column`, `cast-column`, `fill-nulls`, `dedupe-rows`, `limit-rows`, `summarise-rows`; Model `join-tables`, `append-rows`.
- Every key is a registry default, and every key the window shows acts. A command joins the registry when its operation is built: a reserved verb shows greyed in the palette and the help sheet, and greyed means a missing input, never a missing feature.
- A command the window cannot take says in a sentence what it needs. Its key, a header's menu row and a palette row say the same sentence.
- The look comes from `meridian-design`'s tokens and the pieces `meridian-egui` already has: `field_row`, `key_chip`, `list_row`, `pane_header`, `picker`, and the theme's `TextEdit`. Add no new component and no colour literal (`tests/token_discipline.rs`, `tests/colour_literals.rs`).
- Each test opens its own copy of a fixture in a temp folder, since Save writes beside the data file. A headless window opens no GPU device, so no target here joins the `serial-targets:` list in `.github/workflows/test.yml`. Pixel baselines are drawn only on the Mac Studio, and no task here needs one.
- A comment that cites a test by name cites one that exists at `HEAD`.
- Before each commit: `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, and the touched tests. After each commit: `cargo xtask ci --step comment-citations`. One cargo build at a time per session. The shell crate's first test build takes minutes; later ones reuse it.
- Commit messages say what is now true, in a sentence, as this repository's history does.

## Review Focus

1. **A column whose name needs quotes** (`median income` and `order` in `tests/data/with_nulls.csv`). A value arc takes as a name is written bare, and a value arc takes as SQL is written as an identifier, so the recorded step reads the right column. Pinned in Task 6 (`a_column_is_given_bare_to_a_name_and_quoted_to_sql`), and in Tasks 9, 10 and 13 on that file.
2. **An operation started over the loaded table's grid while a `where` or `order by` is applied and not recorded.** It would read the table, not the rows shown, so it does not start and says to save or clear the view first. Pinned in Task 6 (`an_operation_over_an_unrecorded_view_says_save_or_clear_it_first`).
3. **⌘S while an operation is still being made.** One arc has admitted is kept, then recorded with the rest. An unfinished or refused one leaves the rest saved and itself open, with its reason. Pinned in Task 6 (`cmd_s_keeps_an_admitted_draft_first`, `cmd_s_saves_the_rest_and_leaves_an_unfinished_draft_open`).
4. **An edit arc or DuckDB refuses** (`limit-rows` count `ten`, a cast a value cannot take). The reason goes on the status band, the last good rows stay faded, Enter keeps nothing, and no file is written. Pinned in Task 4 (`a_refused_edit_keeps_the_last_good_rows_faded`) and in each operation task's refusal test.
5. **Typing faster than arc answers.** The rows and the header always answer the text as it stands: an answer for text since changed is dropped, and the last edit is always read. Pinned in Task 4 (`an_answer_for_text_since_changed_is_never_shown`, `the_last_edit_of_a_burst_is_always_read`).

## What the operations expect of the chrome slices

These are interfaces, not designs; each slice decides its own shape.

**Slice 1 (availability declarations and the menu bar).**
- Its planned `VerbEntry` fields (`label`, `needs`) go on the entries in `operations.rs`, in whichever of slice 1 and Task 2 lands second. Labels: "Hide column", "Rename column", "Derive column", "Cast column", "Fill nulls", "Dedupe rows", "Limit rows", "Summarise rows", "Join tables", "Append rows".
- The needs the operations declare (`operation_ui::Need`, Task 2), and what each would be in slice 1's terms:

| `operation_ui::Need` | Slice 1 | Sentence |
|---|---|---|
| `ProtocolSource` | `Condition::ProtocolSource` | "{command} needs a Protocol of your own: open a data file or a folder." |
| `TakesSources` | `Condition::TakesSources` | "Sources are added to Protocols Brightfield made, and this one was not." |
| `Rows` | `Selected::Rows` | "{command} needs a table's rows." |
| `GridCell` | `Selected::GridCell` | "{command} needs the grid's cursor on a cell." |
| `Column` | a new selection | "{command} needs a column: put the grid's cursor on one, or start from its header." |
| `ColumnHasNulls` | a new condition | "{command} needs a column with missing values." |
| `ViewRecorded` | a new condition | "{command} reads the table as recorded: save or clear the where and order by first." |
| `LoadedTable` | a new condition | the grid preview's own offer ("… has not been run; Run the Protocol to filter or sort its rows") |
| `AnotherTable` | a new condition | "{command} needs a second table in the Protocol." |
| `AnotherTableSharingAColumn` | a new condition | "{command} needs a second table with matching columns." |

- One evaluator answers for the palette, the menus, the ribbon, a header's menu and a key press alike. An operation that cannot start stays listed and keeps its key.
- The menus and the help sheet list an operation's verb once its slot is built, read from `brightfield_keys::operations::operation_verbs()` and grouped by stage, so an operation joins a menu with no edit to a menu table.
- A menu item reaches an operation by sending `Request::Verb(<verb>)`; `MeridianApp::start_operation_verb` is the one entry point. Nothing in this plan waits on the menu bar.
- If slice 1 has already folded `chart_offers` into one palette list, or factored the filter and sort asks out of `grid_cursor_keys`, Task 6 adds the built verbs to that list and calls the factored ask, rather than doing either again.
- If slice 1's sweep that every listed key is answered has landed, the operation host records each verb a key performed, where that sweep reads it.

**Slice 2 (the ribbon).** The ribbon reads built operations from `operation_ui::built()`: their `stage`, `verb` and `icon`. It does not keep a list of them, so an operation task never edits `ribbon.rs`. `operation_ui::Stage` has no Publish; whichever of the ribbon and Task 2 lands second makes the two `Stage` enums one. Most of the icons named here are not yet in `meridian-egui`'s Tabler manifest (it holds 48), and adding them is the design system's work for the ribbon.

**Slice 5 (the inspector).**
- The Operation section is where Task 7 hosts an operation's own form, and where filter and sort keep today's explanation.
- The column section's "what can be done with it" lists each built operation that starts from a header, with its key, read from `operation_ui::built()`. Choosing one starts it on that column through `start_operation`.

## File structure

| File | Responsibility |
|---|---|
| `crates/brightfield-shell/src/operation.rs` | arc's description, whole; `record_step` naming each read by its role |
| `crates/brightfield-shell/src/operation_ui/mod.rs` | `OperationUi`, `Draft`, `Begin`, `Need`, `Start`, and the table of 13 slots |
| `crates/brightfield-shell/src/operation_ui/<op>.rs` (13) | one operation: where it starts, its defaults, its in-place field or form, its name |
| `crates/brightfield-shell/src/operation_ui/draft.rs` | arc's model for a draft, from a throwaway Protocol |
| `crates/brightfield-shell/src/operation_ui/reads.rs` | the draft's reads: arc per edit, the latest answer wins, on the warm preview |
| `crates/brightfield-shell/src/operation_ui/open.rs` | `OperationContext`, `Availability` and the sentences |
| `crates/brightfield-shell/src/window/operation_host.rs` | the window's side: start, keys, reads each frame, keep, cancel, status (a child module of `window`, so it reaches the window's private fields) |
| `crates/brightfield-shell/src/spine_pick.rs` | the spine picking a table for an operation |
| `crates/brightfield-keys/src/operations.rs` | one key-registry entry per operation, behind its own `*_BUILT` |
| `crates/brightfield-engine/src/preview.rs` | `PreviewOp::Model`, `PreviewOp::ReadFile` and the warm preview |
| `crates/brightfield-shell/src/generated.rs` | a generated step of any operation read back; `effect_step_name` |
| `crates/brightfield-shell/tests/support/operations.rs` | the harness every operation test drives the window with |
| `docs/operation-forms.md` | the shared parts, the calls above, and how to add an operation |

---

### Task 1: The whole of arc's description, and a request that names each read by its role

**Files:**
- Modify: `crates/brightfield-shell/src/operation.rs` (`Parameter`, `Role`, `OperationDescription`, `describe`; add `ValueShape`, `SqlKind`, `StepRequest`, `Recorded`, `record_step`; `record` becomes a call to `record_step`; module doc)
- Test: `crates/brightfield-shell/tests/operation_descriptions.rs` (new)

**Interfaces:**
- Consumes: `crate::run::{arc_with_args, Runner}`.
- Produces:

```rust
pub enum SqlKind { Condition, Order, Expression, SelectList, Partition, Type }
pub enum ValueShape {
    Sql(SqlKind),                                   // x-kind condition | order | expression | select-list | partition | type
    Column,                                         // a string with x-kind column
    Columns,                                        // an array whose items have x-kind column
    Integer { minimum: Option<i64>, maximum: Option<i64> },
    Choice { words: Vec<String>, default: Option<String> },   // an enum
    Path,                                           // x-kind path
    Text,                                           // a string with no x-kind
}
pub struct Parameter { pub key: String, pub description: String, pub offered: Vec<Offered>, pub shape: ValueShape, pub required: bool }
pub struct Role { pub name: String, pub kind: String, pub required: bool, pub description: String, pub list: bool, pub min_items: usize }
pub struct OperationDescription { pub long_name: String, pub summary: String, pub reads: Vec<Role>, pub parameters: Vec<Parameter>,
    pub needed_together: Vec<(String, Vec<String>)> }   // dependentRequired, in key order
pub struct StepRequest { pub long_name: String, pub on: Option<String>, pub reads: Vec<(String, String)>, pub args: Vec<(String, String)> }
pub struct Recorded { pub said: String, pub notes: Vec<String> }   // stdout; stderr lines that open with `note: `, prefix dropped
pub fn record_step(runner: &Runner, dir: &Path, name: &str, request: &StepRequest) -> Result<Recorded, String>
```

`record_step` runs `operation record <long_name> [--on <on>] [--read <role>=<table>]… --name <name> [--arg <key>=<value>]…` in `dir` with no `--dir`, as `record` does. A list role is given once per table, in order. A `Columns` value is passed as its items joined by `,`, arc's command-line form. arc writes its notes to stderr even when it records, so they are kept.

- [ ] **Step 1: Write the failing tests** in `operation_descriptions.rs`. The runner is `Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))`, so every shape is read from arc at the pinned rev.

```rust
#[test] fn every_operation_arc_lists_is_described() {
    // list_operations: exactly the 13 long names of the spec's table; describe() is Ok for each.
}
#[test] fn each_value_has_the_shape_arc_gives_it() {
    use ValueShape::*; use SqlKind::*;
    let want = [
        ("filter-rows","where",Sql(Condition),true), ("sort-rows","order_by",Sql(Order),true),
        ("hide-column","column",Column,true), ("rename-column","column",Column,true), ("rename-column","to",Column,true),
        ("derive-column","column",Column,true), ("derive-column","expression",Sql(Expression),true),
        ("cast-column","column",Column,true), ("cast-column","type",Sql(Type),true),
        ("fill-nulls","column",Column,true), ("fill-nulls","value",Sql(Expression),true),
        ("dedupe-rows","key",Sql(Partition),false), ("dedupe-rows","order_by",Sql(Order),false),
        ("limit-rows","count",Integer{minimum:Some(0),maximum:Some(i64::MAX)},true), ("limit-rows","order_by",Sql(Order),false),
        ("summarise-rows","aggregates",Sql(SelectList),true), ("summarise-rows","group_by",Sql(SelectList),false),
        ("join-tables","using",Columns,false), ("join-tables","condition",Sql(Condition),false), ("join-tables","prefix",Text,false),
        ("read-file","path",Path,true),
    ];
    // each found by (long_name, key) with that shape and required flag;
    // join-tables' kind is Choice { words: [left, inner, right, full, semi, anti, cross], default: Some("left") }, not required;
    // append-rows has no parameter.
}
#[test] fn the_reads_say_which_tables_and_whether_a_list() {
    // join-tables' `other`: required, !list. append-rows' `other`: required, list, min_items 1.
    // read-file reads nothing. Every other operation reads a required `table`.
}
#[test] fn dedupe_rows_needs_its_key_and_order_together() {
    // needed_together == [("key", ["order_by"]), ("order_by", ["key"])]
}
#[test] fn a_request_names_each_read_by_its_role() {
    // In a temp Protocol of two SQL steps making `housing` and `by_age`:
    // join-tables on housing, reads [("other","by_age")], args [("using","house_age")] -> the model holds "LEFT JOIN by_age USING (house_age)";
    // append-rows on housing, reads [("other","by_age"),("other","housing")] -> "UNION ALL BY NAME" twice, by_age first.
}
#[test] fn a_refused_request_returns_arcs_words_and_writes_nothing() {
    // limit-rows count "ten": Err contains "is an integer, not \"ten\""; every file in the folder is byte-identical after.
}
#[test] fn arcs_notes_are_kept_when_it_records() {
    // read-file path "https://example.com/x.csv": Ok, notes[0] contains "is read live at each run".
}
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test operation_descriptions`. Expected: FAIL to compile (`ValueShape`, `record_step` not found).
- [ ] **Step 3: Implement** the shapes in `describe`. `shape` is read from the property's `type`, `x-kind`, `enum`, `default`, `minimum`, `maximum` and `items`. `required` comes from the schema's `required` list, and `needed_together` from its `dependentRequired`; a role's `list` and `min_items` are read off the role. Then write `record_step`, and make `record` call it. Update the module doc's paragraph on recording.
- [ ] **Step 4: Run.** Run: `cargo test -p brightfield-shell --test operation_descriptions --test operation_runner --test operation_panel --test save_records_operation`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Read the whole of arc's description of an operation, and record one naming each table it reads by its role"`, then `cargo xtask ci --step comment-citations`.

### Task 2: A slot for every operation, in Brightfield's operation registry and the key registry

**Files:**
- Create: `crates/brightfield-shell/src/operation_ui/mod.rs`
- Create: `crates/brightfield-shell/src/operation_ui/{filter_rows,sort_rows,hide_column,rename_column,derive_column,cast_column,fill_nulls,dedupe_rows,limit_rows,summarise_rows,join_tables,append_rows,read_file}.rs`. Each holds `pub fn ui() -> Option<&'static OperationUi> { None }` and a module doc of one line naming the operation and saying it is not built yet.
- Modify: `crates/brightfield-shell/src/lib.rs` (`pub mod operation_ui;`, and a line in the crate doc's list)
- Create: `crates/brightfield-keys/src/operations.rs`
- Modify: `crates/brightfield-keys/src/lib.rs` (`pub mod operations;`)
- Modify: `crates/brightfield-keys/src/registry.rs` (`registry()` appends `crate::operations::operation_verbs()`; `longname_snapshot_is_stable` compares the registry less `operations::is_operation_verb` names; `the_grid_context_moves_the_cursor_a_cell_on_each_of_its_keys` adds each built operation verb's Grid key to `expected`, read from `operations::operation_verbs()`)
- Test: `crates/brightfield-shell/tests/operation_registry.rs` (new); unit tests in `operations.rs`

**Interfaces:**
- Consumes: `crate::grid_foot::FootField`, `crate::one_step::ColumnFacts`, `crate::operation::{OperationDescription, StepRequest}`, `brightfield_ossie::Model`, `brightfield_protocol::silver::FinetypeTransforms`.
- Produces (`operation_ui/mod.rs`):

```rust
pub struct OperationUi {
    pub long_name: &'static str,         // arc's
    pub verb: &'static str,              // the registry verb that starts it
    pub stage: Stage,
    pub icon: &'static str,              // a Tabler name, for the ribbon
    pub starts: &'static [Start],        // the places it begins from
    pub menu_words: &'static str,        // its row in a header's menu: "Hide column", "Summarise by this column"
    pub needs: &'static [Need],
    pub begin: fn(&Begin<'_>) -> Result<Draft, String>,   // the draft it starts as: its sensible defaults
    pub keeps: Keeps,
    pub field: Option<InPlace>,          // a field drawn in place on the header, when it asks there
    pub form: Option<fn(&mut Draft, &mut egui::Ui, &FormCx<'_>) -> FormEvent>,   // its form in the Operation section, when it has one
    pub name: NameRule,
}
pub enum Stage { BringIn, Shape, Model }
pub enum Start { Cell, Header(HeaderPart), Spine { role: &'static str }, FootField(FootField), SourcesBrowser }
pub enum HeaderPart { Menu, Name, Type, Missing, NewColumn }
pub enum Keeps { AtOnce, OnEnter }
pub struct InPlace { pub part: HeaderPart, pub keys: &'static [&'static str], pub suggestions: fn(&Begin<'_>, &Draft) -> Vec<String> }
pub enum Need { ProtocolSource, TakesSources, Rows, GridCell, Column, ColumnHasNulls, ViewRecorded, LoadedTable,
                AnotherTable, AnotherTableSharingAColumn }
pub struct Draft { pub long_name: &'static str, pub on: Option<String>, pub reads: Vec<(String, String)>,
                   pub values: Vec<(&'static str, String)>, pub focus: Option<&'static str> }
impl Draft {
    pub fn empty(long_name: &'static str) -> Self;           // no table, reads or values
    pub fn value(&self, key: &str) -> Option<&str>;
    pub fn set(&mut self, key: &'static str, text: String);
    pub fn request(&self) -> StepRequest;                     // args are the values that are not empty
    pub fn missing(&self, description: &OperationDescription) -> Option<String>;   // "<long name> needs <key>"
}
pub struct Begin<'a> {
    pub start: Start, pub table: Option<&'a str>, pub column: Option<&'a str>, pub columns: &'a [String],
    pub facts: &'a [ColumnFacts], pub description: &'a OperationDescription,
    pub taken: &'a [(String, Vec<String>)],                   // tables the spine took, with their columns
    pub model: Option<&'a Model>, pub transforms: &'a FinetypeTransforms,
    pub quote: &'a dyn Fn(&str) -> String,                    // a column as SQL spells it
}
pub struct FormCx<'a> { pub begin: &'a Begin<'a>, pub answer: Option<&'a DraftAnswer>, pub mode: crate::design::Mode }
pub enum FormEvent { None, Changed, Keep, Cancel }
pub struct NameRule { pub word: &'static str, pub from: By, pub with_by: bool }
pub enum By { Nothing, Value(&'static str) }
pub const OPERATIONS: [(&str, fn() -> Option<&'static OperationUi>); 13];   // arc's order: filter_rows::ui, sort_rows::ui, …
pub fn spec(long_name: &str) -> Option<&'static OperationUi>
pub fn by_verb(verb: &str) -> Option<&'static OperationUi>
pub fn built() -> impl Iterator<Item = &'static OperationUi>
pub fn label(long_name: &str) -> String   // "hide-column" -> "Hide column"
```

`DraftAnswer` is Task 4's. Declare it here as a forward stub, `pub struct DraftAnswer;` in `reads.rs`, so this task compiles; Task 4 fills it.

`By::Value(key)` names the step by the value's first comma-separated item. It drops a surrounding `"` and a trailing `asc` or `desc`, and names nothing when what is left holds anything but letters, digits, `_` and spaces.

- Produces (`operations.rs`): `pub fn operation_verbs() -> Vec<VerbEntry>` (the built ones, in arc's order); `pub fn is_operation_verb(longname: &str) -> bool`; and `pub fn all_operation_entries() -> Vec<(bool, VerbEntry)>` for the key-collision test. Each operation is a block of its own, separated from its neighbours by its entry function:

```rust
// ---- hide-column ----
const HIDE_COLUMN_BUILT: bool = false;
fn hide_column() -> VerbEntry { VerbEntry { longname: "hide-column", binding_specs: vec![grid("-")], … } }
```

Every entry: `tier: CommandTier::View`, `drives: Drives::Navigation`, `status: VerbStatus::Built`, `reserved_reason: None`, `scope_applicability: vec![View]`. `read-file` has no entry, since its command is `open-sources-browser`. The keys and help, decided here once so that no two tasks pick the same key:

| Verb | Key | Context | Help | Scores (f, m, c): note |
|---|---|---|---|---|
| `hide-column` | `-` | Grid | Hide the cursor's column; its header's menu does the same | 3, 4, 4: "- is VisiData's hide; the Workspace's - zooms in its own context" |
| `rename-column` | `shift-r` | Grid | Rename the cursor's column in its header; a double-click on the name does the same | 3, 4, 2: "R = rename; bare r stays free for the grid's movement" |
| `derive-column` | `=` | Grid | Add a column from an expression, typed in a new header at the table's end | 3, 4, 4: "= is VisiData's new column from an expression" |
| `cast-column` | `shift-t` | Grid | Change the cursor's column's type, starting from the type FineType reads | 2, 4, 2: "T = type; retype-column is the model's, in its own context" |
| `fill-nulls` | `f` | Grid | Fill the cursor's column's missing values, starting from a suggested value | 2, 4, 3: "f is VisiData's fill; the grid's filter is w" |
| `dedupe-rows` | `shift-u` | Grid | Remove the table's duplicate rows | 2, 3, 2: "U = unique; bare u is undo elsewhere" |
| `limit-rows` | `shift-l` | Grid | Keep the top rows by the cursor's column: ten, largest first, until changed | 2, 4, 2: "L = limit; bare l moves the cursor right" |
| `summarise-rows` | `shift-f` | Grid | Summarise the rows by the cursor's column: a count of each value, then the values to add | 3, 3, 5: "F is VisiData's frequency table of the cursor's column" |
| `join-tables` | `&` | Grid | Join the table in front to one picked in the spine, on keys proposed from the model or the cursor's column | 2, 4, 5: "& is VisiData's join" |
| `append-rows` | `shift-a` | Grid | Append the rows of tables picked in the spine, matching columns by name | 2, 4, 3: "A = append, as vim's A" |

- [ ] **Step 1: Write the failing tests.** In `operations.rs`:

```rust
#[test] fn no_operation_key_is_taken_in_its_context() {
    // for every entry, built or not: no other registry verb, and no other operation entry,
    // binds the same keystrokes in the same BindingContext
}
#[test] fn only_built_operations_join_the_registry() {
    // operation_verbs() is exactly the entries whose *_BUILT is true, in arc's order;
    // registry() holds each of them and no unbuilt one
}
#[test] fn an_operations_verb_is_its_long_name_and_writes_nothing() { /* longname == long name; tier View */ }
```

In `operation_registry.rs`, with the suite's runner:

```rust
#[test] fn every_operation_arc_holds_has_one_slot() { /* OPERATIONS' names == list_operations(), as sets, 13 */ }
#[test] fn an_operation_is_built_exactly_when_its_command_is_in_the_key_registry() {
    // spec(op).is_some() <=> registry() has its verb; filter-rows' verb is filter-rows-at-cursor,
    // sort-rows' sort-rows-at-cursor, read-file's open-sources-browser; an unbuilt op's long name is no registry verb
}
#[test] fn each_built_operations_draft_names_only_what_arc_describes() {
    // begin() over a context the test builds from the housing sample: each value key is a parameter arc describes,
    // each read role is a table role it describes; an InPlace key is a value key
}
#[test] fn each_built_operations_needs_agree_with_what_arc_says_it_reads() {
    // ProtocolSource or TakesSources always; Rows iff a required `table` role; AnotherTable iff a required single `other`;
    // AnotherTableSharingAColumn iff a required list `other`
}
#[test] fn each_built_operation_is_filed_under_its_stage() { /* read-file BringIn; join-tables, append-rows Model; the rest Shape */ }
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-keys` then `cargo test -p brightfield-shell --test operation_registry`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** the two files and the stubs, and fold `operation_verbs()` into `registry()`. Then reword the two registry tests named above, so that an operation built later edits neither.
- [ ] **Step 4: Run.** The same two commands. Expected: PASS. The shell tests pass with nothing built, and they bite as each operation fills its slot.
- [ ] **Step 5: Commit.** `git commit -m "Give each of arc's operations a slot in Brightfield's operation registry and the key registry, empty until it is built"`, then `--step comment-citations`.

### Task 3: A step arc generated for any operation opens, lays out and previews

**Files:**
- Modify: `crates/brightfield-engine/src/preview.rs` (`PreviewOp::Model`; module doc)
- Modify: `crates/brightfield-shell/src/generated.rs` (`StepEffect::Generated`; `parse_any_generated`; `chain_of`; module doc)
- Modify: `crates/brightfield-shell/src/one_step.rs` (`folder_inputs` hands the graph a lineage model for a `Generated` step, as for a SQL step)
- Modify: `crates/brightfield-shell/src/window.rs` (`preview_request`: a `Generated` step shows no field)
- Modify (compile only): each other `match` on `PreviewOp` (`rg -n 'PreviewOp::' crates`)
- Test: `crates/brightfield-engine/tests/preview_model.rs` (new), `crates/brightfield-shell/tests/generated_any_operation.rs` (new)

**Interfaces:**
- Consumes: Task 1's `record_step` (the test builds its folder through arc).
- Produces:

```rust
pub enum PreviewOp { Where(String), OrderBy(String), Statement(String),
    /// arc's model for a generated step, run as written once the preview's access is shut; it makes the step's table.
    Model(String) }
pub enum StepEffect { Operation(KnownOperation), Generated(String /* long name */), Sql }
pub fn parse_any_generated(step_name: &str, long_name: &str, tables: &[String], model_sql: &str) -> Result<GeneratedStep, String>
```

A model runs after `enable_external_access` is off and the configuration is locked. Whatever it does lands in the preview's own in-memory database, which is the guard for a model's several statements (`rename-column` and `derive-column` write a transaction). Once it has run, a table of the step's name must exist; otherwise the read refuses with `PreviewRefusal::Engine("<step>: arc's model made no table of its name")`. The page of a `Model` step is read in the order its rows were made, as a `Statement`'s is.

`parse_any_generated` takes any long name that is neither `filter-rows` nor `sort-rows`. The model must hold the line `CREATE OR REPLACE TABLE "<step>" AS` second, or third after `BEGIN TRANSACTION;`. `reads` are the tables after ` on ` in the header, split on `, `; `read-file`'s header has none. `on` is the first of them, and `argument` is the model as written. Anything else is refused as `step "<name>" carries the generated header but its model does not make "<name>"`. In `chain_of`, a `Generated` step becomes `PreviewOp::Model(argument)`.

- [ ] **Step 1: Write the failing tests.** In `preview_model.rs`, over a temp CSV `a,c\n1,x\n2,y\n`:

```rust
#[test] fn a_model_makes_its_step_and_the_next_step_reads_it() {
    // chain: ("renamed", Model("BEGIN TRANSACTION;\nCREATE OR REPLACE TABLE \"renamed\" AS\nSELECT *\nFROM t;\nALTER TABLE \"renamed\" RENAME COLUMN a TO b;\nCOMMIT;\n")),
    //        ("kept", Where("b > 1"))  -> one row, columns [b, c]
}
#[test] fn a_model_cannot_reach_outside_the_preview() {
    // Model holding `COPY (SELECT 1) TO '<tmp>/x.csv'` -> Err(Engine(..)) and no file at <tmp>/x.csv;
    // Model holding `SET enable_external_access = true` -> Err(Engine(..))
}
#[test] fn a_model_that_makes_no_table_of_its_name_is_refused() {
    // step "named", Model("CREATE OR REPLACE TABLE \"other\" AS SELECT 1;") -> Err(Engine("named: arc's model made no table of its name"))
}
```

In `generated_any_operation.rs`, on a copy of `california_housing_sample.csv` saved once so that its load is written, with steps recorded through `record_step`:
- `h`: hide-column on the root (`column=house_age`);
- `r`: rename-column on `h` (`median_income` to `income`);
- `s`: summarise-rows on the root (`count(*) AS n`, grouped by `house_age`);
- `j`: join-tables on the root with `other=s`, `using=house_age`;
- `a`: append-rows on `h` with `other=r`.

```rust
#[test] fn a_folder_of_any_generated_steps_opens_as_its_chain() {
    // generated_protocol_in: names [h, r, s, j, a]; each effect Generated(<its long name>);
    // reads h [root], r [h], s [root], j [root, s], a [h, r]
}
#[test] fn each_generated_step_previews_through_what_it_reads() {
    // preview_chain(a) names [h, r, a]; r's first page has `income` and no `median_income`;
    // j's first page has a column `s_n`
}
#[test] fn the_window_lays_the_steps_out_not_run() {
    // a headless window on the folder: the spine lists h, r, s, j, a, none run;
    // r's grid is a preview (PREVIEW_LABEL) whose columns include `income`
}
#[test] fn a_generated_model_edited_away_from_its_step_is_refused_naming_it() {
    // r's model with its CREATE target rewritten to "other": generated_protocol_in is Err containing `step "r"`
}
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-engine --test preview_model` then `cargo test -p brightfield-shell --test generated_any_operation`. Expected: FAIL (`Model` not found; the folder refused as "not an operation brightfield reads").
- [ ] **Step 3: Implement** the variant in the preview's chain loop, `parse_any_generated` in `parse_generated`'s refusal arm, the `chain_of` arm, the lineage arm and the `preview_request` arm. Update both module docs: say what a model may do in the preview and why that is safe there.
- [ ] **Step 4: Run.** The two commands above, then `cargo test -p brightfield-shell --test generated_steps --test typed_sql_nodes --test typed_sql_reads --test save_records_operation`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Open, lay out and preview a step arc generated for any operation, by running arc's own model in the preview"`, then `--step comment-citations`.

### Task 4: The warm preview: arc's model for a draft, read at each edit, the latest answer winning

**Files:**
- Modify: `crates/brightfield-engine/src/preview.rs` (`WarmPreview`; `PreviewOp::ReadFile`; module doc)
- Modify: `crates/brightfield-engine/src/lib.rs` (re-exports)
- Modify: `crates/brightfield-shell/src/generated.rs` (`chain_of`: a `Generated("read-file")` step whose path is a local CSV or Parquet becomes `PreviewOp::ReadFile`, the path resolved against the Protocol's folder; any other path previews nothing, saying "Brightfield previews a local CSV or Parquet file; Run reads <path>.")
- Create: `crates/brightfield-shell/src/operation_ui/draft.rs`, `crates/brightfield-shell/src/operation_ui/reads.rs` (replacing Task 2's stub `DraftAnswer`)
- Modify: `crates/brightfield-shell/src/operation_ui/mod.rs` (`pub mod draft; pub mod reads;`)
- Test: `crates/brightfield-engine/tests/warm_preview.rs` (new), `crates/brightfield-shell/tests/operation_reads.rs` (new)

**Interfaces:**
- Consumes: Task 1's `StepRequest`, `record_step`; Task 3's `PreviewOp::Model`; the engine's `FileReader`, `PreviewStop`, `PreviewRefusal`, `PREVIEW_DEADLINE`, `PREVIEW_FILE_LIMIT_BYTES`.
- Produces:

```rust
// brightfield-engine
pub enum PreviewOp { /* as Task 3 */ ReadFile { path: PathBuf, reader: FileReader } }   // a further root, read before access is shut
pub struct WarmPreview { /* one connection */ }
pub struct WarmPage { pub batches: Vec<RecordBatch>, pub rows: u64, pub rows_in: u64, pub missing: Vec<(String, u64)> }
impl WarmPreview {
    /// Load `data` as `root_table` (and each ReadFile of `upstream`), make each other step of `upstream`, then shut access and lock.
    pub fn open(data: &Path, reader: FileReader, root_table: &str, upstream: &[(String, PreviewOp)], file_limit: u64) -> Result<Self, PreviewRefusal>;
    /// Run `op` as the step `name` over what `open` made (dropping the last draft's table first), and read its first `limit` rows,
    /// its row count, the row count of the table it reads (`on`), and each column's missing count.
    pub fn read(&mut self, name: &str, on: Option<&str>, op: &PreviewOp, limit: u64, stop: &PreviewStop) -> Result<WarmPage, PreviewRefusal>;
}
// brightfield-shell, operation_ui/draft.rs
pub struct DraftModel { pub model: String, pub notes: Vec<String> }
pub fn draft_model(runner: &Runner, name: &str, request: &StepRequest) -> Result<DraftModel, String>;
// operation_ui/reads.rs
pub struct DraftAnswer { pub request: StepRequest, pub name: String, pub model: DraftModel, pub page: GridPage,
    pub rows: u64, pub rows_in: u64, pub missing: Vec<(String, u64)>, pub unread: Option<String> }
pub struct DraftReads { /* the worker that owns the WarmPreview */ }
impl DraftReads {
    pub fn start(source: crate::typed_sql::TypedSource, upstream_of: Vec<String> /* tables the draft reads */, wake: impl Fn() + Send + Sync + 'static) -> Self;
    pub fn edit(&mut self, name: &str, request: StepRequest);   // read at once; an answer for an older request is dropped
    pub fn poll(&mut self) -> bool;                               // true when what is shown changed
    pub fn current(&self) -> Option<&DraftAnswer>;               // the answer for the request as it stands
    pub fn shown(&self) -> Option<&DraftAnswer>;                 // the last good answer, maybe for older text
    pub fn refusal(&self) -> Option<&str>;                       // arc's or DuckDB's words for the request as it stands
    pub fn faded(&self) -> bool;                                  // shown() is not current(): the grid draws it faded
}
```

**`draft_model`** writes, in a temp folder it removes after, a manifest naming one stub step per table the request reads (`bf_stub_1`, `bf_stub_2`, …). Each stub's model is `CREATE OR REPLACE TABLE "<table>" AS SELECT 1 AS x;`, the table spelled as a quoted identifier, since arc finds what a step makes from its model. It then calls `record_step` there, and reads the model back through the manifest's `sql:` for `name`. The text is what arc writes for the same request in the real Protocol, byte for byte: arc's spelling of a name does not depend on the folder.

**`DraftReads`.** One worker thread owns the `WarmPreview`, opened over the draft's upstream chain (`preview_chain_reading` of the tables it reads) when the draft starts. An `edit` with arc's previous call still running waits behind it, and only the newest waiting request is read when that call returns, so at most one arc child runs and the last edit of a burst is always read. Each read gets arc's model, then runs it with `PreviewOp::Model`. A refusal from arc, or a `PreviewRefusal::Engine`, is the read's refusal, and `shown()` stays the last good answer. A `TooLarge` or `TookTooLong` answers with an empty page and `unread` saying why. A read starts only once the request has changed, and `PreviewStop` interrupts a read a newer edit has passed.

- [ ] **Step 1: Write the failing tests.** In `warm_preview.rs`:

```rust
#[test] fn the_table_is_loaded_once_and_each_read_runs_only_the_draft() {
    // open over a temp CSV; two reads with different Models; the file's modification after open does not change the second page
}
#[test] fn a_read_reports_the_rows_the_rows_it_read_and_each_columns_missing_count() {
    // with_nulls copy: Model filling house_age -> rows 3, rows_in 3, missing [("house_age", 0), ...]
}
#[test] fn a_second_file_is_read_before_access_is_shut() { /* upstream holds ReadFile of a second CSV; a Model joining the two reads */ }
#[test] fn a_draft_cannot_reach_outside_the_preview() { /* as Task 3's, on the warm connection */ }
```

In `operation_reads.rs`, with the suite's runner on a copy of the housing sample:

```rust
#[test] fn a_drafts_model_is_what_arc_records_in_the_protocol() {
    // draft_model(runner, "h", &hide) == the model record_step writes for "h" in a copy of the folder, byte for byte
}
#[test] fn an_edit_is_read_with_no_pause() { /* edit, poll until current(): elapsed < TYPED_PAUSE + arc's call; no pause applied */ }
#[test] fn an_answer_for_text_since_changed_is_never_shown() {
    // edit rename `to` "a", then "ab" before the first answer: current().request's `to` is "ab"; no frame showed "a" as current
}
#[test] fn the_last_edit_of_a_burst_is_always_read() { /* ten edits in one frame -> current() answers the tenth */ }
#[test] fn a_refused_edit_keeps_the_last_good_rows_faded() {
    // a good edit, then limit-rows count "ten": refusal() holds arc's `is an integer, not "ten"`; shown() is the good one; faded()
}
#[test] fn a_preview_too_large_still_answers_with_arcs_model() { /* file_limit 1: current() has model, empty page, unread Some */ }
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-engine --test warm_preview` then `cargo test -p brightfield-shell --test operation_reads`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** `WarmPreview` beside the cold reads, sharing their load, shutting and limits, then `draft_model` and `DraftReads`. In `preview.rs`'s module doc, say that the warm connection holds the whole table, as Instant SQL's cache does not, so a preview of an aggregate is exact.
- [ ] **Step 4: Run.** The two commands, then `cargo test -p brightfield-engine --test preview_read --test preview_model`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Preview a draft at each edit on a warm connection that holds the table, running the model arc writes for it"`, then `--step comment-citations`.

### Task 5: Keeping a draft, taking it back, recording it on ⌘S, and its SQL in the Editor tab

**Files:**
- Modify: `crates/brightfield-shell/src/typed_sql.rs` (`TypedStep::operation`; `as_generated`; module doc)
- Modify: `crates/brightfield-shell/src/generated.rs` (`effect_step_name`; `sql_step_name_for` calls it with `"sql"`)
- Modify: `crates/brightfield-shell/src/app.rs` (`typed_lines` and a pending step's version words name the operation; `replace_typed` clears `operation`; `replace_draft`; `discard_kept`)
- Modify: `crates/brightfield-shell/src/protocol.rs` (`removable_step` answers a kept step too)
- Modify: `crates/brightfield-shell/src/window.rs` (`record_pending` records a draft through `record_step`; `remove-step` over a kept step discards it, refused while a later kept step reads it; the bring-to-front tail of `keep_statement` becomes `bring_kept_to_front`, which both call; `pub fn keep_draft`; the statement form does not hold a kept draft; the Editor tab shows the SQL of the step in front)
- Modify: `crates/brightfield-shell/src/editor.rs` (`EditorPane::show_sql(title, text)`: an in-memory read-only buffer, with no file, in the read-only treatment)
- Modify (compile only): `crates/brightfield-shell/tests/typed_sql_nodes.rs` (`operation: None` in `typed`)
- Test: `crates/brightfield-shell/tests/operation_keep.rs` (new)

**Interfaces:**
- Consumes: Task 1's `StepRequest`, `record_step`; Task 3's `StepEffect::Generated`; Task 4's `draft_model`.
- Produces:

```rust
// typed_sql.rs
pub struct TypedStep { pub name: String, pub statement: String, pub reads: Vec<String>, pub rows: Option<u64>,
    pub operation: Option<StepRequest> }       // Some for a step an operation made; `statement` is then arc's model
// generated.rs
pub fn effect_step_name(word: &str, table: Option<&str>, column: Option<&str>, with_by: bool, taken: &BTreeSet<String>) -> String
// app.rs (ChartDoc)
pub(crate) fn replace_draft(&mut self, name: &str, request: StepRequest, model: String);   // keeps its name and place
pub(crate) fn discard_kept(&mut self, name: &str) -> Result<(), String>;                  // refused, naming them, while a later kept step reads it
// window.rs
impl MeridianApp { pub fn keep_draft(&mut self, ctx: &egui::Context, name: String, request: StepRequest, model: String) }
// editor.rs
impl EditorPane { pub fn show_sql(&mut self, title: &str, text: &str) }
```

`effect_step_name` names a step in one of three forms:
- `<table>_<word>_by_<column>` when `with_by` is set;
- `<table>_<word>_<column>` when it is not;
- `<table>_<word>` with no column.

It slugs each part and suffixes a taken name as `sql_step_name_for` does today. `as_generated` gives `StepEffect::Generated(long_name)` for a step with an operation. Words: `typed_lines` gives `"<long name> · <name>"` for such a step and `"SQL step · <name>"` otherwise, and the version a close keeps uses the same. A palette opened over a kept draft's grid starts empty.

**The Editor tab shows the step in front's SQL** while the bottom band shows the Editor and no file of its own is being edited. That is arc's model for a kept draft, or the model file of a recorded generated step, read-only, titled with the step's name. Task 6 adds the draft being made, live.

- [ ] **Step 1: Write the failing tests** in `operation_keep.rs`, on a copy of the housing sample. `hide` is `StepRequest { long_name: "hide-column", on: Some("california_housing_sample"), reads: [], args: [("column","house_age")] }`, and `m` is `draft_model(runner, "h", &hide)`.

```rust
#[test] fn a_kept_draft_is_an_unsaved_step_at_the_spines_foot() {
    // keep_draft(.., "h", hide, m.model): has_unsaved_work(); typed_steps()[0].operation.is_some(); is_typed("h");
    // the canvas holds h's grid without house_age; close-question lines hold "hide-column · h"
}
#[test] fn save_records_a_kept_draft_as_arc_writes_it() { /* save -> the folder's model for h == m.model, byte for byte */ }
#[test] fn save_records_statements_and_drafts_in_the_order_they_were_made() {
    // keep a typed statement, then h; save -> the manifest's new steps are [that statement's name, h]
}
#[test] fn remove_step_takes_a_kept_draft_back() { /* select h on the spine, remove-step -> typed_steps() empty; nothing written */ }
#[test] fn a_kept_draft_another_reads_is_not_taken_back() {
    // keep h, keep r reading h; remove-step on h -> refused naming r; both still kept
}
#[test] fn a_draft_arc_refuses_at_save_stays_kept_in_arcs_words() {
    // keep h; record a step named h into the folder through record_step, as an agent would; save
    // -> banner body in arc's words; typed_steps() still holds h; has_unsaved_work()
}
#[test] fn the_editor_tab_shows_the_step_in_fronts_sql_read_only() {
    // keep h; the Editor tab's text == m.model and it is read-only; after save, the model file's text
}
#[test] fn the_statement_form_over_a_drafts_grid_starts_empty() { /* Space over h's grid: the form's text is "" */ }
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test operation_keep`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** in the order of the Files list. Put `record_step` into the `PendingWork::Typed` arm of `record_pending` when `step.operation` is `Some`, and leave `beyond_edition::statement` over the model text as it is.
- [ ] **Step 4: Run.** `cargo test -p brightfield-shell --test operation_keep --test typed_sql_save --test typed_sql_palette --test typed_sql_nodes --test remove_step --test close_question_keeps_unsaved --test editor_snapshot`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Keep an operation's draft as an unsaved step, take it back from the spine, record it through arc on Save, and show its SQL in the Editor"`, then `--step comment-citations`.

### Task 6: Starting an operation, making it, keeping it: one entry point, with a sentence when it cannot start

**Files:**
- Create: `crates/brightfield-shell/src/operation_ui/open.rs`
- Create: `crates/brightfield-shell/src/window/operation_host.rs` (declared `mod operation_host;` in `window.rs`)
- Modify: `crates/brightfield-shell/src/window.rs` (`mod operation_host;` and the window's hooks into it)
- Modify: `crates/brightfield-shell/src/app.rs` (`ChartDoc` holds the operation being made: `making`, `making_mut`, `set_making`, `take_making`)
- Modify: `crates/brightfield-shell/src/data_grid.rs` (the grid draws an answer's page as the statement form's is, faded while `DraftReads::faded()`, and a preview's validity band from `DraftAnswer::missing`)
- Modify: `crates/brightfield-shell/src/operation_ui/filter_rows.rs`, `operation_ui/sort_rows.rs`
- Modify: `crates/brightfield-shell/src/overlays.rs` (`chart_offers` and `chart_palette_verbs` take each built verb where `saveable`; their docs)
- Modify: `crates/brightfield-shell/tests/overlay_wiring.rs` (one arm for every operation verb), `crates/brightfield-shell/tests/shelf_column_route.rs` (one arm for an operation verb on the Protocol palette)
- Create: `crates/brightfield-shell/tests/support/operations.rs`, `docs/operation-forms.md`
- Test: `crates/brightfield-shell/tests/operation_start.rs` (new)

The window's hooks into the host:
- an arm in `apply`, ahead of the `graph_on_canvas` arm, for a verb `operation_ui::by_verb` knows;
- `grid_cursor_keys` sends every built verb, filter and sort included, to `start_operation_verb`;
- `consume_token` gains `f`, `shift-u`, `shift-l`, `shift-f` and `&`. `&` is read from its `Text` event, since egui 0.35 has no key for it;
- descriptions are fetched for every built operation on the first frame with a runner;
- a key pass ahead of `grid_foot_keys` for Enter, Esc and Tab while an operation is being made;
- `save_everything` asks the host first;
- the status band reads `operation_status_entry`;
- the Editor tab shows the draft being made.

**Interfaces:**
- Consumes: Tasks 2, 4 and 5.
- Produces:

```rust
// operation_ui/open.rs
pub struct OperationContext { pub runner: bool, pub protocol: Option<PathBuf>, pub takes_sources: bool,
    pub in_front: Option<InFront>, pub tables: Vec<(String, Option<Vec<String>>)> }   // every table of the Protocol, recorded or kept, with its columns once known
pub struct InFront { pub table: String, pub loaded: bool, pub rows: bool, pub cell: bool, pub view_unrecorded: bool,
    pub columns: Vec<String>, pub facts: Vec<ColumnFacts>, pub cursor_column: Option<String>, pub step_offer: Option<String> }
pub enum Availability { Available, Unavailable(String) }
pub fn availability(spec: &OperationUi, cx: &OperationContext, column: Option<&str>) -> Availability
// app.rs
pub struct Making { pub spec: &'static OperationUi, pub draft: Draft, pub start: Start, pub column: Option<String> }
// window/operation_host.rs
impl MeridianApp {
    pub(crate) fn operation_context(&self) -> OperationContext;
    pub(crate) fn start_operation(&mut self, ctx: &egui::Context, spec: &'static OperationUi, start: Start, column: Option<String>) -> bool;
    pub(crate) fn start_operation_verb(&mut self, ctx: &egui::Context, verb: &str) -> bool;   // keys, palettes, a menu
    pub fn operation_said(&self) -> Option<&str>;
    pub fn making(&self) -> Option<&Making>;
    #[doc(hidden)] pub fn start_operation_spec(&mut self, ctx: &egui::Context, spec: &'static OperationUi, start: Start, column: Option<&str>) -> bool;
}
pub const OPERATION_STATUS_ID: &str = "operation";
```

**`start_operation`** works in order, saying the first that fails:
1. With no runner, it says "{command} records through arc, and this window was started without it."
2. It checks the description (`Descriptions::get`): pending says "Reading arc's description of {long name}…", and failed says "No description of {long name}: {why}".
3. It checks `availability`, need by need in the order the spec lists them, with the sentence in the slice 1 table above. `{command}` is `operation_ui::label`.
4. `begin` makes the draft. An `Err` is said.
5. For a `FootField` start, it calls today's `ask_where_at_cursor` or `ask_order_at_cursor` with the cursor on a cell, else `foot_offer(field, "")`, then `open_rail_on_operation`.
6. Otherwise it sets `Making`, starts `DraftReads` and sends the first edit. `Keeps::AtOnce` keeps on the first answer that is current with no refusal, and a dedupe that keeps every row is not kept, saying the table has no duplicate rows. A `field` takes the keys in place (Task 7), and a `form` opens the Operation section on it (Task 7).

A sentence stays on the status band until the next key or pointer press.

**While an operation is being made:**
- Each frame, the draft's request goes to `DraftReads::edit` when it changed. The current or shown answer's page goes to the grid, labelled as a preview, faded while `faded()`.
- The Editor tab shows `model`, live.
- The status band holds the refusal in the warning tone; else arc's notes; else `unread`; else `"<long name> on <table> · <rows_in> → <rows> rows"`.
- **Enter** keeps when `missing()` is `None` and `current()` holds arc's model without an engine refusal. Keeping calls `keep_draft` with `spec.name`'s name for the draft (or `replace_draft` when the draft corrects the kept one in front), clears `Making` and restores the rail.
- **Esc** clears `Making`, keeps nothing and gives the grid back.
- **⌘S** keeps first, if Enter would. Otherwise it saves the rest and leaves the draft open, saying why.
- A column given to a value is spelled for that value's shape. A `Column` or `Columns` value takes the bare name; a `Sql` value takes `quote(name)`, which is `grid_foot::column_sql` (made `pub(crate)`): bare when plain, quoted otherwise.

**Correcting a kept draft.** Starting an operation whose table in front is a kept draft of the same operation begins from that draft's request, not from the defaults, and Enter replaces it in place.

**`filter-rows` and `sort-rows`:**

```rust
OperationUi { long_name: "filter-rows", verb: "filter-rows-at-cursor", stage: Stage::Shape, icon: "filter",
    starts: &[Start::FootField(FootField::Where)], menu_words: "Filter rows",
    needs: &[Need::ProtocolSource, Need::GridCell, Need::LoadedTable],
    begin: |b| Ok(Draft::empty("filter-rows")), keeps: Keeps::OnEnter, field: None, form: None,
    name: NameRule { word: "filtered", from: By::Value("where"), with_by: true } }   // recording keeps record_view's own naming
// sort-rows: verb "sort-rows-at-cursor", icon "arrows-sort", FootField(OrderBy), menu_words "Sort rows", name word "sorted"
```

**The harness**, `tests/support/operations.rs`:

```rust
pub struct OpWindow { pub app: MeridianApp, pub ctx: egui::Context /* and its temp folder */ }
impl OpWindow {
    pub fn on_copy_of(fixture: &str) -> Self;                          // copies tests/data/<fixture>, opens it with the suite's runner
    pub fn with_labels(fixture: &str, labels: &[(&str, &str)]) -> Self; // nominates FineType labels as tests/typed_formats_grid.rs's fixture does
    pub fn reopen(&self) -> Self;
    pub fn cursor_on(&mut self, column: &str);                        // the grid focused, the cursor on the first row of `column`
    pub fn press(&mut self, token: &str);                             // a registry token: "-", "shift-r", "&", "cmd-s", …
    pub fn palette(&mut self, verb: &str);
    pub fn type_text(&mut self, text: &str); pub fn key(&mut self, key: egui::Key);
    pub fn click_text(&mut self, text: &str);                         // clicks the shape drawn with that text
    pub fn settle_reads(&mut self);                                   // frames until DraftReads answers the request as it stands, at most 30 s
    pub fn said(&self) -> Option<String>; pub fn making(&self) -> Option<Draft>;
    pub fn grid_columns(&self) -> Vec<String>; pub fn grid_cell(&self, row: usize, column: &str) -> Option<String>;
    pub fn grid_faded(&self) -> bool; pub fn preview_missing(&self, column: &str) -> Option<u64>;
    pub fn editor_text(&self) -> String;
    pub fn kept(&self) -> Vec<TypedStep>; pub fn save(&mut self);
    pub fn manifest_steps(&self) -> Vec<String>; pub fn model_of(&self, step: &str) -> String;
    pub fn show_grid_of(&mut self, step: &str); pub fn step_ran(&self, step: &str) -> bool;
}
```

Task 7 adds the header and Operation-section helpers.

- [ ] **Step 1: Write the failing tests** in `operation_start.rs`, on a copy of the housing sample. `HIDE_LIKE` (`Keeps::AtOnce`, value `column` from `begin.column`) and `RENAME_LIKE` (`Keeps::OnEnter`, values `column` and `to`) are specs local to the test, started with `start_operation_spec`.

```rust
#[test] fn the_palette_opens_filter_rows_in_the_where_field() { /* cursor on a cell; Space, "filter-rows", Enter -> foot_asking() is Where */ }
#[test] fn the_palette_opens_sort_rows_in_the_order_by_field() {}
#[test] fn a_key_off_a_cell_says_why_instead_of_nothing() {
    // the grid in its columns layout, press w -> said() == Some("Filter rows needs the grid's cursor on a cell.")
}
#[test] fn filter_rows_on_a_steps_grid_says_the_steps_offer() { /* a recorded filter step not run; its grid; palette filter-rows -> its offer() */ }
#[test] fn an_operation_over_an_unrecorded_view_says_save_or_clear_it_first() {
    // availability(&HIDE_LIKE, &cx, Some("house_age")) with in_front.view_unrecorded ==
    // Unavailable("Hide column reads the table as recorded: save or clear the where and order by first.")
}
#[test] fn availability_names_the_first_need_it_lacks() { /* one context per Need left unmet, each its sentence; a context meeting all: Available */ }
#[test] fn an_at_once_operation_keeps_on_its_first_good_answer() { /* HIDE_LIKE on house_age -> after settle, kept()[0].operation is Some */ }
#[test] fn a_column_is_given_bare_to_a_name_and_quoted_to_sql() {
    // with_nulls.csv: RENAME_LIKE on `median income` -> column value `median income`;
    // a spec whose `expression` value takes the column -> `"median income"`
}
#[test] fn enter_keeps_and_esc_keeps_nothing() {}
#[test] fn enter_with_a_required_value_empty_says_what_is_missing() { /* RENAME_LIKE with `to` empty -> said() == Some("rename-column needs to") */ }
#[test] fn cmd_s_keeps_an_admitted_draft_first() {}
#[test] fn cmd_s_saves_the_rest_and_leaves_an_unfinished_draft_open() {}
#[test] fn the_editor_follows_the_draft_as_it_is_made() { /* RENAME_LIKE typing "inc" then "income": editor_text() holds `TO income` */ }
#[test] fn starting_over_a_kept_draft_of_the_same_operation_corrects_it() { /* keep, start again on its grid, change, Enter -> one kept step */ }
#[test] fn the_chart_palette_lists_built_operations_only_where_a_protocol_is_saved() {}
```

The arm in `overlay_wiring.rs`'s sweep, for `v if operation_ui::by_verb(v).is_some()`, opens its own copy of the housing sample with the cursor on a cell and confirms `v`. It then asserts that the foot asks, or that `making()` is `Some`, or that a draft was kept, or that `operation_said()` is `Some`.

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test operation_start`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** `open.rs`, the host and the window's hooks, then the grid's faded preview and validity band, the two specs, and the palette. Write `docs/operation-forms.md`. Cover the owner's ruling; the shared parts; the flow from a start to ⌘S; the calls above, each marked as the owner's to overturn; what arc lacks; and how to add an operation: fill `operation_ui/<op>.rs`, flip its `*_BUILT`, write `tests/operation_<op>.rs` from the harness.
- [ ] **Step 4: Run.** `cargo test -p brightfield-shell --test operation_start --test overlay_wiring --test shelf_column_route --test operation_from_the_cursor --test grid_foot --test operation_panel --test operation_registry`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Start an operation from its key or the palette, preview it as it is made, keep it on Enter, or say what it needs"`, then `--step comment-citations`.

### Task 7: The column header and the Operation section as places to act

**Files:**
- Modify: `crates/brightfield-shell/src/column_header.rs` (each cell records the rects of its parts: the name, the type (glyph and storage label), the missing segment of the validity band; a trailing new-column cell; and, while a field is open in place, the field drawn over its part, with any suggestions listed under it as `list_row`s)
- Modify: `crates/brightfield-shell/src/data_grid.rs` (the plain header, which previews draw, records the name part and the new-column cell the same way; the header senses a click, a double-click and a secondary click on each part and reports it)
- Modify: `crates/brightfield-shell/src/operation_panel.rs` (the Operation section draws the form of the operation being made: `spec.form`, given a `FormCx`; filter and sort keep today's explanation; module doc)
- Modify: `crates/brightfield-shell/src/window/operation_host.rs` (a header part's gesture starts the built operation whose `starts` names it; a secondary click opens the header's menu; an in-place field's keys: typing, Tab between `InPlace::keys`, Up and Down through suggestions, Enter, Esc)
- Modify: `crates/brightfield-shell/tests/support/operations.rs` (the header and section helpers)
- Test: `crates/brightfield-shell/tests/operation_header.rs` (new)

**Interfaces:**
- Consumes: Task 6.
- Produces:

```rust
// column_header.rs / data_grid.rs
pub struct HeaderHit { pub column: Option<String> /* None for the new-column cell */, pub part: HeaderPart, pub gesture: Gesture }
pub enum Gesture { Click, DoubleClick, Secondary }
impl ChartDoc { pub fn take_header_hit(&mut self) -> Option<HeaderHit> }
// harness additions
impl OpWindow {
    pub fn header(&mut self, column: &str, part: HeaderPart, gesture: Gesture);
    pub fn header_new_column(&mut self);
    pub fn header_menu(&mut self, column: &str, words: &str);   // secondary click, then the row with those words
    pub fn field_text(&self) -> Option<String>; pub fn suggestions(&self) -> Vec<String>;
    pub fn section_text(&self) -> Vec<String>;                   // the texts the Operation section drew
}
```

Gestures map to operations by their `starts`:
- a double-click on `Name` goes to the one built operation whose starts hold `Header(Name)`;
- a click on `Type`, or on `Missing`, goes to the one whose starts hold that part;
- a click on the new-column cell goes to the one holding `NewColumn`;
- a secondary click anywhere on a cell opens a menu listing every built operation whose starts hold a `Header` part, each as its `menu_words` with its key, unavailable ones greyed with their sentence.

A part no built operation starts from senses nothing, so no header gesture does nothing. While a field is open in place, the cell draws it over its part and the rest of the header stays as it is. The new-column cell draws only while no field is open, and only when an operation starting from it is built.

- [ ] **Step 1: Write the failing tests** in `operation_header.rs`, with specs local to the test that start from each part:

```rust
#[test] fn a_double_click_on_a_name_starts_the_operation_that_starts_there() {}
#[test] fn a_click_on_the_type_and_on_the_missing_count_start_theirs() {}
#[test] fn the_header_menu_lists_each_operation_that_starts_from_a_header_with_its_key() {}
#[test] fn an_operation_the_column_cannot_take_is_greyed_in_the_menu_with_its_sentence() {}
#[test] fn a_part_no_operation_starts_from_senses_nothing() {}
#[test] fn a_field_in_place_takes_typing_tab_and_its_suggestions() {}
#[test] fn the_new_column_cell_shows_only_when_an_operation_starts_there() {}
#[test] fn the_operation_section_draws_the_form_of_the_operation_being_made() {}
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test operation_header`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** the parts' rects in both headers, the gestures, the menu (egui's context menu over `list_row`s), the field in place, and the section's host.
- [ ] **Step 4: Run.** `cargo test -p brightfield-shell --test operation_header --test operation_start --test column_header_type --test data_grid --test token_discipline --test colour_literals`. Expected: PASS. `data_grid` is a serial target; run it alone.
- [ ] **Step 5: Commit.** `git commit -m "Let an operation start from a part of a column's header, ask in place there, or draw its form in the Operation section"`, then `--step comment-citations`.

### Task 8: The spine picks a table for an operation

> **Built.** `docs/operation-forms.md`, under "The spine's pick", is the reference for Tasks 18 and 19 where it differs from the interfaces below: the candidates stay in the spine's order, `SpinePick::add` and `put_cursor_on` answer whether they acted, `would_take` reads what a take would hand over, and `Begin::model` is filled for an operation that started from the spine.

**Files:**
- Create: `crates/brightfield-shell/src/spine_pick.rs`
- Modify: `crates/brightfield-shell/src/lib.rs` (`pub mod spine_pick;`)
- Modify: `crates/brightfield-keys/src/registry.rs` (`BindingContext::SpinePick`; five verbs; `longname_snapshot_is_stable`'s list; a context test as the Grid's is)
- Modify: `crates/brightfield-keys/src/dispatch.rs` (`DispatchContext::SpinePickFocused`; the matrix and its tests)
- Modify: `crates/brightfield-shell/src/protocol.rs` (the spine's rows while it picks)
- Modify: `crates/brightfield-shell/src/window/operation_host.rs` (a `Start::Spine` operation: the spine picks first, then `begin` runs with the tables taken)
- Modify: `crates/brightfield-shell/src/window.rs` (the SpinePick context's bindings resolve while the spine picks; the model pane's spine picks as the generated Protocol's does)
- Test: `crates/brightfield-shell/tests/spine_pick.rs` (new)

**Interfaces:**
- Consumes: Task 6's `start_operation_spec`, `Begin::taken`; Task 4's `PreviewOp::ReadFile`.
- Produces:

```rust
pub struct SpinePick { /* role, list, in_front, candidates, cursor, added */ }
pub struct Candidate { pub table: String, pub columns: Option<Vec<String>>, pub shares: Option<String>, pub related: bool,
    pub source: CandidateSource }
pub enum CandidateSource { Step, ModelTable { grid_copy: PathBuf } }   // a step of the chain, or an own-files table read from its grid copy
impl SpinePick {
    pub fn new(role: &'static str, list: bool, in_front: &str, cursor_column: Option<&str>, candidates: Vec<Candidate>) -> Self;
    pub fn candidates(&self) -> &[Candidate]; pub fn cursor(&self) -> usize; pub fn added(&self) -> &[String];
    pub fn next(&mut self); pub fn previous(&mut self); pub fn add(&mut self);
    pub fn take(&mut self) -> Vec<String>;         // the added tables in order, or the one under the cursor
    pub fn set_columns(&mut self, table: &str, columns: Vec<String>);
}
```

The verbs, all `CommandTier::View`, `Drives::Navigation`, in `BindingContext::SpinePick`: `pick-next-table` (`j`, `down`), `pick-previous-table` (`k`, `up`), `add-table-to-pick` (`space`), `take-picked-table` (`enter`), `back-out-of-pick` (`escape`).

**The candidates:**
- In a Protocol made from a data file: each table its spine lists other than the one in front.
- While the grid holds a table of an own-files Protocol: the model's other tables. Each is read from its grid copy (`own_files::grid_copy`), which the warm preview loads as a `PreviewOp::ReadFile` root.
- `related` is set where the Protocol's model (`model.ossie.yaml`) holds a relationship between the candidate and the table in front.
- The cursor starts on the first related candidate, else the first that shares the cursor's column, else the first.

**How it looks:**
- The rail has the focus ring, and the table in front reads *in front*.
- Rows that are not tables are in the muted ink.
- Each other table trails `related`, or `shares <column>`, once its columns are known. Its columns are read on a worker, so the spine never waits on DuckDB.
- The cursor is a selected row, and an added table is marked selected.
- The status band names the five keys.

Take hands the tables and their columns to `begin` through `Begin::taken`. Esc gives the keyboard back to the grid and keeps nothing.

- [x] **Step 1: Write the failing tests** in `spine_pick.rs`. The spec local to the test is `Start::Spine { role: "other" }`, single and list. The folder is a copy of the housing sample with a step `by_age` (summarise-rows by `house_age`) and a step `top` (limit-rows 10), both recorded through `record_step`.

```rust
#[test] fn the_spine_marks_the_table_in_front_and_starts_on_the_best_candidate() {}
#[test] fn each_table_says_whether_it_shares_the_cursors_column() { /* cursor on house_age: by_age and top trail "shares house_age" */ }
#[test] fn a_table_the_model_relates_comes_first_and_says_so() {
    // a Model built in the test relating the table in front to `top`: top is first and trails "related"
}
#[test] fn enter_takes_the_table_and_begins_the_operation_with_it() {}
#[test] fn space_adds_to_a_list_and_enter_takes_them_in_order() {}
#[test] fn esc_goes_back_to_the_grid_and_keeps_nothing() {}
```

And in `registry.rs`: `the_spine_pick_context_moves_adds_takes_and_backs_out`.
- [x] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-keys` then `cargo test -p brightfield-shell --test spine_pick`. Expected: FAIL.
- [x] **Step 3: Implement** the keys, then `spine_pick.rs`, the spine's rows and the host's spine start.
- [x] **Step 4: Run.** `cargo test -p brightfield-keys` then `cargo test -p brightfield-shell --test spine_pick --test navigator_spine --test operation_start`. Expected: PASS.
- [x] **Step 5: Commit.** `git commit -m "Pick the table an operation reads in the spine, one or a list, related tables first"`, then `--step comment-citations`.

### Every operation task (9 to 19) follows these steps

Each operation task touches three things, and nothing else unless the task says so:
- its own `crates/brightfield-shell/src/operation_ui/<op>.rs`;
- its own `<OP>_BUILT` line in `crates/brightfield-keys/src/operations.rs`;
- a new `crates/brightfield-shell/tests/operation_<op>.rs`, which opens with `#[path = "support/operations.rs"] mod operations;`.

The key, the help and the scores are Task 2's, already written. In each spec, `long_name` and `verb` are the operation's long name.

- [ ] **Step 1: Write the failing tests** the task names, with the values it gives.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test operation_<op>`. Expected: FAIL, the key, header or palette row starts nothing.
- [ ] **Step 3: Fill the slot.** `ui()` returns `Some(&SPEC)`. Write `begin`, and the field or form, in the module. The module doc says how the operation starts, what it asks, where its defaults come from, and any call the task made.
- [ ] **Step 4: Flip** `<OP>_BUILT` to `true`.
- [ ] **Step 5: Run.** `cargo test -p brightfield-shell --test operation_<op> --test operation_registry --test overlay_wiring` then `cargo test -p brightfield-keys`. Expected: PASS.
- [ ] **Step 6: Commit.** `git commit -m "<Label> starts from <where> and records its step through arc"`, then `--step comment-citations`.

Every operation's tests include these five, named for the operation:
- `<op>_starts_from_<place>_with_its_defaults`: the draft's values, or `field_text()`, are the defaults given.
- `<op>_previews_arcs_model_as_it_is_edited`: after each edit given, `grid_columns()` and the cells given, and `editor_text()` holds arc's model.
- `<op>_keeps_a_step_named_by_its_effect_and_save_records_arcs_model`: `kept()[0].name` is the given name. After `save()`, the manifest holds it and `model_of(name)` equals the kept step's `statement`, taken before the save.
- `<op>_reopens_as_a_step_not_run_that_previews_the_same`: in `reopen()`, `step_ran(name)` is false and the step's grid has the same columns.
- `<op>_says_the_refusal_and_keeps_nothing`: with the input given, `said()` holds the fragment given, `grid_faded()` is true while earlier rows were good, Enter keeps nothing, and the folder is unchanged.

The housing sample (`tests/data/california_housing_sample.csv`) loads as the table `california_housing_sample`, 240 rows of `median_income, house_age, avg_rooms, avg_bedrooms, population, avg_occupancy, latitude, longitude, median_house_value`. `tests/data/with_nulls.csv` loads as `with_nulls`, with the columns `city, house_age, order, median income`, and Berkeley's `house_age` is empty. Unless a task says otherwise, `needs` is `&[Need::ProtocolSource, Need::Rows, Need::Column, Need::ViewRecorded]` and `stage` is `Stage::Shape`.

### Task 9: `hide-column`: a direct action on the column

- **Starts:** the header's menu ("Hide column") or `-` on a cell. `starts: &[Start::Cell, Start::Header(HeaderPart::Menu)]`.
- **Asks:** nothing. `Keeps::AtOnce`, with no field and no form.
- **Defaults:** `begin` sets `column` to `begin.column`, the bare name.
- **Preview:** the column leaves the header as soon as arc answers. The draft's table comes to the front, and the status band says "Hid house_age · <remove-step key> takes it back".
- **Spec:** `icon: "eye-off"`, `name: NameRule { word: "without", from: By::Value("column"), with_by: false }`.
- **Tests:**
  - Housing, `-` on `house_age`: kept `california_housing_sample_without_house_age`, eight columns, and the model holds `EXCLUDE (house_age)`.
  - `hide_column_from_the_header_menu_keeps_the_same_step`.
  - `hide_column_quotes_a_column_named_order`: on `with_nulls`, the model holds `EXCLUDE ("order")`.
  - Refusal, on a one-column CSV the test writes: hiding its only column gives DuckDB's reason, and nothing is kept.

### Task 10: `rename-column`: edited in place in the header

- **Starts:** a double-click on the header's name, `shift-r` on a cell, or the menu ("Rename column"). `starts: &[Start::Cell, Start::Header(HeaderPart::Name), Start::Header(HeaderPart::Menu)]`.
- **Asks:** the new name, in place: `field: Some(InPlace { part: HeaderPart::Name, keys: &["to"], suggestions: |_, _| vec![] })`, with `Keeps::OnEnter`.
- **Defaults:** `column` is the column's name, and `to` is the same name, selected whole.
- **Preview:** the header shows the text as typed, and each edit runs arc's model. A name the table already has shows DuckDB's reason, with the rows faded.
- **Spec:** `icon: "cursor-text"`, `name: NameRule { word: "renamed", from: By::Value("column"), with_by: false }`.
- **Tests:**
  - `with_nulls`, a double-click on `median income`: `field_text()` is `median income`. Type `income`: the columns are `city, house_age, order, income`. Enter keeps `with_nulls_renamed_median_income`, and the model holds `RENAME COLUMN "median income" TO income`.
  - Refusals: `city` gives DuckDB's words; a single space gives arc's "the column `to` of `rename-column` is empty or blank".

### Task 11: `derive-column`: an expression typed in a new header, its values filling live

- **Starts:** the header's new-column cell, `=` on a cell, or the menu ("Add a column from this one"). `starts: &[Start::Cell, Start::Header(HeaderPart::NewColumn), Start::Header(HeaderPart::Menu)]`.
- **Asks:** the expression, then the name, in place in a new header cell at the table's end: `field: Some(InPlace { part: HeaderPart::NewColumn, keys: &["expression", "column"], suggestions: |b, _| b.columns.iter().map(|c| (b.quote)(c)).collect() })`, with `Keeps::OnEnter`. The suggestions complete a column as SQL spells it.
- **Defaults:** `expression` is the cursor's or header's column as SQL, selected whole, or empty from the new-column cell. `column` follows the expression until it is typed into: the expression's letters, digits and `_`, runs of anything else made one `_`, trimmed and cut at 30 characters, and made free of the table's columns with `_2`.
- **Preview:** the new column's values fill under the field at each keystroke.
- **Spec:** `icon: "math-function"`, `name: NameRule { word: "with", from: By::Value("column"), with_by: false }`.
- **Tests:**
  - Housing, `=` on `median_income`: `field_text()` is `median_income`. Type `median_income * 2`: the last column is `median_income_2`, and row 0 reads `1.8604`. Tab, then type `double_income`, and the column is renamed live. Enter keeps `california_housing_sample_with_double_income`.
  - Refusal: `sum(` gives arc's "the expression `expression` of `derive-column` is not one SQL expression", with the rows faded.

### Task 12: `cast-column`: from the type FineType reads

- **Starts:** a click on the header's type (the FineType glyph and storage label), `shift-t` on a cell, or the menu ("Change type"). `starts: &[Start::Cell, Start::Header(HeaderPart::Type), Start::Header(HeaderPart::Menu)]`.
- **Asks:** the type, in place over the storage label: `field: Some(InPlace { part: HeaderPart::Type, keys: &["type"], suggestions })`, with `Keeps::OnEnter`.
- **Defaults:** `type` is `transforms.duckdb_type(label)` for the column's FineType label when that differs from its storage type, else empty. `suggestions` lists that type first, then `VARCHAR`, `BIGINT`, `DOUBLE`, `DECIMAL(18, 3)`, `DATE`, `TIMESTAMP` and `BOOLEAN`, less the storage type, filtered by what is typed.
- **Preview:** the column's cells re-read as the type at each keystroke. A value that does not convert gives DuckDB's reason, since arc's cast is a `CAST`, not a `TRY_CAST`, and the run would fail too.
- **Spec:** `icon: "transform"`, `name: NameRule { word: "cast", from: By::Value("column"), with_by: false }`.
- **Tests:**
  - Housing, with `house_age` labelled `representation.numeric.decimal_number` through `with_labels`. A click on its type: `field_text()` is `DOUBLE`, and `suggestions()[0]` is `DOUBLE`. Enter keeps `california_housing_sample_cast_house_age`. A first page read through the kept step's chain has `house_age` as `Float64`.
  - `cast_column_on_a_column_fine_type_agrees_with_opens_empty`.
  - Refusal: `DECIMAL(10` gives arc's "the type `type` of `cast-column` is not one SQL type".

### Task 13: `fill-nulls`: from the column's missing count, with a suggested fill

- **Starts:** a click on the missing segment of the header's validity band, `f` on a cell, or the menu ("Fill missing values"). `starts: &[Start::Cell, Start::Header(HeaderPart::Missing), Start::Header(HeaderPart::Menu)]`.
- **Needs:** the default, and `Need::ColumnHasNulls`.
- **Asks:** the value, in place under the validity band: `field: Some(InPlace { part: HeaderPart::Missing, keys: &["value"], suggestions })`, with `Keeps::OnEnter`.
- **Defaults:** `value` follows the storage type: `0` for a number, `'unknown'` for text, `false` for a boolean, and empty otherwise. `suggestions` adds the column's median (`ColumnFacts::moments`) for a number.
- **Preview:** the preview's validity band for the column empties as the value is typed (`preview_missing(column)` reaches `0`), and the cells fill.
- **Spec:** `icon: "bucket-droplet"`, `name: NameRule { word: "filled", from: By::Value("column"), with_by: false }`.
- **Tests:**
  - `with_nulls`, a click on `house_age`'s missing segment: `field_text()` is `0`, and after settling `preview_missing("house_age") == Some(0)`, with Berkeley reading `0`. Enter keeps `with_nulls_filled_house_age`.
  - `fill_nulls_is_greyed_on_a_column_with_no_missing_values`: `f` on `city` says "Fill nulls needs a column with missing values."
  - Refusals: `'x'` gives DuckDB's conversion error; `1;` gives arc's "holds a `;` outside a string or a comment".

### Task 14: `dedupe-rows`: one action with a sensible default

- **Starts:** the header's menu ("Remove duplicate rows"), `shift-u` on a cell, or the palette. `starts: &[Start::Cell, Start::Header(HeaderPart::Menu)]`.
- **Needs:** `&[Need::ProtocolSource, Need::Rows, Need::ViewRecorded]`.
- **Asks:** nothing at first. `Keeps::AtOnce` with no values, arc's `SELECT DISTINCT *`. A table with no duplicate rows is not kept; the status band says "<table> has no duplicate rows".
- **The form:** the Operation section then shows the form over the kept draft, which refines it in place. "One row per" takes the key, a column list from the table's columns. "The first by" takes the order, which is the key's first column, descending, until it is changed, since arc needs both together.
- **Spec:** `icon: "copy-off"`, `name: NameRule { word: "deduped", from: By::Value("key"), with_by: true }`.
- **Tests:**
  - On a CSV the test writes, `with_nulls`' rows with Alameda's twice, `shift-u` keeps `with_nulls_deduped`, 3 rows from 4, and the model holds `SELECT DISTINCT *`.
  - In the form, set the key to `city`, ordered by `house_age desc`: the kept draft is replaced in place, its model holds `QUALIFY`, and it keeps the name it was kept under, since a name is fixed when a draft is kept.
  - `dedupe_rows_on_a_table_without_duplicates_says_so_and_keeps_nothing`, on the housing sample.
  - Refusal: the order emptied with the key kept gives arc's "the argument `key` of `dedupe-rows` needs `order_by` as well".

### Task 15: `limit-rows`: the top rows by a column

- **Starts:** the header's menu ("Top rows by this column") or `shift-l` on a cell. `starts: &[Start::Cell, Start::Header(HeaderPart::Menu)]`.
- **Asks:** nothing, at first. `Keeps::OnEnter`: the preview is already the answer, and Enter keeps it.
- **Defaults:** `count` is `10`, and `order_by` is the column as SQL with `desc`.
- **The form:** the Operation section holds the count, a number field where Up and Down change it by one and Shift by ten; the direction, as two words, *largest first* and *smallest first*; and the order as text, for an order over several columns.
- **Preview:** each change re-reads the rows.
- **Spec:** `icon: "list-numbers"`, `name: NameRule { word: "first", from: By::Value("count"), with_by: false }`.
- **Tests:**
  - Housing, `shift-l` on `median_house_value`: 10 rows, and row 0 holds the file's greatest value. Up in the count field gives 11 rows. *Smallest first* gives row 0 the least value. Enter keeps `california_housing_sample_first_11`.
  - Refusals: `ten` gives arc's `is an integer, not "ten"`; `-1` gives arc's "has a minimum of 0, not -1".

### Task 16: `summarise-rows`: group-by columns, then the values

- **Starts:** the header's menu ("Summarise by this column") or `shift-f` on a cell. `starts: &[Start::Cell, Start::Header(HeaderPart::Menu)]`.
- **Asks:** nothing to begin, as VisiData's frequency table, with `Keeps::OnEnter`.
- **Defaults:** `group_by` is the column as SQL, and `aggregates` is `count(*) AS n`.
- **The form:** the Operation section has two parts. "Group by" lists the chosen columns as chips, and *add* lists the table's columns: those whose FineType label or tile reads as a category or a date first, by `ColumnFacts::tile`. "Values" holds `count(*) AS n` and a row per number column added, with its function chip: `avg` for a decimal label, `sum` for an integer one, `count(DISTINCT …)` for anything else, each `AS <fn>_<column>`. Both lists are also shown, and editable, as the select lists arc records.
- **Preview:** each change re-reads the summary, which holds every row of the table, since the warm connection holds the whole table.
- **Spec:** `icon: "sum"`, `name: NameRule { word: "summarised", from: By::Value("group_by"), with_by: true }`.
- **Tests:**
  - Housing, `shift-f` on `house_age`: the columns are `house_age, n`, and `n` sums to 240 over the kept step's rows once counted. Add the value `median_income`: a column `avg_median_income`. Add the group `latitude`: the name stays `california_housing_sample_summarised_by_house_age`. Enter keeps.
  - Refusal: `2` typed into the group-by text gives arc's "holds a constant".

### Task 17: `read-file`: through the sources browser

**Files:** `crates/brightfield-shell/src/operation_ui/read_file.rs`; `crates/brightfield-shell/tests/operation_read_file.rs` (new). No `*_BUILT` line.

- **Starts:** the sources browser, `open-sources-browser`'s key, palette row and, later, its ribbon command under Bring in. `starts: &[Start::SourcesBrowser]`.
- **Asks:** what the browser asks, which already proposes a folder's joins through dovetail.
- **Spec:** `verb: "open-sources-browser"`, `stage: Stage::BringIn`, `icon: "file-import"`, `needs: &[Need::TakesSources]`, `begin` unused (`Err` for any start but the browser's), `keeps: Keeps::OnEnter`, no field or form. Its `name` is unused, since the browser names its tables.
- **Tests:**
  - `the_bring_in_command_opens_the_sources_browser`.
  - `a_protocol_brightfield_did_not_make_says_sources_are_not_added_to_it`: slice 1's sentence.
  - `an_agents_read_file_step_opens_and_previews_its_file`: record `read-file` for `with_nulls.csv`, copied beside the housing sample, through `record_step`, then reopen. The step's grid previews 3 rows and 4 columns through `PreviewOp::ReadFile`.
  - `an_agents_read_file_of_a_url_says_it_previews_local_files_only`.

### Task 18: `join-tables`: the second table from the spine, keys proposed

**Waits on** Task 8.

- **Starts:** `&` on a cell, or the header's menu ("Join on this column"). `starts: &[Start::Spine { role: "other" }, Start::Cell, Start::Header(HeaderPart::Menu)]`. The spine picks first, related tables first.
- **Needs:** `&[Need::ProtocolSource, Need::Rows, Need::ViewRecorded, Need::AnotherTable]`.
- **Defaults:** `begin`, once the spine has given the other table, proposes the keys in this order:
  1. The Protocol's model: a relationship between the two tables in `begin.model` gives its columns. The same names on both sides become `using`; otherwise `condition` is `<table>.<a> = <other>.<b>`, each column as SQL spells it, and for several columns joined by `AND`.
  2. The cursor's or header's column, when the other table holds it.
  3. The first column both tables hold.

  `kind` is `left`, arc's default.
- **The form:** the Operation section holds the kind, as arc's seven words; the keys, as a column list when `using` and as text when `condition`, either replacing the other; the prefix, `<other>_` until typed; and a line reading as SQL, `left join by_age using (house_age)`.
- **Preview:** each change re-reads, and the status band says how many columns the join adds. `Keeps::OnEnter`.
- **Spec:** `stage: Stage::Model`, `icon: "arrows-join"`, `name: NameRule { word: "joined", from: By::Value("other"), with_by: false }`.
- **Tests:**
  - On a copy of the housing sample with `by_age` recorded (summarise-rows by `house_age`), then reopened: `&` on `house_age`, then Enter on `by_age`. `using` is `house_age`, the preview adds `by_age_n`, and Enter keeps `california_housing_sample_joined_by_age`, whose model holds `LEFT JOIN by_age USING (house_age)`.
  - `join_keys_come_from_the_models_relationship`: `begin` over a `Model` built in the test relating `customer_id` to `id` gives `condition` `orders.customer_id = customers.id`.
  - `join_tables_with_no_other_table_says_what_it_needs`: "Join tables needs a second table in the Protocol."
  - Refusal: kind `cross` with the key kept gives arc's "exactly one of `using` or `condition` is required for every kind but `cross`".

### Task 19: `append-rows`: columns matched by name

**Waits on** Task 8.

- **Starts:** `shift-a` on a cell, or the header's menu ("Append rows of other tables"). `starts: &[Start::Spine { role: "other" }, Start::Cell, Start::Header(HeaderPart::Menu)]`. The spine picks a list, with Space adding; a table sharing no column name is dimmed and reads *shares no column*.
- **Needs:** `&[Need::ProtocolSource, Need::Rows, Need::ViewRecorded, Need::AnotherTableSharingAColumn]`.
- **Asks:** nothing more, with `Keeps::OnEnter`.
- **The form:** the Operation section shows how arc's `UNION ALL BY NAME` matches the columns. A row per column name, in the order the result takes them (the table in front's first), and a column per table, each cell *✓* or *null*, so the analyst sees what each table will lack.
- **Preview:** the appended rows.
- **Spec:** `stage: Stage::Model`, `icon: "row-insert-bottom"`, `name: NameRule { word: "appended", from: By::Nothing, with_by: false }`.
- **Tests:**
  - On a copy of the housing sample with `top` (limit-rows 10) and `without_age` (hide-column `house_age`) recorded, then reopened: `shift-a`, Space on `top`, Space on `without_age`, Enter. `section_text()` holds `house_age` with *null* under `without_age`. Enter keeps `california_housing_sample_appended`, 490 rows once counted, and its model holds `UNION ALL BY NAME` twice, `top` first.
  - `append_rows_needs_a_table_sharing_a_column`: a Protocol whose only other table shares no column name gives "Append rows needs a second table with matching columns." Build it with summarise-rows `count(*) AS n` and no group.

### Task 20: Availability through the registry's declarations, and the fold

**Waits on** every other task, and the chrome's slice 1 (availability declarations).

**Files:**
- Modify: `crates/brightfield-keys/src/operations.rs` (each entry's `needs` in slice 1's form; the operation verbs grouped by stage for slice 1's menus, if its menu model takes them so)
- Modify: slice 1's `Condition` and `Selected`, or their equivalents, gaining the needs the table under "What the operations expect" marks new, each with its sentence
- Modify: `crates/brightfield-shell/src/operation_ui/open.rs` (the facts `OperationContext` gathers fill slice 1's `Situation`; `availability` becomes a reading of `MeridianApp::availability_of`, and its own sentences go)
- Modify: `crates/brightfield-shell/src/window/operation_host.rs`, `crates/brightfield-shell/src/overlays.rs` (an unavailable operation's palette row and header-menu row are greyed with its sentence, as slice 1 greys the rest)
- Modify: `docs/operation-forms.md` (final)
- Delete: `docs/superpowers/plans/2026-11-22-operation-forms.md`
- Test: `crates/brightfield-shell/tests/operation_availability.rs` (new)

- [ ] **Step 1: Write the failing tests:**
  - `each_operations_declared_needs_match_what_arc_says_it_reads`: Task 2's check, now made over the registry's `needs`.
  - `an_operation_unavailable_here_is_greyed_in_the_palette_with_its_sentence`.
  - `its_key_its_menu_row_and_its_palette_row_say_the_same_sentence`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test operation_availability`. Expected: FAIL.
- [ ] **Step 3: Implement**, then delete the operations' own evaluator.
- [ ] **Step 4: Run.** `cargo test -p brightfield-keys`, then `cargo test -p brightfield-shell --test operation_availability --test availability --test operation_start --test operation_header --test operation_registry --test overlay_wiring`. Expected: PASS.
- [ ] **Step 5: Fold.** In `docs/operation-forms.md`, make sure each operation's rulings live in its own module doc, and that the calls above say whether the owner kept or overturned each. Delete this plan. The chrome spec stays for its other slices.
- [ ] **Step 6: Commit.** `git commit -m "Declare each operation's needs in the key registry, so the palette, the menus, the header and the ribbon grey it with one sentence"`, then `--step comment-citations`.

**What arc lacks**, for `docs/operation-forms.md` and as arcform's work:
1. **A render that writes nothing, in-process.** Each edit records into a throwaway Protocol of stub tables through a child process, about 150 ms a call when this plan was written. Instant SQL answers a keystroke in under 10 ms. A library call that returns the model for a request, with arc's refusals and no folder, would close most of that gap and retire `draft_model`'s stubs.
2. **A record of a step's operation that a program can read.** A generated step carries only its model and a header arc calls a line "for a person reading the model and not a format a program reads back". So Brightfield reads the tables a step reads from that header, which is ambiguous for a name holding `, `, and correcting a recorded step from its form is out of reach.
3. **Which column values must already exist.** `x-kind: column` does not say whether a value must be a column of `table` (`hide-column`'s `column`) or is a new name (`rename-column`'s `to`, `derive-column`'s `column`), so each operation's `begin` says it.
4. **A list item holding a comma.** The command line splits a list on `,`, so a `using` column whose name holds one can be given through `arc mcp` alone.
5. **Rules the schema does not state.** `join-tables` takes exactly one of `using` or `condition`, which only arc's check enforces, so a form learns it from the refusal.
6. **No stage, and no other tool's word, in a description.** Brightfield holds the stages, and the palette's search by another tool's word waits on arc's description carrying those words.
