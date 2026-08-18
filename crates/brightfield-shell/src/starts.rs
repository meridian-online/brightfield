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
//!   thumbnail `include_bytes!`-ed at compile time, so there is no path to get
//!   wrong: a start works from any working directory and cannot be broken by
//!   moving the checkout. Each start ships **two** thumbnails, one per
//!   [`Mode`], and [`Start::thumbnail_for`] picks between the two compiled-in
//!   byte slices — never a file path, so the property holds for the pair the
//!   same way it held for the one. What ships is the *spec*, which is not the
//!   same as what the spec reads — see [`Start::remote`] below, and the
//!   narrowing it forced on the sentence that used to end this bullet. (Note
//!   what the no-network claim is NOT, for the starts that keep it: `ureq` and `rustls`
//!   do reach this binary, as normal dependencies of `arc` by way of
//!   `brightfield-protocol`. The property that holds is behavioural — those
//!   starts open without touching the network — not an absence from the
//!   dependency graph. Do not restate it as the latter; it is false and
//!   trivially falsifiable with `cargo tree`.)
//! - **Choosing one lands on a result.** [`load`] returns a *composed*
//!   dashboard or a *built* asset graph, not a file path and not an editor
//!   buffer. A front door whose second click opens a blank surface has moved
//!   the blank canvas rather than removed it.
//! - **Their data is a table the engine queries.** Every chart start's
//!   `data:` entries resolve to DuckDB views the engine materialises —
//!   generated, aggregated and ordered where the data lives. None of them is
//!   a YAML file of inline numbers: inline rows are a testing affordance under
//!   the emitter's 1000-row cap, and several of the starts deliberately query
//!   tables past that cap to keep the distinction honest. Where the table
//!   comes from varies — the generated starts declare it in SQL, and
//!   [`CROSSWALK_CHART`] reads a published Parquet — but what the chart draws
//!   is a query result either way, never a literal.
//!
//! # Why an id rather than a path
//!
//! The id is what [`SavedLayout::opened`](brightfield_workbench::SavedLayout)
//! records, so a later launch can restore what was open. An id cannot name a
//! file that has since been deleted, so restoring one has no failure path to
//! design; and an id this build does not recognise resolves to `None`, which
//! means the same thing as never having opened anything.
//!
//! # Why these five
//!
//! The crosswalk is the anchor, and it appears twice — as the declaration and
//! as the result.
//!
//! The **manifest** is the vendored EDGAR ↔ GLEIF Protocol under
//! `examples/protocol/`, and it is the most legible thing this product
//! renders: a ten-file fan-in, a parameterised fetch-and-extract family, a
//! long-running operator step, a validation gate and one terminal sink. It
//! also loads with no run behind it and no engine — `build_graph` reads the
//! declared steps, so the `https://` inputs in the manifest are graph nodes
//! that are never fetched.
//!
//! The **chart** is what that Protocol produces, once someone has run it and
//! published the Parquet: the whole crosswalk, read live over httpfs and drawn
//! as two aggregating marks. It is the only start whose data is not inside the
//! binary, which is what [`Start::remote`] exists to say out loud. It is here
//! because a lineage graph cannot be filtered, panned or brushed, and a
//! product whose gallery offers only the declaration has nothing on it to
//! demonstrate the verbs with.
//!
//! The three generated chart starts each land on a different drawn answer — a
//! two-plot dashboard, a histogram, a ranked bar chart — and each is a
//! self-contained proof of the same architecture: the spec declares queries,
//! the engine holds the tables, the chart reads the result. Between three and
//! five is the deliberate size of this set — few enough to choose from at a
//! glance, enough that the gallery reads as a gallery.
//!
//! The chart view's empty pane offers the first chart start; the protocol
//! view's empty canvas offers the crosswalk manifest. Neither switches the
//! view out from under the click, neither empty pane offers a start that
//! needs the network, and the front door's gallery makes all five one click
//! away.
//!
//! # The one thing the crosswalk MANIFEST has to say out loud
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
//! # The one thing the crosswalk CHART has to say out loud
//!
//! It needs the network, and every start beside it opens without one — a
//! difference a user meets at the click, so it is disclosed at the click:
//! [`Start::remote`] is the property, [`REMOTE_MARK`] is the words, and
//! `a_start_that_reaches_the_network_says_so_on_its_own_button` is what keeps
//! the flag and the label from drifting apart — exactly the strut
//! [`Start::run_less`] already has.
//!
//! **What it does with no network is stated, not assumed.** The load fails —
//! DuckDB binds a view over an `https://` Parquet eagerly, so the failure
//! happens at open rather than at draw — and it fails as a structured
//! `EngineError` naming the network and the URL, which
//! [`MeridianApp::open_start`](crate::window::MeridianApp) raises as an error
//! banner over a window that stays up. It is not a blank chart and not a
//! silent partial one. `crates/brightfield-shell/tests/crosswalk_chart.rs`
//! holds that offline, hermetically, by denying the engine the httpfs
//! extension rather than by unplugging anything.
//!
//! # The thumbnails are shipped product surface
//!
//! Each start carries a pre-rendered PNG of what its click lands on, drawn on
//! the front door's gallery card — one for each [`Mode`], selected by
//! [`Start::thumbnail_for`] rather than re-rendered, so a card is never mid
//! composition on the surface that decides whether a stranger keeps looking.
//! They are committed under `assets/starts/` and regenerated by a test from
//! the bundled specs — rendering them live at startup was considered and
//! rejected (slow, and a render that can fail has no business on a first-run
//! surface). The regeneration test is also the gate that fails if a bundled
//! start stops rendering at all — for every start whose spec that test can
//! compose without a network, which is the whole set bar [`CROSSWALK_CHART`];
//! that one's render gate is the network-gated test beside it, run on demand
//! rather than in CI.

