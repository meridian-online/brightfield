//! Open in Brightfield: the `brightfield://open` link, the Protocol archive it names, and trust.
//!
//! The link is checked before anything is fetched; an archive is checked entry by entry before
//! anything is written, and its bytes are counted as they are written; a Protocol Brightfield did
//! not make opens restricted, drawing the results it carries with DuckDB confined to them, until
//! the analyst trusts it. Trust is a record in the store, never read from the folder.
//!
//! Every window is headless, with the `brightfield-shell` binary cargo built as its runner, and
//! the config folder (which holds the trust store), the Protocols folder and arc's folders point
//! at a scratch folder of this file's, so nothing is written under the home directory. Archives
//! are served from a stand-in server on this Mac, so the download runs through the network path
//! with no network.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use brightfield_shell::design::Mode;
use brightfield_shell::open_queue::OpenRequest;
use brightfield_shell::protocol_archive::{self, Limits};
use brightfield_shell::run::Runner;
use brightfield_shell::trust::Store;
use brightfield_shell::window::{Boot, MeridianApp, TrustScope};

/// How long a download or a run is waited for.
const PATIENCE: Duration = Duration::from_secs(120);

fn scratch() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("bf-protocol-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for sub in ["config", "history", "db", "protocols", "input"] {
            std::fs::create_dir_all(root.join(sub)).expect("a scratch folder");
        }
        // Canonical, so the paths the window and the store compare are spelled one way.
        let root = root.canonicalize().expect("the scratch folder resolves");
        // Set once, before any test in this file reads them, and never changed.
        std::env::set_var(
            brightfield_shell::startup::CONFIG_DIR_VAR,
            root.join("config"),
        );
        std::env::set_var(
            brightfield_shell::protocols_dir::PROTOCOLS_DIR_VAR,
            root.join("protocols"),
        );
        std::env::set_var("ARCFORM_HISTORY_DIR", root.join("history"));
        std::env::set_var("ARCFORM_DB_DIR", root.join("db"));
        std::env::set_var("ARCFORM_FETCH_CACHE", "off");
        root
    })
}

/// The Protocols folder.
fn protocols() -> PathBuf {
    scratch().join("protocols")
}

/// A fresh folder for one test's inputs.
fn input(name: &str) -> PathBuf {
    let dir = scratch().join("input").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("an input folder");
    dir
}

/// A headless window, before it has drawn anything.
fn app() -> MeridianApp {
    scratch();
    MeridianApp::headless(Boot::empty(), Mode::Light)
        .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))))
}

/// One frame of `app`.
fn frame(app: &mut MeridianApp, ctx: &egui::Context) {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1440.0, 900.0),
        )),
        ..Default::default()
    };
    let _ = ctx.run_ui(raw, |ui| app.draw(ui));
}

/// What the window raised, so a failed open's own words reach the assertion.
fn said(app: &MeridianApp) -> String {
    app.notifications()
        .iter()
        .map(|n| format!("{}: {}", n.title, n.body.clone().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The committed sample a Protocol here reads.
fn measures_csv() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/shipped-protocol/data/measures.csv")
}

/// The panel the shipped sample draws, reading the table `measures`.
fn measures_panel() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/shipped-protocol/panels/overview.yaml"),
    )
    .expect("the sample panel reads")
}

/// Write a Protocol folder at `dir`, named `name`, as a website's archive holds one: a step that
/// reads its CSV, a panel over the table it makes, and the results of its last run in the
/// database its manifest names inside the folder.
fn shipped_protocol(dir: &Path, name: &str) {
    for sub in ["data", "models", "panels", "build"] {
        std::fs::create_dir_all(dir.join(sub)).expect("a folder in the Protocol");
    }
    std::fs::write(
        dir.join("arcform.yaml"),
        format!(
            "name: {name}\nengine: duckdb\ndb: build/{name}.duckdb\nsteps:\n  - name: measures\n    \
             sql: models/measures.sql\n"
        ),
    )
    .expect("the manifest");
    std::fs::write(
        dir.join("models/measures.sql"),
        "CREATE OR REPLACE TABLE \"measures\" AS\nSELECT * FROM read_csv('data/measures.csv')\n",
    )
    .expect("the step");
    std::fs::copy(measures_csv(), dir.join("data/measures.csv")).expect("the data");
    std::fs::write(dir.join("panels/overview.yaml"), measures_panel()).expect("the panel");
    let conn = duckdb::Connection::open(dir.join(format!("build/{name}.duckdb")))
        .expect("the shipped database");
    conn.execute_batch(&format!(
        "CREATE TABLE measures AS SELECT * FROM read_csv('{}')",
        measures_csv().display()
    ))
    .expect("the shipped results");
}

/// Zip the folder `dir` as `zip`, its entries under `top/`.
fn zip_folder(dir: &Path, zip: &Path, top: &str) {
    let mut out = zip::ZipWriter::new(std::fs::File::create(zip).expect("the zip"));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(at) = stack.pop() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&at)
            .expect("the folder reads")
            .map(|e| e.expect("an entry").path())
            .collect();
        entries.sort();
        for path in entries {
            let rel = path.strip_prefix(dir).expect("inside the folder");
            let name = format!("{top}/{}", rel.to_string_lossy());
            if path.is_dir() {
                out.add_directory(format!("{name}/"), options)
                    .expect("a folder entry");
                stack.push(path);
            } else {
                out.start_file(name, options).expect("a file entry");
                out.write_all(&std::fs::read(&path).expect("the file reads"))
                    .expect("the file writes");
            }
        }
    }
    out.finish().expect("the zip is written");
}

/// A zip of `entries`, each a name and its bytes, stored as written.
fn zip_of(zip: &Path, entries: &[(&str, &[u8])]) {
    let mut out = zip::ZipWriter::new(std::fs::File::create(zip).expect("the zip"));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        out.start_file(*name, options).expect("an entry");
        out.write_all(bytes).expect("its bytes");
    }
    out.finish().expect("the zip is written");
}

/// A minimal manifest for a Protocol named `name`.
fn manifest(name: &str) -> Vec<u8> {
    format!("name: {name}\nengine: duckdb\nsteps: []\n").into_bytes()
}

