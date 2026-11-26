//! The region grid query and the cells predicate, run against an in-memory
//! DuckDB and checked against counts worked out by hand.

use brightfield_sql::ir::Predicate;
use brightfield_sql::regions::{cell_index_sql, region_grid_sql, GridExtent};
use duckdb::Connection;

/// 1,000 rows: `x` runs 0..=999 and `y` is `x % 10`, plus a row with nulls.
fn connection() -> Connection {
    let c = Connection::open_in_memory().expect("duckdb");
    c.execute_batch(
        "CREATE TABLE t AS SELECT CAST(i AS DOUBLE) AS x, CAST(i % 10 AS DOUBLE) AS y \
         FROM range(1000) r(i); \
         INSERT INTO t VALUES (NULL, 3.0), (500.0, NULL);",
    )
    .expect("table");
    c
}

fn grid(c: &Connection, extent: &GridExtent) -> Vec<(i32, i32, f64)> {
    let sql = region_grid_sql("SELECT * FROM t", "x", "y", extent).expect("usable extent");
    let mut stmt = c.prepare(&sql).expect("grid sql prepares");
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("grid runs")
        .map(|r| r.expect("row"))
        .collect()
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).expect("count")
}

#[test]
fn the_grid_counts_only_rows_in_view() {
    let c = connection();
    // The lower half of x: 0..=499 is 500 rows. The null rows are out.
    let extent = GridExtent {
        x: (0.0, 499.0),
        y: (0.0, 9.0),
        bins: 100,
    };
    let cells = grid(&c, &extent);
    let total: f64 = cells.iter().map(|c| c.2).sum();
    assert_eq!(total, 500.0);
    assert!(cells
        .windows(2)
        .all(|w| (w[0].1, w[0].0) < (w[1].1, w[1].0)));
    assert!(cells
        .iter()
        .all(|&(bx, by, _)| (0..100).contains(&bx) && (0..100).contains(&by)));
}

#[test]
fn the_top_edge_falls_in_the_last_bin() {
    let c = connection();
    let extent = GridExtent {
        x: (0.0, 999.0),
        y: (0.0, 9.0),
        bins: 100,
    };
    let cells = grid(&c, &extent);
    // The one row at x = 999 (y = 9) sits on both top edges.
    let top = cells.iter().find(|&&(bx, by, _)| bx == 99 && by == 99);
    assert_eq!(top.map(|c| c.2), Some(1.0));
    assert!(cells.iter().all(|&(bx, by, _)| bx <= 99 && by <= 99));
    assert_eq!(cells.iter().map(|c| c.2).sum::<f64>(), 1000.0);
}

#[test]
fn a_cells_predicate_selects_exactly_the_rows_in_those_cells() {
    let c = connection();
    let extent = GridExtent {
        x: (0.0, 999.0),
        y: (0.0, 9.0),
        bins: 100,
    };
    // x = i falls in bin floor(i / 999 * 100) = floor(i * 0.1001), so bin 0
    // holds i = 0..=9 and bin 1 holds i = 10..=19. y = i % 10 falls in bin
    // floor(y / 9 * 100): 0, 11, 22, 33, 44, 55, 66, 77, 88, 99.
    // Cells (bx = 0, by = 0): the row (0, 0). Cells (bx = 1, by = 99): the row
    // (19, 9), since the only i in 10..=19 with i % 10 = 9 is 19.
    let cells = vec![99 * 100 + 1, 0, 0];
    let p = Predicate::cells("x", "y", extent.clone(), cells);
    let selected = count(&c, &format!("SELECT count(*) FROM t WHERE {p}"));
    assert_eq!(selected, 2);

    // The cell expression agrees with the grid query on every occupied cell.
    // It needs the null guard the grid and the predicate carry: `least` skips a
    // null, so a null coordinate would otherwise land in the last bin.
    let index = cell_index_sql("x", "y", &extent);
    for (bx, by, n) in grid(&c, &extent) {
        let in_cell = count(
            &c,
            &format!(
                "SELECT count(*) FROM t WHERE x IS NOT NULL AND y IS NOT NULL AND {index} = {}",
                by * 100 + bx
            ),
        );
        assert_eq!(in_cell as f64, n, "cell ({bx}, {by})");
    }
}
