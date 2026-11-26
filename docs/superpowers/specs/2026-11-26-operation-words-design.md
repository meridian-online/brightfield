# Finding an operation by its title, its name or another tool's word

## Intent

An analyst who has never used Brightfield knows what they want to do to a table, and often the word another tool uses for it, but not what Brightfield calls it. With a table's grid open, they type into the palette and find the operation. They can type its title ("Fill in missing values"), its long name (`fill-nulls`), a shorthand, or a word another tool uses ("Keep Only", `fill_null`). Choosing the row starts the operation exactly as its key does, and the step it records is the same, byte for byte. Each row shows the operation's key, so the palette teaches the shortcut an analyst will use once they know the operation.

Brightfield's own taxonomy is what the analyst learns. Other tools' words are a way in, not a second vocabulary. The owner's words: "These tools provide good reference but nothing is binding us to those taxonomies. The most important thing in our taxonomy is that it's intuitive, coherent and has consistent naming conventions."

An agent searching through arc finds what the palette finds, because both read the same words from the same catalogue.

## Goals

1. The palette over a grid finds each built operation by its title, its arc long name, a fuzzy spelling of either, its help sentence, or any of its aliases.
2. A row leads with the operation's title. Its muted detail says how it matched:
   - the operation's sentence, for a match on the title or the sentence;
   - the long name, for a match on the long name;
   - the alias, for a match on an alias, with its source named only when the source may be named (see "Aliases and their sources").
3. Results are ranked in tiers, with frecency only as a tiebreak inside a tier.
4. Choosing a row starts the operation through the same entry point as its key. The step it records is byte-identical to the key's for the same target and arguments, and a test drives this end to end.
5. arc serves each operation's title and aliases in `arc operation describe`. A new `arc operation search <text>`, on the command line and through the MCP server, answers with the same matches and how each one matched.
6. The titles and aliases come from Meridian's one operation catalogue, `operators/catalogue.json` in the landscape repository. That is the catalogue the public operation pages are built from.

## Non-goals

- **No "not built yet" state, and no new operations.** This work builds no operation. It ships with the operations the window has when it merges, and each operation built later joins the word search with no extra work: its catalogue entry already exists, and its row is listed once its key is built. A word for an operation the window does not have yet matches nothing. Building the rest is separate work, which the launch waits on: every operation class in scope built in arc and the window, short of any that cannot be built because a service is out of reach or a package is not in a registry.
- **No operators.** The palette lists operations only. When operators are listed later, they come from the same catalogue.
- The word is never recorded. Once a row is chosen, only the operation travels: the step, the panel and the history say its long name.
- The palette's layout, its card and where it opens stay as they are.
- Spreadsheet formulas and features (Excel, Google Sheets, Flash Fill) are their own brainstorm. Most spreadsheet formulas are functions used inside an expression, not operations.
- No learning per query. Only one open-source launcher read (Kupfer) ranks by the query that led to a choice. Raycast claims it, but its code cannot be read.

## Anti-goals

These would make it a failure even if it technically works:

- **Synonyms flood the list.** Typing three letters must not list a dozen rows matched through stray letters in other tools' words. Aliases match by whole words and word prefixes only, never fuzzily.
- **Usage outranks a correct match.** A row the analyst uses often must not push a row they named exactly down the list, as Zed and VS Code do. Frecency only orders rows that matched equally well.
- **Closed-source tools are named.** A proprietary tool is not named beside its word, in the palette, in arc's output or in the docs.
- **The title and the long name drift.** Each operation has one title, held in one place, which the window, arc and the public pages all read.

## Rulings this design follows

