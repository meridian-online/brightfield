# Finding an operation by its title, its name or another tool's word: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The palette over a grid finds each built operation by its title, its long name, another tool's word or its sentence, ranks the rows in tiers, and starts the chosen operation exactly as its key does. arc answers the same search for an agent.

**Architecture:** The operation catalogue (the landscape repository, `operators/`) gains an entry per arc operation. Its build writes a small public extract, `operations.json`. arc embeds that extract, serves it in `describe`, and holds the one matcher, which it exposes as a library module and as `arc operation search`. Brightfield bumps its arc pin, ranks operation rows by arc's matcher and its other verbs by the same tiers, persists frecency, and reads each operation's title everywhere the window names it.

**Tech Stack:** Python 3 under `uv`, with pytest and ruff (landscape); Rust (arcform and Brightfield); egui and kittest snapshots (Brightfield).

**Spec:** `docs/superpowers/specs/2026-11-26-operation-words-design.md` on this branch. Read it before any task: the tiers, the alias rules and the thirteen titles are defined there.

## Global Constraints

- The tiers, highest first: (1) exact title or long name, case-folded; (2) a prefix of the title or long name, or of any word in either, with long names split at hyphens; (3) an exact whole alias; (4) a prefix of any word inside an alias; (5) a fuzzy subsequence match on the title or long name; (6) a match in the sentence or help. Aliases never match fuzzily.
- Within a tier, rows order by score, then frecency, then frequency, then name. Frecency never moves a row across tiers.
- An alias carries a `source` only for these named tools: DuckDB, Polars, Ibis, DataFusion, dbt, SQLMesh, OpenRefine, VisiData, KNIME, Apache Hop, Apache NiFi, subtoken, mlpack and YDF, or for the spec `SQL`. Every other tool's word has no source.
- No proprietary tool's name, no URL and no unpublished class reaches arc's source or output.
- The row detail reads `also known as “<word>”` for an alias with no source, `<word> in <source>` for an alias with a source, the long name for a long-name match, and the sentence for a title or sentence match. The quotes are typographic.
- The thirteen titles are fixed by the spec's catalogue section. Do not reword them.
- Long names are still looked up exactly when an operation is recorded. Aliases are for finding an operation only, and no alias is ever recorded.
- Brightfield and arcform are public repositories. Keep planning identifiers, task numbers and private repository names out of code, comments, commits, branch names and pull request text. Refer to the catalogue as "Meridian's operation catalogue".
- Brightfield's gate is `cargo xtask ci`, and a pull request's gate is `cargo xtask gate <pr>` on the Mac Studio, which takes about 51 minutes on an idle machine. Before each commit run `cargo xtask ci --step fmt`, `--step clippy` and the tests the change touches. After each commit run `--step comment-citations`.
- Pixel baselines are redrawn only on the Mac Studio. Never redraw one from a hosted run.

## Review Focus

1. **Case, spacing and apostrophes in a query.** "Keep only ", "KEEP ONLY", "column's" and "column’s" all match as their trimmed, case-folded, apostrophe-folded forms. Pinned in Task 4 and Task 6.
2. **The palette before arc's description arrives, or with no arc at all.** Operation rows still read their titles and match their aliases from the embedded extract. The order does not jump when the description comes in. Pinned in Task 8.
3. **An unbuilt operation's word.** "Remove Duplicates" lists no row while dedupe-rows is not built in the window, and does list it once its key is built. Pinned in Task 8.
4. **A frecency file that is missing, empty, corrupt or from a later format.** The palette starts with empty frecency, says nothing and overwrites the file on the next pick. Pinned in Task 7.
5. **A word several operations share, or an alias that equals another operation's title word.** Each operation is listed once, at its own best tier, in a deterministic order. Pinned in Task 4 and Task 9.

---

### Task 1: The catalogue holds an entry per arc operation (landscape)

**Files:**
- Modify: `operators/vocabulary.yaml` (new top-level `operations:` list)
- Modify: `operators/build.py`: `read_vocabulary`, and a new `validate_operations(vocab: dict) -> list[str]` called where `validate_v3` is
- Test: `tests/test_operations_vocabulary.py`

