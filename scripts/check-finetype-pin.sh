#!/usr/bin/env bash
# Every consumer of the FineType pin answers with what the pin declares.
#
#   scripts/check-finetype-pin.sh
#   scripts/check-finetype-pin.sh --pin FILE --fetch CMD --package CMD --workflow FILE
#
# packaging/finetype-pin.env declares a FineType tag and a model registry
# revision, and three things stage from them.
# The failure this exists to catch is one of them quietly carrying a literal
# instead — the pin then reads as reviewed while the release builds against
# something nobody looked at, and the two disagree silently because nothing
# ever compares them.
#
# IT EXECUTES THE CONSUMERS, AND THAT IS LESS THAN IT SOUNDS. What it runs is
# each consumer's `--print` mode, not the path that decides which bytes get
# staged, so what is compared is "can this file read the pin" and not "does
# this file stage what the pin says". A print mode can be right while the code
# beside it reads a literal — measured, on this branch, in scripts/package.sh.
#
# The staging paths are pinned elsewhere and by running them:
# scripts/package-finetype-selftest.sh overrides the pin with a tag its fixture
# does not carry, and scripts/fetch-finetype-bundle-selftest.sh drives the real
# fetch against a loopback registry. This file is the cheap cross-check that
# the three readers agree, not the evidence that any of them is used.
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
TAG=$("${HERE}/finetype-pin.sh" --tag) || fail "the pin does not read: ${PIN}"

# The registry revision, on the same footing as the tag. The bundle has two
# sources and only one of them is the tag: a FineType release attaches the
# extension and the catalogue, and the model comes from the registry, so a pin
# that declared only the tag would leave the model bytes decided on the day.
# `finetype-pin.sh --revision` is where a branch name is refused; this is what
# makes that refusal run on a pull request rather than on a release.
REVISION=$("${HERE}/finetype-pin.sh" --revision) \
  || fail "the pin's model revision does not read: ${PIN}"

echo "   declared: ${TAG} at ${REVISION}  (${PIN})"

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

# COMMENT LINES ARE STRIPPED FIRST, and that is not tidiness. The first version
# of this grepped the whole file, and deleting the `BRIGHTFIELD_FINETYPE_BUNDLE`
# setting from the Package step left the check GREEN — because the comment
# above that step explains what the variable is for and mentions it by name. A
# file that documents its own mechanism satisfies every scan for the mechanism's
# name. All five readings below run against code only for that reason, and
# scripts/check-finetype-pin-selftest.sh drives each of them with a fixture that
# names the thing in a comment and does not do it.
#
# A YAML comment is a line whose first non-space character is `#`. A `#` inside
# a `run:` block is shell, not YAML, and is deliberately still read: a shell
# comment in a release step is a line that runs.
CODE="$(grep -vE '^[[:space:]]*#' "$WORKFLOW")"

reads() { printf '%s\n' "$CODE" | grep -q -- "$1"; }

# The four things the release path has to do, in the order it does them. They
# are read rather than run for the reason above, and they are HERE rather than
# in four files because they are one claim: the release stages the pinned
# bundle and then proves it did. Deleting any one of them leaves the other
# three green — the variable is the one that stings, because a workflow that
# fetches a bundle correctly and never hands it to packaging produces exactly
# the empty artifact this whole change exists to stop.
reads 'scripts/fetch-finetype-bundle.sh' \
  || fail "${WORKFLOW} does not obtain its FineType bundle through \
scripts/fetch-finetype-bundle.sh, so what it stages is not the pinned release"

reads 'scripts/finetype-pin.sh' \
  || fail "${WORKFLOW} does not read packaging/finetype-pin.env through \
scripts/finetype-pin.sh"

reads 'BRIGHTFIELD_FINETYPE_BUNDLE' \
  || fail "${WORKFLOW} never sets BRIGHTFIELD_FINETYPE_BUNDLE, so scripts/package.sh \
returns at the first line of stage_finetype and the artifact ships with no type source"

reads 'scripts/check-artifact-type-source.sh' \
  || fail "${WORKFLOW} never reads the packaged artifact back with \
scripts/check-artifact-type-source.sh, so a release that staged nothing would publish green"

# A FineType tag written into the workflow. Both spellings a release would use:
# the asset-name form the fetch script builds, and the pinned tag on its own.
if literal=$(printf '%s\n' "$CODE" | grep -nE "finetype-v[0-9]+\.[0-9]+\.[0-9]+|${TAG}"); then
  echo "$literal" | sed 's/^/     /' >&2
  fail "${WORKFLOW} names a FineType version literally on the lines above; it must \
read scripts/finetype-pin.sh instead"
fi

echo "check-finetype-pin: ${TAG} at ${REVISION} — the declaration and every consumer agree."
