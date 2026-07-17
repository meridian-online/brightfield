# Spec — Protocol DAG spine (card 0025, v1.0)

Headless asset-graph render of an arcform Protocol manifest. Phase 1 of the asset-first
protocol view: parse → typed graph → deterministic layout → vello scene → PNG on the
existing `BRIGHTFIELD_DUMP_PNG` path. No window, no keyboard, no CTE parsing, no run state.

## 1. Crate: `crates/brightfield-protocol` (gpui-free, vello-free)

Pure data + geometry. Depends on: `serde`, `serde_yaml`, `brightfield-sql` (parse), and the
workspace `sqlparser` version (tokenizer for statement splitting). No gpui, no vello, no
duckdb. Everything deterministic: `BTreeMap`/`BTreeSet`/`Vec` only — **no `HashMap` anywhere
on the graph/layout path** (pds-ac06).

### 1.1 `manifest.rs` — the interim contract parser

Serde model of arcform's manifest as it exists today (see the vendored fixture):

```
Manifest { name, engine, engine_version, db, params, defaults, steps: Vec<Step> }
Step {
  name: String,
  op: Option<String>,        // "http_fetch@1" — split into (name, version)
  with: Option<Mapping>,     // preserved raw for the seam label
  sql: Option<String>,       // model path relative to the manifest dir
  command: Option<String>,   // opaque-by-design steps
  depends_on: Vec<String>,   // file/table refs (default empty)
}
```

Unknown keys are **ignored, never errors** (the interim parser must survive manifest
evolution). `op` without `@` ⇒ version `"?"`. A step with none of op/sql/command parses as
an opaque step (renders like command). SQL model files are read relative to the manifest's
parent dir; a missing model file is a **step-level degrade** (opaque chip for that step's
SQL), not a parse failure.

### 1.2 `sql_assets.rs` — per-statement asset extraction

- `split_statements(sql: &str) -> Vec<String>`: sqlparser `Tokenizer` over the DuckDb
  dialect; split on `Token::SemiColon` at top level (the tokenizer already consumes
  strings/comments so a `;` inside a literal never splits). Empty/whitespace-only
  fragments dropped. If tokenisation itself fails, the whole file becomes ONE opaque
  fragment (still a chip, never a silent skip).
- Per statement, via `brightfield_sql::conform::parse_and_normalise`:
  - `CREATE [OR REPLACE] TABLE|VIEW <name> AS …` ⇒ produced relation `<name>`.
  - Table-factor refs in the statement ⇒ consumed relations; `read_parquet('p')` /
    `read_csv[_auto]` / `read_xlsx` / `read_json[_auto]` table functions ⇒ consumed FILE
    paths (first string literal argument).
  - Statement parse error ⇒ `StatementAssets::Opaque { index, error }` (pds-ac04) — the
    fragment keeps its byte range so later cards can highlight it.
- Produced-relation typing: the LAST producing statement in the LAST SQL step that
  produces a given relation is TABLE; producing statements whose relation is consumed
  later within the same file and never referenced by any other step are INTERNAL.
  (Statement-level only — CTE parsing is a later card.)

### 1.3 `graph.rs` — the typed asset graph

```
AssetKind  { Source, File, Table, Internal, Dataset }
AssetNode  { id: AssetId, kind, label, step: Option<StepId> }   // dotted ids, stable
SeamKind   { Op { name, version }, Sql { model }, Command, Opaque }
Seam       { step: StepId, kind, gate: bool }                    // gate = finetype_validate
Edge       { from: AssetId, to: AssetId, via: Option<StepId>, shield: bool }
AssetGraph { nodes: BTreeMap<AssetId, AssetNode>, seams: BTreeMap<StepId, Seam>, edges: Vec<Edge> }
```

Derivation rules (interim, from manifest + sql_assets):
- `with.url` ⇒ SOURCE node (label = host); `with.out` / `with.dest` ⇒ FILE node (label =
  path); `with.input` ⇒ edge from that TABLE relation; `depends_on` entries ⇒ edges from
  the FILE/TABLE node they name (path-looking ⇒ FILE, bare ident ⇒ TABLE).
- `sql:` steps contribute their per-statement produced/consumed sets.
- The manifest's terminal exported parquet (the `parquet_export` step whose `dest` no other
  step reads) ⇒ its FILE node is re-kinded **Dataset** (the sink).
