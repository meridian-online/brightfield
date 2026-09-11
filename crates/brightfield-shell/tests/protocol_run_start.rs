//! The front door offers a Protocol that has run, and every surface that
//! exists to report a run reports it.
//!
//! # What was wrong, and what these tests are for
//!
//! Every Protocol this build could open was a **declaration**. The only thing
//! that built a document for the protocol view derived its graph from a
//! manifest's declared steps, so the per-step statuses, the per-asset
//! measurements and the per-step detail were empty on every input the binary
//! could reach: the spine said *not run* against each step, the ledger's strip
//! said *not run*, and the quality output was empty on every screen a stranger
//! could get to. A picture that carries the work that made it is the thing this
//! product claims no other tool has, and its own catalogue could not show it.
//!
//! So the assertions below are about **states**, not about the presence of a
//! start: that no step on the run's graph reports never-run, that the spine's
//! rows carry those states where a reader can see them, and that the ledger's
//! strip carries the run's own outcome. A test that only asserted the card
//! exists would have passed over the defect this card is for.
//!
//! Nothing here executes anything. Brightfield runs no step; what it reads is
//! the artefact a run emits.

use std::path::PathBuf;

use brightfield_protocol::contract_graph::SeamStatus;
use brightfield_protocol::layout::Flow;
use brightfield_shell::design::Mode;
use brightfield_shell::protocol::{self, ProtocolModel, SpineRole};
use brightfield_shell::starts::{self, Opened};
use brightfield_shell::startup::default_layout;
use brightfield_shell::window::{Boot, MeridianApp};
use brightfield_workbench::arrangement;

/// The word the spine and the steps sheet both use for a step nothing ran.
const NOT_RUN: &str = "not run";

/// Where the emitted contract the run start ships actually lives — the
/// `brightfield-protocol` fixture the `include_bytes!` in `starts.rs` points
/// at, resolved from this crate rather than from the working directory.
fn contract_on_disk() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../brightfield-protocol/fixtures/edgar_gleif.contract.json")
}

/// The run start, loaded into the model the rails read.
fn run_model() -> ProtocolModel {
    match starts::load(starts::CROSSWALK_RUN).expect("the run start loads") {
        Opened::Protocol(inputs) => ProtocolModel::new(*inputs, Flow::Vertical),
        Opened::Charts(_) => panic!("the run start opened a chart, not a Protocol"),
    }
}

/// A window over the run start, and one `egui::Context` for its whole life —
/// the shape `front_door.rs` and `navigator_spine.rs` use.
struct Window {
    app: MeridianApp,
    ctx: egui::Context,
    screen: egui::Rect,
}

impl Window {
    fn on(id: &str) -> Self {
        let boot = Boot::start(id, Flow::Vertical).expect("the start loads");
        Self {
            app: MeridianApp::headless_with_layout(boot, default_layout(), Mode::Light),
            ctx: egui::Context::default(),
            screen: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0)),
        }
    }

    /// Draw three frames and hand back every text the last one drew, with the
    /// rect it drew into.
    ///
    /// Three, for the reason `arrangement.rs` gives: a resizable panel reports
    /// the size it settled at on the frame after, so a rail read on the first
    /// frame is read at a size nothing will keep.
    fn settle_and_read(&mut self) -> Vec<(egui::Rect, String)> {
        let raw = egui::RawInput {
            screen_rect: Some(self.screen),
            ..Default::default()
        };
        let mut out = Vec::new();
        for _ in 0..3 {
            let frame = self.ctx.run_ui(raw.clone(), |ui| self.app.draw(ui));
            out.clear();
            for clipped in &frame.shapes {
                collect_placed_text(&clipped.shape, &mut out);
            }
        }
        out
    }
}

