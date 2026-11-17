//! **A read or a write the system refused, named by its path**: what the window's refused-access
//! banner reports and what makes the headless capture exit 3.
//!
//! A refusal reaches Brightfield in two shapes. The protocol graph classes a model file it could
//! not open as unreadable ([`brightfield_protocol::Degradation::ModelUnreadable`]), and
//! [`refusals_in_graph`] reads those back. A child that was refused (arc, DuckDB's CLI, dovetail)
//! says so only in its own text, and [`refusal_in_engine_text`] finds the path in it.
//!
//! Both editions report a refusal: in the managed edition it is usually a folder no grant covers,
//! and in the full edition macOS's privacy controls refuse reads too. [`Refusal::remedy`] says
//! what to do in each.

use std::path::{Path, PathBuf};

use brightfield_protocol::graph::{degrades, AssetGraph, Degradation};

use crate::edition::{self, Capability};

/// Exit status of the headless capture when a model read was refused. The PNG is still written;
/// 1 stays "no image was produced" and 2 a usage error.
pub const EXIT_REFUSED: u8 = 3;

/// What was being done when the system refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusedDuring {
    /// Reading a `sql:` step's model file to draw the Protocol.
    ModelRead,
    /// Reading a source a step names.
    SourceRead,
    /// Saving, exporting, or writing a run's outputs.
    Write,
    /// A run, whose child said it was refused.
    Run,
}

/// One refused path, and what was being done when it was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The path the system refused.
    pub path: PathBuf,
    /// What was being done.
    pub during: RefusedDuring,
}

impl Refusal {
    /// The folder to grant so that `path` can be read: the folder holding it, or the nearest one
    /// above that exists, since a folder at mode `000` refuses whatever is inside.
    #[must_use]
    pub fn folder(&self) -> PathBuf {
        self.path
            .parent()
            .and_then(|parent| parent.ancestors().find(|a| a.exists()))
            .map_or_else(|| self.path.clone(), Path::to_path_buf)
    }

    /// The sentence saying how to let Brightfield in, for this edition: grant the folder in the
    /// managed edition, or allow it in System Settings in the full one.
    #[must_use]
    pub fn remedy(&self) -> String {
        if edition::current().allows(Capability::UngrantedPaths) {
            format!(
                "Allow Brightfield to read {} under System Settings → Privacy & Security → Files \
                 and Folders, or change its permissions, then open the Protocol again.",
                self.folder().display()
            )
        } else {
            format!(
                "Grant access to {} to draw it in full.",
                self.folder().display()
            )
        }
    }
}

/// The sentence the panel shows when a refused read asks for `folder`.
#[must_use]
pub fn why_refused(folder: &Path) -> String {
    format!(
        "Brightfield was refused {}. Grant access to it to draw the Protocol in full.",
        folder.display()
    )
}

/// The `arcform.yaml` of the Protocol a refused `path` is in: the nearest folder above it that
/// holds one, which is the Protocol to open again once the refused folder is granted.
#[must_use]
pub fn manifest_above(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .skip(1)
        .map(|dir| dir.join(arc::spec::MANIFEST_FILENAME))
        .find(|manifest| manifest.is_file())
}

/// Every model read the graph classes as refused, in node-id order, one per path.
#[must_use]
pub fn refusals_in_graph(graph: &AssetGraph) -> Vec<Refusal> {
    let mut out: Vec<Refusal> = Vec::new();
    for degrade in degrades(graph) {
        if degrade.class != Degradation::ModelUnreadable {
            continue;
        }
        let Some(path) = unreadable_path(&degrade.detail) else {
            continue;
        };
        if out.iter().all(|r| r.path != path) {
            out.push(Refusal {
                path,
                during: RefusedDuring::ModelRead,
            });
        }
    }
    out
}

/// The path in an `unreadable:` degrade's text, which the model read writes as
/// `unreadable: <path>: <io error> — <guidance>`.
fn unreadable_path(detail: &str) -> Option<PathBuf> {
    let rest = detail.strip_prefix("unreadable: ")?;
    let before_guidance = rest.split_once(" — ").map_or(rest, |(head, _)| head);
    let (path, _io_error) = before_guidance.rsplit_once(": ")?;
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// The words a refused read or write is reported with: the OS's for `EACCES` and `EPERM`, which
/// DuckDB's and arc's errors carry through.
const REFUSED_WORDS: [&str; 2] = ["Permission denied", "Operation not permitted"];

/// **The path a child's error text says was refused**, if it says one was: DuckDB's
/// `IO Error: Cannot open file "/a/b.csv": Operation not permitted`, or the OS's
/// `/a/b.csv: Permission denied (os error 13)`. The first refusal in the text wins. `None` when
/// the text names no refusal, or names one with no path in it.
#[must_use]
pub fn refusal_in_engine_text(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let at = REFUSED_WORDS.iter().filter_map(|w| line.find(w)).min()?;
        path_before(&line[..at])
    })
}

/// The path the text before a refusal's words names: the last quoted absolute path, else the
/// last absolute path that ends at a `: `.
fn path_before(head: &str) -> Option<PathBuf> {
    for quote in ['"', '\''] {
        let parts: Vec<&str> = head.split(quote).collect();
        // Quoted runs are the odd-indexed parts of a split on the quote.
        if let Some(quoted) = parts
            .iter()
            .skip(1)
            .step_by(2)
            .take(parts.len().saturating_sub(1) / 2)
            .rev()
            .find(|p| is_absolute(p))
        {
            return Some(PathBuf::from(*quoted));
        }
    }
    let trimmed = head.trim_end().strip_suffix(':')?;
    let start = trimmed
        .match_indices(['/', '~'])
        .map(|(i, _)| i)
        .find(|&i| i == 0 || trimmed[..i].ends_with(' ') || trimmed[..i].ends_with(':'))?;
    let path = trimmed[start..].trim();
    is_absolute(path).then(|| PathBuf::from(path))
}

fn is_absolute(text: &str) -> bool {
    text.starts_with('/') || text.starts_with("~/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_text_with_permission_denied_names_the_path() {
        assert_eq!(
            refusal_in_engine_text("IO Error: Cannot open file \"/a/b.csv\": Permission denied"),
            Some(PathBuf::from("/a/b.csv"))
        );
    }

    #[test]
    fn the_os_wording_names_the_path() {
        assert_eq!(
            refusal_in_engine_text(
                "step load failed\nError: /Users/x/p/models/t.sql: Permission denied (os error 13)"
            ),
            Some(PathBuf::from("/Users/x/p/models/t.sql"))
        );
    }

    #[test]
    fn text_without_a_refusal_names_nothing() {
        assert_eq!(
            refusal_in_engine_text("Binder Error: column \"x\" not found"),
            None
        );
        assert_eq!(refusal_in_engine_text("Permission denied"), None);
    }

    #[test]
    fn the_unreadable_degrade_text_gives_its_path() {
        assert_eq!(
            unreadable_path(
                "unreadable: /p/models/t.sql: Permission denied (os error 13) — the read was \
                 refused."
            ),
            Some(PathBuf::from("/p/models/t.sql"))
        );
        assert_eq!(
            unreadable_path("absent: /p/models/t.sql: No such file"),
            None
        );
    }

    #[test]
    fn the_folder_to_grant_is_the_nearest_that_exists() {
        let refusal = Refusal {
            path: PathBuf::from("/nowhere-at-all/models/t.sql"),
            during: RefusedDuring::ModelRead,
        };
        assert_eq!(refusal.folder(), PathBuf::from("/"));
    }
}
