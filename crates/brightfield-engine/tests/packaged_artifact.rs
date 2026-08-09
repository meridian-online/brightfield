//! The packaged layout, resolved and opened the way a packaged binary does.
//!
//! Between "the files are in the tarball" and "the application finds them"
//! there is a gap nothing else here covers: `scripts/check-bundled-extension.sh`
//! reads a directory somebody hands it, and the engine's unit tests exercise
//! `bundle_beside` against synthesised directories. Neither asks whether the
//! shape `scripts/package.sh` actually produces is the shape
//! `LoadOptions::packaged` actually looks for.
//!
//! This does, against a real unpacked artefact:
//!
//! ```text
//! tar -xzf dist/brightfield-<version>-<target>.tar.gz -C /tmp/art
//! BRIGHTFIELD_UNPACKED_ARTIFACT=/tmp/art/brightfield-<version>-<target> \
//!   cargo test -p brightfield-engine --test packaged_artifact -- --ignored
//! ```
//!
//! Its own test binary because it must be the FIRST thing in its process to
//! set `FINETYPE_MODEL_DIR` — otherwise it would classify with some other
//! bundle's model and prove nothing about the artefact's own.
//!
//! What it does NOT cover: the GUI. It never launches the packaged binary, so
//! it says nothing about whether the application starts, renders, or reaches
//! the type source from its own process. `scripts/verify-airgapped.sh` is that,
//! and it opens windows.

use std::path::PathBuf;

use brightfield_engine::semantic::{self, TypeSourceSpec};
use brightfield_engine::{Engine, LoadOptions, NetworkPolicy, ProfileOutcome, SemanticType};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};

const FIXTURE: &str = r#"
data:
  people:
    - { email: "alice@example.com" }
    - { email: "bob@example.org" }
    - { email: "carol@example.net" }
    - { email: "dan@corp.co.uk" }
plot:
  - mark: dot
    data: { from: people }
"#;

/// The packaged tarball's own layout resolves, opens with the network denied,
/// and types a column — using the artefact's extension and the artefact's
/// model, nothing else on the machine.
#[test]
#[ignore = "needs an unpacked artefact: set BRIGHTFIELD_UNPACKED_ARTIFACT"]
fn a_packaged_artifact_resolves_its_own_bundle_and_types_a_column() {
    let Some(root) = std::env::var_os("BRIGHTFIELD_UNPACKED_ARTIFACT").map(PathBuf::from) else {
        panic!("BRIGHTFIELD_UNPACKED_ARTIFACT is not set");
    };
    let exe = root.join("brightfield");
    assert!(
        exe.is_file(),
        "{} is not an unpacked Brightfield artefact",
        root.display()
    );

    // The resolution the application performs, against the directory the
    // packaging script actually produced.
    let bundle = semantic::bundle_beside(&exe).unwrap_or_else(|| {
        panic!(
            "the packaged layout at {} is not one bundle_beside finds",
            root.display()
        )
    });
    assert_eq!(bundle, root.join("finetype"));

    // Nothing in this process has pointed FINETYPE_MODEL_DIR anywhere, so
    // opening this bundle points it at the ARTEFACT's model. If that model is
    // absent, truncated or a dangling symlink, the canary inside `open` fires
    // and this fails.
    assert_eq!(
        std::env::var_os("FINETYPE_MODEL_DIR"),
        None,
        "this test only proves the artefact's own model when nothing has \
         already pointed the extension elsewhere"
    );

    let parsed = parse_spec(FIXTURE, Format::Yaml).expect("the fixture parses");
    let analysis = analyse_spec(&parsed.spec).expect("the fixture analyses");
    let options = LoadOptions {
        network: NetworkPolicy::Disabled,
        extension_directory: None,
        type_source: Some(TypeSourceSpec::Bundle(bundle.clone())),
    };
    let session = Engine::new()
        .load_spec_with(parsed.spec, analysis, None, &options)
        .expect("the fixture loads")
        .session;
    assert_eq!(
        session.type_source_error(),
        None,
        "the artefact's own bundle did not come up"
    );

    assert_eq!(
        std::env::var_os("FINETYPE_MODEL_DIR").map(PathBuf::from),
        Some(bundle.join(semantic::MODEL_DIR)),
        "the extension was not pointed at the artefact's model"
    );

    let columns = match session
        .profile_sources()
        .into_iter()
        .find(|p| p.name == "people")
        .expect("one source")
        .outcome
    {
        ProfileOutcome::Profiled { columns, .. } => columns,
        other => panic!("expected a profiled source, got {other:?}"),
    };
    let email = columns.iter().find(|c| c.name == "email").expect("email");
    match &email.semantic {
        SemanticType::Labelled { label, .. } => assert_eq!(label, "identity.person.email"),
        other => panic!("the packaged bundle did not type the column: {other:?}"),
    }
}
