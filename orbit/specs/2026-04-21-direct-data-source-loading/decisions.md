# Decision Pack — Card 0004: Direct Data Source Loading

Rally: **Layer 2 SQL emission**.
Card: `orbit/cards/0004-direct-data-source-loading.yaml`.
Scope: deciding how the Layer 2 SQL emitter translates parsed `DataSource` AST nodes into DuckDB SQL that registers or reads each of the five formats (Parquet, CSV, JSON, inline, DuckDB file) named in the brief §3.2.

## What is already fixed (not up for debate here)

These are inherited from card 0001 (`2026-04-20-mosaic-spec-driven-visualisation/spec.yaml`, ac-02) and the parser at `crates/brightfield-spec/src/parse.rs:458–517`:

- `DataSource { kind: DataSourceKind, extras: IndexMap<String, SpecValue> }` — node shape.
- `DataSourceKind::{Shorthand(String), File(String), Query(String), InlineRows(Vec<SpecValue>), Typed(String), Opaque}` — the six recognised parse-time shapes.
- `extras` preserves siblings like `select:`, `where:`, `type:`, `layer:` verbatim so the emitter does not re-parse YAML.
- Mosaic-version-pinned corpus lives at `crates/brightfield-spec/vendor/mosaic-specs/yaml/` — 54 specs, all of which use `file:` with Parquet, CSV, or GeoJSON paths (relative or `https://` URLs); none currently carry `.duckdb` references or top-level inline-row `data:` blocks. Mark-level inline data (`data: [15]` in `seattle-temp.yaml`) is a separate path handled by `MarkData::Inline` at mark emission time, not here.
- Layer 2 conformance is gated by `SqlEquivalenceCheck` in `crates/brightfield-conformance/src/layer.rs:162–178`, currently `Pending { reason: "SQL emitter not yet available" }`.

What this pack decides: how the Layer-2 emitter walks each `DataSourceKind` into a DuckDB SQL fragment, how paths are resolved, how the result is captured for conformance, and where the five-format guarantee is enforced.

---

## Decision 1 — Path resolution base for relative `file:` paths

