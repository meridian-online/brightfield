# packaging

What `scripts/package.sh` needs to assemble `Brightfield.app` and the disk image it ships in. Nothing here is compiled, and no test reads it.

| File | What it is |
|---|---|
| `Info.plist` | The bundle's property list, copied to `Contents/Info.plist`. Its three placeholder values are overwritten per build — see the comment in the file. |
| `Brightfield.icns` | The application icon, copied to `Contents/Resources/`. **Not MIT** — see `LICENSE-BRAND.md`. |
| `Brightfield.svg` | The 1024×1024 master the `.icns` is rasterised from. **Not MIT** — see `LICENSE-BRAND.md`. |
| `make-icns.sh` | Regenerates the `.icns` from the `.svg`. A developer-machine tool; the build does not run it. |
| `LICENSE-BRAND.md` | The carve-out taking the two brand artefacts out of the repository's MIT grant. |
| `Brightfield.entitlements` | The one entitlement the app and the signed tarball binary are signed with. See below. |

## Where the icon comes from

`Brightfield.svg` is the Meridian prime mark — the path in `meridian_white.svg` in the design-system repository's `meridian-design/brand/` — drawn white over a black plate. The plate is a superellipse inscribed in the 824×824 body of the 1024×1024 macOS icon grid, which is the shape the platform's own icons take; a plain rounded rectangle reads as a foreign application on the Dock beside them.

The mark is not redrawn, recoloured or re-proportioned: the path data is the master's, and the only transform applied to it is a uniform scale and a translation to the centre of the canvas. `make-icns.sh` is the whole derivation.

## How FineType ships in the app

`Contents/Resources/finetype/` holds the FineType extension in two parts: `finetype.dylib`, the extension's Mach-O cut at the end of its `__LINKEDIT` segment and signed, and `finetype.duckdb_extension.trailer`, DuckDB's metadata trailer, the bytes that followed it, unchanged. Beside them are the model, the taxonomy catalogue and `bundle-manifest.sha256`, which records a hash for every file, both parts included.

The trailer is split off because `codesign` cannot sign a file with bytes after its signature, and DuckDB appends its trailer after the Mach-O. Notarisation refuses nested code that fails strict validation, so the joined file could not ship in a notarised app, whoever signed it. `scripts/split-extension.sh` makes the cut, and `scripts/package.sh` signs the library part with the app's identity, or ad-hoc in an unsigned build, so an ad-hoc app exercises the same layout a signed one ships.

The tarball keeps the single joined `finetype.duckdb_extension`. No notarisation ticket can be stapled to a tarball, so nothing is gained by splitting it, and the tarball is unchanged. The Homebrew cask installs the disk image, not the tarball.

DuckDB needs the extension as one file named `finetype.duckdb_extension`, so the engine joins the parts before it loads them, into `~/Library/Caches/online.meridian.brightfield/finetype/<sha256 of the joined bytes>/finetype.duckdb_extension` (`crates/brightfield-engine/src/semantic/split.rs`). A signed app may not write into its own bundle, which is why the copy lives outside it.

The trust model of that copy:

- Both parts are checked against the manifest before anything is joined, and the bytes joined are checked again against the hashes the manifest records, so the copy holds exactly the bytes that were packaged.
- The folder is named by the hash of the joined bytes, so two bundles or two Brightfield versions never share a copy, and a copy never needs invalidating.
- A copy already there is used only if its bytes still hash to its folder's name. Otherwise it is written again, to a temporary file renamed into place, so two processes starting at once each end with a whole, correct file.
- Anyone who can write to the user's Caches folder as that user could swap the copy between the check and the load. That is outside what this defends against: the same user could equally replace the app's own files or the extensions DuckDB downloads into the same home folder.
- Old copies are not swept. Caches is purgeable, and macOS may clear it; the next launch joins the parts again.

When the cache folder cannot be written, or `HOME` is unset or not absolute, the type source fails with an error naming why, and Brightfield runs without semantic types and says so, as it does for any bundle that will not open.

## Why the editions differ

