//! Folding a run's live `.jsonl` stream collapses to the latest state per step
//! (last-valid-line-wins) and surfaces the terminal run outcome. Malformed and
//! blank lines are skipped; the tailing `StreamReader` picks up appended lines
//! across polls and buffers a partial trailing line.

use std::io::Write;
use std::path::PathBuf;

use brightfield_protocol::contract::Outcome;
use brightfield_protocol::{fold_stream, StreamReader};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
}

#[test]
fn fold_collapses_to_latest_state_per_step() {
    let content = std::fs::read_to_string(fixture("sample_run.jsonl")).expect("read stream fixture");
    let state = fold_stream(&content);

    // One entry per distinct step; last-valid-line-wins (load went
    // running -> success).
    assert_eq!(state.steps.len(), 2);
    assert_eq!(state.steps["load"].state, "success");
    assert_eq!(state.steps["load"].ts.as_deref(), Some("2026-07-18T10:01:07Z"));
    assert_eq!(state.steps["tally"].state, "success");

    // Terminal event captured — the reconcile signal.
    assert!(state.complete);
    assert_eq!(state.outcome, Some(Outcome::Success));
}

#[test]
fn fold_skips_malformed_and_blank_lines() {
    let content = "\n{ not json }\n{\"step\":\"a\",\"state\":\"running\"}\n{\"step\":\"a\",\"state\":\"success\"}\n";
    let state = fold_stream(content);
    assert_eq!(state.steps.len(), 1);
    assert_eq!(state.steps["a"].state, "success");
    assert!(!state.complete);
}

#[test]
fn tailing_reader_picks_up_appends_and_buffers_partial_lines() {
    let mut path = std::env::temp_dir();
    path.push(format!("bf-protocol-stream-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);

    let mut reader = StreamReader::open(path.clone());

    // Missing file yet: poll is a no-op, not an error.
    assert!(reader.poll().expect("poll missing file").steps.is_empty());

    // First append: one complete line + one partial (no trailing newline).
    {
        let mut f = std::fs::File::create(&path).expect("create");
        write!(
            f,
            "{{\"step\":\"load\",\"state\":\"running\"}}\n{{\"step\":\"load\",\"state\":\"succ"
        )
        .unwrap();
    }
    let state = reader.poll().expect("poll 1");
    assert_eq!(state.steps["load"].state, "running", "partial line held back");

    // Complete the partial line and add the terminal event.
    {
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).expect("append");
        write!(f, "ess\"}}\n{{\"event\":\"run_complete\",\"outcome\":\"success\"}}\n").unwrap();
    }
    let state = reader.poll().expect("poll 2");
    assert_eq!(state.steps["load"].state, "success", "buffered line completed");
    assert!(state.complete);
    assert_eq!(state.outcome, Some(Outcome::Success));

    let _ = std::fs::remove_file(&path);
}
