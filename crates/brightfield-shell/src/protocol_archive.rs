//! Open in Brightfield: the `brightfield://open` link a web page hands over, the Protocol archive
//! it names, and how that archive is checked and unpacked into the Protocols folder.
//!
//! **The link** is `brightfield://open?url=<address>`, where the address is an `https` URL on
//! `meridian.online` or one of its subdomains ([`check_link`]). Anything else is refused with a
//! sentence saying why, before anything is fetched.
//!
//! **The archive** is a `.zip` whose top level is one Protocol folder holding `arcform.yaml`. Its
//! entries are checked before a file is written ([`unpack`],
//! `an_archive_with_dotdot_or_absolute_entries_writes_nothing`): an entry may not be absolute,
//! hold `..`, or be a link, there are at most [`MAX_ENTRIES`] of them, and the sizes their headers declare add
//! up to at most [`MAX_BYTES`]. A Protocol whose sources, steps, database or data packages name a
//! path on the Mac it was made on, or a path outside its own folder, is refused naming that path,
//! since it could run on no other Mac. Then a folder is claimed in the Protocols folder, named
//! after the archive's Protocol (`-2` and so on on a clash), and the entries are written, counting
//! the bytes actually written and stopping at the same limit, since a header can understate a
//! size; a stopped unpack takes its folder away again.
//!
//! **The download** ([`Download`]) runs on a worker thread and follows redirects one at a time,
//! checking each address it is sent to as the link's own address was checked, so a redirect off
//! `meridian.online` is refused. The worker unpacks the archive too, so the window takes a
//! Protocol folder or a sentence.

use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use url::Url;

/// The link's scheme.
pub const SCHEME: &str = "brightfield";

/// The link's host: what it asks Brightfield to do.
pub const OPEN: &str = "open";

/// The site archives are downloaded from, with its subdomains.
pub const SITE: &str = "meridian.online";

/// The most entries an archive may hold.
pub const MAX_ENTRIES: usize = 10_000;

/// The most bytes an archive may unpack to, and the most a download may bring: 2 GiB.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The most redirects a download follows.
const MAX_REDIRECTS: usize = 5;

/// How long a connection may take to establish, as [`crate::remote`] allows.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How long a download waits for its next bytes before it gives up: silence, not the whole
/// download, so a slow archive still arrives, and a stalled one ends with a sentence rather than
/// holding the one download the window runs at a time.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The largest text file read into memory to be checked before anything is written.
const MAX_CHECKED_TEXT: u64 = 16 * 1024 * 1024;

/// The manifest that makes a folder a Protocol.
const MANIFEST: &str = "arcform.yaml";

/// The record of an own-files Protocol's sources.
const SOURCES_FILE: &str = ".sources.json";

/// The file name of a Frictionless data package.
const PACKAGE_FILE: &str = "datapackage.json";

/// How an address a download is sent to is checked: [`check_address`] for a link from a web page.
pub type Allowed = fn(&Url) -> Result<(), String>;

/// The limits an unpack is held to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most entries.
    pub entries: usize,
    /// The most bytes, declared or written.
    pub bytes: u64,
}

impl Limits {
    /// The limits a Protocol archive is held to.
    pub const ARCHIVE: Self = Self {
        entries: MAX_ENTRIES,
        bytes: MAX_BYTES,
    };
}

/// Check the address a link names: `https`, on `meridian.online` or one of its subdomains.
///
/// # Errors
///
/// A sentence saying why the address is refused.
pub fn check_address(address: &Url) -> Result<(), String> {
    if address.scheme() != "https" {
        return Err(format!(
            "{address} is not an https address, so Brightfield does not download it. Open in \
             Brightfield takes https addresses on {SITE}."
        ));
    }
    let host = address.host_str().unwrap_or_default().to_ascii_lowercase();
    let on_site = host == SITE
        || host
            .strip_suffix(SITE)
            .is_some_and(|sub| sub.len() > 1 && sub.ends_with('.'));
    if !on_site {
        let named = if host.is_empty() {
            "That address names no site".to_string()
        } else {
            format!("{host} is not {SITE}")
        };
        return Err(format!(
            "{named}, so Brightfield does not download from it. Open in Brightfield takes \
             addresses on {SITE} and its subdomains."
        ));
    }
    Ok(())
}

