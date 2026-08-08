#!/usr/bin/env bash
# Package the single-binary distribution: one native executable, no runtime
# dependencies beyond the OS graphics stack.
#
#   scripts/package.sh [VERSION] [TARGET]
#
#   VERSION  asset version label, e.g. v0.2.0 (default: v<crate>-local)
#   TARGET   Rust target triple (default: the host)
#
# Produces, under dist/:
#
#   brightfield-<VERSION>-<TARGET>.tar.gz          the distributable
#   brightfield-<VERSION>-<TARGET>.tar.gz.sha256   its checksum sidecar
#   brightfield-<VERSION>-<TARGET>.dmg             the disk image (darwin only)
#   brightfield-<VERSION>-<TARGET>.dmg.sha256      its checksum sidecar
#
# — the same asset naming the install.meridian.online convention consumes
# (`<tool>-<version>-<target>.tar.gz` + `.sha256` per asset), so the tag-driven
# release workflow and this script are the same packaging, not two.
#
# TWO ARTIFACTS PER DARWIN TARGET, FROM ONE BUILD, and the split is the point.
#
# The tarball is what `curl … | bash`, the Homebrew formula and
# scripts/verify-airgapped.sh consume, and its interior does not change. It
# contains:
#   brightfield     the binary (built from the brightfield-shell crate)
#   LICENSE
#   README.txt      what it is, how to open the bundled examples
#   examples/       the self-contained specs (inline data, no network), the
#                   vendored Protocol manifests, and examples/remote/ — the
#                   specs that DO fetch. Everything the air-gapped smoke test
#                   (scripts/verify-airgapped.sh) needs is inside the artifact
#                   it tests, and that now includes the negative case: a spec
#                   that must FAIL with the network denied, which cannot be
#                   demonstrated against an artifact that does not carry one.
#                   examples/live/ is still deliberately NOT shipped, and the
#                   distinction is worth keeping straight — a live spec needs a
#                   data file this repo does not vendor, so it could only ever
#                   fail for want of a file the user was told to download; a
#                   remote spec is complete, and either reaches its source or
#                   says exactly why it could not.
#
# The disk image is what a download button hands a stranger: a window holding
# Brightfield.app and an /Applications alias. A notarization ticket cannot be
# stapled to a `.tar.gz` at any signing state — `stapler` refuses the format —
# so the bundle is the artifact that can carry one. The bundle keeps its own
# copy of `examples/` and `LICENSE` under Contents/Resources, because an app
# launched from /Applications has no sibling directory to read them from.
#
# Before either artifact is assembled, the script audits the BUILT
# EXECUTABLE's linked libraries (otool -L / ldd) against an OS allowlist and
# fails on anything else — the mechanical form of "no runtime deps beyond
# graphics drivers". (On macOS and Linux the GPU stack is reached via OS
# frameworks / dlopen at runtime, so it never appears as a link-time
# dependency; anything non-OS that does appear is a packaging regression.)
# Auditing the built executable rather than a staged copy is what makes one
# audit cover both artifacts, and it is what keeps a mistyped staging path from
# turning the audit into a no-op that still prints its clean verdict.
#
# SIGNING. Two environment variables drive it, and with neither set the bundle
# is signed ad-hoc and the tarball's Mach-O is left exactly as the linker signed
# it:
#
#   BRIGHTFIELD_SIGN_IDENTITY   a codesign identity, e.g.
#                               "Developer ID Application: … (TEAMID)". Signs
#                               the tarball's bare Mach-O and the bundle, both
#                               with the hardened runtime and a secure
#                               timestamp.
#   BRIGHTFIELD_NOTARY_PROFILE  a keychain profile stored by
#                               `xcrun notarytool store-credentials`. Read only
#                               when an identity is also set. Submits the
#                               bundle once and staples the ticket to it.
#
# The chain runs in this order: sign the bare Mach-O → sign the bundle → zip the
# bundle for submission → one `notarytool submit` → one `stapler staple` on the
# bundle → build the image around the stapled bundle. The image itself is
# neither signed nor stapled: the ticket rides on the bundle inside it, which is
# what a stranger's machine reads at launch. If the image staple is wanted
# later, `codesign` and `stapler staple` on the image append after
# `hdiutil create` and change nothing above them.
set -euo pipefail
cd "$(dirname "$0")/.."

SIGN_IDENTITY="${BRIGHTFIELD_SIGN_IDENTITY:-}"
NOTARY_PROFILE="${BRIGHTFIELD_NOTARY_PROFILE:-}"