/// The folders directly in `dir`, by name.
fn folders_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Rewrite the uncompressed size every header in `zip` declares for its entries to `size`.
fn declare_sizes(zip: &Path, size: u32) {
    let mut bytes = std::fs::read(zip).expect("the zip reads");
    let mut i = 0;
    while i + 4 <= bytes.len() {
        match &bytes[i..i + 4] {
            // A local file header: the uncompressed size is 22 bytes in.
            [0x50, 0x4b, 0x03, 0x04] => {
                bytes[i + 22..i + 26].copy_from_slice(&size.to_le_bytes());
                i += 30;
            }
            // A central directory header: 24 bytes in.
            [0x50, 0x4b, 0x01, 0x02] => {
                bytes[i + 24..i + 28].copy_from_slice(&size.to_le_bytes());
                i += 46;
            }
            _ => i += 1,
        }
    }
    std::fs::write(zip, bytes).expect("the zip writes");
}

/// The run records arc has written for the Protocol in `dir`.
fn run_records(dir: &Path) -> usize {
    std::fs::read_dir(brightfield_shell::run::runs_dir(dir))
        .map(|r| {
            r.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        })
        .unwrap_or(0)
}

/// Drive frames until `done` holds, or fail saying `what`.
fn until(
    app: &mut MeridianApp,
    ctx: &egui::Context,
    what: &str,
    done: impl Fn(&MeridianApp) -> bool,
) {
    let deadline = Instant::now() + PATIENCE;
    while !done(app) {
        assert!(Instant::now() < deadline, "{what}: {}", said(app));
        std::thread::sleep(Duration::from_millis(50));
        frame(app, ctx);
    }
}

