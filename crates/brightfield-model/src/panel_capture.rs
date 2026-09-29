//! Capturing an explored chart as a protocol panel file.
//!
//! A chart explored on top of a protocol is a working [`Spec`] — the one
//! [`ChartEdit`](brightfield_spec::edit::ChartEdit) mutations shape live. When a
//! person decides an explored chart is worth keeping, it is written beside the
//! protocol's models as `panels/<name>.yaml`.
//!
//! A panel that [`capture_panel`] creates whole is brightfield's own artifact,
//! machine-authored end to end, so capture is a plain canonical re-serialisation
//! ([`serialise_spec`]): **regeneration IS conformance**, and there is no
//! comment in it for a byte-preserving writer to keep. The round-trip is the contract —
//! `parse -> edit -> serialise -> re-parse` yields the same chart AST.
//!
//! **That holds for a chart brightfield creates and not for one it edits.** A
//! chart file written from a data file carries the dashboard generator's
//! comments, and an analyst may add their own beside them; the canonical
//! re-serialisation keeps no comment, so it cannot be how an edit reaches such
//! a file. An edit is written into the file's text as a change to one line, by
//! `brightfield_protocol::write_chart_edit`, and [`panel_file`] and
//! [`write_panel_text`] are the path and the write that route shares with
//! capture: the same `panels/<name>.yaml` under the same slug of the name.
//!
//! The panel binds to the protocol through the **asset namespace**: a mark's
//! `from:` source names a table, and that table is the output of a protocol
//! step. Capture preserves those names verbatim, so a panel and the steps that
//! feed it join by name with nothing to register.
//!
//! The `panels:` manifest block is **deferred**: a captured panel is discovered
//! by directory convention (a file under `panels/`), not by an entry in
//! `arcform.yaml`. Capture writes the file and nothing else.

use std::path::{Path, PathBuf};

use brightfield_spec::ast::Spec;
use brightfield_spec::serialise_spec;

use crate::spec_save::save_spec_atomic;

/// The subdirectory captured panels land in, beside `models/`.
pub const PANELS_DIR: &str = "panels";

/// Why a panel capture could not be written.
#[derive(Debug)]
pub enum PanelCaptureError {
    /// The chart could not be canonically serialised (a malformed working Spec).
    Serialise(String),
    /// A filesystem write failed.
    Io(std::io::Error),
}

impl std::fmt::Display for PanelCaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Serialise(e) => write!(f, "could not serialise the chart: {e}"),
            Self::Io(e) => write!(f, "could not write the panel file: {e}"),
        }
    }
}

impl std::error::Error for PanelCaptureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialise(_) => None,
            Self::Io(e) => Some(e),
        }
    }
}

/// Write the explored chart `spec` as `panels/<name>.yaml` in the protocol
/// directory `dir`, beside `models/`. Returns the panel's path relative to
/// `dir`.
///
/// Capture is a canonical re-serialisation — lossy by design (it reformats and
/// drops comments), because a captured panel has no authorship to preserve. The
/// chart's `from:` sources — the asset-namespace tables it reads — survive
/// verbatim, so the panel binds to its producing steps by name.
///
/// `panels/` is created if absent. The file name is slugged for the filesystem;
/// the chart inside keeps whatever titles it carries.
///
/// # Errors
///
/// [`PanelCaptureError::Serialise`] when the working chart will not serialise;
/// [`PanelCaptureError::Io`] when the directory or file write fails.
pub fn capture_panel(dir: &Path, name: &str, spec: &Spec) -> Result<PathBuf, PanelCaptureError> {
    let yaml = serialise_spec(spec).map_err(PanelCaptureError::Serialise)?;
    let panels_abs = dir.join(PANELS_DIR);
    std::fs::create_dir_all(&panels_abs).map_err(PanelCaptureError::Io)?;
    let rel = panel_rel(name);
    std::fs::write(dir.join(&rel), yaml).map_err(PanelCaptureError::Io)?;
    Ok(rel)
}

/// The panel file for `name`, relative to the protocol directory:
/// `panels/<slug of name>.yaml`. The one place the convention is spelled.
fn panel_rel(name: &str) -> PathBuf {
    PathBuf::from(PANELS_DIR).join(format!("{}.yaml", filename_slug(name)))
}

/// Where the panel file for `name` is, under the protocol directory `dir` —
/// the path [`capture_panel`] writes, joined onto `dir`.
#[must_use]
pub fn panel_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(panel_rel(name))
}