# The repo's exact toolchain pin (the same one CI nails via its toolchain
# action; there is no rust-toolchain.toml). Overridable, but the default must
# not be "whatever cargo the shell finds" — the workspace floor is above some
# installed defaults, and a release artifact should be built by the pinned
# compiler, not by luck.
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.95.0}"

CRATE_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/brightfield-shell/Cargo.toml | head -1)
VERSION="${1:-v${CRATE_VERSION}-local}"

# A named release tag must carry the crate's own version. The release workflow
# passes GITHUB_REF_NAME here — a `v*` tag — and the whole artifact (tarball
# name, README, and the binary's own `--version`) is labelled from it. If the
# tag says v0.2.0 while the crate is still 0.1.0, that artifact ships mislabelled
# and the in-binary version disagrees with the download. Refuse to build it.
#
# Runs before `rustc`/`cargo` so the check is a fast, toolchain-free fail. Only
# fires when a VERSION argument was actually given: with none, VERSION is the
# `v<crate>-local` default, which is derived from the crate and cannot disagree.
if [ "$#" -ge 1 ]; then
  tag_core="${1#v}"          # strip a leading v (the workflow tags as `v*`)
  tag_core="${tag_core%%-*}" # strip any -prerelease / -local suffix
  if [ "$tag_core" != "$CRATE_VERSION" ]; then
    echo "package.sh: version mismatch — tag '${1}' resolves to '${tag_core}', but" >&2
    echo "  crates/brightfield-shell/Cargo.toml declares version ${CRATE_VERSION}." >&2
    echo "  Bump the crate version or retag so they agree; refusing to build a" >&2
    echo "  mislabelled artifact." >&2
    exit 1
  fi
fi

HOST=$(rustc -vV | sed -n 's/^host: //p')
TARGET="${2:-$HOST}"

NAME="brightfield-${VERSION}-${TARGET}"
STAGE="dist/${NAME}"

echo "== build (release, locked): ${TARGET}"
BUILD=(cargo build --release --locked -p brightfield-shell --bin brightfield-shell)
BIN="target/release/brightfield-shell"
if [ "$TARGET" != "$HOST" ]; then
  BUILD+=(--target "$TARGET")
  BIN="target/${TARGET}/release/brightfield-shell"
fi
"${BUILD[@]}"

# ---------------------------------------------------------------------------
# The audit runs HERE: against the built executable, before anything is copied
# anywhere. Both artifacts carry this same object file, so one reading covers
# both by construction, and no staging path can be mistyped between the reading
# and the verdict.
#
# `audit_rule` doubles as the record of whether a rule ran at all. A branch that
# sets it must reach a verdict; a branch that leaves it empty says so on its own
# line and never reaches "clean", because a clean verdict from a case that
# inspected nothing is the failure this whole block is arranged to prevent.
# ---------------------------------------------------------------------------
echo "== audit: linked libraries (${BIN})"
[ -f "$BIN" ] || { echo "audit FAILED: no executable at ${BIN}"; exit 1; }
audit_ok=1
audit_rule=""
audit_lines=0
case "$TARGET" in
  *-apple-darwin)
    audit_rule="otool -L"
    # otool reads any Mach-O regardless of host arch. Skip the first line
    # (the file's own name); allow OS library dirs only.
    while IFS= read -r line; do
      audit_lines=$((audit_lines + 1))
      lib=$(echo "$line" | awk '{print $1}')
      case "$lib" in
        /usr/lib/*|/System/Library/*) ;;
        *) echo "   NON-OS LINKAGE: $lib"; audit_ok=0 ;;
      esac
    done < <(otool -L "$BIN" | tail -n +2)
    ;;
  *-linux-*)
    if [ "$TARGET" = "$HOST" ]; then
      audit_rule="ldd"
      while IFS= read -r line; do
        audit_lines=$((audit_lines + 1))
        lib=$(echo "$line" | awk '{print $1}')
        case "$lib" in
          linux-vdso*|ld-linux*|/lib*/ld-linux*) ;;
          libc.so*|libm.so*|libdl.so*|libpthread.so*|librt.so*|libgcc_s.so*|libutil.so*) ;;
          *) echo "   NON-OS LINKAGE: $line"; audit_ok=0 ;;
        esac
      done < <(ldd "$BIN")
    else
      echo "   NOT AUDITED (cross build: ldd needs the target OS — run it there)"
    fi
    ;;
  *)
    echo "   NOT AUDITED (no audit rule for ${TARGET})"
    ;;
