# test-default under cargo-nextest

## Goal

`cargo xtask ci --step test-default` runs its targets with `cargo nextest run` instead of `cargo test`, and every test file's scratch folders are safe when each test runs in its own process and when two runs overlap on one machine. Measured on 2026-11-24, test-default's targets took 11 min 18 s under cargo-nextest at 18 threads against 35 min 15 s under cargo test; two tests failed because of shared scratch folders.

## Goals

- test-default runs its targets (`--bins` and every non-serial `--test`) through `cargo nextest run --locked --workspace`, at the run's test threads, then the doctests through `cargo test --doc`, which nextest does not run.
- A machine without `cargo-nextest` stops the step with what to install, as a missing `cargo-deny` does.
- The step's failure excerpt reads nextest's output as well as libtest's.
- No test clears or shares a scratch folder that another test process, or another run, is using. A folder made once and cleared on first use is named for its owner: a test (a fixed name only that test uses) or a process (the process id).
- Tests that serialise on an in-process lock are checked: where the lock guards process-wide state (environment, working directory, statics) a process per test already isolates them; where it guards a shared folder, the folder becomes per test or per process.
- `typed_sql_reads`' timed test still runs with nothing beside it, through nextest's `threads-required`.
- The test steps run with their own `TMPDIR`, made fresh under the system temp folder and removed when the step ends, so per-process folders do not pile up in a session's temp folder. Folders left by a killed run are swept by the next.
- The hand-run hosted `test.yml` installs cargo-nextest.
- `docs/pull-request-gate.md` records the change and the measured time.

## Non-goals

- test-one-thread stays on `cargo test -- --test-threads=1`. Its GPU-bound targets are serial by design.
- No test is rewritten beyond its scratch-folder and serialisation setup.
- No change to the gate's core count rule.

## Calls made on Hugh's behalf (he was away)

- Doctests stay on cargo test inside test-default, since nextest cannot run them.
- nextest keeps its default fail-fast, matching the gate's stop-at-first-failure.
- The per-step `TMPDIR` is added because 12,199 test scratch folders, 59 GB, already sat in this machine's temp folder from per-process names under cargo test, and a process per test would multiply them by about fifteen.
- Scratch roots under `CARGO_TARGET_TMPDIR` that were cleared once per process become per test where each test already had its own subfolder, so nothing accumulates in the target directory.
