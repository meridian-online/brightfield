# Warm the build cache before the test shards

## Why

When a pull request changes `Cargo.lock`, the test shards start from a cache that is effectively empty, and three or four of the five are cancelled at the 36-minute job limit with no test failing. The cache does get restored, but from the previous lockfile's key, and Swatinem/rust-cache deletes every artifact in `target/` older than a week whenever the key is only a partial match. Warm runs never rebuild the dependencies, so nearly all of them are older than that. A run then recompiles about 470 crates in 14–15 minutes, against 12 crates in 4.4 minutes warm, and each of the five shards pays that cost.

The DuckDB 2.0 move on 29 Oct rewrites `Cargo.lock` and recompiles DuckDB, which alone is about 9 minutes of a cold compile, with the rest of the build waiting on it.

A second cost hides in warm runs. All five shards save to the same cache key, and the first shard to finish wins. The shard that runs the sibling-manifest check builds its own variant of DuckDB, because `cargo test -p brightfield-protocol --test sibling_manifests` resolves a narrower feature set. That variant is usually missing from the saved cache, so the shard compiles it again on warm runs too.

## Goals

- Every shard finishes inside the 36-minute limit on a run whose lockfile changed, including the DuckDB move, and on a run with no cache at all.
- A dependency is compiled once per lockfile, not once per shard.
- Warm runs add no more than about a minute.
- The suite and `cargo xtask ci` on a person's machine are unchanged.

## Non-goals

- Raising the 36-minute shard limit. It caps runner cost and flags anything that slows a shard down.
- Adding shards.
- Shipping test executables between jobs. The local build has 272 test targets with a median executable of 176 MB, which would be about 48 GB per run. This was the first design, and it was dropped for that reason.
- Linking a prebuilt DuckDB.

## Approaches considered

- **Stop the cache wipe and rebalance.** Small, but a truly cold run and the DuckDB move's run still compile on every shard, at about 38 minutes per balanced shard. Rejected.
- **Build once, ship the executables.** Rejected at about 48 GB per run.
- **Warm the cache in a job ahead of the shards.** Chosen.

## The design

A `warm-cache` job on `macos-15` runs before the shards, which `need` it.

- It installs the pinned toolchain and asks rust-cache, with `lookup-only: true`, `save-if: false` and `shared-key: test`, whether this lockfile's exact key exists. It downloads nothing. On a hit, every later step is skipped, and the job ends in about a minute.
- On a miss, it restores through rust-cache as the shards do (`shared-key: test`), reclaims disk, fetches nothing else, and runs a new xtask step, `warm-cache`. That step compiles what the shards will compile:
  - `cargo test --locked --workspace --no-run`
  - `cargo test --locked -p brightfield-protocol --test sibling_manifests --no-run`
  - `cargo build --locked --bin conformance`

  rust-cache's post step then saves the result under the exact key.
- The step is `only_in_ci`, because a local run's test steps build what they run.

The shards are unchanged. They restore an exact match and compile only the workspace crates, which rust-cache never caches, as warm shards do today. Once the warm job has saved the exact key, the shards' saves see an up-to-date cache and skip.

The `test` summary job needs both jobs and is red unless both are `success`.

The `target-seconds:` figures are refreshed from the first green run, because default-3 no longer compiles DuckDB on warm runs.

## Expected timings

These are estimated from the 22 Oct logs:

| Job | Warm | Lockfile changed or cold |
|---|---|---|
| warm-cache | ~1 min | ~30–35 min (restore, reclaim, three compiles, save) |
| each shard | as today (~23–33 min) | as warm |

A run whose lockfile changed takes about an hour end to end, because the two jobs run one after the other. A warm run takes about a minute longer than today.

## Verification, through CI only

1. A warm run on the pull request: the warm job reports a hit and skips, and every shard is green.
2. A cold run, forced by a temporary commit that changes `prefix-key` on every rust-cache step, then reverted. The warm job builds and saves. Every shard's log shows "full match: true" and fewer than 30 `Compiling` lines, and every job finishes under 36 minutes.
3. The test totals match a recent `main` run.

If the warm job itself goes over 36 minutes when cold, its ceiling is a separate question from the shards' and is raised only with its measured time written beside it.
