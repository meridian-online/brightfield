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
//!   byte slices, not a file path, so the property holds for the pair the
//!   same way it held for the one. What ships is the *spec*, which is not the
//!   same as what the spec reads — see [`Start::remote`] below, and the
//!   narrowing it forced on the sentence that used to end this bullet. (Note
//!   what the no-network claim is NOT, for the starts that keep it: `ureq` and `rustls`
//!   do reach this binary, as normal dependencies of `arc` by way of
//!   `brightfield-protocol`. The property that holds is behavioural — those
//!   starts open without touching the network — not an absence from the
//!   dependency graph. Do not restate it as the latter; it is false and
//!   trivially falsifiable with `cargo tree`.)
//! - **Choosing one lands on a result.** A front door whose second click opens
//!   a blank surface has moved the blank canvas rather than removed it, so
//!   [`load`] returns a *composed* dashboard or a *built* asset graph rather
//!   than an editor buffer. It returns a **path** for exactly one kind of
//!   start — one that opens a data file ([`Start::data`]) — and that is not
//!   the exception this bullet is guarding against: the caller's very next
//!   call opens the path through [`crate::data_file::open`], which is where a
//!   composed dashboard and a built graph both come from for a file the reader
//!   picked themselves. Handing the path over is what makes those two routes
//!   one route rather than two that agree.
//! - **Their data is a table the engine queries.** Every chart start's
//!   `data:` entries resolve to DuckDB views the engine materialises —
//!   generated, aggregated and ordered where the data lives. None of them is
//!   a YAML file of inline numbers: inline rows are a testing affordance under
//!   the emitter's 1000-row cap, and several of the starts deliberately query
//!   tables past that cap to keep the distinction honest. Where the table
//!   comes from varies — the generated starts declare it in SQL,
//!   [`CROSSWALK_CHART`] reads a published Parquet, and
//!   [`CALIFORNIA_HOUSING`] fetches its authors' own file — but what the
//!   chart draws is a query result either way, not a literal.
//!
//! [`PALMER_PENGUINS`] and [`USGS_EARTHQUAKES`] are shipped Protocols
//! ([`Start::protocol`]) over well-known data bundled in their folders, so
//! their first run reads no network. [`NYC_YELLOW_TAXI`], [`ONLINE_RETAIL`] and
//! [`EPOCH_AI_MODELS`] are shipped Protocols whose first step fetches the
//! data from its issuer's own URL, so they are [`Start::remote`]: the taxi
//! month and the retail workbook are tens of megabytes, and Epoch AI's file
//! grows as models are released. Each folder under `examples/starts/` is
//! the Protocol a run of the `dashboard-from-data` skill wrote for its dataset,
//! with a Frictionless `datapackage.json` naming the source and its terms.
//!
//! # Why an id rather than a path
//!
//! The id is what [`SavedLayout::opened`](brightfield_workbench::SavedLayout)
//! records, so a later launch can restore what was open. An id cannot name a
//! file that has since been deleted, so restoring one has no failure path to
//! design; and an id this build does not recognise resolves to `None`, which
//! means the same thing as never having opened anything.
//!
//! # Why the door shows these six
//!
//! The door offers six cards, and each draws a different picture, so a reader
//! choosing between them is choosing a kind of answer rather than a dataset:
//!
//! - **Palmer Penguins** draws measures by species: bill shape in each of
//!   three groups, and the trend that reverses inside every one of them.
//! - **USGS earthquakes** draw a map: where a year's tremors struck, beside
//!   the weeks they came in.
//! - **NYC yellow taxi** draws the trips' rhythms: the weekday rush and the
//!   weekend night, hour by hour.
//! - **Online Retail II** draws customers over time: who comes back, month by
//!   month.
//! - **Epoch AI's notable models** draw training compute over time, against the
//!   year each model was published.
//! - **The crosswalk run** draws one Protocol as the run it records: every
//!   step's state, and the run it came from.
//!
//! California Housing, the crosswalk declaration and the crosswalk chart leave
//! the door but stay in the binary: they still sit in [`STARTS`], still
//! resolve through [`find`] and still load, and the crosswalk declaration is
//! still the first start to fill the protocol view, so that view's empty
//! pane offers it as before. What they lose is a card on the first screen,
//! which [`Start::on_door`] decides and [`on_door`] iterates. The generated
//! chart starts left the door earlier for the reason that remains true of
//! them: they are generated data with nothing to do to it. The paragraphs
//! below describe the set the binary carries, all twelve starts, not the six
//! the door shows.
//!
//! The **manifest** is the vendored EDGAR ↔ GLEIF Protocol under
//! `examples/protocol/`, and it is the most legible thing this product
//! renders: a ten-file fan-in, a parameterised fetch-and-extract family, a
//! long-running operator step, a validation gate and one terminal sink. It
//! also loads with no run behind it and no engine — `build_graph` reads the
//! declared steps, so the `https://` inputs in the manifest are graph nodes
//! that are never fetched.
//!
//! The **run** is [`CROSSWALK_RUN`], and it is the manifest's other state:
//! an emitted Protocol+Run contract for the same Protocol, so the steps on it
//! carry the states a run recorded and the ledger's strip says what that run
//! came to —
//! `crates/brightfield-shell/tests/protocol_run_start.rs`'s
//! `the_run_start_lands_on_a_graph_where_no_step_reports_never_run` walks them. It is not an execution — brightfield runs no step, and running a
//! Protocol belongs to `arc` — it is the *artefact* a run emits, read the way
//! this view was built to read one. Until it shipped, every Protocol this
//! binary could open was a declaration, so every surface that exists to report
//! a run reported the same nothing on every screen a stranger could reach.
//!
//! It does not retire the run-less start: the door shows the run and leaves the
//! declaration to the protocol view's empty pane, but a Protocol with no run
//! behind it is a real state an analyst meets on their own first manifest — the
//! state the `(no run)` disclosure exists for.
//!
//! The **chart** is what that Protocol produces, once someone has run it and
//! published the Parquet: the whole crosswalk, read live over httpfs and drawn
//! as two aggregating marks. Its data is not inside the binary, which is what
//! [`Start::remote`] exists to say out loud. It is here
//! because a lineage graph cannot be filtered, panned or brushed, and a
//! product whose binary carries only the declaration and the run has nothing
//! on it to demonstrate the verbs with. It is not on the door, which draws the
//! run and the five real-data starts.
//!
//! The three generated chart starts each land on a different drawn answer — a
//! two-plot dashboard, a histogram, a ranked bar chart — and each is a
//! self-contained proof of the same architecture: the spec declares queries,
//! the engine holds the tables, the chart reads the result. The
//! deliberate size of the **gallery** is six — few enough to choose from at a
//! glance, enough that it reads as a gallery — and the gallery is what
//! [`Start::on_door`] answers, not the length of [`STARTS`].
//!
//! The chart view's empty pane offers the first chart start; the protocol
//! view's empty canvas offers the crosswalk manifest — [`for_pane`] hands a
//! pane the FIRST start that fills it, and the manifest is still first.
//! Neither switches the view out from under the click, and neither empty pane
//! offers a start that needs the network. The front door's Datasets section offers the starts
//! that declare [`Start::on_door`], which are the six above.
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
//! It needs the network, and the starts that are not remote open without one
//! — a difference a user meets at the click, so it is disclosed at the click:
//! [`Start::remote`] is the property, [`REMOTE_MARK`] is the words, and
//! `a_start_that_reaches_the_network_says_so_on_its_own_button` is what keeps
//! the flag and the label from drifting apart — exactly the strut
//! [`Start::run_less`] already has.
//!
//! **What it does with no network is stated, not assumed.** The load fails,
//! and it fails naming the network and the URL, which
//! [`MeridianApp::open_start`](crate::window::MeridianApp) raises as an error
//! banner over a window that stays up. It is not a blank chart and not a
//! silent partial one.
//!
//! *Which* code says so moved when the fetch did. DuckDB binds a view over an
//! `https://` Parquet **eagerly**, so the whole download used to happen inside
//! the frame that took the click and the refusal was the engine's
//! `EngineError::RemoteSourceFailed`. [`crate::remote`] now moves the bytes to
//! a local file on a worker first, so what the engine is handed is a `file:`
//! source it can read at local speed and the refusal a user meets is the
//! fetch's — worded to name the same two things, because the banner is the
//! same banner. Both are still true and both are held: the engine's is held
//! hermetically by `crates/brightfield-shell/tests/crosswalk_chart.rs`, which
//! denies it the httpfs extension rather than unplugging anything, and the
//! fetch's by `crates/brightfield-shell/tests/remote_start.rs`, over a port on
//! localhost with nothing listening on it.
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
//! start stops rendering at all, and it covers the whole set on every push.
//! [`CROSSWALK_CHART`] is drawn there from a committed reduced copy of the
//! published table rather than over the network, through [`load_with`]: the
//! chart reads the tier, the method and the length of the search text, so a
//! copy keeping those three draws the same bins and the same counts at a
//! fortieth of the size. `scripts/cut-crosswalk-fixture.sh` cuts it and says
//! why; the nightly network run checks the published table has not moved
//! since. [`CALIFORNIA_HOUSING`]'s are drawn the same way, through
//! [`load_with`], from a committed copy of the archive its authors gave
//! StatLib, which `crates/brightfield-shell/tests/data/houses.json`
//! accounts for.

