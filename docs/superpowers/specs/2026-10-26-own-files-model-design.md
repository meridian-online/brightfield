# An analyst's own files open as a first viable model

Design for Brightfield opening an analyst's own data: each table arrives profiled by its FineType types, and when what they open holds more than one table, Brightfield proposes how the tables fit together as a small set of related, cleaned tables, which the analyst inspects and reshapes, mostly on a diagram.

## Intent

The first hour with Brightfield is for gaining insight and priming for action. Insight comes from analytical operations and visualisations; action is the change the analyst wants to see in the world, set off later by egress, an exported artefact or a call to a downstream system. This design is the first layer of insight on the analyst's own data.

In medallion terms an analyst brings raw files (bronze) and leaves with a **first viable data model** (silver): every column cleaned to its FineType type, the tables joined along the relationships the data supports, reduced to the minimal set of related tables that carries the analysis. Everything past that, from export and charts to segmentation, scoring, entity resolution and machine learning, is gold, built on this model and not part of this design.

The feelings the first hour has to give include "this is easy", "aha! I can see it now" and "results are so quick". The "aha" here is an analyst seeing their own pile of files as one model; the ease and speed are how little they had to do to get it.

The analyst this serves is the one Hugh was as a solutions analyst: someone who receives extracts from another organisation's databases, has to understand a schema they didn't design, and then shapes it into something they can enrich and analyse. They use the model for two things:

1. **Lineage at the source.** A data quality problem in the model traces back to the dataset as it was received, so the analyst can say "this came in broken" rather than "we broke it".
2. **A pathway to a minimal set of related tables**, such as entities for slow-moving facts and events for timestamped observations, from which customer attributes and segments are later made. Entities and events is one useful combination, not a shape every dataset must take.

Two problems are kept apart throughout:

- **What's in this one table?** The profile. Every opened table gets it, a single CSV included. It is drawn by the grid and its column header, and it is not redesigned here beyond the header band showing each column's FineType type by its title and icon.
- **How do these tables fit together, and what can I build from them?** The model. Only an open with more than one table gets it, and it is the subject of this design.

## Goals

1. **Open what analysts bring.** The front door opens, besides a single CSV, TSV or Parquet file:
   - a folder of CSV, TSV or Parquet files;
   - an Excel workbook, each sheet a table;
   - a DuckDB or SQLite database file, each table a table.
   CSV folders and Excel workbooks are the common case and get the most care.
2. **Every table is profiled by its FineType types**, as a single file is today, and the compact header band names each column's FineType type by its title and icon.
3. **One open specification per tool's output.** finetype describes each source as a Frictionless data package (datapackage.org). dovetail describes how the sources fit together as one Apache Ossie semantic model across them. Brightfield draws them and arc materialises them. Each boundary is a file that can be read, diffed and tested on its own.
4. **Brightfield builds its best guess straight away** and lets the analyst fix it afterwards. No question is asked before the first picture. People ignore suggestions made before their own first move and value them after it (Heer, 2019), and flow needs immediate feedback with nothing breaking concentration.
5. **The silver model is cleaned and FineType-aware.** Each column is cast to its FineType type through that type's transformation contract. Values that fail the cast are kept beside the model where the analyst can see them, never dropped.
6. **dovetail proposes the minimal related set by table role.** It does not fit the tables to a star schema or any one pattern. It infers each table's role from evidence it already has, and the familiar patterns become ways of combining roles:
   - **Entity:** one row per thing, with a real key. dovetail finds key-like parents.
   - **Event:** one row per timestamped occurrence, pointing at an entity. FineType finds the timestamp, and dovetail finds the pointer.
   - **Reference:** a small lookup that only labels another table. Fold it in.
   - **Bridge:** two pointers and little else, which is a many-to-many link. Keep it, because it is the relationship.
   - **Snapshot:** an entity repeated over dates, giving state over time.
   - **Unclear:** say so rather than force it.

   The test of minimal is tidy data's: one table per kind of thing observed, and no lookup left that only labels another. When the data fits no familiar pattern the reduction still happens: references fold in, duplicates collapse, and each remaining table carries its role.
7. **Joins come graded with their evidence.** A likely join is verified against the data and accepted without asking. A possible join is plausible but unproven, and is drawn as a proposal. Each carries why: value overlap, orphan count, whether the two columns' FineType types agree, name similarity, and its cardinality.
8. **The Ossie file is the model, and the picture is how it is mostly edited.** Every reshape the analyst makes on the diagram or the list rewrites the Ossie file. A hand edit to the file redraws the picture. An edit Brightfield cannot act on is listed as a warning, item by item, as OWOX's Model Canvas lists what it won't import.
9. **The representation, the inspection and the interaction all follow "overview first, zoom and filter, then details on demand".** Hugh, on the reshape step: "This is probably the most important step."
10. **Nothing hidden, no run button.** Roles and joins appear as dovetail finds them, on a sample first and then verified on the full data, with the model usable before verification ends. Every reshape updates the file and the preview at once. Every proposal shows why on demand.
11. **The model is part of the analyst's Protocol.** Loading, cleaning and the model are recorded so the whole can be rerun, audited and saved with the analyst's work.

