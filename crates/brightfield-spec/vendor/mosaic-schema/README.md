# Vendored Mosaic spec schema

Mosaic's published JSON Schema for its declarative spec, copied byte for byte. It sits beside the example corpus in `../mosaic-specs/`, taken from the same upstream commit, so the schema describes the vocabulary the corpus is written in.

## Upstream

- **Repo:** https://github.com/uwdata/mosaic
- **Commit SHA:** `d4d41a3275dbd6bc7995e1d1a82b0be18769bbca` (tag `v0.24.2`), the commit `../mosaic-specs/README.md` names
- **Date:** 2026-04-12
- **Path in upstream:** `docs/public/schema/v0.24.2.json`. At that commit `docs/public/schema/latest.json` is a symbolic link to the same file; the site serves it as the schema for Mosaic 0.24.2.
- **SHA-256 of the copy:** `16f04664e577eb4f0f0c29e88c62b032899810493f4d20652c19034e97bdd5f4`

## What reads it

`crates/brightfield-spec/build.rs` reads `definitions.PlotAttributes.properties` and keeps each axis attribute name: `grid` and `axis` and `align` and `padding` and `xyDomain` and `facetGrid` and `facetLabel`, which carry no `x` or `y` before a capital letter and are listed by name in `crates/brightfield-spec/src/axis_vocabulary/schema.rs`; every name that is `x` or `y` followed by a capital letter; and every name that is `fx` or `fy` followed by a capital letter, a facet axis's. `facetMargin` and the other `facet…` names are not axis attributes by that rule. The names become `brightfield_spec::axis_vocabulary::SCHEMA_AXIS_ATTRIBUTES`, and a plot that sets one of them which brightfield does not read is named in a `ParseWarning::UnreadAxisAttribute`. Removing a name from this file stops that warning for it, because the list is read from here at build time and nowhere else.

## Refresh procedure

1. `cd <path to upstream mosaic clone>` and check out the commit the corpus is vendored from.
2. `cp docs/public/schema/v<version>.json <this dir>/` and delete the old file.
3. Update the path in `crates/brightfield-spec/build.rs` (`SCHEMA_PATH`), and the **Commit SHA**, **Date**, **Path** and **SHA-256** entries above.
4. Re-run `cargo test -p brightfield-spec --test axis_vocabulary_unread`. A new axis name in the schema that a corpus spec carries is a real signal: either brightfield reads it now (add it to `READ_AXIS_ATTRIBUTES`) or the corpus test's expected list grows.

## Licensing

Upstream Mosaic is BSD-3-Clause (Copyright (c) 2023-2025, UW Interactive Data Lab). The schema is reproduced unmodified under the same licence terms, for reading the names of Mosaic's plot attributes.
