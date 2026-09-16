//! What happens before the window exists: where the layout file is, and
//! reading it in the one order that works.
//!
//! # The ordering, which is the whole reason this is a function
//!
//! A `PaneKey` in a layout file deserialises through
//! [`ItemId`](brightfield_workbench::ItemId)'s `Deserialize`, which resolves
//! the string against a process-global vocabulary that is empty until a
//! registry publishes into it. One unknown id fails the whole envelope, so a
//! read that happens before both views have published reports a perfectly good
//! file as [`LoadOutcome::Corrupt`] and silently hands the user the default
//! arrangement.
//!
//! **Both** registries, not just the one whose document the command line
//! named: the window's one tree carries every pane of both, and they all
//! deserialise in the same pass.
//!
//! The publish calls used to live inside `MeridianApp::assemble`, which runs
//! inside `eframe::run_native`'s creation closure — i.e. after the viewport
//! has already been built. A saved window size can only reach
//! `ViewportBuilder` if the read happens *before* `run_native`, and the read is
//! only valid after the publish, so the publish comes here. It stays in
//! `assemble` as well: [`ItemRegistry::publish_ids`](brightfield_workbench::ItemRegistry::publish_ids)
//! returns early when everything is already known, so the second call costs
//! nothing and the headless tiers go on publishing without this module.
//!
//! # What this deliberately does not do
//!
//! It does not build a [`MeridianApp`](crate::window::MeridianApp), and the
//! two functions here that reach a **layout** file — [`layout_path`] and
//! [`boot_layout`] — have no caller in `window.rs`. It does call
//! [`default_layout`], which builds the window's declared tree and reads
//! nothing.
//!
//! That is what keeps `cargo test` and the PNG capture path off the
//! developer's real
//! `~/Library/Application Support/Brightfield/workspace-layout.json`: no
//! constructor can reach a saved layout, because the only way one gets in is
//! as an argument, and the save path is an `Option<PathBuf>` only `main`
//! fills in.
//!
//! [`datasets_dir`] is the exception and it is deliberately a narrow one: it
//! names a directory rather than the layout file, and [`crate::starts::load`]
//! calls it to put a bundled data file where a second launch will find it. A suite
//! that loads such a start therefore points [`CONFIG_DIR_VAR`] at its own
//! scratch directory first — `datasets_into_scratch` in
//! `crates/brightfield-shell/tests/front_door.rs` is that call, and
//! `a_bundled_data_start_writes_under_the_configured_directory` beside it is
//! what fails if the resolution stops honouring the override.

use std::path::{Path, PathBuf};

use brightfield_workbench::persist::{self, LoadOutcome, SavedLayout};
use brightfield_workbench::{window_tree, Workspace};

use crate::app::chart_registry;
use crate::protocol::protocol_registry;

/// The environment variable that relocates the config directory, for tests and
/// portable installs. The name the gpui-era shell already used.
pub const CONFIG_DIR_VAR: &str = "BRIGHTFIELD_CONFIG_DIR";

/// The default arrangement: **one** tile tree over every pane both registries
/// declare, at the default window geometry.
///
/// One tree because one window draws one arrangement — the panes of both
/// documents share it, and [`window_tree`] takes their placements together
/// rather than each registry's tree separately. The chart document's panes
/// come first, so the centre strip opens on the chart rather than on the
/// graph; that is the same "no opinion" default the window's own canvas rule
/// lands on for a window holding both.
///
/// Read twice — as the fallback for a file that will not load, and as the
/// yardstick [`persist::from_json`] measures a short file against — so it is
/// declared once.
#[must_use]
pub fn default_layout() -> SavedLayout {
    let mut placements = chart_registry().placements();
    placements.extend(protocol_registry().placements());
    SavedLayout::new(Workspace::new(window_tree(&placements)))
}

