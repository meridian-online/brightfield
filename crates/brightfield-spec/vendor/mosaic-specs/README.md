# Vendored Mosaic spec corpus

Reference corpus vendored from upstream Mosaic for use by the
`brightfield-spec` crate's corpus totality test and structural tests.

## Upstream

- **Repo:** https://github.com/uwdata/mosaic
- **Commit SHA:** `d4d41a3275dbd6bc7995e1d1a82b0be18769bbca`
- **Date:** 2026-04-12
- **Path in upstream:** `docs/public/specs/yaml/`

## Contents

- `yaml/*.yaml` — 54 YAML specifications covering the full breadth of the
  Mosaic 0.24.x vocabulary (marks, interactors, inputs, legends, concats,
  selections, parametrised queries, SQL expressions).

## Refresh procedure

1. `cd <path to upstream mosaic clone>`
2. `git pull origin main`
3. `git rev-parse HEAD` → new SHA
4. `cp docs/public/specs/yaml/*.yaml <this dir>/yaml/`
5. Update the **Commit SHA** and **Date** entries above.
6. Re-run `cargo test -p brightfield-spec --test corpus_totality`. Any regression
   is a real signal — either Mosaic added new vocabulary (update
   `src/vocab.rs`), or the parser regressed.

## Licensing

Upstream Mosaic is BSD-3-Clause. The vendored YAML files are specifications
(data, not code) reproduced under the same licence terms for reference-corpus
testing purposes.
