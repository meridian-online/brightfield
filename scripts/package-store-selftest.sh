#!/usr/bin/env bash
# Check the Mac App Store package scripts/package-store.sh makes.
#
#   scripts/package-store-selftest.sh           build the package ad-hoc, then check it
#   scripts/package-store-selftest.sh APP       check the Brightfield.app at APP
#
# BRIGHTFIELD_STORE_APP, when set, names APP in place of the argument, so the
# `package-store-selftest` step checks a package a workflow signed for the
# Store rather than building its own.
#
# What it requires of the bundle:
#
#   - the app's signature carries exactly the five entitlements
#     packaging/store/Brightfield.entitlements names, plus the application and
#     team identifiers when a provisioning profile is embedded;
#   - each helper's carries exactly app-sandbox and inherit;
#   - Contents/Helpers/duckdb is the pinned DuckDB build: its --version prints
#     what DuckDB v2.0.0's release prints, which arc checks;
#   - no .duckdb_extension file is anywhere in the bundle;
#   - Brightfield and its helper copy link Meridian's DuckDB by @rpath and find
#     it in Contents/Frameworks;
#   - FineType's manifest names files, none of them an extension file, and
#     they hash as recorded;
#   - Info.plist answers App Store Connect's encryption question;
#   - codesign --verify --strict --deep passes on the app.
#
# Then it shows the checks can fail: a copy of the bundle with one helper
# re-signed with the app's entitlements, and a copy carrying an extension file,
# are each refused.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
EXPECTED_VERSION="v2.0.0 (Cyanoptera) d54cbbe3df"
APP_KEYS="com.apple.security.app-sandbox com.apple.security.files.bookmarks.app-scope com.apple.security.files.downloads.read-write com.apple.security.files.user-selected.read-write com.apple.security.network.client"
PROFILE_KEYS="com.apple.application-identifier com.apple.developer.team-identifier"
HELPER_KEYS="com.apple.security.app-sandbox com.apple.security.inherit"
HELPERS="brightfield-helper duckdb finetype"

APP="${BRIGHTFIELD_STORE_APP:-${1:-}}"
if [ -z "$APP" ]; then
  "$ROOT/scripts/package-store.sh" || { echo "package-store-selftest: scripts/package-store.sh failed" >&2; exit 1; }
  APP="$ROOT/dist/store/Brightfield.app"
fi
[ -d "$APP/Contents" ] || { echo "package-store-selftest: no app bundle at ${APP}" >&2; exit 1; }

SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/bf-store-selftest.XXXXXX")"
trap 'rm -rf "$SCRATCH"' EXIT

# The entitlement keys in PATH's signature whose value is true or a string,
# sorted and on one line.
entitlement_keys() {
  codesign -d --entitlements - --xml "$1" 2>/dev/null | python3 -I -c '
import plistlib, sys
data = sys.stdin.buffer.read()
ents = plistlib.loads(data) if data.strip() else {}
print(" ".join(sorted(k for k, v in ents.items() if v is True or isinstance(v, str))))
'
}

sorted_words() { tr ' ' '\n' <<<"$*" | LC_ALL=C sort | tr '\n' ' ' | sed 's/ $//'; }

