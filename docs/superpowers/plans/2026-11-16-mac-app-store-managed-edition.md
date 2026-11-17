# Brightfield's managed edition on the Mac App Store: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A sandboxed build of Brightfield, the managed edition, that App Review can accept. It opens, saves, reopens and runs the analyst's own Protocols through folder grants it keeps, reports refused access in the window, and opens restricted any Protocol with a step it may not run.

**Architecture:** One code base with an edition switch set at build time. Grants are security-scoped bookmarks kept in a grant store and held open while children use them. Children inherit them, as measured. The managed edition runs on Meridian's own build of DuckDB with its extensions compiled in, as a linked library and as the CLI arc runs. A separate packaging path signs the sandboxed bundle for the Store.

**Tech Stack:** Rust (the workspace's pinned toolchain), egui/eframe, objc2 and objc2-foundation for AppKit and Foundation calls, duckdb-rs 1.20000.0 (DuckDB 2.0.0) and, in the managed edition, Meridian's build of DuckDB 2.0.0 with extensions compiled in, `codesign`, `productbuild`.

**Spec:** `docs/superpowers/specs/2026-11-16-mac-app-store-managed-edition-design.md`. Read it before any task. The plan argues from it.

## Global Constraints

- Build slots: the signed download's sessions build first. Start no cargo build while three Brightfield sessions are building. Task 1 runs when a slot frees. Task 2 starts now: Hugh ruled on 2026-11-17 that it need not wait for the signed download, which waits on Apple's review of the developer enrolment rather than on code, and no Brightfield session was building; if the signing certificate arrives during it, the signing work takes the next free slot. Tasks 3 onwards wait for the signed download to ship.
- Task 1's findings revise the spec before Task 2 starts. Where a finding contradicts a task below, the revised spec wins, and the task is edited before it is built.
- One code base: every managed-edition difference is a question asked of `Edition` (Task 2). No `#[cfg]` on edition outside `edition.rs`.
- The full edition's behaviour does not change, except the refused-access banner and the headless exit (Task 6), which apply to both editions.
- Every grant is read-write.
- Store entitlements, verbatim. App: `com.apple.security.app-sandbox`, `com.apple.security.files.user-selected.read-write`, `com.apple.security.files.bookmarks.app-scope`, `com.apple.security.files.downloads.read-write`, `com.apple.security.network.client`. Helpers: exactly `com.apple.security.app-sandbox` and `com.apple.security.inherit`.
- Built-in DuckDB extensions, compiled in, with no `.duckdb_extension` file in the bundle: autocomplete, excel, sqlite, httpfs, ducklake, spatial, FineType. Automatic install and load are off, and any other `INSTALL` or `LOAD` is refused.
- The managed edition's DuckDB CLI is DuckDB's own shell from Meridian's build of DuckDB 2.0.0 (`d54cbbe3df17e8e5ce68312f483b4155836112aa`). `--version` prints exactly what the pinned release prints.
- Children are passed nothing: they inherit the grants (`docs/app-sandbox.md`). Brightfield keeps a grant open while a child uses it.
- The repo is public: no private planning identifiers in code, comments, commits, branch names or pull request text.
- The gate is `cargo xtask ci`. Before each commit run `cargo xtask ci --step fmt`, `--step clippy` and the tests the task touches, and after each commit `--step comment-citations`.

## Review Focus

1. **A granted folder renamed or moved in Finder.** The bookmark resolves to the new path, the Protocol follows, and the stale bookmark is re-saved. Test in Task 3.
2. **An external drive unmounted, then remounted.** Opening a Protocol on it while unmounted reports that it needs its drive, without crashing or forgetting the grant. Once remounted, it opens. Test in Task 3.
3. **A Protocol with sources in two folders, one of them granted.** Opening reports the ungranted folder and draws nothing half-read. Granting it completes the open. Test in Task 4.
4. **A folder deleted, or a grant made unreachable, during a run.** The run ends with a refusal naming the path, not an engine error alone. Test in Task 7.
5. **A Protocol folder in iCloud Drive or another synced folder.** A grant and its bookmark work there as on a local disk. Hand check in Task 14's proof run.

---

### Task 1: Re-measure today's build under App Sandbox (done, 2026-11-17)

Done: `scripts/sandbox-measure.sh`, `scripts/sandbox-measure/` and `docs/app-sandbox.md`, and the spec revised for its two findings with Hugh's rulings. The cases left for the first TestFlight build are in Task 14.

---

### Task 2: The edition switch

**Files:**
- Create: `crates/brightfield-shell/src/edition.rs`
- Modify: `crates/brightfield-shell/Cargo.toml` (feature `managed-edition`), `crates/brightfield-shell/src/lib.rs`
- Test: `crates/brightfield-shell/tests/edition.rs`

**Interfaces:**
- Produces:
  - `pub enum Edition { Full, Managed }`.
  - `pub fn current() -> Edition`: `Managed` exactly when built with `--features managed-edition`.
  - `pub enum Capability { CommandSteps, OutsidePrograms, ExtensionInstall, LauncherCommand, UngrantedPaths }`.
  - `impl Edition { pub fn allows(self, c: Capability) -> bool }`.
  - `pub fn full_edition_url() -> &'static str`, returning `https://meridian.online/brightfield`.
- The editions' names are working names, chosen elsewhere. Every name an analyst may read is defined in `edition.rs` (`Edition::name`), so a rename touches that file alone; the variant identifiers are internal.

- [ ] **Step 1:** Write the failing test `full_allows_everything_managed_allows_nothing_on_the_list`. It asserts `Edition::Full.allows(c)` for every `Capability`, and `!Edition::Managed.allows(c)` for every `Capability`.
- [ ] **Step 2:** Write the failing test `current_is_full_without_the_feature`, which asserts `edition::current() == Edition::Full` in a default build.
- [ ] **Step 3:** Run `cargo test -p brightfield-shell --test edition`. Expected: FAIL, because `edition` is unresolved.
- [ ] **Step 4:** Implement `edition.rs` and the feature.
- [ ] **Step 5:** Run `cargo test -p brightfield-shell --test edition`, then `cargo test -p brightfield-shell --features managed-edition --test edition -- --ignored current_is_managed_with_the_feature`. Expected: PASS for both.
- [ ] **Step 6:** Commit.

**Verification:** the two commands in Step 5. About 5 to 15 minutes incremental; a cold build of the feature is longer.

---

### Task 3: The grant store

**Files:**
- Create: `crates/brightfield-shell/src/grants.rs` (platform-neutral store)
- Create: `crates/brightfield-shell/src/mac_grants.rs` (Foundation calls; `unsafe` is allowed here, as in `mac_open.rs`)
- Modify: `crates/brightfield-shell/src/lib.rs`, `crates/brightfield-shell/src/main.rs`, to load the store at launch beside `trust::use_config_dir()`
- Test: `crates/brightfield-shell/tests/grants.rs`

**Interfaces:**
- Produces:
  - `pub struct Grants`.
  - `Grants::load(config_dir: &Path) -> Grants`: reads `grants.json`, resolves each bookmark, starts access, re-saves stale bookmarks and drops ones that no longer resolve.
  - `fn grant(&self, folder: &Path) -> Result<(), GrantError>`.
  - `fn covering(&self, path: &Path) -> Option<PathBuf>`, returning the granted folder at or above `path`.
  - `fn needs(&self, paths: &[PathBuf]) -> Vec<PathBuf>`, returning the folders still to grant for those paths.
  - `fn hold(&self, folders: &[PathBuf]) -> GrantHold`: keeps access to `folders` started until the returned guard drops, for a run or a join proposal.
  - `pub enum GrantError { Unresolvable(PathBuf), Unmounted(PathBuf), Io(String) }`.
  - `pub fn store() -> &'static Grants`, the process-wide store.
  - `mac_grants`: `make_bookmark(path: &Path) -> Result<Vec<u8>, String>`, `resolve_bookmark(data: &[u8]) -> Result<(PathBuf, bool /* stale */), String>`, `start_access(path: &Path) -> bool`.
- In the full edition `needs` always returns empty, so callers need no edition check.

- [ ] **Step 1:** Write the failing tests:
  - `a_granted_folder_covers_what_is_inside_it`: grant a temp dir, then `covering(dir/a/b.csv) == Some(dir)`.
  - `grants_survive_a_reload`: grant, `Grants::load` again, and `covering` still matches.
  - `a_moved_folder_is_followed_and_resaved` (Review Focus 1): grant, rename the dir, reload. `covering(new/x)` matches, and `grants.json` now names the new path.
  - `an_unmounted_volume_is_reported_not_dropped` (Review Focus 2): bookmark a path on a disk image attached with `hdiutil`, detach, reload. The result is `GrantError::Unmounted`, and the entry is still in `grants.json`. Mark it `#[ignore]` with the reason "attaches a disk image", and run it in CI's macOS job.
  - `needs_is_empty_in_the_full_edition`.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --features managed-edition --test grants`. Expected: FAIL.
- [ ] **Step 3:** Implement the store and the Foundation calls. Use `NSURL bookmarkDataWithOptions:` with `NSURLBookmarkCreationWithSecurityScope`, and `URLByResolvingBookmarkData:options:...bookmarkDataIsStale:` with `NSURLBookmarkResolutionWithSecurityScope`.
- [ ] **Step 4:** Run the tests in Step 2, then `cargo test -p brightfield-shell --features managed-edition --test grants -- --ignored`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands. About 5 to 15 minutes incremental.

---

### Task 4: Every way a path arrives asks for its folder

**Files:**
- Modify: `crates/brightfield-shell/src/mac_open.rs` (the open panel's results and the odoc event are saved as grants), `open_queue.rs` (`route` checks `grants::store().needs(...)` before opening, and a lone `arcform.yaml` asks for its folder), `window.rs` (the folder pickers near `:7668`, `:7683` and `:7693` save their result; drops; the typed Location field near `:2545`; restoring at launch near `:12719`; Recents), `sources_browser.rs` (`places()` lists granted folders and Downloads in the managed edition, plus a "Grant a folder…" entry), `own_files.rs` (adding a source outside every grant asks for its folder)
- Create: `crates/brightfield-shell/src/ask_grant.rs`, which opens the panel on a given folder with one sentence of why, and grants the result
- Test: `crates/brightfield-shell/tests/grant_on_open.rs`

**Interfaces:**
- Consumes: `grants::store()`, `Grants::needs`, `Grants::grant`, `Edition`.
- Produces:
  - `pub fn ask_for(folder: &Path, why: &str) -> Option<PathBuf>`, in `ask_grant.rs`. It opens the panel on `folder`, grants and returns the chosen folder, or `None` if the analyst cancels.
  - `pub enum OpenOutcome { Opened, NeedsGrant(Vec<PathBuf>) }`, returned by `open_queue::route`.

- [ ] **Step 1:** Write the failing tests. Each builds a fake store in the managed edition, with the panel replaced by a test double that records the folder it was opened on:
  - `a_lone_manifest_asks_for_its_folder`: routing `dir/arcform.yaml` with nothing granted gives `NeedsGrant([dir])`.
  - `a_protocol_with_an_ungranted_source_folder_asks_for_it` (Review Focus 3): the Protocol's dir is granted and its `.sources.json` names `/other/x.csv`, giving `NeedsGrant([/other])`. Nothing is drawn.
  - `a_lone_data_file_opens_with_its_own_grant`: routing a granted file gives `Opened`, makes its Protocol in the Protocols folder, and writes nothing in the file's folder.
  - `recents_without_a_grant_ask_rather_than_open_half_read`.
  - `the_location_field_accepts_only_a_granted_folder`.
  - `the_sources_browser_lists_granted_places_and_downloads`.
  - `the_full_edition_routes_as_before`: the existing `open_gestures` and `front_door_open` tests still pass unchanged.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --features managed-edition --test grant_on_open`. Expected: FAIL.
- [ ] **Step 3:** Implement. A `.duckdb` source opens read-only. If DuckDB reports that it needs a file beside the database, ask for the database's folder.
- [ ] **Step 4:** Run Step 2's test, then `cargo test -p brightfield-shell --test open_gestures --test front_door_open --test open_makes_a_protocol --test relaunch_restores`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands. About 10 to 20 minutes incremental.

---

### Task 5: The first launch asks for the Protocols folder

**Files:**
- Modify: `crates/brightfield-shell/src/protocols_dir.rs` (in the managed edition, `root()` is the granted folder, and with no grant it returns `None`), `window.rs` (the door asks once on the first launch with no grant), `preferences.rs` (the "Use as the Protocols folder" result is granted)
- Test: `crates/brightfield-shell/tests/first_launch_protocols_folder.rs`

**Interfaces:**
- Consumes: `ask_grant::ask_for`, `grants::store()`.
- Produces: `pub fn root() -> Option<PathBuf>` in the managed edition. The full edition keeps its present signature through a thin wrapper, so no full-edition caller changes.

- [ ] **Step 1:** Write the failing tests:
  - `the_first_managed_launch_asks_once_with_protocols_proposed`: the test double sees one panel, opened on Documents, with the name `Protocols` proposed. A second launch sees none.
  - `cancelling_leaves_the_door_open_and_asks_again_on_new_protocol`.
  - `the_full_edition_never_asks`.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --features managed-edition --test first_launch_protocols_folder`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run Step 2's test, then `cargo test -p brightfield-shell --test protocols_dir --test open_refused_protocols_folder --test new_protocol`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands. About 10 minutes incremental.

---

### Task 6: Refused access is reported in the window (both editions)

**Files:**
- Create: `crates/brightfield-shell/src/refusal.rs`
- Modify: `crates/brightfield-shell/src/window.rs` (a `say_refused` banner beside `say_folder_refused` near `:4597`, raised from the loaded graph's `ModelUnreadable` degrades and from run results; the Protocol is not marked complete while it stands), `crates/brightfield-shell/src/main.rs` and `protocol.rs` (the headless path exits 3 when `degrade_report()` holds an `unreadable:` entry)
- Test: `crates/brightfield-shell/tests/refused_access.rs`; update `tests/protocol_degrade_channel.rs`

**Interfaces:**
- Produces:
  - `pub struct Refusal { pub path: PathBuf, pub during: RefusedDuring }`.
  - `pub enum RefusedDuring { ModelRead, SourceRead, Write, Run }`.
  - `pub fn refusals_in_graph(graph: &ProtocolGraph) -> Vec<Refusal>`.
  - `pub fn refusal_in_engine_text(text: &str) -> Option<PathBuf>`, which recognises DuckDB's and the OS's "Permission denied" and "Operation not permitted" messages with their path.
  - The banner's action calls `ask_grant::ask_for(nearest_folder, ...)` in the managed edition. In the full edition it names System Settings → Privacy & Security → Files and Folders, as `protocols_dir.rs` already words it.
- Exit status 3 is "a model read was refused".

- [ ] **Step 1:** Write the failing tests:
  - `a_refused_model_raises_the_banner_naming_its_path`: with `models/` at mode `000`, the window holds one banner naming `models/`, and the Protocol is not marked complete.
  - `the_headless_path_exits_3_on_a_refused_model`: the `brightfield-shot` binary on the same Protocol exits 3. The stderr detector line is unchanged.
  - `an_absent_model_still_exits_0`.
  - `engine_text_with_operation_not_permitted_names_the_path`: `refusal_in_engine_text("IO Error: Cannot open file \"/a/b.csv\": Operation not permitted") == Some("/a/b.csv")`.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --test refused_access`. Expected: FAIL.
- [ ] **Step 3:** Implement. Update `protocol_degrade_channel.rs`'s exit assertions for the refused cases from 0 to 3.
- [ ] **Step 4:** Run `cargo test -p brightfield-shell --test refused_access --test protocol_degrade_channel --test load_diagnostics`, then `./scripts/compare-example-pngs.sh` and `./scripts/check-agent-protocols.sh`. Expected: PASS. No shipped example has an unreadable model, so neither script sees exit 3.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands. About 15 to 25 minutes, most of it the two scripts.

---

### Task 7: Runs and join proposals under the grants

**Files:**
- Modify: `crates/brightfield-shell/src/run.rs` (`arc_command` near `:528` takes a `GrantHold` over the Protocol folder, its sources' folders and the Protocols folder for the life of the run), `discovery.rs` (the same for dovetail), `refusal.rs` (run failures go through `refusal_in_engine_text`)
- Test: `crates/brightfield-shell/tests/child_grants.rs`

**Interfaces:**
- Consumes: `Grants::hold`, `refusal::refusal_in_engine_text`.
- Produces: the window raises a `Refusal { during: RefusedDuring::Run, .. }` for a refused run.

- [ ] **Step 1:** Write the failing tests:
  - `a_run_holds_its_folders_until_it_ends`: while the run is going, the store reports the Protocol folder and both source folders held; after it ends, none.
  - `a_run_refused_mid_way_reports_the_path` (Review Focus 4): delete the source folder, or set it to `000`, after the run starts. The run ends with a refusal naming it.
  - `dovetail_holds_the_folder_it_proposes_over`.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --features managed-edition --test child_grants`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run Step 2's test, then `cargo test -p brightfield-shell --test arc_child_way --test dovetail_child --test run_control --test protocol_run_start`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands, about 15 minutes incremental. That a sandboxed run reaches the grant is shown by `scripts/sandbox-measure.sh premises` and by Task 13's packaged baseline.

---

### Task 8: DuckDB built with its extensions

**Files:**
- Create: `scripts/build-duckdb.sh`. It fetches the DuckDB 2.0.0 source at `d54cbbe3df17e8e5ce68312f483b4155836112aa` and the pinned sources of each extension, with checksums. It builds, with DuckDB's own build and an extension configuration listing them, a static library and the CLI with autocomplete, excel, sqlite, httpfs, ducklake, spatial and FineType compiled in, into `target/duckdb-managed/`.
- Create: `scripts/duckdb-managed/extensions.cmake` (the extension configuration, each extension pinned to a commit), `scripts/build-duckdb-selftest.sh`
- Modify: `crates/brightfield-shell/Cargo.toml` and the workspace `Cargo.toml` (under `managed-edition`, link duckdb-rs to `target/duckdb-managed/` rather than its bundled DuckDB), `xtask` (step `duckdb-managed`), `deny.toml` if the extensions' licences need it
- Test: `scripts/build-duckdb-selftest.sh`, `crates/brightfield-engine/tests/managed_duckdb.rs`

**Interfaces:**
- Produces: `target/duckdb-managed/duckdb` (CLI) and `target/duckdb-managed/libduckdb_static.a` with headers, and the `managed-edition` feature linking them.

- [ ] **Step 1:** Write `build-duckdb-selftest.sh`. On the built CLI it asserts:
  - `--version` prints `v2.0.0 (Cyanoptera) d54cbbe3df`;
  - `select extension_name from duckdb_extensions() where loaded and installed` lists each of the seven with `install_mode` statically linked;
  - `install h3` fails, since autoinstall is off in this build;
  - `otool -L` shows no dynamic DuckDB library.
  Run it. Expected: FAIL.
- [ ] **Step 2:** Write the failing test `managed_duckdb_has_the_extensions_in_process` in `managed_duckdb.rs`. Under `managed-edition`, an in-memory connection reads an `.xlsx` with `read_xlsx` and a SQLite file with `sqlite_scan`, with no extension directory and the network denied.
- [ ] **Step 3:** Write `build-duckdb.sh` and the extension configuration, and wire the feature. FineType is the riskiest extension. If it will not build statically, stop and raise it with Hugh, because the spec's fallback (ship without in-process FineType typing first) needs his agreement.
- [ ] **Step 4:** Run `cargo xtask ci --step duckdb-managed` (it runs the build and its self-test), then `cargo test -p brightfield-engine --features managed-edition --test managed_duckdb`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands. A cold DuckDB build with these extensions is long: budget 30 to 90 minutes, and spatial is the heaviest. Incremental runs are minutes.

---

### Task 9: The parity gate

**Files:**
- Create: `scripts/check-duckdb-parity.sh`. It runs every shipped Protocol that runs offline, and the agent Protocols `check-agent-protocols.sh` runs, once with `ARC_DUCKDB_BIN` set to the pinned release CLI and once set to `target/duckdb-managed/duckdb`, and compares their outputs, run records and exit statuses. It then checks out arcform at the revision `Cargo.lock` pins and runs arc's engine and tokenizer tests against Meridian's CLI.
- Create: `scripts/check-duckdb-parity-selftest.sh`. It proves the comparison fails when one output differs by a byte.
- Modify: `xtask` (steps `duckdb-parity` and `duckdb-parity-selftest`), `.github/workflows/test.yml` (a job running them when `scripts/build-duckdb*`, `scripts/duckdb-managed/`, `packaging/store/`, `Cargo.lock` or `scripts/check-duckdb-parity*` change)

- [ ] **Step 1:** Write the self-test first. Run `./scripts/check-duckdb-parity-selftest.sh`. Expected: FAIL, because the script does not exist.
- [ ] **Step 2:** Write `check-duckdb-parity.sh` and wire the xtask steps.
- [ ] **Step 3:** Run `cargo xtask ci --step duckdb-parity-selftest`, then `cargo xtask ci --step duckdb-parity`. Expected: PASS, with every Protocol named and "identical" beside it. A Protocol needing `uv`, such as edgar_gleif's resolve step, is listed as "restricted in the managed edition" and checked by Task 10's test instead. A difference is a failure to fix, not to allow. If one cannot be fixed, stop and raise it with Hugh.
- [ ] **Step 4:** Run `cargo xtask ci --step workflow-coverage`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** `cargo xtask ci --step duckdb-parity`, about 20 to 30 minutes, most of it arc's tests.

---

### Task 10: Protocols beyond the edition open restricted

**Files:**
- Create: `crates/brightfield-shell/src/beyond_edition.rs`
- Modify: `crates/brightfield-shell/src/trust.rs` (`refuses_run` near `:83` also refuses when `beyond_edition` finds a step; trust does not lift it), `window.rs` (the restricted banner names the step and reason and links to `edition::full_edition_url()`; adding such a step in the managed edition says so as it is added)
- Test: `crates/brightfield-shell/tests/beyond_edition.rs`

**Interfaces:**
- Consumes: `Edition`, `Capability`, the parsed manifest type from `brightfield-protocol`.
- Produces:
  - `pub fn beyond_edition(manifest: &Manifest, edition: Edition) -> Option<BeyondEdition>`.
  - `pub struct BeyondEdition { pub step: String, pub reason: Reason }`.
  - `pub enum Reason { CommandStep(String), Precondition, OutsideProgram(String), Extension(String) }`.
  - `pub fn sentence(b: &BeyondEdition) -> String`, for example `Step clean runs uv, which this edition can't run.`
- Outside programs: arc operators that start a program, except `finetype` once Task 12 ships it. Extensions: any `INSTALL`/`LOAD` or declared extension asset outside the built-in set listed in Global Constraints.

- [ ] **Step 1:** Write the failing tests:
  - `a_command_step_is_beyond_the_managed_edition`: `command: uv run clean.py` gives `CommandStep("uv")`, with sentence `Step clean runs uv, which this edition can't run.`
  - `a_shell_precondition_is_beyond_it`.
  - `an_unbundled_extension_is_beyond_it`: `LOAD h3` gives `Extension("h3")`.
  - `spatial_and_excel_are_not`.
  - `the_full_edition_finds_nothing_beyond_it`.
  - `trust_does_not_lift_the_edition_restriction`.
  - `edgar_gleif_opens_restricted_naming_its_resolve_step`.
  - `every_other_shipped_protocol_opens_unrestricted`.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --features managed-edition --test beyond_edition`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run Step 2's test, then `cargo test -p brightfield-shell --test protocol_archive --test shipped_protocol_start`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands. About 10 to 15 minutes incremental.

---

### Task 11: Extensions built in, install and load off

**Files:**
- Modify: `crates/brightfield-engine/src/lib.rs` (in the managed edition, `autoinstall_known_extensions` and `autoload_known_extensions` are false, and `NetworkPolicy::Auto` never issues `INSTALL`), `crates/brightfield-engine/src/sources.rs` (the load SQL's `INSTALL excel; LOAD excel` and `INSTALL sqlite; LOAD sqlite` are left out in the managed edition, since the extensions are already loaded), `crates/brightfield-shell/src/semantic.rs` (FineType is not loaded from a file in the managed edition)
- Test: `crates/brightfield-engine/tests/built_in_extensions.rs`

**Interfaces:**
- Consumes: `Edition`. The engine crate takes it as a constructor argument, because it doesn't depend on the shell crate. Task 8's `managed-edition` DuckDB.

- [ ] **Step 1:** Write the failing tests:
  - `managed_reads_a_workbook_with_no_install_and_no_network`: run under `sandbox-exec` with the network denied, as `tests/front_door.rs` does. Reading an `.xlsx` succeeds.
  - `managed_refuses_install_and_load_of_anything_else`: `INSTALL h3` and `LOAD '/x.duckdb_extension'` error with the edition sentence.
  - `the_full_edition_installs_as_before`.
- [ ] **Step 2:** Run `cargo test -p brightfield-engine --features managed-edition --test built_in_extensions`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run Step 2's test, then `./scripts/check-bundled-extension-selftest.sh` for the full edition. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands. About 10 to 20 minutes.

---

### Task 12: FineType's CLI as a helper

**Files:**
- Modify: `scripts/finetype-pin.sh` or `scripts/fetch-finetype-bundle.sh` (fetch the pinned `finetype` CLI for `osx_arm64` with its checksum), `crates/brightfield-shell/src/run.rs` (set `FINETYPE_BIN` to `Contents/Helpers/finetype` when it is present, as `ARC_DUCKDB_BIN` is set near `:88`), `beyond_edition.rs` (`finetype` is not an outside program)
- Test: `crates/brightfield-shell/tests/finetype_helper.rs`

- [ ] **Step 1:** Write the failing tests:
  - `arc_is_pointed_at_the_bundled_finetype`: with a fake bundle layout, `arc_command` sets `FINETYPE_BIN` to it.
  - `a_finetype_operator_is_within_the_managed_edition`.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --features managed-edition --test finetype_helper`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run Step 2's test, then `./scripts/check-finetype-pin-selftest.sh`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands. About 10 minutes.

---

### Task 13: Store packaging and signing

**Files:**
- Create: `packaging/store/Brightfield.entitlements` and `packaging/store/Helper.entitlements` (verbatim from Global Constraints), `scripts/package-store.sh`, `scripts/package-store-selftest.sh`, `.github/workflows/store.yml`
- Modify: `packaging/README.md` (the no-entitlements decision stands for the full edition, and the Store path is described beside it), `xtask` (step `package-store-selftest`), `packaging/Info.plist` handling in the script (`ITSAppUsesNonExemptEncryption` false)

What `package-store.sh` does:
1. Builds release with `--features managed-edition`.
2. Lays out `Contents/MacOS/brightfield`, the helper copy for the arc and dovetail roles, `Contents/Helpers/duckdb` (Task 8's CLI), `Contents/Helpers/finetype`, and FineType's model. No `.duckdb_extension` file.
3. Signs inside out: the helpers with `Helper.entitlements`, then the app with `Brightfield.entitlements` and the embedded provisioning profile.
4. Runs `productbuild --sign "3rd Party Mac Developer Installer: …"`.
5. Without Store identities, signs ad-hoc for local checks.

- [ ] **Step 1:** Write `package-store-selftest.sh`. It asserts on an ad-hoc package: `codesign -d --entitlements -` on the app prints exactly the five app keys; on each helper it prints exactly `app-sandbox` and `inherit`; `Contents/Helpers/duckdb --version` prints the pinned version; `find` finds no `.duckdb_extension` in the bundle; `codesign --verify --strict --deep` passes on the app. Run it. Expected: FAIL.
- [ ] **Step 2:** Write `package-store.sh` and the entitlements files.
- [ ] **Step 3:** Run `cargo xtask ci --step package-store-selftest`. Expected: PASS.
- [ ] **Step 4:** Run `scripts/sandbox-measure.sh baseline` against the ad-hoc Store package's `brightfield-shot`. Expected: a grant on `arcform.yaml` alone now exits 3, naming the refused model, rather than drawing the degraded picture at exit 0. The cases that need a real panel are listed for the proof run.
- [ ] **Step 5:** Write `store.yml`. It triggers on `workflow_dispatch` and Store tags, imports the Mac App Distribution and Mac Installer Distribution identities and the profile from secrets, runs `package-store.sh`, and uploads the `.pkg` as an artifact. The upload to App Store Connect stays manual until Task 14. Run `cargo xtask ci --step workflow-coverage`. Expected: PASS.
- [ ] **Step 6:** Commit.

**Verification:** `cargo xtask ci --step package-store-selftest`, about 30 to 60 minutes for its release build; then the baseline run, about 10 minutes.

---

### Task 14: App Store Connect, submission and the proof run

Waits on the developer membership's move to the company. The steps marked **Hugh** are his.

- [ ] **Step 1, Hugh:** Accept the latest Apple Developer Program License Agreement as Account Holder, and add an Admin. Confirm in App Store Connect that a free app needs no Paid Apps Agreement.
- [ ] **Step 2:** Register the explicit App ID `online.meridian.brightfield`, or the separate identifier Task 1 decided on. Create the Mac App Distribution and Mac Installer Distribution certificates and the Mac App Store provisioning profile, and store them as `store.yml`'s secrets.
- [ ] **Step 3, Hugh:** Approve the listing: description, screenshots, age rating, privacy policy URL, App Privacy answers and support URL. Draft each for him from the README and the privacy policy.
- [ ] **Step 4:** Run `store.yml`, upload the `.pkg` with Transporter or `xcrun altool --upload-package`, and install it through TestFlight for Mac.
- [ ] **Step 5:** Do the proof run on a Mac, or a fresh macOS user account, that has never run Brightfield:
  - pick the Protocols folder;
  - open a Protocol folder, including one in iCloud Drive (Review Focus 5);
  - save a change;
  - quit and relaunch, and it reopens without asking;
  - set one model to mode `000` and reopen: the window reports the refused access;
  - run a Protocol;
  - open edgar_gleif: it opens restricted, naming its resolve step;
  - make Brightfield the default app for CSV, and confirm macOS takes it;
  - with the full edition also installed, check that each opens its own Recents and that Finder's Open With lists both.
  Record each in `docs/app-sandbox.md`.
- [ ] **Step 6, Hugh:** Submit for review.

**Verification:** the proof run's record in `docs/app-sandbox.md`. About an hour by hand, plus App Review's own time.

---

### Task 15: Fold the design into the docs

**Files:**
- Modify: `README.md` (a section on the managed edition: the editions table with working names; what it runs and what opens restricted; the Protocols folder asked once; grants and the Grant access banner; opening from Terminal with `open -a Brightfield`, and a shell function the analyst can add), `CHANGELOG.md`, `docs/app-sandbox.md` (the architecture: grants, children, Meridian's DuckDB build, the parity gate, with each of Hugh's rulings and its reason)
- Delete: `docs/superpowers/specs/2026-11-16-mac-app-store-managed-edition-design.md` and this plan

- [ ] **Step 1:** Write the docs.
- [ ] **Step 2:** Run `cargo xtask ci --step scale-claims --step measured-figures --step comment-citations`. Expected: PASS.
- [ ] **Step 3:** Delete the spec and plan, and commit.

**Verification:** Step 2's command, about 2 minutes.

## Order

- Task 1 is done.
- Task 2 comes first.
- Then Tasks 3, 8 and 12 in any order; they share nothing.
- Task 3 comes before Tasks 4, 5, 6 and 7.
- Task 8 comes before Tasks 9 and 11.
- Task 2 comes before Task 10. Task 12 comes before Task 10's `finetype` exception, so if Task 10 lands first, it lands without the exception and Task 12 adds it.
- Tasks 4 to 12 all come before Task 13.
- Task 13 comes before Task 14.
- Task 15 is last.