The editions may reach FineType by different routes where that makes a channel's distribution simpler. The App Store edition compiles its extensions into DuckDB. Studio's signed download ships FineType split as above and joins it at load. This route was chosen because it keeps FineType working offline from first launch, at the pinned version, with the tarball and Homebrew unchanged. Installing FineType from DuckDB's community registry on demand would leave a Mac that has never been online without semantic types and run whatever version the registry carries; compiling it into DuckDB for Studio as well would mean an engine change and an app that no longer matches the tarball.

## The entitlements file

`Brightfield.entitlements` carries one key, `com.apple.security.cs.disable-library-validation`, and `scripts/package.sh` signs the app with it, ad-hoc or not, and the tarball's binary when an identity signs it. Under the hardened runtime a process loads only libraries signed by its own team, and Studio loads DuckDB extensions it downloads at run time from DuckDB's repository (spatial, excel, sqlite and others), which are signed by other teams or ad-hoc. DuckDB's own command-line shell carries the same entitlement for the same reason. Any further entitlement is added only with a measured failure that shows why.

FineType itself loads without the entitlement when its library and the host process share a team. That was measured on 2026-11-20 with a DuckDB host signed by the same Developer ID under the hardened runtime, not yet with Brightfield as the host.

## Two things this bundle deliberately does not carry

**No App Sandbox.** The sandbox is initialised from the signature's entitlements, and a Developer ID build has no reason to declare its key, so the entitlements file above leaves it out. Declaring it would take file access away from a program whose whole job is to open the files a specification names.

**No signature on the disk image.** The notarisation ticket is stapled to `Brightfield.app`, and the image is neither signed nor stapled: the ticket rides on the bundle inside it, which is what a downloading Mac reads at launch.

## Signing

`scripts/package.sh` signs the bundle ad-hoc unless `BRIGHTFIELD_SIGN_IDENTITY` names a certificate, and the header of that script documents the chain that runs when one does: sign the tarball's binary, the FineType library and the bundle with the hardened runtime, notarise the bundle, staple it, and check it with `syspolicy_check distribution`. In a release, `scripts/release-signing.sh` sets it up from five repository secrets its header names: the release job imports the Developer ID certificate into a keychain of its own and hands `package.sh` the identity and a notarisation profile. With none of the secrets entered it builds ad-hoc. For a local signed build, `scripts/local-signing.sh` does the same from 1Password. `signing-rehearsal.yml` runs the signed path with a throwaway self-signed identity. It runs only by hand (`gh workflow run signing-rehearsal.yml --ref <branch>`), and `cargo xtask gate` prints that command when a pull request touches the signing paths.

## The Homebrew cask

Homebrew carries one Brightfield install: a cask built from the notarised `.dmg`, which puts `Brightfield.app` in the Applications folder and links a `brightfield` command. The command is an exec shim and not a symlink to the app's binary, because the binary finds its bundled type source from its own path, and a symlink loses it.

The formula was retired in the release that ships the cask. The operator ruled on 2026-11-20 that the cask replaces the formula rather than sitting beside it, and gave his reason: "having both could be confusing."

The release writes the cask only from a notarised build (`scripts/require-notarised.sh`), and commits the cask, the formula's removal and `tap_migrations.json` together, so the tap never holds a cask from an ad-hoc build or a formula beside its replacement. Gatekeeper accepts the notarised app; `scripts/check-brew-install.sh` reads its verdict with `spctl` on every release.

Migration: on `brew update`, a trusted cask migrates a formula install; otherwise Homebrew prints the commands to run. The cask refuses to install over a linked formula, naming `brew uninstall --formula brightfield`, so an install that would collide with the formula's link stops before it changes anything.

The checks that guard the cask:

- `scripts/write-brightfield-cask.sh` generates it, and `scripts/check-cask-layout.sh` requires the app stanza, the shim linked as `bin/brightfield`, and the formula refusal inside `preflight_steps`.
- `scripts/check-cask-asset.sh` proves the cask's urls resolve and its checksums match the release's disk images.
- `scripts/check-brew-install.sh` installs the published cask on a clean Mac, in `release.yml`'s `brew-install` job.
- `scripts/check-brew-install-branch.sh` does the same for a branch build, by hand (`gh workflow run brew-install-branch.yml --ref <branch>`).
