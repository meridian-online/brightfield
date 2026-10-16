//! A typed statement is read off the UI thread, once the typing pauses, and a
//! read the text has moved past is stopped.
//!
//! arc is the `brightfield-shell` binary cargo built for this suite, as in
//! `tests/sql_through_arc.rs`, and the rows are the housing start's own file.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use brightfield_engine::{FileReader, PreviewOp};
use brightfield_shell::data_grid::{use_own_preview_gate_on_this_thread, PREVIEW_ROWS};
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
        chain: Vec::new(),
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
        answered(FILTER).filter_page.is_none(),
        "no order, no page of its own"
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

#[test]
fn a_refused_text_keeps_the_last_good_rows_and_says_why() {
    let mut r = reads(TYPED_PAUSE);
    answer(&mut r, FILTER);
    answer(&mut r, "select * from nowhere");
    assert_eq!(r.shown().unwrap().text, FILTER);
    assert!(r.current().is_none());
    assert!(r.refusal().unwrap().contains("nowhere"));
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
    let mut source = housing_source();
    source.chain.push((
        "t".into(),
        PreviewOp::Statement(
            "select house_age, count(*) from california_housing group by house_age".into(),
        ),
    ));
    let rx = spawn_row_count(source, || {});
    assert_eq!(rx.recv_timeout(Duration::from_secs(60)).unwrap(), Ok(52));
}
