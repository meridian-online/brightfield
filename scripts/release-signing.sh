#!/usr/bin/env bash
# Hand scripts/package.sh a Developer ID identity and a notarisation profile on
# a CI runner, from repository secrets, and take them away again afterwards.
#
#   scripts/release-signing.sh setup              # release.yml, on a tag
#   scripts/release-signing.sh setup --rehearsal  # signing-rehearsal.yml only
#   scripts/release-signing.sh cleanup
#
# package.sh already signs with the hardened runtime, notarises and staples
# when BRIGHTFIELD_SIGN_IDENTITY and BRIGHTFIELD_NOTARY_PROFILE are set, and
# signs ad-hoc when they are not. This script is the part a runner needs before
# that: a keychain holding the certificate, and a notarytool profile stored in
# the same keychain. It reads five environment variables, which release.yml
# fills from repository secrets of the same names:
#
#   MACOS_DEVELOPER_ID_P12_BASE64    the Developer ID Application certificate
#                                    and its private key, exported from
#                                    Keychain Access as a .p12, base64-encoded
#   MACOS_DEVELOPER_ID_P12_PASSWORD  the password the .p12 was exported with
#   APPLE_NOTARY_KEY_P8_BASE64       an App Store Connect API key (.p8),
#                                    base64-encoded
#   APPLE_NOTARY_KEY_ID              that key's Key ID
#   APPLE_NOTARY_ISSUER_ID           the team's Issuer ID (a UUID)
#
# An API key rather than an Apple ID and app-specific password, so a release
# does not depend on one person's Apple ID.
#
# WHAT `setup` DOES WITH THEM
#   None set:     prints that the build signs ad-hoc, writes that to the run
#                 summary, exports nothing and exits 0. This is every release
#                 so far, and it stays byte-for-byte what package.sh does today.
#   Some set:     exits 1 naming the missing ones. A half-entered set is a
#                 mistake, and shipping a signed but un-notarised build because
#                 of it would be quieter than it deserves.
#   All set:      creates a keychain under $RUNNER_TEMP with a random password,
#                 imports the .p12 into it, lets codesign use the key without a
#                 prompt, puts the keychain on the user search list, stores the
#                 API key as a notarytool profile in it, and appends
#                 BRIGHTFIELD_SIGN_IDENTITY, BRIGHTFIELD_NOTARY_PROFILE and
#                 BRIGHTFIELD_NOTARY_KEYCHAIN to $GITHUB_ENV for the steps after.
#
# The identity must be named "Developer ID Application: …". Only --rehearsal
# lifts that, and --rehearsal refuses to run on a tag, so a self-signed
# identity cannot reach a release even if one were put in the secrets.
# --rehearsal also stores the notary profile without asking Apple to validate
# it, because the rehearsal's API key is a throwaway that Apple has never seen;
# a release validates it, so wrong credentials fail here, before the build,
# rather than forty minutes later at `notarytool submit`.
#
# WHAT THIS DOES NOT YET MAKE PASS. package.sh signs the bare Mach-O and the
# outer bundle, and leaves the code nested in the bundle as it was staged: the
# DuckDB CLI with its upstream signature, and the FineType extension ad-hoc.
# Apple's notary service refuses a bundle with ad-hoc nested code, so until the
# nested code is signed too, a tag built with these secrets stops at
# `notarytool submit` and publishes nothing. That fails closed: no build goes
# out half-signed, and removing the secrets restores the ad-hoc release.
# signing-rehearsal.yml records what each nested Mach-O carries on every run.
#
# `cleanup` takes the keychain off the search list, deletes it and the key file,
# and checks both are gone. It is safe to run when `setup` signed ad-hoc or
# failed half way, and release.yml runs it with `if: always()`.
set -euo pipefail

: "${RUNNER_TEMP:?release-signing.sh runs on a CI runner: RUNNER_TEMP is not set}"

