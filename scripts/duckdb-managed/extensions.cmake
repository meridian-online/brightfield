# The DuckDB extensions compiled into the managed edition's DuckDB.
#
# scripts/build-duckdb.sh hands this file to DuckDB's own build as its extension
# configuration. Every extension named here is built from source and linked into
# libduckdb.dylib, the one DuckDB library the managed edition's app carries, which
# Brightfield links and DuckDB's command-line shell loads. No `.duckdb_extension`
# file is built or shipped: Apple's codesign refuses any file with bytes after its
# signature, and DuckDB appends its metadata and signature block to every
# extension file, as measured on macOS 27.2.
#
# The five out-of-tree extensions are pinned to the commits DuckDB v2.0.0 itself
# pins in .github/config/extensions/<name>.cmake at d54cbbe3df, so this build
# compiles the same extension code the DuckDB release published as loadable
# files. Bumping DuckDB moves these with it: copy the new release's GIT_TAGs.
#
# A git commit names its tree by hash, so a GIT_TAG here is the checksum of the
# source it fetches: DuckDB's build checks the commit out by that hash.

# DuckDB's own in-tree extensions, which its release shell links as well. Keeping
# them makes `duckdb_extensions()` agree with the release shell, which the parity
# check compares against. autocomplete is the one of the seven built-ins that is
# in-tree; arc's lineage tokenizer calls its `sql_tokenize`.
duckdb_extension_load(core_functions)
duckdb_extension_load(parquet)
duckdb_extension_load(json)
duckdb_extension_load(icu)
duckdb_extension_load(autocomplete)

duckdb_extension_load(excel
    GIT_URL https://github.com/duckdb/duckdb-excel
    GIT_TAG 81bb84b115b97cc4edec4827714cac80da0127e1
    INCLUDE_DIR src/excel/include
    )

# APPLY_PATCHES here applies Brightfield's own patch, from
# scripts/duckdb-managed/patches/sqlite_scanner/, which scripts/build-duckdb.sh
# copies to where DuckDB's build reads extension patches. This extension
# compiles its own SQLite and spatial links vcpkg's, and in one library their
# sqlite3_* symbols collide; the patch renames this extension's copy, at compile
# time, and changes none of its code.
duckdb_extension_load(sqlite_scanner
    GIT_URL https://github.com/duckdb/duckdb-sqlite
    GIT_TAG e223dc01493414cef6d5960744f1e048d59fc592
    SUBMODULES database-connector
    APPLY_PATCHES
    )

duckdb_extension_load(httpfs
    GIT_URL https://github.com/duckdb/duckdb-httpfs
    GIT_TAG 77392a72077a85897db2519fae11e2c0da972b8a
    )

# APPLY_PATCHES applies DuckDB's own patches for this commit, from
# .github/patches/extensions/ducklake/ in the DuckDB source, as its release did.
duckdb_extension_load(ducklake
    GIT_URL https://github.com/duckdb/ducklake
    GIT_TAG 417a9117f590e8141742ac0579d02f6a90162e37
    APPLY_PATCHES
    )

duckdb_extension_load(spatial
    GIT_URL https://github.com/duckdb/duckdb-spatial
    GIT_TAG 4e066c17e2d1f24625baff4c62430d68ceb4e41f
    INCLUDE_DIR src/spatial
    TEST_DIR test/sql
    )

# FineType is Meridian's own extension, written in Rust against DuckDB's C
# extension API. DuckDB 2.0 links such an extension statically through a describe
# function (src/include/duckdb_static_extension.h), which
# scripts/duckdb-managed/finetype/CMakeLists.txt generates around the static
# library scripts/build-duckdb.sh builds from FineType's pinned source.
duckdb_extension_load(finetype
    SOURCE_DIR ${CMAKE_CURRENT_LIST_DIR}/finetype
    INCLUDE_DIR ${CMAKE_CURRENT_LIST_DIR}/finetype
    EXTENSION_VERSION ${BRIGHTFIELD_FINETYPE_VERSION}
    )

duckdb_extension_statically_link(core_functions parquet json icu autocomplete
    excel sqlite_scanner httpfs ducklake spatial finetype)