## Non-goals

- **Joins out to reference data the analyst did not bring**, such as postcode geography for a postcode column. This is the natural next step and the reason FineType and a registry of known datasets exist, but the first viable model relates only the tables the analyst opened.
- **Gold:** customer attributes, segments, metrics, scoring, resolution, machine learning. The model makes them one step away; it does not build them. Ossie metrics are not authored here.
- **The outlets for action:** egress, export of the model, calls to downstream systems.
- **Redesigning the profile.** The grid, its header and its transposed row view keep their current design; the only change is the header band's FineType title and icon.
- **Other sources:** JSON, remote URLs, cloud storage, live database connections, DuckLake.
- **Drag and drop of files onto the window.** The picker and the typed path are the routes in.
- **Platforms other than macOS** for this milestone's validation, as for the rest of the launch.

## Anti-goals

It fails, even if it works, if:

- the analyst is asked questions before they see anything;
- the overview is a hairball canvas they have to untangle before they can read it;
- a proposal can't say why it was made;
- the analyst waits on a spinner or presses a run button to see the effect of a change;
- the model lives in a format only Brightfield reads.

## Constraints

- **DuckDB 2.0.** Brightfield is on DuckDB 2.0 (`duckdb = "=1.20000.0"`, bundled). dovetail is on DuckDB 1.4. Two DuckDB builds cannot be linked into one binary, so **dovetail must move to DuckDB 2.0 before Brightfield links it**. That is dovetail's change, ahead of this milestone's build.
- **FineType reaches Brightfield as a DuckDB extension bundle** (`finetype.duckdb_extension`, its model directory and `taxonomy-schemas.json`) found beside the executable, and dovetail uses finetype's crates in-process. The two must agree on the FineType version; today Brightfield's bundle pin and dovetail's crate pin are set separately.
- **finetype does not yet write a per-source data package.** Today `dovetail survey` writes the `datapackage.json`, typed by FineType, and `dovetail relate` writes `foreignKeys` into it. Under goal 3, finetype owns the per-source data package and dovetail's cross-source output is Ossie. Until finetype ships that, Brightfield may produce the per-source package through dovetail's existing survey; the boundary file is the same either way.
- **Ossie is a draft.** The core specification is `0.2.0.dev0`, incubating at Apache, and its validator rejects versions it does not support. Pin one version; expect to move with it. Ossie relationships are statements: there is no grade and no evidence in the core, and datasets have no role field. Roles, grades, evidence and folds are carried in `custom_extensions` under one Meridian vendor name. FineType types are carried the same way unless finetype publishes its taxonomy as an Ossie ontology of value types, which would let a field point at its type through a standard ontology mapping.
- **What DuckDB can do is done in DuckDB.** Hugh, 2026-10-26: replicating something DuckDB can do costs data lineage and the reproducibility of the transformation. So workbooks are read with DuckDB's `read_xlsx` (its `excel` extension) and SQLite files through its `sqlite` extension, in the load steps arc runs, not by a Rust library inside Brightfield. These extensions download on first use, which is acceptable, as FineType's model already downloads on first use; while one is fetched the window says so. An installer that offers to pre-load extensions is separate work.
- **The keyboard grammar.** Every action is a verb in `brightfield-keys`' registry, reached by key, by the Space palette, by code and by an agent. `h`, `j`, `k` and `l` already move focus.
- **The picker today** accepts single `csv`, `tsv` and `parquet` files and refuses `.duckdb` on purpose, because its one-view path has no answer for a file with many tables. The profiler returns `Unsupported` for an attached database. Both change.

## Approaches

Where dovetail runs, and how Brightfield talks to it:

| | How | Gains | Costs |
|---|---|---|---|
| **A (chosen)** | dovetail is linked into Brightfield's binary and runs as a child process of it, as arc does today. They talk through files: finetype's data packages in, the Ossie model out. | One signed binary. The open specifications are the real boundary and are testable on their own. A dovetail crash cannot take the window down. | dovetail must move to DuckDB 2.0 first. |
| B | dovetail ships as its own helper binary inside the app bundle, as the DuckDB CLI does. | Its DuckDB version is independent of Brightfield's. | A second binary to sign and notarise, its ML stack adding to the download, and version drift between the two. |
| C | dovetail's library runs in-process on Brightfield's own DuckDB connection. | No files are re-read, so discovery is fastest. | Crash and dependency trees are coupled, and the boundary becomes a Rust API rather than Ossie. |

