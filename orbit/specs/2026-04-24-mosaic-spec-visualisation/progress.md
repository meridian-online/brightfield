# Implementation Progress

Spec path: orbit/specs/2026-04-24-mosaic-spec-visualisation/spec.yaml
Spec hash: sha256:01fb64204ad939cfabafbee1a959495eb8bdbc7ba43551c6be0bcc69d8b68b09
Started: 2026-04-24
Current AC: ac-07

## Hard Constraints
- [x] brightfield-render must NOT gain dependencies on brightfield-engine or brightfield-sql
- [x] brightfield-ui must NOT gain dependencies on brightfield-engine or brightfield-sql
- [x] All existing tests across all crates must continue to pass
- [x] The pipeline must handle marks that fail to lower gracefully
- [x] The app binary accepts a spec file path as a CLI argument
- [x] SimpleLowerer emits QueryPlan::Source (SELECT * FROM view)
- [x] ChannelMap::from_mark extracts literals only; ParamRef skipped
- [x] infer_scales_multi and build_multi_mark_scene are additive
- [x] The orchestration pipeline is sequential and blocking

## Detours
- Arrow version upgrade: brightfield-render upgraded from arrow 54 to 58 to match duckdb's arrow version, enabling RecordBatch interop in brightfield-app
- MarkKind::Bar doesn't exist: spec said "Bar, BarX, BarY" but enum only has BarX and BarY; registered 4 kinds instead of 5
- GPUI window deferred: full Xcode + Metal compiler needed for gpui_macos; pipeline runs headlessly with scene stats output

## Acceptance Criteria
- [x] ac-01: SimpleLowerer implements MarkLower, registered for Dot/Line/BarX/BarY
- [x] ac-02: infer_scales_multi with domain union across batches
- [x] ac-03: build_multi_mark_scene with shared scales
- [x] ac-04: brightfield-app binary crate with orchestration pipeline
- [x] ac-05: Graceful mark failure handling (skip + warn)
- [x] ac-06: ChannelMap::from_mark logs warning on ParamRef
- [x] ac-07 (gate): All existing tests pass — cargo test --workspace (337 passed, 0 failed)
