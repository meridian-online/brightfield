//! The hand-authored hostile corpus, round-tripped through the arc write path.
//!
//! Local/cloud parity has a testable meaning: a spec edited here and a spec
//! hand-written must be indistinguishable to `arc`. Each corpus spec is
//! deliberately hostile to a serialise-back writer — comments that explain
//! WHY, non-default key order, block-vs-flow style variation, block scalars
//! (`|`, `|-`, `>`), trailing whitespace inside a literal block, and the
//! ambiguous comment-ownership case in a block sequence. A machine-generated
//! corpus would pass trivially and prove nothing; every file in
//! `tests/corpus/` was written by hand, with intent recorded in its comments.
//!
//! Three properties, each ASSERTED rather than eyeballed:
//!
//! 1. **arc loads every one** — [`arc::spec::Manifest::from_yaml_str`] is the
//!    same parse + validation gate `arc run` loads a protocol with
//!    (`Manifest::load` delegates to it), so a corpus spec passing here is a
//!    spec the engine accepts. (Executing the corpus under the `arc` *binary*
//!    is a local proof, not part of this suite: the binary is not a build
//!    product of this workspace — it lives in the arcform repo behind its
//!    `cli` feature — and a hermetic test cannot shell out to it. The corpus
//!    is authored so that every command-step spec runs cleanly under
//!    `arc run` when that binary is available; `ops-flow.yaml` alone is
//!    load-gated only, since its fetch step would touch the network.)
//! 2. **Edits preserve every untargeted byte** — for each spec, the expected
//!    post-edit document is CONSTRUCTED from the original bytes (a single
//!    `replacen` of the targeted fragment, or an append), so byte-equality
//!    against it proves the edit touched nothing else. No canonicalisation,
//!    no reflow, no comment loss.
//! 3. **The result still loads** — `apply_edits`/`edit_spec` gate the result
//!    through the same loader before it can reach disk, and the file-level
//!    round trip is exercised against a real directory via [`edit_spec`].
//!
//! This test file is also the proof of a negative: brightfield edits specs
//! exclusively through `arc::spec::{apply_edits, edit_spec}` — nowhere in this
//! workspace is a whole spec it did not create re-serialised.

use std::fs;
use std::path::PathBuf;

use arc::spec::{apply_edits, edit_spec, Manifest, SpecEdit, MANIFEST_FILENAME};

/// The corpus, by construction-kind of its byte-level expectation.
const CORPUS: &[&str] = &[
    "pinned-fetch.yaml",
    "reordered.yaml",
    "styles.yaml",
    "block-scalars.yaml",
    "seq-comments.yaml",
    "ops-flow.yaml",
];

fn corpus_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus")
        .join(name)
}

fn corpus(name: &str) -> String {
    fs::read_to_string(corpus_path(name)).unwrap_or_else(|e| panic!("read corpus {name}: {e}"))
}

/// A scratch protocol directory holding one corpus spec as `arcform.yaml`.
struct SpecDir(PathBuf);

impl SpecDir {
    fn new(test: &str, text: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("bf-roundtrip-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch dir");
        fs::write(dir.join(MANIFEST_FILENAME), text).expect("seed spec");
        Self(dir)
    }

    fn read_back(&self) -> String {
        fs::read_to_string(self.0.join(MANIFEST_FILENAME)).expect("read back")
    }
}

impl Drop for SpecDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Apply `edits` to `name` both in memory and against a real directory, and
/// assert the result is byte-for-byte `expected` in both — the file-level
/// round trip and the in-memory one cannot drift apart.
fn assert_roundtrip(name: &str, test: &str, edits: &[SpecEdit], expected: &str) {
    let original = corpus(name);

    // In memory: apply + gate.
    let validated =
        apply_edits(&original, edits).unwrap_or_else(|e| panic!("{name}: edit refused: {e}"));
    assert_eq!(
        validated.text(),
        expected,
        "{name}: a byte outside the targeted edit changed (in-memory path)"
    );

    // Against a directory: the whole read -> apply -> validate -> atomic-write
    // path, then the bytes that actually landed on disk.
    let dir = SpecDir::new(test, &original);
    edit_spec(&dir.0, edits).unwrap_or_else(|e| panic!("{name}: edit_spec refused: {e}"));
    assert_eq!(
        dir.read_back(),
        expected,
        "{name}: a byte outside the targeted edit changed (file path)"
    );

    // The result is still a spec arc loads — the parity property itself.
    Manifest::from_yaml_str(expected)
        .unwrap_or_else(|e| panic!("{name}: edited spec no longer loads: {e}"));
}

