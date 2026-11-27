#!/usr/bin/env bash
# Hand scripts/package-store.sh the Mac App Store identities and provisioning
# profile on a CI runner, from repository secrets, and take them away again.
#
#   scripts/store-signing.sh setup     # store.yml, before package-store.sh
#   scripts/store-signing.sh cleanup   # store.yml, always, at the end
#
# It reads four environment variables, which store.yml fills from repository
# secrets of the same names:
#
#   MAS_APP_P12_BASE64        the Mac App Distribution certificate ("3rd Party
#                             Mac Developer Application" or "Apple
#                             Distribution") and its key, as a base64 .p12
#   MAS_INSTALLER_P12_BASE64  the Mac Installer Distribution certificate ("3rd
#                             Party Mac Developer Installer" or "Mac Installer
#                             Distribution") and its key, as a base64 .p12
#   MAS_P12_PASSWORD          the password both .p12 files were exported with
#   MAS_PROFILE_BASE64        the Mac App Store provisioning profile for
#                             online.meridian.brightfield, base64
#
# None set: says the package is built ad-hoc and unsigned, and exports nothing.
# Some set: exits 1 naming the missing ones. All set: makes a keychain under
# $RUNNER_TEMP with a random password, imports both identities into it, lets
# codesign and productbuild use their keys without a prompt, writes the profile
# beside it, and appends BRIGHTFIELD_STORE_APP_IDENTITY,
# BRIGHTFIELD_STORE_INSTALLER_IDENTITY, BRIGHTFIELD_STORE_PROFILE and
# BRIGHTFIELD_STORE_KEYCHAIN to $GITHUB_ENV.
set -euo pipefail

VARS=(MAS_APP_P12_BASE64 MAS_INSTALLER_P12_BASE64 MAS_P12_PASSWORD MAS_PROFILE_BASE64)
TEMP="${RUNNER_TEMP:?store-signing.sh runs on a CI runner, which sets RUNNER_TEMP}"
KEYCHAIN="${TEMP}/store-signing.keychain-db"

summary() { if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then echo "$*" >> "$GITHUB_STEP_SUMMARY"; fi; }

setup() {
  local set=() missing=() v
  for v in "${VARS[@]}"; do
    if [ -n "${!v:-}" ]; then set+=("$v"); else missing+=("$v"); fi
  done
  if [ "${#set[@]}" -eq 0 ]; then
    echo "store-signing: no Store secrets are set; the package is signed ad-hoc and left unsigned"
    summary "Store package: signed ad-hoc, package unsigned (no Store secrets set)."
    return 0
  fi
  if [ "${#missing[@]}" -gt 0 ]; then
    echo "store-signing: set all four Store secrets or none; missing: ${missing[*]}" >&2
    exit 1
  fi

  local password
  password="$(openssl rand -hex 24)"
  security create-keychain -p "$password" "$KEYCHAIN"
  security set-keychain-settings -lut 21600 "$KEYCHAIN"
  security unlock-keychain -p "$password" "$KEYCHAIN"
  local p12
  for p12 in MAS_APP_P12_BASE64 MAS_INSTALLER_P12_BASE64; do
    base64 --decode <<<"${!p12}" > "${TEMP}/${p12}.p12"
    security import "${TEMP}/${p12}.p12" -k "$KEYCHAIN" -P "$MAS_P12_PASSWORD" \
      -T /usr/bin/codesign -T /usr/bin/productbuild -T /usr/bin/security
    rm -f "${TEMP}/${p12}.p12"
  done
  security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$KEYCHAIN" >/dev/null
  # shellcheck disable=SC2046 # the search list is a list of words
  security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')

  local app installer
  app="$(security find-identity -v -p codesigning "$KEYCHAIN" \
    | grep -oE '"(3rd Party Mac Developer Application|Apple Distribution): [^"]+"' | head -1 | tr -d '"' || true)"
  installer="$(security find-identity -v "$KEYCHAIN" \
    | grep -oE '"(3rd Party Mac Developer Installer|Mac Installer Distribution): [^"]+"' | head -1 | tr -d '"' || true)"
  [ -n "$app" ] || { echo "store-signing: MAS_APP_P12_BASE64 holds no Mac App Distribution identity" >&2; exit 1; }
  [ -n "$installer" ] || { echo "store-signing: MAS_INSTALLER_P12_BASE64 holds no Mac Installer Distribution identity" >&2; exit 1; }

  local profile="${TEMP}/store.provisionprofile"
  base64 --decode <<<"$MAS_PROFILE_BASE64" > "$profile"
  {
    echo "BRIGHTFIELD_STORE_APP_IDENTITY=${app}"
    echo "BRIGHTFIELD_STORE_INSTALLER_IDENTITY=${installer}"
    echo "BRIGHTFIELD_STORE_PROFILE=${profile}"
    echo "BRIGHTFIELD_STORE_KEYCHAIN=${KEYCHAIN}"
  } >> "${GITHUB_ENV:?store-signing.sh setup runs on a CI runner, which sets GITHUB_ENV}"
  echo "store-signing: signing as ${app}; packaging as ${installer}"
  summary "Store package: signed as ${app}, packaged as ${installer}."
}

cleanup() {
  if [ -f "$KEYCHAIN" ]; then security delete-keychain "$KEYCHAIN" || true; fi
  rm -f "${TEMP}/store.provisionprofile"
}

case "${1:-}" in
  setup) setup ;;
  cleanup) cleanup ;;
  *) echo "usage: $0 setup | cleanup" >&2; exit 2 ;;
esac
