# The signed download passes notarisation with FineType inside it

## Intent

Brightfield Studio's macOS download, `Brightfield.app` inside a `.dmg`, is signed with Meridian's Developer ID under the hardened runtime, accepted by Apple's notary service, stapled, and passed by `syspolicy_check distribution`. A run from the signed app still loads FineType and types columns offline, starts the bundled DuckDB CLI, and loads the DuckDB extensions Studio installs at run time (spatial, excel, sqlite and the rest).

Why now: the release signing credentials are set in the repository, so the next version tag signs and notarises. Until this lands that tag stops at notarisation and publishes nothing, because Apple's notary service refuses the ad-hoc FineType extension inside the bundle. The first signed release is due on 28 November. A locally built, signed, notarised and stapled `.dmg` from this branch is the input to an offline check on a second Mac on 23 November.

## What is true today

- `scripts/package.sh` signs the tarball's bare Mach-O and the outer app bundle with the identity and the hardened runtime, then notarises and staples. It passes no entitlements, and it leaves nested code as it was staged.
- The DuckDB CLI at `Contents/Helpers/duckdb` carries DuckDB Foundation's Developer ID, a secure timestamp, the runtime flag and the `com.apple.security.cs.disable-library-validation` entitlement. Notarisation accepts nested code signed by another team's Developer ID, so it is expected to pass as staged. This is checked by the notarisation run, not assumed.
- The FineType extension at `Contents/Resources/finetype/finetype.duckdb_extension` is ad-hoc signed. `scripts/set-extension-rpath-id.sh` re-signs it after cutting off DuckDB's metadata trailer, renaming it, and putting the trailer back. Apple's `codesign` cannot sign the file as a whole. DuckDB appends its trailer after the Mach-O image, and anything after a code signature fails strict validation, whoever signs it. Measured on macOS 27.2: a library with bytes appended after its signature still loads.
- The in-process engine runs `INSTALL x; LOAD x;` for spatial, excel, sqlite and other extensions downloaded from DuckDB's repository. Those files are not signed by Meridian's team. Under the hardened runtime, macOS refuses to map a library signed by another team unless the process carries `disable-library-validation`. Today the signed app would carry no such entitlement, so those loads would fail.
- Measured on 2026-11-20 on Apple silicon, macOS 27.2.1, FineType v0.6.67, with the pinned DuckDB CLI re-signed ad-hoc under the hardened runtime as the host:
  - The extension cut at the end of `__LINKEDIT` passes `codesign --verify --strict` on its own. Its trailer is 534 bytes.
  - Rejoined with its trailer, it loads and classifies email addresses in a host that carries `disable-library-validation`.
  - Without that entitlement, dyld refuses it: "mapping process and mapped file (non-platform) have different Team IDs".
  - DuckDB derives the entry point from the file name (`finetype_init_c_api`), so the rejoined copy must be named `finetype.duckdb_extension`.
- Measured on 2026-11-20 with the real Developer ID and notary key, through `scripts/release-signing.sh setup` run locally, on a throwaway app shaped like Brightfield.app. Its main executable was signed with the hardened runtime and the entitlement. Its `Contents/Helpers/duckdb` was as staged. Its `Contents/Resources/finetype/` held the cut part, signed with the Developer ID and a timestamp, and the trailer as a data file.
  - The notary service accepted it with no issues. It stapled and passed `syspolicy_check distribution`.
  - The part, rejoined with its trailer, loads and types email addresses in a DuckDB CLI re-signed with the same Developer ID under the hardened runtime. It loads with or without the entitlement, because the Team IDs match.
  - The same host without the entitlement refuses an ad-hoc extension ("different Team IDs"). That is what an extension downloaded from DuckDB's repository is to it, so the entitlement is needed for Studio's downloaded extensions.
  - Still unmeasured: Brightfield itself as the host. The build's first signed run checks that.
- `FinetypeBundle::open` (crates/brightfield-engine/src/semantic.rs) reads the trailer for the ABI check, verifies every file against `bundle-manifest.sha256`, checks the model and catalogue, and then runs `LOAD '<path>'`. `bundle_beside` finds the bundle by looking for `finetype.duckdb_extension`.

## Goals

1. The app bundle notarises, staples and passes `syspolicy_check distribution`, signed with the Developer ID.
2. The signed app loads FineType from the bundle, with no network, from first launch, and passes the existing canary (three email addresses classified as email).
3. The signed app starts the bundled DuckDB CLI (the Run control, `--check-engine`).
4. The signed app, and the signed bare binary in the tarball, load extensions downloaded at run time.
5. Every entitlement the signed app carries is named in the pull request with its reason.
6. A locally built `.dmg` from this branch is signed, notarised and stapled, for the second-Mac check.

## Non-goals

