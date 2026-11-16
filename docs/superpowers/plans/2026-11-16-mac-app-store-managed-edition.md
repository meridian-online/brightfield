# Brightfield's managed edition on the Mac App Store: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A sandboxed build of Brightfield, the managed edition, that App Review can accept. It opens, saves, reopens and runs the analyst's own Protocols through folder grants it keeps, reports refused access in the window, and opens restricted any Protocol with a step it may not run.

**Architecture:** One code base with an edition switch set at build time. Grants are security-scoped bookmarks kept in a grant store. Brightfield's arc and dovetail child roles, and a third role that is DuckDB's own shell compiled in, resolve the bookmarks passed in their environment. A separate packaging path signs the sandboxed bundle for the Store.

**Tech Stack:** Rust (the workspace's pinned toolchain), egui/eframe, objc2 and objc2-foundation for AppKit and Foundation calls, duckdb-rs 1.20000.0 bundled (DuckDB 2.0.0), DuckDB's shell source from the same release, `codesign`, `productbuild`.

**Spec:** `docs/superpowers/specs/2026-11-16-mac-app-store-managed-edition-design.md`. Read it before any task. The plan argues from it.

## Global Constraints

- Build slots: the signed download's sessions build first. Start no cargo build while three Brightfield sessions are building. Task 1 runs when a slot frees; Tasks 2 onwards wait for the signed download to ship.
- Task 1's findings revise the spec before Task 2 starts. Where a finding contradicts a task below, the revised spec wins, and the task is edited before it is built.
- One code base: every managed-edition difference is a question asked of `Edition` (Task 2). No `#[cfg]` on edition outside `edition.rs`.
- The full edition's behaviour does not change, except the refused-access banner and the headless exit (Task 6), which apply to both editions.
- Every grant is read-write.
- Store entitlements, verbatim. App: `com.apple.security.app-sandbox`, `com.apple.security.files.user-selected.read-write`, `com.apple.security.files.bookmarks.app-scope`, `com.apple.security.files.downloads.read-write`, `com.apple.security.network.client`. Helpers: exactly `com.apple.security.app-sandbox` and `com.apple.security.inherit`.
- Bundled DuckDB extensions: autocomplete (in the shell), excel, sqlite, httpfs, ducklake, spatial, FineType. Automatic install is off, and an `INSTALL` from the network is refused.
- The stand-in is DuckDB's own shell from DuckDB 2.0.0, linked to the library Brightfield links. `--version` prints exactly what the pinned CLI prints.
- The repo is public: no private planning identifiers in code, comments, commits, branch names or pull request text.
- The gate is `cargo xtask ci`. Before each commit run `cargo xtask ci --step fmt`, `--step clippy` and the tests the task touches, and after each commit `--step comment-citations`.

## Review Focus

1. **A granted folder renamed or moved in Finder.** The bookmark resolves to the new path, the Protocol follows, and the stale bookmark is re-saved. Test in Task 3.
2. **An external drive unmounted, then remounted.** Opening a Protocol on it while unmounted reports that it needs its drive, without crashing or forgetting the grant. Once remounted, it opens. Test in Task 3.
3. **A Protocol with sources in two folders, one of them granted.** Opening reports the ungranted folder and draws nothing half-read. Granting it completes the open. Test in Task 4.
4. **A folder deleted, or a grant made unreachable, during a run.** The run ends with a refusal naming the path, not an engine error alone. Test in Task 7.
5. **A Protocol folder in iCloud Drive or another synced folder.** A grant and its bookmark work there as on a local disk. Hand check in Task 14's proof run.

---

### Task 1: Re-measure today's build under App Sandbox

Runs in this planning issue when a build slot frees, before any build task. It builds `main` once in release.

**Files:**
- Create: `scripts/sandbox-measure.sh`: wraps a built binary in throwaway `.app` variants, ad-hoc signed with an entitlements plist per variant as the v0.1.2 measurement did, runs each case, and prints a table of case, stderr detector line, exit status and PNG sha256.
- Create: `scripts/sandbox-measure/entitlements/*.plist`: one per variant: `sandbox`, `dir-ro`, `dir-rw`, `file-only`, `helper-inherit`.
- Create: `scripts/sandbox-measure/bookmark-child/`: a small Rust or Swift probe that makes a security-scoped bookmark, passes it to a child signed with `inherit` (and on to a grandchild), and has each try to read a file in the folder.
- Create: `docs/app-sandbox.md`: the method, the results table and what each result means for the design.

- [ ] **Step 1:** Build release once: `cargo build --release -p brightfield-shell` (5 to 55 minutes, depending on the cache).
- [ ] **Step 2:** Run the baseline cases from the spec's re-measure section, numbers 1 to 7, through `scripts/sandbox-measure.sh baseline`. The `temporary-exception.files.absolute-path` entitlements stand in for panel grants, as in v0.1.2. Cases 1 to 3 and 8 need the real panel or Finder, so do those by hand on the ad-hoc bundle and record them in `docs/app-sandbox.md`.
- [ ] **Step 3:** Run the premise spikes, numbers 8 to 14, through `scripts/sandbox-measure.sh premises`.
- [ ] **Step 4:** Write `docs/app-sandbox.md` with every result. For each premise that failed, write the change to the spec beside it.
- [ ] **Step 5:** Revise the spec for each failed premise, and the tasks below that depend on it. Raise any change to what gets built with Hugh.
- [ ] **Step 6:** Commit `scripts/sandbox-measure*`, `docs/app-sandbox.md` and the spec edits.

**Verification:** `scripts/sandbox-measure.sh premises` prints a row per premise. Expected: rows 8 and 10 pass, which is the design's premise; any other result is recorded and acted on in Step 5. About 10 minutes on a built binary, plus about 30 minutes for the hand cases.

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
  - `fn env_value(&self, paths: &[PathBuf]) -> String`, the bookmarks covering `paths`, encoded for the child.
  - `pub const GRANTS_ENV: &str = "BRIGHTFIELD_GRANTS"`.
  - `pub fn resolve_from_env()`: in a child, resolves `GRANTS_ENV` and starts access.
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
- [ ] **Step 3:** Implement the store and the Foundation calls. Use `NSURL bookmarkDataWithOptions:` with `NSURLBookmarkCreationWithSecurityScope` for the store and with no options for `env_value`, which is Apple's way to hand a child a bookmark. Use `URLByResolvingBookmarkData:options:...bookmarkDataIsStale:`.
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

### Task 7: Children receive the grants

**Files:**
- Modify: `crates/brightfield-shell/src/run.rs` (`arc_command` near `:528` sets `GRANTS_ENV` from `grants::store().env_value(...)` over the Protocol folder, its sources' folders and the Protocols folder; `serve_as_runner_if_asked` calls `grants::resolve_from_env()` first), `discovery.rs` (the same, for dovetail and `serve_as_dovetail_if_asked`), `refusal.rs` (run failures go through `refusal_in_engine_text`)
- Test: `crates/brightfield-shell/tests/child_grants.rs`

**Interfaces:**
- Consumes: `Grants::env_value`, `grants::resolve_from_env`, `refusal::refusal_in_engine_text`.
- Produces: the window raises a `Refusal { during: RefusedDuring::Run, .. }` for a refused run.

- [ ] **Step 1:** Write the failing tests:
  - `the_arc_child_is_handed_the_protocol_and_source_folders`: with a two-folder Protocol, the command built by `arc_command` carries `GRANTS_ENV`, which decodes to both folders.
  - `a_run_refused_mid_way_reports_the_path` (Review Focus 4): delete the source folder, or set it to `000`, after the run starts. The run ends with a refusal naming it.
  - `dovetail_is_handed_the_folder_it_proposes_over`.
- [ ] **Step 2:** Run `cargo test -p brightfield-shell --features managed-edition --test child_grants`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run Step 2's test, then `cargo test -p brightfield-shell --test arc_child_way --test dovetail_child --test run_control --test protocol_run_start`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands, about 15 minutes incremental. Whether the sandboxed handover works is shown by `scripts/sandbox-measure.sh premises` row 8 on a managed build: about 10 minutes after a release build.

---

### Task 8: The DuckDB shell stand-in

**Files:**
- Create: `crates/brightfield-duckdb-shell/` with:
  - `Cargo.toml`;
  - `build.rs`, which compiles the vendored shell with `cc` against libduckdb-sys's bundled headers;
  - `src/lib.rs`, which exposes `pub fn run_shell(args: Vec<OsString>) -> i32`;
  - `vendor/shell/`, DuckDB 2.0.0's `tools/shell` sources and the third-party code they include, with DuckDB's MIT licence and a `VENDORED.md` naming the release commit `d54cbbe3df17e8e5ce68312f483b4155836112aa`.
- Modify: `crates/brightfield-shell/src/main.rs` (before every other role: when the executable's file name is `duckdb`, call `grants::resolve_from_env()`, then `exit(run_shell(args))`), `Cargo.toml` (workspace member), `deny.toml` if the licence check needs it
- Test: `crates/brightfield-duckdb-shell/tests/forms.rs`

**Interfaces:**
- Produces: `brightfield_duckdb_shell::run_shell(args: Vec<OsString>) -> i32`.
- The role is chosen by executable file name, not by an environment variable. arc's environment carries `BRIGHTFIELD_RUN_AS_ARC`, which would otherwise turn the shell back into arc.

- [ ] **Step 1:** Write the failing tests. Each runs a copy of the built binary named `duckdb` beside the pinned CLI from `target/duckdb-cli`, and compares stdout, stderr and exit status byte for byte:
  - `version_matches_the_pinned_cli`.
  - `list_mode_c_matches`: `-noheader -list -c "select 1, 'a'"`.
  - `file_then_dot_commands_match`: `<db> -f step.sql -c ".bail off" -c ".timer off" -c ".mode list" -c ".headers off" -c ".once '<tmp>'" -c "select extension_name, loaded from duckdb_extensions() order by 1"`, comparing the `.once` file too.
  - `json_tokenizer_on_stdin_matches`: `-no-init -json` with `select * from sql_tokenize('select 1')` on stdin.
  - `list_noheader_on_stdin_matches`.
  - `brightfield_engine_check_form_matches`: `-batch -noheader -csv -init <file> -c "select 42"`.
  - `a_failing_statement_exits_as_the_cli_does`.
  - `agent_mode_zero_is_honoured`: the same output with `DUCKDB_AGENT_MODE=0` set.
- [ ] **Step 2:** Run `cargo test -p brightfield-duckdb-shell --test forms`. Expected: FAIL.
- [ ] **Step 3:** Vendor the shell and write `build.rs`. If DuckDB 2.0.0's `sql_tokenize` lives outside the core library, add the extension's source the same way.
- [ ] **Step 4:** Run `cargo test -p brightfield-duckdb-shell --test forms`. Expected: PASS. A difference is a failure to fix, not to allow. If one cannot be fixed, stop and raise it with Hugh, with the spec's approach B as the alternative.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's command. About 20 to 40 minutes the first time, since it compiles the shell and needs `target/duckdb-cli`; about 5 minutes incremental.

---

### Task 9: The parity gate

**Files:**
- Create: `scripts/check-shell-parity.sh`. It checks out arcform at the revision `Cargo.lock` pins, and runs arc's engine and tokenizer tests with `ARC_DUCKDB_BIN` set first to the pinned CLI, then to the stand-in. It then runs every shipped Protocol that runs offline, and the agent Protocols `check-agent-protocols.sh` runs, under each, and compares their outputs, run records and exit statuses.
- Create: `scripts/check-shell-parity-selftest.sh`. It proves the comparison fails when one output differs by a byte.
- Modify: `xtask` (steps `shell-parity` and `shell-parity-selftest`), `.github/workflows/test.yml` (a job running `cargo xtask ci --step shell-parity` when paths under `crates/brightfield-duckdb-shell/`, `packaging/store/`, `Cargo.lock` or `scripts/check-shell-parity*` change)

- [ ] **Step 1:** Write the self-test first. Run `./scripts/check-shell-parity-selftest.sh`. Expected: FAIL, because the script does not exist.
- [ ] **Step 2:** Write `check-shell-parity.sh` and wire the xtask steps.
- [ ] **Step 3:** Run `cargo xtask ci --step shell-parity-selftest`, then `cargo xtask ci --step shell-parity`. Expected: PASS, with every Protocol named and "identical" beside it. A Protocol needing `uv`, such as edgar_gleif's resolve step, is listed as "restricted in the managed edition" and checked by Task 10's test instead.
- [ ] **Step 4:** Run `cargo xtask ci --step workflow-coverage`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** `cargo xtask ci --step shell-parity`, about 20 to 30 minutes, most of it arc's tests run twice.

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
- Outside programs: arc operators that start a program, except `finetype` once Task 12 ships it. Extensions: any `INSTALL`/`LOAD` or declared extension asset outside the bundled set listed in Global Constraints.

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

### Task 11: Bundled extensions, downloads off

**Files:**
- Modify: `crates/brightfield-engine/src/lib.rs` (in the managed edition, the extension directory is the bundle's, `autoinstall_known_extensions` is false, and `NetworkPolicy::Auto` never issues `INSTALL` from the network), `crates/brightfield-engine/src/sources.rs` (the load SQL's `INSTALL excel`/`INSTALL sqlite` become `LOAD` from the bundle in the managed edition), `scripts/package.sh`, or Task 13's `scripts/package-store.sh` (stage the extensions for DuckDB 2.0.0 on `osx_arm64`)
- Create: `scripts/fetch-duckdb-extensions.sh` (fetches the pinned extensions with checksums, at package time, not at run time)
- Test: `crates/brightfield-engine/tests/bundled_extensions.rs`

**Interfaces:**
- Consumes: `Edition`. The engine crate takes it as a constructor argument, because it doesn't depend on the shell crate.
- Produces: `pub fn bundled_extension_dir() -> Option<PathBuf>` in the engine, which is `Contents/Resources/duckdb-extensions` beside the executable.

- [ ] **Step 1:** Write the failing tests:
  - `managed_loads_excel_from_the_bundle_with_no_network`: run under `sandbox-exec` with the network denied, as `tests/front_door.rs` does, against a staged extension dir. Reading an `.xlsx` succeeds.
  - `managed_refuses_install_from_the_network`: `INSTALL h3` errors with the edition sentence.
  - `the_full_edition_installs_as_before`.
- [ ] **Step 2:** Run `cargo test -p brightfield-engine --features managed-edition --test bundled_extensions`. Expected: FAIL.
- [ ] **Step 3:** Implement. Whether extensions keep DuckDB's signature check after `codesign` comes from Task 1, premise 12. Follow the revised spec.
- [ ] **Step 4:** Run Step 2's test, then `./scripts/check-bundled-extension-selftest.sh`. Expected: PASS.
- [ ] **Step 5:** Commit.

**Verification:** Step 4's commands. About 10 to 20 minutes, including the extension fetch.

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
2. Lays out `Contents/MacOS/brightfield`, the helper copy, `Contents/Helpers/duckdb` (the helper under that name), `Contents/Helpers/finetype`, `Contents/Resources/duckdb-extensions/`, the FineType bundle and model, and no DuckDB CLI.
3. Signs inside out: extensions, then FineType's extension, `finetype`, the helpers with `Helper.entitlements`, then the app with `Brightfield.entitlements` and the embedded provisioning profile.
4. Runs `productbuild --sign "3rd Party Mac Developer Installer: …"`.
5. Without Store identities, signs ad-hoc for local checks.

- [ ] **Step 1:** Write `package-store-selftest.sh`. It asserts on an ad-hoc package: `codesign -d --entitlements -` on the app prints exactly the five app keys; on each helper it prints exactly `app-sandbox` and `inherit`; `Contents/Helpers/duckdb --version` prints the pinned version; no file named like the upstream CLI's signature is present. Run it. Expected: FAIL.
- [ ] **Step 2:** Write `package-store.sh` and the entitlements files.
- [ ] **Step 3:** Run `cargo xtask ci --step package-store-selftest`. Expected: PASS.
- [ ] **Step 4:** Run `scripts/sandbox-measure.sh baseline` against the ad-hoc Store package. Expected: every baseline case that failed in Task 1 now passes, except the cases that need a real panel, which are listed for the proof run.
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
  - open edgar_gleif: it opens restricted, naming its resolve step.
  Record each in `docs/app-sandbox.md`.
- [ ] **Step 6, Hugh:** Submit for review.

**Verification:** the proof run's record in `docs/app-sandbox.md`. About an hour by hand, plus App Review's own time.

---

### Task 15: Fold the design into the docs

**Files:**
- Modify: `README.md` (a section on the managed edition: the editions table with working names; what it runs and what opens restricted; the Protocols folder asked once; grants and the Grant access banner; opening from Terminal with `open -a Brightfield`, and a shell function the analyst can add), `CHANGELOG.md`, `docs/app-sandbox.md` (the architecture: grants, children, the stand-in, the parity gate, with each of Hugh's rulings and its reason)
- Delete: `docs/superpowers/specs/2026-11-16-mac-app-store-managed-edition-design.md` and this plan

- [ ] **Step 1:** Write the docs.
- [ ] **Step 2:** Run `cargo xtask ci --step scale-claims --step measured-figures --step comment-citations`. Expected: PASS.
- [ ] **Step 3:** Delete the spec and plan, and commit.

**Verification:** Step 2's command, about 2 minutes.

## Order

- Task 1 comes first, and its findings may change any task.
- Then Task 2.
- Then Tasks 3, 8, 11 and 12 in any order; they share nothing.
- Task 3 comes before Tasks 4, 5, 6 and 7.
- Task 8 comes before Task 9.
- Task 2 comes before Task 10. Task 12 comes before Task 10's `finetype` exception, so if Task 10 lands first, it lands without the exception and Task 12 adds it.
- Tasks 4 to 12 all come before Task 13.
- Task 13 comes before Task 14.
- Task 15 is last.
