//! Opening a data file the user chose: a CSV or a Parquet, straight to a
//! queryable table.
//!
//! Until this existed the app could be pointed at a chart spec or at a
//! protocol manifest and at nothing else. There was no file dialog anywhere in
//! the tree and no dependency that could raise one, so the only way to see your
//! own data was to write a spec describing it first — which is asking someone
//! to author a document about a file before they are allowed to look at the
//! file.
//!
//! # What opening a file means here
//!
//! The file becomes a **DuckDB view in a live session**, exactly as a `file:`
//! source in a hand-written spec would: [`open`] synthesises a spec whose one
//! `data:` entry is the chosen path, loads it live, and hands back the session
//! and its first composition. Nothing is read into Rust memory on the way — the
//! Data pane beside the chart then reads `SELECT * FROM <view>` back through
//! the engine's windowed row seam, so a Parquet larger than memory opens as
//! readily as a small CSV.
//!
//! **What is drawn on it is [`crate::dashboard`]'s decision, not this
//! module's**: a tile per column, each chosen from what that column means. This
//! module owns the route — what may be opened, how the schema is read, and
//! where the generated spec is written for the reader — and holds no opinion
//! about pictures.
//!
//! # Why the schema is read before the spec is written
//!
//! A spec has to name columns, and nobody knows the columns of a file they have
//! not opened. So [`open`] loads twice: once over a **root-less** spec that
//! declares only the source (no plot, no marks — nothing executes), which is
//! what makes `profile_sources` able to answer what the columns are and what
//! they hold; and once over the spec that first answer lets it write. The cost
//! is a profile pass over the table before the first picture, and it buys a
//! dashboard chosen from the data rather than guessed at.
//!
//! # Why a URL is refused
//!
//! This opens a file on this machine. DuckDB will happily bind a view over an
//! `https://` Parquet through `httpfs`, so a path box that passed the string on
//! would fetch instead — and succeed, with nothing red anywhere. [`accept`]
//! refuses anything carrying a URL scheme, before the engine is involved at
//! all, and says so.
//!
//! # Why a name the reader would take as a pattern is refused
//!
//! A chosen path crosses two languages on its way to DuckDB: it is written into
//! a YAML scalar, and it is handed to `read_csv` / `read_parquet`, which resolve
//! it as a **glob**. Each has characters that mean something other than
//! themselves, and in both the failure is the same and it is silent — the file
//! that opens is not the file that was picked, under a window titled with the
//! picked name.
//!
//! Measured against this build's DuckDB (`SELECT version()` reports `v1.5.2`),
//! with `sales1.csv` sitting beside `sales[1].csv`: `read_csv` on the second
//! path returns the first file's rows. `glob()` is what decides it, and the
//! danger is exactly *`glob(p)` matching a non-empty set that is not `{p}`* —
//! when nothing matches, DuckDB falls back to the literal path, which is why
//! most punctuation is harmless. There is no way to turn this off. Measured on
//! the same build: no reader keyword disables it, the list form
//! `read_csv([…])` still globs, a `file://` prefix still globs, and `\[` is not
//! an escape — it matches nothing rather than matching the bracket.
//!
//! So [`accept`] **refuses** these names rather than escaping them, and the
//! reason is that the escape is not ours to rely on. The one spelling that does
//! work today, rewriting `[` as the character class `[[]`, is undocumented
//! dialect: if a DuckDB bump changed it the app would go back to opening the
//! wrong file silently, which is the one outcome that is not allowed here.
//! Escaping in the emitter was rejected for a second reason — `brightfield-sql`
//! is shared, and a glob in a hand-written spec's `file:` is a feature there,
//! so that seam cannot tell a pattern from a name. A refusal naming the file
//! and the character is the answer that cannot silently be wrong, and it is the
//! same answer a URL gets.
//!
//! The control-character half is the YAML side: a line break inside a
//! single-quoted scalar is **folded to a space** by the parser, so
//! `sales<LF>2026.csv` would come back as `sales 2026.csv` and open that file
//! instead. It is refused in the same place and for the same reason.

use std::path::{Path, PathBuf};

use brightfield_engine::{ColumnProfile, Engine, LoadOptions, ProfileOutcome};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::{parse_spec, Format};

use crate::dashboard::Dashboard;
use crate::pipeline::{Composed, LiveDashboard};

