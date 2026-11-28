#!/usr/bin/env bash
# Package the managed edition, the sandboxed build for the Mac App Store.
#
#   scripts/package-store.sh [VERSION]
#
#   VERSION  label for the package's file name, e.g. v0.2.0 (default:
#            v<crate>-local). Its core must be the crate version, as in
#            scripts/package.sh.
#
# Produces, under dist/store/:
#
#   Brightfield.app                     the sandboxed bundle
#   Brightfield-<VERSION>.pkg           the installer package App Store Connect takes
#   brightfield-shot                    the managed edition's headless renderer,
#                                       copied from the build, outside the bundle, which
#                                       scripts/sandbox-measure.sh baseline --package
#                                       wraps in the bundle's own frameworks
#
# The full edition's packaging is scripts/package.sh, and this script leaves it
# alone. packaging/README.md says how the two differ and why.
#
# WHAT THE BUNDLE HOLDS.
#
#   Contents/MacOS/brightfield          Brightfield, built with --features managed-duckdb
#   Contents/Helpers/brightfield-helper the same executable, which the window starts as
#                                       arc and as dovetail (Runner::for_executable in
#                                       crates/brightfield-shell/src/run.rs)
#   Contents/Helpers/duckdb             DuckDB's shell from Meridian's build
#                                       (scripts/build-duckdb.sh), which arc runs
#                                       a Protocol's SQL steps with
#   Contents/Helpers/finetype           FineType's command line, which arc's
#                                       FineType operators run
#   Contents/Frameworks/                libduckdb.dylib, with the extensions
#                                       compiled in, and GEOS beside it
#   Contents/Resources/finetype/        FineType's model and taxonomy catalogue,
#                                       with bundle-manifest.sha256 over them.
#                                       No extension file: FineType is compiled
#                                       into the library
#   Contents/Resources/                 the icon, LICENSE and the examples, as
#                                       the full edition's bundle carries them
#   Contents/Resources/licenses/        the licence notices of the third-party
#                                       code the bundle carries: what the managed
#                                       DuckDB build wrote, GEOS's licence and the
#                                       offer of its source among them, and the
#                                       Rust crates compiled into Brightfield.
#                                       README.txt there says what is where
#
# No .duckdb_extension file is in the bundle, because codesign cannot sign one
# (docs/app-sandbox.md), and every piece of code in a Store bundle is signed.
#
# SIGNING runs inside out: the libraries in Frameworks, then each helper with
# packaging/store/Helper.entitlements (exactly app-sandbox and inherit) and an
# identifier of its own, then the app with packaging/store/Brightfield.entitlements.
# The hardened runtime is not turned on: the Store does not require it, and with
# no library loaded at run time from outside the bundle it would add nothing.
# Four variables drive it:
#
#   BRIGHTFIELD_STORE_APP_IDENTITY        "3rd Party Mac Developer Application: …"
#                                         or "Apple Distribution: …"
#   BRIGHTFIELD_STORE_INSTALLER_IDENTITY  "3rd Party Mac Developer Installer: …"
#   BRIGHTFIELD_STORE_PROFILE             the Mac App Store provisioning profile,
#                                         embedded as Contents/embedded.provisionprofile
#   BRIGHTFIELD_STORE_KEYCHAIN            the keychain holding both identities,
#                                         when it is not on the search list
#
# With all three of the first set, the bundle is signed for the Store: the
# app's signature then also carries the application and team identifiers read
# from the profile, which App Store Connect requires to match it, and the
# package is signed with the installer identity. With none set, every signature
# is ad-hoc and the package is unsigned, for local checks: an ad-hoc bundle
# runs under the same sandbox as a signed one, since the kernel applies App
# Sandbox from the entitlements in the signature, whoever signed it. Setting
# some but not all of the three is refused.
#
# INPUTS. The managed DuckDB build, at target/duckdb-managed or wherever
# BRIGHTFIELD_DUCKDB_MANAGED_DIR says (cargo xtask ci --step duckdb-managed
# makes it). FineType's model and catalogue from BRIGHTFIELD_FINETYPE_BUNDLE, a
# directory scripts/fetch-finetype-bundle.sh assembled, and its command line
# from BRIGHTFIELD_FINETYPE_CLI; either one unset is fetched at the pin, which
# needs the network the first time, and kept beside the DuckDB build.
#
# scripts/package-store-selftest.sh checks what this produces.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)

export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.95.0}"