KEYCHAIN="${RUNNER_TEMP}/brightfield-signing.keychain-db"
KEY_FILE="${RUNNER_TEMP}/brightfield-notary-key.p8"
P12_FILE="${RUNNER_TEMP}/brightfield-developer-id.p12"
PROFILE="brightfield-notary"
SECRETS=(
  MACOS_DEVELOPER_ID_P12_BASE64
  MACOS_DEVELOPER_ID_P12_PASSWORD
  APPLE_NOTARY_KEY_P8_BASE64
  APPLE_NOTARY_KEY_ID
  APPLE_NOTARY_ISSUER_ID
)

summary() {
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    echo "$1" >> "$GITHUB_STEP_SUMMARY"
  fi
}

# The keychains on the user search list, one path per line, without the quotes
# `security` prints around each.
search_list() {
  security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"[[:space:]]*$//'
}

setup() {
  local rehearsal=""
  case "${1:-}" in
    "") ;;
    --rehearsal) rehearsal=1 ;;
    *) echo "release-signing.sh: unknown setup option '$1'" >&2; exit 2 ;;
  esac
  if [ -n "$rehearsal" ] && [ "${GITHUB_REF_TYPE:-}" = "tag" ]; then
    echo "release-signing.sh: --rehearsal on a tag (${GITHUB_REF_NAME:-?})." >&2
    echo "  The rehearsal accepts an identity that is not Developer ID; a release must not." >&2
    exit 1
  fi

  local present=() missing=()
  local name
  for name in "${SECRETS[@]}"; do
    if [ -n "${!name:-}" ]; then present+=("$name"); else missing+=("$name"); fi
  done

  if [ "${#present[@]}" -eq 0 ]; then
    echo "== signing: ad-hoc. No Developer ID secrets are set, so package.sh signs the"
    echo "   bundle ad-hoc and leaves the tarball's Mach-O as the linker signed it."
    echo "::notice title=Signing::ad-hoc: no Developer ID secrets are set; this build is not notarised."
    summary "Signing: **ad-hoc** — no Developer ID secrets are set; this build is not notarised."
    return 0
  fi
  if [ "${#missing[@]}" -gt 0 ]; then
    echo "release-signing.sh: some signing secrets are set and some are not." >&2
    echo "  set:     ${present[*]}" >&2
    echo "  missing: ${missing[*]}" >&2
    echo "  Enter all five to sign and notarise, or none to build ad-hoc." >&2
    exit 1
  fi
  : "${GITHUB_ENV:?release-signing.sh setup needs GITHUB_ENV to hand the identity to later steps}"

  if [ -e "$KEYCHAIN" ]; then
    echo "release-signing.sh: ${KEYCHAIN} already exists; run cleanup first." >&2
    exit 1
  fi

  local password
  password=$(openssl rand -hex 24)
  echo "::add-mask::${password}"

  echo "== signing: keychain ${KEYCHAIN}"
  security create-keychain -p "$password" "$KEYCHAIN"
  # No auto-lock within the job's lifetime: a release build takes most of an
  # hour, and a keychain that locks mid-build fails codesign with a prompt
  # nobody can answer.
  security set-keychain-settings -lut 21600 "$KEYCHAIN"
  security unlock-keychain -p "$password" "$KEYCHAIN"

  printf '%s' "$MACOS_DEVELOPER_ID_P12_BASE64" | base64 --decode > "$P12_FILE"
  security import "$P12_FILE" -k "$KEYCHAIN" -f pkcs12 \
    -P "$MACOS_DEVELOPER_ID_P12_PASSWORD" -T /usr/bin/codesign
  rm -f "$P12_FILE"
  # Without this, codesign asks the keychain for permission to use the key,
  # and on a runner that prompt hangs the job until it times out.
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s \
    -k "$password" "$KEYCHAIN" > /dev/null

  # Prepend, keeping the runner's own keychains, so codesign finds the identity
  # by hash without a --keychain argument package.sh does not pass.
  local existing=()
  while IFS= read -r name; do
    [ -n "$name" ] && existing+=("$name")
  done < <(search_list)
  security list-keychains -d user -s "$KEYCHAIN" ${existing[@]+"${existing[@]}"}

  # Every code-signing identity in this keychain, valid or not: a Developer ID
  # certificate whose intermediate is missing from the runner is not "valid"
  # to find-identity, and still signs. The name check below is what decides.
  local identities
  identities=$(security find-identity -p codesigning "$KEYCHAIN" \
    | sed -n 's/^[[:space:]]*[0-9][0-9]*) \([0-9A-F]\{40\}\) "\([^"]*\)".*$/\1 \2/p' | sort -u)
  local count
  count=$(printf '%s\n' "$identities" | grep -c . || true)
  if [ "$count" -ne 1 ]; then
    echo "release-signing.sh: the .p12 holds ${count} code-signing identities; it must hold one." >&2
    printf '%s\n' "$identities" >&2
    exit 1
  fi
  local hash identity_name
  hash=${identities%% *}
  identity_name=${identities#* }
  case "$identity_name" in
    "Developer ID Application: "*) ;;
    *)
      if [ -z "$rehearsal" ]; then
        echo "release-signing.sh: the .p12's identity is '${identity_name}'." >&2
        echo "  A release signs only with a \"Developer ID Application: …\" identity." >&2
        exit 1
      fi
      ;;
  esac
  echo "== signing: identity ${hash} \"${identity_name}\""

  printf '%s' "$APPLE_NOTARY_KEY_P8_BASE64" | base64 --decode > "$KEY_FILE"
  chmod 600 "$KEY_FILE"
  local validate=--validate
  [ -n "$rehearsal" ] && validate=--no-validate
  echo "== signing: notarytool profile ${PROFILE} (${validate#--})"
  xcrun notarytool store-credentials "$PROFILE" \
    --key "$KEY_FILE" --key-id "$APPLE_NOTARY_KEY_ID" --issuer "$APPLE_NOTARY_ISSUER_ID" \
    --keychain "$KEYCHAIN" "$validate"
  rm -f "$KEY_FILE"

  {
    echo "BRIGHTFIELD_SIGN_IDENTITY=${hash}"
    echo "BRIGHTFIELD_NOTARY_PROFILE=${PROFILE}"
    echo "BRIGHTFIELD_NOTARY_KEYCHAIN=${KEYCHAIN}"
  } >> "$GITHUB_ENV"
  summary "Signing: **${identity_name}**, hardened runtime; notarised through profile \`${PROFILE}\`."
}

cleanup() {
  rm -f "$KEY_FILE" "$P12_FILE"
  local kept=() name
  while IFS= read -r name; do
    [ -n "$name" ] && [ "$name" != "$KEYCHAIN" ] && kept+=("$name")
  done < <(search_list)
  if [ "${#kept[@]}" -gt 0 ]; then
    security list-keychains -d user -s "${kept[@]}"
  fi
  if [ -e "$KEYCHAIN" ]; then
    security delete-keychain "$KEYCHAIN"
    echo "== signing: deleted ${KEYCHAIN}"
  else
    echo "== signing: no keychain to delete"
  fi
  if [ -e "$KEYCHAIN" ] || search_list | grep -Fxq "$KEYCHAIN"; then
    echo "release-signing.sh: ${KEYCHAIN} is still present after cleanup." >&2
    exit 1
  fi
  if [ -e "$KEY_FILE" ] || [ -e "$P12_FILE" ]; then
    echo "release-signing.sh: a key file is still present after cleanup." >&2
    exit 1
  fi
}

case "${1:-}" in
  setup) shift; setup "$@" ;;
  cleanup) cleanup ;;
  *) echo "usage: release-signing.sh setup [--rehearsal] | cleanup" >&2; exit 2 ;;
esac
