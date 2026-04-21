//! Pure-function DuckDB SQL emitter for Mosaic spec data sources.
//!
//! Translates [`brightfield_spec::ast::DataSource`] nodes into executable DuckDB
//! SQL. Each data source produces a `CREATE OR REPLACE VIEW` statement (or
//! `ATTACH` for `.duckdb`/`.db` files) at spec-mount time.
//!
//! This crate performs **no I/O and holds no DuckDB connection** — it is a
//! pure string-generation layer. Card 0003 extends it with per-plot SELECT
//! emission.

pub mod binding;
pub mod conform;
pub mod emit;
pub mod error;
pub mod ir;
pub mod lower;
pub mod passes;
pub mod render;
pub(crate) mod source;
