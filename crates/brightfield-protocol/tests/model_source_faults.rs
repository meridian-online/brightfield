//! A protocol whose models cannot be read must not draw a chart that looks
//! finished.
//!
//! Measured before this suite existed: two runs of the same binary — one whose
//! `models/` directory had been made unreadable, one whose grant never covered
//! it — produced the same picture, the same node count and the same exit code
//! as each other, with nothing on screen saying either had drawn less than it
//! was asked to. The degrade itself is deliberate (`graph.rs` states why: a
//! protocol naming a model that is not there should still draw what it can),
//! so what is fixed here is that a degraded render is legible AS degraded, and
//! that *absent* and *unreadable* stop being one shape — one is a protocol
//! authored that way, the other is a permission the reader can widen.
//!
//! Unix only: the refused read is produced with `chmod`, and there is no
//! portable equivalent. The one CI runner this repo builds on is macOS.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use brightfield_protocol::graph::{build_graph, degrades, load_model_sources, Degradation};
use brightfield_protocol::layout::{layout, Layout, LayoutConfig};
use brightfield_protocol::{parse_manifest_str, AssetGraph, AssetKind};

const MANIFEST: &str = r"
name: acc
steps:
  - name: fetch
    op: http_fetch@1
    with:
      url: https://example.com/a.csv
      out: build/a.csv
  - name: tier
    sql: models/entity_tiering_rules.sql
    depends_on: [build/a.csv]
  - name: export
    op: parquet_export@1
    with:
      input: tiered
      dest: build/tiered.parquet
";

const MODEL: &str = "CREATE TABLE staged AS SELECT * FROM read_csv('build/a.csv');\n\
                     CREATE TABLE tiered AS SELECT * FROM staged;\n";

/// Restore a path's mode when the scope ends — including on a panic. A test
/// that leaves a `000` directory behind poisons every test after it and the
/// next `cargo` invocation's cleanup with it, so this is a `Drop` rather than
/// a line at the end of the happy path.
struct RestoreMode {
    path: PathBuf,
    mode: u32,
}

impl RestoreMode {
    /// Set `path` to `mode`, remembering what it was.
    fn set(path: &Path, mode: u32) -> Self {
        let was = fs::metadata(path).expect("stat before chmod").permissions();
        let guard = Self {
            path: path.to_path_buf(),
            mode: was.mode(),
        };
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
        guard
    }
}

impl Drop for RestoreMode {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
    }
}

