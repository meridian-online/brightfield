#!/usr/bin/env bash
# Every consumer of the FineType pin answers with the pinned tag.
#
#   scripts/check-finetype-pin.sh
#   scripts/check-finetype-pin.sh --pin FILE --fetch CMD --package CMD --workflow FILE
#
# packaging/finetype-pin.env declares one tag and three things stage from it.
# The failure this exists to catch is one of them quietly carrying a literal
# instead — the pin then reads as reviewed while the release builds against
# something nobody looked at, and the two disagree silently because nothing
# ever compares them.
#
# IT RUNS THE CONSUMERS RATHER THAN READING THEM. `scripts/package.sh
# --print-finetype-pin` and `scripts/fetch-finetype-bundle.sh --print-tag` each
# print the same variable their real path uses, so what is compared here is
# behaviour and not source text. That distinction is the whole design: a scan
# for a version literal is a guess about how somebody would write the mistake,
# and it takes one rename, one shared helper or one indirection to walk past.
#
# The workflow is the exception and it is named as one. A YAML file cannot be
# executed here, so the release workflow is checked by reading it: it must
# obtain its bundle through scripts/fetch-finetype-bundle.sh (which takes no
# tag argument, so there is nowhere to pass a different one), it must read the
# pin through scripts/finetype-pin.sh, and it must carry no FineType tag
# literal. That is the weakest check in this file and the only one that could
# be evaded by writing the mistake in a shape nobody predicted.
#
# WHAT THIS DOES NOT COVER: whether the bytes that were staged carry the pinned
# version. Nothing textual can — the version stamp comes from FineType's build.
# scripts/package.sh passes the pin to scripts/check-bundled-extension.sh,
# which compares it against the extension's own trailer, and
# scripts/package-finetype-selftest.sh drives that refusal with a pin the
# fixture deliberately disagrees with.
#
# The overrides exist for scripts/check-finetype-pin-selftest.sh, which has to
# hand this a consumer that answers wrongly to prove the refusals can fire.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "${HERE}/.." && pwd)

PIN="${ROOT}/packaging/finetype-pin.env"
FETCH_CMD="${HERE}/fetch-finetype-bundle.sh --print-tag"
PACKAGE_CMD="${HERE}/package.sh --print-finetype-pin"
WORKFLOW="${ROOT}/.github/workflows/release.yml"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --pin)      PIN="$2";         shift 2 ;;
    --fetch)    FETCH_CMD="$2";   shift 2 ;;
    --package)  PACKAGE_CMD="$2"; shift 2 ;;
    --workflow) WORKFLOW="$2";    shift 2 ;;
    *) echo "check-finetype-pin: unknown argument '$1'" >&2; exit 2 ;;
  esac
done

fail() { echo "check-finetype-pin: $*" >&2; exit 1; }

export BRIGHTFIELD_FINETYPE_PIN="$PIN"

# The declaration itself, through the one reader. Its own refusals — no file,
# no FINETYPE_TAG, a tag that is not a v<major>.<minor>.<patch> — are what
# fires here.
TAG=$("${HERE}/finetype-pin.sh") || fail "the pin does not read: ${PIN}"
echo "   declared: ${TAG}  (${PIN})"

# consumer NAME COMMAND — run it and require the pinned tag back.
consumer() {
  local name="$1" cmd="$2" answer
  # shellcheck disable=SC2086  # the command is a word list on purpose
  answer=$(cd "$ROOT" && $cmd 2>&1) || fail "${name} failed rather than answering:
${answer}"
  answer=$(printf '%s' "$answer" | tr -d '[:space:]')
  [ "$answer" = "$TAG" ] || fail "${name} stages FineType '${answer}' while \
${PIN} declares '${TAG}'"
  echo "   ${name}: ${answer}"
}

consumer "scripts/fetch-finetype-bundle.sh" "$FETCH_CMD"
consumer "scripts/package.sh" "$PACKAGE_CMD"

[ -f "$WORKFLOW" ] || fail "no workflow at ${WORKFLOW}"

grep -q 'scripts/fetch-finetype-bundle.sh' "$WORKFLOW" \
  || fail "${WORKFLOW} does not obtain its FineType bundle through \
scripts/fetch-finetype-bundle.sh, so what it stages is not the pinned release"

grep -q 'scripts/finetype-pin.sh' "$WORKFLOW" \
  || fail "${WORKFLOW} does not read packaging/finetype-pin.env through \
scripts/finetype-pin.sh"

# A FineType tag written into the workflow. Both spellings a release would use:
# the asset-name form the fetch script builds, and the pinned tag on its own.
if literal=$(grep -nE "finetype-v[0-9]+\.[0-9]+\.[0-9]+|${TAG}" "$WORKFLOW"); then
  echo "$literal" | sed 's/^/     /' >&2
  fail "${WORKFLOW} names a FineType version literally on the lines above; it must \
read scripts/finetype-pin.sh instead"
fi

echo "check-finetype-pin: ${TAG} — the declaration and every consumer agree."
