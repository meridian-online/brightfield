# The signed download notarises with FineType inside it: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `Brightfield.app` signed with the Developer ID under the hardened runtime notarises, staples and passes `syspolicy_check distribution`. It still types columns offline through FineType, starts its DuckDB CLI, and loads downloaded extensions.

**Architecture:** In the app only, FineType's extension ships as two files: its Mach-O cut at the end of `__LINKEDIT` and signed on its own, and DuckDB's metadata trailer as data. `FinetypeBundle::open` verifies both against the manifest, joins them into a content-addressed copy under `~/Library/Caches`, and loads the copy. The app and the tarball's bare binary carry `com.apple.security.cs.disable-library-validation`, so extensions downloaded from DuckDB's repository still load under the hardened runtime.

**Tech Stack:** Rust (brightfield-engine, duckdb-rs), bash packaging scripts, codesign, notarytool, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-19-signed-download-notarises-design.md`

## Global Constraints

- The split layout applies to the `.app` only. The tarball keeps the single `finetype.duckdb_extension`.
- The split file names are `finetype.dylib` (Mach-O part) and `finetype.duckdb_extension.trailer`. The rejoined copy must be named `finetype.duckdb_extension`, because DuckDB derives the entry point (`finetype_init_c_api`) from the file name.
- Cache path: `$HOME/Library/Caches/online.meridian.brightfield/finetype/<sha256 of the joined bytes, lowercase hex>/finetype.duckdb_extension`.
- Exactly one entitlement: `com.apple.security.cs.disable-library-validation`, in `packaging/Brightfield.entitlements`.
- Ad-hoc and signed builds use the same layout.
- Public repo: no planning identifiers (Kata refs, card or decision ids) in code, comments, commit messages or PR text.
- Gate, from CLAUDE.md: before each commit run `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy` and the touched tests. After each commit run `cargo xtask ci --step comment-citations`. Never run `cargo fmt --all`.
- At most 6 cargo jobs and 6 test threads, and one cargo build at a time.
- After each task's commit, run `roborev wait <sha>` and fold its findings into the task's fix round.

## Review Focus

1. **A stale or tampered cache copy.** If the copy under Caches has bytes that don't match its directory key, it is rewritten before `LOAD`, never loaded. Owned by Task 1 (`a_cache_copy_whose_bytes_changed_is_rewritten_before_load`).
2. **Two Brightfield processes opening the bundle at once.** Both must end with a correct copy, and neither may load a half-written file. Owned by Task 1 (`concurrent_materialise_leaves_one_correct_copy`, two threads).
3. **An unwritable cache folder.** The error names the folder, and Brightfield runs without a type source rather than falling back somewhere else. Owned by Task 1 (`an_unwritable_cache_names_the_folder`).
4. **A split bundle missing one of its two parts.** It is refused with a message naming the missing file, and is not treated as "no bundle". Owned by Task 1 (`a_split_bundle_missing_its_trailer_is_refused_by_name`) and Task 2 (selftest case).
5. **`HOME` unset or relative.** Cache resolution refuses with a clear error rather than writing into the working directory. Owned by Task 1 (`cache_root_refuses_a_missing_or_relative_home`).

---

### Task 1: The engine loads a split bundle through a cache copy

**Files:**
- Create: `crates/brightfield-engine/src/semantic/split.rs`, if `semantic.rs` has no submodule directory. Otherwise follow the crate's existing layout: the split and join logic and cache handling go in their own module, which `semantic.rs` calls.
- Modify: `crates/brightfield-engine/src/semantic.rs` (`FinetypeBundle::open` around lines 534–597, `bundle_beside` at 1131, constants beside `EXTENSION_FILE` at 479).
- Test: unit tests in the new module and in `semantic.rs`'s `mod tests`.

**Interfaces:**
- Produces:
  - `pub const SPLIT_LIBRARY_FILE: &str = "finetype.dylib";`
  - `pub const SPLIT_TRAILER_FILE: &str = "finetype.duckdb_extension.trailer";`
  - `pub fn cache_root() -> Result<PathBuf, String>`: `$HOME/Library/Caches/online.meridian.brightfield/finetype`. An error if `HOME` is unset, empty or not absolute.
  - `pub fn materialise_split(dir: &Path, cache_root: &Path) -> Result<PathBuf, String>`: reads the two parts, joins them, and returns the path of a copy named `EXTENSION_FILE` under `<cache_root>/<sha256>/`. It reuses an existing copy only if its hash matches, and otherwise writes a temp file in that directory and renames it.
  - `bundle_beside` returns a directory holding either `EXTENSION_FILE` or both split files.
- `FinetypeBundle::open` keeps its signature.

- [ ] **Step 1: Write the failing tests.** Use synthesised bytes, a fake Mach-O body plus the existing `trailer(...)` helper, under a per-test temp dir:
  - `split_parts_join_back_byte_for_byte`: `materialise_split` returns a path whose bytes equal `body ++ trailer`, whose file name is `finetype.duckdb_extension`, and whose parent directory name is the lowercase sha256 of those bytes.
  - `a_second_materialise_reuses_the_copy`: the second call returns the same path, and the file's mtime is unchanged.
  - `a_cache_copy_whose_bytes_changed_is_rewritten_before_load`: overwrite the copy with junk. The next call returns the same path, with the correct bytes again.
  - `concurrent_materialise_leaves_one_correct_copy`: two threads call it at once, both return `Ok`, and the file has the correct bytes.
  - `an_unwritable_cache_names_the_folder`: make the cache root mode `0o500`. The error string contains the cache root's path.
  - `cache_root_refuses_a_missing_or_relative_home`: test through a helper `cache_root_from(home: Option<&OsStr>)`, so the tests don't mutate the process environment. `None`, `""` and `"rel/home"` are errors. `"/h"` gives `/h/Library/Caches/online.meridian.brightfield/finetype`.
  - `a_split_bundle_missing_its_trailer_is_refused_by_name`: `FinetypeBundle::open` on a dir holding only `finetype.dylib` errors with a message containing `finetype.duckdb_extension.trailer`.
  - `bundle_beside_finds_a_split_bundle`: under the `../Resources/finetype` layout holding only the two split files, it returns `Some`.
  - `the_stamp_is_read_from_the_trailer_file_in_a_split_bundle`: `open` on a split bundle with a wrong-platform trailer fails at the ABI check (message contains the platform) before any `LOAD`.
  - `a_split_part_that_fails_the_manifest_is_refused_before_load`: a manifest recording a different hash for `finetype.dylib` makes `open` fail with "is not the file that was packaged".
- [ ] **Step 2: Run them and see them fail.** `cargo test -p brightfield-engine semantic -- --test-threads=6`. They fail to compile, or fail on the missing functions.
- [ ] **Step 3: Implement.** In `open`:
  - Detect the layout. The joined file wins if present.
  - For a split bundle, read the stamp with `read_stamp(&trailer_bytes)`, which works because `read_stamp` reads the last 512 bytes.
  - Run `check_abi`, `verify_against_manifest` and the model and catalogue checks unchanged.
  - Only then call `materialise_split(dir, &cache_root()?)` and `LOAD` the returned path.
  - Hash with the `sha2` crate if it is already in the workspace graph; otherwise hash through DuckDB `sha256(read_blob(?))` as `verify_against_manifest` does. Don't add a dependency for this.
  - Error strings go through the existing `at(...)` so they name the bundle dir.
- [ ] **Step 4: Run the tests and see them pass.** Same command. All new and existing `semantic` tests pass.
- [ ] **Step 5: Run the gate checks and commit.** `cargo xtask ci --step fmt`, `cargo xtask ci --step clippy`, then `git commit -m "Load a FineType bundle shipped as a signed library and its trailer"`. Then `cargo xtask ci --step comment-citations` and `roborev wait <sha>`.

### Task 2: Packaging ships the split layout in the app, with the entitlement

**Files:**
- Create: `scripts/split-extension.sh`, usage `scripts/split-extension.sh EXT DEST_DIR`. It writes `DEST_DIR/finetype.dylib` and `DEST_DIR/finetype.duckdb_extension.trailer`. It refuses a trailer shorter than 512 bytes, refuses a file that is not a dylib, and checks that `cat part trailer` equals the input.
- Create: `packaging/Brightfield.entitlements`, a plist with `com.apple.security.cs.disable-library-validation` set to true and nothing else.
- Modify: `scripts/set-extension-rpath-id.sh`. Its `__LINKEDIT`-end awk moves into a function both scripts use. Put it in `scripts/lib/extension-macho-end.sh` and `source` it, so the offset is read in one place.
- Modify: `scripts/package.sh`:
  - `stage_finetype` takes a second argument, `split` or `joined`. The app passes `split`.
  - After the rename, `split` runs `split-extension.sh`, signs `finetype.dylib`, checks it with `codesign --verify --strict`, and removes the joined file before the manifest is written. Signing uses `--force --options runtime --timestamp --sign "$SIGN_IDENTITY"` when an identity is set, and `--force --sign -` otherwise.
  - Both `codesign` calls on `$APP`, and the identity signing of `$STAGE/brightfield`, gain `--entitlements packaging/Brightfield.entitlements`.
  - Update the header's SIGNING section to say what is now signed and why the entitlement exists.
- Modify: `scripts/check-bundled-extension.sh`. Accept either layout. In a split bundle, read the trailer from the trailer file and require `codesign --verify --strict` on `finetype.dylib`. A bundle holding one split part and not the other fails, naming the missing file.
- Modify: `scripts/check-bundled-extension-selftest.sh`. Add cases: a valid split bundle passes, a split bundle missing the trailer fails with its name, and a split bundle whose part fails strict verification fails. Build the fixtures with the selftest's existing fixture helpers.
- Modify: `packaging/README.md`, with one paragraph on the entitlements file and its reason.

**Interfaces:**
- Consumes: the file names fixed in Global Constraints.
- Produces: an app whose `Contents/Resources/finetype/` holds `finetype.dylib`, `finetype.duckdb_extension.trailer`, `model/`, `taxonomy-schemas.json` and `bundle-manifest.sha256`, with no `finetype.duckdb_extension`.

- [ ] **Step 1: Add the failing selftest cases.** Run `scripts/check-bundled-extension-selftest.sh`. It reports the new split cases failing.
- [ ] **Step 2: Implement `split-extension.sh` and the shared offset function, and extend `check-bundled-extension.sh`.** The selftest then passes.
- [ ] **Step 3: Change `package.sh` and add the entitlements file.** Then run an ad-hoc package locally. The FineType bundle from `scripts/fetch-finetype-bundle.sh aarch64-apple-darwin <dir>` is passed as `BRIGHTFIELD_FINETYPE_BUNDLE`, and the pinned CLI dir as `BRIGHTFIELD_DUCKDB_CLI`; read package.sh's header for its exact inputs. Run `scripts/package.sh v<crate>-local aarch64-apple-darwin`. Expected: it finishes, and `codesign -d --entitlements - <app>` shows the entitlement.
- [ ] **Step 4: Verify the built artifacts.**
  - `scripts/check-artifact-type-source.sh dist/<name>.dmg aarch64-apple-darwin` passes (it loads, models and labels a column through the split path).
  - `scripts/verify-airgapped.sh dist/<name>.dmg` passes.
  - `scripts/verify-airgapped.sh dist/<name>.tar.gz` passes.
- [ ] **Step 5: Run the gate checks and commit.** Run the `bundled-extension-selftest` step. Commit: `git commit -m "Ship FineType in the app as a signed library and its trailer, with library validation off"`. Then run `comment-citations` and `roborev wait`.

### Task 3: The rehearsal and release paths say what is now true

**Files:**
- Modify: `.github/workflows/signing-rehearsal.yml`:
  - The "Both signatures carry…" step also checks `Contents/Resources/finetype/finetype.dylib`, which must carry the rehearsal authority, the runtime flag and a timestamp.
  - It also checks `Contents/Helpers/duckdb`, which must carry `Authority=Developer ID Application: Stichting DuckDB Foundation (7NCTQWA3HA)` and a timestamp.
  - It also checks that `codesign -d --entitlements - <app>` names `com.apple.security.cs.disable-library-validation`.
  - Rewrite the "A report, not a gate" comment. Keep the table step.
- Modify: `scripts/release-signing.sh`. Rewrite the "WHAT THIS DOES NOT YET MAKE PASS" section to say what the split layout makes pass, and that a signed tag now notarises the bundle.
- Modify: `.github/workflows/release.yml`. Correct any comment that says a signed tag stops at notarisation.
- Modify the xtask workflow lint if one asserts on these comments or steps. Run `cargo xtask ci --list` and the workflows step to find out.

- [ ] **Step 1: Make the workflow and comment edits.**
- [ ] **Step 2: Check the workflow files.** Run `cargo xtask ci --step <the workflows/lint step named by --list>` (it must pass) and `actionlint .github/workflows/signing-rehearsal.yml` if `actionlint` is installed.
- [ ] **Step 3: Commit.** `git commit -m "Check the signed bundle's nested code instead of only reporting it"`. Then run `comment-citations` and `roborev wait`. The rehearsal itself runs when the PR is pushed. It must still fail at `notarytool submit` with the 401, and now pass the new assertions.

### Task 4: A local signed build needs only a 1Password sign-in

**Files:**
- Create: `scripts/local-signing.sh`, with commands `setup` and `cleanup`:
  - `setup` sets `RUNNER_TEMP` and `GITHUB_ENV` to a folder under `${TMPDIR}` that it creates, and reads the five variables with `op read` from 1Password's Meridian vault. The item and file names are in the spec's last section: the `.p12` file and its `password` field; the `.p8` file, `Key ID` and `Issuer ID`.
  - It then runs `scripts/release-signing.sh setup` and prints the four `BRIGHTFIELD_*` lines, ready for `eval` or sourcing.
  - `cleanup` runs `release-signing.sh cleanup` with the same `RUNNER_TEMP` and removes the folder.
  - It never echoes a secret. It refuses with "run `op signin` first" when `op read` fails.
  - Refer to the 1Password items by title, not by item id.
- Modify: the `package.sh` header SIGNING section, with one line pointing to `local-signing.sh` for local signed builds.

- [ ] **Step 1: Write the script.** Run `scripts/local-signing.sh setup` after Hugh's `op signin`. Expected: release-signing.sh prints the Developer ID identity line and "Credentials validated".
- [ ] **Step 2: Run `scripts/local-signing.sh cleanup`.** Expected: `security list-keychains -d user` no longer lists `brightfield-signing`.
- [ ] **Step 3: Commit.** `git commit -m "Set up local Developer ID signing from 1Password with one command"`, then run `roborev wait`.

### Task 5: The signed, notarised .dmg, checked as a stranger would get it

This task produces evidence, not code. It needs Hugh's `op signin`. Flag the issue `needs-human` before asking.

- [ ] **Step 1: Build the signed app.** Run `scripts/local-signing.sh setup`, source its output, then run `scripts/package.sh v<crate>-local aarch64-apple-darwin` with the FineType bundle and CLI as in Task 2. Expected in the log: `== notarize:`, notarytool `status: Accepted`, "The staple and validate action worked!", and `syspolicy_check distribution` exit 0. Keep the notarytool submission id. Fetch `notarytool log <id>` and confirm `issues` is null.
- [ ] **Step 2: Check that the app actually runs.** From the mounted `.dmg`:
  - `Brightfield.app/Contents/MacOS/brightfield --check-type-source` labels a column.
  - `--check-engine` reports a match.
  - A query with `INSTALL spatial; LOAD spatial;`, run once online, then loads offline. That is the downloaded-extension leg.
  - Also run `scripts/verify-airgapped.sh` and `scripts/check-artifact-type-source.sh` on the signed `.dmg`.
  - Repeat the type-source and spatial checks on the signed tarball's bare `brightfield`, unpacked. That is what Homebrew and the install command deliver: a Developer-ID-signed, hardened-runtime binary loading the ad-hoc joined FineType file. It relies on the same entitlement.
- [ ] **Step 3: Prepare the second-Mac check.** Copy the `.dmg` and its `.sha256` to a stable path outside `dist/`, and run `xattr -w com.apple.quarantine "0081;$(printf %x $(date +%s));Safari;" <dmg>`. Report the path, the checksum, and the steps for Hugh: a separate macOS user account, network off, open the `.dmg`, drag the app to Applications, launch it, and open an example with typed columns.
- [ ] **Step 4: Run `scripts/local-signing.sh cleanup`.** Record the submission id and the results as a comment on the issue.

### Task 6: Fold and finish

- [ ] **Step 1: Fold the spec into the docs.** Put the outcome into the repo's own docs on this branch. That means `package.sh`'s header, already done in Tasks 2 and 4, and a short section in `packaging/README.md`. Cover the layout, why the trailer is split off, the entitlement and its reason, and Hugh's rulings: the editions may reach FineType by different routes, and Studio's download ships it split.
- [ ] **Step 2: Remove the planning documents.** Delete `docs/superpowers/specs/2026-11-19-signed-download-notarises-design.md` and this plan, and commit.
- [ ] **Step 3: Run the full gate.** Run `cargo xtask ci` once, push, and open the PR. The description names the entitlement and its reason, gives test totals from CI, gives the notarisation evidence (submission id, staple, syspolicy) and lists the rulings.
- [ ] **Step 4: Review and close.** Run `roborev review --branch --wait`, fix or answer each finding at Medium and above, and close the review. Wait on `gh pr checks <pr> --watch`. Link the PR to the issue in Forge as methods' loop.md describes. Report the PR as ready, and stop.