### Context
54/54 corpus specs use relative paths under `file:` (e.g. `data/athletes.parquet`). A spec authored on disk has a natural anchor (the spec file's parent directory); a spec pasted into a REPL or fetched over HTTP does not. The brief promises "point brightfield at a Parquet file and start exploring" — so path references must resolve deterministically without an ingestion step. The parser drops the source path on the floor today (it takes `&str`), so the emitter cannot recover it after the fact.

### Options
- **A. Resolve relative paths against the spec file's parent directory.** Thread a `base_dir: Option<PathBuf>` through `parse_spec_path` into `ParseOutput`; the emitter joins `base_dir` with any non-absolute, non-URL `file:` value. HTTP(S) URLs pass through untouched.
- **B. Resolve relative paths against CWD.** Emit `read_parquet('data/...')` verbatim; DuckDB resolves against the process CWD.
- **C. Require absolute paths in specs.** Reject relative paths at parse time with `ParseError::MalformedDataDef`.

### Trade-offs
- **A (spec-relative)** — mirrors how every corpus spec is obviously meant to work (`athletes.yaml` lives next to `data/athletes.parquet`), matches JSON/YAML-import convention, and means `brightfield run spec.yaml` works from any shell CWD. Cost: parser needs to know the source path; requires a new `base_dir` field on `ParseOutput` and a `parse_spec_path` that populates it. `parse_spec(&str, Format)` without a path (tests, pasted SQL, stdin) resolves against CWD as fallback.
- **B (CWD)** — zero plumbing, minimal code. Loses: breaks every corpus spec unless the user `cd`s into the vendor tree first, and silently picks up files from unrelated CWDs — a footgun for a tool whose value is "open spec file, see data".
- **C (absolute only)** — trivially unambiguous. Loses: breaks 54/54 corpus specs and upstream Mosaic portability (card 0002's entire thesis). Non-starter.

### Recommendation
**Option A.** Add `base_dir: Option<PathBuf>` to `ParseOutput`; have `parse_spec_path` set it to `source_path.parent()` and `parse_spec(&str, Format)` leave it `None` (emitter falls back to CWD, which is only hit in tests/REPL). This is a small additive change to card 0001's parser surface and matches the card's stated goal literally: "point brightfield at a Parquet file" where "at" means the spec file's neighbourhood. HTTP(S) `file:` values are a separate bucket and pass through verbatim — consistent with how `observable-latency.yaml` and `flights-10m.yaml` already use remote URLs.

---

## Decision 2 — Format dispatch for `DataSourceKind::File`

### Context
The card enumerates five formats but the AST collapses all file-backed sources into `DataSourceKind::File(String)`. Dispatch to the right DuckDB function (`read_parquet`, `read_csv`, `read_json_auto`, or `ATTACH`) must happen in the emitter. The `.duckdb` attach path is structurally different from the three reader functions — it needs an alias and lives at the session level, not per-query.

### Options
- **A. Extension sniff.** `*.parquet` → `read_parquet()`, `*.csv | *.tsv` → `read_csv()`, `*.json | *.ndjson | *.geojson` → `read_json_auto()`, `*.duckdb | *.db` → `ATTACH`. Unknown → `ParseError::MalformedDataDef` (or let DuckDB auto-detect via `read_csv_auto` if we want permissive).
- **B. Require explicit `type:` on every file source.** `{ file: ..., type: parquet|csv|json|duckdb }`. No sniffing.
- **C. Extension sniff with `type:` override.** Default to sniffing (A); if `type:` is present in `extras`, it wins.

### Trade-offs
- **A (sniff only)** — matches corpus reality: every one of the 54 specs relies on extension-driven inference (none carry a `type:` key for Parquet/CSV). Zero friction for the author. Loses: ambiguous extensions (`.json` for records vs NDJSON vs GeoJSON — see Decision 4) and mis-named files fail opaquely.
- **B (explicit only)** — unambiguous but breaks every corpus spec. Makes brightfield reject specs Mosaic-web happily runs, killing card 0002's portability goal.
- **C (sniff + override)** — corpus specs work unchanged; spatial/GeoJSON sources already use `type: spatial` (see `unemployment.yaml:8-10`, `earthquakes-globe.yaml:10-14`) which the parser already lifts into `DataSourceKind::Typed`. The override slot is free and already present in `extras`.

### Recommendation
**Option C.** Sniff extension by default; honour `type:` when present in `extras`. For `.duckdb`/`.db`, dispatch to `ATTACH` (see Decision 5). `Typed("spatial")` sources route to `ST_Read(...)` — within this card's goal because `earthquakes-globe.yaml` pairs `.parquet` with `.json` and the card's "JSON" format must cover at least the GeoJSON path that appears in 8 of 54 corpus specs. Unknown extensions surface at emit time as a typed `EmitError::UnknownFormat { path, extension }` — actionable and symmetric with how parse errors are carried (card 0001's `ParseError` convention).

---

## Decision 3 — CSV column typing

### Context
Scenario "Load a CSV file with inferred column types" explicitly says *DuckDB-inferred column types*. DuckDB exposes two families: `read_csv_auto()` (single-pass sample + type detection, most forgiving) and `read_csv()` with explicit `columns=`/`types=` (strict, requires an author-supplied schema). The corpus contains exactly two CSV specs (`area-sine.yaml`, `triangle-wave.yaml`); neither declares column types. The card's "without an explicit schema" clause ties our hand.

### Options
- **A. Always `read_csv_auto(path)`.** No type options passed; brightfield never second-guesses DuckDB.
- **B. `read_csv(path, auto_detect=true)`.** Equivalent in DuckDB 1.1+, with `extras` passthrough for `header`, `delim`, etc.
- **C. `read_csv_auto(path)` default, `read_csv(path, <explicit opts>)` when `extras` supplies `columns`/`types`/`delim`/`header`.**

### Trade-offs
- **A (always auto)** — simplest. Loses: no escape hatch for a messy CSV DuckDB mis-detects (half-numeric ID columns, mixed-locale decimals). But the card's scope is "inferred types", so this may be fine for now.
- **B (read_csv with auto_detect)** — functionally equivalent to A today, but sets up the `extras` passthrough pattern for when `delim: ';'` or `header: false` becomes necessary. Slightly more verbose emitted SQL.
- **C (auto default, explicit when asked)** — most flexible. Loses: decision branching the emitter must carry forever; doubles the surface conformance must lock down.

### Recommendation
**Option B.** Emit `read_csv(<path>, auto_detect=true, ...extras)` where any `extras` keys in a fixed allow-list (`delim`, `header`, `columns`, `types`, `skip`, `nullstr`) are serialised as DuckDB kwargs; keys outside the allow-list are dropped with a `ParseWarning::UnknownOption` (card 0001 already has this warning type). This honours "DuckDB-inferred column types" by default, preserves author escape hatches that Mosaic-web already supports, and keeps the emission site uniform — no branching on "did the author give me a schema". Restricts the surface the conformance layer must pin.

---

## Decision 4 — JSON flavour selection

### Context
JSON under `file:` has three plausible shapes: a single array of row-objects, newline-delimited JSON (NDJSON), or GeoJSON (`{type: "FeatureCollection", features: [...]}`). DuckDB exposes `read_json_auto()` (handles array + NDJSON), `read_ndjson_auto()`, and the spatial extension's `ST_Read()` (GeoJSON). The corpus evidence: 8 specs use `type: spatial` with a GeoJSON-shaped file; no spec currently uses a bare non-spatial JSON data source.

### Options
- **A. `read_json_auto()` for all `*.json` unless `type:` says otherwise.** GeoJSON requires the author to declare `type: spatial`.
- **B. Content-sniff the first bytes of the file.** Read first 512 bytes at emit time, detect `{"type":"FeatureCollection"` vs `[` vs `{`, dispatch accordingly.
- **C. Require `type:` on every JSON source.**

### Trade-offs
- **A (type-driven)** — zero I/O at emit time (pure spec-to-SQL transform, stays deterministic and cacheable); matches exactly the 8 corpus specs that already declare `type: spatial` for their GeoJSON. Loses: a user pointing brightfield at a bare `.geojson` without `type: spatial` gets a `read_json_auto()` call that may succeed with a weird schema or fail unintelligibly — but the extension `.geojson` lets the emitter emit a targeted error: "looks like GeoJSON; add `type: spatial`".
- **B (content sniff)** — most DWIM. Loses: turns emission into an I/O operation — the emitter can no longer emit SQL for a spec with a remote URL without first fetching; defeats reactive re-emission (Decision 6) because the trigger isn't available; breaks determinism of the conformance string snapshots.
- **C (explicit type only)** — breaks every non-spatial corpus `file:` json (though there aren't any currently). Rigid.

### Recommendation
**Option A.** Dispatch on extension: `.json` / `.ndjson` → `read_json_auto(path, format='auto')` (DuckDB's own format detection inside the auto call handles array vs NDJSON); `.geojson` → emit `EmitError::UnknownFormat` unless `type: spatial` is present; `type: spatial` → `ST_Read(path)` (optionally with `layer: <name>` from `extras`, which 5 corpus specs use). This keeps emission a pure function of the spec + `base_dir` — critical for Decision 6 and Decision 7.

---

## Decision 5 — DuckDB-file attach semantics

### Context
Scenario "Attach to an existing DuckDB database file" requires `ATTACH '<path>' AS <alias>`. Two questions: (1) how is the alias derived from the spec's `data:` entry; (2) what happens when a spec attaches two DuckDB files whose inferred aliases collide. Zero corpus specs use DuckDB-file attach today; this path is net-new.

### Options
- **A. Use the `data:` key as the attach alias.** `data: { mydb: { file: foo.duckdb } }` → `ATTACH 'foo.duckdb' AS mydb`. References to `mydb.some_table` inside `Query` and mark `from:` resolve through DuckDB's attached-db namespace naturally.
- **B. Derive alias from filename stem.** `foo.duckdb` → alias `foo`. Collisions resolve with suffixes (`foo_1`).
- **C. Require an explicit `as:` key in the `data:` entry.** `data: { mydb: { file: foo.duckdb, as: bar } }`.

### Trade-offs
- **A (key-as-alias)** — consistent with how Mosaic treats `data:` keys elsewhere (`data: { flights: ... }` means "bind this name"); one namespace, no collision case (map keys are unique by YAML rules, enforced by `IndexMap`). Loses: alias must be a valid SQL identifier; `data:` keys with dashes (`data: { my-db: ... }`) need quoting — we already accept any string key in `IndexMap<String, DataSource>` so we must emit double-quoted identifiers for non-ident-safe keys.
- **B (filename stem)** — matches what a DuckDB shell user would type. Loses: two attached databases whose basenames happen to match (`a/data.duckdb`, `b/data.duckdb`) collide; the spec becomes path-dependent for the alias, which is surprising.
- **C (explicit `as:`)** — most controllable. Loses: adds a required field Mosaic-web doesn't have; can't port a `data: { mydb: { file: foo.duckdb } }` spec without editing.

### Recommendation
**Option A** with identifier quoting. Emit `ATTACH '<resolved-path>' AS "<spec-key>" (READ_ONLY)` — `READ_ONLY` because the card's scope is read-only exploration (the brief's "explore" framing), and it prevents brightfield from corrupting a production DuckDB file the user pointed it at. Tables inside are referenced as `"<spec-key>".table_name`. Name collisions cannot occur because `data:` keys are a map and the parser rejects duplicates via `IndexMap` behaviour inherited from card 0001.

---

## Decision 6 — Source registration lifecycle (views vs inline subqueries)

### Context
Card 0003 (sibling in this rally) promises reactive re-emission: a param change re-emits the SQL for affected plots. If data sources are wrapped inline into every query (e.g. `SELECT ... FROM (SELECT * FROM read_parquet('...')) AS athletes`), each reactive tick re-runs path resolution and DuckDB re-opens the file handle. If they are registered once at spec-load as `CREATE OR REPLACE VIEW <name> AS ...`, the view sticks and subsequent queries just reference it — cheaper under reactive pressure but stateful in the DuckDB connection. The `gaia.yaml` corpus spec (a 5M-row Parquet) explicitly stresses repeated querying of the same source under a selection brush — the card 0003 hot path.

### Options
- **A. Wrap every source inline into the query.** Each query's `FROM <name>` gets rewritten to `FROM (<emitted-source-sql>) AS <name>`. Stateless, fully functional.
- **B. Register each source as a view at spec-load.** Emit `CREATE OR REPLACE VIEW "<name>" AS <source-sql>` once; references stay as bare identifiers. Requires a two-phase emission: (i) setup DDL at spec-mount, (ii) per-query SELECTs.
- **C. Register Parquet/CSV/JSON as views; inline `Query` and `InlineRows`.** Hybrid: file-backed sources get views; query and inline sources are already SQL and get substituted inline.

### Trade-offs
- **A (inline)** — stateless, emission is a pure function — easiest for conformance (Decision 7) to snapshot. Loses: every reactive re-emission repeats the `read_parquet('...')` call; on a 5M-row file, DuckDB memoises the scan at plan time but still pays metadata-read cost per query. Also: author's `where: Symbol = 'AAPL'` extra (`line.yaml`) has to be injected into every reference site — more surface for bugs.
- **B (views everywhere)** — clean. Loses: stateful — the DuckDB connection carries view definitions that must be torn down on spec reload; a `CREATE OR REPLACE VIEW` over a `Query` source means the view's SQL is authored text, and re-parsing errors surface at view-create time rather than at the plot query. Also: `InlineRows` as a view requires the rows to be materialised, not re-inlined each call (fine, but a table write).
- **C (hybrid)** — matches the cost profile: file scans benefit from view reuse and the author's `where:`/`select:` `extras` get applied once inside the view; `Query` sources are already a CTE and don't gain from view wrapping; `InlineRows` are tiny and cheap to inline. Loses: two code paths in the emitter, one for view-setup and one for inline.

### Recommendation
**Option B** for v1 — register every source as a view at spec-mount. The two-phase split (setup DDL, per-plot SELECT) is load-bearing for Decision 7 (conformance can snapshot the setup DDL as a single string per source) and for card 0003's reactive hot path (param changes don't re-emit the view, only the SELECT that references it). `InlineRows` is materialised as `CREATE OR REPLACE VIEW "<name>" AS SELECT ... UNION ALL SELECT ...` or equivalent `VALUES (...)` (see Decision 8) — the `VALUES`/`SELECT UNION` form lives inside the view. `Query` sources become `CREATE OR REPLACE VIEW "<name>" AS <author-SQL>`. This collapses to one emitter code path with one per-kind "how do I construct the view body" function.

---

## Decision 7 — Conformance capture for Layer 2 data-source emission

### Context
`SqlEquivalenceCheck` in `crates/brightfield-conformance/src/layer.rs:162–178` is a `Pending` stub today. Once the emitter lands, card 0002's conformance runner needs a deterministic artefact to compare against expectations. Data-source emission is a natural first slice: it's bounded per source, string-comparable, and independent of plot-level query synthesis (which is a separate card further up the rally).

### Options
- **A. String-snapshot the emitted view DDL per source.** `<corpus-name>.layer2.expected.sql` contains a canonical rendering of every source's `CREATE OR REPLACE VIEW` statement, one per line. Conformance diffs actual vs expected.
- **B. Normalise to a typed structure and compare structurally.** Parse the emitted DDL back into a `DataSourceDdl { view_name, body_kind: ReadParquet|ReadCsv|ReadJson|Attach|Values|Query, args: BTreeMap }` and compare that.
- **C. Round-trip through DuckDB: execute the DDL, `DESCRIBE` the view, compare the schema.** Semantic check rather than syntactic.

### Trade-offs
- **A (string snapshot)** — trivial to author, trivial to review in PR, catches every change (intentional or otherwise). Loses: brittle — a cosmetic change (`READ_ONLY` before vs after the path) triggers every snapshot. Mitigated by a canonicalisation pass (sorted kwargs, fixed whitespace).
- **B (typed compare)** — resilient to cosmetic reshuffling. Loses: requires a SQL parser in the conformance crate (either sqlparser-rs or our own), which is net-new infrastructure for a hedged benefit.
- **C (execute + DESCRIBE)** — catches semantic equivalence across implementation strategies. Loses: requires the data files to exist at conformance-check time — breaks for HTTP URLs in CI without network, breaks for `.duckdb` fixtures that aren't checked in, and makes the conformance suite an integration test. Mosaic-web itself gives us no `DESCRIBE` output to compare against anyway.
- Note on C's applicability: the brief's primary Layer-2 goal is *emitted-SQL equivalence*, not *query-result equivalence* — the latter is a later card. Decision 7 only has to close the loop for the former.

### Recommendation
**Option A with canonicalisation.** Each corpus spec gets a sibling `<name>.layer2.expected.sql` listing its source-DDL block in a fixed format: view-name alphabetical, kwargs alphabetical, whitespace normalised. `SqlEquivalenceCheck` for v1.1 flips from `Pending` to a string-diff against this file, scoped to the data-source portion. Plot-SELECT conformance stays `Pending` until its own card. This is the cheapest path to a real Layer 2 gate and aligns with card 0002's deviation registry — cosmetic emitter changes get one-line `.expected.sql` updates, semantic changes show up as a diff in PR review.

---

## Decision 8 — Inline-row emission strategy

### Context
Scenario "Inline data in the spec works without an external file" requires the parser's `DataSourceKind::InlineRows(Vec<SpecValue>)` to produce working DuckDB SQL. DuckDB supports three inline forms: `VALUES (...)`, `SELECT ... UNION ALL SELECT ...`, and `CREATE TEMP TABLE ... INSERT VALUES`. The rows are `Vec<SpecValue>` — object-per-row (name/value pairs) or array-per-row (positional) — the parser preserves both. Corpus has no top-level inline `data:` example, but `seattle-temp.yaml:27` shows a mark-level `data: [15]` literal (handled elsewhere).

### Options
- **A. `VALUES (row1), (row2), ...` wrapped in `SELECT * FROM (VALUES ...) AS t(col1, col2, ...)`.** Requires column names — for object-per-row, take keys from the first row; for array-per-row, synthesise `c0, c1, ...`.
- **B. `SELECT v1, v2, ... UNION ALL SELECT v1, v2, ...`.** Every row gets its own `SELECT`. Self-labels columns via `AS`.
- **C. `CREATE TEMP TABLE "<name>" ...; INSERT INTO ... VALUES (...)`.** Real materialised table.

### Trade-offs
- **A (VALUES clause)** — concise, lives inside a view body naturally (Decision 6), DuckDB plans it once. Column-name derivation from the first row's keys is a minor risk: heterogeneous row objects would silently lose keys. Document: we snapshot the first row's keys; subsequent rows' extra keys become `ParseWarning::UnknownOption` (reused).
- **B (UNION ALL SELECTs)** — no column-naming problem (each SELECT carries `AS`). Loses: verbose — N rows means N SELECTs; SQL size blows up for anything >~20 rows, inflating the Decision 7 snapshot unreasonably.
- **C (TEMP TABLE)** — most flexible, supports large inline datasets. Loses: stateful DDL + DML, two statements per source, complicates the "view-is-the-unit" assumption in Decision 6, and inline data large enough to need a temp table almost certainly belongs in an external file.

### Recommendation
**Option A** with a row-count guard. Emit `CREATE OR REPLACE VIEW "<name>" AS SELECT * FROM (VALUES (...), ...) AS t("col1", "col2", ...)`. If row count exceeds 1000 (a deliberate ceiling — large enough for every realistic inline case, small enough that one snapshot stays reviewable), emit `ParseError::MalformedDataDef { detail: "inline row count {n} exceeds 1000 — use a file source" }`. This gives the card its stated inline-data guarantee, keeps Decision 7 snapshots human-sized, and draws an explicit line against spec authors misusing inline rows as a data-loading strategy — the card says "without an external file", not "in place of a file".

---

## Summary table

```
| #  | Decision                                    | Recommendation                                                    |
|----|---------------------------------------------|-------------------------------------------------------------------|
| 1  | Path resolution base                        | Spec-file parent directory (base_dir threaded through parse)      |
| 2  | Format dispatch for File                    | Extension sniff, type: override                                   |
| 3  | CSV typing                                  | read_csv(path, auto_detect=true, ...allow-listed extras)          |
| 4  | JSON flavour                                | read_json_auto for .json/.ndjson; type: spatial → ST_Read         |
| 5  | DuckDB attach alias                         | ATTACH '<path>' AS "<data-key>" (READ_ONLY)                       |
| 6  | Source registration lifecycle               | CREATE OR REPLACE VIEW at spec-mount; SELECTs reference bare name |
| 7  | Conformance capture                         | <name>.layer2.expected.sql string-snapshot, canonicalised         |
| 8  | Inline-row emission                         | VALUES inside view body; hard-cap at 1000 rows                    |
```

## Cross-cutting notes

- **Five-format guarantee**: Decisions 2/3/4/5/8 cover Parquet (2), CSV (3), JSON (4), DuckDB (5), inline (8) respectively. Combined with Decision 1 (paths resolve) and Decision 6 (views register), the card's stated goal is fully walked.
- **Interaction with card 0003 (sibling in rally)**: Decision 6's view-registration lifecycle is the mechanism card 0003 needs — reactive re-emission only has to recompute the SELECT, not the source DDL. Decisions here should land *before* card 0003 begins implementing reactive re-emission; the rally ordering should reflect this.
- **Interaction with card 0002 (web portability)**: all decisions preserve the Mosaic `data:` wire shape — no new required fields, all corpus specs parse unchanged. Decision 5's `READ_ONLY` is the one emission-side divergence; it can be registered in `DEVIATIONS.md` as a safety-driven deliberate divergence if Mosaic-web ever opens its attached DBs read-write.
- **Out of scope for this card** (belongs in a later rally card): plot-level SELECT synthesis, `filterBy` predicate injection into view references, M4 pre-aggregation (`overview-detail.yaml` references this), `config: { extensions: spatial }` auto-loading at spec-mount.