/// The file extensions this action opens, lowercased and without the dot.
///
/// The engine's source emitter dispatches on exactly these spellings for the
/// two formats a data file arrives in; `tsv` rides the same reader as `csv`.
/// JSON and the DuckDB/DuckLake attach forms are deliberately absent: they are
/// emitted by the same dispatch but they are not what "open a data file" means
/// to someone with a table in front of them, and each carries its own
/// behaviour (a whole catalog attaches, a database file has many tables) that
/// this one-view path has no answer for.
pub const OPENABLE_EXTENSIONS: &[&str] = &["csv", "tsv", "parquet"];

/// The `data:` key the chosen file is declared under, and therefore the name of
/// the DuckDB view it becomes.
///
/// A fixed name rather than one derived from the file: a view name is a SQL
/// identifier that the synthesised spec, the emitted DDL and every mark query
/// have to agree on, and deriving it from a user-chosen file name means
/// deriving it from arbitrary bytes. The user never sees this string — the
/// window is titled from the file name.
pub const SOURCE: &str = "opened";

/// Everything opening a file produced: the live session over it, the first
/// composition, the dashboard that was chosen for it, and where the generated
/// spec was written for the reader.
///
/// A struct rather than a tuple because the fourth thing is optional and the
/// third is what a caller asks questions of — which tiles, which columns were
/// left out, and why.
pub struct OpenedFile {
    /// The session holding the file as a DuckDB view.
    pub live: LiveDashboard,
    /// The first composition over it.
    pub composed: Composed,
    /// The dashboard [`crate::dashboard::Dashboard::of`] chose: a tile per
    /// column, and the columns it declined.
    pub dashboard: Dashboard,
    /// Where the generated spec was written so the editor pane can open it —
    /// `None` when the scratch write failed, which is not a reason to refuse a
    /// file that opened.
    pub spec_file: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// What may be opened
// ---------------------------------------------------------------------------

/// The characters this build refuses in a chosen path because DuckDB's file
/// readers resolve a path as a **glob**, where each of them means something
/// other than itself: `*` and `?` are wildcards, `[…]` is a character class and
/// `{…}` is an alternation.
///
/// Wider than what this DuckDB was measured mis-resolving, deliberately. On
/// `v1.5.2` only `*`, `?` and a `[` that closes into a class were observed
/// selecting a different file; `]`, `{` and `}` resolved to themselves. They
/// are refused anyway because they are the closing halves of the same two
/// constructs, and the cost of the two errors is not symmetric — refusing a
/// name that would have worked is a sentence the user can act on, while
/// accepting one that stops working on a DuckDB bump is a wrong table nobody
/// sees. `every_character_this_duckdb_reads_as_a_pattern_is_refused` in
/// `tests/data_file.rs` asks DuckDB itself, so a bump that widens the dialect
/// past this list reddens rather than shipping.
const PATTERN_CHARACTERS: &[char] = &['*', '?', '[', ']', '{', '}'];

/// The chosen location as a path this build will open, or the words to show
/// the user for refusing it.
///
/// Five refusals, each with its own sentence, and all five happen **before**
/// the engine exists — a refusal that arrived as a DuckDB binder error would
/// name a SQL view rather than the thing the user picked.
///
/// What the last two defend is the invariant the rest of this module rests on:
/// **the path that reaches DuckDB names the file the user chose.** See the
/// module docs for the two measured ways it can stop being true.
///
/// # Errors
///
/// A URL rather than a local path; a path with no extension or an extension
/// this build does not read; a path the reader would resolve as a pattern
/// (see `PATTERN_CHARACTERS`) or one carrying a control character; or a path
/// naming something that is not a readable file.
pub fn accept(chosen: &str) -> Result<PathBuf, String> {
    if let Some(scheme) = url_scheme(chosen) {
        return Err(format!(
            "{chosen}: Brightfield opens files on this machine, and `{scheme}:` \
             is a URL. Download the file and open it from disk."
        ));
    }
    let path = PathBuf::from(chosen);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match extension {
        Some(ext) if OPENABLE_EXTENSIONS.contains(&ext.as_str()) => {}
        Some(ext) => {
            return Err(format!(
                "{chosen}: Brightfield opens {}; this is a .{ext}.",
                openable_prose()
            ))
        }
        None => {
            return Err(format!(
                "{chosen}: Brightfield opens {}, and this path names no file \
                 type at all.",
                openable_prose()
            ))
        }
    }
    if let Some(found) = chosen.chars().find(|c| PATTERN_CHARACTERS.contains(c)) {
        return Err(format!(
            "{chosen}: Brightfield cannot open this path, because `{found}` is \
             pattern syntax to the reader underneath — it would match the name \
             rather than read it, and could open a different file. Rename the \
             file without it, or move it to a folder whose name has none."
        ));
    }
    if chosen.chars().any(char::is_control) {
        return Err(format!(
            "{}: Brightfield cannot open this path, because it contains a \
             control character — a line break or a similar invisible one — and \
             there is no way to write that into a spec without it naming a \
             different file. Rename the file without it.",
            chosen.escape_debug()
        ));
    }
    if !path.is_file() {
        return Err(format!("{chosen}: there is no file there to open."));
    }
    Ok(path)
}

/// `.csv, .tsv and .parquet files`, written from the one list so the sentence
/// cannot describe a set this build does not read.
fn openable_prose() -> String {
    let dotted: Vec<String> = OPENABLE_EXTENSIONS
        .iter()
        .map(|e| format!(".{e}"))
        .collect();
    match dotted.split_last() {
        Some((last, rest)) if !rest.is_empty() => {
            format!("{} and {last} files", rest.join(", "))
        }
        Some((last, _)) => format!("{last} files"),
        None => "no files".to_string(),
    }
}

/// The URL scheme `chosen` carries, if it carries one.
///
/// A scheme is `[A-Za-z][A-Za-z0-9+.-]*` before a `:`, per RFC 3986 — with one
/// narrowing that is not pedantry: a **single-letter** scheme is not treated as
/// one, because `C:\Users\…` is a Windows path and refusing it as a URL would
/// be a worse answer than opening it. Two letters is the shortest real scheme
/// in use, and no drive letter is two characters.
///
/// Both the `scheme://host/…` and the bare `scheme:…` forms are caught: the
/// second is how DuckDB spells a DuckLake catalog, and it reaches the network
/// exactly as the first does.
fn url_scheme(chosen: &str) -> Option<&str> {
    let colon = chosen.find(':')?;
    let scheme = &chosen[..colon];
    if scheme.len() < 2 {
        return None;
    }
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
        return None;
    }
    Some(scheme)
}

