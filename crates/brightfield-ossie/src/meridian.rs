//! Meridian's extension entries: encode and decode the `data` JSON string, and
//! split a list of extensions into Meridian's and everyone else's.

use serde::{de::DeserializeOwned, Serialize};

use crate::model::Extension;
use crate::VENDOR;

/// Encode `meta` as a `MERIDIAN` entry.
pub fn encode<T: Serialize>(meta: &T) -> Extension {
    Extension {
        vendor_name: VENDOR.to_string(),
        // Plain structs of strings, numbers and options always serialise.
        data: serde_json::to_string(meta).expect("meridian metadata serialises"),
    }
}

/// Split `exts` into the decoded Meridian entry (the first one) and the other
/// vendors' entries, in order. Any further `MERIDIAN` entries are kept with the others.
/// An element with no Meridian entry (a hand-written one) gets `default()`, so
/// the file stays readable; an entry whose data is not valid is an error.
pub fn split<T: DeserializeOwned>(
    exts: Vec<Extension>,
    what: &str,
    default: impl FnOnce() -> T,
) -> Result<(T, Vec<Extension>), String> {
    let mut meta = None;
    let mut others = Vec::new();
    for e in exts {
        if meta.is_none() && e.vendor_name == VENDOR {
            meta = Some(
                serde_json::from_str::<T>(&e.data)
                    .map_err(|err| format!("{what}: bad {VENDOR} extension data: {err}"))?,
            );
        } else {
            others.push(e);
        }
    }
    Ok((meta.unwrap_or_else(default), others))
}

/// The entries to write: Meridian's first, then the other vendors'.
pub fn join<T: Serialize>(meta: &T, others: &[Extension]) -> Vec<Extension> {
    let mut v = vec![encode(meta)];
    v.extend(others.iter().cloned());
    v
}