use brightfield_workbench::ItemId;

use crate::design::Mode;
use crate::pipeline::{Composed, LiveDashboard};
use crate::protocol::{load_contract_str, load_protocol_str, ProtocolInputs};

/// The EDGAR ↔ GLEIF crosswalk Protocol — the anchor of the set.
pub const CROSSWALK: &str = "edgar-gleif-crosswalk";
/// The EDGAR ↔ GLEIF crosswalk as a Protocol that **has run** — the same
/// subject as [`CROSSWALK`] with a run behind it.
///
/// The pair is the point, and it is why this does not replace the run-less
/// start beside it: a declaration and a result are two states an analyst meets,
/// the second on their own first manifest, and the door offers both.
pub const CROSSWALK_RUN: &str = "edgar-gleif-crosswalk-run";
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
/// California Housing: its authors' own file, fetched from StatLib and
/// opened as the one-step Protocol a file picked through the dialog opens as.
///
/// The start that is a **data file** rather than a spec or a manifest — see
/// [`Start::data`] for what that changes and [`load`]'s data arm for how it
/// opens. Its data is not in the binary: see [`crate::housing`] for why.
pub const CALIFORNIA_HOUSING: &str = "california-housing";
/// Palmer Penguins: a shipped Protocol over the palmerpenguins package's own
/// CSV, bundled, whose panel lays bill length against bill depth by species.
pub const PALMER_PENGUINS: &str = "palmer-penguins";
/// The USGS's own earthquakes of magnitude 4.5 and up in 2025: a shipped
/// Protocol over a bundled Parquet cut by `scripts/cut-usgs-earthquakes.sh`,
/// whose panel is a map of where they struck beside their weeks.
pub const USGS_EARTHQUAKES: &str = "usgs-earthquakes-2025";
/// The NYC Taxi and Limousine Commission's yellow taxi trips of January 2026:
/// a shipped Protocol that fetches the TLC's own month of trips, whose panel
/// lays the trips out by hour and day of the week.
pub const NYC_YELLOW_TAXI: &str = "nyc-yellow-taxi";
/// UCI's Online Retail II: a shipped Protocol that fetches the UCI Machine
/// Learning Repository's own archive, whose panel shows each month's orders by
/// whether the customer was new or returning.
///
/// Its first run reaches a second host as well: the workbook is read with
/// DuckDB's `excel` extension, which DuckDB downloads from
/// extensions.duckdb.org when it is not installed already.
pub const ONLINE_RETAIL: &str = "online-retail-ii";
/// Epoch AI's notable AI models: a shipped Protocol that fetches Epoch AI's
/// own file, whose panel lays training compute against the date each model
/// was published.
pub const EPOCH_AI_MODELS: &str = "epoch-ai-models";

/// A data file a start **fetches**, and the name the click writes it under.
///
/// One struct rather than four fields on [`Start`], because they are one
/// fact: a URL with no name has no place to land, a name with no way to make
/// its file from what was fetched cannot be written, and a descriptor
/// belonging to another source misdescribes the one beside it. An
/// `Option<FetchedData>` therefore has two states.
pub struct FetchedData {
    /// The name the file is written under — and therefore what the locator
    /// band says, what the one SQL step reads, and (as its stem) what the
    /// table the Protocol produces is called. See [`crate::one_step`].
    ///
    /// A **name**, not a path: [`dataset_path`] decides the directory, so a
    /// start cannot name somewhere outside it.
    pub file_name: &'static str,
    /// Where the source is fetched from.
    pub url: &'static str,
    /// Which shape of file [`Self::prepare`] writes, as part of
    /// [`dataset_path`]: raised when a change to `prepare` changes its output,
    /// so a file an earlier build wrote is fetched again rather than reopened
    /// as this build's.
    pub revision: u32,
    /// Turns the fetched file, its first argument, into the data file at its
    /// second.
    pub prepare: fn(&std::path::Path, &std::path::Path) -> Result<(), String>,
    /// The Frictionless descriptor of what [`Self::prepare`] writes: its
    /// columns, where they came from, and the terms the source states.
    ///
    /// Shipped beside the start because a third-party dataset has to carry
    /// its provenance with it, and read by
    /// `the_fetched_dataset_is_described_by_its_own_descriptor` so it is a
    /// gate rather than a note: its `path` is held against
    /// [`Self::file_name`], its source against [`Self::url`], and its columns
    /// against the file [`Self::prepare`] writes.
    pub descriptor: &'static str,
}