- **The owner, 2026-11-26, on names:** "It depends on the use case and the user. Some analysts may learn to use kebab case or shorthand - I became very used to pandas methods for instance. It's more likely that analysts will search by title, and become familiar with a keyboard shortcut or chord." So the row shows the title, the long name ranks as high as the title, and the key is on every row.
- **The owner, 2026-11-26, on aliases:** "We don't need to be so explicit - particularly with closed source or proprietary tools, we should simplify this into just an 'alias' or 'otherwise known as' array without the tool names. Exceptions to this are for broadly known, open source tools we can reference them by name - or if there's a canonical spec to reference like finetype has it." This replaces the 29 September ruling that a row names the word and its tool. Of that ruling, what still stands: a row found by a word is listed under the operation's own name, and the word travels no further.
- **The owner, 2026-11-26, on unbuilt operations:** the launch waits until "all the operators are built in arcform and supported in brightfield", with exceptions only for one that cannot be built because a service is out of reach or a package is not in a registry.
- **The owner, 2026-11-02:** the operation catalogue lives in the landscape repository, which the public pages read now and arc adopts later. There is one catalogue for operations and operators, never two.
- **An earlier ruling:** the words are held in arc's description, and the build holds no words. Brightfield gets them from arc at runtime, not from a copy of its own.
- **The 29 September ruling, still standing:** a filter or a sort chosen from the palette is asked in the row at the grid's foot, as its key asks. Since then each other operation has gained its own place, and a palette row starts it there through the same entry point as its key.

## Aliases and their sources

Each operation carries an alias list: words other tools use for it, held without the tool's name. An alias may also carry a source. It does so only when the source is one of these:

- **A well-known open-source tool or library**, which may be named: DuckDB, Polars, Ibis, DataFusion, dbt, SQLMesh, OpenRefine, VisiData, KNIME, Apache Hop, Apache NiFi, subtoken, mlpack, YDF. pandas and dplyr join them once the inventory lists them.
- **A canonical spec.** For operations this is the SQL standard (ISO/IEC 9075). It is named as "SQL" and covers `WHERE`, `ORDER BY`, `GROUP BY` and the like. DuckDB's `PIVOT` is named as DuckDB's, since it extends the standard.

Every other tool's word is an alias with no source: Alteryx, Power Query, Tableau Prep, Trifacta, Azure Data Factory, AWS Glue, EasyMorph, Talend, Pentaho, RapidMiner, Bruin, duckle and SeaTunnel.

A row's detail reads `also known as "Keep Only"` for an alias with no source, `fill_null in Polars` for an open-source tool, and `WHERE in SQL` for the spec. The quotes are typographic. A word that several tools use is one alias, compared case-folded, and it names a source when any of those tools may be named.

## What the analyst sees

The palette over a grid, typing:

```
› keep only
  Filter rows        also known as "Keep Only"                w
  Hide a column      also known as "Keep Only Field"          -
› fill
  Fill in missing values    Fill each missing value in a column   f
› fill_null
  Fill in missing values    fill_null in Polars               f
› fill-n
  Fill in missing values    fill-nulls                        f
› where
  Filter rows        WHERE in SQL                             w
```

On an empty query, the rows read under the ribbon's stage headings, as they do today.

## Ranking

A row's tier is the best of its matches. A lower number ranks higher, and at each degree of match our own names rank above an alias:

1. An exact match, case-folded, on the title or the long name.
2. A prefix of the title or the long name, or of any word in the title. So "filter" lists "Filter rows" before any operation that another tool calls "Filter".
3. An exact match, case-folded, on a whole alias.
4. A prefix of any word inside an alias. So "kee" finds "Keep Only", but not "Peek".
5. A fuzzy match on the title or the long name, by the palette's existing subsequence scorer.
6. A match in the operation's sentence or the window's help text.

Aliases never match fuzzily. A verb that is not an operation has no title and no aliases, so it matches on its long name in tiers 1, 2 and 5 and on its help in tier 6, as it does now. Inside a tier, rows are ordered by the existing score, then by frecency, then by the current order. Frecency is kept per operation, as a count and a last-used time. It is kept between sessions and capped, as Zed (1,000 uses) and VS Code (50 commands) cap theirs. It is not kept per query.

This follows what the code of real palettes does: a name outranks a synonym (JetBrains' Find Action by match mode, Kupfer at 0.95 of the name's score, Ulauncher at 0.6), and exact and prefix matches outrank fuzzy ones. The research is summarised on the issue this work belongs to.

