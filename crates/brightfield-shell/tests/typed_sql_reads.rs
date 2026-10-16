//! A typed statement is read off the UI thread, once the typing pauses, and a
//! read the text has moved past is stopped.
//!
//! arc is the `brightfield-shell` binary cargo built for this suite, as in
//! `tests/sql_through_arc.rs`, and the rows are the housing start's own file.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use brightfield_engine::{FileReader, PreviewOp};
use brightfield_shell::data_grid::{use_own_preview_gate_on_this_thread, PREVIEW_ROWS};
use brightfield_shell::generated::{GeneratedStep, KnownOperation, StepEffect};
use brightfield_shell::operation::Recognised;
use brightfield_shell::run::Runner;
use brightfield_shell::typed_sql::{
    spawn_row_count, TypedAnswer, TypedReads, TypedSource, TYPED_PAUSE,
};

const FILTER: &str = "select * from california_housing where house_age > 28";
const SLOW: &str = "select sum(range) from range(1000000000000000)";

fn housing_source() -> TypedSource {
    use_own_preview_gate_on_this_thread();
    TypedSource {
        data: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("assets/starts/california_housing.parquet"),
        reader: FileReader::Parquet,
        root_table: "california_housing".into(),
        steps: Vec::new(),
        order: None,
        runner: Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))),
    }
}

fn reads(pause: Duration) -> TypedReads {
    TypedReads::new(housing_source(), pause, || {})
}

