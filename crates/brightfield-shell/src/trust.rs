//! Which Protocols may run: the trust store.
//!
//! A Protocol's steps are SQL, and SQL can read and write any file on the Mac, so a Protocol
//! Brightfield did not make opens restricted: its spine, steps and sources are shown and nothing
//! runs until the analyst trusts it. Trust is a record in this store, a file in the config folder
//! beside the Preferences file, never something read from the folder itself: no marker, layout or
//! location inside a Protocol's folder can make it trusted.
//!
//! A record names a Protocol, or a folder that trusts every Protocol beneath it, including ones
//! added later. Each record keeps the folder's path and its identity on disk: the volume, the
//! file number and the moment the folder was made. A folder moved in the Finder keeps its
//! identity, so a Protocol Brightfield made is still trusted when it is found somewhere else, and
//! a folder made where a trusted one used to be is a different folder and is not.
//!
//! The store also remembers where a restricted Protocol came from (the host of the link it was
//! downloaded by, or the archive it was unpacked from), so the banner can name it on a later open.
//! That note grants nothing.
//!
//! **Where the store is.** The app names its config folder at launch ([`use_config_dir`]). A
//! process that has not, such as a test, keeps the store in the config folder
//! `BRIGHTFIELD_CONFIG_DIR` names when it is set, and otherwise in memory for the life of the
//! process, so no test writes under the home directory.

use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::{json, Value};

/// The file in the config folder that keeps the trust store.
pub const TRUST_FILE: &str = "trust.json";

/// A folder's identity on disk, which a move on the same volume keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Identity {
    /// The volume the folder is on.
    volume: u64,
    /// The folder's file number on that volume.
    file: u64,
    /// When the folder was made, as seconds and nanoseconds since 1970, where the system says.
    born: Option<(u64, u32)>,
}

/// One record: a Protocol or a folder, by path and identity.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    /// Where the folder was when the record was last written.
    path: PathBuf,
    /// Its identity, where the system gives one.
    identity: Option<Identity>,
    /// Whether the record trusts every Protocol beneath the folder, not the folder alone.
    folder: bool,
}

/// What the store holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Ledger {
    /// Whether the Protocols from before trust have been recorded ([`Store::record_older_once`]).
    swept: bool,
    /// The trusted Protocols and folders.
    trusted: Vec<Record>,
    /// Where restricted Protocols came from.
    origins: Vec<(Record, String)>,
}

/// The config file the app named at launch.
static INSTALLED: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The store of a process that keeps it in memory.
static MEMORY: Mutex<Option<Ledger>> = Mutex::new(None);

/// Held across each read and write of the store, so two changes at once do not lose one.
static LOCK: Mutex<()> = Mutex::new(());

/// Keep the trust store in the app's config folder from now on: what the app does at launch.
pub fn use_config_dir() {
    let dir = brightfield_workbench::persist::config_dir(
        std::env::var(crate::startup::CONFIG_DIR_VAR)
            .ok()
            .as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    );
    *INSTALLED.lock().unwrap_or_else(PoisonError::into_inner) = dir.map(|d| d.join(TRUST_FILE));
}

/// The trust store: in a file, or in this process's memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    /// The file, or `None` for this process's memory.
    file: Option<PathBuf>,
}

impl Store {
    /// The store this process uses: the file the app named at launch, else the one in the config
    /// folder `BRIGHTFIELD_CONFIG_DIR` names, else memory.
    #[must_use]
    pub fn current() -> Self {
        let installed = INSTALLED
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let file = installed.or_else(|| {
            std::env::var(crate::startup::CONFIG_DIR_VAR)
                .ok()
                .filter(|d| !d.is_empty())
                .map(|d| PathBuf::from(d).join(TRUST_FILE))
        });
        Self { file }
    }

    /// The store kept in `file`.
    #[must_use]
    pub fn at(file: &Path) -> Self {
        Self {
            file: Some(file.to_path_buf()),
        }
    }

    /// Trust the Protocol in `dir`.
    ///
    /// # Errors
    ///
    /// A folder that is not there, or a store that cannot be written.
    pub fn trust(&self, dir: &Path) -> Result<(), String> {
        self.record(dir, false)
    }

    /// Trust the folder `dir` and every Protocol beneath it, including ones added later.
    ///
    /// # Errors
    ///
    /// A folder that is not there, or a store that cannot be written.
    pub fn trust_folder(&self, dir: &Path) -> Result<(), String> {
        self.record(dir, true)
    }

