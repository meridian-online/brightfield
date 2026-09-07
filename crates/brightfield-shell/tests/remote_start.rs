//! Taking the remote start leaves the window drawing, and says how far it has
//! got.
//!
//! # Why there is a server in here at all
//!
//! The claim under test is about *time*: that the frames keep coming while the
//! bytes are still arriving. Nothing in the shipped set can show that without a
//! source that takes a measurable while to deliver, and the published lake is
//! not one this suite may reach — `test.yml` runs on every push and a test that
//! fetched someone else's server would be green or red for reasons that have
//! nothing to do with this repository (`crosswalk_chart.rs` makes the same
//! trade in the other direction, with `#[ignore]`).
//!
//! So the server is here, on `127.0.0.1:0`, and it **trickles**: it writes its
//! body in chunks with a pause between them, which is the only way to hold a
//! fetch open long enough for a test to drive frames through it. It is also how
//! the two halves of the readout are told apart — [`Stub::declaring_length`]
//! sends a `Content-Length` and [`Stub::silent_about_length`] does not, and the
//! card says different things over the two.
//!
//! # What it serves, and why it is a real Parquet
//!
//! An earlier form of this file served synthetic bytes and stopped at the
//! readout. That left the **tail** of the path unheld, and the tail was broken:
//! `open_remote_start` took an id and a source list separately, so these tests
//! handed the shipped start's id with a stub URL, the fetch went to localhost,
//! and one frame later `poll_fetch` composed the spec it read back off the id —
//! which names the published lake. Every run of this suite fetched production,
//! and nothing here could see it, because no test asserted what landed.
//!
//! So the stub serves a Parquet DuckDB can actually read, the spec handed in
//! names the stub, and each success case asserts the **landed document**: that
//! it composed, that its own spec now reads a local file rather than a URL, and
//! that the server was connected to exactly once — by the worker, and not again
//! by the engine.
//!
//! Nothing here resolves a name or leaves the loopback interface. The counter
//! on each stub is what proves it in both directions: the local starts open
//! with the server standing there and never connect to it, and a landed remote
//! document does not go back to it.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::spec_data_files;
use brightfield_shell::remote;
use brightfield_shell::starts;
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp, DOOR_ENTRY_PROMISE};

/// How many chunks a trickling stub cuts its body into.
///
/// Enough that a fetch spans several frames at [`TRICKLE`] apart, few enough
/// that the whole test is under a second.
const CHUNKS: usize = 6;

/// The pause between one chunk and the next.
const TRICKLE: Duration = Duration::from_millis(30);

/// How long a test will drive frames waiting for a fetch before calling it
/// hung. Generous: a loaded CI runner is slow and the cost of a wrong answer
/// here is a flake, not a missed defect.
const PATIENCE: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// The stub
// ---------------------------------------------------------------------------

/// A one-shot HTTP server on the loopback interface that answers slowly.
struct Stub {
    addr: SocketAddr,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl Stub {
    /// Serve `body` in [`CHUNKS`] pieces, declaring its total length.
    fn declaring_length(body: Vec<u8>) -> Self {
        Self::serve(body, true)
    }

    /// Serve `body` in [`CHUNKS`] pieces with no `Content-Length` — the case a
    /// readout has no denominator for, which is a real server behaviour and
    /// not a contrivance: a body streamed under `Connection: close` carries no
    /// length by definition.
    fn silent_about_length(body: Vec<u8>) -> Self {
        Self::serve(body, false)
    }

    /// A server that accepts and answers nothing this fetch can use — bound so
    /// the port is real, then never written to. Used for the URL of a source
    /// nothing serves: the listener is dropped immediately, so a connection to
    /// this address is refused rather than routed off the machine.
    fn nothing_listening() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("the bound address");
        drop(listener);
        addr
    }

