//! The design-gallery conformance gate.
//!
//! Three layers, each holding a different claim about `src/gallery.rs`:
//!
//! - **Completeness by source grep** — every `impl Component for` in the
//!   gallery source is reachable through `catalog()`, counted textually
//!   rather than collected at link time. Link-time inventory can be stripped
//!   in exactly the builds nobody is looking at; a grep cannot.
//! - **The registry seam** — the gallery-inclusive chart registry passes the
//!   workbench audit (empty state, prose style, registered verbs, tab
//!   toggle), the flag-off registry has no gallery pane, and the published
//!   id vocabulary covers the gallery either way, so a layout saved with the
//!   flag on still loads with it off.
//! - **The five-item per-primitive gate** — for every gated entry, rendered
//!   solo through the same composition `--gallery` captures:
//!   1. `get_by_role_and_label` resolves — unlabelled is untestable and
//!      screen-reader-invisible at once;
//!   2. keyboard actuation with **no pointer event** flips the evidence
//!      label from absent to present (checked absent first, so the
//!      assertion cannot pass vacuously);
//!   3. `Node::rect()` height equals the *named* ladder value the entry
//!      declares, or the entry carries written prose for why its height is
//!      intrinsic;
//!   4. token cleanliness is the sibling `token_discipline` test's grep,
//!      which scans every file under `src/` — `gallery.rs` included, with
//!      no opt-in (asserted here so removing the file from that scan would
//!      redden this suite too);
//!   5. two goldens, light and dark, at the `kittest.toml` floor
//!      (`tests/snapshots/gallery_*.png`; regenerate with
//!      `UPDATE_SNAPSHOTS=1 cargo +1.95.0 test -p brightfield-shell --test
//!      gallery_gate`).
//!
//! The pixel and accessibility layers need a wgpu adapter, like the sibling
//! snapshot tier — see `snapshot.rs` for why there is deliberately no skip
//! switch.

use brightfield_shell::app::{chart_registry_with, ChartDoc};
use brightfield_shell::capture::capture_component;
use brightfield_shell::design::Mode;
use brightfield_shell::gallery::{
    catalog, enabled, region_catalog, region_row, regions_ui, solo, ActuationInput, Component,
    ComponentStatus, FocusTarget, GateHeight, ProbeRole, RegionEntry, GALLERY,
};
use brightfield_workbench::arrangement::{
    self, Collapse, Edge, Extent, Occupant, Region, RegionFrame, RegionId,
};
use brightfield_workbench::audit;
use brightfield_workbench::registry::Slot;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use egui_kittest::{Harness, SnapshotOptions, SnapshotResults};

// ---------------------------------------------------------------------------
// Completeness by source grep
// ---------------------------------------------------------------------------

/// The gallery source, read at compile time so the grep can never scan a
/// stale copy.
const GALLERY_SRC: &str = include_str!("../src/gallery.rs");

