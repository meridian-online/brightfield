#!/usr/bin/env bash
# Prove scripts/check-finetype-pin.sh reddens, one disagreement at a time.
#
# The check it exercises is cheap and runs on every pull request, so this is
# not about a rare code path. It is about the check being able to fail at all:
# a pin check that agrees with itself is the exact shape of a guard that
# reports green over a release staging whatever it likes.
#
# Both directions, like the other gate self-tests here. The real repository
# must PASS — a check that cried wolf would be reverted within the day and the
# pin would go unchecked again — and each way a consumer can disagree must
# FAIL, with a message naming what disagreed.
#
# Every fixture below overrides one input and leaves the rest real, so what is
# proven is that the check reads that input rather than that it can be made to
# fail somehow.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
CHECK="${HERE}/check-finetype-pin.sh"
ROOT=$(cd "${HERE}/.." && pwd)

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bf-pin-selftest.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

fails=0
out="$TMP/out"

expect_pass() {
  local name="$1"; shift
  if "$CHECK" "$@" > "$out" 2>&1; then
    echo "  ok   ${name}"
  else
    echo "  FAIL ${name}: expected a pass, got a refusal"
    sed 's/^/       /' "$out"
    fails=$((fails + 1))
  fi
}

expect_fail() {
  local name="$1" needle="$2"; shift 2
  if "$CHECK" "$@" > "$out" 2>&1; then
    echo "  FAIL ${name}: expected a refusal, got a pass"
    sed 's/^/       /' "$out"
    fails=$((fails + 1))
    return
  fi
  if ! grep -qF -- "$needle" "$out"; then
    echo "  FAIL ${name}: refused without saying ${needle}"
    sed 's/^/       /' "$out"
    fails=$((fails + 1))
    return
  fi
  echo "  ok   ${name}"
}

# A stub release workflow that satisfies every textual requirement, so a case
# about the pin or a consumer is not passing or failing for the workflow's
# reasons.
GOOD_WORKFLOW="$TMP/good-workflow.yml"
cat > "$GOOD_WORKFLOW" <<'YML'
      - run: |
          TAG=$(scripts/finetype-pin.sh)
          scripts/fetch-finetype-bundle.sh "${{ matrix.target }}" "$RUNNER_TEMP/finetype"
      - env:
          BRIGHTFIELD_FINETYPE_BUNDLE: ${{ runner.temp }}/finetype
        run: scripts/package.sh "$GITHUB_REF_NAME" "${{ matrix.target }}"
      - run: scripts/check-artifact-type-source.sh --run dist/x.tar.gz "${{ matrix.target }}"
YML

# A well-formed pin is two lines: the tag and the registry revision. Every
# fixture below starts from that and breaks one thing, so a case cannot pass
# because the fixture was malformed in some other way.
REAL_REVISION=$("${HERE}/finetype-pin.sh" --revision)
pin_with() { # pin_with FILE TAG_LINE REVISION_LINE
  printf '%s\n%s\n' "$2" "$3" > "$1"
}

echo "== the real repository"
expect_pass "the pin, both consumers and the release workflow agree"

echo "== a declaration that does not read"
pin_with "$TMP/no-tag.env" "FINETYPE_RELEASE=v0.6.58" "FINETYPE_MODEL_REVISION=$REAL_REVISION"
expect_fail "a pin declaring no FINETYPE_TAG" "declares no FINETYPE_TAG" \
  --pin "$TMP/no-tag.env" --workflow "$GOOD_WORKFLOW"

pin_with "$TMP/no-v.env" "FINETYPE_TAG=0.6.58" "FINETYPE_MODEL_REVISION=$REAL_REVISION"
expect_fail "a tag with no leading v" "is not a v<major>.<minor>.<patch> tag" \
  --pin "$TMP/no-v.env" --workflow "$GOOD_WORKFLOW"

pin_with "$TMP/floating.env" "FINETYPE_TAG=latest" "FINETYPE_MODEL_REVISION=$REAL_REVISION"
expect_fail "a floating tag" "is not a v<major>.<minor>.<patch> tag" \
  --pin "$TMP/floating.env" --workflow "$GOOD_WORKFLOW"

