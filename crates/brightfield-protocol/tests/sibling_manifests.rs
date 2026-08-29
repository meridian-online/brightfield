//! Does the pinned `arc` still understand the Protocols the pipeline actually
//! ships?
//!
//! **The failure this exists to catch.** `brightfield-protocol` consumes the
//! arcform spec library by git **rev** (`Cargo.toml`, and README §"The arcform
//! dependency" for why). The pin is deliberate; what is not deliberate is how
//! far behind it drifts. Between 2026-08-23 and 2026-08-29 arcform's operator
//! catalog grew `text_embed`, `umap_project` and `uv`, the sibling data repo
//! shipped Protocols that use them, and brightfield's pin stayed where it was —
//! so opening the sibling repo's `medmcqa/arcform.yaml` in the shell answered
//! `unknown operator 'text_embed' (not in the operator catalog)`. Nothing
//! reddened. The signal was a person opening a file and being refused.
//!
//! **Why the check lives here and not in the sibling repo.** The thing that
//! goes stale is *this* crate's pin, and the only build that knows what the pin
//! resolves to is this one. The sibling repo cannot see it.
//!
//! **How the manifests arrive.** `OPEN_ANALYTICS_DIR` names a checkout of the
//! sibling data repo. `.github/workflows/test.yml` checks it out into
//! `$RUNNER_TEMP` and sets the variable; locally, point it at the sibling
//! clone. The two tests that read it are `#[ignore]`d for exactly the reason
//! `brightfield-engine`'s bundle tests are: the input is not in this checkout,
//! so a per-push suite that hard-required it would be red on any machine
//! without it.
//!
//! **`#[ignore]` is the hole this file is most careful about.** `cargo test --
//! --ignored` with a filter matching nothing exits 0 having run nothing, and a
//! walk over an absent or empty directory reports success and reads exactly
//! like full coverage. Three guards, at three different altitudes:
//!
//! 1. [`collect_manifests`] returns `Err` — never an empty `Ok` — when the root
//!    is missing, when a required subtree is missing, when a subtree is under
//!    its floor, or when the total is under its floor. Each of those four
//!    refusals is held by a test: `floor_refuses_a_root_that_is_not_there`,
//!    `floor_refuses_an_empty_directory`,
//!    `floor_refuses_a_subtree_under_its_own_floor_even_when_the_total_is_met`,
//!    and `floor_accepts_the_census_this_check_was_cut_against` for the other side of it.
//! 2. The floors themselves are exercised by the four `floor_*` tests below,
//!    which are **not** `#[ignore]`d: they build synthetic trees under
//!    `CARGO_TARGET_TMPDIR` and assert each refusal fires. So the guard's own
//!    self-test runs in the ordinary `cargo test --workspace` on every push,
//!    with no sibling checkout needed.
//! 3. The workflow parses libtest's executed count back out and holds it to a
//!    floor, because a filter that matches nothing never reaches this file.

use std::fs;
use std::path::{Path, PathBuf};

use brightfield_protocol::graph::{build_graph, load_model_sources};
use brightfield_protocol::parse_manifest_str;

/// Where the sibling data repo is checked out.
const DIR_VAR: &str = "OPEN_ANALYTICS_DIR";

/// Subtrees that must be present, and how many `arcform.yaml` files each must
/// hold at minimum.
///
/// **Per subtree, not only in total** — the same reason
/// `nightly-network.yml`'s guard is per target. One total floor of ten stays
/// green when `datasets/` disappears and `examples/` grows by four, which is
/// the arrangement that would quietly stop checking the published packages.
///
/// The numbers are the census taken when this file was written (2026-08-29):
/// four datasets, six examples. They are FLOORS — a new Protocol in either
/// subtree must not redden this — so the only thing they catch is a subtree
/// shrinking or vanishing, which is the thing that would fake coverage.
const SUBTREE_FLOORS: &[(&str, usize)] = &[("datasets", 4), ("examples", 6)];

/// The sum of the subtree floors, stated once so the log shows what the walk
/// expected to examine.
const TOTAL_FLOOR: usize = 10;

/// The manifest filename arcform gives a Protocol.
const MANIFEST: &str = "arcform.yaml";

