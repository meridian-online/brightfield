//! The folder new Protocols are made in, and how a new one is named.
//!
//! An open makes a Protocol, so they accumulate somewhere the analyst can see and reach with
//! the Finder: `~/Documents/Protocols`, or the folder `BRIGHTFIELD_PROTOCOLS_DIR` names. Tests
//! point the variable at a temporary folder so none of them writes under the home directory.

use std::fmt;
use std::path::{Path, PathBuf};

/// The environment variable that relocates the Protocols folder, for tests and portable installs.
pub const PROTOCOLS_DIR_VAR: &str = "BRIGHTFIELD_PROTOCOLS_DIR";

/// Why a Protocol's folder could not be claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    /// The system refused the write in this folder, which on macOS means Brightfield has not been
    /// given access to it.
    Refused(PathBuf),
    /// A failure other than a refusal, in the file system's words.
    Io(String),
}

impl fmt::Display for ClaimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(parent) => write!(
                f,
                "Brightfield cannot make a Protocol in {}. Allow it under System Settings → \
                 Privacy & Security → Files and Folders, then open the file again.",
                parent.display()
            ),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ClaimError {}

/// Where new Protocols are made: `$BRIGHTFIELD_PROTOCOLS_DIR`, else `~/Documents/Protocols`.
#[must_use]
pub fn root() -> PathBuf {
    if let Some(dir) = std::env::var_os(PROTOCOLS_DIR_VAR).filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    // A launch from the Mac's desktop always has a home directory.
    std::env::home_dir()
        .unwrap_or_default()
        .join("Documents")
        .join("Protocols")
}

/// Make `parent` if it is missing, then claim a new folder in it named `stem`, or `stem-2`,
/// `stem-3` and so on when that is taken. The first Protocol takes the bare name.
///
/// `create_dir` is the claim: it fails if the folder exists, so two claims at once cannot both
/// take a name. Returns the folder and its name.
///
/// # Errors
///
/// [`ClaimError::Refused`] when the system denies the write in `parent`; [`ClaimError::Io`] for
/// any other failure.
pub fn claim(parent: &Path, stem: &str) -> Result<(PathBuf, String), ClaimError> {
    std::fs::create_dir_all(parent).map_err(|e| classify(&e, parent))?;
    (1..)
        .find_map(|n| {
            let name = if n == 1 {
                stem.to_string()
            } else {
                format!("{stem}-{n}")
            };
            let dir = parent.join(&name);
            match std::fs::create_dir(&dir) {
                Ok(()) => Some(Ok((dir, name))),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(e) => Some(Err(classify(&e, parent))),
            }
        })
        .expect("an unbounded range")
}

fn classify(error: &std::io::Error, parent: &Path) -> ClaimError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        ClaimError::Refused(parent.to_path_buf())
    } else {
        ClaimError::Io(format!("{}: {error}", parent.display()))
    }
}
