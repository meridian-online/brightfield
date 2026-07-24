//! The starting points that ship inside the binary.
//!
//! A launch that names no spec has to open on *something*, and for a long time
//! that something was a hardcoded `examples/dashboard.yaml` read relative to
//! the working directory: from the repo root it silently opened a dashboard
//! nobody asked for, and from anywhere else it printed a read error and never
//! made a window at all. Both are the same defect — the product had no answer
//! to "I just started it".
//!
//! The answer is the front door — the window's own surface when nothing is
//! open — whose gallery offers these, and the three properties that make them
//! worth offering:
//!
//! - **They ship with the binary.** Every spec is `include_str!`-ed and every
//!   thumbnail `include_bytes!`-ed at compile time, so opening one fetches
//!   nothing and there is no path to get wrong: a start works from any working
//!   directory and cannot be broken by moving the checkout. (Note what this
//!   claim is NOT: `ureq` and `rustls` do reach this binary, as normal
//!   dependencies of `arc` by way of `brightfield-protocol`. The property that
//!   holds is behavioural — these starts open without touching the network —
//!   not an absence from the dependency graph. Do not restate it as the
//!   latter; it is false and trivially falsifiable with `cargo tree`.)
//! - **Choosing one lands on a result.** [`load`] returns a *composed*
//!   dashboard or a *built* asset graph, not a file path and not an editor
//!   buffer. A front door whose second click opens a blank surface has moved
//!   the blank canvas rather than removed it.
//! - **Their data is a table the engine queries.** Every chart start's
//!   `data:` entries are SQL queries the engine materialises as DuckDB views
//!   — generated, aggregated and ordered where the data lives. None of them
//!   is a YAML file of inline numbers: inline rows are a testing affordance
//!   under the emitter's 1000-row cap, and two of the starts deliberately
//!   query tables past that cap to keep the distinction honest.
//!
//! # Why an id rather than a path
//!
//! The id is what [`SavedLayout::opened`](brightfield_workbench::SavedLayout)
//! records, so a later launch can restore what was open. An id cannot name a
//! file that has since been deleted, so restoring one has no failure path to
//! design; and an id this build does not recognise resolves to `None`, which
//! means the same thing as never having opened anything.
//!
//! # Why these four
//!
//! The crosswalk is the anchor. It is the vendored EDGAR ↔ GLEIF manifest
//! under `examples/protocol/`, and it is the most legible thing this product
//! renders: a ten-file fan-in, a parameterised fetch-and-extract family, a
//! long-running operator step, a validation gate and one terminal sink. It
//! also loads with no run behind it and no engine — `build_graph` reads the
//! declared steps, so the `https://` inputs in the manifest are graph nodes
//! that are never fetched.
//!
//! The three chart starts around it each land on a different drawn answer —
//! a two-plot dashboard, a histogram, a ranked bar chart — and each is a
//! self-contained proof of the same architecture: the spec declares queries,
//! the engine holds the tables, the chart reads the result. Between three and
//! five is the deliberate size of this set — few enough to choose from at a
//! glance, enough that the gallery reads as a gallery.
//!
//! The chart view's empty pane offers the first chart start; the protocol
//! view's empty canvas offers the crosswalk. Neither switches the view out
//! from under the click, and the front door's gallery makes all four one
//! click away.
//!
//! # The one thing the crosswalk has to say out loud
//!
//! The crosswalk loads with no run behind it, and that is exactly what
//! [`protocol::run_less_manifest_refusal`](crate::protocol::run_less_manifest_refusal)
//! gates: this view's default input is an emitted Protocol+Run contract, a
//! manifest is a *declaration* of the same shape, and nothing on the canvas
//! tells them apart. A path handed in from outside needs
//! [`OFFLINE_VAR`](crate::protocol::OFFLINE_VAR) set before it will be drawn.
//!
//! A shipped start does not go through that gate, and the reason is not that
//! this binary vendored it — that would be a narrower rule than the recorded
//! one, and dressing up an exemption as a distinction. The reason is that the
//! disclosure the variable exists to force is made somewhere the variable
//! cannot reach: on the button, before the click. [`Start::run_less`] is what
//! makes that a property of the start rather than a promise about a string,
//! and `a_start_that_opens_a_run_less_manifest_says_so_on_its_own_button`
//! is what keeps the two from drifting apart.
//!
//! **Once, and then remembered.** The launch *after* that click reopens the
//! same start out of
//! [`SavedLayout::opened`](brightfield_workbench::SavedLayout) with no button
//! and no click anywhere in the path, and the restored graph carries `(no
//! run)` nowhere on it. So the exemption is disclosed at the pick and the
//! layout file is the memory of that pick — not re-disclosed each time it is
//! taken. `run_less_manifest_refusal` states that in full, including what the
//! memory can come from and what invalidates it.
//!
//! # The thumbnails are shipped product surface
//!
//! Each start carries a pre-rendered PNG of what its click lands on, drawn on
//! the front door's gallery card. They are committed under `assets/starts/`
//! and regenerated by a test from the bundled specs — rendering them live at
//! startup was considered and rejected (slow, and a render that can fail has
//! no business on a first-run surface). The regeneration test is also the
//! gate that fails if a bundled start stops rendering at all.

