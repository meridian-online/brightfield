#!/usr/bin/env bash
# Print the FineType release tag this repository stages. THE one reader of
# packaging/finetype-pin.env.
#
#   scripts/finetype-pin.sh          -> v0.6.58
#
# Three consumers call this and none of them parses the file itself, which is
# what makes "they cannot disagree" a property of the code rather than a
# convention: scripts/fetch-finetype-bundle.sh, scripts/package.sh and
# .github/workflows/release.yml. scripts/check-finetype-pin.sh runs each of
# them and compares the answers.
#
# BRIGHTFIELD_FINETYPE_PIN overrides the file it reads. That exists for
# scripts/check-finetype-pin-selftest.sh, which has to hand this a malformed
# pin to prove the refusals below can fire; a release never sets it.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
PIN="${BRIGHTFIELD_FINETYPE_PIN:-${HERE}/../packaging/finetype-pin.env}"

fail() { echo "finetype-pin: $*" >&2; exit 1; }

[ -f "$PIN" ] || fail "no pin file at ${PIN}"

# One assignment, read as data. Sourcing the file would let a pin execute
# whatever it liked in the caller's shell, and every caller here is a release
# path.
tag=$(sed -n 's/^FINETYPE_TAG=//p' "$PIN" | head -1)

[ -n "$tag" ] || fail "${PIN} declares no FINETYPE_TAG"

# `v` then three dotted numbers, and nothing else. A tag that does not look
# like a FineType release tag would otherwise be pasted straight into an asset
# url and come back as a 404 during a release, which is the worst moment to
# find out.
printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' \
  || fail "${PIN} declares FINETYPE_TAG='${tag}', which is not a v<major>.<minor>.<patch> tag"

printf '%s\n' "$tag"