/// Every text in `shape`, with **where it was drawn** — the galley's own box,
/// moved to the position the painter was given.
///
/// The position is the point. "The frame drew the words `last run · success`"
/// is a weaker claim than "the ledger's strip drew them", and the second is
/// what an assertion about the strip has to make: a run outcome printed
/// anywhere else on the window would satisfy the first.
///
/// `Shape::Vec` nests, so a walk that reads the top level and stops misses
/// whatever a widget put inside a group.
fn collect_placed_text(shape: &egui::epaint::Shape, into: &mut Vec<(egui::Rect, String)>) {
    match shape {
        egui::epaint::Shape::Text(t) => {
            into.push((
                t.galley.rect.translate(t.pos.to_vec2()),
                t.galley.text().to_string(),
            ));
        }
        egui::epaint::Shape::Vec(shapes) => {
            for s in shapes {
                collect_placed_text(s, into);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// AC1 — the graph carries a run
// ---------------------------------------------------------------------------

/// The door offers a crosswalk start whose click lands on a graph carrying a
/// run: no step on it reports never-run.
///
/// Read off the model rather than off the contract, because the model is what
/// every rail draws from. Every step is checked, not a sample — a start whose
/// first step carried a state and whose last did not would pass a spot check
/// and would still be the defect.
///
/// Watched redden, one mutation: `protocol::load_contract_str` building with
/// `statuses: BTreeMap::new()`, which is the manifest path's value. The
/// assertion below then reports 4 of 4 steps as never-run.
#[test]
fn the_run_start_lands_on_a_graph_where_no_step_reports_never_run() {
    assert!(
        starts::find(starts::CROSSWALK_RUN).is_some_and(|s| s.on_door),
        "the run start is not on the door, so nothing a stranger can reach \
         opens it"
    );
    let model = run_model();
    let states = model.step_states();
    assert!(
        !states.is_empty(),
        "the run start's graph carries no steps at all, so `no step reports \
         never-run` is vacuously true"
    );
    let never_run: Vec<&String> = states
        .iter()
        .filter(|(_, s)| **s == SeamStatus::NotRun)
        .map(|(name, _)| name)
        .collect();
    assert!(
        never_run.is_empty(),
        "{} of {} steps on the run start's graph report never-run: {never_run:?}",
        never_run.len(),
        states.len()
    );
    let run = model
        .run()
        .expect("a Protocol opened from a contract carries the run it came off");
    assert_eq!(
        protocol::outcome_word(run.outcome),
        "success",
        "the shipped contract's own outcome is not what the model reports"
    );
}

/// …and the run-less start beside it still reports exactly the opposite, on
/// the same reading.
///
/// The pair is what makes the assertion above mean something: `not run` is a
/// real state this build still renders, and the contract path is what is new,
/// not the vocabulary.
///
/// Watched redden, one mutation: `protocol::inputs_from` returning
/// `run: Some(...)` — the manifest then claims a run and the second assertion
/// fails.
#[test]
fn the_run_less_start_still_reports_every_step_as_never_run() {
    let model = match starts::load(starts::CROSSWALK).expect("the manifest start loads") {
        Opened::Protocol(inputs) => ProtocolModel::new(*inputs, Flow::Vertical),
        Opened::Charts(_) => panic!("the manifest start opened a chart"),
    };
    let states = model.step_states();
    assert!(
        !states.is_empty(),
        "the manifest start's graph carries no steps"
    );
    assert!(
        states.values().all(|s| *s == SeamStatus::NotRun),
        "a declaration reported a run state for at least one step: {states:?}"
    );
    assert!(
        model.run().is_none(),
        "a manifest with no run behind it claims a run"
    );
}

// ---------------------------------------------------------------------------
// AC2 — the spine and the ledger's strip
// ---------------------------------------------------------------------------

/// The spine's step rows read the states the run recorded, and the ledger's
/// strip reads the run's outcome — both off a drawn frame.
///
/// The strip's words are located, not merely found: they are asserted to be
/// inside `rail_summary_rect(LEDGER_RAIL)`, because the same string printed
/// anywhere else on the window would satisfy a search of the frame's text and
/// say nothing about the strip.
///
/// Watched redden, two mutations. `MeridianApp::ledger_summary`'s run arm
/// deleted: the strip then draws no summary, and the expect on
/// `rail_summary_rect` fails. `SpineRow`'s step arm built with
/// `status_word(SeamStatus::NotRun)`, so a step row trails `not run` whatever
/// the run recorded: the first assertion fails, reporting 4 of 4 rows.
#[test]
fn the_spine_and_the_ledger_strip_read_the_run_off_the_frame() {
    let mut win = Window::on(starts::CROSSWALK_RUN);
    let drawn = win.settle_and_read();

    let steps: Vec<&brightfield_shell::protocol::SpineRowDrawn> = win
        .app
        .spine_rows()
        .iter()
        .filter(|row| row.role == SpineRole::Step)
        .collect();
    assert!(
        !steps.is_empty(),
        "the spine drew no step rows, so this test read nothing"
    );
    let unrun: Vec<&str> = steps
        .iter()
        .filter(|row| row.kind.contains(NOT_RUN))
        .map(|row| row.label.as_str())
        .collect();
    assert!(
        unrun.is_empty(),
        "{} of {} step rows on the spine still trail {NOT_RUN:?}: {unrun:?}",
        unrun.len(),
        steps.len()
    );
    assert!(
        steps.iter().all(|row| row.kind.contains("ok")),
        "a step row carries neither {NOT_RUN:?} nor a recorded state: {:?}",
        steps.iter().map(|r| &r.kind).collect::<Vec<_>>()
    );

    let summary = win
        .app
        .rail_summary_rect(arrangement::LEDGER_RAIL)
        .expect("the ledger's strip drew a summary");
    let at_the_strip: Vec<&str> = drawn
        .iter()
        .filter(|(rect, _)| summary.expand(1.0).contains_rect(*rect))
        .map(|(_, text)| text.as_str())
        .collect();
    assert!(
        at_the_strip
            .iter()
            .any(|text| text.contains("last run") && text.contains("success")),
        "the ledger's strip at {summary:?} says {at_the_strip:?}, which does \
         not name the run this Protocol came off"
    );
    assert!(
        at_the_strip.iter().all(|text| !text.contains(NOT_RUN)),
        "the ledger's strip still says {NOT_RUN:?}: {at_the_strip:?}"
    );
}

/// The one-step arm of the same strip is untouched: a Protocol with no run
/// behind it still gets the step summary it had.
///
/// This is what the run arm had to be added *in front of* rather than instead
/// of — a data file opens as a Protocol of one step with no run, and its strip
/// is the whole of the list it stands for.
///
/// Watched redden, one mutation: `ledger_summary`'s run arm returning
/// unconditionally (`Some(format!("last run · {}", …))` over an `unwrap_or`
/// default) — the one-step Protocol then draws a run line and this fails.
#[test]
fn a_protocol_with_no_run_still_summarises_its_one_step() {
    let inputs = protocol::load_protocol_str(
        "name: one\nengine: duckdb\nsteps:\n  - name: read\n    op: http_fetch@1\n    \
         with:\n      url: https://example.invalid/a.csv\n      out: build/a.csv\n",
        &[],
    )
    .expect("a one-step manifest loads");
    let model = ProtocolModel::new(inputs, Flow::Vertical);
    assert_eq!(model.sheet().len(), 1, "this fixture is not one step");
    assert!(model.run().is_none(), "a manifest carries no run");
    let row = &model.sheet().rows()[0];
    assert_eq!(
        row.status, NOT_RUN,
        "a step with no run behind it reports something other than {NOT_RUN:?}"
    );
}

// ---------------------------------------------------------------------------
// AC3 — one artefact, not a copy
// ---------------------------------------------------------------------------

/// The contract the start ships is the file in `brightfield-protocol`'s
/// fixtures, byte for byte, and what the start draws is what that file says.
///
/// Three assertions, and each holds something the others do not. The byte
/// comparison is what makes *one artefact* a fact rather than a hope: a copy
/// under `assets/starts/` would pass every other test here for exactly as long
/// as the two agreed. The **seam** comparison holds what the spine and the
/// canvas tint from. The **sheet** comparison holds what the ledger's Steps
/// pane lists, which is a second reading of the same contract built by a
/// different call — and until it was here, swapping that call for the manifest
/// path's seam synthesis left the six tests in this file green while the Steps
/// pane read `not run` down its status column.
///
/// Watched redden, three mutations. `starts::CROSSWALK_RUN_CONTRACT` pointed at
/// a copy with one step's state edited: the byte assertion fails.
/// `load_contract_str` building with `statuses: BTreeMap::new()`: the seam
/// comparison fails on the first step. `load_contract_str` building its sheet
/// with `synth_sheet_rows` (the manifest path's) instead of
/// `StepsSheet::from_view`, which puts `not run` in the status column: the
/// sheet comparison fails at `fetch_cik_lookup`.
#[test]
fn the_run_start_draws_the_contract_it_ships() {
    let on_disk = std::fs::read(contract_on_disk()).expect("the fixture is where starts.rs says");
    assert_eq!(
        starts::CROSSWALK_RUN_CONTRACT,
        on_disk.as_slice(),
        "the bytes the start ships are not the bytes of the fixture it names — \
         there are two artefacts, and this is the moment they disagreed"
    );

    let contract: serde_json::Value =
        serde_json::from_slice(&on_disk).expect("the fixture is JSON");
    let declared: Vec<(String, String)> = contract["steps"]
        .as_array()
        .expect("the contract lists steps")
        .iter()
        .map(|step| {
            (
                step["name"].as_str().expect("a step name").to_string(),
                step["status"]["state"]
                    .as_str()
                    .expect("a step state")
                    .to_string(),
            )
        })
        .collect();
    assert!(
        declared.len() >= 4,
        "the fixture is down to {} step(s); this comparison has almost nothing \
         left to hold",
        declared.len()
    );

    let model = run_model();
    let drawn = model.step_states();
    assert_eq!(
        drawn.len(),
        declared.len(),
        "the document drew {} step(s) for a contract of {}",
        drawn.len(),
        declared.len()
    );
    for (name, state) in &declared {
        let got = drawn
            .get(name)
            .unwrap_or_else(|| panic!("the contract declares a step {name:?} the document lost"));
        let want = match state.as_str() {
            "success" => SeamStatus::Ok,
            "failed" => SeamStatus::Failed,
            "skipped" => SeamStatus::Skipped,
            other => panic!("the fixture carries a state {other:?} this test does not map"),
        };
        assert_eq!(
            *got, want,
            "the contract records {name} as {state:?} and the document draws \
             it as {got:?}"
        );
    }

    // …and the ledger's Steps pane, which is a second reading of the same
    // contract built by a different call. The status column here is the one a
    // reader sees listed under the strip.
    let sheet = model.sheet();
    assert_eq!(
        sheet.rows().len(),
        declared.len(),
        "the steps sheet lists {} row(s) for a contract of {} step(s)",
        sheet.rows().len(),
        declared.len()
    );
    for (name, state) in &declared {
        let row = sheet
            .rows()
            .iter()
            .find(|r| r.name == *name)
            .unwrap_or_else(|| panic!("the steps sheet lost the step {name:?}"));
        let want = match state.as_str() {
            "success" => "ok",
            "failed" => "failed",
            "skipped" => "skipped",
            other => panic!("the fixture carries a state {other:?} this test does not map"),
        };
        assert_eq!(
            row.status, want,
            "the contract records {name} as {state:?} and the steps sheet \
             lists it as {:?}",
            row.status
        );
    }
}

// ---------------------------------------------------------------------------
// AC4 — the run-less start is unchanged
// ---------------------------------------------------------------------------

/// The set now holds a Protocol start of **each** kind, which is what makes the
/// run-less disclosure gate a two-sided check rather than a tautology.
///
/// `a_start_that_opens_a_run_less_manifest_says_so_on_its_own_button` in
/// `front_door.rs` asserts the flag and the mark agree in both directions over
/// every shipped start. While every Protocol start was run-less, the second
/// direction — a label carrying the mark without the flag — had nothing in the
/// set that could ever exercise it. It has now.
///
/// Watched redden, one mutation: `CROSSWALK_RUN`'s label given the
/// `(no run)` mark — this test's last assertion fails, and so does the
/// front-door gate it is about.
#[test]
fn the_door_offers_a_protocol_of_each_kind_and_only_the_run_less_one_says_so() {
    let protocols: Vec<&starts::Start> = starts::STARTS
        .iter()
        .filter(|s| s.on_door && s.spec.is_none())
        .collect();
    assert!(
        protocols.iter().any(|s| s.run_less),
        "no Protocol start on the door is run-less, so the `(no run)` \
         disclosure has nothing to disclose"
    );
    assert!(
        protocols.iter().any(|s| !s.run_less),
        "every Protocol start on the door is run-less, so the disclosure gate \
         is checking one side of a question with one answer in it"
    );
    for start in &protocols {
        assert_eq!(
            start.run_less,
            start.label.contains(starts::RUN_LESS_MARK),
            "{}'s label {:?} and its run_less flag ({}) disagree",
            start.id,
            start.label,
            start.run_less
        );
    }
}
