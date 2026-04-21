# Implementation Progress

Spec path: orbit/specs/2026-04-21-direct-data-source-loading/spec.yaml
Spec hash: sha256:02538542c75fd1e17ba2ba4901845db7e3763db0df5f65c4cc50f37bc29b1928
Started: 2026-04-21
Current AC: ac-16

## Hard Constraints
- [x] New crate `brightfield-sql` at `crates/brightfield-sql/`. Depends on `brightfield-spec` (path). Workspace member. No DuckDB runtime dependency.
- [x] `ParseOutput` gains `base_dir: Option<PathBuf>`. Accepted tech debt (no `#[non_exhaustive]`).
- [x] Extension-sniffing dispatch with Typed/Opaque error handling.
- [x] CSV reader option allow-list: delim, header, columns, types, skip, nullstr.
- [x] Inline rows: insertion-order column names, 1000-row cap.
- [x] DuckDB ATTACH uses READ_ONLY. Deviation DEV-0002 (DEV-0001 was already taken by the rendering deviation).
- [x] Emission is pure function — no I/O, no DuckDB connection.
- [x] SqlEquivalenceCheck flips from Pending to string-diff for DDL slice.
- [x] Expected SQL fixtures use canonical form via render.rs.
- [x] HTTP(S) URLs pass through verbatim.

## Detours

2026-04-21: DEV-0001 already taken by rendering deviation — used DEV-0002 for ATTACH read-only divergence. Also updated DEV-0001 to remove layer 2 from suppression (now active).
Return to: ac-15

## Acceptance Criteria
- [x] ac-01: New crate brightfield-sql with module layout — crates/brightfield-sql/src/{lib,error,emit,source,render}.rs
- [x] ac-02: EmitError enum with thiserror — UnknownFormat, InlineRowLimit, InvariantViolation
- [x] ac-03: ParseOutput.base_dir field — parse_spec_path populates, parse_spec leaves None
- [x] ac-04: SourceDdl struct and emit_sources entry point — dispatches all DataSourceKind variants
- [x] ac-05: Parquet emission with path resolution — read_parquet(), HTTP passthrough
- [x] ac-06: CSV emission with allow-listed extras — auto_detect=true, kwargs in alphabetical order
- [x] ac-07: JSON/GeoJSON/spatial emission — read_json_auto(), ST_Read(), .geojson requires type:spatial
- [x] ac-08: DuckDB ATTACH emission — ATTACH … AS … (READ_ONLY)
- [x] ac-09: Inline-row emission with VALUES clause — object-per-row and array-per-row, 1000-row cap
- [x] ac-10: Query and Shorthand emission — CREATE OR REPLACE VIEW wrapping
- [x] ac-11: canonicalise_ddl in render.rs — sorts by view_name, normalises kwargs, whitespace
- [x] ac-12: SqlEquivalenceCheck activation in conformance — reads .layer2.expected.sql, string-diffs
- [x] ac-13: Layer-2 expected SQL fixtures for curated corpus — 10 files generated
- [x] ac-14: Update curated .expected.yaml layer_2 values — all flipped from pending to pass
- [x] ac-15: DEV-0002 deviation registry entry — ATTACH read-only divergence
- [x] ac-16 (gate): Workspace tests and compilation pass — cargo test --workspace green, cargo check clean
- [x] ac-17: Error-path dispatch for Typed/Opaque/unknown extension — InvariantViolation and UnknownFormat
