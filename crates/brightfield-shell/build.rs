//! In the managed edition Brightfield links the DuckDB library
//! `scripts/build-duckdb.sh` builds, by its install name
//! `@rpath/libduckdb.dylib`, and the Mac App Store bundle carries it in
//! `Contents/Frameworks/`. So the managed edition's binaries carry the rpath
//! `@executable_path/../Frameworks`, which from `Contents/MacOS/` is that
//! folder, and its test binaries an rpath to the built library's directory
//! (`DEP_DUCKDB_LIB_DIR`, from the vendored libduckdb-sys), for a test that
//! runs one under `sandbox-exec`, which strips Cargo's library path. A build
//! path never goes into a binary that ships. The full edition links DuckDB
//! statically, and this adds nothing to it.

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_DUCKDB_LIB_DIR");
    if std::env::var_os("CARGO_FEATURE_MANAGED_DUCKDB").is_none() {
        return;
    }
    println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@executable_path/../Frameworks");
    let lib_dir = std::env::var("DEP_DUCKDB_LIB_DIR")
        .expect("the managed edition's libduckdb-sys reports the library's directory");
    println!("cargo:rustc-link-arg-tests=-Wl,-rpath,{lib_dir}");
}