use brightfield_workbench::ViewKind;

use crate::design::Mode;
use crate::pipeline::{Composed, LiveDashboard};
use crate::protocol::{load_protocol_str, ProtocolInputs};

/// The EDGAR ↔ GLEIF crosswalk Protocol — the anchor of the set.
pub const CROSSWALK: &str = "edgar-gleif-crosswalk";
/// The published EDGAR ↔ GLEIF crosswalk, charted: the same subject as
/// [`CROSSWALK`] on the far side of a run and a publish. The one start whose
/// data is fetched rather than shipped — see [`Start::remote`].
pub const CROSSWALK_CHART: &str = "edgar-gleif-crosswalk-chart";
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
    /// A pre-rendered PNG of what this start opens onto in [`Mode::Light`],
    /// drawn on its gallery card. Committed under `assets/starts/` and held
    /// against the bundled spec by a regeneration test — never rendered live
    /// at startup. Read through [`Start::thumbnail_for`] rather than
    /// directly, so a caller cannot draw this and forget its dark
    /// counterpart.
    pub thumbnail: &'static [u8],
    /// The same picture in [`Mode::Dark`] — a second, independently
    /// pre-rendered PNG, not the light one recoloured at draw time. Held
    /// against the bundled spec by the same regeneration test, over the
    /// dark capture path.
    pub thumbnail_dark: &'static [u8],
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
    /// Whether opening this reaches the **network** — its spec reads a source
    /// this binary does not carry.
    ///
    /// The bullet at the top of this module says a start's spec and thumbnail
    /// ship inside the binary. They do, for every start. What does not
    /// necessarily ship is the *data the spec reads*, and a user meets that
    /// difference at the click: with no connection this start does not open,
    /// where the others do. So it is disclosed at the click, by
    /// [`REMOTE_MARK`] in the [`label`](Self::label), on the same strut
    /// [`run_less`](Self::run_less) uses — drop the mark and the test fails
    /// rather than the disclosure quietly disappearing.
    ///
    /// It is also what the hermetic front-door gates read to decide which
    /// starts they can load and render without a network. A `remote` start is
    /// covered by the `#[ignore]`d network tests beside them instead; the
    /// hermetic pass asserts how many it skipped, so the exemption cannot
    /// swallow the whole set.
    pub remote: bool,
    /// The chart spec this start opens — the bytes [`load`] composes — and
    /// `None` for a start that opens a Protocol manifest instead.
    ///
    /// **The single declaration.** [`load`] composes this field rather than
    /// dispatching to a second table of `include_str!`s keyed by id, so a
    /// check reading it here reads what a click opens. The field alone does
    /// not make that true — [`load`] still carries a by-id arm for the
    /// manifest start, and an arm added beside it could open a chart from
    /// other bytes. `the_spec_a_start_carries_is_the_spec_its_click_opens` is
    /// what holds the two together: it pins this field's presence to the
    /// declared [`Start::view`], and compares what [`load`] composes against
    /// this text for every start that opens without a network.
    ///
    /// Public because what a start *declares* is decidable without composing
    /// it, and for [`CROSSWALK_CHART`] that is the only way it is decidable at
    /// all: composing that one fetches. The selection producers a start ships
    /// with, and the claims its own header comment makes about them, are read
    /// off these bytes by `crates/brightfield-shell/tests/start_interaction.rs`.
    pub spec: Option<&'static str>,
}