- `finetype_validate` steps produce **no node**: `gate: true` on the seam, and the edge
  into the asset named by `with.parquet`/`with.schema`'s guarded target carries
  `shield: true` (pds-ac05).
- Node ids: `asset.<protocol>.<name>` (relation) / `file.<protocol>.<path>` /
  `stmt.<protocol>.<step>#<n>` for INTERNAL statement intermediates.

### 1.4 `collapse.rs` — parameterised family collapse

Detection (pds-ac03): maximal runs of ≥2 step *pairs* whose names share a prefix and
differ only in a trailing token (`fetch_ncen_2026q2`/`extract_ncen_2026q2`, …). Rule:
strip the last `_`-separated token from each step name; group consecutive steps whose
stripped names form a repeating cycle over the same op sequence. Each family becomes one
`FamilyTile { label, count }` graph node; member nodes/edges are removed and external
edges re-target the tile (the deepest-visible-ancestor rule). Collapse is a pure
`AssetGraph -> AssetGraph` fold, unit-testable without pixels.

### 1.5 `layout.rs` — deterministic Sugiyama

`layout(graph: &AssetGraph, config) -> Layout { positions: BTreeMap<NodeId, Rect>, lanes }`
- Longest-path layering left→right (sources col 0, Dataset sink last).
- Dummy nodes for edges spanning >1 layer (edge lanes).
- Crossing reduction: exactly **4 median sweeps** (fixed), ties broken by node id
  (lexicographic) — never by insertion or hash order.
- Coordinates quantised to whole pixels. Same graph in ⇒ identical `Layout` out
  (pds-ac06 pins this with a repeated-call equality test on the edgar_gleif fixture).

## 2. `brightfield-render/src/asset_scene.rs`

One pure fn in the `scene.rs` idiom: `render_asset_graph(scene: &mut vello::Scene,
layout: &Layout, graph: &AssetGraph)`. Node cards per kind (SOURCE pill / FILE document
silhouette / TABLE card / INTERNAL smaller muted card / DATASET double-ring / family tile
with `×N` count), seam chevrons on edge bundles, shield glyph on `shield` edges,
orthogonal edge routing along the dummy-node lanes. Ink from the meridian-design tokens
already used by scene.rs; labels via the existing `text` module (Inter). No text
measurement in layout — card widths come from char-count heuristics in
`brightfield-protocol` (the Dagster trick: geometry before pixels).

## 3. `brightfield-app` wiring

- **Sniff** (before Mosaic spec parsing): a YAML whose top level has a `steps:` sequence
  and NO `plot:`/`data:` keys is a Protocol manifest. Mosaic specs are untouched
  (pds-ac08).
- **Dump arm**: `BootMode::HeadlessDump` + protocol input ⇒ parse → graph → collapse →
  layout → `render_asset_graph` → the exact `VelloRenderer::render_to_pixels` → PNG →
  `return` shape the dashboard arm uses (same returns-before-workspace guarantee).
- **Window arm**: protocol input ⇒ eprintln a clear "the windowed protocol view lands in
  a later card — use BRIGHTFIELD_DUMP_PNG=<path> for the DAG render" and exit 0
  (pds-ac07).

## 4. Fixtures + tests

- `examples/protocol/edgar_gleif/{arcform.yaml, models/*.sql}` vendored verbatim from the
  public open-analytics repo (MIT, same project family; provenance note in a sibling
  README.md). Plus `examples/protocol/degrade.yaml` + one model with a deliberately
  unparseable middle statement (pds-ac04 fixture).
- Unit: manifest parse (op/sql/command/unknown-keys), splitter (semicolon-in-string,
  comment, empty fragments), per-statement degrade, asset derivation per rule, family
  collapse fold, layout determinism (repeated-call equality + no-HashMap review), tier.sql
  statement chain (pds-ac02 as a graph assertion, not pixels).
- Integration (dump_seam idiom): edgar_gleif dump non-empty + byte-identical twice
  (pds-ac01); degrade fixture dumps with the chip (coverage assertion); full existing
  gallery untouched (pds-ac08 rides the existing example tests).

## 5. Out of scope (later cards)

CTE parsing/hulls, DESCRIBE/EXPLAIN measurement, windowed ProtocolPanel, keyboard
altitude, run states/rail/diff, selector bar, inspector, outline, sheets, mosaic panel
attachment. The Protocol+Run JSON contract replaces `manifest.rs` when arcform emits it —
keep that module thin and marked `// INTERIM CONTRACT`.
