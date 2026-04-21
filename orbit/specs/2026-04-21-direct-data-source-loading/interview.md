# Interview — Card 0004: Direct Data Source Loading

Card: `orbit/cards/0004-direct-data-source-loading.yaml`
Rally: `layer 2 SQL emission` (`orbit/specs/2026-04-21-layer-2-sql-emission-rally/rally.yaml`)
Decision pack: `orbit/specs/2026-04-21-direct-data-source-loading/decisions.md`
Mode: rally design — decision pack authored by forked sub-agent, all eight decisions approved wholesale at the consolidated decision gate.

## Card summary

| Field | Value |
|-------|-------|
| Feature | Direct data source loading |
| As a | analyst with data in common local formats |
| I want | to point brightfield at a Parquet, CSV, JSON, or DuckDB file and start exploring |
| So that | I don't need a separate ingestion step just to look at my data |
| Goal | All five formats named in the brief (Parquet, CSV, JSON, inline, DuckDB) loadable by path reference from a spec |

Scenarios (4):
1. Open a Parquet file by path
2. Attach to an existing DuckDB database file
3. Load a CSV file with inferred column types
4. Inline data in the spec works without an external file

## Context

The parser (card 0001) already accepts all five formats structurally — `DataSourceKind::{File | Query | InlineRows | Typed | Opaque}` covers the surface. What's missing is the Layer-2 emitter that translates each `DataSource` AST node into DuckDB SQL that registers or reads the source.

Fixed constraints inherited from card 0001 (`crates/brightfield-spec/src/parse.rs:458-517`):
- `DataSource { kind, extras: IndexMap<String, SpecValue> }` preserves siblings (`select:`, `where:`, `type:`, `layer:`) verbatim.
- The 54-spec vendored Mosaic corpus at `crates/brightfield-spec/vendor/mosaic-specs/yaml/` is the corpus this emitter must make runnable: every one uses relative `file:` paths, 8 use `type: spatial` for GeoJSON, none currently reference `.duckdb` files or top-level inline data.

Conformance gate: `SqlEquivalenceCheck` at `crates/brightfield-conformance/src/layer.rs:162-178` — today `Pending`, will flip to a real pass/fail for the data-source DDL slice this card ships.

## Approved decisions

### D1 — Path resolution against spec-file parent directory

Add `base_dir: Option<PathBuf>` to `ParseOutput`. `parse_spec_path` populates it from `source_path.parent()`; `parse_spec(&str, Format)` (tests, pasted input, REPL) leaves it `None` — emitter falls back to CWD in that case. HTTP(S) URLs in `file:` values pass through verbatim. This is a small additive change to card 0001's parser surface; it unblocks all 54 corpus specs running from any shell CWD.

### D2 — Extension-sniffing format dispatch with `type:` override

`*.parquet` → `read_parquet()`, `*.csv`/`*.tsv` → `read_csv(..., auto_detect=true)` (D3), `*.json`/`*.ndjson` → `read_json_auto()` (D4), `*.geojson` → emit `EmitError::UnknownFormat` unless `type: spatial` is present, `type: spatial` → `ST_Read()`, `*.duckdb`/`*.db` → `ATTACH` (D5). Unknown extension → `EmitError::UnknownFormat { path, extension }`.

### D3 — `read_csv(path, auto_detect=true, …extras)` with allow-listed options