**A separate step, which can be dropped:** with an empty query, the column under the grid's cursor raises the operations that fit it. A column with missing values raises fill-nulls, and a column whose detected type differs from its stored type raises cast-column. Tableau Prep's recommendations and Dataiku's column menu do this. It is the last task of the plan, so it can be cut without touching the rest.

## Approaches considered

1. **The catalogue holds the words, arc embeds them, and Brightfield reads arc. Chosen.** The catalogue gains each operation's title, its link to the class or variant it serves, its aliases and their sources. arc embeds a copy of the catalogue file, with the catalogue commit it was taken from, and serves it through `describe` and a new `search`. Brightfield reads the description once per session and ranks the rows itself. This keeps the one-catalogue ruling, the ruling that the build holds no words, and the rule that an agent finds what the palette finds. The cost is three repositories changing in order: the catalogue, then arc, then Brightfield's pin of arc.
2. **Words written by hand in arc's source.** This starts faster, but it makes a second catalogue that drifts from the one the public pages read. The 2 November ruling turned that down.
3. **Brightfield reads the catalogue directly.** This skips arc, but an agent searching arc would then find nothing the palette finds, and the build would hold words.

## The design

### The catalogue (landscape repository)

`operators/vocabulary.yaml` gains an `operations` list, one entry per arc operation, written by hand:

```yaml
- long_name: fill-nulls
  title: Fill in missing values
  sentence: Fill each missing value in a column with a value or an expression.
  serves: [shape.columns.nulls.constant, shape.columns.nulls.expression]
```

- **`serves` lists only the variants the operation performs**, never a nearest home. An operation serves a whole class only when it performs every variant in it. An operation takes the crosswalk words of each variant it serves. A variant several operations perform together lists each of them: the `all_in_one` variant, Alteryx's "Select", is served by hide, rename and cast. A crosswalk row whose word names the opposite of the operation, such as "Null If" under the fill class, falls in a variant the operation does not serve, so it is never an alias. The variant ids in the example stand for whichever of the class's variants the catalogue holds. Writing the thirteen entries is the catalogue task's work, and the owner reviews the entries in its pull request.
- **`sentence` is the operation's one sentence**, read by the palette as the row's detail for a title match and as the text tier 6 matches. arc's own `summary` stays, as the longer text a caller of `describe` reads. Brightfield's help line for a verb stays for verbs that are not operations.
- **`build.py` writes `operators/operations.json`**, a second, small file that arc embeds. It holds each operation's long name, title, sentence and aliases, with each alias's source already decided: present only for a tool marked `named`, or for the SQL spec. It carries no URLs, no crosswalk rows, no unpublished classes and no proprietary tool's name. It is generated, never edited, and stamped with `landscape_commit` as `catalogue.json` is. arc is public and the catalogue repository is not, so arc embeds this file and never `catalogue.json`.
- A tool table gains a `named` flag and a display name for each tool, and a spec table holds "SQL". A hand-kept list may add or drop words, since some crosswalk names are not words an analyst types: `Table.SelectRows` stays, while a name that is only a code path or a version string is dropped. Those rules are the catalogue's to keep, and its own check runs them.
- `catalogue.json` gains the same `operations` list, so the public pages can name each class's operations.
- The public operation pages switch their per-tool lists to the same alias list. That is the web repository's change, made after this one.

### arc

- `Operation` gains `title`, `sentence` and `aliases`. They are read from the embedded `operations.json` through `include_str!`, never written by hand. A test refuses an operation with no entry, or an entry with no operation.
- `arc operation describe <name>` and `arc operation list --json` carry `title`, `sentence` and `aliases` (`[{word, source?}]`). The MCP tools carry the same fields.
- **arc holds the one matcher.** A public library function takes a query and returns each matching operation with its tier, its score within the tier and what it matched (`title`, `long_name`, `alias` or `sentence`), with the word and its source. It implements the six tiers above, with its own subsequence scorer for tier 5. `arc operation search <text>` and the MCP tool `operation_search` print its answer, in tier order with no frecency.
- Updating the embedded file is a pull request in arc that copies it and keeps its `landscape_commit` stamp.