esac
if [ -n "$audit_rule" ]; then
  # Reading zero lines is a failure, not a pass. otool -L and ldd write nothing
  # to stdout when they cannot open the file they were handed, so the loop above
  # would not run once and every check below it would be satisfied by an empty
  # reading.
  [ "$audit_lines" -gt 0 ] || {
    echo "audit FAILED: ${audit_rule} listed no linked libraries for ${BIN}"; exit 1; }
  [ "$audit_ok" -eq 1 ] || { echo "audit FAILED: the binary links non-OS libraries"; exit 1; }
  echo "   clean: OS libraries only (${audit_lines} entries read)"
fi

# ---------------------------------------------------------------------------
# THE SEMANTIC TYPE SOURCE — the FineType extension, its model and its schema
# catalogue, staged into the artifact so a packaged Brightfield can ask what a
# column MEANS with no network and no cache.
#
# It is not built here. This repository does not build FineType and must not
# start: the extension is a cdylib from another workspace with its own
# toolchain pin, and the model is a ~17 MB artifact from a model registry.
# BRIGHTFIELD_FINETYPE_BUNDLE names a directory somebody else assembled, and
# everything below is about refusing a bad one rather than making a good one.
#
# With the variable unset the artifact ships WITHOUT a type source and the
# application reports every column as NotAsked. That is a supported build, not
# a degraded one — it is what a contributor's local `scripts/package.sh` run
# produces — so this is a message, not a failure.
#
# What makes a bundle acceptable is scripts/check-bundled-extension.sh, which
# is also what scripts/verify-airgapped.sh runs over the bundle inside a built
# artifact — one reading of the metadata trailer, not two that can drift. Its
# own self-test runs on every pull request; this call and the airgap one happen
# only on a release.
# ---------------------------------------------------------------------------
FINETYPE_BUNDLE="${BRIGHTFIELD_FINETYPE_BUNDLE:-}"
if [ -z "$FINETYPE_BUNDLE" ]; then
  echo "== type source: none (BRIGHTFIELD_FINETYPE_BUNDLE unset)"
  echo "   the artifact ships without one; columns report no semantic label"
else
  echo "== type source: ${FINETYPE_BUNDLE}"
  # The DuckDB platform name for the target being PACKAGED, which is not
  # necessarily the host's. An unknown target is a hard failure rather than an
  # unchecked pass: a mapping nobody added is a bundle nobody checked.
  case "$TARGET" in
    aarch64-apple-darwin)      want_platform=osx_arm64 ;;
    x86_64-apple-darwin)       want_platform=osx_amd64 ;;
    aarch64-unknown-linux-gnu) want_platform=linux_arm64 ;;
    x86_64-unknown-linux-gnu)  want_platform=linux_amd64 ;;
    *)
      echo "package.sh: no DuckDB platform name known for target ${TARGET}, so the" >&2
      echo "  bundled extension cannot be checked against it. Add the mapping." >&2
      exit 1 ;;
  esac
  scripts/check-bundled-extension.sh "$FINETYPE_BUNDLE" "$want_platform" \
    || { echo "package.sh: refusing to stage a bundle that would not load." >&2; exit 1; }
fi