**Interfaces:**
- Produces: a vocabulary `operations` list of `{long_name: str, title: str, sentence: str, serves: list[str]}`, where each `serves` item is a variant id, or a class id only when the operation performs every one of that class's variants.

- [ ] **Step 1: Write the failing tests**

```python
def test_every_arc_operation_has_one_entry():
    ops = read_vocabulary()["operations"]
    assert sorted(o["long_name"] for o in ops) == sorted(ARC_OPERATIONS)  # the 13 long names, listed in the test

def test_titles_are_the_approved_thirteen():
    titles = {o["long_name"]: o["title"] for o in read_vocabulary()["operations"]}
    assert titles["derive-column"] == "Add a calculated column"
    assert titles["cast-column"] == "Change a column's type"
    assert titles["fill-nulls"] == "Fill in missing values"
    # ...one assertion for each of the 13, copied from the spec

def test_serves_names_real_variants_only():
    assert validate_operations({"operations": [{"long_name": "fill-nulls", "title": "t", "sentence": "s.", "serves": ["shape.columns.nulls.nope"]}], **CLASSES}) == ["fill-nulls serves unknown id shape.columns.nulls.nope"]

def test_fill_nulls_does_not_serve_set_null():
    serves = next(o for o in read_vocabulary()["operations"] if o["long_name"] == "fill-nulls")["serves"]
    assert "shape.columns.nulls.set_null" not in serves and "shape.columns.nulls" not in serves

def test_all_in_one_is_served_by_hide_rename_and_cast():
    servers = [o["long_name"] for o in read_vocabulary()["operations"] if "shape.columns.select.all_in_one" in o["serves"]]
    assert sorted(servers) == ["cast-column", "hide-column", "rename-column"]
```

Also test that each `sentence` ends with a full stop and is one sentence.

- [ ] **Step 2: Run them to see them fail.** Run `uv run pytest -q tests/test_operations_vocabulary.py`. The expected result is FAIL with a `KeyError: 'operations'`.

- [ ] **Step 3: Write the 13 entries and `validate_operations`.** `serves` lists only the variants each operation performs. Read each variant's `meaning` in `vocabulary.yaml` to decide, and do not serve a whole class unless every variant fits. Starting points:
  - filter-rows: `keep_matching`, `chosen_values`, `contains_text`.
  - sort-rows: `by_columns`, `reverse`.
  - hide-column: `shape.columns.select.keep_or_drop`, `all_in_one`.
  - rename-column: `rename`, `all_in_one`.
  - derive-column: `add_expression`, `constant`, `copy`.
  - cast-column: `convert`, `all_in_one`.
  - fill-nulls: `constant`, `coalesce`.
  - The rest map to `shape.table.aggregate`, `shape.rows.dedupe`, `shape.rows.limit`, `join.keyed`, `join.union` and `acquire.file.*`, at variant level.

  Each sentence is plain and short, and says what the analyst gets.

- [ ] **Step 4: Run them to see them pass.** Run `uv run pytest -q tests/test_operations_vocabulary.py`, then `make test` (ruff plus pytest, under a minute).

- [ ] **Step 5: Commit.** Commit with the message "Give each arc operation a title, a sentence and the variants it serves". The pull request description lists the 13 `serves` lists so the owner can review them.

### Task 2: The catalogue writes a public extract of the operations' words (landscape)

**Files:**
- Modify: `operators/vocabulary.yaml` (new `tools:` table of `{id, name, named: bool}` for all 27 tools, and a `specs:` table holding `{id: sql, name: SQL}`)
- Modify: `operators/build.py`: new `operation_aliases(op: dict, classes: list[dict], tools: dict, rules: dict) -> list[dict]`, and new `export_operations(root, commit) -> dict`, written to `operators/operations.json` beside `catalogue.json` by the same commands (`build.py` and `--restamp`). `export_catalogue` also adds the `operations` list to `catalogue.json`.
- Create: `operators/alias_rules.yaml` (words to add per operation, including the SQL spec words, and crosswalk names to drop as not being words)
- Test: `tests/test_operations_export.py`

**Interfaces:**
- Consumes: Task 1's `operations` list.
- Produces: the shape of `operations.json`: `{"version": 1, "landscape_commit": str, "operations": [{"long_name", "title", "sentence", "aliases": [{"word": str, "source"?: str}]}]}`. Aliases are sorted case-folded, with no two aliases of one operation equal when case-folded.