use brightfield_workbench::ViewKind;

use crate::pipeline::{compose_spec_str, Composed};
use crate::protocol::{load_protocol_str, ProtocolInputs};

/// The EDGAR ↔ GLEIF crosswalk Protocol — the anchor of the set.
pub const CROSSWALK: &str = "edgar-gleif-crosswalk";
/// The two-plot signals dashboard: a year of readings and their weekday
/// profile, both views over one generated table.
pub const DASHBOARD: &str = "signals-dashboard";
/// The histogram: five thousand generated samples, binned in-engine.
pub const DISTRIBUTION: &str = "reading-distribution";
/// The ranked bar chart: generated events, aggregated in-engine.
pub const BREAKDOWN: &str = "activity-breakdown";

/// One shipped starting point.
pub struct Start {
    /// The stable id. Recorded in the layout file; never shown to the user.
    pub id: &'static str,
    /// What the control that opens it is called — on an empty pane's button
    /// and on the front door's gallery card alike.
    pub label: &'static str,
    /// One line under the label on the gallery card: what the click lands on.
    pub summary: &'static str,
    /// The view it fills — and therefore the view whose empty state offers it.
    pub view: ViewKind,
    /// A pre-rendered PNG of what this start opens onto, drawn on its gallery
    /// card. Committed under `assets/starts/` and held against the bundled
    /// spec by a regeneration test — never rendered live at startup.
    pub thumbnail: &'static [u8],
    /// Whether what this opens is a *declaration* rather than a result — a
    /// Protocol manifest with no run behind it.
    ///
    /// A start that sets this is exempt from
    /// [`protocol::run_less_manifest_refusal`](crate::protocol::run_less_manifest_refusal)
    /// only because its [`label`](Self::label) makes the same disclosure at
    /// the moment of the click. The two are held together by
    /// [`RUN_LESS_MARK`] and the test that checks every start against it; drop
    /// the mark from the label and that test fails rather than the exemption
    /// quietly becoming a hole.
    ///
    /// A later launch reopens the same start with no click to disclose it —
    /// the exemption is made once at the pick and then remembered, and
    /// `run_less_manifest_refusal` is where that is stated in full.
    pub run_less: bool,
}

/// What a [`Start::run_less`] start's label has to contain.
///
/// A literal rather than a formatting rule so a test can assert it and a
/// translator cannot lose it by accident.
pub const RUN_LESS_MARK: &str = "(no run)";

/// Every shipped starting point, in the order the front door's gallery shows
/// them — the crosswalk anchors, so it comes first.
///
/// The one declaration: the empty states and the front door read their
/// affordances' labels and ids from here, [`load`] dispatches on the same ids,
/// and the boot path resolves the recorded id through the same list. A start
/// added here without a loader arm fails [`load`] loudly rather than becoming
/// a button that does nothing.
pub const STARTS: &[Start] = &[
    Start {
        id: CROSSWALK,
        label: "Open the EDGAR ↔ GLEIF crosswalk (no run)",
        summary: "A real ten-source protocol, fetch to validated crosswalk, \
                  as a lineage graph.",
        view: ViewKind::Protocol,
        thumbnail: include_bytes!("../assets/starts/edgar-gleif-crosswalk.png"),
        run_less: true,
    },
    Start {
        id: DASHBOARD,
        label: "Open the signals dashboard",
        summary: "A year of generated daily readings beside their weekday \
                  profile.",
        view: ViewKind::Charts,
        thumbnail: include_bytes!("../assets/starts/signals-dashboard.png"),
        run_less: false,
    },
    Start {
        id: DISTRIBUTION,
        label: "Open the reading distribution",
        summary: "Five thousand generated samples, binned into a histogram \
                  in-engine.",
        view: ViewKind::Charts,
        thumbnail: include_bytes!("../assets/starts/reading-distribution.png"),
        run_less: false,
    },
    Start {
        id: BREAKDOWN,
        label: "Open the activity breakdown",
        summary: "Generated events, aggregated and ranked by the engine.",
        view: ViewKind::Charts,
        thumbnail: include_bytes!("../assets/starts/activity-breakdown.png"),
        run_less: false,
    },
];