pin_with "$TMP/minor.env" "FINETYPE_TAG=v0.6" "FINETYPE_MODEL_REVISION=$REAL_REVISION"
expect_fail "a two-part version" "is not a v<major>.<minor>.<patch> tag" \
  --pin "$TMP/minor.env" --workflow "$GOOD_WORKFLOW"

expect_fail "a pin file that is not there" "no pin file at" \
  --pin "$TMP/absent.env" --workflow "$GOOD_WORKFLOW"

# THE REGISTRY REVISION, on the same footing as the tag. The bundle has two
# sources and the model's bytes are decided by this line; a pin naming only a
# tag would leave them decided on the day of the build. A branch name is the
# case that matters, because it RESOLVES — nothing downstream notices that the
# model it staged is whatever the registry held that morning.
pin_with "$TMP/no-rev.env" "FINETYPE_TAG=v0.6.58" "# no revision here"
expect_fail "a pin declaring no model revision" "declares no FINETYPE_MODEL_REVISION" \
  --pin "$TMP/no-rev.env" --workflow "$GOOD_WORKFLOW"

pin_with "$TMP/branch-rev.env" "FINETYPE_TAG=v0.6.58" "FINETYPE_MODEL_REVISION=main"
expect_fail "a branch name where the revision goes" "is not a 40-character commit sha" \
  --pin "$TMP/branch-rev.env" --workflow "$GOOD_WORKFLOW"

pin_with "$TMP/short-rev.env" "FINETYPE_TAG=v0.6.58" "FINETYPE_MODEL_REVISION=${REAL_REVISION:0:7}"
expect_fail "an abbreviated commit sha" "is not a 40-character commit sha" \
  --pin "$TMP/short-rev.env" --workflow "$GOOD_WORKFLOW"

pin_with "$TMP/upper-rev.env" "FINETYPE_TAG=v0.6.58" \
  "FINETYPE_MODEL_REVISION=$(printf '%s' "$REAL_REVISION" | tr 'a-f' 'A-F')"
expect_fail "a revision that is not lowercase hex" "is not a 40-character commit sha" \
  --pin "$TMP/upper-rev.env" --workflow "$GOOD_WORKFLOW"

echo "== a consumer that stages something else"
# The tag here is well-formed and simply not the pinned one, which is the
# realistic shape: somebody bumps the pin and one consumer keeps its literal.
expect_fail "the fetch script answering with another tag" \
  "scripts/fetch-finetype-bundle.sh stages FineType 'v9.9.9'" \
  --fetch "echo v9.9.9" --workflow "$GOOD_WORKFLOW"

expect_fail "package.sh answering with another tag" \
  "scripts/package.sh stages FineType 'v9.9.9'" \
  --package "echo v9.9.9" --workflow "$GOOD_WORKFLOW"

expect_fail "a consumer that cannot answer at all" "failed rather than answering" \
  --fetch "false" --workflow "$GOOD_WORKFLOW"

echo "== a release workflow that does not stage the pinned release"
cat > "$TMP/no-fetch.yml" <<'YML'
      - run: TAG=$(scripts/finetype-pin.sh); echo "$TAG"
YML
expect_fail "a workflow that never calls the fetch script" \
  "does not obtain its FineType bundle" --workflow "$TMP/no-fetch.yml"

cat > "$TMP/no-pin-read.yml" <<'YML'
      - run: scripts/fetch-finetype-bundle.sh "${{ matrix.target }}" "$RUNNER_TEMP/finetype"
        env:
          BRIGHTFIELD_FINETYPE_BUNDLE: x
      - run: scripts/check-artifact-type-source.sh dist/x.tar.gz t
YML
expect_fail "a workflow that never reads the pin" \
  "does not read packaging/finetype-pin.env" --workflow "$TMP/no-pin-read.yml"

