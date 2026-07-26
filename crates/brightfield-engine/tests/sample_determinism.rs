//! What DuckDB can and cannot promise about a pushed-down sample.
//!
//! A sampled picture is only honest if the same filter state always draws the
//! same points: a drag that reshuffles the sample makes points appear and
//! vanish for reasons the reader cannot see, and the reader has no way to tell
//! that from real movement in the data. So the sampling clause is chosen for
//! **determinism**, and this file is where that choice is checked against the
//! DuckDB actually linked in — not against a CLI on someone's PATH, which is a
//! different build at a different version.
//!
//! Four properties are asserted:
//!
//! 1. `TABLESAMPLE reservoir` **reshuffles** when the input changes. It is a
//!    streaming reservoir over whatever rows reach it, so narrowing a filter
//!    re-draws the survivors — disqualifying, and asserted here so nobody
//!    reaches for it later thinking a seed would have fixed it.
//! 2. Seeded `bernoulli` is **position-dependent, not row-dependent**: reorder
//!    the input and a different set survives. Also disqualifying.
//! 3. A **power-of-two hash modulus** is a pure function of the row's own
//!    bytes, so it survives both, and successive moduli **nest**: every row
//!    kept at `% 8 = 0` is also kept at `% 4 = 0` and `% 2 = 0`. Halving the
//!    modulus can therefore only ADD points. A non-power-of-two modulus
//!    forecloses that permanently, which is why the IR takes the exponent.
//! 4. The **placement** of that modulus relative to the highlight projection is
//!    what decides whether a brush reshuffles the drawn set — measured over rows
//!    in both nestings, rather than argued from the emitted SQL. The emitter's
//!    own guard lives in `brightfield-sql`'s `sampling` tests; this is the
//!    measurement it stands on, and it is also where the "modulus ≥ 32" rule
//!    that guard cites comes from.

use duckdb::Connection;

/// One table, one shape, used by every check here.
fn seeded(conn: &Connection, rows: u64) {
    conn.execute_batch(&format!(
        "CREATE OR REPLACE TABLE t AS
         SELECT i AS id,
                (i * 2654435761) % 100000 AS a,
                (i % 97) AS b
         FROM range({rows}) tbl(i);"
    ))
    .expect("seed table");
}

fn ids(conn: &Connection, sql: &str) -> Vec<i64> {
    let mut stmt = conn.prepare(sql).expect("prepare");
    let rows = stmt
        .query_map([], |r| r.get::<_, i64>(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    rows
}

fn scalar_u64(conn: &Connection, sql: &str) -> u64 {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0))
        .map(|v| v as u64)
        .expect("scalar")
}

/// The linked DuckDB's own version string, printed so a disagreement between
/// this file's claims and a future run is attributable rather than mysterious.
#[test]
fn report_bundled_duckdb_version() {
    let conn = Connection::open_in_memory().expect("open");
    let v: String = conn
        .query_row("SELECT version()", [], |r| r.get(0))
        .expect("version");
    eprintln!("bundled DuckDB version: {v}");
    assert!(v.starts_with('v'), "unexpected version string: {v}");
}

/// Reservoir `TABLESAMPLE` is a streaming reservoir over the rows that reach
/// it, so a filter change re-draws it. The overlap between the sample under a
/// wide filter and the sample under a narrow one is far below what a stable
/// sample would give — the point being that the surviving set is not a
/// property of the rows.
#[test]
fn reservoir_tablesample_reshuffles_when_the_filter_changes() {
    let conn = Connection::open_in_memory().expect("open");
    seeded(&conn, 10_000);

    let wide = ids(
        &conn,
        "SELECT id FROM (SELECT * FROM t WHERE a < 100000) USING SAMPLE reservoir(1000 ROWS) \
         ORDER BY id",
    );
    let narrow = ids(
        &conn,
        "SELECT id FROM (SELECT * FROM t WHERE a < 90000) USING SAMPLE reservoir(1000 ROWS) \
         ORDER BY id",
    );

    // Every id in `narrow` satisfies the wide predicate too, so a sample that
    // were a property of the ROW would be almost entirely retained.
    // A row-determined sample would retain ~all of `narrow`; chance alone
    // retains ~10% (1000 picks out of 10000 rows). Measured on the bundled
    // v1.5.2: 102/1000 — chance, not retention.
    let kept = narrow.iter().filter(|id| wide.contains(id)).count();
    assert!(
        kept * 2 < narrow.len(),
        "reservoir retained {kept}/{} across a filter change — anywhere near total \
         retention would mean it had become row-determined, and the determinism \
         argument for the hash modulus would need re-reading before it is trusted",
        narrow.len()
    );
}