    fn serve(body: Vec<u8>, declare_length: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("the bound address");
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                counter.fetch_add(1, Ordering::SeqCst);
                answer(stream, &body, declare_length);
            }
        });
        Self { addr, hits, stop }
    }

    /// The URL a spec would name this server's file under. The `.parquet` tail
    /// is not decoration: `brightfield_sql`'s DDL dispatch reads the
    /// extension, and `remote::Fetch` keeps it on the local name for that
    /// reason.
    fn url(&self) -> String {
        format!("http://{}/table.parquet", self.addr)
    }

    /// How many connections this server has accepted.
    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    /// Stop answering: the listener is closed, so a later connection to this
    /// address is refused.
    ///
    /// What it is for is the one claim a running server cannot carry — that the
    /// open left a **file** behind and not a live connection to this port.
    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // The accept loop only notices between connections, so knock once.
        std::net::TcpStream::connect(self.addr).ok();
    }
}

/// Read the request off `stream` and trickle `body` back.
fn answer(mut stream: TcpStream, body: &[u8], declare_length: bool) {
    // The request has to be drained before the response, or the client sees a
    // reset instead of an answer.
    let mut reader = BufReader::new(stream.try_clone().expect("clone the socket"));
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        if line == "\r\n" || line == "\n" {
            break;
        }
        line.clear();
    }
    let head = if declare_length {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
    } else {
        // No length, so the body ends where the connection does. `ureq` reads
        // to EOF here, which is what leaves the readout with a numerator and
        // no denominator.
        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
         Connection: close\r\n\r\n"
            .to_string()
    };
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let per = body.len().div_ceil(CHUNKS).max(1);
    for chunk in body.chunks(per) {
        if stream.write_all(chunk).is_err() {
            return;
        }
        if stream.flush().is_err() {
            return;
        }
        std::thread::sleep(TRICKLE);
    }
}

/// The distinct strings in `readouts`, in first-seen order.
///
/// A failure here would otherwise print one entry per frame — several hundred
/// of them, nearly all identical — and bury the one line a reader needs.
fn distinct(readouts: &[String]) -> Vec<&str> {
    let mut seen: Vec<&str> = Vec::new();
    for r in readouts {
        if !seen.contains(&r.as_str()) {
            seen.push(r);
        }
    }
    seen
}

/// A Parquet the engine can read, as bytes for a stub to serve.
///
/// Written through the **same DuckDB** the engine reads it back with —
/// `brightfield-engine`'s own dependency, on the same version line — for the
/// reason `data_file.rs` gives about its own fixture: a gate must not pass
/// because a second writer's idea of Parquet happens to agree with the
/// reader's.
///
/// Real Parquet rather than synthetic bytes because the assertion that matters
/// is about the **landed document**. Bytes that will not bind leave a test that
/// can only watch a meter, which is how the composition came to be aimed at the
/// published lake with nothing here able to say so.
fn parquet_body(rows: usize) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!(
        "bf-remote-fixture-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).expect("a place to write the fixture");
    let path = dir.join("table.parquet");
    let conn = duckdb::Connection::open_in_memory().expect("an in-memory DuckDB");
    conn.execute_batch(&format!(
        "COPY (SELECT i AS v, (i % 7) AS bucket FROM range({rows}) t(i)) TO '{}' (FORMAT PARQUET)",
        path.display()
    ))
    .expect("the COPY runs");
    let bytes = std::fs::read(&path).expect("the fixture is on disk");
    std::fs::remove_dir_all(&dir).ok();
    assert!(
        bytes.len() > 1024,
        "the fixture is {} bytes, too small for a readout to move over",
        bytes.len()
    );
    bytes
}

/// A chart spec whose one source is `url` — what a test hands
/// [`MeridianApp::open_remote_start`] in place of a shipped start's own spec.
///
/// It aggregates, so nothing here depends on the sampler, and it names the
/// source once, under `data:`, which is the only place a source is named.
fn spec_reading(url: &str) -> String {
    format!(
        "data:\n  t:\n    file: \"{url}\"\nplot:\n  - mark: rectY\n    \
         data: {{ from: t }}\n    x: {{ bin: v }}\n    y: {{ count: }}\nwidth: 640\nheight: 400\n"
    )
}

/// What the landed document is holding: the local files its spec now reads.
///
/// Read off the composed document's own spec through
/// [`brightfield_shell::pipeline::spec_data_files`], which skips a source with
/// a `://` in it — so a document that bound the network answers with an **empty
/// list**, and one that bound the fetched file answers with the path. That is
/// the two-sided reading; `is_empty()` is the failure.
fn local_files_of(app: &MeridianApp) -> Vec<std::path::PathBuf> {
    let live = app
        .chart_doc()
        .live_dashboard()
        .expect("the landed document carries its session");
    spec_data_files(live.spec(), None)
}