- The App Store edition. It compiles its extensions into DuckDB (the managed DuckDB build) and does not change here.
- Notarising the tarball. `stapler` cannot staple a `.tar.gz`. The tarball's FineType file stays as it is today, and Homebrew and the install command are unchanged.
- Switching Studio to the managed DuckDB build, or installing FineType from DuckDB's community registry. Both were considered and not taken (see Approaches).
- Signing or stapling the `.dmg` itself. The ticket rides on the bundle inside it, as today.

## Anti-goals

- A signed app that passes notarisation but cannot type columns offline. The second-Mac check and the airgapped release gate would both miss this if they only checked that the app launches.
- A FineType copy whose bytes are not the ones packaged. The manifest check must cover what is actually loaded.
- An ad-hoc build whose layout differs from a signed build's. CI runs ad-hoc on every pull request, so the ad-hoc app must exercise the same split-and-rejoin path as the signed one, or CI would test a layout that never ships.

## Constraints

- Release date 28 November, second-Mac check 23 November.
- macOS is the validated target. The split layout applies only to the `.app`.
- The airgapped gate (`scripts/verify-airgapped.sh`) runs the `.dmg` with `HOME` redirected and the network denied, and it must stay green.
- `cargo xtask ci` is the gate. `scripts/package-finetype-selftest.sh` and `scripts/check-artifact-type-source.sh` already cover the bundle and must cover the new layout.

## Approaches

1. **Ship FineType in two parts and rejoin them at load (chosen).** The app carries the extension's Mach-O without its trailer, signed with the Developer ID (or ad-hoc in an unsigned build), plus the trailer as a data file. On first use, Brightfield joins the two into a copy in its cache folder and loads that copy. It keeps offline typing from first launch and the pinned FineType version, leaves the tarball and Homebrew unchanged, and fits the timeline. The cost is that the app writes a library to disk before loading it. Studio already does this with every extension it downloads.
2. **Install FineType from DuckDB's community registry on demand**, as spatial is installed. This is the simplest change: the app ships no FineType code. The costs are that a Mac that has never been online gets no semantic types, which fails the offline check and the airgapped gate, and that Studio would run whatever version the registry carries.
3. **The managed DuckDB build with FineType compiled in**, as the App Store edition does. The costs are an engine change nine days before release, an hour-long cold build, and an app that no longer matches the tarball.

Hugh chose route 1 on 2026-11-19, after ruling that the editions may take different routes for FineType where that makes a channel's distribution simpler. The App Store edition compiles it in, and Studio's signed download ships it split.

## The design

### Layout in the app

`Contents/Resources/finetype/` holds:

| File | What it is |
| --- | --- |
| `finetype.dylib` | the extension's Mach-O, from offset 0 to the end of `__LINKEDIT`, with the `@rpath/` install name, signed |
| `finetype.duckdb_extension.trailer` | DuckDB's metadata trailer, the bytes after `__LINKEDIT`, unchanged |
| `model/`, `taxonomy-schemas.json` | as today |
| `bundle-manifest.sha256` | hashes of every file above, as today |

The file names are the builder's call. The suffix should keep the Mach-O part recognisable as a library and the trailer recognisable as data.

The tarball keeps the single `finetype.duckdb_extension`.

### Packaging (`scripts/package.sh`)

For the app only, `stage_finetype` gains a split step after `set-extension-rpath-id.sh` and before the manifest is written:

1. Cut the file at the end of `__LINKEDIT`, using the same reading `set-extension-rpath-id.sh` already does. Refuse a trailer shorter than 512 bytes, as it does.
2. Sign the Mach-O part. With `SIGN_IDENTITY` set: `codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY"`. Otherwise sign it ad-hoc. It is already ad-hoc signed from the rename, so this step is a no-op apart from proving the part verifies on its own: `codesign --verify --strict` must pass on the part in both cases.
3. Remove the joined file, and write the manifest over the parts.

The shared cutting logic moves into one helper used by both scripts, so the `__LINKEDIT` offset is read in one place.

Entitlements: a new `packaging/Brightfield.entitlements` carries exactly `com.apple.security.cs.disable-library-validation`. It is passed with `--entitlements` when signing the outer app and when signing the tarball's bare Mach-O with an identity. The ad-hoc app signature carries it too, so ad-hoc and signed builds run under the same rules apart from the runtime flag. The reason given in the pull request: Studio loads DuckDB extensions it downloads at run time (spatial, excel, sqlite and others), which are not signed by Meridian's team, and DuckDB's own CLI carries the same entitlement for the same reason. FineType itself does not need it in the signed app, because its Mach-O part is signed by Meridian's team. Any further entitlement that the hardened-runtime run turns out to need is added only with a measured failure that shows why.

### Loading (`crates/brightfield-engine/src/semantic.rs`)

`FinetypeBundle::open` accepts either layout:

- **Joined** (`finetype.duckdb_extension` present): unchanged.
- **Split** (`finetype.dylib` and the trailer present): the stamp is read from the trailer file, and `check_abi` and `verify_against_manifest` run over the bundle directory as today, before anything is loaded. Then the two parts are joined into a cache copy, and `LOAD` runs on the copy's path.