/// Where this machine's layout file lives, or `None` if it has nowhere to put
/// one.
///
/// `None` is a legitimate answer and not an error: the window runs with
/// persistence off rather than refusing to start.
#[must_use]
pub fn layout_path() -> Option<PathBuf> {
    persist::layout_path(
        std::env::var(CONFIG_DIR_VAR).ok().as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// The directory under [`layout_path`]'s own directory where a start that
/// ships a data file puts those bytes.
///
/// A subdirectory rather than the config directory itself so a reader opening
/// it finds one folder of data beside one layout file, rather than a Parquet
/// filed next to a JSON that has nothing to do with it.
pub const DATASETS_DIR: &str = "datasets";

/// Where this machine keeps the data files bundled starts write, or `None` if
/// it has nowhere to put them.
///
/// The same policy [`layout_path`] takes and the same override —
/// [`CONFIG_DIR_VAR`] relocates both together, which is the whole reason
/// `persist::config_dir` is the directory rather than the layout file. A
/// portable install that moved one and not the other would put a start's data
/// somewhere the next launch does not look.
///
/// **This is the one function here a constructor does reach**, through
/// [`crate::starts::load`], and the paragraph above about `cargo test` is
/// narrower because of it: a suite that loads a start shipping a data file
/// writes that file, so those suites set [`CONFIG_DIR_VAR`] to their own
/// scratch directory. The layout file is still out of reach — nothing here
/// reads or writes one.
#[must_use]
pub fn datasets_dir() -> Option<PathBuf> {
    persist::config_dir(
        std::env::var(CONFIG_DIR_VAR).ok().as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
    .map(|dir| dir.join(DATASETS_DIR))
}

/// Publish both registries' item vocabularies, then read the layout at
/// `path`.
///
/// `None` means persistence is off for this run, which reports
/// [`LoadOutcome::NoFile`] — the same outcome as a first boot, because it is
/// the same situation from the window's point of view: nothing to restore.
///
/// Never fails. Every way the file can be bad resolves to the default
/// arrangement plus a reason, because a layout file is not worth refusing to
/// start over.
#[must_use]
pub fn boot_layout(path: Option<&Path>) -> (SavedLayout, LoadOutcome) {
    crate::app::publish_item_ids();
    crate::protocol::publish_item_ids();
    match path {
        Some(path) => persist::load(path, default_layout),
        None => (default_layout(), LoadOutcome::NoFile),
    }
}

/// Whether a load handed back a window geometry the **user** chose, rather
/// than the default one.
///
/// Not the same question as [`LoadOutcome::restored`], and the difference is a
/// real defect if they are conflated. `restored()` answers "is everything on
/// screen something you arranged", which [`LoadOutcome::Incomplete`] answers
/// `false` to because the arrangement was rebuilt from the default. But an
/// `Incomplete` file still *carried* the size and position the user last left
/// the window at, and those are fine. Resizing their window because an upgrade
/// added a pane is a change they did not ask for and cannot undo except by
/// resizing it back.
#[must_use]
pub const fn kept_window_geometry(outcome: LoadOutcome) -> bool {
    matches!(outcome, LoadOutcome::Restored | LoadOutcome::Incomplete)
}

/// What the window opens on: the spec named on the command line, or failing
/// that whatever the layout remembers was open, or failing that nothing.
///
/// The precedence is the decision worth stating rather than deriving. A spec
/// on the command line wins because you asked for *that* one, and being shown
/// something else because it is where you left off would be the window arguing
/// with you. With nothing named there is no such instruction, so the remembered
/// start stands — that is the whole of the front door "morphing": a launch with
/// work to restore restores it, and a surface with content in it is no longer
/// an invitation.
///
/// A remembered start that will not load is reported and dropped rather than
/// propagated. It can only mean the id was written by a build that shipped a
/// start this one does not, and refusing to open a window over that would turn
/// a stale line in a config file into a product that will not start.
///
/// # Nothing here chooses what the window looks at
///
/// A start used to name the view it filled, and a *restored* start had to be
/// stripped of that opinion so the layout file's own recorded view could
/// stand. Both halves are gone: the window has one arrangement, and what the
/// canvas holds is [`CanvasHolds`](crate::window::CanvasHolds), a latch
/// reconciled each frame from the documents through
/// [`graph_takes_the_canvas`](crate::window::graph_takes_the_canvas) —
/// except where a reader has chosen it directly, by clicking the graph chip.
/// So a boot carries documents and nothing else, and this function's whole
/// job is deciding **which** documents.
///
/// # Errors
///
/// Only from `spec`: a file that cannot be read, a Protocol manifest without
/// the offline gate, or a spec the pipeline rejects. Those are worth failing
/// on, because the user named them.
pub fn opening_boot(
    spec: Option<&str>,
    opened: Option<&str>,
    flow: brightfield_protocol::layout::Flow,
    sample: Option<brightfield_sql::ir::SampleRate>,
) -> Result<crate::window::Boot, String> {
    if let Some(spec) = spec {
        return crate::window::Boot::open_sampled(spec, flow, None, sample);
    }
    let Some(id) = opened else {
        return Ok(crate::window::Boot::empty());
    };
    Ok(crate::window::Boot::start(id, flow).unwrap_or_else(|e| {
        eprintln!("could not reopen {id}: {e}");
        crate::window::Boot::empty()
    }))
}
