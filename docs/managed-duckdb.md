# The managed edition's DuckDB

Brightfield's managed edition, the build for the Mac App Store, runs on one DuckDB library, `libduckdb.dylib`, which Meridian builds from DuckDB's source with the extensions the edition needs compiled in. Brightfield links it, and DuckDB's own command-line shell, which arc runs a Protocol's SQL steps with, loads the same file. The full edition is unchanged: it links duckdb-rs's bundled DuckDB and fetches extensions as before.

## Why it is built this way

A Mac App Store app may not download code, and every piece of code in its bundle is signed with the developer's distribution identity. A DuckDB extension file cannot be signed: DuckDB appends its metadata and signature after the Mach-O image, and `codesign` refuses a file with bytes after its signature. So the extensions are compiled into the library, and no `.duckdb_extension` file ships.

Hugh chose on 2026-11-17 to build DuckDB once, as a library in the bundle that both Brightfield and the shell load, rather than a static DuckDB inside Brightfield and a second copy inside the shell. The app then carries one copy of DuckDB and its extensions, and the engine that runs a Protocol and the engine that reads its result in the window are the same build.

The built-in extensions are autocomplete, excel, sqlite, httpfs, ducklake, spatial and FineType, with DuckDB's own core_functions, parquet, json and icu, which its release shell also links. Spatial stays in the set at the sizes measured below. Hugh ruled that a size well above those goes back to him rather than dropping spatial, because spatial and Mosaic are where Protocols show the most.

## How it is built

`scripts/build-duckdb.sh` builds it into `target/duckdb-managed/`: `lib/libduckdb.dylib` with the install name `@rpath/libduckdb.dylib`, `include/`, and `bin/duckdb`, the shell. It drives DuckDB's own build, `make release`, with `scripts/duckdb-managed/extensions.cmake` as its extension configuration.

- **Pins.** DuckDB v2.0.0 at `d54cbbe3df`, fetched by commit and checked. The five out-of-tree extensions are at the commits DuckDB v2.0.0 itself pins, so this is the extension code the DuckDB release published as files. vcpkg, which builds the C libraries the extensions need (GDAL, PROJ, GEOS, curl, OpenSSL and others), is at the commit DuckDB's release workflows use, with a triplet that builds them for release only and for macOS 11, DuckDB's own floor (`scripts/duckdb-managed/triplets/`). FineType is at the tag `packaging/finetype-pin.env` names, and its commit is checked.
- **The shell links the library.** Two checked edits to DuckDB's `tools/shell/CMakeLists.txt` point the shell at the shared library instead of a static copy. Its rpaths are `@executable_path/../lib`, for the build's layout, and `@executable_path/../Frameworks`, for an app bundle's `Contents/Helpers/`.
- **FineType links statically, unchanged.** FineType is a Rust extension written against DuckDB's C extension API. DuckDB 2.0 links such an extension statically through a describe function (`duckdb_static_extension.h`), but loads it with no function table, expecting it to call DuckDB's C functions directly. FineType's bindings, duckdb-rs's `loadable-extension`, only call through that table. `scripts/duckdb-managed/finetype/` wraps FineType's entry point in a small one that hands it the table DuckDB gives a loaded extension file, built by DuckDB's own `CreateAPIv1()`. So the design's fallback, shipping without in-process FineType typing, was not needed.
- **The sqlite extension's own SQLite is renamed.** The sqlite extension compiles its own SQLite, and spatial links vcpkg's for GDAL and PROJ. In one library their `sqlite3_*` symbols collide. A patch in `scripts/duckdb-managed/patches/sqlite_scanner/`, applied through DuckDB's own extension-patch mechanism, renames the sqlite extension's copy at compile time with a force-included header. Neither extension's code changes.

A cold build is 30 to 90 minutes, most of it the C libraries spatial needs; vcpkg keeps those in its binary cache, so a later build skips them.

## How Brightfield links it

Every DuckDB user in Brightfield's graph (Brightfield, arcform and dovetail) asks duckdb-rs for its bundled build, and a Cargo feature cannot be switched off once a crate turns it on. So libduckdb-sys is vendored in `vendor/libduckdb-sys/` with one added feature, `managed-linked`, which links the managed library through the crate's own linked-library path and compiles no bundled DuckDB. Brightfield's `managed-edition` feature turns it on. `vendor/libduckdb-sys/BRIGHTFIELD.md` says how to carry the change to a new version, and `scripts/check-vendored-libduckdb-sys.sh` holds the vendored copy to the bytes crates.io publishes, with the change applied from `vendor/libduckdb-sys/BRIGHTFIELD.patch`.

The managed edition's app binary carries the rpath `@executable_path/../Frameworks`, so from `Contents/MacOS/` it loads `Contents/Frameworks/libduckdb.dylib`. Its test binaries carry an rpath to the built library instead; no build path goes into a binary that ships.

## What checks it

- `scripts/build-duckdb-selftest.sh`, run by the `duckdb-managed` step after the build: the shell's `--version` prints `v2.0.0 (Cyanoptera) d54cbbe3df`, as the release does and as arc checks; the seven extensions are loaded and statically linked, and autocomplete, spatial and FineType answer; no extension file exists and none is written; the shell links exactly `@rpath/libduckdb.dylib`, binds DuckDB's symbols from it, and at run time loads that library and no other, from the build's layout and from an app bundle's.
- `crates/brightfield-engine/tests/managed_duckdb.rs`, in the `managed-duckdb-test` step: with the network denied and an empty extension directory, the engine reads a workbook and a SQLite file in process; and the test binary links the library and defines none of DuckDB's C API itself.
- `crates/brightfield-shell/tests/managed_duckdb_link.rs`, in the same step: Brightfield, laid out as the bundle lays it out, loads `Contents/Frameworks/libduckdb.dylib` and no other DuckDB.

Both steps are CI-only, since the local pull-request gate builds in a clean worktree and a cold build would add up to 90 minutes to every pull request. `.github/workflows/managed-duckdb.yml` runs them on the pull requests that change the build or the feature's wiring.

## Sizes

Measured on 2026-11-17 on Apple Silicon, release builds:

| | bytes |
|---|---|
| `libduckdb.dylib`, DuckDB with the eleven extensions | 132,688,400 |
| DuckDB's shell, linking that library | 958,864 |
| DuckDB v2.0.0's release shell, for comparison | 66,244,528 |
| Brightfield's binary, managed edition | 123,431,232 |
| Brightfield's binary, full edition, with its static DuckDB | 184,100,272 |

So the managed edition's DuckDB, library and shell together, is about 133.6 MB, against about 126.9 MB of DuckDB in the full edition's bundle before any extension is fetched: the 60.7 MB by which its Brightfield binary is larger, and the release shell. The full edition's v0.1.8 disk image is 117.9 MB. The managed edition's disk image is measured when its packaging is built.

## Open

- GEOS, which spatial uses, is LGPL-2.1, and it is linked statically into the library. Every other C library the build links is under a permissive licence. The full edition never distributes GEOS, since spatial is fetched from DuckDB at run time; the managed edition would. How the Store build meets the LGPL is Hugh's decision.
- An `INSTALL` of an extension that is already compiled in still goes to the network in this build, as in DuckDB's release. Offline it fails; online it would fetch an extension file. Turning installs and loads off in the managed edition is its own piece of work, which handles it.