APP_IDENTITY="${BRIGHTFIELD_STORE_APP_IDENTITY:-}"
INSTALLER_IDENTITY="${BRIGHTFIELD_STORE_INSTALLER_IDENTITY:-}"
PROFILE="${BRIGHTFIELD_STORE_PROFILE:-}"
KEYCHAIN="${BRIGHTFIELD_STORE_KEYCHAIN:-}"

fail() { echo "package-store.sh: $*" >&2; exit 1; }

signed=""
if [ -n "$APP_IDENTITY" ] || [ -n "$INSTALLER_IDENTITY" ] || [ -n "$PROFILE" ]; then
  [ -n "$APP_IDENTITY" ] && [ -n "$INSTALLER_IDENTITY" ] && [ -n "$PROFILE" ] \
    || fail "BRIGHTFIELD_STORE_APP_IDENTITY, BRIGHTFIELD_STORE_INSTALLER_IDENTITY and BRIGHTFIELD_STORE_PROFILE are set together or not at all"
  [ -f "$PROFILE" ] || fail "no provisioning profile at ${PROFILE}"
  signed=1
fi

CRATE_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/brightfield-shell/Cargo.toml | head -1)
VERSION="${1:-v${CRATE_VERSION}-local}"
tag_core="${VERSION#v}"
tag_core="${tag_core%%-*}"
[ "$tag_core" = "$CRATE_VERSION" ] \
  || fail "version '${VERSION}' resolves to '${tag_core}', but crates/brightfield-shell/Cargo.toml declares ${CRATE_VERSION}"

HOST=$(rustc -vV | sed -n 's/^host: //p')
case "$HOST" in
  aarch64-apple-darwin) ;;
  *) fail "the Store package is built on Apple Silicon only, and this host is ${HOST}" ;;
esac

