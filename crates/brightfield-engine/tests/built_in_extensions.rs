//! Extensions built in, with install and load off: how the engine reads when
//! its DuckDB has the extensions compiled in, as the managed edition's does.
//!
//! The refusals and the settings are the engine's, not the library's, so they
//! are tested in every build by asking for [`Extensions::BuiltIn`] on whatever
//! DuckDB this build links. Reading a workbook with nothing installed needs
//! the excel extension compiled in, so that test runs only with the
//! `managed-duckdb` feature, in the `managed-duckdb-test` step.

use std::path::{Path, PathBuf};

use brightfield_engine::error::EngineError;
use brightfield_engine::semantic::{self, TypeSourceSpec};
use brightfield_engine::sources::{discover, load_sql_for, Reader};
use brightfield_engine::{Engine, Extensions, LoadOptions};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

/// A spec whose one source is the query `sql`.
fn spec_with_query(sql: &str) -> String {
    format!(
        "data:\n  t:\n    query: {}\nplot:\n  - mark: dot\n    data: {{ from: t }}\n",
        serde_json::to_string(sql).expect("a JSON string is a YAML string")
    )
}

// The engine's own error, as `load_spec_with` returns it, so a test can match its variant.
#[allow(clippy::result_large_err)]
fn load(spec: &str, options: &LoadOptions) -> Result<brightfield_engine::Session, EngineError> {
    let parsed = parse_spec(spec, Format::Yaml).expect("the spec parses");
    let analysis = analyse_spec(&parsed.spec).expect("the spec analyses");
    Engine::new()
        .load_spec_with(parsed.spec, analysis, None, options)
        .map(|loaded| loaded.session)
}

fn built_in() -> LoadOptions {
    LoadOptions {
        extensions: Extensions::BuiltIn,
        ..LoadOptions::default()
    }
}

/// **A session whose extensions are built in refuses a source that installs
/// an extension, updates them, or loads one that is not built in, with the
/// edition's sentence; and a source that reads as a second statement, such as
/// a `SET` that turns automatic install back on, after a query or in a name.**
#[test]
fn managed_refuses_install_and_load_of_anything_else() {
    for (sql, sentence) in [
        (
            "SELECT 1 AS x; INSTALL h3",
            "This edition can't install the DuckDB extension h3: it runs only the DuckDB \
             extensions built into it.",
        ),
        (
            "SELECT 1 AS x; INSTALL h3 FROM community",
            "This edition can't install the DuckDB extension h3",
        ),
        (
            "SELECT 1 AS x; LOAD h3",
            "This edition can't load the DuckDB extension h3",
        ),
        (
            "SELECT 1 AS x; LOAD '/x.duckdb_extension'",
            "This edition can't load the DuckDB extension /x.duckdb_extension",
        ),
        (
            "SELECT 1 AS x; FORCE INSTALL excel",
            "This edition can't install the DuckDB extension excel",
        ),
        (
            "SELECT 1 AS x; UPDATE EXTENSIONS",
            "This edition can't update DuckDB extensions",
        ),
    ] {
        match load(&spec_with_query(sql), &built_in()) {
            Err(EngineError::ExtensionRefused {
                source_name,
                refusal,
            }) => {
                assert_eq!(source_name, "t");
                assert!(
                    refusal.starts_with(sentence),
                    "{sql:?} was refused with {refusal:?}, not {sentence:?}"
                );
            }
            Err(other) => panic!("{sql:?} failed, but not as refused: {other}"),
            Ok(_) => panic!("{sql:?} loaded on a session whose extensions are built in"),
        }
    }

    // A second statement is refused whatever it is, and however it is quoted,
    // since it could change the settings that keep the session built in.
    for sql in [
        "SELECT 1 AS x; SET autoinstall_known_extensions = true",
        "SELECT $$'$$ AS x; RESET custom_extension_repository; SELECT $$'$$",
    ] {
        match load(&spec_with_query(sql), &built_in()) {
            Err(EngineError::ExtensionRefused { refusal, .. }) => {
                assert_eq!(refusal, brightfield_engine::extensions::ONE_STATEMENT);
            }
            Err(other) => panic!("{sql:?} failed, but not as refused: {other}"),
            Ok(_) => panic!("{sql:?} loaded on a session whose extensions are built in"),
        }
    }
    // A source name holding a quote reaches past the view it names.
    let named = "data:\n  'a\" AS SELECT 1; SET autoinstall_known_extensions = true; CREATE VIEW \"b':\n    - { x: 1 }\nplot:\n  - mark: dot\n    data: { from: a }\n";
    match load(named, &built_in()) {
        Err(EngineError::ExtensionRefused { refusal, .. }) => {
            assert_eq!(refusal, brightfield_engine::extensions::ONE_STATEMENT);
        }
        Err(other) => panic!("the quoted name failed, but not as refused: {other}"),
        Ok(_) => panic!("a name carrying a SET loaded on a built-in session"),
    }

    // One query, even one whose text mentions the words, loads.
    load(
        &spec_with_query("SELECT 'INSTALL h3; SET x = 1' AS x"),
        &built_in(),
    )
    .expect("a single query loads");

    // A load of a built-in extension is not refused: it is already loaded.
    // (Whether it then loads depends on the DuckDB this build links, so the
    // statement is checked, not run.)
    assert!(brightfield_engine::extensions::refused_when_built_in("LOAD spatial").is_none());
}