// ---------------------------------------------------------------------------
// The synthesised spec
// ---------------------------------------------------------------------------

/// The root-less spec that declares `path` as a source and nothing else.
///
/// No `plot:`, so no mark and no query executes when it loads — the source's
/// DDL is a `CREATE OR REPLACE VIEW`, which for both a CSV and a Parquet reads
/// metadata rather than the table. This is what the schema is read through.
#[must_use]
pub fn source_spec(path: &Path) -> String {
    format!(
        "data:\n  {SOURCE}:\n    file: {}\n",
        yaml_quoted(&path.to_string_lossy())
    )
}

/// `value` as a YAML single-quoted scalar.
///
/// Single-quoted rather than double-quoted because the only escape a
/// single-quoted YAML scalar has is a doubled quote — no backslashes, no
/// interpretation — so a Windows path or a column name full of punctuation
/// survives verbatim.
///
/// What single-quoting cannot carry is a **line break**: YAML folds one inside
/// a quoted scalar to a space, so a value holding one would parse back as a
/// different string. Two guards keep such a value from reaching here, and this
/// function is correct only because both do — `accept`'s control-character
/// refusal for a path, and `chart_kinds::fields_of`'s name check for a column,
/// which is what stops an unwritable name reaching a chart kind's builder.
fn yaml_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// What the window is titled for an opened file: its file name, which is what
/// the person who picked it was looking at — not the absolute path they never
/// typed.
pub(crate) fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