/// The panel tiles' outcomes: `Ok` for each that drew, with the problems of each that did not.
fn tile_outcomes(app: &MeridianApp) -> Vec<Result<(), String>> {
    app.panel_tiles()
        .and_then(|t| t.panels())
        .map(|p| {
            p.tiles
                .iter()
                .map(|t| match &t.outcome {
                    Ok(_) => Ok(()),
                    Err(problems) => Err(format!("{problems:?}")),
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// A stand-in server
// ---------------------------------------------------------------------------

/// What the stand-in answers a path with.
#[derive(Clone)]
enum Answer {
    /// 200 with these bytes.
    Body(Vec<u8>),
    /// A redirect to this address.
    Redirect(String),
}

/// A server on this Mac answering each path with its [`Answer`], and 404 otherwise.
struct Server {
    port: u16,
    hits: Arc<AtomicUsize>,
}

impl Server {
    fn serve(routes: Vec<(&'static str, Answer)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = listener.local_addr().expect("the bound address").port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                counter.fetch_add(1, Ordering::SeqCst);
                answer(stream, &routes);
            }
        });
        Self { port, hits }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

fn answer(mut stream: TcpStream, routes: &[(&'static str, Answer)]) {
    let mut reader = BufReader::new(stream.try_clone().expect("clone the socket"));
    let mut request = String::new();
    let _ = reader.read_line(&mut request);
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        if line == "\r\n" || line == "\n" {
            break;
        }
        line.clear();
    }
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let head_and_body = match routes.iter().find(|(p, _)| *p == path).map(|(_, a)| a) {
        Some(Answer::Body(body)) => (
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            ),
            body.clone(),
        ),
        Some(Answer::Redirect(to)) => (
            format!(
                "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: \
                 close\r\n\r\n"
            ),
            Vec::new(),
        ),
        None => (
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
            Vec::new(),
        ),
    };
    let _ = stream.write_all(head_and_body.0.as_bytes());
    let _ = stream.write_all(&head_and_body.1);
    let _ = stream.flush();
}

/// The check a test's downloads use: this Mac's stand-in over plain http, or what a link from a
/// web page may name.
fn this_mac_or_site(address: &url::Url) -> Result<(), String> {
    if address.scheme() == "http" && address.host_str() == Some("127.0.0.1") {
        return Ok(());
    }
    protocol_archive::check_address(address)
}

// ---------------------------------------------------------------------------
// The link
// ---------------------------------------------------------------------------

#[test]
fn a_meridian_link_is_accepted() {
    let address = protocol_archive::check_link(
        "brightfield://open?url=https://meridian.online/protocols/usgs/earthquakes_2025.zip",
    )
    .expect("a link to meridian.online is accepted");
    assert_eq!(
        address.as_str(),
        "https://meridian.online/protocols/usgs/earthquakes_2025.zip"
    );
}

#[test]
fn a_subdomain_link_is_accepted() {
    let address = protocol_archive::check_link(
        "brightfield://open?url=https://openlake.meridian.online/x.zip",
    )
    .expect("a link to a subdomain is accepted");
    assert_eq!(address.host_str(), Some("openlake.meridian.online"));
    let encoded = protocol_archive::check_link(
        "brightfield://open?url=https%3A%2F%2Fmeridian.online%2Fp%2Fq.zip",
    )
    .expect("an address percent-encoded in the link is read");
    assert_eq!(encoded.as_str(), "https://meridian.online/p/q.zip");
}

#[test]
fn other_hosts_and_http_are_refused_with_a_sentence() {
    for (link, names) in [
        (
            "brightfield://open?url=https://evil.example/x.zip",
            "evil.example is not meridian.online",
        ),
        (
            "brightfield://open?url=http://meridian.online/x.zip",
            "is not an https address",
        ),
        (
            "brightfield://open?url=https://meridian.online.evil.example/x.zip",
            "meridian.online.evil.example is not meridian.online",
        ),
        (
            "brightfield://open?url=https://meridian.online@evil.example/x.zip",
            "evil.example is not meridian.online",
        ),
        (
            "brightfield://open?url=https://notmeridian.online/x.zip",
            "notmeridian.online is not meridian.online",
        ),
        ("brightfield://anything", "is not a link Brightfield opens"),
        ("brightfield://open", "names 0 addresses"),
    ] {
        let refused = protocol_archive::check_link(link).expect_err(link);
        assert!(refused.contains(names), "{link}: {refused}");
        assert!(refused.ends_with('.'), "a sentence: {refused}");
    }
}

#[test]
fn a_redirect_off_meridian_online_is_refused() {
    let dir = input("redirected");
    shipped_protocol(&dir.join("src"), "redirected");
    let zip = dir.join("redirected.zip");
    zip_folder(&dir.join("src"), &zip, "redirected");
    let good = std::fs::read(&zip).expect("the zip reads");
    let server = Server::serve(vec![
        (
            "/away.zip",
            Answer::Redirect("https://evil.example/x.zip".to_string()),
        ),
        ("/hop.zip", Answer::Redirect("/redirected.zip".to_string())),
        ("/redirected.zip", Answer::Body(good)),
    ]);
    let into = input("redirected-into");

    let address = url::Url::parse(&server.url("/away.zip")).unwrap();
    let mut download =
        protocol_archive::Download::begin(address, this_mac_or_site, into.clone(), || {});
    let refused = wait_for(&mut download).expect_err("a redirect off the site is refused");
    assert!(
        refused.contains("redirected to https://evil.example/x.zip")
            && refused.contains("evil.example is not meridian.online"),
        "{refused}"
    );
    assert!(folders_in(&into).is_empty(), "nothing was unpacked");

    // A redirect that stays where it may is followed, and checked again at its end.
    let address = url::Url::parse(&server.url("/hop.zip")).unwrap();
    let mut download =
        protocol_archive::Download::begin(address, this_mac_or_site, into.clone(), || {});
    let dir = wait_for(&mut download).expect("a redirect on this Mac is followed");
    assert_eq!(dir, into.join("redirected"));
    assert!(dir.join("arcform.yaml").is_file());
}

/// Wait for `download` to answer.
fn wait_for(download: &mut protocol_archive::Download) -> Result<PathBuf, String> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(result) = download.take() {
            return result;
        }
        assert!(Instant::now() < deadline, "the download never answered");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---------------------------------------------------------------------------
// The archive
// ---------------------------------------------------------------------------

#[test]
fn an_archive_with_dotdot_or_absolute_entries_writes_nothing() {
    let dir = input("escaping");
    let into = input("escaping-into");
    for (case, entries) in [
        (
            "dotdot",
            vec![
                ("p/arcform.yaml", manifest("p")),
                ("p/../../escaped.txt", b"out".to_vec()),
            ],
        ),
        (
            "inner-dotdot",
            vec![
                ("p/arcform.yaml", manifest("p")),
                ("p/models/../data/a.csv", b"a\n1\n".to_vec()),
            ],
        ),
        (
            "absolute",
            vec![
                ("p/arcform.yaml", manifest("p")),
                ("/tmp/escaped.txt", b"out".to_vec()),
            ],
        ),
        (
            "two-tops",
            vec![
                ("p/arcform.yaml", manifest("p")),
                ("q/a.txt", b"q".to_vec()),
            ],
        ),
        ("no-manifest", vec![("p/a.txt", b"a".to_vec())]),
        (
            "case",
            vec![
                ("p/arcform.yaml", manifest("p")),
                ("p/models/a.sql", b"SELECT 1\n".to_vec()),
                ("p/MODELS/a.sql", b"SELECT 2\n".to_vec()),
            ],
        ),
    ] {
        let zip = dir.join(format!("{case}.zip"));
        let borrowed: Vec<(&str, &[u8])> =
            entries.iter().map(|(n, b)| (*n, b.as_slice())).collect();
        zip_of(&zip, &borrowed);
        let refused = protocol_archive::unpack(&zip, &into).expect_err(case);
        assert!(
            refused.contains("Nothing was unpacked"),
            "{case}: {refused}"
        );
        assert!(folders_in(&into).is_empty(), "{case}: nothing was written");
    }
    assert!(!dir.join("escaped.txt").exists() && !scratch().join("escaped.txt").exists());

    // A link inside the archive is refused too.
    let zip = dir.join("link.zip");
    let mut out = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    out.start_file("p/arcform.yaml", options).unwrap();
    out.write_all(&manifest("p")).unwrap();
    out.add_symlink("p/link", "/etc/passwd", options).unwrap();
    out.finish().unwrap();
    let refused = protocol_archive::unpack(&zip, &into).expect_err("a link");
    assert!(refused.contains("holds a link"), "{refused}");
    assert!(folders_in(&into).is_empty());
}

#[test]
fn an_archive_past_the_limits_writes_nothing() {
    let dir = input("limits");
    let into = input("limits-into");

    let many = dir.join("many.zip");
    let mut out = zip::ZipWriter::new(std::fs::File::create(&many).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    out.start_file("p/arcform.yaml", options).unwrap();
    out.write_all(&manifest("p")).unwrap();
    for i in 0..protocol_archive::MAX_ENTRIES {
        out.start_file(format!("p/data/{i}.txt"), options).unwrap();
    }
    out.finish().unwrap();
    let refused = protocol_archive::unpack(&many, &into).expect_err("too many entries");
    assert!(
        refused.contains("10001 entries, more than the 10000"),
        "{refused}"
    );
    assert!(folders_in(&into).is_empty());

    // Headers that declare more than the limit, though the bytes are few.
    let large = dir.join("large.zip");
    zip_of(
        &large,
        &[
            ("p/arcform.yaml", &manifest("p")),
            ("p/data/a.bin", &[0u8; 64]),
            ("p/data/b.bin", &[0u8; 64]),
        ],
    );
    declare_sizes(&large, 1_200_000_000);
    let refused = protocol_archive::unpack(&large, &into).expect_err("too large by its headers");
    assert!(refused.contains("would unpack to more than"), "{refused}");
    assert!(folders_in(&into).is_empty());
}

#[test]
fn an_archive_that_understates_its_sizes_is_stopped_and_removed() {
    let dir = input("understated");
    let into = input("understated-into");
    let zip = dir.join("understated.zip");
    zip_of(
        &zip,
        &[
            ("p/arcform.yaml", &manifest("p")),
            ("p/data/big.bin", &vec![7u8; 64 * 1024]),
        ],
    );
    // Each entry says it is 100 bytes, which is under the limit; the bytes are more.
    declare_sizes(&zip, 100);
    let limits = Limits {
        entries: 10,
        bytes: 4096,
    };
    let refused =
        protocol_archive::unpack_within(&zip, &into, limits).expect_err("stopped as it writes");
    assert!(
        refused.contains("holds more than its headers say"),
        "{refused}"
    );
    assert!(
        folders_in(&into).is_empty(),
        "the partial folder is taken away: {:?}",
        folders_in(&into)
    );
}

#[test]
fn an_archive_naming_an_absolute_local_path_is_refused_naming_it() {
    let dir = input("absolute-path");
    let into = input("absolute-path-into");
    let sources = br#"{"dir": "/Users/someone/p", "tables": [{"name": "a", "file": "/Users/someone/data/a.csv", "reader": "csv"}], "skipped": []}"#;
    for (case, entries, named) in [
        (
            "sources",
            vec![
                ("p/arcform.yaml", manifest("p")),
                ("p/.sources.json", sources.to_vec()),
            ],
            "/Users/someone/data/a.csv",
        ),
        (
            "sql",
            vec![
                (
                    "p/arcform.yaml",
                    b"name: p\nengine: duckdb\nsteps:\n  - name: a\n    sql: models/a.sql\n"
                        .to_vec(),
                ),
                (
                    "p/models/a.sql",
                    b"-- a comment naming '/etc/hosts' is not read\nCREATE TABLE a AS SELECT * \
                      FROM read_csv('/Users/someone/data/a.csv')\n"
                        .to_vec(),
                ),
            ],
            "/Users/someone/data/a.csv",
        ),
        (
            "database",
            vec![(
                "p/arcform.yaml",
                b"name: p\nengine: duckdb\ndb: /Users/someone/p.duckdb\nsteps: []\n".to_vec(),
            )],
            "/Users/someone/p.duckdb",
        ),
        (
            "home",
            vec![
                (
                    "p/arcform.yaml",
                    b"name: p\nengine: duckdb\nsteps:\n  - name: a\n    sql: models/a.sql\n"
                        .to_vec(),
                ),
                (
                    "p/models/a.sql",
                    b"SELECT * FROM read_parquet('~/data/a.parquet')\n".to_vec(),
                ),
            ],
            "~/data/a.parquet",
        ),
        (
            "from",
            vec![
                (
                    "p/arcform.yaml",
                    b"name: p\nengine: duckdb\nsteps:\n  - name: a\n    sql: models/a.sql\n"
                        .to_vec(),
                ),
                (
                    "p/models/a.sql",
                    b"SELECT * FROM '/Users/someone/a.parquet'\n".to_vec(),
                ),
            ],
            "/Users/someone/a.parquet",
        ),
        (
            // The manifest spells the step's file another way; the file is read all the same.
            "spelled",
            vec![
                (
                    "p/arcform.yaml",
                    b"name: p\nengine: duckdb\nsteps:\n  - name: a\n    sql: ./models/a.sql\n"
                        .to_vec(),
                ),
                (
                    "p/models/a.sql",
                    b"COPY (SELECT 1) TO '/Users/someone/out.csv'\n".to_vec(),
                ),
            ],
            "/Users/someone/out.csv",
        ),
    ] {
        let zip = dir.join(format!("{case}.zip"));
        let borrowed: Vec<(&str, &[u8])> =
            entries.iter().map(|(n, b)| (*n, b.as_slice())).collect();
        zip_of(&zip, &borrowed);
        let refused = protocol_archive::unpack(&zip, &into).expect_err(case);
        assert!(
            refused.contains(named) && refused.contains("could run on no other"),
            "{case}: {refused}"
        );
        assert!(folders_in(&into).is_empty(), "{case}: nothing was written");
    }

    // A remote source is not a local path, and a literal that is not a file, such as a pattern,
    // is not one either.
    let zip = dir.join("remote.zip");
    zip_of(
        &zip,
        &[
            (
                "p/arcform.yaml",
                b"name: p\nengine: duckdb\nsteps:\n  - name: a\n    sql: models/a.sql\n",
            ),
            (
                "p/models/a.sql",
                b"SELECT * FROM read_parquet('https://meridian.online/a.parquet') WHERE s = '/' \
                  AND p LIKE '/api/%' AND regexp_replace(q, '/+$', '') = '\\\\' \
                  AND r = '/index.html'\n",
            ),
        ],
    );
    let unpacked = protocol_archive::unpack(&zip, &into).expect("a remote source unpacks");
    assert!(unpacked.join("models/a.sql").is_file());
}

#[test]
fn a_clashing_archive_takes_dash_two() {
    let dir = input("clashing");
    let into = input("clashing-into");
    shipped_protocol(&dir.join("src"), "quakes");
    let zip = dir.join("quakes.zip");
    zip_folder(&dir.join("src"), &zip, "quakes");
    let first = protocol_archive::unpack(&zip, &into).expect("the first unpacks");
    let second = protocol_archive::unpack(&zip, &into).expect("the second unpacks");
    assert_eq!(first, into.join("quakes"));
    assert_eq!(second, into.join("quakes-2"));
    assert!(second.join("panels/overview.yaml").is_file());
    assert!(second.join("build/quakes.duckdb").is_file());
}

#[test]
fn a_source_path_escaping_the_folder_is_refused() {
    let dir = input("escaping-source");
    let into = input("escaping-source-into");
    let sources =
        br#"{"tables": [{"name": "a", "file": "../../a.csv", "reader": "csv"}], "skipped": []}"#;
    let package = br#"{"name": "a", "resources": [{"name": "a", "path": "../../../a.csv"}]}"#;
    let inside = br#"{"name": "a", "resources": [{"name": "a", "path": "../../data/a.csv"}]}"#;
    for (case, entries, named) in [
        (
            "sources",
            vec![
                ("p/arcform.yaml", manifest("p")),
                ("p/.sources.json", sources.to_vec()),
            ],
            "../../a.csv",
        ),
        (
            "package",
            vec![
                ("p/arcform.yaml", manifest("p")),
                ("p/packages/a/datapackage.json", package.to_vec()),
            ],
            "../../../a.csv",
        ),
        (
            "sql",
            vec![
                (
                    "p/arcform.yaml",
                    b"name: p\nengine: duckdb\nsteps:\n  - name: a\n    sql: models/a.sql\n"
                        .to_vec(),
                ),
                (
                    "p/models/a.sql",
                    b"SELECT * FROM read_csv('data/../../a.csv')\n".to_vec(),
                ),
            ],
            "data/../../a.csv",
        ),
        (
            "step-file",
            vec![(
                "p/arcform.yaml",
                b"name: p\nengine: duckdb\nsteps:\n  - name: a\n    sql: ../a.sql\n".to_vec(),
            )],
            "../a.sql",
        ),
    ] {
        let zip = dir.join(format!("{case}.zip"));
        let borrowed: Vec<(&str, &[u8])> =
            entries.iter().map(|(n, b)| (*n, b.as_slice())).collect();
        zip_of(&zip, &borrowed);
        let refused = protocol_archive::unpack(&zip, &into).expect_err(case);
        assert!(
            refused.contains(named) && refused.contains("outside the Protocol's folder"),
            "{case}: {refused}"
        );
        assert!(folders_in(&into).is_empty(), "{case}: nothing was written");
    }

    // A package that reads a file inside the Protocol, by climbing out of its own folder, is fine.
    let zip = dir.join("inside.zip");
    zip_of(
        &zip,
        &[
            ("p/arcform.yaml", &manifest("p")),
            ("p/packages/a/datapackage.json", inside),
            ("p/data/a.csv", b"a\n1\n"),
        ],
    );
    protocol_archive::unpack(&zip, &into).expect("a package reading inside its Protocol unpacks");
}

// ---------------------------------------------------------------------------
// Restricted and trusted opens
// ---------------------------------------------------------------------------

#[test]
fn an_archived_protocol_opens_restricted_and_draws_its_shipped_results() {
    let dir = input("archived");
    let server = Server::serve(Vec::new());
    shipped_protocol(&dir.join("src"), "archived");
    // A step that would fetch from the stand-in, so a fetch is seen if one happens.
    let manifest_path = dir.join("src/arcform.yaml");
    let mut text = std::fs::read_to_string(&manifest_path).unwrap();
    text.push_str(&format!(
        "  - name: fetch\n    op: http_fetch\n    with:\n      out: data/fetched.csv\n      url: {}\n",
        server.url("/fetched.csv")
    ));
    std::fs::write(&manifest_path, text).unwrap();
    let zip = dir.join("archived.zip");
    zip_folder(&dir.join("src"), &zip, "archived");

    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(zip));
    frame(&mut app, &ctx);
    frame(&mut app, &ctx);

    let restricted = app
        .restricted()
        .cloned()
        .unwrap_or_else(|| panic!("the archive opens restricted: {}", said(&app)));
    assert_eq!(restricted.dir, protocols().join("archived"));
    assert_eq!(restricted.origin, "the archive archived.zip");
    assert!(
        restricted.said().contains("archived.zip"),
        "the band names it"
    );
    assert_eq!(
        tile_outcomes(&app),
        vec![Ok(())],
        "the panel draws from the shipped results"
    );
    assert!(
        app.panel_tiles().is_some_and(|t| t.is_confined()),
        "drawn confined"
    );
    assert!(!app.run_protocol(&ctx), "a restricted Protocol is not run");
    for _ in 0..5 {
        frame(&mut app, &ctx);
    }
    assert_eq!(run_records(&restricted.dir), 0, "no run record is written");
    assert_eq!(server.hits(), 0, "nothing is fetched");
    assert!(
        said(&app).contains("restricted, so it was not run"),
        "{}",
        said(&app)
    );
    assert!(
        !app.open_panel_with_grid(0),
        "its chart stays a picture until it is trusted"
    );
}

#[test]
fn trust_unlocks_running_and_is_remembered() {
    let dir = input("trusting").join("trusted_here");
    shipped_protocol(&dir, "trusted_here");
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(dir.clone()));
    frame(&mut app, &ctx);
    assert!(
        app.restricted().is_some(),
        "it opens restricted: {}",
        said(&app)
    );
    assert!(!app.run_protocol(&ctx));
    assert_eq!(run_records(&dir), 0);

    assert!(
        app.trust_restricted(&ctx, TrustScope::Protocol),
        "{}",
        said(&app)
    );
    frame(&mut app, &ctx);
    assert!(app.restricted().is_none(), "trusted: {}", said(&app));
    assert!(
        app.run_protocol(&ctx),
        "a trusted Protocol runs: {}",
        said(&app)
    );
    until(&mut app, &ctx, "the run lands", |a| !a.run_in_progress());
    assert!(
        run_records(&dir) >= 1,
        "the run wrote its record: {}",
        said(&app)
    );

    let mut again = self::app();
    frame(&mut again, &ctx);
    again.open_queue().push(OpenRequest::Path(dir.clone()));
    frame(&mut again, &ctx);
    assert!(
        again.restricted().is_none(),
        "reopened, it is not restricted: {}",
        said(&again)
    );
}

#[test]
fn a_protocol_folder_brightfield_did_not_make_opens_restricted() {
    let outside = input("foreign").join("foreign_outside");
    shipped_protocol(&outside, "foreign_outside");
    let inside = protocols().join("foreign_inside");
    let _ = std::fs::remove_dir_all(&inside);
    shipped_protocol(&inside, "foreign_inside");
    for dir in [&outside, &inside] {
        let mut app = app();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        app.open_queue().push(OpenRequest::Path(dir.clone()));
        frame(&mut app, &ctx);
        let restricted = app
            .restricted()
            .unwrap_or_else(|| panic!("{} opens restricted: {}", dir.display(), said(&app)));
        assert!(
            restricted
                .origin
                .starts_with("a folder Brightfield did not make"),
            "{}",
            restricted.origin
        );
    }

    // One Brightfield made opens trusted, wherever it was made.
    let csv = input("made").join("made_here.csv");
    std::fs::copy(measures_csv(), &csv).unwrap();
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(csv));
    frame(&mut app, &ctx);
    let made = app
        .own_protocol()
        .map(|p| p.dir.clone())
        .unwrap_or_else(|| panic!("the CSV makes a Protocol: {}", said(&app)));
    assert!(Store::current().is_trusted(&made));
    let mut again = self::app();
    frame(&mut again, &ctx);
    again.open_queue().push(OpenRequest::Path(made.clone()));
    frame(&mut again, &ctx);
    assert!(again.restricted().is_none(), "{}", said(&again));
}

#[test]
fn an_unpacked_archive_in_the_protocols_folder_is_not_trusted_by_location() {
    let dir = input("by-location");
    shipped_protocol(&dir.join("src"), "by_location");
    let zip = dir.join("by_location.zip");
    zip_folder(&dir.join("src"), &zip, "by_location");
    let unpacked = protocol_archive::unpack(&zip, &protocols()).expect("it unpacks");
    assert!(unpacked.starts_with(protocols()));
    assert!(!Store::current().is_trusted(&unpacked));
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(unpacked));
    frame(&mut app, &ctx);
    assert!(app.restricted().is_some(), "{}", said(&app));
}

#[test]
fn a_brightfield_made_protocol_in_recents_from_before_trust_opens_trusted() {
    // Made as an earlier Brightfield made them, with no trust record: a one-step Protocol saved
    // beside its CSV, an own-files Protocol in Application Support's older folder, and a folder
    // Brightfield did not make.
    let one_step_dir = input("before-trust-one-step");
    let csv = one_step_dir.join("before.csv");
    std::fs::copy(measures_csv(), &csv).unwrap();
    let opened = brightfield_shell::data_file::open(&csv.to_string_lossy()).expect("the CSV opens");
    opened
        .protocol
        .save_to(&one_step_dir)
        .expect("it saves beside the CSV");
    let store = Store::current();
    store.revoke(&one_step_dir).unwrap();
    assert!(!store.is_trusted(&one_step_dir), "made before trust");

    let old_own = brightfield_shell::starts::datasets_dir()
        .join("protocols")
        .join(brightfield_shell::own_files::OWN_DIR);
    let folder = input("before-trust-own");
    for name in ["a.csv", "b.csv"] {
        std::fs::copy(measures_csv(), folder.join(name)).unwrap();
    }
    let own =
        brightfield_shell::own_files::create_in(&old_own, &folder).expect("an own-files Protocol");
    // The model dovetail writes, which an own-files Protocol of several sources holds.
    std::fs::write(own.dir.join(brightfield_shell::discovery::MODEL_FILE), "").unwrap();
    store.revoke(&own.dir).unwrap();
    assert!(!store.is_trusted(&own.dir));

    let foreign = input("before-trust-foreign").join("foreign_recent");
    shipped_protocol(&foreign, "foreign_recent");

    let mut layout = brightfield_shell::startup::default_layout();
    for dir in [&one_step_dir, &foreign] {
        let id = dir.join("arcform.yaml").to_string_lossy().into_owned();
        layout.remember(
            &id,
            "recent",
            Default::default(),
            Default::default(),
            Default::default(),
            1,
        );
    }
    let mut app = MeridianApp::headless_with_layout(Boot::empty(), layout, Mode::Light)
        .running_with(Some(Runner::at(env!("CARGO_BIN_EXE_brightfield-shell"))));
    assert!(
        app.record_older_protocols_once(),
        "the first launch records them"
    );
    assert!(
        store.is_trusted(&one_step_dir),
        "the one-step Protocol in Recents"
    );
    assert!(
        store.is_trusted(&own.dir),
        "the own-files Protocol in Application Support"
    );
    assert!(
        !store.is_trusted(&foreign),
        "a foreign folder in Recents is not"
    );

    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(foreign.clone()));
    frame(&mut app, &ctx);
    assert!(app.restricted().is_some(), "{}", said(&app));
    app.open_queue()
        .push(OpenRequest::Path(one_step_dir.join("arcform.yaml")));
    frame(&mut app, &ctx);
    frame(&mut app, &ctx);
    assert!(app.restricted().is_none(), "{}", said(&app));

    // A later launch records nothing more, though Recents holds another such Protocol.
    let later_dir = input("before-trust-later");
    let later_csv = later_dir.join("later.csv");
    std::fs::copy(measures_csv(), &later_csv).unwrap();
    let opened = brightfield_shell::data_file::open(&later_csv.to_string_lossy()).unwrap();
    opened.protocol.save_to(&later_dir).unwrap();
    store.revoke(&later_dir).unwrap();
    let mut layout = brightfield_shell::startup::default_layout();
    let id = later_dir
        .join("arcform.yaml")
        .to_string_lossy()
        .into_owned();
    layout.remember(
        &id,
        "later",
        Default::default(),
        Default::default(),
        Default::default(),
        2,
    );
    let mut later = MeridianApp::headless_with_layout(Boot::empty(), layout, Mode::Light);
    assert!(!later.record_older_protocols_once(), "recorded once only");
    assert!(
        !store.is_trusted(&later_dir),
        "added to Recents later, it is not swept in"
    );
}

#[test]
fn the_first_trust_aware_launch_records_older_protocols_once() {
    let dir = input("once-store");
    let store = Store::at(&dir.join("config").join("trust.json"));
    let a = dir.join("a");
    let b = dir.join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    assert!(store.record_older_once(std::slice::from_ref(&a)).unwrap());
    assert!(store.is_trusted(&a));
    assert!(!store.record_older_once(std::slice::from_ref(&b)).unwrap());
    assert!(!store.is_trusted(&b), "a later sweep records nothing");
    // And the store keeps that across a restart, being a file.
    let reopened = Store::at(&dir.join("config").join("trust.json"));
    assert!(reopened.is_trusted(&a) && !reopened.is_trusted(&b));
    assert!(!reopened
        .record_older_once(std::slice::from_ref(&b))
        .unwrap());
}

#[test]
fn a_brightfield_made_protocol_moved_in_finder_stays_trusted() {
    let folder = input("moving");
    for name in ["a.csv", "b.csv"] {
        std::fs::copy(measures_csv(), folder.join(name)).unwrap();
    }
    let parent = input("moving-made");
    let made = brightfield_shell::own_files::create_in(&parent, &folder).expect("a Protocol");
    let moved_to = input("moving-elsewhere").join("moved");
    std::fs::rename(&made.dir, &moved_to).expect("moved as the Finder moves it");
    let store = Store::current();
    assert!(
        store.is_trusted(&moved_to),
        "it keeps its trust where it is now"
    );

    // And a folder made where it used to be is not the one that was trusted.
    std::fs::create_dir_all(&made.dir).unwrap();
    std::fs::write(made.dir.join("arcform.yaml"), manifest("impostor")).unwrap();
    assert!(!store.is_trusted(&made.dir), "a new folder at the old path");

    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(moved_to.clone()));
    frame(&mut app, &ctx);
    assert!(app.restricted().is_none(), "{}", said(&app));
}

