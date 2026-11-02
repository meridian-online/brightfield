# Dovetail as Brightfield's discoverer

## Intent

When an analyst opens a folder, workbook or database of related tables, Brightfield should propose how those tables relate: each table's role, its key, and the joins between them. Today the own-files flow runs a stand-in that lists every table as unclear with no joins, so the analyst draws every join by hand. dovetail's `model` command now writes exactly the proposal Brightfield needs, as an Ossie model, in a quick sample pass and a full pass that verifies it on every row. This work links dovetail into the Brightfield binary, runs it as a child process for each pass, and makes Brightfield read what it writes.

Done means: `dovetail_child_writes_valid_ossie` runs unignored and passes, and opening a folder of related tables proposes dovetail's joins.

## Goals

1. **The hand-edit marking fix comes first.** A hand edit in the model's file view marks an element as the analyst's only where the edited text differs from the text the editor started from. Where the text and that starting point agree, the element takes the model's current version, so a discovery pass that changed it while the editor held old text is not reverted.
2. **The model crate reads what dovetail writes.** `brightfield-ossie` types dovetail's new Meridian keys: on a dataset `role_scores`, `fold_column`, `combined_into` and `combination` (`kind`, `parts`, `token_column`); on the model `pass`, `typing` and `sample_rows`. Every Meridian key Brightfield does not type survives a read and a write unchanged, so the next key dovetail adds is not lost when Brightfield saves the model. The vendored schema is renamed `ossie-schema.json`, its name upstream, with its licence beside it.
3. **A combined dataset gets a silver table.** dovetail combines tables that are one kind of thing split across files (a partition, a snapshot series, a duplicate) into a dataset whose `source` is a query over the parts' raw tables. Its silver table is built from that query, each part with `combined_into` gets no silver table of its own, and joins are measured against the combined table. Without this, opening a folder with a partition pair breaks the silver build.
4. **dovetail runs as a child process of this binary.** The workspace takes `dovetail` as a git dependency pinned to commit `c5c1a4f` on dovetail's main, with `default-features = false` (no candle classifier). A second runner variable beside `BRIGHTFIELD_RUN_AS_ARC`, `BRIGHTFIELD_RUN_AS_DOVETAIL`, makes `main` hand the process to `dovetail::cli_main` before any window work, as it does for arc.
5. **`Dovetail` implements `Discovery`.** Each pass runs `dovetail model` with `--db`, `--packages <protocol>/packages`, `--base <protocol>/model.ossie.yaml` when the file exists, `--pass`, `--name` (the Protocol folder's name, as the stand-in names the model), `--out` the pass's proposal file, and for the full pass `--verify <protocol>/.discovery/sample.ossie.yaml` when the sample proposal exists. Brightfield reads the proposal file as the pass's answer. A superseded pass's child is killed and its file not read. A child that fails answers its stderr.
6. **Opening and Rediscover use dovetail.** The own-files open (`data_file::open_any`), the full pass the pane starts when it opens, and the pane's Rediscover action all run `Dovetail`. A single table (one sheet, one database table) keeps the stand-in: it has nothing to relate.

## Non-goals

- Folds as one struct column in silver. dovetail writes `fold_column`, and Brightfield reads and keeps it, but silver still folds a reference into flat `<reference>_<field>` columns. Changing that renames the columns every chart over a host table reads, which is a product change of its own. Raised as a follow-up.
- Showing role scores, marginal roles, provenance, or combinations in the pane, and splitting a combination apart again. The keys are read and kept; the pane shows what it shows today. Follow-ups.
- Running the sample pass of Rediscover off the window's thread. Today's Rediscover runs the sample pass where it is asked for, and with dovetail that takes seconds rather than nothing. Raised as a follow-up rather than restructured here.
- Moving dovetail's FineType pin. dovetail pins FineType v0.6.64, and Brightfield stages v0.6.67. With `--packages`, which Brightfield always passes, dovetail takes every field's type from the packages Brightfield profiled, so the older pin only reaches dovetail's own label-to-type map. Raised as a follow-up on dovetail; no gate added here.

## Anti-goals

- dovetail writing `model.ossie.yaml`. It is Brightfield's alone to write; the child writes only `.discovery/<pass>.ossie.yaml`.
- A discovery pass making an analyst edit fail. An edit saved while a full pass runs must still run the Protocol.
- A discovery pass rearranging what the analyst decided. Analyst elements are carried by dovetail through `--base` and kept by Brightfield's merge.

## Constraints

- **The database lock.** dovetail opens `--db` read-only, and a DuckDB file opened read-only by one process cannot be opened for writing by another. The pane runs `arc` over the Protocol's database after every save, on the window's thread, while the full pass may be running. Pointing dovetail at the live database would make that run fail.
- One cargo build at a time on Hugh's Mac; dovetail adds DuckDB nothing new (both pin `duckdb = "=1.20000.0"` bundled), but it adds `parquet` 53, jaq and finetype-core.
- dovetail has no tag; the git dependency pins a commit, recorded in `Cargo.lock`.
- The repo is public: no planning identifiers in code, commits or the pull request.

## Approaches

For the database lock, the one real design choice:

1. **A copy of the database per pass (chosen).** Before each pass, `Dovetail` copies the Protocol's database file into `.discovery/`, while holding a process-wide lock that `own_files::run_loads` also holds, and points `--db` at the copy. The copy is removed when the pass ends. Costs the disk and time of one file copy per pass, which is small beside dovetail's own reading of every table. An edit during a full pass waits at most for the copy, never for the pass.
2. **Hold arc runs until the full pass lands.** No copy, but the pane would defer the run an edit owes, and the analyst's silver tables and join measurements would lag their edits by a full pass. That changes what an edit does today.
3. **A database of views over the profile's Parquet snapshots.** No copy of the data, but dovetail lists only base tables as a run's tables, so views are not seen. Would need a dovetail change.

For the hand-edit fix, the fix is the one the branch review gave; there is no second approach worth carrying.

## How it gets built (my calls, Hugh away)

- **Hand-edit marking.** `mark_changed` takes `base` as well as `current`. For each dataset, field and relationship in the edited model: when `base` holds it and the edited one equals `base`'s (by the same comparisons used today), it is replaced by `current`'s version when `current` holds it, and dropped when `current` no longer does (a pass removed it since); otherwise it is the analyst's. A dataset's own part and its fields are compared separately, as today, so a field edit does not claim the dataset.
- **Unknown Meridian keys.** Each Meridian meta struct gains a flattened `extras` map that keeps the keys it does not type. The typed new keys are optional and skipped when empty, so a stand-in model writes the same bytes as before.
- **Combined silver.** A dataset whose `combination` is set reads from `(<its source>)` instead of `raw_<name>`. Since dovetail's combined query lists the parts' columns without the lineage columns, the silver source unions an empty row of the three lineage columns by name ahead of it, so the lineage columns are present, NULL today, and filled the day dovetail's query carries them. `has_silver` is false for a part with `combined_into`. `write_silver` writes steps for each combined dataset as well as each source table, and `drop_unmade_silver` drops a part's silver tables.
- **Warnings.** A join's `warning` has one home. Brightfield's measurement over the silver tables runs after every run and overwrites it, as today; dovetail's full-pass warning stands until then. The silver measurement is over the tables the charts read, so it is the one to trust when both exist. Dovetail's next full pass writes its own again.
- **The child.** `Dovetail` holds the runner program (`Runner`), as arc's runs do. It removes a stale proposal file before the run, starts the child with stdin closed and `NO_COLOR`, reads stderr on a thread, polls the child every 50 ms, kills it and answers an error when `cancel` is requested. `discover` is `discover_until` with a cancel never requested.
- **Exit status.** `serve_as_dovetail_if_asked` ends the process with dovetail's `ExitCode`.
- **The open blocks while the sample pass runs.** An own-files open already runs on the window's thread and holds the window while its tables load and profile (`open_path_now` says so). dovetail's sample pass adds its seconds to that wait. The stand-in model is not shown first.
- **The lineage row is a zero-row select.** `SELECT NULL::VARCHAR AS __source_file, NULL::VARCHAR AS __source_part, NULL::BIGINT AS __source_row WHERE false`, the types the load steps give them, unioned by name ahead of the combined query. It adds no row.
- **What the lock covers, and the copy.** Every write to the Protocol's database from this process takes the lock: `run_loads` (arc's runs, which also make the silver tables) and `drop_unmade_silver`, the one in-process writer. The copy takes the database file and its `.wal` when one is beside it.
- **Each run has its own files.** Each pass copies the database to a file named for the run (`.discovery/run-<n>.duckdb`, a process-wide counter) and has the child write to `.discovery/run-<n>.<pass>.ossie.yaml`. Once the child exits 0 and the pass was not cancelled, Brightfield renames that file to the pass's proposal file. A run removes its own copy, its out file and the out's `.writing` beside it when it ends, whether it succeeded, failed or was killed. Two children, a superseded one being killed and its successor, then never write one file.
- **`--verify` is never stale.** Each pass removes its own proposal file before it runs. A full pass therefore finds `sample.ossie.yaml` only when the sample pass before it succeeded, since a failed sample pass removed it and wrote none. A sample pass also removes `full.ossie.yaml`.
- **Only the open checks for a single table.** A one-table open never reaches the model pane, so the pane's full pass and Rediscover run `Dovetail` without a check.

## Tests

- The hand-edit fix: a model whose dovetail join was changed by a pass after the editor's text was taken; an unrelated edit of that text keeps the pass's join and does not mark it the analyst's; editing the join itself still marks it.
- The model crate: dovetail's two goldens (copied into `brightfield-ossie/fixtures/`) read, validate, type their new keys, and write back with every Meridian key they held; an unknown key survives a round trip.
- Silver: a model with a partition combination gives the combined dataset a silver statement over its query with lineage columns, gives the parts none, and the statement runs in DuckDB over two raw tables.
- The child: `dovetail_child_writes_valid_ossie` unignored; a cancelled pass kills its child and answers no model; an open of the retail folder proposes joins (`opening_related_tables_proposes_dovetails_joins`); a save during a running full pass runs the Protocol.

## Left to the builder

Names of private helpers, where the lock lives (`own_files` is the natural home), and test fixture layout.