Hugh chose A. Speed does not separate them much: dovetail runs only for discovery and verification, while each reshape is Brightfield rewriting the Ossie file and previewing on a sample through DuckDB, which stays immediate under A.

## What was decided

### The flow

1. The analyst opens a file, a folder, a workbook or a database file.
2. Each table is read and profiled by FineType, and each source's data package is written. A single table stops here: it lands where a single file lands today, on its grid and dashboard, with the profile.
3. For more than one table, dovetail runs as a child process over the data packages and the loaded tables. On a sample it proposes roles, joins and folds and writes the Ossie model; the model view opens on it at once. dovetail then verifies against the full data and rewrites the model with verified grades. The view updates in place as grades settle, with no spinner over the picture.
4. Each table's silver form is its columns cast through their FineType transformation contracts, with failing values set aside in a companion table the analyst can open.
5. The analyst inspects and reshapes. Each action rewrites the Ossie file. The preview follows at once, on a sample.
6. The analyst picks where to start, and goes on to the grid, a chart or the dashboard of the table they chose, or opens a join's joined rows in the grid.

### Lineage to the row

Every loaded table keeps where each row came from: its source file, its sheet for a workbook or its table for a database file, and its line or row number. These travel through the silver cast into the silver tables and into the failed-values table, so a bad value in the model points at the line in the extract the analyst received. Hugh chose row-level lineage, because naming the rows is what proves "this came in broken".

### What silver materialises

arc materialises one cleaned table per table kept in the model, with folded references joined into the table they label and tables left out of the model not materialised. Joins between the remaining tables are not materialised as tables: they live in the Ossie model, which keeps the set related rather than flattened, and a preview or chart joins only what it needs. Each silver table has its failed-values companion.

The cast is FineType's transformation contract: each type's `transform` expression from the taxonomy, producing its `broad_type`, as `finetype validate` applies it. It is applied so that a value that fails is recorded in the companion table rather than stopping the load.

### The analyst's decisions win

Every element of the Ossie model records who decided it, in the Meridian vendor extension: dovetail's proposal or the analyst's decision. An analyst's decision (a kept or dropped join, a changed role, a fold, a table left out, a retyped column, a hand edit) is never overwritten by dovetail: not by the verification pass that follows the sample pass, and not by a later run. dovetail's rewrite only updates elements it proposed and the analyst has not touched.

### What a rerun does

Rerunning the Protocol reloads and recasts the data and re-verifies the joins and roles already in the Ossie model against it. Where the new data weakens one, say orphans appear in a likely join, Brightfield says so as a warning on that join, and does not restructure the model. Discovering afresh is an explicit action, "Rediscover", which proposes again around the analyst's decisions without changing them.

### The model view

One model, three views, one inspector.

**Diagram, the landing view.** Built on the OWOX Model Canvas shape that Hugh found most intuitive:

- Each table is a card with its role marker, name, row count and field count. A table with folded references names them on its card.
- Each join is an edge labelled with its own keys and cardinality (`customer_id = customer_id  N:1`), so an edge explains itself without a click. Likely joins are solid; possible joins are dashed and coloured as proposals, with their grade on the label.
- **The layout comes from the roles:** events on the left, entities on the right, snapshots, bridges and unclear tables below. It is deterministic, so the picture does not rearrange itself between opens, and it reuses Brightfield's own layered layout and Vello drawing of the Protocol graph.
- A header line summarises the model: tables, joins, how many possible joins wait, how many tables are unclear.

**List.** Hamilton Ulmer's form for a DAG whose nodes are few: each table a row grouped by role, with its metadata beside it (role, rows, key, time span, how clean it is, its joins), and the joins routed in a narrow gutter on the left. Possible joins are listed first as a short review queue, the pattern Informatica's foreign-key profiling and ERDbuilder share. It is the compact form once the model is settled.

**Ossie file.** The YAML, editable. The selection follows between views: selecting a join in the diagram scrolls the file to its relationship.

**Inspector, details on demand,** on the right:

- *For a join:* its keys and cardinality, its grade, its evidence (value overlap, orphans, the FineType type on each side, names, whether it is verified on the full data), and a live preview of the joined rows on a sample, as Instant SQL previews any part of a query. Actions: keep a possible join, drop a join, change its keys, show it in the file.
- *For a table:* its role, which can be changed here; its columns, each with its FineType type as a chip and how clean it is; what was folded into it; and its failed values. Actions: change role, fold into or unfold from another table, leave the table out of the model, retype a column.