echo "== stage: ${STAGE}"
rm -rf "$STAGE"
mkdir -p "$STAGE/examples"
# Distributed under the product's name; the crate keeps its own.
cp "$BIN" "$STAGE/brightfield"
cp LICENSE "$STAGE/LICENSE"
cp examples/*.yaml "$STAGE/examples/"
cp -R examples/protocol "$STAGE/examples/protocol"
cp -R examples/remote "$STAGE/examples/remote"
# `cp -RL` dereferences: a model fetched through a content-addressed cache is a
# tree of symlinks into that cache, and copying the links would produce an
# artifact that works only on the machine that built it.
[ -z "$FINETYPE_BUNDLE" ] || cp -RL "$FINETYPE_BUNDLE" "$STAGE/finetype"
cat > "$STAGE/README.txt" <<EOF
brightfield ${VERSION} (${TARGET})

One native binary. No server, no webview, no language runtime, and no network
needed to run — the only thing it asks of the machine is a working graphics
driver. Nothing here reports anywhere; it reaches out only when a spec names a
remote source, a DuckLake catalog or a spatial source.

Everything under examples/ is local and self-contained EXCEPT examples/remote/,
which is where the specs that fetch are kept, so that "does this need a
connection" is a fact about a path rather than something you find out by
running one. With no connection a remote spec does not open: it exits non-zero
having named the network and the URL it could not reach, and nothing else in
here is affected.

Start on the bundled gallery:

  ./brightfield

Or run it on a bundled example:

  ./brightfield examples/bars.yaml

Open a Protocol manifest (rendered from the manifest alone, no run):

  BRIGHTFIELD_PROTOCOL_OFFLINE=1 ./brightfield examples/protocol/edgar_gleif/arcform.yaml

Chart the published EDGAR-GLEIF crosswalk (this one needs a connection):

  ./brightfield examples/remote/edgar-gleif-crosswalk.yaml

Unattended smoke test (renders, screenshots itself, exits; exit 0 means the
PNG landed):

  ./brightfield examples/bars.yaml --shot-after 45 --shot-out smoke.png

Flags: [SPEC.yaml] [--theme light|dark] [--flow vertical|horizontal]
       [--shot-out PATH] [--shot-after N]
EOF

# The tarball's Mach-O is signed only when a real identity is given. With none,
# it keeps the ad-hoc signature the linker gave it — which is what ships today,
# and re-signing it ad-hoc would change the code-signing identifier for no gain.
if [ -n "$SIGN_IDENTITY" ]; then
  case "$TARGET" in
    *-apple-darwin)
      echo "== sign: ${STAGE}/brightfield"
      codesign --force --options runtime --timestamp \
        --sign "$SIGN_IDENTITY" "$STAGE/brightfield"
      ;;
  esac
fi

echo "== archive"
tar -czf "dist/${NAME}.tar.gz" -C dist "$NAME"
(cd dist && shasum -a 256 "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256")
rm -rf "$STAGE"

case "$TARGET" in
  *-apple-darwin)
    APPSTAGE="dist/${NAME}.image"
    APP="$APPSTAGE/Brightfield.app"

    echo "== bundle: ${APP}"
    rm -rf "$APPSTAGE"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/examples"
    cp "$BIN" "$APP/Contents/MacOS/brightfield"
    cp packaging/Info.plist "$APP/Contents/Info.plist"
    cp packaging/Brightfield.icns "$APP/Contents/Resources/Brightfield.icns"
    cp LICENSE "$APP/Contents/Resources/LICENSE"
    cp examples/*.yaml "$APP/Contents/Resources/examples/"
    cp -R examples/protocol "$APP/Contents/Resources/examples/protocol"
    cp -R examples/remote "$APP/Contents/Resources/examples/remote"
    # Resources/, not MacOS/: an app launched from /Applications has no sibling
    # directory, and `semantic::bundle_beside` looks in both places for exactly
    # this reason.
    [ -z "$FINETYPE_BUNDLE" ] || cp -RL "$FINETYPE_BUNDLE" "$APP/Contents/Resources/finetype"

    # The system floor is read out of the executable rather than declared, so a
    # toolchain that moves its deployment target moves this with it. An empty
    # reading is a hard failure: LSMinimumSystemVersion is what stops the bundle
    # launching on a system its own Mach-O will not run on.
    #
    # Both load commands are read because the two darwin targets in the release
    # matrix do not agree on which one they carry. Measured on this toolchain:
    # aarch64-apple-darwin emits `LC_BUILD_VERSION` with a `minos` field,
    # x86_64-apple-darwin emits the older `LC_VERSION_MIN_MACOSX` with a
    # `version` field, and neither carries the other's. `want` is cleared at
    # every `cmd` line so a field of the same name in some later load command
    # — LC_SOURCE_VERSION also has a `version` — cannot be read as this one.
    MINOS=$(otool -l "$BIN" | awk '
      $1 == "cmd" && $2 == "LC_BUILD_VERSION"     { want = "minos";   next }
      $1 == "cmd" && $2 == "LC_VERSION_MIN_MACOSX" { want = "version"; next }
      $1 == "cmd"                                  { want = "";       next }
      want != "" && $1 == want                     { print $2; exit }
    ')
    [ -n "$MINOS" ] || {
      echo "package.sh: no macOS version floor in ${BIN} — neither" >&2
      echo "  LC_BUILD_VERSION nor LC_VERSION_MIN_MACOSX was readable, so" >&2
      echo "  LSMinimumSystemVersion cannot be set honestly." >&2
      exit 1; }
    plutil -replace CFBundleShortVersionString -string "$CRATE_VERSION" "$APP/Contents/Info.plist"
    plutil -replace CFBundleVersion -string "$CRATE_VERSION" "$APP/Contents/Info.plist"
    plutil -replace LSMinimumSystemVersion -string "$MINOS" "$APP/Contents/Info.plist"
    plutil -lint "$APP/Contents/Info.plist"

    # CFBundleIconFile is a name, not a path, and a name that resolves to
    # nothing costs an icon rather than a build. Resolve it here so it costs a
    # build instead.
    ICON=$(plutil -extract CFBundleIconFile raw "$APP/Contents/Info.plist")
    [ -f "$APP/Contents/Resources/${ICON}.icns" ] || {
      echo "package.sh: CFBundleIconFile '${ICON}' resolves to no .icns under" >&2
      echo "  ${APP}/Contents/Resources." >&2
      exit 1; }

    echo "== sign: ${APP}"
    if [ -n "$SIGN_IDENTITY" ]; then
      codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$APP"
    else
      codesign --force --sign - "$APP"
    fi
    codesign --verify --strict "$APP"

    STAPLED=""
    if [ -n "$SIGN_IDENTITY" ] && [ -n "$NOTARY_PROFILE" ]; then
      echo "== notarize: ${APP}"
      ZIP="dist/${NAME}.submission.zip"
      rm -f "$ZIP"
      # ditto -c -k --keepParent is the form notarytool accepts: a zip that
      # preserves the bundle's symlinks, extended attributes and top directory.
      ditto -c -k --keepParent "$APP" "$ZIP"
      xcrun notarytool submit "$ZIP" --keychain-profile "$NOTARY_PROFILE" --wait
      xcrun stapler staple "$APP"
      rm -f "$ZIP"
      STAPLED=1
    fi

    # syspolicy_check is the local gate: it reads the ticket off the bundle and
    # reaches a verdict without a network round trip, which is what disqualifies
    # `stapler validate` (an online check) and `spctl --assess` (which accepted a
    # bundle whose ticket file had been removed).
    #
    # It is a gate only once a ticket is supposed to be there. An ad-hoc build
    # has none by design, so its verdict is printed and not acted on.
    if command -v syspolicy_check >/dev/null 2>&1; then
      echo "== distribution check: ${APP}"
      sp_status=0
      syspolicy_check distribution "$APP" || sp_status=$?
      if [ -n "$STAPLED" ]; then
        [ "$sp_status" -eq 0 ] || {
          echo "package.sh: syspolicy_check distribution failed (${sp_status}) on a" >&2
          echo "  notarized and stapled bundle. Do not ship this image." >&2
          exit 1; }
      else
        echo "   (unsigned build: reported, not gated — exit ${sp_status})"
      fi
    else
      echo "== distribution check: skipped (no syspolicy_check on this machine)"
    fi

    # The alias a stranger drags the bundle onto. A symlink is what Finder
    # renders as the Applications folder on a mounted image.
    ln -s /Applications "$APPSTAGE/Applications"

    echo "== image: dist/${NAME}.dmg"
    # -format ULFO is pinned: it is the LZFSE-compressed UDIF format, and it
    # produced a smaller image than UDZO over the same payload when the two were
    # measured against each other. Nothing in this repo reddens if the flag is
    # dropped, so it is commented here rather than left to look incidental.
    #
    # -size is pinned for a harder reason. Left to size the volume from
    # -srcfolder on its own, hdiutil failed intermittently on this payload with
    # "create failed - No space left on device" partway through copying the
    # executable — on inputs identical to a run that had just succeeded. So the
    # volume is sized from the staged bytes plus headroom for the filesystem's
    # own metadata. ULFO is a compressed read-only conversion, and unused blocks
    # do not survive it.
    rm -f "dist/${NAME}.dmg"
    stage_kb=$(du -sk "$APPSTAGE" | awk '{print $1}')
    hdiutil create -volname Brightfield -srcfolder "$APPSTAGE" \
      -size "$((stage_kb + 65536))k" -format ULFO -ov "dist/${NAME}.dmg"
    (cd dist && shasum -a 256 "${NAME}.dmg" > "${NAME}.dmg.sha256")
    rm -rf "$APPSTAGE"
    ;;
esac

echo "== done"
ls -lh "dist/${NAME}.tar.gz" "dist/${NAME}.tar.gz.sha256"
echo "verify air-gapped: scripts/verify-airgapped.sh dist/${NAME}.tar.gz"
if [ -f "dist/${NAME}.dmg" ]; then
  ls -lh "dist/${NAME}.dmg" "dist/${NAME}.dmg.sha256"
  echo "verify air-gapped: scripts/verify-airgapped.sh dist/${NAME}.dmg"
fi