The cache copy:

- Location: `$HOME/Library/Caches/online.meridian.brightfield/finetype/<sha256 of the joined bytes>/finetype.duckdb_extension`. Keying the directory by content means that two Brightfield versions or two bundles never share a copy by accident, and that the copy needs no separate invalidation.
- Written atomically: write to a temporary file in the same directory, then rename. Two Brightfield processes starting at once both end up with a correct file.
- Reused when present, but only after its bytes are hashed and match the directory's key. A copy that does not match is rewritten.
- Integrity: the joined bytes come only from parts that the manifest has just verified, and the copy is checked against the hash of those bytes before `LOAD`. So the manifest covers what is loaded.
- When the cache folder cannot be written, the type source fails with an error naming the folder. Brightfield then runs without a type source and says so, as it does for any bundle that will not open. It does not fall back to a temporary folder.

`bundle_beside` finds a bundle by either file. `semantic::EXTENSION_FILE` stays the joined name. The split names become constants beside it.

### Checks that read the bundle

- `scripts/check-bundled-extension.sh` reads the trailer from the trailer file when the bundle is split, and otherwise keeps its checks. It also checks that the Mach-O part passes `codesign --verify --strict`.
- `scripts/verify-airgapped.sh` needs no change in its logic. Its redirected `HOME` means the cache copy is written under the run's temporary home, which also proves that first launch writes the copy offline. A leg asserting that the copy exists afterwards is the builder's call.
- `scripts/package-finetype-selftest.sh` and `scripts/check-artifact-type-source.sh` cover the split layout in the `.dmg`.
- `.github/workflows/signing-rehearsal.yml` already tabulates what each nested Mach-O carries. It now expects `finetype.dylib` to carry the rehearsal identity with a timestamp, and the helper to carry DuckDB's.
- The comments in `scripts/release-signing.sh` and `release.yml` that say a signed tag stops at notarisation are updated to say what is now true.

### The local signed `.dmg`

The signed build for the second-Mac check is made on Hugh's Mac with the credentials in 1Password's Meridian vault: the Developer ID Application `.p12` and its password, and the App Store Connect API key (`.p8`, Key ID, Issuer ID). A new script, `scripts/local-signing.sh`, wraps this so a later session needs only Hugh's 1Password sign-in. The certificate stays in 1Password and is never imported into a lasting keychain (the assistant's call, explained to Hugh on 2026-11-20: an unattended Developer ID key could sign code as Meridian without his knowledge). The script reuses `scripts/release-signing.sh setup`, run with `RUNNER_TEMP` and `GITHUB_ENV` pointed at a scratch folder and the five variables read from 1Password with `op read`. That builds the same throwaway keychain and notary profile a release uses, so the local build exercises the release's own setup. `cleanup` runs afterwards. The 1Password sign-in is interactive, so Hugh runs it.

The resulting `.dmg` passes `scripts/verify-airgapped.sh` and `syspolicy_check distribution`. Its path and checksum are reported for the second-Mac check.

The second-Mac check runs on this Mac, which is Apple silicon like the published `.dmg`. This Mac is also the build machine, so two things make it behave like a clean one (the assistant's call):

- The `.dmg` is given the `com.apple.quarantine` attribute a browser download carries, so Gatekeeper assesses it at first open.
- The check runs from a separate macOS user account with the network off, so no DuckDB extension cache or FineType model from development is reachable.

### If a signed run fails

The route was measured end to end on 2026-11-20 before the plan was written, so no fallback is built in parallel (the assistant's call). If Brightfield itself fails where the throwaway app passed, the build stops and reports the failure with its evidence. The fallback is then Hugh's choice: the managed DuckDB build with FineType compiled in, the signed `.dmg` without FineType, or a later date.

## Verification

- Unit tests in `semantic.rs`, with synthesised parts in the style of the existing trailer tests: split and join round-trip byte for byte, a reused cache copy, a corrupted cache copy rewritten, a part that fails the manifest refused before any `LOAD`, and an unwritable cache folder giving the named error.
- `cargo xtask ci` green, including the ad-hoc `.dmg` running the split path through `check-artifact-type-source.sh` and `verify-airgapped.sh`.
- A local signed run: notarytool accepts the bundle, `stapler staple` succeeds, `syspolicy_check distribution` passes, and with the network off the app types a column (canary), `--check-engine` matches, and `LOAD spatial` succeeds once spatial is installed.

## Left to the builder

- File names for the two parts, and whether the shared cut logic is a shell helper or a small Rust tool.
- How `HOME` is resolved for the cache. Follow the existing `Application Support` lookups in the workspace.
- Whether `verify-airgapped.sh` asserts the cache copy's presence.
- The order of tasks. The riskiest assumption should be measured first: a Developer-ID-signed Mach-O part with the trailer re-appended loads under the hardened runtime with the entitlement.