/// The start with this id, if this build has one.
#[must_use]
pub fn find(id: &str) -> Option<&'static Start> {
    STARTS.iter().find(|s| s.id == id)
}

/// The start that fills `view`, if it has one — the first of `view`'s starts,
/// which is what an empty pane's single button offers. The front door's
/// gallery offers all of them.
#[must_use]
pub fn for_view(view: ViewKind) -> Option<&'static Start> {
    STARTS.iter().find(|s| s.view == view)
}

/// A loaded start: the document it produced, and therefore the view it fills.
pub enum Opened {
    /// A composed dashboard for the charts view.
    Charts(Box<Composed>),
    /// A built asset graph for the protocol view.
    Protocol(Box<ProtocolInputs>),
}

impl Opened {
    /// The view this fills.
    #[must_use]
    pub const fn view(&self) -> ViewKind {
        match self {
            Opened::Charts(_) => ViewKind::Charts,
            Opened::Protocol(_) => ViewKind::Protocol,
        }
    }
}

const DASHBOARD_SPEC: &str = include_str!("../assets/starts/signals-dashboard.yaml");
const DISTRIBUTION_SPEC: &str = include_str!("../assets/starts/reading-distribution.yaml");
const BREAKDOWN_SPEC: &str = include_str!("../assets/starts/activity-breakdown.yaml");
const CROSSWALK_MANIFEST: &str =
    include_str!("../../../examples/protocol/edgar_gleif/arcform.yaml");

/// The crosswalk's `sql:` models, keyed exactly as its manifest spells them.
const CROSSWALK_MODELS: &[(&str, &str)] = &[
    (
        "models/sec_entities.sql",
        include_str!("../../../examples/protocol/edgar_gleif/models/sec_entities.sql"),
    ),
    (
        "models/load.sql",
        include_str!("../../../examples/protocol/edgar_gleif/models/load.sql"),
    ),
    (
        "models/tier.sql",
        include_str!("../../../examples/protocol/edgar_gleif/models/tier.sql"),
    ),
    (
        "models/package.sql",
        include_str!("../../../examples/protocol/edgar_gleif/models/package.sql"),
    ),
];

/// Load the start with this id, all the way to a renderable document.
///
/// The chart starts' data is declared as SQL queries in their specs, so
/// composing one touches no file — `base_dir` is `None` and there is nothing
/// for a relative `file:` to resolve against; the engine materialises the
/// declared views and executes the marks over them. The crosswalk's graph is
/// derived from its declared steps, so it touches no file and no network
/// either.
///
/// # Errors
///
/// If `id` is not a start this build ships, or if the embedded fixture fails
/// to compose. The second is a build-time defect rather than a user's
/// circumstance — `every_shipped_start_loads_into_a_document_with_something_in_it`
/// and the thumbnail regeneration test in `tests/front_door.rs` are what keep
/// it from reaching a user.
pub fn load(id: &str) -> Result<Opened, String> {
    match id {
        DASHBOARD => compose_spec_str(DASHBOARD_SPEC, None)
            .map(|composed| Opened::Charts(Box::new(composed))),
        DISTRIBUTION => compose_spec_str(DISTRIBUTION_SPEC, None)
            .map(|composed| Opened::Charts(Box::new(composed))),
        BREAKDOWN => compose_spec_str(BREAKDOWN_SPEC, None)
            .map(|composed| Opened::Charts(Box::new(composed))),
        CROSSWALK => load_protocol_str(CROSSWALK_MANIFEST, CROSSWALK_MODELS)
            .map(|inputs| Opened::Protocol(Box::new(inputs))),
        other => Err(format!("no shipped starting point named {other:?}")),
    }
}
