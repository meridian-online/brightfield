# Plan: the gate's tests use the machine's idle cores

Design: `docs/superpowers/specs/2026-11-24-gate-uses-idle-cores-design.md`. One cargo build at a time; each measurement runs with no other slot held.

## Tasks

1. **Key the database lock by database.** `own_files::database_lock(db)` takes the database path; a static map holds one mutex per canonical path. Callers: `run_loads`, `drop_unmade_silver`, `forget_after_loads`, the model pane's rerun, discovery's `copy_database`, rename's `move_database` (both paths, in order). A unit test shows a held lock on one database does not block another's and does block a second taker of the same one.
   Verify: `cargo test -p brightfield-shell --lib own_files` passes; `cargo test -p brightfield-shell --test open_makes_a_protocol -- --test-threads=18` passes in under 40 s (102 s before).
2. **Measure `test-default` after task 1** at 6 and at 18 threads on a quiet machine, and comment both times on the issue. Depends on 1.
   Verify: both runs pass; times recorded.
3. **Unlock `model_file_edit`** if task 2 shows it as the step's longest binary by a wide margin: each test opens its own copy of the retail extract, and the file's whole-test lock goes. Depends on 2.
   Verify: `cargo test -p brightfield-shell --test model_file_edit -- --test-threads=18` passes three times running, and its time drops.
4. **The gate and the nightly choose their threads from the slots**, if task 2 shows 18 beating 6. A function in xtask reads `build-slots status --json`, counts holders that are not the run's own process or an ancestor, and answers 18 when there are none, else 6; the gate and the nightly set `RUST_TEST_THREADS` and `CARGO_BUILD_JOBS` for their run from it and say which they chose. Unit tests cover the reading: no holders, only our own, another, unreadable output. Depends on 2.
   Verify: `cargo test -p xtask` passes.
5. **Measure cargo-nextest** on `test-default`'s targets at 18 threads; adopt only on a real gain over task 2's 18-thread time. Depends on 1 and 3.
   Verify: time recorded on the issue, with the decision.
6. **Document and measure the gate**: `docs/pull-request-gate.md` says how the gate and the nightly pick their threads and why (the lock finding), and records this branch's own gate wall time beside 1 h 25 min. Depends on 4.
   Verify: `cargo xtask gate <this pull request>` green; its wall time in the doc.
