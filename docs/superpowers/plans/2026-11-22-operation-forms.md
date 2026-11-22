# The operation forms Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A form in Brightfield's window for each of arcform's 13 operations, built from arc's own description of it, aimed at the node in front, previewing the result there, and recording the step through arc on ⌘S.

**Architecture:** A form is the pattern the grid's foot row already sets for `filter-rows` and `sort-rows`: the foot row asks what the cursor does not supply, one value at a time, and the Operation panel in the Inspector rail shows every value the operation takes. Each operation has a slot in a forms registry (`crates/brightfield-shell/src/forms/`) and in the key registry (`crates/brightfield-keys/src/operations.rs`), and each slot is filled by its own task in its own files, so the per-operation tasks run side by side. While a form asks, arc records the draft into a throwaway Protocol of stub tables, and the model arc wrote runs in the preview's sandbox: the preview is arc's SQL, and arc's refusals arrive while the analyst is still asking. Enter keeps the draft as an unsaved step, beside the statements kept from the palette, and ⌘S records it with `arc operation record`.

**Tech Stack:** Rust; egui/eframe 0.35; DuckDB 2.0 through the bundled `duckdb` 1.20000.0; arc (arcform, rev-pinned) run as this binary's child; `meridian-design` and `meridian-egui`.

**Spec:** `docs/superpowers/specs/2026-11-21-brightfield-chrome-design.md`: read "The operation forms", "Sequencing", the availability bullets of "3. The ribbon's command row", and the Goals and Anti-goals. Earlier rulings this plan builds on are stated under Global Constraints in plain words.

## How the work fans out

| Phase | Tasks | Run side by side | Waits on |
|---|---|---|---|
| 1. The shared pattern, one branch and one session | 1 and 2, then 3, 4, 5, 6 in that order | 1 with 2 | nothing |
| 2. The fan-out, a branch and a session each | 7 (the spine picks a table); 8 to 15 (Shape: `hide-column`, `rename-column`, `derive-column`, `cast-column`, `fill-nulls`, `dedupe-rows`, `limit-rows`, `summarise-rows`) | all nine | phase 1 merged |
| 3. Model and Bring in | 16 (`join-tables`), 17 (`append-rows`), 18 (`read-file`) | all three, and any phase 2 task still open | 7 merged |
| 4. Closing | 19 (availability through the registry; the fold) | — | every other task, and the chrome's slice 1 |

**What keeps phase 2 and 3 apart.** Phase 1 creates every slot up front: a stub file per operation under `forms/`, the forms table that lists all 13, and a key-registry entry per operation behind its own `*_BUILT` constant with its key already chosen. A Shape task then touches exactly three files: its own `crates/brightfield-shell/src/forms/<op>.rs`, its own `*_BUILT` line in `crates/brightfield-keys/src/operations.rs` (each in a block of its own, so two flips never touch neighbouring lines), and its own new test file. Tasks 16 and 17 add nothing to that. Task 7 owns the spine's picking mode; Task 18 owns the spine's foot row and a second root in the preview, and waits on Task 7 because both draw in `protocol.rs`.

## Calls this plan makes on the owner's behalf

Each is reversible by a sentence, and each is in `docs/operation-forms.md` once Task 6 writes it.

1. **"Form" means the foot row's question line with the panel beside it**, the pattern the spec names, and not a panel of fields. An earlier ruling turned that panel down ("a sea of dials and dropdowns").
2. **An operation other than filter and sort, once kept, is a new node**: an unsaved step at the spine's foot with the not-recorded mark and its table in front, as a statement kept from the palette is. Filter and sort stay views of the node in front. The earlier frames drew an applied join as a field in the band; with ten operations that band would not hold them, and each of them makes a table of another shape.
3. **The preview runs arc's own model.** Brightfield writes no operation's SQL; arc records into a throwaway Protocol and Brightfield runs what arc wrote.
4. **Optional values are asked on the line**, reached with Tab, since the panel holds no field of its own.
5. **A form over a `where` or `order by` that is applied and not recorded does not open**, and says why: it would read the table, not the rows on screen.
6. **Enter keeps** once arc has admitted the values and DuckDB has run arc's model, or once arc has admitted them and the preview could not read for size or time. ⌘S with such a form open keeps it first.
7. **Names.** The approved rule `<table>_<word>_by_<column>` stays for row operations (`_deduped_by_`, `_summarised_by_`); column operations read `<table>_<word>_<column>` (`_without_`, `_renamed_`, `_with_`, `_cast_`, `_filled_`); `limit-rows` is `<table>_first_<count>`, `join-tables` `<table>_joined_<other>`, `append-rows` `<table>_appended`, `read-file` the file's stem.
8. **Keys** take VisiData's where it has one: `-` hide, `=` derive, `f` fill, `shift-f` summarise (its frequency table), `&` join. The rest: `shift-r` rename, `shift-t` cast, `shift-u` dedupe, `shift-l` limit, `shift-a` append, `shift-o` read a file (on the spine).
9. **Filter and sort join the forms registry** (one list, one availability, the palette) and keep their foot fields.
10. **`append-rows` needs another table sharing a column name**, matching the spec's sentence "Append rows needs a second table with matching columns."
11. **A kept draft's SQL is shown read-only in the panel**; the palette does not hold a draft for editing as SQL, since arc's model for some operations is a transaction, not one query.

## Global Constraints

