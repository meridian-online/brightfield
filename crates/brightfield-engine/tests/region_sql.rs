//! The region grid query, the cells predicate and the region label query, run
//! against an in-memory DuckDB and checked against counts worked out by hand.

use brightfield_sql::ir::Predicate;
use brightfield_sql::regions::{
    cell_index_sql, label_kind, region_grid_sql, region_label_sql, GridExtent, LabelColumn,
    LabelRole, NUMBER_BINS,
};
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
    let p = Predicate::cells("x", "y", extent.clone(), cells).expect("usable extent");
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

/// Rows for the label query, on a 10 × 10 grid over `(0, 10)` on both axes:
/// three in the left band of cells, one in the right band, one between them,
/// one with a null coordinate and one outside the extent.
fn label_connection() -> Connection {
    let c = Connection::open_in_memory().expect("duckdb");
    c.execute_batch(
        "CREATE TABLE q (x DOUBLE, y DOUBLE, place VARCHAR, species VARCHAR, value DOUBLE); \
         INSERT INTO q VALUES \
           (1.5, 1.5, '45 km SW of Severo-Kuril''sk, Russia', 'Gentoo', 10), \
           (1.5, 2.5, '120 km E of Petropavlovsk-Kamchatsky, Russia', 'Gentoo ', 20), \
           (2.5, 1.5, 'Komandorskiye Ostrova, Russia', 'Adelie', 30), \
           (7.5, 7.5, '100 km NNE of Neiafu, Tonga', 'Adelie', 40), \
           (4.5, 4.5, 'Fiji region', NULL, 50), \
           (NULL, 1.0, 'Nowhere, Russia', 'Gentoo', 60), \
           (20.0, 1.0, 'Outside, Russia', 'Gentoo', 70);",
    )
    .expect("table");
    c
}

/// Region 0 is every cell with `bx` in 0..3, region 1 every cell with `bx` in
/// 6..9, on the grid `label_connection`'s rows lie on.
fn banded_cells() -> Vec<(u32, u32)> {
    (0..10)
        .flat_map(|by| {
            (0..3)
                .map(move |bx| (by * 10 + bx, 0))
                .chain((6..9).map(move |bx| (by * 10 + bx, 1)))
        })
        .collect()
}

type SummaryRow = (Option<i32>, i32, String, Option<String>, f64);

fn summary(c: &Connection, sql: &str) -> Vec<SummaryRow> {
    let mut stmt = c.prepare(sql).expect("label sql prepares");
    stmt.query_map([], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
    })
    .expect("label sql runs")
    .map(|r| r.expect("row"))
    .collect()
}

fn fact(rows: &[SummaryRow], col: Option<i32>, region: i32, kind: &str, key: Option<&str>) -> f64 {
    let found: Vec<f64> = rows
        .iter()
        .filter(|r| r.0 == col && r.1 == region && r.2 == kind && r.3.as_deref() == key)
        .map(|r| r.4)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "{col:?} {region} {kind} {key:?} in {rows:#?}"
    );
    found[0]
}