/// Open `chosen` as a live table: the session holding the file as a DuckDB
/// view, the first composition over it, **the dashboard that was chosen for
/// it** — a tile per column — and the file the generated spec was written to.
///
/// The dashboard is [`crate::dashboard::Dashboard::of`]'s, which is where the
/// per-column choice and the layout live. This function's own job is the two
/// loads either side of it: the root-less one that answers what the columns
/// are, and the real one over the spec that answer let it write.
///
/// # Errors
///
/// Every failure carries the chosen path and the reason, because the window
/// raises this string as a banner and a banner that says only "could not open"
/// is a blank frame with a sentence on it. The reasons are: the location was
/// refused (see `accept`); DuckDB would not read the file, in which case its
/// own words come through; or no column in the table admits a tile — and then
/// the sentence names each column and why, because "nothing to draw" about a
/// file the user can see the contents of is not an answer.
pub fn open(chosen: &str) -> Result<OpenedFile, String> {
    let path = accept(chosen)?;
    let columns = columns_of(&path)?;
    let dashboard = Dashboard::of(&path, &columns);
    if dashboard.tiles().is_empty() {
        let left: Vec<String> = dashboard
            .omitted()
            .iter()
            .map(|o| format!("{}: {}", o.column, o.because))
            .collect();
        return Err(format!(
            "{}: opened, but there is nothing here to draw — no column in it \
             admits a picture this build can make. {}",
            path.display(),
            if left.is_empty() {
                "It declares no columns at all.".to_string()
            } else {
                left.join("; ")
            }
        ));
    }
    let spec = dashboard.to_spec();
    let mut live =
        LiveDashboard::load_str(&spec, None).map_err(|e| format!("{}: {e}", path.display()))?;
    let composed = live
        .present()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let spec_file = write_spec_file(&path, &spec);
    Ok(OpenedFile {
        live,
        composed,
        dashboard,
        spec_file,
    })
}

/// The directory generated specs are written to, per process.
///
/// The temp directory rather than beside the data file: opening a file to look
/// at it must not write into the folder it came from. Per process rather than a
/// fixed name, so two runs cannot edit each other's copy and nothing inherits a
/// directory another user created.
fn spec_scratch_dir() -> PathBuf {
    std::env::temp_dir().join(format!("brightfield-generated-{}", std::process::id()))
}

/// Write `spec` where the editor pane can open it, and hand back the path.
///
/// **The bytes are the bytes that composed the picture**, not a re-rendering of
/// them — which is what makes the file evidence rather than documentation: a
/// reader comparing the chart to the spec is comparing it to what ran.
///
/// `None` on any failure. A scratch write that did not land is a spec the reader
/// cannot open; it is not a reason to refuse a file that opened, so the caller
/// carries on without one.
fn write_spec_file(data: &Path, spec: &str) -> Option<PathBuf> {
    let dir = spec_scratch_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let stem = data.file_stem().unwrap_or(data.as_os_str());
    let path = dir.join(stem).with_extension("yaml");
    std::fs::write(&path, spec).ok()?;
    Some(path)
}

/// The profiled columns of `path`, read through a root-less spec.
///
/// # Errors
///
/// The engine's own words, prefixed with the path — this is where a file that
/// is not really a Parquet, or a CSV whose rows do not line up, is caught, and
/// DuckDB's message is the whole of what the reader needs.
fn columns_of(path: &Path) -> Result<Vec<ColumnProfile>, String> {
    let spec = source_spec(path);
    let parsed = parse_spec(&spec, Format::Yaml)
        .map_err(|e| format!("{}: parse error: {e}", path.display()))?;
    let analysis = analyse_spec(&parsed.spec)
        .map_err(|e| format!("{}: analysis error: {e}", path.display()))?;
    let load = Engine::new()
        .load_spec_with(parsed.spec, analysis, None, &LoadOptions::packaged())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let profile = load
        .session
        .profile_sources()
        .into_iter()
        .find(|p| p.name == SOURCE)
        .ok_or_else(|| {
            format!(
                "{}: the engine loaded the file but reported no source to \
                 profile.",
                path.display()
            )
        })?;
    match profile.outcome {
        ProfileOutcome::Profiled { columns, .. } => Ok(columns),
        ProfileOutcome::Failed(reason) => Err(format!("{}: {reason}", path.display())),
        ProfileOutcome::Unsupported => Err(format!(
            "{}: this build cannot read the schema of that source.",
            path.display()
        )),
    }
}

/// Ask the operating system for a data file, blocking until the user chooses
/// or cancels.
///
/// The one call in this crate that opens a window nobody laid out, and the one
/// thing here no headless test drives — everything a test needs to know about
/// opening a file is decided by `accept`, `first_look` and `open`, none of
/// which needs a dialog. The dialog's own job is to return a path.
///
/// Filtered to the extensions `accept` will take, so the common refusal never
/// has to be shown: a picker that offers a `.txt` and then declines it is a
/// picker arguing with itself.
#[must_use]
pub fn pick() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open a data file")
        .add_filter("Data files", OPENABLE_EXTENSIONS)
        .pick_file()
}

