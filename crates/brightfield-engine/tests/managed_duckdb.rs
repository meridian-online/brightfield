//! The managed edition's DuckDB, as the engine links it.
//!
//! Built with `--features managed-edition`, the engine runs on the library
//! `scripts/build-duckdb.sh` builds: DuckDB v2.0.0 with autocomplete, excel,
//! sqlite, httpfs, ducklake, spatial and FineType compiled in, linked by its
//! install name `@rpath/libduckdb.dylib`. These tests hold the two properties the
//! Mac App Store build rests on: the extensions are there in process with no
//! extension file and no network, and the binary carries no DuckDB of its own
//! beside the library.
//!
//! Without the feature this file compiles to nothing, so the full edition's test
//! run is unchanged.
#![cfg(all(feature = "managed-edition", target_os = "macos"))]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The seven extensions the managed edition compiles in, by the names
/// `duckdb_extensions()` reports. `sqlite_scanner` is the sqlite extension.
const BUILT_IN: [&str; 7] = [
    "autocomplete",
    "excel",
    "sqlite_scanner",
    "httpfs",
    "ducklake",
    "spatial",
    "finetype",
];

/// The jail the child below runs in: everything allowed but the network, as in
/// brightfield-shell's `tests/front_door.rs`.
const NETWORK_DENIED: &str = "(version 1)(allow default)(deny network*)";

/// Set in the jailed child's environment: the port of a loopback listener the
/// parent holds open, and the directory the child points DuckDB's extension
/// directory at.
const JAILED_PORT_VAR: &str = "BRIGHTFIELD_TEST_MANAGED_DUCKDB_PORT";
const JAILED_EXTENSION_DIR_VAR: &str = "BRIGHTFIELD_TEST_MANAGED_DUCKDB_EXTENSION_DIR";

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

/// **The seven extensions are loaded in process, and a workbook and a SQLite
/// file read with no extension file and the network denied.**
///
/// The test re-runs itself under `sandbox-exec` with the network denied and
/// `HOME` at an empty directory, since DuckDB's default extension directory is
/// `$HOME/.duckdb`. The child first proves the jail denies, by failing to reach
/// a loopback listener the parent holds open, then opens an in-memory database
/// with its extension directory at another empty directory and automatic
/// install and load off, so nothing can arrive from outside the library. The
/// parent then requires the child to have run exactly this test and passed, and
/// both directories to still be empty.
///
/// Linked against duckdb-rs's bundled DuckDB instead, the child fails at the
/// extension list: the bundled build links none of the seven.
#[test]
fn managed_duckdb_has_the_extensions_in_process() {
    const NAME: &str = "managed_duckdb_has_the_extensions_in_process";

    if let Some(port) = std::env::var_os(JAILED_PORT_VAR) {
        // ── the jailed child ────────────────────────────────────────────────
        let port: u16 = port
            .to_string_lossy()
            .parse()
            .expect("the parent hands the child a port");
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
            "the jail let a loopback connection through, so this run had a network and proves \
             nothing about running without one"
        );
        let extension_dir = std::env::var(JAILED_EXTENSION_DIR_VAR).expect("the parent names it");
        let conn = duckdb::Connection::open_in_memory().expect("open an in-memory database");
        conn.execute_batch(&format!(
            "SET extension_directories = ['{}']; \
             SET autoinstall_known_extensions = false; \
             SET autoload_known_extensions = false;",
            extension_dir.replace('\'', "''")
        ))
        .expect("point DuckDB at an empty extension directory");

        let mut statement = conn
            .prepare(
                "SELECT extension_name FROM duckdb_extensions() \
                 WHERE loaded AND install_mode = 'STATICALLY_LINKED' ORDER BY extension_name",
            )
            .expect("list the extensions");
        let linked: Vec<String> = statement
            .query_map([], |row| row.get(0))
            .expect("read the extension list")
            .map(|name| name.expect("an extension name"))
            .collect();
        let missing: Vec<&str> = BUILT_IN
            .iter()
            .copied()
            .filter(|name| !linked.iter().any(|l| l == name))
            .collect();
        assert!(
            missing.is_empty(),
            "the managed edition's DuckDB does not have {missing:?} loaded and statically linked; \
             it reports {linked:?}"
        );

        let book = data("book.xlsx");
        let rows: i64 = conn
            .query_row(
                &format!(
                    "SELECT count(*) FROM read_xlsx('{}', sheet = 'Orders', range = 'A2:C5')",
                    book.display()
                ),
                [],
                |row| row.get(0),
            )
            .expect("read the workbook with the built-in excel extension");
        assert_eq!(rows, 3, "book.xlsx's Orders sheet holds three data rows");

        let shop = data("shop.sqlite");
        let total: f64 = conn
            .query_row(
                &format!(
                    "SELECT sum(total) FROM sqlite_scan('{}', 'orders')",
                    shop.display()
                ),
                [],
                |row| row.get(0),
            )
            .expect("read the SQLite file with the built-in sqlite extension");
        assert!(
            (total - 29.5).abs() < 1e-9,
            "shop.sqlite's orders total 29.5, read {total}"
        );
        return;
    }

    // ── the parent ──────────────────────────────────────────────────────────
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("managed-duckdb");
    let _ = std::fs::remove_dir_all(&root);
    let (home, extensions) = (root.join("home"), root.join("extensions"));
    for dir in [&home, &extensions] {
        std::fs::create_dir_all(dir).expect("make the jailed child's directories");
    }
    let control = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    let port = control.local_addr().expect("its address").port();

    let out = Command::new("/usr/bin/sandbox-exec")
        .args(["-p", NETWORK_DENIED])
        .arg(std::env::current_exe().expect("this test binary"))
        .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env(JAILED_PORT_VAR, port.to_string())
        .env(JAILED_EXTENSION_DIR_VAR, &extensions)
        .env("HOME", &home)
        .output()
        .expect("sandbox-exec runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the jailed run failed ({}):\n{stdout}\n{stderr}",
        out.status
    );
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "the jailed run did not run exactly this test, so its exit status says nothing:\n{stdout}"
    );
    drop(control);

    let written: Vec<PathBuf> = [&home, &extensions]
        .iter()
        .flat_map(|d| files_under(d))
        .collect();
    assert!(
        written.is_empty(),
        "the jailed run wrote extension files: {written:?}"
    );
}