/// Check a `brightfield://open?url=<address>` link and answer the address it names.
///
/// # Errors
///
/// A sentence saying why the link is refused.
pub fn check_link(link: &str) -> Result<Url, String> {
    check_link_with(link, check_address)
}

/// [`check_link`], with the address checked by `allowed`, so a test can serve an archive from
/// this Mac.
///
/// # Errors
///
/// A sentence saying why the link is refused.
pub fn check_link_with(link: &str, allowed: Allowed) -> Result<Url, String> {
    let parsed =
        Url::parse(link).map_err(|_| format!("{link} is not a link Brightfield can read."))?;
    if parsed.scheme() != SCHEME || parsed.host_str() != Some(OPEN) {
        return Err(format!(
            "{link} is not a link Brightfield opens: it opens {SCHEME}://{OPEN}?url=<address> \
             links."
        ));
    }
    let addresses: Vec<String> = parsed
        .query_pairs()
        .filter(|(key, _)| key == "url")
        .map(|(_, value)| value.into_owned())
        .collect();
    let [address] = addresses.as_slice() else {
        return Err(format!(
            "{link} names {} addresses, and Brightfield opens a link that names one.",
            addresses.len()
        ));
    };
    let address = Url::parse(address)
        .map_err(|_| format!("{address}, which {link} names, is not an address."))?;
    allowed(&address)?;
    Ok(address)
}

/// Unpack the Protocol archive `zip` into a new folder in `into`, within [`Limits::ARCHIVE`], and
/// answer the folder. See the module's header for the checks.
///
/// # Errors
///
/// A sentence naming what was refused, with nothing left behind.
pub fn unpack(zip: &Path, into: &Path) -> Result<PathBuf, String> {
    unpack_within(zip, into, Limits::ARCHIVE)
}

