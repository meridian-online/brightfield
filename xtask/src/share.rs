//! How much of the machine a gate or nightly run takes: all its cores when no
//! other build holds a build slot at the run's start, and a session's share,
//! 6 cargo jobs and 6 test threads, otherwise.
//!
//! A build slot is a file `slot-N.lock` in the slots directory that a cargo
//! build holds an OS advisory lock on while it runs, through the methods
//! plugin's cargo shim. The shim tells the cargo it starts which slot it took
//! in `MERIDIAN_BUILD_SLOT`, so a gate's own slot is told apart from another
//! session's. The reading is made once, at the run's start: a session that
//! starts compiling during the run shares the cores with it until it ends.

use std::fs::{self, TryLockError};
use std::path::{Path, PathBuf};

/// The cargo job and test thread count a Claude Code session runs with, so
/// three sessions can build at once.
pub const SESSION_SHARE: usize = 6;

/// How many threads and jobs a run takes, and why, for its log.
#[derive(Debug, PartialEq)]
pub struct Share {
    pub threads: usize,
    pub why: String,
}

/// The share for a run starting now on this machine.
pub fn at_start() -> Share {
    let dir = slots_dir();
    let own = std::env::var("MERIDIAN_BUILD_SLOT")
        .ok()
        .and_then(|n| n.trim().parse::<u32>().ok());
    let cores = std::thread::available_parallelism().map_or(SESSION_SHARE, |n| n.get());
    choose(held_elsewhere(dir.as_deref(), own), cores)
}

/// The share given the other builds' slots, or why they could not be read,
/// and the machine's core count.
fn choose(held: Result<Vec<u32>, String>, cores: usize) -> Share {
    match held {
        Ok(held) if held.is_empty() => Share {
            threads: cores.max(SESSION_SHARE),
            why: "no other build holds a slot".to_owned(),
        },
        Ok(held) => {
            let slots: Vec<String> = held.iter().map(u32::to_string).collect();
            Share {
                threads: SESSION_SHARE,
                why: format!("another build holds slot {}", slots.join(", ")),
            }
        }
        Err(e) => Share {
            threads: SESSION_SHARE,
            why: format!("the build slots could not be read: {e}"),
        },
    }
}

/// The slots directory: `MERIDIAN_BUILD_SLOTS_DIR`, else
/// `~/.cache/meridian/build-slots`, as the shim reads it.
fn slots_dir() -> Option<PathBuf> {
    std::env::var_os("MERIDIAN_BUILD_SLOTS_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/meridian/build-slots"))
        })
}

/// The numbers of the slots in `dir` that something other than slot `own`'s
/// holder has locked. A directory that is not there holds no slot.
fn held_elsewhere(dir: Option<&Path>, own: Option<u32>) -> Result<Vec<u32>, String> {
    let dir = dir.ok_or("neither MERIDIAN_BUILD_SLOTS_DIR nor HOME is set")?;
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut held = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(number) = name
            .to_str()
            .and_then(|n| n.strip_prefix("slot-"))
            .and_then(|n| n.strip_suffix(".lock"))
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if Some(number) == own {
            continue;
        }
        let path = entry.path();
        let file = fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        // Taking the lock for a moment is how the shim's own status reads a
        // slot; the file closes, and the lock goes, at the end of this block.
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => held.push(number),
            Err(TryLockError::Error(e)) => return Err(format!("{}: {e}", path.display())),
        }
    }
    held.sort_unstable();
    Ok(held)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(tag: &str, numbers: &[u32]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-share-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for n in numbers {
            fs::write(dir.join(format!("slot-{n}.lock")), "").unwrap();
        }
        dir
    }

    fn hold(dir: &Path, n: u32) -> fs::File {
        let file = fs::File::open(dir.join(format!("slot-{n}.lock"))).unwrap();
        file.lock().unwrap();
        file
    }

    #[test]
    fn free_slots_are_no_other_build() {
        let dir = slots("free", &[1, 2, 3]);
        assert_eq!(held_elsewhere(Some(&dir), None), Ok(vec![]));
        assert_eq!(choose(Ok(vec![]), 18).threads, 18);
    }

    #[test]
    fn the_runs_own_slot_is_not_another_build() {
        let dir = slots("own", &[1, 2, 3]);
        let _own = hold(&dir, 2);
        assert_eq!(held_elsewhere(Some(&dir), Some(2)), Ok(vec![]));
    }

    #[test]
    fn another_builds_slot_takes_the_run_to_a_sessions_share() {
        let dir = slots("other", &[1, 2, 3]);
        let _own = hold(&dir, 1);
        let _other = hold(&dir, 3);
        let held = held_elsewhere(Some(&dir), Some(1));
        assert_eq!(held, Ok(vec![3]));
        let share = choose(held, 18);
        assert_eq!(share.threads, SESSION_SHARE);
        assert_eq!(share.why, "another build holds slot 3");
    }

    #[test]
    fn a_slots_directory_that_is_not_there_holds_no_slot() {
        let dir = std::env::temp_dir().join(format!("xtask-share-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(held_elsewhere(Some(&dir), None), Ok(vec![]));
    }

    #[test]
    fn slots_that_cannot_be_read_take_a_sessions_share() {
        let share = choose(Err("no HOME".to_owned()), 18);
        assert_eq!(share.threads, SESSION_SHARE);
        assert!(share.why.contains("no HOME"), "{}", share.why);
        assert_eq!(held_elsewhere(None, None).map_err(|_| ()), Err(()));
    }

    #[test]
    fn a_machine_with_fewer_cores_than_a_share_still_gets_a_share() {
        assert_eq!(choose(Ok(vec![]), 4).threads, SESSION_SHARE);
    }
}