    /// Take away the records on `dir`, as a Protocol or as a folder. A Protocol beneath a folder
    /// that is still trusted stays trusted.
    ///
    /// # Errors
    ///
    /// A store that cannot be written.
    pub fn revoke(&self, dir: &Path) -> Result<(), String> {
        let place = Place::of(dir);
        self.change(|ledger| {
            ledger.trusted.retain(|r| !place.is(r));
        })
    }

    /// Whether the Protocol in `dir` is trusted: by a record of its own, or by a folder above it
    /// (or the folder itself) trusted with everything beneath it. A folder that is not there is not
    /// trusted.
    #[must_use]
    pub fn is_trusted(&self, dir: &Path) -> bool {
        let ledger = self.read();
        trusted_in(&ledger, dir)
    }

    /// The folders trusted with everything beneath them, as last recorded.
    #[must_use]
    pub fn folders(&self) -> Vec<PathBuf> {
        self.read()
            .trusted
            .into_iter()
            .filter(|r| r.folder)
            .map(|r| r.path)
            .collect()
    }

    /// Point the records on the folder that was at `old` at `new`, where it is now: what a rename
    /// or a move does, so a trusted Protocol stays trusted. A Protocol trusted only through a
    /// folder above `old` gets a record of its own at `new`, so a move the analyst asked for never
    /// leaves it restricted. A Protocol that was not trusted stays as it was.
    ///
    /// # Errors
    ///
    /// A store that cannot be written.
    pub fn repoint(&self, old: &Path, new: &Path) -> Result<(), String> {
        let was = absolute_lexical(old);
        let now = Place::of(new);
        self.change(|ledger| {
            let mut own = false;
            for record in &mut ledger.trusted {
                if record.path == was || now.is(record) {
                    own = true;
                    record.path.clone_from(&now.path);
                    record.identity.clone_from(&now.identity);
                }
            }
            for (record, _) in &mut ledger.origins {
                if record.path == was || now.is(record) {
                    record.path.clone_from(&now.path);
                    record.identity.clone_from(&now.identity);
                }
            }
            let above = was
                .parent()
                .is_some_and(|parent| folder_trusted_above(ledger, parent));
            if !own && above && !trusted_in(ledger, &now.path) {
                ledger.trusted.push(now.record(false));
            }
        })
    }

    /// Follow a folder the Finder moved: a record that names the folder by its identity is
    /// pointed at where it is now. Nothing is trusted that was not.
    ///
    /// # Errors
    ///
    /// A store that cannot be written.
    pub fn follow(&self, dir: &Path) -> Result<(), String> {
        let place = Place::of(dir);
        if place.identity.is_none() {
            return Ok(());
        }
        let ledger = self.read();
        let stale = ledger
            .trusted
            .iter()
            .chain(ledger.origins.iter().map(|(r, _)| r))
            .any(|r| place.is(r) && r.path != place.path);
        if !stale {
            return Ok(());
        }
        self.change(|ledger| {
            for record in ledger
                .trusted
                .iter_mut()
                .chain(ledger.origins.iter_mut().map(|(r, _)| r))
            {
                if place.is(record) {
                    record.path.clone_from(&place.path);
                }
            }
        })
    }

    /// Note that the Protocol in `dir` came from `origin`, such as the host of the link it was
    /// downloaded by. The note grants no trust.
    ///
    /// # Errors
    ///
    /// A folder that is not there, or a store that cannot be written.
    pub fn note_origin(&self, dir: &Path, origin: &str) -> Result<(), String> {
        let place = Place::of(dir);
        if !place.path.is_dir() {
            return Err(format!("{}: no such folder", dir.display()));
        }
        self.change(|ledger| {
            ledger.origins.retain(|(r, _)| !place.is(r));
            ledger
                .origins
                .push((place.record(false), origin.to_string()));
        })
    }

    /// Where the Protocol in `dir` came from, when the store noted it.
    #[must_use]
    pub fn origin_of(&self, dir: &Path) -> Option<String> {
        let place = Place::of(dir);
        self.read()
            .origins
            .into_iter()
            .find(|(r, _)| place.is(r))
            .map(|(_, origin)| origin)
    }

    /// Trust each Protocol in `dirs`, once: the first launch of a build that keeps trust records
    /// the Protocols Brightfield made before it. A later call records nothing. Answers whether
    /// this call recorded them.
    ///
    /// # Errors
    ///
    /// A store that cannot be written.
    pub fn record_older_once(&self, dirs: &[PathBuf]) -> Result<bool, String> {
        let mut swept = false;
        self.change(|ledger| {
            if ledger.swept {
                return;
            }
            for dir in dirs {
                let place = Place::of(dir);
                if place.path.is_dir() && !ledger.trusted.iter().any(|r| place.is(r)) {
                    ledger.trusted.push(place.record(false));
                }
            }
            ledger.swept = true;
            swept = true;
        })?;
        Ok(swept)
    }