/// Every corpus spec passes the gate `arc run` loads with, before any edit.
#[test]
fn corpus_loads_through_arcs_own_gate() {
    for name in CORPUS {
        let m = Manifest::from_yaml_str(&corpus(name))
            .unwrap_or_else(|e| panic!("{name}: arc refuses the corpus spec: {e}"));
        assert!(!m.steps.is_empty(), "{name}: a corpus spec exercises steps");
    }
}

/// Replacing a quoted param default touches its bytes and nothing else — the
/// trailing same-line comment after the value survives verbatim.
#[test]
fn pinned_fetch_replace_preserves_trailing_comment() {
    let original = corpus("pinned-fetch.yaml");
    let expected = original.replacen("default: \"2024\"", "default: \"2025\"", 1);
    assert_ne!(original, expected, "the edit targets a real fragment");
    assert_roundtrip(
        "pinned-fetch.yaml",
        "pinned",
        &[SpecEdit::Replace {
            path: vec!["params".into(), "station_rev".into(), "default".into()],
            value: "\"2025\"".into(),
        }],
        &expected,
    );
    // The comment sitting on the edited line is untouched.
    assert!(
        expected.contains("default: \"2025\"      # see the header comment before bumping this"),
        "the trailing comment survives on the edited line"
    );
}

/// Adding a root key lands after the AUTHOR's last entry — `name:` at the
/// bottom, because the author put it there — not at any canonical position.
#[test]
fn reordered_add_respects_the_authors_key_order() {
    let original = corpus("reordered.yaml");
    assert!(
        original.ends_with("name: reordered\n"),
        "fixture: the author's signature key is last"
    );
    let expected = format!("{original}timeout_sec: 600\n");
    assert_roundtrip(
        "reordered.yaml",
        "reordered",
        &[SpecEdit::Add {
            path: vec![],
            key: "timeout_sec".into(),
            value: "600".into(),
        }],
        &expected,
    );
}

/// A batch of two edits — one inside a flow mapping, one a three-word
/// fragment of a folded scalar — leaves the mixed styles exactly as authored.
#[test]
fn styles_batch_edit_preserves_flow_and_folded_style() {
    let original = corpus("styles.yaml");
    let expected =
        original
            .replacen("\"hello\"", "\"hullo\"", 1)
            .replacen("precisely", "exactly", 1);
    assert_roundtrip(
        "styles.yaml",
        "styles",
        &[
            SpecEdit::Replace {
                path: vec!["params".into(), "greeting".into(), "default".into()],
                value: "\"hullo\"".into(),
            },
            SpecEdit::RewriteFragment {
                path: vec!["steps".into(), 1usize.into(), "command".into()],
                from: "precisely".into(),
                to: "exactly".into(),
            },
        ],
        &expected,
    );
    // The flow-mapped sequence item and the flow param survive as flow.
    assert!(expected.contains("{name: flow_item, command: echo compact}"));
    assert!(expected.contains("greeting: { default: \"hullo\" }"));
}

/// A fragment rewrite inside a `|` literal block: the block's other lines —
/// including the one with deliberate trailing spaces — survive byte-exact,
/// and the `|-` chomped block below is untouched.
#[test]
fn block_scalars_keep_trailing_whitespace_and_chomping() {
    let original = corpus("block-scalars.yaml");
    assert!(
        original.contains("echo \"=== tide report ===\"   \n"),
        "fixture: the banner line really carries trailing spaces"
    );
    let expected = original.replacen("tide report", "tide ledger", 1);
    assert_roundtrip(
        "block-scalars.yaml",
        "blocks",
        &[SpecEdit::RewriteFragment {
            path: vec!["steps".into(), 0usize.into(), "command".into()],
            from: "tide report".into(),
            to: "tide ledger".into(),
        }],
        &expected,
    );
    assert!(
        expected.contains("echo \"=== tide ledger ===\"   \n"),
        "the trailing spaces are still there after the edit"
    );
}

/// The flush comment header and the blank-separated section marker in
/// `seq-comments.yaml`, spelled once for all three ownership tests.
const FLUSH_HEADER_AND_ITEM: &str = "  # flush against `second` — this is second's header and MUST travel with it\n  - name: second\n    command: echo second\n";
const SECTION_MARKER: &str = "  # a section marker for the tail of the pipeline";