**Overview, zoom and filter, details.** The diagram and the list are the overview. Selecting a table or a join is the zoom and filter: the rest dims and only what is connected to the selection stays lit. The inspector is the details. The same three levels hold in each view.

### Reshaping

Every action is a registered verb, reachable by key, by the palette, by code and by an agent, and each one rewrites the Ossie file:

- keep or drop a possible join; drop a likely join;
- add a join by connecting a column on one card to a column on another;
- change a table's role;
- fold a reference into the table it labels, or unfold it;
- leave a table out of the model, or bring it back;
- retype a column to another FineType type, which recasts it in the silver table;
- edit the Ossie file by hand.

**Switching views:** `v` cycles Diagram, List and Ossie file; the palette also offers each by name. `l` is not used, because it already moves focus.

### Where it lives

The model is recorded in the analyst's Protocol folder. Its load and cleaning steps are arc steps, so the silver tables are materialised by arc and rerun with the Protocol. The Ossie model sits in the folder as one file, and each source's data package beside it. Saving the Protocol saves the model.

Opening a folder, workbook or database file creates that Protocol folder without asking, in Brightfield's own data directory beside the getting-started Protocols, and the analyst moves it where they want when they save. A single file keeps today's route.

### What comes from where

| Piece | Owner | Artefact |
|---|---|---|
| Per-source typing and description | finetype | One Frictionless data package per source, with each field's FineType type |
| Cleaning contract per type | finetype | The type's transformation contract, applied as the silver cast |
| Roles, joins, grades, evidence, folds | dovetail | One Ossie semantic model across the sources |
| Loading and materialising silver | arc | Protocol steps and their tables |
| Drawing, inspection, reshaping, preview | Brightfield | The model view, writing the Ossie file |

## What dovetail and finetype need to deliver

**Brightfield builds ahead of this work** (Hugh's choice): the readers, the profile, the silver cast and the model view are built against hand-written Ossie models and data packages that stand in for dovetail's and finetype's output, and dovetail is wired in as a child process when it lands. The open file formats are what make that safe; the stand-in files are checked against the pinned Ossie schema so they cannot drift from what the specification allows.

**One FineType version.** Brightfield's packaging pin for the FineType bundle is the version of record. dovetail's crate pin must name the same release, and Brightfield's gate fails when they differ. Brightfield links dovetail without its neural-classifier feature: dovetail reads each field's type from finetype's data package, so the classifier is not linked twice.

This design depends on work in two other repositories. Each is designed in its own repository:

- **dovetail on DuckDB 2.0**, with its FineType pin agreed with the bundle Brightfield ships.
- **dovetail writes an Ossie model** across the sources, carrying relationships with their keys, roles, grades, evidence and folds in its vendor extensions, and reading finetype's per-source data packages as input.
- **dovetail infers table roles** and proposes the minimal related set, as goal 6 describes, each role with its evidence.
- **dovetail can run on a sample and then verify**, so the first model arrives before full verification ends.
- **finetype writes the per-source data package** (until then, dovetail's survey writes it).

## Left to the builder

- How a folder is read: which files it takes, whether it recurses, and what it does with files it cannot read (list them; do not fail the open).
- The sample size and sampling method for discovery and preview. Hamilton Ulmer's note on join sampling applies: sample the most upstream table of the foreign-key graph and then join, rather than joining two samples.
- The Meridian vendor name and the JSON shape inside `custom_extensions`, and the pinned Ossie version.
- File names and layout inside the Protocol folder.
- Whether previews query through DuckDB's `ossie` community extension (which refuses fan-out joins) or through Brightfield's own SQL.
- The visual treatment of roles, grades and dimming, from the Meridian design system.
- Thresholds that make a table "small" for a reference or a join "likely", which are dovetail's to set.

## References

- Hamilton Ulmer: the Instant SQL talk (AI Council 2025), on immediate feedback, nothing hidden, and "overview first, zoom and filter, then details on demand"; his note on sampling joins from the root of the foreign-key graph; his post on drawing a DAG's nodes as table rows.
- Apache Ossie core and ontology specifications; OWOX's and Entropy Data's introductions to Ossie, with their Model Canvas and List and Diagram views.
- dbdiagram.io and DBML: the model as text with a live diagram.
- Informatica's foreign-key profiling view and the ERDbuilder R package: reviewing inferred joins with their evidence.
- Hadley Wickham, "Tidy Data": one table per type of observational unit.
- Jeffrey Heer, "Agency plus automation" (2019): suggestions before a person's first move are ignored.
