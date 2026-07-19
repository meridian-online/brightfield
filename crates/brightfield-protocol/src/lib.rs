//! Protocol run -> typed asset graph -> deterministic layout.
//!
//! The asset-first protocol view's headless spine: derive the typed asset-
//! lineage graph (data states are nodes, steps shrink to seams), collapse
//! parameterised step families, and lay the graph out with a hand-ported
//! deterministic Sugiyama pass.
//!
//! **Two inputs, one model.** The **default** input is the arcform Protocol+Run
//! JSON contract emitted by `arc run` ([`contract`] structs, [`contract_graph`]
//! builder): the run already lists its assets and steps, so the view renders a
//! real run — with per-asset row counts, per-step status, and a live `.jsonl`
//! stream ([`stream`]) folded on top. The interim [`manifest`] parser (an
//! arcform `arcform.yaml` + its `models/*.sql`) is the **gated offline/fixture
//! fallback** — it infers the same graph without a run, for demos and tests.
//! Both produce the crate's one [`AssetGraph`]/[`Seam`]/[`Edge`] model, so the
//! same layout, family collapse, and renderer draw either.
//!
//! **Framework-free by contract:** no gpui, no vello, no duckdb — pure data +
//! geometry, so `brightfield-render` can draw the result and the whole pipeline
//! is unit-testable without pixels. Everything on the graph/layout path is
//! `BTreeMap`/`BTreeSet`/`Vec` — **no `HashMap` anywhere** — so the same input
//! always yields the same coordinates.
//!
//! **Dependency chain:** `brightfield-sql` (statement parse) -> this crate ->
//! `brightfield-render` (asset_scene) -> `brightfield-app` (dump arm).

pub mod collapse;
pub mod contract;
pub mod contract_graph;
pub mod error;
pub mod graph;
pub mod layout;
pub mod manifest;
pub mod nav;
pub mod panel;
pub mod sheet;
pub mod sql_assets;
pub mod stream;

use std::fs;
use std::path::Path;

pub use collapse::collapse_families;
pub use contract::{parse_contract, Contract, SUPPORTED_CONTRACT_FAMILY, SUPPORTED_CONTRACT_VERSION};
pub use contract_graph::{
    apply_stream, build_contract_view, AssetMeta, ContractView, RunView, SeamStatus, StepView,
};
pub use error::Error;
pub use graph::{AssetGraph, AssetKind, AssetNode, Edge, Seam, SeamKind};
pub use layout::{layout, Flow, Layout, LayoutConfig};
pub use manifest::{is_protocol_manifest, parse_manifest_str, Manifest, Step};
pub use nav::{Dir, FoldOutcome, ProtocolNav};
pub use panel::{inspector_for, kind_label, outline_order, outline_rows, InspectorFacts, OutlineRow};
pub use sheet::{StepRow, StepsSheet};
pub use stream::{fold_stream, StreamReader, StreamState};

/// Build a [`ContractView`] from raw Protocol+Run contract bytes — the default
/// entry point. Enforces the supported `contract_version` family.
///
/// # Errors
/// - [`Error::Parse`] if the bytes are not a structurally valid contract.
/// - [`Error::UnsupportedVersion`] if `contract_version` is outside the
///   [`SUPPORTED_CONTRACT_FAMILY`].
pub fn view_from_contract_bytes(bytes: &[u8]) -> Result<ContractView, Error> {
    let contract = parse_contract(bytes)?;
    if !contract.is_supported_version() {
        return Err(Error::UnsupportedVersion { found: contract.contract_version });
    }
    Ok(build_contract_view(&contract))
}

/// Load a contract file into a [`ContractView`].
///
/// # Errors
/// [`Error::Io`] if the file cannot be read, else the errors of
/// [`view_from_contract_bytes`].
pub fn load_contract(path: &Path) -> Result<ContractView, Error> {
    let bytes = fs::read(path).map_err(|source| Error::Io { path: path.to_path_buf(), source })?;
    view_from_contract_bytes(&bytes)
}

/// Load a contract file and, when a live `.jsonl` stream path is given, fold it
/// onto the view's step states (last-line-wins per step; the `.json` is the
/// authoritative complete step set).
///
/// # Errors
/// The errors of [`load_contract`]; a stream that cannot be read surfaces as
/// [`Error::Io`], while a missing stream file is not an error (no live state).
pub fn load_contract_with_stream(
    contract_path: &Path,
    stream_path: Option<&Path>,
) -> Result<ContractView, Error> {
    let mut view = load_contract(contract_path)?;
    if let Some(stream_path) = stream_path {
        let mut reader = StreamReader::open(stream_path);
        let state = reader
            .poll()
            .map_err(|source| Error::Io { path: stream_path.to_path_buf(), source })?;
        apply_stream(&mut view, state);
    }
    Ok(view)
}
