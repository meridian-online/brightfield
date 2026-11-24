# Plan: test-default under cargo-nextest

Spec: `docs/superpowers/specs/2026-11-24-test-default-under-nextest.md`.

## Tasks

1. **Scratch folders safe across processes.** Every test file whose scratch root is made once and cleared on first use: a fixed name under `std::env::temp_dir()` gains the process id; a root under `CARGO_TARGET_TMPDIR` cleared once per process becomes per test, or moves under `temp_dir()` with the process id. Shared-folder locks are checked as the spec says. Includes the log-binned histogram tests' fixed folder. Verify: `cargo nextest run --locked --workspace` over the default group passes, and the targets touched pass under `cargo test`. Independent of task 2.
2. **xtask runs test-default under nextest.** `cargo nextest run` for the targets, `cargo test --doc` for the doctests, a stop with what to install when nextest is missing, a failure excerpt that reads nextest's output, `.config/nextest.toml` with `typed_sql_reads`' timed test on all threads, and a fresh per-step `TMPDIR` for the test steps. Verify: `cargo test -p xtask`; `cargo xtask ci --step test-default`. Independent of task 1 in code; its full run depends on task 1.
3. **Hosted workflow and docs.** `test.yml` installs cargo-nextest; `docs/pull-request-gate.md` and CLAUDE.md say test-default runs under nextest and why. Verify: `cargo xtask ci --step workflow-coverage`. Depends on 2.
4. **Measure.** test-default under nextest, only when `build-slots status` shows no other holder. Record in `docs/pull-request-gate.md`. Depends on 1 and 2.
5. **Full gate** on the pushed branch: `cargo xtask gate <pr>` (about 50 minutes, every core). Depends on all.
