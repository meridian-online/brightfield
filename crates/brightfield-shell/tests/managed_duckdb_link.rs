//! Brightfield's managed edition loads the DuckDB library in its bundle, and no
//! other DuckDB.
//!
//! Built with `--features managed-duckdb`, the app binary links
//! `@rpath/libduckdb.dylib` and carries the rpath
//! `@executable_path/../Frameworks` (build.rs). This test lays the binary and
//! the library out as the Mac App Store bundle does, `Contents/MacOS/` beside
//! `Contents/Frameworks/`, starts it, and reads which DuckDB images the dynamic
//! loader loaded. Without the feature this file compiles to nothing.
#![cfg(all(feature = "managed-duckdb", target_os = "macos"))]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The managed DuckDB library, where `scripts/build-duckdb.sh` puts it.
fn managed_library() -> PathBuf {
    let dir = std::env::var_os("BRIGHTFIELD_DUCKDB_MANAGED_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/duckdb-managed")
        });
    dir.join("lib/libduckdb.dylib")
}

/// **The app binary links the bundled library by rpath, carries no DuckDB of
/// its own, and from a bundle's layout loads the library in
/// `Contents/Frameworks/` and no other.**
///
/// Linked against duckdb-rs's bundled DuckDB instead, the binary links no
/// DuckDB library and defines `duckdb_open` itself, and this fails at the first
/// assertion.
#[test]
fn the_app_loads_the_bundled_duckdb_and_no_other() {
    let exe = Path::new(env!("CARGO_BIN_EXE_brightfield-shell"));

    let linked = Command::new("otool")
        .arg("-L")
        .arg(exe)
        .output()
        .expect("otool runs");
    let linked = String::from_utf8_lossy(&linked.stdout);
    let duckdbs: Vec<&str> = linked
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| l.contains("duckdb"))
        .collect();
    assert!(
        duckdbs.len() == 1 && duckdbs[0].starts_with("@rpath/libduckdb.dylib "),
        "the app should link @rpath/libduckdb.dylib and no other DuckDB, and links {duckdbs:?}"
    );

    let defined = Command::new("nm")
        .args(["-gUj"])
        .arg(exe)
        .output()
        .expect("nm runs");
    assert!(
        !String::from_utf8_lossy(&defined.stdout)
            .lines()
            .any(|s| s == "_duckdb_open"),
        "the app defines duckdb_open itself, so it carries a DuckDB of its own"
    );

    let commands = Command::new("otool")
        .arg("-l")
        .arg(exe)
        .output()
        .expect("otool runs");
    let commands = String::from_utf8_lossy(&commands.stdout);
    let rpaths: Vec<&str> = commands
        .lines()
        .map(str::trim)
        .filter_map(|l| l.strip_prefix("path "))
        .map(|l| l.split(" (offset").next().unwrap_or(l))
        .collect();
    assert_eq!(
        rpaths,
        ["@executable_path/../Frameworks"],
        "the app's only rpath should be its bundle's Frameworks folder"
    );

    // The bundle's layout, and the binary started from it.
    let contents =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("managed-bundle/Brightfield.app/Contents");
    let _ = std::fs::remove_dir_all(&contents);
    for dir in ["MacOS", "Frameworks"] {
        std::fs::create_dir_all(contents.join(dir)).expect("make the bundle's folders");
    }
    let app = contents.join("MacOS/Brightfield");
    std::fs::copy(exe, &app).expect("copy the app binary into the bundle");
    // DuckDB's library and the libraries it links beside it (GEOS), as the
    // bundle carries them.
    let library = managed_library();
    let lib_dir = library.parent().expect("the library's directory");
    for entry in std::fs::read_dir(lib_dir).expect("read the built libraries") {
        let path = entry.expect("a built library").path();
        if path.extension().is_some_and(|e| e == "dylib") {
            let name = path.file_name().expect("a file name");
            std::fs::copy(&path, contents.join("Frameworks").join(name))
                .unwrap_or_else(|e| panic!("copy {} into the bundle: {e}", path.display()));
        }
    }

    let out = Command::new(&app)
        .arg("--version")
        .env("DYLD_PRINT_LIBRARIES", "1")
        // Cargo's library path would let the loader find the library outside
        // the bundle; the bundle's rpath alone has to.
        .env_remove("DYLD_FALLBACK_LIBRARY_PATH")
        .env_remove("DYLD_LIBRARY_PATH")
        .output()
        .expect("the app starts from the bundle");
    assert!(
        out.status.success(),
        "the app did not start from the bundle: {out:?}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let loaded: Vec<&str> = stderr
        .lines()
        .filter_map(|l| l.split("> ").nth(1))
        .filter(|path| {
            Path::new(path)
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains("duckdb"))
        })
        .collect();
    let bundled = contents
        .join("Frameworks/libduckdb.dylib")
        .canonicalize()
        .expect("the bundled library");
    assert_eq!(
        loaded,
        [bundled.to_str().expect("a UTF-8 path")],
        "from the bundle, the app should load its Frameworks' libduckdb.dylib and no other DuckDB"
    );
}