/// **Nothing installs or loads by itself on a session whose extensions are
/// built in, whatever its network policy.**
#[test]
fn a_built_in_session_has_automatic_install_and_load_off() {
    let session = load(&spec_with_query("SELECT 1 AS x"), &built_in()).expect("it loads");
    let get = |k: &str| session.duckdb_setting(k).expect("the setting reads");
    assert_eq!(get("autoinstall_known_extensions"), "false");
    assert_eq!(get("autoload_known_extensions"), "false");
    assert!(
        get("custom_extension_repository").contains("/dev/null/"),
        "the extension repository still resolves"
    );
}

/// **The full edition installs as before.** Its load steps install and load
/// the extension the reader needs, a source that installs one is not refused
/// at the engine, and automatic install is on.
#[test]
fn the_full_edition_installs_as_before() {
    let book = discover(&data("book.xlsx")).expect("the workbook is found");
    let table = book
        .tables
        .iter()
        .find(|t| t.reader == Reader::XlsxSheet)
        .expect("a sheet");
    let sql = load_sql_for(table, &data(""), Extensions::Installable);
    assert!(
        sql.starts_with("INSTALL excel; LOAD excel; "),
        "the full edition's load step no longer installs excel: {sql}"
    );

    let options = LoadOptions {
        extensions: Extensions::Installable,
        ..LoadOptions::default()
    };
    let session = load(&spec_with_query("SELECT 1 AS x"), &options).expect("it loads");
    assert_eq!(
        session
            .duckdb_setting("autoinstall_known_extensions")
            .expect("the setting reads"),
        "true"
    );
    // Refused at the engine only when built in. (Here DuckDB itself may then
    // fail it for want of a network; what matters is that the refusal is not
    // the engine's.)
    if let Err(EngineError::ExtensionRefused { refusal, .. }) =
        load(&spec_with_query("SELECT 1 AS x; LOAD json"), &options)
    {
        panic!("the full edition refused a load: {refusal}");
    }
}

/// **A built-in session's load steps name no extension**, so the Protocol
/// they are written into runs on the managed edition's DuckDB with no install
/// and on DuckDB's release command line, which installs a reader it lacks on
/// first use.
#[test]
fn a_built_in_load_step_names_no_extension() {
    for file in ["book.xlsx", "shop.sqlite"] {
        let set = discover(&data(file)).expect("the file is found");
        for table in &set.tables {
            let sql = load_sql_for(table, &data(""), Extensions::BuiltIn);
            assert!(
                brightfield_engine::extensions::statements(&sql).is_empty(),
                "{file}'s load step names an extension: {sql}"
            );
        }
    }
}

/// The one folder both FineType tests build their bundles in, and the lock
/// they take while they use it. A FineType open seals the process's model
/// folder the first time it gets that far, which on the managed library it
/// does, and refuses a different folder after that; so the tests share one
/// folder, one at a time.
fn bundle_dir() -> (std::sync::MutexGuard<'static, ()>, PathBuf) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = std::env::temp_dir().join(format!("bf-built-in-bundle-{}", std::process::id()));
    (guard, dir)
}

