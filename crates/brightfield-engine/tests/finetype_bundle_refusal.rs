//! What a broken bundle does — the half a happy-path test cannot see.
//!
//! Its own test binary, and that is not cosmetic. The FineType extension picks
//! its model up from `FINETYPE_MODEL_DIR`, a process-global that
//! `semantic::FinetypeBundle::open` writes once and never overwrites, and the
//! extension resolves it into a `OnceLock` on first use. Two bundles with
//! different models therefore cannot be exercised in one process: the second
//! would silently get the first's model. Cargo gives each `tests/*.rs` file
//! its own binary, so this file gets its own process and its own first write.
//!
//! Bundle-dependent tests are `#[ignore]`d for the reason given in
//! `finetype_bundle.rs`.

use std::path::{Path, PathBuf};

use brightfield_engine::semantic;
use brightfield_engine::{Engine, LoadOptions, NetworkPolicy, SemanticType};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};

const FIXTURE: &str = r#"
data:
  people:
    - { email: "alice@example.com" }
    - { email: "bob@example.org" }
    - { email: "carol@example.net" }
plot:
  - mark: dot
    data: { from: people }
"#;

fn bundle() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os(
        "BRIGHTFIELD_FINETYPE_BUNDLE",
    )?))
}

/// Load `FIXTURE` pointing the type source at `dir`, and return what the
/// session says about the attempt.
fn load_against(dir: &Path) -> (Option<String>, Option<String>, Vec<SemanticType>) {
    let parsed = parse_spec(FIXTURE, Format::Yaml).expect("the fixture parses");
    let analysis = analyse_spec(&parsed.spec).expect("the fixture analyses");
    let options = LoadOptions {
        network: NetworkPolicy::Disabled,
        extension_directory: None,
        type_source: Some(semantic::TypeSourceSpec::Bundle(dir.to_path_buf())),
    };
    let load = Engine::new()
        .load_spec_with(parsed.spec, analysis, None, &options)
        .expect("a broken type source must not fail the load");
    let name = load.session.type_source_name().map(str::to_string);
    let err = load.session.type_source_error().map(str::to_string);
    let semantics = match load
        .session
        .profile_sources()
        .into_iter()
        .find(|p| p.name == "people")
        .expect("one source")
        .outcome
    {
        brightfield_engine::ProfileOutcome::Profiled { columns, .. } => {
            columns.into_iter().map(|c| c.semantic).collect()
        }
        other => panic!("expected a profiled source, got {other:?}"),
    };
    (name, err, semantics)
}

/// Write a bundle with a well-formed DuckDB metadata trailer and fake contents
/// everywhere else.
///
/// The extension is not an extension — but it is stamped for the DuckDB this
/// build links, so `read_stamp` and `check_abi` pass and the file-level checks
/// past them are reachable. Nothing here can survive as far as `LOAD`, which is
/// the point: it exercises the stretch of `open` that decides on bytes alone.
fn synthetic_bundle(dir: &Path, label_map: &str, catalogue: &str) {
    let (platform, version) = {
        let conn = duckdb::Connection::open_in_memory().expect("in-memory duckdb");
        let p: String = conn
            .query_row("SELECT * FROM pragma_platform()", [], |r| r.get(0))
            .unwrap();
        let v: String = conn
            .query_row("SELECT version()", [], |r| r.get(0))
            .unwrap();
        (p, v)
    };
    // A floor at or below the running engine, whatever that is.
    let floor = version.split('.').next().unwrap_or("v1").to_string() + ".0.0";

    let pad = |s: &str| {
        let mut f = [0u8; 32];
        f[..s.len()].copy_from_slice(s.as_bytes());
        f
    };
    let mut file = b"not really a shared library".to_vec();
    file.extend_from_slice(&[0u8; 96]);
    file.extend_from_slice(&pad("C_STRUCT"));
    file.extend_from_slice(&pad("0.0.0"));
    file.extend_from_slice(&pad(&floor));
    file.extend_from_slice(&pad(&platform));
    file.extend_from_slice(&pad("4"));
    file.extend_from_slice(&[0u8; 256]);

    std::fs::create_dir_all(dir.join(semantic::MODEL_DIR)).unwrap();
    std::fs::write(dir.join(semantic::EXTENSION_FILE), file).unwrap();
    std::fs::write(
        dir.join(semantic::MODEL_DIR).join("model.safetensors"),
        b"fake",
    )
    .unwrap();
    std::fs::write(
        dir.join(semantic::MODEL_DIR).join(semantic::LABEL_MAP_FILE),
        label_map,
    )
    .unwrap();
    std::fs::write(dir.join(semantic::SCHEMA_CATALOGUE), catalogue).unwrap();
}

const ONE_LABEL_CATALOGUE: &str =
    r#"[{"x-finetype-label": "identity.person.email", "type": "string", "pattern": "^.+@.+$"}]"#;