/// One shipped starting point.
pub struct Start {
    /// The stable id. Recorded in the layout file; never shown to the user.
    pub id: &'static str,
    /// What the control that opens it is called — on an empty pane's button
    /// and on the front door's gallery card alike.
    pub label: &'static str,
    /// One line under the label on the gallery card: what the click lands on.
    pub summary: &'static str,
    /// The pane it fills — and therefore the pane whose empty state offers
    /// it.
    ///
    /// A pane rather than a document, because that is the question every
    /// reader of this field actually asks: which empty state offers this
    /// start, and which pane's way-in rect does the front door record its card
    /// under. [`for_pane`] is the lookup.
    pub fills: ItemId,
    /// A pre-rendered PNG of what this start opens onto in [`Mode::Light`],
    /// drawn on its gallery card. Committed under `assets/starts/` and held
    /// against the bundled spec by a regeneration test — never rendered live
    /// at startup. Read through [`Start::thumbnail_for`] rather than
    /// directly.
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
    /// Whether opening this reaches the **network** — its spec, or its
    /// [`data`](Self::data), reads a source this binary does not carry.
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
    /// starts they can load without a network. A `remote` start's load is
    /// covered by the `#[ignore]`d network tests beside them instead; the
    /// hermetic pass asserts how many it skipped, so the exemption cannot
    /// swallow the whole set. Its thumbnails are not exempt: the regeneration
    /// test draws them from a committed copy of the data, through
    /// [`load_with`].
    pub remote: bool,
    /// Whether the front door's Datasets section offers this start.
    ///
    /// **Not the same question as whether the build ships it.** A start that
    /// clears this is still in [`STARTS`], still resolves through [`find`] and
    /// [`for_pane`], still loads through [`load`], and still reopens out of a
    /// recorded layout — so an empty pane's button and a restored session are
    /// unaffected by clearing it. What it loses is a card on the first screen.
    ///
    /// Hugh's look of 2026-09-05 is why the flag exists: the three generated
    /// chart starts draw a picture and offer nothing to do with it, over data
    /// no analyst has a reason to care about, and a first screen whose
    /// catalogue is mostly those is a first screen that argues against the
    /// product. The Datasets section is therefore the real subject — the
    /// five real-data starts and the crosswalk run — and the generated starts
    /// stay in the binary for the job they are actually good at: giving the
    /// chart pane's empty state a button that works with no network, and
    /// giving the tests a chart to compose without one.
    ///
    /// `the_door_offers_only_starts_that_declare_themselves_for_it` is what
    /// holds the section to this field rather than to the length of
    /// [`STARTS`].
    pub on_door: bool,
    /// The chart spec this start opens — the bytes [`load`] composes — and
    /// `None` for a data-file, Protocol-manifest, run-record or shipped
    /// Protocol start.
    ///
    /// **The single declaration.** [`load`] composes this field rather than
    /// dispatching to a second table of `include_str!`s keyed by id, so a
    /// check reading it here reads what a click opens. The field alone does
    /// not make that true — [`load`] still carries a by-id arm for the
    /// manifest start, and an arm added beside it could open a chart from
    /// other bytes. `the_spec_a_start_carries_is_the_spec_its_click_opens` is
    /// what holds the two together: it pins this field's presence to the
    /// declared [`Start::fills`], and compares what [`load`] composes against
    /// this text for every start that opens without a network.
    ///
    /// Public because what a start *declares* is decidable without composing
    /// it, and for [`CROSSWALK_CHART`] that is the only way it is decidable at
    /// all: composing that one fetches. The selection producers a start ships
    /// with, and the claims its own header comment makes about them, are read
    /// off these bytes by `crates/brightfield-shell/tests/start_interaction.rs`.
    pub spec: Option<&'static str>,
    /// The **data file** this start fetches and opens, or `None` for a chart,
    /// Protocol-manifest, run-record or shipped Protocol start.
    ///
    /// Exclusive with [`spec`](Self::spec) and [`protocol`](Self::protocol), and
    /// `a_start_declares_a_spec_or_a_data_file_and_never_both` is what holds
    /// that rather than [`load`]'s arm order quietly picking a winner.
    ///
    /// **What it buys is one route into a data file rather than two** —
    /// `the_card_and_the_picker_land_on_one_window` is what reads that back.
    ///
    /// [`load`] fetches the file, writes it to disk and hands back the path;
    /// the caller then opens that path the way it opens a path the reader
    /// picked out of the dialog — [`crate::window::Boot::data_file`], over
    /// [`crate::data_file::open`]. So the click and the picker cannot land on
    /// two different windows, because after the write they are the same call
    /// with the same argument. `the_card_and_the_picker_land_on_one_window` is
    /// what reads that back off two real opens.
    pub data: Option<FetchedData>,
    /// The **Protocol folder** this start writes, runs and opens on its
    /// panel, or `None` for a chart, data-file, Protocol-manifest or
    /// run-record start.
    ///
    /// Exclusive with [`spec`](Self::spec) and [`data`](Self::data), which
    /// `a_start_declares_a_spec_or_a_data_file_and_never_both` holds.
    ///
    /// [`load`] writes the folder to [`protocol_path`] and hands back where it
    /// is; the window then runs it with the embedded arc runner when it has
    /// not run, and opens its first panel beside its grid, so a click lands
    /// where an analyst's own run of the same Protocol would. A Protocol whose
    /// steps include an `http_fetch` reaches the network on its first run, and
    /// its start says so with [`REMOTE_MARK`] like any other remote start.
    pub protocol: Option<ShippedProtocol>,
}

/// A Protocol folder a start ships inside the binary: every file of it, by
/// its path relative to the folder.
///
/// The folder is written whole to [`protocol_path`], which names the
/// revision, so a build that changes the folder raises [`Self::revision`]
/// and writes it afresh beside the older one rather than reopening the older
/// one's files or the database its run made.
pub struct ShippedProtocol {
    /// The folder's name under its revision's directory.
    pub dir_name: &'static str,
    /// Which shape of folder this build ships, as part of [`protocol_path`].
    pub revision: u32,
    /// Each file of the folder, as `(path relative to the folder, bytes)`.
    pub files: &'static [(&'static str, &'static [u8])],
}

