//! In the managed edition the engine links the DuckDB library
//! `scripts/build-duckdb.sh` builds, `libduckdb.dylib`, by its install name
//! `@rpath/libduckdb.dylib`. A test binary finds it through Cargo's library
//! search path only while Cargo's environment reaches it, and macOS strips that
//! environment from a program `sandbox-exec` starts, as the network-denied test
//! in `tests/managed_duckdb.rs` does. So the engine's tests carry an rpath to the
//! library's directory, which the vendored libduckdb-sys reports as
//! `DEP_DUCKDB_LIB_DIR`. The full edition links DuckDB statically, and this adds
//! nothing to it.

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_DUCKDB_LIB_DIR");
    if std::env::var_os("CARGO_FEATURE_MANAGED_DUCKDB").is_none() {
        return;
    }
    let lib_dir = std::env::var("DEP_DUCKDB_LIB_DIR")
        .expect("the managed edition's libduckdb-sys reports the library's directory");
    println!("cargo:rustc-link-arg-tests=-Wl,-rpath,{lib_dir}");
}