/// A model and a schema catalogue from different FineType versions is refused
/// by `open` itself — not merely by the function that decides it.
///
/// The distinction is the whole reason this test exists. A unit test on
/// `CatalogueCoverage::accept` stays green when the CALL is deleted from
/// `open`, and the call is on a path no CI run reaches without a real bundle.
/// A synthesised bundle reaches it, because every file-level check now runs
/// before the extension is loaded.
#[test]
fn a_bundle_whose_catalogue_does_not_describe_its_model_is_refused_by_open() {
    let dir = std::env::temp_dir().join(format!("bf-skew-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let skewed = r#"["a.b.c", "d.e.f", "g.h.i", "identity.person.email"]"#;
    synthetic_bundle(&dir, skewed, ONE_LABEL_CATALOGUE);

    let (name, err, semantics) = load_against(&dir);
    assert_eq!(name, None);
    let err = err.expect("a skewed bundle has to be reported");
    assert!(
        err.contains("different FineType versions"),
        "the refusal does not name the cause: {err}"
    );
    assert_eq!(semantics, vec![SemanticType::NotAsked]);

    // And a bundle whose catalogue DOES describe its model gets past this check
    // — it then dies at the LOAD, because the extension here is not one. Two
    // different refusals from two bundles differing only in the label map is
    // what says the coverage check is the thing being exercised.
    let agreed = r#"["identity.person.email"]"#;
    std::fs::write(
        dir.join(semantic::MODEL_DIR).join(semantic::LABEL_MAP_FILE),
        agreed,
    )
    .unwrap();
    let (_, err, _) = load_against(&dir);
    let err = err.expect("a fake extension still cannot load");
    assert!(
        !err.contains("different FineType versions"),
        "the coverage check fired on a catalogue that does describe the model: {err}"
    );
    assert!(
        err.contains("LOAD"),
        "expected the load to be what failed: {err}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A bundle whose bytes changed after packaging is refused by `open` itself,
/// before the extension is loaded.
///
/// Pins the CALL, for the same reason as the coverage test above: a unit test
/// on `verify_against_manifest` stays green when the call is deleted from
/// `open`, and "verified before loading" is the only version of this check
/// worth having — after the LOAD the code is already running.
#[test]
fn a_bundle_that_does_not_match_its_manifest_is_refused_before_the_extension_loads() {
    let dir = std::env::temp_dir().join(format!("bf-tamper-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    synthetic_bundle(&dir, r#"["identity.person.email"]"#, ONE_LABEL_CATALOGUE);
    // A hash that is well-formed and simply is not this file's.
    std::fs::write(
        dir.join(semantic::MANIFEST_FILE),
        format!("{}  {}\n", "0".repeat(64), semantic::SCHEMA_CATALOGUE),
    )
    .unwrap();

    let (name, err, semantics) = load_against(&dir);
    assert_eq!(name, None);
    let err = err.expect("a bundle contradicting its manifest has to be reported");
    assert!(
        err.contains("not the file that was packaged") && err.contains(semantic::SCHEMA_CATALOGUE),
        "the refusal does not name the file or the cause: {err}"
    );
    assert!(
        !err.contains("LOAD"),
        "the extension was loaded before its bundle was verified: {err}"
    );
    assert_eq!(semantics, vec![SemanticType::NotAsked]);

    // Remove the contradiction and the same bundle gets past this check, dying
    // at the LOAD instead — two refusals from bundles differing only in a
    // manifest is what says the manifest is what was consulted.
    std::fs::remove_file(dir.join(semantic::MANIFEST_FILE)).unwrap();
    let (_, err, _) = load_against(&dir);
    let err = err.expect("a fake extension still cannot load");
    assert!(
        err.contains("LOAD"),
        "expected the load to be what failed: {err}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A session with a native type source cannot acquire an extension, and CAN
/// still autoload one it already has.
///
/// Both halves, and the second is not padding — it is a regression test. The
/// first version of this restriction reused `NetworkPolicy::Disabled` wholesale,
/// which also switches `autoload_known_extensions` off. Autoload is not a
/// network control: it is how DuckDB registers an extension it already has, and
/// the bundled library carries `parquet` statically. Turning it off stopped
/// Parquet files opening at all, which is a user's first action after choosing
/// one.
#[test]
fn a_native_type_source_costs_the_session_acquisition_but_not_autoload() {
    let dir = std::env::temp_dir().join(format!("bf-settings-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    synthetic_bundle(&dir, r#"["identity.person.email"]"#, ONE_LABEL_CATALOGUE);

    let settings = |type_source: Option<semantic::TypeSourceSpec>| {
        let parsed = parse_spec(FIXTURE, Format::Yaml).expect("the fixture parses");
        let analysis = analyse_spec(&parsed.spec).expect("the fixture analyses");
        let options = LoadOptions {
            type_source,
            ..LoadOptions::default() // NetworkPolicy::Auto
        };
        let session = Engine::new()
            .load_spec_with(parsed.spec, analysis, None, &options)
            .expect("the fixture loads")
            .session;
        let get = |k: &str| -> String {
            session
                .duckdb_setting(k)
                .unwrap_or_else(|e| panic!("reading {k}: {e}"))
        };
        (
            get("autoinstall_known_extensions"),
            get("autoload_known_extensions"),
            get("custom_extension_repository"),
        )
    };

    // A bundle needs unsigned extensions, so acquisition is off …
    let (autoinstall, autoload, repo) =
        settings(Some(semantic::TypeSourceSpec::Bundle(dir.clone())));
    assert_eq!(
        autoinstall, "false",
        "a relaxed session could still install"
    );
    assert!(
        repo.contains("/dev/null/"),
        "the extension repository still resolves: {repo}"
    );
    // … and autoload is NOT, because that is what registers the statically
    // linked parquet reader.
    assert_eq!(
        autoload, "true",
        "autoload was switched off, which stops a Parquet file opening"
    );

    // With no type source the session is untouched.
    let (autoinstall, autoload, _) = settings(None);
    assert_eq!(autoinstall, "true");
    assert_eq!(autoload, "true");

    std::fs::remove_dir_all(&dir).ok();
}

/// A directory that is not a bundle is refused, by name, and the session still
/// loads.
///
/// The second half matters as much as the first: a spec renders identically
/// with or without a type source, so losing the whole dashboard over a missing
/// optional file would be absurd.
#[test]
fn a_directory_that_is_not_a_bundle_is_refused_and_the_load_survives() {
    let dir = std::env::temp_dir().join(format!("bf-not-a-bundle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (name, err, semantics) = load_against(&dir);
    assert_eq!(name, None, "nothing came up");
    let err = err.expect("a configured-and-refused bundle has to say so");
    assert!(
        err.contains(semantic::EXTENSION_FILE) && err.contains(&dir.display().to_string()),
        "the refusal names neither the missing file nor the directory: {err}"
    );
    assert_eq!(semantics, vec![SemanticType::NotAsked]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A bundle whose extension file is a plain shared library — never stamped
/// with DuckDB's metadata trailer — is refused before any `LOAD` is attempted.
#[test]
fn an_unstamped_extension_is_refused_by_the_abi_check() {
    let dir = std::env::temp_dir().join(format!("bf-unstamped-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(semantic::MODEL_DIR)).unwrap();
    std::fs::write(dir.join(semantic::EXTENSION_FILE), vec![0u8; 4096]).unwrap();
    std::fs::write(dir.join(semantic::SCHEMA_CATALOGUE), "[]").unwrap();

    let (name, err, semantics) = load_against(&dir);
    assert_eq!(name, None);
    let err = err.expect("an unstamped library is not a DuckDB extension");
    assert!(
        err.contains("not a DuckDB extension"),
        "the refusal does not name the trailer: {err}"
    );
    assert_eq!(semantics, vec![SemanticType::NotAsked]);
    std::fs::remove_dir_all(&dir).ok();
}

/// The canary. A bundle carrying the real extension and an unreadable model is
/// REFUSED, rather than reporting every column of every source as unlabelled.
///
/// This is the failure the canary exists for and it is invisible without one:
/// with no model reachable the classifier panics inside its own
/// `catch_unwind`, `ft_profile` returns `unknown` at confidence 0, and the SQL
/// surface is byte-for-byte what an honest "I have nothing to say about this
/// column" looks like. Every column in the application would quietly go
/// unlabelled and nothing would be wrong anywhere a reader could look.
#[test]
#[ignore = "needs a FineType bundle: set BRIGHTFIELD_FINETYPE_BUNDLE"]
fn a_bundle_whose_model_will_not_load_is_refused_not_reported_as_unlabelled() {
    let Some(real) = bundle() else {
        panic!("BRIGHTFIELD_FINETYPE_BUNDLE is not set");
    };
    let dir = std::env::temp_dir().join(format!("bf-dead-model-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(semantic::MODEL_DIR)).unwrap();
    std::fs::copy(
        real.join(semantic::EXTENSION_FILE),
        dir.join(semantic::EXTENSION_FILE),
    )
    .unwrap();
    std::fs::copy(
        real.join(semantic::SCHEMA_CATALOGUE),
        dir.join(semantic::SCHEMA_CATALOGUE),
    )
    .unwrap();
    // Present, so the existence check passes; not a model, so loading it does
    // not. This is the shape a truncated download or a bad `cp` leaves behind.
    std::fs::write(
        dir.join(semantic::MODEL_DIR).join("model.safetensors"),
        b"not a safetensors file",
    )
    .unwrap();

    let (name, err, semantics) = load_against(&dir);
    assert_eq!(name, None, "a bundle that cannot classify is not a source");
    let err = err.expect("a dead model has to be reported");
    assert!(
        err.contains("canary") && err.contains("unlabelled"),
        "the refusal does not explain what it prevented: {err}"
    );
    assert_eq!(
        semantics,
        vec![SemanticType::NotAsked],
        "a refused bundle claims nothing, rather than claiming `unknown`"
    );
    std::fs::remove_dir_all(&dir).ok();
}
