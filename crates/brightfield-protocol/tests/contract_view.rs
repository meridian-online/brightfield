//! The emitted Protocol+Run contract (`contract_version` `"b4/1"`)
//! deserializes and builds the crate's typed `AssetGraph` — mapped ids,
//! edge-ordered (topological) lineage, re-derived view kinds, and a sql step
//! exposing its `sql_text`. Also the skipped-producer / live-stream-fold /
//! version-gate reconciliations.

use std::path::PathBuf;

use brightfield_protocol::contract::{Outcome, StepState};
use brightfield_protocol::graph::AssetKind;
use brightfield_protocol::{
    apply_stream, build_contract_view, collapse_families, fold_stream, layout, load_contract,
    load_contract_with_stream, parse_contract, view_from_contract_bytes, Error, LayoutConfig,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

const CONTRACT: &str = "sample_run.contract.json";
const STREAM: &str = "sample_run.jsonl";

#[test]
fn real_sample_deserializes_and_maps_to_asset_graph() {
    let bytes = std::fs::read(fixture(CONTRACT)).expect("read fixture");
    let contract = parse_contract(&bytes).expect("parse contract");
    assert_eq!(contract.contract_version, "b4/1");
    assert!(contract.is_supported_version());

    let view = build_contract_view(&contract);

    // (1) Node ids are namespaced by protocol + kind + name (flat contract ids
    //     `table:widgets` → `asset.widgets_demo.widgets`).
    assert!(view.graph.nodes.contains_key("asset.widgets_demo.widgets"));
    assert!(view
        .graph
        .nodes
        .contains_key("asset.widgets_demo.widget_tally"));
    let widgets = &view.graph.nodes["asset.widgets_demo.widgets"];
    assert_eq!(widgets.kind, AssetKind::Table);
    assert_eq!(widgets.label, "widgets");
    assert_eq!(widgets.step.as_deref(), Some("load"));

    // (2) produced_by / consumed_by + statement reads become an asset->asset
    //     edge through the producing step's seam.
    assert!(
        view.graph
            .edges
            .iter()
            .any(|e| e.from == "asset.widgets_demo.widgets"
                && e.to == "asset.widgets_demo.widget_tally"
                && e.via.as_deref() == Some("tally")),
        "widgets -> widget_tally via tally: {:?}",
        view.graph.edges
    );

    // (3) The order is topological — a producer precedes its consumer even
    //     though the contract lists assets alphabetically (tally before widgets).
    let pos = |id: &str| {
        view.order
            .iter()
            .position(|x| x == id)
            .expect("id in order")
    };
    assert!(
        pos("asset.widgets_demo.widgets") < pos("asset.widgets_demo.widget_tally"),
        "topological: {:?}",
        view.order
    );

    // (4) A sql step exposes its sql_text.
    let load = &view.steps["load"];
    assert_eq!(load.kind, brightfield_protocol::contract::StepKind::Sql);
    assert_eq!(load.state, StepState::Success);
    let sql = load.sql_text.as_deref().expect("sql step carries sql_text");
    assert!(
        sql.contains("CREATE OR REPLACE TABLE widgets"),
        "sql_text is the real body: {sql}"
    );

    // (5) Measured row_count rides the sidecar; a successful producer is
    //     materialised.
    assert_eq!(view.assets["asset.widgets_demo.widgets"].row_count, Some(5));
    assert!(view.assets["asset.widgets_demo.widgets"].materialized);

    // The built graph feeds the existing layout/collapse pipeline unchanged.
    let g = collapse_families(&view.graph);
    let l = layout(&g, &LayoutConfig::default());
    assert_eq!(l.positions.len(), g.nodes.len(), "every node is placed");

    // Deterministic: two builds yield identical graphs.
    let view2 = build_contract_view(&contract);
    assert_eq!(view.graph, view2.graph);
}

/// A relation produced then read only inside its own model file — and never
/// consumed by another step — is INTERNAL; a `parquet_export` terminal is the
/// Dataset sink; the intermediate stays a TABLE.
#[test]
fn view_kinds_internal_table_and_dataset_are_rederived() {
    let json = r#"{
      "contract_version": "b4/1",
      "run": { "run_id": "r", "protocol": { "name": "p" }, "outcome": "success",
               "finished_at": "2026-07-18T00:00:00Z" },
      "assets": [
        { "id": "table:staged", "kind": "table", "name": "staged",
          "produced_by": "transform", "consumed_by": [] },
        { "id": "table:cleaned", "kind": "table", "name": "cleaned",
          "produced_by": "transform", "consumed_by": ["export"] },
        { "id": "file:build/out.parquet", "kind": "file", "name": "out.parquet",
          "produced_by": "export", "consumed_by": [] }
      ],
      "steps": [
        { "name": "transform", "kind": "sql",
          "sql": { "sql_text": "…", "statements": [
              { "produces": ["staged"], "reads": [] },
              { "produces": ["cleaned"], "reads": ["staged"] } ] },
          "status": { "state": "success" } },
        { "name": "export", "kind": "op",
          "op_ref": { "name": "parquet_export", "version_resolved": "1" },
          "status": { "state": "success" } }
      ]
    }"#;
    let view = view_from_contract_bytes(json.as_bytes()).expect("build");

    assert_eq!(view.graph.nodes["asset.p.staged"].kind, AssetKind::Internal);
    assert_eq!(view.graph.nodes["asset.p.cleaned"].kind, AssetKind::Table);
    assert_eq!(
        view.graph.nodes["file.p.build/out.parquet"].kind,
        AssetKind::Dataset
    );

    let has = |from: &str, to: &str, via: &str| {
        view.graph
            .edges
            .iter()
            .any(|e| e.from == from && e.to == to && e.via.as_deref() == Some(via))
    };
    assert!(
        has("asset.p.staged", "asset.p.cleaned", "transform"),
        "intra-step internal chain"
    );
    assert!(
        has("asset.p.cleaned", "file.p.build/out.parquet", "export"),
        "export consumes cleaned"
    );
}