# Each of the four release-path assertions dropped on its own, because the
# other three stay green when one goes. The variable is the sharp one: a
# workflow can fetch a bundle correctly, package, and hand packaging nothing.
grep -v 'BRIGHTFIELD_FINETYPE_BUNDLE' "$GOOD_WORKFLOW" > "$TMP/no-variable.yml"
expect_fail "a workflow that fetches a bundle and never hands it to packaging" \
  "never sets BRIGHTFIELD_FINETYPE_BUNDLE" --workflow "$TMP/no-variable.yml"

grep -v 'check-artifact-type-source' "$GOOD_WORKFLOW" > "$TMP/no-readback.yml"
expect_fail "a workflow that never reads the packaged artifact back" \
  "never reads the packaged artifact back" --workflow "$TMP/no-readback.yml"

# ---------------------------------------------------------------------------
# THE SAME FOUR, EXPLAINED IN A COMMENT AND NOT DONE. This is the shape that
# beat the first version of the check: the release workflow documents its own
# mechanism, so every scan for the mechanism's name was satisfied by the
# paragraph above the step. Deleting the BRIGHTFIELD_FINETYPE_BUNDLE setting
# left the check green because the comment explaining it stayed.
#
# One fixture per assertion, each naming its subject in a YAML comment and
# doing none of it. A check that passes these is reading its own documentation.
# ---------------------------------------------------------------------------
described_only() { # described_only FILE DROP
  {
    printf '      # This step used to run %s. The comment stayed; the step did not.\n' "$2"
    grep -v -- "$2" "$GOOD_WORKFLOW"
  } > "$1"
}

described_only "$TMP/described-variable.yml" "BRIGHTFIELD_FINETYPE_BUNDLE"
expect_fail "the variable named in a comment and never set" \
  "never sets BRIGHTFIELD_FINETYPE_BUNDLE" --workflow "$TMP/described-variable.yml"

described_only "$TMP/described-readback.yml" "scripts/check-artifact-type-source.sh"
expect_fail "the read-back named in a comment and never run" \
  "never reads the packaged artifact back" --workflow "$TMP/described-readback.yml"

described_only "$TMP/described-fetch.yml" "scripts/fetch-finetype-bundle.sh"
expect_fail "the fetch named in a comment and never called" \
  "does not obtain its FineType bundle" --workflow "$TMP/described-fetch.yml"

described_only "$TMP/described-pin.yml" "scripts/finetype-pin.sh"
expect_fail "the pin reader named in a comment and never run" \
  "does not read packaging/finetype-pin.env" --workflow "$TMP/described-pin.yml"

# The literal cases use the tag the real pin declares, because that is the one
# a copy-paste produces and the one a scan for "some version somewhere" would
# be least likely to distinguish from the declaration.
REAL_TAG=$("${HERE}/finetype-pin.sh")
{
  cat "$GOOD_WORKFLOW"
  printf '          curl -O "https://example.invalid/finetype-%s-aarch64-apple-darwin.duckdb_extension"\n' "$REAL_TAG"
} > "$TMP/asset-literal.yml"
expect_fail "a workflow naming an asset by a literal tag" \
  "names a FineType version literally" --workflow "$TMP/asset-literal.yml"

{
  cat "$GOOD_WORKFLOW"
  printf '          FINETYPE_TAG=%s\n' "$REAL_TAG"
} > "$TMP/bare-literal.yml"
expect_fail "a workflow assigning the tag literally" \
  "names a FineType version literally" --workflow "$TMP/bare-literal.yml"

echo "== the check is not simply always red"
# A pin the real repository does not carry, with the real consumers reading it
# through BRIGHTFIELD_FINETYPE_PIN. If this failed, every case above would be
# passing because the check refuses any overridden input rather than because it
# read the disagreement.
pin_with "$TMP/other.env" "FINETYPE_TAG=v1.2.3" "FINETYPE_MODEL_REVISION=$REAL_REVISION"
expect_pass "another well-formed pin every consumer reads" \
  --pin "$TMP/other.env" --workflow "$GOOD_WORKFLOW"

echo
if [ "$fails" -ne 0 ]; then
  echo "check-finetype-pin-selftest: ${fails} case(s) did not behave as required." >&2
  exit 1
fi
echo "check-finetype-pin-selftest: the check passes what it should and refuses what it should."
