# Build the tests once, run them in shards

## Why

When a pull request changes `Cargo.lock`, the test shards start from a cache that is effectively empty, and three or four of the five get cancelled at the 36-minute job limit with no test failing. The cache does get restored, but from the previous lockfile's key, and Swatinem/rust-cache deletes every artifact in `target/` older than a week whenever the key is only a partial match. Warm runs never rebuild the dependencies, so nearly every artifact is older than that, and a run recompiles about 470 crates in 14–15 minutes, against 12 crates in 4.4 minutes warm. Each of the five shards pays that compile.

The DuckDB 2.0 move on 29 Oct rewrites `Cargo.lock` and recompiles DuckDB, which alone is about 9 minutes of a cold compile with the rest of the build waiting on it. No cache fix avoids that, and today it would land on every shard of every run on that pull request.

## Goals

- A run that starts with no usable cache finishes every job inside the 36-minute limit, and so does a run that recompiles DuckDB.
- The compile is paid once per run, not once per shard.
- The suite that runs is unchanged: the same targets, the same one-thread and default-threads groups, the same test counts, the same libtest behaviour (one process per target, as `cargo test` runs it).
- `cargo xtask ci` on a person's machine runs as it does today.

## Non-goals

- Raising the 36-minute limit. It caps runner cost and flags anything that slows a shard down, so it stays where it is.
- Adding shards. Each one adds fixed setup cost.
- Linking a prebuilt DuckDB instead of compiling it. That changes packaging.
- Moving to cargo-nextest. It runs each test in its own process, which changes how the GPU targets share a device and how `region_gate`'s targets share the dataset staging file.

## Approaches considered

- **Stop the cache wipe and rebalance the shards.** Small: one step plus refreshed `target-seconds:` figures. But a truly cold run, and the DuckDB move's run, still compile on every shard. Balanced shards average about 27.5 minutes warm, so a cold run lands near 38. Rejected because it misses the first goal.
- **Build once, run in shards.** Chosen. One job compiles everything and hands the binaries to the shards, so a cold compile costs one job about 25 minutes and the shards never compile.
- **More shards, or a higher limit.** Rejected for the cost and canary reasons above.

## The design

**A `build-tests` job** runs first. It keeps today's setup: checkout, the pinned toolchain, rust-cache, the disk reclaim. Then a new xtask step, `build-tests`, does three things:

- Runs `cargo test --locked --workspace --no-run --message-format=json`.
- Builds the `conformance` binary and the workspace binaries that tests reach through `CARGO_BIN_EXE_*`.
- Writes a manifest. For each test executable, the manifest records its package, target name and kind, its path, and its package's root directory.

The job runs the doctests itself, because they cannot be prebuilt. It uploads the executables, the binaries and the manifest as one artifact. The artifact keeps the paths the files have under `target/`. Tests bake in absolute paths at compile time (`CARGO_BIN_EXE_*`, `CARGO_TARGET_TMPDIR`), and every hosted runner checks out to the same path, so those paths hold on the shards.

**The shards** `need` the build job. Each one checks out, installs the toolchain (for `cargo metadata` and xtask), fetches the pinned DuckDB CLI, and unpacks the artifact into `target/`. There is no rust-cache and no disk reclaim, since the shards don't compile. The `test-one-thread` and `test-default` steps keep their names and their packing. When the manifest is present, they run each selected executable directly, as `cargo test` would:

- the working directory set to the package root;
- `CARGO_MANIFEST_DIR` and the `CARGO_PKG_*` variables set;
- `--test-threads=1` for the one-thread group;
- every executable run even after one fails, and the step fails at the end if any did, matching `cargo test --no-fail-fast`. This needs checking: today's step stops at the first failing target. Whichever behaviour applies today is the one kept.

When the manifest is absent, as on a person's machine, they call `cargo test` exactly as now. `sibling-manifests` and `conformance` follow the same rule and run their prebuilt executables on the last default shard.

**The cache fix still goes in, on the build job.** Before the job's cache is saved, a step updates the modification times of `target/debug/{deps,build,.fingerprint}`, so that a later partial match keeps them. A lockfile bump then rebuilds only what changed.

**The `target-seconds:` figures** are refreshed from the first green run, so the shards are balanced on their test time alone.

**The `test` summary job** needs both the build job and the shards, and fails if either is red.

## Expected timings

These are from the logs of the 22 Oct runs, not yet measured:

| Job | Warm | Cold or DuckDB move |
|---|---|---|
| build-tests | ~14 min | ~25–28 min |
| each shard | ~3 min setup + its test share (~13 min when balanced) | the same |

Wall time on a warm run is about the build job plus the slowest shard, roughly 30 minutes. The slowest shard today takes 36.

## Verification, through CI only

The Mac's local build slot is taken, so every check runs in CI:

1. A warm run on the pull request. Every job is green, and the total of tests executed matches a recent `main` run's total, group by group.
2. A cold run, forced by a temporary commit that changes rust-cache's `prefix-key`, then reverted. Every job finishes under 36 minutes, and the build job's time is recorded.
3. The partial-match path: a temporary commit that bumps one small dependency in `Cargo.lock`, then reverted. The build job's log shows a partial match and a compile far below 470 crates.

## Left to the builder

- How the artifact is packed (tar or a directory upload) and its size. The executables should be small, because macOS keeps debug info in the object files, but this has to be checked on the first run.
- Whether xtask itself ships in the artifact, or compiles on each shard.
- The exact set of `CARGO_PKG_*` and other environment variables that `cargo test` sets for a test process, read from cargo's documentation.
