# The gate's tests use the machine's idle cores

## Why

The pull-request gate takes about 85 minutes on the Mac Studio, and one gate runs at a time, so every fix round on every branch waits that long. `test-default` is 1 h 12 min of it. Most of the machine's 18 cores sit idle during the run.

## What the measurement found

On main at 009f05e6 (2026-11-24), on a quiet machine:

- `test-default` at 18 test threads took 1 h 08 min, against 1 h 11 min at 6. Each binary's time was within a few percent of its time at 6 threads, and the run averaged 228% CPU. More test threads alone buy nothing.
- The cause is `own_files::database_lock()` in brightfield-shell: one process-wide mutex held for the whole of each own-files `arc` run and for discovery's copy of a Protocol's database. Any two tests in one binary that open a Protocol wait on each other, whichever Protocols they open. A sample at 18 threads shows test threads parked in it, with one `arc` child running.
- `open_makes_a_protocol` shows the size of it. Its tests run one by one sum to 195 s, the slowest being 14 s. As a binary it takes 186 s at 1 thread, 102 s at 6 and 107 s at 18. With the lock made a no-op, it takes 43 s at 6 and 27 s at 18, all 31 passing.

## Goals

- Writes to one Protocol's database still exclude a copy of that database; writes to different Protocols' databases no longer wait on each other.
- `test-default`'s wall time drops, measured at 6 and at 18 threads.
- If 18 threads then beats 6, the gate and the nightly run at 18 threads and jobs when no other build holds a slot at their start, and at 6 otherwise (Hugh's ruling, 2026-11-24).
- `docs/pull-request-gate.md` records the gate's wall time on a typical pull request after the change, beside today's 1 h 25 min.

## Non-goals

- Changing which tests run, or moving tests between the one-thread and default groups.
- A machine-wide count of cores. The slot count at the gate's start is the signal; a session that starts compiling mid-gate shares cores with it (a risk Hugh accepted).
- Adopting cargo-nextest unless it measures a real gain over the fixed `cargo test` run; it would add a tool every machine running the gate must install.

## Approaches

1. **Key the lock by database (chosen).** A map from each Protocol's database path to its own mutex. Every caller already knows the Protocol's folder or its database. Rename, which moves a database from one key to another, takes both locks in path order. The lock keeps its one purpose, and the app behaves the same with one Protocol open.
2. **Drop the lock.** The measurement's experiment. It throws away the guarantee that a discovery pass never copies a database part-way through a write, which a user can hit by editing a model while discovery runs.
3. **Cargo-nextest alone.** One process per test would sidestep a process-wide lock in tests, but it leaves the app's lock global, adds a tool to the gate, and is untested against this suite's static per-binary scratch folders.

## Decisions made on Hugh's behalf (he is away)

- The lock's key is the database path, made canonical through its nearest existing ancestor, so two spellings of one folder share a lock. When a caller cannot resolve the database path, it keys on the Protocol folder; discovery's copy cannot happen in that case, so nothing it excludes is lost.
- The test files' own locks are left as they are unless a measurement shows one costing real time. `model_file_edit` holds its lock for each test's whole life, so it is the candidate; the others hold theirs around an open, or guard the working directory and environment variables, which are per process.
- The slot reading is `build-slots status --json`, held slots minus those whose holder is the gate's own process or an ancestor of it. When the tool is missing or its answer cannot be read, the gate runs at 6.

## Left to the builder

The names of the new functions, the test that shows two databases' locks are independent and one database's lock still excludes, and how the gate prints the thread count it chose.