#[test]
fn an_archive_reopened_from_recents_or_the_door_stays_restricted() {
    let dir = input("reopened");
    shipped_protocol(&dir.join("src"), "reopened");
    let zip = dir.join("reopened.zip");
    zip_folder(&dir.join("src"), &zip, "reopened");
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(zip));
    frame(&mut app, &ctx);
    let unpacked = app
        .restricted()
        .map(|r| r.dir.clone())
        .unwrap_or_else(|| panic!("restricted: {}", said(&app)));

    // Reopened as a Recents or tree row reopens it, by its manifest, in a window of its own.
    let mut again = self::app();
    frame(&mut again, &ctx);
    again.open_protocol_path(&ctx, &unpacked.join("arcform.yaml").to_string_lossy());
    frame(&mut again, &ctx);
    let restricted = again
        .restricted()
        .unwrap_or_else(|| panic!("still restricted: {}", said(&again)));
    assert_eq!(
        restricted.origin, "the archive reopened.zip",
        "and it still names where it came from"
    );
    assert!(!again.run_protocol(&ctx));
    assert_eq!(run_records(&unpacked), 0);
}

#[test]
fn an_archive_carrying_a_made_by_brightfield_marker_or_the_one_step_layout_stays_restricted() {
    let dir = input("disguised");
    let csv = std::fs::read(measures_csv()).unwrap();
    // An empty own-files Protocol's layout, with the marker New Protocol writes.
    let empty = dir.join("empty_shape.zip");
    zip_of(
        &empty,
        &[
            ("empty_shape/arcform.yaml", &manifest("empty_shape")),
            (
                "empty_shape/.sources.json",
                br#"{"tables": [], "skipped": []}"#,
            ),
            ("empty_shape/.untitled", b"empty_shape\n"),
            ("empty_shape/models/.keep", b""),
        ],
    );
    // A one-step Protocol's layout: a load of the CSV beside the manifest.
    let source = input("disguised-one-step");
    let beside = source.join("one_step_shape.csv");
    std::fs::write(&beside, &csv).unwrap();
    let opened = brightfield_shell::data_file::open(&beside.to_string_lossy()).unwrap();
    opened.protocol.save_to(&source).unwrap();
    let one_step = dir.join("one_step_shape.zip");
    zip_folder(&source, &one_step, "one_step_shape");

    for zip in [empty, one_step] {
        let mut app = app();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);
        app.open_queue().push(OpenRequest::Path(zip.clone()));
        frame(&mut app, &ctx);
        assert!(
            app.restricted().is_some(),
            "{} opens restricted: {}",
            zip.display(),
            said(&app)
        );
    }
}