/// Seeded `bernoulli` decides per ROW POSITION in the stream, not per row
/// value: adding an `ORDER BY` under the same seed changes which rows survive.
/// A brush that changes the scan order therefore changes the picture.
#[test]
fn seeded_bernoulli_is_position_dependent_not_row_dependent() {
    let conn = Connection::open_in_memory().expect("open");
    seeded(&conn, 10_000);

    let natural = ids(
        &conn,
        "SELECT id FROM (SELECT * FROM t) USING SAMPLE bernoulli(10%) REPEATABLE (42) ORDER BY id",
    );
    let reordered = ids(
        &conn,
        "SELECT id FROM (SELECT * FROM t ORDER BY b, id) USING SAMPLE bernoulli(10%) \
         REPEATABLE (42) ORDER BY id",
    );

    let kept = reordered.iter().filter(|id| natural.contains(id)).count();
    assert!(
        kept * 2 < reordered.len(),
        "seeded bernoulli retained {kept}/{} across a pure reordering — it would have to be \
         row-determined for that, and it is not",
        reordered.len()
    );
}

/// The shape actually shipped. `hash(<row>)` is a pure function of the row's
/// own values, so the surviving set is identical under any filter or ordering,
/// and power-of-two moduli nest exactly.
#[test]
fn power_of_two_hash_moduli_nest_and_survive_filter_and_order_changes() {
    let conn = Connection::open_in_memory().expect("open");
    seeded(&conn, 1_000_000);

    // Nesting: no row survives a finer modulus without surviving every coarser
    // one. Checked as a count of violations over the whole million rows.
    for (fine, coarse) in [(8_u32, 4_u32), (16, 8), (64, 16), (1024, 64)] {
        let violations = scalar_u64(
            &conn,
            &format!(
                "SELECT count(*) FROM t WHERE hash(t) % {fine} = 0 AND hash(t) % {coarse} <> 0"
            ),
        );
        assert_eq!(
            violations, 0,
            "modulus {fine} must nest inside {coarse}, found {violations} rows that do not"
        );
    }

    // Row-determined, not position-determined: the same predicate over a
    // reordered scan keeps exactly the same ids.
    let natural = ids(
        &conn,
        "SELECT id FROM t WHERE hash(t) % 1024 = 0 ORDER BY id",
    );
    let reordered = ids(
        &conn,
        "SELECT id FROM (SELECT * FROM t ORDER BY b DESC, id DESC) AS _s \
         WHERE hash(_s) % 1024 = 0 ORDER BY id",
    );
    assert_eq!(
        natural, reordered,
        "the same rows must survive under any scan order"
    );
    assert!(
        !natural.is_empty(),
        "a 1/1024 sample of 10^6 rows must not be empty"
    );

    // And under a narrowing filter the sample is exactly the restriction of
    // the wider sample — no re-draw.
    let wide: Vec<i64> = ids(
        &conn,
        "SELECT id FROM t WHERE hash(t) % 1024 = 0 ORDER BY id",
    );
    let narrow = ids(
        &conn,
        "SELECT id FROM (SELECT * FROM t WHERE a < 50000) AS _s WHERE hash(_s) % 1024 = 0 \
         ORDER BY id",
    );
    assert!(
        narrow.iter().all(|id| wide.contains(id)),
        "narrowing the filter must only ever REMOVE points from the sample"
    );
    assert!(
        !narrow.is_empty(),
        "the narrowed sample must not be empty, or the check proves nothing"
    );
}

/// The exact clause the IR renders, run against a subquery alias — the form
/// `Sample` emits. A struct-valued `hash(<alias>)` over the whole row is the
/// part most likely to break under a DuckDB upgrade, so it is exercised
/// literally rather than paraphrased.
#[test]
fn the_emitted_clause_shape_binds_and_selects_the_expected_share() {
    let conn = Connection::open_in_memory().expect("open");
    seeded(&conn, 100_000);
    let n = scalar_u64(
        &conn,
        "SELECT count(*) FROM (SELECT * FROM (SELECT * FROM t) AS _s WHERE hash(_s) % 16 = 0)",
    );
    // A hash is not a perfectly uniform partition; assert the order of
    // magnitude, not an exact count (which would be a hash-stability
    // assertion, and this project does not rely on that).
    assert!(
        (4_000..9_000).contains(&n),
        "a 1/16 sample of 100000 rows landed at {n} — far from the ~6250 a usable hash gives"
    );
}