### Brightfield

- **Reading.** `OperationDescription` (`crates/brightfield-shell/src/operation.rs`) gains `title`, `sentence` and `aliases`. They are read once per session off the UI thread, with the rest of the description. Brightfield links arc as a library, so the shell reads the same embedded file directly at start for the palette's first frames. The description from the `arc` binary stays the one it records with.
- **Matching.** The palette ranks an operation's row by arc's matcher, called from `brightfield-shell`, and adds frecency within each tier. A verb that is not an operation is ranked by `brightfield-keys` on the same tier numbers (tiers 1, 2, 5 and 6, as described above), so the two kinds of row interleave in one list. A test runs both matchers over alias-free inputs and asserts they agree on the tier.
- **The row.** In `overlays.rs`, `CommandPalette::row` leads an operation's row with its title, and its detail says how the row matched. An operation's row is keyed by the operation's long name. filter-rows' and sort-rows' key verbs, `filter-rows-at-cursor` and `sort-rows-at-cursor`, differ from their long names, and their rows read "Filter rows" and "Sort rows" with `filter-rows` and `sort-rows` as their long names.
- **One title everywhere.** Each place the window names an operation reads its title from the catalogue: the palette, the ribbon, a header's menu, the inspector's Operation section and the status band. That replaces the verb entries' own `label` for operations. Any pixel baseline this changes is redrawn on the Mac Studio.
- **Frecency.** `RecencyCounter` becomes a per-operation count plus a last-used time. It is written beside the window's other per-user state (`brightfield-model`'s state files) and capped at the newest 1,000 uses. It orders rows within a tier only, and orders the empty query within each stage, as recency does today.
- **Choosing.** Confirming a row goes through `MeridianApp::start_operation`, as it does today. Nothing about the alias travels.
- **The pin.** Brightfield's pin of arc moves to the arc commit that serves the titles and aliases. That brings in whatever else arc has landed since `f5f3f07`, which the pin bump's own pull request takes care of.

## Constraints

- Brightfield is public, and the landscape repository is private. This spec names the catalogue and its fields, which the public pages already publish. It carries no planning identifiers.
- arc's long names are looked up exactly when an operation is recorded, and that does not change: aliases are for finding an operation, never for recording one.
- The gate is `cargo xtask ci`, and a pull request's gate is `cargo xtask gate <pr>`. Pixel baselines are drawn on the Mac Studio only. If a palette snapshot changes, it is redrawn there.

## The test the issue asks for

With a table's grid open:

1. Type an alias into the palette.
2. Assert that the operation's row is listed under its title, with the alias in its detail.
3. Choose it, give its arguments in its own place, and Save.
4. Assert that the recorded step is byte-identical to the one its key records for the same cell and arguments.

The test is modelled on `tests/operation_from_the_cursor.rs` and `tests/support/operations.rs`. A second case types a word that several operations share and asserts that each is listed under its own title.

## Left to the builder

- The exact fuzzy and prefix scoring weights within a tier, provided the tier order holds.
- How the state file for frecency is named, and what happens when it is unreadable. In that case frecency starts empty and nothing else changes.
- Which crosswalk names the catalogue's check drops as not being words, recorded in the catalogue's own rules.
- The detail's wording when a row matches on both its title and an alias: the better tier wins, and between equals the title wins.

## Calls made on the owner's behalf

Each is reversible and the owner's to overturn.

1. arc embeds a generated extract of the catalogue, never the catalogue itself, so no proprietary tool's name or URL reaches arc's public source.
2. An operation serves only the variants it performs. Its aliases are those variants' words.
3. Our own names outrank aliases at each degree of match: an exact title, then a title prefix, then an exact alias.
4. Each operation has one sentence in the catalogue, which the palette shows and matches. arc's `summary` stays as the longer text.
5. arc holds the one matcher, and Brightfield calls it. A parity test keeps the verbs' matcher in line.
6. The title replaces the window's own label for an operation everywhere the window names it.
7. Sort is treated as filter is: its row reads its title and its long name, not its key verb.