- This repository is public. Put no tracker refs, decision ids, milestone ids or card ids, and no absolute local paths, in code, comments, commits, pull request text or branch names. State a ruling in its own words.
- **arc holds the catalogue.** Brightfield writes no operation's SQL. The preview runs the model arc writes. The step a Save records is `arc operation record`'s, byte for byte what any other way of recording the same operation writes.
- A form is built from `arc operation describe`, run through the window's runner. The panel's sentences are arc's; the values, the keys and where each value came from are Brightfield's.
- **The foot row asks and the panel shows.** The foot row asks what the cursor does not supply, one value at a time, on one line, with the answers already given left in view. The panel lists every value and holds no field of its own.
- A join's second table, and a union's list of tables, are picked in the spine. An operation that reads no table (`read-file`) asks its file in a new row at the spine's foot, reading `from '…'`.
- Live edit, explicit save: nothing is written to the Protocol before ⌘S.
- Save asks for no name. A name is the rule's, slugged, with `_2`, `_3` for a name taken, and it says only what that one step did. It is fixed when the draft is kept.
- Brightfield holds the stage mapping: Bring in `read-file`; Shape `filter-rows`, `sort-rows`, `hide-column`, `rename-column`, `derive-column`, `cast-column`, `fill-nulls`, `dedupe-rows`, `limit-rows`, `summarise-rows`; Model `join-tables`, `append-rows`.
- Every key is a registry default, and every key the window shows acts. A command joins the registry when its form is built: a reserved verb shows greyed in the palette and the help sheet, and greyed means a missing input, never a missing feature.
- A command the window cannot take says in a sentence what it needs. Its key, and its palette row when confirmed, say the same sentence.
- The look comes from `meridian-design`'s tokens and the pieces `meridian-egui` already has (`field_row`, `key_chip`, `list_row`, `pane_header`, the theme's `TextEdit`). No new component and no colour literal (`tests/token_discipline.rs`, `tests/colour_literals.rs`).
- Each test opens its own copy of a fixture in a temp folder, since Save writes beside the data file. A headless window opens no GPU device, so no target here joins the `serial-targets:` list in `.github/workflows/test.yml`. Pixel baselines are drawn only on the Mac Studio, and no task here needs one.
- A comment that cites a test by name cites one that exists at `HEAD`.
- Before each commit: `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, and the touched tests. After each commit: `cargo xtask ci --step comment-citations`. One cargo build at a time per session. The shell crate's first test build takes minutes; later ones reuse it.
- Commit messages say what is now true, in a sentence, as this repository's history does.

## Review Focus

1. **A column whose name needs quotes, seeded from the cursor** (`median income` and `order` in `tests/data/with_nulls.csv`). A value arc takes as a name is written bare, and a value arc takes as SQL is written as an identifier, so the recorded step reads the right column. Pinned in Task 6 (`a_cursor_column_is_seeded_bare_into_a_name_and_quoted_into_sql`), and in Tasks 8, 9 and 12 on that file.
2. **A form opened over the loaded table's grid while a `where` or `order by` is applied and not recorded.** The form would read the table, not the rows shown, so it does not open and says to save or clear the view first. Pinned in Task 5 (`a_form_over_an_unrecorded_view_says_save_or_clear_it_first`).
3. **⌘S with a form still open.** A form arc has admitted is kept, then recorded with the rest. An unfinished or refused one leaves the rest saved and the form open with its reason. Pinned in Task 6 (`cmd_s_with_a_form_open_keeps_it_first_when_arc_admitted_it`, `cmd_s_with_an_unfinished_form_saves_the_rest_and_leaves_it_open`).
4. **arc refuses the values while they are asked** (`limit-rows` count `ten`, `summarise-rows` grouped by `2`). arc's words go on the status band, Enter keeps nothing, and no file is written. Pinned in Task 4 (`arcs_refusal_of_a_draft_is_said_and_nothing_is_written`) and in each operation task's refusal test.
5. **The preview cannot read while arc admitted the values.** When the data file is over the preview's limit, or the read passes its deadline, Enter still keeps and the status band says the rows could not be read. When DuckDB refuses arc's model (a column that does not exist), Enter keeps nothing. Pinned in Task 6 (`a_draft_arc_admits_is_kept_even_when_its_preview_is_too_large`, `a_draft_whose_model_duckdb_refuses_is_not_kept`).

## What the forms expect of the chrome slices

These are interfaces, not designs; each slice decides its own shape.

**Slice 1 (availability declarations and the menu bar).**
- Its planned `VerbEntry` fields (`label`, `needs`) go on the operation entries in `operations.rs`, in whichever of slice 1 and Task 2 lands second. Labels: "Hide column", "Rename column", "Derive column", "Cast column", "Fill nulls", "Dedupe rows", "Limit rows", "Summarise rows", "Join tables", "Append rows", "Read file".
- The needs the forms declare (`forms::Need`, Task 2), and what each would be in slice 1's terms:

| `forms::Need` | Slice 1 | Sentence |
|---|---|---|
| `ProtocolSource` | `Condition::ProtocolSource` | "{command} needs a Protocol of your own: open a data file or a folder." |
| `GridCell` | `Selected::GridCell` | "{command} needs the grid's cursor on a cell." |
| `ViewRecorded` | a new condition | "{command} reads the table as recorded: save or clear the where and order by first." |
| `LoadedTable` | a new condition | the grid preview's own offer ("… has not been run; Run the Protocol to filter or sort its rows") |
| `AnotherTable` | a new condition | "{command} needs a second table in the Protocol." |
| `AnotherTableSharingAColumn` | a new condition | "{command} needs a second table with matching columns." |

- One evaluator answers for the palette, the menus, the ribbon and a key press alike. A form that cannot open stays listed and keeps its key.
- The menus and the help sheet list a form's verb once its slot is built, read from `brightfield_keys::operations::operation_verbs()` and grouped by stage, so a form joins a menu with no edit to a menu table.
- A menu item reaches a form by sending `Request::Verb(<verb>)`; `MeridianApp::open_form_verb` is the one entry point. Nothing in this plan waits on the menu bar.
- If slice 1 has folded `chart_offers` into one palette list, or factored the filter and sort asks out of `grid_cursor_keys`, before Task 5 lands, Task 5 adds the built form verbs to that list and calls the factored ask, rather than doing either again.
- If slice 1's sweep that every listed key is answered has landed, the form host records each form verb a key performed, where that sweep reads it, so a built form's key counts as answered.

**Slice 2 (the ribbon).** The ribbon reads built forms from `forms::built()`: their `stage`, `verb` and `icon`. It does not keep a list of them, so a form task never edits `ribbon.rs`. `forms::Stage` has no Publish; whichever of the ribbon and Task 2 lands second makes the two `Stage` enums one. Most of the icons named here are not yet in `meridian-egui`'s Tabler manifest (it holds 48), and adding them is the design system's work for the ribbon.

**Slice 5 (the inspector).** The Operation section is the panel Task 6 draws. `open_rail_on_operation` and `restore_rail_after_ask` are the two calls a form makes on the rail.

## File structure

| File | Responsibility |
|---|---|
| `crates/brightfield-shell/src/operation.rs` | arc's description, whole; `record_step` naming each read by its role |
| `crates/brightfield-shell/src/forms/mod.rs` | `FormSpec` and the table of 13 slots |
| `crates/brightfield-shell/src/forms/<op>.rs` (13) | one operation's form: its line, needs, name and module doc |
| `crates/brightfield-shell/src/forms/draft.rs` | arc's model for a draft, from a throwaway Protocol |
| `crates/brightfield-shell/src/forms/open.rs` | `FormContext`, `Availability` and the sentences |
| `crates/brightfield-shell/src/forms/line.rs` | the question line: values, seeds, focus, words, the request |
| `crates/brightfield-shell/src/window/form_host.rs` | the window's side of a form: open, keys, reads, keep, status (a child module of `window`, so it reaches the window's private fields) |
| `crates/brightfield-shell/src/spine_pick.rs` | the spine picking a table for an operation |
| `crates/brightfield-keys/src/operations.rs` | one key-registry entry per operation, behind its own `*_BUILT` |
| `crates/brightfield-engine/src/preview.rs` | `PreviewOp::Model`, and for `read-file` `PreviewOp::ReadFile` |
| `crates/brightfield-shell/src/generated.rs` | a generated step of any operation read back; `effect_step_name` |
| `crates/brightfield-shell/tests/support/forms.rs` | the harness every form test drives the window with |
| `docs/operation-forms.md` | the pattern, the calls above, and how to add a form |

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

`record_step` runs `operation record <long_name> [--on <on>] [--read <role>=<table>]… --name <name> [--arg <key>=<value>]…` in `dir` with no `--dir`, as `record` does. A list role appears once per table, in order. A `Columns` value is passed as its items joined by `,`, arc's command-line form. arc writes its notes to stderr even when it records, for example "`https://…` is read live at each run", so they are kept.

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

### Task 2: A slot for every operation, in the forms registry and the key registry