impl Start {
    /// This start's thumbnail for `mode` — one of the two compiled-in byte
    /// slices [`thumbnail`](Self::thumbnail) or
    /// [`thumbnail_dark`](Self::thumbnail_dark), picked here rather than by a
    /// file path a caller could get wrong.
    #[must_use]
    pub fn thumbnail_for(&self, mode: Mode) -> &'static [u8] {
        match mode {
            Mode::Light => self.thumbnail,
            Mode::Dark => self.thumbnail_dark,
        }
    }
}

/// What a [`Start::run_less`] start's label has to contain.
///
/// A literal rather than a formatting rule so a test can assert it and a
/// translator cannot lose it by accident.
pub const RUN_LESS_MARK: &str = "(no run)";

/// What a [`Start::remote`] start's label has to contain — the same device as
/// [`RUN_LESS_MARK`], for the other property a click cannot take back.
pub const REMOTE_MARK: &str = "over the network";

/// Every shipped starting point, in the order the front door's gallery shows
/// them — the crosswalk anchors, so it comes first.
///
/// The one declaration: the empty states and the front door read their
/// affordances' labels and ids from here, [`load`] composes the
/// [`Start::spec`] carried on the entry, and the boot path resolves the
/// recorded id through the same list. A start added here with neither a
/// `spec:` nor a manifest arm in [`load`] fails there loudly rather than
/// becoming a dead button — `every_shipped_start_loads_into_a_document_with_something_in_it`
/// is the test that reaches that refusal.
pub const STARTS: &[Start] = &[
    Start {
        id: CROSSWALK,
        label: "Open the EDGAR ↔ GLEIF crosswalk (no run)",
        summary: "A real ten-source protocol, fetch to validated crosswalk, \
                  as a lineage graph.",
        view: ViewKind::Protocol,
        thumbnail: include_bytes!("../assets/starts/edgar-gleif-crosswalk.png"),
        thumbnail_dark: include_bytes!("../assets/starts/edgar-gleif-crosswalk-dark.png"),
        run_less: true,
        remote: false,
        // A Protocol manifest, not a chart spec — see `load`.
        spec: None,
    },
    Start {
        id: DASHBOARD,
        label: "Open the signals dashboard",
        summary: "A year of generated daily readings beside their weekday \
                  profile.",
        view: ViewKind::Charts,
        thumbnail: include_bytes!("../assets/starts/signals-dashboard.png"),
        thumbnail_dark: include_bytes!("../assets/starts/signals-dashboard-dark.png"),
        run_less: false,
        remote: false,
        spec: Some(DASHBOARD_SPEC),
    },
    Start {
        id: DISTRIBUTION,
        label: "Open the reading distribution",
        summary: "Five thousand generated samples, binned into a histogram \
                  in-engine.",
        view: ViewKind::Charts,
        thumbnail: include_bytes!("../assets/starts/reading-distribution.png"),
        thumbnail_dark: include_bytes!("../assets/starts/reading-distribution-dark.png"),
        run_less: false,
        remote: false,
        spec: Some(DISTRIBUTION_SPEC),
    },
    Start {
        id: BREAKDOWN,
        label: "Open the activity breakdown",
        summary: "Generated events, aggregated and ranked by the engine.",
        view: ViewKind::Charts,
        thumbnail: include_bytes!("../assets/starts/activity-breakdown.png"),
        thumbnail_dark: include_bytes!("../assets/starts/activity-breakdown-dark.png"),
        run_less: false,
        remote: false,
        spec: Some(BREAKDOWN_SPEC),
    },
    // Last on purpose. `for_view` hands an empty pane the FIRST start for its
    // view, and an empty state whose one button can fail for want of a
    // connection is a worse empty state than the one it replaced. The gallery
    // offers this alongside the rest; the pane's single button stays local.
    Start {
        id: CROSSWALK_CHART,
        label: "Open the EDGAR ↔ GLEIF crosswalk chart (over the network)",
        summary: "The whole published crosswalk, binned and grouped in-engine \
                  — read live over https.",
        view: ViewKind::Charts,
        thumbnail: include_bytes!("../assets/starts/edgar-gleif-crosswalk-chart.png"),
        thumbnail_dark: include_bytes!("../assets/starts/edgar-gleif-crosswalk-chart-dark.png"),
        run_less: false,
        remote: true,
        spec: Some(CROSSWALK_CHART_SPEC),
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

/// A loaded chart start: the first composition, and the session it came off.
///
/// The session is carried rather than dropped because
/// [`ChartDoc::reflow_to`](crate::app::ChartDoc::reflow_to) re-composites
/// through it. A document opened without one holds the size its spec declares
/// for the life of the window, whatever box the pane hands it — which is what
/// `a_shipped_start_relays_out_when_its_pane_changes_size` holds.
pub struct OpenedChart {
    /// The live session behind the dashboard, held across frames.
    pub live: LiveDashboard,
    /// The composition the load produced, at the spec's own declared size.
    pub composed: Composed,
}

/// A loaded start: the document it produced, and therefore the view it fills.
pub enum Opened {
    /// A composed dashboard for the charts view, and its live session.
    Charts(Box<OpenedChart>),
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

/// The crosswalk chart, included from `examples/` rather than copied into
/// `assets/starts/` — the same arrangement the private `CROSSWALK_MANIFEST`
/// has, and for the same reason: one file, so the spec a reader runs from the
/// checkout and the spec the button opens cannot drift.
///
/// Public so a test can hold the shipped bytes to the mark family this start
/// is *for*, with no engine, no data and no network.
pub const CROSSWALK_CHART_SPEC: &str =
    include_str!("../../../examples/remote/edgar-gleif-crosswalk.yaml");

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
/// The generated chart starts' data is declared as SQL queries in their specs,
/// so composing one touches no file — `base_dir` is `None` and there is
/// nothing for a relative `file:` to resolve against; the engine materialises
/// the declared views and executes the marks over them. The crosswalk's graph
/// is derived from its declared steps, so it touches no file and no network
/// either.
///
/// [`CROSSWALK_CHART`] is the exception, and the only one: its `file:` source
/// is an `https://` URL, so composing it fetches. That is what
/// [`Start::remote`] declares and what the label discloses.
///
/// # Errors
///
/// If `id` is not a start this build ships, or if the embedded fixture fails
/// to compose. For every start but [`CROSSWALK_CHART`] the second is a
/// build-time defect rather than a user's circumstance —
/// `every_shipped_start_loads_into_a_document_with_something_in_it` and the
/// thumbnail regeneration test in `tests/front_door.rs` are what keep it from
/// reaching a user. For [`CROSSWALK_CHART`] it is *also* the ordinary offline
/// answer: a structured engine error naming the network and the URL, which the
/// caller raises as a banner rather than swallowing.
pub fn load(id: &str) -> Result<Opened, String> {
    let Some(start) = find(id) else {
        return Err(format!("no shipped starting point named {id:?}"));
    };
    // The chart arm reads the start's own [`Start::spec`] rather than a second
    // table keyed by id, so the bytes a check reads off the entry are the bytes
    // the click composes. The manifest arm stays keyed by id: a chart start
    // added with no `spec:` must fail loudly here, not silently open the
    // crosswalk's lineage graph.
    if let Some(spec) = start.spec {
        return chart(spec);
    }
    match id {
        CROSSWALK => load_protocol_str(CROSSWALK_MANIFEST, CROSSWALK_MODELS)
            .map(|inputs| Opened::Protocol(Box::new(inputs))),
        other => Err(format!(
            "the shipped starting point {other:?} declares neither a chart spec \
             nor a manifest loader"
        )),
    }
}

/// One chart start, loaded live and composed once — the shape every entry in
/// [`load`]'s chart arms takes.
///
/// The sampling policy is applied by [`LiveDashboard`]'s own constructor, so a
/// start is decided the same way a file opened from the command line is.
fn chart(spec: &str) -> Result<Opened, String> {
    let mut live = LiveDashboard::load_str(spec, None)?;
    let composed = live.present()?;
    Ok(Opened::Charts(Box::new(OpenedChart { live, composed })))
}