/// A fresh protocol directory: `arcform.yaml` plus its one model.
fn fixture(case: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("model_faults_{case}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("models")).expect("create fixture");
    fs::write(dir.join("arcform.yaml"), MANIFEST).expect("write manifest");
    fs::write(dir.join("models/entity_tiering_rules.sql"), MODEL).expect("write model");
    dir
}

/// The graph as the offline path builds it: manifest on disk, models read off
/// the same disk. Nothing is stubbed — the whole point is the real read.
fn graph_at(dir: &Path) -> AssetGraph {
    let text = fs::read_to_string(dir.join("arcform.yaml")).expect("read manifest");
    let manifest = parse_manifest_str(&text).expect("parse manifest");
    let sources = load_model_sources(&manifest, dir);
    build_graph(&manifest, &sources)
}

/// Everything the renderer draws a node from: the laid-out geometry, and each
/// node's visual class and label. `brightfield-render` reads exactly these —
/// it switches its treatment on `AssetKind` and paints `AssetNode::label` into
/// the card the layout sized. Equal here means the same picture; different
/// here means a different one.
fn render_inputs(graph: &AssetGraph) -> (Layout, Vec<(AssetKind, String)>) {
    let drawn = graph
        .nodes
        .values()
        .map(|n| (n.kind, n.label.clone()))
        .collect();
    (layout(graph, &LayoutConfig::default()), drawn)
}

/// The step-level chip a degraded `sql:` step draws, if there is one. The id
/// is shared with the step's first statement intermediate — the same slot
/// holds one or the other — so the kind, not the id, decides.
fn chip_label(graph: &AssetGraph) -> Option<String> {
    let node = graph.nodes.get("stmt.acc.tier#0")?;
    (node.kind == AssetKind::Opaque).then(|| node.label.clone())
}

/// True when this process can read `path` regardless of its mode — i.e. it is
/// running as root, where `chmod 000` refuses nobody and the refused-read half
/// of these tests cannot be staged at all.
fn refusal_is_impossible(path: &Path) -> bool {
    fs::read_to_string(path).is_ok()
}

/// AC1 + AC3. Both cases are built for real — models readable, then models
/// present and refused — and what the renderer would draw differs. Making the
/// two identical again reddens this.
#[test]
fn a_refused_model_does_not_render_like_a_readable_one() {
    let readable_dir = fixture("readable");
    let readable = graph_at(&readable_dir);
    assert!(
        degrades(&readable).is_empty(),
        "the control case is a complete render: {:?}",
        degrades(&readable)
    );
    assert!(
        chip_label(&readable).is_none(),
        "a readable model explodes into statements, not a chip"
    );

    let refused_dir = fixture("refused");
    let model = refused_dir.join("models/entity_tiering_rules.sql");
    let _guard = RestoreMode::set(&refused_dir.join("models"), 0o000);
    if refusal_is_impossible(&model) {
        eprintln!(
            "SKIPPED a_refused_model_does_not_render_like_a_readable_one: this process reads \
             a 000 directory (running as root), so a refused read cannot be staged"
        );
        return;
    }
    let refused = graph_at(&refused_dir);

    assert_ne!(
        render_inputs(&readable),
        render_inputs(&refused),
        "a refused model must not draw the same picture as a readable one"
    );
    // Not merely different somewhere: the refusal is a visible chip carrying a
    // badge, where the readable model is the statements it contains.
    assert_eq!(
        chip_label(&refused).as_deref(),
        Some("unreadable: models/entity_tiering_rules.sql"),
        "the chip names the cause on the canvas"
    );
    assert_ne!(
        readable.nodes.len(),
        refused.nodes.len(),
        "the refused render is not the same node count as the complete one"
    );

    drop(_guard);
    let _ = fs::remove_dir_all(&readable_dir);
    let _ = fs::remove_dir_all(&refused_dir);
}

/// AC2. A model that was never there and a model that is there and refused are
/// two different chips, and the difference sits at the head of the label —
/// where a card too narrow for the whole path still shows it.
#[test]
fn absent_and_refused_models_draw_different_chips() {
    let absent_dir = fixture("absent");
    fs::remove_file(absent_dir.join("models/entity_tiering_rules.sql")).expect("remove model");
    let absent = graph_at(&absent_dir);

    let refused_dir = fixture("refused_vs_absent");
    let model = refused_dir.join("models/entity_tiering_rules.sql");
    let _guard = RestoreMode::set(&model, 0o000);
    if refusal_is_impossible(&model) {
        eprintln!(
            "SKIPPED absent_and_refused_models_draw_different_chips: this process reads a 000 \
             file (running as root), so a refused read cannot be staged"
        );
        return;
    }
    let refused = graph_at(&refused_dir);

    let (a, r) = (
        chip_label(&absent).expect("absent model degrades to a chip"),
        chip_label(&refused).expect("refused model degrades to a chip"),
    );
    assert_ne!(a, r, "the two causes are not one label");
    let diverges_at = a
        .chars()
        .zip(r.chars())
        .position(|(x, y)| x != y)
        .unwrap_or(usize::MAX);
    assert!(
        diverges_at < 4,
        "the cause is legible before a narrow card elides the tail (diverges at char \
         {diverges_at}): {a:?} vs {r:?}"
    );
    assert_ne!(
        render_inputs(&absent),
        render_inputs(&refused),
        "absent and refused are two pictures, not one"
    );

    drop(_guard);
    let _ = fs::remove_dir_all(&absent_dir);
    let _ = fs::remove_dir_all(&refused_dir);
}

/// AC4. The channel that answers "did this draw everything it was asked for"
/// without anyone looking at the picture, and says which class of problem it
/// hit when it did not.
#[test]
fn degrades_names_the_class_off_a_real_filesystem() {
    let complete_dir = fixture("complete");
    assert_eq!(
        degrades(&graph_at(&complete_dir)),
        Vec::new(),
        "a complete render reports nothing"
    );

    let absent_dir = fixture("absent_class");
    fs::remove_file(absent_dir.join("models/entity_tiering_rules.sql")).expect("remove model");
    let found = degrades(&graph_at(&absent_dir));
    assert_eq!(found.len(), 1, "one degrade: {found:?}");
    assert_eq!(found[0].class, Degradation::ModelAbsent);
    assert_eq!(found[0].step.as_deref(), Some("tier"));

    let refused_dir = fixture("refused_class");
    let model = refused_dir.join("models/entity_tiering_rules.sql");
    let _guard = RestoreMode::set(&refused_dir.join("models"), 0o000);
    if refusal_is_impossible(&model) {
        eprintln!(
            "SKIPPED the refused half of degrades_names_the_class_off_a_real_filesystem: this \
             process reads a 000 directory (running as root)"
        );
        return;
    }
    let found = degrades(&graph_at(&refused_dir));
    assert_eq!(found.len(), 1, "one degrade: {found:?}");
    assert_eq!(
        found[0].class,
        Degradation::ModelUnreadable,
        "a refused read is not reported as an absent file — the reader can fix one of them"
    );
    assert!(
        found[0].detail.contains("Access, not authorship"),
        "the detail says whose problem it is: {:?}",
        found[0].detail
    );

    drop(_guard);
    for dir in [&complete_dir, &absent_dir, &refused_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

/// The `Drop` guard is load-bearing, so it is held to it: a panic inside the
/// scope still restores the mode. Without this the first failing assertion in
/// any test above leaves a `000` directory in the target tree.
#[test]
fn the_mode_guard_restores_through_a_panic() {
    let dir = fixture("guard");
    let models = dir.join("models");
    let before = fs::metadata(&models).expect("stat").permissions().mode();
    // The deliberate panic below would otherwise print a backtrace notice into
    // an otherwise-passing run, which reads as a failure to whoever is looking.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let unwound = std::panic::catch_unwind(|| {
        let _guard = RestoreMode::set(&models, 0o000);
        panic!("as an assertion would");
    });
    std::panic::set_hook(hook);
    assert!(unwound.is_err(), "the scope did panic");
    assert_eq!(
        fs::metadata(&models).expect("stat").permissions().mode(),
        before,
        "the mode is back after the unwind"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A caller-built source map — a start whose models are compiled into the
/// binary, with no directory to read — has no filesystem verdict to report,
/// and must not be dressed up as one.
#[test]
fn a_source_map_that_touched_no_filesystem_claims_no_verdict() {
    let manifest = parse_manifest_str(MANIFEST).expect("parse manifest");
    let mut sources: BTreeMap<String, Result<String, String>> = BTreeMap::new();
    sources.insert(
        "tier".to_string(),
        Err("models/entity_tiering_rules.sql: not embedded".to_string()),
    );
    let found = degrades(&build_graph(&manifest, &sources));
    assert_eq!(found.len(), 1, "one degrade: {found:?}");
    assert_eq!(found[0].class, Degradation::ModelUnavailable);
}