- [ ] **Step 1: Write the failing tests**

```python
def test_a_proprietary_tools_word_has_no_source():
    fill = op("filter-rows")
    keep = [a for a in fill["aliases"] if a["word"] == "Keep Only"]
    assert keep == [{"word": "Keep Only"}]

def test_an_open_source_word_names_its_tool():
    assert {"word": "fill_null", "source": "Polars"} in op("fill-nulls")["aliases"]

def test_sql_words_name_the_spec():
    assert {"word": "WHERE", "source": "SQL"} in op("filter-rows")["aliases"]

def test_the_extract_holds_no_proprietary_name_or_url():
    text = (OPERATORS / "operations.json").read_text().lower()
    for banned in ("alteryx", "tableau", "power query", "powerquery", "trifacta", "http"):
        assert banned not in text

def test_a_word_shared_by_tools_is_one_alias_named_if_any_tool_may_be_named():
    ...  # build from a fixture of two crosswalk rows, "Filter" from alteryx and from polars -> [{"word": "Filter", "source": "Polars"}]

def test_null_if_is_not_an_alias_of_fill_nulls():
    assert all(a["word"].lower() != "null if" for a in op("fill-nulls")["aliases"])
```

- [ ] **Step 2: Run them to see them fail.** Run `uv run pytest -q tests/test_operations_export.py`. The expected result is FAIL with `operations.json` missing.

- [ ] **Step 3: Implement `operation_aliases` and `export_operations`.**
  - An operation's words are the crosswalk rows whose `variant` it serves, plus rows of a class it serves whole.
  - Group the words case-folded and keep the first spelling.
  - The source is the display name of the first named tool using the word, else none.
  - Then apply `alias_rules.yaml`.
  - Add a check that fails the build when a word in `operations.json` contains a proprietary tool's id or name.

- [ ] **Step 4: Run them to see them pass.** Run `uv run pytest -q tests/test_operations_export.py`, then `make test`, then `uv run python operators/build.py` to regenerate both files.

- [ ] **Step 5: Commit.** Commit with the message "Write operations.json, the operations' titles and aliases with no proprietary name". After merge, run `--restamp` on main as the README says.

### Task 3: arc embeds the extract and describes each operation's title, sentence and aliases (arcform)

**Files:**
- Create: `operations.json` at arcform's repository root, copied from Meridian's operation catalogue at the merge commit of Task 2
- Create: `src/words.rs`, a public module, compiled with no feature (so `default-features = false` builds have it)
- Modify: `src/lib.rs` (`pub mod words;`), `src/record.rs` (`listing()` and `description()` add `title`, `sentence` and `aliases`), `src/mcp/hero.rs` (the `operation_describe` output carries the same fields)
- Test: unit tests in `src/words.rs`, plus the existing describe tests in `src/cli.rs`

**Interfaces:**
- Produces:
  - `pub struct Alias { pub word: String, pub source: Option<String> }`
  - `pub struct OperationWords { pub long_name: String, pub title: String, pub sentence: String, pub aliases: Vec<Alias> }`
  - `pub fn operations() -> &'static [OperationWords]`, parsed once with `std::sync::LazyLock` from `include_str!("../operations.json")`
  - `pub fn of(long_name: &str) -> Option<&'static OperationWords>`
  - `pub fn landscape_commit() -> &'static str`

- [ ] **Step 1: Write the failing tests.**
  - `every_catalogue_operation_has_words_and_every_entry_an_operation`: for each `crate::record::operations()` entry, `of(long_name)` is `Some`, and the counts are equal.
  - `describe_carries_title_sentence_and_aliases`: `crate::record::describe("fill-nulls")` has `"title": "Fill in missing values"`, and `aliases` contains `{"word": "fill_null", "source": "Polars"}`.
  - `list_json_carries_the_title`: `operation_list(true, …)` output's `fill-nulls` entry has its title.

- [ ] **Step 2: Run them to see them fail.** Run `cargo test words:: operation_describe operation_list`. The expected result is FAIL with no module `words`.