# check APP: print "ok"/"FAIL" lines; exit 1 if any check failed.
check() {
  local app="$1" failed=0 want got helper
  ok() { echo "ok   $*"; }
  bad() { echo "FAIL $*"; failed=1; }

  want="$APP_KEYS"
  if [ -f "$app/Contents/embedded.provisionprofile" ]; then want="$APP_KEYS $PROFILE_KEYS"; fi
  want="$(sorted_words "$want")"
  got="$(entitlement_keys "$app")"
  if [ "$got" = "$want" ]; then ok "the app's entitlements are exactly: $got"
  else bad "the app's entitlements are '$got', not '$want'"; fi

  for helper in $HELPERS; do
    local path="$app/Contents/Helpers/$helper"
    if [ ! -x "$path" ]; then bad "no helper at Contents/Helpers/$helper"; continue; fi
    got="$(entitlement_keys "$path")"
    if [ "$got" = "$(sorted_words "$HELPER_KEYS")" ]; then ok "Contents/Helpers/$helper's entitlements are exactly: $got"
    else bad "Contents/Helpers/$helper's entitlements are '$got', not app-sandbox and inherit"; fi
  done

  # A helper signed with inherit is stopped when it is started by anything but
  # a sandboxed parent, so the version is read from a copy signed with no
  # entitlements, laid out with the bundle's Frameworks beside it.
  local probe="$SCRATCH/version/Contents"
  rm -rf "$SCRATCH/version"; mkdir -p "$probe/Helpers"
  cp -R "$app/Contents/Frameworks" "$probe/Frameworks"
  cp "$app/Contents/Helpers/duckdb" "$probe/Helpers/duckdb"
  codesign --force --sign - "$probe/Helpers/duckdb" 2>/dev/null
  got="$(DUCKDB_AGENT_MODE=0 "$probe/Helpers/duckdb" --version 2>&1)"
  if [ "$got" = "$EXPECTED_VERSION" ]; then ok "Contents/Helpers/duckdb --version prints $got"
  else bad "Contents/Helpers/duckdb --version prints '$got', not '$EXPECTED_VERSION'"; fi
  # dyld prints the library's resolved path, with no `..` and through /private.
  local frameworks loaded
  frameworks="$(cd "$probe/Frameworks" && pwd -P)"
  loaded="$(DYLD_PRINT_LIBRARIES=1 DUCKDB_AGENT_MODE=0 "$probe/Helpers/duckdb" --version 2>&1 | grep 'libduckdb.dylib' || true)"
  if [ "$(wc -l <<<"$loaded" | tr -d ' ')" = 1 ] && [[ "$loaded" == *" $frameworks/libduckdb.dylib" ]]; then
    ok "Contents/Helpers/duckdb loads libduckdb.dylib from Contents/Frameworks"
  else bad "Contents/Helpers/duckdb does not load libduckdb.dylib from Contents/Frameworks"; fi

  got="$(find "$app" -name '*.duckdb_extension')"
  if [ -z "$got" ]; then ok "no .duckdb_extension file in the bundle"
  else bad "extension files in the bundle: $got"; fi

  local exe
  for exe in "$app/Contents/MacOS/brightfield" "$app/Contents/Helpers/brightfield-helper"; do
    if otool -L "$exe" | grep -q '@rpath/libduckdb.dylib' \
       && otool -l "$exe" | grep -A2 LC_RPATH | grep -q '@executable_path/../Frameworks'; then
      ok "${exe#"$app"/} links @rpath/libduckdb.dylib with the rpath @executable_path/../Frameworks"
    else bad "${exe#"$app"/} does not link Meridian's DuckDB from Contents/Frameworks"; fi
  done
  if [ -f "$app/Contents/Frameworks/libduckdb.dylib" ]; then ok "Contents/Frameworks holds libduckdb.dylib"
  else bad "no Contents/Frameworks/libduckdb.dylib"; fi

  local ft="$app/Contents/Resources/finetype" manifest
  manifest="$ft/bundle-manifest.sha256"
  if [ ! -s "$manifest" ]; then bad "no FineType manifest at Resources/finetype/bundle-manifest.sha256"
  elif grep -Eq '  (finetype\.duckdb_extension|finetype\.dylib|finetype\.duckdb_extension\.trailer)$' "$manifest"; then
    bad "FineType's manifest names an extension file"
  elif ! grep -q '  model/model.safetensors$' "$manifest" || ! grep -q '  taxonomy-schemas.json$' "$manifest"; then
    bad "FineType's manifest does not name the model and the catalogue"
  elif (cd "$ft" && shasum -a 256 -c --quiet bundle-manifest.sha256 >/dev/null 2>&1); then
    ok "FineType's manifest names $(wc -l < "$manifest" | tr -d ' ') files, no extension file, and they hash as recorded"
  else bad "a file in Resources/finetype does not hash as its manifest records"; fi

  if [ "$(plutil -extract ITSAppUsesNonExemptEncryption raw "$app/Contents/Info.plist" 2>/dev/null)" = false ]; then
    ok "Info.plist sets ITSAppUsesNonExemptEncryption to false"
  else bad "Info.plist does not set ITSAppUsesNonExemptEncryption to false"; fi

  if codesign --verify --strict --deep "$app" 2>"$SCRATCH/verify.txt"; then ok "codesign --verify --strict --deep passes"
  else bad "codesign --verify --strict --deep: $(tr '\n' ' ' < "$SCRATCH/verify.txt")"; fi

  return "$failed"
}

echo "== the package: ${APP}"
check "$APP"
status=$?

echo "== the checks refuse a helper signed with the app's entitlements"
cp -R "$APP" "$SCRATCH/wrong-helper.app"
codesign --force --sign - --entitlements "$ROOT/packaging/store/Brightfield.entitlements" \
  "$SCRATCH/wrong-helper.app/Contents/Helpers/duckdb" 2>/dev/null
codesign --force --sign - --entitlements "$ROOT/packaging/store/Brightfield.entitlements" \
  "$SCRATCH/wrong-helper.app" 2>/dev/null
# Each check's output is read whole: it fails, and pipefail would make a
# pipeline into grep fail with it.
refused="$(check "$SCRATCH/wrong-helper.app")"
if grep -q "^FAIL Contents/Helpers/duckdb's entitlements" <<<"$refused"; then
  echo "ok   refused"
else echo "FAIL a helper with the app's entitlements passed"; status=1; fi

echo "== the checks refuse a bundle carrying an extension file"
cp -R "$APP" "$SCRATCH/extension.app"
touch "$SCRATCH/extension.app/Contents/Resources/finetype/finetype.duckdb_extension"
refused="$(check "$SCRATCH/extension.app")"
if grep -q '^FAIL extension files in the bundle' <<<"$refused"; then
  echo "ok   refused"
else echo "FAIL a bundle with an extension file passed"; status=1; fi

if [ "$status" -eq 0 ]; then echo "package-store-selftest: all checks passed"
else echo "package-store-selftest: FAILED" >&2; fi
exit "$status"