/// [`unpack`], within `limits`.
///
/// # Errors
///
/// As [`unpack`].
pub fn unpack_within(zip: &Path, into: &Path, limits: Limits) -> Result<PathBuf, String> {
    let named = zip.file_name().map_or_else(
        || zip.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let file = std::fs::File::open(zip).map_err(|e| format!("{named} could not be read: {e}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("{named} is not a zip Brightfield can read: {e}"))?;
    let plan = plan(&mut archive, &named, limits)?;
    check_contents(&mut archive, &plan, &named)?;
    let (dir, _) = crate::protocols_dir::claim(into, &plan.top).map_err(|e| e.to_string())?;
    if let Err(why) = write_entries(&mut archive, &plan, &dir, &named, limits.bytes) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(why);
    }
    Ok(dir)
}

/// What an archive's entries are to become: the top folder's name, and each entry by index with
/// its path inside that folder.
struct Plan {
    top: String,
    entries: Vec<(usize, PathBuf, bool)>,
}

impl Plan {
    /// The index of the file entry at `path` inside the Protocol folder.
    fn file_at(&self, path: &Path) -> Option<usize> {
        self.entries
            .iter()
            .find(|(_, at, dir)| !dir && at == path)
            .map(|(i, _, _)| *i)
    }
}

/// Check every entry's name, kind and declared size, before anything is written.
fn plan(
    archive: &mut zip::ZipArchive<std::fs::File>,
    named: &str,
    limits: Limits,
) -> Result<Plan, String> {
    let nothing = "Nothing was unpacked.";
    if archive.len() > limits.entries {
        return Err(format!(
            "{named} holds {} entries, more than the {} an archive may hold. {nothing}",
            archive.len(),
            limits.entries
        ));
    }
    let mut declared: u64 = 0;
    let mut top: Option<String> = None;
    let mut manifest = false;
    let mut entries = Vec::with_capacity(archive.len());
    // Each file by its path in lower case: a Mac's file system ignores case, so two entries whose
    // paths differ only in case would write one file, the second over the first once it is checked.
    let mut files = std::collections::HashSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|e| format!("{named} could not be read: {e}"))?;
        let raw = entry.name().to_string();
        if entry.is_symlink() {
            return Err(format!(
                "{named} holds a link, {raw}, and an archive may hold files and folders only. \
                 {nothing}"
            ));
        }
        let escapes = raw.starts_with(['/', '\\'])
            || raw.split(['/', '\\']).any(|part| part == "..")
            || raw.as_bytes().get(1) == Some(&b':');
        let path = match entry.enclosed_name() {
            Some(path) if !escapes => path,
            _ => {
                return Err(format!(
                    "{named} holds {raw}, which points outside its folder. {nothing}"
                ))
            }
        };
        declared = declared.saturating_add(entry.size());
        if declared > limits.bytes {
            return Err(format!(
                "{named} would unpack to more than {}. {nothing}",
                crate::protocol::human_bytes(limits.bytes)
            ));
        }
        let mut parts = path.components().map(|c| c.as_os_str().to_string_lossy());
        let Some(first) = parts.next().map(|p| p.into_owned()) else {
            continue;
        };
        let rest: PathBuf = parts.map(|p| p.into_owned()).collect();
        let dir = entry.is_dir();
        if rest.as_os_str().is_empty() && !dir {
            return Err(format!(
                "{named} does not hold one Protocol folder: {raw} sits beside it. {nothing}"
            ));
        }
        match &top {
            Some(t) if *t != first => {
                return Err(format!(
                    "{named} holds more than one folder at its top ({t} and {first}), and a \
                     Protocol archive holds one. {nothing}"
                ))
            }
            Some(_) => {}
            None => top = Some(first),
        }
        if !dir && rest == Path::new(MANIFEST) {
            manifest = true;
        }
        if !dir && !files.insert(rest.to_string_lossy().to_lowercase()) {
            return Err(format!(
                "{named} holds two entries named {raw}, apart from their case, and a Mac keeps one \
                 file for both. {nothing}"
            ));
        }
        entries.push((index, rest, dir));
    }
    let top = top.filter(|_| manifest).ok_or_else(|| {
        format!(
            "{named} holds no folder with {MANIFEST} at its top, so it is not a Protocol archive. \
             {nothing}"
        )
    })?;
    if top.starts_with('.') {
        return Err(format!(
            "{named} names its Protocol folder {top}, which would be hidden in the Finder. \
             {nothing}"
        ));
    }
    Ok(Plan { top, entries })
}

/// Read the text entry at `path` inside the Protocol folder, if the archive holds one, up to
/// [`MAX_CHECKED_TEXT`].
fn read_text(
    archive: &mut zip::ZipArchive<std::fs::File>,
    plan: &Plan,
    path: &Path,
    named: &str,
) -> Result<Option<String>, String> {
    let Some(index) = plan.file_at(path) else {
        return Ok(None);
    };
    let entry = archive
        .by_index(index)
        .map_err(|e| format!("{named} could not be read: {e}"))?;
    let mut text = String::new();
    entry
        .take(MAX_CHECKED_TEXT)
        .read_to_string(&mut text)
        .map_err(|e| format!("{}, in {named}, could not be read: {e}", path.display()))?;
    Ok(Some(text))
}