/// Every `impl Component for` in the source is in the catalog: the counts
/// match, so a demo written but not registered is a red test, not a silent
/// absence. Ids are unique, kebab-case, and every intrinsic-height entry
/// carries real prose.
#[test]
fn the_catalog_is_complete_against_the_source() {
    let impls = GALLERY_SRC.matches("impl Component for ").count();
    let entries = catalog();
    assert_eq!(
        impls,
        entries.len(),
        "gallery.rs has {impls} `impl Component for` but catalog() returns \
         {} — register every component explicitly",
        entries.len()
    );

    let mut ids: Vec<&str> = entries.iter().map(|c| c.info().id).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "duplicate component id in the catalog");

    for component in &entries {
        let info = component.info();
        assert!(
            info.id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "{}: component ids are kebab-case",
            info.id
        );
        assert!(!info.name.is_empty(), "{}: empty display name", info.id);
        assert!(
            !info.probe.label.is_empty(),
            "{}: an unlabelled probe cannot resolve",
            info.id
        );
        if let GateHeight::Intrinsic(reason) = info.height {
            assert!(
                !reason.trim().is_empty(),
                "{}: an intrinsic height must say why",
                info.id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The regions — the second enumeration
// ---------------------------------------------------------------------------

/// A band under construction: declared, drawn, and with no frame named for it
/// yet. A shipped arrangement cannot carry one — `audit_arrangement` refuses
/// it — so the two cases below build their own; there is otherwise no
/// unstyled region to ask the gallery about.
static UNSTYLED: Region = Region {
    id: RegionId::new("nobody-styled-this"),
    edge: Edge::Top,
    extent: Extent::Band(arrangement::TITLE_BAND_HEIGHT),
    occupant: Occupant::Chrome("a band under construction"),
    collapse: Collapse::Fixed,
    frame: RegionFrame::Unstyled,
};

/// The listing is the arrangement, not a second copy of it: one entry per
/// declared region, in the order the window lays them out.
///
/// A hand-written list here would be exactly the drift the arrangement module
/// exists to end — a region added there and forgotten here is a region the
/// one surface meant to show everything does not show.
#[test]
fn the_region_listing_is_the_arrangement() {
    let declared = arrangement::default_arrangement().regions;
    let listed = region_catalog();
    assert_eq!(
        listed.len(),
        declared.len(),
        "the arrangement declares {} regions and the gallery lists {} — the \
         listing is derived from the arrangement, so a difference means it \
         stopped being",
        declared.len(),
        listed.len()
    );
    for (entry, region) in listed.iter().zip(declared) {
        assert_eq!(
            entry.region.id, region.id,
            "the listing is out of step with the arrangement it is read from"
        );
        assert!(
            entry.contract().contains(region.frame.function()),
            "{}: its row says {:?}, which does not name what draws its box",
            region.id,
            entry.contract()
        );
    }
    assert!(
        !listed.is_empty(),
        "an empty listing satisfies every assertion above"
    );
}

/// A region with no frame is listed under the draft pill rather than being
/// absent — the same honesty device [`ComponentStatus::Draft`] is for a
/// primitive the per-primitive gate cannot hold yet.
#[test]
fn a_region_with_no_frame_is_listed_as_draft() {
    assert_eq!(
        RegionEntry::of(&UNSTYLED).status,
        ComponentStatus::Draft,
        "a region with no frame declared is unfinished, and unfinished is \
         visible rather than silent"
    );
    assert!(
        !RegionEntry::of(&UNSTYLED).status.gated(),
        "a draft region is not held to a contract it has not got yet"
    );
    for entry in region_catalog() {
        assert_eq!(
            entry.status,
            ComponentStatus::Live,
            "{}: the shipped arrangement lists a region as {:?}; every region \
             it declares is on the shipping surface and has a frame",
            entry.region.id,
            entry.status
        );
    }
}

/// Every region reaches the screen, and so does a region with no style.
///
/// Drawn rather than counted, which is the difference between this and the
/// two above: the listing could be complete and the pane could still render
/// none of it. Each row is resolved by the name it draws and by the line
/// saying how it sizes, so a row that drew a name and nothing else fails too.
#[test]
fn every_region_is_drawn_in_the_gallery_with_what_frames_it() {
    let mut harness = Harness::builder()
        // Tall enough that no row is laid out past the bottom of the frame:
        // a row that never got a box has no node to resolve, which would read
        // here as the gallery not drawing it.
        .with_size(egui::vec2(560.0, 1400.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .build_ui(|ui| {
            brightfield_shell::design::apply(ui.ctx(), Mode::Light);
            regions_ui(ui);
            // The unstyled case, drawn through the same row the listing uses.
            region_row(ui, RegionEntry::of(&UNSTYLED));
        });
    harness.run();

    for entry in region_catalog() {
        let contract = entry.contract();
        // The name is unique across the arrangement, so it is resolved
        // singly. The contract line is not — two bands on the same rung with
        // the same frame say the same sentence — so this asks that it drew at
        // all rather than that it drew once.
        harness.get_by_label(entry.region.id.as_str());
        assert!(
            harness
                .query_all_by_label(contract.as_str())
                .next()
                .is_some(),
            "{}: its row drew a name and no contract line",
            entry.region.id
        );
    }
    harness.get_by_label(UNSTYLED.id.as_str());
    harness.get_by_label(RegionEntry::of(&UNSTYLED).status.label());
}

/// Gate item 4's anchor: the token-discipline grep lives beside this suite
/// and scans `src/` wholesale, so the gallery file is covered exactly as
/// long as it stays under `src/` — which this pins.
#[test]
fn the_gallery_source_sits_under_the_token_discipline_scan() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("gallery.rs");
    assert!(
        path.is_file(),
        "src/gallery.rs moved out of the token_discipline scan root"
    );
}

// ---------------------------------------------------------------------------
// The registry seam
// ---------------------------------------------------------------------------

/// The gallery-inclusive registry is on the workbench contract: the audit
/// holds its empty state and its toggle verb, the pane is a centre tab, and
/// the flag-off registry does not have it.
#[test]
fn the_gallery_pane_is_a_dev_flagged_centre_tab_on_the_contract() {
    let with = chart_registry_with(true);
    audit(&with, &ChartDoc::empty()).expect("the gallery-inclusive chart view is on the contract");
    let spec = with
        .specs()
        .iter()
        .find(|s| s.id == GALLERY)
        .expect("the gallery pane is in the flag-on registry");
    assert_eq!(
        spec.slot,
        Slot::CentreTab,
        "the gallery is a tab beside the chart, named by the strip"
    );

    let without = chart_registry_with(false);
    assert!(
        !without.ids().contains(&GALLERY),
        "the shipping arrangement must not grow a dev pane"
    );

    // The vocabulary covers the gallery id whatever the flag says — that is
    // what keeps a layout saved flag-on loadable flag-off.
    brightfield_shell::app::publish_item_ids();
    assert!(
        brightfield_workbench::ItemId::known().contains(&GALLERY),
        "publish_item_ids must publish the gallery id unconditionally"
    );
}

/// This test binary does not set the flag, so the environment default is the
/// shipping arrangement — pinned so a stray `export` on a dev machine reads
/// as a local condition, not a green suite.
#[test]
fn the_flag_is_off_unless_the_environment_says_otherwise() {
    if std::env::var_os(brightfield_shell::gallery::GALLERY_VAR).is_none() {
        assert!(!enabled());
    }
}

// ---------------------------------------------------------------------------
// The five-item per-primitive gate
// ---------------------------------------------------------------------------

/// The accesskit role a [`ProbeRole`] names.
fn role_of(probe: ProbeRole) -> Role {
    match probe {
        ProbeRole::Button => Role::Button,
        ProbeRole::Label => Role::Label,
        ProbeRole::TextInput => Role::TextInput,
    }
}

/// One component rendered solo — the same composition `--gallery` captures,
/// so the gate measures the frame an agent would screenshot.
///
/// `pixels_per_point` is a parameter because the two consumers disagree on
/// purpose: the goldens render at 2.0 like the sibling chrome snapshots,
/// while the measuring test runs at 1.0 so a node's reported box is in the
/// same unit as the ladder whatever convention the accessibility layer uses
/// for bounds.
fn solo_harness(
    mut component: Box<dyn Component>,
    mode: Mode,
    pixels_per_point: f32,
) -> Harness<'static> {
    let (w, h) = component.info().solo_size;
    Harness::builder()
        .with_size(egui::vec2(w, h))
        .with_pixels_per_point(pixels_per_point)
        .wgpu()
        .build_ui(move |ui| solo(ui, mode, component.as_mut()))
}

/// Items 1–3 of the gate, over every gated entry.
#[test]
fn every_gated_component_resolves_actuates_and_sits_on_a_rung() {
    for component in catalog() {
        let info = component.info();
        if !info.status.gated() {
            continue;
        }
        let mut harness = solo_harness(component, Mode::Light, 1.0);
        // Settle frames: fonts install on the first, and the alignment
        // scope consumes its accumulator on the next.
        harness.run();

        // 1. The probe resolves by role AND label.
        //
        // Every lookup in this sweep is asked as a *query* first and got
        // second, so the failure carries the id of the entry that is wrong.
        // kittest's own panic prints the query it could not satisfy and the
        // tree it searched, and nothing about who asked — which reads as a
        // lookup failure somewhere in the catalog rather than as one named
        // entry declaring something its specimen does not draw. The
        // comparisons in this loop already name `info.id`; the lookups did
        // not, and a sweep is exactly where that matters.
        let probe_role = role_of(info.probe.role);
        {
            assert!(
                harness
                    .query_by_role_and_label(probe_role, info.probe.label)
                    .is_some(),
                "{}: nothing in the solo frame carries role {:?} with label \
                 {:?} — the entry names a probe its specimen does not draw",
                info.id,
                info.probe.role,
                info.probe.label
            );
            let node = harness.get_by_role_and_label(probe_role, info.probe.label);
            // 3. The declared height: a named ladder value, measured on the
            // probe node or the named node; or written prose.
            match info.height {
                GateHeight::Rung { rung, node: named } => {
                    let rect = match named {
                        Some(label) => {
                            assert!(
                                harness.query_by_label(label).is_some(),
                                "{}: the entry measures a node labelled {:?} \
                                 and the specimen draws none",
                                info.id,
                                label
                            );
                            harness.get_by_label(label).rect()
                        }
                        None => node.rect(),
                    };
                    let expected = rung.value();
                    assert!(
                        (rect.height() - expected).abs() <= 0.5,
                        "{}: measured {} logical pt, {} names {} — pick the \
                         rung the box actually sits on",
                        info.id,
                        rect.height(),
                        rung.name(),
                        expected
                    );
                }
                GateHeight::Intrinsic(reason) => {
                    assert!(!reason.trim().is_empty(), "{}: empty prose", info.id);
                }
            }
        }

        // 2. Keyboard actuation, with no pointer event anywhere in the test.
        if let Some(actuation) = info.actuation {
            assert!(
                harness.query_by_label(actuation.evidence).is_none(),
                "{}: evidence {:?} present before actuation — the assertion \
                 would be vacuous",
                info.id,
                actuation.evidence
            );
            {
                let target = match actuation.focus {
                    FocusTarget::Probe => {
                        harness.get_by_role_and_label(probe_role, info.probe.label)
                    }
                    FocusTarget::Role(role) => {
                        assert!(
                            harness.query_by_role(role_of(role)).is_some(),
                            "{}: the actuation focuses the one node of role \
                             {:?} in the solo frame and the specimen draws \
                             none — a control the keyboard cannot reach is \
                             not a keyboard path",
                            info.id,
                            role
                        );
                        harness.get_by_role(role_of(role))
                    }
                };
                target.focus();
            }
            harness.run();
            match actuation.input {
                ActuationInput::Key(key) => harness.key_press(key),
                ActuationInput::Text(text) => harness.event(egui::Event::Text(text.to_owned())),
            }
            harness.run();
            assert!(
                harness.query_by_label(actuation.evidence).is_some(),
                "{}: {:?} was sent to the focused node and the evidence {:?} \
                 is still absent — the keyboard did not reach the specimen",
                info.id,
                actuation.input,
                actuation.evidence
            );
        }
    }
}

/// Item 5: two goldens per gated component, light and dark, at the repo
/// floor (`kittest.toml`; see the policy comment there before any per-test
/// override).
#[test]
fn every_gated_component_has_light_and_dark_goldens() {
    // One harness per component per mode, so the per-test results are merged
    // into a single `SnapshotResults` — the shape egui_kittest requires for a
    // consistent `UPDATE_SNAPSHOTS=1` pass, and the reason every failure in
    // the sweep is reported rather than only the first.
    let mut results = SnapshotResults::new();
    for (mode, suffix) in [(Mode::Light, "light"), (Mode::Dark, "dark")] {
        for component in catalog() {
            let info = component.info();
            if !info.status.gated() {
                continue;
            }
            let name = format!("gallery_{}_{suffix}", info.id.replace('-', "_"));
            let mut harness = solo_harness(component, mode, 2.0);
            harness.run();
            harness.snapshot_options(&name, &SnapshotOptions::default());
            results.extend(harness.take_snapshot_results());
        }
    }
    // Dropping the merged results is what asserts them.
    drop(results);
}

// ---------------------------------------------------------------------------
// The capture seam
// ---------------------------------------------------------------------------

/// An unknown component id is refused before any GPU work, and the refusal
/// teaches: it lists the catalog, so the next invocation can be right.
#[test]
fn a_shot_of_an_unknown_component_names_the_catalog() {
    let out = std::env::temp_dir().join("bf-gallery-unknown.png");
    let err = capture_component("no-such-component", Mode::Light, 1.0, None, &out)
        .expect_err("an unknown id cannot capture");
    assert!(
        err.contains("no-such-component") && err.contains("list-row"),
        "the error names the ask and the catalog: {err}"
    );
    assert!(!out.exists(), "a refused capture writes nothing");
}