MANAGED="${BRIGHTFIELD_DUCKDB_MANAGED_DIR:-target/duckdb-managed}"
case "$MANAGED" in /*) ;; *) MANAGED="${ROOT}/${MANAGED}" ;; esac
[ -f "$MANAGED/lib/libduckdb.dylib" ] && [ -x "$MANAGED/bin/duckdb" ] \
  || fail "no managed DuckDB build in ${MANAGED}: run cargo xtask ci --step duckdb-managed"
[ -f "$MANAGED/licenses/GEOS/SOURCE.txt" ] \
  || fail "the managed DuckDB build in ${MANAGED} wrote no licence notices: build it again with cargo xtask ci --step duckdb-managed"

OUT="dist/store"
WORK="${OUT}/work"
APP="${OUT}/Brightfield.app"
PKG="${OUT}/Brightfield-${VERSION}.pkg"
rm -rf "$OUT"
mkdir -p "$WORK"

echo "== FineType"
# A fetch is kept beside the managed DuckDB build, under the pin's tag and
# model revision, so a later package on the same pin reads no network: the
# nightly keeps that directory between runs, and a host that is down at night
# must not turn main red. A changed pin is a new directory. Each fetch checks
# what it downloads against its published hashes before it is kept, and is
# renamed into place whole, so a fetch that stopped half way is never read.
FT_KEPT="${MANAGED}/finetype-kept/$(scripts/finetype-pin.sh --tag)-$(scripts/finetype-pin.sh --revision)"
mkdir -p "$(dirname "$FT_KEPT")"
FINETYPE_BUNDLE="${BRIGHTFIELD_FINETYPE_BUNDLE:-}"
if [ -z "$FINETYPE_BUNDLE" ]; then
  FINETYPE_BUNDLE="${FT_KEPT}/bundle"
  if [ -d "$FINETYPE_BUNDLE" ]; then
    echo "   the bundle fetched for this pin: ${FINETYPE_BUNDLE}"
  else
    rm -rf "${FT_KEPT}/bundle.partial"
    scripts/fetch-finetype-bundle.sh "$HOST" "${FT_KEPT}/bundle.partial"
    mv "${FT_KEPT}/bundle.partial" "$FINETYPE_BUNDLE"
  fi
fi
[ -f "$FINETYPE_BUNDLE/model/model.safetensors" ] && [ -f "$FINETYPE_BUNDLE/taxonomy-schemas.json" ] \
  || fail "${FINETYPE_BUNDLE} holds no model/model.safetensors and taxonomy-schemas.json"
FINETYPE_CLI="${BRIGHTFIELD_FINETYPE_CLI:-}"
if [ -z "$FINETYPE_CLI" ]; then
  FINETYPE_CLI="${FT_KEPT}/finetype"
  if [ -x "$FINETYPE_CLI" ]; then
    echo "   the command line fetched for this pin: ${FINETYPE_CLI}"
  else
    rm -f "${FT_KEPT}/finetype.partial"
    mkdir -p "$FT_KEPT"
    scripts/fetch-finetype-bundle.sh --cli "$HOST" "${FT_KEPT}/finetype.partial"
    mv "${FT_KEPT}/finetype.partial" "$FINETYPE_CLI"
  fi
fi
[ -x "$FINETYPE_CLI" ] || fail "no FineType command line at ${FINETYPE_CLI}"

echo "== build (release, locked, managed edition)"
BRIGHTFIELD_DUCKDB_MANAGED_DIR="$MANAGED" cargo build --release --locked -p brightfield-shell \
  --features managed-duckdb --bin brightfield-shell --bin brightfield-shot
# Where cargo put them: CARGO_TARGET_DIR when set, as the gate and the nightly
# set it, and ./target otherwise.
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
BIN="${TARGET_DIR}/release/brightfield-shell"
SHOT="${TARGET_DIR}/release/brightfield-shot"

echo "== audit: linked libraries (${BIN})"
# The OS's libraries, and Meridian's DuckDB by its install name, which the
# bundle carries in Contents/Frameworks. Anything else would be a library the
# bundle does not hold.
otool -L "$BIN" | tail -n +2 | awk '{print $1}' | while read -r lib; do
  case "$lib" in
    /usr/lib/*|/System/Library/*|@rpath/libduckdb.dylib) ;;
    *) fail "${BIN} links ${lib}, which is neither the OS's nor the bundle's" ;;
  esac
done
otool -L "$BIN" | grep -q '@rpath/libduckdb.dylib' \
  || fail "${BIN} does not link @rpath/libduckdb.dylib: was it built with --features managed-duckdb?"

echo "== bundle: ${APP}"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Helpers" "$APP/Contents/Frameworks" \
  "$APP/Contents/Resources/examples"
cp "$BIN" "$APP/Contents/MacOS/brightfield"
cp "$BIN" "$APP/Contents/Helpers/brightfield-helper"
cp "$MANAGED/bin/duckdb" "$APP/Contents/Helpers/duckdb"
cp "$FINETYPE_CLI" "$APP/Contents/Helpers/finetype"
chmod 755 "$APP/Contents/Helpers/"*
cp "$MANAGED"/lib/*.dylib "$APP/Contents/Frameworks/"
cp "$SHOT" "${OUT}/brightfield-shot"

cp packaging/Brightfield.icns "$APP/Contents/Resources/Brightfield.icns"
cp LICENSE "$APP/Contents/Resources/LICENSE"
cp examples/*.yaml "$APP/Contents/Resources/examples/"
cp -R examples/protocol "$APP/Contents/Resources/examples/protocol"
cp -R examples/remote "$APP/Contents/Resources/examples/remote"
mkdir -p "$APP/Contents/Resources/examples/data"
cp crates/brightfield-shell/tests/data/edgar_gleif_reduced.parquet crates/brightfield-shell/tests/data/edgar_gleif_reduced.json \
  "$APP/Contents/Resources/examples/data/"

# The licence notices: the managed build's, for the libraries in Frameworks and
# FineType's command line, and the Rust crates compiled into Brightfield, read
# from the lockfile with the features it was built with above.
NOTICES="$APP/Contents/Resources/licenses"
cp -R "$MANAGED/licenses" "$NOTICES"
python3 scripts/duckdb-managed/notices/notices.py rust "$NOTICES/rust-crates/brightfield.txt" \
  --manifest-path crates/brightfield-shell/Cargo.toml --package brightfield-shell \
  --features managed-duckdb --home https://github.com/meridian-online/brightfield \
  --title "Brightfield, Contents/MacOS/brightfield and Contents/Helpers/brightfield-helper"
python3 scripts/duckdb-managed/notices/notices.py index "$NOTICES"

# FineType's model and catalogue, and nothing that is code: the extension is
# compiled into the library. The manifest names only these files, so the
# engine's built-in open has something to verify.
FT="$APP/Contents/Resources/finetype"
mkdir -p "$FT"
cp -RL "$FINETYPE_BUNDLE/model" "$FT/model"
cp "$FINETYPE_BUNDLE/taxonomy-schemas.json" "$FT/taxonomy-schemas.json"
( cd "$FT" && find . -type f ! -name bundle-manifest.sha256 | sed 's|^\./||' | LC_ALL=C sort \
    | xargs shasum -a 256 > bundle-manifest.sha256 )
echo "   staged finetype/: $(wc -l < "$FT/bundle-manifest.sha256" | tr -d ' ') files, hashes recorded"

if find "$APP" -name '*.duckdb_extension' | grep -q .; then
  fail "a .duckdb_extension file is in the bundle, which codesign cannot sign"
fi

cp packaging/Info.plist "$APP/Contents/Info.plist"
MINOS=$(otool -l "$BIN" | awk '
  $1 == "cmd" && $2 == "LC_BUILD_VERSION" { want = "minos"; next }
  $1 == "cmd"                             { want = "";      next }
  want != "" && $1 == want                { print $2; exit }
')
[ -n "$MINOS" ] || fail "no macOS version floor in ${BIN}"
# The build is Apple silicon only, and App Store Connect refuses such an app
# when it declares a floor below macOS 12.0, though its binaries run on 11. The
# Store package declares the later of the two; the full edition keeps its own.
if [ "$(printf '%s\n12.0\n' "$MINOS" | sort -V | head -1)" != 12.0 ]; then MINOS=12.0; fi
plist() { plutil "$@" "$APP/Contents/Info.plist"; }
# packaging/Info.plist carries the full edition's identifier. The Store
# edition keeps online.meridian.brightfield, which its App ID, provisioning
# profile and App Store Connect record name.
plist -replace CFBundleIdentifier -string online.meridian.brightfield
plist -replace CFBundleURLTypes.0.CFBundleURLName -string online.meridian.brightfield
plist -replace CFBundleShortVersionString -string "$CRATE_VERSION"
plist -replace CFBundleVersion -string "$CRATE_VERSION"
plist -replace LSMinimumSystemVersion -string "$MINOS"
# App Store Connect asks every upload these two. The app uses only the
# encryption the OS provides (HTTPS through DuckDB's httpfs), which is exempt.
plist -replace ITSAppUsesNonExemptEncryption -bool NO
plist -replace LSApplicationCategoryType -string public.app-category.developer-tools
plutil -lint "$APP/Contents/Info.plist"

echo "== sign: inside out ($([ -n "$signed" ] && echo "for the Store" || echo ad-hoc))"
if [ -n "$signed" ]; then
  SIGN=(codesign --force --timestamp --sign "$APP_IDENTITY")
  if [ -n "$KEYCHAIN" ]; then SIGN+=(--keychain "$KEYCHAIN"); fi
else
  SIGN=(codesign --force --sign -)
fi

for lib in "$APP"/Contents/Frameworks/*.dylib; do
  "${SIGN[@]}" "$lib"
done
for helper in brightfield-helper duckdb finetype; do
  "${SIGN[@]}" --identifier "online.meridian.brightfield.${helper}" \
    --entitlements packaging/store/Helper.entitlements "$APP/Contents/Helpers/${helper}"
done

APP_ENTITLEMENTS=packaging/store/Brightfield.entitlements
if [ -n "$signed" ]; then
  cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"
  security cms -D -i "$PROFILE" > "${WORK}/profile.plist"
  app_id=$(plutil -extract Entitlements.com\\.apple\\.application-identifier raw "${WORK}/profile.plist")
  team_id=$(plutil -extract Entitlements.com\\.apple\\.developer\\.team-identifier raw "${WORK}/profile.plist")
  [ "$app_id" = "${team_id}.online.meridian.brightfield" ] \
    || fail "the profile is for ${app_id}, not ${team_id}.online.meridian.brightfield"
  APP_ENTITLEMENTS="${WORK}/Brightfield.entitlements"
  cp packaging/store/Brightfield.entitlements "$APP_ENTITLEMENTS"
  plutil -insert com\\.apple\\.application-identifier -string "$app_id" "$APP_ENTITLEMENTS"
  plutil -insert com\\.apple\\.developer\\.team-identifier -string "$team_id" "$APP_ENTITLEMENTS"
fi
"${SIGN[@]}" --entitlements "$APP_ENTITLEMENTS" "$APP"
codesign --verify --strict --deep "$APP"

echo "== package: ${PKG}"
PRODUCT=(productbuild --component "$APP" /Applications)
if [ -n "$signed" ]; then
  PRODUCT+=(--sign "$INSTALLER_IDENTITY")
  if [ -n "$KEYCHAIN" ]; then PRODUCT+=(--keychain "$KEYCHAIN"); fi
fi
"${PRODUCT[@]}" "$PKG"
rm -rf "$WORK"

echo "== done"
du -sk "$APP" | awk '{printf "   Brightfield.app: %d KiB\n", $1}'
ls -l "$PKG" | awk '{printf "   %s: %d bytes\n", $NF, $5}'
echo "check it: scripts/package-store-selftest.sh ${APP}"