- [ ] **Step 3: Implement `src/words.rs` and the description fields.** A malformed embedded file panics in a test, never at a user's run: the parse is checked by the first test.

- [ ] **Step 4: Run them to see them pass.** Run `cargo test`, the full arcform suite, which takes several minutes.

- [ ] **Step 5: Commit.** Commit with the message "Describe each operation's title, sentence and the words other tools use for it". The commit body names the catalogue commit the file was copied from.

### Task 4: arc's matcher ranks operations in six tiers (arcform)

**Files:**
- Modify: `src/words.rs`
- Test: unit tests in `src/words.rs`

**Interfaces:**
- Consumes: Task 3's `OperationWords` and `Alias`.
- Produces:
  - `pub enum Matched { Title, LongName, Alias { word: String, source: Option<String> }, Sentence }`
  - `pub struct Rank { pub tier: u8, pub score: i32, pub matched: Matched }`, with tier 1 the highest
  - `pub struct Candidate<'a> { pub title: Option<&'a str>, pub long_name: &'a str, pub aliases: &'a [Alias], pub sentence: &'a str }`
  - `pub fn rank(query: &str, candidate: &Candidate) -> Option<Rank>`, which returns `None` for no match and the best tier when several apply
  - `pub fn normalise(query: &str) -> String`, which trims, case-folds, folds `’` to `'`, and collapses runs of whitespace
  - `pub struct Hit { pub long_name: &'static str, pub title: &'static str, pub rank: Rank }`
  - `pub fn search(query: &str) -> Vec<Hit>`, sorted by tier, then score descending, then long name

- [ ] **Step 1: Write the failing table test** `ranks_follow_the_six_tiers`, with these rows (query, long name, expected tier, expected matched):

| query | operation | tier | matched |
|---|---|---|---|
| `Fill in missing values` | fill-nulls | 1 | Title |
| `fill-nulls` | fill-nulls | 1 | LongName |
| `derive` | derive-column | 2 | LongName |
| `column` | hide-column | 2 | LongName |
| `filter` | filter-rows | 2 | Title |
| `keep only` | filter-rows | 3 | Alias "Keep Only", no source |
| `kee` | filter-rows | 4 | Alias "Keep Only" |
| `fltr` | filter-rows | 5 | LongName |
| `each missing value` | fill-nulls | 6 | Sentence |

  Further tests:
  - `aliases_never_match_fuzzily`: `rank("kp nly", filter-rows)` is not tier 3 or 4.
  - `peek_does_not_find_keep`: no hit for `eek` on an alias.
  - `case_and_apostrophes_fold`: `rank("CHANGE A COLUMN’S TYPE ", cast-column)` is tier 1.
  - `filter_lists_filter_rows_before_any_alias_named_filter`: in `search("filter")`, filter-rows comes before every hit at tier 3 or below.
  - `a_word_shared_by_operations_lists_each_once`: a fixture of two candidates sharing the alias "Select" gives two hits.

- [ ] **Step 2: Run them to see them fail.** Run `cargo test words::`. The expected result is FAIL with `rank` undefined.

- [ ] **Step 3: Implement `rank` and `search`.** Tier 5 uses a subsequence scorer in this module, which rewards consecutive runs and the starts of words. Tier 6 is a case-folded substring match on the sentence.

- [ ] **Step 4: Run them to see them pass.** Run `cargo test words::`.

- [ ] **Step 5: Commit.** Commit with the message "Rank operations by title, long name, alias and sentence, in six tiers".

### Task 5: `arc operation search` and the MCP tool `operation_search` (arcform)

**Files:**
- Modify: `src/cli.rs` (`OperationCmd::Search { text: String, json: bool }`, and `fn operation_search(text: &str, json: bool, out: &mut impl Write) -> Result<()>`), `src/mcp/hero.rs` (tool `operation_search`, with input `{text: string}` and the same JSON)
- Test: `src/cli.rs` tests, and the MCP tool tests beside `operation_describe`'s

**Interfaces:**
- Consumes: Task 4's `search`.
- Produces: the JSON form, an array of `{long_name, title, tier, matched: "title"|"long_name"|"alias"|"sentence", word?, source?}`. The text form prints one line per hit: the title, two spaces, then the detail as the palette words it.