// ---------------------------------------------------------------------------
// Unit tests — the pure decisions. The engine-backed path is exercised in
// `tests/data_file.rs` against real files on disk.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_workbench::registry::Field;

    /// Every URL form a path box realistically receives is refused, and the
    /// refusal says the word — what this defends is that the box opens a file
    /// on this machine, so "it happens to fail later" is not the same answer.
    #[test]
    fn a_url_is_refused_by_scheme_not_by_hostname() {
        for url in [
            "https://openlake.meridian.online/edgar_gleif.parquet",
            "http://example.com/data.csv",
            "s3://bucket/key.parquet",
            "gs://bucket/key.parquet",
            "ducklake:https://example.com/catalog.ducklake",
            "file:///Users/someone/data.csv",
        ] {
            let refusal = accept(url).expect_err("a URL is not a local file");
            assert!(
                refusal.contains("URL"),
                "the refusal of {url} has to say what it refused: {refusal}"
            );
            assert!(
                refusal.contains(url),
                "the refusal of {url} has to name it: {refusal}"
            );
        }
    }

    /// …and a Windows drive letter is not a URL scheme. The single-letter
    /// narrowing is the whole of what stops `C:\data.csv` reading as one.
    #[test]
    fn a_drive_letter_is_not_a_scheme() {
        assert_eq!(url_scheme("C:\\Users\\hugh\\data.csv"), None);
        assert_eq!(url_scheme("/Users/hugh/data.csv"), None);
        assert_eq!(url_scheme("data.csv"), None);
        assert_eq!(url_scheme("https://x/y.csv"), Some("https"));
        assert_eq!(url_scheme("ducklake:x"), Some("ducklake"));
    }

    /// An extension this build does not read is refused by name, and the
    /// sentence lists what it does read — written from the one list, so it
    /// cannot describe a set that does not exist.
    #[test]
    fn an_unreadable_extension_is_refused_with_the_list_it_is_not_on() {
        let refusal = accept("/tmp/notes.txt").expect_err(".txt is not openable");
        assert!(refusal.contains(".txt"), "{refusal}");
        for ext in OPENABLE_EXTENSIONS {
            assert!(
                refusal.contains(&format!(".{ext}")),
                "the refusal has to name .{ext} as openable: {refusal}"
            );
        }
    }

    /// A path the reader would resolve as a pattern is refused by name and by
    /// character, and the refusal happens without touching the disk — these
    /// paths are not written anywhere, so a check that needed the file to exist
    /// would be gating the wrong thing.
    ///
    /// The sentence has to carry the character, because "rename it" is useless
    /// advice when the user cannot see which byte is the problem.
    #[test]
    fn a_path_the_reader_would_take_as_a_pattern_is_refused_by_character() {
        for (path, offender) in [
            ("/tmp/sales[1].csv", '['),
            ("/tmp/sales].csv", ']'),
            ("/tmp/sales*.csv", '*'),
            ("/tmp/sales?.csv", '?'),
            ("/tmp/sales{a,b}.csv", '{'),
            ("/tmp/sales}.csv", '}'),
            // A folder is resolved by the same glob the file name is.
            ("/tmp/quarter[1]/sales.csv", '['),
        ] {
            let refusal = accept(path).unwrap_err_or_else_message(path);
            assert!(
                refusal.contains(path),
                "the refusal must name it: {refusal}"
            );
            assert!(
                refusal.contains(offender),
                "the refusal of {path} must name `{offender}`: {refusal}"
            );
            assert!(
                refusal.contains("pattern"),
                "the refusal of {path} must say why: {refusal}"
            );
        }
    }

    /// `expect_err` with a sentence naming the path, so a regression reads as
    /// "this path opened" rather than as an `unwrap` backtrace.
    trait RefusalExt {
        fn unwrap_err_or_else_message(self, path: &str) -> String;
    }

    impl RefusalExt for Result<PathBuf, String> {
        fn unwrap_err_or_else_message(self, path: &str) -> String {
            match self {
                Err(message) => message,
                Ok(accepted) => panic!(
                    "{path} is pattern syntax to the reader underneath and must \
                     not be accepted — it was, as {}",
                    accepted.display()
                ),
            }
        }
    }

    /// …and a path carrying a control character is refused too, with the
    /// character shown escaped rather than raw — a banner holding a real line
    /// break is a banner that does not read as one line.
    #[test]
    fn a_path_with_a_control_character_is_refused_and_shown_escaped() {
        let refusal = accept("/tmp/sales\n2026.csv").expect_err("a line break is not openable");
        assert!(
            refusal.contains("/tmp/sales\\n2026.csv"),
            "the refusal names the path with the control character escaped: {refusal}"
        );
        assert!(
            !refusal.contains('\n'),
            "…and holds no raw line break of its own: {refusal:?}"
        );
        assert!(refusal.contains("control character"), "{refusal}");
        assert!(accept("/tmp/sales\r2026.csv").is_err());
        assert!(accept("/tmp/sales\t2026.csv").is_err());
    }

    /// An ordinary path with punctuation that means nothing to either language
    /// is NOT refused by these two checks — they are narrow on purpose, and a
    /// guard that turned away `Hugh's data (final).csv` would be a worse answer
    /// than the defect it prevents.
    ///
    /// Asserted by the message, because a path that does not exist is refused
    /// by `is_file` at the end regardless: what is gated is *which* sentence.
    #[test]
    fn ordinary_punctuation_is_not_mistaken_for_a_pattern() {
        for path in [
            "/tmp/Hugh's data (final).csv",
            "/tmp/sales+2026.csv",
            "/tmp/sales@2026 #2.csv",
            "/tmp/100% of sales, £ & $.csv",
            "C:\\Users\\hugh\\sales.csv",
        ] {
            let refusal = accept(path).expect_err("none of these exist on disk");
            assert!(
                refusal.contains("there is no file there to open"),
                "{path} must reach the last refusal, not a pattern one: {refusal}"
            );
        }
    }

    /// **Every kind the registry ships** parses under this module's header and
    /// names the file's own view as its source — not the two that happened to
    /// exist when this was written.
    ///
    /// Stated over the registry rather than over a list of shapes because that
    /// is the only form of the claim a kind added later has to satisfy: the
    /// Data pane tabulates `SELECT * FROM <the first mark's source>`, so a kind
    /// pointing its mark at a rolled-up view would put summary rows in the grid
    /// where the user is entitled to their file, and no assertion written
    /// against `rectY` and `cell` would notice.
    #[test]
    fn every_kind_reads_the_files_own_view() {
        let path = PathBuf::from("/tmp/t.parquet");
        for kind in crate::chart_kinds::registry().kinds() {
            let fields: Vec<Field> = kind
                .slots
                .iter()
                .filter(|slot| slot.required)
                .enumerate()
                .map(|(i, slot)| Field::new(format!("c{i}"), slot.accepts[0]))
                .collect();
            let binding = kind.bind(&fields).expect("a kind binds its own slots");
            let block = kind
                .spec(&binding, &kind.options())
                .expect("a kind builds from its own binding");
            let spec = format!("{}{block}", source_spec(&path));
            let parsed = parse_spec(&spec, Format::Yaml)
                .unwrap_or_else(|e| panic!("{}: does not parse: {e}\n{spec}", kind.id));
            let marks = brightfield_sql::collect_marks(&parsed.spec);
            assert!(!marks.is_empty(), "{}: declares no mark", kind.id);
            for mark in marks {
                let from = format!("{:?}", mark.data);
                assert!(
                    from.contains(SOURCE),
                    "{}: a mark must read the file's own view, not a rolled-up \
                     one: {from}",
                    kind.id
                );
            }
        }
    }

    /// **The spec the reader opens is the spec that ran**, byte for byte.
    ///
    /// The whole of what makes a generated dashboard inspectable rather than
    /// opaque is that the file is a copy of the source the picture was composed
    /// from. A writer that re-rendered the spec — reordered a key, dropped the
    /// comments that say why each tile is the tile it is — would produce a
    /// document that reads plausibly and is not evidence of anything.
    #[test]
    fn the_written_spec_is_the_source_that_composed_the_picture() {
        let data = PathBuf::from("/tmp/bf-written-spec/readings.csv");
        let spec = "meta:\n  title: 'readings.csv'\n# why this tile\nplot: []\n";
        let written = write_spec_file(&data, spec).expect("the scratch write lands");
        assert_eq!(
            std::fs::read_to_string(&written).expect("the file reads back"),
            spec
        );
        assert_eq!(
            written.extension().and_then(|e| e.to_str()),
            Some("yaml"),
            "the editor opens it as a spec, so it has to be named like one: {}",
            written.display()
        );
        assert!(
            written.starts_with(std::env::temp_dir()),
            "a file opened to be looked at must not write beside itself: {}",
            written.display()
        );
        std::fs::remove_file(&written).ok();
    }
}