impl ShippedProtocol {
    /// The folder's `arcform.yaml`, when it ships one as text.
    #[must_use]
    pub fn manifest(&self) -> Option<&'static str> {
        self.files
            .iter()
            .find(|(path, _)| *path == arc::spec::MANIFEST_FILENAME)
            .and_then(|(_, bytes)| std::str::from_utf8(bytes).ok())
    }

    /// The URLs this Protocol's `http_fetch` steps fetch: what its first run
    /// reads over the network, and what makes its start [`Start::remote`].
    #[must_use]
    pub fn fetch_urls(&self) -> Vec<String> {
        self.manifest().map_or_else(Vec::new, fetch_urls)
    }
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
/// them: the crosswalk declaration first, because the protocol view's empty pane
/// offers the first start that fills it, then the door's own order.
///
/// The one declaration: the empty states and the front door read their
/// affordances' labels and ids from here, [`load`] composes the
/// [`Start::spec`] carried on the entry, and the boot path resolves the
/// recorded id through the same list. A start added here with no `spec:`, no
/// `data:` and no manifest arm in [`load`] fails there loudly rather than
/// becoming a dead button — `every_shipped_start_loads_into_a_document_with_something_in_it`
/// is the test that reaches that refusal.
pub const STARTS: &[Start] = &[
    Start {
        id: CROSSWALK,
        label: "Open the EDGAR ↔ GLEIF crosswalk (no run)",
        summary: "A real ten-source protocol, fetch to validated crosswalk, \
                  as a lineage graph.",
        fills: crate::protocol::CANVAS,
        thumbnail: include_bytes!("../assets/starts/edgar-gleif-crosswalk.png"),
        thumbnail_dark: include_bytes!("../assets/starts/edgar-gleif-crosswalk-dark.png"),
        run_less: true,
        remote: false,
        on_door: false,
        // A Protocol manifest, not a chart spec — see `load`.
        spec: None,
        data: None,
        protocol: None,
    },
    Start {
        id: DASHBOARD,
        label: "Open the signals dashboard",
        summary: "A year of generated daily readings beside their weekday \
                  profile.",
        fills: crate::app::CHART,
        thumbnail: include_bytes!("../assets/starts/signals-dashboard.png"),
        thumbnail_dark: include_bytes!("../assets/starts/signals-dashboard-dark.png"),
        run_less: false,
        remote: false,
        on_door: false,
        spec: Some(DASHBOARD_SPEC),
        data: None,
        protocol: None,
    },
    Start {
        id: DISTRIBUTION,
        label: "Open the reading distribution",
        summary: "Five thousand generated samples, binned into a histogram \
                  in-engine.",
        fills: crate::app::CHART,
        thumbnail: include_bytes!("../assets/starts/reading-distribution.png"),
        thumbnail_dark: include_bytes!("../assets/starts/reading-distribution-dark.png"),
        run_less: false,
        remote: false,
        on_door: false,
        spec: Some(DISTRIBUTION_SPEC),
        data: None,
        protocol: None,
    },
    Start {
        id: BREAKDOWN,
        label: "Open the activity breakdown",
        summary: "Generated events, aggregated and ranked by the engine.",
        fills: crate::app::CHART,
        thumbnail: include_bytes!("../assets/starts/activity-breakdown.png"),
        thumbnail_dark: include_bytes!("../assets/starts/activity-breakdown-dark.png"),
        run_less: false,
        remote: false,
        on_door: false,
        spec: Some(BREAKDOWN_SPEC),
        data: None,
        protocol: None,
    },
    // Two shipped Protocols whose data is in the folder, so their first run
    // reads no network. Each folder is the Protocol the `dashboard-from-data`
    // skill wrote for its dataset, with its Frictionless descriptor beside it.
    Start {
        id: PALMER_PENGUINS,
        label: "Open Palmer Penguins",
        summary: "344 Antarctic penguins: bill shape by species, and the \
                  trend that reverses inside each.",
        fills: crate::app::CHART,
        thumbnail: include_bytes!("../assets/starts/palmer-penguins.png"),
        thumbnail_dark: include_bytes!("../assets/starts/palmer-penguins-dark.png"),
        run_less: false,
        remote: false,
        on_door: true,
        spec: None,
        data: None,
        protocol: Some(ShippedProtocol {
            dir_name: PALMER_PENGUINS,
            revision: 1,
            files: PALMER_PENGUINS_FILES,
        }),
    },
    Start {
        id: USGS_EARTHQUAKES,
        label: "Open the USGS earthquakes of 2025",
        summary: "8,502 earthquakes of magnitude 4.5 and up, on a map beside \
                  the weeks they struck.",
        fills: crate::app::CHART,
        thumbnail: include_bytes!("../assets/starts/usgs-earthquakes-2025.png"),
        thumbnail_dark: include_bytes!("../assets/starts/usgs-earthquakes-2025-dark.png"),
        run_less: false,
        remote: false,
        on_door: true,
        spec: None,
        data: None,
        protocol: Some(ShippedProtocol {
            dir_name: USGS_EARTHQUAKES,
            revision: 1,
            files: USGS_EARTHQUAKES_FILES,
        }),
    },
    // Three shipped Protocols that fetch their data from its issuer's own URL
    // on the first run, so each says so on its button. Each folder is the
    // Protocol the `dashboard-from-data` skill wrote for its dataset, with its
    // Frictionless descriptor naming the issuer, the terms and the credit.
    Start {
        id: NYC_YELLOW_TAXI,
        label: "Open NYC yellow taxi trips (over the network)",
        summary: "January 2026's yellow cab trips from the TLC: the weekday \
                  rush and the weekend night, hour by hour.",
        fills: crate::app::CHART,
        thumbnail: include_bytes!("../assets/starts/nyc-yellow-taxi.png"),
        thumbnail_dark: include_bytes!("../assets/starts/nyc-yellow-taxi-dark.png"),
        run_less: false,
        remote: true,
        on_door: true,
        spec: None,
        data: None,
        protocol: Some(ShippedProtocol {
            dir_name: NYC_YELLOW_TAXI,
            revision: 1,
            files: NYC_YELLOW_TAXI_FILES,
        }),
    },
    Start {
        id: ONLINE_RETAIL,
        label: "Open Online Retail II (over the network)",
        summary: "Two years of a UK online shop's orders from UCI: who \
                  comes back, month by month.",
        fills: crate::app::CHART,
        thumbnail: include_bytes!("../assets/starts/online-retail-ii.png"),
        thumbnail_dark: include_bytes!("../assets/starts/online-retail-ii-dark.png"),
        run_less: false,
        remote: true,
        on_door: true,
        spec: None,
        data: None,
        protocol: Some(ShippedProtocol {
            dir_name: ONLINE_RETAIL,
            revision: 1,
            files: ONLINE_RETAIL_FILES,
        }),
    },
    Start {
        id: EPOCH_AI_MODELS,
        label: "Open Epoch AI's notable models (over the network)",
        summary: "Notable AI models from Epoch AI: training compute against \
                  the year each was published.",
        fills: crate::app::CHART,
        thumbnail: include_bytes!("../assets/starts/epoch-ai-models.png"),
        thumbnail_dark: include_bytes!("../assets/starts/epoch-ai-models-dark.png"),
        run_less: false,
        remote: true,
        on_door: true,
        spec: None,
        data: None,
        protocol: Some(ShippedProtocol {
            dir_name: EPOCH_AI_MODELS,
            revision: 1,
            files: EPOCH_AI_MODELS_FILES,
        }),
    },
    // The door's last card: the one Protocol shown as the run it records.
    Start {
        id: CROSSWALK_RUN,
        label: "Open a run of the EDGAR ↔ GLEIF crosswalk",
        summary: "The same protocol on the far side of a run: every step's \
                  recorded state, and the run it came from.",
        fills: crate::protocol::CANVAS,
        thumbnail: include_bytes!("../assets/starts/edgar-gleif-crosswalk-run.png"),
        thumbnail_dark: include_bytes!("../assets/starts/edgar-gleif-crosswalk-run-dark.png"),
        // **Not run-less**, which is the whole of what this start adds: its
        // graph carries the states a run recorded, so nothing about it needs
        // the disclosure [`RUN_LESS_MARK`] makes, and
        // `a_start_that_opens_a_run_less_manifest_says_so_on_its_own_button`
        // would fail if the label carried it anyway.
        run_less: false,
        remote: false,
        on_door: true,
        // A Protocol+Run contract, not a chart spec — see `load`.
        spec: None,
        data: None,
        protocol: None,
    },
    // Placed after the generated chart starts rather than beside the
    // crosswalks, and the position is `for_pane`'s rather than the door's.
    // It is off the door, but still loads and reopens. `for_pane` hands an
    // empty pane the FIRST start that fills it, and the chart pane's empty
    // state is not this card's question: changing which button that pane offers is a decision about that empty state, and it is
    // not one this start was added to make.
    Start {
        id: CALIFORNIA_HOUSING,
        label: "Open California Housing (over the network)",
        summary: "Californian census districts from StatLib: nine measures, \
                  and a map from their coordinates.",
        fills: crate::app::CHART,
        thumbnail: include_bytes!("../assets/starts/california-housing.png"),
        thumbnail_dark: include_bytes!("../assets/starts/california-housing-dark.png"),
        // **Not run-less.** A run-less start opens a Protocol *declaration*
        // with no result behind it; this one reads the file and produces the
        // table, so there is a result and nothing to disclose. That is the
        // same reading `crate::window::Boot::open` makes of a one-step
        // manifest, and `crate::protocol::run_less_manifest_refusal` is where
        // the distinction is written out.
        run_less: false,
        // The first open fetches the authors' archive from StatLib, so the
        // label says so. Later opens find the file already written and reach
        // no network — see `dataset_path`.
        remote: true,
        on_door: false,
        // A data file, not a chart spec and not a manifest — see `load`'s
        // data arm and `Start::data`.
        spec: None,
        data: Some(FetchedData {
            file_name: HOUSING_FILE,
            url: crate::housing::URL,
            revision: 1,
            prepare: crate::housing::unpack,
            descriptor: HOUSING_DESCRIPTOR,
        }),
        protocol: None,
    },
    // Last on purpose. `for_pane` hands an empty pane the FIRST start that
    // fills it, and an empty state whose one button can fail for want of a
    // connection is a worse empty state than the one it replaced. The gallery
    // offers this alongside the rest; the pane's single button stays local.
    Start {
        id: CROSSWALK_CHART,
        label: "Open the EDGAR ↔ GLEIF crosswalk chart (over the network)",
        summary: "The whole published crosswalk, binned and grouped in-engine \
                  — read live over https.",
        fills: crate::app::CHART,
        thumbnail: include_bytes!("../assets/starts/edgar-gleif-crosswalk-chart.png"),
        thumbnail_dark: include_bytes!("../assets/starts/edgar-gleif-crosswalk-chart-dark.png"),
        run_less: false,
        remote: true,
        on_door: false,
        spec: Some(CROSSWALK_CHART_SPEC),
        data: None,
        protocol: None,
    },
];

/// The start with this id, if this build has one.
#[must_use]
pub fn find(id: &str) -> Option<&'static Start> {
    STARTS.iter().find(|s| s.id == id)
}

/// The start that fills `pane`, if this build ships one — the first of that
/// pane's starts, which is what its empty state's single button offers.
///
/// Reads [`STARTS`] and not [`on_door`]: an empty pane's button is a different
/// surface from the front door's catalogue, and the property that button needs
/// is that it opens with no network, which
/// [`Start::remote`] is what says. The generated chart starts are off the door
/// and are still the answer here.
#[must_use]
pub fn for_pane(pane: ItemId) -> Option<&'static Start> {
    STARTS.iter().find(|s| s.fills == pane)
}

/// The starts the front door's Datasets section offers, in [`STARTS`] order.
///
/// See [`Start::on_door`] for what clearing the flag does and does not take
/// away.
pub fn on_door() -> impl Iterator<Item = &'static Start> {
    STARTS.iter().filter(|s| s.on_door)
}