#[test]
fn the_label_query_runs_and_counts_rows_per_region() {
    let c = label_connection();
    let extent = GridExtent {
        x: (0.0, 10.0),
        y: (0.0, 10.0),
        bins: 10,
    };
    let columns = [
        LabelColumn {
            name: "place".into(),
            role: LabelRole::Text,
        },
        LabelColumn {
            name: "species".into(),
            role: LabelRole::Category,
        },
        LabelColumn {
            name: "value".into(),
            role: LabelRole::Number,
        },
    ];
    let sql = region_label_sql(
        "SELECT * FROM q",
        "x",
        "y",
        &extent,
        &banded_cells(),
        &columns,
    )
    .expect("usable extent");
    let rows = summary(&c, &sql);

    // Rows per region; the row between the bands is region -1, and the rows
    // with a null coordinate or outside the extent are in no region at all.
    assert_eq!(fact(&rows, None, 0, label_kind::ROWS, None), 3.0);
    assert_eq!(fact(&rows, None, 1, label_kind::ROWS, None), 1.0);
    assert_eq!(fact(&rows, None, -1, label_kind::ROWS, None), 1.0);
    let totals: f64 = rows
        .iter()
        .filter(|r| r.2 == label_kind::ROWS)
        .map(|r| r.4)
        .sum();
    assert_eq!(totals, 5.0);

    // Terms count rows, in the capitalisation the rows use. Numbers, distance
    // and compass words and short words are dropped; a hyphen and an
    // apostrophe stay inside a word, and a phrase never crosses a comma.
    let place = Some(0);
    assert_eq!(fact(&rows, place, 0, label_kind::TERM, Some("Russia")), 3.0);
    assert_eq!(
        fact(&rows, place, 0, label_kind::TERM, Some("Severo-Kuril'sk")),
        1.0
    );
    assert_eq!(
        fact(
            &rows,
            place,
            0,
            label_kind::TERM,
            Some("Komandorskiye Ostrova")
        ),
        1.0
    );
    for unwritten in ["Severo-Kuril'sk Russia", "Ostrova Russia", "Kuril'sk"] {
        assert!(
            !rows.iter().any(|r| r.3.as_deref() == Some(unwritten)),
            "{unwritten} is not a term"
        );
    }
    assert_eq!(fact(&rows, place, 1, label_kind::TERM, Some("Tonga")), 1.0);
    for dropped in ["45", "km", "SW", "of", "NNE", "E", "100"] {
        assert!(
            !rows.iter().any(|r| r.0 == place
                && r.3
                    .as_deref()
                    .is_some_and(|k| k.split(' ').any(|w| w.eq_ignore_ascii_case(dropped)))),
            "{dropped} is not a term"
        );
    }

    // Category values count rows, trimmed; a null is not a value.
    let species = Some(1);
    assert_eq!(
        fact(&rows, species, 0, label_kind::VALUE, Some("Gentoo")),
        2.0
    );
    assert_eq!(
        fact(&rows, species, 0, label_kind::VALUE, Some("Adelie")),
        1.0
    );
    assert!(!rows.iter().any(|r| r.0 == species && r.1 == -1));

    // Exact quartiles per region: region 0 holds 10, 20 and 30.
    let value = Some(2);
    assert_eq!(fact(&rows, value, 0, label_kind::Q1, None), 15.0);
    assert_eq!(fact(&rows, value, 0, label_kind::Q3, None), 25.0);
    assert_eq!(fact(&rows, value, 0, label_kind::PRESENT, None), 3.0);

    // The histogram spans the view's 10..=50 in NUMBER_BINS bins, so a value v
    // is in bin floor((v - 10) / 40 * 1024), and 50 is in the last bin.
    assert_eq!(NUMBER_BINS, 1024);
    assert_eq!(fact(&rows, value, -1, label_kind::LO, None), 10.0);
    assert_eq!(fact(&rows, value, -1, label_kind::HI, None), 50.0);
    let bins: Vec<(i32, &str, f64)> = rows
        .iter()
        .filter(|r| r.0 == value && r.2 == label_kind::BIN)
        .map(|r| (r.1, r.3.as_deref().unwrap_or_default(), r.4))
        .collect();
    assert_eq!(
        bins,
        [
            (-1, "1023", 1.0),
            (0, "0", 1.0),
            (0, "256", 1.0),
            (0, "512", 1.0),
            (1, "768", 1.0)
        ]
    );
    // Each bin also carries the least and greatest number in it.
    assert_eq!(
        fact(&rows, value, 0, label_kind::BIN_MIN, Some("256")),
        20.0
    );
    assert_eq!(
        fact(&rows, value, 0, label_kind::BIN_MAX, Some("256")),
        20.0
    );
    assert_eq!(
        fact(&rows, value, -1, label_kind::BIN_MAX, Some("1023")),
        50.0
    );

    // The rows are read once, however many columns are summarised.
    let plan: String = c
        .query_row(&format!("EXPLAIN {sql}"), [], |r| r.get(1))
        .expect("explain");
    assert_eq!(plan.matches("Seq Scan").count(), 1, "{plan}");
}

#[test]
fn the_label_query_needs_a_usable_extent_and_a_region() {
    let columns = [LabelColumn {
        name: "species".into(),
        role: LabelRole::Category,
    }];
    let usable = GridExtent {
        x: (0.0, 10.0),
        y: (0.0, 10.0),
        bins: 10,
    };
    let flat = GridExtent {
        x: (1.0, 1.0),
        ..usable.clone()
    };
    let none = region_label_sql(
        "SELECT * FROM q",
        "x",
        "y",
        &flat,
        &banded_cells(),
        &columns,
    );
    assert_eq!(none, None);
    assert_eq!(
        region_label_sql("SELECT * FROM q", "x", "y", &usable, &[], &columns),
        None
    );
}
