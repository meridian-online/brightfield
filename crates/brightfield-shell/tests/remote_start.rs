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
//! Nothing here resolves a name or leaves the loopback interface. The counter
//! on each stub is what proves it in the other direction too: the local starts
//! open with the server standing there and never connect to it.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
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
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                counter.fetch_add(1, Ordering::SeqCst);
                answer(stream, &body, declare_length);
            }
        });
        Self { addr, hits }
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

/// A body big enough that the readout has something to count, and made of
/// bytes rather than of a Parquet: what is under test here is the fetch, and
/// nothing in these tests hands the result to the engine.
fn body(bytes: usize) -> Vec<u8> {
    (0..bytes).map(|i| (i % 251) as u8).collect()
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

    /// Begin a remote start's fetch against `sources` — the arm
    /// `MeridianApp::open_start` takes for a start whose spec names an
    /// `https://` source, with the sources supplied rather than read so this
    /// suite can aim it at the loopback stub instead of the published lake.
    fn take_remote(&mut self, id: &'static str, sources: Vec<String>) {
        let ctx = self.ctx.clone();
        self.app.open_remote_start(&ctx, id, sources);
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
    let stub = Stub::declaring_length(body(512 * 1024));
    let mut win = Window::open();
    win.frame();
    win.take_remote(starts::CROSSWALK_CHART, vec![stub.url()]);

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
    assert_eq!(stub.hits(), 1, "the fetch made exactly one request");
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
    const TOTAL: usize = 512 * 1024;
    let stub = Stub::declaring_length(body(TOTAL));
    let mut win = Window::open();
    win.frame();
    win.take_remote(starts::CROSSWALK_CHART, vec![stub.url()]);

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
    let at_zero = remote::readout(0, Some(TOTAL as u64));
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
}

/// A server that declares no length leaves the readout with the count alone —
/// no total, and no invented one.
///
/// Watched redden, one mutation: `remote::readout` returning the two-number
/// form with `received` as the denominator when `declared` is `None`. The
/// readout then carries ` of `, against the assertion below.
#[test]
fn the_card_reads_the_count_alone_when_no_length_was_declared() {
    let stub = Stub::silent_about_length(body(512 * 1024));
    let mut win = Window::open();
    win.frame();
    win.take_remote(starts::CROSSWALK_CHART, vec![stub.url()]);

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
    win.take_remote(starts::CROSSWALK_CHART, vec![url.clone()]);
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
        let sources = starts::network_sources(start.id).expect("every shipped spec parses");
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
    assert_eq!(
        starts::network_sources(starts::CROSSWALK_CHART).expect("the shipped spec parses"),
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
    let stub = Stub::declaring_length(body(1024));
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

/// The spec the engine is handed names the local file, and names it only where
/// the fetch actually wrote one — the comments in the shipped spec that quote
/// the same URL are left alone.
///
/// This is what makes the composition a local one. The crosswalk spec names
/// `https://openlake.meridian.online/edgar_gleif.parquet` in its header comment
/// as well as under `data:`, so a rewrite over the text would have moved both
/// and a rewrite over the parse moves one.
///
/// Watched redden, one mutation: `remote::repointed` returning the parse
/// unchanged — the source then still reads the stub's URL and the first
/// assertion fails.
#[test]
fn the_composed_spec_reads_the_local_file_and_the_comments_are_untouched() {
    let stub = Stub::declaring_length(body(4096));
    let url = stub.url();
    let spec = format!(
        "# a comment naming {url}\ndata:\n  t:\n    file: \"{url}\"\nplot:\n  \
         - mark: dot\n    data: {{ from: t }}\n    x: a\n    y: b\n"
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
        4096,
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
