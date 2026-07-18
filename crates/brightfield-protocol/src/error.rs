//! Error surface for loading a protocol graph from the emitted contract.
//!
//! Hand-rolled (no `thiserror`) to keep this library crate's dependency
//! surface minimal — downstream app crates may wrap these in a richer report
//! type.

use std::fmt;
use std::path::PathBuf;

use crate::contract::SUPPORTED_CONTRACT_FAMILY;

/// Failure modes when loading a [`ContractView`](crate::contract_graph::ContractView)
/// from a contract file.
#[derive(Debug)]
pub enum Error {
    /// Failed to read a file from disk.
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The bytes were not a structurally valid Protocol+Run contract.
    Parse(serde_json::Error),
    /// The `contract_version` is outside the `b4/` family this reader targets.
    UnsupportedVersion {
        /// The version found in the document.
        found: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "failed to read {}: {source}", path.display()),
            Error::Parse(e) => write!(f, "failed to parse Protocol+Run contract: {e}"),
            Error::UnsupportedVersion { found } => write!(
                f,
                "unsupported contract_version {found:?}; this reader targets the {SUPPORTED_CONTRACT_FAMILY:?} family"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::Parse(e) => Some(e),
            Error::UnsupportedVersion { .. } => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Parse(e)
    }
}