/// A value edit inside the ambiguous sequence leaves both comments where the
/// author put them.
#[test]
fn seq_comments_value_edit_moves_no_comment() {
    let original = corpus("seq-comments.yaml");
    let expected = original.replacen("echo second", "echo 2nd", 1);
    assert_roundtrip(
        "seq-comments.yaml",
        "seq-replace",
        &[SpecEdit::Replace {
            path: vec!["steps".into(), 1usize.into(), "command".into()],
            value: "echo 2nd".into(),
        }],
        &expected,
    );
}

/// Deleting the item under the flush header takes the header with it — it is
/// the ITEM's comment — while the blank-separated section marker belongs to
/// the sequence and stays, along with every byte around it.
#[test]
fn seq_comments_delete_takes_the_flush_header_only() {
    let original = corpus("seq-comments.yaml");
    // The deleted extent: the blank line separating `first` from the header,
    // the flush header, and the item itself.
    let expected = original.replacen(&format!("\n{FLUSH_HEADER_AND_ITEM}"), "", 1);
    assert_roundtrip(
        "seq-comments.yaml",
        "seq-delete",
        &[SpecEdit::Delete {
            path: vec!["steps".into(), 1usize.into()],
        }],
        &expected,
    );
    assert!(
        !expected.contains("second's header"),
        "the flush header died with its item"
    );
    assert!(
        expected.contains(SECTION_MARKER),
        "the sequence-owned marker stayed put"
    );
}

/// Reordering the item moves its flush header WITH it; the section marker —
/// blank-separated, so sequence-owned — does not move, and every byte outside
/// the sequence splice is identical.
#[test]
fn seq_comments_reorder_moves_the_flush_header_with_its_item() {
    let original = corpus("seq-comments.yaml");
    // Observed, deterministic splice: the header+item block is lifted out of
    // its slot (its surrounding blank lines stay) and lands after the last
    // item's body — still flush above `- name: second`, so ownership is
    // preserved on re-parse.
    let expected = format!(
        "{}{FLUSH_HEADER_AND_ITEM}",
        original.replacen(FLUSH_HEADER_AND_ITEM, "", 1)
    );
    assert_roundtrip(
        "seq-comments.yaml",
        "seq-reorder",
        &[SpecEdit::Reorder {
            path: vec!["steps".into()],
            from: 1,
            to: 2,
        }],
        &expected,
    );
    // Re-parse: the moved item is now last, and still named by its header.
    let m = Manifest::from_yaml_str(&expected).expect("reordered spec loads");
    let names: Vec<&str> = m.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["first", "third", "second"],
        "the order really moved"
    );
    assert!(
        expected.contains("MUST travel with it\n  - name: second"),
        "the flush header is still flush above its item in the moved block"
    );
}

/// A replace inside a flow-style `with:` mapping preserves the flow braces,
/// the quoting style, and the trailing same-line comment culture of the file.
#[test]
fn ops_flow_replace_inside_flow_mapping() {
    let original = corpus("ops-flow.yaml");
    let expected = original.replacen(
        "'https://data.example/tides.csv'",
        "'https://mirror.example/tides.csv'",
        1,
    );
    assert_roundtrip(
        "ops-flow.yaml",
        "ops-flow",
        &[SpecEdit::Replace {
            path: vec!["steps".into(), 0usize.into(), "with".into(), "url".into()],
            value: "'https://mirror.example/tides.csv'".into(),
        }],
        &expected,
    );
    assert!(
        expected
            .contains("with: { url: 'https://mirror.example/tides.csv', out: build/tides.csv }"),
        "the flow mapping is still a flow mapping, single quotes intact"
    );
}

/// A refused edit writes nothing: targeting a path that does not exist leaves
/// the file on disk byte-identical.
#[test]
fn refused_edit_leaves_the_file_untouched() {
    let original = corpus("pinned-fetch.yaml");
    let dir = SpecDir::new("refused", &original);
    let err = edit_spec(
        &dir.0,
        &[SpecEdit::Replace {
            path: vec!["params".into(), "no_such_param".into(), "default".into()],
            value: "\"x\"".into(),
        }],
    );
    assert!(err.is_err(), "an unresolvable path is a refusal");
    assert_eq!(
        dir.read_back(),
        original,
        "a refused edit may not change a byte on disk"
    );
}