/// Assert the window landed on a document composed from `stub`'s bytes on
/// disk, and that the engine never went back to `stub` for them.
///
/// The three success cases share this because they are three readings of one
/// claim, and a claim written out three times is three places for one of them
/// to be quietly weakened.
fn assert_landed_locally(app: &MeridianApp, stub: &Stub) {
    assert!(
        !app.chart_doc().is_empty(),
        "the fetch finished and nothing was composed"
    );
    let files = local_files_of(app);
    assert_eq!(
        files.len(),
        1,
        "the landed document reads {files:?} — a document that bound the URL \
         instead of the fetched file has no local source at all"
    );
    assert!(
        files[0].is_file(),
        "the landed document reads {}, which is not on disk",
        files[0].display()
    );
    assert_eq!(
        stub.hits(),
        1,
        "the stub was connected to {} times: once is the worker, and any more \
         is the engine reading the URL rather than the file the worker wrote",
        stub.hits()
    );
}

// ---------------------------------------------------------------------------
// The window
// ---------------------------------------------------------------------------

/// A window under test, and one `egui::Context` for its whole life — the shape
/// `front_door.rs` uses, and for the reason it records there.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn open() -> Self {
        Self {
            app: MeridianApp::headless_with_layout(Boot::empty(), default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0)),
        }
    }

    /// Begin a remote start's fetch of `spec` — the arm
    /// `MeridianApp::open_start` takes for a start whose spec names a fetchable
    /// source, with the **spec** supplied rather than read off the id, so this
    /// suite aims the whole path at the loopback stub: what is fetched and what
    /// is composed both come off these bytes.
    fn take_remote(&mut self, id: &'static str, spec: &str) {
        let ctx = self.ctx.clone();
        assert!(
            self.app.open_remote_start(&ctx, id, spec),
            "the spec handed in names no fetchable source, so no fetch was \
             latched and this test is about nothing"
        );
    }

    /// Click the door's gallery card for `id`, where the last frame drew it —
    /// the real route into `MeridianApp::open_start`.
    ///
    /// **One frame, and no settling frame after it.** `open_start` runs in that
    /// frame's request drain, so what it decided is readable when this
    /// returns; a second frame would start the worker, which for a remote start
    /// is a connection this suite must not make.
    fn take_the_card(&mut self, id: &str) {
        let target = self
            .app
            .front_door_card_rect(id)
            .unwrap_or_else(|| panic!("the door drew no card for {id}"));
        let click = vec![
            egui::Event::PointerMoved(target.center()),
            egui::Event::PointerButton {
                pos: target.center(),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: target.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            events: click,
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    fn frame(&mut self) {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        let _ = self.ctx.run_ui(raw, |ui| self.app.draw(ui));
    }

    /// Drive frames until this window stops fetching, and say how many frames
    /// were drawn **while the fetch was outstanding**.
    ///
    /// The count excludes the frame the fetch landed on: that one drew a
    /// document, not a door, and counting it would let a window that froze for
    /// the whole download and then drew once report a two.
    fn drive_until_fetched(&mut self) -> usize {
        let deadline = std::time::Instant::now() + PATIENCE;
        let mut drawn = 0;
        while self.app.fetching_start().is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "the fetch did not finish inside {PATIENCE:?} — this test is \
                 hung, not slow"
            );
            self.frame();
            if self.app.fetching_start().is_some() {
                drawn += 1;
            }
        }
        drawn
    }
}

// ---------------------------------------------------------------------------
// AC1 — the window keeps drawing
// ---------------------------------------------------------------------------

