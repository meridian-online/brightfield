---
status: accepted
date-created: 2026-04-21
date-modified: 2026-04-21
---
# 0001. Conformance fixtures co-located with curated specs

## Context and Problem Statement

Layer-2 conformance uses golden SQL snapshots (`.layer2.expected.sql`) to verify
that the emitter produces correct DDL for each curated spec. These fixtures
contain file paths like `data/flights-200k.parquet` because the curated specs
they validate are vendored Mosaic examples that reference local files.

The question is where these test fixtures should live: alongside the curated
specs in `vendor/curated/yaml/`, or in a dedicated test fixtures directory.

This also surfaces a broader boundary concern: the curated specs hard-code file
paths, which could be mistaken for production data flow. In production, arcform
owns data materialisation and authors (or resolves) the spec's `data:` section.
Brightfield is source-agnostic — it emits SQL for whatever the spec declares.

## Considered Options

- **Option A: Co-locate** — `.layer2.expected.sql` files sit next to the `.yaml`
  specs they validate in `vendor/curated/yaml/`.
- **Option B: Separate** — Move fixtures to `tests/fixtures/layer2/` with a
  naming convention that maps back to curated specs.

## Decision Outcome

Chosen option: **Option A (co-locate)**, because the pairing between spec and
fixture is immediately obvious by adjacency (`flights-200k.yaml` next to
`flights-200k.layer2.expected.sql`). The `SqlEquivalenceCheck` already resolves
the fixture path as a sibling of the spec file — moving fixtures would require
changing that lookup logic for no functional gain.

### Consequences

- Good, because the relationship between spec and fixture is self-documenting
- Good, because `SqlEquivalenceCheck` uses simple sibling-file resolution
- Good, because adding a new curated spec and its fixture is a single-directory operation
- Bad, because `vendor/curated/yaml/` contains both source specs and test artefacts, which could confuse newcomers
- Mitigated by the `.layer2.expected.sql` suffix clearly marking these as conformance artefacts, not source specs

### Boundary clarification

The curated specs are **test inputs**, not production configuration. In
production, arcform materialises data and authors the `data:` section of specs
that brightfield consumes. The `file: data/flights-200k.parquet` paths in curated
specs are Mosaic's example conventions — they prove the emitter works, they do
not encode a production data pipeline.