#[test]
fn a_protocol_new_protocol_made_outside_the_protocols_folder_is_trusted() {
    let elsewhere = input("new-elsewhere");
    let made = brightfield_shell::own_files::create_empty(&elsewhere).expect("New Protocol");
    assert!(!made.dir.starts_with(protocols()));
    assert!(Store::current().is_trusted(&made.dir));
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(made.dir.clone()));
    frame(&mut app, &ctx);
    assert!(app.restricted().is_none(), "{}", said(&app));
    assert_eq!(app.empty_protocol().map(|p| p.dir.clone()), Some(made.dir));
}

#[test]
fn rename_relocate_and_the_preferences_move_repoint_trust_records() {
    let store = Store::current();
    let folder = input("repoint-sources");
    std::fs::copy(measures_csv(), folder.join("a.csv")).unwrap();
    let parent = input("repoint-made");
    let made = brightfield_shell::own_files::create_empty(&parent).expect("a Protocol");

    let renamed = brightfield_shell::rename::rename(&made.dir, "renamed").expect("renamed");
    assert!(store.is_trusted(&renamed), "after a rename");
    let elsewhere = input("repoint-elsewhere");
    let relocated = brightfield_shell::rename::relocate(&renamed, &elsewhere).expect("relocated");
    assert!(store.is_trusted(&relocated), "after Location moves it");

    // The Preferences move, from one Protocols folder to another.
    let from = input("repoint-from");
    let to = input("repoint-to");
    let in_from = brightfield_shell::own_files::create_empty(&from).expect("a Protocol");
    let report = brightfield_shell::protocols_dir::move_all(&from, &to);
    assert!(report.failed.is_none(), "{}", report.said());
    let (_, moved) = report
        .moved
        .iter()
        .find(|(was, _)| {
            was.canonicalize().ok() == in_from.dir.canonicalize().ok() || *was == in_from.dir
        })
        .cloned()
        .unwrap_or_else(|| panic!("it moved: {:?}", report.moved));
    assert!(store.is_trusted(&moved), "after the Preferences move");

    // A Protocol trusted only through a folder above it keeps its trust when moved out from
    // under that folder, since the move was asked for.
    let group = input("repoint-group");
    let beneath = group.join("beneath");
    shipped_protocol(&beneath, "beneath");
    store.trust_folder(&group).unwrap();
    assert!(store.is_trusted(&beneath));
    let outside = input("repoint-outside");
    let moved_out = brightfield_shell::rename::relocate(&beneath, &outside).expect("moved out");
    assert!(
        store.is_trusted(&moved_out),
        "its trust is recorded on itself as it moves"
    );
    store.revoke(&group).unwrap();
    assert!(store.is_trusted(&moved_out));

    // One that was not trusted is not made so by a move.
    let stranger = input("repoint-stranger").join("stranger");
    shipped_protocol(&stranger, "stranger");
    let moved_stranger =
        brightfield_shell::rename::relocate(&stranger, &input("repoint-stranger-to")).unwrap();
    assert!(!store.is_trusted(&moved_stranger));
}

