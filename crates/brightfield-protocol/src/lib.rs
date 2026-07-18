//! Protocol manifest -> typed asset graph -> deterministic layout (card 0025).
//!
//! The asset-first protocol view's headless spine: parse an arcform Protocol
//! manifest, derive the typed asset-lineage graph (data states are nodes,
//! steps shrink to seams), collapse parameterised step families, and lay the
//! graph out with a hand-ported deterministic Sugiyama pass.
//!
//! **Framework-free by contract:** no gpui, no vello, no duckdb — pure data +
//! geometry, so `brightfield-render` can draw the result and the whole
//! pipeline is unit-testable without pixels. Everything on the graph/layout
//! path is `BTreeMap`/`BTreeSet`/`Vec` — **no `HashMap` anywhere** — so the
//! same manifest always yields the same coordinates (pds-ac06).
//!
//! **Dependency chain:** `brightfield-sql` (statement parse) -> this crate ->
//! `brightfield-render` (asset_scene) -> `brightfield-app` (dump arm).

pub mod collapse;
pub mod graph;
pub mod layout;
pub mod manifest;
pub mod sql_assets;

pub use collapse::collapse_families;
pub use graph::{AssetGraph, AssetKind, AssetNode, Edge, Seam, SeamKind};
pub use layout::{layout, Layout, LayoutConfig};
pub use manifest::{is_protocol_manifest, parse_manifest_str, Manifest, Step};