/// A `finetype_validate` step is a shield on the edge into the asset it
/// guards, never a node of its own.
#[test]
fn finetype_validate_is_a_shield_not_a_node() {
    let json = r#"{
      "contract_version": "b4/1",
      "run": { "run_id": "r", "protocol": { "name": "p" }, "outcome": "success" },
      "assets": [
        { "id": "file:build/out.parquet", "kind": "file", "name": "out.parquet",
          "produced_by": "export", "consumed_by": ["validate"] }
      ],
      "steps": [
        { "name": "export", "kind": "op",
          "op_ref": { "name": "parquet_export" }, "status": { "state": "success" } },
        { "name": "validate", "kind": "op",
          "op_ref": { "name": "finetype_validate" }, "status": { "state": "success" } }
      ]
    }"#;
    let view = view_from_contract_bytes(json.as_bytes()).expect("build");
    // The gate seam exists and is marked; no node belongs to it.
    assert!(view.graph.seams["validate"].gate);
    assert!(view.steps["validate"].gate);
    assert!(view
        .graph
        .nodes
        .values()
        .all(|n| n.step.as_deref() != Some("validate")));
    // The parquet, read only by the gate + validate sidecar, is still the sink.
    assert_eq!(
        view.graph.nodes["file.p.build/out.parquet"].kind,
        AssetKind::Dataset
    );
}

/// An asset whose `produced_by` step was SKIPPED is not treated as materialised
/// (its `row_count` is null) — flagged with an issue, and never the Dataset
/// sink.
#[test]
fn skipped_producer_asset_is_not_materialised() {
    let json = r#"{
      "contract_version": "b4/1",
      "run": { "run_id": "r", "protocol": { "name": "p" }, "outcome": "partial" },
      "assets": [
        { "id": "file:build/out.parquet", "kind": "file", "name": "out.parquet",
          "path": null, "bytes": null, "row_count": null, "content_hash": null,
          "produced_by": "export", "consumed_by": [] }
      ],
      "steps": [
        { "name": "export", "kind": "op",
          "op_ref": { "name": "parquet_export" },
          "status": { "state": "skipped", "skip_reason": "upstream failed" } }
      ]
    }"#;
    let view = view_from_contract_bytes(json.as_bytes()).expect("build");

    let meta = &view.assets["file.p.build/out.parquet"];
    assert!(
        !meta.materialized,
        "a skipped producer leaves the asset unmaterialised"
    );
    assert_eq!(meta.row_count, None);

    let node = &view.graph.nodes["file.p.build/out.parquet"];
    assert!(
        node.issue.is_some(),
        "the unmaterialised asset carries an issue"
    );
    assert_ne!(
        node.kind,
        AssetKind::Dataset,
        "a skipped export is never the materialised sink"
    );
}

#[test]
fn live_stream_folds_onto_step_state_authoritative_json_wins() {
    // The `.json` is the authoritative complete step set; the thinner `.jsonl`
    // only overlays live per-step state (last-line-wins) + the run outcome.
    let mut view = load_contract(&fixture(CONTRACT)).expect("load contract");
    assert!(
        view.steps.values().all(|s| s.live_state.is_none()),
        "no live state before fold"
    );

    let content = std::fs::read_to_string(fixture(STREAM)).expect("read stream");
    let stream = fold_stream(&content);
    apply_stream(&mut view, &stream);

    assert_eq!(view.steps["load"].live_state.as_deref(), Some("success"));
    assert_eq!(view.steps["tally"].live_state.as_deref(), Some("success"));
    assert!(view.run.complete, "run_complete is the reconcile signal");
    assert_eq!(view.run.outcome, Outcome::Success);

    // The one-shot loader wires the same fold.
    let loaded =
        load_contract_with_stream(&fixture(CONTRACT), Some(&fixture(STREAM))).expect("load");
    assert_eq!(loaded.steps["load"].live_state.as_deref(), Some("success"));
}

#[test]
fn unsupported_contract_version_is_rejected() {
    let json = r#"{"contract_version":"z9/0","run":{"run_id":"r","protocol":{"name":"p"},"outcome":"success"},"assets":[],"steps":[]}"#;
    let err = view_from_contract_bytes(json.as_bytes()).expect_err("bad version rejected");
    assert!(
        matches!(err, Error::UnsupportedVersion { .. }),
        "got {err:?}"
    );
}
