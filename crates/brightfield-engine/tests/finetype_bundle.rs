//! The semantic type source, exercised against a real FineType bundle.
//!
//! Every test here needs an artefact this repository does not vendor: an
//! extension binary, a ~17 MB model and a schema catalogue. They are therefore
//! `#[ignore]`d and read the bundle's location from
//! `BRIGHTFIELD_FINETYPE_BUNDLE` — so a plain `cargo test` reports them as
//! ignored (visible in the count) rather than silently passing on nothing.
//!
//! ```text
//! BRIGHTFIELD_FINETYPE_BUNDLE=/path/to/bundle \
//!   cargo test -p brightfield-engine --test finetype_bundle -- --ignored
//! ```
//!
//! The bundle's shape is fixed by `brightfield_engine::semantic`:
//! `finetype.duckdb_extension`, `model/`, `taxonomy-schemas.json`. It is
//! assembled by `scripts/package.sh` for a release; for a local run, build the
//! extension in a FineType checkout and copy the model beside it.
//!
//! What these DO cover: that the extension loads into the DuckDB this
//! workspace bundles, that it produces a label no DuckDB type carries, that a
//! column whose values contradict its label is reported unusable, and that
//! unusable is distinguishable from unlabelled. What they do NOT cover: the
//! packaged artefact (`scripts/verify-airgapped.sh` is that), or any label
//! beyond the two the fixtures pin.

use std::path::PathBuf;

use brightfield_engine::semantic::{self, ExtensionStamp, TypeSourceSpec};
use brightfield_engine::{
    Engine, LoadOptions, NetworkPolicy, ProfileOutcome, SemanticType, Session, ValueCheck,
};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};

/// The bundle under test, or `None` when the environment does not name one.
fn bundle() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("BRIGHTFIELD_FINETYPE_BUNDLE")?);
    assert!(
        dir.join(semantic::EXTENSION_FILE).is_file(),
        "BRIGHTFIELD_FINETYPE_BUNDLE={} carries no {}",
        dir.display(),
        semantic::EXTENSION_FILE
    );
    Some(dir)
}

/// A spec holding two inline columns, both VARCHAR as far as DuckDB is
/// concerned:
///
/// - `email` — genuine addresses.
/// - `claims_email` — eight strings of which exactly one is an address. The
///   classifier labels it `identity.person.email` anyway; the label's own
///   pattern does not.
const FIXTURE: &str = r#"
data:
  people:
    - { email: "alice@example.com", claims_email: "alice@example.com" }
    - { email: "bob@example.org",   claims_email: "not-an-email" }
    - { email: "carol@example.net", claims_email: "also not one" }
    - { email: "dan@corp.co.uk",    claims_email: "@@@" }
    - { email: "erin@example.com",  claims_email: "nope" }
    - { email: "frank@example.org", claims_email: "x" }
    - { email: "grace@example.net", claims_email: "y z" }
    - { email: "heidi@corp.co.uk",  claims_email: "!!!" }
plot:
  - mark: dot
    data: { from: people }
"#;

/// Load `FIXTURE` with the network denied and the bundle as the type source.
fn session_with(bundle: &std::path::Path) -> Session {
    let parsed = parse_spec(FIXTURE, Format::Yaml).expect("the fixture parses");
    let analysis = analyse_spec(&parsed.spec).expect("the fixture analyses");
    let options = LoadOptions {
        network: NetworkPolicy::Disabled,
        extension_directory: None,
        type_source: Some(TypeSourceSpec::Bundle(bundle.to_path_buf())),
    };
    let load = Engine::new()
        .load_spec_with(parsed.spec, analysis, None, &options)
        .expect("the fixture loads");
    assert_eq!(
        load.session.type_source_error(),
        None,
        "the bundle failed to come up"
    );
    assert!(
        load.session.type_source_name().is_some(),
        "no type source after a clean open"
    );
    load.session
}

fn columns(session: &Session) -> Vec<brightfield_engine::ColumnProfile> {
    let profile = session
        .profile_sources()
        .into_iter()
        .find(|p| p.name == "people")
        .expect("the fixture declares one source");
    match profile.outcome {
        ProfileOutcome::Profiled { columns, .. } => columns,
        other => panic!("expected a profiled source, got {other:?}"),
    }
}

fn column<'a>(
    cols: &'a [brightfield_engine::ColumnProfile],
    name: &str,
) -> &'a brightfield_engine::ColumnProfile {
    cols.iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no column {name}"))
}