/// **A FineType bundle opened on a built-in session loads no extension file.**
///
/// The bundle carries a file where the extension would be, which is not a
/// library. Opened as the full edition opens it, the open gets as far as
/// loading that file, and fails there. Opened built in, it is never read: the
/// open fails at asking DuckDB for FineType's version, since this DuckDB, if it
/// is the bundled one, has no FineType compiled in.
#[test]
fn a_built_in_finetype_bundle_loads_no_file() {
    let (_lock, dir) = bundle_dir();
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(dir.join(semantic::MODEL_DIR)).unwrap();
    std::fs::write(dir.join(semantic::EXTENSION_FILE), b"not a library").unwrap();
    std::fs::write(
        dir.join(semantic::MODEL_DIR).join("model.safetensors"),
        b"fake",
    )
    .unwrap();
    std::fs::write(
        dir.join(semantic::MODEL_DIR).join(semantic::LABEL_MAP_FILE),
        r#"["identity.person.email"]"#,
    )
    .unwrap();
    std::fs::write(
        dir.join(semantic::SCHEMA_CATALOGUE),
        r#"[{"x-finetype-label": "identity.person.email", "type": "string", "pattern": "^.+@.+$"}]"#,
    )
    .unwrap();

    let spec = TypeSourceSpec::Bundle(dir.clone());
    assert!(!spec.needs_unsigned_extensions_for(Extensions::BuiltIn));
    assert!(spec.needs_unsigned_extensions_for(Extensions::Installable));

    let conn = duckdb::Connection::open_in_memory().unwrap();
    let has_finetype = conn
        .query_row("SELECT ft_version()", [], |r| r.get::<_, String>(0))
        .is_ok();
    match spec.open_for(&conn, Extensions::BuiltIn) {
        Err(e) if !has_finetype => assert!(
            e.contains("ft_version()") && e.contains("no FineType compiled in"),
            "the built-in open failed somewhere other than asking for FineType: {e}"
        ),
        // With FineType compiled in, the fake model fails the canary instead.
        Err(e) => assert!(e.contains("canary"), "{e}"),
        Ok(_) => panic!("a bundle with a fake model passed its canary"),
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A bundle in `dir` with a fake extension file and model, the label map
/// `labels`, and a catalogue describing one label.
fn fake_bundle(dir: &Path, labels: &str) {
    std::fs::remove_dir_all(dir).ok();
    std::fs::create_dir_all(dir.join(semantic::MODEL_DIR)).unwrap();
    std::fs::write(dir.join(semantic::EXTENSION_FILE), b"not a library").unwrap();
    std::fs::write(
        dir.join(semantic::MODEL_DIR).join("model.safetensors"),
        b"fake",
    )
    .unwrap();
    std::fs::write(
        dir.join(semantic::MODEL_DIR).join(semantic::LABEL_MAP_FILE),
        labels,
    )
    .unwrap();
    std::fs::write(
        dir.join(semantic::SCHEMA_CATALOGUE),
        r#"[{"x-finetype-label": "identity.person.email", "type": "string", "pattern": "^.+@.+$"}]"#,
    )
    .unwrap();
}

fn open_built_in(dir: &Path) -> String {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    match TypeSourceSpec::Bundle(dir.to_path_buf()).open_for(&conn, Extensions::BuiltIn) {
        Ok(_) => panic!("a bundle with a fake model opened"),
        Err(e) => e,
    }
}

/// **A built-in open refuses a bundle by its files before it asks DuckDB for
/// FineType**: a catalogue that does not describe the model, a file the
/// manifest contradicts, a manifest naming only the extension file, and a
/// missing model. Each is refused naming its cause and not `ft_version()`; the
/// same bundle put right gets past that check. A manifest entry for the
/// extension file, which a built-in open neither needs nor reads, is passed
/// over, so a manifest that names nothing else verifies nothing. A session whose bundle is refused still loads,
/// with the refusal remembered.
#[test]
fn a_built_in_open_refuses_a_bad_bundle_before_asking_for_finetype() {
    let (_lock, dir) = bundle_dir();
    let agreed = r#"["identity.person.email"]"#;

    fake_bundle(
        &dir,
        r#"["a.b.c", "d.e.f", "g.h.i", "identity.person.email"]"#,
    );
    let e = open_built_in(&dir);
    assert!(
        e.contains("different FineType versions") && !e.contains("ft_version()"),
        "{e}"
    );
    fake_bundle(&dir, agreed);
    let e = open_built_in(&dir);
    assert!(!e.contains("different FineType versions"), "{e}");

    std::fs::write(
        dir.join(semantic::MANIFEST_FILE),
        format!("{}  {}\n", "0".repeat(64), semantic::SCHEMA_CATALOGUE),
    )
    .unwrap();
    let e = open_built_in(&dir);
    assert!(
        e.contains("not the file that was packaged")
            && e.contains(semantic::SCHEMA_CATALOGUE)
            && !e.contains("ft_version()"),
        "{e}"
    );
    std::fs::write(
        dir.join(semantic::MANIFEST_FILE),
        format!("{}  {}\n", "0".repeat(64), semantic::EXTENSION_FILE),
    )
    .unwrap();
    let e = open_built_in(&dir);
    assert!(
        e.contains("names no files, so it verified nothing") && !e.contains("ft_version()"),
        "a manifest naming only the extension verifies nothing in a built-in open: {e}"
    );
    let catalogue_hash: String = duckdb::Connection::open_in_memory()
        .unwrap()
        .query_row(
            "SELECT lower(sha256(content)) FROM read_blob(?)",
            duckdb::params![dir
                .join(semantic::SCHEMA_CATALOGUE)
                .to_string_lossy()
                .as_ref()],
            |r| r.get(0),
        )
        .unwrap();
    std::fs::write(
        dir.join(semantic::MANIFEST_FILE),
        format!(
            "{}  {}\n{catalogue_hash}  {}\n",
            "0".repeat(64),
            semantic::EXTENSION_FILE,
            semantic::SCHEMA_CATALOGUE
        ),
    )
    .unwrap();
    let e = open_built_in(&dir);
    assert!(
        !e.contains("not the file that was packaged") && !e.contains("verified nothing"),
        "{e}"
    );

    std::fs::remove_file(dir.join(semantic::MODEL_DIR).join("model.safetensors")).unwrap();
    let e = open_built_in(&dir);
    assert!(
        e.contains("no model.safetensors") && !e.contains("ft_version()"),
        "{e}"
    );

    let options = LoadOptions {
        type_source: Some(TypeSourceSpec::Bundle(dir.clone())),
        ..built_in()
    };
    let session = load(&spec_with_query("SELECT 'a@b.c' AS email"), &options)
        .expect("a refused bundle does not stop the load");
    assert!(
        session
            .type_source_error()
            .is_some_and(|e| e.contains("no model.safetensors")),
        "{:?}",
        session.type_source_error()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// **The managed edition reads a workbook and a SQLite file with no install
/// and no network.**
///
/// Discovery and the load steps it writes run under `sandbox-exec` with the
/// network denied and `HOME` and the extension directory at empty folders, on
/// a connection with automatic install and load off, so a reader has to come
/// from the library. The parent then requires the child to have run
/// exactly this test and passed, and both folders to still be empty.
#[cfg(all(feature = "managed-duckdb", target_os = "macos"))]
#[test]
fn managed_reads_a_workbook_with_no_install_and_no_network() {
    const NAME: &str = "managed_reads_a_workbook_with_no_install_and_no_network";
    const JAILED: &str = "BRIGHTFIELD_TEST_BUILT_IN_EXTENSIONS_JAILED";
    const NETWORK_DENIED: &str = "(version 1)(allow default)(deny network*)";

    if let Some(dirs) = std::env::var_os(JAILED) {
        let extension_dir = PathBuf::from(dirs);
        assert_eq!(Extensions::linked(), Extensions::BuiltIn);
        for (file, reader) in [
            ("book.xlsx", Reader::XlsxSheet),
            ("shop.sqlite", Reader::SqliteTable),
        ] {
            let set = discover(&data(file)).expect("discovery reads it with no install");
            let table = set
                .tables
                .iter()
                .find(|t| t.reader == reader)
                .expect("a table");
            let conn = duckdb::Connection::open_in_memory().unwrap();
            conn.execute_batch(&format!(
                "SET extension_directories = ['{}']; \
                 SET autoinstall_known_extensions = false; \
                 SET autoload_known_extensions = false;",
                extension_dir.display()
            ))
            .unwrap();
            conn.execute_batch(&load_sql_for(table, &data(""), Extensions::BuiltIn))
                .unwrap_or_else(|e| panic!("{file}'s load step failed: {e}"));
            let n: i64 = conn
                .query_row(
                    &format!("SELECT count(*) FROM \"raw_{}\"", table.name),
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(n > 0, "{file} loaded no rows");
        }
        return;
    }

    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("built-in-extensions");
    let _ = std::fs::remove_dir_all(&root);
    let (home, extensions) = (root.join("home"), root.join("extensions"));
    for dir in [&home, &extensions] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let out = std::process::Command::new("/usr/bin/sandbox-exec")
        .args(["-p", NETWORK_DENIED])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env(JAILED, &extensions)
        .env("HOME", &home)
        .output()
        .expect("sandbox-exec runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the jailed run failed:\n{stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("test result: ok. 1 passed"),
        "the jailed run did not run exactly this test:\n{stdout}"
    );
    let left: Vec<_> = [&home, &extensions].iter().flat_map(|d| walk(d)).collect();
    assert!(left.is_empty(), "the jailed run wrote {left:?}");
}

#[cfg(all(feature = "managed-duckdb", target_os = "macos"))]
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else {
            found.push(path);
        }
    }
    found
}