/// Taking a start whose source is fetched leaves the window drawing: more than
/// one frame goes by with the fetch still outstanding, and the door is on every
/// one of them.
///
/// **The frame count is the whole assertion**, and it is what the synchronous
/// path could not produce: a fetch that happened inside the click returns with
/// the document already open, so the first frame after it draws a chart and
/// this loop counts zero.
///
/// Watched redden, one mutation: `MeridianApp::open_start`'s fetch arm replaced
/// by `crate::starts::load(id)` — the eager path — reports 0 frames drawn while
/// fetching, against the `> 1` below.
#[test]
fn the_window_keeps_drawing_while_a_remote_start_is_fetching() {
    let stub = Stub::declaring_length(parquet_body(40_000));
    let mut win = Window::open();
    win.frame();
    win.take_remote(starts::CROSSWALK_CHART, &spec_reading(&stub.url()));

    let mut door_drawn = 0;
    let deadline = std::time::Instant::now() + PATIENCE;
    while win.app.fetching_start().is_some() {
        assert!(
            std::time::Instant::now() < deadline,
            "the fetch did not finish inside {PATIENCE:?}"
        );
        win.frame();
        if win.app.fetching_start().is_some() {
            door_drawn += 1;
            assert!(
                win.app
                    .front_door_card_rect(starts::CROSSWALK_CHART)
                    .is_some(),
                "the window drew a frame with the fetch outstanding, but the \
                 door's card was not on it — the frame is not the door"
            );
        }
    }
    assert!(
        door_drawn > 1,
        "only {door_drawn} frame(s) were drawn while the fetch was \
         outstanding: the window is not drawing through the fetch"
    );
    // One more frame, so the landed bytes are composed rather than merely
    // received.
    win.frame();
    assert_landed_locally(&win.app, &stub);
}

// ---------------------------------------------------------------------------
// AC2 — the readout
// ---------------------------------------------------------------------------

/// While the fetch is outstanding the card's foot reads what has arrived
/// against what was promised.
///
/// Read off the frame — `front_door_card_foot` answers with the drawn galley's
/// own glyphs — rather than off the meter, because the claim is about what a
/// reader sees at the foot of the card they just clicked.
///
/// Watched redden, one mutation: `door_card`'s `foot` bound to
/// `DOOR_ENTRY_PROMISE` unconditionally. The card then reads the resting
/// promise while the fetch is outstanding, no frame carries ` of `, and both
/// assertions below fail.
#[test]
fn the_card_reads_what_has_arrived_against_the_declared_length() {
    let served = parquet_body(40_000);
    let total = served.len() as u64;
    let stub = Stub::declaring_length(served);
    let mut win = Window::open();
    win.frame();
    win.take_remote(starts::CROSSWALK_CHART, &spec_reading(&stub.url()));

    let mut readouts = Vec::new();
    let deadline = std::time::Instant::now() + PATIENCE;
    while win.app.fetching_start().is_some() {
        assert!(std::time::Instant::now() < deadline, "the fetch hung");
        win.frame();
        if let Some(foot) = win.app.front_door_card_foot(starts::CROSSWALK_CHART) {
            if win.app.fetching_start().is_some() {
                readouts.push(foot.to_string());
            }
        }
    }
    // The words the formatter itself produces for that total, taken through
    // its own public entry rather than written out here: a test that spelled
    // `512.0 KB` would go on passing if `readout` started counting in
    // something else.
    let at_zero = remote::readout(0, Some(total));
    let denominator = at_zero
        .rsplit_once(" of ")
        .expect("the two-number readout says `of`")
        .1
        .to_string();
    assert!(
        readouts.iter().any(|r| r.contains(" of ")),
        "no frame's card carried a received-against-declared readout: {:?}",
        distinct(&readouts)
    );
    assert!(
        readouts
            .iter()
            .all(|r| !r.contains(" of ") || r.ends_with(&denominator)),
        "a readout named a total that is not the {denominator} the server \
         declared: {:?}",
        distinct(&readouts)
    );
    assert!(
        readouts.iter().all(|r| r != DOOR_ENTRY_PROMISE),
        "a frame with the fetch outstanding still drew the resting promise: \
         {:?}",
        distinct(&readouts)
    );
    win.frame();
    assert_landed_locally(&win.app, &stub);
}