**Files:**
- Create: `crates/brightfield-shell/src/forms/mod.rs`
- Create: `crates/brightfield-shell/src/forms/{filter_rows,sort_rows,hide_column,rename_column,derive_column,cast_column,fill_nulls,dedupe_rows,limit_rows,summarise_rows,join_tables,append_rows,read_file}.rs`. Each holds `pub fn form() -> Option<&'static FormSpec> { None }` and a module doc of one line naming the operation and saying its form is not built yet.
- Modify: `crates/brightfield-shell/src/lib.rs` (`pub mod forms;`, and a line in the crate doc's list)
- Create: `crates/brightfield-keys/src/operations.rs`
- Modify: `crates/brightfield-keys/src/lib.rs` (`pub mod operations;`)
- Modify: `crates/brightfield-keys/src/registry.rs` (`registry()` appends `crate::operations::operation_verbs()`; `longname_snapshot_is_stable` compares the registry less `operations::is_operation_verb` names; `the_grid_context_moves_the_cursor_a_cell_on_each_of_its_keys` adds each built operation verb's Grid key to `expected`, read from `operations::operation_verbs()`)
- Test: `crates/brightfield-shell/tests/forms_registry.rs` (new); unit tests in `operations.rs`

**Interfaces:**
- Consumes: `crate::grid_foot::FootField`; `brightfield_keys::{VerbEntry, BindingContext, …}`.
- Produces (`forms/mod.rs`):

```rust
pub struct FormSpec {
    pub long_name: &'static str,   // arc's
    pub verb: &'static str,        // the registry verb that opens it
    pub stage: Stage,
    pub icon: &'static str,        // a Tabler name, for the ribbon
    pub asks_in: AsksIn,
    pub line: &'static [Ask],      // every value arc describes, in the order the line shows them
    pub opens_on: &'static str,    // the key the line opens on
    pub needs: &'static [Need],
    pub name: NameRule,
}
pub enum Stage { BringIn, Shape, Model }
pub enum AsksIn { FootField(FootField), QuestionLine, SpinePick { role: &'static str }, SpineFoot }
pub struct Ask { pub key: &'static str, pub words: &'static str, pub after: &'static str, pub seed: Seed,
                 pub instead_of: Option<&'static str> }   // a value given drops the one it names from the request
pub enum Seed { Empty, CursorColumn, CursorColumnIfShared { role: &'static str }, Text(&'static str), Default }
pub enum Need { ProtocolSource, GridCell, ViewRecorded, LoadedTable, AnotherTable, AnotherTableSharingAColumn }
pub struct NameRule { pub word: &'static str, pub from: By, pub with_by: bool }
pub enum By { Nothing, Value(&'static str), PathStem(&'static str) }
pub const OPERATIONS: [(&str, fn() -> Option<&'static FormSpec>); 13];   // arc's order: filter_rows::form, sort_rows::form, …
pub fn spec(long_name: &str) -> Option<&'static FormSpec>
pub fn by_verb(verb: &str) -> Option<&'static FormSpec>
pub fn built() -> impl Iterator<Item = &'static FormSpec>
pub fn label(long_name: &str) -> String   // "hide-column" -> "Hide column"
```

A line value renders as `words` + its text + `after`. The line is its shown values joined by one space, so `Ask { key: "using", words: "using (", after: ")" }` reads `using (house_age)`.

- Produces (`operations.rs`): `pub fn operation_verbs() -> Vec<VerbEntry>` (the built ones, in arc's order); `pub fn is_operation_verb(longname: &str) -> bool`; and `pub fn all_operation_entries() -> Vec<(bool, VerbEntry)>` for the key-collision test. Each operation is a block of its own, separated from its neighbours by its entry function:

```rust
// ---- hide-column ----
const HIDE_COLUMN_BUILT: bool = false;
fn hide_column() -> VerbEntry { VerbEntry { longname: "hide-column", binding_specs: vec![grid("-")], … } }
```

Every entry: `tier: CommandTier::View`, `drives: Drives::Navigation`, `status: VerbStatus::Built`, `reserved_reason: None`, `scope_applicability: vec![View]` (and `Protocol` for `read-file`). The keys and help, decided here once so that no two tasks pick the same key:

| Verb | Key | Context | Help | Scores (f, m, c): note |
|---|---|---|---|---|
| `hide-column` | `-` | Grid | Hide the cursor's column: the line at the grid's foot reads exclude and the column | 3, 4, 4: "- is VisiData's hide; the Workspace's - zooms in its own context" |
| `rename-column` | `shift-r` | Grid | Rename the cursor's column: the line asks the new name | 3, 4, 2: "R = rename; bare r stays free for the grid's movement" |
| `derive-column` | `=` | Grid | Add a column computed by an expression, starting from the cursor's column | 3, 4, 4: "= is VisiData's new column from an expression" |
| `cast-column` | `shift-t` | Grid | Change the cursor's column's type: the line asks the type | 2, 4, 2: "T = type; retype-column is the model's, in its own context" |
| `fill-nulls` | `f` | Grid | Fill the missing values of the cursor's column: the line asks the value | 2, 4, 3: "f is VisiData's fill; the grid's filter is w" |
| `dedupe-rows` | `shift-u` | Grid | Keep one row of each set of duplicates, keyed on the cursor's column | 2, 3, 2: "U = unique; bare u is undo elsewhere" |
| `limit-rows` | `shift-l` | Grid | Keep the first rows in the order of the cursor's column: the line asks how many | 2, 4, 2: "L = limit; bare l moves the cursor right" |
| `summarise-rows` | `shift-f` | Grid | Summarise the rows by the cursor's column: a count of each value, the aggregates editable | 3, 3, 5: "F is VisiData's frequency table of the cursor's column" |
| `join-tables` | `&` | Grid | Join the table in front to another, picked in the spine, on the cursor's column | 2, 4, 5: "& is VisiData's join" |
| `append-rows` | `shift-a` | Grid | Append the rows of other tables, picked in the spine, matching columns by name | 2, 4, 3: "A = append, as vim's A" |
| `read-file` | `shift-o` | Protocol | Read a file into a new table at the spine's foot: the row asks the path | 2, 4, 4: "O = open, VisiData's o; bare o organises joins here and sorts in the grid" |

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

In `forms_registry.rs`, with the suite's runner:

```rust
#[test] fn every_operation_arc_holds_has_one_slot() { /* OPERATIONS' names == list_operations(), as sets, 13 */ }
#[test] fn a_form_is_built_exactly_when_its_command_is_in_the_key_registry() {
    // spec(op).is_some() <=> registry() has spec.verb; filter-rows' verb is filter-rows-at-cursor,
    // sort-rows' sort-rows-at-cursor; an unbuilt op's long name is no registry verb
}
#[test] fn each_built_form_asks_every_value_arc_describes() {
    // every parameter key arc describes is a line key; every line key is a parameter or a table role;
    // opens_on is a line key; instead_of names a line key
}
#[test] fn each_built_forms_needs_agree_with_what_arc_says_it_reads() {
    // ProtocolSource always; GridCell iff a required `table` role; AnotherTable iff a required single `other`;
    // AnotherTableSharingAColumn iff a required list `other`
}
#[test] fn each_built_form_is_filed_under_its_stage() { /* read-file BringIn; join-tables, append-rows Model; the rest Shape */ }
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-keys` then `cargo test -p brightfield-shell --test forms_registry`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** the two files and the stubs, and fold `operation_verbs()` into `registry()`. Then reword the two registry tests named above, so that a form built later edits neither.
- [ ] **Step 4: Run.** The same two commands. Expected: PASS. The forms tests pass with nothing built, and they bite as each form fills its slot.
- [ ] **Step 5: Commit.** `git commit -m "Give each of arc's operations a slot in the forms registry and the key registry, empty until its form is built"`, then `--step comment-citations`.

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

In `generated_any_operation.rs`, on a copy of `california_housing_sample.csv` saved once so that its load is written, with steps recorded through `record_step`: `h` hide-column on the root (`column=house_age`); `r` rename-column on `h` (`median_income` to `income`); `s` summarise-rows on the root (`count(*) AS n`, grouped by `house_age`); `j` join-tables on the root with `other=s`, `using=house_age`; `a` append-rows on `h` with `other=r`.

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

### Task 4: A draft arc writes: previewed as arc's model, kept as an unsaved step, recorded on ⌘S

**Files:**
- Create: `crates/brightfield-shell/src/forms/draft.rs`
- Modify: `crates/brightfield-shell/src/forms/mod.rs` (`pub mod draft;`)
- Modify: `crates/brightfield-shell/src/typed_sql.rs` (`TypedStep::operation`; `as_generated`; `TypedReads::edit_draft` and its worker; `TypedAnswer::{model, unread}`; `TypedSource::file_limit`; module doc)
- Modify: `crates/brightfield-shell/src/generated.rs` (`effect_step_name`; `sql_step_name_for` calls it with `"sql"`)
- Modify: `crates/brightfield-shell/src/app.rs` (`typed_lines` and a pending step's version words name the operation; `replace_typed` clears `operation`)
- Modify: `crates/brightfield-shell/src/window.rs` (`record_pending` records a draft through `record_step`; the bring-to-front tail of `keep_statement` becomes `bring_kept_to_front`, which both call; `pub fn keep_draft`; the statement form does not hold a kept draft; `typed_source` sets `file_limit` from the document's preview limit)
- Modify (compile only): `crates/brightfield-shell/tests/typed_sql_nodes.rs` (`operation: None` in `typed`)
- Test: `crates/brightfield-shell/tests/form_drafts.rs` (new)

**Interfaces:**
- Consumes: Task 1's `StepRequest`, `record_step`, `Recorded`; Task 3's `PreviewOp::Model`, `StepEffect::Generated`.
- Produces:

```rust
// forms/draft.rs
pub struct DraftModel { pub model: String, pub notes: Vec<String> }
pub fn draft_model(runner: &Runner, name: &str, request: &StepRequest) -> Result<DraftModel, String>
// typed_sql.rs
pub struct TypedStep { pub name: String, pub statement: String, pub reads: Vec<String>, pub rows: Option<u64>,
    pub operation: Option<StepRequest> }       // Some for a step a form made; `statement` is then arc's model
pub struct TypedAnswer { /* as now */ pub model: Option<DraftModel>, pub unread: Option<String> }
impl TypedReads { pub fn edit_draft(&mut self, name: &str, request: StepRequest, now: Instant) }
pub struct TypedSource { /* as now */ pub file_limit: u64 }
// generated.rs
pub fn effect_step_name(word: &str, table: Option<&str>, column: Option<&str>, with_by: bool, taken: &BTreeSet<String>) -> String
// window.rs
impl MeridianApp { pub fn keep_draft(&mut self, ctx: &egui::Context, name: String, request: StepRequest, model: String) }
```

`draft_model` writes, in a temp folder it removes after, a manifest naming one stub step per table the request reads (`bf_stub_1`, `bf_stub_2`, …). Each stub's model is `CREATE OR REPLACE TABLE "<table>" AS SELECT 1 AS x;`, the table spelled as a quoted identifier, since arc finds what a step makes from its model. It then calls `record_step` there, and reads the model back through the manifest's `sql:` for `name`. The text is what arc writes for the same request in the real Protocol, byte for byte: arc's spelling of a name does not depend on the folder. A call took about 150 ms when this plan was written.

A draft read works in two parts. First it gets arc's model, and a refusal there is the read's refusal, in arc's words. Then it reads the first page through `preview_chain_reading` and the draft as `PreviewOp::Model`. A `PreviewRefusal::TooLarge` or `TookTooLong` answers with an empty page and `unread` saying why; a `PreviewRefusal::Engine` refuses the read.

`effect_step_name` names a step `<table>_<word>_by_<column>` when `with_by` is set and `<table>_<word>_<column>` when it is not, or `<table>_<word>` with no column; an empty `word` names it by `table` alone, as `read-file` does with the file's stem. It slugs each part and suffixes a taken name as `sql_step_name_for` does today.

`as_generated` gives `StepEffect::Generated(long_name)` for a step with an operation. Words: `typed_lines` gives `"<long name> · <name>"` for such a step and `"SQL step · <name>"` otherwise, and the version a close keeps uses the same. A palette opened over a kept draft's grid starts empty.

- [ ] **Step 1: Write the failing tests** in `form_drafts.rs`, on a copy of `california_housing_sample.csv` with the suite's runner. `hide` is `StepRequest { long_name: "hide-column", on: Some("california_housing_sample"), reads: [], args: [("column","house_age")] }`.

```rust
#[test] fn a_drafts_model_is_what_arc_records_in_the_protocol() {
    // m = draft_model(runner, "h", &hide); keep_draft(.., "h", hide, m.model.clone()); save
    // -> the folder's model for step h == m.model, byte for byte
}
#[test] fn a_kept_draft_is_an_unsaved_step_at_the_spines_foot() {
    // after keep_draft: has_unsaved_work(); typed_steps()[0].operation.is_some(); is_typed("h");
    // the canvas holds h's grid, whose columns lack house_age; close-question lines hold "hide-column · h"
}
#[test] fn a_draft_reads_a_kept_draft_before_it() {
    // keep h; edit_draft "r": rename-column on h, median_income -> income; once answered: the page lacks house_age and has income
}
#[test] fn arcs_refusal_of_a_draft_is_said_and_nothing_is_written() {
    // edit_draft limit-rows count "ten": refusal() contains "is an integer, not \"ten\""; folder byte-identical; nothing kept
}
#[test] fn save_records_statements_and_drafts_in_the_order_they_were_made() {
    // keep a typed statement, then h; save -> the manifest's new steps are [that statement's name, h];
    // h's model opens with "-- generated: hide-column on california_housing_sample"
}
#[test] fn a_draft_arc_refuses_at_save_stays_kept_in_arcs_words() {
    // keep h; record a step named h into the folder through record_step, as an agent would; save
    // -> banner body in arc's words; typed_steps() still holds h; has_unsaved_work()
}
#[test] fn the_statement_form_over_a_drafts_grid_starts_empty() { /* Space over h's grid: the form's text is "" */ }
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test form_drafts`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** in the order of the Files list. Put `record_step` into the `PendingWork::Typed` arm of `record_pending` when `step.operation` is `Some`, and leave `beyond_edition::statement` over the model text as it is.
- [ ] **Step 4: Run.** `cargo test -p brightfield-shell --test form_drafts --test typed_sql_save --test typed_sql_palette --test typed_sql_nodes --test typed_sql_reads --test close_question_keeps_unsaved`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Keep a form's draft as an unsaved step previewed as arc writes it, and record it through arc on Save"`, then `--step comment-citations`.

### Task 5: Opening a form, from its key or the palette, with a sentence when it cannot; filter and sort join

**Files:**
- Create: `crates/brightfield-shell/src/forms/open.rs`
- Create: `crates/brightfield-shell/src/window/form_host.rs` (declared `mod form_host;` in `window.rs`)
- Modify: `crates/brightfield-shell/src/window.rs` (`mod form_host;`; an arm in `apply` ahead of the `graph_on_canvas` arm for a verb `forms::by_verb` knows; `grid_cursor_keys` sends every built form verb, filter and sort included, to `open_form_verb`; `consume_token` gains `f`, `shift-u`, `shift-l`, `shift-f`, `shift-o` and `&`; descriptions fetched for every built form on the first frame with a runner; the status band reads `form_status_entry`)
- Modify: `crates/brightfield-shell/src/forms/filter_rows.rs`, `forms/sort_rows.rs`
- Modify: `crates/brightfield-shell/src/overlays.rs` (`chart_offers` and `chart_palette_verbs` take each built form verb where `saveable`; their docs)
- Modify: `crates/brightfield-shell/tests/overlay_wiring.rs` (one arm for every form verb), `crates/brightfield-shell/tests/shelf_column_route.rs` (one arm for a form verb on the Protocol palette)
- Test: `crates/brightfield-shell/tests/form_opening.rs` (new)

**Interfaces:**
- Consumes: Task 2's `FormSpec`, `Need`, `by_verb`, `built`, `label`; `operation::Descriptions`.
- Produces:

```rust
// forms/open.rs
pub struct FormContext { pub runner: bool, pub protocol: Option<PathBuf>, pub in_front: Option<InFront>,
    pub tables: Vec<(String, Option<Vec<String>>)> }              // every table of the Protocol, recorded or kept, with its columns once known
pub struct InFront { pub table: String, pub loaded: bool, pub cell: bool /* rows drawn, cursor on a cell */,
    pub view_unrecorded: bool, pub columns: Vec<String>, pub cursor_column: Option<String>, pub step_offer: Option<String> }
pub enum Availability { Available, Unavailable(String) }
pub fn availability(spec: &FormSpec, cx: &FormContext) -> Availability
// window/form_host.rs
impl MeridianApp {
    pub(crate) fn form_context(&self) -> FormContext;
    pub(crate) fn open_form_verb(&mut self, ctx: &egui::Context, verb: &str) -> bool;   // keys, palettes and a menu all come here
    pub fn form_said(&self) -> Option<&str>;
}
pub const FORM_STATUS_ID: &str = "operation-form";
```

`open_form_verb` works in order. With no runner, it says "{command} records through arc, and this window was started without it.". It checks the description (`Descriptions::get`): pending says "Reading arc's description of {long name}…", and failed says "No description of {long name}: {why}". Then `availability`; then the open. `{command}` is `forms::label`. Needs are checked in the order a spec lists them, and the first unmet one is said, with the sentence in the slice 1 table above. A sentence stays on the status band until the next key or pointer press. The `FootField` arm is today's: `ask_where_at_cursor` or `ask_order_at_cursor` with the cursor on a cell, else `foot_offer(field, "")`, then `open_rail_on_operation`. The `QuestionLine`, `SpinePick` and `SpineFoot` arms answer `false`, and Tasks 6, 7 and 18 fill them; no unbuilt form reaches them, since an unbuilt one is in no registry.

`filter-rows` and `sort-rows` specs:

```rust
FormSpec { long_name: "filter-rows", verb: "filter-rows-at-cursor", stage: Stage::Shape, icon: "filter",
    asks_in: AsksIn::FootField(FootField::Where),
    line: &[Ask { key: "where", words: "where ", after: "", seed: Seed::Empty, instead_of: None }],   // the foot field seeds it from the cell
    opens_on: "where", needs: &[Need::ProtocolSource, Need::GridCell, Need::LoadedTable],
    name: NameRule { word: "filtered", from: By::Value("where"), with_by: true } }   // recording keeps record_view's own naming
// sort-rows: verb "sort-rows-at-cursor", icon "arrows-sort", FootField(OrderBy), key "order_by", words "order by ",
// needs the same, name word "sorted"
```

- [ ] **Step 1: Write the failing tests** in `form_opening.rs`, on a copy of the housing sample:

```rust
#[test] fn the_palette_opens_filter_rows_in_the_where_field() { /* cursor on a cell; Space, "filter-rows", Enter -> foot_asking() is Where; rail on Operation */ }
#[test] fn the_palette_opens_sort_rows_in_the_order_by_field() { /* the same for OrderBy */ }
#[test] fn a_forms_key_off_a_cell_says_why_instead_of_nothing() {
    // the grid in its columns layout, press w -> form_said() == Some("Filter rows needs the grid's cursor on a cell."); foot not asking
}
#[test] fn filter_rows_on_a_steps_grid_says_the_steps_offer() {
    // a folder with a recorded filter step not run; its grid; palette filter-rows -> form_said() is that preview's offer()
}
#[test] fn a_form_over_an_unrecorded_view_says_save_or_clear_it_first() {
    // availability(&HIDE_LIKE, &cx) with in_front.view_unrecorded ==
    // Unavailable("Hide column reads the table as recorded: save or clear the where and order by first.")
    // (HIDE_LIKE: a spec local to the test, needs [ProtocolSource, GridCell, ViewRecorded])
}
#[test] fn availability_names_the_first_need_it_lacks() {
    // one context per Need left unmet, each answering its sentence from the table above; a context meeting all answers Available
}
#[test] fn the_chart_palette_lists_built_forms_only_where_a_protocol_is_saved() {
    // chart_palette_verbs(true) holds filter-rows-at-cursor and sort-rows-at-cursor; chart_palette_verbs(false) holds neither
}
```

The arm in `overlay_wiring.rs`'s sweep, for `v if forms::by_verb(v).is_some()`, opens its own copy of the housing sample with the cursor on a cell and confirms `v`. It then asserts that the foot asks, or that `form_said()` is `Some`. Task 6 adds the open question line to that assertion.

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test form_opening`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** `open.rs`, then `form_host.rs` and the `window.rs` hooks, then the two specs and the palette.
- [ ] **Step 4: Run.** `cargo test -p brightfield-shell --test form_opening --test overlay_wiring --test shelf_column_route --test operation_from_the_cursor --test grid_foot --test forms_registry`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Open an operation's form from its key or the palette, or say what it needs; filter and sort are the first two"`, then `--step comment-citations`.

### Task 6: The question line and the panel: one value at a time, every value shown, Enter keeps

**Files:**
- Create: `crates/brightfield-shell/src/forms/line.rs`
- Modify: `crates/brightfield-shell/src/forms/mod.rs` (`pub mod line;`)
- Modify: `crates/brightfield-shell/src/app.rs` (`ChartDoc` holds `Option<QuestionLine>`: `form`, `form_mut`, `set_form`, `take_form`)
- Modify: `crates/brightfield-shell/src/grid_foot.rs` (while a form asks, the band draws its line in the two fields' place, on the loaded table's grid and a step's alike; `column_sql` becomes `pub(crate)`; module doc)
- Modify: `crates/brightfield-shell/src/operation_panel.rs` (the panel reads the open line: each read role and value with where it came from, arc's sentence and offered words, and arc's model under *SQL*; `ValueFrom` gains `ArcDefault`, `Spine`, `NotGiven`; module doc, keeping "the panel changes no value")
- Modify: `crates/brightfield-shell/src/window/form_host.rs` (the `QuestionLine` arm; a key pass ahead of `grid_foot_keys` for Tab, Shift-Tab, Enter and Esc; each frame the line's request goes to `TypedReads::edit_draft` and the answer's page to the grid, as the statement form's is (`GridPreview::of_typed`, `show_typed_preview`); ⌘S keeps first; the status line; `pub fn form_open`; `#[doc(hidden)] pub fn open_form_spec`)
- Modify: `crates/brightfield-shell/src/window.rs` (call the key pass; `save_everything` asks the form host first)
- Modify: `crates/brightfield-shell/tests/overlay_wiring.rs` (the form arm also accepts `form_open()`)
- Create: `crates/brightfield-shell/tests/support/forms.rs`, `docs/operation-forms.md`
- Test: `crates/brightfield-shell/tests/form_question_line.rs` (new)

**Interfaces:**
- Consumes: Tasks 1 to 5.
- Produces:

```rust
// forms/line.rs
pub struct QuestionLine { /* spec, description, on, values, focus, taken tables */ }
pub struct LineValue { pub key: &'static str, pub text: String, pub from: ValueFrom, pub selected_whole: bool }
impl QuestionLine {
    pub fn open(spec: &'static FormSpec, description: OperationDescription, in_front: &InFront, quote: impl Fn(&str) -> String) -> Self;
    pub fn spec(&self) -> &'static FormSpec;
    pub fn values(&self) -> &[LineValue];
    pub fn focused(&self) -> &'static str;
    pub fn words(&self) -> String;                 // the line as it reads, the focused value's text included, trailing space trimmed
    pub fn type_text(&mut self, text: &str);       // replaces a value selected whole
    pub fn tab(&mut self); pub fn back_tab(&mut self);
    pub fn take_tables(&mut self, role: &str, tables: Vec<String>, columns: Vec<Vec<String>>);
    pub fn missing(&self) -> Option<String>;       // "<long name> needs <key>" for the first required value left empty
    pub fn request(&self) -> StepRequest;
    pub fn step_name(&self, taken: &BTreeSet<String>) -> String;
}
// form_host
impl MeridianApp { pub fn form_open(&self) -> bool; #[doc(hidden)] pub fn open_form_spec(&mut self, ctx: &egui::Context, spec: &'static FormSpec) -> bool }
```

Rules the line keeps, each from the description:
- **Seeds.** `CursorColumn` puts the cursor's column in a `Column` or `Columns` value as the bare name, and in a `Sql` value as `column_sql` spells it: bare when plain, quoted otherwise. `CursorColumnIfShared { role }` does the same when a table taken for `role` holds that column (case folded), and leaves the value empty otherwise. `Text` is the text; `Default` is arc's `Choice` default. Every seeded value is selected whole, so the first key typed replaces it.
- **Shown.** An optional value that is empty and not focused is left out of the words, and so is a value another one's `instead_of` dropped.
- **Requests.** `on` is the table in front, or `None` when arc's description reads no `table`. `reads` are the tables taken in the spine. `args` are the values that are not empty and not dropped, excluding role keys; a `Columns` value's items are split on `,` and trimmed.
- **Names.** `By::Value(key)` takes the value's first comma-separated item, drops a surrounding `"` and a trailing `asc` or `desc`, and names nothing when what is left holds anything but letters, digits, `_` and spaces. `By::PathStem(key)` is the file's stem. The name goes through `effect_step_name`, against the manifest's names and the kept steps'.
- **Keys.** Enter keeps when `missing()` is `None` and the read of the line as it stands has answered with arc's model; otherwise it says `missing()` or the refusal. Keeping calls `keep_draft` with the line's name, request and model, closes the line and `restore_rail_after_ask`. Esc closes, keeps nothing and gives the grid back. ⌘S with a line open keeps it first if Enter would; otherwise it saves the rest and leaves the line open, saying why.
- **The status line.** arc's refusal, or DuckDB's, in the warning tone; else arc's notes; else `unread`; else `"<long name> on <table>"`.

The harness, `tests/support/forms.rs`:

```rust
pub struct FormWindow { pub app: MeridianApp, pub ctx: egui::Context /* and its temp folder */ }
impl FormWindow {
    pub fn on_copy_of(fixture: &str) -> Self;     // copies tests/data/<fixture> into a temp folder, opens it with the suite's runner
    pub fn reopen(&self) -> Self;                 // a new window on the same folder's manifest
    pub fn cursor_on(&mut self, column: &str);    // the grid focused, the cursor on the first row of `column`
    pub fn press(&mut self, token: &str);         // a registry token: "-", "shift-r", "&", "cmd-s", …
    pub fn palette(&mut self, verb: &str);
    pub fn type_text(&mut self, text: &str); pub fn key(&mut self, key: egui::Key);
    pub fn settle_reads(&mut self);               // frames until the draft read answers, at most 30 s
    pub fn line(&self) -> Option<String>; pub fn said(&self) -> Option<String>;
    pub fn grid_columns(&self) -> Vec<String>; pub fn grid_cell(&self, row: usize, column: &str) -> Option<String>;
    pub fn kept(&self) -> Vec<TypedStep>; pub fn save(&mut self);
    pub fn manifest_steps(&self) -> Vec<String>; pub fn model_of(&self, step: &str) -> String;
    pub fn show_grid_of(&mut self, step: &str); pub fn step_ran(&self, step: &str) -> bool;
}
```

- [ ] **Step 1: Write the failing tests** in `form_question_line.rs`. `RENAME` and `DERIVE` are specs local to the test, with the lines Tasks 9 and 10 give their operations; `open_form_spec` opens them.

```rust
#[test] fn the_line_opens_seeded_from_the_cursor_and_on_the_value_it_asks() {
    // with_nulls.csv, cursor on `median income`, RENAME -> line() == "rename median income as"; focused "to"
}
#[test] fn a_cursor_column_is_seeded_bare_into_a_name_and_quoted_into_sql() {
    // cursor on `order`: RENAME's column value is `order`; DERIVE's expression value is `"order"`
}
#[test] fn typing_previews_arcs_model_on_the_node_in_front() {
    // RENAME, type "income", settle -> grid_columns() == [city, house_age, order, income];
    // the panel's SQL holds `RENAME COLUMN "median income" TO income`
}
#[test] fn tab_moves_to_the_next_value_and_shift_tab_back() {}
#[test] fn enter_keeps_the_step_and_brings_its_table_to_the_front() {
    // kept()[0].name == "with_nulls_renamed_median_income", operation Some; its grid on the canvas; form_open() false
}
#[test] fn esc_keeps_nothing_and_gives_the_grid_back() {}
#[test] fn enter_with_a_required_value_empty_says_what_is_missing() { /* said() == Some("rename-column needs to"); nothing kept */ }
#[test] fn the_panel_shows_every_value_and_where_it_came_from() {
    // table ← the node in view; column ← the cursor; to ← asked; arc's sentence under each
}
#[test] fn cmd_s_with_a_form_open_keeps_it_first_when_arc_admitted_it() { /* typed, settled, cmd-s -> the manifest holds the step */ }
#[test] fn cmd_s_with_an_unfinished_form_saves_the_rest_and_leaves_it_open() {
    // a statement kept, then RENAME open with `to` empty, cmd-s -> the statement recorded; form_open(); said() names `to`
}
#[test] fn a_draft_arc_admits_is_kept_even_when_its_preview_is_too_large() {
    // chart_doc_mut().set_preview_file_limit(1); RENAME to "income" -> said() holds the too-large sentence; Enter keeps
}
#[test] fn a_draft_whose_model_duckdb_refuses_is_not_kept() {
    // RENAME `median income` to `city` (a column the table has) -> said() holds DuckDB's words; Enter keeps nothing
}
```

- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test form_question_line`. Expected: FAIL to compile.
- [ ] **Step 3: Implement** `line.rs`, the document field, the band, the panel, then the host. Write `docs/operation-forms.md`. Cover what a form is; the flow from key to ⌘S; the rulings under Global Constraints; the calls above, each marked as the owner's to overturn; what arc lacks (see Task 19); and how to add a form: fill `forms/<op>.rs`, flip its `*_BUILT`, write `tests/form_<op>.rs` from the harness.
- [ ] **Step 4: Run.** `cargo test -p brightfield-shell --test form_question_line --test operation_panel --test grid_foot --test form_opening --test overlay_wiring --test token_discipline --test colour_literals`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Ask an operation's values one at a time in the grid's foot row, show each in the panel, and keep the step on Enter"`, then `--step comment-citations`.

### Task 7: The spine picks a table for an operation

**Files:**
- Create: `crates/brightfield-shell/src/spine_pick.rs`
- Modify: `crates/brightfield-shell/src/lib.rs` (`pub mod spine_pick;`)
- Modify: `crates/brightfield-keys/src/registry.rs` (`BindingContext::SpinePick`; five verbs; `longname_snapshot_is_stable`'s list; a context test as the Grid's is)
- Modify: `crates/brightfield-keys/src/dispatch.rs` (`DispatchContext::SpinePickFocused`; the matrix and its tests)
- Modify: `crates/brightfield-shell/src/protocol.rs` (the spine's rows while it picks)
- Modify: `crates/brightfield-shell/src/window/form_host.rs` (the `SpinePick` arm)
- Modify: `crates/brightfield-shell/src/window.rs` (the SpinePick context's bindings resolve while the spine picks)
- Test: `crates/brightfield-shell/tests/spine_pick.rs` (new)

**Interfaces:**
- Consumes: Task 6's `QuestionLine::take_tables`, `open_form_spec`.
- Produces:

```rust
pub struct SpinePick { /* role, list, in_front, candidates, cursor, added */ }
pub struct Candidate { pub table: String, pub columns: Option<Vec<String>>, pub shares: Option<String> /* the cursor's column, when it holds it */ }
impl SpinePick {
    pub fn new(role: &'static str, list: bool, in_front: &str, cursor_column: Option<&str>, tables: Vec<String>) -> Self;
    pub fn candidates(&self) -> &[Candidate]; pub fn cursor(&self) -> usize; pub fn added(&self) -> &[String];
    pub fn next(&mut self); pub fn previous(&mut self); pub fn add(&mut self);
    pub fn take(&mut self) -> Vec<String>;         // the added tables in order, or the one under the cursor
    pub fn set_columns(&mut self, table: &str, columns: Vec<String>);
}
```

The verbs, all `CommandTier::View`, `Drives::Navigation`, in `BindingContext::SpinePick`: `pick-next-table` (`j`, `down`), `pick-previous-table` (`k`, `up`), `add-table-to-pick` (`space`), `take-picked-table` (`enter`), `back-out-of-pick` (`escape`).

How it looks: the rail has the focus ring; the table in front reads *in front*; rows that are not tables are in the muted ink; each other table trails `shares <column>` once its columns are known; the cursor is a selected row, and an added table is marked selected. A candidate's columns are read on a worker, as a first page of no rows through its chain, so the spine never waits on DuckDB. While the spine picks, the line reads its words and then `← from the spine`, and the status band names the five keys. Take hands the tables and their columns to `take_tables` and gives the keyboard back to the line. Esc gives it back to the grid and keeps nothing.

- [ ] **Step 1: Write the failing tests** in `spine_pick.rs`, with a spec local to the test shaped like Task 16's. The folder is a copy of the housing sample with a step `by_age` (summarise-rows by `house_age`) and a step `top` (limit-rows 10), both recorded through `record_step`.

```rust
#[test] fn the_spine_marks_the_table_in_front_and_starts_on_the_first_other() {}
#[test] fn each_table_says_whether_it_shares_the_cursors_column() { /* cursor on house_age: by_age and top trail "shares house_age" */ }
#[test] fn enter_takes_the_table_and_gives_the_line_back_the_keys() {
    // on by_age, Enter -> line() == "left join by_age using (house_age)"; focused "using"; the grid has the keys
}
#[test] fn space_adds_to_a_list_and_enter_takes_them_in_order() { /* a list-role spec: add top then by_age -> take_tables got [top, by_age] */ }
#[test] fn esc_goes_back_to_the_grid_and_keeps_nothing() {}
```

And in `registry.rs`: `the_spine_pick_context_moves_adds_takes_and_backs_out`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-keys` then `cargo test -p brightfield-shell --test spine_pick`. Expected: FAIL.
- [ ] **Step 3: Implement** the keys, then `spine_pick.rs`, then the spine's rows and the host arm.
- [ ] **Step 4: Run.** `cargo test -p brightfield-keys` then `cargo test -p brightfield-shell --test spine_pick --test navigator_spine --test form_question_line`. Expected: PASS.
- [ ] **Step 5: Commit.** `git commit -m "Pick the table an operation reads in the spine, one or a list, with the table in front marked"`, then `--step comment-citations`.

### Every operation task (8 to 18) follows these steps

Each operation task touches its own `crates/brightfield-shell/src/forms/<op>.rs`, its own `<OP>_BUILT` line in `crates/brightfield-keys/src/operations.rs`, and a new `crates/brightfield-shell/tests/form_<op>.rs`, and nothing else unless the task says so. Its test file opens with `#[path = "support/forms.rs"] mod forms;`. The key, the help and the scores are Task 2's, already written.

- [ ] **Step 1: Write the failing tests** the task names, with the values it gives.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test form_<op>`. Expected: FAIL, the key or palette row opens nothing.
- [ ] **Step 3: Fill the slot.** `form()` returns `Some(&SPEC)`, the task's spec. The module doc says what the line reads, the key, the name rule, and any call the task made.
- [ ] **Step 4: Flip** `<OP>_BUILT` to `true`.
- [ ] **Step 5: Run.** `cargo test -p brightfield-shell --test form_<op> --test forms_registry --test overlay_wiring` then `cargo test -p brightfield-keys`. Expected: PASS.
- [ ] **Step 6: Commit.** `git commit -m "<Label>'s form asks <what it asks> and records the step through arc"`, then `--step comment-citations`.

Every operation's tests include these five, named for the operation:
- `<op>_opens_from_its_key_seeded_from_the_cursor`: `line()` reads the given words.
- `<op>_previews_arcs_model_on_the_table_in_front`: `grid_columns()` and the cells given.
- `<op>_keeps_a_step_named_by_its_effect_and_save_records_arcs_model`: `kept()[0].name` is the given name; after `save()` the manifest holds it, and `model_of(name) == kept_model`, the kept step's `statement`, taken before the save.
- `<op>_reopens_as_a_step_not_run_that_previews_the_same`: in `reopen()`, `step_ran(name)` is false and the step's grid has the same columns.
- `<op>_says_the_refusal_and_keeps_nothing`: with the input given, `said()` holds the fragment given, Enter keeps nothing, and the folder is unchanged.

The housing sample (`tests/data/california_housing_sample.csv`) loads as the table `california_housing_sample`, 240 rows of `median_income, house_age, avg_rooms, avg_bedrooms, population, avg_occupancy, latitude, longitude, median_house_value`. `tests/data/with_nulls.csv` loads as `with_nulls`, with the columns `city, house_age, order, median income`, and Berkeley's `house_age` is empty.

Tasks 9 to 15 give only the fields that differ from Task 8's spec. In each of them, `long_name` and `verb` are the operation's long name, `stage` is `Stage::Shape`, `asks_in` is `AsksIn::QuestionLine`, and `needs` is `&[Need::ProtocolSource, Need::GridCell, Need::ViewRecorded]`.

### Task 8: `hide-column`

```rust
static SPEC: FormSpec = FormSpec { long_name: "hide-column", verb: "hide-column", stage: Stage::Shape, icon: "eye-off",
    asks_in: AsksIn::QuestionLine,
    line: &[Ask { key: "column", words: "exclude ", after: "", seed: Seed::CursorColumn, instead_of: None }],
    opens_on: "column", needs: &[Need::ProtocolSource, Need::GridCell, Need::ViewRecorded],
    name: NameRule { word: "without", from: By::Value("column"), with_by: false } };
```

- Housing, cursor on `house_age`, press `-`: line `exclude house_age`; the preview has the eight other columns; Enter keeps `california_housing_sample_without_house_age`.
- `hide_column_quotes_a_column_named_order`: on `with_nulls`, cursor on `order`; the kept model holds `EXCLUDE ("order")`.
- Refusal: type `nope`. `said()` holds `nope` in DuckDB's words, since arc admits a name and the run finds it missing.

### Task 9: `rename-column`

```rust
line: &[Ask { key: "column", words: "rename ", after: "", seed: Seed::CursorColumn, instead_of: None },
        Ask { key: "to", words: "as ", after: "", seed: Seed::Empty, instead_of: None }],
opens_on: "to", needs: [ProtocolSource, GridCell, ViewRecorded], icon: "cursor-text",
name: NameRule { word: "renamed", from: By::Value("column"), with_by: false }
```

- `with_nulls`, cursor on `median income`, `shift-r`: line `rename median income as`; type `income`; the preview's columns are `city, house_age, order, income`; Enter keeps `with_nulls_renamed_median_income`; the model holds `RENAME COLUMN "median income" TO income`.
- Refusals: `to` typed as one space gives arc's "the column `to` of `rename-column` is empty or blank"; `to` typed as `city` gives DuckDB's words and nothing is kept.

### Task 10: `derive-column`

```rust
line: &[Ask { key: "expression", words: "", after: "", seed: Seed::CursorColumn, instead_of: None },
        Ask { key: "column", words: "as ", after: "", seed: Seed::Empty, instead_of: None }],
opens_on: "expression", needs: [ProtocolSource, GridCell, ViewRecorded], icon: "math-function",
name: NameRule { word: "with", from: By::Value("column"), with_by: false }
```

- Housing, cursor on `median_income`, `=`: line `median_income as`, the expression selected whole. Type `median_income * 2`, Tab, then `double_income`. The preview has ten columns, the last `double_income`, and row 0 reads `1.8604`. Enter keeps `california_housing_sample_with_double_income`.
- Refusal: expression `sum(` gives arc's "the expression `expression` of `derive-column` is not one SQL expression".

### Task 11: `cast-column`

```rust
line: &[Ask { key: "column", words: "cast ", after: "", seed: Seed::CursorColumn, instead_of: None },
        Ask { key: "type", words: "as ", after: "", seed: Seed::Empty, instead_of: None }],
opens_on: "type", needs: [ProtocolSource, GridCell, ViewRecorded], icon: "transform",
name: NameRule { word: "cast", from: By::Value("column"), with_by: false }
```

- Housing, cursor on `house_age`, `shift-t`: line `cast house_age as`; type `VARCHAR`. Enter keeps `california_housing_sample_cast_house_age`. A first page read through the kept step's chain (`preview_chain_reading`, then `preview_first_page`) has `house_age` as a string type (`Utf8` or `Utf8View`).
- Refusal: type `DECIMAL(10` gives arc's "the type `type` of `cast-column` is not one SQL type".

### Task 12: `fill-nulls`

```rust
line: &[Ask { key: "column", words: "coalesce(", after: ",", seed: Seed::CursorColumn, instead_of: None },
        Ask { key: "value", words: "", after: ")", seed: Seed::Empty, instead_of: None }],
opens_on: "value", needs: [ProtocolSource, GridCell, ViewRecorded], icon: "bucket-droplet",
name: NameRule { word: "filled", from: By::Value("column"), with_by: false }
```

- `with_nulls`, cursor on `house_age`, `f`: line `coalesce(house_age,`. Type `0`, and the line reads `coalesce(house_age, 0)`. The preview's Berkeley row reads `0` under `house_age`. Enter keeps `with_nulls_filled_house_age`.
- Refusal: type `1;` gives arc's "holds a `;` outside a string or a comment".

### Task 13: `dedupe-rows`

```rust
line: &[Ask { key: "key", words: "distinct on (", after: ")", seed: Seed::CursorColumn, instead_of: None },
        Ask { key: "order_by", words: "order by ", after: "", seed: Seed::CursorColumn, instead_of: None }],
opens_on: "key", needs: [ProtocolSource, GridCell, ViewRecorded], icon: "copy-off",
name: NameRule { word: "deduped", from: By::Value("key"), with_by: true }
```

- Housing, cursor on `house_age`, `shift-u`: line `distinct on (house_age) order by house_age`. Enter keeps `california_housing_sample_deduped_by_house_age`. Once counted, its rows equal DuckDB's `SELECT count(DISTINCT house_age)` over the copied file, which the test computes with the `duckdb` crate.
- `dedupe_rows_with_both_values_emptied_keeps_distinct_rows`: with both values emptied, the model reads `SELECT DISTINCT *`, and the name is `california_housing_sample_deduped`.
- Refusal: `order_by` emptied with `key` kept gives arc's "the argument `key` of `dedupe-rows` needs `order_by` as well".

### Task 14: `limit-rows`

```rust
line: &[Ask { key: "order_by", words: "order by ", after: "", seed: Seed::CursorColumn, instead_of: None },
        Ask { key: "count", words: "limit ", after: "", seed: Seed::Empty, instead_of: None }],
opens_on: "count", needs: [ProtocolSource, GridCell, ViewRecorded], icon: "list-numbers",
name: NameRule { word: "first", from: By::Value("count"), with_by: false }
```

- Housing, cursor on `house_age`, `shift-l`: line `order by house_age limit`. Type `10`: the preview has 10 rows, `house_age` not descending, and row 0 holds the file's least `house_age`. Enter keeps `california_housing_sample_first_10`.
- Refusals: `ten` gives arc's `is an integer, not "ten"`; `-1` gives arc's "has a minimum of 0, not -1".

### Task 15: `summarise-rows`

```rust
line: &[Ask { key: "aggregates", words: "select ", after: "", seed: Seed::Text("count(*) AS n"), instead_of: None },
        Ask { key: "group_by", words: "group by ", after: "", seed: Seed::CursorColumn, instead_of: None }],
opens_on: "aggregates", needs: [ProtocolSource, GridCell, ViewRecorded], icon: "sum",
name: NameRule { word: "summarised", from: By::Value("group_by"), with_by: true }
```

- Housing, cursor on `house_age`, `shift-f`: line `select count(*) AS n group by house_age`, and Enter alone keeps, as VisiData's frequency table needs no answer. The preview's columns are `house_age, n`. The kept step is `california_housing_sample_summarised_by_house_age`; once counted, its rows equal `count(DISTINCT house_age)` and its `n` sums to 240.
- Refusal: `group_by` typed `2` gives arc's "holds a constant".

### Task 16: `join-tables`

**Waits on** Task 7. Files as every operation task.

```rust
static SPEC: FormSpec = FormSpec { long_name: "join-tables", verb: "join-tables", stage: Stage::Model, icon: "arrows-join",
    asks_in: AsksIn::SpinePick { role: "other" },
    line: &[Ask { key: "kind", words: "", after: "", seed: Seed::Default, instead_of: None },
            Ask { key: "other", words: "join ", after: "", seed: Seed::Empty, instead_of: None },
            Ask { key: "using", words: "using (", after: ")", seed: Seed::CursorColumnIfShared { role: "other" }, instead_of: None },
            Ask { key: "condition", words: "on ", after: "", seed: Seed::Empty, instead_of: Some("using") },
            Ask { key: "prefix", words: "prefix ", after: "", seed: Seed::Empty, instead_of: None }],
    opens_on: "other", needs: &[Need::ProtocolSource, Need::GridCell, Need::ViewRecorded, Need::AnotherTable],
    name: NameRule { word: "joined", from: By::Value("other"), with_by: false } };
```

- Folder: a copy of the housing sample with `by_age` recorded through `record_step` (summarise-rows by `house_age`), then reopened. On the loaded table's grid, cursor on `house_age`, press `&`: the spine picks; Enter on `by_age` gives the line `left join by_age using (house_age)`. The preview adds the column `by_age_n`. Enter keeps `california_housing_sample_joined_by_age`, and its model holds `LEFT JOIN by_age USING (house_age)`.
- `join_tables_a_condition_replaces_the_key`: Tab to `condition` and type `california_housing_sample.house_age = by_age.house_age`. The line drops `using`, and the model holds `ON california_housing_sample.house_age = by_age.house_age`.
- `join_tables_with_no_other_table_says_what_it_needs`: on a fresh copy, `&` gives `said() == Some("Join tables needs a second table in the Protocol.")`.
- Refusal: kind typed `cross` with the key kept gives arc's "exactly one of `using` or `condition` is required for every kind but `cross`".
- `&` is a text event: egui 0.35 has no key for it, and Task 5's `consume_token("&")` reads the `Text("&")` event. The test sends `egui::Event::Text("&".into())`.

### Task 17: `append-rows`

**Waits on** Task 7. Files as every operation task.

```rust
line: &[Ask { key: "other", words: "union all by name ", after: "", seed: Seed::Empty, instead_of: None }],
asks_in: AsksIn::SpinePick { role: "other" }, opens_on: "other", stage: Stage::Model, icon: "row-insert-bottom",
needs: [ProtocolSource, GridCell, ViewRecorded, AnotherTableSharingAColumn],
name: NameRule { word: "appended", from: By::Nothing, with_by: false }
```

- Folder: a copy of the housing sample with `top` (limit-rows 10) and `without_age` (hide-column `house_age`) recorded, then reopened. On the loaded table, `shift-a`; Space on `top`, Space on `without_age`, Enter: the line reads `union all by name top, without_age`. Enter keeps `california_housing_sample_appended`. Once counted it holds 490 rows, and the model holds `UNION ALL BY NAME` twice, `top` first.
- `append_rows_needs_a_table_sharing_a_column`: a Protocol whose only other table is `by_age` (columns `house_age, n`) still shares `house_age`, so the form opens. A Protocol whose other table shares no column name gives "Append rows needs a second table with matching columns." Build it with summarise-rows `count(*) AS n` and no group.

### Task 18: `read-file`

**Waits on** Task 7.

**Files:**
- Modify: `crates/brightfield-shell/src/forms/read_file.rs`; `crates/brightfield-keys/src/operations.rs` (`READ_FILE_BUILT`)
- Modify: `crates/brightfield-engine/src/preview.rs` (`PreviewOp::ReadFile`: the file loaded with its reader before access is shut, as the root is)
- Modify: `crates/brightfield-shell/src/generated.rs` (`chain_of`: a `Generated("read-file")` step whose path is a local CSV or Parquet becomes `PreviewOp::ReadFile`, the path resolved against the Protocol's folder; any other path previews nothing, saying "Brightfield previews a local CSV or Parquet file; Run reads <path>.")
- Modify: `crates/brightfield-shell/src/one_step.rs` (a `read-file` step's lineage model reads its file, so the graph draws the file's node)
- Modify: `crates/brightfield-shell/src/protocol.rs` (the spine's foot row while `read-file` asks: a hollow marker, the line, and the files below it)
- Modify: `crates/brightfield-shell/src/window/form_host.rs` (the `SpineFoot` arm: the line is drawn in the spine's foot row, its field takes the keys, and the Protocol palette's row and the Protocol-context key open it)
- Test: `crates/brightfield-shell/tests/form_read_file.rs` (new); `crates/brightfield-engine/tests/preview_model.rs` (one test added)

```rust
static SPEC: FormSpec = FormSpec { long_name: "read-file", verb: "read-file", stage: Stage::BringIn, icon: "file-import",
    asks_in: AsksIn::SpineFoot,
    line: &[Ask { key: "path", words: "from '", after: "'", seed: Seed::Empty, instead_of: None }],
    opens_on: "path", needs: &[Need::ProtocolSource],
    name: NameRule { word: "", from: By::PathStem("path"), with_by: false } };
```

Interfaces produced: `PreviewOp::ReadFile { path: PathBuf, reader: FileReader }`. The files listed under the row are those in the Protocol's folder that `FileReader::for_path` reads, by name. Typing filters them, and the Down arrow and Enter take one.

- Folder: a copy of the housing sample with `with_nulls.csv` copied beside it. From the spine, press `shift-o`: a new row at the spine's foot reads `from '`, with `with_nulls.csv` listed under it. Type `with_nulls.csv`. The canvas holds a preview of 3 rows and 4 columns. Enter keeps `with_nulls`, and the spine shows its file, step and table with the not-recorded mark. ⌘S records a model holding `FROM 'with_nulls.csv'`. On reopening, the step's grid previews 3 rows.
- `read_file_a_later_step_previews_from_the_second_file`: after the save, hide `city` on `with_nulls`. The preview lacks `city` and has 3 rows, read through `PreviewOp::ReadFile`.
- `read_file_a_url_is_kept_with_arcs_note_and_no_preview`: `https://example.com/x.csv` has `said()` holding arc's "is read live at each run" and the no-preview sentence, and Enter keeps it.
- `read_file_a_taken_name_is_suffixed`: reading a second file whose stem is `california_housing_sample` keeps `california_housing_sample_2`.
- In `preview_model.rs`: `a_second_file_is_read_before_access_is_shut`.

Commit: `git commit -m "Read a file into a new table from a row at the spine's foot, and preview steps that read it"`.

### Task 19: Availability through the registry's declarations, and the fold

**Waits on** every other task, and the chrome's slice 1 (availability declarations).

**Files:**
- Modify: `crates/brightfield-keys/src/operations.rs` (each entry's `needs` in slice 1's form; the operation verbs grouped by stage for slice 1's menus, if its menu model takes them so)
- Modify: slice 1's `Condition`, or its equivalent, gaining the needs the table under "What the forms expect" marks new, each with its sentence
- Modify: `crates/brightfield-shell/src/forms/open.rs` (the facts `FormContext` gathers fill slice 1's `Situation`; `availability` becomes a reading of `MeridianApp::availability_of`, and its own sentences go)
- Modify: `crates/brightfield-shell/src/window/form_host.rs`, `crates/brightfield-shell/src/overlays.rs` (an unavailable form's palette row is greyed with its sentence, as slice 1 greys the rest)
- Modify: `docs/operation-forms.md` (final)
- Delete: `docs/superpowers/plans/2026-11-22-operation-forms.md`
- Test: `crates/brightfield-shell/tests/form_availability.rs` (new)

- [ ] **Step 1: Write the failing tests:**
  - `each_forms_declared_needs_match_what_arc_says_it_reads`: Task 2's check, now made over the registry's `needs`.
  - `a_form_unavailable_here_is_greyed_in_the_palette_with_its_sentence`.
  - `a_forms_key_and_its_palette_row_say_the_same_sentence`.
- [ ] **Step 2: Run them to see them fail.** Run: `cargo test -p brightfield-shell --test form_availability`. Expected: FAIL.
- [ ] **Step 3: Implement**, then delete the forms' own evaluator.
- [ ] **Step 4: Run.** `cargo test -p brightfield-keys`, then `cargo test -p brightfield-shell --test form_availability --test availability --test form_opening --test forms_registry --test overlay_wiring`. Expected: PASS.
- [ ] **Step 5: Fold.** In `docs/operation-forms.md`, make sure each operation's rulings live in its own module doc, and that the calls above say whether the owner kept or overturned each. Delete this plan. The chrome spec stays for its other slices.
- [ ] **Step 6: Commit.** `git commit -m "Declare each operation's needs in the key registry, so the palette, the menus and the ribbon grey a form with one sentence"`, then `--step comment-citations`.

**What arc lacks**, for `docs/operation-forms.md` and as arcform's work:
1. **A render that writes nothing.** The forms record into a throwaway Protocol of stub tables to see a step's model, a child process per pause. A call that returns the model for a request, with arc's refusals and no folder, would retire `draft_model`'s stubs.
2. **A record of a step's operation that a program can read.** A generated step carries only its model and a header arc calls a line "for a person reading the model and not a format a program reads back". Brightfield reads the tables a step reads from that header, which is ambiguous for a name holding `, `. It cannot show a recorded step's values in the panel.
3. **Which column values must already exist.** `x-kind: column` does not say whether a value must be a column of `table` (`hide-column`'s `column`) or is a new name (`rename-column`'s `to`, `derive-column`'s `column`), so each line's seeds say it.
4. **A list item holding a comma.** The command line splits a list on `,`, so a `using` column whose name holds one can be given through `arc mcp` alone.
5. **No stage, and no other tool's word, in a description.** Brightfield holds the stages; the palette's search by another tool's word waits on arc's description carrying those words.