#[test]
fn trusting_a_folder_trusts_protocols_beneath_it_including_later_ones() {
    let group = input("trusted-group");
    let first = group.join("first");
    shipped_protocol(&first, "first");
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(first.clone()));
    frame(&mut app, &ctx);
    let offered = app
        .restricted()
        .map(|r| r.folders_above())
        .unwrap_or_else(|| panic!("restricted: {}", said(&app)));
    assert_eq!(
        offered.first(),
        Some(&group),
        "the band offers its folder first"
    );
    assert!(app.trust_restricted(&ctx, TrustScope::Folder(group.clone())));
    frame(&mut app, &ctx);
    assert!(app.restricted().is_none(), "{}", said(&app));
    assert!(Store::current().folders().contains(&group));

    // An archive unpacked into the folder later arrives trusted.
    let dir = input("trusted-group-archive");
    shipped_protocol(&dir.join("src"), "later");
    let zip = dir.join("later.zip");
    zip_folder(&dir.join("src"), &zip, "later");
    let later = protocol_archive::unpack(&zip, &group).expect("it unpacks");
    assert!(Store::current().is_trusted(&later));
    let mut again = self::app();
    frame(&mut again, &ctx);
    again.open_queue().push(OpenRequest::Path(later));
    frame(&mut again, &ctx);
    assert!(again.restricted().is_none(), "{}", said(&again));
}