- [ ] **Step 1: Write the failing tests.**
  - `operation_search_names_how_each_hit_matched`: `--json` for `keep only` holds `{"long_name":"filter-rows","title":"Filter rows","tier":3,"matched":"alias","word":"Keep Only"}` and no `source` key.
  - `operation_search_text_reads_as_the_palette`: the text form for `fill_null` prints `Fill in missing values  fill_null in Polars`.
  - `operation_search_with_no_hit_prints_nothing_and_succeeds`.
  - An MCP test that the tool lists, and that its answer equals the CLI's JSON.

- [ ] **Step 2: Run them to see them fail.** Run `cargo test operation_search`.

- [ ] **Step 3: Implement both.** Follow `operation_list`'s handling of a failed write.

- [ ] **Step 4: Run them to see them pass.** Run `cargo test`.

- [ ] **Step 5: Commit, push, and open the arcform pull request for Tasks 3 to 5.** Commit with the message "arc operation search answers with each operation a text finds and how". The merge commit is the rev Task 6 pins.

### Task 6: Brightfield pins arc with the words (brightfield)

**Files:**
- Modify: `Cargo.toml:58` (`rev`), `Cargo.lock`, following README §"The arcform dependency"
- Modify: anything the bump breaks, fixed in this task
- Test: `crates/brightfield-shell/tests/operation_descriptions.rs` (extend)

**Interfaces:**
- Produces: `arc::words` available to `brightfield-shell`. `OperationDescription` (`crates/brightfield-shell/src/operation.rs:65`) gains `title: String`, `sentence: String` and `aliases: Vec<arc::words::Alias>`, parsed in `describe` (`:302`).

- [ ] **Step 1: Write the failing test** `a_description_carries_the_title_and_aliases`: describing `fill-nulls` through the runner gives the title "Fill in missing values" and an alias `fill_null` from Polars.
- [ ] **Step 2: Bump the rev and the lock, and run the test to see it fail.** Run `cargo test -p brightfield-shell --test operation_descriptions`. The expected result is FAIL with the fields missing.
- [ ] **Step 3: Parse the fields, and fix any fallout from arc's other changes since the old pin.**
- [ ] **Step 4: Run the gate.** Run `cargo xtask ci`, which takes about 51 minutes on the Mac Studio. The expected result is every step green.
- [ ] **Step 5: Commit.** Commit with the message "Pin arc at the commit that describes each operation's words".

### Task 7: Frecency per operation, kept between sessions (brightfield)

**Files:**
- Modify: `crates/brightfield-keys/src/palette.rs`, replacing `RecencyCounter` with `Frecency`
- Create: `crates/brightfield-model/src/palette_history_file.rs`, framework-free, saved under the user config dir as `dock_state_file.rs` does
- Modify: `crates/brightfield-shell/src/window.rs` (the `recency` field at `:4019` and its construction at `:4537`; load at start and save after each pick), `crates/brightfield-shell/src/gallery.rs:1279`
- Test: unit tests in both new and changed files

**Interfaces:**
- Produces:
  - `pub struct Frecency` with `pub fn new() -> Self`
  - `pub fn record(&mut self, longname: &str, now_secs: u64)`
  - `pub fn weight(&self, longname: &str, now_secs: u64) -> u64`, which is the count times a recency factor halving every 14 days; higher means used more
  - `pub fn to_json(&self) -> String` and `pub fn from_json(text: &str) -> Option<Self>`
  - The cap: only the newest 1,000 uses are kept
  - In `brightfield-model`: `pub fn load_palette_history(dir: &Path) -> Frecency` (empty on any failure) and `pub fn save_palette_history(dir: &Path, f: &Frecency) -> io::Result<()>`

- [ ] **Step 1: Write the failing tests.**
  - `frecency_prefers_recent_and_frequent`.
  - `frecency_keeps_only_the_newest_1000_uses`.
  - `a_corrupt_missing_or_future_history_loads_empty`: the cases are `""`, `"{"`, `{"version":99}` and a missing file.
  - `frecency_round_trips`.