/// How many starts [`on_door`] yields, at compile time.
///
/// The door's column is as wide as its Datasets row needs, and that width is a
/// `const` because the window's own geometry is derived from it before any
/// frame is drawn. A `const fn` over [`STARTS`] keeps the count and the flag
/// the same fact; deriving the width from `STARTS.len()` is what made a
/// two-card door lay out a five-card column.
#[must_use]
pub const fn on_door_count() -> usize {
    let mut count = 0;
    let mut i = 0;
    while i < STARTS.len() {
        if STARTS[i].on_door {
            count += 1;
        }
        i += 1;
    }
    count
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
    /// The files a [`Start::remote`] start's sources were fetched into, for
    /// the life of the document that reads them.
    ///
    /// `None` for a start that reads no network. Carried rather than
    /// dropped for the same reason `live` is, and a sharper one: the engine
    /// binds a **view** over the fetched path, so every query re-reads the
    /// file. Dropping this at the end of the open would delete the Parquet out
    /// from under the session that is about to be brushed.
    pub fetched: Option<crate::remote::Fetched>,
}

/// A loaded start: the document it produced.
///
/// The window holds both documents whichever one a start filled, so this says
/// which of the two to replace, and does not say what the window then *is* —
/// that is [`MeridianApp`](crate::window::MeridianApp)'s to derive from the
/// documents it ends up holding.
pub enum Opened {
    /// A composed dashboard, and its live session.
    Charts(Box<OpenedChart>),
    /// A built asset graph.
    Protocol(Box<ProtocolInputs>),
    /// A **data file**, now on disk at this path, for the caller to open the
    /// way it opens a file a reader picked out of the dialog.
    ///
    /// A path rather than a loaded document, and that is the point rather than
    /// an economy. The window already has one route into a data file —
    /// [`crate::window::Boot::data_file`] over [`crate::data_file::open`] —
    /// and handing back the path makes the click take **that** route rather
    /// than a second one composed here that would have to be kept in step with
    /// it. What the two routes then share is not an agreement; it is the same
    /// function called with the same argument.
    File(std::path::PathBuf),
    /// A **Protocol folder**, written at this path by a [`Start::protocol`]
    /// start, for the caller to run when it has not run and open on its
    /// panels.
    Panels(std::path::PathBuf),
}

/// The name [`CALIFORNIA_HOUSING`]'s file is written under.
///
/// Underscored rather than hyphenated like the start's id, because this is the
/// name a **reader** meets — in the locator band, in the one SQL step, and as
/// the table's own name, which [`crate::one_step`] derives from the stem by
/// mapping a non-alphanumeric byte to `_`. A hyphen would make the file and
/// the table read differently for no gain.
pub const HOUSING_FILE: &str = "california_housing.parquet";

/// The descriptor of the file [`CALIFORNIA_HOUSING`] writes — see
/// [`FetchedData::descriptor`].
pub const HOUSING_DESCRIPTOR: &str =
    include_str!("../assets/starts/california_housing.datapackage.json");

/// The files of the [`NYC_YELLOW_TAXI`] Protocol folder under `examples/starts/`, as its
/// start writes them. Its data is not among them: the first run fetches it.
const NYC_YELLOW_TAXI_FILES: &[(&str, &[u8])] = &[
    (
        "arcform.yaml",
        include_bytes!("../../../examples/starts/nyc-yellow-taxi/arcform.yaml"),
    ),
    (
        "models/01_raw_trips.sql",
        include_bytes!("../../../examples/starts/nyc-yellow-taxi/models/01_raw_trips.sql"),
    ),
    (
        "models/02_trips.sql",
        include_bytes!("../../../examples/starts/nyc-yellow-taxi/models/02_trips.sql"),
    ),
    (
        "schemas/raw_trips.json",
        include_bytes!("../../../examples/starts/nyc-yellow-taxi/schemas/raw_trips.json"),
    ),
    (
        "panels/dashboard.yaml",
        include_bytes!("../../../examples/starts/nyc-yellow-taxi/panels/dashboard.yaml"),
    ),
    (
        "datapackage.json",
        include_bytes!("../../../examples/starts/nyc-yellow-taxi/datapackage.json"),
    ),
];

/// The files of the [`ONLINE_RETAIL`] Protocol folder under `examples/starts/`, as its
/// start writes them. Its data is not among them: the first run fetches it.
const ONLINE_RETAIL_FILES: &[(&str, &[u8])] = &[
    (
        "arcform.yaml",
        include_bytes!("../../../examples/starts/online-retail-ii/arcform.yaml"),
    ),
    (
        "models/01_retail_lines.sql",
        include_bytes!("../../../examples/starts/online-retail-ii/models/01_retail_lines.sql"),
    ),
    (
        "models/02_retail_orders.sql",
        include_bytes!("../../../examples/starts/online-retail-ii/models/02_retail_orders.sql"),
    ),
    (
        "schemas/retail_lines.json",
        include_bytes!("../../../examples/starts/online-retail-ii/schemas/retail_lines.json"),
    ),
    (
        "panels/dashboard.yaml",
        include_bytes!("../../../examples/starts/online-retail-ii/panels/dashboard.yaml"),
    ),
    (
        "datapackage.json",
        include_bytes!("../../../examples/starts/online-retail-ii/datapackage.json"),
    ),
];

/// The files of the [`EPOCH_AI_MODELS`] Protocol folder under `examples/starts/`, as its
/// start writes them. Its data is not among them: the first run fetches it.
const EPOCH_AI_MODELS_FILES: &[(&str, &[u8])] = &[
    (
        "arcform.yaml",
        include_bytes!("../../../examples/starts/epoch-ai-models/arcform.yaml"),
    ),
    (
        "models/01_raw_models.sql",
        include_bytes!("../../../examples/starts/epoch-ai-models/models/01_raw_models.sql"),
    ),
    (
        "models/02_models.sql",
        include_bytes!("../../../examples/starts/epoch-ai-models/models/02_models.sql"),
    ),
    (
        "schemas/raw_models.json",
        include_bytes!("../../../examples/starts/epoch-ai-models/schemas/raw_models.json"),
    ),
    (
        "panels/dashboard.yaml",
        include_bytes!("../../../examples/starts/epoch-ai-models/panels/dashboard.yaml"),
    ),
    (
        "datapackage.json",
        include_bytes!("../../../examples/starts/epoch-ai-models/datapackage.json"),
    ),
];

/// The files of the [`PALMER_PENGUINS`] Protocol folder under `examples/starts/`, as its
/// start writes them.
const PALMER_PENGUINS_FILES: &[(&str, &[u8])] = &[
    (
        "arcform.yaml",
        include_bytes!("../../../examples/starts/palmer-penguins/arcform.yaml"),
    ),
    (
        "models/01_raw_penguins.sql",
        include_bytes!("../../../examples/starts/palmer-penguins/models/01_raw_penguins.sql"),
    ),
    (
        "models/02_penguins.sql",
        include_bytes!("../../../examples/starts/palmer-penguins/models/02_penguins.sql"),
    ),
    (
        "schemas/raw_penguins.json",
        include_bytes!("../../../examples/starts/palmer-penguins/schemas/raw_penguins.json"),
    ),
    (
        "panels/dashboard.yaml",
        include_bytes!("../../../examples/starts/palmer-penguins/panels/dashboard.yaml"),
    ),
    (
        "datapackage.json",
        include_bytes!("../../../examples/starts/palmer-penguins/datapackage.json"),
    ),
    (
        "data/penguins.csv",
        include_bytes!("../../../examples/starts/palmer-penguins/data/penguins.csv"),
    ),
];