/// **The placement, measured on rows.** The highlight pass appends a
/// `(<pred>) AS __bf_selected` boolean; whether the sample sits ABOVE or BELOW
/// that projection decides whether a brush redraws which points are on screen.
///
/// Both nestings are written out literally here rather than emitted, so this
/// stays a measurement of DuckDB's behaviour and cannot go vacuous when the
/// emitter changes. The emitter's own placement guard is
/// `sample_sits_below_the_highlight_projection_so_a_brush_cannot_reshuffle_it`
/// in `brightfield-sql`; this is what that guard is guarding.
///
/// It also establishes the **modulus ≥ 32** rule that guard cites. Below 32 the
/// row hash is blind to the appended boolean — the same check at 8 or 16 passes
/// in BOTH nestings and proves nothing — which is a second, independent way for
/// a test of this property to be vacuous.
///
/// Measured on the bundled DuckDB over 200 000 rows, moving one brush:
///
/// | modulus | below (shipped) | above (mutant) |
/// |---|---|---|
/// | 32 | 6 255 drawn, 0 moved | 6 374 drawn, 2 636 moved (41 %) |
/// | 64 | 3 136 drawn, 0 moved | 3 103 drawn, 1 304 moved (42 %) |
/// | 128 | 1 561 drawn, 0 moved | 1 563 drawn, 673 moved (43 %) |
#[test]
fn the_highlight_flag_must_stay_outside_the_hashed_tuple() {
    let conn = Connection::open_in_memory().expect("open");
    seeded(&conn, 200_000);

    /// The shipped nesting: hash the row, THEN append the flag.
    fn below(pred: &str, m: u32) -> String {
        format!(
            "SELECT id FROM (SELECT *, ({pred}) AS __bf_selected \
             FROM (SELECT * FROM t) AS _s WHERE hash(_s) % {m} = 0) ORDER BY id"
        )
    }
    /// The mutant nesting: append the flag, THEN hash the row including it.
    fn above(pred: &str, m: u32) -> String {
        format!(
            "SELECT id FROM (SELECT * FROM (SELECT *, ({pred}) AS __bf_selected FROM t) \
             AS _s WHERE hash(_s) % {m} = 0) ORDER BY id"
        )
    }

    // Two brush states over the same rows. Only the highlight predicate moves;
    // no row is filtered out by either, so a stable sample must draw exactly
    // the same points under both.
    //
    // `a` is spread over 0..100000, so these two flag exactly the rows in
    // [50000, 90000) differently — 40 % of the table. That 40 % is also the
    // fraction of the drawn set the broken nesting is EXPECTED to lose: a row
    // whose flag did not change hashes the same either way and stays, and a row
    // whose flag did change is re-rolled at 1-in-`m` and almost never survives.
    // The assertion below is set against a fifth rather than against that 40 %,
    // so it is a statement about "large" and not a fit to one hash's output.
    let (brush_a, brush_b) = ("a < 50000", "a < 90000");

    for m in [32_u32, 64, 128] {
        let stable_a = ids(&conn, &below(brush_a, m));
        let stable_b = ids(&conn, &below(brush_b, m));
        assert!(
            !stable_a.is_empty(),
            "fixture check: a 1-in-{m} sample of 200000 rows must not be empty"
        );
        assert_eq!(
            stable_a, stable_b,
            "modulus {m}: with the flag OUTSIDE the hashed tuple, moving the brush must \
             leave the drawn set byte-identical"
        );

        let shuffled_a = ids(&conn, &above(brush_a, m));
        let shuffled_b = ids(&conn, &above(brush_b, m));
        let moved = shuffled_a
            .iter()
            .filter(|id| !shuffled_b.contains(id))
            .count();
        eprintln!(
            "modulus {m:>4}: below → {} drawn, 0 moved;  above → {} drawn, {moved} moved ({:.0} %)",
            stable_a.len(),
            shuffled_a.len(),
            moved as f64 * 100.0 / shuffled_a.len() as f64,
        );
        assert!(
            moved * 5 > shuffled_a.len(),
            "modulus {m}: with the flag INSIDE the hashed tuple only {moved} of {} drawn \
             rows changed under a brush. The whole point of pinning the placement is that \
             this number is large — if it has collapsed, this test has stopped measuring \
             the failure it exists to describe",
            shuffled_a.len()
        );
    }

    // And why the rule is "≥ 32": below that the hash does not see the boolean
    // at all, so the mutant nesting looks perfectly stable. A guard written at
    // 8 or 16 would pass on broken code.
    for m in [8_u32, 16] {
        let blind_a = ids(&conn, &above(brush_a, m));
        let blind_b = ids(&conn, &above(brush_b, m));
        assert_eq!(
            blind_a, blind_b,
            "modulus {m}: expected the row hash to be BLIND to the appended boolean here. \
             If this now differs, the hash has changed and the ≥ 32 rule quoted in the \
             emitter's placement guard needs re-measuring, not deleting"
        );
    }
}