/// Every `arcform.yaml` under `root`, in sorted order, or the reason the walk
/// cannot be trusted.
///
/// Returns `Err` rather than an empty `Ok` for the shapes of absence the
/// `floor_*` tests below enumerate, so no caller can mistake "found nothing"
/// for "found nothing wrong".
fn collect_manifests(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.is_dir() {
        return Err(format!(
            "{} is not a directory — {DIR_VAR} must name a checkout of the \
             sibling data repo, not a file and not a path that does not exist",
            root.display()
        ));
    }

    let mut all = Vec::new();
    for (subtree, floor) in SUBTREE_FLOORS {
        let dir = root.join(subtree);
        if !dir.is_dir() {
            return Err(format!(
                "{} has no {subtree}/ subtree — the checkout is not the sibling \
                 data repo, or the repo's layout moved and this floor is stale",
                root.display()
            ));
        }
        let mut found = Vec::new();
        walk(&dir, &mut found).map_err(|e| format!("walking {}: {e}", dir.display()))?;
        found.sort();
        if found.len() < *floor {
            return Err(format!(
                "{}/ holds {} {MANIFEST} file(s), floor is {floor} — a walk over \
                 an emptied or moved subtree reports success and reads exactly \
                 like full coverage, so it is refused instead",
                subtree,
                found.len()
            ));
        }
        all.extend(found);
    }

    if all.len() < TOTAL_FLOOR {
        return Err(format!(
            "{} {MANIFEST} file(s) found under {}, floor is {TOTAL_FLOOR}",
            all.len(),
            root.display()
        ));
    }
    all.sort();
    Ok(all)
}

/// Depth-first walk collecting files named [`MANIFEST`]. Symlinked directories
/// are not followed — a self-referential link would not terminate, and nothing
/// in the sibling repo needs one.
fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            walk(&path, out)?;
        } else if meta.is_file() && path.file_name().is_some_and(|n| n == MANIFEST) {
            out.push(path);
        }
    }
    Ok(())
}

/// The sibling checkout, or a panic naming the variable. Deliberately not an
/// early `return` — a test that skips itself when its input is missing is a
/// test that passes when the input is missing.
fn sibling_root() -> PathBuf {
    let Some(dir) = std::env::var_os(DIR_VAR) else {
        panic!(
            "{DIR_VAR} is not set. This test reads the sibling data repo's \
             Protocols; CI checks it out and sets the variable, and locally it \
             points at the sibling clone."
        );
    };
    PathBuf::from(dir)
}

/// `root`-relative display path, so a failure names the Protocol the way the
/// sibling repo does rather than by an absolute CI path.
fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

// ---------------------------------------------------------------------------
// The check.
// ---------------------------------------------------------------------------

/// Every Protocol the sibling data repo ships loads through the same gate
/// `arc run` loads with — which is what the Protocol panel opens a file with.
///
/// A failure names the manifest and quotes arc's own diagnostic, so the CI log
/// says the same thing the shell said to the person who found this by hand:
/// `unknown operator 'text_embed' (not in the operator catalog)`.
#[test]
#[ignore = "needs the sibling data repo checked out: set OPEN_ANALYTICS_DIR"]
fn every_shipped_protocol_parses_against_the_pinned_arc() {
    let root = sibling_root();
    let manifests = collect_manifests(&root).unwrap_or_else(|e| panic!("{e}"));

    let mut refused = Vec::new();
    for path in &manifests {
        let text =
            fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        if let Err(e) = parse_manifest_str(&text) {
            refused.push(format!("  {}: {e}", rel(&root, path)));
        }
    }

    assert!(
        refused.is_empty(),
        "{} of {} shipped Protocol(s) are refused by the pinned arc — the \
         Protocol panel answers exactly this when someone opens one. Bump the \
         pin (README §\"The arcform dependency\": the arc rev, the root \
         [patch.crates-io] sqlparser rev, and Cargo.lock, in one commit).\n{}",
        refused.len(),
        manifests.len(),
        refused.join("\n")
    );

    println!(
        "{} shipped Protocol(s) parsed against the pinned arc (floor {TOTAL_FLOOR})",
        manifests.len()
    );
}

