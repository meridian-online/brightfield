//! `read-file` in the window: the sources browser.
//!
//! A file is brought in through the sources browser, which adds it with its
//! FineType profile and data package and proposes a folder's joins through
//! dovetail, none of which an arc `read-file` step records. So the operation
//! starts only from the browser's own command, `open-sources-browser`, by its
//! key, its palette row or its ribbon command under Connect, and that command
//! opens the browser rather than a draft
//! (`the_bring_in_command_opens_the_sources_browser`). It asks what the browser
//! asks, and the browser names the tables it adds, so the slot has no field, no
//! form and no name of its own.
//!
//! A `read-file` step recorded some other way, by an agent through arc, still
//! opens as a step of the Protocol and previews its file
//! (`an_agents_read_file_step_opens_and_previews_its_file`).

use super::{Begin, By, Draft, Keeps, NameRule, OperationUi, Stage, Start};

/// No draft: the browser adds the file, and the window opens the browser for
/// this operation before a draft would begin.
fn begin(_: &Begin<'_>) -> Result<Draft, String> {
    Err("Add sources brings a file in through the sources browser.".to_string())
}

static UI: OperationUi = OperationUi {
    long_name: "read-file",
    verb: "open-sources-browser",
    stage: Stage::Connect,
    // The ribbon's icon for the same command.
    icon: "database",
    starts: &[Start::SourcesBrowser],
    menu_words: "Add sources",
    begin,
    keeps: Keeps::OnEnter,
    field: None,
    form: None,
    name: NameRule {
        word: "read",
        from: By::Nothing,
        with_by: false,
    },
};

/// `read-file`'s slot.
#[must_use]
pub fn ui() -> Option<&'static OperationUi> {
    Some(&UI)
}