/// **The binary links the managed library by rpath and carries no DuckDB of its
/// own.**
///
/// `otool -L` must list `@rpath/libduckdb.dylib` and no other DuckDB library,
/// and the binary must define none of DuckDB's C API and bind `duckdb_open_ext`,
/// which duckdb-rs opens a database with, from the library at launch. Linked against duckdb-rs's bundled
/// DuckDB instead, the C API is defined in the binary itself and this fails
/// there.
#[test]
fn the_engine_links_the_managed_library_and_no_other_duckdb() {
    let exe = std::env::current_exe().expect("this test binary");

    let linked = Command::new("otool")
        .arg("-L")
        .arg(&exe)
        .output()
        .expect("otool runs");
    let linked = String::from_utf8_lossy(&linked.stdout);
    let duckdbs: Vec<&str> = linked
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| line.contains("duckdb"))
        .collect();
    assert_eq!(
        duckdbs.len(),
        1,
        "the binary should link exactly one DuckDB library, and links {duckdbs:?}"
    );
    assert!(
        duckdbs[0].starts_with("@rpath/libduckdb.dylib "),
        "the binary links {} rather than @rpath/libduckdb.dylib",
        duckdbs[0]
    );

    // DuckDB's C API, the functions duckdb-rs calls: `duckdb_` and a lower-case
    // letter, which leaves out the `duckdb_` C++ namespaces of its vendored
    // libraries.
    let c_api = |flag: &str| -> Vec<String> {
        let out = Command::new("nm")
            .args([flag, "-j"])
            .arg(&exe)
            .output()
            .expect("nm runs");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|symbol| {
                symbol
                    .strip_prefix("_duckdb_")
                    .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_lowercase()))
            })
            .map(str::to_owned)
            .collect()
    };
    let defined = c_api("-gU");
    assert!(
        defined.is_empty(),
        "the binary defines DuckDB's C API itself ({} functions, {:?} among them), so it \
         carries a DuckDB of its own beside the library",
        defined.len(),
        defined.first()
    );
    assert!(
        c_api("-u").iter().any(|s| s == "_duckdb_open_ext"),
        "the binary does not bind duckdb_open_ext, which duckdb-rs opens a database with, \
         from a library"
    );
}