/// A server that declares no length leaves the readout with the count alone —
/// no total, and no invented one.
///
/// Watched redden, one mutation: `remote::readout` returning the two-number
/// form with `received` as the denominator when `declared` is `None`. The
/// readout then carries ` of `, against the assertion below.
#[test]
fn the_card_reads_the_count_alone_when_no_length_was_declared() {
    let stub = Stub::silent_about_length(parquet_body(40_000));
    let mut win = Window::open();
    win.frame();
    win.take_remote(starts::CROSSWALK_CHART, &spec_reading(&stub.url()));

    let mut readouts = Vec::new();
    let deadline = std::time::Instant::now() + PATIENCE;
    while win.app.fetching_start().is_some() {
        assert!(std::time::Instant::now() < deadline, "the fetch hung");
        win.frame();
        if win.app.fetching_start().is_some() {
            if let Some(foot) = win.app.front_door_card_foot(starts::CROSSWALK_CHART) {
                readouts.push(foot.to_string());
            }
        }
    }
    assert!(
        !readouts.is_empty(),
        "no frame drew the card while the fetch was outstanding"
    );
    assert!(
        readouts.iter().all(|r| !r.contains(" of ")),
        "the readout named a total the server never declared: {:?}",
        distinct(&readouts)
    );
    assert!(
        readouts.iter().any(|r| r != DOOR_ENTRY_PROMISE),
        "no frame's card said anything about the fetch: {:?}",
        distinct(&readouts)
    );
    win.frame();
    assert_landed_locally(&win.app, &stub);
}

// ---------------------------------------------------------------------------
// AC3 — a fetch that fails
// ---------------------------------------------------------------------------

/// A fetch that cannot reach what it was told to read raises a banner naming
/// the network and the URL, over a window that stays up on the door.
///
/// The same two things the eager bind's `EngineError::RemoteSourceFailed` names
/// — `crosswalk_chart.rs` asserts them of the engine's message, and this
/// asserts them of the fetch's, because the fetch is what a user meets now.
///
/// Watched redden, one mutation: `remote::fetch_one`'s connect error mapped to
/// `format!("{e}")` instead of the sentence naming the URL — the message then
/// carries neither the URL nor the word `network`.
#[test]
fn a_fetch_that_fails_raises_a_banner_naming_the_network_and_the_url() {
    let url = format!("http://{}/table.parquet", Stub::nothing_listening());
    let mut win = Window::open();
    win.frame();
    win.take_remote(starts::CROSSWALK_CHART, &spec_reading(&url));
    win.drive_until_fetched();
    // One more frame, so the refusal is drawn rather than merely raised.
    win.frame();

    let raised: Vec<String> = win
        .app
        .notifications()
        .iter()
        .map(|n| format!("{} {}", n.title, n.body.clone().unwrap_or_default()))
        .collect();
    assert_eq!(
        raised.len(),
        1,
        "a failed fetch raised {} banner(s), not one: {raised:?}",
        raised.len()
    );
    let said = raised[0].clone();
    assert!(
        said.contains("network"),
        "the banner does not name the network: {said}"
    );
    assert!(
        said.contains(&url),
        "the banner does not name what it could not reach: {said}"
    );
    assert!(
        win.app.front_door_is_live(),
        "the window did not stay on the door through the refusal"
    );
    assert!(
        win.app
            .front_door_card_rect(starts::CROSSWALK_CHART)
            .is_some(),
        "the door drew no card after the refusal — the window did not stay up"
    );
}

// ---------------------------------------------------------------------------
// AC4 — the local starts are untouched
// ---------------------------------------------------------------------------

/// No start but the remote one has anything to fetch, and the one that does
/// names exactly the URL its shipped spec names.
///
/// The decision `MeridianApp::open_start` makes, read where it is made. It is
/// pure — no engine, no window, no connection — because the input is the
/// start's own `spec:` bytes.
///
/// Watched redden, one mutation: `remote::https_sources` matching on
/// `path.starts_with("http")` rather than [`remote::HTTPS`] — harmless here;
/// and the real one, `https_sources` returning `Ok(Vec::new())` unconditionally,
/// which empties the crosswalk chart's list and fails the last assertion.
#[test]
fn only_the_remote_start_has_anything_to_fetch() {
    for start in starts::STARTS {
        let sources = start.spec.map_or_else(Vec::new, |spec| {
            remote::remote_sources(spec).expect("a shipped spec parses")
        });
        assert_eq!(
            start.remote,
            !sources.is_empty(),
            "{}'s remote flag ({}) and its fetched sources ({sources:?}) \
             disagree — the flag is what the hermetic gates read to decide \
             which starts they may open",
            start.id,
            start.remote
        );
    }
    let chart = starts::find(starts::CROSSWALK_CHART).expect("the shipped set has it");
    assert_eq!(
        remote::remote_sources(chart.spec.expect("it carries a spec")).expect("it parses"),
        vec!["https://openlake.meridian.online/edgar_gleif.parquet".to_string()],
        "the crosswalk chart fetches something other than the URL its shipped \
         spec reads"
    );
}