/// Refuse a Protocol that names a path on the Mac it was made on, or one outside its own folder:
/// in its manifest's database, dotenv files and step SQL files, in the files its SQL reads (every
/// `.sql` file in the archive, however the manifest spells its path), in its record of sources,
/// and in its data packages.
fn check_contents(
    archive: &mut zip::ZipArchive<std::fs::File>,
    plan: &Plan,
    named: &str,
) -> Result<(), String> {
    let text = read_text(archive, plan, Path::new(MANIFEST), named)?
        .ok_or_else(|| format!("{named} holds no {MANIFEST}."))?;
    let manifest = arc::spec::Manifest::from_yaml_str(&text)
        .map_err(|e| format!("{named}'s {MANIFEST} is not a Protocol arc reads: {e}"))?;
    let root = Path::new("");
    let refuse = |what: &str, path: &str| -> Result<(), String> {
        if absolute_local(path) {
            return Err(format!(
                "{named}'s {what} names {path}, a place on the Mac it was made on, so it could run \
                 on no other. Nothing was unpacked."
            ));
        }
        if escapes(root, path) {
            return Err(format!(
                "{named}'s {what} names {path}, which is outside the Protocol's folder. Nothing \
                 was unpacked."
            ));
        }
        Ok(())
    };
    if let Some(db) = &manifest.db {
        refuse("database", db)?;
    }
    for dotenv in &manifest.dotenv {
        refuse("dotenv file", dotenv)?;
    }
    for step in &manifest.steps {
        if let Some(sql) = &step.sql {
            refuse(&format!("step {}", step.name), sql)?;
        }
    }
    let sql_files: Vec<PathBuf> = plan
        .entries
        .iter()
        .filter(|(_, path, dir)| {
            !dir && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("sql"))
        })
        .map(|(_, path, _)| path.clone())
        .collect();
    for sql in sql_files {
        let Some(text) = read_text(archive, plan, &sql, named)? else {
            continue;
        };
        for literal in file_literals(&text) {
            if !remote(&literal) {
                refuse(&format!("SQL file {}", sql.display()), &literal)?;
            }
        }
    }
    if let Some(text) = read_text(archive, plan, Path::new(SOURCES_FILE), named)? {
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("{named}'s {SOURCES_FILE} cannot be read: {e}"))?;
        for table in value
            .get("tables")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(file) = table.get("file").and_then(serde_json::Value::as_str) {
                refuse("record of its sources", file)?;
            }
        }
    }
    let packages: Vec<PathBuf> = plan
        .entries
        .iter()
        .filter(|(_, path, dir)| !dir && path.file_name().is_some_and(|n| n == PACKAGE_FILE))
        .map(|(_, path, _)| path.clone())
        .collect();
    for package in packages {
        let Some(text) = read_text(archive, plan, &package, named)? else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("{named}'s {} cannot be read: {e}", package.display()))?;
        let base = package.parent().unwrap_or(root);
        for resource in value
            .get("resources")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let paths: Vec<&str> = match resource.get("path") {
                Some(serde_json::Value::String(path)) => vec![path.as_str()],
                Some(serde_json::Value::Array(paths)) => {
                    paths.iter().filter_map(serde_json::Value::as_str).collect()
                }
                _ => Vec::new(),
            };
            for path in paths.into_iter().filter(|p| !remote(p)) {
                if absolute_local(path) {
                    return Err(format!(
                        "{named}'s data package {} names {path}, a place on the Mac it was made \
                         on, so it could run on no other. Nothing was unpacked.",
                        package.display()
                    ));
                }
                if escapes(base, path) {
                    return Err(format!(
                        "{named}'s data package {} names {path}, which is outside the \
                         Protocol's folder. Nothing was unpacked.",
                        package.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Write each entry under `dir`, counting the bytes written and stopping past `limit`.
fn write_entries(
    archive: &mut zip::ZipArchive<std::fs::File>,
    plan: &Plan,
    dir: &Path,
    named: &str,
    limit: u64,
) -> Result<(), String> {
    let mut written: u64 = 0;
    let mut buf = vec![0u8; 64 * 1024];
    for (index, path, is_dir) in &plan.entries {
        let target = dir.join(path);
        if *is_dir {
            std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let mut entry = archive
            .by_index(*index)
            .map_err(|e| format!("{named} could not be read: {e}"))?;
        let mut out =
            std::fs::File::create(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        loop {
            let n = entry.read(&mut buf).map_err(|e| {
                format!(
                    "{named} is damaged: {} could not be read: {e}. Nothing was kept.",
                    path.display()
                )
            })?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > limit {
                return Err(format!(
                    "{named} holds more than its headers say, and unpacking it stopped at {}. \
                     Nothing was kept.",
                    crate::protocol::human_bytes(limit)
                ));
            }
            out.write_all(&buf[..n])
                .map_err(|e| format!("{}: {e}", target.display()))?;
        }
    }
    Ok(())
}

/// Whether `text` names a remote address, such as `https://…` or `s3://…`, rather than a file.
fn remote(text: &str) -> bool {
    text.split_once("://").is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && !scheme.eq_ignore_ascii_case("file")
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Whether `text` names an absolute place on this Mac or another: `/…`, `~/…`, `file:…`, a drive
/// letter or a network share.
fn absolute_local(text: &str) -> bool {
    let path = text
        .strip_prefix("file://")
        .or_else(|| text.strip_prefix("file:"))
        .unwrap_or(text);
    let bytes = path.as_bytes();
    (path.starts_with('/') && !path.trim_start_matches('/').is_empty())
        || path == "~"
        || path.starts_with("~/")
        || path.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\'))
}

/// Whether `path`, read from the folder `base` inside a Protocol, reaches above the Protocol's
/// own folder by its `..` parts.
fn escapes(base: &Path, path: &str) -> bool {
    let mut depth = base.components().count();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => match depth.checked_sub(1) {
                Some(up) => depth = up,
                None => return true,
            },
            _ => depth += 1,
        }
    }
    false
}

/// The functions besides the `read_` ones whose string arguments name files.
const FILE_FUNCTIONS: [&str; 11] = [
    "sqlite_scan",
    "sqlite_attach",
    "parquet_scan",
    "parquet_metadata",
    "parquet_schema",
    "parquet_file_metadata",
    "glob",
    "sniff_csv",
    "st_read",
    "delta_scan",
    "iceberg_scan",
];

/// The words a file's literal follows in SQL: `FROM 'a.csv'`, `JOIN 'b.parquet'`,
/// `ATTACH 'c.duckdb'` and `ATTACH IF NOT EXISTS 'c.duckdb'`, `COPY t TO 'd.csv'`,
/// `LOAD 'e.duckdb_extension'`, and `IMPORT DATABASE 'f'` and `EXPORT DATABASE 'f'`.
const FILE_WORDS: [&str; 7] = ["from", "join", "attach", "exists", "to", "load", "database"];

/// The word pairs that end in one of [`FILE_WORDS`] and compare with a pattern rather than read a
/// file: `SIMILAR TO '/api/.*'` and `IS [NOT] DISTINCT FROM '/x'`.
const PATTERN_WORDS: [(&str, &str); 2] = [("similar", "to"), ("distinct", "from")];

/// The single-quoted literals in `sql` that name files: an argument of a function whose name
/// starts `read_` or is one of [`FILE_FUNCTIONS`], a list inside one, and a literal straight after
/// one of [`FILE_WORDS`] that is not one of [`PATTERN_WORDS`]. Other literals, such as a pattern
/// for `LIKE` or `regexp_replace`, are not files and are passed over, as are comments and quoted
/// identifiers.
fn file_literals(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    // For each open parenthesis, whether it holds a file function's arguments.
    let mut calls: Vec<bool> = Vec::new();
    // The last word read, in lower case, and whether it is the last thing read but spaces, so
    // `read_csv (` and `FROM '` are told from `x) (` and `= '`.
    let mut word = String::new();
    // The word before it, so a pattern after `SIMILAR TO` is told from a file after `COPY t TO`.
    let mut previous = String::new();
    let mut after_word = false;
    let mut in_word = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_alphanumeric() || c == '_' {
            if !in_word {
                previous = std::mem::take(&mut word);
            }
            word.extend(c.to_lowercase());
            in_word = true;
            after_word = true;
            i += 1;
            continue;
        }
        in_word = false;
        match c {
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    i += 1;
                }
                i += 2;
                continue;
            }
            '\'' | '"' => {
                let mut text = String::new();
                i += 1;
                while i < chars.len() {
                    if chars[i] == c {
                        if chars.get(i + 1) == Some(&c) {
                            text.push(c);
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
                if c == '\'' {
                    let in_call = calls.last().copied().unwrap_or(false);
                    let pattern = PATTERN_WORDS
                        .iter()
                        .any(|(before, last)| *before == previous && *last == word);
                    let after_file_word =
                        after_word && FILE_WORDS.contains(&word.as_str()) && !pattern;
                    if in_call || after_file_word {
                        out.push(text);
                    }
                }
                after_word = false;
            }
            c if c.is_whitespace() => {}
            '(' => {
                calls.push(
                    after_word
                        && (word.starts_with("read_") || FILE_FUNCTIONS.contains(&word.as_str())),
                );
                after_word = false;
            }
            ')' => {
                calls.pop();
                after_word = false;
            }
            _ => after_word = false,
        }
        i += 1;
    }
    out
}

/// An archive download running on a worker thread, unpacked on the same worker once it has
/// arrived.
pub struct Download {
    address: Url,
    meter: Arc<crate::remote::Meter>,
    outcome: Receiver<Result<PathBuf, String>>,
    done: bool,
}

impl Download {
    /// Download the archive at `address` and unpack it into `into`, on a thread of its own. Each
    /// address a redirect sends it to is checked by `allowed` before it is fetched. `wake` is
    /// called as bytes arrive and once at the end.
    #[must_use]
    pub fn begin(
        address: Url,
        allowed: Allowed,
        into: PathBuf,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        Self::begin_within(address, allowed, into, READ_TIMEOUT, wake)
    }

    /// [`Self::begin`], giving up after `silence` with no bytes, so a test can stall a download
    /// without waiting [`READ_TIMEOUT`].
    #[must_use]
    pub fn begin_within(
        address: Url,
        allowed: Allowed,
        into: PathBuf,
        silence: Duration,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        let meter = Arc::new(crate::remote::Meter::default());
        let (tx, outcome) = mpsc::channel();
        let worker_meter = Arc::clone(&meter);
        let worker_address = address.clone();
        std::thread::spawn(move || {
            let result = download_and_unpack(
                &worker_address,
                allowed,
                &into,
                silence,
                &worker_meter,
                &wake,
            );
            tx.send(result).ok();
            wake();
        });
        Self {
            address,
            meter,
            outcome,
            done: false,
        }
    }

    /// The address the link named.
    #[must_use]
    pub fn address(&self) -> &Url {
        &self.address
    }

    /// The download's answer, once there is one: the unpacked Protocol folder, or a sentence.
    pub fn take(&mut self) -> Option<Result<PathBuf, String>> {
        if self.done {
            return None;
        }
        match self.outcome.try_recv() {
            Ok(result) => {
                self.done = true;
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.done = true;
                Some(Err(format!(
                    "Downloading {} stopped without an answer.",
                    self.address
                )))
            }
        }
    }

    /// What the window says while the download runs: the bytes received, against the length the
    /// server declared where it declared one.
    #[must_use]
    pub fn readout(&self) -> String {
        crate::remote::readout(self.meter.received(), self.meter.declared())
    }
}

/// Download the archive at `address` into a scratch folder and unpack it into `into`.
fn download_and_unpack(
    address: &Url,
    allowed: Allowed,
    into: &Path,
    silence: Duration,
    meter: &crate::remote::Meter,
    wake: &(impl Fn() + Send + 'static),
) -> Result<PathBuf, String> {
    let scratch = tempfile::Builder::new()
        .prefix("brightfield-archive-")
        .tempdir()
        .map_err(|e| format!("could not make a place to download into: {e}"))?;
    let name = address
        .path_segments()
        .and_then(|mut s| s.next_back().map(str::to_string))
        .filter(|n| {
            !n.is_empty()
                && n.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        })
        .unwrap_or_else(|| "protocol.zip".to_string());
    let file = scratch.path().join(name);
    fetch(address, allowed, &file, silence, meter, wake)?;
    unpack(&file, into)
}

/// Fetch `address` into `file`, following at most [`MAX_REDIRECTS`] redirects, each checked by
/// `allowed`, and giving up after `silence` with no bytes.
fn fetch(
    address: &Url,
    allowed: Allowed,
    file: &Path,
    silence: Duration,
    meter: &crate::remote::Meter,
    wake: &(impl Fn() + Send + 'static),
) -> Result<(), String> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(silence)
        .build();
    let mut at = address.clone();
    for _ in 0..=MAX_REDIRECTS {
        let response = match agent.get(at.as_str()).call() {
            Ok(response) => response,
            Err(ureq::Error::Status(code, _)) => {
                return Err(format!(
                    "{at} answered {code}, so there was nothing to open."
                ))
            }
            Err(e) => return Err(format!("Brightfield could not download {at}: {e}")),
        };
        if (300..400).contains(&response.status()) {
            let location = response.header("Location").ok_or_else(|| {
                format!("{at} redirected without saying where to, so nothing was downloaded.")
            })?;
            let next = at
                .join(location)
                .map_err(|_| format!("{at} redirected to {location}, which is not an address."))?;
            allowed(&next).map_err(|why| format!("{at} redirected to {next}. {why}"))?;
            at = next;
            continue;
        }
        if let Some(total) = response
            .header("Content-Length")
            .and_then(|v| v.trim().parse::<u64>().ok())
        {
            if total > MAX_BYTES {
                return Err(format!(
                    "{at} is {}, more than the {} an archive may be.",
                    crate::protocol::human_bytes(total),
                    crate::protocol::human_bytes(MAX_BYTES)
                ));
            }
            meter.declare(total);
        }
        let mut body = response.into_reader();
        let mut out =
            std::fs::File::create(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut buf = vec![0u8; 64 * 1024];
        let mut received: u64 = 0;
        loop {
            let n = body
                .read(&mut buf)
                .map_err(|e| format!("Brightfield could not download {at}: {e}"))?;
            if n == 0 {
                break;
            }
            received += n as u64;
            if received > MAX_BYTES {
                return Err(format!(
                    "{at} is more than the {} an archive may be.",
                    crate::protocol::human_bytes(MAX_BYTES)
                ));
            }
            out.write_all(&buf[..n])
                .map_err(|e| format!("{}: {e}", file.display()))?;
            meter.add_received(n as u64);
            wake();
        }
        return out.flush().map_err(|e| format!("{}: {e}", file.display()));
    }
    Err(format!(
        "{address} redirected more than {MAX_REDIRECTS} times, so nothing was downloaded."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_literal_is_read_past_comments_identifiers_and_patterns() {
        let sql = "-- reads '/etc/passwd'\nSELECT \"it's\" FROM read_csv('data/a.csv') /* '/x' */ \
                   WHERE s = 'it''s' AND p LIKE '/api/%' AND regexp_replace(q, '/+$', '') = '\\\\'";
        assert_eq!(file_literals(sql), vec!["data/a.csv"]);
        assert_eq!(
            file_literals(
                "SELECT * FROM 'b.parquet' JOIN read_parquet(['c.parquet', 'd.parquet'], \
                 hive_partitioning = true) USING (k); COPY t TO 'e.csv'; ATTACH 'f.duckdb' AS f"
            ),
            vec!["b.parquet", "c.parquet", "d.parquet", "e.csv", "f.duckdb"]
        );
        assert_eq!(
            file_literals("SELECT * FROM READ_CSV ('g.csv', delim = ',')"),
            vec!["g.csv", ","]
        );
    }

    #[test]
    fn a_path_is_absolute_by_its_start_and_remote_by_its_scheme() {
        for absolute in [
            "/Users/me/a.csv",
            "~/a.csv",
            "file:///tmp/a.csv",
            "C:\\data\\a.csv",
            "\\\\server\\share",
        ] {
            assert!(absolute_local(absolute), "{absolute}");
        }
        for not in ["/", "data/a.csv", "a.csv", "https://meridian.online/a.csv"] {
            assert!(!absolute_local(not), "{not}");
        }
        assert!(remote("https://meridian.online/a.csv"));
        assert!(remote("s3://bucket/a.parquet"));
        assert!(!remote("file:///tmp/a.csv"));
        assert!(!remote("data/a.csv"));
    }

    #[test]
    fn a_path_escapes_by_climbing_past_its_base() {
        assert!(!escapes(Path::new(""), "data/a.csv"));
        assert!(!escapes(Path::new(""), "data/../a.csv"));
        assert!(escapes(Path::new(""), "../a.csv"));
        assert!(escapes(Path::new(""), "data/../../a.csv"));
        assert!(!escapes(Path::new("packages/a"), "../../data/a.csv"));
        assert!(escapes(Path::new("packages/a"), "../../../a.csv"));
    }
}