#[test]
fn removing_a_trusted_folder_restricts_what_it_covered() {
    let group = input("removed-group");
    let beneath = group.join("deep").join("beneath");
    shipped_protocol(&beneath, "beneath");
    let store = Store::current();
    store.trust_folder(&group).unwrap();
    assert!(store.is_trusted(&beneath));

    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.untrust_folder(&ctx, &group);
    assert!(!store.folders().contains(&group));
    assert!(!store.is_trusted(&beneath));
    app.open_queue().push(OpenRequest::Path(beneath.clone()));
    frame(&mut app, &ctx);
    assert!(app.restricted().is_some(), "{}", said(&app));
}

#[test]
fn restricted_drawing_reads_only_shipped_results() {
    let dir = input("bounded").join("bounded");
    shipped_protocol(&dir, "bounded");
    let secret = input("bounded-secret").join("secret.csv");
    std::fs::write(&secret, "flipper_mm,mass_g,species\n1,2,x\n").unwrap();
    // A view in the shipped database that reads a file outside the folder.
    {
        let conn = duckdb::Connection::open(dir.join("build/bounded.duckdb")).unwrap();
        conn.execute_batch(&format!(
            "CREATE VIEW leaked AS SELECT * FROM read_csv('{}')",
            secret.display()
        ))
        .unwrap();
    }
    let panel = measures_panel();
    std::fs::write(
        dir.join("panels/via_view.yaml"),
        panel.replace("from: measures", "from: leaked"),
    )
    .unwrap();
    // A panel that names the outside file as its own data.
    std::fs::write(
        dir.join("panels/via_file.yaml"),
        format!(
            "data:\n  outside: {{ file: {} }}\n{}",
            secret.display(),
            panel.replace("from: measures", "from: outside")
        ),
    )
    .unwrap();

    let panels = brightfield_shell::protocol_panels::load_shipped(
        &dir,
        brightfield_shell::panel_tiles::viewport(egui::vec2(480.0, 320.0)),
    );
    // The view's copy is refused as the snapshot is taken, so nothing opens from the database.
    let problem = panels.err().map(|p| p.message()).unwrap_or_default();
    assert!(
        problem.contains("disabled by configuration"),
        "the view's read of an outside file is refused: {problem}"
    );

    // Without the view, the shipped table draws and the outside file does not.
    std::fs::remove_file(dir.join("panels/via_view.yaml")).unwrap();
    let panels = brightfield_shell::protocol_panels::load_shipped(
        &dir,
        brightfield_shell::panel_tiles::viewport(egui::vec2(480.0, 320.0)),
    )
    .expect("it opens from its shipped results");
    let outcomes: Vec<(String, bool)> = panels
        .tiles
        .iter()
        .map(|t| (t.file.to_string_lossy().into_owned(), t.outcome.is_ok()))
        .collect();
    assert_eq!(
        outcomes,
        vec![
            ("panels/overview.yaml".to_string(), true),
            ("panels/via_file.yaml".to_string(), false),
        ]
    );
    let refused = match &panels.tiles[1].outcome {
        Err(problems) => format!("{problems:?}"),
        Ok(_) => String::new(),
    };
    assert!(
        refused.contains("disabled by configuration"),
        "the panel's own read of an outside file is refused: {refused}"
    );

    // The same Protocol trusted reads the file, so the refusal is the confinement's.
    let drawn = brightfield_shell::pipeline::compose_spec_str_in(
        &format!(
            "data:\n  outside: {{ file: {} }}\n{}",
            secret.display(),
            panel.replace("from: measures", "from: outside")
        ),
        Some(&dir),
        brightfield_spec::layout::Rect {
            x: 0.0,
            y: 0.0,
            width: 480.0,
            height: 320.0,
        },
        Mode::Light,
    );
    assert!(drawn.is_ok(), "{:?}", drawn.err());
    let confined = brightfield_shell::pipeline::compose_spec_str_confined(
        "data:\n  t: { file: https://meridian.online/t.parquet }\nplot:\n  - mark: dot\n    data: \
         { from: t }\n    x: a\n    y: b\n",
        Some(&dir),
        brightfield_spec::layout::Rect {
            x: 0.0,
            y: 0.0,
            width: 480.0,
            height: 320.0,
        },
        Mode::Light,
        &[dir.join("build")],
    );
    assert!(confined.is_err(), "a remote read is refused too");
}