/// Clicking the remote start's real card records a fetch of the URL its
/// shipped spec names, and leaves the window on the door.
///
/// **The join this file could not otherwise reach.** The tests above hand
/// `open_remote_start` a stub's URL, which leaves open whether the card click
/// gets there and which URL it would carry. This drives the
/// real `MeridianApp::open_start` through the real gallery card and reads back
/// what it decided — and it can, without a connection, because the click
/// *latches* the fetch and `draw` starts the worker on the next frame. No
/// second frame is drawn, so no socket is opened; dropping the window drops a
/// `PendingStart` with no worker behind it.
///
/// Watched redden, two mutations. `open_start` passing `Vec::new()` in place of
/// the sources it resolved: the second assertion reads an empty list. And
/// `open_start`'s fetch arm deleted, so a remote start falls through to the
/// eager `starts::load`: `fetching_start()` is then `None` and the first
/// assertion fails — that one does reach the network while it is applied, which
/// is why it is not the mutation kept in the file.
#[test]
fn taking_the_remote_card_records_the_url_its_spec_names() {
    let mut win = Window::open();
    win.frame();
    win.take_the_card(starts::CROSSWALK_CHART);
    assert_eq!(
        win.app.fetching_start(),
        Some(starts::CROSSWALK_CHART),
        "the card click did not start a fetch — the window either opened the \
         start synchronously or did nothing"
    );
    assert_eq!(
        win.app.fetching_sources(),
        ["https://openlake.meridian.online/edgar_gleif.parquet".to_string()],
        "the click fetches something other than what the shipped spec reads"
    );
    assert!(
        win.app.front_door_is_live(),
        "the window left the door while the fetch was still only latched"
    );
    assert_eq!(
        win.app.front_door_card_foot(starts::CROSSWALK_CHART),
        Some(DOOR_ENTRY_PROMISE),
        "the frame the click landed on drew its card before the click was \
         resolved, so its foot is still the resting promise"
    );
}

/// Opening a start that declares no network leaves the fetch path unused,
/// with a server standing on localhost that never hears from it.
///
/// The server is the point: an assertion that `fetching_start()` is `None`
/// would also pass over a start that fetched something *else*. A counter on a
/// socket answers the question a flag cannot.
///
/// The **click** is the other half. `starts::CROSSWALK` is the local start the
/// door offers, so this drives the real `MeridianApp::open_start` through the
/// real card rather than calling the fetch entry point directly — which is the
/// arm the mutation below moves.
///
/// Watched redden, one mutation: `open_start`'s `Ok(sources) if
/// !sources.is_empty()` guard relaxed to `Ok(sources)`, which sends a start
/// with an empty source list down the fetch arm. The crosswalk then does not
/// open and `fetching_start()` is `Some`, failing the first two assertions.
#[test]
fn a_start_that_declares_no_network_never_enters_the_fetch_path() {
    let stub = Stub::declaring_length(parquet_body(1_000));
    let mut win = Window::open();
    win.frame();
    win.take_the_card(starts::CROSSWALK);
    assert!(
        win.app.fetching_start().is_none(),
        "{} left a fetch outstanding",
        starts::CROSSWALK
    );
    assert!(
        !win.app.front_door_is_live(),
        "{} did not open — the window is still on the door, so this test never \
         reached the state it is about",
        starts::CROSSWALK
    );

    // …and the rest of the local set through `starts::load`, which is the
    // loader `open_start`'s local arm calls: a start that grew a fetched source
    // and forgot to declare it reaches for a socket here.
    for start in starts::STARTS.iter().filter(|s| !s.remote) {
        let boot = Boot::start(start.id, Flow::Vertical)
            .unwrap_or_else(|e| panic!("{} no longer opens: {e}", start.id));
        assert!(
            boot.has_chart() || !boot.protocol.graph_full.nodes.is_empty(),
            "{} opened onto nothing",
            start.id
        );
    }
    assert_eq!(
        stub.hits(),
        0,
        "a start that declares no network connected to the stub server {} \
         time(s)",
        stub.hits()
    );
}