    /// Trust `dir` as a Protocol (`folder` false) or a folder of them.
    fn record(&self, dir: &Path, folder: bool) -> Result<(), String> {
        let place = Place::of(dir);
        if place.identity.is_none() && !place.path.is_dir() {
            return Err(format!("{}: no such folder", dir.display()));
        }
        self.change(|ledger| {
            match ledger
                .trusted
                .iter_mut()
                .find(|r| r.folder == folder && place.is(r))
            {
                Some(record) => *record = place.record(folder),
                None => ledger.trusted.push(place.record(folder)),
            }
        })
    }

    /// The ledger as it stands.
    fn read(&self) -> Ledger {
        let _held = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        match &self.file {
            Some(file) => read_file(file).unwrap_or_default(),
            None => MEMORY
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .unwrap_or_default(),
        }
    }

    /// Change the ledger with `edit` and keep it.
    fn change(&self, edit: impl FnOnce(&mut Ledger)) -> Result<(), String> {
        let _held = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        match &self.file {
            Some(file) => {
                let mut ledger = match read_file(file) {
                    Ok(ledger) => ledger,
                    Err(why) => {
                        // A store that cannot be read is set aside rather than written over, so
                        // what it held can still be found.
                        let aside = file.with_extension("json.unreadable");
                        eprintln!(
                            "warning: {} could not be read ({why}); it is kept as {}",
                            file.display(),
                            aside.display()
                        );
                        let _ = std::fs::rename(file, &aside);
                        Ledger::default()
                    }
                };
                edit(&mut ledger);
                write_file(file, &ledger)
            }
            None => {
                let mut memory = MEMORY.lock().unwrap_or_else(PoisonError::into_inner);
                edit(memory.get_or_insert_with(Ledger::default));
                Ok(())
            }
        }
    }
}

/// A folder as the store compares it: its path, made absolute and its links followed, and its
/// identity.
struct Place {
    path: PathBuf,
    identity: Option<Identity>,
}

impl Place {
    fn of(dir: &Path) -> Self {
        let path = dir.canonicalize().unwrap_or_else(|_| absolute_lexical(dir));
        let identity = identity(&path);
        Self { path, identity }
    }

    /// Whether `record` names this folder. With an identity on both sides, the folder must be the
    /// same file made at the same moment, on the same volume or at the same path (a volume can be
    /// numbered differently from one mount to the next). Otherwise the paths must match.
    fn is(&self, record: &Record) -> bool {
        match (&record.identity, &self.identity) {
            (Some(r), Some(d)) => {
                r.file == d.file
                    && r.born == d.born
                    && (r.volume == d.volume || record.path == self.path)
            }
            _ => record.path == self.path,
        }
    }

    fn record(&self, folder: bool) -> Record {
        Record {
            path: self.path.clone(),
            identity: self.identity.clone(),
            folder,
        }
    }
}

/// Whether `ledger` trusts the Protocol in `dir`.
fn trusted_in(ledger: &Ledger, dir: &Path) -> bool {
    let place = Place::of(dir);
    if !place.path.is_dir() {
        return false;
    }
    ledger.trusted.iter().any(|r| !r.folder && place.is(r))
        || folder_trusted_above(ledger, &place.path)
}

/// Whether a folder record covers `dir`: one on `dir` itself or on a folder above it.
fn folder_trusted_above(ledger: &Ledger, dir: &Path) -> bool {
    if !ledger.trusted.iter().any(|r| r.folder) {
        return false;
    }
    let start = dir.canonicalize().unwrap_or_else(|_| absolute_lexical(dir));
    start.ancestors().any(|folder| {
        let place = Place {
            path: folder.to_path_buf(),
            identity: identity(folder),
        };
        ledger.trusted.iter().any(|r| r.folder && place.is(r))
    })
}

/// `path` made absolute against the working directory, with `.` and `..` resolved by name: what a
/// folder that is no longer there is compared by.
fn absolute_lexical(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    // A folder that has gone may sit in one that is still there under another spelling, such as
    // `/tmp` for `/private/tmp`, so the part that is there is spelled as the system spells it.
    match (out.parent(), out.file_name()) {
        (Some(parent), Some(name)) => parent
            .canonicalize()
            .map_or_else(|_| out.clone(), |p| p.join(name)),
        _ => out,
    }
}