/// A Protocol that parses is not yet one the canvas can draw. This is the
/// floor under that difference: every shipped Protocol derives a graph with
/// nodes in it and lineage between them.
///
/// It is a floor, not a picture. What it catches is a manifest that loads and
/// then draws an empty or edgeless canvas — the shape a new operator vocabulary
/// would produce if nothing in the derivation recognised any of its `with:`
/// keys. It does NOT claim the drawing is right; see the PR that added this
/// file for what the 23-step crosswalk actually draws for `uv` steps.
#[test]
#[ignore = "needs the sibling data repo checked out: set OPEN_ANALYTICS_DIR"]
fn every_shipped_protocol_derives_a_graph_with_lineage() {
    let root = sibling_root();
    let manifests = collect_manifests(&root).unwrap_or_else(|e| panic!("{e}"));

    let mut thin = Vec::new();
    let mut drawn = 0usize;
    for path in &manifests {
        let text =
            fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let Ok(manifest) = parse_manifest_str(&text) else {
            continue; // the parse test above is where a refusal is reported
        };
        let dir = path.parent().expect("a manifest has a parent directory");
        let sources = load_model_sources(&manifest, dir);
        let graph = build_graph(&manifest, &sources);
        if graph.nodes.is_empty() || graph.edges.is_empty() {
            thin.push(format!(
                "  {}: {} step(s) -> {} node(s), {} edge(s)",
                rel(&root, path),
                manifest.steps.len(),
                graph.nodes.len(),
                graph.edges.len()
            ));
        } else {
            drawn += 1;
        }
    }

    assert!(
        thin.is_empty(),
        "{} shipped Protocol(s) parse but derive no drawable lineage — parsing \
         is necessary and not sufficient:\n{}",
        thin.len(),
        thin.join("\n")
    );
    assert_eq!(
        drawn,
        manifests.len(),
        "every walked Protocol must have been drawn"
    );

    println!("{drawn} shipped Protocol(s) derived a graph with lineage");
}

// ---------------------------------------------------------------------------
// The guard's own self-test. NOT `#[ignore]`d: these run in the ordinary
// `cargo test --workspace` on every push, with no sibling checkout, because a
// floor nobody exercises is a floor nobody knows fires.
// ---------------------------------------------------------------------------

/// Build a synthetic sibling tree under this integration test's own temp dir.
/// `counts` is (subtree, how many `arcform.yaml` files to plant).
fn plant(case: &str, counts: &[(&str, usize)]) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("sibling-{case}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create the synthetic root");
    for (subtree, n) in counts {
        let dir = root.join(subtree);
        fs::create_dir_all(&dir).expect("create the subtree");
        for i in 0..*n {
            let proto = dir.join(format!("p{i}"));
            fs::create_dir_all(&proto).expect("create the protocol directory");
            fs::write(proto.join(MANIFEST), "name: p\nsteps: []\n").expect("plant a manifest");
        }
    }
    root
}

#[test]
fn floor_refuses_a_root_that_is_not_there() {
    let missing = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sibling-absent");
    let _ = fs::remove_dir_all(&missing);
    let err = collect_manifests(&missing).expect_err("an absent root must be refused");
    assert!(err.contains("is not a directory"), "{err}");
}

#[test]
fn floor_refuses_an_empty_directory() {
    let root = plant("empty", &[]);
    let err = collect_manifests(&root).expect_err("an empty root must be refused");
    assert!(err.contains("has no datasets/ subtree"), "{err}");
}

#[test]
fn floor_refuses_a_subtree_under_its_own_floor_even_when_the_total_is_met() {
    // Fourteen manifests in total — comfortably over TOTAL_FLOOR — but
    // `datasets/` has three. A total-only floor would call this green.
    let root = plant("lopsided", &[("datasets", 3), ("examples", 11)]);
    let err =
        collect_manifests(&root).expect_err("a short subtree must be refused on its own floor");
    assert!(
        err.contains("datasets/ holds 3") && err.contains("floor is 4"),
        "the refusal names the subtree and its floor: {err}"
    );
}

#[test]
fn floor_accepts_the_census_this_check_was_cut_against() {
    let root = plant("census", &[("datasets", 4), ("examples", 6)]);
    let found = collect_manifests(&root).expect("four datasets and six examples clear the floors");
    assert_eq!(found.len(), TOTAL_FLOOR);
    // Sorted, and every entry is a manifest — the walk returns paths a caller
    // can read, not directories.
    assert!(found.windows(2).all(|w| w[0] < w[1]), "sorted");
    assert!(found
        .iter()
        .all(|p| p.file_name().is_some_and(|n| n == MANIFEST)));
}

#[test]
fn floor_finds_manifests_at_any_depth() {
    let root = plant("nested", &[("datasets", 4), ("examples", 6)]);
    // A Protocol nested one level deeper than the others is still found, so a
    // subtree reorganisation does not silently drop coverage.
    let deep = root.join("examples").join("group").join("deep");
    fs::create_dir_all(&deep).expect("create a deeper protocol directory");
    fs::write(deep.join(MANIFEST), "name: p\nsteps: []\n").expect("plant a nested manifest");
    let found = collect_manifests(&root).expect("the floors are met");
    assert_eq!(found.len(), TOTAL_FLOOR + 1);
}
