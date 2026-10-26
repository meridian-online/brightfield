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
pub fn split<T: DeserializeOwned>(
    exts: Vec<Extension>,
    what: &str,
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
    meta.map(|m| (m, others))
        .ok_or_else(|| format!("{what}: no {VENDOR} extension"))
}

/// The entries to write: Meridian's first, then the other vendors'.
pub fn join<T: Serialize>(meta: &T, others: &[Extension]) -> Vec<Extension> {
    let mut v = vec![encode(meta)];
    v.extend(others.iter().cloned());
    v
}