- [ ] **Step 2: Run them to see them fail.** Run `cargo test -p brightfield-keys palette` and `cargo test -p brightfield-model palette_history`.
- [ ] **Step 3: Implement both, and wire the window.** The empty query's order inside each stage uses `weight`, as recency was used before.
- [ ] **Step 4: Run them to see them pass.** Run the same two commands, plus `cargo test -p brightfield-shell --test overlay_wiring`.
- [ ] **Step 5: Commit.** Commit with the message "Keep the palette's frecency per command between sessions".

### Task 8: The palette ranks operation rows by arc's matcher and says how each matched (brightfield)

**Files:**
- Modify: `crates/brightfield-keys/src/palette.rs`: `PaletteCandidate` gains `tier: u8`, `title: Option<String>` and `detail: Option<String>`; `palette_filter` ranks every verb on tiers 1, 2, 5 and 6, via `pub fn rank_verb(query: &str, longname: &str, help: &str) -> Option<(u8, i32)>`, and sorts by tier, score, frecency, frequency, name
- Modify: `crates/brightfield-shell/src/overlays.rs`: `CommandPalette` re-ranks each operation verb's candidate through `arc::words::rank`, using the operation's long name (filter-rows and sort-rows map from their `-at-cursor` verbs through `operation_ui::by_verb`), and matches unlisted operations' aliases too. `row()` leads with the title and sets the detail per the Global Constraints.
- Test: `crates/brightfield-shell/tests/palette_words.rs` (new), and `rank_verb` unit tests in `brightfield-keys`

**Interfaces:**
- Consumes: Task 4's `arc::words::{rank, of, Candidate, Rank, Matched}`, and Task 7's `Frecency`.
- Produces: `pub fn operation_detail(matched: &Matched, long_name: &str, sentence: &str) -> String` in `overlays.rs`, which Task 9 reuses.

- [ ] **Step 1: Write the failing tests.**
  - `keep_only_lists_filter_rows_under_its_title`: the first row reads `Filter rows`, with the detail `also known as “Keep Only”`.
  - `fill_null_names_polars`: the detail is `fill_null in Polars`.
  - `derive_finds_add_a_calculated_column_with_its_long_name`.
  - `an_unbuilt_operations_word_lists_nothing`: "Remove Duplicates" gives no dedupe row while `DEDUPE_ROWS_BUILT` is false.
  - `rows_read_titles_before_the_description_arrives`: with the runner absent, the rows still read titles and the order equals the order once the description is in.
  - `the_verbs_matcher_agrees_with_arcs_on_tiers`: for every registry verb and each of 20 fixed queries, `rank_verb`'s tier equals `arc::words::rank`'s with no title, no aliases and the help as the sentence.

- [ ] **Step 2: Run them to see them fail.** Run `cargo test -p brightfield-shell --test palette_words`.
- [ ] **Step 3: Implement as the Files block says.** Read titles and aliases from `arc::words::of`, which is embedded, so the palette never waits on the runner.
- [ ] **Step 4: Run them to see them pass, and redraw the snapshots.** Run that test, `cargo test -p brightfield-keys`, and `cargo test -p brightfield-shell --test overlay_wiring --test typed_sql_palette`. If `shell_palette_open_*.png` changes, redraw it on the Mac Studio.
- [ ] **Step 5: Commit.** Commit with the message "The palette finds an operation by its title, long name or another tool's word, and says which".

### Task 9: Choosing a row found by a word records the key's step, byte for byte (brightfield)

**Files:**
- Create: `crates/brightfield-shell/tests/operation_from_a_word.rs`, modelled on `tests/operation_from_the_cursor.rs` and `tests/support/operations.rs`

**Interfaces:**
- Consumes: Task 8's palette and the `OpWindow` harness.

- [ ] **Step 1: Write the tests.**
  - `a_word_typed_in_the_palette_records_the_keys_step`: open a fixture's grid with the cursor on a cell, open the palette and type `keep only`. Assert that the first row is `Filter rows` with its alias detail. Confirm it, take the foot row's suggested condition with Enter, and Save. Then do the same from a second copy of the fixture through the `w` key. Assert the two step files are byte-identical.
  - `a_word_for_fill_records_the_fill_keys_step`: the same, with `fill_null`, on a column with missing values, against the `f` key.
  - `a_word_several_operations_share_lists_each_under_its_own_title`: an alias that hide-column and rename-column share, both from the `all_in_one` variant, lists both rows, each once.