/// The files of the [`USGS_EARTHQUAKES`] Protocol folder under `examples/starts/`, as its
/// start writes them.
const USGS_EARTHQUAKES_FILES: &[(&str, &[u8])] = &[
    (
        "arcform.yaml",
        include_bytes!("../../../examples/starts/usgs-earthquakes-2025/arcform.yaml"),
    ),
    (
        "models/01_raw_earthquakes.sql",
        include_bytes!(
            "../../../examples/starts/usgs-earthquakes-2025/models/01_raw_earthquakes.sql"
        ),
    ),
    (
        "models/02_earthquakes.sql",
        include_bytes!("../../../examples/starts/usgs-earthquakes-2025/models/02_earthquakes.sql"),
    ),
    (
        "schemas/raw_earthquakes.json",
        include_bytes!(
            "../../../examples/starts/usgs-earthquakes-2025/schemas/raw_earthquakes.json"
        ),
    ),
    (
        "panels/dashboard.yaml",
        include_bytes!("../../../examples/starts/usgs-earthquakes-2025/panels/dashboard.yaml"),
    ),
    (
        "datapackage.json",
        include_bytes!("../../../examples/starts/usgs-earthquakes-2025/datapackage.json"),
    ),
    (
        "data/earthquakes_2025.parquet",
        include_bytes!(
            "../../../examples/starts/usgs-earthquakes-2025/data/earthquakes_2025.parquet"
        ),
    ),
];

const DASHBOARD_SPEC: &str = include_str!("../assets/starts/signals-dashboard.yaml");
const DISTRIBUTION_SPEC: &str = include_str!("../assets/starts/reading-distribution.yaml");
const BREAKDOWN_SPEC: &str = include_str!("../assets/starts/activity-breakdown.yaml");
const CROSSWALK_MANIFEST: &str =
    include_str!("../../../examples/protocol/edgar_gleif/arcform.yaml");

/// The emitted Protocol+Run contract [`CROSSWALK_RUN`] opens.
///
/// **The artefact itself, not a copy of it.** `include_bytes!` reaches across
/// into `brightfield-protocol`'s own fixture directory rather than duplicating
/// the JSON under `assets/starts/`, on the same argument
/// [`CROSSWALK_CHART_SPEC`] makes for reaching into `examples/`: one file, so
/// what a reader inspects and what the button opens cannot drift.
/// `the_run_start_draws_the_contract_it_ships` compares the graph this composes
/// against the steps in these very bytes.
///
/// Bytes rather than a string because that is what
/// [`brightfield_protocol::view_from_contract_bytes`] takes — the contract is
/// JSON on a wire, not source anybody edits.
///
/// Public so a test can compare these bytes with the file on disk and say
/// *one artefact* rather than *two that agree today*.
pub const CROSSWALK_RUN_CONTRACT: &[u8] =
    include_bytes!("../../brightfield-protocol/fixtures/edgar_gleif.contract.json");

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
/// [`CROSSWALK_CHART`] and [`CALIFORNIA_HOUSING`] are the exceptions: the
/// first's `file:` source is an `https://` URL, so composing it fetches, and
/// the second fetches its data file the first time it is opened on a machine.
/// That is what [`Start::remote`] declares and what the label discloses.
///
/// **This entry point waits for that fetch**, and the window does not call it
/// for a start that has one. `open_start` in
/// [`MeridianApp`](crate::window::MeridianApp) starts the fetch on a worker and
/// composes on a later frame, so the click leaves the window drawing; what is left here is for callers with no frames to keep —
/// the network-gated tests, the thumbnail regeneration, a launch restoring a
/// recorded start. Both routes end at [`compose`], so a difference between what
/// a test opens and what a click opens would have to be a difference in the
/// bytes fetched.
///
/// # Errors
///
/// If `id` is not a start this build ships, or if the embedded fixture fails
/// to compose. For a start that is not [`Start::remote`] the second is a
/// build-time defect rather than a user's circumstance —
/// `every_shipped_start_loads_into_a_document_with_something_in_it` and the
/// thumbnail regeneration test in `tests/front_door.rs` are what keep it from
/// reaching a user. For a remote start it is *also* the ordinary offline
/// answer: an error naming the network and the URL, which the caller raises
/// as a banner rather than swallowing.
pub fn load(id: &str) -> Result<Opened, String> {
    let Some(start) = find(id) else {
        return Err(format!("no shipped starting point named {id:?}"));
    };
    // The data-file arm, and it is first because it is the one arm that puts
    // something on disk: a start declaring both a `data:` and a `spec:` would
    // otherwise fetch nothing and compose the spec, which is a silent wrong
    // answer rather than a loud one. The two are exclusive and
    // `a_start_declares_a_spec_or_a_data_file_and_never_both` is what holds it.
    //
    // A file an earlier open wrote is opened as it is, with no fetch: this is
    // also the path a launch restoring the start takes, and a restore should
    // not need the network the first open did.
    if let Some(data) = &start.data {
        let path = dataset_path(data);
        if path.is_file() {
            return Ok(Opened::File(path));
        }
        let fetched = crate::remote::Fetch::begin(vec![data.url.to_string()], || {}).wait()?;
        return write_fetched(data, &fetched).map(Opened::File);
    }
    // The Protocol arm writes the folder and hands back where it is. Running
    // it is the caller's: the window runs it on a worker and keeps drawing,
    // and `Boot` runs it to completion before it composes.
    if let Some(protocol) = &start.protocol {
        return write_protocol(protocol).map(Opened::Panels);
    }
    // The chart arm reads the start's own [`Start::spec`] rather than a second
    // table keyed by id, so the bytes a check reads off the entry are the bytes
    // the click composes. The manifest arm stays keyed by id: a chart start
    // added with no `spec:` must fail loudly here, not silently open the
    // crosswalk's lineage graph.
    if let Some(spec) = start.spec {
        // The sources this spec reads over the network, moved to local files
        // BEFORE the engine sees the spec — which is what makes the composition
        // below a local one whether the start is remote or not. This waits,
        // because `load` is the blocking entry: the window does not call it for
        // a remote start (see `MeridianApp::open_start`), and the callers that
        // do — the network-gated tests, the thumbnail regeneration, a launch
        // restoring a recorded start — have no frame to keep drawing.
        let sources = crate::remote::remote_sources(spec)?;
        if sources.is_empty() {
            return compose(spec, None);
        }
        let fetched = crate::remote::Fetch::begin(sources, || {}).wait()?;
        return compose(spec, Some(fetched));
    }
    match id {
        CROSSWALK => load_protocol_str(CROSSWALK_MANIFEST, CROSSWALK_MODELS)
            .map(|inputs| Opened::Protocol(Box::new(inputs))),
        CROSSWALK_RUN => load_contract_str(CROSSWALK_RUN_CONTRACT)
            .map(|inputs| Opened::Protocol(Box::new(inputs))),
        other => Err(format!(
            "the shipped starting point {other:?} declares neither a chart spec \
             nor a manifest loader"
        )),
    }
}

/// Load the start with this id over sources already on disk, rather than
/// fetching them — [`load`]'s chart and data arms with the fetch handed in.
///
/// For the per-push thumbnail tests, which render [`CROSSWALK_CHART`] from a
/// committed reduced copy of the published table (see
/// `scripts/cut-crosswalk-fixture.sh` for what the copy keeps and why) and
/// [`CALIFORNIA_HOUSING`] from a committed copy of its authors' archive. The
/// composition is [`compose`], and the data file is written by
/// [`write_fetched`], the same functions a click and [`load`] end at, so what
/// differs from a live open is which bytes were read.
///
/// A data start writes here even when an earlier open left a file at
/// [`dataset_path`], and replaces it: what was handed in is what the caller
/// asked to open.
///
/// # Errors
///
/// If `id` is not a chart or data start this build ships, its spec does not
/// parse, or `fetched` does not stand in for every source it reads over the
/// network — composing with one left over would reach the network after all.
pub fn load_with(id: &str, fetched: crate::remote::Fetched) -> Result<Opened, String> {
    let Some(start) = find(id) else {
        return Err(format!("no shipped starting point named {id:?}"));
    };
    if let Some(data) = &start.data {
        return write_fetched(data, &fetched).map(Opened::File);
    }
    let Some(spec) = start.spec else {
        return Err(format!("the shipped starting point {id:?} is not a chart"));
    };
    let missing: Vec<String> = crate::remote::remote_sources(spec)?
        .into_iter()
        .filter(|url| fetched.local_path(url).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "{id:?} reads {} over the network, and nothing stands in for it",
            missing.join(", ")
        ));
    }
    compose(spec, Some(fetched))
}