#[test]
fn a_good_archive_unpacks_and_draws() {
    let dir = input("good");
    shipped_protocol(&dir.join("src"), "good_link");
    let zip = dir.join("good_link.zip");
    zip_folder(&dir.join("src"), &zip, "good_link");
    let server = Server::serve(vec![(
        "/protocols/good_link.zip",
        Answer::Body(std::fs::read(&zip).unwrap()),
    )]);

    let mut app = app().fetching_archives_with(this_mac_or_site);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Link(format!(
        "brightfield://open?url={}",
        server.url("/protocols/good_link.zip")
    )));
    frame(&mut app, &ctx);
    assert!(
        app.is_downloading() || app.restricted().is_some(),
        "{}",
        said(&app)
    );
    until(&mut app, &ctx, "the archive opens", |a| {
        a.restricted().is_some()
    });
    let restricted = app.restricted().cloned().unwrap();
    assert_eq!(restricted.dir, protocols().join("good_link"));
    assert_eq!(restricted.origin, "127.0.0.1", "the band names the host");
    assert!(restricted.said().contains("127.0.0.1"));
    assert_eq!(
        tile_outcomes(&app),
        vec![Ok(())],
        "its shipped results draw"
    );
    assert!(server.hits() >= 1);
    assert!(
        !said(&app).contains("Downloading"),
        "the download's banner is down: {}",
        said(&app)
    );

    // A link the window refuses says why, titled for a link, and fetches nothing.
    let before = server.hits();
    app.open_queue().push(OpenRequest::Link(
        "brightfield://open?url=https://evil.example/x.zip".to_string(),
    ));
    frame(&mut app, &ctx);
    assert!(
        said(&app).contains(brightfield_shell::window::LINK_NOTICE_TITLE)
            && said(&app).contains("evil.example is not meridian.online"),
        "{}",
        said(&app)
    );
    assert_eq!(server.hits(), before);
    let mut buf = Vec::new();
    let _ = std::fs::File::open(&zip).map(|mut f| f.read_to_end(&mut buf));
    assert!(!buf.is_empty());
}

#[test]
fn a_download_that_stalls_ends_with_a_sentence() {
    // Headers, then silence on an open connection.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                if line == "\r\n" {
                    break;
                }
                line.clear();
            }
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\nPK",
            );
            let _ = stream.flush();
            std::thread::sleep(Duration::from_secs(30));
        }
    });
    let into = input("stalled-into");
    let address = url::Url::parse(&format!("http://127.0.0.1:{port}/stalled.zip")).unwrap();
    let mut download = protocol_archive::Download::begin_within(
        address,
        this_mac_or_site,
        into.clone(),
        Duration::from_secs(1),
        || {},
    );
    let started = Instant::now();
    let refused = wait_for(&mut download).expect_err("a stalled download ends");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "it ends after the silence, not the server's sleep"
    );
    assert!(refused.contains("could not download"), "{refused}");
    assert!(folders_in(&into).is_empty());
}

#[test]
fn trust_over_a_draft_asks_the_close_question_first() {
    let dir = input("trust-draft").join("trust_draft");
    shipped_protocol(&dir, "trust_draft");
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx);
    app.open_queue().push(OpenRequest::Path(dir.clone()));
    frame(&mut app, &ctx);
    assert!(app.restricted().is_some(), "{}", said(&app));
    app.chart_doc_mut()
        .apply_grid_view(brightfield_engine::RowsView {
            condition: Some("mass_g > 4000".to_owned()),
            order: None,
        });
    frame(&mut app, &ctx);
    assert!(app.has_unsaved_work(), "a draft is held");

    assert!(app.trust_restricted(&ctx, TrustScope::Protocol));
    frame(&mut app, &ctx);
    assert!(
        app.close_question().is_some(),
        "the reopen asks about the draft first: {}",
        said(&app)
    );
    assert!(app.has_unsaved_work(), "and nothing was dropped unasked");
    assert!(Store::current().is_trusted(&dir), "the trust is recorded");
}
