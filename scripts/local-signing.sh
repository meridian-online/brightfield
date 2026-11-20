#!/usr/bin/env bash
# Hand scripts/package.sh a Developer ID identity and a notarisation profile on
# this Mac, from 1Password, with nothing for the person to do but sign in.
#
#   op signin
#   eval "$(scripts/local-signing.sh setup)"   # exports the four BRIGHTFIELD_* variables
#   scripts/package.sh ...
#   scripts/local-signing.sh cleanup
#
# It reuses scripts/release-signing.sh, so a local signed build exercises the
# same keychain and notarytool-profile setup a release runs, with the five
# secrets that script reads taken from 1Password's Meridian vault with
# `op read` instead of from repository secrets. RUNNER_TEMP and GITHUB_ENV are
# pointed at a scratch folder this script owns.
#
# The certificate is never imported into a lasting keychain. The throwaway
# keychain release-signing.sh builds holds it for the length of one build, and
# `cleanup` deletes it: a Developer ID key left unattended in a login keychain
# could sign code as Meridian without anyone knowing.
#
# `setup` prints only the export lines, on stdout. Progress goes to stderr, so
# the eval above sees nothing else. No secret is ever printed: the values move
# from `op read` into environment variables of one child process and nowhere
# else.
#
# 1Password items are found by title prefix in the vault, then read by id,
# because a title with parentheses or a colon is awkward inside an op://
# reference. The prefixes carry no key or team id, so the identifiers of the
# signing material stay out of this public file. The file to read is the item's
# own .p12 or .p8 attachment, whatever it is called. Each name can be
# overridden:
#
#   BRIGHTFIELD_OP_VAULT      Meridian
#   BRIGHTFIELD_OP_CERT_ITEM  Apple Developer ID Application certificate
#   BRIGHTFIELD_OP_KEY_ITEM   App Store Connect API key: brightfield notarisation
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VAULT="${BRIGHTFIELD_OP_VAULT:-Meridian}"
CERT_PREFIX="${BRIGHTFIELD_OP_CERT_ITEM:-Apple Developer ID Application certificate}"
KEY_PREFIX="${BRIGHTFIELD_OP_KEY_ITEM:-App Store Connect API key: brightfield notarisation}"

# A fixed path under TMPDIR, so `cleanup` finds what `setup` made.
SCRATCH="${TMPDIR:-/tmp}"
SCRATCH="${SCRATCH%/}/brightfield-local-signing"

refuse() {
  echo "local-signing.sh: $1" >&2
  echo "  Run \`op signin\` first, then try again." >&2
  exit 1
}

# op's own error text is dropped: it can name the item being read.
quietly() { "$@" 2>/dev/null; }

# item_id <title prefix>: the id of the one item in the vault whose title starts with it.
item_id() {
  local list ids count
  list="$(quietly op item list --vault "$VAULT" --format json)" \
    || refuse "cannot list the ${VAULT} vault in 1Password."
  ids="$(jq -r --arg p "$1" '.[] | select(.title | startswith($p)) | .id' <<<"$list")"
  count="$(grep -c . <<<"$ids" || true)"
  if [ "$count" != 1 ]; then
    echo "local-signing.sh: ${count} items in ${VAULT} start with \"$1\"; expected exactly one." >&2
    exit 1
  fi
  echo "$ids"
}

# attachment_name <item id> <extension>: the name of the item's one file with that extension.
attachment_name() {
  local item names count
  item="$(quietly op item get "$1" --vault "$VAULT" --format json)" \
    || refuse "cannot read an item in the ${VAULT} vault."
  names="$(jq -r --arg e ".$2" '(.files // [])[] | select(.name | endswith($e)) | .name' <<<"$item")"
  count="$(grep -c . <<<"$names" || true)"
  if [ "$count" != 1 ]; then
    echo "local-signing.sh: an item in ${VAULT} holds ${count} .$2 files; expected exactly one." >&2
    exit 1
  fi
  echo "$names"
}

# op_read <item id> <field or file name>
op_read() {
  quietly op read "op://${VAULT}/$1/$2" || refuse "cannot read a secret from the ${VAULT} vault."
}

setup() {
  command -v op >/dev/null || { echo "local-signing.sh: the 1Password CLI (op) is not installed." >&2; exit 1; }
  command -v jq >/dev/null || { echo "local-signing.sh: jq is not installed." >&2; exit 1; }

  if [ -e "$SCRATCH" ]; then
    echo "local-signing.sh: ${SCRATCH} already exists; run cleanup first." >&2
    exit 1
  fi

  # Read everything before creating anything, so a missing sign-in leaves no
  # keychain and no folder behind.
  local cert key p12_name p8_name
  cert="$(item_id "$CERT_PREFIX")"
  key="$(item_id "$KEY_PREFIX")"
  p12_name="$(attachment_name "$cert" p12)"
  p8_name="$(attachment_name "$key" p8)"

  local p12_b64 p12_pw p8_b64 key_id issuer_id
  p12_b64="$(op_read "$cert" "$p12_name" | base64)"
  p12_pw="$(op_read "$cert" password)"
  p8_b64="$(op_read "$key" "$p8_name" | base64)"
  key_id="$(op_read "$key" "Key ID")"
  issuer_id="$(op_read "$key" "Issuer ID")"

  umask 077
  mkdir -p "$SCRATCH/runner"
  : > "$SCRATCH/github_env"

  echo "== local signing: setting up in ${SCRATCH}" >&2
  # release-signing.sh's own output goes to stderr so stdout carries only the exports.
  if ! RUNNER_TEMP="$SCRATCH/runner" GITHUB_ENV="$SCRATCH/github_env" \
       MACOS_DEVELOPER_ID_P12_BASE64="$p12_b64" \
       MACOS_DEVELOPER_ID_P12_PASSWORD="$p12_pw" \
       APPLE_NOTARY_KEY_P8_BASE64="$p8_b64" \
       APPLE_NOTARY_KEY_ID="$key_id" \
       APPLE_NOTARY_ISSUER_ID="$issuer_id" \
       "$HERE/release-signing.sh" setup >&2; then
    echo "local-signing.sh: release-signing.sh setup failed; cleaning up." >&2
    cleanup >&2 || true
    exit 1
  fi

  local line
  while IFS= read -r line; do
    case "$line" in
      BRIGHTFIELD_*=*) printf 'export %s=%q\n' "${line%%=*}" "${line#*=}" ;;
    esac
  done < "$SCRATCH/github_env"
}

cleanup() {
  # Safe when nothing was set up: release-signing.sh reports "no keychain to
  # delete" and the folder is simply absent.
  if [ -d "$SCRATCH/runner" ]; then
    RUNNER_TEMP="$SCRATCH/runner" "$HERE/release-signing.sh" cleanup
  else
    echo "== local signing: nothing to clean up"
  fi
  rm -rf "$SCRATCH"
}

case "${1:-}" in
  setup) setup ;;
  cleanup) cleanup ;;
  *) echo "usage: local-signing.sh setup | cleanup" >&2; exit 2 ;;
esac
