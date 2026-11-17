//! Brightfield's addition to libduckdb-sys: the `managed-linked` feature.
//!
//! Brightfield's managed edition, the Mac App Store build, runs on one DuckDB
//! library, `libduckdb.dylib`, with its extensions compiled in, which
//! Brightfield links and DuckDB's own shell loads. `scripts/build-duckdb.sh` in
//! Brightfield builds it into `target/duckdb-managed/`.
//!
//! Every crate in Brightfield's graph that uses DuckDB asks for `bundled`
//! (Brightfield's own, and arcform's and dovetail's), and Cargo features only
//! add, so no feature can take `bundled` away again. `managed-linked` overrides
//! it instead: with it on, this build script links the managed library through
//! the crate's own linked-library path (`build_linked.rs`, as `DUCKDB_LIB_DIR`
//! would), and the bundled DuckDB is not compiled at all.
//!
//! The library is looked up in `$BRIGHTFIELD_DUCKDB_MANAGED_DIR`, or else in
//! `target/duckdb-managed/` of the workspace this crate is vendored into.

use std::{env, path::PathBuf};

pub fn main(out_dir: &str) {
    println!("cargo:rerun-if-env-changed=BRIGHTFIELD_DUCKDB_MANAGED_DIR");
    let dir = match env::var_os("BRIGHTFIELD_DUCKDB_MANAGED_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR"))
            .join("../../target/duckdb-managed"),
    };
    let lib_dir = dir.join("lib");
    let include_dir = dir.join("include");
    let dylib = lib_dir.join("libduckdb.dylib");
    if !dylib.is_file() || !include_dir.join("duckdb.h").is_file() {
        panic!(
            "the managed edition links the DuckDB library scripts/build-duckdb.sh builds, \
             and {} has none; run scripts/build-duckdb.sh first",
            dir.display()
        );
    }
    println!("cargo:rerun-if-changed={}", dylib.display());
    // build_linked reads these as a user's own DUCKDB_LIB_DIR would be read. A
    // build script runs single-threaded, so setting them here races nothing.
    unsafe {
        env::set_var("DUCKDB_LIB_DIR", &lib_dir);
        env::set_var("DUCKDB_INCLUDE_DIR", &include_dir);
        env::remove_var("DUCKDB_STATIC");
    }
    crate::build_linked::main(out_dir);
    // A crate that depends on this one directly reads this as DEP_DUCKDB_LIB_DIR,
    // to give its own test binaries an rpath to the library.
    println!("cargo:lib_dir={}", lib_dir.display());
}