#[cfg(unix)]
fn identity(dir: &Path) -> Option<Identity> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::metadata(dir).ok()?;
    if !meta.is_dir() {
        return None;
    }
    let born = meta
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| (d.as_secs(), d.subsec_nanos()));
    Some(Identity {
        volume: meta.dev(),
        file: meta.ino(),
        born,
    })
}

#[cfg(not(unix))]
fn identity(_dir: &Path) -> Option<Identity> {
    None
}

/// Read the ledger from `file`; a file that is not there is an empty ledger.
fn read_file(file: &Path) -> Result<Ledger, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::default()),
        Err(e) => return Err(e.to_string()),
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let records = |key: &str| -> Result<Vec<(Record, Option<String>)>, String> {
        value
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|v| {
                let path = v
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or("a record with no path")?;
                let identity = match (
                    v.get("volume").and_then(Value::as_u64),
                    v.get("file").and_then(Value::as_u64),
                ) {
                    (Some(volume), Some(file)) => Some(Identity {
                        volume,
                        file,
                        born: v.get("born").and_then(Value::as_array).and_then(|b| {
                            let secs = b.first()?.as_u64()?;
                            let nanos = u32::try_from(b.get(1)?.as_u64()?).ok()?;
                            Some((secs, nanos))
                        }),
                    }),
                    _ => None,
                };
                Ok((
                    Record {
                        path: PathBuf::from(path),
                        identity,
                        folder: v.get("folder").and_then(Value::as_bool).unwrap_or(false),
                    },
                    v.get("origin").and_then(Value::as_str).map(str::to_string),
                ))
            })
            .collect()
    };
    Ok(Ledger {
        swept: value.get("swept").and_then(Value::as_bool).unwrap_or(false),
        trusted: records("trusted")?.into_iter().map(|(r, _)| r).collect(),
        origins: records("origins")?
            .into_iter()
            .filter_map(|(r, origin)| Some((r, origin?)))
            .collect(),
    })
}

/// Write `ledger` to `file`, through a file beside it renamed into place, so a write cut short
/// leaves the old store whole.
fn write_file(file: &Path, ledger: &Ledger) -> Result<(), String> {
    let record = |r: &Record| {
        let mut v = json!({ "path": r.path.to_string_lossy(), "folder": r.folder });
        if let Some(id) = &r.identity {
            v["volume"] = json!(id.volume);
            v["file"] = json!(id.file);
            if let Some((secs, nanos)) = id.born {
                v["born"] = json!([secs, nanos]);
            }
        }
        v
    };
    let value = json!({
        "swept": ledger.swept,
        "trusted": ledger.trusted.iter().map(record).collect::<Vec<_>>(),
        "origins": ledger.origins.iter().map(|(r, origin)| {
            let mut v = record(r);
            v["origin"] = json!(origin);
            v
        }).collect::<Vec<_>>(),
    });
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())? + "\n";
    let partial = file.with_extension("json.partial");
    std::fs::write(&partial, text).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, file).map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store in a scratch file, and a scratch folder to make Protocols in.
    fn scratch(name: &str) -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().expect("a scratch folder");
        let store = Store::at(&tmp.path().join(name).join(TRUST_FILE));
        (tmp, store)
    }

    #[test]
    fn a_trusted_protocol_is_trusted_and_its_neighbour_is_not() {
        let (tmp, store) = scratch("one");
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        store.trust(&a).unwrap();
        assert!(store.is_trusted(&a));
        assert!(!store.is_trusted(&b));
        assert!(!store.is_trusted(&tmp.path().join("gone")));
    }

    #[test]
    fn a_folder_made_where_a_trusted_one_was_is_not_trusted() {
        let (tmp, store) = scratch("reuse");
        let a = tmp.path().join("a");
        std::fs::create_dir_all(&a).unwrap();
        store.trust(&a).unwrap();
        std::fs::remove_dir_all(&a).unwrap();
        // Something else between, so the new folder cannot take the old one's number.
        std::fs::create_dir_all(tmp.path().join("between")).unwrap();
        std::fs::create_dir_all(&a).unwrap();
        if identity(&a).is_some() {
            assert!(!store.is_trusted(&a), "a new folder at the old path");
        }
    }

    #[test]
    fn a_store_that_cannot_be_read_is_set_aside_and_trusts_nothing() {
        let (tmp, store) = scratch("bad");
        let file = tmp.path().join("bad").join(TRUST_FILE);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "not json").unwrap();
        let a = tmp.path().join("a");
        std::fs::create_dir_all(&a).unwrap();
        assert!(!store.is_trusted(&a));
        store.trust(&a).unwrap();
        assert!(store.is_trusted(&a));
        assert!(file.with_extension("json.unreadable").is_file());
    }
}