- [ ] **Step 2: Run them.** Run `cargo test -p brightfield-shell --test operation_from_a_word`. They pass if Task 8 is right; a failure here is a bug in Task 8, fixed there.
- [ ] **Step 3: Commit.** Commit with the message "Test that an operation found by another tool's word records its key's step".

### Task 10: One title everywhere the window names an operation (brightfield)

**Files:**
- Modify: each site that names an operation from its `label` or from `operation_ui::label(long_name)`. Read the title through a new `pub fn title(long_name: &str) -> String` in `crates/brightfield-shell/src/operation_ui/mod.rs:497`, replacing `label`, which returns `arc::words::of(long_name)`'s title or, for an unknown name, today's words. The sites are `operation_ui/header.rs`, `operation_panel.rs`, `operation_ui/open.rs`, `window/operation_host.rs` and the ribbon's operation rows. The ribbon reads `brightfield_keys::operations`' `label`, which is replaced by the title at the shell's edge.
- Test: `crates/brightfield-shell/tests/operation_registry.rs` (extend), plus existing header and panel tests updated to the new wording

- [ ] **Step 1: Write the failing test.** `every_place_names_an_operation_by_its_title`: for each built operation, the header menu row, the inspector section heading, the ribbon entry and the status band's "needs" sentence contain `title(long_name)`. Check `derive-column` explicitly against "Add a calculated column".
- [ ] **Step 2: Run it to see it fail.** Run `cargo test -p brightfield-shell --test operation_registry`.
- [ ] **Step 3: Replace `label` with `title` at each site, and update the expected strings in the existing tests.**
- [ ] **Step 4: Run them to see them pass, and redraw the snapshots.** Run `cargo test -p brightfield-shell`, which takes several minutes. Redraw any changed header, menu or inspector snapshot on the Mac Studio.
- [ ] **Step 5: Commit.** Commit with the message "Name each operation by its catalogue title throughout the window".

### Task 11: With an empty query, the cursor's column raises the operations that fit it (brightfield, optional)

This task can be dropped without touching the others.

**Files:**
- Modify: `crates/brightfield-shell/src/overlays.rs` (`CommandPalette::with_situation` gains the cursor column's missing count, and its FineType type against its stored type)
- Test: `crates/brightfield-shell/tests/palette_words.rs`

- [ ] **Step 1: Write the failing tests.**
  - `a_column_with_missing_values_puts_fill_first_in_its_stage`.
  - `a_column_whose_detected_type_differs_puts_cast_first_in_its_stage`.
  - `with_no_cursor_the_empty_query_is_unchanged`.
- [ ] **Step 2: Run them to see them fail.** Run `cargo test -p brightfield-shell --test palette_words`.
- [ ] **Step 3: Implement.** A fitting operation moves to the head of its own stage's rows only. Stage order stays as it is.
- [ ] **Step 4: Run them to see them pass.**
- [ ] **Step 5: Commit.** Commit with the message "With an empty query, the palette raises the operations that fit the cursor's column".

### Task 12: Fold the design into the docs and make the branch ready (brightfield)

**Files:**
- Create: `docs/palette-search.md` (the tiers, the alias rules and their reasons, the row's detail, frecency, where the words come from, and the owner's rulings in plain words with their reasons)
- Modify: `docs/operation-forms.md` (in "What arc lacks", item 6 now covers only stage), and the doc list in `CLAUDE.md` (one line saying when to read `docs/palette-search.md`)
- Delete: `docs/superpowers/specs/2026-11-26-operation-words-design.md` and this plan

- [ ] **Step 1: Write the docs.** Run `cargo xtask ci --step comment-citations` after committing.
- [ ] **Step 2: Run the full gate.** Run `cargo xtask gate <pr>` on the Mac Studio, which takes about 51 minutes. The expected result is a green `local-gate` status.
- [ ] **Step 3: Run the branch review.** Run `roborev review --branch --wait`, fix every finding at Medium and above, and close the review.
- [ ] **Step 4: Link the pull request to its issue in Forge,** and put the gate's test totals and the rulings in its description.