/// AC1. The engine loads the extension out of a bundle directory with the
/// network denied, and gets back a label DuckDB's own type system has no way
/// to express.
///
/// The `assert_ne` on `type_name` is the load-bearing half: `VARCHAR` is what
/// DESCRIBE says about this column, and `identity.person.email` is not a
/// DuckDB type, an alias for one, or derivable from one.
#[test]
#[ignore = "needs a FineType bundle: set BRIGHTFIELD_FINETYPE_BUNDLE"]
fn a_label_arrives_that_a_database_type_could_not_have_produced() {
    let Some(dir) = bundle() else {
        panic!("BRIGHTFIELD_FINETYPE_BUNDLE is not set");
    };
    let session = session_with(&dir);
    let cols = columns(&session);
    let email = column(&cols, "email");

    assert_eq!(email.type_name, "VARCHAR", "DuckDB's whole answer");
    match &email.semantic {
        SemanticType::Labelled {
            label,
            confidence,
            check,
        } => {
            assert_eq!(label, "identity.person.email");
            assert_ne!(
                label, &email.type_name,
                "the semantic label restates the database type"
            );
            assert!(
                *confidence > 0.0,
                "confidence {confidence} is not a number \
                the classifier produced — a model that failed to load returns 0.0"
            );
            assert_eq!(
                check,
                &ValueCheck::Checked {
                    checked: 8,
                    failed: 0
                },
                "all eight addresses satisfy the email pattern"
            );
        }
        other => panic!("expected a checked email label, got {other:?}"),
    }
}

/// AC3. A column the classifier labels and the values contradict is
/// `Unusable`, and `Unusable` is not `Unlabelled`.
///
/// Both facts are asserted here rather than in two tests because the point is
/// the DIFFERENCE: a caller has to be able to act on "the values disagree"
/// without acting the same way on "there is nothing to say".
#[test]
#[ignore = "needs a FineType bundle: set BRIGHTFIELD_FINETYPE_BUNDLE"]
fn a_column_whose_values_contradict_its_label_is_unusable_and_not_merely_unlabelled() {
    let Some(dir) = bundle() else {
        panic!("BRIGHTFIELD_FINETYPE_BUNDLE is not set");
    };
    let session = session_with(&dir);
    let cols = columns(&session);
    let claims = column(&cols, "claims_email");

    match &claims.semantic {
        SemanticType::Unusable {
            label,
            checked,
            failed,
            first_failure,
            ..
        } => {
            assert_eq!(label, "identity.person.email");
            assert_eq!(*checked, 8);
            assert_eq!(
                *failed, 7,
                "one of the eight is a real address; the rest are not"
            );
            assert!(
                first_failure.is_some(),
                "an unusable column has to say what failed"
            );
        }
        other => panic!("expected an unusable column, got {other:?}"),
    }

    assert_ne!(
        claims.semantic,
        SemanticType::Unlabelled,
        "a contradicted label must not read as no label"
    );
    assert!(
        !claims.semantic.is_usable(),
        "nothing may draw from a contradicted column"
    );
    assert!(
        column(&cols, "email").semantic.is_usable(),
        "the sound column in the same pass is unaffected"
    );
    // Both carry the SAME label. The label alone cannot tell them apart —
    // which is exactly why the check exists.
    assert_eq!(
        claims.semantic.label(),
        column(&cols, "email").semantic.label()
    );
}

/// AC5, live. The bundled extension's own metadata trailer is compared against
/// the DuckDB this workspace links, through the same function the bundle
/// loader uses — and then the extension is actually loaded and made to answer,
/// because a stamp comparison cannot see a runtime break.
#[test]
#[ignore = "needs a FineType bundle: set BRIGHTFIELD_FINETYPE_BUNDLE"]
fn the_bundled_extension_and_the_bundled_duckdb_agree() {
    let Some(dir) = bundle() else {
        panic!("BRIGHTFIELD_FINETYPE_BUNDLE is not set");
    };
    let bytes = std::fs::read(dir.join(semantic::EXTENSION_FILE)).expect("read the extension");
    let stamp: ExtensionStamp = semantic::read_stamp(&bytes).expect("a stamped extension");
    assert_eq!(
        stamp.abi_type, "C_STRUCT",
        "only the stable C API survives a DuckDB bump"
    );

    let session = session_with(&dir);
    let (platform, version) = session
        .duckdb_platform_and_version()
        .expect("the session's DuckDB names itself");
    semantic::check_abi(&stamp, &platform, &version)
        .expect("the bundled extension is loadable by the bundled DuckDB");

    // And it did not merely load — it answered.
    assert!(session
        .type_source_name()
        .is_some_and(|n| n.contains("finetype")));
}

/// A session given no bundle claims nothing about any column's meaning.
///
/// This one is NOT ignored: it is the guard that keeps the default path honest
/// when the bundle is absent, which is every CI run.
#[test]
fn without_a_bundle_no_column_carries_a_claim() {
    let parsed = parse_spec(FIXTURE, Format::Yaml).expect("the fixture parses");
    let analysis = analyse_spec(&parsed.spec).expect("the fixture analyses");
    let session = Engine::new()
        .load_spec(parsed.spec, analysis, None)
        .expect("the fixture loads")
        .session;
    assert_eq!(session.type_source_name(), None);
    assert_eq!(session.type_source_error(), None);
    let cols = columns(&session);
    assert_eq!(cols.len(), 2);
    for c in &cols {
        assert_eq!(
            c.semantic,
            SemanticType::NotAsked,
            "{} claims a meaning nobody asked for",
            c.name
        );
        assert!(!c.semantic.is_usable());
    }
    // The type name still arrives — the semantic pass is additive.
    assert_eq!(column(&cols, "email").type_name, "VARCHAR");
}
