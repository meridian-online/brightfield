//! Deterministic synthetic datasets, written to Parquet through DuckDB.
//!
//! Every column is a pure function of the row index through DuckDB's `hash()`
//! — no RNG, no seed state — so two runs (and two machines on the same DuckDB
//! version) generate byte-for-byte the same logical data. The distributions
//! are chosen for the scenarios, not for realism: `value_a` is uniform on
//! [0, 100) (a high-cardinality brushed axis — ~every row distinct),
//! `value_b` is a bell-ish sum of three uniforms on [0, ~100), `value_c` is
//! uniform over the 40 integers [0, 40) (a bounded-cardinality brushed axis —
//! the shape where a derived data cube stays small however many rows the
//! table holds), and `category` takes ten values.

use std::path::{Path, PathBuf};

/// Generate (or reuse) the `rows`-row Parquet dataset under `data_dir`.
///
/// An existing file of the right name is reused as-is: the columns are a pure
/// function of the row index, so a present file IS the dataset — regeneration
/// could only reproduce it.
pub fn ensure_dataset(
    conn: &duckdb::Connection,
    data_dir: &Path,
    rows: u64,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("create {}: {e}", data_dir.display()))?;
    // v2: adds `value_c` (the bounded-cardinality axis). The version is in
    // the filename because an existing file is reused as-is.
    let path = data_dir.join(format!("uniform_v2_{rows}.parquet"));
    if path.exists() {
        return Ok(path);
    }
    let sql = format!(
        "COPY (
            SELECT
                i::BIGINT AS id,
                ((hash(i) % 1000000) / 1000000.0 * 100.0)::DOUBLE AS value_a,
                (((hash(i * 2 + 1) % 1000) + (hash(i * 3 + 2) % 1000) + (hash(i * 5 + 3) % 1000)) / 30.0)::DOUBLE AS value_b,
                (hash(i * 11 + 7) % 40)::DOUBLE AS value_c,
                ('c' || (hash(i * 7 + 5) % 10))::VARCHAR AS category
            FROM range({rows}) AS r(i)
        ) TO '{}' (FORMAT PARQUET)",
        path.display()
    );
    conn.execute_batch(&sql)
        .map_err(|e| format!("generate {}: {e}", path.display()))?;
    Ok(path)
}

/// The DuckDB library version, for the machine profile.
pub fn duckdb_version(conn: &duckdb::Connection) -> String {
    conn.query_row("SELECT version()", [], |row| row.get::<_, String>(0))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_is_deterministic_across_connections() {
        let dir = std::env::temp_dir().join(format!("bf-bench-det-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let conn_a = duckdb::Connection::open_in_memory().expect("duckdb");
        let a = ensure_dataset(&conn_a, &dir.join("a"), 1000).expect("dataset a");
        let conn_b = duckdb::Connection::open_in_memory().expect("duckdb");
        let b = ensure_dataset(&conn_b, &dir.join("b"), 1000).expect("dataset b");

        // Compare logical content, not file bytes: Parquet metadata may embed
        // writer details, but the rows must be identical.
        let digest = |p: &Path| -> String {
            let conn = duckdb::Connection::open_in_memory().expect("duckdb");
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) || '/' || SUM(hash(id, value_a, value_b, value_c, category))::VARCHAR \
                     FROM read_parquet('{}')",
                    p.display()
                ),
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("digest")
        };
        assert_eq!(digest(&a), digest(&b), "same rows from independent runs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn value_a_spans_the_brushable_domain() {
        let dir = std::env::temp_dir().join(format!("bf-bench-domain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let conn = duckdb::Connection::open_in_memory().expect("duckdb");
        let p = ensure_dataset(&conn, &dir, 10_000).expect("dataset");
        let (lo, hi): (f64, f64) = conn
            .query_row(
                &format!(
                    "SELECT MIN(value_a), MAX(value_a) FROM read_parquet('{}')",
                    p.display()
                ),
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("range");
        assert!(lo >= 0.0 && hi < 100.0, "domain [0,100): got [{lo},{hi}]");
        assert!(hi - lo > 90.0, "spread covers the domain: got [{lo},{hi}]");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn value_c_is_bounded_cardinality() {
        let dir = std::env::temp_dir().join(format!("bf-bench-card-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let conn = duckdb::Connection::open_in_memory().expect("duckdb");
        let p = ensure_dataset(&conn, &dir, 10_000).expect("dataset");
        let (distinct, lo, hi): (i64, f64, f64) = conn
            .query_row(
                &format!(
                    "SELECT COUNT(DISTINCT value_c), MIN(value_c), MAX(value_c) \
                     FROM read_parquet('{}')",
                    p.display()
                ),
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("cardinality");
        assert_eq!(distinct, 40, "exactly forty distinct values");
        assert!(lo >= 0.0 && hi < 40.0, "domain [0,40): got [{lo},{hi}]");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