Fixed allow-list: `delim`, `header`, `columns`, `types`, `skip`, `nullstr`. Extras outside the allow-list → `ParseWarning::UnknownOption` (card 0001's warning type, reused). Honours "DuckDB-inferred column types" by default and preserves Mosaic-web's escape hatches.

### D4 — JSON flavour: type-driven, no content sniffing

`.json`/`.ndjson` → `read_json_auto(path, format='auto')` — DuckDB's own detection inside handles array vs NDJSON. `.geojson` without `type: spatial` → emit error. `type: spatial` → `ST_Read(path, layer=<extras.layer>)`. Emission stays a pure function of the spec + `base_dir` — no I/O at emit time. Critical for D6's reactive lifecycle and D7's deterministic snapshots.

### D5 — `ATTACH '<path>' AS "<data-key>" (READ_ONLY)`

Use the `data:` map key as the attach alias (quoted for non-ident-safe keys). `READ_ONLY` is a deliberate safety divergence from Mosaic-web — the card's scope is read-only exploration; prevents corrupting a production DuckDB file the user pointed at. Registered as a deviation entry. Name collision cannot occur — `data:` is a map, parser inherits `IndexMap` uniqueness from card 0001.

### D6 — `CREATE OR REPLACE VIEW` at spec-mount

Two-phase emission: **(i) setup DDL** — one `CREATE OR REPLACE VIEW "<name>" AS <source-sql>` per source at spec-mount; **(ii) per-plot SELECTs** that reference the view by bare name. Author's `where:`/`select:` extras get applied once inside the view. This is the handshake with card 0003's reactive hot path (D5 shape-cache) — a param change re-emits the SELECT, not the view.

`DataSourceKind` dispatch inside the view body:
- `File` → extension-dispatched reader call (D2-D5)
- `Query` → author-SQL as view body (already SQL)
- `InlineRows` → `VALUES` clause (D8)
- `Typed("spatial")` → `ST_Read` call

### D7 — `<name>.layer2.expected.sql` string-snapshot with canonicalisation

Per-corpus-spec sibling file listing the source-DDL block in a fixed format: view-name alphabetical, kwargs alphabetical, whitespace normalised. `SqlEquivalenceCheck` for v1.1 flips from `Pending` to a string-diff against this file, scoped to the data-source portion. Plot-SELECT conformance stays `Pending` until card 0003 ships.

Split across siblings intentionally: 0004's string-snapshot on DDL + 0003's sqlparser-rs structural diff on query. DDL is simple and diffable; query SQL is structurally rich and benefits from AST-level comparison.

### D8 — Inline rows via `VALUES` inside view body, 1000-row cap

`CREATE OR REPLACE VIEW "<name>" AS SELECT * FROM (VALUES (…), …) AS t("col1", …)`. Column names from first row's keys (for object-per-row) or synthesised `c0, c1, …` (for array-per-row). Rows > 1000 → `ParseError::MalformedDataDef { detail: "inline row count {n} exceeds 1000 — use a file source" }`. Keeps snapshots reviewable; discourages inline-rows as a data-loading strategy.

## Noted divergence

**D5's `READ_ONLY` on `ATTACH`** — add an entry to the deviation registry (card 0002 mechanism) as `DEV-NNNN: ATTACH read-only divergence`. This is the one emission-side departure from Mosaic-web's wire shape in this card. Justified by the "exploration" framing — the card's stated scope is reading, not writing.

## Implementation surface

### New crate: `brightfield-sql` (emitter scaffold)

This card **establishes** the emitter crate. Card 0003 extends it. Module layout for 0004's portion:
- `error.rs` — `EmitError::{UnknownFormat, InvariantViolation, …}`
- `source.rs` — `DataSourceEmitter`: per-`DataSourceKind` DDL emission
- `emit.rs` — public `fn emit_sources(spec: &Spec, preflight: &SupportReport) -> Result<Vec<SourceDdl>, EmitError>`
- `render.rs` — minimal canonicalisation helpers used by both 0004's DDL-snapshot shape and (later) 0003's query-SQL shape

Output type (shared with 0003, established here):
```rust
pub struct SourceDdl {
    pub view_name: String,
    pub sql: String,      // the canonicalised CREATE OR REPLACE VIEW statement
    pub source_kind: SourceKindTag,  // for conformance classification
}
```

### Modified: `crates/brightfield-spec/src/parse.rs`

Additive: `ParseOutput { spec, warnings, base_dir: Option<PathBuf> }`. `parse_spec_path` populates `base_dir`; `parse_spec` does not. Existing callers unaffected (adding a field to a pub struct is a breaking change in strict interpretation — but this crate hasn't shipped to external consumers; rally-internal).

### Modified: `crates/brightfield-conformance/src/layer.rs`

Shared with card 0003. 0004 implements `SqlEquivalenceCheck` for the data-source-DDL portion only; when 0003 lands, the method extends to cover query SQL. Expectation files (`<name>.expected.yaml`) gain a `layer_2` value that can be `pass`/`pending`/`suppressed(DEV-NNNN)` per-slice.

### New conformance fixtures

`crates/brightfield-conformance/vendor/curated/yaml/<name>.layer2.expected.sql` — one per curated spec, capturing the expected data-source-DDL block in canonical form.

## Cross-card touchpoints

- **Card 0003 (sibling in rally).** Shared crate `brightfield-sql`, shared `SqlEquivalenceCheck` in `layer.rs`. 0004 lands first (establishes crate scaffold + DDL emission + D6 view-registration lifecycle that 0003 D5 depends on); 0003 extends. **Serial ordering**.
- **Card 0001 (shipped).** AST `DataSource`, `DataSourceKind`, `SpecValue`, `ParseOutput` surface extended with `base_dir`. Additive only.
- **Card 0002 (shipped).** Deviation registry gains `DEV-NNNN: ATTACH read-only` entry; expectation-file schema accepts `layer_2: pass` for the DDL slice.