/// One chart start, loaded live and composed once — the shape every entry in
/// [`load`]'s chart arms takes.
///
/// The sampling policy is applied by [`LiveDashboard`]'s own constructor, so a
/// start is decided the same way a file opened from the command line is.
///
/// `fetched` repoints the spec's `https://` sources at the local files a
/// [`crate::remote::Fetch`] wrote, and is `None` for a start with no such
/// source. **Both callers land here** — [`load`], which waits for the fetch,
/// and [`crate::window::MeridianApp`], which polls one across frames — so what
/// the network-gated test opens and what a click opens are composed by one
/// function rather than by two that agree today.
pub fn compose(spec: &str, fetched: Option<crate::remote::Fetched>) -> Result<Opened, String> {
    let mut live = match &fetched {
        Some(files) => {
            let parsed = crate::remote::repointed(spec, files)?;
            LiveDashboard::load_parsed(parsed, None, None)?
        }
        None => LiveDashboard::load_str(spec, None)?,
    };
    let composed = live.present()?;
    Ok(Opened::Charts(Box::new(OpenedChart {
        live,
        composed,
        fetched,
    })))
}

/// Where a bundled data file is written on this machine.
///
/// [`crate::startup::datasets_dir`] when this machine resolves a config
/// directory, and a process-scoped temporary directory when it does not. The
/// fallback is not a second policy: a machine with no home and no
/// `XDG_CONFIG_HOME` has nowhere to keep a layout file either — see
/// `brightfield_workbench::persist::config_dir`, which answers `None` for
/// exactly that machine — so there is no remembered start for a stable path to
/// reopen, and the path would be a promise the platform cannot keep. Naming it after the process keeps two brightfields on such a
/// machine from writing the same path.
#[must_use]
pub fn datasets_dir() -> std::path::PathBuf {
    crate::startup::datasets_dir().unwrap_or_else(|| {
        std::env::temp_dir().join(format!("brightfield-datasets-{}", std::process::id()))
    })
}

/// The subdirectory of [`datasets_dir`] a fetched data file is written into.
///
/// Its own directory, not [`datasets_dir`] itself, because builds before this
/// one wrote a different copy of California Housing to
/// `datasets_dir()/california_housing.parquet`. [`load`] opens a file it finds
/// at [`dataset_path`] without fetching, so sharing that path would have
/// reopened the old copy on a machine that had one.
pub const FETCHED_DIR: &str = "fetched";

/// Where `data`'s file is written on this machine, and where [`load`] looks
/// for it before fetching: `fetched/v{revision}/{file_name}` under
/// [`datasets_dir`].
///
/// **A stable path, not a temporary one**: the locator band names a real
/// file, Save writes the Protocol beside it, and the next launch — which
/// restores this start through
/// [`SavedLayout::opened`](brightfield_workbench::persist::SavedLayout::opened)
/// — finds the file already there and opens it with no network.
#[must_use]
pub fn dataset_path(data: &FetchedData) -> std::path::PathBuf {
    datasets_dir()
        .join(FETCHED_DIR)
        .join(format!("v{}", data.revision))
        .join(data.file_name)
}

/// Make `data`'s file at [`dataset_path`] from what `fetched` brought back
/// from [`FetchedData::url`], and hand back where it is.
///
/// Called with the fetch already done: by [`load`] after waiting for it, by
/// the window when its worker finishes, and by [`load_with`] with a stand-in.
///
/// **The write lands by rename.** [`FetchedData::prepare`] writes to a staging
/// name in the same directory (so the rename is within one filesystem and is
/// atomic), which is renamed onto the path. A reader that opened the window
/// twice in quick succession therefore sees either the previous file or the
/// whole new one, rather than a Parquet with no footer — which DuckDB reports
/// as a corrupt file rather than as a file that is not finished being
/// written. A failed prepare leaves no file for [`load`] to find and trust,
/// which `an_archive_that_is_not_the_authors_file_is_refused_and_leaves_nothing`
/// in `tests/remote_start.rs` holds.
///
/// # Errors
///
/// If `fetched` holds nothing for the URL, the directory cannot be created,
/// the prepare fails, or the rename fails — each carrying the path or the URL
/// and the underlying words. The caller raises it as the banner a start that
/// will not open gets.
pub fn write_fetched(
    data: &FetchedData,
    fetched: &crate::remote::Fetched,
) -> Result<std::path::PathBuf, String> {
    let Some(source) = fetched.local_path(data.url) else {
        return Err(format!("nothing was fetched from {}", data.url));
    };
    let path = dataset_path(data);
    let dir = path
        .parent()
        .expect("dataset_path is a file inside a revision directory");
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Named per write, not per process: two writes in one process — two
    // tests on two threads, or a click while a restore is writing — would
    // otherwise share a staging file, and the second rename would find none.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let staging = dir.join(format!(
        "{}.{}-{}.part",
        data.file_name,
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    // A staging file nothing will ever finish is litter in a directory the
    // reader can see, so every failure from here removes it. Removing it
    // cannot fail the call it is reporting.
    let _ = std::fs::remove_file(&staging);
    if let Err(e) = (data.prepare)(source, &staging) {
        let _ = std::fs::remove_file(&staging);
        return Err(format!("the file fetched from {}: {e}", data.url));
    }
    std::fs::rename(&staging, &path).map_err(|e| {
        let _ = std::fs::remove_file(&staging);
        format!("{} -> {}: {e}", staging.display(), path.display())
    })?;
    Ok(path)
}

/// The subdirectory of [`datasets_dir`] a shipped Protocol folder is written
/// into, beside [`FETCHED_DIR`].
pub const PROTOCOLS_DIR: &str = "protocols";

/// Where `protocol`'s folder is written on this machine:
/// `protocols/v{revision}/{dir_name}` under [`datasets_dir`].
///
/// A stable path, so the next launch finds the folder and the run it made,
/// and reopens it with no run and no network.
#[must_use]
pub fn protocol_path(protocol: &ShippedProtocol) -> std::path::PathBuf {
    datasets_dir()
        .join(PROTOCOLS_DIR)
        .join(format!("v{}", protocol.revision))
        .join(protocol.dir_name)
}

