# libduckdb-sys, vendored

This is libduckdb-sys 1.20000.0 as crates.io publishes it (duckdb-rs commit `1b8f7cca6e28c5ea8c4185aaa266424f25c3dfdc`, recorded in `.cargo_vcs_info.json`), with one feature added. The workspace's `Cargo.toml` puts it in place of the crates.io copy through `[patch.crates-io]`.

## The change

`managed-linked`, a new feature, makes the build script link the DuckDB library Brightfield's managed edition builds (`scripts/build-duckdb.sh`, into `target/duckdb-managed/`) instead of compiling the bundled DuckDB. Without the feature the crate builds exactly as published.

The files that differ from the published crate:

- `managed_linked.rs`, new: finds the managed library and hands it to the crate's own linked-library path, `build_linked.rs`, as `DUCKDB_LIB_DIR` would.
- `build.rs`: `main` runs `managed_linked::main` when the feature is on, and `build_linked` and `build_paths` compile when it is on as well as without `bundled`.
- `Cargo.toml`: the `managed-linked` feature.
- `BRIGHTFIELD.md`, this file, and `BRIGHTFIELD.patch`, the change to `build.rs` and `Cargo.toml` as a diff against the published files. `scripts/check-vendored-libduckdb-sys.sh` applies it to the published crate and requires the result to equal these files byte for byte. `.cargo-ok`, Cargo's unpack marker, is left out.

## Why it is vendored

Brightfield, arcform and dovetail each depend on `duckdb` with `bundled`. Cargo features only add, so once any crate turns `bundled` on, nothing else in the build can turn it off, and the managed edition would compile and link a second DuckDB of its own. The only place to switch the bundled build off is libduckdb-sys's build script.

## Moving to a new version

When the workspace's `duckdb` moves, copy the new libduckdb-sys out of Cargo's registry (`~/.cargo/registry/src/*/libduckdb-sys-<version>`) over this directory, apply `BRIGHTFIELD.patch` to it (`patch -p0 -V none < BRIGHTFIELD.patch`, from this directory; `-V none` keeps patch from writing a backup over the crate's own `Cargo.toml.orig`), move the published sha256 in `scripts/check-vendored-libduckdb-sys.sh`, and move the version in the workspace's `libduckdb-sys` entry with `duckdb`'s.