/// Write `text` as the panel file for `name` under `dir`, through
/// [`save_spec_atomic`]: a sibling temp file renamed over the destination, so a
/// reader does not see half a file, and `text` equal to the file's bytes leaves
/// the file untouched. Returns the file's path, [`panel_file`]'s answer joined
/// onto `dir`.
///
/// The bytes are `text`'s and only `text`'s — no parse, no re-serialisation —
/// which is what lets a caller that has placed an edit into a file's own text
/// hand the result here without a comment being lost.
///
/// `panels/` is created if absent.
///
/// # Errors
///
/// The filesystem's error when the directory cannot be created or the write or
/// the rename fails; the destination is left as it was.
pub fn write_panel_text(dir: &Path, name: &str, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir.join(PANELS_DIR))?;
    let path = panel_file(dir, name);
    save_spec_atomic(text, &path)?;
    Ok(path)
}

/// The panel name, made safe for a filename: anything outside `[A-Za-z0-9_-]`
/// becomes `_`, and a name with nothing usable degrades to `panel`.
fn filename_slug(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if slug.chars().all(|c| c == '_') {
        "panel".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brightfield_spec::edit::{apply, ChartEdit};
    use brightfield_spec::vocab::MarkKind;
    use brightfield_spec::{parse_spec, Format};

    fn parse(yaml: &str) -> Spec {
        parse_spec(yaml, Format::Yaml).expect("parse").spec
    }

    /// A minimal explored chart whose mark reads a protocol table by name.
    const EXPLORED: &str = "\
meta:
  title: Dover tides
plot:
  - mark: dot
    data: { from: dover_tides }
    x: day
    y: tide_m
";

    #[test]
    fn a_captured_panel_lands_under_panels_and_reparses_to_the_same_chart() {
        let dir = tempdir();
        let spec = parse(EXPLORED);

        let rel = capture_panel(dir.path(), "dover_tides", &spec).expect("captures");
        assert_eq!(rel, PathBuf::from("panels/dover_tides.yaml"));

        // The file landed beside where models/ would be.
        let written = std::fs::read_to_string(dir.path().join(&rel)).expect("read panel");
        // Regeneration IS conformance: re-parsing the panel yields the same chart.
        assert_eq!(
            parse(&written),
            spec,
            "the panel round-trips to the same AST"
        );
    }

    #[test]
    fn the_from_binding_survives_capture_verbatim() {
        let dir = tempdir();
        let spec = parse(EXPLORED);
        let rel = capture_panel(dir.path(), "dover_tides", &spec).expect("captures");
        let written = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
        assert!(
            written.contains("dover_tides"),
            "the from: table binds to the step asset by name: {written}"
        );
    }

    #[test]
    fn a_chart_edited_before_capture_is_the_one_written() {
        let dir = tempdir();
        let mut spec = parse(EXPLORED);
        // The working chart is whatever ChartEdit mutations produced.
        apply(
            &mut spec,
            &ChartEdit::ChangeMarkType {
                plot: brightfield_spec::analysis::ComponentPath("root".to_string()),
                mark_ordinal: 0,
                new_kind: MarkKind::Line,
            },
        )
        .expect("clean edit");

        let rel = capture_panel(dir.path(), "dover_tides", &spec).expect("captures");
        let written = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
        assert_eq!(
            parse(&written),
            spec,
            "the captured panel is the edited chart, not the original"
        );
        assert!(
            written.contains("line"),
            "the edit is in the panel: {written}"
        );
    }

    /// The text write and capture agree on where a panel lives, so a chart
    /// created whole and one edited afterwards are one file, not two.
    #[test]
    fn the_text_write_lands_where_capture_writes() {
        let dir = tempdir();
        let spec = parse(EXPLORED);
        let rel = capture_panel(dir.path(), "dover tides", &spec).expect("captures");

        assert_eq!(panel_file(dir.path(), "dover tides"), dir.path().join(&rel));
        let written = write_panel_text(dir.path(), "dover tides", "# kept\nplot: []\n")
            .expect("the text is written");
        assert_eq!(written, dir.path().join(&rel));
    }

    /// The bytes written are the text's, comments and all: a canonical
    /// re-serialisation would have dropped the comment.
    #[test]
    fn the_text_write_keeps_every_byte_of_the_text_and_creates_panels() {
        let dir = tempdir();
        assert!(!dir.path().join(PANELS_DIR).exists());
        let text = "# an analyst's note\nmeta:\n  title: Dover tides   # trailing\n";

        let path = write_panel_text(dir.path(), "dover_tides", text).expect("written");

        assert_eq!(std::fs::read_to_string(path).expect("read"), text);
    }

    #[test]
    fn slug_keeps_safe_characters_and_degrades() {
        assert_eq!(filename_slug("dover_tides"), "dover_tides");
        assert_eq!(filename_slug("top-10 tides!"), "top-10_tides_");
        assert_eq!(filename_slug("///"), "panel");
    }

    fn tempdir() -> TempDir {
        TempDir::new()
    }

    /// A tiny scratch directory that cleans itself up — no dev-dependency needed.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("bf-panel-{}-{seq}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
