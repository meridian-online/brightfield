#!/usr/bin/env bash
# Print a field of the FineType pin. THE one reader of
# packaging/finetype-pin.env.
#
#   scripts/finetype-pin.sh              -> v0.6.58            (the tag)
#   scripts/finetype-pin.sh --revision   -> 94cda10a6…         (the model revision)
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

# Read as data. Sourcing the file would let a pin execute whatever it liked in
# the caller's shell, and every caller here is a release path.
field() { sed -n "s/^$1=//p" "$PIN" | head -1; }

case "${1:---tag}" in
  --tag)
    tag=$(field FINETYPE_TAG)
    [ -n "$tag" ] || fail "${PIN} declares no FINETYPE_TAG"
    # `v` then three dotted numbers, and nothing else. A tag that does not look
    # like a FineType release tag would otherwise be pasted straight into an
    # asset url and come back as a 404 during a release, which is the worst
    # moment to find out.
    printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' \
      || fail "${PIN} declares FINETYPE_TAG='${tag}', which is not a v<major>.<minor>.<patch> tag"
    printf '%s\n' "$tag"
    ;;
  --revision)
    rev=$(field FINETYPE_MODEL_REVISION)
    [ -n "$rev" ] || fail "${PIN} declares no FINETYPE_MODEL_REVISION"
    # A full 40-character commit sha and not a branch name. `main` would fetch
    # whatever the registry holds today, which is the whole thing the pin
    # exists to stop — and it would resolve, so nothing downstream would notice.
    printf '%s\n' "$rev" | grep -Eq '^[0-9a-f]{40}$' \
      || fail "${PIN} declares FINETYPE_MODEL_REVISION='${rev}', which is not a 40-character \
commit sha — a branch name would stage whatever the registry holds on the day"
    printf '%s\n' "$rev"
    ;;
  *)
    fail "unknown field '$1' (expected --tag or --revision)"
    ;;
esac