// ---------------------------------------------------------------------------
// What the fetch hands the engine
// ---------------------------------------------------------------------------

/// The fetch writes the bytes it was served, under a name the engine's DDL
/// dispatch can read, and `repointed` moves the source that named the URL onto
/// that file — leaving anything else that happens to spell the same URL alone.
///
/// **The unit under this claim, not the claim itself.** What a reader cares
/// about is that the *landed document* reads a file, and that is
/// `assert_landed_locally`, which the three success cases above make through
/// `starts::compose`. This is the layer below: it is here because the fetch's
/// own contract — how many bytes, under what name, and what happens when the
/// guard is dropped — has nowhere else to be said, and because the substitution
/// this rejects is a plausible shortcut somebody will reach for.
///
/// The spec below names the URL in a comment as well as under `data:`, which is
/// a case rather than a description of the shipped file: `examples/remote/
/// edgar-gleif-crosswalk.yaml` names it once, under `data:`. Only the parse can
/// say which occurrence of a string is a source, and this is what says so.
///
/// Watched redden, one mutation: `remote::repointed` returning the parse
/// unchanged — the source then still reads the stub's URL and the assertion on
/// the rewritten source fails.
#[test]
fn the_fetch_writes_what_it_was_served_and_repoints_only_the_source() {
    let served = parquet_body(1_000);
    let length = served.len() as u64;
    let stub = Stub::declaring_length(served);
    let url = stub.url();
    let spec = format!(
        "# a comment naming {url}\ndata:\n  t:\n    file: \"{url}\"\nplot:\n  \
         - mark: dot\n    data: {{ from: t }}\n    x: v\n    y: bucket\n"
    );
    let fetched = remote::Fetch::begin(vec![url.clone()], || {})
        .wait()
        .expect("the stub answers");
    let local = fetched
        .local_path(&url)
        .expect("the fetch wrote a file for the URL it was given")
        .to_path_buf();
    assert_eq!(
        std::fs::metadata(&local)
            .expect("the fetched file is on disk")
            .len(),
        length,
        "the fetch wrote a file of the wrong length"
    );
    assert_eq!(
        local.extension().and_then(std::ffi::OsStr::to_str),
        Some("parquet"),
        "the local name lost the extension the engine's DDL dispatch reads"
    );

    let parsed = remote::repointed(&spec, &fetched).expect("the spec parses");
    let source = parsed.spec.data.get("t").expect("the declared source");
    match &source.kind {
        brightfield_spec::ast::DataSourceKind::File(path) => assert_eq!(
            path,
            &local.display().to_string(),
            "the source still reads the network"
        ),
        other => panic!("the source stopped being a file: {other:?}"),
    }

    // And the directory goes when the guard does — which is what lets the
    // window hold one per document without leaving a Parquet behind every time
    // someone opens something else.
    let dir = fetched.dir().to_path_buf();
    assert!(dir.is_dir(), "the fetch left no directory behind it");
    drop(fetched);
    assert!(
        !dir.exists(),
        "dropping the fetch left {} on disk",
        dir.display()
    );
}

// ---------------------------------------------------------------------------
// The latch belongs to the document that was on its way in
// ---------------------------------------------------------------------------