/// Write `protocol`'s folder at [`protocol_path`], and hand back where it is.
///
/// A folder already there for the same revision is left alone: it may hold a
/// run, and it is this build's folder. Otherwise every file is written into a
/// staging folder beside it, which is renamed into place, so a folder is
/// either whole or absent, and a write that fails part way leaves nothing for
/// the next open to find and trust.
///
/// # Errors
///
/// A file path that leaves the folder, or a directory, file or rename that
/// fails, each naming the path.
pub fn write_protocol(protocol: &ShippedProtocol) -> Result<std::path::PathBuf, String> {
    let path = protocol_path(protocol);
    if path.join(arc::spec::MANIFEST_FILENAME).is_file() {
        return Ok(path);
    }
    let parent = path
        .parent()
        .expect("protocol_path is a folder inside a revision directory");
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    sweep_stale_staging(parent, protocol.dir_name);
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let staging = parent.join(format!(
        "{}.{}-{}.part",
        protocol.dir_name,
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&staging);
    // Made once, here, and never again: if a sweep removes it mid-write, the
    // writes below fail rather than making it again around part of a folder.
    std::fs::create_dir(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    let written = write_staged(&staging, protocol.files, protocol.dir_name);
    if let Err(e) = written {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    // A folder there with no manifest is not this revision's folder: a write
    // from before the staging rename, or one a person emptied. It is
    // replaced, or every later open would fail renaming onto it.
    if path.exists() && !path.join(arc::spec::MANIFEST_FILENAME).is_file() {
        let _ = std::fs::remove_dir_all(&path);
    }
    if let Err(e) = std::fs::rename(&staging, &path) {
        let _ = std::fs::remove_dir_all(&staging);
        // Another writer got there first with the same revision's folder.
        if path.join(arc::spec::MANIFEST_FILENAME).is_file() {
            return Ok(path);
        }
        return Err(format!("{} -> {}: {e}", staging.display(), path.display()));
    }
    Ok(path)
}

/// Write `files` into the existing staging folder `staging`, the manifest
/// last, and check each landed whole.
///
/// No directory is made with `create_dir_all`: each one under `staging` is
/// made a level at a time, so a staging folder a sweep removed makes the
/// writes after it fail instead of being made again
/// (`a_swept_staging_folder_fails_its_writer`). The manifest goes last, so
/// a folder missing any of its files has no manifest and is written again by
/// the next open, and a read-back of each file's length catches a file a
/// sweep removed after it was written.
fn write_staged(
    staging: &std::path::Path,
    files: &[(&str, &[u8])],
    dir_name: &str,
) -> Result<(), String> {
    let mut ordered: Vec<&(&str, &[u8])> = files.iter().collect();
    ordered.sort_by_key(|(file, _)| *file == arc::spec::MANIFEST_FILENAME);
    for (file, bytes) in &ordered {
        let relative = std::path::Path::new(file);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(format!(
                "{dir_name}: the file {file:?} is not inside the folder"
            ));
        }
        if let Some(dir) = relative.parent() {
            let mut made = staging.to_path_buf();
            for part in dir.components() {
                made.push(part);
                match std::fs::create_dir(&made) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(format!("{}: {e}", made.display())),
                }
            }
        }
        let target = staging.join(relative);
        std::fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    for (file, bytes) in &ordered {
        let target = staging.join(file);
        let len = std::fs::metadata(&target).map_or(None, |m| Some(m.len()));
        if len != Some(bytes.len() as u64) {
            return Err(format!(
                "{}: the staged file did not land whole",
                target.display()
            ));
        }
    }
    Ok(())
}

/// How old another process's staging folder has to be before a write sweeps
/// it: long past any write of a shipped folder, so a folder another process
/// is writing now is never taken for one it left.
const STALE_STAGING: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Whether `name` is the shape [`write_protocol`] stages `dir_name` under:
/// `{dir_name}.{pid}-{n}.part`, with a digit run either side of the dash.
fn is_staging_of(name: &str, dir_name: &str) -> bool {
    let Some(middle) = name
        .strip_prefix(dir_name)
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_suffix(".part"))
    else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    middle
        .split_once('-')
        .is_some_and(|(pid, n)| digits(pid) && digits(n))
}

/// Remove the staging folders for `dir_name` in `parent` that another
/// process left: a write stopped before its rename, older than
/// [`STALE_STAGING`]. This process's own are left alone, since another of its
/// threads may be writing one.
fn sweep_stale_staging(parent: &std::path::Path, dir_name: &str) {
    let ours = format!("{dir_name}.{}-", std::process::id());
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_staging_of(&name, dir_name) || name.starts_with(&ours) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= STALE_STAGING);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The URLs the `http_fetch` steps of the Protocol in `manifest` fetch, in
/// step order: what a run of it reads over the network.
#[must_use]
pub fn fetch_urls(manifest: &str) -> Vec<String> {
    let Ok(parsed) = serde_yaml::from_str::<serde_yaml::Value>(manifest) else {
        return Vec::new();
    };
    parsed
        .get("steps")
        .and_then(serde_yaml::Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|step| {
            step.get("op")
                .and_then(serde_yaml::Value::as_str)
                .is_some_and(is_fetch_op)
        })
        .filter_map(|step| step.get("with")?.get("url")?.as_str().map(str::to_string))
        .collect()
}

/// Whether a step's `op:` names arc's `http_fetch`, with or without a
/// version (`http_fetch@1`).
#[must_use]
pub fn is_fetch_op(op: &str) -> bool {
    op.strip_prefix("http_fetch").is_some_and(|rest| {
        rest.is_empty()
            || rest
                .strip_prefix('@')
                .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// The database a run of the Protocol in `dir` writes, and its write-ahead
/// log beside it.
fn protocol_db_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(db) = arc::spec::protocol_db_path(dir) else {
        return Vec::new();
    };
    let mut wal = db.clone().into_os_string();
    wal.push(".wal");
    vec![db, std::path::PathBuf::from(wal)]
}

/// Whether the shipped Protocol in `dir` needs a run before its panels can
/// open: it has no database, or its newest run record does not say the run
/// succeeded.
///
/// A database whose newest record failed, or that has no record behind it
/// (a run stopped part way), holds tables nobody finished, and it is removed
/// here, so the run that follows starts from nothing. The window does not
/// ask this while a run of the folder is under way, since that run's
/// database has no record yet.
#[must_use]
pub fn needs_run(dir: &std::path::Path) -> bool {
    let files = protocol_db_files(dir);
    let Some(db) = files.first() else {
        return true;
    };
    let succeeded = arc::spec::Manifest::load(dir).is_ok_and(|manifest| {
        crate::run::records_newest_first(dir, &manifest.name)
            .next()
            .is_some_and(|view| {
                matches!(
                    view.run.outcome,
                    brightfield_protocol::contract::Outcome::Success
                )
            })
    });
    if db.is_file() && succeeded {
        return false;
    }
    for file in &files {
        let _ = std::fs::remove_file(file);
    }
    true
}

/// What a finished run of the shipped Protocol in `dir` means for its start:
/// `Ok` when it ran to a record, and otherwise the words its banner carries.
///
/// A failed run's database is removed first, so the next open runs the
/// Protocol again rather than trusting tables a failed run left. A run that
/// could not fetch one of the Protocol's `http_fetch` URLs says it could not
/// reach that URL over the network; any other failure says the run did not
/// finish. Either way the run's own log follows, so a missing DuckDB CLI is
/// named in arc's own words.
///
/// # Errors
///
/// The banner's words, when the run did not succeed.
pub fn settle_run(dir: &std::path::Path, finished: &crate::run::Finished) -> Result<(), String> {
    if finished.succeeded && finished.record.is_some() {
        return Ok(());
    }
    for file in protocol_db_files(dir) {
        let _ = std::fs::remove_file(file);
    }
    let manifest =
        std::fs::read_to_string(dir.join(arc::spec::MANIFEST_FILENAME)).unwrap_or_default();
    let urls = fetch_urls(&manifest);
    let log = finished.log.trim();
    let mut named: Vec<&String> = urls
        .iter()
        .filter(|url| log.contains(url.as_str()))
        .collect();
    if named.is_empty() && log.contains("http_fetch") {
        named = urls.iter().collect();
    }
    if named.is_empty() {
        Err(format!("the Protocol's run did not finish:\n{log}"))
    } else {
        let urls: Vec<&str> = named.iter().map(|u| u.as_str()).collect();
        Err(format!(
            "could not reach {} over the network, so the Protocol did not run:\n{log}",
            urls.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer whose staging folder is swept out from under it fails,
    /// rather than making the folder again and renaming a part of it into
    /// place.
    #[test]
    fn a_swept_staging_folder_fails_its_writer() {
        let root = std::env::temp_dir().join(format!("bf-staging-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let staging = root.join("shipped.1-0.part");
        std::fs::create_dir(&staging).unwrap();
        std::fs::remove_dir_all(&staging).unwrap();
        let files: &[(&str, &[u8])] =
            &[("models/a.sql", b"select 1"), ("arcform.yaml", b"name: x")];
        assert!(write_staged(&staging, files, "shipped").is_err());
        assert!(!staging.exists(), "the swept staging folder was made again");
        std::fs::remove_dir_all(&root).ok();
    }

    /// The sweep matches only `{dir_name}.{pid}-{n}.part`.
    #[test]
    fn the_sweep_matches_only_its_own_folders_staging_names() {
        assert!(is_staging_of("foo.12-3.part", "foo"));
        assert!(!is_staging_of("foo.bar.12-3.part", "foo"));
        assert!(!is_staging_of("foo.12.part", "foo"));
        assert!(!is_staging_of("foo.12-x.part", "foo"));
        assert!(!is_staging_of("foobar.12-3.part", "foo"));
    }
}