fn poll_until(r: &mut TypedReads, done: impl Fn(&TypedReads) -> bool, within: Duration) {
    let until = Instant::now() + within;
    while !done(r) {
        assert!(Instant::now() < until, "no answer within {within:?}");
        r.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Edit to `text`, read it at once, and wait for its answer or its refusal.
fn answer(r: &mut TypedReads, text: &str) {
    r.edit(text, Instant::now());
    r.read_now();
    poll_until(
        r,
        |r| r.current().is_some() || r.refusal().is_some(),
        Duration::from_secs(60),
    );
}

fn answered(text: &str) -> TypedAnswer {
    let mut r = reads(TYPED_PAUSE);
    answer(&mut r, text);
    r.current()
        .unwrap_or_else(|| panic!("refused: {:?}", r.refusal()))
        .clone()
}

#[test]
fn a_read_starts_only_after_the_pause() {
    let (mut r, t0) = (reads(TYPED_PAUSE), Instant::now());
    r.edit(FILTER, t0);
    r.tick(t0 + Duration::from_millis(299));
    assert_eq!(r.started(), 0);
    r.tick(t0 + TYPED_PAUSE);
    assert_eq!(r.started(), 1);
    r.stop();
}

#[test]
fn the_time_left_in_the_pause_is_what_a_frame_waits_for() {
    let (mut r, t0) = (reads(TYPED_PAUSE), Instant::now());
    assert_eq!(r.pause_left(t0), None, "nothing typed, nothing to wait for");
    r.edit(FILTER, t0);
    assert_eq!(
        r.pause_left(t0 + Duration::from_millis(100)),
        Some(Duration::from_millis(200))
    );
    assert_eq!(r.pause_left(t0 + TYPED_PAUSE * 2), Some(Duration::ZERO));
    r.tick(t0 + TYPED_PAUSE);
    assert_eq!(r.pause_left(t0 + TYPED_PAUSE), None, "the read has started");
    r.stop();
}

#[test]
fn an_answer_carries_the_page_the_reading_and_arcs_reading() {
    let a = answered(FILTER);
    assert_eq!(a.page.rows.len() as u64, PREVIEW_ROWS);
    assert_eq!(a.reading.tables, vec!["california_housing".to_string()]);
    assert_eq!(
        a.recognised,
        Recognised::FilterRows {
            on: "california_housing".into(),
            condition: "house_age > 28".into()
        }
    );
    assert_eq!(a.filter_rows, Some(8_831));
}

#[test]
fn a_filter_of_the_root_table_is_paged_under_the_pending_order() {
    assert!(
        answered(FILTER).filter_page.is_some(),
        "no order still reads the filter's own page"
    );
    let mut source = housing_source();
    source.order = Some("house_age desc".into());
    let mut r = TypedReads::new(source, TYPED_PAUSE, || {});
    answer(&mut r, FILTER);
    let a = r.current().expect("the filter was answered");
    let page = a.filter_page.as_ref().expect("a page under the order");
    let col = page
        .columns
        .iter()
        .position(|c| c.name == "house_age")
        .expect("a house_age column");
    let first: f64 = page.rows[0][col].full().parse().expect("a number");
    assert_eq!(first, 52.0, "the oldest houses first");
    assert_eq!(a.filter_rows, Some(8_831));
}

/// With no order pending, the filter's preview page is the page the applied
/// view reads: the first rows in `ORDER BY ALL` order, not in file order.
#[test]
fn an_unordered_filter_previews_the_page_the_applied_view_reads() {
    use_own_preview_gate_on_this_thread();
    let dir = std::env::temp_dir().join(format!("bf_unsorted_filter_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let data = dir.join("tally.csv");
    let body: String = (1..=1000).rev().map(|n| format!("{n}\n")).collect();
    std::fs::write(&data, format!("v\n{body}")).expect("the file");
    let source = TypedSource {
        data,
        reader: FileReader::Csv,
        root_table: "tally".into(),
        steps: Vec::new(),
        order: None,
        runner: Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))),
    };
    let mut r = TypedReads::new(source, TYPED_PAUSE, || {});
    answer(&mut r, "select * from tally where v > 100");
    let a = r.current().expect("the filter was answered").clone();
    let _ = std::fs::remove_dir_all(&dir);
    let page = a
        .filter_page
        .as_ref()
        .expect("a page of its own for a filter");
    let col = page
        .columns
        .iter()
        .position(|c| c.name == "v")
        .expect("a v column");
    let got: Vec<String> = page
        .rows
        .iter()
        .map(|row| row[col].full().to_string())
        .collect();
    let want: Vec<String> = (101..)
        .take(got.len())
        .map(|n: u32| n.to_string())
        .collect();
    assert!(!got.is_empty());
    assert_eq!(
        got, want,
        "the smallest matching rows first, as the applied view reads them"
    );
}

#[test]
fn a_refused_text_keeps_the_last_good_rows_and_says_why() {
    let mut r = reads(TYPED_PAUSE);
    answer(&mut r, FILTER);
    answer(&mut r, "select * from nowhere");
    assert_eq!(r.shown().unwrap().text, FILTER);
    assert!(r.current().is_none());
    assert!(r.refusal().unwrap().contains("nowhere"));
}

/// A refusal is not kept once other text has answered, so a refusal that
/// may pass on a second try, such as the deadline, is read again when the
/// text comes back to it.
#[test]
fn text_refused_before_is_read_again_after_other_text_answers() {
    let mut r = reads(TYPED_PAUSE);
    answer(&mut r, "select * from nowhere");
    assert!(r.refusal().is_some());
    answer(&mut r, FILTER);
    let now = Instant::now();
    r.edit("select * from nowhere", now);
    assert!(r.refusal().is_none());
    assert_eq!(r.pause_left(now), Some(TYPED_PAUSE));
}

#[test]
fn a_superseded_read_is_stopped() {
    let mut r = reads(TYPED_PAUSE);
    r.edit(SLOW, Instant::now());
    r.read_now();
    std::thread::sleep(Duration::from_millis(200));
    r.edit(FILTER, Instant::now());
    r.read_now();
    assert_eq!(r.superseded(), 1);
    poll_until(&mut r, |r| r.current().is_some(), Duration::from_secs(20));
    assert_eq!(r.shown().unwrap().text, FILTER);
}

#[test]
fn an_edit_stops_the_read_of_the_text_it_left() {
    let mut r = reads(TYPED_PAUSE);
    answer(&mut r, FILTER);
    r.edit(SLOW, Instant::now());
    r.read_now();
    assert!(r.reading());
    r.edit(FILTER, Instant::now());
    assert_eq!(r.superseded(), 1);
    assert!(!r.reading());
    r.tick(Instant::now() + TYPED_PAUSE);
    assert!(!r.reading(), "the answered text is not read again");
    assert_eq!(r.current().unwrap().text, FILTER);
}

#[test]
fn the_ui_thread_is_not_held_by_a_slow_statement() {
    // No pause, so each frame's tick starts a read of the text its edit
    // left, and each edit stops the read before it.
    let mut r = reads(Duration::ZERO);
    r.edit(SLOW, Instant::now());
    r.read_now();
    let mut longest = Duration::ZERO;
    for i in 0..20 {
        let at = Instant::now();
        r.edit(&format!("{SLOW} -- {i}"), at);
        r.tick(Instant::now());
        r.poll();
        longest = longest.max(at.elapsed());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(r.reading());
    assert_eq!(r.superseded(), 20);
    assert!(longest < Duration::from_millis(50), "{longest:?}");
    r.stop();
}

#[test]
fn stopping_drops_a_late_answer() {
    let mut r = reads(TYPED_PAUSE);
    r.edit(FILTER, Instant::now());
    r.read_now();
    r.stop();
    std::thread::sleep(Duration::from_secs(2));
    r.poll();
    assert!(r.shown().is_none() && !r.reading());
}

#[test]
fn dropping_the_reader_stops_its_read() {
    let mut slow = reads(TYPED_PAUSE);
    slow.edit(SLOW, Instant::now());
    slow.read_now();
    std::thread::sleep(Duration::from_millis(200));
    drop(slow);
    // The next reader on this thread reads behind the same gate, so its
    // answer comes only once the dropped reader's read has let go of it.
    let at = Instant::now();
    let a = answered(FILTER);
    assert_eq!(a.text, FILTER);
    assert!(at.elapsed() < Duration::from_secs(20), "{:?}", at.elapsed());
}

#[test]
fn a_kept_steps_rows_are_counted_off_the_thread() {
    let chain = vec![(
        "t".into(),
        PreviewOp::Statement(
            "select house_age, count(*) from california_housing group by house_age".into(),
        ),
    )];
    let rx = spawn_row_count(housing_source(), chain, || {});
    assert_eq!(rx.recv_timeout(Duration::from_secs(60)).unwrap(), Ok(52));
}

/// A SQL step of the Protocol reading `reads`.
fn sql_step(name: &str, statement: &str, reads: &[&str]) -> GeneratedStep {
    GeneratedStep {
        name: name.into(),
        effect: StepEffect::Sql,
        on: reads.first().map(|r| (*r).to_string()).unwrap_or_default(),
        reads: reads.iter().map(|r| (*r).to_string()).collect(),
        argument: statement.into(),
    }
}

/// A read runs only the steps its statement reads and theirs, so a step it
/// has no use for, one that fails or one that would run past the deadline,
/// neither refuses nor holds up its preview.
#[test]
fn a_statement_over_the_loaded_table_previews_past_a_failing_step() {
    let mut source = housing_source();
    source.steps = vec![
        sql_step("broken", "select * from no_such_table", &["no_such_table"]),
        sql_step("slow", SLOW, &[]),
        GeneratedStep {
            name: "old".into(),
            effect: StepEffect::Operation(KnownOperation::FilterRows),
            on: "california_housing".into(),
            reads: vec!["california_housing".into()],
            argument: "house_age > 28".into(),
        },
        sql_step(
            "old_by_age",
            "select house_age, count(*) as n from old group by house_age",
            &["old"],
        ),
    ];
    let read = |text: &str| {
        let mut r = TypedReads::new(source.clone(), TYPED_PAUSE, || {});
        let at = Instant::now();
        answer(&mut r, text);
        assert!(
            at.elapsed() < Duration::from_secs(20),
            "{text:?} waited on a step it does not read: {:?}",
            at.elapsed()
        );
        (r.current().cloned(), r.refusal().map(str::to_owned))
    };
    let (over_root, refused) = read(FILTER);
    assert!(over_root.is_some(), "refused: {refused:?}");
    let (over_steps, refused) = read("select * from old_by_age");
    let over_steps = over_steps.unwrap_or_else(|| panic!("refused: {refused:?}"));
    assert_eq!(
        over_steps.page.rows.len(),
        24,
        "the step it reads is made from the step that step reads"
    );
    let (_, refused) = read("select * from broken");
    assert!(
        refused.is_some_and(|r| r.contains("no_such_table")),
        "a statement reading the failing step is refused for it"
    );
}

/// A statement DuckDB cannot parse is refused in DuckDB's words, with no
/// step built for it to read.
#[test]
fn a_statement_that_does_not_parse_is_refused_without_building_a_step() {
    let mut source = housing_source();
    source.steps = vec![sql_step(
        "broken",
        "select * from no_such_table",
        &["no_such_table"],
    )];
    let mut r = TypedReads::new(source, TYPED_PAUSE, || {});
    answer(&mut r, "select * from california_housing where");
    let refused = r.refusal().expect("refused").to_owned();
    assert!(
        refused.contains("syntax error"),
        "DuckDB's own words: {refused}"
    );
    assert!(!refused.contains("no_such_table"), "{refused}");
}