/// A fetch the reader gave up on does not arrive later and take the window.
///
/// **The defect, in the order it happens.** Click the remote card; the fetch is
/// latched and the door keeps drawing. Change your mind and take a local start;
/// that document opens and you begin reading it. The abandoned fetch lands, and
/// the window silently swaps to the chart you gave up on — over a document you
/// had already started on, with the layout recording the wrong thing to
/// restore.
///
/// It was reachable because the latch was cleared at a list of call sites —
/// `poll_fetch` and `open_home` — and `land_start` was not one of them. So it is
/// cleared in `documents_changed` instead, which is where the three openers
/// this shell has — `land_start`, `adopt_boot` and `open_home` — meet. This
/// test walks the front door's own card; the file picker, a dropped file and
/// the palette reach `adopt_boot` and are closed by the same line without a
/// test each.
///
/// The stub is asked for its hit count afterwards for the second half of the
/// claim: the abandoned worker is not cancelled mid-flight — nothing here can
/// interrupt a socket read — it runs to completion and its answer is dropped,
/// files and all.
///
/// Watched redden, one mutation: `documents_changed`'s `self.fetching = None`
/// removed, which is the whole of the fix. It fails at the first assertion
/// after the card is taken — *opening a document left the abandoned fetch
/// latched* — which is the right place for it to fail: the window is wrong the
/// moment the local start lands, and everything after that is the consequence
/// rather than the defect.
#[test]
fn a_fetch_the_reader_gave_up_on_does_not_arrive_and_take_the_window() {
    let stub = Stub::declaring_length(parquet_body(40_000));
    let mut win = Window::open();
    win.frame();
    win.take_remote(starts::CROSSWALK_CHART, &spec_reading(&stub.url()));
    win.frame();
    assert_eq!(
        win.app.fetching_start(),
        Some(starts::CROSSWALK_CHART),
        "the fetch is not outstanding, so this test never reached the state it \
         is about"
    );

    // …and the reader changes their mind, through the door's own card.
    win.take_the_card(starts::CROSSWALK);
    assert!(
        win.app.fetching_start().is_none(),
        "opening a document left the abandoned fetch latched"
    );
    let opened = win
        .app
        .layout()
        .opened
        .clone()
        .expect("the local start recorded itself");
    assert_eq!(
        opened,
        starts::CROSSWALK,
        "the local start did not open, so this test never reached the state it \
         is about"
    );

    // Long enough for the abandoned fetch to have finished several times over.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        win.frame();
    }
    assert!(
        win.app.chart_doc().is_empty(),
        "a fetch the reader gave up on landed and put a chart on a window \
         holding the Protocol they had moved to"
    );
    assert_eq!(
        win.app.layout().opened.as_deref(),
        Some(starts::CROSSWALK),
        "the abandoned fetch rewrote what the next launch restores"
    );
    assert_eq!(
        stub.hits(),
        1,
        "the abandoned worker was started {} times",
        stub.hits()
    );
}

// ---------------------------------------------------------------------------
// The fetched files outlive the open
// ---------------------------------------------------------------------------

/// A re-query after the document is open still answers, with the stub gone.
///
/// The engine binds a **view** over the fetched path rather than copying the
/// rows in, so every query re-reads the file: a guard dropped at the end of the
/// open would delete the Parquet under a session that is about to be brushed,
/// resized or re-composited. Nothing in the success cases above could see that
/// — they read the document on the frame it landed, while the file is still
/// there whether anything owns it or not.
///
/// The stub is **stopped** before the re-query, which is the other half: a
/// document that had bound the URL instead of the file would fail here for a
/// reason that looks the same and is not, so the local-file assertion runs
/// first.
///
/// Watched redden, one mutation: `land_start`'s `self.remote_files =
/// chart.fetched;` replaced with `= None;` and the guard dropped. The fetched
/// directory is removed at the end of the open, and this fails inside
/// `assert_landed_locally` — *the landed document reads
/// …/0-table.parquet, which is not on disk* — before the re-query is reached,
/// which is the honest order: a file that is gone cannot be re-read.
#[test]
fn the_fetched_file_outlives_the_open_and_a_re_query_still_answers() {
    let stub = Stub::declaring_length(parquet_body(40_000));
    let mut win = Window::open();
    win.frame();
    win.take_remote(starts::CROSSWALK_CHART, &spec_reading(&stub.url()));
    win.drive_until_fetched();
    win.frame();
    assert_landed_locally(&win.app, &stub);

    let file = local_files_of(&win.app)[0].clone();
    stub.stop();
    assert!(
        file.is_file(),
        "the open deleted {} — the session is reading a file that is gone",
        file.display()
    );

    let declared = {
        let doc = win.app.chart_doc();
        egui::vec2(doc.composed.width as f32, doc.composed.height as f32)
    };
    assert!(
        win.app.chart_doc_mut().reflow_to(declared * 0.5),
        "the document could not re-composite after the open: the engine \
         re-reads the fetched file on every query, and it is no longer there"
    );
    assert!(
        !win.app.chart_doc().is_empty(),
        "the re-composite emptied the document"
    );
}
